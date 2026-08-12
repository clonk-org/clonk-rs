use clonk_engine::{
    ActionSpec, ActionState, CommandDirection, Definition, Engine, MovementProfile, SpawnConfig,
    Vector2, CATEGORY_OBJECT,
};
use std::collections::HashMap;

const FLIGHT_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
"#;

#[test]
fn flight_procedure_applies_gravity() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("Flyer", "Flyer", FLIGHT_SCRIPT)?;
    let mut actions = HashMap::new();
    actions.insert("Jump".to_string(), ActionSpec::for_procedure("Flight"));
    definition.configure_actions(Some("Jump".to_string()), actions);
    definition.set_movement_profile(MovementProfile::default());
    engine.register_definition(definition)?;

    let object_id = engine.spawn_object(
        SpawnConfig::new("Flyer")
            // C4D_Object: StaticBack categories skip gravity and movement
            // (C4Object.cpp:4662, C4Movement.cpp:564).
            .with_category(CATEGORY_OBJECT)
            .with_action(ActionState::new("Jump"))
            .with_velocity(Vector2::new(0, -5))
            .with_command_direction(CommandDirection::Stop),
    )?;

    let snapshot = engine.tick()?;
    let object = crate::support::TestValueExt::test_value(snapshot.object(object_id));
    assert_eq!(object.velocity.x, 0);
    assert_eq!(object.velocity.y, -5);
    assert_eq!(
        object
            .fixed_velocity
            .expect("sub-pixel gravity should be recorded")
            .y
            .val(),
        -327549,
        "flight should accumulate C4Fixed gravity"
    );

    Ok(())
}
