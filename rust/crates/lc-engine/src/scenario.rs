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
    ActionDefinition as ResourceActionDefinition, ActionMap as ResourceActionMap, ColorByOwnerMask,
    DefinitionError as ResourceDefinitionError, GraphicsImage, Group, GroupError,
    ResourceDefinition as ResourceDefinitionData,
};
use serde::de::Error as _;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::Deserialize;

use crate::{
    action::ActionSpec, ActionState, CommandDirection, Definition, DefinitionActionFacet,
    DefinitionActionGraphics, DefinitionComponent, DefinitionPicture, DefinitionPictureImage,
    DefinitionSpriteImage, Direction, EffectState, Engine, EngineError, EnvironmentSettings,
    Landscape, MovementProfile, ObjectId, ObjectStatus, PhysicsSettings, RgbColor, SkyParallaxMode,
    SkySettings, SpawnConfig, Vector2, FULL_CON,
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
    movement: MovementProfile,
    category: i32,
    value: i32,
    mass: i32,
    picture: Option<DefinitionPicture>,
    picture_image: Option<GraphicsImage>,
    graphics_image: Option<GraphicsImage>,
    color_by_owner_mask: Option<ColorByOwnerMask>,
    additional_graphics: HashMap<String, ResourceGraphicsVariant>,
    resource_group: Option<Group>,
    components: Vec<DefinitionComponent>,
    line_connect: u32,
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
    environment: Option<EnvironmentSettings>,
    sky: Option<SkyConfig>,
    script: Option<ScenarioScriptSource>,
    objectives: ScenarioObjectives,
    construction_needs_material: bool,
    structures_need_energy: bool,
    base_buy_enabled: bool,
    base_sell_enabled: bool,
    landscape_insert_thrust: bool,
    /// The scenario's own System.c4g script sources: C++ loads them into
    /// the global script engine (C4Game::LoadScenarioScripts,
    /// C4Game.cpp:3317-3343).
    system_scripts: Vec<(String, String)>,
    /// The four C4SPlrStart slots, consumed by joining players
    /// (C4Player::ScenarioInit, C4Player.cpp:670-777).
    player_starts: Vec<PlayerStart>,
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
        let group = Group::open(path)?;
        Self::load_from_group_with(&group, resolver)
    }

    pub fn load_from_group_with<R: LegacyDefinitionResolver>(
        group: &Group,
        resolver: &R,
    ) -> Result<Self, ScenarioError> {
        match Self::load_from_group(group) {
            Ok(scenario) => Ok(scenario),
            Err(ScenarioError::ManifestMissing) => Self::load_legacy_from_group(group, resolver),
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

    fn load_legacy_from_group<R: LegacyDefinitionResolver>(
        group: &Group,
        resolver: &R,
    ) -> Result<Self, ScenarioError> {
        let manifest = parse_legacy_scenario_manifest(group)?;

        let mut collected = Vec::new();
        let mut seen_ids = HashSet::new();

        // The scenario group itself is a definition source: every .c4d
        // child is a scenario-local definition, loaded with override
        // priority over the packs (C4Game::InitDefs loads the scenario
        // last with fOverload; first-wins dedup here gives the same
        // outcome by collecting locals first).
        for entry in group.entries()? {
            if !entry.is_directory {
                continue;
            }
            let name = entry.relative_path.to_string_lossy().to_ascii_lowercase();
            if !(name.ends_with(".c4d") || name.ends_with(".ocd") || name.ends_with(".ocg")) {
                continue;
            }
            let child = group.open_child(&entry.relative_path)?;
            collect_definitions_from_group(&child, &mut seen_ids, &mut collected)?;
        }

        // The parent folder chain is a definition source too: every .c4d
        // child of a .c4f ancestor serves the scenarios inside it (C++
        // folder definitions — e.g. Hazard.c4f/ScenObjects.c4d).
        let mut ancestor = group.root().parent();
        while let Some(folder) = ancestor {
            let is_folder_group = folder
                .file_name()
                .map(|name| {
                    name.to_string_lossy()
                        .to_ascii_lowercase()
                        .ends_with(".c4f")
                })
                .unwrap_or(false);
            if !is_folder_group {
                break;
            }
            if let Ok(folder_group) = Group::open(folder) {
                for entry in folder_group.entries()? {
                    if !entry.is_directory {
                        continue;
                    }
                    let name = entry.relative_path.to_string_lossy().to_ascii_lowercase();
                    if !(name.ends_with(".c4d") || name.ends_with(".ocd")) {
                        continue;
                    }
                    let child = folder_group.open_child(&entry.relative_path)?;
                    collect_definitions_from_group(&child, &mut seen_ids, &mut collected)?;
                }
            }
            ancestor = folder.parent();
        }

        for spec in &manifest.definition_specs {
            let groups = resolver.resolve_definition_groups(group, spec)?;
            if groups.is_empty() {
                return Err(ScenarioError::LegacyDefinitionNotFound { path: spec.clone() });
            }
            for definition_group in groups {
                collect_definitions_from_group(&definition_group, &mut seen_ids, &mut collected)?;
            }
        }

        if !manifest.core.definitions.skip_defs.is_empty() {
            let skip_ids: HashSet<String> = manifest
                .core
                .definitions
                .skip_defs
                .iter()
                .map(|entry| entry.id.clone())
                .collect();
            collected.retain(|definition| {
                let id_upper = definition.id.to_ascii_uppercase();
                !skip_ids.contains(&id_upper)
            });
        }

        if collected.is_empty() {
            return Err(ScenarioError::NoDefinitions);
        }

        let script = load_legacy_scenario_script(group)?;
        let landscape = load_legacy_landscape(group, &manifest)?;
        let mut initial_spawns = collect_initial_spawns(&manifest.sections, &collected)?;
        let mut object_spawns = collect_legacy_objects(group, &collected)?;
        initial_spawns.append(&mut object_spawns);
        let physics = derive_legacy_physics(&manifest)?;
        let environment = derive_legacy_environment(&manifest)?;

        Ok(Self {
            name: manifest.title,
            description: manifest.description,
            ticks: None,
            ground_height_hint: manifest.ground_height_hint,
            definitions: collected,
            initial_spawns,
            landscape,
            physics,
            environment: Some(environment),
            sky: None,
            script,
            objectives: ScenarioObjectives::from_legacy_game(&manifest.core.game),
            construction_needs_material: manifest.core.game.realism.construction_needs_material,
            structures_need_energy: manifest.core.game.realism.structures_need_energy,
            base_buy_enabled: (manifest.core.game.realism.base_functionality & BASEFUNC_BUY) != 0,
            base_sell_enabled: (manifest.core.game.realism.base_functionality & BASEFUNC_SELL) != 0,
            landscape_insert_thrust: manifest.core.game.realism.landscape_insert_thrust != 0,
            system_scripts: load_scenario_system_scripts(group)?,
            player_starts: PlayerStart::slots_from_legacy(&manifest.core.players),
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

    pub fn physics(&self) -> Option<PhysicsSettings> {
        self.physics
    }

    pub fn environment(&self) -> Option<EnvironmentSettings> {
        self.environment
    }

    pub fn sky(&self) -> Option<&SkyConfig> {
        self.sky.as_ref()
    }

    pub fn apply(&self, engine: &mut Engine) -> Result<Vec<ObjectId>, ScenarioError> {
        engine.clear_scenario_script();
        // The scenario's own System.c4g joins the global script engine
        // before any definition code runs (C4Game::LoadScenarioScripts,
        // C4Game.cpp:3317-3343).
        if !self.system_scripts.is_empty() {
            engine.install_additional_global_scripts(&self.system_scripts);
        }
        engine.configure_objectives(self.objectives.clone());
        // C4SPlrStart outlives scenario load: ScenarioInit reads it when a
        // player joins (C4Player.cpp:670-777).
        engine.set_player_starts(self.player_starts.clone());
        if let Some(landscape) = &self.landscape {
            engine.set_landscape(landscape.clone());
        } else {
            engine.clear_landscape();
        }

        if let Some(physics) = self.physics {
            engine.set_physics(physics);
        }

        engine.set_environment(self.environment.unwrap_or_default());
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

        for definition in &self.definitions {
            let name = definition.name.as_deref().unwrap_or(&definition.id);
            // C4Def::Load ignores Script.Load failures (C4Def.cpp:632): a
            // definition with a broken script still loads, script-less; the
            // error only shows in the log.
            let mut compiled = match Definition::from_script(&definition.id, name, &definition.script)
            {
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
            if let Some(actions) = &definition.actions {
                compiled.configure_actions(actions.default_action.clone(), actions.specs.clone());
                compiled.configure_action_graphics(actions.graphics.clone());
            }
            compiled.set_crew_member(definition.crew_member);
            compiled.set_movement_profile(definition.movement);
            compiled.set_category(definition.category);
            compiled.set_value(definition.value);
            compiled.set_mass(definition.mass);
            compiled.set_picture(definition.picture);
            let picture_image = definition
                .picture_image
                .as_ref()
                .map(DefinitionPictureImage::from_resource);
            compiled.set_picture_image(picture_image);
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

        // CRITICAL: Resolve #include directives now that all definitions are registered
        // This allows child definitions to inherit functions from parent definitions
        engine.resolve_includes()?;

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

        if let Some(script) = &self.script {
            // A scenario script that fails to COMPILE logs and the round
            // runs script-less (C4ScriptHost load behavior); Initialize
            // runtime errors are already tolerated inside
            // `install_scenario_script`.
            match engine.install_scenario_script(&script.name, &script.source) {
                Ok(mut additional) => created.append(&mut additional),
                Err(EngineError::Script {
                    definition,
                    function,
                    source,
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
                    movement: MovementProfile::default(),
                    category: category_override.unwrap_or(crate::DEFAULT_CATEGORY),
                    value: 0,
                    mass: 0,
                    picture: None,
                    picture_image: None,
                    graphics_image: None,
                    color_by_owner_mask: None,
                    additional_graphics: HashMap::new(),
                    resource_group: None,
                    components: Vec::new(),
                    line_connect: 0,
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

        Ok(Self {
            name: manifest.name,
            description: manifest.description,
            ticks: manifest.ticks,
            ground_height_hint,
            definitions,
            initial_spawns: spawns,
            landscape,
            physics,
            environment,
            sky,
            script,
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: false,
            base_sell_enabled: false,
            landscape_insert_thrust: false,
            system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
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
        let section = current_section
            .clone()
            .unwrap_or_else(|| "head".to_string());
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

fn load_legacy_scenario_script(
    group: &Group,
) -> Result<Option<ScenarioScriptSource>, ScenarioError> {
    const SCRIPT_CANDIDATES: [&str; 1] = ["Script.c"];
    for candidate in SCRIPT_CANDIDATES {
        if !group.exists(candidate) {
            continue;
        }
        let bytes = group.read_file(candidate)?;
        // Use lossy UTF-8 conversion to handle legacy ISO-8859-1/Windows-1252 encoded scripts
        let source = String::from_utf8_lossy(&bytes).into_owned();
        return Ok(Some(ScenarioScriptSource {
            name: candidate.to_string(),
            source,
        }));
    }
    Ok(None)
}

fn collect_initial_spawns(
    sections: &HashMap<String, Vec<(String, String)>>,
    definitions: &[ScenarioDefinition],
) -> Result<Vec<ScenarioSpawn>, ScenarioError> {
    let mut spawns = Vec::new();
    let mut player_sections: Vec<(&str, &Vec<(String, String)>)> = sections
        .iter()
        .filter_map(|(section, entries)| {
            if section.starts_with("player") {
                Some((section.as_str(), entries))
            } else {
                None
            }
        })
        .collect();
    player_sections.sort_by(|a, b| a.0.cmp(b.0));

    for (section_name, entries) in player_sections {
        let owner = match owner_index_from_section(section_name) {
            Some(owner) => owner,
            None => continue,
        };
        let position = entries.iter().find_map(|(key, value)| {
            if key.eq_ignore_ascii_case("Position") {
                parse_player_position(value)
            } else {
                None
            }
        });

        // `StandardCrew=` names the native crew def and `Clonks=` is its
        // C4SVal COUNT (C4SPlrStart: NativeCrew + Crew C4SVal(1,0,1,10),
        // C4Scenario.cpp:261,278-279) — NOT a crew-ID list. The count uses
        // Std bounded to [Min,Max]; the synced Evaluate draw waits on the
        // player-join port.
        let standard_crew = entries
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("StandardCrew"))
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("None"));
        if let Some(token) = standard_crew {
            let count = entries
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("Clonks"))
                .and_then(|(key, value)| {
                    parse_legacy_c4s_value(key, value, LegacyC4SVal::new(1, 0, 1, 10)).ok()
                })
                .unwrap_or(LegacyC4SVal::new(1, 0, 1, 10))
                .base()
                .max(0);
            match find_definition_by_token(definitions, token) {
                Some(definition) => {
                    for _ in 0..count {
                        let mut config = SpawnConfig::new(&definition.id)
                            .with_owner(owner)
                            .with_crew_member(true);
                        if let Some(position) = position {
                            config = config.with_position(position);
                        }
                        spawns.push(ScenarioSpawn {
                            handle: None,
                            container_handle: None,
                            config,
                        });
                    }
                }
                None => tracing::warn!(
                    section = section_name,
                    definition = token,
                    "StandardCrew names an unknown definition; no native crew placed"
                ),
            }
        }

        for (_, value) in entries
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("Crew"))
        {
            for (token, count) in parse_crew_entries(value) {
                let definition = find_definition_by_token(definitions, &token)
                    .ok_or_else(|| ScenarioError::UnknownDefinition(token.clone()))?;
                for _ in 0..count {
                    let mut config = SpawnConfig::new(&definition.id)
                        .with_owner(owner)
                        .with_crew_member(true);
                    if let Some(position) = position {
                        config = config.with_position(position);
                    }
                    spawns.push(ScenarioSpawn {
                        handle: None,
                        container_handle: None,
                        config,
                    });
                }
            }
        }
    }

    Ok(spawns)
}

/// Collects the global scripts of a System.c4g group (the `*.c` entries,
/// sorted by name) for `Engine::install_global_scripts` — C++ loads these
/// into `Game.ScriptEngine` at init (C4Game InitScriptEngine).
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
    sources.sort_by(|a, b| a.0.cmp(&b.0));
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

fn load_legacy_landscape(
    group: &Group,
    manifest: &LegacyScenarioManifest,
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
        (read_optional("Map.bmp")?, map_zoom_u32)
    };

    if let Some(bytes) = map_bytes {
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

    let fallback_map_width = map_width_hint.unwrap_or(96);
    let fallback_map_height = map_height_hint.unwrap_or(50);
    let width_product =
        i64::from(fallback_map_width).saturating_mul(i64::from(map_zoom_u32));
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

fn derive_legacy_physics(
    manifest: &LegacyScenarioManifest,
) -> Result<Option<PhysicsSettings>, ScenarioError> {
    let entries = manifest.sections.get("landscape");
    if entries.is_none() {
        return Ok(None);
    }
    let gravity_defaults = LegacyC4SVal::new(100, 0, 10, 200);
    let gravity = legacy_c4s_value(entries, "gravity", gravity_defaults)?;
    let mut physics = PhysicsSettings::default();
    physics.gravity = gravity.base();
    Ok(Some(physics))
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
    environment = environment.with_season(season_value.base().clamp(0, 100));

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

    let mut spawns = Vec::new();
    for record in records.into_iter() {
        if let Some(spawn) = record.into_spawn(&definition_ids)? {
            spawns.push(spawn);
        }
    }
    Ok(spawns)
}

#[derive(Debug, Default)]
struct LegacyObjectRecord {
    line: usize,
    id: Option<String>,
    number: Option<u64>,
    status: Option<ObjectStatus>,
    owner: Option<i32>,
    x: Option<i32>,
    y: Option<i32>,
    xdir: Option<i32>,
    ydir: Option<i32>,
    energy: Option<i32>,
    construction: Option<i32>,
    alive: Option<bool>,
    category: Option<i32>,
    direction: Option<Direction>,
    command_direction: Option<CommandDirection>,
    action_name: Option<String>,
    action_phase: Option<i32>,
    action_ticks: Option<u32>,
    action_data: Option<i32>,
    action_target: Option<u64>,
    action_target2: Option<u64>,
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
                self.xdir = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid XDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "ydir" => {
                self.ydir = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid YDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "energy" => {
                self.energy = Some(parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid Energy `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?);
            }
            "con" => {
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
                // C4Object Dir is a plain int — multi-directional defs use
                // 0..Directions-1 (Dir=8 in Knights Camp.c4s). The two-way
                // engine model keeps its default for now (PORT_STATUS).
                match Direction::from_script_value(raw) {
                    Some(direction) => self.direction = Some(direction),
                    None => tracing::warn!(
                        line = self.line,
                        value = raw,
                        "Objects.txt Dir exceeds the two-way direction model; keeping default"
                    ),
                }
            }
            "comdir" => {
                let raw = parse_i32(trimmed_value).map_err(|err| {
                    ScenarioError::LegacyObjectsParse(format!(
                        "Objects.txt line {}: invalid ComDir `{}` ({})",
                        self.line, trimmed_value, err
                    ))
                })?;
                match CommandDirection::from_script_value(raw) {
                    Some(command_direction) => {
                        self.command_direction = Some(command_direction)
                    }
                    None => tracing::warn!(
                        line = self.line,
                        value = raw,
                        "Objects.txt ComDir outside the COMD_* range; keeping default"
                    ),
                }
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
    ) -> Result<Option<ScenarioSpawn>, ScenarioError> {
        let Self {
            line,
            id,
            number,
            status,
            owner,
            x,
            y,
            xdir,
            ydir,
            energy,
            construction,
            alive,
            category,
            direction,
            command_direction,
            action_name,
            action_phase,
            action_ticks,
            action_data,
            action_target,
            action_target2,
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

        let mut config = SpawnConfig::new(id.clone()).with_id(ObjectId::new(number));
        config = config.with_position(Vector2::new(x.unwrap_or(0), y.unwrap_or(0)));

        if xdir.is_some() || ydir.is_some() {
            config = config.with_velocity(Vector2::new(xdir.unwrap_or(0), ydir.unwrap_or(0)));
        }
        if let Some(owner) = owner {
            config = config.with_owner(owner);
        }
        if let Some(energy) = energy {
            config = config.with_energy(energy);
        }
        if let Some(construction) = construction {
            config = config.with_construction(construction);
        }
        if let Some(alive) = alive {
            config = config.with_alive(alive);
        }
        if let Some(category) = category {
            config = config.with_category(category);
        }
        if let Some(status) = status {
            if status != ObjectStatus::Normal {
                config = config.with_status(status);
            }
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
    ticks: Option<u32>,
    data: Option<i32>,
    target: Option<u64>,
    target2: Option<u64>,
) -> Option<ActionState> {
    let name = name?;
    let mut state = ActionState::new(name);
    if let Some(value) = phase {
        state.phase = value;
    }
    if let Some(value) = ticks {
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

fn collect_definitions_from_group(
    group: &Group,
    seen_ids: &mut HashSet<String>,
    output: &mut Vec<ScenarioDefinition>,
) -> Result<(), ScenarioError> {
    if group.exists("DefCore.txt") {
        let resource = ResourceDefinitionData::load(group)?;
        let id = resource.core.id.clone();
        if seen_ids.insert(id.clone()) {
            output.push(scenario_definition_from_resource(
                resource,
                Some(group.clone()),
            ));
        }
    }

    for entry in group.entries()? {
        if !entry.is_directory {
            continue;
        }
        let child = group.open_child(&entry.relative_path)?;
        collect_definitions_from_group(&child, seen_ids, output)?;
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
        graphics_image,
        color_by_owner_mask,
        additional_graphics,
    } = resource;
    let actions = action_map.map(|map| convert_action_map(&map));

    ScenarioDefinition {
        id: core.id,
        name: core.name,
        script: script.combined().to_string(),
        actions,
        crew_member: core.crew_member,
        movement: MovementProfile::default(),
        category: core.category,
        value: core.value,
        mass: core.mass,
        picture: core.picture.map(DefinitionPicture::from),
        picture_image,
        graphics_image,
        color_by_owner_mask,
        additional_graphics,
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
            settings.back_color = Some(rgb_to_bgr_u32(color.into_color()));
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
    fn legacy_c4sval_evaluate_draws_via_the_game_rng_like_cpp() {
        // C4SVal::Evaluate (C4Scenario.cpp:43-46):
        //   return BoundBy(Std + Random(2 * Rnd + 1) - Rnd, Min, Max);
        // One Random draw per Evaluate — even Rnd == 0 calls Random(1),
        // which advances the LCG stream (C4Random.h:40-61).
        let mut reference = crate::rng::LcgRng::new(42);
        let expected = (10 + reference.random(2 * 3 + 1) - 3).clamp(0, 250);

        let mut rng = crate::rng::LcgRng::new(42);
        assert_eq!(LegacyC4SVal::new(10, 3, 0, 250).evaluate(&mut rng), expected);
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
                        "Walk": { "length": 2, "next": "Idle" },
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
                movement: MovementProfile::default(),
                category: crate::DEFAULT_CATEGORY,
                value: 0,
                mass: 0,
                picture: None,
                picture_image: None,
                graphics_image: None,
                color_by_owner_mask: None,
                additional_graphics: HashMap::new(),
                resource_group: None,
                components: Vec::new(),
                line_connect: 0,
            }],
            initial_spawns: vec![ScenarioSpawn {
                handle: None,
                container_handle: None,
                config: SpawnConfig::new("Mover"),
            }],
            landscape: None,
            physics: None,
            environment: None,
            sky: None,
            script: Some(ScenarioScriptSource {
                name: "Script.c".into(),
                source: scenario_script.to_string(),
            }),
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            landscape_insert_thrust: false,
            system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
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
                movement: MovementProfile::default(),
                category: crate::DEFAULT_CATEGORY,
                value: 0,
                mass: 0,
                picture: None,
                picture_image: None,
                graphics_image: None,
                color_by_owner_mask: None,
                additional_graphics: HashMap::new(),
                resource_group: None,
                components: Vec::new(),
                line_connect: 0,
            }],
            initial_spawns: vec![ScenarioSpawn {
                handle: None,
                container_handle: None,
                config: SpawnConfig::new("Mover").with_owner(1),
            }],
            landscape: None,
            physics: None,
            environment: None,
            sky: None,
            script: Some(ScenarioScriptSource {
                name: "Script.c".into(),
                source: scenario_script.to_string(),
            }),
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            landscape_insert_thrust: false,
            system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
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
    fn objects_dir_values_beyond_the_two_way_enum_do_not_abort_load() {
        // C4Object::CompileFunc reads Dir as a plain int — multi-directional
        // defs legitimately store Dir=8 (Knights.c4f/Camp.c4s/Objects.txt:
        // 1098). The two-direction engine model keeps its default until the
        // Dir model widens (PORT_STATUS); loading must not abort.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nAction=Float\nDir=8\nComDir=5\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine
                .object_snapshot(ObjectId::new(10))
                .is_some(),
            "the Dir=8 object loaded"
        );
    }

    #[test]
    fn standard_crew_with_clonks_count_spawns_native_crew_like_cpp() {
        // [PlayerN] `Clonks=` is the C4SVal crew COUNT — `Crew` in
        // C4SPlrStart, default C4SVal(1,0,1,10) (C4Scenario.cpp:261,279) —
        // and `StandardCrew=` names the native crew def (NativeCrew, :278).
        // It is NOT a crew-ID list ('Clonks=5,0,1,10' must not become
        // "unknown definition `5,0,1,10`"). The count uses Std (the synced
        // Evaluate draw waits on the player-join port).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=NativeCrew\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nStandardCrew=GOOD\nClonks=2,0,1,10\nPosition=120,160\n",
        )
        .expect("write scenario core");
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 2, "Clonks Std=2 native crew spawn");
        for id in &created {
            let object = engine.object_snapshot(*id).expect("crew exists");
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
        // engine — GoldRush's 31 dialogue/helper scripts live there.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Local.c4s");
        let system = scenario_dir.join("System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Helpers.c"),
            "global func ScenarioLocalHelper() { return 42; }\n",
        )
        .expect("write helper script");
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
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");

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
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 1, "the spawn itself succeeds");
        let gone = engine
            .object_snapshot(created[0])
            .map(|snapshot| snapshot.status != ObjectStatus::Normal)
            .unwrap_or(true);
        assert!(gone, "the object ends removed like C++");
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
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 1);
        assert!(engine.object_snapshot(created[0]).is_some());
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
        objects.extend_from_slice(b"# M\xe4dchen\n[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n");
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
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 1, "the good crew member still spawns");
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
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            created.len(),
            1,
            "the object spawns despite the Construction error"
        );
        assert!(engine.object_snapshot(created[0]).is_some());
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
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 1, "crew spawned before Initialize ran");
        assert!(
            engine.scenario_script.is_some(),
            "the scenario script stays installed after the Initialize error"
        );
    }

    #[test]
    fn scenario_script_parse_errors_are_logged_not_fatal_like_cpp() {
        // A scenario Script.c that fails to compile logs the parse error and
        // the scenario runs without a script (C4ScriptHost load behavior).
        let dir = tempdir().expect("tempdir");
        let scenario_dir =
            write_resilience_fixture(dir.path(), None, "global func {{{ broken\n");
        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 1);
        assert!(
            engine.scenario_script.is_none(),
            "no scenario script installed when it cannot compile"
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
            "[Head]\nTitle=Legacy Test\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=Foo=2\nPosition=120,160\n",
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

        let mut engine = Engine::with_seed(0);
        let created = scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
        assert_eq!(
            created.len(),
            2,
            "expected two crew spawns from legacy scenario"
        );
        for id in &created {
            let object = engine.object_snapshot(*id).expect("spawned object present");
            assert_eq!(object.definition_id, "FOOO");
            assert_eq!(object.owner, 0);
            assert!(
                object.crew_member,
                "legacy crew should be marked as crew member"
            );
            assert_eq!(object.position, Vector2::new(120, 160));
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
            "[Object]\nid=BOX1\nNumber=100\nStatus=1\nCategory=0\nOwner=1\nX=10\nY=20\nContents=101\n\n[Object]\nid=GEM1\nNumber=101\nStatus=1\nCategory=0\nX=30\nY=40\nXDir=-5\nYDir=3\nEnergy=77\nAlive=false\nDir=1\nComDir=3\nAction=Idle\nActionTime=6\nPhase=2\nActionData=5\nActionTarget1=100\n",
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
        assert_eq!(first.config.position, Vector2::new(10, 20));
        assert_eq!(first.config.id, Some(ObjectId::new(100)));

        let second = &scenario.initial_spawns[1];
        assert_eq!(second.handle.as_deref(), Some("101"));
        assert_eq!(second.container_handle.as_deref(), Some("100"));
        assert_eq!(second.config.definition_id, "GEM1");
        assert_eq!(second.config.position, Vector2::new(30, 40));
        assert_eq!(second.config.velocity, Vector2::new(-5, 3));
        assert_eq!(second.config.energy, 77);
        assert_eq!(second.config.alive, Some(false));
        assert_eq!(second.config.category, Some(0));
        assert_eq!(second.config.direction, Direction::Right);
        assert_eq!(second.config.command_direction, CommandDirection::Right);
        let action = second.config.action.as_ref().expect("action state present");
        assert_eq!(action.name, "Idle");
        assert_eq!(action.ticks, 6);
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
        assert_eq!(box_snapshot.position, Vector2::new(10, 20));

        let gem_snapshot = engine
            .object_snapshot(ObjectId::new(101))
            .expect("gem object");
        assert_eq!(gem_snapshot.definition_id, "GEM1");
        assert_eq!(gem_snapshot.position, Vector2::new(30, 40));
        assert_eq!(gem_snapshot.velocity, Vector2::new(-5, 3));
        assert_eq!(gem_snapshot.energy, 77);
        assert!(!gem_snapshot.alive);
        assert_eq!(gem_snapshot.container, Some(ObjectId::new(100)));
        assert_eq!(gem_snapshot.direction, Direction::Right);
        assert_eq!(gem_snapshot.command_direction, CommandDirection::Right);
        assert_eq!(gem_snapshot.action.name, "Idle");
        assert_eq!(gem_snapshot.action.ticks, 6);
        assert_eq!(gem_snapshot.action.phase, 2);
        assert_eq!(gem_snapshot.action.data, 5);
        assert_eq!(gem_snapshot.action.target, Some(ObjectId::new(100)));
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
        assert_eq!(
            configured_environment.wind, 10,
            "engine should receive wind base"
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
