use clonk_engine::{Definition, Engine, SpawnConfig};
use clonk_script::Value;

#[test]
fn global_script_reports_old_style_local_without_poisoning_the_function() {
    let mut engine = Engine::new();
    let loaded = engine.install_global_scripts(&[(
        "System.c4g/BodyDeclarations.c".to_string(),
        r#"#strict
global Broken:
    local forbidden;
    return(9);
global Healthy:
    return(7);
"#
        .to_string(),
    )]);
    assert_eq!(loaded, 1, "the global script remains recoverable");
    assert!(engine.debug_global_has_function("Broken"));
    assert!(engine.debug_global_has_function("Healthy"));

    crate::support::TestValueExt::test_value(engine.register_definition(
        crate::support::TestValueExt::test_value(Definition::from_script(
            "CALL",
            "Global declaration caller",
            "func CallBroken() { return Broken(); }\n\
                 func CallHealthy() { return Healthy(); }",
        )),
    ));
    let object =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("CALL")));
    let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));

    assert_eq!(
        engine
            .call_object_function(index, "CallHealthy", Vec::new())
            .expect("the later healthy global function survives recovery"),
        Value::Int(7)
    );
    assert_eq!(
        engine
            .call_object_function(index, "CallBroken", Vec::new())
            .expect("the preparser-only diagnostic does not poison the function"),
        Value::Int(9)
    );
}
