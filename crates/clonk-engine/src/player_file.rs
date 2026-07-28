//! Legacy `.c4p` player files: C4PlayerInfoCore (`Player.txt`) plus the
//! crew roster of `*.c4i` child groups (C4ObjectInfoList::Load,
//! C4ObjectInfoList.cpp:56-83). The join pipeline consumes this to mirror
//! `C4Player::Load` (C4Player.cpp:1089-1107).

use std::collections::HashSet;

use clonk_resources::{Group, PhysicalInfo};
use serde::{Deserialize, Serialize};

use crate::scenario::ScenarioError;
use crate::{
    bounded_crew_portrait_file, bounded_loaded_crew_type_name, CrewInfoCoreFields,
    CrewPermanentPortrait, CrewPortrait, CrewPortraitState, DefinitionId,
};

/// C4StringTable IDs and live object numbers used when denumerating an
/// embedded runtime player's ExtraData. These are scenario-wide in native
/// C++; the player child group deliberately carries no private Strings.txt.
#[derive(Debug, Clone, Default)]
pub struct PersistedC4ValueResolution {
    pub strings: clonk_script::StringRegistrations,
    pub object_numbers: HashSet<u64>,
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn is_one_i32(value: &i32) -> bool {
    *value == 1
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_crew_rank_name() -> String {
    "Clonk".to_string()
}

fn default_player_name() -> String {
    "Neuling".to_string()
}

fn default_player_rank_name() -> String {
    // C4PlayerInfoCore::Default uses the built-in German fallback when no
    // process-local C4RankSystem is supplied. The localized compile default
    // is only relevant while omitting the field on write.
    "Rang".to_string()
}

fn default_pref_color_dw() -> u32 {
    0xff
}

fn default_pref_mouse() -> bool {
    true
}

fn default_pref_mouse_value() -> i32 {
    1
}

/// Exact persisted `C4RoundResult` nested below `[LastRound]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlayerLastRoundState {
    #[serde(
        default,
        with = "clonk_script::c4_string_serde",
        skip_serializing_if = "String::is_empty"
    )]
    pub title: String,
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub date: u32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub duration: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub won: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub score: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub final_score: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub total_score: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub bonus: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub level: i32,
}

fn u32_is_zero(value: &u32) -> bool {
    *value == 0
}

/// Complete `C4PlayerInfoCore` retained independently from the live
/// `C4Player` fields. In C++ this object owns profile identity/preferences
/// that cannot be reconstructed from the assigned in-round color or slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerInfoCoreState {
    #[serde(
        default = "default_player_name",
        with = "clonk_script::c4_string_serde"
    )]
    pub pref_name: String,
    #[serde(default, with = "clonk_script::c4_string_serde")]
    pub comment: String,
    #[serde(default)]
    pub rank: i32,
    #[serde(
        default = "default_player_rank_name",
        with = "clonk_script::c4_string_serde"
    )]
    pub rank_name: String,
    #[serde(default)]
    pub score: i32,
    #[serde(default)]
    pub rounds: i32,
    #[serde(default)]
    pub rounds_won: i32,
    #[serde(default)]
    pub rounds_lost: i32,
    #[serde(default)]
    pub total_playing_time: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_data: Vec<(String, clonk_script::Value)>,
    #[serde(default)]
    pub pref_color: i32,
    #[serde(default = "default_pref_color_dw")]
    pub pref_color_dw: u32,
    #[serde(default)]
    pub pref_color2_dw: u32,
    #[serde(default)]
    pub pref_control: i32,
    #[serde(default)]
    pub pref_control_style: bool,
    /// Exact persisted `PrefControlStyle` integer. Runtime consumers use the
    /// boolean projection above, but C4PlayerInfoCore stores and recompiles the
    /// original `int32_t` value.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pref_control_style_value: i32,
    #[serde(default)]
    pub pref_auto_context_menu: bool,
    /// Exact post-load `PrefAutoContextMenu` integer. An omitted `-1` compiler
    /// default has already inherited `PrefControlStyle` at this boundary.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pref_auto_context_menu_value: i32,
    #[serde(default)]
    pub pref_position: i32,
    #[serde(default = "default_pref_mouse")]
    pub pref_mouse: bool,
    /// Exact persisted `PrefMouse` integer; nonzero values remain enabled but
    /// are not normalized to one when C++ writes the core again.
    #[serde(
        default = "default_pref_mouse_value",
        skip_serializing_if = "is_one_i32"
    )]
    pub pref_mouse_value: i32,
    #[serde(default)]
    pub last_round: PlayerLastRoundState,
}

impl Default for PlayerInfoCoreState {
    fn default() -> Self {
        Self {
            pref_name: default_player_name(),
            comment: String::new(),
            rank: 0,
            rank_name: default_player_rank_name(),
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            extra_data: Vec::new(),
            pref_color: 0,
            pref_color_dw: default_pref_color_dw(),
            pref_color2_dw: 0,
            pref_control: 0,
            pref_control_style: false,
            pref_control_style_value: 0,
            pref_auto_context_menu: false,
            pref_auto_context_menu_value: 0,
            pref_position: 0,
            pref_mouse: true,
            pref_mouse_value: 1,
            last_round: PlayerLastRoundState::default(),
        }
    }
}

#[derive(Debug)]
struct ObjectInfoIniNode {
    name: String,
    value: Option<String>,
    indent: usize,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// The ordered subset of `StdCompilerINIRead::CreateNameTree` needed by
/// `C4ObjectInfoCore::CompileFunc`. Names are exact, and indentation decides
/// which nodes are direct children or siblings.
#[derive(Debug)]
struct ObjectInfoIniTree {
    nodes: Vec<ObjectInfoIniNode>,
}

impl ObjectInfoIniTree {
    fn parse(source: &str) -> Self {
        let source = source.split_once('\0').map_or(source, |(prefix, _)| prefix);
        let mut tree = Self {
            nodes: vec![ObjectInfoIniNode {
                name: String::new(),
                value: None,
                indent: 0,
                parent: None,
                children: Vec::new(),
            }],
        };
        let mut current = 0;

        for line in source.split(['\r', '\n']) {
            let bytes = line.as_bytes();
            let indent = bytes
                .iter()
                .take_while(|byte| matches!(**byte, b' ' | b'\t'))
                .count();
            let mut position = indent;
            let section = bytes.get(position) == Some(&b'[')
                && bytes.get(position + 1).is_some_and(u8::is_ascii_alphabetic);
            if section {
                position += 1;
            } else if !bytes.get(position).is_some_and(u8::is_ascii_alphabetic) {
                continue;
            }

            let node_indent = indent + usize::from(!section);
            // CreateNameTree changes its current tree position before it
            // validates the delimiter, so malformed dedented lines still
            // close an indented section.
            while current != 0 && tree.nodes[current].indent >= node_indent {
                current = tree.nodes[current].parent.unwrap_or(0);
            }

            let name_start = position;
            while bytes
                .get(position)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
            {
                position += 1;
            }
            let name = &line[name_start..position];
            while bytes
                .get(position)
                .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
            {
                position += 1;
            }
            let delimiter = if section { b']' } else { b'=' };
            if bytes.get(position) != Some(&delimiter) {
                continue;
            }
            position += 1;

            let index = tree.nodes.len();
            tree.nodes.push(ObjectInfoIniNode {
                name: name.to_string(),
                value: (!section).then(|| line[position..].to_string()),
                indent: node_indent,
                parent: Some(current),
                children: Vec::new(),
            });
            tree.nodes[current].children.push(index);
            if section {
                current = index;
            }
        }

        tree
    }

    fn first_named_child(&self, parent: usize, name: &str) -> Option<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|index| self.nodes[*index].name == name)
    }

    fn value(&self, parent: usize, name: &str) -> Option<&str> {
        self.first_named_child(parent, name).map(|index| {
            // Naming lookup consumes the first matching node even if it was
            // written with section syntax and therefore has no value payload.
            self.nodes[index].value.as_deref().unwrap_or("")
        })
    }

    fn followed_root_physical(&self, object_info: usize) -> Option<usize> {
        let parent = self.nodes[object_info].parent?;
        let siblings = &self.nodes[parent].children;
        let position = siblings.iter().position(|index| *index == object_info)?;
        let next = *siblings.get(position + 1)?;
        if self.nodes[next].name != "Physical" {
            return None;
        }

        // FollowName first validates the next sibling, removes ObjectInfo,
        // then performs a fresh Name("Physical") lookup from the parent.
        // Consequently, an earlier Physical node wins this second lookup.
        self.first_named_child(parent, "Physical")
    }
}

fn projected_object_info_value(value: &str) -> String {
    // StdCompilerINIRead::ReadString(RCT_All) skips only spaces and tabs
    // immediately after `=`. Everything else through the physical line end,
    // including trailing whitespace and `//`, is string data.
    value.trim_start_matches([' ', '\t']).to_string()
}

/// One crew-roster entry: C4ObjectInfoCore (C4InfoCore.cpp:526-548) with
/// the runtime recruitment flags (C4ObjectInfo::InAction / HasDied) that
/// `GetIdle` filters on (C4ObjectInfoList.cpp:113-142) — both start clear
/// when loaded from file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewInfo {
    /// `id` (C4ID of the crew definition; "Clonk" defaults to none here —
    /// C4ID_None loads stay unresolvable like C++).
    pub id: String,
    /// `Name` (default "Clonk").
    #[serde(with = "clonk_script::c4_string_serde")]
    pub name: String,
    /// `DeathMessage` (default empty), emitted verbatim by DeathAnnounce.
    #[serde(
        default,
        with = "clonk_script::c4_string_serde",
        skip_serializing_if = "String::is_empty"
    )]
    pub death_message: String,
    /// Remaining persisted scalar C4ObjectInfoCore fields.
    #[serde(default, flatten)]
    pub core: CrewInfoCoreFields,
    /// `Rank` (default 0).
    pub rank: i32,
    /// Persisted `C4ObjectInfoCore::sRankName` (`RankName`, default "Clonk").
    #[serde(default = "default_crew_rank_name")]
    pub rank_name: String,
    /// `Experience` (default 0) — GetIdle prefers the highest.
    pub experience: i32,
    /// Persistent number of rounds in which this crew info participated.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub rounds: i32,
    /// Persistent `C4ObjectInfoCore::Physical`, compiled from the sibling
    /// `[Physical]` section and promotion-updated for `rank` on load.
    #[serde(default)]
    pub physical: PhysicalInfo,
    /// Persistent death tally (`C4ObjectInfoCore::DeathCount`).
    #[serde(default)]
    pub death_count: i32,
    /// Persistent active-play seconds (C4ObjectInfoCore::TotalPlayingTime).
    #[serde(default)]
    pub total_playing_time: i32,
    /// Persistent Unix-time birthday (`C4ObjectInfoCore::Birthday`).
    #[serde(default)]
    pub birthday: i32,
    /// Cached five-playing-hour age used by `C4Object::ExecLife`.
    #[serde(default)]
    pub age: i32,
    /// `Participation` (default 1) — GetIdle requires 1.
    pub participation: i32,
    /// Recruited this round (C4ObjectInfo::InAction).
    pub in_action: bool,
    /// Sticky per-round participation bit (`C4ObjectInfo::WasInAction`).
    /// Retiring or dying clears `in_action` but not this flag.
    #[serde(default, skip_serializing_if = "is_false")]
    pub was_in_action: bool,
    /// Game time at the last Recruit call; meaningful only in action.
    #[serde(default)]
    pub in_action_time: i32,
    /// Died this round (C4ObjectInfo::HasDied).
    pub has_died: bool,
    /// `C4ObjectInfoCore::ExtraData`: ordered named C4Value slots persisted
    /// with this crew entry. GetCrewExtraData returns any stored value;
    /// SetCrewExtraData limits newly written types separately.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_data: Vec<(String, clonk_script::Value)>,
    /// Current, saved-fallback, and pending-permanent portrait state owned by
    /// this exact C4ObjectInfo roster entry.
    #[serde(default)]
    pub portraits: CrewPortraitState,
}

/// `C4ObjectInfoCore::Default` (C4InfoCore.cpp:498-524): a nameless crew
/// entry at rank 0 with the "Clonk" rank name and `Participation` 1.
///
/// The roster literals across the engine and its tests all restated these
/// same defaults field by field; they now override only what they mean to
/// change.
impl Default for CrewInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            death_message: String::new(),
            core: CrewInfoCoreFields::default(),
            rank: 0,
            rank_name: default_crew_rank_name(),
            experience: 0,
            rounds: 0,
            physical: PhysicalInfo::default(),
            death_count: 0,
            total_playing_time: 0,
            birthday: 0,
            age: 0,
            participation: 1,
            in_action: false,
            was_in_action: false,
            in_action_time: 0,
            has_died: false,
            extra_data: Vec::new(),
            portraits: CrewPortraitState::default(),
        }
    }
}

impl CrewInfo {
    /// Loads one `*.c4i` crew group, including its embedded custom-portrait
    /// state, with the same parser used by [`PlayerFile::load`].
    pub fn load(group: &Group) -> Result<Self, ScenarioError> {
        Self::load_with_filename(group, &[], true, None)
    }

    fn load_with_filename(
        group: &Group,
        filename: &[u8],
        load_unnamed_portrait: bool,
        value_resolution: Option<&PersistedC4ValueResolution>,
    ) -> Result<Self, ScenarioError> {
        let bytes = group.read_file("ObjectInfo.txt")?;
        let source = clonk_script::c4_string_from_bytes(&bytes);
        let assets = load_crew_portrait_assets(group);
        let mut info = Self::from_object_info_source(
            &source,
            assets.loaded,
            load_unnamed_portrait,
            value_resolution,
        );
        info.core.original_filename = clonk_script::c4_string_from_bytes(filename);
        if load_unnamed_portrait || info.core.portrait_file == "custom" {
            info.core.portrait_png = assets.png;
            info.core.portrait_overlay_png = assets.overlay_png;
            info.core.portrait_bmp = assets.bmp;
        }
        Ok(info)
    }

    fn from_object_info_source(
        source: &str,
        has_custom_portrait: bool,
        load_unnamed_portrait: bool,
        value_resolution: Option<&PersistedC4ValueResolution>,
    ) -> Self {
        let tree = ObjectInfoIniTree::parse(source);
        let object_info = tree.first_named_child(0, "ObjectInfo");
        let physical_section = object_info.and_then(|index| tree.followed_root_physical(index));
        let entry = |parent: Option<usize>, key: &str| -> Option<String> {
            parent
                .and_then(|parent| tree.value(parent, key))
                .map(projected_object_info_value)
        };
        let int = |key: &str, default: i32| -> i32 {
            entry(object_info, key)
                .and_then(|value| parse_leading_i32(&value))
                .unwrap_or(default)
        };
        let rank = int("Rank", 0);
        let mut physical = PhysicalInfo::default();
        for name in [
            "Energy",
            "Breath",
            "Walk",
            "Jump",
            "Scale",
            "Hangle",
            "Dig",
            "Swim",
            "Throw",
            "Push",
            "Fight",
            "Magic",
            "Float",
            "CanScale",
            "CanHangle",
            "CanDig",
            "CanConstruct",
            "CanChop",
            "CanFly",
            "CorrosionResist",
            "BreatheWater",
        ] {
            if let Some(value) =
                entry(physical_section, name).and_then(|value| parse_leading_i32(&value))
            {
                physical.set_by_name(name, value);
            }
        }
        let physical = crate::promotion_updated_physical(physical, rank, None);
        let id = entry(object_info, "id").unwrap_or_default();
        let mut portrait_file = entry(object_info, "PortraitFile")
            .map(|name| bounded_crew_portrait_file(&name))
            .unwrap_or_default();
        let portraits = loaded_portrait_state(
            &id,
            &mut portrait_file,
            has_custom_portrait,
            load_unnamed_portrait,
        );
        let death_message = object_info
            .and_then(|parent| tree.value(parent, "DeathMessage"))
            .map(|value| value.trim_start_matches([' ', '\t']).to_string())
            .map(normalize_death_message)
            .unwrap_or_default();
        Self {
            id,
            name: entry(object_info, "Name").unwrap_or_else(|| "Clonk".to_string()),
            death_message,
            core: CrewInfoCoreFields {
                portrait_file,
                next_rank_name: entry(object_info, "NextRankName")
                    .map(|value| decode_escaped_ini_string(&value))
                    .unwrap_or_default(),
                type_name: entry(object_info, "TypeName")
                    .map(|name| bounded_loaded_crew_type_name(&name))
                    .unwrap_or_else(|| "Clonk".to_string()),
                next_rank_exp: int("NextRankExp", 0),
                ..CrewInfoCoreFields::default()
            },
            rank,
            rank_name: entry(object_info, "RankName")
                .map(|value| decode_escaped_ini_string(&value))
                .unwrap_or_else(default_crew_rank_name),
            experience: int("Experience", 0),
            rounds: int("Rounds", 0),
            physical,
            death_count: int("DeathCount", 0),
            total_playing_time: int("TotalPlayingTime", 0),
            birthday: int("Birthday", 0),
            age: int("Age", 0),
            participation: int("Participation", 1),
            in_action: false,
            was_in_action: false,
            in_action_time: 0,
            has_died: false,
            extra_data: entry(object_info, "ExtraData")
                .and_then(|value| parse_persisted_value_map(&value, value_resolution).ok())
                .unwrap_or_default(),
            portraits,
        }
    }
}

/// `C4ObjectInfoCore::DeathMessage` is a 75-byte C string. Compile replaces
/// a leading `@` with a space so saved crew cannot make the announcement
/// permanent (`C4InfoCore.cpp:526-559`).
fn normalize_death_message(value: String) -> String {
    let mut bytes = clonk_script::c4_string_bytes(&value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes.truncate(75);
    if bytes.first() == Some(&b'@') {
        bytes[0] = b' ';
    }
    clonk_script::c4_string_from_bytes(&bytes)
}

fn loaded_portrait_state(
    own_definition: &str,
    portrait_file: &mut String,
    has_custom_portrait: bool,
    load_unnamed_portrait: bool,
) -> CrewPortraitState {
    let custom = || CrewPortrait {
        source: None,
        name: "custom".to_string(),
    };
    if portrait_file == "custom" {
        if has_custom_portrait {
            let portrait = custom();
            return CrewPortraitState {
                current: Some(portrait.clone()),
                fallback: Some(portrait),
                permanent: CrewPermanentPortrait::Absent,
            };
        }
        // C4ObjectInfo::Load clears a stale `custom` spec when neither the
        // old BMP nor PNG payload can be loaded.
        portrait_file.clear();
        return CrewPortraitState::default();
    }
    if portrait_file.is_empty() && has_custom_portrait && load_unnamed_portrait {
        // The legacy import path owns the current graphics directly and
        // writes "custom" into PortraitFile, but does not create
        // pCustomPortrait. Permanent GetPortrait therefore evaluates that
        // string with the info's own definition ID.
        *portrait_file = "custom".to_string();
        return CrewPortraitState {
            current: Some(custom()),
            fallback: Some(CrewPortrait {
                source: Some(DefinitionId::from(own_definition)),
                name: "custom".to_string(),
            }),
            permanent: CrewPermanentPortrait::Absent,
        };
    }
    if portrait_file.is_empty() {
        return CrewPortraitState::default();
    }

    let portrait = evaluate_portrait_string(portrait_file, own_definition);
    CrewPortraitState {
        current: (portrait_file != "none").then(|| portrait.clone()),
        fallback: Some(portrait),
        permanent: CrewPermanentPortrait::Absent,
    }
}

fn evaluate_portrait_string(spec: &str, own_definition: &str) -> CrewPortrait {
    let bytes = spec.as_bytes();
    if bytes.len() > 6 && bytes[4] == b':' && bytes[5] == b':' {
        let tail = &spec[6..];
        let name = tail
            .split_once("::")
            .map_or(tail, |(_, portrait_name)| portrait_name);
        CrewPortrait {
            source: Some(DefinitionId::from(&spec[..4])),
            name: name.to_string(),
        }
    } else {
        CrewPortrait {
            source: Some(DefinitionId::from(own_definition)),
            name: spec.to_string(),
        }
    }
}

/// The parsed player file: C4PlayerInfoCore (C4InfoCore.cpp:148-177) and
/// the crew roster in group order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerFile {
    /// Complete retained player core. The long-standing flat fields below
    /// remain the join projection and override their corresponding values in
    /// [`PlayerFile::exact_info_core`] so synthetic callers stay compatible.
    pub info_core: PlayerInfoCoreState,
    /// `[Player] Name` (default "Neuling").
    pub name: String,
    /// `[Player] Score`, the persistent settlement score
    /// (C4InfoCore.cpp:156; default 0).
    pub score: i32,
    /// Persistent completed-round counters from `C4PlayerInfoCore`.
    pub rounds: i32,
    pub rounds_won: i32,
    pub rounds_lost: i32,
    /// `[Player] TotalPlayingTime` in seconds
    /// (C4InfoCore.cpp:160; default 0).
    pub total_playing_time: i32,
    /// `[Preferences] Color` — the indexed preferred color (default 0).
    pub pref_color: i32,
    /// `[Preferences] ColorDw` — 24-bit RGB preference (default 0xff).
    pub pref_color_dw: u32,
    /// `[Preferences] AlternateColorDw` — host-local fallback color used by
    /// `C4PlayerInfoListAttributeConflictResolver` (default 0 / absent).
    pub pref_color2_dw: u32,
    /// `[Preferences] Position` — preferred start position (default 0).
    pub pref_position: i32,
    /// `[Preferences] Control` — raw preferred control set. Synthetic cores
    /// start on Keyboard1 (0); loaded files default an omitted key to
    /// Keyboard2 (1).
    pub pref_control: i32,
    /// `[Preferences] Mouse` — whether this player initially requests mouse
    /// control. C++ treats every nonzero stored integer as enabled.
    pub pref_mouse: bool,
    /// `[Preferences] AutoStopControl` — PrefControlStyle: Jump'n'Run
    /// control when 1 (C4InfoCore.cpp:170; default 0 = classic, :84).
    pub pref_control_style: bool,
    /// `[Preferences] AutoContextMenu` — automatically open context menus
    /// when entering opted-in containers. If omitted, C++ defaults this to
    /// `pref_control_style` (C4InfoCore.cpp:103-115,171).
    pub pref_auto_context_menu: bool,
    /// Crew roster, `*.c4i` entries in group order then subfolder recursion
    /// (C4ObjectInfoList.cpp:56-83).
    pub crew: Vec<CrewInfo>,
}

impl Default for PlayerFile {
    fn default() -> Self {
        Self {
            info_core: PlayerInfoCoreState::default(),
            name: "Neuling".to_string(),
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            pref_color: 0,
            pref_color_dw: 0xff,
            pref_color2_dw: 0,
            pref_position: 0,
            pref_control: 0,
            pref_mouse: true,
            pref_control_style: false,
            pref_auto_context_menu: false,
            crew: Vec::new(),
        }
    }
}

impl PlayerFile {
    /// Return the authoritative core with compatibility projection fields
    /// folded back in. This mirrors the single inherited C++ core after its
    /// live counters/preferences have been changed by callers.
    pub fn exact_info_core(&self) -> PlayerInfoCoreState {
        let mut core = self.info_core.clone();
        core.pref_name = self.name.clone();
        core.score = self.score;
        core.rounds = self.rounds;
        core.rounds_won = self.rounds_won;
        core.rounds_lost = self.rounds_lost;
        core.total_playing_time = self.total_playing_time;
        core.pref_color = self.pref_color;
        core.pref_color_dw = self.pref_color_dw;
        core.pref_color2_dw = self.pref_color2_dw;
        core.pref_position = self.pref_position;
        core.pref_control = self.pref_control;
        core.pref_mouse = self.pref_mouse;
        if (core.pref_mouse_value != 0) != self.pref_mouse {
            core.pref_mouse_value = i32::from(self.pref_mouse);
        }
        core.pref_control_style = self.pref_control_style;
        if (core.pref_control_style_value != 0) != self.pref_control_style {
            core.pref_control_style_value = i32::from(self.pref_control_style);
        }
        core.pref_auto_context_menu = self.pref_auto_context_menu;
        if (core.pref_auto_context_menu_value != 0) != self.pref_auto_context_menu {
            core.pref_auto_context_menu_value = i32::from(self.pref_auto_context_menu);
        }
        core
    }

    /// `C4PlayerInfoCore::GetPrefColorValue`: use the 24-bit ColorDw when
    /// nonzero, otherwise map the indexed legacy color with the stock table.
    pub fn normalized_preferred_color(&self) -> u32 {
        if self.pref_color_dw != 0 {
            return self.pref_color_dw & 0x00ff_ffff;
        }
        const PLAYER_COLORS: [u32; 12] = [
            0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xc48444, 0x784830, 0xa04400, 0xf08050,
            0x848484, 0xffffff, 0x0094f8, 0xbc00c0,
        ];
        usize::try_from(self.pref_color)
            .ok()
            .and_then(|index| PLAYER_COLORS.get(index))
            .copied()
            .unwrap_or(0xaaaaaa)
    }

    /// `C4PlayerInfoCore::Load` masks AlternateColorDw to 24-bit RGB. Zero
    /// remains meaningful: it tells the host that no alternate was selected.
    pub fn normalized_alternate_color(&self) -> u32 {
        self.pref_color2_dw & 0x00ff_ffff
    }

    pub fn load(group: &Group) -> Result<Self, ScenarioError> {
        Self::load_with_portraits(group, true)
    }

    /// The C4Player::Init remote-player path still resolves explicit portrait
    /// specs/custom payloads but does not adopt an otherwise unnamed embedded
    /// portrait (C4ObjectInfo.cpp:79-151).
    pub fn load_with_portraits(
        group: &Group,
        load_unnamed_portraits: bool,
    ) -> Result<Self, ScenarioError> {
        Self::load_with_portraits_and_optional_value_resolution(group, load_unnamed_portraits, None)
    }

    pub fn load_with_portraits_and_value_resolution(
        group: &Group,
        load_unnamed_portraits: bool,
        value_resolution: &PersistedC4ValueResolution,
    ) -> Result<Self, ScenarioError> {
        Self::load_with_portraits_and_optional_value_resolution(
            group,
            load_unnamed_portraits,
            Some(value_resolution),
        )
    }

    fn load_with_portraits_and_optional_value_resolution(
        group: &Group,
        load_unnamed_portraits: bool,
        value_resolution: Option<&PersistedC4ValueResolution>,
    ) -> Result<Self, ScenarioError> {
        let core_bytes = group.read_file("Player.txt")?;
        // Player core strings remain native C4 bytes until a presentation
        // consumer decodes them.
        let core_text = clonk_script::c4_string_from_bytes(&core_bytes);
        let tree = ObjectInfoIniTree::parse(&core_text);
        let entry = |section: &str, key: &str| -> Option<String> {
            tree.first_named_child(0, section)
                .and_then(|section| tree.value(section, key))
                .map(projected_object_info_value)
        };
        let int = |section: &str, key: &str, default: i32| -> i32 {
            entry(section, key)
                .and_then(|value| parse_leading_i32(&value))
                .unwrap_or(default)
        };

        let mut crew = Vec::new();
        collect_crew(group, &mut crew, load_unnamed_portraits, value_resolution)?;
        let pref_control_style_value = int("Preferences", "AutoStopControl", 0);
        let pref_control_style = pref_control_style_value != 0;
        let pref_auto_context_menu_value = match int("Preferences", "AutoContextMenu", -1) {
            -1 => pref_control_style_value,
            value => value,
        };
        let pref_auto_context_menu = pref_auto_context_menu_value != 0;

        let mut pref_name = bounded_player_string(
            &entry("Player", "Name").unwrap_or_else(default_player_name),
            30,
        );
        clonk_core::std_markup::Markup::strip_markup(&mut pref_name);
        let pref_color = int("Preferences", "Color", 0);
        let pref_color_dw_raw = entry("Preferences", "ColorDw")
            .and_then(|value| parse_leading_i32(&value))
            .map(|value| value as u32)
            .unwrap_or(0xff);
        let pref_color_dw = if pref_color_dw_raw == 0 {
            preferred_color_from_index(pref_color)
        } else {
            pref_color_dw_raw & 0x00ff_ffff
        };
        let pref_color2_dw = int("Preferences", "AlternateColorDw", 0) as u32 & 0x00ff_ffff;
        let pref_position = int("Preferences", "Position", 0);
        let pref_control = int("Preferences", "Control", 1);
        let pref_mouse_value = int("Preferences", "Mouse", 1);
        let pref_mouse = pref_mouse_value != 0;
        let info_core = PlayerInfoCoreState {
            pref_name: pref_name.clone(),
            comment: bounded_player_string(&entry("Player", "Comment").unwrap_or_default(), 256),
            rank: int("Player", "Rank", 0),
            // English is the canonical resource language for headless Rust
            // serialization. A present localized value remains verbatim.
            rank_name: bounded_player_string(
                &entry("Player", "RankName").unwrap_or_else(|| "Rank".to_string()),
                30,
            ),
            score: int("Player", "Score", 0),
            rounds: int("Player", "Rounds", 0),
            rounds_won: int("Player", "RoundsWon", 0),
            rounds_lost: int("Player", "RoundsLost", 0),
            total_playing_time: int("Player", "TotalPlayingTime", 0),
            extra_data: entry("Player", "ExtraData")
                .and_then(|value| parse_persisted_value_map(&value, value_resolution).ok())
                .unwrap_or_default(),
            pref_color,
            pref_color_dw,
            pref_color2_dw,
            pref_control,
            pref_control_style,
            pref_control_style_value,
            pref_auto_context_menu,
            pref_auto_context_menu_value,
            pref_position,
            pref_mouse,
            pref_mouse_value,
            last_round: PlayerLastRoundState {
                title: entry("LastRound", "Title")
                    .map(|title| decode_escaped_ini_string(&title))
                    .unwrap_or_default(),
                date: int("LastRound", "Date", 0) as u32,
                duration: int("LastRound", "Duration", 0),
                won: int("LastRound", "Won", 0),
                score: int("LastRound", "Score", 0),
                final_score: int("LastRound", "FinalScore", 0),
                total_score: int("LastRound", "TotalScore", 0),
                bonus: int("LastRound", "Bonus", 0),
                level: int("LastRound", "Level", 0),
            },
        };

        Ok(Self {
            name: pref_name,
            score: info_core.score,
            rounds: info_core.rounds,
            rounds_won: info_core.rounds_won,
            rounds_lost: info_core.rounds_lost,
            total_playing_time: info_core.total_playing_time,
            pref_color,
            pref_color_dw,
            pref_color2_dw,
            pref_position,
            pref_control,
            pref_mouse,
            pref_control_style,
            pref_auto_context_menu,
            crew,
            info_core,
        })
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self, ScenarioError> {
        let group = Group::open(path)?;
        Self::load(&group)
    }

    pub fn load_from_bytes(path: std::path::PathBuf, data: Vec<u8>) -> Result<Self, ScenarioError> {
        let group = Group::from_memory(path, data)?;
        Self::load(&group)
    }

    pub fn load_from_bytes_with_portraits(
        path: std::path::PathBuf,
        data: Vec<u8>,
        load_unnamed_portraits: bool,
    ) -> Result<Self, ScenarioError> {
        let group = Group::from_memory(path, data)?;
        Self::load_with_portraits(&group, load_unnamed_portraits)
    }

    pub fn load_from_bytes_with_portraits_and_value_resolution(
        path: std::path::PathBuf,
        data: Vec<u8>,
        load_unnamed_portraits: bool,
        value_resolution: &PersistedC4ValueResolution,
    ) -> Result<Self, ScenarioError> {
        let group = Group::from_memory(path, data)?;
        Self::load_with_portraits_and_value_resolution(
            &group,
            load_unnamed_portraits,
            value_resolution,
        )
    }
}

/// `C4ObjectInfoList::Load` (C4ObjectInfoList.cpp:56-83): all `*.c4i`
/// child groups in entry order, then recursion into remaining subgroups.
fn collect_crew(
    group: &Group,
    crew: &mut Vec<CrewInfo>,
    load_unnamed_portraits: bool,
    value_resolution: Option<&PersistedC4ValueResolution>,
) -> Result<(), ScenarioError> {
    let mut subgroups = Vec::new();
    for entry in group.entries()? {
        if std::env::var("LC_C4P_DEBUG").is_ok() {
            eprintln!("C4P entry: {entry:?}");
        }
        let is_info = entry
            .name_bytes
            .get(entry.name_bytes.len().saturating_sub(4)..)
            .is_some_and(|extension| extension.eq_ignore_ascii_case(b".c4i"));
        let child = if group.is_directory() {
            group.open_child(&entry.relative_path)
        } else {
            group.read_entry_bytes_exact(&entry).and_then(|bytes| {
                Group::from_raw_memory(path_from_group_name_bytes(&entry.name_bytes), bytes)
            })
        };
        let Ok(child) = child else {
            continue;
        };
        if is_info {
            if let Ok(info) = CrewInfo::load_with_filename(
                &child,
                &entry.name_bytes,
                load_unnamed_portraits,
                value_resolution,
            ) {
                crew.push(info);
            }
        }
        subgroups.push(child);
    }
    for child in subgroups {
        collect_crew(&child, crew, load_unnamed_portraits, value_resolution)?;
    }
    Ok(())
}

fn path_from_group_name_bytes(bytes: &[u8]) -> std::path::PathBuf {
    clonk_resources::path_from_legacy_bytes(bytes)
}

#[derive(Default)]
struct LoadedCrewPortraitAssets {
    loaded: bool,
    png: Vec<u8>,
    overlay_png: Vec<u8>,
    bmp: Vec<u8>,
}

fn load_crew_portrait_assets(group: &Group) -> LoadedCrewPortraitAssets {
    let Ok(entries) = group.entries() else {
        return LoadedCrewPortraitAssets::default();
    };
    let find = |name: &str| {
        entries.iter().find(|entry| {
            !entry.is_directory
                && entry.relative_path.components().count() == 1
                && entry
                    .relative_path
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
        })
    };
    let read_decode = |entry: &clonk_resources::GroupEntry, format| {
        let bytes = group.read_entry_bytes_exact(entry).ok()?;
        let decoded = image::load_from_memory_with_format(&bytes, format).ok()?;
        Some((bytes, decoded))
    };

    // C4DefGraphics::LoadGraphics tries an existing PNG first and does not
    // fall back to the old BMP when that PNG is corrupt.
    let (png, bmp, base) = if let Some(png) = find("Portrait.png") {
        let Some((bytes, decoded)) = read_decode(png, image::ImageFormat::Png) else {
            return LoadedCrewPortraitAssets::default();
        };
        (bytes, Vec::new(), decoded)
    } else {
        let Some((bytes, decoded)) =
            find("Portrait.bmp").and_then(|bmp| read_decode(bmp, image::ImageFormat::Bmp))
        else {
            return LoadedCrewPortraitAssets::default();
        };
        (Vec::new(), bytes, decoded)
    };
    let overlay_png = if let Some(overlay) = find("PortraitOverlay.png") {
        let Some((bytes, overlay)) = read_decode(overlay, image::ImageFormat::Png) else {
            return LoadedCrewPortraitAssets::default();
        };
        if overlay.width() != base.width() || overlay.height() != base.height() {
            return LoadedCrewPortraitAssets::default();
        }
        bytes
    } else {
        Vec::new()
    };
    LoadedCrewPortraitAssets {
        loaded: true,
        png,
        overlay_png,
        bmp,
    }
}

/// StdCompilerINIRead numbers parse strtol-style: leading integer, trailing
/// junk ignored.
fn parse_leading_i32(value: &str) -> Option<i32> {
    let trimmed = value.trim_start();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        let end = hex
            .char_indices()
            .take_while(|&(_, ch)| ch.is_ascii_hexdigit())
            .map(|(index, ch)| index + ch.len_utf8())
            .last()?;
        return i64::from_str_radix(&hex[..end], 16)
            .ok()
            .map(|value| value as i32);
    }
    let end = trimmed
        .char_indices()
        .take_while(|&(index, ch)| ch.is_ascii_digit() || (index == 0 && (ch == '-' || ch == '+')))
        .map(|(index, ch)| index + ch.len_utf8())
        .last()?;
    trimmed[..end].parse::<i64>().ok().map(|v| v as i32)
}

fn bounded_player_string(value: &str, max_bytes: usize) -> String {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes.truncate(max_bytes);
    clonk_script::c4_string_from_bytes(&bytes)
}

fn preferred_color_from_index(index: i32) -> u32 {
    const PLAYER_COLORS: [u32; 12] = [
        0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xc48444, 0x784830, 0xa04400, 0xf08050, 0x848484,
        0xffffff, 0x0094f8, 0xbc00c0,
    ];
    usize::try_from(index)
        .ok()
        .and_then(|index| PLAYER_COLORS.get(index))
        .copied()
        .unwrap_or(0xaaaaaa)
}

fn parse_persisted_value_map(
    encoded: &str,
    resolution: Option<&PersistedC4ValueResolution>,
) -> Result<Vec<(String, clonk_script::Value)>, ScenarioError> {
    let (count, payload) = encoded
        .split_once(';')
        .map_or((encoded, None), |(count, payload)| (count, Some(payload)));
    let mut count_parser = PersistedC4ValueParser::new(count, resolution);
    let count = count_parser.integer().unwrap_or(0);
    if count == 0 {
        return Ok(Vec::new());
    }
    let count = usize::try_from(count).map_err(|_| {
        ScenarioError::LegacyParse(format!("negative C4ValueMapData count `{count}`"))
    })?;
    if count > 1_000_000 {
        return Err(ScenarioError::LegacyParse(format!(
            "C4ValueMapData count `{count}` exceeds C4Value MaxSize"
        )));
    }
    let payload = payload.ok_or_else(|| {
        ScenarioError::LegacyParse(format!(
            "C4ValueMapData declares {count} entries without a payload"
        ))
    })?;
    let mut parser = PersistedC4ValueParser::new(payload, resolution);
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        if index != 0 {
            parser.expect(b',')?;
        }
        let name = parser.take_until(b'=')?.trim_matches([' ', '\t']);
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(parser.error(format!("invalid C4ValueMapData identifier `{name}`")));
        }
        parser.expect(b'=')?;
        let value = parser.value()?;
        entries.push((name.to_string(), value));
    }
    Ok(entries)
}

struct PersistedC4ValueParser<'a> {
    input: &'a [u8],
    position: usize,
    resolution: Option<&'a PersistedC4ValueResolution>,
    // CompileFunc resolves the complete containing value stream before the
    // later pointer-denumeration pass can destroy removed map keys.
    compile_holds: Vec<clonk_script::Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PersistedDirectObjectStatus {
    raw: Option<i32>,
    missing: bool,
}

impl PersistedDirectObjectStatus {
    const NONE: Self = Self {
        raw: None,
        missing: false,
    };
}

fn persistent_any_fallback(number: i32) -> clonk_script::Value {
    if number == 0 {
        return clonk_script::Value::Nil;
    }
    let raw = number as u32;
    if raw >= 10_000
        && raw
            .to_le_bytes()
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(raw as usize))
    } else {
        clonk_script::Value::Int(number)
    }
}

impl<'a> PersistedC4ValueParser<'a> {
    fn new(input: &'a str, resolution: Option<&'a PersistedC4ValueResolution>) -> Self {
        Self {
            input: input.as_bytes(),
            position: 0,
            resolution,
            compile_holds: Vec::new(),
        }
    }

    fn error(&self, detail: impl Into<String>) -> ScenarioError {
        ScenarioError::LegacyParse(format!(
            "invalid persistent C4Value at byte {}: {}",
            self.position,
            detail.into()
        ))
    }

    fn skip_whitespace(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), ScenarioError> {
        self.skip_whitespace();
        if self.input.get(self.position) != Some(&expected) {
            return Err(self.error(format!("expected `{}`", char::from(expected))));
        }
        self.position += 1;
        Ok(())
    }

    fn take_until(&mut self, delimiter: u8) -> Result<&'a str, ScenarioError> {
        let start = self.position;
        let Some(relative) = self.input[start..]
            .iter()
            .position(|byte| *byte == delimiter)
        else {
            return Err(self.error(format!("expected `{}`", char::from(delimiter))));
        };
        self.position += relative;
        std::str::from_utf8(&self.input[start..self.position])
            .map_err(|_| self.error("non-UTF-8 compiler token"))
    }

    fn integer(&mut self) -> Result<i32, ScenarioError> {
        self.skip_whitespace();
        let start = self.position;
        if self
            .input
            .get(self.position..self.position.saturating_add(2))
            .is_some_and(|prefix| matches!(prefix, [b'0', b'x' | b'X']))
        {
            self.position += 2;
            let digit_start = self.position;
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_hexdigit)
            {
                self.position += 1;
            }
            if self.position == digit_start {
                return Err(self.error("expected hexadecimal integer"));
            }
            let value = std::str::from_utf8(&self.input[digit_start..self.position])
                .ok()
                .and_then(|value| u64::from_str_radix(value, 16).ok())
                .map(|value| value as i32)
                .ok_or_else(|| self.error("invalid hexadecimal integer"))?;
            self.skip_whitespace();
            return Ok(value);
        }
        if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
            self.position += 1;
        }
        let digit_start = self.position;
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
        }
        if self.position == digit_start {
            return Err(self.error("expected signed integer"));
        }
        let value = std::str::from_utf8(&self.input[start..self.position])
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .map(|value| value as i32)
            .ok_or_else(|| self.error("invalid signed integer"))?;
        self.skip_whitespace();
        Ok(value)
    }

    fn count(&mut self) -> Result<usize, ScenarioError> {
        let value = self.integer()?;
        let count = usize::try_from(value)
            .map_err(|_| self.error(format!("negative element count `{value}`")))?;
        if count > 1_000_000 {
            return Err(self.error(format!("element count `{count}` exceeds C4Value MaxSize")));
        }
        Ok(count)
    }

    fn map_keys_equal(
        left: &clonk_script::Value,
        left_object: PersistedDirectObjectStatus,
        right: &clonk_script::Value,
        right_object: PersistedDirectObjectStatus,
    ) -> bool {
        match (left_object.raw, right_object.raw) {
            (Some(left), Some(right)) => left == right,
            (None, None) => left == right,
            _ => false,
        }
    }

    fn map_assignment_is_nil(
        value: &clonk_script::Value,
        object: PersistedDirectObjectStatus,
    ) -> bool {
        if let Some(raw) = object.raw {
            return raw == 0;
        }
        match value {
            clonk_script::Value::Nil => true,
            clonk_script::Value::C4Id(value) => clonk_script::c4_id_raw(value) == 0,
            clonk_script::Value::Object(value) => *value == 0,
            _ => false,
        }
    }

    fn value(&mut self) -> Result<clonk_script::Value, ScenarioError> {
        self.value_with_direct_object_status()
            .map(|(value, _)| value)
    }

    fn value_with_direct_object_status(
        &mut self,
    ) -> Result<(clonk_script::Value, PersistedDirectObjectStatus), ScenarioError> {
        self.skip_whitespace();
        // StdCompilerINIRead::Character accepts only alphabetic bytes. A
        // legacy untagged integer therefore falls through C4Value's caught
        // NotFoundException as `A` without consuming its first digit.
        let kind = self.input.get(self.position).copied().map_or(b'A', |kind| {
            if kind.is_ascii_alphabetic() {
                self.position += 1;
                kind
            } else {
                b'A'
            }
        });
        match kind {
            b'A' => {
                let value = self.integer()?;
                if (1_000_000_000..=1_001_000_000).contains(&value) {
                    let number = u64::try_from(value - 1_000_000_000).ok();
                    if let Some(number) = number.filter(|number| {
                        self.resolution
                            .is_some_and(|resolution| resolution.object_numbers.contains(number))
                    }) {
                        Ok((
                            clonk_script::Value::Object(number),
                            PersistedDirectObjectStatus::NONE,
                        ))
                    } else {
                        Ok((
                            persistent_any_fallback(value),
                            PersistedDirectObjectStatus::NONE,
                        ))
                    }
                } else {
                    Ok((
                        persistent_any_fallback(value),
                        PersistedDirectObjectStatus::NONE,
                    ))
                }
            }
            b'i' => self.integer().map(|value| {
                (
                    clonk_script::Value::Int(value),
                    PersistedDirectObjectStatus::NONE,
                )
            }),
            b'b' => self.integer().map(|value| {
                (
                    clonk_script::Value::from_c4_bool_raw(value),
                    PersistedDirectObjectStatus::NONE,
                )
            }),
            b'I' => self.integer().map(|value| {
                (
                    clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(
                        value as isize as usize,
                    )),
                    PersistedDirectObjectStatus::NONE,
                )
            }),
            b'S' => {
                let id = self.integer()?;
                let value = self
                    .resolution
                    .and_then(|resolution| clonk_script::resolve_c4_string(&resolution.strings, id))
                    .map(clonk_script::Value::String)
                    .unwrap_or(clonk_script::Value::Nil);
                Ok((value, PersistedDirectObjectStatus::NONE))
            }
            b'o' | b'O' => {
                let raw = self.integer()?;
                let mut number = raw;
                if number >= 1_000_000_000 {
                    number -= 1_000_000_000;
                }
                let number = u64::try_from(number).ok();
                let value = number
                    .filter(|number| {
                        self.resolution
                            .is_some_and(|resolution| resolution.object_numbers.contains(number))
                    })
                    .map(clonk_script::Value::Object);
                let missing = value.is_none();
                Ok((
                    value.unwrap_or(clonk_script::Value::Nil),
                    PersistedDirectObjectStatus {
                        raw: Some(raw),
                        missing,
                    },
                ))
            }
            b'a' => {
                self.expect(b'[')?;
                let count = self.count()?;
                self.expect(b';')?;
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    self.skip_whitespace();
                    if self.input.get(self.position) == Some(&b']') {
                        break;
                    }
                    if index != 0 {
                        self.expect(b',')?;
                    }
                    values.push(self.value()?);
                }
                values.resize(count, clonk_script::Value::Nil);
                self.expect(b']')?;
                Ok((
                    clonk_script::Value::Array(values),
                    PersistedDirectObjectStatus::NONE,
                ))
            }
            b'm' => {
                self.expect(b'[')?;
                let count = self.count()?;
                self.expect(b';')?;
                let mut entries = Vec::<(
                    clonk_script::Value,
                    PersistedDirectObjectStatus,
                    clonk_script::Value,
                    PersistedDirectObjectStatus,
                )>::with_capacity(count);
                let mut compiled_empty_values = Vec::new();
                for index in 0..count {
                    if index != 0 {
                        self.expect(b';')?;
                    }
                    let (key, key_object) = self.value_with_direct_object_status()?;
                    self.expect(b'=')?;
                    let (value, value_object) = self.value_with_direct_object_status()?;
                    if let Some(existing) =
                        entries
                            .iter()
                            .position(|(existing_key, existing_key_object, _, _)| {
                                Self::map_keys_equal(
                                    existing_key,
                                    *existing_key_object,
                                    &key,
                                    key_object,
                                )
                            })
                    {
                        if Self::map_assignment_is_nil(&value, value_object)
                            && !Self::map_assignment_is_nil(
                                &entries[existing].2,
                                entries[existing].3,
                            )
                        {
                            entries.remove(existing);
                            compiled_empty_values.push(clonk_script::Value::Nil);
                        } else {
                            entries[existing].2 = value;
                            entries[existing].3 = value_object;
                        }
                    } else {
                        let _ = compiled_empty_values.pop();
                        entries.push((key, key_object, value, value_object));
                    }
                }
                self.expect(b']')?;
                let mut values = clonk_script::ValueMap::with_capacity(entries.len());
                let mut removed_values = Vec::new();
                for (key, key_object, value, value_object) in entries {
                    if key_object.missing || value_object.missing {
                        if value_object.missing && !key_object.missing {
                            self.compile_holds.push(key);
                        }
                        removed_values.push(value);
                    } else {
                        values.insert_key(key, value);
                    }
                }
                for value in compiled_empty_values {
                    values.recycle_value_slot(value);
                }
                for value in removed_values {
                    values.recycle_value_slot(value);
                }
                Ok((
                    clonk_script::Value::Proplist(values),
                    PersistedDirectObjectStatus::NONE,
                ))
            }
            // GetC4VFromID maps unknown alphabetic tags to C4V_Any.
            _ => self.integer().map(|value| {
                (
                    persistent_any_fallback(value),
                    PersistedDirectObjectStatus::NONE,
                )
            }),
        }
    }
}

/// `StdStrBuf` values are escaped while fixed-size C strings are not. The
/// reader accepts an unquoted legacy fallback just like StdCompiler.
fn decode_escaped_ini_string(value: &str) -> String {
    let trimmed = value.trim_start_matches([' ', '\t']);
    let Some(inner) = trimmed.strip_prefix('"') else {
        return trimmed.to_string();
    };
    let input = clonk_script::c4_string_bytes(inner);
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'"' {
            break;
        }
        if input[index] != b'\\' {
            output.push(input[index]);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&escape) = input.get(index) else {
            break;
        };
        index += 1;
        match escape {
            b'a' => output.push(0x07),
            b'b' => output.push(0x08),
            b'f' => output.push(0x0c),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(0x0b),
            b'\'' => output.push(b'\''),
            b'"' => output.push(b'"'),
            b'\\' => output.push(b'\\'),
            b'?' => output.push(b'?'),
            b'x' => {
                let start = index;
                let mut hexadecimal = 0_u32;
                while let Some(next) = input.get(index).copied() {
                    let Some(digit) = (next as char).to_digit(16) else {
                        break;
                    };
                    hexadecimal = hexadecimal.wrapping_mul(16).wrapping_add(digit);
                    index += 1;
                }
                if index == start {
                    output.push(b'x');
                } else {
                    output.push(hexadecimal as u8);
                }
            }
            digit @ b'0'..=b'7' => {
                let mut octal = u32::from(digit - b'0');
                while let Some(&next @ b'0'..=b'7') = input.get(index) {
                    octal = octal.wrapping_mul(8).wrapping_add(u32::from(next - b'0'));
                    index += 1;
                }
                output.push(octal as u8);
            }
            other => output.push(other),
        }
    }
    if let Some(nul) = output.iter().position(|byte| *byte == 0) {
        output.truncate(nul);
    }
    clonk_script::c4_string_from_bytes(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn synthetic_player_core_defaults_to_keyboard_one_with_mouse() {
        // C4PlayerInfoCore::Default is used for synthetic/script cores and
        // freshly created players. It selects Keyboard1 and enables mouse
        // preference (pristine 9ffa0a5d src/C4InfoCore.cpp:66-85;
        // src/C4StartupPlrSelDlg.cpp:1103-1114).
        let player = PlayerFile::default();

        assert_eq!(player.pref_control, 0);
        assert!(player.pref_mouse);
    }

    #[test]
    fn persistent_value_map_resolves_nested_scenario_values() {
        let strings = clonk_script::new_string_registrations();
        for (id, value) in [(0, "first"), (1, "second"), (2, "key")] {
            clonk_script::register_loaded_c4_string(&strings, id, value);
        }
        let resolution = PersistedC4ValueResolution {
            strings,
            object_numbers: HashSet::from([42]),
        };

        assert_eq!(
            parse_persisted_value_map("1;Complex=a[3;S1,O42,m[1;S2=S0]]", Some(&resolution),)
                .unwrap(),
            vec![(
                "Complex".to_string(),
                clonk_script::Value::Array(vec![
                    clonk_script::Value::String("second".into()),
                    clonk_script::Value::Object(42),
                    clonk_script::Value::Proplist(
                        [(
                            clonk_script::Value::String("key".into()),
                            clonk_script::Value::String("first".into()),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                ]),
            )]
        );
    }

    #[test]
    fn persistent_map_denumeration_retains_missing_key_value_slots() {
        let strings = clonk_script::new_string_registrations();
        clonk_script::register_loaded_c4_string(&strings, 0, "loaded");
        let resolution = PersistedC4ValueResolution {
            strings: strings.clone(),
            object_numbers: HashSet::new(),
        };

        let values = parse_persisted_value_map("1;Map=m[2;o999=S0;i1=i2]", Some(&resolution))
            .expect("persistent map parses");
        let Some((_, clonk_script::Value::Proplist(mut map))) = values.into_iter().next() else {
            panic!("persistent value is a map");
        };
        assert_eq!(
            map.get_key(&clonk_script::Value::Int(1)),
            Some(&clonk_script::Value::Int(2))
        );
        assert!(clonk_script::resolve_c4_string(&strings, 0).is_some());

        map.insert_key(clonk_script::Value::Int(3), clonk_script::Value::Int(4));
        assert!(clonk_script::resolve_c4_string(&strings, 0).is_none());
    }

    #[test]
    fn loads_player_core_and_crew_roster_like_cpp() {
        // C4Player::Load (C4Player.cpp:1089-1107): C4PlayerInfoCore from
        // Player.txt (C4InfoCore.cpp:148-177) and the crew info list from
        // the *.c4i child groups (C4ObjectInfoList.cpp:56-83), each
        // carrying a C4ObjectInfoCore (C4InfoCore.cpp:526-548).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Tester.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Tyler\nComment=Profile comment\nRank=3\nRankName=Veteran\nScore=250\nRounds=11\nRoundsWon=7\nRoundsLost=4\nTotalPlayingTime=1234\nExtraData=3;Flag=b1,Raw=b7,Badge=I1145851719\n\n[Preferences]\nColor=4\nColorDw=12345678\nAlternateColorDw=4289449455\nPosition=2\nControl=3\nMouse=0\nAutoStopControl=1\n\n[LastRound]\nTitle=\"Deep \\\"Mine\\\"\\n\"\nDate=4294967294\nDuration=77\nWon=1\nScore=8\nFinalScore=108\nTotalScore=358\nBonus=100\nLevel=2\n",
        )
        .expect("write core");

        let first = root.join("Wipf.c4i");
        std::fs::create_dir_all(&first).expect("info dir");
        std::fs::write(
            first.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=COWB\nName=Wipf\nDeathMessage=@Gone // but remembered  \nPortraitFile=TRPR::Captain\nRank=2\nRankName=Lieutenant\nNextRankName=Captain\nTypeName=Cowboy\nParticipation=1\nExperience=900\nNextRankExp=5196\nRounds=6\nDeathCount=7\nTotalPlayingTime=17999\nBirthday=123\nAge=7\n\n[Physical]\nWalk=80000\n",
        )
        .expect("write info");

        let second = root.join("Zorro.c4i");
        std::fs::create_dir_all(&second).expect("info dir");
        std::fs::write(
            second.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=TRPR\nName=Zorro\nExperience=50\n",
        )
        .expect("write info");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");
        assert_eq!(player.name, "Tyler");
        // C4PlayerInfoCore::CompileFunc stores both values in [Player]
        // (C4InfoCore.cpp:148-161).
        assert_eq!(player.score, 250);
        assert_eq!(
            (player.rounds, player.rounds_won, player.rounds_lost),
            (11, 7, 4)
        );
        assert_eq!(player.total_playing_time, 1_234);
        assert_eq!(player.info_core.comment, "Profile comment");
        assert_eq!(player.info_core.rank, 3);
        assert_eq!(player.info_core.rank_name, "Veteran");
        assert_eq!(
            player.info_core.extra_data,
            vec![
                ("Flag".to_string(), clonk_script::Value::Bool(true)),
                ("Raw".to_string(), clonk_script::Value::RawBool(7)),
                (
                    "Badge".to_string(),
                    clonk_script::Value::C4Id("GOLD".to_string())
                ),
            ]
        );
        assert_eq!(player.info_core.last_round.title, "Deep \"Mine\"\n");
        assert_eq!(player.info_core.last_round.date, 4_294_967_294);
        assert_eq!(player.info_core.last_round.duration, 77);
        assert_eq!(player.info_core.last_round.won, 1);
        assert_eq!(player.info_core.last_round.final_score, 108);
        assert_eq!(player.info_core.last_round.total_score, 358);
        assert_eq!(player.info_core.last_round.bonus, 100);
        assert_eq!(player.info_core.last_round.level, 2);
        assert_eq!(player.pref_color, 4);
        assert_eq!(player.pref_color_dw, 12345678);
        assert_eq!(player.pref_color2_dw, 0x00ab_cdef);
        assert_eq!(player.normalized_alternate_color(), 0x00ab_cdef);
        assert_eq!(player.pref_position, 2);
        assert_eq!(player.pref_control, 3);
        assert!(!player.pref_mouse);
        assert!(
            player.pref_control_style,
            "AutoStopControl=1 selects Jump'n'Run control (C4InfoCore.cpp:170)"
        );

        assert_eq!(player.crew.len(), 2);
        let wipf = player
            .crew
            .iter()
            .find(|info| info.name == "Wipf")
            .expect("Wipf parsed");
        assert_eq!(wipf.id, "COWB");
        assert_eq!(wipf.death_message, " Gone // but remembered  ");
        assert_eq!(wipf.rank, 2);
        assert_eq!(wipf.rank_name, "Lieutenant");
        assert_eq!(wipf.core.portrait_file, "TRPR::Captain");
        assert_eq!(wipf.core.original_filename, "Wipf.c4i");
        assert_eq!(wipf.core.next_rank_name, "Captain");
        assert_eq!(wipf.core.type_name, "Cowboy");
        assert_eq!(wipf.core.next_rank_exp, 5_196);
        assert_eq!(wipf.experience, 900);
        assert_eq!(wipf.rounds, 6);
        assert_eq!(wipf.physical.walk, 80_000);
        assert_eq!(wipf.physical.energy, 60_000);
        assert_eq!(wipf.physical.can_scale, 1);
        assert_eq!(wipf.physical.can_hangle, 1);
        assert_eq!(wipf.physical.can_dig, 1);
        assert_eq!(wipf.physical.can_construct, 1);
        assert_eq!(wipf.physical.can_chop, 1);
        assert_eq!(wipf.death_count, 7);
        assert_eq!(wipf.total_playing_time, 17_999);
        assert_eq!(wipf.birthday, 123);
        assert_eq!(wipf.age, 7);
        let portrait = wipf
            .portraits
            .fallback
            .as_ref()
            .expect("saved portrait spec evaluates");
        assert_eq!(portrait.source.as_deref(), Some("TRPR"));
        assert_eq!(portrait.name, "Captain");
        assert_eq!(wipf.portraits.current.as_ref(), Some(portrait));
        assert_eq!(
            wipf.portraits.permanent,
            CrewPermanentPortrait::Absent,
            "a loaded PortraitFile is a fallback, not pNewPortrait"
        );
        assert_eq!(wipf.participation, 1);
        assert!(!wipf.in_action);
        assert!(!wipf.was_in_action);
        assert!(!wipf.has_died);
        let zorro = player
            .crew
            .iter()
            .find(|info| info.name == "Zorro")
            .expect("Zorro parsed");
        assert_eq!(zorro.id, "TRPR");
        assert_eq!(zorro.rank, 0, "Rank defaults to 0");
        assert_eq!(zorro.rank_name, "Clonk", "RankName defaults to Clonk");
        assert_eq!(zorro.core.original_filename, "Zorro.c4i");
        assert_eq!(
            CrewInfoCoreFields {
                original_filename: String::new(),
                ..zorro.core.clone()
            },
            CrewInfoCoreFields::default()
        );
        assert_eq!(zorro.death_count, 0, "DeathCount defaults to 0");
        assert_eq!((zorro.birthday, zorro.age), (0, 0));
        assert_eq!(zorro.participation, 1, "Participation defaults to 1");
    }

    #[test]
    fn crew_info_loads_one_group_with_custom_portrait_state() {
        let dir = tempdir().expect("tempdir");
        let crew_path = dir.path().join("Veteran.c4i");
        std::fs::create_dir(&crew_path).expect("crew dir");
        std::fs::write(
            crew_path.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=CLNK\nName=Veteran\nPortraitFile=custom\nExperience=123\nParticipation=0\n",
        )
        .expect("write crew core");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(crew_path.join("Portrait.png"))
            .expect("write crew portrait");
        let group = Group::open(&crew_path).expect("open crew group");

        let crew = CrewInfo::load(&group).expect("load crew info");

        assert_eq!(crew.id, "CLNK");
        assert_eq!(crew.name, "Veteran");
        assert_eq!(crew.experience, 123);
        assert_eq!(crew.participation, 0);
        assert_eq!(crew.core.portrait_file, "custom");
        let portrait = crew
            .portraits
            .current
            .as_ref()
            .expect("decoded custom portrait is current");
        assert_eq!(portrait.source, None);
        assert_eq!(portrait.name, "custom");
    }

    #[test]
    fn object_info_ini_names_are_exact_case() {
        let wrong_section = CrewInfo::from_object_info_source(
            "[objectinfo]\nid=WRONG\nName=Wrong\nNextRankExp=77\n[Physical]\nWalk=90000\n",
            false,
            true,
            None,
        );
        assert!(wrong_section.id.is_empty());
        assert_eq!(wrong_section.name, "Clonk");
        assert_eq!(wrong_section.core.next_rank_exp, 0);
        assert_eq!(wrong_section.physical.walk, 0);

        let wrong_values = CrewInfo::from_object_info_source(
            "[ObjectInfo]\nid=CASE\nnextrankexp=77\n[Physical]\nwalk=90000\n",
            false,
            true,
            None,
        );
        assert_eq!(wrong_values.id, "CASE");
        assert_eq!(wrong_values.core.next_rank_exp, 0);
        assert_eq!(wrong_values.physical.walk, 0);

        let wrong_physical_section = CrewInfo::from_object_info_source(
            "[ObjectInfo]\nid=CASE\n[physical]\nWalk=90000\n",
            false,
            true,
            None,
        );
        assert_eq!(wrong_physical_section.physical.walk, 0);
    }

    #[test]
    fn object_info_ini_duplicate_nodes_are_first_exact_match() {
        let info = CrewInfo::from_object_info_source(
            "[ObjectInfo]\n\
             id=FIRST\n\
             Name=First\n\
             nextrankexp=1\n\
             NextRankExp=40\n\
             NextRankExp=50\n\
             Experience=7\n\
             Experience=8\n\
             [Physical]\n\
             walk=3\n\
             Walk=200\n\
             Walk=300\n\
             [ObjectInfo]\n\
             id=SECOND\n\
             Name=Ignored\n\
             NextRankExp=999\n\
             [Physical]\n\
             Jump=444\n",
            false,
            true,
            None,
        );

        assert_eq!(info.id, "FIRST");
        assert_eq!(info.name, "First");
        assert_eq!(info.core.next_rank_exp, 40);
        assert_eq!(info.experience, 7);
        assert_eq!(info.physical.walk, 200);
        assert_eq!(info.physical.jump, 0);
    }

    #[test]
    fn object_info_ini_physical_follows_cpp_tree_position() {
        for (label, source, expected_walk) in [
            (
                "adjacent root sibling",
                "[ObjectInfo]\nName=Adjacent\n[Physical]\nWalk=101\n",
                101,
            ),
            (
                "intervening root sibling",
                "[ObjectInfo]\nName=Blocked\n[Other]\nValue=1\n[Physical]\nWalk=202\n",
                0,
            ),
            (
                "nested physical",
                "[ObjectInfo]\nName=Nested physical\n [Physical]\n Walk=303\n",
                0,
            ),
            (
                "nested intervening section",
                "[ObjectInfo]\nName=Nested other\n [Other]\n  Value=1\n[Physical]\nWalk=404\n",
                404,
            ),
            (
                "repeated object info",
                "[ObjectInfo]\nName=First\n[ObjectInfo]\nName=Second\n[Physical]\nWalk=505\n",
                0,
            ),
            (
                "earlier physical wins after adjacency gate",
                "[Physical]\nWalk=111\n[ObjectInfo]\nName=Middle\n[Physical]\nWalk=222\n",
                111,
            ),
        ] {
            let info = CrewInfo::from_object_info_source(source, false, true, None);
            assert_eq!(info.physical.walk, expected_walk, "{label}");
        }
    }

    #[test]
    fn loads_custom_portrait_fallback_only_when_the_embedded_image_decodes() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Portraits.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(root.join("Player.txt"), "[Player]\nName=Portraits\n")
            .expect("write player core");

        for (group_name, crew_name, portrait_file, image_name) in [
            ("Valid.c4i", "Valid custom", "custom", Some("Portrait.png")),
            ("Broken.c4i", "Broken custom", "custom", None),
            ("Embedded.c4i", "Embedded custom", "", Some("Portrait.bmp")),
        ] {
            let group = root.join(group_name);
            std::fs::create_dir_all(&group).expect("crew group");
            std::fs::write(
                group.join("ObjectInfo.txt"),
                format!("[ObjectInfo]\nid=CLNK\nName={crew_name}\nPortraitFile={portrait_file}\n"),
            )
            .expect("write crew core");
            if let Some(image_name) = image_name {
                image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
                    .save(group.join(image_name))
                    .expect("write custom portrait image");
            }
        }
        let corrupt = root.join("CorruptPng.c4i");
        std::fs::create_dir_all(&corrupt).expect("corrupt crew group");
        std::fs::write(
            corrupt.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=CLNK\nName=Corrupt PNG\nPortraitFile=custom\n",
        )
        .expect("write corrupt crew core");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(corrupt.join("Portrait.bmp"))
            .expect("write valid legacy fallback");
        std::fs::write(corrupt.join("Portrait.png"), b"not a png")
            .expect("write corrupt preferred PNG");

        let mismatched = root.join("MismatchedOverlay.c4i");
        std::fs::create_dir_all(&mismatched).expect("overlay crew group");
        std::fs::write(
            mismatched.join("ObjectInfo.txt"),
            "[ObjectInfo]\nid=CLNK\nName=Mismatched overlay\nPortraitFile=custom\n",
        )
        .expect("write overlay crew core");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(mismatched.join("Portrait.png"))
            .expect("write overlay base");
        image::RgbaImage::from_pixel(2, 1, image::Rgba([4, 5, 6, 255]))
            .save(mismatched.join("PortraitOverlay.png"))
            .expect("write mismatched overlay");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");
        let valid = player
            .crew
            .iter()
            .find(|info| info.name == "Valid custom")
            .expect("explicit custom crew parsed");
        let valid_fallback = valid
            .portraits
            .fallback
            .as_ref()
            .expect("pCustomPortrait fallback retained");
        assert_eq!(valid_fallback.source, None);
        assert_eq!(valid_fallback.name, "custom");
        assert_eq!(valid.portraits.current.as_ref(), Some(valid_fallback));
        assert_eq!(valid.core.portrait_file, "custom");

        let embedded = player
            .crew
            .iter()
            .find(|info| info.name == "Embedded custom")
            .expect("legacy embedded portrait parsed");
        let current = embedded
            .portraits
            .current
            .as_ref()
            .expect("embedded image is current");
        assert_eq!(current.source, None);
        let fallback = embedded
            .portraits
            .fallback
            .as_ref()
            .expect("synthesized PortraitFile fallback exists");
        assert_eq!(fallback.source.as_deref(), Some("CLNK"));
        assert_eq!(fallback.name, "custom");
        assert_eq!(embedded.core.portrait_file, "custom");

        for name in ["Broken custom", "Corrupt PNG", "Mismatched overlay"] {
            let info = player
                .crew
                .iter()
                .find(|info| info.name == name)
                .expect("invalid custom crew parsed");
            assert_eq!(info.portraits, CrewPortraitState::default());
            assert!(info.core.portrait_file.is_empty());
        }

        let group = Group::open(&root).expect("reopen player group");
        let remote =
            PlayerFile::load_with_portraits(&group, false).expect("remote player file loads");
        let explicit = remote
            .crew
            .iter()
            .find(|info| info.name == "Valid custom")
            .expect("explicit custom portrait remains loadable remotely");
        assert_eq!(explicit.core.portrait_file, "custom");
        assert!(!explicit.core.portrait_png.is_empty());
        assert!(explicit.portraits.current.is_some());

        let unnamed = remote
            .crew
            .iter()
            .find(|info| info.name == "Embedded custom")
            .expect("unnamed embedded portrait crew remains present");
        assert!(unnamed.core.portrait_file.is_empty());
        assert!(unnamed.core.portrait_png.is_empty());
        assert!(unnamed.core.portrait_bmp.is_empty());
        assert_eq!(unnamed.portraits, CrewPortraitState::default());
    }

    #[test]
    fn native_player_and_crew_names_remain_raw_c4_bytes() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Native.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            [b"[Player]\nName=Andr".as_slice(), &[0xe9], b"\n"].concat(),
        )
        .expect("write native player core");
        let crew = root.join("Rene.c4i");
        std::fs::create_dir_all(&crew).expect("crew dir");
        std::fs::write(
            crew.join("ObjectInfo.txt"),
            [
                b"[ObjectInfo]\nid=CLNK\nName=Ren".as_slice(),
                &[0xe9],
                b"\n",
            ]
            .concat(),
        )
        .expect("write native crew core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");
        assert_eq!(clonk_script::c4_string_bytes(&player.name), b"Andr\xe9");
        assert_eq!(player.crew.len(), 1);
        assert_eq!(
            clonk_script::c4_string_bytes(&player.crew[0].name),
            b"Ren\xe9"
        );
    }

    #[test]
    fn missing_core_keys_fall_back_to_cpp_defaults() {
        // C4PlayerInfoCore defaults (C4InfoCore.cpp:152,166-173):
        // Name "Neuling", Color 0, ColorDw 0xff, Position 0.
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Empty.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(root.join("Player.txt"), "[Player]\n").expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");
        assert_eq!(player.name, "Neuling");
        assert_eq!(player.score, 0);
        assert_eq!(player.total_playing_time, 0);
        assert_eq!(player.pref_color, 0);
        assert_eq!(player.pref_color_dw, 0xff);
        assert_eq!(player.pref_color2_dw, 0);
        assert_eq!(player.pref_position, 0);
        assert_eq!(
            player.pref_control, 1,
            "omitted loaded-file Control defaults to Keyboard2"
        );
        assert!(player.pref_mouse, "omitted Mouse defaults to enabled");
        assert!(
            !player.pref_control_style,
            "AutoStopControl defaults to 0 = classic (C4InfoCore.cpp:84)"
        );
        assert!(
            !player.pref_auto_context_menu,
            "AutoContextMenu inherits the default classic style (C4InfoCore.cpp:103-115)"
        );
        assert!(player.crew.is_empty());
    }

    #[test]
    fn legacy_crew_json_defaults_death_count_to_zero() {
        let info: CrewInfo = serde_json::from_str(
            r#"{"id":"CLNK","name":"Clonk","rank":0,"experience":0,"total_playing_time":0,"participation":1,"in_action":false,"in_action_time":0,"has_died":false}"#,
        )
        .expect("pre-DeathCount crew JSON remains readable");

        assert_eq!(info.rank_name, "Clonk");
        assert_eq!(info.death_count, 0);
        assert!(info.death_message.is_empty());
    }

    #[test]
    fn player_file_core_names_are_exact_case() {
        // StdCompilerINIRead compares every section and value name exactly.
        // Wrong-case PlayerInfoCore names are unexpected entries, so
        // CompileFunc applies its loaded-file defaults (pristine 9ffa0a5d
        // src/StdCompiler.cpp:498-525; src/C4InfoCore.cpp:148-176,565-582).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Case.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[player]\n\
             Name=Wrong section\n\
             Comment=Wrong section\n\
             Rank=1\n\
             RankName=Wrong section\n\
             Score=2\n\
             Rounds=3\n\
             RoundsWon=4\n\
             RoundsLost=5\n\
             TotalPlayingTime=6\n\
             ExtraData=1;Wrong=b1\n\
             [preferences]\n\
             Color=1\n\
             ColorDw=1193046\n\
             AlternateColorDw=11259375\n\
             Control=3\n\
             AutoStopControl=1\n\
             AutoContextMenu=1\n\
             Position=2\n\
             Mouse=0\n\
             [lastround]\n\
             Title=Wrong section\n\
             Date=1\n\
             Duration=2\n\
             Won=1\n\
             Score=3\n\
             FinalScore=4\n\
             TotalScore=5\n\
             Bonus=6\n\
             Level=7\n\
             [Player]\n\
             name=Wrong key\n\
             comment=Wrong key\n\
             rank=8\n\
             rankname=Wrong key\n\
             score=9\n\
             rounds=10\n\
             roundswon=11\n\
             roundslost=12\n\
             totalplayingtime=13\n\
             extradata=1;Wrong=b1\n\
             [Preferences]\n\
             color=2\n\
             colordw=6636321\n\
             alternatecolordw=1267611\n\
             control=4\n\
             autostopcontrol=1\n\
             autocontextmenu=1\n\
             position=3\n\
             mouse=0\n\
             [LastRound]\n\
             title=Wrong key\n\
             date=8\n\
             duration=9\n\
             won=1\n\
             score=10\n\
             finalscore=11\n\
             totalscore=12\n\
             bonus=13\n\
             level=14\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert_eq!(
            player.info_core,
            PlayerInfoCoreState {
                pref_name: "Neuling".to_string(),
                rank_name: "Rank".to_string(),
                pref_color_dw: 0xff,
                pref_control: 1,
                last_round: PlayerLastRoundState {
                    title: String::new(),
                    date: 0,
                    duration: 0,
                    won: 0,
                    score: 0,
                    final_score: 0,
                    total_score: 0,
                    bonus: 0,
                    level: 0,
                },
                ..Default::default()
            }
        );
    }

    #[test]
    fn control_and_mouse_accept_cpp_hex_numbers_with_trailing_text() {
        // StdCompilerINIRead selects base 16 for a 0x prefix and strtol stops
        // at the first non-digit (pristine 9ffa0a5d
        // src/StdCompiler.h:705-722; src/StdCompiler.cpp:646-649).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Hex.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Preferences]\nControl=0x4gamepad\nMouse=0x0mouse\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert_eq!(player.pref_control, 4);
        assert!(!player.pref_mouse);
    }

    #[test]
    fn player_core_keeps_raw_strings_and_noncanonical_integer_preferences() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Exact.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Value // not a comment  \nComment=Keep trailing  \nExtraData=1;Zero=I0\n\n[Preferences]\nAutoStopControl=2\nAutoContextMenu=-2\nMouse=7\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert_eq!(player.name, "Value // not a comment  ");
        assert_eq!(player.info_core.comment, "Keep trailing  ");
        assert_eq!(player.info_core.pref_control_style_value, 2);
        assert_eq!(player.info_core.pref_auto_context_menu_value, -2);
        assert_eq!(player.info_core.pref_mouse_value, 7);
        assert!(player.pref_control_style);
        assert!(player.pref_auto_context_menu);
        assert!(player.pref_mouse);
        assert_eq!(
            player.info_core.extra_data,
            vec![(
                "Zero".to_string(),
                clonk_script::Value::C4Id(clonk_script::c4_id_from_raw(0)),
            )]
        );
    }

    #[test]
    fn object_rank_names_use_stdstrbuf_escaping() {
        let info = CrewInfo::from_object_info_source(
            "[ObjectInfo]\nRankName=\"Lieutenant \\\"A\\\"\"\nNextRankName=\"Captain\\nTwo\" trailing\n",
            false,
            true,
            None,
        );

        assert_eq!(info.rank_name, "Lieutenant \"A\"");
        assert_eq!(info.core.next_rank_name, "Captain\nTwo");
    }

    #[test]
    fn loads_explicit_auto_context_menu_preference_like_cpp() {
        // C4PlayerInfoCore::CompileFunc reads [Preferences] AutoContextMenu
        // as PrefAutoContextMenu (src/C4InfoCore.cpp:164-172).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("AutoMenu.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nAutoContextMenu=1\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert!(player.pref_auto_context_menu);
    }

    #[test]
    fn omitted_auto_context_menu_defaults_to_control_style_like_cpp() {
        // C4PlayerInfoCore::CompileFunc defaults AutoContextMenu to -1;
        // C4PlayerInfoCore::Load then replaces -1 with PrefControlStyle
        // (src/C4InfoCore.cpp:103-115,164-172).
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("DefaultAutoMenu.c4p");
        std::fs::create_dir_all(&root).expect("player dir");
        std::fs::write(
            root.join("Player.txt"),
            "[Player]\nName=Tyler\n\n[Preferences]\nAutoStopControl=1\n",
        )
        .expect("write core");

        let player = PlayerFile::load_from_path(&root).expect("player file loads");

        assert!(player.pref_auto_context_menu);
    }

    #[test]
    fn loads_cpp_packed_player_data_from_memory() {
        // Remote C4ControlJoinPlayer saves its PlrData blob as a temporary
        // .c4p and C4Player::Load opens that packed group
        // (src/C4Control.cpp:731-744; src/C4Player.cpp:267-284,1089-1106).
        let bytes = include_bytes!("../tests/fixtures/embedded_player.c4p").to_vec();

        let player =
            PlayerFile::load_from_bytes(std::path::PathBuf::from("embedded_player.c4p"), bytes)
                .expect("C++-packed PlrData loads");

        assert_eq!(player.name, "Embedded Tyler");
        assert_eq!((player.score, player.total_playing_time), (42, 99));
        assert_eq!((player.pref_color, player.pref_position), (3, 2));
        assert_eq!(player.pref_color_dw, 1_122_867);
        assert!(player.pref_control_style);
        assert!(!player.pref_auto_context_menu);
    }
}
