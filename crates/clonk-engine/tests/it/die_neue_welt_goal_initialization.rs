//! DieNeueWelt's `Initialize` creates a `CROB` goal and adds seven types to
//! `FindObject(CROB)` before it places the bandits, digs the elevator shaft
//! and starts the script counter. The goal keeps its "array" in numbered
//! locals `createType0..9` written with `eval`. The port used to miss the goal
//! just created (clonk-org/clonk-rs#1698), so the additions went to a restored
//! goal that already held seven, and the fourth ran
//! `eval("createType10 = WTOW")`, naming a local the goal never declared.
//! C4Aul's parser refuses an unknown identifier and DirectExec answers a parse
//! error with nil (C4AulExec.cpp:1689-1699); the port raised a runtime error
//! there instead, and `Initialize` ended with none of what follows
//! (clonk-org/clonk-rs-content#86).

use crate::support::real_scenario::load_installed_scenario;
use clonk_script::Value;

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

/// C4Game::NewObject links the goal `Initialize` creates in front of the
/// restored ones (C4GameObjects.cpp:54-71; C4ObjectList.cpp:155-175), so the
/// seven `FindObject(CROB)->AddType(..)` that follow fill it, and each
/// restored goal keeps the types `Objects.txt` gave it.
#[test]
fn die_neue_welts_initialize_fills_the_goal_it_creates() {
    let engine = load_installed_scenario("Collection.c4f/Settling.c4f/DieNeueWelt.c4s", 0);

    let mut goals = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "CROB")
        .map(|goal| {
            let types = goal
                .local_vars
                .get("createTypes")
                .and_then(Value::as_c4_int);
            (goal.id.as_u64(), types)
        })
        .collect::<Vec<_>>();
    goals.sort_unstable();
    let created = goals.split_off(3);
    assert_eq!(
        goals,
        [(46882, Some(49)), (66111, Some(7)), (66115, Some(7))],
        "the restored goals"
    );
    assert_eq!(
        created.iter().map(|(_, types)| *types).collect::<Vec<_>>(),
        [Some(7)],
        "the goal Initialize creates: {created:?}"
    );
}
