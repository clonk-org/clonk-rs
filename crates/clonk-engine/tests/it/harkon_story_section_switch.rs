//! MissionsHarkon's story object (`_STY`, each mission's `Time.c4d`) schedules
//! `DoStart` with `ScheduleCall`, whose global `FxIntScheduleCallTimer` runs
//! it through `Call` (planet `System.c4g/Helpers.c:153-160`). `DoStart` keeps
//! the story inactive across `LoadScenarioSection("Map", 3)` and then calls
//! the story's own `HideAttackers`. The port resumed that child with the VM
//! of the global callback, so `HideAttackers` could not see the story's
//! locals and failed at its first line, and the unfinished schedule fired
//! again every frame (clonk-org/clonk-rs#1717).

use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_script::Value;

#[test]
fn a_harkon_story_hides_its_attackers_after_switching_to_the_map() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Adventures.c4f/MissionsHarkon.c4f/Mission1A.c4s",
        0,
    );
    // The join creates the story and starts it, which schedules `DoStart`.
    let _harkon = join_local_player(&mut engine, "Harkon");
    let _ = engine.tick_without_snapshot();

    let story = engine
        .first_object_for_definition("_STY")
        .and_then(|story| engine.object_snapshot(story))
        .expect("the join starts the story");
    // The five robbers and both Kanderians (`Time.c4d/Script.c:17-33`).
    assert_eq!(story.local_vars.get("attacker_count"), Some(&Value::Int(7)));
}
