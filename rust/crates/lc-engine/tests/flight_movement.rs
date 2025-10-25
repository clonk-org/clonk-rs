use lc_engine::{
    ActionSpec, ActionState, CommandDirection, Definition, Engine, MovementProfile, SpawnConfig,
    Vector2,
};
use std::collections::HashMap;

const FLIGHT_SCRIPT: &str = r#"
global func Initialize(state, random) { return nil; }

global func Step(state, frame, random) { return nil; }
"#;

#[test]
fn flight_procedure_applies_gravity() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("Flyer", "Flyer", FLIGHT_SCRIPT)?;
    let mut actions = HashMap::new();
    actions.insert(
        "Jump".to_string(),
        ActionSpec::default().with_procedure("Flight"),
    );
    definition.configure_actions(Some("Jump".to_string()), actions);
    definition.set_movement_profile(MovementProfile::default());
    engine.register_definition(definition)?;

    let object_id = engine.spawn_object(
        SpawnConfig::new("Flyer")
            .with_action(ActionState::new("Jump"))
            .with_velocity(Vector2::new(0, -5))
            .with_command_direction(CommandDirection::Stop),
    )?;

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object present after tick");
    assert_eq!(object.velocity.x, 0);
    assert_eq!(object.velocity.y, -4, "flight should accumulate gravity");

    Ok(())
}
