use crate::{Definition, Engine, EngineError, Recorder, Recording, SpawnConfig, Vector2};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_expected_frame_count() {
        let recording = basic_movement_recording(6).expect("recording succeeds");
        assert_eq!(recording.frames().len(), 6);
    }
}
