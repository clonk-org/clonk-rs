use crate::{
    ActionState, C4Fixed, Definition, Engine, EngineError, EnvironmentSettings, FixedVec2,
    FloatVector2, Landscape, ObjectStatus, ObjectUpdate, ParticleCommand, ParticleConfig,
    ParticleLayer, ParticleScope, QueuedCommand, Recorder, Recording, RgbColor, SpawnConfig,
    Vector2, CATEGORY_OBJECT,
};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static RESOURCE_FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct SnapshotScenario {
    pub name: &'static str,
    pub default_frames: usize,
    pub generator: fn(usize) -> Result<Recording, EngineError>,
}

const BASIC_MOVEMENT_SCRIPT: &str = r#"
#strict 3
global func Initialize(state, random)
{
    return { energy = state.energy + 5 };
}

global func Step(state, frame, random)
{
    var new_vy = state.velocity[1];
    if (state.position[1] >= 30 && new_vy >= 0)
    {
        new_vy = -4;
    }

    var spawn_list = nil;
    if (frame == 3)
    {
        spawn_list = [
            { definition = "Mover", position = [state.position[0] - 5, 0], velocity = [0, 0], energy = 20 }
        ];
    }

    var should_destroy = frame >= 6;
    var next_energy = state.energy - 1;
    if (next_energy < 0)
    {
        next_energy = 0;
    }

    return {
        velocity = [state.velocity[0], new_vy],
        energy = next_energy,
        spawn = spawn_list,
        destroy = should_destroy,
    };
}
"#;

const PASSIVE_SCRIPT: &str = r#"
global func Initialize(state, random)
{
    return 0;
}

global func Step(state, frame, random)
{
    return 0;
}
"#;

/// Generate a deterministic recording of the `Mover` definition interacting with
/// gravity, spawning a helper, and eventually cleaning up.
pub fn basic_movement_recording(frames: usize) -> Result<Recording, EngineError> {
    let mut engine = Engine::with_seed(12345);
    let definition = Definition::from_script("Mover", "Mover", BASIC_MOVEMENT_SCRIPT)?;
    engine.register_definition(definition)?;
    engine.spawn_object(
        SpawnConfig::new("Mover")
            // C4D_Object: default-category (StaticBack) placements never run
            // ExecMovement (C4Movement.cpp:564) — the fixture must keep
            // exercising integration.
            .with_category(CATEGORY_OBJECT)
            .with_position(Vector2::new(-10, 0))
            .with_velocity(Vector2::new(3, -4))
            .with_energy(80),
    )?;

    let mut recorder = Recorder::new();
    for _ in 0..frames {
        let snapshot = engine.tick()?;
        recorder.record(&snapshot);
    }

    Ok(recorder.into_recording())
}

/// Generate a deterministic recording that exercises the command queue as well as
/// particle creation/clearing and object destruction.
pub fn queued_command_recording(frames: usize) -> Result<Recording, EngineError> {
    let mut engine = Engine::with_seed(424242);
    let commander = Definition::from_script("Commander", "Commander", PASSIVE_SCRIPT)?;
    let helper = Definition::from_script("Helper", "Helper", PASSIVE_SCRIPT)?;
    engine.register_definition(commander)?;
    engine.register_definition(helper)?;
    engine.set_landscape(Landscape::flat(48, 6));

    let commander_id = engine.spawn_object(
        SpawnConfig::new("Commander")
            .with_category(CATEGORY_OBJECT)
            .with_position(Vector2::new(2, 0))
            .with_velocity(Vector2::new(1, -2))
            .with_energy(64),
    )?;

    let helper_spawn = SpawnConfig::new("Helper")
        .with_category(CATEGORY_OBJECT)
        .with_position(Vector2::new(6, 0))
        .with_velocity(Vector2::new(0, -1))
        .with_energy(24)
        .with_owner(1);

    let particle_create = ParticleCommand::Create(ParticleConfig {
        definition_id: "Spark".into(),
        position: FloatVector2::new(0.0, -1.5),
        velocity: FloatVector2::new(0.25, -0.5),
        life: 4,
        parameter_a: 0.75,
        parameter_b: 3,
        layer: ParticleLayer::ObjectFront(commander_id),
    });

    let queue_spawn = QueuedCommand::new(
        1,
        ObjectUpdate::new()
            .with_velocity(Vector2::new(3, -3))
            .with_energy(58),
    )
    .with_spawns(vec![helper_spawn.clone()])
    .with_particles(vec![particle_create]);
    engine.queue_object_command(commander_id, queue_spawn)?;

    let particle_clear = ParticleCommand::Clear {
        definition_id: Some("Spark".into()),
        scope: ParticleScope::Object(commander_id),
    };

    let queue_destroy = QueuedCommand::new(
        3,
        ObjectUpdate::new()
            .with_owner(2)
            .with_status(ObjectStatus::Inactive),
    )
    .with_particles(vec![particle_clear])
    .with_destroy(true);
    engine.queue_object_command(commander_id, queue_destroy)?;

    let mut recorder = Recorder::new();
    for _ in 0..frames {
        let snapshot = engine.tick()?;
        recorder.record(&snapshot);
    }

    Ok(recorder.into_recording())
}

/// Generate a deterministic recording that exercises dynamic environment updates such as
/// wind drift, time-of-day advance, temperature cycles, and precipitation.
pub fn environment_cycle_recording(frames: usize) -> Result<Recording, EngineError> {
    let mut engine = Engine::with_seed(7_654_321);

    let environment = EnvironmentSettings::new(6)
        .with_wind_variation(4, 9)
        .with_time_of_day(1_200)
        .with_time_speed(35)
        .with_precipitation(42)
        .with_temperature(18)
        .with_temperature_cycle(12, 18, 6)
        .with_sky_color(RgbColor::new(24, 48, 96));
    engine.set_environment(environment);
    engine.set_landscape(Landscape::flat(96, 18));

    let drifter = Definition::from_script("Drifter", "Drifter", PASSIVE_SCRIPT)?;
    engine.register_definition(drifter)?;

    engine.spawn_object(
        SpawnConfig::new("Drifter")
            .with_position(Vector2::new(20, 4))
            .with_velocity(Vector2::new(0, 0))
            .with_energy(72)
            .with_owner(1),
    )?;

    let mut recorder = Recorder::new();
    for _ in 0..frames {
        let snapshot = engine.tick()?;
        recorder.record(&snapshot);
    }

    Ok(recorder.into_recording())
}

fn resource_fixture_error(stage: &str, error: impl std::fmt::Display) -> EngineError {
    EngineError::invalid_script_output(
        "FXP1",
        "resource_float_snapshot",
        format!("{stage}: {error}"),
    )
}

fn resource_float_definition() -> Result<Definition, EngineError> {
    let sequence = RESOURCE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "clonk-engine-resource-float-{}-{sequence}",
        std::process::id()
    ));
    let result = (|| {
        fs::create_dir(&root).map_err(|error| resource_fixture_error("create fixture", error))?;
        fs::write(
            root.join("DefCore.txt"),
            b"[DefCore]\nid=FXP1\nName=Wolke\n",
        )
        .map_err(|error| resource_fixture_error("write DefCore", error))?;
        fs::write(
            root.join("ActMap.txt"),
            b"[Action]\nName=Process\nProcedure=FLOAT\nLength=15\nDelay=2\nNextAction=Process\n",
        )
        .map_err(|error| resource_fixture_error("write ActMap", error))?;
        fs::write(root.join("Script.c"), b"#strict\n")
            .map_err(|error| resource_fixture_error("write Script", error))?;

        let group = clonk_resources::Group::open(&root)
            .map_err(|error| resource_fixture_error("open resource group", error))?;
        let resource = clonk_resources::ResourceDefinition::load(&group)
            .map_err(|error| resource_fixture_error("load resource definition", error))?;
        Definition::from_resource(&resource)
    })();
    let _ = fs::remove_dir_all(&root);
    result
}

/// Generate a recording of a real-resource definition with no `[Physical]`
/// section. C++'s zero-default `Physical.Float` must clamp DFA_FLOAT's raw
/// velocity to zero (C4InfoCore.cpp:239-242; C4Object.cpp:5291-5310).
pub fn resource_float_zero_recording(frames: usize) -> Result<Recording, EngineError> {
    let mut engine = Engine::with_seed(24680);
    engine.register_definition(resource_float_definition()?)?;
    engine.spawn_object(
        SpawnConfig::new("FXP1")
            .with_category(CATEGORY_OBJECT)
            .with_mobile(true)
            .with_action(ActionState::new("Process"))
            .with_fixed_velocity(FixedVec2::new(
                C4Fixed::from_raw(123_456),
                C4Fixed::from_raw(-654_321),
            )),
    )?;

    let mut recorder = Recorder::new();
    for _ in 0..frames {
        recorder.record(&engine.tick()?);
    }
    Ok(recorder.into_recording())
}

pub const SNAPSHOT_SCENARIOS: &[SnapshotScenario] = &[
    SnapshotScenario {
        name: "basic_movement",
        default_frames: 6,
        generator: basic_movement_recording,
    },
    SnapshotScenario {
        name: "queued_commands",
        default_frames: 6,
        generator: queued_command_recording,
    },
    SnapshotScenario {
        name: "environment_cycle",
        default_frames: 8,
        generator: environment_cycle_recording,
    },
    SnapshotScenario {
        name: "resource_float_zero",
        default_frames: 3,
        generator: resource_float_zero_recording,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_expected_frame_count() {
        let recording = basic_movement_recording(6).expect("recording succeeds");
        assert_eq!(recording.frames().len(), 6);
    }

    #[test]
    fn queued_command_recording_produces_expected_length() {
        let recording = queued_command_recording(6).expect("recording succeeds");
        assert_eq!(recording.frames().len(), 6);
    }

    #[test]
    fn environment_cycle_recording_produces_expected_length() {
        let recording = environment_cycle_recording(8).expect("recording succeeds");
        assert_eq!(recording.frames().len(), 8);
    }

    #[test]
    fn resource_float_zero_recording_produces_expected_length() {
        let recording = resource_float_zero_recording(3).expect("recording succeeds");
        assert_eq!(recording.frames().len(), 3);
    }
}
