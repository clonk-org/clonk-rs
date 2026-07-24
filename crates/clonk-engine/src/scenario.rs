use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use clonk_resources::definition::{
    ActionFacet as ResourceActionFacet, DefCore as ResourceDefCore,
    DefinitionGraphicsVariant as ResourceGraphicsVariant,
};
use clonk_resources::{
    decode_legacy_script_text, localize_script_source_with_components,
    ActionDefinition as ResourceActionDefinition, ActionMap as ResourceActionMap, ColorByOwnerMask,
    ComponentGroups, DefinitionError as ResourceDefinitionError, GraphicsImage, Group, GroupError,
    LanguagePacks, ParticleDefinition as ResourceParticleDefinition, RankNameTable,
    ResourceDefinition as ResourceDefinitionData,
};
use image::{load_from_memory, ImageError, ImageFormat};
use serde::de::Error as _;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::action::is_builtin_idle_name;
use crate::landscape::{
    LandscapeRasterState, RuntimeTexMapLookup, RuntimeTexMapMaterial, RuntimeTexMapState,
};
use crate::network_game_data::{
    decode_legacy_game_string, parse_landscape_game_data, InitialNetworkGameApplyError,
    InitialNetworkGameData, InitialNetworkGameError, LandscapeGameData,
};
use crate::{
    action::ActionSpec, ActionState, CommandDirection, Definition, DefinitionActionFacet,
    DefinitionActionGraphics, DefinitionComponent, DefinitionId, DefinitionPicture,
    DefinitionPictureImage, DefinitionRect, DefinitionSpriteImage, Direction, EffectState,
    EffectVarValue, Engine, EngineError, EnvironmentSettings, Landscape, LegacyCString,
    MaterialSet, MovementProfile, ObjectId, ObjectStatus, PhysicsSettings, RgbColor,
    RoundResultsState, ScoreboardState, ScriptGlobalState, SkyFrame, SkyParallaxMode, SkySettings,
    SpawnConfig, TeamInfo, Vector2, FULL_CON, LANDSCAPE_MODE_DYNAMIC, LANDSCAPE_MODE_EXACT,
    LANDSCAPE_MODE_STATIC,
};

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("scenario manifest `Scenario.json` not found")]
    ManifestMissing,
    #[error("legacy scenario core `Scenario.txt` not found")]
    LegacyCoreMissing,
    #[error("invalid legacy scenario section name `{path}`; expected 1..=30 bytes")]
    InvalidScenarioSectionName { path: PathBuf },
    #[error("failed to parse scenario manifest: {0}")]
    ManifestParse(#[from] serde_json::Error),
    #[error("scenario resource error: {0}")]
    Resources(#[from] GroupError),
    #[error("material enumeration failed: {0}")]
    MaterialEnumeration(#[from] clonk_resources::material::MaterialEnumerationError),
    #[error("script file `{path}` is missing from the scenario")]
    MissingScript { path: PathBuf },
    #[error("script file `{path}` is not valid UTF-8")]
    ScriptEncoding { path: PathBuf },
    #[error("legacy scenario core `Scenario.txt` is not valid UTF-8")]
    LegacyCoreEncoding,
    #[error("legacy scenario definition source `{path}` could not be located")]
    LegacyDefinitionNotFound { path: String },
    #[error("definition load error: {0}")]
    Definition(#[from] ResourceDefinitionError),
    #[error("invalid legacy scenario data: {0}")]
    LegacyParse(String),
    #[error("classic scenario title cannot be truncated at byte {limit} without splitting UTF-8")]
    LoaderTitleTruncationBoundary { limit: usize },
    #[error("legacy objects file `Objects.txt` is not valid UTF-8")]
    LegacyObjectsEncoding,
    #[error("invalid legacy objects data: {0}")]
    LegacyObjectsParse(String),
    #[error("invalid legacy round results data: {0}")]
    LegacyRoundResultsParse(String),
    #[error("legacy map `Map.bmp` could not be decoded: {source}")]
    LegacyMapDecode {
        #[source]
        source: ImageError,
    },
    #[error("legacy map `Map.bmp` has zero width or height")]
    LegacyMapEmpty,
    #[error("duplicate definition id `{0}` in scenario manifest")]
    DuplicateDefinition(String),
    #[error("initial object references unknown definition `{0}`")]
    UnknownDefinition(String),
    #[error("initial object handle `{0}` is duplicated")]
    DuplicateHandle(String),
    #[error("initial object references unknown container handle `{0}`")]
    UnknownContainerHandle(String),
    #[error("container dependency cycle detected for handle `{0}`")]
    ContainerDependencyCycle(String),
    #[error("invalid landscape specification: {0}")]
    InvalidLandscape(String),
    #[error("scenario did not declare any object definitions")]
    NoDefinitions,
    #[error("invalid physics settings: {0}")]
    InvalidPhysics(String),
    #[error("definition `{id}` has invalid movement settings: {detail}")]
    InvalidMovement { id: String, detail: String },
    #[error("sky surface `{path}` is missing from the scenario")]
    SkySurfaceMissing { path: PathBuf },
    #[error("failed to decode sky surface `{path}`: {source}")]
    SkySurfaceDecode {
        path: PathBuf,
        #[source]
        source: ImageError,
    },
    #[error("invalid sky configuration: {0}")]
    InvalidSky(String),
    #[error("engine error while applying scenario: {0}")]
    Engine(#[from] EngineError),
    #[error("invalid saved Game.txt runtime state: {0}")]
    InitialNetworkGameApply(#[from] InitialNetworkGameApplyError),
    #[error("invalid saved Game.txt compiled runtime section: {0}")]
    InitialNetworkRuntime(String),
    #[error("initial network Scenario.txt serialization requires a legacy Scenario.txt core")]
    InitialNetworkScenarioUnsupported,
    #[error("initial record Scenario.txt serialization requires a legacy Scenario.txt core")]
    InitialRecordScenarioUnsupported,
    #[error("initial network team metadata requires a legacy Scenario.txt core")]
    InitialNetworkTeamMetadataUnsupported,
    #[error("initial network team {team_id} has no scenario-derived C++ default color")]
    InitialNetworkTeamColorUnsupported { team_id: i32 },
    #[error("initial network team distribution value {value} has no known C++ semantic")]
    InitialNetworkTeamDistributionUnsupported { value: u8 },
    #[error("offline startup preflight does not support JSON Scenario.json manifests")]
    OfflineStartupJsonUnsupported,
    #[error("offline startup preflight does not support legacy savegames yet")]
    OfflineStartupSavegameUnsupported,
    #[error("offline startup preflight does not support legacy replays yet")]
    OfflineStartupReplayUnsupported,
    #[error("offline startup preflight does not support SavePlayerInfos.txt yet")]
    OfflineStartupRestoreInfosUnsupported,
}

/// The C4GameParameters player capacity available before offline InitLocal.
///
/// This preflight deliberately excludes definitions, materials and landscape
/// creation so callers can admit configured players before constructing a
/// `MapPlayerExtend` landscape. `random_seed` is `Some` only when an existing
/// Parameters.txt supplied the compiled seed; a missing component leaves the
/// application to install C++'s time/LC_PIN_SEED default before map creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineScenarioStartupPreflight {
    pub max_players: i32,
    pub random_seed: Option<i32>,
    /// `C4S.Head.SaveGame`: ordinary offline startup still admits the
    /// configured local player files, but associates them with the saved
    /// restore rows before recreating the live players.
    pub save_game: bool,
}

/// Parameters that must be frozen before a replay's dynamic landscape is
/// created. `startup_player_count` already includes C4Game's frame-zero
/// overwrite from `PlayerInfos.txt`; nonzero-frame records retain the value
/// serialized in `Parameters.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayScenarioStartupPreflight {
    pub random_seed: i32,
    pub startup_player_count: i32,
}

#[derive(Debug, Clone)]
struct ScenarioDefinition {
    id: String,
    name: Option<String>,
    description: Option<String>,
    clonk_names: Option<String>,
    script: String,
    /// Native C4ScriptHost::ScriptName (`<group full name>/Script.c`).
    script_name: Option<String>,
    actions: Option<DefinitionActions>,
    crew_member: bool,
    can_be_base: bool,
    movement: MovementProfile,
    category: i32,
    value: i32,
    mass: i32,
    picture: Option<DefinitionPicture>,
    picture_image: Option<GraphicsImage>,
    picture_color_by_owner_mask: Option<ColorByOwnerMask>,
    graphics_image: Option<GraphicsImage>,
    color_by_owner_mask: Option<ColorByOwnerMask>,
    additional_graphics: HashMap<String, ResourceGraphicsVariant>,
    /// First def portrait (C4CFN_Portraits, src/C4Components.h:88) for the
    /// HUD cursor info (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-320).
    portrait_image: Option<GraphicsImage>,
    portrait_graphics_image: Option<GraphicsImage>,
    portrait_color_by_owner_mask: Option<ColorByOwnerMask>,
    portrait_graphics: Vec<ResourceGraphicsVariant>,
    /// Def rank symbols (C4Def::pRankSymbols, src/C4Def.cpp:684-691).
    rank_symbols_image: Option<GraphicsImage>,
    rank_names: Option<RankNameTable>,
    rank_base: Option<i32>,
    rank_symbol_count: Option<u32>,
    resource_group: Option<Group>,
    components: Vec<DefinitionComponent>,
    line_connect: u32,
    /// DefCore shape vertices + rect (the spawn shape; task #15 carries
    /// the rest of the core).
    vertices: Vec<clonk_resources::definition::DefVertex>,
    shape: Option<clonk_resources::definition::PictureRect>,
    /// The FULL DefCore for legacy defs — applied via
    /// Engine::apply_resource_core so no core field silently drops
    /// (physicals/Float/Timer/Grab did).
    core: Option<clonk_resources::definition::DefCore>,
}

#[derive(Debug, Clone)]
pub struct SkyConfig {
    pub settings: SkySettings,
    pub surface: Option<Arc<GraphicsImage>>,
}

#[derive(Debug, Clone)]
struct DefinitionActions {
    default_action: Option<String>,
    specs: HashMap<String, ActionSpec>,
    physical: Vec<(String, ActionSpec)>,
    graphics: HashMap<String, DefinitionActionGraphics>,
    reflections: HashMap<String, crate::action::C4ActionReflection>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScenarioSpawn {
    pub(crate) handle: Option<String>,
    pub(crate) container_handle: Option<String>,
    /// Separately compiled C4ObjectList links, in exact First -> Next order.
    pub(crate) contents_handles: Vec<String>,
    /// C4Object::nInfo from `Info=`. Binding this name to player-owned crew
    /// metadata happens after the corresponding player files are restored.
    pub(crate) info_name: Option<String>,
    pub(crate) config: SpawnConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct ScenarioSectionSpec {
    pub(crate) name: String,
    /// Original section payload retained for C4ScenarioSection::EnsureTempStore.
    /// The implicit main section points at the scenario root and is filtered
    /// to C4FLS_Section when it first becomes modified; named sections retain
    /// their complete child group so files outside the two saved categories
    /// survive a landscape-only or objects-only switch.
    pub(crate) source_group: Option<Group>,
    pub(crate) landscape: Option<Landscape>,
    pub(crate) landscape_systems: ScenarioLandscapeSystems,
    pub(crate) exact_landscape: bool,
    pub(crate) texmap_lookups: Vec<RuntimeTexMapLookup>,
    pub(crate) resynthesize_static_map: bool,
    pub(crate) map_creator: Option<crate::map_creator_s2::MapCreatorS2State>,
    pub(crate) s2_overload: Option<ScenarioSectionS2Spec>,
    pub(crate) gravity: LegacyC4SVal,
    pub(crate) post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks,
    pub(crate) keep_map_creator: bool,
    pub(crate) no_initialize: bool,
    /// Precompiled fallback for the active root during initial startup and
    /// synthetic sections without a backing C4Group. Inactive real sections
    /// leave this empty and compile their source/frozen Objects.txt on every
    /// activation.
    pub(crate) objects: Vec<ScenarioSpawn>,
    pub(crate) scenario_values: ScenarioValueStore,
    pub(crate) base_reject_entrance_enabled: bool,
    pub(crate) base_extinguish_enabled: bool,
    pub(crate) environment: EnvironmentSettings,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ScenarioLandscapeSystems {
    pub(crate) pxs: Option<crate::pxs::PxsSystem>,
    pub(crate) mass_movers: Option<crate::mass_mover::MassMoverSet>,
}

/// The source and scenario values that C4Landscape::Init consumes only when
/// a section is activated. A retained C4MapCreatorS2 must parse this source
/// into its live tree; eager section preparation cannot resolve templates or
/// first-free texture slots contributed by previously active sections.
#[derive(Debug, Clone)]
pub(crate) struct ScenarioSectionS2Spec {
    pub(crate) source: String,
    pub(crate) map_width: LegacyC4SVal,
    pub(crate) map_height: LegacyC4SVal,
    pub(crate) map_player_extend: bool,
    pub(crate) player_count: i32,
    pub(crate) map_zoom: LegacyC4SVal,
    pub(crate) diff: Option<clonk_resources::bitmap::IndexedBitmap>,
    pub(crate) left_open: i32,
    pub(crate) right_open: i32,
    pub(crate) top_open: bool,
    pub(crate) bottom_open: bool,
    pub(crate) auto_scan_side_open: bool,
    pub(crate) no_scan: bool,
    pub(crate) shade_materials: bool,
    pub(crate) script_functions: HashSet<String>,
}

#[derive(Debug, Clone)]
struct ScenarioScriptSource {
    name: String,
    source: String,
    /// Real (legacy) content gets the C++ callback convention — no
    /// synthetic state argument (Game.Script.Call(PSF_Initialize) has no
    /// parameters; GRBroadcast args start with the player number,
    /// C4Player.cpp:769-775). JSON fixtures keep the state proplist.
    c4_args: bool,
}

/// Named non-global functions visible from the fully linked scenario host
/// when Landscape.txt is parsed. Build the same lightweight script-host tree
/// used at apply time so #appendto is resolved before scenario #include.
/// Engine-global functions remain excluded: Game.Script.GetSFunc performs a
/// local-owner lookup and the declaring host's global FnLink is unnamed.
fn scenario_map_callback_functions(
    script: Option<&ScenarioScriptSource>,
    definitions: &[ScenarioDefinition],
    definition_load_steps: &[DefinitionLoadStep],
    scenario_system_scripts: &[(String, String)],
) -> Result<HashSet<String>, ScenarioError> {
    let Some(script) = script else {
        return Ok(HashSet::new());
    };

    let mut linker = Engine::new();
    for step in definition_load_steps {
        match step {
            DefinitionLoadStep::Definition(id) => {
                let definition = definitions
                    .iter()
                    .find(|definition| definition.id.eq_ignore_ascii_case(id))
                    .ok_or_else(|| ScenarioError::UnknownDefinition(id.clone()))?;
                let name = definition.name.as_deref().unwrap_or(&definition.id);
                let mut compiled =
                    Definition::from_script(&definition.id, name, &definition.script)
                        .or_else(|_| Definition::from_script(&definition.id, name, ""))?;
                if let Some(script_name) = &definition.script_name {
                    compiled.set_script_name(script_name.clone());
                }
                compiled.set_c4_callback_convention(true);
                linker.register_definition(compiled)?;
            }
            DefinitionLoadStep::SystemScripts(sources) => {
                linker.install_additional_global_scripts(sources);
            }
            // Particle definitions have no script host. Their render and
            // simulation metadata is registered only when the scenario is
            // applied to the live engine.
            DefinitionLoadStep::Particle(_) => {}
            // Destroyed overload hosts retain declarations only; their
            // functions no longer participate in append/include linking.
            DefinitionLoadStep::Declarations { .. } => {}
        }
    }

    match linker.load_scenario_script_with_convention(&script.name, &script.source, true) {
        Ok(()) => {}
        Err(EngineError::Script { .. }) => return Ok(HashSet::new()),
        Err(other) => return Err(other.into()),
    }
    linker.install_scenario_global_scripts(scenario_system_scripts);
    linker.resolve_appends();
    linker.resolve_includes()?;
    Ok(linker.scenario_local_function_names())
}

#[derive(Debug, Clone)]
enum DefinitionLoadStep {
    Definition(String),
    Declarations { name: String, source: String },
    SystemScripts(Vec<(String, String)>),
    Particle(ResourceParticleDefinition),
}

#[derive(Debug)]
enum CollectedDefinition {
    Definition(ScenarioDefinition),
    SystemScripts(Vec<(String, String)>),
    Particle(ResourceParticleDefinition),
}

// C4Game::InitDefs checks every loaded definition against the running engine
// tuple before script linking (C4Game.cpp:108-115; C4Version.h:28-32).
const DEFINITION_ENGINE_VERSION: [i32; 5] = [4, 9, 11, 0, 362];

fn definition_requires_newer_engine(definition: &ScenarioDefinition) -> bool {
    let Some(core) = definition.core.as_ref() else {
        // Scenario.json definitions are a Rust-only fixture surface, not
        // C4Def entries participating in CheckEngineVersion.
        return false;
    };
    let version = core.version;
    match version[..4].cmp(&DEFINITION_ENGINE_VERSION[..4]) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // CompareVersion treats a non-positive definition build as a
        // wildcard and only compares positive candidate builds.
        std::cmp::Ordering::Equal => version[4] > 0 && version[4] > DEFINITION_ENGINE_VERSION[4],
    }
}

fn prune_incompatible_definitions(definitions: &mut Vec<ScenarioDefinition>) {
    definitions.retain(|definition| !definition_requires_newer_engine(definition));

    // C4DefList::CheckRequireDef repeats removal until membership is stable:
    // removing one dependency can invalidate another definition on the next
    // pass. Closed cycles and self-requirements remain because every named ID
    // is present throughout the fixpoint.
    loop {
        let present_ids: HashSet<String> = definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect();
        let previous_len = definitions.len();
        definitions.retain(|definition| {
            definition.core.as_ref().is_none_or(|core| {
                core.require_defs
                    .iter()
                    .all(|required| present_ids.contains(required))
            })
        });
        if definitions.len() == previous_len {
            break;
        }
    }
}

#[derive(Debug)]
pub struct Scenario {
    /// Parsed C4Scenario core retained for C++-format save/network
    /// serialization. JSON-only fixtures have no legacy core.
    legacy_core: Option<LegacyScenarioCore>,
    /// Post-compile Teams.txt state. `None` means the legacy group had no
    /// Teams.txt, so C4TeamList::Load derives defaults from the scenario.
    legacy_team_metadata: Option<LoadedLegacyTeamMetadata>,
    name: Option<String>,
    description: Option<String>,
    ticks: Option<u32>,
    ground_height_hint: Option<i32>,
    /// The ordered C4Game::InitMaterialTexture material chain retained from
    /// legacy loading. Scenario-local definitions precede admitted external
    /// groups and therefore win duplicate names (C4Game.cpp:901-977,
    /// C4Material.cpp:263-299).
    material_library: Option<clonk_resources::MaterialLibrary>,
    definitions: Vec<ScenarioDefinition>,
    /// `[Game] ValueOverloads`: C4Game::InitValueOverloads applies these to
    /// the loaded definitions immediately before Objects.Load
    /// (C4Game.cpp:2704-2713,3997-4004).
    value_overloads: Vec<(String, i32)>,
    initial_spawns: Vec<ScenarioSpawn>,
    landscape: Option<Landscape>,
    post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks,
    keep_map_creator: bool,
    scenario_sections: Vec<ScenarioSectionSpec>,
    physics: Option<PhysicsSettings>,
    /// Runtime C4Landscape::CompileFunc state restored from a savegame's
    /// Game.txt. Its presence also suppresses the fresh ScenarioInit gravity
    /// overwrite when the scenario is applied.
    runtime_landscape: Option<LandscapeGameData>,
    /// The C4Aul string enumeration loaded from `Strings.txt`. Compiled
    /// Globals/GlobalNamed/effect variables refer to these integer IDs.
    legacy_string_table: clonk_script::StringRegistrations,
    /// `RoundResults.txt` compiled after InitControl and before pointer
    /// denumeration. A missing component retains C4RoundResults::Init's
    /// scenario-melee default.
    round_results: RoundResultsState,
    /// The `[Landscape] Gravity` C4SVal — evaluated through the synced
    /// ledger at apply time (C4Landscape::ScenarioInit, C4Landscape.cpp:66).
    gravity: LegacyC4SVal,
    environment: Option<EnvironmentSettings>,
    sky: Option<SkyConfig>,
    script: Option<ScenarioScriptSource>,
    objectives: ScenarioObjectives,
    construction_needs_material: bool,
    structures_need_energy: bool,
    base_buy_enabled: bool,
    base_sell_enabled: bool,
    base_auto_sell_enabled: bool,
    base_reject_entrance_enabled: bool,
    base_regenerate_energy_enabled: bool,
    base_extinguish_enabled: bool,
    base_regenerate_energy_price: i32,
    landscape_insert_thrust: bool,
    /// `[Head] DisableMouse`: prevents every joined player from receiving
    /// mouse control (`C4Player::InitControl`, C4Player.cpp:1907-1912).
    disable_mouse: bool,
    /// `[Head] ForcedAutoContextMenu`: `None` keeps the player-file
    /// preference; `Some` forces automatic context menus for all players
    /// (C4Player::ApplyForcedControl, C4Player.cpp:2369-2375).
    forced_auto_context_menu: Option<bool>,
    /// `[Head] ForcedAutoStopControl`: `None` keeps the player-file
    /// preference; `Some` forces classic/Jump'n'Run control for all players
    /// (C4Player::ApplyForcedControl, C4Player.cpp:2369-2389).
    forced_control_style: Option<bool>,
    /// The surviving definition hosts and definition-pack System.c4g hosts in
    /// their C4DefList::Load order. System hosts remain in place when a later
    /// definition overload removes an earlier same-ID definition host.
    definition_load_steps: Vec<DefinitionLoadStep>,
    /// Ordered external/folder definition resources used by the Rust load.
    /// Old saves with a Game.txt DefinitionFiles block expose that later C++
    /// override as unresolved in `lobby_metadata`; the scenario group itself
    /// is deliberately absent.
    definition_resource_paths: Vec<PathBuf>,
    /// Exact ordered `NRT_Definitions` roots registered in `Game.GroupSet`.
    /// This includes folder-local definition roots after the selected external
    /// vector: C++ first registers those folders at folder priority, then adds
    /// them to `DefinitionFilenames` and registers them again at definition
    /// priority (C4Game.cpp:210-213,2432-2442,3961-3994).
    definition_root_groups: Vec<Group>,
    /// Exact `C4SoundSystem::LoadEffects` group stream produced by
    /// `C4DefList::Load`, in native load order. Unlike the surviving
    /// definition list, this retains pure sound `.c4d` groups, rejected
    /// DefCore groups and duplicate visits so later samples can overload
    /// earlier samples exactly as they do in C++.
    sound_effect_groups: Vec<Group>,
    /// The scenario's own System.c4g sources. C++ loads these after defs
    /// specifically to give them overload priority (C4Game.cpp:2606-2617).
    scenario_system_scripts: Vec<(String, String)>,
    /// The four C4SPlrStart slots, consumed by joining players
    /// (C4Player::ScenarioInit, C4Player.cpp:670-777).
    player_starts: Vec<PlayerStart>,
    /// Ordered `Game.Teams` entries from the scenario's Teams.txt.
    teams: Vec<TeamInfo>,
    /// Immutable, pre-game presentation inputs retained from Scenario.txt,
    /// Parameters.txt and Teams.txt. JSON fixture scenarios deliberately keep
    /// this as `None`: those files do not declare the legacy lobby contract.
    lobby_metadata: Option<ScenarioLobbyMetadata>,
    /// The scenario's own Names.txt, overriding the standard clonk names
    /// in Game.Names (C4Game.cpp:3288-3289).
    standard_names: Option<String>,
    /// `[Landscape] MapZoom` kept as a C4SVal: ScenarioInit evaluates it
    /// per configured start coordinate (C4Player.cpp:713-714).
    map_zoom: LegacyC4SVal,
    /// The C4Weather::Init scenario evaluates (C4Weather.cpp:36-70):
    /// present only for legacy scenario loads — `apply` replays the
    /// synced-RNG init draws so the ledger matches C++ from frame 0.
    pub(crate) weather_init: Option<LegacyWeatherInit>,
    /// The C4Game::InitGame environment placements (C4Game.cpp:2493-2503);
    /// present only for legacy scenario loads.
    pub(crate) init_placement: Option<LegacyInitPlacement>,
}

/// One ordered C4IDList entry exposed without depending on clonk-network's wire
/// types. A count omitted in Scenario.txt is represented as zero, matching
/// C4IDList::Entry::CompileFunc (C4IDList.cpp:250-267).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioIdListEntry {
    pub id: String,
    pub count: i32,
}

impl ScenarioIdListEntry {
    pub fn new(id: impl Into<String>, count: i32) -> Self {
        Self {
            id: id.into(),
            count,
        }
    }
}

/// The authoritative `C4GameParameters::Rules` and `Goals` lists used by
/// `C4Game::InitRules`/`InitGoals`. Network startup supplies these from
/// JoinData so every peer places the same post-conversion, lobby-adjusted
/// objects instead of rereading its local Scenario.txt lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameParameterRuleGoalLists {
    rules: Vec<ScenarioIdListEntry>,
    goals: Vec<ScenarioIdListEntry>,
}

impl GameParameterRuleGoalLists {
    pub fn new(rules: Vec<ScenarioIdListEntry>, goals: Vec<ScenarioIdListEntry>) -> Self {
        Self { rules, goals }
    }

    pub fn rules(&self) -> &[ScenarioIdListEntry] {
        &self.rules
    }

    pub fn goals(&self) -> &[ScenarioIdListEntry] {
        &self.goals
    }
}

/// Scenario-derived inputs used by the initial network resource and
/// Parameters.txt paths. Values reflect the loaded C4Scenario after its
/// legacy goal/rule conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkScenarioMetadata {
    pub icon: i32,
    pub definition_modules: Vec<String>,
    pub random_seed: i32,
    pub max_players: i32,
    pub use_fair_crew: bool,
    pub fair_crew_forced: bool,
    pub fair_crew_strength: i32,
    pub rules: Vec<ScenarioIdListEntry>,
    pub goals: Vec<ScenarioIdListEntry>,
}

/// C4TeamList's team-distribution mode, kept independent from clonk-network's
/// binary representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InitialNetworkTeamDistribution {
    Free = 0,
    Host = 1,
    None = 2,
    Random = 3,
    RandomInvisible = 4,
}

/// One ordered team in the scenario-derived initial C4TeamList.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkTeam {
    pub id: i32,
    pub name: LegacyCString,
    pub player_start_index: i32,
    pub player_ids: Vec<i32>,
    pub color: u32,
    pub icon_spec: LegacyCString,
    pub max_players: i32,
}

/// The complete neutral engine snapshot needed to construct the initial
/// JoinGameParameters C4TeamList.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkTeamMetadata {
    pub active: bool,
    pub custom: bool,
    pub allow_hostility_change: bool,
    pub allow_team_switch: bool,
    pub auto_generate_teams: bool,
    pub last_team_id: i32,
    pub team_distribution: InitialNetworkTeamDistribution,
    pub team_colors: bool,
    pub max_script_players: i32,
    pub script_player_names: LegacyCString,
    pub random_team_count: i32,
    pub teams: Vec<InitialNetworkTeam>,
}

impl InitialNetworkTeamMetadata {
    fn teams_file_defaults() -> Self {
        Self {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 0,
            team_distribution: InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        }
    }

    fn without_teams_file(game: &LegacyGame) -> Self {
        // GetIDCount(..., 1) treats a present zero-count melee/rivalry entry
        // as enabled (C4Teams.cpp:625-632; C4IDList.cpp:75-82).
        let melee = legacy_id_count_or(&game.goals, "MELE", 1) != 0
            || legacy_id_count_or(&game.rules, "RVLR", 1) != 0
            || legacy_id_count_or(&game.goals, "MEL2", 1) != 0;
        Self {
            active: melee,
            custom: false,
            allow_hostility_change: true,
            allow_team_switch: false,
            auto_generate_teams: melee,
            last_team_id: 0,
            team_distribution: InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        }
    }
}

/// Legacy scenario data needed before a game is running. The projection keeps
/// Scenario.txt defaults separate from Parameters.txt overrides and Teams.txt
/// state, matching the C++ ownership boundaries instead of inventing a single
/// already-resolved game-parameter object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyMetadata {
    head: ScenarioLobbyHead,
    definitions: ScenarioLobbyDefinitions,
    game_parameter_defaults: ScenarioGameParameterValues,
    embedded_game_parameters: Option<ScenarioGameParameterOverrides>,
    teams: ScenarioLobbyTeams,
}

impl ScenarioLobbyMetadata {
    /// Scenario.txt `[Head]` values and their C4Scenario-derived defaults.
    pub fn head(&self) -> &ScenarioLobbyHead {
        &self.head
    }

    /// Scenario.txt `[Definitions]` plus the external selection actually used
    /// by this load.
    pub fn definitions(&self) -> &ScenarioLobbyDefinitions {
        &self.definitions
    }

    /// Values explicitly owned by an embedded Parameters.txt. `None` means
    /// C4GameParameters starts from Scenario.txt and may still consult user
    /// configuration (notably for an unforced fair-crew choice).
    pub fn embedded_game_parameters(&self) -> Option<&ScenarioGameParameterOverrides> {
        self.embedded_game_parameters.as_ref()
    }

    /// C4GameParameters values after applying compiler defaults but before
    /// runtime configuration, league enforcement, savegame restore-player
    /// floors, or network-reference replacement.
    pub fn game_parameter_defaults(&self) -> &ScenarioGameParameterValues {
        &self.game_parameter_defaults
    }

    /// Whether the compiler-default view can be merged with Parameters.txt
    /// without consulting runtime state. Even `EmbeddedParametersFile` is a
    /// pre-runtime view: league, restore-player and network-reference changes
    /// remain outside Scenario.
    pub fn game_parameter_resolution(&self) -> ScenarioGameParameterResolution {
        if self.embedded_game_parameters.is_some() {
            ScenarioGameParameterResolution::EmbeddedFileBeforeRuntimeAdjustments
        } else {
            ScenarioGameParameterResolution::RequiresRuntimeConfiguration
        }
    }

    /// Fully defaulted Parameters.txt compiler result, when such a file is
    /// present. Runtime-owned adjustments described by
    /// `game_parameter_resolution` have not been applied.
    pub fn embedded_game_parameter_values(&self) -> Option<ScenarioGameParameterValues> {
        self.embedded_game_parameters
            .as_ref()
            .map(|overrides| overrides.apply_to(&self.game_parameter_defaults))
    }

    /// Teams.txt state, or the precisely derived no-file default.
    pub fn teams(&self) -> &ScenarioLobbyTeams {
        &self.teams
    }
}

/// Scenario.txt `[Head]` inputs used by scenario selection and lobby startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyHead {
    configured_min_players: i32,
    effective_min_players: i32,
    max_players: i32,
    max_players_league: i32,
    save_game: bool,
    replay: bool,
    loader: ScenarioLoaderMetadata,
    fair_crew_force: ScenarioFairCrewForce,
    fair_crew_strength: i32,
    random_seed: i32,
    network_game: bool,
    network_runtime_join: bool,
}

impl ScenarioLobbyHead {
    /// Raw `MinPlayer`; zero asks C4Scenario::GetMinPlayer to derive 1 or 2.
    pub fn configured_min_players(&self) -> i32 {
        self.configured_min_players
    }

    /// C4Scenario::GetMinPlayer result after legacy goal conversion.
    pub fn min_players(&self) -> i32 {
        self.effective_min_players
    }

    /// Scenario default for C4GameParameters::MaxPlayers. Parameters.txt or
    /// league enforcement may replace it later.
    pub fn max_players(&self) -> i32 {
        self.max_players
    }

    /// Scenario default applied when league rules are enforced.
    pub fn max_players_league(&self) -> i32 {
        self.max_players_league
    }

    pub fn is_save_game(&self) -> bool {
        self.save_game
    }

    pub fn is_replay(&self) -> bool {
        self.replay
    }

    pub fn loader(&self) -> &ScenarioLoaderMetadata {
        &self.loader
    }

    pub fn fair_crew_force(&self) -> ScenarioFairCrewForce {
        self.fair_crew_force
    }

    pub fn fair_crew_strength(&self) -> i32 {
        self.fair_crew_strength
    }

    pub fn random_seed(&self) -> i32 {
        self.random_seed
    }

    pub fn was_network_game(&self) -> bool {
        self.network_game
    }

    pub fn allows_network_runtime_join(&self) -> bool {
        self.network_runtime_join
    }
}

/// The loader pattern is retained, but image selection remains deferred until
/// the complete resource group set and SafeRandom stream exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLoaderMetadata {
    configured_specification: String,
}

/// Scenario-head inputs C4Game has established immediately before
/// `C4GraphicsSystem::InitLoaderScreen` runs.
///
/// This deliberately reuses the full StdCompiler-faithful Scenario.txt
/// parser.  The app must choose the loader before the much more expensive
/// definition load completes, and a second lightweight INI parser would get
/// `RCT_All`, exact-name and DefaultAdapt behavior wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLoaderHead {
    loader: ScenarioLoaderMetadata,
    font: String,
    mission_access: String,
    origin: Option<String>,
    definition_modules: Vec<String>,
    definition_module_spellings: Vec<String>,
    local_only: bool,
    effective_min_players: i32,
    max_players: i32,
    save_game: bool,
    replay: bool,
    savegame_definition_override: ScenarioSavegameDefinitionOverride,
    /// Unicode presentation text decoded from the native scenario component.
    scenario_title: String,
    /// Exact native bytes copied by `C4ComponentHost::GetLanguageString` into
    /// `Game.Parameters.ScenarioTitle`.
    scenario_title_native: LegacyCString,
}

impl ScenarioLoaderHead {
    pub fn load_from_group(group: &Group) -> Result<Self, ScenarioError> {
        Self::load_from_group_with_languages(group, &["US"])
    }

    /// Parses the Scenario.txt fields that control resource-group
    /// registration without resolving or validating the presentation title.
    /// The returned head deliberately has an empty `scenario_title`; callers
    /// must use this only for Origin/Definitions/Extra resource setup.
    pub fn load_from_group_for_resource_registration(group: &Group) -> Result<Self, ScenarioError> {
        let manifest = parse_legacy_scenario_manifest(group)?;
        let savegame_definition_override =
            load_savegame_definition_override(group, manifest.core.head.save_game != 0)?;
        Ok(Self::from_manifest(
            manifest,
            savegame_definition_override,
            String::new(),
            LegacyCString::default(),
        ))
    }

    pub fn load_from_group_with_languages<S: AsRef<str>>(
        group: &Group,
        languages: &[S],
    ) -> Result<Self, ScenarioError> {
        Self::load_from_group_with_languages_and_packs(group, languages, &LanguagePacks::default())
    }

    pub fn load_from_group_with_languages_and_packs<S: AsRef<str>>(
        group: &Group,
        languages: &[S],
        language_packs: &LanguagePacks,
    ) -> Result<Self, ScenarioError> {
        let manifest = parse_legacy_scenario_manifest(group)?;
        let savegame_definition_override =
            load_savegame_definition_override(group, manifest.core.head.save_game != 0)?;
        let components = language_packs.component_groups(
            group,
            Some(group),
            manifest.core.head.origin.as_deref(),
        );
        let (scenario_title, scenario_title_native) =
            match load_loader_scenario_title(&components, languages)? {
                Some(title) => title,
                None => {
                    if let Some(native) = manifest.head_title_native.as_ref() {
                        let native = validate_name_ex_no_empty_bytes(native.as_bytes());
                        (decode_legacy_script_text(native.as_bytes()), native)
                    } else {
                        let title = validate_name_ex_no_empty(manifest.core.head.title.clone())?;
                        let native = LegacyCString::from_bytes(title.as_bytes().to_vec())
                            .expect("a parsed Scenario.txt title contains no NUL");
                        (title, native)
                    }
                }
            };
        Ok(Self::from_manifest(
            manifest,
            savegame_definition_override,
            scenario_title,
            scenario_title_native,
        ))
    }

    fn from_manifest(
        manifest: LegacyScenarioManifest,
        savegame_definition_override: ScenarioSavegameDefinitionOverride,
        scenario_title: String,
        scenario_title_native: LegacyCString,
    ) -> Self {
        let definition_module_spellings = manifest
            .core
            .definitions
            .reflected_definitions
            .clone()
            .filter(|spellings| spellings.len() == manifest.definition_specs.len())
            .unwrap_or_else(|| manifest.definition_specs.clone());
        Self {
            loader: ScenarioLoaderMetadata {
                configured_specification: manifest.core.head.loader.clone(),
            },
            font: manifest.core.head.font.clone(),
            mission_access: manifest.core.head.mission_access.clone(),
            origin: manifest.core.head.origin.clone(),
            definition_modules: manifest.definition_specs,
            definition_module_spellings,
            local_only: manifest.core.definitions.local_only,
            effective_min_players: legacy_effective_min_players(&manifest.core),
            max_players: manifest.core.head.max_player,
            save_game: manifest.core.head.save_game != 0,
            replay: manifest.core.head.replay != 0,
            savegame_definition_override,
            scenario_title,
            scenario_title_native,
        }
    }

    pub fn loader(&self) -> &ScenarioLoaderMetadata {
        &self.loader
    }

    /// Empty means `Config.General.RXFontName`, exactly as in C4Game.
    pub fn font(&self) -> &str {
        &self.font
    }

    /// Raw `Head.MissionAccess` after the scenario compiler's RCT_All
    /// adaptation. Empty means the scenario is not access-gated.
    pub fn mission_access(&self) -> &str {
        &self.mission_access
    }

    /// Raw Scenario.txt Origin after StdCompiler string adaptation.
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub fn configured_definition_modules(&self) -> &[String] {
        &self.definition_modules
    }

    pub fn configured_definition_module_spellings(&self) -> &[String] {
        &self.definition_module_spellings
    }

    pub fn local_only(&self) -> bool {
        self.local_only
    }

    /// `C4Scenario::GetMinPlayer`, cached by the startup scenario entry.
    pub fn min_players(&self) -> i32 {
        self.effective_min_players
    }

    pub fn max_players(&self) -> i32 {
        self.max_players
    }

    pub fn is_save_game(&self) -> bool {
        self.save_game
    }

    pub fn is_replay(&self) -> bool {
        self.replay
    }

    pub fn savegame_definition_override(&self) -> &ScenarioSavegameDefinitionOverride {
        &self.savegame_definition_override
    }

    /// Effective `Game.Parameters.ScenarioTitle` at loader initialization.
    pub fn scenario_title(&self) -> &str {
        &self.scenario_title
    }

    /// Exact native bytes used by C++ for synchronized parameters and the
    /// initial network save. This remains byte-preserving even when
    /// `scenario_title()` decodes legacy CP1252 for presentation.
    pub fn scenario_title_bytes(&self) -> &[u8] {
        self.scenario_title_native.as_bytes()
    }
}

impl ScenarioLoaderMetadata {
    /// Raw Scenario.txt `Loader`, including the meaningful empty default.
    pub fn configured_specification(&self) -> &str {
        &self.configured_specification
    }

    /// Pattern passed to the resource search; an empty configured value means
    /// `Loader*` in C4LoaderScreen::Init.
    pub fn effective_specification(&self) -> &str {
        if self.configured_specification.is_empty() {
            "Loader*"
        } else {
            &self.configured_specification
        }
    }

    pub fn selection(&self) -> ScenarioLoaderSelection {
        ScenarioLoaderSelection::DeferredResourceSearch
    }
}

/// Loader selection cannot be completed from Scenario.txt alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioLoaderSelection {
    /// C4LoaderScreen searches the ordered group set and chooses through the
    /// runtime random stream; no concrete image has been guessed.
    DeferredResourceSearch,
}

/// Raw `ForcedNoCrew` semantics. Unknown values are retained rather than
/// silently coerced to one of the three defined C4SForceFairCrew values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioFairCrewForce {
    Free,
    FairCrew,
    NormalCrew,
    Unknown(i32),
}

impl ScenarioFairCrewForce {
    fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Free,
            1 => Self::FairCrew,
            2 => Self::NormalCrew,
            value => Self::Unknown(value),
        }
    }

    pub fn raw_value(self) -> i32 {
        match self {
            Self::Free => 0,
            Self::FairCrew => 1,
            Self::NormalCrew => 2,
            Self::Unknown(value) => value,
        }
    }

    /// The forced UseFairCrew value, or `None` when user configuration owns
    /// the initial choice.
    pub fn forced_use_fair_crew(self) -> Option<bool> {
        match self {
            Self::Free => None,
            Self::FairCrew => Some(true),
            Self::NormalCrew => Some(false),
            // C4GameParameters compares exactly against FairCrew for the
            // value, but treats every nonzero raw value as forced.
            Self::Unknown(value) => (value != 0).then_some(false),
        }
    }

    pub fn is_forced(self) -> bool {
        self.raw_value() != 0
    }
}

/// Which definition vector supplied the Scenario.txt phase, before an old
/// savegame may apply its Game.txt compatibility override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioDefinitionSelectionSource {
    ScenarioPreset,
    CallerDefaults,
    FixedCallerSelection,
}

#[derive(Debug, Clone, Copy)]
enum DefinitionPathExpansion<'a> {
    DirectoryRoot(&'a Path),
    LiteralPrefix(&'a Path),
}

/// Scenario definition settings and the immutable effective selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyDefinitions {
    local_only: bool,
    allow_user_change: bool,
    configured_modules: Vec<String>,
    configured_module_spellings: Vec<String>,
    requested_modules: Vec<String>,
    requested_module_spellings: Vec<String>,
    selection_source: ScenarioDefinitionSelectionSource,
    definition_root_applied: bool,
    resolved_load_resources: Vec<PathBuf>,
    savegame_override: ScenarioSavegameDefinitionOverride,
}

impl ScenarioLobbyDefinitions {
    pub fn is_local_only(&self) -> bool {
        self.local_only
    }

    pub fn allows_user_change(&self) -> bool {
        self.allow_user_change
    }

    /// Raw ordered modules compiled from `[Definitions]`.
    pub fn configured_modules(&self) -> &[String] {
        &self.configured_modules
    }

    /// Exact native strings compiled from `[Definitions]`, before the Rust
    /// resolver normalizes separators or redundant relative components.
    pub fn configured_module_spellings(&self) -> &[String] {
        &self.configured_module_spellings
    }

    /// Ordered external modules selected from Scenario.txt/caller state,
    /// before DefinitionPath expansion or an old-save Game.txt override.
    pub fn requested_modules(&self) -> &[String] {
        &self.requested_modules
    }

    /// Exact native strings selected for this round before `DefinitionPath`
    /// is concatenated. C++ retains redundant relative components here.
    pub fn requested_module_spellings(&self) -> &[String] {
        &self.requested_module_spellings
    }

    pub fn selection_source(&self) -> ScenarioDefinitionSelectionSource {
        self.selection_source
    }

    pub fn definition_root_applied(&self) -> bool {
        self.definition_root_applied
    }

    /// External and folder-local resources resolved before the old-save
    /// Game.txt compatibility override. The scenario group is absent.
    pub fn load_resources_before_savegame_override(&self) -> &[PathBuf] {
        &self.resolved_load_resources
    }

    pub fn resolved_load_resources(&self) -> Option<&[PathBuf]> {
        matches!(
            &self.savegame_override,
            ScenarioSavegameDefinitionOverride::None
        )
        .then_some(self.resolved_load_resources.as_slice())
    }

    /// The load vector is final only when this is `None`. Old exact saves may
    /// replace it from Game.txt after Scenario.txt has been compiled.
    pub fn savegame_override(&self) -> &ScenarioSavegameDefinitionOverride {
        &self.savegame_override
    }

    pub fn effective_modules(&self) -> Option<&[String]> {
        matches!(
            &self.savegame_override,
            ScenarioSavegameDefinitionOverride::None
        )
        .then_some(self.requested_modules.as_slice())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioSavegameDefinitionOverride {
    None,
    /// Game.txt contains the legacy `[DefinitionFiles]` compatibility block.
    /// The C++ loader mutates the definition vector through an ad-hoc textual
    /// parser after opening the scenario; retain the lines and do not label
    /// the pre-savegame selection effective.
    GameText {
        definition_lines: Vec<String>,
    },
}

impl ScenarioSavegameDefinitionOverride {
    pub fn definition_lines(&self) -> Option<&[String]> {
        match self {
            Self::None => None,
            Self::GameText { definition_lines } => Some(definition_lines),
        }
    }
}

#[derive(Debug)]
struct LoadedLegacyTeamMetadata {
    metadata: InitialNetworkTeamMetadata,
    random_color_team_id: Option<i32>,
    unsupported_team_distribution: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioGameParameterResolution {
    EmbeddedFileBeforeRuntimeAdjustments,
    RequiresRuntimeConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyIdEntry {
    id: String,
    count: i32,
}

impl ScenarioLobbyIdEntry {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn count(&self) -> i32 {
        self.count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyClient {
    id: i32,
    activated: bool,
    observer: bool,
    name: String,
    nick: String,
    lobby_ready: bool,
}

impl ScenarioLobbyClient {
    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn is_activated(&self) -> bool {
        self.activated
    }

    pub fn is_observer(&self) -> bool {
        self.observer
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn nick(&self) -> &str {
        &self.nick
    }

    pub fn is_lobby_ready(&self) -> bool {
        self.lobby_ready
    }
}

/// Concrete compiler defaults. These are not final lobby values until the
/// runtime-owned configuration, league, savegame and network boundaries have
/// been applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioGameParameterValues {
    random_seed: i32,
    startup_player_count: i32,
    max_players: i32,
    use_fair_crew: bool,
    fair_crew_forced: bool,
    fair_crew_strength: i32,
    allow_debug: bool,
    is_network_game: bool,
    control_rate: i32,
    auto_frame_skip: bool,
    rules: Vec<ScenarioLobbyIdEntry>,
    goals: Vec<ScenarioLobbyIdEntry>,
    league: String,
    clients: Vec<ScenarioLobbyClient>,
}

impl ScenarioGameParameterValues {
    pub fn random_seed(&self) -> i32 {
        self.random_seed
    }

    pub fn startup_player_count(&self) -> i32 {
        self.startup_player_count
    }

    pub fn max_players(&self) -> i32 {
        self.max_players
    }

    pub fn use_fair_crew(&self) -> bool {
        self.use_fair_crew
    }

    pub fn fair_crew_forced(&self) -> bool {
        self.fair_crew_forced
    }

    pub fn fair_crew_strength(&self) -> i32 {
        self.fair_crew_strength
    }

    pub fn allow_debug(&self) -> bool {
        self.allow_debug
    }

    pub fn is_network_game(&self) -> bool {
        self.is_network_game
    }

    pub fn control_rate(&self) -> i32 {
        self.control_rate
    }

    pub fn auto_frame_skip(&self) -> bool {
        self.auto_frame_skip
    }

    pub fn rules(&self) -> &[ScenarioLobbyIdEntry] {
        &self.rules
    }

    pub fn goals(&self) -> &[ScenarioLobbyIdEntry] {
        &self.goals
    }

    pub fn league(&self) -> &str {
        &self.league
    }

    pub fn clients(&self) -> &[ScenarioLobbyClient] {
        &self.clients
    }
}

/// Fields which Parameters.txt can own instead of Scenario.txt defaults.
/// Each `Option` preserves whether the value was actually present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioGameParameterOverrides {
    random_seed: Option<i32>,
    max_players: Option<i32>,
    startup_player_count: Option<i32>,
    use_fair_crew: Option<bool>,
    fair_crew_forced: Option<bool>,
    fair_crew_strength: Option<i32>,
    allow_debug: Option<bool>,
    is_network_game: Option<bool>,
    control_rate: Option<i32>,
    auto_frame_skip: Option<bool>,
    rules: Option<Vec<ScenarioLobbyIdEntry>>,
    goals: Option<Vec<ScenarioLobbyIdEntry>>,
    league: Option<String>,
    clients: Vec<ScenarioLobbyClient>,
}

impl ScenarioGameParameterOverrides {
    pub fn random_seed(&self) -> Option<i32> {
        self.random_seed
    }

    pub fn max_players(&self) -> Option<i32> {
        self.max_players
    }

    pub fn startup_player_count(&self) -> Option<i32> {
        self.startup_player_count
    }

    pub fn use_fair_crew(&self) -> Option<bool> {
        self.use_fair_crew
    }

    pub fn fair_crew_forced(&self) -> Option<bool> {
        self.fair_crew_forced
    }

    pub fn fair_crew_strength(&self) -> Option<i32> {
        self.fair_crew_strength
    }

    pub fn allow_debug(&self) -> Option<bool> {
        self.allow_debug
    }

    pub fn is_network_game(&self) -> Option<bool> {
        self.is_network_game
    }

    pub fn control_rate(&self) -> Option<i32> {
        self.control_rate
    }

    pub fn auto_frame_skip(&self) -> Option<bool> {
        self.auto_frame_skip
    }

    pub fn rules(&self) -> Option<&[ScenarioLobbyIdEntry]> {
        self.rules.as_deref()
    }

    pub fn goals(&self) -> Option<&[ScenarioLobbyIdEntry]> {
        self.goals.as_deref()
    }

    pub fn league(&self) -> Option<&str> {
        self.league.as_deref()
    }

    pub fn clients(&self) -> &[ScenarioLobbyClient] {
        &self.clients
    }

    fn apply_to(&self, defaults: &ScenarioGameParameterValues) -> ScenarioGameParameterValues {
        ScenarioGameParameterValues {
            random_seed: self.random_seed.unwrap_or(defaults.random_seed),
            startup_player_count: self
                .startup_player_count
                .unwrap_or(defaults.startup_player_count),
            max_players: self.max_players.unwrap_or(defaults.max_players),
            use_fair_crew: self.use_fair_crew.unwrap_or(defaults.use_fair_crew),
            fair_crew_forced: self.fair_crew_forced.unwrap_or(defaults.fair_crew_forced),
            fair_crew_strength: self
                .fair_crew_strength
                .unwrap_or(defaults.fair_crew_strength),
            allow_debug: self.allow_debug.unwrap_or(defaults.allow_debug),
            is_network_game: self.is_network_game.unwrap_or(defaults.is_network_game),
            control_rate: self.control_rate.unwrap_or(defaults.control_rate),
            auto_frame_skip: self.auto_frame_skip.unwrap_or(defaults.auto_frame_skip),
            rules: self.rules.clone().unwrap_or_else(|| defaults.rules.clone()),
            goals: self.goals.clone().unwrap_or_else(|| defaults.goals.clone()),
            league: self
                .league
                .clone()
                .unwrap_or_else(|| defaults.league.clone()),
            clients: self.clients.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioTeamsSource {
    TeamsFile,
    DerivedScenarioDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioTeamDistribution {
    Free,
    Host,
    None,
    Random,
    RandomInvisible,
    Numeric(u8),
}

/// How C4Team::RecheckColor obtains the presentation color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioTeamColor {
    Explicit(u32),
    DefaultForId(u32),
    /// Team IDs outside the fixed palette consume SafeRandom and therefore
    /// cannot be resolved from the scenario in isolation.
    DeferredRuntimeRandom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyTeam {
    id: i32,
    name: String,
    player_start_index: i32,
    player_count: i32,
    players: Vec<i32>,
    configured_color: u32,
    icon_spec: Option<String>,
    max_players: i32,
}

impl ScenarioLobbyTeam {
    pub fn id(&self) -> i32 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn player_start_index(&self) -> i32 {
        self.player_start_index
    }

    pub fn player_count(&self) -> i32 {
        self.player_count
    }

    pub fn players(&self) -> &[i32] {
        &self.players
    }

    /// Raw Teams.txt color. Zero invokes C4Team::RecheckColor.
    pub fn configured_color(&self) -> u32 {
        self.configured_color
    }

    pub fn color(&self) -> ScenarioTeamColor {
        if self.configured_color != 0 {
            return ScenarioTeamColor::Explicit(self.configured_color);
        }
        const DEFAULT_COLORS: [u32; 10] = [
            0xF4_00_00, 0x00_C8_00, 0xFC_F4_1C, 0x20_20_FF, 0xC4_84_44, 0xFF_FF_FF, 0x84_84_84,
            0xFF_00_EF, 0x00_FF_FF, 0x78_48_30,
        ];
        if (1..=10).contains(&self.id) {
            ScenarioTeamColor::DefaultForId(DEFAULT_COLORS[(self.id - 1) as usize])
        } else {
            ScenarioTeamColor::DeferredRuntimeRandom
        }
    }

    pub fn icon_spec(&self) -> Option<&str> {
        self.icon_spec.as_deref()
    }

    /// Raw `MaxPlayer`; zero is the C++ unlimited sentinel.
    pub fn configured_max_players(&self) -> i32 {
        self.max_players
    }

    pub fn max_players(&self) -> Option<i32> {
        (self.max_players != 0).then_some(self.max_players)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyTeams {
    source: ScenarioTeamsSource,
    active: bool,
    custom: bool,
    allow_hostility_change: bool,
    allow_team_switch: bool,
    configured_auto_generate: bool,
    auto_generate: bool,
    configured_last_team_id: i32,
    last_team_id: i32,
    distribution: ScenarioTeamDistribution,
    team_colors: bool,
    max_script_players: i32,
    script_player_names: String,
    random_team_count: i32,
    teams: Vec<ScenarioLobbyTeam>,
}

impl ScenarioLobbyTeams {
    pub fn source(&self) -> ScenarioTeamsSource {
        self.source
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn is_custom(&self) -> bool {
        self.custom
    }

    pub fn allows_hostility_change(&self) -> bool {
        self.allow_hostility_change
    }

    pub fn allows_team_switch(&self) -> bool {
        self.allow_team_switch
    }

    pub fn auto_generates_teams(&self) -> bool {
        self.auto_generate
    }

    pub fn configured_auto_generate(&self) -> bool {
        self.configured_auto_generate
    }

    pub fn configured_last_team_id(&self) -> i32 {
        self.configured_last_team_id
    }

    pub fn last_team_id(&self) -> i32 {
        self.last_team_id
    }

    pub fn distribution(&self) -> ScenarioTeamDistribution {
        self.distribution
    }

    pub fn uses_team_colors(&self) -> bool {
        self.team_colors
    }

    pub fn max_script_players(&self) -> i32 {
        self.max_script_players
    }

    /// Raw `|`-separated script player name list. An empty value delegates to
    /// the runtime-localized "Computer" label.
    pub fn script_player_names(&self) -> &str {
        &self.script_player_names
    }

    pub fn random_team_count(&self) -> i32 {
        self.random_team_count
    }

    pub fn teams(&self) -> &[ScenarioLobbyTeam] {
        &self.teams
    }
}

/// The C4Game::InitGame environment-placement inputs (C4Game.cpp:
/// 2493-2503): Scenario.txt `[Landscape] Vegetation=/InEarth=`,
/// `[Animals]`, `[Environment] Objects=` and `[Game] Goals=/Rules=`,
/// plus the NoInitialize gate and the MEarth material name.
#[derive(Debug, Clone, Default)]
pub(crate) struct LegacyInitPlacement {
    pub save_game: bool,
    pub no_initialize: bool,
    pub vegetation: Vec<(String, i32)>,
    pub vegetation_level: LegacyC4SVal,
    pub in_earth: Vec<(String, i32)>,
    pub in_earth_level: LegacyC4SVal,
    pub animals: Vec<(String, i32)>,
    pub nests: Vec<(String, i32)>,
    pub environment: Vec<(String, i32)>,
    pub goals: Vec<(String, i32)>,
    pub rules: Vec<(String, i32)>,
    pub earth_material: String,
}

impl Default for LegacyC4SVal {
    fn default() -> Self {
        Self::new(0, 0, 0, 100)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScenarioObjectives {
    #[doc(hidden)]
    pub create_objects: Vec<CreateObjectObjective>,
    #[doc(hidden)]
    pub clear_objects: Vec<ClearObjectObjective>,
    #[doc(hidden)]
    pub clear_materials: Vec<ClearMaterialObjective>,
}

#[derive(Debug, Clone)]
pub struct CreateObjectObjective {
    #[doc(hidden)]
    pub definition: String,
    #[doc(hidden)]
    pub count: i32,
}

#[derive(Debug, Clone)]
pub struct ClearObjectObjective {
    #[doc(hidden)]
    pub definition: String,
    #[doc(hidden)]
    pub count: i32,
}

#[derive(Debug, Clone)]
pub struct ClearMaterialObjective {
    pub(crate) material: String,
    pub(crate) count: i32,
}

impl ScenarioObjectives {
    fn from_legacy_game(game: &LegacyGame) -> Self {
        let mut objectives = ScenarioObjectives::default();

        for entry in &game.create_objects {
            let count = entry.count.unwrap_or(0);
            if count <= 0 {
                continue;
            }
            objectives.create_objects.push(CreateObjectObjective {
                definition: entry.id.clone(),
                count,
            });
        }

        for entry in &game.clear_objects {
            let count = entry.count.unwrap_or(0);
            objectives.clear_objects.push(ClearObjectObjective {
                definition: entry.id.clone(),
                count,
            });
        }

        for entry in &game.clear_materials {
            let count = entry.count.unwrap_or(0);
            objectives.clear_materials.push(ClearMaterialObjective {
                material: entry.name.clone(),
                count,
            });
        }

        objectives
    }

    pub fn is_empty(&self) -> bool {
        self.create_objects.is_empty()
            && self.clear_objects.is_empty()
            && self.clear_materials.is_empty()
    }
}

pub trait LegacyDefinitionResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError>;

    /// External language packs installed beside the executable data. The
    /// default keeps embedders and synchronized network resource resolvers
    /// independent from machine-local localization packs.
    fn resolve_language_packs(&self, _scenario: &Group) -> Result<LanguagePacks, ScenarioError> {
        Ok(LanguagePacks::default())
    }

    /// Ordered external `NRT_Material` sources. Material overloads walk a
    /// resource chain and therefore must not inherit the one-group semantics
    /// of an explicit `DefinitionFilenames` vector entry.
    fn resolve_material_groups(&self, scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
        match self.resolve_definition_groups(scenario, "Material.c4g") {
            Ok(groups) => Ok(groups),
            Err(ScenarioError::LegacyDefinitionNotFound { .. }) => Ok(Vec::new()),
            Err(ScenarioError::Resources(
                GroupError::Missing(_) | GroupError::EntryNotFound(_) | GroupError::NotDirectory(_),
            )) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// Ordered external graphics resources used by C4Sky's SkyDef fallback.
    /// A missing Graphics.c4g is an ordinary no-surface result, just like a
    /// missing Material.c4g is an ordinary empty material overload chain.
    fn resolve_graphics_groups(&self, scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
        match self.resolve_definition_groups(scenario, "Graphics.c4g") {
            Ok(groups) => Ok(groups),
            Err(ScenarioError::LegacyDefinitionNotFound { .. }) => Ok(Vec::new()),
            Err(ScenarioError::Resources(
                GroupError::Missing(_) | GroupError::EntryNotFound(_) | GroupError::NotDirectory(_),
            )) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// Ordered graphics lookup chain after the exact external definition-pack
    /// roots have been registered in the game group set. Implementations that
    /// do not model that group set retain their existing graphics chain.
    fn resolve_graphics_groups_with_definition_roots(
        &self,
        scenario: &Group,
        _definition_roots: &[Group],
    ) -> Result<Vec<Group>, ScenarioError> {
        self.resolve_graphics_groups(scenario)
    }
}

struct AuthoritativeNetworkResourceResolver<'a> {
    definition_modules: &'a [String],
    definition_groups: &'a [Group],
    material_groups: &'a [Group],
    graphics_groups: &'a [Group],
    language_packs: &'a LanguagePacks,
}

impl LegacyDefinitionResolver for AuthoritativeNetworkResourceResolver<'_> {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        if identifier.eq_ignore_ascii_case("Material.c4g") {
            return Ok(self.material_groups.to_vec());
        }
        self.definition_modules
            .iter()
            .position(|module| module == identifier)
            .and_then(|index| self.definition_groups.get(index))
            .cloned()
            .map(|group| vec![group])
            .ok_or_else(|| ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            })
    }

    fn resolve_graphics_groups(&self, _scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
        Ok(self.graphics_groups.to_vec())
    }

    fn resolve_language_packs(&self, _scenario: &Group) -> Result<LanguagePacks, ScenarioError> {
        Ok(self.language_packs.clone())
    }
}

impl Scenario {
    /// Whether this scenario uses the shipped `SkyParcour` generator whose
    /// final sky overlays can expose its earlier Water fill.
    pub fn generated_landscape_seed_retry_applies(&self) -> bool {
        self.landscape.as_ref().is_some_and(|landscape| {
            landscape
                .raster_state()
                .and_then(LandscapeRasterState::map_creator)
                .is_some_and(|creator| creator.has_skyparcour_water_exposure_guard())
        })
    }

    /// Whether a fresh authoritative round should try the next random seed
    /// before publishing its generated landscape.
    ///
    /// The shipped HarpoonRace and Sky Race `SkyParcour` map draws Water
    /// inside an Earth overlay and then subtracts sky from the completed
    /// operator chain. Some seeds therefore expose an otherwise rectangular
    /// Water fill with an immediate C4MassMover path. The C++ creator produces
    /// the same invalid Surface8; restricting this workaround to that exact
    /// operator program leaves intentional waterfalls and third-party maps
    /// named `SkyParcour` untouched.
    ///
    /// Exact replay/save/network-client loads must retain their synchronized
    /// seed and should not use this fresh-round predicate.
    pub fn generated_landscape_requires_seed_retry(&self) -> bool {
        if !self.generated_landscape_seed_retry_applies() {
            return false;
        }
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let Some(grid) = landscape.pixel_grid() else {
            return false;
        };
        let (Ok(width), Ok(height)) = (i32::try_from(grid.width()), i32::try_from(grid.height()))
        else {
            return false;
        };
        if width <= 0 || height <= 1 {
            return false;
        }
        let Some(material_library) = self.material_library.as_ref() else {
            return false;
        };
        let materials = MaterialSet::from_resource_library(material_library);
        let Some(water) = materials.get("Water").filter(|water| water.instable()) else {
            return false;
        };
        let water_slots = grid
            .material_names()
            .iter()
            .map(|name| {
                name.as_deref()
                    .is_some_and(|name| clonk_resources::material::c4_names_equal(name, "Water"))
            })
            .collect::<Vec<_>>();
        let width = width as usize;
        let scanned_pixels = grid.bytes().len().saturating_sub(width);

        grid.bytes()
            .iter()
            .take(scanned_pixels)
            .enumerate()
            .any(|(index, &byte)| {
                if !water_slots
                    .get(usize::from(byte & 0x7f))
                    .copied()
                    .unwrap_or(false)
                {
                    return false;
                }
                let mut x = (index % width) as i32;
                let mut y = (index / width) as i32;
                landscape.find_mat_path(
                    &mut x,
                    &mut y,
                    1,
                    water.density(),
                    water.max_slide(),
                    &materials,
                )
            })
    }

    /// Reads the ordinary offline player-admission parameters without loading
    /// definitions, materials or landscape data.
    pub fn preflight_offline_startup_from_path(
        path: impl AsRef<Path>,
    ) -> Result<OfflineScenarioStartupPreflight, ScenarioError> {
        let group = Group::open(path)?;
        Self::preflight_offline_startup_from_group(&group)
    }

    /// Reads the ordinary offline player-admission parameters from an already
    /// opened scenario group. Replays retain their separate CtrlRec path;
    /// modern savegames are admitted so the application can stage their
    /// SavePlayerInfos/Game.txt restoration before landscape creation.
    pub fn preflight_offline_startup_from_group(
        group: &Group,
    ) -> Result<OfflineScenarioStartupPreflight, ScenarioError> {
        if read_optional_legacy_entry(group, "Scenario.json")?.is_some() {
            return Err(ScenarioError::OfflineStartupJsonUnsupported);
        }

        let manifest = parse_legacy_scenario_manifest(group)?;
        let save_game = manifest.core.head.save_game != 0;
        if manifest.core.head.replay != 0 {
            return Err(ScenarioError::OfflineStartupReplayUnsupported);
        }
        if !save_game && read_optional_legacy_entry(group, "SavePlayerInfos.txt")?.is_some() {
            return Err(ScenarioError::OfflineStartupRestoreInfosUnsupported);
        }

        let (max_players, random_seed) = match read_optional_legacy_entry(group, "Parameters.txt")?
        {
            Some(parameters) => (
                parse_legacy_parameters_max_players(&parameters, manifest.core.head.max_player)?,
                Some(parse_legacy_parameters_random_seed(
                    &parameters,
                    manifest.core.head.random_seed,
                )?),
            ),
            None => (manifest.core.head.max_player, None),
        };
        Ok(OfflineScenarioStartupPreflight {
            max_players,
            random_seed,
            save_game,
        })
    }

    /// Reads replay-owned map inputs before definitions or landscape state
    /// are loaded. Non-replay scenarios return `None`.
    pub fn preflight_replay_startup_from_group(
        group: &Group,
    ) -> Result<Option<ReplayScenarioStartupPreflight>, ScenarioError> {
        let manifest = parse_legacy_scenario_manifest(group)?;
        if manifest.core.head.replay == 0 {
            return Ok(None);
        }
        let defaults = game_parameter_defaults(&manifest.core);
        let parameters = load_legacy_game_parameter_overrides(group, &defaults)?
            .map(|overrides| overrides.apply_to(&defaults))
            .unwrap_or(defaults);
        let startup_player_count =
            replay_startup_player_count_from_group(group, parameters.startup_player_count)?;
        Ok(Some(ReplayScenarioStartupPreflight {
            random_seed: parameters.random_seed,
            startup_player_count,
        }))
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ScenarioError> {
        let group = Group::open(path)?;
        Self::load_from_group(&group)
    }

    pub fn load_from_path_with<R: LegacyDefinitionResolver>(
        path: impl AsRef<Path>,
        resolver: &R,
    ) -> Result<Self, ScenarioError> {
        Self::load_from_path_with_seed(path, resolver, 0)
    }

    pub fn load_from_path_with_seed<R: LegacyDefinitionResolver>(
        path: impl AsRef<Path>,
        resolver: &R,
        random_seed: u64,
    ) -> Result<Self, ScenarioError> {
        Self::load_from_path_with_languages_and_seed(path, resolver, &["US", "DE"], random_seed)
    }

    pub fn load_from_path_with_languages<R, S>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        Self::load_from_path_with_languages_and_seed(path, resolver, languages, 0)
    }

    /// Loads a non-fixed legacy scenario with the caller's initial definition
    /// list. A non-empty, non-`LocalOnly` scenario preset replaces this seed;
    /// `LocalOnly` or an empty preset retains it. This models the startup path
    /// that seeds `Objects.c4d` before `C4Game::OpenScenario`.
    pub fn load_from_path_with_languages_and_definition_seed<R, S, M>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        initial_modules: &[M],
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
    {
        let initial_modules = initial_modules
            .iter()
            .map(|module| normalize_definition_path(module.as_ref()))
            .collect::<Vec<_>>();
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed_and_definition_modules(
            &group,
            resolver,
            languages,
            0,
            &initial_modules,
            None,
            None,
        )
    }

    /// Loads a non-fixed legacy scenario with the caller's initial
    /// definition list and applies C4Game's `DefinitionPath` transformation
    /// to the final selected vector. The rooted copies are loaded first in
    /// vector order, followed by the original copies in vector order. Every
    /// copy is required; rooted copies are resolved strictly below
    /// `definition_root` without substituting a search-root match.
    pub fn load_from_path_with_languages_and_definition_seed_in_root<R, S, M, P>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        initial_modules: &[M],
        definition_root: P,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
        P: AsRef<Path>,
    {
        let initial_modules = initial_modules
            .iter()
            .map(|module| normalize_definition_path(module.as_ref()))
            .collect::<Vec<_>>();
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed_and_definition_modules(
            &group,
            resolver,
            languages,
            0,
            &initial_modules,
            None,
            Some(definition_root.as_ref()),
        )
    }

    /// Loads a legacy scenario with the exact external definition-module
    /// vector chosen by `C4DefinitionSelDlg`. This is the `FixedDefinitions`
    /// branch of `C4Game::Init` and therefore replaces, rather than extends,
    /// the scenario's `[Definitions]` preset. Folder/scenario-local
    /// definitions and `SkipDefs` retain their normal behavior.
    pub fn load_from_path_with_languages_and_definition_modules<R, S, M>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        modules: &[M],
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
    {
        let modules = modules
            .iter()
            .map(|module| normalize_definition_path(module.as_ref()))
            .collect::<Vec<_>>();
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed_and_definition_modules(
            &group,
            resolver,
            languages,
            0,
            &[],
            Some(&modules),
            None,
        )
    }

    /// Loads a legacy scenario with a fixed definition-module vector and
    /// applies C4Game's `DefinitionPath` transformation: the complete rooted
    /// block is loaded first, followed by the complete original block. Every
    /// expanded vector entry must resolve to exactly one group. Rooted entries
    /// are opened strictly below `definition_root`; they never fall back to a
    /// normal search root.
    pub fn load_from_path_with_languages_and_definition_modules_in_root<R, S, M, P>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        modules: &[M],
        definition_root: P,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
        P: AsRef<Path>,
    {
        let modules = modules
            .iter()
            .map(|module| normalize_definition_path(module.as_ref()))
            .collect::<Vec<_>>();
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed_and_definition_modules(
            &group,
            resolver,
            languages,
            0,
            &[],
            Some(&modules),
            Some(definition_root.as_ref()),
        )
    }

    pub fn load_from_path_with_languages_and_seed<R, S>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed(&group, resolver, languages, random_seed)
    }

    /// Loads a scenario with the admitted startup-player count used by
    /// legacy dynamic landscapes. `MapPlayerExtend` reads this frozen count
    /// while creating the map (C4Game.cpp:2394-2431;
    /// C4Landscape.cpp:518-522; C4Scenario.cpp:327-334).
    pub fn load_from_path_with_languages_and_seed_and_startup_player_count<R, S>(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        startup_player_count: i32,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed_and_startup_player_count(
            &group,
            resolver,
            languages,
            random_seed,
            startup_player_count,
        )
    }

    /// Loads a legacy scenario with both the selected external definition
    /// vector and C4Game's frozen startup-player count. This is the combined
    /// startup seam used before dynamic landscape creation.
    #[allow(clippy::too_many_arguments)]
    pub fn load_from_path_with_languages_and_seed_and_definition_selection_and_startup_player_count<
        R,
        S,
        M,
    >(
        path: impl AsRef<Path>,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_root: Option<&Path>,
        startup_player_count: i32,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
    {
        let group = Group::open(path)?;
        Self::load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count(
            &group,
            resolver,
            languages,
            random_seed,
            initial_modules,
            fixed_modules,
            definition_root,
            startup_player_count,
        )
    }

    /// Loads a network client's combined scenario from the exact resolved
    /// `C4GameRes` groups. Definitions use the synchronized list rather than
    /// the scenario preset or local folder scan; materials retain C++'s
    /// scenario-local-first, then synchronized-list order. Graphics are the
    /// client's local C4GraphicsResource group chain and are not synchronized
    /// game resources (C4GraphicsResource.cpp:351-380;
    /// C4GameParameters.cpp:73-79,255-271; C4Game.cpp:80-101,876-952).
    pub fn load_network_from_path_with_languages_and_seed<S>(
        path: impl AsRef<Path>,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
    {
        Self::load_network_from_path_with_languages_and_seed_and_packs(
            path,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            &LanguagePacks::default(),
        )
    }

    /// Pack-aware network-client scenario loader. Language packs remain
    /// machine-local presentation resources, while definitions/materials
    /// continue to come exclusively from the synchronized authoritative
    /// vectors.
    #[allow(clippy::too_many_arguments)]
    pub fn load_network_from_path_with_languages_and_seed_and_packs<S>(
        path: impl AsRef<Path>,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
        language_packs: &LanguagePacks,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
    {
        let group = Group::open(path)?;
        Self::load_network_from_group_with_languages_and_seed_and_packs(
            &group,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            language_packs,
        )
    }

    /// Group-backed counterpart used when the synchronized scenario is a
    /// logical child of a packed parent, or when a host must re-apply the
    /// post-publication `C4GameRes` types before `InitDefs`.
    #[allow(clippy::too_many_arguments)]
    pub fn load_network_from_group_with_languages_and_seed_and_packs<S>(
        group: &Group,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
        language_packs: &LanguagePacks,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
    {
        let languages = languages.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        let definition_modules = (0..definition_groups.len())
            .map(|index| format!("__NetworkDefinition{index}.c4d"))
            .collect::<Vec<_>>();
        let resolver = AuthoritativeNetworkResourceResolver {
            definition_modules: &definition_modules,
            definition_groups,
            material_groups,
            graphics_groups,
            language_packs,
        };
        let mut ignore_progress = |_: i32, _: &'static str| {};
        Self::load_from_group_with_languages_and_seed_and_definition_modules_inner(
            group,
            &resolver,
            &languages,
            random_seed,
            &[],
            Some(&definition_modules),
            None,
            legacy_startup_player_count(),
            false,
            &mut ignore_progress,
        )
    }

    pub fn load_from_group_with<R: LegacyDefinitionResolver>(
        group: &Group,
        resolver: &R,
    ) -> Result<Self, ScenarioError> {
        Self::load_from_group_with_seed(group, resolver, 0)
    }

    pub fn load_from_group_with_seed<R: LegacyDefinitionResolver>(
        group: &Group,
        resolver: &R,
        random_seed: u64,
    ) -> Result<Self, ScenarioError> {
        Self::load_from_group_with_languages_and_seed(group, resolver, &["US", "DE"], random_seed)
    }

    pub fn load_from_group_with_languages<R, S>(
        group: &Group,
        resolver: &R,
        languages: &[S],
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_seed(group, resolver, languages, 0)
    }

    pub fn load_from_group_with_languages_and_seed<R, S>(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_seed_and_definition_modules(
            group,
            resolver,
            languages,
            random_seed,
            &[],
            None,
            None,
        )
    }

    /// Loads a scenario group with the admitted startup-player count used by
    /// legacy dynamic landscapes. JSON scenarios do not consume this value.
    pub fn load_from_group_with_languages_and_seed_and_startup_player_count<R, S>(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        startup_player_count: i32,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_seed_and_definition_modules_and_startup_player_count(
            group,
            resolver,
            languages,
            random_seed,
            &[],
            None,
            None,
            startup_player_count,
        )
    }

    /// Loads an already-opened scenario group with the startup definition
    /// selection. This is the group-backed counterpart of the path API for
    /// logical scenarios nested inside packed parent folders.
    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_definition_selection<R, S, M>(
        group: &Group,
        resolver: &R,
        languages: &[S],
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_root: Option<&Path>,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_definition_selection_and_progress(
            group,
            resolver,
            languages,
            initial_modules,
            fixed_modules,
            definition_root,
            |_, _| {},
        )
    }

    /// C++ `DefinitionPath` is a literal filename prefix, not a directory.
    /// This variant preserves exact caller spellings while prepending that
    /// prefix to the selected vector.
    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_definition_selection_and_prefix<R, S, M>(
        group: &Group,
        resolver: &R,
        languages: &[S],
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_prefix: Option<&Path>,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_definition_selection_and_prefix_and_progress(
            group,
            resolver,
            languages,
            initial_modules,
            fixed_modules,
            definition_prefix,
            |_, _| {},
        )
    }

    /// Loads an already-opened scenario group and reports coarse legacy-load
    /// milestones. JSON fixtures do not have C4Game loading milestones and
    /// therefore do not invoke the callback.
    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_definition_selection_and_progress<R, S, M, F>(
        group: &Group,
        resolver: &R,
        languages: &[S],
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_root: Option<&Path>,
        report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
        F: FnMut(i32, &'static str),
    {
        Self::load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_progress(
            group,
            resolver,
            languages,
            0,
            initial_modules,
            fixed_modules,
            definition_root,
            legacy_startup_player_count(),
            report_progress,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_definition_selection_and_prefix_and_progress<
        R,
        S,
        M,
        F,
    >(
        group: &Group,
        resolver: &R,
        languages: &[S],
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_prefix: Option<&Path>,
        report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
        F: FnMut(i32, &'static str),
    {
        Self::load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_prefix_and_progress(
            group,
            resolver,
            languages,
            0,
            initial_modules,
            fixed_modules,
            definition_prefix,
            legacy_startup_player_count(),
            report_progress,
        )
    }

    /// Loads an already-opened scenario group with both the selected
    /// definition vector and frozen startup-player count.
    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count<
        R,
        S,
        M,
    >(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_root: Option<&Path>,
        startup_player_count: i32,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_progress(
            group,
            resolver,
            languages,
            random_seed,
            initial_modules,
            fixed_modules,
            definition_root,
            startup_player_count,
            |_, _| {},
        )
    }

    /// Loads an already-opened legacy scenario with the selected definition
    /// vector and frozen startup-player count, reporting coarse C4Game-style
    /// milestones. The callback deliberately stops after decoding the object
    /// records (93); activation and progress 100 belong to the application
    /// thread.
    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_progress<
        R,
        S,
        M,
        F,
    >(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_root: Option<&Path>,
        startup_player_count: i32,
        mut report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
        F: FnMut(i32, &'static str),
    {
        let initial_spellings = initial_modules
            .iter()
            .map(|module| module.as_ref().to_owned())
            .collect::<Vec<_>>();
        let initial_modules = initial_modules
            .iter()
            .map(|module| normalize_definition_path(module.as_ref()))
            .collect::<Vec<_>>();
        let fixed_spellings = fixed_modules.map(|modules| {
            modules
                .iter()
                .map(|module| module.as_ref().to_owned())
                .collect::<Vec<_>>()
        });
        let fixed_modules = fixed_modules.map(|modules| {
            modules
                .iter()
                .map(|module| normalize_definition_path(module.as_ref()))
                .collect::<Vec<_>>()
        });
        Self::load_from_group_with_languages_and_seed_and_definition_modules_inner_with_expansion(
            group,
            resolver,
            &languages.iter().map(AsRef::as_ref).collect::<Vec<_>>(),
            random_seed,
            &initial_modules,
            Some(&initial_spellings),
            fixed_modules.as_deref(),
            fixed_spellings.as_deref(),
            definition_root.map(DefinitionPathExpansion::DirectoryRoot),
            startup_player_count,
            true,
            &mut report_progress,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load_from_group_with_languages_and_seed_and_definition_selection_and_startup_player_count_and_prefix_and_progress<
        R,
        S,
        M,
        F,
    >(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_modules: &[M],
        fixed_modules: Option<&[M]>,
        definition_prefix: Option<&Path>,
        startup_player_count: i32,
        mut report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
        M: AsRef<str>,
        F: FnMut(i32, &'static str),
    {
        let initial_spellings = initial_modules
            .iter()
            .map(|module| module.as_ref().to_owned())
            .collect::<Vec<_>>();
        let initial_modules = initial_spellings
            .iter()
            .map(|module| normalize_definition_path(module))
            .collect::<Vec<_>>();
        let fixed_spellings = fixed_modules.map(|modules| {
            modules
                .iter()
                .map(|module| module.as_ref().to_owned())
                .collect::<Vec<_>>()
        });
        let fixed_modules = fixed_spellings.as_ref().map(|modules| {
            modules
                .iter()
                .map(|module| normalize_definition_path(module))
                .collect::<Vec<_>>()
        });
        let languages = languages.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        Self::load_from_group_with_languages_and_seed_and_definition_modules_inner_with_expansion(
            group,
            resolver,
            &languages,
            random_seed,
            &initial_modules,
            Some(&initial_spellings),
            fixed_modules.as_deref(),
            fixed_spellings.as_deref(),
            definition_prefix.map(DefinitionPathExpansion::LiteralPrefix),
            startup_player_count,
            true,
            &mut report_progress,
        )
    }

    fn load_from_group_with_languages_and_seed_and_definition_modules<R, S>(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_definition_modules: &[String],
        definition_modules: Option<&[String]>,
        selector_definition_root: Option<&Path>,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        Self::load_from_group_with_languages_and_seed_and_definition_modules_and_startup_player_count(
            group,
            resolver,
            languages,
            random_seed,
            initial_definition_modules,
            definition_modules,
            selector_definition_root,
            legacy_startup_player_count(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn load_from_group_with_languages_and_seed_and_definition_modules_and_startup_player_count<
        R,
        S,
    >(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_definition_modules: &[String],
        definition_modules: Option<&[String]>,
        selector_definition_root: Option<&Path>,
        startup_player_count: i32,
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        let mut ignore_progress = |_: i32, _: &'static str| {};
        Self::load_from_group_with_languages_and_seed_and_definition_modules_and_startup_player_count_and_progress(
            group,
            resolver,
            languages,
            random_seed,
            initial_definition_modules,
            definition_modules,
            selector_definition_root,
            startup_player_count,
            &mut ignore_progress,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn load_from_group_with_languages_and_seed_and_definition_modules_and_startup_player_count_and_progress<
        R,
        S,
    >(
        group: &Group,
        resolver: &R,
        languages: &[S],
        random_seed: u64,
        initial_definition_modules: &[String],
        definition_modules: Option<&[String]>,
        selector_definition_root: Option<&Path>,
        startup_player_count: i32,
        report_progress: &mut dyn FnMut(i32, &'static str),
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        let languages = languages.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        Self::load_from_group_with_languages_and_seed_and_definition_modules_inner_with_expansion(
            group,
            resolver,
            &languages,
            random_seed,
            initial_definition_modules,
            None,
            definition_modules,
            None,
            selector_definition_root.map(DefinitionPathExpansion::DirectoryRoot),
            startup_player_count,
            true,
            report_progress,
        )
    }

    // Keep caller-friendly generic APIs above, but erase their types before
    // entering the large legacy loader so it is code-generated only once.
    #[allow(clippy::too_many_arguments)]
    fn load_from_group_with_languages_and_seed_and_definition_modules_inner(
        group: &Group,
        resolver: &dyn LegacyDefinitionResolver,
        languages: &[&str],
        random_seed: u64,
        initial_definition_modules: &[String],
        definition_modules: Option<&[String]>,
        selector_definition_root: Option<&Path>,
        startup_player_count: i32,
        discover_folder_definitions: bool,
        report_progress: &mut dyn FnMut(i32, &'static str),
    ) -> Result<Self, ScenarioError> {
        Self::load_from_group_with_languages_and_seed_and_definition_modules_inner_with_expansion(
            group,
            resolver,
            languages,
            random_seed,
            initial_definition_modules,
            None,
            definition_modules,
            None,
            selector_definition_root.map(DefinitionPathExpansion::DirectoryRoot),
            startup_player_count,
            discover_folder_definitions,
            report_progress,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn load_from_group_with_languages_and_seed_and_definition_modules_inner_with_expansion(
        group: &Group,
        resolver: &dyn LegacyDefinitionResolver,
        languages: &[&str],
        random_seed: u64,
        initial_definition_modules: &[String],
        initial_definition_spellings: Option<&[String]>,
        definition_modules: Option<&[String]>,
        definition_spellings: Option<&[String]>,
        definition_path_expansion: Option<DefinitionPathExpansion<'_>>,
        startup_player_count: i32,
        discover_folder_definitions: bool,
        report_progress: &mut dyn FnMut(i32, &'static str),
    ) -> Result<Self, ScenarioError> {
        match Self::load_from_group(group) {
            Ok(scenario) => Ok(scenario),
            Err(ScenarioError::ManifestMissing) => Self::load_legacy_from_group(
                group,
                resolver,
                languages,
                random_seed,
                initial_definition_modules,
                initial_definition_spellings,
                definition_modules,
                definition_spellings,
                definition_path_expansion,
                startup_player_count,
                discover_folder_definitions,
                report_progress,
            ),
            Err(err) => Err(err),
        }
    }

    pub fn load_from_group(group: &Group) -> Result<Self, ScenarioError> {
        let manifest_bytes = match group.read_file("Scenario.json") {
            Ok(bytes) => bytes,
            Err(GroupError::EntryNotFound(_)) => return Err(ScenarioError::ManifestMissing),
            Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ScenarioError::ManifestMissing);
            }
            Err(error) => return Err(ScenarioError::Resources(error)),
        };

        let manifest: ScenarioManifest = serde_json::from_slice(&manifest_bytes)?;
        Scenario::from_manifest(group, manifest)
    }

    /// Serializes the initial `Scenario.txt` inside a C++ network dynamic.
    ///
    /// C4GameSave::SaveCore copies the loaded C4Scenario, applies the current
    /// engine/title/definition/origin fields, and C4GameSaveNetwork(true) sets
    /// the two network flags without forcing runtime-save flags
    /// (C4GameSave.cpp:58-108,612-617; C4GameSave.h:43-46,173-187).
    /// `scenario_origin` is used only when the loaded core has no Origin, just
    /// like SaveCore retains an existing origin before falling back to the
    /// running scenario filename (C4GameSave.cpp:93-101).
    pub fn serialize_initial_network_scenario(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Result<Vec<u8>, ScenarioError> {
        self.legacy_core
            .as_ref()
            .map(|core| {
                core.initial_network_save(
                    scenario_title,
                    definition_modules,
                    definition_executable_path,
                    definition_path,
                    scenario_origin,
                )
                .serialize()
            })
            .ok_or(ScenarioError::InitialNetworkScenarioUnsupported)
    }

    /// Serializes the initial `Scenario.txt` inside a C++ record group.
    ///
    /// Initial records use the same `C4GameSave::SaveCore` projection as an
    /// initial network dynamic, but `C4GameSaveRecord::AdjustCore` marks the
    /// result as a replay, selects the record icon, and leaves NetworkGame
    /// cleared (C4GameSave.cpp:58-108,576-584; C4GameSave.h:148-168).
    /// `record_title` is the already-formatted title stored by the caller
    /// (`NNN <scenario title> [<build>]` in the C++ application).
    pub fn serialize_initial_record_scenario(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Result<Vec<u8>, ScenarioError> {
        self.legacy_core
            .as_ref()
            .map(|core| {
                core.initial_record_save(
                    record_title,
                    definition_modules,
                    definition_executable_path,
                    definition_path,
                    scenario_origin,
                )
                .serialize()
            })
            .ok_or(ScenarioError::InitialRecordScenarioUnsupported)
    }

    /// Returns the C4Scenario values consumed while creating the initial
    /// network parameters and definition resource.
    ///
    /// C4Scenario::Load performs C4SGame::ConvertGoals before these defaults
    /// are read; C4SDefinitions::GetModules suppresses its list for LocalOnly
    /// scenarios (C4Scenario.cpp:86-97,456-459,503-540;
    /// C4GameParameters.cpp:555-568).
    pub fn initial_network_scenario_metadata(
        &self,
    ) -> Result<InitialNetworkScenarioMetadata, ScenarioError> {
        let core = self
            .legacy_core
            .as_ref()
            .ok_or(ScenarioError::InitialNetworkScenarioUnsupported)?;
        let core = core.after_load_conversion();

        Ok(InitialNetworkScenarioMetadata {
            icon: core.head.icon,
            definition_modules: if core.definitions.local_only {
                Vec::new()
            } else {
                core.definitions.definitions.clone()
            },
            random_seed: core.head.random_seed,
            max_players: core.head.max_player,
            use_fair_crew: core.head.forced_fair_crew == 1,
            fair_crew_forced: core.head.forced_fair_crew != 0,
            fair_crew_strength: core.head.fair_crew_strength,
            rules: scenario_id_list_entries(&core.game.rules),
            goals: scenario_id_list_entries(&core.game.goals),
        })
    }

    /// Returns the post-load C4TeamList used by JoinGameParameters.
    ///
    /// Existing Teams.txt files use C4TeamList/C4Team compiler defaults and
    /// retain repeated-section order. If the file is absent, C4TeamList::Load
    /// derives its active/autogenerate flags from the already-converted
    /// scenario goals and rules (C4Teams.cpp:138-150,556-651).
    pub fn initial_network_team_metadata(
        &self,
    ) -> Result<InitialNetworkTeamMetadata, ScenarioError> {
        let core = self
            .legacy_core
            .as_ref()
            .ok_or(ScenarioError::InitialNetworkTeamMetadataUnsupported)?;

        if let Some(loaded) = self.legacy_team_metadata.as_ref() {
            if let Some(value) = loaded.unsupported_team_distribution {
                return Err(ScenarioError::InitialNetworkTeamDistributionUnsupported { value });
            }
            if let Some(team_id) = loaded.random_color_team_id {
                return Err(ScenarioError::InitialNetworkTeamColorUnsupported { team_id });
            }
            return Ok(loaded.metadata.clone());
        }

        let core = core.after_load_conversion();
        Ok(InitialNetworkTeamMetadata::without_teams_file(&core.game))
    }

    fn load_legacy_from_group(
        group: &Group,
        resolver: &dyn LegacyDefinitionResolver,
        languages: &[&str],
        random_seed: u64,
        initial_definition_modules: &[String],
        initial_definition_spellings: Option<&[String]>,
        definition_modules: Option<&[String]>,
        definition_spellings: Option<&[String]>,
        definition_path_expansion: Option<DefinitionPathExpansion<'_>>,
        startup_player_count: i32,
        discover_folder_definitions: bool,
        report_progress: &mut dyn FnMut(i32, &'static str),
    ) -> Result<Self, ScenarioError> {
        let mut manifest = parse_legacy_scenario_manifest(group)?;
        let language_packs = resolver.resolve_language_packs(group)?;
        let scenario_origin = manifest.core.head.origin.clone();
        let scenario_components =
            language_packs.component_groups(group, Some(group), scenario_origin.as_deref());
        let is_savegame = manifest.core.head.save_game != 0;
        let runtime_landscape = load_runtime_landscape_data(group, is_savegame)?;
        let runtime_current_section = load_runtime_current_scenario_section(group)?;
        // Exact old saves replace the normal definition vector from Game.txt.
        // Discover that boundary before trying to resolve Scenario.txt modules:
        // those modules may intentionally no longer exist.
        let savegame_override =
            load_savegame_definition_override(group, manifest.core.head.save_game != 0)?;
        let has_unresolved_savegame_definitions = matches!(
            &savegame_override,
            ScenarioSavegameDefinitionOverride::GameText { .. }
        );
        report_progress(4, "Scenario manifest and components decoded");

        let skip_ids: HashSet<String> = manifest
            .core
            .definitions
            .skip_defs
            .iter()
            .map(|entry| entry.id.to_ascii_uppercase())
            .collect();
        let mut load_items = Vec::new();
        let mut definition_resource_paths = Vec::new();
        let mut definition_root_groups = Vec::new();
        let mut sound_effect_groups = Vec::new();

        // C4Game::InitDefs loads explicit definition resources first, then
        // folder-local resources from outermost to innermost, and finally the
        // scenario group itself (C4Game.cpp:81-103, 210-213, 3961-3994).
        let configured_definition_spellings = manifest
            .core
            .definitions
            .reflected_definitions
            .clone()
            .filter(|spellings| spellings.len() == manifest.definition_specs.len())
            .unwrap_or_else(|| manifest.definition_specs.clone());
        let (definition_specs, selected_definition_spellings, definition_selection_source) =
            match definition_modules {
                Some(modules) => (
                    modules,
                    definition_spellings
                        .filter(|spellings| spellings.len() == modules.len())
                        .unwrap_or(modules),
                    ScenarioDefinitionSelectionSource::FixedCallerSelection,
                ),
                None if !manifest.core.definitions.local_only
                    && !manifest.definition_specs.is_empty() =>
                {
                    (
                        manifest.definition_specs.as_slice(),
                        configured_definition_spellings.as_slice(),
                        ScenarioDefinitionSelectionSource::ScenarioPreset,
                    )
                }
                None => (
                    initial_definition_modules,
                    initial_definition_spellings
                        .filter(|spellings| spellings.len() == initial_definition_modules.len())
                        .unwrap_or(initial_definition_modules),
                    ScenarioDefinitionSelectionSource::CallerDefaults,
                ),
            };
        let requested_definition_modules = definition_specs.to_vec();
        let requested_definition_spellings = selected_definition_spellings.to_vec();
        report_progress(8, "Definition selection resolved");
        let definition_specs_to_resolve = if has_unresolved_savegame_definitions {
            &[][..]
        } else {
            definition_specs
        };
        let definition_spellings_to_resolve = if has_unresolved_savegame_definitions {
            &[][..]
        } else {
            selected_definition_spellings
        };
        if let Some(expansion) = definition_path_expansion {
            // C4Game::OpenScenario inserts an expanded copy of every selected
            // module at the vector's beginning, preserving the original
            // vector afterward. Resolve the two blocks independently so a
            // missing expanded copy can never be hidden by a search-root match.
            for (spec, spelling) in definition_specs_to_resolve
                .iter()
                .zip(definition_spellings_to_resolve)
            {
                let definition_group = match expansion {
                    DefinitionPathExpansion::DirectoryRoot(root) => {
                        resolve_rooted_definition_group(root, spec)?
                    }
                    DefinitionPathExpansion::LiteralPrefix(prefix) => {
                        resolve_prefixed_definition_group(prefix, spelling)?
                    }
                };
                definition_resource_paths.push(definition_group.root().to_path_buf());
                definition_root_groups.push(definition_group.clone());
                collect_definitions_from_group(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                )?;
            }
            for spec in definition_specs_to_resolve {
                let definition_group = resolve_one_definition_group(group, resolver, spec)?;
                definition_resource_paths.push(definition_group.root().to_path_buf());
                definition_root_groups.push(definition_group.clone());
                collect_definitions_from_group(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                )?;
            }
        } else {
            for spec in definition_specs_to_resolve {
                let definition_group = resolve_one_definition_group(group, resolver, spec)?;
                definition_resource_paths.push(definition_group.root().to_path_buf());
                definition_root_groups.push(definition_group.clone());
                collect_definitions_from_group(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                )?;
            }
        }

        if discover_folder_definitions {
            for folder_group in folder_local_definition_groups(group)? {
                definition_resource_paths.push(folder_group.root().to_path_buf());
                definition_root_groups.push(folder_group.clone());
                collect_definitions_from_group(
                    &folder_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                )?;
            }
        }

        // InitDefs' scenario pass disables System.c4g discovery because the
        // scenario-local group is loaded later by LoadScenarioScripts.
        collect_definitions_from_group(
            group,
            false,
            &skip_ids,
            languages,
            &language_packs,
            group,
            scenario_origin.as_deref(),
            &mut sound_effect_groups,
            &mut load_items,
        )?;

        // fOverload replaces and destroys an earlier same-ID C4Def script,
        // while System hosts loaded between the two definitions remain live.
        // Keep only the last definition event for each ID without flattening
        // the surviving System host order.
        let mut last_definition = HashMap::new();
        for (index, item) in load_items.iter().enumerate() {
            if let CollectedDefinition::Definition(definition) = item {
                last_definition.insert(definition.id.to_ascii_uppercase(), index);
            }
        }
        let mut collected = Vec::new();
        let mut definition_load_steps = Vec::new();
        for (index, item) in load_items.into_iter().enumerate() {
            match item {
                CollectedDefinition::Definition(definition)
                    if last_definition.get(&definition.id.to_ascii_uppercase()) == Some(&index) =>
                {
                    definition_load_steps
                        .push(DefinitionLoadStep::Definition(definition.id.clone()));
                    collected.push(definition);
                }
                CollectedDefinition::Definition(definition) => {
                    definition_load_steps.push(DefinitionLoadStep::Declarations {
                        name: definition.id,
                        source: definition.script,
                    });
                }
                CollectedDefinition::SystemScripts(sources) if !sources.is_empty() => {
                    definition_load_steps.push(DefinitionLoadStep::SystemScripts(sources));
                }
                CollectedDefinition::SystemScripts(_) => {}
                CollectedDefinition::Particle(definition) => {
                    definition_load_steps.push(DefinitionLoadStep::Particle(definition));
                }
            }
        }

        // C4Game checks for an entirely missing definition load before these
        // filters; filtering every loaded def is therefore not itself fatal.
        if collected.is_empty() && !has_unresolved_savegame_definitions {
            return Err(ScenarioError::NoDefinitions);
        }

        prune_incompatible_definitions(&mut collected);
        let retained_definition_ids: HashSet<&str> = collected
            .iter()
            .map(|definition| definition.id.as_str())
            .collect();
        definition_load_steps.retain(|step| match step {
            DefinitionLoadStep::Definition(id) => retained_definition_ids.contains(id.as_str()),
            DefinitionLoadStep::Declarations { .. }
            | DefinitionLoadStep::SystemScripts(_)
            | DefinitionLoadStep::Particle(_) => true,
        });
        report_progress(40, "Definition metadata and sources collected");

        let script = load_legacy_scenario_script(group, &scenario_components, languages)?;
        let scenario_system_scripts = load_scenario_system_scripts(
            group,
            &language_packs,
            scenario_origin.as_deref(),
            languages,
        )?;
        report_progress(56, "Scenario script sources loaded");
        let map_callback_functions = scenario_map_callback_functions(
            script.as_ref(),
            &collected,
            &definition_load_steps,
            &scenario_system_scripts,
        )?;
        report_progress(57, "Scenario callback names indexed");
        let mut classifier = build_map_pixel_classifier(group, resolver)?;
        report_progress(58, "Material and texture-map data decoded");
        let material_library = classifier
            .as_ref()
            .and_then(MapPixelClassifier::material_library)
            .cloned();
        report_progress(60, "Material library prepared");
        let mut post_init_map_callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut prepared_map_creator = None;
        let mut landscape = load_legacy_landscape(
            group,
            &manifest,
            runtime_landscape.as_ref(),
            false,
            classifier.as_mut(),
            random_seed,
            startup_player_count,
            &map_callback_functions,
            &mut post_init_map_callbacks,
            &mut prepared_map_creator,
        )?;
        report_progress(88, "Landscape data generated or decoded");
        if let (Some(runtime), Some(landscape)) = (runtime_landscape, landscape.as_mut()) {
            if is_savegame {
                landscape.set_border_open(
                    runtime.left_open,
                    runtime.right_open,
                    runtime.top_open,
                    runtime.bottom_open,
                );
            }
            landscape.set_modulation(runtime.mat_modulation);
        }
        report_progress(89, "Landscape data finalized");
        report_progress(90, "Loading landscape auxiliary data");
        let landscape_systems =
            load_legacy_landscape_systems_with_progress(group, report_progress)?;
        // Crew never spawns at scenario load: C4Game::InitPlayers queues
        // CID_JoinPlr and C4Player::ScenarioInit places crew at JOIN time
        // (C4Player.cpp:481-570) — see Engine::join_player.
        let legacy_string_table = load_legacy_string_table(group)?;
        let converted_core = manifest.core.after_load_conversion();
        let round_results = load_legacy_round_results(
            group,
            legacy_game_is_melee_after_conversion(&converted_core.game),
        )?;
        let initial_spawns = if has_unresolved_savegame_definitions {
            // Object IDs cannot be decoded truthfully until the legacy
            // DefinitionFiles text has been interpreted by a runtime loader.
            Vec::new()
        } else {
            collect_legacy_objects(group, &collected, &legacy_string_table)?
        };
        report_progress(93, "Object records decoded");
        let (mut physics, gravity) = derive_legacy_physics(&manifest)?;
        if let Some(runtime) = runtime_landscape.filter(|_| is_savegame) {
            let physics = physics.get_or_insert_with(PhysicsSettings::default);
            physics.set_raw_gravity(runtime.gravity);
        }
        let environment = derive_legacy_environment(&manifest)?;
        let weather_init = derive_legacy_weather_init(&manifest)?;
        // C4Sky always initializes for a running game (C4Sky::Init,
        // C4Sky.cpp:71-152): bitmap sky or fade gradient.
        let sky = derive_legacy_sky(
            group,
            resolver,
            &definition_root_groups,
            &mut manifest,
            random_seed,
        )?;
        // C4Sky::Init mutates the stored SkyDef's comma separators whenever
        // the direct scenario `Sky` bitmap misses. Retain that post-init core
        // for script reflection and save/network serialization.
        let legacy_core = manifest.core.clone();
        let scenario_sections = load_legacy_scenario_sections(
            group,
            &manifest,
            classifier.as_mut(),
            random_seed,
            startup_player_count,
            &runtime_current_section,
            &landscape,
            &landscape_systems,
            &initial_spawns,
            environment,
            sky.surface.is_some(),
            &map_callback_functions,
            &post_init_map_callbacks,
        )?;
        let (_, legacy_team_metadata) =
            load_initial_network_teams(group, &scenario_components, languages)?;
        let (teams, lobby_teams) =
            load_legacy_teams(group, &scenario_components, languages, &manifest.core)?;
        let game_parameter_defaults = game_parameter_defaults(&manifest.core);
        let embedded_game_parameters =
            load_legacy_game_parameter_overrides(group, &game_parameter_defaults)?;
        let effective_min_players = legacy_effective_min_players(&manifest.core);
        let lobby_metadata = ScenarioLobbyMetadata {
            head: ScenarioLobbyHead {
                configured_min_players: manifest.core.head.min_player,
                effective_min_players,
                max_players: manifest.core.head.max_player,
                max_players_league: manifest.core.head.max_player_league,
                save_game: manifest.core.head.save_game != 0,
                replay: manifest.core.head.replay != 0,
                loader: ScenarioLoaderMetadata {
                    configured_specification: manifest.core.head.loader.clone(),
                },
                fair_crew_force: ScenarioFairCrewForce::from_raw(
                    manifest.core.head.forced_fair_crew,
                ),
                fair_crew_strength: manifest.core.head.fair_crew_strength,
                random_seed: manifest.core.head.random_seed,
                network_game: manifest.core.head.network_game,
                network_runtime_join: manifest.core.head.network_runtime_join,
            },
            definitions: ScenarioLobbyDefinitions {
                local_only: manifest.core.definitions.local_only,
                allow_user_change: manifest.core.definitions.allow_user_change,
                configured_modules: manifest.definition_specs.clone(),
                configured_module_spellings: configured_definition_spellings,
                requested_modules: requested_definition_modules,
                requested_module_spellings: requested_definition_spellings,
                selection_source: definition_selection_source,
                definition_root_applied: definition_path_expansion.is_some(),
                resolved_load_resources: definition_resource_paths.clone(),
                savegame_override,
            },
            game_parameter_defaults,
            embedded_game_parameters,
            teams: lobby_teams,
        };
        Ok(Self {
            legacy_core: Some(legacy_core),
            legacy_team_metadata,
            name: manifest.title,
            description: manifest.description,
            ticks: None,
            ground_height_hint: manifest.ground_height_hint,
            material_library,
            definitions: collected,
            value_overloads: id_list_pairs(&manifest.core.game.realism.value_overloads),
            initial_spawns,
            landscape,
            post_init_map_callbacks,
            keep_map_creator: manifest.core.landscape.keep_map_creator,
            scenario_sections,
            physics,
            runtime_landscape,
            legacy_string_table,
            round_results,
            gravity,
            environment: Some(environment),
            weather_init: Some(weather_init),
            sky: Some(sky),
            script,
            objectives: ScenarioObjectives::from_legacy_game(&manifest.core.game),
            construction_needs_material: manifest.core.game.realism.construction_needs_material,
            structures_need_energy: manifest.core.game.realism.structures_need_energy,
            base_buy_enabled: (manifest.core.game.realism.base_functionality & BASEFUNC_BUY) != 0,
            base_sell_enabled: (manifest.core.game.realism.base_functionality & BASEFUNC_SELL) != 0,
            base_auto_sell_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_AUTO_SELL_CONTENTS)
                != 0,
            base_reject_entrance_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_REJECT_ENTRANCE)
                != 0,
            base_regenerate_energy_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_REGENERATE_ENERGY)
                != 0,
            base_extinguish_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_EXTINGUISH)
                != 0,
            base_regenerate_energy_price: manifest.core.game.realism.base_regenerate_energy_price,
            landscape_insert_thrust: manifest.core.game.realism.landscape_insert_thrust != 0,
            disable_mouse: manifest.core.head.disable_mouse != 0,
            forced_auto_context_menu: (manifest.core.head.forced_auto_context_menu >= 0)
                .then_some(manifest.core.head.forced_auto_context_menu != 0),
            forced_control_style: (manifest.core.head.forced_control_style >= 0)
                .then_some(manifest.core.head.forced_control_style != 0),
            definition_load_steps,
            definition_resource_paths,
            definition_root_groups,
            sound_effect_groups,
            scenario_system_scripts,
            player_starts: PlayerStart::slots_from_legacy(&manifest.core.players),
            teams,
            lobby_metadata: Some(lobby_metadata),
            standard_names: group
                .read_file("Names.txt")
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            map_zoom: manifest.core.landscape.map_zoom,
            init_placement: Some(LegacyInitPlacement {
                save_game: manifest.core.head.save_game != 0,
                no_initialize: manifest.core.head.no_initialize != 0,
                vegetation: id_list_pairs(&manifest.core.landscape.vegetation),
                vegetation_level: manifest.core.landscape.vegetation_level,
                in_earth: id_list_pairs(&manifest.core.landscape.in_earth),
                in_earth_level: manifest.core.landscape.in_earth_level,
                animals: id_list_pairs(&manifest.core.animals.free_life),
                nests: id_list_pairs(&manifest.core.animals.earth_nest),
                environment: id_list_pairs(&manifest.core.environment.objects),
                goals: id_list_pairs(&converted_core.game.goals),
                rules: id_list_pairs(&converted_core.game.rules),
                earth_material: manifest.core.landscape.material.clone(),
            }),
        })
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Whether the parsed legacy `[Head] NetworkGame` safety flag is set.
    ///
    /// C4Game checks this flag after retrieving and opening a client scenario
    /// and refuses non-network scenarios (C4Game.cpp:2551-2564).
    pub fn network_game(&self) -> bool {
        self.legacy_core
            .as_ref()
            .is_some_and(|core| core.head.network_game)
    }

    pub fn configured_ticks(&self) -> Option<u32> {
        self.ticks
    }

    pub fn ground_height_hint(&self) -> Option<i32> {
        self.ground_height_hint
    }

    pub fn visit_definition_groups<F>(&self, mut f: F)
    where
        F: FnMut(&str, &Group),
    {
        for definition in &self.definitions {
            if let Some(group) = &definition.resource_group {
                f(&definition.id, group);
            }
        }
    }

    /// Returns Rust's ordered external/folder definition resources. Consult
    /// `lobby_metadata().definitions().savegame_override()` before treating
    /// this as the C++-effective vector for an old exact save.
    pub fn definition_resource_paths(&self) -> &[PathBuf] {
        &self.definition_resource_paths
    }

    /// Exact ordered resources registered as `C4GSCnt_DefinitionRoot`,
    /// including folder-local definition roots appended by C++.
    pub fn definition_root_groups(&self) -> &[Group] {
        &self.definition_root_groups
    }

    /// Exact ordered groups for which native `C4Def::Load` invokes
    /// `C4SoundSystem::LoadEffects`. Entries are direct-load events, not
    /// definition roots: callers must inspect only each group's direct audio
    /// files and must preserve duplicates and order.
    pub fn sound_effect_groups(&self) -> &[Group] {
        &self.sound_effect_groups
    }

    /// Immutable legacy pre-game metadata. JSON-only engine fixtures return
    /// `None` because they do not contain Scenario.txt/Teams.txt semantics.
    pub fn lobby_metadata(&self) -> Option<&ScenarioLobbyMetadata> {
        self.lobby_metadata.as_ref()
    }

    pub fn has_initial_objects(&self) -> bool {
        !self.initial_spawns.is_empty()
    }

    pub(crate) fn scenario_sections(&self) -> &[ScenarioSectionSpec] {
        &self.scenario_sections
    }

    pub fn objectives(&self) -> &ScenarioObjectives {
        &self.objectives
    }

    /// The scenario-wide `ForcedAutoStopControl` override, or `None` when
    /// joining players should retain their player-file preference.
    pub fn forced_control_style(&self) -> Option<bool> {
        self.forced_control_style
    }

    /// Whether `[Head] DisableMouse` prevents player mouse control.
    pub fn disables_mouse(&self) -> bool {
        self.disable_mouse
    }

    /// The scenario-wide `ForcedAutoContextMenu` override, or `None` when
    /// joining players should retain their player-file preference.
    pub fn forced_auto_context_menu(&self) -> Option<bool> {
        self.forced_auto_context_menu
    }

    pub fn physics(&self) -> Option<PhysicsSettings> {
        self.physics
    }

    pub fn environment(&self) -> Option<EnvironmentSettings> {
        self.environment
    }

    pub fn sky(&self) -> Option<&SkyConfig> {
        self.sky.as_ref()
    }

    fn initial_team_configuration(&self) -> crate::TeamConfiguration {
        let team_metadata = self
            .legacy_team_metadata
            .as_ref()
            .map(|loaded| loaded.metadata.clone())
            .or_else(|| {
                self.legacy_core.as_ref().map(|core| {
                    let core = core.after_load_conversion();
                    InitialNetworkTeamMetadata::without_teams_file(&core.game)
                })
            });
        team_metadata
            .map(|teams| crate::TeamConfiguration {
                custom: teams.custom,
                active: teams.active,
                allow_hostility_change: teams.allow_hostility_change,
                distribution: teams.team_distribution as i32,
                allow_team_switch: teams.allow_team_switch,
                auto_generate_teams: teams.auto_generate_teams,
                team_colors: teams.team_colors,
            })
            .unwrap_or_default()
    }

    /// Applies the scenario through C4Game::InitGame and the subsequent
    /// synchronization pass, but deliberately leaves Script.Initialize for
    /// the caller. Fresh games use this boundary to run InitGameFinal before
    /// the queued startup-player controls execute.
    pub fn apply_before_players(
        &self,
        engine: &mut Engine,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine, true, None, None, None, None, true, None,
        )
    }

    /// Applies a fresh or restored scenario after installing the authoritative
    /// saved/runtime C4TeamList configuration. Definition and placement
    /// callbacks therefore observe the same values as the eventual game.
    #[doc(hidden)]
    pub fn apply_before_players_with_team_configuration(
        &self,
        engine: &mut Engine,
        team_configuration: crate::TeamConfiguration,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine,
            true,
            Some(team_configuration),
            None,
            None,
            None,
            true,
            None,
        )
    }

    /// Applies an ordinary offline savegame after installing its compiled
    /// `Game.txt` state. This is the non-network counterpart of
    /// [`Self::apply_before_network_final_init_with_game_data`]: offline
    /// startup performs its synchronization immediately, before
    /// `SavePlayerInfos` recreates the live players.
    #[doc(hidden)]
    pub fn apply_before_players_with_game_data(
        &self,
        engine: &mut Engine,
        game_data: &InitialNetworkGameData,
        team_configuration: Option<crate::TeamConfiguration>,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine,
            true,
            team_configuration,
            None,
            None,
            Some(game_data),
            true,
            None,
        )
    }

    /// Resource/setup pass used immediately before restoring an authoritative
    /// saved EngineState. The save's exact landscape has no live S2 creator,
    /// so replaying callbacks from the original Scenario.txt would duplicate
    /// side effects that are not necessarily overwritten by restore.
    #[doc(hidden)]
    pub fn apply_before_players_for_restore(
        &self,
        engine: &mut Engine,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine, true, None, None, None, None, false, None,
        )
    }

    #[doc(hidden)]
    pub fn apply_before_players_for_restore_with_team_configuration(
        &self,
        engine: &mut Engine,
        team_configuration: crate::TeamConfiguration,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine,
            true,
            Some(team_configuration),
            None,
            None,
            None,
            false,
            None,
        )
    }

    /// Applies the `C4Game::InitGame` phase of a network game, preserving the
    /// loaded synchronization state until the GO status barrier commits.
    /// `C4Network2::FinalInit` performs the matching final synchronization.
    pub fn apply_before_network_final_init(
        &self,
        engine: &mut Engine,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine, false, None, None, None, None, true, None,
        )
    }

    /// Network variant of
    /// [`Scenario::apply_before_players_with_team_configuration`].
    #[doc(hidden)]
    pub fn apply_before_network_final_init_with_team_configuration(
        &self,
        engine: &mut Engine,
        team_configuration: crate::TeamConfiguration,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine,
            false,
            Some(team_configuration),
            None,
            None,
            None,
            true,
            None,
        )
    }

    /// Applies the network initialization phase after installing the
    /// authoritative runtime C4TeamList entries and configuration. Both are
    /// visible to definition and placement callbacks.
    #[doc(hidden)]
    pub fn apply_before_network_final_init_with_team_registry(
        &self,
        engine: &mut Engine,
        teams: Vec<TeamInfo>,
        team_configuration: crate::TeamConfiguration,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine,
            false,
            Some(team_configuration),
            Some(teams),
            None,
            None,
            true,
            None,
        )
    }

    /// Network initialization with the already-compiled Game.txt state, or
    /// InitSystem defaults when the component was absent. State is installed
    /// after static resource setup and before definition callbacks; the later
    /// fresh/savegame Sky and Weather branches retain their native ordering.
    #[doc(hidden)]
    pub fn apply_before_network_final_init_with_game_data(
        &self,
        engine: &mut Engine,
        game_data: &InitialNetworkGameData,
        team_configuration: Option<crate::TeamConfiguration>,
        team_registry: Option<Vec<TeamInfo>>,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        self.apply_before_players_with_final_synchronize(
            engine,
            false,
            team_configuration,
            team_registry,
            None,
            Some(game_data),
            true,
            None,
        )
    }

    /// App startup projection of C4Game::InitGame. When requested, capture
    /// the initial-record Game.txt at InitControl's native seam: objects and
    /// loaded runtime blocks exist, but InitializeDef, environment placement,
    /// Weather.Init and Script.Initialize have not executed yet.
    #[doc(hidden)]
    pub fn apply_before_players_for_game_start(
        &self,
        engine: &mut Engine,
        network_game: bool,
        game_data: Option<&InitialNetworkGameData>,
        team_configuration: Option<crate::TeamConfiguration>,
        team_registry: Option<Vec<TeamInfo>>,
        game_parameter_rule_goal_lists: Option<&GameParameterRuleGoalLists>,
        initial_record_music_enabled: Option<bool>,
    ) -> Result<
        (
            Vec<ObjectId>,
            Option<Result<InitialNetworkGameData, InitialNetworkGameError>>,
        ),
        ScenarioError,
    > {
        let mut initial_record = None;
        let capture =
            initial_record_music_enabled.map(|music_enabled| (music_enabled, &mut initial_record));
        let created = self.apply_before_players_with_final_synchronize(
            engine,
            !network_game,
            team_configuration,
            network_game.then_some(team_registry).flatten(),
            game_parameter_rule_goal_lists,
            game_data,
            true,
            capture,
        )?;
        Ok((created, initial_record))
    }

    /// Validate every compiled runtime block while host preparation is still
    /// side-effect free. GO reparses the retained canonical bytes and applies
    /// the same staged state after resources and objects exist.
    pub fn validate_initial_network_game_data(
        &self,
        game_data: &InitialNetworkGameData,
    ) -> Result<(), ScenarioError> {
        game_data.validate_runtime_application()?;
        InitialNetworkRuntimeState::parse(game_data)?;
        Ok(())
    }

    /// Live environment staged before the native Weather.Init boundary.
    /// Fresh legacy games still expose C4Weather::Default here; savegames
    /// and synthetic scenarios need their configured metadata immediately.
    fn environment_before_weather_init(&self, runtime_savegame: bool) -> EnvironmentSettings {
        if self.weather_init.is_some() && !runtime_savegame {
            EnvironmentSettings::default()
        } else {
            self.environment.unwrap_or_default()
        }
    }

    /// Initial C4Landscape::CreateMapS2 runs after script linking in C++.
    /// Resource loading has already parsed/evaluated the creator so texture
    /// slots and diagnostics stay deterministic; replay only RenderTo here,
    /// on the real scenario host, then evaluate MapZoom from the resulting
    /// fixed map RNG and replace the eager preview landscape.
    fn rerender_initial_s2_map(
        &self,
        engine: &mut Engine,
        callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    ) -> Result<(), ScenarioError> {
        let Some(spec) = self
            .scenario_sections
            .first()
            .and_then(|section| section.s2_overload.as_ref())
        else {
            return Ok(());
        };
        let Some((mut creator, mut map_rng, map_seed, modulation, texmap)) =
            engine.landscape.as_ref().and_then(|landscape| {
                let raster = landscape.raster_state()?;
                let creator = raster.map_creator()?.clone();
                Some((
                    creator.clone(),
                    creator.pre_render_rng()?,
                    landscape.map_seed(),
                    landscape.modulation(),
                    raster.texmap().clone(),
                ))
            })
        else {
            return Ok(());
        };

        let bitmap = {
            let mut call = |rng: &mut crate::rng::LcgRng, function: &str, args: [i32; 4]| {
                engine.call_map_script_algorithm(rng, function, args)
            };
            crate::map_creator_s2::rerender_last_s2_map_with_script_algo(
                &mut creator,
                &mut map_rng,
                &mut call,
            )
        };
        let Some(bitmap) = bitmap else {
            return Ok(());
        };

        let map_zoom = spec.map_zoom.evaluate(&mut map_rng) as u32 as i32;
        creator.set_callback_map_zoom(map_zoom);
        *callbacks = creator.callback_state();
        let classifier = MapPixelClassifier::from_runtime_state(texmap);
        let mut landscape = classified_landscape(&bitmap, &classifier, map_zoom, map_seed)?;
        landscape
            .save_initial()
            .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
        if let Some(diff) = spec.diff.as_ref() {
            // Initial ApplyDiff failure is non-fatal in native Landscape::Init.
            let _ = landscape.apply_diff(diff);
        }
        landscape.set_shade_materials(spec.shade_materials);
        landscape.set_no_scan(spec.no_scan);
        landscape.set_border_open(
            spec.left_open,
            spec.right_open,
            spec.top_open,
            spec.bottom_open,
        );
        if spec.auto_scan_side_open {
            landscape.scan_side_open();
        }
        landscape.set_modulation(modulation);
        landscape.set_runtime_mode(LANDSCAPE_MODE_DYNAMIC);
        landscape
            .raster_state_mut()
            .expect("classified S2 landscapes carry raster state")
            .set_map_creator(Some(creator.clone()));
        engine.refresh_initial_s2_section(&landscape, &creator, callbacks);
        engine.set_landscape(landscape);
        Ok(())
    }

    fn apply_before_players_with_final_synchronize(
        &self,
        engine: &mut Engine,
        final_synchronize: bool,
        team_configuration_override: Option<crate::TeamConfiguration>,
        team_registry_override: Option<Vec<TeamInfo>>,
        game_parameter_rule_goal_lists: Option<&GameParameterRuleGoalLists>,
        initial_network_game: Option<&InitialNetworkGameData>,
        execute_post_init_map_callbacks: bool,
        initial_record_capture: Option<(
            bool,
            &mut Option<Result<InitialNetworkGameData, InitialNetworkGameError>>,
        )>,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        let mut live_post_init_map_callbacks = self.post_init_map_callbacks.clone();
        let mut initial_network_runtime = initial_network_game
            .map(InitialNetworkRuntimeState::parse)
            .transpose()?;
        engine.clear_scenario_script();
        // The same scenario-level C4StringTable is used by Objects.txt and
        // by embedded player-file ExtraData restored later in this startup
        // pipeline. Publish the exact enumeration before either consumer can
        // bind runtime state.
        engine.adopt_legacy_string_table(self.legacy_string_table.clone());
        // C4Scenario::Load/ConvertGoals and C4Landscape::Init have completed
        // before any definition/scenario initialization callback can query
        // Game.C4S. Reset on every apply so a reused Engine cannot retain the
        // preceding scenario's reflection state.
        engine.set_scenario_values(
            self.legacy_core
                .as_ref()
                .map(|core| {
                    ScenarioValueStore::from_runtime_core(
                        core,
                        self.sky
                            .as_ref()
                            .and_then(|sky| sky.surface.as_ref())
                            .is_some(),
                    )
                })
                .unwrap_or_default(),
        );
        engine.configure_scenario_sections(&self.scenario_sections);
        // C4GraphicsSystem::Default initializes all nine controls before a
        // fresh scenario-apply boundary, after which scenario Initialize may
        // call SetGamma. Save loading restores its captured controls after
        // this resource/setup pass (C4GraphicsSystem.cpp:277-281;
        // C4Game.cpp:882-960).
        engine.reset_gamma_controls();
        engine.configure_objectives(self.objectives.clone());
        // C4Player::InitControl applies the scenario head override to every
        // subsequent join (C4Player.cpp:1747,2369-2389).
        engine.set_forced_control_style(self.forced_control_style);
        engine.set_forced_auto_context_menu(self.forced_auto_context_menu);
        // C4SPlrStart outlives scenario load: ScenarioInit reads it when a
        // player joins (C4Player.cpp:670-777).
        engine.set_player_starts(self.player_starts.clone());
        if let Ok(metadata) = self.initial_network_team_metadata() {
            engine.set_initial_network_team_metadata(&metadata);
        }
        engine.set_teams(team_registry_override.unwrap_or_else(|| self.teams.clone()));
        // Game.Teams retains all seven script-queryable values. A missing or
        // zero-byte Teams.txt uses the scenario-derived C4TeamList defaults;
        // a compiled file keeps its independent flags even when it has no
        // Team entries. Network and save callers may replace that static
        // seed before any InitializeDef or placement callback can observe it.
        engine.set_team_configuration(
            team_configuration_override.unwrap_or_else(|| self.initial_team_configuration()),
        );
        engine.set_map_zoom(self.map_zoom);
        // A scenario Names.txt overrides the standard clonk names
        // (C4Game.cpp:3288-3289); without one the installer's choice (the
        // planet System.c4g Names.txt) stays in place.
        if self.standard_names.is_some() {
            engine.set_standard_names(self.standard_names.clone());
        }
        if let Some(material_library) = &self.material_library {
            engine.configure_materials_from_library(material_library);
        }
        if let Some(landscape) = &self.landscape {
            engine.set_landscape(landscape.clone());
        } else {
            engine.clear_landscape();
        }
        // The first section is the root world. In an exact save that may be
        // a non-Main current section; a retained SectMain child is inactive.
        let root_landscape_systems = self
            .scenario_sections
            .first()
            .map(|section| section.landscape_systems.clone())
            .unwrap_or_default();
        engine.load_scenario_landscape_systems(
            &root_landscape_systems,
            self.landscape.is_some() || self.scenario_sections.is_empty(),
        );

        // C4Landscape::ScenarioInit evaluates Gravity through the synced
        // ledger (C4Landscape.cpp:66) BEFORE Weather.Init's draws —
        // probe-verified: the C++ pre-wind sequence is [Gravity r=1,
        // Season r=1, YearSpeed r=1, Climate r=1, Wind r=151]. C4S always
        // has a Landscape block (defaults), so the draw is unconditional
        // on the legacy path; skipping it shifted every weather value by
        // one ledger position (the 402 Breeze/Still wind class).
        let runtime_savegame = self
            .legacy_core
            .as_ref()
            .is_some_and(|core| core.head.save_game != 0);
        let scenario_gravity = (self.weather_init.is_some() && !runtime_savegame)
            .then(|| engine.evaluate_scenario_gravity(self.gravity));
        if let Some(mut physics) = self.physics {
            if let Some(gravity) = scenario_gravity {
                physics.set_script_gravity(gravity);
            }
            engine.set_physics(physics);
        }

        // C4Weather::Default remains the live weather for a fresh legacy
        // game until Weather.Init(true), after InitializeDef, all placement
        // callbacks and Landscape.PostInitMap (C4Weather.cpp:186-194;
        // C4Game.cpp:2505-2525). Savegames must instead expose their
        // scenario metadata here so Game.txt can overlay the compiled live
        // Weather state before those callbacks; synthetic scenarios have no
        // deferred Weather.Init and keep their configured environment.
        engine.set_environment(self.environment_before_weather_init(runtime_savegame));
        // Weather.Init's draws happen AFTER the definitions, the loaded
        // objects and the InitVegetation→InitGoals placements — see the
        // block below the spawn loop (C4Game.cpp:2496-2507).
        if let Some(sky) = &self.sky {
            engine.set_sky(sky.settings.clone());
        } else {
            engine.clear_sky();
        }

        engine.set_construction_needs_material(self.construction_needs_material);
        engine.set_structures_need_energy(self.structures_need_energy);
        engine.set_base_buy_enabled(self.base_buy_enabled);
        engine.set_base_sell_enabled(self.base_sell_enabled);
        engine.set_base_auto_sell_enabled(self.base_auto_sell_enabled);
        engine.set_base_reject_entrance_enabled(self.base_reject_entrance_enabled);
        engine.set_base_regenerate_energy_enabled(self.base_regenerate_energy_enabled);
        engine.set_base_extinguish_enabled(self.base_extinguish_enabled);
        engine.set_base_regenerate_energy_price(self.base_regenerate_energy_price);
        engine.set_landscape_insert_thrust(self.landscape_insert_thrust);

        if let Some(game_data) = initial_network_game {
            engine.apply_initial_network_game_data(game_data)?;
            if let Some(runtime) = initial_network_runtime.as_mut() {
                if let Some(sky) = runtime.sky.take() {
                    let settings = engine.sky_settings().cloned().unwrap_or_default();
                    let sky_scroll_mode = self
                        .legacy_core
                        .as_ref()
                        .map_or(0, |core| core.landscape.sky_scroll_mode);
                    engine.apply_initial_network_sky_frame(&sky.into_frame(
                        settings,
                        runtime_savegame,
                        sky_scroll_mode,
                    ));
                }
                engine.apply_initial_network_scoreboard(std::mem::take(&mut runtime.scoreboard));
            }
        }

        for step in &self.definition_load_steps {
            let definition = match step {
                DefinitionLoadStep::Definition(id) => self
                    .definitions
                    .iter()
                    .find(|definition| definition.id.eq_ignore_ascii_case(id))
                    .ok_or_else(|| ScenarioError::UnknownDefinition(id.clone()))?,
                DefinitionLoadStep::Declarations { name, source } => {
                    match clonk_script::Script::compile_c4_string(source) {
                        Ok(script) => {
                            for diagnostic in script.parse_diagnostics() {
                                tracing::warn!(
                                    definition = %name,
                                    %diagnostic,
                                    "superseded definition parse error quarantined; continuing like C++"
                                );
                            }
                            if let Err(diagnostic) =
                                clonk_script::register_global_declarations_with_strings(
                                    script.var_decls(),
                                    &engine.script_globals,
                                    Some(&engine.script_global_consts),
                                    &engine.script_string_registrations,
                                )
                            {
                                tracing::warn!(
                                    definition = %name,
                                    %diagnostic,
                                    "superseded definition static-constant link diagnostic; continuing like C++"
                                );
                            }
                        }
                        Err(error) => tracing::warn!(
                            definition = %name,
                            %error,
                            "superseded definition script failed to preparse; continuing like C++"
                        ),
                    }
                    continue;
                }
                DefinitionLoadStep::SystemScripts(sources) => {
                    engine.install_additional_global_scripts(sources);
                    continue;
                }
                DefinitionLoadStep::Particle(resource) => {
                    if let Err(error) = engine.register_particle_resource(resource) {
                        tracing::warn!(
                            particle = %resource.core.name,
                            %error,
                            "particle definition failed to register; skipping"
                        );
                    }
                    continue;
                }
            };
            let name = definition.name.as_deref().unwrap_or(&definition.id);
            // C4Def::Load ignores Script.Load failures (C4Def.cpp:632): a
            // definition with a broken script still loads, script-less; the
            // error only shows in the log.
            let mut compiled =
                match Definition::from_script(&definition.id, name, &definition.script) {
                    Ok(compiled) => compiled,
                    Err(error) => {
                        tracing::warn!(
                            definition = %definition.id,
                            %error,
                            "definition script failed to load; registering script-less like C++"
                        );
                        Definition::from_script(&definition.id, name, "")?
                    }
                };
            if let Some(script_name) = &definition.script_name {
                compiled.set_script_name(script_name.clone());
            }
            // Real content gets the C++ callback arguments (no parameters;
            // AbortCall gets the last phase — C4Object.cpp:4154-4182).
            compiled.set_c4_callback_convention(true);
            compiled.set_description(definition.description.clone());
            // Legacy defs carry the FULL DefCore: apply it wholesale
            // (physicals, Float, Timer/TimerCall, Grab, fire properties,
            // ... — the hand-picked setters below only cover the DSL
            // manifest surface and silently dropped the rest).
            if let Some(core) = &definition.core {
                Engine::apply_resource_core(&mut compiled, core);
            }
            if let Some(actions) = &definition.actions {
                compiled.configure_actions(actions.default_action.clone(), actions.specs.clone());
                compiled.configure_physical_actions(actions.physical.clone());
                compiled.configure_action_reflections(actions.reflections.clone());
                compiled.configure_action_graphics(actions.graphics.clone());
            }
            // Resource-backed definitions already installed the literal
            // signed DefCore value above. Only the JSON manifest's boolean
            // surface should normalize it to 0/1.
            if definition.core.is_none() {
                compiled.set_crew_member(definition.crew_member);
            }
            compiled.set_can_be_base(definition.can_be_base);
            // DefCore shape: the spawn vertices C++ takes from the def
            // (C4Def Vertices/VertexX/...); without them every spawned
            // object compared vertex-less against the C++ snapshot.
            compiled.set_shape_rect(definition.shape.map(crate::DefinitionRect::from));
            if definition.core.is_none() {
                compiled.set_shape_vertices(
                    definition
                        .vertices
                        .iter()
                        .map(|vertex| {
                            crate::ObjectVertex::new(vertex.x, vertex.y)
                                .with_cnat(vertex.cnat)
                                .with_friction(vertex.friction)
                        })
                        .collect(),
                );
            }
            // C4Def loads ClonkNames with the configured language sequence
            // while loading the definition (C4Def.cpp:641-657). Keep that
            // frozen selection instead of rereading resources at apply time.
            compiled.set_clonk_names(definition.clonk_names.clone());
            compiled.set_movement_profile(definition.movement);
            compiled.set_category(definition.category);
            compiled.set_value(
                self.value_overloads
                    .iter()
                    .find(|(id, _)| id.eq_ignore_ascii_case(&definition.id))
                    .map(|(_, value)| *value)
                    .unwrap_or(definition.value),
            );
            compiled.set_mass(definition.mass);
            compiled.set_picture(definition.picture);
            let picture_image = definition.picture_image.as_ref().map(|image| {
                DefinitionPictureImage::from_resource(
                    image,
                    definition.picture_color_by_owner_mask.as_ref(),
                )
            });
            compiled.set_picture_image(picture_image);
            // HUD cursor-info assets: def portrait + rank symbols
            // (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-341).
            compiled.set_portrait_image(
                definition
                    .portrait_image
                    .as_ref()
                    .map(|image| DefinitionPictureImage::from_resource(image, None)),
            );
            compiled.set_portrait_graphics_image(definition.portrait_graphics_image.as_ref().map(
                |image| {
                    DefinitionPictureImage::from_resource(
                        image,
                        definition.portrait_color_by_owner_mask.as_ref(),
                    )
                },
            ));
            compiled.set_portrait_graphics(
                definition
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
            compiled.set_rank_symbols_image(
                definition
                    .rank_symbols_image
                    .as_ref()
                    .map(|image| DefinitionPictureImage::from_resource(image, None)),
            );
            compiled.set_rank_name_table(definition.rank_names.clone(), definition.rank_base);
            compiled.set_rank_symbol_count(definition.rank_symbol_count);
            let sprite_image = definition.graphics_image.as_ref().map(|image| {
                DefinitionSpriteImage::from_resource(image, definition.color_by_owner_mask.as_ref())
            });
            compiled.set_sprite_image(sprite_image);
            compiled.validate_base_graphics_rects();
            if !definition.additional_graphics.is_empty() {
                let mut variants = HashMap::with_capacity(definition.additional_graphics.len());
                for (key, variant) in &definition.additional_graphics {
                    let mask = variant.color_by_owner_mask.as_ref();
                    variants.insert(
                        key.clone(),
                        DefinitionSpriteImage::from_resource(&variant.image, mask),
                    );
                }
                compiled.set_sprite_variants(variants);
            }
            compiled.set_components(definition.components.clone());
            compiled.set_line_connect(definition.line_connect);
            engine.register_definition(compiled)?;
        }

        // C++ loads Script.c first and then the scenario-local System.c4g,
        // both after InitDefs for overload priority (C4Game.cpp:2606-2617,
        // 3336-3355). Loading is separate from the later Initialize call.
        if let Some(script) = &self.script {
            match engine.load_scenario_script_with_convention(
                &script.name,
                &script.source,
                script.c4_args,
            ) {
                Ok(()) => {}
                Err(EngineError::Script {
                    definition,
                    function,
                    source,
                    recovery: _,
                }) => {
                    tracing::warn!(
                        script = %definition,
                        function,
                        error = %source,
                        "scenario script failed to load; retaining empty host like C++"
                    );
                    // C4GameScriptHost survives a parse failure with its
                    // native full ScriptName even though no functions link.
                    engine.load_scenario_script_with_convention(
                        &script.name,
                        "",
                        script.c4_args,
                    )?;
                }
                Err(other) => return Err(other.into()),
            }
        }
        if !self.scenario_system_scripts.is_empty() {
            engine.install_scenario_global_scripts(&self.scenario_system_scripts);
        }

        // Script linking (C4Game::LinkScriptEngine -> C4AulScriptEngine::Link):
        // appends resolve FIRST, then includes (C4AulLink.cpp:27-28), and
        // `global func` declarations in definition scripts join the
        // engine-global table (AA_GLOBAL ownership).
        // Appends first, includes after: #appendto CLNK functions ARE
        // visible through include chains (live-verified: the GoldRush
        // scenario calls pObj->SetAI(...) — appended to CLNK by the
        // scenario's AI.c4d — on BNDT objects that #include COWB #include
        // CLNK, and C++ resolves it).
        engine.resolve_appends();
        engine.resolve_includes()?;
        self.rerender_initial_s2_map(engine, &mut live_post_init_map_callbacks)?;
        // register_definition already inserted each definition's global funcs
        // in load order. Re-collecting here would put them above the scenario
        // System.c4g that C++ deliberately loaded last.

        let mut pending = self.initial_spawns.clone();
        let legacy_contents_handles = pending
            .iter()
            .filter(|spawn| !spawn.contents_handles.is_empty())
            .filter_map(|spawn| {
                spawn
                    .handle
                    .clone()
                    .map(|parent| (parent, spawn.contents_handles.clone()))
            })
            .collect::<Vec<_>>();
        let legacy_contained_handles = pending
            .iter()
            .filter_map(|spawn| Some((spawn.handle.clone()?, spawn.container_handle.clone()?)))
            .collect::<Vec<_>>();
        let mut handles: HashMap<String, ObjectId> = HashMap::new();
        let mut created = Vec::with_capacity(pending.len() + 4);

        // CRITICAL: Pre-scan all spawns to find maximum explicit ID and reserve ID space
        // This prevents conflicts between auto-assigned IDs (crew members) and explicit IDs (Objects.txt)
        // Must happen BEFORE any objects are spawned to ensure crew get IDs beyond the explicit range
        let max_explicit_id = pending
            .iter()
            .filter_map(|spawn| spawn.config.id)
            .map(|id| id.as_u64())
            .max();

        if let Some(max_id) = max_explicit_id {
            // Reserve ID space: ensure next_object_id is beyond all explicit IDs
            if max_id >= engine.next_object_id {
                engine.next_object_id = max_id + 1;
            }
        }

        if self.legacy_core.is_some() {
            // C4GameObjects::Load constructs every object first and only
            // then denumerates Contained/Contents pointers. Spawn the rows
            // without runtime Enter semantics so mutual containment remains
            // representable and callbacks cannot observe a partial graph.
            for spawn in pending.drain(..) {
                let info_name = spawn.info_name;
                let mut config = spawn.config;
                config.compiler_cache.info = info_name.clone().unwrap_or_default();
                config.container = None;
                let id = engine.spawn_object(config)?;
                engine.remember_legacy_object_info(id, info_name);
                if let Some(handle) = spawn.handle {
                    if handles.insert(handle.clone(), id).is_some() {
                        return Err(ScenarioError::DuplicateHandle(handle));
                    }
                }
                created.push(id);
            }
        } else {
            while !pending.is_empty() {
                let mut progress = false;
                let mut idx = 0;
                while idx < pending.len() {
                    let ready = match &pending[idx].container_handle {
                        Some(handle) => handles.contains_key(handle),
                        None => true,
                    };

                    if !ready {
                        idx += 1;
                        continue;
                    }

                    let mut config = pending[idx].config.clone();
                    if let Some(handle) = &pending[idx].container_handle {
                        let container = *handles
                            .get(handle)
                            .ok_or_else(|| ScenarioError::UnknownContainerHandle(handle.clone()))?;
                        config = config.with_container(container);
                    }

                    let id = engine.spawn_object(config)?;
                    if let Some(handle) = &pending[idx].handle {
                        if handles.contains_key(handle) {
                            return Err(ScenarioError::DuplicateHandle(handle.clone()));
                        }
                        handles.insert(handle.clone(), id);
                    }
                    created.push(id);
                    pending.remove(idx);
                    progress = true;
                    break;
                }

                if !progress {
                    // C++ creates every object first and resolves Contained by
                    // number afterwards (denumeration): a container that never
                    // materializes — e.g. its definition was skipped — leaves
                    // the contents uncontained, never a failure.
                    let producible: HashSet<String> = pending
                        .iter()
                        .filter_map(|spawn| spawn.handle.clone())
                        .collect();
                    let mut cleared = false;
                    for spawn in pending.iter_mut() {
                        let orphaned = spawn
                            .container_handle
                            .as_deref()
                            .map(|handle| !producible.contains(handle))
                            .unwrap_or(false);
                        if orphaned {
                            tracing::warn!(
                                container = spawn.container_handle.as_deref().unwrap_or_default(),
                                "container never materialized; placing the object uncontained \
                             (C++ denumerates missing containers to null)"
                            );
                            spawn.container_handle = None;
                            cleared = true;
                        }
                    }
                    if !cleared {
                        // A genuine containment cycle: C++'s two-phase
                        // denumeration would keep the mutual containment; the
                        // sequential spawn model breaks one edge instead
                        // (documented divergence).
                        if let Some(spawn) = pending.first_mut() {
                            tracing::warn!(
                                container = spawn.container_handle.as_deref().unwrap_or_default(),
                                "containment cycle broken by placing one object uncontained \
                             (C++ keeps mutual containment via denumeration)"
                            );
                            spawn.container_handle = None;
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        if self.legacy_core.is_some() {
            let contents_orders = legacy_contents_handles
                .into_iter()
                .filter_map(|(parent, children)| {
                    handles.get(&parent).copied().map(|parent| {
                        let children = children
                            .into_iter()
                            .filter_map(|child| handles.get(&child).copied())
                            .collect();
                        (parent, children)
                    })
                })
                .collect::<Vec<_>>();
            let contained_links = legacy_contained_handles
                .into_iter()
                .filter_map(|(child, parent)| Some((*handles.get(&child)?, *handles.get(&parent)?)))
                .collect::<Vec<_>>();
            engine.restore_legacy_object_links(&contained_links, &contents_orders);
            engine.finish_legacy_object_load();
        }

        // C4Game::InitGame loads RoundResults.txt after InitControl and all
        // objects, but before script/global-effect pointer denumeration. The
        // result structure contains no object pointers of its own; installing
        // the compiled state here preserves that exact lifetime boundary.
        engine.round_results = self.round_results.clone();

        if let Some((music_enabled, output)) = initial_record_capture {
            let mut captured = engine.capture_initial_record_game_data(music_enabled);
            if let (Ok(captured), Some(loaded)) = (&mut captured, initial_network_game) {
                // InitControl saves before ScriptEngine/global-effect pointer
                // denumeration. Retain the canonical pre-resolution blocks so
                // dangling numeric wrappers are written exactly as loaded.
                captured.compiled_sections.script_engine =
                    loaded.compiled_sections.script_engine.clone();
                captured.compiled_sections.effects = loaded.compiled_sections.effects.clone();
            }
            *output = Some(captured);
        }

        if let Some(runtime) = initial_network_runtime.take() {
            let object_numbers = engine
                .objects
                .iter()
                .filter(|object| object.state.status != ObjectStatus::Deleted)
                .map(|object| object.id.as_u64())
                .collect::<HashSet<_>>();
            let (script_globals, global_effects) =
                runtime.resolve_post_object_state(&object_numbers, &self.legacy_string_table);
            engine.apply_initial_network_post_object_state(&script_globals, global_effects);
        }

        // Every surviving C4Def receives ~InitializeDef after loaded-object
        // denumeration and before the legacy environment placers run
        // (C4Game.cpp:2505-2520).
        let mut initialized = engine.initialize_definition_scripts()?;
        created.append(&mut initialized);

        // C4Game::InitGame environment placements (C4Game.cpp:2493-2503):
        // InitVegetation/InitInEarth/InitAnimals/InitEnvironment/InitRules/
        // InitGoals run after the loaded objects, gated on
        // `!C4S.Head.NoInitialize && LandscapeLoaded`, drawing from the
        // synced ledger BETWEEN the Gravity draw and Weather.Init's.
        if let Some(placement) = self
            .init_placement
            .as_ref()
            .filter(|placement| !placement.no_initialize && self.landscape.is_some())
        {
            // InitRules/InitGoals consume Game.Parameters, whose lists have
            // already passed C4SGame::ConvertGoals and may have been changed
            // by synchronized lobby parameters. Scenario.txt remains the
            // source for every other placement list (C4Game.cpp:2493-2503,
            // 4056-4076).
            let authoritative_placement = game_parameter_rule_goal_lists.map(|lists| {
                let mut placement = placement.clone();
                placement.rules = lists
                    .rules()
                    .iter()
                    .map(|entry| (entry.id.clone(), entry.count))
                    .collect();
                placement.goals = lists
                    .goals()
                    .iter()
                    .map(|entry| (entry.id.clone(), entry.count))
                    .collect();
                placement
            });
            engine
                .run_legacy_init_placements(authoritative_placement.as_ref().unwrap_or(placement));
            // C4Landscape::PostInitMap follows InitGoals inside the same
            // !NoInitialize/LandscapeLoaded block. Callback arrays execute
            // in field-registration order and each bitset in descending
            // pixel order, on the live post-FixRandom synced ledger
            // (C4Game.cpp:2493-2521; C4MapCreatorS2.cpp:49-114).
            if execute_post_init_map_callbacks {
                engine.run_post_init_map_callbacks(&live_post_init_map_callbacks)?;
            }
            if !self.keep_map_creator {
                engine.clear_runtime_map_creator();
            }
        }
        if runtime_savegame {
            // Savegames run Weather.Init(false): retain every compiled live
            // value and RNG position, but refresh the season gamma after
            // InitializeDef and placement callbacks have completed.
            engine.refresh_loaded_weather_gamma_control();
        } else if let Some(weather_init) = self.weather_init.as_ref() {
            // C4Weather::Init runs at the END of C4Game::InitGame after
            // Landscape.ScenarioInit's Gravity draw and the placements
            // (C4Game.cpp:2507).
            engine.apply_weather_init(weather_init)?;
        }
        if self
            .init_placement
            .as_ref()
            .is_some_and(|placement| !placement.save_game)
        {
            // Fresh-game InitGame tail, after Weather.Init: shipped goals
            // such as RACE include GOAL's behavior but still require the
            // separate generic timer object (C4Game.cpp:2531-2535).
            engine.ensure_legacy_goal_controller();
        }

        // C4Game::Init tail: SyncClearance + Synchronize AFTER InitGame,
        // BEFORE InitPlayers (C4Game.cpp:474-475) — collapse every fixed
        // position to itofix(x,y,r) and re-fix the synced RNG. A no-op
        // for synthetic scenarios (created spawns already satisfy both).
        engine.inherit_include_clonk_names();
        if final_synchronize {
            engine.game_start_synchronize()?;
        }
        Ok(created)
    }

    /// Applies a complete scenario with no startup players. Call
    /// [`Scenario::apply_before_players`] when startup-player joins will be
    /// executed later by the control queue.
    pub fn apply(&self, engine: &mut Engine) -> Result<Vec<ObjectId>, ScenarioError> {
        let mut created = self.apply_before_players(engine)?;
        let mut additional = engine.initialize_scenario_script()?;
        created.append(&mut additional);
        Ok(created)
    }

    fn from_manifest(group: &Group, manifest: ScenarioManifest) -> Result<Self, ScenarioError> {
        if manifest.definitions.is_empty() {
            return Err(ScenarioError::NoDefinitions);
        }

        let mut seen_ids = HashSet::new();
        let mut definitions = Vec::with_capacity(manifest.definitions.len());
        for definition in manifest.definitions {
            let DefinitionManifest {
                id,
                name,
                script,
                default_action,
                actions,
                crew_member,
                movement,
                category,
            } = definition;

            if !seen_ids.insert(id.clone()) {
                return Err(ScenarioError::DuplicateDefinition(id));
            }

            let script_path = Path::new(&script);
            let name_override = name.clone();
            let actions_override = if actions.is_empty() && default_action.is_none() {
                None
            } else {
                Some(DefinitionActions {
                    default_action,
                    specs: actions,
                    physical: Vec::new(),
                    graphics: HashMap::new(),
                    reflections: HashMap::new(),
                })
            };

            let movement_override = match movement {
                Some(manifest) => Some(manifest.into_profile(&id)?),
                None => None,
            };

            let category_override =
                category.map(|value| crate::normalize_category(value, crate::DEFAULT_CATEGORY));

            let mut base_definition = None;
            if let Some(parent) = script_path.parent() {
                if !parent.as_os_str().is_empty() {
                    match group.open_child(parent) {
                        Ok(def_group) => match ResourceDefinitionData::load(&def_group) {
                            Ok(resource) => {
                                base_definition = Some(scenario_definition_from_resource(
                                    resource,
                                    Some(def_group),
                                ));
                            }
                            Err(ResourceDefinitionError::DefCoreMissing) => {}
                            Err(ResourceDefinitionError::Resources(group_error))
                                if is_missing_group_error(&group_error) => {}
                            Err(error) => return Err(ScenarioError::Definition(error)),
                        },
                        Err(GroupError::EntryNotFound(_))
                        | Err(GroupError::Missing(_))
                        | Err(GroupError::NotDirectory(_)) => {}
                        Err(error) => return Err(ScenarioError::Resources(error)),
                    }
                }
            }

            let mut scenario_definition = if let Some(base) = base_definition {
                base
            } else {
                let script_bytes = read_group_file_bytes(group, script_path)?;
                let script_source = clonk_script::c4_string_from_bytes(&script_bytes);
                ScenarioDefinition {
                    id: id.clone(),
                    name: name_override.clone(),
                    description: None,
                    clonk_names: None,
                    script: script_source,
                    script_name: Some(
                        group
                            .root()
                            .join(script_path)
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    actions: None,
                    crew_member,
                    can_be_base: false,
                    movement: MovementProfile::default(),
                    category: category_override.unwrap_or(crate::DEFAULT_CATEGORY),
                    value: 0,
                    mass: 0,
                    picture: None,
                    picture_image: None,
                    picture_color_by_owner_mask: None,
                    graphics_image: None,
                    color_by_owner_mask: None,
                    additional_graphics: HashMap::new(),
                    portrait_image: None,
                    portrait_graphics_image: None,
                    portrait_color_by_owner_mask: None,
                    portrait_graphics: Vec::new(),
                    rank_symbols_image: None,
                    rank_names: None,
                    rank_base: None,
                    rank_symbol_count: None,
                    resource_group: None,
                    components: Vec::new(),
                    line_connect: 0,
                    vertices: Vec::new(),
                    shape: None,
                    core: None,
                }
            };

            if scenario_definition.id != id {
                scenario_definition.id = id.clone();
            }

            if let Some(name_value) = name_override {
                scenario_definition.name = Some(name_value);
            }

            if let Some(profile) = movement_override {
                scenario_definition.movement = profile;
            }

            if let Some(category_value) = category_override {
                scenario_definition.category = category_value;
            }

            if crew_member {
                scenario_definition.crew_member = true;
                if let Some(core) = scenario_definition.core.as_mut() {
                    if core.crew_member == 0 {
                        core.crew_member = 1;
                    }
                }
            }

            if let Some(actions) = actions_override {
                scenario_definition.actions = Some(actions);
            }

            definitions.push(scenario_definition);
        }

        let script = if let Some(path) = manifest.script {
            debug_assert_eq!(
                path.trim(),
                path,
                "scenario script path contains leading/trailing whitespace"
            );
            let script_path = Path::new(&path);
            let script_bytes = read_group_file_bytes(group, script_path)?;
            let script_source = clonk_script::c4_string_from_bytes(&script_bytes);
            Some(ScenarioScriptSource {
                name: group
                    .root()
                    .join(script_path)
                    .to_string_lossy()
                    .into_owned(),
                source: script_source,
                c4_args: false,
            })
        } else {
            None
        };

        let mut spawns = Vec::with_capacity(manifest.initial_objects.len());
        let mut handles = HashSet::new();
        for object in manifest.initial_objects {
            if !seen_ids.contains(&object.definition) {
                return Err(ScenarioError::UnknownDefinition(object.definition));
            }

            let ObjectManifest {
                definition,
                position,
                velocity,
                energy,
                owner,
                action,
                effects,
                crew_member,
                alive,
                status,
                handle,
                container,
                category,
            } = object;

            let mut spawn = SpawnConfig::new(definition.clone());
            if let Some(position) = position {
                spawn = spawn.with_position(Vector2::new(position[0], position[1]));
            }
            if let Some(velocity) = velocity {
                spawn = spawn.with_velocity(Vector2::new(velocity[0], velocity[1]));
            }
            if let Some(energy) = energy {
                spawn = spawn.with_energy(energy);
            }
            if let Some(owner) = owner {
                spawn = spawn.with_owner(owner);
            }
            if let Some(action) = action {
                spawn = spawn.with_action(action.into_state());
            }
            if !effects.is_empty() {
                let effect_states = effects
                    .into_iter()
                    .map(EffectManifest::into_state)
                    .collect();
                spawn = spawn.with_effects(effect_states);
            }
            let default_crew = definitions
                .iter()
                .find(|candidate| candidate.id == definition)
                .map(|definition| definition.crew_member)
                .unwrap_or(false);
            match crew_member {
                Some(value) => {
                    spawn = spawn.with_crew_member(value);
                }
                None if default_crew => {
                    spawn = spawn.with_crew_member(true);
                }
                None => {}
            }
            if let Some(alive) = alive {
                spawn = spawn.with_alive(alive);
            }
            if let Some(status) = status {
                spawn = spawn.with_status(status.into());
            }

            if let Some(category) = category {
                spawn = spawn
                    .with_category(crate::normalize_category(category, crate::DEFAULT_CATEGORY));
            }

            let handle = handle
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty());
            if let Some(handle) = handle.as_ref() {
                if !handles.insert(handle.clone()) {
                    return Err(ScenarioError::DuplicateHandle(handle.clone()));
                }
            }

            let container_handle = container
                .map(|h| h.trim().to_string())
                .filter(|h| !h.is_empty());

            spawns.push(ScenarioSpawn {
                handle,
                container_handle,
                contents_handles: Vec::new(),
                info_name: None,
                config: spawn,
            });
        }

        for spawn in &spawns {
            if let Some(container) = &spawn.container_handle {
                if !handles.contains(container) {
                    return Err(ScenarioError::UnknownContainerHandle(container.clone()));
                }
                if let Some(handle) = &spawn.handle {
                    if handle == container {
                        return Err(ScenarioError::ContainerDependencyCycle(handle.clone()));
                    }
                }
            }
        }

        let landscape = match manifest.landscape {
            Some(spec) => Some(spec.into_landscape()?),
            None => None,
        };
        let physics = match manifest.physics {
            Some(spec) => Some(spec.into_settings()?),
            None => None,
        };
        let environment = manifest.environment.map(EnvironmentManifest::into_settings);
        let sky = match manifest.sky {
            Some(spec) => Some(spec.into_config(group)?),
            None => None,
        };
        let ground_height_hint = manifest.ground_height.or_else(|| {
            landscape
                .as_ref()
                .and_then(|landscape| landscape.surface().first().copied())
        });
        let definition_load_steps = definitions
            .iter()
            .map(|definition| DefinitionLoadStep::Definition(definition.id.clone()))
            .collect();

        Ok(Self {
            legacy_core: None,
            legacy_team_metadata: None,
            name: manifest.name,
            description: manifest.description,
            ticks: manifest.ticks,
            ground_height_hint,
            material_library: None,
            definitions,
            value_overloads: Vec::new(),
            initial_spawns: spawns,
            landscape,
            post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks::default(),
            keep_map_creator: false,
            scenario_sections: Vec::new(),
            physics,
            runtime_landscape: None,
            legacy_string_table: clonk_script::new_string_registrations(),
            round_results: RoundResultsState::default(),
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            environment,
            weather_init: None,
            sky,
            script,
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: false,
            base_sell_enabled: false,
            base_auto_sell_enabled: false,
            base_reject_entrance_enabled: true,
            base_regenerate_energy_enabled: true,
            base_extinguish_enabled: true,
            base_regenerate_energy_price: BASE_REGENERATE_ENERGY_PRICE,
            landscape_insert_thrust: false,
            disable_mouse: false,
            forced_auto_context_menu: None,
            forced_control_style: None,
            definition_load_steps,
            definition_resource_paths: Vec::new(),
            definition_root_groups: Vec::new(),
            sound_effect_groups: Vec::new(),
            scenario_system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
            teams: Vec::new(),
            lobby_metadata: None,
            standard_names: None,
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            init_placement: None,
        })
    }
}

struct LegacyScenarioManifest {
    title: Option<String>,
    description: Option<String>,
    /// Exact `[Head] Title` bytes after the Scenario.txt compiler's RCT_All
    /// leading-space handling. Group-backed loads always populate this; direct
    /// string fixtures leave it absent.
    head_title_native: Option<LegacyCString>,
    definition_specs: Vec<String>,
    ground_height_hint: Option<i32>,
    core: LegacyScenarioCore,
    sections: HashMap<String, Vec<(String, String)>>,
}

const BASEFUNC_AUTO_SELL_CONTENTS: i32 = 1 << 0;
const BASEFUNC_REGENERATE_ENERGY: i32 = 1 << 1;
const BASEFUNC_BUY: i32 = 1 << 2;
const BASEFUNC_SELL: i32 = 1 << 3;
const BASEFUNC_REJECT_ENTRANCE: i32 = 1 << 4;
const BASEFUNC_EXTINGUISH: i32 = 1 << 5;
const BASEFUNC_DEFAULT: i32 = 0xffff;
const BASE_REGENERATE_ENERGY_PRICE: i32 = 5;
const DEFAULT_FOW_RESOLUTION: i32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyIdEntry {
    id: String,
    count: Option<i32>,
}

type LegacyIdList = Vec<LegacyIdEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyNameEntry {
    name: String,
    count: Option<i32>,
}

type LegacyNameList = Vec<LegacyNameEntry>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyScenarioCore {
    head: LegacyHead,
    definitions: LegacyDefinitions,
    game: LegacyGame,
    players: Vec<LegacyPlayer>,
    landscape: LegacyLandscape,
    weather: LegacyWeather,
    disasters: LegacyDisasters,
    animals: LegacyAnimals,
    environment: LegacyEnvironment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyHead {
    icon: i32,
    title: String,
    loader: String,
    font: String,
    version: [i32; 5],
    difficulty: i32,
    max_player: i32,
    max_player_league: i32,
    min_player: i32,
    save_game: i32,
    replay: i32,
    film: i32,
    disable_mouse: i32,
    no_initialize: i32,
    random_seed: i32,
    forced_auto_context_menu: i32,
    forced_control_style: i32,
    engine: String,
    mission_access: String,
    network_game: bool,
    network_runtime_join: bool,
    forced_gfx_mode: i32,
    forced_fair_crew: i32,
    fair_crew_strength: i32,
    origin: Option<String>,
}

impl Default for LegacyHead {
    fn default() -> Self {
        Self {
            icon: 18,
            title: "Default Title".to_string(),
            loader: String::new(),
            font: String::new(),
            version: [0; 5],
            difficulty: 0,
            max_player: 12,
            max_player_league: 12,
            min_player: 0,
            save_game: 0,
            replay: 0,
            film: 0,
            disable_mouse: 0,
            no_initialize: 0,
            random_seed: 0,
            forced_auto_context_menu: -1,
            forced_control_style: -1,
            engine: String::new(),
            mission_access: String::new(),
            network_game: false,
            network_runtime_join: false,
            forced_gfx_mode: 0,
            forced_fair_crew: 0,
            fair_crew_strength: 0,
            origin: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyDefinitions {
    local_only: bool,
    allow_user_change: bool,
    definitions: Vec<String>,
    /// Exact strings retained in C4Scenario for StdCompiler reflection.
    /// `definitions` remains path-normalized for the Rust resource resolver.
    reflected_definitions: Option<Vec<String>>,
    skip_defs: LegacyIdList,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyRealism {
    construction_needs_material: bool,
    structures_need_energy: bool,
    value_overloads: LegacyIdList,
    landscape_push_pull: i32,
    landscape_insert_thrust: i32,
    base_functionality: i32,
    base_regenerate_energy_price: i32,
}

impl Default for LegacyRealism {
    fn default() -> Self {
        Self {
            construction_needs_material: false,
            structures_need_energy: true,
            value_overloads: Vec::new(),
            landscape_push_pull: 0,
            landscape_insert_thrust: 0,
            base_functionality: BASEFUNC_DEFAULT,
            base_regenerate_energy_price: BASE_REGENERATE_ENERGY_PRICE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyGame {
    mode: i32,
    elimination: i32,
    cooperative_goal: i32,
    create_objects: LegacyIdList,
    clear_objects: LegacyIdList,
    clear_materials: LegacyNameList,
    value_gain: i32,
    enable_remove_flag: bool,
    realism: LegacyRealism,
    goals: LegacyIdList,
    rules: LegacyIdList,
    fow_color: u32,
}

impl Default for LegacyGame {
    fn default() -> Self {
        Self {
            mode: 0,
            elimination: 1,
            cooperative_goal: 0,
            create_objects: Vec::new(),
            clear_objects: Vec::new(),
            clear_materials: Vec::new(),
            value_gain: 0,
            enable_remove_flag: false,
            realism: LegacyRealism::default(),
            goals: Vec::new(),
            rules: Vec::new(),
            fow_color: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyPlayer {
    standard_crew: Option<String>,
    clonks: LegacyC4SVal,
    wealth: LegacyC4SVal,
    position: [i32; 2],
    enforce_position: i32,
    crew: LegacyIdList,
    buildings: LegacyIdList,
    vehicles: LegacyIdList,
    material: LegacyIdList,
    knowledge: LegacyIdList,
    home_base_material: LegacyIdList,
    home_base_production: LegacyIdList,
    magic: LegacyIdList,
}

impl Default for LegacyPlayer {
    fn default() -> Self {
        Self {
            standard_crew: None,
            clonks: LegacyC4SVal::new(1, 0, 1, 10),
            wealth: LegacyC4SVal::new(0, 0, 0, 250),
            position: [-1, -1],
            enforce_position: 0,
            crew: Vec::new(),
            buildings: Vec::new(),
            vehicles: Vec::new(),
            material: Vec::new(),
            knowledge: Vec::new(),
            home_base_material: Vec::new(),
            home_base_production: Vec::new(),
            magic: Vec::new(),
        }
    }
}

/// `C4S_MaxPlayer` (C4Scenario.h): four `[PlayerN]` start slots; a joining
/// player uses slot `Number % C4S_MaxPlayer` (C4Player.cpp:673).
pub const MAX_PLAYER_STARTS: usize = 4;

/// One `C4SPlrStart` slot (compiled at C4Scenario.cpp:276-291), retained
/// after `Scenario::apply` because `C4Player::ScenarioInit`
/// (C4Player.cpp:670-777) consumes it at JOIN time, not load time. ID lists
/// keep their file order — placement iterates them in order, drawing from
/// the synced RNG per entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStart {
    /// `StandardCrew` (old crew spec; C4ID_None when absent).
    pub native_crew: Option<String>,
    /// `Clonks` — the old-spec crew COUNT C4SVal.
    pub crew_count: LegacyC4SVal,
    /// `Wealth`.
    pub wealth: LegacyC4SVal,
    /// `Position` (map coordinates; -1 = unset).
    pub position: [i32; 2],
    /// `EnforcePosition`.
    pub enforce_position: bool,
    /// `Crew` — the new-spec ready-crew ID list.
    pub ready_crew: Vec<(String, i32)>,
    /// `Buildings`.
    pub ready_base: Vec<(String, i32)>,
    /// `Vehicles`.
    pub ready_vehic: Vec<(String, i32)>,
    /// `Material`.
    pub ready_material: Vec<(String, i32)>,
    /// `Knowledge`.
    pub build_knowledge: Vec<(String, i32)>,
    /// `HomeBaseMaterial`.
    pub home_base_material: Vec<(String, i32)>,
    /// `HomeBaseProduction`.
    pub home_base_production: Vec<(String, i32)>,
    /// `Magic`.
    pub magic: Vec<(String, i32)>,
}

impl Default for PlayerStart {
    fn default() -> Self {
        PlayerStart::from_legacy(&LegacyPlayer::default())
    }
}

impl PlayerStart {
    fn from_legacy(player: &LegacyPlayer) -> Self {
        // C4IDList::Entry starts at count zero and only compiles a count when
        // the textual entry has an `=` separator (C4IDList.cpp:239-253).
        let id_list = |entries: &LegacyIdList| {
            entries
                .iter()
                .map(|entry| (entry.id.clone(), entry.count.unwrap_or(0)))
                .collect()
        };
        Self {
            native_crew: player.standard_crew.clone(),
            crew_count: player.clonks,
            wealth: player.wealth,
            position: player.position,
            enforce_position: player.enforce_position != 0,
            ready_crew: id_list(&player.crew),
            ready_base: id_list(&player.buildings),
            ready_vehic: id_list(&player.vehicles),
            ready_material: id_list(&player.material),
            build_knowledge: id_list(&player.knowledge),
            home_base_material: id_list(&player.home_base_material),
            home_base_production: id_list(&player.home_base_production),
            magic: id_list(&player.magic),
        }
    }

    fn slots_from_legacy(players: &[LegacyPlayer]) -> Vec<PlayerStart> {
        (0..MAX_PLAYER_STARTS)
            .map(|index| {
                players
                    .get(index)
                    .map(PlayerStart::from_legacy)
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyLandscape {
    exact_landscape: bool,
    vegetation: LegacyIdList,
    vegetation_level: LegacyC4SVal,
    in_earth: LegacyIdList,
    in_earth_level: LegacyC4SVal,
    sky: Option<String>,
    sky_fade: [i32; 6],
    no_sky: bool,
    bottom_open: bool,
    top_open: bool,
    left_open: i32,
    right_open: i32,
    auto_scan_side_open: bool,
    map_width: LegacyC4SVal,
    map_height: LegacyC4SVal,
    map_zoom: LegacyC4SVal,
    amplitude: LegacyC4SVal,
    phase: LegacyC4SVal,
    period: LegacyC4SVal,
    random: LegacyC4SVal,
    material: String,
    liquid: String,
    liquid_level: LegacyC4SVal,
    map_player_extend: bool,
    layers: LegacyNameList,
    gravity: LegacyC4SVal,
    no_scan: bool,
    keep_map_creator: bool,
    sky_scroll_mode: i32,
    new_style_landscape: i32,
    fow_resolution: i32,
    shade_materials: bool,
}

impl Default for LegacyLandscape {
    fn default() -> Self {
        Self {
            exact_landscape: false,
            vegetation: Vec::new(),
            vegetation_level: LegacyC4SVal::new(50, 30, 0, 100),
            in_earth: Vec::new(),
            in_earth_level: LegacyC4SVal::new(50, 0, 0, 100),
            sky: None,
            sky_fade: [0; 6],
            no_sky: false,
            bottom_open: false,
            top_open: true,
            left_open: 0,
            right_open: 0,
            auto_scan_side_open: true,
            map_width: LegacyC4SVal::new(100, 0, 64, 250),
            map_height: LegacyC4SVal::new(50, 0, 40, 250),
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            amplitude: LegacyC4SVal::new(0, 0, 0, 100),
            phase: LegacyC4SVal::new(50, 0, 0, 100),
            period: LegacyC4SVal::new(15, 0, 0, 100),
            random: LegacyC4SVal::new(0, 0, 0, 100),
            material: "Earth".to_string(),
            liquid: "Water".to_string(),
            liquid_level: LegacyC4SVal::new(0, 0, 0, 100),
            map_player_extend: false,
            layers: Vec::new(),
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            no_scan: false,
            keep_map_creator: false,
            sky_scroll_mode: 0,
            new_style_landscape: 0,
            fow_resolution: DEFAULT_FOW_RESOLUTION,
            shade_materials: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyWeather {
    climate: LegacyC4SVal,
    start_season: LegacyC4SVal,
    year_speed: LegacyC4SVal,
    rain: LegacyC4SVal,
    wind: LegacyC4SVal,
    lightning: LegacyC4SVal,
    precipitation: String,
    no_gamma: bool,
}

impl Default for LegacyWeather {
    fn default() -> Self {
        Self {
            climate: LegacyC4SVal::new(50, 10, 0, 100),
            start_season: LegacyC4SVal::new(50, 50, 0, 100),
            year_speed: LegacyC4SVal::new(50, 0, 0, 100),
            rain: LegacyC4SVal::new(0, 0, 0, 100),
            wind: LegacyC4SVal::new(0, 70, -100, 100),
            lightning: LegacyC4SVal::new(0, 0, 0, 100),
            precipitation: "Water".to_string(),
            no_gamma: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyDisasters {
    meteorite: LegacyC4SVal,
    volcano: LegacyC4SVal,
    earthquake: LegacyC4SVal,
}

impl Default for LegacyDisasters {
    fn default() -> Self {
        Self {
            meteorite: LegacyC4SVal::new(0, 0, 0, 100),
            volcano: LegacyC4SVal::new(0, 0, 0, 100),
            earthquake: LegacyC4SVal::new(0, 0, 0, 100),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyAnimals {
    free_life: LegacyIdList,
    earth_nest: LegacyIdList,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct LegacyEnvironment {
    objects: LegacyIdList,
}

/// One primitive exposed by `GetValByStdCompiler` while reflecting
/// `Game.C4S` (`C4Script.cpp:3997-4148,4244-4250`).  Keep this distinct from
/// `clonk_script::Value`: scenario loading must not know about VM ownership or
/// string interning, and the host boundary performs the final conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ScenarioValue {
    Int(i32),
    Bool(bool),
    String(String),
    C4Id(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScenarioValueEntry {
    pub(crate) name: String,
    pub(crate) values: Vec<ScenarioValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ScenarioValueSection {
    pub(crate) name: String,
    pub(crate) entries: Vec<ScenarioValueEntry>,
}

/// The fully defaulted `C4Scenario::CompileFunc` view retained at runtime for
/// `GetScenarioVal`.  Values stay in compiler traversal order because
/// `C4ValueGetCompiler` treats `entry_nr` as an index over primitive callbacks
/// (C4Script.cpp:3997-4006), including alternating ID/count list entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ScenarioValueStore {
    pub(crate) sections: Vec<ScenarioValueSection>,
    #[serde(default)]
    core: LegacyScenarioCore,
    #[serde(default)]
    section_head_defaults: Option<[i32; 2]>,
}

impl Default for ScenarioValueStore {
    fn default() -> Self {
        let mut core = LegacyScenarioCore::default();
        // C4Scenario's main-file compiler default differs from
        // C4SRealism::Default (C4Scenario.cpp:237-238).
        core.game.realism.landscape_insert_thrust = 1;
        Self::from_runtime_core(&core, false)
    }
}

impl ScenarioValueStore {
    fn with_section_head_defaults(mut self, context: &LegacyHead) -> Self {
        self.section_head_defaults = Some([
            context.forced_auto_context_menu,
            context.forced_control_style,
        ]);
        self
    }

    pub(crate) fn serialize_runtime_network_save(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.core
            .runtime_network_save(
                scenario_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .serialize()
    }

    /// `C4GameSaveScenario`'s non-initial Scenario.txt rewrite.  Unlike an
    /// exact save this deliberately keeps the scenario's own title,
    /// definition list and Origin while clearing only the fields changed by
    /// the common `C4GameSave::SaveCore` path.
    pub(crate) fn serialize_runtime_scenario_save(&self) -> Vec<u8> {
        self.core.runtime_scenario_save().serialize()
    }

    /// `C4GameSaveSavegame`'s non-initial Scenario.txt rewrite.  The caller
    /// supplies the already-derived icon because the native specialization
    /// obtains it from the destination group's trailing slot number.
    pub(crate) fn serialize_runtime_savegame(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
        icon: i32,
    ) -> Vec<u8> {
        self.core
            .runtime_savegame(
                scenario_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
                icon,
            )
            .serialize()
    }

    /// Non-initial `C4GameSaveRecord` uses the synchronized exact-save core,
    /// then marks the scenario as a replay with the fixed record icon.
    pub(crate) fn serialize_runtime_record_save(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.core
            .runtime_record_save(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .serialize()
    }

    /// Initial-record projection of an already-restored exact savegame.
    /// The runtime store, rather than the source `Scenario`, owns mutations
    /// made before the JSON save was written.
    pub(crate) fn serialize_initial_record_from_runtime_savegame(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Vec<u8> {
        self.core
            .runtime_exact_save_core(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .initial_record_save(
                record_title,
                definition_modules,
                definition_executable_path,
                definition_path,
                scenario_origin,
            )
            .serialize()
    }

    pub(crate) fn serialize_section_save(&self, force_exact: bool) -> Vec<u8> {
        self.core
            .serialize_section(force_exact, self.section_head_defaults)
    }

    pub(crate) fn no_sky(&self) -> bool {
        self.core.landscape.no_sky
    }

    #[cfg(test)]
    pub(crate) fn with_film_for_test(film: i32) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.head.film = film;
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_replay_film_for_test(replay: i32, film: i32) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.head.replay = replay;
        core.head.film = film;
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_value_gain_for_test(value_gain: i32) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.game.value_gain = value_gain;
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_landscape_push_pull_for_test(enabled: bool) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.game.realism.landscape_push_pull = i32::from(enabled);
        Self::from_runtime_core(&core, false)
    }

    #[cfg(test)]
    pub(crate) fn with_no_sky_for_test(enabled: bool) -> Self {
        let mut core = LegacyScenarioCore::default();
        core.landscape.no_sky = enabled;
        Self::from_runtime_core(&core, false)
    }

    fn entry(name: &'static str, values: Vec<ScenarioValue>) -> ScenarioValueEntry {
        ScenarioValueEntry {
            name: name.to_string(),
            values,
        }
    }

    fn ints(values: impl IntoIterator<Item = i32>) -> Vec<ScenarioValue> {
        values.into_iter().map(ScenarioValue::Int).collect()
    }

    fn trimmed_ints<const N: usize>(values: [i32; N], default: i32) -> Vec<ScenarioValue> {
        let len = values
            .iter()
            .rposition(|value| *value != default)
            .map_or(0, |index| index + 1);
        Self::ints(values.into_iter().take(len))
    }

    fn c4s(value: LegacyC4SVal) -> Vec<ScenarioValue> {
        Self::ints([value.std, value.rnd, value.min, value.max])
    }

    fn c4id(value: &str) -> ScenarioValue {
        if value.len() != 4 || value == "NONE" || value == "0000" {
            ScenarioValue::C4Id(String::new())
        } else {
            ScenarioValue::C4Id(value.to_string())
        }
    }

    fn ids(values: &LegacyIdList) -> Vec<ScenarioValue> {
        values
            .iter()
            .flat_map(|entry| {
                [
                    Self::c4id(&entry.id),
                    ScenarioValue::Int(entry.count.unwrap_or(0)),
                ]
            })
            .collect()
    }

    fn names(values: &LegacyNameList) -> Vec<ScenarioValue> {
        values
            .iter()
            .flat_map(|entry| {
                [
                    ScenarioValue::String(entry.name.clone()),
                    ScenarioValue::Int(entry.count.unwrap_or(0)),
                ]
            })
            .collect()
    }

    fn from_core(core: &LegacyScenarioCore) -> Self {
        let head = &core.head;
        let mut sections = vec![ScenarioValueSection {
            name: "Head".to_string(),
            entries: vec![
                Self::entry("Icon", Self::ints([head.icon])),
                Self::entry("Title", vec![ScenarioValue::String(head.title.clone())]),
                Self::entry("Loader", vec![ScenarioValue::String(head.loader.clone())]),
                Self::entry("Font", vec![ScenarioValue::String(head.font.clone())]),
                Self::entry("Version", Self::trimmed_ints(head.version, 0)),
                Self::entry("Difficulty", Self::ints([head.difficulty])),
                // C4SHead::CompileFunc reflects a local, permanently-zero
                // compatibility value for the obsolete Access entry.
                Self::entry("Access", Self::ints([0])),
                Self::entry("MaxPlayer", Self::ints([head.max_player])),
                Self::entry("MaxPlayerLeague", Self::ints([head.max_player_league])),
                Self::entry("MinPlayer", Self::ints([head.min_player])),
                Self::entry("SaveGame", Self::ints([head.save_game])),
                Self::entry("Replay", Self::ints([head.replay])),
                Self::entry("Film", Self::ints([head.film])),
                Self::entry("DisableMouse", Self::ints([head.disable_mouse])),
                Self::entry("NoInitialize", Self::ints([head.no_initialize])),
                Self::entry("RandomSeed", Self::ints([head.random_seed])),
                Self::entry(
                    "ForcedAutoContextMenu",
                    Self::ints([head.forced_auto_context_menu]),
                ),
                Self::entry(
                    "ForcedAutoStopControl",
                    Self::ints([head.forced_control_style]),
                ),
                Self::entry("Engine", vec![ScenarioValue::String(head.engine.clone())]),
                Self::entry(
                    "MissionAccess",
                    vec![ScenarioValue::String(head.mission_access.clone())],
                ),
                Self::entry("NetworkGame", vec![ScenarioValue::Bool(head.network_game)]),
                Self::entry(
                    "NetworkRuntimeJoin",
                    vec![ScenarioValue::Bool(head.network_runtime_join)],
                ),
                Self::entry("ForcedGfxMode", Self::ints([head.forced_gfx_mode])),
                Self::entry("ForcedNoCrew", Self::ints([head.forced_fair_crew])),
                Self::entry("DefCrewStrength", Self::ints([head.fair_crew_strength])),
                Self::entry(
                    "Origin",
                    vec![ScenarioValue::String(
                        head.origin.clone().unwrap_or_default(),
                    )],
                ),
            ],
        }];

        let definitions = &core.definitions;
        sections.push(ScenarioValueSection {
            name: "Definitions".to_string(),
            entries: vec![
                Self::entry(
                    "LocalOnly",
                    vec![ScenarioValue::Bool(definitions.local_only)],
                ),
                Self::entry(
                    "AllowUserChange",
                    vec![ScenarioValue::Bool(definitions.allow_user_change)],
                ),
                Self::entry(
                    "Definitions",
                    definitions
                        .reflected_definitions
                        .as_ref()
                        .unwrap_or(&definitions.definitions)
                        .iter()
                        .cloned()
                        .map(ScenarioValue::String)
                        .collect(),
                ),
                Self::entry("SkipDefs", Self::ids(&definitions.skip_defs)),
            ],
        });

        let game = &core.game;
        sections.push(ScenarioValueSection {
            name: "Game".to_string(),
            entries: vec![
                Self::entry("Mode", Self::ints([game.mode])),
                Self::entry("Elimination", Self::ints([game.elimination])),
                Self::entry("CooperativeGoal", Self::ints([game.cooperative_goal])),
                Self::entry("CreateObjects", Self::ids(&game.create_objects)),
                Self::entry("ClearObjects", Self::ids(&game.clear_objects)),
                Self::entry("ClearMaterials", Self::names(&game.clear_materials)),
                Self::entry("ValueGain", Self::ints([game.value_gain])),
                Self::entry(
                    "EnableRemoveFlag",
                    vec![ScenarioValue::Bool(game.enable_remove_flag)],
                ),
                Self::entry(
                    "StructNeedMaterial",
                    vec![ScenarioValue::Bool(
                        game.realism.construction_needs_material,
                    )],
                ),
                Self::entry(
                    "StructNeedEnergy",
                    vec![ScenarioValue::Bool(game.realism.structures_need_energy)],
                ),
                Self::entry("ValueOverloads", Self::ids(&game.realism.value_overloads)),
                Self::entry(
                    "LandscapePushPull",
                    Self::ints([game.realism.landscape_push_pull]),
                ),
                Self::entry(
                    "LandscapeInsertThrust",
                    Self::ints([game.realism.landscape_insert_thrust]),
                ),
                Self::entry(
                    "BaseFunctionality",
                    Self::ints([game.realism.base_functionality]),
                ),
                Self::entry(
                    "BaseRegenerateEnergyPrice",
                    Self::ints([game.realism.base_regenerate_energy_price]),
                ),
                Self::entry("Goals", Self::ids(&game.goals)),
                Self::entry("Rules", Self::ids(&game.rules)),
                Self::entry("FoWColor", Self::ints([game.fow_color as i32])),
            ],
        });

        for index in 0..MAX_PLAYER_STARTS {
            let player = core.players.get(index).cloned().unwrap_or_default();
            sections.push(ScenarioValueSection {
                name: format!("Player{}", index + 1),
                entries: vec![
                    Self::entry(
                        "StandardCrew",
                        vec![Self::c4id(
                            player.standard_crew.as_deref().unwrap_or_default(),
                        )],
                    ),
                    Self::entry("Clonks", Self::c4s(player.clonks)),
                    Self::entry("Wealth", Self::c4s(player.wealth)),
                    Self::entry("Position", Self::trimmed_ints(player.position, -1)),
                    Self::entry("EnforcePosition", Self::ints([player.enforce_position])),
                    Self::entry("Crew", Self::ids(&player.crew)),
                    Self::entry("Buildings", Self::ids(&player.buildings)),
                    Self::entry("Vehicles", Self::ids(&player.vehicles)),
                    Self::entry("Material", Self::ids(&player.material)),
                    Self::entry("Knowledge", Self::ids(&player.knowledge)),
                    Self::entry("HomeBaseMaterial", Self::ids(&player.home_base_material)),
                    Self::entry(
                        "HomeBaseProduction",
                        Self::ids(&player.home_base_production),
                    ),
                    Self::entry("Magic", Self::ids(&player.magic)),
                ],
            });
        }

        let landscape = &core.landscape;
        sections.push(ScenarioValueSection {
            name: "Landscape".to_string(),
            entries: vec![
                Self::entry(
                    "ExactLandscape",
                    vec![ScenarioValue::Bool(landscape.exact_landscape)],
                ),
                Self::entry("Vegetation", Self::ids(&landscape.vegetation)),
                Self::entry("VegetationLevel", Self::c4s(landscape.vegetation_level)),
                Self::entry("InEarth", Self::ids(&landscape.in_earth)),
                Self::entry("InEarthLevel", Self::c4s(landscape.in_earth_level)),
                Self::entry(
                    "Sky",
                    vec![ScenarioValue::String(
                        landscape.sky.clone().unwrap_or_default(),
                    )],
                ),
                Self::entry("SkyFade", Self::trimmed_ints(landscape.sky_fade, 0)),
                Self::entry("NoSky", vec![ScenarioValue::Bool(landscape.no_sky)]),
                Self::entry(
                    "BottomOpen",
                    vec![ScenarioValue::Bool(landscape.bottom_open)],
                ),
                Self::entry("TopOpen", vec![ScenarioValue::Bool(landscape.top_open)]),
                Self::entry("LeftOpen", Self::ints([landscape.left_open])),
                Self::entry("RightOpen", Self::ints([landscape.right_open])),
                Self::entry(
                    "AutoScanSideOpen",
                    vec![ScenarioValue::Bool(landscape.auto_scan_side_open)],
                ),
                Self::entry("MapWidth", Self::c4s(landscape.map_width)),
                Self::entry("MapHeight", Self::c4s(landscape.map_height)),
                Self::entry("MapZoom", Self::c4s(landscape.map_zoom)),
                Self::entry("Amplitude", Self::c4s(landscape.amplitude)),
                Self::entry("Phase", Self::c4s(landscape.phase)),
                Self::entry("Period", Self::c4s(landscape.period)),
                Self::entry("Random", Self::c4s(landscape.random)),
                Self::entry(
                    "Material",
                    vec![ScenarioValue::String(landscape.material.clone())],
                ),
                Self::entry(
                    "Liquid",
                    vec![ScenarioValue::String(landscape.liquid.clone())],
                ),
                Self::entry("LiquidLevel", Self::c4s(landscape.liquid_level)),
                Self::entry(
                    "MapPlayerExtend",
                    vec![ScenarioValue::Bool(landscape.map_player_extend)],
                ),
                Self::entry("Layers", Self::names(&landscape.layers)),
                Self::entry("Gravity", Self::c4s(landscape.gravity)),
                Self::entry("NoScan", vec![ScenarioValue::Bool(landscape.no_scan)]),
                Self::entry(
                    "KeepMapCreator",
                    vec![ScenarioValue::Bool(landscape.keep_map_creator)],
                ),
                Self::entry("SkyScrollMode", Self::ints([landscape.sky_scroll_mode])),
                Self::entry(
                    "NewStyleLandscape",
                    Self::ints([landscape.new_style_landscape]),
                ),
                Self::entry("FoWRes", Self::ints([landscape.fow_resolution])),
                Self::entry(
                    "ShadeMaterials",
                    vec![ScenarioValue::Bool(landscape.shade_materials)],
                ),
            ],
        });

        sections.push(ScenarioValueSection {
            name: "Animals".to_string(),
            entries: vec![
                Self::entry("Animal", Self::ids(&core.animals.free_life)),
                Self::entry("Nest", Self::ids(&core.animals.earth_nest)),
            ],
        });

        let weather = &core.weather;
        sections.push(ScenarioValueSection {
            name: "Weather".to_string(),
            entries: vec![
                Self::entry("Climate", Self::c4s(weather.climate)),
                Self::entry("StartSeason", Self::c4s(weather.start_season)),
                Self::entry("YearSpeed", Self::c4s(weather.year_speed)),
                Self::entry("Rain", Self::c4s(weather.rain)),
                Self::entry("Wind", Self::c4s(weather.wind)),
                Self::entry("Lightning", Self::c4s(weather.lightning)),
                Self::entry(
                    "Precipitation",
                    vec![ScenarioValue::String(weather.precipitation.clone())],
                ),
                Self::entry("NoGamma", vec![ScenarioValue::Bool(weather.no_gamma)]),
            ],
        });

        sections.push(ScenarioValueSection {
            name: "Disasters".to_string(),
            entries: vec![
                Self::entry("Meteorite", Self::c4s(core.disasters.meteorite)),
                Self::entry("Volcano", Self::c4s(core.disasters.volcano)),
                Self::entry("Earthquake", Self::c4s(core.disasters.earthquake)),
            ],
        });

        sections.push(ScenarioValueSection {
            name: "Environment".to_string(),
            entries: vec![Self::entry("Objects", Self::ids(&core.environment.objects))],
        });

        Self {
            sections,
            core: core.clone(),
            section_head_defaults: None,
        }
    }

    /// Project the state visible to scripts after C4Scenario::Load,
    /// ConvertGoals, and the initial C4Landscape/C4Sky initialization, which
    /// all precede scenario `Initialize` (C4Scenario.cpp:86-97;
    /// C4Landscape.cpp:569-570,677; C4Sky.cpp:84-91).
    fn from_runtime_core(core: &LegacyScenarioCore, has_sky_surface: bool) -> Self {
        let mut runtime = core.after_load_conversion();
        runtime.landscape.map_width.max = 10_000;
        runtime.landscape.map_height.max = 10_000;
        runtime.landscape.new_style_landscape = 2;
        if !has_sky_surface {
            runtime.landscape.sky = runtime.landscape.sky.map(|sky| sky.replace(',', ";"));
        }
        Self::from_core(&runtime)
    }

    /// `C4SGame::IsMelee`: inspect the post-ConvertGoals C4IDList and use
    /// the first exact MELE/MEL2 entry's count for each id.
    pub(crate) fn is_melee(&self) -> bool {
        let goals = self
            .sections
            .iter()
            .find(|section| section.name == "Game")
            .and_then(|section| section.entries.iter().find(|entry| entry.name == "Goals"))
            .map(|entry| entry.values.as_slice())
            .unwrap_or_default();

        ["MELE", "MEL2"].into_iter().any(|wanted| {
            goals
                .chunks(2)
                .find_map(|pair| {
                    let ScenarioValue::C4Id(id) = pair.first()? else {
                        return None;
                    };
                    (id == wanted).then(|| {
                        pair.get(1)
                            .and_then(|value| match value {
                                ScenarioValue::Int(count) => Some(*count),
                                _ => None,
                            })
                            .unwrap_or(0)
                    })
                })
                .is_some_and(|count| count != 0)
        })
    }

    pub(crate) fn landscape_push_pull(&self) -> bool {
        matches!(
            self.get("LandscapePushPull", Some("Game"), 0),
            Some(ScenarioValue::Int(value)) if *value != 0
        )
    }

    /// Runtime `Game.C4S.Game.FoWColor`, retaining the packed unsigned C4
    /// color bits even though `GetScenarioVal` exposes the primitive as an
    /// `int32_t`.
    pub(crate) fn fow_color(&self) -> u32 {
        match self.get("FoWColor", Some("Game"), 0) {
            Some(ScenarioValue::Int(value)) => *value as u32,
            _ => 0,
        }
    }

    /// Runtime `Game.C4S.Landscape.FoWRes`. The fully defaulted scenario
    /// compiler stores `CClrModAddMap::iDefResolutionX` (64) here.
    pub(crate) fn fow_resolution(&self) -> i32 {
        match self.get("FoWRes", Some("Landscape"), 0) {
            Some(ScenarioValue::Int(value)) => *value,
            _ => crate::DEFAULT_FOW_RESOLUTION,
        }
    }

    pub(crate) fn scenario_title(&self) -> &str {
        match self.get("Title", Some("Head"), 0) {
            Some(ScenarioValue::String(title)) => title,
            _ => "",
        }
    }

    /// Mirrors C4ValueGetCompiler's traversal: with no section, same-name
    /// fields in successive sections contribute to one primitive stream;
    /// with a section, only that named C4Scenario child is traversed.
    pub(crate) fn get(
        &self,
        entry: &str,
        section: Option<&str>,
        entry_nr: i32,
    ) -> Option<&ScenarioValue> {
        let mut remaining = usize::try_from(entry_nr).ok()?;
        for candidate in self
            .sections
            .iter()
            .filter(|candidate| section.is_none_or(|name| candidate.name == name))
        {
            // In the one-name form a root section with the requested name
            // becomes the active match. Its named children are then one
            // level too deep for haveCompleteMatch(), so a same-name child
            // (notably [Definitions].Definitions) is shadowed rather than
            // returned (C4Script.cpp:3958-3989).
            if section.is_none() && candidate.name == entry {
                continue;
            }
            for field in candidate.entries.iter().filter(|field| field.name == entry) {
                if remaining < field.values.len() {
                    return field.values.get(remaining);
                }
                remaining -= field.values.len();
            }
        }
        None
    }
}

fn parse_bool_field(field: &str, raw: &str) -> Result<bool, ScenarioError> {
    if let Some(value) = parse_legacy_bool(raw) {
        return Ok(value);
    }
    match parse_i32(raw) {
        Ok(value) => Ok(value != 0),
        Err(err) => Err(ScenarioError::LegacyParse(format!(
            "invalid boolean for `{field}`: {err}"
        ))),
    }
}

/// C4IDList entries as (id, count) pairs — a bare id compiles count 0
/// (mkDefaultAdapt(count, 0), C4IDList.cpp:252).
fn id_list_pairs(list: &LegacyIdList) -> Vec<(String, i32)> {
    list.iter()
        .map(|entry| (entry.id.clone(), entry.count.unwrap_or(0)))
        .collect()
}

fn scenario_id_list_entries(list: &LegacyIdList) -> Vec<ScenarioIdListEntry> {
    list.iter()
        .map(|entry| ScenarioIdListEntry::new(entry.id.clone(), entry.count.unwrap_or(0)))
        .collect()
}

fn set_legacy_id_count(list: &mut LegacyIdList, id: &str, count: i32) {
    if let Some(entry) = list.iter_mut().find(|entry| entry.id == id) {
        entry.count = Some(count);
    } else {
        list.push(LegacyIdEntry {
            id: id.to_owned(),
            count: Some(count),
        });
    }
}

fn legacy_id_count(list: &LegacyIdList, id: &str) -> i32 {
    list.iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.count)
        .unwrap_or(0)
}

fn legacy_id_count_or(list: &LegacyIdList, id: &str, zero_default: i32) -> i32 {
    list.iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.count.unwrap_or(0))
        .map(|count| if count == 0 { zero_default } else { count })
        .unwrap_or(0)
}

fn parse_legacy_id_list(_field: &str, raw: &str) -> Result<LegacyIdList, ScenarioError> {
    let mut entries = Vec::new();
    let mut position = 0;
    let mut first = true;
    loop {
        if !first && !consume_std_separator(raw, &mut position, b';') {
            break;
        }
        first = false;

        skip_std_whitespace(raw, &mut position);
        let id_start = position;
        while position < raw.len()
            && position - id_start < 4
            && is_std_identifier_byte(raw.as_bytes()[position])
        {
            position += 1;
        }
        let id = &raw[id_start..position];
        if !looks_like_compiled_c4id(id) {
            // C4IDList::Entry throws after C4IDAdapt has read at most four
            // identifier bytes. StdSTLContainerAdapt keeps earlier entries
            // and stops before inserting this invalid one.
            break;
        }
        let count = if consume_std_separator(raw, &mut position, b'=') {
            Some(parse_std_i32_prefix_at(raw, &mut position).unwrap_or(0))
        } else {
            None
        };
        entries.push(LegacyIdEntry {
            id: id.to_string(),
            count,
        });
    }
    Ok(entries)
}

fn looks_like_compiled_c4id(id: &str) -> bool {
    if id.len() != 4 || id == "NONE" {
        return false;
    }
    if id.bytes().all(|byte| byte.is_ascii_digit()) {
        return id.parse::<u16>().is_ok_and(|id| id != 0);
    }
    id.bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_std_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn consume_name_list_separator(
    raw: &str,
    position: &mut Option<usize>,
    reenter: &mut Option<usize>,
    separator: u8,
) -> bool {
    // StdCompilerINIRead::Separator parks a mismatched cursor in pReenter;
    // the next separator attempt restores it, even after a defaulted value.
    if let Some(saved) = reenter.take() {
        *position = Some(saved);
    }
    let Some(mut cursor) = *position else {
        return false;
    };
    skip_std_whitespace(raw, &mut cursor);
    if raw.as_bytes().get(cursor) != Some(&separator) {
        *reenter = Some(cursor);
        *position = None;
        return false;
    }
    *position = Some(cursor + 1);
    true
}

fn parse_legacy_name_list(_field: &str, raw: &str) -> Result<LegacyNameList, ScenarioError> {
    const C4_MAX_NAME_LIST: usize = 10;
    const C4_MAX_NAME: usize = 30;

    let mut entries = Vec::new();
    let mut position = Some(0);
    let mut reenter = None;
    for index in 0..C4_MAX_NAME_LIST {
        if index != 0 {
            consume_name_list_separator(raw, &mut position, &mut reenter, b';');
        }

        let name = if let Some(cursor) = position.as_mut() {
            skip_std_whitespace(raw, cursor);
            let name_start = *cursor;
            while *cursor < raw.len()
                && *cursor - name_start < C4_MAX_NAME
                && is_std_identifier_byte(raw.as_bytes()[*cursor])
            {
                *cursor += 1;
            }
            raw[name_start..*cursor].to_string()
        } else {
            String::new()
        };
        let has_count = consume_name_list_separator(raw, &mut position, &mut reenter, b'=');
        let count = if has_count {
            position
                .as_mut()
                .and_then(|cursor| parse_std_i32_prefix_at(raw, cursor))
                .unwrap_or(0)
        } else {
            0
        };
        if !name.is_empty() {
            entries.push(LegacyNameEntry {
                name,
                count: has_count.then_some(count),
            });
        }
    }
    Ok(entries)
}

fn parse_legacy_version(_field: &str, raw: &str) -> Result<[i32; 5], ScenarioError> {
    let mut version = [0; 5];
    compile_defaulted_i32_components(raw, &mut version, &[0; 5], true);
    Ok(version)
}

fn parse_base_functionality_number(raw: &str, position: &mut usize) -> Option<i32> {
    skip_std_whitespace(raw, position);
    let bytes = raw.as_bytes();
    let number_start = *position;
    let mut cursor = number_start;
    // StdCompilerINIRead selects base 16 only for an unsigned token that
    // starts with 0x. A sign therefore makes `-0x10` decimal -0 plus junk.
    let radix =
        if bytes.get(cursor) == Some(&b'0') && matches!(bytes.get(cursor + 1), Some(b'x' | b'X')) {
            cursor += 2;
            16u32
        } else {
            10u32
        };
    let negative = if radix == 10 {
        match bytes.get(cursor) {
            Some(b'-') => {
                cursor += 1;
                true
            }
            Some(b'+') => {
                cursor += 1;
                false
            }
            _ => false,
        }
    } else {
        false
    };
    let digits_start = cursor;
    let mut magnitude = 0u128;
    while let Some(digit) = bytes.get(cursor).and_then(|byte| match byte {
        b'0'..=b'9' => Some(u32::from(*byte - b'0')),
        b'a'..=b'f' if radix == 16 => Some(u32::from(*byte - b'a') + 10),
        b'A'..=b'F' if radix == 16 => Some(u32::from(*byte - b'A') + 10),
        _ => None,
    }) {
        if digit >= radix {
            break;
        }
        magnitude = magnitude
            .saturating_mul(u128::from(radix))
            .saturating_add(u128::from(digit));
        cursor += 1;
    }
    if cursor == digits_start {
        if radix == 16 {
            // strtol("0xG", ..., 16) still consumes the leading zero.
            *position = number_start + 1;
            return Some(0);
        }
        return None;
    }

    // strtol saturates to native C long; assigning that result to int32_t
    // then supplies the platform's ordinary modulo narrowing.
    let long_bits = std::mem::size_of::<std::os::raw::c_long>() * 8;
    let long_max = (1u128 << (long_bits - 1)) - 1;
    let long_min_magnitude = 1u128 << (long_bits - 1);
    let signed = if negative {
        if magnitude >= long_min_magnitude {
            -(long_min_magnitude as i128)
        } else {
            -(magnitude as i128)
        }
    } else {
        magnitude.min(long_max) as i128
    };
    *position = cursor;
    Some((signed as i64) as i32)
}

fn parse_base_functionality(field: &str, raw: &str) -> Result<i32, ScenarioError> {
    if raw.trim().is_empty() {
        return Ok(BASEFUNC_DEFAULT);
    }

    let mut value = 0;
    let mut position = 0;
    loop {
        if let Some(flag) = parse_base_functionality_number(raw, &mut position) {
            value |= flag;
        } else {
            let start = position;
            while raw
                .as_bytes()
                .get(position)
                .is_some_and(|byte| is_std_identifier_byte(*byte))
            {
                position += 1;
            }
            if position == start {
                return Err(ScenarioError::LegacyParse(format!(
                    "missing BaseFunctionality token in `{field}`"
                )));
            }
            let entry = &raw[start..position];
            let flag = match entry {
                "BASEFUNC_Default" => BASEFUNC_DEFAULT,
                "BASEFUNC_AutoSellContents" => BASEFUNC_AUTO_SELL_CONTENTS,
                "BASEFUNC_RegenerateEnergy" => BASEFUNC_REGENERATE_ENERGY,
                "BASEFUNC_Buy" => BASEFUNC_BUY,
                "BASEFUNC_Sell" => BASEFUNC_SELL,
                "BASEFUNC_RejectEntrance" => BASEFUNC_REJECT_ENTRANCE,
                "BASEFUNC_Extinguish" => BASEFUNC_EXTINGUISH,
                _ => {
                    tracing::warn!(field, token = entry, "unknown BaseFunctionality bit name");
                    0
                }
            };
            value |= flag;
        }

        if !consume_std_separator(raw, &mut position, b'|') {
            break;
        }
    }
    Ok(value)
}

fn parse_i32_array<const N: usize>(_field: &str, raw: &str) -> Result<[i32; N], ScenarioError> {
    let mut result = [0; N];
    compile_defaulted_i32_components(raw, &mut result, &[0; N], true);
    Ok(result)
}

fn parse_position(_field: &str, raw: &str) -> Result<[i32; 2], ScenarioError> {
    let mut result = [-1, -1];
    compile_defaulted_i32_components(raw, &mut result, &[-1, -1], true);
    Ok(result)
}

impl LegacyHead {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        let has_max_player_league = entries
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case("MaxPlayerLeague"));
        let mut seen_fields = HashSet::new();
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            // StdCompilerINIRead resolves the first same-name child. A later
            // duplicate neither overwrites it nor exposes a parse failure.
            if !seen_fields.insert(key_lower.clone()) {
                continue;
            }
            let raw = value.trim();
            match key_lower.as_str() {
                "icon" => {
                    self.icon = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "title" => {
                    if !raw.is_empty() {
                        self.title = raw.to_string();
                    }
                }
                "loader" => {
                    if !raw.is_empty() {
                        self.loader = raw.to_string();
                    }
                }
                "font" => {
                    if !raw.is_empty() {
                        self.font = raw.to_string();
                    }
                }
                "version" => {
                    self.version = parse_legacy_version(key, raw)?;
                }
                "difficulty" => {
                    self.difficulty = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "maxplayer" => {
                    self.max_player = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "maxplayerleague" => {
                    self.max_player_league = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "minplayer" => {
                    self.min_player = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "savegame" => {
                    self.save_game = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "replay" => {
                    self.replay = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "film" => {
                    self.film = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "disablemouse" => {
                    self.disable_mouse = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "noinitialize" => {
                    self.no_initialize = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "randomseed" => {
                    self.random_seed = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "forcedautocontextmenu" => {
                    self.forced_auto_context_menu = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "forcedautostopcontrol" => {
                    self.forced_control_style = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "engine" => {
                    if !raw.is_empty() {
                        self.engine = raw.to_string();
                    }
                }
                "missionaccess" => {
                    if !raw.is_empty() {
                        self.mission_access = truncate_legacy_c4_string(raw.to_string(), 512);
                    }
                }
                "networkgame" => {
                    self.network_game = parse_bool_field(key, raw)?;
                }
                "networkruntimejoin" => {
                    self.network_runtime_join = parse_bool_field(key, raw)?;
                }
                "forcedgfxmode" => {
                    self.forced_gfx_mode = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "forcednocrew" => {
                    self.forced_fair_crew = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "defcrewstrength" => {
                    self.fair_crew_strength = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "origin" => {
                    if raw.is_empty() {
                        self.origin = None;
                    } else {
                        self.origin = Some(raw.to_string());
                    }
                }
                _ => {}
            }
        }
        // mkNamingAdapt(MaxPlayerLeague, ..., MaxPlayer) is compiled after
        // MaxPlayer, so an omitted league limit inherits the parsed regular
        // limit rather than C4S_MaxPlayerDefault.
        if !has_max_player_league {
            self.max_player_league = self.max_player;
        }
        Ok(())
    }
}

impl LegacyDefinitions {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        // StdCompilerINIRead keeps the first same-name value in a section.
        // In particular, later scalar duplicates must neither overwrite a
        // valid value nor surface a parse error that C++ never observes.
        if let Some((key, value)) = entries.iter().find(|(key, _)| key == "LocalOnly") {
            self.local_only = parse_bool_field(key, value.trim())?;
        }
        if let Some((key, value)) = entries.iter().find(|(key, _)| key == "AllowUserChange") {
            self.allow_user_change = parse_bool_field(key, value.trim())?;
        }
        if let Some((key, value)) = entries.iter().find(|(key, _)| key == "SkipDefs") {
            self.skip_defs = parse_legacy_id_list(key, value.trim())?;
        }
        // C4SDefinitions::CompileFunc first compiles the comma-separated
        // modern container. Only when that is empty does it query exactly
        // Definition1 through Definition10, one literal module per slot.
        let reflected_definitions = entries
            .iter()
            .find(|(key, _)| key == "Definitions")
            .map(|(_, value)| clonk_resources::scenario::parse_c4s_string_list(value))
            .unwrap_or_default();
        let mut definitions = reflected_definitions
            .iter()
            .map(|value| value.replace('\\', "/"))
            .collect::<Vec<_>>();
        let mut reflected_definitions = reflected_definitions;
        if definitions.is_empty() {
            for index in 1..=10 {
                let key = format!("Definition{index}");
                let Some(raw) = entries
                    .iter()
                    .find(|(entry_key, _)| entry_key == &key)
                    // mkStringAdaptA uses RCT_All: skip leading spaces/tabs,
                    // then retain every byte through the line ending,
                    // including quotes and trailing spaces.
                    .map(|(_, value)| value.trim_start_matches([' ', '\t'].as_ref()))
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                reflected_definitions.push(raw.to_string());
                definitions.push(normalize_definition_path(raw));
            }
        }
        self.reflected_definitions = Some(reflected_definitions);
        self.definitions = definitions;
        Ok(())
    }
}

impl LegacyGame {
    fn clear_old_goals(&mut self) {
        self.create_objects.clear();
        self.clear_objects.clear();
        self.clear_materials.clear();
        self.value_gain = 0;
    }

    /// C4SGame::ConvertGoals, including its in-place selector resets and
    /// ClearOldGoals side effects (C4Scenario.cpp:503-545).
    fn convert_goals_after_load(&mut self) {
        if matches!(self.mode, 1 | 2) {
            set_legacy_id_count(&mut self.goals, "MELE", 1);
            self.clear_old_goals();
        }
        self.mode = 0;

        match self.cooperative_goal {
            1 => {
                set_legacy_id_count(&mut self.goals, "GLDM", 1);
                self.clear_old_goals();
            }
            2 => {
                set_legacy_id_count(&mut self.goals, "MNTK", 1);
                self.clear_old_goals();
            }
            3 => {
                let value_gain = (self.value_gain / 100).max(1);
                set_legacy_id_count(&mut self.goals, "VALG", value_gain);
                self.clear_old_goals();
            }
            _ => {}
        }
        self.cooperative_goal = 0;

        if self.realism.construction_needs_material {
            set_legacy_id_count(&mut self.rules, "CNMT", 1);
        }
        self.realism.construction_needs_material = false;
        if self.realism.structures_need_energy {
            set_legacy_id_count(&mut self.rules, "ENRG", 1);
        }
        self.realism.structures_need_energy = false;
        if self.enable_remove_flag {
            set_legacy_id_count(&mut self.rules, "FGRV", 1);
        }
        self.enable_remove_flag = false;

        match self.elimination {
            0 => set_legacy_id_count(&mut self.rules, "KILC", 1),
            2 => set_legacy_id_count(&mut self.rules, "CTFL", 1),
            _ => {}
        }
        self.elimination = 1;

        if legacy_id_count(&self.rules, "CTFL") != 0 {
            set_legacy_id_count(&mut self.rules, "FGRV", 1);
        }
    }

    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "mode" => {
                    self.mode = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "elimination" => {
                    self.elimination = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "cooperativegoal" => {
                    self.cooperative_goal = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "createobjects" => {
                    self.create_objects = parse_legacy_id_list(key, raw)?;
                }
                "clearobjects" => {
                    self.clear_objects = parse_legacy_id_list(key, raw)?;
                }
                "clearmaterials" => {
                    self.clear_materials = parse_legacy_name_list(key, value)?;
                }
                "valuegain" => {
                    self.value_gain = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "enableremoveflag" => {
                    self.enable_remove_flag = parse_bool_field(key, raw)?;
                }
                "structneedmaterial" => {
                    self.realism.construction_needs_material = parse_bool_field(key, raw)?;
                }
                "structneedenergy" => {
                    self.realism.structures_need_energy = parse_bool_field(key, raw)?;
                }
                "valueoverloads" => {
                    self.realism.value_overloads = parse_legacy_id_list(key, raw)?;
                }
                "landscapepushpull" => {
                    self.realism.landscape_push_pull = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "landscapeinsertthrust" => {
                    self.realism.landscape_insert_thrust = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "basefunctionality" => {
                    self.realism.base_functionality = parse_base_functionality(key, raw)?;
                }
                "baseregenerateenergyprice" => {
                    self.realism.base_regenerate_energy_price = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "goals" => {
                    self.goals = parse_legacy_id_list(key, raw)?;
                }
                "rules" => {
                    self.rules = parse_legacy_id_list(key, raw)?;
                }
                "fowcolor" => {
                    self.fow_color = parse_std_u32(raw).ok_or_else(|| {
                        ScenarioError::LegacyParse(format!("invalid value `{raw}` for `{key}`"))
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyPlayer {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "standardcrew" => {
                    let id = raw
                        .bytes()
                        .take_while(|byte| is_std_identifier_byte(*byte))
                        .take(4)
                        .map(char::from)
                        .collect::<String>();
                    self.standard_crew =
                        (id.len() == 4 && id != "NONE" && id != "0000").then_some(id);
                }
                "clonks" => {
                    self.clonks = parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(1, 0, 1, 10))?;
                }
                "wealth" => {
                    self.wealth =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 250))?;
                }
                "position" => {
                    self.position = parse_position(key, raw)?;
                }
                "enforceposition" => {
                    self.enforce_position = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "crew" => {
                    self.crew = parse_legacy_id_list(key, raw)?;
                }
                "buildings" => {
                    self.buildings = parse_legacy_id_list(key, raw)?;
                }
                "vehicles" => {
                    self.vehicles = parse_legacy_id_list(key, raw)?;
                }
                "material" => {
                    self.material = parse_legacy_id_list(key, raw)?;
                }
                "knowledge" => {
                    self.knowledge = parse_legacy_id_list(key, raw)?;
                }
                "homebasematerial" => {
                    self.home_base_material = parse_legacy_id_list(key, raw)?;
                }
                "homebaseproduction" => {
                    self.home_base_production = parse_legacy_id_list(key, raw)?;
                }
                "magic" => {
                    self.magic = parse_legacy_id_list(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyLandscape {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "exactlandscape" => {
                    self.exact_landscape = parse_bool_field(key, raw)?;
                }
                "vegetation" => {
                    self.vegetation = parse_legacy_id_list(key, raw)?;
                }
                "vegetationlevel" => {
                    self.vegetation_level =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 30, 0, 100))?;
                }
                "inearth" => {
                    self.in_earth = parse_legacy_id_list(key, raw)?;
                }
                "inearthlevel" => {
                    self.in_earth_level =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 0, 100))?;
                }
                "sky" => {
                    if raw.is_empty() {
                        self.sky = None;
                    } else {
                        self.sky = Some(raw.to_string());
                    }
                }
                "skyfade" => {
                    self.sky_fade = parse_i32_array::<6>(key, raw)?;
                }
                "nosky" => {
                    self.no_sky = parse_bool_field(key, raw)?;
                }
                "bottomopen" => {
                    self.bottom_open = parse_bool_field(key, raw)?;
                }
                "topopen" => {
                    self.top_open = parse_bool_field(key, raw)?;
                }
                "leftopen" => {
                    self.left_open = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "rightopen" => {
                    self.right_open = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "autoscansideopen" => {
                    self.auto_scan_side_open = parse_bool_field(key, raw)?;
                }
                "mapwidth" => {
                    self.map_width =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(100, 0, 64, 250))?;
                }
                "mapheight" => {
                    self.map_height =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 40, 250))?;
                }
                "mapzoom" => {
                    self.map_zoom =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(10, 0, 5, 15))?;
                }
                "amplitude" => {
                    self.amplitude =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "phase" => {
                    self.phase =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 0, 100))?;
                }
                "period" => {
                    self.period =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(15, 0, 0, 100))?;
                }
                "random" => {
                    self.random =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "material" => {
                    if !raw.is_empty() {
                        self.material = raw.to_string();
                    }
                }
                "liquid" => {
                    if !raw.is_empty() {
                        self.liquid = raw.to_string();
                    }
                }
                "liquidlevel" => {
                    self.liquid_level =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "mapplayerextend" => {
                    self.map_player_extend = parse_bool_field(key, raw)?;
                }
                "layers" => {
                    self.layers = parse_legacy_name_list(key, value)?;
                }
                "gravity" => {
                    self.gravity =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(100, 0, 10, 200))?;
                }
                "noscan" => {
                    self.no_scan = parse_bool_field(key, raw)?;
                }
                "keepmapcreator" => {
                    self.keep_map_creator = parse_bool_field(key, raw)?;
                }
                "skyscrollmode" => {
                    self.sky_scroll_mode = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "newstylelandscape" => {
                    self.new_style_landscape = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "fowres" => {
                    self.fow_resolution = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "shadematerials" => {
                    self.shade_materials = parse_bool_field(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyWeather {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "climate" => {
                    self.climate =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 10, 0, 100))?;
                }
                "startseason" => {
                    self.start_season =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 50, 0, 100))?;
                }
                "yearspeed" => {
                    self.year_speed =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(50, 0, 0, 100))?;
                }
                "rain" => {
                    self.rain = parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "wind" => {
                    self.wind =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 70, -100, 100))?;
                }
                "lightning" => {
                    self.lightning =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "precipitation" => {
                    if !raw.is_empty() {
                        self.precipitation = raw.to_string();
                    }
                }
                "nogamma" => {
                    self.no_gamma = parse_bool_field(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyDisasters {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "meteorite" => {
                    self.meteorite =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "volcano" => {
                    self.volcano =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                "earthquake" => {
                    self.earthquake =
                        parse_legacy_c4s_value(key, raw, LegacyC4SVal::new(0, 0, 0, 100))?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyAnimals {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "animal" => {
                    self.free_life = parse_legacy_id_list(key, raw)?;
                }
                "nest" => {
                    self.earth_nest = parse_legacy_id_list(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyEnvironment {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            if key_lower == "objects" {
                self.objects = parse_legacy_id_list(key, raw)?;
            }
        }
        Ok(())
    }
}

const CURRENT_SCENARIO_VERSION: [i32; 4] = [4, 9, 11, 0];
const C4_MAX_TITLE: usize = 512;

impl LegacyScenarioCore {
    /// Returns the fully loaded C4Scenario state without mutating the retained
    /// parsed source. C4Scenario::Load performs this conversion immediately
    /// after Compile, before either parameters or SaveCore can observe it
    /// (C4Scenario.cpp:86-97).
    fn after_load_conversion(&self) -> Self {
        let mut loaded = self.clone();
        loaded.game.convert_goals_after_load();
        loaded
    }

    fn initial_save_core(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.after_load_conversion();

        // C4GameSave::SaveCore updates the first four C4XVer components but
        // deliberately leaves the fifth (historic build component) intact
        // (C4GameSave.cpp:58-64).
        saved.head.version[..CURRENT_SCENARIO_VERSION.len()]
            .copy_from_slice(&CURRENT_SCENARIO_VERSION);
        // SCopy(..., C4MaxTitle) copies the native C string through the first
        // NUL and keeps at most C4MaxTitle bytes (C4GameSave.cpp:84;
        // C4Strings.cpp:67-81).
        saved.head.title = truncate_legacy_c4_string(scenario_title.to_owned(), C4_MAX_TITLE);
        saved.head.mission_access.clear();
        // SaveCore resets NetworkGame before the save specialization applies
        // its own flags. NetworkRuntimeJoin is deliberately retained here.
        saved.head.network_game = false;
        saved.head.forced_gfx_mode = 1;

        // C4SDefinitions::SetModules replaces the list and derives LocalOnly
        // from whether it is empty (C4Scenario.cpp:461-478).
        saved.definitions.definitions = set_legacy_definition_modules(
            definition_modules,
            definition_executable_path,
            definition_path,
        );
        saved.definitions.reflected_definitions = None;
        saved.definitions.local_only = definition_modules.is_empty();

        // GetSaveOrigin retains an existing origin; only an empty origin is
        // populated from the running scenario filename
        // (C4GameSave.cpp:93-101). C4SHead normalizes alternate separators
        // to the current platform while loading (C4Scenario.cpp:200-202).
        let origin = saved
            .head
            .origin
            .as_deref()
            .filter(|origin| !origin.is_empty())
            .unwrap_or(scenario_origin);
        saved.head.origin = (!origin.is_empty()).then(|| normalize_legacy_path(origin));

        // fInitial intentionally leaves NoInitialize and SaveGame unchanged
        // (C4GameSave.cpp:65-75).
        saved
    }

    fn initial_network_save(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.initial_save_core(
            scenario_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.network_game = true;
        saved.head.network_runtime_join = false;
        saved
    }

    fn initial_record_save(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.initial_save_core(
            record_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.replay = 1;
        saved.head.icon = 29;
        saved
    }

    fn runtime_network_save(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.runtime_exact_save_core(
            scenario_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.network_game = true;
        saved.head.network_runtime_join = true;
        saved
    }

    fn runtime_scenario_save(&self) -> Self {
        let mut saved = self.clone();
        saved.head.version[..CURRENT_SCENARIO_VERSION.len()]
            .copy_from_slice(&CURRENT_SCENARIO_VERSION);
        saved.head.no_initialize = 1;
        saved.head.save_game = 0;
        // SaveCore clears NetworkGame for every non-initial save, but does
        // not touch NetworkRuntimeJoin. Preserve that slightly surprising
        // distinction for scenarios that originated from a runtime dynamic.
        saved.head.network_game = false;
        saved.head.mission_access.clear();
        saved.head.forced_gfx_mode = 1;
        saved
    }

    fn runtime_savegame(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
        icon: i32,
    ) -> Self {
        let mut saved = self.runtime_exact_save_core(
            scenario_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.icon = icon;
        saved
    }

    fn runtime_record_save(
        &self,
        record_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.runtime_exact_save_core(
            record_title,
            definition_modules,
            definition_executable_path,
            definition_path,
            scenario_origin,
        );
        saved.head.replay = 1;
        saved.head.icon = 29;
        saved
    }

    fn runtime_exact_save_core(
        &self,
        scenario_title: &str,
        definition_modules: &[String],
        definition_executable_path: &str,
        definition_path: &str,
        scenario_origin: &str,
    ) -> Self {
        let mut saved = self.runtime_scenario_save();
        saved.head.title = truncate_legacy_c4_string(scenario_title.to_owned(), C4_MAX_TITLE);
        saved.head.save_game = 1;
        saved.definitions.definitions = set_legacy_definition_modules(
            definition_modules,
            definition_executable_path,
            definition_path,
        );
        saved.definitions.reflected_definitions = None;
        saved.definitions.local_only = definition_modules.is_empty();
        if saved.head.origin.as_deref().is_none_or(str::is_empty) {
            saved.head.origin =
                (!scenario_origin.is_empty()).then(|| normalize_legacy_path(scenario_origin));
        }
        saved
    }

    fn serialize_section(&self, force_exact: bool, head_defaults: Option<[i32; 2]>) -> Vec<u8> {
        let mut writer = LegacyScenarioIniWriter::default();
        let [context_menu_default, control_style_default] = head_defaults.unwrap_or([
            self.head.forced_auto_context_menu,
            self.head.forced_control_style,
        ]);
        let mut head = Vec::new();
        push_value(&mut head, "NoInitialize", self.head.no_initialize, 0);
        push_value(&mut head, "RandomSeed", self.head.random_seed, 0);
        push_value(
            &mut head,
            "ForcedAutoContextMenu",
            self.head.forced_auto_context_menu,
            context_menu_default,
        );
        push_value(
            &mut head,
            "ForcedAutoStopControl",
            self.head.forced_control_style,
            control_style_default,
        );
        writer.push_section("Head", head);

        let mut game = serialize_legacy_game(&self.game);
        game.retain(|(name, _)| *name != "ValueOverloads");
        writer.push_section("Game", game);
        for index in 0..MAX_PLAYER_STARTS {
            let player = self.players.get(index).cloned().unwrap_or_default();
            writer.push_section(
                &format!("Player{}", index + 1),
                serialize_legacy_player(&player),
            );
        }
        let mut landscape = self.landscape.clone();
        landscape.exact_landscape |= force_exact;
        writer.push_section(
            "Landscape",
            serialize_legacy_landscape(&landscape, self.uses_new_landscape_defaults()),
        );
        writer.push_section("Animals", serialize_legacy_animals(&self.animals));
        writer.push_section("Weather", serialize_legacy_weather(&self.weather));
        writer.push_section("Disasters", serialize_legacy_disasters(&self.disasters));
        writer.push_section(
            "Environment",
            serialize_legacy_environment(&self.environment),
        );
        writer.finish()
    }

    fn serialize(&self) -> Vec<u8> {
        let mut writer = LegacyScenarioIniWriter::default();
        writer.push_section("Head", serialize_legacy_head(&self.head));
        writer.push_section(
            "Definitions",
            serialize_legacy_definitions(&self.definitions),
        );
        writer.push_section("Game", serialize_legacy_game(&self.game));
        for index in 0..MAX_PLAYER_STARTS {
            let player = self.players.get(index).cloned().unwrap_or_default();
            writer.push_section(
                &format!("Player{}", index + 1),
                serialize_legacy_player(&player),
            );
        }
        writer.push_section(
            "Landscape",
            serialize_legacy_landscape(&self.landscape, self.uses_new_landscape_defaults()),
        );
        writer.push_section("Animals", serialize_legacy_animals(&self.animals));
        writer.push_section("Weather", serialize_legacy_weather(&self.weather));
        writer.push_section("Disasters", serialize_legacy_disasters(&self.disasters));
        writer.push_section(
            "Environment",
            serialize_legacy_environment(&self.environment),
        );
        writer.finish()
    }

    fn uses_new_landscape_defaults(&self) -> bool {
        self.head.version[0] == 0 || self.head.version >= [4, 6, 5, 0, 0]
    }
}

type LegacyIniFields = Vec<(&'static str, String)>;

#[derive(Default)]
struct LegacyScenarioIniWriter {
    output: Vec<u8>,
}

impl LegacyScenarioIniWriter {
    fn push_section(&mut self, name: &str, fields: LegacyIniFields) {
        if fields.is_empty() {
            return;
        }
        if !self.output.is_empty() {
            self.output.extend_from_slice(b"\r\n");
        }
        self.output.push(b'[');
        self.output.extend_from_slice(name.as_bytes());
        self.output.extend_from_slice(b"]\r\n");
        for (key, value) in fields {
            self.output.extend_from_slice(key.as_bytes());
            self.output.push(b'=');
            self.output
                .extend_from_slice(&clonk_script::c4_string_bytes(&value));
            self.output.extend_from_slice(b"\r\n");
        }
    }

    fn finish(self) -> Vec<u8> {
        self.output
    }
}

fn push_value<T>(fields: &mut LegacyIniFields, key: &'static str, value: T, default: T)
where
    T: fmt::Display + PartialEq,
{
    if value != default {
        fields.push((key, value.to_string()));
    }
}

fn push_raw_string(fields: &mut LegacyIniFields, key: &'static str, value: &str, default: &str) {
    if value != default {
        fields.push((key, value.to_owned()));
    }
}

fn push_i32_bool(fields: &mut LegacyIniFields, key: &'static str, value: bool, default: bool) {
    push_value(fields, key, i32::from(value), i32::from(default));
}

fn push_i32_array(fields: &mut LegacyIniFields, key: &'static str, values: &[i32], default: i32) {
    let count = values
        .iter()
        .rposition(|value| *value != default)
        .map_or(0, |index| index + 1);
    if count > 0 {
        fields.push((
            key,
            values[..count]
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
}

fn push_c4s_value(
    fields: &mut LegacyIniFields,
    key: &'static str,
    value: LegacyC4SVal,
    default: LegacyC4SVal,
) {
    if value != default {
        fields.push((
            key,
            format!("{},{},{},{}", value.std, value.rnd, value.min, value.max),
        ));
    }
}

fn push_id_list(fields: &mut LegacyIniFields, key: &'static str, values: &LegacyIdList) {
    if !values.is_empty() {
        fields.push((key, format_id_list(values)));
    }
}

fn push_name_list(fields: &mut LegacyIniFields, key: &'static str, values: &LegacyNameList) {
    if !values.is_empty() {
        fields.push((key, format_name_list(values)));
    }
}

fn format_id_list(values: &LegacyIdList) -> String {
    values
        .iter()
        .map(|entry| format!("{}={}", entry.id, entry.count.unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(";")
}

fn format_name_list(values: &LegacyNameList) -> String {
    values
        .iter()
        .map(|entry| format!("{}={}", entry.name, entry.count.unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(";")
}

fn serialize_legacy_head(head: &LegacyHead) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SHead::CompileFunc field order/defaults (C4Scenario.cpp:164-203).
    push_value(&mut fields, "Icon", head.icon, 18);
    push_raw_string(&mut fields, "Title", &head.title, "Default Title");
    push_raw_string(&mut fields, "Loader", &head.loader, "");
    push_raw_string(&mut fields, "Font", &head.font, "");
    push_i32_array(&mut fields, "Version", &head.version, 0);
    push_value(&mut fields, "Difficulty", head.difficulty, 0);
    push_value(&mut fields, "MaxPlayer", head.max_player, 12);
    push_value(
        &mut fields,
        "MaxPlayerLeague",
        head.max_player_league,
        head.max_player,
    );
    push_value(&mut fields, "MinPlayer", head.min_player, 0);
    push_value(&mut fields, "SaveGame", head.save_game, 0);
    push_value(&mut fields, "Replay", head.replay, 0);
    push_value(&mut fields, "Film", head.film, 0);
    push_value(&mut fields, "DisableMouse", head.disable_mouse, 0);
    push_value(&mut fields, "NoInitialize", head.no_initialize, 0);
    push_value(&mut fields, "RandomSeed", head.random_seed, 0);
    push_value(
        &mut fields,
        "ForcedAutoContextMenu",
        head.forced_auto_context_menu,
        -1,
    );
    push_value(
        &mut fields,
        "ForcedAutoStopControl",
        head.forced_control_style,
        -1,
    );
    push_raw_string(&mut fields, "Engine", &head.engine, "");
    push_raw_string(&mut fields, "MissionAccess", &head.mission_access, "");
    push_value(&mut fields, "NetworkGame", head.network_game, false);
    push_value(
        &mut fields,
        "NetworkRuntimeJoin",
        head.network_runtime_join,
        false,
    );
    push_value(&mut fields, "ForcedGfxMode", head.forced_gfx_mode, 0);
    push_value(&mut fields, "ForcedNoCrew", head.forced_fair_crew, 0);
    push_value(&mut fields, "DefCrewStrength", head.fair_crew_strength, 0);
    push_raw_string(
        &mut fields,
        "Origin",
        head.origin.as_deref().unwrap_or_default(),
        "",
    );
    fields
}

fn serialize_legacy_definitions(definitions: &LegacyDefinitions) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SDefinitions::CompileFunc (C4Scenario.cpp:480-500).
    push_value(&mut fields, "LocalOnly", definitions.local_only, false);
    push_value(
        &mut fields,
        "AllowUserChange",
        definitions.allow_user_change,
        false,
    );
    if !definitions.definitions.is_empty() {
        fields.push((
            "Definitions",
            definitions
                .definitions
                .iter()
                .map(|module| escape_cpp_ini_string(module))
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
    push_id_list(&mut fields, "SkipDefs", &definitions.skip_defs);
    fields
}

fn serialize_legacy_game(game: &LegacyGame) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SGame::CompileFunc (C4Scenario.cpp:221-257).
    push_value(&mut fields, "Mode", game.mode, 0);
    push_value(&mut fields, "Elimination", game.elimination, 1);
    push_value(&mut fields, "CooperativeGoal", game.cooperative_goal, 0);
    push_id_list(&mut fields, "CreateObjects", &game.create_objects);
    push_id_list(&mut fields, "ClearObjects", &game.clear_objects);
    push_name_list(&mut fields, "ClearMaterials", &game.clear_materials);
    push_value(&mut fields, "ValueGain", game.value_gain, 0);
    push_value(
        &mut fields,
        "EnableRemoveFlag",
        game.enable_remove_flag,
        false,
    );
    push_value(
        &mut fields,
        "StructNeedMaterial",
        game.realism.construction_needs_material,
        false,
    );
    push_value(
        &mut fields,
        "StructNeedEnergy",
        game.realism.structures_need_energy,
        true,
    );
    push_id_list(&mut fields, "ValueOverloads", &game.realism.value_overloads);
    push_value(
        &mut fields,
        "LandscapePushPull",
        game.realism.landscape_push_pull,
        0,
    );
    push_value(
        &mut fields,
        "LandscapeInsertThrust",
        game.realism.landscape_insert_thrust,
        1,
    );
    if game.realism.base_functionality != BASEFUNC_DEFAULT {
        if let Some(value) = format_base_functionality(game.realism.base_functionality) {
            fields.push(("BaseFunctionality", value));
        }
    }
    push_value(
        &mut fields,
        "BaseRegenerateEnergyPrice",
        game.realism.base_regenerate_energy_price,
        BASE_REGENERATE_ENERGY_PRICE,
    );
    push_id_list(&mut fields, "Goals", &game.goals);
    push_id_list(&mut fields, "Rules", &game.rules);
    push_value(&mut fields, "FoWColor", game.fow_color, 0);
    fields
}

fn serialize_legacy_player(player: &LegacyPlayer) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SPlrStart::CompileFunc (C4Scenario.cpp:276-291).
    if let Some(crew) = player.standard_crew.as_deref() {
        push_raw_string(&mut fields, "StandardCrew", crew, "");
    }
    push_c4s_value(
        &mut fields,
        "Clonks",
        player.clonks,
        LegacyC4SVal::new(1, 0, 1, 10),
    );
    push_c4s_value(
        &mut fields,
        "Wealth",
        player.wealth,
        LegacyC4SVal::new(0, 0, 0, 250),
    );
    push_i32_array(&mut fields, "Position", &player.position, -1);
    push_value(&mut fields, "EnforcePosition", player.enforce_position, 0);
    push_id_list(&mut fields, "Crew", &player.crew);
    push_id_list(&mut fields, "Buildings", &player.buildings);
    push_id_list(&mut fields, "Vehicles", &player.vehicles);
    push_id_list(&mut fields, "Material", &player.material);
    push_id_list(&mut fields, "Knowledge", &player.knowledge);
    push_id_list(&mut fields, "HomeBaseMaterial", &player.home_base_material);
    push_id_list(
        &mut fields,
        "HomeBaseProduction",
        &player.home_base_production,
    );
    push_id_list(&mut fields, "Magic", &player.magic);
    fields
}

fn serialize_legacy_landscape(
    landscape: &LegacyLandscape,
    shade_materials_default: bool,
) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SLandscape::CompileFunc (C4Scenario.cpp:336-370). SaveCore has
    // already set a current engine version, so ShadeMaterials defaults true.
    push_value(
        &mut fields,
        "ExactLandscape",
        landscape.exact_landscape,
        false,
    );
    push_id_list(&mut fields, "Vegetation", &landscape.vegetation);
    push_c4s_value(
        &mut fields,
        "VegetationLevel",
        landscape.vegetation_level,
        LegacyC4SVal::new(50, 30, 0, 100),
    );
    push_id_list(&mut fields, "InEarth", &landscape.in_earth);
    push_c4s_value(
        &mut fields,
        "InEarthLevel",
        landscape.in_earth_level,
        LegacyC4SVal::new(50, 0, 0, 100),
    );
    push_raw_string(
        &mut fields,
        "Sky",
        landscape.sky.as_deref().unwrap_or_default(),
        "",
    );
    push_i32_array(&mut fields, "SkyFade", &landscape.sky_fade, 0);
    push_value(&mut fields, "NoSky", landscape.no_sky, false);
    push_value(&mut fields, "BottomOpen", landscape.bottom_open, false);
    push_value(&mut fields, "TopOpen", landscape.top_open, true);
    push_value(&mut fields, "LeftOpen", landscape.left_open, 0);
    push_value(&mut fields, "RightOpen", landscape.right_open, 0);
    push_value(
        &mut fields,
        "AutoScanSideOpen",
        landscape.auto_scan_side_open,
        true,
    );
    push_c4s_value(
        &mut fields,
        "MapWidth",
        landscape.map_width,
        LegacyC4SVal::new(100, 0, 64, 250),
    );
    push_c4s_value(
        &mut fields,
        "MapHeight",
        landscape.map_height,
        LegacyC4SVal::new(50, 0, 40, 250),
    );
    push_c4s_value(
        &mut fields,
        "MapZoom",
        landscape.map_zoom,
        LegacyC4SVal::new(10, 0, 5, 15),
    );
    push_c4s_value(
        &mut fields,
        "Amplitude",
        landscape.amplitude,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Phase",
        landscape.phase,
        LegacyC4SVal::new(50, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Period",
        landscape.period,
        LegacyC4SVal::new(15, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Random",
        landscape.random,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_raw_string(&mut fields, "Material", &landscape.material, "Earth");
    push_raw_string(&mut fields, "Liquid", &landscape.liquid, "Water");
    push_c4s_value(
        &mut fields,
        "LiquidLevel",
        landscape.liquid_level,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_value(
        &mut fields,
        "MapPlayerExtend",
        landscape.map_player_extend,
        false,
    );
    push_name_list(&mut fields, "Layers", &landscape.layers);
    push_c4s_value(
        &mut fields,
        "Gravity",
        landscape.gravity,
        LegacyC4SVal::new(100, 0, 10, 200),
    );
    push_value(&mut fields, "NoScan", landscape.no_scan, false);
    push_value(
        &mut fields,
        "KeepMapCreator",
        landscape.keep_map_creator,
        false,
    );
    push_value(&mut fields, "SkyScrollMode", landscape.sky_scroll_mode, 0);
    push_value(
        &mut fields,
        "NewStyleLandscape",
        landscape.new_style_landscape,
        0,
    );
    push_value(
        &mut fields,
        "FoWRes",
        landscape.fow_resolution,
        DEFAULT_FOW_RESOLUTION,
    );
    push_value(
        &mut fields,
        "ShadeMaterials",
        landscape.shade_materials,
        shade_materials_default,
    );
    fields
}

fn serialize_legacy_animals(animals: &LegacyAnimals) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SAnimals::CompileFunc (C4Scenario.cpp:394-404).
    push_id_list(&mut fields, "Animal", &animals.free_life);
    push_id_list(&mut fields, "Nest", &animals.earth_nest);
    fields
}

fn serialize_legacy_weather(weather: &LegacyWeather) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SWeather::CompileFunc (C4Scenario.cpp:372-392).
    push_c4s_value(
        &mut fields,
        "Climate",
        weather.climate,
        LegacyC4SVal::new(50, 10, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "StartSeason",
        weather.start_season,
        LegacyC4SVal::new(50, 50, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "YearSpeed",
        weather.year_speed,
        LegacyC4SVal::new(50, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Rain",
        weather.rain,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Wind",
        weather.wind,
        LegacyC4SVal::new(0, 70, -100, 100),
    );
    push_c4s_value(
        &mut fields,
        "Lightning",
        weather.lightning,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_raw_string(
        &mut fields,
        "Precipitation",
        &weather.precipitation,
        "Water",
    );
    push_value(&mut fields, "NoGamma", weather.no_gamma, true);
    fields
}

fn serialize_legacy_disasters(disasters: &LegacyDisasters) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SDisasters::CompileFunc (C4Scenario.cpp:427-439).
    push_c4s_value(
        &mut fields,
        "Meteorite",
        disasters.meteorite,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Volcano",
        disasters.volcano,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    push_c4s_value(
        &mut fields,
        "Earthquake",
        disasters.earthquake,
        LegacyC4SVal::new(0, 0, 0, 100),
    );
    fields
}

fn serialize_legacy_environment(environment: &LegacyEnvironment) -> LegacyIniFields {
    let mut fields = Vec::new();
    // C4SEnvironment::CompileFunc (C4Scenario.cpp:406-414).
    push_id_list(&mut fields, "Objects", &environment.objects);
    fields
}

fn format_base_functionality(value: i32) -> Option<String> {
    let entries = [
        ("BASEFUNC_AutoSellContents", BASEFUNC_AUTO_SELL_CONTENTS),
        ("BASEFUNC_RegenerateEnergy", BASEFUNC_REGENERATE_ENERGY),
        ("BASEFUNC_Buy", BASEFUNC_BUY),
        ("BASEFUNC_Sell", BASEFUNC_SELL),
        ("BASEFUNC_RejectEntrance", BASEFUNC_REJECT_ENTRANCE),
        ("BASEFUNC_Extinguish", BASEFUNC_EXTINGUISH),
        ("BASEFUNC_Default", BASEFUNC_DEFAULT),
    ];
    let mut remaining = value;
    let mut parts = Vec::new();
    for (name, mask) in entries {
        if remaining != 0 && (mask & remaining) == mask {
            parts.push(name.to_owned());
            remaining &= !mask;
        }
    }
    if remaining != 0 {
        parts.push(remaining.to_string());
    }
    (!parts.is_empty()).then(|| parts.join("|"))
}

fn escape_cpp_ini_string(value: &str) -> String {
    let value = clonk_script::c4_string_bytes(value);
    let mut escaped = Vec::with_capacity(value.len() + 2);
    escaped.push(b'"');
    let mut previous_was_numeric_escape = false;
    for byte in value {
        // StdCompilerINIWrite applies `isprint` to unsigned native bytes.
        // The legacy single-byte locale treats the upper printable block as
        // text too; preserving it here is what keeps C4 filenames byte-exact.
        let printable = byte.is_ascii_graphic() || byte == b' ' || byte >= 0xa0;
        if printable
            && byte != b'\\'
            && byte != b'"'
            && !(previous_was_numeric_escape && byte.is_ascii_digit())
        {
            escaped.push(byte);
            previous_was_numeric_escape = false;
            continue;
        }
        previous_was_numeric_escape = false;
        match byte {
            b'\x07' => escaped.extend_from_slice(b"\\a"),
            b'\x08' => escaped.extend_from_slice(b"\\b"),
            b'\x0c' => escaped.extend_from_slice(b"\\f"),
            b'\n' => escaped.extend_from_slice(b"\\n"),
            b'\r' => escaped.extend_from_slice(b"\\r"),
            b'\t' => escaped.extend_from_slice(b"\\t"),
            b'\x0b' => escaped.extend_from_slice(b"\\v"),
            b'"' => escaped.extend_from_slice(b"\\\""),
            b'\\' => escaped.extend_from_slice(b"\\\\"),
            byte => {
                escaped.push(b'\\');
                escaped.extend_from_slice(format!("{byte:o}").as_bytes());
                previous_was_numeric_escape = true;
            }
        }
    }
    escaped.push(b'"');
    clonk_script::c4_string_from_bytes(&escaped)
}

fn normalize_legacy_path(path: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        path.replace('\\', "/")
    } else {
        path.replace('/', "\\")
    }
}

/// `C4SDefinitions::SetModules`: preserve every separator and redundant path
/// component, then strip ExePath and DefinitionPath in that exact order.
/// Native compares the requested prefix length case-insensitively and does
/// not require a component boundary (C4Scenario.cpp:461-478).
fn set_legacy_definition_modules(
    modules: &[String],
    executable_path: &str,
    definition_path: &str,
) -> Vec<String> {
    let executable_path = clonk_script::c4_string_bytes(executable_path);
    let definition_path = clonk_script::c4_string_bytes(definition_path);

    modules
        .iter()
        .map(|module| {
            let mut bytes = clonk_script::c4_string_bytes(module);
            for prefix in [&executable_path, &definition_path] {
                if !prefix.is_empty()
                    && bytes.len() >= prefix.len()
                    && bytes[..prefix.len()]
                        .iter()
                        .zip(prefix.iter())
                        .all(|(left, right)| {
                            legacy_byte_capital(*left) == legacy_byte_capital(*right)
                        })
                {
                    bytes.drain(..prefix.len());
                }
            }
            clonk_script::c4_string_from_bytes(&bytes)
        })
        .collect()
}

fn legacy_byte_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - 32,
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

impl LegacyScenarioCore {
    fn from_sections(
        sections: &HashMap<String, Vec<(String, String)>>,
    ) -> Result<Self, ScenarioError> {
        let mut core = LegacyScenarioCore::default();
        if let Some(entries) = sections.get("head") {
            core.head.apply_entries(entries)?;
            // MaxPlayerLeague's compile default is the already-read
            // MaxPlayer, not C4S_MaxPlayerDefault
            // (C4Scenario.cpp:177-179).
            if !entries
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case("MaxPlayerLeague"))
            {
                core.head.max_player_league = core.head.max_player;
            }
        }
        if let Some(entries) = sections.get("definitions") {
            core.definitions.apply_entries(entries)?;
        }
        // C4SRealism::Default starts at zero, but the main-scenario compiler
        // defaults this field to one before applying any explicit value
        // (C4Scenario.cpp:416-425,237-238).
        core.game.realism.landscape_insert_thrust = 1;
        if let Some(entries) = sections.get("game") {
            core.game.apply_entries(entries)?;
        }
        // ShadeMaterials' absent-value default depends on the version that
        // Head compiled first (C4Scenario.cpp:120-133,336-370).
        core.landscape.shade_materials =
            core.head.version[0] == 0 || core.head.version >= [4, 6, 5, 0, 0];
        if let Some(entries) = sections.get("landscape") {
            core.landscape.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("weather") {
            core.weather.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("disasters") {
            core.disasters.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("animals") {
            core.animals.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("environment") {
            core.environment.apply_entries(entries)?;
        }

        for (section, entries) in sections {
            if !section.starts_with("player") {
                continue;
            }
            let Some(owner) = owner_index_from_section(section) else {
                continue;
            };
            if owner < 0 {
                continue;
            }
            let index = owner as usize;
            if core.players.len() <= index {
                core.players.resize(index + 1, LegacyPlayer::default());
            }
            core.players[index].apply_entries(entries)?;
        }

        Ok(core)
    }

    /// Compile a present section Scenario.txt with C4Scenario's `fSection`
    /// field set. Named fields in the compiled subset receive their naming
    /// defaults even when their whole INI section is absent; only the fields
    /// omitted by the C++ section compiler retain the main core's values
    /// (C4Scenario.cpp:120-134,164-204,221-257,441-445).
    fn compile_section(
        &self,
        sections: &HashMap<String, Vec<(String, String)>>,
    ) -> Result<Self, ScenarioError> {
        let mut core = LegacyScenarioCore::default();

        // Head compiles only these four fields in section mode. The two
        // forced-control defaults are statics captured from the main Head;
        // every other Head value survives unchanged.
        core.head = self.head.clone();
        core.head.no_initialize = 0;
        core.head.random_seed = 0;
        if let Some(entries) = sections.get("head") {
            let retained_max_player_league = core.head.max_player_league;
            let entries = entries
                .iter()
                .filter(|(key, _)| {
                    [
                        "NoInitialize",
                        "RandomSeed",
                        "ForcedAutoContextMenu",
                        "ForcedAutoStopControl",
                    ]
                    .iter()
                    .any(|allowed| key.eq_ignore_ascii_case(allowed))
                })
                .cloned()
                .collect::<Vec<_>>();
            core.head.apply_entries(&entries)?;
            // `LegacyHead::apply_entries` normally implements the main-file
            // MaxPlayerLeague fallback. That field is not compiled at all in
            // section mode, so preserve the already-loaded main value.
            core.head.max_player_league = retained_max_player_league;
        }

        // Definitions and ValueOverloads are not visited by the section
        // compiler at all.
        core.definitions = self.definitions.clone();
        core.game.realism.value_overloads = self.game.realism.value_overloads.clone();
        // This compiler default differs from C4SRealism::Default().
        core.game.realism.landscape_insert_thrust = 1;
        if let Some(entries) = sections.get("game") {
            let entries = entries
                .iter()
                .filter(|(key, _)| !key.eq_ignore_ascii_case("ValueOverloads"))
                .cloned()
                .collect::<Vec<_>>();
            core.game.apply_entries(&entries)?;
        }

        core.players = vec![LegacyPlayer::default(); MAX_PLAYER_STARTS];
        for index in 0..MAX_PLAYER_STARTS {
            if let Some(entries) = sections.get(&format!("player{}", index + 1)) {
                core.players[index].apply_entries(entries)?;
            }
        }

        // Version is a retained Head field, so ShadeMaterials' naming default
        // is derived from the main scenario version even for a section.
        core.landscape.shade_materials =
            core.head.version[0] == 0 || core.head.version >= [4, 6, 5, 0, 0];
        if let Some(entries) = sections.get("landscape") {
            core.landscape.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("animals") {
            core.animals.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("weather") {
            core.weather.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("disasters") {
            core.disasters.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("environment") {
            core.environment.apply_entries(entries)?;
        }

        apply_scenario_rct_all_strings(&mut core, sections, false);
        Ok(core)
    }
}

fn parse_legacy_scenario_manifest(group: &Group) -> Result<LegacyScenarioManifest, ScenarioError> {
    let bytes = match read_group_file_case_insensitive(group, "Scenario.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Err(ScenarioError::LegacyCoreMissing),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ScenarioError::LegacyCoreMissing);
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    // StdCompiler receives Scenario.txt as a C string. A packed component may
    // carry its terminating NUL in the stored size; anything after the first
    // NUL is invisible to C++ and must not influence loader metadata.
    let visible_len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let visible = &bytes[..visible_len];
    let text = clonk_script::c4_string_from_bytes(visible);
    let mut manifest = parse_legacy_scenario_text(&text)?;

    // Parse a byte-for-byte Latin-1 projection of just the title as well. The
    // INI grammar is ASCII, so this retains the exact value bytes while finding
    // the same Head field independently of the script string representation.
    // Avoid semantically compiling the duplicate projection: an unrelated
    // non-ASCII field must not create a second failure surface.
    let native_text = bytes_as_latin1_string(visible);
    let native_tree = LegacyIniTree::parse(&native_text);
    let native_title = native_tree
        .first_section(0, "Head")
        .and_then(|head| native_tree.value(head, "Title"))
        .map(parse_rct_all)
        .unwrap_or_else(|| "Default Title".to_string())
        .chars()
        .map(|character| character as u8)
        .collect::<Vec<_>>();
    manifest.head_title_native = LegacyCString::from_bytes(native_title);
    Ok(manifest)
}

fn overlay_legacy_scenario_manifest(
    base: &LegacyScenarioManifest,
    overlay: LegacyScenarioManifest,
) -> Result<LegacyScenarioManifest, ScenarioError> {
    // Raw section entries must remain separate from the main file: landscape
    // and weather loaders also use their absence to select C++ defaults.
    let sections = overlay.sections;
    let core = base.core.compile_section(&sections)?;
    let ground_height_hint = derive_ground_height_hint(&sections);
    let definition_specs = core.definitions.definitions.clone();

    Ok(LegacyScenarioManifest {
        title: base.title.clone(),
        description: base.description.clone(),
        head_title_native: base.head_title_native.clone(),
        definition_specs,
        ground_height_hint,
        core,
        sections,
    })
}

fn read_group_file_case_insensitive(group: &Group, name: &str) -> Result<Vec<u8>, GroupError> {
    try_read_group_file_case_insensitive(group, name)?
        .ok_or_else(|| GroupError::EntryNotFound(PathBuf::from(name)))
}

fn read_optional_legacy_entry(group: &Group, name: &str) -> Result<Option<Vec<u8>>, ScenarioError> {
    match group.read_file(name) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(GroupError::EntryNotFound(_)) => Ok(None),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

/// Extracts an INI name with the same whitespace rules as
/// `StdCompilerINIRead::CreateNameTree`: spaces are name characters, while a
/// tab terminates the name and may be followed only by spaces or more tabs.
fn stdcompiler_ini_name(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }

    let mut end = 0;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b' ' || *byte == b'_')
    {
        end += 1;
    }

    bytes[end..]
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then(|| &raw[..end])
}

fn try_read_group_file_case_insensitive(
    group: &Group,
    name: &str,
) -> Result<Option<Vec<u8>>, GroupError> {
    let entry = group.entries()?.into_iter().find(|entry| {
        entry.relative_path.components().count() == 1
            && entry
                .relative_path
                .to_str()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
    });
    entry
        .map(|entry| group.read_file(entry.relative_path))
        .transpose()
}

fn load_loader_scenario_title<S: AsRef<str>>(
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<(String, LegacyCString)>, ScenarioError> {
    let candidates = languages
        .iter()
        .map(|language| format!("Title{}.txt", language.as_ref()))
        .chain(std::iter::once("Title.txt".to_string()));
    for candidate in candidates {
        let Some(component) = components.read(&candidate)? else {
            continue;
        };
        let source = component
            .bytes
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default();
        for language in languages {
            let needle = format!("{}:", language.as_ref());
            if let Some(position) = cpp_ssearch_end(source, needle.as_bytes()) {
                let value = &source[position..];
                // C4ComponentHost first searches the complete remainder for
                // CR and only falls back to LF when no CR exists anywhere.
                let end = value
                    .iter()
                    .position(|byte| *byte == b'\r')
                    .or_else(|| value.iter().position(|byte| *byte == b'\n'))
                    .unwrap_or(value.len());
                let native = value[..end].to_vec();
                let presentation = decode_legacy_script_text(&native);
                let native = LegacyCString::from_bytes(native)
                    .expect("the title component was truncated before its first NUL");
                return Ok(Some((presentation, native)));
            }
        }
        // C4ComponentHost keeps the first existing component even when it
        // contains no requested language; Head.Title is then the fallback.
        return Ok(None);
    }
    Ok(None)
}

fn cpp_ssearch_end(source: &[u8], needle: &[u8]) -> Option<usize> {
    let mut matched = 0usize;
    for (index, byte) in source.iter().enumerate() {
        if *byte == needle[matched] {
            matched += 1;
        } else {
            // C++ SSearch does not reconsider the mismatching byte as the
            // beginning of a new partial match.
            matched = 0;
        }
        if matched >= needle.len() {
            return Some(index + 1);
        }
    }
    None
}

fn validate_name_ex_no_empty(mut value: String) -> Result<String, ScenarioError> {
    value = value
        .trim_matches(|character: char| character.is_ascii_whitespace())
        .to_string();
    if value.is_empty() {
        return Ok("Unknown".to_string());
    }
    if value.len() > 120 {
        if !value.is_char_boundary(120) {
            return Err(ScenarioError::LoaderTitleTruncationBoundary { limit: 120 });
        }
        value.truncate(120);
    }
    Ok(value)
}

fn validate_name_ex_no_empty_bytes(value: &[u8]) -> LegacyCString {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    let mut value = value[start..end].to_vec();
    if value.is_empty() {
        value.extend_from_slice(b"Unknown");
    }
    value.truncate(120);
    LegacyCString::from_bytes(value).expect("a Scenario.txt title contains no interior NUL")
}

/// Splits legacy compiler input on LF, CRLF, or lone CR without changing the
/// physical-line count of ordinary LF/CRLF files.
fn legacy_ini_lines(source: &str) -> impl Iterator<Item = &str> {
    // `str::lines` recognizes LF and CRLF, but not a lone CR. Split LF first
    // to keep CRLF as one physical line, then split any remaining bare CRs.
    source.split_inclusive('\n').flat_map(|line| {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        line.split('\r')
    })
}

/// Reads the first exact `[Parameters] MaxPlayers` value. The compiler's
/// scenario-derived default is `C4S.Head.MaxPlayer`; Parameters.txt may
/// replace it before offline players are admitted (pristine 9ffa0a5d
/// src/C4GameParameters.cpp:408-422,553-558).
fn parse_legacy_parameters_max_players(
    bytes: &[u8],
    scenario_default: i32,
) -> Result<i32, ScenarioError> {
    parse_legacy_parameters_i32(bytes, "MaxPlayers", scenario_default)
}

fn parse_legacy_parameters_random_seed(
    bytes: &[u8],
    scenario_default: i32,
) -> Result<i32, ScenarioError> {
    parse_legacy_parameters_i32(bytes, "RandomSeed", scenario_default)
}

fn parse_legacy_parameters_i32(
    bytes: &[u8],
    field: &str,
    scenario_default: i32,
) -> Result<i32, ScenarioError> {
    let text = String::from_utf8_lossy(bytes);
    let mut in_parameters = false;
    let mut saw_parameters = false;

    for raw_line in legacy_ini_lines(&text) {
        let mut line = raw_line.trim();
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }
        if let Some(index) = line.find("//") {
            line = line[..index].trim_end();
        }
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let Some(section) = stdcompiler_ini_name(&line[1..line.len() - 1]) else {
                // Like CreateNameTree, an invalid header does not leave the
                // current section.
                continue;
            };
            if in_parameters {
                break;
            }
            in_parameters = section == "Parameters" && !saw_parameters;
            saw_parameters |= section == "Parameters";
            continue;
        }
        if !in_parameters {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let Some(key) = stdcompiler_ini_name(raw_key.trim()) else {
            continue;
        };
        if key != field {
            continue;
        }
        return parse_i32(raw_value.trim()).map_err(|error| {
            ScenarioError::LegacyParse(format!(
                "invalid Parameters.txt {field} value `{}`: {error}",
                raw_value.trim()
            ))
        });
    }

    Ok(scenario_default)
}

fn parse_legacy_scenario_text(text: &str) -> Result<LegacyScenarioManifest, ScenarioError> {
    const HEAD_KEYS: &[&str] = &[
        "Icon",
        "Title",
        "Description",
        "Loader",
        "Font",
        "Version",
        "Difficulty",
        "MaxPlayer",
        "MaxPlayerLeague",
        "MinPlayer",
        "SaveGame",
        "Replay",
        "Film",
        "DisableMouse",
        "NoInitialize",
        "RandomSeed",
        "ForcedAutoContextMenu",
        "ForcedAutoStopControl",
        "Engine",
        "MissionAccess",
        "NetworkGame",
        "NetworkRuntimeJoin",
        "ForcedGfxMode",
        "ForcedNoCrew",
        "DefCrewStrength",
        "Origin",
    ];
    const DEFINITION_KEYS: &[&str] = &[
        "LocalOnly",
        "AllowUserChange",
        "Definitions",
        "Definition1",
        "Definition2",
        "Definition3",
        "Definition4",
        "Definition5",
        "Definition6",
        "Definition7",
        "Definition8",
        "Definition9",
        "Definition10",
        "SkipDefs",
    ];
    const GAME_KEYS: &[&str] = &[
        "Mode",
        "Elimination",
        "CooperativeGoal",
        "CreateObjects",
        "ClearObjects",
        "ClearMaterials",
        "ValueGain",
        "EnableRemoveFlag",
        "StructNeedMaterial",
        "StructNeedEnergy",
        "ValueOverloads",
        "LandscapePushPull",
        "LandscapeInsertThrust",
        "BaseFunctionality",
        "BaseRegenerateEnergyPrice",
        "Goals",
        "Rules",
        "FoWColor",
    ];
    const PLAYER_KEYS: &[&str] = &[
        "StandardCrew",
        "Clonks",
        "Wealth",
        "Position",
        "EnforcePosition",
        "Crew",
        "Buildings",
        "Vehicles",
        "Material",
        "Knowledge",
        "HomeBaseMaterial",
        "HomeBaseProduction",
        "Magic",
    ];
    const LANDSCAPE_KEYS: &[&str] = &[
        "ExactLandscape",
        "Vegetation",
        "VegetationLevel",
        "InEarth",
        "InEarthLevel",
        "Sky",
        "SkyFade",
        "NoSky",
        "BottomOpen",
        "TopOpen",
        "LeftOpen",
        "RightOpen",
        "AutoScanSideOpen",
        "MapWidth",
        "MapHeight",
        "MapZoom",
        "Amplitude",
        "Phase",
        "Period",
        "Random",
        "Material",
        "Liquid",
        "LiquidLevel",
        "MapPlayerExtend",
        "Layers",
        "Gravity",
        "NoScan",
        "KeepMapCreator",
        "SkyScrollMode",
        "NewStyleLandscape",
        "FoWRes",
        "ShadeMaterials",
    ];
    const WEATHER_KEYS: &[&str] = &[
        "Climate",
        "StartSeason",
        "YearSpeed",
        "Rain",
        "Wind",
        "Lightning",
        "Precipitation",
        "NoGamma",
    ];

    let tree = LegacyIniTree::parse(text);
    let mut sections = HashMap::new();

    insert_validated_scenario_section::<LegacyHead>(
        &tree,
        &mut sections,
        "Head",
        "head",
        HEAD_KEYS,
        &["NetworkGame", "NetworkRuntimeJoin"],
        &["SaveGame", "Replay", "DisableMouse", "NoInitialize"],
        LegacyHead::apply_entries,
    );
    insert_validated_scenario_section::<LegacyDefinitions>(
        &tree,
        &mut sections,
        "Definitions",
        "definitions",
        DEFINITION_KEYS,
        &["LocalOnly", "AllowUserChange"],
        &[],
        LegacyDefinitions::apply_entries,
    );
    insert_validated_scenario_section::<LegacyGame>(
        &tree,
        &mut sections,
        "Game",
        "game",
        GAME_KEYS,
        &["EnableRemoveFlag", "StructNeedMaterial", "StructNeedEnergy"],
        &[],
        LegacyGame::apply_entries,
    );
    for player in 1..=MAX_PLAYER_STARTS {
        let source_name = format!("Player{player}");
        let storage_name = format!("player{player}");
        insert_validated_scenario_section::<LegacyPlayer>(
            &tree,
            &mut sections,
            &source_name,
            &storage_name,
            PLAYER_KEYS,
            &[],
            &["EnforcePosition"],
            LegacyPlayer::apply_entries,
        );
    }
    insert_validated_scenario_section::<LegacyLandscape>(
        &tree,
        &mut sections,
        "Landscape",
        "landscape",
        LANDSCAPE_KEYS,
        &[
            "ExactLandscape",
            "NoSky",
            "BottomOpen",
            "TopOpen",
            "AutoScanSideOpen",
            "MapPlayerExtend",
            "NoScan",
            "KeepMapCreator",
            "ShadeMaterials",
        ],
        &[],
        LegacyLandscape::apply_entries,
    );
    insert_validated_scenario_section::<LegacyWeather>(
        &tree,
        &mut sections,
        "Weather",
        "weather",
        WEATHER_KEYS,
        &["NoGamma"],
        &[],
        LegacyWeather::apply_entries,
    );
    insert_validated_scenario_section::<LegacyDisasters>(
        &tree,
        &mut sections,
        "Disasters",
        "disasters",
        &["Meteorite", "Volcano", "Earthquake"],
        &[],
        &[],
        LegacyDisasters::apply_entries,
    );
    insert_validated_scenario_section::<LegacyAnimals>(
        &tree,
        &mut sections,
        "Animals",
        "animals",
        &["Animal", "Nest"],
        &[],
        &[],
        LegacyAnimals::apply_entries,
    );
    insert_validated_scenario_section::<LegacyEnvironment>(
        &tree,
        &mut sections,
        "Environment",
        "environment",
        &["Objects"],
        &[],
        &[],
        LegacyEnvironment::apply_entries,
    );

    let title = sections
        .get("head")
        .and_then(|entries| find_rct_all_entry(entries, "Title"));
    let description = sections
        .get("head")
        .and_then(|entries| find_rct_all_entry(entries, "Description"));

    let ground_height_hint = derive_ground_height_hint(&sections);
    let mut core = LegacyScenarioCore::from_sections(&sections)?;
    apply_scenario_rct_all_strings(&mut core, &sections, true);
    let definition_specs = core.definitions.definitions.clone();

    Ok(LegacyScenarioManifest {
        title,
        description,
        head_title_native: None,
        definition_specs,
        ground_height_hint,
        core,
        sections,
    })
}

fn insert_validated_scenario_section<T: Default>(
    tree: &LegacyIniTree,
    sections: &mut HashMap<String, Vec<(String, String)>>,
    source_name: &str,
    storage_name: &str,
    allowed_keys: &[&str],
    bool_keys: &[&str],
    integer_bool_keys: &[&str],
    apply: fn(&mut T, &[(String, String)]) -> Result<(), ScenarioError>,
) {
    let Some(section) = tree.first_section(0, source_name) else {
        return;
    };
    let allowed = allowed_keys.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for child in tree.nodes[section].children.iter().copied() {
        let node = &tree.nodes[child];
        if node.section || !allowed.contains(node.name.as_str()) || !seen.insert(node.name.clone())
        {
            continue;
        }
        let value = node.value.clone().unwrap_or_default();
        if bool_keys.contains(&node.name.as_str()) && parse_std_bool(&value).is_none() {
            continue;
        }
        if integer_bool_keys.contains(&node.name.as_str()) && parse_std_i32(&value).is_none() {
            continue;
        }
        let entry = (node.name.clone(), value);
        let mut probe = T::default();
        if apply(&mut probe, std::slice::from_ref(&entry)).is_ok() {
            entries.push(entry);
        }
    }
    sections.insert(storage_name.to_string(), entries);
}

fn find_rct_all_entry(entries: &[(String, String)], key: &str) -> Option<String> {
    entries
        .iter()
        .find(|(entry_key, _)| entry_key == key)
        .map(|(_, value)| parse_rct_all(value))
        .filter(|value| !value.is_empty())
}

fn apply_scenario_rct_all_strings(
    core: &mut LegacyScenarioCore,
    sections: &HashMap<String, Vec<(String, String)>>,
    compile_head: bool,
) {
    let raw = |section: &str, key: &str| {
        sections.get(section).and_then(|entries| {
            entries
                .iter()
                .find(|(entry_key, _)| entry_key == key)
                .map(|(_, value)| parse_rct_all(value))
        })
    };

    if compile_head {
        if let Some(value) = raw("head", "Title") {
            core.head.title = value;
        }
        if let Some(value) = raw("head", "Loader") {
            core.head.loader = value;
        }
        if let Some(value) = raw("head", "Font") {
            core.head.font = value;
        }
        if let Some(value) = raw("head", "Engine") {
            core.head.engine = value;
        }
        if let Some(value) = raw("head", "MissionAccess") {
            core.head.mission_access = truncate_legacy_c4_string(value, 512);
        }
        if let Some(value) = raw("head", "Origin") {
            core.head.origin = Some(validate_subpath_filename(value));
        }
    }
    if let Some(value) = raw("landscape", "Sky") {
        core.landscape.sky = (!value.is_empty()).then_some(value);
    }
    if let Some(value) = raw("landscape", "Material") {
        core.landscape.material = value;
    }
    if let Some(value) = raw("landscape", "Liquid") {
        core.landscape.liquid = value;
    }
    if let Some(value) = raw("weather", "Precipitation") {
        core.weather.precipitation = value;
    }
}

/// `C4InVal::VAL_SubPathFilename` plus C4SHead's platform separator
/// normalization (`C4Scenario.cpp:200-202`). Validation mutates bad input
/// rather than rejecting the scenario.
fn validate_subpath_filename(mut value: String) -> String {
    if value.is_empty() {
        value = "empty".to_string();
    }
    value = value.replace("..", "__");
    if value.starts_with('/') || value.starts_with('\\') {
        value.replace_range(..1, "_");
    }
    value = value
        .chars()
        .map(|character| match character {
            '*' | '?' | '<' | '>' | ';' | '|' | ':' => '_',
            '\\' if cfg!(not(windows)) => '/',
            '/' if cfg!(windows) => '\\',
            other => other,
        })
        .collect();
    value
}

fn find_entry(entries: &[(String, String)], key: &str) -> Option<String> {
    find_entry_including_empty(entries, key)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn find_entry_including_empty<'a>(entries: &'a [(String, String)], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.trim())
}

fn normalize_definition_path(raw: &str) -> String {
    let mut trimmed = raw.trim().trim_matches(['"', '\''].as_ref());
    while let Some(stripped) = trimmed.strip_prefix("./") {
        trimmed = stripped;
    }
    while let Some(stripped) = trimmed.strip_prefix(".\\") {
        trimmed = stripped;
    }
    let normalized = trimmed.replace('\\', "/");
    normalized.trim_end_matches('/').to_string()
}

fn derive_ground_height_hint(sections: &HashMap<String, Vec<(String, String)>>) -> Option<i32> {
    let landscape = sections.get("landscape")?;
    let height = landscape
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("MapHeight"))
        .and_then(|(_, value)| parse_c4sval_std(value));
    let zoom = landscape
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("MapZoom"))
        .and_then(|(_, value)| parse_c4sval_std(value))
        .unwrap_or(1);
    height.map(|h| h.max(0).saturating_mul(zoom.max(1)))
}

fn parse_c4sval_std(value: &str) -> Option<i32> {
    let first = value.split(',').next()?.trim();
    if first.is_empty() {
        None
    } else {
        first.parse::<i32>().ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyC4SVal {
    pub std: i32,
    pub rnd: i32,
    pub min: i32,
    pub max: i32,
}

impl LegacyC4SVal {
    pub const fn new(std: i32, rnd: i32, min: i32, max: i32) -> Self {
        Self { std, rnd, min, max }
    }

    pub(crate) fn base(self) -> i32 {
        let (min, max) = ordered_bounds(self.min, self.max);
        self.std.clamp(min, max)
    }

    /// `C4SVal::Evaluate` (C4Scenario.cpp:43-46): one synced game-RNG draw,
    /// `BoundBy(Std + Random(2 * Rnd + 1) - Rnd, Min, Max)`. BoundBy makes no
    /// ordered-bounds assumption (Standard.h), so this avoids `clamp`'s
    /// min<=max panic.
    pub fn evaluate(self, rng: &mut crate::rng::LcgRng) -> i32 {
        let value = self.std + rng.random(2 * self.rnd + 1) - self.rnd;
        if value < self.min {
            self.min
        } else if value > self.max {
            self.max
        } else {
            value
        }
    }
}

const fn ordered_bounds(a: i32, b: i32) -> (i32, i32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn parse_legacy_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// `C4ComponentHost::LoadAppend` copies at most two native bytes from each
/// comma-separated language segment (C4ComponentHost.cpp:174-184).
fn legacy_script_language_code(language: &str) -> String {
    let code = clonk_script::c4_string_bytes(language);
    let visible = code
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(code.len());
    clonk_script::c4_string_from_bytes(&code[..visible.min(2)])
}

fn load_legacy_scenario_script<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Option<ScenarioScriptSource>, ScenarioError> {
    // C4CFN_Script is three independent LoadAppend segments. Each localized
    // segment restarts LanguageEx priority, and a failed read advances to the
    // next language without making scenario startup fail. The empty language
    // string still contributes one empty code, selecting Script.c a second
    // time through the Script{}.c segment (C4Components.h:55;
    // C4ComponentHost.cpp:155-220).
    const SCRIPT_SEGMENTS: [&str; 3] = ["Script.c", "Script{}.c", "C4Script{}.c"];
    let language_codes = languages
        .iter()
        .map(|language| legacy_script_language_code(language.as_ref()))
        .collect::<Vec<_>>();
    let mut assembled = Vec::new();
    for segment in SCRIPT_SEGMENTS {
        let selected = if segment.contains("{}") {
            if language_codes.is_empty() {
                group.read_file(segment.replacen("{}", "", 1)).ok()
            } else {
                language_codes
                    .iter()
                    .find_map(|code| group.read_file(segment.replacen("{}", code, 1)).ok())
            }
        } else {
            group.read_file(segment).ok()
        };
        let Some(bytes) = selected else {
            continue;
        };

        // LoadAppend prefixes every successfully read component, including
        // an empty one, and SCopy truncates only that component at its first
        // NUL before later segments are appended.
        assembled.push(b'\n');
        assembled.extend_from_slice(bytes.split(|byte| *byte == 0).next().unwrap_or_default());
    }

    let source = clonk_script::c4_string_from_bytes(&assembled);
    // C4ScriptHost passes the same two-byte LanguageEx segments to its
    // C4LangStringTable after component assembly.
    let source = localize_script_source_with_components(components, &source, &language_codes)?;
    // C4GameScriptHost exists even when every optional component is absent.
    // Retain that empty host and the canonical Script.c diagnostic name so
    // DirectExec/eval does not fall back to Game.ScriptEngine.
    Ok(Some(ScenarioScriptSource {
        name: group.root().join("Script.c").to_string_lossy().into_owned(),
        source,
        c4_args: true,
    }))
}

/// Byte-preserving C4LangStringTable::ReplaceStrings for Teams.txt. Unlike
/// C4ComponentHost, C4TeamList::Load does not call EnsureUnicode, so both the
/// source and replacement values remain in their original byte encoding
/// (C4Teams.cpp:614-655; C4LangStringTable.cpp:33-148).
fn localize_legacy_team_source<S: AsRef<str>>(
    components: &ComponentGroups,
    source: &[u8],
    languages: &[S],
) -> Result<Vec<u8>, GroupError> {
    let mut table = None;
    for candidate in std::iter::once("StringTbl.txt".to_owned()).chain(
        languages
            .iter()
            .map(|language| format!("StringTbl{}.txt", language.as_ref())),
    ) {
        if let Some(component) = components.read(candidate)? {
            table = Some(component.bytes);
            break;
        }
    }
    let Some(table) = table else {
        return Ok(source
            .split(|byte| *byte == 0)
            .next()
            .unwrap_or_default()
            .to_vec());
    };
    let table = table.split(|byte| *byte == 0).next().unwrap_or_default();
    let mut entries: Vec<(&[u8], &[u8])> = Vec::new();
    for line in table.split(|byte| matches!(*byte, b'\r' | b'\n')) {
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = &line[..separator];
        if entries.iter().any(|(existing, _)| *existing == key) {
            continue;
        }
        entries.push((key, &line[separator + 1..]));
    }

    let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
    let mut localized = Vec::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(open_offset) = source[cursor..].iter().position(|byte| *byte == b'$') {
        let open = cursor + open_offset;
        let key_start = open + 1;
        let Some(close_offset) = source[key_start..].iter().position(|byte| *byte == b'$') else {
            break;
        };
        let close = key_start + close_offset;
        let key = &source[key_start..close];
        let valid = key.len() <= 30
            && key.iter().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'~' | b'+' | b'-')
            });
        localized.extend_from_slice(&source[cursor..open]);
        if valid {
            if let Some((_, replacement)) = entries.iter().find(|(entry, _)| *entry == key) {
                localized.extend_from_slice(replacement);
            } else {
                localized.extend_from_slice(&source[open..=close]);
            }
        } else {
            localized.extend_from_slice(&source[open..=close]);
        }
        cursor = close + 1;
    }
    localized.extend_from_slice(&source[cursor..]);
    Ok(localized)
}

fn load_initial_network_teams<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
) -> Result<(Vec<TeamInfo>, Option<LoadedLegacyTeamMetadata>), ScenarioError> {
    if !group.exists("Teams.txt") {
        return Ok((Vec::new(), None));
    }
    let source = group.read_file("Teams.txt")?;
    if source.is_empty() {
        // LoadEntryString rejects a zero-sized entry, selecting the same
        // scenario-derived path as a missing Teams.txt (C4Group.cpp:2243-2259;
        // C4Teams.cpp:619-647).
        return Ok((Vec::new(), None));
    }
    let source = localize_legacy_team_source(components, &source, languages)?;
    let source = bytes_as_latin1_string(&source);
    let loaded = parse_legacy_team_metadata_source(&source)?;
    let teams = team_infos_from_initial_network_metadata(&loaded.metadata);
    Ok((teams, Some(loaded)))
}

fn team_infos_from_initial_network_metadata(
    metadata: &InitialNetworkTeamMetadata,
) -> Vec<TeamInfo> {
    metadata
        .teams
        .iter()
        .map(|team| {
            TeamInfo::new(
                team.id,
                clonk_script::c4_string_from_bytes(team.name.as_bytes()),
                team.color,
            )
            .with_player_ids(
                team.player_ids
                    .iter()
                    .copied()
                    .filter(|player_id| *player_id > 0)
                    .collect(),
            )
            .with_player_start_index(team.player_start_index)
            .with_max_players(team.max_players)
            .with_icon_spec(clonk_script::c4_string_from_bytes(
                team.icon_spec.as_bytes(),
            ))
        })
        .collect()
}

fn apply_initial_network_team_strings(
    lobby: &mut ScenarioLobbyTeams,
    metadata: &InitialNetworkTeamMetadata,
) {
    lobby.script_player_names =
        clonk_script::c4_string_from_bytes(metadata.script_player_names.as_bytes());
    for (lobby_team, team) in lobby.teams.iter_mut().zip(&metadata.teams) {
        lobby_team.name = clonk_script::c4_string_from_bytes(team.name.as_bytes());
        let icon_spec = clonk_script::c4_string_from_bytes(team.icon_spec.as_bytes());
        lobby_team.icon_spec = (!icon_spec.is_empty()).then_some(icon_spec);
    }
}

#[derive(Default)]
struct LegacyTeamBuilder {
    id: i32,
    name: Vec<u8>,
    player_start_index: i32,
    player_count: i32,
    player_ids: Vec<i32>,
    color: u32,
    icon_spec: Vec<u8>,
    max_players: i32,
}

impl LegacyTeamBuilder {
    fn finish(self) -> Result<InitialNetworkTeam, ScenarioError> {
        let player_count = usize::try_from(self.player_count).map_err(|_| {
            ScenarioError::LegacyParse(format!(
                "Teams.txt team {} has negative PlayerCount {}",
                self.id, self.player_count
            ))
        })?;
        let mut player_ids = vec![-1; player_count];
        for (target, source) in player_ids.iter_mut().zip(self.player_ids) {
            *target = source;
        }
        Ok(InitialNetworkTeam {
            id: self.id,
            name: team_legacy_cstring(truncate_team_name(self.name), "Name")?,
            player_start_index: self.player_start_index,
            player_ids,
            color: self.color,
            icon_spec: team_legacy_cstring(self.icon_spec, "IconSpec")?,
            max_players: self.max_players,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyTeamSection {
    None,
    Teams,
    Team(usize),
    Other(usize),
}

fn parse_legacy_team_metadata_source(
    source: &str,
) -> Result<LoadedLegacyTeamMetadata, ScenarioError> {
    let mut metadata = InitialNetworkTeamMetadata::teams_file_defaults();
    let mut section = LegacyTeamSection::None;
    let mut teams_indent = None;
    let mut current_team: Option<LegacyTeamBuilder> = None;
    let mut unsupported_team_distribution = None;

    for (index, raw_line) in legacy_ini_lines(source).enumerate() {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        if line.is_empty()
            || line.starts_with(';')
            || line.starts_with('#')
            || line.starts_with("//")
        {
            continue;
        }
        if let Some(section_name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.split_once(']').map(|(name, _)| name))
        {
            if let Some(team) = current_team.take() {
                metadata.teams.push(team.finish()?);
            }
            section = if section_name == "Teams" {
                teams_indent = Some(indent);
                LegacyTeamSection::Teams
            } else if section_name == "Team"
                && teams_indent.is_some_and(|teams_indent| indent > teams_indent)
            {
                current_team = Some(LegacyTeamBuilder::default());
                LegacyTeamSection::Team(indent)
            } else if teams_indent.is_some_and(|teams_indent| indent > teams_indent) {
                LegacyTeamSection::Other(indent)
            } else {
                teams_indent = None;
                LegacyTeamSection::None
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let line_number = index + 1;
        match section {
            LegacyTeamSection::Teams
                if teams_indent.is_some_and(|teams_indent| indent + 1 > teams_indent) =>
            {
                apply_legacy_team_list_field(
                    &mut metadata,
                    &mut unsupported_team_distribution,
                    key,
                    value,
                    line_number,
                )?;
            }
            LegacyTeamSection::Team(team_indent) if indent + 1 > team_indent => {
                if let Some(team) = current_team.as_mut() {
                    apply_legacy_team_field(team, key, value, line_number)?;
                }
            }
            LegacyTeamSection::Other(child_indent) if indent + 1 > child_indent => {}
            LegacyTeamSection::Team(_) | LegacyTeamSection::Other(_) => {
                if let Some(team) = current_team.take() {
                    metadata.teams.push(team.finish()?);
                }
                section = LegacyTeamSection::Teams;
                if teams_indent.is_some_and(|teams_indent| indent + 1 > teams_indent) {
                    apply_legacy_team_list_field(
                        &mut metadata,
                        &mut unsupported_team_distribution,
                        key,
                        value,
                        line_number,
                    )?;
                }
            }
            LegacyTeamSection::None | LegacyTeamSection::Teams => {}
        }
    }
    if let Some(team) = current_team {
        metadata.teams.push(team.finish()?);
    }

    let largest_team_id = metadata.teams.iter().map(|team| team.id).fold(0, i32::max);
    metadata.last_team_id = metadata.last_team_id.max(largest_team_id);
    if metadata.teams.is_empty() {
        metadata.auto_generate_teams = true;
    }

    const DEFAULT_TEAM_COLORS: [u32; 10] = [
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
    let mut random_color_team_id = None;
    for team in &mut metadata.teams {
        if team.color != 0 {
            continue;
        }
        if let Some(color) = team
            .id
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| DEFAULT_TEAM_COLORS.get(index))
        {
            team.color = *color;
        } else if random_color_team_id.is_none() {
            // C++ calls process-global SafeRandom here, so scenario data alone
            // cannot reproduce the host's chosen snapshot color exactly
            // (C4Teams.cpp:181-218; C4PlayerInfoConflicts.cpp:36-41).
            random_color_team_id = Some(team.id);
        }
    }

    Ok(LoadedLegacyTeamMetadata {
        metadata,
        random_color_team_id,
        unsupported_team_distribution,
    })
}

fn apply_legacy_team_list_field(
    metadata: &mut InitialNetworkTeamMetadata,
    unsupported_team_distribution: &mut Option<u8>,
    key: &str,
    value: &str,
    line: usize,
) -> Result<(), ScenarioError> {
    match key {
        "Active" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.active = value;
            }
        }
        "Custom" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.custom = value;
            }
        }
        "AllowHostilityChange" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.allow_hostility_change = value;
            }
        }
        "AllowTeamSwitch" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.allow_team_switch = value;
            }
        }
        "AutoGenerateTeams" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.auto_generate_teams = value;
            }
        }
        "LastTeamID" => metadata.last_team_id = parse_team_i32(key, value, line)?,
        "TeamDistribution" => {
            let (distribution, unsupported) = parse_team_distribution(value);
            if let Some(distribution) = distribution {
                metadata.team_distribution = distribution;
            }
            if unsupported.is_some() {
                *unsupported_team_distribution = unsupported;
            }
        }
        "TeamColors" => {
            if let Some(value) = parse_team_bool(value) {
                metadata.team_colors = value;
            }
        }
        "MaxScriptPlayers" => {
            metadata.max_script_players = parse_team_i32(key, value, line)?;
        }
        "ScriptPlayerNames" => {
            metadata.script_player_names = team_legacy_cstring(
                parse_team_escaped_bytes(value, line, key)?,
                "ScriptPlayerNames",
            )?;
        }
        "RandomTeamCount" => {
            metadata.random_team_count = parse_team_i32(key, value, line)?;
        }
        _ => {}
    }
    Ok(())
}

fn apply_legacy_team_field(
    team: &mut LegacyTeamBuilder,
    key: &str,
    value: &str,
    line: usize,
) -> Result<(), ScenarioError> {
    match key {
        "id" => team.id = parse_team_i32(key, value, line)?,
        "Name" => {
            team.name = latin1_string_as_bytes(value.trim_start_matches([' ', '\t']), line, key)?;
        }
        "PlrStartIndex" => team.player_start_index = parse_team_i32(key, value, line)?,
        "PlayerCount" => team.player_count = parse_team_i32(key, value, line)?,
        "Players" => {
            team.player_ids = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| parse_team_i32(key, value, line))
                .collect::<Result<_, _>>()?;
        }
        "Color" => {
            let parsed = parse_i64(value).map_err(|error| {
                team_parse_error(line, format!("invalid {key} `{value}`: {error}"))
            })?;
            team.color = u32::try_from(parsed).map_err(|_| {
                team_parse_error(line, format!("{key} `{value}` is outside uint32"))
            })?;
        }
        "IconSpec" => team.icon_spec = parse_team_escaped_bytes(value, line, key)?,
        "MaxPlayer" => team.max_players = parse_team_i32(key, value, line)?,
        _ => {}
    }
    Ok(())
}

fn parse_team_bool(value: &str) -> Option<bool> {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'1') && bytes.get(1).is_none_or(|byte| !byte.is_ascii_digit()) {
        Some(true)
    } else if bytes.first() == Some(&b'0') && bytes.get(1).is_none_or(|byte| !byte.is_ascii_digit())
    {
        Some(false)
    } else if bytes.starts_with(b"true") {
        Some(true)
    } else if bytes.starts_with(b"false") {
        Some(false)
    } else {
        None
    }
}

fn load_legacy_teams<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
    core: &LegacyScenarioCore,
) -> Result<(Vec<TeamInfo>, ScenarioLobbyTeams), ScenarioError> {
    if !group.exists("Teams.txt") {
        return Ok((Vec::new(), derive_legacy_teams_default(core)));
    }
    let source = group.read_file("Teams.txt")?;
    if source.is_empty() {
        // C4Group::LoadEntryString rejects a zero-sized entry, so the lobby
        // projection must take the same scenario-derived branch as runtime.
        return Ok((Vec::new(), derive_legacy_teams_default(core)));
    }
    let source = localize_legacy_team_source(components, &source, languages)?;
    // C4Group::LoadEntryString and C4LangStringTable::ReplaceStrings keep
    // Teams.txt as native bytes. Parse the decoded projection only for the
    // existing lobby/configuration semantics, then replace every C4 string
    // with the byte-exact values used by C4TeamList and the script runtime.
    let exact = parse_legacy_team_metadata_source(&bytes_as_latin1_string(&source))?;
    let mut metadata = parse_legacy_teams_source(&decode_legacy_script_text(&source));
    apply_initial_network_team_strings(&mut metadata, &exact.metadata);
    let teams = team_infos_from_initial_network_metadata(&exact.metadata);
    Ok((teams, metadata))
}

#[derive(Debug)]
struct LegacyIniNode {
    name: String,
    value: Option<String>,
    section: bool,
    indent: isize,
    parent: Option<usize>,
    children: Vec<usize>,
}

#[derive(Debug)]
struct LegacyIniTree {
    nodes: Vec<LegacyIniNode>,
}

impl LegacyIniTree {
    fn parse(source: &str) -> Self {
        let mut tree = Self {
            nodes: vec![LegacyIniNode {
                name: String::new(),
                value: None,
                section: true,
                indent: -1,
                parent: None,
                children: Vec::new(),
            }],
        };
        let mut current = 0;
        for line in legacy_ini_lines(source) {
            let indent = line
                .as_bytes()
                .iter()
                .take_while(|byte| matches!(**byte, b' ' | b'\t'))
                .count();
            let bytes = line.as_bytes();
            let mut position = indent;
            let section = bytes.get(position) == Some(&b'[')
                && bytes.get(position + 1).is_some_and(u8::is_ascii_alphabetic);
            if section {
                position += 1;
            } else if !bytes.get(position).is_some_and(u8::is_ascii_alphabetic) {
                continue;
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
            let expected = if section { b']' } else { b'=' };
            if bytes.get(position) != Some(&expected) {
                continue;
            }
            position += 1;
            let node_indent = (indent + usize::from(!section)) as isize;
            while current != 0 && tree.nodes[current].indent >= node_indent {
                current = tree.nodes[current].parent.unwrap_or(0);
            }
            let index = tree.nodes.len();
            tree.nodes.push(LegacyIniNode {
                name: name.to_string(),
                value: (!section).then(|| line[position..].to_string()),
                section,
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

    fn first_section(&self, parent: usize, name: &str) -> Option<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|index| self.nodes[*index].section && self.nodes[*index].name == name)
    }

    fn sections(&self, parent: usize, name: &str) -> impl Iterator<Item = usize> + '_ {
        let name = name.to_string();
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .filter(move |index| self.nodes[*index].section && self.nodes[*index].name == name)
    }

    fn value(&self, parent: usize, name: &str) -> Option<&str> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|index| !self.nodes[*index].section && self.nodes[*index].name == name)
            .and_then(|index| self.nodes[index].value.as_deref())
    }

    fn has_value(&self, parent: usize, name: &str) -> bool {
        self.value(parent, name).is_some()
    }
}

fn parse_legacy_teams_source(source: &str) -> ScenarioLobbyTeams {
    let tree = LegacyIniTree::parse(source);
    let Some(section) = tree.first_section(0, "Teams") else {
        return ScenarioLobbyTeams {
            source: ScenarioTeamsSource::TeamsFile,
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            configured_auto_generate: false,
            auto_generate: true,
            configured_last_team_id: 0,
            last_team_id: 0,
            distribution: ScenarioTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: String::new(),
            random_team_count: 0,
            teams: Vec::new(),
        };
    };
    let configured_auto_generate = ini_bool(&tree, section, "AutoGenerateTeams", false);
    let configured_last_team_id = ini_i32(&tree, section, "LastTeamID", 0);
    let mut teams = Vec::new();
    for team_section in tree.sections(section, "Team") {
        let player_count = ini_i32(&tree, team_section, "PlayerCount", 0);
        teams.push(ScenarioLobbyTeam {
            id: ini_i32(&tree, team_section, "id", 0),
            name: truncate_legacy_string(ini_rct_all(&tree, team_section, "Name", ""), 30),
            player_start_index: ini_i32(&tree, team_section, "PlrStartIndex", 0),
            player_count,
            players: ini_i32_array(&tree, team_section, "Players", player_count, -1),
            configured_color: ini_u32(&tree, team_section, "Color", 0),
            icon_spec: {
                let value = ini_std_string(&tree, team_section, "IconSpec", "");
                (!value.is_empty()).then_some(value)
            },
            max_players: ini_i32(&tree, team_section, "MaxPlayer", 0),
        });
    }
    let mut metadata = ScenarioLobbyTeams {
        source: ScenarioTeamsSource::TeamsFile,
        active: ini_bool(&tree, section, "Active", true),
        custom: ini_bool(&tree, section, "Custom", true),
        allow_hostility_change: ini_bool(&tree, section, "AllowHostilityChange", false),
        allow_team_switch: ini_bool(&tree, section, "AllowTeamSwitch", false),
        configured_auto_generate,
        auto_generate: configured_auto_generate,
        configured_last_team_id,
        last_team_id: configured_last_team_id,
        distribution: ini_team_distribution(&tree, section),
        team_colors: ini_bool(&tree, section, "TeamColors", false),
        max_script_players: ini_i32(&tree, section, "MaxScriptPlayers", 0),
        script_player_names: ini_std_string(&tree, section, "ScriptPlayerNames", ""),
        random_team_count: ini_i32(&tree, section, "RandomTeamCount", 0),
        teams,
    };

    // C4TeamList::CompileFunc performs these two post-compile adjustments.
    if metadata.teams.is_empty() {
        metadata.auto_generate = true;
    }
    let largest = metadata.teams.iter().map(|team| team.id).fold(0, i32::max);
    metadata.last_team_id = metadata.last_team_id.max(largest);
    metadata
}

fn derive_legacy_teams_default(core: &LegacyScenarioCore) -> ScenarioLobbyTeams {
    let melee = matches!(core.game.mode, 1 | 2)
        || core.game.goals.iter().any(|entry| {
            entry.id.eq_ignore_ascii_case("MELE") || entry.id.eq_ignore_ascii_case("MEL2")
        })
        || core
            .game
            .rules
            .iter()
            .any(|entry| entry.id.eq_ignore_ascii_case("RVLR"));
    ScenarioLobbyTeams {
        source: ScenarioTeamsSource::DerivedScenarioDefault,
        active: melee,
        custom: false,
        allow_hostility_change: true,
        allow_team_switch: false,
        configured_auto_generate: false,
        auto_generate: melee,
        configured_last_team_id: 0,
        last_team_id: 0,
        distribution: ScenarioTeamDistribution::Free,
        team_colors: false,
        max_script_players: 0,
        script_player_names: String::new(),
        random_team_count: 0,
        teams: Vec::new(),
    }
}

fn legacy_game_is_melee_after_conversion(game: &LegacyGame) -> bool {
    matches!(game.mode, 1 | 2)
        || ["MELE", "MEL2"]
            .iter()
            .any(|id| first_legacy_id_count(&game.goals, id, 0) != 0)
}

fn legacy_effective_min_players(core: &LegacyScenarioCore) -> i32 {
    if core.head.min_player != 0 {
        core.head.min_player
    } else if legacy_game_is_melee_after_conversion(&core.game) {
        2
    } else {
        1
    }
}

fn load_legacy_game_parameter_overrides(
    group: &Group,
    defaults: &ScenarioGameParameterValues,
) -> Result<Option<ScenarioGameParameterOverrides>, ScenarioError> {
    if !group.exists("Parameters.txt") {
        return Ok(None);
    }
    let source = decode_legacy_script_text(&group.read_file("Parameters.txt")?);
    Ok(Some(parse_legacy_game_parameter_overrides(
        &source, defaults,
    )))
}

fn load_savegame_definition_override(
    group: &Group,
    save_game: bool,
) -> Result<ScenarioSavegameDefinitionOverride, ScenarioError> {
    if !save_game {
        return Ok(ScenarioSavegameDefinitionOverride::None);
    }
    let Some(bytes) = try_read_group_file_case_insensitive(group, "Game.txt")? else {
        return Ok(ScenarioSavegameDefinitionOverride::None);
    };
    let source = decode_legacy_script_text(&bytes);
    let Some(position) = source.find("[DefinitionFiles]") else {
        return Ok(ScenarioSavegameDefinitionOverride::None);
    };
    let mut definition_lines = Vec::new();
    let mut found = false;
    for line in source[position..].lines().skip(1) {
        if line.starts_with("Definition") && line.contains('=') {
            found = true;
            definition_lines.push(line.to_string());
        } else if found {
            break;
        }
    }
    Ok(ScenarioSavegameDefinitionOverride::GameText { definition_lines })
}

fn load_runtime_landscape_data(
    group: &Group,
    savegame_defaults: bool,
) -> Result<Option<LandscapeGameData>, ScenarioError> {
    Ok(
        match try_read_group_file_case_insensitive(group, "Game.txt")? {
            Some(bytes) => Some(parse_landscape_game_data(&bytes)),
            None if savegame_defaults => Some(LandscapeGameData::default()),
            None => None,
        },
    )
}

fn load_runtime_current_scenario_section(group: &Group) -> Result<String, ScenarioError> {
    let current = try_read_group_file_case_insensitive(group, "Game.txt")?
        .map(|bytes| crate::parse_initial_network_game_data(&bytes).current_scenario_section)
        .unwrap_or_default();
    Ok(if current.is_empty() {
        "main".to_string()
    } else {
        current
    })
}

fn load_legacy_round_results(
    group: &Group,
    melee: bool,
) -> Result<RoundResultsState, ScenarioError> {
    let Some(source) = try_read_group_file_case_insensitive(group, "RoundResults.txt")? else {
        // C4Game calls RoundResults.Init when the component is absent. Init
        // changes only this scenario-derived default on a freshly-cleared
        // game instance (C4Game.cpp:2477-2486; C4RoundResults.cpp:240-245).
        return Ok(RoundResultsState {
            hide_settlement_score: melee,
            ..RoundResultsState::default()
        });
    };

    RoundResultsState::from_legacy_ini(&source, melee)
        .map_err(ScenarioError::LegacyRoundResultsParse)
}

fn parse_legacy_game_parameter_overrides(
    source: &str,
    defaults: &ScenarioGameParameterValues,
) -> ScenarioGameParameterOverrides {
    let tree = LegacyIniTree::parse(source);
    let section = tree.first_section(0, "Parameters");
    let mut overrides = ScenarioGameParameterOverrides {
        random_seed: None,
        max_players: None,
        startup_player_count: None,
        use_fair_crew: None,
        fair_crew_forced: None,
        fair_crew_strength: None,
        allow_debug: None,
        is_network_game: None,
        control_rate: None,
        auto_frame_skip: None,
        rules: None,
        goals: None,
        league: None,
        clients: Vec::new(),
    };
    let Some(section) = section else {
        return overrides;
    };
    overrides.random_seed = ini_optional_i32(&tree, section, "RandomSeed", defaults.random_seed);
    overrides.startup_player_count = ini_optional_i32(
        &tree,
        section,
        "StartupPlayerCount",
        defaults.startup_player_count,
    );
    overrides.max_players = ini_optional_i32(&tree, section, "MaxPlayers", defaults.max_players);
    overrides.use_fair_crew =
        ini_optional_bool(&tree, section, "UseFairCrew", defaults.use_fair_crew);
    overrides.fair_crew_forced =
        ini_optional_bool(&tree, section, "FairCrewForced", defaults.fair_crew_forced);
    overrides.fair_crew_strength = ini_optional_i32(
        &tree,
        section,
        "FairCrewStrength",
        defaults.fair_crew_strength,
    );
    overrides.allow_debug = ini_optional_bool(&tree, section, "AllowDebug", defaults.allow_debug);
    overrides.is_network_game =
        ini_optional_bool(&tree, section, "IsNetworkGame", defaults.is_network_game);
    overrides.control_rate = ini_optional_i32(&tree, section, "ControlRate", defaults.control_rate);
    overrides.auto_frame_skip =
        ini_optional_bool(&tree, section, "AutoFrameSkip", defaults.auto_frame_skip);
    overrides.rules = ini_optional_id_list(&tree, section, "Rules", &defaults.rules);
    overrides.goals = ini_optional_id_list(&tree, section, "Goals", &defaults.goals);
    overrides.league = tree
        .has_value(section, "League")
        .then(|| ini_std_string(&tree, section, "League", &defaults.league));
    overrides.clients = tree
        .sections(section, "Client")
        .map(|client| ScenarioLobbyClient {
            id: ini_i32(&tree, client, "ID", -1),
            activated: ini_bool(&tree, client, "Activated", false),
            observer: ini_bool(&tree, client, "Observer", false),
            name: ini_validated_client_name(&tree, client, "Name"),
            nick: ini_validated_client_name(&tree, client, "Nick"),
            lobby_ready: ini_bool(&tree, client, "LobbyReady", false),
        })
        .collect();
    // C4ClientList::Add inserts each compiled client by ascending ID.
    overrides.clients.sort_by_key(ScenarioLobbyClient::id);
    overrides
}

fn game_parameter_defaults(core: &LegacyScenarioCore) -> ScenarioGameParameterValues {
    let (rules, goals) = converted_legacy_rules_and_goals(&core.game);
    ScenarioGameParameterValues {
        random_seed: core.head.random_seed,
        startup_player_count: 0,
        max_players: core.head.max_player,
        use_fair_crew: core.head.forced_fair_crew == 1,
        fair_crew_forced: core.head.forced_fair_crew != 0,
        fair_crew_strength: core.head.fair_crew_strength,
        allow_debug: true,
        is_network_game: false,
        control_rate: -1,
        auto_frame_skip: false,
        rules: lobby_id_entries(&rules),
        goals: lobby_id_entries(&goals),
        league: String::new(),
        clients: Vec::new(),
    }
}

fn converted_legacy_rules_and_goals(game: &LegacyGame) -> (LegacyIdList, LegacyIdList) {
    let mut rules = game.rules.clone();
    let mut goals = game.goals.clone();
    if matches!(game.mode, 1 | 2) {
        set_first_legacy_id(&mut goals, "MELE", 1);
    }
    match game.cooperative_goal {
        1 => set_first_legacy_id(&mut goals, "GLDM", 1),
        2 => set_first_legacy_id(&mut goals, "MNTK", 1),
        3 => set_first_legacy_id(&mut goals, "VALG", (game.value_gain / 100).max(1)),
        _ => {}
    }
    if game.realism.construction_needs_material {
        set_first_legacy_id(&mut rules, "CNMT", 1);
    }
    if game.realism.structures_need_energy {
        set_first_legacy_id(&mut rules, "ENRG", 1);
    }
    if game.enable_remove_flag {
        set_first_legacy_id(&mut rules, "FGRV", 1);
    }
    match game.elimination {
        0 => set_first_legacy_id(&mut rules, "KILC", 1),
        2 => {
            set_first_legacy_id(&mut rules, "CTFL", 1);
            set_first_legacy_id(&mut rules, "FGRV", 1);
        }
        _ => {}
    }
    if first_legacy_id_count(&rules, "CTFL", 0) != 0 {
        set_first_legacy_id(&mut rules, "FGRV", 1);
    }
    (rules, goals)
}

fn first_legacy_id_count(list: &LegacyIdList, id: &str, zero_default: i32) -> i32 {
    list.iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(id))
        .map_or(0, |entry| match entry.count.unwrap_or(0) {
            0 => zero_default,
            count => count,
        })
}

fn set_first_legacy_id(list: &mut LegacyIdList, id: &str, count: i32) {
    if let Some(entry) = list
        .iter_mut()
        .find(|entry| entry.id.eq_ignore_ascii_case(id))
    {
        entry.count = Some(count);
    } else {
        list.push(LegacyIdEntry {
            id: id.to_string(),
            count: Some(count),
        });
    }
}

fn lobby_id_entries(list: &LegacyIdList) -> Vec<ScenarioLobbyIdEntry> {
    list.iter()
        .map(|entry| ScenarioLobbyIdEntry {
            id: entry.id.clone(),
            count: entry.count.unwrap_or(0),
        })
        .collect()
}

fn ini_i32(tree: &LegacyIniTree, parent: usize, name: &str, default: i32) -> i32 {
    tree.value(parent, name)
        .and_then(parse_std_i32)
        .unwrap_or(default)
}

fn ini_optional_i32(tree: &LegacyIniTree, parent: usize, name: &str, default: i32) -> Option<i32> {
    tree.has_value(parent, name)
        .then(|| ini_i32(tree, parent, name, default))
}

fn ini_u32(tree: &LegacyIniTree, parent: usize, name: &str, default: u32) -> u32 {
    tree.value(parent, name)
        .and_then(parse_std_i64)
        .map(|value| value as u32)
        .unwrap_or(default)
}

fn ini_bool(tree: &LegacyIniTree, parent: usize, name: &str, default: bool) -> bool {
    tree.value(parent, name)
        .and_then(parse_std_bool)
        .unwrap_or(default)
}

fn ini_optional_bool(
    tree: &LegacyIniTree,
    parent: usize,
    name: &str,
    default: bool,
) -> Option<bool> {
    tree.has_value(parent, name)
        .then(|| ini_bool(tree, parent, name, default))
}

fn ini_rct_all(tree: &LegacyIniTree, parent: usize, name: &str, default: &str) -> String {
    tree.value(parent, name)
        .map(parse_rct_all)
        .unwrap_or_else(|| default.to_string())
}

fn ini_std_string(tree: &LegacyIniTree, parent: usize, name: &str, default: &str) -> String {
    tree.value(parent, name)
        .map(parse_std_string)
        .unwrap_or_else(|| default.to_string())
}

fn ini_validated_client_name(tree: &LegacyIniTree, parent: usize, name: &str) -> String {
    let Some(raw) = tree.value(parent, name) else {
        // A missing naming takes DefaultAdapt's empty default without running
        // ValidatedStdStrBuf::CompileFunc, so validation is deliberately not
        // applied here.
        return String::new();
    };
    let value = parse_std_string(raw).replace('{', "");
    let value = value.trim().to_string();
    if value.is_empty() {
        "Unknown".to_string()
    } else {
        truncate_legacy_string(value, 30)
    }
}

fn ini_i32_array(
    tree: &LegacyIniTree,
    parent: usize,
    name: &str,
    count: i32,
    default: i32,
) -> Vec<i32> {
    let Ok(count) = usize::try_from(count) else {
        return Vec::new();
    };
    let mut values = vec![default; count];
    if let Some(raw) = tree.value(parent, name) {
        let component_defaults = vec![default; count];
        compile_defaulted_i32_components(raw, &mut values, &component_defaults, true);
    }
    values
}

fn ini_optional_id_list(
    tree: &LegacyIniTree,
    parent: usize,
    name: &str,
    default: &[ScenarioLobbyIdEntry],
) -> Option<Vec<ScenarioLobbyIdEntry>> {
    let raw = tree.value(parent, name)?;
    Some(
        parse_legacy_id_list(name, parse_rct_all(raw).as_str())
            .map(|list| lobby_id_entries(&list))
            .unwrap_or_else(|_| default.to_vec()),
    )
}

fn ini_team_distribution(tree: &LegacyIniTree, parent: usize) -> ScenarioTeamDistribution {
    let Some(raw) = tree.value(parent, "TeamDistribution") else {
        return ScenarioTeamDistribution::Free;
    };
    if let Some(value) = parse_std_i64(raw) {
        let value = if value < 0 {
            u8::MAX
        } else {
            value.min(u8::MAX as i64) as u8
        };
        return match value {
            0 => ScenarioTeamDistribution::Free,
            1 => ScenarioTeamDistribution::Host,
            2 => ScenarioTeamDistribution::None,
            3 => ScenarioTeamDistribution::Random,
            4 => ScenarioTeamDistribution::RandomInvisible,
            value => ScenarioTeamDistribution::Numeric(value),
        };
    }
    match parse_identifier(raw) {
        Some("Free") => ScenarioTeamDistribution::Free,
        Some("Host") => ScenarioTeamDistribution::Host,
        Some("None") => ScenarioTeamDistribution::None,
        Some("Random") => ScenarioTeamDistribution::Random,
        Some("RandomInv") => ScenarioTeamDistribution::RandomInvisible,
        Some(name) => {
            tracing::warn!(name, "unknown legacy TeamDistribution; using Free");
            ScenarioTeamDistribution::Free
        }
        None => ScenarioTeamDistribution::Free,
    }
}

fn parse_std_i32(raw: &str) -> Option<i32> {
    parse_std_i64(raw).and_then(|value| i32::try_from(value).ok())
}

fn parse_std_u32(raw: &str) -> Option<u32> {
    let raw = raw.trim_start_matches([' ', '\t']);
    let bytes = raw.as_bytes();
    let mut cursor = 0;

    // StdCompilerINIRead selects hexadecimal only when the number itself
    // begins with 0x. A leading sign therefore keeps strtoul in base 10.
    let radix = if bytes.get(cursor) == Some(&b'0')
        && bytes
            .get(cursor + 1)
            .is_some_and(|byte| matches!(byte, b'x' | b'X'))
    {
        cursor += 2;
        16u32
    } else {
        10u32
    };
    let negative = if radix == 10 {
        match bytes.get(cursor) {
            Some(b'-') => {
                cursor += 1;
                true
            }
            Some(b'+') => {
                cursor += 1;
                false
            }
            _ => false,
        }
    } else {
        false
    };
    let digits_start = cursor;
    let mut magnitude = 0u128;
    while let Some(digit) = bytes.get(cursor).and_then(|byte| match byte {
        b'0'..=b'9' => Some(u32::from(*byte - b'0')),
        b'a'..=b'f' if radix == 16 => Some(u32::from(*byte - b'a') + 10),
        b'A'..=b'F' if radix == 16 => Some(u32::from(*byte - b'A') + 10),
        _ => None,
    }) {
        if digit >= radix {
            break;
        }
        magnitude = magnitude
            .saturating_mul(u128::from(radix))
            .saturating_add(u128::from(digit));
        cursor += 1;
    }
    if cursor == digits_start {
        // strtoul("0x", ..., 16) still consumes the leading zero.
        return (radix == 16).then_some(0);
    }

    let c_ulong_bits = std::mem::size_of::<std::os::raw::c_ulong>() * 8;
    let c_ulong_max = (1u128 << c_ulong_bits) - 1;
    let unsigned = if magnitude > c_ulong_max {
        c_ulong_max
    } else if negative {
        0u128.wrapping_sub(magnitude) & c_ulong_max
    } else {
        magnitude
    };
    Some(unsigned as u32)
}

fn compile_defaulted_i32_components(
    raw: &str,
    values: &mut [i32],
    defaults: &[i32],
    fill_rest_on_separator_failure: bool,
) {
    debug_assert_eq!(values.len(), defaults.len());
    let mut position = 0;
    for index in 0..values.len() {
        if index != 0 && !consume_std_separator(raw, &mut position, b',') {
            if fill_rest_on_separator_failure {
                values[index..].copy_from_slice(&defaults[index..]);
            }
            break;
        }
        values[index] = parse_std_i32_prefix_at(raw, &mut position).unwrap_or(defaults[index]);
    }
}

fn skip_std_whitespace(raw: &str, position: &mut usize) {
    while raw
        .as_bytes()
        .get(*position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        *position += 1;
    }
}

fn consume_std_separator(raw: &str, position: &mut usize, separator: u8) -> bool {
    skip_std_whitespace(raw, position);
    if raw.as_bytes().get(*position) != Some(&separator) {
        return false;
    }
    *position += 1;
    true
}

fn parse_std_i32_prefix_at(raw: &str, position: &mut usize) -> Option<i32> {
    skip_std_whitespace(raw, position);
    let start = *position;
    let bytes = raw.as_bytes();
    let signed = matches!(bytes.get(start), Some(b'+' | b'-'));
    let sign_length = usize::from(signed);
    let unsigned_start = start + sign_length;
    let hexadecimal = !signed
        && bytes.get(unsigned_start) == Some(&b'0')
        && matches!(bytes.get(unsigned_start + 1), Some(b'x' | b'X'));
    let digit_start = unsigned_start + usize::from(hexadecimal) * 2;
    if digit_start > bytes.len() {
        return None;
    }
    let digit_length = bytes[digit_start..]
        .iter()
        .take_while(|byte| {
            if hexadecimal {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
        .count();
    if digit_length == 0 {
        return None;
    }
    let end = digit_start + digit_length;
    *position = end;
    let digits = std::str::from_utf8(&bytes[digit_start..end]).ok()?;
    let magnitude = i64::from_str_radix(digits, if hexadecimal { 16 } else { 10 }).ok()?;
    let signed_value = if bytes.get(start) == Some(&b'-') {
        magnitude.checked_neg()?
    } else {
        magnitude
    };
    i32::try_from(signed_value).ok()
}

fn parse_std_i64(raw: &str) -> Option<i64> {
    let raw = raw.trim_start_matches([' ', '\t']);
    let (sign, rest, had_sign) = if let Some(rest) = raw.strip_prefix('-') {
        (-1_i64, rest, true)
    } else if let Some(rest) = raw.strip_prefix('+') {
        (1_i64, rest, true)
    } else {
        (1_i64, raw, false)
    };
    let (radix, digits) = if !had_sign {
        rest.strip_prefix("0x")
            .or_else(|| rest.strip_prefix("0X"))
            .map_or((10, rest), |digits| (16, digits))
    } else {
        (10, rest)
    };
    let length = digits
        .bytes()
        .take_while(|byte| match radix {
            16 => byte.is_ascii_hexdigit(),
            _ => byte.is_ascii_digit(),
        })
        .count();
    if length == 0 {
        return None;
    }
    i64::from_str_radix(&digits[..length], radix)
        .ok()
        .and_then(|value| value.checked_mul(sign))
}

fn parse_std_bool(raw: &str) -> Option<bool> {
    if raw.starts_with('1') && !raw.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        Some(true)
    } else if raw.starts_with('0') && !raw.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        Some(false)
    } else if raw.starts_with("true") {
        Some(true)
    } else if raw.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn parse_team_i32(key: &str, value: &str, line: usize) -> Result<i32, ScenarioError> {
    parse_i32(value)
        .map_err(|error| team_parse_error(line, format!("invalid {key} `{value}`: {error}")))
}

fn parse_team_distribution(value: &str) -> (Option<InitialNetworkTeamDistribution>, Option<u8>) {
    let value = value.trim_start_matches([' ', '\t']);
    let identifier_end = value
        .bytes()
        .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        .unwrap_or(value.len());
    let named = match &value[..identifier_end] {
        "Free" => Some(InitialNetworkTeamDistribution::Free),
        "Host" => Some(InitialNetworkTeamDistribution::Host),
        "None" => Some(InitialNetworkTeamDistribution::None),
        "Random" => Some(InitialNetworkTeamDistribution::Random),
        "RandomInv" => Some(InitialNetworkTeamDistribution::RandomInvisible),
        _ => None,
    };
    if named.is_some() {
        return (named, None);
    }

    let starts_numeric = value
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_digit() || matches!(*byte, b'+' | b'-'));
    if !starts_numeric {
        return (None, None);
    }
    let parsed = parse_i64(value).unwrap_or(0);
    let numeric = if parsed < 0 {
        u8::MAX
    } else {
        parsed.min(i64::from(u8::MAX)) as u8
    };
    let known = match numeric {
        0 => Some(InitialNetworkTeamDistribution::Free),
        1 => Some(InitialNetworkTeamDistribution::Host),
        2 => Some(InitialNetworkTeamDistribution::None),
        3 => Some(InitialNetworkTeamDistribution::Random),
        4 => Some(InitialNetworkTeamDistribution::RandomInvisible),
        _ => None,
    };
    if known.is_some() {
        (known, None)
    } else {
        (None, Some(numeric))
    }
}

fn parse_team_escaped_bytes(
    value: &str,
    line: usize,
    field: &str,
) -> Result<Vec<u8>, ScenarioError> {
    // StdStrBuf's escaped reader falls back to RCT_All when the first byte is
    // not a quote. RCT_All skips leading space/tab but retains the tail
    // verbatim (StdCompiler.cpp:734-743,897-998).
    if !value.starts_with('"') {
        return latin1_string_as_bytes(value.trim_start_matches([' ', '\t']), line, field);
    }
    parse_legacy_object_name(value, line)
        .map(|value| value.unwrap_or_default())
        .map_err(|error| team_parse_error(line, format!("invalid {field}: {error}")))
        .and_then(|value| latin1_string_as_bytes(&value, line, field))
}

fn team_parse_error(line: usize, detail: String) -> ScenarioError {
    ScenarioError::LegacyParse(format!("Teams.txt line {line}: {detail}"))
}

fn truncate_team_name(mut name: Vec<u8>) -> Vec<u8> {
    const C4_MAX_NAME: usize = 30;
    if name.len() > C4_MAX_NAME {
        name.truncate(C4_MAX_NAME);
    }
    name
}

fn bytes_as_latin1_string(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

fn latin1_string_as_bytes(value: &str, line: usize, field: &str) -> Result<Vec<u8>, ScenarioError> {
    value
        .chars()
        .map(|character| {
            u8::try_from(u32::from(character)).map_err(|_| {
                team_parse_error(
                    line,
                    format!(
                        "{field} contains a non-byte character U+{:04X}",
                        u32::from(character)
                    ),
                )
            })
        })
        .collect()
}

fn team_legacy_cstring(bytes: Vec<u8>, field: &str) -> Result<LegacyCString, ScenarioError> {
    LegacyCString::from_bytes(bytes).ok_or_else(|| {
        ScenarioError::LegacyParse(format!("Teams.txt {field} contains an interior NUL"))
    })
}

fn parse_identifier(raw: &str) -> Option<&str> {
    let raw = raw.trim_start_matches([' ', '\t']);
    let length = raw
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        .count();
    (length > 0).then(|| &raw[..length])
}

fn parse_rct_all(raw: &str) -> String {
    raw.trim_start_matches([' ', '\t']).to_string()
}

fn truncate_legacy_string(value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        value
    } else {
        String::from_utf8_lossy(&value.as_bytes()[..max_bytes]).into_owned()
    }
}

fn truncate_legacy_c4_string(value: String, max_bytes: usize) -> String {
    let bytes = clonk_script::c4_string_bytes(&value);
    let visible_len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())
        .min(max_bytes);
    clonk_script::c4_string_from_bytes(&bytes[..visible_len])
}

fn parse_std_string(raw: &str) -> String {
    match raw.strip_prefix('"') {
        Some(escaped) => decode_legacy_escaped_string(escaped),
        None => parse_rct_all(raw),
    }
}

fn decode_legacy_escaped_string(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'"' {
            break;
        }
        if byte != b'\\' {
            output.push(byte);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&escaped) = bytes.get(index) else {
            break;
        };
        index += 1;
        match escaped {
            b'a' => output.push(b'\x07'),
            b'b' => output.push(b'\x08'),
            b'f' => output.push(b'\x0c'),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(b'\x0b'),
            b'\'' => output.push(b'\''),
            b'"' => output.push(b'"'),
            b'\\' => output.push(b'\\'),
            b'?' => output.push(b'?'),
            b'x' => {
                let start = index;
                while bytes.get(index).is_some_and(u8::is_ascii_hexdigit) {
                    index += 1;
                }
                if index == start {
                    output.push(b'x');
                } else if let Ok(hex) = std::str::from_utf8(&bytes[start..index]) {
                    output.push(u32::from_str_radix(hex, 16).unwrap_or(0) as u8);
                }
            }
            b'0'..=b'7' => {
                let mut value = u32::from(escaped - b'0');
                while let Some(next @ b'0'..=b'7') = bytes.get(index).copied() {
                    value = value * 8 + u32::from(next - b'0');
                    index += 1;
                }
                output.push(value as u8);
            }
            other => output.push(other),
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// Collects and localizes the scripts of a System.c4g group in the group's
/// existing entry order, matching C4Group::FindNextEntry and the shared
/// C4LangStringTable passed to every host (C4Game.cpp:2777-2791,3346-3355).
pub fn load_system_scripts(group: &Group) -> Result<Vec<(String, String)>, ScenarioError> {
    load_system_scripts_with_components(group, &ComponentGroups::local(group), &["US", "DE"])
}

/// Pack-aware System.c4g script loader. `components` must represent the
/// System group itself; script files remain local while its StringTbl is
/// selected through C4ComponentHost::LoadEx's local-plus-pack group set.
pub fn load_system_scripts_with_components<S: AsRef<str>>(
    group: &Group,
    components: &ComponentGroups,
    languages: &[S],
) -> Result<Vec<(String, String)>, ScenarioError> {
    let mut sources = Vec::new();
    for entry in group.entries()? {
        if !legacy_group_wildcard_match(b"*.c", &entry.name_bytes) {
            continue;
        }
        let name = clonk_script::c4_string_from_bytes(&entry.name_bytes);
        let bytes = match group.read_entry_bytes_exact(&entry) {
            Ok(bytes) => bytes,
            Err(_) => {
                // C4Game registers the host before C4ScriptHost::Load and
                // ignores a load failure, so later matching entries remain.
                sources.push((name, String::new()));
                continue;
            }
        };
        let source = clonk_script::c4_string_from_bytes(&bytes);
        let source = localize_script_source_with_components(components, &source, languages)?;
        sources.push((name, source));
    }
    Ok(sources)
}

/// The scenario's own System.c4g scripts, empty when the group has none
/// (C4Game::LoadScenarioScripts opens C4CFN_System as a child and loads
/// every C4CFN_ScriptFiles entry, C4Game.cpp:3317-3343).
fn load_scenario_system_scripts<S: AsRef<str>>(
    group: &Group,
    language_packs: &LanguagePacks,
    scenario_origin: Option<&str>,
    languages: &[S],
) -> Result<Vec<(String, String)>, ScenarioError> {
    group
        .open_child(Path::new("System.c4g"))
        .ok()
        .map(|system| {
            let components = language_packs.component_groups(&system, Some(group), scenario_origin);
            load_system_scripts_with_components(&system, &components, languages)
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

/// Evaluate `[Landscape] MapZoom` with the C4S default
/// `C4SVal(10, 0, 5, 15)` (C4Scenario.cpp:307,353) against the local
/// FixRandom map-creation ledger.
fn legacy_map_zoom(section: Option<&Vec<(String, String)>>, rng: &mut crate::rng::LcgRng) -> u32 {
    legacy_map_zoom_value(section).evaluate(rng) as u32
}

fn legacy_map_zoom_value(section: Option<&Vec<(String, String)>>) -> LegacyC4SVal {
    let default = LegacyC4SVal::new(10, 0, 5, 15);
    section
        .and_then(|entries| find_entry_including_empty(entries, "mapzoom"))
        .and_then(|raw| parse_legacy_c4s_value("MapZoom", raw, default).ok())
        .unwrap_or(default)
}

fn legacy_random_seed(fallback: u64) -> u64 {
    std::env::var("LC_RUST_ENGINE_RANDOM_SEED")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn map_seed_from_random_seed(random_seed: u64) -> i32 {
    let mut rng = crate::rng::LcgRng::seed_from_u64(random_seed);
    rng.random(3_133_700)
}

/// The ChunkOZoom jitter seed: after C4Game::FixRandom(RandomSeed) fills
/// FRndBuf3 with 500 draws, C++ draws `MapSeed = Random(3133700)` before
/// re-fixing for map creation (C4Game.cpp:2651; C4Landscape.cpp:563-579).
/// The shadow bridge can hand the already-drawn C++ value across directly.
fn legacy_map_seed(random_seed: u64) -> i32 {
    let map_seed = std::env::var("LC_RUST_ENGINE_MAP_SEED")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or_else(|| map_seed_from_random_seed(legacy_random_seed(random_seed)));
    if std::env::var("LC_DEBUG_MAP").is_ok() {
        eprintln!("RUST MAPSEED {map_seed}");
    }
    map_seed
}

/// `Game.FixRandom(Game.Parameters.RandomSeed)` before map creation
/// (C4Landscape.cpp:578): the map creators draw from a freshly fixed
/// ledger, and the bracket re-fixes afterwards (C4Landscape.cpp:734), so
/// map creation never shifts the post-init synced ledger. The caller supplies
/// the established `Parameters.RandomSeed`; the env shadow remains a test
/// bridge for comparison with the C++ engine.
fn legacy_map_creation_rng(random_seed: u64) -> crate::rng::LcgRng {
    crate::rng::LcgRng::seed_from_u64(legacy_random_seed(random_seed))
}

/// C4Game::InitGame overwrites the serialized parameter from the initial
/// player-info list only at frame zero (C4Game.cpp:2455-2456). Runtime
/// records/savegames retain the value captured by C4GameParameters instead.
fn replay_startup_player_count_from_group(
    group: &Group,
    serialized_startup_player_count: i32,
) -> Result<i32, ScenarioError> {
    let frame = try_read_group_file_case_insensitive(group, "Game.txt")?
        .map(|source| crate::parse_initial_network_game_data(&source).frame)
        .unwrap_or_default();
    if frame != 0 {
        return Ok(serialized_startup_player_count);
    }

    let Some(source) = try_read_group_file_case_insensitive(group, "PlayerInfos.txt")? else {
        return Ok(0);
    };
    let tree = LegacyIniTree::parse(&bytes_as_latin1_string(&source));
    let Some(root) = tree.first_section(0, "PlayerInfoList") else {
        return Ok(0);
    };
    Ok(tree
        .sections(root, "Client")
        .flat_map(|client| tree.sections(client, "Player"))
        .filter(|player| {
            !tree.value(*player, "Flags").is_some_and(|flags| {
                flags
                    .split(['|', ','])
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .any(|token| {
                        token.eq_ignore_ascii_case("Removed")
                            || parse_std_u32(token).is_some_and(|value| {
                                value & u32::from(crate::PLAYER_INFO_FLAG_REMOVED) != 0
                            })
                    })
            })
        })
        .fold(0_i32, |count, _| count.saturating_add(1)))
}

/// `Game.Parameters.StartupPlayerCount` (MapPlayerExtend input,
/// C4Landscape.cpp:518): the headless harness joins one player.
fn legacy_startup_player_count() -> i32 {
    std::env::var("LC_RUST_ENGINE_STARTUP_PLAYERS")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(1)
}

/// The C4MapCreator inputs from the parsed `[Landscape]` section.
fn basic_map_params(landscape: &LegacyLandscape) -> crate::map_creator::BasicMapParams {
    crate::map_creator::BasicMapParams {
        map_width: landscape.map_width,
        map_height: landscape.map_height,
        map_player_extend: landscape.map_player_extend,
        amplitude: landscape.amplitude,
        phase: landscape.phase,
        period: landscape.period,
        random: landscape.random,
        material: landscape.material.clone(),
        liquid: landscape.liquid.clone(),
        liquid_level: landscape.liquid_level,
        layers: landscape
            .layers
            .iter()
            .map(|entry| (entry.name.clone(), entry.count.unwrap_or(0)))
            .collect(),
    }
}

/// Map-pixel material classification (the Pix2Mat/Pix2Dens tables,
/// C4Wrappers.h:110-145, C4Landscape.cpp:2832-2839): a pixel byte's low 7
/// bits are the texmap index (bit 0x80 = IFT); index 0, unmapped entries
/// and unknown materials are sky (MNone, density 0).
#[derive(Clone)]
pub(crate) struct MapPixelClassifier {
    state: RuntimeTexMapState,
    material_library: Option<clonk_resources::MaterialLibrary>,
    texmap_lookups: Vec<RuntimeTexMapLookup>,
}

impl MapPixelClassifier {
    pub(crate) fn from_runtime_state(state: RuntimeTexMapState) -> Self {
        Self {
            state,
            material_library: None,
            texmap_lookups: Vec::new(),
        }
    }

    /// Empty texture-map stand-in used only to reproduce map-creator RNG
    /// consumption when legacy resource activation supplied no classifier.
    fn empty_for_map_creation() -> Self {
        Self {
            state: RuntimeTexMapState {
                densities: vec![0; 128],
                material_names: vec![None; 128],
                texture_names: vec![None; 128],
                match_texture_names: vec![None; 128],
                shapes: vec![None; 128],
                materials: Vec::new(),
                texture_inventory: Vec::new(),
                default_material_entries: Vec::new(),
                material_crossmap_entries: Vec::new(),
                ..Default::default()
            },
            material_library: None,
            texmap_lookups: Vec::new(),
        }
    }

    pub(crate) fn into_runtime_state(self) -> RuntimeTexMapState {
        self.state
    }

    /// Bare-slot constructor for unit tests (no material groups behind
    /// the slots — `get_index` adds fail like a full C++ texture map).
    #[cfg(test)]
    pub(crate) fn from_slots(
        densities: [i32; 128],
        names: Vec<Option<String>>,
        textures: Vec<Option<String>>,
        shapes: Vec<Option<crate::chunky::ChunkShape>>,
    ) -> Self {
        Self {
            state: RuntimeTexMapState {
                densities: densities.to_vec(),
                material_names: names,
                match_texture_names: textures.clone(),
                texture_names: textures,
                shapes,
                materials: Vec::new(),
                texture_inventory: Vec::new(),
                default_material_entries: Vec::new(),
                material_crossmap_entries: Vec::new(),
                ..Default::default()
            },
            material_library: None,
            texmap_lookups: Vec::new(),
        }
    }

    /// Test constructor with a material library and texture inventory so
    /// `mat=`/`tex=` validation and `GetIndex` adds behave like a real
    /// scenario load.
    #[cfg(test)]
    pub(crate) fn from_slots_with_library(
        densities: [i32; 128],
        names: Vec<Option<String>>,
        textures: Vec<Option<String>>,
        shapes: Vec<Option<crate::chunky::ChunkShape>>,
        library: clonk_resources::MaterialLibrary,
        texture_inventory: Vec<String>,
    ) -> Self {
        let materials = library
            .iter()
            .map(Self::runtime_material)
            .collect::<Vec<_>>();
        Self {
            state: RuntimeTexMapState {
                densities: densities.to_vec(),
                material_names: names,
                match_texture_names: textures.clone(),
                texture_names: textures,
                shapes,
                materials,
                texture_inventory,
                default_material_entries: Vec::new(),
                material_crossmap_entries: Vec::new(),
                ..Default::default()
            },
            material_library: Some(library),
            texmap_lookups: Vec::new(),
        }
    }

    fn material_library(&self) -> Option<&clonk_resources::MaterialLibrary> {
        self.material_library.as_ref()
    }

    fn runtime_material(material: &clonk_resources::MaterialDefinition) -> RuntimeTexMapMaterial {
        RuntimeTexMapMaterial {
            name: material.name().to_string(),
            density: material.int("Density").unwrap_or(0),
            shape: crate::chunky::ChunkShape::from_shape(material.int("Shape").unwrap_or(0)),
        }
    }

    /// DensitySolid: density >= C4M_Solid=50 (C4Wrappers.h:68-71).
    fn is_solid(&self, pixel: u8) -> bool {
        self.density(pixel) >= 50
    }

    /// DensityLiquid: C4M_Liquid=25 <= density < 50 (C4Wrappers.h:78-81).
    fn is_liquid(&self, pixel: u8) -> bool {
        (25..50).contains(&self.density(pixel))
    }

    fn density(&self, pixel: u8) -> i32 {
        self.state.densities[(pixel & 0x7F) as usize]
    }

    /// C4TextureMap::CheckTexture (the map creators validate `tex=`
    /// fields against the loaded texture inventory).
    pub(crate) fn texture_exists(&self, name: &str) -> bool {
        self.state.texture_exists(name)
    }

    /// The material definition behind a name, scenario-local first
    /// (C4MaterialMap::Get order after the prepending loads).
    pub(crate) fn material(&self, name: &str) -> Option<&RuntimeTexMapMaterial> {
        self.state.material(name)
    }

    /// C4TextureMap::GetIndex (C4Texture.cpp:319-345): the existing
    /// (material, texture) slot — material match, texture match when
    /// given — else the first free slot when `add_if_not_exist`. 0 = fail.
    pub(crate) fn get_index(
        &mut self,
        mat_name: &str,
        tex_name: Option<&str>,
        add_if_not_exist: bool,
    ) -> u8 {
        self.state.get_index(mat_name, tex_name, add_if_not_exist)
    }

    /// C4TextureMap::GetIndexMatTex (C4Texture.cpp:346-367): split the
    /// `Material-Texture` pair, try the exact pair, then the default
    /// texture; final fallback is the material's default entry
    /// (DefaultMatTex — the first slot carrying the material).
    pub(crate) fn get_index_mat_tex(
        &mut self,
        material_texture: &str,
        default_texture: Option<&str>,
    ) -> u8 {
        let eager_index = self
            .state
            .get_index_mat_tex(material_texture, default_texture);
        self.texmap_lookups.push(RuntimeTexMapLookup {
            material_texture: material_texture.to_owned(),
            default_texture: default_texture.map(str::to_owned),
            eager_index,
        });
        eager_index
    }

    fn clear_texmap_lookups(&mut self) {
        self.texmap_lookups.clear();
    }

    fn texmap_lookups(&self) -> &[RuntimeTexMapLookup] {
        &self.texmap_lookups
    }
}

/// TexMap.txt + material densities from the ordered NRT_Material chain.
/// C4Game::InitMaterialTexture loads the scenario group first, then each
/// external source admitted by the preceding TexMap's independent
/// OverloadMaterials/OverloadTextures flags (C4Game.cpp:901-977).
/// `None` only when there is no material source at all. A first source
/// without TexMap.txt still builds from an empty table before
/// CrossMapMaterials allocates its dynamic entries.
pub(crate) fn build_map_pixel_classifier(
    group: &Group,
    resolver: &dyn LegacyDefinitionResolver,
) -> Result<Option<MapPixelClassifier>, ScenarioError> {
    // Parse the root savegame ledger before any no-material/no-texmap return:
    // C++ still calls LoadEnumeration after an empty material loop, and a
    // listed name must then fail against Num=0.
    let enumeration = match try_read_group_file_case_insensitive(group, "MatMap.txt")? {
        Some(source) if !source.is_empty() => Some(
            clonk_resources::material::MaterialEnumeration::parse(&source)?,
        ),
        Some(_) | None => None,
    };
    let mut material_groups = Vec::new();
    let mut scenario_material_root = None;
    match group.open_child("Material.c4g") {
        Ok(local) => {
            scenario_material_root = Some(local.root().to_path_buf());
            material_groups.push(local);
        }
        Err(
            GroupError::EntryNotFound(_) | GroupError::Missing(_) | GroupError::NotDirectory(_),
        ) => {}
        Err(error) => return Err(ScenarioError::Resources(error)),
    }
    for candidate in resolver.resolve_material_groups(group)? {
        if scenario_material_root.as_deref() != Some(candidate.root()) {
            material_groups.push(candidate);
        }
    }

    let Some(first_group) = material_groups.first() else {
        if let Some(name) = enumeration
            .as_ref()
            .and_then(|enumeration| enumeration.names().first())
        {
            return Err(
                clonk_resources::material::MaterialEnumerationError::MissingMaterial(name.clone())
                    .into(),
            );
        }
        return Ok(None);
    };
    let texmap = first_group
        .read_file("TexMap.txt")
        .ok()
        .map(|source| clonk_resources::texmap::TextureMap::parse_bytes(&source));

    let mut material_libraries: Vec<clonk_resources::MaterialLibrary> = Vec::new();
    let mut texture_inventory: Vec<String> = Vec::new();
    let mut load_materials = true;
    let mut load_textures = true;
    for (index, material_group) in material_groups.iter().enumerate() {
        if !load_materials && !load_textures {
            break;
        }

        // The first source supplies the actual table. Later sources only
        // expose continuation flags through LoadFlags; a missing TexMap at
        // that point stops both chains before that group's contents load
        // (C4Game.cpp:940-976).
        let later_texmap = if index == 0 {
            None
        } else {
            let Ok(source) = material_group.read_file("TexMap.txt") else {
                break;
            };
            Some(clonk_resources::texmap::TextureMap::parse_flags_bytes(
                &source,
            ))
        };
        let flags = later_texmap.as_ref().or(texmap.as_ref());
        let mut next_materials = flags.is_some_and(|flags| flags.overload_materials);
        let mut next_textures = flags.is_some_and(|flags| flags.overload_textures);

        if load_materials {
            match clonk_resources::MaterialLibrary::from_group(material_group) {
                Ok(library) => {
                    // C4MaterialMap::Load counts only names not provided by an
                    // earlier source. A zero-count load automatically admits
                    // the next source even without OverloadMaterials.
                    let loaded_count = library
                        .iter()
                        .filter(|definition| {
                            material_libraries
                                .iter()
                                .all(|loaded| loaded.get(definition.name()).is_none())
                        })
                        .count();
                    if loaded_count == 0 {
                        next_materials = true;
                    }
                    material_libraries.push(library);
                }
                Err(_) => next_materials = true,
            }
        }

        if load_textures {
            // C4TextureMap::LoadTextures likewise counts only newly admitted
            // image basenames; a zero-count load keeps the texture chain open
            // (C4Texture.cpp:266-310; C4Game.cpp:956-962).
            let mut loaded_count = 0;
            let entries = material_group.entries().unwrap_or_default();
            // LoadTextures scans the whole group twice: PNG first, then BMP.
            for extension in [b".png".as_slice(), b".bmp".as_slice()] {
                for entry in &entries {
                    if entry.is_directory
                        || entry.name_bytes.len() < extension.len()
                        || !entry.name_bytes[entry.name_bytes.len() - extension.len()..]
                            .eq_ignore_ascii_case(extension)
                    {
                        continue;
                    }
                    // SReplaceChar(texname, '.', 0) exposes the bytes before
                    // the first dot, rather than Path::file_stem's suffix.
                    let stem_end = entry
                        .name_bytes
                        .iter()
                        .position(|byte| *byte == b'.')
                        .unwrap_or(entry.name_bytes.len());
                    let full_stem =
                        clonk_script::c4_string_from_bytes(&entry.name_bytes[..stem_end]);
                    // Duplicate detection precedes the fixed-name copy. A
                    // long candidate therefore never equals a stored 15-byte
                    // prefix and every long prefix collision is admitted.
                    if texture_inventory
                        .iter()
                        .any(|stored| clonk_resources::material::c4_names_equal(stored, &full_stem))
                    {
                        continue;
                    }
                    // GroupReadSurfacePNG returns an allocated surface even
                    // if its decoder reports failure. Bitmap admission does
                    // require a successfully decoded Surface8.
                    if extension.eq_ignore_ascii_case(b".bmp")
                        && material_group
                            .read_entry_bytes_exact(entry)
                            .ok()
                            .and_then(|bytes| {
                                clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok()
                            })
                            .is_none()
                    {
                        continue;
                    }
                    texture_inventory
                        .push(clonk_resources::material::truncate_c4m_name(&full_stem));
                    loaded_count += 1;
                }
            }
            if loaded_count == 0 {
                next_textures = true;
            }
        }

        load_materials = next_materials;
        load_textures = next_textures;
    }

    // Each C4MaterialMap::Load prepends its fresh names, so later/global
    // uniques precede earlier/local definitions while earlier sources win
    // collisions (C4Material.cpp:263-299).
    let material_loads: Vec<_> = material_libraries.iter().collect();
    let mut material_library =
        clonk_resources::MaterialLibrary::from_overloaded_loads(&material_loads).ok();

    // Savegames retain the numeric material order in root MatMap.txt. C++
    // applies this pairwise-swap ledger after every material source has
    // loaded but before TextureMap.Init and CrossMapMaterials
    // (C4Game.cpp:979-993; C4Material.cpp:510-558).
    if let Some(enumeration) = enumeration
        .as_ref()
        .filter(|enumeration| !enumeration.is_empty())
    {
        let library = material_library.as_mut().ok_or_else(|| {
            clonk_resources::material::MaterialEnumerationError::MissingMaterial(
                enumeration.names()[0].clone(),
            )
        })?;
        library.sort_enumeration(enumeration)?;
    }

    // LoadMap returns zero for a missing first TexMap, but C++ retains the
    // empty C4TextureMap and still runs Init + CrossMapMaterials. Parsing an
    // empty source gives the normal 128-slot table with both overload flags
    // false, without changing the independent resource-chain decisions above.
    let texmap = texmap.unwrap_or_else(|| clonk_resources::texmap::TextureMap::parse_bytes(b""));
    let overload_materials = texmap.overload_materials;
    let overload_textures = texmap.overload_textures;

    let runtime_materials = material_library
        .iter()
        .flat_map(|library| library.iter())
        .map(MapPixelClassifier::runtime_material)
        .collect();

    let mut densities = [0i32; 128];
    let mut names: Vec<Option<String>> = vec![None; 128];
    let mut grid_textures: Vec<Option<String>> = vec![None; 128];
    let mut shapes: Vec<Option<crate::chunky::ChunkShape>> = vec![None; 128];
    for (index, slot) in densities.iter_mut().enumerate() {
        names[index] = texmap
            .entry(index as u8)
            .map(|entry| entry.material.clone());
        grid_textures[index] = texmap.entry(index as u8).map(|entry| entry.texture.clone());
        let material = texmap.entry(index as u8).and_then(|entry| {
            material_library
                .as_ref()
                .and_then(|library| library.get(&entry.material))
        });
        shapes[index] = material.map(|material| {
            crate::chunky::ChunkShape::from_shape(material.int("Shape").unwrap_or(0))
        });
        *slot = material
            .and_then(|material| material.int("Density"))
            .unwrap_or(0);
        // "Special, hardcoded crap": liquids render <mat>-Smooth with
        // the Liquid texture (C4TexMapEntry::Init, C4Texture.cpp:79-82).
        if (25..50).contains(&*slot)
            && grid_textures[index]
                .as_deref()
                .is_some_and(|texture| clonk_resources::material::c4_names_equal(texture, "Smooth"))
        {
            grid_textures[index] = Some("Liquid".to_string());
        }
    }
    // Raw texmap textures for GetIndex pair matching.
    let mut match_textures: Vec<Option<String>> = vec![None; 128];
    for (index, slot) in match_textures.iter_mut().enumerate() {
        *slot = texmap.entry(index as u8).map(|entry| entry.texture.clone());
    }
    // Collected as owned (name, overlay, cross-specs) rows so the loops below
    // can mutate the classifier slots.
    let ordered: Vec<(String, Option<String>, Vec<String>)> = material_library
        .iter()
        .flat_map(|library| library.iter())
        .map(|material| {
            (
                material.name().to_string(),
                material.value("TextureOverlay").map(str::to_string),
                ["BlastShiftTo", "BelowTempConvertTo", "AboveTempConvertTo"]
                    .iter()
                    .filter_map(|key| material.strings(key).first().cloned())
                    .filter(|spec| !spec.is_empty())
                    .collect(),
            )
        })
        .collect();

    let mut classifier = MapPixelClassifier {
        state: RuntimeTexMapState {
            densities: densities.to_vec(),
            material_names: names,
            texture_names: grid_textures,
            shapes,
            match_texture_names: match_textures,
            materials: runtime_materials,
            texture_inventory,
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            overload_materials,
            overload_textures,
            ..Default::default()
        },
        material_library,
        texmap_lookups: Vec::new(),
    };

    // C4TextureMap::Init initializes every parsed entry only after the final
    // material and texture inventories have loaded. Entries whose material
    // or effective texture cannot be resolved are cleared before
    // CrossMapMaterials, making their slots available to the ascending
    // GetIndex allocation scan (C4Texture.cpp:68-104,229-244). For liquid
    // `Material-Smooth` entries, `texture_names` already carries the
    // hard-coded effective `Liquid` lookup while `match_texture_names`
    // deliberately retains the raw `Smooth` pair.
    let invalid_slots = (1..127usize)
        .filter(|&slot| {
            let Some(material_name) = classifier.state.material_names[slot].as_deref() else {
                return false;
            };
            classifier.state.material(material_name).is_none()
                || classifier.state.texture_names[slot]
                    .as_deref()
                    .is_none_or(|texture_name| !classifier.state.texture_exists(texture_name))
        })
        .map(|slot| slot as u8)
        .collect::<Vec<_>>();
    classifier.state.clear_entries(&invalid_slots);

    // Dynamic texmap entries (C4MaterialMap::CrossMapMaterials,
    // C4Material.cpp:345-484): the DefaultMatTex loop registers
    // (MaterialName, TextureOverlay-or-"Smooth") for EVERY material with
    // fAddIfNotExist — an exact (mat, tex) pair miss fills the FIRST FREE
    // slot (1, 2, 3, …) — then the BlastShiftTo/BelowTempConvertTo/
    // AboveTempConvertTo specs go through GetIndexMatTex the same way.
    // Legacy maps rely on the deterministic slots: GoldRush's road pixels
    // are byte 3 = the third add, Vehicle-Smooth (live-probe-verified
    // slots: [Ice-Sponge, FlyAshes-Spots, Vehicle-Smooth, Ashes-Spots,
    // Ore-Structure, Tunnel-Smooth2, Brick-Brick, Rock2-Rough]).
    // First loop: DefaultMatTex (C4Material.cpp:349-370).
    for (name, overlay, _) in &ordered {
        if name.is_empty() {
            continue;
        }
        let overlay = overlay
            .as_deref()
            .filter(|overlay| classifier.texture_exists(overlay))
            .unwrap_or("Smooth")
            .to_string();
        let default_entry = classifier.get_index(name, Some(&overlay), true);
        classifier
            .state
            .set_default_material_entry(name, default_entry);
    }
    // Second loop: the cross-ref specs (C4Material.cpp:474-484).
    for (_, _, specs) in &ordered {
        for spec in specs {
            let entry = classifier.get_index_mat_tex(spec, None);
            classifier.state.material_crossmap_entries.push(entry);
        }
    }

    if std::env::var("LC_DEBUG_MAP").is_ok() {
        for slot in 1..9usize {
            eprintln!(
                "RUSTTEX {slot} = {:?} density={}",
                classifier.state.material_names[slot], classifier.state.densities[slot]
            );
        }
    }
    Ok(Some(classifier))
}

fn invalid_exact_landscape_pixel(width: u32, slot: usize, byte: u8) -> ScenarioError {
    let width = width.max(1) as usize;
    let x = slot % width;
    let y = slot / width;
    ScenarioError::InvalidLandscape(format!(
        "landscape loading error at ({x}/{y}): pixel value {byte} is not a valid material"
    ))
}

/// Convert the two historical exact-landscape byte formats and enforce the
/// current PixCol2Mat gate. The two native branches are deliberately not one
/// match: a PNG entry suppresses format-0 conversion, but format 1 converts
/// independently and never goes through the live-byte validation afterwards
/// (C4Landscape.cpp:1557-1600).
fn convert_exact_landscape_indices(
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    texmap: &RuntimeTexMapState,
    format: i32,
    png_present: bool,
) -> Result<Vec<u8>, ScenarioError> {
    let mut indices = bitmap.indices.clone();
    let material_count = texmap.materials.len();

    if !png_present && format == 0 {
        for (slot, byte) in indices.iter_mut().enumerate() {
            let source = *byte;
            let old_index = usize::from(source & 63);
            let material = (source >= 128 && old_index < material_count.saturating_mul(3))
                .then_some(old_index / 3)
                .ok_or_else(|| invalid_exact_landscape_pixel(bitmap.width, slot, source))?;
            // Native Mat2PixColDefault(MNone) indexes outside the material
            // array for malformed format-0 input. Reject that undefined case
            // rather than manufacturing a material byte.
            let default = texmap
                .default_material_entry_by_index(material as i32)
                .unwrap_or(0);
            let ift = if source >= 192 { 0x80 } else { 0 };
            *byte = default.wrapping_add(ift);
        }
    }

    if format == 1 {
        let vehicle = texmap
            .materials
            .iter()
            .position(|material| {
                clonk_resources::material::c4_names_equal(&material.name, "Vehicle")
            })
            .map_or(-1, |index| index as i32);
        let material_count = material_count as i32;
        for byte in &mut indices {
            let source = *byte;
            let mut material = i32::from(source & 0x7f) - 1;
            if material > vehicle {
                if material == vehicle + 1 {
                    material = vehicle;
                } else {
                    material -= 2;
                }
            }
            *byte = if (0..material_count).contains(&material) {
                texmap
                    .default_material_entry_by_index(material)
                    .unwrap_or(0)
                    .wrapping_add(source & 0x80)
            } else {
                0
            };
        }
        return Ok(indices);
    }

    for (slot, &byte) in indices.iter().enumerate() {
        if byte == 0 {
            continue;
        }
        let texmap_slot = usize::from(byte & 0x7f);
        let valid = (1..127).contains(&texmap_slot)
            && texmap
                .material_names
                .get(texmap_slot)
                .and_then(Option::as_ref)
                .is_some();
        if !valid {
            return Err(invalid_exact_landscape_pixel(bitmap.width, slot, byte));
        }
    }
    Ok(indices)
}

fn decode_exact_landscape_png(source: &[u8], width: u32, height: u32) -> Result<Vec<u32>, String> {
    let rgba = image::load_from_memory_with_format(source, ImageFormat::Png)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    if rgba.width() < width || rgba.height() < height {
        // Native code performs unchecked source reads for a smaller PNG.
        // Contain that undefined case as the same nonfatal PNG-load failure.
        return Err(format!(
            "Landscape.png is {}x{}, smaller than Landscape.bmp {width}x{height}",
            rgba.width(),
            rgba.height()
        ));
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let [red, green, blue, alpha] = rgba.get_pixel(x, y).0;
            let transparency = 255_u8.wrapping_sub(alpha);
            let color = (u32::from(transparency) << 24)
                | (u32::from(red) << 16)
                | (u32::from(green) << 8)
                | u32::from(blue);
            pixels.push(if transparency == 0xff {
                0xff00_0000
            } else {
                color
            });
        }
    }
    Ok(pixels)
}

/// Install an exact landscape's decoded index plane directly as Surface8.
/// C4Landscape::Load keeps the texture map but no C4Landscape::Map, and does
/// not apply MapZoom/ChunkOZoom (C4Landscape.cpp:658-668,1520-1600).
fn exact_classified_landscape(
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    classifier: &MapPixelClassifier,
    map_seed: i32,
    format: i32,
    landscape_png: Option<&[u8]>,
) -> Result<Landscape, ScenarioError> {
    let surface32_pixels = landscape_png.and_then(|source| {
        match decode_exact_landscape_png(source, bitmap.width, bitmap.height) {
            Ok(pixels) => Some(pixels),
            Err(error) => {
                tracing::error!(
                    error,
                    "could not load 32-bit landscape surface from Landscape.png"
                );
                None
            }
        }
    });
    let indices = convert_exact_landscape_indices(
        bitmap,
        &classifier.state,
        format,
        landscape_png.is_some(),
    )?;
    let world_height = bitmap.height as i32;
    let mut landscape = Landscape::new(bitmap.width, vec![world_height; bitmap.width as usize])
        .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
    landscape.set_world_height(world_height);
    let mut pixels = crate::landscape::PixelGrid::new(
        bitmap.width,
        bitmap.height,
        indices,
        classifier.state.densities.clone(),
        classifier.state.material_names.clone(),
        classifier.state.texture_names.clone(),
    );
    if let Some(surface32_pixels) = surface32_pixels {
        pixels.install_initial_surface32_pixels(surface32_pixels);
    }
    landscape.set_pixel_grid(pixels);
    landscape.refresh_all_raster_columns();
    landscape.set_raster_state(LandscapeRasterState::new(
        0,
        map_seed,
        classifier.state.clone(),
    ));
    Ok(landscape)
}

/// Build the landscape from a classified 8-bit map: the map zooms through
/// ChunkOZoom into the Surface8 pixel plane (chunky material rims and
/// slope smoothers, C4Landscape::MapToSurface → TexOZoom → ChunkOZoom,
/// C4Landscape.cpp:336-480), then the column approximation — surface
/// heights, liquid segments, IFT tunnel ranges — derives from that plane.
pub(crate) fn classified_landscape(
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    classifier: &MapPixelClassifier,
    zoom: i32,
    map_seed: i32,
) -> Result<Landscape, ScenarioError> {
    let map_width = bitmap.width as i32;
    let map_height = bitmap.height as i32;
    let rendered_width = bitmap.width.saturating_mul(zoom as u32);
    let rendered_height = map_height.saturating_mul(zoom).max(0) as u32;
    // C4Landscape::Init clamps the allocated Surface8 independently of the
    // map's zoomed render rectangle (C4Landscape.cpp:638-641). MapToSurface
    // still clips ChunkOZoom to the smaller rectangle, so pad its finished
    // bytes instead of letting inclusive Flat/rough chunk edges bleed into
    // the right/bottom sky margin.
    let final_width = rendered_width.max(100);
    let final_height = rendered_height.max(100);
    let world_height = final_height as i32;
    let plane_width = final_width as usize;

    let synthesized = crate::chunky::synthesize_landscape(
        &bitmap.indices,
        map_width,
        map_height,
        zoom,
        map_seed,
        &classifier.state.shapes,
    )
    .into_bytes();
    let bytes = if (final_width, final_height) == (rendered_width, rendered_height) {
        synthesized
    } else {
        let final_width = final_width as usize;
        let rendered_width = rendered_width as usize;
        let rendered_height = rendered_height as usize;
        let mut padded = vec![0; final_width * final_height as usize];
        for row in 0..rendered_height {
            let source = row * rendered_width;
            let target = row * final_width;
            padded[target..target + rendered_width]
                .copy_from_slice(&synthesized[source..source + rendered_width]);
        }
        padded
    };
    let mut landscape = Landscape::new(final_width, vec![world_height; plane_width])
        .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
    landscape.set_world_height(world_height);
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        final_width,
        final_height,
        bytes,
        classifier.state.densities.clone(),
        classifier.state.material_names.clone(),
        classifier.state.texture_names.clone(),
    ));
    landscape.refresh_all_raster_columns();
    let mut raster_state = LandscapeRasterState::new(zoom, map_seed, classifier.state.clone());
    raster_state.set_map(bitmap);
    landscape.set_raster_state(raster_state);

    // Loaded water is at rest: C4MassMoverSet starts empty and movers are
    // created only by landscape CHANGES via CheckInstability, never at
    // load (C4Game.cpp:1782 MassMover.Default(); the c4b Load only fires
    // for saved games).
    Ok(landscape)
}

fn load_legacy_landscape(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    runtime: Option<&LandscapeGameData>,
    overload_current: bool,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    map_callback_functions: &HashSet<String>,
    post_init_map_callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    prepared_map_creator: &mut Option<crate::map_creator_s2::MapCreatorS2State>,
) -> Result<Option<Landscape>, ScenarioError> {
    *post_init_map_callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
    let Some(mut landscape) = load_legacy_landscape_body(
        group,
        manifest,
        runtime,
        overload_current,
        classifier,
        random_seed,
        startup_player_count,
        map_callback_functions,
        post_init_map_callbacks,
        prepared_map_creator,
    )?
    else {
        return Ok(None);
    };
    landscape.set_shade_materials(manifest.core.landscape.shade_materials);
    // C4Landscape::Init captures pInitial before attempting to load the
    // optional legacy diff. ApplyDiff failure is non-fatal, including a
    // missing or unreadable DiffLandscape.bmp.
    let diff = if let Some(bytes) = try_read_group_file_case_insensitive(group, "DiffLandscape.bmp")
        .ok()
        .flatten()
    {
        clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok()
    } else {
        None
    };
    if landscape.pixel_grid().is_some() {
        landscape
            .save_initial()
            .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
        if let Some(diff) = diff.as_ref() {
            let _ = landscape.apply_diff(diff);
        }
    } else if diff.is_some() {
        return Err(ScenarioError::InvalidLandscape(
            "DiffLandscape.bmp requires a Surface8 pixel grid".to_string(),
        ));
    }
    // C4Landscape::ScenarioInit (C4Landscape.cpp:67-73): the border-open
    // keys, then the AutoScanSideOpen side scan over the built landscape.
    let borders = &manifest.core.landscape;
    landscape.set_no_scan(borders.no_scan);
    landscape.set_border_open(
        borders.left_open,
        borders.right_open,
        borders.top_open,
        borders.bottom_open,
    );
    if borders.auto_scan_side_open {
        landscape.scan_side_open();
    }
    Ok(Some(landscape))
}

fn load_legacy_landscape_body(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    runtime: Option<&LandscapeGameData>,
    overload_current: bool,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    map_callback_functions: &HashSet<String>,
    post_init_map_callbacks: &mut crate::map_creator_s2::PostInitMapCallbacks,
    prepared_map_creator: &mut Option<crate::map_creator_s2::MapCreatorS2State>,
) -> Result<Option<Landscape>, ScenarioError> {
    *prepared_map_creator = None;
    let landscape_section = manifest.sections.get("landscape");
    let map_width_hint = manifest.core.landscape.map_width.std.max(1);
    let map_height_hint = manifest.core.landscape.map_height.std.max(1);
    let exact_landscape = manifest.core.landscape.exact_landscape;
    let map_seed = runtime
        .map(|runtime| runtime.map_seed)
        .filter(|seed| *seed != 0)
        .unwrap_or_else(|| legacy_map_seed(random_seed));
    let precompiled_mode = runtime
        .map(|runtime| runtime.mode)
        .filter(|mode| *mode != 0);
    let set_initial_mode = |landscape: &mut Landscape, inferred| {
        if let Some(mode) = precompiled_mode {
            landscape.set_runtime_mode(mode);
        } else {
            let _ = landscape.set_mode(inferred);
        }
    };
    let mut map_rng = legacy_map_creation_rng(random_seed);

    let read_optional = |name: &str| {
        try_read_group_file_case_insensitive(group, name).map_err(ScenarioError::Resources)
    };

    // ExactLandscape: Landscape.bmp IS the landscape — C++ reads it
    // straight into the pixel surface (GroupReadSurface8), so it decodes
    // at pixel scale (zoom 1) here. Returning no landscape would leave
    // GBackSolid answering "never solid" and hang placement loops in real
    // content (Grass.c4d Initialize).
    let (map_bytes, map_zoom_override, old_landscape_map) = if exact_landscape {
        // C4Landscape::Load requires C4CFN_Landscape. Exact mode never falls
        // back to Map.bmp (C4Landscape.cpp:1520-1524).
        (
            Some(read_group_file_case_insensitive(group, "Landscape.bmp")?),
            Some(1),
            false,
        )
    } else {
        // Static map: Map.bmp, with Landscape.bmp accepted as the map for
        // downwards compatibility (C4Landscape.cpp:593-601) — most CR
        // content (GoldRush included) ships only Landscape.bmp.
        match read_optional("Map.bmp")? {
            Some(bytes) => (Some(bytes), None, false),
            None => {
                let fallback = read_optional("Landscape.bmp")?;
                let map_changed = fallback.is_some();
                (fallback, None, map_changed)
            }
        }
    };
    let exact_landscape_png = if exact_landscape {
        read_optional("Landscape.png")?
    } else {
        None
    };

    let mut classifier = classifier;
    if let Some(bytes) = map_bytes {
        let retained_indexed =
            clonk_resources::bitmap::IndexedBitmap::decode_with_palette(&bytes).ok();
        let retained_indexed_map = retained_indexed.as_ref().map(|(bitmap, _)| bitmap.clone());
        // Material-classified path: the map's 8-bit palette indices are
        // texmap keys (GroupReadSurface8 keeps the index bytes). Without
        // a TexMap or for non-indexed images, the sky-pixel heuristic
        // below stands in.
        if let Some(classifier) = classifier.take() {
            if let Some((bitmap, source_palette)) = retained_indexed.as_ref() {
                let mut landscape = if exact_landscape {
                    exact_classified_landscape(
                        bitmap,
                        classifier,
                        map_seed,
                        manifest.core.landscape.new_style_landscape,
                        exact_landscape_png.as_deref(),
                    )?
                } else {
                    let map_zoom_u32 = legacy_map_zoom(landscape_section, &mut map_rng);
                    classified_landscape(bitmap, classifier, map_zoom_u32 as i32, map_seed)?
                };
                set_initial_mode(
                    &mut landscape,
                    if exact_landscape {
                        LANDSCAPE_MODE_EXACT
                    } else {
                        LANDSCAPE_MODE_STATIC
                    },
                );
                if exact_landscape {
                    landscape
                        .raster_state_mut()
                        .expect("exact classified landscapes carry raster state")
                        .set_surface_palette(*source_palette);
                }
                if old_landscape_map {
                    landscape
                        .raster_state_mut()
                        .expect("classified landscapes carry raster state")
                        .set_map_changed();
                }
                return Ok(Some(landscape));
            }
        }
        let dynamic =
            load_from_memory(&bytes).map_err(|source| ScenarioError::LegacyMapDecode { source })?;
        let rgba = dynamic.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        if width == 0 || height == 0 {
            return Err(ScenarioError::LegacyMapEmpty);
        }

        let map_zoom_u32 =
            map_zoom_override.unwrap_or_else(|| legacy_map_zoom(landscape_section, &mut map_rng));
        let map_zoom_i32 = map_zoom_u32 as i32;
        let sky_pixel = rgba.get_pixel(0, 0).0;
        let rendered_height = (height as i32).saturating_mul(map_zoom_i32).max(0);
        let world_height = if exact_landscape {
            rendered_height
        } else {
            rendered_height.max(100)
        };
        let capacity = (width as usize).saturating_mul(map_zoom_u32 as usize);
        let mut surfaces = Vec::with_capacity(capacity);

        for x in 0..width {
            // The landscape column model stores the SURFACE Y coordinate
            // (solid from `y >= surface`): the first non-sky map row, zoomed.
            // An all-sky column has no solid (surface at the world bottom).
            let surface_world = (0..height)
                .find(|&y| rgba.get_pixel(x, y).0 != sky_pixel)
                .map(|y| (y as i32).saturating_mul(map_zoom_i32))
                .unwrap_or(world_height);

            for _ in 0..map_zoom_u32 {
                surfaces.push(surface_world);
            }
        }

        if surfaces.iter().all(|&surface| surface >= world_height) {
            // No solid anywhere (the whole map read as sky): fall back to a
            // ground level from the hints so the world has a floor.
            let ground_height = manifest
                .ground_height_hint
                .map(|hint| hint.max(0))
                .unwrap_or(0);
            let surface = (world_height - ground_height).max(0);
            surfaces.fill(surface);
        }

        let rendered_width = width.saturating_mul(map_zoom_u32);
        let final_width = if exact_landscape {
            rendered_width
        } else {
            rendered_width.max(100)
        };
        surfaces.resize(final_width as usize, world_height);
        let mut landscape = Landscape::new(final_width, surfaces)
            .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
        // GBackHgt is known exactly here (map height × zoom); placement
        // searches and `Random(GBackHgt - 32)` draws bound on it.
        landscape.set_world_height(world_height);
        set_initial_mode(
            &mut landscape,
            if exact_landscape {
                LANDSCAPE_MODE_EXACT
            } else {
                LANDSCAPE_MODE_STATIC
            },
        );
        let mut raster_state = LandscapeRasterState::new(
            if exact_landscape { 0 } else { map_zoom_i32 },
            map_seed,
            RuntimeTexMapState::default(),
        );
        if exact_landscape {
            if let Some((_, source_palette)) = retained_indexed.as_ref() {
                raster_state.set_surface_palette(*source_palette);
            }
        }
        if !exact_landscape {
            if let Some(bitmap) = retained_indexed_map.as_ref() {
                raster_state.set_map(bitmap);
            }
        }
        if old_landscape_map {
            raster_state.set_map_changed();
        }
        landscape.set_raster_state(raster_state);
        return Ok(Some(landscape));
    }

    if exact_landscape {
        return Ok(None);
    }

    let landscape_script = read_optional("Landscape.txt")?;
    if overload_current && landscape_script.is_none() {
        // Section Init is an overload: unlike initial game creation it does
        // not fall back to C4MapCreator::CreateMap. The current Surface8 is
        // retained, PXS/MassMover state stays live, and LandscapeLoaded stays
        // false.
        return Ok(None);
    }

    // Dynamic map (C4Landscape::Init, C4Landscape.cpp:606-614): a
    // Landscape.txt map description renders through C4MapCreatorS2
    // (CreateMapS2, C4Landscape.cpp:530-546); otherwise the basic
    // C4MapCreator builds the 8-bit map from the [Landscape] keys. Both
    // draw from the FixRandom(RandomSeed) bracket (C4Landscape.cpp:
    // 578,734), so they never shift the post-init synced ledger.
    // Requires a texture map for the material bytes.
    if let Some(classifier) = classifier.take() {
        let players = startup_player_count;
        let landscape_core = &manifest.core.landscape;
        let mut retained_creator = None;
        let bitmap = if let Some(bytes) = landscape_script.as_deref() {
            let creation = crate::map_creator_s2::create_s2_map_with_state_and_functions(
                &String::from_utf8_lossy(bytes),
                classifier,
                landscape_core.map_width,
                landscape_core.map_height,
                landscape_core.map_player_extend,
                players,
                &mut map_rng,
                map_callback_functions,
            );
            *post_init_map_callbacks = creation.callbacks;
            // CreateMapS2 keeps pMapCreator alive through InitializeDef,
            // placements and PostInitMap even when KeepMapCreator is false;
            // PostInitMap performs the conditional destruction afterward.
            retained_creator = Some(creation.creator);
            *prepared_map_creator = retained_creator.clone();
            match creation.bitmap {
                Some(bitmap) => bitmap,
                None if overload_current => return Ok(None),
                None => {
                    // Dynamic map by scenario (C4Landscape.cpp:612-614) is
                    // available only during initial, non-overload creation.
                    let params = basic_map_params(landscape_core);
                    crate::map_creator::create_basic_map(&params, classifier, players, &mut map_rng)
                }
            }
        } else {
            let params = basic_map_params(landscape_core);
            crate::map_creator::create_basic_map(&params, classifier, players, &mut map_rng)
        };
        let map_zoom_u32 = legacy_map_zoom(landscape_section, &mut map_rng);
        post_init_map_callbacks.set_map_zoom(map_zoom_u32 as i32);
        if let Some(creator) = retained_creator.as_mut() {
            creator.set_callback_map_zoom(map_zoom_u32 as i32);
        }
        *prepared_map_creator = retained_creator.clone();
        let mut landscape =
            classified_landscape(&bitmap, classifier, map_zoom_u32 as i32, map_seed)?;
        landscape
            .raster_state_mut()
            .expect("classified landscapes carry raster state")
            .set_map_creator(retained_creator);
        set_initial_mode(&mut landscape, LANDSCAPE_MODE_DYNAMIC);
        return Ok(Some(landscape));
    }

    // Even without activated material resources, C++ has already run the
    // map creator before evaluating MapZoom. Render into an empty classifier
    // solely to advance this local FixRandom ledger to the same position;
    // the flat compatibility landscape below remains the returned result.
    let players = startup_player_count;
    let landscape_core = &manifest.core.landscape;
    let mut discarded_classifier = MapPixelClassifier::empty_for_map_creation();
    let mut discarded_creator = None;
    if let Some(bytes) = landscape_script.as_deref() {
        let creation = crate::map_creator_s2::create_s2_map_with_state_and_functions(
            &String::from_utf8_lossy(bytes),
            &mut discarded_classifier,
            landscape_core.map_width,
            landscape_core.map_height,
            landscape_core.map_player_extend,
            players,
            &mut map_rng,
            map_callback_functions,
        );
        *post_init_map_callbacks = creation.callbacks;
        discarded_creator = Some(creation.creator);
        *prepared_map_creator = discarded_creator.clone();
        if creation.bitmap.is_none() {
            if overload_current {
                return Ok(None);
            }
            let params = basic_map_params(landscape_core);
            let _ = crate::map_creator::create_basic_map(
                &params,
                &mut discarded_classifier,
                players,
                &mut map_rng,
            );
        }
    } else {
        let params = basic_map_params(landscape_core);
        let _ = crate::map_creator::create_basic_map(
            &params,
            &mut discarded_classifier,
            players,
            &mut map_rng,
        );
    }

    let map_zoom_u32 = legacy_map_zoom(landscape_section, &mut map_rng);
    post_init_map_callbacks.set_map_zoom(map_zoom_u32 as i32);
    if let Some(creator) = discarded_creator.as_mut() {
        creator.set_callback_map_zoom(map_zoom_u32 as i32);
    }
    *prepared_map_creator = discarded_creator.clone();
    let width_product = i64::from(map_width_hint).saturating_mul(i64::from(map_zoom_u32));
    let width_u32 = width_product
        .clamp(1, i64::from(u32::MAX))
        .try_into()
        .unwrap_or(u32::MAX)
        .max(100);
    let fallback_height = map_height_hint.saturating_mul(map_zoom_u32 as i32).max(100);
    let mut landscape = Landscape::flat(width_u32, fallback_height);
    landscape.set_world_height(fallback_height);
    if discarded_creator.is_some() {
        let mut raster_state =
            LandscapeRasterState::new(map_zoom_u32 as i32, map_seed, RuntimeTexMapState::default());
        raster_state.set_map_creator(discarded_creator);
        landscape.set_raster_state(raster_state);
    }
    set_initial_mode(&mut landscape, LANDSCAPE_MODE_DYNAMIC);
    Ok(Some(landscape))
}

fn parse_legacy_c4s_value(
    _field: &str,
    raw: &str,
    defaults: LegacyC4SVal,
) -> Result<LegacyC4SVal, ScenarioError> {
    let mut values = [defaults.std, defaults.rnd, defaults.min, defaults.max];
    // C4SVal::CompileFunc defaults its individual members independently of
    // the outer naming adaptor's prefilled scenario-specific value.
    compile_defaulted_i32_components(raw, &mut values, &[0, 0, 0, 100], false);
    Ok(LegacyC4SVal::new(
        values[0], values[1], values[2], values[3],
    ))
}

fn legacy_c4s_value(
    entries: Option<&Vec<(String, String)>>,
    key: &str,
    defaults: LegacyC4SVal,
) -> Result<LegacyC4SVal, ScenarioError> {
    match entries.and_then(|entries| find_entry_including_empty(entries, key)) {
        Some(raw) => parse_legacy_c4s_value(key, raw, defaults),
        None => Ok(defaults),
    }
}

/// C4Surface::LoadAny extension search order for extension-less names
/// (C4Surface.cpp:855).
const LEGACY_SKY_EXTENSIONS: [&str; 4] = ["png", "bmp", "jpeg", "jpg"];

/// The default sky fade when `SkyDefFade` has a signed sum of zero: game
/// palette entries CSkyDef1=104 and 104+19 (C4Sky::SetFadePalette,
/// C4Sky.cpp:56-62; C4Landscape.h:34), scaled `<< 2` at load
/// (C4GraphicsResource.cpp:183-184). Values read from
/// planet/Graphics.c4g/C4.PAL.
const LEGACY_SKY_FADE_TOP_DEFAULT: RgbColor = RgbColor::new(28, 64, 152);
const LEGACY_SKY_FADE_BOTTOM_DEFAULT: RgbColor = RgbColor::new(192, 196, 252);

/// Mirrors C4Sky::Init for legacy scenario loads (C4Sky.cpp:71-152): first
/// try the scenario's implicit `Sky` bitmap, then pick one entry from SkyDef
/// with stateless SeededRandom and search the scenario before Graphics.c4g
/// (C4Sky.cpp:82-105). A loaded bitmap gets white fade, is tiled up to
/// 128x128 (SurfaceEnsureSize, C4Sky.cpp:28-52,109-111), and applies the
/// SkyScrollMode parallax mapping (C4Sky.cpp:114-125). Without one the sky is
/// the `SkyDefFade` gradient (SetFadePalette, C4Sky.cpp:54-68).
fn derive_legacy_sky(
    group: &Group,
    resolver: &dyn LegacyDefinitionResolver,
    definition_roots: &[Group],
    manifest: &mut LegacyScenarioManifest,
    random_seed: u64,
) -> Result<SkyConfig, ScenarioError> {
    let mut settings = SkySettings::default();
    let mut surface = load_legacy_sky_surface(group, "Sky");

    if surface.is_none() {
        // C4Sky::Init mutates the stored SkyDef before section selection, so
        // scripts observe semicolons even when the selected bitmap loads.
        if let Some(sky_def) = manifest.core.landscape.sky.as_mut() {
            *sky_def = sky_def.replace(',', ";");
        }

        let sky_def = manifest.core.landscape.sky.as_deref().unwrap_or_default();
        // split() preserves leading, consecutive, and trailing empty slots;
        // C++ counts all of them with SCharCount(';') + 1.
        let section_count = sky_def.split(';').count();
        let selected_index =
            crate::rng::LcgRng::seeded_random(random_seed as u32, section_count as u32) as usize;
        let selected = sky_def
            .split(';')
            .nth(selected_index)
            .unwrap_or_default()
            .trim()
            .to_string();

        if !selected.is_empty() && selected != "Default" {
            surface = load_legacy_sky_surface(group, &selected);
            if surface.is_none() {
                let graphics_groups = resolver
                    .resolve_graphics_groups_with_definition_roots(group, definition_roots)?;
                surface = load_legacy_sky_surface_from_groups(&graphics_groups, &selected);
            }
        }
    }

    if let Some((width, height, pixels)) = surface {
        settings.fade_top = RgbColor::new(255, 255, 255);
        settings.fade_bottom = RgbColor::new(255, 255, 255);
        // SkyScrollMode (C4Sky.cpp:114-125): 1 = wind-driven xdir with
        // stronger y-parallax; 2 = stronger parallax both ways
        // (ParallaxMode itself stays Fixed in case 2, like C++).
        match manifest.core.landscape.sky_scroll_mode {
            1 => {
                settings.parallax_mode = SkyParallaxMode::Wind;
                settings.parallax_y = 20;
            }
            2 => {
                settings.parallax_x = 20;
                settings.parallax_y = 20;
            }
            _ => {}
        }
        let (width, height, pixels) = ensure_sky_surface_size(width, height, pixels, 128, 128);
        settings = settings.with_surface(width, height);
        return Ok(SkyConfig {
            settings,
            surface: Some(Arc::new(GraphicsImage::new(width, height, pixels))),
        });
    }

    // No sky surface: fade gradient (C4Sky.cpp:129-134). A zero signed sum
    // across SkyDefFade selects the palette default (C4Sky.cpp:56-62).
    let fade = manifest.core.landscape.sky_fade;
    if fade.iter().sum::<i32>() == 0 {
        settings.fade_top = LEGACY_SKY_FADE_TOP_DEFAULT;
        settings.fade_bottom = LEGACY_SKY_FADE_BOTTOM_DEFAULT;
    } else {
        // C4RGB projects every signed channel through `& 0xff`; it does not
        // clamp values outside the byte range (StdColors.h:52).
        let channel = |value: i32| value as u8;
        settings.fade_top = RgbColor::new(channel(fade[0]), channel(fade[1]), channel(fade[2]));
        settings.fade_bottom = RgbColor::new(channel(fade[3]), channel(fade[4]), channel(fade[5]));
    }
    Ok(SkyConfig {
        settings,
        surface: None,
    })
}

/// C4Surface::LoadAny filename candidates. An explicit extension suppresses
/// extension probing; otherwise png/bmp/jpeg/jpg are tried in this order
/// (C4Surface.cpp:846-865).
fn legacy_sky_filename_patterns(name: &str) -> Vec<String> {
    if Path::new(name)
        .extension()
        .is_some_and(|extension| !extension.is_empty())
    {
        vec![name.to_string()]
    } else {
        LEGACY_SKY_EXTENSIONS
            .iter()
            .map(|extension| format!("{name}.{extension}"))
            .collect()
    }
}

/// Byte-for-byte equivalent of StdFile's ASCII-insensitive `WildcardMatch`,
/// used by C4Group::FindEntry for SkyDef names such as `Pyroclastic*`.
fn legacy_group_wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if value_index >= value.len() {
            break;
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            let next_value = saved_value + 1;
            pattern_index = saved_pattern;
            value_index = next_value;
            backtrack_value = Some(next_value);
        } else {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

enum LegacySkyEntryMatch {
    Missing,
    Found(Option<Vec<u8>>),
}

fn read_legacy_group_wildcard(group: &Group, pattern: &str) -> LegacySkyEntryMatch {
    let Ok(entries) = group.entries() else {
        return LegacySkyEntryMatch::Missing;
    };
    let Some(entry) = entries
        .into_iter()
        .find(|entry| legacy_group_wildcard_match(pattern.as_bytes(), entry.name_bytes.as_slice()))
    else {
        return LegacySkyEntryMatch::Missing;
    };
    LegacySkyEntryMatch::Found(group.read_entry_bytes_exact(&entry).ok())
}

fn decode_legacy_sky_surface(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    load_from_memory(bytes).ok().map(|decoded| {
        let rgba = decoded.to_rgba8();
        let (width, height) = rgba.dimensions();
        (width, height, rgba.into_raw())
    })
}

/// Load a named surface from one scenario group. The first existing extension
/// is decoded once; a broken higher-priority file does not fall through to a
/// lower-priority extension (C4Surface.cpp:846-865).
fn load_legacy_sky_surface(group: &Group, name: &str) -> Option<(u32, u32, Vec<u8>)> {
    for pattern in legacy_sky_filename_patterns(name) {
        match read_legacy_group_wildcard(group, &pattern) {
            LegacySkyEntryMatch::Missing => {}
            LegacySkyEntryMatch::Found(bytes) => {
                return bytes.as_deref().and_then(decode_legacy_sky_surface);
            }
        }
    }
    None
}

/// Load an extensionless SkyDef name from the ordered GraphicsResource group
/// set. C4Surface's group-set overload searches extension before group and,
/// due to its exact control flow, returns false for already-extended names
/// (C4Surface.cpp:867-890).
fn load_legacy_sky_surface_from_groups(
    groups: &[Group],
    name: &str,
) -> Option<(u32, u32, Vec<u8>)> {
    if Path::new(name)
        .extension()
        .is_some_and(|extension| !extension.is_empty())
    {
        return None;
    }
    for extension in LEGACY_SKY_EXTENSIONS {
        let pattern = format!("{name}.{extension}");
        for group in groups {
            match read_legacy_group_wildcard(group, &pattern) {
                LegacySkyEntryMatch::Missing => {}
                LegacySkyEntryMatch::Found(bytes) => {
                    return bytes.as_deref().and_then(decode_legacy_sky_surface);
                }
            }
        }
    }
    None
}

/// SurfaceEnsureSize (C4Sky.cpp:28-52): enlarge to at least
/// `min_width` x `min_height` by whole-tile repetition of the original.
fn ensure_sky_surface_size(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    min_width: u32,
    min_height: u32,
) -> (u32, u32, Vec<u8>) {
    if width == 0 || height == 0 {
        return (width, height, pixels);
    }
    let mut dest_width = width;
    let mut dest_height = height;
    while dest_width < min_width {
        dest_width += width;
    }
    while dest_height < min_height {
        dest_height += height;
    }
    if dest_width == width && dest_height == height {
        return (width, height, pixels);
    }
    let row_bytes = (width * 4) as usize;
    let mut enlarged = Vec::with_capacity((dest_width * dest_height * 4) as usize);
    for y in 0..dest_height {
        let source_row = &pixels[(y % height) as usize * row_bytes..][..row_bytes];
        for _ in 0..dest_width / width {
            enlarged.extend_from_slice(source_row);
        }
    }
    (dest_width, dest_height, enlarged)
}

fn derive_legacy_physics(
    manifest: &LegacyScenarioManifest,
) -> Result<(Option<PhysicsSettings>, LegacyC4SVal), ScenarioError> {
    let gravity_defaults = LegacyC4SVal::new(100, 0, 10, 200);
    let entries = manifest.sections.get("landscape");
    if entries.is_none() {
        return Ok((None, gravity_defaults));
    }
    let gravity = legacy_c4s_value(entries, "gravity", gravity_defaults)?;
    let mut physics = PhysicsSettings::default();
    physics.gravity = gravity.base();
    Ok((Some(physics), gravity))
}

/// The C4SVals C4Weather::Init evaluates at scenario start
/// (C4Weather.cpp:36-70) plus the NoInitialize gate for the rain-cloud
/// block (:49-58) and the late NoGamma assignment (:65).
#[derive(Debug, Clone)]
#[doc(hidden)]
pub struct LegacyWeatherInit {
    #[doc(hidden)]
    pub season: LegacyC4SVal,
    #[doc(hidden)]
    pub year_speed: LegacyC4SVal,
    #[doc(hidden)]
    pub climate: LegacyC4SVal,
    #[doc(hidden)]
    pub wind: LegacyC4SVal,
    #[doc(hidden)]
    pub rain: LegacyC4SVal,
    #[doc(hidden)]
    pub precipitation: String,
    #[doc(hidden)]
    pub lightning: LegacyC4SVal,
    #[doc(hidden)]
    pub meteorite: LegacyC4SVal,
    #[doc(hidden)]
    pub volcano: LegacyC4SVal,
    #[doc(hidden)]
    pub earthquake: LegacyC4SVal,
    #[doc(hidden)]
    pub no_initialize: bool,
    #[doc(hidden)]
    pub no_gamma: bool,
}

fn derive_legacy_weather_init(
    manifest: &LegacyScenarioManifest,
) -> Result<LegacyWeatherInit, ScenarioError> {
    let weather = manifest.sections.get("weather");
    let disasters = manifest.sections.get("disasters");
    // C4SWeather::Default (C4Scenario.cpp:372-379) and
    // C4SDisasters::Default (:427-432); C4SVal::Default = (0,0,0,100).
    Ok(LegacyWeatherInit {
        season: legacy_c4s_value(weather, "startseason", LegacyC4SVal::new(50, 50, 0, 100))?,
        year_speed: legacy_c4s_value(weather, "yearspeed", LegacyC4SVal::new(50, 0, 0, 100))?,
        climate: legacy_c4s_value(weather, "climate", LegacyC4SVal::new(50, 10, 0, 100))?,
        wind: legacy_c4s_value(weather, "wind", LegacyC4SVal::new(0, 70, -100, 100))?,
        rain: legacy_c4s_value(weather, "rain", LegacyC4SVal::new(0, 0, 0, 100))?,
        precipitation: manifest.core.weather.precipitation.clone(),
        lightning: legacy_c4s_value(weather, "lightning", LegacyC4SVal::new(0, 0, 0, 100))?,
        meteorite: legacy_c4s_value(disasters, "meteorite", LegacyC4SVal::new(0, 0, 0, 100))?,
        volcano: legacy_c4s_value(disasters, "volcano", LegacyC4SVal::new(0, 0, 0, 100))?,
        earthquake: legacy_c4s_value(disasters, "earthquake", LegacyC4SVal::new(0, 0, 0, 100))?,
        no_initialize: manifest.core.head.no_initialize != 0,
        no_gamma: manifest.core.weather.no_gamma,
    })
}

fn derive_legacy_environment(
    manifest: &LegacyScenarioManifest,
) -> Result<EnvironmentSettings, ScenarioError> {
    let weather_entries = manifest.sections.get("weather");
    let disasters_entries = manifest.sections.get("disasters");

    let wind_defaults = LegacyC4SVal::new(0, 70, -100, 100);
    let wind = legacy_c4s_value(weather_entries, "wind", wind_defaults)?;
    let mut environment = EnvironmentSettings::new(wind.base());
    environment.set_legacy_wind_value(wind.std, wind.rnd, wind.min, wind.max);

    let climate_defaults = LegacyC4SVal::new(50, 10, 0, 100);
    let climate_value = legacy_c4s_value(weather_entries, "climate", climate_defaults)?;
    let climate = 100 - climate_value.base() - 50;
    environment = environment.with_climate(climate);
    environment = environment.with_temperature(climate);

    let season_defaults = LegacyC4SVal::new(50, 50, 0, 100);
    let season_value = legacy_c4s_value(weather_entries, "startseason", season_defaults)?;
    // C4Weather::Init assigns StartSeason.Evaluate() without an extra
    // clamp (C4Weather.cpp:41); the C4SVal Min/Max also bound Execute's
    // season wrap (:82-83).
    environment.season = season_value.base();
    environment = environment.with_season_bounds(season_value.min, season_value.max);

    let year_defaults = LegacyC4SVal::new(50, 0, 0, 100);
    let year_speed = legacy_c4s_value(weather_entries, "yearspeed", year_defaults)?.base();
    environment = environment.with_year_speed(year_speed);

    let rain_defaults = LegacyC4SVal::new(0, 0, 0, 100);
    let rain_value = legacy_c4s_value(weather_entries, "rain", rain_defaults)?.base();
    environment = environment.with_precipitation(rain_value);
    environment = environment.with_precipitation_strength(rain_value);

    let lightning_defaults = LegacyC4SVal::new(0, 0, 0, 100);
    let lightning = legacy_c4s_value(weather_entries, "lightning", lightning_defaults)?.base();
    environment = environment.with_lightning(lightning);

    let no_gamma = weather_entries
        .and_then(|entries| find_entry(entries, "nogamma"))
        .and_then(|value| parse_legacy_bool(&value))
        .unwrap_or(true);
    environment = if no_gamma {
        environment.with_gamma_disabled()
    } else {
        environment.with_gamma_enabled()
    };

    let disaster_defaults = LegacyC4SVal::new(0, 0, 0, 100);
    let meteorite = legacy_c4s_value(disasters_entries, "meteorite", disaster_defaults)?.base();
    let volcano = legacy_c4s_value(disasters_entries, "volcano", disaster_defaults)?.base();
    let earthquake = legacy_c4s_value(disasters_entries, "earthquake", disaster_defaults)?.base();
    environment = environment
        .with_meteorite(meteorite)
        .with_volcano(volcano)
        .with_earthquake(earthquake);

    Ok(environment)
}

fn legacy_scenario_section_name(path: &Path) -> Result<Option<String>, ScenarioError> {
    if path.components().count() != 1 {
        return Ok(None);
    }
    let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
        return Ok(None);
    };
    let lower = filename.to_ascii_lowercase();
    if !lower.starts_with("sect") || !lower.ends_with(".c4g") {
        return Ok(None);
    }
    let name = &filename[4..filename.len() - 4];
    if name.is_empty() || name.len() > 30 {
        return Err(ScenarioError::InvalidScenarioSectionName {
            path: path.to_path_buf(),
        });
    }
    Ok(Some(name.to_owned()))
}

fn load_legacy_landscape_systems(group: &Group) -> Result<ScenarioLandscapeSystems, ScenarioError> {
    let mut ignore_progress = |_: i32, _: &'static str| {};
    load_legacy_landscape_systems_with_progress(group, &mut ignore_progress)
}

fn load_legacy_landscape_systems_with_progress(
    group: &Group,
    report_progress: &mut dyn FnMut(i32, &'static str),
) -> Result<ScenarioLandscapeSystems, ScenarioError> {
    let pxs = read_optional_legacy_entry(group, "PXS.c4b")?
        .map(|bytes| {
            crate::pxs::PxsSystem::from_c4b(&bytes)
                .map_err(|error| ScenarioError::LegacyParse(error.to_string()))
        })
        .transpose()?;
    report_progress(91, "PXS loading complete");
    let mass_movers = read_optional_legacy_entry(group, "MassMover.c4b")?
        .map(|bytes| {
            crate::mass_mover::MassMoverSet::from_c4b(&bytes)
                .map_err(|error| ScenarioError::LegacyParse(error.to_string()))
        })
        .transpose()?;
    report_progress(92, "Mass mover loading complete");
    Ok(ScenarioLandscapeSystems { pxs, mass_movers })
}

#[allow(clippy::too_many_arguments)]
fn load_legacy_scenario_sections(
    group: &Group,
    main_manifest: &LegacyScenarioManifest,
    classifier: Option<&mut MapPixelClassifier>,
    random_seed: u64,
    startup_player_count: i32,
    root_section_name: &str,
    main_landscape: &Option<Landscape>,
    main_landscape_systems: &ScenarioLandscapeSystems,
    main_objects: &[ScenarioSpawn],
    main_environment: EnvironmentSettings,
    has_sky_surface: bool,
    map_callback_functions: &HashSet<String>,
    main_post_init_map_callbacks: &crate::map_creator_s2::PostInitMapCallbacks,
) -> Result<Vec<ScenarioSectionSpec>, ScenarioError> {
    let classifier_baseline = classifier.as_deref().cloned();
    let persistent_runtime = main_landscape.as_ref().map(|landscape| LandscapeGameData {
        map_seed: landscape.map_seed(),
        mat_modulation: landscape.modulation(),
        ..LandscapeGameData::default()
    });
    let main_s2_source = try_read_group_file_case_insensitive(group, "Landscape.txt")?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let main_s2_diff = try_read_group_file_case_insensitive(group, "DiffLandscape.bmp")
        .ok()
        .flatten()
        .and_then(|bytes| clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok());
    let main_has_s2_creator = main_landscape
        .as_ref()
        .and_then(Landscape::raster_state)
        .and_then(LandscapeRasterState::map_creator)
        .is_some();
    let main_s2_overload = main_s2_source
        .filter(|_| main_has_s2_creator)
        .map(|source| ScenarioSectionS2Spec {
            source,
            map_width: main_manifest.core.landscape.map_width,
            map_height: main_manifest.core.landscape.map_height,
            map_player_extend: main_manifest.core.landscape.map_player_extend,
            player_count: startup_player_count,
            map_zoom: main_manifest.core.landscape.map_zoom,
            diff: main_s2_diff,
            left_open: main_manifest.core.landscape.left_open,
            right_open: main_manifest.core.landscape.right_open,
            top_open: main_manifest.core.landscape.top_open,
            bottom_open: main_manifest.core.landscape.bottom_open,
            auto_scan_side_open: main_manifest.core.landscape.auto_scan_side_open,
            no_scan: main_manifest.core.landscape.no_scan,
            shade_materials: main_manifest.core.landscape.shade_materials,
            script_functions: map_callback_functions.clone(),
        });
    let mut sections = vec![ScenarioSectionSpec {
        // An exact save stores the live current section in the root and
        // identifies it through Game.CurrentScenarioSection. `SectMain.c4g`
        // is then a distinct departed section when the current one is not
        // Main (C4GameSave::SaveScenarioSections).
        name: root_section_name.to_string(),
        source_group: Some(group.clone()),
        landscape: main_landscape.clone(),
        landscape_systems: main_landscape_systems.clone(),
        exact_landscape: main_manifest.core.landscape.exact_landscape,
        texmap_lookups: Vec::new(),
        resynthesize_static_map: false,
        map_creator: main_landscape
            .as_ref()
            .and_then(Landscape::raster_state)
            .and_then(LandscapeRasterState::map_creator)
            .cloned(),
        s2_overload: main_s2_overload,
        gravity: main_manifest.core.landscape.gravity,
        post_init_map_callbacks: main_post_init_map_callbacks.clone(),
        keep_map_creator: main_manifest.core.landscape.keep_map_creator,
        no_initialize: main_manifest.core.head.no_initialize != 0,
        objects: main_objects.to_vec(),
        scenario_values: ScenarioValueStore::from_runtime_core(
            &main_manifest.core,
            has_sky_surface,
        )
        .with_section_head_defaults(&main_manifest.core.head),
        base_reject_entrance_enabled: (main_manifest.core.game.realism.base_functionality
            & BASEFUNC_REJECT_ENTRANCE)
            != 0,
        base_extinguish_enabled: (main_manifest.core.game.realism.base_functionality
            & BASEFUNC_EXTINGUISH)
            != 0,
        environment: main_environment,
    }];

    let mut discovered = Vec::new();
    for entry in group.entries()? {
        let Some(name) = legacy_scenario_section_name(&entry.relative_path)? else {
            continue;
        };
        // The root always wins a stale duplicate. Unlike the old hard-coded
        // Main filter, this retains SectMain when another section is current.
        if !name.eq_ignore_ascii_case(root_section_name) {
            discovered.push((name, entry.relative_path));
        }
    }
    discovered.sort_by(|(left, _), (right, _)| {
        left.to_ascii_lowercase()
            .cmp(&right.to_ascii_lowercase())
            .then_with(|| left.cmp(right))
    });

    for (name, path) in discovered {
        let mut section_classifier = classifier_baseline.clone();
        if let Some(classifier) = section_classifier.as_mut() {
            classifier.clear_texmap_lookups();
        }
        let section_group = group.open_child(path)?;
        let manifest = match parse_legacy_scenario_manifest(&section_group) {
            Ok(overlay) => Some(overlay_legacy_scenario_manifest(main_manifest, overlay)?),
            Err(ScenarioError::LegacyCoreMissing) => None,
            Err(error) => return Err(error),
        };
        let manifest = manifest.as_ref().unwrap_or(main_manifest);
        let s2_source = try_read_group_file_case_insensitive(&section_group, "Landscape.txt")?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let s2_diff = try_read_group_file_case_insensitive(&section_group, "DiffLandscape.bmp")
            .ok()
            .flatten()
            .and_then(|bytes| clonk_resources::bitmap::IndexedBitmap::decode(&bytes).ok());
        let mut post_init_map_callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut prepared_map_creator = None;
        let mut landscape = load_legacy_landscape(
            &section_group,
            manifest,
            persistent_runtime.as_ref(),
            true,
            section_classifier.as_mut(),
            random_seed,
            startup_player_count,
            map_callback_functions,
            &mut post_init_map_callbacks,
            &mut prepared_map_creator,
        )?;
        let landscape_systems = load_legacy_landscape_systems(&section_group)?;
        if let (Some(runtime), Some(landscape)) = (persistent_runtime, landscape.as_mut()) {
            landscape.set_modulation(runtime.mat_modulation);
        }
        let texmap_lookups = section_classifier
            .as_ref()
            .map(|classifier| classifier.texmap_lookups().to_vec())
            .unwrap_or_default();
        let resynthesize_static_map = !manifest.core.landscape.exact_landscape
            && landscape
                .as_ref()
                .and_then(Landscape::raster_state)
                .is_some_and(|state| state.map().is_some() && state.map_creator().is_none());
        let environment = derive_legacy_environment(manifest)?;
        let scenario_values =
            ScenarioValueStore::from_runtime_core(&manifest.core, has_sky_surface)
                .with_section_head_defaults(&main_manifest.core.head);
        let has_s2_overload = prepared_map_creator.is_some() && s2_source.is_some();
        sections.push(ScenarioSectionSpec {
            name,
            source_group: Some(section_group),
            landscape,
            landscape_systems,
            exact_landscape: manifest.core.landscape.exact_landscape,
            texmap_lookups,
            resynthesize_static_map,
            map_creator: prepared_map_creator,
            s2_overload: has_s2_overload
                .then(|| {
                    s2_source.map(|source| ScenarioSectionS2Spec {
                        source,
                        map_width: manifest.core.landscape.map_width,
                        map_height: manifest.core.landscape.map_height,
                        map_player_extend: manifest.core.landscape.map_player_extend,
                        player_count: startup_player_count,
                        map_zoom: manifest.core.landscape.map_zoom,
                        diff: s2_diff,
                        left_open: manifest.core.landscape.left_open,
                        right_open: manifest.core.landscape.right_open,
                        top_open: manifest.core.landscape.top_open,
                        bottom_open: manifest.core.landscape.bottom_open,
                        auto_scan_side_open: manifest.core.landscape.auto_scan_side_open,
                        no_scan: manifest.core.landscape.no_scan,
                        shade_materials: manifest.core.landscape.shade_materials,
                        script_functions: map_callback_functions.clone(),
                    })
                })
                .flatten(),
            gravity: manifest.core.landscape.gravity,
            post_init_map_callbacks,
            keep_map_creator: manifest.core.landscape.keep_map_creator,
            no_initialize: manifest.core.head.no_initialize != 0,
            // C4ScenarioSection retains the child group but does not compile
            // Objects.txt during scenario discovery. C4GameObjects::Load
            // reopens and compiles it on every activation against the then-
            // current process-global C4StringTable.
            objects: Vec::new(),
            scenario_values,
            base_reject_entrance_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_REJECT_ENTRANCE)
                != 0,
            base_extinguish_enabled: (manifest.core.game.realism.base_functionality
                & BASEFUNC_EXTINGUISH)
                != 0,
            environment,
        });
    }

    Ok(sections)
}

fn collect_legacy_objects(
    group: &Group,
    definitions: &[ScenarioDefinition],
    string_registrations: &clonk_script::StringRegistrations,
) -> Result<Vec<ScenarioSpawn>, ScenarioError> {
    let definition_ids = definitions
        .iter()
        .map(|definition| definition.id.as_str())
        .collect::<HashSet<_>>();
    collect_legacy_objects_with_definition_ids(
        group,
        &definition_ids,
        string_registrations,
        &HashSet::new(),
    )
}

/// Compile one section's Objects.txt at its C4GameObjects::Load boundary.
/// Section groups do not own a string table: S# values resolve against the
/// process-global table as it exists at this activation.
pub(crate) fn collect_legacy_objects_with_definition_ids(
    group: &Group,
    definition_ids: &HashSet<&str>,
    string_registrations: &clonk_script::StringRegistrations,
    retained_object_numbers: &HashSet<u64>,
) -> Result<Vec<ScenarioSpawn>, ScenarioError> {
    let bytes = match group.read_file("Objects.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(Vec::new()),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    // C++ reads Objects.txt as raw bytes in the config charset; fall back to
    // a Latin-1 decode so Windows-1252 umlauts survive (Drachenfels.c4s).
    let text = String::from_utf8(bytes).unwrap_or_else(|error| {
        error
            .into_bytes()
            .iter()
            .map(|&byte| byte as char)
            .collect()
    });
    let mut records = parse_legacy_objects(&text)?;
    if records.is_empty() {
        return Ok(Vec::new());
    }

    let mut index_by_number: HashMap<u64, usize> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        if let Some(number) = record.number {
            index_by_number.insert(number, index);
        }
    }

    for index in 0..records.len() {
        let parent_number = match records[index].number {
            Some(value) => value,
            None => continue,
        };
        let child_numbers: Vec<u64> = records[index].contents.clone();
        for child_number in child_numbers {
            if child_number == 0 || child_number == parent_number {
                continue;
            }
            if let Some(child_index) = index_by_number.get(&child_number).copied() {
                if records[child_index].contained.is_none()
                    && records[child_index].inferred_container.is_none()
                {
                    records[child_index].inferred_container = Some(parent_number);
                }
            }
        }
    }

    // C4GameObjects::ObjectPointer searches both the newly compiled main
    // list and the retained inactive list. Section loads therefore resolve
    // saved command pointers to preserved objects as well as sibling rows.
    let mut object_numbers = retained_object_numbers.clone();
    object_numbers.extend(
        records
            .iter()
            .filter(|record| !matches!(record.status, Some(ObjectStatus::Deleted)))
            .filter(|record| {
                record
                    .id
                    .as_deref()
                    .is_some_and(|id| definition_ids.contains(id))
            })
            .filter_map(|record| record.number),
    );
    let value_resolution = SerializedC4ValueResolution {
        object_numbers: &object_numbers,
        string_registrations,
    };

    let mut spawns = Vec::new();
    for record in records.into_iter() {
        if let Some(spawn) = record.into_spawn(definition_ids, &value_resolution)? {
            spawns.push(spawn);
        }
    }
    Ok(spawns)
}

/// C4StringTable::Load assigns each Strings.txt line its zero-based enum ID.
/// Repeated text reuses the existing C4String and updates that one instance to
/// the later ID, so the earlier ID is no longer resolvable
/// (C4StringTable.cpp:201-216).
fn load_legacy_string_table(
    group: &Group,
) -> Result<clonk_script::StringRegistrations, ScenarioError> {
    let string_registrations = clonk_script::new_string_registrations();
    let bytes = match group.read_file("Strings.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(string_registrations),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(string_registrations);
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    // SCopySegment/SCharPos scan the component as a C string. Bytes after
    // the first embedded NUL are therefore invisible to the whole line walk.
    let bytes = &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())];
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let index = i32::try_from(index).unwrap_or(i32::MAX);
        // SCopySegment copies at most C4AUL_MAX_String bytes and
        // SReplaceChar turns the first CR into the string terminator
        // (C4StringTable.cpp:208-211).
        let end = line
            .iter()
            .position(|byte| *byte == b'\r')
            .unwrap_or(line.len())
            .min(1024);
        // C4StringTable::Load passes the component bytes straight to
        // RegString. Strings.txt is not presentation text and must not pass
        // through the CP1252-to-Unicode decoder used by names/descriptions.
        let value = clonk_script::c4_string_from_bytes(&line[..end]);
        // RegisterLoaded performs the native C-string-prefix lookup. Equal
        // lines reuse one C4String identity and the later line overwrites
        // that shared instance's current enumeration ID.
        clonk_script::register_loaded_c4_string(&string_registrations, index, &value);
    }
    Ok(string_registrations)
}

#[derive(Debug, Default)]
struct LegacyObjectRecord {
    line: usize,
    id: Option<String>,
    number: Option<u64>,
    /// C4Object::CustomName (`Name=`, C4Object.cpp:2749-2760).
    custom_name: Option<String>,
    /// Player-owned C4ObjectInfo lookup name (`Info=`).
    info_name: Option<String>,
    status: Option<ObjectStatus>,
    owner: Option<i32>,
    /// C4Object::Controller, compiled verbatim with default NO_OWNER
    /// (C4Object.cpp:2739).
    controller: Option<i32>,
    /// Kill attribution cached by C4Object (`LastEngLossPlr=`).
    last_energy_loss_cause: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
    motion_x: Option<i32>,
    motion_y: Option<i32>,
    /// The frame of the most recent solid-attachment movement. Native uses
    /// -1 as its compile default; retain the signed word until spawn wiring
    /// converts it to the engine's optional frame representation.
    last_attach_movement_frame: Option<i32>,
    no_collect_delay: Option<i32>,
    base: Option<i32>,
    /// Saved velocity is C4Fixed (C4Object.cpp:2765-2766), float-encoded
    /// in real content (`XDir=f...`).
    xdir: Option<crate::math::C4Fixed>,
    ydir: Option<crate::math::C4Fixed>,
    /// Saved sub-pixel position/rotation and angular velocity — the same
    /// C4Fixed encoding (FixX/FixY/FixR/RDir, C4Object.cpp:2762-2767).
    /// C++ reads them INDEPENDENTLY of the integer X/Y/Rotation and never
    /// reconciles after load.
    fix_x: Option<crate::math::C4Fixed>,
    fix_y: Option<crate::math::C4Fixed>,
    fix_r: Option<crate::math::C4Fixed>,
    rdir: Option<crate::math::C4Fixed>,
    /// C4Object::Mobile, serialized with default false (C4Object.cpp:2772).
    mobile: Option<bool>,
    solid_mask: Option<Vec<i32>>,
    /// Whole-degree rotation (`Rotation=`, C4Object.cpp:2744).
    rotation: Option<i32>,
    /// Mid-cycle Def TimerCall counter (`Timer=`, default 0,
    /// C4Object.cpp:2738).
    timer: Option<i32>,
    /// Numbered C4Object::Local slots (`Locals=`, C4Object.cpp:2788;
    /// C4ValueList::CompileFunc, C4ValueList.cpp:102-136).
    locals: Option<Vec<SerializedC4Value>>,
    /// Per-object script locals (`LocalNamed=`, C4Object.cpp:2788;
    /// C4ValueMapData::CompileFunc, C4ValueMap.cpp:236-295).
    local_named: Option<Vec<(String, SerializedC4Value)>>,
    /// The CURRENT shape's vertices, serialized by C4Shape::CompileFunc
    /// into the [Object] section (C4Shape.cpp:495-515): the effective
    /// post-Con/rotation shape, loaded verbatim.
    vertex_count: Option<i32>,
    vertex_x: Option<Vec<i32>>,
    vertex_y: Option<Vec<i32>>,
    vertex_cnat: Option<Vec<i32>>,
    vertex_friction: Option<Vec<i32>>,
    /// Exact live C4Shape rectangle compiled inline into the Object section.
    shape_width: Option<i32>,
    shape_height: Option<i32>,
    shape_offset: Option<Vec<i32>>,
    /// Exact live C4Shape::FireTop; missing values compile as zero.
    shape_fire_top: Option<i32>,
    /// C4Object::fOwnVertices. The original vertex copy occupies raw shape
    /// slots 15.. and is used by later UpdateShape calls.
    own_vertices: Option<bool>,
    /// Saved live C4Shape::ContactDensity (C4Shape.cpp:495-510).
    contact_density: Option<i32>,
    shape_attach_x: Option<i32>,
    shape_attach_y: Option<i32>,
    shape_attach_vertex: Option<i32>,
    own_mass: Option<i32>,
    /// C4Object::Mass is compiled independently of OwnMass. C++ currently
    /// refreshes derived mass on later construction changes, not as part of
    /// Objects.txt parsing, so keep the serialized cache available.
    mass: Option<i32>,
    damage: Option<i32>,
    energy: Option<i32>,
    /// C4Object::NeedEnergy (`NeedEnergy=`, C4Object.cpp:2805).
    need_energy: Option<bool>,
    /// C4Object::Select (`Selected=`, C4Object.cpp:2800).
    selected: Option<bool>,
    /// C4Object::MagicEnergy (`MagicEnergy=`, C4Object.cpp:2768).
    magic_energy: Option<i32>,
    construction: Option<i32>,
    alive: Option<bool>,
    breath: Option<i32>,
    fire_phase: Option<i32>,
    on_fire: Option<bool>,
    in_liquid: Option<bool>,
    /// C4Object::EntranceStatus (`EntranceStatus=`, C4Object.cpp:2803).
    entrance_status: Option<bool>,
    physical_temporary: Option<bool>,
    ocf: Option<u32>,
    category: Option<i32>,
    direction: Option<Direction>,
    command_direction: Option<CommandDirection>,
    action_name: Option<String>,
    action_phase: Option<i32>,
    /// Action.Time (`ActionTime=`, C4Object.cpp:2745 area).
    action_ticks: Option<i32>,
    /// Action.PhaseDelay (`PhaseDelay=`), the intra-phase counter.
    action_phase_delay: Option<i32>,
    action_data: Option<i32>,
    action_target: Option<i32>,
    action_target2: Option<i32>,
    /// Raw C4EnumeratedObjectPtr::number for C4Object::pLayer. Keep the
    /// signed cache word even when denumeration cannot resolve a pointer.
    layer: Option<i32>,
    /// C4Object::Visibility (`Visibility=`, C4Object.cpp:2814).
    visibility: Option<i32>,
    /// C4Object::BlitMode (`BlitMode=`, C4Object.cpp:2817).
    blit_mode: Option<u32>,
    /// C4Object::Color (`Color=`/`ColorDw=`, C4Object.cpp:2786-2787).
    color: Option<u32>,
    /// C4Object::ColorMod (`ColorMod=`, C4Object.cpp:2816).
    color_modulation: Option<u32>,
    /// C4Object::PictureRect (`Picture=`, C4Object.cpp:2798).
    picture_rect: Option<DefinitionRect>,
    plr_view_range: Option<i32>,
    crew_disabled: Option<bool>,
    base_graphics: Option<crate::ObjectBaseGraphics>,
    draw_transform: Option<crate::DrawTransform>,
    effects: Option<Vec<SerializedEffectState>>,
    graphics_overlays: Option<Vec<SerializedObjectGraphicsOverlay>>,
    temporary_physical: Option<crate::PhysicalInfo>,
    physical_changes: Vec<(String, i32)>,
    /// StdCompilerINIRead removes the first matching naming node after it is
    /// consumed, so duplicate C4PhysicalInfo names never overwrite it.
    physical_fields_seen: HashSet<String>,
    commands: BTreeMap<usize, SerializedLegacyCommand>,
    /// Saved C4Object::Component (`Component=WOOD=5;METL=1;`).
    components: Option<Vec<(DefinitionId, i32)>>,
    /// Raw C4EnumeratedObjectPtr::number for C4Object::Contained.
    contained: Option<i32>,
    /// Live relationship inferred from a parent's Contents list when the
    /// child's serialized Contained cache was absent. Keep this separate so
    /// GetObjectVal still observes the native zero compiler default.
    inferred_container: Option<u64>,
    contents: Vec<u64>,
}

/// Object references inside a graphics overlay are denumerated only after
/// all Objects.txt rows have been accepted. Keep the raw signed number here
/// while parsing, just like [`SerializedC4Value::ObjectNumber`].
#[derive(Debug, Clone, PartialEq)]
struct SerializedObjectGraphicsOverlay {
    id: i32,
    mode: crate::GraphicsOverlayMode,
    definition: Option<DefinitionId>,
    graphics_name: Option<String>,
    action: Option<String>,
    blit_mode: u32,
    phase: i32,
    transform: crate::DrawTransform,
    color_modulation: u32,
    overlay_object: i32,
}

/// Exact C4Command::CompileFunc projection. Integer flags intentionally remain
/// integers: the native fields are int32 words and old saves may contain
/// non-canonical truthy values. The parser also accepts the native `$1` and
/// unversioned layouts; all versions resolve into this current representation.
#[derive(Debug, Clone, PartialEq)]
struct SerializedLegacyCommand {
    name: String,
    tx: SerializedC4Value,
    ty: i32,
    target: i32,
    target2: i32,
    data: i32,
    update_interval: i32,
    evaluated: i32,
    path_checked: i32,
    finished: i32,
    failures: i32,
    retries: i32,
    permit: i32,
    base_mode: i32,
    text: String,
}

fn denumerate_legacy_object_number(raw: i32, object_numbers: &HashSet<u64>) -> Option<ObjectId> {
    let number = if (1_000_000_000..=1_001_000_000).contains(&raw) {
        raw - 1_000_000_000
    } else {
        raw
    };
    u64::try_from(number)
        .ok()
        .filter(|number| *number != 0 && object_numbers.contains(number))
        .map(ObjectId::new)
}

impl SerializedObjectGraphicsOverlay {
    fn resolve(self, object_numbers: &HashSet<u64>) -> crate::ObjectGraphicsOverlay {
        crate::ObjectGraphicsOverlay {
            id: self.id,
            mode: self.mode,
            definition: self.definition,
            graphics_name: self.graphics_name,
            action: self.action,
            phase: self.phase,
            blit_mode: self.blit_mode,
            color_modulation: self.color_modulation,
            overlay_object: denumerate_legacy_object_number(self.overlay_object, object_numbers),
            transform: Some(self.transform),
        }
    }
}

impl SerializedLegacyCommand {
    fn resolve(
        self,
        _line: usize,
        resolution: &SerializedC4ValueResolution<'_>,
    ) -> Result<crate::command::LegacyCommandSave, ScenarioError> {
        let is_call = self.name == "Call";
        let tx_value = self.tx.resolve(resolution);
        let (tx, tx_definition) = match &tx_value {
            clonk_script::Value::Nil => (None, None),
            clonk_script::Value::Int(value) => (Some(*value), None),
            clonk_script::Value::C4Id(value) => (None, Some(value.clone())),
            _ => (None, None),
        };
        Ok(crate::command::LegacyCommandSave {
            view: crate::command::CommandView {
                name: self.name,
                target: denumerate_legacy_object_number(self.target, resolution.object_numbers),
                tx,
                tx_value: Some(tx_value),
                tx_definition,
                ty: Some(self.ty),
                target2: denumerate_legacy_object_number(self.target2, resolution.object_numbers),
                data: crate::command::CommandData::Integer(self.data),
                legacy_data: is_call.then_some(self.data),
                finished: self.finished != 0,
            },
            update_interval: self.update_interval,
            evaluated: self.evaluated,
            path_checked: self.path_checked,
            finished: self.finished,
            failures: self.failures,
            retries: self.retries,
            permit: self.permit,
            base_mode: self.base_mode,
            text: self.text,
        })
    }
}

impl LegacyObjectRecord {
    fn new(line: usize) -> Self {
        Self {
            line,
            ..Self::default()
        }
    }

    fn apply_property(&mut self, key: &str, value: &str) -> Result<(), ScenarioError> {
        // StdCompilerINIRead looks up naming nodes byte-for-byte. Map only
        // the exact spellings used by C4Object::CompileFunc; a wrong-case
        // line is an unused naming and leaves the compile default intact.
        let normalized_key = match key {
            "id" => "id",
            "Name" => "name",
            "Number" => "number",
            "Status" => "status",
            "Info" => "info",
            "Owner" => "owner",
            "Timer" => "timer",
            "Controller" => "controller",
            "LastEngLossPlr" => "lastenglossplr",
            "Category" => "category",
            "X" => "x",
            "Y" => "y",
            "Rotation" => "rotation",
            "MotionX" => "motionx",
            "MotionY" => "motiony",
            "LastSolidAtchFrame" => "lastsolidatchframe",
            "NoCollectDelay" => "nocollectdelay",
            "Base" => "base",
            "Size" => "size",
            "OwnMass" => "ownmass",
            "Mass" => "mass",
            "Damage" => "damage",
            "Energy" => "energy",
            "MagicEnergy" => "magicenergy",
            "Alive" => "alive",
            "Breath" => "breath",
            "FirePhase" => "firephase",
            "Color" => "color",
            "ColorDw" => "colordw",
            "Locals" => "locals",
            "FixX" => "fixx",
            "FixY" => "fixy",
            "FixR" => "fixr",
            "XDir" => "xdir",
            "YDir" => "ydir",
            "RDir" => "rdir",
            "Width" => "width",
            "Height" => "height",
            "Offset" => "offset",
            "Vertices" => "vertices",
            "VertexX" => "vertexx",
            "VertexY" => "vertexy",
            "VertexCNAT" => "vertexcnat",
            "VertexFriction" => "vertexfriction",
            "ContactDensity" => "contactdensity",
            "FireTop" => "firetop",
            "AttachX" => "attachx",
            "AttachY" => "attachy",
            "AttachVtx" => "attachvtx",
            "OwnVertices" => "ownvertices",
            "SolidMask" => "solidmask",
            "Picture" => "picture",
            "Mobile" => "mobile",
            "Selected" => "selected",
            "OnFire" => "onfire",
            "InLiquid" => "inliquid",
            "EntranceStatus" => "entrancestatus",
            "PhysicalTemporary" => "physicaltemporary",
            "NeedEnergy" => "needenergy",
            "OCF" => "ocf",
            "Action" => "action",
            "Dir" => "dir",
            "ComDir" => "comdir",
            "ActionTime" => "actiontime",
            "ActionData" => "actiondata",
            "Phase" => "phase",
            "PhaseDelay" => "phasedelay",
            "Contained" => "contained",
            "ActionTarget1" => "actiontarget1",
            "ActionTarget2" => "actiontarget2",
            "Component" => "component",
            "Contents" => "contents",
            "PlrViewRange" => "plrviewrange",
            "Visibility" => "visibility",
            "LocalNamed" => "localnamed",
            "ColorMod" => "colormod",
            "BlitMode" => "blitmode",
            "CrewDisabled" => "crewdisabled",
            "Layer" => "layer",
            "Graphics" => "graphics",
            "DrawTransform" => "drawtransform",
            "Effects" => "effects",
            "GfxOverlay" => "gfxoverlay",
            _ => return Ok(()),
        };
        let trimmed_value = value.trim();
        match normalized_key {
            "id" => {
                self.id = Some(trimmed_value.to_string());
            }
            "number" => {
                let number = parse_i64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Number `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                if number < 0 {
                    return Err(ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: Number must be >= 0 (got {})",
                        self.line, number
                    )));
                }
                self.number = Some(number as u64);
            }
            "name" => {
                self.custom_name = parse_legacy_object_name(trimmed_value, self.line)?;
            }
            "info" => {
                // nInfo is compiled through RCT_All: leading horizontal
                // whitespace is skipped, but the remainder of the physical
                // line (including `//` and trailing spaces) is data.
                let whole_line = value.trim_start_matches([' ', '\t']);
                self.info_name = (!whole_line.is_empty()).then(|| whole_line.to_string());
            }
            "status" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Status `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.status = Some(ObjectStatus::from_script_value(raw).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: unsupported Status value {}",
                        self.line, raw
                    ))
                })?);
            }
            "owner" => {
                let owner = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Owner `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.owner = Some(owner);
            }
            "controller" => {
                let controller = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Controller `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.controller = Some(controller);
            }
            "lastenglossplr" => {
                self.last_energy_loss_cause = Some(parse_object_i32(
                    trimmed_value,
                    self.line,
                    "LastEngLossPlr",
                )?);
            }
            "x" => {
                self.x = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid X `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "y" => {
                self.y = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Y `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "motionx" => {
                self.motion_x = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid MotionX `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "motiony" => {
                self.motion_y = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid MotionY `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "lastsolidatchframe" => {
                self.last_attach_movement_frame = Some(parse_object_i32(
                    trimmed_value,
                    self.line,
                    "LastSolidAtchFrame",
                )?);
            }
            "nocollectdelay" => {
                self.no_collect_delay = Some(parse_object_i32(
                    trimmed_value,
                    self.line,
                    "NoCollectDelay",
                )?);
            }
            "base" => {
                self.base = Some(parse_object_i32(trimmed_value, self.line, "Base")?);
            }
            "xdir" => {
                self.xdir = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid XDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "ydir" => {
                self.ydir = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid YDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "fixx" => {
                self.fix_x = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FixX `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "fixy" => {
                self.fix_y = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FixY `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "fixr" => {
                self.fix_r = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FixR `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "rdir" => {
                self.rdir = Some(parse_c4fixed(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid RDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "mobile" => {
                let mobile = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Mobile `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.mobile = Some(mobile);
            }
            "solidmask" => {
                // C4Object::CompileFunc SolidMask (default Def->SolidMask,
                // C4Object.cpp:2770): six ints; 0,0,0,0,0,0 = mask OFF.
                self.solid_mask = Some(parse_i32_list(trimmed_value, self.line, "SolidMask")?);
            }
            "rotation" => {
                self.rotation = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Rotation `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "timer" => {
                self.timer = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Timer `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "locals" => {
                self.locals = Some(parse_local_slots(trimmed_value, self.line)?);
            }
            "localnamed" => {
                self.local_named = Some(parse_local_named(trimmed_value, self.line)?);
            }
            "width" => {
                self.shape_width = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Width `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "height" => {
                self.shape_height = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Height `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "offset" => {
                self.shape_offset = Some(parse_i32_list(trimmed_value, self.line, "Offset")?);
            }
            "vertices" => {
                self.vertex_count = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Vertices `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "vertexx" => {
                self.vertex_x = Some(parse_i32_list(trimmed_value, self.line, "VertexX")?);
            }
            "vertexy" => {
                self.vertex_y = Some(parse_i32_list(trimmed_value, self.line, "VertexY")?);
            }
            "vertexcnat" => {
                self.vertex_cnat = Some(parse_i32_list(trimmed_value, self.line, "VertexCNAT")?);
            }
            "vertexfriction" => {
                self.vertex_friction =
                    Some(parse_i32_list(trimmed_value, self.line, "VertexFriction")?);
            }
            "ownvertices" => {
                let own_vertices = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid OwnVertices `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.own_vertices = Some(own_vertices);
            }
            "contactdensity" => {
                self.contact_density = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ContactDensity `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "firetop" => {
                self.shape_fire_top = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid FireTop `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "attachx" => {
                self.shape_attach_x = Some(parse_object_i32(trimmed_value, self.line, "AttachX")?);
            }
            "attachy" => {
                self.shape_attach_y = Some(parse_object_i32(trimmed_value, self.line, "AttachY")?);
            }
            "attachvtx" => {
                self.shape_attach_vertex =
                    Some(parse_object_i32(trimmed_value, self.line, "AttachVtx")?);
            }
            "ownmass" => {
                self.own_mass = Some(parse_object_i32(trimmed_value, self.line, "OwnMass")?);
            }
            "mass" => {
                self.mass = Some(parse_object_i32(trimmed_value, self.line, "Mass")?);
            }
            "damage" => {
                self.damage = Some(parse_object_i32(trimmed_value, self.line, "Damage")?);
            }
            "energy" => {
                self.energy = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Energy `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "breath" => {
                self.breath = Some(parse_object_i32(trimmed_value, self.line, "Breath")?);
            }
            "firephase" => {
                self.fire_phase = Some(parse_object_i32(trimmed_value, self.line, "FirePhase")?);
            }
            "needenergy" => {
                let need_energy = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid NeedEnergy `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.need_energy = Some(need_energy);
            }
            "selected" => {
                let selected = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Selected `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.selected = Some(selected);
            }
            "onfire" => {
                self.on_fire = Some(parse_object_bool(trimmed_value, self.line, "OnFire")?);
            }
            // C4Object::MagicEnergy compiles verbatim with default 0
            // (C4Object.cpp:2768) — Drachenfels' wizards carry it.
            "magicenergy" => {
                self.magic_energy = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid MagicEnergy `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            // C++ saves Con under the key "Size" (C4Object::CompileFunc,
            // C4Object.cpp:2763); the GoldRush bushes carry Size=25610
            // and grow toward FullCon from there.
            "size" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Con `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                let raw = if value > 1000 {
                    value
                } else {
                    (value.clamp(0, 100) * FULL_CON) / 100
                };
                self.construction = Some(raw.max(0));
            }
            "alive" => {
                let alive = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Alive `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.alive = Some(alive);
            }
            // C4Object::InLiquid, persisted with default false
            // (C4Object.cpp:2775) — GoldRush carries InLiquid=1 on its
            // underwater fish and bubbles.
            "inliquid" => {
                let in_liquid = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid InLiquid `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.in_liquid = Some(in_liquid);
            }
            "entrancestatus" => {
                let entrance_status = parse_bool(trimmed_value).ok_or_else(|| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid EntranceStatus `{}`",
                        self.line, trimmed_value
                    ))
                })?;
                self.entrance_status = Some(entrance_status);
            }
            "physicaltemporary" => {
                if self.physical_temporary.is_none() {
                    self.physical_temporary = Some(parse_object_compiler_bool(value));
                }
            }
            "ocf" => {
                self.ocf = Some(parse_object_u32(trimmed_value, self.line, "OCF")?);
            }
            "category" => {
                self.category = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Category `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "dir" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Dir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                // C4Action::CompileFunc persists Dir verbatim without action-
                // range validation (C4Action.cpp:45-54).
                self.direction = Some(Direction::from_raw(raw));
            }
            "comdir" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ComDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                // C4Action::CompileFunc persists ComDir verbatim without
                // COMD_* range validation (C4Action.cpp:45-54).
                self.command_direction = Some(CommandDirection::from_raw(raw));
            }
            "action" => {
                self.action_name = Some(trimmed_value.to_string());
            }
            "actiontime" => {
                let ticks = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTime `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.action_ticks = Some(ticks);
            }
            "phasedelay" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid PhaseDelay `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.action_phase_delay = Some(value);
            }
            "actiondata" => {
                self.action_data = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionData `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "phase" => {
                self.action_phase = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Phase `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "actiontarget1" => {
                self.action_target = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTarget1 `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "actiontarget2" => {
                self.action_target2 = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTarget2 `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "layer" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Layer `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.layer = Some(value);
            }
            "visibility" => {
                self.visibility = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Visibility `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "blitmode" => {
                self.blit_mode = Some(parse_object_u32(trimmed_value, self.line, "BlitMode")?);
            }
            "color" | "colordw" => {
                self.color = Some(parse_object_u32(trimmed_value, self.line, "ColorDw")?);
            }
            "colormod" => {
                self.color_modulation =
                    Some(parse_object_u32(trimmed_value, self.line, "ColorMod")?);
            }
            "picture" => {
                let values = parse_i32_list(trimmed_value, self.line, "Picture")?;
                if values.len() != 4 {
                    return Err(ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: Picture requires 4 integers (got {})",
                        self.line,
                        values.len()
                    )));
                }
                self.picture_rect = Some(DefinitionRect::new(
                    values[0], values[1], values[2], values[3],
                ));
            }
            "plrviewrange" => {
                self.plr_view_range =
                    Some(parse_object_i32(trimmed_value, self.line, "PlrViewRange")?);
            }
            "crewdisabled" => {
                self.crew_disabled =
                    Some(parse_object_bool(trimmed_value, self.line, "CrewDisabled")?);
            }
            "graphics" => {
                self.base_graphics = Some(parse_legacy_object_graphics(
                    trimmed_value,
                    self.line,
                    "Graphics",
                )?);
            }
            "drawtransform" => {
                self.draw_transform = Some(parse_legacy_draw_transform(
                    trimmed_value,
                    self.line,
                    "DrawTransform",
                )?);
            }
            "effects" => {
                self.effects = Some(parse_legacy_object_effects(trimmed_value, self.line)?);
            }
            "gfxoverlay" => {
                self.graphics_overlays =
                    Some(parse_legacy_graphics_overlays(trimmed_value, self.line)?);
            }
            "component" => {
                self.components = Some(parse_legacy_object_components(trimmed_value, self.line)?);
            }
            "contained" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Contained `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.contained = Some(value);
            }
            "contents" => {
                let mut entries = Vec::new();
                for token in trimmed_value.split(';') {
                    let candidate = token.trim();
                    if candidate.is_empty() {
                        continue;
                    }
                    let value = parse_i64(candidate).map_err(|err| {
                        ScenarioError::LegacyObjectsParse(format!(
                            "Objects.txt line {}: invalid Contents entry `{}` ({})",
                            self.line, candidate, err
                        ))
                    })?;
                    if value > 0 {
                        entries.push(value as u64);
                    }
                }
                self.contents = entries;
            }
            _ => {}
        }
        Ok(())
    }

    fn begin_physical_section(&mut self) {
        if self.physical_temporary == Some(true) {
            self.temporary_physical
                .get_or_insert_with(crate::PhysicalInfo::default);
        }
    }

    fn apply_physical_property(
        &mut self,
        key: &str,
        value: &str,
        _line: usize,
    ) -> Result<(), ScenarioError> {
        // C4Object::CompileFunc never follows this sibling section while the
        // flag is false or absent. Its contents are unused namings, including
        // malformed values, rather than parse errors.
        if self.physical_temporary != Some(true) {
            return Ok(());
        }
        if key != "Changes" && !is_legacy_physical_name(key) {
            return Ok(());
        }
        if !self.physical_fields_seen.insert(key.to_string()) {
            return Ok(());
        }
        if key == "Changes" {
            self.physical_changes = parse_legacy_physical_changes(value);
            return Ok(());
        }

        let physical = self
            .temporary_physical
            .get_or_insert_with(crate::PhysicalInfo::default);
        let parsed = match key {
            "Energy" | "Breath" | "Walk" | "Jump" | "Scale" | "Hangle" | "Dig" | "Swim"
            | "Throw" | "Push" | "Fight" | "Magic" | "Float" | "CanScale" | "CanHangle"
            | "CanDig" | "CanConstruct" | "CanChop" | "CanFly" | "CorrosionResist"
            | "BreatheWater" => {
                // Every physical field is wrapped in mkNamingAdapt(..., 0).
                // A malformed first naming is consumed and defaults to zero.
                parse_std_i32(value).unwrap_or_default()
            }
            _ => unreachable!("unknown physical names returned before parsing"),
        };
        match key {
            "Energy" => physical.energy = parsed,
            "Breath" => physical.breath = parsed,
            "Walk" => physical.walk = parsed,
            "Jump" => physical.jump = parsed,
            "Scale" => physical.scale = parsed,
            "Hangle" => physical.hangle = parsed,
            "Dig" => physical.dig = parsed,
            "Swim" => physical.swim = parsed,
            "Throw" => physical.throw = parsed,
            "Push" => physical.push = parsed,
            "Fight" => physical.fight = parsed,
            "Magic" => physical.magic = parsed,
            "Float" => physical.float = parsed,
            "CanScale" => physical.can_scale = parsed,
            "CanHangle" => physical.can_hangle = parsed,
            "CanDig" => physical.can_dig = parsed,
            "CanConstruct" => physical.can_construct = parsed,
            "CanChop" => physical.can_chop = parsed,
            "CanFly" => physical.can_fly = parsed,
            "CorrosionResist" => physical.corrosion_resist = parsed,
            "BreatheWater" => physical.breathe_water = parsed,
            _ => unreachable!("all recognized physical names are assigned"),
        }
        Ok(())
    }

    fn apply_command_property(
        &mut self,
        key: &str,
        value: &str,
        line: usize,
    ) -> Result<(), ScenarioError> {
        let Some(index) = key.strip_prefix("Command") else {
            return Ok(());
        };
        let Ok(index) = index.parse::<usize>() else {
            return Ok(());
        };
        if index == 0 || key != format!("Command{index}") {
            return Ok(());
        }
        let command = parse_legacy_object_command(value, line)?;
        self.commands.insert(index, command);
        Ok(())
    }

    fn into_spawn(
        self,
        definition_ids: &HashSet<&str>,
        value_resolution: &SerializedC4ValueResolution<'_>,
    ) -> Result<Option<ScenarioSpawn>, ScenarioError> {
        let Self {
            line,
            id,
            number,
            custom_name,
            info_name,
            status,
            owner,
            controller,
            last_energy_loss_cause,
            x,
            y,
            motion_x,
            motion_y,
            last_attach_movement_frame,
            no_collect_delay,
            base,
            xdir,
            ydir,
            fix_x,
            fix_y,
            fix_r,
            rdir,
            mobile,
            solid_mask,
            rotation,
            timer,
            locals,
            local_named,
            vertex_count,
            vertex_x,
            vertex_y,
            vertex_cnat,
            vertex_friction,
            shape_width,
            shape_height,
            shape_offset,
            shape_fire_top,
            own_vertices,
            contact_density,
            shape_attach_x,
            shape_attach_y,
            shape_attach_vertex,
            own_mass,
            mass,
            damage,
            energy,
            need_energy,
            selected,
            magic_energy,
            construction,
            alive,
            breath,
            fire_phase,
            on_fire,
            in_liquid,
            entrance_status,
            physical_temporary,
            ocf,
            category,
            direction,
            command_direction,
            action_name,
            action_phase,
            action_ticks,
            action_phase_delay,
            action_data,
            action_target,
            action_target2,
            layer,
            visibility,
            blit_mode,
            color,
            color_modulation,
            picture_rect,
            plr_view_range,
            crew_disabled,
            base_graphics,
            draw_transform,
            effects,
            graphics_overlays,
            temporary_physical,
            physical_changes,
            physical_fields_seen: _,
            commands,
            components,
            contained,
            inferred_container,
            contents,
        } = self;

        let id = id.ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: object missing `id`",
                line
            ))
        })?;

        if !definition_ids.contains(id.as_str()) {
            // C++ resolves each Objects.txt entry with C4Id2Def: an unknown
            // id produces no object (logged) and the load continues.
            tracing::warn!(
                definition = %id,
                line,
                "Objects.txt references an unknown definition; skipping the object"
            );
            return Ok(None);
        }

        let number = number.ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: object `{}` missing `Number`",
                line, id
            ))
        })?;

        if matches!(status, Some(ObjectStatus::Deleted)) {
            return Ok(None);
        }

        let mut config = SpawnConfig::new(id.clone())
            .with_id(ObjectId::new(number))
            // Objects.txt entries are LOADED, not created: no
            // Construction/Initialize (C4GameObjects.cpp:535-618).
            .with_loaded(true)
            .with_native_compiled_object_defaults();
        config.compiler_cache = crate::ObjectCompilerCache {
            info: info_name.clone().unwrap_or_default(),
            contained: contained.unwrap_or(0),
            action_target1: action_target.unwrap_or(0),
            action_target2: action_target2.unwrap_or(0),
            layer: layer.unwrap_or(0),
        };
        let offset = shape_offset.unwrap_or_default();
        config = config
            .with_shape_rect(crate::DefinitionRect::new(
                offset.first().copied().unwrap_or(0),
                offset.get(1).copied().unwrap_or(0),
                shape_width.unwrap_or(0),
                shape_height.unwrap_or(0),
            ))
            .with_shape_fire_top(shape_fire_top.unwrap_or(0));
        config = config.with_position(Vector2::new(x.unwrap_or(0), y.unwrap_or(0)));
        config.motion_x = motion_x.unwrap_or(0);
        config.motion_y = motion_y.unwrap_or(0);
        if let Some(custom_name) = custom_name {
            config = config.with_custom_name(custom_name);
        }
        if let Some(layer) = layer
            .and_then(|layer| u64::try_from(layer).ok())
            .filter(|layer| *layer != 0)
        {
            config = config.with_layer(ObjectId::new(layer));
        }
        if let Some(visibility) = visibility {
            config = config.with_visibility(visibility);
        }
        if let Some(blit_mode) = blit_mode {
            config = config.with_blit_mode(blit_mode);
        }
        if let Some(color) = color {
            config = config.with_color(color);
        }
        if let Some(color_modulation) = color_modulation {
            config = config.with_color_modulation(color_modulation);
        }
        if let Some(picture_rect) = picture_rect {
            config = config.with_picture_rect(picture_rect);
        }
        if let Some(components) = components {
            config = config.with_ordered_components(components);
        }
        if let Some(contact_density) = contact_density {
            config = config.with_contact_density(contact_density);
        }
        config.damage = damage;
        config.breath = breath;
        config.own_mass = own_mass;
        config.compiled_mass = mass;
        config.on_fire = on_fire;
        config.fire_phase = fire_phase;
        config.last_attach_movement_frame = last_attach_movement_frame;
        config.last_energy_loss_cause = last_energy_loss_cause;
        config.no_collect_delay = no_collect_delay;
        config.base = base;
        config.compiled_ocf = ocf;
        config.crew_disabled = crew_disabled;
        config.plr_view_range = plr_view_range;
        config.base_graphics = base_graphics;
        config.draw_transform = draw_transform;
        config.graphics_overlays = graphics_overlays
            .unwrap_or_default()
            .into_iter()
            .map(|overlay| overlay.resolve(value_resolution.object_numbers))
            .collect();
        if shape_attach_x.is_some() || shape_attach_y.is_some() || shape_attach_vertex.is_some() {
            config.shape_attach = Some(crate::ShapeAttachRecord {
                // AttachMat is deliberately not compiled by C4Shape.
                mat_valid: false,
                mat_vehicle: false,
                x: shape_attach_x.unwrap_or(0),
                y: shape_attach_y.unwrap_or(0),
                vtx: shape_attach_vertex.unwrap_or(0),
            });
        }

        let resolved_effects = effects
            .unwrap_or_default()
            .into_iter()
            .map(|effect| effect.resolve(value_resolution))
            .collect::<Vec<_>>();
        config.fire_caused_by = Some(
            resolved_effects
                .iter()
                .find(|effect| effect.name == crate::C4FX_FIRE)
                .and_then(|effect| effect.vars.get(1))
                .and_then(|value| match value {
                    EffectVarValue::Int(value) => Some(*value),
                    EffectVarValue::Bool(value) => Some(i32::from(*value)),
                    EffectVarValue::RawBool(value) => Some(*value as u32 as i32),
                    _ => None,
                })
                .unwrap_or(crate::OWNER_NONE),
        );
        config.effects = resolved_effects;

        if physical_temporary.unwrap_or(false) {
            config.temporary_physical = Some(temporary_physical.unwrap_or_default());
            config.physical_changes = physical_changes;
        }

        let mut commands = commands.into_iter().peekable();
        let mut expected_command = 1usize;
        let mut resolved_commands = Vec::new();
        while commands
            .peek()
            .is_some_and(|(index, _)| *index == expected_command)
        {
            let (_, command) = commands.next().expect("peeked command exists");
            resolved_commands.push(command.resolve(line, value_resolution)?);
            expected_command += 1;
        }
        if !resolved_commands.is_empty() {
            config.command_stack = Some(
                crate::command::CommandStackSnapshot::from_legacy_save_commands(resolved_commands)
                    .map_err(|error| {
                        ScenarioError::LegacyObjectsParse(format!(
                            "Objects.txt line {line}: invalid [Commands] stack ({error:?})"
                        ))
                    })?,
            );
        }

        if xdir.is_some() || ydir.is_some() {
            // Exact C4Fixed velocity (C4Object.cpp:2765-2766); the pixel
            // mirror follows fixtoi like C4Object::velocity_pixels.
            let fixed = crate::math::FixedVec2 {
                x: xdir.unwrap_or_default(),
                y: ydir.unwrap_or_default(),
            };
            config = config
                .with_velocity(Vector2::new(
                    crate::math::fixtoi(fixed.x),
                    crate::math::fixtoi(fixed.y),
                ))
                .with_fixed_velocity(fixed);
        }
        // Exact sub-pixel position (FixX/FixY, C4Object.cpp:2762-2763).
        // C++ keeps integer X/Y and fixed coords independent after load;
        // each missing naming value compiles as Fix0. Supplying the zero pair
        // is observable for inactive rows, which never receive SyncClearance.
        config = config.with_fixed_position(crate::math::FixedVec2 {
            x: fix_x.unwrap_or_default(),
            y: fix_y.unwrap_or_default(),
        });
        if let Some(rotation) = rotation {
            config = config.with_rotation(rotation);
        }
        if let Some(fix_r) = fix_r {
            config = config.with_fixed_rotation(fix_r);
        }
        if let Some(rdir) = rdir {
            config = config.with_rotation_velocity(rdir);
        }
        // Loaded objects keep the serialized Mobile verbatim (default
        // false) — they bypass Init, and nothing after C4GameObjects::Load
        // rewrites the flag (C4Object.cpp:2772).
        config = config.with_mobile(mobile.unwrap_or(false));
        if let Some(timer) = timer {
            config = config.with_timer(timer);
        }
        let mut local_vars = HashMap::new();
        if let Some(locals) = locals {
            // C4ValueList slots and named locals are denumerated only after
            // every object exists (C4GameObjects.cpp:600-608).
            local_vars.extend(locals.into_iter().enumerate().map(|(index, value)| {
                (format!("__local_{index}"), value.resolve(value_resolution))
            }));
        }
        if let Some(local_named) = local_named {
            local_vars.extend(
                local_named
                    .into_iter()
                    .map(|(name, value)| (name, value.resolve(value_resolution))),
            );
        }
        if !local_vars.is_empty() {
            config = config.with_local_vars(local_vars);
        }
        // The saved shape's vertices (C4Shape::CompileFunc into [Object],
        // C4Shape.cpp:495-515): the CURRENT effective shape, loaded
        // verbatim (spawn_single skips the Con/rotation re-transform for
        // loaded vertices). Missing arrays read as 0 (mkArrayAdapt).
        // C4Object::Clear zeroes the complete shape before Objects.txt is
        // compiled, so a missing `Vertices` is an explicit VtxNum=0 rather
        // than "fall back to the definition". mkArrayAdapt independently
        // compiles all 30 slots and may retain nonzero dormant values beyond
        // VtxNum (notably own-vertex backups at slots 15+).
        let vertex_count = vertex_count.unwrap_or(0).clamp(0, 30) as usize;
        let component = |list: &Option<Vec<i32>>, index: usize| {
            list.as_ref()
                .and_then(|values| values.get(index).copied())
                .unwrap_or(0)
        };
        let vertex_slots: Vec<crate::ObjectVertex> = (0..30)
            .map(|index| {
                crate::ObjectVertex::new(component(&vertex_x, index), component(&vertex_y, index))
                    .with_cnat(component(&vertex_cnat, index) as u32)
                    .with_friction(component(&vertex_friction, index))
            })
            .collect();
        config = config
            .with_vertices(vertex_slots[..vertex_count].to_vec())
            .with_shape_vertex_slots(vertex_count, vertex_slots);
        if let Some(own_vertices) = own_vertices {
            config = config.with_owns_shape_vertices(own_vertices);
        }
        if let Some(owner) = owner {
            config = config.with_owner(owner);
        }
        if let Some(controller) = controller {
            config = config.with_controller(controller);
        }
        if let Some(energy) = energy {
            config = config.with_energy(energy);
        }
        if let Some(need_energy) = need_energy {
            config = config.with_need_energy(need_energy);
        }
        if let Some(selected) = selected {
            config = config.with_selected(selected);
        }
        if let Some(magic_energy) = magic_energy {
            config = config.with_magic_energy(magic_energy);
        }
        // C4Object::Clear initializes Con to zero before compilation;
        // Objects.txt omits Size only when that exact zero is intended.
        config = config.with_construction(construction.unwrap_or(0));
        if let Some(alive) = alive {
            config = config.with_alive(alive);
        }
        if let Some(in_liquid) = in_liquid {
            config = config.with_in_liquid(in_liquid);
        }
        if let Some(entrance_status) = entrance_status {
            config = config.with_entrance_status(entrance_status);
        }
        if let Some(category) = category {
            config = config.with_category(category);
        }
        if let Some(status) = status {
            if status != ObjectStatus::Normal {
                config = config.with_status(status);
            }
        }
        if let Some(values) = solid_mask {
            let mut it = values.into_iter().chain(std::iter::repeat(0));
            let rect = crate::DefinitionTargetRect::new(
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
                it.next().unwrap_or(0),
            );
            config = config.with_solid_mask(rect);
        }
        if let Some(direction) = direction {
            config = config.with_direction(direction);
        }
        if let Some(command_direction) = command_direction {
            config = config.with_command_direction(command_direction);
        }
        if let Some(action_state) = build_action_state(
            action_name,
            action_phase,
            action_ticks,
            action_phase_delay,
            action_data,
            action_target,
            action_target2,
        ) {
            config = config.with_action(action_state);
        }

        let container_handle = contained
            .map(|number| {
                if (1_000_000_000..=1_001_000_000).contains(&number) {
                    number - 1_000_000_000
                } else {
                    number
                }
            })
            .filter(|number| *number > 0)
            .map(|number| number.to_string())
            .or_else(|| inferred_container.map(|number| number.to_string()));
        Ok(Some(ScenarioSpawn {
            handle: Some(number.to_string()),
            container_handle,
            contents_handles: contents
                .into_iter()
                .map(|value| value.to_string())
                .collect(),
            info_name,
            config,
        }))
    }
}

fn build_action_state(
    name: Option<String>,
    phase: Option<i32>,
    time: Option<i32>,
    phase_delay: Option<i32>,
    data: Option<i32>,
    target: Option<i32>,
    target2: Option<i32>,
) -> Option<ActionState> {
    if name.is_none()
        && phase.is_none()
        && time.is_none()
        && phase_delay.is_none()
        && data.is_none()
        && target.is_none()
        && target2.is_none()
    {
        return None;
    }
    // C4Action::CompileFunc compiles every field independently. A save may
    // carry ActionTarget1/2 without an explicit Action name; its zeroed
    // fixed-size Name buffer is an empty string. SetActionByName("") then
    // fails, preserving the saved fixed coordinates, while the pointers
    // still proceed through DenumeratePointers.
    // `C4Action::Name` is a `C4MaxName + 1` fixed buffer compiled through
    // `toC4CStr` (C4Action.cpp:45-54). Both lookup and a failed lookup's
    // observable raw name therefore see at most 30 native bytes, not 30
    // Unicode scalar values. Round-trip through the C4 byte projection so
    // legacy high bytes are neither split nor counted as UTF-8 characters.
    let name = name.unwrap_or_default();
    let name_bytes = clonk_script::c4_string_bytes(&name);
    let visible_len = name_bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(name_bytes.len())
        .min(30);
    let name = clonk_script::c4_string_from_bytes(&name_bytes[..visible_len]);
    let name = if is_builtin_idle_name(&name) {
        "Idle".to_string()
    } else {
        name
    };
    let mut state = ActionState::new(name);
    if let Some(value) = phase {
        state.phase = value;
    }
    // ActionTime= is Action.Time; PhaseDelay= is the intra-phase counter
    // (C4Object.cpp:2840-2849 restores Time/Phase/PhaseDelay verbatim).
    if let Some(value) = time {
        state.time = value;
    }
    if let Some(value) = phase_delay {
        state.ticks = value;
    }
    if let Some(value) = data {
        state.data = value;
    }
    if let Some(target) = target.and_then(|target| u64::try_from(target).ok()) {
        state.target = Some(ObjectId::new(target));
    }
    if let Some(target2) = target2.and_then(|target| u64::try_from(target).ok()) {
        state.target2 = Some(ObjectId::new(target2));
    }
    Some(state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyObjectParseSection {
    Object,
    Physical,
    Commands,
    Other,
}

fn parse_legacy_ini_section_name(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b'[') || !bytes.get(1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut position = 1usize;
    while bytes
        .get(position)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
    {
        position += 1;
    }
    let name_end = position;
    while bytes
        .get(position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        position += 1;
    }
    (bytes.get(position) == Some(&b']')).then(|| &line[1..name_end])
}

fn parse_legacy_ini_property(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut position = 0usize;
    while bytes
        .get(position)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
    {
        position += 1;
    }
    let name_end = position;
    while bytes
        .get(position)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        position += 1;
    }
    if bytes.get(position) != Some(&b'=') {
        return None;
    }
    Some((&line[..name_end], &line[position + 1..]))
}

fn parse_legacy_objects(text: &str) -> Result<Vec<LegacyObjectRecord>, ScenarioError> {
    let mut records = Vec::new();
    let mut current: Option<LegacyObjectRecord> = None;
    let mut section_stack: Vec<(usize, LegacyObjectParseSection)> = Vec::new();
    let mut object_indent = None;
    // FollowName("Physical") only sees the next sibling of [Object]. A child
    // section does not consume that position, but a same-level (or outer)
    // section does, even when its name is otherwise unknown.
    let mut physical_may_follow_object = false;

    for (index, raw_line) in text.lines().enumerate() {
        // StdCompilerINIRead does not have an inline-comment syntax. In
        // particular, `//` inside RCT_All values (Info and command Text) is
        // ordinary persisted data. Retain the right-hand end of every line.
        let raw_line = raw_line.trim_start_matches('\u{feff}');
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let line = raw_line.trim_start_matches([' ', '\t']);
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("//") || line.starts_with(';') {
            continue;
        }
        if let Some(section_name) = parse_legacy_ini_section_name(line) {
            while section_stack
                .last()
                .is_some_and(|(section_indent, _)| *section_indent >= indent)
            {
                section_stack.pop();
            }
            let has_parent_section = !section_stack.is_empty();
            // Only [Object] creates a row. Nested naming environments belong
            // to that row and must route their properties to their own
            // compiler instead of falling through to C4Object::CompileFunc.
            let parsed_section = if section_name == "Object" && !has_parent_section {
                if let Some(record) = current.take() {
                    records.push(record);
                }
                current = Some(LegacyObjectRecord::new(index + 1));
                object_indent = Some(indent);
                physical_may_follow_object = true;
                LegacyObjectParseSection::Object
            } else if section_name == "Physical"
                && current.is_some()
                && !has_parent_section
                && physical_may_follow_object
            {
                if let Some(record) = current.as_mut() {
                    record.begin_physical_section();
                }
                physical_may_follow_object = false;
                LegacyObjectParseSection::Physical
            } else if section_name == "Commands" {
                if object_indent.is_some_and(|object_indent| indent <= object_indent) {
                    physical_may_follow_object = false;
                }
                LegacyObjectParseSection::Commands
            } else {
                if object_indent.is_some_and(|object_indent| indent <= object_indent) {
                    physical_may_follow_object = false;
                }
                LegacyObjectParseSection::Other
            };
            section_stack.push((indent, parsed_section));
            continue;
        }
        let Some((key, value)) = parse_legacy_ini_property(line) else {
            continue;
        };

        // Native INI values receive one implicit indentation level. Pop any
        // child sections the value has left, revealing its enclosing naming.
        while section_stack
            .last()
            .is_some_and(|(section_indent, _)| *section_indent > indent)
        {
            section_stack.pop();
        }
        let section = section_stack
            .last()
            .map_or(LegacyObjectParseSection::Other, |(_, section)| *section);
        if section == LegacyObjectParseSection::Other
            && object_indent.is_some_and(|object_indent| indent < object_indent)
        {
            physical_may_follow_object = false;
        }
        match section {
            LegacyObjectParseSection::Object => {
                let record = current.as_mut().expect("Object section creates a record");
                record.apply_property(key, value)?;
            }
            LegacyObjectParseSection::Physical => {
                if let Some(record) = current.as_mut() {
                    record.apply_physical_property(key, value, index + 1)?;
                }
            }
            LegacyObjectParseSection::Commands => {
                if let Some(record) = current.as_mut() {
                    record.apply_command_property(key, value, index + 1)?;
                }
            }
            LegacyObjectParseSection::Other => {}
        }
    }

    if let Some(record) = current.take() {
        records.push(record);
    }

    Ok(records)
}

fn object_property_error(line: usize, key: &str, value: &str, detail: &str) -> ScenarioError {
    ScenarioError::LegacyObjectsParse(format!(
        "Objects.txt line {line}: invalid {key} `{value}` ({detail})"
    ))
}

fn parse_object_i32(value: &str, line: usize, key: &str) -> Result<i32, ScenarioError> {
    parse_i32(value).map_err(|error| object_property_error(line, key, value, &error))
}

fn parse_object_u32(value: &str, line: usize, key: &str) -> Result<u32, ScenarioError> {
    // StdCompilerINIRead reads unsigned fields through strtoul and then
    // stores the low uint32 word. In particular, older Objects.txt files
    // spell high-bit OCF/colour values as signed decimal numbers. Preserve
    // those bits instead of rejecting the leading minus sign.
    parse_std_u32(value)
        .ok_or_else(|| object_property_error(line, key, value, "invalid uint32 value"))
}

fn parse_object_bool(value: &str, line: usize, key: &str) -> Result<bool, ScenarioError> {
    parse_bool(value)
        .ok_or_else(|| object_property_error(line, key, value, "expected a boolean value"))
}

/// StdCompilerINIRead::Boolean reads directly after `=` without skipping
/// whitespace. It accepts the exact lowercase prefixes `true` and `false`, or
/// a leading 0/1 not followed by another digit. Invalid input is caught by the
/// surrounding default adaptor and becomes false.
fn parse_object_compiler_bool(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'1') && !bytes.get(1).is_some_and(u8::is_ascii_digit) {
        true
    } else if bytes.first() == Some(&b'0') && !bytes.get(1).is_some_and(u8::is_ascii_digit) {
        false
    } else {
        value.starts_with("true")
    }
}

fn parse_legacy_object_graphics(
    value: &str,
    line: usize,
    key: &str,
) -> Result<crate::ObjectBaseGraphics, ScenarioError> {
    let Some((definition, graphics_name)) = value.split_once("::") else {
        return Err(object_property_error(
            line,
            key,
            value,
            "expected DEFN::GraphicsName",
        ));
    };
    let definition = definition.trim();
    if clonk_script::c4_string_bytes(definition).len() != 4
        || clonk_script::c4_id_raw(definition) == 0
    {
        return Err(object_property_error(
            line,
            key,
            value,
            "definition id must contain exactly four native C4 bytes",
        ));
    }
    let graphics_name = graphics_name.trim();
    Ok(crate::ObjectBaseGraphics {
        definition: definition.to_string(),
        graphics_name: (!graphics_name.is_empty()).then(|| graphics_name.to_string()),
        // C4DefGraphicsAdapt contains only the definition/name pair. The
        // object's independent BlitMode field is compiled elsewhere.
        blit_mode: 0,
    })
}

fn parse_legacy_draw_transform(
    value: &str,
    line: usize,
    key: &str,
) -> Result<crate::DrawTransform, ScenarioError> {
    let fields = split_outside_delimiter(value, ',');
    if !(7..=10).contains(&fields.len()) {
        return Err(object_property_error(
            line,
            key,
            value,
            &format!(
                "expected six affine values, FlipDir, and up to three projective values; found {} fields",
                fields.len()
            ),
        ));
    }
    let mut matrix = [0.0_f32; 9];
    matrix[8] = 1.0;
    for (index, field) in fields.iter().take(6).enumerate() {
        matrix[index] = field.trim().parse::<f32>().map_err(|error| {
            object_property_error(
                line,
                key,
                value,
                &format!("invalid matrix component {}: {error}", index + 1),
            )
        })?;
    }
    let flip_dir = parse_object_i32(fields[6].trim(), line, key)?;
    for (offset, field) in fields.iter().skip(7).enumerate() {
        matrix[6 + offset] = field.trim().parse::<f32>().map_err(|error| {
            object_property_error(
                line,
                key,
                value,
                &format!("invalid projective component {}: {error}", offset + 1),
            )
        })?;
    }
    Ok(crate::DrawTransform::from_matrix_with_flip_dir(
        matrix, flip_dir,
    ))
}

fn parse_legacy_graphics_overlays(
    value: &str,
    line: usize,
) -> Result<Vec<SerializedObjectGraphicsOverlay>, ScenarioError> {
    split_outside_delimiter(value, ';')
        .into_iter()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| parse_legacy_graphics_overlay(entry, line))
        .collect()
}

fn parse_legacy_graphics_overlay(
    value: &str,
    line: usize,
) -> Result<SerializedObjectGraphicsOverlay, ScenarioError> {
    let fields = split_outside_delimiter(value, ',');
    if !(7..=9).contains(&fields.len()) {
        return Err(object_property_error(
            line,
            "GfxOverlay",
            value,
            &format!("expected 7 to 9 fields, found {}", fields.len()),
        ));
    }
    let graphics = fields[1].trim();
    let graphics = if graphics.is_empty() {
        None
    } else {
        Some(parse_legacy_object_graphics(
            graphics,
            line,
            "GfxOverlay graphics",
        )?)
    };
    let mode_value = parse_object_i32(fields[2].trim(), line, "GfxOverlay mode")?;
    let mode = crate::GraphicsOverlayMode::from_script_value(mode_value).ok_or_else(|| {
        object_property_error(
            line,
            "GfxOverlay mode",
            fields[2].trim(),
            "unsupported graphics-overlay mode",
        )
    })?;
    let transform = fields[6]
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| {
            object_property_error(
                line,
                "GfxOverlay transform",
                fields[6].trim(),
                "expected a parenthesized draw transform",
            )
        })?;
    Ok(SerializedObjectGraphicsOverlay {
        id: parse_object_i32(fields[0].trim(), line, "GfxOverlay id")?,
        mode,
        definition: graphics
            .as_ref()
            .map(|graphics| graphics.definition.clone()),
        graphics_name: graphics.and_then(|graphics| graphics.graphics_name),
        action: (!fields[3].trim().is_empty()).then(|| fields[3].trim().to_string()),
        blit_mode: parse_object_u32(fields[4].trim(), line, "GfxOverlay blit mode")?,
        phase: parse_object_i32(fields[5].trim(), line, "GfxOverlay phase")?,
        transform: parse_legacy_draw_transform(transform, line, "GfxOverlay transform")?,
        color_modulation: if fields.len() >= 8 {
            parse_object_u32(fields[7].trim(), line, "GfxOverlay color modulation")?
        } else {
            0x00ff_ffff
        },
        overlay_object: if fields.len() >= 9 {
            parse_object_i32(fields[8].trim(), line, "GfxOverlay object")?
        } else {
            0
        },
    })
}

fn parse_legacy_physical_changes(value: &str) -> Vec<(String, i32)> {
    let mut changes = Vec::new();
    let mut position = 0usize;
    loop {
        skip_std_whitespace(value, &mut position);
        let name_start = position;
        while value
            .as_bytes()
            .get(position)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        {
            position += 1;
        }
        if position == name_start {
            break;
        }
        let name = &value[name_start..position];
        if !is_legacy_physical_name(name) {
            break;
        }
        if !consume_std_separator(value, &mut position, b'=') {
            break;
        }
        let Some(previous) = parse_std_i32_prefix_at(value, &mut position) else {
            break;
        };
        changes.push((name.to_string(), previous));
        if !consume_std_separator(value, &mut position, b',') {
            break;
        }
    }
    changes
}

fn is_legacy_physical_name(name: &str) -> bool {
    matches!(
        name,
        "Energy"
            | "Breath"
            | "Walk"
            | "Jump"
            | "Scale"
            | "Hangle"
            | "Dig"
            | "Swim"
            | "Throw"
            | "Push"
            | "Fight"
            | "Magic"
            | "Float"
            | "CanScale"
            | "CanHangle"
            | "CanDig"
            | "CanConstruct"
            | "CanChop"
            | "CanFly"
            | "CorrosionResist"
            | "BreatheWater"
    )
}

fn parse_legacy_object_command(
    value: &str,
    line: usize,
) -> Result<SerializedLegacyCommand, ScenarioError> {
    let value = value.trim_start();
    let (version, payload) = if let Some(versioned) = value.strip_prefix('$') {
        let (version, payload) = versioned.split_once(',').ok_or_else(|| {
            object_property_error(
                line,
                "Command",
                value,
                "versioned command is missing its first separator",
            )
        })?;
        let version = parse_object_i32(version.trim(), line, "Command version")?;
        (version, payload)
    } else {
        (0, value)
    };

    // Version zero has no BaseMode field. Versions one and later do. The
    // final RCT_All text field may itself contain commas, so cap the split at
    // the layout's exact field count.
    let field_count = if version > 0 { 15 } else { 14 };
    let fields = split_outside_delimiter_limit(payload, ',', field_count);
    if fields.len() != field_count {
        return Err(object_property_error(
            line,
            "Command",
            value,
            &format!(
                "command version {version} requires {field_count} payload fields, found {}",
                fields.len()
            ),
        ));
    }
    let name = fields[0].trim();
    if crate::command::CommandId::from_name(name).is_none() {
        return Err(object_property_error(
            line,
            "Command name",
            name,
            "unknown C4 command",
        ));
    }
    let integer = |index: usize, label: &str| parse_object_i32(fields[index].trim(), line, label);
    let base_mode = if version > 0 {
        integer(13, "Command BaseMode")?
    } else {
        0
    };
    let text_index = if version > 0 { 14 } else { 13 };
    let mut text = fields[text_index].to_string();
    // C4Command::CompileFunc's compatibility repair for old layouts.
    if version < 2 && text == "0" {
        text.clear();
    }
    Ok(SerializedLegacyCommand {
        name: name.to_string(),
        tx: parse_serialized_c4value(fields[1].trim(), line)?,
        ty: integer(2, "Command Ty")?,
        target: integer(3, "Command Target")?,
        target2: integer(4, "Command Target2")?,
        data: integer(5, "Command Data")?,
        update_interval: integer(6, "Command UpdateInterval")?,
        evaluated: integer(7, "Command Evaluated")?,
        path_checked: integer(8, "Command PathChecked")?,
        finished: integer(9, "Command Finished")?,
        failures: integer(10, "Command Failures")?,
        retries: integer(11, "Command Retries")?,
        permit: integer(12, "Command Permit")?,
        base_mode,
        // RCT_All consumes the complete remaining field, including commas.
        text,
    })
}

fn parse_legacy_object_effects(
    value: &str,
    line: usize,
) -> Result<Vec<SerializedEffectState>, ScenarioError> {
    split_outside_delimiter(value, ',')
        .into_iter()
        .map(str::trim)
        .filter(|effect| !effect.is_empty())
        .map(|effect| {
            parse_serialized_effect_state(effect, line)
                .map_err(|detail| object_property_error(line, "Effects", effect, detail.as_str()))
        })
        .collect()
}

/// C4Object::CustomName uses StdCompiler's escaped-string adapter. Modern
/// saves quote the value; older shipped saves keep the whole unquoted line
/// (StdCompiler.cpp:734-741, 936-976, 1006-1062).
fn parse_legacy_object_name(value: &str, line: usize) -> Result<Option<String>, ScenarioError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !trimmed.starts_with('"') {
        return Ok(Some(trimmed.to_string()));
    }

    let mut chars = trimmed[1..].chars().peekable();
    let mut decoded = String::new();
    let mut terminated = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                terminated = true;
                break;
            }
            '\\' => {
                let escaped = match chars.next() {
                    Some('a') => '\u{0007}',
                    Some('b') => '\u{0008}',
                    Some('f') => '\u{000c}',
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    Some('v') => '\u{000b}',
                    Some('\'') => '\'',
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('?') => '?',
                    Some('x') => {
                        let mut code = 0u32;
                        let mut found = false;
                        while let Some(digit) = chars.peek().and_then(|next| next.to_digit(16)) {
                            found = true;
                            code = code.wrapping_mul(16).wrapping_add(digit);
                            chars.next();
                        }
                        if found {
                            char::from_u32(code & 0xff).unwrap_or('\0')
                        } else {
                            'x'
                        }
                    }
                    Some(first @ '0'..='7') => {
                        let mut code = first.to_digit(8).unwrap_or(0);
                        while let Some(digit) = chars.peek().and_then(|next| next.to_digit(8)) {
                            code = code.wrapping_mul(8).wrapping_add(digit);
                            chars.next();
                        }
                        char::from_u32(code & 0xff).unwrap_or('\0')
                    }
                    Some(other) => other,
                    None => {
                        return Err(ScenarioError::LegacyObjectsParse(format!(
                            "Objects.txt line {}: unterminated escape in Name",
                            line
                        )));
                    }
                };
                decoded.push(escaped);
            }
            other => decoded.push(other),
        }
    }

    if !terminated || chars.any(|ch| !ch.is_whitespace()) {
        return Err(ScenarioError::LegacyObjectsParse(format!(
            "Objects.txt line {}: unterminated or malformed quoted Name `{}`",
            line, trimmed
        )));
    }
    Ok((!decoded.is_empty()).then_some(decoded))
}

fn parse_i64(value: &str) -> Result<i64, std::num::ParseIntError> {
    let trimmed = value.trim();
    // Handle C4Fixed format: 'f' or 'F' prefix indicates a fixed-point number
    // Strip the prefix and parse the integer value (which may be hex or decimal)
    let trimmed = trimmed
        .strip_prefix('f')
        .or_else(|| trimmed.strip_prefix('F'))
        .unwrap_or(trimmed);

    if let Some(rest) = trimmed
        .strip_prefix("-0x")
        .or_else(|| trimmed.strip_prefix("-0X"))
    {
        i64::from_str_radix(rest, 16).map(|parsed| -parsed)
    } else if let Some(rest) = trimmed
        .strip_prefix("+0x")
        .or_else(|| trimmed.strip_prefix("+0X"))
    {
        i64::from_str_radix(rest, 16)
    } else if let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        i64::from_str_radix(rest, 16)
    } else {
        // StdCompilerINIRead reads numbers strtol-style: optional sign +
        // leading digits, trailing junk ignored (real content carries
        // trailing `;`, e.g. `Position=22,28;` in LastWill.c4s). No digits
        // at all stays an error (the empty-slice parse).
        let (sign, digits) = match trimmed.as_bytes().first() {
            Some(b'-') => (-1i64, &trimmed[1..]),
            Some(b'+') => (1, &trimmed[1..]),
            _ => (1, trimmed),
        };
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        digits[..end].parse::<i64>().map(|value| sign * value)
    }
}

fn parse_i32(value: &str) -> Result<i32, String> {
    let parsed = parse_i64(value).map_err(|err| err.to_string())?;
    i32::try_from(parsed).map_err(|_| "value out of range for i32".to_string())
}

/// Objects.txt `LocalNamed=` (C4ValueMapData::CompileFunc,
/// C4ValueMap.cpp:236-295): `<count>;name=<value>,name=<value>,...` where
/// each value uses the C4Value type-char encoding (GetC4VID,
/// C4Value.cpp:368-394). A zero count writes no separator and no entries.
#[derive(Debug, Default)]
struct InitialNetworkRuntimeState {
    sky: Option<InitialNetworkSkyState>,
    script_globals: SerializedScriptGlobalState,
    global_effects: Vec<SerializedEffectState>,
    scoreboard: ScoreboardState,
}

#[derive(Debug)]
struct InitialNetworkSkyState {
    fixed: [i32; 4],
    modulation: u32,
    parallax_x: i32,
    parallax_y: i32,
    parallax_mode: i32,
    back_color: u32,
    back_color_enabled: bool,
}

#[derive(Debug, Default)]
struct SerializedScriptGlobalState {
    numbered: Vec<SerializedC4Value>,
    named: Vec<(String, SerializedC4Value)>,
}

#[derive(Debug, Clone, PartialEq)]
struct SerializedEffectState {
    number: i32,
    name: String,
    priority: i32,
    interval: i32,
    timer: i32,
    command_target: i32,
    command_id: Option<String>,
    vars: Vec<SerializedC4Value>,
}

impl InitialNetworkRuntimeState {
    fn parse(data: &InitialNetworkGameData) -> Result<Self, ScenarioError> {
        Ok(Self {
            sky: data
                .compiled_sections
                .sky()
                .map(parse_initial_network_sky)
                .transpose()?,
            script_globals: data
                .compiled_sections
                .script_engine()
                .map(parse_initial_network_script_globals)
                .transpose()?
                .unwrap_or_default(),
            global_effects: data
                .compiled_sections
                .effects()
                .map(parse_initial_network_effects)
                .transpose()?
                .unwrap_or_default(),
            scoreboard: data
                .compiled_sections
                .scoreboard()
                .map(parse_initial_network_scoreboard)
                .transpose()?
                .unwrap_or_default(),
        })
    }

    fn resolve_post_object_state(
        self,
        object_numbers: &HashSet<u64>,
        string_registrations: &clonk_script::StringRegistrations,
    ) -> (ScriptGlobalState, Vec<EffectState>) {
        let resolution = SerializedC4ValueResolution {
            object_numbers,
            string_registrations,
        };
        let numbered = self
            .script_globals
            .numbered
            .into_iter()
            .enumerate()
            .filter_map(|(index, value)| {
                i32::try_from(index)
                    .ok()
                    .map(|index| (index, value.resolve(&resolution)))
            })
            .collect::<BTreeMap<_, _>>();
        let named = self
            .script_globals
            .named
            .into_iter()
            .map(|(name, value)| (name, value.resolve(&resolution)))
            .collect::<BTreeMap<_, _>>();
        let effects = self
            .global_effects
            .into_iter()
            .map(|effect| effect.resolve(&resolution))
            .collect();
        (ScriptGlobalState { numbered, named }, effects)
    }
}

impl InitialNetworkSkyState {
    /// C4Game compiles the runtime words before C4Landscape::Init calls
    /// C4Sky::Init. Fresh games reset scroll position/speed/parallax there;
    /// savegames retain them. A loaded bitmap then applies SkyScrollMode on
    /// top in both cases (C4Game.cpp:2654-2665; C4Sky.cpp:71-125).
    fn into_frame(
        self,
        mut settings: SkySettings,
        savegame: bool,
        sky_scroll_mode: i32,
    ) -> SkyFrame {
        let fixed = if savegame { self.fixed } else { [0; 4] };
        settings.parallax_x = if savegame { self.parallax_x } else { 10 };
        settings.parallax_y = if savegame { self.parallax_y } else { 10 };
        settings.parallax_mode = if savegame && self.parallax_mode == 1 {
            SkyParallaxMode::Wind
        } else {
            SkyParallaxMode::Fixed
        };
        if settings.has_surface {
            match sky_scroll_mode {
                1 => {
                    settings.parallax_mode = SkyParallaxMode::Wind;
                    settings.parallax_y = 20;
                }
                2 => {
                    settings.parallax_x = 20;
                    settings.parallax_y = 20;
                }
                _ => {}
            }
        }
        settings.modulation = Some(self.modulation);
        settings.back_color_raw = self.back_color;
        settings.back_color = self.back_color_enabled.then_some(self.back_color);
        settings.base_xdir = crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[2]));
        settings.base_ydir = crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[3]));
        SkyFrame {
            settings,
            offset_x: crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[0])),
            offset_y: crate::math::fixtof(crate::math::C4Fixed::from_raw(fixed[1])),
            fixed: Some(fixed),
        }
    }
}

impl SerializedEffectState {
    fn resolve(self, resolution: &SerializedC4ValueResolution<'_>) -> EffectState {
        // C4EnumeratedObjectPtr only recognizes the old pointer-offset
        // spelling inside the complete C4EnumPointer1..C4EnumPointer2 range.
        // A modern, raw object number above that range must not be shifted
        // (C4EnumeratedObjectPtr.cpp:32-42).
        let command_target = if (1_000_000_000..=1_001_000_000).contains(&self.command_target) {
            self.command_target - 1_000_000_000
        } else {
            self.command_target
        };
        let command_target = u64::try_from(command_target)
            .ok()
            .filter(|number| *number != 0 && resolution.object_numbers.contains(number))
            .and_then(|number| i32::try_from(number).ok());
        EffectState {
            number: self.number,
            name: self.name,
            priority: self.priority,
            interval: self.interval,
            timer: self.timer,
            command_target,
            command_id: self.command_id,
            vars: self
                .vars
                .into_iter()
                .map(|value| effect_var_from_value(value.resolve(resolution)))
                .collect(),
            // A compiled effect has already run its synchronous Start call.
            start_dispatched: true,
        }
    }
}

fn initial_network_section_tree(
    bytes: &[u8],
    name: &str,
) -> Result<(LegacyIniTree, usize), ScenarioError> {
    let source = clonk_script::c4_string_from_bytes(bytes);
    let tree = LegacyIniTree::parse(&source);
    let section = tree.first_section(0, name).ok_or_else(|| {
        ScenarioError::InitialNetworkRuntime(format!(
            "retained [{name}] block has no [{name}] section"
        ))
    })?;
    Ok((tree, section))
}

fn parse_initial_network_sky(bytes: &[u8]) -> Result<InitialNetworkSkyState, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Sky")?;
    Ok(InitialNetworkSkyState {
        fixed: [
            ini_i32(&tree, section, "X", 0),
            ini_i32(&tree, section, "Y", 0),
            ini_i32(&tree, section, "XDir", 0),
            ini_i32(&tree, section, "YDir", 0),
        ],
        modulation: ini_u32(&tree, section, "Modulation", 0x00ff_ffff),
        parallax_x: ini_i32(&tree, section, "ParX", 10),
        parallax_y: ini_i32(&tree, section, "ParY", 10),
        parallax_mode: ini_i32(&tree, section, "ParMode", 0),
        back_color: ini_u32(&tree, section, "BackClr", 0),
        back_color_enabled: ini_bool(&tree, section, "BackClrEnabled", false),
    })
}

fn parse_initial_network_script_globals(
    bytes: &[u8],
) -> Result<SerializedScriptGlobalState, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Script")?;
    let numbered = match tree.value(section, "Globals") {
        Some(value) => parse_local_slots(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script] Globals: {error}"))
        })?,
        None => parse_nested_script_globals(bytes)?,
    };
    let named = match tree.value(section, "GlobalNamed") {
        Some(value) => parse_local_named(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script] GlobalNamed: {error}"))
        })?,
        None => parse_nested_script_global_named(bytes)?,
    };
    Ok(SerializedScriptGlobalState { numbered, named })
}

fn nested_script_entries(bytes: &[u8], target: &str) -> Option<Vec<(String, String)>> {
    let source = clonk_script::c4_string_from_bytes(bytes);
    let mut target_indent = None;
    let mut entries = Vec::new();
    for line in legacy_ini_lines(&source) {
        let indent = line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let trimmed = line.trim_start_matches([' ', '\t']);
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|value| value.split_once(']'))
            .map(|(name, _)| name)
        {
            if target_indent.is_some_and(|target_indent| indent <= target_indent) {
                break;
            }
            if name == target {
                target_indent = Some(indent);
            }
            continue;
        }
        if target_indent.is_none() {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        entries.push((name.trim().to_string(), value.to_string()));
    }
    target_indent.map(|_| entries)
}

fn parse_nested_script_globals(bytes: &[u8]) -> Result<Vec<SerializedC4Value>, ScenarioError> {
    let Some(entries) = nested_script_entries(bytes, "Globals") else {
        return Ok(Vec::new());
    };
    if let Some((_, value)) = entries
        .iter()
        .find(|(name, _)| matches!(name.as_str(), "Value" | "Values" | "Data"))
    {
        return parse_local_slots(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script][Globals]: {error}"))
        });
    }
    let mut indexed = Vec::new();
    for (name, encoded) in entries {
        let index = name.parse::<usize>().map_err(|_| {
            ScenarioError::InitialNetworkRuntime(format!(
                "[Script][Globals] invalid slot name `{name}`"
            ))
        })?;
        let value = parse_nested_script_c4value(&encoded)?;
        indexed.push((index, value));
    }
    indexed.sort_by_key(|(index, _)| *index);
    let size = indexed
        .last()
        .map_or(0, |(index, _)| index.saturating_add(1));
    let mut values = (0..size)
        .map(|_| SerializedC4Value::Value(clonk_script::Value::Nil))
        .collect::<Vec<_>>();
    for (index, value) in indexed {
        values[index] = value;
    }
    Ok(values)
}

fn parse_nested_script_global_named(
    bytes: &[u8],
) -> Result<Vec<(String, SerializedC4Value)>, ScenarioError> {
    let Some(entries) = nested_script_entries(bytes, "GlobalNamed") else {
        return Ok(Vec::new());
    };
    if let Some((_, value)) = entries
        .iter()
        .find(|(name, _)| matches!(name.as_str(), "Value" | "Values" | "Data"))
    {
        return parse_local_named(value, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("[Script][GlobalNamed]: {error}"))
        });
    }
    entries
        .into_iter()
        .map(|(name, encoded)| Ok((name, parse_nested_script_c4value(&encoded)?)))
        .collect()
}

fn parse_nested_script_c4value(encoded: &str) -> Result<SerializedC4Value, ScenarioError> {
    let encoded = encoded.trim();
    if encoded.chars().next().is_some_and(|type_char| {
        matches!(
            type_char,
            'A' | 'i' | 'b' | 'o' | 'O' | 'I' | 'S' | 'a' | 'm'
        )
    }) {
        return parse_serialized_c4value(encoded, 1).map_err(|error| {
            ScenarioError::InitialNetworkRuntime(format!("nested Script C4Value: {error}"))
        });
    }
    let value = parse_i32(encoded).map_err(|error| {
        ScenarioError::InitialNetworkRuntime(format!(
            "nested Script value `{encoded}` is neither typed nor an integer ({error})"
        ))
    })?;
    Ok(SerializedC4Value::Value(if value == 0 {
        clonk_script::Value::Nil
    } else {
        clonk_script::Value::Int(value)
    }))
}

fn parse_initial_network_effects(
    bytes: &[u8],
) -> Result<Vec<SerializedEffectState>, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Effects")?;
    let Some(serialized) = tree.value(section, "GlobalEffects") else {
        return Ok(Vec::new());
    };
    split_outside_delimiter(serialized.trim(), ',')
        .into_iter()
        .map(str::trim)
        .filter(|effect| !effect.is_empty())
        .map(parse_initial_network_effect)
        .collect()
}

fn parse_initial_network_effect(serialized: &str) -> Result<SerializedEffectState, ScenarioError> {
    let error = |detail: String| {
        ScenarioError::InitialNetworkRuntime(format!(
            "[Effects] GlobalEffects `{serialized}`: {detail}"
        ))
    };
    parse_serialized_effect_state(serialized, 1).map_err(error)
}

/// Shared C4Effect::CompileFunc decoder for global and per-object chains.
/// Its variables remain serialized until all object numbers and Strings.txt
/// entries are available for the native denumeration pass.
fn parse_serialized_effect_state(
    serialized: &str,
    line: usize,
) -> Result<SerializedEffectState, String> {
    let error = |detail: String| format!("effect `{serialized}`: {detail}");
    let open = serialized
        .find('(')
        .ok_or_else(|| error("missing `(`".to_string()))?;
    let close = serialized[open + 1..]
        .find(')')
        .map(|index| open + 1 + index)
        .ok_or_else(|| error("missing `)`".to_string()))?;
    let name = serialized[..open].trim();
    if name.is_empty() {
        return Err(error("missing effect name".to_string()));
    }
    let fields = split_outside_delimiter(&serialized[open + 1..close], ',');
    if fields.len() != 6 {
        return Err(error(format!(
            "expected 6 header fields, found {}",
            fields.len()
        )));
    }
    let int_field = |index: usize, label: &str| {
        parse_std_i32(fields[index])
            .ok_or_else(|| error(format!("invalid {label} value `{}`", fields[index].trim())))
    };
    let command_id = fields[5].trim();
    let command_id = if command_id == "NONE" {
        None
    } else if clonk_script::c4_string_bytes(command_id).len() == 4 {
        Some(command_id.to_string())
    } else {
        None
    };
    let tail = serialized[close + 1..].trim();
    let vars = if tail.is_empty() {
        Vec::new()
    } else {
        let inner = tail
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .ok_or_else(|| error(format!("invalid effect variable list `{tail}`")))?;
        parse_local_slots(inner, line).map_err(|parse_error| error(parse_error.to_string()))?
    };
    Ok(SerializedEffectState {
        name: name.to_string(),
        number: int_field(0, "number")?,
        priority: int_field(1, "priority")?,
        timer: int_field(2, "time")?,
        interval: int_field(3, "interval")?,
        command_target: int_field(4, "command target")?,
        command_id,
        vars,
    })
}

fn parse_initial_network_scoreboard(bytes: &[u8]) -> Result<ScoreboardState, ScenarioError> {
    let (tree, section) = initial_network_section_tree(bytes, "Scoreboard")?;
    let rows = ini_i32(&tree, section, "Rows", 0);
    let columns = ini_i32(&tree, section, "Cols", 0);
    let show_count = ini_i32(&tree, section, "DlgShow", 0);
    let row_count = usize::try_from(rows).map_err(|_| {
        ScenarioError::InitialNetworkRuntime(format!("[Scoreboard] negative Rows value {rows}"))
    })?;
    let column_count = usize::try_from(columns).map_err(|_| {
        ScenarioError::InitialNetworkRuntime(format!("[Scoreboard] negative Cols value {columns}"))
    })?;
    let cell_count = row_count.checked_mul(column_count).ok_or_else(|| {
        ScenarioError::InitialNetworkRuntime(format!(
            "[Scoreboard] dimensions {rows}x{columns} overflow the host address space"
        ))
    })?;
    let mut cells = Vec::new();
    cells.try_reserve_exact(cell_count).map_err(|_| {
        ScenarioError::InitialNetworkRuntime(format!(
            "[Scoreboard] dimensions {rows}x{columns} cannot be allocated"
        ))
    })?;
    for row in 0..row_count {
        for column in 0..column_count {
            let string_key = format!("Cell{column}_{row}String");
            let value_key = format!("Cell{column}_{row}Value");
            let text = tree
                .value(section, &string_key)
                .map(decode_legacy_game_string)
                .ok_or_else(|| {
                    ScenarioError::InitialNetworkRuntime(format!(
                        "[Scoreboard] missing required `{string_key}`"
                    ))
                })?;
            let value = tree
                .value(section, &value_key)
                .and_then(parse_std_i32)
                .ok_or_else(|| {
                    ScenarioError::InitialNetworkRuntime(format!(
                        "[Scoreboard] missing or invalid required `{value_key}`"
                    ))
                })?;
            cells.push((Some(text), value));
        }
    }
    ScoreboardState::from_compiled_cells(row_count, column_count, show_count, cells).ok_or_else(
        || {
            ScenarioError::InitialNetworkRuntime(format!(
                "[Scoreboard] dimensions {rows}x{columns} disagree with the compiled cell matrix"
            ))
        },
    )
}

fn effect_var_from_value(value: clonk_script::Value) -> EffectVarValue {
    use clonk_script::Value;
    match value {
        Value::Int(value) => EffectVarValue::Int(value),
        Value::Bool(value) => EffectVarValue::Bool(value),
        Value::RawBool(value) => EffectVarValue::RawBool(value),
        Value::String(value) => EffectVarValue::String(value),
        Value::C4Id(value) => EffectVarValue::C4Id(value),
        Value::Object(value) => EffectVarValue::Object(value),
        Value::Array(values) => {
            EffectVarValue::Array(values.into_iter().map(effect_var_from_value).collect())
        }
        Value::Proplist(values) => EffectVarValue::Proplist(values),
        Value::Nil => EffectVarValue::Nil,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum SerializedC4Value {
    Value(clonk_script::Value),
    /// Untyped legacy C4V_Any word. Values in the old enumerated-pointer
    /// range are denumerated when the referenced object exists; otherwise
    /// C4Value::GuessType leaves these serialized words as integers.
    Any(i32),
    ObjectNumber(i32),
    StringTableIndex(i32),
    Array(Vec<SerializedC4Value>),
    Map {
        entries: Vec<(SerializedC4Value, SerializedC4Value)>,
        // Compile-time removals can only leave cleared (nil) mapped slots.
        // C4ValueHash::DenumeratePointers does not traverse emptyValues.
        empty_value_count: usize,
    },
}

struct SerializedC4ValueResolution<'a> {
    object_numbers: &'a HashSet<u64>,
    string_registrations: &'a clonk_script::StringRegistrations,
}

impl SerializedC4Value {
    /// Mirror C4Value::DenumeratePointer and serialized-string lookup
    /// (C4Value.cpp:686-713,783-798). Serialized identities become live VM
    /// values only after the accepted object-number and string tables exist.
    fn resolve(self, resolution: &SerializedC4ValueResolution<'_>) -> clonk_script::Value {
        self.resolve_strings(resolution.string_registrations)
            .denumerate_objects(resolution.object_numbers)
    }

    /// C4Value::CompileFunc resolves every serialized string while compiling
    /// the complete container. Pointer denumeration is a later pass, so a
    /// missing object entry cannot prevent a sibling from claiming the same
    /// loaded C4String identity.
    fn resolve_strings(
        self,
        string_registrations: &clonk_script::StringRegistrations,
    ) -> SerializedC4Value {
        match self {
            Self::StringTableIndex(index) => Self::Value(
                clonk_script::resolve_c4_string(string_registrations, index)
                    .map(clonk_script::Value::String)
                    .unwrap_or(clonk_script::Value::Nil),
            ),
            Self::Array(values) => Self::Array(
                values
                    .into_iter()
                    .map(|value| value.resolve_strings(string_registrations))
                    .collect(),
            ),
            Self::Map {
                entries,
                empty_value_count,
            } => {
                let mut compiled_entries =
                    Vec::<(SerializedC4Value, SerializedC4Value)>::with_capacity(entries.len());
                let mut compiled_empty_value_count = empty_value_count;
                for (key, value) in entries {
                    let key = key.resolve_strings(string_registrations);
                    let value = value.resolve_strings(string_registrations);
                    if let Some(index) = compiled_entries
                        .iter()
                        .position(|(existing, _)| existing == &key)
                    {
                        if value.is_compiled_nil() && !compiled_entries[index].1.is_compiled_nil() {
                            compiled_entries.remove(index);
                            compiled_empty_value_count += 1;
                        } else {
                            compiled_entries[index].1 = value;
                        }
                    } else {
                        // CompileFunc's `map[key] = value` consumes a recycled
                        // mapped slot only for a genuinely new key. Compile-
                        // time removals leave nil slots, so assigning nil to
                        // one takes C4Value::Set's unchanged-value return.
                        compiled_empty_value_count = compiled_empty_value_count.saturating_sub(1);
                        compiled_entries.push((key, value));
                    }
                }
                Self::Map {
                    entries: compiled_entries,
                    empty_value_count: compiled_empty_value_count,
                }
            }
            value => value,
        }
    }

    fn denumerate_objects(self, object_numbers: &HashSet<u64>) -> clonk_script::Value {
        use clonk_script::Value;
        match self {
            Self::Value(value) => value,
            Self::Any(number) => {
                if (1_000_000_000..=1_001_000_000).contains(&number) {
                    let object_number = number - 1_000_000_000;
                    if let Ok(object_number) = u64::try_from(object_number) {
                        if object_numbers.contains(&object_number) {
                            return Value::Object(object_number);
                        }
                    }
                }
                serialized_any_fallback(number)
            }
            Self::ObjectNumber(number) => {
                // Old pointer-enumeration saves add C4EnumPointer1. For an
                // explicitly C4V_C4ObjectEnum value C++ subtracts it from any
                // value at or above the lower bound, then searches active and
                // inactive object lists (C4Value.cpp:693-703).
                let number = if number >= 1_000_000_000 {
                    number - 1_000_000_000
                } else {
                    number
                };
                u64::try_from(number)
                    .ok()
                    .filter(|number| object_numbers.contains(number))
                    .map(Value::Object)
                    .unwrap_or(Value::Nil)
            }
            Self::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| value.denumerate_objects(object_numbers))
                    .collect(),
            ),
            Self::Map {
                entries,
                empty_value_count,
            } => {
                // Denumerate every key and value before mutating the visible
                // hash. C4ValueHash::DenumeratePointers iterates already-
                // compiled C4Values; a key that clears retains its mapped
                // slot in emptyValues, while a value that clears contributes
                // the now-nil slot.
                let entries = entries
                    .into_iter()
                    .map(|(key, value)| {
                        let missing_key = key.is_missing_direct_object(object_numbers);
                        let missing_value = value.is_missing_direct_object(object_numbers);
                        (
                            missing_key || missing_value,
                            key.denumerate_objects(object_numbers),
                            value.denumerate_objects(object_numbers),
                        )
                    })
                    .collect::<Vec<_>>();
                let mut values = clonk_script::ValueMap::with_capacity(entries.len());
                let mut removed_values = Vec::new();
                for (removed, key, value) in entries {
                    if removed {
                        removed_values.push(value);
                        continue;
                    }
                    values.insert_key(key, value);
                }
                // Every surviving slot was allocated during CompileFunc,
                // before DenumeratePointers can populate emptyValues. Queue
                // removed slots only now so ordinary loaded entries cannot
                // accidentally reuse them. push_front/pop_front makes the
                // last removed slot the first one reused, matching Vec::pop.
                for _ in 0..empty_value_count {
                    values.recycle_value_slot(Value::Nil);
                }
                for value in removed_values {
                    values.recycle_value_slot(value);
                }
                Value::Proplist(values)
            }
            Self::StringTableIndex(_) => {
                unreachable!("serialized strings resolve before object denumeration")
            }
        }
    }

    fn is_missing_direct_object(&self, object_numbers: &HashSet<u64>) -> bool {
        let Self::ObjectNumber(number) = self else {
            return false;
        };
        let number = if *number >= 1_000_000_000 {
            *number - 1_000_000_000
        } else {
            *number
        };
        u64::try_from(number)
            .ok()
            .is_none_or(|number| !object_numbers.contains(&number))
    }

    fn is_compiled_nil(&self) -> bool {
        match self {
            Self::Value(clonk_script::Value::Nil) | Self::Any(0) | Self::ObjectNumber(0) => true,
            Self::Value(clonk_script::Value::C4Id(value)) => clonk_script::c4_id_raw(value) == 0,
            _ => false,
        }
    }
}

fn serialized_any_fallback(number: i32) -> clonk_script::Value {
    if number == 0 {
        return clonk_script::Value::Nil;
    }
    // GuessType checks packed literal IDs before falling back to int. The
    // numeric 1..9999 spelling is deliberately excluded by its >=10000 gate
    // (C4Value.cpp:299-330; C4Id.cpp:55-67).
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

/// Objects.txt `Locals=` is C4ValueList::CompileFunc
/// (C4ValueList.cpp:102-136). Current saves write
/// `<size>;<typed-value>,...`; trailing default values may be omitted. The
/// pre-size legacy form stores its first raw integer in slot zero and always
/// restores the ten C4MaxVariable slots.
fn parse_local_slots(value: &str, line: usize) -> Result<Vec<SerializedC4Value>, ScenarioError> {
    const C4_MAX_VARIABLE: usize = 10;
    const C4_VALUE_LIST_MAX_SIZE: usize = 1_000_000;

    let parse_error = |detail: String| {
        ScenarioError::LegacyObjectsParse(format!("Objects.txt line {}: {}", line, detail))
    };
    let trimmed = value.trim();
    let (size, values): (usize, Vec<SerializedC4Value>) = if let Some((size_text, values_text)) =
        trimmed.split_once(';')
    {
        let size = parse_i32(size_text.trim())
            .map_err(|error| parse_error(format!("invalid Locals size `{size_text}` ({error})")))?
            .try_into()
            .map_err(|_| parse_error(format!("invalid negative Locals size `{size_text}`")))?;
        if size > C4_VALUE_LIST_MAX_SIZE {
            return Err(parse_error(format!(
                "Locals size {size} exceeds C4ValueList::MaxSize"
            )));
        }
        let values = split_outside_brackets(values_text)
            .into_iter()
            .take(size)
            .map(str::trim)
            .map(|encoded| parse_serialized_c4value(encoded, line))
            .collect::<Result<Vec<_>, _>>()?;
        (size, values)
    } else {
        let mut encoded = split_outside_brackets(trimmed).into_iter();
        let first = encoded.next().unwrap_or_default().trim();
        let first = parse_i32(first).map_err(|error| {
            parse_error(format!("invalid legacy Locals value `{first}` ({error})"))
        })?;
        let mut values = vec![SerializedC4Value::Any(first)];
        values.extend(
            encoded
                .take(C4_MAX_VARIABLE - 1)
                .map(str::trim)
                .map(|entry| parse_serialized_c4value(entry, line))
                .collect::<Result<Vec<_>, _>>()?,
        );
        (C4_MAX_VARIABLE, values)
    };

    let mut values = values;
    values.truncate(size);
    values.resize_with(size, || SerializedC4Value::Value(clonk_script::Value::Nil));
    Ok(values)
}

fn parse_local_named(
    value: &str,
    line: usize,
) -> Result<Vec<(String, SerializedC4Value)>, ScenarioError> {
    let trimmed = value.trim();
    let (count_text, rest) = trimmed
        .split_once(';')
        .map_or((trimmed, None), |(count, rest)| (count, Some(rest)));
    let count = parse_std_i32(count_text).unwrap_or(0);
    if count == 0 {
        // C4ValueMapData returns immediately for a zero/defaulted count and
        // never consumes a trailing payload.
        return Ok(Vec::new());
    }
    let count = usize::try_from(count).map_err(|_| {
        ScenarioError::LegacyObjectsParse(format!(
            "Objects.txt line {}: invalid negative LocalNamed count `{}`",
            line, count_text
        ))
    })?;
    let rest = rest.ok_or_else(|| {
        ScenarioError::LegacyObjectsParse(format!(
            "Objects.txt line {}: LocalNamed count {} is missing `;`",
            line, count
        ))
    })?;
    let mut parts = split_outside_brackets(rest).into_iter();
    let mut entries = Vec::new();
    for index in 0..count {
        let part = parts.next().ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: LocalNamed declares {} entries but contains {}",
                line, count, index
            ))
        })?;
        let part = part.trim();
        if part.is_empty() {
            return Err(ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: LocalNamed entry {} is empty",
                line, index
            )));
        }
        let Some((name, encoded)) = part.split_once('=') else {
            return Err(ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: LocalNamed entry `{}` missing `=`",
                line, part
            )));
        };
        entries.push((
            name.trim().to_string(),
            parse_serialized_c4value(encoded.trim(), line)?,
        ));
    }
    // StdCompiler reads exactly iValueCnt entries. Any remaining bytes in
    // the named value are ignored, so trailing entries must not leak into
    // the live name map.
    Ok(entries)
}

/// Split on commas outside `[...]` (array payloads carry their own commas).
fn split_outside_brackets(text: &str) -> Vec<&str> {
    split_outside_delimiter(text, ',')
}

fn split_outside_delimiter(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut square_depth = 0usize;
    let mut round_depth = 0usize;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            '(' => round_depth += 1,
            ')' => round_depth = round_depth.saturating_sub(1),
            ch if ch == delimiter && square_depth == 0 && round_depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// `splitn`, but separators nested in the C4Value/transform bracket pairs do
/// not count. The unsplit tail is needed for C4Command::Text (RCT_All), which
/// may itself contain commas.
fn split_outside_delimiter_limit(text: &str, delimiter: char, limit: usize) -> Vec<&str> {
    if limit <= 1 {
        return vec![text];
    }
    let mut parts = Vec::with_capacity(limit);
    let mut square_depth = 0usize;
    let mut round_depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            '(' => round_depth += 1,
            ')' => round_depth = round_depth.saturating_sub(1),
            ch if ch == delimiter
                && square_depth == 0
                && round_depth == 0
                && parts.len() + 1 < limit =>
            {
                parts.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// One serialized C4Value (C4Value::CompileFunc, C4Value.cpp:717-800 +
/// GetC4VID :368-394): `A`=any (zero reads nil; old pointer-range words
/// denumerate before remaining nonzero values guess int), `i`=int, `b`=bool,
/// `o`/`O`=enumerated object number (0 = no object),
/// `I`=C4ID stored as its signed 32-bit payload, `a[size;elems]`=array with
/// trailing nils omitted on write, and `S` indexes the scenario Strings.txt.
/// `m[count;key=value;...]` retains arbitrary typed keys in insertion order.
fn parse_serialized_c4value(
    encoded: &str,
    line: usize,
) -> Result<SerializedC4Value, ScenarioError> {
    use clonk_script::Value;
    let parse_error = |detail: String| {
        ScenarioError::LegacyObjectsParse(format!("Objects.txt line {}: {}", line, detail))
    };
    let mut chars = encoded.chars();
    let Some(type_char) = chars.next() else {
        return Ok(SerializedC4Value::Value(Value::Nil));
    };
    let payload = &encoded[type_char.len_utf8()..];
    let int_payload = || {
        parse_i32(payload.trim())
            .map_err(|err| parse_error(format!("invalid C4Value payload `{}` ({})", encoded, err)))
    };
    match type_char {
        'A' => Ok(SerializedC4Value::Any(int_payload()?)),
        'i' => Ok(SerializedC4Value::Value(Value::Int(int_payload()?))),
        'b' => Ok(SerializedC4Value::Value(Value::from_c4_bool_raw(
            int_payload()?,
        ))),
        'o' | 'O' => Ok(SerializedC4Value::ObjectNumber(int_payload()?)),
        'I' => Ok(SerializedC4Value::Value(Value::C4Id(
            clonk_script::c4_id_from_raw(int_payload()? as isize as usize),
        ))),
        'a' => {
            let inner = payload
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .ok_or_else(|| {
                    parse_error(format!(
                        "invalid C4Value array `{}` (expected a[...])",
                        encoded
                    ))
                })?;
            let (size_text, elements_text) = inner.split_once(';').unwrap_or((inner, ""));
            let size = parse_i32(size_text.trim()).map_err(|err| {
                parse_error(format!("invalid array size in `{}` ({})", encoded, err))
            })?;
            if !(0..=1_000_000).contains(&size) {
                return Err(parse_error(format!(
                    "array size {} in `{}` exceeds C4ValueList::MaxSize",
                    size, encoded
                )));
            }
            let size = size as usize;
            let mut elements: Vec<SerializedC4Value> = split_outside_brackets(elements_text)
                .into_iter()
                .take(size)
                .map(str::trim)
                .map(|element| parse_serialized_c4value(element, line))
                .collect::<Result<_, _>>()?;
            // Trailing nils are omitted on write; restore the full size.
            if elements.len() < size {
                elements.resize_with(size, || SerializedC4Value::Value(Value::Nil));
            }
            Ok(SerializedC4Value::Array(elements))
        }
        'S' => Ok(SerializedC4Value::StringTableIndex(int_payload()?)),
        'm' => {
            let inner = payload
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .ok_or_else(|| {
                    parse_error(format!(
                        "invalid C4Value map `{}` (expected m[...])",
                        encoded
                    ))
                })?;
            let (count_text, entries_text) = inner.split_once(';').unwrap_or((inner, ""));
            let count = parse_i32(count_text.trim()).map_err(|err| {
                parse_error(format!("invalid map size in `{}` ({})", encoded, err))
            })?;
            if count < 0 {
                return Err(parse_error(format!("negative map size in `{}`", encoded)));
            }
            let count = count as usize;
            let mut serialized_entries = split_outside_delimiter(entries_text, ';').into_iter();
            let mut entries = Vec::new();
            for index in 0..count {
                let entry = serialized_entries.next().ok_or_else(|| {
                    parse_error(format!(
                        "map `{}` declares {} entries but contains {}",
                        encoded, count, index
                    ))
                })?;
                let entry = entry.trim();
                let equals = entry
                    .char_indices()
                    .scan((0usize, 0usize), |depth, (index, ch)| {
                        match ch {
                            '[' => depth.0 += 1,
                            ']' => depth.0 = depth.0.saturating_sub(1),
                            '(' => depth.1 += 1,
                            ')' => depth.1 = depth.1.saturating_sub(1),
                            '=' if depth.0 == 0 && depth.1 == 0 => return Some(Some(index)),
                            _ => {}
                        }
                        Some(None)
                    })
                    .flatten()
                    .next()
                    .ok_or_else(|| parse_error(format!("map entry `{entry}` missing `=`")))?;
                let key = parse_serialized_c4value(entry[..equals].trim(), line)?;
                let value = parse_serialized_c4value(entry[equals + 1..].trim(), line)?;
                entries.push((key, value));
            }
            Ok(SerializedC4Value::Map {
                entries,
                empty_value_count: 0,
            })
        }
        // Character only consumes an alphabetic byte. A raw number therefore
        // falls back to C4V_Any without consuming its first digit; an unknown
        // alphabetic type byte is consumed and GetC4VFromID also returns Any.
        // C4V_pC4Value is the one nonserializable exception.
        'V' => Err(parse_error(format!(
            "nonserializable C4Value reference in `{}`",
            encoded
        ))),
        other if other.is_ascii_alphabetic() => Ok(SerializedC4Value::Any(int_payload()?)),
        _ => Ok(SerializedC4Value::Any(parse_i32(encoded.trim()).map_err(
            |err| parse_error(format!("invalid C4Value payload `{}` ({})", encoded, err)),
        )?)),
    }
}

/// Comma-separated int array (StdCompiler mkArrayAdapt serialization,
/// e.g. `VertexX=2,-14,14`).
fn parse_i32_list(value: &str, line: usize, key: &str) -> Result<Vec<i32>, ScenarioError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            parse_i32(entry).map_err(|err| {
                ScenarioError::LegacyObjectsParse(format!(
                    "Objects.txt line {}: invalid {} entry `{}` ({})",
                    line, key, entry, err
                ))
            })
        })
        .collect()
}

/// C4IDList::CompileFunc with values (C4IDList.cpp:240-259): semicolon-
/// separated four-character IDs, each optionally followed by `=count`.
fn parse_legacy_object_components(
    value: &str,
    line: usize,
) -> Result<Vec<(DefinitionId, i32)>, ScenarioError> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let (id, count) = entry.split_once('=').unwrap_or((entry, "0"));
            let id = id.trim();
            let valid_id = id.len() == 4
                && id != "NONE"
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
            if !valid_id {
                return Err(ScenarioError::LegacyObjectsParse(format!(
                    "Objects.txt line {}: invalid Component id `{}`",
                    line, id
                )));
            }
            let count = parse_i32(count.trim()).map_err(|err| {
                ScenarioError::LegacyObjectsParse(format!(
                    "Objects.txt line {}: invalid Component count `{}` ({})",
                    line,
                    count.trim(),
                    err
                ))
            })?;
            Ok((DefinitionId::from(id), count))
        })
        .collect()
}

/// A serialized C4Fixed (Fixed.h:247-266): lowercase `f` means the int32
/// contains float bits converted through `FLOAT_TO_FIXED`; any other
/// alphabetic format byte leaves the following int32 raw. A missing format
/// byte is the old raw representation. GoldRush's hanging stalactites carry
/// `YDir=f1067030938` = 1.2 px/frame.
fn parse_c4fixed(value: &str) -> Result<crate::math::C4Fixed, String> {
    let trimmed = value.trim();
    // StdCompilerINIRead::Character consumes any alphabetic format byte.
    // Only lowercase `f` requests the legacy float-bit conversion; every
    // other letter (including `F`) leaves the following int32 word raw.
    let (format, rest) = match trimmed.as_bytes().first().copied() {
        Some(format) if format.is_ascii_alphabetic() => (Some(format), &trimmed[1..]),
        _ => (None, trimmed),
    };
    let raw = parse_std_i32(rest).ok_or_else(|| "invalid int32 value".to_string())?;
    if format == Some(b'f') {
        Ok(crate::math::ftofix(f32::from_bits(raw as u32)))
    } else {
        Ok(crate::math::C4Fixed::from_raw(raw))
    }
}

fn parse_u64(value: &str) -> Result<u64, String> {
    let parsed = parse_i64(value).map_err(|err| err.to_string())?;
    if parsed < 0 {
        Err("value must be >= 0".to_string())
    } else {
        Ok(parsed as u64)
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn owner_index_from_section(section: &str) -> Option<i32> {
    let suffix = section.trim_start_matches("player");
    if suffix.is_empty() {
        return Some(0);
    }
    let index = suffix.parse::<i32>().ok()?;
    let owner = index - 1;
    if owner < 0 {
        None
    } else {
        Some(owner)
    }
}

fn parse_player_position(value: &str) -> Option<Vector2> {
    let mut parts = value
        .split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty());
    let x = parts.next()?.parse::<i32>().ok()?;
    let y = parts.next()?.parse::<i32>().ok()?;
    Some(Vector2::new(x, y))
}

fn parse_crew_entries(value: &str) -> Vec<(String, i32)> {
    value
        .split(';')
        .filter_map(|segment| {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                return None;
            }
            let mut parts = trimmed
                .split('=')
                .map(|part| part.trim())
                .filter(|part| !part.is_empty());
            let token = parts.next()?.to_string();
            if token.is_empty() {
                return None;
            }
            let count = parts
                .next_back()
                .and_then(|raw| raw.parse::<i32>().ok())
                .filter(|count| *count > 0)
                .unwrap_or(1);
            Some((token, count))
        })
        .collect()
}

fn find_definition_by_token<'a>(
    definitions: &'a [ScenarioDefinition],
    token: &str,
) -> Option<&'a ScenarioDefinition> {
    if token.is_empty() {
        return None;
    }
    let trimmed = token.trim();
    if trimmed.len() == 4
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        let upper = trimmed.to_ascii_uppercase();
        if let Some(definition) = definitions
            .iter()
            .find(|definition| definition.id.eq_ignore_ascii_case(&upper))
        {
            return Some(definition);
        }
    }

    let lower = trimmed.to_ascii_lowercase();
    definitions.iter().find(|definition| {
        if definition.id.eq_ignore_ascii_case(trimmed) {
            return true;
        }
        match definition.name.as_ref() {
            Some(name) => name.eq_ignore_ascii_case(trimmed) || name.to_ascii_lowercase() == lower,
            None => false,
        }
    })
}

fn is_missing_group_error(error: &GroupError) -> bool {
    matches!(
        error,
        GroupError::Missing(_) | GroupError::NotDirectory(_) | GroupError::EntryNotFound(_)
    ) || matches!(
        error,
        GroupError::Io(io_error) if io_error.kind() == io::ErrorKind::NotFound
    )
}

fn resolve_one_definition_group(
    scenario: &Group,
    resolver: &dyn LegacyDefinitionResolver,
    spec: &str,
) -> Result<Group, ScenarioError> {
    let normalized = spec.replace('\\', "/");
    if normalized.is_empty() {
        return Err(ScenarioError::LegacyDefinitionNotFound {
            path: spec.to_string(),
        });
    }

    let normalized_path = legacy_definition_path(&normalized);
    if normalized_path.is_absolute() {
        return match open_group_path(&normalized_path) {
            Ok(group) => Ok(group),
            Err(error) if is_missing_group_error(&error) => {
                Err(ScenarioError::LegacyDefinitionNotFound { path: normalized })
            }
            Err(error) => Err(ScenarioError::Resources(error)),
        };
    }

    // C4GameResList opens one filename once. The resolver owns search-path
    // priority (global/external roots before scenario fallback); retain only
    // its first result so intentional external `.c4f/...` paths are valid.
    resolver
        .resolve_definition_groups(scenario, &normalized)?
        .into_iter()
        .next()
        .ok_or_else(|| ScenarioError::LegacyDefinitionNotFound { path: normalized })
}

fn legacy_definition_path(value: &str) -> PathBuf {
    clonk_resources::path_from_legacy_bytes(&clonk_script::c4_string_bytes(value))
}

fn open_group_relative_case_insensitive(
    mut group: Group,
    relative: &Path,
) -> Result<Group, GroupError> {
    for component in relative.components() {
        let Component::Normal(name) = component else {
            if component == Component::CurDir {
                continue;
            }
            return Err(GroupError::EntryNotFound(relative.to_path_buf()));
        };
        let name_bytes = legacy_group_path_component_bytes(name);
        let entry = group
            .entries()?
            .into_iter()
            .find(|entry| entry.name_bytes.eq_ignore_ascii_case(&name_bytes))
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        group = group.open_child_entry_exact(&entry)?;
    }
    Ok(group)
}

fn legacy_group_path_component_bytes(name: &OsStr) -> Vec<u8> {
    clonk_resources::path_to_legacy_bytes(Path::new(name))
}

/// Opens physical groups and virtual paths nested inside packed groups. A
/// packed child group's `root()` is a stable full-name label rather than a
/// host-filesystem path; walking from the deepest physical prefix makes that
/// retained label usable as a fixed definition resource on restart.
fn open_group_path(path: &Path) -> Result<Group, GroupError> {
    let direct_error = match Group::open(path) {
        Ok(group) => return Ok(group),
        Err(error) if is_missing_group_error(&error) => error,
        Err(error) => return Err(error),
    };

    for physical_prefix in path.ancestors().skip(1) {
        if physical_prefix.as_os_str().is_empty() || !physical_prefix.exists() {
            continue;
        }
        let group = Group::open(physical_prefix)?;
        let relative = path
            .strip_prefix(physical_prefix)
            .map_err(|_| GroupError::EntryNotFound(path.to_path_buf()))?;
        return open_group_relative_case_insensitive(group, relative);
    }
    Err(direct_error)
}

/// Opens `spec` strictly below `root`, one immediate group component at a
/// time. C4Group entry lookup is ASCII-case-insensitive even while crossing
/// packed child groups; host `Path::join` alone cannot model either property.
fn resolve_rooted_definition_group(root: &Path, spec: &str) -> Result<Group, ScenarioError> {
    let normalized = spec.replace('\\', "/");
    let normalized_path = legacy_definition_path(&normalized);
    let candidate = root.join(&normalized_path);
    let not_found = || ScenarioError::LegacyDefinitionNotFound {
        path: candidate.display().to_string(),
    };
    let group = match open_group_path(root) {
        Ok(group) => group,
        Err(error) if is_missing_group_error(&error) => return Err(not_found()),
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    match open_group_relative_case_insensitive(group, &normalized_path) {
        Ok(group) => Ok(group),
        Err(error) if is_missing_group_error(&error) => Err(not_found()),
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

/// Applies C4Game's exact `DefinitionPath + module` operation. Unlike
/// `Path::join`, this neither inserts a separator nor lets an absolute module
/// replace the configured prefix.
fn resolve_prefixed_definition_group(
    prefix: &Path,
    spelling: &str,
) -> Result<Group, ScenarioError> {
    let mut candidate = legacy_path_bytes(prefix);
    candidate.extend(clonk_script::c4_string_bytes(spelling));
    for byte in &mut candidate {
        if *byte == b'\\' {
            *byte = std::path::MAIN_SEPARATOR as u8;
        }
    }
    let candidate = legacy_path_from_bytes(candidate);
    match open_group_path(&candidate) {
        Ok(group) => Ok(group),
        Err(error) if is_missing_group_error(&error) => {
            Err(ScenarioError::LegacyDefinitionNotFound {
                path: candidate.display().to_string(),
            })
        }
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

fn legacy_path_bytes(path: &Path) -> Vec<u8> {
    clonk_resources::path_to_legacy_bytes(path)
}

fn legacy_path_from_bytes(bytes: Vec<u8>) -> PathBuf {
    clonk_resources::path_from_legacy_bytes(&bytes)
}

fn folder_local_definition_groups(scenario: &Group) -> Result<Vec<Group>, ScenarioError> {
    let mut folder_paths = scenario
        .root()
        .ancestors()
        .skip(1)
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c4f"))
        })
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    folder_paths.reverse();

    let mut groups = Vec::new();
    for path in folder_paths {
        let group = match open_group_path(&path) {
            Ok(group) => group,
            // C4Game::FoldersWithLocalsDefs skips path prefixes it cannot
            // open rather than turning them into definition resources.
            Err(error) if is_missing_group_error(&error) => continue,
            Err(error) => return Err(ScenarioError::Resources(error)),
        };
        let has_immediate_definition = group
            .entries()?
            .into_iter()
            .any(|entry| legacy_group_wildcard_match(b"*.c4d", &entry.name_bytes));
        if has_immediate_definition {
            groups.push(group);
        }
    }
    Ok(groups)
}

fn collect_definitions_from_group<S: AsRef<str>>(
    group: &Group,
    load_system_groups: bool,
    skip_ids: &HashSet<String>,
    languages: &[S],
    language_packs: &LanguagePacks,
    scenario: &Group,
    scenario_origin: Option<&str>,
    sound_effect_groups: &mut Vec<Group>,
    output: &mut Vec<CollectedDefinition>,
) -> Result<(), ScenarioError> {
    let mut primary_definition = false;
    // C4Def::Load diverts Particle.txt groups into C4ParticleDef before it
    // even attempts DefCore; they never become object definitions.
    if group.exists("Particle.txt") {
        // C4Def::Load marks particle groups as non-definitions, loads the
        // particle metadata, and then still runs the invalid-definition
        // LoadEffects path regardless of whether that metadata succeeded.
        sound_effect_groups.push(group.clone());
        match ResourceParticleDefinition::load(group) {
            Ok(definition) => output.push(CollectedDefinition::Particle(definition)),
            Err(error) => tracing::warn!(
                group = %group.root().display(),
                %error,
                "particle definition failed to load; skipping"
            ),
        }
    } else if group.exists("DefCore.txt") {
        // C4Def::Load checks SkipDefs immediately after DefCore, before
        // scripts, ActMap, graphics, sounds, or localized auxiliary data.
        // Probe the ID first so malformed data in a skipped definition is
        // never observed.
        let core = match ResourceDefCore::load(group) {
            Ok(core) => Some(core),
            Err(ResourceDefinitionError::DefCoreMissing) => {
                sound_effect_groups.push(group.clone());
                None
            }
            Err(error) if is_rejected_definition_error(&error) => {
                warn_rejected_definition(group, &error);
                // A failed C4DefCore::Load deliberately turns the group into
                // a pure sound container before C4DefList visits children.
                sound_effect_groups.push(group.clone());
                None
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(core) = core {
            if !core.has_valid_id() {
                tracing::warn!(
                    id = %core.id,
                    group = %group.root().display(),
                    "skipping definition with invalid C4ID"
                );
                // NeededGfxMode is checked even after an invalid ID made the
                // definition unsuccessful. OLDGFX therefore suppresses the
                // otherwise intentional pure-sound fallback.
                if core.needed_gfx_mode != 2 {
                    sound_effect_groups.push(group.clone());
                }
            } else if skip_ids.contains(&core.id.to_ascii_uppercase()) {
                // C4Def::Load checks SkipDefs before the graphics-mode gate.
            } else if core.needed_gfx_mode == 2 {
                // C4DGFXMODE_OLDGFX is no longer supported. Native returns
                // false here without a dedicated diagnostic.
            } else {
                let components =
                    language_packs.component_groups(group, Some(scenario), scenario_origin);
                match ResourceDefinitionData::load_with_core_and_languages_and_components(
                    group,
                    core,
                    languages,
                    &components,
                ) {
                    Ok(resource) => {
                        if resource.graphics_image.is_none() {
                            warn_rejected_definition(
                                group,
                                &"required Graphics.png/Graphics.bmp is missing or invalid",
                            );
                        } else {
                            primary_definition = true;
                            // Valid definitions reach LoadEffects only after
                            // bitmap, portrait and ActMap/resource loading has
                            // succeeded. Retain the event before descending
                            // into child definitions.
                            sound_effect_groups.push(group.clone());
                            let mut definition =
                                scenario_definition_from_resource(resource, Some(group.clone()));
                            definition.script = localize_script_source_with_components(
                                &components,
                                &definition.script,
                                languages,
                            )?;
                            output.push(CollectedDefinition::Definition(definition));
                        }
                    }
                    Err(error) if is_rejected_definition_error(&error) => {
                        warn_rejected_definition(group, &error);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    } else {
        // Missing DefCore is the canonical pure `.c4d` sound-folder case.
        sound_effect_groups.push(group.clone());
    }

    // C4DefList::Load recursively visits only *.c4d children.
    for entry in group.entries()? {
        if !legacy_group_wildcard_match(b"*.c4d", &entry.name_bytes) {
            continue;
        }
        // FindNextEntry("*.c4d") also sees normal files and corrupt packed
        // entries. C4Group::OpenAsChild failure simply skips that candidate.
        let Ok(child) = group.open_child_entry_exact(&entry) else {
            continue;
        };
        // The recursive call omits fLoadSysGroups in C++, so its default true
        // applies even when only the scenario root suppressed System loading.
        collect_definitions_from_group(
            &child,
            true,
            skip_ids,
            languages,
            language_packs,
            scenario,
            scenario_origin,
            sound_effect_groups,
            output,
        )?;
    }

    // A non-primary definition root loads its System.c4g only AFTER all child
    // definitions (C4Def.cpp:927-968). Direct primary definitions suppress
    // their own System group, as does the scenario-file InitDefs pass.
    if !primary_definition && load_system_groups {
        if let Ok(system) = group.open_child(Path::new("System.c4g")) {
            let components =
                language_packs.component_groups(&system, Some(scenario), scenario_origin);
            if let Ok(sources) =
                load_system_scripts_with_components(&system, &components, languages)
            {
                output.push(CollectedDefinition::SystemScripts(sources));
            }
        }
    }
    Ok(())
}

fn is_rejected_definition_error(error: &ResourceDefinitionError) -> bool {
    matches!(
        error,
        ResourceDefinitionError::MissingDefCoreField(_)
            | ResourceDefinitionError::InvalidCategoryValue(_)
            | ResourceDefinitionError::DefCoreParse(_)
            | ResourceDefinitionError::ActMapParse(_)
            | ResourceDefinitionError::Graphics { .. }
            | ResourceDefinitionError::ColorByOwnerOverlay { .. }
    )
}

fn warn_rejected_definition(group: &Group, error: &impl fmt::Display) {
    tracing::warn!(
        group = %group.root().display(),
        error = %error,
        "definition failed to load; skipping"
    );
}

fn scenario_definition_from_resource(
    resource: ResourceDefinitionData,
    source_group: Option<Group>,
) -> ScenarioDefinition {
    let script_name = source_group
        .as_ref()
        .map(|group| group.root().join("Script.c").to_string_lossy().into_owned());
    let description = resource.description().map(str::to_owned);
    let ResourceDefinitionData {
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
        rank_names,
        rank_base,
        rank_symbol_count,
        clonk_names,
    } = resource;
    let actions = action_map.map(|map| convert_action_map(&map));
    let full_core = core.clone();

    ScenarioDefinition {
        id: core.id,
        name: core.name,
        description,
        clonk_names,
        script: script.combined().to_string(),
        script_name,
        actions,
        crew_member: core.crew_member != 0,
        can_be_base: core.can_be_base,
        movement: MovementProfile::default(),
        category: core.category,
        value: core.value,
        mass: core.mass,
        picture: core.picture.map(DefinitionPicture::from),
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
        rank_names,
        rank_base,
        rank_symbol_count,
        resource_group: source_group,
        components: core
            .components
            .into_iter()
            .map(|component| DefinitionComponent {
                id: component.id,
                count: component.count,
            })
            .collect(),
        line_connect: core.line_connect,
        vertices: core.vertices,
        shape: core.shape,
        core: Some(full_core),
    }
}

fn convert_action_map(map: &ResourceActionMap) -> DefinitionActions {
    let mut specs = HashMap::new();
    let mut physical = Vec::with_capacity(map.actions.len());
    let mut graphics = HashMap::new();
    graphics.insert(
        crate::PHYSICAL_ACTION_GRAPHICS_MARKER.to_string(),
        DefinitionActionGraphics::default(),
    );
    let mut reflections = HashMap::new();
    for (index, (name, definition)) in map.actions.iter().enumerate() {
        let (spec, visuals) = convert_action_definition(definition);
        physical.push((name.clone(), spec.clone()));
        // SetActionByName and FnGetActMapVal both scan the physical ActMap
        // forward, so the first duplicate name wins.
        specs.entry(name.clone()).or_insert(spec);
        graphics
            .entry(name.clone())
            .or_insert_with(|| visuals.clone());
        graphics.insert(
            crate::physical_action_graphics_key(index.min(u32::MAX as usize) as u32),
            visuals,
        );
        reflections
            .entry(name.clone())
            .or_insert_with(|| crate::action::C4ActionReflection::from_resource(name, definition));
    }
    DefinitionActions {
        default_action: map.default_action.clone(),
        specs,
        physical,
        graphics,
        reflections,
    }
}

fn convert_action_definition(
    action: &ResourceActionDefinition,
) -> (ActionSpec, DefinitionActionGraphics) {
    let mut spec = ActionSpec::default();
    if let Some(length) = action.length {
        spec = spec.with_length(length);
    }
    if let Some(next) = &action.next_action {
        spec = spec.with_next(next.clone());
    }
    spec = spec.with_next_index(action.next_action_index);
    if let Some(procedure) = action.procedure.as_deref().and_then(|procedure| {
        clonk_resources::definition::PROCEDURE_NAMES
            .iter()
            .find(|candidate| **candidate == procedure)
    }) {
        spec = spec.with_procedure(*procedure);
    }
    if let Some(delay) = action.delay {
        spec = spec.with_delay(delay);
    }
    if let Some(step) = action.step {
        spec = spec.with_step(step);
    }
    if let Some(phase_call) = &action.phase_call {
        spec = spec.with_phase_call(phase_call.clone());
    }
    if let Some(start_call) = &action.start_call {
        spec = spec.with_start_call(start_call.clone());
    }
    if let Some(end_call) = &action.end_call {
        spec = spec.with_end_call(end_call.clone());
    }
    if let Some(abort_call) = &action.abort_call {
        spec = spec.with_abort_call(abort_call.clone());
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
    // ActMap Attach: the ExecAction default case zeroes dirs and
    // mobilizes instead of applying gravity (C4Object.cpp:5426-5437) —
    // dropping it made every NONE-procedure aimer/rider free-fall.
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
    graphics.facet = action.facet.as_ref().map(convert_action_facet);
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

fn read_group_file_bytes(group: &Group, path: &Path) -> Result<Vec<u8>, ScenarioError> {
    match group.read_file(path) {
        Ok(bytes) => Ok(bytes),
        Err(GroupError::EntryNotFound(_)) => read_file_from_fs(group, path),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            read_file_from_fs(group, path)
        }
        Err(error) => Err(ScenarioError::Resources(error)),
    }
}

fn read_file_from_fs(group: &Group, path: &Path) -> Result<Vec<u8>, ScenarioError> {
    let fallback = group.root().join(path);
    fs::read(&fallback).map_err(|_| ScenarioError::MissingScript {
        path: PathBuf::from(path),
    })
}

#[derive(Debug, Deserialize)]
struct ScenarioManifest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    ticks: Option<u32>,
    #[serde(default)]
    ground_height: Option<i32>,
    #[serde(default)]
    definitions: Vec<DefinitionManifest>,
    #[serde(default)]
    initial_objects: Vec<ObjectManifest>,
    #[serde(default)]
    landscape: Option<LandscapeManifest>,
    #[serde(default)]
    physics: Option<PhysicsManifest>,
    #[serde(default)]
    environment: Option<EnvironmentManifest>,
    #[serde(default)]
    sky: Option<SkyManifest>,
    #[serde(default)]
    script: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DefinitionManifest {
    id: String,
    #[serde(default)]
    name: Option<String>,
    script: String,
    #[serde(default)]
    default_action: Option<String>,
    #[serde(default)]
    actions: HashMap<String, ActionSpec>,
    #[serde(default)]
    crew_member: bool,
    #[serde(default)]
    movement: Option<MovementManifest>,
    #[serde(default)]
    category: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct MovementManifest {
    #[serde(default)]
    float: Option<FloatMovementManifest>,
    #[serde(default)]
    swim: Option<SwimMovementManifest>,
    #[serde(default)]
    walk: Option<WalkMovementManifest>,
    #[serde(default)]
    scale: Option<ScaleMovementManifest>,
    #[serde(default)]
    hangle: Option<HangleMovementManifest>,
    #[serde(default)]
    dig: Option<DigMovementManifest>,
}

#[derive(Debug, Deserialize, Default)]
struct FloatMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct SwimMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct WalkMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct ScaleMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct HangleMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
    #[serde(default)]
    acceleration: Option<i32>,
}

#[derive(Debug, Deserialize, Default)]
struct DigMovementManifest {
    #[serde(default)]
    speed: Option<i32>,
}

impl MovementManifest {
    fn into_profile(self, id: &str) -> Result<MovementProfile, ScenarioError> {
        let mut profile = MovementProfile::default();
        if let Some(float) = self.float {
            if let Some(speed) = float.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("float.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.float_speed = speed;
            }
            if let Some(acceleration) = float.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("float.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.float_acceleration = acceleration;
            }
        }
        if let Some(swim) = self.swim {
            if let Some(speed) = swim.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("swim.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.swim_speed = speed;
            }
            if let Some(acceleration) = swim.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("swim.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.swim_acceleration = acceleration;
            }
        }
        if let Some(walk) = self.walk {
            if let Some(speed) = walk.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("walk.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.walk_speed = speed;
            }
            if let Some(acceleration) = walk.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("walk.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.walk_acceleration = acceleration;
            }
        }
        if let Some(scale) = self.scale {
            if let Some(speed) = scale.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("scale.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.scale_speed = speed;
            }
            if let Some(acceleration) = scale.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("scale.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.scale_acceleration = acceleration;
            }
        }
        if let Some(hangle) = self.hangle {
            if let Some(speed) = hangle.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("hangle.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.hangle_speed = speed;
            }
            if let Some(acceleration) = hangle.acceleration {
                if acceleration < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("hangle.acceleration must be >= 0 (got {acceleration})"),
                    });
                }
                profile.hangle_acceleration = acceleration;
            }
        }
        if let Some(dig) = self.dig {
            if let Some(speed) = dig.speed {
                if speed < 0 {
                    return Err(ScenarioError::InvalidMovement {
                        id: id.to_string(),
                        detail: format!("dig.speed must be >= 0 (got {speed})"),
                    });
                }
                profile.dig_speed = speed;
            }
        }
        Ok(profile)
    }
}

#[derive(Debug, Deserialize)]
struct ObjectManifest {
    definition: String,
    #[serde(default)]
    position: Option<[i32; 2]>,
    #[serde(default)]
    velocity: Option<[i32; 2]>,
    #[serde(default)]
    energy: Option<i32>,
    #[serde(default)]
    owner: Option<i32>,
    #[serde(default)]
    action: Option<ActionManifest>,
    #[serde(default)]
    effects: Vec<EffectManifest>,
    #[serde(default)]
    crew_member: Option<bool>,
    #[serde(default)]
    alive: Option<bool>,
    #[serde(default)]
    status: Option<ObjectStatusSpec>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    category: Option<i32>,
}

#[derive(Debug)]
struct ObjectStatusSpec(ObjectStatus);

impl ObjectStatusSpec {
    fn from_name(name: &str) -> Option<ObjectStatus> {
        if name.eq_ignore_ascii_case("deleted") {
            Some(ObjectStatus::Deleted)
        } else if name.eq_ignore_ascii_case("normal") {
            Some(ObjectStatus::Normal)
        } else if name.eq_ignore_ascii_case("inactive") {
            Some(ObjectStatus::Inactive)
        } else {
            None
        }
    }

    fn from_code(code: i64) -> Option<ObjectStatus> {
        match code {
            0 => Some(ObjectStatus::Deleted),
            1 => Some(ObjectStatus::Normal),
            2 => Some(ObjectStatus::Inactive),
            _ => None,
        }
    }
}

impl From<ObjectStatusSpec> for ObjectStatus {
    fn from(spec: ObjectStatusSpec) -> Self {
        spec.0
    }
}

impl<'de> Deserialize<'de> for ObjectStatusSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StatusVisitor;

        impl<'de> Visitor<'de> for StatusVisitor {
            type Value = ObjectStatusSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(
                    "an object status (\"deleted\", \"normal\", \"inactive\") or numeric code 0/1/2",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ObjectStatusSpec::from_name(value)
                    .map(ObjectStatusSpec)
                    .ok_or_else(|| E::custom(format!("unknown object status `{value}`")))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                ObjectStatusSpec::from_code(value)
                    .map(ObjectStatusSpec)
                    .ok_or_else(|| E::custom(format!("unsupported object status code {value}")))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value > i64::MAX as u64 {
                    return Err(E::custom(format!("unsupported object status code {value}")));
                }
                self.visit_i64(value as i64)
            }
        }

        deserializer.deserialize_any(StatusVisitor)
    }
}

#[derive(Debug, Deserialize)]
struct ActionManifest {
    name: String,
    #[serde(default)]
    phase: Option<i32>,
    #[serde(default)]
    ticks: Option<i32>,
    #[serde(default)]
    data: Option<i32>,
}

impl ActionManifest {
    fn into_state(self) -> ActionState {
        let mut state = ActionState::new(self.name);
        if let Some(phase) = self.phase {
            state.phase = phase;
        }
        if let Some(ticks) = self.ticks {
            state.ticks = ticks;
        }
        if let Some(data) = self.data {
            state.data = data;
        }
        state
    }
}

#[derive(Debug, Deserialize)]
struct EffectManifest {
    name: String,
    #[serde(default = "EffectManifest::default_priority")]
    priority: i32,
    #[serde(default = "EffectManifest::default_interval")]
    interval: i32,
    #[serde(default)]
    timer: i32,
}

impl EffectManifest {
    fn default_priority() -> i32 {
        100
    }

    fn default_interval() -> i32 {
        1
    }

    fn into_state(self) -> EffectState {
        EffectState::new(self.name)
            .with_priority(self.priority)
            .with_interval(self.interval)
            .with_timer(self.timer)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LandscapeManifest {
    Flat { width: u32, height: i32 },
    HeightMap { width: u32, heights: Vec<i32> },
}

impl LandscapeManifest {
    fn into_landscape(self) -> Result<Landscape, ScenarioError> {
        match self {
            LandscapeManifest::Flat { width, height } => Ok(Landscape::flat(width, height)),
            LandscapeManifest::HeightMap { width, heights } => Landscape::new(width, heights)
                .map_err(|error| ScenarioError::InvalidLandscape(error.to_string())),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PhysicsManifest {
    #[serde(default)]
    gravity: Option<i32>,
    #[serde(default)]
    max_fall_speed: Option<i32>,
    #[serde(default)]
    max_rise_speed: Option<i32>,
    #[serde(default)]
    max_horizontal_speed: Option<i32>,
}

impl PhysicsManifest {
    fn into_settings(self) -> Result<PhysicsSettings, ScenarioError> {
        let defaults = PhysicsSettings::default();
        let gravity = self.gravity.unwrap_or(defaults.gravity);
        let max_fall_speed = self.max_fall_speed.unwrap_or(defaults.max_fall_speed);
        let max_rise_speed = self.max_rise_speed.unwrap_or(defaults.max_rise_speed);

        let settings = PhysicsSettings::checked(gravity, max_fall_speed, max_rise_speed)
            .map_err(|detail| ScenarioError::InvalidPhysics(detail.to_string()))?;

        if let Some(max_horizontal_speed) = self.max_horizontal_speed {
            return settings
                .with_max_horizontal_speed(max_horizontal_speed)
                .map_err(|detail| ScenarioError::InvalidPhysics(detail.to_string()));
        }

        Ok(settings)
    }
}

#[derive(Debug, Deserialize)]
struct EnvironmentManifest {
    #[serde(default)]
    wind: Option<i32>,
    #[serde(default)]
    wind_variation: Option<i32>,
    #[serde(default)]
    wind_period: Option<u32>,
    #[serde(default)]
    temperature: Option<i32>,
    #[serde(default)]
    climate: Option<i32>,
    #[serde(default)]
    temperature_variation: Option<i32>,
    #[serde(default)]
    temperature_period: Option<u32>,
    #[serde(default)]
    temperature_phase: Option<u32>,
    #[serde(default)]
    time_of_day: Option<i32>,
    #[serde(default)]
    time_speed: Option<i32>,
    #[serde(default)]
    precipitation: Option<i32>,
    #[serde(default)]
    sky_color: Option<ColorSpec>,
    #[serde(default)]
    season: Option<i32>,
    #[serde(default)]
    year_speed: Option<i32>,
    #[serde(default)]
    temperature_range: Option<i32>,
    #[serde(default)]
    lightning: Option<i32>,
    #[serde(default)]
    meteorite: Option<i32>,
    #[serde(default)]
    volcano: Option<i32>,
    #[serde(default)]
    earthquake: Option<i32>,
    #[serde(default)]
    precipitation_strength: Option<i32>,
    #[serde(default)]
    gamma_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SkyManifest {
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    fade_top: Option<ColorSpec>,
    #[serde(default)]
    fade_bottom: Option<ColorSpec>,
    #[serde(default)]
    scroll_mode: Option<String>,
    #[serde(default)]
    parallax_x: Option<i32>,
    #[serde(default)]
    parallax_y: Option<i32>,
    #[serde(default)]
    xdir: Option<f32>,
    #[serde(default)]
    ydir: Option<f32>,
    #[serde(default)]
    modulation: Option<ColorSpec>,
    #[serde(default)]
    back_color: Option<ColorSpec>,
}

#[derive(Debug)]
struct ColorSpec(RgbColor);

impl ColorSpec {
    fn into_color(self) -> RgbColor {
        self.0
    }
}

impl<'de> Deserialize<'de> for ColorSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ColorVisitor;

        impl<'de> Visitor<'de> for ColorVisitor {
            type Value = ColorSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a hex string #RRGGBB or an array [r, g, b]")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut components = Vec::with_capacity(3);
                while let Some(value) = seq.next_element::<i32>()? {
                    if !(0..=255).contains(&value) {
                        return Err(A::Error::custom(format!(
                            "color components must be between 0 and 255 (got {value})"
                        )));
                    }
                    components.push(value as u8);
                }

                if components.len() != 3 {
                    return Err(A::Error::invalid_length(
                        components.len(),
                        &"array with exactly three entries [r, g, b]",
                    ));
                }

                Ok(ColorSpec(RgbColor::new(
                    components[0],
                    components[1],
                    components[2],
                )))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                parse_hex_color(value).map(ColorSpec).map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        fn parse_hex_color(value: &str) -> Result<RgbColor, String> {
            let trimmed = value.trim();
            let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
            if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!(
                    "expected hex color in RRGGBB format, got `{}`",
                    value
                ));
            }

            let parse_component = |segment: &str| -> Result<u8, String> {
                u8::from_str_radix(segment, 16)
                    .map_err(|_| format!("invalid hex component `{segment}`"))
            };

            let r = parse_component(&hex[0..2])?;
            let g = parse_component(&hex[2..4])?;
            let b = parse_component(&hex[4..6])?;
            Ok(RgbColor::new(r, g, b))
        }

        deserializer.deserialize_any(ColorVisitor)
    }
}

impl EnvironmentManifest {
    fn into_settings(self) -> EnvironmentSettings {
        let mut settings = EnvironmentSettings::new(self.wind.unwrap_or(0));
        if let Some(variation) = self.wind_variation {
            let period = self.wind_period.unwrap_or(120);
            settings = settings.with_wind_variation(variation, period);
        }
        if let Some(climate) = self.climate {
            settings = settings.with_climate(climate);
        }
        if let Some(temperature) = self.temperature {
            settings = settings.with_temperature(temperature);
        }
        if self.temperature_variation.is_some()
            || self.temperature_period.is_some()
            || self.temperature_phase.is_some()
        {
            let variation = self.temperature_variation.unwrap_or(0);
            let period = self.temperature_period.unwrap_or(600);
            let phase = self.temperature_phase.unwrap_or(0);
            settings = settings.with_temperature_cycle(variation, period, phase);
        }
        if let Some(time_of_day) = self.time_of_day {
            settings = settings.with_time_of_day(time_of_day);
        }
        if let Some(time_speed) = self.time_speed {
            settings = settings.with_time_speed(time_speed);
        }
        if let Some(precipitation) = self.precipitation {
            settings = settings.with_precipitation(precipitation);
            if self.precipitation_strength.is_none() {
                settings = settings.with_precipitation_strength(precipitation);
            }
        }
        if let Some(color) = self.sky_color {
            settings = settings.with_sky_color(color.into_color());
        }
        if let Some(season) = self.season {
            settings = settings.with_season(season);
        }
        if let Some(year_speed) = self.year_speed {
            settings = settings.with_year_speed(year_speed);
        }
        if let Some(range) = self.temperature_range {
            settings = settings.with_temperature_range(range);
        }
        if let Some(lightning) = self.lightning {
            settings = settings.with_lightning(lightning);
        }
        if let Some(meteorite) = self.meteorite {
            settings = settings.with_meteorite(meteorite);
        }
        if let Some(volcano) = self.volcano {
            settings = settings.with_volcano(volcano);
        }
        if let Some(earthquake) = self.earthquake {
            settings = settings.with_earthquake(earthquake);
        }
        if let Some(strength) = self.precipitation_strength {
            settings = settings.with_precipitation_strength(strength);
        }
        if let Some(enabled) = self.gamma_enabled {
            settings = if enabled {
                settings.with_gamma_enabled()
            } else {
                settings.with_gamma_disabled()
            };
        }
        settings
    }
}

impl SkyManifest {
    fn into_config(self, group: &Group) -> Result<SkyConfig, ScenarioError> {
        let mut settings = SkySettings::default();
        let mut surface_image = None;

        if let Some(surface_name) = self.surface {
            let path = PathBuf::from(&surface_name);
            let bytes = match group.read_file(&path) {
                Ok(bytes) => bytes,
                Err(GroupError::EntryNotFound(_)) => {
                    return Err(ScenarioError::SkySurfaceMissing { path });
                }
                Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ScenarioError::SkySurfaceMissing { path });
                }
                Err(error) => return Err(ScenarioError::Resources(error)),
            };

            let decoded =
                load_from_memory(&bytes).map_err(|source| ScenarioError::SkySurfaceDecode {
                    path: path.clone(),
                    source,
                })?;
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            let pixels = rgba.into_raw();
            settings = settings.with_surface(width, height);
            surface_image = Some(Arc::new(GraphicsImage::new(width, height, pixels)));
        }

        if let Some(color) = self.fade_top {
            settings.fade_top = color.into_color();
        }
        if let Some(color) = self.fade_bottom {
            settings.fade_bottom = color.into_color();
        }
        if let Some(mode) = self.scroll_mode {
            settings.parallax_mode = parse_scroll_mode(&mode)?;
        }
        if let Some(value) = self.parallax_x {
            settings.parallax_x = value;
        }
        if let Some(value) = self.parallax_y {
            settings.parallax_y = value;
        }
        if let Some(value) = self.xdir {
            settings.base_xdir = value;
        }
        if let Some(value) = self.ydir {
            settings.base_ydir = value;
        }
        if let Some(color) = self.modulation {
            settings.modulation = Some(rgb_to_bgr_u32(color.into_color()));
        }
        if let Some(color) = self.back_color {
            let back_color = rgb_to_bgr_u32(color.into_color());
            settings.back_color = Some(back_color);
            settings.back_color_raw = back_color;
        }

        Ok(SkyConfig {
            settings,
            surface: surface_image,
        })
    }
}

fn parse_scroll_mode(value: &str) -> Result<SkyParallaxMode, ScenarioError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(SkyParallaxMode::Fixed);
    }
    if let Ok(code) = trimmed.parse::<i32>() {
        return match code {
            0 => Ok(SkyParallaxMode::Fixed),
            1 => Ok(SkyParallaxMode::Wind),
            2 => Ok(SkyParallaxMode::Parallax),
            other => Err(ScenarioError::InvalidSky(format!(
                "unknown sky scroll mode code {other}"
            ))),
        };
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "fixed" => Ok(SkyParallaxMode::Fixed),
        "wind" => Ok(SkyParallaxMode::Wind),
        "parallax" => Ok(SkyParallaxMode::Parallax),
        other => Err(ScenarioError::InvalidSky(format!(
            "unknown sky scroll mode `{other}`"
        ))),
    }
}

fn rgb_to_bgr_u32(color: RgbColor) -> u32 {
    u32::from(color.b) | (u32::from(color.g) << 8) | (u32::from(color.r) << 16)
}

#[cfg(test)]
fn write_test_definition_graphics(path: &Path) {
    image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
        .save(path.join("Graphics.png"))
        .expect("write definition graphics");
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::bmp::BmpEncoder, ColorType, Rgba, RgbaImage};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{subscriber, Level};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    #[test]
    fn initial_network_scenario_matches_pristine_cpp_tutorial01_differential() {
        // C4GameSave::SaveCore + C4GameSaveNetwork::AdjustCore and
        // C4Scenario::CompileFunc (C4GameSave.cpp:58-108,612-617;
        // C4Scenario.cpp:100-134,164-439).
        let source = include_str!("../../../content/Tutorial.c4f/Tutorial01.c4s/Scenario.txt");
        let scenario = scenario_with_retained_legacy_core(source);
        let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];

        let actual = scenario
            .serialize_initial_network_scenario(
                "A Clonk",
                &definitions,
                "",
                "",
                "Tutorial.c4f/Tutorial01.c4s",
            )
            .expect("legacy initial network scenario serializes");

        assert_eq!(
            actual,
            TUTORIAL01_PRISTINE_CPP_INITIAL_NETWORK_SCENARIO.as_bytes()
        );
    }

    #[test]
    fn material_and_texture_names_use_c4m_max_name_bytes() {
        let material_prefix = b"MaterialPrefix\x80";
        let texture_prefix = b"TexturePrefixX\x81";
        assert_eq!(material_prefix.len(), 15);
        assert_eq!(texture_prefix.len(), 15);

        let mut material_long_a = material_prefix.to_vec();
        material_long_a.extend_from_slice(b"First");
        let mut material_long_b = material_prefix.to_vec();
        material_long_b.extend_from_slice(b"Second");
        let mut texture_long_a = texture_prefix.to_vec();
        texture_long_a.extend_from_slice(b"One");
        let mut texture_long_b = texture_prefix.to_vec();
        texture_long_b.extend_from_slice(b"Two");

        let material_source = |name: &[u8], density: i32| {
            let mut source = b"[Material]\nName=".to_vec();
            source.extend_from_slice(name);
            source.extend_from_slice(format!("\nDensity={density}\nTextureOverlay=").as_bytes());
            source.extend_from_slice(texture_prefix);
            source.push(b'\n');
            source
        };
        let mut texmap_source = b"20=".to_vec();
        texmap_source.extend_from_slice(material_prefix);
        texmap_source.push(b'-');
        texmap_source.extend_from_slice(texture_prefix);
        texmap_source.extend_from_slice(b"\n21=");
        texmap_source.extend_from_slice(&material_long_a);
        texmap_source.push(b'-');
        texmap_source.extend_from_slice(&texture_long_a);
        texmap_source.push(b'\n');

        let parsed_texmap = clonk_resources::texmap::TextureMap::parse_bytes(&texmap_source);
        assert_eq!(
            clonk_script::c4_string_bytes(&parsed_texmap.entry(21).unwrap().material),
            material_long_a,
            "TexMap names remain raw and unbounded"
        );
        assert_eq!(
            clonk_script::c4_string_bytes(&parsed_texmap.entry(21).unwrap().texture),
            texture_long_a,
            "non-UTF-8 TexMap bytes survive without replacement"
        );

        let mut materials = clonk_resources::MutableGroup::new("Material.c4g");
        materials
            .add_file("A.c4m", material_source(&material_long_a, 61))
            .unwrap();
        materials
            .add_file("B.c4m", material_source(&material_long_b, 72))
            .unwrap();
        materials.add_file("TexMap.txt", texmap_source).unwrap();
        let texture_bitmap = encode_indexed_bmp(&[&[0u8]]);
        for stem in [&texture_long_a, &texture_long_b] {
            let mut filename = stem.to_vec();
            filename.extend_from_slice(b".bmp");
            materials
                .add_file_bytes_with_metadata(filename, texture_bitmap.clone(), 1, false)
                .unwrap();
        }
        materials
            .add_file(
                "Mislabeled.bmp",
                include_bytes!("../../../content/Material.c4g/Snow.png").to_vec(),
            )
            .unwrap();

        let mut scenario = clonk_resources::MutableGroup::new("ByteNames.c4s");
        scenario
            .add_packed_child_with_metadata(
                "Material.c4g",
                materials.pack_raw().unwrap(),
                0,
                1,
                false,
            )
            .unwrap();
        let group =
            Group::from_raw_memory(PathBuf::from("ByteNames.c4s"), scenario.pack_raw().unwrap())
                .unwrap();
        let classifier =
            build_map_pixel_classifier(&group, &FileSystemResolver { roots: Vec::new() })
                .unwrap()
                .unwrap();
        let library = classifier.material_library().unwrap();
        let names = library
            .iter()
            .map(|material| clonk_script::c4_string_bytes(material.name()))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![material_prefix.to_vec(), material_prefix.to_vec()]
        );
        for material in library.iter() {
            assert_eq!(
                material.value("Name").map(clonk_script::c4_string_bytes),
                Some(material_prefix.to_vec()),
                "compiled Name reflection matches the fixed live core"
            );
        }
        assert_eq!(
            library
                .get(&clonk_script::c4_string_from_bytes(material_prefix))
                .and_then(|material| material.int("Density")),
            Some(61),
            "the first same-load fixed-name collision owns name lookup"
        );
        assert!(
            library
                .get(&clonk_script::c4_string_from_bytes(&material_long_a))
                .is_none(),
            "whole-name lookup does not prefix-match a fixed identity"
        );

        assert_eq!(
            classifier
                .state
                .texture_inventory
                .iter()
                .map(|name| clonk_script::c4_string_bytes(name))
                .collect::<Vec<_>>(),
            vec![texture_prefix.to_vec(), texture_prefix.to_vec()],
            "long candidates admit before truncation, but a PNG payload named BMP is rejected"
        );
        assert_eq!(
            classifier.state.densities[20], 61,
            "the fixed TexMap identity resolves the first material collision"
        );
        assert!(classifier.state.material_names[20].is_some());
        assert!(
            classifier.state.material_names[21].is_none(),
            "an unbounded TexMap pair does not prefix-match fixed identities"
        );
    }

    // Bodies live in byte-verbatim contiguous parts so the module — and
    // every test id it exports — stays exactly as it was.
    include!("scenario/tests/part_01.rs");
    include!("scenario/tests/part_02.rs");
    include!("scenario/tests/part_03.rs");
    include!("scenario/tests/part_04.rs");
    include!("scenario/tests/part_05.rs");
    include!("scenario/tests/part_06.rs");
    include!("scenario/tests/part_07.rs");
    include!("scenario/tests/part_08.rs");
}

#[cfg(test)]
mod game_start_sync {
    use super::*;
    use tempfile::tempdir;

    struct ProbeResolver {
        roots: Vec<std::path::PathBuf>,
    }
    impl LegacyDefinitionResolver for ProbeResolver {
        fn resolve_definition_groups(
            &self,
            scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            let normalized = identifier.replace('\\', "/");
            let path = std::path::Path::new(&normalized);
            let mut groups = Vec::new();
            if let Ok(child) = scenario.open_child(path) {
                groups.push(child);
            }
            for root in &self.roots {
                let candidate = root.join(path);
                if candidate.exists() {
                    groups.push(Group::open(&candidate)?);
                }
            }
            Ok(groups)
        }
    }

    fn write_palm_def(defs: &std::path::Path) {
        let palm = defs.join("Palm.c4d");
        std::fs::create_dir_all(&palm).expect("palm dir");
        std::fs::write(
            palm.join("DefCore.txt"),
            "[DefCore]\nid=PALM\nName=Palm\nCategory=1\nWidth=40\nHeight=56\nOffset=-20,-28\nVertices=1\nVertexY=22\n",
        )
        .expect("defcore");
        std::fs::write(
            palm.join("ActMap.txt"),
            "[Action]\nName=Still\nDelay=4\nLength=1\nNextAction=Still\nStartCall=Still\n\n\
             [Action]\nName=Breeze\nDelay=2\nLength=20\nNextAction=Breeze\nStartCall=Breeze\n",
        )
        .expect("actmap");
        // The real Palm1/Tree StartCalls flip Breeze<->Still by wind
        // (Objects.c4d/Vegetation.c4d): if a loaded spawn fired StartCall,
        // the saved Breeze would collapse to Still like the live bug.
        std::fs::write(
            palm.join("Script.c"),
            "#strict\nfunc Still() { return(1); }\nfunc Breeze() { SetAction(\"Still\"); return(1); }\n",
        )
        .expect("script");
        write_test_definition_graphics(&palm);
    }

    fn load(dir: &std::path::Path) -> (Engine, Scenario) {
        let resolver = ProbeResolver {
            roots: vec![dir.to_path_buf()],
        };
        let scenario_dir = dir.join("Sync.c4s");
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(11);
        scenario.apply(&mut engine).expect("scenario applies");
        (engine, scenario)
    }

    fn write_scenario(dir: &std::path::Path, objects: &str) {
        let scenario_dir = dir.join("Sync.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Sync\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("core");
        std::fs::write(scenario_dir.join("Objects.txt"), objects).expect("objects");
    }

    // C4Game::Init runs SyncClearance + Synchronize AFTER InitGame and
    // BEFORE InitPlayers (C4Game.cpp:474-475): every object's fixed
    // position collapses to itofix(x,y,r) (C4Object::SyncClearance,
    // C4Object.cpp:3803-3815) — grown trees carry y != fixtoi(fix_y) in
    // the savefile because DoCon adjusts y without touching fix_y — and
    // the loaded action restores only when the name resolves in the
    // ActMap (C4Object.cpp:2840-2849); C4Action::Default leaves Name
    // empty, so records without Action= stay ActIdle (no def default:
    // C++ has no such concept). GoldRush oracle: TRE2 #3 (no Action=,
    // FixY 28px below Y) sits at (204,258) Idle in C++; PLM1 #42 keeps
    // Action=Breeze Phase=18. Saved active rows carry Size=FullCon; omitting
    // it compiles Con=0 and correctly forces even a resolved action to Idle.
    #[test]
    fn loaded_objects_sync_clearance_and_action_rules_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(
            dir.path(),
            "[Object]\nid=PALM\nNumber=3\nCategory=1\nSize=100000\nX=204\nY=258\nFixX=f1129054208\nFixY=f1133445120\n\n\
             [Object]\nid=PALM\nNumber=42\nCategory=1\nSize=100000\nX=981\nY=280\nAction=Breeze\nDir=1\nActionTime=694\nPhase=18\nPhaseDelay=4\nYDir=f1149998051\n\n\
             [Object]\nid=PALM\nNumber=43\nCategory=1\nSize=100000\nX=100\nY=100\nAction=Stand\nPhase=2\n",
        );
        let (engine, _) = load(dir.path());

        let (_, action, _phase, position, fix_y, ..) =
            engine.debug_object_by_id(3).expect("tree exists");
        assert_eq!(
            action,
            crate::action::DEFAULT_ACTION_NAME,
            "no Action= -> ActIdle"
        );
        assert_eq!(position, Vector2::new(204, 258), "saved center kept");
        assert_eq!(
            fix_y, 258,
            "SyncClearance collapses fix to itofix(y) (C4Object.cpp:3810)"
        );

        let (_, action, phase, ..) = engine.debug_object_by_id(42).expect("palm exists");
        assert_eq!(action, "Breeze", "resolved saved action survives");
        assert_eq!(phase, 18, "saved phase survives");

        let (_, action, phase, ..) = engine.debug_object_by_id(43).expect("third exists");
        assert_eq!(
            action,
            crate::action::DEFAULT_ACTION_NAME,
            "unresolvable saved action (CCAN Stand) falls to Idle, not a def default"
        );
        assert_eq!(
            phase, 2,
            "failed saved-action lookup leaves the compiled phase untouched"
        );
    }

    #[test]
    fn loaded_actions_restore_signed_counters_and_cpp_data_rules() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");
        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        std::fs::write(
            loaded.join("ActMap.txt"),
            "[Action]\nName=Passive\nDelay=0\nLength=1\n\n\
             [Action]\nName=Attached\nProcedure=ATTACH\nDelay=0\nLength=1\n",
        )
        .expect("actmap");
        std::fs::write(
            loaded.join("Script.c"),
            "#strict\npublic func ReadLoadedAction() { return [GetAction(), GetObjectVal(\"Action\"), GetActTime(), GetObjectVal(\"PhaseDelay\"), GetObjectVal(\"ActionData\")]; }\n",
        )
        .expect("script");
        write_scenario(
            dir.path(),
            "[Object]\nid=LOAD\nNumber=100\nCategory=16\nSize=100000\nX=10\nY=20\nFixX=F999424\nFixY=F1327104\nAction=Passive\nActionTime=-7\nPhase=-2\nPhaseDelay=-3\nActionData=41\n\n\
             [Object]\nid=LOAD\nNumber=101\nCategory=16\nSize=100000\nAction=Attached\nActionTime=-8\nPhase=-4\nPhaseDelay=-5\nActionData=42\n\n\
             [Object]\nid=LOAD\nNumber=102\nCategory=16\nSize=100000\nX=30\nY=40\nFixX=F2015232\nFixY=F2654208\nAction=Missing\nActionTime=-9\nPhase=-6\nPhaseDelay=-7\nActionData=43\n\n\
             [Object]\nid=LOAD\nNumber=103\nCategory=16\nSize=50000\nAction=Attached\nActionTime=-10\nPhase=-8\nPhaseDelay=-9\nActionData=44\n",
        );

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(dir.path().join("Sync.c4s"), &resolver)
            .expect("scenario loads");
        let mut engine = Engine::with_seed(11);
        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("pre-final-sync scenario applies");
        let passive = engine
            .object_snapshot(ObjectId::new(100))
            .expect("passive object");
        assert_eq!(passive.action.name, "Passive");
        assert_eq!(
            (
                passive.action.time,
                passive.action.phase,
                passive.action.ticks
            ),
            (-7, -2, -3)
        );
        assert_eq!(
            passive.action.data, 41,
            "ActIdle -> DFA_NONE preserves Action.Data"
        );
        let passive_index = engine
            .find_object_index(ObjectId::new(100))
            .expect("passive object is live");
        assert_eq!(
            engine.objects[passive_index].fixed_position,
            crate::math::FixedVec2::from_ints(10, 20),
            "successful load-time SetAction resynchronizes FixX/FixY"
        );

        let attached = engine
            .object_snapshot(ObjectId::new(101))
            .expect("attached object");
        assert_eq!(attached.action.name, "Attached");
        assert_eq!(
            (
                attached.action.time,
                attached.action.phase,
                attached.action.ticks
            ),
            (-8, -4, -5)
        );
        assert_eq!(
            attached.action.data, 0,
            "ActIdle -> non-NONE procedure clears Action.Data"
        );

        let missing = engine
            .object_snapshot(ObjectId::new(102))
            .expect("missing-action object");
        assert_eq!(missing.action.name, crate::action::DEFAULT_ACTION_NAME);
        assert_eq!(missing.action.act_map_index, None);
        assert_eq!(missing.action.raw_name.as_deref(), Some("Missing"));
        assert_eq!(
            (
                missing.action.time,
                missing.action.phase,
                missing.action.ticks
            ),
            (-9, -6, -7)
        );
        assert_eq!(missing.action.data, 43);
        let missing_index = engine
            .find_object_index(ObjectId::new(102))
            .expect("missing-action object is live");
        assert_eq!(
            engine.objects[missing_index].fixed_position,
            crate::math::FixedVec2 {
                x: crate::math::C4Fixed::from_raw(2_015_232),
                y: crate::math::C4Fixed::from_raw(2_654_208),
            },
            "failed SetActionByName retains compiled FixX/FixY"
        );

        let partial = engine
            .object_snapshot(ObjectId::new(103))
            .expect("partial object");
        assert_eq!(partial.action.name, crate::action::DEFAULT_ACTION_NAME);
        assert_eq!(partial.action.compiled_name(), "");
        assert_eq!(
            (
                partial.action.time,
                partial.action.phase,
                partial.action.ticks
            ),
            (-10, -8, -9)
        );
        assert_eq!(
            partial.action.data, 44,
            "incomplete-object coercion remains DFA_NONE -> DFA_NONE"
        );

        assert_eq!(
            engine
                .call_object_function(missing_index, "ReadLoadedAction", Vec::new())
                .expect("raw action probe succeeds"),
            clonk_script::Value::Array(vec![
                clonk_script::Value::String("Idle".to_string().into()),
                clonk_script::Value::String("Missing".to_string().into()),
                clonk_script::Value::Int(-9),
                clonk_script::Value::Int(-7),
                clonk_script::Value::Int(43),
            ])
        );

        let encoded = serde_json::to_string(&missing).expect("snapshot serializes");
        let decoded: crate::ObjectSnapshot =
            serde_json::from_str(&encoded).expect("snapshot deserializes");
        assert_eq!(decoded.action.raw_name.as_deref(), Some("Missing"));
    }

    #[test]
    fn loaded_action_names_truncate_to_the_cpp_30_native_byte_buffer() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");

        // Twenty-eight ASCII bytes plus the two-byte UTF-8 spelling of `é`
        // fill C4Action::Name exactly. The suffix exists only in the source
        // file and must be discarded before SetActionByName runs.
        let matching_name = format!("{}é", "M".repeat(28));
        let unresolved_name = format!("{}é", "U".repeat(28));
        assert_eq!(clonk_script::c4_string_bytes(&matching_name).len(), 30);
        assert_eq!(clonk_script::c4_string_bytes(&unresolved_name).len(), 30);

        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        std::fs::write(
            loaded.join("ActMap.txt"),
            format!("[Action]\nName={matching_name}\nDelay=0\nLength=1\n"),
        )
        .expect("actmap");
        std::fs::write(
            loaded.join("Script.c"),
            "#strict 2\nfunc ReadRawAction() { return GetObjectVal(\"Action\"); }\n",
        )
        .expect("script");
        write_scenario(
            dir.path(),
            &format!(
                "[Object]\nid=LOAD\nNumber=100\nCategory=16\nSize=100000\nX=10\nY=20\nFixX=F999424\nFixY=F1327104\nAction={matching_name}TRAILING\n\n\
                 [Object]\nid=LOAD\nNumber=101\nCategory=16\nSize=100000\nX=30\nY=40\nFixX=F2015232\nFixY=F2654208\nAction={unresolved_name}TRAILING\n"
            ),
        );

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(dir.path().join("Sync.c4s"), &resolver)
            .expect("scenario loads");
        let mut engine = Engine::with_seed(11);
        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("pre-final-sync scenario applies");

        let matched_index = engine
            .find_object_index(ObjectId::new(100))
            .expect("matched object exists");
        let matched = &engine.objects[matched_index];
        assert_eq!(matched.state.action.name, matching_name);
        assert_eq!(matched.state.action.raw_name, None);
        assert_eq!(
            matched.fixed_position,
            crate::math::FixedVec2::from_ints(10, 20),
            "the truncated physical name resolves and SetAction resynchronizes FixX/FixY"
        );
        assert_eq!(
            engine
                .call_object_function(matched_index, "ReadRawAction", Vec::new())
                .expect("matched raw action reads"),
            clonk_script::Value::String(matching_name.clone().into())
        );

        let unresolved_index = engine
            .find_object_index(ObjectId::new(101))
            .expect("unresolved object exists");
        let unresolved = &engine.objects[unresolved_index];
        assert_eq!(
            unresolved.state.action.name,
            crate::action::DEFAULT_ACTION_NAME
        );
        assert_eq!(
            unresolved.state.action.raw_name.as_deref(),
            Some(unresolved_name.as_str()),
            "a failed lookup retains only the compiled 30-byte buffer"
        );
        assert_eq!(
            unresolved.fixed_position,
            crate::math::FixedVec2 {
                x: crate::math::C4Fixed::from_raw(2_015_232),
                y: crate::math::C4Fixed::from_raw(2_654_208),
            },
            "the unresolved truncated name leaves the serialized fixed position untouched"
        );
        assert_eq!(
            engine
                .call_object_function(unresolved_index, "ReadRawAction", Vec::new())
                .expect("unresolved raw action reads"),
            clonk_script::Value::String(unresolved_name.into())
        );
    }

    #[test]
    fn objects_txt_missing_action_targets_are_null_before_scenario_callbacks() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");
        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        std::fs::write(
            loaded.join("Script.c"),
            "#strict\nlocal seen_target1, seen_target2;\npublic func CaptureTargets() { seen_target1 = GetActionTarget(0); seen_target2 = GetActionTarget(1); return 1; }\n",
        )
        .expect("script");
        write_scenario(
            dir.path(),
            "[Object]\nid=LOAD\nNumber=100\nCategory=16\nAction=Idle\nActionTarget1=999\nActionTarget2=1000\n",
        );
        let scenario_dir = dir.path().join("Sync.c4s");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nfunc Initialize() { var obj = FindObject(LOAD); obj->CaptureTargets(); return 1; }\n",
        )
        .expect("scenario script");

        let (engine, _) = load(dir.path());
        let object = engine
            .object_snapshot(ObjectId::new(100))
            .expect("loaded object");
        assert_eq!(object.action.target, None);
        assert_eq!(object.action.target2, None);
        assert_eq!(
            object.local_vars.get("seen_target1"),
            Some(&clonk_script::Value::Nil),
            "scenario Initialize runs after ActionTarget1 denumeration"
        );
        assert_eq!(
            object.local_vars.get("seen_target2"),
            Some(&clonk_script::Value::Nil),
            "scenario Initialize runs after ActionTarget2 denumeration"
        );
    }

    #[test]
    fn objects_txt_action_targets_accept_the_old_enumeration_offset() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let loaded = defs.join("Loaded.c4d");
        std::fs::create_dir_all(&loaded).expect("definition dir");
        std::fs::write(
            loaded.join("DefCore.txt"),
            "[DefCore]\nid=LOAD\nName=Loaded\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&loaded);
        write_scenario(
            dir.path(),
            "[Object]\nid=LOAD\nNumber=100\nCategory=16\nActionTarget1=1000000042\nActionTarget2=1000000043\n\n\
             [Object]\nid=LOAD\nNumber=42\nCategory=16\n",
        );

        let (engine, _) = load(dir.path());
        let holder = engine
            .object_snapshot(ObjectId::new(100))
            .expect("holder object");
        assert_eq!(
            holder.action.raw_name.as_deref(),
            Some(""),
            "a missing Action= retains C4Action's empty compiled Name buffer"
        );
        assert_eq!(holder.action.target, Some(ObjectId::new(42)));
        assert_eq!(holder.action.target2, None);
    }

    #[test]
    fn network_apply_defers_cpp_final_sync_until_status_commit() {
        // C4Game::Init performs InitGame before Network.FinalInit; only after
        // every client reaches and acknowledges GO does FinalInit run
        // SyncClearance + Synchronize (pristine 9ffa0a5d src/C4Game.cpp:457-478;
        // src/C4Network2.cpp:558-615).
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(
            dir.path(),
            "[Object]\nid=PALM\nNumber=3\nCategory=1\nX=15\nY=5\nFixX=F999424\nFixY=F327680\n",
        );
        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with(dir.path().join("Sync.c4s"), &resolver)
            .expect("scenario loads");
        let mut engine = Engine::with_seed(11);

        scenario
            .apply_before_network_final_init(&mut engine)
            .expect("network InitGame phase applies");
        let object = engine
            .objects
            .iter()
            .find(|object| object.id == ObjectId::new(3))
            .expect("loaded object exists");
        assert_eq!(
            object.fixed_position.x.val(),
            999_424,
            "network InitGame preserves the saved sub-pixel position before GO commits"
        );

        engine
            .game_start_synchronize()
            .expect("network final synchronization succeeds");
        let object = engine
            .objects
            .iter()
            .find(|object| object.id == ObjectId::new(3))
            .expect("loaded object survives final sync");
        assert_eq!(
            object.fixed_position.x.val(),
            crate::math::itofix(15).val(),
            "Network.FinalInit performs the delayed SyncClearance at the GO barrier"
        );
    }

    // C4GameObjects::Load removes inactive rows from the active list before
    // UpdateFaces, so they do not construct or put a solid mask until
    // StatusActivate runs UpdateFace(true). StatusDeactivate does not remove
    // an existing mask.
    #[test]
    fn legacy_loaded_inactive_object_does_not_put_solid_mask_until_activated() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let gate = defs.join("Gate.c4d");
        std::fs::create_dir_all(&gate).expect("gate dir");
        std::fs::write(
            gate.join("DefCore.txt"),
            "[DefCore]\nid=GATE\nName=Gate\nCategory=2\nWidth=1\nHeight=1\nOffset=0,0\nSolidMask=0,0,1,1,0,0\n",
        )
        .expect("defcore");
        std::fs::write(
            gate.join("Script.c"),
            "#strict\npublic func ActivateMask() { return SetObjectStatus(1); }\n\
             public func DeactivateMask() { return SetObjectStatus(2); }\n",
        )
        .expect("script");
        write_test_definition_graphics(&gate);

        let scenario_dir = dir.path().join("Sync.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Inactive mask\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("scenario core");
        image::GrayImage::from_pixel(4, 4, image::Luma([0]))
            .save(scenario_dir.join("Landscape.bmp"))
            .expect("landscape");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GATE\nNumber=61\nStatus=2\nCategory=2\nX=10\nY=10\nSize=100000\nWidth=1\nHeight=1\nOffset=0,0\n\n\
             [Object]\nid=GATE\nNumber=62\nStatus=1\nCategory=2\nX=20\nY=10\nSize=100000\nWidth=1\nHeight=1\nOffset=0,0\n",
        )
        .expect("objects");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# dynamic slots only\n").expect("texmap");
        std::fs::write(
            materials.join("Vehicle.c4m"),
            "[Material]\nName=Vehicle\nDensity=100\nTextureOverlay=Smooth\n",
        )
        .expect("vehicle material");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
            .save(materials.join("Smooth.png"))
            .expect("texture");

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(61);
        scenario.apply(&mut engine).expect("scenario applies");
        let id = ObjectId::new(61);
        let loaded_normal = ObjectId::new(62);

        assert_eq!(engine.debug_solid_mask_is_put(id.as_u64()), Some(false));
        assert_eq!(engine.debug_landscape_byte(10, 10), Some(0));
        let index = engine.find_object_index(id).expect("inactive gate exists");
        assert_eq!(
            engine.objects[index].solid_mask_instance_sequence, None,
            "the load path must not allocate an ordering slot"
        );
        assert_eq!(
            engine.debug_solid_mask_is_put(loaded_normal.as_u64()),
            Some(true),
            "loaded normal objects still receive the initial UpdateFaces pass"
        );
        assert_eq!(
            engine.debug_landscape_material_name(20, 10).as_deref(),
            Some("Vehicle")
        );

        assert_eq!(
            engine
                .call_object_function(index, "ActivateMask", Vec::new())
                .expect("activation executes"),
            clonk_script::Value::Bool(true)
        );
        assert_eq!(engine.debug_solid_mask_is_put(id.as_u64()), Some(true));
        assert_eq!(
            engine.debug_landscape_material_name(10, 10).as_deref(),
            Some("Vehicle")
        );
        let index = engine.find_object_index(id).expect("active gate exists");
        let activated_sequence = engine.objects[index]
            .solid_mask_instance_sequence
            .expect("activation allocates the mask ordering slot");

        assert_eq!(
            engine
                .call_object_function(index, "DeactivateMask", Vec::new())
                .expect("deactivation executes"),
            clonk_script::Value::Bool(true)
        );
        assert_eq!(
            engine.debug_solid_mask_is_put(id.as_u64()),
            Some(true),
            "runtime deactivation retains the existing mask"
        );
        assert_eq!(
            engine.debug_landscape_material_name(10, 10).as_deref(),
            Some("Vehicle")
        );
        let index = engine.find_object_index(id).expect("inactive gate remains");
        assert_eq!(
            engine.objects[index].solid_mask_instance_sequence,
            Some(activated_sequence),
            "runtime deactivation preserves the existing mask ordering slot"
        );
    }

    // C4Object::CompileFunc reads SolidMask= with DEFAULT Def->SolidMask
    // (C4Object.cpp:2770): a saved 0,0,0,0,0,0 means the object's solid
    // mask is OFF (opened gates/doors save that way); the def's mask must
    // not resurrect it. FnSetSolidMask (C4Script.cpp:271-278) drives the
    // same per-object rect at runtime.
    #[test]
    fn objects_txt_solid_mask_overrides_the_definition_mask() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let gate = defs.join("Gate.c4d");
        std::fs::create_dir_all(&gate).expect("gate dir");
        std::fs::write(
            gate.join("DefCore.txt"),
            "[DefCore]\nid=GATE\nName=Gate\nCategory=2\nWidth=10\nHeight=40\nOffset=-5,-20\nSolidMask=0,0,10,40,0,0\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&gate);
        write_scenario(
            dir.path(),
            "[Object]\nid=GATE\nNumber=7\nCategory=2\nX=50\nY=50\nSolidMask=0,0,0,0,0,0\n\n\
             [Object]\nid=GATE\nNumber=8\nCategory=2\nX=90\nY=50\n",
        );
        let (engine, _) = load(dir.path());

        // The 1x1 fixture bitmap is too small for the def-level 10x40 mask,
        // so like C++ that mask never activates; the loader state is the pin.
        let overrides = engine.debug_solid_mask_override(7);
        assert_eq!(
            overrides,
            Some(Some((0, 0, 0, 0))),
            "saved SolidMask=0,0,0,0,0,0 turns the mask OFF (C4Object.cpp:2770)"
        );
        assert_eq!(
            engine.debug_solid_mask_override(8),
            Some(None),
            "no saved key keeps the definition default"
        );
    }

    // GoldRush DoInitialize (Script.c:33-35) pins unowned crew NPCs from
    // the SCENARIO-SCRIPT context: `while(pObj = FindObjectOwner(0,-1,
    // 0,0,0,0,OCF_CrewMember,0,0,pObj)) AddEffect("StayThere",...)`.
    // The Fx handlers are scenario GLOBALS (Script.c:553-564) — resolved
    // through GetFuncRecursive in C++ (C4Effect.cpp:31-40).
    #[test]
    fn scenario_script_pins_unowned_crew_with_stay_there_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let npc = defs.join("Npc.c4d");
        std::fs::create_dir_all(&npc).expect("npc dir");
        std::fs::write(
            npc.join("DefCore.txt"),
            "[DefCore]\nid=NPCX\nName=Npc\nCategory=66056\nCrewMember=1\nWidth=8\nHeight=20\nOffset=-4,-10\n",
        )
        .expect("npc core");
        write_test_definition_graphics(&npc);

        let scenario_dir = dir.path().join("Sync.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Sync\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=NPCX\nNumber=30\nCategory=66056\nX=50\nY=50\nAlive=1\n\n\
             [Object]\nid=NPCX\nNumber=31\nCategory=66056\nX=90\nY=50\nAlive=1\n",
        )
        .expect("objects");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\n\
             protected func InitializePlayer(iPlr) {\n\
               var i, pObj;\n\
               while(pObj = FindObjectOwner(0,-1,0,0,0,0,OCF_CrewMember,0,0,pObj))\n\
                 AddEffect(\"StayThere\",pObj,1,35,pObj);\n\
               return(1);\n\
             }\n\
             global func FxStayThereStart(pTarget, iNumber, fTmp)\n\
             {\n\
               if(fTmp) return();\n\
               EffectVar(0, pTarget, iNumber) = GetX(pTarget);\n\
               EffectVar(1, pTarget, iNumber) = GetY(pTarget);\n\
             }\n",
        )
        .expect("script");

        let resolver = ProbeResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(3);
        scenario.apply(&mut engine).expect("scenario applies");
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Test".into(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("join succeeds");

        let snapshot = engine.snapshot();
        let pinned = snapshot
            .objects
            .iter()
            .filter(|object| {
                object.definition_id == "NPCX"
                    && object.effects.iter().any(|e| e.name == "StayThere")
            })
            .count();
        assert_eq!(pinned, 2, "both unowned crew NPCs got StayThere");
        let stored = snapshot
            .objects
            .iter()
            .find(|object| object.id.as_u64() == 30)
            .and_then(|object| {
                object
                    .effects
                    .iter()
                    .find(|e| e.name == "StayThere")
                    .map(|e| e.vars.clone())
            })
            .expect("effect present");
        assert!(
            matches!(stored.first(), Some(crate::effect::EffectVarValue::Int(50))),
            "the GLOBAL FxStayThereStart stored GetX via the seam \
             (C4Effect.cpp:31-40 GetFuncRecursive), got {stored:?}"
        );
    }

    // C4Game::Synchronize's tail broadcasts ~UpdateTransferZone to every
    // active Game.Objects entry (C4Game.cpp:3713-3714,3727-3729;
    // C4GameObjects.cpp:54-58; C4ObjectList.cpp:734-739) AFTER the FixRandom
    // re-fix. GoldRush oracle:
    // the placed cannon's handler
    // (Cannon.c4d/Script.c:20-25) re-runs Initialize() because the stale
    // saved Action=Stand loaded as Idle - SetAction("Ready") +
    // SetDir(Random(2)) (the first draw of the fresh ledger) + the GC4V
    // crosshair as the FIRST created object (C++ NEWOBJ 1420, frame 0,
    // pre-join).
    #[test]
    fn game_start_broadcasts_update_transfer_zone_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let cannon = defs.join("Cannon.c4d");
        std::fs::create_dir_all(&cannon).expect("cannon dir");
        std::fs::write(
            cannon.join("DefCore.txt"),
            "[DefCore]\nid=CANN\nName=Cannon\nCategory=16\n",
        )
        .expect("defcore");
        write_test_definition_graphics(&cannon);
        std::fs::write(
            cannon.join("ActMap.txt"),
            "[Action]\nName=Ready\nDelay=0\nLength=8\n",
        )
        .expect("actmap");
        std::fs::write(
            cannon.join("Script.c"),
            "#strict\n\
             protected func Initialize() {\n\
                 SetAction(\"Ready\");\n\
                 SetDir(Random(2));\n\
                 CreateObject(MARK, 0, 0, -1);\n\
                 return(1);\n\
             }\n\
             protected func UpdateTransferZone() { if(ActIdle()) Initialize(); return(1); }\n",
        )
        .expect("script");
        let marker = defs.join("Marker.c4d");
        std::fs::create_dir_all(&marker).expect("marker dir");
        std::fs::write(
            marker.join("DefCore.txt"),
            "[DefCore]\nid=MARK\nName=Marker\nCategory=16\n",
        )
        .expect("marker core");
        write_test_definition_graphics(&marker);

        write_scenario(
            dir.path(),
            "[Object]\nid=CANN\nNumber=439\nCategory=16\nSize=100000\nX=100\nY=100\nAction=Stand\n",
        );
        let (engine, _) = load(dir.path());

        let (_, action, ..) = engine.debug_object_by_id(439).expect("cannon exists");
        assert_eq!(
            action, "Ready",
            "the ~UpdateTransferZone broadcast re-ran Initialize (Cannon.c4d:23)"
        );
        assert!(
            engine.debug_object_by_id(440).is_some(),
            "the crosshair-analog is the FIRST created object (C++ 1420)"
        );

        // The Random(2) draw came off the FRESH post-Synchronize ledger.
        let mut fresh = crate::rng::LcgRng::seed_from_u64(11);
        fresh.random(2);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            fresh.random(1_000_000),
            "SetDir(Random(2)) drew after the FixRandom re-fix (C4Game.cpp:3695,3710)"
        );
    }

    // C4Game::Synchronize re-fixes the RNG AFTER the weather-init draws
    // (C4Game.cpp:3695): the post-apply ledger is a FRESH FixRandom(seed)
    // stream — the join draws from position zero.
    #[test]
    fn game_start_refixes_the_ledger_after_weather_draws_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(dir.path(), "");
        let (engine, _) = load(dir.path());

        let mut fresh = crate::rng::LcgRng::seed_from_u64(11);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            fresh.random(1_000_000),
            "post-apply ledger = fresh FixRandom(seed) (C4Game.cpp:3695)"
        );
    }
}
