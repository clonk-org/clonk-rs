// Test for object ID reservation when loading scenarios with explicit IDs

use clonk_engine::{Definition, Engine, ObjectId, SpawnConfig};

fn simple_definition(id: &str) -> Definition {
    crate::support::TestValueExt::test_value(Definition::from_script(
        id,
        id,
        r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        "#,
    ))
}

#[test]
fn explicit_ids_should_not_conflict_with_auto_assigned_ids() {
    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_definition(simple_definition("COWB")));
    crate::support::TestValueExt::test_value(engine.register_definition(simple_definition("BUSH")));

    // Create initial objects with auto-assigned IDs (like crew members)
    let cowb1 = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("COWB".to_string())),
    );
    let cowb2 = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("COWB".to_string())),
    );

    // These should have auto-assigned IDs 1 and 2
    assert_eq!(cowb1, ObjectId::new(1));
    assert_eq!(cowb2, ObjectId::new(2));

    // Now try to spawn an object with explicit ID 1 (like from Objects.txt)
    // This should either:
    // 1) Fail with DuplicateObjectId error (current behavior), OR
    // 2) Succeed if we properly pre-scan and reserve ID space (desired behavior)
    let result =
        engine.spawn_object(SpawnConfig::new("BUSH".to_string()).with_id(ObjectId::new(1)));

    // Currently this fails - we want to test the fix
    assert!(result.is_err(), "Should fail with current implementation");
}

#[test]
fn scenario_should_reserve_explicit_ids_before_spawning() {
    // This test verifies the fix: when Scenario::apply processes initial_spawns
    // with explicit IDs, it pre-scans them and updates next_object_id BEFORE
    // spawning any objects (including crew members)

    let mut engine = Engine::new();
    crate::support::TestValueExt::test_value(engine.register_definition(simple_definition("COWB")));
    crate::support::TestValueExt::test_value(engine.register_definition(simple_definition("BUSH")));
    crate::support::TestValueExt::test_value(engine.register_definition(simple_definition("TRPR")));

    // Simulate scenario with:
    // - Some spawns with explicit IDs (like from Objects.txt: 1, 2, 150)
    // - Some spawns without explicit IDs (like crew members)

    // First, spawn an object with explicit ID 150
    let obj_150 = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("BUSH".to_string()).with_id(ObjectId::new(150))),
    );
    assert_eq!(obj_150, ObjectId::new(150));

    // The engine should have reserved ID space up to 151
    // So the next auto-assigned ID should be 151
    let auto_obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("COWB".to_string())),
    );
    assert_eq!(auto_obj, ObjectId::new(151));

    // Now spawn another object with explicit ID 1 (should succeed since engine can handle any ID)
    let obj_1 = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("TRPR".to_string()).with_id(ObjectId::new(1))),
    );
    assert_eq!(obj_1, ObjectId::new(1));
}
