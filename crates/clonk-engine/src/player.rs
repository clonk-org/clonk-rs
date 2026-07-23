use crate::player_file::{PlayerInfoCoreState, PlayerLastRoundState};
use crate::{DefinitionId, ObjectId, RgbColor, Vector2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub(crate) const MAX_HOME_BASE_MATERIAL: u32 = 25;
const MAX_SET_WEALTH: i32 = 100_000;
const MAX_WEALTH_ADJUSTMENT: i32 = 10_000;
const MAX_SCORE: i32 = 100_000;
const MIN_SCORE: i32 = -100_000;
const PLAYER_VIEW_DELAY: i32 = 100;
const VIEWPORT_SCROLL_BORDER: i32 = 40;

pub const PLAYER_VIEW_MODE_CURSOR: i32 = 0;
pub const PLAYER_VIEW_MODE_TARGET: i32 = 1;
pub const PLAYER_VIEW_MODE_SCROLLING: i32 = 2;

fn resolved_view_object(
    view_mode: i32,
    view_target: Option<ObjectId>,
    view_cursor: Option<ObjectId>,
    cursor: Option<ObjectId>,
) -> Option<ObjectId> {
    match view_mode {
        PLAYER_VIEW_MODE_CURSOR => view_cursor.or(cursor),
        PLAYER_VIEW_MODE_TARGET => view_target,
        PLAYER_VIEW_MODE_SCROLLING => None,
        _ => None,
    }
}

fn bound_view_center(value: i32, lower: i32, upper: i32) -> i32 {
    // C++ BoundBy evaluates the lower comparison first and deliberately does
    // not normalize inverted bounds (C4Math.h:23).
    if value < lower {
        lower
    } else if value > upper {
        upper
    } else {
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum PlayerStatus {
    #[default]
    Inactive,
    Active,
    Eliminated,
    TeamSelection,
    TeamSelectionPending,
    Surrendered,
}

/// The network client that owns a runtime player (`C4Player::AtClient`).
///
/// C++ uses the signed sentinel `C4ClientIDUnknown == -1`; keeping it in a
/// dedicated transparent type prevents Rust's numeric `Default` (zero, the
/// host client) from accidentally granting ownership to legacy save data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerAtClient(i32);

impl PlayerAtClient {
    pub const UNKNOWN: Self = Self(-1);
    pub const HOST: Self = Self(0);

    pub const fn new(client_id: i32) -> Self {
        Self(client_id)
    }

    pub const fn get(self) -> i32 {
        self.0
    }

    fn is_unknown(value: &Self) -> bool {
        *value == Self::UNKNOWN
    }
}

impl Default for PlayerAtClient {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

fn bounded_at_client_name(name: String) -> String {
    const C4_MAX_TITLE: usize = 512;
    let mut bytes = clonk_script::c4_string_bytes(&name);
    bytes.truncate(C4_MAX_TITLE);
    clonk_script::c4_string_from_bytes(&bytes)
}

fn bounded_message_buf(message: String) -> String {
    const C4_MESSAGE_BUFFER_LENGTH: usize = 256;
    let mut bytes = clonk_script::c4_string_bytes(&message);
    bytes.truncate(C4_MESSAGE_BUFFER_LENGTH);
    clonk_script::c4_string_from_bytes(&bytes)
}

fn set_ordered_id_count<K: PartialEq>(
    entries: &mut Vec<(K, i32)>,
    id: K,
    count: i32,
    add_new: bool,
) -> bool {
    if let Some((_, stored)) = entries.iter_mut().find(|(stored, _)| stored == &id) {
        *stored = count;
        true
    } else if add_new {
        entries.push((id, count));
        true
    } else {
        false
    }
}

fn delete_ordered_id<K: PartialEq>(entries: &mut Vec<(K, i32)>, id: &K) -> bool {
    let Some(index) = entries.iter().position(|(stored, _)| stored == id) else {
        return false;
    };
    entries.remove(index);
    true
}

fn ordered_id_count<K: PartialEq>(entries: &[(K, i32)], id: &K, zero_default: i32) -> i32 {
    entries
        .iter()
        .find(|(stored, _)| stored == id)
        .map(|(_, count)| if *count == 0 { zero_default } else { *count })
        .unwrap_or(0)
}

fn ordered_entries_from_unsigned_map(map: &HashMap<DefinitionId, u32>) -> Vec<(DefinitionId, i32)> {
    let mut entries = map
        .iter()
        .map(|(id, count)| (id.clone(), i32::try_from(*count).unwrap_or(i32::MAX)))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn unsigned_first_count_projection(entries: &[(DefinitionId, i32)]) -> HashMap<DefinitionId, u32> {
    let mut projection = HashMap::new();
    for (id, count) in entries {
        projection
            .entry(id.clone())
            .or_insert_with(|| u32::try_from(*count).unwrap_or(0));
    }
    projection
}

fn ordered_ids_projection(entries: &[(DefinitionId, i32)]) -> Vec<DefinitionId> {
    entries.iter().map(|(id, _)| id.clone()).collect()
}

fn sorted_unique_ids_projection(entries: &[(DefinitionId, i32)]) -> Vec<DefinitionId> {
    let mut projection = ordered_ids_projection(entries);
    projection.sort();
    projection.dedup();
    projection
}

fn hostility_projection(entries: &[(i32, i32)]) -> Vec<i32> {
    let mut seen = HashSet::new();
    let mut projection = Vec::new();
    for (raw_id, count) in entries {
        if seen.insert(*raw_id) && *count != 0 {
            projection.push(raw_id.wrapping_sub(1));
        }
    }
    projection.sort_unstable();
    projection
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerViewport {
    #[serde(default)]
    pub focus: Option<ObjectId>,
    pub center: Vector2,
    #[serde(default = "default_zoom")]
    pub zoom: f32,
}

impl PlayerViewport {
    pub fn new(center: Vector2) -> Self {
        Self {
            focus: None,
            center,
            zoom: default_zoom(),
        }
    }

    pub fn with_focus(mut self, focus: Option<ObjectId>) -> Self {
        self.focus = focus;
        self
    }

    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom.max(0.0);
        self
    }
}

fn default_zoom() -> f32 {
    1.0
}

/// One `C4MessageBoardQuery` retained by a runtime player. The answered bit
/// is deliberately not serialized: C++ re-asks an in-flight query after a
/// savegame reload (C4MessageInput.cpp:790-801).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MessageBoardQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ObjectId>,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        with = "clonk_script::c4_string_serde"
    )]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub uppercase: bool,
    #[serde(skip)]
    pub answered: bool,
}

impl MessageBoardQuery {
    pub fn new(target: Option<ObjectId>, prompt: String, uppercase: bool) -> Self {
        Self {
            target,
            prompt,
            uppercase,
            answered: false,
        }
    }
}

/// Process-local `C4ChatInputDialog` projection for a script query. Query
/// registration is synchronized player state; the one visible edit line is
/// runtime presentation state and is not part of a savegame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMessageBoardInput {
    pub player: i32,
    pub target: Option<ObjectId>,
    pub prompt: String,
    pub uppercase: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlayerState {
    pub id: i32,
    /// Unique `C4Player::ID` linking this runtime player to `C4PlayerInfo`;
    /// distinct from the in-round `C4Player::Number` stored in `id`
    /// (C4Player.h:67-70).
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub player_info_id: i32,
    /// Saved `C4Player::AtClient`; the JoinPlayer packet overwrites this when
    /// recreating a player for the current network client association.
    #[serde(default, skip_serializing_if = "PlayerAtClient::is_unknown")]
    pub at_client: PlayerAtClient,
    /// Join-time snapshot of `C4Player::AtClientName`. `None` is the C++
    /// runtime default, `"Local"`; `Some("")` remains a real empty network
    /// client name rather than being conflated with that default.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub at_client_name: Option<String>,
    /// Stable C4PlayerInfo type used to recompute LocalControl on restore.
    #[serde(default, skip_serializing_if = "is_false")]
    pub script_player: bool,
    /// Saved `C4PlayerInfo::PIF_NoEliminationCheck` projection. The C++
    /// runtime field itself is Local-NoSave, but exact saves retain the
    /// authoritative player-info bit and reapply it during recreation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub no_elimination_check: bool,
    #[serde(default, with = "clonk_script::c4_string_serde")]
    pub name: String,
    #[serde(default)]
    pub status: PlayerStatus,
    /// Exact persisted `C4Player::Status` compiler word. The semantic status
    /// above folds elimination/surrender into Rust variants, while C++ stores
    /// this independent signed integer verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_value: Option<i32>,
    #[serde(default)]
    pub team: Option<i32>,
    #[serde(default)]
    pub surrendered: bool,
    /// Exact persisted `C4Player::Surrendered` integer. Gameplay uses the
    /// boolean projection above, but the runtime compiler does not normalize
    /// nonzero values when writing Game.txt again.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub surrendered_value: i32,
    /// Exact persisted `C4Player::Eliminated` integer. Gameplay uses the
    /// folded `status` projection, but the compiler preserves any nonzero
    /// signed value independently of `Status` and `Surrendered`.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub eliminated_value: i32,
    /// Result flag projected from the linked `C4PlayerInfo::PIF_Won` state
    /// (`C4PlayerInfo.h:63,219-237`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub won: bool,
    /// `C4Player::Evaluated`, kept separate from game-over so evaluation is
    /// idempotent (`C4Player.cpp:930-970`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub evaluated: bool,
    #[serde(default)]
    pub wealth: i32,
    #[serde(default)]
    pub points: i32,
    /// Persistent settlement score from `C4PlayerInfoCore`, not the in-round
    /// `Points` counter (`C4InfoCore.h:202-204`).
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub score: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub rounds: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub rounds_won: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub rounds_lost: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub total_playing_time: i32,
    /// Exact external `C4PlayerInfoCore` retained alongside the in-round
    /// `C4Player`. It owns profile-only fields such as comment, rank and
    /// LastRound as well as the unassigned preferences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_info_core: Option<PlayerInfoCoreState>,
    #[serde(default)]
    pub value: i32,
    #[serde(default)]
    pub initial_value: i32,
    #[serde(default)]
    pub value_gain: i32,
    #[serde(default)]
    pub objects_owned: u32,
    #[serde(default)]
    pub initial_value_set: bool,
    #[serde(default)]
    pub knowledge: Vec<DefinitionId>,
    /// Exact ordered/signed C4IDList backing for `Knowledge`. The legacy
    /// projection above remains for old Rust saves; this list is authoritative
    /// when present and preserves duplicate and zero-count entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_entries: Vec<(DefinitionId, i32)>,
    /// Legacy ID-only projection of the ordered `C4Player::Magic` list.
    #[serde(default)]
    pub magic: Vec<DefinitionId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub magic_entries: Vec<(DefinitionId, i32)>,
    #[serde(default)]
    pub inventory: HashMap<DefinitionId, u32>,
    #[serde(default)]
    pub cursor: Option<ObjectId>,
    /// C4Player::ViewMode (C4PVM_Cursor/Target/Scrolling), saved by C++.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub view_mode: i32,
    /// C4Player::ViewCursor: the independently saved cursor-mode camera
    /// pointer. This is distinct from a temporary C4PVM_Target ViewTarget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_cursor: Option<ObjectId>,
    /// Saved `C4Player::Captain`. This is assigned once during FinalInit
    /// when the KillTheCaptain rule is present; it is not derived from the
    /// current cursor or highest-ranked crew on each query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captain: Option<ObjectId>,
    /// C4Player::ViewTarget is explicitly NO-SAVE. It remains in live
    /// snapshots so presentation can resolve the target-mode center, but never
    /// enters serialized engine state.
    #[serde(skip)]
    pub view_target: Option<ObjectId>,
    /// Independently saved `C4Player::ViewX/ViewY`. `None` is retained only
    /// while reading older Rust snapshots that inferred these values from the
    /// first presentation viewport; every live C4Player owns the scalars even
    /// when no local C4Viewport exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_center: Option<Vector2>,
    /// Process-local presentation viewports. Their camera centers are not the
    /// persisted `C4Player::ViewX/ViewY` fields above.
    #[serde(default)]
    pub viewports: Vec<PlayerViewport>,
    /// Runtime-only `C4Viewport::ViewOffsX/Y` presentation displacement.
    /// Scripts such as FXQ1 use this independently of the player's saved
    /// `ViewX/ViewY` center (C4Viewport.cpp:1183-1214).
    #[serde(default, skip_serializing_if = "is_zero_vector")]
    pub view_offset: Vector2,
    /// Frames the wealth/value HUD change indicators remain visible.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub view_wealth: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub view_value: i32,
    #[serde(default)]
    pub crew: Vec<ObjectId>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub crew_created: i32,
    #[serde(default)]
    pub home_base_material: HashMap<DefinitionId, u32>,
    /// Exact ordered/signed `C4Player::HomeBaseMaterial` C4IDList.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub home_base_material_entries: Vec<(DefinitionId, i32)>,
    #[serde(default)]
    pub home_base_production: HashMap<DefinitionId, u32>,
    /// Exact ordered/signed `C4Player::HomeBaseProduction` C4IDList.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub home_base_production_entries: Vec<(DefinitionId, i32)>,
    #[serde(default)]
    pub production_delay: u32,
    #[serde(default)]
    pub production_unit: u32,
    #[serde(default)]
    pub color: Option<RgbColor>,
    /// Exact persisted `C4Player::ColorDw`. The RGB projection above is used
    /// by render code, but C++ compiles the complete unsigned 32-bit word and
    /// does not discard its high byte. `None` marks older Rust snapshots that
    /// only retained the RGB projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_dw_raw: Option<u32>,
    /// Old-gfx palette index (`C4Player::Color`), distinct from RGB
    /// `ColorDw`. `None` represents the live C++ sentinel -1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_index: Option<i32>,
    /// Distributed startup-position slot (`C4Player::Position`). `None`
    /// represents the live C++ sentinel -1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_index: Option<i32>,
    /// Effective runtime control set. Missing runtime-save data compiles to
    /// C++'s serialized default `0`; a fresh live Player starts at `-1`.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub control_set: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub mouse_control: i32,
    /// Player-core preferences used when InitControl recomputes the effective
    /// process-local assignment after a savegame recreation.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub pref_control: i32,
    /// `None` is the C4PlayerInfoCore default (`PrefMouse=1`) for legacy Rust
    /// saves that predate this retained player-core value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pref_mouse: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pref_control_style: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pref_auto_context_menu: bool,
    /// Saved `C4Player::fFogOfWar` setting (C4Player.cpp:1580).
    #[serde(default)]
    pub fog_of_war: bool,
    /// Whether fog of war was explicitly forced instead of selected by mouse
    /// control (C4Player::bForceFogOfWar, C4Player.cpp:1581).
    #[serde(default)]
    pub force_fog_of_war: bool,
    /// Startup overlay flag. Fresh players start true; absent save data uses
    /// the serialized C++ default false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_startup: bool,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub select_count: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub message_status: i32,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        with = "clonk_script::c4_string_serde"
    )]
    pub message_buf: String,
    /// `C4Player::pMsgBoardQuery` in linked-list order. Save preparation
    /// retains only the head because the C++ query compiler omits `pNext`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_board_queries: Vec<MessageBoardQuery>,
    /// C4Player::ShowControlPos: tutorial-selected placement for the command
    /// hint strip (FnSetPlrShowControlPos, C4Script.cpp:2561-2566).
    #[serde(default)]
    pub show_control_position: i32,
    /// C4Player::ShowControl: three ten-bit layers selecting command hints,
    /// their key labels, and blinking labels (C4Viewport.cpp:1424-1439).
    #[serde(default)]
    pub show_control: i32,
    /// Sorted/unique nonzero-opponent projection of `C4Player::Hostility`,
    /// retained for older Rust saves.
    #[serde(default)]
    pub hostility: Vec<i32>,
    /// Exact C4Player::Hostility C4IDList. Keys are raw numeric C4IDs
    /// (`opponent Number + 1`), not player numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hostility_entries: Vec<(i32, i32)>,
    /// Direct-com input state (C4Player.h:118-121, serialized by
    /// C4Player::CompileFunc "LastCom"/"LastComDel"/"LastComDownDouble"/
    /// "PressedComs"/"AutoStopControl"/"CursorFlash", C4Player.cpp:1596-1604).
    #[serde(default)]
    pub control: PlayerControlState,
    /// C4Player::ExtraData (C4ValueMapData) — the script-managed named
    /// slots of Fn[Set/Get]PlrExtraData (C4Script.cpp:4692-4747). The setter
    /// validates ordinary script writes separately; loaded/runtime-save state
    /// retains every serializable C4Value variant. Insertion order is the
    /// C4ValueMapNames order.
    #[serde(default)]
    pub extra_data: Vec<(String, clonk_script::Value)>,
}

impl PlayerState {
    pub(crate) fn exact_status_value(&self) -> i32 {
        self.status_value
            .unwrap_or_else(|| player_status_compiler_value(self.status))
    }

    pub(crate) fn exact_eliminated_value(&self) -> i32 {
        retained_runtime_flag_value(
            self.eliminated_value,
            matches!(
                self.status,
                PlayerStatus::Eliminated | PlayerStatus::Surrendered
            ),
        )
    }

    pub(crate) fn exact_color_dw(&self) -> u32 {
        exact_player_color_dw(self.color_dw_raw, self.color)
    }

    pub(crate) fn set_color_dw(&mut self, color: u32) {
        self.color = Some(RgbColor::new(
            ((color >> 16) & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            (color & 0xff) as u8,
        ));
        self.color_dw_raw = Some(color);
    }

    pub(crate) fn exact_surrendered_value(&self) -> i32 {
        retained_runtime_flag_value(self.surrendered_value, self.surrendered)
    }

    pub(crate) fn exact_view_center(&self) -> Vector2 {
        self.view_center
            .or_else(|| self.viewports.first().map(|viewport| viewport.center))
            .unwrap_or(Vector2::ZERO)
    }

    pub(crate) fn exact_knowledge_entries(&self) -> Vec<(DefinitionId, i32)> {
        if self.knowledge_entries.is_empty() && !self.knowledge.is_empty() {
            self.knowledge.iter().cloned().map(|id| (id, 1)).collect()
        } else {
            self.knowledge_entries.clone()
        }
    }

    pub(crate) fn exact_magic_entries(&self) -> Vec<(DefinitionId, i32)> {
        if self.magic_entries.is_empty() && !self.magic.is_empty() {
            self.magic.iter().cloned().map(|id| (id, 1)).collect()
        } else {
            self.magic_entries.clone()
        }
    }

    pub(crate) fn exact_home_base_material_entries(&self) -> Vec<(DefinitionId, i32)> {
        if self.home_base_material_entries.is_empty() && !self.home_base_material.is_empty() {
            ordered_entries_from_unsigned_map(&self.home_base_material)
        } else {
            self.home_base_material_entries.clone()
        }
    }

    pub(crate) fn exact_home_base_production_entries(&self) -> Vec<(DefinitionId, i32)> {
        if self.home_base_production_entries.is_empty() && !self.home_base_production.is_empty() {
            ordered_entries_from_unsigned_map(&self.home_base_production)
        } else {
            self.home_base_production_entries.clone()
        }
    }

    pub(crate) fn exact_hostility_entries(&self) -> Vec<(i32, i32)> {
        if self.hostility_entries.is_empty() && !self.hostility.is_empty() {
            self.hostility
                .iter()
                .copied()
                .map(|opponent| (opponent.wrapping_add(1), 1))
                .collect()
        } else {
            self.hostility_entries.clone()
        }
    }

    pub(crate) fn set_knowledge_entry(&mut self, definition_id: DefinitionId) {
        let mut entries = self.exact_knowledge_entries();
        set_ordered_id_count(&mut entries, definition_id, 1, true);
        self.knowledge = sorted_unique_ids_projection(&entries);
        self.knowledge_entries = entries;
    }

    pub(crate) fn remove_knowledge_entry(&mut self, definition_id: &DefinitionId) -> bool {
        let mut entries = self.exact_knowledge_entries();
        if !delete_ordered_id(&mut entries, definition_id) {
            return false;
        }
        self.knowledge = sorted_unique_ids_projection(&entries);
        self.knowledge_entries = entries;
        true
    }

    pub(crate) fn knows_definition(&self, definition_id: &DefinitionId) -> bool {
        self.exact_knowledge_entries()
            .iter()
            .any(|(id, _)| id == definition_id)
    }

    pub(crate) fn set_magic_entry(&mut self, definition_id: DefinitionId) {
        let mut entries = self.exact_magic_entries();
        set_ordered_id_count(&mut entries, definition_id, 1, true);
        self.magic = ordered_ids_projection(&entries);
        self.magic_entries = entries;
    }

    pub(crate) fn remove_magic_entry(&mut self, definition_id: &DefinitionId) -> bool {
        let mut entries = self.exact_magic_entries();
        if !delete_ordered_id(&mut entries, definition_id) {
            return false;
        }
        self.magic = ordered_ids_projection(&entries);
        self.magic_entries = entries;
        true
    }

    pub(crate) fn knows_magic(&self, definition_id: &DefinitionId) -> bool {
        self.exact_magic_entries()
            .iter()
            .any(|(id, _)| id == definition_id)
    }

    pub(crate) fn set_hostility_entry(&mut self, opponent: i32, hostile: bool) {
        let mut entries = self.exact_hostility_entries();
        set_ordered_id_count(
            &mut entries,
            opponent.wrapping_add(1),
            i32::from(hostile),
            true,
        );
        self.hostility = hostility_projection(&entries);
        self.hostility_entries = entries;
    }

    pub(crate) fn is_hostile_towards(&self, opponent: i32) -> bool {
        if self.hostility_entries.is_empty() {
            self.hostility.contains(&opponent)
        } else {
            ordered_id_count(&self.hostility_entries, &opponent.wrapping_add(1), 0) != 0
        }
    }

    pub(crate) fn set_home_base_material_entries(&mut self, entries: Vec<(DefinitionId, i32)>) {
        self.home_base_material = unsigned_first_count_projection(&entries);
        self.home_base_material_entries = entries;
    }

    pub(crate) fn set_home_base_production_entries(&mut self, entries: Vec<(DefinitionId, i32)>) {
        self.home_base_production = unsigned_first_count_projection(&entries);
        self.home_base_production_entries = entries;
    }

    pub(crate) fn adjust_home_base_material_entry(
        &mut self,
        definition_id: DefinitionId,
        delta: i32,
    ) {
        let mut entries = self.exact_home_base_material_entries();
        let current = ordered_id_count(&entries, &definition_id, 0);
        let updated = current.wrapping_add(delta);
        set_ordered_id_count(&mut entries, definition_id, updated, true);
        self.set_home_base_material_entries(entries);
    }

    pub(crate) fn adjust_home_base_production_entry(
        &mut self,
        definition_id: DefinitionId,
        delta: i32,
    ) {
        let mut entries = self.exact_home_base_production_entries();
        let current = ordered_id_count(&entries, &definition_id, 0);
        let updated = current.wrapping_add(delta);
        set_ordered_id_count(&mut entries, definition_id, updated, true);
        self.set_home_base_production_entries(entries);
    }

    pub(crate) fn set_view_target(&mut self, target: Option<ObjectId>) {
        self.view_mode = PLAYER_VIEW_MODE_TARGET;
        self.view_target = target;
    }

    pub(crate) fn set_view_cursor(&mut self, view_cursor: Option<ObjectId>) {
        self.view_cursor = view_cursor;
        self.sync_viewport_focus();
    }

    fn sync_viewport_focus(&mut self) {
        let focus = self.view_cursor.or(self.cursor);
        for viewport in &mut self.viewports {
            viewport.focus = focus;
        }
    }

    pub(crate) fn clear_object_pointers_before_cursor_adjust(
        &mut self,
        object: ObjectId,
    ) -> bool {
        self.crew.retain(|member| *member != object);
        if self.captain == Some(object) {
            self.captain = None;
        }
        if self.cursor == Some(object) {
            // C4Player::ClearPointers writes Cursor=null before entering
            // AdjustCursorCommand, while ViewCursor/ViewTarget remain live
            // until that callbackful command has returned.
            self.cursor = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn clear_object_pointers_after_cursor_adjust(&mut self, object: ObjectId) {
        if self.view_cursor == Some(object) {
            self.view_cursor = None;
        }
        if self.view_target == Some(object) {
            self.view_target = None;
        }
        self.remove_message_board_query(Some(object));
        self.sync_viewport_focus();
    }

    pub(crate) fn reset_cursor_view(&mut self) {
        if self.view_cursor.is_none() && self.cursor.is_none() {
            return;
        }
        self.view_mode = PLAYER_VIEW_MODE_CURSOR;
        self.view_target = None;
    }

    pub(crate) fn resolved_view_object(&self) -> Option<ObjectId> {
        resolved_view_object(
            self.view_mode,
            self.view_target,
            self.view_cursor,
            self.cursor,
        )
    }

    pub(crate) fn update_view(&mut self, position: Option<Vector2>) {
        let focus = self.view_cursor.or(self.cursor);
        if let Some(position) = position {
            self.view_center = Some(position);
        }
        for viewport in &mut self.viewports {
            viewport.focus = focus;
            if let Some(position) = position {
                viewport.center = position;
            }
        }
    }

    pub(crate) fn clear_object_pointers(&mut self, object: ObjectId) {
        self.clear_object_pointers_before_cursor_adjust(object);
        self.clear_object_pointers_after_cursor_adjust(object);
    }

    pub(crate) fn call_message_board(&mut self, query: MessageBoardQuery) {
        self.remove_message_board_query(query.target);
        self.message_board_queries.push(query);
    }

    pub(crate) fn remove_message_board_query(&mut self, target: Option<ObjectId>) -> bool {
        let Some(index) = self
            .message_board_queries
            .iter()
            .position(|query| query.target == target)
        else {
            return false;
        };
        self.message_board_queries.remove(index);
        true
    }

    pub(crate) fn prepare_for_save(&mut self) {
        self.view_target = None;
        // C4Player saves pMsgBoardQuery through one pointer adaptor, while
        // C4MessageBoardQuery::CompileFunc does not compile pNext. Therefore
        // only the list head survives a save/load cycle.
        self.message_board_queries.truncate(1);
        for query in &mut self.message_board_queries {
            query.answered = false;
        }
    }

    pub(crate) fn restore_runtime_view(&mut self) {
        self.view_target = None;
        let focus = self.view_cursor.or(self.cursor);
        for viewport in &mut self.viewports {
            viewport.focus = focus;
        }
    }
}

/// C4Player's per-player direct-com bookkeeping (C4Player.h:118-121):
/// the LastCom single/double synthesis buffer, the pressed-com bitmask
/// for Jump'n'Run control, and the cursor/select flash timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlayerControlState {
    /// `LastCom` — the full signed compiler word buffered for
    /// COM_Single/COM_Double synthesis (`int32_t` in C4Player.h:121;
    /// C4Player::InCom, C4Player.cpp:1522-1536).
    #[serde(default)]
    pub last_com: i32,
    /// `LastComDelay` — frames since LastCom was buffered; > C4DoubleClick
    /// flushes the COM_Single (C4Player::Execute, C4Player.cpp:1215-1229).
    #[serde(default)]
    pub last_com_delay: i32,
    /// `LastComDownDouble` — countdown after a COM_Down_D that converts the
    /// next throw command to a drop (PlayerObjectCommand,
    /// C4ObjectCom.cpp:1024-1036).
    #[serde(default)]
    pub last_com_down_double: i32,
    /// `PressedComs` — bit per held plain com (C4Player::InCom,
    /// C4Player.cpp:1520-1521, 1541-1548).
    #[serde(default)]
    pub pressed_coms: i32,
    /// `ControlStyle` — Jump'n'Run (AutoStopControl) control when true
    /// (C4Player.cpp:2373; default 0 = classic, C4InfoCore.cpp:84).
    #[serde(default)]
    pub control_style: bool,
    /// Exact persisted `C4Player::ControlStyle` integer.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub control_style_value: i32,
    /// Effective C4Player::AutoContextMenu preference after the scenario
    /// ForcedAutoContextMenu override (C4Player.cpp:2369-2375).
    #[serde(default)]
    pub auto_context_menu: bool,
    /// Exact persisted `C4Player::AutoContextMenu` integer.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub auto_context_menu_value: i32,
    /// `CursorFlash` — frames the cursor arrow above the cursor clonk stays
    /// visible (C4Game::DrawCursors gate, C4Game.cpp:1863; set to 30 on
    /// cursor changes, decremented in C4Player::Execute, C4Player.cpp:242).
    #[serde(default)]
    pub cursor_flash: i32,
    /// `SelectFlash` — frames the crew select marks stay visible
    /// (C4Object::Draw gate, C4Object.cpp:2497-2502).
    #[serde(default)]
    pub select_flash: i32,
    /// `CursorSelection` — armed to 1 by CursorLeft/CursorRight
    /// (C4Player.cpp:1274,1292); the next regular com commits the pick via
    /// UpdateSelectionToggleStatus (:1355-1365).
    #[serde(default)]
    pub cursor_selection: i32,
    /// `CursorToggled` — set when CursorToggle ran in selection mode
    /// (C4Player.cpp:1326); switches the commit from SelectSingleByCursor
    /// to AdjustCursorCommand.
    #[serde(default)]
    pub cursor_toggled: i32,
}

impl PlayerControlState {
    pub(crate) fn exact_control_style_value(&self) -> i32 {
        retained_runtime_flag_value(self.control_style_value, self.control_style)
    }

    pub(crate) fn exact_auto_context_menu_value(&self) -> i32 {
        retained_runtime_flag_value(self.auto_context_menu_value, self.auto_context_menu)
    }

    pub(crate) fn reconcile_integer_flags(&mut self) {
        self.control_style_value = self.exact_control_style_value();
        self.auto_context_menu_value = self.exact_auto_context_menu_value();
    }

    pub(crate) fn set_control_style_value(&mut self, value: i32) {
        self.control_style_value = value;
        self.control_style = value != 0;
    }

    pub(crate) fn set_auto_context_menu_value(&mut self, value: i32) {
        self.auto_context_menu_value = value;
        self.auto_context_menu = value != 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CountedControlType {
    Command,
    DirectCom,
}

#[derive(Debug, Clone)]
pub struct Player {
    id: i32,
    player_info_id: i32,
    at_client: PlayerAtClient,
    at_client_name: String,
    script_player: bool,
    /// `C4Player::NoEliminationCheck`: local/no-save and restored from the
    /// current `C4PlayerInfo`, never from serialized player runtime data.
    no_elimination_check: bool,
    name: String,
    status: PlayerStatus,
    status_value: Option<i32>,
    team: Option<i32>,
    surrendered: bool,
    surrendered_value: i32,
    eliminated_value: i32,
    won: bool,
    evaluated: bool,
    wealth: i32,
    points: i32,
    score: i32,
    rounds: i32,
    rounds_won: i32,
    rounds_lost: i32,
    total_playing_time: i32,
    player_info_core: Option<PlayerInfoCoreState>,
    /// `C4Player::GameJoinTime`: local runtime baseline, deliberately absent
    /// from PlayerState/save data (C4Player.h:78; C4Player.cpp:389-390).
    game_join_time: i32,
    /// `C4Player::RetireDelay`: runtime-only, set to C4RetireDelay by
    /// Eliminate and decremented once per player Execute (C4Player.cpp:
    /// 2015-2021, 239; C4Constants.h:36).
    retire_delay: i32,
    value: i32,
    initial_value: i32,
    value_gain: i32,
    objects_owned: u32,
    initial_value_set: bool,
    knowledge: HashSet<DefinitionId>,
    knowledge_entries: Vec<(DefinitionId, i32)>,
    magic: Vec<DefinitionId>,
    magic_entries: Vec<(DefinitionId, i32)>,
    inventory: HashMap<DefinitionId, u32>,
    cursor: Option<ObjectId>,
    view_mode: i32,
    view_cursor: Option<ObjectId>,
    captain: Option<ObjectId>,
    view_target: Option<ObjectId>,
    /// Saved `C4Player::ViewX/ViewY`, independent of process-local viewports.
    /// `None` exists only as a migration marker for old Rust snapshots.
    view_center: Option<Vector2>,
    viewports: Vec<PlayerViewport>,
    view_offset: Vector2,
    view_wealth: i32,
    view_value: i32,
    crew: Vec<ObjectId>,
    /// Runtime-only `C4Player::FoWViewObjs`. The list is rebuilt from
    /// objects' saved `PlrViewRange` values after a restore and deliberately
    /// does not enter `PlayerState` (`C4Player.h:97`).
    fow_view_objects: Vec<ObjectId>,
    crew_created: i32,
    home_base_material: HashMap<DefinitionId, u32>,
    home_base_material_entries: Vec<(DefinitionId, i32)>,
    home_base_production: HashMap<DefinitionId, u32>,
    home_base_production_entries: Vec<(DefinitionId, i32)>,
    production_delay: u32,
    production_unit: u32,
    color: Option<RgbColor>,
    /// Exact `C4Player::ColorDw` compiler word. `None` retains the migration
    /// distinction for old Rust state whose RGB projection is authoritative.
    color_dw_raw: Option<u32>,
    fog_of_war: bool,
    force_fog_of_war: bool,
    pub(crate) show_control_position: i32,
    pub(crate) show_control: i32,
    /// C4Player::FlashCom (NoSave): the exact contextual command whose key
    /// cell blinks in C4Object::DrawCommand.
    flash_command: i32,
    hostility: HashSet<i32>,
    hostility_entries: Vec<(i32, i32)>,
    /// The indexed player color chosen at ScenarioInit
    /// (C4Player.cpp:678-685; C4PlayerList::ColorTaken scans it). -1 until
    /// the join assigns one.
    color_index: i32,
    /// The startup position slot taken at ScenarioInit
    /// (C4Player.cpp:717-732; C4PlayerList::PositionTaken). -1 when unset.
    position_index: i32,
    /// Runtime input assignment (`C4Player::Control`/`MouseControl`).
    control_set: i32,
    mouse_control: i32,
    pref_control: i32,
    pref_mouse: bool,
    pref_control_style: bool,
    pref_auto_context_menu: bool,
    show_startup: bool,
    select_count: i32,
    message_status: i32,
    message_buf: String,
    message_board_queries: Vec<MessageBoardQuery>,
    /// Runtime-only C4Player control statistics. C++ resets these on player
    /// initialization and omits them from C4Player::CompileFunc.
    control_count: i32,
    action_count: i32,
    last_counted_control: Option<(CountedControlType, i32)>,
    /// Direct-com input state (C4Player.h:118-121).
    #[doc(hidden)]
    pub control: PlayerControlState,
    /// C4Player::ExtraData named slots (Fn[Set/Get]PlrExtraData).
    pub(crate) extra_data: Vec<(String, clonk_script::Value)>,
}

impl Player {
    pub fn new(id: i32, name: impl Into<String>) -> Self {
        Self {
            id,
            player_info_id: 0,
            at_client: PlayerAtClient::UNKNOWN,
            at_client_name: "Local".to_string(),
            script_player: false,
            no_elimination_check: false,
            name: name.into(),
            status: PlayerStatus::Active,
            status_value: None,
            team: None,
            surrendered: false,
            surrendered_value: 0,
            eliminated_value: 0,
            won: false,
            evaluated: false,
            wealth: 0,
            points: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            player_info_core: None,
            game_join_time: 0,
            retire_delay: 0,
            value: 0,
            initial_value: 0,
            value_gain: 0,
            objects_owned: 0,
            initial_value_set: false,
            knowledge: HashSet::new(),
            knowledge_entries: Vec::new(),
            magic: Vec::new(),
            magic_entries: Vec::new(),
            inventory: HashMap::new(),
            cursor: None,
            view_mode: PLAYER_VIEW_MODE_CURSOR,
            view_cursor: None,
            captain: None,
            view_target: None,
            view_center: Some(Vector2::ZERO),
            viewports: Vec::new(),
            view_offset: Vector2::ZERO,
            view_wealth: 0,
            view_value: 0,
            crew: Vec::new(),
            fow_view_objects: Vec::new(),
            crew_created: 0,
            home_base_material: HashMap::new(),
            home_base_material_entries: Vec::new(),
            home_base_production: HashMap::new(),
            home_base_production_entries: Vec::new(),
            production_delay: 0,
            production_unit: 0,
            color: None,
            color_dw_raw: None,
            fog_of_war: false,
            force_fog_of_war: false,
            show_control_position: 0,
            show_control: 0,
            flash_command: 0,
            hostility: HashSet::new(),
            hostility_entries: Vec::new(),
            color_index: -1,
            position_index: -1,
            control_set: -1,
            mouse_control: 0,
            pref_control: 0,
            pref_mouse: true,
            pref_control_style: false,
            pref_auto_context_menu: false,
            show_startup: true,
            select_count: 0,
            message_status: 0,
            message_buf: String::new(),
            message_board_queries: Vec::new(),
            control_count: 0,
            action_count: 0,
            last_counted_control: None,
            control: PlayerControlState::default(),
            extra_data: Vec::new(),
        }
    }

    /// Whether this player uses Jump'n'Run/AutoStop control. Local key-up
    /// routing consults this just as C4Game::LocalControlKeyUp reads
    /// `C4Player::ControlStyle` (C4Game.cpp:3578-3592).
    pub fn control_style(&self) -> bool {
        self.control.control_style
    }

    pub fn control_count(&self) -> i32 {
        self.control_count
    }

    pub fn action_count(&self) -> i32 {
        self.action_count
    }

    /// `C4Player::CountControl`'s player-statistics half. Returns whether
    /// this `(type, id)` pair is a new action and should advance the cursor
    /// crew's runtime C4ObjectInfo control count.
    pub(crate) fn count_control(
        &mut self,
        control_type: CountedControlType,
        id: i32,
        count: i32,
    ) -> bool {
        self.control_count = self.control_count.wrapping_add(count);
        if self.last_counted_control == Some((control_type, id)) {
            return false;
        }
        self.last_counted_control = Some((control_type, id));
        self.action_count = self.action_count.wrapping_add(count);
        true
    }

    /// Drain the control-frame statistics without disturbing
    /// `C4Player::LastControl`. Native action de-duplication spans statistics
    /// samples, so only the two sampled counters are reset here.
    pub(crate) fn take_control_counts(&mut self) -> (i32, i32) {
        (
            std::mem::take(&mut self.control_count),
            std::mem::take(&mut self.action_count),
        )
    }

    pub fn control_set(&self) -> i32 {
        self.control_set
    }

    pub fn mouse_control(&self) -> i32 {
        self.mouse_control
    }

    pub(crate) fn set_runtime_control(&mut self, control_set: i32, mouse_control: i32) {
        self.control_set = control_set;
        self.mouse_control = mouse_control;
        self.control_count = 0;
        self.action_count = 0;
        self.last_counted_control = None;
    }

    pub(crate) fn set_control_preferences(&mut self, pref_control: i32, pref_mouse: bool) {
        self.pref_control = pref_control;
        self.pref_mouse = pref_mouse;
    }

    pub fn control_preferences(&self) -> (i32, bool) {
        (self.pref_control, self.pref_mouse)
    }

    pub(crate) fn set_control_style_preferences(
        &mut self,
        pref_control_style: bool,
        pref_auto_context_menu: bool,
    ) {
        self.pref_control_style = pref_control_style;
        self.pref_auto_context_menu = pref_auto_context_menu;
    }

    pub fn control_style_preferences(&self) -> (bool, bool) {
        (self.pref_control_style, self.pref_auto_context_menu)
    }

    pub fn show_startup(&self) -> bool {
        self.show_startup
    }

    pub fn set_show_startup(&mut self, show: bool) {
        self.show_startup = show;
    }

    pub(crate) fn hide_startup(&mut self) {
        self.show_startup = false;
    }

    pub fn select_count(&self) -> i32 {
        self.select_count
    }

    pub(crate) fn set_select_count(&mut self, select_count: i32) {
        self.select_count = select_count;
    }

    pub fn message_status(&self) -> i32 {
        self.message_status
    }

    pub fn set_message_status(&mut self, status: i32) {
        self.message_status = status;
    }

    pub fn message_buf(&self) -> &str {
        &self.message_buf
    }

    pub fn set_message_buf(&mut self, message: impl Into<String>) {
        self.message_buf = bounded_message_buf(message.into());
    }

    pub fn message_board_queries(&self) -> &[MessageBoardQuery] {
        &self.message_board_queries
    }

    pub(crate) fn call_message_board(&mut self, query: MessageBoardQuery) {
        self.remove_message_board_query(query.target);
        self.message_board_queries.push(query);
    }

    pub(crate) fn remove_message_board_query(&mut self, target: Option<ObjectId>) -> bool {
        let Some(index) = self
            .message_board_queries
            .iter()
            .position(|query| query.target == target)
        else {
            return false;
        };
        self.message_board_queries.remove(index);
        true
    }

    pub(crate) fn mark_message_board_query_answered(&mut self, target: Option<ObjectId>) -> bool {
        let Some(query) = self
            .message_board_queries
            .iter_mut()
            .find(|query| query.target == target && !query.answered)
        else {
            return false;
        };
        query.answered = true;
        true
    }

    pub fn view_wealth(&self) -> i32 {
        self.view_wealth
    }

    pub fn set_view_wealth(&mut self, delay: i32) {
        self.view_wealth = delay;
    }

    pub(crate) fn arm_view_wealth(&mut self) {
        self.view_wealth = PLAYER_VIEW_DELAY;
    }

    pub fn view_value(&self) -> i32 {
        self.view_value
    }

    pub fn set_view_value(&mut self, delay: i32) {
        self.view_value = delay;
    }

    pub(crate) fn arm_view_value(&mut self) {
        self.view_value = PLAYER_VIEW_DELAY;
    }

    pub(crate) fn advance_runtime_delays(&mut self) {
        if self.status == PlayerStatus::Inactive {
            return;
        }
        if self.message_status > 0 {
            self.message_status -= 1;
        }
        if self.view_wealth > 0 {
            self.view_wealth -= 1;
        }
        if self.view_value > 0 {
            self.view_value -= 1;
        }
    }

    pub fn from_config(config: PlayerConfig) -> Self {
        let PlayerConfig {
            id,
            player_info_id,
            name,
            status,
            team,
            surrendered,
            wealth,
            points,
            score,
            rounds,
            rounds_won,
            rounds_lost,
            total_playing_time,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge,
            magic,
            inventory,
            cursor,
            viewports,
            home_base_material,
            home_base_production,
            production_delay,
            production_unit,
            color,
        } = config;
        let knowledge_entries = knowledge.into_iter().map(|id| (id, 1)).collect::<Vec<_>>();
        let knowledge = knowledge_entries.iter().map(|(id, _)| id.clone()).collect();
        let magic_entries = magic.into_iter().map(|id| (id, 1)).collect::<Vec<_>>();
        let magic = magic_entries.iter().map(|(id, _)| id.clone()).collect();
        let home_base_material_entries = ordered_entries_from_unsigned_map(&home_base_material);
        let home_base_production_entries = ordered_entries_from_unsigned_map(&home_base_production);
        let view_center = viewports
            .first()
            .map(|viewport| viewport.center)
            .unwrap_or(Vector2::ZERO);
        Self {
            id,
            player_info_id,
            at_client: PlayerAtClient::UNKNOWN,
            at_client_name: "Local".to_string(),
            script_player: false,
            no_elimination_check: false,
            name,
            status,
            status_value: None,
            team,
            surrendered,
            surrendered_value: i32::from(surrendered),
            eliminated_value: i32::from(matches!(
                status,
                PlayerStatus::Eliminated | PlayerStatus::Surrendered
            )),
            won: false,
            evaluated: false,
            wealth,
            points,
            score,
            rounds,
            rounds_won,
            rounds_lost,
            total_playing_time,
            player_info_core: None,
            game_join_time: 0,
            retire_delay: 0,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge,
            knowledge_entries,
            magic,
            magic_entries,
            inventory,
            cursor,
            view_mode: PLAYER_VIEW_MODE_CURSOR,
            view_cursor: None,
            captain: None,
            view_target: None,
            view_center: Some(view_center),
            viewports,
            view_offset: Vector2::ZERO,
            view_wealth: 0,
            view_value: 0,
            crew: Vec::new(),
            fow_view_objects: Vec::new(),
            crew_created: 0,
            home_base_material,
            home_base_material_entries,
            home_base_production,
            home_base_production_entries,
            production_delay,
            production_unit,
            color,
            color_dw_raw: None,
            fog_of_war: false,
            force_fog_of_war: false,
            show_control_position: 0,
            show_control: 0,
            flash_command: 0,
            hostility: HashSet::new(),
            hostility_entries: Vec::new(),
            color_index: -1,
            position_index: -1,
            control_set: -1,
            mouse_control: 0,
            pref_control: 0,
            pref_mouse: true,
            pref_control_style: false,
            pref_auto_context_menu: false,
            show_startup: true,
            select_count: 0,
            message_status: 0,
            message_buf: String::new(),
            message_board_queries: Vec::new(),
            control_count: 0,
            action_count: 0,
            last_counted_control: None,
            control: PlayerControlState::default(),
            extra_data: Vec::new(),
        }
    }

    pub fn from_state(state: PlayerState) -> Self {
        let PlayerState {
            id,
            player_info_id,
            at_client,
            at_client_name,
            script_player,
            no_elimination_check,
            name,
            status,
            status_value,
            team,
            surrendered,
            surrendered_value,
            eliminated_value,
            won,
            evaluated,
            wealth,
            points,
            score,
            rounds,
            rounds_won,
            rounds_lost,
            total_playing_time,
            player_info_core,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge,
            knowledge_entries,
            magic,
            magic_entries,
            inventory,
            cursor,
            view_mode,
            view_cursor,
            captain,
            view_target: _,
            view_center,
            viewports,
            view_offset,
            view_wealth,
            view_value,
            crew,
            crew_created,
            home_base_material,
            home_base_material_entries,
            home_base_production,
            home_base_production_entries,
            production_delay,
            production_unit,
            color,
            color_dw_raw,
            color_index,
            position_index,
            control_set,
            mouse_control,
            pref_control,
            pref_mouse,
            pref_control_style,
            pref_auto_context_menu,
            fog_of_war,
            force_fog_of_war,
            show_startup,
            select_count,
            message_status,
            message_buf,
            message_board_queries,
            show_control_position,
            show_control,
            hostility,
            hostility_entries,
            mut control,
            extra_data,
        } = state;
        let surrendered_value = retained_runtime_flag_value(surrendered_value, surrendered);
        let eliminated_value = retained_runtime_flag_value(
            eliminated_value,
            matches!(status, PlayerStatus::Eliminated | PlayerStatus::Surrendered),
        );
        control.reconcile_integer_flags();
        let knowledge_entries = if knowledge_entries.is_empty() && !knowledge.is_empty() {
            knowledge.into_iter().map(|id| (id, 1)).collect()
        } else {
            knowledge_entries
        };
        let knowledge = knowledge_entries.iter().map(|(id, _)| id.clone()).collect();
        let magic_entries = if magic_entries.is_empty() && !magic.is_empty() {
            magic.into_iter().map(|id| (id, 1)).collect()
        } else {
            magic_entries
        };
        let magic = magic_entries.iter().map(|(id, _)| id.clone()).collect();
        let home_base_material_entries =
            if home_base_material_entries.is_empty() && !home_base_material.is_empty() {
                ordered_entries_from_unsigned_map(&home_base_material)
            } else {
                home_base_material_entries
            };
        let home_base_material = unsigned_first_count_projection(&home_base_material_entries);
        let home_base_production_entries =
            if home_base_production_entries.is_empty() && !home_base_production.is_empty() {
                ordered_entries_from_unsigned_map(&home_base_production)
            } else {
                home_base_production_entries
            };
        let home_base_production = unsigned_first_count_projection(&home_base_production_entries);
        let hostility_entries = if hostility_entries.is_empty() && !hostility.is_empty() {
            hostility
                .into_iter()
                .map(|opponent| (opponent.wrapping_add(1), 1))
                .collect()
        } else {
            hostility_entries
        };
        let hostility = hostility_projection(&hostility_entries)
            .into_iter()
            .collect();
        Self {
            id,
            player_info_id,
            at_client,
            at_client_name: bounded_at_client_name(
                at_client_name.unwrap_or_else(|| "Local".to_string()),
            ),
            script_player,
            no_elimination_check,
            name,
            status,
            status_value,
            team,
            surrendered,
            surrendered_value,
            eliminated_value,
            won,
            evaluated,
            wealth,
            points,
            score,
            rounds,
            rounds_won,
            rounds_lost,
            total_playing_time,
            player_info_core,
            game_join_time: 0,
            retire_delay: 0,
            value,
            initial_value,
            value_gain,
            objects_owned,
            initial_value_set,
            knowledge,
            knowledge_entries,
            magic,
            magic_entries,
            inventory,
            cursor,
            view_mode,
            view_cursor,
            captain,
            view_target: None,
            view_center,
            viewports,
            view_offset,
            view_wealth,
            view_value,
            crew,
            fow_view_objects: Vec::new(),
            crew_created,
            home_base_material,
            home_base_material_entries,
            home_base_production,
            home_base_production_entries,
            production_delay,
            production_unit,
            color,
            color_dw_raw,
            fog_of_war,
            force_fog_of_war,
            show_control_position,
            show_control,
            flash_command: 0,
            hostility,
            hostility_entries,
            color_index: color_index.unwrap_or(-1),
            position_index: position_index.unwrap_or(-1),
            control_set,
            mouse_control,
            pref_control,
            pref_mouse: pref_mouse.unwrap_or(true),
            pref_control_style,
            pref_auto_context_menu,
            show_startup,
            select_count,
            message_status,
            message_buf: bounded_message_buf(message_buf),
            message_board_queries,
            control_count: 0,
            action_count: 0,
            last_counted_control: None,
            control,
            extra_data,
        }
    }

    pub fn to_state(&self) -> PlayerState {
        PlayerState {
            id: self.id,
            player_info_id: self.player_info_id,
            at_client: self.at_client,
            at_client_name: (self.at_client_name != "Local").then(|| self.at_client_name.clone()),
            script_player: self.script_player,
            no_elimination_check: self.no_elimination_check,
            name: self.name.clone(),
            status: self.status,
            status_value: self.status_value,
            team: self.team,
            surrendered: self.surrendered,
            surrendered_value: self.surrendered_value,
            eliminated_value: self.eliminated_value,
            won: self.won,
            evaluated: self.evaluated,
            wealth: self.wealth,
            points: self.points,
            score: self.score,
            rounds: self.rounds,
            rounds_won: self.rounds_won,
            rounds_lost: self.rounds_lost,
            total_playing_time: self.total_playing_time,
            player_info_core: self.player_info_core.clone(),
            value: self.value,
            initial_value: self.initial_value,
            value_gain: self.value_gain,
            objects_owned: self.objects_owned,
            initial_value_set: self.initial_value_set,
            knowledge: sorted_unique_ids_projection(&self.knowledge_entries),
            knowledge_entries: self.knowledge_entries.clone(),
            magic: ordered_ids_projection(&self.magic_entries),
            magic_entries: self.magic_entries.clone(),
            inventory: self.inventory.clone(),
            cursor: self.cursor,
            view_mode: self.view_mode,
            view_cursor: self.view_cursor,
            captain: self.captain,
            view_target: self.view_target,
            view_center: self.view_center,
            viewports: self.viewports.clone(),
            view_offset: self.view_offset,
            view_wealth: self.view_wealth,
            view_value: self.view_value,
            crew: self.crew.clone(),
            crew_created: self.crew_created,
            home_base_material: self.home_base_material.clone(),
            home_base_material_entries: self.home_base_material_entries.clone(),
            home_base_production: self.home_base_production.clone(),
            home_base_production_entries: self.home_base_production_entries.clone(),
            production_delay: self.production_delay,
            production_unit: self.production_unit,
            color: self.color,
            color_dw_raw: self.color_dw_raw,
            color_index: (self.color_index != -1).then_some(self.color_index),
            position_index: (self.position_index != -1).then_some(self.position_index),
            control_set: self.control_set,
            mouse_control: self.mouse_control,
            pref_control: self.pref_control,
            pref_mouse: (!self.pref_mouse).then_some(false),
            pref_control_style: self.pref_control_style,
            pref_auto_context_menu: self.pref_auto_context_menu,
            fog_of_war: self.fog_of_war,
            force_fog_of_war: self.force_fog_of_war,
            show_startup: self.show_startup,
            select_count: self.select_count,
            message_status: self.message_status,
            message_buf: self.message_buf.clone(),
            message_board_queries: self.message_board_queries.clone(),
            show_control_position: self.show_control_position,
            show_control: self.show_control,
            hostility: hostility_projection(&self.hostility_entries),
            hostility_entries: self.hostility_entries.clone(),
            control: self.control,
            extra_data: self.extra_data.clone(),
        }
    }

    /// Declare or revoke hostility toward another player
    /// (C4Player::Hostility set, fed into C4PlayerList::Hostile).
    pub fn set_hostile_towards(&mut self, opponent: i32, hostile: bool) {
        set_ordered_id_count(
            &mut self.hostility_entries,
            opponent.wrapping_add(1),
            i32::from(hostile),
            true,
        );
        self.hostility = hostility_projection(&self.hostility_entries)
            .into_iter()
            .collect();
    }

    pub fn is_hostile_towards(&self, opponent: i32) -> bool {
        ordered_id_count(&self.hostility_entries, &opponent.wrapping_add(1), 0) != 0
    }

    pub(crate) fn hostility_entries(&self) -> &[(i32, i32)] {
        &self.hostility_entries
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn player_info_id(&self) -> i32 {
        self.player_info_id
    }

    pub fn player_info_core(&self) -> Option<&PlayerInfoCoreState> {
        self.player_info_core.as_ref()
    }

    /// Attach the exact profile core that produced this runtime player.
    /// C4Player inherits C4PlayerInfoCore, so its ExtraData map is the same
    /// object scripts query at runtime rather than a second retained copy.
    pub fn set_player_info_core(&mut self, core: PlayerInfoCoreState) {
        self.extra_data.clone_from(&core.extra_data);
        self.player_info_core = Some(core);
    }

    pub fn at_client(&self) -> PlayerAtClient {
        self.at_client
    }

    pub fn set_at_client(&mut self, at_client: PlayerAtClient) {
        self.at_client = at_client;
    }

    pub fn at_client_name(&self) -> &str {
        &self.at_client_name
    }

    pub fn set_at_client_name(&mut self, name: impl Into<String>) {
        self.at_client_name = bounded_at_client_name(name.into());
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_script_player(&self) -> bool {
        self.script_player
    }

    pub(crate) fn set_script_player(&mut self, script_player: bool) {
        self.script_player = script_player;
    }

    pub fn no_elimination_check(&self) -> bool {
        self.no_elimination_check
    }

    pub(crate) fn set_no_elimination_check(&mut self, no_elimination_check: bool) {
        self.no_elimination_check = no_elimination_check;
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn status(&self) -> PlayerStatus {
        self.status
    }

    pub fn set_status(&mut self, status: PlayerStatus) {
        if matches!(status, PlayerStatus::Eliminated) {
            self.surrendered = false;
            self.surrendered_value = 0;
        }
        self.status = status;
        self.status_value = None;
        self.eliminated_value = i32::from(matches!(
            status,
            PlayerStatus::Eliminated | PlayerStatus::Surrendered
        ));
    }

    /// C4Player::Eliminate's one-way state transition and 60-frame retire
    /// delay (C4Player.cpp:2015-2021; C4Constants.h:36).
    pub(crate) fn eliminate(&mut self) -> bool {
        if self.status == PlayerStatus::Eliminated {
            return false;
        }
        self.status_value = Some(
            self.status_value
                .unwrap_or_else(|| player_status_compiler_value(self.status)),
        );
        self.surrendered = false;
        self.surrendered_value = 0;
        self.eliminated_value = 1;
        self.status = PlayerStatus::Eliminated;
        self.retire_delay = 60;
        true
    }

    /// Player::Execute decrements the delay before C4PlayerList retires at
    /// most one ready eliminated player after the player loop.
    pub(crate) fn advance_retire_delay(&mut self) -> bool {
        if self.retire_delay > 0 {
            self.retire_delay -= 1;
        }
        matches!(
            self.status,
            PlayerStatus::Eliminated | PlayerStatus::Surrendered
        ) && self.retire_delay == 0
    }

    pub fn team(&self) -> Option<i32> {
        self.team
    }

    pub fn set_team(&mut self, team: Option<i32>) {
        self.team = team;
    }

    pub fn color(&self) -> Option<RgbColor> {
        self.color
    }

    pub fn set_color(&mut self, color: Option<RgbColor>) {
        self.color = color;
        self.color_dw_raw = None;
    }

    pub(crate) fn color_dw(&self) -> u32 {
        exact_player_color_dw(self.color_dw_raw, self.color)
    }

    pub(crate) fn set_color_dw(&mut self, color: u32) {
        self.color = Some(RgbColor::new(
            ((color >> 16) & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            (color & 0xff) as u8,
        ));
        self.color_dw_raw = Some(color);
    }

    pub fn fog_of_war(&self) -> bool {
        self.fog_of_war
    }

    pub fn force_fog_of_war(&self) -> bool {
        self.force_fog_of_war
    }

    pub fn flash_command(&self) -> i32 {
        self.flash_command
    }

    pub(crate) fn set_flash_command(&mut self, command: i32) {
        self.flash_command = command;
    }

    /// Explicitly enable or disable fog of war. Unlike automatic mouse-control
    /// selection, either value forces the setting (C4Player.cpp:815-824).
    pub fn set_fog_of_war(&mut self, enabled: bool) {
        self.fog_of_war = enabled;
        self.force_fog_of_war = true;
    }

    pub(crate) fn initialize_mouse_fog_of_war(&mut self) {
        if self.mouse_control != 0 && !self.force_fog_of_war && !self.fog_of_war {
            self.fog_of_war = true;
        }
    }

    pub(crate) fn apply_mouse_control_toggle(&mut self, enabled: bool) {
        self.mouse_control = i32::from(enabled);
        if enabled {
            if !self.force_fog_of_war && !self.fog_of_war {
                self.fog_of_war = true;
            }
        } else {
            if self.view_mode == PLAYER_VIEW_MODE_SCROLLING {
                self.view_mode = PLAYER_VIEW_MODE_CURSOR;
                self.view_target = None;
            }
            if !self.force_fog_of_war {
                self.fog_of_war = false;
            }
        }
    }

    pub fn surrendered(&self) -> bool {
        self.surrendered
    }

    pub(crate) fn mark_won(&mut self) {
        self.won = true;
    }

    /// C4Player::Evaluate's settlement score, persistent round counters and
    /// profile-time update. Returns the old/new score pair exactly once.
    pub(crate) fn evaluate(
        &mut self,
        average_value_gain: i32,
        melee: bool,
        game_time: i32,
        scenario_title: String,
        unix_time: u32,
    ) -> Option<(i32, i32)> {
        if self.evaluated {
            return None;
        }
        self.won = !matches!(
            self.status,
            PlayerStatus::Eliminated | PlayerStatus::Surrendered
        ) && !self.surrendered;
        let score_old = self.score;
        let success_bonus = if self.won { 100 } else { 0 };
        let settlement_score = if melee {
            self.value_gain
        } else {
            average_value_gain
        }
        .max(0);
        let final_score = settlement_score.wrapping_add(success_bonus);
        let total_score = self.score.wrapping_add(final_score);
        self.player_info_core
            .get_or_insert_with(PlayerInfoCoreState::default)
            .last_round = PlayerLastRoundState {
            title: scenario_title,
            date: unix_time,
            duration: game_time,
            won: i32::from(self.won),
            score: settlement_score,
            final_score,
            total_score,
            bonus: success_bonus,
            level: 0,
        };
        self.score = total_score;
        self.rounds = self.rounds.wrapping_add(1);
        if self.won {
            self.rounds_won = self.rounds_won.wrapping_add(1);
        } else {
            self.rounds_lost = self.rounds_lost.wrapping_add(1);
        }
        self.total_playing_time = self
            .total_playing_time
            .wrapping_add(game_time.wrapping_sub(self.game_join_time));
        self.evaluated = true;
        Some((score_old, self.score))
    }

    pub fn set_surrendered(&mut self, surrendered: bool) {
        // C4Player::Surrender is idempotent: a repeated surrender must not
        // restart an already-running RetireDelay (C4Player.cpp:971-979).
        if surrendered && self.surrendered {
            return;
        }
        self.surrendered = surrendered;
        self.surrendered_value = i32::from(surrendered);
        if surrendered {
            self.status_value = Some(
                self.status_value
                    .unwrap_or_else(|| player_status_compiler_value(self.status)),
            );
            self.eliminated_value = 1;
            self.status = PlayerStatus::Surrendered;
            self.retire_delay = 60;
        } else if self.status == PlayerStatus::Surrendered {
            self.status = PlayerStatus::Active;
            self.status_value = None;
            self.eliminated_value = 0;
            self.retire_delay = 0;
        }
    }

    pub fn wealth(&self) -> i32 {
        self.wealth
    }

    pub fn set_wealth(&mut self, wealth: i32) {
        self.wealth = wealth.clamp(0, MAX_SET_WEALTH);
    }

    pub fn adjust_wealth(&mut self, delta: i32) -> i32 {
        let updated = (self.wealth as i64 + i64::from(delta))
            .clamp(0, i64::from(MAX_WEALTH_ADJUSTMENT)) as i32;
        self.wealth = updated;
        self.arm_view_wealth();
        self.wealth
    }

    pub fn points(&self) -> i32 {
        self.points
    }

    pub fn score(&self) -> i32 {
        self.score
    }

    pub fn rounds(&self) -> i32 {
        self.rounds
    }

    pub fn rounds_won(&self) -> i32 {
        self.rounds_won
    }

    pub fn rounds_lost(&self) -> i32 {
        self.rounds_lost
    }

    pub fn total_playing_time(&self) -> i32 {
        self.total_playing_time
    }

    #[doc(hidden)]
    pub fn game_join_time(&self) -> i32 {
        self.game_join_time
    }

    #[doc(hidden)]
    pub fn set_game_join_time(&mut self, game_time: i32) {
        self.game_join_time = game_time;
    }

    /// `C4Player::LocalSync`'s process-local playing-time checkpoint.
    pub(crate) fn synchronize_playing_time(&mut self, game_time: i32) {
        self.total_playing_time = self
            .total_playing_time
            .wrapping_add(game_time.wrapping_sub(self.game_join_time));
        self.game_join_time = game_time;
    }

    pub fn set_points(&mut self, points: i32) -> i32 {
        self.points = points.clamp(MIN_SCORE, MAX_SCORE);
        self.points
    }

    pub fn adjust_points(&mut self, delta: i32) -> i32 {
        let updated = (self.points as i64 + i64::from(delta))
            .clamp(i64::from(MIN_SCORE), i64::from(MAX_SCORE)) as i32;
        self.points = updated;
        self.arm_view_value();
        self.points
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn value_gain(&self) -> i32 {
        self.value_gain
    }

    pub fn initial_value(&self) -> i32 {
        self.initial_value
    }

    pub(crate) fn initial_value_is_set(&self) -> bool {
        self.initial_value_set
    }

    pub fn objects_owned(&self) -> u32 {
        self.objects_owned
    }

    pub fn update_asset_value(&mut self, value: i32, objects_owned: u32) {
        let previous = (self.value_gain, self.objects_owned);
        self.value = value;
        self.objects_owned = objects_owned;
        self.finish_asset_value_update(previous);
    }

    pub(crate) fn begin_asset_value_update(&mut self) -> (i32, u32) {
        let previous = (self.value_gain, self.objects_owned);
        self.value = self.points.wrapping_add(self.wealth);
        self.objects_owned = 0;
        previous
    }

    pub(crate) fn count_owned_asset(&mut self) {
        self.objects_owned = self.objects_owned.wrapping_add(1);
    }

    pub(crate) fn add_asset_value(&mut self, value: i32) {
        self.value = self.value.wrapping_add(value);
    }

    pub(crate) fn finish_asset_value_update(&mut self, previous: (i32, u32)) {
        self.value_gain = self.value.wrapping_sub(self.initial_value);
        if self.value_gain != previous.0 || self.objects_owned != previous.1 {
            self.arm_view_value();
        }
    }

    pub fn reset_initial_value(&mut self) {
        self.initial_value = self.value;
        self.initial_value_set = true;
    }

    pub fn knowledge(&self) -> impl Iterator<Item = &DefinitionId> {
        self.knowledge_entries.iter().map(|(id, _)| id)
    }

    pub(crate) fn knowledge_entries(&self) -> &[(DefinitionId, i32)] {
        &self.knowledge_entries
    }

    pub(crate) fn set_knowledge_entries(&mut self, entries: Vec<(DefinitionId, i32)>) {
        self.knowledge = entries.iter().map(|(id, _)| id.clone()).collect();
        self.knowledge_entries = entries;
    }

    pub fn grant_knowledge(&mut self, definition_id: DefinitionId) {
        set_ordered_id_count(&mut self.knowledge_entries, definition_id, 1, true);
        self.knowledge = self
            .knowledge_entries
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
    }

    pub fn revoke_knowledge(&mut self, definition_id: &DefinitionId) {
        delete_ordered_id(&mut self.knowledge_entries, definition_id);
        self.knowledge = self
            .knowledge_entries
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
    }

    pub fn magic(&self) -> impl Iterator<Item = &DefinitionId> {
        self.magic_entries.iter().map(|(id, _)| id)
    }

    pub(crate) fn magic_entries(&self) -> &[(DefinitionId, i32)] {
        &self.magic_entries
    }

    pub(crate) fn set_magic_entries(&mut self, entries: Vec<(DefinitionId, i32)>) {
        self.magic = ordered_ids_projection(&entries);
        self.magic_entries = entries;
    }

    pub fn set_magic(&mut self, magic: Vec<DefinitionId>) {
        self.set_magic_entries(magic.into_iter().map(|id| (id, 1)).collect());
    }

    pub fn grant_magic(&mut self, definition_id: DefinitionId) {
        set_ordered_id_count(&mut self.magic_entries, definition_id, 1, true);
        self.magic = ordered_ids_projection(&self.magic_entries);
    }

    pub fn revoke_magic(&mut self, definition_id: &DefinitionId) {
        delete_ordered_id(&mut self.magic_entries, definition_id);
        self.magic = ordered_ids_projection(&self.magic_entries);
    }

    pub fn inventory(&self) -> &HashMap<DefinitionId, u32> {
        &self.inventory
    }

    pub fn set_inventory_item(&mut self, definition_id: DefinitionId, quantity: u32) {
        if quantity == 0 {
            self.inventory.remove(&definition_id);
        } else {
            self.inventory.insert(definition_id, quantity);
        }
    }

    pub fn adjust_inventory_item(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        let current = self.inventory.get(&definition_id).copied().unwrap_or(0);
        let updated = if delta >= 0 {
            current.saturating_add(delta as u32)
        } else {
            let decrease = delta.checked_abs().unwrap_or(i32::MAX) as u32;
            current.saturating_sub(decrease)
        };
        if updated == 0 {
            self.inventory.remove(&definition_id);
        } else {
            self.inventory.insert(definition_id.clone(), updated);
        }
        updated
    }

    pub fn cursor(&self) -> Option<ObjectId> {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: Option<ObjectId>) {
        self.cursor = cursor;
        self.sync_viewport_focus();
    }

    pub fn captain(&self) -> Option<ObjectId> {
        self.captain
    }

    pub(crate) fn set_captain(&mut self, captain: Option<ObjectId>) {
        self.captain = captain;
    }

    pub fn view_cursor(&self) -> Option<ObjectId> {
        self.view_cursor
    }

    /// Raw `C4ControlMessage::Say` focus lookup: live `ViewTarget` first,
    /// then `Cursor`. This deliberately ignores ViewMode and ViewCursor;
    /// C++'s message control reads those two object pointers directly
    /// (`src/C4Control.cpp:1141-1145`).
    pub(crate) fn raw_view_target_or_cursor(&self) -> Option<ObjectId> {
        self.view_target.or(self.cursor)
    }

    pub(crate) fn raw_view_target(&self) -> Option<ObjectId> {
        self.view_target
    }

    pub(crate) fn raw_view_mode(&self) -> i32 {
        self.view_mode
    }

    pub(crate) fn resolved_view_object(&self) -> Option<ObjectId> {
        resolved_view_object(
            self.view_mode,
            self.view_target,
            self.view_cursor,
            self.cursor,
        )
    }

    pub fn set_view_cursor(&mut self, view_cursor: Option<ObjectId>) {
        self.view_cursor = view_cursor;
        self.sync_viewport_focus();
    }

    pub(crate) fn set_view_target(&mut self, view_target: Option<ObjectId>) {
        self.view_mode = PLAYER_VIEW_MODE_TARGET;
        self.view_target = view_target;
    }

    pub(crate) fn scroll_view(
        &mut self,
        delta: Vector2,
        view_width: i32,
        view_height: i32,
        world_width: i32,
        world_height: i32,
        fullscreen: bool,
    ) {
        // C4Player::ScrollView enters scrolling mode through SetViewMode,
        // which also clears a temporary target (C4Player.cpp:917-920,
        // 1863-1869).
        self.view_mode = PLAYER_VIEW_MODE_SCROLLING;
        self.view_target = None;

        let border = if fullscreen {
            VIEWPORT_SCROLL_BORDER
        } else {
            0
        };
        let min_x = view_width / 2 - border;
        let max_x = world_width + border - view_width / 2;
        let min_y = view_height / 2 - border;
        let max_y = world_height + border - view_height / 2;

        let view_center = self.view_center();
        self.view_center = Some(Vector2::new(
            bound_view_center(view_center.x.wrapping_add(delta.x), min_x, max_x),
            bound_view_center(view_center.y.wrapping_add(delta.y), min_y, max_y),
        ));

        for viewport in &mut self.viewports {
            viewport.center.x = bound_view_center(
                viewport.center.x.wrapping_add(delta.x),
                min_x,
                max_x,
            );
            viewport.center.y = bound_view_center(
                viewport.center.y.wrapping_add(delta.y),
                min_y,
                max_y,
            );
        }
    }

    pub(crate) fn reset_cursor_view(&mut self) {
        if self.view_cursor.is_none() && self.cursor.is_none() {
            return;
        }
        self.view_mode = PLAYER_VIEW_MODE_CURSOR;
        self.view_target = None;
    }

    pub(crate) fn clear_object_pointers(&mut self, object: ObjectId) {
        self.clear_object_pointers_before_cursor_adjust(object);
        self.clear_object_pointers_after_cursor_adjust(object);
    }

    pub(crate) fn clear_object_pointers_before_cursor_adjust(
        &mut self,
        object: ObjectId,
    ) -> bool {
        self.crew.retain(|member| *member != object);
        if self.captain == Some(object) {
            self.captain = None;
        }
        if self.cursor == Some(object) {
            self.cursor = None;
            return true;
        }
        false
    }

    pub(crate) fn clear_object_pointers_after_cursor_adjust(&mut self, object: ObjectId) {
        if self.view_cursor == Some(object) {
            self.view_cursor = None;
        }
        if self.view_target == Some(object) {
            self.view_target = None;
        }
        self.remove_message_board_query(Some(object));
        self.sync_viewport_focus();
    }

    /// Apply the destructive DenumeratePointers half of C4Player save
    /// preparation. Explicit object-pointer adaptors become null when their
    /// target is not in C4GameObjects; list/value fields use separate compiler
    /// paths and deliberately remain untouched.
    pub(crate) fn denumerate_live_save_pointer_fields(&mut self, object_numbers: &HashSet<u64>) {
        let absent = |object: ObjectId| !object_numbers.contains(&object.as_u64());
        if self.cursor.is_some_and(absent) {
            self.cursor = None;
        }
        if self.view_cursor.is_some_and(absent) {
            self.view_cursor = None;
        }
        if self.captain.is_some_and(absent) {
            self.captain = None;
        }
        for query in &mut self.message_board_queries {
            if query.target.is_some_and(absent) {
                query.target = None;
            }
        }
        self.sync_viewport_focus();
    }

    fn sync_viewport_focus(&mut self) {
        let focus = self.view_cursor.or(self.cursor);
        for viewport in &mut self.viewports {
            viewport.focus = focus;
        }
    }

    pub(crate) fn update_view(&mut self, position: Option<Vector2>) {
        let focus = self.view_cursor.or(self.cursor);
        if let Some(position) = position {
            self.view_center = Some(position);
        }
        if self.viewports.is_empty() {
            if let Some(position) = position {
                self.viewports
                    .push(PlayerViewport::new(position).with_focus(focus));
            }
        } else {
            for viewport in &mut self.viewports {
                viewport.focus = focus;
                if let Some(position) = position {
                    viewport.center = position;
                }
            }
        }
    }

    pub fn viewports(&self) -> &[PlayerViewport] {
        &self.viewports
    }

    /// Current `C4Player::ViewX/ViewY`, with the first logical viewport used
    /// only to migrate snapshots written before the independent fields existed.
    pub fn view_center(&self) -> Vector2 {
        self.view_center
            .or_else(|| self.viewports.first().map(|viewport| viewport.center))
            .unwrap_or(Vector2::ZERO)
    }

    pub fn set_view_center(&mut self, center: Vector2) {
        self.view_center = Some(center);
    }

    pub fn view_offset(&self) -> Vector2 {
        self.view_offset
    }

    pub fn set_view_offset(&mut self, offset: Vector2) {
        self.view_offset = offset;
    }

    pub fn replace_viewports(&mut self, viewports: Vec<PlayerViewport>) {
        self.viewports = viewports;
    }

    pub fn set_viewport(&mut self, index: usize, viewport: PlayerViewport) {
        if self.viewports.len() <= index {
            self.viewports
                .resize_with(index + 1, || PlayerViewport::new(Vector2::ZERO));
        }
        self.viewports[index] = viewport;
    }

    pub fn crew(&self) -> &[ObjectId] {
        &self.crew
    }

    pub(crate) fn fow_view_objects(&self) -> &[ObjectId] {
        &self.fow_view_objects
    }

    pub(crate) fn has_fow_view_object(&self, object: ObjectId) -> bool {
        self.fow_view_objects.contains(&object)
    }

    pub(crate) fn add_fow_view_object(&mut self, object: ObjectId) {
        if !self.fow_view_objects.contains(&object) {
            self.fow_view_objects.push(object);
        }
    }

    pub(crate) fn remove_fow_view_object(&mut self, object: ObjectId) {
        self.fow_view_objects.retain(|candidate| *candidate != object);
    }

    pub(crate) fn clear_fow_view_objects(&mut self) {
        self.fow_view_objects.clear();
    }

    pub fn crew_created(&self) -> i32 {
        self.crew_created
    }

    pub fn set_crew_created(&mut self, crew_created: i32) {
        self.crew_created = crew_created;
    }

    pub(crate) fn increment_crew_created(&mut self) -> i32 {
        self.crew_created = self.crew_created.wrapping_add(1);
        self.crew_created
    }

    pub fn color_index(&self) -> i32 {
        self.color_index
    }

    pub fn set_color_index(&mut self, index: i32) {
        self.color_index = index;
    }

    pub fn position_index(&self) -> i32 {
        self.position_index
    }

    pub fn set_position_index(&mut self, index: i32) {
        self.position_index = index;
    }

    pub fn set_crew(&mut self, crew: Vec<ObjectId>) {
        self.crew = crew;
    }

    pub fn home_base_material(&self) -> &HashMap<DefinitionId, u32> {
        &self.home_base_material
    }

    pub fn home_base_material_entries(&self) -> &[(DefinitionId, i32)] {
        &self.home_base_material_entries
    }

    pub fn set_home_base_material(&mut self, material: HashMap<DefinitionId, u32>) {
        self.set_home_base_material_entries(ordered_entries_from_unsigned_map(&material));
    }

    pub fn set_home_base_material_entries(&mut self, entries: Vec<(DefinitionId, i32)>) {
        self.home_base_material = unsigned_first_count_projection(&entries);
        self.home_base_material_entries = entries;
    }

    pub fn home_base_production(&self) -> &HashMap<DefinitionId, u32> {
        &self.home_base_production
    }

    pub fn home_base_production_entries(&self) -> &[(DefinitionId, i32)] {
        &self.home_base_production_entries
    }

    pub fn set_home_base_production(&mut self, production: HashMap<DefinitionId, u32>) {
        self.set_home_base_production_entries(ordered_entries_from_unsigned_map(&production));
    }

    pub fn set_home_base_production_entries(&mut self, entries: Vec<(DefinitionId, i32)>) {
        self.home_base_production = unsigned_first_count_projection(&entries);
        self.home_base_production_entries = entries;
    }

    pub fn adjust_home_base_material(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        let current = ordered_id_count(&self.home_base_material_entries, &definition_id, 0);
        let updated = current.wrapping_add(delta);
        set_ordered_id_count(
            &mut self.home_base_material_entries,
            definition_id,
            updated,
            true,
        );
        self.home_base_material = unsigned_first_count_projection(&self.home_base_material_entries);
        u32::try_from(updated).unwrap_or(0)
    }

    pub fn adjust_home_base_production(&mut self, definition_id: DefinitionId, delta: i32) -> u32 {
        let current = ordered_id_count(&self.home_base_production_entries, &definition_id, 0);
        let updated = current.wrapping_add(delta);
        set_ordered_id_count(
            &mut self.home_base_production_entries,
            definition_id,
            updated,
            true,
        );
        self.home_base_production =
            unsigned_first_count_projection(&self.home_base_production_entries);
        u32::try_from(updated).unwrap_or(0)
    }

    pub fn advance_home_base_production(&mut self) -> bool {
        self.advance_home_base_production_as_leader(true)
    }

    pub(crate) fn advance_home_base_production_as_leader(&mut self, is_team_leader: bool) -> bool {
        self.production_delay = self.production_delay.saturating_add(1);
        if self.production_delay < 60 {
            return false;
        }
        // Team-homebase followers return after incrementing and retain their
        // >=60 delay; only the team's first still-present runtime player
        // produces/resets (C4Team::GetFirstActivePlayerID).
        if !is_team_leader {
            return false;
        }
        self.production_delay = 0;
        self.production_unit = self.production_unit.wrapping_add(1);
        let mut changed = false;
        let production = self.home_base_production_entries.clone();
        for (definition_id, count) in production {
            if count <= 0 {
                continue;
            }
            let frequency = (11_i32 - count).clamp(1, 10) as u32;
            if frequency == 0 {
                continue;
            }
            if self.production_unit % frequency == 0 {
                let current = ordered_id_count(&self.home_base_material_entries, &definition_id, 0);
                if current < MAX_HOME_BASE_MATERIAL as i32 {
                    set_ordered_id_count(
                        &mut self.home_base_material_entries,
                        definition_id,
                        current.saturating_add(1),
                        true,
                    );
                    changed = true;
                }
            }
        }
        if changed {
            self.home_base_material =
                unsigned_first_count_projection(&self.home_base_material_entries);
        }
        changed
    }

    pub fn sync_home_base_material_from(&mut self, other: &Player) {
        self.set_home_base_material_entries(other.home_base_material_entries.clone());
    }

    pub fn production_delay(&self) -> u32 {
        self.production_delay
    }

    pub fn set_production_delay(&mut self, delay: u32) {
        self.production_delay = delay;
    }

    pub fn production_unit(&self) -> u32 {
        self.production_unit
    }

    pub fn set_production_unit(&mut self, unit: u32) {
        self.production_unit = unit;
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn retained_runtime_flag_value(raw: i32, enabled: bool) -> i32 {
    if (raw != 0) == enabled {
        raw
    } else {
        i32::from(enabled)
    }
}

fn player_status_compiler_value(status: PlayerStatus) -> i32 {
    match status {
        PlayerStatus::Inactive => 0,
        PlayerStatus::Active | PlayerStatus::Eliminated | PlayerStatus::Surrendered => 1,
        PlayerStatus::TeamSelection => 2,
        PlayerStatus::TeamSelectionPending => 3,
    }
}

fn rgb_color_dw(color: RgbColor) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

fn exact_player_color_dw(raw: Option<u32>, color: Option<RgbColor>) -> u32 {
    match (raw, color) {
        (Some(raw), Some(color)) if raw & 0x00ff_ffff == rgb_color_dw(color) => raw,
        (_, Some(color)) => rgb_color_dw(color),
        (Some(raw), None) => raw,
        (None, None) => 0,
    }
}

fn is_zero_vector(value: &Vector2) -> bool {
    *value == Vector2::ZERO
}

#[derive(Debug, Clone)]
pub struct PlayerConfig {
    id: i32,
    player_info_id: i32,
    name: String,
    status: PlayerStatus,
    team: Option<i32>,
    surrendered: bool,
    wealth: i32,
    points: i32,
    score: i32,
    rounds: i32,
    rounds_won: i32,
    rounds_lost: i32,
    total_playing_time: i32,
    value: i32,
    initial_value: i32,
    value_gain: i32,
    objects_owned: u32,
    initial_value_set: bool,
    knowledge: Vec<DefinitionId>,
    magic: Vec<DefinitionId>,
    inventory: HashMap<DefinitionId, u32>,
    cursor: Option<ObjectId>,
    viewports: Vec<PlayerViewport>,
    home_base_material: HashMap<DefinitionId, u32>,
    home_base_production: HashMap<DefinitionId, u32>,
    production_delay: u32,
    production_unit: u32,
    color: Option<RgbColor>,
}

impl PlayerConfig {
    pub fn new(id: i32, name: impl Into<String>) -> Self {
        Self {
            id,
            player_info_id: 0,
            name: name.into(),
            status: PlayerStatus::Active,
            team: None,
            surrendered: false,
            wealth: 0,
            points: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            value: 0,
            initial_value: 0,
            value_gain: 0,
            objects_owned: 0,
            initial_value_set: false,
            knowledge: Vec::new(),
            magic: Vec::new(),
            inventory: HashMap::new(),
            cursor: None,
            viewports: Vec::new(),
            home_base_material: HashMap::new(),
            home_base_production: HashMap::new(),
            production_delay: 0,
            production_unit: 0,
            color: None,
        }
    }

    pub fn with_status(mut self, status: PlayerStatus) -> Self {
        self.status = status;
        self
    }

    pub fn with_player_info_id(mut self, player_info_id: i32) -> Self {
        self.player_info_id = player_info_id;
        self
    }

    pub fn with_team(mut self, team: Option<i32>) -> Self {
        self.team = team;
        self
    }

    pub fn with_surrendered(mut self, surrendered: bool) -> Self {
        self.surrendered = surrendered;
        self
    }

    pub fn with_wealth(mut self, wealth: i32) -> Self {
        self.wealth = wealth;
        self
    }

    pub fn with_points(mut self, points: i32) -> Self {
        self.points = points;
        self
    }

    pub fn with_score(mut self, score: i32) -> Self {
        self.score = score;
        self
    }

    pub fn with_rounds(mut self, rounds: i32, rounds_won: i32, rounds_lost: i32) -> Self {
        self.rounds = rounds;
        self.rounds_won = rounds_won;
        self.rounds_lost = rounds_lost;
        self
    }

    pub fn with_total_playing_time(mut self, total_playing_time: i32) -> Self {
        self.total_playing_time = total_playing_time;
        self
    }

    pub fn with_initial_value(mut self, value: i32) -> Self {
        self.initial_value = value;
        self.value = value;
        self.value_gain = 0;
        self.initial_value_set = true;
        self
    }

    pub fn with_objects_owned(mut self, objects_owned: u32) -> Self {
        self.objects_owned = objects_owned;
        self
    }

    pub fn with_knowledge<I>(mut self, knowledge: I) -> Self
    where
        I: IntoIterator<Item = DefinitionId>,
    {
        self.knowledge = knowledge.into_iter().collect();
        self
    }

    pub fn with_magic<I>(mut self, magic: I) -> Self
    where
        I: IntoIterator<Item = DefinitionId>,
    {
        self.magic = magic.into_iter().collect();
        self
    }

    pub fn with_inventory(mut self, inventory: HashMap<DefinitionId, u32>) -> Self {
        self.inventory = inventory;
        self
    }

    pub fn with_cursor(mut self, cursor: Option<ObjectId>) -> Self {
        self.cursor = cursor;
        self
    }

    pub fn with_viewports<I>(mut self, viewports: I) -> Self
    where
        I: IntoIterator<Item = PlayerViewport>,
    {
        self.viewports = viewports.into_iter().collect();
        self
    }

    pub fn with_home_base_material(mut self, material: HashMap<DefinitionId, u32>) -> Self {
        self.home_base_material = material;
        self
    }

    pub fn with_home_base_production(mut self, production: HashMap<DefinitionId, u32>) -> Self {
        self.home_base_production = production;
        self
    }

    pub fn with_production_delay(mut self, delay: u32) -> Self {
        self.production_delay = delay;
        self
    }

    pub fn with_production_unit(mut self, unit: u32) -> Self {
        self.production_unit = unit;
        self
    }

    pub fn with_color(mut self, color: Option<RgbColor>) -> Self {
        self.color = color;
        self
    }

    pub fn id(&self) -> i32 {
        self.id
    }

    pub(crate) fn player_info_id(&self) -> i32 {
        self.player_info_id
    }

    pub fn build(self) -> Player {
        Player::from_config(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_compiler_words_preserve_full_last_com_and_color_dw() {
        let state = PlayerState {
            id: 2,
            color: Some(RgbColor::new(0x12, 0x34, 0x56)),
            color_dw_raw: Some(0xff12_3456),
            control: PlayerControlState {
                last_com: -2_147_483_630,
                ..PlayerControlState::default()
            },
            ..PlayerState::default()
        };

        assert_eq!(state.exact_color_dw(), 0xff12_3456);
        let restored = Player::from_state(state.clone()).to_state();
        assert_eq!(restored, state);
        assert_eq!(restored.control.last_com, -2_147_483_630);

        let encoded = serde_json::to_string(&restored).expect("player state serializes");
        let decoded: PlayerState =
            serde_json::from_str(&encoded).expect("player state deserializes");
        assert_eq!(decoded.exact_color_dw(), 0xff12_3456);
        assert_eq!(decoded.control.last_com, -2_147_483_630);
    }

    #[test]
    fn legacy_rgb_projection_supersedes_a_stale_raw_color_companion() {
        let state = PlayerState {
            color: Some(RgbColor::new(0xab, 0xcd, 0xef)),
            color_dw_raw: Some(0xff12_3456),
            ..PlayerState::default()
        };

        assert_eq!(state.exact_color_dw(), 0x00ab_cdef);
        let mut player = Player::from_state(state);
        player.set_color(Some(RgbColor::new(1, 2, 3)));
        let updated = player.to_state();
        assert_eq!(updated.color_dw_raw, None);
        assert_eq!(updated.exact_color_dw(), 0x0001_0203);
    }

    #[test]
    fn set_fog_of_war_forces_override_and_persists_both_flags() {
        // C4Player::SetFoW (C4Player.cpp:815-824) always forces the setting,
        // including when disabling fog of war; CompileFunc persists both flags
        // as FogOfWar and ForceFogOfWar (C4Player.cpp:1580-1581).
        let mut player = Player::new(1, "Player");
        assert!(!player.fog_of_war());
        assert!(!player.force_fog_of_war());
        let configured = PlayerConfig::new(2, "Configured").build();
        assert!(!configured.fog_of_war());
        assert!(!configured.force_fog_of_war());

        player.set_fog_of_war(true);
        let enabled = player.to_state();
        assert!(enabled.fog_of_war);
        assert!(enabled.force_fog_of_war);

        let mut restored = Player::from_state(enabled);
        restored.set_fog_of_war(false);
        let disabled = restored.to_state();
        assert!(!disabled.fog_of_war);
        assert!(disabled.force_fog_of_war);
    }

    #[test]
    fn persisted_fog_of_war_flags_default_to_false_when_absent() {
        let state: PlayerState = serde_json::from_str(r#"{"id":2}"#)
            .unwrap_or_else(|error| panic!("valid legacy player state: {error}"));

        assert!(!state.fog_of_war);
        assert!(!state.force_fog_of_war);
    }

    #[test]
    fn evaluation_fields_default_when_absent_and_round_trip() {
        // C4Player persists its distinct PlayerInfo ID and Evaluated flag
        // (C4Player.cpp:1567-1571), while Score and TotalPlayingTime belong
        // to C4PlayerInfoCore (C4InfoCore.cpp:156-160). Keep all four distinct
        // from the in-round player number and Points fields.
        let legacy: PlayerState = serde_json::from_str(r#"{"id":2,"points":17}"#)
            .unwrap_or_else(|error| panic!("valid legacy player state: {error}"));
        assert_eq!(legacy.player_info_id, 0);
        assert!(!legacy.won);
        assert!(!legacy.evaluated);
        assert_eq!(legacy.score, 0);
        assert_eq!(legacy.total_playing_time, 0);

        let evaluated = PlayerState {
            id: 2,
            player_info_id: 41,
            won: true,
            evaluated: true,
            score: 250,
            total_playing_time: 1_234,
            view_cursor: Some(ObjectId::new(7)),
            captain: Some(ObjectId::new(8)),
            ..PlayerState::default()
        };
        assert_eq!(Player::from_state(evaluated.clone()).to_state(), evaluated);
    }

    #[test]
    fn ordered_player_id_lists_preserve_signed_counts_duplicates_and_first_match_mutation() {
        let state = PlayerState {
            id: 2,
            knowledge_entries: vec![("DUPL".into(), 0), ("DUPL".into(), 7)],
            magic_entries: vec![("FIRE".into(), -3), ("WIND".into(), 0)],
            home_base_material_entries: vec![("ZINC".into(), -5), ("BRIK".into(), 0)],
            home_base_production_entries: vec![("ROCK".into(), 11), ("ROCK".into(), 2)],
            hostility_entries: vec![(4, 0), (4, 9), (2, -1)],
            ..PlayerState::default()
        };
        let json = serde_json::to_string(&state).expect("ordered player state serializes");
        let decoded: PlayerState =
            serde_json::from_str(&json).expect("ordered player state deserializes");
        let mut player = Player::from_state(decoded);
        let restored = player.to_state();

        assert_eq!(restored.knowledge_entries, state.knowledge_entries);
        assert_eq!(restored.magic_entries, state.magic_entries);
        assert_eq!(
            restored.home_base_material_entries,
            state.home_base_material_entries
        );
        assert_eq!(
            restored.home_base_production_entries,
            state.home_base_production_entries
        );
        assert_eq!(restored.hostility_entries, state.hostility_entries);
        assert!(!player.is_hostile_towards(3));
        assert!(player.is_hostile_towards(1));

        player.grant_knowledge("DUPL".into());
        assert_eq!(
            player.knowledge_entries(),
            &[("DUPL".into(), 1), ("DUPL".into(), 7)]
        );
        player.revoke_knowledge(&"DUPL".into());
        assert_eq!(player.knowledge_entries(), &[("DUPL".into(), 7)]);

        player.set_hostile_towards(8, false);
        assert_eq!(player.hostility_entries().last(), Some(&(9, 0)));
        assert!(!player.is_hostile_towards(8));
    }

    #[test]
    fn home_base_adjustments_keep_raw_counts_while_automatic_production_caps_at_25() {
        let mut player = Player::new(1, "Player");

        assert_eq!(player.adjust_home_base_material("MAT".into(), 100), 100);
        assert_eq!(player.adjust_home_base_material("MAT".into(), -105), 0);
        assert_eq!(player.adjust_home_base_material("MZERO".into(), 0), 0);
        assert_eq!(
            player.home_base_material_entries(),
            &[("MAT".into(), -5), ("MZERO".into(), 0)]
        );

        assert_eq!(player.adjust_home_base_production("PROD".into(), 100), 100);
        assert_eq!(player.adjust_home_base_production("PROD".into(), -105), 0);
        assert_eq!(player.adjust_home_base_production("PZERO".into(), 0), 0);
        assert_eq!(
            player.home_base_production_entries(),
            &[("PROD".into(), -5), ("PZERO".into(), 0)]
        );

        player.set_home_base_material_entries(vec![("AUTO".into(), 24)]);
        player.set_home_base_production_entries(vec![("AUTO".into(), 10)]);
        player.set_production_delay(59);
        assert!(player.advance_home_base_production());
        assert_eq!(player.home_base_material_entries(), &[("AUTO".into(), 25)]);

        player.set_production_delay(59);
        assert!(!player.advance_home_base_production());
        assert_eq!(player.home_base_material_entries(), &[("AUTO".into(), 25)]);
    }

    #[test]
    fn legacy_player_list_projections_migrate_to_ordered_backing() {
        let legacy: PlayerState = serde_json::from_str(
            r#"{
                "id": 3,
                "knowledge": ["PLAN", "BRIK"],
                "magic": ["WIND", "FIRE"],
                "home_base_material": {"ZINC": 0, "BRIK": 4},
                "home_base_production": {"ROCK": 2},
                "hostility": [7, 1]
            }"#,
        )
        .expect("legacy player state decodes");
        let migrated = Player::from_state(legacy).to_state();

        assert_eq!(
            migrated.knowledge_entries,
            vec![("PLAN".into(), 1), ("BRIK".into(), 1)]
        );
        assert_eq!(
            migrated.magic_entries,
            vec![("WIND".into(), 1), ("FIRE".into(), 1)]
        );
        assert_eq!(
            migrated.home_base_material_entries,
            vec![("BRIK".into(), 4), ("ZINC".into(), 0)]
        );
        assert_eq!(
            migrated.home_base_production_entries,
            vec![("ROCK".into(), 2)]
        );
        assert_eq!(migrated.hostility_entries, vec![(8, 1), (2, 1)]);
    }

    #[test]
    fn default_evaluation_fields_do_not_change_serialized_player_shape() {
        let value = serde_json::to_value(PlayerState {
            id: 2,
            ..PlayerState::default()
        })
        .unwrap_or_else(|error| panic!("player state serializes: {error}"));

        for field in [
            "player_info_id",
            "won",
            "evaluated",
            "score",
            "total_playing_time",
            "view_mode",
            "view_cursor",
            "captain",
            "view_target",
            "message_board_queries",
        ] {
            assert!(
                value.get(field).is_none(),
                "unexpected default field {field}"
            );
        }
    }

    #[test]
    fn message_board_query_save_keeps_only_the_head_without_the_answered_runtime_bit() {
        let mut state = PlayerState {
            id: 2,
            message_board_queries: vec![
                MessageBoardQuery {
                    target: Some(ObjectId::new(7)),
                    prompt: "Name?".to_string(),
                    uppercase: true,
                    answered: true,
                },
                MessageBoardQuery {
                    target: Some(ObjectId::new(8)),
                    prompt: "Dropped tail".to_string(),
                    answered: true,
                    ..MessageBoardQuery::default()
                },
            ],
            ..PlayerState::default()
        };

        state.prepare_for_save();
        assert_eq!(state.message_board_queries.len(), 1);
        assert_eq!(state.message_board_queries[0].prompt, "Name?");
        assert!(!state.message_board_queries[0].answered);

        let encoded = serde_json::to_value(&state)
            .unwrap_or_else(|error| panic!("message-board query serializes: {error}"));
        let query = &encoded["message_board_queries"][0];
        assert_eq!(query["target"], serde_json::json!(7));
        assert_eq!(query["prompt"], "Name?");
        assert_eq!(query["uppercase"], true);
        assert!(query.get("answered").is_none());

        let restored: PlayerState = serde_json::from_value(encoded)
            .unwrap_or_else(|error| panic!("message-board query restores: {error}"));
        assert!(!restored.message_board_queries[0].answered);
    }

    #[test]
    fn player_client_ownership_defaults_unknown_and_round_trips() {
        // C4Player::DefaultRuntimeData uses C4ClientIDUnknown (-1), and
        // CompileFunc persists AtClient with that exact default
        // (pristine 9ffa0a5d src/C4Player.cpp:1556-1563,1718-1724;
        // src/C4Client.h:25-28).
        let legacy: PlayerState = serde_json::from_str(r#"{"id":2}"#)
            .unwrap_or_else(|error| panic!("legacy player state decodes: {error}"));
        assert_eq!(legacy.at_client, PlayerAtClient::UNKNOWN);
        assert!(
            serde_json::to_value(&legacy)
                .expect("legacy state serializes")
                .get("at_client")
                .is_none(),
            "unknown ownership is the omitted default"
        );

        let owned = PlayerState {
            id: 2,
            at_client: PlayerAtClient::new(7),
            ..PlayerState::default()
        };
        assert_eq!(Player::from_state(owned.clone()).to_state(), owned);
        assert_eq!(
            serde_json::to_value(&owned)
                .expect("owned state serializes")
                .get("at_client")
                .and_then(serde_json::Value::as_i64),
            Some(7)
        );

        let networked = PlayerState {
            id: 3,
            at_client: PlayerAtClient::new(8),
            at_client_name: Some(String::new()),
            color_index: Some(4),
            position_index: Some(2),
            ..PlayerState::default()
        };
        assert_eq!(Player::from_state(networked.clone()).to_state(), networked);

        let mut bounded = Player::new(4, "Player");
        bounded.set_at_client_name(format!("{}é", "x".repeat(511)));
        assert_eq!(
            clonk_script::c4_string_bytes(bounded.at_client_name()),
            [vec![b'x'; 511], vec![0xc3]].concat()
        );
        let restored = Player::from_state(PlayerState {
            at_client_name: Some("x".repeat(513)),
            ..PlayerState::default()
        });
        assert_eq!(
            clonk_script::c4_string_byte_len(restored.at_client_name()),
            512
        );
    }

    #[test]
    fn native_player_strings_survive_state_serialization() {
        let state = PlayerState {
            name: clonk_script::c4_string_from_bytes(b"Andr\xe9"),
            at_client_name: Some(clonk_script::c4_string_from_bytes(b"M\xfcnchen")),
            message_buf: clonk_script::c4_string_from_bytes(b"Gr\xfc\xdfe"),
            message_board_queries: vec![MessageBoardQuery {
                prompt: clonk_script::c4_string_from_bytes(b"W\xe4hlen"),
                ..MessageBoardQuery::default()
            }],
            ..PlayerState::default()
        };

        let encoded = serde_json::to_value(&state).expect("serialize player state");
        let restored: PlayerState =
            serde_json::from_value(encoded).expect("deserialize player state");
        assert_eq!(clonk_script::c4_string_bytes(&restored.name), b"Andr\xe9");
        assert_eq!(
            restored
                .at_client_name
                .as_deref()
                .map(clonk_script::c4_string_bytes),
            Some(b"M\xfcnchen".to_vec())
        );
        assert_eq!(
            clonk_script::c4_string_bytes(&restored.message_buf),
            b"Gr\xfc\xdfe"
        );
        assert_eq!(
            clonk_script::c4_string_bytes(&restored.message_board_queries[0].prompt),
            b"W\xe4hlen"
        );
    }

    #[test]
    fn player_config_carries_profile_values_but_not_the_join_clock() {
        // C4Player::Init copies the linked C4PlayerInfo ID before loading the
        // C4PlayerInfoCore profile values (C4Player.cpp:246-276). The local
        // GameJoinTime baseline is runtime-only and starts at zero until Init
        // assigns Game.Time (C4Player.cpp:389-390,1075).
        let player = PlayerConfig::new(2, "Profile")
            .with_player_info_id(41)
            .with_score(250)
            .with_rounds(11, 7, 4)
            .with_total_playing_time(1_234)
            .build();

        let state = player.to_state();
        assert_eq!(state.player_info_id, 41);
        assert_eq!(state.score, 250);
        assert_eq!((state.rounds, state.rounds_won, state.rounds_lost), (11, 7, 4));
        assert_eq!(state.total_playing_time, 1_234);
        assert_eq!(player.game_join_time(), 0);

        let restored = Player::from_state(state);
        assert_eq!(restored.game_join_time(), 0);
    }

    #[test]
    fn runtime_scalar_defaults_and_state_round_trip_match_cpp_lifecycle() {
        let legacy: PlayerState = serde_json::from_str(r#"{"id":2}"#)
            .unwrap_or_else(|error| panic!("legacy player state decodes: {error}"));
        assert_eq!(legacy.control_set, 0);
        assert_eq!(legacy.mouse_control, 0);
        assert!(!legacy.show_startup);
        assert_eq!(legacy.captain, None);
        assert_eq!(legacy.message_buf, "");

        let restored_legacy = Player::from_state(legacy);
        assert_eq!(restored_legacy.control_set(), 0);
        assert!(!restored_legacy.show_startup());

        let fresh = Player::new(3, "Fresh");
        assert_eq!(fresh.control_set(), -1);
        assert!(fresh.show_startup());

        let state = PlayerState {
            id: 4,
            captain: Some(ObjectId::new(9)),
            view_wealth: 3,
            view_value: 4,
            crew_created: 5,
            control_set: 2,
            mouse_control: 1,
            pref_control: 6,
            pref_mouse: Some(false),
            pref_control_style: true,
            pref_auto_context_menu: true,
            show_startup: true,
            select_count: 6,
            message_status: 7,
            message_buf: "hello".to_string(),
            ..PlayerState::default()
        };
        assert_eq!(Player::from_state(state.clone()).to_state(), state);
    }

    #[test]
    fn runtime_scalar_setters_bound_messages_and_clear_captain() {
        let captain = ObjectId::new(9);
        let mut player = Player::new(1, "Player");
        player.set_runtime_control(3, 1);
        player.set_select_count(4);
        player.set_captain(Some(captain));
        player.set_crew_created(8);
        assert_eq!(player.increment_crew_created(), 9);
        player.set_message_buf(format!("{}é", "x".repeat(255)));

        assert_eq!(player.control_set(), 3);
        assert_eq!(player.mouse_control(), 1);
        assert_eq!(player.select_count(), 4);
        assert_eq!(player.captain(), Some(captain));
        assert_eq!(player.crew_created(), 9);
        assert_eq!(
            clonk_script::c4_string_bytes(player.message_buf()),
            [vec![b'x'; 255], vec![0xc3]].concat()
        );

        player.clear_object_pointers(captain);
        assert_eq!(player.captain(), None);
    }

    #[test]
    fn live_save_denumeration_clears_only_explicit_off_list_pointer_fields() {
        let listed = ObjectId::new(7);
        let missing = ObjectId::new(8);
        let view_target = ObjectId::new(9);
        let mut player = Player::from_state(PlayerState {
            cursor: Some(missing),
            view_cursor: Some(listed),
            captain: Some(missing),
            view_target: Some(view_target),
            crew: vec![missing],
            viewports: vec![PlayerViewport::new(Vector2::ZERO).with_focus(Some(missing))],
            message_board_queries: vec![
                MessageBoardQuery::new(Some(missing), "missing".into(), false),
                MessageBoardQuery::new(Some(listed), "listed".into(), false),
            ],
            extra_data: vec![("object".into(), clonk_script::Value::Object(missing.as_u64()))],
            ..PlayerState::default()
        });
        player.set_view_target(Some(view_target));

        player.denumerate_live_save_pointer_fields(&HashSet::from([listed.as_u64()]));
        let state = player.to_state();

        assert_eq!(state.cursor, None);
        assert_eq!(state.view_cursor, Some(listed));
        assert_eq!(state.captain, None);
        assert_eq!(state.view_target, Some(view_target));
        assert_eq!(state.crew, [missing]);
        assert_eq!(
            state
                .message_board_queries
                .iter()
                .map(|query| query.target)
                .collect::<Vec<_>>(),
            [None, Some(listed)]
        );
        assert_eq!(
            state.extra_data,
            [("object".into(), clonk_script::Value::Object(missing.as_u64()))]
        );
        assert_eq!(state.viewports[0].focus, Some(listed));
    }

    #[test]
    fn saved_view_center_restores_without_a_presentation_viewport() {
        let saved = PlayerState {
            id: 4,
            view_center: Some(Vector2::new(321, -17)),
            viewports: Vec::new(),
            ..PlayerState::default()
        };

        let mut player = Player::from_state(saved.clone());
        assert_eq!(player.view_center(), Vector2::new(321, -17));
        assert!(player.viewports().is_empty());
        player.update_view(None);
        assert_eq!(player.to_state(), saved);

        // Old Rust snapshots had only the presentation projection. Keep that
        // migration fallback without making it authoritative for new saves.
        let legacy = PlayerState {
            id: 5,
            viewports: vec![PlayerViewport::new(Vector2::new(44, 55))],
            ..PlayerState::default()
        };
        let legacy = Player::from_state(legacy);
        assert_eq!(legacy.view_center(), Vector2::new(44, 55));
        assert_eq!(legacy.to_state().view_center, None);
    }

    #[test]
    fn scroll_view_enters_scrolling_mode_clears_target_and_clamps_all_viewports() {
        let mut player = Player::new(1, "Player");
        player.set_view_target(Some(ObjectId::new(9)));
        player.replace_viewports(vec![
            PlayerViewport::new(Vector2::new(15, 995)),
            PlayerViewport::new(Vector2::new(500, 500)),
        ]);

        player.scroll_view(Vector2::new(-10, 10), 100, 80, 1_000, 1_000, true);

        let state = player.to_state();
        assert_eq!(state.view_mode, PLAYER_VIEW_MODE_SCROLLING);
        assert_eq!(state.view_target, None);
        assert_eq!(state.viewports[0].center, Vector2::new(10, 1_000));
        assert_eq!(state.viewports[1].center, Vector2::new(490, 510));

        // The ordinary view update receives no resolved object while scrolling
        // and therefore preserves both the mode and free-scroll centers.
        player.update_view(None);
        assert_eq!(player.to_state(), state);
    }

    #[test]
    fn scroll_view_uses_windowed_bounds_and_cpp_inverted_bound_order() {
        let mut player = Player::new(1, "Player");
        player.replace_viewports(vec![PlayerViewport::new(Vector2::new(45, 965))]);
        player.scroll_view(Vector2::new(-10, 10), 100, 80, 1_000, 1_000, false);
        assert_eq!(player.viewports()[0].center, Vector2::new(50, 960));

        player.replace_viewports(vec![PlayerViewport::new(Vector2::new(0, 0))]);
        player.scroll_view(Vector2::new(0, 0), 100, 80, 20, 10, false);
        assert_eq!(
            player.viewports()[0].center,
            Vector2::new(50, 40),
            "C++ BoundBy returns the lower bound first when bounds invert"
        );
    }

    #[test]
    fn wealth_value_and_message_delays_follow_active_player_frames() {
        let mut player = Player::new(1, "Player");
        player.adjust_wealth(0);
        player.adjust_points(0);
        player.set_message_status(2);
        assert_eq!(player.view_wealth(), PLAYER_VIEW_DELAY);
        assert_eq!(player.view_value(), PLAYER_VIEW_DELAY);

        player.advance_runtime_delays();
        assert_eq!(player.message_status(), 1);
        assert_eq!(player.view_wealth(), PLAYER_VIEW_DELAY - 1);
        assert_eq!(player.view_value(), PLAYER_VIEW_DELAY - 1);

        player.set_status(PlayerStatus::Inactive);
        player.advance_runtime_delays();
        assert_eq!(player.message_status(), 1);
        assert_eq!(player.view_wealth(), PLAYER_VIEW_DELAY - 1);
        assert_eq!(player.view_value(), PLAYER_VIEW_DELAY - 1);
    }

    #[test]
    fn asset_updates_compare_cached_gain_and_count_without_resetting_baseline() {
        let mut player = Player::new(1, "Player");
        player.update_asset_value(50, 0);
        assert_eq!(player.initial_value(), 0);
        assert!(!player.to_state().initial_value_set);
        assert_eq!(player.value_gain(), 50);
        assert_eq!(player.view_value(), PLAYER_VIEW_DELAY);

        player.reset_initial_value();
        assert_eq!(
            player.value_gain(),
            50,
            "InitialValue assignment does not rewrite the cached ValueGain"
        );
        player.set_view_value(0);
        player.update_asset_value(50, 0);
        assert_eq!(player.view_value(), PLAYER_VIEW_DELAY);

        player.update_asset_value(50, 1);
        assert_eq!(player.view_value(), PLAYER_VIEW_DELAY);
    }
}
