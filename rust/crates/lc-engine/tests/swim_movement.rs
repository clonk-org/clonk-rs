use lc_engine::{
    ActionSpec, ActionState, CommandDirection, Definition, Engine, MovementProfile, ObjectUpdate,
    SpawnConfig,
};
use std::collections::HashMap;

const SWIM_SCRIPT: &str = r#"
global func Initialize(state, random) { return nil; }

global func Step(state, frame, random) { return nil; }
"#;

#[test]
fn swim_procedure_handles_direction_and_drift() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("Swimmer", "Swimmer", SWIM_SCRIPT)?;
    let mut actions = HashMap::new();
    actions.insert(
        "Swim".to_string(),
        ActionSpec::default().with_procedure("Swim"),
    );
    definition.configure_actions(Some("Swim".to_string()), actions);
    definition.set_movement_profile(
        MovementProfile::default()
            .with_swim_speed(8)
            .with_swim_acceleration(2),
    );
    engine.register_definition(definition)?;

    let object_id = engine.spawn_object(
        SpawnConfig::new("Swimmer")
            .with_action(ActionState::new("Swim"))
            .with_command_direction(CommandDirection::UpRight),
    )?;

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object must exist after swimming up-right");
    assert_eq!(object.velocity.x, 2);
    assert_eq!(object.velocity.y, -2);
    assert_eq!(
        object
            .fixed_velocity
            .expect("swim gravity remains sub-pixel")
            .y
            .val(),
        -131006
    );

    engine.apply_object_update(
        object_id,
        ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
    )?;

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object must exist after drifting in water");
    assert_eq!(object.velocity.x, 0);
    assert_eq!(object.velocity.y, 0);
    assert_eq!(
        object
            .fixed_velocity
            .expect("swim drift keeps fixed gravity")
            .y
            .val(),
        66
    );

    Ok(())
}
