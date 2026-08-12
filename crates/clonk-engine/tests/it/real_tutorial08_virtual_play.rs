#![allow(dead_code)]

use std::error::Error;

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_SPECIAL2,
    COM_THROW, COM_UP,
};

fn load_tutorial08() -> (Engine, i32) {
    // C4Game::InitAnimals uses the scenario RNG and PlaceAnimal for every
    // WIPF. Seed 202 deterministically places all ten WIPFs on the walkable
    // surface under MapSeed propagation while exercising the C++ path.
    let mut engine = load_tutorial(8, 202);
    let owner = crate::support::TestValueExt::test_value(engine.join_player(JoinPlayerConfig {
        name: "Tutorial 8 virtual player".to_owned(),
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
        control_style: false,
        auto_context_menu: false,
        startup_player_count: 1,
    }))
    .number();
    (engine, owner)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn definition_count(engine: &Engine, definition: &str) -> usize {
    engine.object_count_for_definition(definition)
}

fn contained_definition_count(engine: &Engine, container: ObjectId, definition: &str) -> usize {
    engine.object_count_for_definition_in_container(definition, container)
}

fn carried_object(engine: &Engine, carrier: ObjectId) -> Option<ObjectId> {
    engine
        .object_snapshot(carrier)
        .and_then(|carrier| carrier.contents.first().copied())
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn wait_for_tutorial_message_lines(
    engine: &mut Engine,
    owner: i32,
    expected: &[&str],
    max_ticks: usize,
) -> u64 {
    let expected = expected
        .iter()
        .map(|line| (*line).to_string())
        .collect::<Vec<_>>();
    for _ in 0..max_ticks {
        let frame = crate::support::TestValueExt::test_value(engine.tick());
        let lines = frame
            .hud
            .messages
            .iter()
            .filter(|message| {
                message.player == Some(owner) && message.decoration.as_deref() == Some("DECO")
            })
            .flat_map(|message| message.lines.iter().cloned())
            .collect::<Vec<_>>();
        if lines == expected {
            return frame.frame;
        }
    }
    panic!("Tutorial08 did not reach tutorial message state {expected:?} in {max_ticks} ticks");
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

fn release_direction_controls(player: &mut VirtualPlayer<'_>) -> Result<(), Box<dyn Error>> {
    for control in [COM_LEFT, COM_RIGHT, COM_UP, COM_DOWN] {
        player.release(control)?;
    }
    Ok(())
}

fn move_next_to(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    target: ObjectId,
    milestone: &str,
) -> Result<(), Box<dyn Error>> {
    release_direction_controls(player)?;
    player.ticks(12)?;
    let direction = crate::support::TestValueExt::test_value(
        player
            .engine()
            .object_snapshot(clonk)
            .zip(player.engine().object_snapshot(target))
            .map(|(clonk, target)| {
                if clonk.position.x < target.position.x {
                    COM_RIGHT
                } else {
                    COM_LEFT
                }
            }),
    );
    player.hold_until(direction, milestone, 1_200, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(target))
            .is_some_and(|(clonk, target)| {
                (clonk.position.x - target.position.x).abs() <= 8
                    && (clonk.position.y - target.position.y).abs() <= 18
            })
    })?;
    Ok(())
}

fn load_carried_object_into_lorry(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    lorry: ObjectId,
) -> Result<(), Box<dyn Error>> {
    let item = crate::support::TestValueExt::test_value(carried_object(player.engine(), clonk));
    move_next_to(player, clonk, lorry, "the Clonk returns to LORY")?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs LORY for loading", 80, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    })?;
    player.tap(COM_THROW)?;
    player.wait_until("the carried object enters LORY", 60, |engine| {
        engine
            .object_snapshot(item)
            .is_some_and(|object| object.container == Some(lorry))
    })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk releases LORY after loading", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    Ok(())
}

#[test]
fn tutorial08_virtual_player_completes_the_real_scenario() -> Result<(), Box<dyn Error>> {
    let (mut engine, owner) = load_tutorial08();
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let lorry = crate::support::TestValueExt::test_value(object_with_definition(&engine, "LORY"));
    let hut = crate::support::TestValueExt::test_value(object_with_definition(&engine, "HUT3"));
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("Tutorial08 teaches catching WIPFs", 500, |engine| {
        tutorial_message_contains(engine, "catch them either by hand or with the lorry")
    })?;
    // Seed 202 still has exactly the scenario-required ten WIPFs here.
    assert_eq!(definition_count(player.engine(), "WIPF"), 10);

    // Sweep the actual surface with ordinary Left/Right controls. Any
    // collectible encountered is loaded through LORY's real Push/Throw path;
    // incidental material therefore cannot occupy the single inventory slot
    // and hide a later WIPF pickup.
    for _ in 0..40 {
        if contained_definition_count(player.engine(), lorry, "WIPF") == 10 {
            break;
        }
        if carried_object(player.engine(), clonk).is_none() {
            release_direction_controls(&mut player)?;
            player.ticks(12)?;
            let mut search_control = if player
                .engine()
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x < 400)
            {
                COM_RIGHT
            } else {
                COM_LEFT
            };
            player.press(search_control)?;
            for _ in 0..4_000 {
                player.ticks(1)?;
                if carried_object(player.engine(), clonk).is_some() {
                    break;
                }
                let x = crate::support::TestValueExt::test_value(
                    player.engine().object_snapshot(clonk),
                )
                .position
                .x;
                let next_control = if x <= 8 {
                    COM_RIGHT
                } else if x >= 790 {
                    COM_LEFT
                } else {
                    search_control
                };
                if next_control != search_control {
                    player.release(search_control)?;
                    player.ticks(12)?;
                    search_control = next_control;
                    player.press(search_control)?;
                }
            }
            player.release(search_control)?;
        }
        player.assert_milestone("the Clonk catches a surface object", |engine| {
            carried_object(engine, clonk).is_some()
        })?;
        load_carried_object_into_lorry(&mut player, clonk, lorry)?;
    }
    assert_eq!(
        contained_definition_count(player.engine(), lorry, "WIPF"),
        10,
        "the real surface sweep loads all ten WIPFs into LORY"
    );

    // LORY::Entrance unloads its contents into structures. Push its center
    // into HUT3's real entrance rectangle (2,4,17,21), then use DFA_PUSH Up
    // to enter the grabbed vehicle exactly like C++ ObjectComEnter.
    move_next_to(
        &mut player,
        clonk,
        lorry,
        "the Clonk returns to loaded LORY",
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs loaded LORY", 80, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    })?;
    let toward_hut = crate::support::TestValueExt::test_value(
        player
            .engine()
            .object_snapshot(lorry)
            .zip(player.engine().object_snapshot(hut))
            .map(|(lorry, hut)| {
                if lorry.position.x < hut.position.x + 10 {
                    COM_RIGHT
                } else {
                    COM_LEFT
                }
            }),
    );
    player.hold_until(
        toward_hut,
        "loaded LORY aligns with HUT3's entrance",
        600,
        |engine| {
            engine
                .object_snapshot(lorry)
                .zip(engine.object_snapshot(hut))
                .is_some_and(|(lorry, hut)| {
                    let dx = lorry.position.x - hut.position.x;
                    let dy = lorry.position.y - hut.position.y;
                    (2..19).contains(&dx) && (4..25).contains(&dy)
                })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("loaded LORY enters HUT3", 80, |engine| {
        engine
            .object_snapshot(lorry)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("LORY unloads all WIPFs into HUT3", 80, |engine| {
        contained_definition_count(engine, hut, "WIPF") == 10
    })?;

    player.wait_until("the Clonk releases contained LORY", 80, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    let toward_entrance = crate::support::TestValueExt::test_value(
        player
            .engine()
            .object_snapshot(clonk)
            .zip(player.engine().object_snapshot(hut))
            .map(|(clonk, hut)| {
                if clonk.position.x < hut.position.x + 10 {
                    COM_RIGHT
                } else {
                    COM_LEFT
                }
            }),
    );
    player.hold_until(
        toward_entrance,
        "the Clonk aligns with HUT3's entrance",
        300,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(hut))
                .is_some_and(|(clonk, hut)| {
                    let dx = clonk.position.x - hut.position.x;
                    (2..19).contains(&dx) && clonk.action.name == "Walk"
                })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the Clonk enters HUT3", 80, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;

    // With classic auto-context disabled, contained Dig opens the friendly
    // base's real Sell menu directly. Special2 is C++ COM_MenuEnterAll and
    // executes the grouped WIPF row's Command2, selling all ten at once.
    player.tap(COM_DIG)?;
    player.wait_until("HUT3 opens its real Sell menu", 40, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(5))
    })?;
    let wipf_index = crate::support::TestValueExt::test_value(
        player
            .engine()
            .cursor_object_menu(owner)
            .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "WIPF")),
    );
    player.menu_navigate_to_index(wipf_index)?;
    player.tap(COM_SPECIAL2)?;
    player.wait_until(
        "selling all WIPFs removes the objective animals",
        120,
        |engine| definition_count(engine, "WIPF") == 0,
    )?;

    player.wait_until(
        "Tutorial08 fulfills SCRG and reaches GameOver",
        800,
        Engine::is_game_over,
    )?;
    player.assert_milestone("Tutorial08 records its fulfilled SCRG goal", |engine| {
        engine
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG")
    })?;
    assert_eq!(
        player.engine().next_mission().path,
        r"Tutorial.c4f\Tutorial09.c4s"
    );
    Ok(())
}

#[test]
fn tutorial08_empty_messages_clear_each_instruction_at_cpp_timing() {
    let (mut engine, owner) = load_tutorial08();

    // The real Script2/4/6 sections display permanent TutorialMessage text;
    // Script3/5/7 pass an explicit empty string to clear it, with waits of
    // 20 and 5 script ticks between transitions
    // (Tutorial08.c4s/Script.c:37-70; Tutorial.c:22-37).
    let script2 = wait_for_tutorial_message_lines(
        &mut engine,
        owner,
        &["These furry little creatures enjoy running and jumping around the place."],
        500,
    );
    let script3 = wait_for_tutorial_message_lines(&mut engine, owner, &[], 250);
    let script4 = wait_for_tutorial_message_lines(
        &mut engine,
        owner,
        &["You can catch them either by hand or with the lorry."],
        100,
    );
    let script5 = wait_for_tutorial_message_lines(&mut engine, owner, &[], 250);
    let script6 = wait_for_tutorial_message_lines(
        &mut engine,
        owner,
        &["Your goal is to catch all wipfs and sell them at your homebase."],
        100,
    );
    let script7 = wait_for_tutorial_message_lines(&mut engine, owner, &[], 250);

    // C4GameScriptHost invokes one ScriptN on each Tick10, and wait(n)
    // resumes through Schedule after n*10 frames (C4ScriptHost.cpp:222-230;
    // Tutorial.c:33-37). Each empty CustomMessage must first remove the
    // same-player/same-position permanent message, then add nothing
    // (C4Script.cpp:5995-6039; C4GameMessage.cpp:290-305,332-337).
    assert_eq!(
        (script2, script3, script4, script5, script6, script7),
        (120, 320, 370, 570, 620, 820)
    );
    assert_eq!(script3 - script2, 200);
    assert_eq!(script4 - script3, 50);
    assert_eq!(script5 - script4, 200);
    assert_eq!(script6 - script5, 50);
    assert_eq!(script7 - script6, 200);
}
