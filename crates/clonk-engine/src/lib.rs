#![allow(dead_code, unreachable_patterns, unused_variables)]
#![allow(
    clippy::doc_lazy_continuation,
    clippy::field_reassign_with_default,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::match_like_matches_macro,
    clippy::needless_range_loop,
    clippy::question_mark,
    clippy::should_implement_trait,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::vec_init_then_push
)]

#[doc(hidden)]
pub mod action;
mod chunky;
#[doc(hidden)]
pub mod command;
#[doc(hidden)]
pub mod compat;
mod control;
mod control_execution;
mod direct_com;
#[doc(hidden)]
pub mod effect;
pub mod fixtures;
mod init_placement;
mod input;
#[doc(hidden)]
pub mod landscape;
mod live_c4_player;
mod live_c4_save;
mod map_creator;
mod map_creator_s2;
mod mass_mover;
#[doc(hidden)]
pub mod material;
#[doc(hidden)]
pub use clonk_engine_core::math;
mod message;
mod native_function_parameters;
mod network_game_data;
pub mod ocf;
#[cfg(test)]
mod parity_differential;
pub mod particles;
mod pathfinder;
mod player;
pub mod player_file;
use player_file::PlayerInfoCoreState;
pub mod pxs;
mod record;
mod runtime_join_player_restore;
#[doc(hidden)]
pub use clonk_engine_core::rng;
mod round_results;
pub mod scenario;
mod scoreboard;
mod script_constants;
#[doc(hidden)]
pub mod sector;
mod sky;
#[cfg(test)]
mod test_game_call_ex;
pub mod text_spec;
#[doc(hidden)]
pub mod transfer;

pub use action::{
    ActionLibrary, ActionProcedure, ActionSpec, ActionState, ActionUpdate, ActionUpdateResult,
};
use action::{ScriptCallbackLink, ScriptCallbackTarget, SharedActionLibrary};
pub use command::{CommandStackSnapshot, MenuRequest, MenuRequestKind};
#[doc(hidden)]
pub use compat::BlastReplay;
pub use control::{
    append_control_packet_ini, encode_control_packet_ini, interpret_player_control_command,
    parse_control_ini, parse_replay_player_infos_ini, ActivateGameGoalMenuControlData,
    ActivateGameGoalRuleControlData, ClientCoreControlData, ClientJoinControlData,
    ClientRemoveControlData, ClientUpdateControlData, CommandKind, ControlButton, ControlCommand,
    ControlEvent, ControlIniEncodeError, ControlIniPacketMode, ControlPacket, ControlPacketId,
    ControlParseError, ControlPlayerInfoEntry, CustomCommandControlData, DebugRecordControlData,
    EliminatePlayerControlData, EmDrawToolControlData, EmDropDefControlData,
    EmMoveObjectControlData, InitScenarioPlayerControlData, JoinPlayerControlData,
    JoinPlayerSource, LegacyCString, MessageBoardAnswerControlData, MessageControlData,
    NetworkResourceCore, PlayerCommandControlData, PlayerControlData, PlayerInfoControlData,
    PlayerInfoUpdateRequest, PlayerSelectControlData, RemovePlayerControlData,
    ReplayPlayerInfosDocument, ScriptControlData, ScriptStrictness, SetControlData,
    SetPlayerTeamControlData, SurrenderPlayerControlData, SyncCheckPacket, SynchronizeControlData,
    ToggleHostilityControlData, VoteControlData, C4MN_ADJUST_POSITION,
    CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS, CLIENT_PLAYER_INFO_FLAG_INITIAL,
    CLIENT_PLAYER_INFO_FLAG_UPDATED, CLIENT_UPDATE_ACTIVATE, CLIENT_UPDATE_SET_OBSERVER, COM_CHAT,
    COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG,
    COM_DOUBLE, COM_DOWN, COM_HELP, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN, COM_MENU_ENTER,
    COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SELECT, COM_MENU_SHOW_TEXT,
    COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL,
    COM_SPECIAL2, COM_THROW, COM_UP, COM_WHEEL_DOWN, COM_WHEEL_UP, EMDT_BRUSH, EMDT_FILL,
    EMDT_LINE, EMDT_RECT, EMDT_SET_MODE, EMMO_DUPLICATE, EMMO_ENTER, EMMO_EXIT, EMMO_MOVE,
    EMMO_REMOVE, EMMO_SCRIPT, MESSAGE_TYPE_ALERT, MESSAGE_TYPE_ME, MESSAGE_TYPE_NORMAL,
    MESSAGE_TYPE_PRIVATE, MESSAGE_TYPE_SAY, MESSAGE_TYPE_SOUND, MESSAGE_TYPE_SYSTEM,
    MESSAGE_TYPE_TEAM, NETWORK_RESOURCE_TYPE_NULL, PLAYER_INFO_FLAG_ATTRIBUTES_FIXED,
    PLAYER_INFO_FLAG_DISCONNECTED, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE,
    PLAYER_INFO_FLAG_IN_SCENARIO_FILE, PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_JOIN_ISSUED,
    PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK, PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
    PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_FLAG_SAVEGAME_JOIN, PLAYER_INFO_FLAG_VOTED_OUT,
    PLAYER_INFO_FLAG_WON, PLAYER_INFO_TYPE_NONE, PLAYER_INFO_TYPE_SCRIPT, PLAYER_INFO_TYPE_USER,
    SCRIPT_SCOPE_CONSOLE, SCRIPT_SCOPE_GLOBAL, SET_VALUE_CONTROL_RATE, SET_VALUE_DISABLE_DEBUG,
    SET_VALUE_FAIR_CREW, SET_VALUE_MAX_PLAYER, SET_VALUE_NONE, SET_VALUE_TEAM_COLORS,
    SET_VALUE_TEAM_DISTRIBUTION, VOTE_TYPE_CANCEL, VOTE_TYPE_KICK, VOTE_TYPE_NONE, VOTE_TYPE_PAUSE,
};
pub use control_execution::{
    assign_initial_host_player_teams, assign_initial_offline_player_teams,
    generate_default_initial_team, prepare_join_player_config, resolve_remote_embedded_player_data,
    resolve_remote_embedded_player_data_with_engine, AdmittedPlayerTeamUpdate,
    ControlClientRegistry, ControlClientState, ControlPlayerInfoRegistry,
    InitialHostTeamAssignmentOracle, JoinPlayerPreparation, PlayerInfoAdmission,
    PrepareJoinPlayerError, RemoteEmbeddedPlayerData, ResolveRemoteEmbeddedPlayerDataError,
    TeamColorUpdateError,
};
pub use direct_com::MouseWorldCursor;
pub use effect::{EffectState, EffectVarValue};
pub use input::PlayerInputState;
pub use landscape::{
    BlastResult, CollisionResolution, Landscape, LandscapeCommand, LandscapeError,
    LandscapePersistenceError, LiquidColumn, LiquidSegment, LANDSCAPE_MODE_DYNAMIC,
    LANDSCAPE_MODE_EXACT, LANDSCAPE_MODE_STATIC, LANDSCAPE_MODE_UNDEFINED,
};
pub use live_c4_player::{
    serialize_aggressively_stripped_c4_player, serialize_live_c4_player,
    serialize_live_c4_player_for_synchronization, serialize_live_c4_player_from_state,
    serialize_live_c4_player_state, serialize_live_c4_player_with_options,
    serialize_live_c4_player_with_options_and_enumeration,
    strip_unresolved_remote_crew_for_synchronization, LiveC4CrewProfileCleanup, LiveC4PlayerError,
    LiveC4PlayerSaveOptions, LiveC4SynchronizedPlayerGroup,
};
pub use live_c4_save::{
    LiveC4ComponentHost, LiveC4SaveComponentMutation, LiveC4SaveComponentRef, LiveC4SaveComponents,
    LiveC4SaveEntry, LiveC4SaveEntryKind, LiveC4SaveError, LiveC4SaveLandscapeMutation,
    LiveC4SaveNamedComponent, LiveC4SavePlayerPolicy, LiveC4SavePolicy,
    LiveC4SavePreLandscapeComponents, LiveC4SaveScenarioSectionMutation, LiveC4SaveSpec,
    LiveC4ValueEncodeError, LiveC4ValueEnumeration,
};
pub use material::{Material, MaterialId, MaterialSet};
pub use message::{
    MessageKind, MessageSnapshot, FLAG_ALIGN_CENTER, FLAG_ALIGN_LEFT, FLAG_ALIGN_RIGHT,
    FLAG_BOTTOM, FLAG_HCENTER, FLAG_LEFT, FLAG_NO_BREAK, FLAG_RIGHT, FLAG_TOP, FLAG_VCENTER,
    FLAG_WIDTH_REL, FLAG_X_REL, FLAG_Y_REL,
};
pub use network_game_data::{
    parse_initial_network_game_data, parse_landscape_game_data, serialize_initial_network_game,
    InitialNetworkCompiledSections, InitialNetworkGameApplyError, InitialNetworkGameData,
    InitialNetworkGameError, InitialNetworkMessageBoardCommand, LandscapeGameData,
    MessageBoardCommandRestriction, UnsupportedInitialNetworkGameState,
    INITIAL_NETWORK_DEFAULT_SYNC_RATE, LANDSCAPE_DEFAULT_GRAVITY_RAW,
};
pub use pathfinder::{
    PathFinder, PathWaypoint, PathfinderDebugRay, PathfinderDebugRayStatus,
    PathfinderDebugSnapshot, PathfinderDebugZone,
};
pub use player::{
    ActiveMessageBoardInput, MessageBoardQuery, Player, PlayerAtClient, PlayerConfig,
    PlayerControlState, PlayerState, PlayerStatus, PlayerViewport, PLAYER_VIEW_MODE_CURSOR,
    PLAYER_VIEW_MODE_SCROLLING, PLAYER_VIEW_MODE_TARGET,
};
pub use record::{
    BinaryControlRecord, Playback, PlaybackError, Recorder, Recording, RCT_CTRL, RCT_CTRL_PKT,
    RCT_END, RCT_FRAME,
};
pub use round_results::{
    LeagueRoundResultUpdate, RoundResultsNetworkResult, RoundResultsPlayerState,
    RoundResultsPlayerStatus, RoundResultsState,
};
pub use runtime_join_player_restore::{
    RestoredRuntimeJoinPlayer, RuntimeJoinPlayerRestoreError, RuntimeJoinPlayerSource,
};
pub use scenario::{
    GameParameterRuleGoalLists, InitialNetworkScenarioMetadata, InitialNetworkTeam,
    InitialNetworkTeamDistribution, InitialNetworkTeamMetadata, LegacyC4SVal, PlayerStart,
    ReplayScenarioStartupPreflight, Scenario, ScenarioDefinitionSelectionSource, ScenarioError,
    ScenarioFairCrewForce, ScenarioGameParameterOverrides, ScenarioGameParameterResolution,
    ScenarioGameParameterValues, ScenarioIdListEntry, ScenarioLoaderMetadata,
    ScenarioLoaderSelection, ScenarioLobbyClient, ScenarioLobbyDefinitions, ScenarioLobbyHead,
    ScenarioLobbyIdEntry, ScenarioLobbyMetadata, ScenarioLobbyTeam, ScenarioLobbyTeams,
    ScenarioObjectives, ScenarioSavegameDefinitionOverride, ScenarioTeamColor,
    ScenarioTeamDistribution, ScenarioTeamsSource, SkyConfig, MAX_PLAYER_STARTS,
};
use scoreboard::ScoreboardPresentationSink;
pub use scoreboard::{
    ScoreboardCell, ScoreboardPresentationRequest, ScoreboardState, SCOREBOARD_CAPTION,
};
pub use sky::{SkyFrame, SkyParallaxMode, SkySettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommandKind {
    Focus,
    DropAll,
}

#[cfg(test)]
#[path = "lib_tests/command_definition_snapshot_cache_regression.rs"]
mod command_definition_snapshot_cache_regression;

#[cfg(test)]
#[path = "lib_tests/signed_action_direction_regression.rs"]
mod signed_action_direction_regression;

#[cfg(test)]
#[path = "lib_tests/stale_phase_call_regression.rs"]
mod stale_phase_call_regression;

impl MenuCommandKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            MenuCommandKind::Focus => "focus",
            MenuCommandKind::DropAll => "drop_all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuCommandSelection {
    pub primary_id: ObjectId,
    pub instances: Vec<ObjectId>,
    pub definition_id: DefinitionId,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuEntry {
    pub function: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptContextFunction {
    pub function: String,
    pub label: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub image_phase: i32,
    pub condition: Option<String>,
    pub has_description: bool,
}

fn script_context_function_metadata(function: &clonk_script::Function) -> ScriptContextFunction {
    let mut label = String::new();
    let mut image = None;
    let mut image_phase = 0;
    let mut condition = None;
    let mut description = None;
    if let Some(raw) = function.description.as_deref() {
        let mut segments = raw.split('|');
        label = segments.next().unwrap_or_default().trim().to_owned();
        for segment in segments {
            let Some((key, value)) = segment.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "Image" => {
                    let (identifier, phase) = value
                        .split_once(':')
                        .map_or((value, None), |(identifier, phase)| {
                            (identifier, Some(phase))
                        });
                    image = Some(identifier.to_owned());
                    image_phase = phase.and_then(|phase| phase.parse().ok()).unwrap_or(0);
                }
                "Condition" => condition = Some(value.to_owned()),
                "Desc" => description = (!value.is_empty()).then(|| value.to_owned()),
                _ => {}
            }
        }
    }
    ScriptContextFunction {
        function: function.name.clone(),
        label,
        description,
        image,
        image_phase,
        condition,
        has_description: function
            .description
            .as_deref()
            .is_some_and(|description| !description.is_empty()),
    }
}

use command::{
    definition_id_to_c4id, AcquireScriptResult, CallResultAction, CommandData,
    CommandDefinitionSnapshot, CommandEvent, CommandEventInstanceKind, CommandFailureFeedback,
    CommandId, CommandMode, CommandObjectSnapshot, CommandOperation, CommandPlayerSnapshot,
    CommandRequest, CommandRuntimeContext, CommandStack, CommandStepResult, GetAttemptDisposition,
};
use compat::{
    enter_audio_context, enter_environment_context, enter_physics_context, enter_random_context,
    object_reference_value, AudioRegistry, BlastPixelReplay, DefinitionMetadata,
    EffectContextOutcome, EnvironmentDelta, HostSolidMaskImage, HostSolidMaskMetadata,
    HostWorldContext, HostWorldObject, LandscapeOperation, LazyHostWorldProvider,
    NextMissionCommand, ObjectOrderCommand, ObjectOrderFunction, PhysicsDelta, PlayerCommand,
};
use effect::{EffectCommand, EffectEvent, EffectEventKind, EffectStopReason};
use material::{
    evaluate_corrosion, MaterialInteractionEvent, MaterialReaction, MaterialReactionKind,
};
use message::{MessageCommand, MessageManager, MessageSpec, PersistedMessage};
use ocf::NORMAL as OCF_NORMAL;
use sector::{SectorMap, SectorObject};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::AddAssign;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

#[cfg(test)]
std::thread_local! {
    static HOST_WORLD_OBJECT_MATERIALIZATIONS: Cell<usize> = const { Cell::new(0) };
    static HOST_WORLD_LANDSCAPE_MATERIALIZATIONS: Cell<usize> = const { Cell::new(0) };
    static SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS: Cell<usize> = const { Cell::new(0) };
}

use crate::math::{
    fixed10, fixed100, fixed256, fixtoi, fixtoi_prec, itofix, itofix_prec, C4Fixed, FixedVec2,
};
pub use crate::rng::LcgRng;
use clonk_resources::definition::{
    ActionFacet as ResourceActionFacet, TargetRect as ResourceTargetRect,
};
pub use clonk_resources::definition::{APS_COLOR, APS_GRAPHICS, APS_NAME, APS_OVERLAY};
pub use clonk_resources::PhysicalInfo;
use clonk_resources::{
    ActionDefinition as ResourceActionDefinition, PictureRect as ResourcePictureRect,
    RankNameTable, ResourceDefinition as ResourceDefinitionData, C4_MAX_PHYSICAL,
};
pub use clonk_script::ScriptError;

use clonk_script::{DebuggerHooks, Engine as ScriptEngine, Value, ValueMap};
use mass_mover::MassMoverSet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sky::{SkyAdjustment, SkyState};
use thiserror::Error;
use transfer::{TransferZoneCommand, TransferZoneRect, TransferZoneState, TransferZoneTable};

pub type DefinitionId = String;

pub const OWNER_NONE: i32 = -1;
/// C4Game::OpenGame's normal application timer before SetGameSpeed changes it.
pub const DEFAULT_GAME_TICK_DELAY_MS: u64 = 28;

pub(crate) fn next_game_tick_delay_revision() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
    NEXT_REVISION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("game tick timer revision space exhausted")
}

pub const FULL_CON: i32 = 100_000;
pub const VIS_ALL: i32 = 0;
pub const VIS_NONE: i32 = 1;
pub const VIS_OWNER: i32 = 2;
pub const VIS_ALLIES: i32 = 4;
pub const VIS_ENEMIES: i32 = 8;
pub const VIS_LOCAL: i32 = 16;
pub const VIS_GOD: i32 = 32;
pub const VIS_LAYER_TOGGLE: i32 = 64;
pub const VIS_OVERLAY_ONLY: i32 = 128;

/// `C4DefCore::HideBar` bits (src/C4Def.h:278-284).
pub const HIDE_HUD_BAR_ENERGY: i32 = 0x01;
pub const HIDE_HUD_BAR_MAGIC_ENERGY: i32 = 0x02;
pub const HIDE_HUD_BAR_BREATH: i32 = 0x04;
/// `C4DefCore::HideHud` bits (src/C4Def.h:286-295).
pub const HIDE_HUD_ELEMENT_PORTRAIT: i32 = 0x01;
pub const HIDE_HUD_ELEMENT_CAPTAIN: i32 = 0x02;
pub const HIDE_HUD_ELEMENT_NAME: i32 = 0x04;
pub const HIDE_HUD_ELEMENT_RANK: i32 = 0x08;
pub const HIDE_HUD_ELEMENT_RANK_IMAGE: i32 = 0x10;
pub const HIDE_HUD_ELEMENT_INVENTORY: i32 = 0x20;

/// `BoundBy(value, 0, maximum)` as used by `C4Object::DoEnergy`.
///
/// The C++ helper compares the lower bound first and does not normalize an
/// inverted range, so keep the branches explicit instead of using `clamp`.
pub(crate) fn bound_energy(value: i32, maximum: i32) -> i32 {
    if value < 0 {
        0
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

/// Active process-global `LoadResStr` entries used by
/// C4Object::GetNeededMatStr. Headless engines default to the shipped US
/// text; the app overwrites these from its frozen installed language table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NeededMaterialStrings {
    need: String,
    none: String,
}

impl NeededMaterialStrings {
    fn new(need: impl Into<String>, none: impl Into<String>) -> Self {
        Self {
            need: need.into(),
            none: none.into(),
        }
    }

    fn format_need(&self, object_name: &str) -> String {
        self.need.replacen("%s", object_name, 1)
    }

    fn format_none(&self, object_name: &str) -> String {
        self.none.replacen("%s", object_name, 1)
    }
}

impl Default for NeededMaterialStrings {
    fn default() -> Self {
        Self::new("%s|needs", "%s needs|no more material.")
    }
}

/// `0xff000000 | Pal.GetClr(FColors[FRed])`: the C4.PAL red used by every
/// ConstructionCheck feedback message (C4GameMessage.cpp:280-282;
/// C4Surface.cpp:1304; StdColors.h:32).
pub(crate) const CONSTRUCTION_CHECK_MESSAGE_COLOR: u32 = 0xfff4_0000;

/// Active process-global `LoadResStr` entries used by ConstructionCheck's
/// red failure feedback (C4Landscape.cpp:2131-2163). Headless engines
/// default to the shipped US text; the app overwrites these from its frozen
/// installed language table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConstructionCheckStrings {
    /// IDS_OBJ_UNDEF — takes the requested C4ID text.
    pub(crate) undefined: String,
    /// IDS_OBJ_NOCON — takes the definition name.
    pub(crate) no_construction: String,
    /// IDS_OBJ_NOROOM.
    pub(crate) no_room: String,
    /// IDS_OBJ_NOLEVEL.
    pub(crate) no_level: String,
    /// IDS_OBJ_NOOTHER — takes the blocking object's name.
    pub(crate) no_other: String,
}

impl ConstructionCheckStrings {
    fn new(
        undefined: impl Into<String>,
        no_construction: impl Into<String>,
        no_room: impl Into<String>,
        no_level: impl Into<String>,
        no_other: impl Into<String>,
    ) -> Self {
        Self {
            undefined: undefined.into(),
            no_construction: no_construction.into(),
            no_room: no_room.into(),
            no_level: no_level.into(),
            no_other: no_other.into(),
        }
    }

    pub(crate) fn format_undefined(&self, id_text: &str) -> String {
        self.undefined.replacen("%s", id_text, 1)
    }

    pub(crate) fn format_not_constructable(&self, definition_name: &str) -> String {
        self.no_construction.replacen("%s", definition_name, 1)
    }

    pub(crate) fn format_blocked(&self, blocker_name: &str) -> String {
        self.no_other.replacen("%s", blocker_name, 1)
    }
}

impl Default for ConstructionCheckStrings {
    fn default() -> Self {
        Self::new(
            "Structure %s undefined.",
            "%s cannot|be built.",
            "Not enough room!",
            "No level ground!",
            "%s is in the way.",
        )
    }
}

fn us_default_rank_names() -> Vec<String> {
    [
        "Clonk",
        "Ensign",
        "Lieutenant",
        "Captain",
        "Major",
        "Lieutenant Colonel",
        "Colonel",
        "Brigade General",
        "Major General",
        "Lieutenant General",
        "General",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// The `C4Object::DoCon` gate for its expensive mass/face/component refresh
/// (C4Object.cpp:1439-1447). Construction still changes between percent
/// boundaries, but the component list only follows these refresh points.
pub(crate) fn docon_refreshes_construction(before: i32, after: i32) -> bool {
    let step_size = FULL_CON / 100;
    before / step_size != after / step_size || after >= FULL_CON || after == 0 || before >= FULL_CON
}

/// `ComponentConCutoff` / `ComponentConGain` (C4Object.cpp:510-526).
/// C++ iterates the object's existing component entries; it never inserts a
/// definition component that the object no longer carries.
pub(crate) fn docon_component_counts(
    current: &HashMap<DefinitionId, i32>,
    order: &[DefinitionId],
    definition: &[(DefinitionId, i32)],
    construction: i32,
    change: i32,
) -> HashMap<DefinitionId, i32> {
    let mut updated = current.clone();
    for (index, id) in order.iter().enumerate() {
        if let Some(count) = current.get(id) {
            // C4Object::ComponentConGain/Cutoff index Def->Component by the
            // object's C4IDList position, not by ID (C4Object.cpp:510-526).
            let definition_count = definition.get(index).map_or(0, |(_, count)| *count);
            let scaled = scaled_definition_component_count(definition_count, construction);
            let count = if change < 0 {
                (*count).min(scaled)
            } else {
                (*count).max(scaled)
            };
            updated.insert(id.clone(), count);
        }
    }
    updated
}

fn normalized_component_order(
    components: &HashMap<DefinitionId, i32>,
    order: Vec<DefinitionId>,
    definition_order: &[DefinitionId],
) -> Vec<DefinitionId> {
    // C4IDList is a vector and can contain duplicate IDs (the shipped
    // Bazooka DefCore contains ENAP twice). Preserve an explicit order
    // verbatim; old map-only states recover the definition's vector order.
    let source = if order.is_empty() {
        definition_order.to_vec()
    } else {
        order
    };
    let mut normalized = source
        .into_iter()
        .filter(|id| components.contains_key(id))
        .collect::<Vec<_>>();
    let mut extras = components
        .keys()
        .filter(|id| !normalized.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    extras.sort();
    normalized.extend(extras);
    normalized
}

fn definition_component_counts(
    definition: &[(DefinitionId, i32)],
    construction: i32,
) -> HashMap<DefinitionId, i32> {
    let mut counts = HashMap::new();
    for (id, count) in definition {
        counts
            .entry(id.clone())
            .or_insert_with(|| fresh_definition_component_count(*count, construction));
    }
    counts
}

/// Component state around `C4Game::NewObject`'s initial construction pass.
/// At Con=0, scripts observe Init's raw copy after ComponentConCutoff; at a
/// nonzero initial Con, the following ComponentConGain has already produced
/// the signed, scaled count (C4Object.cpp:197-199,510-526;
/// C4Game.cpp:1129-1142).
fn fresh_definition_component_count(count: i32, construction: i32) -> i32 {
    let construction = construction.max(0);
    let after_init_cutoff = count.min(0);
    if construction == 0 || !docon_refreshes_construction(0, construction) {
        after_init_cutoff
    } else {
        after_init_cutoff.max(scaled_definition_component_count(count, construction))
    }
}

fn scaled_definition_component_count(count: i32, construction: i32) -> i32 {
    let product = i64::from(count) * i64::from(construction.max(0));
    (product / i64::from(FULL_CON)) as i32
}

/// Energy-loss cause types (C4Effects.h:59-67), passed to Fx*Damage.
/// Damage cause types (C4Effects.h:53-56).
pub const C4FX_CALL_DMG_SCRIPT: i32 = 0;
pub const C4FX_CALL_DMG_BLAST: i32 = 1;
pub const C4FX_CALL_DMG_FIRE: i32 = 2;
pub const C4FX_CALL_DMG_CHOP: i32 = 3;
/// The engine-internal fire effect (C4Effects.h:152-157).
pub const C4FX_FIRE: &str = "Fire";
pub const C4FX_FIRE_PRIORITY: i32 = 100;
pub const C4FX_FIRE_TIMER_INTERVAL: i32 = 1;
pub const MAX_FIRE_PHASE: i32 = 15;
/// Fire appearance modes (C4Effects.h:70-74).
pub const C4FX_FIRE_MODE_STRUCT_VEH: i32 = 1;
pub const C4FX_FIRE_MODE_LIVING_VEG: i32 = 2;
pub const C4FX_FIRE_MODE_OBJECT: i32 = 3;
pub const C4FX_CALL_ENG_SCRIPT: i32 = 32;
pub const C4FX_CALL_ENG_BLAST: i32 = 33;
pub const C4FX_CALL_ENG_OBJ_HIT: i32 = 34;
pub const C4FX_CALL_ENG_FIRE: i32 = 35;
pub const C4FX_CALL_ENG_BASE_REFRESH: i32 = 36;
pub const C4FX_CALL_ENG_ASPHYXIATION: i32 = 37;
pub const C4FX_CALL_ENG_CORROSION: i32 = 38;
pub const C4FX_CALL_ENG_STRUCT: i32 = 39;
pub const C4FX_CALL_ENG_GET_PUNCHED: i32 = 40;
const GAME_OVER_CHECK_INTERVAL: u8 = 35;
#[doc(hidden)]
pub const FIRE_DEFINITION_ID: &str = "FLAM";
/// The bubble object BubbleOut creates (C4Effect.cpp:847-857).
const BUBBLE_DEFINITION_ID: &str = "FXU1";
/// `Config.Graphics.SmokeLevel` default (C4Config.cpp:452).
pub const DEFAULT_SMOKE_LEVEL: i32 = 200;
/// `CClrModAddMap::iDefResolutionX/Y` and the C4Scenario FoWRes default.
pub const DEFAULT_FOW_RESOLUTION: i32 = 64;
/// `GetSmokeLevel` while `C4GameControl::SyncMode` is active.
const SYNC_SMOKE_LEVEL: i32 = 150;

fn bubble_cap_reached(bubble_count: usize, smoke_level: i32) -> bool {
    let Ok(smoke_level) = usize::try_from(smoke_level) else {
        // C++ compares the nonnegative ObjectCount directly with the signed
        // config value, so zero and negative limits reject every creation.
        return true;
    };
    bubble_count >= smoke_level
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphicsOverlayMode {
    None = 0,
    Base = 1,
    Action = 2,
    Picture = 3,
    IngamePicture = 4,
    Object = 5,
    ExtraGraphics = 6,
}

impl GraphicsOverlayMode {
    pub fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(GraphicsOverlayMode::None),
            1 => Some(GraphicsOverlayMode::Base),
            2 => Some(GraphicsOverlayMode::Action),
            3 => Some(GraphicsOverlayMode::Picture),
            4 => Some(GraphicsOverlayMode::IngamePicture),
            5 => Some(GraphicsOverlayMode::Object),
            6 => Some(GraphicsOverlayMode::ExtraGraphics),
            _ => None,
        }
    }
}

fn draw_transform_component_is_zero(component: &f32) -> bool {
    *component == 0.0
}

fn draw_transform_component_is_one(component: &f32) -> bool {
    *component == 1.0
}

fn draw_transform_homogeneous_default() -> f32 {
    1.0
}

fn draw_transform_flip_dir_default() -> i32 {
    1
}

fn draw_transform_flip_dir_is_default(flip_dir: &i32) -> bool {
    *flip_dir == 1
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrawTransform {
    pub scale_x: f32,
    pub scale_y: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    #[serde(default, skip_serializing_if = "draw_transform_component_is_zero")]
    shear_x: f32,
    #[serde(default, skip_serializing_if = "draw_transform_component_is_zero")]
    shear_y: f32,
    #[serde(default, skip_serializing_if = "draw_transform_component_is_zero")]
    projective_x: f32,
    #[serde(default, skip_serializing_if = "draw_transform_component_is_zero")]
    projective_y: f32,
    #[serde(
        default = "draw_transform_homogeneous_default",
        skip_serializing_if = "draw_transform_component_is_one"
    )]
    homogeneous: f32,
    /// C4DrawTransform::FlipDir. The stored matrix already contains the
    /// corresponding sign in mat[0], exactly like the native type.
    #[serde(
        default = "draw_transform_flip_dir_default",
        skip_serializing_if = "draw_transform_flip_dir_is_default"
    )]
    flip_dir: i32,
}

impl DrawTransform {
    pub fn identity() -> Self {
        Self::from_matrix([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
    }

    pub fn is_identity(&self) -> bool {
        self.matrix() == Self::identity().matrix() && self.flip_dir == 1
    }

    pub fn from_components(scale_x: f32, scale_y: f32, offset_x: f32, offset_y: f32) -> Self {
        Self::from_matrix([
            scale_x, 0.0, offset_x, 0.0, scale_y, offset_y, 0.0, 0.0, 1.0,
        ])
    }

    pub fn from_matrix(matrix: [f32; 9]) -> Self {
        Self::from_matrix_with_flip_dir(matrix, 1)
    }

    /// Restores the already-flipped native matrix and its independent
    /// FlipDir field from Objects.txt.
    pub fn from_matrix_with_flip_dir(matrix: [f32; 9], flip_dir: i32) -> Self {
        Self {
            scale_x: matrix[0],
            shear_x: matrix[1],
            offset_x: matrix[2],
            shear_y: matrix[3],
            scale_y: matrix[4],
            offset_y: matrix[5],
            projective_x: matrix[6],
            projective_y: matrix[7],
            homogeneous: matrix[8],
            flip_dir,
        }
    }

    pub fn flip_dir(&self) -> i32 {
        self.flip_dir
    }

    /// C4DrawTransform::SetFlipDir: changing the logical flip also toggles
    /// the x-axis matrix component already consumed by the renderer.
    pub fn with_flip_dir(mut self, flip_dir: i32) -> Self {
        if self.flip_dir != flip_dir {
            self.flip_dir = flip_dir;
            self.scale_x = -self.scale_x;
        }
        self
    }

    pub fn matrix(&self) -> [f32; 9] {
        [
            self.scale_x,
            self.shear_x,
            self.offset_x,
            self.shear_y,
            self.scale_y,
            self.offset_y,
            self.projective_x,
            self.projective_y,
            self.homogeneous,
        ]
    }

    /// Matches `C4DrawTransform::operator*=`: the supplied delta is applied
    /// before the current transform, so this returns `delta * self`.
    pub fn combined(self, delta: Self) -> Self {
        let matrix = self.matrix();
        let rhs = delta.matrix();

        Self::from_matrix_with_flip_dir(
            [
                matrix[0] * rhs[0] + matrix[3] * rhs[1] + matrix[6] * rhs[2],
                matrix[1] * rhs[0] + matrix[4] * rhs[1] + matrix[7] * rhs[2],
                matrix[2] * rhs[0] + matrix[5] * rhs[1] + matrix[8] * rhs[2],
                matrix[0] * rhs[3] + matrix[3] * rhs[4] + matrix[6] * rhs[5],
                matrix[1] * rhs[3] + matrix[4] * rhs[4] + matrix[7] * rhs[5],
                matrix[2] * rhs[3] + matrix[5] * rhs[4] + matrix[8] * rhs[5],
                matrix[0] * rhs[6] + matrix[3] * rhs[7] + matrix[6] * rhs[8],
                matrix[1] * rhs[6] + matrix[4] * rhs[7] + matrix[7] * rhs[8],
                matrix[2] * rhs[6] + matrix[5] * rhs[7] + matrix[8] * rhs[8],
            ],
            self.flip_dir,
        )
    }
}

#[cfg(test)]
#[path = "lib_tests/draw_transform_tests.rs"]
mod draw_transform_tests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectGraphicsOverlay {
    pub id: i32,
    pub mode: GraphicsOverlayMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DefinitionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default)]
    pub phase: i32,
    #[serde(default)]
    pub blit_mode: u32,
    #[serde(default)]
    pub color_modulation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_object: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<DrawTransform>,
}

impl ObjectGraphicsOverlay {
    pub fn new(id: i32, mode: GraphicsOverlayMode) -> Self {
        Self {
            id,
            mode,
            definition: None,
            graphics_name: None,
            action: None,
            phase: 0,
            blit_mode: 0,
            color_modulation: 0x00ff_ffff,
            overlay_object: None,
            transform: None,
        }
    }

    pub fn with_definition(mut self, definition: Option<DefinitionId>) -> Self {
        self.definition = definition;
        self
    }

    pub fn with_graphics_name(mut self, name: Option<String>) -> Self {
        self.graphics_name = name;
        self
    }

    pub fn with_action(mut self, action: Option<String>) -> Self {
        self.action = action;
        self
    }

    pub fn with_blit_mode(mut self, blit_mode: u32) -> Self {
        self.blit_mode = blit_mode;
        self
    }

    pub fn with_overlay_object(mut self, overlay_object: Option<ObjectId>) -> Self {
        self.overlay_object = overlay_object;
        self
    }

    pub fn with_transform(mut self, transform: Option<DrawTransform>) -> Self {
        self.transform = transform;
        self
    }
}

/// Whether two valid, resolved graphics references name the same native
/// `C4DefGraphics` pointer. Named graphics use C++ `SEqualNoCase` over their
/// legacy bytes; an absent/empty name denotes the definition's default
/// graphics. An unresolved serialized name does not model a native pointer and
/// is outside this comparison's input contract.
pub(crate) fn resolved_graphics_equal(
    left_definition: Option<&str>,
    left_name: Option<&str>,
    right_definition: Option<&str>,
    right_name: Option<&str>,
) -> bool {
    let (Some(left_definition), Some(right_definition)) = (left_definition, right_definition)
    else {
        // A null pSourceGfx compares by pointer value; no serialized name can
        // make two null references different.
        return left_definition.is_none() && right_definition.is_none();
    };
    if left_definition != right_definition {
        return false;
    }
    match (
        left_name.filter(|name| !name.is_empty()),
        right_name.filter(|name| !name.is_empty()),
    ) {
        (None, None) => true,
        (Some(left), Some(right)) => clonk_resources::material::c4_names_equal(left, right),
        _ => false,
    }
}

/// `C4GraphicsOverlay::operator==` fields used by
/// `C4Object::CanConcatPictureWith`. Animation phase and overlay ID are
/// deliberately handled outside this comparison. Native overlays always
/// own an identity transform, whereas the Rust snapshot omits that default.
pub(crate) fn picture_overlays_equal(
    left: &ObjectGraphicsOverlay,
    right: &ObjectGraphicsOverlay,
) -> bool {
    let optional_string_equal = |left: Option<&str>, right: Option<&str>| {
        left.filter(|value| !value.is_empty()) == right.filter(|value| !value.is_empty())
    };
    left.mode == right.mode
        && resolved_graphics_equal(
            left.definition.as_deref(),
            left.graphics_name.as_deref(),
            right.definition.as_deref(),
            right.graphics_name.as_deref(),
        )
        && optional_string_equal(left.action.as_deref(), right.action.as_deref())
        && left.blit_mode == right.blit_mode
        && left.color_modulation == right.color_modulation
        && left.transform.unwrap_or_else(DrawTransform::identity)
            == right.transform.unwrap_or_else(DrawTransform::identity)
        && left.overlay_object == right.overlay_object
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectBaseGraphics {
    pub definition: DefinitionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_name: Option<String>,
    #[serde(default)]
    pub blit_mode: u32,
}

pub const CNAT_NONE: u32 = 0;
pub const CNAT_LEFT: u32 = 1;
pub const CNAT_RIGHT: u32 = 2;
pub const CNAT_TOP: u32 = 4;
pub const CNAT_BOTTOM: u32 = 8;
pub const CNAT_CENTER: u32 = 16;
pub const CNAT_MULTI_ATTACH: u32 = 32;
pub const CNAT_NO_COLLISION: u32 = 64;
const CNAT_FLAGS: u32 = CNAT_MULTI_ATTACH | CNAT_NO_COLLISION;
#[doc(hidden)]
pub const C4D_BORDER_SIDES: i32 = 1;
#[doc(hidden)]
pub const C4D_BORDER_TOP: i32 = 2;
#[doc(hidden)]
pub const C4D_BORDER_BOTTOM: i32 = 4;
#[doc(hidden)]
pub const C4D_BORDER_LAYER: i32 = 8;

/// Native `CollectionLimit && ObjectCount() >= CollectionLimit` with the
/// nonnegative list count narrowed to C++'s signed comparison domain.
pub(crate) fn collection_limit_reached(limit: i32, contents_count: usize) -> bool {
    limit != 0 && i32::try_from(contents_count).unwrap_or(i32::MAX) >= limit
}
const CONTACT_DENSITY_SOLID: i32 = 50;
const C4M_VEHICLE: i32 = 100;
/// `C4M_Solid` / `C4M_SemiSolid` (C4Material.h:201-202): the GBackSolid
/// and GBackSemiSolid density thresholds.
const C4M_SOLID: i32 = 50;
const C4M_SEMI_SOLID: i32 = 25;
const ATTACH_RANGE: i32 = 5;
const FIX_FULL_CIRCLE: i32 = 360;
const FIX_HALF_CIRCLE: i32 = 180;

pub const CATEGORY_STATIC_BACK: i32 = 1 << 0;
pub const CATEGORY_STRUCTURE: i32 = 1 << 1;
pub const CATEGORY_VEHICLE: i32 = 1 << 2;
pub const CATEGORY_LIVING: i32 = 1 << 3;
pub const CATEGORY_OBJECT: i32 = 1 << 4;
#[doc(hidden)]
pub const CATEGORY_GOAL: i32 = 1 << 5;
#[doc(hidden)]
pub const CATEGORY_SELECT_KNOWLEDGE: i32 = 1 << 10;
pub const CATEGORY_MAGIC: i32 = 1 << 17;
pub const CATEGORY_PARALLAX: i32 = 1 << 21;
pub const CATEGORY_MOUSE_SELECT: i32 = 1 << 22;
/// Fallback assigned by `C4Def::Load` when DefCore `Version` is older than
/// 4.0 or omitted (src/C4Def.cpp:573-581).
pub const DEFAULT_DEFINITION_VERSION: [i32; 5] = [4, 9, 10, 7, 0];

pub(crate) fn minimum_con_activation_denied(category: i32, construction: i32) -> bool {
    category & (CATEGORY_VEHICLE | CATEGORY_OBJECT) != 0
        && category & CATEGORY_SELECT_KNOWLEDGE != 0
        && construction < FULL_CON
}

fn definition_version_at_least(version: [i32; 5], required: [i32; 4]) -> bool {
    (version[0], version[1], version[2], version[3])
        >= (required[0], required[1], required[2], required[3])
}
pub const CATEGORY_SORT_LIMIT: i32 = CATEGORY_STATIC_BACK
    | CATEGORY_STRUCTURE
    | CATEGORY_VEHICLE
    | CATEGORY_LIVING
    | CATEGORY_OBJECT;
pub const DEFAULT_CATEGORY: i32 = CATEGORY_STATIC_BACK;

/// The two object classes which can enter C4MouseControl's moving drag.
/// Carryable has cursor priority over Grab=1 when a definition has both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseDragSource {
    Carryable,
    Vehicle,
}

/// Presentation result of `C4MouseControl::DragMoving` for a carryable.
/// A throw retains the direction and `ShowPoint` landing coordinate selected
/// by the first successful `FindThrowingPosition` probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseDragCarryableCursor {
    Drop,
    Throw { direction: i32, landing: Vector2 },
}

/// `C4D_Grab_Put` / `C4D_Grab_Get` (C4Def.h:80-81): the GrabPutGet
/// DefCore bits feeding OCF_Container (SetOCF, C4Object.cpp:658-660).
pub const GRAB_PUT_GET_PUT: i32 = 1;
pub const GRAB_PUT_GET_GET: i32 = 2;

/// `C4D_VehicleControl_Outside` / `C4D_VehicleControl_Inside`
/// (C4Def.h:111-113): the SetCommand ControlCommand overloads
/// (C4Object.cpp:3944-3969).
pub const VEHICLE_CONTROL_OUTSIDE: i32 = 1;
pub const VEHICLE_CONTROL_INSIDE: i32 = 2;

pub const LINE_CONNECT_POWER_INPUT: u32 = 1;
pub const LINE_CONNECT_POWER_OUTPUT: u32 = 1 << 1;
pub const LINE_CONNECT_LIQUID_INPUT: u32 = 1 << 2;
pub const LINE_CONNECT_LIQUID_OUTPUT: u32 = 1 << 3;
pub const LINE_CONNECT_POWER_GENERATOR: u32 = 1 << 4;
pub const LINE_CONNECT_POWER_CONSUMER: u32 = 1 << 5;
pub const LINE_CONNECT_LIQUID_PUMP: u32 = 1 << 6;
pub const LINE_CONNECT_CONNECT_ROPE: u32 = 1 << 7;
pub const LINE_CONNECT_ENERGY_HOLDER: u32 = 1 << 8;

fn default_rng() -> LcgRng {
    LcgRng::default()
}

fn compute_blast_size(radius: i32) -> i64 {
    let r = i64::from(radius.max(0));
    (r * r * 6283) / 2000
}

fn compute_blast_grade(radius: i32) -> i64 {
    let level = radius.max(0);
    let raw = (level / 10) - 1;
    i64::from(raw.clamp(1, 3))
}

pub(crate) fn normalize_category(raw: i32, fallback: i32) -> i32 {
    let sort_bits = raw & CATEGORY_SORT_LIMIT;
    if sort_bits != 0 {
        raw
    } else {
        let fallback_bits = fallback & CATEGORY_SORT_LIMIT;
        let replacement = if fallback_bits != 0 {
            fallback_bits
        } else {
            CATEGORY_STATIC_BACK
        };
        (raw & !CATEGORY_SORT_LIMIT) | replacement
    }
}

pub(crate) fn default_category() -> i32 {
    DEFAULT_CATEGORY
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectId(u64);

impl ObjectId {
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ObjectStatus {
    Deleted,
    #[default]
    Normal,
    Inactive,
}

impl ObjectStatus {
    pub const fn is_active(self) -> bool {
        matches!(self, ObjectStatus::Normal)
    }

    pub const fn to_script_value(self) -> i32 {
        match self {
            ObjectStatus::Deleted => 0,
            ObjectStatus::Normal => 1,
            ObjectStatus::Inactive => 2,
        }
    }

    pub fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(ObjectStatus::Deleted),
            1 => Some(ObjectStatus::Normal),
            2 => Some(ObjectStatus::Inactive),
            _ => None,
        }
    }
}

/// C4Action::Dir: the raw direction index for the current action.
///
/// `DIR_Left` and `DIR_Right` are the first two conventional values, but
/// multi-directional ActMaps use the full signed int32 domain in saved object
/// state (C4Action.cpp:45-54).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Direction(i32);

impl Direction {
    #[allow(non_upper_case_globals)]
    pub const Left: Self = Self(0);
    #[allow(non_upper_case_globals)]
    pub const Right: Self = Self(1);

    pub const fn from_raw(value: i32) -> Self {
        Self(value)
    }

    pub const fn to_script_value(self) -> i32 {
        self.0
    }

    pub const fn from_script_value(value: i32) -> Self {
        Self(value)
    }
}

impl Serialize for Direction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(self.to_script_value())
    }
}

impl<'de> Deserialize<'de> for Direction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = i32::deserialize(deserializer)?;
        Ok(Direction::from_raw(raw))
    }
}

/// C4Action::ComDir storage.
///
/// The named `COMD_*` values form the engine's directional ring, but C++
/// compiles and scripts the backing `int32_t` verbatim (C4Action.cpp:45-54;
/// C4Script.cpp:792-796). Values outside the ring therefore remain distinct
/// from `COMD_Stop` across loaded and snapshotted action state.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct CommandDirection(i32);

impl CommandDirection {
    #[allow(non_upper_case_globals)]
    pub const Stop: Self = Self(0);
    #[allow(non_upper_case_globals)]
    pub const Up: Self = Self(1);
    #[allow(non_upper_case_globals)]
    pub const UpRight: Self = Self(2);
    #[allow(non_upper_case_globals)]
    pub const Right: Self = Self(3);
    #[allow(non_upper_case_globals)]
    pub const DownRight: Self = Self(4);
    #[allow(non_upper_case_globals)]
    pub const Down: Self = Self(5);
    #[allow(non_upper_case_globals)]
    pub const DownLeft: Self = Self(6);
    #[allow(non_upper_case_globals)]
    pub const Left: Self = Self(7);
    #[allow(non_upper_case_globals)]
    pub const UpLeft: Self = Self(8);

    pub const fn from_raw(value: i32) -> Self {
        Self(value)
    }

    pub const fn to_script_value(self) -> i32 {
        self.0
    }

    pub const fn from_script_value(value: i32) -> Option<Self> {
        match value {
            0..=8 => Some(Self(value)),
            _ => None,
        }
    }

    pub const fn axis_components(self) -> (i32, i32) {
        match self {
            CommandDirection::Stop => (0, 0),
            CommandDirection::Up => (0, -1),
            CommandDirection::UpRight => (1, -1),
            CommandDirection::Right => (1, 0),
            CommandDirection::DownRight => (1, 1),
            CommandDirection::Down => (0, 1),
            CommandDirection::DownLeft => (-1, 1),
            CommandDirection::Left => (-1, 0),
            CommandDirection::UpLeft => (-1, -1),
            _ => (0, 0),
        }
    }
}

impl Serialize for CommandDirection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(self.to_script_value())
    }
}

impl<'de> Deserialize<'de> for CommandDirection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = i32::deserialize(deserializer)?;
        Ok(CommandDirection::from_raw(raw))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// The nine independent C4GraphicsSystem gamma controls, each with the
/// black/mid/white curve points stored as raw 0xRRGGBB values
/// (`C4MaxGammaRamps`, C4Constants.h:45-46; C4GraphicsSystem.h:51).
pub const GAMMA_RAMP_COUNT: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaControlState {
    pub ramps: [[u32; 3]; GAMMA_RAMP_COUNT],
}

impl GammaControlState {
    pub const RAMP_COUNT: usize = GAMMA_RAMP_COUNT;
    pub const DEFAULT_RAMP: [u32; 3] = [0x000000, 0x808080, 0xffffff];

    pub const fn new() -> Self {
        Self {
            ramps: [Self::DEFAULT_RAMP; Self::RAMP_COUNT],
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// `C4GraphicsSystem::SetGamma` index gate and slot write
    /// (C4GraphicsSystem.cpp:772-784). Returns false for the silent invalid
    /// index path.
    pub fn set_ramp(&mut self, index: i32, points: [u32; 3]) -> bool {
        let Ok(index) = usize::try_from(index) else {
            return false;
        };
        let Some(ramp) = self.ramps.get_mut(index) else {
            return false;
        };
        *ramp = points;
        true
    }

    pub fn ramp(&self, index: usize) -> Option<[u32; 3]> {
        self.ramps.get(index).copied()
    }

    /// `C4GraphicsSystem::ApplyGamma` adds every control's per-channel
    /// displacement from the default curve, then clamps the three combined
    /// points (`C4GraphicsSystem.cpp:787-809`).
    pub fn combined_control_points(&self) -> [u32; 3] {
        const DEFAULT_CHANNEL: [i32; 3] = [0x00, 0x80, 0xff];
        let mut combined = [0_u32; 3];
        for (point, output) in combined.iter_mut().enumerate() {
            let default = DEFAULT_CHANNEL[point];
            let mut channels = [default; 3];
            for (channel, value) in channels.iter_mut().enumerate() {
                let shift = 16 - channel * 8;
                for ramp in &self.ramps {
                    let component = ((ramp[point] >> shift) & 0xff) as i32;
                    *value += component - default;
                }
                *value = (*value).clamp(0, 255);
            }
            *output =
                ((channels[0] as u32) << 16) | ((channels[1] as u32) << 8) | channels[2] as u32;
        }
        combined
    }
}

impl Default for GammaControlState {
    fn default() -> Self {
        Self::new()
    }
}

/// What a joining player brings to `Engine::join_player` — the data
/// C4Player::Init/ScenarioInit reads from the C4PlayerInfo and the loaded
/// .c4p file (C4Player.cpp:246-352, 670-777).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinPlayerConfig {
    /// Player name (from C4PlayerInfo; falls back to the file core name).
    pub name: String,
    /// Existing unique `C4PlayerInfo::ID`. Allocation remains the host's
    /// responsibility (`C4PlayerInfo.cpp:781-799`).
    pub player_info_id: i32,
    /// Persistent C4PlayerInfoCore settlement score.
    pub score: i32,
    /// Persistent C4PlayerInfoCore completed-round counters.
    pub rounds: i32,
    pub rounds_won: i32,
    pub rounds_lost: i32,
    /// Persistent C4PlayerInfoCore total playing time in seconds.
    pub total_playing_time: i32,
    /// Team id (0/None when teamless).
    pub team: Option<i32>,
    /// Resolved 24-bit player color (`pInfo->GetColor()`,
    /// C4Player.cpp:692).
    pub color_dw: u32,
    /// Indexed color preference (`PrefColor`, C4Player.cpp:680-685).
    pub pref_color: i32,
    /// Start-position preference (`PrefPosition`, C4Player.cpp:717-732).
    pub pref_position: i32,
    /// The crew roster from the player file (C4Player::CrewInfoList).
    pub crew: Vec<player_file::CrewInfo>,
    /// `PrefControlStyle` (AutoStopControl): Jump'n'Run control when true
    /// (C4Player::InitControl, C4Player.cpp:2371-2380). The scenario-wide
    /// `ForcedControlStyle` is stored separately from this preference.
    pub control_style: bool,
    /// `PrefAutoContextMenu` after the player-file `-1` fallback. The
    /// scenario-wide ForcedAutoContextMenu override is stored separately
    /// (C4Player::ApplyForcedControl, C4Player.cpp:2369-2375).
    pub auto_context_menu: bool,
    /// `Game.Parameters.StartupPlayerCount` — gates the standard-position
    /// distribution (C4Player.cpp:719).
    pub startup_player_count: i32,
}

/// The client-local result of `C4Player::InitControl`, supplied by the
/// frontend before a player is registered so `PreInitializePlayer` and
/// `InitializePlayer` observe the final `Control`/`MouseControl` values.
///
/// These values are deliberately separate from [`JoinPlayerConfig`]: the
/// latter is synchronized player-file/game data, while control-set ownership
/// depends on the current process' input devices and already joined local
/// players (C4Player.cpp:1871-1918).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerRuntimeControl {
    pub control_set: i32,
    pub mouse_control: i32,
    pub preferred_control_set: i32,
    pub prefers_mouse: bool,
}

impl PlayerRuntimeControl {
    pub const NONE: Self = Self {
        control_set: -1,
        mouse_control: 0,
        preferred_control_set: 0,
        prefers_mouse: true,
    };

    pub const fn new(control_set: i32, mouse_control: i32) -> Self {
        Self {
            control_set,
            mouse_control,
            preferred_control_set: control_set,
            prefers_mouse: mouse_control != 0,
        }
    }

    pub const fn with_preferences(
        control_set: i32,
        mouse_control: i32,
        preferred_control_set: i32,
        prefers_mouse: bool,
    ) -> Self {
        Self {
            control_set,
            mouse_control,
            preferred_control_set,
            prefers_mouse,
        }
    }
}

impl Default for PlayerRuntimeControl {
    fn default() -> Self {
        Self::NONE
    }
}

/// Process-local display names for one configured control binding.
///
/// C++ obtains these from `Config.Controls`/`Config.Gamepads`; they are UI
/// configuration and therefore deliberately remain outside synchronized
/// engine state and savegame snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlKeyName {
    long: String,
    short: String,
}

impl ControlKeyName {
    pub fn new(long: impl Into<String>, short: impl Into<String>) -> Self {
        Self {
            long: long.into(),
            short: short.into(),
        }
    }

    fn display(&self, short: bool) -> &str {
        if short {
            &self.short
        } else {
            &self.long
        }
    }
}

/// One ordered entry from the scenario's `Teams.txt` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamInfo {
    pub id: i32,
    #[serde(with = "clonk_script::c4_string_serde")]
    pub name: String,
    pub color: u32,
    /// Ordered `C4Team::piPlayers` player-info IDs. Production uses the
    /// first ID in this list that still has a runtime `C4Player`; this order
    /// is independent of the reusable in-round player number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub player_ids: Vec<i32>,
    /// Optional one-based `[PlayerN]` scenario-start slot shared by every
    /// player on this team (`C4Team::iPlrStartIndex`, C4Teams.h:58).
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub player_start_index: i32,
    /// Zero means unlimited; positive values cap new team joins
    /// (`C4Team::iMaxPlayer`, C4Teams.cpp:545-560).
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub max_players: i32,
    /// `Teams.txt` DrawTextSpecImage recipe used by team-selection and
    /// evaluation screens.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub icon_spec: Option<String>,
}

/// Live `C4TeamList` settings queried by `GetTeamConfig`. Keep these
/// independently of the team entries: an empty custom Teams.txt and a
/// missing non-melee Teams.txt have different configuration values.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamConfiguration {
    pub custom: bool,
    pub active: bool,
    pub allow_hostility_change: bool,
    pub distribution: i32,
    pub allow_team_switch: bool,
    pub auto_generate_teams: bool,
    pub team_colors: bool,
}

impl Default for TeamConfiguration {
    fn default() -> Self {
        // Raw C4TeamList constructor/Clear state (C4Teams.h:154-157).
        Self {
            custom: false,
            active: true,
            allow_hostility_change: true,
            distribution: 0,
            allow_team_switch: false,
            auto_generate_teams: false,
            team_colors: false,
        }
    }
}

impl TeamConfiguration {
    pub(crate) fn script_value(self, query: i32) -> Option<i32> {
        match query {
            1 => Some(i32::from(self.custom)),
            2 => Some(i32::from(self.active)),
            3 => Some(i32::from(self.allow_hostility_change)),
            4 => Some(self.distribution),
            5 => Some(i32::from(self.allow_team_switch)),
            6 => Some(i32::from(self.auto_generate_teams)),
            7 => Some(i32::from(self.team_colors)),
            _ => None,
        }
    }
}

impl From<&InitialNetworkTeamMetadata> for TeamConfiguration {
    fn from(metadata: &InitialNetworkTeamMetadata) -> Self {
        Self {
            custom: metadata.custom,
            active: metadata.active,
            allow_hostility_change: metadata.allow_hostility_change,
            distribution: metadata.team_distribution as i32,
            allow_team_switch: metadata.allow_team_switch,
            auto_generate_teams: metadata.auto_generate_teams,
            team_colors: metadata.team_colors,
        }
    }
}

impl TeamInfo {
    pub fn new(id: i32, name: impl Into<String>, color: u32) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            player_ids: Vec::new(),
            player_start_index: 0,
            max_players: 0,
            icon_spec: None,
        }
    }

    pub fn with_player_ids(mut self, player_ids: Vec<i32>) -> Self {
        self.player_ids = player_ids;
        self
    }

    pub fn with_player_start_index(mut self, player_start_index: i32) -> Self {
        self.player_start_index = player_start_index;
        self
    }

    pub fn with_max_players(mut self, max_players: i32) -> Self {
        self.max_players = max_players;
        self
    }

    pub fn with_icon_spec(mut self, icon_spec: impl Into<String>) -> Self {
        let icon_spec = icon_spec.into();
        self.icon_spec = (!icon_spec.is_empty()).then_some(icon_spec);
        self
    }
}

const DEFAULT_GENERATED_TEAM_COLORS: [u32; 10] = [
    0x00f4_0000,
    0x0000_c800,
    0x00fc_f41c,
    0x0020_20ff,
    0x00c4_8444,
    0x00ff_ffff,
    0x0084_8484,
    0x00ff_00ef,
    0x0000_ffff,
    0x0078_4830,
];

pub(crate) fn default_generated_team_color(id: i32) -> Option<u32> {
    id.checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| DEFAULT_GENERATED_TEAM_COLORS.get(index))
        .copied()
}

/// An initialized player's assigned number plus the start position and base
/// that ScenarioInit passed to InitializePlayer (C4Player.cpp:769-775).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinedPlayer {
    pub number: i32,
    pub start_x: i32,
    pub start_y: i32,
    pub first_base: Option<ObjectId>,
}

/// The two valid outcomes of registering a player with the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPlayerOutcome {
    /// ScenarioInit and InitializePlayer completed for this player.
    Initialized(JoinedPlayer),
    /// PreInitializePlayer completed, but ScenarioInit waits for a team choice.
    AwaitingTeamSelection { number: i32 },
}

impl JoinPlayerOutcome {
    pub const fn number(self) -> i32 {
        match self {
            Self::Initialized(joined) => joined.number,
            Self::AwaitingTeamSelection { number } => number,
        }
    }

    pub const fn initialized(self) -> Option<JoinedPlayer> {
        match self {
            Self::Initialized(joined) => Some(joined),
            Self::AwaitingTeamSelection { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlJoinPlayerSemantics {
    script_player: bool,
    scenario_init: bool,
    no_elimination_check: bool,
    extra_id: Option<DefinitionId>,
    /// Authoritative C4PlayerInfo league score. Generic/synthetic joins use
    /// None so they do not overwrite a preseeded retained PlayerInfo row.
    league_score: Option<i32>,
    league_progress_data: Option<Vec<u8>>,
}

impl Default for ControlJoinPlayerSemantics {
    fn default() -> Self {
        Self {
            script_player: false,
            scenario_init: true,
            no_elimination_check: false,
            extra_id: None,
            league_score: None,
            league_progress_data: None,
        }
    }
}

impl From<&ControlPlayerInfoEntry> for ControlJoinPlayerSemantics {
    fn from(info: &ControlPlayerInfoEntry) -> Self {
        let extra_id = (info.extra_data != *b"NONE" && info.extra_data != *b"0000")
            .then(|| String::from_utf8(info.extra_data.to_vec()).ok())
            .flatten();
        Self {
            script_player: info.is_script_player(),
            scenario_init: !info.no_scenario_init(),
            no_elimination_check: info.no_elimination_check(),
            extra_id,
            league_score: Some(info.league_score),
            league_progress_data: (!info.league_progress_data_is_null
                || !info.league_progress_data.is_empty())
            .then(|| info.league_progress_data.as_bytes().to_vec()),
        }
    }
}

/// The `pObj->Info` data a crew object carries (CreateInfoObject links the
/// C4ObjectInfo, C4Game.cpp:1156-1170): name shown by GetName, rank used
/// by GetHiRank.
fn default_crew_rank_name() -> String {
    "Clonk".to_string()
}

fn default_crew_type_name() -> String {
    "Clonk".to_string()
}

fn bounded_c4_string(value: &str, max_len: usize) -> String {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes.truncate(max_len);
    clonk_script::c4_string_from_bytes(&bytes)
}

pub(crate) fn bounded_crew_type_name(name: &str) -> String {
    bounded_c4_string(name, 30)
}

pub(crate) fn bounded_loaded_crew_type_name(name: &str) -> String {
    bounded_c4_string(name, 31)
}

pub(crate) fn bounded_crew_portrait_file(name: &str) -> String {
    bounded_c4_string(name, 36)
}

fn default_crew_participation() -> i32 {
    1
}

fn is_default_crew_type_name(value: &String) -> bool {
    value == "Clonk"
}

fn is_default_crew_participation(value: &i32) -> bool {
    *value == 1
}

/// C4ObjectInfoCore fields that are independent of the live portrait
/// graphics and the ordinary rank/experience counters. These values are
/// persisted verbatim when an info is loaded; `UpdateCustomRanks` refreshes
/// the next-rank pair only when a new info is created or a crew file is
/// saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewInfoCoreFields {
    /// Original `C4ObjectInfo::Filename`. A fresh network player group still
    /// keeps this name because the attempted rename has no source entry.
    #[serde(
        default,
        with = "clonk_script::c4_string_serde",
        skip_serializing_if = "String::is_empty"
    )]
    pub original_filename: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub portrait_file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next_rank_name: String,
    #[serde(
        default = "default_crew_type_name",
        skip_serializing_if = "is_default_crew_type_name"
    )]
    pub type_name: String,
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub next_rank_exp: i32,
    /// Loaded, decodable portrait/rank payloads retained for creation of a
    /// fresh network `.c4p` group. C++ owns equivalent graphics surfaces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portrait_png: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portrait_overlay_png: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portrait_bmp: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rank_png: Vec<u8>,
    /// Reconstruction source for an owned portrait produced by
    /// `SetPortrait(..., copy=true)`. Native C++ retains the copied surface;
    /// Rust retains its immutable source until it is encoded for a save.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owned_portrait_source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owned_portrait_name: String,
}

impl Default for CrewInfoCoreFields {
    fn default() -> Self {
        Self {
            original_filename: String::new(),
            portrait_file: String::new(),
            next_rank_name: String::new(),
            type_name: default_crew_type_name(),
            next_rank_exp: 0,
            portrait_png: Vec::new(),
            portrait_overlay_png: Vec::new(),
            portrait_bmp: Vec::new(),
            rank_png: Vec::new(),
            owned_portrait_source: String::new(),
            owned_portrait_name: String::new(),
        }
    }
}

/// Output projection of `C4ObjectInfoCore::GetNextRankInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrewNextRankInfo<'a> {
    /// `None` means native lookup would leave the optional name output
    /// untouched; `Some("")` represents a found or stored empty name.
    pub name: Option<&'a str>,
    pub experience: i32,
}

impl CrewNextRankInfo<'_> {
    /// Native code treats only `EXP_NoPromotion` (-1) as exhausted; other
    /// negative persisted values still report that promotion is possible.
    pub fn promotion_possible(&self) -> bool {
        self.experience != -1
    }
}

impl CrewInfoCoreFields {
    /// Resolve stored custom progression or project the supplied default rank
    /// system at `rank + 1`, without rewriting the persisted next-rank fields.
    pub fn next_rank_info<'a, S: AsRef<str>>(
        &'a self,
        rank: i32,
        default_rank_names: &'a [S],
        default_rank_base: i32,
    ) -> CrewNextRankInfo<'a> {
        if self.next_rank_exp != 0 {
            return CrewNextRankInfo {
                name: Some(&self.next_rank_name),
                experience: self.next_rank_exp,
            };
        }

        let Some(next_rank) = rank.checked_add(1) else {
            return CrewNextRankInfo {
                name: None,
                experience: -1,
            };
        };
        let Some(name) = usize::try_from(next_rank)
            .ok()
            .and_then(|rank| default_rank_names.get(rank))
            .map(AsRef::as_ref)
        else {
            return CrewNextRankInfo {
                name: None,
                experience: -1,
            };
        };
        CrewNextRankInfo {
            name: Some(name),
            experience: rank_experience(next_rank, default_rank_base),
        }
    }
}

/// One resolved C4ObjectInfo portrait. Definition-backed portraits retain
/// their source ID; owned/custom graphics deliberately do not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewPortrait {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DefinitionId>,
    pub name: String,
}

/// Runtime `C4ObjectInfo::pNewPortrait` state. `Absent` must remain distinct
/// from an allocated-but-empty portrait because only the former falls back
/// to the saved portrait specification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrewPermanentPortrait {
    #[default]
    Absent,
    ExplicitNone,
    Assigned(CrewPortrait),
}

/// Portrait data owned by one C4ObjectInfo pointer. This travels as a unit
/// through GrabObjectInfo and survives retirement in the player's roster.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewPortraitState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<CrewPortrait>,
    /// Evaluated `PortraitFile`/custom-file fallback. It is intentionally not
    /// validated here: permanent GetPortrait returns the saved spec even if
    /// its definition graphics are unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<CrewPortrait>,
    #[serde(default)]
    pub permanent: CrewPermanentPortrait,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewObjectInfo {
    /// Original C4ObjectInfoCore definition id. This deliberately survives a
    /// runtime ChangeDef; System.c4g uses it to cap physical training by the
    /// recruited crew's source definition.
    pub definition_id: DefinitionId,
    #[serde(with = "clonk_script::c4_string_serde")]
    pub name: String,
    /// Verbatim C4ObjectInfoCore death announcement; empty selects the
    /// localized random fallback.
    #[serde(
        default,
        with = "clonk_script::c4_string_serde",
        skip_serializing_if = "String::is_empty"
    )]
    pub death_message: String,
    /// Persisted `PortraitFile`, `NextRankName`, `TypeName`, and
    /// `NextRankExp` values from C4ObjectInfoCore.
    #[serde(default, flatten)]
    pub core: CrewInfoCoreFields,
    pub rank: i32,
    /// Stored `C4ObjectInfoCore::sRankName`. Silent over-table promotions
    /// retain the preceding name (C4InfoCore.cpp:428-435).
    #[serde(default = "default_crew_rank_name")]
    pub rank_name: String,
    pub experience: i32,
    /// Persisted C4ObjectInfoCore participation flag. This is independent
    /// of live crew-list membership.
    #[serde(
        default = "default_crew_participation",
        skip_serializing_if = "is_default_crew_participation"
    )]
    pub participation: i32,
    /// Persistent C4ObjectInfoCore participation-round tally.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub rounds: i32,
    /// Persistent C4ObjectInfoCore death tally.
    #[serde(default)]
    pub death_count: i32,
    #[serde(default)]
    pub total_playing_time: i32,
    #[serde(default)]
    pub birthday: i32,
    #[serde(default)]
    pub age: i32,
    /// Runtime `C4ObjectInfo::InActionTime`; the object-info pointer shares
    /// this data with its owning roster entry in C++.
    #[serde(default)]
    pub in_action_time: i32,
    /// Ordered `C4ObjectInfoCore::ExtraData` slots. This live projection is
    /// mirrored from the owning roster entry and follows GrabObjectInfo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_data: Vec<(String, clonk_script::Value)>,
    #[serde(default)]
    pub portraits: CrewPortraitState,
}

/// Stable identity of one C4ObjectInfo inside a player's CrewInfoList.
/// Roster index is the exact pointer-equivalent needed by Retire and
/// GrabObjectInfo; info fields are not unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CrewInfoLink {
    pub player_id: i32,
    pub roster_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WeatherEvent {
    Lightning { position: i32 },
    Meteorite { x: i32 },
    Earthquake { x: i32, y: i32 },
    Volcano { x: i32, y: i32, size: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CrewRole(String);

impl CrewRole {
    pub fn new(role: impl Into<String>) -> Self {
        Self(role.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CrewRole {
    fn from(role: &str) -> Self {
        Self::new(role)
    }
}

impl From<String> for CrewRole {
    fn from(role: String) -> Self {
        Self::new(role)
    }
}

impl fmt::Display for CrewRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrewCommandTarget {
    Cursor,
    Selection,
    Role(CrewRole),
}

impl CrewCommandTarget {
    pub const fn cursor() -> Self {
        Self::Cursor
    }

    pub const fn selection() -> Self {
        Self::Selection
    }

    pub fn role(role: impl Into<CrewRole>) -> Self {
        Self::Role(role.into())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vector2 {
    pub x: i32,
    pub y: i32,
}

impl Vector2 {
    pub const ZERO: Self = Self { x: 0, y: 0 };

    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    fn to_value(self) -> Value {
        Value::Array(vec![Value::Int(self.x), Value::Int(self.y)])
    }
}

impl AddAssign<Vector2> for Vector2 {
    fn add_assign(&mut self, rhs: Vector2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FloatVector2 {
    pub x: f32,
    pub y: f32,
}

impl FloatVector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl PartialEq for FloatVector2 {
    fn eq(&self, other: &Self) -> bool {
        self.x.to_bits() == other.x.to_bits() && self.y.to_bits() == other.y.to_bits()
    }
}

impl Eq for FloatVector2 {}

impl AddAssign<FloatVector2> for FloatVector2 {
    fn add_assign(&mut self, rhs: FloatVector2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", content = "object")]
pub enum ParticleLayer {
    #[serde(rename = "global")]
    Global,
    #[serde(rename = "front")]
    ObjectFront(ObjectId),
    #[serde(rename = "back")]
    ObjectBack(ObjectId),
}

impl ParticleLayer {
    pub fn from_ffi(layer: i32, has_owner: bool, owner_id: u64) -> Option<Self> {
        match layer {
            0 => Some(Self::Global),
            1 => {
                if !has_owner {
                    None
                } else {
                    Some(Self::ObjectFront(ObjectId::new(owner_id)))
                }
            }
            2 => {
                if !has_owner {
                    None
                } else {
                    Some(Self::ObjectBack(ObjectId::new(owner_id)))
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleSnapshot {
    pub definition_id: String,
    pub position: FloatVector2,
    pub velocity: FloatVector2,
    pub life: i32,
    #[serde(default)]
    pub parameter_a: f32,
    #[serde(default)]
    pub parameter_b: i32,
    pub layer: ParticleLayer,
    /// Raw `C4Fixed` `[x, y, xdir, ydir]` for C4PXS pixel sprites — the
    /// sync-relevant state; the float fields above are lossy projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pxs_fixed: Option<[i32; 4]>,
    /// Saved chunk/slot position (`chunk * PXS_CHUNK_SIZE + slot`) of a
    /// C4PXS pixel sprite: C4PXSSystem::Save/Load keep the whole chunk
    /// layout, MNone gaps included (C4PXS.cpp:346-349, 383-397), so
    /// restore must place each pixel back in its slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pxs_slot: Option<u32>,
}

impl PartialEq for ParticleSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.definition_id == other.definition_id
            && self.position == other.position
            && self.velocity == other.velocity
            && self.life == other.life
            && self.parameter_a.to_bits() == other.parameter_a.to_bits()
            && self.parameter_b == other.parameter_b
            && self.layer == other.layer
            && self.pxs_fixed == other.pxs_fixed
            && self.pxs_slot == other.pxs_slot
    }
}

impl Eq for ParticleSnapshot {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticleScope {
    Global,
    Object(ObjectId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleConfig {
    pub definition_id: String,
    pub position: FloatVector2,
    pub velocity: FloatVector2,
    pub life: i32,
    pub parameter_a: f32,
    pub parameter_b: i32,
    pub layer: ParticleLayer,
}

impl PartialEq for ParticleConfig {
    fn eq(&self, other: &Self) -> bool {
        self.definition_id == other.definition_id
            && self.position == other.position
            && self.velocity == other.velocity
            && self.life == other.life
            && self.parameter_a.to_bits() == other.parameter_a.to_bits()
            && self.parameter_b == other.parameter_b
            && self.layer == other.layer
    }
}

impl Eq for ParticleConfig {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParticleCommand {
    Create(ParticleConfig),
    Clear {
        definition_id: Option<String>,
        scope: ParticleScope,
    },
    /// `C4ParticleSystem::Cast` via FnCastParticles/FnCastBackParticles
    /// (C4Script.cpp:4881-4908). Coordinates are world coordinates (the
    /// caller's local offset is already applied); `a0`/`a1` carry the script
    /// ints divided by 10, `b0`/`b1` the raw color bounds.
    Cast {
        definition_id: String,
        amount: i32,
        x: f32,
        y: f32,
        level: i32,
        a0: f32,
        b0: u32,
        a1: f32,
        b1: u32,
        layer: ParticleLayer,
    },
    /// `C4ParticleSystem::Push` via FnPushParticles (C4Script.cpp:4910-4923):
    /// deltas are script ints divided by 10; no def = push every particle.
    Push {
        definition_id: Option<String>,
        dxdir: f32,
        dydir: f32,
    },
}

#[derive(Debug, Clone)]
struct ActiveParticle {
    snapshot: ParticleSnapshot,
    original_life: i32,
}

impl ActiveParticle {
    fn from_config(config: ParticleConfig) -> Self {
        let ParticleConfig {
            definition_id,
            position,
            velocity,
            life,
            parameter_a,
            parameter_b,
            layer,
        } = config;
        let clamped_life = life.max(0);
        let snapshot = ParticleSnapshot {
            definition_id,
            position,
            velocity,
            life: clamped_life,
            parameter_a,
            parameter_b,
            layer,
            pxs_fixed: None,
            pxs_slot: None,
        };
        Self {
            snapshot,
            original_life: clamped_life,
        }
    }

    fn from_snapshot(mut snapshot: ParticleSnapshot) -> Self {
        if snapshot.life < 0 {
            snapshot.life = 0;
        }
        let original_life = snapshot.life;
        Self {
            snapshot,
            original_life,
        }
    }

    fn tick(&mut self) {
        self.snapshot.position += self.snapshot.velocity;
        if self.original_life > 0 && self.snapshot.life > 0 {
            self.snapshot.life -= 1;
        }
    }

    fn is_expired(&self) -> bool {
        self.original_life > 0 && self.snapshot.life == 0
    }

    fn snapshot(&self) -> ParticleSnapshot {
        self.snapshot.clone()
    }
}

/// Snapshot form of a `C4ParticleSystem` particle (save/load + FFI surface).
fn system_particle_snapshot(particle: &particles::Particle) -> ParticleSnapshot {
    ParticleSnapshot {
        definition_id: particle.def_name.clone(),
        position: FloatVector2::new(particle.x, particle.y),
        velocity: FloatVector2::new(particle.xdir, particle.ydir),
        life: particle.life,
        parameter_a: particle.a,
        parameter_b: particle.b,
        layer: particle.layer.clone(),
        pxs_fixed: None,
        pxs_slot: None,
    }
}

/// Snapshot form of a C4PXS pixel sprite. The float position/velocity are
/// `fixtof` projections for display; `pxs_fixed` carries the raw sync-relevant
/// `C4Fixed` state for lossless save/load. `slot` is the saved chunk-major
/// slot position (C4PXSSystem::Save keeps the chunk layout verbatim,
/// C4PXS.cpp:346-349); presentation snapshots retain it because Draw derives
/// each graphical phase and size from the slot within its 500-entry chunk
/// (C4PXS.cpp:285-304).
fn pxs_snapshot(pxs: &pxs::Pxs, materials: &MaterialSet, slot: Option<u32>) -> ParticleSnapshot {
    let definition_id = materials
        .get_by_id(pxs.mat)
        .map(|material| format!("material/pxs/{}", material.normalized_name()))
        .unwrap_or_else(|| "material/pxs/unknown".to_string());
    ParticleSnapshot {
        definition_id,
        position: FloatVector2::new(math::fixtof(pxs.x), math::fixtof(pxs.y)),
        velocity: FloatVector2::new(math::fixtof(pxs.xdir), math::fixtof(pxs.ydir)),
        life: 0,
        parameter_a: 0.0,
        parameter_b: pxs.mat.index() as i32,
        layer: ParticleLayer::Global,
        pxs_fixed: Some([pxs.x.val(), pxs.y.val(), pxs.xdir.val(), pxs.ydir.val()]),
        pxs_slot: slot,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HudSnapshot {
    #[serde(default)]
    pub players: Vec<HudPlayerSnapshot>,
    #[serde(default)]
    pub messages: Vec<MessageSnapshot>,
    #[serde(default, skip_serializing_if = "ScoreboardState::is_default")]
    pub scoreboard: ScoreboardState,
    /// Ordered runtime-only `DoDlgShow` reconciliations. Initialization and
    /// save-load activity is discarded before capture begins in shared GUI
    /// mode; this queue is never part of deterministic save state.
    #[serde(skip)]
    pub scoreboard_presentations: Vec<ScoreboardPresentationRequest>,
    /// Players controlled by this client (`C4Player::LocalControl`, NO-SAVE).
    #[serde(skip)]
    pub local_players: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudPlayerSnapshot {
    pub owner: i32,
    #[serde(default)]
    pub crew: Vec<ObjectId>,
    #[serde(default)]
    pub focus: Option<ObjectId>,
    #[serde(default)]
    pub eliminated: bool,
    #[serde(default)]
    pub wealth: i32,
    #[serde(default)]
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SurfaceSnapshot {
    #[serde(default)]
    pub label: String,
    pub width: i32,
    pub height: i32,
    pub hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPacketDirection {
    #[default]
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NetworkPacketSnapshot {
    #[serde(default)]
    pub direction: NetworkPacketDirection,
    pub status: u8,
    pub size: u32,
    pub hash: u64,
    pub client_id: i32,
    #[serde(default)]
    pub connection_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObjectVertex {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub cnat: u32,
    #[serde(default)]
    pub friction: i32,
}

impl ObjectVertex {
    pub fn new(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            cnat: CNAT_NONE,
            friction: 0,
        }
    }

    pub fn with_cnat(mut self, cnat: u32) -> Self {
        self.cnat = cnat;
        self
    }

    pub fn with_friction(mut self, friction: i32) -> Self {
        self.friction = friction;
        self
    }
}

const MAX_SHAPE_VERTICES: usize = 30;

/// The complete fixed-size C4Shape vertex storage. `ObjectState::vertices`
/// remains the public active-prefix view, while this retains dormant slots
/// that C++ deliberately keeps beyond `VtxNum` (C4Shape.h/C4Shape.cpp).
///
/// In particular, RemoveVertex shifts only X/Y and AddVertex overwrites only
/// X/Y. The per-slot CNAT/friction values must therefore survive while the
/// active count is zero (Alchemy's Warp spell relies on that round-trip).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ShapeVertexBuffer {
    count: u8,
    slots: [ObjectVertex; MAX_SHAPE_VERTICES],
}

impl Default for ShapeVertexBuffer {
    fn default() -> Self {
        Self {
            count: 0,
            slots: [ObjectVertex::default(); MAX_SHAPE_VERTICES],
        }
    }
}

impl ShapeVertexBuffer {
    pub(crate) fn active_count(&self) -> usize {
        usize::from(self.count).min(MAX_SHAPE_VERTICES)
    }

    fn from_active(vertices: &[ObjectVertex]) -> Self {
        Self::from_slots(vertices.len(), vertices)
    }

    fn from_slots(active_count: usize, vertices: &[ObjectVertex]) -> Self {
        let mut buffer = Self::default();
        let copy_len = vertices.len().min(MAX_SHAPE_VERTICES);
        buffer.slots[..copy_len].copy_from_slice(&vertices[..copy_len]);
        buffer.count = active_count.min(MAX_SHAPE_VERTICES) as u8;
        buffer
    }

    pub(crate) fn active(&self) -> &[ObjectVertex] {
        &self.slots[..self.active_count()]
    }

    pub(crate) fn slots(&self) -> &[ObjectVertex; MAX_SHAPE_VERTICES] {
        &self.slots
    }

    fn active_vec(&self) -> Vec<ObjectVertex> {
        self.active().to_vec()
    }

    /// C4Shape::CopyFrom's vertex copy: replace the active prefix and count,
    /// but leave every slot beyond the new count untouched.
    fn replace_active(&mut self, vertices: &[ObjectVertex]) {
        let count = vertices.len().min(MAX_SHAPE_VERTICES);
        self.slots[..count].copy_from_slice(&vertices[..count]);
        self.count = count as u8;
    }

    fn add(&mut self, x: i32, y: i32) -> bool {
        let index = usize::from(self.count);
        if index >= MAX_SHAPE_VERTICES {
            return false;
        }
        self.slots[index].x = x;
        self.slots[index].y = y;
        self.count += 1;
        true
    }

    fn remove(&mut self, index: i32) -> bool {
        let Ok(index) = usize::try_from(index) else {
            return false;
        };
        let count = self.active_count();
        if index >= count {
            return false;
        }
        for slot in index..count - 1 {
            self.slots[slot].x = self.slots[slot + 1].x;
            self.slots[slot].y = self.slots[slot + 1].y;
        }
        self.count = (count - 1) as u8;
        true
    }

    fn is_canonical_for(&self, active: &[ObjectVertex]) -> bool {
        self.active() == active
            && self.slots[self.active_count()..]
                .iter()
                .all(|vertex| *vertex == ObjectVertex::default())
    }

    fn own_original_vertices(&self) -> Vec<ObjectVertex> {
        let count = self.active_count().min(MAX_SHAPE_VERTICES / 2);
        self.slots[MAX_SHAPE_VERTICES / 2..MAX_SHAPE_VERTICES / 2 + count].to_vec()
    }

    /// `C4Shape::CreateOwnOriginalCopy` (C4Shape.cpp:484-494): seed the
    /// backup half from the definition shape and truncate the active count to
    /// what that half can hold. Entering own-vertex mode copies the *definition*
    /// vertices, not the object's current (Con/rotation-transformed) ones.
    fn create_own_original_copy(&mut self, definition: &[ObjectVertex]) {
        let count = definition.len().min(MAX_SHAPE_VERTICES / 2);
        self.count = count as u8;
        self.slots[MAX_SHAPE_VERTICES / 2..MAX_SHAPE_VERTICES / 2 + count]
            .copy_from_slice(&definition[..count]);
    }

    /// One `C4Shape` slot, addressed exactly like the C++ `VtxX[iIndex]`
    /// arrays — including the backup half that own-vertex mode writes.
    fn slot_mut(&mut self, index: usize) -> Option<&mut ObjectVertex> {
        self.slots.get_mut(index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsSettings {
    pub gravity: i32,
    /// Exact `C4Landscape::Gravity` bits restored from a C++ `Game.txt`.
    /// Ordinary scenario/script gravity uses the public integer projection;
    /// this override exists because the runtime compiler accepts every 16.16
    /// value, including values that cannot round-trip through `GetGravity`.
    gravity_raw: Option<i32>,
    pub max_fall_speed: i32,
    pub max_rise_speed: i32,
    pub max_horizontal_speed: i32,
}

impl PhysicsSettings {
    pub const DEFAULT_MAX_HORIZONTAL_SPEED: i32 = 12;

    pub const fn new(gravity: i32, max_fall_speed: i32, max_rise_speed: i32) -> Self {
        Self {
            gravity,
            gravity_raw: None,
            max_fall_speed,
            max_rise_speed,
            max_horizontal_speed: Self::DEFAULT_MAX_HORIZONTAL_SPEED,
        }
    }

    pub fn checked(
        gravity: i32,
        max_fall_speed: i32,
        max_rise_speed: i32,
    ) -> Result<Self, &'static str> {
        if max_rise_speed > max_fall_speed {
            return Err("max_rise_speed must be <= max_fall_speed");
        }
        Ok(Self::new(gravity, max_fall_speed, max_rise_speed))
    }

    pub fn with_max_horizontal_speed(
        self,
        max_horizontal_speed: i32,
    ) -> Result<Self, &'static str> {
        if max_horizontal_speed < 0 {
            return Err("max_horizontal_speed must be >= 0");
        }
        Ok(Self {
            max_horizontal_speed,
            ..self
        })
    }

    const fn default_max_horizontal_speed() -> i32 {
        Self::DEFAULT_MAX_HORIZONTAL_SPEED
    }

    pub fn gravity_as_c4fixed(&self) -> C4Fixed {
        self.canonical_gravity_raw()
            .map(C4Fixed::from_raw)
            .unwrap_or_else(|| fixed100(self.gravity) / 5)
    }

    /// Raw signed 16.16 value used by C4's runtime landscape compiler.
    pub fn gravity_raw(&self) -> i32 {
        self.gravity_as_c4fixed().val()
    }

    pub(crate) fn set_script_gravity(&mut self, gravity: i32) {
        self.gravity = gravity.clamp(-300, 300);
        self.gravity_raw = None;
    }

    pub(crate) fn set_raw_gravity(&mut self, gravity_raw: i32) {
        self.gravity_raw = Some(gravity_raw);
        self.gravity = fixtoi(C4Fixed::from_raw(gravity_raw) * 500);
    }

    fn reconcile_raw_gravity(&mut self) {
        if self.gravity_raw != self.canonical_gravity_raw() {
            // The public script-facing field was edited through the existing
            // PhysicsSettings API. Treat that as a new SetGravity-style value
            // instead of letting a hidden savegame override win.
            self.gravity_raw = None;
        }
    }

    fn canonical_gravity_raw(&self) -> Option<i32> {
        self.gravity_raw
            .filter(|raw| fixtoi(C4Fixed::from_raw(*raw) * 500) == self.gravity)
    }

    fn clamp_fixed_velocity(&self, velocity: &mut FixedVec2) {
        let min_vertical = self.max_rise_speed.min(self.max_fall_speed);
        let max_vertical = self.max_rise_speed.max(self.max_fall_speed);
        velocity.y =
            clamp_fixed_to_limit_pair(velocity.y, itofix(min_vertical), itofix(max_vertical));
        let max_horizontal = self.max_horizontal_speed.max(0);
        velocity.x = clamp_fixed_to_limit(velocity.x, max_horizontal);
    }
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            gravity: 1,
            gravity_raw: None,
            max_fall_speed: 12,
            max_rise_speed: -20,
            max_horizontal_speed: Self::DEFAULT_MAX_HORIZONTAL_SPEED,
        }
    }
}

impl Serialize for PhysicsSettings {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Fields {
            gravity: i32,
            #[serde(skip_serializing_if = "Option::is_none")]
            gravity_raw: Option<i32>,
            max_fall_speed: i32,
            max_rise_speed: i32,
            max_horizontal_speed: i32,
        }

        Fields {
            gravity: self.gravity,
            gravity_raw: self.canonical_gravity_raw(),
            max_fall_speed: self.max_fall_speed,
            max_rise_speed: self.max_rise_speed,
            max_horizontal_speed: self.max_horizontal_speed,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PhysicsSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Fields {
            gravity: i32,
            #[serde(default)]
            gravity_raw: Option<i32>,
            max_fall_speed: i32,
            max_rise_speed: i32,
            #[serde(default = "PhysicsSettings::default_max_horizontal_speed")]
            max_horizontal_speed: i32,
        }

        let fields = Fields::deserialize(deserializer)?;
        let mut physics = Self {
            gravity: fields.gravity,
            gravity_raw: fields.gravity_raw,
            max_fall_speed: fields.max_fall_speed,
            max_rise_speed: fields.max_rise_speed,
            max_horizontal_speed: fields.max_horizontal_speed,
        };
        physics.reconcile_raw_gravity();
        Ok(physics)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MovementProfile {
    pub walk_speed: i32,
    pub walk_acceleration: i32,
    pub float_speed: i32,
    pub float_acceleration: i32,
    pub swim_speed: i32,
    pub swim_acceleration: i32,
    pub scale_speed: i32,
    pub scale_acceleration: i32,
    pub hangle_speed: i32,
    pub hangle_acceleration: i32,
    pub dig_speed: i32,
}

impl MovementProfile {
    pub const fn new(float_speed: i32, float_acceleration: i32) -> Self {
        Self {
            walk_speed: 8,
            walk_acceleration: 1,
            float_speed,
            float_acceleration,
            swim_speed: 6,
            swim_acceleration: 1,
            scale_speed: 8,
            scale_acceleration: 1,
            hangle_speed: 8,
            hangle_acceleration: 1,
            dig_speed: 8,
        }
    }

    pub fn with_walk_speed(mut self, walk_speed: i32) -> Self {
        self.walk_speed = walk_speed;
        self
    }

    pub fn with_walk_acceleration(mut self, walk_acceleration: i32) -> Self {
        self.walk_acceleration = walk_acceleration;
        self
    }

    pub fn with_float_speed(mut self, float_speed: i32) -> Self {
        self.float_speed = float_speed;
        self
    }

    pub fn with_float_acceleration(mut self, float_acceleration: i32) -> Self {
        self.float_acceleration = float_acceleration;
        self
    }

    pub fn with_swim_speed(mut self, swim_speed: i32) -> Self {
        self.swim_speed = swim_speed;
        self
    }

    pub fn with_swim_acceleration(mut self, swim_acceleration: i32) -> Self {
        self.swim_acceleration = swim_acceleration;
        self
    }

    pub fn with_scale_speed(mut self, scale_speed: i32) -> Self {
        self.scale_speed = scale_speed;
        self
    }

    pub fn with_scale_acceleration(mut self, scale_acceleration: i32) -> Self {
        self.scale_acceleration = scale_acceleration;
        self
    }

    pub fn with_hangle_speed(mut self, hangle_speed: i32) -> Self {
        self.hangle_speed = hangle_speed;
        self
    }

    pub fn with_hangle_acceleration(mut self, hangle_acceleration: i32) -> Self {
        self.hangle_acceleration = hangle_acceleration;
        self
    }

    pub fn with_dig_speed(mut self, dig_speed: i32) -> Self {
        self.dig_speed = dig_speed;
        self
    }
}

impl Default for MovementProfile {
    fn default() -> Self {
        Self {
            walk_speed: 8,
            walk_acceleration: 1,
            float_speed: 6,
            float_acceleration: 1,
            swim_speed: 6,
            swim_acceleration: 1,
            scale_speed: 8,
            scale_acceleration: 1,
            hangle_speed: 8,
            hangle_acceleration: 1,
            dig_speed: 8,
        }
    }
}

#[derive(Clone, Copy)]
#[doc(hidden)]
pub struct BridgeParameters {
    #[doc(hidden)]
    pub duration: i32,
    #[doc(hidden)]
    pub move_clonk: bool,
    wall: bool,
    material: Option<MaterialId>,
}

impl BridgeParameters {
    #[doc(hidden)]
    pub fn from_action_data(data: i32) -> Self {
        let raw = data as u32;
        let duration_raw = (raw >> 16) & 0xFFFF;
        let duration = if duration_raw == 0 {
            100
        } else {
            duration_raw as i32
        };
        let move_clonk = (raw & 0x100) != 0;
        let wall = (raw & 0x200) != 0;
        let material_byte = (raw & 0xFF) as u8;
        let material = (material_byte != 0xFF)
            .then(|| MaterialId::new(usize::from(material_byte)))
            .flatten();
        Self {
            duration,
            move_clonk,
            wall,
            material,
        }
    }

    fn step_interval(&self, direction: CommandDirection) -> Option<i32> {
        if self.wall {
            match direction {
                CommandDirection::Left | CommandDirection::Right => Some(4),
                CommandDirection::UpLeft | CommandDirection::UpRight | CommandDirection::Up => {
                    Some(5)
                }
                _ => None,
            }
        } else {
            match direction {
                CommandDirection::Left | CommandDirection::Right => Some(5),
                CommandDirection::Up => Some(4),
                CommandDirection::UpLeft | CommandDirection::UpRight => Some(6),
                _ => None,
            }
        }
    }
}

#[doc(hidden)]
pub fn encode_bridge_action_data(
    duration: i32,
    move_clonk: bool,
    wall: bool,
    material: i32,
) -> i32 {
    let clamped_duration = duration.clamp(0, 0xFFFF) as u32;
    let mut raw = clamped_duration << 16;
    if move_clonk {
        raw |= 1 << 8;
    }
    if wall {
        raw |= 1 << 9;
    }
    let material_byte = if material < 0 {
        0xFF
    } else {
        (material as u32) & 0xFF
    };
    raw |= material_byte;
    raw as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSettings {
    pub wind: i32,
    #[serde(default)]
    pub base_wind: i32,
    #[serde(default)]
    pub wind_target: i32,
    #[serde(default)]
    pub wind_update_timer: u16,
    #[serde(default)]
    pub wind_update_interval: u16,
    #[serde(default)]
    pub wind_variation: i32,
    #[serde(default)]
    pub wind_period: u32,
    /// Scenario wind bounds (C4SWeather::Default: Wind.Set(0, 70, -100, 100),
    /// C4Scenario.cpp:377).
    #[serde(default = "default_wind_min")]
    pub wind_min: i32,
    #[serde(default = "default_wind_max")]
    pub wind_max: i32,
    #[serde(default)]
    pub temperature: i32,
    #[serde(default)]
    pub climate: i32,
    #[serde(default)]
    pub temperature_variation: i32,
    #[serde(default)]
    pub temperature_period: u32,
    #[serde(default)]
    pub temperature_phase: u32,
    #[serde(default)]
    pub time_of_day: u16,
    #[serde(default)]
    pub time_speed: i16,
    #[serde(default)]
    pub precipitation: i32,
    #[serde(default)]
    pub sky_color: Option<RgbColor>,
    #[serde(default)]
    pub season: i32,
    #[serde(default)]
    pub year_speed: i32,
    #[serde(default)]
    pub season_delay: i32,
    /// Scenario `StartSeason.Min` — the wrap target of the season advance
    /// (C4Weather.cpp:82-83); C4SVal default 0 (C4Scenario.h:30).
    #[serde(default)]
    pub season_min: i32,
    /// Scenario `StartSeason.Max` — the wrap threshold of the season
    /// advance (C4Weather.cpp:82); C4SVal default 100 (C4Scenario.h:30).
    #[serde(default = "EnvironmentSettings::default_season_max")]
    pub season_max: i32,
    #[serde(default = "EnvironmentSettings::default_temperature_range")]
    pub temperature_range: i32,
    #[serde(default)]
    pub lightning: i32,
    #[serde(default)]
    pub meteorite: i32,
    #[serde(default)]
    pub volcano: i32,
    #[serde(default)]
    pub earthquake: i32,
    #[serde(default)]
    pub precipitation_strength: i32,
    #[serde(default = "EnvironmentSettings::default_no_gamma")]
    pub no_gamma: bool,
}

impl EnvironmentSettings {
    pub const TIME_CYCLE: u16 = 2400;
    const MAX_TIME_SPEED: i16 = 120;

    const fn default_temperature_range() -> i32 {
        30
    }

    const fn default_no_gamma() -> bool {
        true
    }

    const fn default_season_max() -> i32 {
        100
    }

    pub const fn new(wind: i32) -> Self {
        Self {
            wind,
            base_wind: wind,
            wind_target: wind,
            wind_update_timer: 0,
            wind_update_interval: 0,
            wind_variation: 0,
            wind_period: 0,
            wind_min: -100,
            wind_max: 100,
            temperature: 0,
            climate: 0,
            temperature_variation: 0,
            temperature_period: 0,
            temperature_phase: 0,
            time_of_day: 0,
            time_speed: 0,
            precipitation: 0,
            sky_color: None,
            season: 0,
            year_speed: 0,
            season_delay: 0,
            season_min: 0,
            season_max: Self::default_season_max(),
            temperature_range: Self::default_temperature_range(),
            lightning: 0,
            meteorite: 0,
            volcano: 0,
            earthquake: 0,
            precipitation_strength: 0,
            no_gamma: Self::default_no_gamma(),
        }
    }

    pub fn with_wind_variation(mut self, variation: i32, period: u32) -> Self {
        if variation == 0 {
            self.wind_variation = 0;
            self.wind_period = 0;
            self.wind_target = self.base_wind;
            self.wind_update_interval = 0;
            self.wind_update_timer = 0;
            return self;
        }
        self.wind_variation = variation.abs();
        self.wind_period = period.max(2);
        self.wind_update_interval = Self::default_wind_update_interval(self.wind_period);
        self.wind_update_timer = 0;
        self.wind_target = self.wind;
        self.base_wind = self.wind;
        self
    }

    /// Preserve the scenario `Weather.Wind` C4SVal verbatim. Runtime
    /// `C4Weather::Execute` re-evaluates these exact Std/Rnd/Min/Max fields;
    /// deriving a smaller variation from the bounded initial value changes
    /// both the RNG range and the resulting target wind.
    pub(crate) fn set_legacy_wind_value(&mut self, std: i32, rnd: i32, min: i32, max: i32) {
        self.base_wind = std;
        self.wind_variation = rnd;
        self.wind_period = if rnd == 0 { 0 } else { 2_000 };
        self.wind_update_interval = if rnd == 0 {
            0
        } else {
            Self::default_wind_update_interval(self.wind_period)
        };
        self.wind_update_timer = 0;
        self.wind_min = min;
        self.wind_max = max;
    }

    pub fn with_temperature(mut self, temperature: i32) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_climate(mut self, climate: i32) -> Self {
        self.climate = climate.clamp(-50, 50);
        self
    }

    pub fn with_temperature_cycle(mut self, variation: i32, period: u32, phase: u32) -> Self {
        if variation == 0 {
            self.temperature_variation = 0;
            self.temperature_period = 0;
            self.temperature_phase = 0;
            return self;
        }

        let amplitude = variation.abs();
        let normalized_period = period.max(2);
        self.temperature_variation = amplitude;
        self.temperature_period = normalized_period;
        self.temperature_phase = if normalized_period == 0 {
            0
        } else {
            phase % normalized_period
        };
        self
    }

    pub fn with_time_of_day(mut self, time_of_day: i32) -> Self {
        self.time_of_day = Self::normalize_time_of_day(time_of_day);
        self
    }

    pub fn with_time_speed(mut self, time_speed: i32) -> Self {
        self.time_speed = Self::clamp_time_speed(time_speed);
        self
    }

    pub fn with_precipitation(mut self, precipitation: i32) -> Self {
        let clamped = precipitation.clamp(-100, 100);
        self.precipitation = clamped;
        self
    }

    pub fn with_sky_color(mut self, color: RgbColor) -> Self {
        self.sky_color = Some(color);
        self
    }

    pub fn with_season(mut self, season: i32) -> Self {
        self.season = season.clamp(0, 100);
        self
    }

    pub fn with_year_speed(mut self, year_speed: i32) -> Self {
        self.year_speed = year_speed;
        self
    }

    /// Scenario `StartSeason` Min/Max (C4SVal, C4Scenario.h:30) — the wrap
    /// bounds of C4Weather::Execute's season advance (C4Weather.cpp:82-83).
    pub fn with_season_bounds(mut self, min: i32, max: i32) -> Self {
        self.season_min = min;
        self.season_max = max;
        self
    }

    pub fn with_temperature_range(mut self, range: i32) -> Self {
        self.temperature_range = range.clamp(0, 100);
        self
    }

    pub fn with_lightning(mut self, level: i32) -> Self {
        self.lightning = level.clamp(0, 100);
        self
    }

    pub fn with_meteorite(mut self, level: i32) -> Self {
        self.meteorite = level.clamp(0, 100);
        self
    }

    pub fn with_volcano(mut self, level: i32) -> Self {
        self.volcano = level.clamp(0, 100);
        self
    }

    pub fn with_earthquake(mut self, level: i32) -> Self {
        self.earthquake = level.clamp(0, 100);
        self
    }

    pub fn with_precipitation_strength(mut self, strength: i32) -> Self {
        self.precipitation_strength = strength.clamp(-100, 100);
        self
    }

    pub fn with_gamma_enabled(mut self) -> Self {
        self.no_gamma = false;
        self
    }

    pub fn with_gamma_disabled(mut self) -> Self {
        self.no_gamma = true;
        self
    }

    fn default_wind_update_interval(period: u32) -> u16 {
        if period == 0 {
            return 60;
        }
        let normalized = (period / 2).max(1);
        if normalized >= u32::from(u16::MAX) {
            u16::MAX
        } else {
            normalized as u16
        }
    }

    pub fn without_sky_color(mut self) -> Self {
        self.sky_color = None;
        self
    }

    pub fn sky_color(&self) -> Option<RgbColor> {
        self.sky_color
    }

    pub fn resolved_sky_color(&self, ambient_temperature: i32) -> RgbColor {
        self.sky_color
            .unwrap_or_else(|| Self::dynamic_sky_color(self.time_of_day, ambient_temperature))
    }

    pub fn season_gamma(&self) -> Option<(RgbColor, RgbColor, RgbColor)> {
        if self.no_gamma {
            None
        } else {
            Some(Self::compute_season_gamma(self.season, self.temperature))
        }
    }

    fn season_gamma_control_points(&self) -> Option<[u32; 3]> {
        let pack = |color: RgbColor| {
            (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
        };
        self.season_gamma()
            .map(|(low, middle, high)| [pack(low), pack(middle), pack(high)])
    }

    fn dynamic_sky_color(time_of_day: u16, ambient_temperature: i32) -> RgbColor {
        let normalized_time = f32::from(time_of_day) / f32::from(Self::TIME_CYCLE.max(1));
        let daylight = (1.0 - (normalized_time * core::f32::consts::TAU).cos()) * 0.5;
        let daylight = daylight.clamp(0.0, 1.0);

        let temperature_factor = ((ambient_temperature + 50) as f32 / 100.0).clamp(0.0, 1.0);

        let cold_day = [96.0, 140.0, 212.0];
        let warm_day = [148.0, 196.0, 255.0];
        let night = [12.0, 20.0, 48.0];

        let mut day_color = [0.0; 3];
        for (idx, value) in day_color.iter_mut().enumerate() {
            let cold = cold_day[idx];
            let warm = warm_day[idx];
            *value = cold + (warm - cold) * temperature_factor;
        }

        let mut channel = [0u8; 3];
        for idx in 0..3 {
            let value = night[idx] + (day_color[idx] - night[idx]) * daylight;
            channel[idx] = value.round().clamp(0.0, 255.0) as u8;
        }

        RgbColor::new(channel[0], channel[1], channel[2])
    }

    fn compute_season_gamma(season: i32, temperature: i32) -> (RgbColor, RgbColor, RgbColor) {
        const SEASON_COLORS: [[u32; 3]; 4] = [
            [0x000000, 0x7f7f90, 0xefefff],
            [0x070f00, 0x90a07f, 0xffffdf],
            [0x000000, 0x808080, 0xffffff],
            [0x0f0700, 0xa08067, 0xffffdf],
        ];

        // Preserve C++'s truncation-toward-zero division and signed
        // remainder. This differs from Euclidean modulo for StartSeason
        // values -1..-24 (C4Weather.cpp:263-264).
        let primary = ((season / 25) % 4).rem_euclid(4) as usize;
        let secondary = (primary + 1) % 4;

        let mut offset = season % 25;
        offset = offset.clamp(5, 19);
        let offset_primary = offset - 5;
        let offset_secondary = 15 - offset_primary;

        let mut ramp = [0u32; 3];
        for (idx, color) in ramp.iter_mut().enumerate() {
            let mut accumulated = 0u32;
            for channel_shift in [0usize, 8, 16] {
                let c1 = ((SEASON_COLORS[primary][idx] >> channel_shift) & 0xff) as i32;
                let c2 = ((SEASON_COLORS[secondary][idx] >> channel_shift) & 0xff) as i32;
                let mut value = (c1 * offset_secondary + c2 * offset_primary) / 15;
                if temperature < 0 {
                    if channel_shift == 0 {
                        value -= temperature / 2;
                    } else {
                        value += temperature / 2;
                    }
                }
                let value = value.clamp(0, 255) as u32;
                accumulated |= value << channel_shift;
            }
            *color = accumulated;
        }

        (
            Self::color_from_bgr(ramp[0]),
            Self::color_from_bgr(ramp[1]),
            Self::color_from_bgr(ramp[2]),
        )
    }

    fn color_from_bgr(value: u32) -> RgbColor {
        let r = ((value >> 16) & 0xff) as u8;
        let g = ((value >> 8) & 0xff) as u8;
        let b = (value & 0xff) as u8;
        RgbColor::new(r, g, b)
    }

    pub fn ambient_temperature(&self, frame: u64) -> i32 {
        let base = self.temperature.saturating_add(self.climate);
        if self.temperature_variation == 0 || self.temperature_period == 0 {
            return base;
        }

        let period = self.temperature_period as f32;
        let frame_offset = if self.temperature_period == 0 {
            0
        } else {
            (frame.wrapping_add(u64::from(self.temperature_phase)))
                % u64::from(self.temperature_period)
        };
        let phase = frame_offset as f32 / period;
        let angle = phase * core::f32::consts::TAU;
        let delta = (self.temperature_variation as f32 * angle.cos()).round() as i32;
        base.saturating_sub(delta)
    }

    pub fn temperature_at_height(&self, frame: u64, y: i32, world_height: i32) -> i32 {
        let ambient = self.ambient_temperature(frame);
        if world_height <= 0 {
            return ambient.clamp(-100, 100);
        }
        let clamped_height = y.clamp(0, world_height);
        let fraction = clamped_height as f32 / world_height as f32;
        let gradient = (fraction * 2.0) - 1.0;
        let max_offset = (self.temperature_range / 2).clamp(0, 50);
        let offset = (gradient * max_offset as f32).round() as i32;
        ambient.saturating_add(offset).clamp(-100, 100)
    }

    /// `C4Weather::Execute` (C4Weather.cpp:72-101): season and temperature
    /// step on Tick35 frames; `TargetWind = C4S.Weather.Wind.Evaluate()` on
    /// Tick1000 frames — ONE synced draw,
    /// `BoundBy(Std + Random(2*Rnd + 1) - Rnd, Min, Max)`
    /// (C4SVal::Evaluate, C4Scenario.cpp:43-46); the wind itself steps ±1
    /// toward the target on Tick10 frames.
    pub fn advance_frame(&mut self, rng: &mut LcgRng, frame: u64) -> Option<[u32; 3]> {
        let season_gamma_update = if frame.is_multiple_of(35) {
            let season_changed = self.update_season();
            // C4Weather::Execute refreshes the season curve inside the
            // rollover branch, before this frame's temperature step
            // (C4Weather.cpp:77-93).
            let update = season_changed
                .then(|| self.season_gamma_control_points())
                .flatten();
            self.update_temperature_from_season();
            update
        } else {
            None
        };
        if frame.is_multiple_of(1000) {
            let rnd = self.wind_variation;
            let range = rnd.wrapping_mul(2).wrapping_add(1);
            let target = self
                .base_wind
                .wrapping_add(rng.random(range))
                .wrapping_sub(rnd);
            self.wind_target = bound_by_ordered(target, self.wind_min, self.wind_max);
        }
        if frame.is_multiple_of(10) {
            let stepped = self
                .wind
                .wrapping_add(self.wind_target.wrapping_sub(self.wind).signum());
            self.wind = bound_by_ordered(stepped, self.wind_min, self.wind_max);
        }
        self.advance_time_of_day();
        self.update_precipitation_runtime();
        season_gamma_update
    }

    pub fn time_of_day(&self) -> u16 {
        self.time_of_day
    }

    pub fn time_speed(&self) -> i16 {
        self.time_speed
    }

    pub fn precipitation(&self) -> i32 {
        self.precipitation
    }

    /// The current wind (C4Weather::Wind). State advances in `advance_frame`
    /// with the C++ tick gates; the frame parameter is kept for caller
    /// compatibility but the value is the mutable wind state, matching
    /// `C4Weather::GetWind` minus the position-dependent tunnel check.
    pub fn wind_force(&self, _frame: u64) -> i32 {
        self.wind
    }

    fn normalize_time_of_day(time_of_day: i32) -> u16 {
        let cycle = i32::from(Self::TIME_CYCLE);
        time_of_day.rem_euclid(cycle) as u16
    }

    fn clamp_time_speed(time_speed: i32) -> i16 {
        let max = i32::from(Self::MAX_TIME_SPEED);
        time_speed.clamp(-max, max) as i16
    }

    fn advance_time_of_day(&mut self) {
        if self.time_speed == 0 {
            return;
        }
        let next = (i32::from(self.time_of_day) + i32::from(self.time_speed))
            .rem_euclid(i32::from(Self::TIME_CYCLE));
        self.time_of_day = next as u16;
    }

    /// C4Weather::Execute's season advance (C4Weather.cpp:77-85):
    /// `SeasonDelay += YearSpeed`; at 200 the delay resets to ZERO (not
    /// modulo) and the season steps exactly once, wrapping to the scenario
    /// `StartSeason.Min` only when it exceeds `StartSeason.Max`.
    fn update_season(&mut self) -> bool {
        self.season_delay = self.season_delay.saturating_add(self.year_speed);
        if self.season_delay >= 200 {
            self.season_delay = 0;
            self.season += 1;
            if self.season > self.season_max {
                self.season = self.season_min;
            }
            true
        } else {
            false
        }
    }

    /// C4Weather::Execute's temperature step (C4Weather.cpp:88-93): every
    /// Tick35 the temperature moves one degree toward
    /// `Climate - int32(TemperatureRange * cos(6.28 * Season / 100.0))` —
    /// a double-precision cosine of the LITERAL 6.28 (not tau) with the
    /// C++ truncating int cast, and no TemperatureRange gate.
    // The 6.28 is the C++ oracle's own approximation (C4Weather.cpp:90) —
    // substituting the real TAU would desync the temperature by a degree.
    #[allow(clippy::approx_constant)]
    fn update_temperature_from_season(&mut self) {
        let delta = (f64::from(self.temperature_range)
            * (6.28 * f64::from(self.season) / 100.0).cos()) as i32;
        let target = self.climate.saturating_sub(delta);
        if self.temperature < target {
            self.temperature = self.temperature.saturating_add(1);
        } else if self.temperature > target {
            self.temperature = self.temperature.saturating_sub(1);
        }
    }

    fn update_precipitation_runtime(&mut self) {
        if self.precipitation_strength != 0 {
            self.precipitation = self.precipitation_strength;
        }
    }

    /// Normalize deprecated JSON-fixture scheduler fields. C++ weather does
    /// not have this scheduler, and its persisted Wind/TargetWind plus the
    /// scenario Wind.Std (`base_wind`) must remain untouched here.
    pub fn refresh_runtime_fields(&mut self) {
        if self.wind_update_interval == 0 && self.wind_variation > 0 {
            self.wind_update_interval = Self::default_wind_update_interval(self.wind_period);
        }

        if self.wind_variation == 0 {
            self.wind_update_interval = 0;
            self.wind_update_timer = 0;
        } else {
            if self.wind_update_interval == 0 {
                self.wind_update_interval = 1;
            }
            if self.wind_update_timer >= self.wind_update_interval {
                self.wind_update_timer %= self.wind_update_interval;
            }
        }
    }
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self::new(0)
    }
}

fn default_fow_resolution() -> i32 {
    DEFAULT_FOW_RESOLUTION
}

fn is_default_fow_resolution(value: &i32) -> bool {
    *value == DEFAULT_FOW_RESOLUTION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentFrame {
    pub settings: EnvironmentSettings,
    pub wind_force: i32,
    pub ambient_temperature: i32,
    #[serde(default)]
    pub precipitation: i32,
    #[serde(default)]
    pub sky_color: Option<RgbColor>,
    /// Presentation state is recorded with the simulation frame but remains
    /// intentionally absent from C4ControlSyncCheck.
    #[serde(default, skip_serializing_if = "GammaControlState::is_default")]
    pub gamma: GammaControlState,
    /// Packed C4 `Game.C4S.Game.FoWColor` used only by viewport presentation.
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub fow_color: u32,
    /// `Game.C4S.Landscape.FoWRes`; both modulation-map axes use this value.
    #[serde(
        default = "default_fow_resolution",
        skip_serializing_if = "is_default_fow_resolution"
    )]
    pub fow_resolution: i32,
}

impl Default for EnvironmentFrame {
    fn default() -> Self {
        Self {
            settings: EnvironmentSettings::default(),
            wind_force: 0,
            ambient_temperature: 0,
            precipitation: 0,
            sky_color: None,
            gamma: GammaControlState::default(),
            fow_color: 0,
            fow_resolution: default_fow_resolution(),
        }
    }
}

fn default_wind_min() -> i32 {
    -100
}

fn default_wind_max() -> i32 {
    100
}

fn bound_by_ordered(value: i32, minimum: i32, maximum: i32) -> i32 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

fn default_owner() -> i32 {
    OWNER_NONE
}

fn default_alive() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn minus_one_i32() -> i32 {
    -1
}

fn default_construction() -> i32 {
    FULL_CON
}

fn default_contact_density() -> i32 {
    CONTACT_DENSITY_SOLID
}

fn is_default_contact_density(value: &i32) -> bool {
    *value == CONTACT_DENSITY_SOLID
}

/// One menu entry (C4MenuItem, C4Menu.h:60-101) — the sim-observable core:
/// the composed left/right-click commands (FnAddMenuItem, C4Script.cpp:
/// 1556-1597), count, item id, selectability (= command non-empty,
/// C4Script.cpp:1729) and the C4MN_Add_PassValue payload. Symbols and
/// captions are presentation; the caption is kept for the menu UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ObjectMenuSymbol {
    /// The item's definition picture (`C4MenuItem::SetSymbol`,
    /// C4Menu.cpp:119-128).
    #[default]
    Definition,
    /// First carried definition picture plus `fctHand` phase 0
    /// (`C4ObjectMenu.cpp:347-355`).
    Put,
    /// `DrawMenuSymbol(C4MN_Buy, ...)` (C4Menu.cpp:61-65).
    Buy { owner: i32 },
    /// `DrawMenuSymbol(C4MN_Sell, ...)` (C4Menu.cpp:66-70).
    Sell { owner: i32 },
    /// Target picture plus OKCancel phase (0,1) (C4ObjectMenu.cpp:405-414).
    Info,
    /// Standalone OKCancel phase (0,1) used by the internal C4MN_Info
    /// title facet (C4Object.cpp:2008-2012).
    InfoTitle,
    /// `fctExit` (C4ObjectMenu.cpp:422-427).
    Exit,
    /// `GfxR->fctConstruction` for the context-menu BuildInfo row
    /// (C4ObjectMenu.cpp:401-408).
    Construction,
}

impl ObjectMenuSymbol {
    fn is_definition(&self) -> bool {
        *self == Self::Definition
    }
}

/// Optional lower-strip presentation selected by `C4Menu::Extra`
/// (`C4Menu.h:47-56`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ObjectMenuExtra {
    #[default]
    None,
    /// `C4MN_Extra_Components`: show the selected item's cached build
    /// requirements (`C4Menu.cpp:895-898`).
    Components,
    /// `C4MN_Extra_Value`: show the selected item's value beside the
    /// wealth symbol (`C4Menu.cpp:843-907`).
    Value,
    MagicValue,
    Info,
    ComponentsMagic,
    LiveMagicValue,
    ComponentsLiveMagic,
    /// A nonstandard numeric value still reserves C++'s lower strip even
    /// though `C4Menu::DrawElement` has no matching draw case.
    Unknown(i32),
}

impl ObjectMenuExtra {
    fn is_none(&self) -> bool {
        *self == Self::None
    }

    pub(crate) fn from_legacy(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Components,
            2 => Self::Value,
            3 => Self::MagicValue,
            4 => Self::Info,
            5 => Self::ComponentsMagic,
            6 => Self::LiveMagicValue,
            7 => Self::ComponentsLiveMagic,
            value => Self::Unknown(value),
        }
    }
}

/// One cached `C4MenuItem::Components` entry. C++ resolves this when the
/// item is constructed, preserving C4IDList order for the footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMenuComponent {
    pub definition_id: String,
    pub count: i32,
}

/// Presentation source selected by `C4MN_Add_Img*` while AddMenuItem builds
/// the row symbol (C4Script.cpp:1595-1716).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ObjectMenuImage {
    None,
    #[default]
    Definition,
    Rank {
        rank: i32,
    },
    Indexed {
        index: i32,
    },
    ObjectRank {
        object: ObjectId,
    },
    Object {
        object: ObjectId,
    },
    TextSpec {
        spec: String,
        color: u32,
    },
    Color {
        color: u32,
    },
    IndexedColor {
        index: i32,
        color: u32,
    },
}

/// The object-picture inputs captured while `AddMenuItem` constructs an
/// Object/ObjectRank symbol. C++ renders that symbol into the menu item at
/// add time, so later graphics, color, overlay, or deletion changes must not
/// affect the row (`C4Script.cpp:1617-1678`; `C4Menu.cpp:388-398`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectMenuPictureSnapshot {
    pub definition_id: DefinitionId,
    /// `C4Menu::GetSymbolSize()` at AddMenuItem time (35, or 64 for
    /// Dialog). ObjectRank Context also observes the pre-layout 35px item
    /// height for the normal immediate-add path.
    #[serde(default = "default_object_menu_symbol_size")]
    pub symbol_size: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub blit_mode: u32,
    #[serde(default)]
    pub color: u32,
    #[serde(default)]
    pub color_modulation: u32,
    #[serde(default)]
    pub picture_rect: DefinitionRect,
    /// `pGfxObj->Info->Rank`, present only when ObjectRank found crew info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<i32>,
}

fn default_object_menu_symbol_size() -> i32 {
    35
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectMenuItem {
    pub caption: String,
    /// Tooltip text shown after C4MN_InfoCaption_Delay.
    #[serde(default)]
    pub info_caption: String,
    pub command: String,
    pub command2: String,
    /// C4MN_Item_NoCount (12345678) when the script passed no count
    /// (C4Script.cpp:1726).
    pub count: i32,
    /// Canonical typed-C4ID storage for idItem ("NONE" for no id).
    /// Render through `clonk_script::c4_id_text` only at presentation/source
    /// boundaries; the storage spelling distinguishes equal-looking IDs.
    pub item_id: String,
    /// Presentation recipe for symbols that are not a plain definition
    /// picture. The default preserves existing script-created menu state.
    #[serde(default, skip_serializing_if = "ObjectMenuSymbol::is_definition")]
    pub symbol: ObjectMenuSymbol,
    /// Script-selected symbol recipe. Internal object menus normally retain
    /// the default definition/object picture path above.
    #[serde(default, skip_serializing_if = "ObjectMenuImage::is_definition")]
    pub image: ObjectMenuImage,
    /// The `pDef` selected at add time. Unknown `idItem` values fall back to
    /// the menu object's definition while the original item ID remains in
    /// commands and `item_id` (`C4Script.cpp:1481-1483`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_definition_id: Option<DefinitionId>,
    /// Cached inputs for Object/ObjectRank script image recipes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_snapshot: Option<ObjectMenuPictureSnapshot>,
    /// Object whose `Picture2Facet` supplied this row's symbol during an
    /// internal object-menu refill. This is presentation provenance, not
    /// `C4MenuItem::Object`: Sell rows pass a null item object after copying
    /// the representative's picture (C4ObjectMenu.cpp:246-271).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_object: Option<ObjectId>,
    /// Cached component requirements for `C4MN_Extra_Components` and its
    /// magic variants (`C4MenuItem`, C4Menu.h:91; C4Menu.cpp:92-97).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ObjectMenuComponent>,
    pub selectable: bool,
    /// Some(value) iff C4MN_Add_PassValue was set (C4Script.cpp:1549-1554).
    pub value: Option<i32>,
    /// Dialog text byte offset. `-1` means fully shown; `0..` is the raw
    /// caption byte position reached by C4MenuItem::DoTextProgress.
    #[serde(default = "minus_one_i32", skip_serializing_if = "i32_is_minus_one")]
    pub text_display_progress: i32,
}

/// Revalidated source for a classic menu-origin construction drag.
///
/// `definition_id` always comes from `C4MenuItem::id` (`item_id` here),
/// never from the presentation-definition fallback. `definition_c4id` is
/// ready for `PlayerCommandControlData::data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMenuConstructionDrag {
    pub menu_object_id: ObjectId,
    pub definition_id: DefinitionId,
    pub definition_c4id: i32,
}

impl ObjectMenuImage {
    fn is_definition(&self) -> bool {
        matches!(self, Self::Definition)
    }
}

fn menu_progress_markup_len(text: &[u8]) -> Option<usize> {
    if text.first() != Some(&b'<') {
        return None;
    }
    let close = text.get(1..)?.iter().position(|byte| *byte == b'>')? + 1;
    let tag = text.get(1..close)?;
    if tag.len() > 49 {
        return None;
    }
    let space = tag.iter().position(|byte| *byte == b' ');
    let (name, parameters) = match space {
        Some(space) => (&tag[..space], Some(&tag[space + 1..])),
        None => (tag, None),
    };
    let recognized = if name.first() == Some(&b'/') {
        parameters.is_none()
    } else if name == b"i" {
        parameters.is_none()
    } else if name == b"c" {
        parameters.is_some_and(|parameters| parameters.len() <= 8)
    } else {
        false
    };
    recognized.then_some(close + 1)
}

impl ObjectMenuItem {
    fn do_text_progress(&mut self, amount: &mut i32) {
        if self.text_display_progress < 0 {
            return;
        }
        if self.selectable || self.caption.is_empty() {
            self.text_display_progress = -1;
            return;
        }
        let bytes = clonk_script::c4_string_bytes(&self.caption);
        let mut position = usize::try_from(self.text_display_progress)
            .unwrap_or_default()
            .min(bytes.len());
        while *amount != 0 && position < bytes.len() {
            while let Some(length) = menu_progress_markup_len(&bytes[position..]) {
                position += length;
                if position >= bytes.len() {
                    break;
                }
            }
            if position >= bytes.len() {
                break;
            }
            *amount -= 1;
            position += 1;
        }
        self.text_display_progress = if position >= bytes.len() {
            -1
        } else {
            position as i32
        };
    }
}

/// `C4MenuItem` copies InfoCaption through a `C4MaxTitle` buffer, then
/// normalizes LF/CR for the menu text renderer (C4Menu.cpp:76-91).
const C4_MAX_TITLE: usize = 512;

pub(crate) fn normalize_menu_info_caption(text: impl Into<String>) -> String {
    let text = text.into();
    let mut bytes = clonk_script::c4_string_bytes(&text);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes.truncate(C4_MAX_TITLE);
    for byte in &mut bytes {
        *byte = match *byte {
            b'\n' => b' ',
            b'\r' => b'|',
            other => other,
        };
    }
    clonk_script::c4_string_from_bytes(&bytes)
}

#[cfg(test)]
#[path = "lib_tests/object_menu_byte_tests.rs"]
mod object_menu_byte_tests;

/// A script-created object menu (C4ObjectMenu; FnCreateMenu →
/// C4ObjectMenu::Init, C4ObjectMenu.cpp:86-91): the minimal state scripts
/// can observe — GetMenu reads `identification`, SelectMenuItem moves
/// `selection`, MenuQueryCancel/OnMenuSelection dispatch on the callback
/// type captured at initialization (CB_Object or CB_Scenario).
/// C++ never persists menus in Objects.txt, so this state is runtime-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMenuState {
    /// C4Menu::Caption from CreateMenu's szEmpty argument; the app uses
    /// it as the menu title until Normal-style selection captions replace
    /// it (C4Menu.cpp:351-358, 577-584).
    #[serde(default)]
    pub caption: String,
    /// Definition picture used as the wooden title-bar icon. This is the
    /// CreateMenu symbol even when idMenuID overrides `identification`.
    #[serde(default)]
    pub symbol_id: String,
    /// Presentation recipe for title symbols composed by `DrawMenuSymbol`
    /// instead of a plain definition picture (`C4Menu.cpp:42-82`).
    #[serde(default, skip_serializing_if = "ObjectMenuSymbol::is_definition")]
    pub title_symbol: ObjectMenuSymbol,
    /// C4Menu::Identification: idMenuID if given, else the symbol id
    /// (C4Script.cpp:1452). Kept as the raw script value (C4ID or int)
    /// so GetMenu returns exactly what the script compares against.
    pub identification: Value,
    /// C4Menu::Style (& C4MN_Style_BaseMask, C4Menu.cpp:359).
    pub style: i32,
    /// `C4MN_Style_EqualItemHeight` is an independent style bit consumed
    /// after the base style is masked (C4Menu.cpp:359-366).
    #[serde(default, skip_serializing_if = "is_false")]
    pub equal_item_height: bool,
    /// C4Menu::Permanent (SetPermanent, C4Menu.cpp:942-945).
    pub permanent: bool,
    /// Requested top-left in logical viewport-local pixels. `Some` models
    /// `C4MN_Align_Free` plus `C4Menu::SetLocation`; `None` retains the
    /// style's default alignment. The app clamps it after menu sizing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Vector2>,
    /// Process-local allocation identity used only to distinguish a fresh
    /// menu from an in-place refill in presentation caches.
    #[doc(hidden)]
    #[serde(skip, default = "crate::direct_com::next_object_menu_runtime_id")]
    pub runtime_id: u64,
    /// Optional lower-strip payload selected by `C4Menu::SetExtra`.
    #[serde(default, skip_serializing_if = "ObjectMenuExtra::is_none")]
    pub extra: ObjectMenuExtra,
    /// `C4Menu::ExtraData`, used by the magic-value footer variants.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub extra_data: i32,
    /// Runtime identity of the internal C4ObjectMenu allocation while a
    /// refill is in progress. Nested Activate/Get initialization reuses it;
    /// script CreateMenu replacement does not. C++ keeps the equivalent in
    /// the menu/window objects, not script-visible ExtraData.
    #[doc(hidden)]
    #[serde(skip)]
    pub internal_refill_token: u64,
    /// C4Menu::Selection (-1 = none, C4Menu.cpp:284).
    pub selection: i32,
    /// C4ObjectMenu::UserMenu — script menus always pass fUserMenu=true
    /// (C4Script.cpp:1451), enabling MenuQueryCancel/OnMenuSelection.
    pub user_menu: bool,
    /// C4ObjectMenu::Object — the callback target for CB_Object. Pointer
    /// clearing may turn this into None without changing the callback type.
    pub command_object: Option<ObjectId>,
    /// C4ObjectMenu::eCallbackType == CB_Scenario. This is captured by
    /// LocalInit and deliberately survives command-object pointer clearing
    /// (C4ObjectMenu.cpp:78-84; C4ObjectMenu::ClearPointers).
    #[serde(default, skip_serializing_if = "is_false")]
    pub scenario_callbacks: bool,
    /// C4ObjectMenu::RefillObject. Internal object menus retain this exact
    /// target because an explicit Activate/Get target need not be the
    /// command object's current container (C4ObjectMenu.h:61-71).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refill_object: Option<ObjectId>,
    /// Last target-content count observed by C4ObjectMenu::Execute. A
    /// mismatch requests an immediate refill; otherwise the 35-tick timer
    /// supplies the periodic refill (C4ObjectMenu.cpp:448-459).
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub refill_object_contents_count: i32,
    pub items: Vec<ObjectMenuItem>,
    /// C4Menu::Columns — 0 = auto layout (C4Menu::Default, C4Menu.cpp:299);
    /// script-set via SetMenuSize (C4Menu::SetSize, C4Menu.cpp:635-640).
    #[serde(default)]
    pub columns: i32,
    /// C4Menu::Lines — 0 = auto layout; see `columns`.
    #[serde(default)]
    pub lines: i32,
    /// C4Menu::fTextProgressing. The shared budget is immediately
    /// distributed into each item's `text_display_progress`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub text_progressing: bool,
    /// SetMenuDecoration's immediate FrameDecoration::SetByDef snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoration: Option<ObjectMenuFrameDecoration>,
}

impl PartialEq for ObjectMenuState {
    fn eq(&self, other: &Self) -> bool {
        // Allocation/refill tokens are process-local mechanics, not part of
        // the script-visible or serialized menu state.
        self.caption == other.caption
            && self.symbol_id == other.symbol_id
            && self.title_symbol == other.title_symbol
            && self.identification == other.identification
            && self.style == other.style
            && self.equal_item_height == other.equal_item_height
            && self.permanent == other.permanent
            && self.location == other.location
            && self.extra == other.extra
            && self.extra_data == other.extra_data
            && self.selection == other.selection
            && self.user_menu == other.user_menu
            && self.command_object == other.command_object
            && self.scenario_callbacks == other.scenario_callbacks
            && self.refill_object == other.refill_object
            && self.refill_object_contents_count == other.refill_object_contents_count
            && self.items == other.items
            && self.columns == other.columns
            && self.lines == other.lines
            && self.text_progressing == other.text_progressing
            && self.decoration == other.decoration
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMenuFrameDecoration {
    pub source_definition: String,
    pub background_color: u32,
    pub border_top: i32,
    pub border_left: i32,
    pub border_right: i32,
    pub border_bottom: i32,
    pub top: Option<DefinitionActionFacet>,
    pub top_right: Option<DefinitionActionFacet>,
    pub right: Option<DefinitionActionFacet>,
    pub bottom_right: Option<DefinitionActionFacet>,
    pub bottom: Option<DefinitionActionFacet>,
    pub bottom_left: Option<DefinitionActionFacet>,
    pub left: Option<DefinitionActionFacet>,
    pub top_left: Option<DefinitionActionFacet>,
}

impl ObjectMenuState {
    pub(crate) fn set_text_progress(&mut self, mut amount: i32, add: bool) -> bool {
        if add {
            if !self.text_progressing {
                return false;
            }
        } else {
            self.text_progressing = amount >= 0;
        }

        let first_text = usize::from(
            self.items
                .first()
                .is_some_and(|item| item.caption.is_empty()),
        );
        let mut unfinished = false;
        for item in self.items.iter_mut().skip(first_text) {
            if !self.text_progressing {
                item.text_display_progress = -1;
                continue;
            }
            if !add {
                item.text_display_progress = 0;
            }
            if amount != 0 {
                item.do_text_progress(&mut amount);
            }
            unfinished |= item.text_display_progress > -1;
        }
        self.text_progressing = unfinished;
        true
    }

    pub(crate) fn reveal_text(&mut self) {
        let _ = self.set_text_progress(-1, false);
    }
}

/// C4Shape attach bookkeeping (`AttachMat`/`iAttachX`/`iAttachY`/
/// `iAttachVtx`, C4Shape.h:52-55): `AttachMat` resets to MNone at the top
/// of every `C4Shape::Attach` while the position/vertex fields only
/// overwrite on a successful attach (C4Shape.cpp:165-270). The movement
/// rotation step restores the WHOLE record on contact undo (`Shape =
/// lshape`, C4Movement.cpp:395-417). A dense pixel always maps to a real
/// material (solid masks paint MVehic), so `AttachMat != MNone` is
/// exactly "the last attach succeeded" — tracked as `mat_valid`. The
/// cached MVehic identity must survive independently of later landscape
/// changes because `C4Game::ShakeObjects` reads it directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeAttachRecord {
    /// `Shape.AttachMat != MNone`.
    pub mat_valid: bool,
    /// `Shape.AttachMat == MVehic` at the successful attachment probe.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mat_vehicle: bool,
    /// Absolute attachment position (`iAttachX`/`iAttachY`).
    pub x: i32,
    pub y: i32,
    /// The vertex index that attached (`iAttachVtx`).
    pub vtx: i32,
}

impl ShapeAttachRecord {
    /// serde gate: the all-default record is C4Shape's initial state
    /// (C4Shape.cpp:34) — skip it so recorded snapshots stay byte-stable.
    pub fn is_unattached(&self) -> bool {
        *self == Self::default()
    }
}

fn u32_is_zero(value: &u32) -> bool {
    *value == 0
}

fn i32_is_zero(value: &i32) -> bool {
    *value == 0
}

fn legacy_c_string_bytes(mut bytes: Vec<u8>) -> Vec<u8> {
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes
}

fn default_use_fair_crew() -> bool {
    true
}

fn default_fair_crew_strength() -> i32 {
    1_000
}

fn default_message_board_commands() -> Vec<InitialNetworkMessageBoardCommand> {
    vec![InitialNetworkMessageBoardCommand::speed()]
}

pub(crate) const DEFAULT_MUSIC_LEVEL: u8 = 100;

const fn default_music_level() -> u8 {
    DEFAULT_MUSIC_LEVEL
}

fn music_level_is_default(level: &u8) -> bool {
    *level == DEFAULT_MUSIC_LEVEL
}

fn message_board_commands_are_default(commands: &[InitialNetworkMessageBoardCommand]) -> bool {
    commands == [InitialNetworkMessageBoardCommand::speed()]
}

fn i32_is_minus_one(value: &i32) -> bool {
    *value == -1
}

const fn default_last_attach_movement_frame() -> i32 {
    -1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectState {
    /// C4Object::CustomName: Some overrides the crew-info/definition name;
    /// None uses the fallback chain (C4Object.cpp:2103-2116).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub custom_name: Option<String>,
    pub position: Vector2,
    pub velocity: Vector2,
    /// Transient script-call mirror of fix_x/fix_y. Like
    /// `script_fixed_velocity`, this is never persisted.
    #[serde(skip)]
    pub script_fixed_position: Option<FixedVec2>,
    /// Transient script-call mirror of the object's TRUE sub-pixel dirs:
    /// C++ Fn(Get|Set)XDir/YDir read/write the LIVE C4Fixed xdir/ydir
    /// (C4Script.cpp:697-732, :1160-1180) — an INT-seeded scope reads a
    /// 0.4 px/f drift as 0 and mis-takes script guards (the GoldRush
    /// fish's `if (GetXDir() > 0) SetXDir(0)`, f100 wall). Set only on
    /// script-call snapshots (`Object::script_state_snapshot`); never
    /// persisted.
    #[serde(skip)]
    pub script_fixed_velocity: Option<FixedVec2>,
    /// Transient script-call mirror of the object's TRUE angular velocity.
    /// Like the fixed position/velocity mirrors, this is never persisted.
    #[serde(skip)]
    pub script_rotation_velocity: Option<C4Fixed>,
    /// Transient script-call mirror of the raw 16.16 rotation accumulator
    /// (`C4Object::fix_r`). GetObjectVal reflects this independently of the
    /// whole-degree `Rotation` field.
    #[serde(skip)]
    pub script_fixed_rotation: Option<C4Fixed>,
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    /// C4Object::NeedEnergy: the persistent insufficient-power marker set
    /// by EnergyCheck and energy-consuming actions (C4Object.cpp:118,
    /// 1478, 4739-4752; persisted at :2805).
    #[serde(default)]
    pub need_energy: bool,
    #[serde(default)]
    pub damage: i32,
    #[serde(default)]
    pub magic_energy: i32,
    #[serde(default)]
    pub magic_capacity: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
    pub action: ActionState,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    /// Complete C4Shape slot storage. Public snapshots expose only the active
    /// `vertices` prefix; engine persistence carries this separately.
    #[serde(skip)]
    shape_vertices: ShapeVertexBuffer,
    /// Live C4Shape::ContactDensity. SetContactDensity mutates this
    /// per-object value and C4Shape::CompileFunc persists it.
    #[serde(
        default = "default_contact_density",
        skip_serializing_if = "is_default_contact_density"
    )]
    pub contact_density: i32,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub layer: Option<ObjectId>,
    /// C4Object::Visibility (`Visibility=` in Objects.txt). Zero is
    /// `VIS_All`; nonzero values are the VIS_* bitmask consumed by
    /// C4Object::IsVisible (C4Object.cpp:5600-5629).
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub visibility: i32,
    /// C4Object::BlitMode, distinct from SetGraphics base/overlay modes.
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub blit_mode: u32,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    /// Runtime identity of this object's current link in its container's
    /// `C4ObjectList`. Every successful Enter allocates a fresh link, even
    /// when an Exit+Enter callback restores the same final container/slot.
    #[serde(skip)]
    pub(crate) contents_link_generation: u64,
    #[serde(default)]
    pub components: HashMap<DefinitionId, i32>,
    /// C4Object::Component is a C4IDList: indexed access follows insertion
    /// order independently of the count map, and zero-count entries remain
    /// present (C4IDList.cpp:38-45,85-103).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_order: Vec<DefinitionId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    /// C4Object::Controller (C4Object.h:127): the player whose action
    /// chain caused this object — kill/cause tracing. NO_OWNER before
    /// Init (C4Object.cpp:86) and as the Objects.txt compile default
    /// (C4Object.cpp:2739); Init seeds it from the explicit controller or
    /// the owner (C4Object.cpp:162).
    #[serde(default = "default_owner")]
    pub controller: i32,
    #[serde(default = "default_category")]
    pub category: i32,
    #[serde(default)]
    pub crew_member: bool,
    /// C4Object::PlrViewRange, persisted by Objects.txt and initialized to
    /// zero. Joining a player crew raises a zero range to the classic 500.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub plr_view_range: i32,
    /// C4Object::Select: the authoritative crew-selection bit persisted as
    /// `Selected` independently of C4Player::Cursor (C4Object.h:153;
    /// C4Object.cpp:2800).
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    /// C4Object::CrewDisabled (FnSetCrewEnabled, C4Script.cpp:4814-4836).
    #[serde(default)]
    pub crew_disabled: bool,
    #[serde(default = "default_alive")]
    pub alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default)]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    /// Per-object storage for script-level local variables
    /// These are initialized to nil in Construction() and persist across all function calls
    #[serde(default)]
    pub local_vars: HashMap<String, Value>,
    /// Cached liquid flag (C4Object::InLiquid, C4Object.h:156): loaded from
    /// Objects.txt (default false, C4Object.cpp:2775), updated inside
    /// movement (DoMovement, C4Movement.cpp:443-460), cleared on container
    /// Exit (C4Object.cpp:1528). FnInLiquid reads THIS flag, not the
    /// landscape (C4Script.cpp:1864-1868).
    #[serde(default)]
    pub in_liquid: bool,
    /// C4Object::Mobile (C4Object.h:152): set at Init for nonzero-dir
    /// non-StaticBack spawns (C4Object.cpp:183-185), by SetXDir/SetYDir/
    /// SetRDir and the action procedures, cleared by ExecMovement when all
    /// dirs reach zero (C4Movement.cpp:572) and re-set by the Tick10
    /// gravity-mobilization pulse (:581-586). Only Mobile objects run
    /// DoMovement or idle gravity. Loaded from Objects.txt `Mobile=`
    /// (default false, C4Object.cpp:2772).
    #[serde(default)]
    pub mobile: bool,
    /// Per-object SolidMask rect (C4Object::SolidMask; SetSolidMask,
    /// C4Script.cpp:271-278). None = the definition's mask; a zero-area
    /// rect = mask OFF (opened gates save SolidMask=0,0,0,0,0,0).
    #[serde(default)]
    pub solid_mask_override: Option<DefinitionTargetRect>,
    /// The Def TimerCall counter (C4Object::Timer, C4Object.cpp:1085-1091):
    /// ++ every Execute, fires Def->TimerCall and resets at Def->Timer.
    /// Saved mid-cycle in Objects.txt (default 0, C4Object.cpp:2738).
    #[serde(default)]
    pub timer: i32,
    /// Script-set extra mass (C4Object::OwnMass, C4Object.cpp:94): SetMass
    /// stores iValue - Def->Mass here; Mass = max((Def->Mass + OwnMass) *
    /// Con / FullCon, 1) (UpdateMass, C4Object.cpp:497-500).
    #[serde(default)]
    pub own_mass: i32,
    /// Burning state (C4Object::OnFire, C4Object.h:205). Set by Incinerate
    /// via the fire effect start (C4Effect.cpp:633); drives OCF_OnFire and
    /// the per-frame ExecFire burning.
    #[serde(default)]
    pub on_fire: bool,
    /// Fire animation phase 0..MaxFirePhase (C4Object::FirePhase; initialized
    /// to Random(MaxFirePhase) at fire start, C4Effect.cpp:634 — a synced
    /// draw).
    #[serde(default)]
    pub fire_phase: i32,
    /// Player that caused the fire (the fire effect's CausedBy var, read by
    /// C4Object::GetFireCausePlr for contact-incineration attribution).
    #[serde(default = "default_owner")]
    pub fire_caused_by: i32,
    /// `C4ObjectInfo::Physical` surrogate for crew members until the info
    /// model lands — cloned lazily from the definition physicals on first
    /// training/permanent write. None = read the definition physicals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_physical: Option<PhysicalInfo>,
    /// `PhysicalTemporary`/`TemporaryPhysical` (C4Object.h): the script-set
    /// temporary physicals, taking precedence in GetPhysical
    /// (C4Object.cpp:2118-2134). None = temporary mode off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_physical: Option<PhysicalInfo>,
    /// `C4TempPhysicalInfo::Changes` (C4InfoCore.h:113): PHYS_StackTemporary
    /// registrations as (physical name, previous value), newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_changes: Vec<(String, i32)>,
    /// `C4Object::Breath` on the raw 0..=Physical.Breath scale, filled from
    /// the physicals at birth (C4Object.cpp:193).
    #[serde(default)]
    pub breath: i32,
    /// EntranceStatus flag toggled by SetEntrance (C4Script.cpp:690-695).
    #[serde(default)]
    pub entrance_status: bool,
    /// The object's script menu (C4Object::Menu; FnCreateMenu,
    /// C4Script.cpp:1426-1459). None = no menu. Runtime-only in C++ too
    /// (Objects.txt has no menu section).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<ObjectMenuState>,
    /// Object color from SetColorDw (C4Script.cpp:3661-3668, C4Object Color).
    #[serde(default)]
    pub color: u32,
    /// Object drawing modulation (`C4Object::ColorMod`). Zero disables
    /// modulation; otherwise the raw C4 color is applied at draw time.
    #[serde(default)]
    pub color_modulation: u32,
    /// Per-object inventory/menu picture facet (`C4Object::PictureRect`). A
    /// zero width selects the definition picture, but the raw zero rect still
    /// participates in picture-stack equality (C4Object.cpp:3123-3127,
    /// 6173-6191).
    #[serde(default)]
    pub picture_rect: DefinitionRect,
    /// Per-object shape rectangle from SetShape (C4Script.cpp:5182-5196);
    /// None means the definition shape applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_override: Option<DefinitionRect>,
    /// The CACHED object character flags, like C++ `obj->OCF`: refreshed by
    /// the SetOCF events (spawn Init, updates, container changes, fire) and
    /// once per frame at Execute-start (C4Object.cpp:215,1058; C4Object.h:361)
    /// — readers consume this field, never a fresh compute.
    #[serde(default)]
    pub ocf: u32,
    /// C4Shape attach bookkeeping — see [`ShapeAttachRecord`]. Objects.txt
    /// persists `iAttachX`/`iAttachY`/`iAttachVtx`; `AttachMat` itself is
    /// runtime-only (C4Shape.cpp:511-514).
    #[serde(default, skip_serializing_if = "ShapeAttachRecord::is_unattached")]
    pub shape_attach: ShapeAttachRecord,
    /// `C4Object::Action.t_attach` as latched by ExecAction this frame
    /// (C4Object.cpp:4692 + the per-procedure ORs), mirrored from
    /// `frame_t_attach` for the script host seam (FnAdjustWalkRotation
    /// reads it, C4Script.cpp:5444).
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub t_attach: u32,
    /// `C4Object::NoCollectDelay` (C4Object.h:134): armed to 2 by
    /// ObjectComDrop (C4ObjectCom.cpp:668-669), counted down by DirectCom
    /// (C4Object.cpp:3371-3374) and SetCommand (C4Object.cpp:3939-3940);
    /// while nonzero it vetoes OCF_Collection (SetOCF, C4Object.cpp:598).
    /// Objects.txt "NoCollectDelay" (default 0, C4Object.cpp:2773).
    #[serde(default)]
    pub no_collect_delay: i32,
    /// `C4Object::Base` (C4Object.h:135): the player number this object is
    /// a home base for, NO_OWNER otherwise. Assigned by ExecBase's Tick10
    /// flag check (C4Object.cpp:1000-1018), cleared by its Tick35 lost-flag
    /// arm (:1024-1031); invalid players clear on load (C4Object.cpp:3161).
    #[serde(default = "default_owner")]
    pub base: i32,
}

#[derive(Debug, Clone)]
struct ActionChange {
    previous: ActionState,
    requested_name_change: bool,
}

impl ActionChange {
    fn should_record(&self, current: &ActionState) -> bool {
        self.requested_name_change
            || self.previous.name != current.name
            || self.previous.act_map_index != current.act_map_index
    }
}

#[derive(Debug, Clone, Default)]
struct ApplyDeltaOutcome {
    container_change: Option<(Option<ObjectId>, Option<ObjectId>)>,
    action_change: Option<ActionChange>,
    /// A nonzero energy reached 0 while alive — the caller runs
    /// AssignDeath (C4Object::DoEnergy, C4Object.cpp:1363).
    energy_died: bool,
}

/// The state a pending mid-call spawn would have once created — C++
/// CreateObject fully creates objects DURING the call (Game.CreateObject
/// -> NewObject), so nested `obj->Method()` calls on them need a callable
/// scope before the copy-out spawn happens. Mirrors `spawn_single`'s
/// defaults; the authoritative state is still built by the spawn, with
/// the nested outcome folding only the touched fields on top.
pub(crate) fn preview_spawn_state(
    position: Vector2,
    owner: i32,
    controller: i32,
    category: i32,
    construction: i32,
    contact_density: i32,
    vertices: Vec<ObjectVertex>,
) -> ObjectState {
    let shape_vertices = ShapeVertexBuffer::from_active(&vertices);
    ObjectState {
        custom_name: None,
        script_fixed_position: None,
        script_fixed_velocity: None,
        script_rotation_velocity: None,
        script_fixed_rotation: None,
        position,
        velocity: Vector2::ZERO,
        rotation: 0,
        shape_attach: ShapeAttachRecord::default(),
        t_attach: 0,
        no_collect_delay: 0,
        base: OWNER_NONE,
        energy: 0,
        need_energy: false,
        damage: 0,
        magic_energy: 0,
        magic_capacity: 0,
        construction: construction.clamp(0, FULL_CON),
        action: ActionState::new("Idle"),
        direction: Direction::default(),
        command_direction: CommandDirection::default(),
        effects: Vec::new(),
        vertices,
        shape_vertices,
        contact_density,
        container: None,
        layer: None,
        visibility: 0,
        blit_mode: 0,
        contents: Vec::new(),
        contents_link_generation: 0,
        components: HashMap::new(),
        component_order: Vec::new(),
        status: ObjectStatus::Normal,
        owner,
        controller,
        category,
        crew_member: false,
        plr_view_range: 0,
        selected: false,
        crew_disabled: false,
        alive: true,
        base_graphics: None,
        graphics_overlays: Vec::new(),
        draw_transform: None,
        local_vars: HashMap::new(),
        in_liquid: false,
        mobile: false,
        solid_mask_override: None,
        timer: 0,
        own_mass: 0,
        on_fire: false,
        fire_phase: 0,
        fire_caused_by: OWNER_NONE,
        info_physical: None,
        temporary_physical: None,
        physical_changes: Vec::new(),
        breath: 0,
        entrance_status: false,
        menu: None,
        color: 0,
        color_modulation: 0,
        picture_rect: DefinitionRect::default(),
        shape_override: None,
        ocf: OCF_NORMAL,
    }
}

pub(crate) fn preview_spawn_state_with_components(
    position: Vector2,
    owner: i32,
    controller: i32,
    category: i32,
    construction: i32,
    contact_density: i32,
    vertices: Vec<ObjectVertex>,
    definition_components: &[(DefinitionId, i32)],
) -> ObjectState {
    let mut state = preview_spawn_state(
        position,
        owner,
        controller,
        category,
        construction,
        contact_density,
        vertices,
    );
    state.component_order = definition_components
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    state.components = definition_component_counts(definition_components, construction);
    state
}

impl ObjectState {
    fn apply_delta(&mut self, delta: &ObjectDelta, library: &ActionLibrary) -> ApplyDeltaOutcome {
        let previous_container = self.container;
        let mut container_change = None;
        let mut action_change = None;
        if let Some(position) = delta.position {
            self.position = position;
        }
        if let Some(velocity) = delta.velocity {
            self.velocity = velocity;
        }
        if let Some(rotation) = delta.rotation {
            self.rotation = rotation.rem_euclid(360);
        }
        if let Some(custom_name) = &delta.custom_name {
            self.custom_name = custom_name.clone();
        }
        if let Some(layer) = delta.layer {
            self.layer = layer;
        }
        if let Some(visibility) = delta.visibility {
            self.visibility = visibility;
        }
        if let Some(blit_mode) = delta.blit_mode {
            self.blit_mode = blit_mode;
        }
        if let Some(picture_rect) = delta.picture_rect {
            self.picture_rect = picture_rect;
        }
        if let Some(color) = delta.color {
            self.color = color;
        }
        if let Some(color_modulation) = delta.color_modulation {
            self.color_modulation = color_modulation;
        }
        let mut energy_died = false;
        if let Some(energy) = delta.energy {
            energy_died =
                !delta.host_energy_death_checked && self.alive && self.energy != 0 && energy == 0;
            self.energy = energy;
        }
        if let Some(breath) = delta.breath {
            self.breath = breath;
        }
        if let Some(need_energy) = delta.need_energy {
            self.need_energy = need_energy;
        }
        if let Some(damage) = delta.damage {
            self.damage = damage.max(0);
        }
        if let Some((fire_caused_by, fire_phase)) = delta.fire {
            // Staged incinerate outcome — see ObjectUpdate::fire.
            self.on_fire = true;
            self.fire_caused_by = fire_caused_by;
            self.fire_phase = fire_phase;
        }
        if let Some(flag) = delta.fire_flag {
            // Bare SetOnFire write — see ObjectUpdate::fire_flag.
            self.on_fire = flag;
        }
        if let Some(magic_energy) = delta.magic_energy {
            self.magic_energy = magic_energy.max(0);
        }
        if let Some(magic_capacity) = delta.magic_capacity {
            self.magic_capacity = magic_capacity.max(0);
        }
        if let Some(construction) = delta.construction {
            self.construction = if delta.construction_via_docon {
                construction.max(0)
            } else {
                construction.clamp(0, FULL_CON)
            };
        }
        if let Some(contact_density) = delta.contact_density {
            self.contact_density = contact_density;
        }
        if let Some(direction) = delta.direction {
            self.direction = direction;
        }
        if let Some(command_direction) = delta.command_direction {
            self.command_direction = command_direction;
        }
        let raw_change_def_idle = delta.change_def.is_some()
            && delta.action.as_ref().is_some_and(|action| {
                action.force
                    && action.callbacks_dispatched
                    && action.name.as_deref() == Some("Idle")
            });
        if let Some(action) = &delta.action {
            let requested_name_change = action.name.is_some();
            let previous_action = self.action.clone();
            let result = if raw_change_def_idle {
                // C4Object::ChangeDef's post-callback `Action.Act=ActIdle`
                // writes the action slot only. Apply the already-staged
                // callback payload without ActionState's ordinary implicit
                // name-change resets or new-library reconciliation.
                if let Some(name) = &action.name {
                    self.action.name = name.clone();
                    self.action.act_map_index = None;
                }
                if delta.change_def_reset_action_time {
                    self.action.time = 0;
                }
                if let Some(phase) = action.phase {
                    self.action.phase = phase;
                    if action.ticks.is_none() {
                        self.action.ticks = 0;
                    }
                }
                if let Some(ticks) = action.ticks {
                    self.action.ticks = ticks;
                }
                if let Some(data) = action.data {
                    self.action.data = data;
                }
                if let Some(target) = action.target {
                    self.action.target = target;
                }
                if let Some(target2) = action.target2 {
                    self.action.target2 = target2;
                }
                ActionUpdateResult::Applied
            } else {
                self.action.apply_update_with_library(action, library)
            };
            if matches!(result, ActionUpdateResult::Applied) {
                action_change = Some(ActionChange {
                    previous: previous_action,
                    requested_name_change,
                });
            }
        } else {
            self.action.reconcile_with_library(library);
        }
        if let Some(vertices) = &delta.vertices {
            self.vertices = vertices.clone();
        }
        // C4ObjectList::ShiftContents (C4ObjectList.cpp:815-833): cyclic
        // rotation so the target becomes First — relative order preserved.
        // An id not (or no longer) in the list is a no-op.
        if let Some(new_front) = delta.contents_front {
            if let Some(index) = self.contents.iter().position(|id| *id == new_front) {
                self.contents.rotate_left(index);
            }
        }
        if let Some(overlays) = &delta.graphics_overlays {
            self.graphics_overlays = overlays.clone();
        }
        if let Some(transform) = &delta.draw_transform {
            self.draw_transform = *transform;
        }
        if let Some(base_graphics) = &delta.base_graphics {
            self.base_graphics = base_graphics.clone();
        }
        if let Some(owner) = delta.owner {
            self.owner = owner;
            // SetOwner "automatically updates controller"
            // (C4Object.cpp:5499-5500); an explicit SetController in the
            // same batch still wins below.
            self.controller = delta.controller.unwrap_or(owner);
        } else if let Some(controller) = delta.controller {
            self.controller = controller;
        }
        if let Some(base) = delta.base {
            self.base = base;
        }
        if let Some(category) = delta.category {
            self.category = category;
        }
        if let Some(own_mass) = delta.own_mass {
            self.own_mass = own_mass;
        }
        if let Some(crew_member) = delta.crew_member {
            self.crew_member = crew_member;
        }
        if let Some(plr_view_range) = delta.plr_view_range {
            self.plr_view_range = plr_view_range;
        }
        if let Some(selected) = delta.selected {
            self.selected = selected;
        }
        if let Some(crew_disabled) = delta.crew_disabled {
            self.crew_disabled = crew_disabled;
        }
        if let Some(rect) = delta.solid_mask_override {
            self.solid_mask_override = Some(rect);
        }
        if let Some(menu) = &delta.menu {
            self.menu = menu.clone();
        }
        if let Some(shape_override) = delta.shape_override {
            self.shape_override = shape_override;
        }
        if let Some(alive) = delta.alive {
            self.alive = alive;
        }
        if let Some(entrance_status) = delta.entrance_status {
            self.entrance_status = entrance_status;
        }
        if let Some(status) = delta.status {
            self.status = status;
        }
        if let Some(container) = delta.container {
            if self.container != container {
                self.container = container;
                container_change = Some((previous_container, self.container));
                // C4Object::Enter/Exit force-close the moving object's menu
                // (CloseMenu(true), C4Object.cpp:1555/:1594). A delta that
                // carries its OWN menu write already folded the correct
                // post-Enter/Exit state above (script scopes stage the close
                // at Enter/Exit call time) — only menu-less deltas (the
                // engine-internal collect/grab/enter movers) close here.
                if delta.menu.is_none() {
                    self.menu = None;
                }
            }
        }
        if let Some(components) = &delta.components {
            self.component_order = normalized_component_order(
                components,
                delta
                    .component_order
                    .clone()
                    .unwrap_or_else(|| self.component_order.clone()),
                &[],
            );
            self.components = components.clone();
        } else if let Some(component_order) = &delta.component_order {
            self.component_order =
                normalized_component_order(&self.components, component_order.clone(), &[]);
        }
        if let Some(local_vars) = &delta.local_vars {
            self.local_vars = local_vars.clone();
        }
        if let Some(physicals) = &delta.physicals {
            self.info_physical = physicals.info;
            self.temporary_physical = physicals.temporary;
            self.physical_changes = physicals.changes.clone();
        }

        if !raw_change_def_idle {
            self.action.reconcile_with_library(library);
        }
        ApplyDeltaOutcome {
            energy_died,
            container_change,
            action_change: action_change.filter(|change| change.should_record(&self.action)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ChangeDefContentsSort {
    pub container: ObjectId,
    pub category: i32,
    pub definition_id: DefinitionId,
    pub unsorted: bool,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct ObjectDelta {
    /// Non-field operation carried through callback/command aggregation.
    change_def: Option<String>,
    change_def_reinsert: bool,
    change_def_contents_sort: Option<ChangeDefContentsSort>,
    change_def_reset_action_time: bool,
    /// Some(Some(name)) sets C4Object::CustomName; Some(None) clears it.
    custom_name: Option<Option<String>>,
    /// Some(Some(object)) sets C4Object::pLayer; Some(None) clears it.
    layer: Option<Option<ObjectId>>,
    /// Exact C4Object compiler-cache overwrite. Typed pointer assignment does
    /// not touch this; literal-null resets and enumeration do.
    compiler_cache: Option<ObjectCompilerCache>,
    /// C4Object::Visibility overwrite.
    visibility: Option<i32>,
    /// C4Object::BlitMode overwrite.
    blit_mode: Option<u32>,
    /// C4Object::PictureRect overwrite (FnSetPicture).
    picture_rect: Option<DefinitionRect>,
    /// C4Object::Color overwrite (FnSetColorDw).
    color: Option<u32>,
    /// C4Object::ColorMod overwrite (FnSetClrModulation).
    color_modulation: Option<u32>,
    solid_mask_override: Option<DefinitionTargetRect>,
    /// Host-time C4SolidMask construction token. Callback copy-out may fold
    /// object updates and spawns in a different order than C++; carrying the
    /// token preserves the native linked-list age across that boundary.
    solid_mask_instance_sequence: Option<u64>,
    /// Script menu write-through (FnCreateMenu/FnCloseMenu et al.):
    /// Some(None) = closed, Some(Some(_)) = open/replaced.
    menu: Option<Option<ObjectMenuState>>,
    /// Some(Some(rect)) installs SetShape; Some(None) is UpdateShape's
    /// explicit restoration of the definition shape.
    shape_override: Option<Option<DefinitionRect>>,
    position: Option<Vector2>,
    velocity: Option<Vector2>,
    /// Sub-pixel velocity in 16.16 fixed-point. When present, this takes
    /// precedence over the whole-pixel `velocity` mirror so that script
    /// surfaces (e.g. `SetXDir`) can express fractional `C4Fixed` velocity
    /// exactly, matching C++ `pObj->xdir = itofix(n, prec)` (`C4Script.cpp:697`).
    fixed_velocity: Option<FixedVec2>,
    /// Component-only dir writes (FnSetXDir/FnSetYDir, C4Script.cpp:
    /// 697-732): C++ assigns ONE of xdir/ydir — the other keeps its
    /// full sub-pixel value (a whole-vector write from a script scope
    /// would quantize it through the int mirror).
    fixed_velocity_x: Option<C4Fixed>,
    fixed_velocity_y: Option<C4Fixed>,
    /// Explicit `C4Object::Mobile` overwrite for native object helpers whose
    /// velocity assignment does not share the script Set*Dir mobilization
    /// rule (notably C4Object::Fling's Tumble branch).
    mobile: Option<bool>,
    /// Explicit `C4Object::Action.t_attach` overwrite. Native Fling clears
    /// either CNAT_Bottom or the complete mask in the same frame.
    t_attach: Option<u32>,
    /// Live per-object C4Shape::ContactDensity overwrite.
    contact_density: Option<i32>,
    rotation: Option<i32>,
    /// Sub-pixel angular velocity (16.16 fixed-point degrees/frame) set by
    /// `SetRDir`. Mirrors C++ `pObj->rdir = itofix(n, prec)` (`C4Script.cpp:710`).
    rotation_velocity: Option<C4Fixed>,
    energy: Option<i32>,
    /// The energy write already evaluated C4Object::DoEnergy's synchronous
    /// zero-crossing predicate at its host-call site. This marker belongs to
    /// this specific write: a later merged energy overwrite replaces it.
    host_energy_death_checked: bool,
    /// C4Object::Breath overwrite staged by FnDoBreath.
    breath: Option<i32>,
    /// C4Object::NeedEnergy overwrite.
    need_energy: Option<bool>,
    /// Kill-trace mark riding an energy write (C4Object.cpp:1351-1353).
    energy_loss_cause: Option<i32>,
    /// Staged incinerate outcome `(caused_by, fire_phase)` — see
    /// `ObjectUpdate::fire`.
    fire: Option<(i32, i32)>,
    /// Bare OnFire flag write — see `ObjectUpdate::fire_flag`.
    fire_flag: Option<bool>,
    damage: Option<i32>,
    magic_energy: Option<i32>,
    magic_capacity: Option<i32>,
    construction: Option<i32>,
    /// The construction write came from C4Object::DoCon and therefore keeps
    /// DoCon-specific component/lifecycle semantics.
    construction_via_docon: bool,
    /// No later SetAction/position write resynchronized fix_x/fix_y after the
    /// staged DoCon bottom adjustment.
    construction_preserves_fixed_position: bool,
    resolved_docon_position: Option<Vector2>,
    resolved_docon_fixed_position: Option<FixedVec2>,
    direction: Option<Direction>,
    command_direction: Option<CommandDirection>,
    action: Option<ActionUpdate>,
    status: Option<ObjectStatus>,
    owner: Option<i32>,
    /// C4Object::Base overwrite used by FLAG/FlyBase SetOwner propagation.
    base: Option<i32>,
    /// FnSetController (C4Script.cpp:1322-1331) / SetOwner's automatic
    /// controller update (C4Object.cpp:5499-5500).
    controller: Option<i32>,
    category: Option<i32>,
    own_mass: Option<i32>,
    crew_member: Option<bool>,
    plr_view_range: Option<i32>,
    selected: Option<bool>,
    crew_disabled: Option<bool>,
    alive: Option<bool>,
    entrance_status: Option<bool>,
    container: Option<Option<ObjectId>>,
    /// Direct overwrite of the current `C4Object::Shape` vertex list. Unlike
    /// `vertices`, this does not enable `fOwnVertices` or replace the
    /// untransformed shape base.
    live_vertices: Option<Vec<ObjectVertex>>,
    /// Exact fixed-slot C4Shape overwrite, including dormant slots beyond
    /// VtxNum. This wins over `live_vertices` when both are present.
    shape_vertices: Option<ShapeVertexBuffer>,
    /// Permanent own-vertex base used by SetVertex's own-vertex modes.
    vertices: Option<Vec<ObjectVertex>>,
    /// `VTX_Set` installs the base without the `UpdateShape` that
    /// `VTX_SetPermanentUpd` performs (C4Script.cpp:1324-1325).
    vertices_defer_shape_update: bool,
    graphics_overlays: Option<Vec<ObjectGraphicsOverlay>>,
    draw_transform: Option<Option<DrawTransform>>,
    base_graphics: Option<Option<ObjectBaseGraphics>>,
    components: Option<HashMap<DefinitionId, i32>>,
    component_order: Option<Vec<DefinitionId>>,
    material_contents: Option<Vec<i32>>,
    local_vars: Option<HashMap<String, Value>>,
    physicals: Option<PhysicalsUpdate>,
    /// Rotate `contents` cyclically so this id becomes the front —
    /// C4ObjectList::ShiftContents (C4ObjectList.cpp:815-833).
    contents_front: Option<ObjectId>,
}

impl ObjectDelta {
    fn merge_update(&mut self, update: ObjectUpdate) {
        let changes_definition = update.change_def.is_some();
        if let Some(change_def) = update.change_def.as_ref() {
            self.change_def = Some(change_def.clone());
        }
        if changes_definition {
            self.change_def_reinsert = update.change_def_reinsert;
            self.change_def_contents_sort = update.change_def_contents_sort.clone();
            self.change_def_reset_action_time = update.change_def_reset_action_time;
        } else if update.change_def_reinsert {
            self.change_def_reinsert = true;
        }
        let writes_construction = update.construction.is_some();
        let construction_via_docon = update.construction_via_docon;
        let construction_preserves_fixed_position = update.construction_preserves_fixed_position;
        let resynchronizes_fixed_position = update.position.is_some()
            || update
                .action
                .as_ref()
                .is_some_and(|action| action.name.is_some());
        let replaces_docon_position =
            update.position.is_some() || (writes_construction && !construction_via_docon);
        if replaces_docon_position && update.resolved_docon_position.is_none() {
            self.resolved_docon_position = None;
        }
        if (resynchronizes_fixed_position || replaces_docon_position)
            && update.resolved_docon_fixed_position.is_none()
        {
            self.resolved_docon_fixed_position = None;
        }
        if let Some(custom_name) = update.custom_name {
            self.custom_name = Some(custom_name);
        }
        if let Some(layer) = update.layer {
            self.layer = Some(layer);
        }
        if let Some(compiler_cache) = update.compiler_cache {
            self.compiler_cache = Some(compiler_cache);
        }
        if let Some(visibility) = update.visibility {
            self.visibility = Some(visibility);
        }
        if let Some(blit_mode) = update.blit_mode {
            self.blit_mode = Some(blit_mode);
        }
        if let Some(picture_rect) = update.picture_rect {
            self.picture_rect = Some(picture_rect);
        }
        if let Some(color) = update.color {
            self.color = Some(color);
        }
        if let Some(color_modulation) = update.color_modulation {
            self.color_modulation = Some(color_modulation);
        }
        if let Some(position) = update.position {
            self.position = Some(position);
        }
        if let Some(position) = update.resolved_docon_position {
            self.resolved_docon_position = Some(position);
        }
        if let Some(position) = update.resolved_docon_fixed_position {
            self.resolved_docon_fixed_position = Some(position);
        }
        if let Some(own_mass) = update.own_mass {
            self.own_mass = Some(own_mass);
        }
        if let Some(velocity) = update.velocity {
            self.velocity = Some(velocity);
        }
        if let Some(fixed_velocity) = update.fixed_velocity {
            self.fixed_velocity = Some(fixed_velocity);
        }
        if let Some(x) = update.fixed_velocity_x {
            self.fixed_velocity_x = Some(x);
        }
        if let Some(y) = update.fixed_velocity_y {
            self.fixed_velocity_y = Some(y);
        }
        if let Some(mobile) = update.mobile {
            self.mobile = Some(mobile);
        }
        if let Some(t_attach) = update.t_attach {
            self.t_attach = Some(t_attach);
        }
        if let Some(contact_density) = update.contact_density {
            self.contact_density = Some(contact_density);
        }
        if let Some(rotation) = update.rotation {
            self.rotation = Some(rotation);
        }
        if let Some(rotation_velocity) = update.rotation_velocity {
            self.rotation_velocity = Some(rotation_velocity);
        }
        if let Some(energy) = update.energy {
            self.energy = Some(energy);
            self.host_energy_death_checked = update.host_energy_death_checked;
        }
        if let Some(breath) = update.breath {
            self.breath = Some(breath);
        }
        if let Some(need_energy) = update.need_energy {
            self.need_energy = Some(need_energy);
        }
        if let Some(cause) = update.energy_loss_cause {
            self.energy_loss_cause = Some(cause);
        }
        if let Some(fire) = update.fire {
            self.fire = Some(fire);
            // stage_ignite is the later write when no bare flag accompanies
            // this update, so it supersedes a previously merged flag.
            self.fire_flag = update.fire_flag;
        } else if let Some(flag) = update.fire_flag {
            // A bare SetOnFire changes only the flag and retains any phase
            // and attribution payload staged earlier in the batch.
            self.fire_flag = Some(flag);
        }
        if let Some(construction) = update.construction {
            self.construction = Some(construction);
        }
        if writes_construction {
            self.construction_via_docon = construction_via_docon;
            self.construction_preserves_fixed_position = construction_preserves_fixed_position;
        } else if resynchronizes_fixed_position {
            self.construction_preserves_fixed_position = false;
        }
        if let Some(damage) = update.damage {
            self.damage = Some(damage);
        }
        if let Some(magic_energy) = update.magic_energy {
            self.magic_energy = Some(magic_energy);
        }
        if let Some(magic_capacity) = update.magic_capacity {
            self.magic_capacity = Some(magic_capacity);
        }
        if let Some(direction) = update.direction {
            self.direction = Some(direction);
        }
        if let Some(command_direction) = update.command_direction {
            self.command_direction = Some(command_direction);
        }
        if let Some(owner) = update.owner {
            self.owner = Some(owner);
        }
        if let Some(base) = update.base {
            self.base = Some(base);
        }
        if let Some(controller) = update.controller {
            self.controller = Some(controller);
        }
        if let Some(category) = update.category {
            self.category = Some(category);
        }
        if let Some(crew_member) = update.crew_member {
            self.crew_member = Some(crew_member);
        }
        if let Some(plr_view_range) = update.plr_view_range {
            self.plr_view_range = Some(plr_view_range);
        }
        if let Some(selected) = update.selected {
            self.selected = Some(selected);
        }
        if let Some(crew_disabled) = update.crew_disabled {
            self.crew_disabled = Some(crew_disabled);
        }
        if let Some(rect) = update.solid_mask_override {
            self.solid_mask_override = Some(rect);
        }
        if let Some(sequence) = update.solid_mask_instance_sequence {
            self.solid_mask_instance_sequence = Some(sequence);
        }
        if let Some(menu) = update.menu {
            self.menu = Some(menu);
        }
        if let Some(shape_override) = update.shape_override {
            self.shape_override = Some(shape_override);
        }

        if let Some(alive) = update.alive {
            self.alive = Some(alive);
        }
        if let Some(entrance_status) = update.entrance_status {
            self.entrance_status = Some(entrance_status);
        }
        if let Some(container) = update.container {
            self.container = Some(container);
        }
        if let Some(status) = update.status {
            self.status = Some(status);
        }
        if let Some(vertices) = update.live_vertices {
            self.live_vertices = Some(vertices);
        }
        if let Some(vertices) = update.shape_vertices {
            self.shape_vertices = Some(vertices);
        }
        if let Some(vertices) = update.vertices {
            self.vertices = Some(vertices);
            self.vertices_defer_shape_update = update.vertices_defer_shape_update;
        }
        if let Some(overlays) = update.graphics_overlays {
            self.graphics_overlays = Some(overlays);
        }
        if let Some(transform) = update.draw_transform {
            self.draw_transform = Some(transform);
        }
        if let Some(base_graphics) = update.base_graphics {
            self.base_graphics = Some(base_graphics);
        }
        if let Some(components) = update.components {
            self.components = Some(components);
        }
        if let Some(component_order) = update.component_order {
            self.component_order = Some(component_order);
        }
        if let Some(material_contents) = update.material_contents {
            self.material_contents = Some(material_contents);
        }
        if let Some(physicals) = update.physicals {
            self.physicals = Some(physicals);
        }
        if let Some(contents_front) = update.contents_front {
            self.contents_front = Some(contents_front);
        }
        if let Some(action) = update.action {
            match &mut self.action {
                Some(existing) => existing.merge(action),
                None => self.action = Some(action),
            }
        }
    }
}

impl From<ObjectUpdate> for ObjectDelta {
    fn from(update: ObjectUpdate) -> Self {
        Self {
            change_def: update.change_def,
            change_def_reinsert: update.change_def_reinsert,
            change_def_contents_sort: update.change_def_contents_sort,
            change_def_reset_action_time: update.change_def_reset_action_time,
            custom_name: update.custom_name,
            layer: update.layer,
            compiler_cache: update.compiler_cache,
            visibility: update.visibility,
            blit_mode: update.blit_mode,
            picture_rect: update.picture_rect,
            color: update.color,
            color_modulation: update.color_modulation,
            fixed_velocity_x: update.fixed_velocity_x,
            fixed_velocity_y: update.fixed_velocity_y,
            position: update.position,
            own_mass: update.own_mass,
            velocity: update.velocity,
            fixed_velocity: update.fixed_velocity,
            mobile: update.mobile,
            t_attach: update.t_attach,
            contact_density: update.contact_density,
            rotation: update.rotation,
            rotation_velocity: update.rotation_velocity,
            energy: update.energy,
            host_energy_death_checked: update.host_energy_death_checked,
            breath: update.breath,
            need_energy: update.need_energy,
            energy_loss_cause: update.energy_loss_cause,
            fire: update.fire,
            fire_flag: update.fire_flag,
            construction: update.construction,
            construction_via_docon: update.construction_via_docon,
            construction_preserves_fixed_position: update.construction_preserves_fixed_position,
            resolved_docon_position: update.resolved_docon_position,
            resolved_docon_fixed_position: update.resolved_docon_fixed_position,
            damage: update.damage,
            magic_energy: update.magic_energy,
            magic_capacity: update.magic_capacity,
            direction: update.direction,
            command_direction: update.command_direction,
            action: update.action,
            status: update.status,
            owner: update.owner,
            base: update.base,
            controller: update.controller,
            category: update.category,
            crew_member: update.crew_member,
            plr_view_range: update.plr_view_range,
            selected: update.selected,
            crew_disabled: update.crew_disabled,
            solid_mask_override: update.solid_mask_override,
            solid_mask_instance_sequence: update.solid_mask_instance_sequence,
            menu: update.menu,
            shape_override: update.shape_override,
            alive: update.alive,
            entrance_status: update.entrance_status,
            container: update.container,
            live_vertices: update.live_vertices,
            shape_vertices: update.shape_vertices,
            vertices: update.vertices,
            vertices_defer_shape_update: update.vertices_defer_shape_update,
            graphics_overlays: update.graphics_overlays,
            draw_transform: update.draw_transform,
            base_graphics: update.base_graphics,
            components: update.components,
            component_order: update.component_order,
            material_contents: update.material_contents,
            local_vars: update.local_vars,
            physicals: update.physicals,
            contents_front: update.contents_front,
        }
    }
}

/// Serde's default nested-Option handling collapses an explicit JSON `null`
/// into the same outer `None` as a missing field. Update fields need all three
/// states, so a present value always wraps the decoded inner option.
fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn serialize_double_optional_c4_string<S>(
    value: &Option<Option<String>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    struct C4StringRef<'a>(&'a str);

    impl serde::Serialize for C4StringRef<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            clonk_script::c4_string_serde::serialize_ref(self.0, serializer)
        }
    }

    match value {
        Some(Some(value)) => serializer.serialize_some(&C4StringRef(value)),
        Some(None) | None => serializer.serialize_none(),
    }
}

fn deserialize_double_optional_c4_string<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct C4String(#[serde(with = "clonk_script::c4_string_serde")] String);

    Option::<C4String>::deserialize(deserializer).map(|value| Some(value.map(|value| value.0)))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ObjectUpdate {
    /// C4Object::CustomName write: Some(Some(name)) sets; Some(None) clears.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_double_optional_c4_string",
        deserialize_with = "deserialize_double_optional_c4_string"
    )]
    pub custom_name: Option<Option<String>>,
    /// C4Object::pLayer write: Some(Some(object)) sets; Some(None) clears.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub layer: Option<Option<ObjectId>>,
    /// Runtime-visible compiler caches. Kept separate from the associated live
    /// pointers because C4EnumeratedObjectPtr only clears its number for a
    /// literal-null assignment and only refreshes it during enumeration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub compiler_cache: Option<ObjectCompilerCache>,
    /// SetVisibility (C4Script.cpp:3860-3869).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<i32>,
    /// C4Object::BlitMode overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blit_mode: Option<u32>,
    /// FnSetPicture's raw C4Object::PictureRect overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_rect: Option<DefinitionRect>,
    /// SetSolidMask's rect update (Some = set; zero-area = mask OFF).
    #[serde(default)]
    pub solid_mask_override: Option<DefinitionTargetRect>,
    /// Runtime-only C4SolidMask construction order reserved while a script
    /// callback is still executing. It must not enter save/control data.
    #[serde(skip)]
    #[doc(hidden)]
    pub solid_mask_instance_sequence: Option<u64>,
    /// FnChangeDef's definition swap (C4Object::ChangeDef,
    /// C4Object.cpp:1180-1231).
    #[serde(default)]
    pub change_def: Option<String>,
    /// A callback removed and re-added this object's contents link. The
    /// final container may equal the pre-call value, so an ordinary Option
    /// delta would otherwise collapse the mandatory remove/re-add into a
    /// no-op. ChangeDef additionally carries its special sort metadata.
    #[serde(default, skip_serializing_if = "is_false")]
    #[doc(hidden)]
    pub change_def_reinsert: bool,
    /// Sort key of a contents link established by an old-definition action
    /// callback before ChangeDef swaps Def. A vetoed final Enter leaves that
    /// exact link in place rather than re-adding it as Unsorted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub change_def_contents_sort: Option<ChangeDefContentsSort>,
    /// The ordinary SetAction(ActIdle) phase of ChangeDef succeeded, so its
    /// action-name transition reset C4Action::Time before the unconditional
    /// raw ActIdle slot overwrite.
    #[serde(default, skip_serializing_if = "is_false")]
    #[doc(hidden)]
    pub change_def_reset_action_time: bool,
    pub position: Option<Vector2>,
    pub velocity: Option<Vector2>,
    /// Sub-pixel velocity in 16.16 fixed-point, set by precision-aware script
    /// surfaces (`SetXDir`/`SetYDir`). Takes precedence over `velocity` when
    /// applied. Mirrors C++ storing velocity as `C4Fixed` (`C4Object.h` xdir/ydir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    /// See ObjectDelta::fixed_velocity_x — component-only SetXDir/SetYDir.
    pub fixed_velocity_x: Option<C4Fixed>,
    pub fixed_velocity_y: Option<C4Fixed>,
    /// Explicit C4Object::Mobile overwrite. Applied after the generic
    /// velocity-write mobilization so native helpers can preserve or force
    /// the exact C++ branch behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile: Option<bool>,
    /// Explicit C4Object::Action.t_attach overwrite for same-frame native
    /// attachment changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_attach: Option<u32>,
    /// Sub-pixel angular velocity (16.16 fixed degrees/frame) from `SetRDir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    #[serde(default)]
    pub rotation: Option<i32>,
    pub energy: Option<i32>,
    /// Runtime-only marker that this exact energy write already evaluated
    /// the native DoEnergy death predicate synchronously. It prevents the
    /// later callback fold from replaying AssignDeath after a revival or a
    /// SetAlive change, and never enters save/control data.
    #[serde(skip)]
    #[doc(hidden)]
    pub host_energy_death_checked: bool,
    /// C4Object::Breath overwrite on the raw physical scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breath: Option<i32>,
    /// C4Object::NeedEnergy overwrite (FnEnergyCheck).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_energy: Option<bool>,
    /// C4Object::DoEnergy's kill-trace mark (C4Object.cpp:1351-1353),
    /// staged with the UpdatLastEnergyLossCause guard (:1369-1378) already
    /// applied at call time — the fold writes it through unconditionally
    /// (Punch's post-fling write is unguarded too, C4ObjectCom.cpp:755,762).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_loss_cause: Option<i32>,
    /// Staged C4Object::Incinerate outcome `(caused_by, fire_phase)` from
    /// the host seam — the RNG draw and the Incineration callback already
    /// ran mid-call; the fold only writes the fire state bits
    /// (fxFireStart core, C4Effect.cpp:632-634).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire: Option<(i32, i32)>,
    /// Bare OnFire flag write — the engine-internal FnFxFireStop /
    /// FnFxFireStart temp arms (C4Effect.cpp:563-565, 775-791). A preceding
    /// `fire` payload stays staged so SetOnFire can change only the flag
    /// without discarding the already-written cause and phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_flag: Option<bool>,
    #[serde(default)]
    pub damage: Option<i32>,
    #[serde(default)]
    pub magic_energy: Option<i32>,
    #[serde(default)]
    pub magic_capacity: Option<i32>,
    #[serde(default)]
    pub construction: Option<i32>,
    /// Internal provenance for a staged C4Object::DoCon write. Generic
    /// construction assignments still resynchronize fixed position.
    #[serde(default, skip_serializing_if = "is_false")]
    pub construction_via_docon: bool,
    /// Final fixed-position ordering for a staged DoCon. SetAction and
    /// explicit position writes after DoCon clear this bit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub construction_preserves_fixed_position: bool,
    /// Runtime-only host fold of DoCon's sequential integer/fixed position.
    #[serde(skip)]
    pub resolved_docon_position: Option<Vector2>,
    #[serde(skip)]
    pub resolved_docon_fixed_position: Option<FixedVec2>,
    pub action: Option<ActionUpdate>,
    #[serde(default)]
    pub direction: Option<Direction>,
    #[serde(default)]
    pub command_direction: Option<CommandDirection>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    /// Final cached C4Object::OCF overwrite for native helpers whose
    /// temporary state transition intentionally leaves the cache stale.
    /// FnCollect restores NoCollectDelay without a second UpdateOCF
    /// (C4Script.cpp:397-413), so its temporary Collection bit survives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocf_override: Option<u32>,
    #[serde(default)]
    pub owner: Option<i32>,
    /// C4Object::Base overwrite used by FLAG/FlyBase SetOwner propagation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<i32>,
    /// FnSetController (C4Script.cpp:1322-1331); also recorded by
    /// SetOwner's automatic controller update (C4Object.cpp:5499-5500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<i32>,
    #[serde(default)]
    pub category: Option<i32>,
    /// SetMass (C4Script.cpp:3620-3626): OwnMass = value - Def->Mass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_mass: Option<i32>,
    #[serde(default)]
    pub crew_member: Option<bool>,
    /// FnSetPlrViewRange / MakeCrewMember's zero-to-default update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plr_view_range: Option<i32>,
    /// SetObjectCrewStatus changes the player's roster without running
    /// AdjustCursorCommand; suppress the generic crew-bit cursor repair.
    #[serde(default, skip_serializing_if = "is_false")]
    pub crew_status_change: bool,
    /// Live C4Object::Info rank write. Some(Some(rank)) attaches/updates
    /// rank data; Some(None) clears the linked info rank.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub info_rank: Option<Option<i32>>,
    /// Player whose CrewInfoList owns the live C4Object::Info pointer.
    /// Some(None) clears that pointer ownership alongside info_rank.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub info_link: Option<Option<CrewInfoLink>>,
    /// C4Object::Select overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    pub crew_disabled: Option<bool>,
    #[serde(default)]
    pub alive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<Option<ObjectId>>,
    /// Direct current-shape vertex overwrite. This deliberately leaves the
    /// object's own-vertex mode unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_vertices: Option<Vec<ObjectVertex>>,
    /// Exact current C4Shape slot overwrite, including dormant slots beyond
    /// the active vertex count. Internal host mutations use this so delayed
    /// command/state serialization cannot discard RemoveVertex metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub shape_vertices: Option<ShapeVertexBuffer>,
    /// FnSetContactDensity's live C4Shape::ContactDensity overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_density: Option<i32>,
    /// Permanent own-vertex base overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<ObjectVertex>>,
    /// `SetVertex`'s plain own-vertex mode (`VTX_Set`) writes only the shape's
    /// backup half; the live shape keeps its vertices until some later
    /// `UpdateShape` restores them (C4Script.cpp:1297-1325). Set this so the
    /// accompanying `vertices` base does not run `UpdateShape` right away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[doc(hidden)]
    pub vertices_defer_shape_update: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics_overlays: Option<Vec<ObjectGraphicsOverlay>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<Option<DrawTransform>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<Option<ObjectBaseGraphics>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<HashMap<DefinitionId, i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_order: Option<Vec<DefinitionId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_vars: Option<HashMap<String, Value>>,
    /// Full physical-state overwrite from the physicals host functions
    /// (SetPhysical/TrainPhysical/ResetPhysical, C4Script.cpp:552-636).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physicals: Option<PhysicalsUpdate>,
    /// SetEntrance (C4Script.cpp:690-695).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrance_status: Option<bool>,
    /// Script menu write: Some(Some(_)) = CreateMenu/AddMenuItem/
    /// SelectMenuItem left this state, Some(None) = CloseMenu
    /// (C4Object::CloseMenu, C4Object.cpp:2009-2017).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Option<ObjectMenuState>>,
    /// SetColorDw (C4Script.cpp:3661-3668).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    /// SetClrModulation's raw C4Object::ColorMod overwrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_modulation: Option<u32>,
    /// SetShape / UpdateShape: Some(Some(rect)) installs an override;
    /// Some(None) clears it when UpdateFace(true) rebuilds a non-line shape.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub shape_override: Option<Option<DefinitionRect>>,
    /// Runtime-only C4Object::MaterialContents overwrite produced by the
    /// synchronous DigFree host preview. It never enters save/control data.
    #[serde(skip)]
    #[doc(hidden)]
    pub material_contents: Option<Vec<i32>>,
    /// Rotate the contents list cyclically so this id becomes the front —
    /// C4ObjectList::ShiftContents (C4ObjectList.cpp:815-833), the
    /// FnShiftContents/DirectComContents write path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_front: Option<ObjectId>,
}

/// The complete per-object physical state as left by a script callback —
/// applied wholesale (a cleared temporary set must overwrite engine state).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhysicalsUpdate {
    pub info: Option<PhysicalInfo>,
    pub temporary: Option<PhysicalInfo>,
    pub changes: Vec<(String, i32)>,
}

impl ObjectUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    /// The staged OnFire outcome, if any fire write is pending.
    pub fn staged_on_fire(&self) -> Option<bool> {
        self.fire_flag.or_else(|| self.fire.map(|_| true))
    }

    /// Stage the full fxFireStart outcome (C4Effect.cpp:632-634).
    pub fn stage_ignite(&mut self, caused_by: i32, phase: i32) {
        self.fire = Some((caused_by, phase));
        self.fire_flag = None;
    }

    /// Stage a bare SetOnFire write (FnFxFireStop / temp arms,
    /// C4Effect.cpp:563-565, 775-791).
    pub fn stage_fire_flag(&mut self, flag: bool) {
        self.fire_flag = Some(flag);
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_contents_front(mut self, contents_front: ObjectId) -> Self {
        self.contents_front = Some(contents_front);
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = Some(velocity);
        self
    }

    pub fn with_rotation(mut self, rotation: i32) -> Self {
        self.rotation = Some(rotation);
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = Some(energy);
        self
    }

    pub fn with_breath(mut self, breath: i32) -> Self {
        self.breath = Some(breath);
        self
    }

    pub fn with_need_energy(mut self, need_energy: bool) -> Self {
        self.need_energy = Some(need_energy);
        self
    }

    pub fn with_damage(mut self, damage: i32) -> Self {
        self.damage = Some(damage);
        self
    }

    pub fn with_construction(mut self, construction: i32) -> Self {
        self.construction = Some(construction.clamp(0, FULL_CON));
        self
    }

    pub fn with_contact_density(mut self, contact_density: i32) -> Self {
        self.contact_density = Some(contact_density);
        self
    }

    pub fn with_magic_energy(mut self, magic_energy: i32) -> Self {
        self.magic_energy = Some(magic_energy);
        self
    }

    pub fn with_magic_capacity(mut self, magic_capacity: i32) -> Self {
        self.magic_capacity = Some(magic_capacity);
        self
    }

    pub fn set_damage(&mut self, damage: i32) {
        self.damage = Some(damage);
    }

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn with_command_direction(mut self, command_direction: CommandDirection) -> Self {
        self.command_direction = Some(command_direction);
        self
    }

    pub fn with_action(mut self, name: impl Into<String>) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_name(name);
        self.action = Some(update);
        self
    }

    pub fn with_action_phase(mut self, phase: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_phase(phase);
        self.action = Some(update);
        self
    }

    pub fn with_action_ticks(mut self, ticks: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_ticks(ticks);
        self.action = Some(update);
        self
    }

    pub fn with_action_data(mut self, data: i32) -> Self {
        let mut update = self.action.unwrap_or_default();
        update.set_data(data);
        self.action = Some(update);
        self
    }

    pub fn with_action_update(mut self, update: ActionUpdate) -> Self {
        self.action = Some(update);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn with_base(mut self, base: i32) -> Self {
        self.base = Some(base);
        self
    }

    pub fn with_category(mut self, category: i32) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_status(mut self, status: ObjectStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_container(mut self, container: ObjectId) -> Self {
        self.container = Some(Some(container));
        self
    }

    pub fn with_layer(mut self, layer: ObjectId) -> Self {
        self.layer = Some(Some(layer));
        self
    }

    pub fn clear_layer(mut self) -> Self {
        self.layer = Some(None);
        self
    }

    pub fn with_blit_mode(mut self, blit_mode: u32) -> Self {
        self.blit_mode = Some(blit_mode);
        self
    }

    pub fn clear_container(mut self) -> Self {
        self.container = Some(None);
        self
    }

    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = Some(crew_member);
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = Some(alive);
        self
    }

    /// Whether the staged host operations call `C4Object::SetOCF` at the
    /// mutation site. Plain callback completion and velocity/direction writes
    /// leave Execute's cached OCF untouched (C4Object.cpp:1082-1093).
    fn refreshes_ocf_like_cpp(&self) -> bool {
        self.change_def.is_some()
            || self.construction.is_some()
            || self.container.is_some()
            || self
                .action
                .as_ref()
                .is_some_and(|action| action.name.is_some())
            || self.category.is_some()
            || self.alive.is_some()
            || self.fire.is_some()
            || self.fire_flag.is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.custom_name.is_none()
            && self.layer.is_none()
            && self.compiler_cache.is_none()
            && self.visibility.is_none()
            && self.blit_mode.is_none()
            && self.picture_rect.is_none()
            && self.color.is_none()
            && self.color_modulation.is_none()
            && self.solid_mask_override.is_none()
            && self.solid_mask_instance_sequence.is_none()
            && self.shape_override.is_none()
            && self.material_contents.is_none()
            && self.change_def.is_none()
            && !self.change_def_reinsert
            && self.change_def_contents_sort.is_none()
            && !self.change_def_reset_action_time
            && self.position.is_none()
            && self.resolved_docon_position.is_none()
            && self.resolved_docon_fixed_position.is_none()
            && self.velocity.is_none()
            && self.fixed_velocity.is_none()
            && self.fixed_velocity_x.is_none()
            && self.fixed_velocity_y.is_none()
            && self.mobile.is_none()
            && self.t_attach.is_none()
            && self.rotation.is_none()
            && self.rotation_velocity.is_none()
            && self.energy.is_none()
            && self.breath.is_none()
            && self.need_energy.is_none()
            && self.energy_loss_cause.is_none()
            && self.fire.is_none()
            && self.fire_flag.is_none()
            && self.construction.is_none()
            && self.damage.is_none()
            && self.magic_energy.is_none()
            && self.magic_capacity.is_none()
            && self.direction.is_none()
            && self.command_direction.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.ocf_override.is_none()
            && self.owner.is_none()
            && self.base.is_none()
            && self.controller.is_none()
            && self.category.is_none()
            && self.crew_member.is_none()
            && self.plr_view_range.is_none()
            && !self.crew_status_change
            && self.info_rank.is_none()
            && self.info_link.is_none()
            && self.selected.is_none()
            && self.alive.is_none()
            && self.entrance_status.is_none()
            && self.container.is_none()
            && self.live_vertices.is_none()
            && self.shape_vertices.is_none()
            && self.contact_density.is_none()
            && self.vertices.is_none()
            && self.graphics_overlays.is_none()
            && self.draw_transform.is_none()
            && self.base_graphics.is_none()
            && self.components.is_none()
            && self.component_order.is_none()
            && self.physicals.is_none()
            && self.contents_front.is_none()
            && self.menu.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub delay: u32,
    pub update: ObjectUpdate,
    pub effects: Vec<EffectCommand>,
    #[serde(default)]
    pub events: Vec<CommandEvent>,
    pub destroy: bool,
    pub spawns: Vec<SpawnConfig>,
    #[serde(default)]
    pub landscape: Vec<LandscapeCommand>,
    #[serde(default)]
    pub particles: Vec<ParticleCommand>,
}

impl QueuedCommand {
    pub fn new(delay: u32, update: ObjectUpdate) -> Self {
        Self {
            delay,
            update,
            effects: Vec::new(),
            events: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
        }
    }

    pub fn immediate(update: ObjectUpdate) -> Self {
        Self {
            delay: 0,
            update,
            effects: Vec::new(),
            events: Vec::new(),
            destroy: false,
            spawns: Vec::new(),
            landscape: Vec::new(),
            particles: Vec::new(),
        }
    }

    pub fn with_delay(mut self, delay: u32) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_effects(mut self, effects: Vec<EffectCommand>) -> Self {
        self.effects = effects;
        self
    }

    pub fn with_events(mut self, events: Vec<CommandEvent>) -> Self {
        self.events = events;
        self
    }

    pub fn with_destroy(mut self, destroy: bool) -> Self {
        self.destroy = destroy;
        self
    }

    pub fn with_spawns(mut self, spawns: Vec<SpawnConfig>) -> Self {
        self.spawns = spawns;
        self
    }

    pub fn with_landscape(mut self, commands: Vec<LandscapeCommand>) -> Self {
        self.landscape = commands;
        self
    }

    pub fn with_particles(mut self, particles: Vec<ParticleCommand>) -> Self {
        self.particles = particles;
        self
    }

    pub fn update(&self) -> &ObjectUpdate {
        &self.update
    }

    pub fn effects(&self) -> &[EffectCommand] {
        &self.effects
    }

    pub fn landscape(&self) -> &[LandscapeCommand] {
        &self.landscape
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CrewSelection {
    cursor: Option<ObjectId>,
}

impl CrewSelection {
    fn prune(&mut self, alive: &HashSet<ObjectId>) {
        if let Some(cursor) = self.cursor {
            if !alive.contains(&cursor) {
                self.cursor = None;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.cursor.is_none()
    }

    fn cursor(&self) -> Option<ObjectId> {
        self.cursor
    }

    fn set_cursor(&mut self, cursor: Option<ObjectId>) {
        self.cursor = cursor;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CrewSelectionState {
    #[serde(default)]
    pub selected: Vec<ObjectId>,
    #[serde(default)]
    pub cursor: Option<ObjectId>,
}

impl From<CrewSelectionState> for CrewSelection {
    fn from(state: CrewSelectionState) -> Self {
        Self {
            cursor: state.cursor,
        }
    }
}

#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct Object {
    #[doc(hidden)]
    pub id: ObjectId,
    #[doc(hidden)]
    pub definition_id: DefinitionId,
    #[doc(hidden)]
    pub state: ObjectState,
    /// C4Object::Mass is a compiled cache, not a derived getter. Objects.txt
    /// restores it verbatim (including zero) until a native UpdateMass path
    /// runs. The contents snapshot lets runtime Enter/Exit invalidate a
    /// parent's cache without disturbing load-time link denumeration.
    compiled_mass: Option<i32>,
    compiled_mass_contents: Vec<ObjectId>,
    /// C4Object::Unsorted is a transient, non-savegame flag. ChangeDef sets
    /// it without requesting a sweep; a later C4Object::Resort request (or
    /// Objects.Synchronize) clears it while remove/re-adding the main-list
    /// link. C4ObjectList::Add also treats the flag as a sort override for
    /// contents links.
    #[doc(hidden)]
    pub unsorted: bool,
    /// Deferred contents-link key for a link established during ChangeDef's
    /// old-action callbacks. It is consumed by the engine-side list fold and
    /// is transient like `Unsorted`.
    change_def_contents_sort: Option<ChangeDefContentsSort>,
    #[doc(hidden)]
    pub fixed_position: FixedVec2,
    #[doc(hidden)]
    pub fixed_velocity: FixedVec2,
    /// C4Object::motion_x/motion_y: whole-pixel displacement accumulated by
    /// DoMotion during the most recent DoMovement invocation.
    #[doc(hidden)]
    pub motion_x: i32,
    #[doc(hidden)]
    pub motion_y: i32,
    /// Raw fields consumed by C4Object::CompileFunc rather than reconstructed
    /// from their associated live pointers. C4EnumeratedObjectPtr keeps its
    /// signed enumeration word independent of the resolved Object pointer.
    compiler_cache: ObjectCompilerCache,
    /// 16.16 fixed-point rotation accumulator (C++ `fix_r`, `C4Object.h:149`).
    /// `state.rotation` (whole degrees) is its `fixtoi` projection.
    #[doc(hidden)]
    pub fixed_rotation: C4Fixed,
    /// 16.16 fixed-point angular velocity in degrees/frame (C++ `rdir`,
    /// `C4Object.h:150`). Set by `SetRDir`; applied as `fix_r += rdir * 5` each
    /// frame (`C4Movement.cpp:376`).
    #[doc(hidden)]
    pub rotation_velocity: C4Fixed,
    #[doc(hidden)]
    pub destroyed: bool,
    /// Last `Info->Physical` storage after AssignRemoval clears the object's
    /// Info pointer. A native caller may still hold the original C++ pointer
    /// for the remainder of the stack frame (ObjectComDigDouble does).
    retired_info_physical: Option<PhysicalInfo>,
    /// The baked solid mask (grid worlds only; C4Object::pSolidMaskData).
    #[doc(hidden)]
    pub solid_mask_bake: Option<SolidMaskBake>,
    /// C4SolidMask::MaskPut for an eligible mask whose landscape-clipped
    /// rectangle is empty. Such a put has no raster bake, but it must remain
    /// logically put so movement can restore riders and a later Remove can
    /// clear the lifecycle state (C4SolidMask.cpp:75-79,176-195,231-262).
    solid_mask_empty_put: bool,
    /// Construction order of the live C4SolidMask instance. This survives
    /// ordinary Remove/Put cycles (including fully off-landscape puts) and is
    /// cleared only when C++ would delete pSolidMaskData.
    solid_mask_instance_sequence: Option<u64>,
    /// This frame's latched Action.t_attach (C4Object.cpp:4692 + the
    /// per-procedure assignments): ExecAction computes it BEFORE the
    /// phase-wrap SetAction — the movement that follows runs with the
    /// OLD action's attach (the wrapping HeadUp bison free-falls its
    /// wrap frame instead of snapping to a floor 5px down).
    #[doc(hidden)]
    pub frame_t_attach: u32,
    /// C4Object::t_contact latched by the most recent ContactCheck. Command
    /// execution precedes this frame's movement and therefore reads the
    /// previous movement frame's value (C4Movement.cpp:166-182,470).
    #[doc(hidden)]
    pub frame_t_contact: u32,
    /// Live C4Shape::VtxContactCNAT aligned with the current Shape vertices.
    /// Collision response reads this again after synchronous Contact* calls.
    frame_vertex_contacts: Vec<u32>,
    /// Live C4Shape::ContactCNAT/ContactCount. Unlike `frame_t_contact`, these
    /// may be replaced by a shape refresh inside a synchronous Contact*
    /// callback before ContactCheck returns.
    frame_shape_contact_cnat: u32,
    frame_shape_contact_count: i32,
    /// This frame's UprightAttach bits (C4Object.cpp:4698-4705): the
    /// per-frame `Action.t_attach |= Def->UprightAttach` OR that feeds the
    /// movement config. Transient — recomputed at every ExecAction, never
    /// serialized.
    upright_t_attach: u32,
    /// C4Object::iLastAttachMovementFrame: prevents two moving masks from
    /// carrying the same object twice in one frame (C4SolidMask.cpp:187-193).
    last_attach_movement_frame: i32,
    /// Last energy-loss causing player (C4Object::LastEnergyLossCausePlayer,
    /// read by AssignDeath for kill attribution).
    #[doc(hidden)]
    pub last_energy_loss_cause: i32,
    /// Transient C4Object::InMat cache. SetOCF/UpdateOCF sample it; ExecLife
    /// deliberately reads that cached, normally pre-movement material.
    in_mat: Option<MaterialId>,
    command_queue: VecDeque<QueuedCommand>,
    #[doc(hidden)]
    pub commands: CommandStack,
    pending_action_events: VecDeque<ActionTransitionEvent>,
    /// The DFA_SWIM free-fall exit `return`ed out of ExecAction this
    /// frame BEFORE any t_attach assignment — the frame's movement runs
    /// unattached (C4Object.cpp:4692,4956).
    swim_exit_this_frame: bool,
    material_contents: Vec<i32>,
    shape_template: ObjectShapeTemplate,
    /// Exact live C4Shape rectangle. Con/r may change without UpdateShape,
    /// so this cannot be reconstructed from the current scalar fields.
    shape_rect: Option<DefinitionRect>,
    /// Live C4Shape::FireTop. Unlike the definition template this survives
    /// SetShape and loaded-object shape data until UpdateShape rebuilds the
    /// complete shape from DefCore (C4Shape.cpp:495-510).
    shape_fire_top: i32,
    #[doc(hidden)]
    pub own_shape_vertices: Option<Vec<ObjectVertex>>,
}

/// Private C4Object compiler caches exposed verbatim by GetObjectVal. Pointer
/// assignment and pointer denumeration do not refresh these words; an object
/// enumeration pass does. Keeping them independent is observable for freshly
/// assigned, stale, legacy-offset, and unresolved references.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ObjectCompilerCache {
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub contained: i32,
    #[serde(default)]
    pub action_target1: i32,
    #[serde(default)]
    pub action_target2: i32,
    #[serde(default)]
    pub layer: i32,
}

impl ObjectCompilerCache {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Stable position of C4Effect::Execute's live-list cursor. Effect numbers
/// identify the current node even when callbacks insert into the prefix; the
/// priority is only a fallback for legacy command paths that still unlink the
/// current node immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EffectFrameCursor {
    number: i32,
    priority: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionTransitionKind {
    Natural,
    Forced,
}

#[derive(Debug, Clone)]
struct ActionTransitionEvent {
    previous_action: ActionState,
    kind: ActionTransitionKind,
}

#[derive(Debug)]
struct ContainerUpdateRecord {
    object_id: ObjectId,
    previous: Option<ObjectId>,
    new: Option<ObjectId>,
}

#[derive(Debug, Clone)]
struct ObjectShapeTemplate {
    vertices: Vec<ObjectVertex>,
    rect: Option<DefinitionRect>,
    fire_top: i32,
    stretch_growth: bool,
    rotateable: i32,
    /// DefCore Line type: Line objects skip every shape/vertex refresh
    /// (C4Object::UpdateShape early return, C4Object.cpp:322-324) — the
    /// CONNECT exec owns their vertices.
    line: i32,
}

impl ObjectShapeTemplate {
    fn new(
        vertices: Vec<ObjectVertex>,
        rect: Option<DefinitionRect>,
        fire_top: i32,
        stretch_growth: bool,
        rotateable: i32,
    ) -> Self {
        Self {
            vertices,
            rect,
            fire_top,
            stretch_growth,
            rotateable,
            line: 0,
        }
    }

    fn with_line(mut self, line: i32) -> Self {
        self.line = line;
        self
    }
}

#[derive(Debug, Default)]
struct CommandQueueOutcome {
    spawns: Vec<SpawnConfig>,
    destroy: bool,
    effect_events: Vec<EffectEvent>,
    container_updates: Vec<ContainerUpdateRecord>,
    command_events: Vec<CommandEvent>,
    particles: Vec<ParticleCommand>,
    definition_changed: bool,
    change_def_reinsert: bool,
}

/// Record a fixed-point vector in a snapshot only when it carries sub-pixel
/// detail beyond its whole-pixel projection — i.e. when `fixtoi(fixed)` would
/// not round-trip back to `fixed`. Returns `None` for whole-pixel values so the
/// snapshot stays minimal and reconstructs losslessly via `itofix(pixels)`.
fn subpixel_or_none(fixed: FixedVec2, pixels: Vector2) -> Option<FixedVec2> {
    if fixed == FixedVec2::from_ints(pixels.x, pixels.y) {
        None
    } else {
        Some(fixed)
    }
}

impl Object {
    /// C++ APIs that explicitly test Status accept every nonzero value,
    /// including C4OS_INACTIVE. Raw pointers may remain dereferenceable
    /// after Status reaches zero; callers must not use this as a pointer gate.
    fn has_nonzero_status(&self) -> bool {
        !self.destroyed && self.state.status != ObjectStatus::Deleted
    }

    fn new(
        id: ObjectId,
        definition_id: DefinitionId,
        state: ObjectState,
        shape_template: ObjectShapeTemplate,
        own_shape_vertices: Option<Vec<ObjectVertex>>,
    ) -> Self {
        let frame_vertex_contacts = vec![0; state.vertices.len()];
        let fixed_position = FixedVec2::from_ints(state.position.x, state.position.y);
        let fixed_velocity = FixedVec2::from_ints(state.velocity.x, state.velocity.y);
        let fixed_rotation = itofix(state.rotation);
        let shape_fire_top = scaled_shape_fire_top(
            shape_template.fire_top,
            state.construction,
            shape_template.line,
        );
        let shape_rect = if shape_template.line != 0 {
            shape_template.rect
        } else {
            transformed_shape_rect(
                shape_template.rect,
                state.construction,
                shape_template.stretch_growth,
                shape_template.rotateable,
                state.rotation,
            )
        };
        Self {
            id,
            definition_id,
            compiled_mass: None,
            compiled_mass_contents: Vec::new(),
            unsorted: false,
            change_def_contents_sort: None,
            fixed_position,
            fixed_velocity,
            motion_x: 0,
            motion_y: 0,
            compiler_cache: ObjectCompilerCache::default(),
            fixed_rotation,
            rotation_velocity: C4Fixed::ZERO,
            destroyed: matches!(state.status, ObjectStatus::Deleted),
            retired_info_physical: None,
            frame_t_attach: 0,
            frame_t_contact: 0,
            frame_vertex_contacts,
            frame_shape_contact_cnat: 0,
            frame_shape_contact_count: 0,
            solid_mask_bake: None,
            solid_mask_empty_put: false,
            solid_mask_instance_sequence: None,
            state,
            upright_t_attach: 0,
            last_attach_movement_frame: -1,
            last_energy_loss_cause: OWNER_NONE,
            in_mat: None,
            command_queue: VecDeque::new(),
            commands: CommandStack::new(),
            pending_action_events: VecDeque::new(),
            swim_exit_this_frame: false,
            material_contents: Vec::new(),
            shape_template,
            shape_rect,
            shape_fire_top,
            own_shape_vertices,
        }
    }

    #[allow(dead_code)]
    fn command_count(&self) -> usize {
        self.commands.len()
    }

    fn fixed_vec_to_pixels(value: FixedVec2) -> Vector2 {
        Vector2::new(value.int_x(), value.int_y())
    }

    fn position_pixels(&self) -> Vector2 {
        Self::fixed_vec_to_pixels(self.fixed_position)
    }

    fn velocity_pixels(&self) -> Vector2 {
        Self::fixed_vec_to_pixels(self.fixed_velocity)
    }

    #[doc(hidden)]
    pub fn set_position(&mut self, position: Vector2) {
        self.state.position = position;
        self.fixed_position = FixedVec2::from_ints(position.x, position.y);
    }

    fn set_velocity(&mut self, velocity: Vector2) {
        self.state.velocity = velocity;
        self.fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
    }

    fn set_velocity_preserving_subpixel(&mut self, velocity: Vector2) {
        fn preserve_axis(fixed: &mut C4Fixed, previous: i32, resolved: i32) {
            if resolved == previous {
                return;
            }
            if resolved == 0 {
                *fixed = C4Fixed::ZERO;
                return;
            }
            if previous != 0 && previous.signum() != resolved.signum() {
                *fixed = -*fixed;
                return;
            }
            *fixed = itofix(resolved);
        }

        let previous = self.state.velocity;
        preserve_axis(&mut self.fixed_velocity.x, previous.x, velocity.x);
        preserve_axis(&mut self.fixed_velocity.y, previous.y, velocity.y);
        self.refresh_velocity_from_fixed();
    }

    fn refresh_velocity_from_fixed(&mut self) {
        self.state.velocity = self.velocity_pixels();
    }

    fn shape_base_vertices(&self) -> &[ObjectVertex] {
        self.own_shape_vertices
            .as_deref()
            .unwrap_or(&self.shape_template.vertices)
    }

    fn unrotated_shape_vertices(&self) -> Vec<ObjectVertex> {
        if self.shape_template.line != 0 {
            return self.state.shape_vertices.active_vec();
        }
        transformed_shape_vertices(
            self.shape_base_vertices(),
            self.state.construction,
            self.shape_template.stretch_growth,
            0,
            0,
        )
    }

    #[doc(hidden)]
    pub fn current_shape_rect(&self) -> Option<DefinitionRect> {
        self.state.shape_override.or(self.shape_rect)
    }

    fn definition_derived_shape_rect(&self) -> Option<DefinitionRect> {
        if self.shape_template.line != 0 {
            return self.shape_template.rect;
        }
        transformed_shape_rect(
            self.shape_template.rect,
            self.state.construction,
            self.shape_template.stretch_growth,
            self.shape_template.rotateable,
            self.state.rotation,
        )
    }

    fn definition_derived_fire_top(&self) -> i32 {
        scaled_shape_fire_top(
            self.shape_template.fire_top,
            self.state.construction,
            self.shape_template.line,
        )
    }

    fn refresh_shape_after_state_change(
        &mut self,
        previous_construction: i32,
        previous_rect: Option<DefinitionRect>,
        preserve_bottom: bool,
    ) {
        self.refresh_shape_geometry();
        if self.shape_template.line != 0 {
            return;
        }

        let step_size = FULL_CON / 100;
        let previous_step = previous_construction / step_size;
        let current_step = self.state.construction / step_size;
        let step_diff = current_step - previous_step;
        let current_rect = self.current_shape_rect();
        let adjusted_y = if preserve_bottom {
            docon_adjusted_position_y(
                self.state.position.y,
                previous_rect,
                self.state.position.y,
                current_rect,
                self.state.rotation,
                self.state.category,
                previous_step,
                step_diff,
                self.shape_template.rect.map_or(0, |rect| rect.height),
            )
        } else {
            self.state.position.y
        };
        if adjusted_y != self.state.position.y {
            self.set_position(Vector2::new(self.state.position.x, adjusted_y));
        }
    }

    fn refresh_shape_geometry(&mut self) {
        if self.shape_template.line != 0 {
            // Line shape independent (C4Object.cpp:322-324).
            return;
        }
        // C4Shape::CopyFrom replaces AttachMat from the definition shape
        // while deliberately retaining iAttachX/Y/Vtx
        // (C4Shape.cpp:421-443). Definition shapes start unattached.
        self.state.shape_attach.mat_valid = false;
        self.state.shape_attach.mat_vehicle = false;
        self.state.shape_override = None;
        self.shape_rect = self.definition_derived_shape_rect();
        self.shape_fire_top = self.definition_derived_fire_top();
        let vertices = transformed_shape_vertices(
            self.shape_base_vertices(),
            self.state.construction,
            self.shape_template.stretch_growth,
            self.shape_template.rotateable,
            self.state.rotation,
        );
        self.state.shape_vertices.replace_active(&vertices);
        self.state.vertices = vertices;
        self.frame_vertex_contacts = vec![0; self.state.vertices.len()];
        self.frame_shape_contact_cnat = CNAT_NONE;
        self.frame_shape_contact_count = 0;
    }

    fn latch_shape_contact(&mut self, contact: &ShapeContact) {
        self.frame_t_contact = contact.contact_cnat;
        self.frame_shape_contact_cnat = contact.contact_cnat;
        self.frame_shape_contact_count = contact.count();
        self.frame_vertex_contacts
            .resize(self.state.vertices.len(), 0);
        for index in 0..self.state.vertices.len() {
            // Native CheckContact skips CNAT_NoCollision vertices without
            // touching their retained VtxContactCNAT slot.
            if self.state.vertices[index].cnat & CNAT_NO_COLLISION == 0 {
                self.frame_vertex_contacts[index] =
                    contact.vertex_contacts.get(index).copied().unwrap_or(0);
            }
        }
    }

    fn movement_attach(&self) -> u32 {
        if self.swim_exit_this_frame {
            CNAT_NONE
        } else {
            self.frame_t_attach
        }
    }

    fn live_contact_has_vertex_cnat(&self, cnat: u32) -> bool {
        self.frame_vertex_contacts
            .iter()
            .take(self.state.vertices.len())
            .any(|contact| contact & cnat != 0)
    }

    fn live_contact_first_friction(&self) -> i32 {
        self.state
            .vertices
            .iter()
            .zip(&self.frame_vertex_contacts)
            .find_map(|(vertex, &contact)| (contact != 0).then_some(vertex.friction))
            .unwrap_or(0)
    }

    fn live_contact_first_weight(&self) -> i32 {
        self.state
            .vertices
            .iter()
            .zip(&self.frame_vertex_contacts)
            .find_map(|(vertex, &contact)| (contact != 0).then_some(vertex.x.signum()))
            .unwrap_or(0)
    }

    fn set_owned_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.own_shape_vertices = Some(vertices);
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.refresh_shape_after_state_change(previous_construction, previous_rect, false);
    }

    fn set_live_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.state.shape_vertices.replace_active(&vertices);
        self.state.vertices = vertices;
        self.frame_vertex_contacts
            .resize(self.state.vertices.len(), 0);
    }

    fn set_shape_vertex_buffer(&mut self, vertices: ShapeVertexBuffer) {
        self.state.vertices = vertices.active_vec();
        self.state.shape_vertices = vertices;
        self.frame_vertex_contacts
            .resize(self.state.vertices.len(), 0);
    }

    fn set_construction(&mut self, construction: i32) {
        self.compiled_mass = None;
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.state.construction = construction.clamp(0, FULL_CON);
        self.refresh_shape_after_state_change(previous_construction, previous_rect, true);
    }

    fn set_construction_from_docon(&mut self, construction: i32) {
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        self.state.construction = construction.max(0);
        if !docon_refreshes_construction(previous_construction, self.state.construction) {
            return;
        }
        self.compiled_mass = None;
        self.refresh_shape_after_state_change(previous_construction, previous_rect, true);
    }

    fn remember_compiled_mass_contents(&mut self) {
        if self.compiled_mass.is_some() {
            self.compiled_mass_contents = self.state.contents.clone();
        }
    }

    #[doc(hidden)]
    pub fn set_fixed_velocity(&mut self, velocity: FixedVec2) {
        self.fixed_velocity = velocity;
        self.state.velocity = self.velocity_pixels();
    }

    /// Script/host-call snapshot: carries the TRUE sub-pixel dirs so the
    /// scope's GetXDir/GetYDir read the live C4Fixed values like C++
    /// (C4Script.cpp:1160-1180) instead of the int-quantized mirror.
    fn script_state_snapshot(&self) -> ObjectState {
        #[cfg(test)]
        SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let mut state = self.state.clone();
        // A handful of engine-internal temporary shape probes replace the
        // active Vec directly before dispatching callbacks. Preserve the raw
        // dormant tail, but make the script-visible prefix match that exact
        // live shape at call time.
        state.shape_vertices.replace_active(&state.vertices);
        state.script_fixed_position = Some(self.fixed_position);
        state.script_fixed_velocity = Some(self.fixed_velocity);
        state.script_rotation_velocity = Some(self.rotation_velocity);
        state.script_fixed_rotation = Some(self.fixed_rotation);
        state.shape_override = self.current_shape_rect();
        state
    }

    fn clamp_velocity(&mut self, physics: &PhysicsSettings) {
        physics.clamp_fixed_velocity(&mut self.fixed_velocity);
        self.refresh_velocity_from_fixed();
    }

    fn apply_delta(
        &mut self,
        delta: &ObjectDelta,
        action_library: &ActionLibrary,
    ) -> ApplyDeltaOutcome {
        if delta.change_def.is_some() {
            self.change_def_contents_sort = delta.change_def_contents_sort.clone();
        }
        if let Some(sequence) = delta.solid_mask_instance_sequence {
            self.solid_mask_instance_sequence = Some(sequence);
        } else if delta.change_def.is_some()
            || delta.solid_mask_override.is_some()
            || delta
                .base_graphics
                .as_ref()
                .is_some_and(|graphics| self.state.base_graphics.as_ref() != graphics.as_ref())
        {
            // ChangeDef, SetSolidMask and a real SetGraphics change delete
            // pSolidMaskData before UpdateSolidMask constructs a new tail
            // instance (C4Object.cpp:382-400,1207-1244,3809-3817).
            self.solid_mask_instance_sequence = None;
        }
        if let Some(compiler_cache) = &delta.compiler_cache {
            self.compiler_cache = compiler_cache.clone();
        }
        let previous_rect = self.current_shape_rect();
        let previous_construction = self.state.construction;
        let construction_refreshes_shape = delta.construction.is_some_and(|construction| {
            !delta.construction_via_docon
                || docon_refreshes_construction(previous_construction, construction.max(0))
        });
        if delta.own_mass.is_some() || delta.change_def.is_some() || construction_refreshes_shape {
            self.compiled_mass = None;
        }
        let shape_changed = construction_refreshes_shape
            || delta.rotation.is_some()
            || (delta.vertices.is_some() && !delta.vertices_defer_shape_update);
        // Kill-trace mark BEFORE the energy write (C4Object.cpp:1351-1361)
        // so AssignDeath credits the new cause.
        if let Some(cause) = delta.energy_loss_cause {
            self.last_energy_loss_cause = cause;
        }
        let outcome = self.state.apply_delta(delta, action_library);
        if let Some(position) = delta.position {
            self.fixed_position = FixedVec2::from_ints(position.x, position.y);
        }
        // A DoCon integer-y result is part of the live state seen by a later
        // merged SetAction. Install it before SetAction resynchronizes the
        // fixed coordinates; an explicit same-DoCon fixed result still wins
        // below for the inverse (SetAction-before-bottom-adjust) ordering.
        if let Some(position) = delta.resolved_docon_position {
            self.state.position = position;
        }
        // C4Object::SetAction resyncs the fixed coords to the integer
        // position once past its early returns (C4Object.cpp:4144).
        if outcome
            .action_change
            .as_ref()
            .map(|change| change.requested_name_change)
            .unwrap_or(false)
        {
            self.fixed_position =
                FixedVec2::from_ints(self.state.position.x, self.state.position.y);
        }
        if let Some(position) = delta.resolved_docon_fixed_position {
            self.fixed_position = position;
        }
        // No reprojection from the fixed coords otherwise: C++ x/y only
        // change via explicit assignment or movement — DoCon's initial
        // adjust legitimately leaves y and fixtoi(fix_y) split.
        if let Some(fixed_velocity) = delta.fixed_velocity {
            // Sub-pixel velocity is authoritative; derive the whole-pixel mirror
            // from it (matches C++ where xdir/ydir as C4Fixed are the source of
            // truth and the integer view is `fixtoi`).
            self.fixed_velocity = fixed_velocity;
            self.state.velocity = self.velocity_pixels();
        } else if let Some(velocity) = delta.velocity {
            self.fixed_velocity = FixedVec2::from_ints(velocity.x, velocity.y);
        } else {
            self.state.velocity = self.velocity_pixels();
        }
        // Component dir writes land on the TRUE fixed velocity — the
        // untouched component keeps its sub-pixel value (FnSetXDir only
        // assigns xdir, C4Script.cpp:697-705).
        if let Some(x) = delta.fixed_velocity_x {
            self.fixed_velocity.x = x;
            self.state.velocity = self.velocity_pixels();
        }
        if let Some(y) = delta.fixed_velocity_y {
            self.fixed_velocity.y = y;
            self.state.velocity = self.velocity_pixels();
        }
        // Script dir writes mobilize unconditionally: FnSetXDir/FnSetYDir/
        // FnSetRDir all end in `pObj->Mobile = 1` (C4Script.cpp:705,718,732).
        if delta.fixed_velocity.is_some()
            || delta.fixed_velocity_x.is_some()
            || delta.fixed_velocity_y.is_some()
            || delta.velocity.is_some()
            || delta.rotation_velocity.is_some()
        {
            self.state.mobile = true;
        }
        // Native object helpers can assign velocity without sharing the
        // script Set*Dir mobilization rule. Their explicit result wins over
        // the generic rule above (C4Object::Fling's Tumble branch preserves
        // Mobile, while Jump/raw fallback set it).
        if let Some(mobile) = delta.mobile {
            self.state.mobile = mobile;
        }
        if let Some(t_attach) = delta.t_attach {
            // Movement consumes the already-latched frame value after an
            // action callback; keep the serializable mirror in sync too.
            self.state.t_attach = t_attach;
            self.frame_t_attach = t_attach;
        }
        if delta.rotation.is_some() {
            // An explicit rotation re-seeds the fixed accumulator, mirroring C++
            // forcing `fix_r = itofix(r)` (`C4Movement.cpp:418`).
            self.fixed_rotation = itofix(self.state.rotation);
        }
        if let Some(rotation_velocity) = delta.rotation_velocity {
            self.rotation_velocity = rotation_velocity;
        }
        if let Some(vertices) = &delta.vertices {
            self.own_shape_vertices = Some(vertices.clone());
        }
        if shape_changed {
            let fixed_position = self.fixed_position;
            self.refresh_shape_after_state_change(
                previous_construction,
                previous_rect,
                delta.construction.is_some() && delta.resolved_docon_position.is_none(),
            );
            if delta.construction_preserves_fixed_position {
                // DoCon's straight-con bottom adjustment calls UpdatePos,
                // which updates sectors but never fix_x/fix_y
                // (C4Object.cpp:1462-1495, C4Object::UpdatePos at :346-354).
                self.fixed_position = fixed_position;
            }
        }
        if let Some(shape_override) = delta.shape_override {
            self.state.shape_override = shape_override;
            match shape_override {
                Some(rect) => self.shape_rect = Some(rect),
                None if !shape_changed => self.refresh_shape_geometry(),
                None => {}
            }
        }
        if let Some(material_contents) = &delta.material_contents {
            self.material_contents = material_contents.clone();
        }
        if let Some(vertices) = &delta.live_vertices {
            self.set_live_shape_vertices(vertices.clone());
        }
        if let Some(vertices) = &delta.shape_vertices {
            self.set_shape_vertex_buffer(vertices.clone());
        }
        outcome
    }

    fn advance_fixed_position(&mut self) {
        self.fixed_position += self.fixed_velocity;
        self.state.position = self.position_pixels();
        self.state.velocity = self.velocity_pixels();
    }

    fn advance_fixed_position_per_pixel(
        &mut self,
        landscape: Option<&mut Landscape>,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        mut on_contact: impl FnMut(
            &mut Object,
            &Landscape,
            MovementContactDispatch,
        ) -> Result<(), EngineError>,
        mut on_do_motion: impl FnMut(&mut Object, &mut Landscape) -> Result<(), EngineError>,
    ) -> Result<MovementStepOutcome, EngineError> {
        let Some(landscape) = landscape else {
            let previous_position = self.state.position;
            self.advance_fixed_position();
            self.motion_x = self
                .motion_x
                .saturating_add(self.state.position.x - previous_position.x);
            self.motion_y = self
                .motion_y
                .saturating_add(self.state.position.y - previous_position.y);
            return Ok(MovementStepOutcome {
                no_attach: false,
                redirect_yr: false,
                any_contact: false,
                contact_cnat: 0,
                solid_mask_removed: self.state.position != previous_position,
            });
        };

        if self.movement_attach() != CNAT_NONE {
            return self.advance_attached_shape_position(
                landscape,
                materials,
                movement,
                false,
                &mut on_contact,
                &mut on_do_motion,
            );
        }

        let mut outcome = MovementStepOutcome::default();
        let mut solid_mask_removed = false;
        self.fixed_position.x += self.fixed_velocity.x;
        let mut target_x = fixtoi(self.fixed_position.x);
        self.apply_side_bounds(&mut target_x, landscape, movement, &mut on_contact)?;
        while self.state.position.x != target_x {
            let next_x = self.state.position.x + sign_i32(target_x - self.state.position.x);
            let candidate = Vector2::new(next_x, self.state.position.y);
            let excluded_solid_mask = solid_mask_removed.then_some(movement.object_id);
            let contact = shape_contact_check(
                &self.state.vertices,
                candidate,
                landscape,
                materials,
                movement.solid_masks,
                excluded_solid_mask,
                self.state.contact_density,
            );
            // ContactCheck writes t_contact on every probe, including a
            // free probe (zero). Commands on the next frame observe this
            // latch rather than re-probing the resting position.
            self.latch_shape_contact(&contact);
            if contact.is_contact() {
                if coach_debug_id() == Some(movement.object_id.as_u64()) {
                    let details: Vec<String> = self
                        .state
                        .vertices
                        .iter()
                        .map(|vertex| {
                            let vx = candidate.x + vertex.x;
                            let vy = candidate.y + vertex.y;
                            let density = movement_density_at(
                                landscape,
                                materials,
                                movement.solid_masks,
                                excluded_solid_mask,
                                vx,
                                vy,
                            );
                            format!(
                                "v({},{})@({vx},{vy}) d={density} byte={:?}",
                                vertex.x,
                                vertex.y,
                                landscape.grid_byte_at(vx, vy)
                            )
                        })
                        .collect();
                    crate::rng::rng_trace_line(&format!(
                        "HCONTACT cand=({},{}) excl={:?} masks={} {}",
                        candidate.x,
                        candidate.y,
                        excluded_solid_mask,
                        movement.solid_masks.len(),
                        details.join(" | ")
                    ));
                }
                on_contact(self, landscape, MovementContactDispatch::ShapeProbe)?;
                if self.frame_shape_contact_count != 0 {
                    outcome.any_contact = true;
                    outcome.contact_cnat |= self.frame_t_contact;
                    self.fixed_position.x = itofix(self.state.position.x);
                    redirect_force(&mut self.fixed_velocity.x, &mut self.fixed_velocity.y, -1);
                    let friction = self.live_contact_first_friction();
                    apply_contact_friction(&mut self.fixed_velocity.y, friction);
                    break;
                }
            }
            if !solid_mask_removed {
                on_do_motion(self, landscape)?;
            }
            self.motion_x = self.motion_x.saturating_add(next_x - self.state.position.x);
            self.state.position.x = next_x;
            solid_mask_removed = true;
        }

        self.fixed_position.y += self.fixed_velocity.y;
        let mut target_y = fixtoi(self.fixed_position.y);
        self.apply_vertical_bounds(&mut target_y, landscape, movement, &mut on_contact)?;
        while self.state.position.y != target_y {
            let next_y = self.state.position.y + sign_i32(target_y - self.state.position.y);
            let candidate = Vector2::new(self.state.position.x, next_y);
            let excluded_solid_mask = solid_mask_removed.then_some(movement.object_id);
            let contact = shape_contact_check(
                &self.state.vertices,
                candidate,
                landscape,
                materials,
                movement.solid_masks,
                excluded_solid_mask,
                self.state.contact_density,
            );
            self.latch_shape_contact(&contact);
            if contact.is_contact() {
                if coach_debug_id() == Some(movement.object_id.as_u64()) {
                    crate::rng::rng_trace_line(&format!(
                        "VCONTACT cand=({},{}) first_friction={} xdir_before={} ydir_before={}",
                        candidate.x,
                        candidate.y,
                        contact.first_friction(),
                        self.fixed_velocity.x.val(),
                        self.fixed_velocity.y.val()
                    ));
                }
                on_contact(self, landscape, MovementContactDispatch::ShapeProbe)?;
                let contact_count = self.frame_shape_contact_count;
                if contact_count != 0 {
                    outcome.any_contact = true;
                    outcome.contact_cnat |= self.frame_t_contact;
                    self.fixed_position.y = itofix(self.state.position.y);
                    let friction = self.live_contact_first_friction();
                    apply_contact_friction(&mut self.fixed_velocity.x, friction);
                    if !self.live_contact_has_vertex_cnat(CNAT_LEFT) {
                        redirect_force(&mut self.fixed_velocity.y, &mut self.fixed_velocity.x, -1);
                    } else if !self.live_contact_has_vertex_cnat(CNAT_RIGHT) {
                        redirect_force(&mut self.fixed_velocity.y, &mut self.fixed_velocity.x, 1);
                    } else {
                        if self.state.ocf & crate::ocf::ROTATE != 0
                            && contact_count == 1
                            && !self.state.alive
                        {
                            let weight = self.live_contact_first_weight();
                            redirect_force(
                                &mut self.fixed_velocity.y,
                                &mut self.rotation_velocity,
                                -weight,
                            );
                            // C++ fRedirectYR remembers this arm for the rest of
                            // this DoMovement, even when no force was transferred.
                            outcome.redirect_yr = true;
                        }
                        self.fixed_velocity.y = C4Fixed::ZERO;
                    }
                    break;
                }
            }
            if !solid_mask_removed {
                on_do_motion(self, landscape)?;
            }
            self.motion_y = self.motion_y.saturating_add(next_y - self.state.position.y);
            self.state.position.y = next_y;
            solid_mask_removed = true;
        }

        outcome.solid_mask_removed = solid_mask_removed;
        if self.movement_attach() != CNAT_NONE {
            let attached = self.advance_attached_shape_position(
                landscape,
                materials,
                movement,
                solid_mask_removed,
                &mut on_contact,
                &mut on_do_motion,
            )?;
            outcome.no_attach |= attached.no_attach;
            outcome.redirect_yr |= attached.redirect_yr;
            outcome.any_contact |= attached.any_contact;
            outcome.contact_cnat |= attached.contact_cnat;
            outcome.solid_mask_removed |= attached.solid_mask_removed;
        }
        self.state.velocity = self.velocity_pixels();
        Ok(outcome)
    }

    fn advance_attached_shape_position(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        initial_solid_mask_removed: bool,
        on_contact: &mut impl FnMut(
            &mut Object,
            &Landscape,
            MovementContactDispatch,
        ) -> Result<(), EngineError>,
        on_do_motion: &mut impl FnMut(&mut Object, &mut Landscape) -> Result<(), EngineError>,
    ) -> Result<MovementStepOutcome, EngineError> {
        self.fixed_position += self.fixed_velocity;
        let mut target_x = fixtoi(self.fixed_position.x);
        let mut target_y = fixtoi(self.fixed_position.y);
        self.apply_side_bounds(&mut target_x, landscape, movement, on_contact)?;
        self.apply_vertical_bounds(&mut target_y, landscape, movement, on_contact)?;

        let mut no_attach = false;
        let mut any_contact = false;
        let mut contact_cnat = CNAT_NONE;
        let mut solid_mask_removed = initial_solid_mask_removed;
        let mut first_step = true;
        while first_step || self.state.position.x != target_x || self.state.position.y != target_y {
            first_step = false;
            let original = Vector2::new(
                self.state.position.x + sign_i32(target_x - self.state.position.x),
                self.state.position.y + sign_i32(target_y - self.state.position.y),
            );
            let mut candidate = original;
            if !shape_attach(
                &self.state.vertices,
                &mut candidate,
                self.movement_attach(),
                landscape,
                materials,
                movement.solid_masks,
                solid_mask_removed.then_some(movement.object_id),
                self.state.contact_density,
                &mut self.state.shape_attach,
            ) {
                no_attach = true;
            }

            let contact = shape_contact_check(
                &self.state.vertices,
                candidate,
                landscape,
                materials,
                movement.solid_masks,
                solid_mask_removed.then_some(movement.object_id),
                self.state.contact_density,
            );
            self.latch_shape_contact(&contact);
            if contact.is_contact() {
                if coach_debug_id() == Some(movement.object_id.as_u64()) {
                    crate::rng::rng_trace_line(&format!(
                        "ACONTACT cand=({},{}) orig=({},{}) attach={} pos=({},{})",
                        candidate.x,
                        candidate.y,
                        original.x,
                        original.y,
                        self.movement_attach(),
                        self.state.position.x,
                        self.state.position.y
                    ));
                }
                on_contact(self, landscape, MovementContactDispatch::ShapeProbe)?;
                if self.frame_shape_contact_count != 0 {
                    any_contact = true;
                    contact_cnat |= self.frame_t_contact;
                    self.fixed_position =
                        FixedVec2::from_ints(self.state.position.x, self.state.position.y);
                    // The at_xovr/at_yovr override bookkeeping runs AFTER the
                    // contact arm, UNCONDITIONALLY (C4Movement.cpp:363-368):
                    // an attach-adjusted step that then contacts still zeroes
                    // the dirs on the adjusted axes — the resting coach's
                    // re-arm gravity quantum dies here and the wagon
                    // demobilizes (rust f138 wall: ydir 13107/Mobile alive
                    // where cpp rested at 0/false).
                    if candidate.x != original.x {
                        self.fixed_velocity.x = C4Fixed::ZERO;
                    }
                    if candidate.y != original.y {
                        self.fixed_velocity.y = C4Fixed::ZERO;
                    }
                    break;
                }
            }

            // NO candidate==position early exit: C++'s do-loop still runs
            // the ctco override bookkeeping when the attach-corrected step
            // lands exactly on the current position (DoMotion(0,0) is a
            // no-op but at_xovr/at_yovr zero the dirs, resync the fixed
            // coords and retarget ctco to the current pixel so the while
            // condition exits, C4Movement.cpp:337-369). Skipping it left a
            // freshly-turned walker's Turn-carried ydir alive (the
            // GoldRush snake f57 wall: rust fell ballistic at fix_y
            // 383.8/ydir 0.8 where cpp settled at 383.0/0).
            let override_x = candidate.x != original.x;
            let override_y = candidate.y != original.y;
            if (override_x || override_y) && coach_debug_id() == Some(movement.object_id.as_u64()) {
                crate::rng::rng_trace_line(&format!(
                    "ATTOVR cand=({},{}) orig=({},{}) attach={}",
                    candidate.x,
                    candidate.y,
                    original.x,
                    original.y,
                    self.movement_attach()
                ));
            }
            if !solid_mask_removed {
                on_do_motion(self, landscape)?;
            }
            self.motion_x = self
                .motion_x
                .saturating_add(candidate.x - self.state.position.x);
            self.motion_y = self
                .motion_y
                .saturating_add(candidate.y - self.state.position.y);
            self.state.position = candidate;

            if override_x {
                target_x = self.state.position.x;
                self.fixed_velocity.x = C4Fixed::ZERO;
                self.fixed_position.x = itofix(self.state.position.x);
            }
            if override_y {
                target_y = self.state.position.y;
                self.fixed_velocity.y = C4Fixed::ZERO;
                self.fixed_position.y = itofix(self.state.position.y);
            }
            solid_mask_removed = true;
        }

        self.state.velocity = self.velocity_pixels();
        Ok(MovementStepOutcome {
            no_attach,
            redirect_yr: false,
            any_contact,
            contact_cnat,
            solid_mask_removed,
        })
    }

    fn apply_side_bounds(
        &mut self,
        target_x: &mut i32,
        landscape: &Landscape,
        movement: MovementContactConfig<'_>,
        on_contact: &mut impl FnMut(
            &mut Object,
            &Landscape,
            MovementContactDispatch,
        ) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let live = movement.live.get();
        if let Some(layer) = live.layer_bounds {
            if layer.border_bound & C4D_BORDER_LAYER != 0
                && !matches!(live.action_procedure, ActionProcedure::Attach)
            {
                let shape_x = self.current_shape_rect().map(|shape| shape.x).unwrap_or(0);
                let (low, high) = if self.state.category & CATEGORY_STATIC_BACK != 0 {
                    (
                        layer.position.x + layer.shape_rect.x,
                        layer.position.x + layer.shape_rect.x + layer.shape_rect.width,
                    )
                } else {
                    (
                        layer.position.x + layer.shape_rect.x - shape_x,
                        layer.position.x + layer.shape_rect.x + layer.shape_rect.width + shape_x,
                    )
                };
                for cnat in target_bounds(target_x, low, high, CNAT_LEFT, CNAT_RIGHT)
                    .into_iter()
                    .flatten()
                {
                    self.fixed_velocity.x = C4Fixed::ZERO;
                    self.state.velocity = self.velocity_pixels();
                    on_contact(self, landscape, MovementContactDispatch::Direct(cnat))?;
                }
            }
        }

        if movement.live.get().border_bound & C4D_BORDER_SIDES == 0 {
            return Ok(());
        }
        let shape_x = self.current_shape_rect().map(|shape| shape.x).unwrap_or(0);
        for cnat in target_bounds(
            target_x,
            -shape_x,
            landscape.width() as i32 + shape_x,
            CNAT_LEFT,
            CNAT_RIGHT,
        )
        .into_iter()
        .flatten()
        {
            self.fixed_velocity.x = C4Fixed::ZERO;
            self.state.velocity = self.velocity_pixels();
            on_contact(self, landscape, MovementContactDispatch::Direct(cnat))?;
        }
        Ok(())
    }

    fn apply_vertical_bounds(
        &mut self,
        target_y: &mut i32,
        landscape: &Landscape,
        movement: MovementContactConfig<'_>,
        on_contact: &mut impl FnMut(
            &mut Object,
            &Landscape,
            MovementContactDispatch,
        ) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let live = movement.live.get();
        if let Some(layer) = live.layer_bounds {
            if layer.border_bound & C4D_BORDER_LAYER != 0
                && !matches!(live.action_procedure, ActionProcedure::Attach)
            {
                let shape_y = self.current_shape_rect().map(|shape| shape.y).unwrap_or(0);
                let (low, high) = if self.state.category & CATEGORY_STATIC_BACK != 0 {
                    (
                        layer.position.y + layer.shape_rect.y,
                        layer.position.y + layer.shape_rect.y + layer.shape_rect.height,
                    )
                } else {
                    (
                        layer.position.y + layer.shape_rect.y - shape_y,
                        layer.position.y + layer.shape_rect.y + layer.shape_rect.height + shape_y,
                    )
                };
                for cnat in target_bounds(target_y, low, high, CNAT_TOP, CNAT_BOTTOM)
                    .into_iter()
                    .flatten()
                {
                    self.fixed_velocity.y = C4Fixed::ZERO;
                    self.state.velocity = self.velocity_pixels();
                    on_contact(self, landscape, MovementContactDispatch::Direct(cnat))?;
                }
            }
        }

        if movement.live.get().border_bound & C4D_BORDER_TOP != 0 {
            let shape_y = self.current_shape_rect().map(|shape| shape.y).unwrap_or(0);
            for cnat in target_bounds(target_y, -shape_y, 1_000_000, CNAT_TOP, CNAT_BOTTOM)
                .into_iter()
                .flatten()
            {
                self.fixed_velocity.y = C4Fixed::ZERO;
                self.state.velocity = self.velocity_pixels();
                on_contact(self, landscape, MovementContactDispatch::Direct(cnat))?;
            }
        }
        if movement.live.get().border_bound & C4D_BORDER_BOTTOM != 0 {
            let shape_y = self.current_shape_rect().map(|shape| shape.y).unwrap_or(0);
            let bottom = landscape.estimated_height() + shape_y;
            for cnat in target_bounds(target_y, -1_000_000, bottom, CNAT_TOP, CNAT_BOTTOM)
                .into_iter()
                .flatten()
            {
                self.fixed_velocity.y = C4Fixed::ZERO;
                self.state.velocity = self.velocity_pixels();
                on_contact(self, landscape, MovementContactDispatch::Direct(cnat))?;
            }
        }
        self.state.velocity = self.velocity_pixels();
        Ok(())
    }

    /// Accumulate angular velocity into the fixed rotation, mirroring the fixed
    /// state pieces of C++ `C4Movement.cpp:373-436`: the rotation block is
    /// gated by the cached OCF_Rotate bit. Otherwise it applies `fix_r +=
    /// rdir * 5`; finite `Def->Rotateable` ranges clamp
    /// `fix_r`/zero `rdir`, then contact-aware rotation walks one degree at a
    /// time before the half-circle wrap projects the integer degree.
    fn advance_fixed_rotation(
        &mut self,
        landscape: Option<&Landscape>,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        no_attach: bool,
        redirect_yr: bool,
        solid_mask_removed: bool,
        mut on_contact: impl FnMut(
            &mut Object,
            &Landscape,
            MovementContactDispatch,
        ) -> Result<(), EngineError>,
    ) -> Result<(bool, u32), EngineError> {
        if self.state.ocf & crate::ocf::ROTATE == 0 {
            return Ok((false, 0));
        }
        if !self.rotation_velocity.is_nonzero() {
            return Ok((false, 0));
        }
        self.fixed_rotation += self.rotation_velocity * 5;
        let rotateable = movement.live.get().rotateable;
        if rotateable > 1 {
            let limit = itofix(rotateable);
            if self.fixed_rotation > limit {
                self.fixed_rotation = limit;
                self.rotation_velocity = C4Fixed::ZERO;
            }
            if self.fixed_rotation < -limit {
                self.fixed_rotation = -limit;
                self.rotation_velocity = C4Fixed::ZERO;
            }
        }

        let target_rotation = fixtoi(self.fixed_rotation);
        let mut contact_outcome = (false, 0);
        if let Some(landscape) = landscape {
            contact_outcome = self.advance_fixed_rotation_with_contact(
                target_rotation,
                landscape,
                materials,
                movement,
                no_attach,
                redirect_yr,
                solid_mask_removed,
                &mut on_contact,
            )?;
        } else {
            let changed = self.state.rotation != target_rotation;
            self.state.rotation = target_rotation;
            if changed && self.shape_template.line == 0 {
                self.refresh_shape_geometry();
            }
        }

        // Circle bounds change raw r/fix_r only. C++ does not UpdateShape
        // after this wrap, so a 360° step retains the enlarged live Shape
        // produced immediately before r becomes zero (C4Movement.cpp:434-435).
        let half_circle = itofix(FIX_HALF_CIRCLE);
        let full_circle = itofix(FIX_FULL_CIRCLE);
        if self.fixed_rotation < -half_circle {
            self.fixed_rotation += full_circle;
            self.state.rotation = fixtoi(self.fixed_rotation);
        }
        if self.fixed_rotation > half_circle {
            self.fixed_rotation -= full_circle;
            self.state.rotation = fixtoi(self.fixed_rotation);
        }
        Ok(contact_outcome)
    }

    fn advance_fixed_rotation_with_contact(
        &mut self,
        target_rotation: i32,
        landscape: &Landscape,
        materials: &MaterialSet,
        movement: MovementContactConfig<'_>,
        no_attach: bool,
        redirect_yr: bool,
        solid_mask_removed: bool,
        on_contact: &mut impl FnMut(
            &mut Object,
            &Landscape,
            MovementContactDispatch,
        ) -> Result<(), EngineError>,
    ) -> Result<(bool, u32), EngineError> {
        let mut any_contact = false;
        let mut contact_cnat = 0;
        while self.state.rotation != target_rotation {
            let previous_rotation = self.state.rotation;
            let previous_vertices = self.state.vertices.clone();
            let previous_shape_vertices = self.state.shape_vertices.clone();
            let previous_shape_rect = self.shape_rect;
            let previous_fire_top = self.shape_fire_top;
            let previous_shape_override = self.state.shape_override;
            let previous_vertex_contacts = self.frame_vertex_contacts.clone();
            let previous_shape_contact_cnat = self.frame_shape_contact_cnat;
            let previous_shape_contact_count = self.frame_shape_contact_count;
            let previous_position = self.state.position;
            // `C4Shape lshape = Shape` — the contact undo restores the
            // attach record too (C4Movement.cpp:395-417).
            let previous_attach = self.state.shape_attach;

            self.state.rotation += sign_i32(target_rotation - self.state.rotation);
            if self.shape_template.line == 0 {
                self.refresh_shape_geometry();
            }

            let mut candidate_position = self.state.position;
            let attach = self.movement_attach();
            if attach != CNAT_NONE && !no_attach {
                shape_attach(
                    &self.state.vertices,
                    &mut candidate_position,
                    attach,
                    landscape,
                    materials,
                    movement.solid_masks,
                    solid_mask_removed.then_some(movement.object_id),
                    self.state.contact_density,
                    &mut self.state.shape_attach,
                );
            }

            let contact = shape_contact_check(
                &self.state.vertices,
                candidate_position,
                landscape,
                materials,
                movement.solid_masks,
                solid_mask_removed.then_some(movement.object_id),
                self.state.contact_density,
            );
            self.latch_shape_contact(&contact);
            if contact.is_contact() {
                on_contact(self, landscape, MovementContactDispatch::ShapeProbe)?;
                let contact_count = self.frame_shape_contact_count;
                if contact_count != 0 {
                    any_contact = true;
                    contact_cnat |= self.frame_t_contact;
                    self.state.rotation = previous_rotation;
                    self.state.vertices = previous_vertices;
                    self.state.shape_vertices = previous_shape_vertices;
                    self.shape_rect = previous_shape_rect;
                    self.shape_fire_top = previous_fire_top;
                    self.state.shape_override = previous_shape_override;
                    self.frame_vertex_contacts = previous_vertex_contacts;
                    self.frame_shape_contact_cnat = previous_shape_contact_cnat;
                    self.frame_shape_contact_count = previous_shape_contact_count;
                    self.state.position = previous_position;
                    self.state.shape_attach = previous_attach;
                    // The rotation walk only restores Shape/r/fix_r on contact;
                    // fix_x/fix_y retain the preceding translation result
                    // (C4Movement.cpp:392-425).
                    self.fixed_rotation = itofix(previous_rotation);
                    if contact_count == 1 && !redirect_yr {
                        redirect_force(&mut self.rotation_velocity, &mut self.fixed_velocity.y, -1);
                    }
                    self.rotation_velocity = C4Fixed::ZERO;
                    self.refresh_velocity_from_fixed();
                    break;
                }
            }

            self.state.position = candidate_position;
            // Successful attached turns may shift integer x/y, but C++ does
            // not fold that transient Shape.Attach correction into fix_x/y.
        }
        Ok((any_contact, contact_cnat))
    }

    fn apply_command_operations<I>(&mut self, operations: I)
    where
        I: IntoIterator<Item = CommandOperation>,
    {
        for operation in operations {
            match operation {
                // C4Object::ClearCommands removes the complete native
                // command chain. The compatibility queue contains deferred
                // continuations for that same chain, so entries already
                // present at this chronological Clear must disappear too.
                CommandOperation::Clear => {
                    self.commands.clear();
                    self.command_queue.clear();
                }
                CommandOperation::PushFront(request) => {
                    let _ = self.commands.push_front(request);
                }
                CommandOperation::PushBack(request) => {
                    let _ = self.commands.push_back(request);
                }
                CommandOperation::Finish { index, success } => {
                    self.commands.finish_entry_public(index, success);
                }
                CommandOperation::DecrementNoCollectDelay => {
                    // C4Object::SetCommand entry (C4Object.cpp:3941-3942).
                    if self.state.no_collect_delay > 0 {
                        self.state.no_collect_delay -= 1;
                    }
                }
                CommandOperation::SetNoCollectDelay { value, ocf } => {
                    self.state.no_collect_delay = value;
                    self.state.ocf = ocf;
                }
                CommandOperation::Restore(snapshot) => {
                    self.commands.restore_from_snapshot(&snapshot);
                }
            }
        }
    }

    fn step_command_stack(
        &mut self,
        ctx: CommandRuntimeContext<'_>,
        gravity: C4Fixed,
    ) -> Option<CommandStepResult> {
        self.commands.execute_front_with_gravity(&ctx, gravity)
    }

    #[doc(hidden)]
    pub fn mark_destroyed(&mut self) -> Vec<EffectEvent> {
        if self.destroyed {
            return Vec::new();
        }
        self.destroyed = true;
        self.state.status = ObjectStatus::Deleted;
        self.drain_effects_with_reason(EffectStopReason::Destroyed)
    }

    fn snapshot(&self, library: Option<&ActionLibrary>) -> ObjectSnapshot {
        let procedure = library
            .and_then(|lib| {
                lib.procedure_name_for_entry(
                    &self.state.action.name,
                    self.state.action.act_map_index,
                )
            })
            .map(|name| name.to_string());
        // The INT position is the sim-state x/y (C++ exports object->x/y);
        // it may legitimately differ from fixtoi(fix) after the DoCon
        // initial split — the fixed coords travel separately below.
        let position = self.state.position;
        let velocity = self.velocity_pixels();
        let rotation_velocity = self
            .rotation_velocity
            .is_nonzero()
            .then_some(self.rotation_velocity);
        // C++ always persists fix_r. Omit it only when reconstruction from
        // raw `r` is lossless; a stopped object may still retain a fractional
        // accumulator from its last rotation step.
        let fixed_rotation =
            (self.fixed_rotation != itofix(self.state.rotation)).then_some(self.fixed_rotation);
        let current_shape = self.current_shape_rect();
        let current_shape = (current_shape != self.definition_derived_shape_rect())
            .then_some(current_shape)
            .flatten();
        let current_fire_top = (self.shape_fire_top != self.definition_derived_fire_top())
            .then_some(self.shape_fire_top);
        ObjectSnapshot {
            id: self.id,
            definition_id: self.definition_id.clone(),
            custom_name: self.state.custom_name.clone(),
            position,
            velocity,
            // C++ persists `r` verbatim; DoMovement keeps active rotation in
            // (-180, 180], so a left lean remains negative across a save.
            rotation: self.state.rotation,
            energy: self.state.energy,
            need_energy: self.state.need_energy,
            damage: self.state.damage,
            magic_energy: self.state.magic_energy,
            magic_capacity: self.state.magic_capacity,
            construction: self.state.construction,
            action: self.state.action.clone(),
            direction: self.state.direction,
            command_direction: self.state.command_direction,
            action_procedure: procedure,
            effects: self.state.effects.clone(),
            vertices: self.state.vertices.clone(),
            current_shape,
            current_fire_top,
            contact_density: self.state.contact_density,
            own_vertices: self.own_shape_vertices.clone(),
            vertex_contacts: self.frame_vertex_contacts.clone(),
            solid_mask_override: self.state.solid_mask_override,
            container: self.state.container,
            layer: self.state.layer,
            visibility: self.state.visibility,
            blit_mode: self.state.blit_mode,
            color: self.state.color,
            color_modulation: self.state.color_modulation,
            picture_rect: self.state.picture_rect,
            contents: self.state.contents.clone(),
            components: self.state.components.clone(),
            component_order: self.state.component_order.clone(),
            status: self.state.status,
            owner: self.state.owner,
            base: self.state.base,
            controller: self.state.controller,
            category: self.state.category,
            crew_member: self.state.crew_member,
            plr_view_range: self.state.plr_view_range,
            selected: self.state.selected,
            alive: self.state.alive,
            base_graphics: self.state.base_graphics.clone(),
            graphics_overlays: self.state.graphics_overlays.clone(),
            draw_transform: self.state.draw_transform,
            command_queue: self.command_queue.iter().cloned().collect(),
            command_stack: self.commands.snapshot(),
            local_vars: self.state.local_vars.clone(),
            in_liquid: self.state.in_liquid,
            mobile: self.state.mobile,
            ocf: self.state.ocf,
            timer: self.state.timer,
            own_mass: self.state.own_mass,
            on_fire: self.state.on_fire,
            fire_phase: self.state.fire_phase,
            fire_caused_by: self.state.fire_caused_by,
            info_physical: self.state.info_physical,
            temporary_physical: self.state.temporary_physical,
            physical_changes: self.state.physical_changes.clone(),
            breath: self.state.breath,
            last_energy_loss_cause: self.last_energy_loss_cause,
            fixed_position: subpixel_or_none(self.fixed_position, position),
            fixed_velocity: subpixel_or_none(self.fixed_velocity, velocity),
            rotation_velocity,
            fixed_rotation,
        }
    }

    fn apply_effect_commands(&mut self, commands: &[EffectCommand]) -> Vec<EffectEvent> {
        let mut events = Vec::new();
        for command in commands {
            match command {
                EffectCommand::Add {
                    effect,
                    constructor_values,
                } => {
                    let is_update = effect.number > 0
                        && self
                            .state
                            .effects
                            .iter()
                            .any(|existing| existing.number == effect.number);
                    let (inserted, _) = self.insert_effect(effect.clone());
                    if !is_update {
                        events.push(EffectEvent::started(
                            inserted,
                            constructor_values
                                .clone()
                                .unwrap_or_else(|| std::array::from_fn(|_| Value::Nil)),
                        ));
                    }
                }
                EffectCommand::Update(effect) => {
                    if let Some(existing) = self
                        .state
                        .effects
                        .iter_mut()
                        .find(|existing| existing.number == effect.number)
                    {
                        *existing = effect.clone();
                    }
                }
                EffectCommand::Remove { name, no_callbacks } => {
                    if let Some(removed) = self.mark_effect_dead(name) {
                        if !no_callbacks {
                            events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
                        }
                    }
                }
                EffectCommand::RemoveNumber {
                    number,
                    no_callbacks,
                } => {
                    if let Some(removed) = self.mark_effect_dead_by_number(*number) {
                        if !no_callbacks {
                            events.push(EffectEvent::stopped(removed, EffectStopReason::Removed));
                        }
                    }
                }
                EffectCommand::UnlinkNumber { number } => {
                    self.remove_effect_by_number(*number);
                }
                EffectCommand::Clear => {
                    events.extend(self.drain_effects_with_reason(EffectStopReason::Cleared));
                }
            }
        }
        events
    }

    fn ensure_material_capacity(&mut self, count: usize) {
        if self.material_contents.len() < count {
            self.material_contents.resize(count, 0);
        }
    }

    fn material_content(&self, material: MaterialId) -> i32 {
        let index = material.index();
        self.material_contents.get(index).copied().unwrap_or(0)
    }

    fn set_material_content(&mut self, material: MaterialId, amount: i32) {
        let index = material.index();
        if self.material_contents.len() <= index {
            self.material_contents.resize(index + 1, 0);
        }
        self.material_contents[index] = amount.max(0);
    }

    fn add_material_content(&mut self, material: MaterialId, amount: i32) {
        if amount <= 0 {
            return;
        }
        let index = material.index();
        if self.material_contents.len() <= index {
            self.material_contents.resize(index + 1, 0);
        }
        let slot = &mut self.material_contents[index];
        *slot = slot.saturating_add(amount);
    }

    fn apply_material_interaction(&mut self, material: &Material) {
        let friction = material.friction();
        if friction != 0 {
            self.fixed_velocity.x =
                apply_horizontal_friction_fixed(self.fixed_velocity.x, friction);
            self.refresh_velocity_from_fixed();
            for vertex in &mut self.state.vertices {
                vertex.friction = friction;
            }
        }
    }

    fn apply_collision_resolution(&mut self, resolution: &CollisionResolution) {
        self.set_position(resolution.position);
        self.set_velocity_preserving_subpixel(resolution.velocity);
    }

    // iIntervall/iTime are stored verbatim (C4Effect.cpp:66-67) - a zero
    // interval means the timer never fires.
    /// C4Effect::New semantics: same-name effects COEXIST; each gets a
    /// per-object monotonic number (max existing + 1, C4Effect.cpp:76-78).
    /// A carried nonzero number matching an existing effect is an UPDATE
    /// (EffectVar writes fold back through the add command).
    fn insert_effect(&mut self, mut effect: EffectState) -> (EffectState, Option<EffectState>) {
        if effect.timer < 0 {
            effect.timer = 0;
        }
        if effect.number > 0 {
            if let Some(existing) = self
                .state
                .effects
                .iter_mut()
                .find(|existing| existing.number == effect.number)
            {
                *existing = effect.clone();
                return (effect, None);
            }
        }
        if effect.number == 0 {
            effect.number = self
                .state
                .effects
                .iter()
                .map(|existing| existing.number)
                .max()
                .unwrap_or(0)
                .saturating_add(1)
                .max(1);
        }
        // Equal-priority nodes are newest-first. A fresh max+1 number still
        // inserts at the front, while a carried number (for example a denied
        // Kill victim) returns to its original position among its peers.
        let priority = effect.priority.unsigned_abs();
        let mut insert_pos = 0;
        while insert_pos < self.state.effects.len() {
            let existing = &self.state.effects[insert_pos];
            let existing_priority = existing.priority.unsigned_abs();
            if existing_priority > priority
                || (existing_priority == priority && existing.number < effect.number)
            {
                break;
            }
            insert_pos += 1;
        }
        let inserted = effect.clone();
        self.state.effects.insert(insert_pos, effect);
        (inserted, None)
    }

    fn mark_effect_dead(&mut self, name: &str) -> Option<EffectState> {
        let effect = self
            .state
            .effects
            .iter_mut()
            .find(|effect| effect.name == name && effect.priority != 0)?;
        let stopped = effect.clone();
        effect.priority = 0;
        Some(stopped)
    }

    fn mark_effect_dead_by_number(&mut self, number: i32) -> Option<EffectState> {
        let effect = self
            .state
            .effects
            .iter_mut()
            .find(|effect| effect.number == number && effect.priority != 0)?;
        let stopped = effect.clone();
        effect.priority = 0;
        Some(stopped)
    }

    /// Remove THE effect (by its C++ identity, iNumber — names may repeat).
    fn remove_effect_by_number(&mut self, number: i32) -> Option<EffectState> {
        self.state
            .effects
            .iter()
            .position(|existing| existing.number == number)
            .map(|index| self.state.effects.remove(index))
    }

    fn drain_effects_with_reason(&mut self, reason: EffectStopReason) -> Vec<EffectEvent> {
        self.state
            .effects
            .drain(..)
            // C4Effect::ClearAll recurses through pNext before stopping its
            // own node, so every whole-list clear runs tail-to-head.
            .rev()
            .map(|effect| EffectEvent::stopped(effect, reason))
            .collect()
    }

    fn advance_effect_frame_cursor(
        &mut self,
        cursor: Option<EffectFrameCursor>,
    ) -> Option<(EffectFrameCursor, Option<EffectEvent>)> {
        advance_effect_frame_cursor(&mut self.state.effects, cursor)
    }

    fn enqueue_commands<I>(&mut self, commands: I)
    where
        I: IntoIterator<Item = QueuedCommand>,
    {
        self.command_queue.extend(commands);
    }

    fn apply_status(&mut self, status: ObjectStatus) -> Vec<EffectEvent> {
        if self.state.status == status {
            return Vec::new();
        }

        self.state.status = status;
        match status {
            ObjectStatus::Deleted => self.mark_destroyed(),
            _ => {
                if status.is_active() {
                    self.destroyed = false;
                }
                Vec::new()
            }
        }
    }

    fn record_action_event(&mut self, previous: ActionState, kind: ActionTransitionKind) {
        self.pending_action_events.push_back(ActionTransitionEvent {
            previous_action: previous,
            kind,
        });
    }

    fn execute_command_queue(
        &mut self,
        physics: &PhysicsSettings,
        materials: &MaterialSet,
        mut landscape: Option<&mut Landscape>,
        action_library: &ActionLibrary,
        definitions: &HashMap<DefinitionId, Definition>,
        players: &HashMap<i32, Player>,
    ) -> CommandQueueOutcome {
        let mut outcome = CommandQueueOutcome::default();
        loop {
            let execute_now = match self.command_queue.front_mut() {
                Some(command) if command.delay == 0 => true,
                Some(command) => {
                    command.delay -= 1;
                    false
                }
                None => break,
            };

            if !execute_now {
                break;
            }

            let command = self.command_queue.pop_front().expect("front exists");
            let delta: ObjectDelta = command.update.into();
            if let Some(new_def) = delta.change_def.as_deref() {
                if let Some(definition) = definitions.get(new_def) {
                    let owner_color =
                        players
                            .get(&self.state.owner)
                            .and_then(Player::color)
                            .map(|color| {
                                u32::from(color.r) << 16
                                    | u32::from(color.g) << 8
                                    | u32::from(color.b)
                            });
                    Engine::apply_change_object_def_to_object(
                        self,
                        new_def,
                        definition,
                        materials.len(),
                        owner_color,
                    );
                    outcome.definition_changed = true;
                }
            }
            if delta.change_def.is_some() {
                outcome.change_def_reinsert = delta.change_def_reinsert;
            }
            let current_action_library = definitions
                .get(&self.definition_id)
                .map(Definition::action_library)
                .unwrap_or(action_library);
            let delta_outcome = self.apply_delta(&delta, current_action_library);
            let callbacks_dispatched = delta
                .action
                .as_ref()
                .map(|action| action.callbacks_dispatched)
                .unwrap_or(false);
            if let Some(change) = delta_outcome.action_change {
                if !callbacks_dispatched {
                    self.record_action_event(change.previous, ActionTransitionKind::Forced);
                }
            }
            if let Some((previous, new)) = delta_outcome.container_change {
                outcome.container_updates.push(ContainerUpdateRecord {
                    object_id: self.id,
                    previous,
                    new,
                });
            }
            let mut effect_events = self.apply_effect_commands(&command.effects);
            if let Some(status) = delta.status {
                let mut status_events = self.apply_status(status);
                if !status_events.is_empty() {
                    effect_events.append(&mut status_events);
                }
                if matches!(status, ObjectStatus::Deleted) {
                    outcome.destroy = true;
                }
            }
            self.clamp_velocity(physics);
            if command.destroy {
                if !matches!(self.state.status, ObjectStatus::Deleted) {
                    effect_events.extend(self.mark_destroyed());
                }
                outcome.destroy = true;
            }
            if !effect_events.is_empty() {
                outcome.effect_events.extend(effect_events);
            }
            if !command.spawns.is_empty() {
                outcome.spawns.extend(command.spawns);
            }
            if !command.events.is_empty() {
                outcome.command_events.extend(command.events);
            }
            if !command.particles.is_empty() {
                outcome.particles.extend(command.particles);
            }
            if let Some(landscape_ref) = &mut landscape {
                for op in command.landscape.iter() {
                    op.apply(landscape_ref);
                }
            }
            if !self.state.vertices.is_empty() {
                if let Some(landscape_ref) = &mut landscape {
                    let resolution = (**landscape_ref)
                        .resolve_collision(self.state.position, self.state.velocity);
                    if resolution.collided {
                        self.apply_collision_resolution(&resolution);
                        if let Some(material_id) = resolution.material {
                            if let Some(material) = materials.get_by_id(material_id) {
                                self.apply_material_interaction(material);
                            }
                        }
                    }
                }
            }

            if outcome.destroy {
                self.command_queue.clear();
                break;
            }
        }
        outcome
    }
}

/// Converts a script error from an engine-initiated callback into a logged
/// no-op. C++ runs lifecycle/game calls with `fPassErrors=false`: the error
/// shows in the log, the call yields nil, and the game continues
/// (C4AulExec.cpp:1318-1342). Non-script engine errors stay fatal.
/// `C4RankSystem::RankByExperience` with the default curve
/// Experience(rank) = rank^1.5 * RankBase(=1000) (C4RankSystem.cpp:226-237).
fn fair_crew_rank(experience: i32, rank_base: i32) -> i32 {
    let mut rank = 0;
    loop {
        let next = ((rank + 1) as f64).powf(1.5) * f64::from(rank_base);
        if next as i32 <= experience {
            rank += 1;
        } else {
            return rank;
        }
    }
}

/// `C4RankSystem::Experience` for the game-global rank system initialized
/// with `RankBase=1000` (C4RankSystem.cpp:226-229; C4Game.cpp:3518-3524).
/// `C4Object::DoExperience` deliberately uses this system for the promotion
/// threshold even when the crew definition supplies custom rank names.
fn rank_experience(rank: i32, rank_base: i32) -> i32 {
    if rank < 0 {
        return 0;
    }
    ((rank as f64).powf(1.5) * f64::from(rank_base)) as i32
}

pub(crate) fn crew_rank_experience(rank: i32) -> i32 {
    rank_experience(rank, 1_000)
}

/// `C4ObjectInfoCore::UpdateCustomRanks`' finite-table projection. `None`
/// means the definition has no custom rank system and therefore stores the
/// zero tag; an exhausted custom table stores `EXP_NoPromotion` (-1).
pub(crate) fn custom_next_rank_info(
    rank_names: Option<&RankNameTable>,
    rank_base: Option<i32>,
    rank: i32,
) -> (String, i32) {
    let Some(rank_names) = rank_names else {
        return (String::new(), 0);
    };
    let Some(next_rank) = rank
        .checked_add(1)
        .and_then(|rank| usize::try_from(rank).ok())
    else {
        return (String::new(), -1);
    };
    let Some(next_name) = rank_names.get(next_rank).filter(|name| !name.is_empty()) else {
        return (String::new(), -1);
    };
    let rank_base = rank_base.filter(|base| *base != 0).unwrap_or(1_000);
    let experience = ((next_rank as f64).powf(1.5) * f64::from(rank_base)) as i32;
    (next_name.into_owned(), experience)
}

/// Refresh the current custom rank name and stored next-rank fields. Callers
/// deliberately invoke this only at C++'s creation/save seams, never on load
/// or promotion.
fn update_custom_rank_fields(
    rank_name: &mut String,
    core: &mut CrewInfoCoreFields,
    rank: i32,
    rank_names: Option<&RankNameTable>,
    rank_base: Option<i32>,
) {
    if let Some(current_name) = rank_names
        .and_then(|names| usize::try_from(rank).ok().and_then(|rank| names.get(rank)))
        .filter(|name| !name.is_empty())
    {
        rank_name.clear();
        rank_name.push_str(&current_name);
    }
    (core.next_rank_name, core.next_rank_exp) = custom_next_rank_info(rank_names, rank_base, rank);
}

/// The state-changing half of `C4Object::DoExperience`
/// (C4Object.cpp:1518-1529). Returns whether this call promoted the info.
/// Promotion is intentionally limited to one rank per call, and hitting the
/// exact maximum suppresses promotion just like the native `< MaxExperience`
/// guard. The legacy executable performs an unchecked signed addition before
/// `BoundBy`; wrapping preserves that two's-complement runtime behavior.
pub(crate) fn adjust_crew_experience(info: &mut CrewObjectInfo, change: i32) -> bool {
    const MAX_EXPERIENCE: i32 = 100_000_000;

    info.experience = info
        .experience
        .wrapping_add(change)
        .clamp(0, MAX_EXPERIENCE);
    let next_rank = info.rank.saturating_add(1);
    if info.experience < MAX_EXPERIENCE && info.experience >= crew_rank_experience(next_rank) {
        info.rank = next_rank;
        true
    } else {
        false
    }
}

/// Native field updates in `C4PhysicalInfo::PromotionUpdate`
/// (C4InfoCore.cpp:207-222), before its optional definition-script
/// `GetFairCrewPhysical` overrides. Fair crew additionally trains Scale,
/// Hangle, Swim and Fight linearly toward `C4MaxPhysical` by rank 20.
pub(crate) fn promotion_updated_physical(
    mut physical: PhysicalInfo,
    rank: i32,
    training_definition: Option<PhysicalInfo>,
) -> PhysicalInfo {
    if rank >= 0 {
        physical.can_dig = 1;
        physical.can_chop = 1;
        physical.can_construct = 1;
        physical.can_scale = 1;
        physical.can_hangle = 1;
    }
    physical.energy = physical
        .energy
        .max((50 + 5 * rank.clamp(0, 10)) * (C4_MAX_PHYSICAL / 100));
    if let Some(definition) = training_definition {
        let train_rank = rank.clamp(0, 20);
        let train = |value: i32| value + (C4_MAX_PHYSICAL - value) * train_rank / 20;
        physical.scale = train(definition.scale);
        physical.hangle = train(definition.hangle);
        physical.swim = train(definition.swim);
        physical.fight = train(definition.fight);
    }
    physical
}

/// The persistent `C4ObjectInfo::Physical` installed on a joined/recruited
/// crew member. Fair crew never overwrites this training: GetPhysical selects
/// the definition's fair-crew projection live while the round option is on.
pub(crate) fn crew_info_physical(definition: PhysicalInfo, info_rank: i32) -> PhysicalInfo {
    promotion_updated_physical(definition, info_rank, None)
}

/// Numeric half of `C4Def::GetFairCrewPhysicals`; cache ownership and script
/// callbacks live at the definition-resolution seams below.
pub(crate) fn fair_crew_physical(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
) -> PhysicalInfo {
    let rank = fair_crew_rank(strength, rank_base);
    promotion_updated_physical(definition, rank, Some(definition))
}

const FAIR_CREW_PHYSICAL_NAMES: [&str; 21] = [
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
];

pub(crate) type FairCrewPhysicalCache = Rc<RefCell<HashMap<DefinitionId, PhysicalInfo>>>;

enum FairCrewProjectionStart {
    Cached(PhysicalInfo),
    New { physical: PhysicalInfo, rank: i32 },
}

fn begin_fair_crew_projection(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
    definition_id: &DefinitionId,
    cache: &FairCrewPhysicalCache,
) -> FairCrewProjectionStart {
    if let Some(physical) = cache.borrow().get(definition_id).copied() {
        return FairCrewProjectionStart::Cached(physical);
    }
    let rank = fair_crew_rank(strength, rank_base);
    let physical = promotion_updated_physical(definition, rank, Some(definition));
    // Native publishes pFairCrewPhysical before PromotionUpdate invokes any
    // hooks. Re-entrant GetPhysical calls therefore observe this in-progress
    // projection instead of recursively filling another cache entry.
    cache.borrow_mut().insert(definition_id.clone(), physical);
    FairCrewProjectionStart::New { physical, rank }
}

pub(crate) fn fair_crew_physical_cached(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
    definition_id: &DefinitionId,
    cache: &FairCrewPhysicalCache,
) -> PhysicalInfo {
    match begin_fair_crew_projection(definition, strength, rank_base, definition_id, cache) {
        FairCrewProjectionStart::Cached(physical)
        | FairCrewProjectionStart::New { physical, .. } => physical,
    }
}

/// C4PhysicalInfo::PromotionUpdate's definition callback. Every field is
/// passed by reference after the numeric promotion; the callback's result is
/// only the commit gate for the final reference value.
fn apply_fair_crew_physical_script(
    definition_physical: PhysicalInfo,
    mut physical: PhysicalInfo,
    rank: i32,
    definition_id: &DefinitionId,
    script: &ScriptEngine,
    cache: &FairCrewPhysicalCache,
) -> PhysicalInfo {
    if !script.has_function("GetFairCrewPhysical") {
        return physical;
    }
    compat::with_fair_crew_definition_context(definition_id.clone(), definition_physical, || {
        for name in FAIR_CREW_PHYSICAL_NAMES {
            let current = physical.value_by_name(name).unwrap_or_default();
            let args = [Value::from(name), Value::Int(rank), Value::Int(current)];
            match script.call_with_ref_args("GetFairCrewPhysical", &args) {
                Ok((commit, final_args)) if compat::value_raw_truthy(&commit) => {
                    let value = final_args
                        .get(2)
                        .and_then(Value::as_c4_int)
                        .unwrap_or_default();
                    physical.set_by_name(name, value);
                    cache.borrow_mut().insert(definition_id.clone(), physical);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        definition = %definition_id,
                        function = "GetFairCrewPhysical",
                        field = name,
                        error = %error,
                        "script error in fair-crew physical callback; retaining numeric value"
                    );
                    log_runtime_call_frames(definition_id, error.call_frames());
                }
            }
        }
        physical
    })
}

fn fair_crew_physical_with_script(
    definition: PhysicalInfo,
    strength: i32,
    rank_base: i32,
    definition_id: &DefinitionId,
    script: &ScriptEngine,
    cache: &FairCrewPhysicalCache,
) -> PhysicalInfo {
    match begin_fair_crew_projection(definition, strength, rank_base, definition_id, cache) {
        FairCrewProjectionStart::Cached(physical) => physical,
        FairCrewProjectionStart::New { physical, rank } => apply_fair_crew_physical_script(
            definition,
            physical,
            rank,
            definition_id,
            script,
            cache,
        ),
    }
}

fn log_runtime_call_frames(definition: &str, frames: &[clonk_script::RuntimeCallFrame]) {
    for frame in frames {
        if let Some(dump) = frame.direct_exec_display() {
            tracing::info!(" by: {dump}");
            continue;
        }
        let mut dump = format!("{}({})", frame.function(), frame.arguments());
        if let Some(object) = frame.object_context() {
            dump.push_str(&format!(" (obj {object})"));
        } else if let Some(definition) = frame.definition_context() {
            dump.push_str(&format!(" (def {definition})"));
        }
        let source_name = frame.source_name().unwrap_or(definition);
        if !source_name.is_empty() {
            dump.push_str(&format!(" ({source_name}:{})", frame.source_line()));
        }
        tracing::info!(" by: {dump}");
    }
}

fn tolerate_script_error<T>(result: Result<T, EngineError>) -> Result<Option<T>, EngineError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(EngineError::Script {
            definition,
            function,
            source,
            // Funnels consume the recovery payload before tolerating;
            // errors reaching here without a funnel carry none.
            recovery: _,
        }) => {
            tracing::warn!(
                %definition,
                function,
                error = %source,
                "script error in engine callback; continuing like the C++ fail-safe exec"
            );
            log_runtime_call_frames(&definition, source.call_frames());
            Ok(None)
        }
        Err(other) => Err(other),
    }
}

fn script_execution_error(
    definition: String,
    function: String,
    source: ScriptError,
    recovery: Option<Box<ScriptCallRecovery>>,
) -> EngineError {
    EngineError::Script {
        definition,
        function,
        source,
        recovery,
    }
}

#[cfg(test)]
#[path = "lib_tests/failsafe_call_stack_diagnostic_regression.rs"]
mod failsafe_call_stack_diagnostic_regression;

/// Everything a failed outer script call mutated BEFORE the error: the
/// partial host outcome plus the advanced RNG and audio state. C++ keeps
/// all of it (mutations land on the live objects as they happen); the
/// engine funnels apply it before handling the error.
#[derive(Debug)]
pub struct ScriptCallRecovery {
    pub(crate) outcome: compat::EffectContextOutcome,
    pub(crate) audio: AudioRegistry,
    pub(crate) rng: LcgRng,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("definition `{0}` is already registered")]
    DefinitionAlreadyExists(String),
    #[error("unknown definition `{0}`")]
    UnknownDefinition(String),
    #[error("player {0} already exists")]
    PlayerAlreadyExists(i32),
    #[error("unknown player {0}")]
    UnknownPlayer(i32),
    #[error("unknown object `{0}`")]
    UnknownObject(ObjectId),
    #[error("container error for object {object}: {detail}")]
    Container { object: ObjectId, detail: String },
    #[error("crew selection error for owner {owner}: {detail}")]
    CrewSelection { owner: i32, detail: String },
    #[error("crew role error for owner {owner}: {detail}")]
    CrewRole { owner: i32, detail: String },
    #[error("script error in {function} of `{definition}`")]
    Script {
        definition: String,
        function: String,
        #[source]
        source: ScriptError,
        /// The failed call's PRE-ERROR outcome. C4AulExec errors abort the
        /// call but roll nothing back (C4AulExec.cpp:1318-1342) — C++
        /// already mutated the live objects — so the engine funnel applies
        /// this before surfacing the error.
        recovery: Option<Box<ScriptCallRecovery>>,
    },
    #[error("invalid script output in {function} of `{definition}`: {detail}")]
    InvalidScriptOutput {
        definition: String,
        function: String,
        detail: String,
    },
    /// An app-owned classic UI path reached an unported presentation or
    /// action boundary. This is deliberately distinct from script output:
    /// the control fail-safe must never downgrade it into a status line.
    #[error("classic menu parity boundary: {detail}")]
    ClassicMenuParityBoundary { detail: String },
    #[error("object id `{0}` is already in use")]
    DuplicateObjectId(ObjectId),
    #[error("invalid scenario-section landscape: {0}")]
    InvalidScenarioSectionLandscape(String),
    #[error(transparent)]
    RuntimeJoinPlayerRestore(#[from] RuntimeJoinPlayerRestoreError),
    #[error("failed to persist scenario section `{section}`: {detail}")]
    ScenarioSectionSave { section: String, detail: String },
    #[error("failed to load objects for scenario section `{section}`: {detail}")]
    ScenarioSectionObjects { section: String, detail: String },
}

#[derive(Debug, Error)]
pub enum EngineStateIoError {
    #[error("I/O error while handling engine state")]
    Io(#[from] io::Error),
    #[error("failed to (de)serialize engine state as JSON")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnConfig {
    #[serde(default)]
    pub id: Option<ObjectId>,
    pub definition_id: DefinitionId,
    /// Saved or explicitly assigned C4Object::CustomName.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub custom_name: Option<String>,
    pub position: Vector2,
    pub velocity: Vector2,
    /// Saved C4Object::motion_x/motion_y frame caches.
    #[serde(default)]
    pub motion_x: i32,
    #[serde(default)]
    pub motion_y: i32,
    /// Exact stale/raw C4Object compiler caches. Loaded Objects.txt records
    /// populate these before pointer denumeration; fresh objects leave them
    /// at the native zero/empty defaults.
    #[serde(default, skip_serializing_if = "ObjectCompilerCache::is_default")]
    #[doc(hidden)]
    pub compiler_cache: ObjectCompilerCache,
    /// Exact sub-pixel velocity: savegame `XDir`/`YDir` are serialized
    /// C4Fixed values (C4Object.cpp:2765-2766), not whole pixels. Takes
    /// precedence over `velocity` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    #[serde(default)]
    pub rotation: i32,
    /// None = the C4Object::Init rule (alive -> GetPhysical()->Energy,
    /// else 0; C4Object.cpp:191-192), unless native compiled-object defaults
    /// are selected. Some = explicit raw value (loader).
    pub energy: Option<i32>,
    /// Saved C4Object::Damage. None uses the fresh-object zero default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<i32>,
    /// Saved C4Object::NeedEnergy. New objects default to false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_energy: Option<bool>,
    /// C4Object::MagicEnergy, compiled verbatim from saves with default 0
    /// (C4Object.cpp:2768). MagicPhysicalFactor raw scale.
    #[serde(default)]
    pub magic_energy: Option<i32>,
    #[serde(default = "default_construction")]
    pub construction: i32,
    pub action: Option<ActionState>,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    /// Saved temporary physical block and its ordered stack history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_physical: Option<PhysicalInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_changes: Vec<(String, i32)>,
    /// Saved raw breath counter; fresh and generic programmatic loaded objects
    /// seed this from Physical.Breath, while native compiled records default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub breath: Option<i32>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    /// Exact saved C4Shape slot storage. Loaded Objects.txt may carry
    /// non-default values beyond `Vertices`; fresh spawns leave this None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub shape_vertices: Option<ShapeVertexBuffer>,
    /// Saved C4Object::fOwnVertices flag. Its untransformed backup lives in
    /// raw shape slots 15.. and is reconstructed during loaded spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub owns_shape_vertices: Option<bool>,
    /// Exact saved live C4Shape rectangle (Width/Height/Offset). Loaded
    /// objects install it verbatim; future UpdateShape rebuilds from DefCore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_rect: Option<DefinitionRect>,
    /// Saved live C4Shape::ContactDensity. None means a fresh object copies
    /// its definition; a loaded object defaults to C4M_Solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_density: Option<i32>,
    /// Saved live C4Shape::FireTop. Fresh objects derive it from DefCore and
    /// construction; loaded objects compile the embedded shape verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_fire_top: Option<i32>,
    /// Saved C4Shape attachment coordinates. AttachMat itself is not compiled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_attach: Option<ShapeAttachRecord>,
    /// Explicit per-object `C4Object::Component` list. Loaded Objects.txt
    /// entries compile this verbatim (C4Object.cpp:2811); fresh objects use
    /// their definition components scaled to initial Con when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<HashMap<DefinitionId, i32>>,
    /// Explicit C4IDList order for loaded/runtime component lists. None uses
    /// definition order for fresh objects and a deterministic key fallback
    /// for legacy Rust states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_order: Option<Vec<DefinitionId>>,
    pub owner: i32,
    /// Explicit C4Object::Controller: Objects.txt `Controller=` on loads
    /// (compile default NO_OWNER, C4Object.cpp:2739) or the creating
    /// controller from script CreateObject/CreateConstruction
    /// (C4Script.cpp:1905-1906, 1932-1933). None/NO_OWNER = the Init rule
    /// (owner) for fresh spawns (C4Object.cpp:162).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<i32>,
    #[serde(default)]
    pub crew_member: Option<bool>,
    /// Saved C4Object::CrewDisabled bit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crew_disabled: Option<bool>,
    /// Saved C4Object::PlrViewRange (`PlrViewRange=` in Objects.txt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plr_view_range: Option<i32>,
    /// Saved C4Object::Select bit (`Selected=` in Objects.txt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default)]
    pub status: Option<ObjectStatus>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    #[serde(default)]
    pub layer: Option<ObjectId>,
    /// Saved C4Object::Visibility. None/zero is VIS_All.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<i32>,
    /// Saved C4Object::BlitMode. None uses the definition default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blit_mode: Option<u32>,
    /// Saved raw C4Object::PictureRect (`Picture=` in Objects.txt). A zero
    /// rect is meaningful: Picture2Facet then falls back to the def picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_rect: Option<DefinitionRect>,
    /// Saved C4Object::Color (`ColorDw=` in Objects.txt). Fresh
    /// ColorByOwner objects derive it from their owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    /// Saved C4Object::ColorMod (`ColorMod=` in Objects.txt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_modulation: Option<u32>,
    /// Saved object graphics selection, draw transform, and overlay chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    #[serde(default)]
    pub alive: Option<bool>,
    /// Saved C4Object::Category. Native compiled records default to zero;
    /// fresh and generic programmatic loaded objects use the definition.
    #[serde(default)]
    pub category: Option<i32>,
    /// `InLiquid` from Objects.txt (C4Object.cpp:2775, default false).
    #[serde(default)]
    pub in_liquid: Option<bool>,
    /// Saved C4Object::EntranceStatus. Loaded objects skip Initialize, so
    /// Objects.txt must restore this independently (C4Object.cpp:2803).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrance_status: Option<bool>,
    /// Exact sub-pixel position: savegame `FixX`/`FixY` are serialized
    /// C4Fixed values (C4Object.cpp:2762-2763). C++ never reconciles them
    /// with the integer X/Y after load — the override leaves `position`
    /// untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_position: Option<FixedVec2>,
    /// Exact fixed rotation accumulator (`FixR`, C4Object.cpp:2764);
    /// independent of the whole-degree `rotation` like FixX/FixY.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_rotation: Option<C4Fixed>,
    /// Angular velocity (`RDir`, C4Object.cpp:2767).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    /// Explicit Mobile flag (Objects.txt `Mobile=`, C4Object.cpp:2772).
    /// None = the C4Object::Init rule for fresh spawns.
    #[serde(default)]
    pub mobile: Option<bool>,
    /// Mid-cycle Def TimerCall counter (Objects.txt `Timer=`, default 0,
    /// C4Object.cpp:2738).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<i32>,
    /// Saved script mass override and the compiled Mass cache. The cache is
    /// authoritative after Objects.txt load until a native UpdateMass path;
    /// a marked native compiled record installs zero when this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_mass: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub compiled_mass: Option<i32>,
    /// Saved burning state. fire_caused_by is recovered from the Fire effect
    /// because native Objects.txt does not carry a separate scalar field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_fire: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_phase: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_caused_by: Option<i32>,
    /// Saved private C4Object bookkeeping fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attach_movement_frame: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_energy_loss_cause: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_collect_delay: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<i32>,
    /// Parsed OCF is retained for format completeness, but loaded objects run
    /// SetOCF after compilation and therefore replace it from restored state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub compiled_ocf: Option<u32>,
    /// Exact top-first C4Command linked-list state from [Commands].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_stack: Option<CommandStackSnapshot>,
    /// Per-object script locals (Objects.txt `LocalNamed=`,
    /// C4Object.cpp:2788) — loaded verbatim into ObjectState.local_vars.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub local_vars: HashMap<String, Value>,
    /// The object is LOADED (Objects.txt / savegame), not created:
    /// C4GameObjects::Load (C4GameObjects.cpp:535-618) only compiles and
    /// denumerates — Construction/Initialize fire for new objects only
    /// (C4Object::Init). This selects the loaded-object lifecycle; omitted
    /// scalar values keep the programmatic fixture fallbacks unless
    /// [`Self::native_compiled_object_defaults`] is also set.
    #[serde(default)]
    pub loaded: bool,
    /// Apply C4Object::CompileFunc's literal defaults for omitted Objects.txt
    /// scalar fields: Category, Energy, Breath, and Mass all start at zero.
    /// Scenario loading sets this together with [`Self::loaded`]; keeping the
    /// marker separate lets programmatic callback-suppressed fixtures retain
    /// definition-derived values when they do not model a compiled record.
    #[serde(default, skip_serializing_if = "is_false")]
    pub native_compiled_object_defaults: bool,
    /// Saved per-object SolidMask rect (Objects.txt SolidMask=; default
    /// keeps the definition mask, C4Object.cpp:2770).
    #[serde(default)]
    pub solid_mask: Option<DefinitionTargetRect>,
    /// Runtime-only C4SolidMask construction order reserved by synchronous
    /// script creation before this deferred spawn materializes.
    #[serde(skip)]
    #[doc(hidden)]
    pub solid_mask_instance_sequence: Option<u64>,
    /// Construction/Initialize already ran synchronously inside the
    /// creating host call (C4Game::NewObject semantics,
    /// C4Game.cpp:1117-1127) - materialization must not repeat them.
    #[serde(default)]
    pub initialized: bool,
    /// The DoCon bottom-growth y-adjust already happened at the creation
    /// seam (CreateObject computed the preview from the FINAL center) —
    /// materialization must not re-apply it.
    #[serde(default)]
    pub position_adjusted: bool,
}

impl SpawnConfig {
    pub fn new(definition_id: impl Into<String>) -> Self {
        Self {
            id: None,
            definition_id: definition_id.into(),
            custom_name: None,
            position: Vector2::ZERO,
            velocity: Vector2::ZERO,
            motion_x: 0,
            motion_y: 0,
            compiler_cache: ObjectCompilerCache::default(),
            fixed_velocity: None,
            rotation: 0,
            energy: None,
            damage: None,
            need_energy: None,
            magic_energy: None,
            construction: FULL_CON,
            action: None,
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            effects: Vec::new(),
            temporary_physical: None,
            physical_changes: Vec::new(),
            breath: None,
            vertices: Vec::new(),
            shape_vertices: None,
            owns_shape_vertices: None,
            shape_rect: None,
            contact_density: None,
            shape_fire_top: None,
            shape_attach: None,
            components: None,
            component_order: None,
            owner: OWNER_NONE,
            controller: None,
            crew_member: None,
            crew_disabled: None,
            plr_view_range: None,
            selected: None,
            status: None,
            container: None,
            layer: None,
            visibility: None,
            blit_mode: None,
            picture_rect: None,
            color: None,
            color_modulation: None,
            base_graphics: None,
            graphics_overlays: Vec::new(),
            draw_transform: None,
            alive: None,
            category: None,
            in_liquid: None,
            entrance_status: None,
            fixed_position: None,
            fixed_rotation: None,
            rotation_velocity: None,
            mobile: None,
            timer: None,
            own_mass: None,
            compiled_mass: None,
            on_fire: None,
            fire_phase: None,
            fire_caused_by: None,
            last_attach_movement_frame: None,
            last_energy_loss_cause: None,
            no_collect_delay: None,
            base: None,
            compiled_ocf: None,
            command_stack: None,
            local_vars: HashMap::new(),
            loaded: false,
            native_compiled_object_defaults: false,
            solid_mask: None,
            solid_mask_instance_sequence: None,
            initialized: false,
            position_adjusted: false,
        }
    }

    pub fn with_in_liquid(mut self, in_liquid: bool) -> Self {
        self.in_liquid = Some(in_liquid);
        self
    }

    pub fn with_entrance_status(mut self, entrance_status: bool) -> Self {
        self.entrance_status = Some(entrance_status);
        self
    }

    pub fn with_custom_name(mut self, name: impl Into<String>) -> Self {
        self.custom_name = Some(name.into());
        self
    }

    pub fn with_fixed_position(mut self, position: FixedVec2) -> Self {
        self.fixed_position = Some(position);
        self
    }

    pub fn with_fixed_rotation(mut self, rotation: C4Fixed) -> Self {
        self.fixed_rotation = Some(rotation);
        self
    }

    pub fn with_rotation_velocity(mut self, velocity: C4Fixed) -> Self {
        self.rotation_velocity = Some(velocity);
        self
    }

    pub fn with_mobile(mut self, mobile: bool) -> Self {
        self.mobile = Some(mobile);
        self
    }

    pub fn with_timer(mut self, timer: i32) -> Self {
        self.timer = Some(timer);
        self
    }

    pub fn with_local_vars(mut self, local_vars: HashMap<String, Value>) -> Self {
        self.local_vars = local_vars;
        self
    }

    pub fn with_solid_mask(mut self, rect: DefinitionTargetRect) -> Self {
        self.solid_mask = Some(rect);
        self
    }

    pub fn with_loaded(mut self, loaded: bool) -> Self {
        self.loaded = loaded;
        self
    }

    /// Mark this configuration as a native compiled Objects.txt record.
    /// Call together with [`Self::with_loaded`] to reproduce the complete
    /// C4GameObjects::Load lifecycle.
    pub fn with_native_compiled_object_defaults(mut self) -> Self {
        self.native_compiled_object_defaults = true;
        self
    }

    pub fn with_fixed_velocity(mut self, velocity: FixedVec2) -> Self {
        self.fixed_velocity = Some(velocity);
        self
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = position;
        self
    }

    pub fn with_rotation(mut self, rotation: i32) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = velocity;
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = Some(energy);
        self
    }

    pub fn with_need_energy(mut self, need_energy: bool) -> Self {
        self.need_energy = Some(need_energy);
        self
    }

    pub fn with_magic_energy(mut self, magic_energy: i32) -> Self {
        self.magic_energy = Some(magic_energy);
        self
    }

    pub fn with_construction(mut self, construction: i32) -> Self {
        // Final clamping needs the target definition and the loaded-object
        // flag, both resolved by spawn_single. Preserve the raw nonnegative
        // C4Object::Con value here (Oversize and Objects.txt may exceed 100%).
        self.construction = construction.max(0);
        self
    }

    pub fn with_direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn with_command_direction(mut self, command_direction: CommandDirection) -> Self {
        self.command_direction = command_direction;
        self
    }

    pub fn with_action(mut self, action: ActionState) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_effects(mut self, effects: Vec<EffectState>) -> Self {
        self.effects = effects;
        self
    }

    pub fn add_effect(mut self, effect: EffectState) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn with_vertices(mut self, vertices: Vec<ObjectVertex>) -> Self {
        self.vertices = vertices;
        self
    }

    pub(crate) fn with_shape_vertex_slots(
        mut self,
        active_count: usize,
        slots: Vec<ObjectVertex>,
    ) -> Self {
        self.shape_vertices = Some(ShapeVertexBuffer::from_slots(active_count, &slots));
        self
    }

    pub(crate) fn with_owns_shape_vertices(mut self, owns_vertices: bool) -> Self {
        self.owns_shape_vertices = Some(owns_vertices);
        self
    }

    pub fn with_shape_rect(mut self, rect: DefinitionRect) -> Self {
        self.shape_rect = Some(rect);
        self
    }

    pub fn with_contact_density(mut self, contact_density: i32) -> Self {
        self.contact_density = Some(contact_density);
        self
    }

    pub fn with_shape_fire_top(mut self, fire_top: i32) -> Self {
        self.shape_fire_top = Some(fire_top);
        self
    }

    pub fn with_components(mut self, components: HashMap<DefinitionId, i32>) -> Self {
        self.components = Some(components);
        // A map has no C4IDList ordering. Spawn resolves known entries in
        // definition order and appends only unknown extras deterministically.
        self.component_order = None;
        self
    }

    pub fn with_ordered_components(mut self, components: Vec<(DefinitionId, i32)>) -> Self {
        let mut counts = HashMap::new();
        let mut order = Vec::new();
        for (id, count) in components {
            order.push(id.clone());
            counts.entry(id).or_insert(count);
        }
        self.components = Some(counts);
        self.component_order = Some(order);
        self
    }

    pub fn add_vertex(mut self, vertex: ObjectVertex) -> Self {
        self.vertices.push(vertex);
        self
    }

    pub fn with_owner(mut self, owner: i32) -> Self {
        self.owner = owner;
        self
    }

    pub fn with_controller(mut self, controller: i32) -> Self {
        self.controller = Some(controller);
        self
    }

    pub fn with_category(mut self, category: i32) -> Self {
        self.category = Some(category);
        self
    }

    pub fn with_id(mut self, id: ObjectId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_status(mut self, status: ObjectStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_crew_member(mut self, crew_member: bool) -> Self {
        self.crew_member = Some(crew_member);
        self
    }

    pub fn with_plr_view_range(mut self, plr_view_range: i32) -> Self {
        self.plr_view_range = Some(plr_view_range);
        self
    }

    pub fn with_selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub fn with_alive(mut self, alive: bool) -> Self {
        self.alive = Some(alive);
        self
    }

    pub fn with_container(mut self, container: ObjectId) -> Self {
        self.container = Some(container);
        self
    }

    pub fn with_layer(mut self, layer: ObjectId) -> Self {
        self.layer = Some(layer);
        self
    }

    pub fn with_visibility(mut self, visibility: i32) -> Self {
        self.visibility = Some(visibility);
        self
    }

    pub fn with_blit_mode(mut self, blit_mode: u32) -> Self {
        self.blit_mode = Some(blit_mode);
        self
    }

    pub fn with_picture_rect(mut self, picture_rect: DefinitionRect) -> Self {
        self.picture_rect = Some(picture_rect);
        self
    }

    pub fn with_color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    pub fn with_color_modulation(mut self, color_modulation: u32) -> Self {
        self.color_modulation = Some(color_modulation);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: ObjectId,
    pub definition_id: DefinitionId,
    /// C4Object::CustomName; None falls back to crew-info/definition name.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "clonk_script::c4_optional_string_serde"
    )]
    pub custom_name: Option<String>,
    pub position: Vector2,
    pub velocity: Vector2,
    #[serde(default)]
    pub rotation: i32,
    pub energy: i32,
    /// C4Object::NeedEnergy, persisted by C++ as `NeedEnergy=`
    /// (C4Object.cpp:2805).
    #[serde(default, skip_serializing_if = "is_false")]
    pub need_energy: bool,
    #[serde(default)]
    pub damage: i32,
    #[serde(default)]
    pub magic_energy: i32,
    #[serde(default)]
    pub magic_capacity: i32,
    #[serde(default = "default_construction")]
    pub construction: i32,
    #[serde(default)]
    pub action: ActionState,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub command_direction: CommandDirection,
    #[serde(default)]
    pub action_procedure: Option<String>,
    #[serde(default)]
    pub effects: Vec<EffectState>,
    #[serde(default)]
    pub vertices: Vec<ObjectVertex>,
    /// Exceptional live C4Object::Shape rectangle after SetShape or a loaded
    /// embedded shape. None means the exact rectangle is reconstructible from
    /// DefCore, Con and rotation. The sparse sidecar preserves C++ runtime
    /// shape state in renderer snapshots and saved recordings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_shape: Option<DefinitionRect>,
    /// Exceptional live C4Shape::FireTop loaded with the embedded shape.
    /// None means DefCore FireTop scaled by Con (C4Shape.cpp:103-127,495-510).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_fire_top: Option<i32>,
    /// Saved live C4Shape::ContactDensity (C4Shape.cpp:495-510).
    #[serde(
        default = "default_contact_density",
        skip_serializing_if = "is_default_contact_density"
    )]
    pub contact_density: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_vertices: Option<Vec<ObjectVertex>>,
    /// `C4Shape::VtxContactCNAT` values aligned with `vertices`. These are
    /// presentation diagnostics, not collision input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vertex_contacts: Vec<u32>,
    /// Per-object `C4Object::SolidMask` override. `None` selects DefCore;
    /// a zero-area rectangle explicitly disables the definition mask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_mask_override: Option<DefinitionTargetRect>,
    #[serde(default)]
    pub container: Option<ObjectId>,
    /// C4Object::pLayer (Objects.txt `Layer=` / SetObjectLayer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<ObjectId>,
    /// C4Object::Visibility; zero is VIS_All.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub visibility: i32,
    /// C4Object::BlitMode, including the C4GFXBLIT_CUSTOM marker.
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub blit_mode: u32,
    /// C4Object::Color (owner color for ColorByOwner definitions, or an
    /// explicit SetColorDw/Objects.txt value).
    #[serde(default)]
    pub color: u32,
    /// C4Object::ColorMod, persisted as `ColorMod=` in Objects.txt.
    #[serde(default)]
    pub color_modulation: u32,
    /// Raw per-object C4Object::PictureRect. Width zero selects the
    /// definition-default facet at draw time.
    #[serde(default)]
    pub picture_rect: DefinitionRect,
    #[serde(default)]
    pub contents: Vec<ObjectId>,
    #[serde(default)]
    pub components: HashMap<DefinitionId, i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub component_order: Vec<DefinitionId>,
    #[serde(default)]
    pub status: ObjectStatus,
    #[serde(default = "default_owner")]
    pub owner: i32,
    /// C4Object::Controller (persisted like the savegame `Controller`
    /// field, default NO_OWNER, C4Object.cpp:2739).
    #[serde(default = "default_owner")]
    pub controller: i32,
    #[serde(default = "default_category")]
    pub category: i32,
    #[serde(default)]
    pub crew_member: bool,
    /// Saved C4Object::PlrViewRange.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub plr_view_range: i32,
    /// C4Object::Select, persisted independently of the player's cursor
    /// (C4Object.cpp:2800).
    #[serde(default, skip_serializing_if = "is_false")]
    pub selected: bool,
    #[serde(default = "default_alive")]
    pub alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_graphics: Option<ObjectBaseGraphics>,
    #[serde(default)]
    pub graphics_overlays: Vec<ObjectGraphicsOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_transform: Option<DrawTransform>,
    #[serde(default)]
    pub command_queue: Vec<QueuedCommand>,
    #[serde(default)]
    pub command_stack: CommandStackSnapshot,
    #[serde(default)]
    pub local_vars: HashMap<String, Value>,
    /// Cached liquid flag (C4Object::InLiquid, persisted like the
    /// savegame `InLiquid` field, C4Object.cpp:2775).
    #[serde(default)]
    pub in_liquid: bool,
    /// C4Object::Mobile (persisted like the savegame `Mobile` field,
    /// C4Object.cpp:2772).
    #[serde(default)]
    pub mobile: bool,
    /// The object's current OCF bits (C4Object::OCF) — broadcast world
    /// feeds need them for OCF-filtered searches.
    #[serde(default)]
    pub ocf: u32,
    /// The Def TimerCall counter (persisted like the savegame `Timer`
    /// field, C4Object.cpp:2738).
    #[serde(default)]
    pub timer: i32,
    /// C4Object::OwnMass (SetMass leftovers; persisted like the savegame).
    #[serde(default)]
    pub own_mass: i32,
    /// Burning state (C4Object::OnFire) with its animation phase and the
    /// causing player (the fire effect's CausedBy var).
    #[serde(default)]
    pub on_fire: bool,
    #[serde(default)]
    pub fire_phase: i32,
    #[serde(default = "default_owner")]
    pub fire_caused_by: i32,
    /// `C4ObjectInfo::Physical` surrogate (crew training); C++ persists info
    /// physicals with the crew file — carried on the object until the
    /// C4ObjectInfo model lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info_physical: Option<PhysicalInfo>,
    /// `PhysicalTemporary`/`TemporaryPhysical` (C4Object.cpp:2777,2798-2801).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporary_physical: Option<PhysicalInfo>,
    /// `C4TempPhysicalInfo::Changes` (C4InfoCore.cpp:306) as
    /// (physical name, previous value) pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_changes: Vec<(String, i32)>,
    /// `C4Object::Breath` (C4Object.cpp:2738 compile list).
    #[serde(default)]
    pub breath: i32,
    /// `LastEngLossPlr` (C4Object.cpp:2740) — kill attribution.
    #[serde(default = "default_owner")]
    pub last_energy_loss_cause: i32,
    /// `C4Object::Base` (C4Object.h:135): home-base player or NO_OWNER
    /// (ExecBase flag assignment, C4Object.cpp:1000-1031).
    #[serde(default = "default_owner")]
    pub base: i32,
    /// Raw 16.16 fixed-point position, recorded only when it carries sub-pixel
    /// detail beyond the whole-pixel `position` (i.e. `position != fixtoi(fix)`).
    /// `None` ⇒ reconstruct losslessly via `itofix(position)`. Mirrors C++
    /// persisting both `x` and `fix_x` (`C4Object.cpp:2742`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_position: Option<FixedVec2>,
    /// Raw 16.16 fixed-point velocity, recorded only when it carries sub-pixel
    /// detail beyond the whole-pixel `velocity`. `None` ⇒ `itofix(velocity)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_velocity: Option<FixedVec2>,
    /// Raw 16.16 fixed-point angular velocity (`rdir`), present when nonzero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_velocity: Option<C4Fixed>,
    /// Raw rotation accumulator (`fix_r`), omitted only when it equals
    /// `itofix(rotation)` and can be reconstructed exactly. C++ persists raw
    /// signed `r`, `fix_r`, and `rdir` independently (C4Object.cpp:2769,
    /// 2789,2792).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_rotation: Option<C4Fixed>,
}

/// Persisted runtime values owned by `C4AulScriptEngine`: numbered
/// `Global(index)` slots and declared `GlobalNamed` statics
/// (C4Aul.cpp:506-520). Ordered maps keep JSON byte-stable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScriptGlobalState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub numbered: BTreeMap<i32, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub named: BTreeMap<String, Value>,
}

/// Presentation metadata for definitions whose `Line` DefCore field turns
/// their live shape vertices into a polyline (`C4Object::DrawLine`). Kept on
/// the snapshot because the renderer deliberately has no mutable `Engine`
/// access.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionLineMetadata {
    #[serde(default)]
    pub line: i32,
    #[serde(default)]
    pub line_intersect: i32,
}

impl ScriptGlobalState {
    pub fn is_empty(&self) -> bool {
        self.numbered.is_empty() && self.named.is_empty()
    }
}

pub(crate) fn denumerate_script_value(value: &Value, object_numbers: &HashSet<u64>) -> Value {
    match value {
        Value::Object(id) => object_numbers
            .get(id)
            .map_or(Value::Nil, |_| Value::Object(*id)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| denumerate_script_value(value, object_numbers))
                .collect(),
        ),
        Value::Proplist(entries) => Value::Proplist(denumerate_script_map(entries, object_numbers)),
        value => value.clone(),
    }
}

fn denumerate_script_map(entries: &ValueMap, object_numbers: &HashSet<u64>) -> ValueMap {
    // Existing emptyValues are not part of C4ValueHash's visible iterator and
    // are therefore not denumerated. Preserve their current LIFO reuse order;
    // new removals are pushed in front of them by removeValue.
    let hidden_values = entries.hidden_values().cloned().collect::<Vec<_>>();
    let mut resolved = ValueMap::with_capacity(entries.len());
    let mut removed_values = Vec::new();
    for (key, value) in entries {
        let removed = is_missing_script_object(key, object_numbers)
            || is_missing_script_object(value, object_numbers);
        let key = denumerate_script_value(key, object_numbers);
        let value = denumerate_script_value(value, object_numbers);
        if removed {
            // A cleared key retains its mapped value; a cleared mapped value
            // is already nil. Both slots enter C4ValueHash::emptyValues.
            removed_values.push(value);
        } else {
            resolved.insert_key(key, value);
        }
    }
    for value in hidden_values.into_iter().rev() {
        resolved.recycle_value_slot(value);
    }
    for value in removed_values {
        resolved.recycle_value_slot(value);
    }
    resolved
}

fn is_missing_script_object(value: &Value, object_numbers: &HashSet<u64>) -> bool {
    matches!(value, Value::Object(id) if !object_numbers.contains(id))
}

#[cfg(test)]
#[test]
fn map_denumeration_erases_missing_direct_object_entries() {
    let mut map = ValueMap::new();
    map.insert_key(Value::Object(7), Value::Int(1));
    map.insert("direct_value".into(), Value::Object(7));
    map.insert_key(
        Value::Array(vec![Value::Object(7)]),
        Value::String("nested_key".into()),
    );
    map.insert("nested_value".into(), Value::Array(vec![Value::Object(7)]));
    map.insert_key(Value::Object(8), Value::String("live".into()));

    let restored = denumerate_script_value(&Value::Proplist(map), &HashSet::from([8]));
    let Value::Proplist(map) = restored else {
        panic!("map remains a map");
    };
    assert_eq!(map.len(), 3);
    assert_eq!(
        map.get_key(&Value::Array(vec![Value::Nil])),
        Some(&Value::String("nested_key".into()))
    );
    assert_eq!(
        map.get("nested_value"),
        Some(&Value::Array(vec![Value::Nil]))
    );
    assert_eq!(
        map.get_key(&Value::Object(8)),
        Some(&Value::String("live".into()))
    );
    assert_eq!(
        map.hidden_values().cloned().collect::<Vec<_>>(),
        vec![Value::Nil, Value::Int(1)],
        "removed mapped slots remain alive in native LIFO reuse order"
    );
}

#[cfg(test)]
#[test]
fn effect_map_denumeration_preserves_existing_and_removed_hidden_slots() {
    let mut map = ValueMap::new();
    map.insert_key(Value::Object(7), Value::String("removed value".into()));
    map.insert_key(Value::Int(1), Value::Object(7));
    map.insert_key(Value::Object(8), Value::String("visible".into()));
    map.recycle_value_slot(Value::String("older hidden".into()));
    let mut effect_value = EffectVarValue::Proplist(map);

    denumerate_effect_value(&mut effect_value, &HashSet::from([8]));

    let EffectVarValue::Proplist(map) = effect_value else {
        panic!("effect value remains a map");
    };
    assert_eq!(map.len(), 1);
    assert_eq!(
        map.get_key(&Value::Object(8)),
        Some(&Value::String("visible".into()))
    );
    assert_eq!(
        map.hidden_values().cloned().collect::<Vec<_>>(),
        vec![
            Value::Nil,
            Value::String("removed value".into()),
            Value::String("older hidden".into()),
        ]
    );
}

#[cfg(test)]
#[test]
fn object_snapshot_carries_exceptional_live_shape_and_fire_top_for_rendering() {
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("FIRE", "Fire fixture", "").expect("definition compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(-6, -4, 12, 8)));
    definition.set_shape_vertices(vec![ObjectVertex::new(2, -1), ObjectVertex::new(5, 3)]);
    definition.set_rotateable(1);
    definition.set_fire_top(7);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let id = engine
        .spawn_object(SpawnConfig::new("FIRE").with_rotation(90))
        .expect("object spawns");

    assert_eq!(engine.definition_fire_top("FIRE"), 7);
    assert_eq!(
        engine.object_current_shape_rect(id),
        Some(DefinitionRect::new(-9, -9, 18, 18))
    );
    assert_eq!(
        engine
            .object_snapshot(id)
            .and_then(|object| object.current_shape),
        None,
        "definition-derived geometry stays a sparse snapshot sidecar"
    );

    let mut update = ObjectUpdate::new();
    update.shape_override = Some(Some(DefinitionRect::new(2, 3, 4, 5)));
    engine
        .apply_object_update(id, update)
        .expect("SetShape-style override applies");
    assert_eq!(
        engine
            .object_snapshot(id)
            .and_then(|object| object.current_shape),
        Some(DefinitionRect::new(2, 3, 4, 5))
    );

    let loaded = engine
        .spawn_object(
            SpawnConfig::new("FIRE")
                .with_loaded(true)
                .with_shape_fire_top(11),
        )
        .expect("loaded object spawns");
    assert_eq!(
        engine
            .object_snapshot(loaded)
            .and_then(|object| object.current_fire_top),
        Some(11)
    );
    let snapshot = engine.snapshot();
    let encoded = serde_json::to_string(&snapshot).expect("snapshot serializes");
    assert!(encoded.contains("current_shape"));
    assert!(encoded.contains("current_fire_top"));
    let decoded: SimulationSnapshot =
        serde_json::from_str(&encoded).expect("snapshot deserializes");
    assert_eq!(decoded, snapshot, "live shape sidecars round-trip");

    let state = engine.capture_state();
    let encoded = state.to_json_string().expect("engine state serializes");
    let decoded = EngineState::from_json_str(&encoded).expect("engine state deserializes");
    engine
        .restore_state(&decoded)
        .expect("engine state restores");
    let restored = engine
        .object_snapshot(loaded)
        .expect("loaded object restored");
    assert_eq!(restored.current_fire_top, Some(11));
    assert_eq!(
        engine
            .object_snapshot(id)
            .and_then(|object| object.current_shape),
        Some(DefinitionRect::new(2, 3, 4, 5))
    );

    let partial = engine
        .spawn_object(
            SpawnConfig::new("FIRE")
                .with_loaded(true)
                .with_construction(500)
                .with_shape_rect(DefinitionRect::new(3, 4, 5, 6))
                .with_shape_fire_top(9),
        )
        .expect("partial loaded object spawns");
    let partial_index = engine
        .find_object_index(partial)
        .expect("partial object index");
    engine
        .do_con(partial_index, 100)
        .expect("same-percent DoCon succeeds");
    let same_percent = engine.object_snapshot(partial).expect("partial snapshot");
    assert_eq!(
        same_percent.current_shape,
        Some(DefinitionRect::new(3, 4, 5, 6))
    );
    assert_eq!(same_percent.current_fire_top, Some(9));
    engine
        .do_con(partial_index, 400)
        .expect("percent-crossing DoCon succeeds");
    let crossed = engine.object_snapshot(partial).expect("crossed snapshot");
    assert_eq!(crossed.current_shape, None);
    assert_eq!(crossed.current_fire_top, None);
}

#[cfg(test)]
#[test]
fn movement_circle_wrap_retains_pre_wrap_live_shape() {
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("TURN", "Turning fixture", "").expect("definition compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(-3, -4, 6, 8)));
    definition.set_rotateable(1);
    engine
        .register_definition(definition)
        .expect("definition registers");
    let id = engine
        .spawn_object(
            SpawnConfig::new("TURN")
                .with_rotation(355)
                .with_rotation_velocity(itofix(1)),
        )
        .expect("object spawns");
    let index = engine.find_object_index(id).expect("object index");
    let materials = MaterialSet::new();
    let live = Cell::new(MovementLiveConfig {
        border_bound: 0,
        rotateable: 1,
        action_procedure: ActionProcedure::Undefined,
        action_is_idle: true,
        layer_bounds: None,
    });
    let movement = MovementContactConfig {
        live: &live,
        solid_masks: &[],
        object_id: id,
    };
    engine.objects[index].state.ocf |= ocf::ROTATE;
    engine.objects[index]
        .advance_fixed_rotation(
            None,
            &materials,
            movement,
            false,
            false,
            false,
            |_, _, _| Ok(()),
        )
        .expect("rotation step succeeds");

    let object = &engine.objects[engine.find_object_index(id).expect("object remains")];
    assert_eq!(object.state.rotation, 0, "raw r wraps after reaching 360");
    assert_eq!(object.fixed_rotation, C4Fixed::ZERO);
    assert_eq!(
        object.current_shape_rect(),
        Some(DefinitionRect::new(-7, -7, 14, 14)),
        "C++ does not rebuild Shape after the circle-bound r write"
    );
    assert_eq!(
        object.snapshot(None).current_shape,
        Some(DefinitionRect::new(-7, -7, 14, 14))
    );
}

#[cfg(test)]
#[test]
fn vertical_bounds_preserve_cpp_million_pixel_sentinels() {
    struct Case {
        name: &'static str,
        border_bound: i32,
        target: i32,
        expected_target: i32,
        initial_ydir: i32,
        expected_contact: Option<u32>,
    }

    let mut engine = Engine::new();
    let mut definition = Definition::from_script("VBND", "Vertical bounds fixture", "")
        .expect("definition compiles");
    definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    engine
        .register_definition(definition)
        .expect("definition registers");
    let object_id = engine
        .spawn_object(SpawnConfig::new("VBND"))
        .expect("object spawns");
    let object_index = engine.find_object_index(object_id).expect("object index");
    let landscape = Landscape::flat(8, 20);
    let live = Cell::new(MovementLiveConfig {
        border_bound: 0,
        rotateable: 0,
        action_procedure: ActionProcedure::Undefined,
        action_is_idle: true,
        layer_bounds: None,
    });
    let movement = MovementContactConfig {
        live: &live,
        solid_masks: &[],
        object_id,
    };
    // A raw fix_y projects only to about +/-32K, but the preceding layer
    // TargetBounds arm can replace that target with any i32. Exercise the
    // shared vertical helper directly instead of walking a million pixels.
    let original_fixed_position = FixedVec2::new(fixed100(25), fixed100(-25));

    let cases = [
        Case {
            name: "top inside",
            border_bound: C4D_BORDER_TOP,
            target: 999_999,
            expected_target: 999_999,
            initial_ydir: 3,
            expected_contact: None,
        },
        Case {
            name: "top exact",
            border_bound: C4D_BORDER_TOP,
            target: 1_000_000,
            expected_target: 1_000_000,
            initial_ydir: 3,
            expected_contact: None,
        },
        Case {
            name: "top across",
            border_bound: C4D_BORDER_TOP,
            target: 1_000_001,
            expected_target: 1_000_000,
            initial_ydir: 3,
            expected_contact: Some(CNAT_BOTTOM),
        },
        Case {
            name: "bottom inside",
            border_bound: C4D_BORDER_BOTTOM,
            target: -999_999,
            expected_target: -999_999,
            initial_ydir: -3,
            expected_contact: None,
        },
        Case {
            name: "bottom exact",
            border_bound: C4D_BORDER_BOTTOM,
            target: -1_000_000,
            expected_target: -1_000_000,
            initial_ydir: -3,
            expected_contact: None,
        },
        Case {
            name: "bottom across",
            border_bound: C4D_BORDER_BOTTOM,
            target: -1_000_001,
            expected_target: -1_000_000,
            initial_ydir: -3,
            expected_contact: Some(CNAT_TOP),
        },
    ];

    for case in cases {
        live.set(MovementLiveConfig {
            border_bound: case.border_bound,
            rotateable: 0,
            action_procedure: ActionProcedure::Undefined,
            action_is_idle: true,
            layer_bounds: None,
        });
        let object = &mut engine.objects[object_index];
        object.fixed_position = original_fixed_position;
        object.fixed_velocity = FixedVec2::new(itofix(2), itofix(case.initial_ydir));
        object.state.velocity = object.velocity_pixels();
        object.frame_t_contact = CNAT_LEFT;
        object.frame_shape_contact_cnat = CNAT_RIGHT;
        object.frame_shape_contact_count = 7;

        let mut target = case.target;
        let mut contacts = Vec::new();
        {
            let mut on_contact =
                |object: &mut Object, _: &Landscape, dispatch: MovementContactDispatch| {
                    let MovementContactDispatch::Direct(cnat) = dispatch else {
                        panic!("TargetBounds must dispatch a direct contact");
                    };
                    contacts.push((
                        cnat,
                        object.fixed_velocity.x.val(),
                        object.fixed_velocity.y.val(),
                        object.state.velocity.x,
                        object.state.velocity.y,
                    ));
                    Ok(())
                };
            object
                .apply_vertical_bounds(&mut target, &landscape, movement, &mut on_contact)
                .expect(case.name);
        }

        let expected_ydir = if case.expected_contact.is_some() {
            C4Fixed::ZERO
        } else {
            itofix(case.initial_ydir)
        };
        let expected_contacts = case
            .expected_contact
            .into_iter()
            .map(|cnat| (cnat, itofix(2).val(), 0, 2, 0))
            .collect::<Vec<_>>();
        assert_eq!(target, case.expected_target, "{} target", case.name);
        assert_eq!(object.fixed_velocity.y, expected_ydir, "{} ydir", case.name);
        assert_eq!(
            object.state.velocity.y,
            fixtoi(expected_ydir),
            "{} integer ydir",
            case.name
        );
        assert_eq!(object.fixed_velocity.x, itofix(2), "{} xdir", case.name);
        assert_eq!(object.state.velocity.x, 2, "{} integer xdir", case.name);
        assert_eq!(contacts, expected_contacts, "{} contacts", case.name);
        assert_eq!(
            object.fixed_position, original_fixed_position,
            "{} must not resynchronize fix_y",
            case.name
        );
        assert_eq!(object.frame_t_contact, CNAT_LEFT, "{} t_contact", case.name);
        assert_eq!(
            object.frame_shape_contact_cnat, CNAT_RIGHT,
            "{} shape contact CNAT",
            case.name
        );
        assert_eq!(
            object.frame_shape_contact_count, 7,
            "{} shape contact count",
            case.name
        );
    }
}

#[cfg(test)]
#[test]
fn shape_refresh_order_preserves_only_later_script_vertex_edits() {
    // C4Object::SetRotation performs UpdateFace/UpdateShape inline. A later
    // AddVertex survives; an earlier AddVertex is discarded by that refresh.
    // Exercise both the calling-object and foreign-object copy-out folds.
    let mut engine = Engine::new();
    let mut definition = Definition::from_script(
        "VRTX",
        "Vertex ordering fixture",
        r#"#strict
public func RotateThenAdd()
{
    SetR(90);
    AddVertex(17, -9);
    return GetVertexNum();
}
public func AddThenRotate()
{
    AddVertex(17, -9);
    SetR(90);
    return GetVertexNum();
}
public func MutateTarget(target)
{
    return target->RotateThenAdd();
}
"#,
    )
    .expect("vertex-ordering definition compiles");
    definition.set_c4_callback_convention(true);
    definition.set_shape_rect(Some(DefinitionRect::new(-2, -2, 4, 4)));
    definition.set_shape_vertices(vec![ObjectVertex::new(2, 0)]);
    definition.set_rotateable(1);
    engine
        .register_definition(definition)
        .expect("vertex-ordering definition registers");

    let rotate_then_add = engine
        .spawn_object(SpawnConfig::new("VRTX"))
        .expect("rotate-then-add object spawns");
    let rotate_then_add_index = engine
        .find_object_index(rotate_then_add)
        .expect("rotate-then-add object index");
    assert_eq!(
        engine
            .call_object_function(rotate_then_add_index, "RotateThenAdd", Vec::new())
            .expect("rotate-then-add runs"),
        Value::Int(2)
    );
    assert_eq!(
        engine
            .object_snapshot(rotate_then_add)
            .expect("rotate-then-add snapshot")
            .vertices
            .last(),
        Some(&ObjectVertex::new(17, -9))
    );

    let add_then_rotate = engine
        .spawn_object(SpawnConfig::new("VRTX"))
        .expect("add-then-rotate object spawns");
    let add_then_rotate_index = engine
        .find_object_index(add_then_rotate)
        .expect("add-then-rotate object index");
    assert_eq!(
        engine
            .call_object_function(add_then_rotate_index, "AddThenRotate", Vec::new())
            .expect("add-then-rotate runs"),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .object_snapshot(add_then_rotate)
            .expect("add-then-rotate snapshot")
            .vertices,
        vec![ObjectVertex::new(0, 2)]
    );

    let caller = engine
        .spawn_object(SpawnConfig::new("VRTX"))
        .expect("foreign caller spawns");
    let foreign = engine
        .spawn_object(SpawnConfig::new("VRTX"))
        .expect("foreign target spawns");
    let caller_index = engine.find_object_index(caller).expect("caller index");
    assert_eq!(
        engine
            .call_object_function(
                caller_index,
                "MutateTarget",
                vec![Value::Object(foreign.as_u64())],
            )
            .expect("foreign mutation runs"),
        Value::Int(2)
    );
    assert_eq!(
        engine
            .object_snapshot(foreign)
            .expect("foreign snapshot")
            .vertices
            .last(),
        Some(&ObjectVertex::new(17, -9))
    );
}

#[cfg(test)]
#[test]
fn enter_and_status_activation_refresh_shape_before_callbacks_but_keep_later_setshape() {
    let mut engine = Engine::new();
    let mut container = Definition::from_script(
        "CONT",
        "Container",
        r#"#strict
local collection_wdt;
protected func Collection2(item) { collection_wdt = GetObjWidth(item); }
public func ReadCollectionWdt() { return collection_wdt; }
"#,
    )
    .expect("container compiles");
    container.set_c4_callback_convention(true);
    engine
        .register_definition(container)
        .expect("container registers");

    let mut item = Definition::from_script(
        "ITEM",
        "Item",
        r#"#strict
local entrance_wdt;
public func ProbeEnter(container)
{
    SetShape(-1, -1, 27, 41);
    if (!Enter(container)) return -1;
    var after_enter = GetObjWidth();
    SetShape(-2, -3, 9, 11);
    return after_enter;
}
protected func Entrance(container) { entrance_wdt = GetObjWidth(); }
public func ReadEntranceWdt() { return entrance_wdt; }
"#,
    )
    .expect("item compiles");
    item.set_c4_callback_convention(true);
    item.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
    engine.register_definition(item).expect("item registers");

    let mut activatable = Definition::from_script(
        "ACTV",
        "Activatable",
        r#"#strict
local transfer_wdt;
public func ProbeActivate()
{
    SetShape(-1, -1, 27, 41);
    if (!SetObjectStatus(1)) return -1;
    var after_activate = GetObjWidth();
    SetShape(-2, -3, 9, 11);
    return after_activate;
}
protected func UpdateTransferZone() { transfer_wdt = GetObjWidth(); }
public func ReadTransferWdt() { return transfer_wdt; }
"#,
    )
    .expect("activatable compiles");
    activatable.set_c4_callback_convention(true);
    activatable.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
    engine
        .register_definition(activatable)
        .expect("activatable registers");

    let container_id = engine
        .spawn_object(SpawnConfig::new("CONT"))
        .expect("container spawns");
    let item_id = engine
        .spawn_object(SpawnConfig::new("ITEM"))
        .expect("item spawns");
    let item_index = engine.find_object_index(item_id).expect("item index");
    assert_eq!(
        engine
            .call_object_function(
                item_index,
                "ProbeEnter",
                vec![Value::Object(container_id.as_u64())],
            )
            .expect("Enter probe runs"),
        Value::Int(4),
        "Enter's UpdateFace is visible before the host call returns"
    );
    let item_index = engine.find_object_index(item_id).expect("item remains");
    assert_eq!(
        engine
            .call_object_function(item_index, "ReadEntranceWdt", Vec::new())
            .expect("Entrance observation reads"),
        Value::Int(4)
    );
    let container_index = engine
        .find_object_index(container_id)
        .expect("container remains");
    assert_eq!(
        engine
            .call_object_function(container_index, "ReadCollectionWdt", Vec::new())
            .expect("Collection2 observation reads"),
        Value::Int(4)
    );
    assert_eq!(
        engine
            .object_snapshot(item_id)
            .expect("item snapshot")
            .current_shape,
        Some(DefinitionRect::new(-2, -3, 9, 11)),
        "SetShape after Enter survives the deferred host fold"
    );

    let activatable_id = engine
        .spawn_object(
            SpawnConfig::new("ACTV")
                .with_status(ObjectStatus::Inactive)
                .with_loaded(true)
                .with_shape_rect(DefinitionRect::new(-1, -1, 27, 41)),
        )
        .expect("inactive object spawns");
    let activatable_index = engine
        .find_object_index(activatable_id)
        .expect("activatable index");
    assert_eq!(
        engine
            .call_object_function(activatable_index, "ProbeActivate", Vec::new())
            .expect("activation probe runs"),
        Value::Int(4)
    );
    let activatable_index = engine
        .find_object_index(activatable_id)
        .expect("activatable remains");
    assert_eq!(
        engine
            .call_object_function(activatable_index, "ReadTransferWdt", Vec::new())
            .expect("UpdateTransferZone observation reads"),
        Value::Int(4)
    );
    let activatable = engine
        .object_snapshot(activatable_id)
        .expect("activatable snapshot");
    assert_eq!(activatable.status, ObjectStatus::Normal);
    assert_eq!(
        activatable.current_shape,
        Some(DefinitionRect::new(-2, -3, 9, 11)),
        "SetShape after StatusActivate survives its native UpdateFace"
    );
}

fn denumerate_object_reference(reference: &mut Option<ObjectId>, object_numbers: &HashSet<u64>) {
    if reference.is_some_and(|id| !object_numbers.contains(&id.as_u64())) {
        *reference = None;
    }
}

fn denumerate_legacy_enumerated_object_reference(
    reference: &mut Option<ObjectId>,
    object_numbers: &HashSet<u64>,
) {
    let Some(raw) = reference.map(ObjectId::as_u64) else {
        return;
    };
    // Old saves used Game.Objects.Enumerated and added C4EnumPointer1.
    // C4EnumeratedObjectPtr recognizes only this inclusive reserved band.
    let number = if (1_000_000_000..=1_001_000_000).contains(&raw) {
        raw - 1_000_000_000
    } else {
        raw
    };
    *reference = object_numbers
        .contains(&number)
        .then(|| ObjectId::new(number));
}

#[cfg(test)]
#[test]
fn legacy_enumerated_object_reference_removes_the_compatibility_offset() {
    let mut reference = Some(ObjectId::new(1_000_000_042));
    denumerate_legacy_enumerated_object_reference(&mut reference, &HashSet::from([42]));
    assert_eq!(reference, Some(ObjectId::new(42)));

    let mut missing = Some(ObjectId::new(1_000_000_043));
    denumerate_legacy_enumerated_object_reference(&mut missing, &HashSet::from([42]));
    assert_eq!(missing, None);
}

#[cfg(test)]
#[test]
fn legacy_object_load_denumerates_every_object_pointer_wrapper() -> Result<(), EngineError> {
    let mut engine = Engine::new();
    engine.register_definition(Definition::from_script("PTRS", "Pointers", "")?)?;
    let target = engine.spawn_object(
        SpawnConfig::new("PTRS")
            .with_id(ObjectId::new(42))
            .with_loaded(true),
    )?;
    let source = engine.spawn_object(
        SpawnConfig::new("PTRS")
            .with_id(ObjectId::new(7))
            .with_loaded(true),
    )?;
    let source_index = engine
        .find_object_index(source)
        .expect("source object remains present");
    let legacy_target = ObjectId::new(1_000_000_042);
    let source_state = &mut engine.objects[source_index].state;
    source_state.action.target = Some(legacy_target);
    source_state.action.target2 = Some(legacy_target);
    source_state.container = Some(legacy_target);
    source_state.layer = Some(legacy_target);
    source_state.graphics_overlays.push(
        ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Object)
            .with_overlay_object(Some(legacy_target)),
    );

    engine.finish_legacy_object_load();

    let source_index = engine
        .find_object_index(source)
        .expect("source object remains present");
    let source_state = &engine.objects[source_index].state;
    assert_eq!(source_state.action.target, Some(target));
    assert_eq!(source_state.action.target2, Some(target));
    assert_eq!(source_state.container, Some(target));
    assert_eq!(source_state.layer, Some(target));
    assert_eq!(
        source_state.graphics_overlays[0].overlay_object,
        Some(target)
    );
    Ok(())
}

fn denumerate_effect_value(value: &mut EffectVarValue, object_numbers: &HashSet<u64>) {
    match value {
        EffectVarValue::Object(id) if !object_numbers.contains(id) => {
            *value = EffectVarValue::Nil;
        }
        EffectVarValue::Array(values) => {
            for value in values {
                denumerate_effect_value(value, object_numbers);
            }
        }
        EffectVarValue::Proplist(entries) => {
            *entries = denumerate_script_map(entries, object_numbers);
        }
        _ => {}
    }
}

fn denumerate_effect(effect: &mut EffectState, object_numbers: &HashSet<u64>) {
    if effect.command_target.is_some_and(|target| {
        u64::try_from(target)
            .ok()
            .is_none_or(|target| !object_numbers.contains(&target))
    }) {
        effect.command_target = None;
    }
    for value in &mut effect.vars {
        denumerate_effect_value(value, object_numbers);
    }
}

/// Complete the load-only half of `C4Effect::DenumeratePointers`.
///
/// Native loading resolves the command object first and then
/// `AssignCallbackFunctions`/`GetCallbackScript` refreshes the serialized
/// command ID from that object's current definition. This is deliberately
/// separate from ordinary pointer clearing: removal denumeration must not
/// pretend that callbacks were rebound as part of a fresh load.
fn denumerate_loaded_effect(
    effect: &mut EffectState,
    object_numbers: &HashSet<u64>,
    object_definition_ids: &HashMap<u64, DefinitionId>,
) {
    denumerate_effect(effect, object_numbers);
    let Some(command_target) = effect
        .command_target
        .and_then(|target| u64::try_from(target).ok())
    else {
        return;
    };
    if let Some(definition_id) = object_definition_ids.get(&command_target) {
        effect.command_id = Some(definition_id.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FogOfWarPlayerFrame {
    /// Ordered runtime `C4Player::FoWViewObjs` projection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub view_objects: Vec<ObjectId>,
    /// Runtime-only `C4Player::ViewTarget`, retained by presentation records
    /// even though native player save data deliberately omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_target: Option<ObjectId>,
}

/// Presentation-only requests emitted while advancing one simulation frame.
///
/// This is the lightweight result returned by
/// [`Engine::tick_with_presentation`]. It contains the same transient requests
/// that [`Engine::tick`] attaches to its [`SimulationSnapshot`], without the
/// cost of constructing that full snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickPresentation {
    /// Ordered runtime scoreboard dialog reconciliations.
    pub scoreboard_presentations: Vec<ScoreboardPresentationRequest>,
    /// Menu requests emitted by object commands during the frame.
    pub menu_requests: Vec<MenuRequest>,
    /// Audio commands emitted during the frame.
    pub audio: Vec<AudioCommand>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    pub frame: u64,
    /// `C4Game::Time`, deliberately independent from FrameCounter
    /// (C4Game.cpp:1755-1759,1939-1955).
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub game_time: i32,
    #[serde(default)]
    pub game_over: bool,
    #[serde(default, skip_serializing_if = "RoundResultsState::is_empty")]
    pub round_results: RoundResultsState,
    /// Exact `Game.Parameters.League` bytes. This is independent from the
    /// LeagueAddress-derived `isLeague()` flag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub league_name: Vec<u8>,
    /// Persistent C4PlayerInfo progress buffers keyed by exact info ID.
    /// `None` preserves a null StdStrBuf; `Some(Vec::new())` is allocated
    /// empty and must remain distinguishable for script return values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub player_info_league_progress_data: BTreeMap<i32, Option<Vec<u8>>>,
    /// Nonzero C4PlayerInfo league-score overrides keyed by exact info ID.
    /// Known rows absent from this map have the serialized/default score 0.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub player_info_league_scores: BTreeMap<i32, i32>,
    #[serde(default)]
    pub physics: Option<PhysicsSettings>,
    pub objects: Vec<ObjectSnapshot>,
    /// C++ `C4ObjectList` order as consumed by Draw's Last -> Prev passes
    /// (C4ObjectList.cpp:387-396). Object snapshots remain ID-sorted for
    /// deterministic consumers; an empty list is the legacy fallback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub render_order: Vec<ObjectId>,
    #[serde(default)]
    pub environment: EnvironmentFrame,
    #[serde(default)]
    pub sky: Option<SkyFrame>,
    #[serde(default)]
    pub weather_events: Vec<WeatherEvent>,
    #[serde(default)]
    pub global_effects: Vec<EffectState>,
    #[serde(default, skip_serializing_if = "ScriptGlobalState::is_empty")]
    pub script_globals: ScriptGlobalState,
    #[serde(default)]
    pub particles: Vec<ParticleSnapshot>,
    #[serde(default)]
    pub players: Vec<PlayerState>,
    /// Runtime fog-of-war player projections. These are deterministic frame
    /// presentation data; the underlying player fields remain NoSave.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fow_players: BTreeMap<i32, FogOfWarPlayerFrame>,
    #[serde(default)]
    pub crew_selection: HashMap<i32, CrewSelectionState>,
    #[serde(default)]
    pub crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    #[serde(default)]
    pub known_crew_owners: Vec<i32>,
    #[serde(default)]
    pub eliminated_crew_owners: Vec<i32>,
    #[serde(default)]
    pub landscape: Option<Landscape>,
    #[serde(default = "default_rng")]
    pub rng: LcgRng,
    #[serde(default)]
    pub surfaces: Vec<SurfaceSnapshot>,
    #[serde(default)]
    pub hud: HudSnapshot,
    #[serde(default)]
    pub controls: Vec<String>,
    #[serde(default)]
    pub network_packets: Vec<NetworkPacketSnapshot>,
    #[serde(default)]
    pub definition_categories: HashMap<DefinitionId, i32>,
    /// Definition `ClosedContainer` values needed by the viewport FoW pass.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub definition_closed_containers: BTreeMap<DefinitionId, i32>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub definition_lines: HashMap<DefinitionId, DefinitionLineMetadata>,
    #[serde(default)]
    pub transfer_zones: Vec<TransferZoneState>,
    /// Presentation-only graph retained by the most recent pathfinder run.
    #[serde(default, skip_serializing_if = "PathfinderDebugSnapshot::is_empty")]
    pub pathfinder_debug: PathfinderDebugSnapshot,
    #[serde(default)]
    pub menu_requests: Vec<MenuRequest>,
    #[serde(default)]
    pub audio: Vec<AudioCommand>,
}

impl SimulationSnapshot {
    pub fn object(&self, id: ObjectId) -> Option<&ObjectSnapshot> {
        self.objects.iter().find(|object| object.id == id)
    }

    pub fn object_visible_for_player(&self, id: ObjectId, player: i32, as_overlay: bool) -> bool {
        self.object(id).is_some_and(|object| {
            object_visible_for_player(&self.objects, &self.players, object, player, as_overlay)
        })
    }
}

#[cfg(test)]
#[test]
fn simulation_snapshot_roundtrips_fow_presentation_and_defaults_legacy_frames() {
    let mut snapshot = Engine::new().snapshot();
    snapshot.environment.fow_color = 0x7f12_3456;
    snapshot.environment.fow_resolution = 96;
    snapshot.fow_players.insert(
        2,
        FogOfWarPlayerFrame {
            view_objects: vec![ObjectId::new(9), ObjectId::new(3)],
            view_target: Some(ObjectId::new(11)),
        },
    );
    snapshot
        .definition_closed_containers
        .insert("HUT1".into(), 1);

    let encoded = serde_json::to_value(&snapshot).expect("snapshot serializes");
    let restored: SimulationSnapshot =
        serde_json::from_value(encoded.clone()).expect("snapshot deserializes");
    assert_eq!(restored.environment.fow_color, 0x7f12_3456);
    assert_eq!(restored.environment.fow_resolution, 96);
    assert_eq!(restored.fow_players, snapshot.fow_players);
    assert_eq!(
        restored.definition_closed_containers,
        snapshot.definition_closed_containers
    );

    let mut legacy = encoded;
    let root = legacy.as_object_mut().expect("snapshot JSON object");
    root.remove("fow_players");
    root.remove("definition_closed_containers");
    let environment = root
        .get_mut("environment")
        .and_then(serde_json::Value::as_object_mut)
        .expect("environment JSON object");
    environment.remove("fow_color");
    environment.remove("fow_resolution");
    let restored: SimulationSnapshot =
        serde_json::from_value(legacy).expect("legacy snapshot deserializes");
    assert_eq!(restored.environment.fow_color, 0);
    assert_eq!(restored.environment.fow_resolution, DEFAULT_FOW_RESOLUTION);
    assert!(restored.fow_players.is_empty());
    assert!(restored.definition_closed_containers.is_empty());
}

/// C4Object::IsVisible (C4Object.cpp:5600-5629), shared by presentation
/// consumers that only hold snapshot slices.
pub fn object_visible_for_player(
    objects: &[ObjectSnapshot],
    players: &[PlayerState],
    object: &ObjectSnapshot,
    player: i32,
    as_overlay: bool,
) -> bool {
    fn hostile(players: &[PlayerState], first: i32, second: i32) -> bool {
        let Some(first_player) = players.iter().find(|candidate| candidate.id == first) else {
            return false;
        };
        let Some(second_player) = players.iter().find(|candidate| candidate.id == second) else {
            return false;
        };
        first != second
            && (first_player.is_hostile_towards(second) || second_player.is_hostile_towards(first))
    }

    fn inner(
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        object: &ObjectSnapshot,
        player: i32,
        as_overlay: bool,
        visiting: &mut HashSet<ObjectId>,
    ) -> bool {
        // Valid C++ layer graphs are acyclic. Avoid unbounded recursion for a
        // malformed imported cycle while preserving the already-evaluated
        // ancestor's visibility.
        if !visiting.insert(object.id) {
            return true;
        }
        let result = (|| {
            let visibility = object.visibility;
            if visibility & VIS_OVERLAY_ONLY != 0 {
                if !as_overlay {
                    return false;
                }
                if visibility == VIS_OVERLAY_ONLY {
                    return true;
                }
            }

            if !as_overlay {
                if let Some(layer_id) = object.layer.filter(|layer| *layer != object.id) {
                    if let Some(layer) = objects.iter().find(|candidate| candidate.id == layer_id) {
                        let mut layer_visible =
                            inner(objects, players, layer, player, false, visiting);
                        if layer.visibility & VIS_LAYER_TOGGLE != 0 {
                            layer_visible = !layer_visible;
                        }
                        if !layer_visible {
                            return false;
                        }
                    }
                }
            }

            if visibility == VIS_ALL {
                return true;
            }

            let mut visible = visibility & VIS_OWNER != 0 && player == object.owner;
            if player != OWNER_NONE {
                let is_other = player != object.owner;
                let is_hostile = hostile(players, player, object.owner);
                if visibility & VIS_ALLIES != 0 {
                    visible |= is_other && !is_hostile;
                }
                if visibility & VIS_ENEMIES != 0 {
                    visible |= is_other && is_hostile;
                }
                if visibility & VIS_LOCAL != 0 && player >= 0 {
                    let slot = player / 32;
                    let bit = (player % 32) as u32;
                    let local = object
                        .local_vars
                        .get(&format!("__local_{slot}"))
                        .and_then(Value::as_c4_int)
                        .unwrap_or(0);
                    visible |= local & 1_i32.wrapping_shl(bit) != 0;
                }
            } else {
                visible |= visibility & VIS_GOD != 0;
            }
            visible
        })();
        visiting.remove(&object.id);
        result
    }

    inner(
        objects,
        players,
        object,
        player,
        as_overlay,
        &mut HashSet::new(),
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedObject {
    pub snapshot: ObjectSnapshot,
    /// Loaded C4Object::Mass cache. None means the object has already passed
    /// through UpdateMass (or came from a legacy Rust snapshot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compiled_mass: Option<i32>,
    #[serde(default)]
    pub command_queue: Vec<QueuedCommand>,
    #[serde(default)]
    pub command_stack: CommandStackSnapshot,
    #[serde(default)]
    motion_x: i32,
    #[serde(default)]
    motion_y: i32,
    #[serde(default, skip_serializing_if = "ObjectCompilerCache::is_default")]
    compiler_cache: ObjectCompilerCache,
    #[serde(
        default = "default_last_attach_movement_frame",
        skip_serializing_if = "i32_is_minus_one"
    )]
    last_attach_movement_frame: i32,
    #[serde(default)]
    no_collect_delay: i32,
    #[serde(default, skip_serializing_if = "ShapeAttachRecord::is_unattached")]
    shape_attach: ShapeAttachRecord,
    #[serde(default, skip_serializing_if = "is_false")]
    entrance_status: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    crew_disabled: bool,
    /// C4Object::SolidMask is savegame state, while pSolidMaskData and its
    /// material buffer are rebuilt after loading (C4Object.cpp:2797;
    /// C4Object.h:177-178).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    solid_mask_override: Option<DefinitionTargetRect>,
    /// Dormant fixed C4Shape slots are stateful but intentionally absent
    /// from public/differential ObjectSnapshot. Persist only when the raw
    /// buffer cannot be reconstructed from the active vertex prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shape_vertices: Option<ShapeVertexBuffer>,
}

/// Scenario selected by `SetNextMission` for the evaluation dialog.
/// C4Game persists the path, button text, and description independently
/// (C4Game.cpp:1963-1965).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextMissionState {
    pub path: String,
    pub text: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub frame: u64,
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub game_time: i32,
    /// Saved `Game.Parameters.MaxPlayers`. `None` keeps the parameter value
    /// seeded by the scenario/app when restoring an older Rust state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_players: Option<i32>,
    /// Saved `Game.Parameters.StartupPlayerCount`. `None` keeps the value
    /// frozen by the replay/app startup seam when restoring an older state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_player_count: Option<i32>,
    /// Saved `Game.Parameters.League`. None preserves an app-seeded value
    /// when restoring Rust states written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub league_name: Option<Vec<u8>>,
    /// Saved `Game.PlayerInfos` league progress projection. None preserves
    /// the app/control registry projection for older Rust state files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_info_league_progress_data: Option<BTreeMap<i32, Option<Vec<u8>>>>,
    /// Saved nonzero C4PlayerInfo league-score overrides. None preserves the
    /// app/control-registry projection when restoring older Rust states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_info_league_scores: Option<BTreeMap<i32, i32>>,
    /// Saved `Game.Parameters.UseFairCrew`. Older Rust states predate this
    /// field and resume with the standalone engine's enabled default.
    #[serde(default = "default_use_fair_crew")]
    pub use_fair_crew: bool,
    /// Saved `Game.Parameters.FairCrewStrength`. Older Rust states resume at
    /// LegacyClonk's configured default strength.
    #[serde(default = "default_fair_crew_strength")]
    pub fair_crew_strength: i32,
    /// Saved `Game.Parameters.FairCrewForced`. `None` keeps the synchronized
    /// scenario value when restoring states written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fair_crew_forced: Option<bool>,
    /// Saved `Game.Parameters.AllowDebug`. `None` keeps the synchronized
    /// scenario value when restoring older states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_debug: Option<bool>,
    /// Saved `Game.Control.ControlRate`. `None` preserves the timing installed
    /// by the scenario/network bootstrap for older Rust states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_rate: Option<i32>,
    /// Saved `Game.MessageInput::Commands`. Older Rust states predate this
    /// field and resume with the stock `/speed` registration.
    #[serde(
        default = "default_message_board_commands",
        skip_serializing_if = "message_board_commands_are_default"
    )]
    pub message_board_commands: Vec<InitialNetworkMessageBoardCommand>,
    pub physics: PhysicsSettings,
    pub environment: EnvironmentSettings,
    /// C4Weather::CompileFunc persists Game.GraphicsSystem.dwGamma under
    /// `Gamma` with default controls for legacy saves (C4Weather.cpp:302-309).
    #[serde(default, skip_serializing_if = "GammaControlState::is_default")]
    pub gamma: GammaControlState,
    /// Raw `Game.PlayList` filter persisted in Game.txt. `None` is the
    /// internal default playlist; `Some("")` is an explicit filter matching
    /// no normally named song. Older Rust saves predate this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_list: Option<String>,
    /// Saved `Game.iMusicLevel`, multiplied into configured music volume
    /// while a game is running. C++ omits its default value from Game.txt.
    #[serde(
        default = "default_music_level",
        skip_serializing_if = "music_level_is_default"
    )]
    pub music_level: u8,
    pub next_object_id: u64,
    #[serde(default)]
    pub landscape: Option<Landscape>,
    /// True when `landscape` was captured through the native active-list
    /// RemoveSolidMasks bracket and therefore needs active masks re-put after
    /// object restoration. Pixels owned by linked runtime-inactive masks
    /// intentionally survive that bracket; loaded inactive objects have no
    /// mask instance. Older EngineState JSON and SimulationSnapshot
    /// projections contain the live baked plane and retain the old no-re-put
    /// behavior.
    #[serde(default, skip_serializing_if = "is_false")]
    #[doc(hidden)]
    pub solid_masks_removed_from_landscape: bool,
    /// Standalone Rust state files retain the active `Game.C4S` reflection
    /// view. C++ saves this beside Game.txt as Scenario.txt; `None` keeps the
    /// preloaded scenario when reading older Rust states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub scenario_values: Option<scenario::ScenarioValueStore>,
    /// Saved BASEFUNC_RejectEntrance projection. None keeps the scenario
    /// value already installed when restoring states written before L051.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_reject_entrance_enabled: Option<bool>,
    #[serde(default)]
    pub objects: Vec<PersistedObject>,
    /// Main object-list order in execution direction (the C++ list reversed).
    /// Kept separate so comparator snapshots may retain their normalized order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object_order: Vec<ObjectId>,
    /// Inactive object-list order in the same reversed representation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inactive_object_order: Vec<ObjectId>,
    #[serde(default)]
    pub particles: Vec<ParticleSnapshot>,
    #[serde(default)]
    pub players: Vec<PlayerState>,
    /// States written before ordered C4Player::Crew persistence omitted the
    /// roster field entirely. False requests the one-time owner+legacy-bit
    /// compatibility import; true makes an intentionally empty roster exact.
    #[serde(default, skip_serializing_if = "is_false")]
    pub player_crew_rosters_authoritative: bool,
    /// C4PlayerInfoList::iLastPlayerID (C4PlayerInfo.cpp:1733-1742);
    /// allocation is a later behavior.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    pub last_player_info_id: i32,
    /// Scenario `[Head] ForcedAutoStopControl`; retained so players joining
    /// after a save restore receive the same scenario override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced_control_style: Option<bool>,
    /// Scenario `[Head] ForcedAutoContextMenu`; retained for later joins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced_auto_context_menu: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<TeamInfo>,
    /// Full saved C4TeamList configuration. None keeps the scenario-seeded
    /// values when loading Rust states written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[doc(hidden)]
    pub team_configuration: Option<TeamConfiguration>,
    /// Remaining C4TeamList::CompileFunc state not represented by the
    /// script-queryable TeamConfiguration flags.
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    #[doc(hidden)]
    pub team_last_team_id: i32,
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    #[doc(hidden)]
    pub team_max_script_players: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[doc(hidden)]
    pub team_script_player_names: Vec<u8>,
    #[serde(default, skip_serializing_if = "i32_is_zero")]
    #[doc(hidden)]
    pub team_random_team_count: i32,
    #[serde(default)]
    pub crew_selection: HashMap<i32, CrewSelectionState>,
    #[serde(default)]
    pub crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    /// Persistent C4ObjectInfoList entries for each runtime player.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub crew_info_rosters: HashMap<i32, Vec<player_file::CrewInfo>>,
    /// Stable list traversal order for each C4ObjectInfoList. Entries live in
    /// append-only slots so exact object-info pointers remain serializable,
    /// while C++ `New` inserts the newest list node at the head.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub crew_info_order: HashMap<i32, Vec<usize>>,
    /// Full payload and exact roster pointer for each live C4Object::Info.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub crew_object_infos: HashMap<ObjectId, CrewObjectInfo>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub crew_info_links: HashMap<ObjectId, CrewInfoLink>,
    #[serde(default)]
    pub global_effects: Vec<EffectState>,
    #[serde(default, skip_serializing_if = "ScriptGlobalState::is_empty")]
    pub script_globals: ScriptGlobalState,
    #[serde(default)]
    pub known_crew_owners: Vec<i32>,
    #[serde(default)]
    pub eliminated_crew_owners: Vec<i32>,
    #[serde(default)]
    pub transfer_zones: Vec<TransferZoneState>,
    #[serde(default)]
    pub messages: Vec<PersistedMessage>,
    #[serde(default)]
    pub pending_menu_requests: Vec<MenuRequest>,
    #[serde(default)]
    pub next_mission: NextMissionState,
    #[serde(default, skip_serializing_if = "ScoreboardState::is_default")]
    pub scoreboard: ScoreboardState,
    #[serde(default)]
    pub game_over: bool,
    #[serde(default, skip_serializing_if = "RoundResultsState::is_empty")]
    pub round_results: RoundResultsState,
    #[serde(default)]
    pub landscape_insert_thrust: bool,
    /// Saved `Game.Rules & C4RULE_StructuresSnowIn`. C++ persists the
    /// derived Rules bitmask between its Tick255 refreshes (C4Game.cpp:1957).
    #[serde(default)]
    pub structures_snow_in: bool,
    /// Saved `Game.Rules & C4RULE_FlagRemoveable`. Like the C++ Rules
    /// bitmask, this stays cached between frame-one/Tick255 refreshes.
    #[serde(default)]
    pub flag_removeable: bool,
    /// The persistent C4MassMoverSet slots (MassMover.c4b in C++ saves,
    /// C4MassMover.cpp:181-217).
    #[serde(default)]
    pub mass_movers: MassMoverSet,
    /// The C4Sky scroll state a savegame persists (C4Sky::CompileFunc,
    /// C4Sky.cpp:248-251).
    #[serde(default)]
    pub sky: Option<SkyFrame>,
    pub rng: LcgRng,
}

impl EngineState {
    /// Serializes the engine state to a writer using pretty-printed JSON.
    pub fn to_writer<W: Write>(&self, mut writer: W) -> Result<(), EngineStateIoError> {
        serde_json::to_writer_pretty(&mut writer, self).map_err(EngineStateIoError::from)?;
        writer.flush().map_err(EngineStateIoError::from)
    }

    /// Deserializes an engine state from any reader containing JSON data.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self, EngineStateIoError> {
        serde_json::from_reader(reader).map_err(EngineStateIoError::from)
    }

    /// Saves the state to a JSON file at the given path.
    pub fn save_to_path<P: AsRef<Path>>(&self, path: P) -> Result<(), EngineStateIoError> {
        let mut file = File::create(path)?;
        self.to_writer(&mut file)
    }

    /// Loads a state from a JSON file at the given path.
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self, EngineStateIoError> {
        let file = File::open(path)?;
        Self::from_reader(file)
    }

    /// Serializes the state into a pretty-printed JSON string.
    pub fn to_json_string(&self) -> Result<String, EngineStateIoError> {
        serde_json::to_string_pretty(self).map_err(EngineStateIoError::from)
    }

    /// Parses an engine state from a JSON string.
    pub fn from_json_str(json: &str) -> Result<Self, EngineStateIoError> {
        serde_json::from_str(json).map_err(EngineStateIoError::from)
    }

    /// Builds an engine state snapshot from a simulation frame.
    pub fn from_snapshot(snapshot: &SimulationSnapshot) -> Self {
        let physics = snapshot.physics.unwrap_or_default();

        let mut objects = Vec::with_capacity(snapshot.objects.len());
        for object in &snapshot.objects {
            objects.push(PersistedObject {
                snapshot: object.clone(),
                compiled_mass: None,
                command_queue: object.command_queue.clone(),
                command_stack: object.command_stack.clone(),
                motion_x: 0,
                motion_y: 0,
                compiler_cache: ObjectCompilerCache::default(),
                last_attach_movement_frame: -1,
                no_collect_delay: 0,
                shape_attach: ShapeAttachRecord::default(),
                entrance_status: false,
                crew_disabled: false,
                solid_mask_override: None,
                shape_vertices: None,
            });
        }

        let mut known_crew_owners = snapshot.known_crew_owners.clone();
        known_crew_owners.sort_unstable();
        known_crew_owners.dedup();

        let mut eliminated_crew_owners = snapshot.eliminated_crew_owners.clone();
        eliminated_crew_owners.sort_unstable();
        eliminated_crew_owners.dedup();

        let next_object_id = snapshot
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let players: Vec<_> = snapshot
            .players
            .iter()
            .cloned()
            .map(|mut player| {
                player.prepare_for_save();
                player
            })
            .collect();
        let joined_player_info_ids = players
            .iter()
            .map(|player| player.player_info_id)
            .filter(|id| *id != 0)
            .collect::<HashSet<_>>();
        let saved_player_info_league_progress_data = snapshot
            .player_info_league_progress_data
            .iter()
            .filter(|(id, _)| joined_player_info_ids.contains(id))
            .map(|(&id, data)| (id, Some(data.clone().unwrap_or_default())))
            .collect();
        let saved_player_info_league_scores = snapshot
            .player_info_league_scores
            .iter()
            .filter(|(id, score)| joined_player_info_ids.contains(id) && **score != 0)
            .map(|(&id, &score)| (id, score))
            .collect();
        let mut round_results = snapshot.round_results.clone();
        round_results.prepare_for_save();

        Self {
            frame: snapshot.frame,
            game_time: snapshot.game_time,
            max_players: None,
            startup_player_count: None,
            league_name: Some(snapshot.league_name.clone()),
            player_info_league_progress_data: Some(saved_player_info_league_progress_data),
            player_info_league_scores: Some(saved_player_info_league_scores),
            use_fair_crew: default_use_fair_crew(),
            fair_crew_strength: default_fair_crew_strength(),
            fair_crew_forced: None,
            allow_debug: None,
            control_rate: None,
            message_board_commands: default_message_board_commands(),
            physics,
            environment: snapshot.environment.settings,
            gamma: snapshot.environment.gamma,
            play_list: None,
            music_level: DEFAULT_MUSIC_LEVEL,
            next_object_id,
            landscape: snapshot.landscape.clone(),
            solid_masks_removed_from_landscape: false,
            scenario_values: None,
            base_reject_entrance_enabled: None,
            objects,
            object_order: snapshot.render_order.clone(),
            inactive_object_order: Vec::new(),
            particles: snapshot.particles.clone(),
            players,
            player_crew_rosters_authoritative: true,
            last_player_info_id: snapshot
                .players
                .iter()
                .map(|player| player.player_info_id)
                .chain(
                    snapshot
                        .round_results
                        .players
                        .iter()
                        .map(|player| player.player_info_id),
                )
                .max()
                .unwrap_or(0)
                .max(0),
            forced_control_style: None,
            forced_auto_context_menu: None,
            teams: Vec::new(),
            team_configuration: None,
            team_last_team_id: 0,
            team_max_script_players: 0,
            team_script_player_names: Vec::new(),
            team_random_team_count: 0,
            crew_selection: snapshot.crew_selection.clone(),
            crew_roles: snapshot.crew_roles.clone(),
            crew_info_rosters: HashMap::new(),
            crew_info_order: HashMap::new(),
            crew_object_infos: HashMap::new(),
            crew_info_links: HashMap::new(),
            global_effects: snapshot.global_effects.clone(),
            script_globals: snapshot.script_globals.clone(),
            known_crew_owners,
            eliminated_crew_owners,
            transfer_zones: snapshot.transfer_zones.clone(),
            pending_menu_requests: snapshot.menu_requests.clone(),
            messages: Vec::new(),
            next_mission: NextMissionState::default(),
            scoreboard: snapshot.hud.scoreboard.clone(),
            game_over: snapshot.game_over,
            round_results,
            landscape_insert_thrust: false,
            structures_snow_in: false,
            flag_removeable: false,
            // SimulationSnapshot carries no mover slots (the C++ snapshot
            // boundary is object-level); the set restores empty.
            mass_movers: MassMoverSet::new(),
            sky: snapshot.sky.clone(),
            rng: snapshot.rng.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinitionPicture {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<ResourcePictureRect> for DefinitionPicture {
    fn from(rect: ResourcePictureRect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl DefinitionRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_positive(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    pub fn contains_offset(&self, dx: i32, dy: i32) -> bool {
        let local_x = dx - self.x;
        let local_y = dy - self.y;
        if local_x < 0 || local_y < 0 {
            return false;
        }
        if local_x >= self.width || local_y >= self.height {
            return false;
        }
        true
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        if !self.is_positive() {
            return false;
        }
        let local_x = i64::from(x) - i64::from(self.x);
        let local_y = i64::from(y) - i64::from(self.y);
        local_x >= 0
            && local_y >= 0
            && local_x < i64::from(self.width)
            && local_y < i64::from(self.height)
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        if !self.is_positive() || !other.is_positive() {
            return false;
        }
        let self_right = i64::from(self.x) + i64::from(self.width);
        let self_bottom = i64::from(self.y) + i64::from(self.height);
        let other_right = i64::from(other.x) + i64::from(other.width);
        let other_bottom = i64::from(other.y) + i64::from(other.height);
        i64::from(self.x) < other_right
            && i64::from(other.x) < self_right
            && i64::from(self.y) < other_bottom
            && i64::from(other.y) < self_bottom
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionTargetRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

impl DefinitionTargetRect {
    pub const fn new(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        target_x: i32,
        target_y: i32,
    ) -> Self {
        Self {
            x,
            y,
            width,
            height,
            target_x,
            target_y,
        }
    }

    pub fn is_positive(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// `C4Object::CheckSolidMaskRect` (C4Object.cpp:3820-3827). The size
    /// limits deliberately use the OLD source coordinates: a negative source
    /// origin moves to zero without shrinking the requested width/height.
    pub(crate) fn checked_for_solid_mask_bitmap(
        self,
        bitmap_width: i32,
        bitmap_height: i32,
    ) -> Self {
        let mut checked = Self::new(
            self.x.max(0),
            self.y.max(0),
            self.width.min(bitmap_width.saturating_sub(self.x)),
            self.height.min(bitmap_height.saturating_sub(self.y)),
            self.target_x,
            self.target_y,
        );
        if checked.height <= 0 {
            checked.width = 0;
        }
        checked
    }
}

/// Copy one already-checked C4SolidMask rectangle from its active bitmap.
/// The legacy negative-origin clamp can retain samples beyond the bitmap;
/// `C4Surface::GetPixDw` returns zero there, which the inverted-alpha
/// `IsPixTransparent` path classifies as solid.
pub(crate) fn solid_mask_pixels_for_checked_bitmap(
    mask: DefinitionTargetRect,
    bitmap_width: i32,
    bitmap_height: i32,
    source: &[u8],
) -> Option<Arc<Vec<u8>>> {
    if !mask.is_positive() {
        return None;
    }
    let stride = usize::try_from(bitmap_width).ok()?.checked_mul(4)?;
    let mut solid = Vec::with_capacity(usize::try_from(mask.width.checked_mul(mask.height)?).ok()?);
    for y in 0..mask.height {
        for x in 0..mask.width {
            let Some(source_x) = mask.x.checked_add(x) else {
                solid.push(1);
                continue;
            };
            let Some(source_y) = mask.y.checked_add(y) else {
                solid.push(1);
                continue;
            };
            if source_x < 0 || source_y < 0 || source_x >= bitmap_width || source_y >= bitmap_height
            {
                solid.push(1);
                continue;
            }
            let alpha = usize::try_from(source_y)
                .ok()
                .and_then(|row| row.checked_mul(stride))
                .and_then(|offset| {
                    usize::try_from(source_x)
                        .ok()
                        .and_then(|column| column.checked_mul(4))
                        .and_then(|column| offset.checked_add(column))
                })
                .and_then(|offset| offset.checked_add(3))
                .and_then(|index| source.get(index))
                .copied();
            solid.push(u8::from(alpha.is_none_or(|alpha| alpha >= 128)));
        }
    }
    Some(Arc::new(solid))
}

impl From<ResourceTargetRect> for DefinitionTargetRect {
    fn from(rect: ResourceTargetRect) -> Self {
        Self::new(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.target_x,
            rect.target_y,
        )
    }
}

impl From<ResourcePictureRect> for DefinitionRect {
    fn from(rect: ResourcePictureRect) -> Self {
        Self::new(rect.x, rect.y, rect.width, rect.height)
    }
}

fn vertex_bounds_rect(position: Vector2, vertices: &[ObjectVertex]) -> Option<DefinitionRect> {
    let first = vertices.first()?;
    let mut min_x = first.x;
    let mut max_x = first.x;
    let mut min_y = first.y;
    let mut max_y = first.y;
    for vertex in &vertices[1..] {
        min_x = min_x.min(vertex.x);
        max_x = max_x.max(vertex.x);
        min_y = min_y.min(vertex.y);
        max_y = max_y.max(vertex.y);
    }
    Some(DefinitionRect::new(
        position.x.saturating_add(min_x),
        position.y.saturating_add(min_y),
        max_x.saturating_sub(min_x).saturating_add(1),
        max_y.saturating_sub(min_y).saturating_add(1),
    ))
}

#[derive(Clone)]
pub struct DefinitionPictureImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
    color_mask: Option<Arc<[u8]>>,
}

fn definition_color_mask_channels(mask: &[u8], width: u32, height: u32) -> Option<usize> {
    let pixels = usize::try_from(u64::from(width).checked_mul(u64::from(height))?).ok()?;
    if mask.len() == pixels {
        Some(1)
    } else if mask.len() == pixels.checked_mul(4)? {
        Some(4)
    } else {
        None
    }
}

fn definition_color_mask_has_coverage(mask: &[u8], channels: usize) -> bool {
    if channels == 4 {
        mask.chunks_exact(4).any(|pixel| pixel[3] != 0)
    } else {
        mask.iter().any(|value| *value != 0)
    }
}

const C4_DEFINITION_GAME_PALETTE: &[u8; 256 * 3] =
    include_bytes!("../../../planet/Graphics.c4g/C4.PAL");

fn c4_definition_palette_pixel(index: u8) -> [u8; 4] {
    if index == 0 {
        return [0, 0, 0, 0];
    }
    if index == 191 {
        return [0, 0, 255, 128];
    }
    let offset = usize::from(index) * 3;
    [
        C4_DEFINITION_GAME_PALETTE[offset] << 2,
        C4_DEFINITION_GAME_PALETTE[offset + 1] << 2,
        C4_DEFINITION_GAME_PALETTE[offset + 2] << 2,
        255,
    ]
}

fn c4_definition_palette_lookup() -> &'static HashMap<[u8; 4], u8> {
    static LOOKUP: OnceLock<HashMap<[u8; 4], u8>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        let mut lookup = HashMap::with_capacity(256);
        for index in 0..=u8::MAX {
            // SurfaceAllowColor scans from zero upward, so duplicate palette
            // colors resolve to the first index (StdDDraw2.cpp:1224-1229).
            lookup
                .entry(c4_definition_palette_pixel(index))
                .or_insert(index);
        }
        lookup
    })
}

fn c4_material_definition_colors(material: &Material) -> [[u8; 4]; 3] {
    std::array::from_fn(|index| {
        let offset = index * 3;
        let transparency = material.alpha().get(index).copied().unwrap_or(0) as u8;
        [
            material.color().get(offset).copied().unwrap_or(0) as u8,
            material.color().get(offset + 1).copied().unwrap_or(0) as u8,
            material.color().get(offset + 2).copied().unwrap_or(0) as u8,
            255_u8.wrapping_sub(transparency),
        ]
    })
}

fn colorize_definition_pixels(pixels: &mut Arc<[u8]>, colors: &[[u8; 4]; 3]) {
    let lookup = c4_definition_palette_lookup();
    let mut recolored = pixels.to_vec();
    let mut changed = false;
    for pixel in recolored.chunks_exact_mut(4) {
        let key = [pixel[0], pixel[1], pixel[2], pixel[3]];
        let Some(index) = lookup.get(&key).copied().filter(|index| *index != 0) else {
            continue;
        };
        let replacement = colors[usize::from(index - 1) % colors.len()];
        if pixel != replacement {
            pixel.copy_from_slice(&replacement);
            changed = true;
        }
    }
    if changed {
        *pixels = Arc::from(recolored.into_boxed_slice());
    }
}

impl DefinitionPictureImage {
    fn from_resource(
        image: &clonk_resources::GraphicsImage,
        mask: Option<&clonk_resources::ColorByOwnerMask>,
    ) -> Self {
        let color_mask = mask.and_then(|mask| {
            if (mask.width, mask.height) != (image.width(), image.height()) {
                return None;
            }
            let channels =
                definition_color_mask_channels(&mask.pixels, image.width(), image.height())?;
            definition_color_mask_has_coverage(&mask.pixels, channels)
                .then(|| Arc::from(mask.pixels.clone().into_boxed_slice()))
        });
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.clone_pixels(),
            color_mask,
        }
    }

    fn from_sprite_rect(sprite: &DefinitionSpriteImage, rect: DefinitionRect) -> Option<Self> {
        let x = u32::try_from(rect.x).ok()?;
        let y = u32::try_from(rect.y).ok()?;
        let width = u32::try_from(rect.width).ok()?;
        let height = u32::try_from(rect.height).ok()?;
        if width == 0
            || height == 0
            || x.checked_add(width)? > sprite.width
            || y.checked_add(height)? > sprite.height
        {
            return None;
        }

        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for row in y..y + height {
            let start = ((row * sprite.width + x) * 4) as usize;
            let end = start + width as usize * 4;
            pixels.extend_from_slice(sprite.pixels.get(start..end)?);
        }
        let color_mask = sprite.color_mask.as_ref().and_then(|mask| {
            let channels = definition_color_mask_channels(mask, sprite.width, sprite.height)?;
            let mut cropped = Vec::with_capacity(width as usize * height as usize * channels);
            for row in y..y + height {
                let start = (row * sprite.width + x) as usize * channels;
                let end = start + width as usize * channels;
                cropped.extend_from_slice(mask.get(start..end)?);
            }
            Some(Arc::from(cropped.into_boxed_slice()))
        });
        Some(Self {
            width,
            height,
            pixels: Arc::from(pixels.into_boxed_slice()),
            color_mask,
        })
    }

    /// Builds a facet-sized image while clipping source coordinates to the
    /// bitmap. C4Facet permits source rectangles outside the surface; the
    /// renderer clips those pixels rather than rejecting the facet or using
    /// phase zero (`C4Def::Picture2Facet`, C4Def.cpp:1374-1378).
    fn from_sprite_rect_clipped(
        sprite: &DefinitionSpriteImage,
        rect: DefinitionRect,
    ) -> Option<Self> {
        let width = u32::try_from(rect.width).ok()?;
        let height = u32::try_from(rect.height).ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let pixel_count = usize::try_from(u64::from(width).checked_mul(u64::from(height))?).ok()?;
        let mut pixels = vec![0; pixel_count.checked_mul(4)?];
        let color_mask_channels = sprite
            .color_mask
            .as_ref()
            .and_then(|mask| definition_color_mask_channels(mask, sprite.width, sprite.height));
        let mut color_mask = color_mask_channels
            .and_then(|channels| pixel_count.checked_mul(channels))
            .map(|len| vec![0; len]);

        let left = i64::from(rect.x).max(0);
        let top = i64::from(rect.y).max(0);
        let right = (i64::from(rect.x) + i64::from(rect.width)).min(i64::from(sprite.width));
        let bottom = (i64::from(rect.y) + i64::from(rect.height)).min(i64::from(sprite.height));
        if left < right && top < bottom {
            let copy_width = usize::try_from(right - left).ok()?;
            let destination_x = usize::try_from(left - i64::from(rect.x)).ok()?;
            for source_y in top..bottom {
                let destination_y = usize::try_from(source_y - i64::from(rect.y)).ok()?;
                let source_start =
                    usize::try_from((source_y * i64::from(sprite.width) + left).checked_mul(4)?)
                        .ok()?;
                let source_end = source_start.checked_add(copy_width.checked_mul(4)?)?;
                let destination_start = destination_y
                    .checked_mul(width as usize)?
                    .checked_add(destination_x)?
                    .checked_mul(4)?;
                let destination_end = destination_start.checked_add(copy_width.checked_mul(4)?)?;
                pixels
                    .get_mut(destination_start..destination_end)?
                    .copy_from_slice(sprite.pixels.get(source_start..source_end)?);

                if let (Some(destination_mask), Some(source_mask)) =
                    (color_mask.as_mut(), sprite.color_mask.as_ref())
                {
                    let channels = color_mask_channels?;
                    let source_start = usize::try_from(source_y * i64::from(sprite.width) + left)
                        .ok()?
                        .checked_mul(channels)?;
                    let source_end = source_start.checked_add(copy_width.checked_mul(channels)?)?;
                    let destination_start = destination_y
                        .checked_mul(width as usize)?
                        .checked_add(destination_x)?;
                    let destination_start = destination_start.checked_mul(channels)?;
                    let destination_end =
                        destination_start.checked_add(copy_width.checked_mul(channels)?)?;
                    destination_mask
                        .get_mut(destination_start..destination_end)?
                        .copy_from_slice(source_mask.get(source_start..source_end)?);
                }
            }
        }

        Some(Self {
            width,
            height,
            pixels: Arc::from(pixels.into_boxed_slice()),
            color_mask: color_mask.map(|mask| Arc::from(mask.into_boxed_slice())),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn into_pixels(self) -> Arc<[u8]> {
        self.pixels
    }

    pub fn color_mask(&self) -> Option<Arc<[u8]>> {
        self.color_mask.as_ref().map(Arc::clone)
    }

    fn colorize_by_material(&mut self, colors: &[[u8; 4]; 3]) {
        colorize_definition_pixels(&mut self.pixels, colors);
    }
}

#[derive(Clone)]
pub struct DefinitionSpriteImage {
    #[doc(hidden)]
    pub width: u32,
    #[doc(hidden)]
    pub height: u32,
    #[doc(hidden)]
    pub pixels: Arc<[u8]>,
    #[doc(hidden)]
    pub color_mask: Option<Arc<[u8]>>,
}

impl DefinitionSpriteImage {
    fn from_resource(
        image: &clonk_resources::GraphicsImage,
        mask: Option<&clonk_resources::ColorByOwnerMask>,
    ) -> Self {
        let color_mask = mask.and_then(|mask| {
            if mask.width != image.width() || mask.height != image.height() {
                return None;
            }
            let channels =
                definition_color_mask_channels(&mask.pixels, image.width(), image.height())?;
            if !definition_color_mask_has_coverage(&mask.pixels, channels) {
                return None;
            }
            Some(Arc::from(mask.pixels.clone().into_boxed_slice()))
        });
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.clone_pixels(),
            color_mask,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    pub fn into_pixels(self) -> Arc<[u8]> {
        self.pixels
    }

    pub fn color_mask(&self) -> Option<Arc<[u8]>> {
        self.color_mask.as_ref().map(Arc::clone)
    }

    fn solid_mask_source_pixels(&self) -> Arc<[u8]> {
        let Some(overlay) = self
            .color_mask
            .as_ref()
            .filter(|overlay| overlay.len() == self.pixels.len())
        else {
            return Arc::clone(&self.pixels);
        };
        let mut pixels = self.pixels.to_vec();
        for (base, overlay) in pixels.chunks_exact_mut(4).zip(overlay.chunks_exact(4)) {
            // C4Surface::GetPixDw composites the owner surface before
            // IsPixTransparent samples a solid mask. With C4's inverse-alpha
            // BltAlpha helper this is a saturating sum of conventional
            // opacities, not just the base PNG's alpha.
            base[3] = base[3].saturating_add(overlay[3]);
        }
        Arc::from(pixels.into_boxed_slice())
    }

    fn colorize_by_material(&mut self, colors: &[[u8; 4]; 3]) {
        colorize_definition_pixels(&mut self.pixels, colors);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionActionFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub target_x: i32,
    pub target_y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DefinitionActionGraphics {
    pub facet: Option<DefinitionActionFacet>,
    pub directions: i32,
    pub flip_dir: Option<i32>,
    pub reverse: bool,
    pub facet_base: bool,
    pub facet_top_face: bool,
    pub facet_target_stretch: bool,
    pub length: Option<i32>,
}

/// Reserved metadata key indicating that an action-graphics map also carries
/// physical ActMap slots. Legacy/synthetic maps omit it and remain name-only.
#[doc(hidden)]
pub const PHYSICAL_ACTION_GRAPHICS_MARKER: &str = "\0lc:actmap:physical";

#[doc(hidden)]
pub fn physical_action_graphics_key(index: u32) -> String {
    format!("\0lc:actmap:{index}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionComponent {
    pub id: DefinitionId,
    pub count: i32,
}

#[derive(Clone)]
pub struct Definition {
    id: DefinitionId,
    name: String,
    /// DefCore `Version` / C4Def::rC4XVer (src/C4Def.h:190).
    version: [i32; 5],
    /// DefCore `RequireDef`; reflected as an ID-only C4IDList.
    require_defs: Vec<String>,
    /// Exact signed compiler values for DefCore fields whose gameplay
    /// projection is intentionally normalized to bool/option/nonnegative.
    def_core_reflected_ints: HashMap<String, i32>,
    /// Trimmed localized C4Def description (`C4Def::GetDesc`).
    description: Option<String>,
    /// Shared compiled script: `host_world_context()` hands clones of this
    /// `Arc` to host functions so nested script calls (Find_Func, GameCall)
    /// can execute another definition's functions mid-VM-call.
    script: Arc<ScriptEngine>,
    /// The preparsed, unlinked script owned by this definition. Relinking
    /// rebuilds `script` from this pristine copy so include/append copies
    /// and global-function links cannot accumulate.
    base_script: clonk_script::Script,
    /// The raw Script.c text — read-only presentation support and declaration
    /// ordering for C4ScriptHost `[..|Image=..]` descriptors
    /// (C4AulParse.cpp:301-380).
    script_source: String,
    includes: Vec<String>,
    /// Mirrors C4AulScript::IncludesResolved: definitions already linked in a
    /// prior resolve pass must not copy the same parent functions again when
    /// a later definition is registered.
    includes_resolved: bool,
    /// `#appendto` targets of this definition's script
    /// (C4AulScript::Appends; resolved by Engine::resolve_appends).
    appends: Vec<clonk_script::AppendTo>,
    has_construction: bool,
    has_initialize: bool,
    has_step: bool,
    action_library: ActionLibrary,
    /// Whether `C4DefScriptHost::AfterLink` populated action and TimerCall
    /// function pointers for the current linked script tree.
    callbacks_linked: bool,
    action_graphics: HashMap<String, DefinitionActionGraphics>,
    crew_member: bool,
    /// Literal signed DefCore `CrewMember` value returned by FnCrewMember.
    /// `crew_member` remains the derived nonzero gameplay capability.
    crew_member_value: i32,
    no_standard_crew: i32,
    /// DefCore `SilentCommands`: suppresses C4Command::Fail's common
    /// message/sound/ComDir-stop tail, but not command-specific callbacks.
    silent_commands: bool,
    /// DefCore `CanBeBase` (C4Def.cpp; FirstBase detection in
    /// PlaceReadyBase, C4Player.cpp:596-599).
    can_be_base: bool,
    /// The def's ClonkNames list content (C4Def.cpp:645-652,
    /// C4CFN_ClonkNames): overrides Game.Names for new crew infos.
    clonk_names: Option<String>,
    /// C4Def::fClonkNamesOwned: inherited include data must be cleared by
    /// ResetIncludeDependencies while a definition's own list survives.
    clonk_names_owned: bool,
    movement: MovementProfile,
    category: i32,
    max_user_select: i32,
    /// DefCore `BlitMode`, copied into C4Object::BlitMode at Init/reset.
    blit_mode: u32,
    /// DefCore `ColorByOwner`, used by picture-stack equality.
    color_by_owner: bool,
    color_by_material: String,
    /// DefCore `AllowPictureStack` APS_* exception bits.
    allow_picture_stack: i32,
    /// C4Def graphics scale (`C4DefCore::Scale / 100.0f`).
    graphics_scale: f32,
    ocf_base: u32,
    value: i32,
    /// DefCore `NoSell`; any nonzero value prevents SellFromBase from
    /// selecting this definition as the root sale object.
    no_sell: i32,
    /// DefCore `Rebuy` (C4Def.cpp:359): permits sold objects to introduce
    /// their definition into home-base stock.
    rebuyable: bool,
    /// DefCore `BaseAutoSell` (C4Def.cpp:457): sold automatically by a base
    /// while BASEFUNC_AutoSellContents is active.
    base_auto_sell: bool,
    mass: i32,
    move_to_range: i32,
    /// DefCore `Pathfinder` (C4Def.cpp:399): nonzero enables MoveTo path
    /// search for non-crew objects and supplies the clamped search level.
    pathfinder: i32,
    /// DefCore `NoTransferZones` (C4Def.cpp:415): exclude transfer-zone
    /// edges from this definition's MoveTo path searches.
    no_transfer_zones: i32,
    /// DefCore `NoPushEnter` (C4Def.cpp:396): any nonzero value prevents
    /// this definition from executing Enter commands.
    no_push_enter: i32,
    drag_image_picture: i32,
    picture: Option<DefinitionPicture>,
    picture_image: Option<DefinitionPictureImage>,
    /// First def portrait (C4CFN_Portraits, src/C4Components.h:88) — HUD
    /// cursor info only (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-320).
    portrait_image: Option<DefinitionPictureImage>,
    /// ColorByOwner-aware portrait surface retained for
    /// C4Game::DrawTextSpecImage portrait specifications.
    portrait_graphics_image: Option<DefinitionPictureImage>,
    /// All named `Portrait*.*` variants, keyed by lowercase portrait name.
    portrait_graphics: Vec<(String, DefinitionPictureImage)>,
    /// Def rank symbols (C4Def::pRankSymbols from Rank.png,
    /// src/C4Def.cpp:684-691) — HUD cursor info only.
    rank_symbols_image: Option<DefinitionPictureImage>,
    /// Finite localized `C4Def::pRankNames` table used by Promote. This is
    /// independent of the rank-symbol strip and may be inherited.
    rank_names: Option<RankNameTable>,
    /// `C4RankSystem::Base` paired with `rank_names`. Like the native
    /// `pRankNames` pointer, an inherited rank table carries its curve.
    rank_base: Option<i32>,
    rank_names_owned: bool,
    /// Base rank-cell count after localized extension cells are removed.
    rank_symbol_count: Option<u32>,
    /// Whether this definition loaded its own strip. Non-owned pointers are
    /// overwritten by each linked include just like C4Def's
    /// `fRankSymbolsOwned`/`IncludeDefinition` path.
    rank_symbols_owned: bool,
    sprite_image: Option<DefinitionSpriteImage>,
    sprite_variants: HashMap<String, DefinitionSpriteImage>,
    shape: Option<DefinitionRect>,
    /// C4Shape::FireTop (C4Shape.cpp:509).
    fire_top: i32,
    /// DefCore `LiftTop` (C4Def.cpp:385), used by DFA_LIFT's target-height
    /// callback gate (C4Object.cpp:5281-5286).
    lift_top: i32,
    solid_mask: Option<DefinitionTargetRect>,
    def_core_solid_mask: Option<DefinitionTargetRect>,
    /// DefCore `TopFace` (C4Def.cpp:306), drawn in the second object pass.
    top_face: Option<DefinitionTargetRect>,
    def_core_top_face: Option<DefinitionTargetRect>,
    shape_vertices: Vec<ObjectVertex>,
    /// Complete fixed C4Shape slots from DefCore. Fresh C4Object::Init copies
    /// the whole shape, not just its active VtxNum prefix.
    shape_vertex_slots: ShapeVertexBuffer,
    contact_density: i32,
    contact_function_calls: bool,
    collection_rect: Option<DefinitionRect>,
    def_core_collection_rect: Option<DefinitionRect>,
    collection_limit: i32,
    /// DefCore `Fragile`; outdoor Put must not throw these objects into a
    /// target's collection area.
    fragile: bool,
    /// Raw DefCore `Projectile`; any nonzero value lets Attack select the
    /// object from the attacker's contents.
    projectile: i32,
    explosive: i32,
    collectible: bool,
    /// DefCore `NoGet` (src/C4Def.cpp:412): omit this definition from
    /// manual get/activate menus when set to any nonzero value.
    no_get: bool,
    /// `GrabPutGet` DefCore bitfield (src/C4Def.cpp:364-373) — read by the
    /// viewport command-row presentation (C4Object::DrawCommands).
    grab_put_get: i32,
    /// DefCore `VehicleControl` (src/C4Def.cpp:398):
    /// C4D_VehicleControl_Outside=1 | C4D_VehicleControl_Inside=2, the
    /// SetCommand ControlCommand overloads (C4Object.cpp:3944-3969).
    vehicle_control: i32,
    constructable: bool,
    construction_offset: i32,
    stretch_growth: bool,
    /// DefCore `Oversize`: DoCon may grow beyond FullCon.
    oversize: bool,
    /// `Placement=` (C4Def.cpp:312): 0 surface, 1 liquid, 2 air.
    placement: i32,
    /// `Growth=` (C4Def.cpp:358): PlaceVegetation's random-growth gate.
    growth: i32,
    basement: i32,
    rotateable: i32,
    border_bound: i32,
    upright_attach: i32,
    /// RotatedSolidmasks (C4Def.cpp:414): the solid mask stays put while
    /// the object is rotated (C4Object::UpdateSolidMask gate,
    /// C4Object.cpp:5655) and bakes through the rotated branch of
    /// C4SolidMask::Put (C4SolidMask.cpp:108-174).
    rotated_solid_masks: bool,
    /// DefCore `AutoContextMenu` (C4Def.cpp:416): entering this container
    /// may automatically open its context menu (C4Object.cpp:2049-2056).
    auto_context_menu: bool,
    needed_gfx_mode: i32,
    no_component_mass: bool,
    /// NoStabilize=1 opts out of the small-tilt upright snap
    /// (C4Object::Stabilize, C4Movement.cpp:491).
    no_stabilize: bool,
    hide_hud_bars: i32,
    hide_hud_elements: i32,
    /// DefCore Timer= interval in frames (default 35, C4Def.cpp:298).
    timer: i32,
    /// DefCore TimerCall= function name (C4Def.cpp:299), fired every
    /// Timer-th Execute per object (C4Object.cpp:1085-1091). None when
    /// the def names no callback (C++ links to nullptr).
    timer_call: Option<String>,
    /// Runtime-only `C4Def::TimerCall` function pointer cache.
    timer_call_link: ScriptCallbackLink,
    /// Runtime-only `C4DefScriptHost::SFn_ControlTransfer` pointer cache.
    control_transfer_link: ScriptCallbackLink,
    components: Vec<DefinitionComponent>,
    line_connect: u32,
    /// ContactIncinerate=N: 1-in-N contact-fire chance (0 = not inflammable).
    contact_incinerate: i32,
    /// BlastIncinerate=N: incinerate once accumulated Damage reaches N after
    /// a blast (C4Object::Blast, C4Object.cpp:1421-1423); 0 = off.
    blast_incinerate: i32,
    /// ContainBlast=1: shields contents from explosions (the DoExplosion
    /// container walk, C4Effect.cpp:884).
    contain_blast: i32,
    /// Any nonzero `ClosedContainer` prevents contained objects from
    /// inheriting this object's cached `InMat`.
    closed_container: i32,
    /// HorizontalFix=1 (C4Def::NoHorizontalMove, C4Def.cpp:383): exempt
    /// from shockwave flings.
    no_horizontal_move: i32,
    no_burn_decay: bool,
    no_burn_damage: bool,
    /// NoBreath=1: exempt from the ExecLife breathing check (C4Object.cpp:880).
    no_breath: bool,
    temporary_crew: i32,
    smoke_rate: i32,
    /// `Float` DefCore value (C4Def.cpp:379): IsInLiquidCheck buoyancy line
    /// offset (C4Object.cpp:5609-5612).
    float_line: i32,
    line: i32,
    line_intersect: i32,
    /// Grab DefCore value: 0 none, 1 grab+push, 2 grab-only (C4Object.cpp:1763).
    grab: i32,
    burn_turn_to: Option<String>,
    /// DefCore `ConstructTo` / C4Def::BuildTurnTo: successful Build ticks
    /// change the construction target after DoCon.
    build_turn_to: Option<String>,
    incomplete_activity: bool,
    /// `Exclusive` DefCore flag (C4Def.cpp:313): OCF_Exclusive — no action
    /// through this, no construction in front of it (SetOCF,
    /// C4Object.cpp:581-583).
    exclusive: bool,
    /// `Edible` DefCore flag (C4Def.cpp:355): OCF_Edible (SetOCF,
    /// C4Object.cpp:630-632).
    edible: bool,
    /// `Prey` DefCore flag (C4Def.cpp:354): OCF_Prey while alive (SetOCF,
    /// C4Object.cpp:615-618).
    prey: bool,
    /// `AttractLightning` DefCore flag (C4Def.cpp:391): OCF_AttractLightning
    /// at FullCon (SetOCF, C4Object.cpp:623-626).
    attract_lightning: bool,
    /// `Entrance` DefCore rect (C4Def.cpp:309): the enter/activate area
    /// (OCF_Entrance, SetOCF C4Object.cpp:584-587; area check in
    /// GetOCFForPos, C4Object.cpp:1149-1153).
    entrance_rect: Option<DefinitionRect>,
    /// `RotatedEntrance` (C4Def.cpp:377): 0 = upright only, 1 = any
    /// rotation, N = rotations up to N degrees (SetOCF, C4Object.cpp:586).
    rotated_entrance: i32,
    /// `NoFight` DefCore flag (C4Def.cpp:413): suppresses OCF_FightReady
    /// (SetOCF, C4Object.cpp:606-610).
    no_fight: bool,
    /// `Chop` DefCore flag (C4Def::Chopable, C4Def.cpp:378): OCF_Chop
    /// candidate (SetOCF, C4Object.cpp:570-575).
    chopable: bool,
    /// The [Physical] DefCore section (C4Def::Physical).
    physical: PhysicalInfo,
    /// Real C4Script content gets the C++ callback arguments — no parameters
    /// for StartCall/EndCall/PhaseCall, the last phase for AbortCall
    /// (C4Object.cpp:4154-4182) — while synthetic command-DSL fixtures keep
    /// the additive (state, action) convention.
    c4_callback_args: bool,
    /// SolidMask alpha pixels extracted from the sprite once at set time —
    /// the movement loop reads them per masked object per moving object per
    /// tick, and re-scanning the sprite there dominated the loop.
    solid_mask_pixels: SolidMaskPixels,
    /// Per-active-graphics/override-rect pixel cache (Objects.txt SolidMask=
    /// picks its own sprite region, while SetGraphics picks another bitmap).
    solid_mask_rect_cache:
        std::cell::RefCell<HashMap<(Option<String>, i32, i32, i32, i32), SolidMaskPixels>>,
}

/// Precomputed solid-mask pixel data for `solid_masks_for_movement`.
#[derive(Debug, Clone, Default)]
enum SolidMaskPixels {
    /// No sprite image: the whole mask rect is solid.
    #[default]
    Rectangle,
    /// Per-pixel alpha mask (1 = solid).
    Alpha(Arc<Vec<u8>>),
    /// Invalid rect or unavailable named bitmap: ignore the mask entirely.
    OutOfBounds,
}

#[derive(Debug, Clone, Copy)]
enum ActionCallbackKind {
    Start,
    End,
    Phase,
    Abort,
}

impl ActionCallbackKind {
    fn context(self) -> &'static str {
        match self {
            ActionCallbackKind::Start => "action start",
            ActionCallbackKind::End => "action end",
            ActionCallbackKind::Phase => "action phase",
            ActionCallbackKind::Abort => "action abort",
        }
    }
}

/// Context labels used by actual C4AulScript::DirectExec call sites. Several
/// Rust compatibility adapters reuse the expression evaluator for native C++
/// calls; those must not appear in C4Aul's DirectExec trace/profiler totals.
fn is_cpp_direct_exec_context(context: &str) -> bool {
    matches!(
        context,
        "console script" | "internal script" | "MenuCommand"
    )
}

impl Definition {
    pub fn from_script(
        id: impl Into<String>,
        name: impl Into<String>,
        source: &str,
    ) -> Result<Self, EngineError> {
        let id = id.into();
        let name = name.into();

        // Compile the script to extract includes before adding to engine
        let compiled_script =
            clonk_script::Script::compile_c4_string(source).map_err(|parse_error| {
                EngineError::Script {
                    definition: id.clone(),
                    function: "load".to_string(),
                    source: parse_error.into(),
                    recovery: None,
                }
            })?;
        for diagnostic in compiled_script.parse_diagnostics() {
            tracing::warn!(
                definition = %id,
                %diagnostic,
                "definition script parse error quarantined; continuing like C++"
            );
        }

        let includes = compiled_script.includes().to_vec();
        let appends = compiled_script.appends().to_vec();

        let mut script = ScriptEngine::new();
        script.set_script_name(id.clone());
        script.set_definition_name(name.clone());
        script.set_definition_context(true);
        script.add_script(compiled_script.clone());
        compat::register_host_functions(&mut script);
        // Synthetic command-DSL fixtures historically declare these as
        // `global func`; real C4 content switches to local-only callback
        // gates in set_c4_callback_convention below.
        let has_construction = script.has_function("Construction");
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        let base_auto_sell = id.eq_ignore_ascii_case("GOLD");
        // The engine is single-threaded; Arc is shared ownership for host
        // contexts, not cross-thread transport (the script engine holds the
        // Rc-based GlobalNamed table).
        #[allow(clippy::arc_with_non_send_sync)]
        Ok(Self {
            id,
            name,
            version: DEFAULT_DEFINITION_VERSION,
            require_defs: Vec::new(),
            def_core_reflected_ints: HashMap::new(),
            description: None,
            script: Arc::new(script),
            base_script: compiled_script,
            script_source: source.to_string(),
            includes,
            includes_resolved: false,
            appends,
            has_construction,
            has_initialize,
            has_step,
            action_library: ActionLibrary::default(),
            callbacks_linked: false,
            action_graphics: HashMap::new(),
            crew_member: false,
            crew_member_value: 0,
            no_standard_crew: 0,
            silent_commands: false,
            can_be_base: false,
            clonk_names: None,
            clonk_names_owned: false,
            movement: MovementProfile::default(),
            category: DEFAULT_CATEGORY,
            max_user_select: 0,
            blit_mode: 0,
            color_by_owner: false,
            color_by_material: String::new(),
            allow_picture_stack: 0,
            graphics_scale: 1.0,
            ocf_base: OCF_NORMAL,
            value: 0,
            no_sell: 0,
            rebuyable: false,
            base_auto_sell,
            mass: 0,
            move_to_range: 0,
            pathfinder: 0,
            no_transfer_zones: 0,
            no_push_enter: 0,
            drag_image_picture: 0,
            picture: None,
            picture_image: None,
            portrait_image: None,
            portrait_graphics_image: None,
            portrait_graphics: Vec::new(),
            rank_symbols_image: None,
            rank_names: None,
            rank_base: None,
            rank_names_owned: false,
            rank_symbol_count: None,
            rank_symbols_owned: false,
            sprite_image: None,
            sprite_variants: HashMap::new(),
            shape: None,
            fire_top: 0,
            lift_top: 0,
            solid_mask: None,
            def_core_solid_mask: None,
            top_face: None,
            def_core_top_face: None,
            shape_vertices: Vec::new(),
            shape_vertex_slots: ShapeVertexBuffer::default(),
            contact_density: CONTACT_DENSITY_SOLID,
            contact_function_calls: false,
            collection_rect: None,
            def_core_collection_rect: None,
            collection_limit: 0,
            fragile: false,
            projectile: 0,
            explosive: 0,
            collectible: false,
            no_get: false,
            grab_put_get: 0,
            vehicle_control: 0,
            constructable: false,
            construction_offset: 0,
            stretch_growth: false,
            oversize: false,
            placement: 0,
            growth: 0,
            basement: 0,
            rotateable: 0,
            border_bound: 0,
            upright_attach: 0,
            rotated_solid_masks: false,
            auto_context_menu: false,
            needed_gfx_mode: 0,
            no_component_mass: false,
            no_stabilize: false,
            hide_hud_bars: 0,
            hide_hud_elements: 0,
            timer: 35,
            timer_call: None,
            timer_call_link: ScriptCallbackLink::default(),
            control_transfer_link: ScriptCallbackLink::default(),
            components: Vec::new(),
            line_connect: 0,
            contact_incinerate: 0,
            blast_incinerate: 0,
            contain_blast: 0,
            closed_container: 0,
            no_horizontal_move: 0,
            no_burn_decay: false,
            no_breath: false,
            temporary_crew: 0,
            smoke_rate: 100,
            float_line: 0,
            line: 0,
            line_intersect: 0,
            grab: 0,
            no_burn_damage: false,
            burn_turn_to: None,
            build_turn_to: None,
            incomplete_activity: false,
            exclusive: false,
            edible: false,
            prey: false,
            attract_lightning: false,
            entrance_rect: None,
            rotated_entrance: 0,
            no_fight: false,
            chopable: false,
            physical: PhysicalInfo::default(),
            c4_callback_args: false,
            solid_mask_pixels: SolidMaskPixels::default(),
            solid_mask_rect_cache: std::cell::RefCell::new(HashMap::new()),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn set_name(&mut self, name: String) {
        Arc::make_mut(&mut self.script).set_definition_name(name.clone());
        self.name = name;
    }

    pub fn version(&self) -> [i32; 5] {
        self.version
    }

    pub fn set_version(&mut self, version: [i32; 5]) {
        self.version = if definition_version_at_least(version, [4, 0, 0, 0]) {
            version
        } else {
            DEFAULT_DEFINITION_VERSION
        };
    }

    pub(crate) fn version_at_least(&self, required: [i32; 4]) -> bool {
        definition_version_at_least(self.version, required)
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Effective target `Context*` functions with their C4Aul description
    /// metadata. C++ enumerates the linked script list from `FuncL` backward
    /// and parses caption/Image/Condition/Desc from the raw description
    /// segments (C4Aul.cpp:357-379; C4AulParse.cpp:309-380).
    pub(crate) fn script_context_functions(&self) -> Vec<ScriptContextFunction> {
        self.script_menu_functions("Context")
            .into_iter()
            .filter(|function| function.has_description)
            .collect()
    }

    /// Effective script functions whose names start with `prefix`, in the
    /// reverse declaration order used by C4AulScript::GetSFunc(index,
    /// prefix). Unlike the public Context-menu projection above, native
    /// AddContextFunctions also enumerates functions with no description.
    pub(crate) fn script_menu_functions(&self, prefix: &str) -> Vec<ScriptContextFunction> {
        self.script
            .local_functions_in_get_sfunc_order()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(_, function)| {
                let mut metadata = script_context_function_metadata(function);
                if metadata.condition.as_ref().is_some_and(|condition| {
                    self.script.resolve_function(condition, true).is_none()
                }) {
                    // C4Aul stores the resolved condition pointer in the
                    // annotation. An unresolved name is therefore equivalent
                    // to omitting Condition, not a deferred failing call.
                    metadata.condition = None;
                }
                metadata
            })
            .collect()
    }

    pub(crate) fn script_menu_function(&self, name: &str) -> Option<ScriptContextFunction> {
        self.script.resolve_function(name, false).map(|resolution| {
            let mut metadata = script_context_function_metadata(resolution.function.as_ref());
            if metadata
                .condition
                .as_ref()
                .is_some_and(|condition| self.script.resolve_function(condition, true).is_none())
            {
                metadata.condition = None;
            }
            metadata
        })
    }

    pub fn set_description(&mut self, description: Option<String>) {
        self.description = description.filter(|text| !text.is_empty());
    }

    fn set_script_name(&mut self, script_name: impl Into<String>) {
        Arc::make_mut(&mut self.script).set_script_name(script_name);
    }

    fn set_game_script_name(&mut self, script_name: impl Into<String>) {
        Arc::make_mut(&mut self.script).set_game_script_name(script_name);
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.script.has_function(name)
    }

    pub fn includes(&self) -> &[String] {
        &self.includes
    }

    pub fn appends(&self) -> &[clonk_script::AppendTo] {
        &self.appends
    }

    /// Recomputes the lifecycle-callback gates after script linking
    /// (appends/includes can introduce Construction/Initialize/Step).
    fn refresh_script_flags(&mut self) {
        let (has_construction, has_initialize, has_step) = if self.c4_callback_args {
            (
                self.script.has_local_function("Construction"),
                self.script.has_local_function("Initialize"),
                self.script.has_local_function("Step"),
            )
        } else {
            (
                self.script.has_function("Construction"),
                self.script.has_function("Initialize"),
                self.script.has_function("Step"),
            )
        };
        self.has_construction = has_construction;
        self.has_initialize = has_initialize;
        self.has_step = has_step;
    }

    fn mark_callbacks_unlinked(&mut self) {
        self.callbacks_linked = false;
        self.action_library.reset_callback_links();
        self.timer_call_link.reset();
        self.control_transfer_link.reset();
    }

    /// Cache ActMap, TimerCall and ControlTransfer functions after
    /// appends/includes are final. C++'s two GetSFunc layers each consume
    /// one leading failsafe marker.
    fn link_callbacks(&mut self) {
        if self.callbacks_linked {
            return;
        }

        let script = Arc::clone(&self.script);
        let definition = self.id.clone();
        let resolve = |configured: &str| {
            if configured.is_empty() {
                return None;
            }
            let function_name = configured.strip_prefix('~').unwrap_or(configured);
            let function_name = function_name.strip_prefix('~').unwrap_or(function_name);
            script
                .resolve_function(function_name, false)
                .map(|resolution| ScriptCallbackTarget::linked(function_name, resolution))
        };

        self.action_library
            .link_callbacks(|action, slot, configured| {
                let target = resolve(configured);
                if target.is_none() && !configured.is_empty() {
                    tracing::warn!(
                        definition = %definition,
                        "Error getting Action {}: {} function '{}'",
                        action,
                        slot,
                        configured
                    );
                }
                target
            });

        let timer_target = self.timer_call.as_deref().and_then(|configured| {
            let target = resolve(configured);
            if target.is_none() && !configured.is_empty() {
                tracing::warn!(
                    definition = %definition,
                    "Error getting TimerCall function '{}'",
                    configured
                );
            }
            target
        });
        self.timer_call_link.set_linked(timer_target);
        self.control_transfer_link
            .set_linked(resolve("~ControlTransfer"));
        self.callbacks_linked = true;
    }

    /// Restores the host to its own preparsed functions. C++ UnLink deletes
    /// linked include/append copies but retains original script functions and
    /// the engine-owned global value cells.
    fn reset_script_links(&mut self) {
        self.mark_callbacks_unlinked();
        Arc::make_mut(&mut self.script).replace_script_deferred(self.base_script.clone(), false);
        self.includes_resolved = false;
        if !self.rank_names_owned {
            self.rank_names = None;
            self.rank_base = None;
        }
        if !self.clonk_names_owned {
            self.clonk_names = None;
        }
        if !self.rank_symbols_owned {
            self.rank_symbols_image = None;
            self.rank_symbol_count = None;
        }
        self.refresh_script_flags();
    }

    fn replace_base_script(&mut self, source: &str, script: clonk_script::Script) {
        self.mark_callbacks_unlinked();
        self.includes = script.includes().to_vec();
        self.appends = script.appends().to_vec();
        self.script_source = source.to_owned();
        self.base_script = script;
        self.includes_resolved = false;
    }

    fn include_definition_metadata(&mut self, parent: &Definition) {
        if !self.clonk_names_owned {
            self.clonk_names = parent.clonk_names.clone();
        }
        if !self.rank_names_owned {
            self.rank_names = parent.rank_names.clone();
            self.rank_base = parent.rank_base;
        }
        if !self.rank_symbols_owned {
            self.rank_symbols_image = parent.rank_symbols_image.clone();
            self.rank_symbol_count = parent.rank_symbol_count;
        }
    }

    pub fn merge_from(&mut self, parent: &Definition) {
        self.mark_callbacks_unlinked();
        Arc::make_mut(&mut self.script).merge_from(&parent.script);
        self.include_definition_metadata(parent);
        self.refresh_script_flags();
    }

    pub fn function_count(&self) -> usize {
        self.script.function_count()
    }

    /// Distinct function roots plus all inherited overload nodes. Unlike
    /// [`Self::function_count`], this detects duplicate link copies.
    pub fn linked_function_count(&self) -> usize {
        self.script.linked_function_count()
    }

    pub fn from_resource(resource: &ResourceDefinitionData) -> Result<Self, EngineError> {
        let name = resource
            .core
            .name
            .clone()
            .unwrap_or_else(|| "Undefined".to_string());
        let mut definition =
            Definition::from_script(resource.core.id.clone(), name, resource.script.combined())?;
        definition.description = resource.description().map(str::to_owned);
        definition.set_clonk_names(resource.clonk_names.clone());
        // Real content gets the C++ callback arguments (no parameters;
        // AbortCall gets the last phase — C4Object.cpp:4154-4182).
        definition.set_c4_callback_convention(true);
        definition.set_version(resource.core.version);
        definition.require_defs = resource.core.require_defs.clone();

        if let Some(action_map) = &resource.action_map {
            let mut specs = HashMap::new();
            let mut physical_actions = Vec::with_capacity(action_map.actions.len());
            let mut visuals = HashMap::new();
            let mut reflections = HashMap::new();
            visuals.insert(
                PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
                DefinitionActionGraphics::default(),
            );
            for (index, (action_name, action_def)) in action_map.actions.iter().enumerate() {
                let (spec, graphics) = Self::convert_action_definition(action_def);
                physical_actions.push((action_name.clone(), spec.clone()));
                // SetActionByName and FnGetActMapVal both scan the physical
                // ActMap forward, so the first duplicate name wins.
                specs.entry(action_name.clone()).or_insert(spec);
                visuals
                    .entry(action_name.clone())
                    .or_insert_with(|| graphics.clone());
                visuals.insert(
                    physical_action_graphics_key(index.min(u32::MAX as usize) as u32),
                    graphics,
                );
                reflections.entry(action_name.clone()).or_insert_with(|| {
                    crate::action::C4ActionReflection::from_resource(action_name, action_def)
                });
            }
            // Real ActMaps carry no default action: C++ objects start
            // ActIdle (C4Object::Init). Only DSL manifests set one.
            let default_action = action_map.default_action.clone();
            definition.configure_actions(default_action.clone(), specs);
            definition.configure_physical_actions(physical_actions);
            definition.configure_action_reflections(reflections);
            definition.configure_action_graphics(visuals);
        }

        definition.set_crew_member_value(resource.core.crew_member);
        definition.no_standard_crew = resource.core.no_standard_crew;
        definition.set_silent_commands(resource.core.silent_commands);
        definition.set_category(resource.core.category);
        definition.max_user_select = resource.core.max_user_select;
        definition.set_blit_mode(resource.core.blit_mode);
        definition.set_color_by_owner(resource.core.color_by_owner);
        definition.color_by_material = resource.core.color_by_material.clone();
        definition.set_allow_picture_stack(resource.core.allow_picture_stack);
        definition.set_graphics_scale(resource.core.graphics_scale as f32 / 100.0);
        definition.set_value(resource.core.value);
        definition.set_no_sell(resource.core.no_sell);
        definition.set_rebuyable(resource.core.rebuyable);
        definition.set_base_auto_sell(resource.core.base_auto_sell);
        definition.set_mass(resource.core.mass);
        definition.set_picture(resource.core.picture.map(DefinitionPicture::from));
        definition.set_solid_mask(resource.core.solid_mask.map(DefinitionTargetRect::from));
        definition.set_top_face(resource.core.top_face.map(DefinitionTargetRect::from));
        if let Some(image) = resource.picture_image.as_ref() {
            definition.set_picture_image(Some(DefinitionPictureImage::from_resource(
                image,
                resource.picture_color_by_owner_mask.as_ref(),
            )));
        }
        if let Some(image) = resource.portrait_image.as_ref() {
            definition.set_portrait_image(Some(DefinitionPictureImage::from_resource(image, None)));
        }
        if let Some(image) = resource.portrait_graphics_image.as_ref() {
            definition.set_portrait_graphics_image(Some(DefinitionPictureImage::from_resource(
                image,
                resource.portrait_color_by_owner_mask.as_ref(),
            )));
        }
        definition.set_portrait_graphics(
            resource
                .portrait_graphics
                .iter()
                .map(|portrait| {
                    (
                        portrait.name.clone(),
                        DefinitionPictureImage::from_resource(
                            &portrait.image,
                            portrait.color_by_owner_mask.as_ref(),
                        ),
                    )
                })
                .collect(),
        );
        if let Some(image) = resource.rank_symbols_image.as_ref() {
            definition
                .set_rank_symbols_image(Some(DefinitionPictureImage::from_resource(image, None)));
        }
        definition.set_rank_name_table(resource.rank_names.clone(), resource.rank_base);
        definition.set_rank_symbol_count(resource.rank_symbol_count);
        if let Some(image) = resource.graphics_image.as_ref() {
            let mask = resource.color_by_owner_mask.as_ref();
            definition.set_sprite_image(Some(DefinitionSpriteImage::from_resource(image, mask)));
        }
        definition.validate_base_graphics_rects();
        if !resource.additional_graphics.is_empty() {
            let mut variants = HashMap::with_capacity(resource.additional_graphics.len());
            for (key, variant) in &resource.additional_graphics {
                let mask = variant.color_by_owner_mask.as_ref();
                variants.insert(
                    key.clone(),
                    DefinitionSpriteImage::from_resource(&variant.image, mask),
                );
            }
            definition.set_sprite_variants(variants);
        }
        definition.set_shape_rect(resource.core.shape.map(DefinitionRect::from));
        definition.set_fire_top(resource.core.fire_top);
        definition.set_lift_top(resource.core.lift_top);
        definition.set_shape_vertex_slots(
            resource.core.vertices.len(),
            &resource
                .core
                .vertex_slots
                .iter()
                .map(|vertex| {
                    ObjectVertex::new(vertex.x, vertex.y)
                        .with_cnat(vertex.cnat)
                        .with_friction(vertex.friction)
                })
                .collect::<Vec<_>>(),
        );
        definition.set_contact_density(resource.core.contact_density);
        definition.set_contact_function_calls(resource.core.contact_function_calls);
        definition.set_collection_rect(resource.core.collection.map(DefinitionRect::from));
        definition.set_collection_limit(resource.core.collection_limit);
        definition.set_fragile(resource.core.fragile);
        definition.set_projectile(resource.core.projectile);
        definition.explosive = resource.core.explosive;
        definition.set_fire_properties(
            resource.core.contact_incinerate,
            resource.core.no_burn_decay,
            resource.core.no_burn_damage,
        );
        definition.set_blast_incinerate(resource.core.blast_incinerate);
        definition.set_contain_blast(resource.core.contain_blast);
        definition.set_closed_container(resource.core.closed_container);
        definition.set_no_horizontal_move(resource.core.no_horizontal_move);
        definition.set_burn_turn_to(resource.core.burn_turn_to.clone());
        definition.set_build_turn_to(resource.core.build_turn_to.clone());
        definition.set_incomplete_activity(resource.core.incomplete_activity);
        definition.set_no_breath(resource.core.no_breath);
        definition.temporary_crew = resource.core.temporary_crew;
        definition.smoke_rate = resource.core.smoke_rate;
        definition.set_grab(resource.core.grab);
        definition.set_move_to_range(resource.core.move_to_range);
        definition.set_pathfinder(resource.core.pathfinder);
        definition.set_no_transfer_zones(resource.core.no_transfer_zones);
        definition.set_no_push_enter(resource.core.no_push_enter);
        definition.drag_image_picture = resource.core.drag_image_picture;
        definition.float_line = resource.core.float_line;
        definition.set_line(resource.core.line);
        definition.set_line_intersect(resource.core.line_intersect);
        definition.set_physical(resource.core.physical);
        definition.set_collectible(resource.core.collectible);
        definition.set_no_get(resource.core.no_get != 0);
        definition.set_grab_put_get(resource.core.grab_put_get);
        definition.set_vehicle_control(resource.core.vehicle_control);
        definition.set_constructable(resource.core.constructable);
        definition.set_can_be_base(resource.core.can_be_base);
        definition.set_construction_offset(resource.core.con_size_off);
        definition.set_stretch_growth(resource.core.stretch_growth);
        definition.set_oversize(resource.core.oversize);
        definition.set_placement(resource.core.placement);
        definition.set_growth(resource.core.growth);
        definition.set_basement(resource.core.basement);
        definition.set_rotateable(resource.core.rotateable);
        definition.set_border_bound(resource.core.border_bound);
        definition.set_upright_attach(resource.core.upright_attach);
        definition.set_rotated_solid_masks(resource.core.rotated_solid_masks);
        definition.set_auto_context_menu(resource.core.auto_context_menu);
        definition.needed_gfx_mode = resource.core.needed_gfx_mode;
        definition.set_no_component_mass(resource.core.no_component_mass);
        definition.set_no_stabilize(resource.core.no_stabilize);
        definition.hide_hud_bars = resource.core.hide_hud_bars;
        definition.hide_hud_elements = resource.core.hide_hud_elements;
        definition.set_timer(resource.core.timer);
        definition.set_timer_call(resource.core.timer_call.clone());
        if !resource.core.components.is_empty() {
            let components = resource
                .core
                .components
                .iter()
                .map(|component| DefinitionComponent {
                    id: component.id.clone(),
                    count: component.count,
                })
                .collect();
            definition.set_components(components);
        }
        definition.set_line_connect(resource.core.line_connect);
        definition.set_exclusive(resource.core.exclusive);
        definition.set_edible(resource.core.edible);
        definition.set_prey(resource.core.prey);
        definition.set_attract_lightning(resource.core.attract_lightning);
        definition.set_entrance_rect(resource.core.entrance.map(DefinitionRect::from));
        definition.set_rotated_entrance(resource.core.rotated_entrance);
        definition.set_no_fight(resource.core.no_fight);
        definition.set_chopable(resource.core.chopable);
        definition.def_core_reflected_ints = resource.core.reflected_ints.clone();
        Ok(definition)
    }

    fn convert_action_definition(
        action: &ResourceActionDefinition,
    ) -> (ActionSpec, DefinitionActionGraphics) {
        let mut spec = ActionSpec::default();
        if let Some(procedure) = action.procedure.as_deref().and_then(|procedure| {
            clonk_resources::definition::PROCEDURE_NAMES
                .iter()
                .find(|candidate| **candidate == procedure)
        }) {
            spec = spec.with_procedure(*procedure);
        }
        if let Some(length) = action.length {
            spec = spec.with_length(length);
        }
        if let Some(next) = &action.next_action {
            spec = spec.with_next(next.clone());
        }
        spec = spec.with_next_index(action.next_action_index);
        if let Some(delay) = action.delay {
            spec = spec.with_delay(delay);
        }
        if let Some(step) = action.step {
            spec = spec.with_step(step);
        }
        if let Some(phase) = &action.phase_call {
            spec = spec.with_phase_call(phase.clone());
        }
        if let Some(start) = &action.start_call {
            spec = spec.with_start_call(start.clone());
        }
        if let Some(end) = &action.end_call {
            spec = spec.with_end_call(end.clone());
        }
        if let Some(abort) = &action.abort_call {
            spec = spec.with_abort_call(abort.clone());
        }
        if action.no_other_action {
            spec = spec.with_no_other_action(true);
        }
        if action.disabled {
            spec = spec.with_disabled(true);
        }
        if action.energy_usage != 0 {
            spec = spec.with_energy_usage(action.energy_usage);
        }
        if let Some(in_liquid_action) = &action.in_liquid_action {
            spec = spec.with_in_liquid_action(in_liquid_action.clone());
        }
        if let Some(directions) = action.directions {
            spec = spec.with_directions(directions);
        }
        if let Some(turn_action) = &action.turn_action {
            spec = spec.with_turn_action(turn_action.clone());
        }
        if let Some(dig_free) = action.dig_free {
            spec = spec.with_dig_free(dig_free);
        }
        if action.attach != 0 {
            spec = spec.with_attach(action.attach);
        }
        let mut graphics = DefinitionActionGraphics::default();
        graphics.length = action.length;
        graphics.directions = action.directions.unwrap_or(1);
        graphics.flip_dir = action.flip_dir;
        graphics.reverse = action.reverse;
        graphics.facet_base = action.facet_base;
        graphics.facet_top_face = action.facet_top_face;
        graphics.facet_target_stretch = action.facet_target_stretch;
        graphics.facet = action.facet.as_ref().map(Self::convert_action_facet);
        (spec, graphics)
    }

    fn convert_action_facet(facet: &ResourceActionFacet) -> DefinitionActionFacet {
        DefinitionActionFacet {
            x: facet.x,
            y: facet.y,
            width: facet.width,
            height: facet.height,
            target_x: facet.target_x,
            target_y: facet.target_y,
        }
    }

    pub fn set_debugger_hooks(&mut self, hooks: DebuggerHooks) {
        Arc::make_mut(&mut self.script).set_debugger_hooks(hooks);
    }

    /// Switch this definition's engine callbacks to the C++ argument
    /// convention (C4Object.cpp:4154-4182). Real content loaded from
    /// resources runs this way; synthetic command-DSL fixtures do not.
    pub fn set_c4_callback_convention(&mut self, enabled: bool) {
        self.c4_callback_args = enabled;
        self.refresh_script_flags();
    }

    /// Shared handle to the compiled script for nested script calls
    /// (Find_Func/GameCall targets resolve functions on the target object's
    /// own definition script, C4Aul.cpp:130-148).
    pub(crate) fn script_arc(&self) -> Arc<ScriptEngine> {
        Arc::clone(&self.script)
    }

    /// Shares the System.c4g global-function table into this script host.
    pub(crate) fn set_global_functions(
        &mut self,
        functions: Option<Arc<HashMap<String, clonk_script::Function>>>,
    ) {
        Arc::make_mut(&mut self.script).set_global_functions(functions);
    }

    pub fn configure_actions(
        &mut self,
        default_action: Option<String>,
        specs: HashMap<String, ActionSpec>,
    ) {
        self.action_library = ActionLibrary::new(default_action, specs);
        self.mark_callbacks_unlinked();
    }

    pub(crate) fn configure_physical_actions(&mut self, actions: Vec<(String, ActionSpec)>) {
        self.action_library.set_physical_actions(actions);
        self.mark_callbacks_unlinked();
    }

    pub(crate) fn configure_action_reflections(
        &mut self,
        reflections: HashMap<String, action::C4ActionReflection>,
    ) {
        self.action_library.set_reflections(reflections);
    }

    pub fn configure_action_graphics(
        &mut self,
        graphics: HashMap<String, DefinitionActionGraphics>,
    ) {
        self.action_graphics = graphics;
    }

    pub fn action_library(&self) -> &ActionLibrary {
        &self.action_library
    }

    fn shared_action_library(&self, world: &HostWorldContext) -> SharedActionLibrary {
        world
            .definition_metadata(self.id.as_str())
            .map(|metadata| metadata.action_library.clone())
            .unwrap_or_else(|| self.action_library.clone().into())
    }

    pub fn action_graphics(&self) -> &HashMap<String, DefinitionActionGraphics> {
        &self.action_graphics
    }

    pub fn graphics_for_action(&self, action: &str) -> Option<&DefinitionActionGraphics> {
        self.action_graphics.get(action)
    }

    pub fn default_action_state(&self) -> ActionState {
        ActionState::new(self.action_library.default_action())
    }

    pub fn is_crew(&self) -> bool {
        self.crew_member
    }

    pub fn crew_member_value(&self) -> i32 {
        self.crew_member_value
    }

    pub fn can_be_base(&self) -> bool {
        self.can_be_base
    }

    pub fn set_can_be_base(&mut self, can_be_base: bool) {
        self.can_be_base = can_be_base;
    }

    pub fn clonk_names(&self) -> Option<&str> {
        self.clonk_names.as_deref()
    }

    pub fn set_clonk_names(&mut self, clonk_names: Option<String>) {
        self.clonk_names_owned = clonk_names.is_some();
        self.clonk_names = clonk_names;
    }

    pub fn set_crew_member(&mut self, crew_member: bool) {
        self.crew_member = crew_member;
        self.crew_member_value = i32::from(crew_member);
    }

    /// Preserve C4DefCore's raw signed integer while deriving the boolean
    /// capability used by OCF and player-crew behavior.
    pub fn set_crew_member_value(&mut self, crew_member: i32) {
        self.crew_member = crew_member != 0;
        self.crew_member_value = crew_member;
    }

    pub fn silent_commands(&self) -> bool {
        self.silent_commands
    }

    pub fn set_silent_commands(&mut self, silent_commands: bool) {
        self.silent_commands = silent_commands;
    }

    pub fn movement_profile(&self) -> MovementProfile {
        self.movement
    }

    pub fn set_movement_profile(&mut self, movement: MovementProfile) {
        self.movement = movement;
    }

    pub fn category(&self) -> i32 {
        self.category
    }

    pub fn set_category(&mut self, category: i32) {
        self.category = normalize_category(category, DEFAULT_CATEGORY);
    }

    pub fn blit_mode(&self) -> u32 {
        self.blit_mode
    }

    pub fn set_blit_mode(&mut self, blit_mode: u32) {
        self.blit_mode = blit_mode;
    }

    pub fn color_by_owner(&self) -> bool {
        self.color_by_owner
    }

    pub fn set_color_by_owner(&mut self, color_by_owner: bool) {
        self.color_by_owner = color_by_owner;
    }

    pub fn allow_picture_stack(&self) -> i32 {
        self.allow_picture_stack
    }

    pub fn set_allow_picture_stack(&mut self, allow_picture_stack: i32) {
        self.allow_picture_stack = allow_picture_stack;
    }

    pub fn graphics_scale(&self) -> f32 {
        self.graphics_scale
    }

    pub fn set_graphics_scale(&mut self, graphics_scale: f32) {
        self.graphics_scale = graphics_scale.max(0.0);
    }

    pub fn ocf_base(&self) -> u32 {
        let mut ocf = self.ocf_base;
        if self.rotateable != 0 {
            ocf |= crate::ocf::ROTATE;
        }
        // OCF_Grab from the Grab DefCore value (C4Object SetOCF).
        if self.grab != 0 {
            ocf |= crate::ocf::GRAB;
        }
        ocf
    }

    pub fn set_ocf_base(&mut self, ocf: u32) {
        self.ocf_base = ocf | OCF_NORMAL;
    }

    pub fn rotateable(&self) -> i32 {
        self.rotateable
    }

    pub fn set_rotateable(&mut self, rotateable: i32) {
        self.rotateable = rotateable;
    }

    /// The def+state arm of C4Object::SetOCF (C4Object.cpp:526-666).
    /// Context-dependent bits (HitSpeeds, Chop, InSolid, InFree, Available)
    /// join in `Engine::compute_object_ocf`. The raw `ocf_base` seed is the
    /// fixture shortcut for def flags this model does not carry.
    pub fn compute_ocf(&self, state: &ObjectState) -> u32 {
        self.compute_ocf_with_contents_count(state, state.contents.len())
    }

    /// Live SetOCF variant. `C4ObjectList::ObjectCount` ignores removed list
    /// holes but retains C4OS_INACTIVE entries, so Engine callers supply the
    /// count resolved against the authoritative object table.
    fn compute_ocf_with_contents_count(&self, state: &ObjectState, contents_count: usize) -> u32 {
        // OCF_Normal: the OCF is never zero (SetOCF, C4Object.cpp:547-548)
        let mut ocf = self.ocf_base | OCF_NORMAL;
        // OCF_NotContained (SetOCF, C4Object.cpp:627-629); OCF_Available
        // joins in Engine::compute_object_ocf with its container and
        // landscape clauses (C4Object.cpp:645-648).
        if state.container.is_none() {
            ocf |= crate::ocf::NOT_CONTAINED;
        }
        // OCF_FullCon (SetOCF, C4Object.cpp:567-569)
        if state.construction >= FULL_CON {
            ocf |= crate::ocf::FULL_CON;
        }
        // OCF_Living/OCF_Alive (SetOCF, C4Object.cpp:600-605)
        if state.category & CATEGORY_LIVING != 0 {
            ocf |= crate::ocf::LIVING;
            if state.alive {
                ocf |= crate::ocf::ALIVE;
            }
        }
        // OCF_Prey: Def->Prey && the RAW Alive flag (SetOCF,
        // C4Object.cpp:615-618)
        if self.prey && state.alive {
            ocf |= crate::ocf::PREY;
        }
        // OCF_CrewMember: Def->CrewMember && the RAW Alive flag
        // (SetOCF, C4Object.cpp:619-622)
        if self.crew_member && state.alive {
            ocf |= crate::ocf::CREW_MEMBER;
        }
        // OCF_AttractLightning at FullCon (SetOCF, C4Object.cpp:623-626)
        if self.attract_lightning && ocf & crate::ocf::FULL_CON != 0 {
            ocf |= crate::ocf::ATTRACT_LIGHTNING;
        }
        // OCF_Edible (SetOCF, C4Object.cpp:630-632)
        if self.edible {
            ocf |= crate::ocf::EDIBLE;
        }
        // OCF_FightReady: the OCF_Alive BIT, an action without
        // ObjectDisabled, and !Def->NoFight (SetOCF, C4Object.cpp:606-610).
        if ocf & crate::ocf::ALIVE != 0
            && !self
                .action_library
                .disables_object_for_entry(&state.action.name, state.action.act_map_index)
            && !self.no_fight
        {
            ocf |= crate::ocf::FIGHT_READY;
        }
        // OCF_Construct: can be built outside (SetOCF, C4Object.cpp:549-552)
        if self.constructable
            && state.construction < FULL_CON
            && state.rotation == 0
            && !state.on_fire
        {
            ocf |= crate::ocf::CONSTRUCT;
        }
        // OCF_Rotate: rotateable, but not a minimum (invisible)
        // construction site (SetOCF, C4Object.cpp:576-580)
        if self.rotateable != 0 && state.construction > 100 {
            ocf |= crate::ocf::ROTATE;
        }
        // OCF_Exclusive (SetOCF, C4Object.cpp:581-583)
        if self.exclusive {
            ocf |= crate::ocf::EXCLUSIVE;
        }
        // OCF_Entrance: positive entrance area, FullCon, and the
        // RotatedEntrance rotation gate (SetOCF, C4Object.cpp:584-587)
        if self
            .entrance_rect
            .is_some_and(|rect| rect.width > 0 && rect.height > 0)
            && ocf & crate::ocf::FULL_CON != 0
            && (self.rotated_entrance == 1 || state.rotation <= self.rotated_entrance)
        {
            ocf |= crate::ocf::ENTRANCE;
        }
        // OCF_Grab: Grab DefCore value, never on StaticBack objects
        // (SetOCF, C4Object.cpp:553-555)
        if self.grab != 0 && state.category & CATEGORY_STATIC_BACK == 0 {
            ocf |= crate::ocf::GRAB;
        }
        if self.collectible {
            ocf |= crate::ocf::CARRYABLE;
        }
        // OCF_OnFire (SetOCF, C4Object.cpp:559-561)
        if state.on_fire {
            ocf |= crate::ocf::ON_FIRE;
        }
        // OCF_InLiquid: the cached flag, uncontained only (SetOCF
        // C4Object.cpp:633-636, UpdateOCF :729-732).
        if state.in_liquid && state.container.is_none() {
            ocf |= crate::ocf::IN_LIQUID;
        }
        // OCF_Inflammable: not burning, ContactIncinerate set, not a dead
        // living (SetOCF, C4Object.cpp:562-566)
        if !state.on_fire
            && self.contact_incinerate > 0
            && (state.category & CATEGORY_LIVING == 0 || state.alive)
        {
            ocf |= crate::ocf::INFLAMMABLE;
        }
        if self.collection_ocf_enabled(state, contents_count, state.no_collect_delay) {
            ocf |= crate::ocf::COLLECTION;
        }
        // OCF_LineConstruct: FullCon + any LineConnect bit besides
        // C4D_EnergyHolder (SetOCF, C4Object.cpp:611-614)
        if ocf & crate::ocf::FULL_CON != 0 && self.line_connect & !LINE_CONNECT_ENERGY_HOLDER != 0 {
            ocf |= crate::ocf::LINE_CONSTRUCT;
        }
        // OCF_PowerConsumer (SetOCF, C4Object.cpp:649-652)
        if self.line_connect & LINE_CONNECT_POWER_CONSUMER != 0 && ocf & crate::ocf::FULL_CON != 0 {
            ocf |= crate::ocf::POWER_CONSUMER;
        }
        // OCF_PowerSupply: a generator, or an energized power output
        // (SetOCF, C4Object.cpp:653-657)
        if (self.line_connect & LINE_CONNECT_POWER_GENERATOR != 0
            || (self.line_connect & LINE_CONNECT_POWER_OUTPUT != 0 && state.energy > 0))
            && ocf & crate::ocf::FULL_CON != 0
        {
            ocf |= crate::ocf::POWER_SUPPLY;
        }
        // OCF_Container: Grab_Put, Grab_Get or an open entrance (SetOCF,
        // C4Object.cpp:658-660)
        if self.grab_put_get & (GRAB_PUT_GET_PUT | GRAB_PUT_GET_GET) != 0
            || ocf & crate::ocf::ENTRANCE != 0
        {
            ocf |= crate::ocf::CONTAINER;
        }
        ocf
    }

    /// The OCF_Collection bit decision (SetOCF, C4Object.cpp:593-599), including
    /// the raw fixture seed and explicit Contents and NoCollectDelay values.
    /// Host-context previews can therefore substitute those scalars without
    /// cloning the full state.
    fn collection_ocf_enabled(
        &self,
        state: &ObjectState,
        contents_len: usize,
        no_collect_delay: i32,
    ) -> bool {
        if self.ocf_base & crate::ocf::COLLECTION != 0 {
            return true;
        }
        (self.ocf_base & crate::ocf::FULL_CON != 0
            || state.construction >= FULL_CON
            || self.incomplete_activity)
            && self.collection_rect.is_some_and(|rect| rect.is_positive())
            && !collection_limit_reached(self.collection_limit, contents_len)
            && !self
                .action_library
                .disables_object_for_entry(&state.action.name, state.action.act_map_index)
            && no_collect_delay == 0
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    pub fn set_value(&mut self, value: i32) {
        self.def_core_reflected_ints
            .insert("Value".to_string(), value);
        self.value = value;
    }

    pub fn no_sell(&self) -> i32 {
        self.no_sell
    }

    pub fn set_no_sell(&mut self, no_sell: i32) {
        self.no_sell = no_sell;
    }

    pub fn rebuyable(&self) -> bool {
        self.rebuyable
    }

    pub fn set_rebuyable(&mut self, rebuyable: bool) {
        self.rebuyable = rebuyable;
    }

    pub fn base_auto_sell(&self) -> bool {
        self.base_auto_sell
    }

    pub fn set_base_auto_sell(&mut self, base_auto_sell: bool) {
        self.base_auto_sell = base_auto_sell;
    }

    pub fn mass(&self) -> i32 {
        self.mass
    }

    pub fn set_mass(&mut self, mass: i32) {
        self.mass = mass.max(0);
    }

    pub fn move_to_range(&self) -> i32 {
        self.move_to_range
    }

    pub fn set_move_to_range(&mut self, move_to_range: i32) {
        self.move_to_range = move_to_range;
    }

    pub fn pathfinder(&self) -> i32 {
        self.pathfinder
    }

    pub fn set_pathfinder(&mut self, pathfinder: i32) {
        self.pathfinder = pathfinder;
    }

    pub fn no_transfer_zones(&self) -> i32 {
        self.no_transfer_zones
    }

    pub fn set_no_transfer_zones(&mut self, no_transfer_zones: i32) {
        self.no_transfer_zones = no_transfer_zones;
    }

    pub fn no_push_enter(&self) -> i32 {
        self.no_push_enter
    }

    pub fn set_no_push_enter(&mut self, no_push_enter: i32) {
        self.no_push_enter = no_push_enter;
    }

    /// `Grab` DefCore value (C4Def.h): 0 = not grabbable, 1 = grab and
    /// push, 2 = grab only (C4Object.cpp:1763).
    pub fn grab(&self) -> i32 {
        self.grab
    }

    pub fn set_grab(&mut self, grab: i32) {
        self.grab = grab;
    }

    pub fn picture(&self) -> Option<DefinitionPicture> {
        self.picture
    }

    pub fn set_picture(&mut self, picture: Option<DefinitionPicture>) {
        self.picture = picture;
    }

    pub fn picture_image(&self) -> Option<&DefinitionPictureImage> {
        self.picture_image.as_ref()
    }

    pub fn set_picture_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.picture_image = image;
    }

    /// First def portrait (C4CFN_Portraits, src/C4Components.h:88).
    pub fn portrait_image(&self) -> Option<&DefinitionPictureImage> {
        self.portrait_image.as_ref()
    }

    pub fn set_portrait_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.portrait_image = image;
    }

    pub fn portrait_graphics_image(&self) -> Option<&DefinitionPictureImage> {
        self.portrait_graphics_image.as_ref()
    }

    pub fn set_portrait_graphics_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.portrait_graphics_image = image;
    }

    pub fn portrait_graphics(&self, name: &str) -> Option<&DefinitionPictureImage> {
        self.portrait_graphics
            .iter()
            .find(|(candidate, _)| clonk_resources::material::c4_names_equal(candidate, name))
            .map(|(_, image)| image)
    }

    pub(crate) fn portrait_graphics_names(&self) -> impl Iterator<Item = &str> {
        self.portrait_graphics.iter().map(|(name, _)| name.as_str())
    }

    pub fn set_portrait_graphics(&mut self, portraits: Vec<(String, DefinitionPictureImage)>) {
        self.portrait_graphics = portraits;
    }

    /// Def rank symbols (C4Def::pRankSymbols, src/C4Def.cpp:684-691).
    pub fn rank_symbols_image(&self) -> Option<&DefinitionPictureImage> {
        self.rank_symbols_image.as_ref()
    }

    pub fn set_rank_symbols_image(&mut self, image: Option<DefinitionPictureImage>) {
        self.rank_symbols_owned = image.is_some();
        self.rank_symbols_image = image;
    }

    pub fn rank_names(&self) -> Option<&RankNameTable> {
        self.rank_names.as_ref()
    }

    pub fn set_rank_names(&mut self, names: Option<Vec<String>>) {
        let base = names.as_ref().map(|_| 1_000);
        self.set_rank_system(names, base);
    }

    pub fn rank_base(&self) -> Option<i32> {
        self.rank_base
    }

    pub fn set_rank_system(&mut self, names: Option<Vec<String>>, rank_base: Option<i32>) {
        self.set_rank_name_table(names.map(RankNameTable::from_resolved_names), rank_base);
    }

    fn set_rank_name_table(&mut self, names: Option<RankNameTable>, rank_base: Option<i32>) {
        self.rank_names_owned = names.is_some();
        self.rank_names = names;
        self.rank_base = self.rank_names.as_ref().map(|_| match rank_base {
            Some(0) | None => 1_000,
            Some(base) => base,
        });
    }

    pub fn rank_symbol_count(&self) -> Option<u32> {
        self.rank_symbol_count
    }

    pub fn set_rank_symbol_count(&mut self, count: Option<u32>) {
        self.rank_symbol_count = count;
    }

    /// C4Def::ColorizeByMaterial / C4DefGraphics::ColorizeByMaterial:
    /// recolor every surface in the base/additional/portrait graphics chain.
    /// Rust retains separate cropped presentation images, so recolor those
    /// copies too; otherwise C4Def::Picture and the first portrait would keep
    /// showing the pre-colorization bitmap.
    fn colorize_by_material(&mut self, materials: &MaterialSet) {
        if self.color_by_material.is_empty() {
            return;
        }
        let Some(material) = materials.get(&self.color_by_material) else {
            tracing::error!(
                "C4Def::ColorizeByMaterial: mat {} not defined",
                self.color_by_material
            );
            return;
        };
        let colors = c4_material_definition_colors(material);

        if let Some(image) = self.sprite_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        for image in self.sprite_variants.values_mut() {
            image.colorize_by_material(&colors);
        }
        if let Some(image) = self.picture_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        if let Some(image) = self.portrait_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        if let Some(image) = self.portrait_graphics_image.as_mut() {
            image.colorize_by_material(&colors);
        }
        for (_, image) in &mut self.portrait_graphics {
            image.colorize_by_material(&colors);
        }

        // Material alpha can change the collision mask derived from the base
        // sprite. Rebuilding also drops every lazily cached named mask.
        self.rebuild_solid_mask_pixels();
    }

    pub fn sprite_image(&self) -> Option<&DefinitionSpriteImage> {
        self.sprite_image.as_ref()
    }

    pub fn set_sprite_image(&mut self, image: Option<DefinitionSpriteImage>) {
        self.sprite_image = image;
        self.rebuild_solid_mask_pixels();
    }

    /// C4Def::Load clears invalid BASE DefCore graphics rectangles before
    /// objects copy them or the renderer sees TopFace (C4Def.cpp:727-741).
    pub(crate) fn validate_base_graphics_rects(&mut self) {
        let Some(image) = self.sprite_image.as_ref() else {
            return;
        };
        let image_width = i64::from(image.width);
        let image_height = i64::from(image.height);

        let invalid_solid_mask = self.def_core_solid_mask.is_some_and(|solid_mask| {
            solid_mask.x < 0
                || solid_mask.y < 0
                || i64::from(solid_mask.x) + i64::from(solid_mask.width) > image_width
                || i64::from(solid_mask.y) + i64::from(solid_mask.height) > image_height
        });
        if invalid_solid_mask {
            self.set_solid_mask(None);
        }

        let logical_width = image_width as f32 / self.graphics_scale;
        let logical_height = image_height as f32 / self.graphics_scale;
        let invalid_top_face = self.def_core_top_face.is_some_and(|top_face| {
            top_face.x < 0
                || top_face.y < 0
                || (i64::from(top_face.x) + i64::from(top_face.width)) as f32 > logical_width
                || (i64::from(top_face.y) + i64::from(top_face.height)) as f32 > logical_height
        });
        if invalid_top_face {
            tracing::warn!(
                definition = %self.id,
                name = %self.name,
                "invalid TopFace; cleared"
            );
            self.set_top_face(None);
        }
    }

    pub fn sprite_image_variant(
        &self,
        graphics_name: Option<&str>,
    ) -> Option<&DefinitionSpriteImage> {
        match graphics_name {
            None | Some("") => self.sprite_image.as_ref(),
            Some(name) => {
                let key = clonk_resources::material::c4_name_key(name);
                self.sprite_variants.get(&key)
            }
        }
    }

    pub fn set_sprite_variants(&mut self, variants: HashMap<String, DefinitionSpriteImage>) {
        self.sprite_variants = variants;
        self.solid_mask_rect_cache.borrow_mut().clear();
    }

    pub fn sprite_variant_keys(&self) -> Vec<String> {
        self.sprite_variants.keys().cloned().collect()
    }

    pub fn shape_rect(&self) -> Option<DefinitionRect> {
        self.shape
    }

    /// `Float` DefCore value (C4Def.cpp:379) — the buoyancy line.
    pub fn line(&self) -> i32 {
        self.line
    }

    pub fn set_line(&mut self, line: i32) {
        self.line = line;
    }

    pub fn line_intersect(&self) -> i32 {
        self.line_intersect
    }

    pub fn set_line_intersect(&mut self, line_intersect: i32) {
        self.line_intersect = line_intersect;
    }

    pub fn set_float_line(&mut self, float_line: i32) {
        self.float_line = float_line;
    }

    pub fn set_shape_rect(&mut self, rect: Option<DefinitionRect>) {
        self.shape = rect;
    }

    pub fn solid_mask(&self) -> Option<DefinitionTargetRect> {
        self.solid_mask
    }

    pub fn set_solid_mask(&mut self, rect: Option<DefinitionTargetRect>) {
        self.def_core_solid_mask = rect;
        self.solid_mask = rect.filter(DefinitionTargetRect::is_positive);
        self.rebuild_solid_mask_pixels();
    }

    pub fn top_face(&self) -> Option<DefinitionTargetRect> {
        self.top_face
    }

    pub fn set_top_face(&mut self, rect: Option<DefinitionTargetRect>) {
        self.def_core_top_face = rect;
        self.top_face = rect.filter(DefinitionTargetRect::is_positive);
    }

    /// Per-pixel decode for an ARBITRARY mask rect — Objects.txt
    /// SolidMask= overrides pick a different sprite region per object
    /// (C4Object::SolidMask; the CTWR platform has NO DefCore mask at
    /// all). Cached per rect.
    fn solid_mask_pixels_for_rect(
        &self,
        mask: DefinitionTargetRect,
        graphics_name: Option<&str>,
    ) -> SolidMaskPixels {
        let graphics_key = graphics_name
            .filter(|name| !name.is_empty())
            .map(clonk_resources::material::c4_name_key);
        if graphics_key.is_none() && Some(mask) == self.solid_mask {
            return self.solid_mask_pixels.clone();
        }
        let key = (
            graphics_key.clone(),
            mask.x,
            mask.y,
            mask.width,
            mask.height,
        );
        if let Some(cached) = self.solid_mask_rect_cache.borrow().get(&key) {
            return cached.clone();
        }
        let computed = self.compute_solid_mask_pixels(mask, graphics_key.as_deref());
        self.solid_mask_rect_cache
            .borrow_mut()
            .insert(key, computed.clone());
        computed
    }

    fn compute_solid_mask_pixels(
        &self,
        mask: DefinitionTargetRect,
        graphics_name: Option<&str>,
    ) -> SolidMaskPixels {
        let Some(image) = self.sprite_image_variant(graphics_name) else {
            return if graphics_name.is_none() {
                SolidMaskPixels::Rectangle
            } else {
                // C++ SetGraphics rejects an unknown named graphic. Never
                // silently substitute the owning definition's default bitmap.
                SolidMaskPixels::OutOfBounds
            };
        };
        let image_width = i32::try_from(image.width).unwrap_or(i32::MAX);
        let image_height = i32::try_from(image.height).unwrap_or(i32::MAX);
        let pixels = image.solid_mask_source_pixels();
        solid_mask_pixels_for_checked_bitmap(mask, image_width, image_height, pixels.as_ref())
            .map(SolidMaskPixels::Alpha)
            .unwrap_or(SolidMaskPixels::OutOfBounds)
    }

    /// Extract the SolidMask alpha pixels from the sprite (alpha != 0 =
    /// solid), mirroring the per-tick scan this replaces.
    fn rebuild_solid_mask_pixels(&mut self) {
        self.solid_mask_rect_cache.borrow_mut().clear();
        let Some(mask) = self.solid_mask else {
            self.solid_mask_pixels = SolidMaskPixels::default();
            return;
        };
        if let Some(image) = self.sprite_image.as_ref() {
            let right = i64::from(mask.x) + i64::from(mask.width);
            let bottom = i64::from(mask.y) + i64::from(mask.height);
            if mask.x < 0
                || mask.y < 0
                || right > i64::from(image.width)
                || bottom > i64::from(image.height)
            {
                // The definition-level cache holds the RAW DefCore rect.
                // Object Init/runtime checks persist a distinct checked rect
                // and therefore bypass this entry. Keep malformed raw sizes
                // from allocating before C4Def::Load validation runs.
                self.solid_mask_pixels = SolidMaskPixels::OutOfBounds;
                return;
            }
        }
        self.solid_mask_pixels = self.compute_solid_mask_pixels(mask, None);
    }

    pub fn shape_vertices(&self) -> &[ObjectVertex] {
        &self.shape_vertices
    }

    pub fn set_shape_vertices(&mut self, vertices: Vec<ObjectVertex>) {
        self.shape_vertex_slots = ShapeVertexBuffer::from_active(&vertices);
        self.shape_vertices = vertices;
    }

    fn set_shape_vertex_slots(&mut self, active_count: usize, slots: &[ObjectVertex]) {
        self.shape_vertex_slots = ShapeVertexBuffer::from_slots(active_count, slots);
        self.shape_vertices = self.shape_vertex_slots.active_vec();
    }

    fn shape_vertex_buffer(&self) -> &ShapeVertexBuffer {
        &self.shape_vertex_slots
    }

    pub fn contact_density(&self) -> i32 {
        self.contact_density
    }

    pub fn set_contact_density(&mut self, contact_density: i32) {
        self.contact_density = contact_density;
    }

    pub fn contact_function_calls(&self) -> bool {
        self.contact_function_calls
    }

    pub fn set_contact_function_calls(&mut self, contact_function_calls: bool) {
        self.contact_function_calls = contact_function_calls;
    }

    pub fn border_bound(&self) -> i32 {
        self.border_bound
    }

    pub fn set_border_bound(&mut self, border_bound: i32) {
        self.border_bound = border_bound;
    }

    pub fn upright_attach(&self) -> i32 {
        self.upright_attach
    }

    pub fn no_stabilize(&self) -> bool {
        self.no_stabilize
    }

    pub fn set_no_stabilize(&mut self, no_stabilize: bool) {
        self.no_stabilize = no_stabilize;
    }

    pub fn timer(&self) -> i32 {
        self.timer
    }

    pub fn set_timer(&mut self, timer: i32) {
        self.timer = timer;
    }

    pub fn timer_call(&self) -> Option<&str> {
        self.timer_call.as_deref()
    }

    fn timer_callback(&self) -> Option<ScriptCallbackTarget> {
        self.timer_call_link.target(self.timer_call.as_deref())
    }

    fn control_transfer_callback(&self) -> Option<ScriptCallbackTarget> {
        self.control_transfer_link.target(Some("ControlTransfer"))
    }

    pub fn set_timer_call(&mut self, timer_call: Option<String>) {
        self.timer_call = timer_call;
        self.mark_callbacks_unlinked();
    }

    pub fn set_upright_attach(&mut self, upright_attach: i32) {
        self.upright_attach = upright_attach;
    }

    /// RotatedSolidmasks (C4Def.cpp:414): rotation does not disable the
    /// solid mask (C4Object.cpp:5655).
    pub fn rotated_solid_masks(&self) -> bool {
        self.rotated_solid_masks
    }

    pub fn auto_context_menu(&self) -> bool {
        self.auto_context_menu
    }

    pub fn no_component_mass(&self) -> bool {
        self.no_component_mass
    }

    pub fn set_no_component_mass(&mut self, no_component_mass: bool) {
        self.no_component_mass = no_component_mass;
    }

    pub fn set_rotated_solid_masks(&mut self, rotated_solid_masks: bool) {
        self.rotated_solid_masks = rotated_solid_masks;
    }

    pub fn set_auto_context_menu(&mut self, auto_context_menu: bool) {
        self.auto_context_menu = auto_context_menu;
    }

    pub fn collection_rect(&self) -> Option<DefinitionRect> {
        self.collection_rect
    }

    pub fn fire_top(&self) -> i32 {
        self.fire_top
    }

    pub fn set_fire_top(&mut self, fire_top: i32) {
        self.fire_top = fire_top;
    }

    pub fn lift_top(&self) -> i32 {
        self.lift_top
    }

    pub fn set_lift_top(&mut self, lift_top: i32) {
        self.lift_top = lift_top;
    }

    pub fn set_collection_rect(&mut self, rect: Option<DefinitionRect>) {
        self.def_core_collection_rect = rect;
        self.collection_rect = rect.filter(DefinitionRect::is_positive);
    }

    pub fn collection_limit(&self) -> i32 {
        self.collection_limit
    }

    pub fn contact_incinerate(&self) -> i32 {
        self.contact_incinerate
    }

    pub fn blast_incinerate(&self) -> i32 {
        self.blast_incinerate
    }

    pub fn set_blast_incinerate(&mut self, blast_incinerate: i32) {
        self.blast_incinerate = blast_incinerate;
    }

    pub fn contain_blast(&self) -> i32 {
        self.contain_blast
    }

    pub fn set_contain_blast(&mut self, contain_blast: i32) {
        self.contain_blast = contain_blast;
    }

    pub fn closed_container(&self) -> i32 {
        self.closed_container
    }

    pub fn set_closed_container(&mut self, closed_container: i32) {
        self.closed_container = closed_container;
    }

    pub fn no_horizontal_move(&self) -> i32 {
        self.no_horizontal_move
    }

    pub fn set_no_horizontal_move(&mut self, no_horizontal_move: i32) {
        self.no_horizontal_move = no_horizontal_move;
    }

    pub fn no_burn_decay(&self) -> bool {
        self.no_burn_decay
    }

    pub fn no_burn_damage(&self) -> bool {
        self.no_burn_damage
    }

    pub fn no_breath(&self) -> bool {
        self.no_breath
    }

    pub fn set_no_breath(&mut self, no_breath: bool) {
        self.no_breath = no_breath;
    }

    pub fn set_fire_properties(
        &mut self,
        contact_incinerate: i32,
        no_burn_decay: bool,
        no_burn_damage: bool,
    ) {
        self.contact_incinerate = contact_incinerate;
        self.no_burn_decay = no_burn_decay;
        self.no_burn_damage = no_burn_damage;
    }

    pub fn burn_turn_to(&self) -> Option<&str> {
        self.burn_turn_to.as_deref()
    }

    pub fn build_turn_to(&self) -> Option<&str> {
        self.build_turn_to.as_deref()
    }

    pub fn incomplete_activity(&self) -> bool {
        self.incomplete_activity
    }

    pub fn set_burn_turn_to(&mut self, target: Option<String>) {
        self.burn_turn_to = target;
    }

    pub fn set_build_turn_to(&mut self, target: Option<String>) {
        self.build_turn_to = target;
    }

    pub fn set_incomplete_activity(&mut self, enabled: bool) {
        self.incomplete_activity = enabled;
    }

    pub fn is_exclusive(&self) -> bool {
        self.exclusive
    }

    pub fn set_exclusive(&mut self, exclusive: bool) {
        self.exclusive = exclusive;
    }

    pub fn set_edible(&mut self, edible: bool) {
        self.edible = edible;
    }

    pub fn set_prey(&mut self, prey: bool) {
        self.prey = prey;
    }

    pub fn set_attract_lightning(&mut self, attract_lightning: bool) {
        self.attract_lightning = attract_lightning;
    }

    pub fn entrance_rect(&self) -> Option<DefinitionRect> {
        self.entrance_rect
    }

    pub fn set_entrance_rect(&mut self, entrance_rect: Option<DefinitionRect>) {
        self.entrance_rect = entrance_rect;
    }

    pub fn set_rotated_entrance(&mut self, rotated_entrance: i32) {
        self.rotated_entrance = rotated_entrance;
    }

    pub fn set_no_fight(&mut self, no_fight: bool) {
        self.no_fight = no_fight;
    }

    pub fn is_chopable(&self) -> bool {
        self.chopable
    }

    pub fn set_chopable(&mut self, chopable: bool) {
        self.chopable = chopable;
    }

    pub fn physical(&self) -> &PhysicalInfo {
        &self.physical
    }

    /// Seed for the FnAdjustWalkRotation host seam (C4Script.cpp:5439-5448):
    /// Def->Rotateable, the frame's Action.t_attach, the shape attach record
    /// and Def->Shape.VtxX[Shape.iAttachVtx] (C4Object.cpp:6023).
    fn walk_rotation_seed(&self, state: &ObjectState) -> compat::WalkRotationSeed {
        compat::WalkRotationSeed {
            rotateable: self.rotateable,
            t_attach: state.t_attach,
            attach: state.shape_attach,
            def_attach_vtx_x: usize::try_from(state.shape_attach.vtx)
                .ok()
                .and_then(|vtx| self.shape_vertices.get(vtx))
                .map(|vertex| vertex.x)
                .unwrap_or(0),
        }
    }

    pub fn set_physical(&mut self, physical: PhysicalInfo) {
        self.physical = physical;
    }

    pub fn set_collection_limit(&mut self, limit: i32) {
        self.collection_limit = limit;
    }

    pub fn fragile(&self) -> bool {
        self.fragile
    }

    pub fn set_fragile(&mut self, fragile: bool) {
        self.fragile = fragile;
    }

    pub fn projectile(&self) -> i32 {
        self.projectile
    }

    pub fn set_projectile(&mut self, projectile: i32) {
        self.projectile = projectile;
    }

    pub fn is_collectible(&self) -> bool {
        self.collectible
    }

    pub fn no_get(&self) -> bool {
        self.no_get
    }

    pub fn set_no_get(&mut self, no_get: bool) {
        self.no_get = no_get;
    }

    pub fn hide_hud_bars(&self) -> i32 {
        self.hide_hud_bars
    }

    pub fn set_hide_hud_bars(&mut self, hide_hud_bars: i32) {
        self.hide_hud_bars = hide_hud_bars;
    }

    pub fn hide_hud_elements(&self) -> i32 {
        self.hide_hud_elements
    }

    pub fn set_hide_hud_elements(&mut self, hide_hud_elements: i32) {
        self.hide_hud_elements = hide_hud_elements;
    }

    pub fn grab_put_get(&self) -> i32 {
        self.grab_put_get
    }

    pub fn vehicle_control(&self) -> i32 {
        self.vehicle_control
    }

    pub fn set_vehicle_control(&mut self, vehicle_control: i32) {
        self.vehicle_control = vehicle_control;
    }

    pub fn set_grab_put_get(&mut self, grab_put_get: i32) {
        self.grab_put_get = grab_put_get;
    }

    pub fn set_collectible(&mut self, collectible: bool) {
        self.collectible = collectible;
    }

    pub fn is_constructable(&self) -> bool {
        self.constructable
    }

    pub fn set_constructable(&mut self, constructable: bool) {
        self.constructable = constructable;
    }

    pub fn construction_offset(&self) -> i32 {
        self.construction_offset
    }

    pub fn set_construction_offset(&mut self, offset: i32) {
        self.construction_offset = offset;
    }

    pub fn stretch_growth(&self) -> bool {
        self.stretch_growth
    }

    pub fn set_stretch_growth(&mut self, stretch_growth: bool) {
        self.stretch_growth = stretch_growth;
    }

    pub fn oversize(&self) -> bool {
        self.oversize
    }

    pub fn set_oversize(&mut self, oversize: bool) {
        self.oversize = oversize;
    }

    pub fn placement(&self) -> i32 {
        self.placement
    }

    pub fn set_placement(&mut self, placement: i32) {
        self.placement = placement;
    }

    pub fn growth(&self) -> i32 {
        self.growth
    }

    pub fn set_growth(&mut self, growth: i32) {
        self.growth = growth;
    }

    pub fn basement(&self) -> i32 {
        self.basement
    }

    pub fn set_basement(&mut self, basement: i32) {
        self.basement = basement;
    }

    pub fn components(&self) -> &[DefinitionComponent] {
        &self.components
    }

    pub fn set_components(&mut self, components: Vec<DefinitionComponent>) {
        self.components = components;
    }

    pub fn line_connect(&self) -> u32 {
        self.line_connect
    }

    pub fn set_line_connect(&mut self, line_connect: u32) {
        self.line_connect = line_connect;
    }

    fn call_initialize(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        random: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            CommandBatch,
            AudioRegistry,
            LcgRng,
            u64,
            Option<EngineError>,
        ),
        EngineError,
    > {
        if !self.has_initialize {
            return Ok((
                CommandBatch::default(),
                audio,
                rng,
                world.next_object_id(),
                None,
            ));
        }
        // C++ Initialize runs with no parameters (PSF_Initialize broadcast);
        // (state, random) is the synthetic command-DSL convention.
        let args = if self.c4_callback_args {
            Vec::new()
        } else {
            vec![
                build_state_value(&self.id, object_id, state, &self.action_library),
                Value::Int(random),
            ]
        };
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        // Cells live OUTSIDE the session so a mid-call error keeps the
        // pre-error local writes (C4AulExec aborts the call but rolls
        // nothing back, C4AulExec.cpp:1318-1342).
        let cells = clonk_script::LocalCells::from_local_vars(&state.local_vars);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.time,
                    state.action.data,
                    state.action.phase,
                    self.shared_action_library(&world),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_action_index(state.action.act_map_index)
                .with_shape_vertices(&state.shape_vertices)
                .with_definition_id(self.id.as_str())
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_controller(state.controller)
                .with_in_liquid(state.in_liquid)
                .with_own_mass(state.own_mass)
                .with_physicals(
                    state.info_physical,
                    state.temporary_physical,
                    state.physical_changes.clone(),
                    *self.physical(),
                )
                .with_walk_rotation(self.walk_rotation_seed(state))
                .with_script_fixed_position(state.script_fixed_position)
                .with_script_fixed_velocity(state.script_fixed_velocity)
                .with_script_rotation_velocity(state.script_rotation_velocity)
                .with_script_fixed_rotation(state.script_fixed_rotation)
                .with_magic_energy(state.magic_energy)
                .with_breath(state.breath)
                .with_need_energy(state.need_energy)
                .with_ocf(state.ocf),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                compat::register_session_local_cells(object_id, cells.clone());
                self.script.call_with_cells_and_this(
                    "Initialize",
                    &args,
                    &cells,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        // A script error aborts the call but the partial outcome below
        // still folds — C++'s CreateObject/SetTransferZone/Random side
        // effects all landed live before the unwind and NewObj's
        // `Number = ++ObjectEnumerationIndex` (C4Game.cpp:1119) stays
        // advanced; the error surfaces to the caller as a value.
        let (returned, script_error) = match result {
            Ok(value) => (Some(value), None),
            Err(source) => (
                None,
                Some(script_execution_error(
                    self.id.clone(),
                    "Initialize".to_string(),
                    source,
                    None,
                )),
            ),
        };
        let mut batch = match returned {
            Some(value) => parse_command(&self.id, "Initialize", value)?,
            None => CommandBatch::default(),
        };
        // Store updated local variables in the delta so they persist
        batch.delta.local_vars = Some(cells.snapshot());
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;
        batch.other_objects.extend(other_objects);
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);

        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if let Some(update) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &update);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        if !object_commands.is_empty() {
            batch.commands.extend(object_commands);
        }
        if !command_operations.is_empty() {
            batch.command_ops.extend(command_operations);
        }
        if destroy_object {
            batch.destroy = true;
        }
        if !host_object_effects.is_empty() {
            batch.effects.extend(host_object_effects);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
        }
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview = host_raster_preview;
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id, script_error))
    }

    fn call_construction(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            CommandBatch,
            AudioRegistry,
            LcgRng,
            u64,
            Option<EngineError>,
        ),
        EngineError,
    > {
        if !self.has_construction {
            return Ok((
                CommandBatch::default(),
                audio,
                rng,
                world.next_object_id(),
                None,
            ));
        }
        // Construction() takes no arguments
        let args: [Value; 0] = [];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        // Cells live OUTSIDE the session so a mid-call error keeps the
        // pre-error local writes (C4AulExec.cpp:1318-1342, no rollback).
        let cells = clonk_script::LocalCells::from_local_vars(&state.local_vars);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.time,
                    state.action.data,
                    state.action.phase,
                    self.shared_action_library(&world),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_action_index(state.action.act_map_index)
                .with_shape_vertices(&state.shape_vertices)
                .with_definition_id(self.id.as_str())
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_controller(state.controller)
                .with_in_liquid(state.in_liquid)
                .with_own_mass(state.own_mass)
                .with_physicals(
                    state.info_physical,
                    state.temporary_physical,
                    state.physical_changes.clone(),
                    *self.physical(),
                )
                .with_walk_rotation(self.walk_rotation_seed(state))
                .with_script_fixed_position(state.script_fixed_position)
                .with_script_fixed_velocity(state.script_fixed_velocity)
                .with_script_rotation_velocity(state.script_rotation_velocity)
                .with_script_fixed_rotation(state.script_fixed_rotation)
                .with_magic_energy(state.magic_energy)
                .with_breath(state.breath)
                .with_need_energy(state.need_energy)
                .with_ocf(state.ocf),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                compat::register_session_local_cells(object_id, cells.clone());
                self.script.call_with_cells_and_this(
                    "Construction",
                    &args,
                    &cells,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        // A script error aborts the call but the partial outcome below
        // still folds (C4AulExec.cpp:1318-1342, no rollback) — the error
        // surfaces to the caller as a value.
        let script_error = result.err().map(|source| {
            script_execution_error(self.id.clone(), "Construction".to_string(), source, None)
        });
        // Construction() return value is not used (it just returns 0 or nil)
        // We only care about side effects (initializing local variables, etc.)
        // But we DO need to capture updated local variable values
        let mut batch = CommandBatch::default();
        // Store updated local variables in the delta so they persist
        batch.delta.local_vars = Some(cells.snapshot());
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;
        batch.other_objects.extend(other_objects);
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);

        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if let Some(update) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &update);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        batch.spawns.extend(host_spawns);
        batch.landscape_ops.extend(host_landscape_ops);
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview = host_raster_preview;
        batch.particles.extend(host_particles);
        batch.transfer_zones.extend(host_transfer_zones);
        batch.messages.extend(host_messages);
        batch.commands.extend(object_commands);
        batch.command_ops.extend(command_operations);
        batch.effects.extend(host_object_effects);
        batch.global_effects.extend(host_global_effects);
        batch.destroy = destroy_object;
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id, script_error))
    }

    fn call_step(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        frame: u64,
        random: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(CommandBatch, AudioRegistry, LcgRng, u64), EngineError> {
        if !self.has_step {
            return Ok((CommandBatch::default(), audio, rng, world.next_object_id()));
        }
        let frame_value = if frame > i32::MAX as u64 {
            i32::MAX
        } else {
            frame as i32
        };
        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::Int(frame_value),
            Value::Int(random),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.time,
                    state.action.data,
                    state.action.phase,
                    self.shared_action_library(&world),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    self.ocf_base,
                    self.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_action_index(state.action.act_map_index)
                .with_shape_vertices(&state.shape_vertices)
                .with_definition_id(self.id.as_str())
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_controller(state.controller)
                .with_in_liquid(state.in_liquid)
                .with_own_mass(state.own_mass)
                .with_physicals(
                    state.info_physical,
                    state.temporary_physical,
                    state.physical_changes.clone(),
                    *self.physical(),
                )
                .with_walk_rotation(self.walk_rotation_seed(state))
                .with_script_fixed_position(state.script_fixed_position)
                .with_script_fixed_velocity(state.script_fixed_velocity)
                .with_script_rotation_velocity(state.script_rotation_velocity)
                .with_script_fixed_rotation(state.script_fixed_rotation)
                .with_magic_energy(state.magic_energy)
                .with_breath(state.breath)
                .with_need_energy(state.need_energy)
                .with_ocf(state.ocf),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_session("Step", &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let (result, updated_local_vars) = result.map_err(|source| {
            script_execution_error(self.id.clone(), "Step".to_string(), source, None)
        })?;
        let mut batch = parse_command(&self.id, "Step", result)?;
        batch.delta.local_vars = Some(updated_local_vars);
        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;
        batch.other_objects.extend(other_objects);
        batch.audio.extend(host_audio.events);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);

        if let Some(delta) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &delta);
        }
        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }

        if let Some(update) = object_update {
            batch.delta.merge_update(update);
        }
        if !object_commands.is_empty() {
            batch.commands.extend(object_commands);
        }
        if !command_operations.is_empty() {
            batch.command_ops.extend(command_operations);
        }
        if destroy_object {
            batch.destroy = true;
        }
        if !host_object_effects.is_empty() {
            batch.effects.extend(host_object_effects);
        }
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
        }
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview = host_raster_preview;
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, next_object_id))
    }

    fn call_action_callback(
        &self,
        object_definition: &Definition,
        callback: &ScriptCallbackTarget,
        kind: ActionCallbackKind,
        state: &ObjectState,
        object_id: ObjectId,
        action_name: &str,
        abort_phase: Option<i32>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        // Linked definitions arrive with an exact retained function body.
        // Unlinked synthetic fixtures keep their historical name-based path.

        // C++ ActMap callbacks pass no parameters, except AbortCall which
        // gets the last phase (C4Object.cpp:4154,4168,4182). The (state,
        // action) pair is the synthetic command-DSL convention.
        let args = if self.c4_callback_args {
            match kind {
                ActionCallbackKind::Abort => {
                    vec![Value::Int(abort_phase.unwrap_or(state.action.phase))]
                }
                _ => Vec::new(),
            }
        } else {
            vec![
                build_state_value(
                    &object_definition.id,
                    object_id,
                    state,
                    &object_definition.action_library,
                ),
                Value::String(action_name.to_string().into()),
            ]
        };
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            Some(
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.time,
                    state.action.data,
                    state.action.phase,
                    object_definition.shared_action_library(&world),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    object_definition.ocf_base,
                    object_definition.crew_member,
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_action_index(state.action.act_map_index)
                .with_shape_vertices(&state.shape_vertices)
                .with_definition_id(object_definition.id.as_str())
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_base_graphics(state.base_graphics.clone())
                .with_alive(state.alive)
                .with_controller(state.controller)
                .with_in_liquid(state.in_liquid)
                .with_own_mass(state.own_mass)
                .with_physicals(
                    state.info_physical,
                    state.temporary_physical,
                    state.physical_changes.clone(),
                    *object_definition.physical(),
                )
                .with_walk_rotation(object_definition.walk_rotation_seed(state))
                .with_script_fixed_position(state.script_fixed_position)
                .with_script_fixed_velocity(state.script_fixed_velocity)
                .with_script_rotation_velocity(state.script_rotation_velocity)
                .with_script_fixed_rotation(state.script_fixed_rotation)
                .with_magic_energy(state.magic_energy)
                .with_breath(state.breath)
                .with_need_energy(state.need_energy)
                .with_ocf(state.ocf),
            ),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_callback_session(callback, &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let mut host_effects = host_effects;
        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }
        let audio_state = audio_guard.finish();
        let (value, updated_local_vars) = match result {
            Ok(value) => value,
            Err(source) => {
                return Err(script_execution_error(
                    self.id.clone(),
                    kind.context().to_string(),
                    source,
                    Some(Box::new(ScriptCallRecovery {
                        outcome: host_effects,
                        audio: audio_state,
                        rng,
                    })),
                ));
            }
        };

        // Action callbacks can return any value in C4Script.
        // The return value is typically used to indicate success/failure (e.g., return 1).
        // Unlike some other callback types, we don't validate or use the return value here.
        // This matches the C++ engine behavior where callbacks like Scaling() return int.
        drop(value);

        // Store updated local variables so they persist
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        Ok((host_effects, audio_state, rng))
    }

    fn call_menu_entries(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Vec<ContextMenuEntry>, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function("MenuEntries") {
            return Ok((Vec::new(), audio, rng));
        }

        let args = [build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        )];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.time,
            state.action.data,
            state.action.phase,
            self.shared_action_library(&world),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_action_index(state.action.act_map_index)
        .with_shape_vertices(&state.shape_vertices)
        .with_definition_id(self.id.as_str())
        .with_alive(state.alive)
        .with_controller(state.controller)
        .with_in_liquid(state.in_liquid)
        .with_own_mass(state.own_mass)
        .with_physicals(
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            *self.physical(),
        )
        .with_base_graphics(state.base_graphics.clone())
        // The scope publishes its whole overlay list, so it must start
        // from the object's real overlays: C4Object::GetGraphicsOverlay
        // splices a single node (src/C4Object.cpp:5962-5977).
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_walk_rotation(self.walk_rotation_seed(state))
        .with_script_fixed_position(state.script_fixed_position)
        .with_script_fixed_velocity(state.script_fixed_velocity)
        .with_script_rotation_velocity(state.script_rotation_velocity)
        .with_script_fixed_rotation(state.script_fixed_rotation)
        .with_magic_energy(state.magic_energy)
        .with_breath(state.breath)
        .with_need_energy(state.need_energy)
        .with_ocf(state.ocf);
        let (result, outcome) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_session("MenuEntries", &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, _updated_local_vars) = result.map_err(|source| {
            script_execution_error(self.id.clone(), "MenuEntries".to_string(), source, None)
        })?;
        // MenuEntries shouldn't modify local vars, so we discard them
        let entries = self.parse_context_menu_entries(value)?;

        if !outcome.object.is_empty()
            || !outcome.global.is_empty()
            || outcome.object_update.is_some()
            || !outcome.object_commands.is_empty()
            || outcome.destroy_object
            || outcome.environment.is_some()
            || outcome.physics.is_some()
            || !outcome.spawns.is_empty()
            || !outcome.particles.is_empty()
            || !outcome.transfer_zones.is_empty()
            || !outcome.messages.is_empty()
            || !outcome.audio.events.is_empty()
            || outcome.trigger_game_over
        {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries".to_string(),
                detail: "callback must not modify game state".to_string(),
            });
        }
        if !environment_delta.is_empty() || !physics_delta.is_empty() {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries".to_string(),
                detail: "callback must not modify global state".to_string(),
            });
        }

        let audio_state = audio_guard.finish();
        Ok((entries, audio_state, rng))
    }

    fn call_menu_command(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        kind: MenuCommandKind,
        selection: &MenuCommandSelection,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function("MenuCommand") {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args = [
            build_state_value(&self.id, object_id, state, &self.action_library),
            Value::String(kind.as_str().to_string().into()),
            build_menu_selection_value(selection),
        ];
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.time,
            state.action.data,
            state.action.phase,
            self.shared_action_library(&world),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_action_index(state.action.act_map_index)
        .with_shape_vertices(&state.shape_vertices)
        .with_definition_id(self.id.as_str())
        .with_alive(state.alive)
        .with_controller(state.controller)
        .with_in_liquid(state.in_liquid)
        .with_own_mass(state.own_mass)
        .with_physicals(
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            *self.physical(),
        )
        .with_base_graphics(state.base_graphics.clone())
        // The scope publishes its whole overlay list, so it must start
        // from the object's real overlays: C4Object::GetGraphicsOverlay
        // splices a single node (src/C4Object.cpp:5962-5977).
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_walk_rotation(self.walk_rotation_seed(state))
        .with_script_fixed_position(state.script_fixed_position)
        .with_script_fixed_velocity(state.script_fixed_velocity)
        .with_script_rotation_velocity(state.script_rotation_velocity)
        .with_script_fixed_rotation(state.script_fixed_rotation)
        .with_magic_energy(state.magic_energy)
        .with_breath(state.breath)
        .with_need_energy(state.need_energy)
        .with_ocf(state.ocf);
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || self.run_live_object_session("MenuCommand", &args, &state.local_vars, object_id),
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| {
            script_execution_error(self.id.clone(), "MenuCommand".to_string(), source, None)
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        // Same bool cast as C4Object::MenuCommand (C4Object.cpp:3736):
        // script results coerce by raw truthiness, never a type error.
        let handled = compat::value_raw_truthy(&value);

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((handled, host_effects, audio_state, rng))
    }

    fn call_control(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args: [Value; 0] = [];
        let (value, host_effects, audio_state, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Control",
            |script, cells, this| script.call_with_cells_and_this(function, &args, cells, this),
        )?;
        // C4Object::CallControl (C4Object.cpp:3300) evaluates the script
        // result with `static_cast<bool>(...)` — C4Value raw-data truthiness
        // (C4Value.h:76,183-185): only nil, 0 and false are unhandled. Real
        // control scripts return ints (`return(1)`), never bools.
        Ok((
            compat::value_raw_truthy(&value),
            host_effects,
            audio_state,
            rng,
        ))
    }

    /// Run one OUTER object VM call on LIVE local cells registered as the
    /// object's session with the host context: nested calls the host
    /// routes back onto this object (and cross-object Local/LocalN
    /// references) share the storage, so mid-call local writes are visible
    /// in both directions — C++ mutates the ONE live C4Object. Must run
    /// inside `with_effect_context_with_state`; the caller folds the final
    /// locals via the returned snapshot.
    fn run_live_object_session(
        &self,
        function: &str,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
        object_id: ObjectId,
    ) -> Result<(Value, HashMap<String, Value>), clonk_script::ScriptError> {
        self.run_live_object_callback_session(
            &ScriptCallbackTarget::unlinked(function),
            args,
            local_vars,
            object_id,
        )
    }

    fn run_live_object_callback_session(
        &self,
        callback: &ScriptCallbackTarget,
        args: &[Value],
        local_vars: &HashMap<String, Value>,
        object_id: ObjectId,
    ) -> Result<(Value, HashMap<String, Value>), clonk_script::ScriptError> {
        let cells = clonk_script::LocalCells::from_local_vars(local_vars);
        compat::register_session_local_cells(object_id, cells.clone());
        let result = match callback.resolution() {
            Some(resolution) => self.script.call_pinned_with_cells_and_this(
                resolution.function.as_ref(),
                resolution.scope == clonk_script::ScriptFunctionScope::Global,
                args,
                &cells,
                compat::object_reference_value(object_id),
            ),
            None => self.script.call_with_cells_and_this(
                callback.function_name(),
                args,
                &cells,
                compat::object_reference_value(object_id),
            ),
        };
        result.map(|value| (value, cells.snapshot()))
    }

    #[doc(hidden)]
    pub fn call_object_function(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        args: &[Value],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        self.call_object_callback(
            self,
            state,
            object_id,
            &ScriptCallbackTarget::unlinked(function),
            args,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn call_object_callback(
        &self,
        object_definition: &Definition,
        state: &ObjectState,
        object_id: ObjectId,
        callback: &ScriptCallbackTarget,
        args: &[Value],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if callback.resolution().is_none() && !self.script.has_function(callback.function_name()) {
            let next_object_id = world.next_object_id();
            return Ok((
                Value::Nil,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let arg_values: Vec<Value> = args.to_vec();
        let function = callback.function_name();
        self.exec_in_object_context_for_definition(
            object_definition,
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            function,
            |script, cells, this| match callback.resolution() {
                Some(resolution) => script.call_pinned_with_cells_and_this(
                    resolution.function.as_ref(),
                    resolution.scope == clonk_script::ScriptFunctionScope::Global,
                    &arg_values,
                    cells,
                    this,
                ),
                None => script.call_with_cells_and_this(function, &arg_values, cells, this),
            },
        )
    }

    /// Run C4Object::Incinerate inside the carrier's live host context so its
    /// C4Effect constructor shares the canonical AddEffect check/start path.
    #[allow(clippy::too_many_arguments)]
    fn incinerate_object(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        caused_by: i32,
        blasted: bool,
        incinerating: Option<ObjectId>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Incinerate",
            |_script, _cells, _this| {
                compat::incinerate_target(object_id, caused_by, blasted, incinerating)
                    .map(Value::Bool)
                    .map_err(Into::into)
            },
        )?;
        Ok((matches!(value, Value::Bool(true)), outcome, audio, rng))
    }

    /// Run C4Object::GetValue for C4Player::UpdateValue without exposing the
    /// host helper to script-name shadowing. CalcValue/CalcDefValue and
    /// construction scaling execute in the object's complete live context.
    #[allow(clippy::too_many_arguments)]
    fn player_asset_object_value(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        player: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(i32, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let args = [
            compat::object_reference_value(object_id),
            Value::Nil,
            Value::Nil,
            Value::Int(player),
        ];
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "GetValue",
            |_script, _cells, _this| compat::get_value(&args).map_err(Into::into),
        )?;
        Ok((value.as_c4_int().unwrap_or(0), outcome, audio, rng))
    }

    /// Run C4Def::GetValue for C4Command::Buy without exposing the host
    /// helper to script-name shadowing. CalcDefValue and the base's
    /// CalcBuyValue still execute in the actor's complete live host context.
    #[allow(clippy::too_many_arguments)]
    fn command_buy_value(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        item_definition: &str,
        base_id: ObjectId,
        buyer: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            Option<i32>,
            compat::EffectContextOutcome,
            AudioRegistry,
            LcgRng,
        ),
        EngineError,
    > {
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "GetValue",
            |_script, _cells, _this| {
                compat::calculated_definition_value(item_definition, Some(base_id), buyer)
                    .map(|price| price.map(Value::Int).unwrap_or(Value::Nil))
                    .map_err(Into::into)
            },
        )?;
        let price = match value {
            Value::Int(value) => Some(value),
            _ => None,
        };
        Ok((price, outcome, audio, rng))
    }

    /// Invoke the native FnBuy/C4Player::Buy path directly so a script
    /// function named Buy cannot shadow the engine operation.
    #[allow(clippy::too_many_arguments)]
    fn command_buy_item(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        item_definition: &str,
        buyer: i32,
        payer: i32,
        base_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let args = [
            Value::C4Id(item_definition.to_string()),
            Value::Int(buyer),
            Value::Int(payer),
            compat::object_reference_value(base_id),
            Value::Bool(false),
        ];
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Buy",
            |_script, _cells, _this| compat::buy(&args).map_err(Into::into),
        )?;
        Ok((matches!(value, Value::Object(_)), outcome, audio, rng))
    }

    /// Invoke C4Player::Sell2Home directly so a script function named Sell
    /// cannot shadow the command's recursive native transaction.
    #[allow(clippy::too_many_arguments)]
    fn command_sell_item(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        sold_object: ObjectId,
        base_owner: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "Sell",
            |_script, _cells, _this| {
                compat::sell_object_to_home_live(sold_object, base_owner)
                    .map(Value::Bool)
                    .map_err(Into::into)
            },
        )?;
        Ok((matches!(value, Value::Bool(true)), outcome, audio, rng))
    }

    /// The shared object-context execution seam: installs the physics/
    /// environment/random/audio guards and the HostObjectContext, runs
    /// `invoke` on the definition script against LIVE local cells
    /// registered as the object's session — nested calls the host routes
    /// back onto the same object share the storage, so mid-call local
    /// writes are visible in both directions (C++ mutates the ONE live
    /// C4Object; its object locals are by-reference) — and folds the
    /// outcome. The body every `call_object_function`-style entry shares.
    #[allow(clippy::too_many_arguments)]
    fn exec_in_object_context<F>(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
        label: &str,
        invoke: F,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError>
    where
        F: FnOnce(
            &clonk_script::Engine,
            &clonk_script::LocalCells,
            Value,
        ) -> Result<Value, clonk_script::ScriptError>,
    {
        self.exec_in_object_context_for_definition(
            self,
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            label,
            invoke,
        )
    }

    /// Execute a callback owned by `self` against an object whose live
    /// definition may differ. Native retained function pointers preserve
    /// their original script host while `Exec(pObj, ...)` derives `this`,
    /// object locals and object metadata from the supplied receiver.
    #[allow(clippy::too_many_arguments)]
    fn exec_in_object_context_for_definition<F>(
        &self,
        object_definition: &Definition,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
        label: &str,
        invoke: F,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError>
    where
        F: FnOnce(
            &clonk_script::Engine,
            &clonk_script::LocalCells,
            Value,
        ) -> Result<Value, clonk_script::ScriptError>,
    {
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.time,
            state.action.data,
            state.action.phase,
            object_definition.shared_action_library(&world),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            object_definition.ocf_base,
            object_definition.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_action_index(state.action.act_map_index)
        .with_shape_vertices(&state.shape_vertices)
        .with_definition_id(object_definition.id.as_str())
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_base_graphics(state.base_graphics.clone())
        .with_alive(state.alive)
        .with_controller(state.controller)
        .with_in_liquid(state.in_liquid)
        .with_own_mass(state.own_mass)
        .with_physicals(
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            *object_definition.physical(),
        )
        .with_walk_rotation(object_definition.walk_rotation_seed(state))
        .with_script_fixed_position(state.script_fixed_position)
        .with_script_fixed_velocity(state.script_fixed_velocity)
        .with_script_rotation_velocity(state.script_rotation_velocity)
        .with_script_fixed_rotation(state.script_fixed_rotation)
        .with_magic_energy(state.magic_energy)
        .with_breath(state.breath)
        .with_need_energy(state.need_energy)
        .with_ocf(state.ocf);
        let cells = clonk_script::LocalCells::from_local_vars(&state.local_vars);
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                compat::register_session_local_cells(object_id, cells.clone());
                invoke(
                    &self.script,
                    &cells,
                    compat::object_reference_value(object_id),
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        // The outcome folds for BOTH exits: on a script error the partial
        // outcome — everything mutated before the unwind, including the
        // cells' local writes — travels as the error's recovery payload
        // (C4AulExec aborts the call but rolls nothing back,
        // C4AulExec.cpp:1318-1342).
        // Store updated local variables — the final cell snapshot carries
        // nested-call writes too (shared live storage).
        let updated_local_vars = cells.snapshot();
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        let value = match result {
            Ok(value) => value,
            Err(source) => {
                return Err(script_execution_error(
                    self.id.clone(),
                    label.to_string(),
                    source,
                    Some(Box::new(ScriptCallRecovery {
                        outcome: host_effects,
                        audio: audio_state,
                        rng,
                    })),
                ));
            }
        };
        Ok((value, host_effects, audio_state, rng))
    }

    /// `C4Object::GetInfoString`'s effect walk, executed in the target's
    /// live object context so `Fx*Info` callbacks retain their normal side
    /// effects, RNG, audio, local-variable, and nested-object semantics.
    #[allow(clippy::too_many_arguments)]
    fn call_object_effect_info(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<
        (
            Vec<String>,
            compat::EffectContextOutcome,
            AudioRegistry,
            LcgRng,
        ),
        EngineError,
    > {
        let effects = state.effects.clone();
        let (value, outcome, audio, rng) = self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            "GetInfoString",
            |_script, _cells, _this| {
                Ok(Value::Array(
                    compat::object_effect_info_lines(object_id, &effects)
                        .into_iter()
                        .map(Value::from)
                        .collect(),
                ))
            },
        )?;
        let lines = match value {
            Value::Array(values) => values
                .into_iter()
                .filter_map(|value| match value {
                    Value::String(line) => Some(line.into_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        Ok((lines, outcome, audio, rng))
    }

    /// C4AulScript::DirectExec in this definition's object context — the
    /// C4Object::MenuCommand seam (C4Object.cpp:3756-3760): `source` runs
    /// as ONE expression with the object's locals and `this`.
    #[allow(clippy::too_many_arguments)]
    fn direct_exec_object_expression(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        source: &str,
        label: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            label,
            |script, cells, this| {
                script.direct_exec_with_cells_and_this_in_context_diagnostics(
                    source,
                    cells,
                    this,
                    label,
                    is_cpp_direct_exec_context(label),
                )
            },
        )
    }

    /// Synchronized-control DirectExec variant. Unlike object-menu commands,
    /// the temporary expression script carries strictness on the packet.
    #[allow(clippy::too_many_arguments)]
    fn direct_exec_object_expression_at_strict(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        source: &str,
        label: &str,
        strict_level: Option<u8>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(Value, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        self.exec_in_object_context(
            state,
            object_id,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
            label,
            |script, cells, this| {
                script.direct_exec_with_cells_and_this_at_strict_in_context_diagnostics(
                    source,
                    cells,
                    this,
                    strict_level,
                    label,
                    is_cpp_direct_exec_context(label),
                )
            },
        )
    }

    fn call_menu_callback(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        function: &str,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(bool, compat::EffectContextOutcome, AudioRegistry, LcgRng), EngineError> {
        if !self.script.has_function(function) {
            let next_object_id = world.next_object_id();
            return Ok((
                false,
                compat::EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
            ));
        }

        let args = [build_state_value(
            &self.id,
            object_id,
            state,
            &self.action_library,
        )];
        let args_call = args.clone();
        let function_name = function.to_string();
        let function_call = function_name.clone();
        let local_vars_call = state.local_vars.clone();
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let object_context = compat::HostObjectContext::with_category(
            object_id,
            state.container,
            state.status,
            state.energy,
            state.damage,
            state.construction,
            state.owner,
            state.position,
            state.velocity,
            state.rotation,
            &state.effects,
            state.action.name.clone(),
            state.action.time,
            state.action.data,
            state.action.phase,
            self.shared_action_library(&world),
            state.direction,
            state.command_direction,
            0,
            state.action.target,
            state.action.target2,
            &state.vertices,
            state.category,
            self.ocf_base,
            self.crew_member,
            state.draw_transform,
            state.base_graphics.clone(),
        )
        .with_action_index(state.action.act_map_index)
        .with_shape_vertices(&state.shape_vertices)
        .with_definition_id(self.id.as_str())
        .with_alive(state.alive)
        .with_controller(state.controller)
        .with_in_liquid(state.in_liquid)
        .with_own_mass(state.own_mass)
        .with_physicals(
            state.info_physical,
            state.temporary_physical,
            state.physical_changes.clone(),
            *self.physical(),
        )
        .with_base_graphics(state.base_graphics.clone())
        // The scope publishes its whole overlay list, so it must start
        // from the object's real overlays: C4Object::GetGraphicsOverlay
        // splices a single node (src/C4Object.cpp:5962-5977).
        .with_graphics_overlays(state.graphics_overlays.clone())
        .with_walk_rotation(self.walk_rotation_seed(state))
        .with_script_fixed_position(state.script_fixed_position)
        .with_script_fixed_velocity(state.script_fixed_velocity)
        .with_script_rotation_velocity(state.script_rotation_velocity)
        .with_script_fixed_rotation(state.script_fixed_rotation)
        .with_magic_energy(state.magic_energy)
        .with_breath(state.breath)
        .with_need_energy(state.need_energy)
        .with_ocf(state.ocf);
        let (result, mut host_effects) = compat::with_effect_context_with_state(
            Some(object_context),
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            move || {
                self.run_live_object_session(
                    &function_call,
                    &args_call,
                    &local_vars_call,
                    object_id,
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let (value, updated_local_vars) = result.map_err(|source| {
            script_execution_error(
                format!("{}::{}", self.id, function),
                "MenuCallback".to_string(),
                source,
                None,
            )
        })?;
        // Store updated local variables
        if let Some(object_update) = &mut host_effects.object_update {
            object_update.local_vars = Some(updated_local_vars);
        } else {
            let mut update = ObjectUpdate::default();
            update.local_vars = Some(updated_local_vars);
            host_effects.object_update = Some(update);
        }
        // C4Object::MenuCommand (C4Object.cpp:3732-3736): the executed menu
        // function's result is `static_cast<bool>(DirectExec(...))` — raw
        // C4Value truthiness; real context functions return ints.
        let handled = compat::value_raw_truthy(&value);

        if !environment_delta.is_empty() {
            host_effects.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            host_effects.physics = Some(physics_delta);
        }

        let audio_state = audio_guard.finish();
        Ok((handled, host_effects, audio_state, rng))
    }

    fn parse_context_menu_entries(
        &self,
        value: Value,
    ) -> Result<Vec<ContextMenuEntry>, EngineError> {
        let Value::Array(entries) = value else {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.id.clone(),
                function: "MenuEntries".to_string(),
                detail: format!("expected array (got {})", value.type_name()),
            });
        };

        let mut result = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            let Value::Proplist(props) = entry else {
                return Err(EngineError::InvalidScriptOutput {
                    definition: self.id.clone(),
                    function: "MenuEntries".to_string(),
                    detail: format!(
                        "entry {index} must be a proplist (got {})",
                        entry.type_name()
                    ),
                });
            };

            let label = match props.get("label") {
                Some(Value::String(text)) if !text.is_empty() => text.to_string(),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!(
                            "entry {index} field `label` must be non-empty string (got {})",
                            other.type_name()
                        ),
                    });
                }
                None => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!("entry {index} missing required field `label`"),
                    });
                }
            };

            let function = match props.get("callback") {
                Some(Value::String(name)) if !name.is_empty() => name.to_string(),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!(
                            "entry {index} field `callback` must be non-empty string (got {})",
                            other.type_name()
                        ),
                    });
                }
                None => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!("entry {index} missing required field `callback`"),
                    });
                }
            };

            let description = match props.get("description") {
                Some(Value::String(text)) if text.is_empty() => None,
                Some(Value::String(text)) => Some(text.to_string()),
                Some(other) => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: self.id.clone(),
                        function: "MenuEntries".to_string(),
                        detail: format!(
                            "entry {index} field `description` must be string (got {})",
                            other.type_name()
                        ),
                    });
                }
                None => None,
            };

            result.push(ContextMenuEntry {
                function,
                label,
                description,
            });
        }

        Ok(result)
    }

    fn call_effect_start(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        constructor_values: &[Value; 4],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        // C4Effect's constructor calls Fx*Start with iTemp=0 followed by
        // the four rVal arguments supplied to AddEffect
        // (C4Effect.cpp:118-129). Deferred object starts must retain the
        // same argument list as the synchronous priority-one/global path.
        let mut extras = vec![Value::Int(0)];
        extras.extend(constructor_values.iter().cloned());
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Start",
            "FxStart",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    fn call_effect_timer(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        frame: u64,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Timer",
            "FxTimer",
            vec![Value::Int(effect.timer)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// `Fx<Name>Damage` (C4Effect.cpp:312-322): the effect is asked to
    /// modify a damage/energy change before it applies.
    #[allow(clippy::too_many_arguments)]
    fn call_effect_damage(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        change: i32,
        cause: i32,
        caused_by: i32,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Damage",
            "FxDamage",
            vec![Value::Int(change), Value::Int(cause), Value::Int(caused_by)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// `Fx<Name>Effect` check call (C4Effect.cpp:280-282): the checker
    /// effect is asked about a pending new effect — the callback receives
    /// the new name plus the AddEffect rVal1-4 (C++ passes them at
    /// positions 5-8; the deferred convention appends them after the name).
    #[allow(clippy::too_many_arguments)]
    fn call_effect_effect(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        checker: &EffectState,
        pending: &EffectState,
        constructor_values: &[Value; 4],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let mut extras = vec![Value::String(pending.name.clone().into())];
        // rVal1-4 (C4Effect.cpp:282): always four slots, missing = nil.
        extras.extend(constructor_values.iter().cloned());
        self.dispatch_effect_callback(
            carrier,
            checker,
            "Effect",
            "FxEffect",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// `Fx<Name>Add` merge seam (DoCall PSFS_FxAdd, C4Effect.cpp:300-301):
    /// the ACCEPTOR effect receives the annulled new effect's name, timer
    /// interval and the first four AddEffect parameters.
    #[allow(clippy::too_many_arguments)]
    fn call_effect_add(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        acceptor: &EffectState,
        pending: &EffectState,
        constructor_values: &[Value; 4],
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let mut extras = vec![
            Value::String(pending.name.clone().into()),
            Value::Int(pending.interval),
        ];
        // rVal1-4 (C4Effect.cpp:301): always four slots, missing = nil.
        extras.extend(constructor_values.iter().cloned());
        self.dispatch_effect_callback(
            carrier,
            acceptor,
            "Add",
            "FxAdd",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    fn call_effect_stop(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        reason: EffectStopReason,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let mut extras = vec![effect_stop_reason_value(reason)];
        if matches!(reason, EffectStopReason::Temp) {
            // fTemp = true (TempRemoveUpperEffects, C4Effect.cpp:489).
            extras.push(Value::Bool(true));
        }
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Stop",
            "FxStop",
            extras,
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    /// Reactivate a temp-removed effect (TempReaddUpperEffects,
    /// C4Effect.cpp:505): Fx*Start with iTemp = C4FxCall_Temp
    /// (C4Effects.h:47); the result is ignored like C++.
    #[allow(clippy::too_many_arguments)]
    fn call_effect_temp_readd(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        self.dispatch_effect_callback(
            carrier,
            effect,
            "Start",
            "FxStart",
            vec![Value::Int(1)],
            rng,
            global_effects,
            physics,
            environment,
            frame,
            world,
            game_over_triggered,
            audio,
        )
    }

    fn dispatch_effect_callback(
        &self,
        carrier: Option<(&ObjectState, ObjectId)>,
        effect: &EffectState,
        event: &'static str,
        function_label: &'static str,
        mut extras: Vec<Value>,
        rng: LcgRng,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        frame: u64,
        world: HostWorldContext,
        game_over_triggered: bool,
        audio: AudioRegistry,
    ) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
        let next_object_id = world.next_object_id();
        let callback_name = format!("Fx{}{}", effect.name, event);
        let callback = resolve_effect_script_callback(effect, &callback_name, &world);
        // AddFunc registers the engine's FxFireStart below script functions.
        // C4Effect therefore calls the native body only when script lookup did
        // not find an override; inherited() from an override already reaches
        // the registered host through the ordinary VM path.
        let native_fire_start = callback.is_none() && effect.name == C4FX_FIRE && event == "Start";
        if callback.is_none() && !native_fire_start {
            return Ok((
                EffectContextOutcome::empty(next_object_id, audio.clone()),
                audio,
                rng,
                None,
            ));
        }
        // Fx callbacks resolve CODE on the effect command target/id, but
        // pForObj remains the affected object's real C4Object. Its ActMap,
        // physicals, OCF metadata and definition id therefore come from the
        // carrier, never from `self` merely because `self` owns the callback
        // script (C4Effect.cpp:42-57,128-129,342-345).
        let carrier_definition_id = carrier.and_then(|(_, object_id)| {
            world
                .get(object_id)
                .map(|object| object.definition_id().to_string())
        });
        let carrier_metadata = carrier_definition_id
            .as_deref()
            .and_then(|id| world.definition_metadata(id))
            .cloned();

        let mut args = Vec::with_capacity(2 + extras.len());
        // The affected object is the first argument — C++ passes nullptr
        // (nil) for GLOBAL effects (C4Effect::Execute, C4Effect.cpp:345).
        args.push(
            carrier
                .map(|(state, object_id)| {
                    if self.c4_callback_args {
                        compat::object_reference_value(object_id)
                    } else {
                        build_state_value(
                            carrier_definition_id.as_deref().unwrap_or(&self.id),
                            object_id,
                            state,
                            carrier_metadata
                                .as_ref()
                                .map(|metadata| &*metadata.action_library)
                                .unwrap_or(&self.action_library),
                        )
                    }
                })
                .unwrap_or(Value::Nil),
        );
        args.push(build_effect_value(effect));
        args.append(&mut extras);

        // Effect callbacks execute on the effect's command target
        // (pFn->Exec(pCommandTarget, ...), C4Effect.cpp:129,345,392,456):
        // `this()` is the command target and its object locals are live.
        // The affected carrier and command target may be different objects,
        // so copy locals from the command target's world snapshot unless it
        // is the carrier whose threaded event snapshot is newer.
        let context_object = callback
            .as_ref()
            .and_then(|callback| callback.command_object)
            .or_else(|| {
                native_fire_start
                    .then(|| {
                        effect
                            .command_target
                            .map(|target| ObjectId::new(target as u64))
                    })
                    .flatten()
                    .filter(|target| world.get(*target).is_some())
            });
        let context_is_self =
            carrier.is_some_and(|(_, object_id)| context_object == Some(object_id));
        let context_this = context_object
            .map(compat::object_reference_value)
            .unwrap_or(Value::Nil);
        let context_locals = if context_is_self {
            carrier
                .map(|(state, _)| state.local_vars.clone())
                .unwrap_or_default()
        } else {
            context_object
                .and_then(|object_id| world.get(object_id))
                .and_then(|object| object.full_state().map(|state| state.local_vars.clone()))
                .unwrap_or_default()
        };
        // The callback's LIVE local cells: registered as the object's
        // session so nested calls / cross-object references back onto it
        // share the storage (C++ mutates the one live C4Object).
        let context_cells = clonk_script::LocalCells::from_local_vars(&context_locals);

        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, frame);
        let guard = enter_random_context(rng);
        let audio_guard = enter_audio_context(audio);
        let callback_definition_context = callback
            .as_ref()
            .and_then(|callback| callback.definition_context.clone())
            .or_else(|| {
                native_fire_start
                    .then(|| {
                        context_object.and_then(|object| {
                            world
                                .get(object)
                                .map(|object| DefinitionId::from(object.definition_id()))
                        })
                    })
                    .flatten()
            });
        let (result, mut commands) = compat::with_effect_context_with_state_and_definition(
            carrier.map(|(state, object_id)| {
                let carrier_walk_rotation = carrier_metadata
                    .as_ref()
                    .map(|metadata| compat::WalkRotationSeed {
                        rotateable: metadata.rotateable,
                        t_attach: state.t_attach,
                        attach: state.shape_attach,
                        def_attach_vtx_x: usize::try_from(state.shape_attach.vtx)
                            .ok()
                            .and_then(|vtx| metadata.vertices.get(vtx))
                            .map(|vertex| vertex.x)
                            .unwrap_or(0),
                    })
                    .unwrap_or_else(|| self.walk_rotation_seed(state));
                compat::HostObjectContext::with_category(
                    object_id,
                    state.container,
                    state.status,
                    state.energy,
                    state.damage,
                    state.construction,
                    state.owner,
                    state.position,
                    state.velocity,
                    state.rotation,
                    &state.effects,
                    state.action.name.clone(),
                    state.action.time,
                    state.action.data,
                    state.action.phase,
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.action_library.clone())
                        .unwrap_or_else(|| self.shared_action_library(&world)),
                    state.direction,
                    state.command_direction,
                    0,
                    state.action.target,
                    state.action.target2,
                    &state.vertices,
                    state.category,
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.ocf_base)
                        .unwrap_or(self.ocf_base),
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.crew_member)
                        .unwrap_or(self.crew_member),
                    state.draw_transform,
                    state.base_graphics.clone(),
                )
                .with_action_index(state.action.act_map_index)
                .with_shape_vertices(&state.shape_vertices)
                .with_definition_id(carrier_definition_id.as_deref().unwrap_or(self.id.as_str()))
                .with_alive(state.alive)
                .with_controller(state.controller)
                .with_in_liquid(state.in_liquid)
                .with_own_mass(state.own_mass)
                .with_physicals(
                    state.info_physical,
                    state.temporary_physical,
                    state.physical_changes.clone(),
                    carrier_metadata
                        .as_ref()
                        .map(|metadata| metadata.physical)
                        .unwrap_or(*self.physical()),
                )
                .with_base_graphics(state.base_graphics.clone())
                // C4Object::GetGraphicsOverlay splices a single node into the
                // live pGfxOverlay list (src/C4Object.cpp:5962-5977), so an
                // effect callback that writes one overlay leaves the object's
                // other overlays alone. The scope publishes its WHOLE overlay
                // list (compat/contexts.rs:8892), so it must start from the
                // carrier's real overlays or the write deletes the rest.
                .with_graphics_overlays(state.graphics_overlays.clone())
                .with_walk_rotation(carrier_walk_rotation)
                .with_script_fixed_position(state.script_fixed_position)
                .with_script_fixed_velocity(state.script_fixed_velocity)
                .with_script_rotation_velocity(state.script_rotation_velocity)
                .with_script_fixed_rotation(state.script_fixed_rotation)
                .with_magic_energy(state.magic_energy)
                .with_breath(state.breath)
                .with_need_energy(state.need_energy)
                .with_ocf(state.ocf)
            }),
            callback_definition_context,
            context_object,
            global_effects,
            world,
            next_object_id,
            game_over_triggered,
            || {
                if let Some(session_id) = context_object {
                    compat::register_session_local_cells(session_id, context_cells.clone());
                }
                if native_fire_start {
                    return compat::fx_fire_start(&args)
                        .map(|value| Some((value, context_cells.snapshot())))
                        .map_err(ScriptError::from);
                }
                let callback = callback
                    .as_ref()
                    .expect("script callback exists when native fallback is inactive");
                if callback.engine_global_entry {
                    if context_object.is_some() {
                        return callback
                            .script
                            .call_pinned_with_cells_and_this(
                                &callback.resolution.function,
                                true,
                                &args,
                                &context_cells,
                                context_this,
                            )
                            .map(|value| Some((value, context_cells.snapshot())));
                    }
                    return callback
                        .script
                        .call_pinned_with_ref_args(&callback.resolution.function, true, &args)
                        .map(|(value, _)| Some((value, HashMap::new())));
                }
                if context_object.is_some() {
                    return callback.script.call_effect_callback_in_context_with_cells(
                        &effect.name,
                        event,
                        &args,
                        &context_cells,
                        context_this,
                    );
                }
                callback.script.call_effect_callback_in_context(
                    &effect.name,
                    event,
                    &args,
                    &context_locals,
                    context_this,
                )
            },
        );
        let rng = guard.finish();
        let physics_delta = physics_guard.finish();
        let environment_delta = env_guard.finish();
        let audio_state = audio_guard.finish();

        let callback_result = recover_effect_callback_error(
            result,
            &context_cells,
            format!("{}::{}::{}", self.id, effect.name, function_label),
        )?;
        if !environment_delta.is_empty() {
            commands.environment = Some(environment_delta);
        }
        if !physics_delta.is_empty() {
            commands.physics = Some(physics_delta);
        }
        let callback_result = callback_result.map(|(value, updated_locals)| {
            if context_is_self {
                commands.context_locals = Some(updated_locals);
            } else if let Some(context_object) = context_object {
                append_effect_command_target_locals(&mut commands, context_object, updated_locals);
            }
            value
        });
        Ok((commands, audio_state, rng, callback_result))
    }
}

#[cfg(test)]
#[test]
fn collection_ocf_scalar_overrides_match_materialized_state() -> Result<(), EngineError> {
    let mut definition = Definition::from_script("COLP", "Collection preview", "")?;
    definition.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
    definition.set_collection_limit(2);
    let mut state = preview_spawn_state(
        Vector2::ZERO,
        OWNER_NONE,
        OWNER_NONE,
        DEFAULT_CATEGORY,
        FULL_CON,
        CONTACT_DENSITY_SOLID,
        Vec::new(),
    );
    state.contents = vec![ObjectId::new(1), ObjectId::new(2)];
    state.no_collect_delay = 2;

    assert!(!definition.collection_ocf_enabled(&state, state.contents.len(), 0));
    assert!(definition.collection_ocf_enabled(&state, 0, 0));
    assert!(!definition.collection_ocf_enabled(&state, 0, state.no_collect_delay));
    for (contents_len, no_collect_delay) in [(state.contents.len(), 0), (0, 0)] {
        let preview = definition.collection_ocf_enabled(&state, contents_len, no_collect_delay);
        let mut materialized = state.clone();
        materialized.contents.truncate(contents_len);
        materialized.no_collect_delay = no_collect_delay;
        assert_eq!(
            preview,
            definition.compute_ocf(&materialized) & crate::ocf::COLLECTION != 0,
        );
    }
    assert_eq!(state.contents.len(), 2, "preview leaves Contents untouched");
    assert_eq!(state.no_collect_delay, 2, "preview leaves delay untouched");

    state.construction = FULL_CON - 1;
    assert!(!definition.collection_ocf_enabled(&state, 0, 0));
    definition.set_ocf_base(crate::ocf::FULL_CON);
    assert!(definition.collection_ocf_enabled(&state, 0, 0));

    definition.set_ocf_base(crate::ocf::COLLECTION);
    definition.set_collection_rect(None);
    let seeded =
        definition.collection_ocf_enabled(&state, state.contents.len(), state.no_collect_delay);
    assert!(seeded, "raw OCF fixture seed bypasses dynamic gates");
    assert_eq!(
        seeded,
        definition.compute_ocf(&state) & crate::ocf::COLLECTION != 0,
    );
    Ok(())
}

struct ScenarioScript {
    name: String,
    /// C++ callback convention: no synthetic state argument, no fixture
    /// Step calls (real content; see ScenarioScriptSource::c4_args).
    c4_args: bool,
    /// Shared like `Definition.script`: `host_world_context()` hands `Arc`
    /// clones to host functions so GameCall/GameCallEx can run scenario
    /// functions mid-VM-call.
    script: Arc<ScriptEngine>,
    /// Pristine scenario host used to discard linked copies during ReLink.
    base_script: clonk_script::Script,
    /// Whether `base_script`'s definition includes have been copied into the
    /// live scenario host for the current link pass.
    includes_resolved: bool,
    has_initialize: bool,
    has_step: bool,
}

impl ScenarioScript {
    fn from_source(name: impl Into<String>, source: &str) -> Result<Self, EngineError> {
        let name = name.into();
        let mut script = ScriptEngine::new();
        script.set_script_name(name.clone());
        script.set_game_script_name(name.clone());
        let compiled = clonk_script::Script::compile_c4_string(source).map_err(|source| {
            EngineError::Script {
                definition: name.clone(),
                function: "load".to_string(),
                source: ScriptError::from(source),
                recovery: None,
            }
        })?;
        for diagnostic in compiled.parse_diagnostics() {
            tracing::warn!(
                script = %name,
                %diagnostic,
                "scenario script parse error quarantined; continuing like C++"
            );
        }
        script.add_script(compiled.clone());
        compat::register_host_functions(&mut script);
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        #[allow(clippy::arc_with_non_send_sync)] // single-threaded sharing
        Ok(Self {
            name,
            c4_args: false,
            script: Arc::new(script),
            base_script: compiled,
            includes_resolved: false,
            has_initialize,
            has_step,
        })
    }

    /// Shared handle for the world context (GameCall resolution).
    fn script_arc(&self) -> Arc<ScriptEngine> {
        Arc::clone(&self.script)
    }

    /// Shares the System.c4g global-function table into the scenario host.
    fn set_global_functions(
        &mut self,
        functions: Option<Arc<HashMap<String, clonk_script::Function>>>,
    ) {
        Arc::make_mut(&mut self.script).set_global_functions(functions);
    }

    fn reset_script_links(&mut self) {
        Arc::make_mut(&mut self.script).replace_script_deferred(self.base_script.clone(), false);
        self.includes_resolved = false;
        self.has_initialize = self.script.has_function("Initialize");
        self.has_step = self.script.has_function("Step");
    }

    fn refresh_script_flags(&mut self) {
        self.has_initialize = self.script.has_function("Initialize");
        self.has_step = self.script.has_function("Step");
    }

    fn local_function_names(&self) -> HashSet<String> {
        self.script
            .functions()
            .keys()
            .filter(|name| self.script.has_local_function(name))
            .cloned()
            .collect()
    }

    /// `C4AulScript::GetSFunc(index)` walks the linked function list from its
    /// tail, including append/include copies in their resolved positions.
    fn local_function_names_in_get_sfunc_order(&self) -> Vec<String> {
        self.script
            .local_functions_in_get_sfunc_order()
            .map(|(name, _)| name.clone())
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn initialize(
        &mut self,
        snapshot: &SimulationSnapshot,
        world: HostWorldContext,
        scoreboard: Rc<RefCell<ScoreboardState>>,
        materials: Rc<MaterialSet>,
        rng: LcgRng,
        random: i32,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        particle_defs: HashSet<String>,
        definition_scripts: HashMap<DefinitionId, Arc<ScriptEngine>>,
        definition_metadata: Rc<HashMap<DefinitionId, compat::DefinitionMetadata>>,
        definition_order: Rc<Vec<DefinitionId>>,
        network_game: bool,
        engine_next_object_id: u64,
        scenario_script_counter: i32,
    ) -> Result<(ScenarioBatch, AudioRegistry, LcgRng, Option<EngineError>), EngineError> {
        if !self.has_initialize {
            return Ok((ScenarioBatch::default(), audio, rng, None));
        }
        // C++: Game.Script.Call(PSF_Initialize) has NO parameters; the
        // state/random pair is the JSON-fixture convention.
        let mut args = Vec::with_capacity(2);
        if !self.c4_args {
            args.push(build_scenario_state_value(snapshot));
            args.push(Value::Int(random));
        }
        self.call_raw(
            "Initialize",
            args,
            snapshot,
            world,
            scoreboard,
            materials,
            rng,
            snapshot.frame,
            global_effects,
            physics,
            environment,
            audio,
            particle_defs,
            definition_scripts,
            definition_metadata,
            definition_order,
            network_game,
            engine_next_object_id,
            scenario_script_counter,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        snapshot: &SimulationSnapshot,
        world: HostWorldContext,
        scoreboard: Rc<RefCell<ScoreboardState>>,
        materials: Rc<MaterialSet>,
        rng: LcgRng,
        random: i32,
        frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        particle_defs: HashSet<String>,
        definition_scripts: HashMap<DefinitionId, Arc<ScriptEngine>>,
        definition_metadata: Rc<HashMap<DefinitionId, compat::DefinitionMetadata>>,
        definition_order: Rc<Vec<DefinitionId>>,
        network_game: bool,
        engine_next_object_id: u64,
        scenario_script_counter: i32,
    ) -> Result<(ScenarioBatch, AudioRegistry, LcgRng), EngineError> {
        if !self.has_step {
            return Ok((ScenarioBatch::default(), audio, rng));
        }
        let mut args = Vec::with_capacity(3);
        args.push(build_scenario_state_value(snapshot));
        let truncated = if frame > i32::MAX as u64 {
            i32::MAX
        } else {
            frame as i32
        };
        args.push(Value::Int(truncated));
        args.push(Value::Int(random));
        match self.call_raw(
            "Step",
            args,
            snapshot,
            world,
            scoreboard,
            materials,
            rng,
            frame,
            global_effects,
            physics,
            environment,
            audio,
            particle_defs,
            definition_scripts,
            definition_metadata,
            definition_order,
            network_game,
            engine_next_object_id,
            scenario_script_counter,
        ) {
            Ok((batch, audio, rng, None)) => Ok((batch, audio, rng)),
            // Strict wrappers surface the script error (fixtures assert
            // on it); the partial batch is dropped like before.
            Ok((_, _, _, Some(error))) => Err(error),
            Err(error) => Err(error),
        }
    }

    fn has_function(&self, function: &str) -> bool {
        self.script.has_function(function)
    }

    #[allow(clippy::too_many_arguments)]
    fn call_raw(
        &mut self,
        function: &str,
        args: Vec<Value>,
        snapshot: &SimulationSnapshot,
        world: HostWorldContext,
        scoreboard: Rc<RefCell<ScoreboardState>>,
        materials: Rc<MaterialSet>,
        rng: LcgRng,
        env_frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        particle_defs: HashSet<String>,
        definition_scripts: HashMap<DefinitionId, Arc<ScriptEngine>>,
        definition_metadata: Rc<HashMap<DefinitionId, compat::DefinitionMetadata>>,
        definition_order: Rc<Vec<DefinitionId>>,
        network_game: bool,
        engine_next_object_id: u64,
        scenario_script_counter: i32,
    ) -> Result<(ScenarioBatch, AudioRegistry, LcgRng, Option<EngineError>), EngineError> {
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, env_frame);
        let guard = enter_random_context(rng);
        // Definition scripts ride along so nested calls (obj->Method on
        // objects the scenario script creates or finds) resolve like the
        // broadcast path (FindSameNameFunc on the target's def).
        // The snapshot world's metadata table only knows categories — the
        // ENGINE's full table (shapes, physicals, ActMaps) rides along so
        // CreateObject applies the DoCon bottom adjust and OCF correctly.
        // The snapshot world derives max(live ids)+1 — that RE-MINTS the
        // number of any object created and removed earlier this frame.
        // C4Game::NewObj's ObjectEnumerationIndex only ever increments, so
        // the engine's persistent allocator is authoritative (the GoldRush
        // intro _TLK collided with a burned same-frame FXU1 id here).
        let world = world
            .with_scoreboard(scoreboard)
            .with_materials(Some(materials))
            .with_definition_metadata(definition_metadata)
            .with_definition_order(definition_order)
            .with_particle_defs(particle_defs)
            .with_definition_scripts(definition_scripts)
            .with_network_game(network_game)
            .with_scenario_script_counter(scenario_script_counter);
        let next_object_id = world.next_object_id().max(engine_next_object_id);
        let world = world.with_next_object_id(next_object_id);
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = compat::with_effect_context_with_state(
            None,
            global_effects,
            world,
            next_object_id,
            snapshot.game_over,
            || self.script.call(function, &args),
        );
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        // C++ mutates live state as the script runs: an error only aborts
        // the CONTINUATION — everything staged before it stands
        // (C4AulExec fail-safe). The host-side batch folds regardless;
        // the error rides along for the caller to log/propagate.
        let (result, script_error) = match result {
            Ok(value) => (value, None),
            Err(source) => (
                Value::Nil,
                Some(script_execution_error(
                    self.name.clone(),
                    function.to_string(),
                    source,
                    None,
                )),
            ),
        };

        let compat::EffectContextOutcome {
            object: host_object_effects,
            global: host_global_effects,
            object_update,
            object_commands,
            command_operations,
            command_events: _,
            destroy_object,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id: _,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;

        if !host_object_effects.is_empty()
            || object_update.is_some()
            || !object_commands.is_empty()
            || !command_operations.is_empty()
            || destroy_object
        {
            return Err(EngineError::InvalidScriptOutput {
                definition: self.name.clone(),
                function: function.to_string(),
                detail: "scenario scripts may not enqueue object commands".into(),
            });
        }

        let mut batch = parse_scenario_command(&self.name, function, result)?;
        batch.other_objects.extend(other_objects);
        if !host_player_commands.is_empty() {
            batch.player_commands.extend(host_player_commands);
        }
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);
        if !host_global_effects.is_empty() {
            batch.global_effects.extend(host_global_effects);
        }
        if !host_landscape_ops.is_empty() {
            batch.landscape_ops.extend(host_landscape_ops);
        }
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview.0 = host_raster_preview;
        if let Some(delta) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &delta);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        if !host_spawns.is_empty() {
            batch.spawns.extend(host_spawns);
        }
        if !host_particles.is_empty() {
            batch.particles.extend(host_particles);
        }
        if !host_transfer_zones.is_empty() {
            batch.transfer_zones.extend(host_transfer_zones);
        }
        if !host_messages.is_empty() {
            batch.messages.extend(host_messages);
        }
        if !host_audio.events.is_empty() {
            batch.audio.extend(host_audio.events);
        }
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        Ok((batch, audio_state, rng, script_error))
    }

    /// Raw-value, no-object-context call used by definition initialization
    /// and mrfScript. `fPassErrors=false` semantics: errors yield `None`
    /// while host side effects made before the error still fold into the
    /// returned batch.
    #[allow(clippy::too_many_arguments)]
    fn call_value_for_script(
        script_name: &str,
        script: &ScriptEngine,
        definition_context: Option<DefinitionId>,
        function: &str,
        args: &[Value],
        world: HostWorldContext,
        rng: LcgRng,
        env_frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        game_over_triggered: bool,
    ) -> (
        Option<Value>,
        Vec<Value>,
        ScenarioBatch,
        AudioRegistry,
        LcgRng,
        Option<EngineError>,
    ) {
        Self::execute_value_for_script(
            script_name,
            definition_context,
            function,
            args,
            world,
            rng,
            env_frame,
            global_effects,
            physics,
            environment,
            audio,
            game_over_triggered,
            || script.call_with_ref_args(function, args),
        )
    }

    /// DirectExec counterpart of [`Self::call_value_for_script`]. The
    /// expression has no object context and uses the synchronized packet's
    /// strictness instead of the destination host's source strictness.
    #[allow(clippy::too_many_arguments)]
    fn direct_exec_value_for_script(
        script_name: &str,
        script: &ScriptEngine,
        source: &str,
        function_label: &str,
        strict_level: Option<u8>,
        world: HostWorldContext,
        rng: LcgRng,
        env_frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        game_over_triggered: bool,
    ) -> (
        Option<Value>,
        ScenarioBatch,
        AudioRegistry,
        LcgRng,
        Option<EngineError>,
    ) {
        let local_vars = HashMap::new();
        let (value, _finals, batch, audio, rng, error) = Self::execute_value_for_script(
            script_name,
            None,
            function_label,
            &[],
            world,
            rng,
            env_frame,
            global_effects,
            physics,
            environment,
            audio,
            game_over_triggered,
            || {
                script
                    .direct_exec_with_locals_and_this_at_strict_in_context_diagnostics(
                        source,
                        &local_vars,
                        Value::Nil,
                        strict_level,
                        function_label,
                        is_cpp_direct_exec_context(function_label),
                    )
                    .map(|(value, _locals)| (value, Vec::new()))
            },
        );
        (value, batch, audio, rng, error)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_value_for_script<F>(
        script_name: &str,
        definition_context: Option<DefinitionId>,
        function: &str,
        fallback_args: &[Value],
        world: HostWorldContext,
        rng: LcgRng,
        env_frame: u64,
        global_effects: &[EffectState],
        physics: PhysicsSettings,
        environment: EnvironmentSettings,
        audio: AudioRegistry,
        game_over_triggered: bool,
        call: F,
    ) -> (
        Option<Value>,
        Vec<Value>,
        ScenarioBatch,
        AudioRegistry,
        LcgRng,
        Option<EngineError>,
    )
    where
        F: FnOnce() -> Result<(Value, Vec<Value>), ScriptError>,
    {
        let physics_guard = enter_physics_context(physics);
        let env_guard = enter_environment_context(environment, env_frame);
        let guard = enter_random_context(rng);
        let next_object_id = world.next_object_id();
        let audio_guard = enter_audio_context(audio);
        let (result, host_effects) = match definition_context {
            Some(definition) => compat::with_definition_effect_context_with_state(
                definition,
                global_effects,
                world,
                next_object_id,
                game_over_triggered,
                call,
            ),
            None => compat::with_effect_context_with_state(
                None,
                global_effects,
                world,
                next_object_id,
                game_over_triggered,
                call,
            ),
        };
        let rng = guard.finish();
        let mut physics_delta = physics_guard.finish();
        let mut environment_delta = env_guard.finish();
        let (value, finals, script_error) = match result {
            Ok((value, finals)) => (Some(value), finals, None),
            Err(source) => {
                let error = script_execution_error(
                    script_name.to_string(),
                    function.to_string(),
                    source,
                    None,
                );
                if let EngineError::Script { source, .. } = &error {
                    tracing::warn!(
                        script = %script_name,
                        function,
                        %source,
                        "script callback error (continuing like C++ fail-safe exec)"
                    );
                    log_runtime_call_frames(script_name, source.call_frames());
                }
                // The unwound call loses the cells; C++ would keep par
                // mutations made before the error — narrow documented
                // divergence, the original values stand in.
                (None, fallback_args.to_vec(), Some(error))
            }
        };

        let compat::EffectContextOutcome {
            object: _,
            global: host_global_effects,
            object_update: _,
            object_commands: _,
            command_operations: _,
            command_events: _,
            destroy_object: _,
            environment: environment_from_host,
            physics: physics_from_host,
            spawns: host_spawns,
            landscape: host_landscape_ops,
            solid_mask_operations: host_solid_mask_operations,
            host_raster_preview,
            particles: host_particles,
            transfer_zones: host_transfer_zones,
            messages: host_messages,
            player_commands: host_player_commands,
            object_order_commands: host_object_order_commands,
            next_mission_commands: host_next_mission_commands,
            audio: host_audio,
            trigger_game_over: host_trigger_game_over,
            script_go: host_script_go,
            script_counter: host_script_counter,
            next_object_id: _,
            other_objects,
            context_locals: _,
            menu_requests: _,
        } = host_effects;

        let mut batch = ScenarioBatch {
            other_objects,
            ..ScenarioBatch::default()
        };
        batch.player_commands.extend(host_player_commands);
        batch
            .object_order_commands
            .extend(host_object_order_commands);
        batch
            .next_mission_commands
            .extend(host_next_mission_commands);
        batch.global_effects.extend(host_global_effects);
        batch.landscape_ops.extend(host_landscape_ops);
        batch
            .solid_mask_operations
            .extend(host_solid_mask_operations);
        batch.host_raster_preview.0 = host_raster_preview;
        if let Some(delta) = environment_from_host {
            merge_environment_delta(&mut environment_delta, &delta);
        }
        if !environment_delta.is_empty() {
            batch.environment = Some(environment_delta);
        }
        if let Some(delta) = physics_from_host {
            merge_physics_delta(&mut physics_delta, &delta);
        }
        if !physics_delta.is_empty() {
            batch.physics = Some(physics_delta);
        }
        batch.spawns.extend(host_spawns);
        batch.particles.extend(host_particles);
        batch.transfer_zones.extend(host_transfer_zones);
        batch.messages.extend(host_messages);
        batch.audio.extend(host_audio.events);
        if host_trigger_game_over {
            batch.trigger_game_over = true;
        }
        if host_script_go.is_some() {
            batch.script_go = host_script_go;
        }
        if host_script_counter.is_some() {
            batch.script_counter = host_script_counter;
        }
        let audio_state = audio_guard.finish();
        (value, finals, batch, audio_state, rng, script_error)
    }
}

/// Local text work retained until the presentation layer knows whether a
/// message-family speech instance was actually created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechFallback {
    id: u64,
    message: MessageSpec,
}

impl SpeechFallback {
    pub(crate) fn new(id: u64, message: MessageSpec) -> Self {
        Self { id, message }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn into_message(self) -> MessageSpec {
        self.message
    }
}

/// Frontend result for a message-family speech request whose text decision
/// was deferred until the local sound system attempted `NewInstance`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechPlaybackOutcome {
    Played(SpeechFallback),
    Rejected(SpeechFallback),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCommand {
    PlaySound {
        name: String,
        target: Option<ObjectId>,
        volume: u8,
        looped: bool,
        #[serde(default)]
        multiple: bool,
        #[serde(default)]
        custom_falloff: Option<i32>,
    },
    /// `Message`/`PlayerMessage`/`PlrMessage` speech. Unlike `Sound`, these
    /// calls suppress text only if the frontend creates the logical instance.
    PlaySpeech {
        name: String,
        target: Option<ObjectId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<SpeechFallback>,
    },
    DetachObjectSounds {
        target: ObjectId,
        position: Vector2,
    },
    /// `StartSoundEffectAt`: a non-looping global sample whose mix is
    /// calculated once from this landscape position by the presentation
    /// layer.
    PlaySoundAt {
        name: String,
        position: Vector2,
    },
    StopSound {
        name: String,
        target: Option<ObjectId>,
    },
    /// Positive C4 `SoundLevel`: update any live matching instance in place,
    /// or start a loop when the frontend has no such instance.
    SetSoundVolume {
        name: String,
        target: Option<ObjectId>,
        volume: i32,
    },
    PlayMusic {
        name: String,
        looped: bool,
    },
    StopMusic,
    SetMusicLevel {
        level: u8,
    },
    SetMusicPlaylist {
        playlist: Option<String>,
        restart: bool,
    },
}

#[derive(Debug, Default)]
struct CommandBatch {
    delta: ObjectDelta,
    spawns: Vec<SpawnConfig>,
    destroy: bool,
    commands: Vec<QueuedCommand>,
    command_ops: Vec<CommandOperation>,
    effects: Vec<EffectCommand>,
    /// Nested-call mutations to other objects (Find_Func reentrancy).
    other_objects: Vec<compat::NestedObjectOutcome>,
    global_effects: Vec<EffectCommand>,
    environment: Option<EnvironmentDelta>,
    physics: Option<PhysicsDelta>,
    landscape_ops: Vec<LandscapeOperation>,
    solid_mask_operations: Vec<HostSolidMaskOperation>,
    host_raster_preview: Option<compat::HostRasterPreview>,
    particles: Vec<ParticleCommand>,
    transfer_zones: Vec<TransferZoneCommand>,
    audio: Vec<AudioCommand>,
    messages: Vec<MessageCommand>,
    player_commands: Vec<PlayerCommand>,
    object_order_commands: Vec<ObjectOrderCommand>,
    next_mission_commands: Vec<NextMissionCommand>,
    trigger_game_over: bool,
    script_go: Option<bool>,
    script_counter: Option<i32>,
}

/// Opaque carrier required by ScenarioBatch's externally constructible
/// compatibility surface. Runtime callers cannot forge replay operations.
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct HostSolidMaskOperations(Vec<HostSolidMaskOperation>);

impl HostSolidMaskOperations {
    fn extend(&mut self, operations: impl IntoIterator<Item = HostSolidMaskOperation>) {
        self.0.extend(operations);
    }
}

/// Opaque callback-final raster carrier for ScenarioBatch's public
/// compatibility surface.
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct HostRasterPreviewState(Option<compat::HostRasterPreview>);

#[derive(Debug, Default)]
#[doc(hidden)]
pub struct ScenarioBatch {
    #[doc(hidden)]
    pub spawns: Vec<SpawnConfig>,
    /// Nested-call mutations to other objects (Find_Func reentrancy).
    #[doc(hidden)]
    pub other_objects: Vec<compat::NestedObjectOutcome>,
    #[doc(hidden)]
    pub global_effects: Vec<EffectCommand>,
    #[doc(hidden)]
    pub environment: Option<EnvironmentDelta>,
    #[doc(hidden)]
    pub physics: Option<PhysicsDelta>,
    #[doc(hidden)]
    pub landscape_ops: Vec<LandscapeOperation>,
    #[doc(hidden)]
    pub solid_mask_operations: HostSolidMaskOperations,
    #[doc(hidden)]
    pub host_raster_preview: HostRasterPreviewState,
    #[doc(hidden)]
    pub landscape: Vec<LandscapeCommand>,
    #[doc(hidden)]
    pub particles: Vec<ParticleCommand>,
    #[doc(hidden)]
    pub transfer_zones: Vec<TransferZoneCommand>,
    #[doc(hidden)]
    pub audio: Vec<AudioCommand>,
    #[doc(hidden)]
    pub messages: Vec<MessageCommand>,
    #[doc(hidden)]
    pub player_commands: Vec<PlayerCommand>,
    #[doc(hidden)]
    pub object_order_commands: Vec<ObjectOrderCommand>,
    #[doc(hidden)]
    pub next_mission_commands: Vec<NextMissionCommand>,
    #[doc(hidden)]
    pub trigger_game_over: bool,
    #[doc(hidden)]
    pub script_go: Option<bool>,
    #[doc(hidden)]
    pub script_counter: Option<i32>,
}

#[derive(Clone)]
struct RuntimeScenarioSection {
    name: String,
    /// True only for the implicit section backed by the scenario root group.
    /// C++ distinguishes this through an empty C4ScenarioSection::Filename;
    /// the visible section name may be anything, including a named `Main`.
    source_is_scenario_root: bool,
    /// Mirrors C4ScenarioSection::fModified. A section becomes save-worthy
    /// only when a changing section switch persists its landscape or objects.
    modified: bool,
    landscape_modified: bool,
    objects_modified: bool,
    /// Raw temporary C4Group image created when this section was left.
    /// Final C4GameSave copies this image unchanged instead of rebuilding it
    /// from later string IDs, globals, or object state.
    frozen_group: Option<Vec<u8>>,
    source_group: Option<clonk_resources::Group>,
    landscape: Option<Landscape>,
    landscape_systems: scenario::ScenarioLandscapeSystems,
    exact_landscape: bool,
    texmap_lookups: Vec<landscape::RuntimeTexMapLookup>,
    resynthesize_static_map: bool,
    map_creator: Option<map_creator_s2::MapCreatorS2State>,
    s2_overload: Option<scenario::ScenarioSectionS2Spec>,
    gravity: scenario::LegacyC4SVal,
    post_init_map_callbacks: map_creator_s2::PostInitMapCallbacks,
    keep_map_creator: bool,
    no_initialize: bool,
    /// Synthetic-group fallback only. Real source/frozen sections recompile
    /// Objects.txt for every C4GameObjects::Load boundary.
    initial_objects: Vec<scenario::ScenarioSpawn>,
    saved_objects: Option<Vec<PersistedObject>>,
    saved_object_order: Vec<ObjectId>,
    scenario_values: scenario::ScenarioValueStore,
    base_reject_entrance_enabled: bool,
    base_extinguish_enabled: bool,
    environment: EnvironmentSettings,
}

/// Persistent script hosts in C4AulScriptEngine child order. ReLink replays
/// this complete ledger to reconstruct global functions and append copies.
#[derive(Clone)]
enum ScriptLinkSource {
    Script {
        name: String,
        base_script: clonk_script::Script,
        script: Arc<ScriptEngine>,
    },
    Definition(DefinitionId),
    Scenario,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectEnterOutcome {
    Entered,
    RejectedEntrance,
    RejectedCollect,
    Removed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GetEnterOutcome {
    Entered,
    Retry,
    Completed,
    MinimumConstructionDenied(String),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectComPutTakeOutcome {
    Finished,
    NeedsGet(ObjectId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImmediateCommandResume {
    Front,
    MoveToAfterStop,
    MoveToAfterFlight(u64),
    BuildAfterStop(u64),
    ExitAfterStop(u64),
    ThrowPrelude(u64),
    DropPrelude(u64),
    PutAfterStop(u64),
    ConstructAfterStop(u64),
    ConstructScript {
        command_instance_id: u64,
        result: AcquireScriptResult,
    },
    ConstructSpawn {
        command_instance_id: u64,
        construction_id: Option<ObjectId>,
    },
    Physical {
        command_instance_id: u64,
        physical: PhysicalInfo,
    },
}

/// Network control timing copied from a C++ `C4PacketJoinData` before
/// synchronized gameplay starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkControlTiming {
    start_control_tick: i32,
    control_rate: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("network control rate {control_rate} is outside the C++ host range 1..=20")]
pub struct InvalidNetworkControlRate {
    control_rate: i32,
}

impl NetworkControlTiming {
    pub const MIN_CONTROL_RATE: i32 = 1;
    pub const MAX_CONTROL_RATE: i32 = 20;

    pub fn new(
        start_control_tick: i32,
        control_rate: i32,
    ) -> Result<Self, InvalidNetworkControlRate> {
        if !(Self::MIN_CONTROL_RATE..=Self::MAX_CONTROL_RATE).contains(&control_rate) {
            return Err(InvalidNetworkControlRate { control_rate });
        }
        Ok(Self {
            start_control_tick,
            control_rate,
        })
    }
}

/// Process-local gates consulted while executing a synchronized script
/// control. Sender identity remains part of the packet; these flags describe
/// whether this execution is live or replayed and whether the corresponding
/// non-host escape hatch is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptControlPolicy {
    pub is_replay: bool,
    pub console_active: bool,
    pub allow_scripting_in_replays: bool,
}

/// Function names offered by the developer console's script-entry
/// autocomplete. The platform shell owns presentation because Win32 prepends
/// scenario functions and a divider while GTK appends bare names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleScriptCompletionCatalog {
    pub engine_functions: Vec<String>,
    pub scenario_functions: Vec<String>,
}

/// Goal rows produced by one synchronized `ActivateGameGoalMenu` call.
/// Evaluation happens on every peer; `open_menu` is true only where
/// the addressed player is locally controlled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameGoalMenuRequest {
    pub player: i32,
    pub goals: Vec<DefinitionId>,
    pub fulfilled_goals: Vec<DefinitionId>,
    pub open_menu: bool,
}

/// Process-local pause action requested by the script `PauseGame` builtin.
/// The embedding app owns the actual console/network pause transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseGameRequest {
    Halt,
    Toggle,
}

/// Process-local physical-viewport mutations requested by script natives.
/// The embedding app owns the physical viewport and applies this single
/// ordered stream without changing synchronized player or snapshot state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportPresentationRequest {
    SetFilmView { player: i32 },
    SetViewOffset { player: i32, offset: Vector2 },
}

/// Process-local network pacing request produced by the script `SetPreSend`
/// builtin. Every peer executes the synchronized call, then the embedding app
/// matches `client_pattern` against its own client name before applying the
/// target and classic flash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkTargetFpsRequest {
    pub target_fps: i32,
    pub client_pattern: Option<String>,
}

impl ScriptControlPolicy {
    pub const fn live(console_active: bool) -> Self {
        Self {
            is_replay: false,
            console_active,
            allow_scripting_in_replays: false,
        }
    }

    pub const fn replay(allow_scripting_in_replays: bool) -> Self {
        Self {
            is_replay: true,
            console_active: false,
            allow_scripting_in_replays,
        }
    }
}

/// Process-local `Config.General.MissionAccess` storage shared by game engines.
///
/// This deliberately lives outside [`EngineState`]: mission access is local
/// application configuration in C++, not synchronized or savegame state.
#[derive(Clone, Debug)]
pub struct MissionAccessStore {
    inner: Rc<RefCell<String>>,
}

impl MissionAccessStore {
    pub fn new(mission_access: impl Into<String>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(mission_access.into())),
        }
    }

    /// Tests one password against the live process-local semicolon module
    /// list with the same legacy-byte, case-insensitive rules as
    /// `GetMissionAccess`/`SIsModule`.
    pub fn contains(&self, password: &str) -> bool {
        crate::compat::mission_access_contains(&self.inner.borrow(), password)
    }

    /// Current process-local `Config.General.MissionAccess` value.
    pub fn snapshot(&self) -> String {
        self.inner.borrow().clone()
    }

    /// Applies `SAddModules`/`SRemoveModules`-style semicolon modules and
    /// returns the value that should be persisted to configuration.
    ///
    /// Module matching is ASCII-case-insensitive and surrounding spaces are
    /// ignored, as in `SGetModule` plus `SIsModule(..., false)`.
    pub fn update_modules(&self, modules: &str, remove: bool) -> String {
        let requested = modules
            .split(';')
            .map(str::trim)
            .filter(|module| !module.is_empty())
            .collect::<Vec<_>>();
        let mut current = self
            .inner
            .borrow()
            .split(';')
            .map(str::trim)
            .filter(|module| !module.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if remove {
            current.retain(|entry| {
                !requested
                    .iter()
                    .any(|module| entry.eq_ignore_ascii_case(module))
            });
        } else {
            for module in requested {
                if !current
                    .iter()
                    .any(|entry| entry.eq_ignore_ascii_case(module))
                {
                    current.push(module.to_string());
                }
            }
        }
        let value = current.join(";");
        *self.inner.borrow_mut() = value.clone();
        value
    }
}

impl Default for MissionAccessStore {
    fn default() -> Self {
        Self::new(String::new())
    }
}

/// One-shot process-local request to enable `Config.Graphics.ShowCommands`.
///
/// Fresh engines owned by one app share this store. It stays outside
/// [`EngineState`], and draining it lets a later user toggle remain off until
/// another SetPlrShowCommand call.
#[derive(Clone, Debug, Default)]
pub struct ShowCommandsRequestStore {
    inner: Rc<std::cell::Cell<bool>>,
}

impl ShowCommandsRequestStore {
    pub fn request_enable(&self) {
        self.inner.set(true);
    }

    pub fn take_enable_request(&self) -> bool {
        self.inner.replace(false)
    }
}

#[cfg(test)]
#[test]
fn mission_access_store_can_be_shared_across_engines() {
    let store = MissionAccessStore::new("First");
    let mut first = Engine::new();
    first.set_mission_access_store(store.clone());
    let mut second = Engine::new();
    second.set_mission_access_store(store);

    first.mission_access.inner.borrow_mut().push_str(";Second");
    assert_eq!(&*second.mission_access.inner.borrow(), "First;Second");
    assert!(second.mission_access.contains("first"));
    assert!(second.mission_access.contains("SECOND"));
    assert!(!second.mission_access.contains("Fir"));
}

#[cfg(test)]
#[test]
fn mission_access_store_updates_semicolon_modules_case_insensitively() {
    let store = MissionAccessStore::new("Alpha; Beta");
    assert_eq!(
        store.update_modules("beta;Gamma", false),
        "Alpha;Beta;Gamma"
    );
    assert_eq!(store.snapshot(), "Alpha;Beta;Gamma");
    assert_eq!(store.update_modules("alpha;GAMMA", true), "Beta");
}

pub struct Engine {
    #[doc(hidden)]
    pub(crate) definitions: HashMap<DefinitionId, Definition>,
    /// Definition registration order — C++ links scripts in child
    /// registration order (C4AulScript::Child0 walk, C4AulLink.cpp:31),
    /// which decides the overload chain when several appends hit the same
    /// target function.
    definition_load_order: Vec<DefinitionId>,
    /// Runtime `Game.Defs` order after C4DefList::SortByID. This is distinct
    /// from script-host registration order, which still controls linking.
    runtime_definition_order: Rc<Vec<DefinitionId>>,
    /// The engine-global `static` table (Game.ScriptEngine.GlobalNamed):
    /// one shared named-variable table for every script host (scenario
    /// script, definitions, appended scripts). pub(crate) for tests.
    pub(crate) script_globals: clonk_script::GlobalVariables,
    /// The engine-global numbered-variable table
    /// (`Game.ScriptEngine.Global`), shared by every script host.
    pub(crate) script_global_slots: clonk_script::GlobalSlots,
    /// The engine-global `static const` registry (the C4Aul global
    /// constant table, RegisterGlobalConstant C4Aul.cpp:484): shared into
    /// every script host so pre-#strict-2 constant calls (`NAME()`)
    /// resolve constants declared by OTHER scripts (MagiClonk's
    /// `MCLK_ComboExtraDataName()` running in the MAGE host).
    pub(crate) script_global_consts: clonk_script::GlobalVariables,
    /// Engine-global C4String registration order shared by every script host.
    /// Runtime save enumeration filters this ledger to currently referenced
    /// values before assigning Strings.txt IDs.
    pub(crate) script_string_registrations: clonk_script::StringRegistrations,
    /// Exact zero-based `Strings.txt` enumeration from the scenario/save
    /// group currently being loaded. Object state consumes it during parse;
    /// embedded player files need the same IDs later in the restore pipeline.
    legacy_string_table: clonk_script::StringRegistrations,
    /// Every live script host in C4AulScriptEngine child order. Definition-pack
    /// System hosts are interleaved with definitions; scenario Script.c and
    /// its System.c4g hosts follow all definitions.
    script_link_sources: Vec<ScriptLinkSource>,
    /// Definition hosts whose source was reparsed at runtime, in reparse
    /// order. C4Aul recreates their engine-owned global functions at FuncL's
    /// tail, while their append position remains the original child order.
    reloaded_global_definitions: Vec<DefinitionId>,
    /// Bumped whenever `objects` changes SHAPE (push/retain/clear) so the
    /// id->index cache below can trust its entries; indices are stable
    /// between bumps (destruction only flags, removal happens in the
    /// end-of-tick retain).
    objects_generation: std::cell::Cell<u64>,
    /// Generation-stamped id->index map: CrossCheck resolves object ids per
    /// candidate pair and the former linear scan dominated tick time.
    object_index_cache: std::cell::RefCell<(u64, HashMap<ObjectId, usize>)>,
    /// Shared metadata view of `definitions` for host contexts; definitions
    /// only change while loading, so this is built once and dropped on any
    /// definition mutation (host contexts are built per script callback and
    /// re-cloning every ActionLibrary there dominated tick time).
    definition_metadata_cache:
        std::cell::RefCell<Option<Rc<HashMap<DefinitionId, compat::DefinitionMetadata>>>>,
    /// Immutable command-facing definition fields. Command execution consults
    /// this table every tick, while definitions only change during loading.
    /// Sharing one table avoids cloning IDs and rescanning every ActMap on
    /// each frame and immediate command continuation.
    command_definition_snapshot_cache:
        std::cell::RefCell<Option<Rc<HashMap<DefinitionId, CommandDefinitionSnapshot>>>>,
    /// Native `C4Def::pFairCrewPhysical`: one lazily filled projection per
    /// definition, shared by engine reads and every copied script host world.
    /// Derived definition state is intentionally absent from EngineState.
    fair_crew_physical_cache: FairCrewPhysicalCache,
    /// Definition/script lookup data shared by copied host worlds. These
    /// tables change only at definition-load or script-relink boundaries.
    host_definition_tables_cache: std::cell::RefCell<Option<Rc<compat::HostDefinitionTables>>>,
    /// Cached pending-spawn solid-mask descriptors. Sprite pixel payloads
    /// remain Arc-shared; host contexts only clone this Rc table.
    solid_mask_metadata_cache:
        std::cell::RefCell<Option<Rc<HashMap<DefinitionId, compat::HostSolidMaskMetadata>>>>,
    #[doc(hidden)]
    pub materials: MaterialSet,
    /// Shared view of `materials` for host contexts (FnMaterial);
    /// invalidated when the library is (re)configured.
    materials_shared: std::cell::RefCell<Option<Rc<MaterialSet>>>,
    #[doc(hidden)]
    pub objects: Vec<Object>,
    #[doc(hidden)]
    pub next_object_id: u64,
    /// Next C4SolidMask construction-order token. Runtime-only: native
    /// pSolidMaskData is NoSave and is rebuilt after a load.
    next_solid_mask_instance_sequence: u64,
    /// Channel folds are suppressed while chronological host mask
    /// operations are staged for replay.
    defer_solid_mask_updates: bool,
    /// Operations accumulated by the outermost deferred fold. Nested spawn,
    /// effect, and callback folds append to this same chronological stream.
    deferred_solid_mask_operations: Vec<HostSolidMaskOperation>,
    /// Exact callback-final raster exposed to synchronous callbacks that run
    /// before the outermost chronological stream reaches authoritative replay.
    deferred_host_raster_preview: Option<compat::HostRasterPreview>,
    #[doc(hidden)]
    pub rng: LcgRng,
    /// Game.Script.Go — the scenario Script%d counter gate (FnScriptGo,
    /// C4Script.cpp:2782-2786).
    #[doc(hidden)]
    pub scenario_script_go: bool,
    /// Game.Script.Counter — the next Script%d section
    /// (C4GameScriptHost::Execute, C4ScriptHost.cpp:222-232).
    scenario_script_counter: i32,
    /// Game.Parameters.RandomSeed - kept for the game-start re-fix
    /// (C4Game::Synchronize, C4Game.cpp:3695).
    random_seed: u64,
    /// `Game.Parameters.MaxPlayers`. `None` means the embedding app or
    /// scenario has not attached the active game-parameter value yet.
    max_players: Option<i32>,
    /// `Game.Parameters.StartupPlayerCount`, frozen once before landscape
    /// creation and reused by every initial or runtime player join.
    startup_player_count: Option<i32>,
    /// `Game.Parameters.UseFairCrew`: when enabled, objects carrying crew
    /// info read definition-based fair-crew physicals instead of their
    /// persistent trained info physicals.
    use_fair_crew: bool,
    /// `Game.Parameters.FairCrewStrength`, interpreted through the active
    /// rank system when deriving fair-crew physicals.
    fair_crew_strength: i32,
    /// `Game.Parameters.FairCrewForced`: CID_Set(FairCrew) is a no-op while
    /// this synchronized scenario lock is active.
    fair_crew_forced: bool,
    /// `Game.Parameters.AllowDebug`, cleared by CID_Set(DisableDebug)
    /// regardless of packet author.
    allow_debug: bool,
    /// Process-local `Game.DebugMode`. This is deliberately distinct from
    /// the synchronized AllowDebug parameter, but DisableDebug clears both.
    debug_mode: bool,
    /// `Game.Parameters.IsNetworkGame`, derived from the app's active
    /// network session just as C++ copies `Game.NetworkActive` during
    /// parameter setup (C4GameParameters.cpp:429-434).
    network_game: bool,
    /// Whether `C4GameControl::eMode == CM_Network`. Unlike `network_game`,
    /// ChangeToLocal clears this while preserving Game.Parameters.
    network_control_mode: bool,
    /// Whether recording is active or pre-armed for game initialization.
    /// C++ includes `pRecord` in `C4GameControl::SyncMode`; this process-local
    /// state deliberately does not enter synchronized snapshots.
    recording_active: bool,
    /// Process-local `Game.Control.isReplay()` state. It suppresses local
    /// presentation without changing synchronized goal evaluation.
    replay_control: bool,
    /// Whether the embedding app currently owns a primary physical viewport.
    /// Scenario Initialize runs before this becomes true.
    film_viewport_available: bool,
    /// `Game.Parameters::isLeague()` — specifically whether the synchronized
    /// LeagueAddress is non-empty. This is independent from network play:
    /// ordinary network games may still allow script-driven team switches.
    league_game: bool,
    /// Exact synchronized `Game.Parameters.League` bytes. Unlike
    /// `league_game`, this is the gate used by league progress-data APIs.
    league_name: Rc<Vec<u8>>,
    /// All retained C4PlayerInfo progress buffers, keyed by persistent ID.
    player_info_league_progress_data: Rc<BTreeMap<i32, Option<Vec<u8>>>>,
    /// Sparse nonzero C4PlayerInfo league scores. Known IDs are tracked by
    /// the progress projection/player list; an absent score is exactly 0.
    player_info_league_scores: Rc<BTreeMap<i32, i32>>,
    /// Whether this process owns authoritative control input. Offline games
    /// and network hosts do; clients and replay consumers do not.
    control_host: bool,
    /// Deferred CreateScriptPlayer updates. PlayerInfo must enter the same
    /// app/control path as every other join instead of mutating players from
    /// inside the script callback.
    player_info_updates: Rc<RefCell<Vec<PlayerInfoUpdateRequest>>>,
    /// Script writes already folded into the engine that the embedding
    /// PlayerInfo registry must mirror before its next full projection.
    player_info_league_progress_updates: Vec<(i32, Option<Vec<u8>>)>,
    /// Host-side `Game.Input` requests produced by
    /// `EliminatePlayer(plr, true)`. The app moves these into a later
    /// synchronized control tick; applying the script callback must not
    /// remove the player inline.
    pending_remove_player_controls: Vec<RemovePlayerControlData>,
    /// Runtime presentation requests produced after synchronized goal
    /// evaluation. This is deliberately excluded from EngineState.
    pending_game_goal_menu_requests: Vec<GameGoalMenuRequest>,
    /// Process-local console pause requests emitted by `PauseGame`. Shared
    /// into copied host contexts so nested calls preserve script call order.
    pause_game_requests: Rc<RefCell<Vec<PauseGameRequest>>>,
    /// Process-local pacing writes emitted by `SetPreSend`, in script-call
    /// order. The app owns client-name matching, the network clock and flash.
    network_target_fps_requests: Rc<RefCell<Vec<NetworkTargetFpsRequest>>>,
    /// Process-local physical viewport requests emitted by `SetFilmView` and
    /// `SetViewOffset`. Physical viewport ownership remains in the app.
    viewport_presentation_requests: Rc<RefCell<Vec<ViewportPresentationRequest>>>,
    /// Process-local `Console.EditCursor.Target`. The developer console owns
    /// this pointer; synchronized state and snapshots deliberately do not.
    edit_cursor_target: Option<ObjectId>,
    /// Explicit client-local players. `None` is the standalone/headless
    /// default where every registered player has local control.
    local_players: Option<HashSet<i32>>,
    /// Process-local `Config.Controls`/`Config.Gamepads` display names,
    /// keyed by effective player control set. This presentation-only table
    /// is intentionally absent from `EngineState` and snapshots.
    control_key_names: Rc<HashMap<i32, Vec<ControlKeyName>>>,
    /// The process-local singleton script-query edit line. Player query
    /// registration is synchronized and saved; this presentation state is
    /// deliberately runtime-only like C++ `Game.MessageInput`.
    active_message_board_input: Option<ActiveMessageBoardInput>,
    /// Live `C4MessageInput::Commands` registry. Entries retain insertion
    /// order for Game.txt/JoinData serialization; lookup is exact and the
    /// first registration of a name wins.
    message_board_commands: Vec<InitialNetworkMessageBoardCommand>,
    /// The C++ master object list (`::Objects`) kept in EXEC order:
    /// C4Game::ExecObjects walks the list from the BACK (C4Game.cpp:1582),
    /// so this vec is the C4ObjectList REVERSED — index 0 executes first.
    /// Maintained by `insert_into_exec_list` (C4ObjectList::Add stMain
    /// semantics) on spawn and pruned of removed ids each tick. Enter/Exit
    /// never touch it (C4Object.cpp:1513-1615 only move Contents).
    #[doc(hidden)]
    pub exec_list: Vec<ObjectId>,
    /// Bumped whenever `insert_exec_link` adds or re-adds a main-list link.
    /// Such insertions can happen during the live object walk, so command
    /// snapshots use this to refresh their forward-list tie-break ranks.
    exec_list_insert_generation: u64,
    /// `C4GameObjects::InactiveObjects`, also stored in reverse C++ list
    /// order so the same stMain insertion rules as `exec_list` apply.
    /// Unlike the retained execution ledger above, this list is updated on
    /// every modeled status transition and is authoritative for callbacks
    /// that explicitly walk InactiveObjects.
    inactive_exec_list: Vec<ObjectId>,
    /// Deferred category resorts plus FnSetObjectOrder requests. Category
    /// work runs first; relative requests execute newest-first afterward.
    #[doc(hidden)]
    pub pending_object_order_commands: Vec<ObjectOrderCommand>,
    /// Native `C4Game::fResortAnyObject`. This is deliberately independent
    /// of ResortProc: relative and category-order requests do not set it.
    resort_any_object: bool,
    /// Next `exec_list` slot during the live reverse-list walk. Insertions
    /// before this cursor have already missed the C++ iterator; insertions at
    /// or after it still execute this frame.
    exec_cursor: Option<usize>,
    #[doc(hidden)]
    pub frame: u64,
    /// Process-local C4Application timer state. It is deliberately excluded
    /// from save/sync snapshots; synchronized custom commands update every
    /// peer's application timer independently.
    game_tick_delay_ms: Rc<std::cell::Cell<u64>>,
    game_tick_delay_revision: Rc<std::cell::Cell<u64>>,
    /// `C4Game::Time` seconds, advanced only by `sec1_timer`.
    #[doc(hidden)]
    pub game_time: i32,
    /// Runtime-only one-second latch (`C4Game::TimeGo`), never serialized.
    #[doc(hidden)]
    pub time_go: bool,
    #[doc(hidden)]
    pub landscape: Option<Landscape>,
    #[doc(hidden)]
    pub sectors: Option<SectorMap>,
    #[doc(hidden)]
    pub physics: PhysicsSettings,
    #[doc(hidden)]
    pub environment: EnvironmentSettings,
    /// Game.GraphicsSystem.dwGamma: nine independently additive ramps.
    #[doc(hidden)]
    pub gamma: GammaControlState,
    sky: Option<SkyState>,
    global_effects: Vec<EffectState>,
    particles: Vec<ActiveParticle>,
    /// C4ParticleSystem port (def-based particles, src/C4Particles.cpp). The
    /// `particles` Vec above only serves def-less legacy fixture particles.
    particle_system: particles::ParticleSystem,
    /// C4PXSSystem port (sync-relevant pixel sprites, src/C4PXS.cpp).
    #[doc(hidden)]
    pub pxs_system: pxs::PxsSystem,
    /// Control/sync-check state machine (C4GameControl): ControlTick advances
    /// every ControlRate frames; a sync check is digested every SyncRate
    /// frames (C4SyncCheckRate = 100) and kept for 50 frames.
    #[doc(hidden)]
    pub control_rate: i32,
    #[doc(hidden)]
    pub control_tick: i32,
    #[doc(hidden)]
    pub sync_rate: i32,
    do_sync: bool,
    sync_checks: Vec<SyncCheckPacket>,
    #[doc(hidden)]
    pub mass_movers: MassMoverSet,
    weather_events: Vec<WeatherEvent>,
    scenario_script: Option<ScenarioScript>,
    /// The System.c4g global-function table (Game.ScriptEngine in C++),
    /// shared into every script host.
    #[doc(hidden)]
    pub global_script_functions: Option<Arc<HashMap<String, clonk_script::Function>>>,
    /// Physical `Game.ScriptEngine` SFunc insertion order (`Func0` to
    /// `FuncL`). Context menus enumerate this ledger backward and suppress
    /// older exact-name overloads just like `C4AulScript::GetSFunc`.
    global_script_function_order: Vec<String>,
    next_mission: NextMissionState,
    /// `Game.RestartRestoreInfos.What`: a process-runtime mask for the next
    /// network restart. C++ does not compile it into save/snapshot state.
    restart_restore_info_mask: i32,
    game_over_triggered: bool,
    /// Runtime-only C4Game::Evaluated guard. Restored games re-evaluate
    /// after their next synchronized frame just like C++.
    game_evaluated: bool,
    /// C4RoundResults, kept distinct from the game-over trigger and runtime
    /// evaluation guard (C4Game.cpp:845-854).
    #[doc(hidden)]
    pub round_results: RoundResultsState,
    objectives: ScenarioObjectives,
    objective_check_counter: u8,
    players_registered: bool,
    #[doc(hidden)]
    pub players: HashMap<i32, Player>,
    /// Exact `C4PlayerList` link order. Player numbers normally sort ascending,
    /// but native `RecheckPlayerSort` has observable insertion edge cases that
    /// cannot be reconstructed from the map alone.
    player_order: Vec<i32>,
    /// Join inputs retained for every live C4PlayerInfo-backed player.
    /// `ScenarioAndTeamInit` may rerun `ScenarioInit` after the initial join,
    /// not only while runtime team choice is pending.
    pending_player_joins: HashMap<i32, JoinPlayerConfig>,
    /// C4PlayerInfoList::iLastPlayerID, persisted and repaired across loads.
    #[doc(hidden)]
    pub last_player_info_id: i32,
    /// Scenario `[Head] ForcedAutoStopControl`, separate from each player's
    /// effective `PlayerControlState::control_style` preference.
    forced_control_style: Option<bool>,
    /// Scenario `[Head] ForcedAutoContextMenu`, separate from each player's
    /// effective `PlayerControlState::auto_context_menu` preference.
    forced_auto_context_menu: Option<bool>,
    /// Ordered `Game.Teams` entries loaded from the scenario's Teams.txt.
    teams: Rc<Vec<TeamInfo>>,
    /// Complete live `Game.Teams` configuration. The team vector alone is
    /// insufficient to reconstruct Custom/Active/AutoGenerateTeams.
    team_configuration: TeamConfiguration,
    /// Savegame-only C4TeamList compiler fields. These remain independent
    /// from the live team vector and its seven script-queryable flags.
    team_last_team_id: i32,
    team_max_script_players: i32,
    team_script_player_names: Vec<u8>,
    team_random_team_count: i32,
    /// `C4TeamList::IsRuntimeJoinTeamChoice`: custom, active team lists
    /// postpone teamless user ScenarioInit until a team control executes.
    runtime_join_team_choice: bool,
    crew_selection: HashMap<i32, CrewSelection>,
    crew_roles: HashMap<i32, HashMap<ObjectId, CrewRole>>,
    /// The four C4SPlrStart slots retained from the scenario: consumed at
    /// player JOIN by the ScenarioInit port (C4Player.cpp:670-777).
    player_starts: Vec<scenario::PlayerStart>,
    /// `Game.Names` — the standard clonk-name list (planet System.c4g
    /// Names.txt, overridable by a scenario Names.txt; C4Game.cpp:2772,
    /// 3288-3289). Crew-info creation draws names from it.
    standard_names: Option<String>,
    /// `[Landscape] MapZoom` retained as a C4SVal: ScenarioInit evaluates
    /// it per start coordinate (C4Player.cpp:713-714) — synced RNG draws.
    map_zoom: scenario::LegacyC4SVal,
    /// Fully defaulted, post-load `Game.C4S` reflection table used by
    /// GetScenarioVal. Kept independently of evaluated landscape/weather
    /// state, exactly like C++ retains C4Scenario beside those subsystems.
    scenario_values: Rc<scenario::ScenarioValueStore>,
    /// Runtime-loadable `Sect*.c4g` payloads plus the implicit main section.
    /// Keys are ASCII-lowercase because C4ScenarioSection lookup is
    /// case-insensitive (C4Game.cpp:4101-4104).
    scenario_sections: HashMap<String, RuntimeScenarioSection>,
    /// `Game.pScenarioSections` linked-list traversal order. Each discovered
    /// named section is prepended; the implicit current node joins only when
    /// the first LoadScenarioSection call prepends it.
    scenario_section_order: Vec<String>,
    /// Whether C++ has materialized `pCurrentScenarioSection`. Compiling
    /// CurrentScenarioSection from Game.txt does not create this pointer; the
    /// first LoadScenarioSection call does and prepends it to the list.
    scenario_current_section_registered: bool,
    current_scenario_section: String,
    last_scenario_section_flags: Option<i32>,
    /// Per-player crew info lists (C4Player::CrewInfoList): the roster
    /// GetIdle/New recruit from at join.
    crew_rosters: HashMap<i32, Vec<player_file::CrewInfo>>,
    /// C4ObjectInfoList traversal order expressed as stable roster indices.
    /// New entries are appended to `crew_rosters` for pointer identity but
    /// inserted at the front here like `C4ObjectInfoList::New`.
    crew_info_order: HashMap<i32, Vec<usize>>,
    /// Crew object -> its C4ObjectInfo data (name/rank/experience), the
    /// `pObj->Info` link of CreateInfoObject (C4Game.cpp:1156-1170).
    crew_object_infos: Rc<HashMap<ObjectId, CrewObjectInfo>>,
    /// Shared rank view of `crew_object_infos` for host contexts
    /// (GetHiRank); rebuilt when crew infos change (joins are rare).
    crew_ranks: Rc<HashMap<u64, i32>>,
    /// Owning C4Player::CrewInfoList for each live object-info pointer.
    /// This is independent of C4Object::Owner and crew-list membership.
    crew_info_links: Rc<HashMap<ObjectId, CrewInfoLink>>,
    /// Objects.txt rows awaiting C4GameObjects::AssignInfo in InitGameFinal.
    /// A present `None` records a loaded object without an `Info=` line: it
    /// can still need MakeCrewMember when a restored Player::Crew points at
    /// it. This transient load ledger is intentionally absent from saves.
    pending_legacy_object_infos: HashMap<ObjectId, Option<String>>,
    /// Runtime-only C4ObjectInfo::ControlCount, keyed by the stable roster
    /// pointer identity so it follows an info reattached to another object.
    crew_info_control_counts: HashMap<CrewInfoLink, i32>,
    team_home_base_rule: bool,
    needed_material_strings: Rc<NeededMaterialStrings>,
    /// Process-local `IDS_OBJ_NODIG` template from Application.ResStrTable.
    /// The app refreshes it with the active language and reinstalls it on
    /// fresh engines; headless engines retain the shipped US fallback.
    object_no_dig_resource_string: Rc<String>,
    /// Process-local ConstructionCheck feedback templates from
    /// Application.ResStrTable (C4Landscape.cpp:2131-2163).
    construction_check_strings: Rc<ConstructionCheckStrings>,
    /// Process-local `Game.Rank` names frozen from IDS_GAME_DEFRANKS during
    /// game initialization. Rank numbers and experience remain synchronized;
    /// this localized presentation table deliberately stays out of snapshots.
    default_rank_names: Rc<Vec<String>>,
    construction_needs_material: bool,
    structures_need_energy: bool,
    structures_snow_in: bool,
    flag_removeable: bool,
    base_buy_enabled: bool,
    base_sell_enabled: bool,
    base_auto_sell_enabled: bool,
    base_reject_entrance_enabled: bool,
    base_regenerate_energy_enabled: bool,
    base_extinguish_enabled: bool,
    base_regenerate_energy_price: i32,
    landscape_insert_thrust: bool,
    known_crew_owners: HashSet<i32>,
    eliminated_crew_owners: HashSet<i32>,
    transfer_zones: TransferZoneTable,
    /// Runtime-only settings on `Game.PathFinder`. MoveTo overwrites both
    /// before an obstructed search; script GetPath reuses the last pair.
    pathfinder_level: i32,
    pathfinder_transfer_zones_enabled: bool,
    /// Process-local presentation state retained by native's global
    /// `Game.PathFinder`; excluded from saves and sync checks.
    pathfinder_debug: Rc<RefCell<PathfinderDebugSnapshot>>,
    audio_registry: AudioRegistry,
    #[doc(hidden)]
    pub pending_audio: Vec<AudioCommand>,
    #[doc(hidden)]
    pub pending_menu_requests: Vec<MenuRequest>,
    messages: MessageManager,
    /// Engine-held mission-password surrogate for process config. This stays
    /// outside EngineState/save serialization, like C++ Config.
    mission_access: MissionAccessStore,
    /// FnSetPlrShowCommand's process-local ShowCommands enable request.
    show_commands_requests: ShowCommandsRequestStore,
    scoreboard: Rc<RefCell<ScoreboardState>>,
    scoreboard_presentations: Rc<RefCell<ScoreboardPresentationSink>>,
}

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn clamp_fixed_to_limit(value: C4Fixed, limit: i32) -> C4Fixed {
    if limit <= 0 {
        C4Fixed::ZERO
    } else {
        value.clamp(itofix(-limit), itofix(limit))
    }
}

fn clamp_fixed_to_limit_pair(value: C4Fixed, min: C4Fixed, max: C4Fixed) -> C4Fixed {
    value.clamp(min, max)
}

fn saturating_i64_to_i32(value: i64) -> i32 {
    if value > i64::from(i32::MAX) {
        i32::MAX
    } else if value < i64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

fn saturating_u64_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn step_fixed_toward(current: C4Fixed, desired: C4Fixed, step: C4Fixed) -> C4Fixed {
    if current == desired || step <= C4Fixed::ZERO {
        return desired;
    }
    let delta = i64::from(desired.val()) - i64::from(current.val());
    let step = i64::from(step.val());
    if delta.abs() <= step {
        desired
    } else {
        let next = if delta > 0 {
            i64::from(current.val()).saturating_add(step)
        } else {
            i64::from(current.val()).saturating_sub(step)
        };
        C4Fixed::from_raw(saturating_i64_to_i32(next))
    }
}

/// ComName(byCom) (C4ObjectCom.cpp:800-852): base name plus the
/// Single/Double/Released suffix shared by the Control/Contained script
/// callback families (PSF_Control "~Control{}" / PSF_ContainedControl
/// "~Contained{}", C4Script.h:71-72).
fn com_name(command: ControlCommand, kind: CommandKind) -> Option<String> {
    let base = match command {
        ControlCommand::Throw => "Throw",
        ControlCommand::Dig => "Dig",
        ControlCommand::Special => "Special",
        ControlCommand::Special2 => "Special2",
        _ => return None,
    };

    let suffix = match kind {
        CommandKind::Press => "",
        CommandKind::Single => "Single",
        CommandKind::Double => "Double",
        CommandKind::Release => "Released",
    };

    Some(format!("{base}{suffix}"))
}

fn control_function_name(command: ControlCommand, kind: CommandKind) -> Option<String> {
    com_name(command, kind).map(|com| format!("Control{com}"))
}

fn apply_horizontal_friction_fixed(value: C4Fixed, friction: i32) -> C4Fixed {
    let raw = value.val();
    if raw == 0 || friction == 0 {
        return value;
    }
    let friction = friction.clamp(0, 100);
    if friction == 0 {
        return value;
    }
    let magnitude = i64::from(raw).abs();
    let mut retained = magnitude.saturating_mul(i64::from(100 - friction)) / 100;
    if retained == magnitude && friction > 0 {
        retained = magnitude.saturating_sub(1);
    }
    let signed = if retained == 0 {
        0
    } else if raw > 0 {
        retained
    } else {
        -retained
    };
    C4Fixed::from_raw(saturating_i64_to_i32(signed))
}

#[derive(Debug, Clone, Copy)]
struct LayerMovementBounds {
    position: Vector2,
    shape_rect: DefinitionRect,
    border_bound: i32,
}

/// The effective solid-mask parameters of an eligible object
/// (C4Object::UpdateSolidMask reads SolidMask/Shape/r off the object,
/// C4Object.cpp:5648-5656).
#[derive(Debug, Clone)]
pub(crate) struct SolidMaskSpec {
    mask: DefinitionTargetRect,
    pixels: Option<Arc<Vec<u8>>>,
    shape_x: i32,
    shape_y: i32,
    /// MaskPutRotation (C4SolidMask.cpp:42): the object's `r` at put
    /// time; nonzero only with Def->RotatedSolidmasks.
    rotation: i32,
}

/// One synchronous C4Object::UpdateSolidMask result captured while script
/// callbacks run against their copy-on-write host world. Replaying these in
/// call order preserves buffer ownership across Rust's separate outer and
/// foreign-object outcome channels.
#[derive(Debug, Clone)]
pub(crate) enum HostSolidMaskOperation {
    Remove {
        object_id: ObjectId,
    },
    Put {
        object_id: ObjectId,
        spec: SolidMaskSpec,
        position: Vector2,
        instance_sequence: u64,
    },
    /// Landscape calls share the synchronous timeline with C4SolidMask
    /// Remove/Put. Transactional calls may commute with masks, but not with
    /// raw SetPix-style writes or with one another.
    Landscape {
        operation: LandscapeOperation,
    },
}

/// The rotated-put parameters of a bake (C4SolidMask.cpp:108-174): the
/// buffer is the MatBuffPitch square around the rotated mask extent and
/// membership needs the inverse-rotation sample per buffer cell.
#[derive(Debug, Clone, Copy)]
struct RotatedBake {
    /// MaskPutRotation in degrees.
    rotation: i32,
    /// MatBuffPitch = int(sqrt(Wdt^2+Hgt^2)) + 1 (ctor,
    /// C4SolidMask.cpp:415): the enlarged square buffer edge.
    mat_buff_pitch: i32,
    /// SolidMask.Hgt (bounds partner of `SolidMaskBake::mask_width`).
    mask_height: i32,
}

/// A PUT solid mask (C4SolidMask::Put, unrotated, C4SolidMask.cpp:
/// 24-107): the landscape-clipped MaskPutRect plus the saved background
/// bytes ("MatBuff"); the vehicle byte marks unused buffer slots.
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct SolidMaskBake {
    /// C4SolidMask linked-list construction order. Higher means newer and
    /// therefore earlier in the native Last->Prev survivor re-put walk.
    instance_sequence: u64,
    /// MaskPutRect (landscape space, clipped).
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    /// Buffer-space offset of the clipped rect (MaskPutRect.tx/ty).
    /// Unrotated, buffer coordinates coincide with mask coordinates;
    /// rotated, they index the MatBuffPitch square.
    tx: i32,
    ty: i32,
    /// Full mask width (the alpha-pixel row pitch).
    mask_width: i32,
    /// Per-pixel alpha mask (1 = solid); None = full rectangle.
    pixels: Option<Arc<Vec<u8>>>,
    /// Saved background bytes, row-major width*height (the clipped
    /// window of C++'s MatBuffPitch-pitched pSolidMaskMatBuff).
    buffer: Vec<u8>,
    /// Some for a rotated put (C4SolidMask.cpp:108-174).
    rotated: Option<RotatedBake>,
}

/// Objects resting on a moving solid mask at removal time. C++ stores this
/// beside the mask until `Put(..., fRestoreAttachment=true)` can translate
/// them by the mask owner's movement delta (C4SolidMask.cpp:178-195,
/// 276-305).
#[derive(Debug)]
struct SolidMaskAttachmentBackup {
    /// Identity of the exact C4SolidMask instance that captured these
    /// riders. Recreating the mask discards its internal attachment list.
    instance_sequence: Option<u64>,
    removal_position: Vector2,
    object_ids: Vec<ObjectId>,
}

impl SolidMaskBake {
    fn overlaps(&self, other: &SolidMaskBake) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    /// Alpha lookup at MASK coordinates (pSolidMask[iMy*iPitch+iMx],
    /// C4SolidMask.cpp:150); the rect variant (no sprite) is solid
    /// everywhere the caller's bounds allow.
    fn mask_pixel(&self, mask_x: i32, mask_y: i32) -> bool {
        match &self.pixels {
            None => true,
            Some(pixels) => pixels
                .get((mask_y * self.mask_width + mask_x) as usize)
                .map(|value| *value != 0)
                .unwrap_or(false),
        }
    }

    /// Mask membership at BUFFER coordinates. Unrotated they ARE mask
    /// coordinates; rotated, the cell maps back into the mask through
    /// the inverse-rotation sample of C4SolidMask::Put. Per-cell C4Fixed
    /// products equal the put loop's accumulation bit-for-bit: every
    /// accumuland is (integer)*itofix(1)-scaled, so `itofix(n) * m`
    /// divides exactly by the 2^16 scale — no truncation anywhere until
    /// the final fixtoi, which both paths share.
    ///
    /// NB for a future attached-object backup port: C++'s
    /// DensityProvider does NOT take this sample for rotated masks — it
    /// reads the put BUFFER (C4SolidMask.cpp:218-227), so pixels another
    /// mask claimed first count non-solid there. `buffer[..] != vehicle`
    /// is the faithful test, not `mask_set`.
    fn mask_set(&self, buff_x: i32, buff_y: i32) -> bool {
        match self.rotated {
            None => self.mask_pixel(buff_x, buff_y),
            Some(rotated) => {
                // Matrix of -MaskPutRotation (C4SolidMask.cpp:111-112).
                let negated = itofix(-rotated.rotation);
                let ma1 = negated.cos_deg();
                let ma2 = -negated.sin_deg();
                let mb1 = negated.sin_deg();
                let mb2 = negated.cos_deg();
                let half = rotated.mat_buff_pitch / 2;
                // iMx/iMy (C4SolidMask.cpp:147-148).
                let mask_x = fixtoi(itofix(buff_x - half) * ma1 + itofix(buff_y - half) * ma2)
                    + self.mask_width / 2;
                let mask_y = fixtoi(itofix(buff_x - half) * mb1 + itofix(buff_y - half) * mb2)
                    + rotated.mask_height / 2;
                mask_x >= 0
                    && mask_y >= 0
                    && mask_x < self.mask_width
                    && mask_y < rotated.mask_height
                    && self.mask_pixel(mask_x, mask_y)
            }
        }
    }

    /// DensityProvider for attachment backup (C4SolidMask.cpp:204-228).
    /// Unrotated masks read their raw alpha even when another mask owned the
    /// landscape pixel; rotated masks instead read the saved put buffer.
    fn provides_attachment_density_at(&self, vehicle: u8, x: i32, y: i32) -> bool {
        let local_x = x - self.x;
        let local_y = y - self.y;
        if local_x < 0 || local_y < 0 || local_x >= self.width || local_y >= self.height {
            return false;
        }
        match self.rotated {
            None => self.mask_set(self.tx + local_x, self.ty + local_y),
            Some(_) => self.buffer[(local_y * self.width + local_x) as usize] != vehicle,
        }
    }

    /// The raster half of C4SolidMask::Remove(false, false): restore only
    /// mask-owned pixels that are still MCVehic. This deliberately has no
    /// instability, overlap re-put, attachment, or live-object side effects;
    /// capture uses it on a cloned landscape (C4SolidMask.cpp:240-259).
    fn restore_background(&self, landscape: &mut Landscape, vehicle: u8) {
        for cy in 0..self.height {
            for cx in 0..self.width {
                let saved = self.buffer[(cy * self.width + cx) as usize];
                if saved == vehicle {
                    continue;
                }
                let x = self.x + cx;
                let y = self.y + cy;
                if landscape.grid_byte_at(x, y) == Some(vehicle) {
                    landscape.grid_write_byte(x, y, saved);
                }
            }
        }
    }

    /// The clipped, non-regular `Put` issued for a surviving mask by
    /// `C4SolidMask::Remove` (C4SolidMask.cpp:39-54,263-274). Only opaque
    /// cells in the removed rectangle are visited. Newly exposed background
    /// replaces the survivor's saved byte, while an MCVehic byte retains the
    /// old buffer ownership.
    fn reput_after_removal(
        &mut self,
        removed: &SolidMaskBake,
        landscape: &mut Landscape,
        vehicle: u8,
    ) {
        let clip_x0 = removed.x.max(self.x);
        let clip_y0 = removed.y.max(self.y);
        let clip_x1 = (removed.x + removed.width).min(self.x + self.width);
        let clip_y1 = (removed.y + removed.height).min(self.y + self.height);
        for ly in clip_y0..clip_y1 {
            for lx in clip_x0..clip_x1 {
                let mx = self.tx + (lx - self.x);
                let my = self.ty + (ly - self.y);
                if !self.mask_set(mx, my) {
                    continue;
                }
                let current = landscape.grid_byte_at(lx, ly).unwrap_or(0);
                if current != vehicle {
                    self.buffer[((ly - self.y) * self.width + (lx - self.x)) as usize] = current;
                }
                landscape.grid_write_byte(lx, ly, vehicle);
            }
        }
    }
}

/// Raster-only half of `C4SolidMask::Put`. Script callbacks run against a
/// copy-on-write landscape snapshot, so they need the exact same clipped
/// bake (including the saved material buffer) without mutating the engine's
/// authoritative object or producing instability/attachment side effects.
fn put_solid_mask_raster(
    landscape: &mut Landscape,
    spec: SolidMaskSpec,
    position: Vector2,
    instance_sequence: u64,
) -> Option<SolidMaskBake> {
    let vehicle = landscape.grid_vehicle_byte()?;
    let (grid_width, grid_height) = landscape.grid_dimensions()?;
    let SolidMaskSpec {
        mask,
        pixels,
        shape_x,
        shape_y,
        rotation,
    } = spec;

    if rotation == 0 {
        let ox = position.x + shape_x + mask.target_x;
        let oy = position.y + shape_y + mask.target_y;
        let mut rect_x = ox;
        let mut tx = 0;
        if rect_x < 0 {
            tx = -rect_x;
            rect_x = 0;
        }
        let mut rect_y = oy;
        let mut ty = 0;
        if rect_y < 0 {
            ty = -rect_y;
            rect_y = 0;
        }
        let width = (ox + mask.width).min(grid_width) - rect_x;
        let height = (oy + mask.height).min(grid_height) - rect_y;
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut bake = SolidMaskBake {
            instance_sequence,
            x: rect_x,
            y: rect_y,
            width,
            height,
            tx,
            ty,
            mask_width: mask.width,
            pixels,
            buffer: vec![vehicle; (width * height) as usize],
            rotated: None,
        };
        for cy in 0..height {
            for cx in 0..width {
                if !bake.mask_set(tx + cx, ty + cy) {
                    continue;
                }
                let lx = rect_x + cx;
                let ly = rect_y + cy;
                // A regular put saves MCVehic too; Remove simply never uses
                // that buffer slot for restoration (C4SolidMask.cpp:92-96).
                let old = landscape.grid_byte_at(lx, ly).unwrap_or(0);
                bake.buffer[(cy * width + cx) as usize] = old;
                landscape.grid_write_byte(lx, ly, vehicle);
            }
        }
        return Some(bake);
    }

    // Rotated C4SolidMask::Put (C4SolidMask.cpp:108-174).
    let mat_buff_pitch =
        f64::from(mask.width * mask.width + mask.height * mask.height).sqrt() as i32 + 1;
    let negated = itofix(-rotation);
    let ma1 = negated.cos_deg();
    let ma2 = -negated.sin_deg();
    let mb1 = negated.sin_deg();
    let mb2 = negated.cos_deg();
    let center_x = shape_x + mask.target_x + mask.width / 2;
    let center_y = shape_y + mask.target_y + mask.height / 2;
    let xstart =
        position.x + fixtoi(ma1 * itofix(center_x) - ma2 * itofix(center_y)) - mat_buff_pitch / 2;
    let ystart =
        position.y + fixtoi(-mb1 * itofix(center_x) + mb2 * itofix(center_y)) - mat_buff_pitch / 2;
    let mut rect_x = xstart;
    let mut tx = 0;
    if rect_x < 0 {
        tx = -rect_x;
        rect_x = 0;
    }
    let mut rect_y = ystart;
    let mut ty = 0;
    if rect_y < 0 {
        ty = -rect_y;
        rect_y = 0;
    }
    let width = (xstart + mat_buff_pitch).min(grid_width) - rect_x;
    let height = (ystart + mat_buff_pitch).min(grid_height) - rect_y;
    if width <= 0 || height <= 0 {
        return None;
    }
    let mut bake = SolidMaskBake {
        instance_sequence,
        x: rect_x,
        y: rect_y,
        width,
        height,
        tx,
        ty,
        mask_width: mask.width,
        pixels,
        buffer: vec![vehicle; (width * height) as usize],
        rotated: Some(RotatedBake {
            rotation,
            mat_buff_pitch,
            mask_height: mask.height,
        }),
    };
    let x0 = itofix(tx - mat_buff_pitch / 2);
    let y0 = itofix(ty - mat_buff_pitch / 2);
    let mut ya = y0 * ma2;
    let mut yb = y0 * mb2;
    for cy in 0..height {
        let mut xa = x0 * ma1;
        let mut xb = x0 * mb1;
        for cx in 0..width {
            let mask_x = fixtoi(xa + ya) + mask.width / 2;
            let mask_y = fixtoi(xb + yb) + mask.height / 2;
            if mask_x >= 0
                && mask_y >= 0
                && mask_x < mask.width
                && mask_y < mask.height
                && bake.mask_pixel(mask_x, mask_y)
            {
                let lx = rect_x + cx;
                let ly = rect_y + cy;
                let old = landscape.grid_byte_at(lx, ly).unwrap_or(0);
                bake.buffer[(cy * width + cx) as usize] = old;
                landscape.grid_write_byte(lx, ly, vehicle);
            }
            xa += ma1;
            xb += mb1;
        }
        ya += ma2;
        yb += mb2;
    }
    Some(bake)
}

#[derive(Debug, Clone)]
struct SolidMaskRect {
    object_id: ObjectId,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pixels: Option<Arc<Vec<u8>>>,
}

impl SolidMaskRect {
    fn contains(&self, x: i32, y: i32) -> bool {
        if self.width <= 0 || self.height <= 0 {
            return false;
        }
        let local_x = i64::from(x) - i64::from(self.x);
        let local_y = i64::from(y) - i64::from(self.y);
        local_x >= 0
            && local_y >= 0
            && local_x < i64::from(self.width)
            && local_y < i64::from(self.height)
            && self
                .pixels
                .as_ref()
                .map(|pixels| {
                    let index = local_y as usize * self.width as usize + local_x as usize;
                    pixels.get(index).copied().unwrap_or(0) != 0
                })
                .unwrap_or(true)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct MovementStepOutcome {
    no_attach: bool,
    /// C++ DoMovement-local fRedirectYR: the vertical both-sides arm already
    /// transferred ydir into rdir, so a later rotation contact must not send
    /// rdir back into ydir during this invocation.
    redirect_yr: bool,
    any_contact: bool,
    /// The frame's accumulated contact CNAT bits (C++ `iContacts`,
    /// C4Movement.cpp:358) — ContactAction dispatches on them.
    contact_cnat: u32,
    solid_mask_removed: bool,
}

#[derive(Debug, Clone, Copy)]
#[doc(hidden)]
pub struct ExecMovementOutcome {
    #[doc(hidden)]
    pub alive: bool,
    /// Whether the positional integration reached C4Object::DoMotion.
    /// Only that path removes the solid mask with attachment backup;
    /// rotation-only updates use UpdateSolidMask(false) in C++.
    did_motion: bool,
}

#[derive(Debug, Clone, Copy)]
struct MovementContactConfig<'a> {
    live: &'a Cell<MovementLiveConfig>,
    /// Solid-mask raster/overlay visibility has its own synchronous callback
    /// fold. Keep this slice tied to that lifecycle rather than treating it as
    /// definition configuration.
    solid_masks: &'a [SolidMaskRect],
    object_id: ObjectId,
}

#[derive(Debug, Clone, Copy)]
struct MovementLiveConfig {
    border_bound: i32,
    rotateable: i32,
    action_procedure: ActionProcedure,
    action_is_idle: bool,
    layer_bounds: Option<LayerMovementBounds>,
}

#[derive(Debug, Clone, Copy)]
enum MovementContactDispatch {
    ShapeProbe,
    Direct(u32),
}

fn movement_live_config_for(
    object: &Object,
    definitions: &HashMap<DefinitionId, Definition>,
    layer_bounds: Option<LayerMovementBounds>,
) -> MovementLiveConfig {
    let definition = definitions.get(&object.definition_id);
    let action_library = definition.map(Definition::action_library);
    MovementLiveConfig {
        border_bound: definition.map(Definition::border_bound).unwrap_or(0),
        rotateable: definition.map(Definition::rotateable).unwrap_or(0),
        action_procedure: action_library
            .map(|library| {
                library.procedure_for_entry(
                    &object.state.action.name,
                    object.state.action.act_map_index,
                )
            })
            .unwrap_or(ActionProcedure::Undefined),
        action_is_idle: action_library
            .is_none_or(|library| library.is_idle_state(&object.state.action)),
        layer_bounds,
    }
}

fn layer_movement_bounds_from_split(
    object: &Object,
    objects_before: &[Object],
    objects_after: &[Object],
    definitions: &HashMap<DefinitionId, Definition>,
) -> Option<LayerMovementBounds> {
    let layer_id = object.state.layer?;
    let layer = if object.id == layer_id {
        object
    } else {
        objects_before
            .iter()
            .chain(objects_after)
            .find(|candidate| candidate.id == layer_id)?
    };
    let definition = definitions.get(&layer.definition_id)?;
    Some(LayerMovementBounds {
        position: layer.position_pixels(),
        shape_rect: layer.current_shape_rect()?,
        border_bound: definition.border_bound(),
    })
}

#[derive(Debug, Clone, Copy)]
struct ContactVertexInfo {
    friction: i32,
}

#[derive(Debug, Clone, Default)]
struct ShapeContact {
    contact_cnat: u32,
    vertices: Vec<ContactVertexInfo>,
    /// Per-shape-vertex C4Shape::VtxContactCNAT values. Entries for
    /// CNAT_NoCollision vertices remain zero and are ignored by the latch.
    vertex_contacts: Vec<u32>,
}

impl ShapeContact {
    fn count(&self) -> i32 {
        self.vertices.len() as i32
    }

    fn is_contact(&self) -> bool {
        !self.vertices.is_empty()
    }

    fn first_friction(&self) -> i32 {
        self.vertices
            .first()
            .map(|vertex| vertex.friction)
            .unwrap_or(0)
    }
}

fn sign_i32(value: i32) -> i32 {
    value.signum()
}

/// LC_COACHDBG forensics gate (diagnostic only) — cached so the movement
/// contact loops don't pay an env lookup per abort.
fn coach_debug_enabled() -> bool {
    coach_debug_id().is_some()
}

/// TEMP stage probe for the traced object.
fn dbg_stage(object: &Object, stage: &str) {
    if coach_debug_id() == Some(object.id.as_u64()) {
        crate::rng::rng_trace_line(&format!(
            "STG {stage} pos=({},{}) fix=({},{}) dirs=({},{}) act={} comdir={:?} dir={:?} mobile={}",
            object.state.position.x,
            object.state.position.y,
            object.fixed_position.x.val(),
            object.fixed_position.y.val(),
            object.fixed_velocity.x.val(),
            object.fixed_velocity.y.val(),
            object.state.action.name,
            object.state.command_direction,
            object.state.direction,
            object.state.mobile
        ));
    }
}

/// LC_COACHDBG's traced object id: `LC_COACHDBG=<id>`, any non-numeric
/// value keeps the original coach target 1450 (diagnostic only).
fn coach_debug_id() -> Option<u64> {
    static ID: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *ID.get_or_init(|| {
        std::env::var("LC_COACHDBG")
            .ok()
            .map(|raw| raw.trim().parse::<u64>().unwrap_or(1450))
    })
}

fn redirect_force(from: &mut C4Fixed, to: &mut C4Fixed, direction: i32) {
    let redirect = fixed100(50);
    let magnitude =
        C4Fixed::from_raw(saturating_i64_to_i32(i64::from(from.val()).abs())).min(redirect);
    if magnitude == C4Fixed::ZERO {
        return;
    }
    *from -= magnitude * from.val().signum();
    *to += magnitude * direction;
}

fn apply_contact_friction(value: &mut C4Fixed, percent: i32) {
    let friction = fixed100(30) * percent / 100;
    if *value > friction {
        *value -= friction;
    } else if *value < -friction {
        *value += friction;
    } else {
        *value = C4Fixed::ZERO;
    }
}

fn contact_callback_name(cnat: u32) -> Option<&'static str> {
    match cnat {
        CNAT_LEFT => Some("ContactLeft"),
        CNAT_RIGHT => Some("ContactRight"),
        CNAT_TOP => Some("ContactTop"),
        CNAT_BOTTOM => Some("ContactBottom"),
        _ => None,
    }
}

fn contact_action_wall_tumble_x(cnat: u32) -> C4Fixed {
    match cnat {
        CNAT_LEFT => math::fixed100(150),
        CNAT_RIGHT => -math::fixed100(150),
        _ => C4Fixed::ZERO,
    }
}

#[doc(hidden)]
pub fn movement_hit_speed_flags(velocity: FixedVec2) -> u32 {
    let speed = i64::from(velocity.x.val()).abs() + i64::from(velocity.y.val()).abs();
    let mut flags = 0;
    if speed >= i64::from(fixed100(150).val()) {
        flags |= crate::ocf::HIT_SPEED1;
    }
    if speed >= i64::from(itofix(2).val()) {
        flags |= crate::ocf::HIT_SPEED2;
    }
    if speed >= i64::from(itofix(6).val()) {
        flags |= crate::ocf::HIT_SPEED3;
    }
    if speed >= i64::from(itofix(8).val()) {
        flags |= crate::ocf::HIT_SPEED4;
    }
    flags
}

fn construction_percent(construction: i32) -> i32 {
    ((i64::from(construction.max(0)) * 100) / i64::from(FULL_CON)) as i32
}

fn scaled_shape_fire_top(fire_top: i32, construction: i32, line: i32) -> i32 {
    if line != 0 || construction == FULL_CON {
        return fire_top;
    }
    let percent = construction_percent(construction);
    saturating_i64_to_i32(i64::from(fire_top) * i64::from(percent) / 100)
}

fn construction_scaled_vertices(
    vertices: &[ObjectVertex],
    construction: i32,
    stretch_growth: bool,
) -> Vec<ObjectVertex> {
    let percent = construction_percent(construction);
    vertices
        .iter()
        .map(|vertex| {
            let mut scaled = *vertex;
            if stretch_growth {
                scaled.x = scaled.x * percent / 100;
            }
            scaled.y = scaled.y * percent / 100;
            scaled
        })
        .collect()
}

fn transformed_shape_vertices(
    vertices: &[ObjectVertex],
    construction: i32,
    stretch_growth: bool,
    rotateable: i32,
    rotation: i32,
) -> Vec<ObjectVertex> {
    let scaled = if construction == FULL_CON {
        vertices.to_vec()
    } else {
        construction_scaled_vertices(vertices, construction, stretch_growth)
    };
    if rotateable != 0 && rotation != 0 {
        rotated_vertices(&scaled, rotation)
    } else {
        scaled
    }
}

/// DoCon's post-callback integer-y adjustment. Straight objects retain the
/// shape bottom captured on entry; rotated structures move upward by the
/// positive construction-step delta and the current definition height
/// (C4Object.cpp:1475-1502).
pub(crate) fn docon_adjusted_position_y(
    entry_y: i32,
    entry_shape: Option<DefinitionRect>,
    current_y: i32,
    current_shape: Option<DefinitionRect>,
    rotation: i32,
    category: i32,
    previous_step: i32,
    step_diff: i32,
    definition_height: i32,
) -> i32 {
    // C++ tests the raw integer `r` here (`if (!r)`), so a loaded r=360
    // takes the rotated branch even though it has the same orientation.
    if rotation == 0 {
        if let (Some(previous), Some(current)) = (entry_shape, current_shape) {
            if previous.height != current.height || previous.y != current.y {
                let bottom = entry_y
                    .saturating_add(previous.y)
                    .saturating_add(previous.height);
                return bottom
                    .saturating_sub(current.height)
                    .saturating_sub(current.y);
            }
        }
    } else if category & CATEGORY_STRUCTURE != 0 && step_diff > 0 {
        let previous_lift = previous_step.saturating_mul(definition_height) / 100;
        let current_lift = previous_step
            .saturating_add(step_diff)
            .saturating_mul(definition_height)
            / 100;
        return current_y.saturating_sub(current_lift.saturating_sub(previous_lift));
    }
    current_y
}

/// The C4Object::DoCon(fInitial) straight-con bottom adjust
/// (C4Object.cpp:1463-1470): strgt_con_b is computed from the ENTRY
/// (con-0) shape, and the move fires ONLY when the con growth actually
/// resized the shape — a def whose con-0 shape equals its full shape
/// (1x1 connect beams) keeps the given center.
pub(crate) fn docon_initial_center_y(
    shape: Option<DefinitionRect>,
    stretch_growth: bool,
    line: i32,
    construction: i32,
    given_y: i32,
) -> i32 {
    docon_initial_center_y_with_rotation(shape, stretch_growth, line, 0, 0, construction, given_y)
}

/// Rotation-aware form used by direct `C4Game::CreateObject` host paths.
/// The ordinary script `CreateObject` starts at zero rotation; engine sites
/// such as `Split2Components` supply a sampled initial angle before the
/// initial `DoCon(FullCon, true)` keeps the con-zero bottom fixed.
pub(crate) fn docon_initial_center_y_with_rotation(
    shape: Option<DefinitionRect>,
    stretch_growth: bool,
    line: i32,
    rotateable: i32,
    rotation: i32,
    construction: i32,
    given_y: i32,
) -> i32 {
    // Line objects never con-scale their shape (C4Object::UpdateShape
    // returns early for Def->Line, C4Object.cpp:322-324): no adjust.
    if line != 0 {
        return given_y;
    }
    let zero = transformed_shape_rect(shape, 0, stretch_growth, rotateable, rotation);
    let grown = transformed_shape_rect(
        shape,
        construction.max(0),
        stretch_growth,
        rotateable,
        rotation,
    );
    match (zero, grown) {
        (Some(zero), Some(grown)) if zero.height != grown.height || zero.y != grown.y => {
            given_y + zero.y + zero.height - grown.height - grown.y
        }
        _ => given_y,
    }
}

fn transformed_shape_rect(
    rect: Option<DefinitionRect>,
    construction: i32,
    stretch_growth: bool,
    rotateable: i32,
    rotation: i32,
) -> Option<DefinitionRect> {
    let mut rect = rect?;
    if construction != FULL_CON {
        let percent = construction_percent(construction);
        if stretch_growth {
            rect.x = rect.x * percent / 100;
            rect.width = rect.width * percent / 100;
        }
        rect.y = rect.y * percent / 100;
        rect.height = rect.height * percent / 100;
    }
    if rotateable != 0 && rotation != 0 {
        let radius = ((i64::from(rect.x) * i64::from(rect.x)
            + i64::from(rect.y) * i64::from(rect.y)) as f64)
            .sqrt() as i32
            + 2;
        rect.x = -radius;
        rect.y = -radius;
        rect.width = 2 * radius;
        rect.height = 2 * radius;
    }
    Some(rect)
}

fn rotated_vertices(vertices: &[ObjectVertex], rotation: i32) -> Vec<ObjectVertex> {
    if rotation.rem_euclid(360) == 0 {
        return vertices.to_vec();
    }
    let angle = itofix(rotation.rem_euclid(360));
    let cos = angle.cos_deg();
    let sin = angle.sin_deg();
    vertices
        .iter()
        .map(|vertex| {
            let x = fixtoi(cos * vertex.x - sin * vertex.y);
            let y = fixtoi(sin * vertex.x + cos * vertex.y);
            ObjectVertex {
                x,
                y,
                cnat: vertex.cnat,
                friction: vertex.friction,
            }
        })
        .collect()
}

fn movement_density_at(
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    x: i32,
    y: i32,
) -> i32 {
    if solid_masks
        .iter()
        .any(|mask| Some(mask.object_id) != excluded_solid_mask && mask.contains(x, y))
    {
        return C4M_VEHICLE;
    }
    landscape.density_at(x, y, materials)
}

fn movement_is_vehicle_at(
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    x: i32,
    y: i32,
) -> bool {
    solid_masks
        .iter()
        .any(|mask| Some(mask.object_id) != excluded_solid_mask && mask.contains(x, y))
        || materials
            .id_of("Vehicle")
            .is_some_and(|vehicle| landscape.border_material_at(x, y) == Some(vehicle))
}

/// The SCRIPT PathFree (FnPathFree → ::PathFree, C4Landscape.cpp:
/// 2052-2055): the ForLine per-pixel Bresenham (:1683-1738) where any
/// GBackSolid pixel blocks. GBackSolid sees the baked C4SolidMask
/// MCVehic pixels — the rust mask overlay joins via movement_density_at.
pub(crate) fn path_free_exact(
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> bool {
    path_free_exact_hit(landscape, materials, solid_masks, x1, y1, x2, y2).is_none()
}

/// The blocking-coordinate form of SCRIPT PathFree used by FnPathFree2.
/// C++ ForLine normalizes traversal toward the increasing major axis, so the
/// reported blocker is not necessarily the one nearest the caller's start.
pub(crate) fn path_free_exact_hit(
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
) -> Option<Vector2> {
    for_line_first_blocker(x1, y1, x2, y2, |x, y| {
        movement_density_at(landscape, materials, solid_masks, None, x, y) >= 50
    })
}

/// C4Landscape.cpp ForLine (lines 1670-1738), including endpoint swapping,
/// tie handling, and the exact point passed back through `lastx`/`lasty`.
pub(crate) fn for_line_first_blocker(
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    mut blocked: impl FnMut(i32, i32) -> bool,
) -> Option<Vector2> {
    let (mut x1, mut y1, mut x2, mut y2) = (x1, y1, x2, y2);
    if (x2 - x1).abs() < (y2 - y1).abs() {
        if y1 > y2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y1, &mut y2);
        }
        let xincr = if x2 > x1 { 1 } else { -1 };
        let dy = y2 - y1;
        let dx = (x2 - x1).abs();
        let mut d = 2 * dx - dy;
        let aincr = 2 * (dx - dy);
        let bincr = 2 * dx;
        let mut x = x1;
        if blocked(x, y1) {
            return Some(Vector2::new(x, y1));
        }
        for y in (y1 + 1)..=y2 {
            if d >= 0 {
                x += xincr;
                d += aincr;
            } else {
                d += bincr;
            }
            if blocked(x, y) {
                return Some(Vector2::new(x, y));
            }
        }
    } else {
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y1, &mut y2);
        }
        let yincr = if y2 > y1 { 1 } else { -1 };
        let dx = x2 - x1;
        let dy = (y2 - y1).abs();
        let mut d = 2 * dy - dx;
        let aincr = 2 * (dy - dx);
        let bincr = 2 * dy;
        let mut y = y1;
        if blocked(x1, y) {
            return Some(Vector2::new(x1, y));
        }
        for x in (x1 + 1)..=x2 {
            if d >= 0 {
                y += yincr;
                d += aincr;
            } else {
                d += bincr;
            }
            if blocked(x, y) {
                return Some(Vector2::new(x, y));
            }
        }
    }
    None
}

fn shape_contact_check(
    vertices: &[ObjectVertex],
    position: Vector2,
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    contact_density: i32,
) -> ShapeContact {
    let mut contact = ShapeContact {
        vertex_contacts: vec![0; vertices.len()],
        ..ShapeContact::default()
    };
    for (index, vertex) in vertices.iter().enumerate() {
        if vertex.cnat & CNAT_NO_COLLISION != 0 {
            continue;
        }
        let x = position.x + vertex.x;
        let y = position.y + vertex.y;
        if movement_density_at(landscape, materials, solid_masks, excluded_solid_mask, x, y)
            < contact_density
        {
            continue;
        }

        contact.contact_cnat |= vertex.cnat;
        let mut vertex_contact = CNAT_CENTER;
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x,
            y - 1,
        ) >= contact_density
        {
            vertex_contact |= CNAT_TOP;
        }
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x,
            y + 1,
        ) >= contact_density
        {
            vertex_contact |= CNAT_BOTTOM;
        }
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x - 1,
            y,
        ) >= contact_density
        {
            vertex_contact |= CNAT_LEFT;
        }
        if movement_density_at(
            landscape,
            materials,
            solid_masks,
            excluded_solid_mask,
            x + 1,
            y,
        ) >= contact_density
        {
            vertex_contact |= CNAT_RIGHT;
        }
        contact.vertices.push(ContactVertexInfo {
            friction: vertex.friction,
        });
        contact.vertex_contacts[index] = vertex_contact;
    }
    contact
}

fn attach_direction(attach: u32) -> (i32, i32) {
    match attach & !CNAT_FLAGS {
        CNAT_TOP => (0, -1),
        CNAT_BOTTOM => (0, 1),
        CNAT_LEFT => (-1, 0),
        CNAT_RIGHT => (1, 0),
        _ => (0, 0),
    }
}

/// The C4Object::ExecAction per-procedure Action.t_attach map
/// (C4Object.cpp:4690-5437): the switch arms OR procedure-specific
/// attach bits over the pre-switch state (CNAT_None | UprightAttach |
/// the ActMap Attach's MultiAttach flag, :4703/:4758) — only the
/// no-procedure default case applies the FULL ActMap Attach (:5427);
/// DIG and SWIM RESET the whole register to CNAT_None (:4916/:4967).
#[doc(hidden)]
pub fn procedure_t_attach(
    procedure: ActionProcedure,
    is_idle: bool,
    direction: Direction,
    actmap_attach: u32,
    upright_attach: u32,
) -> u32 {
    if is_idle {
        return upright_attach;
    }
    let base = upright_attach | (actmap_attach & CNAT_MULTI_ATTACH);
    match procedure {
        ActionProcedure::Walk
        | ActionProcedure::Kneel
        | ActionProcedure::Throw
        | ActionProcedure::Bridge
        | ActionProcedure::Push
        | ActionProcedure::Pull
        | ActionProcedure::Chop
        | ActionProcedure::Fight => base | CNAT_BOTTOM,
        ActionProcedure::Scale => {
            let mut attach = base;
            if direction == Direction::Left {
                attach |= CNAT_LEFT;
            }
            if direction == Direction::Right {
                attach |= CNAT_RIGHT;
            }
            attach
        }
        ActionProcedure::Hang => base | CNAT_TOP,
        ActionProcedure::Dig | ActionProcedure::Swim => CNAT_NONE,
        ActionProcedure::Undefined | ActionProcedure::Other => upright_attach | actmap_attach,
        _ => base,
    }
}

#[allow(clippy::too_many_arguments)]
fn shape_attach(
    vertices: &[ObjectVertex],
    position: &mut Vector2,
    attach: u32,
    landscape: &Landscape,
    materials: &MaterialSet,
    solid_masks: &[SolidMaskRect],
    excluded_solid_mask: Option<ObjectId>,
    contact_density: i32,
    record: &mut ShapeAttachRecord,
) -> bool {
    // C4Shape::Attach receives Action.t_attach through a uint8_t parameter,
    // so signed/high-bit UprightAttach values are narrowed at this boundary.
    let attach = u32::from(attach as u8);
    // C4Shape::Attach resets AttachMat to MNone up front; the position/
    // vertex fields only overwrite on success (C4Shape.cpp:176,217-219,
    // 253-255).
    record.mat_valid = false;
    record.mat_vehicle = false;
    let (xcd, ycd) = attach_direction(attach);
    if xcd == 0 && ycd == 0 {
        return false;
    }
    let xcrng = -(ATTACH_RANGE * xcd);
    let ycrng = -(ATTACH_RANGE * ycd);
    let mut attached = false;

    if attach & CNAT_MULTI_ATTACH == 0 {
        for (vtx, vertex) in vertices.iter().enumerate() {
            if vertex.cnat & attach == 0 {
                continue;
            }
            let mut xcnt = xcrng;
            let mut ycnt = ycrng;
            while xcnt != -xcrng || ycnt != -ycrng {
                let ax = position.x + vertex.x + xcnt + xcd;
                let ay = position.y + vertex.y + ycnt + ycd;
                if ax >= 0
                    && ax < landscape.width() as i32
                    && movement_density_at(
                        landscape,
                        materials,
                        solid_masks,
                        excluded_solid_mask,
                        ax,
                        ay,
                    ) >= contact_density
                {
                    *record = ShapeAttachRecord {
                        mat_valid: true,
                        mat_vehicle: movement_is_vehicle_at(
                            landscape,
                            materials,
                            solid_masks,
                            excluded_solid_mask,
                            ax,
                            ay,
                        ),
                        x: ax,
                        y: ay,
                        vtx: vtx as i32,
                    };
                    position.x += xcnt;
                    position.y += ycnt;
                    attached = true;
                    break;
                }
                xcnt += xcd;
                ycnt += ycd;
            }
        }
    } else {
        let mut xcnt = xcrng;
        let mut ycnt = ycrng;
        'search: while xcnt != -xcrng || ycnt != -ycrng {
            for (vtx, vertex) in vertices.iter().enumerate() {
                if vertex.cnat & attach == 0 {
                    continue;
                }
                let ax = position.x + vertex.x + xcnt + xcd;
                let ay = position.y + vertex.y + ycnt + ycd;
                if ax >= 0
                    && ax < landscape.width() as i32
                    && movement_density_at(
                        landscape,
                        materials,
                        solid_masks,
                        excluded_solid_mask,
                        ax,
                        ay,
                    ) >= contact_density
                {
                    *record = ShapeAttachRecord {
                        mat_valid: true,
                        mat_vehicle: movement_is_vehicle_at(
                            landscape,
                            materials,
                            solid_masks,
                            excluded_solid_mask,
                            ax,
                            ay,
                        ),
                        x: ax,
                        y: ay,
                        vtx: vtx as i32,
                    };
                    position.x += xcnt;
                    position.y += ycnt;
                    attached = true;
                    break 'search;
                }
            }
            xcnt += xcd;
            ycnt += ycd;
        }
    }

    attached
}

/// C4Object::TargetBounds (C4Movement.cpp:128-163): clamps the INT step
/// target and returns its low/high contacts in execution order. The checks
/// are independent, so inverted limits can trigger both and finish at
/// `high`. Callers zero the corresponding dir before each callback;
/// fix_x/fix_y are not resynchronized.
fn target_bounds(
    target: &mut i32,
    low: i32,
    high: i32,
    cnat_low: u32,
    cnat_high: u32,
) -> [Option<u32>; 2] {
    let mut contacts = [None, None];
    if *target < low {
        *target = low;
        contacts[0] = Some(cnat_low);
    }
    if *target > high {
        *target = high;
        contacts[1] = Some(cnat_high);
    }
    contacts
}

/// DFA_FLOAT ComDir movement with a nonzero Float physical
/// (C4Object.cpp:5268-5286): `xdir/ydir ± FloatAccel` per ComDir (Stop
/// drifts — no deceleration case), both axes clamped to
/// `lLimit = FIXED100(Float)` (not ValByPhysical). DFA_FLOAT never applies
/// gravity. Physical-less fixture definitions keep the legacy
/// `MovementProfile` path instead.
fn apply_float_physical_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    limit: C4Fixed,
) {
    match command_direction {
        CommandDirection::Up => velocity.y -= math::FLOAT_ACCEL,
        CommandDirection::Down => velocity.y += math::FLOAT_ACCEL,
        CommandDirection::Right => velocity.x += math::FLOAT_ACCEL,
        CommandDirection::Left => velocity.x -= math::FLOAT_ACCEL,
        CommandDirection::UpRight => {
            velocity.y -= math::FLOAT_ACCEL;
            velocity.x += math::FLOAT_ACCEL;
        }
        CommandDirection::DownRight => {
            velocity.y += math::FLOAT_ACCEL;
            velocity.x += math::FLOAT_ACCEL;
        }
        CommandDirection::DownLeft => {
            velocity.y += math::FLOAT_ACCEL;
            velocity.x -= math::FLOAT_ACCEL;
        }
        CommandDirection::UpLeft => {
            velocity.y -= math::FLOAT_ACCEL;
            velocity.x -= math::FLOAT_ACCEL;
        }
        CommandDirection::Stop => {}
        _ => {}
    }

    // xdir/ydir bounds (C4Object.cpp:5284-5285).
    if velocity.y < -limit {
        velocity.y = -limit;
    }
    if velocity.y > limit {
        velocity.y = limit;
    }
    if velocity.x > limit {
        velocity.x = limit;
    }
    if velocity.x < -limit {
        velocity.x = -limit;
    }
}

fn apply_float_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
) {
    let (dx, dy) = command_direction.axis_components();
    let accel = itofix(profile.float_acceleration.max(0));

    if dx != 0 && accel > C4Fixed::ZERO {
        velocity.x = clamp_fixed_to_limit(velocity.x + accel * dx, profile.float_speed);
    } else {
        velocity.x = clamp_fixed_to_limit(velocity.x, profile.float_speed);
    }

    if dy != 0 && accel > C4Fixed::ZERO {
        velocity.y = clamp_fixed_to_limit(velocity.y + accel * dy, profile.float_speed);
    } else {
        velocity.y = clamp_fixed_to_limit(velocity.y, profile.float_speed);
    }
}

fn decelerate_fixed_toward_zero(value: C4Fixed, accel: C4Fixed) -> C4Fixed {
    if accel <= C4Fixed::ZERO {
        return value;
    }
    if value > C4Fixed::ZERO {
        (value - accel).max(C4Fixed::ZERO)
    } else if value < C4Fixed::ZERO {
        (value + accel).min(C4Fixed::ZERO)
    } else {
        C4Fixed::ZERO
    }
}

/// DFA_WALK ComDir movement with a nonzero Walk physical
/// (C4Object.cpp:4771-4786): `xdir ± WalkAccel`, clamped per branch to
/// `lLimit = ValByPhysical(280, Walk)`; Stop/Up/Down decelerate and snap to
/// zero within WalkAccel. Physical-less fixture definitions keep the legacy
/// `MovementProfile` path instead.
#[doc(hidden)]
pub fn apply_walk_physical_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    limit: C4Fixed,
) {
    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            velocity.x -= math::WALK_ACCEL;
            if velocity.x < -limit {
                velocity.x = -limit;
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            velocity.x += math::WALK_ACCEL;
            if velocity.x > limit {
                velocity.x = limit;
            }
        }
        CommandDirection::Stop | CommandDirection::Up | CommandDirection::Down => {
            if velocity.x < C4Fixed::ZERO {
                velocity.x += math::WALK_ACCEL;
            }
            if velocity.x > C4Fixed::ZERO {
                velocity.x -= math::WALK_ACCEL;
            }
            if velocity.x > -math::WALK_ACCEL && velocity.x < math::WALK_ACCEL {
                velocity.x = C4Fixed::ZERO;
            }
        }
        _ => {}
    }
}

/// C4Object::AdjustWalkRotation (C4Object.cpp:6031-6097): derive the
/// angular velocity that steers an attached walker toward the floor slope.
/// The caller owns the Rotateable/AttachMat/xdir gate because the script
/// wrapper and DFA_WALK use different guards.
#[allow(clippy::too_many_arguments)]
fn calculate_walk_rotation_velocity(
    rotation: i32,
    attach: ShapeAttachRecord,
    def_attach_vtx_x: i32,
    live_attach_vtx_x: i32,
    range_x: i32,
    range_y: i32,
    speed: i32,
    mut solid: impl FnMut(i32, i32) -> bool,
) -> C4Fixed {
    let dest_angle = if attach.vtx < 0 || def_attach_vtx_x == 0 {
        let mut probe = |x_check: i32| -> i32 {
            let mut offset = 0i32;
            if solid(x_check, attach.y) {
                loop {
                    offset -= 1;
                    if offset <= -range_y {
                        break;
                    }
                    if solid(x_check, attach.y + offset) {
                        offset += 1;
                        break;
                    }
                }
            } else {
                loop {
                    offset += 1;
                    if offset >= range_y {
                        break;
                    }
                    if solid(x_check, attach.y + offset) {
                        offset -= 1;
                        break;
                    }
                }
            }
            offset
        };
        let solid_left = probe(attach.x - range_x);
        let solid_right = probe(attach.x + range_x);
        (solid_right - solid_left) * (35 / range_x.max(1))
    } else if live_attach_vtx_x > 0 {
        -50
    } else {
        50
    };

    if (dest_angle - rotation).abs() <= 2 {
        return C4Fixed::ZERO;
    }
    let bounded = itofix((dest_angle - rotation).clamp(-15, 15));
    let divisor = if speed != 0 { 10000 / speed } else { 0 };
    if divisor != 0 {
        bounded / divisor
    } else {
        C4Fixed::ZERO
    }
}

fn apply_walk_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
) {
    let accel = itofix(profile.walk_acceleration.max(0));
    let limit = profile.walk_speed;

    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            if accel > C4Fixed::ZERO {
                velocity.x -= accel;
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            if accel > C4Fixed::ZERO {
                velocity.x += accel;
            }
        }
        CommandDirection::Stop | CommandDirection::Up | CommandDirection::Down
            if accel > C4Fixed::ZERO =>
        {
            velocity.x = decelerate_fixed_toward_zero(velocity.x, accel);
        }
        _ => {}
    }

    velocity.x = clamp_fixed_to_limit(velocity.x, limit);
}

/// DFA_SWIM ComDir movement with a nonzero Swim physical
/// (C4Object.cpp:4920-4965): `xdir/ydir ± SwimAccel` per ComDir (diagonals
/// drive both axes), Stop decelerates both axes and snaps, then both axes
/// clamp to `lLimit = ValByPhysical(160, Swim)`; facing follows the xdir
/// sign. DFA_SWIM never applies gravity. The InLiquid exit checks and the
/// surface ydir bound still need the liquid model. Physical-less fixture
/// definitions keep the legacy `MovementProfile` path instead.
fn apply_swim_physical_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    limit: C4Fixed,
) -> Option<Direction> {
    match command_direction {
        CommandDirection::Up => velocity.y -= math::SWIM_ACCEL,
        CommandDirection::UpRight => {
            velocity.y -= math::SWIM_ACCEL;
            velocity.x += math::SWIM_ACCEL;
        }
        CommandDirection::Right => velocity.x += math::SWIM_ACCEL,
        CommandDirection::DownRight => {
            velocity.y += math::SWIM_ACCEL;
            velocity.x += math::SWIM_ACCEL;
        }
        CommandDirection::Down => velocity.y += math::SWIM_ACCEL,
        CommandDirection::DownLeft => {
            velocity.y += math::SWIM_ACCEL;
            velocity.x -= math::SWIM_ACCEL;
        }
        CommandDirection::Left => velocity.x -= math::SWIM_ACCEL,
        CommandDirection::UpLeft => {
            velocity.y -= math::SWIM_ACCEL;
            velocity.x -= math::SWIM_ACCEL;
        }
        CommandDirection::Stop => {
            if velocity.x < C4Fixed::ZERO {
                velocity.x += math::SWIM_ACCEL;
            }
            if velocity.x > C4Fixed::ZERO {
                velocity.x -= math::SWIM_ACCEL;
            }
            if velocity.x > -math::SWIM_ACCEL && velocity.x < math::SWIM_ACCEL {
                velocity.x = C4Fixed::ZERO;
            }
            if velocity.y < C4Fixed::ZERO {
                velocity.y += math::SWIM_ACCEL;
            }
            if velocity.y > C4Fixed::ZERO {
                velocity.y -= math::SWIM_ACCEL;
            }
            if velocity.y > -math::SWIM_ACCEL && velocity.y < math::SWIM_ACCEL {
                velocity.y = C4Fixed::ZERO;
            }
        }
        _ => {}
    }

    // xdir/ydir bounds (C4Object.cpp:4959-4960).
    if velocity.y < -limit {
        velocity.y = -limit;
    }
    if velocity.y > limit {
        velocity.y = limit;
    }
    if velocity.x > limit {
        velocity.x = limit;
    }
    if velocity.x < -limit {
        velocity.x = -limit;
    }

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

fn apply_swim_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    gravity_component: C4Fixed,
) {
    let accel = itofix(profile.swim_acceleration.max(0));
    let limit = profile.swim_speed;

    match command_direction {
        CommandDirection::Stop => {
            if accel > C4Fixed::ZERO {
                velocity.x = decelerate_fixed_toward_zero(velocity.x, accel);
                let vertical_without_gravity = velocity.y - gravity_component;
                let decelerated = decelerate_fixed_toward_zero(vertical_without_gravity, accel);
                velocity.y = decelerated + gravity_component;
            }
        }
        _ => {
            if accel > C4Fixed::ZERO {
                let (dx, dy) = command_direction.axis_components();
                if dx != 0 {
                    velocity.x += accel * dx;
                }
                if dy != 0 {
                    velocity.y += accel * dy;
                }
            }
        }
    }

    velocity.x = clamp_fixed_to_limit(velocity.x, limit);
    velocity.y = clamp_fixed_to_limit(velocity.y, limit);
}

/// DFA_SCALE ComDir movement with a nonzero Scale physical
/// (C4Object.cpp:4805-4837): ComDir into the facing wall converts to Up,
/// `ydir ± WalkAccel` clamped per branch to `lLimit = ValByPhysical(200,
/// Scale)`, Left/Right/Stop decelerate and snap, `xdir = 0`. Physical-less
/// fixture definitions keep the legacy `MovementProfile` path instead.
fn apply_scale_physical_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    limit: C4Fixed,
    facing: Direction,
) {
    let effective_direction = match (facing, command_direction) {
        (Direction::Left, CommandDirection::Left) | (Direction::Right, CommandDirection::Right) => {
            CommandDirection::Up
        }
        _ => command_direction,
    };
    match effective_direction {
        CommandDirection::Up | CommandDirection::UpRight | CommandDirection::UpLeft => {
            velocity.y -= math::WALK_ACCEL;
            if velocity.y < -limit {
                velocity.y = -limit;
            }
        }
        CommandDirection::Down | CommandDirection::DownRight | CommandDirection::DownLeft => {
            velocity.y += math::WALK_ACCEL;
            if velocity.y > limit {
                velocity.y = limit;
            }
        }
        CommandDirection::Left | CommandDirection::Right | CommandDirection::Stop => {
            if velocity.y < C4Fixed::ZERO {
                velocity.y += math::WALK_ACCEL;
            }
            if velocity.y > C4Fixed::ZERO {
                velocity.y -= math::WALK_ACCEL;
            }
            if velocity.y > -math::WALK_ACCEL && velocity.y < math::WALK_ACCEL {
                velocity.y = C4Fixed::ZERO;
            }
        }
        _ => {}
    }
    velocity.x = C4Fixed::ZERO;
}

fn apply_scale_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) {
    let accel = itofix(profile.scale_acceleration.max(0));
    let limit = profile.scale_speed;
    let effective_direction = match (facing, command_direction) {
        (Direction::Left, CommandDirection::Left) | (Direction::Right, CommandDirection::Right) => {
            CommandDirection::Up
        }
        _ => command_direction,
    };

    match effective_direction {
        CommandDirection::Up | CommandDirection::UpLeft | CommandDirection::UpRight => {
            if accel > C4Fixed::ZERO {
                velocity.y -= accel;
            }
        }
        CommandDirection::Down | CommandDirection::DownLeft | CommandDirection::DownRight => {
            if accel > C4Fixed::ZERO {
                velocity.y += accel;
            }
        }
        CommandDirection::Left | CommandDirection::Right | CommandDirection::Stop
            if accel > C4Fixed::ZERO =>
        {
            velocity.y = decelerate_fixed_toward_zero(velocity.y, accel);
        }
        _ => {}
    }

    velocity.y = clamp_fixed_to_limit(velocity.y, limit);
    velocity.x = C4Fixed::ZERO;
}

/// DFA_HANGLE ComDir movement with a nonzero Hangle physical
/// (C4Object.cpp:4840-4872): `xdir ± WalkAccel` clamped per branch to
/// `lLimit = ValByPhysical(160, Hangle)`; Up moves in the facing direction
/// (clamped both sides), Stop/Down decelerate and snap, `ydir = 0`, facing
/// follows the xdir sign. Physical-less fixture definitions keep the legacy
/// `MovementProfile` path instead.
fn apply_hangle_physical_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    limit: C4Fixed,
    facing: Direction,
) -> Option<Direction> {
    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            velocity.x -= math::WALK_ACCEL;
            if velocity.x < -limit {
                velocity.x = -limit;
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            velocity.x += math::WALK_ACCEL;
            if velocity.x > limit {
                velocity.x = limit;
            }
        }
        CommandDirection::Up => {
            velocity.x += if matches!(facing, Direction::Left) {
                -math::WALK_ACCEL
            } else {
                math::WALK_ACCEL
            };
            if velocity.x < -limit {
                velocity.x = -limit;
            }
            if velocity.x > limit {
                velocity.x = limit;
            }
        }
        CommandDirection::Stop | CommandDirection::Down => {
            if velocity.x < C4Fixed::ZERO {
                velocity.x += math::WALK_ACCEL;
            }
            if velocity.x > C4Fixed::ZERO {
                velocity.x -= math::WALK_ACCEL;
            }
            if velocity.x > -math::WALK_ACCEL && velocity.x < math::WALK_ACCEL {
                velocity.x = C4Fixed::ZERO;
            }
        }
        _ => {}
    }
    velocity.y = C4Fixed::ZERO;

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

fn apply_hangle_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) -> Option<Direction> {
    let accel = itofix(profile.hangle_acceleration.max(0));
    let limit = profile.hangle_speed;

    match command_direction {
        CommandDirection::Left | CommandDirection::UpLeft | CommandDirection::DownLeft => {
            if accel > C4Fixed::ZERO {
                velocity.x -= accel;
            }
        }
        CommandDirection::Right | CommandDirection::UpRight | CommandDirection::DownRight => {
            if accel > C4Fixed::ZERO {
                velocity.x += accel;
            }
        }
        CommandDirection::Up => {
            if accel > C4Fixed::ZERO {
                if matches!(facing, Direction::Left) {
                    velocity.x -= accel;
                } else {
                    velocity.x += accel;
                }
            }
        }
        CommandDirection::Stop | CommandDirection::Down if accel > C4Fixed::ZERO => {
            velocity.x = decelerate_fixed_toward_zero(velocity.x, accel);
        }
        _ => {}
    }

    velocity.x = clamp_fixed_to_limit(velocity.x, limit);
    velocity.y = C4Fixed::ZERO;

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

/// DFA_DIG ComDir movement with a nonzero Dig physical
/// (C4Object.cpp:4888-4915): dirs assigned directly from
/// `lLimit = ValByPhysical(125, Dig)` — up components are `-lLimit/2`,
/// COMD_Up digs upward in the facing direction, Stop zeroes both axes;
/// facing follows the xdir sign. Physical-less fixture definitions keep the
/// legacy `MovementProfile` path instead.
fn apply_dig_physical_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    limit: C4Fixed,
    facing: Direction,
) -> Option<Direction> {
    let half_up = -(limit / 2);
    match command_direction {
        CommandDirection::Up => {
            velocity.x = if matches!(facing, Direction::Left) {
                -limit
            } else {
                limit
            };
            velocity.y = half_up;
        }
        CommandDirection::UpLeft => {
            velocity.x = -limit;
            velocity.y = half_up;
        }
        CommandDirection::Left => {
            velocity.x = -limit;
            velocity.y = C4Fixed::ZERO;
        }
        CommandDirection::DownLeft => {
            velocity.x = -limit;
            velocity.y = limit;
        }
        CommandDirection::Down => {
            velocity.x = C4Fixed::ZERO;
            velocity.y = limit;
        }
        CommandDirection::DownRight => {
            velocity.x = limit;
            velocity.y = limit;
        }
        CommandDirection::Right => {
            velocity.x = limit;
            velocity.y = C4Fixed::ZERO;
        }
        CommandDirection::UpRight => {
            velocity.x = limit;
            velocity.y = half_up;
        }
        CommandDirection::Stop => {
            velocity.x = C4Fixed::ZERO;
            velocity.y = C4Fixed::ZERO;
            return None;
        }
        _ => {}
    }

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

fn apply_dig_command_movement(
    velocity: &mut FixedVec2,
    command_direction: CommandDirection,
    profile: MovementProfile,
    facing: Direction,
) -> Option<Direction> {
    let speed = profile.dig_speed.max(0);
    let half_speed = speed / 2;
    let speed = itofix(speed);
    let half_speed = itofix(half_speed);

    match command_direction {
        CommandDirection::Stop => {
            velocity.x = C4Fixed::ZERO;
            velocity.y = C4Fixed::ZERO;
            return None;
        }
        CommandDirection::Up => {
            velocity.x = if matches!(facing, Direction::Left) {
                -speed
            } else {
                speed
            };
            velocity.y = -half_speed;
        }
        CommandDirection::UpLeft => {
            velocity.x = -speed;
            velocity.y = -half_speed;
        }
        CommandDirection::Left => {
            velocity.x = -speed;
            velocity.y = C4Fixed::ZERO;
        }
        CommandDirection::DownLeft => {
            velocity.x = -speed;
            velocity.y = speed;
        }
        CommandDirection::Down => {
            velocity.x = C4Fixed::ZERO;
            velocity.y = speed;
        }
        CommandDirection::DownRight => {
            velocity.x = speed;
            velocity.y = speed;
        }
        CommandDirection::Right => {
            velocity.x = speed;
            velocity.y = C4Fixed::ZERO;
        }
        CommandDirection::UpRight => {
            velocity.x = speed;
            velocity.y = -half_speed;
        }
        _ => {}
    }

    if velocity.x < C4Fixed::ZERO {
        Some(Direction::Left)
    } else if velocity.x > C4Fixed::ZERO {
        Some(Direction::Right)
    } else {
        None
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

// `impl Engine` is split across these area files; each mounts under the
// crate root with `use super::*;` and its own `impl Engine { .. }` block,
// methods moved verbatim (see REFACTOR_PLAN.md, wave 2).
#[path = "engine/config.rs"]
mod engine_config;
#[path = "engine/players.rs"]
mod engine_players;
#[path = "engine/world.rs"]
mod engine_world;
#[path = "engine/script_exec.rs"]
mod engine_script_exec;
#[path = "engine/host_tables.rs"]
mod engine_host_tables;
#[path = "engine/game_over.rs"]
mod engine_game_over;
#[path = "engine/crew.rs"]
mod engine_crew;
#[path = "engine/definitions.rs"]
mod engine_definitions;
#[path = "engine/player_view.rs"]
mod engine_player_view;
#[path = "engine/tick.rs"]
mod engine_tick;
#[path = "engine/state.rs"]
mod engine_state;
#[path = "engine/movement.rs"]
mod engine_movement;
#[path = "engine/procedures.rs"]
mod engine_procedures;
#[path = "engine/economy.rs"]
mod engine_economy;
#[path = "engine/exec_order.rs"]
mod engine_exec_order;
#[path = "engine/solid_mask.rs"]
mod engine_solid_mask;
#[path = "engine/command_results.rs"]
mod engine_command_results;
#[path = "engine/landscape_ops.rs"]
mod engine_landscape_ops;
#[path = "engine/spawn_queue.rs"]
mod engine_spawn_queue;

impl Engine {
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    pub fn with_seed(seed: u64) -> Self {
        let script_string_registrations = clonk_script::new_string_registrations();
        let script_global_consts = clonk_script::new_global_variables();
        script_constants::register_script_constants_in_global_table(&script_global_consts);
        let mut engine = Self {
            definitions: HashMap::new(),
            definition_load_order: Vec::new(),
            runtime_definition_order: Rc::new(Vec::new()),
            script_globals: clonk_script::new_global_variables(),
            script_global_slots: clonk_script::new_global_slots(),
            script_global_consts,
            script_string_registrations: script_string_registrations.clone(),
            legacy_string_table: script_string_registrations,
            script_link_sources: Vec::new(),
            reloaded_global_definitions: Vec::new(),
            objects_generation: std::cell::Cell::new(1),
            object_index_cache: std::cell::RefCell::new((0, HashMap::new())),
            definition_metadata_cache: std::cell::RefCell::new(None),
            command_definition_snapshot_cache: std::cell::RefCell::new(None),
            fair_crew_physical_cache: Rc::new(RefCell::new(HashMap::new())),
            host_definition_tables_cache: std::cell::RefCell::new(None),
            solid_mask_metadata_cache: std::cell::RefCell::new(None),
            materials: MaterialSet::default(),
            materials_shared: std::cell::RefCell::new(None),
            objects: Vec::new(),
            next_object_id: 1,
            next_solid_mask_instance_sequence: 1,
            defer_solid_mask_updates: false,
            deferred_solid_mask_operations: Vec::new(),
            deferred_host_raster_preview: None,
            rng: {
                let mut rng = LcgRng::seed_from_u64(seed);
                rng.trace = std::env::var("LC_RUST_RNG_TRACE").is_ok();
                rng
            },
            scenario_script_go: false,
            scenario_script_counter: 0,
            random_seed: seed,
            max_players: None,
            startup_player_count: None,
            use_fair_crew: true,
            fair_crew_strength: 1_000,
            fair_crew_forced: false,
            allow_debug: true,
            debug_mode: false,
            network_game: false,
            network_control_mode: false,
            recording_active: false,
            replay_control: false,
            film_viewport_available: false,
            league_game: false,
            league_name: Rc::new(Vec::new()),
            player_info_league_progress_data: Rc::new(BTreeMap::new()),
            player_info_league_scores: Rc::new(BTreeMap::new()),
            control_host: true,
            player_info_updates: Rc::new(RefCell::new(Vec::new())),
            player_info_league_progress_updates: Vec::new(),
            pending_remove_player_controls: Vec::new(),
            pending_game_goal_menu_requests: Vec::new(),
            pause_game_requests: Rc::new(RefCell::new(Vec::new())),
            network_target_fps_requests: Rc::new(RefCell::new(Vec::new())),
            viewport_presentation_requests: Rc::new(RefCell::new(Vec::new())),
            edit_cursor_target: None,
            local_players: None,
            control_key_names: Rc::new(HashMap::new()),
            active_message_board_input: None,
            message_board_commands: vec![InitialNetworkMessageBoardCommand::speed()],
            exec_list: Vec::new(),
            exec_list_insert_generation: 0,
            inactive_exec_list: Vec::new(),
            pending_object_order_commands: Vec::new(),
            resort_any_object: false,
            exec_cursor: None,
            frame: 0,
            game_tick_delay_ms: Rc::new(std::cell::Cell::new(DEFAULT_GAME_TICK_DELAY_MS)),
            game_tick_delay_revision: Rc::new(
                std::cell::Cell::new(next_game_tick_delay_revision()),
            ),
            game_time: 0,
            time_go: false,
            landscape: None,
            sectors: None,
            physics: PhysicsSettings::default(),
            environment: EnvironmentSettings::default(),
            gamma: GammaControlState::default(),
            sky: None,
            global_effects: Vec::new(),
            particles: Vec::new(),
            particle_system: particles::ParticleSystem::default(),
            pxs_system: pxs::PxsSystem::default(),
            control_rate: 1,
            control_tick: 0,
            sync_rate: 100,
            do_sync: false,
            sync_checks: Vec::new(),
            mass_movers: MassMoverSet::new(),
            weather_events: Vec::new(),
            scenario_script: None,
            global_script_functions: None,
            global_script_function_order: Vec::new(),
            next_mission: NextMissionState::default(),
            restart_restore_info_mask: 0,
            game_over_triggered: false,
            game_evaluated: false,
            round_results: RoundResultsState::default(),
            objectives: ScenarioObjectives::default(),
            objective_check_counter: 0,
            players_registered: false,
            players: HashMap::new(),
            player_order: Vec::new(),
            pending_player_joins: HashMap::new(),
            last_player_info_id: 0,
            forced_control_style: None,
            forced_auto_context_menu: None,
            teams: Rc::new(Vec::new()),
            team_configuration: TeamConfiguration::default(),
            team_last_team_id: 0,
            team_max_script_players: 0,
            team_script_player_names: Vec::new(),
            team_random_team_count: 0,
            runtime_join_team_choice: false,
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            player_starts: vec![scenario::PlayerStart::default(); scenario::MAX_PLAYER_STARTS],
            standard_names: None,
            map_zoom: scenario::LegacyC4SVal::new(10, 0, 5, 15),
            scenario_values: Rc::new(scenario::ScenarioValueStore::default()),
            scenario_sections: HashMap::new(),
            scenario_section_order: Vec::new(),
            scenario_current_section_registered: false,
            current_scenario_section: "main".to_string(),
            last_scenario_section_flags: None,
            crew_rosters: HashMap::new(),
            crew_info_order: HashMap::new(),
            crew_object_infos: Rc::new(HashMap::new()),
            crew_ranks: Rc::new(HashMap::new()),
            crew_info_links: Rc::new(HashMap::new()),
            pending_legacy_object_infos: HashMap::new(),
            crew_info_control_counts: HashMap::new(),
            team_home_base_rule: false,
            needed_material_strings: Rc::new(NeededMaterialStrings::default()),
            object_no_dig_resource_string: Rc::new("%s cannot dig.".to_string()),
            construction_check_strings: Rc::new(ConstructionCheckStrings::default()),
            default_rank_names: Rc::new(us_default_rank_names()),
            construction_needs_material: false,
            structures_need_energy: false,
            structures_snow_in: false,
            flag_removeable: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            base_auto_sell_enabled: true,
            base_reject_entrance_enabled: true,
            base_regenerate_energy_enabled: true,
            base_extinguish_enabled: true,
            base_regenerate_energy_price: 5,
            landscape_insert_thrust: false,
            known_crew_owners: HashSet::new(),
            eliminated_crew_owners: HashSet::new(),
            transfer_zones: TransferZoneTable::default(),
            pathfinder_level: 1,
            pathfinder_transfer_zones_enabled: true,
            pathfinder_debug: Rc::new(RefCell::new(PathfinderDebugSnapshot::default())),
            audio_registry: AudioRegistry::new(),
            pending_audio: Vec::new(),
            pending_menu_requests: Vec::new(),
            messages: MessageManager::new(),
            mission_access: MissionAccessStore::default(),
            show_commands_requests: ShowCommandsRequestStore::default(),
            scoreboard: Rc::new(RefCell::new(ScoreboardState::default())),
            scoreboard_presentations: Rc::new(RefCell::new(ScoreboardPresentationSink::default())),
        };
        engine.environment.refresh_runtime_fields();
        engine
    }

    /// The synchronized seed retained in `Game.Parameters.RandomSeed`.
    pub fn random_seed(&self) -> u64 {
        self.random_seed
    }

    /// Apply the C++ network client's control clock before synchronized
    /// gameplay starts (`C4Network2.cpp:1607-1608`).
    pub fn initialize_network_control_timing(&mut self, timing: NetworkControlTiming) {
        self.control_tick = timing.start_control_tick;
        self.control_rate = timing.control_rate;
    }

}

fn build_state_value(
    definition_id: &str,
    object_id: ObjectId,
    state: &ObjectState,
    library: &ActionLibrary,
) -> Value {
    let mut map = ValueMap::with_capacity(8);
    map.insert(
        "definition".into(),
        Value::String(definition_id.to_string().into()),
    );
    map.insert("id".into(), Value::Int(truncate_to_i32(object_id.as_u64())));
    map.insert("position".into(), state.position.to_value());
    map.insert("velocity".into(), state.velocity.to_value());
    map.insert("energy".into(), Value::Int(state.energy));
    map.insert("construction".into(), Value::Int(state.construction));
    map.insert(
        "direction".into(),
        Value::Int(state.direction.to_script_value()),
    );
    map.insert(
        "command_direction".into(),
        Value::Int(state.command_direction.to_script_value()),
    );
    map.insert("owner".into(), Value::Int(state.owner));
    map.insert("category".into(), Value::Int(state.category));
    map.insert("crew_member".into(), Value::Bool(state.crew_member));
    map.insert("status".into(), Value::Int(state.status.to_script_value()));
    match state.container {
        Some(container) => {
            map.insert(
                "container".into(),
                Value::Int(truncate_to_i32(container.as_u64())),
            );
        }
        None => {
            map.insert("container".into(), Value::Nil);
        }
    }
    let contents: Vec<_> = state
        .contents
        .iter()
        .map(|id| Value::Int(truncate_to_i32(id.as_u64())))
        .collect();
    map.insert("contents".into(), Value::Array(contents));
    let mut action = ValueMap::with_capacity(7);
    action.insert(
        "name".into(),
        Value::String(state.action.name.clone().into()),
    );
    action.insert("phase".into(), Value::Int(state.action.phase));
    action.insert("ticks".into(), Value::Int(state.action.ticks));
    action.insert("data".into(), Value::Int(state.action.data));
    match state.action.target {
        Some(target) => {
            action.insert(
                "target".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target".into(), Value::Nil);
        }
    }
    match state.action.target2 {
        Some(target) => {
            action.insert(
                "target2".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target2".into(), Value::Nil);
        }
    }
    if let Some(procedure) =
        library.procedure_name_for_entry(&state.action.name, state.action.act_map_index)
    {
        action.insert(
            "procedure".into(),
            Value::String(procedure.to_string().into()),
        );
    }
    map.insert("action".into(), Value::Proplist(action));
    let effects: Vec<_> = state
        .effects
        .iter()
        .map(|effect| {
            let mut props = ValueMap::with_capacity(6);
            props.insert("name".into(), Value::String(effect.name.clone().into()));
            props.insert("priority".into(), Value::Int(effect.priority));
            props.insert("interval".into(), Value::Int(effect.interval));
            props.insert("timer".into(), Value::Int(effect.timer));
            if let Some(target) = effect.command_target {
                props.insert("command_target".into(), Value::Int(target));
            }
            if let Some(id) = &effect.command_id {
                props.insert("command_target_id".into(), Value::String(id.clone().into()));
            }
            Value::Proplist(props)
        })
        .collect();
    map.insert("effects".into(), Value::Array(effects));
    Value::Proplist(map)
}

fn build_menu_selection_value(selection: &MenuCommandSelection) -> Value {
    let mut map = ValueMap::with_capacity(4);
    map.insert(
        "primary".into(),
        Value::Int(truncate_to_i32(selection.primary_id.as_u64())),
    );
    let instances: Vec<_> = selection
        .instances
        .iter()
        .map(|id| Value::Int(truncate_to_i32(id.as_u64())))
        .collect();
    map.insert("instances".into(), Value::Array(instances));
    map.insert(
        "definition".into(),
        Value::String(selection.definition_id.clone().into()),
    );
    map.insert(
        "label".into(),
        Value::String(selection.label.clone().into()),
    );
    Value::Proplist(map)
}

fn build_object_snapshot_value(snapshot: &ObjectSnapshot) -> Value {
    let mut map = ValueMap::with_capacity(11);
    map.insert(
        "definition".into(),
        Value::String(snapshot.definition_id.clone().into()),
    );
    map.insert(
        "id".into(),
        Value::Int(truncate_to_i32(snapshot.id.as_u64())),
    );
    map.insert("position".into(), snapshot.position.to_value());
    map.insert("velocity".into(), snapshot.velocity.to_value());
    map.insert("rotation".into(), Value::Int(snapshot.rotation));
    map.insert("energy".into(), Value::Int(snapshot.energy));
    map.insert("construction".into(), Value::Int(snapshot.construction));
    map.insert("damage".into(), Value::Int(snapshot.damage));
    map.insert(
        "direction".into(),
        Value::Int(snapshot.direction.to_script_value()),
    );
    map.insert(
        "command_direction".into(),
        Value::Int(snapshot.command_direction.to_script_value()),
    );
    map.insert("owner".into(), Value::Int(snapshot.owner));
    map.insert("category".into(), Value::Int(snapshot.category));
    map.insert("crew_member".into(), Value::Bool(snapshot.crew_member));
    map.insert(
        "status".into(),
        Value::Int(snapshot.status.to_script_value()),
    );
    match snapshot.container {
        Some(container) => {
            map.insert(
                "container".into(),
                Value::Int(truncate_to_i32(container.as_u64())),
            );
        }
        None => {
            map.insert("container".into(), Value::Nil);
        }
    }
    let contents: Vec<_> = snapshot
        .contents
        .iter()
        .map(|id| Value::Int(truncate_to_i32(id.as_u64())))
        .collect();
    map.insert("contents".into(), Value::Array(contents));
    let mut action = ValueMap::with_capacity(7);
    action.insert(
        "name".into(),
        Value::String(snapshot.action.name.clone().into()),
    );
    action.insert("phase".into(), Value::Int(snapshot.action.phase));
    action.insert("ticks".into(), Value::Int(snapshot.action.ticks));
    action.insert("data".into(), Value::Int(snapshot.action.data));
    match snapshot.action.target {
        Some(target) => {
            action.insert(
                "target".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target".into(), Value::Nil);
        }
    }
    match snapshot.action.target2 {
        Some(target) => {
            action.insert(
                "target2".into(),
                Value::Int(truncate_to_i32(target.as_u64())),
            );
        }
        None => {
            action.insert("target2".into(), Value::Nil);
        }
    }
    if let Some(procedure) = &snapshot.action_procedure {
        action.insert("procedure".into(), Value::String(procedure.clone().into()));
    }
    map.insert("action".into(), Value::Proplist(action));
    let effects: Vec<_> = snapshot.effects.iter().map(build_effect_value).collect();
    map.insert("effects".into(), Value::Array(effects));
    Value::Proplist(map)
}

/// The live `ObjectState` a snapshot entry describes — the full scope
/// nested calls require (mirrors the restore_state mapping; container and
/// contents come straight from the snapshot since no two-phase
/// denumeration is needed for a read-mostly scope seed).
fn object_state_from_snapshot(snapshot: &ObjectSnapshot) -> ObjectState {
    let component_order =
        normalized_component_order(&snapshot.components, snapshot.component_order.clone(), &[]);
    ObjectState {
        custom_name: snapshot.custom_name.clone(),
        script_fixed_position: None,
        script_fixed_velocity: None,
        script_rotation_velocity: snapshot.rotation_velocity,
        script_fixed_rotation: snapshot.fixed_rotation,
        position: snapshot.position,
        velocity: snapshot.velocity,
        rotation: snapshot.rotation,
        shape_attach: ShapeAttachRecord::default(),
        t_attach: 0,
        no_collect_delay: 0,
        base: snapshot.base,
        energy: snapshot.energy,
        need_energy: snapshot.need_energy,
        construction: snapshot.construction,
        damage: snapshot.damage,
        magic_energy: snapshot.magic_energy,
        magic_capacity: snapshot.magic_capacity,
        action: snapshot.action.clone(),
        direction: snapshot.direction,
        command_direction: snapshot.command_direction,
        effects: snapshot.effects.clone(),
        vertices: snapshot.vertices.clone(),
        shape_vertices: ShapeVertexBuffer::from_active(&snapshot.vertices),
        contact_density: snapshot.contact_density,
        container: snapshot.container,
        layer: snapshot.layer,
        visibility: snapshot.visibility,
        blit_mode: snapshot.blit_mode,
        picture_rect: snapshot.picture_rect,
        contents: snapshot.contents.clone(),
        contents_link_generation: 0,
        components: snapshot.components.clone(),
        component_order,
        status: snapshot.status,
        owner: snapshot.owner,
        controller: snapshot.controller,
        category: snapshot.category,
        crew_member: snapshot.crew_member,
        plr_view_range: snapshot.plr_view_range,
        selected: snapshot.selected,
        crew_disabled: false,
        alive: snapshot.alive,
        base_graphics: snapshot.base_graphics.clone(),
        graphics_overlays: snapshot.graphics_overlays.clone(),
        draw_transform: snapshot.draw_transform,
        local_vars: snapshot.local_vars.clone(),
        in_liquid: snapshot.in_liquid,
        mobile: snapshot.mobile,
        solid_mask_override: None,
        timer: snapshot.timer,
        own_mass: snapshot.own_mass,
        on_fire: snapshot.on_fire,
        fire_phase: snapshot.fire_phase,
        fire_caused_by: snapshot.fire_caused_by,
        info_physical: snapshot.info_physical,
        temporary_physical: snapshot.temporary_physical,
        physical_changes: snapshot.physical_changes.clone(),
        breath: snapshot.breath,
        entrance_status: false,
        menu: None,
        color: snapshot.color,
        color_modulation: snapshot.color_modulation,
        shape_override: snapshot.current_shape,
        ocf: OCF_NORMAL,
    }
}

fn host_world_context_from_snapshot(snapshot: &SimulationSnapshot) -> HostWorldContext {
    let next_object_id = snapshot
        .objects
        .iter()
        .map(|object| object.id.as_u64())
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let definition_metadata: HashMap<DefinitionId, DefinitionMetadata> = snapshot
        .definition_categories
        .iter()
        .map(|(id, category)| {
            (
                id.clone(),
                DefinitionMetadata {
                    name: String::new(),
                    portrait_names: Vec::new(),
                    category: *category,
                    border_bound: 0,
                    contact_function_calls: false,
                    blit_mode: 0,
                    ocf_base: OCF_NORMAL,
                    crew_member: false,
                    crew_member_value: 0,
                    silent_commands: false,
                    vehicle_control: 0,
                    action_library: ActionLibrary::default().into(),
                    control_transfer_callback: None,
                    action_graphics: HashMap::new(),
                    value: 0,
                    allow_picture_stack: 0,
                    mass: 0,
                    no_component_mass: false,
                    constructable: false,
                    shape: None,
                    placement: 0,
                    growth: 0,
                    construction_offset: 0,
                    basement: 0,
                    physical: clonk_resources::PhysicalInfo::default(),
                    components: Vec::new(),
                    collection_limit: 0,
                    grab_put_get: 0,
                    line_connect: 0,
                    clonk_name_newlines: None,
                    stretch_growth: false,
                    rotateable: 0,
                    line: snapshot
                        .definition_lines
                        .get(id)
                        .map(|metadata| metadata.line)
                        .unwrap_or(0),
                    vertices: Vec::new(),
                    contact_density: None,
                    fire: compat::DefinitionFireMetadata::default(),
                },
            )
        })
        .collect();
    let players: HashMap<i32, PlayerState> = snapshot
        .players
        .iter()
        .map(|state| (state.id, state.clone()))
        .collect();
    let crew_selection = snapshot.crew_selection.clone();
    let sky_adjustment = snapshot
        .sky
        .as_ref()
        .map(|frame| SkyAdjustment::from_settings(&frame.settings))
        .unwrap_or_default();
    let sky_fade = snapshot.sky.as_ref().map_or_else(
        || {
            let settings = SkySettings::default();
            [settings.fade_top, settings.fade_bottom]
        },
        |frame| [frame.settings.fade_top, frame.settings.fade_bottom],
    );
    HostWorldContext::with_landscape(
        snapshot.objects.iter().map(|object| {
            HostWorldObject::with_category(
                object.id,
                object.definition_id.clone(),
                object.status,
                object.action.name.clone(),
                object.action.target,
                object.action.target2,
                object.action_procedure.clone(),
                object.owner,
                object.category,
                object.energy,
                object.construction,
                object.damage,
                object.position,
                object.velocity,
                object.rotation,
                object.vertices.clone(),
                object.action.data,
                object.action.time,
                object.action.phase,
                object.container,
                object.draw_transform,
            )
            .with_action_index(object.action.act_map_index)
            .with_fixed_motion(
                object
                    .fixed_position
                    .unwrap_or_else(|| FixedVec2::from_ints(object.position.x, object.position.y)),
                object
                    .fixed_velocity
                    .unwrap_or_else(|| FixedVec2::from_ints(object.velocity.x, object.velocity.y)),
            )
            .with_fixed_rotation(
                object
                    .fixed_rotation
                    .unwrap_or_else(|| itofix(object.rotation)),
            )
            .with_rotation_velocity(object.rotation_velocity.unwrap_or_default())
            .with_own_vertices(object.own_vertices.is_some())
            .with_contact_density(object.contact_density)
            .with_direction(object.direction.to_script_value())
            .with_contents(object.contents.clone())
            .with_need_energy(object.need_energy)
            .with_commands(object.command_stack.command_views())
            .with_command_stack(object.command_stack.clone())
            .with_ocf(object.ocf)
            // Nested calls (obj->Method, foreign RemoveObject) need a full
            // scope for WORLD objects too — GoldRush re-runs the placed
            // cannon's Initialize from InitializePlayer
            // (Goldrush.c4s/Script.c:262 → pObj->~Initialize()).
            .with_full_state(Rc::new(object_state_from_snapshot(object)))
            .with_last_energy_loss_cause(object.last_energy_loss_cause)
        }),
        snapshot.landscape.clone(),
        definition_metadata,
        snapshot.transfer_zones.clone(),
        players,
        crew_selection,
        next_object_id,
        false,
    )
    .with_player_order(snapshot.players.iter().map(|state| state.id))
    .with_sky_adjustment(sky_adjustment)
    .with_sky_fade(sky_fade[0], sky_fade[1])
    .with_scoreboard(Rc::new(RefCell::new(snapshot.hud.scoreboard.clone())))
    .with_local_players(snapshot.hud.local_players.iter().copied())
    .with_league_progress_data(
        Rc::new(legacy_c_string_bytes(snapshot.league_name.clone())),
        Rc::new(
            snapshot
                .player_info_league_progress_data
                .iter()
                .filter(|(id, _)| **id != 0)
                .map(|(&id, data)| (id, data.clone().map(legacy_c_string_bytes)))
                .collect(),
        ),
    )
    .with_league_scores(Rc::new(
        snapshot
            .player_info_league_scores
            .iter()
            .filter(|(id, score)| **id != 0 && **score != 0)
            .map(|(&id, &score)| (id, score))
            .collect(),
    ))
}

fn build_scenario_state_value(snapshot: &SimulationSnapshot) -> Value {
    let mut map = ValueMap::with_capacity(5);
    let frame_value = if snapshot.frame > i32::MAX as u64 {
        i32::MAX
    } else {
        snapshot.frame as i32
    };
    map.insert("frame".into(), Value::Int(frame_value));
    map.insert("game_over".into(), Value::Bool(snapshot.game_over));
    match snapshot.physics {
        Some(physics) => {
            map.insert("physics".into(), Value::Proplist(physics_to_map(physics)));
        }
        None => {
            map.insert("physics".into(), Value::Nil);
        }
    }
    map.insert(
        "environment".into(),
        Value::Proplist(environment_frame_to_map(&snapshot.environment)),
    );
    let objects: Vec<_> = snapshot
        .objects
        .iter()
        .map(build_object_snapshot_value)
        .collect();
    map.insert("objects".into(), Value::Array(objects));
    let global_effects: Vec<_> = snapshot
        .global_effects
        .iter()
        .map(build_effect_value)
        .collect();
    map.insert("global_effects".into(), Value::Array(global_effects));
    Value::Proplist(map)
}

fn physics_to_map(settings: PhysicsSettings) -> ValueMap {
    let mut map = ValueMap::with_capacity(4);
    map.insert("gravity".into(), Value::Int(settings.gravity));
    map.insert("max_fall_speed".into(), Value::Int(settings.max_fall_speed));
    map.insert("max_rise_speed".into(), Value::Int(settings.max_rise_speed));
    map.insert(
        "max_horizontal_speed".into(),
        Value::Int(settings.max_horizontal_speed),
    );
    map
}

fn environment_frame_to_map(frame: &EnvironmentFrame) -> ValueMap {
    let mut map = ValueMap::with_capacity(12);
    let settings = frame.settings;
    map.insert("wind".into(), Value::Int(settings.wind));
    map.insert("wind_variation".into(), Value::Int(settings.wind_variation));
    let wind_period = settings.wind_period.min(i32::MAX as u32) as i32;
    map.insert("wind_period".into(), Value::Int(wind_period));
    map.insert("temperature".into(), Value::Int(settings.temperature));
    map.insert("climate".into(), Value::Int(settings.climate));
    map.insert(
        "temperature_variation".into(),
        Value::Int(settings.temperature_variation),
    );
    let temperature_period = settings.temperature_period.min(i32::MAX as u32) as i32;
    map.insert("temperature_period".into(), Value::Int(temperature_period));
    let temperature_phase = settings.temperature_phase.min(i32::MAX as u32) as i32;
    map.insert("temperature_phase".into(), Value::Int(temperature_phase));
    map.insert(
        "time_of_day".into(),
        Value::Int(i32::from(settings.time_of_day)),
    );
    map.insert(
        "time_speed".into(),
        Value::Int(i32::from(settings.time_speed)),
    );
    map.insert("precipitation".into(), Value::Int(settings.precipitation));
    map.insert("current_wind".into(), Value::Int(frame.wind_force));
    map.insert(
        "ambient_temperature".into(),
        Value::Int(frame.ambient_temperature),
    );
    map.insert(
        "sky_color".into(),
        frame.sky_color.map(rgb_to_value).unwrap_or(Value::Nil),
    );
    map
}

fn rgb_to_value(color: RgbColor) -> Value {
    Value::Array(vec![
        Value::Int(i32::from(color.r)),
        Value::Int(i32::from(color.g)),
        Value::Int(i32::from(color.b)),
    ])
}

fn build_effect_value(effect: &EffectState) -> Value {
    // C++ passes iNumber (an int handle) as the Fx* callback's second
    // argument — scripts feed it back into EffectVar/RemoveEffect
    // (FxIntScheduleCallTimer, planet Helpers.c). The old proplist here
    // broke every script that used the handle.
    Value::Int(effect.number)
}

fn merge_environment_delta(target: &mut EnvironmentDelta, source: &EnvironmentDelta) {
    if let Some(wind) = source.wind {
        target.wind = Some(wind);
    }
    if let Some(temperature) = source.temperature {
        target.temperature = Some(temperature);
        target.season_gamma_handled = source.season_gamma_handled;
    }
    if let Some(climate) = source.climate {
        target.climate = Some(climate);
        target.season_gamma_handled = source.season_gamma_handled;
    }
    if let Some(season) = source.season {
        target.season = Some(season);
        target.season_gamma_handled = source.season_gamma_handled;
    }
}

fn merge_physics_delta(target: &mut PhysicsDelta, source: &PhysicsDelta) {
    if let Some(gravity) = source.gravity {
        target.gravity = Some(gravity);
    }
}

fn parse_scenario_command(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<ScenarioBatch, EngineError> {
    match value {
        Value::Nil => Ok(ScenarioBatch::default()),
        // C++ parity: the engine discards scenario-callback return values
        // (Game.Script.Call/GRBroadcast run as bare statements); real
        // scenario scripts routinely `return(1)` from Initialize. The
        // command proplist stays an additive Rust-fixture convenience —
        // any other type is ignored, never an error. (Mirrors
        // parse_command below.)
        Value::Proplist(map) => {
            let mut batch = ScenarioBatch::default();
            for (key, value) in map.into_iter() {
                let Value::String(key) = key else {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{key}`"),
                    });
                };
                match key.as_ref() {
                    "spawn" => {
                        batch
                            .spawns
                            .extend(value_to_spawns(definition, function, value)?);
                    }
                    "global_effects" => {
                        batch
                            .global_effects
                            .extend(value_to_effect_commands(definition, function, value)?);
                    }
                    "physics" => {
                        let delta = value_to_physics_delta(definition, function, value)?;
                        if !delta.is_empty() {
                            if let Some(existing) = &mut batch.physics {
                                merge_physics_delta(existing, &delta);
                            } else {
                                batch.physics = Some(delta);
                            }
                        }
                    }
                    "landscape" => {
                        batch
                            .landscape
                            .extend(value_to_landscape_commands(definition, function, value)?);
                    }
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: format!("unexpected key `{other}`"),
                        });
                    }
                }
            }
            Ok(batch)
        }
        _ => Ok(ScenarioBatch::default()),
    }
}

/// Folded outcome of one global effect-event batch
/// ([`Engine::run_effect_events_for_global`]) — the same channels the
/// object timer batch returns, minus the carrier-object ones.
struct GlobalEffectRunOutcome {
    particles: Vec<ParticleCommand>,
    physics_delta: PhysicsDelta,
    audio_events: Vec<AudioCommand>,
    messages: Vec<MessageCommand>,
    player_commands: Vec<PlayerCommand>,
    object_order_commands: Vec<ObjectOrderCommand>,
    next_mission_commands: Vec<NextMissionCommand>,
    landscape_ops: Vec<LandscapeOperation>,
    solid_mask_operations: Vec<HostSolidMaskOperation>,
    host_raster_preview: Option<compat::HostRasterPreview>,
    transfer_zones: Vec<TransferZoneCommand>,
    spawns: Vec<SpawnConfig>,
    other_objects: Vec<compat::NestedObjectOutcome>,
    next_object_id: u64,
    game_over: bool,
    script_go: Option<bool>,
    script_counter: Option<i32>,
    audio_state: AudioRegistry,
    rng: LcgRng,
}

/// One exact `C4Effect::GetCallbackScript` / `GetFuncRecursive` result.
/// Keeping source selection and function resolution together prevents an
/// arbitrary affected-object or fallback definition from shadowing the
/// engine-global table (C4Effect.cpp:31-56,439-456).
struct EffectScriptCallback {
    script: Arc<ScriptEngine>,
    resolution: clonk_script::ScriptFunctionResolution,
    command_object: Option<ObjectId>,
    definition_context: Option<DefinitionId>,
    /// The resolved SFunc is owned by Game.ScriptEngine. Invoke the pinned
    /// body on its exact LinkedTo host; a command object, when present,
    /// still supplies `this`, live cells, and `cthr->Def`.
    engine_global_entry: bool,
}

/// Fold the VM-final local storage of a command target that is not the
/// affected carrier. C++ executes directly on the one live `C4Object`; the
/// Rust callback context is copy-in/copy-out, so the target travels through
/// the existing foreign-object outcome channel.
fn append_effect_command_target_locals(
    outcome: &mut EffectContextOutcome,
    command_target: ObjectId,
    local_vars: HashMap<String, Value>,
) {
    outcome.other_objects.push(compat::NestedObjectOutcome {
        object_id: command_target,
        effects: Vec::new(),
        update: Some(ObjectUpdate {
            local_vars: Some(local_vars),
            ..ObjectUpdate::default()
        }),
        commands: Vec::new(),
        command_operations: Vec::new(),
        destroy: false,
        assign_death: None,
        contents_orders: Vec::new(),
    });
}

/// C4Effect invokes script callbacks through the fail-safe C4Aul `Exec`
/// path: an ordinary runtime error aborts the callback and yields C4VNull,
/// but mutations performed before the error stay on the live objects
/// (C4AulExec.cpp:1318-1342). Rust callbacks run against copied host state,
/// so turn that error into a nil result while retaining the command target's
/// live local cells for the normal outcome copy-out. Fatal runtime-boundary
/// errors remain errors and must not be downgraded.
fn recover_effect_callback_error(
    result: Result<Option<(Value, HashMap<String, Value>)>, ScriptError>,
    context_cells: &clonk_script::LocalCells,
    definition: String,
) -> Result<Option<(Value, HashMap<String, Value>)>, EngineError> {
    match result {
        Ok(result) => Ok(result),
        Err(source) => {
            match script_execution_error(definition, "EffectCallback".to_string(), source, None) {
                EngineError::Script {
                    definition,
                    function,
                    source,
                    recovery: _,
                } => {
                    tracing::warn!(
                        %definition,
                        function,
                        error = %source,
                        "script error in effect callback; continuing like the C++ fail-safe exec"
                    );
                    log_runtime_call_frames(&definition, source.call_frames());
                    Ok(Some((Value::Nil, context_cells.snapshot())))
                }
                fatal => Err(fatal),
            }
        }
    }
}

fn resolve_effect_script_callback(
    effect: &EffectState,
    callback_name: &str,
    world: &HostWorldContext,
) -> Option<EffectScriptCallback> {
    if let Some(command_object) = effect
        .command_target
        .map(|target| ObjectId::new(target as u64))
        .filter(|target| world.get(*target).is_some())
    {
        let definition = DefinitionId::from(world.get(command_object)?.definition_id());
        let source_script = Arc::clone(world.definition_script(&definition)?);
        let resolution = source_script.resolve_function(callback_name, true)?;
        let is_global = resolution.scope == clonk_script::ScriptFunctionScope::Global;
        let script = if is_global {
            world
                .script_for_host_identity(resolution.host_identity)
                .map(|(_, _, script)| script)?
        } else {
            source_script
        };
        return Some(EffectScriptCallback {
            script,
            resolution,
            command_object: Some(command_object),
            // C4AulExec derives Def from pCommandTarget->Def whenever Obj is
            // non-null, even when the selected SFunc belongs to the engine.
            definition_context: Some(definition),
            engine_global_entry: is_global,
        });
    }

    if let Some(definition) = effect
        .command_id
        .as_ref()
        .map(DefinitionId::from)
        .filter(|definition| world.definition_script(definition).is_some())
    {
        let source_script = Arc::clone(world.definition_script(&definition)?);
        let resolution = source_script.resolve_function(callback_name, true)?;
        let is_global = resolution.scope == clonk_script::ScriptFunctionScope::Global;
        let script = if is_global {
            world
                .script_for_host_identity(resolution.host_identity)
                .map(|(_, _, script)| script)?
        } else {
            source_script
        };
        return Some(EffectScriptCallback {
            script,
            resolution,
            command_object: None,
            definition_context: (!is_global).then_some(definition),
            engine_global_entry: is_global,
        });
    }

    let (script, resolution) = world.resolve_engine_global_script(callback_name)?;
    Some(EffectScriptCallback {
        script,
        resolution,
        command_object: None,
        definition_context: None,
        engine_global_entry: true,
    })
}

/// Dispatch for effects on `Game.pGlobalEffects`. It intentionally has no
/// [`Definition`] receiver: C++ can run System/scenario global callbacks in
/// a game with no loaded definitions, and an arbitrary fallback definition
/// must never contribute local Fx functions.
#[allow(clippy::too_many_arguments)]
fn dispatch_global_effect_callback(
    effect: &EffectState,
    event: &'static str,
    function_label: &'static str,
    mut extras: Vec<Value>,
    rng: LcgRng,
    global_effects: &[EffectState],
    physics: PhysicsSettings,
    environment: EnvironmentSettings,
    frame: u64,
    world: HostWorldContext,
    game_over_triggered: bool,
    audio: AudioRegistry,
) -> Result<(EffectContextOutcome, AudioRegistry, LcgRng, Option<Value>), EngineError> {
    let next_object_id = world.next_object_id();
    let callback_name = format!("Fx{}{}", effect.name, event);
    let Some(callback) = resolve_effect_script_callback(effect, &callback_name, &world) else {
        return Ok((
            EffectContextOutcome::empty(next_object_id, audio.clone()),
            audio,
            rng,
            None,
        ));
    };

    let mut args = Vec::with_capacity(2 + extras.len());
    args.push(Value::Nil);
    args.push(build_effect_value(effect));
    args.append(&mut extras);

    let context_object = callback.command_object;
    let context_this = context_object
        .map(compat::object_reference_value)
        .unwrap_or(Value::Nil);
    let context_locals = context_object
        .and_then(|object_id| world.get(object_id))
        .and_then(|object| object.full_state().map(|state| state.local_vars.clone()))
        .unwrap_or_default();
    let context_cells = clonk_script::LocalCells::from_local_vars(&context_locals);

    let physics_guard = enter_physics_context(physics);
    let env_guard = enter_environment_context(environment, frame);
    let guard = enter_random_context(rng);
    let audio_guard = enter_audio_context(audio);
    let (result, mut commands) = compat::with_effect_context_with_state_and_definition(
        None,
        callback.definition_context.clone(),
        context_object,
        global_effects,
        world,
        next_object_id,
        game_over_triggered,
        || {
            if let Some(session_id) = context_object {
                compat::register_session_local_cells(session_id, context_cells.clone());
            }
            if callback.engine_global_entry {
                if context_object.is_some() {
                    return callback
                        .script
                        .call_pinned_with_cells_and_this(
                            &callback.resolution.function,
                            true,
                            &args,
                            &context_cells,
                            context_this,
                        )
                        .map(|value| Some((value, context_cells.snapshot())));
                }
                return callback
                    .script
                    .call_pinned_with_ref_args(&callback.resolution.function, true, &args)
                    .map(|(value, _)| Some((value, HashMap::new())));
            }
            callback.script.call_effect_callback_in_context_with_cells(
                &effect.name,
                event,
                &args,
                &context_cells,
                context_this,
            )
        },
    );
    let rng = guard.finish();
    let physics_delta = physics_guard.finish();
    let environment_delta = env_guard.finish();
    let audio_state = audio_guard.finish();

    let callback_result = recover_effect_callback_error(
        result,
        &context_cells,
        format!("Game.ScriptEngine::{}::{}", effect.name, function_label),
    )?;
    if !environment_delta.is_empty() {
        commands.environment = Some(environment_delta);
    }
    if !physics_delta.is_empty() {
        commands.physics = Some(physics_delta);
    }
    let callback_result = callback_result.map(|(value, updated_locals)| {
        if let Some(context_object) = context_object {
            append_effect_command_target_locals(&mut commands, context_object, updated_locals);
        }
        value
    });
    Ok((commands, audio_state, rng, callback_result))
}

/// Selects the [`Definition`] receiver that supplies carrier metadata and
/// callback-outcome conversion. The exact C4Effect script source is resolved
/// separately by [`resolve_effect_script_callback`], so this fallback cannot
/// shadow `Game.ScriptEngine` with affected-object locals.
fn resolve_effect_dispatch_definition<'a>(
    effect: &EffectState,
    world: &HostWorldContext,
    definitions: &'a HashMap<DefinitionId, Definition>,
    live_host: Option<(ObjectId, &str)>,
    fallback: &'a Definition,
) -> &'a Definition {
    effect
        .command_target
        .and_then(|target| {
            let target_id = ObjectId::new(target as u64);
            live_host
                .filter(|(host_id, _)| *host_id == target_id)
                .map(|(_, definition_id)| definition_id.to_string())
                .or_else(|| {
                    world
                        .get(target_id)
                        .map(|target| target.definition_id().to_string())
                })
        })
        .or_else(|| effect.command_id.clone())
        .and_then(|def_id| definitions.get(&def_id))
        .unwrap_or(fallback)
}

fn effect_stop_reason_value(reason: EffectStopReason) -> Value {
    match reason {
        // C4Effect::Kill omits the third parameter. An explicit nil is the
        // same ten-slot C4AulParSet value and, unlike integer zero, stays nil
        // for untyped callbacks after parameter conversion; a strict-3 `int`
        // parameter converts the missing slot to C4FxCall_Normal (0).
        EffectStopReason::Removed | EffectStopReason::Replaced => Value::Nil,
        // C4Effect::ClearAll uses C4FxCall_RemoveClear.
        EffectStopReason::Cleared | EffectStopReason::Destroyed => Value::Int(3),
        // C4FxCall_RemoveDeath and C4FxCall_Temp (C4Effects.h:46-50).
        EffectStopReason::Death => Value::Int(4),
        EffectStopReason::Temp => Value::Int(1),
    }
}

/// Locate the suffix after a stable live-effect cursor. Identity wins while
/// the node remains linked; priority/list order recovers the same suffix when
/// a callback unlinks the current node.
fn effect_frame_cursor_next_index(
    effects: &[EffectState],
    cursor: Option<EffectFrameCursor>,
) -> usize {
    match cursor {
        None => 0,
        Some(cursor) => effects
            .iter()
            .position(|effect| {
                effect.number == cursor.number
                    && (effect.priority == 0 || effect.priority.unsigned_abs() == cursor.priority)
            })
            .map(|index| index + 1)
            .unwrap_or_else(|| {
                // Most real removals leave the dead node linked. This
                // structural fallback covers older command producers that
                // still unlink immediately: higher priorities and older
                // equal-priority peers form the suffix.
                effects
                    .iter()
                    .position(|effect| {
                        let priority = effect.priority.unsigned_abs();
                        priority > cursor.priority
                            || (priority == cursor.priority && effect.number < cursor.number)
                    })
                    .unwrap_or(effects.len())
            }),
    }
}

/// Advance one node of C4Effect::Execute's live list walk. Dead nodes are
/// unlinked only when the cursor reaches them; a callback can therefore
/// mutate the suffix before its nodes advance, and an effect inserted after
/// the current node can still execute in this frame.
fn advance_effect_frame_cursor(
    effects: &mut Vec<EffectState>,
    cursor: Option<EffectFrameCursor>,
) -> Option<(EffectFrameCursor, Option<EffectEvent>)> {
    let index = effect_frame_cursor_next_index(effects, cursor);

    loop {
        let effect = effects.get(index)?;
        if effect.priority == 0 {
            effects.remove(index);
            continue;
        }

        let effect = &mut effects[index];
        let cursor = EffectFrameCursor {
            number: effect.number,
            priority: effect.priority.unsigned_abs(),
        };
        let event = effect
            .advance_tick()
            .then(|| EffectEvent::timer(effect.clone()));
        return Some((cursor, event));
    }
}

/// Active effects AFTER `anchor` in the C++ effect list — the list orders
/// ascending by |iPriority| with new-before-equal insertion
/// (C4Effect.cpp:80-94), so an upper effect has a higher priority magnitude
/// or is an equal-magnitude peer inserted earlier (lower number).
/// Priority-1 effects never take temp callbacks (C4Effect.cpp:489,505).
fn upper_effects_of(effects: &[EffectState], anchor: &EffectState) -> Vec<EffectState> {
    effects
        .iter()
        .filter(|existing| {
            existing.number != anchor.number && existing.priority > 0 && existing.priority != 1
        })
        .filter(|existing| {
            let (upper, base) = (
                existing.priority.unsigned_abs(),
                anchor.priority.unsigned_abs(),
            );
            upper > base || (upper == base && existing.number < anchor.number)
        })
        .cloned()
        .collect()
}

/// Removes THE effect by its C++ identity, iNumber (names may repeat).
fn remove_effect_from_stack(stack: &mut Vec<EffectState>, number: i32) -> Option<EffectState> {
    stack
        .iter()
        .position(|existing| existing.number == number)
        .map(|index| stack.remove(index))
}

fn apply_effect_commands_to_stack(target: &mut Vec<EffectState>, commands: &[EffectCommand]) {
    for command in commands {
        match command {
            EffectCommand::Add { effect, .. } => insert_effect_into_stack(target, effect.clone()),
            EffectCommand::Update(effect) => {
                if let Some(existing) = target
                    .iter_mut()
                    .find(|existing| existing.number == effect.number)
                {
                    *existing = effect.clone();
                }
            }
            EffectCommand::Remove { name, .. } => {
                if let Some(effect) = target
                    .iter_mut()
                    .find(|effect| &effect.name == name && effect.priority != 0)
                {
                    effect.priority = 0;
                }
            }
            EffectCommand::RemoveNumber { number, .. } => {
                if let Some(effect) = target
                    .iter_mut()
                    .find(|effect| effect.number == *number && effect.priority != 0)
                {
                    effect.priority = 0;
                }
            }
            EffectCommand::UnlinkNumber { number } => {
                remove_effect_from_stack(target, *number);
            }
            EffectCommand::Clear => target.clear(),
        }
    }
}

fn insert_effect_into_stack(stack: &mut Vec<EffectState>, mut effect: EffectState) {
    if effect.timer < 0 {
        effect.timer = 0;
    }

    // C4Effect::New: same-name effects coexist; numbers are per-list
    // monotonic (C4Effect.cpp:76-78). A carried number matching an
    // existing effect is an in-place update.
    if effect.number > 0 {
        if let Some(existing) = stack
            .iter_mut()
            .find(|existing| existing.number == effect.number)
        {
            *existing = effect;
            return;
        }
    }
    if effect.number == 0 {
        effect.number = stack
            .iter()
            .map(|existing| existing.number)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
    }

    // Preserve C++'s newest-first order for carried equal-priority nodes;
    // fresh effects have max+1 and therefore still insert before all equals.
    let priority = effect.priority.unsigned_abs();
    let mut insert_pos = 0;
    while insert_pos < stack.len() {
        let existing = &stack[insert_pos];
        let existing_priority = existing.priority.unsigned_abs();
        if existing_priority > priority
            || (existing_priority == priority && existing.number < effect.number)
        {
            break;
        }
        insert_pos += 1;
    }

    stack.insert(insert_pos, effect);
}

fn truncate_to_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn parse_command(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<CommandBatch, EngineError> {
    match value {
        Value::Nil => Ok(CommandBatch::default()),
        Value::Proplist(map) => parse_command_from_proplist(definition, function, map),
        // C++ parity: lifecycle callbacks (Initialize/Step) have their return
        // value DISCARDED by the engine (e.g. C4Object.cpp:1483 calls
        // `Call(PSF_Initialize)` as a bare statement). Real definitions routinely
        // return an int from Initialize. The command-delta proplist is an additive
        // Rust convenience; any other return type is simply ignored, never an error.
        _ => Ok(CommandBatch::default()),
    }
}

fn parse_command_from_proplist(
    definition: &str,
    function: &str,
    map: ValueMap,
) -> Result<CommandBatch, EngineError> {
    let mut batch = CommandBatch::default();
    for (key, value) in map.into_iter() {
        let Value::String(key) = key else {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!("unexpected key `{key}`"),
            });
        };
        match key.as_ref() {
            "position" => {
                batch.delta.position = Some(value_to_vector(definition, function, value)?);
            }
            "velocity" => {
                batch.delta.velocity = Some(value_to_vector(definition, function, value)?);
            }
            "energy" => {
                batch.delta.energy = Some(value_to_int(definition, function, value)?);
            }
            "direction" => {
                batch.delta.direction = Some(value_to_direction(definition, function, value)?);
            }
            "command_direction" => {
                batch.delta.command_direction =
                    Some(value_to_command_direction(definition, function, value)?);
            }
            "owner" => {
                batch.delta.owner = Some(value_to_int(definition, function, value)?);
            }
            "action" => {
                let update = value_to_action(definition, function, value)?;
                if let Some(update) = update {
                    ensure_action_delta(&mut batch.delta).merge(update);
                }
            }
            "action_phase" => {
                let phase = value_to_int(definition, function, value)?;
                ensure_action_delta(&mut batch.delta).set_phase(phase);
            }
            "destroy" => {
                batch.destroy = value.as_bool();
            }
            "spawn" => {
                batch
                    .spawns
                    .extend(value_to_spawns(definition, function, value)?);
            }
            "commands" => {
                batch
                    .commands
                    .extend(value_to_commands(definition, function, value)?);
            }
            "effects" => {
                batch
                    .effects
                    .extend(value_to_effect_commands(definition, function, value)?);
            }
            "global_effects" => {
                batch
                    .global_effects
                    .extend(value_to_effect_commands(definition, function, value)?);
            }
            "physics" => {
                let delta = value_to_physics_delta(definition, function, value)?;
                if !delta.is_empty() {
                    if let Some(existing) = &mut batch.physics {
                        merge_physics_delta(existing, &delta);
                    } else {
                        batch.physics = Some(delta);
                    }
                }
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("unexpected key `{other}`"),
                });
            }
        }
    }
    Ok(batch)
}

fn ensure_action_delta(delta: &mut ObjectDelta) -> &mut ActionUpdate {
    delta.action.get_or_insert_with(ActionUpdate::default)
}

fn ensure_action_update(update: &mut ObjectUpdate) -> &mut ActionUpdate {
    update.action.get_or_insert_with(ActionUpdate::default)
}

fn value_to_action(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Option<ActionUpdate>, EngineError> {
    match value {
        Value::Nil => Ok(None),
        Value::String(name) => Ok(Some(ActionUpdate::default().with_name(name))),
        Value::Proplist(map) => parse_action_update(definition, function, map).map(Some),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function: function.to_string(),
            detail: format!(
                "expected string, proplist, or nil for action, got {}",
                other.type_name()
            ),
        }),
    }
}

fn parse_action_update(
    definition: &str,
    function: &str,
    map: ValueMap,
) -> Result<ActionUpdate, EngineError> {
    let mut update = ActionUpdate::default();
    for (key, value) in map.into_iter() {
        let Value::String(key) = key else {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!("unexpected key `{key}` in action proplist"),
            });
        };
        match key.as_ref() {
            "name" => match value {
                Value::String(name) => update.set_name(name),
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!(
                            "expected string for action.name, got {}",
                            other.type_name()
                        ),
                    });
                }
            },
            "phase" => {
                let phase = value_to_int(definition, function, value)?;
                update.set_phase(phase);
            }
            "ticks" => {
                let ticks = value_to_int(definition, function, value)?;
                update.set_ticks(ticks);
            }
            "data" => {
                let data = value_to_int(definition, function, value)?;
                update.set_data(data);
            }
            "target" => {
                let target = value_to_object_reference(definition, function, "target", value)?;
                update.set_target(target);
            }
            "target2" => {
                let target = value_to_object_reference(definition, function, "target2", value)?;
                update.set_target2(target);
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("unexpected key `{other}` in action proplist"),
                });
            }
        }
    }
    Ok(update)
}

fn value_to_vector(definition: &str, function: &str, value: Value) -> Result<Vector2, EngineError> {
    match value {
        Value::Array(values) if values.len() == 2 => {
            let x = match &values[0] {
                Value::Int(v) => *v,
                // Legacy zero literals arrive as nil; this typed Rust
                // command-delta field consumes them through C4Value::getInt.
                Value::Nil => 0,
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("expected int for x component, got {}", other.type_name()),
                    });
                }
            };
            let y = match &values[1] {
                Value::Int(v) => *v,
                Value::Nil => 0,
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("expected int for y component, got {}", other.type_name()),
                    });
                }
            };
            Ok(Vector2::new(x, y))
        }
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function: function.to_string(),
            detail: format!("expected array of two ints, got {}", other.type_name()),
        }),
    }
}

fn value_to_physics_delta(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<PhysicsDelta, EngineError> {
    match value {
        Value::Nil => Ok(PhysicsDelta::default()),
        Value::Proplist(map) => {
            let mut delta = PhysicsDelta::default();
            for (key, entry) in map.into_iter() {
                let Value::String(key) = key else {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected physics key `{key}`"),
                    });
                };
                match key.as_ref() {
                    "gravity" => match entry {
                        Value::Int(val) => delta.gravity = Some(val),
                        Value::Nil => delta.gravity = Some(0),
                        other => {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function: function.to_string(),
                                detail: format!(
                                    "physics.gravity expects int or nil, got {}",
                                    other.type_name()
                                ),
                            });
                        }
                    },
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: format!("unexpected physics key `{other}`"),
                        });
                    }
                }
            }
            Ok(delta)
        }
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function: function.to_string(),
            detail: format!(
                "expected proplist or nil for physics, got {}",
                other.type_name()
            ),
        }),
    }
}

fn value_to_int(definition: &str, function: &str, value: Value) -> Result<i32, EngineError> {
    match value {
        Value::Int(v) => Ok(v),
        // Numeric consumers use the C4Value integer conversion: a legacy
        // zero literal was emitted as nil, then converts back to integer 0.
        Value::Nil => Ok(0),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function: function.to_string(),
            detail: format!("expected int, got {}", other.type_name()),
        }),
    }
}

fn value_to_direction(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Direction, EngineError> {
    let raw = value_to_int(definition, function, value)?;
    Ok(Direction::from_raw(raw))
}

#[doc(hidden)]
pub fn value_to_command_direction(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<CommandDirection, EngineError> {
    let raw = value_to_int(definition, function, value)?;
    Ok(CommandDirection::from_raw(raw))
}

fn value_to_object_reference(
    definition: &str,
    function: &str,
    field: &str,
    value: Value,
) -> Result<Option<ObjectId>, EngineError> {
    match value {
        Value::Nil => Ok(None),
        Value::Int(id) => {
            if id < 0 {
                Ok(None)
            } else {
                Ok(Some(ObjectId::new(id as u64)))
            }
        }
        Value::Object(id) => {
            if id == 0 {
                Ok(None)
            } else {
                Ok(Some(ObjectId::new(id)))
            }
        }
        Value::Proplist(map) => match map.get("id") {
            Some(Value::Int(id)) if *id >= 0 => Ok(Some(ObjectId::new(*id as u64))),
            Some(other) => Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!(
                    "expected int for action.{} proplist id, got {}",
                    field,
                    other.type_name()
                ),
            }),
            None => Ok(None),
        },
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function: function.to_string(),
            detail: format!(
                "expected object, int, proplist, or nil for action.{field}, got {}",
                other.type_name()
            ),
        }),
    }
}

fn value_to_bool(definition: &str, function: &str, value: Value) -> Result<bool, EngineError> {
    match value {
        Value::Bool(v) => Ok(v),
        // A false literal below strict 3 is nil before this explicitly
        // boolean command-delta field consumes it.
        Value::Nil => Ok(false),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function: function.to_string(),
            detail: format!("expected bool, got {}", other.type_name()),
        }),
    }
}

fn value_to_spawns(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Vec<SpawnConfig>, EngineError> {
    let array = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!("expected array for spawn list, got {}", other.type_name()),
            });
        }
    };

    let mut spawns = Vec::with_capacity(array.len());
    for entry in array.into_iter() {
        let map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("spawn entry must be proplist, got {}", other.type_name()),
                });
            }
        };

        let definition_id = match map.get("definition") {
            Some(Value::String(id)) => id.clone(),
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("spawn definition must be string, got {}", other.type_name()),
                });
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: "spawn entry missing `definition`".into(),
                });
            }
        };

        let position = match map.get("position") {
            Some(value) => value_to_vector(definition, function, value.clone())?,
            None => Vector2::ZERO,
        };
        let velocity = match map.get("velocity") {
            Some(value) => value_to_vector(definition, function, value.clone())?,
            None => Vector2::ZERO,
        };
        let energy = match map.get("energy") {
            Some(value) => value_to_int(definition, function, value.clone())?,
            None => 0,
        };
        let owner = match map.get("owner") {
            Some(value) => value_to_int(definition, function, value.clone())?,
            None => OWNER_NONE,
        };

        let direction = match map.get("direction") {
            Some(value) => value_to_direction(definition, function, value.clone())?,
            None => Direction::default(),
        };

        let command_direction = match map.get("command_direction") {
            Some(value) => value_to_command_direction(definition, function, value.clone())?,
            None => CommandDirection::default(),
        };

        let mut action_state = ActionState::default();
        if let Some(value) = map.get("action") {
            if let Some(update) = value_to_action(definition, function, value.clone())? {
                action_state.apply_update(&update);
            }
        }
        if let Some(value) = map.get("action_phase") {
            let phase = value_to_int(definition, function, value.clone())?;
            let mut update = ActionUpdate::default();
            update.set_phase(phase);
            action_state.apply_update(&update);
        }

        let action_override = if action_state == ActionState::default() {
            None
        } else {
            Some(action_state)
        };

        let crew_member = match map.get("crew_member") {
            Some(value) => Some(value_to_bool(definition, function, value.clone())?),
            None => None,
        };

        let mut spawn = SpawnConfig::new(definition_id.clone())
            .with_position(position)
            .with_velocity(velocity)
            .with_energy(energy)
            .with_direction(direction)
            .with_command_direction(command_direction)
            .with_owner(owner);

        if let Some(action_state) = action_override {
            spawn = spawn.with_action(action_state);
        }

        if let Some(crew_member) = crew_member {
            spawn = spawn.with_crew_member(crew_member);
        }

        spawns.push(spawn);
    }

    Ok(spawns)
}

fn value_to_commands(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Vec<QueuedCommand>, EngineError> {
    let array = match value {
        Value::Array(values) => values,
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!("expected array for commands, got {}", other.type_name()),
            });
        }
    };

    let mut commands = Vec::with_capacity(array.len());
    for value in array {
        let map = match value {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!(
                        "expected proplist for command entry, got {}",
                        other.type_name()
                    ),
                });
            }
        };

        let mut delay: Option<u32> = None;
        let mut update = ObjectUpdate::default();
        let mut effects = Vec::new();
        let mut destroy = false;
        let mut spawns = Vec::new();
        let mut landscape_ops = Vec::new();

        for (key, value) in map.into_iter() {
            let Value::String(key) = key else {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("unexpected key `{key}` in command entry"),
                });
            };
            match key.as_ref() {
                "delay" => {
                    let raw_delay = value_to_int(definition, function, value)?;
                    if raw_delay < 0 {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: "delay must be >= 0".into(),
                        });
                    }
                    delay = Some(raw_delay as u32);
                }
                "position" => {
                    update.position = Some(value_to_vector(definition, function, value)?);
                }
                "velocity" => {
                    update.velocity = Some(value_to_vector(definition, function, value)?);
                }
                "energy" => {
                    update.energy = Some(value_to_int(definition, function, value)?);
                }
                "direction" => {
                    update.direction = Some(value_to_direction(definition, function, value)?);
                }
                "command_direction" => {
                    update.command_direction =
                        Some(value_to_command_direction(definition, function, value)?);
                }
                "action" => {
                    if let Some(action) = value_to_action(definition, function, value)? {
                        ensure_action_update(&mut update).merge(action);
                    }
                }
                "action_phase" => {
                    let phase = value_to_int(definition, function, value)?;
                    ensure_action_update(&mut update).set_phase(phase);
                }
                "owner" => {
                    update.owner = Some(value_to_int(definition, function, value)?);
                }
                "effects" => {
                    effects.extend(value_to_effect_commands(definition, function, value)?);
                }
                "destroy" => {
                    destroy = value_to_bool(definition, function, value)?;
                }
                "spawn" => {
                    spawns.extend(value_to_spawns(definition, function, value)?);
                }
                "landscape" => {
                    landscape_ops.extend(value_to_landscape_commands(definition, function, value)?);
                }
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{other}` in command entry"),
                    });
                }
            }
        }

        commands.push(
            QueuedCommand::new(delay.unwrap_or(0), update)
                .with_effects(effects)
                .with_spawns(spawns)
                .with_destroy(destroy)
                .with_landscape(landscape_ops),
        );
    }

    Ok(commands)
}

fn value_to_effect_commands(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Vec<EffectCommand>, EngineError> {
    let entries = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!("expected array for effects, got {}", other.type_name()),
            });
        }
    };

    let mut commands = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("effect entry must be proplist, got {}", other.type_name()),
                });
            }
        };

        let op = match map.shift_remove("op") {
            Some(Value::String(op)) => op,
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("effects.op must be string, got {}", other.type_name()),
                });
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: "effect entry missing `op`".into(),
                });
            }
        };

        match op.as_ref() {
            "add" => {
                let name_value =
                    map.shift_remove("name")
                        .ok_or_else(|| EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: "effect add entry missing `name`".into(),
                        })?;
                let name = match name_value {
                    Value::String(name) => name,
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: format!(
                                "effect name must be string, got {}",
                                other.type_name()
                            ),
                        });
                    }
                };

                let priority = match map.shift_remove("priority") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => 100,
                };

                // C4Effect stores signed intervals verbatim; zero alone
                // disables callbacks (C4Effect.cpp:67,342).
                let interval = match map.shift_remove("interval") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => 0,
                };

                let timer = match map.shift_remove("timer") {
                    Some(value) => {
                        let timer = value_to_int(definition, function, value)?;
                        if timer < 0 {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function: function.to_string(),
                                detail: "effect timer must be >= 0".into(),
                            });
                        }
                        timer
                    }
                    None => 0,
                };

                let command_target = match map.shift_remove("command_target") {
                    Some(Value::Int(value)) => Some(value),
                    Some(Value::Nil) | None => None,
                    Some(other) => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: format!(
                                "effect command_target must be int or nil, got {}",
                                other.type_name()
                            ),
                        });
                    }
                };

                let command_target_id = match map.shift_remove("command_target_id") {
                    Some(Value::String(value)) if !value.is_empty() => Some(value),
                    Some(Value::String(_)) | Some(Value::Nil) | None => None,
                    Some(other) => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: format!(
                                "effect command_target_id must be string or nil, got {}",
                                other.type_name()
                            ),
                        });
                    }
                };

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{}` in effect add entry", key),
                    });
                }

                let effect = EffectState::new(name)
                    .with_priority(priority)
                    .with_interval(interval)
                    .with_timer(timer)
                    .with_command_target(command_target)
                    .with_command_id(command_target_id);
                commands.push(EffectCommand::add(effect));
            }
            "remove" => {
                let name_value =
                    map.shift_remove("name")
                        .ok_or_else(|| EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: "effect remove entry missing `name`".into(),
                        })?;
                let name = match name_value {
                    Value::String(name) => name,
                    other => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: format!(
                                "effect name must be string, got {}",
                                other.type_name()
                            ),
                        });
                    }
                };
                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{}` in effect remove entry", key),
                    });
                }
                commands.push(EffectCommand::remove(name));
            }
            "clear" => {
                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{}` in effect clear entry", key),
                    });
                }
                commands.push(EffectCommand::Clear);
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("unsupported effect op `{}`", other),
                });
            }
        }
    }

    Ok(commands)
}

fn value_to_landscape_commands(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Vec<LandscapeCommand>, EngineError> {
    let entries = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!(
                    "expected array for landscape commands, got {}",
                    other.type_name()
                ),
            });
        }
    };

    let mut commands = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!(
                        "landscape entry must be proplist, got {}",
                        other.type_name()
                    ),
                });
            }
        };

        let op = match map.shift_remove("op") {
            Some(Value::String(op)) => op,
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("landscape.op must be string, got {}", other.type_name()),
                });
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: "landscape entry missing `op`".into(),
                });
            }
        };

        match op.as_ref() {
            "lower" => {
                let start = match map.shift_remove("start") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: "landscape lower entry missing `start`".into(),
                        });
                    }
                };

                let height = match map.shift_remove("height") {
                    Some(value) => value_to_int(definition, function, value)?,
                    None => {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: "landscape lower entry missing `height`".into(),
                        });
                    }
                };

                let end = if let Some(value) = map.shift_remove("end") {
                    value_to_int(definition, function, value)?
                } else if let Some(value) = map.shift_remove("width") {
                    let width = value_to_int(definition, function, value)?;
                    if width <= 0 {
                        return Err(EngineError::InvalidScriptOutput {
                            definition: definition.to_string(),
                            function: function.to_string(),
                            detail: "landscape lower width must be > 0".into(),
                        });
                    }
                    start + width
                } else {
                    start + 1
                };

                if end <= start {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: "landscape lower end must be greater than start".into(),
                    });
                }

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{}` in landscape lower entry", key),
                    });
                }

                commands.push(LandscapeCommand::LowerRange { start, end, height });
            }
            "set_liquid" => {
                let column_value =
                    match map.shift_remove("column").or_else(|| map.shift_remove("x")) {
                        Some(value) => value,
                        None => {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function: function.to_string(),
                                detail: "landscape set_liquid entry missing `column`".into(),
                            });
                        }
                    };

                let column = value_to_int(definition, function, column_value)?;

                let segments_value = map.shift_remove("segments").unwrap_or(Value::Nil);
                let segments = value_to_liquid_segments(definition, function, segments_value)?;

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{}` in landscape set_liquid entry", key),
                    });
                }

                commands.push(LandscapeCommand::SetLiquidColumn { column, segments });
            }
            "clear_liquid" => {
                let column_value =
                    match map.shift_remove("column").or_else(|| map.shift_remove("x")) {
                        Some(value) => value,
                        None => {
                            return Err(EngineError::InvalidScriptOutput {
                                definition: definition.to_string(),
                                function: function.to_string(),
                                detail: "landscape clear_liquid entry missing `column`".into(),
                            });
                        }
                    };

                let column = value_to_int(definition, function, column_value)?;

                if let Some((key, _)) = map.into_iter().next() {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function: function.to_string(),
                        detail: format!("unexpected key `{}` in landscape clear_liquid entry", key),
                    });
                }

                commands.push(LandscapeCommand::ClearLiquidColumn { column });
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!("unsupported landscape op `{other}`"),
                });
            }
        }
    }

    Ok(commands)
}

fn value_to_liquid_segments(
    definition: &str,
    function: &str,
    value: Value,
) -> Result<Vec<LiquidSegment>, EngineError> {
    let entries = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!(
                    "landscape segments must be array or nil, got {}",
                    other.type_name()
                ),
            });
        }
    };

    let mut segments = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut segment_map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: format!(
                        "landscape segment must be proplist, got {}",
                        other.type_name()
                    ),
                });
            }
        };

        let top_value =
            segment_map
                .shift_remove("top")
                .ok_or_else(|| EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: "landscape segment missing `top`".into(),
                })?;

        let bottom_value =
            segment_map
                .shift_remove("bottom")
                .ok_or_else(|| EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function: function.to_string(),
                    detail: "landscape segment missing `bottom`".into(),
                })?;

        let top = value_to_int(definition, function, top_value)?;
        let bottom = value_to_int(definition, function, bottom_value)?;

        if let Some((key, _)) = segment_map.into_iter().next() {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function: function.to_string(),
                detail: format!("unexpected key `{}` in landscape segment entry", key),
            });
        }

        segments.push(LiquidSegment::new(top, bottom));
    }

    Ok(segments)
}

#[cfg(test)]
#[path = "lib_tests/control_message_say_regression.rs"]
mod control_message_say_regression;

#[cfg(test)]
#[path = "lib_tests/l049_scenario_value_gain_regression.rs"]
mod l049_scenario_value_gain_regression;

#[cfg(test)]
#[path = "lib_tests/legacy_contents_order_regression.rs"]
mod legacy_contents_order_regression;

#[cfg(test)]
#[path = "lib_tests/no_standard_crew_idle_regression.rs"]
mod no_standard_crew_idle_regression;

#[cfg(test)]
#[path = "lib_tests/network_stats_control_counts_regression.rs"]
mod network_stats_control_counts_regression;

#[cfg(test)]
#[path = "lib_tests/player_list_order_regression.rs"]
mod player_list_order_regression;

#[cfg(test)]
#[path = "lib_tests/player_view_scroll_regression.rs"]
mod player_view_scroll_regression;

#[cfg(test)]
#[path = "lib_tests/frozen_blast_crossmap_regression.rs"]
mod frozen_blast_crossmap_regression;

#[cfg(test)]
#[path = "lib_tests/closed_border_landscape_accounting_regression.rs"]
mod closed_border_landscape_accounting_regression;

#[cfg(test)]
#[path = "lib_tests/signed_material_runtime_regression.rs"]
mod signed_material_runtime_regression;

#[cfg(test)]
#[path = "lib_tests/dig_out_material_cast_tick5_regression.rs"]
mod dig_out_material_cast_tick5_regression;

#[cfg(test)]
#[path = "lib_tests/startup_player_count_regression.rs"]
mod startup_player_count_regression;

#[cfg(test)]
#[path = "lib_tests/set_game_speed_regression.rs"]
mod set_game_speed_regression;

#[cfg(test)]
#[path = "lib_tests/custom_command_control_parity.rs"]
mod custom_command_control_parity;

#[cfg(test)]
#[path = "lib_tests/music_playlist_regression.rs"]
mod music_playlist_regression;

#[cfg(test)]
#[path = "lib_tests/set_pre_send_regression.rs"]
mod set_pre_send_regression;

#[cfg(test)]
#[path = "lib_tests/component_con_regression.rs"]
mod component_con_regression;

#[cfg(test)]
#[path = "lib_tests/command_contact_regression.rs"]
mod command_contact_regression;

#[cfg(test)]
#[path = "lib_tests/include_local_order_regression.rs"]
mod include_local_order_regression;

#[cfg(test)]
mod material_colorization_regression {
    use super::*;
    use std::fmt;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{subscriber, Level};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    const TEST_GAME_PALETTE: &[u8; 256 * 3] = include_bytes!("../../../planet/Graphics.c4g/C4.PAL");

    fn palette_pixel(index: u8) -> [u8; 4] {
        let offset = usize::from(index) * 3;
        let mut pixel = [
            TEST_GAME_PALETTE[offset] << 2,
            TEST_GAME_PALETTE[offset + 1] << 2,
            TEST_GAME_PALETTE[offset + 2] << 2,
            255,
        ];
        if index == 0 {
            pixel = [0, 0, 0, 0];
        } else if index == 191 {
            pixel = [0, 0, 255, 128];
        }
        pixel
    }

    fn pixels(entries: &[[u8; 4]]) -> Arc<[u8]> {
        Arc::from(
            entries
                .iter()
                .flat_map(|pixel| pixel.iter().copied())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    fn sprite(entries: &[[u8; 4]]) -> DefinitionSpriteImage {
        DefinitionSpriteImage {
            width: entries.len() as u32,
            height: 1,
            pixels: pixels(entries),
            color_mask: None,
        }
    }

    fn picture(entries: &[[u8; 4]]) -> DefinitionPictureImage {
        DefinitionPictureImage {
            width: entries.len() as u32,
            height: 1,
            pixels: pixels(entries),
            color_mask: None,
        }
    }

    fn gold_materials() -> MaterialSet {
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\n\
             Name=Gold\n\
             Color=10,20,30,40,50,60,70,80,90\n\
             Alpha=0,10,255\n",
        )
        .expect("Gold material parses");
        MaterialSet::from_resource_library(&library)
    }

    #[derive(Clone)]
    struct ErrorLayer {
        messages: Arc<Mutex<Vec<String>>>,
    }

    impl<S> Layer<S> for ErrorLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            if *event.metadata().level() != Level::ERROR {
                return;
            }
            let mut visitor = MessageVisitor::default();
            event.record(&mut visitor);
            if let Some(message) = visitor.message {
                self.messages.lock().unwrap().push(message);
            }
        }
    }

    #[derive(Default)]
    struct MessageVisitor {
        message: Option<String>,
    }

    impl Visit for MessageVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            if field.name() == "message" {
                self.message = Some(format!("{value:?}").trim_matches('"').to_string());
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.message = Some(value.to_string());
            }
        }
    }

    fn capture_errors(run: impl FnOnce()) -> Vec<String> {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(ErrorLayer {
            messages: Arc::clone(&messages),
        });
        subscriber::with_default(subscriber, run);
        let captured = messages.lock().unwrap().clone();
        captured
    }

    #[test]
    fn definition_graphics_lookups_use_legacy_byte_case_folding() {
        let lowercase_name = clonk_script::c4_string_from_bytes(b"\xfc");
        let uppercase_name = clonk_script::c4_string_from_bytes(b"\xdc");
        let mut definition =
            Definition::from_script("BYTE", "Byte graphics", "").expect("definition compiles");
        definition.set_sprite_variants(HashMap::from([(
            clonk_resources::material::c4_name_key(&lowercase_name),
            sprite(&[[1, 2, 3, 255]]),
        )]));
        definition
            .set_portrait_graphics(vec![(lowercase_name.clone(), picture(&[[4, 5, 6, 255]]))]);

        assert_eq!(
            definition
                .sprite_image_variant(Some(&uppercase_name))
                .expect("uppercase native query resolves the lowercase variant")
                .pixels()
                .as_ref(),
            [1, 2, 3, 255]
        );
        assert_eq!(
            definition
                .portrait_graphics(&uppercase_name)
                .expect("uppercase native query resolves the lowercase portrait")
                .pixels()
                .as_ref(),
            [4, 5, 6, 255]
        );
        assert!(resolved_graphics_equal(
            Some("BYTE"),
            Some(&lowercase_name),
            Some("BYTE"),
            Some(&uppercase_name),
        ));

        let mask = definition.solid_mask_pixels_for_rect(
            DefinitionTargetRect::new(0, 0, 1, 1, 0, 0),
            Some(&uppercase_name),
        );
        assert!(matches!(mask, SolidMaskPixels::Alpha(_)));
    }

    #[test]
    fn color_by_material_recolors_palette_keys_after_material_load_across_graphics_chain() {
        let unmatched = [1, 2, 3, 4];
        let source = [
            palette_pixel(0),
            palette_pixel(1),
            palette_pixel(2),
            palette_pixel(3),
            palette_pixel(4),
            unmatched,
        ];
        let mut definition =
            Definition::from_script("TINT", "Tinted", "").expect("definition compiles");
        definition.color_by_material = "gOlD".to_string();
        definition.set_sprite_image(Some(sprite(&source)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(3, 0, 1, 1, 0, 0)));
        definition.set_sprite_variants(HashMap::from([(
            "extra".to_string(),
            sprite(&[palette_pixel(5)]),
        )]));
        definition.set_picture_image(Some(picture(&[palette_pixel(6)])));
        definition.set_portrait_image(Some(picture(&[palette_pixel(4)])));
        definition.set_portrait_graphics_image(Some(picture(&[palette_pixel(5)])));
        definition.set_portrait_graphics(vec![("1".to_string(), picture(&[palette_pixel(6)]))]);

        let mut engine = Engine::new();
        engine.set_materials(gold_materials());
        engine
            .register_definition(definition)
            .expect("definition registers after materials");

        let definition = engine.definitions.get("TINT").expect("definition retained");
        assert_eq!(
            definition.sprite_image().unwrap().pixels().as_ref(),
            pixels(&[
                palette_pixel(0),
                [10, 20, 30, 255],
                [40, 50, 60, 245],
                [70, 80, 90, 0],
                [10, 20, 30, 255],
                unmatched,
            ])
            .as_ref(),
        );
        assert_eq!(
            definition
                .sprite_image_variant(Some("extra"))
                .unwrap()
                .pixels()
                .as_ref(),
            [40, 50, 60, 245],
        );
        assert_eq!(
            definition.picture_image().unwrap().pixels().as_ref(),
            [70, 80, 90, 0],
        );
        assert_eq!(
            definition.portrait_image().unwrap().pixels().as_ref(),
            [10, 20, 30, 255],
        );
        assert_eq!(
            definition
                .portrait_graphics_image()
                .unwrap()
                .pixels()
                .as_ref(),
            [40, 50, 60, 245],
        );
        assert_eq!(
            definition.portrait_graphics("1").unwrap().pixels().as_ref(),
            [70, 80, 90, 0],
        );
        match &definition.solid_mask_pixels {
            SolidMaskPixels::Alpha(mask) => assert_eq!(mask.as_slice(), [0]),
            _ => panic!("material alpha must rebuild the cached solid-mask pixels"),
        }
    }

    #[test]
    fn unknown_color_by_material_logs_cpp_error_and_leaves_graphics_untinted() {
        let original = palette_pixel(1);
        let mut definition =
            Definition::from_script("MISS", "Missing", "").expect("definition compiles");
        definition.color_by_material = "UnknownGold".to_string();
        definition.set_sprite_image(Some(sprite(&[original])));

        let mut engine = Engine::new();
        engine.set_materials(gold_materials());
        let messages = capture_errors(|| {
            engine
                .register_definition(definition)
                .expect("unknown material is log-only");
        });

        assert_eq!(
            messages,
            ["C4Def::ColorizeByMaterial: mat UnknownGold not defined"]
        );
        assert_eq!(
            engine
                .definitions
                .get("MISS")
                .unwrap()
                .sprite_image()
                .unwrap()
                .pixels()
                .as_ref(),
            original,
        );
    }
}

#[cfg(test)]
#[path = "lib_tests/missing_include_regression.rs"]
mod missing_include_regression;

#[cfg(test)]
#[path = "lib_tests/script_relink_regression.rs"]
mod script_relink_regression;

#[cfg(test)]
#[path = "lib_tests/internal_player_script_control_parity.rs"]
mod internal_player_script_control_parity;

#[cfg(test)]
#[path = "lib_tests/em_move_object_control_parity.rs"]
mod em_move_object_control_parity;

#[cfg(test)]
#[path = "lib_tests/em_draw_tool_control_parity.rs"]
mod em_draw_tool_control_parity;

#[cfg(test)]
#[path = "lib_tests/em_drop_def_control_parity.rs"]
mod em_drop_def_control_parity;

#[cfg(test)]
#[path = "lib_tests/script_control_execution_tests.rs"]
mod script_control_execution_tests;

#[cfg(test)]
#[path = "lib_tests/object_action_no_other_regression.rs"]
mod object_action_no_other_regression;

#[cfg(test)]
#[path = "lib_tests/put_command_regression.rs"]
mod put_command_regression;

#[cfg(test)]
#[path = "lib_tests/owner_overlay_solid_mask_regression.rs"]
mod owner_overlay_solid_mask_regression;

#[cfg(test)]
#[path = "lib_tests/solid_mask_state_regression.rs"]
mod solid_mask_state_regression;

#[cfg(test)]
#[path = "lib_tests/scenario_section_random_regression.rs"]
mod scenario_section_random_regression;

#[cfg(test)]
#[path = "lib_tests/pathfinder_host_state_regression.rs"]
mod pathfinder_host_state_regression;

#[cfg(test)]
#[path = "lib_tests/landscape_push_pull_regression.rs"]
mod landscape_push_pull_regression;

#[cfg(test)]
#[path = "lib_tests/audio_detach_regression.rs"]
mod audio_detach_regression;

#[cfg(test)]
#[path = "lib_tests/deferred_rank_extension_regression.rs"]
mod deferred_rank_extension_regression;
