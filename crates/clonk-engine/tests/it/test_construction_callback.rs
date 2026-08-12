// Test that Construction() callback is called before action StartCall callbacks
// This reproduces the PLM1/TREE MotionThreshold issue

use clonk_engine::{ActionSpec, ActionState, Definition, Engine, SpawnConfig};
use std::collections::HashMap;

#[test]
fn construction_called_before_action_start() {
    let mut engine = Engine::new();

    // Create a definition with:
    // - local variable initialized in Construction()
    // - action StartCall that uses the variable
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "TEST",
        "Test",
        r#"
        #strict

        local MyValue;

        protected func Construction() {
            MyValue = 42;
            return 0;
        }

        global func Initialize(state, random) {
            return 0;
        }

        protected func OnStart() {
            // This should work if Construction() was called
            // Should fail with "cannot apply '+' to operands of type int and nil" if not
            var result = 10 + MyValue;
            return 0;
        }
        "#,
    ));

    // Configure an action with StartCall
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "Test".to_string(),
        ActionSpec::default()
            .with_procedure("Attach")
            .with_start_call("OnStart")
            .with_length(1)
            .with_step(1),
    );
    definition.configure_actions(Some("Idle".to_string()), actions);

    crate::support::TestValueExt::test_value(engine.register_definition(definition));

    // Spawn object with the Test action that has StartCall=OnStart
    let action_state = ActionState::new("Test");
    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TEST".to_string()).with_action(action_state)),
    );
}

#[test]
fn construction_initializes_local_vars_for_tree() {
    // This directly reproduces the TREE/PLM1 MotionThreshold issue
    let mut engine = Engine::new();

    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "TREE",
        "Tree",
        r#"
        #strict

        local MotionThreshold;

        protected func Construction() {
            MotionThreshold = 5;
            return 0;
        }

        global func Initialize(state, random) {
            return 0;
        }

        protected func Still() {
            // This is what PLM1's Still action does
            // Should fail if MotionThreshold is nil
            var threshold = 49 + MotionThreshold;
            return 0;
        }
        "#,
    ));

    // Configure Still action with StartCall like PLM1
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "Still".to_string(),
        ActionSpec::default()
            .with_procedure("Attach")
            .with_start_call("Still")
            .with_length(1)
            .with_delay(50),
    );
    definition.configure_actions(Some("Idle".to_string()), actions);

    crate::support::TestValueExt::test_value(engine.register_definition(definition));

    // Spawn with Still action
    let action_state = ActionState::new("Still");
    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TREE".to_string()).with_action(action_state)),
    );
}
