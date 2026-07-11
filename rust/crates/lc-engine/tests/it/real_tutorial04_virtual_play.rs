#![allow(dead_code)]

use std::error::Error;

use lc_engine::{
    CommandDirection, Direction, Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RIGHT, COM_SPECIAL2, COM_THROW, COM_UP,
};
use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;

fn load_tutorial04() -> (Engine, i32) {
    let mut engine = load_tutorial(4, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 4 virtual player".to_owned(),
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
        .expect("local Tutorial04 virtual player joins")
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

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<lc_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
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

fn clonk_contents_count(engine: &Engine, clonk: ObjectId, definition: &str) -> usize {
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

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine
        .snapshot()
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
}

fn carry_gold_from_tunnel_to_hut(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
    target_wealth: i32,
) -> Result<(), Box<dyn Error>> {
    player.hold_until(
        COM_RIGHT,
        format!("the {target_wealth}-wealth GOLD trip returns to ELEC"),
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until(
        format!("the {target_wealth}-wealth GOLD trip grabs ELEC"),
        60,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        },
    )?;
    player.hold_until(
        COM_UP,
        format!("ELEC raises the {target_wealth}-wealth GOLD trip"),
        300,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y <= 270)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        format!("the {target_wealth}-wealth GOLD trip lets go of ELEC"),
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name != "Push")
        },
    )?;

    // The surface shaft lip alternates Jump and Scale in C++, so each
    // crossing is driven by the same real Right/Up edges as the first trip
    // (C4Object.cpp:3618-3628,4284-4299,4823-4855).
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..240 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the GOLD-carrying Clonk survives the shaft lip");
        if clonk_now.position.x >= 558 {
            break;
        }
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_RIGHT)?;
            player.press(COM_RIGHT)?;
        } else if landed || left_scale_in_flight {
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_RIGHT)?;
    player.assert_milestone(
        format!("the {target_wealth}-wealth GOLD trip reaches HUT2's hill"),
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 558)
        },
    )?;
    player.wait_until(
        format!("the {target_wealth}-wealth GOLD trip lands beside HUT2"),
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_LEFT,
        format!("the {target_wealth}-wealth GOLD trip aligns with HUT2"),
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 570)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until(
        format!("the {target_wealth}-wealth GOLD trip enters HUT2"),
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container == Some(hut))
        },
    )?;
    player.wait_until(
        format!("HUT2 auto-sells GOLD to reach {target_wealth} wealth"),
        80,
        |engine| {
            engine
                .snapshot()
                .hud
                .players
                .iter()
                .any(|player| player.owner == owner && player.wealth >= target_wealth)
        },
    )?;
    Ok(())
}

fn return_from_hut_to_tunnel(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
) -> Result<(), Box<dyn Error>> {
    player.wait_until(
        "HUT2 restores its context menu after selling GOLD",
        30,
        |engine| object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14)),
    )?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until(
        "the empty Clonk exits HUT2 for another GOLD trip",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        },
    )?;

    player.hold_until(
        COM_LEFT,
        "the empty Clonk returns to the surface shaft",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..120 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the empty Clonk survives the shaft lip");
        if clonk_now.action.name == "Walk" && clonk_now.position.x >= 505 {
            break;
        }
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_RIGHT)?;
            player.press(COM_RIGHT)?;
        } else if landed || left_scale_in_flight {
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_RIGHT)?;
    player.hold_until(
        COM_LEFT,
        "the empty Clonk stands beside ELEC",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the empty Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_DIG,
        "ELEC carries the empty Clonk underground",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 340)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the empty Clonk lets go underground", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    Ok(())
}

fn collect_one_gold_in_tunnel(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
) -> Result<(), Box<dyn Error>> {
    for attempt in 1..=4 {
        if clonk_contents_count(player.engine(), clonk, "GOLD") == 1 {
            break;
        }
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| !object.contents.is_empty())
        {
            // Blast debris is collectible too, and this Clonk has one
            // inventory slot. Drop incidental ROCK before approaching GOLD.
            let start_x = player
                .engine()
                .object_snapshot(clonk)
                .expect("the Clonk survives its incidental ROCK pickup")
                .position
                .x;
            player.tap(COM_THROW)?;
            player.wait_until(
                "the empty Clonk drops incidental blast debris",
                30,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.contents.is_empty())
                },
            )?;
            player.hold_until(
                COM_RIGHT,
                "the Clonk moves away from thrown blast debris",
                60,
                |engine| {
                    clonk_contents_count(engine, clonk, "GOLD") == 1
                        || engine
                            .object_snapshot(clonk)
                            .is_some_and(|object| object.position.x >= start_x + 12)
                },
            )?;
            if clonk_contents_count(player.engine(), clonk, "GOLD") == 1 {
                break;
            }
        }
        player
            .hold_until(
                COM_LEFT,
                format!("the empty Clonk advances toward GOLD on attempt {attempt}"),
                220,
                |engine| {
                    clonk_contents_count(engine, clonk, "GOLD") == 1
                        || engine
                            .object_snapshot(clonk)
                            .is_some_and(|object| {
                                object.action.name == "Hangle" || !object.contents.is_empty()
                            })
                },
            )
            .map_err(|error| {
                let snapshot = player.engine().snapshot();
                let contents = player
                    .engine()
                    .object_snapshot(clonk)
                    .map(|clonk| {
                        clonk
                            .contents
                            .iter()
                            .filter_map(|item| player.engine().object_snapshot(*item))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let gold = snapshot
                    .objects
                    .into_iter()
                    .filter(|object| object.definition_id == "GOLD")
                    .collect::<Vec<_>>();
                format!("{error}; contents={contents:?}; gold={gold:?}")
            })?;
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            player.tap(COM_DOWN)?;
            player.wait_until(
                "the next GOLD trip drops to the tunnel floor",
                60,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.action.name == "Walk")
                },
            )?;
        }
    }
    player.assert_milestone("the empty Clonk collects one more GOLD chunk", |engine| {
        clonk_contents_count(engine, clonk, "GOLD") == 1
    })?;
    Ok(())
}

fn return_from_hut_and_collect_one_gold(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
) -> Result<(), Box<dyn Error>> {
    return_from_hut_to_tunnel(player, clonk, elevator_case, hut, owner)?;
    collect_one_gold_in_tunnel(player, clonk)
}

fn blast_one_tfln(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    next_face_x: i32,
    remaining: usize,
) -> Result<(), Box<dyn Error>> {
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        player.tap(COM_DOWN)?;
    }
    player.wait_until(
        "the Clonk stands before each replacement blast",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.press(COM_LEFT)?;
    for _ in 0..120 {
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x <= next_face_x)
        {
            break;
        }
        player.ticks(1)?;
    }
    player.release(COM_LEFT)?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.position.x > next_face_x)
    {
        // Successive floor blasts can leave a low Earth lip in front of
        // the newly exposed (non-diggable) gold. Clear that lip with the
        // same real Dig+Left controls before placing the next flint.
        player.tap(COM_DIG)?;
        player.wait_until(
            "the Clonk digs through the blast-pocket lip",
            30,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        )?;
        player.hold_until(
            COM_LEFT,
            "the Dig action reaches the next blast face",
            120,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x <= next_face_x + 5)
            },
        )?;
        player.wait_until(
            "the Clonk stops digging at the next gold face",
            40,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        )?;
    }
    // Down+Throw is C++'s ordinary drop chord. It places each flint on
    // the lower tunnel floor instead of arcing into the low ceiling, so
    // successive blasts open the gold face and let the Clonk advance
    // (C4ObjectCom.cpp:1013-1036,658-671).
    player.press(COM_DOWN)?;
    player.tap(COM_THROW)?;
    player.release(COM_DOWN)?;
    player.wait_until(
        "a replacement TFLN leaves the Clonk's inventory",
        30,
        |engine| clonk_contents_count(engine, clonk, "TFLN") == remaining,
    )?;
    let thrown = player
        .engine()
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "TFLN" && object.container.is_none())
        .map(|object| object.id)
        .expect("the thrown replacement TFLN exists in the tunnel");
    let retreat_x = player
        .engine()
        .object_snapshot(clonk)
        .expect("Clonk survives the replacement throw")
        .position
        .x
        + 28;
    player.hold_until(
        COM_RIGHT,
        "the Clonk retreats before each replacement TFLN detonates",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= retreat_x)
        },
    )?;
    player.wait_until("the replacement TFLN detonates", 180, |engine| {
        engine.object_snapshot(thrown).is_none()
    })?;
    player.wait_until(
        "the Clonk recovers after each replacement blast",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    Ok(())
}

#[test]
fn tutorial04_virtual_player_completes_the_real_scenario() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial04();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial04 joins one selected CLNK");
    let hut = object_with_definition(&engine, "HUT2").expect("Tutorial04 creates HUT2");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("the ready base and Clonk finish joining", 180, |engine| {
        engine
            .object_snapshot(hut)
            .is_some_and(|object| object.base == owner)
            && engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
    })?;
    player.wait_until("Tutorial04 asks the Clonk to enter HUT2", 240, |engine| {
        tutorial_message_contains(engine, "Enter your home base")
    })?;

    // Seed zero places HUT2 at (586,245). Its relative Entrance
    // -18,8,16,17 is world [568,584) x [253,270); Up takes the entrance
    // before Jump (Hut2.c4d/DefCore.txt:7-18; C4ObjectCom.cpp:335-350).
    player.hold_until(
        COM_LEFT,
        "the Clonk aligns with HUT2's entrance",
        30,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && (574..578).contains(&object.position.x)
                    && (253..270).contains(&object.position.y)
            })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the Clonk enters HUT2", 50, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("Tutorial04 asks for HUT2 Contents", 240, |engine| {
        tutorial_message_contains(engine, "select 'Contents'")
    })?;

    // Entering HUT2 with AutoContextMenu enabled opens C4MN_Context.
    // Tutorial04 then requires C4MN_Contents (14 -> 18) and CNKT
    // (Script.c:40-78; C4ObjectMenu.cpp:279-325,328-374).
    player.wait_until("HUT2 opens its auto-context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT2 opens its Contents menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(18))
    })?;
    player.wait_until("Tutorial04 asks the Clonk to take CNKT", 240, |engine| {
        tutorial_message_contains(engine, "Take the construction kit")
    })?;
    let conkit_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "CNKT"))
        .expect("Tutorial04 HUT2 contains CNKT");
    player.menu_navigate_to_index(conkit_index)?;
    player.menu_enter()?;
    player.wait_until("the Clonk takes CNKT from HUT2", 60, |engine| {
        clonk_carries(engine, clonk, "CNKT")
    })?;
    player.wait_until("Tutorial04 asks the Clonk to leave HUT2", 240, |engine| {
        tutorial_message_contains(engine, "close the menu and exit")
    })?;

    player.menu_close()?;
    player.wait_until(
        "HUT2 restores its context menu after taking CNKT",
        30,
        |engine| object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14)),
    )?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the CNKT-carrying Clonk exits HUT2", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
    })?;
    player.wait_until("Tutorial04 points at the elevator site", 240, |engine| {
        tutorial_message_contains(engine, "clear area to the left")
    })?;

    player.hold_until(
        COM_LEFT,
        "the Clonk reaches Tutorial04's elevator site",
        120,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (490..=510).contains(&object.position.x)
            })
        },
    )?;
    player.wait_until("Tutorial04 asks for the construction menu", 240, |engine| {
        tutorial_message_contains(engine, "twice quickly to open the construction menu")
    })?;
    player.double_tap(COM_DIG)?;
    player.wait_until("CNKT opens the real CXCN menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::C4Id("CXCN".into()))
    })?;
    player.wait_until(
        "Tutorial04 asks for an ELEV construction site",
        240,
        |engine| tutorial_message_contains(engine, "Create an elevator construction site"),
    )?;
    player.menu_enter()?;
    let elevator = player
        .wait_until("the ELEV construction site is created", 30, |engine| {
            object_with_definition(engine, "ELEV").is_some()
        })
        .map(|_| object_with_definition(player.engine(), "ELEV").expect("ELEV exists"))?;
    player.wait_until("Tutorial04 asks the Clonk to build ELEV", 240, |engine| {
        tutorial_message_contains(engine, "press 'down' to start working")
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk starts building ELEV", 30, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Build")
    })?;
    player.wait_until(
        "Tutorial04 clears the permanent construction instruction",
        // Script131's wait(10) resumes ScriptGo after 10*10 frames
        // (Tutorial.c:33-37; C4ScriptHost.cpp:222-231).
        120,
        |engine| {
            engine
                .object_snapshot(elevator)
                .is_some_and(|object| object.construction < 100_000)
                && !tutorial_message_contains(engine, "press 'down' to start working")
        },
    )?;
    player.wait_until("ELEV finishes and creates ELEC", 720, |engine| {
        object_with_definition(engine, "ELEC").is_some()
            && engine
                .object_snapshot(elevator)
                .is_some_and(|object| object.construction == 100_000)
    })?;
    let elevator_case = object_with_definition(player.engine(), "ELEC").expect("ELEV creates ELEC");
    player.wait_until("Tutorial04 asks the Clonk to grab ELEC", 240, |engine| {
        tutorial_message_contains(engine, "Grab the elevator case")
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.wait_until(
        "Tutorial04 asks the Clonk to drill the shaft",
        240,
        |engine| tutorial_message_contains(engine, "Hold down the 'dig' key"),
    )?;
    player.hold_until(
        COM_DIG,
        "ELEC drills the Clonk to the bottom of the shaft",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 340)
        },
    )?;
    player.wait_until("Tutorial04 asks the Clonk to ride ELEC up", 240, |engine| {
        tutorial_message_contains(engine, "ride the elevator back up")
    })?;
    player.hold_until(
        COM_UP,
        "ELEC carries the Clonk back to the surface",
        240,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y <= 270)
        },
    )?;
    player.wait_until(
        "Tutorial04 asks the Clonk to let go of ELEC",
        240,
        |engine| tutorial_message_contains(engine, "Let go of the elevator case"),
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk lets go of ELEC", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name != "Push")
    })?;
    player.wait_until(
        "Tutorial04 sends the Clonk to collect TFLN",
        240,
        |engine| {
            tutorial_message_contains(engine, "Walk back to the cabin")
                && object_with_definition(engine, "TFLN").is_some()
        },
    )?;
    // The shaft lip requires the same held direction + real jump/scale
    // transitions a player uses, and TFLN's 60-frame fuse starts when its
    // Exit command first hits the ground.
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..60 {
        if clonk_carries(player.engine(), clonk, "TFLN") {
            break;
        }
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("Clonk survives the shaft exit");
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_RIGHT)?;
            player.press(COM_RIGHT)?;
        } else if (landed || left_scale_in_flight) && clonk_now.position.x < 550 {
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_RIGHT)?;
    player.assert_milestone(
        "the Clonk naturally collects TFLN before its fuse expires",
        |engine| clonk_carries(engine, clonk, "TFLN"),
    )?;
    player.hold_until(
        COM_LEFT,
        "the Clonk immediately turns back toward ELEC with TFLN",
        120,
        |engine| {
            tutorial_message_contains(engine, "Ride back down into the mine")
                || engine
                    .object_snapshot(clonk)
                    .zip(engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator_case)| {
                        clonk.position.x <= elevator_case.position.x + 5
                    })
        },
    )?;
    player.wait_until(
        "Tutorial04 sends the TFLN-carrying Clonk down",
        240,
        |engine| {
            tutorial_message_contains(engine, "Ride back down into the mine")
                && clonk_carries(engine, clonk, "TFLN")
        },
    )?;
    let (clonk_x, elevator_x) = player
        .engine()
        .object_snapshot(clonk)
        .zip(player.engine().object_snapshot(elevator_case))
        .map(|(clonk, elevator)| (clonk.position.x, elevator.position.x))
        .expect("the Clonk and ELEC survive the surface return");
    if clonk_x < elevator_x - 5 {
        player.hold_until(
            COM_RIGHT,
            "the Clonk aligns with ELEC from the left",
            120,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .zip(engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        )?;
    } else if clonk_x > elevator_x + 5 {
        player.hold_until(
            COM_LEFT,
            "the Clonk aligns with ELEC from the right",
            120,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .zip(engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator)| {
                        (clonk.position.x - elevator.position.x).abs() <= 5
                    })
            },
        )?;
    }
    if let Some(clonk_now) = player.engine().object_snapshot(clonk) {
        if clonk_now.action.name.starts_with("Scale") {
            let away = if clonk_now.direction == Direction::Left {
                COM_RIGHT
            } else {
                COM_LEFT
            };
            player.tap(away)?;
        }
    }
    player.wait_until("the TFLN-carrying Clonk settles beside ELEC", 120, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(clonk, elevator)| {
                clonk.action.name == "Walk"
                    && (clonk.position.x - elevator.position.x).abs() <= 5
            })
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the TFLN-carrying Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_DIG,
        "ELEC carries the TFLN-carrying Clonk back down",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 340)
        },
    )?;
    player.wait_until("Tutorial04 asks for the gold tunnel", 240, |engine| {
        tutorial_message_contains(engine, "Dig a tunnel all the way")
    })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk lets go of ELEC underground", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;

    // Script175 waits for living crew in the 80x80 rectangle centred on
    // (397,388). A real Dig tap followed by held Left steers DFA_DIG from the
    // shaft to that rectangle (Tutorial04.c4s/Script.c:153-160;
    // C4ObjectCom.cpp:353-362; C4Object.cpp:3573-3631).
    player.tap(COM_DIG)?;
    player.wait_until(
        "the Clonk starts digging toward the gold vein",
        30,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        },
    )?;
    player.press(COM_LEFT)?;
    player.press(COM_DOWN)?;
    // Bottom contact redirects C++'s initial DownLeft to Left. Once the
    // first DigFree pass has opened the face, releasing and pressing Down
    // steers into the descending diagonal (C4Object.cpp:4354-4368).
    player.ticks(12)?;
    player.release(COM_DOWN)?;
    player.press(COM_DOWN)?;
    for _ in 0..240 {
        let reached = tutorial_message_contains(player.engine(), "struck solid gold")
            || player.engine().object_snapshot(clonk).is_some_and(|object| {
                (357..437).contains(&object.position.x)
                    && (348..428).contains(&object.position.y)
            });
        if reached {
            break;
        }
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.command_direction == CommandDirection::Left)
        {
            // Bottom contact redirects DownLeft to Left. A player's fresh
            // Down edge rotates the active dig back toward the vein.
            player.release(COM_DOWN)?;
            player.press(COM_DOWN)?;
        }
        player.ticks(1)?;
    }
    player.release(COM_DOWN)?;
    player.release(COM_LEFT)?;
    player.assert_milestone("the real dig tunnel reaches Tutorial04's gold vein", |engine| {
        tutorial_message_contains(engine, "struck solid gold")
            || engine.object_snapshot(clonk).is_some_and(|object| {
                (357..437).contains(&object.position.x)
                    && (348..428).contains(&object.position.y)
            })
    })?;
    player.wait_until(
        "Tutorial04 asks the Clonk to blast the gold vein",
        120,
        |engine| tutorial_message_contains(engine, "struck solid gold"),
    )?;
    player.wait_until("the Clonk stops digging at the gold face", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;

    let safe_x = player
        .engine()
        .object_snapshot(clonk)
        .expect("Clonk survives the gold tunnel")
        .position
        .x
        + 24;
    player.hold_until(
        COM_RIGHT,
        "the TFLN-carrying Clonk retreats to a safe throwing distance",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= safe_x)
        },
    )?;
    player.tap(COM_LEFT)?;
    player.ticks(1)?;
    player.tap(COM_THROW)?;
    player.wait_until("the Clonk throws TFLN toward the gold vein", 30, |engine| {
        !clonk_carries(engine, clonk, "TFLN")
    })?;
    player.wait_until("the real TFLN blast frees a GOLD chunk", 180, |engine| {
        object_with_definition(engine, "GOLD").is_some()
    })?;
    player.ticks(100)?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk drops from the tunnel ceiling", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the Clonk naturally collects a GOLD chunk from the first blast",
        120,
        |engine| clonk_contents_count(engine, clonk, "GOLD") >= 1,
    )?;
    player.hold_until(
        COM_RIGHT,
        "the GOLD-carrying Clonk returns to ELEC",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the GOLD-carrying Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_UP,
        "ELEC carries the GOLD-carrying Clonk to the surface",
        300,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y <= 270)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the GOLD-carrying Clonk lets go of ELEC", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name != "Push")
    })?;

    // The shaft lip still requires real held-Right plus jump/scale edges
    // (C4Object.cpp:3618-3628,4284-4299,4823-4855).
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..240 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("GOLD-carrying Clonk survives the shaft climb");
        if clonk_now.position.x >= 558 {
            break;
        }
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_RIGHT)?;
            player.press(COM_RIGHT)?;
        } else if landed || left_scale_in_flight {
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
    player.wait_until("the GOLD-carrying Clonk lands beside HUT2", 80, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the GOLD-carrying Clonk aligns with HUT2's entrance",
        80,
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
    player.wait_until("HUT2 sells the first GOLD chunk", 80, |engine| {
        engine
            .snapshot()
            .hud
            .players
            .iter()
            .any(|player| player.owner == owner && player.wealth >= 5)
    })?;

    // Script200 creates three replacement flints in HUT2 after the first
    // successful blast, then Script201 waits for the Clonk to be inside
    // (Tutorial04.c4s/Script.c:181-203).
    player.wait_until(
        "Tutorial04 puts three replacement TFLNs in HUT2",
        400,
        |engine| {
            tutorial_message_contains(engine, "more T-Flints")
                && engine.object_snapshot(hut).is_some_and(|hut| {
                    hut.contents
                        .iter()
                        .filter(|item| {
                            engine
                                .object_snapshot(**item)
                                .is_some_and(|item| item.definition_id == "TFLN")
                        })
                        .count()
                        >= 3
                })
        },
    )?;
    player.wait_until("HUT2 restores its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT2 opens replacement-flint Contents", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(18))
    })?;
    let flint_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "TFLN"))
        .expect("replacement TFLNs appear in HUT2 Contents");
    player.menu_navigate_to_index(flint_index)?;
    // Special2 is C++ COM_MenuEnterAll, selecting Command2 for the contents
    // entry (C4Menu.cpp:433-440,498-523,1047-1054).
    player.tap(COM_SPECIAL2)?;
    player.wait_until(
        "the Clonk takes all three replacement TFLNs",
        120,
        |engine| clonk_contents_count(engine, clonk, "TFLN") >= 3,
    )?;
    player.menu_close()?;
    player.wait_until("HUT2 restores context after taking TFLNs", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the TFLN-carrying Clonk exits HUT2", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
    })?;

    player.hold_until(
        COM_LEFT,
        "the replacement-TFLN Clonk returns to ELEC",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..120 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("replacement-TFLN Clonk survives the shaft lip");
        if clonk_now.action.name == "Walk" && clonk_now.position.x >= 505 {
            break;
        }
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_RIGHT)?;
            player.press(COM_RIGHT)?;
        } else if landed || left_scale_in_flight {
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_RIGHT)?;
    player.hold_until(
        COM_LEFT,
        "the replacement-TFLN Clonk stands beside ELEC",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the replacement-TFLN Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_DIG,
        "ELEC carries the replacement-TFLN Clonk underground",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 340)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the replacement-TFLN Clonk lets go underground",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the replacement-TFLN Clonk returns to the blast tunnel",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 452 && object.position.y >= 365)
        },
    )?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        player.tap(COM_DOWN)?;
    }
    player.wait_until(
        "the Clonk stands at a safe second-blast distance",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;

    let mut next_face_x = 452;
    for remaining in (0..3).rev() {
        blast_one_tfln(&mut player, clonk, next_face_x, remaining)?;
        next_face_x -= 18;
    }
    player.wait_until(
        "the replacement TFLNs blast more GOLD free",
        120,
        |engine| {
            engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == "GOLD")
                .count()
                >= 4
        },
    )?;
    player.ticks(120)?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        player.tap(COM_DOWN)?;
        player.wait_until(
            "the Clonk drops into the second blast pocket",
            60,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        )?;
    }

    player.hold_until(
        COM_LEFT,
        "the Clonk collects one GOLD chunk from the settled blast cluster",
        120,
        |engine| clonk_contents_count(engine, clonk, "GOLD") == 1,
    )?;
    player.assert_milestone(
        "the Clonk carries exactly one nonspecial GOLD chunk",
        |engine| clonk_contents_count(engine, clonk, "GOLD") == 1,
    )?;

    // CLNK permits only one nonspecial inventory object: RejectCollect ends
    // with GetNonSpecialCount() >= MaxContentsCount(), and
    // MaxContentsCount() is 1. C++ Tutorial04 therefore requires four more
    // one-GOLD elevator/base trips after the first sale, rather than a bulk
    // pickup (Clonk.c4d/Script.c:738-763; Tutorial04 Script.c:227-234).
    for sold_chunks in 2..=5 {
        let target_wealth = sold_chunks * 5;
        carry_gold_from_tunnel_to_hut(
            &mut player,
            clonk,
            elevator_case,
            hut,
            owner,
            target_wealth,
        )?;
        if sold_chunks < 5 {
            return_from_hut_and_collect_one_gold(&mut player, clonk, elevator_case, hut, owner)?;
        }
    }
    player.wait_until("Tutorial04 selects Tutorial05", 320, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial05.c4s"
    })?;
    player.wait_until(
        "Tutorial04 fulfilled goal reaches GameOver",
        320,
        |engine| engine.snapshot().game_over,
    )?;
    assert!(
        player
            .engine()
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG"),
        "Tutorial04 must fulfill its real SCRG before selecting Tutorial05"
    );

    Ok(())
}
