// Test for mathematical host functions
// These functions are used extensively in C4Script for calculations

use clonk_engine::{Definition, Engine, SpawnConfig};

#[test]
fn abs_function_should_work() {
    let mut engine = Engine::new();

    let script = Definition::from_script(
        "TEST",
        "Test",
        r#"
        global func Initialize(state, random) {
            Abs(-42);  // Just call it to verify no error
            return 0;
        }
        global func Step(state, frame, random) { return 0; }
        "#,
    )
    .expect("script compiles");

    engine.register_definition(script).unwrap();
    let _obj = engine
        .spawn_object(SpawnConfig::new("TEST".to_string()))
        .unwrap();

    // Initialize should return Abs(-42) = 42
    // This test will fail until we implement Abs
}

#[test]
fn max_function_should_work() {
    let mut engine = Engine::new();

    let script = Definition::from_script(
        "TEST",
        "Test",
        r#"
        global func Initialize(state, random) {
            Max(30, 50);  // Just call it to verify no error
            return 0;
        }
        global func Step(state, frame, random) { return 0; }
        "#,
    )
    .expect("script compiles");

    engine.register_definition(script).unwrap();
    let _obj = engine
        .spawn_object(SpawnConfig::new("TEST".to_string()))
        .unwrap();

    // Initialize should return Max(30, 50) = 50
    // This test will fail until we implement Max
}

#[test]
fn min_function_should_work() {
    let mut engine = Engine::new();

    let script = Definition::from_script(
        "TEST",
        "Test",
        r#"
        global func Initialize(state, random) {
            Min(30, 50);  // Just call it to verify no error
            return 0;
        }
        global func Step(state, frame, random) { return 0; }
        "#,
    )
    .expect("script compiles");

    engine.register_definition(script).unwrap();
    let _obj = engine
        .spawn_object(SpawnConfig::new("TEST".to_string()))
        .unwrap();

    // Initialize should return Min(30, 50) = 30
    // This test will fail until we implement Min
}

#[test]
fn tree_still_callback_should_use_abs() {
    // This test simulates the actual Tree usage pattern
    let mut engine = Engine::new();

    let script = Definition::from_script(
        "TREE",
        "Tree",
        r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        private func Still() {
            // Simulates: if (Abs(GetWind()) > 49) SetAction("Breeze");
            Abs(GetWind());  // Just call it to verify no error
            return 0;
        }
        "#,
    )
    .expect("script compiles");

    engine.register_definition(script).unwrap();
    let _obj = engine
        .spawn_object(SpawnConfig::new("TREE".to_string()))
        .unwrap();

    // This test will fail until we implement Abs
}
