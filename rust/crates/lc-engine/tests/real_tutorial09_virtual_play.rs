#[allow(dead_code)]
mod support;

use std::error::Error;

use lc_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_THROW, COM_UP,
};
use support::real_scenario::load_tutorial;
use support::virtual_player::VirtualPlayer;

fn load_tutorial09() -> (Engine, i32) {
    let mut engine = load_tutorial(9, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 9 virtual player".to_owned(),
            player_info_id: 0,
            score: 0,
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
        .expect("local Tutorial09 virtual player joins")
        .number;
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

fn clonk_carries(engine: &Engine, clonk: ObjectId, definition: &str) -> bool {
    engine.object_snapshot(clonk).is_some_and(|clonk| {
        clonk.contents.iter().any(|item| {
            engine
                .object_snapshot(*item)
                .is_some_and(|item| item.definition_id == definition)
        })
    })
}

fn carried_object(engine: &Engine, clonk: ObjectId, definition: &str) -> Option<ObjectId> {
    engine
        .object_snapshot(clonk)?
        .contents
        .into_iter()
        .find(|item| {
            engine
                .object_snapshot(*item)
                .is_some_and(|item| item.definition_id == definition)
        })
}

fn carried_definition_count(engine: &Engine, clonk: ObjectId, definition: &str) -> usize {
    engine.object_snapshot(clonk).map_or(0, |clonk| {
        clonk
            .contents
            .iter()
            .filter(|item| {
                engine
                    .object_snapshot(**item)
                    .is_some_and(|item| item.definition_id == definition)
            })
            .count()
    })
}

fn closest_free_fish(engine: &Engine, clonk: ObjectId) -> Option<ObjectId> {
    let position = engine.object_snapshot(clonk)?.position;
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "FISH" && object.container.is_none())
        .min_by_key(|object| {
            (object.position.x - position.x).abs() + (object.position.y - position.y).abs()
        })
        .map(|object| object.id)
}

fn swim_until_fish_count(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    target_count: usize,
    max_cycles: u32,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..max_cycles {
        if carried_definition_count(player.engine(), clonk, "FISH") >= target_count {
            return Ok(());
        }
        let clonk_position = player
            .engine()
            .object_snapshot(clonk)
            .expect("Tutorial09 Clonk survives the fish chase")
            .position;
        let fish = closest_free_fish(player.engine(), clonk)
            .expect("Tutorial09 retains a free FISH until the target is reached");
        let fish_position = player
            .engine()
            .object_snapshot(fish)
            .expect("selected Tutorial09 FISH survives the observation")
            .position;
        let horizontal = ((fish_position.x - clonk_position.x).abs() > 3).then_some(
            if fish_position.x < clonk_position.x {
                COM_LEFT
            } else {
                COM_RIGHT
            },
        );
        let vertical = ((fish_position.y - clonk_position.y).abs() > 3).then_some(
            if fish_position.y < clonk_position.y {
                COM_UP
            } else {
                COM_DOWN
            },
        );
        if let Some(control) = horizontal {
            player.press(control)?;
        }
        if let Some(control) = vertical {
            player.press(control)?;
        }
        player.ticks(3)?;
        if let Some(control) = vertical {
            player.release(control)?;
        }
        if let Some(control) = horizontal {
            player.release(control)?;
        }
    }
    player.assert_milestone(
        format!("the Clonk catches {target_count} real FISH"),
        |engine| carried_definition_count(engine, clonk, "FISH") >= target_count,
    )?;
    Ok(())
}

fn swim_to_x(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    target_x: i32,
) -> Result<(), Box<dyn Error>> {
    let x = player
        .engine()
        .object_snapshot(clonk)
        .expect("Tutorial09 Clonk survives the return swim")
        .position
        .x;
    if x < target_x - 4 {
        player.hold_until(
            COM_RIGHT,
            "the Clonk swims below the western shore",
            180,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= target_x - 4)
            },
        )?;
    } else if x > target_x + 4 {
        player.hold_until(
            COM_LEFT,
            "the Clonk swims below the western shore",
            180,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= target_x + 4)
            },
        )?;
    }
    Ok(())
}

fn climb_onto_western_island(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
) -> Result<(), Box<dyn Error>> {
    player.press(COM_RIGHT)?;
    player.press(COM_UP)?;
    let outcome = player.wait_until(
        "the Clonk swims and scales onto the western island",
        220,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 210 && object.action.name == "Walk")
        },
    );
    let release_up = player.release(COM_UP);
    let release_right = player.release(COM_RIGHT);
    outcome?;
    release_up?;
    release_right?;
    Ok(())
}

fn resurface_near_western_shore(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..200 {
        let object = player
            .engine()
            .object_snapshot(clonk)
            .expect("Tutorial09 Clonk survives resurfacing");
        if object.action.name != "Swim" || (object.position.x <= 195 && object.position.y <= 190) {
            return Ok(());
        }
        let horizontal = if object.position.x < 176 {
            Some(COM_RIGHT)
        } else if object.position.x > 184 {
            Some(COM_LEFT)
        } else {
            None
        };
        if let Some(control) = horizontal {
            player.press(control)?;
        }
        player.press(COM_UP)?;
        player.ticks(3)?;
        player.release(COM_UP)?;
        if let Some(control) = horizontal {
            player.release(control)?;
        }
    }
    player.assert_milestone("the Clonk resurfaces at the western shore", |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name != "Swim" || (object.position.x <= 195 && object.position.y <= 190)
        })
    })?;
    Ok(())
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine
        .snapshot()
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
}

fn player_wealth(engine: &Engine, owner: i32) -> i32 {
    engine
        .snapshot()
        .hud
        .players
        .into_iter()
        .find(|player| player.owner == owner)
        .map_or(0, |player| player.wealth)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<lc_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

#[test]
fn tutorial09_virtual_player_completes_the_real_tutorial_route() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial09();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial09 joins one selected CLNK");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("Tutorial09 asks for an igloo", 240, |engine| {
        tutorial_message_contains(engine, "build an igloo")
    })?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk naturally collects CNKT",
        30,
        |engine| clonk_carries(engine, clonk, "CNKT"),
    )?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk reaches the level IGLO construction ground",
        40,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 345)
        },
    )?;

    // C++ activates only the first inventory object on DigDouble
    // (src/C4ObjectCom.cpp:531-540). Ready material is created FLAG then
    // CNKT and C4ObjectList::Add puts the last collected object first, so the
    // real CNKT opens CXCN without a test-only inventory selection.
    player.double_tap(COM_DIG)?;
    player.wait_until("CNKT opens the real CXCN menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::C4Id("CXCN".into()))
    })?;
    let igloo_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "IGLO"))
        .expect("Tutorial09 gives the player IGLO knowledge");
    player.menu_navigate_to_index(igloo_index)?;
    player.menu_enter()?;
    let igloo = player
        .wait_until("the IGLO construction site is created", 30, |engine| {
            object_with_definition(engine, "IGLO").is_some()
        })
        .map(|_| object_with_definition(player.engine(), "IGLO").expect("IGLO exists"))?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk starts building IGLO", 30, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Build")
    })?;
    player.wait_until("IGLO construction finishes", 900, |engine| {
        engine
            .object_snapshot(igloo)
            .is_some_and(|object| object.construction == 100_000)
    })?;
    player.wait_until(
        "the completed build returns the Clonk to Walk",
        30,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the Clonk naturally collects the ready FLAG",
        50,
        |engine| clonk_carries(engine, clonk, "FLAG"),
    )?;
    player.hold_until(
        COM_RIGHT,
        "the FLAG-carrying Clonk reaches IGLO's entrance",
        50,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 325)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the Clonk enters completed IGLO", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(igloo))
    })?;

    // Contained Throw synchronously executes C4CMD_Throw, which puts the
    // first carried object into the container (src/C4Object.cpp:3267-3282;
    // src/C4ObjectCom.cpp:700-712). ExecBase then attaches that direct FLAG
    // and assigns Base on Tick10 (src/C4Object.cpp:1000-1018).
    player.tap(COM_THROW)?;
    player.wait_until(
        "IGLO accepts FLAG and becomes the home base",
        40,
        |engine| {
            engine
                .object_snapshot(igloo)
                .is_some_and(|object| object.base == owner)
        },
    )?;
    player.wait_until("new base opens its context menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the Clonk exits the new IGLO", 50, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
    })?;
    player.wait_until("Tutorial09 asks the Clonk to catch fish", 180, |engine| {
        tutorial_message_contains(engine, "catch some fish")
    })?;
    player.hold_until(
        COM_LEFT,
        "the Clonk dives into the western ocean",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Swim")
        },
    )?;
    player.wait_until("Tutorial09 warns about underwater breath", 220, |engine| {
        tutorial_message_contains(engine, "hold his breath")
    })?;

    // The chase emits only ordinary simultaneous direction presses. C++
    // converts the pressed bitset to the eight-way swim direction
    // (src/C4Object.cpp:3654-3664,3729-3741); Collection then runs on Tick3
    // when the moving FISH center enters the CLNK collection area
    // (src/C4GameObjects.cpp:140-147,185-194).
    swim_until_fish_count(&mut player, clonk, 1, 400)?;
    player.wait_until("Tutorial09 asks the Clonk to sell FISH", 240, |engine| {
        tutorial_message_contains(engine, "Sell the fish")
    })?;
    swim_to_x(&mut player, clonk, 180)?;
    resurface_near_western_shore(&mut player, clonk)?;
    climb_onto_western_island(&mut player, clonk)?;
    player.hold_until(
        COM_RIGHT,
        "the fish-carrying Clonk returns to IGLO",
        100,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 325)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the fish-carrying Clonk enters IGLO", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(igloo))
    })?;
    let fish = carried_object(player.engine(), clonk, "FISH").expect("caught FISH is carried");
    player.wait_until("IGLO opens context with a Put row", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Put")?;
    player.menu_enter()?;
    player.wait_until(
        "the context Put row transfers FISH into IGLO",
        30,
        |engine| {
            engine
                .object_snapshot(fish)
                .is_some_and(|object| object.container == Some(igloo))
        },
    )?;
    player.wait_until("IGLO restores context after Put", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Sell")?;
    player.menu_enter()?;
    player.wait_until("IGLO opens the real Sell menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(5))
    })?;
    let fish_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "FISH"))
        .expect("IGLO Sell menu offers the deposited FISH");
    player.menu_navigate_to_index(fish_index)?;
    player.menu_enter()?;
    player.wait_until(
        "IGLO sells the first FISH for its overloaded value",
        40,
        |engine| player_wealth(engine, owner) == 20 && engine.object_snapshot(fish).is_none(),
    )?;

    for expected_wealth in [40, 60, 80, 100] {
        player.menu_close()?;
        player.wait_until("IGLO restores context after closing Sell", 20, |engine| {
            object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
        })?;
        player.menu_navigate_to_caption("Exit")?;
        player.menu_enter()?;
        player.wait_until("the Clonk exits IGLO for another FISH", 50, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none())
        })?;
        // Leave through IGLO's still-open western entrance before its door
        // closes (FarWorlds Igloo DefCore Entrance=-28,-4,23,18).
        player.hold_until(
            COM_LEFT,
            "the Clonk returns to the western ocean",
            300,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Swim")
            },
        )?;
        swim_until_fish_count(&mut player, clonk, 1, 400)?;
        swim_to_x(&mut player, clonk, 180)?;
        resurface_near_western_shore(&mut player, clonk)?;
        climb_onto_western_island(&mut player, clonk)?;
        player.hold_until(
            COM_RIGHT,
            "the Clonk returns to the IGLO entrance",
            100,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .zip(engine.object_snapshot(igloo))
                    .is_some_and(|(clonk, igloo)| {
                        clonk.action.name == "Walk"
                            && clonk.position.x >= igloo.position.x - 28
                    })
            },
        )?;
        player.tap(COM_UP)?;
        player.wait_until("the Clonk re-enters IGLO with FISH", 40, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(igloo))
        })?;

        let fish = carried_object(player.engine(), clonk, "FISH")
            .expect("the newly caught FISH is in the real inventory");
        player.wait_until("IGLO opens context for another FISH", 20, |engine| {
            object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
        })?;
        player.menu_navigate_to_caption("Put")?;
        player.menu_enter()?;
        player.wait_until("context Put transfers another FISH", 30, |engine| {
            engine
                .object_snapshot(fish)
                .is_some_and(|object| object.container == Some(igloo))
        })?;
        player.wait_until("IGLO restores context after another Put", 20, |engine| {
            object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
        })?;
        player.menu_navigate_to_caption("Sell")?;
        player.menu_enter()?;
        player.wait_until("IGLO restores the real Sell menu", 20, |engine| {
            object_menu_identification(engine, owner) == Some(lc_script::Value::Int(5))
        })?;
        let fish_index = player
            .engine()
            .cursor_object_menu(owner)
            .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "FISH"))
            .expect("IGLO offers the newly deposited FISH for sale");
        player.menu_navigate_to_index(fish_index)?;
        player.menu_enter()?;
        player.wait_until(
            format!("selling FISH raises wealth to {expected_wealth}"),
            40,
            |engine| {
                player_wealth(engine, owner) == expected_wealth
                    && engine.object_snapshot(fish).is_none()
            },
        )?;
    }

    player.wait_until(
        "Tutorial09 fulfills SCRG and reaches GameOver",
        600,
        |engine| engine.snapshot().game_over,
    )?;
    player.assert_milestone("Tutorial09 records its fulfilled SCRG goal", |engine| {
        engine
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG")
    })?;
    assert_eq!(
        player.engine().next_mission().path,
        "Tutorial.c4f\\Tutorial10.c4s",
        "Script11 selects Tutorial10 after wealth reaches 100 (Tutorial09/Script.c:84-89)"
    );

    Ok(())
}
