//! InExantros' hero (`InExantros.C4D/Living.c4d/Held.c4d/Script.c`, `KNIG`)
//! opens with ten one-line `global func`s and continues with old-style
//! functions over some forty `local`s. `Parse_FuncHead` starts every
//! declaration at `AA_PUBLIC`, and an old-style function is created on the
//! engine only when it is itself written `global`
//! (C4AulParse.cpp:1563-1570,1737-1751), so those are ordinary object functions.
//! The port let the `global` of the function before them leak into them and
//! refused every one that read a local, which broke `Death` in the second act
//! and `InitializePlayer -> SpeicherErstellung` in the third
//! (clonk-org/clonk-rs#1671).

use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::SpawnConfig;

#[test]
fn the_inexantros_heros_old_style_functions_read_its_locals() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Adventures.c4f/InExantros.c4f/3.Akt.c4s", 0);
    let hero = engine.spawn_test_object(SpawnConfig::new("KNIG"));

    // `MenuQueryCancel2:` (line 336) sits between the global funcs and the
    // script's first new-style function, which is the stretch the leak reached.
    // On a fresh hero it reads the local `pmenu`, finds no menu open, clears
    // it and answers 0.
    let index = engine.test_object_index(hero);
    let answer = engine
        .call_object_function(index, "MenuQueryCancel2", Vec::new())
        .expect("an old-style function after the global funcs is an object function");
    assert_eq!(answer.as_c4_int().unwrap_or(0), 0);
}
