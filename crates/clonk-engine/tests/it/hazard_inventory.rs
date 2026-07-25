use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{ObjectUpdate, SpawnConfig, COM_SPECIAL};

/// The pinned C++ 4.9.11.0 build 362 rotates this uncontained HZCK's inventory
/// from PIWP to GLWP on ControlSpecial. `ContainedSpecial` is globally
/// unresolved, so its fail-safe call is compiled by evaluating/discarding the
/// target and pushing zero, rather than emitting a runtime object call
/// (oracle-src-pinned src/C4AulParse.cpp:3194-3229). Execution consequently
/// reaches Hazard's `ShiftContents(0,0,0,1)`
/// (Hazard.c4d/Crew.c4d/HazardClonk.c4d/Script.c:249-280).
#[test]
fn hazard_special_cycles_inventory_while_uncontained() {
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let owner = join_local_player(&mut engine, "Hazard inventory parity");
    let hazard_clonk = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.owner == owner && object.definition_id == "HZCK")
        .expect("Hazard joins with an HZCK")
        .id;

    // The tutorial disables its crew during the introduction. Put the HZCK
    // into the ordinary player-controlled state used by C4Player::InCom.
    let mut ready = ObjectUpdate::new().with_action("Walk");
    ready.crew_disabled = Some(false);
    engine
        .apply_object_update(hazard_clonk, ready)
        .expect("enable the tutorial HZCK for player control");
    engine
        .select_crew(owner, [hazard_clonk])
        .expect("select the tutorial HZCK");
    engine
        .set_crew_cursor(owner, Some(hazard_clonk))
        .expect("make the tutorial HZCK the control cursor");

    // C4ObjectList::Add puts newly created contents first. Create GLWP before
    // PIWP so the pistol is the selected front item, matching the live oracle.
    let grenade_launcher = engine
        .spawn_object(
            SpawnConfig::new("GLWP")
                .with_owner(owner)
                .with_container(hazard_clonk),
        )
        .expect("equip Hazard's grenade launcher");
    let pistol = engine
        .spawn_object(
            SpawnConfig::new("PIWP")
                .with_owner(owner)
                .with_container(hazard_clonk),
        )
        .expect("equip Hazard's pistol");

    let before = engine
        .object_snapshot(hazard_clonk)
        .expect("the HZCK remains live before cycling");
    assert_eq!(before.container, None, "the HZCK must be uncontained");
    assert_eq!(before.contents, vec![pistol, grenade_launcher]);

    engine
        .player_in_com(owner, COM_SPECIAL, 0)
        .expect("Special reaches the selected HZCK");

    assert_eq!(
        engine
            .object_snapshot(hazard_clonk)
            .expect("the HZCK remains live after cycling")
            .contents,
        vec![grenade_launcher, pistol],
        "ControlSpecial must rotate the front inventory item like pinned C++"
    );
}
