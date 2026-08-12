use super::*;

#[test]
fn late_definition_registration_invalidates_the_shared_command_table() -> Result<(), EngineError> {
    let mut engine = Engine::new();
    engine.register_definition(test_definition("BASE", "Base", ""))?;

    let first = engine.command_definition_snapshot_table();
    let reused = engine.command_definition_snapshot_table();
    assert!(Rc::ptr_eq(&first, &reused));

    let mut chopper = test_definition("CHOP", "Chopper", "");
    chopper.configure_actions(
        None,
        HashMap::from([(
            "ChopWood".to_string(),
            ActionSpec::default().with_procedure("CHOP"),
        )]),
    );
    engine.register_definition(chopper)?;

    let rebuilt = engine.command_definition_snapshot_table();
    assert!(!Rc::ptr_eq(&first, &rebuilt));
    let snapshot = crate::TestValueExt::test_value(rebuilt.get("CHOP"));
    assert!(snapshot.can_chop);
    assert_eq!(snapshot.chop_action.as_deref(), Some("ChopWood"));
    Ok(())
}

#[test]
fn definition_and_script_boundaries_invalidate_shared_host_tables() -> Result<(), EngineError> {
    let mut engine = Engine::new();
    engine.register_definition(test_definition("BASE", "Base", ""))?;

    let first = engine.host_definition_tables();
    assert!(Rc::ptr_eq(&first, &engine.host_definition_tables()));

    engine.register_definition(test_definition("LATE", "Late", ""))?;
    let after_definition = engine.host_definition_tables();
    assert!(!Rc::ptr_eq(&first, &after_definition));

    engine.install_global_scripts(&[(
        "System.c4g".to_string(),
        "global func SharedHostCacheProbe() { return 1; }".to_string(),
    )]);
    let after_script = engine.host_definition_tables();
    assert!(!Rc::ptr_eq(&after_definition, &after_script));

    engine.set_standard_names(Some("Ada\nGrace".to_string()));
    assert!(!Rc::ptr_eq(&after_script, &engine.host_definition_tables()));
    Ok(())
}
