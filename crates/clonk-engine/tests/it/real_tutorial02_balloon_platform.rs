use crate::support::real_scenario::load_tutorial;
use crate::support::EngineTestExt;
use clonk_engine::{
    CommandDirection, Engine, JoinPlayerConfig, ObjectUpdate, Vector2, COM_DOWN,
    COM_RELEASE_OFFSET, COM_UP,
};

fn load_tutorial02(control_style: bool) -> (Engine, i32) {
    let mut engine = load_tutorial(2, 0);
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 2 balloon platform".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style,
            auto_context_menu: control_style,
            startup_player_count: 1,
        })
        .unwrap_or_else(|error| panic!("Tutorial02 player joins: {error}"));
    (engine, joined.number())
}

#[test]
fn tutorial02_balloon_flight_keeps_the_pushing_clonk_on_its_platform() {
    let (mut engine, player) = load_tutorial02(true);
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(player));
    let balloon = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "BALN"),
    );

    // C4Player::PlaceReadyCrew/PlaceReadyVehic put both objects into the
    // first base and replace their command stacks with Exit before play
    // begins (C4Player.cpp:551-564,619-640).
    let clonk_at_spawn = engine.test_object_snapshot(clonk);
    assert_eq!(clonk_at_spawn.container, balloon.container);
    assert!(clonk_at_spawn.container.is_some());
    assert_eq!(clonk_at_spawn.command_stack.command_names(), ["Exit"]);
    assert_eq!(balloon.command_stack.command_names(), ["Exit"]);

    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
            && engine
                .object_snapshot(balloon.id)
                .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(engine.test_object_snapshot(clonk).container, None);
    assert_eq!(engine.test_object_snapshot(balloon.id).container, None);

    // Fresh players default to Jump'n'Run/AutoStop control. Its single held
    // Down queues Grab, and the completed command enters DFA_PUSH
    // (C4Player.cpp:1490-1554; C4ObjectCom.cpp:247-259,573-588).
    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_DOWN, 0));
    for _ in 0..80 {
        if engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon.id)
        }) {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let pushing = engine.test_object_snapshot(clonk);
    let balloon_before_lift = engine.test_object_snapshot(balloon.id);
    assert_eq!(pushing.action.name, "Push");
    assert_eq!(pushing.action.target, Some(balloon.id));
    let platform_delta_x = pushing.position.x - balloon_before_lift.position.x;
    let platform_delta_y = pushing.position.y - balloon_before_lift.position.y;
    crate::support::TestValueExt::test_value(engine.player_in_com(
        player,
        COM_DOWN + COM_RELEASE_OFFSET,
        0,
    ));

    // While DFA_PUSH is active, the target receives Up first
    // (C4Object.cpp:3520-3537). AutoStop's BALN::ControlUpdate selects
    // COMD_Up, and its Float procedure accelerates upward
    // (C4Object.cpp:3321-3338; Balloon.c4d/Script.c:60-78). DFA_PUSH follows
    // the target and sets CNAT_Bottom (C4Object.cpp:5058-5114). When BALN's DoMotion removes
    // its solid mask, C++ backs up every object contacting that mask, then
    // Put moves each one by BALN's dx/dy before its own attachment pass
    // (C4Movement.cpp:121-126,443-445; C4SolidMask.cpp:178-195,276-305).
    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_UP, 0));
    assert_eq!(
        engine.test_object_snapshot(balloon.id).command_direction,
        CommandDirection::Up
    );

    for lift_frame in 1..=100 {
        if engine
            .object_snapshot(balloon.id)
            .is_some_and(|object| object.position.y <= 275)
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let clonk_now = engine.test_object_snapshot(clonk);
        let balloon_now = engine.test_object_snapshot(balloon.id);
        assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target),
            ("Push", Some(balloon.id)),
            "DFA_PUSH must retain the moving BALN on lift frame {lift_frame}; \
             clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
        assert!(
            (clonk_now.position.x - balloon_now.position.x - platform_delta_x).abs() <= 1,
            "CLNK must remain horizontally attached to BALN's platform on lift frame \
             {lift_frame}; initial delta={platform_delta_x}, clonk={clonk_now:?}, \
             balloon={balloon_now:?}"
        );
        assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain vertically attached to BALN's platform on lift frame \
             {lift_frame}; initial delta={platform_delta_y}, clonk={clonk_now:?}, \
             balloon={balloon_now:?}"
        );
    }

    let balloon_after_lift = engine.test_object_snapshot(balloon.id);
    assert!(
        balloon_after_lift.position.y < balloon_before_lift.position.y
            && balloon_after_lift.position.y <= 275,
        "BALN::ControlUp must move the attached platform into Tutorial02's flight corridor"
    );

    // AutoStop's Up release sends ControlUpdate(COMD_Stop). Wind2Float keeps
    // applying horizontal wind in Stop, so every following frame exercises
    // C4SolidMask's x/y attachment restore across multiple platform widths
    // (C4Object.cpp:3321-3338; Balloon.c4d/Script.c:60-78,103-110;
    // C4SolidMask.cpp:178-195,276-305).
    crate::support::TestValueExt::test_value(engine.player_in_com(
        player,
        COM_UP + COM_RELEASE_OFFSET,
        0,
    ));
    assert_eq!(
        engine.test_object_snapshot(balloon.id).command_direction,
        CommandDirection::Stop
    );

    let coast_start_x = engine.test_object_snapshot(balloon.id).position.x;
    for coast_frame in 1..=600 {
        if engine
            .object_snapshot(balloon.id)
            .is_some_and(|object| (object.position.x - coast_start_x).abs() >= 64)
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let clonk_now = engine.test_object_snapshot(clonk);
        let balloon_now = engine.test_object_snapshot(balloon.id);
        assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target),
            ("Push", Some(balloon.id)),
            "DFA_PUSH must retain the moving BALN on coast frame {coast_frame}; \
             clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
        assert!(
            (clonk_now.position.x - balloon_now.position.x).abs() < 18,
            "CLNK must remain inside BALN's shipped 36px solid-mask platform on coast frame \
             {coast_frame}; clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
        assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must retain its vertical BALN platform offset on coast frame \
             {coast_frame}; initial delta={platform_delta_y}, clonk={clonk_now:?}, \
             balloon={balloon_now:?}"
        );
    }
    let balloon_after_coast = engine.test_object_snapshot(balloon.id);
    assert!(
        (balloon_after_coast.position.x - coast_start_x).abs() >= 64,
        "real Tutorial02 wind must move BALN at least 64px laterally while the CLNK stays attached; \
         start_x={coast_start_x}, balloon={balloon_after_coast:?}"
    );

    let descent_start_y = balloon_after_coast.position.y;
    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_DOWN, 0));
    assert_eq!(
        engine.test_object_snapshot(balloon.id).command_direction,
        CommandDirection::Down
    );
    for descent_frame in 1..=30 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let clonk_now = engine.test_object_snapshot(clonk);
        let balloon_now = engine.test_object_snapshot(balloon.id);
        assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target),
            ("Push", Some(balloon.id)),
            "DFA_PUSH must retain BALN on descent frame {descent_frame}"
        );
        assert!(
            (clonk_now.position.x - balloon_now.position.x).abs() < 18
                && (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain on BALN through descent frame {descent_frame}; \
             clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
    }
    assert!(
        engine
            .object_snapshot(balloon.id)
            .is_some_and(|object| object.position.y > descent_start_y),
        "held AutoStop Down must lower the attached BALN"
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        player,
        COM_DOWN + COM_RELEASE_OFFSET,
        0,
    ));
    assert_eq!(
        engine.test_object_snapshot(balloon.id).command_direction,
        CommandDirection::Stop
    );
}

#[test]
fn tutorial02_open_bottom_removes_the_clonk_in_the_crossing_tick() {
    // Tutorial02 has BottomOpen=1. C4Object::ExecMovement removes an ordinary
    // object at y > GBackHgt in that same tick (src/C4Movement.cpp:598-617).
    let (mut engine, player) = load_tutorial02(true);
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(player));
    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
    assert!(landscape.bottom_open());
    let below_bottom = landscape.estimated_height() + 1;
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(100, below_bottom))
                .with_velocity(Vector2::ZERO),
        ),
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert!(
        engine.object_snapshot(clonk).is_none(),
        "the raw open bottom must not leave the CLNK alive for another frame"
    );
}
