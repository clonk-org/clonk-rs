use clonk_engine::{Definition, Engine};

fn marker_definition(id: &str) -> Definition {
    crate::support::TestValueExt::test_value(Definition::from_script(id, id, "#strict\n"))
}

#[test]
fn script_counter_reads_the_live_post_incremented_counter_and_goto_write() {
    // C++ C4GameScriptHost::Execute formats ScriptN with Counter++ before
    // entering the callback (src/C4ScriptHost.cpp:222-232). ScriptCounter()
    // then returns that live integer (src/C4Script.cpp:3616-3619), while
    // goto(int) replaces it synchronously and returns the assigned value
    // (src/C4Script.cpp:225-229). A later VM error unwinds only the call
    // (src/C4AulExec.cpp:1318-1342), not the already-live goto write.
    const SCENARIO: &str = r#"#strict
func Initialize() { ScriptGo(1); }
func Script0() {
    CreateObject(GOOD);
    if (ScriptCounter() != 1) CreateObject(BAD0);
    if (goto(0) != 0) CreateObject(BADG);
    if (ScriptCounter() != 0) CreateObject(BADS);
    MissingAfterRepeat();
}
func Script1() { CreateObject(BAD1); }
"#;

    let mut engine = Engine::new();
    for id in ["GOOD", "BAD0", "BADG", "BADS", "BAD1"] {
        crate::support::TestValueExt::test_value(engine.register_definition(marker_definition(id)));
    }
    crate::support::TestValueExt::test_value(
        engine.install_scenario_script_with_convention("Tutorial", SCENARIO, true),
    );

    let snapshot = crate::support::TestValueExt::test_value(
        (0..20).try_fold(engine.snapshot(), |_, _| engine.tick()),
    );
    let definitions: Vec<_> = snapshot
        .objects
        .iter()
        .map(|object| object.definition_id.as_str())
        .collect();

    assert_eq!(
        definitions
            .iter()
            .filter(|definition| **definition == "GOOD")
            .count(),
        2,
        "goto must repeat Script0 on the next pulse: {definitions:?}"
    );
    for bad in ["BAD0", "BADG", "BADS", "BAD1"] {
        assert!(
            !definitions.contains(&bad),
            "{bad} marks a ScriptCounter/goto mismatch: {definitions:?}"
        );
    }
}
