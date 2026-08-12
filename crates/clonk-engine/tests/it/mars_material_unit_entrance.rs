//! ClonkMars' Materialeinheit has a `Door` action that carries no `Sound=`
//! (Structures.c4d/Materialunit.c4d/ActMap.txt:46-50), so the overlay driver
//! plays the empty string `GetActMapVal` hands back for it
//! (System.c4g/SetOverlayAction.c:102; C4Script.cpp:4217-4241).
//!
//! `FnSound` never validates that name — it forwards it and returns true
//! unconditionally for network safety (C4Script.cpp:2297-2327). A runtime
//! error there instead aborts `ActivateEntrance` before `DoorActive = true`
//! and before the effect's `OpenEntrance` end call reaches `SetEntrance(1)`
//! (Helpers.c4d/OverlayDoorControl.c4d/Script.c:30-49), and because
//! `C4Object::ActivateEntrance` calls script with `fPassError=false` the
//! building stays sealed for the rest of the round with nothing shown.

use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::command::{CommandId, CommandRequest};
use clonk_engine::{Engine, ObjectId, ObjectUpdate, PlayerConfig, SpawnConfig, Vector2};
use clonk_script::Value;

const PLAYER: i32 = 1;

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = engine.test_object_index(object);
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

fn entrance_open(engine: &Engine, object: ObjectId) -> bool {
    crate::support::TestValueExt::test_value(
        engine
            .find_object_index(object)
            .map(|index| engine.objects[index].state.entrance_status),
    )
}

/// `C4Object::GetEntranceArea` offsets the DefCore entrance rectangle from the
/// object's current position (C4Object.cpp:2074-2093), and `C4Command::Enter`
/// walks a crew member to that rectangle's centre (C4Command.cpp:586-615).
fn door_center(engine: &Engine, unit: ObjectId) -> Vector2 {
    let position = engine.test_object_snapshot(unit).position;
    let entrance = crate::support::TestValueExt::test_value(
        engine
            .definition("UNIT")
            .and_then(|definition| definition.entrance_rect()),
    );
    Vector2::new(
        position.x + entrance.x + entrance.width / 2,
        position.y + entrance.y + entrance.height / 2,
    )
}

#[test]
fn mars_material_unit_door_opens_without_an_actmap_sound() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    engine.register_test_player(PlayerConfig::new(PLAYER, "Mars door tester"));

    let unit = engine.spawn_test_object(
        SpawnConfig::new("UNIT")
            .with_owner(PLAYER)
            .with_position(Vector2::new(300, 200)),
    );
    let clonk = engine.spawn_test_object(
        SpawnConfig::new("SCNK")
            .with_loaded(true)
            .with_owner(PLAYER)
            .with_controller(PLAYER)
            .with_alive(true)
            .with_crew_member(true)
            .with_position(door_center(&engine, unit)),
    );

    assert!(!entrance_open(&engine, unit), "the door starts closed");
    call(
        &mut engine,
        unit,
        "ActivateEntrance",
        vec![Value::Object(clonk.as_u64())],
    );
    assert_eq!(
        engine
            .test_object_snapshot(unit)
            .local_vars
            .get("DoorActive")
            .cloned(),
        Some(Value::Bool(true)),
        "the soundless Door action must not abort ActivateEntrance before it \
         records the open door"
    );

    // The Door action runs Length=10 frames at Delay=2 before the overlay
    // effect's end call reaches OpenEntrance -> SetEntrance(1).
    for _ in 0..40 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        if entrance_open(&engine, unit) {
            break;
        }
    }
    assert!(
        entrance_open(&engine, unit),
        "the Door animation must finish into OpenEntrance"
    );

    // Stand the crew member in the open doorway, where walking there would
    // have left it, and let the ordinary Enter command take over.
    let door = door_center(&engine, unit);
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(clonk, ObjectUpdate::new().with_position(door)),
    );
    let index = engine.test_object_index(clonk);
    crate::support::TestValueExt::test_value(
        engine.objects[index]
            .commands
            .push_back(CommandRequest::new(CommandId::Enter).with_target(Some(unit))),
    );

    for _ in 0..10 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        if engine
            .object_snapshot(clonk)
            .is_some_and(|snapshot| snapshot.container == Some(unit))
        {
            break;
        }
    }

    assert_eq!(
        engine.test_object_snapshot(clonk).container,
        Some(unit),
        "a crew member sent to the Materialeinheit must get in"
    );
}
