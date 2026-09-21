//! Goldwipfcaves overloads the Wipf (`Goldwipf.c4d`, `WIPF`) with a script in
//! which a second `protected func Activity()`, the gold-dust effect, was pasted
//! into the middle of the stock `Activity` at line 53. In C4Aul a function
//! declaration in statement position ends the function being parsed
//! (C4AulParse.cpp:2005-2011,2167-2189), so that script has two `Activity`
//! functions and the later one, the gold dust, is what the timer calls. The
//! port read the nested declaration as a broken statement, skipped to the first
//! function's closing brace, and was left with one `Activity` that raised on
//! every timer call (clonk-org/clonk-rs#1681).

use crate::support::real_scenario::load_installed_scenario_with_selected_definitions;
use crate::support::EngineTestExt;
use clonk_engine::SpawnConfig;
use clonk_script::Value;

/// Calls the timer callback from script, where a function that fails to parse
/// is an error and not a fail-safe nil.
const ACTIVITY_PROBE: &str = r#"#strict
public func Tick(object wipf)
{
    return wipf->Activity();
}
"#;

#[test]
fn the_goldwipf_runs_the_activity_its_author_pasted_in() {
    // The scenario names no definitions of its own, so it runs on the
    // player's startup selection.
    let mut engine = load_installed_scenario_with_selected_definitions(
        "Collection.c4f/Settling.c4f/Goldwipfcaves.c4s",
        0,
        &["Objects.c4d"],
    );
    let wipf = engine.spawn_test_object(SpawnConfig::new("WIPF"));
    engine
        .register_script_definition("GWPR", "Goldwipf probe", ACTIVITY_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("GWPR"));

    let index = engine.test_object_index(probe);
    let answer = engine
        .call_object_function(index, "Tick", vec![Value::Object(wipf.as_u64())])
        .expect("the Goldwipf's Activity runs");
    // The gold-dust body ends in `return(1)`; the stock one it was pasted into
    // never returns a value.
    assert_eq!(answer.as_c4_int(), Some(1));
}
