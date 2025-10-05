pub mod ffi;
mod landscape;
mod record;

pub use landscape::{CollisionResolution, Landscape, LandscapeError};
pub use record::{Playback, PlaybackError, Recorder, Recording};

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::ops::AddAssign;

use lc_script::{Engine as ScriptEngine, ScriptError, Value};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type DefinitionId = String;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectState {
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
}

impl ObjectState {
    fn apply_delta(&mut self, delta: &ObjectDelta) {
        if let Some(position) = delta.position {
            self.position = position;
        }
        if let Some(velocity) = delta.velocity {
            self.velocity = velocity;
        }
        if let Some(energy) = delta.energy {
            self.energy = energy;
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ObjectDelta {
    position: Option<Vector2>,
    velocity: Option<Vector2>,
    energy: Option<i32>,
}

#[derive(Debug, Clone)]
struct Object {
    id: ObjectId,
    definition_id: DefinitionId,
    state: ObjectState,
    destroyed: bool,
}

impl Object {
    fn new(id: ObjectId, definition_id: DefinitionId, state: ObjectState) -> Self {
        Self {
            id,
            definition_id,
            state,
            destroyed: false,
        }
    }

    fn mark_destroyed(&mut self) {
        self.destroyed = true;
    }

    fn snapshot(&self) -> ObjectSnapshot {
        ObjectSnapshot {
            id: self.id,
            definition_id: self.definition_id.clone(),
            position: self.state.position,
            velocity: self.state.velocity,
            energy: self.state.energy,
        }
    }
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("definition `{0}` is already registered")]
    DefinitionAlreadyExists(String),
    #[error("unknown definition `{0}`")]
    UnknownDefinition(String),
    #[error("script error in {function} of `{definition}`")]
    Script {
        definition: String,
        function: &'static str,
        #[source]
        source: ScriptError,
    },
    #[error("invalid script output in {function} of `{definition}`: {detail}")]
    InvalidScriptOutput {
        definition: String,
        function: &'static str,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnConfig {
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
}

impl SpawnConfig {
    pub fn new(definition_id: impl Into<String>) -> Self {
        Self {
            definition_id: definition_id.into(),
            position: Vector2::ZERO,
            velocity: Vector2::ZERO,
            energy: 0,
        }
    }

    pub fn with_position(mut self, position: Vector2) -> Self {
        self.position = position;
        self
    }

    pub fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = velocity;
        self
    }

    pub fn with_energy(mut self, energy: i32) -> Self {
        self.energy = energy;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub id: ObjectId,
    pub definition_id: DefinitionId,
    pub position: Vector2,
    pub velocity: Vector2,
    pub energy: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    pub frame: u64,
    pub objects: Vec<ObjectSnapshot>,
}

impl SimulationSnapshot {
    pub fn object(&self, id: ObjectId) -> Option<&ObjectSnapshot> {
        self.objects.iter().find(|object| object.id == id)
    }
}

pub struct Definition {
    id: DefinitionId,
    name: String,
    script: ScriptEngine,
    has_initialize: bool,
    has_step: bool,
}

impl Definition {
    pub fn from_script(
        id: impl Into<String>,
        name: impl Into<String>,
        source: &str,
    ) -> Result<Self, EngineError> {
        let id = id.into();
        let name = name.into();
        let mut script = ScriptEngine::new();
        script
            .load_script(source)
            .map_err(|source| EngineError::Script {
                definition: id.clone(),
                function: "load",
                source,
            })?;
        let has_initialize = script.has_function("Initialize");
        let has_step = script.has_function("Step");
        Ok(Self {
            id,
            name,
            script,
            has_initialize,
            has_step,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn call_initialize(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        random: i32,
    ) -> Result<CommandBatch, EngineError> {
        if !self.has_initialize {
            return Ok(CommandBatch::default());
        }
        let args = [
            build_state_value(&self.id, object_id, state),
            Value::Int(random),
        ];
        let result =
            self.script
                .call("Initialize", &args)
                .map_err(|source| EngineError::Script {
                    definition: self.id.clone(),
                    function: "Initialize",
                    source,
                })?;
        parse_command(&self.id, "Initialize", result)
    }

    fn call_step(
        &self,
        state: &ObjectState,
        object_id: ObjectId,
        frame: u64,
        random: i32,
    ) -> Result<CommandBatch, EngineError> {
        if !self.has_step {
            return Ok(CommandBatch::default());
        }
        let frame_value = if frame > i32::MAX as u64 {
            i32::MAX
        } else {
            frame as i32
        };
        let args = [
            build_state_value(&self.id, object_id, state),
            Value::Int(frame_value),
            Value::Int(random),
        ];
        let result = self
            .script
            .call("Step", &args)
            .map_err(|source| EngineError::Script {
                definition: self.id.clone(),
                function: "Step",
                source,
            })?;
        parse_command(&self.id, "Step", result)
    }
}

#[derive(Debug, Default)]
struct CommandBatch {
    delta: ObjectDelta,
    spawns: Vec<SpawnConfig>,
    destroy: bool,
}

pub struct Engine {
    definitions: HashMap<DefinitionId, Definition>,
    objects: Vec<Object>,
    next_object_id: u64,
    rng: SmallRng,
    frame: u64,
    landscape: Option<Landscape>,
}

impl Engine {
    pub fn new() -> Self {
        Self::with_seed(0)
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            definitions: HashMap::new(),
            objects: Vec::new(),
            next_object_id: 1,
            rng: SmallRng::seed_from_u64(seed),
            frame: 0,
            landscape: None,
        }
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn set_landscape(&mut self, landscape: Landscape) {
        self.landscape = Some(landscape);
    }

    pub fn clear_landscape(&mut self) {
        self.landscape = None;
    }

    pub fn landscape(&self) -> Option<&Landscape> {
        self.landscape.as_ref()
    }

    pub fn register_definition(&mut self, definition: Definition) -> Result<(), EngineError> {
        let id = definition.id().to_string();
        if self.definitions.contains_key(&id) {
            return Err(EngineError::DefinitionAlreadyExists(id));
        }
        self.definitions.insert(id, definition);
        Ok(())
    }

    pub fn spawn_object(&mut self, config: SpawnConfig) -> Result<ObjectId, EngineError> {
        let (id, additional) = self.spawn_single(config)?;
        self.process_spawn_queue(additional)?;
        Ok(id)
    }

    pub fn tick(&mut self) -> Result<SimulationSnapshot, EngineError> {
        self.frame += 1;
        let frame = self.frame;
        let mut spawn_requests = Vec::new();
        for idx in 0..self.objects.len() {
            {
                let object = &mut self.objects[idx];
                object.state.position += object.state.velocity;
            }

            self.apply_landscape_at_index(idx);

            let object_id = self.objects[idx].id;
            let definition_id = self.objects[idx].definition_id.clone();
            let state_snapshot = self.objects[idx].state.clone();
            let random = self.next_random_i32();

            let command = {
                let definition = self
                    .definitions
                    .get(&definition_id)
                    .ok_or_else(|| EngineError::UnknownDefinition(definition_id.clone()))?;
                definition.call_step(&state_snapshot, object_id, frame, random)?
            };

            {
                let object = &mut self.objects[idx];
                object.state.apply_delta(&command.delta);
                if command.destroy {
                    object.mark_destroyed();
                }
            }
            self.apply_landscape_at_index(idx);
            spawn_requests.extend(command.spawns.into_iter());
        }

        self.objects.retain(|object| !object.destroyed);
        self.process_spawn_queue(spawn_requests)?;
        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> SimulationSnapshot {
        let mut objects: Vec<_> = self.objects.iter().map(Object::snapshot).collect();
        objects.sort_by_key(|object| object.id);
        SimulationSnapshot {
            frame: self.frame,
            objects,
        }
    }

    fn apply_landscape(&self, state: &mut ObjectState) {
        if let Some(landscape) = &self.landscape {
            let resolution = landscape.resolve_collision(state.position, state.velocity);
            if resolution.collided {
                state.position = resolution.position;
                state.velocity = resolution.velocity;
            }
        }
    }

    fn apply_landscape_at_index(&mut self, idx: usize) {
        let resolution = match self.landscape.as_ref() {
            Some(landscape) => {
                let (position, velocity) = {
                    let object = &self.objects[idx];
                    (object.state.position, object.state.velocity)
                };
                landscape.resolve_collision(position, velocity)
            }
            None => return,
        };
        if !resolution.collided {
            return;
        }
        let object = &mut self.objects[idx];
        object.state.position = resolution.position;
        object.state.velocity = resolution.velocity;
    }

    fn next_random_i32(&mut self) -> i32 {
        self.rng.gen()
    }

    fn next_object_id(&mut self) -> ObjectId {
        let id = self.next_object_id;
        self.next_object_id += 1;
        ObjectId::new(id)
    }

    fn spawn_single(
        &mut self,
        config: SpawnConfig,
    ) -> Result<(ObjectId, Vec<SpawnConfig>), EngineError> {
        let def_id = config.definition_id.clone();
        if !self.definitions.contains_key(&def_id) {
            return Err(EngineError::UnknownDefinition(def_id));
        }

        let id = self.next_object_id();
        let mut object = Object::new(
            id,
            config.definition_id.clone(),
            ObjectState {
                position: config.position,
                velocity: config.velocity,
                energy: config.energy,
            },
        );

        let mut additional_spawns = Vec::new();
        if self
            .definitions
            .get(&config.definition_id)
            .map(|definition| definition.has_initialize)
            .unwrap_or(false)
        {
            let random = self.next_random_i32();
            let command = {
                let definition = self
                    .definitions
                    .get(&config.definition_id)
                    .expect("definition must exist");
                definition.call_initialize(&object.state, id, random)?
            };
            if command.destroy {
                return Err(EngineError::InvalidScriptOutput {
                    definition: config.definition_id.clone(),
                    function: "Initialize",
                    detail: "Initialize may not destroy the object".into(),
                });
            }
            object.state.apply_delta(&command.delta);
            additional_spawns = command.spawns;
        }

        self.apply_landscape(&mut object.state);
        self.objects.push(object);
        Ok((id, additional_spawns))
    }

    fn process_spawn_queue(
        &mut self,
        queue: Vec<SpawnConfig>,
    ) -> Result<Vec<ObjectId>, EngineError> {
        let mut pending: VecDeque<_> = queue.into_iter().collect();
        let mut created = Vec::new();
        while let Some(config) = pending.pop_front() {
            let (id, additional) = self.spawn_single(config)?;
            created.push(id);
            for spawn in additional {
                pending.push_back(spawn);
            }
        }
        Ok(created)
    }
}

fn build_state_value(definition_id: &str, object_id: ObjectId, state: &ObjectState) -> Value {
    let mut map = HashMap::with_capacity(5);
    map.insert(
        "definition".into(),
        Value::String(definition_id.to_string()),
    );
    map.insert("id".into(), Value::Int(truncate_to_i32(object_id.as_u64())));
    map.insert("position".into(), state.position.to_value());
    map.insert("velocity".into(), state.velocity.to_value());
    map.insert("energy".into(), Value::Int(state.energy));
    Value::Proplist(map)
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
    function: &'static str,
    value: Value,
) -> Result<CommandBatch, EngineError> {
    match value {
        Value::Nil => Ok(CommandBatch::default()),
        Value::Proplist(map) => parse_command_from_proplist(definition, function, map),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected proplist or nil, got {}", other.type_name()),
        }),
    }
}

fn parse_command_from_proplist(
    definition: &str,
    function: &'static str,
    map: HashMap<String, Value>,
) -> Result<CommandBatch, EngineError> {
    let mut batch = CommandBatch::default();
    for (key, value) in map.into_iter() {
        match key.as_str() {
            "position" => {
                batch.delta.position = Some(value_to_vector(definition, function, value)?);
            }
            "velocity" => {
                batch.delta.velocity = Some(value_to_vector(definition, function, value)?);
            }
            "energy" => {
                batch.delta.energy = Some(value_to_int(definition, function, value)?);
            }
            "destroy" => {
                batch.destroy = value.as_bool();
            }
            "spawn" => {
                batch
                    .spawns
                    .extend(value_to_spawns(definition, function, value)?);
            }
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("unexpected key `{other}`"),
                });
            }
        }
    }
    Ok(batch)
}

fn value_to_vector(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vector2, EngineError> {
    match value {
        Value::Array(values) if values.len() == 2 => {
            let x = match &values[0] {
                Value::Int(v) => *v,
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("expected int for x component, got {}", other.type_name()),
                    })
                }
            };
            let y = match &values[1] {
                Value::Int(v) => *v,
                other => {
                    return Err(EngineError::InvalidScriptOutput {
                        definition: definition.to_string(),
                        function,
                        detail: format!("expected int for y component, got {}", other.type_name()),
                    })
                }
            };
            Ok(Vector2::new(x, y))
        }
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected array[2], got {}", other.type_name()),
        }),
    }
}

fn value_to_int(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<i32, EngineError> {
    match value {
        Value::Int(v) => Ok(v),
        other => Err(EngineError::InvalidScriptOutput {
            definition: definition.to_string(),
            function,
            detail: format!("expected int, got {}", other.type_name()),
        }),
    }
}

fn value_to_spawns(
    definition: &str,
    function: &'static str,
    value: Value,
) -> Result<Vec<SpawnConfig>, EngineError> {
    let array = match value {
        Value::Array(values) => values,
        Value::Nil => return Ok(Vec::new()),
        other => {
            return Err(EngineError::InvalidScriptOutput {
                definition: definition.to_string(),
                function,
                detail: format!("expected array for spawn list, got {}", other.type_name()),
            })
        }
    };

    let mut spawns = Vec::with_capacity(array.len());
    for entry in array.into_iter() {
        let map = match entry {
            Value::Proplist(map) => map,
            other => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("spawn entry must be proplist, got {}", other.type_name()),
                })
            }
        };

        let definition_id = match map.get("definition") {
            Some(Value::String(id)) => id.clone(),
            Some(other) => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: format!("spawn definition must be string, got {}", other.type_name()),
                })
            }
            None => {
                return Err(EngineError::InvalidScriptOutput {
                    definition: definition.to_string(),
                    function,
                    detail: "spawn entry missing `definition`".into(),
                })
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

        spawns.push(SpawnConfig {
            definition_id,
            position,
            velocity,
            energy,
        });
    }

    Ok(spawns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_definition() -> Definition {
        let source = r#"
        global func Initialize(state, random) {
            return { energy = 100 };
        }

        global func Step(state, frame, random) {
            var vx = state.velocity[0] + 1;
            return { velocity = [vx, state.velocity[1]] };
        }
        "#;
        Definition::from_script("Test", "Test", source).expect("script compiles")
    }

    #[test]
    fn applies_velocity_changes_from_step_callback() {
        let mut engine = Engine::with_seed(123);
        engine
            .register_definition(build_definition())
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0))
                    .with_energy(50),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(1, 0));
        assert_eq!(object.velocity, Vector2::new(2, 0));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(3, 0));
        assert_eq!(object.velocity, Vector2::new(3, 0));
    }

    #[test]
    fn spawns_additional_objects_from_step() {
        let source = r#"
        global func Initialize(state, random) {
            return { energy = 42 };
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    spawn = [
                        { definition = "Test", position = [state.position[0] + 5, state.position[1]], velocity = [0, 0], energy = 10 }
                    ]
                };
            }
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(Definition::from_script("Test", "Test", source).unwrap())
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Test"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(0, 0));
        assert_eq!(object.energy, 42);
        assert_eq!(snapshot.objects.len(), 2, "spawned child should exist");

        let spawned = snapshot
            .objects
            .iter()
            .find(|obj| obj.id != id)
            .expect("child object present");
        assert_eq!(spawned.position, Vector2::new(5, 0));
        assert_eq!(spawned.energy, 42);
    }

    #[test]
    fn produces_deterministic_snapshots() {
        let source = r#"
        global func Step(state, frame, random) {
            var new_y = state.position[1] + (random % 3) - 1;
            return { velocity = [state.velocity[0], new_y - state.position[1]] };
        }
        "#;
        let definition = Definition::from_script("Mover", "Mover", source).unwrap();

        let mut engine_a = Engine::with_seed(7);
        engine_a
            .register_definition(definition)
            .expect("definition registers");
        let mut engine_b = Engine::with_seed(7);
        engine_b
            .register_definition(Definition::from_script("Mover", "Mover", source).unwrap())
            .expect("definition registers");

        let id_a = engine_a
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0)),
            )
            .unwrap();
        let id_b = engine_b
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0)),
            )
            .unwrap();

        for _ in 0..5 {
            let snap_a = engine_a.tick().unwrap();
            let snap_b = engine_b.tick().unwrap();
            let obj_a = snap_a.object(id_a).unwrap();
            let obj_b = snap_b.object(id_b).unwrap();
            assert_eq!(obj_a.position, obj_b.position);
            assert_eq!(obj_a.velocity, obj_b.velocity);
        }
    }

    #[test]
    fn clamps_objects_to_landscape_surface() {
        let script = r#"
        global func Step(state, frame, random) {
            return nil;
        }
        "#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(Definition::from_script("Static", "Static", script).unwrap())
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 5));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Static")
                    .with_position(Vector2::new(4, 12))
                    .with_velocity(Vector2::new(0, 3)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 5));
        assert_eq!(object.velocity, Vector2::new(0, 0));
    }

    #[test]
    fn recorder_playback_roundtrip_matches_engine() {
        let mut engine_a = Engine::with_seed(99);
        engine_a
            .register_definition(build_definition())
            .expect("definition registers");
        engine_a.set_landscape(Landscape::flat(32, 0));

        let spawn = SpawnConfig::new("Test")
            .with_position(Vector2::new(0, 0))
            .with_velocity(Vector2::new(1, 0));
        engine_a
            .spawn_object(spawn.clone())
            .expect("spawn succeeds");

        let mut recorder = Recorder::new();
        for _ in 0..5 {
            let snapshot = engine_a.tick().expect("tick succeeds");
            recorder.record(&snapshot);
        }
        let recording = recorder.into_recording();
        let serialized = recording.to_string().expect("serializes");

        let mut playback = Playback::from_str(&serialized).expect("playback loads");

        let mut engine_b = Engine::with_seed(99);
        engine_b
            .register_definition(build_definition())
            .expect("definition registers");
        engine_b.set_landscape(Landscape::flat(32, 0));
        engine_b.spawn_object(spawn).expect("spawn succeeds");

        for _ in 0..5 {
            let snapshot = engine_b.tick().expect("tick succeeds");
            playback
                .validate_snapshot(&snapshot)
                .expect("snapshots match");
        }
        playback.finish().expect("playback completed");
    }
}
