use crate::{decode_legacy_script_text, GraphicsImage, Group, GroupError};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

const C4D_MAX_VERTEX: usize = 30;

/// C4AllowPictureStack bits (src/C4Constants.h:301-309).
pub const APS_COLOR: i32 = 1 << 0;
pub const APS_GRAPHICS: i32 = 1 << 1;
pub const APS_NAME: i32 = 1 << 2;
pub const APS_OVERLAY: i32 = 1 << 3;
const C4M_SOLID: i32 = 50;

/// Files required to construct an engine definition from a classic C4 definition folder.
#[derive(Debug, Clone)]
pub struct Definition {
    pub core: DefCore,
    pub script: DefinitionScript,
    pub action_map: Option<ActionMap>,
    pub picture_image: Option<GraphicsImage>,
    /// ColorByOwner mask cropped to `picture_image`'s `Picture` facet.
    /// C++ obtains both through `Graphics.GetBitmap(color)` in
    /// `C4Def::Picture2Facet` (src/C4Def.cpp:1374-1378).
    pub picture_color_by_owner_mask: Option<ColorByOwnerMask>,
    pub graphics_image: Option<GraphicsImage>,
    pub color_by_owner_mask: Option<ColorByOwnerMask>,
    pub additional_graphics: HashMap<String, DefinitionGraphicsVariant>,
    /// First `Portrait*.*` def portrait (C4CFN_Portraits,
    /// src/C4Components.h:88). C++ assigns fresh crew a *non-synced* random
    /// portrait from the def set (`C4ObjectInfo::SetRandomPortrait`,
    /// src/C4ObjectInfo.cpp:398-425); the Rust HUD deterministically shows
    /// the first.
    pub portrait_image: Option<GraphicsImage>,
    /// ColorByOwner-aware form of `portrait_image`. Kept separately so
    /// existing portrait consumers can migrate without changing while the
    /// C++ text-image path gains its required owner-color surface.
    pub portrait_graphics_image: Option<GraphicsImage>,
    /// ColorByOwner mask paired with `portrait_graphics_image`. C++ maps
    /// Portrait1.png to Overlay1.png while loading definition graphics
    /// (C4DefGraphics.cpp:166-205).
    pub portrait_color_by_owner_mask: Option<ColorByOwnerMask>,
    /// Every `Portrait*.*` graphics entry in group order. `name` is the
    /// suffix after `Portrait` (for example `1` or `IndianChief`).
    pub portrait_graphics: Vec<DefinitionGraphicsVariant>,
    /// The def's own rank symbol strip (`C4Def::pRankSymbols` from
    /// Rank.png, src/C4Def.cpp:684-691).
    pub rank_symbols_image: Option<GraphicsImage>,
    /// Number of base rank cells in `rank_symbols_image`, after subtracting
    /// the extension cells named by leading-`*` entries in the selected
    /// localized `Rank*.txt` (`C4Def::iNumRankSymbols`,
    /// src/C4Def.cpp:694-706). `None` means no valid custom rank strip.
    pub rank_symbol_count: Option<u32>,
    /// Fully resolved custom rank names from the first localized
    /// `Rank{language}.txt|Rank.txt` component. `C4RankSystem` exposes base
    /// names first, followed by every leading-`*` extension format applied to
    /// every base name in order (src/C4RankSystem.cpp:96-180,184-211).
    /// `None` means that no component was present or that C++ would reject it
    /// for containing no ordinary rank name.
    pub rank_names: Option<Vec<String>>,
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
        Self::load_with_languages(group, &crate::scenario::DEFAULT_LANGUAGE_SEQUENCE)
    }

    pub fn load_with_languages<S: AsRef<str>>(
        group: &Group,
        languages: &[S],
    ) -> Result<Self, DefinitionError> {
        let mut core = DefCore::load(group)?;
        if let Some(name) = load_definition_name(group, languages)? {
            core.name = Some(name);
        }

        let mut script = load_scripts(group)?;

        let action_map = match group.read_file("ActMap.txt") {
            Ok(bytes) => Some(parse_act_map(&bytes)?),
            Err(GroupError::EntryNotFound(_)) => None,
            Err(GroupError::Io(ref err)) if err.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(DefinitionError::Resources(error)),
        };
        script.definition_description = load_definition_description(group)?;

        let (graphics_image, color_by_owner_mask, additional_graphics) =
            load_definition_graphics(group, core.color_by_owner);
        let picture_image = load_definition_picture(
            group,
            &core,
            color_by_owner_mask
                .as_ref()
                .and(graphics_image.as_ref()),
        );
        let picture_color_by_owner_mask = crop_definition_picture_mask(
            &core,
            picture_image.as_ref(),
            color_by_owner_mask.as_ref(),
        );
        let portrait_image = load_plain_image(group, "Portrait1.png");
        let (portrait_graphics_image, portrait_color_by_owner_mask) =
            load_graphics_entry(group, Path::new("Portrait1.png"), core.color_by_owner)
                .map(|(image, mask)| (Some(image), mask))
                .unwrap_or((None, None));
        let portrait_graphics = load_portrait_graphics(group, core.color_by_owner);
        // C4Def chooses the PNG branch by entry presence; a corrupt PNG does
        // not fall through to a valid legacy BMP (C4Def.cpp:684-691).
        let rank_symbols_image = if group.exists("Rank.png") {
            load_plain_image(group, "Rank.png")
        } else {
            load_plain_image(group, "Rank.bmp")
        }
        .filter(|image| image.height() > 0 && image.width() / image.height() > 0);
        let rank_name_table = load_rank_name_table(group, languages)?;
        let rank_extension_count = rank_name_table
            .as_ref()
            .map_or(0, |table| table.extension_count);
        let rank_symbol_count = rank_symbols_image.as_ref().and_then(|image| {
            let phase_count = image.width() / image.height().max(1);
            (phase_count > 0).then(|| phase_count.saturating_sub(rank_extension_count).max(1))
        });
        let rank_names = rank_name_table.map(|table| table.names);

        Ok(Self {
            core,
            script,
            action_map,
            picture_image,
            picture_color_by_owner_mask,
            graphics_image,
            color_by_owner_mask,
            additional_graphics,
            portrait_image,
            portrait_graphics_image,
            portrait_color_by_owner_mask,
            portrait_graphics,
            rank_symbols_image,
            rank_symbol_count,
            rank_names,
        })
    }

    /// Trimmed localized definition description exposed by
    /// `C4Def::GetDesc` (C4Def.cpp:713-717).
    pub fn description(&self) -> Option<&str> {
        self.script.definition_description.as_deref()
    }
}

/// Loads the default US entry from `C4CFN_DefDesc = "Desc{}.txt"`.
///
/// The C++ runtime receives a language sequence and ultimately falls back to
/// US (`C4Language::LoadLanguage`, C4Language.cpp:250-263). Resource loading
/// does not yet carry a locale, so selecting that hardcoded fallback keeps the
/// retained description deterministic.
fn load_definition_description(group: &Group) -> Result<Option<String>, DefinitionError> {
    const DESCRIPTION: &str = "DescUS.txt";
    if !group.exists(DESCRIPTION) {
        return Ok(None);
    }

    let bytes = group.read_file(DESCRIPTION)?;
    let description = decode_legacy_script_text(&bytes).trim().to_string();
    Ok((!description.is_empty()).then_some(description))
}

/// `C4Def::Load`'s localized `C4CFN_DefNames = "Names{}.txt|Names.txt"`:
/// load the first filename admitted by the language sequence, then select
/// the first matching `XX:` line from that one component
/// (`C4Def.cpp:635-639`; `C4ComponentHost.cpp:55-94,238-260`).
fn load_definition_name<S: AsRef<str>>(
    group: &Group,
    languages: &[S],
) -> Result<Option<String>, DefinitionError> {
    let Some(candidate) = first_localized_component(group, "Names", languages) else {
        return Ok(None);
    };
    let text = decode_legacy_script_text(&group.read_file(candidate)?);
    Ok(languages.iter().find_map(|language| {
        let needle = format!("{}:", language.as_ref());
        text.find(&needle).and_then(|position| {
            let value = &text[position + needle.len()..];
            let end = value.find(['\r', '\n']).unwrap_or(value.len());
            let value = value[..end].to_string();
            (!value.is_empty()).then_some(value)
        })
    }))
}

/// Selects `Stem{language}.txt|Stem.txt` with the same filename-first,
/// language-sequence order as `C4ComponentHost::Load` (src/C4ComponentHost.cpp:
/// 65-94). Language-pack cross-loading is intentionally outside the local
/// group resource model.
fn first_localized_component<S: AsRef<str>>(
    group: &Group,
    stem: &str,
    languages: &[S],
) -> Option<String> {
    languages
        .iter()
        .map(|language| format!("{stem}{}.txt", language.as_ref()))
        .chain(std::iter::once_with(|| format!("{stem}.txt")))
        .find(|name| group.exists(name))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RankNameTable {
    names: Vec<String>,
    extension_count: u32,
}

/// Loads and resolves the selected definition rank component exactly in the
/// order exposed by `C4RankSystem::GetRankName`: ordinary names first, then
/// each leading-`*` extension applied across all ordinary names. Comments and
/// settings are retained by neither list, and a component without an ordinary
/// name is rejected (src/C4RankSystem.cpp:96-211).
fn load_rank_name_table<S: AsRef<str>>(
    group: &Group,
    languages: &[S],
) -> Result<Option<RankNameTable>, DefinitionError> {
    let Some(candidate) = first_localized_component(group, "Rank", languages) else {
        return Ok(None);
    };
    let text = decode_legacy_script_text(&group.read_file(candidate)?);
    let mut ordinary_names = Vec::new();
    let mut extensions = Vec::new();
    // The C++ loop only processes lines when it encounters CR or LF within
    // the component data; its appended trailing NUL lies outside that loop.
    // Consequently an unterminated final line is intentionally ignored.
    // Embedded NUL bytes are terminators too because C++ tests `!*pPos`.
    for terminated_line in text.split_inclusive(['\0', '\r', '\n']) {
        let line = terminated_line
            .strip_suffix('\0')
            .or_else(|| terminated_line.strip_suffix('\r'))
            .or_else(|| terminated_line.strip_suffix('\n'));
        let Some(line) = line.filter(|line| !line.is_empty()) else {
            continue;
        };
        if let Some(extension) = line.strip_prefix('*') {
            extensions.push(extension.to_string());
        } else if !line.starts_with('#') && !line.contains('=') {
            ordinary_names.push(line.to_string());
        }
    }
    if ordinary_names.is_empty() {
        return Ok(None);
    }

    let extension_count = u32::try_from(extensions.len()).unwrap_or(u32::MAX);
    let mut names = Vec::with_capacity(
        ordinary_names
            .len()
            .saturating_mul(extensions.len().saturating_add(1)),
    );
    names.extend(ordinary_names.iter().cloned());
    for extension in extensions {
        names.extend(
            ordinary_names
                .iter()
                .map(|name| format_rank_extension(&extension, name)),
        );
    }
    Ok(Some(RankNameTable {
        names,
        extension_count,
    }))
}

/// The shipped rank extensions use the `fmt::sprintf(format, base_name)`
/// `%s`/`%%` surface. Parse those tokens instead of a blanket replacement so
/// escaped percent signs cannot accidentally become placeholders.
fn format_rank_extension(format: &str, base_name: &str) -> String {
    let mut output = String::with_capacity(format.len().saturating_add(base_name.len()));
    let mut chars = format.chars().peekable();
    while let Some(current) = chars.next() {
        if current != '%' {
            output.push(current);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                output.push('%');
            }
            Some('s') => {
                chars.next();
                output.push_str(base_name);
            }
            _ => output.push('%'),
        }
    }
    output
}

/// Decodes a single named image from the def group, `None` when absent.
fn load_plain_image(group: &Group, name: &str) -> Option<GraphicsImage> {
    let data = group.read_file(name).ok()?;
    let image = image::load_from_memory(&data).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    (width > 0 && height > 0).then(|| GraphicsImage::new(width, height, image.into_raw()))
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
    pub count: i32,
}

#[derive(Debug, Clone)]
pub struct DefCore {
    pub id: String,
    /// Five-component `rC4XVer` loaded from DefCore `Version=`
    /// (src/C4Def.cpp:124,254).
    pub version: [i32; 5],
    pub name: Option<String>,
    pub category: i32,
    pub crew_member: bool,
    pub value: i32,
    /// `Rebuy` (C4Def.cpp:359): sold objects may introduce their ID into
    /// the player's home-base stock when nonzero.
    pub rebuyable: bool,
    /// `BaseAutoSell` (C4Def.cpp:457): bases automatically sell this object
    /// when BASEFUNC_AutoSellContents is active. GOLD defaults to true.
    pub base_auto_sell: bool,
    pub mass: i32,
    /// `MoveToRange` (C4Def.cpp:400): positive values override the command's
    /// default five-pixel arrival range for non-crew objects.
    pub move_to_range: i32,
    /// `Pathfinder` (C4Def.cpp:399): nonzero opts non-crew objects into
    /// C4Command::MoveTo path search and supplies its clamped search level.
    pub pathfinder: i32,
    /// `NoTransferZones` (C4Def.cpp:415): disables transfer-zone edges in
    /// C4Command::MoveTo path search.
    pub no_transfer_zones: i32,
    pub picture: Option<PictureRect>,
    pub color_by_owner: bool,
    /// DefCore `AllowPictureStack` exceptions to
    /// C4Object::CanConcatPictureWith's picture equality checks.
    pub allow_picture_stack: i32,
    /// DefCore graphics `Scale` as a percent (C4Def.cpp:456,725).
    pub graphics_scale: u32,
    /// Definition-default C4Object::BlitMode (DefCore `BlitMode`).
    pub blit_mode: u32,
    pub shape: Option<PictureRect>,
    /// Shape-relative fire emission offset (C4Shape::FireTop).
    pub fire_top: i32,
    pub solid_mask: Option<TargetRect>,
    /// `TopFace` (C4Def.cpp:306): source facet plus object-relative draw target.
    pub top_face: Option<TargetRect>,
    pub vertices: Vec<DefVertex>,
    /// Complete C4Shape fixed-slot storage. `vertices` is the active
    /// `VtxNum` prefix; these slots also retain dormant array values that a
    /// later AddVertex can expose without rewriting CNAT/friction.
    pub vertex_slots: [DefVertex; C4D_MAX_VERTEX],
    pub contact_density: i32,
    pub contact_function_calls: bool,
    pub collection: Option<PictureRect>,
    pub collection_limit: Option<u32>,
    /// ContactIncinerate=N: 1-in-N chance of catching fire on contact with a
    /// burning object (CrossCheck pass 1, C4GameObjects.cpp:121-125); 0 = not
    /// inflammable.
    pub contact_incinerate: i32,
    /// BlastIncinerate=N: incinerate when accumulated Damage reaches N after
    /// a blast (C4Object::Blast, C4Object.cpp:1421-1423); 0 = off.
    pub blast_incinerate: i32,
    /// ContainBlast=1: this container shields its contents from explosions
    /// (the DoExplosion container walk, C4Effect.cpp:884; C4Def.cpp:380).
    pub contain_blast: i32,
    /// HorizontalFix=1 (C4Def::NoHorizontalMove, C4Def.cpp:383): exempt
    /// from shockwave flings (Game::BlastObjects, C4Game.cpp:1272).
    pub no_horizontal_move: i32,
    /// NoBurnDecay=1: burning does not reduce Con (C4Object.cpp:777-778).
    pub no_burn_decay: bool,
    /// `Float` (C4Def.cpp:379, default 0): buoyancy line offset in percent
    /// of Con — IsInLiquidCheck probes GBackLiquid(x, y + Float*Con/FullCon
    /// - 1) (C4Object.cpp:5609-5612).
    pub float_line: i32,
    /// `Line=` (C4D_Line* type tokens, C4Def.cpp:318-332); Line objects
    /// skip UpdateShape and their vertices span the action targets.
    pub line: i32,
    /// `LineIntersect=` (0 wrapping, 1 direct vertex assignment).
    pub line_intersect: i32,
    /// `Grab` (C4Def.cpp): 0 none, 1 grab+push, 2 grab-only.
    pub grab: i32,
    /// `GrabPutGet` bitfield (src/C4Def.cpp:364-373): C4D_GrabPut=1 |
    /// C4D_GrabGet=2 — the grabbed-vehicle put/get commands.
    pub grab_put_get: i32,
    /// `NoGet` (src/C4Def.cpp:412): any nonzero value hides the definition
    /// from manual get/activate menus. Retain the signed compiler value.
    pub no_get: i32,
    /// `VehicleControl` (src/C4Def.cpp:398, default 0):
    /// C4D_VehicleControl_Outside=1 | C4D_VehicleControl_Inside=2 — the
    /// SetCommand ControlCommand overloads (src/C4Object.cpp:3944-3969).
    pub vehicle_control: i32,
    /// `NoBreath` (C4Def.cpp:409): exempt from the ExecLife breathing check.
    pub no_breath: bool,
    /// NoBurnDamage=1: burning deals no damage (C4Object.cpp:780).
    pub no_burn_damage: bool,
    /// BurnTurnTo=ID: definition change on incineration (C4Effect.cpp:580-585).
    pub burn_turn_to: Option<String>,
    /// `ConstructTo=ID` (`C4Def::BuildTurnTo`): successful Build ticks
    /// change the construction target to this definition after DoCon.
    pub build_turn_to: Option<String>,
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
    /// `Placement=` (C4Def.cpp:312): 0 surface, 1 liquid, 2 air —
    /// PlaceVegetation/PlaceAnimal dispatch on it (C4Game.cpp:2978,3034).
    pub placement: i32,
    /// `Growth=` (C4Def.cpp:358): growth speed; non-zero admits the
    /// random-growth draw in PlaceVegetation (C4Game.cpp:2974).
    pub growth: i32,
    pub basement: i32,
    pub rotateable: i32,
    pub border_bound: i32,
    pub upright_attach: u32,
    /// RotatedSolidmasks (C4Def.cpp:414, default 0): solid masks stay put
    /// while the object is rotated (C4Object.cpp:5655).
    pub rotated_solid_masks: bool,
    /// `AutoContextMenu` (C4Def.cpp:416, default 0): entering this container
    /// may automatically open its context menu (C4Object.cpp:2049-2056).
    pub auto_context_menu: bool,
    /// `SilentCommands` (C4Def.cpp:404, default 0): suppresses the common
    /// command-failure message, sound, and ComDir stop tail.
    pub silent_commands: bool,
    /// `NoComponentMass` (C4Def compile): contents mass does not add to
    /// the live Mass (C4Object::UpdateMass, C4Object.cpp:497-501).
    pub no_component_mass: bool,
    /// NoStabilize (C4Def.cpp:402): opts out of the Stabilize upright snap.
    pub no_stabilize: bool,
    /// Timer= interval in frames (default 35, C4Def.cpp:298).
    pub timer: i32,
    /// TimerCall= function name (C4Def.cpp:299); None when absent/empty.
    pub timer_call: Option<String>,
    pub components: Vec<DefComponent>,
    pub line_connect: u32,
    /// `Entrance` rect (C4Def.cpp:309): the enter/activate area for
    /// OCF_Entrance (SetOCF, C4Object.cpp:584-587).
    pub entrance: Option<PictureRect>,
    /// `RotatedEntrance` (C4Def.cpp:377): 0 = upright only, 1 = any
    /// rotation, N = up to N degrees (SetOCF, C4Object.cpp:586).
    pub rotated_entrance: i32,
    /// `Exclusive` (C4Def.cpp:313): blocks action/construction behind it
    /// (OCF_Exclusive, SetOCF C4Object.cpp:581-583).
    pub exclusive: bool,
    /// `Prey` (C4Def.cpp:354): OCF_Prey while alive (SetOCF,
    /// C4Object.cpp:615-618).
    pub prey: bool,
    /// `Edible` (C4Def.cpp:355): OCF_Edible (SetOCF, C4Object.cpp:630-632).
    pub edible: bool,
    /// `Chop` -> C4Def::Chopable (C4Def.cpp:378): OCF_Chop candidate
    /// (SetOCF, C4Object.cpp:570-575).
    pub chopable: bool,
    /// `AttractLightning` (C4Def.cpp:391): OCF_AttractLightning at FullCon
    /// (SetOCF, C4Object.cpp:623-626).
    pub attract_lightning: bool,
    /// `NoFight` (C4Def.cpp:413): suppresses OCF_FightReady (SetOCF,
    /// C4Object.cpp:606-610).
    pub no_fight: bool,
    /// `CanBeBase` (C4Def.cpp DefCore): marks structures usable as the
    /// FirstBase in PlaceReadyBase (C4Player.cpp:596-599).
    pub can_be_base: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Loaded alongside the other definition text components. Keeping this
    /// private preserves `Definition`'s public resource-parts shape while the
    /// accessor above exposes the C4Def-level description.
    definition_description: Option<String>,
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
    /// `InLiquidAction` (C4ActionDef; the ExecAction head switches to it
    /// while InLiquid, C4Object.cpp:4749-4753).
    pub in_liquid_action: Option<String>,
    pub delay: Option<u32>,
    pub step: Option<u32>,
    pub phase_call: Option<String>,
    pub start_call: Option<String>,
    pub end_call: Option<String>,
    pub abort_call: Option<String>,
    pub no_other_action: bool,
    /// `ObjectDisabled=` (C4ActionDef::Disabled, C4Def.cpp:106): the
    /// action suspends the object — vetoes OCF_Collection/OCF_FightReady
    /// (SetOCF, C4Object.cpp:597,608).
    pub disabled: bool,
    pub dig_free: Option<i32>,
    pub attach: u32,
    pub directions: Option<u32>,
    /// `TurnAction` (C4ActionDef): SetDir fires it on direction change
    /// (C4Object.cpp:4225-4240).
    pub turn_action: Option<String>,
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
            in_liquid_action: None,
            next_action: None,
            next_action_index: ACT_IDLE,
            delay: None,
            step: None,
            phase_call: None,
            start_call: None,
            end_call: None,
            abort_call: None,
            no_other_action: false,
            disabled: false,
            dig_free: None,
            attach: 0,
            turn_action: None,
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
    let mut version = [0; 5];
    let mut name: Option<String> = None;
    let mut category: i32 = 0;
    let mut category_set = false;
    let mut crew_member = false;
    let mut can_be_base = false;
    let mut object_value: i32 = 0;
    let mut rebuyable = false;
    let mut base_auto_sell: Option<bool> = None;
    let mut object_mass: i32 = 0;
    let mut move_to_range: i32 = 0;
    let mut pathfinder: i32 = 0;
    let mut no_transfer_zones: i32 = 0;
    let mut picture: Option<PictureRect> = None;
    let mut color_by_owner = false;
    let mut allow_picture_stack: i32 = 0;
    let mut graphics_scale: u32 = 100;
    let mut blit_mode: u32 = 0;
    let mut shape: Option<PictureRect> = None;
    let mut shape_width: Option<i32> = None;
    let mut shape_height: Option<i32> = None;
    let mut shape_offset: Option<(i32, i32)> = None;
    let mut fire_top: i32 = 0;
    let mut solid_mask: Option<TargetRect> = None;
    let mut top_face: Option<TargetRect> = None;
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
    let mut blast_incinerate: i32 = 0;
    let mut contain_blast: i32 = 0;
    let mut no_horizontal_move: i32 = 0;
    let mut no_burn_decay = false;
    let mut no_breath = false;
    let mut grab = 0;
    let mut float_line = 0;
    let mut line_type: i32 = 0;
    let mut line_intersect: i32 = 0;
    let mut no_burn_damage = false;
    let mut burn_turn_to: Option<String> = None;
    let mut build_turn_to: Option<String> = None;
    let mut incomplete_activity = false;
    let mut physical = PhysicalInfo::default();
    let mut collectible = false;
    let mut grab_put_get: i32 = 0;
    let mut no_get: i32 = 0;
    let mut vehicle_control: i32 = 0;
    let mut constructable = false;
    let mut con_size_off: i32 = 0;
    let mut stretch_growth = false;
    let mut placement: i32 = 0;
    let mut growth: i32 = 0;
    let mut basement: i32 = 0;
    let mut rotateable: i32 = 0;
    let mut border_bound: i32 = 0;
    let mut upright_attach: u32 = 0;
    // RotatedSolidmasks (C4Def.cpp:414, default 0).
    let mut rotated_solid_masks = false;
    // AutoContextMenu (C4Def.cpp:416, default 0).
    let mut auto_context_menu = false;
    // SilentCommands (C4Def.cpp:404, default 0).
    let mut silent_commands = false;
    let mut no_component_mass = false;
    // NoStabilize (C4Def.cpp:402, default 0): opts out of C4Object::Stabilize.
    let mut no_stabilize = false;
    // Timer=/TimerCall= (C4Def.cpp:298-299): the per-object Def timer.
    let mut timer: i32 = 35;
    let mut timer_call: Option<String> = None;
    let mut components: Vec<DefComponent> = Vec::new();
    let mut line_connect: u32 = 0;
    let mut entrance: Option<PictureRect> = None;
    let mut rotated_entrance: i32 = 0;
    let mut exclusive = false;
    let mut prey = false;
    let mut edible = false;
    let mut chopable = false;
    let mut attract_lightning = false;
    let mut no_fight = false;

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
            "version" => {
                fill_i32_array(value, &mut version);
            }
            "name" => {
                if !value.is_empty() {
                    name = Some(value.to_string());
                }
            }
            "value" => {
                object_value = parse_i32(value).unwrap_or(0);
            }
            "rebuy" => {
                rebuyable = parse_bool(value);
            }
            "baseautosell" => {
                base_auto_sell = Some(parse_bool(value));
            }
            "mass" => {
                object_mass = parse_i32(value).unwrap_or(0).max(0);
            }
            "movetorange" => {
                move_to_range = parse_i32(value).unwrap_or(0);
            }
            "pathfinder" => {
                pathfinder = parse_i32(value).unwrap_or(0);
            }
            "notransferzones" => {
                no_transfer_zones = parse_i32(value).unwrap_or(0);
            }
            "category" => {
                category = parse_category(value)?;
                category_set = true;
            }
            "crewmember" => {
                crew_member = parse_bool(value);
            }
            // C4DefCore::CompileFunc names CanBeBase as "Base"
            // (C4Def.cpp:317). Keep the descriptive alias for fixtures.
            "base" | "canbebase" => {
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
            "allowpicturestack" => {
                // StdBitfieldAdapt over the APS_* table
                // (src/C4Def.cpp:419-429); numeric values pass through.
                allow_picture_stack = value
                    .split('|')
                    .map(str::trim)
                    .map(|token| match token {
                        "APS_Color" => APS_COLOR,
                        "APS_Graphics" => APS_GRAPHICS,
                        "APS_Name" => APS_NAME,
                        "APS_Overlay" => APS_OVERLAY,
                        other => other.parse::<i32>().unwrap_or(0),
                    })
                    .fold(0, |flags, bit| flags | bit);
            }
            "scale" => {
                graphics_scale = parse_i32(value).unwrap_or(100).max(0) as u32;
            }
            "blitmode" => {
                blit_mode = parse_i32(value).unwrap_or(0) as u32;
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
            "firetop" => {
                fire_top = parse_i32(value).unwrap_or(0);
            }
            "solidmask" => {
                solid_mask =
                    parse_target_rect(value).filter(|rect| rect.width > 0 && rect.height > 0);
            }
            "topface" => {
                top_face =
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
            "blastincinerate" => {
                blast_incinerate = parse_i32(value).unwrap_or(0);
            }
            "containblast" => {
                contain_blast = parse_i32(value).unwrap_or(0);
            }
            "horizontalfix" => {
                no_horizontal_move = parse_i32(value).unwrap_or(0);
            }
            "noburndecay" => {
                no_burn_decay = parse_bool(value);
            }
            "nobreath" => {
                no_breath = parse_bool(value);
            }
            "line" => {
                line_type = parse_line_type(value);
            }
            "lineintersect" => {
                line_intersect = parse_i32(value).unwrap_or(0);
            }
            "float" => {
                float_line = parse_i32(value).unwrap_or(0);
            }
            "grab" => {
                grab = parse_i32(value).unwrap_or(0).max(0);
            }
            "vehiclecontrol" => {
                // Plain integer compile (src/C4Def.cpp:398).
                vehicle_control = parse_i32(value).unwrap_or(0);
            }
            "grabputget" => {
                // StdBitfieldAdapt over C4D_GrabPut/C4D_GrabGet tokens
                // (src/C4Def.cpp:364-373); numeric values pass through.
                grab_put_get = value
                    .split('|')
                    .map(str::trim)
                    .map(|token| match token {
                        "C4D_GrabPut" => 1,
                        "C4D_GrabGet" => 2,
                        other => other.parse::<i32>().unwrap_or(0),
                    })
                    .fold(0, |acc, bit| acc | bit);
            }
            "noburndamage" => {
                no_burn_damage = parse_bool(value);
            }
            "burnturnto" => {
                if !value.is_empty() {
                    burn_turn_to = Some(value.to_string());
                }
            }
            "constructto" => {
                if !value.is_empty()
                    && !value.eq_ignore_ascii_case("NONE")
                    && value != "0000"
                {
                    build_turn_to = Some(value.to_string());
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
            "noget" => {
                no_get = parse_i32(value).unwrap_or(0);
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
            "placement" => {
                placement = parse_i32(value).unwrap_or(0);
            }
            "growth" => {
                growth = parse_i32(value).unwrap_or(0);
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
            "rotatedsolidmasks" => {
                rotated_solid_masks = parse_bool(value);
            }
            "autocontextmenu" => {
                auto_context_menu = parse_bool(value);
            }
            "silentcommands" => {
                silent_commands = parse_bool(value);
            }
            "nocomponentmass" => {
                no_component_mass = parse_bool(value);
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
            // C4Object::SetOCF DefCore inputs (C4Def.cpp:309-413).
            "entrance" => {
                entrance = parse_rect(value);
            }
            "rotatedentrance" => {
                rotated_entrance = parse_i32(value).unwrap_or(0);
            }
            "exclusive" => {
                exclusive = parse_bool(value);
            }
            "prey" => {
                prey = parse_bool(value);
            }
            "edible" => {
                edible = parse_bool(value);
            }
            "chop" => {
                chopable = parse_bool(value);
            }
            "attractlightning" => {
                attract_lightning = parse_bool(value);
            }
            "nofight" => {
                no_fight = parse_bool(value);
            }
            _ => {}
        }
    }

    let id = id.ok_or(DefinitionError::MissingDefCoreField("id"))?;
    if !category_set {
        // Preserve compatibility with the C++ engine where unspecified category defaults to 0.
        category = 0;
    }

    let vertex_slots = std::array::from_fn(|idx| DefVertex {
        x: vertex_x[idx],
        y: vertex_y[idx],
        cnat: vertex_cnat[idx],
        friction: vertex_friction[idx],
    });
    let vertices = vertex_slots[..vertex_count].to_vec();

    let base_auto_sell = base_auto_sell.unwrap_or_else(|| id.eq_ignore_ascii_case("GOLD"));

    Ok(DefCore {
        id,
        version,
        name,
        category,
        crew_member,
        value: object_value,
        rebuyable,
        base_auto_sell,
        mass: object_mass,
        move_to_range,
        pathfinder,
        no_transfer_zones,
        picture,
        color_by_owner,
        allow_picture_stack,
        graphics_scale,
        blit_mode,
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
        fire_top,
        solid_mask,
        top_face,
        vertices,
        vertex_slots,
        contact_density,
        contact_function_calls,
        collection,
        collection_limit,
        contact_incinerate,
        blast_incinerate,
        contain_blast,
        no_horizontal_move,
        no_burn_decay,
        no_breath,
        grab,
        float_line,
        line: line_type,
        line_intersect,
        no_burn_damage,
        burn_turn_to,
        build_turn_to,
        incomplete_activity,
        physical,
        collectible,
        grab_put_get,
        no_get,
        vehicle_control,
        constructable,
        con_size_off,
        stretch_growth,
        placement,
        growth,
        basement,
        rotateable,
        border_bound,
        upright_attach,
        rotated_solid_masks,
        auto_context_menu,
        silent_commands,
        no_component_mass,
        no_stabilize,
        timer,
        timer_call,
        components,
        line_connect,
        can_be_base,
        entrance,
        rotated_entrance,
        exclusive,
        prey,
        edible,
        chopable,
        attract_lightning,
        no_fight,
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
            // C4IDList::Entry starts at zero. Its compiler reads the count
            // only when an '=' separator is present, and stores the signed
            // int32 verbatim (C4IDList.cpp:239-253). Retain the historical
            // lenient fallback for an explicitly malformed count without
            // conflating that case with a bare ID.
            let count = match count_part {
                Some(raw) => raw.parse::<i32>().unwrap_or(1),
                None => 0,
            };
            Some(DefComponent { id, count })
        })
        .collect()
}

fn normalize_line_connect_token(token: &str) -> String {
    token.trim().replace([' ', '_'], "").to_ascii_lowercase()
}

/// `mkBitfieldAdapt(Line, LineTypes)` (C4Def.cpp:319-333): named values
/// separated by `|` are ORed. In particular, legacy DPIP spells the drain
/// value as `C4D_LinePower|C4D_LineSource` (1 | 2 = 3).
fn parse_line_type(value: &str) -> i32 {
    value.split(['|', ',', ';']).fold(0, |line, token| {
        line | match token.trim() {
            "C4D_LinePower" => 1,
            "C4D_LineSource" => 2,
            "C4D_LineDrain" => 3,
            "C4D_LineLightning" => 4,
            "C4D_LineVolcano" => 5,
            "C4D_LineRope" => 6,
            "C4D_LineColored" => 7,
            "C4D_LineVertex" => 8,
            other => parse_i32(other).unwrap_or(0),
        }
    })
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
            definition_description: None,
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

    Ok(DefinitionScript {
        files,
        combined,
        definition_description: None,
    })
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
            "inliquidaction" => {
                if !value.is_empty() {
                    current_definition.in_liquid_action = Some(value.to_string());
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
            "objectdisabled" => {
                current_definition.disabled = parse_bool(value);
            }
            "digfree" => {
                current_definition.dig_free = parse_i32(value);
            }
            "attach" => {
                current_definition.attach = parse_i32(value).unwrap_or(0).max(0) as u32;
            }
            "turnaction" => {
                if !value.is_empty() {
                    current_definition.turn_action = Some(value.to_string());
                }
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

fn load_definition_picture(
    group: &Group,
    core: &DefCore,
    processed_graphics: Option<&GraphicsImage>,
) -> Option<GraphicsImage> {
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

    let pixels = processed_graphics
        .filter(|graphics| (graphics.width(), graphics.height()) == (width, height))
        .map(|graphics| {
            extract_rgba_bytes(
                graphics.pixels(),
                width,
                crop_x,
                crop_y,
                crop_w,
                crop_h,
            )
        })
        .unwrap_or_else(|| extract_rgba_region(&image, crop_x, crop_y, crop_w, crop_h));
    Some(GraphicsImage::new(crop_w, crop_h, pixels))
}

fn crop_definition_picture_mask(
    core: &DefCore,
    picture: Option<&GraphicsImage>,
    mask: Option<&ColorByOwnerMask>,
) -> Option<ColorByOwnerMask> {
    let picture = picture?;
    let mask = mask?;
    let (crop_x, crop_y, crop_w, crop_h) = match core.picture {
        Some(rect) => normalize_crop(rect, mask.width, mask.height)
            .unwrap_or((0, 0, mask.width, mask.height)),
        None => (0, 0, mask.width, mask.height),
    };
    if (crop_w, crop_h) != (picture.width(), picture.height()) {
        return None;
    }

    let mut pixels = Vec::with_capacity((crop_w * crop_h) as usize);
    for row in crop_y..crop_y + crop_h {
        let start = (row * mask.width + crop_x) as usize;
        let end = start + crop_w as usize;
        pixels.extend_from_slice(&mask.pixels[start..end]);
    }
    pixels
        .iter()
        .any(|value| *value != 0)
        .then_some(ColorByOwnerMask {
            width: crop_w,
            height: crop_h,
            pixels,
        })
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

fn load_portrait_graphics(
    group: &Group,
    color_by_owner: bool,
) -> Vec<DefinitionGraphicsVariant> {
    let mut portraits = Vec::new();
    for entry in group.entries().unwrap_or_default() {
        if entry.is_directory {
            continue;
        }
        let path = entry.relative_path;
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(name) = stem
            .get(..8)
            .filter(|prefix| prefix.eq_ignore_ascii_case("Portrait"))
            .and_then(|_| stem.get(8..))
        else {
            continue;
        };
        let supported = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("png") || extension.eq_ignore_ascii_case("bmp")
            });
        if !supported {
            continue;
        }
        let Some((image, mask)) = load_graphics_entry(group, &path, color_by_owner) else {
            continue;
        };
        portraits.push(DefinitionGraphicsVariant {
            name: name.to_string(),
            image,
            color_by_owner_mask: mask,
        });
    }
    portraits
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
            let suffix = name
                .strip_prefix("Graphics")
                .or_else(|| name.strip_prefix("Portrait"));
            if let Some(stripped) = suffix.filter(|stripped| !stripped.is_empty()) {
                let mut candidate = parent.to_path_buf();
                candidate.push(format!("Overlay{}", stripped));
                candidates.push(candidate);
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

    // Overlay.png IS the ClrByOwner surface (C4DefGraphics.cpp:74-94 +
    // C4Surface::SetAsClrByOwnerOf, C4Surface.cpp:320-331): its pixels are
    // blitted owner-modulated OVER the base with the OVERLAY's alpha. Baked
    // into the single-image + scalar-mask model: the base contribution
    // shrinks by the overlay coverage (exactly black under an opaque
    // overlay), the sprite alpha is the over-composite, and the mask keeps
    // the coverage-scaled overlay intensity so the draw-time
    // `blend_color_by_owner` reproduces `overlay ⊗ owner` for gray overlays.
    let mut pixels = vec![0u8; (width * height) as usize];
    let mut has_mask = false;
    for y in 0..height {
        for x in 0..width {
            let overlay_pixel = overlay.get_pixel(x, y);
            let coverage = u16::from(overlay_pixel[3]);
            if coverage == 0 {
                continue;
            }
            let mask_value = (u16::from(overlay_pixel[0]) * coverage / 255) as u8;
            if mask_value == 0 {
                continue;
            }
            let idx = (y * width + x) as usize;
            pixels[idx] = mask_value;
            has_mask = true;
            let base_pixel = base.get_pixel_mut(x, y);
            let keep = 255 - coverage;
            let base_alpha = u16::from(base_pixel[3]);
            *base_pixel = image::Rgba([
                (u16::from(base_pixel[0]) * keep / 255) as u8,
                (u16::from(base_pixel[1]) * keep / 255) as u8,
                (u16::from(base_pixel[2]) * keep / 255) as u8,
                (base_alpha + coverage * (255 - base_alpha) / 255) as u8,
            ]);
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

fn extract_rgba_bytes(
    pixels: &[u8],
    stride: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut output = Vec::with_capacity((width * height * 4) as usize);
    for row in y..(y + height) {
        let row_start = ((row * stride) + x) as usize * 4;
        let row_end = row_start + (width as usize * 4);
        output.extend_from_slice(&pixels[row_start..row_end]);
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
    // C4TargetRect: x,y,wdt,hgt with mkDefaultAdapt(0) tx,ty — 4 to 6
    // entries are valid (C4TargetRect::CompileFunc, C4Rect.cpp:80-86;
    // Mage.c4d AimMagic uses the 5-value form).
    if parts.len() < 4 || parts.len() > 6 {
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
    let target_x = numbers.get(4).copied().unwrap_or(0);
    let target_y = numbers.get(5).copied().unwrap_or(0);
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
    // Facet= is a C4TargetRect: x,y,wdt,hgt plus DEFAULTED tx,ty — partial
    // lists are valid (C4TargetRect::CompileFunc mkDefaultAdapt,
    // C4Rect.cpp:80-86). Mage.c4d AimMagic uses the 5-value form
    // "0,328,24,20,-4".
    #[test]
    fn action_facet_accepts_partial_target_offsets_like_c4targetrect() {
        let five = parse_action_facet("0,328,24,20,-4").expect("5-value facet parses");
        assert_eq!(
            (five.x, five.y, five.width, five.height, five.target_x, five.target_y),
            (0, 328, 24, 20, -4, 0)
        );
        let six = parse_action_facet("0,260,16,24,0,-4").expect("6-value facet parses");
        assert_eq!((six.target_x, six.target_y), (0, -4));
        let four = parse_action_facet("0,0,16,20").expect("4-value facet parses");
        assert_eq!((four.target_x, four.target_y), (0, 0));
    }

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

    // Overlay.png is the ClrByOwner surface itself: C4DefGraphics::LoadGraphics
    // keeps it as BitmapClr with the base as pMainSfc (C4DefGraphics.cpp:74-94,
    // C4Surface::SetAsClrByOwnerOf, C4Surface.cpp:320-331), so drawing blits
    // the overlay pixel modulated by the owner color OVER the base using the
    // OVERLAY's alpha. The Mage body lives only in Overlay.png (base cells are
    // transparent apart from the staff) — the baked sprite must make those
    // pixels visible with the overlay intensity as mask.
    #[test]
    fn overlay_only_pixels_become_visible_owner_masked_pixels() {
        let mut base = image::RgbaImage::from_pixel(2, 1, image::Rgba([100, 64, 35, 0]));
        // The "staff": opaque base content without overlay coverage.
        base.put_pixel(1, 0, image::Rgba([80, 50, 20, 255]));
        let mut overlay = image::RgbaImage::from_pixel(2, 1, image::Rgba([100, 100, 100, 0]));
        // The "robe": opaque gray overlay over a transparent base pixel.
        overlay.put_pixel(0, 0, image::Rgba([136, 136, 136, 255]));

        let mask = extract_mask_from_overlay(&overlay, &mut base).expect("mask extracted");

        // Robe pixel: fully covered by the overlay — the sprite must be
        // opaque, contribute no untinted color (black base term) and carry
        // the overlay intensity in the mask.
        assert_eq!(base.get_pixel(0, 0), &image::Rgba([0, 0, 0, 255]));
        assert_eq!(mask.pixels[0], 136);
        // Staff pixel: overlay alpha 0 must neither mask (its RGB is the
        // keyed-out background) nor touch the base.
        assert_eq!(base.get_pixel(1, 0), &image::Rgba([80, 50, 20, 255]));
        assert_eq!(mask.pixels[1], 0);
    }

    #[test]
    fn definition_picture_carries_its_cropped_color_by_owner_mask() {
        // C4Def::Picture2Facet takes PictureRect from the definition's
        // ColorByOwner-aware Graphics.GetBitmap(color) surface
        // (src/C4Def.cpp:1374-1378). LoadGraphics keeps Overlay.png as the
        // owner-color surface (src/C4DefGraphics.cpp:73-98), so the picture
        // crop must retain the matching mask instead of freezing its raw
        // Graphics.png colors.
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("Mage.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=MAGE\nColorByOwner=1\nPicture=1,0,1,1\n",
        )
        .expect("DefCore");

        let base = image::RgbaImage::from_pixel(3, 1, image::Rgba([0, 0, 0, 0]));
        base.save(def_dir.join("Graphics.png")).expect("base png");
        let mut overlay = image::RgbaImage::from_pixel(3, 1, image::Rgba([0, 0, 0, 0]));
        overlay.put_pixel(1, 0, image::Rgba([136, 136, 136, 255]));
        overlay
            .save(def_dir.join("Overlay.png"))
            .expect("overlay png");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");
        let picture = definition.picture_image.expect("picture crop");
        assert_eq!((picture.width(), picture.height()), (1, 1));
        assert_eq!(
            picture.pixels(),
            &[0, 0, 0, 255],
            "C++ removes owner-color Overlay pixels from the base surface before Picture2Facet"
        );
        let mask = definition
            .picture_color_by_owner_mask
            .expect("picture must retain owner-color mask");
        assert_eq!((mask.width, mask.height), (1, 1));
        assert_eq!(mask.pixels, vec![136]);
    }

    #[test]
    fn definition_portrait_carries_its_color_by_owner_mask() {
        // C4DefGraphics::LoadAllGraphics maps Portrait1.png to Overlay1.png
        // and loads both with ColorByOwner enabled (C4DefGraphics.cpp:166-205,
        // C4Def.cpp:1250-1264). DrawTextSpecImage later applies the requested
        // portrait color through GetBitmap(dwClr) (C4Game.cpp:4310-4324).
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("Sorcerer.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=SCLK\nColorByOwner=1\n",
        )
        .expect("DefCore");

        let base = image::RgbaImage::from_pixel(1, 1, image::Rgba([80, 50, 20, 0]));
        base.save(def_dir.join("Portrait1.png"))
            .expect("portrait png");
        let overlay = image::RgbaImage::from_pixel(1, 1, image::Rgba([136, 136, 136, 255]));
        overlay
            .save(def_dir.join("Overlay1.png"))
            .expect("portrait overlay png");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 0]))
            .save(def_dir.join("PortraitCaptain1.png"))
            .expect("named portrait png");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([64, 64, 64, 255]))
            .save(def_dir.join("OverlayCaptain1.png"))
            .expect("named portrait overlay png");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");
        let portrait = definition
            .portrait_graphics_image
            .expect("color-aware portrait image");
        assert_eq!(portrait.pixels(), &[0, 0, 0, 255]);
        let mask = definition
            .portrait_color_by_owner_mask
            .expect("portrait must retain owner-color mask");
        assert_eq!((mask.width, mask.height), (1, 1));
        assert_eq!(mask.pixels, vec![136]);
        let named = definition
            .portrait_graphics
            .iter()
            .find(|portrait| portrait.name.eq_ignore_ascii_case("captain1"))
            .expect("named portrait retained");
        assert_eq!(named.name, "Captain1");
        assert_eq!((named.image.width(), named.image.height()), (2, 1));
        assert_eq!(
            named
                .color_by_owner_mask
                .as_ref()
                .map(|mask| mask.pixels.as_slice()),
            Some([64, 64].as_slice())
        );
    }

    #[test]
    fn custom_rank_symbol_count_uses_localized_rank_file_priority() {
        // C4Def loads Rank{}.txt|Rank.txt with the active language sequence,
        // then reserves one trailing strip cell for each leading-'*' rank
        // extension (C4Def.cpp:659-706; C4RankSystem.cpp:96-180).
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("Ranked.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(def_dir.join("DefCore.txt"), b"[DefCore]\nid=RANK\n").expect("DefCore");
        image::RgbaImage::from_pixel(5, 1, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Rank.png"))
            .expect("rank strip");
        fs::write(def_dir.join("RankUS.txt"), b"Recruit\r\n*First %s\r\n").expect("US ranks");
        fs::write(
            def_dir.join("RankDE.txt"),
            b"Rekrut\r\n*Erster %s\r\n*Zweiter %s\r\n",
        )
        .expect("DE ranks");
        fs::write(
            def_dir.join("Rank.txt"),
            b"Fallback\n*One %s\n*Two %s\n*Three %s\n",
        )
        .expect("fallback ranks");

        let group = Group::open(&def_dir).expect("open definition");
        let us = Definition::load_with_languages(&group, &["US", "DE"])
            .expect("load US-priority definition");
        assert_eq!(us.rank_symbol_count, Some(4));
        assert_eq!(
            us.rank_names,
            Some(vec!["Recruit".to_string(), "First Recruit".to_string()])
        );

        let de = Definition::load_with_languages(&group, &["DE", "US"])
            .expect("load DE-priority definition");
        assert_eq!(de.rank_symbol_count, Some(3));
        assert_eq!(
            de.rank_names,
            Some(vec![
                "Rekrut".to_string(),
                "Erster Rekrut".to_string(),
                "Zweiter Rekrut".to_string(),
            ])
        );

        let fallback =
            Definition::load_with_languages(&group, &["FR"]).expect("load fallback definition");
        assert_eq!(fallback.rank_symbol_count, Some(2));
        assert_eq!(
            fallback.rank_names,
            Some(vec![
                "Fallback".to_string(),
                "One Fallback".to_string(),
                "Two Fallback".to_string(),
                "Three Fallback".to_string(),
            ])
        );
    }

    #[test]
    fn custom_rank_names_expand_extensions_in_cpp_order() {
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("ExpandedRanks.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(def_dir.join("DefCore.txt"), b"[DefCore]\nid=EXPR\n").expect("DefCore");
        fs::write(
            def_dir.join("RankUS.txt"),
            b"# comment\r\n*First %s\r\nBase=500\r\nRecruit\r\nIgnored=setting\r\nVeteran\r\n*100%% %s\r\nUnterminated",
        )
        .expect("rank names");

        let definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("open definition"),
            &["US"],
        )
        .expect("load definition");
        assert_eq!(
            definition.rank_names,
            Some(vec![
                "Recruit".to_string(),
                "Veteran".to_string(),
                "First Recruit".to_string(),
                "First Veteran".to_string(),
                "100% Recruit".to_string(),
                "100% Veteran".to_string(),
            ])
        );
    }

    #[test]
    fn custom_rank_symbol_count_matches_invalid_and_saturated_cpp_cases() {
        let temp = tempdir().expect("tempdir");

        let invalid_dir = temp.path().join("InvalidRanks.c4d");
        fs::create_dir(&invalid_dir).expect("invalid definition directory");
        fs::write(invalid_dir.join("DefCore.txt"), b"[DefCore]\nid=INVR\n").expect("DefCore");
        image::RgbaImage::from_pixel(4, 1, image::Rgba([255, 255, 255, 255]))
            .save(invalid_dir.join("Rank.png"))
            .expect("rank strip");
        fs::write(
            invalid_dir.join("RankUS.txt"),
            b"# no ordinary names\nBase=500\n*Unused %s\n",
        )
        .expect("invalid ranks");
        let invalid = Definition::load_with_languages(
            &Group::open(&invalid_dir).expect("open invalid definition"),
            &["US"],
        )
        .expect("load invalid-rank definition");
        assert_eq!(
            invalid.rank_symbol_count,
            Some(4),
            "C4RankSystem rejects a component without ordinary rank names"
        );
        assert_eq!(invalid.rank_names, None);

        let saturated_dir = temp.path().join("SaturatedRanks.c4d");
        fs::create_dir(&saturated_dir).expect("saturated definition directory");
        fs::write(saturated_dir.join("DefCore.txt"), b"[DefCore]\nid=SATR\n").expect("DefCore");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 255, 255, 255]))
            .save(saturated_dir.join("Rank.png"))
            .expect("rank strip");
        fs::write(
            saturated_dir.join("RankUS.txt"),
            b"Recruit\n*One %s\n*Two %s\n*Three %s\n",
        )
        .expect("saturated ranks");
        let saturated = Definition::load_with_languages(
            &Group::open(&saturated_dir).expect("open saturated definition"),
            &["US"],
        )
        .expect("load saturated-rank definition");
        assert_eq!(
            saturated.rank_symbol_count,
            Some(1),
            "C++ clamps the base rank symbol count to at least one"
        );
        assert_eq!(
            saturated.rank_names,
            Some(vec![
                "Recruit".to_string(),
                "One Recruit".to_string(),
                "Two Recruit".to_string(),
                "Three Recruit".to_string(),
            ])
        );

        fs::write(
            saturated_dir.join("RankUS.txt"),
            b"Recruit\n*Unterminated %s",
        )
        .expect("unterminated ranks");
        let unterminated = Definition::load_with_languages(
            &Group::open(&saturated_dir).expect("reopen saturated definition"),
            &["US"],
        )
        .expect("load unterminated-rank definition");
        assert_eq!(
            unterminated.rank_symbol_count,
            Some(2),
            "C++ ignores the final rank line when it has no CR or LF terminator"
        );
        assert_eq!(unterminated.rank_names, Some(vec!["Recruit".to_string()]));
    }

    #[test]
    fn corrupt_rank_png_does_not_fall_through_to_rank_bmp() {
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("BrokenRank.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(def_dir.join("DefCore.txt"), b"[DefCore]\nid=BRKN\n").expect("DefCore");
        fs::write(def_dir.join("Rank.png"), b"not a png").expect("corrupt PNG");
        image::RgbaImage::from_pixel(4, 1, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Rank.bmp"))
            .expect("valid BMP fallback candidate");

        let definition = Definition::load_with_languages(
            &Group::open(&def_dir).expect("open definition"),
            &["US"],
        )
        .expect("definition still loads");
        assert!(definition.rank_symbols_image.is_none());
        assert_eq!(definition.rank_symbol_count, None);
    }

    #[test]
    fn rank_strip_narrower_than_one_square_phase_is_rejected() {
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("NarrowRank.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(def_dir.join("DefCore.txt"), b"[DefCore]\nid=NARR\n").expect("DefCore");
        image::RgbaImage::from_pixel(1, 2, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Rank.png"))
            .expect("narrow rank strip");

        let definition = Definition::load(
            &Group::open(&def_dir).expect("open definition"),
        )
        .expect("definition loads");
        assert!(definition.rank_symbols_image.is_none());
        assert_eq!(definition.rank_symbol_count, None);
    }

    #[test]
    fn all_shipped_portrait_variants_are_retained_recursively() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Content"));
        if !root.is_dir() {
            return;
        }
        let mut definition_dirs = std::collections::BTreeSet::new();
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let portrait = name.starts_with("portrait")
                && (name.ends_with(".png") || name.ends_with(".bmp"));
            if portrait {
                let parent = entry.path().parent().expect("portrait has parent");
                if parent.join("DefCore.txt").is_file() {
                    definition_dirs.insert(parent.to_path_buf());
                }
            }
        }

        let mut checked = 0;
        for directory in definition_dirs {
            let expected = std::fs::read_dir(&directory)
                .expect("read definition directory")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .filter_map(|entry| {
                    let path = entry.path();
                    let stem = path.file_stem()?.to_string_lossy();
                    let extension = path.extension()?.to_string_lossy();
                    (stem
                        .get(..8)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Portrait"))
                        && (extension.eq_ignore_ascii_case("png")
                            || extension.eq_ignore_ascii_case("bmp")))
                    .then(|| stem.get(8..).unwrap_or_default().to_string())
                })
                .collect::<Vec<_>>();
            let definition = Definition::load(&Group::open(&directory).expect("open definition"))
                .expect("load shipped definition");
            for name in &expected {
                assert!(
                    definition
                        .portrait_graphics
                        .iter()
                        .any(|portrait| portrait.name.eq_ignore_ascii_case(name)),
                    "{} must retain Portrait{name}",
                    directory.display()
                );
            }
            checked += expected.len();
        }
        assert_eq!(checked, 75, "recursive shipped portrait census changed");
    }

    #[test]
    fn parse_def_core_basic_fields() {
        let data = br#"
            [DefCore]
            id=CLNK
            Name=Clonk
            Category=C4D_Living|C4D_Object
            CrewMember=1
            BlitMode=2
            MoveToRange=17
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.id, "CLNK");
        assert_eq!(parsed.name.as_deref(), Some("Clonk"));
        assert_eq!(parsed.category, (1 << 3) | (1 << 4));
        assert!(parsed.crew_member);
        assert_eq!(parsed.blit_mode, 2);
        assert_eq!(parsed.move_to_range, 17);
        assert_eq!(parsed.collection, None);
        assert_eq!(parsed.collection_limit, None);
        assert!(!parsed.collectible);

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("default parses");
        assert_eq!(defaulted.blit_mode, 0);
        assert_eq!(defaulted.move_to_range, 0);

        let signed = parse_def_core(b"[DefCore]\nid=SIGN\nMoveToRange=-3\n")
            .expect("signed range parses");
        assert_eq!(signed.move_to_range, -3);
    }

    #[test]
    fn parse_def_core_retains_the_five_component_cpp_version() {
        // C4DefCore::CompileFunc stores Version in the five-slot rC4XVer
        // array, zero-filling omitted components (src/C4Def.cpp:124,254).
        let parsed = parse_def_core(b"[DefCore]\nid=VERS\nVersion=4,9,1,3,27\n")
            .expect("versioned DefCore parses");
        assert_eq!(parsed.version, [4, 9, 1, 3, 27]);

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("defaults parse");
        assert_eq!(defaulted.version, [0; 5]);
    }

    #[test]
    fn parse_def_core_pathfinder_and_transfer_zone_policy() {
        // C4DefCore::CompileFunc reads both fields as integer defaults of
        // zero (C4Def.cpp:399,415); command code treats either sign of a
        // nonzero Pathfinder as enabled and SetLevel clamps it later.
        let parsed = parse_def_core(b"[DefCore]\nid=ROUT\nPathfinder=-4\nNoTransferZones=-2\n")
            .expect("pathfinder DefCore parses");
        assert_eq!(parsed.pathfinder, -4);
        assert_eq!(parsed.no_transfer_zones, -2);

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("defaults parse");
        assert_eq!(defaulted.pathfinder, 0);
        assert_eq!(defaulted.no_transfer_zones, 0);
    }

    #[test]
    fn parse_def_core_allow_picture_stack_bitfield() {
        // C4Def::CompileFunc parses AllowPictureStack through the APS_* table
        // (src/C4Def.cpp:419-429; src/C4Constants.h:301-309).
        let parsed = parse_def_core(
            b"[DefCore]\nid=STACK\nAllowPictureStack=APS_Color|APS_Graphics|APS_Name|APS_Overlay\n",
        )
        .expect("DefCore parses");
        assert_eq!(
            parsed.allow_picture_stack,
            APS_COLOR | APS_GRAPHICS | APS_NAME | APS_OVERLAY
        );

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("DefCore parses");
        assert_eq!(defaulted.allow_picture_stack, 0);
        assert_eq!(defaulted.graphics_scale, 100);

        let scaled = parse_def_core(b"[DefCore]\nid=SCALE\nScale=125\n")
            .expect("graphics scale parses");
        assert_eq!(scaled.graphics_scale, 125);
    }

    #[test]
    fn line_compiles_named_tokens_as_a_bitfield() {
        // C4Def::CompileFunc passes Line through mkBitfieldAdapt with the
        // C4D_Line_* table (C4Def.cpp:319-333). DrainPipe.c4d encodes the
        // drain value 3 as Power(1)|Source(2), not the Drain alias.
        let parsed = parse_def_core(
            b"[DefCore]\nid=DPIP\nLine=C4D_LinePower|C4D_LineSource\n",
        )
        .expect("drain-pipe DefCore parses");

        assert_eq!(parsed.line, 3);
    }

    #[test]
    fn definition_loads_nonempty_legacy_us_description() {
        // C4Def loads Desc{}.txt into C4Def::Desc and trims it before
        // exposing C4Def::GetDesc (C4Def.cpp:713-717). The Context menu
        // adds Info only for a nonempty GetDesc (C4ObjectMenu.cpp:410-423).
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("Hut3.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(def_dir.join("DefCore.txt"), b"[DefCore]\nid=HUT3\n").expect("DefCore");
        fs::write(def_dir.join("DescUS.txt"), b"  A safe home base.\r\n")
            .expect("US description");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");

        assert_eq!(definition.description(), Some("A safe home base."));
    }

    #[test]
    fn definition_name_uses_localized_names_component() {
        // C4Def::Load loads Names{}.txt|Names.txt after DefCore and replaces
        // C4Def::Name with the first language-sequence match
        // (C4Def.cpp:635-639; C4ComponentHost.cpp:238-260). HUT3 therefore
        // presents as "Cabin", not its DefCore fallback "Hut".
        let temp = tempdir().expect("tempdir");
        let def_dir = temp.path().join("Hut3.c4d");
        fs::create_dir(&def_dir).expect("definition directory");
        fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=HUT3\nName=Hut\n",
        )
        .expect("DefCore");
        fs::write(def_dir.join("Names.txt"), b"DE:H\xfctte\r\nUS:Cabin\r\n")
            .expect("localized names");

        let group = Group::open(&def_dir).expect("open definition");
        let definition = Definition::load(&group).expect("load definition");

        assert_eq!(definition.core.name.as_deref(), Some("Cabin"));
        let german = Definition::load_with_languages(&group, &["DE", "US"])
            .expect("load German definition name");
        assert_eq!(german.core.name.as_deref(), Some("Hütte"));
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
    fn parse_real_bird_defcore_physical_float() {
        // The real CRLF content file (skipped when the content tree is
        // absent) — pins the [Physical] Float=200 parse that drives the
        // DFA_FLOAT speed clamp.
        let Ok(bytes) = std::fs::read(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../content/Objects.c4d/Animals.c4d/Bird.c4d/DefCore.txt"
            ),
        ) else {
            return;
        };
        let core = parse_def_core(&bytes).expect("parses");
        assert_eq!(core.physical.float, 200, "[Physical] Float=200");
        assert_eq!(core.physical.energy, 40000, "[Physical] Energy=40000");
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
    fn parse_def_core_blast_shield_and_horizontal_fix() {
        // ContainBlast=1 shields contents from explosions (the DoExplosion
        // container walk, C4Effect.cpp:884; C4Def.cpp:380) and
        // HorizontalFix=1 exempts a def from shockwave flings
        // (C4Def::NoHorizontalMove, C4Def.cpp:383; read back through
        // GetDefCoreVal by BlastObjectsShockwaveCheck).
        let data = br#"
            [DefCore]
            id=HUT1
            Name=Hut
            ContainBlast=1
            HorizontalFix=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.contain_blast, 1);
        assert_eq!(parsed.no_horizontal_move, 1);

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.contain_blast, 0, "default: contents take blasts");
        assert_eq!(parsed.no_horizontal_move, 0, "default: movable");
    }

    #[test]
    fn parse_def_core_blast_incinerate() {
        // BlastIncinerate=N: incinerate when accumulated Damage reaches N
        // after a blast (C4Def.cpp:315, default 0 = off; consumed by
        // C4Object::Blast, C4Object.cpp:1421-1423).
        let data = br#"
            [DefCore]
            id=TRE1
            Name=Tree
            BlastIncinerate=50
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.blast_incinerate, 50);

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.blast_incinerate, 0, "default: no blast incinerate");
    }

    #[test]
    fn parse_def_core_fire_top_and_default() {
        // C4Shape::CompileFunc compiles FireTop directly into DefCore with
        // default zero (C4Shape.cpp:496-510; C4Def.cpp:300-302).
        let parsed = parse_def_core(b"[DefCore]\nid=WMPF\nFireTop=10\n")
            .expect("def core parses");
        assert_eq!(parsed.fire_top, 10);

        let defaulted =
            parse_def_core(b"[DefCore]\nid=NONE\n").expect("default def core parses");
        assert_eq!(defaulted.fire_top, 0);
    }

    #[test]
    fn parse_def_core_set_ocf_fields() {
        // The DefCore flags feeding C4Object::SetOCF (C4Object.cpp:526-666):
        // Entrance rect (C4Def.cpp:309), Exclusive (:313), Prey (:354),
        // Edible (:355), RotatedEntrance (:377), Chop -> Chopable (:378),
        // AttractLightning (:391), NoFight (:413).
        let data = br#"
            [DefCore]
            id=CSTL
            Name=Castle
            Entrance=-10,20,20,15
            Exclusive=1
            Prey=1
            Edible=1
            RotatedEntrance=45
            Chop=1
            AttractLightning=1
            NoFight=1
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(
            parsed.entrance,
            Some(PictureRect {
                x: -10,
                y: 20,
                width: 20,
                height: 15
            })
        );
        assert!(parsed.exclusive);
        assert!(parsed.prey);
        assert!(parsed.edible);
        assert_eq!(parsed.rotated_entrance, 45);
        assert!(parsed.chopable);
        assert!(parsed.attract_lightning);
        assert!(parsed.no_fight);

        let data = br#"
            [DefCore]
            id=STON
            Name=Stone
        "#;
        let parsed = parse_def_core(data).expect("def core parses");
        assert_eq!(parsed.entrance, None, "default: no entrance area");
        assert!(!parsed.exclusive);
        assert!(!parsed.prey);
        assert!(!parsed.edible);
        assert_eq!(parsed.rotated_entrance, 0);
        assert!(!parsed.chopable);
        assert!(!parsed.attract_lightning);
        assert!(!parsed.no_fight);
    }

    #[test]
    fn parse_act_map_records_object_disabled() {
        // ObjectDisabled= (C4ActionDef::Disabled, C4Def.cpp:106): actions
        // that suspend the object — they veto OCF_Collection and
        // OCF_FightReady (SetOCF, C4Object.cpp:597,608).
        let data = br#"
[Action]
Name=Build
Procedure=Build
ObjectDisabled=1

[Action]
Name=Walk
Procedure=Walk
"#;
        let map = parse_act_map(data).expect("act map parsed");
        assert!(map.get("Build").expect("build action").disabled);
        assert!(!map.get("Walk").expect("walk action").disabled);
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
    fn parse_def_core_no_get_preserves_signed_value_and_default() {
        // C4DefCore::CompileFunc stores NoGet as an int32_t with default 0
        // (src/C4Def.cpp:412; src/C4Def.h:264). Menu code treats any
        // nonzero value as excluding the object from get/activate menus.
        let parsed =
            parse_def_core(b"[DefCore]\nid=LOCK\nNoGet=-2\n").expect("NoGet DefCore parses");
        assert_eq!(parsed.no_get, -2);

        let defaulted = parse_def_core(b"[DefCore]\nid=OPEN\n").expect("default DefCore parses");
        assert_eq!(defaulted.no_get, 0);
    }

    #[test]
    fn parse_def_core_grab_put_get_bitfield() {
        // StdBitfieldAdapt tokens (src/C4Def.cpp:364-373), e.g. the Lorry's
        // `GrabPutGet=C4D_GrabGet|C4D_GrabPut`.
        let data = br#"
            [DefCore]
            id=LORY
            GrabPutGet=C4D_GrabGet|C4D_GrabPut
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.grab_put_get, 3);

        let data = br#"
            [DefCore]
            id=CONT
            GrabPutGet=C4D_GrabPut
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.grab_put_get, 1);

        let get_only = parse_def_core(b"[DefCore]\nid=GETR\nGrabPutGet=C4D_GrabGet\n")
            .expect("get-only DefCore parses");
        assert_eq!(get_only.grab_put_get, 2);

        // Hazard's shipped SupplyBox uses the equivalent decimal form.
        let numeric = parse_def_core(b"[DefCore]\nid=SUPP\nGrabPutGet=3\n")
            .expect("numeric GrabPutGet parses");
        assert_eq!(numeric.grab_put_get, 3);

        let defaulted = parse_def_core(b"[DefCore]\nid=NONE\n").expect("default DefCore parses");
        assert_eq!(defaulted.grab_put_get, 0);
    }

    #[test]
    fn parse_def_core_vehicle_control() {
        // Plain integer compile (src/C4Def.cpp:398), e.g. the Airship's
        // `VehicleControl=2` (C4D_VehicleControl_Inside).
        let data = br#"
            [DefCore]
            id=SHIP
            VehicleControl=2
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.vehicle_control, 2);

        let data = br#"
            [DefCore]
            id=CONT
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.vehicle_control, 0, "default 0");
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
    fn parse_def_core_top_face_target_rect() {
        // C4DefCore::CompileFunc reads TopFace as C4TargetRect
        // (src/C4Def.cpp:306); the last two values are the draw target.
        let data = br#"
            [DefCore]
            id=ELEC
            TopFace=0,1,24,26,-3,4
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(
            parsed.top_face,
            Some(TargetRect {
                x: 0,
                y: 1,
                width: 24,
                height: 26,
                target_x: -3,
                target_y: 4,
            })
        );

        let defaulted = parse_def_core(b"[DefCore]\nid=ELEC\n").expect("defcore parsed");
        assert_eq!(defaulted.top_face, None, "C4TargetRect defaults empty");
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
    fn parse_def_core_rotated_solidmasks_flag_like_cpp() {
        // Mirrors src/C4Def.cpp:414: DefCore `RotatedSolidmasks` compiles
        // with default 0; nonzero lets C4Object::UpdateSolidMask keep the
        // mask while rotated (src/C4Object.cpp:5655).
        let data = br#"
            [DefCore]
            id=ELEV
            RotatedSolidmasks=1
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert!(parsed.rotated_solid_masks);

        let defaulted = parse_def_core(b"[DefCore]\nid=HUT0\n").expect("defcore parsed");
        assert!(!defaulted.rotated_solid_masks);
    }

    #[test]
    fn parse_def_core_auto_context_menu_flag_like_cpp() {
        // Mirrors src/C4Def.cpp:416: DefCore `AutoContextMenu` compiles as
        // an integer flag. StdCompilerINIRead matches field names without
        // regard to case, so a value of 1 enables the flag.
        let parsed = parse_def_core(b"[DefCore]\nid=HUT3\naUtOcOnTeXtMeNu=1\n")
            .expect("defcore parsed");

        assert!(parsed.auto_context_menu);
    }

    #[test]
    fn parse_def_core_auto_context_menu_defaults_off_like_cpp() {
        // Mirrors the default argument in src/C4Def.cpp:416: an omitted
        // `AutoContextMenu` field compiles as zero.
        let parsed = parse_def_core(b"[DefCore]\nid=CLNK\n").expect("defcore parsed");

        assert!(!parsed.auto_context_menu);
    }

    #[test]
    fn parse_def_core_silent_commands_flag_and_default_like_cpp() {
        // C4Def::CompileFunc reads SilentCommands with a zero default
        // (src/C4Def.cpp:404), using the compiler's case-insensitive keys.
        let enabled = parse_def_core(b"[DefCore]\nid=CLNK\nsIlEnTcOmMaNdS=yes\n")
            .expect("defcore parsed");
        assert!(enabled.silent_commands);

        let defaulted = parse_def_core(b"[DefCore]\nid=ROCK\n").expect("defcore parsed");
        assert!(!defaulted.silent_commands);
    }

    #[test]
    fn parse_def_core_construct_to_as_build_turn_to_like_cpp() {
        // C4Def::CompileFunc exposes the BuildTurnTo field under the legacy
        // DefCore key `ConstructTo` (src/C4Def.cpp:361).
        let parsed = parse_def_core(b"[DefCore]\nid=SITE\ncOnStRuCtTo=DONE\n")
            .expect("defcore parsed");
        assert_eq!(parsed.build_turn_to.as_deref(), Some("DONE"));

        let defaulted = parse_def_core(b"[DefCore]\nid=SITE\n").expect("defcore parsed");
        assert!(defaulted.build_turn_to.is_none());

        let none = parse_def_core(b"[DefCore]\nid=SITE\nConstructTo=NONE\n")
            .expect("defcore parsed");
        assert!(none.build_turn_to.is_none());
    }

    #[test]
    fn parse_def_core_base_sale_flags_like_cpp() {
        // C4Def::CompileFunc reads Rebuy with default 0 and BaseAutoSell
        // with a GOLD-specific default of 1 (src/C4Def.cpp:359,457).
        let explicit = parse_def_core(
            b"[DefCore]\nid=ORE1\nRebuy=1\nBaseAutoSell=1\n",
        )
        .expect("defcore parsed");
        assert!(explicit.rebuyable);
        assert!(explicit.base_auto_sell);

        let gold = parse_def_core(b"[DefCore]\nid=GOLD\n").expect("defcore parsed");
        assert!(!gold.rebuyable);
        assert!(gold.base_auto_sell);

        let ordinary = parse_def_core(b"[DefCore]\nid=ROCK\n").expect("defcore parsed");
        assert!(!ordinary.rebuyable);
        assert!(!ordinary.base_auto_sell);
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
    fn parse_def_core_retains_dormant_vertex_slots_beyond_count() {
        // C4Shape::CompileFunc adapts all C4D_MaxVertex array slots even
        // when VtxNum is smaller (src/C4Shape.cpp:496-509). A later
        // AddVertex overwrites only X/Y, so this dormant CNAT/friction must
        // survive definition loading (src/C4Shape.cpp:26-31).
        let parsed = parse_def_core(
            b"[DefCore]\nid=TABB\nVertices=1\nVertexX=3,30\nVertexY=4,40\n\
              VertexCNAT=8,10\nVertexFriction=100,250\n",
        )
        .expect("defcore parsed");

        assert_eq!(parsed.vertices.len(), 1);
        assert_eq!(
            parsed.vertex_slots[1],
            DefVertex {
                x: 30,
                y: 40,
                cnat: 10,
                friction: 250,
            }
        );
        assert_eq!(parsed.vertex_slots.len(), C4D_MAX_VERTEX);
    }

    #[test]
    fn parse_def_core_components_list() {
        let data = br#"
            [DefCore]
            id=HUTS
            Components=WOOD:2,Metal=1; rock; ZERO=0; NEGA=-3
        "#;
        let parsed = parse_def_core(data).expect("defcore parsed");
        assert_eq!(parsed.components.len(), 5);
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
                count: 0
            }
        );
        assert_eq!(
            parsed.components[3],
            DefComponent {
                id: "ZERO".to_string(),
                count: 0
            }
        );
        assert_eq!(
            parsed.components[4],
            DefComponent {
                id: "NEGA".to_string(),
                count: -3
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
