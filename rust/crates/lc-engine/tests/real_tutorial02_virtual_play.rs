#[allow(dead_code)]
mod support;

use std::error::Error;

use lc_engine::{Engine, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_THROW, COM_UP};
use support::real_scenario::{join_local_player, load_tutorial};
use support::virtual_player::VirtualPlayer;

fn load_tutorial02() -> (Engine, i32) {
    let mut engine = load_tutorial(2, 0);
    let owner = join_local_player(&mut engine, "Tutorial 2 virtual player");
    (engine, owner)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == definition)
        .map(|object| object.id)
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine
        .snapshot()
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
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

#[test]
fn tutorial02_virtual_player_flies_to_the_far_island_and_collects_loam(
) -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial02();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial02 joins one selected CLNK");
    let balloon = object_with_definition(&engine, "BALN").expect("Tutorial02 places BALN");
    assert_eq!(
        engine.debug_definition_has_function("BALN", "ControlDownSingle"),
        Some(true),
        "the real BALN definition must expose its classic lowering control"
    );
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until(
        "ready crew and balloon leave the first base",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
                && engine
                    .object_snapshot(balloon)
                    .is_some_and(|object| object.container.is_none())
        },
    )?;

    // The natural classic-control route is the same one C++ teaches:
    // repeated Down becomes COM_Down_D (src/C4Player.cpp:1522-1536), which
    // queues Grab and ultimately enters DFA_PUSH (src/C4ObjectCom.cpp:247-259,
    // 573-588). No object action or position is assigned by this test.
    player.double_tap(COM_DOWN)?;
    player.wait_until("Clonk grabs the balloon", 80, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon)
        })
    })?;

    let boarded_clonk = player
        .engine()
        .object_snapshot(clonk)
        .expect("boarded CLNK");
    let boarded_balloon = player
        .engine()
        .object_snapshot(balloon)
        .expect("boarded BALN");
    let platform_delta_y = boarded_clonk.position.y - boarded_balloon.position.y;

    // While pushing, Up is offered to BALN first (src/C4Object.cpp:3520-3537)
    // and BALN::ControlUp starts its Float procedure upward. Wind2Float moves
    // it horizontally with the scenario wind while DFA_PUSH carries the CLNK.
    player.press(COM_UP)?;
    player.wait_until(
        "stable balloon attachment reaches the flight corridor",
        100,
        |engine| {
            let Some(clonk_now) = engine.object_snapshot(clonk) else {
                return false;
            };
            let Some(balloon_now) = engine.object_snapshot(balloon) else {
                return false;
            };
            clonk_now.action.name == "Push"
                && clonk_now.action.target == Some(balloon)
                && balloon_now.position.y <= 275
                && (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1
        },
    )?;
    player.release(COM_UP)?;

    // A delayed DownSingle toggles BALN from Up to Stop in classic controls
    // (Balloon.c4d/Script.c:32-43). Stop deliberately enables the
    // IntWindYDir drift effect (:126-149); it is not an altitude hold.
    player.tap(COM_DOWN)?;
    player.ticks(11)?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(balloon)
            .expect("BALN survives DownSingle")
            .command_direction,
        lc_engine::CommandDirection::Stop,
        "next natural milestone: BALN::ControlDownSingle currently aborts at the unknown \
         ScheduleCall before SetComDir(COMD_Stop)"
    );

    player.wait_until(
        "the stopped balloon coasts to the far island longitude",
        600,
        |engine| {
            let Some(clonk_now) = engine.object_snapshot(clonk) else {
                return false;
            };
            let Some(balloon_now) = engine.object_snapshot(balloon) else {
                return false;
            };
            clonk_now.action.name == "Push"
                && clonk_now.action.target == Some(balloon)
                && balloon_now.position.x >= 520
        },
    )?;

    // A second DownSingle, after the C4DoubleClick window, changes Stop to
    // Down so the Clonk enters Script3's island rectangle. Sending it sooner
    // would synthesize COM_Down_D and ungrab instead (C4Player.cpp:1213-1228,
    // 1490-1553; C4Object.cpp:3520-3567).
    player.tap(COM_DOWN)?;
    player.ticks(11)?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(balloon)
            .expect("BALN survives second DownSingle")
            .command_direction,
        lc_engine::CommandDirection::Down
    );
    let far_island = player.wait_until(
        "Tutorial02 Script3 far-island flight rectangle while still attached",
        240,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && (450..710).contains(&object.position.x)
                    && (250..320).contains(&object.position.y)
            })
        },
    );
    if let Err(error) = far_island {
        panic!(
            "{error}; clonk={:?}; balloon={:?}",
            player.engine().object_snapshot(clonk),
            player.engine().object_snapshot(balloon)
        );
    }

    player.wait_until(
        "Tutorial02 Script3 presents the balloon-release instruction",
        30,
        |engine| tutorial_message_contains(engine, "Let go of the balloon"),
    )?;

    // The tutorial's next natural input is the repeated Down taught by
    // Script3. BALN has no ControlDownDouble override, so DFA_PUSH handles it
    // as ObjectComUnGrab (src/C4Object.cpp:3520-3567).
    player.double_tap(COM_DOWN)?;
    let landing = player.wait_until("Clonk lets go and lands on the far island", 100, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Walk"
                && (450..710).contains(&object.position.x)
                && (270..320).contains(&object.position.y)
        })
    });
    if let Err(error) = landing {
        panic!(
            "{error}; clonk={:?}; balloon={:?}",
            player.engine().object_snapshot(clonk),
            player.engine().object_snapshot(balloon)
        );
    }

    player.wait_until("the landing collectible contact resolves", 20, |engine| {
        clonk_carries(engine, clonk, "FLAG") || clonk_carries(engine, clonk, "LOAM")
    })?;

    // Script3's wait(20) resumes the scenario script after 200 frames; Script4
    // observes the completed ungrab. The C++ counter then visits the missing
    // Script6..Script19 names at its 10-frame cadence before Script20 introduces
    // the collectibles placed by CreateMaterial (Tutorial02/Script.c:27-34,
    // 65-105; C4ScriptHost.cpp:222-231).
    if clonk_carries(player.engine(), clonk, "FLAG") {
        player.wait_until(
            "Tutorial02 asks the player to put down the accidentally collected flag",
            450,
            |engine| tutorial_message_contains(engine, "Please drop the flag for now"),
        )?;
        player.tap(COM_THROW)?;
        let dropped_flag = player.wait_until("the flag leaves the Clonk's inventory", 30, |engine| {
            !clonk_carries(engine, clonk, "FLAG")
        });
        if let Err(error) = dropped_flag {
            let flag = object_with_definition(player.engine(), "FLAG")
                .and_then(|id| player.engine().object_snapshot(id));
            panic!(
                "{error}; clonk={:?}; flag={flag:?}",
                player.engine().object_snapshot(clonk)
            );
        }
    }

    let pickup_prompt = player.wait_until(
        "Tutorial02 asks the player to pick up a loam chunk",
        450,
        |engine| tutorial_message_contains(engine, "Pick up one of the loam chunks"),
    );
    if let Err(error) = pickup_prompt {
        panic!(
            "{error}; clonk={:?}; balloon={:?}; global_effects={:?}; hud={:?}",
            player.engine().object_snapshot(clonk),
            player.engine().object_snapshot(balloon),
            player.engine().global_effects(),
            player.engine().snapshot().hud
        );
    }

    if !clonk_carries(player.engine(), clonk, "LOAM") {
        let collected = player.hold_until(
            COM_LEFT,
            "Clonk naturally collects Tutorial02 loam",
            180,
            |engine| clonk_carries(engine, clonk, "LOAM"),
        );
        if let Err(error) = collected {
            panic!(
                "{error}; clonk={:?}; loam={:?}; flag={:?}",
                player.engine().object_snapshot(clonk),
                object_with_definition(player.engine(), "LOAM")
                    .and_then(|id| player.engine().object_snapshot(id)),
                object_with_definition(player.engine(), "FLAG")
                    .and_then(|id| player.engine().object_snapshot(id))
            );
        }
    }

    // Script40..Script42 advances only after FindObject sees the Clonk's
    // center in (460,280,30,30), then GetMenu observes LMMS
    // (Tutorial02/Script.c:129-149; C4Script.cpp:1418-1424).
    player.tap(COM_DOWN)?;
    player.wait_until(
        "Tutorial02 tells the Clonk to move to the island's left edge",
        120,
        |engine| tutorial_message_contains(engine, "Now move to the very left edge"),
    )?;
    player.hold_until(
        COM_LEFT,
        "Clonk reaches Tutorial02's first bridge position",
        120,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (488..=490).contains(&object.position.x)
            })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until(
        "Tutorial02 asks for a double Dig activation",
        180,
        |engine| tutorial_message_contains(engine, "Press the 'dig' key twice quickly"),
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("LOAM opens its real construction menu", 10, |engine| {
        engine.cursor_object_menu(owner).is_some_and(|(_, menu)| {
            menu.identification == lc_script::Value::C4Id("LMMS".into())
        })
    })?;
    player.wait_until(
        "Tutorial02 observes LMMS and asks for diagonal-left",
        180,
        |engine| tutorial_message_contains(engine, "Select the option 'diagonal left'"),
    )?;
    Ok(())
}
