use crate::support::real_scenario::{
    join_local_player_with_preferences, load_raw_content_scenario,
};
use crate::support::EngineTestExt;

use clonk_engine::{CommandDirection, Engine, COM_DOWN, COM_UP};

fn load_tutorial(number: u8) -> (Engine, i32) {
    let scenario = load_raw_content_scenario(format!("Tutorial.c4f/Tutorial{number:02}.c4s"))
        .unwrap_or_else(|error| panic!("Tutorial{number:02} loads: {error}"));
    let mut engine = Engine::with_seed(0);
    scenario
        .apply(&mut engine)
        .unwrap_or_else(|error| panic!("Tutorial{number:02} applies: {error}"));
    let player = join_local_player_with_preferences(&mut engine, "Tutorial campaign", false, false);
    (engine, player)
}

#[test]
fn tutorial02_ready_balloon_exits_the_first_base() {
    // C4Player::PlaceReadyVehic creates each ready vehicle, enters it into the
    // first base, then immediately replaces its command stack with Exit
    // (C4Player.cpp:619-640, especially :631-636).
    let (mut engine, _) = load_tutorial(2);
    let balloon = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "BALN"),
    );
    let hut =
        engine.test_object_snapshot(crate::support::TestValueExt::test_value(balloon.container));
    assert_eq!(hut.definition_id, "HUT3");
    assert_eq!(
        balloon.command_stack.command_names(),
        vec!["Exit".to_string()],
        "PlaceReadyVehic must queue the C++ Exit command"
    );

    for _ in 0..80 {
        if engine
            .object_snapshot(balloon.id)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.test_object_snapshot(balloon.id).container,
        None,
        "the queued Exit must move BALN out of HUT3"
    );
}

#[test]
fn tutorial02_ready_crew_exits_the_first_base() {
    // C4Player::PlaceReadyCrew enters each newly created crew member into the
    // first base and immediately replaces its command stack with Exit before
    // Recruitment runs (C4Player.cpp:551-564, especially :557-558).
    let (mut engine, player) = load_tutorial(2);
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(player));
    let joined = engine.test_object_snapshot(clonk);
    let hut =
        engine.test_object_snapshot(crate::support::TestValueExt::test_value(joined.container));
    assert_eq!(hut.definition_id, "HUT3");
    assert_eq!(
        joined.command_stack.command_names(),
        vec!["Exit".to_string()],
        "PlaceReadyCrew must queue the C++ Exit command"
    );

    for _ in 0..80 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.test_object_snapshot(clonk).container,
        None,
        "the queued Exit must move the CLNK out of HUT3"
    );
}

#[test]
fn tutorial02_ready_objects_exit_then_the_clonk_controls_the_real_balloon_up() {
    // The first Tutorial02 lesson requires the normal classic-control route:
    // a repeated Down becomes COM_Down_D and queues Grab
    // (C4Player.cpp:1532-1533; C4ObjectCom.cpp:573-588), then the completed
    // Grab switches the CLNK to Push (C4ObjectCom.cpp:247-259). The Push
    // procedure gives the target first refusal on Up
    // (C4Object.cpp:3520-3537); BALN::ControlUp then selects COMD_Up
    // (Objects.c4d/Vehicles.c4d/Balloon.c4d/Script.c:14-29).
    let (mut engine, player) = load_tutorial(2);
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(player));
    let balloon = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "BALN"),
    )
    .id;

    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
            && engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let ready_clonk = engine.test_object_snapshot(clonk);
    let ready_balloon = engine.test_object_snapshot(balloon);
    assert_eq!(ready_clonk.container, None);
    assert_eq!(ready_clonk.action.name, "Walk");
    assert_eq!(ready_balloon.container, None);
    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_DOWN, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_DOWN, 0));
    for _ in 0..80 {
        if engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon)
        }) {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let pushing = engine.test_object_snapshot(clonk);
    assert_eq!(pushing.action.name, "Push");
    assert_eq!(pushing.action.target, Some(balloon));

    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_UP, 0));
    assert_eq!(
        engine.test_object_snapshot(balloon).command_direction,
        CommandDirection::Up,
        "BALN::ControlUp must complete instead of aborting at GetObjHeight"
    );
}
