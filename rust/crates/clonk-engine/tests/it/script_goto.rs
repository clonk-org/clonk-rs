use clonk_engine::{Definition, Engine};

fn marker_definition(id: &str) -> Definition {
    Definition::from_script(id, id, "#strict\n").expect("marker definition compiles")
}

#[test]
fn goto_redirects_the_next_scenario_section_even_when_the_current_section_errors() {
    // C++ Fn_goto writes Game.Script.Counter synchronously before returning
    // (src/C4Script.cpp:225-229). C4GameScriptHost::Execute post-increments the
    // counter used to form the current Script%d name (src/C4ScriptHost.cpp:
    // 222-232), so goto(60) in Script0 redirects the NEXT pulse to Script60.
    // A later script error only unwinds the VM (src/C4AulExec.cpp:1318-1342);
    // it cannot roll that already-live counter write back.
    const SCENARIO: &str = r#"#strict
func Initialize() { ScriptGo(1); }
func Script0() {
    if (goto(60) != 60) CreateObject(BAD1);
    MissingAfterGoto();
}
func Script1() { CreateObject(BAD1); }
func Script60() { CreateObject(GOOD); }
"#;

    let mut engine = Engine::new();
    engine
        .register_definition(marker_definition("GOOD"))
        .expect("GOOD registers");
    engine
        .register_definition(marker_definition("BAD1"))
        .expect("BAD1 registers");
    engine
        .install_scenario_script_with_convention("Tutorial", SCENARIO, true)
        .expect("scenario installs");

    let snapshot = (0..20)
        .try_fold(engine.snapshot(), |_, _| engine.tick())
        .expect("script errors are fail-safe");
    let definitions: Vec<_> = snapshot
        .objects
        .iter()
        .map(|object| object.definition_id.as_str())
        .collect();

    assert!(
        definitions.contains(&"GOOD"),
        "Script60 must run: {definitions:?}"
    );
    assert!(
        !definitions.contains(&"BAD1"),
        "Script1 must be skipped: {definitions:?}"
    );
}
