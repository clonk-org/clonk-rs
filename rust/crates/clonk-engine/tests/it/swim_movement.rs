use clonk_engine::{
    ActionSpec, ActionState, CommandDirection, Definition, Engine, MovementProfile, ObjectUpdate,
    SpawnConfig, CATEGORY_OBJECT,
};
use std::collections::HashMap;

const SWIM_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
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
            // C4D_Object: StaticBack categories skip gravity and movement
            // (C4Object.cpp:4662, C4Movement.cpp:564).
            .with_category(CATEGORY_OBJECT)
            .with_action(ActionState::new("Swim"))
            .with_command_direction(CommandDirection::UpRight),
    )?;

    // C4Object InLiquid: this fixture has no water — arm the flag so the
    // DFA_SWIM out-of-liquid exit (C4Object.cpp:4946-4956) does not
    // convert the swimmer to Walk.
    engine.debug_set_in_liquid(object_id, true);

    let snapshot = engine.tick()?;
    let object = snapshot
        .object(object_id)
        .expect("object must exist after swimming up-right");
    assert_eq!(object.velocity.x, 2);
    assert_eq!(object.velocity.y, -2);
    // DFA_SWIM steers with SwimAccel only — no GravAccel component
    // (C4Object.cpp:4920-4985): the velocity is the pure accumulated
    // acceleration.
    assert_eq!(
        object
            .fixed_velocity
            .map(|velocity| velocity.y.val())
            .unwrap_or(object.velocity.y << 16),
        -131072
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
    // With no gravity component on DFA_SWIM the Stop deceleration comes
    // to a full rest (C4Object.cpp:4941-4947).
    assert_eq!(
        object
            .fixed_velocity
            .map(|velocity| velocity.y.val())
            .unwrap_or(0),
        0
    );

    Ok(())
}
