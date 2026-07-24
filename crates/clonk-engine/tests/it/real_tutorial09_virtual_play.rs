#![allow(dead_code)]

use std::error::Error;

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::{VirtualPlayer, VirtualPlayerError};
use clonk_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_THROW, COM_UP,
};

fn load_tutorial09() -> (Engine, i32) {
    let mut engine = load_tutorial(9, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 9 virtual player".to_owned(),
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
        .expect("local Tutorial09 virtual player joins")
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

fn leave_igloo_for_western_ocean(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
) -> Result<(), Box<dyn Error>> {
    let reaches_ocean = |engine: &Engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Swim")
    };
    // Dynamic snow/PXS can eventually block IGLO's exact C++ exit midpoint
    // and more of the western slope. A player can clear each obstruction
    // with the ordinary Dig control, then keep walking left into the ocean.
    for attempt in 0..8 {
        match player.hold_until(
            COM_LEFT,
            format!("the Clonk returns to the western ocean (attempt {attempt})"),
            80,
            reaches_ocean,
        ) {
            Ok(_) => return Ok(()),
            Err(VirtualPlayerError::Timeout { .. }) => {
                player.tap(COM_DIG)?;
                player.wait_out_double_click()?;
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
    player.assert_milestone(
        "the Clonk clears the western slope into the ocean",
        reaches_ocean,
    )?;
    Ok(())
}

fn catch_and_deposit_another_fish(
    player: &mut VirtualPlayer<'_>,
    owner: i32,
    clonk: ObjectId,
    igloo: ObjectId,
) -> Result<ObjectId, Box<dyn Error>> {
    player.menu_close()?;
    player.wait_until("IGLO restores context after closing Sell", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the Clonk exits IGLO for another FISH", 50, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
    })?;
    // Leave through IGLO's still-open western entrance before its door closes
    // (FarWorlds Igloo DefCore Entrance=-28,-4,23,18).
    leave_igloo_for_western_ocean(player, clonk)?;
    swim_until_fish_count(player, clonk, 1, 400)?;
    swim_to_x(player, clonk, 180)?;
    resurface_near_western_shore(player, clonk)?;
    climb_onto_western_island(player, clonk)?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk returns to the IGLO entrance",
        100,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(igloo))
                .is_some_and(|(clonk, igloo)| {
                    clonk.action.name == "Walk" && clonk.position.x >= igloo.position.x - 28
                })
        },
    )?;
    player.tap(COM_UP)?;
    let enters_igloo = |engine: &Engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(igloo))
    };
    match player.wait_until("the Clonk re-enters IGLO with FISH", 40, enters_igloo) {
        Ok(_) => {}
        Err(VirtualPlayerError::Timeout { .. }) => {
            // Wind/snow can carry a late return past the narrow western
            // entrance. The ordinary correction is to walk back from the
            // east until the Clonk is inside Entrance=-28,-4,23,18 and retry
            // Up; no position or containment state is edited.
            player.hold_until(
                COM_LEFT,
                "the Clonk corrects back to IGLO's western entrance",
                180,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .zip(engine.object_snapshot(igloo))
                        .is_some_and(|(clonk, igloo)| {
                            clonk.action.name == "Walk" && clonk.position.x <= igloo.position.x - 8
                        })
                },
            )?;
            player.tap(COM_UP)?;
            player.wait_until(
                "the Clonk re-enters IGLO after correcting",
                60,
                enters_igloo,
            )?;
        }
        Err(error) => return Err(Box::new(error)),
    }

    let fish = carried_object(player.engine(), clonk, "FISH")
        .expect("the newly caught FISH is in the real inventory");
    player.wait_until("IGLO opens context for another FISH", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Put")?;
    player.menu_enter()?;
    player.wait_until("context Put transfers another FISH", 30, |engine| {
        engine
            .object_snapshot(fish)
            .is_some_and(|object| object.container == Some(igloo))
    })?;
    player.wait_until("IGLO restores context after another Put", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    Ok(fish)
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn player_wealth(engine: &Engine, owner: i32) -> i32 {
    engine.player_wealth(owner).unwrap_or(0)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

#[test]
fn tutorial09_real_ocean_consumes_and_restores_extended_breath() -> Result<(), Box<dyn Error>> {
    let (mut engine, owner) = load_tutorial09();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial09 joins one selected CLNK");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    // Tutorial09/Script.c:25-26 gives the real crew AquaClonk-class Swim and
    // Breath physicals. On breathable Tick5, C++ fills C4Object::Breath to the
    // newly installed maximum in one gulp (src/C4Object.cpp:880-919).
    player.wait_until(
        "Tutorial09 fills the Clonk's extended breath capacity",
        10,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.breath == 250_000)
        },
    )?;
    let full_energy = player
        .engine()
        .object_snapshot(clonk)
        .expect("Tutorial09 Clonk exists")
        .energy;

    player.hold_until(
        COM_LEFT,
        "the Clonk enters Tutorial09's real western ocean",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Swim")
        },
    )?;
    player.press(COM_DOWN)?;
    player.wait_until(
        "the submerged Clonk consumes one breath interval",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.breath < 250_000)
        },
    )?;
    player.release(COM_DOWN)?;

    let submerged = player
        .engine()
        .object_snapshot(clonk)
        .expect("submerged Tutorial09 Clonk survives");
    assert_eq!(
        submerged.breath, 248_000,
        "C++ consumes 2*C4MaxPhysical/100 on the first submerged Tick5 (C4Object.cpp:901-910)"
    );
    assert_eq!(
        submerged.energy, full_energy,
        "C++ consumes breath before asphyxiation can damage energy (C4Object.cpp:903-906)"
    );

    player.hold_until(
        COM_UP,
        "the Clonk surfaces and takes a full breath",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.breath == 250_000)
        },
    )?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(clonk)
            .expect("resurfaced Tutorial09 Clonk survives")
            .energy,
        full_energy
    );

    Ok(())
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::C4Id("CXCN".into()))
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Sell")?;
    player.menu_enter()?;
    player.wait_until("IGLO opens the real Sell menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(5))
    })?;
    let initial_fish_count = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| {
            menu.items
                .iter()
                .find(|item| item.item_id == "FISH")
                .map(|item| item.count)
        })
        .expect("IGLO Sell menu offers the deposited FISH");
    assert_eq!(initial_fish_count, 1);

    // Keep the first FISH in IGLO and physically catch/deposit another. FISH
    // permits APS_Color (Animals/Fish/DefCore.txt), so C++ stContents inserts
    // the new same-ID object at the front of the chunk and C4ObjectListIterator
    // emits one two-object picture group even if the fish colors differ
    // (C4ObjectList.cpp:144-173,849-903; C4Object.cpp:6173-6213).
    let second_fish = catch_and_deposit_another_fish(&mut player, owner, clonk, igloo)?;
    player.menu_navigate_to_caption("Sell")?;
    player.menu_enter()?;
    player.wait_until("IGLO opens its stacked FISH Sell row", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(5))
    })?;
    let (fish_index, stacked_count, representative) = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| {
            let fish_rows = menu
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.item_id == "FISH")
                .collect::<Vec<_>>();
            (fish_rows.len() == 1).then(|| {
                let (index, item) = fish_rows[0];
                (index, item.count, item.picture_object)
            })
        })
        .expect("two real FISH concatenate into one C++ Sell row");
    assert_eq!(stacked_count, 2);
    assert_eq!(
        representative,
        Some(second_fish),
        "stContents inserts a new same-ID object before the existing chunk"
    );
    player.menu_navigate_to_index(fish_index)?;
    player.menu_enter()?;
    player.wait_until("IGLO sells only the stack representative", 40, |engine| {
        player_wealth(engine, owner) == 20
            && engine.object_snapshot(second_fish).is_none()
            && engine.object_snapshot(fish).is_some()
    })?;
    player.wait_until("the FISH Sell row refills with count one", 20, |engine| {
        engine.cursor_object_menu(owner).is_some_and(|(_, menu)| {
            let fish_rows = menu
                .items
                .iter()
                .filter(|item| item.item_id == "FISH")
                .collect::<Vec<_>>();
            menu.identification == clonk_script::Value::Int(5)
                && menu.selection == fish_index as i32
                && fish_rows.len() == 1
                && fish_rows[0].count == 1
                && fish_rows[0].picture_object == Some(fish)
        })
    })?;
    player.menu_enter()?;
    player.wait_until("IGLO sells the remaining FISH", 40, |engine| {
        player_wealth(engine, owner) == 40 && engine.object_snapshot(fish).is_none()
    })?;

    for expected_wealth in [60, 80, 100] {
        let fish = catch_and_deposit_another_fish(&mut player, owner, clonk, igloo)?;
        player.menu_navigate_to_caption("Sell")?;
        player.menu_enter()?;
        player.wait_until("IGLO restores the real Sell menu", 20, |engine| {
            object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(5))
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
        Engine::is_game_over,
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
