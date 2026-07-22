#![allow(dead_code)]

use std::error::Error;

use clonk_engine::{Engine, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_THROW, COM_UP};
use crate::support::real_scenario::{join_local_player, load_tutorial};
use crate::support::virtual_player::VirtualPlayer;

fn load_tutorial02() -> (Engine, i32) {
    let mut engine = load_tutorial(2, 0);
    let owner = join_local_player(&mut engine, "Tutorial 2 virtual player");
    (engine, owner)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
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
fn tutorial02_virtual_player_completes_the_real_tutorial_route() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial02();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial02 joins one selected CLNK");
    let balloon = object_with_definition(&engine, "BALN").expect("Tutorial02 places BALN");
    let hut = object_with_definition(&engine, "HUT3").expect("Tutorial02 places HUT3");
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
        "stable balloon attachment clears the central island",
        160,
        |engine| {
            let Some(clonk_now) = engine.object_snapshot(clonk) else {
                return false;
            };
            let Some(balloon_now) = engine.object_snapshot(balloon) else {
                return false;
            };
            clonk_now.action.name == "Push"
                && clonk_now.action.target == Some(balloon)
                // BALN's rideable platform extends below its object position.
                // Keep ascending until that platform, not merely BALN's origin,
                // is above the central island that the wind carries it across.
                && balloon_now.position.y <= 190
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
        clonk_engine::CommandDirection::Stop,
        "next natural milestone: BALN::ControlDownSingle currently aborts at the unknown \
         ScheduleCall before SetComDir(COMD_Stop)"
    );

    let coast_start = player.engine().object_snapshot(balloon);
    let coast = player.wait_until(
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
    );
    if let Err(error) = coast {
        panic!(
            "{error}; coast_start={coast_start:?}; clonk={:?}; balloon={:?}",
            player.engine().object_snapshot(clonk),
            player.engine().object_snapshot(balloon)
        );
    }

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
        clonk_engine::CommandDirection::Down
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
    let landing = player.wait_until("Clonk lets go in the far-island rectangle", 100, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Walk"
                && (450..710).contains(&object.position.x)
                && (250..320).contains(&object.position.y)
        })
    });
    if let Err(error) = landing {
        panic!(
            "{error}; clonk={:?}; balloon={:?}",
            player.engine().object_snapshot(clonk),
            player.engine().object_snapshot(balloon)
        );
    }

    player.hold_until(
        COM_LEFT,
        "the landed Clonk reaches a Tutorial02 collectible",
        180,
        |engine| clonk_carries(engine, clonk, "FLAG") || clonk_carries(engine, clonk, "LOAM"),
    )?;
    // Releasing a direction key does not stop DFA_WALK. Stop immediately so
    // Script3's fixed wait cannot carry the Clonk off the far island.
    player.tap(COM_DOWN)?;

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
        // Script30 says to throw toward the center of this island. Walk into
        // the LOAM pile while the FLAG still occupies the single inventory
        // slot, then face right before COM_Throw. The released FLAG lands on
        // solid ground between the pile and BALN, while the newly empty Clonk
        // naturally collects LOAM underfoot
        // instead of being collected again (Tutorial02.c4s/StringTblUS.txt:6;
        // C4Object.cpp:3410-3412,4794-4805).
        player.hold_until(
            COM_LEFT,
            "the FLAG-carrying Clonk moves left of the island centre",
            60,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= 500)
            },
        )?;
        player.tap(COM_DOWN)?;
        player.press(COM_RIGHT)?;
        player.wait_until("the stopped Clonk turns toward the island centre", 10, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.direction == clonk_engine::Direction::Right)
        })?;
        player.release(COM_RIGHT)?;
        assert_eq!(
            player
                .engine()
                .object_snapshot(clonk)
                .expect("CLNK faces the island centre before throwing FLAG")
                .direction,
            clonk_engine::Direction::Right
        );
        // Classic controls intentionally keep walking after a direction key
        // is released. Down is the physical stop control for DFA_WALK; use it
        // before throwing so the Clonk does not follow the FLAG off the
        // corrected-map island (C4Object.cpp:3410-3412).
        player.tap(COM_DOWN)?;
        player.tap(COM_THROW)?;
        let dropped_flag =
            player.wait_until("the flag leaves the Clonk's inventory", 30, |engine| {
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

    if !clonk_carries(player.engine(), clonk, "LOAM") {
        let pickup_prompt = player.wait_until(
            "Tutorial02 asks for or naturally collects a loam chunk",
            450,
            |engine| {
                tutorial_message_contains(engine, "Pick up one of the loam chunks")
                    || clonk_carries(engine, clonk, "LOAM")
            },
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
    let move_left_prompt = player.wait_until(
        "Tutorial02 tells the Clonk to move to the island's left edge",
        450,
        |engine| tutorial_message_contains(engine, "Now move to the very left edge"),
    );
    if let Err(error) = move_left_prompt {
        panic!(
            "{error}; clonk={:?}; flag={:?}; hud={:?}",
            player.engine().object_snapshot(clonk),
            object_with_definition(player.engine(), "FLAG")
                .and_then(|id| player.engine().object_snapshot(id)),
            player.engine().snapshot().hud
        );
    }
    // The ready crew's restored Exit InitEvaluation frame shifts the
    // phase-sensitive walking route by one global tick. Stop one pixel
    // earlier so three exact (-16,-16) spans still finish inside Script81's
    // literal y>=240 rectangle rather than supported at y=239.
    player.hold_until(
        COM_LEFT,
        "Clonk reaches Tutorial02's first bridge position",
        120,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (489..=491).contains(&object.position.x)
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
        engine
            .cursor_object_menu(owner)
            .is_some_and(|(_, menu)| menu.identification == clonk_script::Value::C4Id("LMMS".into()))
    })?;
    player.wait_until(
        "Tutorial02 observes LMMS and asks for diagonal-left",
        180,
        |engine| tutorial_message_contains(engine, "Select the option 'diagonal left'"),
    )?;
    player.menu_left()?;
    let selected = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| {
            usize::try_from(menu.selection)
                .ok()
                .map(|index| (menu, index))
        })
        .and_then(|(menu, index)| menu.items.get(index))
        .map(|item| item.caption.as_str());
    assert_eq!(selected, Some("Diagonal left"));

    let bridge_start = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK starts the bridge")
        .position;
    player.menu_enter()?;
    player.wait_until("CLNK enters the real Bridge action", 10, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Bridge")
    })?;
    player.ticks(6)?;
    let first_bridge_step = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK survives the first bridge step");
    assert_eq!(first_bridge_step.action.name, "Bridge");
    assert_eq!(first_bridge_step.action.time, 6);
    assert_eq!(
        first_bridge_step.action.data, 0x0064_0110,
        "real LOAM must request the C++ moving, non-wall Earth bridge"
    );
    assert_eq!(
        first_bridge_step.position,
        clonk_engine::Vector2::new(bridge_start.x - 1, bridge_start.y - 1),
        "C++ DoBridge advances the moving UpLeft bridge for the first time at Action.Time 6 \
         (src/C4Object.cpp:4581-4652)"
    );
    player.wait_until(
        "the 100-frame moving diagonal bridge completes",
        114,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    let bridge_end = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK survives the bridge")
        .position;
    assert_eq!(
        (bridge_end.x - bridge_start.x, bridge_end.y - bridge_start.y),
        (-16, -16),
        "C++ increments Action.Time before DoBridge and moves at times 6..96 \
         (src/C4Object.cpp:4581-4601,4755-4756)"
    );
    let earth = player
        .engine()
        .materials()
        .id_of("Earth")
        .expect("Tutorial02 loads Earth");
    let landscape = player
        .engine()
        .landscape()
        .expect("Tutorial02 keeps its landscape");
    let mut earth_pixels_per_step = Vec::new();
    for step in 0..16 {
        let x = bridge_start.x - step - 4;
        let y = bridge_start.y - step + 9;
        let mut earth_pixels = 0;
        for pixel_y in y..y + 3 {
            for pixel_x in x..x + 4 {
                earth_pixels += usize::from(landscape.material_at(pixel_x, pixel_y) == Some(earth));
            }
        }
        earth_pixels_per_step.push(earth_pixels);
    }
    assert!(
        earth_pixels_per_step.contains(&12),
        "at least one real 4x3 bridge rectangle in open air must be all Earth; \
         per-step Earth pixels={earth_pixels_per_step:?}"
    );

    player.wait_until(
        "Tutorial02 asks for three diagonal bridges",
        180,
        |engine| tutorial_message_contains(engine, "build three diagonal bridges"),
    )?;
    player.hold_until(
        COM_RIGHT,
        "Clonk walks back over the first bridge and collects a second LOAM",
        220,
        |engine| clonk_carries(engine, clonk, "LOAM"),
    )?;
    player.hold_until(
        COM_LEFT,
        "Clonk returns to the first bridge's upper-left endpoint",
        220,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.container.is_none()
                    && object.action.name == "Walk"
                    && object.position.x <= bridge_end.x
            })
        },
    )?;

    // The repeated Dig control opens the carried LOAM's construction menu;
    // selecting its UpLeft item executes the same LOAM::StartBridge path as
    // the first span (Loam.c4d/Script.c:31-60,82-97).
    player.double_tap(COM_DIG)?;
    player.wait_until("second LOAM opens LMMS", 20, |engine| {
        engine
            .cursor_object_menu(owner)
            .is_some_and(|(_, menu)| menu.identification == clonk_script::Value::C4Id("LMMS".into()))
    })?;
    player.menu_navigate_to_caption("Diagonal left")?;
    let second_bridge_start = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK starts the second bridge")
        .position;
    assert!(
        (second_bridge_start.x - bridge_end.x).abs() <= 1
            && (second_bridge_start.y - bridge_end.y).abs() <= 1,
        "the second bridge must continue the first at its upper-left endpoint; \
         first_end={bridge_end:?}, second_start={second_bridge_start:?}"
    );
    player.menu_enter()?;
    player.wait_until("CLNK enters its second Bridge action", 10, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Bridge")
    })?;
    player.wait_until("the second UpLeft bridge completes", 114, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    let second_bridge_end = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK survives the second bridge")
        .position;
    assert_eq!(
        (
            second_bridge_end.x - second_bridge_start.x,
            second_bridge_end.y - second_bridge_start.y,
        ),
        (-16, -16),
        "the second real LOAM span must use C++ DoBridge's 16 moving UpLeft steps \
         (src/C4Object.cpp:4581-4652)"
    );

    player.hold_until(
        COM_RIGHT,
        "Clonk crosses both spans and reaches the remaining materials",
        260,
        |engine| clonk_carries(engine, clonk, "LOAM") || clonk_carries(engine, clonk, "FLAG"),
    )?;
    if clonk_carries(player.engine(), clonk, "FLAG") {
        // FLAG settles among the LOAM pile after Script30. If object-list
        // order recollects it first, naturally throw it toward the far
        // island's right half and continue to the adjacent LOAM.
        player.tap(COM_THROW)?;
        player.wait_until("recollected FLAG leaves the Clonk", 30, |engine| {
            !clonk_carries(engine, clonk, "FLAG")
        })?;
        player.hold_until(
            COM_RIGHT,
            "Clonk collects the third LOAM after rethrowing FLAG",
            100,
            |engine| clonk_carries(engine, clonk, "LOAM"),
        )?;
    }
    player.hold_until(
        COM_LEFT,
        "Clonk approaches the second bridge's upper-left endpoint",
        260,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.container.is_none()
                    && object.action.name == "Walk"
                    && object.position.x <= second_bridge_end.x
            })
        },
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("third LOAM opens LMMS", 20, |engine| {
        engine
            .cursor_object_menu(owner)
            .is_some_and(|(_, menu)| menu.identification == clonk_script::Value::C4Id("LMMS".into()))
    })?;
    player.menu_navigate_to_caption("Diagonal left")?;
    let third_bridge_start = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK starts the third bridge")
        .position;
    assert!(
        (third_bridge_start.x - second_bridge_end.x).abs() <= 1
            && (third_bridge_start.y - second_bridge_end.y).abs() <= 1,
        "the third bridge must continue the second at its upper-left endpoint; \
         second_end={second_bridge_end:?}, third_start={third_bridge_start:?}"
    );
    player.menu_enter()?;
    player.wait_until("CLNK enters its third Bridge action", 10, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Bridge")
    })?;
    player.wait_until("the third UpLeft bridge completes", 114, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    let third_bridge_end = player
        .engine()
        .object_snapshot(clonk)
        .expect("CLNK survives the third bridge")
        .position;
    assert_eq!(
        (
            third_bridge_end.x - third_bridge_start.x,
            third_bridge_end.y - third_bridge_start.y,
        ),
        (-16, -16),
        "the third real LOAM span must use C++ DoBridge's 16 moving UpLeft steps \
         (src/C4Object.cpp:4581-4652)"
    );
    let three_bridge_delta = (
        third_bridge_end.x - bridge_start.x,
        third_bridge_end.y - bridge_start.y,
    );
    assert!(
        (three_bridge_delta.0 + 48).abs() <= 2
            && (three_bridge_delta.1 + 48).abs() <= 2
            && (360..445).contains(&third_bridge_end.x)
            && (240..290).contains(&third_bridge_end.y),
        "three contiguous C++ moving UpLeft bridges must reach Script81's literal gate; \
         delta={three_bridge_delta:?}, end={third_bridge_end:?} \
         (Tutorial02.c4s/Script.c:185-198)"
    );

    // Script81 recognizes the third span's endpoint. Script82 deliberately
    // sends the player back over all three spans for FLAG before the two
    // jumps home (Tutorial02.c4s/Script.c:185-202; StringTblUS.txt:13).
    player.wait_until(
        "Tutorial02 recognizes three contiguous bridges",
        180,
        |engine| tutorial_message_contains(engine, "close enough to jump"),
    )?;

    player.hold_until(
        COM_RIGHT,
        "Clonk reaches the remaining far-island pickup",
        420,
        |engine| clonk_carries(engine, clonk, "FLAG") || clonk_carries(engine, clonk, "LOAM"),
    )?;
    if clonk_carries(player.engine(), clonk, "LOAM") {
        // CreateMaterial supplies four LOAM chunks although Script80 asks
        // for three bridges. Put the spare behind the returning Clonk so its
        // one-slot inventory can collect FLAG (Tutorial02.c4s/Script.c:27-34).
        player.press(COM_LEFT)?;
        player.ticks(1)?;
        player.release(COM_LEFT)?;
        player.tap(COM_DOWN)?;
        player.tap(COM_THROW)?;
        player.wait_until("spare LOAM leaves the Clonk", 30, |engine| {
            !clonk_carries(engine, clonk, "LOAM")
        })?;
        player.wait_until("Clonk finishes throwing spare LOAM", 30, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        })?;
    }
    let flag_id = object_with_definition(player.engine(), "FLAG").expect("FLAG remains");
    let flag_x = player
        .engine()
        .object_snapshot(flag_id)
        .expect("FLAG remains visible")
        .position
        .x;
    let flag_approach = player.hold_until(
        COM_RIGHT,
        "Clonk walks back into FLAG's collection window",
        180,
        |engine| {
            clonk_carries(engine, clonk, "FLAG")
                || engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && object.position.x >= flag_x - 6
                })
        },
    );
    if let Err(error) = flag_approach {
        let snapshot = player.engine().snapshot();
        let flag = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "FLAG");
        let loam = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "LOAM")
            .collect::<Vec<_>>();
        panic!(
            "{error}; clonk={:?}; flag={flag:?}; loam={loam:?}",
            player.engine().object_snapshot(clonk)
        );
    }
    // C4GameObjects performs collection on Tick3. Stop while FLAG is inside
    // CLNK's exact -8..+7 collection span and let that contact pass run,
    // instead of holding Right past the island when pickup is one tick late.
    player.tap(COM_DOWN)?;
    if clonk_carries(player.engine(), clonk, "LOAM") {
        // Four chunks exist for three bridges. Crossing the pile may fill the
        // one-slot inventory again, so throw the final spare back to the left
        // while stopped beside FLAG.
        player.press(COM_LEFT)?;
        player.ticks(1)?;
        player.release(COM_LEFT)?;
        player.tap(COM_DOWN)?;
        player.tap(COM_THROW)?;
        player.wait_until("last spare LOAM leaves the Clonk", 30, |engine| {
            !clonk_carries(engine, clonk, "LOAM")
        })?;
        player.wait_until("Clonk finishes throwing the last spare LOAM", 30, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        })?;
    }
    let flag_pickup = player.wait_until("FLAG enters the Clonk's inventory", 12, |engine| {
        clonk_carries(engine, clonk, "FLAG")
    });
    if let Err(error) = flag_pickup {
        panic!(
            "{error}; clonk={:?}; flag={:?}",
            player.engine().object_snapshot(clonk),
            player.engine().object_snapshot(flag_id)
        );
    }
    let carried_flag = player
        .engine()
        .object_snapshot(clonk)
        .and_then(|object| object.contents.first().copied())
        .and_then(|id| player.engine().object_snapshot(id));
    assert!(
        carried_flag.is_some_and(|object| object.definition_id == "FLAG"),
        "C++ content sorting must place newly collected FLAG in inventory slot zero, \
         because contained Throw deposits that first slot \
         (src/C4ObjectList.cpp:151-175; src/C4ObjectCom.cpp:591-622)"
    );

    player.press(COM_LEFT)?;
    player.wait_until(
        "FLAG-carrying Clonk returns to the third bridge endpoint",
        420,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && object.position.x <= third_bridge_end.x
            })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until(
        "FLAG-carrying Clonk lands on the center island",
        140,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (290..390).contains(&object.position.x)
            })
        },
    )?;
    player.wait_until(
        "Clonk reaches the center island's left jump edge",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 315)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until(
        "FLAG-carrying Clonk lands on the home island",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 230)
        },
    )?;

    let hut_position = player
        .engine()
        .object_snapshot(hut)
        .expect("HUT3 survives the return trip")
        .position;
    player.wait_until("Clonk reaches HUT3's entrance", 160, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Walk"
                && (hut_position.x + 2..hut_position.x + 19).contains(&object.position.x)
                && (hut_position.y + 4..hut_position.y + 25).contains(&object.position.y)
        })
    })?;
    player.release(COM_LEFT)?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(hut)
            .expect("HUT3 before FLAG return")
            .base,
        -1,
        "without its FlyBase FLAG, C++ ExecBase must clear HUT3's base \
         (src/C4Object.cpp:1000-1031)"
    );
    player.tap(COM_UP)?;
    player.wait_until(
        "Clonk enters HUT3 through its real entrance",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        },
    )?;
    player.wait_until(
        "Tutorial02 asks the contained Clonk to put FLAG into HUT3",
        240,
        |engine| tutorial_message_contains(engine, "Press 'throw' to put the flag"),
    )?;
    player.tap(COM_THROW)?;
    player.wait_until("FLAG enters HUT3", 80, |engine| {
        object_with_definition(engine, "FLAG")
            .and_then(|flag| engine.object_snapshot(flag))
            .is_some_and(|flag| flag.container == Some(hut))
    })?;
    player.wait_until("HUT3 restores player zero as its base", 80, |engine| {
        engine
            .object_snapshot(hut)
            .is_some_and(|hut| hut.base == owner)
    })?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(hut)
            .expect("HUT3 after FLAG return")
            .base,
        owner,
        "FLAG return must restore HUT3 as player zero's C++ base"
    );
    player.wait_until("Tutorial02 selects Tutorial03", 180, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial03.c4s"
    })?;
    assert_eq!(
        player.engine().next_mission().path,
        r"Tutorial.c4f\Tutorial03.c4s"
    );
    player.wait_until(
        "Tutorial02 fulfilled goal reaches GameOver",
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
        "Tutorial02 must fulfill its real SCRG before selecting Tutorial03"
    );
    Ok(())
}
