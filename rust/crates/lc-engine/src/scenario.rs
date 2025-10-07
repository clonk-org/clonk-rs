use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use lc_resources::{Group, GroupError};
use serde::Deserialize;

use crate::{
    action::ActionSpec, Definition, Engine, EngineError, Landscape, ObjectId, PhysicsSettings,
    SpawnConfig, Vector2,
};

#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    #[error("scenario manifest `Scenario.json` not found")]
    ManifestMissing,
    #[error("failed to parse scenario manifest: {0}")]
    ManifestParse(#[from] serde_json::Error),
    #[error("scenario resource error: {0}")]
    Resources(#[from] GroupError),
    #[error("script file `{path}` is missing from the scenario")]
    MissingScript { path: PathBuf },
    #[error("script file `{path}` is not valid UTF-8")]
    ScriptEncoding { path: PathBuf },
    #[error("duplicate definition id `{0}` in scenario manifest")]
    DuplicateDefinition(String),
    #[error("initial object references unknown definition `{0}`")]
    UnknownDefinition(String),
    #[error("invalid landscape specification: {0}")]
    InvalidLandscape(String),
    #[error("scenario did not declare any object definitions")]
    NoDefinitions,
    #[error("invalid physics settings: {0}")]
    InvalidPhysics(String),
    #[error("engine error while applying scenario: {0}")]
    Engine(#[from] EngineError),
}

#[derive(Debug, Clone)]
struct ScenarioDefinition {
    id: String,
    name: Option<String>,
    script: String,
    actions: Option<DefinitionActions>,
}

#[derive(Debug, Clone)]
struct DefinitionActions {
    default_action: Option<String>,
    specs: HashMap<String, ActionSpec>,
}

#[derive(Debug)]
pub struct Scenario {
    name: Option<String>,
    ticks: Option<u32>,
    ground_height_hint: Option<i32>,
    definitions: Vec<ScenarioDefinition>,
    initial_spawns: Vec<SpawnConfig>,
    landscape: Option<Landscape>,
    physics: Option<PhysicsSettings>,
}

impl Scenario {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ScenarioError> {
        let group = Group::open(path)?;
        Self::load_from_group(&group)
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

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn configured_ticks(&self) -> Option<u32> {
        self.ticks
    }

    pub fn ground_height_hint(&self) -> Option<i32> {
        self.ground_height_hint
    }

    pub fn has_initial_objects(&self) -> bool {
        !self.initial_spawns.is_empty()
    }

    pub fn physics(&self) -> Option<PhysicsSettings> {
        self.physics
    }

    pub fn apply(&self, engine: &mut Engine) -> Result<Vec<ObjectId>, ScenarioError> {
        if let Some(landscape) = &self.landscape {
            engine.set_landscape(landscape.clone());
        } else {
            engine.clear_landscape();
        }

        if let Some(physics) = self.physics {
            engine.set_physics(physics);
        }

        for definition in &self.definitions {
            let name = definition.name.as_deref().unwrap_or(&definition.id);
            let mut compiled = Definition::from_script(&definition.id, name, &definition.script)?;
            if let Some(actions) = &definition.actions {
                compiled.configure_actions(actions.default_action.clone(), actions.specs.clone());
            }
            engine.register_definition(compiled)?;
        }

        let mut created = Vec::with_capacity(self.initial_spawns.len());
        for spawn in &self.initial_spawns {
            let id = engine.spawn_object(spawn.clone())?;
            created.push(id);
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
            } = definition;

            if !seen_ids.insert(id.clone()) {
                return Err(ScenarioError::DuplicateDefinition(id));
            }

            let script_path = Path::new(&script);
            let script_bytes = match group.read_file(script_path) {
                Ok(bytes) => bytes,
                Err(GroupError::EntryNotFound(_)) => {
                    return Err(ScenarioError::MissingScript {
                        path: PathBuf::from(script_path),
                    })
                }
                Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                    return Err(ScenarioError::MissingScript {
                        path: PathBuf::from(script_path),
                    })
                }
                Err(error) => return Err(ScenarioError::Resources(error)),
            };
            let script_source =
                String::from_utf8(script_bytes).map_err(|_| ScenarioError::ScriptEncoding {
                    path: PathBuf::from(script_path),
                })?;

            let actions = if actions.is_empty() && default_action.is_none() {
                None
            } else {
                Some(DefinitionActions {
                    default_action,
                    specs: actions,
                })
            };

            definitions.push(ScenarioDefinition {
                id,
                name,
                script: script_source,
                actions,
            });
        }

        let mut spawns = Vec::with_capacity(manifest.initial_objects.len());
        for object in manifest.initial_objects {
            if !seen_ids.contains(&object.definition) {
                return Err(ScenarioError::UnknownDefinition(object.definition));
            }

            let mut spawn = SpawnConfig::new(object.definition.clone());
            if let Some(position) = object.position {
                spawn = spawn.with_position(Vector2::new(position[0], position[1]));
            }
            if let Some(velocity) = object.velocity {
                spawn = spawn.with_velocity(Vector2::new(velocity[0], velocity[1]));
            }
            if let Some(energy) = object.energy {
                spawn = spawn.with_energy(energy);
            }
            if let Some(owner) = object.owner {
                spawn = spawn.with_owner(owner);
            }
            spawns.push(spawn);
        }

        let landscape = match manifest.landscape {
            Some(spec) => Some(spec.into_landscape()?),
            None => None,
        };
        let physics = match manifest.physics {
            Some(spec) => Some(spec.into_settings()?),
            None => None,
        };
        let ground_height_hint = manifest.ground_height.or_else(|| {
            landscape
                .as_ref()
                .and_then(|landscape| landscape.surface().first().copied())
        });

        Ok(Self {
            name: manifest.name,
            ticks: manifest.ticks,
            ground_height_hint,
            definitions,
            initial_spawns: spawns,
            landscape,
            physics,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ScenarioManifest {
    #[serde(default)]
    name: Option<String>,
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
}

impl PhysicsManifest {
    fn into_settings(self) -> Result<PhysicsSettings, ScenarioError> {
        let defaults = PhysicsSettings::default();
        let gravity = self.gravity.unwrap_or(defaults.gravity);
        let max_fall_speed = self.max_fall_speed.unwrap_or(defaults.max_fall_speed);
        let max_rise_speed = self.max_rise_speed.unwrap_or(defaults.max_rise_speed);

        PhysicsSettings::checked(gravity, max_fall_speed, max_rise_speed)
            .map_err(|detail| ScenarioError::InvalidPhysics(detail.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                "max_rise_speed": -10
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

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let configured = engine.physics();
        assert_eq!(configured.gravity, 2);
        assert_eq!(configured.max_fall_speed, 8);
        assert_eq!(configured.max_rise_speed, -10);
    }

    #[test]
    fn physics_validation_rejects_invalid_limits() {
        let dir = tempdir().expect("tempdir");
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "physics": {
                "gravity": 1,
                "max_fall_speed": 4,
                "max_rise_speed": 6
            }
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).expect("scripts dir");
        std::fs::write(dir.path().join("Scenario.json"), manifest).expect("write manifest");
        std::fs::write(dir.path().join("scripts/mover.aul"), TEST_SCRIPT).expect("write script");

        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        match error {
            ScenarioError::InvalidPhysics(detail) => {
                assert!(detail.contains("max_rise_speed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
