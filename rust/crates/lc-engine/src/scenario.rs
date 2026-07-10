use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{load_from_memory, ImageError};
use lc_resources::definition::{
    ActionFacet as ResourceActionFacet, DefinitionGraphicsVariant as ResourceGraphicsVariant,
};
use lc_resources::{
    decode_legacy_script_text, localize_script_source,
    ActionDefinition as ResourceActionDefinition, ActionMap as ResourceActionMap, ColorByOwnerMask,
    DefinitionError as ResourceDefinitionError, GraphicsImage, Group, GroupError,
    ResourceDefinition as ResourceDefinitionData,
};
use serde::de::Error as _;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::Deserialize;

use crate::landscape::{LandscapeRasterState, RuntimeTexMapMaterial, RuntimeTexMapState};
use crate::{
    action::ActionSpec, ActionState, CommandDirection, Definition, DefinitionActionFacet,
    DefinitionActionGraphics, DefinitionComponent, DefinitionPicture, DefinitionPictureImage,
    DefinitionSpriteImage, Direction, EffectState, Engine, EngineError, EnvironmentSettings,
    Landscape, MovementProfile, ObjectId, ObjectStatus, PhysicsSettings, RgbColor, SkyParallaxMode,
    SkySettings, SpawnConfig, TeamInfo, Vector2, FULL_CON,
};

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("scenario manifest `Scenario.json` not found")]
    ManifestMissing,
    #[error("legacy scenario core `Scenario.txt` not found")]
    LegacyCoreMissing,
    #[error("failed to parse scenario manifest: {0}")]
    ManifestParse(#[from] serde_json::Error),
    #[error("scenario resource error: {0}")]
    Resources(#[from] GroupError),
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
    #[error("legacy objects file `Objects.txt` is not valid UTF-8")]
    LegacyObjectsEncoding,
    #[error("invalid legacy objects data: {0}")]
    LegacyObjectsParse(String),
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
}

#[derive(Debug, Clone)]
struct ScenarioDefinition {
    id: String,
    name: Option<String>,
    script: String,
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
    /// Def rank symbols (C4Def::pRankSymbols, src/C4Def.cpp:684-691).
    rank_symbols_image: Option<GraphicsImage>,
    resource_group: Option<Group>,
    components: Vec<DefinitionComponent>,
    line_connect: u32,
    /// DefCore shape vertices + rect (the spawn shape; task #15 carries
    /// the rest of the core).
    vertices: Vec<lc_resources::definition::DefVertex>,
    shape: Option<lc_resources::definition::PictureRect>,
    /// The FULL DefCore for legacy defs — applied via
    /// Engine::apply_resource_core so no core field silently drops
    /// (physicals/Float/Timer/Grab did).
    core: Option<lc_resources::definition::DefCore>,
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
    graphics: HashMap<String, DefinitionActionGraphics>,
}

#[derive(Debug, Clone)]
struct ScenarioSpawn {
    handle: Option<String>,
    container_handle: Option<String>,
    config: SpawnConfig,
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

#[derive(Debug, Clone)]
enum DefinitionLoadStep {
    Definition(String),
    Declarations { name: String, source: String },
    SystemScripts(Vec<(String, String)>),
}

#[derive(Debug)]
enum CollectedDefinition {
    Definition(ScenarioDefinition),
    SystemScripts(Vec<(String, String)>),
}

#[derive(Debug)]
pub struct Scenario {
    name: Option<String>,
    description: Option<String>,
    ticks: Option<u32>,
    ground_height_hint: Option<i32>,
    definitions: Vec<ScenarioDefinition>,
    initial_spawns: Vec<ScenarioSpawn>,
    landscape: Option<Landscape>,
    physics: Option<PhysicsSettings>,
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
    landscape_insert_thrust: bool,
    /// `[Head] ForcedAutoStopControl`: `None` keeps the player-file
    /// preference; `Some` forces classic/Jump'n'Run control for all players
    /// (C4Player::ApplyForcedControl, C4Player.cpp:2369-2389).
    forced_control_style: Option<bool>,
    /// The surviving definition hosts and definition-pack System.c4g hosts in
    /// their C4DefList::Load order. System hosts remain in place when a later
    /// definition overload removes an earlier same-ID definition host.
    definition_load_steps: Vec<DefinitionLoadStep>,
    /// The scenario's own System.c4g sources. C++ loads these after defs
    /// specifically to give them overload priority (C4Game.cpp:2606-2617).
    scenario_system_scripts: Vec<(String, String)>,
    /// The four C4SPlrStart slots, consumed by joining players
    /// (C4Player::ScenarioInit, C4Player.cpp:670-777).
    player_starts: Vec<PlayerStart>,
    /// Ordered `Game.Teams` entries from the scenario's Teams.txt.
    teams: Vec<TeamInfo>,
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

/// The C4Game::InitGame environment-placement inputs (C4Game.cpp:
/// 2493-2503): Scenario.txt `[Landscape] Vegetation=/InEarth=`,
/// `[Animals]`, `[Environment] Objects=` and `[Game] Goals=/Rules=`,
/// plus the NoInitialize gate and the MEarth material name.
#[derive(Debug, Clone, Default)]
pub(crate) struct LegacyInitPlacement {
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
    pub(crate) create_objects: Vec<CreateObjectObjective>,
    pub(crate) clear_objects: Vec<ClearObjectObjective>,
    pub(crate) clear_materials: Vec<ClearMaterialObjective>,
}

#[derive(Debug, Clone)]
pub struct CreateObjectObjective {
    pub(crate) definition: String,
    pub(crate) count: i32,
}

#[derive(Debug, Clone)]
pub struct ClearObjectObjective {
    pub(crate) definition: String,
    pub(crate) count: i32,
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
}

impl Scenario {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ScenarioError> {
        let group = Group::open(path)?;
        Self::load_from_group(&group)
    }

    pub fn load_from_path_with<R: LegacyDefinitionResolver>(
        path: impl AsRef<Path>,
        resolver: &R,
    ) -> Result<Self, ScenarioError> {
        Self::load_from_path_with_languages(path, resolver, &["US", "DE"])
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
        let group = Group::open(path)?;
        Self::load_from_group_with_languages(&group, resolver, languages)
    }

    pub fn load_from_group_with<R: LegacyDefinitionResolver>(
        group: &Group,
        resolver: &R,
    ) -> Result<Self, ScenarioError> {
        Self::load_from_group_with_languages(group, resolver, &["US", "DE"])
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
        match Self::load_from_group(group) {
            Ok(scenario) => Ok(scenario),
            Err(ScenarioError::ManifestMissing) => {
                Self::load_legacy_from_group(group, resolver, languages)
            }
            Err(err) => Err(err),
        }
    }

    pub fn load_from_group(group: &Group) -> Result<Self, ScenarioError> {
        let manifest_bytes = match group.read_file("Scenario.json") {
            Ok(bytes) => bytes,
            Err(GroupError::EntryNotFound(_)) => return Err(ScenarioError::ManifestMissing),
            Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ScenarioError::ManifestMissing)
            }
            Err(error) => return Err(ScenarioError::Resources(error)),
        };

        let manifest: ScenarioManifest = serde_json::from_slice(&manifest_bytes)?;
        Scenario::from_manifest(group, manifest)
    }

    fn load_legacy_from_group<R, S>(
        group: &Group,
        resolver: &R,
        languages: &[S],
    ) -> Result<Self, ScenarioError>
    where
        R: LegacyDefinitionResolver,
        S: AsRef<str>,
    {
        let manifest = parse_legacy_scenario_manifest(group)?;

        let skip_ids: HashSet<String> = manifest
            .core
            .definitions
            .skip_defs
            .iter()
            .map(|entry| entry.id.to_ascii_uppercase())
            .collect();
        let mut load_items = Vec::new();

        // C4Game::InitDefs loads explicit definition resources first, then
        // folder-local resources from outermost to innermost, and finally the
        // scenario group itself (C4Game.cpp:81-103, 210-213, 3961-3994).
        for spec in &manifest.definition_specs {
            let groups = resolver.resolve_definition_groups(group, spec)?;
            if groups.is_empty() {
                return Err(ScenarioError::LegacyDefinitionNotFound { path: spec.clone() });
            }
            for definition_group in groups {
                collect_definitions_from_group(
                    &definition_group,
                    true,
                    &skip_ids,
                    languages,
                    &mut load_items,
                )?;
            }
        }

        let mut folder_groups = Vec::new();
        let mut ancestor = group.root().parent();
        while let Some(folder) = ancestor {
            let is_folder_group = folder.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".c4f")
            });
            if !is_folder_group {
                break;
            }
            folder_groups.push(folder.to_path_buf());
            ancestor = folder.parent();
        }
        for folder in folder_groups.iter().rev() {
            if let Ok(folder_group) = Group::open(folder) {
                collect_definitions_from_group(
                    &folder_group,
                    true,
                    &skip_ids,
                    languages,
                    &mut load_items,
                )?;
            }
        }

        // InitDefs' scenario pass disables System.c4g discovery because the
        // scenario-local group is loaded later by LoadScenarioScripts.
        collect_definitions_from_group(group, false, &skip_ids, languages, &mut load_items)?;

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
            }
        }

        if collected.is_empty() {
            return Err(ScenarioError::NoDefinitions);
        }

        let script = load_legacy_scenario_script(group, languages)?;
        let mut classifier = build_map_pixel_classifier(group, resolver);
        let landscape = load_legacy_landscape(group, &manifest, classifier.as_mut())?;
        // Crew never spawns at scenario load: C4Game::InitPlayers queues
        // CID_JoinPlr and C4Player::ScenarioInit places crew at JOIN time
        // (C4Player.cpp:481-570) — see Engine::join_player.
        let initial_spawns = collect_legacy_objects(group, &collected)?;
        let (physics, gravity) = derive_legacy_physics(&manifest)?;
        let environment = derive_legacy_environment(&manifest)?;
        let weather_init = derive_legacy_weather_init(&manifest)?;
        let teams = load_legacy_teams(group, languages)?;
        // C4Sky always initializes for a running game (C4Sky::Init,
        // C4Sky.cpp:71-152): bitmap sky or fade gradient.
        let sky = derive_legacy_sky(group, &manifest);

        Ok(Self {
            name: manifest.title,
            description: manifest.description,
            ticks: None,
            ground_height_hint: manifest.ground_height_hint,
            definitions: collected,
            initial_spawns,
            landscape,
            physics,
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
            landscape_insert_thrust: manifest.core.game.realism.landscape_insert_thrust != 0,
            forced_control_style: (manifest.core.head.forced_control_style >= 0)
                .then_some(manifest.core.head.forced_control_style != 0),
            definition_load_steps,
            scenario_system_scripts: load_scenario_system_scripts(group)?,
            player_starts: PlayerStart::slots_from_legacy(&manifest.core.players),
            teams,
            standard_names: group
                .read_file("Names.txt")
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            map_zoom: manifest.core.landscape.map_zoom,
            init_placement: Some(LegacyInitPlacement {
                no_initialize: manifest.core.head.no_initialize,
                vegetation: id_list_pairs(&manifest.core.landscape.vegetation),
                vegetation_level: manifest.core.landscape.vegetation_level,
                in_earth: id_list_pairs(&manifest.core.landscape.in_earth),
                in_earth_level: manifest.core.landscape.in_earth_level,
                animals: id_list_pairs(&manifest.core.animals.free_life),
                nests: id_list_pairs(&manifest.core.animals.earth_nest),
                environment: id_list_pairs(&manifest.core.environment.objects),
                goals: id_list_pairs(&manifest.core.game.goals),
                rules: id_list_pairs(&manifest.core.game.rules),
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

    pub fn physics(&self) -> Option<PhysicsSettings> {
        self.physics
    }

    pub fn environment(&self) -> Option<EnvironmentSettings> {
        self.environment
    }

    pub fn sky(&self) -> Option<&SkyConfig> {
        self.sky.as_ref()
    }

    /// Applies the scenario through C4Game::InitGame and the subsequent
    /// synchronization pass, but deliberately leaves Script.Initialize for
    /// the caller. Fresh games use this boundary to run InitGameFinal before
    /// the queued startup-player controls execute.
    pub fn apply_before_players(
        &self,
        engine: &mut Engine,
    ) -> Result<Vec<ObjectId>, ScenarioError> {
        engine.clear_scenario_script();
        engine.configure_objectives(self.objectives.clone());
        // C4Player::InitControl applies the scenario head override to every
        // subsequent join (C4Player.cpp:1747,2369-2389).
        engine.set_forced_control_style(self.forced_control_style);
        // C4SPlrStart outlives scenario load: ScenarioInit reads it when a
        // player joins (C4Player.cpp:670-777).
        engine.set_player_starts(self.player_starts.clone());
        engine.set_teams(self.teams.clone());
        engine.set_map_zoom(self.map_zoom);
        // A scenario Names.txt overrides the standard clonk names
        // (C4Game.cpp:3288-3289); without one the installer's choice (the
        // planet System.c4g Names.txt) stays in place.
        if self.standard_names.is_some() {
            engine.set_standard_names(self.standard_names.clone());
        }
        if let Some(landscape) = &self.landscape {
            engine.set_landscape(landscape.clone());
        } else {
            engine.clear_landscape();
        }

        // C4Landscape::ScenarioInit evaluates Gravity through the synced
        // ledger (C4Landscape.cpp:66) BEFORE Weather.Init's draws —
        // probe-verified: the C++ pre-wind sequence is [Gravity r=1,
        // Season r=1, YearSpeed r=1, Climate r=1, Wind r=151]. C4S always
        // has a Landscape block (defaults), so the draw is unconditional
        // on the legacy path; skipping it shifted every weather value by
        // one ledger position (the 402 Breeze/Still wind class).
        let scenario_gravity = self
            .weather_init
            .is_some()
            .then(|| engine.evaluate_scenario_gravity(self.gravity));
        if let Some(mut physics) = self.physics {
            if let Some(gravity) = scenario_gravity {
                physics.gravity = gravity;
            }
            engine.set_physics(physics);
        }

        engine.set_environment(self.environment.unwrap_or_default());
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
        engine.set_landscape_insert_thrust(self.landscape_insert_thrust);

        for step in &self.definition_load_steps {
            let definition = match step {
                DefinitionLoadStep::Definition(id) => self
                    .definitions
                    .iter()
                    .find(|definition| definition.id.eq_ignore_ascii_case(id))
                    .ok_or_else(|| ScenarioError::UnknownDefinition(id.clone()))?,
                DefinitionLoadStep::Declarations { name, source } => {
                    match lc_script::Script::compile(source) {
                        Ok(script) => lc_script::register_global_declarations(
                            script.var_decls(),
                            &engine.script_globals,
                            Some(&engine.script_global_consts),
                        ),
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
            // Real content gets the C++ callback arguments (no parameters;
            // AbortCall gets the last phase — C4Object.cpp:4154-4182).
            compiled.set_c4_callback_convention(true);
            // Legacy defs carry the FULL DefCore: apply it wholesale
            // (physicals, Float, Timer/TimerCall, Grab, fire properties,
            // ... — the hand-picked setters below only cover the DSL
            // manifest surface and silently dropped the rest).
            if let Some(core) = &definition.core {
                Engine::apply_resource_core(&mut compiled, core);
            }
            if let Some(actions) = &definition.actions {
                compiled.configure_actions(actions.default_action.clone(), actions.specs.clone());
                compiled.configure_action_graphics(actions.graphics.clone());
            }
            compiled.set_crew_member(definition.crew_member);
            compiled.set_can_be_base(definition.can_be_base);
            // DefCore shape: the spawn vertices C++ takes from the def
            // (C4Def Vertices/VertexX/...); without them every spawned
            // object compared vertex-less against the C++ snapshot.
            compiled.set_shape_rect(definition.shape.map(crate::DefinitionRect::from));
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
            // ClonkNames{lang}.txt|ClonkNames.txt (C4CFN_ClonkNames,
            // C4Def.cpp:645-652): the language-suffixed list first, then
            // the plain one. Only US is consulted until the language
            // config is ported.
            compiled.set_clonk_names(definition.resource_group.as_ref().and_then(|group| {
                ["ClonkNamesUS.txt", "ClonkNames.txt"]
                    .iter()
                    .find_map(|name| group.read_file(name).ok())
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            }));
            compiled.set_movement_profile(definition.movement);
            compiled.set_category(definition.category);
            compiled.set_value(definition.value);
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
            compiled.set_portrait_graphics_image(
                definition.portrait_graphics_image.as_ref().map(|image| {
                    DefinitionPictureImage::from_resource(
                        image,
                        definition.portrait_color_by_owner_mask.as_ref(),
                    )
                }),
            );
            compiled.set_rank_symbols_image(
                definition
                    .rank_symbols_image
                    .as_ref()
                    .map(|image| DefinitionPictureImage::from_resource(image, None)),
            );
            let sprite_image = definition.graphics_image.as_ref().map(|image| {
                DefinitionSpriteImage::from_resource(image, definition.color_by_owner_mask.as_ref())
            });
            compiled.set_sprite_image(sprite_image);
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
                        "scenario script failed to load; continuing without it like C++"
                    );
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
        // register_definition already inserted each definition's global funcs
        // in load order. Re-collecting here would put them above the scenario
        // System.c4g that C++ deliberately loaded last.

        let mut pending = self.initial_spawns.clone();
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
            engine.run_legacy_init_placements(placement);
        }
        if let Some(weather_init) = self.weather_init.as_ref() {
            // C4Weather::Init runs at the END of C4Game::InitGame after
            // Landscape.ScenarioInit's Gravity draw and the placements
            // (C4Game.cpp:2507).
            engine.apply_weather_init(weather_init)?;
        }

        // C4Game::Init tail: SyncClearance + Synchronize AFTER InitGame,
        // BEFORE InitPlayers (C4Game.cpp:474-475) — collapse every fixed
        // position to itofix(x,y,r) and re-fix the synced RNG. A no-op
        // for synthetic scenarios (created spawns already satisfy both).
        engine.inherit_include_clonk_names();
        engine.game_start_synchronize();
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
                    graphics: HashMap::new(),
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
                // Use lossy UTF-8 conversion to handle legacy ISO-8859-1/Windows-1252 encoded scripts
                let script_source = String::from_utf8_lossy(&script_bytes).into_owned();
                ScenarioDefinition {
                    id: id.clone(),
                    name: name_override.clone(),
                    script: script_source,
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
                    rank_symbols_image: None,
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

            scenario_definition.crew_member = crew_member || scenario_definition.crew_member;

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
            // Use lossy UTF-8 conversion to handle legacy ISO-8859-1/Windows-1252 encoded scripts
            let script_source = String::from_utf8_lossy(&script_bytes).into_owned();
            Some(ScenarioScriptSource {
                name: path,
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
            name: manifest.name,
            description: manifest.description,
            ticks: manifest.ticks,
            ground_height_hint,
            definitions,
            initial_spawns: spawns,
            landscape,
            physics,
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
            landscape_insert_thrust: false,
            forced_control_style: None,
            definition_load_steps,
            scenario_system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
            teams: Vec::new(),
            standard_names: None,
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            init_placement: None,
        })
    }
}

struct LegacyScenarioManifest {
    title: Option<String>,
    description: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyIdEntry {
    id: String,
    count: Option<i32>,
}

type LegacyIdList = Vec<LegacyIdEntry>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyNameEntry {
    name: String,
    count: Option<i32>,
}

type LegacyNameList = Vec<LegacyNameEntry>;

#[derive(Debug, Clone, Default)]
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

#[derive(Debug, Clone)]
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
    save_game: bool,
    replay: bool,
    film: i32,
    disable_mouse: bool,
    no_initialize: bool,
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
            save_game: false,
            replay: false,
            film: 0,
            disable_mouse: false,
            no_initialize: false,
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

#[derive(Debug, Clone, Default)]
struct LegacyDefinitions {
    local_only: bool,
    allow_user_change: bool,
    definitions: Vec<String>,
    skip_defs: LegacyIdList,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
struct LegacyPlayer {
    standard_crew: Option<String>,
    clonks: LegacyC4SVal,
    wealth: LegacyC4SVal,
    position: [i32; 2],
    enforce_position: bool,
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
            enforce_position: false,
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
        // A bare `ID` entry counts as 1 (C4IDList textual entries always
        // carry counts; the default covers hand-written content), while an
        // explicit `ID=0` stays zero (GoldRush pins `Magic=EXTG=0;`).
        let id_list = |entries: &LegacyIdList| {
            entries
                .iter()
                .map(|entry| (entry.id.clone(), entry.count.unwrap_or(1)))
                .collect()
        };
        Self {
            native_crew: player.standard_crew.clone(),
            crew_count: player.clonks,
            wealth: player.wealth,
            position: player.position,
            enforce_position: player.enforce_position,
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, Default)]
struct LegacyAnimals {
    free_life: LegacyIdList,
    earth_nest: LegacyIdList,
}

#[derive(Debug, Clone, Default)]
struct LegacyEnvironment {
    objects: LegacyIdList,
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

fn parse_legacy_id_list(field: &str, raw: &str) -> Result<LegacyIdList, ScenarioError> {
    let mut entries = Vec::new();
    for token in raw.split(';') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, '=');
        let id_part = parts.next().unwrap().trim();
        if id_part.is_empty() {
            return Err(ScenarioError::LegacyParse(format!(
                "missing identifier in `{field}` entry `{trimmed}`"
            )));
        }
        let normalized = id_part.to_ascii_uppercase();
        let count = match parts.next() {
            Some(value_part) => {
                let value_trimmed = value_part.trim();
                if value_trimmed.is_empty() {
                    None
                } else {
                    Some(parse_i32(value_trimmed).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid count `{value_trimmed}` for `{field}` entry `{trimmed}`: {err}"
                        ))
                    })?)
                }
            }
            None => None,
        };
        entries.push(LegacyIdEntry {
            id: normalized,
            count,
        });
    }
    Ok(entries)
}

fn parse_legacy_name_list(field: &str, raw: &str) -> Result<LegacyNameList, ScenarioError> {
    let mut entries = Vec::new();
    for token in raw.split(';') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, '=');
        let name_part = parts.next().unwrap().trim();
        if name_part.is_empty() {
            return Err(ScenarioError::LegacyParse(format!(
                "missing name in `{field}` entry `{trimmed}`"
            )));
        }
        let count = match parts.next() {
            Some(value_part) => {
                let value_trimmed = value_part.trim();
                if value_trimmed.is_empty() {
                    None
                } else {
                    Some(parse_i32(value_trimmed).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid count `{value_trimmed}` for `{field}` entry `{trimmed}`: {err}"
                        ))
                    })?)
                }
            }
            None => None,
        };
        entries.push(LegacyNameEntry {
            name: name_part.to_string(),
            count,
        });
    }
    Ok(entries)
}

fn parse_legacy_version(field: &str, raw: &str) -> Result<[i32; 5], ScenarioError> {
    let mut version = [0; 5];
    for (index, fragment) in raw.split(',').enumerate() {
        if index >= version.len() {
            break;
        }
        let trimmed = fragment.trim();
        if trimmed.is_empty() {
            continue;
        }
        version[index] = parse_i32(trimmed).map_err(|err| {
            ScenarioError::LegacyParse(format!(
                "invalid version component `{trimmed}` for `{field}`: {err}"
            ))
        })?;
    }
    Ok(version)
}

fn parse_base_functionality(field: &str, raw: &str) -> Result<i32, ScenarioError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(BASEFUNC_DEFAULT);
    }
    if let Ok(value) = parse_i32(trimmed) {
        return Ok(value);
    }
    let mut value = 0;
    for token in trimmed.split(['|', ',', '&']) {
        let entry = token.trim();
        if entry.is_empty() {
            continue;
        }
        let flag = match entry {
            "BASEFUNC_Default" => BASEFUNC_DEFAULT,
            "BASEFUNC_AutoSellContents" => BASEFUNC_AUTO_SELL_CONTENTS,
            "BASEFUNC_RegenerateEnergy" => BASEFUNC_REGENERATE_ENERGY,
            "BASEFUNC_Buy" => BASEFUNC_BUY,
            "BASEFUNC_Sell" => BASEFUNC_SELL,
            "BASEFUNC_RejectEntrance" => BASEFUNC_REJECT_ENTRANCE,
            "BASEFUNC_Extinguish" => BASEFUNC_EXTINGUISH,
            other => {
                return Err(ScenarioError::LegacyParse(format!(
                    "unknown BaseFunctionality token `{other}` in `{field}`"
                )))
            }
        };
        if flag == BASEFUNC_DEFAULT {
            value |= BASEFUNC_DEFAULT;
        } else {
            value |= flag;
        }
    }
    if value == 0 {
        Ok(0)
    } else {
        Ok(value)
    }
}

fn parse_i32_array<const N: usize>(field: &str, raw: &str) -> Result<[i32; N], ScenarioError> {
    let mut result = [0; N];
    for (index, fragment) in raw.split([',', ';']).enumerate() {
        if index >= N {
            break;
        }
        let trimmed = fragment.trim();
        if trimmed.is_empty() {
            continue;
        }
        result[index] = parse_i32(trimmed).map_err(|err| {
            ScenarioError::LegacyParse(format!(
                "invalid value `{trimmed}` for `{field}` component {index}: {err}"
            ))
        })?;
    }
    Ok(result)
}

fn parse_position(field: &str, raw: &str) -> Result<[i32; 2], ScenarioError> {
    let mut result = [-1, -1];
    let mut parts = raw
        .split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty());
    if let Some(x) = parts.next() {
        result[0] = parse_i32(x).map_err(|err| {
            ScenarioError::LegacyParse(format!("invalid x coordinate `{x}` for `{field}`: {err}"))
        })?;
    }
    if let Some(y) = parts.next() {
        result[1] = parse_i32(y).map_err(|err| {
            ScenarioError::LegacyParse(format!("invalid y coordinate `{y}` for `{field}`: {err}"))
        })?;
    }
    Ok(result)
}

impl LegacyHead {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
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
                    self.save_game = parse_bool_field(key, raw)?;
                }
                "replay" => {
                    self.replay = parse_bool_field(key, raw)?;
                }
                "film" => {
                    self.film = parse_i32(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                }
                "disablemouse" => {
                    self.disable_mouse = parse_bool_field(key, raw)?;
                }
                "noinitialize" => {
                    self.no_initialize = parse_bool_field(key, raw)?;
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
                        self.mission_access = raw.to_string();
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
        Ok(())
    }
}

impl LegacyDefinitions {
    fn apply_entries(&mut self, entries: &[(String, String)]) -> Result<(), ScenarioError> {
        for (key, value) in entries {
            let key_lower = key.to_ascii_lowercase();
            let raw = value.trim();
            match key_lower.as_str() {
                "localonly" => {
                    self.local_only = parse_bool_field(key, raw)?;
                }
                "allowuserchange" => {
                    self.allow_user_change = parse_bool_field(key, raw)?;
                }
                "definitions" => {
                    for fragment in raw.split([';', ',']) {
                        let trimmed = fragment.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        self.definitions.push(normalize_definition_path(trimmed));
                    }
                }
                _ if key_lower.starts_with("definition") => {
                    for fragment in raw.split([';', ',']) {
                        let trimmed = fragment.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        self.definitions.push(normalize_definition_path(trimmed));
                    }
                }
                "skipdefs" => {
                    self.skip_defs = parse_legacy_id_list(key, raw)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl LegacyGame {
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
                    self.clear_materials = parse_legacy_name_list(key, raw)?;
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
                    let parsed = parse_i64(raw).map_err(|err| {
                        ScenarioError::LegacyParse(format!(
                            "invalid value `{raw}` for `{key}`: {err}"
                        ))
                    })?;
                    if parsed < 0 || parsed > u32::MAX as i64 {
                        return Err(ScenarioError::LegacyParse(format!(
                            "value `{raw}` for `{key}` is out of range"
                        )));
                    }
                    self.fow_color = parsed as u32;
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
                    if raw.is_empty() {
                        self.standard_crew = None;
                    } else {
                        self.standard_crew = Some(raw.to_ascii_uppercase());
                    }
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
                    self.enforce_position = parse_bool_field(key, raw)?;
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
                    self.layers = parse_legacy_name_list(key, raw)?;
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

impl LegacyScenarioCore {
    fn from_sections(
        sections: &HashMap<String, Vec<(String, String)>>,
    ) -> Result<Self, ScenarioError> {
        let mut core = LegacyScenarioCore::default();
        if let Some(entries) = sections.get("head") {
            core.head.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("definitions") {
            core.definitions.apply_entries(entries)?;
        }
        if let Some(entries) = sections.get("game") {
            core.game.apply_entries(entries)?;
        }
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
}

fn parse_legacy_scenario_manifest(group: &Group) -> Result<LegacyScenarioManifest, ScenarioError> {
    let bytes = match group.read_file("Scenario.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Err(ScenarioError::LegacyCoreMissing),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ScenarioError::LegacyCoreMissing)
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    let text = String::from_utf8(bytes).map_err(|_| ScenarioError::LegacyCoreEncoding)?;
    parse_legacy_scenario_text(&text)
}

fn parse_legacy_scenario_text(text: &str) -> Result<LegacyScenarioManifest, ScenarioError> {
    let mut sections: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut current_section: Option<String> = None;

    for raw_line in text.lines() {
        let mut line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with("//") {
            continue;
        }
        if let Some(idx) = line.find("//") {
            line = line[..idx].trim_end();
        }
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].trim().to_ascii_lowercase();
            current_section = Some(name.clone());
            sections.entry(name).or_default();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Values before the first section header are ROOT-level in the
        // C++ INI tree (StdCompilerINIRead::CreateNameTree,
        // StdCompiler.cpp:794-860) — invisible to the [Head] naming.
        // Shipped content depends on it: Tournament.c4s's mangled `Head]`
        // line makes C++ compile a DEFAULT head (NoInitialize=0), so its
        // goal/rule/vegetation placements DO run.
        let Some(section) = current_section.clone() else {
            continue;
        };
        sections
            .entry(section)
            .or_default()
            .push((key.trim().to_string(), value.trim().to_string()));
    }

    let mut seen_specs = HashSet::new();
    let mut definition_specs = Vec::new();
    if let Some(def_entries) = sections.get("definitions") {
        for (key, value) in def_entries {
            if !key.to_ascii_lowercase().starts_with("definition") {
                continue;
            }
            for fragment in split_definition_values(value) {
                if seen_specs.insert(fragment.clone()) {
                    definition_specs.push(fragment);
                }
            }
        }
    }

    let title = sections
        .get("head")
        .and_then(|entries| find_entry(entries, "title"));

    let description = sections
        .get("head")
        .and_then(|entries| find_entry(entries, "description"));

    let ground_height_hint = derive_ground_height_hint(&sections);
    let core = LegacyScenarioCore::from_sections(&sections)?;

    Ok(LegacyScenarioManifest {
        title,
        description,
        definition_specs,
        ground_height_hint,
        core,
        sections,
    })
}

fn find_entry(entries: &[(String, String)], key: &str) -> Option<String> {
    entries
        .iter()
        .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
        .and_then(|(_, value)| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
}

fn split_definition_values(raw: &str) -> Vec<String> {
    raw.split(';')
        .map(|fragment| fragment.trim())
        .filter(|fragment| !fragment.is_empty())
        .map(normalize_definition_path)
        .collect()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn base(self) -> i32 {
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

    fn variation_extent(self) -> i32 {
        let base = self.base();
        let (min, max) = ordered_bounds(self.min, self.max);
        let positive = max.saturating_sub(base).max(0);
        let negative = base.saturating_sub(min).max(0);
        let range = positive.max(negative);
        range.min(self.rnd.abs())
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

fn load_legacy_scenario_script<S: AsRef<str>>(
    group: &Group,
    languages: &[S],
) -> Result<Option<ScenarioScriptSource>, ScenarioError> {
    const SCRIPT_CANDIDATES: [&str; 1] = ["Script.c"];
    for candidate in SCRIPT_CANDIDATES {
        if !group.exists(candidate) {
            continue;
        }
        let bytes = group.read_file(candidate)?;
        let source = decode_legacy_script_text(&bytes);
        let source = localize_script_source(group, &source, languages)?;
        return Ok(Some(ScenarioScriptSource {
            name: candidate.to_string(),
            source,
            c4_args: true,
        }));
    }
    Ok(None)
}

fn load_legacy_teams<S: AsRef<str>>(
    group: &Group,
    languages: &[S],
) -> Result<Vec<TeamInfo>, ScenarioError> {
    if !group.exists("Teams.txt") {
        return Ok(Vec::new());
    }
    let source = decode_legacy_script_text(&group.read_file("Teams.txt")?);
    let source = localize_script_source(group, &source, languages)?;
    Ok(parse_legacy_teams_source(&source))
}

fn parse_legacy_teams_source(source: &str) -> Vec<TeamInfo> {
    let mut teams = Vec::new();
    let mut current = None;
    for line in source.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(team) = current.take() {
                teams.push(team);
            }
            let section = line[1..line.len() - 1].trim();
            current = section
                .eq_ignore_ascii_case("Team")
                .then(|| TeamInfo::new(0, "", 0));
            continue;
        }
        let Some(team) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "id" => {
                if let Ok(id) = value.trim().parse() {
                    team.id = id;
                }
            }
            "name" => team.name = value.trim().to_string(),
            "color" => {
                if let Ok(color) = value.trim().parse() {
                    team.color = color;
                }
            }
            _ => {}
        }
    }
    if let Some(team) = current {
        teams.push(team);
    }
    teams
}

/// Collects the scripts of a System.c4g group in the group's existing entry
/// order, matching C4Group::FindNextEntry (C4Game.cpp:3348-3355).
pub fn load_system_scripts(group: &Group) -> Result<Vec<(String, String)>, ScenarioError> {
    let mut sources = Vec::new();
    for entry in group.entries()? {
        if entry.is_directory {
            continue;
        }
        let name = entry.relative_path.to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".c") {
            continue;
        }
        let bytes = group.read_file(&entry.relative_path)?;
        sources.push((name, String::from_utf8_lossy(&bytes).into_owned()));
    }
    Ok(sources)
}

/// The scenario's own System.c4g scripts, empty when the group has none
/// (C4Game::LoadScenarioScripts opens C4CFN_System as a child and loads
/// every C4CFN_ScriptFiles entry, C4Game.cpp:3317-3343).
fn load_scenario_system_scripts(group: &Group) -> Result<Vec<(String, String)>, ScenarioError> {
    group
        .open_child(Path::new("System.c4g"))
        .ok()
        .map(|system| load_system_scripts(&system))
        .unwrap_or_else(|| Ok(Vec::new()))
}

/// `[Landscape] MapZoom` with the C4S default `C4SVal(10, 0, 5, 15)`
/// (C4Scenario.cpp:307,353), Std bounded to [Min, Max] like Evaluate.
fn legacy_map_zoom(section: Option<&Vec<(String, String)>>) -> u32 {
    let default = LegacyC4SVal::new(10, 0, 5, 15);
    section
        .and_then(|entries| find_entry(entries, "mapzoom"))
        .and_then(|raw| parse_legacy_c4s_value("MapZoom", &raw, default).ok())
        .unwrap_or(default)
        .base()
        .max(1) as u32
}

/// The ChunkOZoom jitter seed: C++ draws `MapSeed = Random(3133700)` at
/// landscape init (C4Landscape.cpp:563). The shadow bridge hands the C++
/// value across via env; standalone runs jitter deterministically from 0.
fn legacy_map_seed() -> i32 {
    let map_seed = std::env::var("LC_RUST_ENGINE_MAP_SEED")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(0);
    if std::env::var("LC_DEBUG_MAP").is_ok() {
        eprintln!("RUST MAPSEED {map_seed}");
    }
    map_seed
}

/// `Game.FixRandom(Game.Parameters.RandomSeed)` before map creation
/// (C4Landscape.cpp:578): the map creators draw from a freshly fixed
/// ledger, and the bracket re-fixes afterwards (C4Landscape.cpp:734), so
/// map creation never shifts the post-init synced ledger. Standalone runs
/// use the engine's default seed 0; the shadow bridge can hand the C++
/// RandomSeed across via env.
fn legacy_map_creation_rng() -> crate::rng::LcgRng {
    let seed = std::env::var("LC_RUST_ENGINE_RANDOM_SEED")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    crate::rng::LcgRng::seed_from_u64(seed)
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
const C4M_MAX_TEX_INDEX: usize = 127;

pub(crate) struct MapPixelClassifier {
    state: RuntimeTexMapState,
}

impl MapPixelClassifier {
    pub(crate) fn from_runtime_state(state: RuntimeTexMapState) -> Self {
        Self { state }
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
            },
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
        library: lc_resources::MaterialLibrary,
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
            },
        }
    }

    fn runtime_material(material: &lc_resources::MaterialDefinition) -> RuntimeTexMapMaterial {
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
        self.state
            .texture_inventory
            .iter()
            .any(|t| t.eq_ignore_ascii_case(name))
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
        for slot in 1..C4M_MAX_TEX_INDEX {
            if let Some(existing) = &self.state.material_names[slot] {
                if existing.eq_ignore_ascii_case(mat_name)
                    && tex_name
                        .map(|tex| {
                            self.state.match_texture_names[slot]
                                .as_deref()
                                .is_some_and(|t| t.eq_ignore_ascii_case(tex))
                        })
                        .unwrap_or(true)
                {
                    return slot as u8;
                }
            }
        }
        if !add_if_not_exist {
            return 0;
        }
        let Some(material) = self.material(mat_name) else {
            return 0;
        };
        let shape = material.shape;
        let density = material.density;
        if let Some(tex) = tex_name {
            if !self.texture_exists(tex) {
                return 0;
            }
        }
        let Some(slot) = (1..C4M_MAX_TEX_INDEX)
            .find(|&slot| self.state.material_names[slot].is_none()) else {
            return 0;
        };
        self.state.material_names[slot] = Some(mat_name.to_string());
        self.state.match_texture_names[slot] = tex_name.map(str::to_string);
        self.state.texture_names[slot] = tex_name.map(str::to_string);
        self.state.shapes[slot] = Some(shape);
        self.state.densities[slot] = density;
        slot as u8
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
        let (material, texture) = match material_texture.split_once('-') {
            Some((material, texture)) => (material, Some(texture)),
            None => (material_texture, None),
        };
        if let Some(texture) = texture {
            let index = self.get_index(material, Some(texture), true);
            if index != 0 {
                return index;
            }
        }
        if let Some(default) = default_texture {
            let index = self.get_index(material, Some(default), true);
            if index != 0 {
                return index;
            }
        }
        // Game.Material.Map[iMaterial].DefaultMatTex: the exact entry stored
        // by CrossMapMaterials, not the first texmap slot with this material
        // (C4Material.cpp:349-370; C4Texture.cpp:360-367).
        self.state.default_material_entry(material).unwrap_or(0)
    }
}

/// TexMap.txt + material densities, scenario-local Material.c4g first
/// (C4Game::InitMaterialTexture loads the scenario's group before the
/// global one; OverloadMaterials in its TexMap.txt admits the global
/// material set). `None` when no TexMap.txt is reachable — the loader
/// then falls back to the sky-pixel heuristic.
pub(crate) fn build_map_pixel_classifier(
    group: &Group,
    resolver: &impl LegacyDefinitionResolver,
) -> Option<MapPixelClassifier> {
    let local = group.open_child("Material.c4g").ok();
    // The resolver lists the scenario-local group first — the GLOBAL one
    // is the first hit rooted elsewhere.
    let local_root = local.as_ref().map(|group| group.root().to_path_buf());
    let global = resolver
        .resolve_definition_groups(group, "Material.c4g")
        .ok()
        .into_iter()
        .flatten()
        .find(|candidate| local_root.as_deref() != Some(candidate.root()));
    let texmap_source = [local.as_ref(), global.as_ref()]
        .into_iter()
        .flatten()
        .find_map(|group| group.read_file("TexMap.txt").ok())?;
    let texmap = lc_resources::texmap::TextureMap::parse(&String::from_utf8_lossy(&texmap_source));

    let local_library = local
        .as_ref()
        .and_then(|group| lc_resources::MaterialLibrary::from_group(group).ok());
    let global_library = (local_library.is_none() || texmap.overload_materials)
        .then(|| {
            global
                .as_ref()
                .and_then(|group| lc_resources::MaterialLibrary::from_group(group).ok())
        })
        .flatten();

    // Collapse the loaded resource libraries into the only material fields
    // runtime texmap allocation consumes. Scenario-local definitions win
    // name collisions, matching C4MaterialMap::Get after overloaded loads.
    let runtime_materials = local_library
        .iter()
        .flat_map(|library| library.iter())
        .chain(
            global_library
                .iter()
                .flat_map(|library| library.iter())
                .filter(|definition| {
                    local_library
                        .as_ref()
                        .is_none_or(|local| local.get(definition.name()).is_none())
                }),
        )
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
        grid_textures[index] = texmap
            .entry(index as u8)
            .map(|entry| entry.texture.clone());
        let material = texmap.entry(index as u8).and_then(|entry| {
            local_library
                .as_ref()
                .and_then(|library| library.get(&entry.material))
                .or_else(|| {
                    global_library
                        .as_ref()
                        .and_then(|library| library.get(&entry.material))
                })
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
                .is_some_and(|texture| texture.eq_ignore_ascii_case("Smooth"))
        {
            grid_textures[index] = Some("Liquid".to_string());
        }
    }
    // Raw texmap textures for GetIndex pair matching.
    let mut match_textures: Vec<Option<String>> = vec![None; 128];
    for (index, slot) in match_textures.iter_mut().enumerate() {
        *slot = texmap
            .entry(index as u8)
            .map(|entry| entry.texture.clone());
    }
    // The texture inventory: image basenames in the material groups
    // (AddEntry validates the texture exists, C4Texture.cpp:116-131).
    let mut texture_inventory: Vec<String> = Vec::new();
    for group in [local.as_ref(), global.as_ref()].into_iter().flatten() {
        for entry in group.entries().unwrap_or_default() {
            let lower = entry.relative_path.to_string_lossy().to_ascii_lowercase();
            if let Some(stem) = lower
                .strip_suffix(".png")
                .or_else(|| lower.strip_suffix(".bmp"))
            {
                texture_inventory.push(stem.to_string());
            }
        }
    }

    // The C++ material-map order: each load PREPENDS new names
    // (scenario loads first, global after — C4Material.cpp:263-299)
    // → [global-uniques…, scenario…], scenario winning collisions.
    // Collected as owned (name, overlay, cross-specs) rows so the loops
    // below can mutate the classifier slots.
    let ordered: Vec<(String, Option<String>, Vec<String>)> = global_library
        .iter()
        .flat_map(|library| library.iter())
        .filter(|definition| {
            local_library
                .as_ref()
                .map(|local| local.get(definition.name()).is_none())
                .unwrap_or(true)
        })
        .chain(local_library.iter().flat_map(|library| library.iter()))
        .map(|material| {
            (
                material.value("Name").unwrap_or_default().to_string(),
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
        },
    };

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
            let (mat_name, tex_name) = match spec.split_once('-') {
                Some((mat, tex)) => (mat, Some(tex.to_string())),
                None => (spec.as_str(), None),
            };
            // GetIndexMatTex: exact pair first, then the material's
            // default entry stands in (no further add).
            classifier.get_index(mat_name, tex_name.as_deref(), true);
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
    Some(classifier)
}

/// Build the landscape from a classified 8-bit map: the map zooms through
/// ChunkOZoom into the Surface8 pixel plane (chunky material rims and
/// slope smoothers, C4Landscape::MapToSurface → TexOZoom → ChunkOZoom,
/// C4Landscape.cpp:336-480), then the column approximation — surface
/// heights, liquid segments, IFT tunnel ranges — derives from that plane.
fn classified_landscape(
    bitmap: &lc_resources::bitmap::IndexedBitmap,
    classifier: &MapPixelClassifier,
    zoom: i32,
    map_seed: i32,
) -> Result<Landscape, ScenarioError> {
    let map_width = bitmap.width as i32;
    let map_height = bitmap.height as i32;
    let world_height = map_height.saturating_mul(zoom).max(0);
    let final_width = bitmap.width.saturating_mul(zoom as u32);
    let plane_width = final_width as usize;

    let bytes = crate::chunky::synthesize_landscape(
        &bitmap.indices,
        map_width,
        map_height,
        zoom,
        map_seed,
        &classifier.state.shapes,
    )
    .into_bytes();
    let density_of = |byte: u8| classifier.state.densities[(byte & 127) as usize];

    let mut surfaces = Vec::with_capacity(plane_width);
    for x in 0..plane_width {
        let surface_world = (0..world_height)
            .find(|&y| density_of(bytes[y as usize * plane_width + x]) >= 50)
            .unwrap_or(world_height);
        surfaces.push(surface_world);
    }

    let mut landscape = Landscape::new(final_width, surfaces)
        .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
    landscape.set_world_height(world_height);

    for x in 0..plane_width {
        let mut segments = Vec::new();
        let mut tunnel_ranges = Vec::new();
        let mut run_start: Option<i32> = None;
        let mut tunnel_start: Option<i32> = None;
        for y in 0..=world_height {
            let pixel = (y < world_height).then(|| bytes[y as usize * plane_width + x]);
            let density = pixel.map(density_of).unwrap_or(0);
            let liquid = (25..50).contains(&density);
            match (liquid, run_start) {
                (true, None) => run_start = Some(y),
                (false, Some(start)) => {
                    segments.push(crate::landscape::LiquidSegment {
                        top: start,
                        bottom: y - 1,
                        material: None,
                    });
                    run_start = None;
                }
                _ => {}
            }
            let tunnel = pixel
                .map(|p| p & lc_resources::texmap::IFT_BIT != 0)
                .unwrap_or(false);
            match (tunnel, tunnel_start) {
                (true, None) => tunnel_start = Some(y),
                (false, Some(start)) => {
                    tunnel_ranges.push((start, y - 1));
                    tunnel_start = None;
                }
                _ => {}
            }
        }
        if !segments.is_empty() {
            landscape.set_liquid_column(x as u32, segments);
        }
        if !tunnel_ranges.is_empty() {
            landscape.set_tunnel_column(x as u32, tunnel_ranges);
        }
    }

    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        final_width,
        world_height.max(0) as u32,
        bytes,
        classifier.state.densities.clone(),
        classifier.state.material_names.clone(),
        classifier.state.texture_names.clone(),
    ));
    landscape.set_raster_state(LandscapeRasterState::new(
        zoom,
        map_seed,
        classifier.state.clone(),
    ));

    // Loaded water is at rest: C4MassMoverSet starts empty and movers are
    // created only by landscape CHANGES via CheckInstability, never at
    // load (C4Game.cpp:1782 MassMover.Default(); the c4b Load only fires
    // for saved games).
    Ok(landscape)
}

fn load_legacy_landscape(
    group: &Group,
    manifest: &LegacyScenarioManifest,
    classifier: Option<&mut MapPixelClassifier>,
) -> Result<Option<Landscape>, ScenarioError> {
    let Some(mut landscape) = load_legacy_landscape_body(group, manifest, classifier)? else {
        return Ok(None);
    };
    // C4Landscape::ScenarioInit (C4Landscape.cpp:67-73): the border-open
    // keys, then the AutoScanSideOpen side scan over the built landscape.
    let borders = &manifest.core.landscape;
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
    classifier: Option<&mut MapPixelClassifier>,
) -> Result<Option<Landscape>, ScenarioError> {
    let landscape_section = manifest.sections.get("landscape");
    let map_zoom_u32 = legacy_map_zoom(landscape_section);
    let map_width_hint = landscape_section
        .and_then(|entries| find_entry(entries, "mapwidth"))
        .and_then(|value| parse_c4sval_std(&value))
        .map(|value| value.max(1));
    let map_height_hint = landscape_section
        .and_then(|entries| find_entry(entries, "mapheight"))
        .and_then(|value| parse_c4sval_std(&value))
        .map(|value| value.max(1));
    let exact_landscape = landscape_section
        .and_then(|entries| find_entry(entries, "exactlandscape"))
        .and_then(|value| parse_legacy_bool(&value))
        .unwrap_or(false);

    let read_optional = |name: &str| match group.read_file(name) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(GroupError::EntryNotFound(_)) => Ok(None),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ScenarioError::Resources(error)),
    };

    // ExactLandscape: Landscape.bmp IS the landscape — C++ reads it
    // straight into the pixel surface (GroupReadSurface8), so it decodes
    // at pixel scale (zoom 1) here. Returning no landscape would leave
    // GBackSolid answering "never solid" and hang placement loops in real
    // content (Grass.c4d Initialize).
    let (map_bytes, map_zoom_u32) = if exact_landscape {
        match read_optional("Landscape.bmp")? {
            Some(bytes) => (Some(bytes), 1),
            None => (read_optional("Map.bmp")?, map_zoom_u32),
        }
    } else {
        // Static map: Map.bmp, with Landscape.bmp accepted as the map for
        // downwards compatibility (C4Landscape.cpp:593-601) — most CR
        // content (GoldRush included) ships only Landscape.bmp.
        match read_optional("Map.bmp")? {
            Some(bytes) => (Some(bytes), map_zoom_u32),
            None => (read_optional("Landscape.bmp")?, map_zoom_u32),
        }
    };

    let mut classifier = classifier;
    if let Some(bytes) = map_bytes {
        // Material-classified path: the map's 8-bit palette indices are
        // texmap keys (GroupReadSurface8 keeps the index bytes). Without
        // a TexMap or for non-indexed images, the sky-pixel heuristic
        // below stands in.
        if let Some(classifier) = classifier.take() {
            if let Ok(bitmap) = lc_resources::bitmap::IndexedBitmap::decode(&bytes) {
                return classified_landscape(
                    &bitmap,
                    classifier,
                    map_zoom_u32 as i32,
                    legacy_map_seed(),
                )
                .map(Some);
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

        let map_zoom_i32 = map_zoom_u32 as i32;
        let sky_pixel = rgba.get_pixel(0, 0).0;
        let world_height = (height as i32).saturating_mul(map_zoom_i32).max(0);
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

        let final_width = width.saturating_mul(map_zoom_u32);
        let mut landscape = Landscape::new(final_width, surfaces)
            .map_err(|error| ScenarioError::InvalidLandscape(error.to_string()))?;
        // GBackHgt is known exactly here (map height × zoom); placement
        // searches and `Random(GBackHgt - 32)` draws bound on it.
        landscape.set_world_height(world_height);
        return Ok(Some(landscape));
    }

    if exact_landscape {
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
        let mut map_rng = legacy_map_creation_rng();
        let players = legacy_startup_player_count();
        let landscape_core = &manifest.core.landscape;
        let mut retained_creator = None;
        let bitmap = if let Some(bytes) = read_optional("Landscape.txt")? {
            let creation = crate::map_creator_s2::create_s2_map_with_state(
                &String::from_utf8_lossy(&bytes),
                classifier,
                landscape_core.map_width,
                landscape_core.map_height,
                landscape_core.map_player_extend,
                players,
                &mut map_rng,
            );
            if landscape_core.keep_map_creator {
                retained_creator = Some(creation.creator);
            }
            creation.bitmap.unwrap_or_else(|| {
                // Dynamic map by scenario (C4Landscape.cpp:612-614) —
                // also the fallback when the exmap yields no map node.
                let params = basic_map_params(landscape_core);
                crate::map_creator::create_basic_map(&params, classifier, players, &mut map_rng)
            })
        } else {
            let params = basic_map_params(landscape_core);
            crate::map_creator::create_basic_map(&params, classifier, players, &mut map_rng)
        };
        let mut landscape =
            classified_landscape(&bitmap, classifier, map_zoom_u32 as i32, legacy_map_seed())?;
        landscape
            .raster_state_mut()
            .expect("classified landscapes carry raster state")
            .set_map_creator(retained_creator);
        return Ok(Some(landscape));
    }

    let fallback_map_width = map_width_hint.unwrap_or(96);
    let fallback_map_height = map_height_hint.unwrap_or(50);
    let width_product = i64::from(fallback_map_width).saturating_mul(i64::from(map_zoom_u32));
    let width_u32 = width_product
        .clamp(1, i64::from(u32::MAX))
        .try_into()
        .unwrap_or(u32::MAX);
    let fallback_height = fallback_map_height
        .saturating_mul(map_zoom_u32 as i32)
        .max(1);
    let mut landscape = Landscape::flat(width_u32, fallback_height);
    landscape.set_world_height(fallback_height);
    Ok(Some(landscape))
}

fn parse_legacy_c4s_value(
    field: &str,
    raw: &str,
    defaults: LegacyC4SVal,
) -> Result<LegacyC4SVal, ScenarioError> {
    let mut result = defaults;
    for (index, fragment) in raw.split(',').enumerate() {
        let trimmed = fragment.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = parse_i32(trimmed).map_err(|err| {
            ScenarioError::LegacyParse(format!(
                "invalid value `{trimmed}` for `{field}` component {index}: {err}"
            ))
        })?;

        match index {
            0 => result.std = parsed,
            1 => result.rnd = parsed,
            2 => result.min = parsed,
            3 => result.max = parsed,
            _ => break,
        }
    }
    Ok(result)
}

fn legacy_c4s_value(
    entries: Option<&Vec<(String, String)>>,
    key: &str,
    defaults: LegacyC4SVal,
) -> Result<LegacyC4SVal, ScenarioError> {
    match entries.and_then(|entries| find_entry(entries, key)) {
        Some(raw) => parse_legacy_c4s_value(key, &raw, defaults),
        None => Ok(defaults),
    }
}

/// C4Surface::LoadAny extension search order for extension-less names
/// (C4Surface.cpp:855).
const LEGACY_SKY_EXTENSIONS: [&str; 4] = ["png", "bmp", "jpeg", "jpg"];

/// The default sky fade when `SkyDefFade` is all zero: game palette
/// entries CSkyDef1=104 and 104+19 (C4Sky::SetFadePalette,
/// C4Sky.cpp:56-62; C4Landscape.h:34), scaled `<< 2` at load
/// (C4GraphicsResource.cpp:183-184). Values read from
/// planet/Graphics.c4g/C4.PAL.
const LEGACY_SKY_FADE_TOP_DEFAULT: RgbColor = RgbColor::new(28, 64, 152);
const LEGACY_SKY_FADE_BOTTOM_DEFAULT: RgbColor = RgbColor::new(192, 196, 252);

/// Mirrors C4Sky::Init for legacy scenario loads (C4Sky.cpp:71-152): the
/// scenario's `Sky` bitmap becomes the sky surface — white fade
/// (C4Sky.cpp:109), tiled up to 128x128 (SurfaceEnsureSize,
/// C4Sky.cpp:28-52,110-111), SkyScrollMode parallax mapping
/// (C4Sky.cpp:114-125). Without one the sky is the `SkyDefFade` gradient
/// (SetFadePalette, C4Sky.cpp:54-68).
///
/// Gap (presentation only): the `SkyDef` tile list — one entry picked by
/// SeededRandom and loaded from the scenario or Graphics.c4g
/// (C4Sky.cpp:88-105) — is not evaluated; such scenarios fall back to the
/// fade gradient here.
fn derive_legacy_sky(group: &Group, manifest: &LegacyScenarioManifest) -> SkyConfig {
    let mut settings = SkySettings::default();

    if let Some((width, height, pixels)) = load_legacy_sky_surface(group) {
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
        return SkyConfig {
            settings,
            surface: Some(Arc::new(GraphicsImage::new(width, height, pixels))),
        };
    }

    // No sky surface: fade gradient (C4Sky.cpp:129-134). All-zero
    // SkyDefFade selects the palette default (C4Sky.cpp:56-62).
    let fade = manifest.core.landscape.sky_fade;
    if fade.iter().sum::<i32>() == 0 {
        settings.fade_top = LEGACY_SKY_FADE_TOP_DEFAULT;
        settings.fade_bottom = LEGACY_SKY_FADE_BOTTOM_DEFAULT;
    } else {
        let channel = |value: i32| value.clamp(0, 255) as u8;
        settings.fade_top = RgbColor::new(channel(fade[0]), channel(fade[1]), channel(fade[2]));
        settings.fade_bottom = RgbColor::new(channel(fade[3]), channel(fade[4]), channel(fade[5]));
    }
    SkyConfig {
        settings,
        surface: None,
    }
}

/// Load `Sky.{png,bmp,jpeg,jpg}` from the scenario group (C4Sky.cpp:84 via
/// C4Surface::LoadAny, C4Surface.cpp:846-865). Like C++, a missing or
/// undecodable bitmap yields `None` (the sky falls back to the fade
/// gradient) instead of an error.
fn load_legacy_sky_surface(group: &Group) -> Option<(u32, u32, Vec<u8>)> {
    LEGACY_SKY_EXTENSIONS
        .iter()
        .filter_map(|extension| group.read_file(format!("Sky.{extension}")).ok())
        .find_map(|bytes| load_from_memory(&bytes).ok())
        .map(|decoded| {
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            (width, height, rgba.into_raw())
        })
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
/// block (:49-58).
#[derive(Debug, Clone)]
pub(crate) struct LegacyWeatherInit {
    pub(crate) season: LegacyC4SVal,
    pub(crate) year_speed: LegacyC4SVal,
    pub(crate) climate: LegacyC4SVal,
    pub(crate) wind: LegacyC4SVal,
    pub(crate) rain: LegacyC4SVal,
    pub(crate) precipitation: String,
    pub(crate) lightning: LegacyC4SVal,
    pub(crate) meteorite: LegacyC4SVal,
    pub(crate) volcano: LegacyC4SVal,
    pub(crate) earthquake: LegacyC4SVal,
    pub(crate) no_initialize: bool,
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
        no_initialize: manifest.core.head.no_initialize,
    })
}

fn derive_legacy_environment(
    manifest: &LegacyScenarioManifest,
) -> Result<EnvironmentSettings, ScenarioError> {
    let weather_entries = manifest.sections.get("weather");
    let disasters_entries = manifest.sections.get("disasters");

    let wind_defaults = LegacyC4SVal::new(0, 70, -100, 100);
    let wind = legacy_c4s_value(weather_entries, "wind", wind_defaults)?;
    let base_wind = wind.base();
    let wind_variation = wind.variation_extent();

    let mut environment = EnvironmentSettings::new(base_wind);
    if wind_variation > 0 {
        environment = environment.with_wind_variation(wind_variation, 2000);
    }

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

fn collect_legacy_objects(
    group: &Group,
    definitions: &[ScenarioDefinition],
) -> Result<Vec<ScenarioSpawn>, ScenarioError> {
    let bytes = match group.read_file("Objects.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(Vec::new()),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Vec::new())
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
                if records[child_index].contained.is_none() {
                    records[child_index].contained = Some(parent_number);
                }
            }
        }
    }

    let definition_ids: HashSet<&str> = definitions
        .iter()
        .map(|definition| definition.id.as_str())
        .collect();
    let object_numbers: HashSet<u64> = records
        .iter()
        .filter(|record| !matches!(record.status, Some(ObjectStatus::Deleted)))
        .filter(|record| {
            record
                .id
                .as_deref()
                .is_some_and(|id| definition_ids.contains(id))
        })
        .filter_map(|record| record.number)
        .collect();
    let strings = load_legacy_string_table(group)?;
    let value_resolution = SerializedC4ValueResolution {
        object_numbers: &object_numbers,
        strings: &strings,
    };

    let mut spawns = Vec::new();
    for record in records.into_iter() {
        if let Some(spawn) = record.into_spawn(&definition_ids, &value_resolution)? {
            spawns.push(spawn);
        }
    }
    Ok(spawns)
}

/// C4StringTable::Load assigns each Strings.txt line its zero-based enum ID.
/// Repeated text reuses the existing C4String and updates that one instance to
/// the later ID, so the earlier ID is no longer resolvable
/// (C4StringTable.cpp:201-216).
fn load_legacy_string_table(group: &Group) -> Result<HashMap<i32, String>, ScenarioError> {
    let bytes = match group.read_file("Strings.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(HashMap::new()),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(ScenarioError::Resources(error)),
    };

    let mut id_by_bytes = HashMap::<Vec<u8>, i32>::new();
    let mut value_by_id = HashMap::new();
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
        let raw_value = line[..end].to_vec();
        let value = decode_legacy_script_text(&raw_value);
        if let Some(previous_id) = id_by_bytes.insert(raw_value, index) {
            value_by_id.remove(&previous_id);
        }
        value_by_id.insert(index, value);
    }
    Ok(value_by_id)
}

#[derive(Debug, Default)]
struct LegacyObjectRecord {
    line: usize,
    id: Option<String>,
    number: Option<u64>,
    /// C4Object::CustomName (`Name=`, C4Object.cpp:2749-2760).
    custom_name: Option<String>,
    status: Option<ObjectStatus>,
    owner: Option<i32>,
    /// C4Object::Controller, compiled verbatim with default NO_OWNER
    /// (C4Object.cpp:2739).
    controller: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
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
    energy: Option<i32>,
    /// C4Object::NeedEnergy (`NeedEnergy=`, C4Object.cpp:2805).
    need_energy: Option<bool>,
    /// C4Object::Select (`Selected=`, C4Object.cpp:2800).
    selected: Option<bool>,
    /// C4Object::MagicEnergy (`MagicEnergy=`, C4Object.cpp:2768).
    magic_energy: Option<i32>,
    construction: Option<i32>,
    alive: Option<bool>,
    in_liquid: Option<bool>,
    category: Option<i32>,
    direction: Option<Direction>,
    command_direction: Option<CommandDirection>,
    action_name: Option<String>,
    action_phase: Option<i32>,
    /// Action.Time (`ActionTime=`, C4Object.cpp:2745 area).
    action_ticks: Option<u32>,
    /// Action.PhaseDelay (`PhaseDelay=`), the intra-phase counter.
    action_phase_delay: Option<u32>,
    action_data: Option<i32>,
    action_target: Option<u64>,
    action_target2: Option<u64>,
    /// C4Object::pLayer (`Layer=`, C4Object.cpp:2819).
    layer: Option<u64>,
    /// C4Object::BlitMode (`BlitMode=`, C4Object.cpp:2817).
    blit_mode: Option<u32>,
    contained: Option<u64>,
    contents: Vec<u64>,
}

impl LegacyObjectRecord {
    fn new(line: usize) -> Self {
        Self {
            line,
            ..Self::default()
        }
    }

    fn apply_property(&mut self, key: &str, value: &str) -> Result<(), ScenarioError> {
        let normalized_key = key.trim().to_ascii_lowercase();
        let trimmed_value = value.trim();
        match normalized_key.as_str() {
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
            "localnamed" => {
                self.local_named = Some(parse_local_named(trimmed_value, self.line)?);
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
            "energy" => {
                self.energy = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Energy `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
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
            "con" | "size" => {
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
                let clamped = raw.clamp(0, FULL_CON);
                self.construction = Some(clamped);
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
                if ticks < 0 {
                    return Err(ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: ActionTime must be >= 0 (got {})",
                        self.line, ticks
                    )));
                }
                self.action_ticks = Some(ticks as u32);
            }
            "phasedelay" => {
                let value = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid PhaseDelay `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.action_phase_delay = Some(value.max(0) as u32);
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
                self.action_target = Some(parse_u64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTarget1 `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "actiontarget2" => {
                self.action_target2 = Some(parse_u64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ActionTarget2 `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "layer" => {
                let value = parse_i64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Layer `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                if value > 0 {
                    self.layer = Some(value as u64);
                }
            }
            "blitmode" => {
                let value = parse_u64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid BlitMode `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                self.blit_mode = Some(value as u32);
            }
            "contained" => {
                let value = parse_i64(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Contained `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                if value > 0 {
                    self.contained = Some(value as u64);
                }
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
            status,
            owner,
            controller,
            x,
            y,
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
            local_named,
            vertex_count,
            vertex_x,
            vertex_y,
            vertex_cnat,
            vertex_friction,
            energy,
            need_energy,
            selected,
            magic_energy,
            construction,
            alive,
            in_liquid,
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
            blit_mode,
            contained,
            contents: _,
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
            .with_loaded(true);
        config = config.with_position(Vector2::new(x.unwrap_or(0), y.unwrap_or(0)));
        if let Some(custom_name) = custom_name {
            config = config.with_custom_name(custom_name);
        }
        if let Some(layer) = layer {
            config = config.with_layer(ObjectId::new(layer));
        }
        if let Some(blit_mode) = blit_mode {
            config = config.with_blit_mode(blit_mode);
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
        if fix_x.is_some() || fix_y.is_some() {
            // Exact sub-pixel position (FixX/FixY, C4Object.cpp:2762-2763).
            // C++ keeps the integer X/Y and the fixed coords INDEPENDENT
            // after load (no reconciliation); a missing key means Fix0 —
            // engine-saved files always carry both for nonzero positions.
            config = config.with_fixed_position(crate::math::FixedVec2 {
                x: fix_x.unwrap_or_default(),
                y: fix_y.unwrap_or_default(),
            });
        }
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
        if let Some(local_named) = local_named {
            // Loaded objects keep their script locals verbatim (the tree
            // MotionThreshold, bandit AI state). C++ resolves string IDs while
            // compiling and denumerates object refs after every object exists.
            config = config.with_local_vars(
                local_named
                    .into_iter()
                    .map(|(name, value)| (name, value.resolve(value_resolution)))
                    .collect(),
            );
        }
        // The saved shape's vertices (C4Shape::CompileFunc into [Object],
        // C4Shape.cpp:495-515): the CURRENT effective shape, loaded
        // verbatim (spawn_single skips the Con/rotation re-transform for
        // loaded vertices). Missing arrays read as 0 (mkArrayAdapt).
        if let Some(count) = vertex_count {
            let count = count.clamp(0, 30) as usize;
            if count > 0 {
                let component = |list: &Option<Vec<i32>>, index: usize| {
                    list.as_ref()
                        .and_then(|values| values.get(index).copied())
                        .unwrap_or(0)
                };
                let vertices: Vec<crate::ObjectVertex> = (0..count)
                    .map(|index| {
                        crate::ObjectVertex::new(
                            component(&vertex_x, index),
                            component(&vertex_y, index),
                        )
                        .with_cnat(component(&vertex_cnat, index) as u32)
                        .with_friction(component(&vertex_friction, index))
                    })
                    .collect();
                config = config.with_vertices(vertices);
            }
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
        if let Some(construction) = construction {
            config = config.with_construction(construction);
        }
        if let Some(alive) = alive {
            config = config.with_alive(alive);
        }
        if let Some(in_liquid) = in_liquid {
            config = config.with_in_liquid(in_liquid);
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

        Ok(Some(ScenarioSpawn {
            handle: Some(number.to_string()),
            container_handle: contained.map(|value| value.to_string()),
            config,
        }))
    }
}

fn build_action_state(
    name: Option<String>,
    phase: Option<i32>,
    time: Option<u32>,
    phase_delay: Option<u32>,
    data: Option<i32>,
    target: Option<u64>,
    target2: Option<u64>,
) -> Option<ActionState> {
    let name = name?;
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
    if let Some(target) = target {
        state.target = Some(ObjectId::new(target));
    }
    if let Some(target2) = target2 {
        state.target2 = Some(ObjectId::new(target2));
    }
    Some(state)
}

fn parse_legacy_objects(text: &str) -> Result<Vec<LegacyObjectRecord>, ScenarioError> {
    let mut records = Vec::new();
    let mut current: Option<LegacyObjectRecord> = None;

    for (index, raw_line) in text.lines().enumerate() {
        let mut line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(stripped) = line.split_once("//") {
            line = stripped.0.trim_end();
        }
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section_name = &line[1..line.len() - 1];
            // Only [Object] sections create new object records
            // Skip subsections like [Physical], [Commands], etc.
            if section_name.eq_ignore_ascii_case("Object") {
                if let Some(record) = current.take() {
                    records.push(record);
                }
                current = Some(LegacyObjectRecord::new(index + 1));
            }
            // Skip subsections - they're optional property overrides we don't use
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let record = current.as_mut().ok_or_else(|| {
            ScenarioError::LegacyObjectsParse(format!(
                "Objects.txt line {}: encountered property `{}` outside of an [Object] section",
                index + 1,
                key.trim()
            ))
        })?;
        record.apply_property(key, value)?;
    }

    if let Some(record) = current.take() {
        records.push(record);
    }

    Ok(records)
}

/// C4Object::CustomName uses StdCompiler's escaped-string adapter. Modern
/// saves quote the value; older shipped saves keep the whole unquoted line
/// (StdCompiler.cpp:734-741, 936-976, 1006-1062).
fn parse_legacy_object_name(
    value: &str,
    line: usize,
) -> Result<Option<String>, ScenarioError> {
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
                        )))
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

/// A serialized C4Fixed (Fixed.h:247-266): an `f` prefix means the int32
/// is FLOAT BITS converted via ftofix (FLOAT_TO_FIXED); `F` or no prefix
/// means the raw fixed-point value. GoldRush's hanging stalactites carry
/// `YDir=f1067030938` = 1.2 px/frame — misread as a raw int it becomes a
/// shattering hit speed.
/// Objects.txt `LocalNamed=` (C4ValueMapData::CompileFunc,
/// C4ValueMap.cpp:236-295): `<count>;name=<value>,name=<value>,...` where
/// each value uses the C4Value type-char encoding (GetC4VID,
/// C4Value.cpp:368-394). A zero count writes no separator and no entries.
#[derive(Debug)]
enum SerializedC4Value {
    Value(lc_script::Value),
    ObjectNumber(i32),
    StringTableIndex(i32),
    Array(Vec<SerializedC4Value>),
    Unsupported(String),
}

struct SerializedC4ValueResolution<'a> {
    object_numbers: &'a HashSet<u64>,
    strings: &'a HashMap<i32, String>,
}

impl SerializedC4Value {
    /// Mirror C4Value::DenumeratePointer and serialized-string lookup
    /// (C4Value.cpp:686-713,783-798). Serialized identities become live VM
    /// values only after the accepted object-number and string tables exist.
    fn resolve(self, resolution: &SerializedC4ValueResolution<'_>) -> lc_script::Value {
        use lc_script::Value;
        match self {
            Self::Value(value) => value,
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
                    .filter(|number| resolution.object_numbers.contains(number))
                    .map(Value::Object)
                    .unwrap_or(Value::Nil)
            }
            Self::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| value.resolve(resolution))
                    .collect(),
            ),
            Self::StringTableIndex(index) => resolution
                .strings
                .get(&index)
                .cloned()
                .map(Value::String)
                .unwrap_or(Value::Nil),
            Self::Unsupported(encoded) => {
                tracing::warn!(
                    value = encoded,
                    "LocalNamed C4Value type not modeled yet; reading as nil"
                );
                Value::Nil
            }
        }
    }
}

fn parse_local_named(
    value: &str,
    line: usize,
) -> Result<Vec<(String, SerializedC4Value)>, ScenarioError> {
    let trimmed = value.trim();
    let Some((_count, rest)) = trimmed.split_once(';') else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for part in split_outside_brackets(rest) {
        let part = part.trim();
        if part.is_empty() {
            continue;
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
    Ok(entries)
}

/// Split on commas outside `[...]` (array payloads carry their own commas).
fn split_outside_brackets(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// One serialized C4Value (C4Value::CompileFunc, C4Value.cpp:717-800 +
/// GetC4VID :368-394): `A`=any (zero data reads back nil, nonzero guesses
/// int), `i`=int, `b`=bool, `O`=enumerated object number (0 = no object),
/// `I`=C4ID stored as its signed 32-bit payload, `a[size;elems]`=array with
/// trailing nils omitted on write, and `S` indexes the scenario Strings.txt.
/// `m` (a generic C4Value-keyed map) is not modeled yet and reads as nil with
/// a warning.
fn parse_serialized_c4value(
    encoded: &str,
    line: usize,
) -> Result<SerializedC4Value, ScenarioError> {
    use lc_script::Value;
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
        'A' => Ok(SerializedC4Value::Value(match int_payload()? {
            0 => Value::Nil,
            other => Value::Int(other),
        })),
        'i' => Ok(SerializedC4Value::Value(Value::Int(int_payload()?))),
        'b' => Ok(SerializedC4Value::Value(Value::Bool(
            int_payload()? != 0,
        ))),
        'O' => Ok(SerializedC4Value::ObjectNumber(int_payload()?)),
        'I' => Ok(SerializedC4Value::Value(match int_payload()? {
            0 => Value::Nil,
            raw @ 1..=9999 => Value::C4Id(format!("{raw:04}")),
            raw => Value::C4Id(
                (raw as u32)
                    .to_le_bytes()
                    .into_iter()
                    .take_while(|byte| *byte != 0)
                    .map(char::from)
                    .collect(),
            ),
        })),
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
            let size = parse_i32(size_text.trim())
                .map_err(|err| {
                    parse_error(format!("invalid array size in `{}` ({})", encoded, err))
                })?
                .clamp(0, 100_000) as usize;
            let mut elements: Vec<SerializedC4Value> = split_outside_brackets(elements_text)
                .into_iter()
                .map(str::trim)
                .filter(|element| !element.is_empty())
                .map(|element| parse_serialized_c4value(element, line))
                .collect::<Result<_, _>>()?;
            // Trailing nils are omitted on write; restore the full size.
            if elements.len() < size {
                elements.resize_with(size, || SerializedC4Value::Value(Value::Nil));
            }
            Ok(SerializedC4Value::Array(elements))
        }
        'S' => Ok(SerializedC4Value::StringTableIndex(int_payload()?)),
        'm' => Ok(SerializedC4Value::Unsupported(encoded.to_string())),
        other => Err(parse_error(format!(
            "unknown C4Value type char `{}` in `{}`",
            other, encoded
        ))),
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

fn parse_c4fixed(value: &str) -> Result<crate::math::C4Fixed, String> {
    let trimmed = value.trim();
    let (float_bits, rest) = match trimmed.as_bytes().first() {
        Some(b'f') => (true, &trimmed[1..]),
        Some(b'F') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let raw = parse_i32(rest)?;
    if float_bits {
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

fn collect_definitions_from_group<S: AsRef<str>>(
    group: &Group,
    load_system_groups: bool,
    skip_ids: &HashSet<String>,
    languages: &[S],
    output: &mut Vec<CollectedDefinition>,
) -> Result<(), ScenarioError> {
    let mut primary_definition = false;
    if group.exists("DefCore.txt") {
        let resource = ResourceDefinitionData::load(group)?;
        if !skip_ids.contains(&resource.core.id.to_ascii_uppercase()) {
            primary_definition = true;
            let mut definition = scenario_definition_from_resource(resource, Some(group.clone()));
            definition.script = localize_script_source(group, &definition.script, languages)?;
            output.push(CollectedDefinition::Definition(definition));
        }
    }

    // C4DefList::Load recursively visits only *.c4d children.
    for entry in group.entries()? {
        if !entry.is_directory {
            continue;
        }
        let name = entry.relative_path.to_string_lossy().to_ascii_lowercase();
        if !name.ends_with(".c4d") {
            continue;
        }
        let child = group.open_child(&entry.relative_path)?;
        // The recursive call omits fLoadSysGroups in C++, so its default true
        // applies even when only the scenario root suppressed System loading.
        collect_definitions_from_group(&child, true, skip_ids, languages, output)?;
    }

    // A non-primary definition root loads its System.c4g only AFTER all child
    // definitions (C4Def.cpp:927-968). Direct primary definitions suppress
    // their own System group, as does the scenario-file InitDefs pass.
    if !primary_definition && load_system_groups {
        if let Ok(system) = group.open_child(Path::new("System.c4g")) {
            if let Ok(sources) = load_system_scripts(&system) {
                output.push(CollectedDefinition::SystemScripts(sources));
            }
        }
    }
    Ok(())
}

fn scenario_definition_from_resource(
    resource: ResourceDefinitionData,
    source_group: Option<Group>,
) -> ScenarioDefinition {
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
        rank_symbols_image,
    } = resource;
    let actions = action_map.map(|map| convert_action_map(&map));
    let full_core = core.clone();

    ScenarioDefinition {
        id: core.id,
        name: core.name,
        script: script.combined().to_string(),
        actions,
        crew_member: core.crew_member,
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
        rank_symbols_image,
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
    let mut graphics = HashMap::new();
    for (name, definition) in &map.actions {
        let (spec, visuals) = convert_action_definition(definition);
        // Duplicate action names: the FIRST entry wins, matching the forward
        // scan in C++ SetActionByName.
        specs.entry(name.clone()).or_insert(spec);
        graphics.entry(name.clone()).or_insert(visuals);
    }
    DefinitionActions {
        default_action: map.default_action.clone(),
        specs,
        graphics,
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
    if let Some(procedure) = &action.procedure {
        spec = spec.with_procedure(procedure.clone());
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
    graphics.directions = action.directions.unwrap_or(1).max(1);
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
    ticks: Option<u32>,
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
                    return Err(ScenarioError::SkySurfaceMissing { path })
                }
                Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ScenarioError::SkySurfaceMissing { path })
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
mod tests {
    use super::*;
    use image::{codecs::bmp::BmpEncoder, ColorType, Rgba, RgbaImage};
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    const TEST_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return nil;
}

global func Step(state, frame, random)
{
    return nil;
}
"#;

    #[test]
    fn legacy_team_sections_keep_cpp_list_order_and_fields() {
        // C4TeamList::CompileFunc reads the repeated Team sections into its
        // array in file order (C4Teams.cpp:556-613).
        let teams = parse_legacy_teams_source(
            "[Teams]\nActive=1\n\n[Team]\nid=2\nName=Right\nColor=16053492\n\n\
             [Team]\nid=1\nName=Left\nColor=14699548\n",
        );
        assert_eq!(
            teams,
            vec![
                TeamInfo::new(2, "Right", 16_053_492),
                TeamInfo::new(1, "Left", 14_699_548),
            ]
        );
    }

    #[test]
    fn legacy_c4sval_evaluate_draws_via_the_game_rng_like_cpp() {
        // C4SVal::Evaluate (C4Scenario.cpp:43-46):
        //   return BoundBy(Std + Random(2 * Rnd + 1) - Rnd, Min, Max);
        // One Random draw per Evaluate — even Rnd == 0 calls Random(1),
        // which advances the LCG stream (C4Random.h:40-61).
        let mut reference = crate::rng::LcgRng::new(42);
        let expected = (10 + reference.random(2 * 3 + 1) - 3).clamp(0, 250);

        let mut rng = crate::rng::LcgRng::new(42);
        assert_eq!(
            LegacyC4SVal::new(10, 3, 0, 250).evaluate(&mut rng),
            expected
        );
        assert_eq!(rng, reference);

        // Rnd == 0 still draws: Random(1) returns 0 but advances hold/count.
        let before = rng.clone();
        assert_eq!(LegacyC4SVal::new(5, 0, 0, 250).evaluate(&mut rng), 5);
        assert_ne!(rng.hold, before.hold);
        assert_eq!(rng.count, before.count + 1);
    }

    #[test]
    fn legacy_scenario_core_parses_all_fields() {
        let legacy = r#"
[Head]
Title=Legacy Land
Icon=7
Loader=LoaderGfx
Font=CustomFont
Version=4,9,10,15,359
Difficulty=3
MaxPlayer=6
MaxPlayerLeague=4
MinPlayer=2
SaveGame=1
Replay=0
Film=2
DisableMouse=1
NoInitialize=1
RandomSeed=12345
ForcedAutoContextMenu=0
ForcedAutoStopControl=1
Engine=Legacy
MissionAccess=MISS
NetworkGame=1
NetworkRuntimeJoin=0
ForcedGfxMode=2
ForcedNoCrew=1
DefCrewStrength=42
Origin=Planet\Legacy.c4s

[Definitions]
LocalOnly=0
AllowUserChange=1
Definitions=Defs.c4d;More.c4d
Definition3=Extra.c4d
SkipDefs=CLNK=2;ROCK

[Game]
Mode=2
Elimination=3
CooperativeGoal=1
CreateObjects=FIRE=3;WOOD=1
ClearObjects=ROCK=2
ClearMaterials=Earth=5;Gold
ValueGain=150
EnableRemoveFlag=1
StructNeedMaterial=1
StructNeedEnergy=0
ValueOverloads=VALU=2
LandscapePushPull=2
LandscapeInsertThrust=3
BaseFunctionality=BASEFUNC_Buy|BASEFUNC_Sell
BaseRegenerateEnergyPrice=12
Goals=GOAL=1
Rules=RULE=1
FoWColor=0x12345678

[Player1]
StandardCrew=CLNK
Clonks=2,1,1,5
Wealth=50,0,0,500
Position=100,200
EnforcePosition=1
Crew=CLNK=2;OCEN
Buildings=HUTS=1
Vehicles=CARR=1
Material=ROCK=3
Knowledge=KNOW
HomeBaseMaterial=WOOD=5
HomeBaseProduction=METL=2
Magic=MAGI=1

[Landscape]
ExactLandscape=1
Vegetation=GRAS;TREE
VegetationLevel=60,20,0,100
InEarth=ROCK;COAL
InEarthLevel=40,0,0,100
Sky=Sky.ocg
SkyFade=1,2,3,4,5,6
NoSky=0
BottomOpen=1
TopOpen=0
LeftOpen=1
RightOpen=2
AutoScanSideOpen=0
MapWidth=120,0,64,250
MapHeight=80,0,40,250
MapZoom=5,0,5,15
Amplitude=10,0,0,100
Phase=25,0,0,100
Period=30,0,0,100
Random=15,0,0,100
Material=Sand
Liquid=Lava
LiquidLevel=5,0,0,100
MapPlayerExtend=1
Layers=Earth=2;Sky=1
Gravity=90,0,10,200
NoScan=1
KeepMapCreator=1
SkyScrollMode=2
NewStyleLandscape=1
FoWRes=128
ShadeMaterials=0

[Weather]
Climate=40,10,0,100
StartSeason=10,20,0,100
YearSpeed=5,0,0,100
Rain=30,0,0,100
Wind=5,10,-50,50
Lightning=20,0,0,100
Precipitation=Oil
NoGamma=0

[Disasters]
Meteorite=10,0,0,100
Volcano=5,0,0,100
Earthquake=3,0,0,100

[Animals]
Animal=WLF_=2
Nest=ANT_=3

[Environment]
Objects=STNE=1;TREE=1
"#;

        let manifest = parse_legacy_scenario_text(legacy).expect("legacy scenario manifest parses");
        let core = &manifest.core;

        assert_eq!(core.head.title, "Legacy Land");
        assert_eq!(core.head.icon, 7);
        assert_eq!(core.head.loader, "LoaderGfx");
        assert_eq!(core.head.font, "CustomFont");
        assert_eq!(core.head.version, [4, 9, 10, 15, 359]);
        assert_eq!(core.head.difficulty, 3);
        assert_eq!(core.head.max_player, 6);
        assert_eq!(core.head.max_player_league, 4);
        assert_eq!(core.head.min_player, 2);
        assert!(core.head.save_game);
        assert!(core.head.disable_mouse);
        assert!(core.head.no_initialize);
        assert_eq!(core.head.random_seed, 12345);
        assert_eq!(core.head.forced_auto_context_menu, 0);
        assert_eq!(core.head.forced_control_style, 1);
        assert_eq!(core.head.engine, "Legacy");
        assert_eq!(core.head.mission_access, "MISS");
        assert!(core.head.network_game);
        assert!(!core.head.network_runtime_join);
        assert_eq!(core.head.forced_gfx_mode, 2);
        assert_eq!(core.head.forced_fair_crew, 1);
        assert_eq!(core.head.fair_crew_strength, 42);
        assert_eq!(core.head.origin.as_deref(), Some("Planet\\Legacy.c4s"));

        assert!(!core.definitions.local_only);
        assert!(core.definitions.allow_user_change);
        assert_eq!(
            core.definitions.definitions,
            vec![
                "Defs.c4d".to_string(),
                "More.c4d".to_string(),
                "Extra.c4d".to_string()
            ]
        );
        assert_eq!(core.definitions.skip_defs.len(), 2);
        assert_eq!(core.definitions.skip_defs[0].id, "CLNK");
        assert_eq!(core.definitions.skip_defs[0].count, Some(2));
        assert_eq!(core.definitions.skip_defs[1].id, "ROCK");
        assert_eq!(core.definitions.skip_defs[1].count, None);

        assert_eq!(core.game.mode, 2);
        assert_eq!(core.game.elimination, 3);
        assert_eq!(core.game.cooperative_goal, 1);
        assert_eq!(core.game.create_objects.len(), 2);
        assert_eq!(core.game.clear_objects.len(), 1);
        assert_eq!(core.game.clear_materials.len(), 2);
        assert_eq!(core.game.value_gain, 150);
        assert!(core.game.enable_remove_flag);
        assert!(core.game.realism.construction_needs_material);
        assert!(!core.game.realism.structures_need_energy);
        assert_eq!(core.game.realism.landscape_push_pull, 2);
        assert_eq!(core.game.realism.landscape_insert_thrust, 3);
        assert_eq!(
            core.game.realism.base_functionality,
            BASEFUNC_BUY | BASEFUNC_SELL
        );
        assert_eq!(core.game.realism.base_regenerate_energy_price, 12);
        assert_eq!(core.game.goals.len(), 1);
        assert_eq!(core.game.rules.len(), 1);
        assert_eq!(core.game.fow_color, 0x1234_5678);

        assert_eq!(core.players.len(), 1);
        let player = &core.players[0];
        assert_eq!(player.standard_crew.as_deref(), Some("CLNK"));
        assert_eq!(player.clonks.std, 2);
        assert_eq!(player.clonks.rnd, 1);
        assert_eq!(player.wealth.std, 50);
        assert_eq!(player.position, [100, 200]);
        assert!(player.enforce_position);
        assert_eq!(player.crew.len(), 2);
        assert_eq!(player.buildings.len(), 1);
        assert_eq!(player.vehicles.len(), 1);
        assert_eq!(player.material.len(), 1);
        assert_eq!(player.knowledge.len(), 1);
        assert_eq!(player.home_base_material.len(), 1);
        assert_eq!(player.home_base_production.len(), 1);
        assert_eq!(player.magic.len(), 1);

        let landscape = &core.landscape;
        assert!(landscape.exact_landscape);
        assert_eq!(landscape.vegetation.len(), 2);
        assert_eq!(landscape.in_earth.len(), 2);
        assert_eq!(landscape.sky.as_deref(), Some("Sky.ocg"));
        assert_eq!(landscape.sky_fade, [1, 2, 3, 4, 5, 6]);
        assert!(landscape.bottom_open);
        assert!(!landscape.top_open);
        assert_eq!(landscape.left_open, 1);
        assert_eq!(landscape.right_open, 2);
        assert!(!landscape.auto_scan_side_open);
        assert_eq!(landscape.map_width.std, 120);
        assert_eq!(landscape.map_height.std, 80);
        assert_eq!(landscape.map_zoom.std, 5);
        assert_eq!(landscape.material, "Sand");
        assert_eq!(landscape.liquid, "Lava");
        assert!(landscape.map_player_extend);
        assert_eq!(landscape.layers.len(), 2);
        assert!(landscape.no_scan);
        assert!(landscape.keep_map_creator);
        assert_eq!(landscape.sky_scroll_mode, 2);
        assert_eq!(landscape.new_style_landscape, 1);
        assert_eq!(landscape.fow_resolution, 128);
        assert!(!landscape.shade_materials);

        let weather = &core.weather;
        assert_eq!(weather.climate.std, 40);
        assert_eq!(weather.start_season.std, 10);
        assert_eq!(weather.year_speed.std, 5);
        assert_eq!(weather.rain.std, 30);
        assert_eq!(weather.wind.std, 5);
        assert_eq!(weather.lightning.std, 20);
        assert_eq!(weather.precipitation, "Oil");
        assert!(!weather.no_gamma);

        assert_eq!(core.disasters.meteorite.std, 10);
        assert_eq!(core.disasters.volcano.std, 5);
        assert_eq!(core.disasters.earthquake.std, 3);

        assert_eq!(core.animals.free_life.len(), 1);
        assert_eq!(core.animals.earth_nest.len(), 1);
        assert_eq!(core.environment.objects.len(), 2);
    }

    #[test]
    fn loads_flat_landscape_scenario() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "name": "Temp Scenario",
            "ticks": 240,
            "landscape": { "kind": "flat", "width": 128, "height": 42 },
            "definitions": [
                { "id": "Mover", "name": "Mover", "script": "scripts/mover.aul" }
            ],
            "initial_objects": [
                { "definition": "Mover", "position": [10, 20], "velocity": [1, -1], "energy": 99 }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        assert_eq!(scenario.name(), Some("Temp Scenario"));
        assert_eq!(scenario.configured_ticks(), Some(240));
        assert_eq!(scenario.ground_height_hint(), Some(42));
        assert!(scenario.has_initial_objects());

        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(created.len(), 1);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.objects.len(), 1);
        let object = &snapshot.objects[0];
        assert_eq!(object.definition_id, "Mover");
        assert_eq!(object.position, Vector2::new(10, 20));
        assert_eq!(object.velocity, Vector2::new(1, -1));
        assert_eq!(object.energy, 99);

        let landscape = engine.landscape().expect("landscape set");
        assert_eq!(landscape.surface_height(0), Some(42));
        assert!(scenario.physics().is_none());
    }

    #[test]
    fn applies_action_configuration_from_manifest() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                {
                    "id": "Mover",
                    "script": "scripts/mover.aul",
                    "default_action": "Walk",
                    "actions": {
                        "Walk": { "length": 2, "delay": 1, "next": "Idle" },
                        "Idle": { "length": 1 }
                    }
                }
            ],
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let mut engine = Engine::with_seed(5);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        let id = created[0];

        let initial = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(initial.action.name, "Walk");
        assert_eq!(initial.action.phase, 0);

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.action.phase, 1);

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.action.phase, 0);
    }

    #[test]
    fn seeds_initial_action_and_effects_from_manifest() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                {
                    "id": "Mover",
                    "script": "scripts/mover.aul",
                    "default_action": "Idle",
                    "actions": {
                        "Idle": { "length": 1 },
                        "Walk": { "length": 5, "next": "Idle" }
                    }
                }
            ],
            "initial_objects": [
                {
                    "definition": "Mover",
                    "action": { "name": "Walk", "phase": 3 },
                    "effects": [
                        { "name": "Intoxicated", "priority": 150, "interval": 3, "timer": 5 }
                    ]
                }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(created.len(), 1);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.objects.len(), 1);
        let object = &snapshot.objects[0];
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.action.phase, 3);
        assert_eq!(object.effects.len(), 1);
        let effect = &object.effects[0];
        assert_eq!(effect.name, "Intoxicated");
        assert_eq!(effect.priority, 150);
        assert_eq!(effect.interval, 3);
        // iTime is stored verbatim - C++ never wraps it (C4Effect.cpp:66-67).
        assert_eq!(effect.timer, 5);
    }

    #[test]
    fn seeds_initial_status_from_manifest() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                {
                    "id": "Mover",
                    "script": "scripts/mover.aul",
                    "default_action": "Idle",
                    "actions": { "Idle": { "length": 1 } }
                }
            ],
            "initial_objects": [
                {
                    "definition": "Mover",
                    "status": "inactive"
                }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        let id = created[0];

        let snapshot = engine.snapshot();
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.status, ObjectStatus::Inactive);
        let initial_phase = object.action.phase;

        let ticked = engine.tick().expect("tick succeeds");
        let object = ticked.object(id).expect("object present");
        assert_eq!(object.status, ObjectStatus::Inactive);
        assert_eq!(object.action.phase, initial_phase);
    }

    #[test]
    fn spawns_contents_with_container_handles() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Chest", "script": "scripts/chest.aul" },
                { "id": "Gem", "script": "scripts/gem.aul" }
            ],
            "initial_objects": [
                { "definition": "Chest", "handle": "store" },
                { "definition": "Gem", "container": "store" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/chest.aul"), TEST_SCRIPT).expect("write script");
        std::fs::write(dir.path().join("scripts/gem.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(created.len(), 2);

        let snapshot = engine.snapshot();
        let chest = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "Chest")
            .expect("chest present");
        let gem = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "Gem")
            .expect("gem present");
        assert_eq!(gem.container, Some(chest.id));
        assert!(chest.contents.contains(&gem.id));
    }

    #[test]
    fn errors_on_unknown_container_handle() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Chest", "script": "scripts/chest.aul" },
                { "id": "Gem", "script": "scripts/gem.aul" }
            ],
            "initial_objects": [
                { "definition": "Gem", "container": "missing" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/chest.aul"), TEST_SCRIPT).expect("write script");
        std::fs::write(dir.path().join("scripts/gem.aul"), TEST_SCRIPT).expect("write script");

        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        match error {
            ScenarioError::UnknownContainerHandle(handle) => assert_eq!(handle, "missing"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn container_cycles_degrade_to_partial_containment() {
        // C++ resolves Contained by two-phase denumeration, so mutual
        // containment loads without error. The sequential spawn model
        // breaks ONE edge (documented divergence) — both objects must
        // exist, with one containment intact.
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Crate", "script": "scripts/crate.aul" },
                { "id": "Barrel", "script": "scripts/barrel.aul" }
            ],
            "initial_objects": [
                { "definition": "Crate", "handle": "crate", "container": "barrel" },
                { "definition": "Barrel", "handle": "barrel", "container": "crate" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/crate.aul"), TEST_SCRIPT).expect("write script");
        std::fs::write(dir.path().join("scripts/barrel.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        let created = scenario
            .apply(&mut engine)
            .expect("apply degrades the cycle");
        assert_eq!(created.len(), 2, "both cycle members spawn");
        let contained_count = created
            .iter()
            .filter_map(|id| engine.object_snapshot(*id))
            .filter(|snapshot| snapshot.container.is_some())
            .count();
        assert_eq!(contained_count, 1, "one containment edge survives");
    }

    #[test]
    fn errors_on_unknown_definition_reference() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "initial_objects": [
                { "definition": "Missing" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        match error {
            ScenarioError::UnknownDefinition(name) => assert_eq!(name, "Missing"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn manifest_missing_returns_error() {
        let dir = tempdir().expect("tempdir");
        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        assert!(matches!(error, ScenarioError::ManifestMissing));
    }

    #[test]
    fn loads_physics_overrides() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "physics": {
                "gravity": 2,
                "max_fall_speed": 8,
                "max_rise_speed": -10,
                "max_horizontal_speed": 7
            }
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let physics = scenario.physics().expect("physics present");
        assert_eq!(physics.gravity, 2);
        assert_eq!(physics.max_fall_speed, 8);
        assert_eq!(physics.max_rise_speed, -10);
        assert_eq!(physics.max_horizontal_speed, 7);

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.physics();
        assert_eq!(configured.gravity, 2);
        assert_eq!(configured.max_fall_speed, 8);
        assert_eq!(configured.max_rise_speed, -10);
        assert_eq!(configured.max_horizontal_speed, 7);
    }

    #[test]
    fn loads_environment_settings_and_applies_to_engine() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": -3
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.wind, -3);
        assert_eq!(environment.wind_variation, 0);
        assert_eq!(environment.wind_period, 0);
        assert_eq!(environment.temperature, 0);
        assert_eq!(environment.precipitation, 0);
        assert!(environment.sky_color.is_none());

        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(created.len(), 1);

        let configured = engine.environment();
        assert_eq!(configured.wind, -3);
        assert_eq!(configured.wind_variation, 0);
        assert_eq!(configured.wind_period, 0);
        assert_eq!(configured.temperature, 0);
        assert_eq!(configured.precipitation, 0);
        assert!(configured.sky_color.is_none());
    }

    #[test]
    fn loads_environment_variation_and_temperature() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 4,
                "wind_variation": -6,
                "wind_period": 180,
                "temperature": -15
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.wind, 4);
        assert_eq!(environment.wind_variation, 6);
        assert_eq!(environment.wind_period, 180);
        assert_eq!(environment.temperature, -15);
        assert_eq!(environment.precipitation, 0);

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.environment();
        assert_eq!(configured.wind, 4);
        assert_eq!(configured.wind_variation, 6);
        assert_eq!(configured.wind_period, 180);
        assert_eq!(configured.temperature, -15);
        assert_eq!(configured.time_of_day, 0);
        assert_eq!(configured.time_speed, 0);
        assert_eq!(configured.precipitation, 0);
    }

    #[test]
    fn loads_environment_climate_and_temperature_cycle() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 1,
                "climate": 8,
                "temperature": -4,
                "temperature_variation": 6,
                "temperature_period": 120,
                "temperature_phase": 30
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.climate, 8);
        assert_eq!(environment.temperature, -4);
        assert_eq!(environment.temperature_variation, 6);
        assert_eq!(environment.temperature_period, 120);
        assert_eq!(environment.temperature_phase, 30);
        assert_eq!(environment.precipitation, 0);

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.environment();
        assert_eq!(configured.climate, 8);
        assert_eq!(configured.temperature, -4);
        assert_eq!(configured.temperature_variation, 6);
        assert_eq!(configured.temperature_period, 120);
        assert_eq!(configured.temperature_phase, 30);
        assert_eq!(configured.precipitation, 0);
    }

    #[test]
    fn loads_environment_time_settings() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 1,
                "time_of_day": -45,
                "time_speed": 400
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.wind, 1);
        assert_eq!(environment.time_of_day, 2355);
        assert_eq!(environment.time_speed, 120);
        assert_eq!(environment.precipitation, 0);

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.environment();
        assert_eq!(configured.wind, 1);
        assert_eq!(configured.time_of_day, 2355);
        assert_eq!(configured.time_speed, 120);
        assert_eq!(configured.precipitation, 0);
    }

    #[test]
    fn loads_environment_precipitation_with_clamping() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 2,
                "precipitation": 140
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.wind, 2);
        assert_eq!(environment.precipitation, 100);

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.environment();
        assert_eq!(configured.wind, 2);
        assert_eq!(configured.precipitation, 100);
    }

    #[test]
    fn loads_environment_sky_color_from_array() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 1,
                "sky_color": [18, 42, 200]
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.sky_color, Some(RgbColor::new(18, 42, 200)));

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.environment();
        assert_eq!(configured.sky_color, Some(RgbColor::new(18, 42, 200)));
    }

    #[test]
    fn loads_environment_sky_color_from_hex() {
        let dir = tempdir().expect("tempdir");
        let manifest = r##"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 0,
                "sky_color": "#7F9AC3"
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "##;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.sky_color, Some(RgbColor::new(0x7F, 0x9A, 0xC3)));

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.environment();
        assert_eq!(configured.sky_color, Some(RgbColor::new(0x7F, 0x9A, 0xC3)));
    }

    #[test]
    fn scenario_without_environment_resets_engine_to_default() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        assert!(scenario.environment().is_none());

        let mut engine = Engine::with_seed(0);
        engine.set_environment(EnvironmentSettings::new(5));
        scenario.apply(&mut engine).expect("scenario applies");

        let configured = engine.environment();
        assert_eq!(configured, EnvironmentSettings::default());
    }

    #[test]
    fn scenario_tracks_crew_member_flags() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Crew", "script": "scripts/crew.aul", "crew_member": true }
            ],
            "initial_objects": [
                { "definition": "Crew", "owner": 1 },
                { "definition": "Crew", "owner": 2, "crew_member": false }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/crew.aul"), TEST_SCRIPT).expect("write script");

        let scenario = Scenario::load_from_path(dir.path()).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");

        assert_eq!(created.len(), 2);
        let first = engine
            .object_snapshot(created[0])
            .expect("first object snapshot");
        assert!(first.crew_member);
        assert_eq!(first.owner, 1);

        let second = engine
            .object_snapshot(created[1])
            .expect("second object snapshot");
        assert!(!second.crew_member);
        assert_eq!(second.owner, 2);
    }

    #[test]
    fn scenario_script_initialize_spawns_objects() {
        let scenario_script = r#"
global func Initialize(state, random)
{
    return { spawn = [ { definition = "Mover", owner = 42, energy = 77 } ] };
}

global func Step(state, frame, random)
{
    return nil;
}
"#;

        let scenario = Scenario {
            name: Some("Script Test".into()),
            description: None,
            ticks: None,
            ground_height_hint: Some(220),
            definitions: vec![ScenarioDefinition {
                id: "Mover".into(),
                name: Some("Mover".into()),
                script: TEST_SCRIPT.to_string(),
                actions: None,
                crew_member: false,
                can_be_base: false,
                movement: MovementProfile::default(),
                category: crate::DEFAULT_CATEGORY,
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
                rank_symbols_image: None,
                resource_group: None,
                components: Vec::new(),
                line_connect: 0,
                vertices: Vec::new(),
                shape: None,
                core: None,
            }],
            initial_spawns: vec![ScenarioSpawn {
                handle: None,
                container_handle: None,
                config: SpawnConfig::new("Mover"),
            }],
            landscape: None,
            physics: None,
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            environment: None,
            weather_init: None,
            sky: None,
            script: Some(ScenarioScriptSource {
                name: "Script.c".into(),
                source: scenario_script.to_string(),
                c4_args: false,
            }),
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            landscape_insert_thrust: false,
            forced_control_style: None,
            definition_load_steps: vec![DefinitionLoadStep::Definition("Mover".into())],
            scenario_system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
            teams: Vec::new(),
            standard_names: None,
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            init_placement: None,
        };

        let mut engine = Engine::with_seed(11);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(created.len(), 2);

        let mut energies: Vec<i32> = created
            .iter()
            .map(|id| engine.object_snapshot(*id).expect("object snapshot").energy)
            .collect();
        energies.sort_unstable();
        assert_eq!(energies, vec![0, 77]);

        let owners: Vec<i32> = created
            .iter()
            .map(|id| engine.object_snapshot(*id).expect("object snapshot").owner)
            .collect();
        assert!(owners.contains(&42));
    }

    #[test]
    fn scenario_script_step_runs_each_tick() {
        let scenario_script = r#"
global func Initialize(state, random)
{
    return nil;
}

global func Step(state, frame, random)
{
    if (frame == 1)
    {
        return { spawn = [ { definition = "Mover", owner = 99 } ] };
    }
    return nil;
}
"#;

        let scenario = Scenario {
            name: Some("Step Test".into()),
            description: None,
            ticks: None,
            ground_height_hint: Some(220),
            definitions: vec![ScenarioDefinition {
                id: "Mover".into(),
                name: Some("Mover".into()),
                script: TEST_SCRIPT.to_string(),
                actions: None,
                crew_member: false,
                can_be_base: false,
                movement: MovementProfile::default(),
                category: crate::DEFAULT_CATEGORY,
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
                rank_symbols_image: None,
                resource_group: None,
                components: Vec::new(),
                line_connect: 0,
                vertices: Vec::new(),
                shape: None,
                core: None,
            }],
            initial_spawns: vec![ScenarioSpawn {
                handle: None,
                container_handle: None,
                config: SpawnConfig::new("Mover").with_owner(1),
            }],
            landscape: None,
            physics: None,
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            environment: None,
            weather_init: None,
            sky: None,
            script: Some(ScenarioScriptSource {
                name: "Script.c".into(),
                source: scenario_script.to_string(),
                c4_args: false,
            }),
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            landscape_insert_thrust: false,
            forced_control_style: None,
            definition_load_steps: vec![DefinitionLoadStep::Definition("Mover".into())],
            scenario_system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
            teams: Vec::new(),
            standard_names: None,
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            init_placement: None,
        };

        let mut engine = Engine::with_seed(7);
        scenario.apply(&mut engine).expect("scenario applies");

        let initial_snapshot = engine.snapshot();
        assert_eq!(initial_snapshot.objects.len(), 1);

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.objects.len(), 2);
        assert!(snapshot.objects.iter().any(|object| object.owner == 99));
    }

    struct FileSystemResolver {
        roots: Vec<PathBuf>,
    }

    impl LegacyDefinitionResolver for FileSystemResolver {
        fn resolve_definition_groups(
            &self,
            scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            let mut groups = Vec::new();
            let normalized = identifier.replace('\\', "/");
            let path = Path::new(&normalized);

            if let Ok(child) = scenario.open_child(path) {
                groups.push(child);
            }

            for root in &self.roots {
                let candidate = root.join(path);
                if !candidate.exists() {
                    continue;
                }
                let group = Group::open(&candidate)?;
                if groups
                    .iter()
                    .all(|existing| existing.root() != group.root())
                {
                    groups.push(group);
                }
            }

            if groups.is_empty() {
                Err(ScenarioError::LegacyDefinitionNotFound {
                    path: identifier.to_string(),
                })
            } else {
                Ok(groups)
            }
        }
    }

    /// Builds a minimal legacy scenario dir with one good definition and an
    /// optional extra definition + scenario script, for resilience tests.
    fn write_resilience_fixture(
        dir: &std::path::Path,
        extra_def: Option<(&str, &str)>,
        scenario_script: &str,
    ) -> std::path::PathBuf {
        let defs_root = dir.join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");

        if let Some((id, script)) = extra_def {
            let extra = defs_root.join(format!("{id}.c4d"));
            std::fs::create_dir_all(&extra).expect("extra definition dir");
            std::fs::write(
                extra.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\nCrewMember=0\n"),
            )
            .expect("write extra defcore");
            std::fs::write(extra.join("Script.c"), script).expect("write extra script");
        }

        let scenario_dir = dir.join("Resilience.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Resilience\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=Good=1\nPosition=120,160\n",
        )
        .expect("write scenario core");
        std::fs::write(scenario_dir.join("Script.c"), scenario_script).expect("write script");
        scenario_dir
    }

    fn write_definition_localization_fixture(
        dir: &std::path::Path,
        script: &str,
        string_table: &str,
    ) -> std::path::PathBuf {
        let scenario_dir = write_resilience_fixture(dir, None, "// no script\n");
        let definition_dir = dir.join("Defs.c4d/Good.c4d");
        std::fs::write(definition_dir.join("Script.c"), script)
            .expect("write definition script");
        std::fs::write(definition_dir.join("StringTblUS.txt"), string_table)
            .expect("write definition string table");
        scenario_dir
    }

    fn apply_resilience_fixture(
        dir: &tempfile::TempDir,
        scenario_dir: &std::path::Path,
    ) -> (Engine, Vec<ObjectId>) {
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        let created = scenario
            .apply(&mut engine)
            .expect("apply tolerates script errors like C++");
        (engine, created)
    }

    #[test]
    fn legacy_scenario_script_replaces_localized_strings_before_compile() {
        // C4Game loads ScenarioLangStringTable before Script.c and
        // C4ScriptHost::MakeScript replaces `$key$` before Preparse
        // (C4Game.cpp:229-230,3336-3341; C4ScriptHost.cpp:66-82).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nglobal func Initialize() { Message(\"$MsgIntro2a$\"); }\n",
        );
        std::fs::write(
            scenario_dir.join("StringTblUS.txt"),
            "MsgIntro2a=Come with me, princess!\n",
        )
        .expect("write string table");
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let script = &scenario.script.as_ref().expect("scenario script").source;

        assert!(script.contains("\"Come with me, princess!\""));
        assert!(!script.contains("$MsgIntro2a$"));

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let snapshot = engine.tick().expect("scenario ticks");
        assert_eq!(snapshot.hud.messages.len(), 1);
        assert_eq!(
            snapshot.hud.messages[0].lines,
            ["Come with me, princess!"]
        );
        assert!(snapshot.audio.iter().all(|command| !matches!(
            command,
            crate::AudioCommand::PlaySound { name, .. } if name == "MsgIntro2a"
        )));
    }

    #[test]
    fn legacy_definition_script_localizes_runtime_strings_without_parse_regressions() {
        // C4Def::Load gives each definition script its local StringTbl and
        // C4ScriptHost::MakeScript replaces `$key$` before Preparse
        // (C4Def.cpp:625-633; C4ScriptHost.cpp:46-82). Localized context text
        // such as `Put/Get` must not make an otherwise valid definition fail.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_definition_localization_fixture(
            dir.path(),
            "#strict\n\
             func Label() { return \"$Reloaded$\"; }\n\
             func ContextAction() {\n\
               [$ActionLabel$|Image=GOOD]\n\
               return 1;\n\
             }\n",
            "Reloaded=Reloaded %dx {{%i}}.\nActionLabel=Put/Get\n",
        );
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("localized definition compiles");
        let id = engine
            .spawn_object(SpawnConfig::new("GOOD"))
            .expect("GOOD spawns");
        let index = engine.find_object_index(id).expect("GOOD object index");

        assert_eq!(
            engine
                .call_object_function(index, "Label", Vec::new())
                .expect("Label runs"),
            lc_script::Value::String("Reloaded %dx {{%i}}.".to_string())
        );
    }

    #[test]
    fn legacy_definition_script_localizes_unquoted_array_values() {
        // C4Def::Load gives each definition script its local StringTbl and
        // C4ScriptHost::MakeScript replaces `$key$` across the whole source
        // before Preparse (C4Def.cpp:625-633; C4ScriptHost.cpp:46-82).
        // Hazard's HHKS definition relies on an unquoted key expanding to a
        // C4Script array literal (Killstats.c4d/StringTblUS.txt:1-6).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_definition_localization_fixture(
            dir.path(),
            "#strict\nfunc Messages() { return $Messages$; }\n",
            "Messages=[\"first\",\"second\"]\n",
        );
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let definition = scenario
            .definitions
            .iter()
            .find(|definition| definition.id == "GOOD")
            .expect("GOOD definition exists");

        assert!(definition.script.contains("return [\"first\",\"second\"]"));
        assert!(!definition.script.contains("$Messages$"));
        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("localized definition compiles");
        let id = engine
            .spawn_object(SpawnConfig::new("GOOD"))
            .expect("GOOD spawns");
        let index = engine.find_object_index(id).expect("GOOD object index");

        assert_eq!(
            engine
                .call_object_function(index, "Messages", Vec::new())
                .expect("Messages runs"),
            lc_script::Value::Array(vec![
                lc_script::Value::String("first".to_string()),
                lc_script::Value::String("second".to_string()),
            ])
        );
    }

    #[test]
    fn legacy_scenario_script_respects_language_order() {
        // C4ComponentHost tries each LanguageEx code in order after the
        // unsuffixed StringTbl.txt candidate (C4ComponentHost.cpp:65-89;
        // C4Components.h:56).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nglobal func Localized() { return \"$MsgIntro2a$\"; }\n",
        );
        std::fs::write(
            scenario_dir.join("StringTblUS.txt"),
            "MsgIntro2a=Come with me, princess!\n",
        )
        .expect("write US string table");
        std::fs::write(
            scenario_dir.join("StringTblDE.txt"),
            "MsgIntro2a=Komm mit mir, Prinzessin!\n",
        )
        .expect("write DE string table");
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario = Scenario::load_from_path_with_languages(
            &scenario_dir,
            &resolver,
            &["DE", "US"],
        )
        .expect("scenario loads");
        let script = &scenario.script.as_ref().expect("scenario script").source;

        assert!(script.contains("\"Komm mit mir, Prinzessin!\""));
        assert!(!script.contains("\"Come with me, princess!\""));
    }

    /// Joins a default test player: the fixture's `[Player1] Crew=Good=1`
    /// places its crew at JOIN like C++ (C4Player::PlaceReadyCrew,
    /// C4Player.cpp:481-570). Returns the objects created by the join.
    fn join_test_player(engine: &mut Engine) -> Vec<ObjectId> {
        let before: std::collections::HashSet<ObjectId> = engine
            .snapshot()
            .objects
            .iter()
            .map(|object| object.id)
            .collect();
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
            })
            .expect("join succeeds");
        engine
            .snapshot()
            .objects
            .iter()
            .map(|object| object.id)
            .filter(|id| !before.contains(id))
            .collect()
    }

    // Fair crew (the LegacyClonk default config; live-oracle probe read
    // UseFairCrew=1, strength 1000): crew physicals come from the def
    // promoted to RankByExperience(1000)=1 (C4Def.cpp:860-874,
    // C4RankSystem.cpp:226-237) — Energy = max(def, 55 * C4MaxPhysical/
    // 100) = 55000 (PromotionUpdate, C4InfoCore.cpp:207-213). GoldRush
    // live oracle: crew read 55000; bandits keep script-set temporary
    // physicals (25000).
    #[test]
    fn crew_infos_promote_physicals_by_rank_like_cpp() {
        let mut engine = Engine::with_seed(3);
        let mut clonk =
            Definition::from_script("CLNK", "Clonk", "#strict\n").expect("clonk compiles");
        clonk.set_crew_member(true);
        clonk.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        clonk.set_physical(crate::PhysicalInfo {
            energy: 50_000,
            scale: 30_000,
            hangle: 30_000,
            swim: 60_000,
            fight: 50_000,
            ..crate::PhysicalInfo::default()
        });
        engine.register_definition(clonk).expect("clonk registers");

        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: vec![crate::player_file::CrewInfo {
                    id: "CLNK".to_string(),
                    name: "Henry".to_string(),
                    rank: 1,
                    experience: 120,
                    participation: 1,
                    in_action: false,
                    has_died: false,
                }],
                startup_player_count: 1,
                control_style: false,
            })
            .expect("join succeeds");

        let crew = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CLNK")
            .expect("crew spawned");
        assert_eq!(
            crew.state.energy, 55_000,
            "rank-1 promotion: max(50000, 55*1000) (C4InfoCore.cpp:212)"
        );
        assert_eq!(
            crew.state.info_physical,
            Some(crate::PhysicalInfo {
                energy: 55_000,
                scale: 33_500,
                hangle: 33_500,
                swim: 62_000,
                fight: 52_500,
                can_scale: 1,
                can_hangle: 1,
                can_dig: 1,
                can_construct: 1,
                can_chop: 1,
                ..crate::PhysicalInfo::default()
            }),
            "FairCrewStrength=1000 promotes the full C4PhysicalInfo like C++"
        );
    }

    #[test]
    fn join_player_runs_scenario_init_with_the_cpp_draw_ledger() {
        // C4Player::ScenarioInit (C4Player.cpp:670-777) consumes the synced
        // RNG in this exact order: Wealth.Evaluate (one draw,
        // C4Scenario.cpp:43-46), all-random start x/y (C4Player.cpp:745-746,
        // 16 + Random(GBack - 32) each), then PlaceReadyCrew draws one
        // Random(tx2 - tx1) per crew member (C4Player.cpp:548) with
        // FindSolidGround settling each position. Crew objects are created
        // at JOIN time — never at scenario load (C4Game::InitPlayers queues
        // CID_JoinPlr; nothing spawns crew during load).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Join\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=2\nWealth=20,5,0,250\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(7);
        scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(
            engine.snapshot().objects.len(),
            0,
            "no crew at load — crew joins with the player like C++"
        );

        let mut replay = engine.rng.clone();
        let landscape = engine.landscape().expect("landscape set").clone();
        let world_width = landscape.width() as i32;
        let world_height = landscape.estimated_height();
        assert_eq!((world_width, world_height), (640, 400));

        // Replay the ledger independently.
        let expected_wealth = LegacyC4SVal::new(20, 5, 0, 250).evaluate(&mut replay);
        let mut ptx = 16 + replay.random(world_width - 32);
        let mut pty = 16 + replay.random(world_height - 32);
        if let Some((nx, ny)) = landscape.find_solid_ground(ptx, pty, 30) {
            ptx = nx;
            pty = ny;
        }
        if let Some((nx, ny)) =
            landscape.find_con_site_spot(ptx, pty, 30, 50, 400, |_, _, _, _| false)
        {
            ptx = nx;
            pty = ny;
        }
        let mut expected_positions = Vec::new();
        for _ in 0..2 {
            let mut ctx = (ptx - 30) + replay.random(60);
            let mut cty = pty;
            if let Some((nx, ny)) = landscape.find_solid_ground(ctx, cty, 0) {
                ctx = nx;
                cty = ny;
            }
            expected_positions.push(Vector2::new(ctx, cty));
        }

        let joined = engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tyler".to_string(),
                team: None,
                color_dw: 0xf40000,
                pref_color: 3,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
            })
            .expect("join succeeds");
        assert_eq!(joined.number, 0);
        assert_eq!((joined.start_x, joined.start_y), (ptx, pty));
        assert!(joined.first_base.is_none());

        // The engine consumed exactly the replayed ledger: same draw count,
        // same LCG state.
        assert_eq!(engine.rng, replay, "RNG stream stays lockstep");

        let player = engine.player(0).expect("player registered");
        assert_eq!(player.wealth(), expected_wealth);
        assert_eq!(player.color_index(), 3, "free PrefColor is taken as-is");

        let snapshot = engine.snapshot();
        let crew: Vec<_> = snapshot
            .objects
            .iter()
            .filter(|object| object.owner == 0 && object.crew_member)
            .collect();
        assert_eq!(crew.len(), 2, "two ready-crew members placed");
        let positions: Vec<_> = crew.iter().map(|object| object.position).collect();
        assert_eq!(positions, expected_positions);

        // Fresh infos: no roster, no name sources -> "Clonk", numbered by
        // MakeValidName (C4ObjectInfoList.cpp:93-101).
        let names: Vec<_> = crew
            .iter()
            .map(|object| {
                engine
                    .crew_object_info(object.id)
                    .expect("crew info recorded")
                    .name
                    .clone()
            })
            .collect();
        assert_eq!(names, vec!["Clonk".to_string(), "Clonk2".to_string()]);
    }

    #[test]
    fn def_core_blast_incinerate_reaches_the_engine_definition() {
        // BlastIncinerate (C4Def.cpp:315) must survive the resource-core
        // apply so C4Object::Blast can consult it (C4Object.cpp:1421-1423).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\nBlastIncinerate=50\n",
        )
        .expect("write defcore");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .definitions
                .get("GOOD")
                .expect("definition registered")
                .blast_incinerate(),
            50
        );
    }

    #[test]
    fn real_system_scripts_explode_blasts_bystanders_end_to_end() {
        // The FLNT class end-to-end: Hit -> Explode -> DoExplosion ->
        // BlastObjects -> BlastObject through the REAL planet/System.c4g
        // scripts (Explode.c, FindObject.c, GetXVal.c). The exploding
        // object removes itself (Explode.c:18), bystanders inside the
        // 10x10 direct-hit rect take the full level as Damage
        // (Explode.c:93-94 -> C4Object::Blast, C4Object.cpp:1416), and
        // the run stays script-error free (the former 'script error in
        // Hit of FLNT' harness class).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        let defs_root = dir.path().join("Defs.c4d");
        let flint = defs_root.join("Flint.c4d");
        std::fs::create_dir_all(&flint).expect("flint dir");
        std::fs::write(
            flint.join("DefCore.txt"),
            "[DefCore]\nid=FLNX\nName=Flint\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
        )
        .expect("flint defcore");
        std::fs::write(
            flint.join("Script.c"),
            "#strict\npublic func ExplodeSize() { return(18); }\nprotected func Hit() { Explode(ExplodeSize()); }\n",
        )
        .expect("flint script");
        let bystander = defs_root.join("Bystander.c4d");
        std::fs::create_dir_all(&bystander).expect("bystander dir");
        std::fs::write(
            bystander.join("DefCore.txt"),
            "[DefCore]\nid=BYST\nName=Bystander\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
        )
        .expect("bystander defcore");
        std::fs::write(bystander.join("Script.c"), "#strict\n").expect("bystander script");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        // The live game installs planet/System.c4g before the scenario.
        let planet = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g");
        let system_scripts: Vec<(String, String)> = ["FindObject.c", "GetXVal.c", "Explode.c"]
            .iter()
            .map(|name| {
                let bytes = std::fs::read(planet.join(name)).expect("system script reads");
                // ISO-8859-1 comments -> chars (the group loader does the
                // same byte-transparent conversion).
                (
                    (*name).to_string(),
                    bytes.iter().map(|&b| b as char).collect::<String>(),
                )
            })
            .collect();
        engine.install_global_scripts(&system_scripts);

        let flint_id = engine
            .spawn_object(SpawnConfig::new("FLNX").with_position(Vector2::new(100, 100)))
            .expect("flint spawns");
        let bystander_id = engine
            .spawn_object(SpawnConfig::new("BYST").with_position(Vector2::new(102, 100)))
            .expect("bystander spawns");
        let flint_idx = engine.find_object_index(flint_id).expect("flint exists");
        engine
            .call_object_function(flint_idx, "Hit", Vec::new())
            .expect("Hit runs without script errors");
        assert!(
            engine
                .find_object_index(flint_id)
                .map(|idx| engine.objects[idx].state.status == ObjectStatus::Deleted)
                .unwrap_or(true),
            "the exploding object removed itself (Explode.c RemoveObject)"
        );
        let bystander_idx = engine
            .find_object_index(bystander_id)
            .expect("bystander survives");
        assert_eq!(
            engine.objects[bystander_idx].state.damage, 18,
            "direct-hit rect victims take the blast level as Damage"
        );
    }

    #[test]
    fn real_system_scripts_shake_viewport_global_effect_end_to_end() {
        // System.c4g ShakeViewPort (planet/System.c4g/Explode.c:166-183)
        // does a nil-target AddEffect("ShakeEffect", pObj, 200, 1) whose
        // global FxShakeEffect* callbacks must fire: Start synchronously
        // inside AddEffect (C4Effect ctor, C4Effect.cpp:128-129), Timer
        // every frame from pGlobalEffects->Execute(nullptr)
        // (C4Game.cpp:830-831, C4Effect.cpp:342-345), and the Timer's -1
        // return kills the effect with its Stop (C4Effect.cpp:350,
        // Explode.c:198 `return(-1)`).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        let probe = dir.path().join("Defs.c4d/Probe.c4d");
        std::fs::create_dir_all(&probe).expect("probe dir");
        std::fs::write(
            probe.join("DefCore.txt"),
            "[DefCore]\nid=PROB\nName=Probe\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
        )
        .expect("probe defcore");
        std::fs::write(
            probe.join("Script.c"),
            "#strict\npublic func Shake() { return(ShakeViewPort(100, 0, 10, 20)); }\n",
        )
        .expect("probe script");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let planet = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../planet/System.c4g");
        let system_scripts: Vec<(String, String)> = ["FindObject.c", "GetXVal.c", "Explode.c"]
            .iter()
            .map(|name| {
                let bytes = std::fs::read(planet.join(name)).expect("system script reads");
                (
                    (*name).to_string(),
                    bytes.iter().map(|&b| b as char).collect::<String>(),
                )
            })
            .collect();
        engine.install_global_scripts(&system_scripts);

        let probe_id = engine
            .spawn_object(SpawnConfig::new("PROB").with_position(Vector2::new(100, 100)))
            .expect("probe spawns");
        let probe_idx = engine.find_object_index(probe_id).expect("probe exists");
        engine
            .call_object_function(probe_idx, "Shake", Vec::new())
            .expect("Shake runs without script errors");

        // FxShakeEffectStart ran inside AddEffect; ShakeViewPort then
        // seeds EffectVar 0..2 with the level and offsets (Explode.c:
        // 175-178).
        assert_eq!(engine.global_effects().len(), 1, "global effect added");
        let effect = &engine.global_effects()[0];
        assert_eq!(effect.name, "ShakeEffect");
        assert_eq!(effect.priority, 200);
        assert_eq!(effect.interval, 1);
        assert_eq!(effect.var(0), crate::EffectVarValue::Int(100));
        assert_eq!(effect.var(1), crate::EffectVarValue::Int(10));
        assert_eq!(effect.var(2), crate::EffectVarValue::Int(20));

        // FxShakeEffectTimer (Explode.c:188-199) fires every frame; its
        // strength formula iLevel/((3*iTime)/2+3)-iTime**2/400 reaches 0
        // at iTime 29 for level 100 -> return(-1) kills the effect.
        let mut death_frame = None;
        for frame in 1..=40 {
            engine.tick().expect("tick runs");
            if engine.global_effects().is_empty() {
                death_frame = Some(frame);
                break;
            }
            assert_eq!(
                engine.global_effects()[0].timer,
                frame,
                "iTime advances every frame while the shake lives"
            );
        }
        assert_eq!(
            death_frame,
            Some(29),
            "the timer's C4Fx_Execute_Kill return removes the global effect"
        );
    }

    #[test]
    fn appendto_scripts_link_into_their_targets_like_c4aullink() {
        // C4AulScript::ResolveAppends (C4AulLink.cpp:29-64) + AppendTo
        // (:114-141): a definition script with `#appendto GOOD` copies its
        // functions into GOOD's script as OVERRIDES (the original stays
        // reachable via inherited), and System.c4g scripts with #appendto
        // do the same (GoldRush's dialogue and AI scripts rely on both).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\n#appendto GOOD\n\
             public func Probe() { return inherited() * 10 + 4; }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Probe() { return 1; }\n",
        )
        .expect("write target script");
        let boost = dir.path().join("Defs.c4d/Boost.c4d");
        std::fs::create_dir_all(&boost).expect("boost dir");
        std::fs::write(
            boost.join("DefCore.txt"),
            "[DefCore]\nid=BOST\nName=Boost\nCategory=0\nCrewMember=0\n",
        )
        .expect("write boost defcore");
        std::fs::write(
            boost.join("Script.c"),
            "#strict\n#appendto GOOD\n\
             public func Probe() { return inherited() * 10 + 2; }\n\
             public func SetAI(szName, iInterval) { return 7; }\n",
        )
        .expect("write boost script");
        let pack_system = dir.path().join("Defs.c4d/System.c4g");
        std::fs::create_dir_all(&pack_system).expect("pack system dir");
        std::fs::write(
            pack_system.join("Append.c"),
            "#strict\n#appendto GOOD\n\
             public func Probe() { return inherited() * 10 + 3; }\n",
        )
        .expect("write pack system append");
        let system = scenario_dir.join("System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Append.c"),
            "#strict\n#appendto GOOD\n\
             static const SYSTEM_APPEND_VALUE = 3;\n\
             public func Probe() { return inherited() * 10 + 5; }\n\
             public func FromSystem() { return SYSTEM_APPEND_VALUE(); }\n",
        )
        .expect("write system append");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let id = engine
            .spawn_object(SpawnConfig::new("GOOD"))
            .expect("target spawns");
        let index = engine.find_object_index(id).expect("object index");
        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("Probe call succeeds"),
            lc_script::Value::Int(12345),
            "appends follow definition, pack System, Script.c, scenario System order"
        );
        assert_eq!(
            engine
                .call_object_function(index, "SetAI", Vec::new())
                .expect("SetAI call succeeds"),
            lc_script::Value::Int(7),
            "appended function exists on the target"
        );
        assert_eq!(
            engine
                .call_object_function(index, "FromSystem", Vec::new())
                .expect("FromSystem call succeeds"),
            lc_script::Value::Int(3),
            "System.c4g appends see their global constants without nil local shadowing"
        );
        assert!(
            engine
                .global_script_functions
                .as_ref()
                .is_none_or(|functions| !functions.contains_key("FromSystem")),
            "a public append function is local to its System host, not engine-global"
        );
    }

    #[test]
    fn objects_created_mid_call_receive_arrow_calls_like_cpp() {
        // C++ CreateObject fully creates the object DURING the call
        // (Game.CreateObject -> NewObject), so `obj->Method()` on the
        // fresh object resolves immediately (GoldRush's DoInitialize does
        // pObj->SetAI(...) right after CreateObject). The copy-in/copy-out
        // model must give pending spawns a callable scope, and their
        // nested outcomes must fold onto the object once spawned.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Mark();\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal hit;\npublic func Mark() { hit = 7; return hit; }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created during Initialize");
        assert_eq!(
            object.local_vars.get("hit"),
            Some(&lc_script::Value::Int(7)),
            "the nested Mark() call ran on the fresh object and folded"
        );
    }

    #[test]
    fn scenario_statics_are_visible_to_definition_scripts() {
        // C4Aul `static` variables live in Game.ScriptEngine.GlobalNamed —
        // ONE table for every script host: GoldRush's scenario Script.c
        // declares `static iDifficulty;` and the appended AI script (in a
        // definition host) reads it (Locals.c4d/AI.c4d SetAI ->
        // SetDifficultyPhysicals(iDifficulty)).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static shared;\n\
             global func Initialize(state, random) {\n\
                 shared = 4;\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Remember();\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal seen;\n\
             public func Remember() { seen = shared; shared = shared + 1; return seen; }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&lc_script::Value::Int(4)),
            "the definition script read the scenario static"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("shared")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(5)),
            "the definition script's write went back to the shared table"
        );
    }

    #[test]
    fn definition_global_funcs_register_engine_wide_like_cpp() {
        // `global func` declarations in DEFINITION scripts belong to
        // Game.ScriptEngine (AA_GLOBAL, C4AulParse preparse): Time.c4d
        // declares `global func IsNight()` and every other script calls it
        // plainly (GetFuncRecursive walks up to the engine,
        // C4Aul.cpp:285-291). Includes/appends never copy global funcs
        // (C4AulLink.cpp:127) — they are reachable through the engine.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Remember();\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal seen;\n\
             public func Remember() { seen = NightCheck(); return seen; }\n",
        )
        .expect("write target script");
        let time = dir.path().join("Defs.c4d/Time.c4d");
        std::fs::create_dir_all(&time).expect("time dir");
        std::fs::write(
            time.join("DefCore.txt"),
            "[DefCore]\nid=TIME\nName=Time\nCategory=0\nCrewMember=0\n",
        )
        .expect("write time defcore");
        std::fs::write(
            time.join("Script.c"),
            "#strict\nglobal func NightCheck() { return 8; }\n",
        )
        .expect("write time script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&lc_script::Value::Int(8)),
            "another def's script called the definition-declared global func"
        );
    }

    #[test]
    fn cross_object_localn_folds_into_the_target_like_cpp() {
        // The GoldRush WSKI pattern (Goldrush.c4s/Script.c:58-62):
        //   pObj = CreateContents(WSKI, pWagon);
        //   LocalN("iWater", pObj) = 90;
        //   pObj->~UpdateGraphics();
        // FnLocalN returns a reference into the TARGET's named locals
        // (C4Script.cpp:4591-4605): the write lands on the fresh object
        // and the nested call right after it sees the new value.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 LocalN(\"iWater\", obj) = 90;\n\
                 obj->Check();\n\
                 LocalN(\"iWater\", obj) += 10;\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal iWater;\nlocal seen;\n\
             public func Check() { seen = iWater; return seen; }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&lc_script::Value::Int(90)),
            "the nested call right after the write saw the new value"
        );
        assert_eq!(
            object.local_vars.get("iWater"),
            Some(&lc_script::Value::Int(100)),
            "the final cell value (write + compound add) folded onto the object"
        );
    }

    #[test]
    fn find_object_uses_the_cpp_argument_layout_and_caller_context() {
        // FnFindObject (C4Script.cpp:2113-2135): parameters are (id, x, y,
        // wdt, hgt, dwOCF, szAction, pActionTarget, vContainer, pFindNext).
        // Local calls EXCLUDE the caller and adjust x/y by the caller's
        // position; vContainer takes an object or the NO_CONTAINER=124 /
        // ANY_CONTAINER=123 sentinels (C4Object.h:83-84) — any other int is
        // simply no filter (C4Value::getObj() yields nil), never an error.
        // GoldRush's cannon Initialize chain depends on this layout
        // (Cannon.c4d/Script.c:31 passes NoContainer() as 9th argument).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some(("BOXD", "// box\n")),
            "global func Initialize() {\n\
                 var a = CreateObject(GOOD, 50, 50, -1);\n\
                 var b = CreateObject(GOOD, 55, 52, -1);\n\
                 var box = CreateObject(BOXD, 90, 90, -1);\n\
                 var c = CreateObject(GOOD, 90, 90, -1);\n\
                 c->Enter(box);\n\
                 a->Probe(b, c);\n\
                 return 1;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\n\
             local iExcluded; local iNoContainer; local iAnyContainer;\n\
             local iFindNext; local iIntTolerant; local iRelative;\n\
             public func Probe(pOther, pContained) {\n\
                 if (FindObject(GOOD) == pOther) iExcluded = 1;\n\
                 if (!FindObject(GOOD, 0,0,0,0, 0, 0, 0, NoContainer(), pOther)) iNoContainer = 1;\n\
                 if (FindObject(GOOD, 0,0,0,0, 0, 0, 0, AnyContainer()) == pContained) iAnyContainer = 1;\n\
                 if (FindObject(GOOD, 0,0,0,0, 0, 0, 0, 0, pOther) == pContained) iFindNext = 1;\n\
                 if (FindObject(GOOD, 0,0,0,0, 0, 0, 0, 7) == pOther) iIntTolerant = 1;\n\
                 if (FindObject(GOOD, -10,-10, 20,20) == pOther) iRelative = 1;\n\
             }\n",
        )
        .expect("write prober script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let prober = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "GOOD")
            .min_by_key(|object| object.id)
            .expect("prober created");
        let flag = |name: &str| prober.local_vars.get(name).cloned();
        assert_eq!(
            flag("iExcluded"),
            Some(lc_script::Value::Int(1)),
            "local calls exclude the caller (C4Script.cpp:2131)"
        );
        assert_eq!(
            flag("iNoContainer"),
            Some(lc_script::Value::Int(1)),
            "NO_CONTAINER in the 9th slot filters contained objects"
        );
        assert_eq!(
            flag("iAnyContainer"),
            Some(lc_script::Value::Int(1)),
            "ANY_CONTAINER in the 9th slot requires containment"
        );
        assert_eq!(
            flag("iFindNext"),
            Some(lc_script::Value::Int(1)),
            "the 10th slot is pFindNext"
        );
        assert_eq!(
            flag("iIntTolerant"),
            Some(lc_script::Value::Int(1)),
            "a non-sentinel int container is no filter, not an error"
        );
        assert_eq!(
            flag("iRelative"),
            Some(lc_script::Value::Int(1)),
            "local calls offset the search rect by the caller's position \
             (C4Script.cpp:2115-2119)"
        );
    }

    #[test]
    fn join_broadcasts_initialize_player_to_rule_objects_like_cpp() {
        // C4GameScriptHost::GRBroadcast (C4ScriptHost.cpp:234-249): every
        // live object with a C4D_Goal|C4D_Rule|C4D_Environment category bit
        // is called BEFORE the scenario script. The join path broadcasts
        // PSF_InitializePlayer this way (C4Player.cpp:769-775) — GoldRush's
        // TeamAccount rule creates the per-player ACNT from it
        // (TeamAccount.c4d/Script.c InitializePlayer).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize() {\n\
                 CreateObject(RULZ, 0, 0, -1);\n\
                 return 1;\n\
             }\n",
        );
        let rule = dir.path().join("Defs.c4d/Rule.c4d");
        std::fs::create_dir_all(&rule).expect("rule dir");
        std::fs::write(
            rule.join("DefCore.txt"),
            "[DefCore]\nid=RULZ\nName=Rule\nCategory=524288\nCrewMember=0\n",
        )
        .expect("write rule defcore");
        std::fs::write(
            rule.join("Script.c"),
            "#strict\nlocal iJoined;\n\
             public func InitializePlayer(iPlr) {\n\
                 iJoined = iPlr + 1;\n\
                 CreateObject(GOOD, 60, 60, iPlr);\n\
                 return 1;\n\
             }\n",
        )
        .expect("write rule script");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        join_test_player(&mut engine);
        let snapshot = engine.snapshot();
        let rule_object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "RULZ")
            .expect("rule object created");
        assert_eq!(
            rule_object.local_vars.get("iJoined"),
            Some(&lc_script::Value::Int(1)),
            "the rule object's InitializePlayer ran for the joining player \
             (GRBroadcast, C4ScriptHost.cpp:234-249)"
        );
        assert!(
            snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GOOD" && object.owner == 0),
            "the rule's InitializePlayer created its per-player object \
             (the TeamAccount ACNT pattern)"
        );
    }

    #[test]
    fn effect_callbacks_run_in_the_command_targets_object_context_like_cpp() {
        // Every effect callback executes with the effect's command target
        // as object context: pFn->Exec(pCommandTarget, ...)
        // (C4Effect.cpp:129,345,392,456) — `this()` is the command target
        // and its object locals are live. GoldRush's bandit AI depends on
        // both: FxAIBanditNoMoveStart does `this()->~ContextDefend()`,
        // equips via CreateContents, and writes the appended local
        // `iOwner=-2` (Goldrush.c4s/Locals.c4d/AI.c4d/Script.c:96-106).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Boot();\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal iSelf;\n\
             public func Boot() { AddEffect(\"Probe\", this(), 1, 0, this()); return 1; }\n\
             public func Tag() { return 1; }\n\
             func FxProbeStart(pThis, iNumber, fTmp) {\n\
                 if (fTmp) return();\n\
                 this()->~Tag();\n\
                 if (this()) iSelf = 1;\n\
                 CreateContents(GOOD);\n\
             }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD" && object.container.is_none())
            .expect("object created");
        assert_eq!(
            object.local_vars.get("iSelf"),
            Some(&lc_script::Value::Int(1)),
            "this() inside the Start callback is the command target \
             (C4Effect.cpp:129), and its direct local write persists"
        );
        assert!(
            snapshot
                .objects
                .iter()
                .any(|candidate| candidate.container == Some(object.id)),
            "CreateContents from the Start callback equips the command \
             target (the GoldRush bandit pattern)"
        );
    }

    // FnGetPlrColorDw (C4Script.cpp:3658-3666): the joined player's
    // resolved 24-bit ColorDw; a missing player reads nil. GoldRush's
    // intro movie Talker colors its text with it.
    #[test]
    fn get_plr_color_dw_returns_the_joined_players_color() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) { return nil; }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal iColor;\n\
             public func Probe(iPlr) {\n\
                 iColor = GetPlrColorDw(iPlr);\n\
                 return(1);\n\
             }\n",
        )
        .expect("write probe script");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        join_test_player(&mut engine);
        let snapshot = engine.snapshot();
        let object_id = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("crew object exists")
            .id;
        let idx = engine.find_object_index(object_id).expect("object exists");
        engine
            .call_object_function(idx, "Probe", vec![lc_script::Value::Int(0)])
            .expect("probe runs");
        let idx = engine.find_object_index(object_id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iColor"),
            Some(&lc_script::Value::Int(0xff0000)),
            "the join color (join_test_player uses color_dw 0xff0000) \
             comes back as C4Player::ColorDw"
        );
    }

    #[test]
    fn goldrush_bandit_order_defend_chain_loads_rifle_like_cpp() {
        // The GoldRush f30 wall: a bandit armed by FxAIBanditNoMoveStart
        // (Goldrush.c4s/Locals.c4d/AI.c4d/Script.c:97-107 — ContextDefend
        // + CreateContents AMBO,AMBO,WINC) must reload at the FIRST
        // OrderDefend timer tick: FxOrderDefendTimer -> ExecuteWatch
        // (Cowboy.c4d/Script.c:641-703) -> WINC::ControlThrow
        // (Winchester.c4d/Script.c:7-31) -> FireRifle (Cowboy:436-456) ->
        // CheckAmmo (Winchester:289-299) -> LoadRifle (Cowboy:499-504);
        // the rifle removes itself and leaves a WCHR crosshair. The
        // fixture distills those scripts with the content call forms.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Boot();\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/ActMap.txt"),
            "[Action]\nName=Walk\nProcedure=NONE\nDirections=2\nFlipDir=1\nLength=16\nDelay=15\nNextAction=Walk\n\n\
             [Action]\nName=AimRifle\nProcedure=NONE\nDirections=2\nFlipDir=1\nLength=10\nDelay=0\nNextAction=Hold\n\n\
             [Action]\nName=LoadRifle\nProcedure=NONE\nDirections=2\nFlipDir=1\nLength=10\nDelay=3\nEndCall=AimAgain\n",
        )
        .expect("write bandit actmap");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            r#"#strict
local iOwner;
local idWeapon;
local iRifleAmmo;
local iAimPhase;
local pOrdrTarget;
local ordrData1, ordrData2;
local iRewrites, iLastAngle;

public func Boot() {
  SetAction("Walk");
  ContextDefend();
  CreateContents(AMBO);
  CreateContents(AMBO);
  CreateContents(WINC);
  iOwner = -2;
  return(1);
}

public func ContextDefend()
{
  if(GetEffect("Order*", this())) RemoveEffect("Order*", this());
  ordrData1 = GetX(); ordrData2 = GetY();
  AddEffect("OrderDefend", this(), 1, 30, this());
  return(1);
}

func FxOrderDefendTimer(pThis, iNumber)
{
  var iDx=Abs(GetX()-ordrData1), iDy=Abs(GetY()-ordrData2);
  if (!pOrdrTarget)
    if (iDx>20 || iDy>50)
      return(SetCommand(this(),"MoveTo",0,ordrData1,ordrData2));
  if (iDx>150 || iDy>150)
    return(1, pOrdrTarget=0);
  pOrdrTarget=FindEnemyUnit();
  if (!pOrdrTarget) return(ExecuteWatch());
  if (ObjectDistance(pOrdrTarget) > 350) return(ExecuteWatch());
  return(1);
}

private func FindEnemyUnit()
{
  // The real Hostile/GetOwner overrides live in the Goldrush AI.c4d
  // appendto; the fixture pins the no-enemy watch path.
  return(0);
}

private func ExecuteWatch()
{
  var pWeapon;
  if(GetCartridgeCount())
  {
    if(pWeapon = FindContents(WINC)) pWeapon->ControlThrow(this());
    if(idWeapon == WINC && !iRifleAmmo) return(LoadRifle());
  }
  if (Random(3)) return(1);
  if (GetAction() eq "Walk" || GetAction() eq "AimRifle")
  {
    SetDir(Random(2));
    var obj = FindObject(WCHR, 0, 0, 0, 0, 0, "Crosshair", this());
    if(obj) { iRewrites++; iLastAngle = Local(0,obj); SetVertexXY(0,-Sin(Local(0,obj),40)*(GetDir()*2-1),Cos(Local(0,obj),40),obj); }
  }
  return(1);
}

private func GetCartridgeCount() { return(GetSpecialCount("IsBullet")); }

private func GetObjectCount(idObj)
{
  var idUnpackedObj;
  if (idUnpackedObj = idObj->~UnpackTo())
    return(GetObjectCount(idUnpackedObj) * idObj->PackCount());
  return(1);
}

private func GetSpecialCount(szTest)
{
  var iCnt, pObj;
  for(var i = 0; pObj = Contents(i); i++)
    if(ObjectCall(pObj, szTest))
      iCnt++;
  for(var i = 0; pObj = Contents(i); i++)
    if(pObj->~UnpackTo())
      if(DefinitionCall(pObj->~UnpackTo(), szTest))
        iCnt += GetObjectCount(pObj);
  return(iCnt);
}

public func FireRifle()
{
  if (!Contents()->~IsRifle()) return(0);
  if (GetAction() eq "Walk")
    if (SetAction("AimRifle"))
      return(1, SetPhase(6));
  return(0);
}

public func LoadRifle()
{
  if(GetAction() eq "AimRifle") { Sound("RifleLoad"); return(SetAction("LoadRifle")); }
}

// Cowboy.c4d/Script.c:266-282: reload the crosshair from the ammo packs
// and resume aiming; Clonk.c4d/Script.c:396-405 GetCartridge.
protected func AimAgain() {
  if(idWeapon == WINC)
    {
    var obj;
    while(iRifleAmmo < 6 && GetCartridgeCount())
      {
      obj = GetCartridge();
      Enter(WINC->GetCrosshair(this()), obj);
      iRifleAmmo++;
      Sound("RifleLoad2");
      }
    SetAction("AimRifle");
    }
  SetPhase(iAimPhase);
  Sound("RifleLoad2");
}

public func GetCartridge()
  {
  var pObj;
  for(var i = 0; pObj = Contents(i); i++)
    if(pObj->~IsCartridgePack())
      return(pObj->~GetItem());
  return(0);
  }

// The post-reload watch idle (ExecuteWatch's Random-gated branch,
// Cowboy.c4d/Script.c:696-701): re-aims the crosshair vertex from the
// stored angle — the f60 live wall read Local(0,obj) here.
public func DoWatchRewrite() {
  SetDir(1);
  var obj = FindObject(WCHR, 0, 0, 0, 0, 0, "Crosshair", this());
  if(obj) SetVertexXY(0,-Sin(Local(0,obj),40)*(GetDir()*2-1),Cos(Local(0,obj),40),obj);
  return(Local(0,obj));
}
"#,
        )
        .expect("write bandit script");
        let defs_root = dir.path().join("Defs.c4d");
        for (id, defcore, script) in [
            (
                "Ammo.c4d",
                "[DefCore]\nid=AMBO\nName=AmmoBox\nCategory=0\nCrewMember=0\n",
                r#"#strict
local iUsedItems;
public func UnpackTo() { return(CSHO); }
public func IsCartridgePack() { return(1); }
public func MaxPackCount() { return(20); }
public func PackCount() { return(MaxPackCount()-LocalN("iUsedItems")); }
public func DoPackCount(iChange)
{
  iUsedItems-=iChange;
  if(PackCount()<=0) return(RemoveObject());
}
public func GetItem()
{
  var obj = CreateContents(UnpackTo(), Contained());
  DoPackCount(-1);
  return(obj);
}
"#,
            ),
            (
                "Shot.c4d",
                "[DefCore]\nid=CSHO\nName=Shot\nCategory=0\nCrewMember=0\n",
                "#strict\npublic func IsBullet() { return(1); }\n",
            ),
            (
                "Crosshair.c4d",
                "[DefCore]\nid=WCHR\nName=Crosshair\nCategory=0\nCrewMember=0\n",
                "#strict\n",
            ),
            (
                "Winchester.c4d",
                "[DefCore]\nid=WINC\nName=Winchester\nCategory=0\nCrewMember=0\nCollectible=1\n",
                r#"#strict
public func IsRifle() { return(1); }
public func ControlThrow(pClonk)
{
  if(!(pClonk->~FireRifle()))
  {
    if(!GetPlrDownDouble(pClonk->GetOwner()))
      return(1);
    else
      return(0);
  }
  SetPhase(6, pClonk);
  var pCross = CreateObject(WCHR, 0, 0, GetOwner(pClonk)); pCross->SetAction("Crosshair", pClonk);
  Local(0,GetCrosshair(pClonk)) = 84;
  WINC->ActualizePhase(pClonk);
  LocalN("iRifleAmmo", pClonk) = ContentsCount();
  while(Contents()) Enter(pCross, Contents());
  LocalN("idWeapon",pClonk)=GetID();
  if(!LocalN("iRifleAmmo", pClonk)) DefinitionCall(GetID(), "CheckAmmo", pClonk);
  RemoveObject();
  return(1);
}
protected func CheckAmmo(pClonk)
{
  if((GetAction(pClonk) ne "AimRifle") && (GetAction(pClonk) ne "RideAimRifle")) return(0);
  if(!pClonk->~GetCartridgeCount()) return(Sound("RevolverNoAmmo", 0, pClonk));
  LocalN("iAimPhase", pClonk)=GetPhase(pClonk);
  pClonk->~LoadRifle();
}
public func GetCrosshair(pClonk)
{
  return(FindObject(WCHR, 0, 0, 0, 0, 0, "Crosshair", pClonk));
}
public func ActualizePhase(pClonk)
{
  var iDir = GetDir(pClonk)*2-1;
  var iAngle = Local(0,GetCrosshair(pClonk));
  var pObj = FindObject(WCHR,0,0,0,0,0,"Crosshair",pClonk);
  SetVertexXY(0,-Sin(iAngle,40)*iDir,Cos(iAngle,40),pObj);
  if(!WildcardMatch(GetAction(pClonk),"*Aim*")) return(1);
  if(iAngle< 90) SetPhase(6,pClonk);
  return(1);
}
"#,
            ),
        ] {
            let def_dir = defs_root.join(id);
            std::fs::create_dir_all(&def_dir).expect("def dir");
            std::fs::write(def_dir.join("DefCore.txt"), defcore).expect("defcore");
            std::fs::write(def_dir.join("Script.c"), script).expect("script");
        }
        // WCHR needs its Crosshair action for SetAction (content ActMap).
        std::fs::write(
            defs_root.join("Crosshair.c4d/ActMap.txt"),
            "[Action]\nName=Crosshair\nProcedure=ATTACH\nLength=1\nDelay=1\nNextAction=Crosshair\n",
        )
        .expect("crosshair actmap");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        // The live game installs planet/System.c4g before the scenario;
        // WINC::ActualizePhase needs its SetVertexXY helper
        // (planet/System.c4g/Commits.c:68-76).
        engine.install_global_scripts(&[(
            "Commits.c".to_string(),
            "#strict\n\
             global func SetVertexXY(int index, int x, int y, object obj) {\n\
               if (!obj && !this()) return(0);\n\
               if (!SetVertex(index, 0, x, obj)) return(0);\n\
               if (!SetVertex(index, 1, y, obj)) return(0);\n\
               return(1);\n\
             }\n"
                .to_string(),
        )]);
        // The OrderDefend effect (interval 30) first fires 30 execs after
        // the scenario-apply Boot (C4Effect::Execute iTime % iIntervall,
        // C4Effect.cpp:340-345).
        for _ in 0..31 {
            engine.tick().expect("tick");
        }
        let snapshot = engine.snapshot();
        let bandit = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("bandit exists");
        assert_eq!(
            bandit.action.name, "LoadRifle",
            "the first OrderDefend tick walks the whole load chain \
             (cpp GoldRush f30: bandits enter LoadRifle)"
        );
        assert_eq!(
            bandit.local_vars.get("idWeapon"),
            Some(&lc_script::Value::C4Id("WINC".into())),
            "ControlThrow recorded the weapon id on the clonk \
             (Winchester.c4d/Script.c:25)"
        );
        assert_eq!(
            bandit.local_vars.get("iRifleAmmo"),
            Some(&lc_script::Value::Int(0)),
            "the empty rifle left iRifleAmmo=0 (Winchester:22)"
        );
        assert_eq!(
            bandit.local_vars.get("iAimPhase"),
            Some(&lc_script::Value::Int(6)),
            "CheckAmmo stored the aim phase before reloading \
             (Winchester:297, FireRifle set phase 6)"
        );
        assert_eq!(
            snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == "WCHR")
                .count(),
            1,
            "ControlThrow created the crosshair (Winchester:18)"
        );
        assert!(
            !snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "WINC"),
            "the rifle removed itself at the end of ControlThrow \
             (Winchester:29 - cpp removes the bandits' rifles at f30)"
        );

        // The f60 class, effect-scope form: keep ticking until the
        // Random-gated watch idle (Cowboy.c4d/Script.c:694-702) fires at
        // least once INSIDE FxOrderDefendTimer; the re-read of the stored
        // angle must be 84, never nil (a nil read flattens the vertex to
        // Sin(0,40)=0 — the f60 live wall's crosshairs at owner+0).
        let bandit_id = bandit.id;
        for _ in 0..200 {
            engine.tick().expect("tick");
        }
        let bandit_idx = engine.find_object_index(bandit_id).expect("bandit exists");
        let rewrites = engine.objects[bandit_idx]
            .state
            .local_vars
            .get("iRewrites")
            .cloned()
            .unwrap_or(lc_script::Value::Nil);
        assert!(
            matches!(rewrites, lc_script::Value::Int(n) if n >= 1),
            "the watch idle rewrite fired at least once in 200 ticks \
             (got {rewrites:?})"
        );
        assert_eq!(
            engine.objects[bandit_idx].state.local_vars.get("iLastAngle"),
            Some(&lc_script::Value::Int(84)),
            "the effect-scope Local(0,obj) re-read sees the stored angle"
        );
        let snapshot = engine.snapshot();
        let cross_state = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "WCHR")
            .expect("crosshair exists");
        let vertex = cross_state.vertices.first().expect("crosshair vertex");
        assert_eq!(
            vertex.x.abs(),
            40,
            "the rewritten vertex keeps the Sin(84,40) magnitude \
             (got {:?})",
            (vertex.x, vertex.y)
        );

        let bandit_idx = engine.find_object_index(bandit_id).expect("bandit exists");
        let angle = engine
            .call_object_function(bandit_idx, "DoWatchRewrite", Vec::new())
            .expect("watch rewrite runs");
        assert_eq!(
            angle,
            lc_script::Value::Int(84),
            "Local(0, FindObject(WCHR, ..., \"Crosshair\", this())) reads \
             the angle stored at load time (Winchester.c4d/Script.c:19)"
        );
        let snapshot = engine.snapshot();
        let cross = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "WCHR")
            .expect("crosshair exists");
        let vertex = cross.vertices.first().expect("crosshair vertex");
        assert_eq!(
            (vertex.x, vertex.y),
            (-40, 4),
            "the rewrite recomputes the vertex from the stored 84 and the \
             Right facing (-Sin(84,40)*1, Cos(84,40))"
        );
    }

    #[test]
    fn namespaced_object_calls_run_the_named_defs_function_on_the_target() {
        // `obj->ID::Func(...)` (AB_CALLNS, C4AulParse.cpp:3160-3245):
        // the function resolves in def ID's script at parse time and runs
        // with the arrow TARGET as context — GoldRush hitches the horse
        // with pObj->CHBM::Connect(...). The target's own same-name
        // function is bypassed.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->HLPR::Tag();\n\
                 return nil;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal seen;\npublic func Tag() { seen = 1; return seen; }\n",
        )
        .expect("write target script");
        let helper = dir.path().join("Defs.c4d/Helper.c4d");
        std::fs::create_dir_all(&helper).expect("helper dir");
        std::fs::write(
            helper.join("DefCore.txt"),
            "[DefCore]\nid=HLPR\nName=Helper\nCategory=0\nCrewMember=0\n",
        )
        .expect("write helper defcore");
        std::fs::write(
            helper.join("Script.c"),
            "#strict\nlocal seen;\npublic func Tag() { seen = 5; return seen; }\n",
        )
        .expect("write helper script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&lc_script::Value::Int(5)),
            "HLPR's code ran with the GOOD object as context"
        );
    }

    #[test]
    fn legacy_scenario_callbacks_use_the_cpp_argument_convention() {
        // C++ scenario calls pass NO synthetic state argument:
        // Game.Script.Call(PSF_Initialize) has no parameters and
        // GRBroadcast(PSF_InitializePlayer, {plr, x, y, base, team, extra})
        // starts with the PLAYER NUMBER (C4Player.cpp:769-775). The
        // state-proplist convention stays a JSON-fixture convenience —
        // legacy content had been receiving shifted arguments
        // (GoldRush's GetCrew(iPlr, ...) got the state map as iPlr).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static joined_player;\nstatic init_arg;\n\
             global func Initialize(first) { init_arg = first; return nil; }\n\
             global func InitializePlayer(plr) { joined_player = plr; return nil; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_arg")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Nil),
            "Initialize runs with NO arguments for legacy content"
        );
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
            })
            .expect("join succeeds");
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("joined_player")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(0)),
            "InitializePlayer's first argument is the player NUMBER"
        );
    }

    #[test]
    fn definition_pack_system_groups_load_into_the_global_engine() {
        // C4DefList::Load opens C4CFN_System inside every definition
        // group and registers its scripts with Game.ScriptEngine
        // (C4Def.cpp:956-977) — Western.c4d/System.c4g carries Find_Clan
        // and friends. They must be callable like any global script.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static probed;\n\
             global func Initialize() { probed = PackHelper() + PACK_ORDER; return nil; }\n",
        );
        let system = dir.path().join("Defs.c4d/System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Helpers.c"),
            "#strict\nstatic const PACK_ORDER = 20;\n\
             global func PackHelper() { return 6; }\n",
        )
        .expect("write pack script");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nstatic const PACK_ORDER = 10;\n",
        )
        .expect("write definition script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("probed")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(26)),
            "the pack's System.c4g loads after its child definitions"
        );
    }

    #[test]
    fn nested_scenario_definition_pack_reenables_system_loading() {
        // InitDefs disables System loading only for the scenario root call.
        // Recursive *.c4d loads use fLoadSysGroups=true again
        // (C4Def.cpp:903-907,939-968).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic nested_result;\n\
             global func Initialize() { nested_result = NestedHelper(); }\n",
        );
        let nested_system = scenario_dir.join("Helpers.c4d/System.c4g");
        std::fs::create_dir_all(&nested_system).expect("nested system dir");
        std::fs::write(
            nested_system.join("Helpers.c"),
            "#strict\nglobal func NestedHelper() { return 73; }\n",
        )
        .expect("write nested helper");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("nested_result")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(73))
        );
    }

    #[test]
    fn overloaded_definition_keeps_declarations_but_loses_functions_and_appends() {
        // C4Def::Load preparses Script.c before C4DefList::Add replaces an
        // existing ID. Destroying the old host unregisters its functions and
        // appends, but GlobalNamed/GlobalConsts are not rolled back
        // (C4Def.cpp:625-633,927-933,1059-1091; C4Aul.cpp:473-481).
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let old = defs.join("Old.c4d");
        let target = defs.join("Target.c4d");
        std::fs::create_dir_all(&old).expect("old def dir");
        std::fs::create_dir_all(&target).expect("target def dir");
        std::fs::write(
            old.join("DefCore.txt"),
            "[DefCore]\nid=DUPS\nName=Old\nCategory=0\n",
        )
        .expect("write old core");
        std::fs::write(
            old.join("Script.c"),
            "#strict\nstatic const SURVIVES_REPLACEMENT = 7;\n\
             global func Clash() { return 1; }\n\
             #appendto TARG\npublic func Hook() { return inherited() * 10 + 1; }\n",
        )
        .expect("write old script");
        std::fs::write(
            target.join("DefCore.txt"),
            "[DefCore]\nid=TARG\nName=Target\nCategory=0\n",
        )
        .expect("write target core");
        std::fs::write(
            target.join("Script.c"),
            "#strict\npublic func Hook() { return 0; }\n",
        )
        .expect("write target script");
        let pack_system = defs.join("System.c4g");
        std::fs::create_dir_all(&pack_system).expect("pack system dir");
        std::fs::write(
            pack_system.join("Globals.c"),
            "#strict\nglobal func Clash() { return 15; }\n\
             public func PackPrivate() { return 9; }\n",
        )
        .expect("write pack globals");

        let scenario_dir = dir.path().join("Replacement.c4s");
        let replacement = scenario_dir.join("Replacement.c4d");
        std::fs::create_dir_all(&replacement).expect("replacement def dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Replacement\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            replacement.join("DefCore.txt"),
            "[DefCore]\nid=DUPS\nName=New\nCategory=0\n",
        )
        .expect("write replacement core");
        std::fs::write(
            replacement.join("Script.c"),
            "#strict\nglobal func Clash() { return 2; }\n\
             #appendto TARG\npublic func Hook() { return inherited() * 10 + 2; }\n",
        )
        .expect("write replacement script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        assert_eq!(
            engine
                .script_global_consts
                .borrow()
                .get("SURVIVES_REPLACEMENT")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(7)),
            "the removed host's preparsed constant remains registered"
        );
        let globals = engine
            .global_script_functions
            .as_ref()
            .expect("global table exists");
        let clash = globals.get("Clash").expect("replacement Clash exists");
        assert!(clash.overloaded.is_some(), "pack System is inherited");
        assert!(
            clash
                .overloaded
                .as_ref()
                .is_some_and(|parent| parent.overloaded.is_none()),
            "the removed definition function is absent from the overload chain"
        );
        assert!(!globals.contains_key("PackPrivate"));
        let id = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let index = engine.find_object_index(id).expect("target index");
        assert_eq!(
            engine
                .call_object_function(index, "Hook", Vec::new())
                .expect("Hook runs"),
            lc_script::Value::Int(2),
            "only the live replacement append participates"
        );
    }

    #[test]
    fn join_name_sources_and_map_zoom_follow_cpp() {
        // New crew infos draw their name from the def's ClonkNames list
        // when it has one (C4ObjectInfoList.cpp:160-164, C4Def.cpp:645-652),
        // else from Game.Names — which a scenario Names.txt overrides
        // (C4Game.cpp:3288-3289). A configured [PlayerN] Position
        // multiplies a MapZoom.Evaluate per coordinate
        // (C4Player.cpp:713-714) — one synced draw each.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/ClonkNames.txt"),
            "Jim\nBob\nJoe\n",
        )
        .expect("write clonk names");
        let plain = dir.path().join("Defs.c4d/Plain.c4d");
        std::fs::create_dir_all(&plain).expect("plain def dir");
        std::fs::write(
            plain.join("DefCore.txt"),
            "[DefCore]\nid=PLAI\nName=Plain\nCategory=0\nCrewMember=1\n",
        )
        .expect("write plain defcore");
        std::fs::write(plain.join("Script.c"), "// plain\n").expect("write plain script");
        std::fs::write(scenario_dir.join("Names.txt"), "Alpha\nBeta\n")
            .expect("write scenario names");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Names\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=1;PLAI=1\nPosition=20,30\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(99);
        scenario.apply(&mut engine).expect("scenario applies");

        let mut replay = engine.rng.clone();
        let landscape = engine.landscape().expect("landscape set").clone();

        // Wealth (default C4SVal(0,0,0,250)) — one draw.
        LegacyC4SVal::new(0, 0, 0, 250).evaluate(&mut replay);
        // Position 20,30 with MapZoom (10,0,5,15) — one draw per axis.
        let mut ptx = (20 * LegacyC4SVal::new(10, 0, 5, 15).evaluate(&mut replay)).clamp(0, 639);
        let mut pty = (30 * LegacyC4SVal::new(10, 0, 5, 15).evaluate(&mut replay)).clamp(0, 399);
        if let Some((nx, ny)) = landscape.find_solid_ground(ptx, pty, 30) {
            ptx = nx;
            pty = ny;
        }
        if let Some((nx, ny)) =
            landscape.find_con_site_spot(ptx, pty, 30, 50, 400, |_, _, _, _| false)
        {
            ptx = nx;
            pty = ny;
        }
        let _ = (ptx, pty);
        // Crew member 1 (GOOD): name from ClonkNames — Random over the
        // newline count (3) — then the placement draw.
        let good_names = ["Jim", "Bob", "Joe"];
        let expected_good_name = good_names[replay.random(3) as usize];
        replay.random(60);
        // Crew member 2 (PLAI): no ClonkNames — name from the scenario
        // Names.txt ("Alpha\nBeta\n" has 2 newlines) — then placement.
        let scenario_names = ["Alpha", "Beta"];
        let expected_plain_name = scenario_names[replay.random(2) as usize];
        replay.random(60);

        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
            })
            .expect("join succeeds");
        assert_eq!(engine.rng, replay, "draw ledger matches");

        let snapshot = engine.snapshot();
        let names: Vec<(String, String)> = snapshot
            .objects
            .iter()
            .filter(|object| object.crew_member)
            .map(|object| {
                (
                    object.definition_id.clone(),
                    engine
                        .crew_object_info(object.id)
                        .expect("crew info recorded")
                        .name
                        .clone(),
                )
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("GOOD".to_string(), expected_good_name.to_string()),
                ("PLAI".to_string(), expected_plain_name.to_string()),
            ]
        );
    }

    #[test]
    fn legacy_player_starts_are_retained_for_the_join_pipeline() {
        // C4SPlrStart (compiled at C4Scenario.cpp:276-291) feeds
        // C4Player::ScenarioInit at join time (C4Player.cpp:670-777):
        // after apply the engine must still know all four start slots —
        // wealth/crew/position/ready lists — for joining players.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Starts\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Player1]\nWealth=50,10,0,250\nCrew=GOOD=2\nBuildings=GOOD=1\n\
             Vehicles=GOOD=1\nMaterial=GOOD=2\nKnowledge=GOOD=1\n\
             HomeBaseMaterial=GOOD=3\nHomeBaseProduction=GOOD=2\nMagic=GOOD=0\n\
             Position=120,160\nEnforcePosition=1\nStandardCrew=GOOD\nClonks=2,0,1,10\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let start = engine.player_start(0).expect("start slot 0 exists");
        assert_eq!(start.wealth, LegacyC4SVal::new(50, 10, 0, 250));
        assert_eq!(start.crew_count, LegacyC4SVal::new(2, 0, 1, 10));
        assert_eq!(start.native_crew.as_deref(), Some("GOOD"));
        assert_eq!(start.position, [120, 160]);
        assert!(start.enforce_position);
        assert_eq!(start.ready_crew, vec![("GOOD".to_string(), 2)]);
        assert_eq!(start.ready_base, vec![("GOOD".to_string(), 1)]);
        assert_eq!(start.ready_vehic, vec![("GOOD".to_string(), 1)]);
        assert_eq!(start.ready_material, vec![("GOOD".to_string(), 2)]);
        assert_eq!(start.build_knowledge, vec![("GOOD".to_string(), 1)]);
        assert_eq!(start.home_base_material, vec![("GOOD".to_string(), 3)]);
        assert_eq!(start.home_base_production, vec![("GOOD".to_string(), 2)]);
        // A zero count stays zero (GoldRush pins `Magic=EXTG=0;`).
        assert_eq!(start.magic, vec![("GOOD".to_string(), 0)]);

        // Unconfigured slots carry the C4SPlrStart defaults
        // (C4Scenario.cpp:294-300 Default()): Wealth (0,0,0,250),
        // Clonks (1,0,1,10), Position (-1,-1).
        let other = engine.player_start(3).expect("start slot 3 exists");
        assert_eq!(other.wealth, LegacyC4SVal::new(0, 0, 0, 250));
        assert_eq!(other.crew_count, LegacyC4SVal::new(1, 0, 1, 10));
        assert_eq!(other.position, [-1, -1]);
        assert!(other.ready_crew.is_empty());
        assert!(engine.player_start(4).is_none(), "only four start slots");
    }

    #[test]
    fn legacy_numbers_tolerate_trailing_junk_like_cpp() {
        // StdCompilerINIRead reads numbers strtol-style: the leading integer
        // parses, trailing junk is ignored. Real content relies on it —
        // `Position=22,28;` (Missions.c4f/LastWill.c4s/Scenario.txt:21).
        assert_eq!(
            parse_position("Position", "22,28;").expect("position parses"),
            [22, 28]
        );
        assert_eq!(parse_i32("7663;").expect("parses"), 7663);
        assert_eq!(parse_i32("-15x").expect("parses"), -15);
        assert_eq!(parse_i32(" 42 trailing words").expect("parses"), 42);
        assert!(parse_i32("junk").is_err(), "no digits is still an error");
    }

    #[test]
    fn map_zoom_defaults_to_ten_and_clamps_like_cpp() {
        // C4SLandscape::Default: MapZoom = C4SVal(10, 0, 5, 15)
        // (C4Scenario.cpp:307,353); Evaluate stays within [Min, Max].
        assert_eq!(legacy_map_zoom(None), 10, "absent key uses the C4S default");
        let entries = vec![("MapZoom".to_string(), "8".to_string())];
        assert_eq!(legacy_map_zoom(Some(&entries)), 8);
        let entries = vec![("MapZoom".to_string(), "1".to_string())];
        assert_eq!(legacy_map_zoom(Some(&entries)), 5, "clamped to Min=5");
        let entries = vec![("MapZoom".to_string(), "99".to_string())];
        assert_eq!(legacy_map_zoom(Some(&entries)), 15, "clamped to Max=15");
    }

    #[test]
    fn objects_dir_values_round_trip_as_raw_ints_like_cpp() {
        // C4Action::CompileFunc reads and writes Dir as a plain int with no
        // validation (C4Action.cpp:45-54). Multi-directional definitions use
        // values beyond DIR_Right; Dragon Rock contains Dir=13 banners.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nAction=Float\nDir=8\nComDir=5\n\n[Object]\nid=GOOD\nNumber=11\nStatus=1\nCategory=0\nX=20\nY=20\nAction=Float\nDir=13\nComDir=5\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(10))
                .expect("Dir=8 object loaded")
                .direction
                .to_script_value(),
            8
        );
        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(11))
                .expect("Dir=13 object loaded")
                .direction
                .to_script_value(),
            13
        );
    }

    #[test]
    fn objects_comdir_values_round_trip_as_raw_ints_like_cpp() {
        // C4Action::CompileFunc reads and writes ComDir as a plain int with
        // no range validation (C4Action.cpp:45-54). Shipped scenarios use
        // ComDir=200 for persisted WDWB objects.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nComDir=200\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(10))
                .expect("ComDir=200 object loaded")
                .command_direction
                .to_script_value(),
            200
        );
    }

    #[test]
    fn standard_crew_with_clonks_count_spawns_native_crew_like_cpp() {
        // [PlayerN] `Clonks=` is the C4SVal crew COUNT — `Crew` in
        // C4SPlrStart, default C4SVal(1,0,1,10) (C4Scenario.cpp:261,279) —
        // and `StandardCrew=` names the native crew def (NativeCrew, :278).
        // It is NOT a crew-ID list ('Clonks=5,0,1,10' must not become
        // "unknown definition `5,0,1,10`").
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=NativeCrew\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nStandardCrew=GOOD\nClonks=2,0,1,10\nPosition=120,160\n",
        )
        .expect("write scenario core");
        // Old-spec PlaceReadyCrew (C4Player.cpp:489-526) evaluates the
        // count with a synced draw and places NativeCrew members at JOIN
        // time — nothing spawns at load.
        let (mut engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 0, "no crew at load");

        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
            })
            .expect("join succeeds");
        let snapshot = engine.snapshot();
        let crew: Vec<_> = snapshot
            .objects
            .iter()
            .filter(|object| object.owner == 0 && object.crew_member)
            .collect();
        assert_eq!(crew.len(), 2, "Clonks Std=2 native crew at join");
        for object in &crew {
            assert_eq!(object.definition_id, "GOOD");
        }
    }

    #[test]
    fn scenario_local_definition_children_load_and_override_packs() {
        // C++ loads the scenario group itself as the LAST definition source
        // with fOverload (C4Game::InitDefs): any .c4d child of the .c4s is
        // a definition, and it overrides same-id pack definitions
        // (Drachenfels.c4s carries Chest.c4d/_CST and friends directly).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        // A local def new to the scenario...
        let local = scenario_dir.join("Thing.c4d");
        std::fs::create_dir_all(&local).expect("local def dir");
        std::fs::write(
            local.join("DefCore.txt"),
            "[DefCore]\nid=THNG\nName=Thing\nCategory=0\nCrewMember=0\n",
        )
        .expect("write local defcore");
        std::fs::write(local.join("Script.c"), "func Tag() { return 5; }\n")
            .expect("write local script");
        // ...and a local override of the pack's GOOD definition.
        let shadow = scenario_dir.join("Good.c4d");
        std::fs::create_dir_all(&shadow).expect("shadow def dir");
        std::fs::write(
            shadow.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=LocalGood\nCategory=0\nCrewMember=0\n",
        )
        .expect("write shadow defcore");
        std::fs::write(shadow.join("Script.c"), "// local override\n")
            .expect("write shadow script");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=THNG\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.object_snapshot(ObjectId::new(10)).is_some(),
            "the local-child definition resolved for Objects.txt"
        );
        assert_eq!(
            engine
                .definitions
                .get("GOOD")
                .map(|definition| definition.name().to_string()),
            Some("LocalGood".to_string()),
            "the scenario-local definition overrides the pack's (fOverload)"
        );
    }

    #[test]
    fn folder_local_definitions_resolve_for_scenarios_like_cpp() {
        // C++ loads the parent folder chain as definition sources: a .c4d
        // inside the .c4f serves every scenario in the folder (Hazard.c4f/
        // ScenObjects.c4d provides _DIA to Tutorial.c4s).
        let dir = tempdir().expect("tempdir");
        let folder = dir.path().join("Pack.c4f");
        let shared = folder.join("Shared.c4d");
        std::fs::create_dir_all(&shared).expect("shared def dir");
        std::fs::write(
            shared.join("DefCore.txt"),
            "[DefCore]\nid=SHRD\nName=Shared\nCategory=0\nCrewMember=0\n",
        )
        .expect("write shared defcore");
        std::fs::write(shared.join("Script.c"), "// shared\n").expect("write shared script");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");

        let scenario_dir = folder.join("Inner.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=FolderLocal\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=Good=1\nPosition=10,10\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=SHRD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        assert!(
            engine.object_snapshot(ObjectId::new(10)).is_some(),
            "the folder-local definition resolved for Objects.txt"
        );
    }
    #[test]
    fn scenario_local_system_c4g_installs_global_scripts() {
        // C4Game::LoadScenarioScripts (C4Game.cpp:3317-3343) loads every
        // script in the scenario's own System.c4g into the global script
        // engine AFTER definitions for overload priority — GoldRush's 31
        // dialogue/helper scripts and Drachenfels' constants rely on this.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Local.c4s");
        let system = scenario_dir.join("System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Helpers.c"),
            "#strict\nstatic const SYSTEM_VALUE = 42;\n\
             static const DERIVED_VALUE = SCENARIO_VALUE;\n\
             global func ScenarioLocalHelper() { return SYSTEM_VALUE(); }\n",
        )
        .expect("write helper script");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nstatic const SYSTEM_VALUE = 31;\n\
             static const SCENARIO_VALUE = 40;\n",
        )
        .expect("write scenario script");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=LocalSystem\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        let good = dir.path().join("Defs.c4d").join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(
            good.join("Script.c"),
            "#strict\nstatic const SYSTEM_VALUE = 17;\n\
             func Probe() { return ScenarioLocalHelper(); }\n",
        )
        .expect("write script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        assert!(
            engine
                .global_script_functions
                .as_ref()
                .is_some_and(|table| table.contains_key("ScenarioLocalHelper")),
            "scenario System.c4g functions reach the global script engine"
        );
        let id = engine
            .spawn_object(SpawnConfig::new("GOOD"))
            .expect("target spawns");
        let index = engine.find_object_index(id).expect("target index");
        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("Probe call succeeds"),
            lc_script::Value::Int(42),
            "scenario System.c4g constants override definition constants"
        );
        assert_eq!(
            engine
                .script_global_consts
                .borrow()
                .get("DERIVED_VALUE")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(40)),
            "scenario System.c4g constants can reference earlier Script.c constants"
        );
    }

    #[test]
    fn scenario_script_globals_exist_before_environment_initialization() {
        // LoadScenarioScripts runs before LinkScriptEngine and InitGame's
        // InitEnvironment object creation (C4Game.cpp:2615-2622, 2493-2503).
        // Those object callbacks therefore see scenario Script.c globals.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic const SCENARIO_READY = 57;\n",
        );
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=ScenarioOrder\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\n\n[Environment]\nObjects=GOOD=1;\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0u8, 0][..], &[0u8, 0][..]]),
        )
        .expect("write landscape");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nstatic initialized_from_scenario;\n\
             func Initialize() { initialized_from_scenario = SCENARIO_READY; }\n",
        )
        .expect("write definition script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("initialized_from_scenario")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(57))
        );
    }

    #[test]
    fn initialize_may_remove_its_own_object_like_cpp() {
        // Placer objects legitimately self-remove in Initialize (the
        // Environment Grass distributor calls RemoveObject() after placing,
        // Objects.c4d/Environment.c4d/Grass.c4d). C++ has no restriction;
        // the object simply ends removed.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Initialize() { RemoveObject(); return 1; }\n",
        )
        .expect("write self-removing script");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
            })
            .expect("the join itself succeeds");
        let lingering = engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.crew_member && object.status == ObjectStatus::Normal);
        assert!(!lingering, "the object ends removed like C++");
    }

    #[test]
    fn create_object_of_unknown_definition_is_nil_not_fatal() {
        // C++ CreateObject resolves the id with C4Id2Def and returns
        // nullptr when it is unknown — never an engine error.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Initialize() { CreateObject(\"XXXX\", 0, 0, -1); return 1; }\n",
        )
        .expect("write spawning script");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let joined = join_test_player(&mut engine);
        assert_eq!(joined.len(), 1, "only the crew member itself spawns");
        assert!(engine.object_snapshot(joined[0]).is_some());
    }

    #[test]
    fn orphan_container_references_spawn_uncontained_like_cpp() {
        // C++ creates all Objects.txt objects first and resolves Contained
        // by number afterwards (denumeration): a missing container leaves
        // the object uncontained — never a load failure. Drachenfels/
        // Hammerfest hit this when a container's definition is skipped.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nContained=999\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine
            .object_snapshot(ObjectId::new(10))
            .expect("the object spawned");
        assert_eq!(
            snapshot.container, None,
            "missing container resolves to uncontained (nullptr denumeration)"
        );
    }

    #[test]
    fn objects_txt_unknown_definitions_are_skipped_like_cpp() {
        // C++ creates Objects.txt objects via C4Id2Def per entry; an unknown
        // id simply produces no object (logged), the rest of the scenario
        // loads. 19 real scenarios reference defs outside their resolver
        // scope and must not hard-fail.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=MISS\nNumber=9\nStatus=1\nCategory=0\nX=1\nY=2\n\n[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.object_snapshot(ObjectId::new(10)).is_some(),
            "the known object spawned"
        );
        assert!(
            engine.object_snapshot(ObjectId::new(9)).is_none(),
            "the unknown-definition object was skipped"
        );
    }

    #[test]
    fn objects_txt_tolerates_windows_1252_like_cpp() {
        // C++ reads Objects.txt as raw bytes (the config charset); a
        // Windows-1252 umlaut must not abort the load
        // (Fantasy.c4f/Drachenfels.c4s fails strict UTF-8 today).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let mut objects = Vec::new();
        objects.extend_from_slice(
            b"# M\xe4dchen\n[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        );
        std::fs::write(scenario_dir.join("Objects.txt"), objects).expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(engine.object_snapshot(ObjectId::new(10)).is_some());
    }

    #[test]
    fn definition_script_parse_errors_are_logged_not_fatal_like_cpp() {
        // C4Def::Load ignores the Script.Load result (C4Def.cpp:632): a
        // definition whose Script.c fails to parse still loads — script-less
        // — and the rest of the scenario is unaffected.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some(("BRKN", "func {{{ not a script\n")),
            "global func Initialize(state, random) { return nil; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            join_test_player(&mut engine).len(),
            1,
            "the good crew member still spawns at join"
        );
        assert!(
            engine.definitions.contains_key("BRKN"),
            "the broken-script definition is registered script-less (C4Def.cpp:632)"
        );
        assert!(engine.definitions.contains_key("GOOD"));
    }

    #[test]
    fn construction_callback_errors_are_logged_not_fatal_like_cpp() {
        // Engine-initiated lifecycle calls are fail-safe in C++
        // (fPassErrors=false → the error logs and the call yields nil,
        // C4AulExec.cpp:1318-1342); the object still spawns.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        // Replace the good def's script with one whose Construction errors.
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Construction() { return NoSuchFunctionAnywhere(); }\n",
        )
        .expect("write erroring script");
        let (mut engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 0, "crew joins with the player, not at load");
        let joined = join_test_player(&mut engine);
        assert_eq!(
            joined.len(),
            1,
            "the crew object spawns despite the Construction error"
        );
        assert!(engine.object_snapshot(joined[0]).is_some());
    }

    #[test]
    fn scenario_initialize_errors_are_logged_not_fatal_like_cpp() {
        // The scenario script's Initialize is a game call (fail-safe): a
        // runtime error logs and the round starts anyway.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) { return BadCall(); }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.scenario_script.is_some(),
            "the scenario script stays installed after the Initialize error"
        );
        assert_eq!(
            join_test_player(&mut engine).len(),
            1,
            "the round continues: a player can still join"
        );
    }

    #[test]
    fn definition_initialize_runs_before_scenario_initialize() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "INIT",
                "static init_def_calls;\n\
                 static init_def_section;\n\
                 static init_def_id;\n\
                 static init_def_value;\n\
                 static init_def_action_length;\n\
                 func InitializeDef(section) {\n\
                     init_def_calls = 1;\n\
                     init_def_section = section;\n\
                     init_def_id = GetID();\n\
                     init_def_value = GetDefCoreVal(\"Value\");\n\
                     init_def_action_length = GetActMapVal(\"Length\", \"Probe\");\n\
                     CreateObject(\"GOOD\", 10, 20, -1);\n\
                     return 1;\n\
                 }\n",
            )),
            "static scenario_saw_init_def;\n\
             global func Initialize() {\n\
                 scenario_saw_init_def = init_def_calls;\n\
                 return 1;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/INIT.c4d/DefCore.txt"),
            "[DefCore]\nid=INIT\nName=Initializer\nCategory=0\nValue=23\n",
        )
        .expect("write initializer defcore");
        std::fs::write(
            dir.path().join("Defs.c4d/INIT.c4d/ActMap.txt"),
            "[Action]\nName=Probe\nProcedure=NONE\nLength=7\n",
        )
        .expect("write initializer actmap");

        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 1, "InitializeDef's CreateObject is committed");
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_section")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Nil),
            "a full scenario load passes a null section name"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_id")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::C4Id("INIT".to_string())),
            "GetID falls back to the no-object definition context"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_value")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(23))
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_action_length")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(7))
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("scenario_saw_init_def")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(1)),
            "scenario Initialize observes the completed definition phase"
        );
    }

    #[test]
    fn definition_initialize_uses_numeric_c4id_order() {
        // C4ID is a little-endian integer: ZAAA sorts before ABBB even
        // though lexical ordering says the opposite (C4DefList::SortByID).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "ZAAA",
                "static init_def_order;\n\
                 func InitializeDef() {\n\
                     if (!init_def_order) init_def_order = 0;\n\
                     init_def_order = init_def_order * 10 + 1;\n\
                 }\n",
            )),
            "// no scenario script\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/DefCore.txt"),
            "[DefCore]\nid=ABBB\nName=ABBB\nCategory=0\n",
        )
        .expect("write ABBB defcore");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "static init_def_order;\n\
             func InitializeDef() {\n\
                 if (!init_def_order) init_def_order = 0;\n\
                 init_def_order = init_def_order * 10 + 2;\n\
             }\n",
        )
        .expect("write ABBB script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_order")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(12))
        );
    }

    #[test]
    fn deferred_apply_synchronizes_before_initialize_and_player_join() {
        // InitPlayers only enqueues startup joins; InitGameFinal calls
        // Script.Initialize before the first control execution performs them
        // (C4Game.cpp:456-483, C4PlayerInfo.cpp:1292-1320). Initialize also
        // sees the RNG re-fixed by Synchronize, not the weather-init ledger.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static observed_players;\n\
             static observed_random;\n\
             global func Initialize() {\n\
                 observed_players = GetPlayerCount();\n\
                 observed_random = Random(100);\n\
                 return 1;\n\
             }\n",
        );
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);

        scenario
            .apply_before_players(&mut engine)
            .expect("pre-player apply succeeds");
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_players")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Nil),
            "Script.Initialize remains pending during the pre-player phase"
        );

        let mut synchronized_rng = engine.rng.clone();
        let expected_random = synchronized_rng.random(100);
        engine
            .initialize_scenario_script()
            .expect("scenario Initialize succeeds");
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_players")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(0)),
            "Initialize runs before queued startup-player joins"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_random")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(expected_random)),
            "Initialize consumes the synchronized RNG stream"
        );
        assert_eq!(engine.rng, synchronized_rng);

        assert_eq!(join_test_player(&mut engine).len(), 1);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_players")
                .map(|cell| cell.borrow().clone()),
            Some(lc_script::Value::Int(0)),
            "the later player join does not retroactively rerun Initialize"
        );
    }

    #[test]
    fn scenario_initialize_may_return_an_int_like_real_content() {
        // C++ discards scenario-callback return values (Game.Script calls
        // run as bare statements): real scenarios `return(1)` from
        // Initialize, which must not abort the apply (two sweep scenarios
        // regressed on this once their Initialize ran to completion).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) { return 1; }\n",
        );
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(engine.scenario_script.is_some());
    }

    #[test]
    fn scenario_script_parse_errors_are_logged_not_fatal_like_cpp() {
        // A scenario Script.c that fails to compile logs the parse error and
        // the scenario runs without a script (C4ScriptHost load behavior).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "global func {{{ broken\n");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.scenario_script.is_none(),
            "no scenario script installed when it cannot compile"
        );
        assert_eq!(
            join_test_player(&mut engine).len(),
            1,
            "the scenario still runs without its script"
        );
    }

    #[test]
    fn loads_legacy_scenario_with_definitions() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let foo_core = defs_root.join("Foo.c4d");
        std::fs::create_dir_all(&foo_core).expect("definition dir");
        std::fs::write(
            foo_core.join("DefCore.txt"),
            "[DefCore]\nid=FOOO\nName=Foo\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(foo_core.join("Script.c"), "// empty definition script\n")
            .expect("write definition script");

        assert!(foo_core.join("DefCore.txt").exists(), "defcore exists");
        assert!(foo_core.join("Script.c").exists(), "script exists");

        let foo_group = Group::open(&foo_core).expect("open foo definition group");
        ResourceDefinitionData::load(&foo_group).expect("load foo definition");

        let scenario_dir = dir.path().join("LegacyScenario.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Test\nForcedAutoStopControl=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=Foo=2\nPosition=120,160\n",
        )
        .expect("write legacy scenario core");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "global func Initialize(state, random) { return nil; }\n",
        )
        .expect("write legacy scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario_group = Group::open(&scenario_dir).expect("open scenario group");
        resolver
            .resolve_definition_groups(&scenario_group, "Defs.c4d")
            .expect("resolve definition root");

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        assert_eq!(scenario.name(), Some("Legacy Test"));
        // C4SHead::ForcedControlStyle is retained past scenario parsing for
        // C4Player::ApplyForcedControl at join (C4Player.cpp:2369-2389).
        assert_eq!(scenario.forced_control_style(), Some(true));

        let mut engine = Engine::with_seed(0);
        let created = scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
        assert_eq!(created.len(), 0, "crew joins with the player, not at load");
        // The `[Player1] Crew=Foo=2` list places at JOIN
        // (C4Player::PlaceReadyCrew new spec, C4Player.cpp:528-570); the
        // exact placement positions are pinned by the draw-ledger test.
        let joined = join_test_player(&mut engine);
        assert_eq!(joined.len(), 2, "two ready-crew members at join");
        assert!(
            engine.player(0).expect("joined player").control_style(),
            "ForcedAutoStopControl=1 overrides the player's classic preference"
        );
        for id in &joined {
            let object = engine.object_snapshot(*id).expect("spawned object present");
            assert_eq!(object.definition_id, "FOOO");
            assert_eq!(object.owner, 0);
            assert!(
                object.crew_member,
                "legacy crew should be marked as crew member"
            );
        }
        let snapshot = engine.snapshot();
        assert!(
            snapshot.definition_categories.contains_key("FOOO"),
            "expected legacy definition to be registered"
        );

        let id = engine
            .spawn_object(SpawnConfig::new("FOOO"))
            .expect("spawn legacy definition");
        let object = engine
            .object_snapshot(id)
            .expect("object created from legacy definition");
        assert_eq!(object.definition_id, "FOOO");
    }

    /// Minimal legacy scenario fixture: one definition pack plus a
    /// `Scenario.txt` with the given `[Landscape]` section body.
    fn write_legacy_sky_fixture(dir: &Path, landscape_section: &str) -> PathBuf {
        let defs_root = dir.join("Defs.c4d");
        let foo_core = defs_root.join("Foo.c4d");
        std::fs::create_dir_all(&foo_core).expect("definition dir");
        std::fs::write(
            foo_core.join("DefCore.txt"),
            "[DefCore]\nid=FOOO\nName=Foo\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");

        let scenario_dir = dir.join("SkyTest.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            format!(
                "[Head]\nTitle=Sky Test\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\n{landscape_section}"
            ),
        )
        .expect("write legacy scenario core");
        scenario_dir
    }

    fn load_legacy_sky(dir: &Path, scenario_dir: &Path) -> Scenario {
        let resolver = FileSystemResolver {
            roots: vec![dir.to_path_buf()],
        };
        Scenario::load_from_path_with(scenario_dir, &resolver).expect("legacy scenario loads")
    }

    #[test]
    fn legacy_scenario_loads_sky_bitmap_like_c4sky_init() {
        // C4Sky::Init loads the scenario's `Sky` bitmap via LoadAny
        // (C4Sky.cpp:82-84; extension search png/bmp/jpeg/jpg,
        // C4Surface.cpp:855), sets the fade colors to white (C4Sky.cpp:109),
        // tiles tiny surfaces up to 128x128 (SurfaceEnsureSize,
        // C4Sky.cpp:28-52,110-111) and maps SkyScrollMode=2 to
        // ParX=ParY=20 without touching ParallaxMode (C4Sky.cpp:122-124).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "SkyScrollMode=2\n");

        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[80u8, 120, 200, 255].repeat(16), 4, 4, ColorType::Rgba8)
            .expect("encode sky jpeg");
        std::fs::write(scenario_dir.join("Sky.jpg"), jpeg).expect("write Sky.jpg");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert!(sky.settings.has_surface);
        assert_eq!((sky.settings.width, sky.settings.height), (128, 128));
        let surface = sky.surface.as_ref().expect("sky surface loaded");
        assert_eq!((surface.width(), surface.height()), (128, 128));
        assert_eq!(sky.settings.fade_top, RgbColor::new(255, 255, 255));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(255, 255, 255));
        assert_eq!(sky.settings.parallax_x, 20);
        assert_eq!(sky.settings.parallax_y, 20);
        assert_eq!(sky.settings.parallax_mode, SkyParallaxMode::Fixed);
    }

    #[test]
    fn legacy_scenario_without_sky_bitmap_falls_back_to_palette_fade() {
        // No sky bitmap: C4Sky::Init drops the surface and takes the fade
        // colors from SkyDefFade (C4Sky.cpp:129-134); an all-zero fade
        // selects game palette entries 104 and 123 — RGB(28,64,152) and
        // RGB(192,196,252) after the <<2 load scaling (C4Sky.cpp:56-62,
        // C4GraphicsResource.cpp:183-184, planet/Graphics.c4g/C4.PAL).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "TopOpen=1\n");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert!(!sky.settings.has_surface);
        assert!(sky.surface.is_none());
        assert_eq!(sky.settings.fade_top, RgbColor::new(28, 64, 152));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(192, 196, 252));
        assert_eq!(sky.settings.parallax_x, 10);
        assert_eq!(sky.settings.parallax_y, 10);
    }

    #[test]
    fn legacy_sky_def_fade_overrides_the_palette_default() {
        // Non-zero SkyDefFade — the `SkyFade` key (C4Scenario.cpp:344) —
        // is two explicit RGB triplets (C4Sky::SetFadePalette,
        // C4Sky.cpp:63-68).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "SkyFade=1,2,3,4,5,6\n");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert_eq!(sky.settings.fade_top, RgbColor::new(1, 2, 3));
        assert_eq!(sky.settings.fade_bottom, RgbColor::new(4, 5, 6));
    }

    #[test]
    fn legacy_sky_scroll_mode_wind_maps_to_wind_parallax() {
        // SkyScrollMode=1: wind-driven xdir plus ParY=20 (C4Sky.cpp:118-121).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_legacy_sky_fixture(dir.path(), "SkyScrollMode=1\n");

        let mut png = Vec::new();
        {
            use image::ImageEncoder as _;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&[10u8, 20, 30, 255], 1, 1, ColorType::Rgba8)
                .expect("encode sky png");
        }
        std::fs::write(scenario_dir.join("Sky.png"), png).expect("write Sky.png");

        let scenario = load_legacy_sky(dir.path(), &scenario_dir);
        let sky = scenario.sky().expect("legacy sky config present");
        assert_eq!(sky.settings.parallax_mode, SkyParallaxMode::Wind);
        assert_eq!(sky.settings.parallax_x, 10);
        assert_eq!(sky.settings.parallax_y, 20);
        // 1x1 tile enlarged to the 128x128 minimum (C4Sky.cpp:110-111).
        assert_eq!((sky.settings.width, sky.settings.height), (128, 128));
    }

    #[test]
    fn legacy_skipdefs_excludes_specified_definitions() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let foo_core = defs_root.join("Foo.c4d");
        std::fs::create_dir_all(&foo_core).expect("foo definition dir");
        std::fs::write(
            foo_core.join("DefCore.txt"),
            "[DefCore]\nid=FOOO\nName=Foo\nCategory=0\nCrewMember=0\n",
        )
        .expect("write foo defcore");
        std::fs::write(foo_core.join("Script.c"), "// foo script\n").expect("write foo script");

        let bar_core = defs_root.join("Bar.c4d");
        std::fs::create_dir_all(&bar_core).expect("bar definition dir");
        std::fs::write(
            bar_core.join("DefCore.txt"),
            "[DefCore]\nid=BARR\nName=Bar\nCategory=0\nCrewMember=0\n",
        )
        .expect("write bar defcore");
        std::fs::write(bar_core.join("Script.c"), "// bar script\n").expect("write bar script");

        let scenario_dir = dir.path().join("SkipDefsScenario.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=SkipDefs\n\n[Definitions]\nDefinition1=Defs.c4d\nSkipDefs=FOOO\n\n[Player1]\nCrew=BARR\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "global func Initialize(state, random) { return nil; }\n",
        )
        .expect("write scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let ids: Vec<String> = scenario
            .definitions
            .iter()
            .map(|def| def.id.clone())
            .collect();
        assert!(
            ids.iter().any(|id| id == "BARR"),
            "expected non-skipped definition to be present"
        );
        assert!(
            !ids.iter().any(|id| id == "FOOO"),
            "expected skipped definition to be filtered out"
        );
    }

    #[test]
    fn parse_c4fixed_reads_the_cpp_serialization_formats() {
        // CompileFunc(C4Fixed&) (Fixed.h:247-266): prefix 'f' = the int32
        // is FLOAT BITS run through FLOAT_TO_FIXED (ftofix truncates);
        // 'F' or no prefix = the raw fixed-point value. GoldRush saves
        // YDir=f1067030938 (bits of 1.2f) on its hanging stalactites.
        assert_eq!(
            parse_c4fixed("f1067030938").expect("parses").val(),
            78643, // trunc(1.2 * 65536)
        );
        assert_eq!(
            parse_c4fixed("f-1063256064").expect("parses").val(),
            -5 * 65536, // bits of -5.0f as negative int32
        );
        assert_eq!(parse_c4fixed("F78643").expect("parses").val(), 78643);
        assert_eq!(parse_c4fixed("123").expect("parses").val(), 123);
    }

    #[test]
    fn legacy_object_name_decodes_cpp_escaped_strings() {
        // StdCompilerINIRead::ReadEscapedChar (StdCompiler.cpp:1006-1062).
        assert_eq!(
            parse_legacy_object_name(r#""Script \"Wipf\" \\ \101\x42""#, 7)
                .expect("escaped name parses")
                .as_deref(),
            Some("Script \"Wipf\" \\ AB")
        );
    }

    #[test]
    fn loads_legacy_objects_txt_spawns_initial_objects() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let box_core = defs_root.join("Box.c4d");
        std::fs::create_dir_all(&box_core).expect("box definition dir");
        std::fs::write(
            box_core.join("DefCore.txt"),
            "[DefCore]\nid=BOX1\nName=Box\nCategory=0\nCrewMember=0\n",
        )
        .expect("write box defcore");
        std::fs::write(box_core.join("Script.c"), "// box script\n").expect("box script");

        let gem_core = defs_root.join("Gem.c4d");
        std::fs::create_dir_all(&gem_core).expect("gem definition dir");
        std::fs::write(
            gem_core.join("DefCore.txt"),
            "[DefCore]\nid=GEM1\nName=Gem\nCategory=0\nCrewMember=0\n",
        )
        .expect("write gem defcore");
        std::fs::write(gem_core.join("Script.c"), "// gem script\n").expect("gem script");

        let scenario_dir = dir.path().join("LegacyObjects.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Objects\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            // XDir/YDir are float-bit C4Fixed like real saves write them
            // (Fixed.h:247-266): f-1063256064 = -5.0, f1077936128 = 3.0.
            "[Object]\nid=BOX1\nNumber=100\nName=Scroll: Alchemist's bag\nStatus=1\nCategory=0\nOwner=1\nController=2\nX=10\nY=20\nContents=101\n\n[Object]\nid=GEM1\nNumber=101\nName=\"ScriptWipf\"\nStatus=1\nCategory=0\nLayer=100\nBlitMode=132\nX=30\nY=40\nXDir=f-1063256064\nYDir=f1077936128\nEnergy=77\nNeedEnergy=1\nSelected=1\nMagicEnergy=192000\nAlive=false\nDir=1\nComDir=3\nAction=Idle\nActionTime=6\nPhase=2\nActionData=5\nActionTarget1=100\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");

        assert_eq!(scenario.initial_spawns.len(), 2);

        let first = &scenario.initial_spawns[0];
        assert_eq!(first.handle.as_deref(), Some("100"));
        assert!(first.container_handle.is_none());
        assert_eq!(first.config.definition_id, "BOX1");
        assert_eq!(first.config.owner, 1);
        // Controller compiles verbatim (C4Object.cpp:2739).
        assert_eq!(first.config.controller, Some(2));
        assert_eq!(first.config.position, Vector2::new(10, 20));
        assert_eq!(first.config.id, Some(ObjectId::new(100)));
        assert_eq!(
            first.config.custom_name.as_deref(),
            Some("Scroll: Alchemist's bag")
        );

        let second = &scenario.initial_spawns[1];
        assert_eq!(second.handle.as_deref(), Some("101"));
        assert_eq!(second.container_handle.as_deref(), Some("100"));
        assert_eq!(second.config.definition_id, "GEM1");
        assert_eq!(second.config.custom_name.as_deref(), Some("ScriptWipf"));
        assert_eq!(second.config.layer, Some(ObjectId::new(100)));
        assert_eq!(second.config.blit_mode, Some(132));
        assert_eq!(second.config.position, Vector2::new(30, 40));
        assert_eq!(second.config.velocity, Vector2::new(-5, 3));
        assert_eq!(second.config.energy, Some(77));
        assert_eq!(second.config.need_energy, Some(true));
        assert_eq!(second.config.selected, Some(true));
        // MagicEnergy compiles verbatim (C4Object.cpp:2768) — the
        // Drachenfels wizards carry six-digit stores.
        assert_eq!(second.config.magic_energy, Some(192_000));
        assert_eq!(second.config.alive, Some(false));
        assert_eq!(second.config.category, Some(0));
        assert_eq!(second.config.direction, Direction::Right);
        assert_eq!(second.config.command_direction, CommandDirection::Right);
        let action = second.config.action.as_ref().expect("action state present");
        assert_eq!(action.name, "Idle");
        // ActionTime= is Action.Time (C4Object.cpp:2745), not the
        // intra-phase PhaseDelay counter.
        assert_eq!(action.time, 6);
        assert_eq!(action.ticks, 0);
        assert_eq!(action.phase, 2);
        assert_eq!(action.data, 5);
        assert_eq!(action.target, Some(ObjectId::new(100)));

        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");

        let box_snapshot = engine
            .object_snapshot(ObjectId::new(100))
            .expect("box object");
        assert_eq!(box_snapshot.definition_id, "BOX1");
        assert_eq!(box_snapshot.owner, 1);
        assert_eq!(box_snapshot.controller, 2, "loaded Controller= sticks");
        assert_eq!(box_snapshot.position, Vector2::new(10, 20));
        assert_eq!(
            box_snapshot.custom_name.as_deref(),
            Some("Scroll: Alchemist's bag")
        );

        let gem_snapshot = engine
            .object_snapshot(ObjectId::new(101))
            .expect("gem object");
        assert_eq!(gem_snapshot.definition_id, "GEM1");
        assert_eq!(gem_snapshot.custom_name.as_deref(), Some("ScriptWipf"));
        assert_eq!(gem_snapshot.layer, Some(ObjectId::new(100)));
        assert_eq!(gem_snapshot.blit_mode, 132);
        assert_eq!(gem_snapshot.position, Vector2::new(30, 40));
        // Loads denumerate Contained without the Enter transfer
        // (C4Object.cpp:1582 never runs): compile default NO_OWNER.
        assert_eq!(gem_snapshot.controller, crate::OWNER_NONE);
        assert_eq!(gem_snapshot.velocity, Vector2::new(-5, 3));
        assert_eq!(gem_snapshot.energy, 77);
        assert!(gem_snapshot.need_energy);
        assert!(gem_snapshot.selected);
        assert_eq!(gem_snapshot.magic_energy, 192_000);
        assert!(!gem_snapshot.alive);
        assert_eq!(gem_snapshot.container, Some(ObjectId::new(100)));
        assert_eq!(gem_snapshot.direction, Direction::Right);
        assert_eq!(gem_snapshot.command_direction, CommandDirection::Right);
        assert_eq!(gem_snapshot.action.name, "Idle");
        // ActIdle carries no phase or time: SetActionByName("Idle") clears
        // the action at load (C4Object.cpp:4214-4215, 2840-2849) — the
        // saved ActionTime=6/Phase=2 do NOT survive on an idle object.
        assert_eq!(gem_snapshot.action.ticks, 0);
        assert_eq!(gem_snapshot.action.phase, 0);
        assert_eq!(gem_snapshot.action.data, 5);
        assert_eq!(gem_snapshot.action.target, Some(ObjectId::new(100)));
    }

    #[test]
    fn scenario_initialize_finds_and_removes_placed_objects_like_cpp() {
        // GoldRush's DoInitialize culls placed editor leftovers:
        //   if(FindObject(_ETG)) RemoveObject(FindObject(_ETG));
        // (Goldrush.c4s/Script.c:28) and re-runs the placed cannon's
        // Initialize via FindObject(CCAN). The scenario script must see
        // Objects.txt placements through FindObject.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let box_core = defs_root.join("Box.c4d");
        std::fs::create_dir_all(&box_core).expect("box definition dir");
        std::fs::write(
            box_core.join("DefCore.txt"),
            "[DefCore]\nid=BOX1\nName=Box\nCategory=0\nCrewMember=0\n",
        )
        .expect("write box defcore");
        std::fs::write(box_core.join("Script.c"), "// box\n").expect("box script");

        let scenario_dir = dir.path().join("LegacyObjects.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Objects\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=BOX1\nNumber=100\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nprotected func InitializePlayer(int iPlr) {\n\
                 if(FindObject(BOX1)) RemoveObject(FindObject(BOX1));\n\
                 return 1;\n\
             }\n",
        )
        .expect("write scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
        join_test_player(&mut engine);

        // AssignRemoval clears Status immediately (C4Object.cpp); the
        // carcass is purged at frame end.
        let count = engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| {
                object.definition_id == "BOX1" && object.status != ObjectStatus::Deleted
            })
            .count();
        assert_eq!(
            count, 0,
            "the scenario script's FindObject saw the placed object and removed it"
        );
    }

    #[test]
    fn objects_txt_placements_do_not_fire_construction_callbacks_like_cpp() {
        // C4GameObjects::Load (C4GameObjects.cpp:535-618) only compiles the
        // entries and denumerates pointers — Construction/Initialize fire
        // for NEW objects only (C4Object::Init). GoldRush depends on this:
        // its placed Cauldrons would otherwise create fresh CampFires and
        // its placed Bubbles would Remove() themselves at load.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let box_core = defs_root.join("Box.c4d");
        std::fs::create_dir_all(&box_core).expect("box definition dir");
        std::fs::write(
            box_core.join("DefCore.txt"),
            "[DefCore]\nid=BOX1\nName=Box\nCategory=0\nCrewMember=0\n",
        )
        .expect("write box defcore");
        std::fs::write(
            box_core.join("Script.c"),
            "#strict\nlocal iMark;\n\
             protected func Construction() { iMark = 1; }\n\
             protected func Initialize() { iMark = 2; CreateObject(GEM1, 5, 5, -1); }\n",
        )
        .expect("box script");

        let gem_core = defs_root.join("Gem.c4d");
        std::fs::create_dir_all(&gem_core).expect("gem definition dir");
        std::fs::write(
            gem_core.join("DefCore.txt"),
            "[DefCore]\nid=GEM1\nName=Gem\nCategory=0\nCrewMember=0\n",
        )
        .expect("write gem defcore");
        std::fs::write(gem_core.join("Script.c"), "// gem script\n").expect("gem script");

        let scenario_dir = dir.path().join("LegacyObjects.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Objects\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=BOX1\nNumber=100\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");

        let snapshot = engine.snapshot();
        let placed = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "BOX1")
            .expect("placed object exists");
        assert!(
            matches!(
                placed.local_vars.get("iMark"),
                None | Some(&lc_script::Value::Nil)
            ),
            "neither Construction nor Initialize ran for the loaded object \
             (got {:?})",
            placed.local_vars.get("iMark")
        );
        assert!(
            !snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GEM1"),
            "Initialize side effects (CreateObject) must not happen at load"
        );
    }

    /// A minimal uncompressed bottom-up 8-bit BMP from top-down rows.
    fn encode_indexed_bmp(rows: &[&[u8]]) -> Vec<u8> {
        let height = rows.len() as u32;
        let width = rows[0].len() as u32;
        let stride = ((width as usize) + 3) & !3;
        let data_offset = 14 + 40 + 256 * 4;
        let file_size = data_offset + stride * height as usize;
        let mut bytes = Vec::with_capacity(file_size);
        bytes.extend_from_slice(b"BM");
        bytes.extend_from_slice(&(file_size as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data_offset as u32).to_le_bytes());
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(&(width as i32).to_le_bytes());
        bytes.extend_from_slice(&(height as i32).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        for _ in 0..4 {
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes.extend_from_slice(&256u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.resize(data_offset, 0);
        for row in rows.iter().rev() {
            bytes.extend_from_slice(row);
            bytes.resize(bytes.len() + (stride - row.len()), 0);
        }
        bytes
    }

    #[test]
    fn in_liquid_is_the_cached_object_flag_like_cpp() {
        // C4Object::InLiquid is a CACHED flag: loaded from Objects.txt
        // (default false, C4Object.cpp:2775), updated only inside movement
        // (DoMovement, C4Movement.cpp:443-460) — FnInLiquid reads the flag,
        // never the landscape (C4Script.cpp:1864-1868). A freshly loaded
        // object in water therefore reads InLiquid()==false until its
        // first movement frame, and a stale loaded flag on dry land clears
        // on the first frame.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(
            good.join("Script.c"),
            "#strict\nlocal iWet;\npublic func Probe() { iWet = InLiquid(); return 1; }\n",
        )
        .expect("write script");

        let scenario_dir = dir.path().join("Liquid.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Liquid\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 30, 30, 0],
                &[0, 20, 20, 0],
                &[30, 20, 20, 0],
                &[30, 30, 30, 0],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(
            materials.join("TexMap.txt"),
            "20=Water-Liquid\n30=Earth-Smooth\n",
        )
        .expect("write texmap");
        std::fs::write(
            materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        )
        .expect("write water");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");
        // A sits in the cave water without the flag; B sits in dry air
        // above column 0 with a stale InLiquid=1. Category 16 (C4D_Object):
        // ExecMovement skips C4D_StaticBack objects entirely
        // (C4Movement.cpp:564), so static placements would keep their
        // loaded flag forever.
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=80\nStatus=1\nCategory=16\nX=15\nY=15\n\n\
             [Object]\nid=GOOD\nNumber=81\nStatus=1\nCategory=16\nX=5\nY=5\nInLiquid=1\n",
        )
        .expect("write objects");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nfunc Initialize() {\n\
                 var pWet;\n\
                 while(pWet = FindObject(GOOD, 0,0,0,0, 0, 0, 0, 0, pWet)) pWet->Probe();\n\
                 return 1;\n\
             }\n",
        )
        .expect("write scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let flag = |engine: &Engine, number: u64| {
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.id == ObjectId::new(number))
                .map(|object| object.in_liquid)
                .expect("object exists")
        };
        let probed = |engine: &Engine, number: u64| {
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.id == ObjectId::new(number))
                .and_then(|object| object.local_vars.get("iWet").cloned())
                .expect("probe ran")
        };

        assert!(!flag(&engine, 80), "loaded default is false even in water");
        assert!(flag(&engine, 81), "loaded InLiquid=1 sticks until movement");
        assert_eq!(
            probed(&engine, 80),
            lc_script::Value::Bool(false),
            "InLiquid() reads the stale flag, not the landscape"
        );
        assert_eq!(
            probed(&engine, 81),
            lc_script::Value::Bool(true),
            "InLiquid() reads the stale loaded flag on dry land too"
        );

        // Loaded placements rest with Mobile=false (C4Object.cpp:2772), so
        // DoMovement — and with it the InLiquid update — never runs until
        // the Tick10 gravity mobilization (C4Movement.cpp:576-587): frames
        // 1-9 keep the stale flags, frame 10 re-mobilizes with zeroed dirs,
        // and frame 11 runs the first DoMovement that refreshes the flag.
        for _ in 0..9 {
            engine.tick().expect("tick succeeds");
        }
        assert!(
            !flag(&engine, 80),
            "immobile objects keep the stale flag (C4Movement.cpp:567)"
        );
        assert!(flag(&engine, 81), "stale flag survives while demobilized");
        engine.tick().expect("mobilization tick succeeds");
        engine.tick().expect("first movement tick succeeds");
        assert!(
            flag(&engine, 80),
            "movement sets the flag in liquid (C4Movement.cpp:443-460)"
        );
        assert!(
            !flag(&engine, 81),
            "movement clears the stale flag on dry land"
        );
    }

    // Objects.txt `LocalNamed=` (C4Object.cpp:2788; C4ValueMapData::
    // CompileFunc, C4ValueMap.cpp:236-295): per-object script locals load
    // verbatim with the C4Value type-char encoding (GetC4VID,
    // C4Value.cpp:368-394) — A=any (zero data reads back nil), i=int,
    // b=bool, I=C4ID, O=enumerated object number, a[size;elems]=array with
    // trailing nils omitted. The I payloads below are verbatim C++-written
    // Dragon Rock values (C4Value.cpp:717-800; C4Id.cpp:26-45).
    #[test]
    fn objects_txt_restores_named_locals_like_cpp() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\n",
        )
        .expect("write defcore");

        let scenario_dir = dir.path().join("Locals.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Locals\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=95\nStatus=1\nX=5\nY=5\n\
             LocalNamed=10;iNum=i17,fFlag=b1,pRef=O80,junk=A0,aList=a[4;i1,i2],\
             idSpell=I1112688205,aiFirst=I1145979202,numeric=I1337,none=I0,\
             aSpells=a[3;I959858757,I1145979202]\n\n\
             [Object]\nid=GOOD\nNumber=80\nStatus=1\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let idx = engine
            .find_object_index(ObjectId::new(95))
            .expect("object exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(locals.get("iNum"), Some(&lc_script::Value::Int(17)));
        assert_eq!(locals.get("fFlag"), Some(&lc_script::Value::Bool(true)));
        assert_eq!(
            locals.get("pRef"),
            Some(&lc_script::Value::Object(80)),
            "O-typed refs resolve after every object has loaded"
        );
        assert_eq!(
            locals.get("junk"),
            Some(&lc_script::Value::Nil),
            "C4V_Any with zero data reads back nil"
        );
        assert_eq!(
            locals.get("aList"),
            Some(&lc_script::Value::Array(vec![
                lc_script::Value::Int(1),
                lc_script::Value::Int(2),
                lc_script::Value::Nil,
                lc_script::Value::Nil,
            ])),
            "arrays restore the declared size; trailing nils are omitted on write"
        );
        assert_eq!(
            locals.get("idSpell"),
            Some(&lc_script::Value::C4Id("MFRB".to_string())),
            "C++'s signed int32 C4ID payload decodes in little-endian byte order"
        );
        assert_eq!(
            locals.get("aiFirst"),
            Some(&lc_script::Value::C4Id("BAND".to_string())),
            "callback-suffix IDs survive even when they are not definitions"
        );
        assert_eq!(
            locals.get("numeric"),
            Some(&lc_script::Value::C4Id("1337".to_string())),
            "numeric C4IDs use the four-digit C4IdText form"
        );
        assert_eq!(
            locals.get("none"),
            Some(&lc_script::Value::Nil),
            "C4ID_None remains falsey instead of becoming the truthy text NONE"
        );
        assert_eq!(
            locals.get("aSpells"),
            Some(&lc_script::Value::Array(vec![
                lc_script::Value::C4Id("EH69".to_string()),
                lc_script::Value::C4Id("BAND".to_string()),
                lc_script::Value::Nil,
            ])),
            "nested C4IDs restore through the recursive array decoder"
        );
    }

    // C4Value::DenumeratePointer subtracts the legacy enumeration offset,
    // searches both active and inactive object lists, and clears a missing
    // object (C4Value.cpp:693-713; C4ObjectList.h:32-34). It recurses through
    // containers, and C4Object applies it to LocalNamed after every object is
    // loaded (C4Value.cpp:686-690; C4Object.cpp:2914-2923).
    // Serialized strings resolve by the IDs loaded from Strings.txt; duplicate
    // text reuses the existing C4String and moves its enum ID to the later line
    // (C4StringTable.cpp:201-216; C4Value.cpp:783-798).
    #[test]
    fn objects_txt_denumerates_named_local_identities_like_cpp() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\n",
        )
        .expect("write defcore");

        let scenario_dir = dir.path().join("Identities.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Identities\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Strings.txt"),
            b"first\r\nM\xfcnkelburg\r\nsame\r\nsame\r\n",
        )
        .expect("write string table");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\n\
             LocalNamed=9;pOffset=O1000000419,pPlain=O419,pMissing=O1000000999,\
             aRefs=a[3;O1000000419,O1000000999],sFirst=S0,sUmlaut=S1,\
             sOldDuplicate=S2,sDuplicate=S3,sMissing=S7\n\n\
             [Object]\nid=GOOD\nNumber=419\nStatus=2\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let holder = engine
            .object_snapshot(ObjectId::new(10))
            .expect("holder exists");
        let locals = &holder.local_vars;
        assert_eq!(
            locals.get("pOffset"),
            Some(&lc_script::Value::Object(419)),
            "C4EnumPointer1 is removed before lookup"
        );
        assert_eq!(
            locals.get("pPlain"),
            Some(&lc_script::Value::Object(419)),
            "unoffset enum values resolve too"
        );
        assert_eq!(locals.get("pMissing"), Some(&lc_script::Value::Nil));
        assert_eq!(
            locals.get("aRefs"),
            Some(&lc_script::Value::Array(vec![
                lc_script::Value::Object(419),
                lc_script::Value::Nil,
                lc_script::Value::Nil,
            ])),
            "container denumeration is recursive"
        );
        assert_eq!(
            locals.get("sFirst"),
            Some(&lc_script::Value::String("first".to_string()))
        );
        assert_eq!(
            locals.get("sUmlaut"),
            Some(&lc_script::Value::String("Münkelburg".to_string()))
        );
        assert_eq!(
            locals.get("sOldDuplicate"),
            Some(&lc_script::Value::Nil),
            "a duplicate string moves the shared C4String to the later enum ID"
        );
        assert_eq!(
            locals.get("sDuplicate"),
            Some(&lc_script::Value::String("same".to_string()))
        );
        assert_eq!(locals.get("sMissing"), Some(&lc_script::Value::Nil));

        let persisted = serde_json::to_string(&holder).expect("snapshot serializes");
        let restored: crate::ObjectSnapshot =
            serde_json::from_str(&persisted).expect("snapshot restores");
        assert_eq!(restored.local_vars, holder.local_vars);
    }

    // C4Weather::Init at scenario start draws from the SYNCED ledger
    // (C4Weather.cpp:36-70): Season, YearSpeed, Climate, Wind (the value
    // trees read through GetWind), the NoInitialize-gated rain block, then
    // Lightning and the Disasters. Every C4SVal::Evaluate draws
    // Random(2*Rnd+1) even for Rnd=0 (C4Scenario.cpp:43-46), so the whole
    // RNG stream shifts if any draw is skipped.
    #[test]
    fn weather_init_retains_the_precipitation_material_name() {
        // C4SWeather::CompileFunc stores `Precipitation` and
        // C4Weather::Init passes that exact name to LaunchCloud
        // (C4Scenario.cpp:390; C4Weather.cpp:55-57,205-211).
        let manifest = parse_legacy_scenario_text(
            "[Head]\nTitle=Rain\n\n[Weather]\nPrecipitation=AcidRain\n",
        )
        .expect("scenario parses");

        let weather = derive_legacy_weather_init(&manifest).expect("weather derives");
        assert_eq!(weather.precipitation, "AcidRain");
    }

    #[test]
    fn scenario_apply_replays_the_weather_init_ledger_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\n",
        )
        .expect("write defcore");

        let scenario_dir = dir.path().join("Windy.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        // GoldRush-shaped: NoInitialize=1 skips the rain block entirely
        // (C4Weather.cpp:49) — 8 draws total.
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Windy\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Weather]\nClimate=10,0\nStartSeason=44,0\nYearSpeed=0\nWind=0,75\n",
        )
        .expect("write scenario core");

        let mut engine = Engine::with_seed(7);
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        scenario.apply(&mut engine).expect("scenario applies");

        // Replay the exact C++ draw order on a twin RNG.
        let mut replay = crate::rng::LcgRng::seed_from_u64(7);
        // Landscape.ScenarioInit's Gravity draw precedes the weather
        // evaluates (C4Landscape.cpp:66).
        let _gravity = LegacyC4SVal::new(100, 0, 10, 200).evaluate(&mut replay);
        let season = LegacyC4SVal::new(44, 0, 0, 100).evaluate(&mut replay);
        let year_speed = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let climate = 100 - LegacyC4SVal::new(10, 0, 0, 100).evaluate(&mut replay) - 50;
        let wind = LegacyC4SVal::new(0, 75, -100, 100).evaluate(&mut replay);
        let lightning = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let meteorite = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let volcano = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let earthquake = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);

        let environment = engine.environment();
        assert_eq!(environment.season, season.clamp(0, 100));
        assert_eq!(environment.year_speed, year_speed);
        assert_eq!(environment.climate, climate);
        assert_eq!(
            (environment.wind, environment.wind_target),
            (wind, wind),
            "Wind = TargetWind = Wind.Evaluate (C4Weather.cpp:47)"
        );
        assert_eq!(environment.lightning, lightning);
        assert_eq!(environment.meteorite, meteorite);
        assert_eq!(environment.volcano, volcano);
        assert_eq!(environment.earthquake, earthquake);

        // After the draws, C4Game::Synchronize re-fixes the ledger
        // (C4Game.cpp:474,3695): the post-apply position is a FRESH
        // FixRandom(seed) stream, not the post-draw position.
        let mut fresh = crate::rng::LcgRng::seed_from_u64(7);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            fresh.random(1_000_000),
            "game-start Synchronize re-fixes the ledger after the weather draws"
        );
    }

    // Objects.txt serializes the CURRENT shape per object (C4Shape::
    // CompileFunc into the [Object] section, C4Shape.cpp:495-515):
    // Vertices/VertexX/VertexY/VertexCNAT/VertexFriction load VERBATIM —
    // they are the post-Con/rotation effective shape, not a base to
    // re-transform. C++ keeps them until the next UpdateShape (which
    // recomputes from the def), so resting objects keep saved overrides
    // like VertexFriction=50 indefinitely.
    #[test]
    fn objects_txt_restores_saved_shape_vertices_verbatim_like_cpp() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        // The def's own shape differs from the saved one: 1 vertex,
        // friction 30 — the 30-vs-50 live-diff class.
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\nRotate=1\n\
             Vertices=1\nVertexX=0\nVertexY=0\nVertexFriction=30\n",
        )
        .expect("write defcore");

        let scenario_dir = dir.path().join("Verts.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Verts\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        // 90: plain saved shape (3 vertices, friction 50) — verbatim.
        // 91: ROTATED object — the saved vertices are already rotated;
        //     applying the spawn rotation again would double-rotate.
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=90\nStatus=1\nX=10\nY=10\n\
             Vertices=3\nVertexX=2,-14,14\nVertexY=11,-4,-4\n\
             VertexCNAT=8,1,2\nVertexFriction=50,50,50\n\n\
             [Object]\nid=GOOD\nNumber=91\nStatus=1\nX=30\nY=10\nRotation=90\n\
             Vertices=1\nVertexX=-11\nVertexY=2\nVertexFriction=50\n\n\
             [Object]\nid=GOOD\nNumber=92\nStatus=1\nX=40\nY=10\nRotation=-9\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let idx = engine
            .find_object_index(ObjectId::new(90))
            .expect("object 90 exists");
        let vertices = &engine.objects[idx].state.vertices;
        assert_eq!(vertices.len(), 3, "saved Vertices= count wins over the def");
        assert_eq!(
            (
                vertices[0].x,
                vertices[0].y,
                vertices[0].cnat,
                vertices[0].friction
            ),
            (2, 11, 8, 50),
            "saved vertex 0 loads verbatim incl. CNAT and friction"
        );
        assert_eq!(
            (vertices[1].x, vertices[1].y, vertices[1].friction),
            (-14, -4, 50)
        );

        let idx = engine
            .find_object_index(ObjectId::new(91))
            .expect("object 91 exists");
        let vertices = &engine.objects[idx].state.vertices;
        assert_eq!(
            (vertices[0].x, vertices[0].y),
            (-11, 2),
            "saved vertices are the ALREADY-rotated shape — no re-rotation at load"
        );
        assert_eq!(engine.objects[idx].state.rotation, 90);
        let idx = engine
            .find_object_index(ObjectId::new(92))
            .expect("object 92 exists");
        assert_eq!(engine.objects[idx].state.rotation, -9);
        assert_eq!(engine.objects[idx].fixed_rotation, crate::itofix(-9));
    }

    // C4Game::InitGame environment placements (C4Game.cpp:2493-2503):
    // scenarios without NoInitialize place [Landscape] Vegetation= on
    // surface soil (PlaceVegetation, C4Game.cpp:2962-3007), InEarth= into
    // earth pixels (PlaceInEarth, C4Game.cpp:2949-2960) and create the
    // [Environment] Objects= / [Game] Goals=/Rules= objects
    // (C4Game.cpp:3988-4018) — all through the synced ledger between the
    // Gravity draw and Weather.Init's.
    #[test]
    fn init_placements_populate_vegetation_inearth_and_rule_objects() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        for (folder, core) in [
            (
                "Tree.c4d",
                // Growth admits the random-con draw (C4Game.cpp:2974).
                "[DefCore]\nid=TREE\nName=Tree\nCategory=2\nWidth=8\nHeight=12\n\
                 Offset=-4,-6\nVertices=1\nVertexX=0\nVertexY=0\nGrowth=4\nPlacement=0\n",
            ),
            (
                "Rock.c4d",
                "[DefCore]\nid=ROCK\nName=Rock\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
            ),
            (
                "Goal.c4d",
                "[DefCore]\nid=GOAL\nName=Goal\nCategory=4096\n",
            ),
            (
                "Rule.c4d",
                "[DefCore]\nid=RULE\nName=Rule\nCategory=8192\n",
            ),
            (
                "Envr.c4d",
                "[DefCore]\nid=ENVR\nName=Envr\nCategory=16384\n",
            ),
        ] {
            let def_dir = defs_root.join(folder);
            std::fs::create_dir_all(&def_dir).expect("definition dir");
            std::fs::write(def_dir.join("DefCore.txt"), core).expect("write defcore");
        }

        let scenario_dir = dir.path().join("Placements.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Placements\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Game]\nGoals=GOAL=1;\nRules=RULE=1;\n\n\
             [Landscape]\nMapZoom=10\nMaterial=Earth\n\
             Vegetation=TREE=1;\nVegetationLevel=100,0\n\
             InEarth=ROCK=1;\nInEarthLevel=100,0\n\n\
             [Environment]\nObjects=ENVR=1;\n",
        )
        .expect("write scenario core");
        // 20x20 map, zoom 10 → 200x200 world: sky rows 0-9, earth 10-19
        // (surface at world y=100, inside PlaceVegetation's [50, hgt-50]).
        let mut rows: Vec<Vec<u8>> = Vec::new();
        for y in 0..20 {
            rows.push(vec![if y < 10 { 0u8 } else { 30 }; 20]);
        }
        let row_refs: Vec<&[u8]> = rows.iter().map(|row| row.as_slice()).collect();
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&row_refs),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "30=Earth-Smooth\n")
            .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        )
        .expect("write earth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let library = lc_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        )
        .expect("materials parse");
        let mut engine = Engine::with_seed(7);
        engine.configure_materials_from_library(&library);
        scenario.apply(&mut engine).expect("scenario applies");

        let count = |id: &str| {
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == id)
                .count()
        };
        // amt = (GBackWdt/50) * VegLevel/100 = 4 tries, 20 attempts each
        // over an all-soil surface: placements land.
        assert!(count("TREE") >= 1, "vegetation placed on surface soil");
        // amt = (200*200/5000) * 100/100 = 8 in-earth tries.
        assert!(count("ROCK") >= 1, "in-earth objects placed");
        assert_eq!(count("GOAL"), 1, "InitGoals creates the goal object");
        assert_eq!(count("RULE"), 1, "InitRules creates the rule object");
        assert_eq!(
            count("ENVR"),
            1,
            "InitEnvironment creates the environment object"
        );

        // Vegetation sits at the earth surface (y=100) + the 3+5 soil
        // probe offsets; in-earth rocks live inside the ground.
        let tree = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "TREE")
            .expect("tree exists");
        assert!(
            (90..=130).contains(&tree.state.position.y),
            "tree y {} anchors at the surface",
            tree.state.position.y
        );
        let rock = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "ROCK")
            .expect("rock exists");
        assert!(
            rock.state.position.y >= 100,
            "rock y {} is inside the earth",
            rock.state.position.y
        );

        // NoInitialize=1 skips the whole block (C4Game.cpp:2493).
        let scenario_txt = std::fs::read_to_string(scenario_dir.join("Scenario.txt")).unwrap();
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            scenario_txt.replace("Title=Placements", "Title=Placements\nNoInitialize=1"),
        )
        .expect("rewrite scenario core");
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario reloads");
        let mut engine = Engine::with_seed(7);
        engine.configure_materials_from_library(&library);
        scenario.apply(&mut engine).expect("scenario applies");
        let count = |id: &str| {
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == id)
                .count()
        };
        assert_eq!(count("TREE"), 0, "NoInitialize skips vegetation");
        assert_eq!(count("GOAL"), 0, "NoInitialize skips goals");
    }

    // Objects.txt Mobile/FixX/FixY/FixR/RDir ingestion
    // (C4Object.cpp:2762-2772): loaded objects keep the serialized Mobile
    // verbatim (default false) with the exact C4Fixed sub-pixel
    // position/rotation state, independent of the integer X/Y/Rotation.
    // A non-Mobile object with stale saved dirs stays frozen until the
    // Tick10 pulse wipes the dirs and re-snaps fix (C4Movement.cpp:576-587).
    #[test]
    fn objects_txt_restores_mobile_and_fixed_state_like_cpp() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\nRotate=1\n",
        )
        .expect("write defcore");

        let scenario_dir = dir.path().join("Fixed.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Fixed\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("write scenario core");
        // 40x40 world: sky everywhere, earth on the bottom row — the
        // objects at y=5 stay in free air.
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 0, 0, 0],
                &[0, 0, 0, 0],
                &[0, 0, 0, 0],
                &[30, 30, 30, 30],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "30=Earth-Smooth\n").expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");
        // 80: Mobile=1 flying right at 0.7 px/frame from x=15.25 —
        //     saved pairs keep x == fixtoi(fix_x) (round-to-nearest), so
        //     the sub-pixel stays under half. itofix(15)+0.25 = 999424;
        //     XDir 0.7 = F45875.
        // 81: Mobile absent (false) with STALE saved dirs — C++ keeps the
        //     dirs but never moves; the frame-10 pulse wipes them.
        // 82: rotating: Rotation=90, FixR = 90.25 deg (F5914624),
        //     RDir = raw 6554 (~0.1 deg/frame).
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=80\nStatus=1\nX=15\nY=5\n\
             FixX=F999424\nFixY=F327680\nXDir=F45875\nMobile=1\n\n\
             [Object]\nid=GOOD\nNumber=81\nStatus=1\nX=25\nY=5\n\
             FixX=F1654784\nFixY=F327680\nXDir=F45875\n\n\
             [Object]\nid=GOOD\nNumber=82\nStatus=1\nX=35\nY=5\n\
             Rotation=90\nFixR=F5914624\nRDir=F6554\nMobile=1\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let idx_of = |engine: &Engine, number: u64| {
            engine
                .find_object_index(ObjectId::new(number))
                .expect("object exists")
        };

        // Ingestion snapshot before any tick.
        let mover = idx_of(&engine, 80);
        assert!(engine.objects[mover].state.mobile, "Mobile=1 sticks");
        // FixX/FixY load verbatim (C4Object.cpp:2762) but the game-start
        // SyncClearance collapses them to itofix(x,y) before InitPlayers
        // (C4Object.cpp:3810, C4Game.cpp:474) — 15 px exactly.
        assert_eq!(
            engine.objects[mover].fixed_position.x.val(),
            crate::math::itofix(15).val(),
            "SyncClearance reduces FixX to itofix(x) at game start"
        );
        assert_eq!(
            engine.objects[mover].state.position,
            Vector2::new(15, 5),
            "the integer X/Y stay independent of FixX/FixY"
        );
        let frozen = idx_of(&engine, 81);
        assert!(
            !engine.objects[frozen].state.mobile,
            "Mobile default false (C4Object.cpp:2772)"
        );
        assert_eq!(
            engine.objects[frozen].state.position,
            Vector2::new(25, 5),
            "the integer X/Y stay independent of the FixX/FixY sub-pixel"
        );
        assert_eq!(
            engine.objects[frozen].fixed_velocity.x.val(),
            45_875,
            "stale saved dirs load verbatim"
        );
        let spinner = idx_of(&engine, 82);
        assert_eq!(engine.objects[spinner].state.rotation, 90);
        // fix_r also collapses to itofix(r) at game start
        // (C4Object.cpp:3811).
        assert_eq!(
            engine.objects[spinner].fixed_rotation.val(),
            crate::math::itofix(90).val(),
            "FixR restores the exact rotation accumulator (C4Object.cpp:2764)"
        );
        assert_eq!(
            engine.objects[spinner].rotation_velocity.val(),
            6_554,
            "RDir restores the angular velocity (C4Object.cpp:2767)"
        );

        // Frame 1: the Mobile mover integrates from its game-start
        // SyncClearance'd position (itofix(15) + 45875 = 1028915 -> 15.70
        // -> pixel 16, fixtoi rounds to nearest); the frozen object holds
        // position AND its stale dirs.
        engine.tick().expect("tick succeeds");
        let mover = idx_of(&engine, 80);
        assert_eq!(
            engine.objects[mover].fixed_position.x.val(),
            crate::math::itofix(15).val() + 45_875
        );
        assert_eq!(engine.objects[mover].state.position.x, 16);
        let frozen = idx_of(&engine, 81);
        assert_eq!(engine.objects[frozen].state.position.x, 25);
        assert_eq!(engine.objects[frozen].fixed_velocity.x.val(), 45_875);

        // Frames 2-9: still frozen. Frame 10: the pulse wipes the stale
        // dirs and re-snaps fix to the integer position
        // (C4Movement.cpp:581-586).
        for _ in 2..=9 {
            engine.tick().expect("tick succeeds");
        }
        let frozen = idx_of(&engine, 81);
        assert_eq!(engine.objects[frozen].fixed_velocity.x.val(), 45_875);
        engine.tick().expect("pulse tick succeeds");
        let frozen = idx_of(&engine, 81);
        assert!(engine.objects[frozen].state.mobile);
        assert_eq!(engine.objects[frozen].fixed_velocity.x.val(), 0);
        assert_eq!(
            engine.objects[frozen].fixed_position.x.val(),
            25 * 65536,
            "the pulse snaps fix_x to itofix(x), discarding the stale sub-pixel"
        );
    }

    #[test]
    fn static_map_classifies_materials_into_surface_and_liquid_columns() {
        // A static-map scenario without Map.bmp falls back to
        // Landscape.bmp (C4Landscape.cpp:593-601). Each map pixel byte is
        // a texmap index (IFT bit 0x80 stripped): index 0 = sky, else
        // TexMap.txt -> material -> density (PixCol2Mat/MatDensity,
        // C4Wrappers.h:110-145); liquid iff 25<=density<50, solid iff
        // density>=50 (C4Wrappers.h:68-81). The map zooms by MapZoom.
        // GoldRush's river bubbles depend on the liquid columns: their
        // LiquidCheck removes them when InLiquid() is false.
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");

        let scenario_dir = dir.path().join("Liquid.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Liquid\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\nLiquid=Water-Smooth\n",
        )
        .expect("write scenario core");
        // Map (4x4): the middle columns are a CAVE river — an earth roof
        // over water over an earth bed (GoldRush's bubbles live in such an
        // underground river, below the column surface). Column 0 is open
        // ground, column 3 all sky.
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 30, 30, 0],
                &[0, 20, 20, 0],
                &[30, 20, 20, 0],
                &[30, 30, 30, 0],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(
            materials.join("TexMap.txt"),
            "# table\n20=Water-Liquid\n30=Earth-Smooth\n",
        )
        .expect("write texmap");
        std::fs::write(
            materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        )
        .expect("write water");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");
        // A placed object INSIDE the pool: C4GameObjects::Load keeps
        // positions verbatim — no spawn-time surface ejection (GoldRush's
        // bubbles and fish sit in an underground river below the column
        // surface).
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=77\nStatus=1\nCategory=0\nX=15\nY=15\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let landscape = engine.landscape().expect("landscape loaded");

        let placed = engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.id == ObjectId::new(77))
            .cloned()
            .expect("placed object exists");
        assert_eq!(
            placed.position,
            Vector2::new(15, 15),
            "loaded objects keep their Objects.txt position (no surface snap)"
        );

        // Map column 1 (world x 10..20): earth roof row 0, water rows 1-2
        // (world y 10..30), earth bed row 3 (world y 30..40).
        assert!(landscape.is_liquid_at(15, 15), "river interior is liquid");
        assert!(
            landscape.is_liquid_at(15, 29),
            "river bottom edge is liquid"
        );
        assert!(!landscape.is_liquid_at(15, 5), "roof above the river");
        assert!(!landscape.is_liquid_at(15, 35), "earth bed is not liquid");
        assert!(landscape.is_solid_at(15, 35), "earth bed is solid");
        assert!(
            landscape.is_semi_solid_at(15, 15),
            "liquid counts as semi-solid (GBackSemiSolid)"
        );
        // Map column 0: earth from row 2 (world y 20).
        assert!(landscape.is_solid_at(5, 25));
        assert!(!landscape.is_liquid_at(5, 25));
        // Map column 3: all sky.
        assert!(!landscape.is_solid_at(35, 38), "sky column has no ground");
    }

    #[test]
    fn legacy_scenario_threads_border_open_keys_into_the_landscape() {
        // C4Landscape::ScenarioInit (C4Landscape.cpp:67-73) copies the
        // Scenario.txt LeftOpen/RightOpen/TopOpen/BottomOpen keys onto the
        // landscape; AutoScanSideOpen=0 keeps the explicit side values.
        let dir = tempdir().expect("tempdir");
        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");

        let scenario_dir = dir.path().join("Borders.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Borders\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\nTopOpen=0\nBottomOpen=1\nLeftOpen=7\nRightOpen=9\nAutoScanSideOpen=0\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 0, 0, 0],
                &[0, 0, 0, 0],
                &[30, 30, 30, 0],
                &[30, 30, 30, 30],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# table\n30=Earth-Smooth\n")
            .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let landscape = engine.landscape().expect("landscape loaded");
        assert_eq!(landscape.left_open(), 7);
        assert_eq!(landscape.right_open(), 9);
        assert!(!landscape.top_open());
        assert!(landscape.bottom_open());
    }

    #[test]
    fn legacy_scenario_auto_scan_side_open_scans_the_border_columns() {
        // AutoScanSideOpen defaults to true (C4Scenario.cpp:297):
        // ScanSideOpen (C4Landscape.cpp:231-238) replaces LeftOpen /
        // RightOpen with the first non-sky pixel of the border columns.
        let dir = tempdir().expect("tempdir");
        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");

        let scenario_dir = dir.path().join("Scan.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Scan\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("write scenario core");
        // Column 0 turns solid at map row 2 (world y 20); column 3 is all
        // sky (right side fully open: RightOpen = world height 40).
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 30, 30, 0],
                &[0, 30, 30, 0],
                &[30, 30, 30, 0],
                &[30, 30, 30, 0],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# table\n30=Earth-Smooth\n")
            .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let landscape = engine.landscape().expect("landscape loaded");
        assert_eq!(landscape.left_open(), 20, "first non-sky pixel in column 0");
        assert_eq!(
            landscape.right_open(),
            40,
            "all-sky border column opens the full height"
        );
    }

    #[test]
    fn get_index_mat_tex_returns_cross_mapped_default_not_first_material_slot() {
        // C4MaterialMap::CrossMapMaterials stores the exact overlay entry in
        // DefaultMatTex (C4Material.cpp:349-370), and GetIndexMatTex returns
        // that field after its explicit-texture attempts (C4Texture.cpp:
        // 346-367). An earlier Earth-Ridge slot must not replace the recorded
        // Earth-Smooth default.
        let mut names = vec![None; 128];
        names[4] = Some("Earth".to_string());
        names[30] = Some("Earth".to_string());
        let mut textures = vec![None; 128];
        textures[4] = Some("Ridge".to_string());
        textures[30] = Some("Smooth".to_string());
        let mut classifier = MapPixelClassifier::from_slots(
            [0; 128],
            names,
            textures,
            vec![None; 128],
        );
        classifier.state.set_default_material_entry("Earth", 30);

        assert_eq!(classifier.get_index_mat_tex("Earth", None), 30);
    }

    #[test]
    fn get_index_never_allocates_reserved_diff_slot_127() {
        // C4M_MaxTexIndex is 127 and index 127 is reserved for landscape
        // diffs (C4Constants.h:63). C4TextureMap::GetIndex searches and
        // allocates only `byIndex < C4M_MaxTexIndex` (C4Texture.cpp:319-340),
        // so a map whose usable slots 1..=126 are full must return 0.
        let mut names = vec![None; 128];
        for (slot, name) in names.iter_mut().enumerate().take(127).skip(1) {
            *name = Some(format!("Taken{slot}"));
        }
        let library = lc_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nShape=2\n",
        )
        .expect("material parses");
        let mut classifier = MapPixelClassifier::from_slots_with_library(
            [0; 128],
            names,
            vec![None; 128],
            vec![None; 128],
            library,
            vec!["smooth".to_string()],
        );

        assert_eq!(classifier.get_index("Earth", Some("Smooth"), true), 0);
        assert!(classifier.state.material_names[127].is_none());
    }

    #[test]
    fn keep_map_creator_persists_the_evaluated_tree_with_raster_state() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("KeepCreator.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Landscape.txt"),
            "overlay Named { seed = 7; }; map Test { seed = 11; };",
        )
        .expect("write Landscape.txt");
        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(
            "[Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=5\nKeepMapCreator=1\n",
        )
        .expect("scenario core parses");
        let mut classifier = MapPixelClassifier::from_slots(
            [0; 128],
            vec![None; 128],
            vec![None; 128],
            vec![None; 128],
        );

        let landscape = load_legacy_landscape_body(&group, &manifest, Some(&mut classifier))
            .expect("landscape loads")
            .expect("landscape exists");
        let raster = landscape.raster_state().expect("raster state retained");
        assert_eq!(raster.map_zoom(), 5);
        assert!(raster.map_creator().is_some(), "KeepMapCreator retains tree");

        let encoded = serde_json::to_string(&landscape).expect("landscape serializes");
        let restored: Landscape = serde_json::from_str(&encoded).expect("landscape restores");
        assert_eq!(restored, landscape, "creator and texmap survive saves");
    }

    #[test]
    fn classified_static_map_builds_the_per_pixel_plane_like_cpp() {
        // MapToLandscape blits each map cell at MapZoom scale into the
        // Surface8 pixel plane (C4Landscape::MapToSurface via
        // ChunkOZoom, C4Landscape.cpp:732-789); GBackSolid then reads
        // Pix2Dens per PIXEL (C4Wrappers.h:174-177), so cave water below
        // the column surface is liquid — never solid — while the earth
        // roof above it stays solid. The column approximation calls
        // everything below the first solid row "solid", which sheds
        // cave-roof objects (GoldRush's stalactites fell and shattered).
        let bitmap = lc_resources::bitmap::IndexedBitmap {
            width: 4,
            height: 4,
            indices: vec![
                0, 30, 30, 0, //
                0, 20, 20, 0, //
                30, 20, 20, 0, //
                30, 30, 30, 0,
            ],
        };
        let mut densities = [0i32; 128];
        densities[20] = 25; // Water
        densities[30] = 100; // Earth
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[20] = Some("Water".into());
        names[30] = Some("Earth".into());
        // No `Shape` in either material: MapChunkType 0 = Flat, chunks
        // box-fill their blocks (C4Landscape.cpp:285-287).
        let mut shapes: Vec<Option<crate::chunky::ChunkShape>> = vec![None; 128];
        shapes[20] = Some(crate::chunky::ChunkShape::Flat);
        shapes[30] = Some(crate::chunky::ChunkShape::Flat);
        let classifier =
            MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);

        let landscape =
            classified_landscape(&bitmap, &classifier, 10, 0).expect("landscape builds");

        let grid = landscape
            .pixel_grid()
            .expect("classified maps build Surface8");
        assert_eq!(
            grid.byte_at(15, 15),
            Some(20),
            "world pixels carry the raw map byte of their zoom block"
        );
        assert_eq!(grid.byte_at(15, 5), Some(30), "roof block is earth");
        assert_eq!(grid.byte_at(35, 35), Some(0), "sky column stays sky");

        // Map column 1 (world x 10..20): earth roof row 0, water rows 1-2,
        // earth bed row 3. Pixel truth: the water interior is NOT solid.
        assert!(
            !landscape.is_solid_at(15, 15),
            "GBackSolid is false in water (density 25 < C4M_Solid)"
        );
        assert!(landscape.is_liquid_at(15, 15), "river interior is liquid");
        assert!(landscape.is_solid_at(15, 5), "earth roof is solid");
        assert!(landscape.is_solid_at(15, 35), "earth bed is solid");
        assert!(
            !landscape.is_solid_at(35, 25),
            "sky below roof level in an open column is not solid"
        );
    }

    #[test]
    fn classified_static_map_synthesizes_chunky_borders_like_chunk_o_zoom() {
        // MapToLandscape zooms map cells through ChunkOZoom: Smooth/Rough
        // materials draw jittered chunk POLYGONS, not blocks
        // (DrawChunk, C4Landscape.cpp:280-313), so material borders
        // bulge past the zoom grid. With MapSeed=0 the cell at map (1,1)
        // (cro=5) reaches one pixel above its block at world columns
        // 5..=7 (hand-stepped in chunky::tests) — cave roofs gain the
        // overhang that keeps stalactites attached in C++.
        let bitmap = lc_resources::bitmap::IndexedBitmap {
            width: 3,
            height: 2,
            indices: vec![
                0, 0, 0, //
                30, 30, 30,
            ],
        };
        let mut densities = [0i32; 128];
        densities[30] = 100;
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[30] = Some("Earth".into());
        let mut shapes: Vec<Option<crate::chunky::ChunkShape>> = vec![None; 128];
        shapes[30] = Some(crate::chunky::ChunkShape::Smooth);
        let classifier =
            MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);

        let landscape = classified_landscape(&bitmap, &classifier, 4, 0).expect("landscape builds");

        assert!(landscape.is_solid_at(6, 3), "chunk bulges above its block");
        assert!(!landscape.is_solid_at(4, 3), "bulge is jitter-shaped");
        assert_eq!(
            landscape.surface_height(6),
            Some(3),
            "surface columns derive from the synthesized plane"
        );
        assert_eq!(landscape.surface_height(4), Some(4));
    }

    #[test]
    fn exact_landscape_bmp_loads_at_pixel_scale() {
        // ExactLandscape=1: Landscape.bmp IS the landscape — C++ reads it
        // straight into the pixel surface (GroupReadSurface8, no MapZoom).
        // The heightfield model reduces it to the column profile at zoom 1;
        // returning NO landscape here left GBackSolid answering "never
        // solid" and hung content like the grass placement loop
        // (Knights.c4f/Dunkelfels.c4s + Grass.c4d Initialize).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Exact\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=Good=1\nPosition=4,2\n\n[Landscape]\nExactLandscape=1\n",
        )
        .expect("write scenario core");
        let mut bitmap = RgbaImage::from_pixel(8, 6, Rgba([0, 0, 255, 255]));
        for y in 2..6 {
            for x in 0..8 {
                bitmap.put_pixel(x, y, Rgba([128, 64, 32, 255]));
            }
        }
        let raw = bitmap.into_raw();
        let mut encoded = Vec::new();
        {
            let mut encoder = BmpEncoder::new(&mut encoded);
            encoder
                .encode(&raw, 8, 6, ColorType::Rgba8)
                .expect("encode landscape bmp");
        }
        std::fs::write(scenario_dir.join("Landscape.bmp"), encoded).expect("write landscape");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let landscape = engine.landscape().expect("exact landscape loads");
        assert_eq!(landscape.width(), 8, "pixel scale: no MapZoom applied");
        assert_eq!(
            landscape.surface(),
            vec![2; 8].as_slice(),
            "the surface Y coordinate is the first ground row"
        );
    }

    #[test]
    fn legacy_map_bmp_creates_landscape_height_profile() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let crew_core = defs_root.join("Crew.c4d");
        std::fs::create_dir_all(&crew_core).expect("crew definition dir");
        std::fs::write(
            crew_core.join("DefCore.txt"),
            "[DefCore]\nid=CLNK\nName=Clonk\nCategory=0\nCrewMember=1\n",
        )
        .expect("write crew defcore");
        std::fs::write(crew_core.join("Script.c"), "// crew script\n").expect("crew script");

        let scenario_dir = dir.path().join("LegacyLandscape.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Landscape\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=CLNK=1\nPosition=40,60\n\n[Landscape]\nMapWidth=4\nMapHeight=4\nMapZoom=2\n",
        )
        .expect("write scenario core");

        let mut map = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 255, 255]));
        for y in 1..4 {
            for x in 0..4 {
                map.put_pixel(x, y, Rgba([128, 64, 32, 255]));
            }
        }
        let raw = map.into_raw();
        let mut encoded = Vec::new();
        {
            let mut encoder = BmpEncoder::new(&mut encoded);
            encoder
                .encode(&raw, 4, 4, ColorType::Rgba8)
                .expect("encode map bmp");
        }
        std::fs::write(scenario_dir.join("Map.bmp"), encoded).expect("write map");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let landscape = engine.landscape().expect("landscape present");
        // MapZoom=2 clamps to the C4SVal Min of 5 (C4Scenario.cpp:307,353):
        // 4 map columns × zoom 5 = 20 landscape columns; ground starts at
        // map row 1 → surface Y = 5.
        assert_eq!(landscape.width(), 20);
        assert_eq!(
            landscape.surface(),
            vec![5; 20].as_slice(),
            "the surface Y coordinate scales with the zoom"
        );
    }

    #[test]
    fn legacy_scenario_populates_physics_and_environment() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let crew_core = defs_root.join("Crew.c4d");
        std::fs::create_dir_all(&crew_core).expect("crew definition dir");
        std::fs::write(
            crew_core.join("DefCore.txt"),
            "[DefCore]\nid=CLNK\nName=Clonk\nCategory=0\nCrewMember=1\n",
        )
        .expect("write crew defcore");
        std::fs::write(crew_core.join("Script.c"), "// crew script\n").expect("crew script");

        let scenario_dir = dir.path().join("LegacyEnvironment.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            r#"
            [Head]
            Title=Legacy Environment

            [Definitions]
            Definition1=Defs.c4d

            [Player1]
            Crew=CLNK=1
            Position=20,40

            [Landscape]
            Gravity=120

            [Weather]
            Wind=10,5,-20,20
            Climate=60
            Rain=35
            Lightning=12
            StartSeason=30,10,0,100
            YearSpeed=45
            NoGamma=0

            [Disasters]
            Meteorite=25
            Volcano=15
            Earthquake=5
            "#,
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");

        let physics = scenario.physics().expect("physics present");
        assert_eq!(
            physics.gravity, 120,
            "expected gravity parsed from Scenario.txt"
        );

        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.wind, 10, "expected wind base from Scenario.txt");
        assert_eq!(
            environment.wind_variation, 5,
            "expected wind variation from Scenario.txt"
        );
        assert_eq!(
            environment.climate, -10,
            "expected climate transformed value"
        );
        assert_eq!(
            environment.temperature, -10,
            "temperature should match initial climate"
        );
        assert_eq!(environment.season, 30, "StartSeason should map to season");
        assert_eq!(environment.year_speed, 45, "YearSpeed should be retained");
        assert_eq!(
            environment.precipitation, 35,
            "rain should map to precipitation"
        );
        assert_eq!(
            environment.precipitation_strength, 35,
            "rain should map to precipitation strength"
        );
        assert_eq!(
            environment.lightning, 12,
            "lightning level should be parsed"
        );
        assert_eq!(
            environment.meteorite, 25,
            "meteorite level should be parsed"
        );
        assert_eq!(environment.volcano, 15, "volcano level should be parsed");
        assert_eq!(
            environment.earthquake, 5,
            "earthquake level should be parsed"
        );
        assert!(
            !environment.no_gamma,
            "NoGamma=0 should enable gamma correction"
        );

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let configured_physics = engine.physics();
        assert_eq!(
            configured_physics.gravity, 120,
            "engine should receive legacy gravity"
        );

        let configured_environment = engine.environment();
        // The applied wind is C4Weather::Init's Wind.Evaluate draw, not the
        // C4SVal base (C4Weather.cpp:47) — replay the init ledger to the
        // wind draw: Season, YearSpeed, Climate precede it.
        let mut replay = crate::rng::LcgRng::seed_from_u64(0);
        // Landscape.ScenarioInit's Gravity draw precedes the weather
        // evaluates (C4Landscape.cpp:66); this scenario's Gravity=120.
        LegacyC4SVal::new(120, 0, 10, 200).evaluate(&mut replay);
        // No NoInitialize: InitVegetation/InitInEarth ALWAYS evaluate
        // their levels — one draw each even with empty id lists
        // (C4Game.cpp:3069,3084) — between the Gravity draw and
        // Weather.Init's.
        LegacyC4SVal::new(50, 30, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(50, 0, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(30, 10, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(45, 0, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(60, 10, 0, 100).evaluate(&mut replay);
        let drawn_wind = LegacyC4SVal::new(10, 5, -20, 20).evaluate(&mut replay);
        assert_eq!(
            configured_environment.wind, drawn_wind,
            "engine wind is the Wind.Evaluate init draw (C4Weather.cpp:47)"
        );
        assert_eq!(
            configured_environment.wind_variation, 5,
            "engine should receive wind variation"
        );
        assert_eq!(
            configured_environment.year_speed, 45,
            "engine should receive year speed"
        );
        assert_eq!(
            configured_environment.lightning, 12,
            "engine should receive lightning level"
        );
        assert_eq!(
            configured_environment.meteorite, 25,
            "engine should receive meteorite level"
        );
        assert_eq!(
            configured_environment.volcano, 15,
            "engine should receive volcano level"
        );
        assert_eq!(
            configured_environment.earthquake, 5,
            "engine should receive earthquake level"
        );
        assert!(
            !configured_environment.no_gamma,
            "engine should reflect gamma enabled flag"
        );
    }

    #[test]
    fn physics_validation_rejects_invalid_limits() {
        let manifest = PhysicsManifest {
            gravity: Some(1),
            max_fall_speed: Some(4),
            max_rise_speed: Some(6),
            max_horizontal_speed: None,
        };

        let error = manifest
            .into_settings()
            .expect_err("invalid physics manifest fails");
        match error {
            ScenarioError::InvalidPhysics(detail) => {
                assert!(detail.contains("max_rise_speed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn physics_validation_rejects_negative_horizontal_speed() {
        let manifest = PhysicsManifest {
            gravity: None,
            max_fall_speed: None,
            max_rise_speed: None,
            max_horizontal_speed: Some(-1),
        };

        let error = manifest
            .into_settings()
            .expect_err("negative horizontal speed fails");
        match error {
            ScenarioError::InvalidPhysics(detail) => {
                assert!(detail.contains("max_horizontal_speed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
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
    // Action=Breeze Phase=18.
    #[test]
    fn loaded_objects_sync_clearance_and_action_rules_like_cpp() {
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        write_palm_def(&defs);
        write_scenario(
            dir.path(),
            "[Object]\nid=PALM\nNumber=3\nCategory=1\nX=204\nY=258\nFixX=f1129054208\nFixY=f1133445120\n\n\
             [Object]\nid=PALM\nNumber=42\nCategory=1\nX=981\nY=280\nAction=Breeze\nDir=1\nActionTime=694\nPhase=18\nPhaseDelay=4\nYDir=f1149998051\n\n\
             [Object]\nid=PALM\nNumber=43\nCategory=1\nX=100\nY=100\nAction=Stand\nPhase=2\n",
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
        assert_eq!(phase, 0, "Idle carries no phase");
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
        write_scenario(
            dir.path(),
            "[Object]\nid=GATE\nNumber=7\nCategory=2\nX=50\nY=50\nSolidMask=0,0,0,0,0,0\n\n\
             [Object]\nid=GATE\nNumber=8\nCategory=2\nX=90\nY=50\n",
        );
        let (engine, _) = load(dir.path());

        // (The fixture def has no Graphics.png, so like C++ the def-level
        // mask never activates for movement; the loader state is the pin.)
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
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
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

    // C4Game::Synchronize's tail broadcasts ~UpdateTransferZone to EVERY
    // object (C4Game.cpp:3710, C4ObjectList.cpp:734-739) AFTER the
    // FixRandom re-fix. GoldRush oracle: the placed cannon's handler
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

        write_scenario(
            dir.path(),
            "[Object]\nid=CANN\nNumber=439\nCategory=16\nX=100\nY=100\nAction=Stand\n",
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
