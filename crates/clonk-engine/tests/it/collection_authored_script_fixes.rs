//! Authored script defects in the imported Collection that the content
//! repository corrects (clonk-org/clonk-rs#1395). Each was reproduced under the
//! pinned C++ engine too, so these are content fixes and not parity gaps: the
//! tests pin that the shipped scripts now run, against the real content.

use crate::support::real_scenario::{
    join_local_player, join_local_player_on_team, load_installed_scenario, object_with_definition,
};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, ObjectId, SpawnConfig};
use clonk_script::Value;

fn call(engine: &mut Engine, object: ObjectId, function: &str) -> Value {
    let index = engine.test_object_index(object);
    engine
        .call_object_function(index, function, Vec::new())
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

/// clonk-org/clonk-rs-content#90: `Ueberlandleitung.c4d/Script.c` declared
/// `var pObj` inside an `if` condition, which neither engine's parser accepts,
/// so the whole UBRL script failed to parse and the power-line goal never
/// evaluated. The scenario's `UBRLTarget` hands it the mine, so the answer is
/// the mine's power state; what is pinned is that the goal answers at all.
#[test]
fn the_schweids_power_line_goal_parses_and_evaluates() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Settling.c4f/RufDerWipfeRE.c4f/Kampagne.c4f/4Schweids.c4s",
        0,
    );
    join_local_player(&mut engine, "Schweids goal");
    let goal = object_with_definition(&engine, "UBRL")
        .unwrap_or_else(|| engine.spawn_test_object(SpawnConfig::new("UBRL")));

    let fulfilled = call(&mut engine, goal, "IsFulfilled");
    assert!(
        matches!(fulfilled.as_c4_int(), Some(0 | 1)),
        "the goal must evaluate to its 0/1 answer, got {fulfilled:?}"
    );
}

/// clonk-org/clonk-rs-content#82: the Teamview pack's `#appendto CLNK` called
/// `FindObject(RTVW)->ScheduleCall(..)` on every `Departure`, but ColdSkies
/// loads the pack without enabling the `RTVW` rule, so the crew's first
/// departure failed with a zero call target. The sibling appends in the same
/// pack guard the lookup; this one does now too.
#[test]
fn a_cold_skies_clonk_departs_without_the_teamview_rule() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/BaseMelees.c4f/ClassicMelees.c4f/ColdSkies.c4s",
        0,
    );
    // A team melee: the player has to be placed on a team to initialise.
    let owner = join_local_player_on_team(&mut engine, "ColdSkies departure", 1);
    assert!(
        object_with_definition(&engine, "RTVW").is_none(),
        "ColdSkies does not enable the Teamview rule"
    );
    let clonk = engine
        .crew_cursor(owner)
        .expect("the joined player has a crew member");

    call(&mut engine, clonk, "Departure");
}
