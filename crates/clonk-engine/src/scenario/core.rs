//! `scenario` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

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
    /// `hGroup.FindEntry(C4CFN_SavePlayerInfos)`: a present component fills
    /// `RestorePlayerInfos` whatever `SaveGame` says, so ordinary scenarios
    /// shipping restore rows reach `InitPlayers`' recreation branch too
    /// (C4GameParameters.cpp:378-385, C4Game.cpp:2841-2843).
    pub restore_player_infos: bool,
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
pub(in crate::scenario) struct ScenarioDefinition {
    pub(in crate::scenario) id: String,
    pub(in crate::scenario) name: Option<String>,
    pub(in crate::scenario) description: Option<String>,
    pub(in crate::scenario) clonk_names: Option<String>,
    pub(in crate::scenario) script: String,
    /// Native C4ScriptHost::ScriptName (`<group full name>/Script.c`).
    pub(in crate::scenario) script_name: Option<String>,
    pub(in crate::scenario) actions: Option<DefinitionActions>,
    pub(in crate::scenario) crew_member: bool,
    pub(in crate::scenario) can_be_base: bool,
    pub(in crate::scenario) movement: MovementProfile,
    pub(in crate::scenario) movement_manifest: bool,
    pub(in crate::scenario) category: i32,
    pub(in crate::scenario) value: i32,
    pub(in crate::scenario) mass: i32,
    pub(in crate::scenario) picture: Option<DefinitionPicture>,
    pub(in crate::scenario) picture_image: Option<GraphicsImage>,
    pub(in crate::scenario) picture_color_by_owner_mask: Option<ColorByOwnerMask>,
    pub(in crate::scenario) graphics_image: Option<GraphicsImage>,
    pub(in crate::scenario) color_by_owner_mask: Option<ColorByOwnerMask>,
    pub(in crate::scenario) additional_graphics: HashMap<String, ResourceGraphicsVariant>,
    /// First def portrait (C4CFN_Portraits, src/C4Components.h:88) for the
    /// HUD cursor info (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-320).
    pub(in crate::scenario) portrait_image: Option<GraphicsImage>,
    pub(in crate::scenario) portrait_graphics_image: Option<GraphicsImage>,
    pub(in crate::scenario) portrait_color_by_owner_mask: Option<ColorByOwnerMask>,
    pub(in crate::scenario) portrait_graphics: Vec<ResourceGraphicsVariant>,
    /// Def rank symbols (C4Def::pRankSymbols, src/C4Def.cpp:684-691).
    pub(in crate::scenario) rank_symbols_image: Option<GraphicsImage>,
    pub(in crate::scenario) rank_names: Option<RankNameTable>,
    pub(in crate::scenario) rank_base: Option<i32>,
    pub(in crate::scenario) rank_symbol_count: Option<u32>,
    pub(in crate::scenario) resource_group: Option<Group>,
    pub(in crate::scenario) components: Vec<DefinitionComponent>,
    pub(in crate::scenario) line_connect: u32,
    /// DefCore shape vertices + rect (the spawn shape; the `core` field
    /// below carries the rest).
    pub(in crate::scenario) vertices: Vec<clonk_resources::definition::DefVertex>,
    pub(in crate::scenario) shape: Option<clonk_resources::definition::PictureRect>,
    /// The FULL DefCore for legacy defs — applied via
    /// Engine::apply_resource_core so no core field silently drops
    /// (physicals/Float/Timer/Grab did).
    pub(in crate::scenario) core: Option<clonk_resources::definition::DefCore>,
}

#[derive(Debug, Clone)]
pub struct SkyConfig {
    pub settings: SkySettings,
    pub surface: Option<Arc<GraphicsImage>>,
}

#[derive(Debug, Clone)]
pub(in crate::scenario) struct DefinitionActions {
    pub(in crate::scenario) default_action: Option<String>,
    pub(in crate::scenario) specs: HashMap<String, ActionSpec>,
    pub(in crate::scenario) physical: Vec<(String, ActionSpec)>,
    pub(in crate::scenario) graphics: HashMap<String, DefinitionActionGraphics>,
    pub(in crate::scenario) reflections: HashMap<String, crate::action::C4ActionReflection>,
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
pub(crate) struct ScenarioScriptSource {
    pub(in crate::scenario) name: String,
    pub(in crate::scenario) source: String,
    /// Real (legacy) content gets the C++ callback convention — no
    /// synthetic state argument (Game.Script.Call(PSF_Initialize) has no
    /// parameters; GRBroadcast args start with the player number,
    /// C4Player.cpp:769-775). JSON fixtures keep the state proplist.
    pub(in crate::scenario) c4_args: bool,
}

/// Named non-global functions visible from the fully linked scenario host
/// when Landscape.txt is parsed. Build the same lightweight script-host tree
/// used at apply time so #appendto is resolved before scenario #include.
/// Engine-global functions remain excluded: Game.Script.GetSFunc performs a
/// local-owner lookup and the declaring host's global FnLink is unnamed.
pub(in crate::scenario) fn scenario_may_need_map_callbacks(
    group: &Group,
) -> Result<bool, ScenarioError> {
    if let Some(source) = try_read_group_file_case_insensitive(group, "Landscape.txt")? {
        // False positives are harmless: comments and values containing these
        // spellings merely retain the linker. Exact field-name bytes cannot
        // be split by whitespace in the S2 grammar, so a miss proves that the
        // parser cannot reach C4MCV_ScriptFunc for this source.
        if [b"evalFn".as_slice(), b"drawFn".as_slice()]
            .into_iter()
            .any(|field| source.windows(field.len()).any(|window| window == field))
        {
            return Ok(true);
        }
    }
    Ok(group
        .entries()?
        .into_iter()
        // This is deliberately only a conservative candidate check. Native
        // validates the 1..=30-byte section name later, after landscape init;
        // validating here would change which load error wins.
        .any(|entry| legacy_group_wildcard_match(b"Sect*.c4g", &entry.name_bytes)))
}

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
pub(in crate::scenario) enum DefinitionLoadStep {
    Definition(String),
    Declarations { name: String, source: String },
    SystemScripts(Vec<(String, String)>),
    Particle(ResourceParticleDefinition),
}

#[derive(Debug)]
pub(in crate::scenario) enum CollectedDefinition {
    Definition(ScenarioDefinition),
    SystemScripts(Vec<(String, String)>),
    Particle(ResourceParticleDefinition),
}

// C4Game::InitDefs checks every loaded definition against the running engine
// tuple before script linking (C4Game.cpp:108-115; C4Version.h:28-32).
use clonk_core::version::ENGINE_VERSION as DEFINITION_ENGINE_VERSION;

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
    pub(in crate::scenario) legacy_core: Option<LegacyScenarioCore>,
    /// Post-compile Teams.txt state. `None` means the legacy group had no
    /// Teams.txt, so C4TeamList::Load derives defaults from the scenario.
    pub(in crate::scenario) legacy_team_metadata: Option<LoadedLegacyTeamMetadata>,
    pub(in crate::scenario) name: Option<String>,
    pub(in crate::scenario) description: Option<String>,
    pub(in crate::scenario) ticks: Option<u32>,
    pub(in crate::scenario) ground_height_hint: Option<i32>,
    /// The ordered C4Game::InitMaterialTexture material chain retained from
    /// legacy loading. Scenario-local definitions precede admitted external
    /// groups and therefore win duplicate names (C4Game.cpp:901-977,
    /// C4Material.cpp:263-299).
    pub(in crate::scenario) material_library: Option<clonk_resources::MaterialLibrary>,
    pub(in crate::scenario) definitions: Vec<ScenarioDefinition>,
    /// `[Game] ValueOverloads`: C4Game::InitValueOverloads applies these to
    /// the loaded definitions immediately before Objects.Load
    /// (C4Game.cpp:2704-2713,3997-4004).
    pub(in crate::scenario) value_overloads: Vec<(String, i32)>,
    pub(in crate::scenario) initial_spawns: Vec<ScenarioSpawn>,
    pub(in crate::scenario) landscape: Option<Landscape>,
    pub(in crate::scenario) post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks,
    pub(in crate::scenario) keep_map_creator: bool,
    /// `MapWdt`/`MapHgt` as `~C4MapCreatorS2` re-evaluates them when the
    /// creator is discarded (clonk-org/clonk-rs#1050).
    pub(in crate::scenario) map_width: LegacyC4SVal,
    pub(in crate::scenario) map_height: LegacyC4SVal,
    pub(in crate::scenario) scenario_sections: Vec<ScenarioSectionSpec>,
    pub(in crate::scenario) physics: Option<PhysicsSettings>,
    /// The C4Aul string enumeration loaded from `Strings.txt`. Compiled
    /// Globals/GlobalNamed/effect variables refer to these integer IDs.
    pub(in crate::scenario) legacy_string_table: clonk_script::StringRegistrations,
    /// `RoundResults.txt` compiled after InitControl and before pointer
    /// denumeration. A missing component retains C4RoundResults::Init's
    /// scenario-melee default.
    pub(in crate::scenario) round_results: RoundResultsState,
    /// The `[Landscape] Gravity` C4SVal — evaluated through the synced
    /// ledger at apply time (C4Landscape::ScenarioInit, C4Landscape.cpp:66).
    pub(in crate::scenario) gravity: LegacyC4SVal,
    pub(in crate::scenario) environment: Option<EnvironmentSettings>,
    pub(in crate::scenario) sky: Option<SkyConfig>,
    pub(in crate::scenario) script: Option<ScenarioScriptSource>,
    pub(in crate::scenario) objectives: ScenarioObjectives,
    pub(in crate::scenario) construction_needs_material: bool,
    pub(in crate::scenario) structures_need_energy: bool,
    pub(in crate::scenario) base_buy_enabled: bool,
    pub(in crate::scenario) base_sell_enabled: bool,
    pub(in crate::scenario) base_auto_sell_enabled: bool,
    pub(in crate::scenario) base_reject_entrance_enabled: bool,
    pub(in crate::scenario) base_regenerate_energy_enabled: bool,
    pub(in crate::scenario) base_extinguish_enabled: bool,
    pub(in crate::scenario) base_regenerate_energy_price: i32,
    pub(in crate::scenario) landscape_insert_thrust: bool,
    /// `[Head] DisableMouse`: prevents every joined player from receiving
    /// mouse control (`C4Player::InitControl`, C4Player.cpp:1907-1912).
    pub(in crate::scenario) disable_mouse: bool,
    /// `[Head] ForcedAutoContextMenu`: `None` keeps the player-file
    /// preference; `Some` forces automatic context menus for all players
    /// (C4Player::ApplyForcedControl, C4Player.cpp:2369-2375).
    pub(in crate::scenario) forced_auto_context_menu: Option<bool>,
    /// `[Head] ForcedAutoStopControl`: `None` keeps the player-file
    /// preference; `Some` forces classic/Jump'n'Run control for all players
    /// (C4Player::ApplyForcedControl, C4Player.cpp:2369-2389).
    pub(in crate::scenario) forced_control_style: Option<bool>,
    /// The surviving definition hosts and definition-pack System.c4g hosts in
    /// their C4DefList::Load order. System hosts remain in place when a later
    /// definition overload removes an earlier same-ID definition host.
    pub(in crate::scenario) definition_load_steps: Vec<DefinitionLoadStep>,
    /// Ordered external/folder definition resources used by the Rust load.
    /// Old saves with a Game.txt DefinitionFiles block expose that later C++
    /// override as unresolved in `lobby_metadata`; the scenario group itself
    /// is deliberately absent.
    pub(in crate::scenario) definition_resource_paths: Vec<PathBuf>,
    /// Exact ordered `NRT_Definitions` roots registered in `Game.GroupSet`.
    /// This includes folder-local definition roots after the selected external
    /// vector: C++ first registers those folders at folder priority, then adds
    /// them to `DefinitionFilenames` and registers them again at definition
    /// priority (C4Game.cpp:210-213,2432-2442,3961-3994).
    pub(in crate::scenario) definition_root_groups: Vec<Group>,
    /// Exact `C4SoundSystem::LoadEffects` group stream produced by
    /// `C4DefList::Load`, in native load order. Unlike the surviving
    /// definition list, this retains pure sound `.c4d` groups, rejected
    /// DefCore groups and duplicate visits so later samples can overload
    /// earlier samples exactly as they do in C++.
    pub(in crate::scenario) sound_effect_groups: Vec<Group>,
    /// The scenario's own System.c4g sources. C++ loads these after defs
    /// specifically to give them overload priority (C4Game.cpp:2606-2617).
    pub(in crate::scenario) scenario_system_scripts: Vec<(String, String)>,
    /// The four C4SPlrStart slots, consumed by joining players
    /// (C4Player::ScenarioInit, C4Player.cpp:670-777).
    pub(in crate::scenario) player_starts: Vec<PlayerStart>,
    /// Ordered `Game.Teams` entries from the scenario's Teams.txt.
    pub(in crate::scenario) teams: Vec<TeamInfo>,
    /// Immutable, pre-game presentation inputs retained from Scenario.txt,
    /// Parameters.txt and Teams.txt. JSON fixture scenarios deliberately keep
    /// this as `None`: those files do not declare the legacy lobby contract.
    pub(in crate::scenario) lobby_metadata: Option<ScenarioLobbyMetadata>,
    /// The scenario's own Names.txt, overriding the standard clonk names
    /// in Game.Names (C4Game.cpp:3288-3289).
    pub(in crate::scenario) standard_names: Option<String>,
    /// `[Landscape] MapZoom` kept as a C4SVal: ScenarioInit evaluates it
    /// per configured start coordinate (C4Player.cpp:713-714).
    pub(in crate::scenario) map_zoom: LegacyC4SVal,
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
    pub(in crate::scenario) fn teams_file_defaults() -> Self {
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
    pub(in crate::scenario) configured_specification: String,
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
    pub(in crate::scenario) fn from_raw(value: i32) -> Self {
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
    effective_modules: Vec<String>,
    effective_module_spellings: Vec<String>,
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

    /// External and folder-local resources, the scenario group aside. An old
    /// save's `[DefinitionFiles]` override has already replaced them.
    pub fn resolved_load_resources(&self) -> &[PathBuf] {
        &self.resolved_load_resources
    }

    /// The `[DefinitionFiles]` lines an old save's Game.txt carried, verbatim.
    /// Kept for reporting: the vectors above already reflect them.
    pub fn savegame_override(&self) -> &ScenarioSavegameDefinitionOverride {
        &self.savegame_override
    }

    /// The vector definitions are actually loaded from — `requested_modules`,
    /// unless an old save's `[DefinitionFiles]` section replaced it
    /// (C4Game.cpp:222-227).
    pub fn effective_modules(&self) -> &[String] {
        &self.effective_modules
    }

    /// The exact native strings behind `effective_modules`. An old save's
    /// `[DefinitionFiles]` names are their own spelling — C++ pushes one
    /// string and never keeps a second (C4Game.cpp:3646).
    pub fn effective_module_spellings(&self) -> &[String] {
        &self.effective_module_spellings
    }
}

/// One `[DefinitionFiles]` line as `C4Game::DefinitionFilenamesFromSaveGame`
/// stores it: `line.substr(1)` (C4Game.cpp:3646).
///
/// Not `line.split('=').nth(1)`. `p = line.find('=', p) != std::string::npos`
/// at C4Game.cpp:3643 assigns the *comparison's* result, so `p` is 1 by the
/// time `substr` reads it and the stored name is the line minus its first
/// character. It is a long-standing C++ bug, and it decides which definition
/// files a savegame asks for, so the port reproduces it rather than the
/// evident intent.
fn savegame_definition_module(line: &str) -> String {
    line.chars().skip(1).collect()
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

    /// The vector `C4Game::DefinitionFilenamesFromSaveGame` leaves behind —
    /// `line.substr(1)` per accepted line, and empty for a section with none
    /// (C4Game.cpp:3635-3651). `None` when Game.txt carried no section at all,
    /// which is the only case that keeps the ordinary selection.
    pub fn effective_modules(&self) -> Option<Vec<String>> {
        self.definition_lines().map(|lines| {
            lines
                .iter()
                .map(String::as_str)
                .map(savegame_definition_module)
                .collect()
        })
    }
}

#[derive(Debug)]
pub(in crate::scenario) struct LoadedLegacyTeamMetadata {
    pub(in crate::scenario) metadata: InitialNetworkTeamMetadata,
    pub(in crate::scenario) random_color_team_id: Option<i32>,
    pub(in crate::scenario) unsupported_team_distribution: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioGameParameterResolution {
    EmbeddedFileBeforeRuntimeAdjustments,
    RequiresRuntimeConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioLobbyIdEntry {
    pub(in crate::scenario) id: String,
    pub(in crate::scenario) count: i32,
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
    pub(in crate::scenario) id: i32,
    pub(in crate::scenario) activated: bool,
    pub(in crate::scenario) observer: bool,
    pub(in crate::scenario) name: String,
    pub(in crate::scenario) nick: String,
    pub(in crate::scenario) lobby_ready: bool,
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
    pub(in crate::scenario) random_seed: i32,
    pub(in crate::scenario) startup_player_count: i32,
    pub(in crate::scenario) max_players: i32,
    pub(in crate::scenario) use_fair_crew: bool,
    pub(in crate::scenario) fair_crew_forced: bool,
    pub(in crate::scenario) fair_crew_strength: i32,
    pub(in crate::scenario) allow_debug: bool,
    pub(in crate::scenario) is_network_game: bool,
    pub(in crate::scenario) control_rate: i32,
    pub(in crate::scenario) auto_frame_skip: bool,
    pub(in crate::scenario) rules: Vec<ScenarioLobbyIdEntry>,
    pub(in crate::scenario) goals: Vec<ScenarioLobbyIdEntry>,
    pub(in crate::scenario) league: String,
    pub(in crate::scenario) clients: Vec<ScenarioLobbyClient>,
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
    pub(in crate::scenario) random_seed: Option<i32>,
    pub(in crate::scenario) max_players: Option<i32>,
    pub(in crate::scenario) startup_player_count: Option<i32>,
    pub(in crate::scenario) use_fair_crew: Option<bool>,
    pub(in crate::scenario) fair_crew_forced: Option<bool>,
    pub(in crate::scenario) fair_crew_strength: Option<i32>,
    pub(in crate::scenario) allow_debug: Option<bool>,
    pub(in crate::scenario) is_network_game: Option<bool>,
    pub(in crate::scenario) control_rate: Option<i32>,
    pub(in crate::scenario) auto_frame_skip: Option<bool>,
    pub(in crate::scenario) rules: Option<Vec<ScenarioLobbyIdEntry>>,
    pub(in crate::scenario) goals: Option<Vec<ScenarioLobbyIdEntry>>,
    pub(in crate::scenario) league: Option<String>,
    pub(in crate::scenario) clients: Vec<ScenarioLobbyClient>,
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
    pub(in crate::scenario) id: i32,
    pub(in crate::scenario) name: String,
    pub(in crate::scenario) player_start_index: i32,
    pub(in crate::scenario) player_count: i32,
    pub(in crate::scenario) players: Vec<i32>,
    pub(in crate::scenario) configured_color: u32,
    pub(in crate::scenario) icon_spec: Option<String>,
    pub(in crate::scenario) max_players: i32,
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
    pub(in crate::scenario) source: ScenarioTeamsSource,
    pub(in crate::scenario) active: bool,
    pub(in crate::scenario) custom: bool,
    pub(in crate::scenario) allow_hostility_change: bool,
    pub(in crate::scenario) allow_team_switch: bool,
    pub(in crate::scenario) configured_auto_generate: bool,
    pub(in crate::scenario) auto_generate: bool,
    pub(in crate::scenario) configured_last_team_id: i32,
    pub(in crate::scenario) last_team_id: i32,
    pub(in crate::scenario) distribution: ScenarioTeamDistribution,
    pub(in crate::scenario) team_colors: bool,
    pub(in crate::scenario) max_script_players: i32,
    pub(in crate::scenario) script_player_names: String,
    pub(in crate::scenario) random_team_count: i32,
    pub(in crate::scenario) teams: Vec<ScenarioLobbyTeam>,
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
    /// Whether lobby preloading must defer the landscape until the final
    /// synchronized player roster is known.
    ///
    /// C++ returns from the preloading second part before Landscape::Init for
    /// a main-section `MapPlayerExtend`, then evaluates `StartupPlayerCount`
    /// after the lobby and creates the map on the foreground InitGame pass.
    /// Child sections are initialized only when activated and read the same
    /// frozen count (src/C4Game.cpp:2455-2462,2642-2649,4084-4223).
    pub fn uses_map_player_extend(&self) -> bool {
        self.legacy_core
            .as_ref()
            .is_some_and(|core| core.landscape.map_player_extend)
            || self.scenario_sections.iter().any(|section| {
                section
                    .s2_overload
                    .as_ref()
                    .is_some_and(|spec| spec.map_player_extend)
            })
    }

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
        let restore_player_infos =
            read_optional_legacy_entry(group, "SavePlayerInfos.txt")?.is_some();

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
            restore_player_infos,
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
        Self::load_network_from_path_with_languages_and_seed_and_packs_and_progress(
            path,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            language_packs,
            |_, _| {},
        )
    }

    /// Progress-reporting counterpart used by the post-lobby network loader.
    /// The callback covers the shared InitGame first/second-part work and
    /// stops at 93; resource transfer and final activation remain separate.
    #[allow(clippy::too_many_arguments)]
    pub fn load_network_from_path_with_languages_and_seed_and_packs_and_progress<S, F>(
        path: impl AsRef<Path>,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
        language_packs: &LanguagePacks,
        report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
        F: FnMut(i32, &str),
    {
        Self::load_network_from_path_with_languages_and_seed_and_packs_and_startup_player_count_and_progress(
            path,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            language_packs,
            legacy_startup_player_count(),
            report_progress,
        )
    }

    /// Network loader with C4Game's already-frozen StartupPlayerCount.
    /// Dynamic landscape creation consumes this value before activation.
    #[allow(clippy::too_many_arguments)]
    pub fn load_network_from_path_with_languages_and_seed_and_packs_and_startup_player_count_and_progress<
        S,
        F,
    >(
        path: impl AsRef<Path>,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
        language_packs: &LanguagePacks,
        startup_player_count: i32,
        report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
        F: FnMut(i32, &str),
    {
        let group = Group::open(path)?;
        Self::load_network_from_group_with_languages_and_seed_and_packs_and_startup_player_count_and_progress(
            &group,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            language_packs,
            startup_player_count,
            report_progress,
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
        Self::load_network_from_group_with_languages_and_seed_and_packs_and_progress(
            group,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            language_packs,
            |_, _| {},
        )
    }

    /// Group-backed network loader with the shared C4Game InitGame progress
    /// callback (src/C4Game.cpp:2551-2721).
    #[allow(clippy::too_many_arguments)]
    pub fn load_network_from_group_with_languages_and_seed_and_packs_and_progress<S, F>(
        group: &Group,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
        language_packs: &LanguagePacks,
        report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
        F: FnMut(i32, &str),
    {
        Self::load_network_from_group_with_languages_and_seed_and_packs_and_startup_player_count_and_progress(
            group,
            definition_groups,
            material_groups,
            graphics_groups,
            languages,
            random_seed,
            language_packs,
            legacy_startup_player_count(),
            report_progress,
        )
    }

    /// Group-backed network loader with the synchronized startup-player
    /// count used by MapPlayerExtend (src/C4Game.cpp:2394-2431;
    /// src/C4Landscape.cpp:518-522).
    #[allow(clippy::too_many_arguments)]
    pub fn load_network_from_group_with_languages_and_seed_and_packs_and_startup_player_count_and_progress<
        S,
        F,
    >(
        group: &Group,
        definition_groups: &[Group],
        material_groups: &[Group],
        graphics_groups: &[Group],
        languages: &[S],
        random_seed: u64,
        language_packs: &LanguagePacks,
        startup_player_count: i32,
        mut report_progress: F,
    ) -> Result<Self, ScenarioError>
    where
        S: AsRef<str>,
        F: FnMut(i32, &str),
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
        Self::load_from_group_with_languages_and_seed_and_definition_modules_inner(
            group,
            &resolver,
            &languages,
            random_seed,
            &[],
            Some(&definition_modules),
            None,
            startup_player_count,
            false,
            &mut report_progress,
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
        F: FnMut(i32, &str),
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
        F: FnMut(i32, &str),
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
        F: FnMut(i32, &str),
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
        F: FnMut(i32, &str),
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

    pub(in crate::scenario) fn load_from_group_with_languages_and_seed_and_definition_modules<
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
        let mut ignore_progress = |_: i32, _: &str| {};
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
        report_progress: &mut dyn FnMut(i32, &str),
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
        report_progress: &mut dyn FnMut(i32, &str),
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
        report_progress: &mut dyn FnMut(i32, &str),
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
        report_progress: &mut dyn FnMut(i32, &str),
    ) -> Result<Self, ScenarioError> {
        let indexed_group = group.is_directory().then(|| group.indexed()).transpose()?;
        let group = indexed_group.as_ref().unwrap_or(group);
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
        // `DefinitionFilenames.clear()` followed by `push_back(line.substr(1))`
        // for every accepted line (C4Game.cpp:3635-3651). The caller applies
        // it after the preset, the DefinitionPath expansion and the
        // folder-local scan (C4Game.cpp:180-227), so it replaces all three —
        // including an empty section, which clears the vector and adds
        // nothing.
        // Only the peer that builds `DefinitionFilenames` itself applies it.
        // A network client's definitions arrive as published `GameRes`, and
        // `C4Game::OpenScenario` skips `Parameters.Load` for one
        // (C4Game.cpp:230-236), so the section never reaches its load — which
        // is the same condition that decides the folder-local scan below.
        let savegame_definition_modules = discover_folder_definitions
            .then(|| savegame_override.effective_modules())
            .flatten();
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
        let (definition_specs_to_resolve, definition_spellings_to_resolve) =
            match savegame_definition_modules.as_deref() {
                Some(modules) => (modules, modules),
                None => (definition_specs, selected_definition_spellings),
            };
        let effective_definition_modules = definition_specs_to_resolve.to_vec();
        let effective_definition_spellings = definition_spellings_to_resolve.to_vec();
        report_progress(8, "Definition selection resolved");
        let folder_definition_groups =
            if discover_folder_definitions && savegame_definition_modules.is_none() {
                folder_local_definition_groups(group)?
            } else {
                // The override's `clear()` also drops the folder-local paths the
                // caller appended just before it (C4Game.cpp:210-227).
                Vec::new()
            };
        let external_definition_group_count = definition_specs_to_resolve.len()
            * if definition_path_expansion.is_some() {
                2
            } else {
                1
            }
            + folder_definition_groups.len();
        let definition_progress_range = |index: usize| {
            let count = external_definition_group_count.max(1) as i32;
            let index = index as i32;
            (10 + 25 * index / count, 10 + 25 * (index + 1) / count)
        };
        let mut external_definition_group_index = 0;
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
                let (min_progress, max_progress) =
                    definition_progress_range(external_definition_group_index);
                external_definition_group_index += 1;
                collect_definitions_from_group_with_progress(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                    min_progress,
                    max_progress,
                    "",
                    report_progress,
                )?;
            }
            for spec in definition_specs_to_resolve {
                let definition_group = resolve_one_definition_group(group, resolver, spec)?;
                definition_resource_paths.push(definition_group.root().to_path_buf());
                definition_root_groups.push(definition_group.clone());
                let (min_progress, max_progress) =
                    definition_progress_range(external_definition_group_index);
                external_definition_group_index += 1;
                collect_definitions_from_group_with_progress(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                    min_progress,
                    max_progress,
                    "",
                    report_progress,
                )?;
            }
        } else {
            for spec in definition_specs_to_resolve {
                let definition_group = resolve_one_definition_group(group, resolver, spec)?;
                definition_resource_paths.push(definition_group.root().to_path_buf());
                definition_root_groups.push(definition_group.clone());
                let (min_progress, max_progress) =
                    definition_progress_range(external_definition_group_index);
                external_definition_group_index += 1;
                collect_definitions_from_group_with_progress(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &language_packs,
                    group,
                    scenario_origin.as_deref(),
                    &mut sound_effect_groups,
                    &mut load_items,
                    min_progress,
                    max_progress,
                    "",
                    report_progress,
                )?;
            }
        }

        for folder_group in folder_definition_groups {
            definition_resource_paths.push(folder_group.root().to_path_buf());
            definition_root_groups.push(folder_group.clone());
            let (min_progress, max_progress) =
                definition_progress_range(external_definition_group_index);
            external_definition_group_index += 1;
            collect_definitions_from_group_with_progress(
                &folder_group,
                true,
                &skip_ids,
                languages,
                &language_packs,
                group,
                scenario_origin.as_deref(),
                &mut sound_effect_groups,
                &mut load_items,
                min_progress,
                max_progress,
                "",
                report_progress,
            )?;
        }

        // InitDefs' scenario pass disables System.c4g discovery because the
        // scenario-local group is loaded later by LoadScenarioScripts.
        collect_definitions_from_group_with_progress(
            group,
            false,
            &skip_ids,
            languages,
            &language_packs,
            group,
            scenario_origin.as_deref(),
            &mut sound_effect_groups,
            &mut load_items,
            35,
            40,
            "Definition metadata and sources collected",
            report_progress,
        )?;

        // fOverload replaces and destroys an earlier same-ID C4Def script,
        // while System hosts loaded between the two definitions remain live.
        // Keep only the last definition event for each ID without flattening
        // the surviving System host order.
        let mut last_definition = HashMap::new();
        // `Graphics.VerboseObjectLoading >= 3` logs every loaded definition's
        // group full name (C4Def.cpp:555-556), and levels 1/2 need the winning
        // definition's name and group to describe each overload (:1051-1058).
        let verbose_level = verbose_loading::verbose_object_loading();
        let mut overload_winners: HashMap<String, (String, String)> = HashMap::new();
        let mut seen_particles: HashSet<String> = HashSet::new();
        // `LoadResStr(IDS_PRC_DEFOVERLOAD)` is process-local like C++'s
        // Application.ResStrTable. Avoid even reading it when level 0 keeps
        // the overload bookkeeping disabled.
        let overload_template =
            (verbose_level >= 1).then(verbose_loading::definition_overload_template);
        for (index, item) in load_items.iter().enumerate() {
            match item {
                CollectedDefinition::Definition(definition) => {
                    let key = definition.id.to_ascii_uppercase();
                    let group = definition
                        .script_name
                        .as_deref()
                        .map(verbose_loading::group_full_name)
                        .unwrap_or(definition.id.as_str());
                    if let Some(line) =
                        verbose_loading::definition_loaded_line(verbose_level, group)
                    {
                        tracing::info!("{line}");
                    }
                    // Only levels >= 1 report overloads, so the default level
                    // pays nothing on a path that dominates scenario load.
                    if verbose_level >= 1 {
                        overload_winners.insert(
                            key.clone(),
                            (
                                definition
                                    .name
                                    .clone()
                                    .unwrap_or_else(|| definition.id.clone()),
                                group.to_owned(),
                            ),
                        );
                    }
                    last_definition.insert(key, index);
                }
                // C4ParticleDef::Load reports an overload against the particle
                // already registered under this name (C4Particles.cpp:180-185).
                CollectedDefinition::Particle(definition)
                    if verbose_level >= 1
                        && !seen_particles.insert(definition.core.name.clone()) =>
                {
                    if let Some(template) = overload_template.as_deref() {
                        if let Some(line) = verbose_loading::particle_overload_line(
                            verbose_level,
                            template,
                            &definition.core.name,
                        ) {
                            tracing::info!("{line}");
                        }
                    }
                }
                CollectedDefinition::Particle(_) => {}
                CollectedDefinition::SystemScripts(_) => {}
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
                    // This definition lost to a later one with the same ID:
                    // C++ logs the overload from the winner's side (:1051-1058).
                    if let Some((winning_name, winning_group)) =
                        overload_winners.get(&definition.id.to_ascii_uppercase())
                    {
                        let old_group = definition
                            .script_name
                            .as_deref()
                            .map(verbose_loading::group_full_name)
                            .unwrap_or(definition.id.as_str());
                        if let Some(template) = overload_template.as_deref() {
                            verbose_loading::definition_overload_lines(
                                verbose_level,
                                template,
                                winning_name,
                                &definition.id,
                                old_group,
                                winning_group,
                            )
                            .iter()
                            .for_each(|line| tracing::info!("{line}"));
                        }
                    }
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
        if collected.is_empty() {
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
        let script = load_legacy_scenario_script(group, &scenario_components, languages)?;
        let scenario_system_scripts = load_scenario_system_scripts(
            group,
            &language_packs,
            scenario_origin.as_deref(),
            languages,
        )?;
        report_progress(56, "Scenario script sources loaded");
        let map_callback_functions = if scenario_may_need_map_callbacks(group)? {
            scenario_map_callback_functions(
                script.as_ref(),
                &collected,
                &definition_load_steps,
                &scenario_system_scripts,
            )?
        } else {
            HashSet::new()
        };
        report_progress(57, "Scenario callback names indexed");
        let mut classifier = build_map_pixel_classifier(group, resolver)?;
        report_progress(58, "Material and texture-map data decoded");
        let material_library = classifier
            .as_ref()
            .and_then(MapPixelClassifier::material_library)
            .cloned();
        // C4Game.cpp:981 reports the exact loaded-material count before the
        // material enumeration and texture-map initialization.
        let loaded_materials = material_library
            .as_ref()
            .map(|library| library.iter().count())
            .unwrap_or_default();
        report_progress(60, &format!("{loaded_materials} materials loaded."));
        let mut post_init_map_callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut prepared_map_creator = None;
        let mut landscape = load_legacy_landscape_with_progress(
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
            report_progress,
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
        let initial_spawns = collect_legacy_objects(group, &collected, &legacy_string_table)?;
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
                effective_modules: effective_definition_modules,
                effective_module_spellings: effective_definition_spellings,
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
            map_width: manifest.core.landscape.map_width,
            map_height: manifest.core.landscape.map_height,
            scenario_sections,
            physics,
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

    /// Rebinds the lightweight pre-lobby projection to the exact definition
    /// rows frozen in network JoinData. The full post-lobby network load still
    /// rebuilds definitions, sounds, materials, scripts and landscape from
    /// those groups; this only makes resource identity consumers observe the
    /// same alias collapse and repetition before that load runs.
    pub fn rebind_network_definition_resource_projection(&mut self, groups: &[Group]) {
        self.definition_resource_paths = groups
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect();
        self.definition_root_groups = groups.to_vec();
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

    pub(in crate::scenario) fn initial_team_configuration(&self) -> crate::TeamConfiguration {
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
            team_registry,
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
    pub(in crate::scenario) fn environment_before_weather_init(
        &self,
        runtime_savegame: bool,
    ) -> EnvironmentSettings {
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
        if !creator.requires_live_script_render() {
            return Ok(());
        }

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

    /// Every `apply_before_players*` entry point funnels through here, which is
    /// what makes it the one place the activation interval can be measured
    /// from (clonk-org/clonk-rs#293).
    #[allow(clippy::too_many_arguments)]
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
        // Timed around the call rather than inside it: the interval has to
        // cover the work no stage claims, and an early `?` return inside the
        // body would otherwise leave that work out of the denominator.
        let started = std::time::Instant::now();
        let applied = self.apply_before_players_stages(
            engine,
            final_synchronize,
            team_configuration_override,
            team_registry_override,
            game_parameter_rule_goal_lists,
            initial_network_game,
            execute_post_init_map_callbacks,
            initial_record_capture,
        );
        engine.record_activation_total(started.elapsed());
        applied
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_before_players_stages(
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
            let started = std::time::Instant::now();
            engine.configure_materials_from_library(material_library);
            engine.record_activation_stage(crate::ActivationStage::Materials, started.elapsed());
        }
        let landscape_started = std::time::Instant::now();
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
        engine.record_activation_stage(
            crate::ActivationStage::Landscape,
            landscape_started.elapsed(),
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

        let definition_registration_started = std::time::Instant::now();
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
            if definition.movement_manifest {
                compiled.set_movement_profile(definition.movement);
            }
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
            // `C4Def::Load` stores the group's own full name as `Filename`
            // (`C4Def.cpp:550`); a console reload re-opens exactly that, and it
            // is what `AddDirectoryForMonitoring` watches.
            compiled.set_source_path(
                definition
                    .resource_group
                    .as_ref()
                    .map(|group| group.root().to_path_buf()),
            );
            engine.register_definition(compiled)?;
        }
        engine.record_activation_stage(
            crate::ActivationStage::DefinitionRegistration,
            definition_registration_started.elapsed(),
        );

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

        let object_placement_started = std::time::Instant::now();
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
            // Cycle edges the sequential spawn had to defer, as
            // `(contained handle, container handle)`.
            let mut deferred_containment: Vec<(String, String)> = Vec::new();
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
                        // A genuine containment cycle. C++ creates every object
                        // first and resolves `Contained` from a number
                        // afterwards, so both edges survive; the sequential
                        // spawn cannot place this one yet, so defer it and
                        // reconnect it below once its container exists.
                        if let Some(spawn) = pending.first_mut() {
                            if let (Some(handle), Some(container)) =
                                (spawn.handle.clone(), spawn.container_handle.take())
                            {
                                deferred_containment.push((handle, container));
                            }
                        } else {
                            break;
                        }
                    }
                }
            }

            // Reconnect the cycle edges now that every object exists. This is
            // the same repair the legacy load path performs — set `Contained`
            // from the resolved id and fix the Contents lists to match
            // (`C4GameObjects::Load`, C4GameObjects.cpp:597-631) — which is
            // what makes a cycle survive on native.
            let cycle_links = deferred_containment
                .into_iter()
                .filter_map(|(child, parent)| Some((*handles.get(&child)?, *handles.get(&parent)?)))
                .collect::<Vec<_>>();
            if !cycle_links.is_empty() {
                engine.restore_legacy_object_links(&cycle_links, &[]);
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
        engine.record_activation_stage(
            crate::ActivationStage::ObjectPlacement,
            object_placement_started.elapsed(),
        );
        let definition_scripts_started = std::time::Instant::now();
        let mut initialized = engine.initialize_definition_scripts()?;
        engine.record_activation_stage(
            crate::ActivationStage::DefinitionScripts,
            definition_scripts_started.elapsed(),
        );
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
            let environment_started = std::time::Instant::now();
            engine
                .run_legacy_init_placements(authoritative_placement.as_ref().unwrap_or(placement));
            engine.record_activation_stage(
                crate::ActivationStage::EnvironmentPlacement,
                environment_started.elapsed(),
            );
            // C4Landscape::PostInitMap follows InitGoals inside the same
            // !NoInitialize/LandscapeLoaded block. Callback arrays execute
            // in field-registration order and each bitset in descending
            // pixel order, on the live post-FixRandom synced ledger
            // (C4Game.cpp:2493-2521; C4MapCreatorS2.cpp:49-114).
            if execute_post_init_map_callbacks {
                let post_init_started = std::time::Instant::now();
                engine.run_post_init_map_callbacks(&live_post_init_map_callbacks)?;
                engine.record_activation_stage(
                    crate::ActivationStage::PostInitMapCallbacks,
                    post_init_started.elapsed(),
                );
            }
            // Freeing the creator is a synchronized draw, not just
            // deallocation: ~C4MapCreatorS2 runs Clear() -> Default() ->
            // C4MCMap::Default, which evaluates MapWdt and MapHgt on the live
            // ledger (C4MapCreatorS2.cpp:633-644,717-740;
            // C4Landscape.cpp:554-556). Native reaches the destructor only
            // when a creator exists, so the draws follow the same condition.
            if !self.keep_map_creator && engine.clear_runtime_map_creator() {
                engine.spend_map_creator_discard_draws(self.map_width, self.map_height);
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
        let started = std::time::Instant::now();
        let additional = engine.initialize_scenario_script();
        let elapsed = started.elapsed();
        engine.record_activation_stage(crate::ActivationStage::ScenarioScript, elapsed);
        // The scenario script runs after `apply_before_players` has closed its
        // own interval, so its span has to be added to the total as well or the
        // stage would exceed the activation it belongs to.
        engine.record_activation_total(elapsed);
        created.append(&mut additional?);
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
                    movement_manifest: false,
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
                scenario_definition.movement_manifest = true;
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
            map_width: LegacyC4SVal::default(),
            map_height: LegacyC4SVal::default(),
            scenario_sections: Vec::new(),
            physics,
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

pub(in crate::scenario) struct LegacyScenarioManifest {
    pub(in crate::scenario) title: Option<String>,
    pub(in crate::scenario) description: Option<String>,
    /// Exact `[Head] Title` bytes after the Scenario.txt compiler's RCT_All
    /// leading-space handling. Group-backed loads always populate this; direct
    /// string fixtures leave it absent.
    pub(in crate::scenario) head_title_native: Option<LegacyCString>,
    pub(in crate::scenario) definition_specs: Vec<String>,
    pub(in crate::scenario) ground_height_hint: Option<i32>,
    pub(in crate::scenario) core: LegacyScenarioCore,
    pub(in crate::scenario) sections: HashMap<String, Vec<(String, String)>>,
}

pub(in crate::scenario) const BASEFUNC_AUTO_SELL_CONTENTS: i32 = 1 << 0;
pub(in crate::scenario) const BASEFUNC_REGENERATE_ENERGY: i32 = 1 << 1;
pub(in crate::scenario) const BASEFUNC_BUY: i32 = 1 << 2;
pub(in crate::scenario) const BASEFUNC_SELL: i32 = 1 << 3;
pub(in crate::scenario) const BASEFUNC_REJECT_ENTRANCE: i32 = 1 << 4;
pub(in crate::scenario) const BASEFUNC_EXTINGUISH: i32 = 1 << 5;
pub(in crate::scenario) const BASEFUNC_DEFAULT: i32 = 0xffff;
pub(in crate::scenario) const BASE_REGENERATE_ENERGY_PRICE: i32 = 5;
pub(crate) const DEFAULT_FOW_RESOLUTION: i32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyIdEntry {
    pub(in crate::scenario) id: String,
    pub(in crate::scenario) count: Option<i32>,
}

pub(in crate::scenario) type LegacyIdList = Vec<LegacyIdEntry>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyNameEntry {
    pub(in crate::scenario) name: String,
    pub(in crate::scenario) count: Option<i32>,
}

pub(in crate::scenario) type LegacyNameList = Vec<LegacyNameEntry>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::scenario) struct LegacyScenarioCore {
    pub(in crate::scenario) head: LegacyHead,
    pub(in crate::scenario) definitions: LegacyDefinitions,
    pub(in crate::scenario) game: LegacyGame,
    pub(in crate::scenario) players: Vec<LegacyPlayer>,
    pub(in crate::scenario) landscape: LegacyLandscape,
    pub(in crate::scenario) weather: LegacyWeather,
    pub(in crate::scenario) disasters: LegacyDisasters,
    pub(in crate::scenario) animals: LegacyAnimals,
    pub(in crate::scenario) environment: LegacyEnvironment,
}
