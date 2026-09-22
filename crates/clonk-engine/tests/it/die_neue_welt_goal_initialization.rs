//! DieNeueWelt's `Initialize` creates a `CROB` goal and adds seven types to
//! `FindObject(CROB)` before it places the bandits, digs the elevator shaft
//! and starts the script counter. The goal keeps its "array" in numbered
//! locals `createType0..9` written with `eval`. The port does not yet find the
//! goal just created (clonk-org/clonk-rs#1698), so the additions go to a
//! restored goal that already holds seven, and the fourth runs
//! `eval("createType10 = WTOW")`, naming a local the goal never declared.
//! C4Aul's parser refuses an unknown identifier and DirectExec answers a parse
//! error with nil (C4AulExec.cpp:1689-1699); the port raised a runtime error
//! there instead, and `Initialize` ended with none of what follows
//! (clonk-org/clonk-rs-content#86).

use crate::support::real_scenario::load_installed_scenario;

#[test]
fn die_neue_welts_initialize_places_its_bandits() {
    let engine = load_installed_scenario("Collection.c4f/Settling.c4f/DieNeueWelt.c4s", 0);

    let bandits = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "BNDT")
        .count();
    assert_eq!(bandits, 3, "the bandits are placed after the goal block");
}
