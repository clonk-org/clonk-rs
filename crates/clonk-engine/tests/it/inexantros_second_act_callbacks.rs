//! InExantros' second act opens `func Initialize() {` at line 43 of its
//! scenario script and never closes it; `RelaunchPlayer:` follows at line 387,
//! then `InitializePlayer:`, `Saving:`, `Win:`, `Horn:`, `Quake:`, `NewNight:`,
//! `NewDay:` and more. In C4Aul a bare old-style label in statement position
//! ends the function (C4AulParse.cpp:2216-2238). Its preparser has shifted past
//! the label's name by then, so that one function is never declared, and
//! everything declared after it is. The port used to skip to the unclosed
//! function's closing brace, which here is the end of the file, and lost every
//! callback of the act (clonk-org/clonk-rs#1696).

use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::SpawnConfig;

/// `GameCall` answers nil for a scenario function that does not exist.
const CALLBACK_PROBE: &str = r#"#strict
public func Has(string callback)
{
    return GameCall(callback);
}
"#;

#[test]
fn the_second_act_keeps_the_callbacks_after_its_unclosed_initialize() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Adventures.c4f/InExantros.c4f/2.Akt.c4s", 0);
    engine
        .register_script_definition("IXPR", "Callback probe", CALLBACK_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("IXPR"));
    let index = engine.test_object_index(probe);
    let mut answer = |callback: &str| {
        engine
            .call_object_function(index, "Has", vec![callback.into()])
            .unwrap_or_else(|error| panic!("GameCall({callback}) runs: {error}"))
            .as_c4_int()
    };

    // `NewNight:` is nothing but `return(1);`, thirteen labels after the
    // unclosed function.
    assert_eq!(answer("NewNight"), Some(1));
    // The label that ended `Initialize` is the one function C4Aul loses too.
    assert_eq!(answer("RelaunchPlayer").unwrap_or(0), 0);
}
