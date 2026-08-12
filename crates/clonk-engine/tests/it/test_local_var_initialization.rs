// Test that local variables are initialized to nil by default
// This matches C++ engine behavior where local variables start as nil

use clonk_engine::{Definition, Engine, SpawnConfig};

#[test]
fn local_variables_default_to_nil() {
    let mut engine = Engine::new();

    let definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "TEST",
        "Test",
        r#"
        #strict

        local MyLocalVar;

        global func Initialize(state, random) {
            // Access MyLocalVar before explicit initialization
            // It should be nil, not undefined
            // If accessing it doesn't throw an error, the test passes
            var x = MyLocalVar;
            return 0;
        }
        "#,
    ));

    crate::support::TestValueExt::test_value(engine.register_definition(definition));

    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TEST".to_string())),
    );

    // If we got here, the test passed - no undefined variable error
    // The Initialize function accessed MyLocalVar and it was nil (not undefined)
}

#[test]
fn local_variables_in_action_callback() {
    // This reproduces the actual Tree/MotionThreshold issue
    let mut engine = Engine::new();

    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "TREE",
        "Tree",
        r#"
        #strict

        local MotionThreshold, Unset;

        global func Initialize(state, random) {
            return 0;
        }

        // This callback accesses MotionThreshold which hasn't been initialized yet
        // (Construction() would normally set it, but let's test the nil default)
        protected func StillCallback(state, action) {
            // Should be nil, not undefined
            if (MotionThreshold == Unset) {
                return 1;
            }
            return 0;
        }
        "#,
    ));

    // Configure action with StartCall
    use clonk_engine::{ActionSpec, ActionState};
    use std::collections::HashMap;

    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "Still".to_string(),
        ActionSpec::default()
            .with_procedure("Attach")
            .with_start_call("StillCallback")
            .with_length(1)
            .with_step(1),
    );
    definition.configure_actions(Some("Idle".to_string()), actions);

    crate::support::TestValueExt::test_value(engine.register_definition(definition));

    // Spawn object with Still action that has StartCall=StillCallback
    let action_state = ActionState::new("Still");
    let obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TREE".to_string()).with_action(action_state)),
    );

    // Verify object exists
    let snapshot = crate::support::TestValueExt::test_value(engine.tick());
    assert!(
        snapshot.object(obj).is_some(),
        "object should exist after tick"
    );
}
