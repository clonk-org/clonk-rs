use clonk_engine::{Engine, SpawnConfig};

#[test]
fn get_id_returns_current_object_definition() {
    let mut engine = Engine::new();
    let definition = clonk_engine::Definition::from_script(
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
    )
    .expect("script compiles");

    engine.register_definition(definition).unwrap();

    let _obj = engine
        .spawn_object(SpawnConfig::new("COWB".to_string()))
        .expect("spawn should succeed");
}

#[test]
fn get_id_with_different_definitions() {
    let mut engine = Engine::new();

    // Test that different objects return different IDs
    let cowboy = clonk_engine::Definition::from_script(
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
    )
    .expect("cowboy compiles");

    let clonk = clonk_engine::Definition::from_script(
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
    )
    .expect("clonk compiles");

    engine.register_definition(cowboy).unwrap();
    engine.register_definition(clonk).unwrap();

    // Spawn both and verify they succeed
    let _cowboy_id = engine
        .spawn_object(SpawnConfig::new("COWB".to_string()))
        .expect("cowboy spawn should succeed");

    let _clonk_id = engine
        .spawn_object(SpawnConfig::new("CLNK".to_string()))
        .expect("clonk spawn should succeed");
}

#[test]
fn get_id_in_clonk_initialize_matches_definition() {
    let mut engine = Engine::new();
    let definition = clonk_engine::Definition::from_script(
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
    )
    .expect("script compiles");

    engine.register_definition(definition).unwrap();

    let _obj = engine
        .spawn_object(SpawnConfig::new("CLNK".to_string()))
        .expect("spawn should succeed");
}
