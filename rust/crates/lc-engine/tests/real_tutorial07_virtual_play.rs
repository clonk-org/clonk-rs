#[allow(dead_code)]
mod support;

use std::error::Error;

use lc_engine::{
    CommandDirection, Direction, Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RIGHT, COM_SPECIAL2, COM_THROW, COM_UP,
};
use support::real_scenario::load_tutorial;
use support::virtual_player::VirtualPlayer;

fn load_tutorial07() -> (Engine, i32) {
    let mut engine = load_tutorial(7, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 7 virtual player".to_owned(),
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
        .expect("local Tutorial07 virtual player joins")
        .number;
    (engine, owner)
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine
        .snapshot()
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
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

fn clonk_carries(engine: &Engine, clonk: ObjectId, definition: &str) -> bool {
    clonk_contents_count(engine, clonk, definition) != 0
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

fn climb_right_out_of_blast_pocket(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    target_x: i32,
    milestone: &str,
) -> Result<(), Box<dyn Error>> {
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    for _ in 0..300 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the Clonk survives the blast-pocket climb");
        if clonk_now.position.x >= target_x {
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
    player.assert_milestone(milestone, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= target_x)
    })?;
    Ok(())
}

fn carry_gold_to_hut(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
    target_wealth: i32,
) -> Result<(), Box<dyn Error>> {
    climb_right_out_of_blast_pocket(
        player,
        clonk,
        105,
        "the GOLD-carrying Clonk climbs out of the blast pocket",
    )?;
    player.wait_out_double_click()?;
    player.hold_until(
        COM_DOWN,
        "the GOLD-carrying Clonk descends beside ELEC",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.y - elevator_case.position.y).abs() <= 20
                })
        },
    )?;
    // The shaft lip leaves the Clonk within ELEC's grab rectangle. A right
    // tap separates this Down from the descent Down in C++'s LastCom buffer.
    player.tap(COM_RIGHT)?;
    player.tap(COM_DOWN)?;
    player.wait_until("the GOLD-carrying Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_UP,
        format!("ELEC raises GOLD toward wealth {target_wealth}"),
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y <= 205)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk releases ELEC at the surface", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.press(COM_LEFT)?;
    let mut previous_action = String::new();
    for _ in 0..300 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the GOLD-carrying Clonk survives the surface lip");
        if clonk_now.position.x <= 70 {
            break;
        }
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_LEFT)?;
            player.press(COM_LEFT)?;
        } else if landed || left_scale_in_flight {
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_LEFT)?;
    player.assert_milestone(
        "the GOLD-carrying Clonk crosses the surface lip",
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 70)
        },
    )?;
    player.wait_until("the GOLD-carrying Clonk lands beside HUT3", 120, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_RIGHT,
        "the GOLD-carrying Clonk steps into HUT3's entrance",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 62)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the GOLD-carrying Clonk enters HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until(
        format!("HUT3 auto-sells GOLD to reach wealth {target_wealth}"),
        80,
        |engine| player_wealth(engine, owner) >= target_wealth,
    )?;
    Ok(())
}

fn return_from_hut_and_collect_gold(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
) -> Result<(), Box<dyn Error>> {
    player.wait_until("HUT3 restores context after selling GOLD", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the empty Clonk exits HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container != Some(hut))
    })?;

    player.press(COM_RIGHT)?;
    let mut previous_action = player
        .engine()
        .object_snapshot(clonk)
        .expect("the empty Clonk survives HUT3 exit")
        .action
        .name;
    for _ in 0..300 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the empty Clonk survives the surface shaft lip");
        if clonk_now.position.x >= 105 {
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
    player.assert_milestone("the empty Clonk crosses the surface shaft lip", |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 105)
    })?;
    player.wait_out_double_click()?;
    player.hold_until(
        COM_DOWN,
        "the empty Clonk descends beside ELEC",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.y - elevator_case.position.y).abs() <= 20
                })
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the empty Clonk stands beside ELEC",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    (clonk.position.x - elevator_case.position.x).abs() <= 5
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
        "ELEC lowers the empty Clonk to the GOLD seam",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 325)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the empty Clonk releases ELEC underground", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_RIGHT,
        "the empty Clonk takes a run-up beside the GOLD seam",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 120)
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the Clonk naturally collects another GOLD chunk",
        180,
        |engine| clonk_carries(engine, clonk, "GOLD"),
    )?;
    Ok(())
}

#[test]
fn tutorial07_virtual_player_completes_the_real_scenario() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial07();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial07 joins one selected CLNK");
    let hut = object_with_definition(&engine, "HUT3").expect("Tutorial07 creates HUT3");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    // Tutorial07 Script2..12 introduces the real route before handing control
    // to the player (Tutorial07.c4s/Script.c:36-90). The virtual player waits
    // through those same engine frames instead of skipping script state.
    player.wait_until(
        "Tutorial07 presents its final route prompt",
        2_000,
        |engine| tutorial_message_contains(engine, "Good luck"),
    )?;
    player.assert_milestone(
        "the Tutorial07 Clonk is available for real input",
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        },
    )?;

    // Seed zero starts the ready crew in HUT3's world entrance rectangle.
    // Up therefore takes ObjectComEnter before Jump; the ordinary context and
    // Contents menus expose the two real FLNT objects (Hut3 DefCore Entrance;
    // C4ObjectCom.cpp:335-350; C4ObjectMenu.cpp:279-374).
    player.tap(COM_UP)?;
    player.wait_until("the Tutorial07 Clonk enters HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("HUT3 opens its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its real Contents menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(18))
    })?;
    let flint_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "FLNT"))
        .expect("Tutorial07 HUT3 contains FLNT");
    player.menu_navigate_to_index(flint_index)?;
    player.tap(COM_SPECIAL2)?;
    player.wait_until("the Clonk takes both Tutorial07 flints", 120, |engine| {
        clonk_contents_count(engine, clonk, "FLNT") == 2
    })?;

    player.menu_close()?;
    player.wait_until("HUT3 restores its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the FLNT-carrying Clonk exits HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
    })?;

    let elevator_case = object_with_definition(player.engine(), "ELEC")
        .expect("Tutorial07 places a ready elevator case");
    player.hold_until(
        COM_RIGHT,
        "the FLNT-carrying Clonk reaches ELEC",
        100,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    (clonk.position.x - elevator_case.position.x).abs() <= 5
                })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs Tutorial07 ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_DIG,
        "ELEC lowers the Clonk to Tutorial07's GOLD layer",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 300)
        },
    )?;

    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk releases ELEC at the GOLD layer", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the Clonk approaches Tutorial07's marked GOLD seam",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 92)
        },
    )?;
    // Script8 points to the GOLD seam at (72,315), and the tutorial gives
    // exactly two real FLNT objects for opening it (Script.c:61-78). Throwing
    // uses the ordinary inventory command; the explosion must excavate and
    // materialize GOLD through the engine's normal landscape path.
    player.tap(COM_THROW)?;
    player.wait_until(
        "the first FLNT leaves the Clonk's inventory",
        60,
        |engine| clonk_contents_count(engine, clonk, "FLNT") == 1,
    )?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk retreats from the first FLNT blast",
        100,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 120)
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the Clonk approaches the seam for the second FLNT",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 92)
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the second FLNT leaves the Clonk's inventory",
        60,
        |engine| clonk_contents_count(engine, clonk, "FLNT") == 0,
    )?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk retreats from the second FLNT blast",
        100,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 120)
        },
    )?;
    player.assert_milestone("the real FLNT blast exposes GOLD objects", |engine| {
        engine
            .snapshot()
            .objects
            .into_iter()
            .any(|object| object.definition_id == "GOLD")
    })?;

    player.hold_until(
        COM_LEFT,
        "the Clonk naturally collects one exposed GOLD chunk",
        160,
        |engine| clonk_carries(engine, clonk, "GOLD"),
    )?;
    carry_gold_to_hut(&mut player, clonk, elevator_case, hut, owner, 5)?;
    for target_wealth in [10, 15, 20] {
        return_from_hut_and_collect_gold(
            &mut player,
            clonk,
            elevator_case,
            hut,
            owner,
        )?;
        carry_gold_to_hut(
            &mut player,
            clonk,
            elevator_case,
            hut,
            owner,
            target_wealth,
        )?;
    }

    let workshop = object_with_definition(player.engine(), "WRKS")
        .expect("Tutorial07 creates the player's workshop");
    player.wait_until(
        "HUT3 restores context after the fourth GOLD sale",
        30,
        |engine| object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14)),
    )?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the funded Clonk exits HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container != Some(hut))
    })?;

    player.press(COM_RIGHT)?;
    let mut previous_action = player
        .engine()
        .object_snapshot(clonk)
        .expect("the funded Clonk survives HUT3 exit")
        .action
        .name;
    for _ in 0..360 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the funded Clonk survives the workshop walk");
        if clonk_now.position.x >= 155 {
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
    player.assert_milestone("the funded Clonk reaches WRKS", |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= 155)
    })?;
    player.wait_until("the funded Clonk lands in WRKS's entrance", 120, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the funded Clonk aligns with WRKS's entrance",
        100,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 160)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the funded Clonk enters WRKS", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(workshop))
    })?;
    player.wait_until("WRKS opens its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Production")?;
    player.menu_enter()?;
    player.wait_until("WRKS opens its real production menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::C4Id("CXCN".to_owned()))
    })?;
    let balloon_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "BALN"))
        .expect("Tutorial07 gives the player BALN production knowledge");
    player.menu_navigate_to_index(balloon_index)?;
    player.menu_enter()?;
    let balloon = player
        .wait_until("WRKS creates the real BALN construction", 80, |engine| {
            object_with_definition(engine, "BALN").is_some()
        })
        .map(|_| object_with_definition(player.engine(), "BALN").expect("BALN exists"))?;
    player
        .wait_until(
            "WRKS completes the BALN through normal production",
            2_400,
            |engine| {
                engine
                    .object_snapshot(balloon)
                    .is_some_and(|object| object.construction == 100_000)
            },
        )
        .map_err(|error| {
            let relevant = player
                .engine()
                .snapshot()
                .objects
                .into_iter()
                .filter(|object| {
                    matches!(
                        object.definition_id.as_str(),
                        "BALN" | "WOOD" | "METL" | "HUT3" | "WRKS" | "CLNK"
                    )
                })
                .collect::<Vec<_>>();
            format!("{error}; production_objects={relevant:?}")
        })?;

    // A completed internal vehicle is activated by C4Command::Build and
    // receives its ordinary Exit command (C4Command.cpp:823-899). Continue
    // only once both the product and worker have left WRKS through normal
    // command execution.
    player.wait_until("the completed BALN exits WRKS", 160, |engine| {
        engine
            .object_snapshot(balloon)
            .is_some_and(|object| object.container.is_none())
    })?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.container.is_some())
    {
        if player.engine().cursor_object_menu(owner).is_none() {
            player.tap(COM_UP)?;
            player.wait_until("WRKS restores its context menu", 30, |engine| {
                object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
            })?;
        }
        player.menu_navigate_to_caption("Exit")?;
        player.menu_enter()?;
    }
    player.wait_until("the balloon builder exits WRKS", 100, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
    })?;

    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk boards the produced BALN", 100, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon)
        })
    })?;
    player.press(COM_UP)?;
    player.wait_until(
        "the BALN climbs to the crystal flight level",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y <= 115)
        },
    )?;
    player.release(COM_UP)?;
    player.tap(COM_DOWN)?;
    player.ticks(11)?;
    player.wait_until("the BALN reaches the opposite cliff", 900, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push"
                && object.action.target == Some(balloon)
                && object.position.x >= 570
        })
    })?;
    let crystal = object_with_definition(player.engine(), "CRYS")
        .expect("Tutorial07 creates its objective crystal");
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the Clonk leaves BALN and lands on the crystal cliff",
        180,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && object.container.is_none()
                    && object.position.x >= 570
            })
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk crosses to the far side of Tutorial07's CRYS",
        120,
        |engine| {
            clonk_carries(engine, clonk, "CRYS")
                || engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 650)
        },
    )?;
    if !clonk_carries(player.engine(), clonk, "CRYS") {
        player
            .hold_until(
                COM_LEFT,
                "the Clonk naturally collects Tutorial07's CRYS",
                180,
                |engine| clonk_carries(engine, clonk, "CRYS"),
            )
            .map_err(|error| {
                format!(
                    "{error}; crystal={:?}; balloon={:?}",
                    player.engine().object_snapshot(crystal),
                    player.engine().object_snapshot(balloon)
                )
            })?;
    }
    player.assert_milestone(
        "the objective crystal is in the Clonk inventory",
        |engine| {
            engine
                .object_snapshot(crystal)
                .is_some_and(|object| object.container == Some(clonk))
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the crystal-carrying Clonk steps fully onto the cliff",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 650 && object.action.name == "Walk")
        },
    )?;
    player.wait_until(
        "Tutorial07 asks the Clonk to dig to the sailboat",
        240,
        |engine| tutorial_message_contains(engine, "Dig through the earth"),
    )?;
    player.tap(COM_DIG)?;
    player.press(COM_DOWN)?;
    player.wait_until("the crystal-carrying Clonk starts digging", 1, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_LEFT)?;
    let tunnel_exit = player.wait_until(
        "the diagonal tunnel opens toward the sailboat cave",
        260,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 575)
        },
    );
    player.release(COM_LEFT)?;
    player.release(COM_DOWN)?;
    tunnel_exit?;
    let sailboat = object_with_definition(player.engine(), "SLBS")
        .or_else(|| object_with_definition(player.engine(), "SLBT"))
        .expect("Tutorial07 creates its return sailboat");
    for lip in 1..=12 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the crystal-carrying Clonk survives above the sailboat");
        if clonk_now.position.y >= 290 {
            break;
        }
        if clonk_now.action.name.starts_with("Scale") {
            player.hold_until(
                COM_DOWN,
                format!("the crystal-carrying Clonk descends cave lip {lip}"),
                180,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.y >= 290 || object.action.name == "Walk"
                    })
                },
            )?;
        } else {
            player.hold_until(
                COM_LEFT,
                format!("the crystal-carrying Clonk reaches cave lip {lip}"),
                120,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.y >= 290 || object.action.name.starts_with("Scale")
                    })
                },
            )?;
        }
    }
    for segment in 1..=8 {
        let start_position = player
            .engine()
            .object_snapshot(clonk)
            .expect("the Clonk survives between cave ledges")
            .position;
        if start_position.y >= 290 {
            break;
        }
        player.tap(COM_DIG)?;
        player.press(COM_DOWN)?;
        if segment >= 2 {
            player.press(COM_RIGHT)?;
        }
        player.wait_until(
            format!("the Clonk starts cave dig segment {segment}"),
            1,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        )?;
        player.assert_milestone(
            format!("cave dig segment {segment} has the requested heading"),
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.command_direction
                        == if segment == 1 {
                            CommandDirection::Down
                        } else {
                            CommandDirection::DownRight
                        }
                })
            },
        )?;
        let segment_descent = player.wait_until(
            format!("cave dig segment {segment} reaches air or the sailboat"),
            180,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.position.y >= 290 || object.action.name == "Walk"
                })
            },
        );
        if segment >= 2 {
            player.release(COM_RIGHT)?;
        }
        player.release(COM_DOWN)?;
        segment_descent?;
        if segment == 2 {
            for lip in 1..=12 {
                let step_start_y = player
                    .engine()
                    .object_snapshot(clonk)
                    .expect("the Clonk survives the tunnel descent")
                    .position
                    .y;
                if step_start_y >= 290 {
                    break;
                }
                if player
                    .engine()
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
                {
                    player.hold_until(
                        COM_RIGHT,
                        format!("the Clonk walks off tunnel lip {lip}"),
                        120,
                        |engine| {
                            engine.object_snapshot(clonk).is_some_and(|object| {
                                object.position.y >= 290
                                    || matches!(object.action.name.as_str(), "Jump" | "Scale")
                            })
                        },
                    )?;
                }
                player.wait_until(
                    format!("the Clonk clears or catches tunnel lip {lip}"),
                    30,
                    |engine| {
                        engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.y >= 290 || object.action.name == "Scale"
                        })
                    },
                )?;
                if player
                    .engine()
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 290)
                {
                    break;
                }
                player.press(COM_DOWN)?;
                let scale_step = player.wait_until(
                    format!("the Clonk scales below tunnel lip {lip}"),
                    120,
                    |engine| {
                        engine.object_snapshot(clonk).is_some_and(|object| {
                            object.position.y >= 290
                                || (object.action.name == "Walk"
                                    && object.position.y > step_start_y)
                        })
                    },
                );
                player.release(COM_DOWN)?;
                scale_step?;
            }
        }
    }
    player.assert_milestone("the Clonk digs into the sailboat cave", |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.y >= 290)
    })?;
    player.assert_milestone(
        "the crystal-carrying Clonk descends to the sailboat",
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y >= 290)
        },
    )?;
    for approach in 1..=12 {
        let (clonk_now, sailboat_now) = player
            .engine()
            .object_snapshot(clonk)
            .zip(player.engine().object_snapshot(sailboat))
            .expect("Clonk and sailboat survive the cave approach");
        let x_distance = (clonk_now.position.x - sailboat_now.position.x).abs();
        let y_distance = (clonk_now.position.y - sailboat_now.position.y).abs();
        if x_distance <= 5 && y_distance <= 20 && clonk_now.action.name == "Walk" {
            break;
        }
        if clonk_now.action.name.starts_with("Scale") {
            if clonk_now.position.y > sailboat_now.position.y + 10 {
                player.hold_until(
                    COM_UP,
                    format!("the Clonk climbs toward the sailboat on approach {approach}"),
                    180,
                    |engine| {
                        engine
                            .object_snapshot(clonk)
                            .zip(engine.object_snapshot(sailboat))
                            .is_some_and(|(clonk, sailboat)| {
                                clonk.position.y <= sailboat.position.y + 10
                                    || clonk.action.name == "Walk"
                            })
                    },
                )?;
            } else {
                let away_from_wall = if clonk_now.direction == Direction::Left {
                    COM_RIGHT
                } else {
                    COM_LEFT
                };
                player.tap(away_from_wall)?;
            }
            continue;
        }
        if clonk_now.action.name == "Jump" {
            player.wait_until(
                format!("the Clonk lands during sailboat approach {approach}"),
                120,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        matches!(object.action.name.as_str(), "Walk" | "Scale" | "ScaleDown")
                    })
                },
            )?;
            continue;
        }
        let horizontal = if clonk_now.position.x < sailboat_now.position.x - 5 {
            COM_RIGHT
        } else {
            COM_LEFT
        };
        player.hold_until(
            horizontal,
            format!("the Clonk closes on the sailboat during approach {approach}"),
            180,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .zip(engine.object_snapshot(sailboat))
                    .is_some_and(|(clonk, sailboat)| {
                        ((clonk.position.x - sailboat.position.x).abs() <= 5
                            && (clonk.position.y - sailboat.position.y).abs() <= 20)
                            || clonk.action.name != "Walk"
                    })
            },
        )?;
    }
    player
        .assert_milestone("the Clonk reaches the sailboat", |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(sailboat))
                .is_some_and(|(clonk, sailboat)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - sailboat.position.x).abs() <= 5
                        && (clonk.position.y - sailboat.position.y).abs() <= 20
                })
        })
        .map_err(|error| {
            format!(
                "{error}; clonk={:?}; sailboat={:?}",
                player.engine().object_snapshot(clonk),
                player.engine().object_snapshot(sailboat)
            )
        })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the crystal-carrying Clonk grabs the sailboat",
        100,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(sailboat)
            })
        },
    )?;

    // Script17 asks the player to sail home after grabbing SLBS
    // (Tutorial07.c4s/Script.c:104-111). SLBS forwards the held left control
    // to its ordinary ControlUpdate/Wind2Sail path (Sailing.c4d/Script.c:29-37,
    // 64-78); no vehicle position is injected by the virtual player.
    player.wait_until("Tutorial07 asks the Clonk to sail home", 120, |engine| {
        tutorial_message_contains(engine, "Use the boat to sail back home")
    })?;
    player.hold_until(
        COM_LEFT,
        "the sailboat reaches Tutorial07's home cave",
        900,
        |engine| {
            engine
                .object_snapshot(sailboat)
                .is_some_and(|object| object.position.x <= 210)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk steps off the sailboat at home", 100, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the crystal-carrying Clonk walks from SLBS into the home cave",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 170)
        },
    )?;
    player.wait_until("the Clonk stands inside the home cave", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the crystal-carrying Clonk reaches the blast-pocket wall",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 70)
        },
    )?;
    climb_right_out_of_blast_pocket(
        &mut player,
        clonk,
        105,
        "the crystal-carrying Clonk climbs into the elevator shaft",
    )?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        let ceiling_control = player
            .engine()
            .object_snapshot(clonk)
            .zip(player.engine().object_snapshot(elevator_case))
            .map(|(clonk, elevator)| {
                if clonk.position.x > elevator.position.x {
                    COM_LEFT
                } else {
                    COM_RIGHT
                }
            })
            .expect("Clonk and ELEC survive the lower cave crossing");
        player.hold_until(
            ceiling_control,
            "the crystal-carrying Clonk crosses the lower cave ceiling",
            180,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name != "Hangle")
            },
        )?;
        player.wait_until(
            "the crystal-carrying Clonk lands at the elevator shaft",
            120,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" || object.action.name.starts_with("Scale")
                })
            },
        )?;
    }
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| !object.action.name.starts_with("Scale"))
    {
        player.tap(COM_UP)?;
        player.wait_until(
            "the crystal-carrying Clonk catches the elevator shaft wall",
            80,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name.starts_with("Scale")
                        || object.action.name == "Hangle"
                        || object.position.y <= 270
                })
            },
        )?;
    }
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        let ceiling_control = player
            .engine()
            .object_snapshot(clonk)
            .zip(player.engine().object_snapshot(elevator_case))
            .map(|(clonk, elevator)| {
                if clonk.position.x > elevator.position.x {
                    COM_LEFT
                } else {
                    COM_RIGHT
                }
            })
            .expect("Clonk and ELEC survive the shaft ceiling crossing");
        player.hold_until(
            ceiling_control,
            "the crystal-carrying Clonk crosses the shaft ceiling",
            180,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name != "Hangle")
            },
        )?;
        player.wait_until(
            "the crystal-carrying Clonk lands inside the elevator shaft",
            120,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" || object.action.name.starts_with("Scale")
                })
            },
        )?;
    }
    if player.engine().object_snapshot(clonk).is_some_and(|object| {
        !object.action.name.starts_with("Scale") && object.position.y > 270
    }) {
        player.tap(COM_UP)?;
        player.wait_until(
            "the crystal-carrying Clonk catches the inner elevator shaft wall",
            80,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name.starts_with("Scale") || object.position.y <= 270
                })
            },
        )?;
    }
    let shaft_climb_control = player
        .engine()
        .object_snapshot(clonk)
        .map(|object| {
            if object.action.name.starts_with("Scale") && object.direction == Direction::Left {
                COM_LEFT
            } else {
                COM_RIGHT
            }
        })
        .expect("the crystal-carrying Clonk survives inside the elevator shaft");
    player.hold_until(
        shaft_climb_control,
        "the crystal-carrying Clonk climbs the home elevator shaft",
        300,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.y <= 215 && object.action.name == "Walk")
        },
    )?;
    player.wait_until(
        "the crystal-carrying Clonk stands on the home surface",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.press(COM_LEFT)?;
    let mut previous_action = player
        .engine()
        .object_snapshot(clonk)
        .expect("the crystal-carrying Clonk survives the shaft ascent")
        .action
        .name;
    for _ in 0..300 {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the crystal-carrying Clonk survives the cabin walk");
        if clonk_now.position.x <= 70 {
            break;
        }
        let action = clonk_now.action.name;
        let entered_scale = action.starts_with("Scale") && !previous_action.starts_with("Scale");
        let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
        let landed = action == "Walk" && previous_action != "Walk";
        if entered_scale {
            player.release(COM_LEFT)?;
            player.press(COM_LEFT)?;
        } else if landed || left_scale_in_flight {
            player.tap(COM_UP)?;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(COM_LEFT)?;
    player.assert_milestone("the crystal-carrying Clonk reaches HUT3", |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x <= 70)
    })?;
    player.wait_until(
        "the crystal-carrying Clonk lands beside HUT3",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the crystal-carrying Clonk aligns with HUT3's entrance",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 62)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the crystal-carrying Clonk enters HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;

    // Script18 unwraps the CRYS container through the Clonk into HUT3, and
    // Script19 fulfills SCRG after the base's normal sale removes CRYS
    // (Tutorial07.c4s/Script.c:113-127).
    player.wait_until("Tutorial07 asks the player to sell CRYS", 240, |engine| {
        tutorial_message_contains(engine, "Sell the crystal")
    })?;
    player.wait_until("HUT3 opens context for the carried CRYS", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Put")?;
    player.menu_enter()?;
    player.wait_until("context Put transfers CRYS into HUT3", 40, |engine| {
        engine
            .object_snapshot(crystal)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("HUT3 restores context after putting CRYS", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Sell")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its real Sell menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(5))
    })?;
    let crystal_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "CRYS"))
        .expect("HUT3 offers the deposited CRYS for sale");
    player.menu_navigate_to_index(crystal_index)?;
    player.menu_enter()?;
    player.wait_until(
        "selling CRYS removes the real objective object",
        60,
        |engine| engine.object_snapshot(crystal).is_none(),
    )?;
    player.wait_until("Tutorial07 selects Tutorial08", 320, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial08.c4s"
    })?;
    player.wait_until(
        "Tutorial07 fulfilled goal reaches GameOver",
        320,
        |engine| engine.snapshot().game_over,
    )?;
    player.assert_milestone("Tutorial07 records its fulfilled SCRG goal", |engine| {
        engine
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG")
    })?;
    assert!(
        player.engine().object_snapshot(crystal).is_none(),
        "Tutorial07's CRYS must be sold before SCRG is fulfilled"
    );
    Ok(())
}
