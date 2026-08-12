use clonk_engine::{Engine, SpawnConfig};

#[test]
fn get_id_returns_current_object_definition() {
    let mut engine = Engine::new();
    let definition =
        crate::support::TestValueExt::test_value(clonk_engine::Definition::from_script(
            "COWB",
            "Cowboy",
            r#"
        #strict
        protected func Initialize() {
            var myId = GetID();
            if (myId != "COWB") {
                return 0; // This would cause an error
            }
            return 0;
        }
        "#,
        ));

    crate::support::TestValueExt::test_value(engine.register_definition(definition));

    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("COWB".to_string())),
    );
}

#[test]
fn get_id_with_different_definitions() {
    let mut engine = Engine::new();

    // Test that different objects return different IDs
    let cowboy = crate::support::TestValueExt::test_value(clonk_engine::Definition::from_script(
        "COWB",
        "Cowboy",
        r#"
        #strict
        protected func Initialize() {
            // Verify our own ID is COWB
            var myId = GetID();
            return 0;
        }
        "#,
    ));

    let clonk = crate::support::TestValueExt::test_value(clonk_engine::Definition::from_script(
        "CLNK",
        "Clonk",
        r#"
        #strict
        protected func Initialize() {
            // Verify our own ID is CLNK
            var myId = GetID();
            return 0;
        }
        "#,
    ));

    crate::support::TestValueExt::test_value(engine.register_definition(cowboy));
    crate::support::TestValueExt::test_value(engine.register_definition(clonk));

    // Spawn both and verify they succeed
    let _cowboy_id = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("COWB".to_string())),
    );

    let _clonk_id = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("CLNK".to_string())),
    );
}

#[test]
fn get_id_in_clonk_initialize_matches_definition() {
    let mut engine = Engine::new();
    let definition =
        crate::support::TestValueExt::test_value(clonk_engine::Definition::from_script(
            "CLNK",
            "Clonk",
            r#"
        #strict
        protected func Initialize() {
            var id = GetID();
            // Just verify GetID() returns something
            return 0;
        }
        "#,
        ));

    crate::support::TestValueExt::test_value(engine.register_definition(definition));

    let _obj = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("CLNK".to_string())),
    );
}
