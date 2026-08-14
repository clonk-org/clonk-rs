use std::error::Error;

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::{
    CommandDirection, Direction, Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RIGHT, COM_SPECIAL2, COM_THROW, COM_UP,
};

fn load_tutorial04() -> (Engine, i32) {
    let mut engine = load_tutorial(4, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 4 virtual player".to_owned(),
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
        .expect("local Tutorial04 virtual player joins")
        .number();
    (engine, owner)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
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

fn object_contents_count(engine: &Engine, container: ObjectId, definition: &str) -> usize {
    engine.object_snapshot(container).map_or(0, |container| {
        container
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

fn recover_clonk_to_walk(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    milestone: &str,
    max_ticks: u32,
    allow_hangle: bool,
) -> Result<(), Box<dyn Error>> {
    let mut previous_action = String::new();
    let mut climbing = false;
    let mut drifting = false;
    for _ in 0..max_ticks {
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the Clonk survives its blast-pocket recovery");
        let action = clonk_now.action.name;
        if action == "Walk" || (allow_hangle && (action == "Hangle" || action.starts_with("Scale")))
        {
            if climbing {
                player.release(COM_UP)?;
            }
            if drifting {
                player.release(COM_RIGHT)?;
            }
            return Ok(());
        }
        if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
            // Stay level with the exposed vein by climbing the real crater
            // wall and taking C++'s scale-corner transition onto its upper
            // ledge (C4Object.cpp:3618-3628,4284-4289,4823-4837).
            if drifting {
                player.release(COM_RIGHT)?;
                drifting = false;
            }
            player.press(COM_UP)?;
            climbing = true;
        } else if !action.starts_with("Scale") && previous_action.starts_with("Scale") && climbing {
            player.release(COM_UP)?;
            climbing = false;
        } else if action == "Hangle" {
            if climbing {
                player.release(COM_UP)?;
                climbing = false;
            }
            if drifting {
                player.release(COM_RIGHT)?;
            }
            player.tap(COM_DOWN)?;
            player.ticks(1)?;
            player.press(COM_RIGHT)?;
            drifting = true;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    if climbing {
        player.release(COM_UP)?;
    }
    if drifting {
        player.release(COM_RIGHT)?;
    }
    player
        .assert_milestone(milestone, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        })
        .map_err(|error| {
            format!(
                "{error}; clonk={:?}",
                player.engine().object_snapshot(clonk)
            )
        })?;
    Ok(())
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn climb_from_gold_pocket(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    deepened_pocket: bool,
) -> Result<(), Box<dyn Error>> {
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| {
            (!deepened_pocket || object.position.x >= 455)
                && object.position.y <= 403
                && !object.action.name.starts_with("Scale")
        })
    {
        return Ok(());
    }
    // Successive blasts deepen the pocket below the original tunnel. The
    // right-hand wall remains the C++ route out: its facing-direction control
    // scales upward. At the diagonal throat, hold Up while letting go toward
    // the tunnel so top contact becomes Hangle, then traverse that ceiling.
    player.hold_until(
        COM_RIGHT,
        "the GOLD-carrying Clonk scales to the tunnel throat",
        360,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.position.y <= 403 && object.action.name.starts_with("Scale")
            })
        },
    )?;
    let throat = player
        .engine()
        .object_snapshot(clonk)
        .expect("GOLD-carrying Clonk reaches the tunnel throat");
    let away = if throat.direction == Direction::Left {
        COM_RIGHT
    } else {
        COM_LEFT
    };
    player.press(COM_UP)?;
    player.tap(away)?;
    player.wait_until(
        "the GOLD-carrying Clonk leaves the tunnel throat",
        120,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Hangle" || object.action.name == "Walk"
            })
        },
    )?;
    player.release(COM_UP)?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        player.hold_until(
            COM_RIGHT,
            "the GOLD-carrying Clonk traverses the upper tunnel ceiling",
            120,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.position.x >= 465 || object.action.name != "Hangle"
                })
            },
        )?;
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            player.tap(COM_DOWN)?;
        }
    }
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.position.y > 403)
    {
        if deepened_pocket {
            // Later blasts deepen the shoulder. Flush the preceding let-go,
            // launch back toward the vein, and use the new top contact to
            // hangle along the diagonal roof before turning right.
            player.wait_out_double_click()?;
            player.tap(COM_LEFT)?;
            player.tap(COM_DOWN)?;
            player.press(COM_UP)?;
            player.wait_until(
                "the GOLD-carrying Clonk reaches the diagonal tunnel roof",
                60,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Hangle"
                            || (object.action.name == "Jump" && object.position.y <= 410)
                            || (object.position.x >= 455 && object.position.y <= 403)
                    })
                },
            )?;
            player.release(COM_UP)?;
            player.hold_until(
                COM_RIGHT,
                "the GOLD-carrying Clonk traverses the diagonal tunnel roof",
                180,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.position.x >= 455 && object.position.y <= 403)
                },
            )?;
        } else {
            // The original shoulder is one jump below the upper tunnel.
            player.press(COM_RIGHT)?;
            let mut right_held = true;
            let mut previous_action = String::new();
            for _ in 0..360 {
                let clonk_now = player
                    .engine()
                    .object_snapshot(clonk)
                    .expect("GOLD-carrying Clonk survives the tunnel throat");
                let action = clonk_now.action.name;
                if clonk_now.position.y <= 403 && !action.starts_with("Scale") {
                    break;
                }
                let entered_scale =
                    action.starts_with("Scale") && !previous_action.starts_with("Scale");
                let left_scale_in_flight = action == "Jump" && previous_action.starts_with("Scale");
                let landed = action == "Walk" && previous_action != "Walk";
                if entered_scale {
                    if right_held {
                        player.release(COM_RIGHT)?;
                        right_held = false;
                    }
                    let away = if clonk_now.direction == Direction::Left {
                        COM_RIGHT
                    } else {
                        COM_LEFT
                    };
                    player.tap(away)?;
                } else if landed || left_scale_in_flight {
                    if !right_held {
                        player.press(COM_RIGHT)?;
                        right_held = true;
                    }
                    player.tap(COM_UP)?;
                } else if action == "Hangle" && previous_action != "Hangle" {
                    player.tap(COM_DOWN)?;
                }
                previous_action = action;
                player.ticks(1)?;
            }
            if right_held {
                player.release(COM_RIGHT)?;
            }
        }
    }
    player
        .assert_milestone(
            "the GOLD-carrying Clonk climbs back to the upper tunnel",
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    (!deepened_pocket || object.position.x >= 455)
                        && object.position.y <= 403
                        && !object.action.name.starts_with("Scale")
                })
            },
        )
        .map_err(|error| {
            format!(
                "{error}; clonk={:?}",
                player.engine().object_snapshot(clonk)
            )
        })?;
    Ok(())
}

fn carry_gold_from_tunnel_to_hut(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
    target_wealth: i32,
) -> Result<(), Box<dyn Error>> {
    // The repaired route uses one replacement blast and then collects every
    // remaining chunk from that same pocket. Its exit geometry therefore
    // does not deepen on later sale trips.
    climb_from_gold_pocket(player, clonk, false)?;
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

fn take_one_replacement_tfln_from_hut(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    hut: ObjectId,
    owner: i32,
) -> Result<(), Box<dyn Error>> {
    player.wait_until(
        "HUT2 restores context before another replacement TFLN trip",
        30,
        |engine| object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14)),
    )?;
    let stock_before = object_contents_count(player.engine(), hut, "TFLN");
    assert!(stock_before > 0, "HUT2 must retain a replacement TFLN");
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT2 opens replacement TFLN Contents", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
    })?;
    let tflint_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "TFLN"))
        .expect("replacement TFLN appears in HUT2 Contents");
    player.menu_navigate_to_index(tflint_index)?;
    // MenuEnterAll exercises C++ Command2; CLNK's one nonspecial slot keeps
    // one TFLN and returns any additional stack entries to HUT2.
    player.tap(COM_SPECIAL2)?;
    player.wait_until("the Clonk takes one replacement TFLN", 120, |engine| {
        clonk_contents_count(engine, clonk, "TFLN") == 1
            && object_contents_count(engine, hut, "TFLN") + 1 == stock_before
    })?;
    player.menu_close()?;
    player.wait_until(
        "HUT2 restores context after taking replacement TFLN",
        30,
        |engine| object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14)),
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
        |engine| object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14)),
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
        "the returning Clonk crosses ELEC's grab position",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.position.x <= elevator_case.position.x + 5
                })
        },
    )?;
    let alignment_action = player
        .engine()
        .object_snapshot(clonk)
        .expect("the returning Clonk reaches ELEC alignment");
    if alignment_action.action.name.starts_with("Scale") {
        let away = if alignment_action.direction == Direction::Left {
            COM_RIGHT
        } else {
            COM_LEFT
        };
        player.tap(away)?;
        player.wait_until("the returning Clonk leaves the shaft lip", 20, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| !object.action.name.starts_with("Scale"))
        })?;
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Hangle")
        {
            player.tap(COM_DOWN)?;
        }
    } else {
        player.tap(COM_DOWN)?;
    }
    player.wait_until("the returning Clonk lands beside ELEC", 90, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    let elevator_offset = player
        .engine()
        .object_snapshot(clonk)
        .zip(player.engine().object_snapshot(elevator_case))
        .map(|(clonk, elevator_case)| clonk.position.x - elevator_case.position.x)
        .expect("the returning Clonk and ELEC survive alignment");
    if elevator_offset < -5 {
        player.hold_until(
            COM_RIGHT,
            "the returning Clonk corrects its ELEC alignment",
            30,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .zip(engine.object_snapshot(elevator_case))
                    .is_some_and(|(clonk, elevator_case)| {
                        clonk.position.x >= elevator_case.position.x - 5
                    })
            },
        )?;
        player.tap(COM_DOWN)?;
    }
    player.assert_milestone("the returning Clonk stands beside ELEC", |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(clonk, elevator_case)| {
                clonk.action.name == "Walk"
                    && (clonk.position.x - elevator_case.position.x).abs() <= 5
            })
    })?;
    player.wait_out_double_click()?;
    player.tap(COM_DOWN)?;
    player.wait_until("the returning Clonk grabs ELEC", 60, |engine| {
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
    for attempt in 1..=8 {
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
            let nearest_gold_x = player
                .engine()
                .snapshot()
                .objects
                .into_iter()
                .filter(|object| object.definition_id == "GOLD" && object.container.is_none())
                .min_by_key(|object| (object.position.x - start_x).abs())
                .expect("a blasted GOLD chunk remains after collecting debris")
                .position
                .x;
            let control = if nearest_gold_x < start_x {
                COM_LEFT
            } else {
                COM_RIGHT
            };
            let away = if control == COM_LEFT {
                COM_RIGHT
            } else {
                COM_LEFT
            };
            let away_direction = if away == COM_LEFT {
                Direction::Left
            } else {
                Direction::Right
            };
            // Face away and use C++'s ordinary Throw command so incidental
            // debris cannot be collected again on the route toward GOLD
            // (C4Object.cpp:3541-3542; C4ObjectCom.cpp:650-671).
            player.hold_until(
                away,
                "the Clonk faces away from the nearest GOLD chunk",
                20,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.direction == away_direction)
                },
            )?;
            player.tap(COM_THROW)?;
            player.wait_until(
                "the empty Clonk throws incidental blast debris away",
                30,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.contents.is_empty())
                },
            )?;
            player.hold_until(
                control,
                "the Clonk moves toward GOLD and away from thrown blast debris",
                60,
                |engine| {
                    clonk_contents_count(engine, clonk, "GOLD") == 1
                        || engine.object_snapshot(clonk).is_some_and(|object| {
                            object.action.name == "Hangle"
                                || object.action.name.starts_with("Scale")
                                || if control == COM_LEFT {
                                    object.position.x <= start_x - 12
                                } else {
                                    object.position.x >= start_x + 12
                                }
                        })
                },
            )?;
            if clonk_contents_count(player.engine(), clonk, "GOLD") == 1 {
                break;
            }
        }
        let action = player
            .engine()
            .object_snapshot(clonk)
            .expect("the Clonk survives its GOLD collection route")
            .action
            .name;
        if action == "Hangle" {
            player.tap(COM_DOWN)?;
            player.wait_until(
                "the next GOLD trip drops to the tunnel floor",
                100,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Walk" || object.action.name.starts_with("Scale")
                    })
                },
            )?;
            continue;
        }
        if action.starts_with("Scale") {
            let direction = player
                .engine()
                .object_snapshot(clonk)
                .expect("the Clonk survives on the blast wall")
                .direction;
            let let_go = if direction == Direction::Left {
                COM_RIGHT
            } else {
                COM_LEFT
            };
            player.tap(let_go)?;
            player.wait_until(
                format!("the empty Clonk lets go into the blast pocket on attempt {attempt}"),
                120,
                |engine| {
                    clonk_contents_count(engine, clonk, "GOLD") == 1
                        || engine.object_snapshot(clonk).is_some_and(|object| {
                            object.action.name == "Walk"
                                || object.action.name == "Hangle"
                                || object.action.name.starts_with("Scale")
                        })
                },
            )?;
            continue;
        }
        if action != "Walk" {
            player.wait_until(
                format!("the empty Clonk settles in the blast pocket on attempt {attempt}"),
                100,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Walk"
                            || object.action.name == "Hangle"
                            || object.action.name.starts_with("Scale")
                    })
                },
            )?;
            continue;
        }

        let clonk_position = player
            .engine()
            .object_snapshot(clonk)
            .expect("the Clonk survives while choosing the next GOLD chunk")
            .position;
        let target = player
            .engine()
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "GOLD" && object.container.is_none())
            .min_by_key(|object| {
                (object.position.x - clonk_position.x).abs()
                    + (object.position.y - clonk_position.y).abs()
            })
            .expect("a blasted GOLD chunk remains in the tunnel");
        let control = if target.position.x < clonk_position.x {
            COM_LEFT
        } else {
            COM_RIGHT
        };
        player.hold_until(
            control,
            format!("the empty Clonk advances toward GOLD on attempt {attempt}"),
            220,
            |engine| {
                clonk_contents_count(engine, clonk, "GOLD") == 1
                    || engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Hangle"
                            || object.action.name.starts_with("Scale")
                            || !object.contents.is_empty()
                    })
            },
        )?;
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
    recover_clonk_to_walk(
        player,
        clonk,
        "the Clonk is stable before each replacement blast",
        60,
        true,
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
    if next_face_x < 414
        && player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| {
                object.position.x > next_face_x
                    && (object.action.name == "Hangle" || object.action.name.starts_with("Scale"))
            })
    {
        let attached = player
            .engine()
            .object_snapshot(clonk)
            .expect("the attached Clonk reaches the replacement-blast face");
        let away = if attached.direction == Direction::Left {
            COM_RIGHT
        } else {
            COM_LEFT
        };
        player.tap(away)?;
        recover_clonk_to_walk(
            player,
            clonk,
            "the Clonk lands inside the deepened blast pocket",
            120,
            false,
        )?;
    }
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.position.x > next_face_x && object.action.name == "Walk")
    {
        // Successive floor blasts can leave a low Earth lip in front of
        // the newly exposed (non-diggable) gold. Clear that lip with the
        // same real Dig+Left controls before placing the next flint.
        for _ in 0..3 {
            recover_clonk_to_walk(
                player,
                clonk,
                "the Clonk recovers before digging through the blast-pocket lip",
                80,
                false,
            )?;
            player.tap(COM_DIG)?;
            for _ in 0..30 {
                if player
                    .engine()
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig")
                {
                    break;
                }
                player.ticks(1)?;
            }
            if player
                .engine()
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
            {
                break;
            }
        }
        player.assert_milestone("the Clonk digs through the blast-pocket lip", |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        })?;
        player.press(COM_LEFT)?;
        player.press(COM_DOWN)?;
        for tick in 0..120 {
            if tick > 0
                && player
                    .engine()
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name != "Dig")
            {
                break;
            }
            if tick == 12 {
                player.release(COM_DOWN)?;
                player.press(COM_DOWN)?;
            }
            player.ticks(1)?;
        }
        player.release(COM_DOWN)?;
        player.release(COM_LEFT)?;
        recover_clonk_to_walk(
            player,
            clonk,
            "the Dig action reaches the next blast face",
            60,
            false,
        )?;
        player.hold_until(
            COM_LEFT,
            "the Clonk grips the deepened replacement-blast face",
            80,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Hangle" || object.action.name.starts_with("Scale")
                })
            },
        )?;
        if next_face_x > 390 {
            player.wait_out_double_click()?;
            player.hold_until(
                COM_DOWN,
                "the Clonk descends the deepened replacement-blast face",
                40,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.position.y >= 438 || !object.action.name.starts_with("Scale")
                    })
                },
            )?;
        }
    }
    recover_clonk_to_walk(
        player,
        clonk,
        "the Clonk stands before throwing at the exposed gold vein",
        80,
        true,
    )?;
    // Use C++'s ordinary Throw command. Walking faces the exposed vein;
    // Scale/Hangle routes the same command to Drop at the attached crater
    // face (C4Object.cpp:3541-3545; C4ObjectCom.cpp:650-671).
    let attached = player
        .engine()
        .object_snapshot(clonk)
        .filter(|object| object.action.name == "Hangle" || object.action.name.starts_with("Scale"))
        .is_some();
    if !attached {
        player.hold_until(
            COM_LEFT,
            "the Clonk faces the exposed GOLD vein",
            30,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && object.direction == Direction::Left
                })
            },
        )?;
        player.ticks(1)?;
    }
    player.wait_out_double_click()?;
    let carried_tflns = player
        .engine()
        .object_snapshot(clonk)
        .map(|clonk| {
            clonk
                .contents
                .into_iter()
                .filter(|item| {
                    player
                        .engine()
                        .object_snapshot(*item)
                        .is_some_and(|item| item.definition_id == "TFLN")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let thrown = carried_tflns
        .first()
        .copied()
        .expect("the Clonk carries the replacement TFLN");
    if next_face_x < 414 && !attached {
        // Arm the TFLN while it still occupies the Clonk's only inventory
        // slot, so blast debris cannot take that slot before the final throw.
        // Double Dig activates Contents(0), and Activated lasts sixty ticks
        // (C4ObjectCom.cpp:531-539; TFlint.c4d/ActMap.txt:1-10).
        player.double_tap(COM_DIG)?;
        player.wait_until(
            "the carried replacement TFLN reaches its final fuse",
            60,
            |engine| {
                engine.object_snapshot(thrown).is_some_and(|flint| {
                    flint.container == Some(clonk)
                        && flint.action.name == "Activated"
                        && flint.action.time >= 48
                })
            },
        )?;
    }
    // C++ throws while walking and drops while scaling/hangling, so this one
    // physical command follows the procedure-specific behavior a player gets.
    player.tap(COM_THROW)?;
    player.wait_until(
        "a replacement TFLN leaves the Clonk's inventory",
        30,
        |engine| {
            clonk_contents_count(engine, clonk, "TFLN") == remaining
                && engine
                    .object_snapshot(thrown)
                    .is_some_and(|flint| flint.container.is_none())
        },
    )?;
    // Keep the retreat control active for the complete fuse. The thrown TFLN
    // continues moving after it first reaches a safe radius, so stopping at a
    // snapshot distance can let it bounce back and detonate beside the Clonk.
    player.press(COM_RIGHT)?;
    let mut held_control = COM_RIGHT;
    let mut previous_action = String::new();
    for _ in 0..180 {
        let flint = player.engine().object_snapshot(thrown);
        if flint.is_none() {
            break;
        }
        let clonk_now = player
            .engine()
            .object_snapshot(clonk)
            .expect("the Clonk survives its replacement-flint retreat");
        let action = clonk_now.action.name.clone();
        if action.starts_with("Scale") && !previous_action.starts_with("Scale") {
            player.release(held_control)?;
            if clonk_now.direction == Direction::Left {
                // The held Right that began while walking does not become a
                // new scale control. Briefly reverse, then re-emit Right:
                // changing controls clears C++'s double-click buffer, so
                // Right reaches the scale handler as a plain edge and the
                // left-facing scaler lets go toward safety
                // (C4Player.cpp:1522-1536; C4Object.cpp:3451-3461).
                player.tap(COM_LEFT)?;
                player.press(COM_RIGHT)?;
                held_control = COM_RIGHT;
            } else {
                // At the far side of the blast pocket the wall faces right.
                // Climb it and use C++'s ordinary scale-corner transition
                // before resuming the retreat (C4Object.cpp:3618-3628,
                // 4284-4289).
                player.press(COM_UP)?;
                held_control = COM_UP;
            }
        } else if !action.starts_with("Scale")
            && previous_action.starts_with("Scale")
            && held_control == COM_UP
        {
            player.release(COM_UP)?;
            player.press(COM_RIGHT)?;
            held_control = COM_RIGHT;
        }
        previous_action = action;
        player.ticks(1)?;
    }
    player.release(held_control)?;
    player.assert_milestone("the replacement TFLN detonates", |engine| {
        engine.object_snapshot(thrown).is_none()
    })?;
    recover_clonk_to_walk(
        player,
        clonk,
        "the Clonk recovers after each replacement blast",
        180,
        true,
    )?;
    Ok(())
}

fn blast_replacement_tfln_and_collect_gold(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    next_face_x: i32,
) -> Result<(), Box<dyn Error>> {
    player.assert_milestone("the Clonk carries one replacement TFLN", |engine| {
        clonk_contents_count(engine, clonk, "TFLN") == 1
    })?;
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
        .is_some_and(|object| {
            object.action.name != "Hangle" && !object.action.name.starts_with("Scale")
        })
    {
        player.tap(COM_DOWN)?;
    }
    player.wait_until(
        "the Clonk stabilizes at a replacement-blast distance",
        60,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    || object.action.name == "Hangle"
                    || object.action.name.starts_with("Scale")
            })
        },
    )?;
    let gold_before = player
        .engine()
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "GOLD")
        .count();
    blast_one_tfln(player, clonk, next_face_x, 0)?;
    player.wait_until(
        "a replacement TFLN frees another GOLD chunk",
        180,
        |engine| {
            engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == "GOLD")
                .count()
                > gold_before
        },
    )?;
    player.ticks(120)?;
    collect_one_gold_in_tunnel(player, clonk)
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT2 opens its Contents menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
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
        |engine| object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14)),
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::C4Id("CXCN".into()))
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
        130,
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
    player.wait_until(
        "the TFLN-carrying Clonk settles beside ELEC",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator.position.x).abs() <= 5
                })
        },
    )?;
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
            || player
                .engine()
                .object_snapshot(clonk)
                .is_some_and(|object| {
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
    player.assert_milestone(
        "the real dig tunnel reaches Tutorial04's gold vein",
        |engine| {
            tutorial_message_contains(engine, "struck solid gold")
                || engine.object_snapshot(clonk).is_some_and(|object| {
                    (357..437).contains(&object.position.x)
                        && (348..428).contains(&object.position.y)
                })
        },
    )?;
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
    // Correct MapSeed terrain leaves the blast-surviving Clonk scaling one
    // tunnel wall. C++ Scale controls let go by pressing away from that wall;
    // Down only climbs downward and therefore cannot release this contact
    // (C4Object.cpp:3436-3452).
    let let_go_control = match player
        .engine()
        .object_snapshot(clonk)
        .expect("Clonk survives the first TFLN blast")
        .direction
    {
        Direction::Left => COM_RIGHT,
        _ => COM_LEFT,
    };
    player.tap(let_go_control)?;
    player.wait_until("the Clonk leaves the tunnel wall", 20, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name != "Scale")
    })?;
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.action.name == "Hangle")
    {
        // The adjacent ceiling can catch the physical away-jump. Down is the
        // ordinary C++ Hangle let-go control; keep steering away while
        // falling so the Clonk does not immediately catch the same wall
        // again (C4Object.cpp:3453-3466).
        player.tap(COM_DOWN)?;
        player.hold_until(
            let_go_control,
            "the Clonk drops from the tunnel ceiling",
            90,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        )?;
    } else {
        player.wait_until("the Clonk drops from the tunnel ceiling", 90, |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        })?;
    }
    player.hold_until(
        COM_LEFT,
        "the Clonk reaches the opened gold shaft wall",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Scale")
        },
    )?;
    player.hold_until(
        COM_DOWN,
        "the Clonk scales down to the freed GOLD chunks",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.y >= 410)
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk stands above a freed GOLD chunk",
        30,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 438)
        },
    )?;
    player.tap(COM_DOWN)?;
    assert_eq!(
        player
            .engine()
            .object_snapshot(clonk)
            .expect("Clonk stands above the first GOLD chunks")
            .direction,
        Direction::Right
    );
    player.tap(COM_DIG)?;
    player.wait_until(
        "the Clonk starts opening the floor above GOLD",
        30,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Dig")
        },
    )?;
    // A right-facing Dig begins DownRight. One real Left edge rotates the
    // active C++ dig direction to Down, opening the thin floor rather than
    // climbing the opposite wall (C4Object.cpp:3467-3470,4354-4368).
    player.tap(COM_LEFT)?;
    player.wait_until(
        "the opened floor yields a collectible chunk",
        120,
        |engine| clonk_carries(engine, clonk, "GOLD") || clonk_carries(engine, clonk, "ROCK"),
    )?;
    if clonk_carries(player.engine(), clonk, "ROCK") {
        // DigFree can expose the neighbouring ROCK before GOLD. Classic
        // Clonks have one inventory slot, so physically throw that chunk away
        // from the gold pocket before collecting the objective.
        player.press(COM_RIGHT)?;
        player.wait_until(
            "the ROCK-carrying Clonk faces away from GOLD",
            10,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.direction == Direction::Right)
            },
        )?;
        player.release(COM_RIGHT)?;
        player.tap(COM_DOWN)?;
        player.tap(COM_THROW)?;
        player.wait_until("the ROCK leaves the Clonk's inventory", 30, |engine| {
            !clonk_carries(engine, clonk, "ROCK")
        })?;
        collect_one_gold_in_tunnel(&mut player, clonk)?;
    }
    player.assert_milestone("the Clonk carries GOLD from the first blast", |engine| {
        clonk_contents_count(engine, clonk, "GOLD") >= 1
    })?;
    climb_from_gold_pocket(&mut player, clonk, false)?;
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
    let first_gold = player
        .engine()
        .object_snapshot(clonk)
        .and_then(|clonk| {
            clonk.contents.into_iter().find(|item| {
                player
                    .engine()
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "GOLD")
            })
        })
        .expect("the physically returning Clonk still carries the first GOLD");
    let wealth_before_sale = player
        .engine()
        .player(owner)
        .expect("Tutorial04 player remains live")
        .wealth();
    let gold_stock_before_sale = player
        .engine()
        .player(owner)
        .expect("Tutorial04 player remains live")
        .home_base_material()
        .get("GOLD")
        .copied()
        .unwrap_or(0);
    let hut_energy_before_sale = player
        .engine()
        .object_snapshot(hut)
        .expect("Tutorial04 HUT2 remains live")
        .energy;
    assert_eq!(
        wealth_before_sale, 0,
        "Tutorial04 reaches its first physical sale with zero wealth"
    );
    assert_eq!(
        gold_stock_before_sale, 0,
        "Tutorial04 has no pre-existing GOLD home-base stock"
    );
    assert_eq!(
        player
            .engine()
            .object_snapshot(first_gold)
            .and_then(|gold| gold.container),
        Some(clonk),
        "GOLD is still nested in CLNK immediately before the physical Up control"
    );

    // GOLD has BaseAutoSell=1 and Value=5 but no Rebuy flag. C++ runs
    // RejectEntrance, Collection2 and Entrance before synchronously invoking
    // the destination base's AutoSellContents inside the successful Enter
    // call. AutoSellContents exits nested GOLD, Sell2Home adds its value,
    // declines to introduce non-rebuyable stock, runs Sale, and removes it
    // (C4Object.cpp:1577-1634,970-995; C4Player.cpp:866-902; GOLD DefCore).
    player.tap(COM_UP)?;
    let mut entry_frame = None;
    for _ in 0..=60 {
        if player
            .engine()
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
        {
            entry_frame = Some(player.engine().frame());
            break;
        }
        assert_eq!(
            player
                .engine()
                .object_snapshot(first_gold)
                .and_then(|gold| gold.container),
            Some(clonk),
            "before HUT2 entry, GOLD must remain carried"
        );
        assert_eq!(
            player
                .engine()
                .player(owner)
                .expect("Tutorial04 player remains live")
                .wealth(),
            wealth_before_sale,
            "HUT2 cannot sell GOLD before the Clonk enters"
        );
        player.ticks(1)?;
    }
    let entry_frame = entry_frame.expect("the physical Up control enters HUT2 within 60 ticks");
    assert!(
        player.engine().object_snapshot(first_gold).is_none(),
        "nested GOLD is removed in the same successful Enter call (frame {entry_frame})"
    );
    assert_eq!(
        clonk_contents_count(player.engine(), clonk, "GOLD"),
        0,
        "the entered Clonk has no GOLD after the synchronous sale"
    );
    assert_eq!(
        object_contents_count(player.engine(), hut, "GOLD"),
        0,
        "GOLD never remains as a live direct HUT2 content"
    );
    // Command execution precedes ExecLife in the same object pass. If entry
    // lands on Tick3, HUT2 can therefore spend GOLD's five wealth on its
    // first BaseRegenerateEnergy purchase before the route observes the
    // frame (C4Object.cpp:1082-1107,814-856). Pin the durable outcome
    // instead of a transient pre-purchase balance.
    player.wait_until(
        "the first GOLD sale funds HUT2's initial energy reserve",
        3,
        |engine| {
            engine.player_wealth(owner) == Some(wealth_before_sale)
                && engine
                    .object_snapshot(hut)
                    .is_some_and(|hut| hut.energy > hut_energy_before_sale)
        },
    )?;
    assert_eq!(
        player
            .engine()
            .player(owner)
            .expect("Tutorial04 player remains live")
            .home_base_material()
            .get("GOLD")
            .copied()
            .unwrap_or(0),
        gold_stock_before_sale,
        "non-rebuyable GOLD does not introduce a new home-base stock entry"
    );

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
    take_one_replacement_tfln_from_hut(&mut player, clonk, hut, owner)?;
    player.assert_milestone("C++ Get keeps one TFLN and puts two back", |engine| {
        clonk_contents_count(engine, clonk, "TFLN") == 1
            && object_contents_count(engine, hut, "TFLN") == 2
    })?;
    return_from_hut_to_tunnel(&mut player, clonk, elevator_case, hut, owner)?;

    // Current seed-zero terrain puts the remaining vein inside one attached
    // replacement-TFLN blast at x=402. The first five wealth buy HUT2's
    // initial energy reserve, so six physical GOLD sales are needed to leave
    // the 25 points Script251 requires.
    blast_replacement_tfln_and_collect_gold(&mut player, clonk, 402)?;
    player.assert_milestone(
        "the Clonk carries exactly one nonspecial GOLD chunk",
        |engine| clonk_contents_count(engine, clonk, "GOLD") == 1,
    )?;

    // CLNK permits only one nonspecial inventory object: RejectCollect ends
    // with GetNonSpecialCount() >= MaxContentsCount(), and
    // MaxContentsCount() is 1. Each remaining chunk therefore takes its own
    // elevator/base trip (Clonk.c4d/Script.c:738-763; Tutorial04 Script.c:227-234).
    for sold_chunks in 2..=6 {
        let target_wealth = (sold_chunks - 1) * 5;
        carry_gold_from_tunnel_to_hut(
            &mut player,
            clonk,
            elevator_case,
            hut,
            owner,
            target_wealth,
        )?;
        if sold_chunks < 6 {
            return_from_hut_and_collect_one_gold(&mut player, clonk, elevator_case, hut, owner)?;
        }
    }
    player.wait_until("Tutorial04 selects Tutorial05", 640, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial05.c4s"
    })?;
    player.wait_until(
        "Tutorial04 fulfilled goal reaches GameOver",
        320,
        Engine::is_game_over,
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
