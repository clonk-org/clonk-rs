// Test that action callbacks can return non-nil values
// In C4Script, action callbacks often return 1 to indicate success

use clonk_engine::{ActionSpec, ActionState, Definition, Engine, SpawnConfig};
use std::collections::HashMap;

#[test]
fn action_callback_can_return_int() {
    let mut engine = Engine::new();

    let mut definition = Definition::from_script(
        "TEST",
        "Test",
        r#"
        global func Initialize(state, random) {
            return 0;
        }

        // This callback returns 1 (like Scaling() in Clonk.c4d)
        protected func TestCallback(state, action) {
            return 1;
        }
        "#,
    )
    .expect("script compiles");

    // Configure action with StartCall
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "TestAction".to_string(),
        ActionSpec::default()
            .with_procedure("Walk")
            .with_start_call("TestCallback")
            .with_length(1)
            .with_step(1),
    );
    definition.configure_actions(Some("Idle".to_string()), actions);

    engine.register_definition(definition).unwrap();

    // Spawn object with TestAction that has StartCall=TestCallback
    let action_state = ActionState::new("TestAction");
    let obj = engine
        .spawn_object(SpawnConfig::new("TEST".to_string()).with_action(action_state))
        .expect("spawn should work");

    // This should NOT fail - action callbacks can return non-nil values
    // The C++ engine allows this behavior
    // The StartCall will be triggered during spawn with the action
    let snapshot = engine.tick().expect("tick should succeed");
    assert!(
        snapshot.object(obj).is_some(),
        "object should exist after tick"
    );
}

#[test]
fn scaling_callback_returns_int() {
    // Reproduces the actual TRPR/Scaling issue
    let mut engine = Engine::new();

    let mut definition = Definition::from_script(
        "CLNK",
        "Clonk",
        r#"
        global func Initialize(state, random) {
            return 0;
        }

        // Actual Scaling function from Clonk.c4d
        protected func Scaling(state, action) {
            return 1;  // Returns int, not nil
        }
        "#,
    )
    .expect("script compiles");

    // Configure Scale action with StartCall=Scaling
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "Scale".to_string(),
        ActionSpec::default()
            .with_procedure("Scale")
            .with_start_call("Scaling")
            .with_length(16)
            .with_step(15),
    );
    definition.configure_actions(Some("Idle".to_string()), actions);

    engine.register_definition(definition).unwrap();

    // Spawn object with Scale action
    let action_state = ActionState::new("Scale");
    let obj = engine
        .spawn_object(SpawnConfig::new("CLNK".to_string()).with_action(action_state))
        .expect("spawn should work - Scaling callback should be allowed to return int");

    // Verify object exists and action works
    let snapshot = engine.tick().expect("tick should succeed");
    assert!(
        snapshot.object(obj).is_some(),
        "object should exist after tick"
    );
}
