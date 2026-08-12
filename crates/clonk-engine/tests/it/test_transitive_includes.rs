// Test that transitive includes work correctly
// This reproduces the TRE2 -> TRE1 -> TREE -> Breeze() issue

use clonk_engine::{Definition, Engine};

#[test]
fn check_functions_after_resolve() {
    // This test directly checks if functions are present after resolve_includes
    let mut engine = Engine::new();

    let tree = crate::support::TestValueExt::test_value(Definition::from_script(
        "TREE",
        "Tree",
        r#"
        #strict
        private func Breeze() { return 1; }
        "#,
    ));

    let tre1 = crate::support::TestValueExt::test_value(Definition::from_script(
        "TRE1",
        "Tree1",
        r#"
        #strict
        #include TREE
        "#,
    ));

    let plm1 = crate::support::TestValueExt::test_value(Definition::from_script(
        "PLM1",
        "Palm1",
        r#"
        #strict
        #include TRE1
        "#,
    ));

    // Register all definitions
    crate::support::TestValueExt::test_value(engine.register_definition(tree));
    crate::support::TestValueExt::test_value(engine.register_definition(tre1));
    crate::support::TestValueExt::test_value(engine.register_definition(plm1));

    // Resolve includes
    crate::support::TestValueExt::test_value(engine.resolve_includes());

    // Now spawn a PLM1 object and verify Breeze is available
    // by calling it in Initialize
    let test_def = crate::support::TestValueExt::test_value(Definition::from_script(
        "TESTPLM",
        "TestPalm",
        r#"
        #strict
        #include PLM1

        global func Initialize(state, random) {
            // Call Breeze which should be inherited from TREE via TRE1
            var result = Breeze();
            return 0;
        }
        "#,
    ));

    crate::support::TestValueExt::test_value(engine.register_definition(test_def));
    crate::support::TestValueExt::test_value(engine.resolve_includes());

    use clonk_engine::SpawnConfig;
    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TESTPLM".to_string())),
    );
}

#[test]
fn transitive_includes_resolve_correctly() {
    let mut engine = Engine::new();

    // Grandparent: defines the function
    let grandparent = crate::support::TestValueExt::test_value(Definition::from_script(
        "TREE",
        "Tree",
        r#"
        #strict

        local MotionThreshold;

        global func Initialize(state, random) {
            return 0;
        }

        private func Breeze() {
            // This function should be available to TRE2 via TRE1
            return 42;
        }
        "#,
    ));

    // Parent: includes grandparent
    let parent = crate::support::TestValueExt::test_value(Definition::from_script(
        "TRE1",
        "Tree1",
        r#"
        #strict
        #include TREE
        "#,
    ));

    // Child: includes parent (transitive to grandparent)
    let child = crate::support::TestValueExt::test_value(Definition::from_script(
        "TRE2",
        "Tree2",
        r#"
        #strict
        #include TRE1
        "#,
    ));

    // Register in order that might cause issues if not handled correctly
    crate::support::TestValueExt::test_value(engine.register_definition(grandparent));
    crate::support::TestValueExt::test_value(engine.register_definition(child)); // Register child before parent
    crate::support::TestValueExt::test_value(engine.register_definition(parent));

    // Resolve includes - this should handle transitive includes correctly
    crate::support::TestValueExt::test_value(engine.resolve_includes());

    // Now test that TRE2 can call Breeze (inherited transitively)
    // We'll spawn a TRE2 and call a function that uses Breeze
    let test_def = crate::support::TestValueExt::test_value(Definition::from_script(
        "TEST",
        "Test",
        r#"
        #strict
        #include TRE2

        global func Initialize(state, random) {
            // Call Breeze which should be inherited from TREE via TRE1
            var result = Breeze();
            if (result == 42) {
                return 0;  // Success
            }
            return 1;  // Failure
        }
        "#,
    ));

    crate::support::TestValueExt::test_value(engine.register_definition(test_def));
    crate::support::TestValueExt::test_value(engine.resolve_includes());

    // Spawn TEST object - Initialize should succeed without "unknown function 'Breeze'" error
    use clonk_engine::SpawnConfig;
    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TEST".to_string())),
    );
}

#[test]
fn action_callback_with_transitive_include() {
    // This more closely reproduces the actual TRE2/Breeze issue with action StartCall
    let mut engine = Engine::new();

    let grandparent = crate::support::TestValueExt::test_value(Definition::from_script(
        "BASE",
        "Base",
        r#"
        #strict

        global func Initialize(state, random) {
            return 0;
        }

        private func ActionCallback() {
            return 123;
        }
        "#,
    ));

    let parent = crate::support::TestValueExt::test_value(Definition::from_script(
        "PARENT",
        "Parent",
        r#"
        #strict
        #include BASE
        "#,
    ));

    let mut child = crate::support::TestValueExt::test_value(Definition::from_script(
        "CHILD",
        "Child",
        r#"
        #strict
        #include PARENT
        "#,
    ));

    // Configure action with StartCall that uses the inherited function
    use clonk_engine::{ActionSpec, ActionState};
    use std::collections::HashMap;

    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "Test".to_string(),
        ActionSpec::default()
            .with_procedure("Attach")
            .with_start_call("ActionCallback") // Calls function from grandparent
            .with_length(1)
            .with_step(1),
    );
    child.configure_actions(Some("Idle".to_string()), actions);

    crate::support::TestValueExt::test_value(engine.register_definition(grandparent));
    crate::support::TestValueExt::test_value(engine.register_definition(child));
    crate::support::TestValueExt::test_value(engine.register_definition(parent));
    crate::support::TestValueExt::test_value(engine.resolve_includes());

    // Spawn with action that triggers StartCall
    let action_state = ActionState::new("Test");
    let _obj = crate::support::TestValueExt::test_value(engine.spawn_object(
        clonk_engine::SpawnConfig::new("CHILD".to_string()).with_action(action_state),
    ));

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
}
