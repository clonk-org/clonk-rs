use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use crate::support::EngineTestExt;
use clonk_engine::{ObjectId, ObjectStatus, ObjectUpdate, COM_DIG};
use clonk_script::Value;

fn local(engine: &clonk_engine::Engine, object: ObjectId, name: &str) -> Value {
    engine
        .object_snapshot(object)
        .unwrap_or_else(|| panic!("object {object:?} remains live"))
        .local_vars
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("object {object:?} declares local {name}"))
}

fn definition_ids(engine: &clonk_engine::Engine, container: ObjectId) -> Vec<String> {
    engine
        .object_snapshot(container)
        .unwrap_or_else(|| panic!("container {container:?} remains live"))
        .contents
        .into_iter()
        .filter_map(|object| {
            engine
                .object_snapshot(object)
                .map(|snapshot| snapshot.definition_id)
        })
        .collect()
}

#[test]
fn eke_empty_integrated_pistol_redraws_to_front_and_reloads_from_one_magazine() {
    // SFT::Holster retains one persistent PT5B and redraws it with
    // Enter(this(), pistol) followed by ShiftContents(0, true, PT5B). C++
    // first inserts that C4D_StaticBack object at the tail and then rotates
    // the exact object to Contents.First (Eke SFT.c4d/Script.c:583-600;
    // oracle C4Script.cpp:1863-1876, C4Object.cpp:5816-5836,
    // C4ObjectList.cpp:815-831).
    //
    // Equiem supplies a real PM5B. Two Dig presses in PistolWalk take the
    // native double-click fallback and activate Contents.First; PT5B then
    // consumes that magazine and fills its persistent ammo local to 100.
    let mut engine =
        load_installed_scenario("EkeReloaded.c4f/InterplanetaryCivilwar.c4f/Equiem.c4s", 0);
    let owner = join_local_player(&mut engine, "Eke pistol parity");
    let sft = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert_eq!(
        definition_ids(&engine, sft),
        ["JP5B", "GS5B", "PM5B"],
        "Equiem supplies the ordinary inventory and one pistol magazine"
    );
    let magazine = crate::support::TestValueExt::test_value(
        engine
            .test_object_snapshot(sft)
            .contents
            .into_iter()
            .find(|&object| {
                engine
                    .object_snapshot(object)
                    .is_some_and(|snapshot| snapshot.definition_id == "PM5B")
            }),
    );

    crate::support::TestValueExt::test_value(
        engine.apply_object_update(sft, ObjectUpdate::new().with_action("Jump")),
    );
    let sft_index = engine.test_object_index(sft);
    assert_eq!(
        engine.call_test_object_function(sft_index, "Holster", Vec::new()),
        Value::Int(1)
    );
    let pistol = crate::support::TestValueExt::test_value(
        engine.test_object_snapshot(sft).contents.first().copied(),
    );
    assert_eq!(engine.test_object_snapshot(pistol).definition_id, "PT5B");
    assert_eq!(engine.test_object_snapshot(sft).action.name, "PistolJump");
    assert_eq!(
        local(&engine, sft, "pistol"),
        Value::Object(pistol.as_u64())
    );
    assert_eq!(local(&engine, pistol, "ammo"), Value::Int(100));

    for expected_ammo in (0..10).rev().map(|shot| shot * 10) {
        let pistol_index = engine.test_object_index(pistol);
        assert_eq!(
            engine.call_test_object_function(
                pistol_index,
                "ControlThrow",
                vec![Value::Object(sft.as_u64())],
            ),
            Value::Int(1)
        );
        assert_eq!(
            local(&engine, pistol, "ammo"),
            Value::Int(expected_ammo),
            "each real pistol shot consumes ten ammo"
        );
        if expected_ammo != 0 {
            crate::support::TestValueExt::test_value(
                engine.apply_object_update(pistol, ObjectUpdate::new().with_action("Idle")),
            );
        }
    }

    let sft_index = engine.test_object_index(sft);
    engine.call_test_object_function(sft_index, "Holster", Vec::new());
    assert!(
        !engine.test_object_snapshot(sft).contents.contains(&pistol),
        "holstering removes PT5B from the carried inventory"
    );
    assert!(
        engine.object_snapshot(pistol).is_some(),
        "the integrated empty pistol is retained for the next draw"
    );

    crate::support::TestValueExt::test_value(
        engine.apply_object_update(sft, ObjectUpdate::new().with_action("Walk")),
    );
    let sft_index = engine.test_object_index(sft);
    assert_eq!(
        engine.call_test_object_function(sft_index, "Holster", Vec::new()),
        Value::Int(1)
    );
    assert_eq!(
        definition_ids(&engine, sft),
        ["PT5B", "JP5B", "GS5B", "PM5B"],
        "redrawing rotates the persistent empty pistol to the inventory front"
    );
    assert_eq!(local(&engine, pistol, "ammo"), Value::Int(0));
    assert_eq!(engine.test_object_snapshot(sft).action.name, "PistolWalk");

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DIG, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DIG, 0));

    assert_eq!(
        local(&engine, pistol, "ammo"),
        Value::Int(100),
        "PT5B::Activate reloads the empty pistol"
    );
    assert!(
        engine
            .object_snapshot(magazine)
            .is_none_or(|snapshot| snapshot.status == ObjectStatus::Deleted),
        "PT5B::Activate assigns removal to Equiem's one PM5B"
    );
    assert_eq!(
        definition_ids(&engine, sft),
        ["PT5B", "JP5B", "GS5B"],
        "reloading removes exactly the magazine and keeps PT5B selected"
    );
}
