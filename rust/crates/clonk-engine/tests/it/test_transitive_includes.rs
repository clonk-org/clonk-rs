// Test that transitive includes work correctly
// This reproduces the TRE2 -> TRE1 -> TREE -> Breeze() issue

use clonk_engine::{Definition, Engine};

#[test]
fn check_functions_after_resolve() {
    // This test directly checks if functions are present after resolve_includes
    let mut engine = Engine::new();

    let tree = Definition::from_script(
        "TREE",
        "Tree",
        r#"
        #strict
        private func Breeze() { return 1; }
        "#,
    )
    .expect("tree compiles");

    let tre1 = Definition::from_script(
        "TRE1",
        "Tree1",
        r#"
        #strict
        #include TREE
        "#,
    )
    .expect("tre1 compiles");

    let plm1 = Definition::from_script(
        "PLM1",
        "Palm1",
        r#"
        #strict
        #include TRE1
        "#,
    )
    .expect("plm1 compiles");

    // Register all definitions
    engine.register_definition(tree).unwrap();
    engine.register_definition(tre1).unwrap();
    engine.register_definition(plm1).unwrap();

    // Resolve includes
    engine.resolve_includes().expect("includes resolve");

    // Now spawn a PLM1 object and verify Breeze is available
    // by calling it in Initialize
    let test_def = Definition::from_script(
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
    )
    .expect("test definition compiles");

    engine.register_definition(test_def).unwrap();
    engine.resolve_includes().expect("test includes resolve");

    use clonk_engine::SpawnConfig;
    let _obj = engine
        .spawn_object(SpawnConfig::new("TESTPLM".to_string()))
        .expect("spawn should succeed - Breeze should be found via PLM1's transitive includes");
}

#[test]
fn transitive_includes_resolve_correctly() {
    let mut engine = Engine::new();

    // Grandparent: defines the function
    let grandparent = Definition::from_script(
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
    )
    .expect("grandparent compiles");

    // Parent: includes grandparent
    let parent = Definition::from_script(
        "TRE1",
        "Tree1",
        r#"
        #strict
        #include TREE
        "#,
    )
    .expect("parent compiles");

    // Child: includes parent (transitive to grandparent)
    let child = Definition::from_script(
        "TRE2",
        "Tree2",
        r#"
        #strict
        #include TRE1
        "#,
    )
    .expect("child compiles");

    // Register in order that might cause issues if not handled correctly
    engine.register_definition(grandparent).unwrap();
    engine.register_definition(child).unwrap(); // Register child before parent
    engine.register_definition(parent).unwrap();

    // Resolve includes - this should handle transitive includes correctly
    engine.resolve_includes().expect("includes resolve");

    // Now test that TRE2 can call Breeze (inherited transitively)
    // We'll spawn a TRE2 and call a function that uses Breeze
    let test_def = Definition::from_script(
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
    )
    .expect("test definition compiles");

    engine.register_definition(test_def).unwrap();
    engine.resolve_includes().expect("test includes resolve");

    // Spawn TEST object - Initialize should succeed without "unknown function 'Breeze'" error
    use clonk_engine::SpawnConfig;
    let _obj = engine
        .spawn_object(SpawnConfig::new("TEST".to_string()))
        .expect("spawn should succeed - Breeze should be found via transitive includes");
}

#[test]
fn action_callback_with_transitive_include() {
    // This more closely reproduces the actual TRE2/Breeze issue with action StartCall
    let mut engine = Engine::new();

    let grandparent = Definition::from_script(
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
    )
    .expect("grandparent compiles");

    let parent = Definition::from_script(
        "PARENT",
        "Parent",
        r#"
        #strict
        #include BASE
        "#,
    )
    .expect("parent compiles");

    let mut child = Definition::from_script(
        "CHILD",
        "Child",
        r#"
        #strict
        #include PARENT
        "#,
    )
    .expect("child compiles");

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

    engine.register_definition(grandparent).unwrap();
    engine.register_definition(child).unwrap();
    engine.register_definition(parent).unwrap();
    engine.resolve_includes().expect("includes resolve");

    // Spawn with action that triggers StartCall
    let action_state = ActionState::new("Test");
    let _obj = engine
        .spawn_object(clonk_engine::SpawnConfig::new("CHILD".to_string()).with_action(action_state))
        .expect("spawn should work - ActionCallback should be found via transitive includes");

    engine.tick_without_snapshot().expect("tick should succeed");
}
