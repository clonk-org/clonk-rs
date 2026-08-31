//! Picking a Western sack up through its pushed-target context row, reported
//! as a silent no-op in clonk-org/clonk-rs#1391.
//!
//! `Western.c4d/Items.c4d/Tools.c4d/Sack.c4d/Script.c:66-72` guards
//! `ControlDigDouble` with `FindContents(SAC1, pClonk)` and the Clonk's own
//! `RejectCollect`, and it emits no message when either refuses. An
//! empty-handed Clonk must still reach `Enter`, which `Entrance` (`:11-17`)
//! answers by swapping SACK for the collectible SAC1 — so the row doing
//! nothing is only correct when the Clonk already carries a sack.

use crate::support::real_scenario::{join_local_player_on_team, load_installed_scenario};
use crate::support::{EngineTestExt, TestValueExt};
use clonk_engine::{
    Engine, ObjectId, ObjectUpdate, SpawnConfig, COM_DOUBLE, COM_DOWN, COM_MENU_SELECT, COM_THROW,
};

/// Frames the shipped crew needs to fall out of its start position and reach
/// DFA_WALK, which `ObjectComGrab` requires (C4ObjectCom.cpp:247-259).
const SETTLE_FRAMES: usize = 400;

/// Totem Hunt loads `Western.c4d`, so SACK/SAC1 and the indian crew are the
/// shipped definitions rather than a fixture (TotemHunt.c4s/Scenario.txt:8-11).
fn totem_hunt_with_a_pushed_sack() -> (Engine, i32, ObjectId, ObjectId) {
    let mut engine = load_installed_scenario("Western.c4f/TotemHunt.c4s", 0);
    let owner = join_local_player_on_team(&mut engine, "Sack pickup parity", 1);
    let clonk = engine.crew_cursor(owner).test_value();

    for _ in 0..SETTLE_FRAMES {
        engine.tick_without_snapshot().test_value();
    }
    assert_eq!(
        engine.test_object_snapshot(clonk).action.name,
        "Walk",
        "the Clonk must stand before it can grab"
    );

    let standing = engine.test_object_snapshot(clonk).position;
    let sack = engine.spawn_test_object(SpawnConfig::new("SACK").with_position(standing));
    assert_ne!(
        engine.test_object_snapshot(sack).ocf & clonk_engine::ocf::GRAB,
        0,
        "`Grab=2` in the sack's DefCore is what makes it a push target"
    );
    // The sack floats, so re-seat it underfoot and let the sector shape index
    // catch up before the double Down asks what is there
    // (C4GameObjects.cpp:87-90; C4ObjectCom.cpp:573-589).
    engine
        .apply_object_update(sack, ObjectUpdate::new().with_position(standing))
        .test_value();
    engine.tick_without_snapshot().test_value();
    engine
        .player_in_com(owner, COM_DOWN | COM_DOUBLE, 0)
        .test_value();
    engine.tick_without_snapshot().test_value();

    let pushing = engine.test_object_snapshot(clonk);
    assert_eq!(pushing.action.name, "Push", "the Clonk must grab the sack");
    assert_eq!(pushing.action.target, Some(sack));

    (engine, owner, clonk, sack)
}

/// Right-click the pushed sack and activate its `Pick up` row, the route the
/// report describes (C4MouseControl.cpp:1253-1260; C4ObjectMenu.cpp:544-713).
fn enter_pick_up_row(engine: &mut Engine, owner: i32, sack: ObjectId) {
    engine
        .player_context_command(owner, sack)
        .expect("right-click queues the sack context command");
    engine.tick_without_snapshot().test_value();
    let menu = engine.cursor_object_menu(owner).test_value().1.clone();
    let row = menu
        .items
        .iter()
        .position(|item| item.command.contains("ControlDigDouble"))
        .unwrap_or_else(|| panic!("the pushed sack exposes its ControlDigDouble row; {menu:?}"));
    assert_eq!(menu.items[row].caption, "Pick up");
    engine
        .player_in_com(owner, COM_MENU_SELECT, row as i32)
        .test_value();
    engine.player_in_com(owner, COM_THROW, 0).test_value();
}

#[test]
fn pushed_sack_pick_up_row_collects_it_for_an_empty_handed_clonk() {
    let (mut engine, owner, clonk, sack) = totem_hunt_with_a_pushed_sack();
    enter_pick_up_row(&mut engine, owner, sack);

    let collected = engine.test_object_snapshot(sack);
    assert_eq!(
        collected.container,
        Some(clonk),
        "ControlDigDouble's Enter must contain the sack"
    );
    assert_eq!(
        collected.definition_id, "SAC1",
        "Entrance swaps the pushable SACK for the collectible SAC1"
    );
}

#[test]
fn pushed_sack_pick_up_row_collects_it_for_a_clonk_with_full_hands() {
    let (mut engine, owner, clonk, sack) = totem_hunt_with_a_pushed_sack();
    // Only `FindContents(SAC1, ...)` and the Clonk's own `RejectCollect` gate
    // the row; no shipped Clonk defines the latter, so ordinary carried items
    // must not suppress the pickup.
    for _ in 0..2 {
        engine.spawn_test_object(SpawnConfig::new("FLNT").with_container(clonk));
    }
    enter_pick_up_row(&mut engine, owner, sack);

    let collected = engine.test_object_snapshot(sack);
    assert_eq!(collected.container, Some(clonk));
    assert_eq!(collected.definition_id, "SAC1");
}

#[test]
fn pushed_sack_pick_up_row_refuses_a_clonk_already_carrying_a_sack() {
    let (mut engine, owner, clonk, sack) = totem_hunt_with_a_pushed_sack();
    engine.spawn_test_object(SpawnConfig::new("SAC1").with_container(clonk));
    enter_pick_up_row(&mut engine, owner, sack);

    let refused = engine.test_object_snapshot(sack);
    assert_eq!(
        refused.container, None,
        "`FindContents(SAC1, pClonk)` returns before Enter, silently"
    );
    assert_eq!(refused.definition_id, "SACK");
}
