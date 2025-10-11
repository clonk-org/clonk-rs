use crate::{
    Definition, Engine, EngineError, FloatVector2, Landscape, ObjectStatus, ObjectUpdate,
    ParticleCommand, ParticleConfig, ParticleLayer, ParticleScope, QueuedCommand, Recorder,
    Recording, SpawnConfig, Vector2,
};

const BASIC_MOVEMENT_SCRIPT: &str = r#"
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
    return nil;
}

global func Step(state, frame, random)
{
    return nil;
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
            .with_position(Vector2::new(2, 0))
            .with_velocity(Vector2::new(1, -2))
            .with_energy(64),
    )?;

    let helper_spawn = SpawnConfig::new("Helper")
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
}
