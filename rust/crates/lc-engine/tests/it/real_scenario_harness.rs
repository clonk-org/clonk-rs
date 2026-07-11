#![allow(dead_code)]

use crate::support::real_scenario::load_tutorial;

#[test]
fn tutorial_harness_boots_the_installed_cpp_global_script_layer() {
    let engine = load_tutorial(2, 0);

    // C++ loads planet/System.c4g before definitions and the scenario
    // (C4Game.cpp:2591-2607,2764-2788). Helpers.c supplies both functions
    // used by Tutorial02 and BALN; a direct Scenario::apply fixture does not.
    for function in ["Schedule", "ScheduleCall", "FxIntScheduleCallTimer"] {
        assert!(
            engine.debug_global_has_function(function),
            "virtual play must expose planet global `{function}`"
        );
    }
    assert_eq!(
        engine.materials().len(),
        21,
        "virtual play must use the installed Material.c4g library"
    );
}
