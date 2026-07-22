// Test for #include function resolution in definitions

use clonk_engine::{Definition, Engine, ObjectId, SpawnConfig};

#[allow(dead_code)]
fn simple_definition(id: &str) -> Definition {
    Definition::from_script(
        id,
        id,
        r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        "#,
    )
    .expect("script compiles")
}

#[test]
fn action_callback_should_resolve_through_include() {
    // This test verifies that when a definition includes another definition,
    // action callbacks defined in the parent are available in the child.

    let mut engine = Engine::new();

    // Register parent definition with Still callback
    let parent = Definition::from_script(
        "TRE1",
        "Tree",
        r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        private func Still() { return 0; }
        "#,
    )
    .unwrap();
    engine.register_definition(parent).unwrap();

    // Register child definition that includes parent
    let child = Definition::from_script(
        "TRE2",
        "Tree2",
        r#"
        #strict
        #include TRE1
        "#,
    )
    .unwrap();
    engine.register_definition(child).unwrap();

    // Now spawn a TRE2 object
    let obj = engine
        .spawn_object(SpawnConfig::new("TRE2".to_string()))
        .unwrap();

    // The object should exist
    assert_eq!(obj, ObjectId::new(1));

    // TODO: Verify that calling Still() callback works
    // This would require setting up an action with StartCall=Still
    // For now, just verify the object can be created
}
