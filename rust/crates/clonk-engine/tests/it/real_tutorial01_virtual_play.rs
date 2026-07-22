#![allow(dead_code)]

use std::error::Error;

use clonk_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_UP,
};
use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;

fn load_tutorial01() -> (Engine, i32) {
    let mut engine = load_tutorial(1, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 1 virtual player".to_owned(),
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
            control_style: true,
            auto_context_menu: true,
            startup_player_count: 1,
        })
        .expect("local Tutorial01 virtual player joins")
        .number();
    (engine, owner)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn clonk_carries(engine: &Engine, clonk: ObjectId, definition: &str) -> bool {
    engine.object_snapshot(clonk).is_some_and(|clonk| {
        clonk.contents.iter().any(|item| {
            engine
                .object_snapshot(*item)
                .is_some_and(|item| item.definition_id == definition)
        })
    })
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

#[test]
fn tutorial01_virtual_player_completes_the_real_tutorial_route() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial01();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial01 joins one selected CLNK");
    let hut = object_with_definition(&engine, "HUT2").expect("Tutorial01 creates HUT2");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("the tumbling Clonk lands in the valley", 180, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.wait_until(
        "Tutorial01 creates FLAG and asks the player to climb left",
        500,
        |engine| {
            object_with_definition(engine, "FLAG").is_some()
                && tutorial_message_contains(engine, "hill to your left")
        },
    )?;

    // Tutorial01 Script50 points at the FLAG placed on the left hill. Under
    // Jump'n'Run controls, held Left supplies the movement direction while
    // Up starts each jump (C4Object.cpp:3573-3592; C4ObjectCom.cpp:335-350).
    player.press(COM_LEFT)?;
    for _ in 0..30 {
        let reached_left_hill = player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x <= 25);
        if clonk_carries(player.engine(), clonk, "FLAG") || reached_left_hill {
            break;
        }
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
        {
            player.tap(COM_UP)?;
        }
        player.ticks(12)?;
    }
    player.release(COM_LEFT)?;
    if !clonk_carries(player.engine(), clonk, "FLAG") {
        player.wait_until(
            "the Clonk lands beside FLAG on the left hill",
            80,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 40)
            },
        )?;
        player.hold_until(
            COM_RIGHT,
            "the Clonk naturally collects FLAG",
            40,
            |engine| clonk_carries(engine, clonk, "FLAG"),
        )?;
    }

    // Script72 points at HUT2. Keep jumping over the valley terrain until
    // the Clonk reaches the real DefCore entrance rectangle, then Up takes
    // C4ObjectCom's Enter path before Jump (C4ObjectCom.cpp:335-350).
    player.press(COM_RIGHT)?;
    for _ in 0..90 {
        let at_hut = player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 558);
        if at_hut {
            break;
        }
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
        {
            player.tap(COM_UP)?;
        }
        player.ticks(12)?;
    }
    player.release(COM_RIGHT)?;
    player.wait_until("the Clonk lands beside HUT2's door", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    // On the corrected seed-zero map the physical jump lands about 64 pixels
    // right of HUT2's entrance. C++ DFA_WALK accelerates toward a 2.8 px/tick
    // limit, so the old 20-tick budget could not cover this ordinary walk;
    // 40 remains a tight movement bound (C4Object.cpp:4782-4815).
    player.hold_until(
        COM_LEFT,
        "the Clonk aligns with HUT2's entrance",
        40,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 570)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the FLAG-carrying Clonk enters HUT2", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;

    // C++ inserts Put before Contents in the contained auto-context menu
    // (C4ObjectMenu.cpp:335-359). Selecting it must transfer FLAG to HUT2;
    // Tutorial01 Script120 then observes the new base (Script.c:101-105).
    player.wait_until("HUT2 opens its auto-context Put menu", 20, |engine| {
        engine
            .cursor_object_menu(owner)
            .is_some_and(|(_, menu)| menu.items.first().is_some_and(|item| item.caption == "Put"))
    })?;
    player.menu_navigate_to_caption("Put")?;
    player.menu_enter()?;
    player.wait_until("FLAG enters HUT2 through the Put command", 80, |engine| {
        object_with_definition(engine, "FLAG").is_some_and(|flag| {
            engine
                .object_snapshot(flag)
                .is_some_and(|object| object.container == Some(hut))
        })
    })?;

    player.wait_until("FLAG turns HUT2 into player zero's base", 80, |engine| {
        engine
            .object_snapshot(hut)
            .is_some_and(|object| object.base == owner)
    })?;
    player.wait_until(
        "Tutorial01 asks the player to select Exit from the menu",
        450,
        |engine| {
            tutorial_message_contains(engine, "select 'Exit'")
                && engine
                    .cursor_object_menu(owner)
                    .is_some_and(|(_, menu)| menu.items.iter().any(|item| item.caption == "Exit"))
        },
    )?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until(
        "the Clonk exits HUT2 through its context menu",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
        },
    )?;

    player.wait_until(
        "Tutorial01 creates GOLD and sends the Clonk back to the valley",
        120,
        |engine| {
            object_with_definition(engine, "GOLD").is_some()
                && tutorial_message_contains(engine, "back into the valley")
        },
    )?;
    let gold =
        object_with_definition(player.engine(), "GOLD").expect("Tutorial01 Script150 creates GOLD");

    // Script160 recognizes the real 150..250 by 250..350 lesson area and
    // calls ResetPhysical before teaching Dig (Tutorial01/Script.c:134-141).
    player.hold_until(
        COM_LEFT,
        "the Clonk walks naturally from HUT2 into the lesson valley",
        260,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                (150..250).contains(&object.position.x) && (250..350).contains(&object.position.y)
            })
        },
    )?;
    player.wait_until(
        "Tutorial01 unlocks digging in the lesson valley",
        160,
        |engine| {
            tutorial_message_contains(engine, "start a digging process")
                && engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.temporary_physical.is_none())
        },
    )?;

    // A physical Dig tap becomes COM_Dig_S only after C4DoubleClick's ten
    // frames. Held Down+Left supplies DownLeft under Jump'n'Run controls;
    // releasing Down then steers the live DFA_DIG tunnel left toward GOLD
    // (C4Player.cpp:1215-1229; C4Object.cpp:3573-3631;
    // C4ObjectCom.cpp:353-362).
    player.tap(COM_DIG)?;
    player.wait_until("the Clonk starts the real Dig action", 30, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_DOWN)?;
    player.press(COM_LEFT)?;
    let depth = player.wait_until(
        "the diagonal tunnel reaches the GOLD's depth",
        140,
        |engine| {
            clonk_carries(engine, clonk, "GOLD")
                || engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 320)
        },
    );
    player.release(COM_DOWN)?;
    if let Err(error) = depth {
        player.release(COM_LEFT)?;
        return Err(Box::new(error));
    }
    let pickup = player.wait_until(
        "the steered dig tunnel naturally collects GOLD",
        180,
        |engine| clonk_carries(engine, clonk, "GOLD"),
    );
    player.release(COM_LEFT)?;
    pickup?;
    player.assert_milestone("GOLD is nested in the Clonk's inventory", |engine| {
        engine
            .object_snapshot(gold)
            .is_some_and(|object| object.container == Some(clonk))
    })?;
    player.wait_until(
        "the Clonk stops digging after collecting GOLD",
        30,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;

    // Walk out through the excavated tunnel, then use the same held Right +
    // real Up inputs to climb back to the cabin. No position, action, or
    // container state is assigned by this route.
    player.hold_until(
        COM_RIGHT,
        "the GOLD-carrying Clonk walks out of the tunnel",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 215)
        },
    )?;
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..1_800 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("GOLD-carrying Clonk survives the return");
        if clonk_now.position.x >= 558 {
            break;
        }
        let action = clonk_now.action.name.clone();
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            // Entering DFA_SCALE while Right was already held does not
            // synthesize another edge. Re-pressing the same real key lets
            // go from a left-facing wall exactly as AutoStopDirectCom does
            // for COM_Right; on this right-facing wall, DFA_SCALE converts
            // Right to Up and climbs (C4Object.cpp:3618-3628,4823-4855).
            player.release(COM_RIGHT)?;
            player.press(COM_RIGHT)?;
        } else if landed || left_scale_in_flight {
            // React on the exact transition frame, before the Clonk falls
            // back onto the wall: Up is the ordinary jump key and its
            // release recomputes the still-held Right direction.
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_RIGHT)?;
    player.assert_milestone("the GOLD-carrying Clonk reaches the cabin hill", |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 558)
    })?;
    player.wait_until(
        "the GOLD-carrying Clonk lands beside HUT2's door",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the GOLD-carrying Clonk aligns with HUT2's entrance",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 570)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the GOLD-carrying Clonk enters HUT2", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;

    player.wait_until("Tutorial01 selects Tutorial02", 240, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial02.c4s"
    })?;
    player.wait_until(
        "Tutorial01 fulfilled goal reaches GameOver",
        320,
        Engine::is_game_over,
    )?;
    let completed = player.engine().snapshot();
    assert!(
        completed
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG"),
        "Tutorial01 must fulfill its real SCRG before selecting Tutorial02"
    );
    Ok(())
}
