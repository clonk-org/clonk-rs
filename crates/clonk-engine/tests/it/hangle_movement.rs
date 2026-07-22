use clonk_engine::{
    ActionSpec, ActionState, CommandDirection, Definition, Direction, Engine, MovementProfile,
    ObjectUpdate, SpawnConfig,
};
use std::collections::HashMap;

const HANGLE_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
"#;

#[test]
fn hangle_procedure_moves_along_ledges() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("Hangler", "Hangler", HANGLE_SCRIPT)?;
    let mut actions = HashMap::new();
    actions.insert(
        "Hang".to_string(),
        ActionSpec::default().with_procedure("Hang"),
    );
    definition.configure_actions(Some("Hang".to_string()), actions);
    definition.set_movement_profile(
        MovementProfile::default()
            .with_hangle_speed(6)
            .with_hangle_acceleration(3),
    );
    engine.register_definition(definition)?;

    let object_id = engine.spawn_object(
        SpawnConfig::new("Hangler")
            .with_action(ActionState::new("Hang"))
            .with_direction(Direction::Right)
            .with_command_direction(CommandDirection::Right),
    )?;

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object must exist after first hangle tick");
    assert_eq!(object.velocity.x, 3);
    assert_eq!(object.velocity.y, 0);
    assert_eq!(object.direction, Direction::Right);

    engine.apply_object_update(
        object_id,
        ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
    )?;

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object must exist after braking on ledge");
    assert_eq!(object.velocity.x, 0);
    assert_eq!(object.velocity.y, 0);
    assert_eq!(object.direction, Direction::Right);

    engine.apply_object_update(
        object_id,
        ObjectUpdate::new()
            .with_direction(Direction::Left)
            .with_command_direction(CommandDirection::Up),
    )?;

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object must exist after traversing up-left");
    assert_eq!(object.velocity.x, -3);
    assert_eq!(object.velocity.y, 0);
    assert_eq!(object.direction, Direction::Left);

    Ok(())
}
