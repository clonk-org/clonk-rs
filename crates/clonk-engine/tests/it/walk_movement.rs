use clonk_engine::{
    ActionSpec, ActionState, CommandDirection, Definition, Direction, Engine, MovementProfile,
    ObjectUpdate, SpawnConfig,
};
use std::collections::HashMap;

const WALKER_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
"#;

#[test]
fn walk_procedure_accelerates_and_brakes() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("Walker", "Walker", WALKER_SCRIPT)?;
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        // C4ActionDef::Directions defaults to one, which only admits
        // DIR_Left. This two-facing WALK fixture must declare both slots so
        // C4Object::SetDir accepts DIR_Right (C4Object.cpp:4239-4242).
        ActionSpec::default()
            .with_procedure("WALK")
            .with_directions(2),
    );
    definition.configure_actions(Some("Walk".to_string()), actions);
    let profile = MovementProfile::default()
        .with_walk_speed(8)
        .with_walk_acceleration(3);
    definition.set_movement_profile(profile);
    engine.register_definition(definition)?;

    let object_id = engine.spawn_object(
        SpawnConfig::new("Walker")
            .with_action(ActionState::new("Walk"))
            .with_energy(10)
            .with_command_direction(CommandDirection::Right),
    )?;

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 3);
    assert_eq!(object.direction, Direction::Right);

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 6);
    assert_eq!(object.direction, Direction::Right);

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 8);
    assert_eq!(object.direction, Direction::Right);

    engine.apply_object_update(
        object_id,
        ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
    )?;

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 5);
    assert_eq!(object.direction, Direction::Right);

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 2);

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 0);

    engine.apply_object_update(
        object_id,
        ObjectUpdate::new().with_command_direction(CommandDirection::Left),
    )?;

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, -3);
    assert_eq!(object.direction, Direction::Left);

    Ok(())
}

#[test]
fn walkto_action_uses_walk_procedure() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("Walker", "Walker", WALKER_SCRIPT)?;
    let mut actions = HashMap::new();
    actions.insert(
        "WalkTo".to_string(),
        // Procedure names are mapped case-sensitively and right-facing
        // motion requires Directions=2 in the C++ ActMap.
        ActionSpec::default()
            .with_procedure("WALK")
            .with_directions(2),
    );
    definition.configure_actions(Some("WalkTo".to_string()), actions);
    definition.set_movement_profile(
        MovementProfile::default()
            .with_walk_speed(6)
            .with_walk_acceleration(2),
    );
    engine.register_definition(definition)?;

    let object_id = engine.spawn_object(
        SpawnConfig::new("Walker")
            .with_action(ActionState::new("WalkTo"))
            .with_command_direction(CommandDirection::Right),
    )?;

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 2);
    assert_eq!(object.direction, Direction::Right);

    Ok(())
}
