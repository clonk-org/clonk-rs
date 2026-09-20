//! Z4Keepers draws its procedural landscape with two `global func` helpers
//! declared in `MiscObjects.c4d/ConstructionSite.c4d/Script.c`, a script that
//! also declares `local x,y;`. Both helpers loop with `for(var x=…)` and
//! `for(var y=…)`, which in C4Aul are the function's own variables: an
//! identifier resolves against the function's parameters, then its `var`s, and
//! only then the script's locals (C4AulParse.cpp:1975-1996). The port used to
//! miss the `for` initialiser when it recorded a function's `var`s, refused
//! both helpers as "using local variable in global function!", and the
//! scenario's structures were never drawn (clonk-org/clonk-rs#1669).

use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::SpawnConfig;
use clonk_script::Value;

/// Calls the shipped helpers the way the construction site does, from script.
const LANDSCAPE_PROBE: &str = r#"#strict
public func Draw(int x, int y)
{
    MaterialCircle("Earth", x, y, 6);
    SkyCircle(x, y, 3);
    return 1;
}
"#;

#[test]
fn z4_keepers_landscape_helpers_run_with_their_loop_vars() {
    let mut engine = load_installed_scenario("Collection.c4f/Knights.c4f/Z4Keepers.c4s", 0);
    engine
        .register_script_definition("Z4LP", "Landscape probe", LANDSCAPE_PROBE)
        .expect("the landscape probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("Z4LP"))
        .expect("the landscape probe spawns");

    let index = engine.test_object_index(probe);
    let drawn = engine
        .call_object_function(index, "Draw", vec![Value::Int(200), Value::Int(200)])
        .expect("MaterialCircle and SkyCircle are callable global functions");
    assert_eq!(drawn.as_c4_int(), Some(1));
}
