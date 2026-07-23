use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::{ObjectUpdate, SpawnConfig, COM_DOWN, COM_RELEASE_OFFSET};

#[test]
fn hazard_three_down_presses_enter_squat_aim_with_a_pistol() {
    // Hazard documents this as "Press stop triple to aim": the second Down
    // arms C4Player::LastComDownDouble and the third ordinary ControlDown sees
    // that latch before entering AimSquat (HazardClonk.c4d/Script.c:118-141,
    // 785-807; oracle-src-pinned src/C4Player.cpp:1490-1554).
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let owner = join_local_player(&mut engine, "Hazard squat-aim parity");
    let hazard_clonk = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.owner == owner && object.definition_id == "HZCK")
        .expect("Hazard joins with an HZCK")
        .id;
    // Hazard's tutorial temporarily disables its crew until the scripted
    // introduction completes. Enable the fixture so C4Player::InCom routes
    // controls to the same HZCK whose ControlDown behavior we are testing.
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
    assert_eq!(engine.crew_cursor(owner), Some(hazard_clonk));

    let pistol = engine
        .spawn_object(
            SpawnConfig::new("PIWP")
                .with_owner(owner)
                .with_container(hazard_clonk),
        )
        .expect("equip Hazard's aimable pistol");
    engine
        .apply_object_update(
            hazard_clonk,
            ObjectUpdate::new().with_contents_front(pistol),
        )
        .expect("select the pistol");

    for press in 1..=3 {
        engine
            .player_in_com(owner, COM_DOWN, 0)
            .unwrap_or_else(|error| panic!("Down press {press} reaches C4Player::InCom: {error}"));
        engine
            .player_in_com(owner, COM_DOWN + COM_RELEASE_OFFSET, 0)
            .unwrap_or_else(|error| {
                panic!("Down release {press} reaches C4Player::InCom: {error}")
            });
    }

    let aimed = engine
        .object_snapshot(hazard_clonk)
        .expect("the HZCK remains live");
    assert_eq!(
        aimed.action.name, "AimSquat",
        "PIWP's deterministic FM_Aim=0 path must enter Hazard's generic firearm aim"
    );
}
