use std::error::Error;

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::{
    CommandDirection, Direction, Engine, JoinPlayerConfig, Landscape, ObjectId, Vector2, COM_DIG,
    COM_DOWN, COM_LEFT, COM_RIGHT, COM_SPECIAL2, COM_THROW, COM_UP,
};

fn load_tutorial07() -> (Engine, i32) {
    let mut engine = load_tutorial(7, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 7 virtual player".to_owned(),
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
        .expect("local Tutorial07 virtual player joins")
        .number();
    (engine, owner)
}

#[test]
fn tutorial07_workshop_basement_keeps_cpp_pre_growth_creation_position() {
    // Numeric oracle: unmodified C++ Tutorial07 with LC_PIN_SEED=0, logged
    // immediately after C4Player::ScenarioInit. PlaceReadyBase calls
    // CreateObjectConstruction(FullCon,true) (C4Player.cpp:580-600), which
    // prepares terrain before NewObject (C4Game.cpp:1191-1238). WRKS is
    // created with construction bottom y=209; its included BAS7 Construction
    // callback creates the basement at object y+8 before initial DoCon
    // (Basement72.c4d/Script.c:72-78; C4Object.cpp:1428-1511). Initial growth
    // therefore lifts WRKS to y=184 and BAS7 to y=213. The probe recorded
    // GetX/GetY for both objects and Surface8 density at the two workshop
    // crossing columns below.
    let (engine, _) = load_tutorial07();
    let workshop = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "WRKS")
        .expect("Tutorial07 creates WRKS");
    let basement = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "BAS7")
        .expect("WRKS Construction creates BAS7");

    assert_eq!(workshop.position, Vector2::new(150, 184));
    assert_eq!(basement.position, Vector2::new(150, 213));
    let grid = engine
        .landscape()
        .and_then(Landscape::pixel_grid)
        .expect("Tutorial07 has an exact Surface8 grid");
    let crossing_densities =
        [145, 129].map(|x| (x, grid.density_at(x, 208), grid.density_at(x, 209)));
    assert_eq!(
        crossing_densities,
        [(145, Some(0), Some(100)), (129, Some(0), Some(100))]
    );
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
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

fn clonk_contents_count(engine: &Engine, clonk: ObjectId, definition: &str) -> usize {
    object_contents_count(engine, clonk, definition)
}

fn clonk_carries(engine: &Engine, clonk: ObjectId, definition: &str) -> bool {
    clonk_contents_count(engine, clonk, definition) != 0
}

fn player_wealth(engine: &Engine, owner: i32) -> i32 {
    engine.player_wealth(owner).unwrap_or(0)
}

struct DetonationTransition {
    center: Vector2,
    before: Landscape,
    after: Landscape,
}

const C4M_SOLID: i32 = 50; // DensitySolid, C4Material.h:200.
const FLNT_BLAST_RADIUS: i32 = 18;

fn assert_flint_blast_changes_terrain(detonation: &DetonationTransition, subject: &str) {
    let before_grid = detonation
        .before
        .pixel_grid()
        .expect("Tutorial07 pre-blast Surface8");
    let after_grid = detonation
        .after
        .pixel_grid()
        .expect("Tutorial07 post-blast Surface8");
    let mut changed_pixels = 0;
    let mut removed_solid_pixels = 0;
    for y_offset in -FLNT_BLAST_RADIUS..=FLNT_BLAST_RADIUS {
        let line_width =
            ((FLNT_BLAST_RADIUS * FLNT_BLAST_RADIUS - y_offset * y_offset) as f64).sqrt() as i32;
        let y = detonation.center.y + y_offset;
        for x_offset in -line_width..line_width + i32::from(line_width == 0) {
            let x = detonation.center.x + x_offset;
            changed_pixels += usize::from(before_grid.byte_at(x, y) != after_grid.byte_at(x, y));
            removed_solid_pixels += usize::from(
                before_grid.density_at(x, y).unwrap_or(0) >= C4M_SOLID
                    && after_grid.density_at(x, y).unwrap_or(0) < C4M_SOLID,
            );
        }
    }
    // FLNT Hit -> Explode(18) -> DoExplosion -> BlastFree clears each
    // material pixel before Blast2Object/PXS casts (Explode.c:10-22,58-65;
    // C4Landscape.cpp:1022-1061). Spawned GOLD without a terrain delta is
    // not a valid physical blast outcome.
    assert!(
        after_grid.revision() > before_grid.revision(),
        "{subject} must invalidate the rendered landscape cache"
    );
    assert!(
        changed_pixels > 0 && removed_solid_pixels > 0,
        "{subject} must change and clear terrain inside its radius (changed={changed_pixels}, removed_solid={removed_solid_pixels})"
    );
}

fn climb_right_out_of_blast_pocket_observing(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    target_x: i32,
    milestone: &str,
    tracked_flint: Option<ObjectId>,
) -> Result<Option<DetonationTransition>, Box<dyn Error>> {
    player.press(COM_RIGHT)?;
    let mut previous_action = String::new();
    let mut detonation = None;
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
        let before_detonation = tracked_flint
            .filter(|_| detonation.is_none())
            .and_then(|flint| {
                player.engine().object_snapshot(flint).and_then(|object| {
                    player
                        .engine()
                        .landscape()
                        .cloned()
                        .map(|landscape| (flint, object.position, landscape))
                })
            });
        player.ticks(1)?;
        if let Some((flint, center, before)) = before_detonation {
            if player.engine().object_snapshot(flint).is_none() {
                let after = player
                    .engine()
                    .landscape()
                    .cloned()
                    .expect("Tutorial07 retains its landscape during FLNT detonation");
                detonation = Some(DetonationTransition {
                    center,
                    before,
                    after,
                });
            }
        }
    }
    player.release(COM_RIGHT)?;
    // Correct seed-zero terrain lets the Clonk clear this pocket before the
    // thrown FLNT completes its physical fall. Keep observing from the safe
    // retreat point so the exact Hit/Explode landscape transition is still
    // captured rather than assuming it happened during horizontal travel.
    if detonation.is_none() {
        if let Some(flint) = tracked_flint {
            for _ in 0..300 {
                let Some(object) = player.engine().object_snapshot(flint) else {
                    break;
                };
                let center = object.position;
                let before = player
                    .engine()
                    .landscape()
                    .cloned()
                    .expect("Tutorial07 retains its landscape before FLNT detonation");
                player.ticks(1)?;
                if player.engine().object_snapshot(flint).is_none() {
                    let after = player
                        .engine()
                        .landscape()
                        .cloned()
                        .expect("Tutorial07 retains its landscape during FLNT detonation");
                    detonation = Some(DetonationTransition {
                        center,
                        before,
                        after,
                    });
                    break;
                }
            }
        }
    }
    player.assert_milestone(milestone, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x >= target_x)
    })?;
    Ok(detonation)
}

fn climb_right_out_of_blast_pocket(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    target_x: i32,
    milestone: &str,
) -> Result<(), Box<dyn Error>> {
    climb_right_out_of_blast_pocket_observing(player, clonk, target_x, milestone, None).map(drop)
}

fn return_to_hut(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
    subject: &str,
    target_wealth: Option<i32>,
) -> Result<(), Box<dyn Error>> {
    climb_right_out_of_blast_pocket(
        player,
        clonk,
        105,
        &format!("{subject} climbs out of the blast pocket"),
    )?;
    player.wait_out_double_click()?;
    let descent_control = player
        .engine()
        .object_snapshot(clonk)
        .zip(player.engine().object_snapshot(elevator_case))
        .map(|(clonk, elevator_case)| {
            if clonk.position.x <= elevator_case.position.x {
                COM_RIGHT
            } else {
                COM_LEFT
            }
        })
        .expect("the Clonk and ELEC survive the blast-pocket descent");
    player.hold_until(
        descent_control,
        format!("{subject} aligns above ELEC"),
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator_case)| {
                    clonk.action.name == "Walk"
                        && (clonk.position.x - elevator_case.position.x).abs() <= 25
                })
        },
    )?;
    // ELEC's C++ FindWaitingClonk accepts this WALK Clonk only after the
    // released horizontal control has set its ComDir to Stop.
    player.wait_until(format!("{subject} descends beside ELEC"), 160, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(clonk, elevator_case)| {
                clonk.action.name == "Walk"
                    && (clonk.position.y - elevator_case.position.y).abs() <= 20
            })
    })?;
    // Crossing the freshly blasted pocket can leave the Clonk on either side
    // of ELEC. Use an ordinary horizontal control to enter its grab rectangle
    // and to separate this Down from the descent Down in C++'s LastCom buffer.
    let align_control = player
        .engine()
        .object_snapshot(clonk)
        .zip(player.engine().object_snapshot(elevator_case))
        .map(|(clonk, elevator_case)| {
            if clonk.position.x < elevator_case.position.x {
                COM_RIGHT
            } else {
                COM_LEFT
            }
        })
        .expect("the Clonk and ELEC survive the HUT3 return");
    player.hold_until(
        align_control,
        format!("{subject} aligns with ELEC"),
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
    player.hold_until(
        COM_DOWN,
        format!("{subject} lands beside ELEC"),
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|clonk| clonk.action.name == "Walk")
        },
    )?;
    let settled_align = player
        .engine()
        .object_snapshot(clonk)
        .zip(player.engine().object_snapshot(elevator_case))
        .map(|(clonk, elevator_case)| {
            if clonk.position.x < elevator_case.position.x {
                COM_RIGHT
            } else {
                COM_LEFT
            }
        })
        .expect("the landed Clonk and ELEC survive the HUT3 return");
    player.hold_until(
        settled_align,
        format!("{subject} settles beside ELEC"),
        60,
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
    player.wait_out_double_click()?;
    // A fresh elevator grab is DownDouble in C++ (C4ObjectCom.cpp:573-589).
    player.double_tap(COM_DOWN)?;
    player.wait_until(format!("{subject} grabs ELEC"), 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    let elevator_start_y = player
        .engine()
        .object_snapshot(elevator_case)
        .expect("ELEC survives before its surface ascent")
        .position
        .y;
    player.hold_until(
        COM_UP,
        format!("ELEC raises {subject} to the surface"),
        360,
        |engine| {
            engine
                .object_snapshot(elevator_case)
                // Wait for SetMoveTo(RangeTop) to halt naturally. The old
                // Clonk y<=205 proxy released Up eight pixels before ELEC's
                // C++ top stop and left the surface bridge open.
                .is_some_and(|object| {
                    object.position.y < elevator_start_y && object.action.name == "Wait"
                })
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
            .expect("the returning Clonk survives the surface lip");
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
    player.assert_milestone(format!("{subject} crosses the surface lip"), |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.position.x <= 70)
    })?;
    player.wait_until(format!("{subject} lands beside HUT3"), 120, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_RIGHT,
        format!("{subject} steps into HUT3's entrance"),
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 62)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until(format!("{subject} enters HUT3"), 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    if let Some(target_wealth) = target_wealth {
        player.wait_until(
            format!("HUT3 auto-sells GOLD to reach wealth {target_wealth}"),
            80,
            |engine| player_wealth(engine, owner) >= target_wealth,
        )?;
    }
    Ok(())
}

fn exit_hut_and_descend_to_gold_seam(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
) -> Result<(), Box<dyn Error>> {
    player.wait_until("HUT3 restores context after selling GOLD", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
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
    player.wait_out_double_click()?;
    player.double_tap(COM_DOWN)?;
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
    Ok(())
}

fn return_from_hut_and_collect_gold(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    elevator_case: ObjectId,
    hut: ObjectId,
    owner: i32,
) -> Result<(), Box<dyn Error>> {
    exit_hut_and_descend_to_gold_seam(player, clonk, elevator_case, hut, owner)?;
    player.hold_until(
        COM_RIGHT,
        "the empty Clonk sweeps the far side of the GOLD seam",
        160,
        |engine| {
            clonk_carries(engine, clonk, "GOLD")
                || engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.x >= 165)
        },
    )?;
    if !clonk_carries(player.engine(), clonk, "GOLD") {
        if player_wealth(player.engine(), owner) < 15 {
            player.hold_until(
                COM_LEFT,
                "the Clonk naturally collects another GOLD chunk",
                180,
                |engine| clonk_carries(engine, clonk, "GOLD"),
            )?;
        } else {
            player.hold_until(
                COM_LEFT,
                "the Clonk returns to the buried GOLD seam",
                100,
                |engine| {
                    clonk_carries(engine, clonk, "GOLD")
                        || engine
                            .object_snapshot(clonk)
                            .is_some_and(|object| object.position.x <= 115)
                },
            )?;
        }
    }
    if !clonk_carries(player.engine(), clonk, "GOLD") {
        player.tap(COM_DIG)?;
        player.wait_until(
            "the Clonk digs into the remaining GOLD seam",
            40,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Dig")
            },
        )?;
        player.press(COM_DOWN)?;
        let collected_gold = player.hold_until(
            COM_LEFT,
            "the Clonk frees and collects another GOLD chunk",
            180,
            |engine| clonk_carries(engine, clonk, "GOLD"),
        );
        player.release(COM_DOWN)?;
        collected_gold?;
    }
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its real Contents menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
    })?;
    let flint_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "FLNT"))
        .expect("Tutorial07 HUT3 contains FLNT");
    player.menu_navigate_to_index(flint_index)?;
    player.tap(COM_SPECIAL2)?;
    player.wait_until("C++ Get keeps one Tutorial07 flint", 120, |engine| {
        clonk_contents_count(engine, clonk, "FLNT") == 1
            && object_contents_count(engine, hut, "FLNT") == 1
    })?;

    player.menu_close()?;
    player.wait_until("HUT3 restores its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
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
        "the Clonk reaches Tutorial07's first GOLD-side blast pocket",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 120 && object.position.y >= 300)
        },
    )?;
    // Script8 points to the GOLD seam at (72,315), and the tutorial gives
    // two real FLNT objects for opening it (Tutorial07.c4s/Script.c:61-78).
    // Vanilla CLNK rejects a second nonspecial object, so the two blasts need
    // two ordinary HUT3/elevator trips (Clonk.c4d/Script.c:738-763).
    let first_flint = player
        .engine()
        .object_snapshot(clonk)
        .and_then(|clonk| {
            clonk.contents.into_iter().find(|item| {
                player
                    .engine()
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "FLNT")
            })
        })
        .expect("the first Tutorial07 FLNT is ready to throw");
    // Releasing ELEC used COM_DOWN double-click handling. Let that window
    // expire so the following COM_THROW remains Throw instead of C++'s
    // intentional down-double Throw-to-Drop conversion.
    player.wait_out_double_click()?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the first FLNT leaves the Clonk's inventory",
        60,
        |engine| clonk_contents_count(engine, clonk, "FLNT") == 0,
    )?;
    let detonation = climb_right_out_of_blast_pocket_observing(
        &mut player,
        clonk,
        120,
        "the Clonk retreats from the first FLNT blast",
        Some(first_flint),
    )?;
    let detonation =
        detonation.expect("the physical retreat observes the first FLNT detonation tick");
    assert!(
        player.engine().object_snapshot(first_flint).is_none(),
        "the first Tutorial07 FLNT detonates during the physical retreat"
    );
    assert_flint_blast_changes_terrain(&detonation, "the first real FLNT blast");
    return_to_hut(
        &mut player,
        clonk,
        elevator_case,
        hut,
        owner,
        "the Clonk after the first FLNT blast",
        None,
    )?;
    player.wait_until("HUT3 opens context for the second FLNT", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 reopens its real Contents menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
    })?;
    let second_flint_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "FLNT"))
        .expect("Tutorial07 HUT3 retains its second FLNT");
    player.menu_navigate_to_index(second_flint_index)?;
    player.menu_enter()?;
    player.wait_until(
        "the Clonk takes the second Tutorial07 flint",
        120,
        |engine| {
            clonk_contents_count(engine, clonk, "FLNT") == 1
                && object_contents_count(engine, hut, "FLNT") == 0
        },
    )?;
    player.menu_close()?;
    exit_hut_and_descend_to_gold_seam(&mut player, clonk, elevator_case, hut, owner)?;
    player.hold_until(
        COM_LEFT,
        "the Clonk reaches Tutorial07's second GOLD-side blast pocket",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x <= 120 && object.position.y >= 300)
        },
    )?;
    let attached = player
        .engine()
        .object_snapshot(clonk)
        .filter(|object| object.action.name == "Hangle" || object.action.name.starts_with("Scale"))
        .map(|object| (object.action.name, object.direction));
    if let Some((action, direction)) = attached {
        let let_go = if action.starts_with("Scale") {
            if direction == Direction::Left {
                COM_RIGHT
            } else {
                COM_LEFT
            }
        } else {
            COM_DOWN
        };
        player.tap(let_go)?;
        player.wait_until(
            "the Clonk lands before the second FLNT throw",
            120,
            |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
            },
        )?;
    }
    player.hold_until(
        COM_LEFT,
        "the Clonk faces the GOLD seam before the second throw",
        30,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && object.direction == Direction::Left
            })
        },
    )?;
    player.wait_out_double_click()?;
    let second_flint = player
        .engine()
        .object_snapshot(clonk)
        .and_then(|clonk| {
            clonk.contents.into_iter().find(|item| {
                player
                    .engine()
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "FLNT")
            })
        })
        .expect("the second Tutorial07 FLNT is ready to throw");
    player.tap(COM_THROW)?;
    player.wait_until(
        "the second FLNT leaves the Clonk's inventory",
        60,
        |engine| {
            engine
                .object_snapshot(second_flint)
                .is_none_or(|flint| flint.container != Some(clonk))
        },
    )?;
    let second_detonation = climb_right_out_of_blast_pocket_observing(
        &mut player,
        clonk,
        120,
        "the Clonk retreats from the second FLNT blast",
        Some(second_flint),
    )?
    .expect("the physical retreat observes the second FLNT detonation tick");
    assert_flint_blast_changes_terrain(&second_detonation, "the second real FLNT blast");
    let seam_offset = Vector2::new(
        second_detonation.center.x - 72,
        second_detonation.center.y - 315,
    );
    assert!(
        seam_offset.x * seam_offset.x + seam_offset.y * seam_offset.y
            <= FLNT_BLAST_RADIUS * FLNT_BLAST_RADIUS,
        "the second physical FLNT blast must intersect Script8's marked GOLD seam at (72,315); center={:?}",
        second_detonation.center
    );
    player.assert_milestone("the two real FLNT blasts expose GOLD objects", |engine| {
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
    return_to_hut(
        &mut player,
        clonk,
        elevator_case,
        hut,
        owner,
        "the GOLD-carrying Clonk",
        None,
    )?;
    // C4Object::ExecLife can spend one sale on the empty base's first 100
    // energy. Let one complete Tick3 interval settle that purchase before
    // deciding whether another physical mining trip is still necessary
    // (C4Object.cpp:814-856).
    for _ in 0..4 {
        player.wait_until("HUT3 restores context after selling GOLD", 30, |engine| {
            object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
        })?;
        player.ticks(3)?;
        if player_wealth(player.engine(), owner) >= 20 {
            break;
        }
        return_from_hut_and_collect_gold(&mut player, clonk, elevator_case, hut, owner)?;
        return_to_hut(
            &mut player,
            clonk,
            elevator_case,
            hut,
            owner,
            "the GOLD-carrying Clonk",
            None,
        )?;
    }
    player.wait_until("HUT3 restores context after the GOLD sales", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.ticks(3)?;
    player.assert_milestone("the physical GOLD sales fund the workshop", |engine| {
        player_wealth(engine, owner) >= 20
    })?;

    let workshop = object_with_definition(player.engine(), "WRKS")
        .expect("Tutorial07 creates the player's workshop");
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Production")?;
    player.menu_enter()?;
    player.wait_until("WRKS opens its real production menu", 30, |engine| {
        object_menu_identification(engine, owner)
            == Some(clonk_script::Value::C4Id("CXCN".to_owned()))
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
                        "BALN"
                            | "WOOD"
                            | "METL"
                            | "HUT3"
                            | "BAS3"
                            | "ELEV"
                            | "ELBS"
                            | "ELEC"
                            | "WRKS"
                            | "BAS7"
                            | "CLNK"
                    )
                })
                .map(|object| {
                    (
                        object.id,
                        object.definition_id,
                        object.position,
                        object.velocity,
                        object.action.name,
                        object.mobile,
                        object.fixed_position,
                        object.fixed_velocity,
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
                object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
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
    // Landscape.bmp's rough crystal shelf rises to about y=130 near x=560.
    // The Push-attached CLNK rides 12px below BALN's origin; rising to y=80
    // leaves roughly 25px clearance after Stop/ClearDir's downward coast.
    player.wait_until(
        "the BALN climbs to the crystal flight level",
        180,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push"
                    && object.action.target == Some(balloon)
                    && object.position.y <= 80
            })
        },
    )?;
    player.release(COM_UP)?;
    player.tap(COM_DOWN)?;
    player.ticks(11)?;
    // At this clearance the scenario wind carries BALN to the objective
    // cliff; x=565 is the physical arrival milestone for its object anchor.
    const CRYSTAL_CLIFF_X: i32 = 565;
    player.wait_until("the BALN reaches the opposite cliff", 900, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push"
                && object.action.target == Some(balloon)
                && object.position.x >= CRYSTAL_CLIFF_X
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
                    && object.position.x >= CRYSTAL_CLIFF_X
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
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.position.y >= 290 || object.action.name == "Walk")
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
    let mut released_hangle = false;
    for _ in 0..160 {
        let action = player
            .engine()
            .object_snapshot(clonk)
            .expect("the crystal-carrying Clonk survives the shaft landing")
            .action
            .name;
        if action == "Walk" {
            break;
        }
        if action == "Hangle" && !released_hangle {
            // Jump'n'Run's exact DFA_HANGLE/COM_Dig arm calls
            // ObjectComLetGo immediately (C4Object.cpp:3635-3640).
            player.tap(COM_DIG)?;
            player.assert_milestone("Hangle/Dig performs C++ ObjectComLetGo", |engine| {
                engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Jump")
            })?;
            released_hangle = true;
        }
        player.ticks(1)?;
    }
    player.assert_milestone(
        "the crystal-carrying Clonk lets go and lands in the elevator shaft",
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    let elevator_align = player
        .engine()
        .object_snapshot(clonk)
        .zip(player.engine().object_snapshot(elevator_case))
        .map(|(clonk, elevator_case)| {
            if clonk.position.x < elevator_case.position.x {
                COM_RIGHT
            } else {
                COM_LEFT
            }
        })
        .expect("the crystal-carrying Clonk and home elevator survive");
    player.hold_until(
        elevator_align,
        "the crystal-carrying Clonk lands beside the home elevator",
        120,
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
    player.wait_until(
        "the home elevator reaches the stopped crystal-carrying Clonk",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(elevator_case))
                .is_some_and(|(clonk, elevator)| {
                    let dx = clonk.position.x - elevator.position.x;
                    let dy = clonk.position.y - elevator.position.y;
                    clonk.action.name == "Walk"
                        && elevator.action.name == "Wait"
                        && engine
                            .object_current_shape_rect(elevator_case)
                            .is_some_and(|shape| shape.contains_offset(dx, dy))
                })
        },
    )?;
    player.wait_out_double_click()?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the crystal-carrying Clonk grabs the home elevator",
        80,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Push" && object.action.target == Some(elevator_case)
            })
        },
    )?;
    let elevator_start_y = player
        .engine()
        .object_snapshot(elevator_case)
        .expect("the home elevator survives before its final ascent")
        .position
        .y;
    player.hold_until(
        COM_UP,
        "the home elevator raises the crystal-carrying Clonk",
        360,
        |engine| {
            engine.object_snapshot(elevator_case).is_some_and(|object| {
                object.position.y < elevator_start_y && object.action.name == "Wait"
            })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the crystal-carrying Clonk releases the home elevator",
        80,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk")
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
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Put")?;
    player.menu_enter()?;
    player.wait_until("context Put transfers CRYS into HUT3", 40, |engine| {
        engine
            .object_snapshot(crystal)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("HUT3 restores context after putting CRYS", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Sell")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its real Sell menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(5))
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
        Engine::is_game_over,
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
