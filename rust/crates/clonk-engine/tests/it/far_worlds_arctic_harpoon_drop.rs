use crate::support::real_scenario::{
    join_local_player, prepare_installed_scenario, PreparedInstalledScenario,
};
use clonk_engine::{Engine, ObjectId, ObjectUpdate, SpawnConfig, COM_THROW};
use clonk_script::Value;

fn arctic_inuk_with_harpoon(
    prepared: &PreparedInstalledScenario,
    name: &str,
) -> (Engine, i32, ObjectId, ObjectId) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, name);
    let inuk = engine
        .crew_cursor(owner)
        .expect("Arctic joins with a selected Inuit");
    engine
        .apply_object_update(inuk, ObjectUpdate::new().clear_container())
        .expect("the Inuit leaves the starting igloo for the control probe");
    let harpoon = engine
        .spawn_object(SpawnConfig::new("HARP").with_container(inuk))
        .expect("the shipped Arctic harpoon enters the Inuit inventory");
    assert_eq!(
        engine
            .object_snapshot(inuk)
            .expect("the Inuit remains live")
            .contents
            .first(),
        Some(&harpoon),
        "the shipped HARP is the selected first inventory item"
    );
    (engine, owner, inuk, harpoon)
}

#[test]
fn arctic_inuk_harpoon_throw_respects_down_double_drop_latch() {
    // INUK::ControlThrow uses GetPlrDownDouble to leave a selected harpoon on
    // the ordinary hardcoded drop path after double-Down; without the latch it
    // starts ThrowHarpoon instead (FarWorlds.c4d/Arctic.c4d/Crew.c4d/
    // Inuk.c4d/Script.c:188-205). C++ returns the live
    // C4Player::LastComDownDouble countdown (C4Script.cpp:2618-2622), which
    // PlayerObjectCommand then converts into Drop (C4ObjectCom.cpp:1020-1036).
    let prepared = prepare_installed_scenario("FarWorlds.c4f/Arctic.c4s", 0);
    let (mut ordinary, _owner, inuk, harpoon) =
        arctic_inuk_with_harpoon(&prepared, "Arctic ordinary harpoon throw");

    let inuk_index = ordinary
        .find_object_index(inuk)
        .expect("the Inuit has an index");
    assert_eq!(
        ordinary
            .call_object_function(inuk_index, "ControlThrow", Vec::new())
            .expect("the shipped ordinary ControlThrow callback completes"),
        Value::Bool(true),
        "without a down-double latch INUK consumes Throw as ThrowHarpoon"
    );
    let ordinary_inuk = ordinary
        .object_snapshot(inuk)
        .expect("the ordinary-throw Inuit remains live");
    assert_eq!(ordinary_inuk.action.name, "ThrowHarpoon");
    assert_eq!(
        ordinary
            .object_snapshot(harpoon)
            .expect("the harpoon survives until ThrowHarpoon's EndCall")
            .container,
        Some(inuk)
    );

    let (mut dropping, owner, inuk, _harpoon) =
        arctic_inuk_with_harpoon(&prepared, "Arctic down-double harpoon drop");
    dropping
        .player_mut(owner)
        .expect("the Arctic player remains joined")
        .control
        .last_com_down_double = 7;

    dropping
        .player_in_com(owner, COM_THROW, 0)
        .expect("latched harpoon drop control completes");
    let dropping_inuk = dropping
        .object_snapshot(inuk)
        .expect("the dropping Inuit remains live");
    assert_eq!(
        dropping_inuk.command_stack.command_names(),
        vec!["Drop"],
        "a live down-double latch bypasses INUK::ThrowHarpoon and reaches C++'s hardcoded Drop"
    );
    assert_ne!(
        dropping_inuk.action.name, "ThrowHarpoon",
        "the double-Down gesture must drop rather than spear-throw the selected HARP"
    );
}
