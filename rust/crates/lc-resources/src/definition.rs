use crate::{GraphicsImage, Group, GroupError};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

const C4D_MAX_VERTEX: usize = 30;
const C4M_SOLID: i32 = 50;

/// Files required to construct an engine definition from a classic C4 definition folder.
#[derive(Debug, Clone)]
pub struct Definition {
    pub core: DefCore,
    pub script: DefinitionScript,
    pub action_map: Option<ActionMap>,
    pub picture_image: Option<GraphicsImage>,
    pub graphics_image: Option<GraphicsImage>,
    pub color_by_owner_mask: Option<ColorByOwnerMask>,
    pub additional_graphics: HashMap<String, DefinitionGraphicsVariant>,
}

#[derive(Debug, Clone)]
pub struct ColorByOwnerMask {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DefinitionGraphicsVariant {
    pub name: String,
    pub image: GraphicsImage,
    pub color_by_owner_mask: Option<ColorByOwnerMask>,
}

impl Definition {
    pub fn load(group: &Group) -> Result<Self, DefinitionError> {
        let core = DefCore::load(group)?;

        let script = load_scripts(group)?;

        let action_map = match group.read_file("ActMap.txt") {
            Ok(bytes) => Some(parse_act_map(&bytes)?),
            Err(GroupError::EntryNotFound(_)) => None,
            Err(GroupError::Io(ref err)) if err.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(DefinitionError::Resources(error)),
        };

        let picture_image = load_definition_picture(group, &core);
        let (graphics_image, color_by_owner_mask, additional_graphics) =
            load_definition_graphics(group, core.color_by_owner);

        Ok(Self {
            core,
            script,
            action_map,
            picture_image,
            graphics_image,
            color_by_owner_mask,
            additional_graphics,
        })
    }
}

/// `C4MaxPhysical` (C4InfoCore.h:31): the 100% value of every physical.
pub const C4_MAX_PHYSICAL: i32 = 100_000;

/// Mirror of `C4PhysicalInfo` (C4InfoCore.h:34-63), parsed from the
/// `[Physical]` section of DefCore.txt with the `C4PhysInfoNameMap` field
/// names (C4InfoCore.cpp:181-205). Defaults are all zero
/// (`C4PhysicalInfo::Default`, C4InfoCore.cpp:239-242).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
pub struct PhysicalInfo {
    pub energy: i32,
    pub breath: i32,
    pub walk: i32,
    pub jump: i32,
    pub scale: i32,
    pub hangle: i32,
    pub dig: i32,
    pub swim: i32,
    pub throw: i32,
    pub push: i32,
    pub fight: i32,
    pub magic: i32,
    pub float: i32,
    pub can_scale: i32,
    pub can_hangle: i32,
    pub can_dig: i32,
    pub can_construct: i32,
    pub can_chop: i32,
    pub can_fly: i32,
    pub corrosion_resist: i32,
    pub breathe_water: i32,
}

impl PhysicalInfo {
    /// A mutable slot by its `C4PhysInfoNameMap` name (the C++
    /// `GetOffsetByName`, C4InfoCore.cpp:181-205; case-insensitive); None
    /// for unknown names.
    pub fn value_mut_by_name(&mut self, name: &str) -> Option<&mut i32> {
        match name.to_ascii_lowercase().as_str() {
            "energy" => Some(&mut self.energy),
            "breath" => Some(&mut self.breath),
            "walk" => Some(&mut self.walk),
            "jump" => Some(&mut self.jump),
            "scale" => Some(&mut self.scale),
            "hangle" => Some(&mut self.hangle),
            "dig" => Some(&mut self.dig),
            "swim" => Some(&mut self.swim),
            "throw" => Some(&mut self.throw),
            "push" => Some(&mut self.push),
            "fight" => Some(&mut self.fight),
            "magic" => Some(&mut self.magic),
            "float" => Some(&mut self.float),
            "canscale" => Some(&mut self.can_scale),
            "canhangle" => Some(&mut self.can_hangle),
            "candig" => Some(&mut self.can_dig),
            "canconstruct" => Some(&mut self.can_construct),
            "canchop" => Some(&mut self.can_chop),
            "canfly" => Some(&mut self.can_fly),
            "corrosionresist" => Some(&mut self.corrosion_resist),
            "breathewater" => Some(&mut self.breathe_water),
            _ => None,
        }
    }

    /// Read a value by its `C4PhysInfoNameMap` name; None for unknown names.
    pub fn value_by_name(&self, name: &str) -> Option<i32> {
        let mut copy = *self;
        copy.value_mut_by_name(name).map(|slot| *slot)
    }

    /// Assign a value by its `C4PhysInfoNameMap` name; returns false for
    /// unknown names.
    pub fn set_by_name(&mut self, name: &str, value: i32) -> bool {
        self.value_mut_by_name(name)
            .map(|slot| *slot = value)
            .is_some()
    }

    /// `C4PhysicalInfo::TrainValue` (C4InfoCore.cpp:279-285): only nonzero
    /// values train; never above the cap, never decreased.
    pub fn train_value(value: &mut i32, train_by: i32, max_train: i32) {
        if *value != 0 {
            *value = (*value + train_by).min(max_train).max(*value);
        }
    }
}

/// Parsed metadata from `DefCore.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefComponent {
    pub id: String,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct DefCore {
    pub id: String,
    pub name: Option<String>,
    pub category: i32,
    pub crew_member: bool,
    pub value: i32,
    pub mass: i32,
    pub picture: Option<PictureRect>,
    pub color_by_owner: bool,
    pub shape: Option<PictureRect>,
    pub solid_mask: Option<TargetRect>,
    pub vertices: Vec<DefVertex>,
    pub contact_density: i32,
    pub contact_function_calls: bool,
    pub collection: Option<PictureRect>,
    pub collection_limit: Option<u32>,
    /// ContactIncinerate=N: 1-in-N chance of catching fire on contact with a
    /// burning object (CrossCheck pass 1, C4GameObjects.cpp:121-125); 0 = not
    /// inflammable.
    pub contact_incinerate: i32,
    /// NoBurnDecay=1: burning does not reduce Con (C4Object.cpp:777-778).
    pub no_burn_decay: bool,
    /// `Float` (C4Def.cpp:379, default 0): buoyancy line offset in percent
    /// of Con — IsInLiquidCheck probes GBackLiquid(x, y + Float*Con/FullCon
    /// - 1) (C4Object.cpp:5609-5612).
    pub float_line: i32,
    /// `Grab` (C4Def.cpp): 0 none, 1 grab+push, 2 grab-only.
    pub grab: i32,
    /// `NoBreath` (C4Def.cpp:409): exempt from the ExecLife breathing check.
    pub no_breath: bool,
    /// NoBurnDamage=1: burning deals no damage (C4Object.cpp:780).
    pub no_burn_damage: bool,
    /// BurnTurnTo=ID: definition change on incineration (C4Effect.cpp:580-585).
    pub burn_turn_to: Option<String>,
    /// IncompleteActivity=1: keeps contents on incineration and allows
    /// collection below FullCon (C4Effect.cpp:588, SetOCF C4Object.cpp:594).
    pub incomplete_activity: bool,
    /// The [Physical] section (C4Def loads it via FollowName("Physical"),
    /// C4Def.cpp:459-460).
    pub physical: PhysicalInfo,
    pub collectible: bool,
    pub constructable: bool,
    pub con_size_off: i32,
    pub stretch_growth: bool,
    pub basement: i32,
    pub rotateable: i32,
    pub border_bound: i32,
    pub upright_attach: u32,
    /// NoStabilize (C4Def.cpp:402): opts out of the Stabilize upright snap.
    pub no_stabilize: bool,
    /// Timer= interval in frames (default 35, C4Def.cpp:298).
    pub timer: i32,
    /// TimerCall= function name (C4Def.cpp:299); None when absent/empty.
    pub timer_call: Option<String>,
    pub components: Vec<DefComponent>,
    pub line_connect: u32,
    /// `CanBeBase` (C4Def.cpp DefCore): marks structures usable as the
    /// FirstBase in PlaceReadyBase (C4Player.cpp:596-599).
    pub can_be_base: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefVertex {
    pub x: i32,
    pub y: i32,
    pub cnat: u32,
    pub friction: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

impl DefCore {
    pub fn load(group: &Group) -> Result<Self, DefinitionError> {
        let bytes = group.read_file("DefCore.txt").map_err(|err| match err {
            GroupError::EntryNotFound(_) => DefinitionError::DefCoreMissing,
            other => DefinitionError::Resources(other),
        })?;
        parse_def_core(&bytes)
    }
}

/// Combined script sources originating from a definition group.
#[derive(Debug, Clone)]
pub struct DefinitionScript {
    files: Vec<DefinitionScriptFile>,
    combined: String,
}

impl DefinitionScript {
    pub fn files(&self) -> &[DefinitionScriptFile] {
        &self.files
    }

    pub fn combined(&self) -> &str {
        &self.combined
    }
}

/// Individual script source file.
#[derive(Debug, Clone)]
pub struct DefinitionScriptFile {
    pub path: PathBuf,
    pub contents: String,
}

/// Rectangle metadata parsed from `Picture=` in `DefCore.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `DFA_NONE` (C4Def.h:429): no procedure.
pub const DFA_NONE: i32 = -1;
/// `ActIdle` (C4Def.h:139): no next action.
pub const ACT_IDLE: i32 = -1;
/// `ActHold` (C4Def.h:140): hold the final phase.
pub const ACT_HOLD: i32 = -2;

/// `ProcedureName` table (C4Def.cpp:38-58); index = DFA_* value
/// (C4Def.h:430-447). Matching is case-SENSITIVE (`SEqual`,
/// C4Def.cpp:781).
pub const PROCEDURE_NAMES: [&str; 18] = [
    "WALK", "FLIGHT", "KNEEL", "SCALE", "HANGLE", "DIG", "SWIM", "THROW", "BRIDGE", "BUILD",
    "PUSH", "CHOP", "LIFT", "FLOAT", "ATTACH", "FIGHT", "CONNECT", "PULL",
];

/// Representation of `ActMap.txt`. Actions keep their file order and
/// duplicates, mirroring the C++ `C4ActionDef` array — `NextAction` indices
/// and first-match name lookups (`SetActionByName`) depend on it.
#[derive(Debug, Clone)]
pub struct ActionMap {
    pub default_action: Option<String>,
    pub actions: Vec<(String, ActionDefinition)>,
}

impl ActionMap {
    /// First action with the given name, like C++ `SetActionByName`'s forward
    /// scan (C4Object.cpp).
    pub fn get(&self, name: &str) -> Option<&ActionDefinition> {
        self.actions
            .iter()
            .find(|(action_name, _)| action_name == name)
            .map(|(_, action)| action)
    }
}

/// Action metadata used to construct runtime action specifications.
#[derive(Debug, Clone)]
pub struct ActionDefinition {
    pub procedure: Option<String>,
    /// Numeric DFA_* procedure from `CrossMapActMap` (C4Def.cpp:778-782);
    /// `DFA_NONE` when ProcedureName has no case-sensitive table match.
    pub procedure_index: i32,
    pub length: Option<u32>,
    pub next_action: Option<String>,
    /// Numeric next action from `CrossMapActMap` (C4Def.cpp:783-792):
    /// `ACT_IDLE`, `ACT_HOLD`, or an index into `ActionMap::actions`.
    pub next_action_index: i32,
    pub delay: Option<u32>,
    pub step: Option<u32>,
    pub phase_call: Option<String>,
    pub start_call: Option<String>,
    pub end_call: Option<String>,
    pub abort_call: Option<String>,
    pub no_other_action: bool,
    pub dig_free: Option<i32>,
    pub attach: u32,
    pub directions: Option<u32>,
    pub flip_dir: Option<u32>,
    pub facet: Option<ActionFacet>,
    pub reverse: bool,
    pub facet_base: bool,
    pub facet_top_face: bool,
    pub facet_target_stretch: bool,
}

impl Default for ActionDefinition {
    fn default() -> Self {
        Self {
            procedure: None,
            // C4ActionDef ctor: Procedure{DFA_NONE} (C4Def.cpp:62),
            // NextAction{ActIdle} (C4Def.h:154).
            procedure_index: DFA_NONE,
            length: None,
            next_action: None,
            next_action_index: ACT_IDLE,
            delay: None,
            step: None,
            phase_call: None,
            start_call: None,
            end_call: None,
            abort_call: None,
            no_other_action: false,
            dig_free: None,
            attach: 0,
            directions: None,
            flip_dir: None,
            facet: None,
            reverse: false,
            facet_base: false,
            facet_top_face: false,
            facet_target_stretch: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

#[derive(Debug, Error)]
pub enum DefinitionError {
    #[error("definition core `DefCore.txt` missing")]
    DefCoreMissing,
    #[error("definition core is missing required field `{0}`")]
    MissingDefCoreField(&'static str),
    #[error("definition core references unknown category flag `{0}`")]
    UnknownCategoryFlag(String),
    #[error("definition core references unknown line connect flag `{0}`")]
    UnknownLineConnectFlag(String),
    #[error("definition core category value `{0}` is not valid")]
    InvalidCategoryValue(String),
    #[error("definition core could not be parsed: {0}")]
    DefCoreParse(String),
    #[error("definition is missing script sources")]
    ScriptMissing,
    #[error("definition script `{path}` is not valid UTF-8")]
    ScriptEncoding {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },
    #[error("ActMap.txt could not be parsed: {0}")]
    ActMapParse(String),
    #[error(transparent)]
    Resources(#[from] GroupError),
}

fn parse_def_core(bytes: &[u8]) -> Result<DefCore, DefinitionError> {
    let text = String::from_utf8_lossy(bytes);
    let mut current_section: Option<String> = None;

    let mut id: Option<String> = None;
    let mut name: Option<String> = None;
    let mut category: i32 = 0;
    let mut category_set = false;
    let mut crew_member = false;
    let mut can_be_base = false;
    let mut object_value: i32 = 0;
    let mut object_mass: i32 = 0;
    let mut picture: Option<PictureRect> = None;
    let mut color_by_owner = false;
    let mut shape: Option<PictureRect> = None;
    let mut shape_width: Option<i32> = None;
    let mut shape_height: Option<i32> = None;
    let mut shape_offset: Option<(i32, i32)> = None;
    let mut solid_mask: Option<TargetRect> = None;
    let mut vertex_count: usize = 0;
    let mut vertex_x = [0i32; C4D_MAX_VERTEX];
    let mut vertex_y = [0i32; C4D_MAX_VERTEX];
    let mut vertex_cnat = [0u32; C4D_MAX_VERTEX];
    let mut vertex_friction = [0i32; C4D_MAX_VERTEX];
    let mut contact_density: i32 = C4M_SOLID;
    let mut contact_function_calls = false;
    let mut collection: Option<PictureRect> = None;
    let mut collection_limit: Option<u32> = None;
    let mut contact_incinerate: i32 = 0;
    let mut no_burn_decay = false;
    let mut no_breath = false;
    let mut grab = 0;
    let mut float_line = 0;
    let mut no_burn_damage = false;
    let mut burn_turn_to: Option<String> = None;
    let mut incomplete_activity = false;
    let mut physical = PhysicalInfo::default();
    let mut collectible = false;
    let mut constructable = false;
    let mut con_size_off: i32 = 0;
    let mut stretch_growth = false;
    let mut basement: i32 = 0;
    let mut rotateable: i32 = 0;
    let mut border_bound: i32 = 0;
    let mut upright_attach: u32 = 0;
    // NoStabilize (C4Def.cpp:402, default 0): opts out of C4Object::Stabilize.
    let mut no_stabilize = false;
    // Timer=/TimerCall= (C4Def.cpp:298-299): the per-object Def timer.
    let mut timer: i32 = 35;
    let mut timer_call: Option<String> = None;
    let mut components: Vec<DefComponent> = Vec::new();
    let mut line_connect: u32 = 0;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = Some(line[1..line.len() - 1].trim().to_ascii_lowercase());
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim();

        let section = current_section.as_deref().unwrap_or("defcore");

        if section == "physical" {
            physical.set_by_name(&key.to_ascii_lowercase(), parse_i32(value).unwrap_or(0));
            continue;
        }

        if section != "defcore" {
            continue;
        }

        match key.to_ascii_lowercase().as_str() {
            "id" => {
                if !value.is_empty() {
                    id = Some(value.to_string());
                }
            }
            "name" => {
                if !value.is_empty() {
                    name = Some(value.to_string());
                }
            }
            "value" => {
                object_value = parse_i32(value).unwrap_or(0);
            }
            "mass" => {
                object_mass = parse_i32(value).unwrap_or(0).max(0);
            }
            "category" => {
                category = parse_category(value)?;
                category_set = true;
            }
            "crewmember" => {
                crew_member = parse_bool(value);
            }
            "canbebase" => {
                can_be_base = parse_bool(value);
            }
            "picture" => {
                if let Some(rect) = parse_rect(value) {
                    picture = Some(rect);
                }
            }
            "colorbyowner" => {
                color_by_owner = parse_i32(value).unwrap_or(0) != 0;
            }
            "shape" => {
                shape = parse_rect(value);
            }
            // C4Def::CompileFunc maps Width/Height/Offset straight into
            // Shape.Wdt/Hgt/x/y (C4Def.cpp) — CR DefCores never carry a
            // combined Shape= key.
            "width" => {
                shape_width = parse_i32(value);
            }
            "height" => {
                shape_height = parse_i32(value);
            }
            "offset" => {
                let mut parts = value.split(',').map(str::trim);
                shape_offset = Some((
                    parts.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    parts.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                ));
            }
            "solidmask" => {
                solid_mask =
                    parse_target_rect(value).filter(|rect| rect.width > 0 && rect.height > 0);
            }
            "vertices" => {
                vertex_count = parse_i32(value)
                    .unwrap_or(0)
                    .clamp(0, C4D_MAX_VERTEX as i32) as usize;
            }
            "vertexx" => {
                fill_i32_array(value, &mut vertex_x);
            }
            "vertexy" => {
                fill_i32_array(value, &mut vertex_y);
            }
            "vertexcnat" => {
                fill_u32_array(value, &mut vertex_cnat);
            }
            "vertexfriction" => {
                fill_i32_array(value, &mut vertex_friction);
            }
            "contactdensity" => {
                contact_density = parse_i32(value).unwrap_or(C4M_SOLID);
            }
            "contactcalls" => {
                contact_function_calls = parse_bool(value);
            }
            "collection" => {
                collection = parse_rect(value).filter(|rect| rect.width > 0 && rect.height > 0);
            }
            "contactincinerate" => {
                contact_incinerate = parse_i32(value).unwrap_or(0).max(0);
            }
            "noburndecay" => {
                no_burn_decay = parse_bool(value);
            }
            "nobreath" => {
                no_breath = parse_bool(value);
            }
            "float" => {
                float_line = parse_i32(value).unwrap_or(0);
            }
            "grab" => {
                grab = parse_i32(value).unwrap_or(0).max(0);
            }
            "noburndamage" => {
                no_burn_damage = parse_bool(value);
            }
            "burnturnto" => {
                if !value.is_empty() {
                    burn_turn_to = Some(value.to_string());
                }
            }
            "incompleteactivity" => {
                incomplete_activity = parse_bool(value);
            }
            "collectionlimit" => {
                collection_limit = match parse_i32(value) {
                    Some(limit) if limit > 0 => Some(limit as u32),
                    _ => None,
                };
            }
            "collectible" => {
                collectible = parse_bool(value);
            }
            "construction" => {
                constructable = parse_bool(value);
            }
            "consizeoff" => {
                con_size_off = parse_i32(value).unwrap_or(0).max(0);
            }
            "stretchgrowth" => {
                stretch_growth = parse_bool(value);
            }
            "basement" => {
                basement = parse_i32(value).unwrap_or(0).max(0);
            }
            "rotate" => {
                rotateable = parse_i32(value).unwrap_or(0).max(0);
            }
            "borderbound" => {
                border_bound = parse_i32(value).unwrap_or(0).max(0);
            }
            "uprightattach" => {
                upright_attach = parse_i32(value).unwrap_or(0).max(0) as u32;
            }
            "nostabilize" => {
                no_stabilize = parse_bool(value);
            }
            "timer" => {
                timer = parse_i32(value).unwrap_or(35);
            }
            "timercall" => {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    timer_call = Some(trimmed.to_string());
                }
            }
            "components" => {
                components = parse_components(value);
            }
            "lineconnect" => {
                line_connect = parse_line_connect(value)?;
            }
            _ => {}
        }
    }

    let id = id.ok_or(DefinitionError::MissingDefCoreField("id"))?;
    if !category_set {
        // Preserve compatibility with the C++ engine where unspecified category defaults to 0.
        category = 0;
    }

    let vertices = (0..vertex_count)
        .map(|idx| DefVertex {
            x: vertex_x[idx],
            y: vertex_y[idx],
            cnat: vertex_cnat[idx],
            friction: vertex_friction[idx],
        })
        .collect();

    Ok(DefCore {
        id,
        name,
        category,
        crew_member,
        value: object_value,
        mass: object_mass,
        picture,
        color_by_owner,
        shape: shape.or_else(|| {
            (shape_width.is_some() || shape_height.is_some() || shape_offset.is_some()).then(
                || {
                    let (x, y) = shape_offset.unwrap_or((0, 0));
                    PictureRect {
                        x,
                        y,
                        width: shape_width.unwrap_or(0),
                        height: shape_height.unwrap_or(0),
                    }
                },
            )
        }),
        solid_mask,
        vertices,
        contact_density,
        contact_function_calls,
        collection,
        collection_limit,
        contact_incinerate,
        no_burn_decay,
        no_breath,
        grab,
        float_line,
        no_burn_damage,
        burn_turn_to,
        incomplete_activity,
        physical,
        collectible,
        constructable,
        con_size_off,
        stretch_growth,
        basement,
        rotateable,
        border_bound,
        upright_attach,
        no_stabilize,
        timer,
        timer_call,
        components,
        line_connect,
        can_be_base,
    })
}

fn parse_components(value: &str) -> Vec<DefComponent> {
    value
        .split([';', ',', ' '])
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (id_part, count_part) = match trimmed.find([':', '=']) {
                Some(idx) => {
                    let (lhs, rhs) = trimmed.split_at(idx);
                    (lhs.trim(), Some(rhs[1..].trim()))
                }
                None => (trimmed, None),
            };
            if id_part.is_empty() {
                return None;
            }
            let id = id_part.to_ascii_uppercase();
            let count = count_part
                .and_then(|raw| raw.parse::<i32>().ok())
                .unwrap_or(1)
                .max(0) as u32;
            let count = if count == 0 { 1 } else { count };
            Some(DefComponent { id, count })
        })
        .collect()
}

fn normalize_line_connect_token(token: &str) -> String {
    token.trim().replace([' ', '_'], "").to_ascii_lowercase()
}

fn parse_line_connect(value: &str) -> Result<u32, DefinitionError> {
    let mut flags = 0u32;
    for token in value.split(['|', ',', ';']) {
        let normalized = normalize_line_connect_token(token);
        if normalized.is_empty() {
            continue;
        }
        let bit = match normalized.as_str() {
            "c4dpowerinput" => 1,
            "c4dpoweroutput" => 1 << 1,
            "c4dliquidinput" => 1 << 2,
            "c4dliquidoutput" => 1 << 3,
            "c4dpowergenerator" => 1 << 4,
            "c4dpowerconsumer" => 1 << 5,
            "c4dliquidpump" => 1 << 6,
            "c4dconnectrope" => 1 << 7,
            "c4denergyholder" => 1 << 8,
            other => {
                return Err(DefinitionError::UnknownLineConnectFlag(other.to_string()));
            }
        };
        flags |= bit;
    }
    Ok(flags)
}

fn load_scripts(group: &Group) -> Result<DefinitionScript, DefinitionError> {
    let mut files: Vec<DefinitionScriptFile> = Vec::new();
    collect_script_files(group, Path::new(""), &mut files)?;

    // Allow definitions without scripts (graphics-only, data-only, etc.)
    // This matches C++ behavior which doesn't require scripts
    if files.is_empty() {
        return Ok(DefinitionScript {
            files,
            combined: String::new(),
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut combined = String::new();
    for file in &files {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("//#file ");
        combined.push_str(&file.path.to_string_lossy());
        combined.push('\n');
        combined.push_str(&file.contents);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }

    Ok(DefinitionScript { files, combined })
}

fn collect_script_files(
    group: &Group,
    prefix: &Path,
    files: &mut Vec<DefinitionScriptFile>,
) -> Result<(), DefinitionError> {
    let entries = group.entries().map_err(DefinitionError::Resources)?;
    for entry in entries {
        let mut relative_path = PathBuf::from(prefix);
        relative_path.push(&entry.relative_path);
        if entry.is_directory {
            let child = group
                .open_child(&entry.relative_path)
                .map_err(DefinitionError::Resources)?;
            if child.exists("DefCore.txt") {
                continue;
            }
            collect_script_files(&child, &relative_path, files)?;
            continue;
        }
        if !is_script_file(&entry.relative_path) {
            continue;
        }
        let data = group
            .read_file(&entry.relative_path)
            .map_err(DefinitionError::Resources)?;
        // Legacy C4Script files use Windows-1252 encoding (superset of ISO-8859-1)
        // Convert to UTF-8 to ensure correct byte indices for position tracking
        let (contents, _, _) = encoding_rs::WINDOWS_1252.decode(&data);
        files.push(DefinitionScriptFile {
            path: relative_path,
            contents: contents.into_owned(),
        });
    }
    Ok(())
}

fn is_script_file(path: &Path) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    if extension.eq_ignore_ascii_case("c") {
        return true;
    }
    false
}

fn parse_act_map(bytes: &[u8]) -> Result<ActionMap, DefinitionError> {
    let text = String::from_utf8_lossy(bytes);
    let mut default_action: Option<String> = None;
    let mut actions: Vec<(String, ActionDefinition)> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_definition = ActionDefinition::default();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            if let Some(name) = current_name.take() {
                actions.push((name, current_definition));
            }
            current_definition = ActionDefinition::default();
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim();

        if key.eq_ignore_ascii_case("Name") {
            if !value.is_empty() {
                if let Some(name) = current_name.replace(value.to_string()) {
                    actions.push((name, current_definition));
                    current_definition = ActionDefinition::default();
                }
            }
            continue;
        }

        if key.eq_ignore_ascii_case("Default") {
            if !value.is_empty() {
                default_action = Some(value.to_string());
            }
            continue;
        }

        match key.to_ascii_lowercase().as_str() {
            "procedure" => {
                if !value.is_empty() {
                    current_definition.procedure = Some(value.to_string());
                }
            }
            "length" => {
                current_definition.length = parse_u32(value);
            }
            "nextaction" => {
                if !value.is_empty() {
                    current_definition.next_action = Some(value.to_string());
                }
            }
            "delay" => {
                current_definition.delay = parse_u32(value);
            }
            "step" => {
                current_definition.step = parse_u32(value);
            }
            "phasecall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.phase_call = Some(value.to_string());
                }
            }
            "startcall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.start_call = Some(value.to_string());
                }
            }
            "endcall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.end_call = Some(value.to_string());
                }
            }
            "abortcall" => {
                if !value.is_empty() && !value.eq_ignore_ascii_case("None") {
                    current_definition.abort_call = Some(value.to_string());
                }
            }
            "nootheraction" => {
                current_definition.no_other_action = parse_bool(value);
            }
            "digfree" => {
                current_definition.dig_free = parse_i32(value);
            }
            "attach" => {
                current_definition.attach = parse_i32(value).unwrap_or(0).max(0) as u32;
            }
            "directions" => {
                current_definition.directions = parse_u32(value);
            }
            "flipdir" => {
                current_definition.flip_dir = parse_u32(value);
            }
            "facet" => {
                current_definition.facet = parse_action_facet(value);
            }
            "reverse" => {
                current_definition.reverse = parse_bool(value);
            }
            "facetbase" => {
                current_definition.facet_base = parse_bool(value);
            }
            "facettopface" => {
                current_definition.facet_top_face = parse_bool(value);
            }
            "facettargetstretch" => {
                current_definition.facet_target_stretch = parse_bool(value);
            }
            _ => {}
        }
    }

    if let Some(name) = current_name {
        actions.push((name, current_definition));
    }

    if actions.is_empty() && default_action.is_some() {
        return Err(DefinitionError::ActMapParse(
            "ActMap.txt declared a default action but no actions".into(),
        ));
    }

    cross_map_act_map(&mut actions);

    Ok(ActionMap {
        default_action,
        actions,
    })
}

/// `C4Def::CrossMapActMap` (C4Def.cpp:773-799): resolve procedure names to
/// DFA_* indices (case-sensitive against `PROCEDURE_NAMES`) and next-action
/// names to indices ("Hold" case-insensitively to `ACT_HOLD`; the C++
/// overwrite loop makes the last duplicate win). The `*Call="None"` clearing
/// from lines 794-797 already happens during parsing.
fn cross_map_act_map(actions: &mut [(String, ActionDefinition)]) {
    let names: Vec<String> = actions.iter().map(|(name, _)| name.clone()).collect();
    for (_, action) in actions.iter_mut() {
        action.procedure_index = DFA_NONE;
        if let Some(procedure) = &action.procedure {
            for (index, table_name) in PROCEDURE_NAMES.iter().enumerate() {
                if procedure == table_name {
                    action.procedure_index = index as i32;
                }
            }
        }
        action.next_action_index = ACT_IDLE;
        if let Some(next) = &action.next_action {
            if next.eq_ignore_ascii_case("Hold") {
                action.next_action_index = ACT_HOLD;
            } else {
                for (index, name) in names.iter().enumerate() {
                    if next == name {
                        action.next_action_index = index as i32;
                    }
                }
            }
        }
    }
}

fn load_definition_picture(group: &Group, core: &DefCore) -> Option<GraphicsImage> {
    let path = find_picture_entry(group).ok().flatten()?;
    let data = group.read_file(&path).ok()?;
    let image = image::load_from_memory(&data).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }

    let (crop_x, crop_y, crop_w, crop_h) = match core.picture {
        Some(rect) => normalize_crop(rect, width, height).unwrap_or((0, 0, width, height)),
        None => (0, 0, width, height),
    };

    let pixels = extract_rgba_region(&image, crop_x, crop_y, crop_w, crop_h);
    Some(GraphicsImage::new(crop_w, crop_h, pixels))
}

fn find_picture_entry(group: &Group) -> Result<Option<PathBuf>, GroupError> {
    const PRIORITY_FILES: [&str; 4] = ["Graphics32.png", "Graphics.png", "Picture.png", "Icon.png"];
    for candidate in PRIORITY_FILES {
        if group.exists(candidate) {
            return Ok(Some(PathBuf::from(candidate)));
        }
    }

    const PRIORITY_GROUPS: [&str; 3] = ["Graphics.ocg", "Graphics.c4d", "Graphics.c4g"];
    for candidate in PRIORITY_GROUPS {
        if let Ok(child) = group.open_child(candidate) {
            if let Some(found) = find_picture_entry(&child)? {
                let mut combined = PathBuf::from(candidate);
                combined.push(found);
                return Ok(Some(combined));
            }
        }
    }

    find_picture_entry_recursive(group, PathBuf::new())
}

fn find_picture_entry_recursive(
    group: &Group,
    base: PathBuf,
) -> Result<Option<PathBuf>, GroupError> {
    for entry in group.entries()? {
        let mut combined = base.clone();
        combined.push(&entry.relative_path);
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            if let Some(found) = find_picture_entry_recursive(&child, combined.clone())? {
                return Ok(Some(found));
            }
        } else if is_image_path(&entry.relative_path) {
            return Ok(Some(combined));
        }
    }
    Ok(None)
}

fn load_definition_graphics(
    group: &Group,
    color_by_owner: bool,
) -> (
    Option<GraphicsImage>,
    Option<ColorByOwnerMask>,
    HashMap<String, DefinitionGraphicsVariant>,
) {
    let mut candidates = collect_graphics_entries(group).unwrap_or_default();
    if candidates.is_empty() {
        return (None, None, HashMap::new());
    }

    let base_path = select_base_graphics(&candidates);
    let mut base_image = None;
    let mut base_mask = None;
    let mut additional = HashMap::new();

    if let Some(base_path) = base_path.clone() {
        if let Some((image, mask)) = load_graphics_entry(group, &base_path, color_by_owner) {
            base_image = Some(image);
            base_mask = mask;
        }
    }

    // Remove the base candidate so it does not get processed as additional graphics.
    if let Some(base_path) = &base_path {
        candidates.retain(|path| path != base_path);
    }

    for path in candidates {
        if let Some((image, mask)) = load_graphics_entry(group, &path, color_by_owner) {
            if let Some(name) = derive_variant_name(&path) {
                if !name.is_empty() {
                    let key = normalize_variant_key(&name);
                    additional
                        .entry(key)
                        .or_insert_with(|| DefinitionGraphicsVariant {
                            name,
                            image,
                            color_by_owner_mask: mask,
                        });
                }
            }
        }
    }

    (base_image, base_mask, additional)
}

fn collect_graphics_entries(group: &Group) -> Result<Vec<PathBuf>, GroupError> {
    let mut entries = Vec::new();
    collect_graphics_entries_recursive(group, PathBuf::new(), false, &mut entries)?;
    Ok(entries)
}

fn collect_graphics_entries_recursive(
    group: &Group,
    base: PathBuf,
    in_graphics_dir: bool,
    entries: &mut Vec<PathBuf>,
) -> Result<(), GroupError> {
    for entry in group.entries()? {
        let mut combined = base.clone();
        combined.push(&entry.relative_path);
        let name_lower = entry
            .relative_path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let next_in_graphics_dir =
            in_graphics_dir || name_lower.contains("graphics") || name_lower.starts_with("gfx");
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            collect_graphics_entries_recursive(
                &child,
                combined.clone(),
                next_in_graphics_dir,
                entries,
            )?;
        } else if (next_in_graphics_dir && is_image_path(&entry.relative_path))
            && (name_lower.starts_with("graphics") || name_lower.starts_with("gfx"))
        {
            entries.push(combined);
        }
    }
    Ok(())
}

fn select_base_graphics(paths: &[PathBuf]) -> Option<PathBuf> {
    const PRIORITY: [&str; 4] = [
        "graphics32.png",
        "graphics64.png",
        "graphics.png",
        "graphics.bmp",
    ];

    for name in PRIORITY {
        let mut best: Option<&PathBuf> = None;
        for path in paths {
            if path
                .file_name()
                .and_then(|file| file.to_str())
                .map(|file| file.eq_ignore_ascii_case(name))
                .unwrap_or(false)
            {
                best = match best {
                    Some(existing) => {
                        let existing_depth = existing.components().count();
                        let path_depth = path.components().count();
                        if path_depth < existing_depth {
                            Some(path)
                        } else {
                            Some(existing)
                        }
                    }
                    None => Some(path),
                };
            }
        }
        if let Some(best) = best {
            return Some(best.clone());
        }
    }

    paths.first().cloned()
}

fn load_graphics_entry(
    group: &Group,
    path: &Path,
    color_by_owner: bool,
) -> Option<(GraphicsImage, Option<ColorByOwnerMask>)> {
    let data = group.read_file(path).ok()?;
    let mut image = image::load_from_memory(&data).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return None;
    }

    let mask = if color_by_owner {
        load_or_generate_color_by_owner_mask(group, path, &mut image)
    } else {
        None
    };

    Some((GraphicsImage::new(width, height, image.into_raw()), mask))
}

fn strip_graphics_prefix(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    if let Some(stripped) = lower.strip_prefix("graphics") {
        let prefix_len = name.len() - stripped.len();
        return Some(&name[prefix_len..]);
    }
    if let Some(stripped) = lower.strip_prefix("gfx") {
        let prefix_len = name.len() - stripped.len();
        return Some(&name[prefix_len..]);
    }
    None
}

fn derive_variant_name(path: &Path) -> Option<String> {
    let file_stem = path.file_stem()?.to_string_lossy();
    if let Some(stripped) = strip_graphics_prefix(&file_stem) {
        if !stripped.is_empty() {
            return Some(stripped.to_string());
        }
    }

    let mut current = path.parent();
    while let Some(parent) = current {
        if let Some(stem) = parent.file_stem().and_then(|s| s.to_str()) {
            if !stem.is_empty()
                && !stem.eq_ignore_ascii_case("graphics")
                && !stem.eq_ignore_ascii_case("gfx")
            {
                return Some(stem.to_string());
            }
        }
        current = parent.parent();
    }
    None
}

fn normalize_variant_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn load_or_generate_color_by_owner_mask(
    group: &Group,
    graphics_path: &Path,
    image: &mut image::RgbaImage,
) -> Option<ColorByOwnerMask> {
    if let Some(overlay) = load_color_by_owner_overlay(group, graphics_path) {
        return extract_mask_from_overlay(&overlay, image);
    }
    generate_color_by_owner_mask(image)
}

fn load_color_by_owner_overlay(group: &Group, graphics_path: &Path) -> Option<image::RgbaImage> {
    let mut candidates = Vec::new();

    if let Some(parent) = graphics_path.parent() {
        if let Some(name) = graphics_path.file_name().and_then(|n| n.to_str()) {
            if let Some(stripped) = name.strip_prefix("Graphics") {
                if !stripped.is_empty() {
                    let mut candidate = parent.to_path_buf();
                    candidate.push(format!("Overlay{}", stripped));
                    candidates.push(candidate);
                }
            }
        }
        let mut overlay_name = parent.to_path_buf();
        overlay_name.push("Overlay.png");
        candidates.push(overlay_name);
    }

    candidates.push(PathBuf::from("Overlay.png"));

    for candidate in candidates {
        if let Ok(data) = group.read_file(&candidate) {
            if let Ok(image) = image::load_from_memory(&data) {
                return Some(image.into_rgba8());
            }
        }
    }

    None
}

fn extract_mask_from_overlay(
    overlay: &image::RgbaImage,
    base: &mut image::RgbaImage,
) -> Option<ColorByOwnerMask> {
    let (width, height) = base.dimensions();
    if overlay.dimensions() != (width, height) {
        return None;
    }

    let mut pixels = vec![0u8; (width * height) as usize];
    let mut has_mask = false;
    for y in 0..height {
        for x in 0..width {
            let overlay_pixel = overlay.get_pixel(x, y);
            let mask_value = overlay_pixel[0];
            if mask_value == 0 {
                continue;
            }
            let idx = (y * width + x) as usize;
            pixels[idx] = mask_value;
            has_mask = true;
            let base_pixel = base.get_pixel_mut(x, y);
            let alpha = base_pixel[3];
            *base_pixel = image::Rgba([255, 255, 255, alpha]);
        }
    }

    if has_mask {
        Some(ColorByOwnerMask {
            width,
            height,
            pixels,
        })
    } else {
        None
    }
}

fn generate_color_by_owner_mask(image: &mut image::RgbaImage) -> Option<ColorByOwnerMask> {
    let (width, height) = image.dimensions();
    let mut pixels = vec![0u8; (width * height) as usize];
    let mut has_mask = false;

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let pixel = image.get_pixel_mut(x, y);
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            let a = pixel[3];
            let dw = u32::from(a) << 24 | u32::from(b) << 16 | u32::from(g) << 8 | u32::from(r);
            if let Some(mask_value) = detect_color_by_owner(dw) {
                pixels[idx] = mask_value;
                pixel[0] = 255;
                pixel[1] = 255;
                pixel[2] = 255;
                pixel[3] = a;
                has_mask = true;
            }
        }
    }

    if has_mask {
        Some(ColorByOwnerMask {
            width,
            height,
            pixels,
        })
    } else {
        None
    }
}

fn detect_color_by_owner(dw_clr: u32) -> Option<u8> {
    const RANGE: i32 = 255;
    const HLSMAX: i32 = RANGE;
    const RGBMAX: i32 = 255;

    let r = ((dw_clr >> 16) & 0xff) as i32;
    let g = ((dw_clr >> 8) & 0xff) as i32;
    let b = (dw_clr & 0xff) as i32;
    let c_max = r.max(g).max(b);
    let c_min = r.min(g).min(b);

    let l = ((c_max + c_min) * HLSMAX + RGBMAX) / (2 * RGBMAX);
    let mut h;
    let s;
    if c_max == c_min {
        s = 0;
        h = (HLSMAX * 2) / 3;
    } else {
        if l <= (HLSMAX / 2) {
            s = ((c_max - c_min) * HLSMAX + ((c_max + c_min) / 2)) / (c_max + c_min);
        } else {
            s = ((c_max - c_min) * HLSMAX + ((2 * RGBMAX - c_max - c_min) / 2))
                / (2 * RGBMAX - c_max - c_min);
        }

        let rdelta = ((c_max - r) * (HLSMAX / 6) + ((c_max - c_min) / 2)) / (c_max - c_min);
        let gdelta = ((c_max - g) * (HLSMAX / 6) + ((c_max - c_min) / 2)) / (c_max - c_min);
        let bdelta = ((c_max - b) * (HLSMAX / 6) + ((c_max - c_min) / 2)) / (c_max - c_min);

        if r == c_max {
            h = bdelta - gdelta;
        } else if g == c_max {
            h = (HLSMAX / 3) + rdelta - bdelta;
        } else {
            h = (2 * HLSMAX) / 3 + gdelta - rdelta;
        }
        if h < 0 {
            h += HLSMAX;
        }
        if h > HLSMAX {
            h -= HLSMAX;
        }
    }

    if !(145..=175).contains(&h) || s <= 100 {
        return None;
    }

    Some((dw_clr & 0xff) as u8)
}

fn is_image_path(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "bmp" | "jpg" | "jpeg" | "tga"
        ),
        None => false,
    }
}

fn normalize_crop(
    rect: PictureRect,
    image_width: u32,
    image_height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let width = rect.width.max(0) as u32;
    let height = rect.height.max(0) as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut x = rect.x.max(0) as u32;
    let mut y = rect.y.max(0) as u32;
    if x >= image_width || y >= image_height {
        return None;
    }
    if x + width > image_width {
        x = x.min(image_width.saturating_sub(1));
    }
    if y + height > image_height {
        y = y.min(image_height.saturating_sub(1));
    }
    let crop_width = width.min(image_width - x);
    let crop_height = height.min(image_height - y);
    Some((x, y, crop_width.max(1), crop_height.max(1)))
}

fn extract_rgba_region(
    image: &image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let stride = image.width();
    let mut output = Vec::with_capacity((width * height * 4) as usize);
    for row in y..(y + height) {
        let row_start = ((row * stride) + x) as usize * 4;
        let row_end = row_start + (width as usize * 4);
        output.extend_from_slice(&image.as_raw()[row_start..row_end]);
    }
    output
}

fn parse_category(value: &str) -> Result<i32, DefinitionError> {
    let mut result: i32 = 0;
    if value.is_empty() {
        return Ok(result);
    }

    for token in value.split(|c: char| c == '|' || c == '+' || c == ',' || c.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(flag) = category_flag(token) {
            result |= flag;
            continue;
        }
        if token.starts_with("C4D_") {
            return Err(DefinitionError::UnknownCategoryFlag(token.to_string()));
        }
        let parsed = parse_i32(token)
            .ok_or_else(|| DefinitionError::InvalidCategoryValue(token.to_string()))?;
        result |= parsed;
    }

    Ok(result)
}

fn category_flag(token: &str) -> Option<i32> {
    let normalized = token.trim();
    for (name, value) in CATEGORY_FLAGS {
        if normalized.eq_ignore_ascii_case(name) {
            return Some(*value);
        }
    }
    None
}

fn parse_bool(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    matches!(lower.as_str(), "1" | "true" | "yes" | "on")
}

fn parse_u32(value: &str) -> Option<u32> {
    parse_i64(value).and_then(|num| if num < 0 { None } else { Some(num as u32) })
}

fn fill_i32_array(value: &str, target: &mut [i32]) {
    for slot in target.iter_mut() {
        *slot = 0;
    }
    for (slot, parsed) in target.iter_mut().zip(parse_int_array(value)) {
        *slot = parsed;
    }
}

fn fill_u32_array(value: &str, target: &mut [u32]) {
    for slot in target.iter_mut() {
        *slot = 0;
    }
    for (slot, parsed) in target.iter_mut().zip(parse_int_array(value)) {
        *slot = parsed.max(0) as u32;
    }
}

fn parse_int_array(value: &str) -> impl Iterator<Item = i32> + '_ {
    value
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .filter_map(parse_i32)
}

fn parse_rect(value: &str) -> Option<PictureRect> {
    let mut parts = value
        .split([',', ';'])
        .map(|part| part.trim())
        .filter(|part| !part.is_empty());
    let x = parse_i32(parts.next()?)?;
    let y = parse_i32(parts.next()?)?;
    let width = parse_i32(parts.next()?)?;
    let height = parse_i32(parts.next()?)?;
    Some(PictureRect {
        x,
        y,
        width,
        height,
    })
}

fn parse_target_rect(value: &str) -> Option<TargetRect> {
    let mut parts = value
        .split([',', ';'])
        .map(|part| part.trim())
        .filter(|part| !part.is_empty());
    let x = parse_i32(parts.next()?)?;
    let y = parse_i32(parts.next()?)?;
    let width = parse_i32(parts.next()?)?;
    let height = parse_i32(parts.next()?)?;
    let target_x = parse_i32(parts.next()?)?;
    let target_y = parse_i32(parts.next()?)?;
    Some(TargetRect {
        x,
        y,
        width,
        height,
        target_x,
        target_y,
    })
}

fn parse_i32(value: &str) -> Option<i32> {
    parse_i64(value).and_then(|num| num.try_into().ok())
}

fn parse_i64(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("0x") {
        i64::from_str_radix(rest, 16).ok()
    } else if let Some(rest) = trimmed.strip_prefix("$") {
        i64::from_str_radix(rest, 16).ok()
    } else if let Some(rest) = trimmed.strip_prefix("0b") {
        i64::from_str_radix(rest, 2).ok()
    } else {
        trimmed.parse().ok()
    }
}

fn parse_action_facet(value: &str) -> Option<ActionFacet> {
    let parts: Vec<_> = value
        .split([',', ';'])
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 4 && parts.len() != 6 {
        return None;
    }
    let mut numbers = Vec::with_capacity(parts.len());
    for part in parts {
        numbers.push(parse_i32(part)?);
    }
    let x = numbers[0];
    let y = numbers[1];
    let width = numbers[2];
    let height = numbers[3];
    let (target_x, target_y) = if numbers.len() == 6 {
        (numbers[4], numbers[5])
    } else {
        (0, 0)
    };
    Some(ActionFacet {
        x,
        y,
        width,
        height,
        target_x,
        target_y,
    })
}

const CATEGORY_FLAGS: &[(&str, i32)] = &[
    ("C4D_None", 0),
    ("C4D_All", !0),
    ("C4D_StaticBack", 1 << 0),
    ("C4D_Structure", 1 << 1),
    ("C4D_Vehicle", 1 << 2),
    ("C4D_Living", 1 << 3),
    ("C4D_Object", 1 << 4),
    (
        "C4D_SortLimit",
        (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4),
    ),
    ("C4D_Goal", 1 << 5),
    ("C4D_Environment", 1 << 6),
    ("C4D_SelectBuilding", 1 << 7),
    ("C4D_SelectVehicle", 1 << 8),
    ("C4D_SelectMaterial", 1 << 9),
    ("C4D_SelectKnowledge", 1 << 10),
    ("C4D_SelectHomebase", 1 << 11),
    ("C4D_SelectAnimal", 1 << 12),
    ("C4D_SelectNest", 1 << 13),
    ("C4D_SelectInEarth", 1 << 14),
    ("C4D_SelectVegetation", 1 << 15),
    ("C4D_TradeLiving", 1 << 16),
    ("C4D_Magic", 1 << 17),
    ("C4D_CrewMember", 1 << 18),
    ("C4D_Rule", 1 << 19),
    ("C4D_Background", 1 << 20),
    ("C4D_Parallax", 1 << 21),
    ("C4D_MouseSelect", 1 << 22),
    ("C4D_Foreground", 1 << 23),
    ("C4D_MouseIgnore", 1 << 24),
    ("C4D_IgnoreFoW", 1 << 25),
    ("C4D_BackgroundOrForeground", (1 << 20) | (1 << 23)),
];

#[cfg(test)]
mod tests {

    // C4Def::CompileFunc maps Width/Height/Offset into Shape.Wdt/Hgt/x/y
    // (C4Def.cpp) — CR DefCores carry no combined Shape= key. The GoldRush
    // wagon COAC (Width=48 Height=40 Offset=-24,-20) needs this rect for
    // the NewObject bottom-growth adjust.
    #[test]
    fn defcore_width_height_offset_compose_the_shape_rect() {
        let core = parse_def_core(
            b"[DefCore]\nid=COAC\nName=Coach\nWidth=48\nHeight=40\nOffset=-24,-20\n",
        )
        .expect("core parses");
        let shape = core.shape.expect("shape synthesized");
        assert_eq!((shape.x, shape.y, shape.width, shape.height), (-24, -20, 48, 40));
    }
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn parse_def_core_basic_fields() {
        let data = br#"
            [DefCore]
            id=CLNK
            Name=Clonk
            Category=C4D_Living|C4D_Object
            CrewMember=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.id, "CLNK");
        assert_eq!(parsed.name.as_deref(), Some("Clonk"));
        assert_eq!(parsed.category, (1 << 3) | (1 << 4));
        assert!(parsed.crew_member);
        assert_eq!(parsed.collection, None);
        assert_eq!(parsed.collection_limit, None);
        assert!(!parsed.collectible);
    }

    #[test]
    fn load_definition_with_scripts_and_actions() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Example.ocd");
        fs::create_dir(&def_dir).unwrap();
        fs::write(
            def_dir.join("DefCore.txt"),
            br#"[DefCore]
id=EXMP
Name=Example
Category=C4D_Object
CrewMember=0
"#,
        )
        .unwrap();
        fs::write(def_dir.join("Script.c"), b"func Initialize() {}\n").unwrap();
        fs::write(
            def_dir.join("ActMap.txt"),
            br#"
[Action]
Name=Idle
Procedure=Walk
Length=20
NextAction=Idle
StartCall=OnIdleStart
EndCall=OnIdleEnd
"#,
        )
        .unwrap();

        let group = Group::open(&def_dir).unwrap();
        let def = Definition::load(&group).expect("definition load succeeds");
        assert_eq!(def.core.id, "EXMP");
        assert_eq!(def.core.name.as_deref(), Some("Example"));
        assert_eq!(def.core.category, 1 << 4);
        assert!(!def.core.crew_member);
        assert_eq!(def.script.files.len(), 1);
        assert!(def.script.combined.contains("Initialize"));
        let action_map = def.action_map.expect("action map present");
        assert!(action_map.default_action.is_none());
        let idle = action_map.get("Idle").expect("idle action present");
        assert_eq!(idle.procedure.as_deref(), Some("Walk"));
        assert_eq!(idle.length, Some(20));
        assert_eq!(idle.next_action.as_deref(), Some("Idle"));
        assert_eq!(idle.start_call.as_deref(), Some("OnIdleStart"));
        assert_eq!(idle.end_call.as_deref(), Some("OnIdleEnd"));
    }

    #[test]
    fn cross_map_act_map_maps_procedures_and_next_actions_like_cpp() {
        // C4Def::CrossMapActMap (C4Def.cpp:773-799): Procedure maps
        // case-SENSITIVELY against the uppercase ProcedureName table
        // (C4Def.cpp:38-58) with DFA_NONE fallback; NextAction maps "Hold"
        // case-insensitively to ActHold, otherwise case-SENSITIVELY against
        // the action names — the overwrite loop means the LAST duplicate
        // wins; no match leaves the ActIdle default.
        let data = br#"
[Action]
Name=Walk
Procedure=WALK
NextAction=hOLD

[Action]
Name=Fall
Procedure=walk
NextAction=Walk

[Action]
Name=Spin
NextAction=spin

[Action]
Name=Dup

[Action]
Name=Dup

[Action]
Name=Ref
Procedure=FLIGHT
NextAction=Dup
"#;
        let map = parse_act_map(data).expect("act map parsed");
        // file order preserved, duplicates kept (C++ array semantics)
        let order: Vec<&str> = map.actions.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(order, ["Walk", "Fall", "Spin", "Dup", "Dup", "Ref"]);

        let walk = map.get("Walk").expect("walk present");
        assert_eq!(walk.procedure_index, 0, "WALK → DFA_WALK");
        assert_eq!(walk.next_action_index, ACT_HOLD, "Hold is case-insensitive");

        let fall = map.get("Fall").expect("fall present");
        assert_eq!(
            fall.procedure_index, DFA_NONE,
            "lowercase 'walk' does not match the case-sensitive table"
        );
        assert_eq!(fall.next_action_index, 0, "NextAction=Walk → index 0");

        let spin = map.get("Spin").expect("spin present");
        assert_eq!(
            spin.next_action_index, ACT_IDLE,
            "case-sensitive miss leaves ActIdle"
        );

        let reference = map.get("Ref").expect("ref present");
        assert_eq!(reference.procedure_index, 1, "FLIGHT → DFA_FLIGHT");
        assert_eq!(
            reference.next_action_index, 4,
            "last duplicate wins (C4Def.cpp:789-791 overwrite loop)"
        );
    }

    #[test]
    fn parse_def_core_physical_section() {
        // C4PhysicalInfo via the [Physical] DefCore section
        // (C4Def.cpp:459-460, name map C4InfoCore.cpp:181-205); defaults are
        // all zero (C4InfoCore.cpp:239-242).
        let data = br#"
            [DefCore]
            id=CLNK
            Name=Clonk

            [Physical]
            Energy=50000
            Walk=35000
            Fight=20000
            CanScale=1
            CorrosionResist=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.physical.energy, 50_000);
        assert_eq!(parsed.physical.walk, 35_000);
        assert_eq!(parsed.physical.fight, 20_000);
        assert_eq!(parsed.physical.can_scale, 1);
        assert_eq!(parsed.physical.corrosion_resist, 1);
        assert_eq!(parsed.physical.jump, 0, "unset physicals default to zero");

        // TrainValue (C4InfoCore.cpp:279-285): zero stays zero, caps hold,
        // never decreases.
        let mut zero = 0;
        PhysicalInfo::train_value(&mut zero, 100, C4_MAX_PHYSICAL);
        assert_eq!(zero, 0);
        let mut value = 99_950;
        PhysicalInfo::train_value(&mut value, 100, C4_MAX_PHYSICAL);
        assert_eq!(value, C4_MAX_PHYSICAL);
        let mut above = 120_000;
        PhysicalInfo::train_value(&mut above, 100, C4_MAX_PHYSICAL);
        assert_eq!(above, 120_000, "never decreased by training");
    }

    #[test]
    fn parse_def_core_fire_fields() {
        // ContactIncinerate / NoBurnDecay / NoBurnDamage (C4Def fire fields;
        // ContactIncinerate feeds the CrossCheck Tick35 arm,
        // C4GameObjects.cpp:121-125).
        let data = br#"
            [DefCore]
            id=FIRY
            Name=Firy
            ContactIncinerate=10
            NoBurnDecay=1
            NoBreath=1
            NoBurnDamage=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.contact_incinerate, 10);
        assert!(parsed.no_burn_decay);
        assert!(parsed.no_breath);
        assert!(parsed.no_burn_damage);

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.contact_incinerate, 0, "default: not inflammable");
        assert!(!parsed.no_burn_decay);
        assert!(!parsed.no_breath, "default: breathing");
        assert!(!parsed.no_burn_damage);
    }

    #[test]
    fn parse_act_map_records_dig_free() {
        let data = br#"
[Action]
Name=Dig
Procedure=Dig
DigFree=24
"#;
        let map = parse_act_map(data).expect("act map parsed");
        let dig = map.get("Dig").expect("dig action present");
        assert_eq!(dig.dig_free, Some(24));
    }

    #[test]
    fn parse_act_map_records_attach_mask() {
        let data = br#"
[Action]
Name=Scale
Procedure=Scale
Attach=1
"#;
        let map = parse_act_map(data).expect("act map parsed");
        let scale = map.get("Scale").expect("scale action present");
        assert_eq!(scale.attach, 1);
    }

    #[test]
    fn parse_def_core_value_mass_picture() {
        let data = br#"
            [DefCore]
            id=VALU
            Name=Valuable
            Value=75
            Mass=12
            Picture=1,2,32,24
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.value, 75);
        assert_eq!(parsed.mass, 12);
        assert_eq!(
            parsed.picture,
            Some(PictureRect {
                x: 1,
                y: 2,
                width: 32,
                height: 24
            })
        );
    }

    #[test]
    fn parse_def_core_collection_fields() {
        let data = br#"
            [DefCore]
            id=PACK
            Shape=-10,-20,20,40
            Collection=-5,-10,10,20
            CollectionLimit=3
            Collectible=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(
            parsed.shape,
            Some(PictureRect {
                x: -10,
                y: -20,
                width: 20,
                height: 40
            })
        );
        assert_eq!(
            parsed.collection,
            Some(PictureRect {
                x: -5,
                y: -10,
                width: 10,
                height: 20
            })
        );
        assert_eq!(parsed.collection_limit, Some(3));
        assert!(parsed.collectible);
    }

    #[test]
    fn parse_def_core_solid_mask_target_rect() {
        let data = br#"
            [DefCore]
            id=BASE
            Shape=-4,-6,12,18
            SolidMask=2,3,8,9,-1,4
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(
            parsed.solid_mask,
            Some(TargetRect {
                x: 2,
                y: 3,
                width: 8,
                height: 9,
                target_x: -1,
                target_y: 4,
            })
        );
    }

    #[test]
    fn parse_def_core_rotate_field() {
        let data = br#"
            [DefCore]
            id=SPNR
            Rotate=12
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.rotateable, 12);
    }

    #[test]
    fn parse_def_core_stretch_growth_field_like_cpp() {
        // Mirrors src/C4Def.cpp:387: DefCore `StretchGrowth` compiles into
        // `GrowthType` with default 0. Hand-derived golden: explicit value 1 is
        // true, and an omitted field is false.
        let data = br#"
            [DefCore]
            id=GROW
            StretchGrowth=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert!(parsed.stretch_growth);

        let defaulted = parse_def_core(b"[DefCore]\nid=JOLT\n").expect("defcore parsed");
        assert!(!defaulted.stretch_growth);
    }

    #[test]
    fn parse_def_core_shape_vertices_and_contact_metadata() {
        let data = br#"
            [DefCore]
            id=CLNK
            Shape=-8,-16,16,32
            Vertices=3
            VertexX=0,-4,4
            VertexY=9,3,3
            VertexCNAT=8,1,2
            VertexFriction=100,300,300
            ContactDensity=25
            ContactCalls=1
            BorderBound=7
            UprightAttach=8
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.vertices.len(), 3);
        assert_eq!(
            parsed.vertices[0],
            DefVertex {
                x: 0,
                y: 9,
                cnat: 8,
                friction: 100,
            }
        );
        assert_eq!(
            parsed.vertices[2],
            DefVertex {
                x: 4,
                y: 3,
                cnat: 2,
                friction: 300,
            }
        );
        assert_eq!(parsed.contact_density, 25);
        assert!(parsed.contact_function_calls);
        assert_eq!(parsed.border_bound, 7);
        assert_eq!(parsed.upright_attach, 8);
    }

    #[test]
    fn parse_def_core_vertex_arrays_zero_fill_missing_entries() {
        let data = br#"
            [DefCore]
            id=SPRS
            Vertices=4
            VertexX=-2,2
            VertexY=5
            VertexFriction=20,30
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.vertices.len(), 4);
        assert_eq!(parsed.vertices[0].x, -2);
        assert_eq!(parsed.vertices[1].x, 2);
        assert_eq!(parsed.vertices[2].x, 0);
        assert_eq!(parsed.vertices[0].y, 5);
        assert_eq!(parsed.vertices[1].y, 0);
        assert_eq!(parsed.vertices[0].friction, 20);
        assert_eq!(parsed.vertices[1].friction, 30);
        assert_eq!(parsed.vertices[2].friction, 0);
    }

    #[test]
    fn parse_def_core_components_list() {
        let data = br#"
            [DefCore]
            id=HUTS
            Components=WOOD:2,Metal=1; rock
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.components.len(), 3);
        assert_eq!(
            parsed.components[0],
            DefComponent {
                id: "WOOD".to_string(),
                count: 2
            }
        );
        assert_eq!(
            parsed.components[1],
            DefComponent {
                id: "METAL".to_string(),
                count: 1
            }
        );
        assert_eq!(
            parsed.components[2],
            DefComponent {
                id: "ROCK".to_string(),
                count: 1
            }
        );
    }

    #[test]
    fn load_definition_collects_scripts_from_nested_groups() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Nested.ocd");
        fs::create_dir(&def_dir).unwrap();
        fs::write(
            def_dir.join("DefCore.txt"),
            br#"[DefCore]
id=NNNN
Name=Nested
Category=C4D_Object
"#,
        )
        .unwrap();
        let script_dir = def_dir.join("Script.c4d");
        fs::create_dir(&script_dir).unwrap();
        fs::write(script_dir.join("Main.c"), b"func Main() {}\n").unwrap();
        let helpers_dir = script_dir.join("Helpers");
        fs::create_dir(&helpers_dir).unwrap();
        fs::write(helpers_dir.join("Util.c"), b"func Util() {}\n").unwrap();

        let group = Group::open(&def_dir).unwrap();
        let definition = Definition::load(&group).expect("definition load succeeds");
        assert!(definition
            .script
            .files()
            .iter()
            .any(|file| file.path == Path::new("Script.c4d").join("Main.c")));
        assert!(definition
            .script
            .files()
            .iter()
            .any(|file| file.path == Path::new("Script.c4d").join("Helpers").join("Util.c")));
    }

    #[test]
    fn load_definition_ignores_nested_definitions() {
        let temp = tempdir().unwrap();
        let def_dir = temp.path().join("Parent.ocd");
        fs::create_dir(&def_dir).unwrap();
        fs::write(
            def_dir.join("DefCore.txt"),
            br#"[DefCore]
id=PARA
Name=Parent
Category=C4D_Object
"#,
        )
        .unwrap();
        fs::write(def_dir.join("Script.c"), b"func Parent() {}\n").unwrap();
        let nested = def_dir.join("Child.ocd");
        fs::create_dir(&nested).unwrap();
        fs::write(
            nested.join("DefCore.txt"),
            br#"[DefCore]
id=CHLD
Name=Child
Category=C4D_Object
"#,
        )
        .unwrap();
        fs::write(nested.join("Script.c"), b"func Child() {}\n").unwrap();

        let group = Group::open(&def_dir).unwrap();
        let definition = Definition::load(&group).expect("definition load succeeds");
        assert_eq!(definition.script.files.len(), 1);
        assert_eq!(definition.script.files[0].path, PathBuf::from("Script.c"));
    }
}
