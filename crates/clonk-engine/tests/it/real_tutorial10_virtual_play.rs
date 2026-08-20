#![allow(dead_code)]

use std::error::Error;

use crate::support::real_scenario::{
    clonk_carries, load_tutorial_with_local_player, object_with_definition,
    tutorial_message_contains,
};
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::{Engine, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_THROW, COM_UP};

fn load_tutorial10() -> (Engine, i32) {
    load_tutorial_with_local_player(10, 0, "Tutorial 10 virtual player", true, true)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

fn line_connects(engine: &Engine, definition: &str, first: ObjectId, second: ObjectId) -> bool {
    engine.snapshot().objects.into_iter().any(|object| {
        object.definition_id == definition
            && object.action.name == "Connect"
            && object.action.target == Some(first)
            && object.action.target2 == Some(second)
    })
}

fn active_uncontained_object_in_rect(
    engine: &Engine,
    definition: &str,
    x: std::ops::RangeInclusive<i32>,
    y: std::ops::RangeInclusive<i32>,
) -> bool {
    engine.snapshot().objects.into_iter().any(|object| {
        object.definition_id == definition
            && object.status.is_active()
            && object.container.is_none()
            && x.contains(&object.position.x)
            && y.contains(&object.position.y)
    })
}

fn take_from_hut_and_exit(
    player: &mut VirtualPlayer<'_>,
    owner: i32,
    clonk: ObjectId,
    hut: ObjectId,
    definition: &str,
) -> Result<(), Box<dyn Error>> {
    player.wait_until("HUT3 opens its auto-context menu", 400, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its real Contents menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
    })?;
    let item_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| {
            menu.items
                .iter()
                .position(|item| item.item_id == definition)
        })
        .unwrap_or_else(|| panic!("Tutorial10 HUT3 contains {definition}"));
    player.menu_navigate_to_index(item_index)?;
    player.menu_enter()?;
    player.wait_until(
        format!("the Tutorial10 Clonk takes {definition}"),
        60,
        |engine| clonk_carries(engine, clonk, definition),
    )?;
    player.menu_close()?;
    player.wait_until("HUT3 restores its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until(
        format!("the {definition}-carrying Clonk exits HUT3"),
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container != Some(hut))
        },
    )?;
    Ok(())
}

fn enter_hut(
    player: &mut VirtualPlayer<'_>,
    clonk: ObjectId,
    hut: ObjectId,
) -> Result<(), Box<dyn Error>> {
    let x = crate::support::TestValueExt::test_value(player.engine().object_snapshot(clonk))
        .position
        .x;
    if x < 458 {
        player.hold_until(
            COM_RIGHT,
            "the Clonk aligns with HUT3's entrance",
            300,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && (458..=472).contains(&object.position.x)
                })
            },
        )?;
    } else if x > 472 {
        player.hold_until(
            COM_LEFT,
            "the Clonk aligns with HUT3's entrance",
            300,
            |engine| {
                engine.object_snapshot(clonk).is_some_and(|object| {
                    object.action.name == "Walk" && (458..=472).contains(&object.position.x)
                })
            },
        )?;
    }
    player.tap(COM_UP)?;
    player.wait_until("the Clonk enters HUT3", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    Ok(())
}

#[test]
fn tutorial10_virtual_player_completes_the_real_scenario() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial10();
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mut player = VirtualPlayer::new(&mut engine, owner);

    // The scenario starts by moving the ready crew to (500,439), then asks
    // for a DRCK left of POWR (Tutorial10.c4s/Script.c:19-25,88-103).
    player.wait_until("Tutorial10 asks for a derrick", 600, |engine| {
        tutorial_message_contains(engine, "build a derrick")
    })?;
    let hut =
        crate::support::TestValueExt::test_value(object_with_definition(player.engine(), "HUT3"));
    if player
        .engine()
        .object_snapshot(clonk)
        .is_some_and(|object| object.container != Some(hut))
    {
        enter_hut(&mut player, clonk, hut)?;
    }
    take_from_hut_and_exit(&mut player, owner, clonk, hut, "CNKT")?;

    player.hold_until(
        COM_LEFT,
        "the Clonk reaches Tutorial10's DRCK site",
        240,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (260..=285).contains(&object.position.x)
            })
        },
    )?;

    // A real DigDouble activates the carried CNKT, and selecting DRCK in
    // CXCN creates the construction site (C4ObjectCom.cpp:531-540;
    // Tutorial10.c4s/Script.c:97-106).
    player.double_tap(COM_DIG)?;
    player.wait_until("CNKT opens the real CXCN menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::C4Id("CXCN".into()))
    })?;
    let derrick_index = crate::support::TestValueExt::test_value(
        player
            .engine()
            .cursor_object_menu(owner)
            .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "DRCK")),
    );
    player.menu_navigate_to_index(derrick_index)?;
    player.menu_enter()?;
    let derrick = player
        .wait_until("the real DRCK construction site is created", 30, |engine| {
            object_with_definition(engine, "DRCK").is_some()
        })
        .map(|_| {
            crate::support::TestValueExt::test_value(object_with_definition(
                player.engine(),
                "DRCK",
            ))
        })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk starts building DRCK", 30, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Build")
    })?;
    player.wait_until("DRCK construction finishes", 900, |engine| {
        engine
            .object_snapshot(derrick)
            .is_some_and(|object| object.construction == 100_000)
    })?;

    player.wait_until("the DRCK builder returns to Walk", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    enter_hut(&mut player, clonk, hut)?;
    take_from_hut_and_exit(&mut player, owner, clonk, hut, "LNKT")?;
    let power_plant =
        crate::support::TestValueExt::test_value(object_with_definition(player.engine(), "POWR"));
    player.hold_until(
        COM_LEFT,
        "the LNKT-carrying Clonk reaches POWR",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(power_plant))
                .is_some_and(|(clonk, plant)| {
                    clonk.action.name == "Walk" && (clonk.position.x - plant.position.x).abs() <= 5
                })
        },
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("LNKT starts a PWRL at POWR", 60, |engine| {
        engine.snapshot().objects.into_iter().any(|object| {
            object.definition_id == "PWRL"
                && object.action.target == Some(power_plant)
                && object.action.target2.is_some()
        })
    })?;
    player.hold_until(COM_LEFT, "the live PWRL reaches DRCK", 180, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(derrick))
            .is_some_and(|(clonk, derrick)| {
                clonk.action.name == "Walk" && (clonk.position.x - derrick.position.x).abs() <= 8
            })
    })?;
    player.double_tap(COM_DIG)?;
    player.wait_until("PWRL connects POWR to DRCK", 60, |engine| {
        line_connects(engine, "PWRL", power_plant, derrick)
    })?;

    enter_hut(&mut player, clonk, hut)?;
    take_from_hut_and_exit(&mut player, owner, clonk, hut, "LNKT")?;
    player.hold_until(COM_LEFT, "the second LNKT reaches DRCK", 220, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(derrick))
            .is_some_and(|(clonk, derrick)| {
                clonk.action.name == "Walk" && (clonk.position.x - derrick.position.x).abs() <= 8
            })
    })?;
    player.double_tap(COM_DIG)?;
    player.wait_until("LNKT starts a DPIP at DRCK", 60, |engine| {
        engine.snapshot().objects.into_iter().any(|object| {
            object.definition_id == "DPIP"
                && object.action.target == Some(derrick)
                && object.action.target2.is_some()
        })
    })?;
    player.hold_until(COM_RIGHT, "the live DPIP reaches POWR", 180, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(power_plant))
            .is_some_and(|(clonk, plant)| {
                clonk.action.name == "Walk" && (clonk.position.x - plant.position.x).abs() <= 5
            })
    })?;
    player.double_tap(COM_DIG)?;
    player.wait_until("DPIP connects DRCK to POWR", 60, |engine| {
        line_connects(engine, "DPIP", derrick, power_plant)
    })?;

    player.hold_until(COM_LEFT, "the Clonk returns to DRCK", 180, |engine| {
        engine
            .object_snapshot(clonk)
            .zip(engine.object_snapshot(derrick))
            .is_some_and(|(clonk, derrick)| {
                clonk.action.name == "Walk" && (clonk.position.x - derrick.position.x).abs() <= 8
            })
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs DRCK", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(derrick)
        })
    })?;
    player.wait_until(
        "Tutorial10 asks DRCK to drill the oil well",
        500,
        |engine| tutorial_message_contains(engine, "drill a well"),
    )?;

    // DRCK's held Down control creates PIPH and drills until the head enters
    // Script2's oil rectangle (Derrick.c4d/Script.c:55-79;
    // Tutorial10.c4s/Script.c:149-165).
    player.press(COM_DOWN)?;
    let drill = player.wait_until("DRCK drills PIPH into the oil layer", 1_200, |engine| {
        object_with_definition(engine, "PIPH").is_some_and(|pipe_head| {
            engine.object_snapshot(pipe_head).is_some_and(|object| {
                (200..700).contains(&object.position.x) && (585..750).contains(&object.position.y)
            })
        })
    });
    player.release(COM_DOWN)?;
    drill?;
    player.wait_until("released Down stops DRCK's PIPH", 30, |engine| {
        object_with_definition(engine, "PIPH").is_some_and(|pipe_head| {
            engine
                .object_snapshot(pipe_head)
                .is_some_and(|object| object.action.name == "Stop")
        })
    })?;

    player.wait_until("Tutorial10 asks for a pump", 500, |engine| {
        tutorial_message_contains(engine, "build a pump")
    })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk releases DRCK after drilling", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    enter_hut(&mut player, clonk, hut)?;
    take_from_hut_and_exit(&mut player, owner, clonk, hut, "CNKT")?;
    player.hold_until(
        COM_RIGHT,
        "the Clonk reaches Tutorial10's PUMP site",
        180,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk" && (540..=560).contains(&object.position.x)
            })
        },
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("CNKT reopens the real CXCN menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::C4Id("CXCN".into()))
    })?;
    let pump_index = crate::support::TestValueExt::test_value(
        player
            .engine()
            .cursor_object_menu(owner)
            .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "PUMP")),
    );
    player.menu_navigate_to_index(pump_index)?;
    player.menu_enter()?;
    let pump = player
        .wait_until("the real PUMP construction site is created", 30, |engine| {
            object_with_definition(engine, "PUMP").is_some()
        })
        .map(|_| {
            crate::support::TestValueExt::test_value(object_with_definition(
                player.engine(),
                "PUMP",
            ))
        })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk starts building PUMP", 30, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Build")
    })?;
    player.wait_until("PUMP construction finishes", 900, |engine| {
        engine
            .object_snapshot(pump)
            .is_some_and(|object| object.construction == 100_000)
    })?;

    // The StructuresNeedEnergy rule appends C4CMD_Energy when PUMP
    // completes; it must acquire LNKT, join the existing power supply, and
    // finish at the consumer (C4Command.cpp:843-858,2244-2311).
    player.wait_until(
        "C4CMD_Energy connects PUMP to the power supply",
        1_500,
        |engine| {
            engine.snapshot().objects.into_iter().any(|object| {
                object.definition_id == "PWRL"
                    && (object.action.target == Some(pump) || object.action.target2 == Some(pump))
            })
        },
    )?;

    player.wait_until("Tutorial10 asks for another line kit", 500, |engine| {
        tutorial_message_contains(engine, "Get another line construction kit")
    })?;
    enter_hut(&mut player, clonk, hut)?;
    take_from_hut_and_exit(&mut player, owner, clonk, hut, "LNKT")?;
    player.wait_until("Tutorial10 asks for a source pipe", 500, |engine| {
        tutorial_message_contains(engine, "create a source pipe")
    })?;
    player.hold_until(
        COM_RIGHT,
        "the source-pipe LNKT reaches PUMP",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(pump))
                .is_some_and(|(clonk, pump)| {
                    clonk.action.name == "Walk" && (clonk.position.x - pump.position.x).abs() <= 8
                })
        },
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("LNKT starts a SPIP at PUMP", 60, |engine| {
        engine.snapshot().objects.into_iter().any(|object| {
            object.definition_id == "SPIP"
                && object.action.target == Some(pump)
                && object.action.target2.is_some()
        })
    })?;

    player.hold_until(
        COM_RIGHT,
        "the source-pipe LNKT reaches the lava lake",
        360,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| (720..=735).contains(&object.position.x))
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until(
        "the source-pipe LNKT lands in the lava lake",
        240,
        |engine| active_uncontained_object_in_rect(engine, "LNKT", 720..=839, 530..=564),
    )?;

    player.wait_until("Tutorial10 asks for one more line kit", 500, |engine| {
        tutorial_message_contains(engine, "Get another line construction kit")
    })?;
    enter_hut(&mut player, clonk, hut)?;
    take_from_hut_and_exit(&mut player, owner, clonk, hut, "LNKT")?;
    // This late in the scenario the ground immediately right of HUT3's
    // entrance is five pixels higher. Leave the entrance trigger to the
    // left first, then use ordinary Jump'n'Run controls to clear the rise;
    // pressing Up in the doorway would just re-enter HUT3.
    player.hold_until(
        COM_LEFT,
        "the drain-pipe LNKT clears HUT3's entrance trigger",
        60,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 454)
        },
    )?;
    player.press(COM_RIGHT)?;
    player.tap(COM_UP)?;
    let doorway_jump = player.wait_until(
        "the drain-pipe LNKT jumps across HUT3's doorway rise",
        120,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 480)
        },
    );
    player.release(COM_RIGHT)?;
    doorway_jump?;
    player.hold_until(
        COM_RIGHT,
        "the drain-pipe LNKT reaches PUMP",
        180,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(pump))
                .is_some_and(|(clonk, pump)| {
                    clonk.action.name == "Walk" && (clonk.position.x - pump.position.x).abs() <= 8
                })
        },
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("LNKT starts a DPIP at PUMP", 60, |engine| {
        engine.snapshot().objects.into_iter().any(|object| {
            object.definition_id == "DPIP"
                && object.action.target == Some(pump)
                && object.action.target2.is_some()
        })
    })?;
    player.hold_until(
        COM_LEFT,
        "the drain-pipe LNKT reaches the left discharge area",
        300,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| (230..=350).contains(&object.position.x))
        },
    )?;
    // C4Command::Throw reaches ObjectComThrow, which only accepts DFA_WALK
    // (C4Command.cpp:981-983; C4ObjectCom.cpp:625-636). The x-range can be
    // reached while the Clonk is still in Jump/DFA_FLIGHT, so wait for the
    // physical landing before pressing Throw.
    player.wait_until(
        "the drain-pipe LNKT carrier lands before throwing",
        120,
        |engine| {
            engine.object_snapshot(clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && object.position.x <= 399
                    && clonk_carries(engine, clonk, "LNKT")
            })
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until("the drain-pipe LNKT lands left of DRCK", 240, |engine| {
        active_uncontained_object_in_rect(engine, "LNKT", 0..=399, 0..=2_000)
    })?;
    player.wait_until("Tutorial10 begins draining the lava lake", 500, |engine| {
        tutorial_message_contains(engine, "wait for the lava lake to be drained")
    })?;

    // Script5 advances only after the real pump/pipe transfer removes the
    // DuroLava at (770,542) (Tutorial10.c4s/Script.c:232-245).
    player.wait_until("Tutorial10 exposes the crystal", 5_000, |engine| {
        tutorial_message_contains(engine, "crystal is exposed")
    })?;

    let crystal =
        crate::support::TestValueExt::test_value(object_with_definition(player.engine(), "CRYS"));
    player.hold_until(
        COM_RIGHT,
        "the Clonk approaches CRYS from the left",
        420,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| (738..=742).contains(&object.position.x))
        },
    )?;
    // Pump production can leave the Clonk carrying a barrel. CLNK has one
    // ordinary inventory slot, so face away from CRYS and throw the barrel
    // behind the route before approaching the crystal from its left side.
    if !clonk_carries(player.engine(), clonk, "CRYS") {
        let incidental = player
            .engine()
            .object_snapshot(clonk)
            .and_then(|object| object.contents.first().copied());
        if let Some(incidental) = incidental {
            assert_eq!(
                player
                    .engine()
                    .object_snapshot(incidental)
                    .expect("the incidental Tutorial10 content survives")
                    .definition_id,
                "BARL",
                "Tutorial10's pump creates exactly the oil BARL discarded here (Tutorial10/Script.c:166)"
            );
            player.hold_until(
                COM_LEFT,
                "the Clonk faces away from CRYS before discarding the barrel",
                30,
                |engine| {
                    engine
                        .object_snapshot(clonk)
                        .is_some_and(|object| object.position.x <= 734)
                },
            )?;
            player.wait_out_double_click()?;
            player.tap(COM_THROW)?;
            player.wait_until(
                "the Clonk discards the barrel behind the crystal route",
                60,
                |engine| {
                    engine.object_snapshot(clonk).is_some_and(|object| {
                        object.action.name == "Walk" && !object.contents.contains(&incidental)
                    })
                },
            )?;
        }
    }
    if !clonk_carries(player.engine(), clonk, "CRYS") {
        player.hold_until(
            COM_RIGHT,
            "the Clonk approaches from the left and collects CRYS",
            120,
            |engine| clonk_carries(engine, clonk, "CRYS"),
        )?;
    }
    player.assert_milestone("Tutorial10's CRYS is in the Clonk inventory", |engine| {
        engine
            .object_snapshot(crystal)
            .is_some_and(|object| object.container == Some(clonk))
    })?;

    enter_hut(&mut player, clonk, hut)?;
    player.wait_until("HUT3 opens context for the carried CRYS", 60, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Put")?;
    player.menu_enter()?;
    player.wait_until("context Put transfers CRYS into HUT3", 60, |engine| {
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
    let crystal_index = crate::support::TestValueExt::test_value(
        player
            .engine()
            .cursor_object_menu(owner)
            .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "CRYS")),
    );
    player.menu_navigate_to_index(crystal_index)?;
    player.menu_enter()?;
    player.wait_until(
        "selling CRYS removes Tutorial10's objective",
        60,
        |engine| engine.object_snapshot(crystal).is_none(),
    )?;

    player.wait_until(
        "Tutorial10 fulfills SCRG and reaches GameOver",
        600,
        Engine::is_game_over,
    )?;
    player.assert_milestone("Tutorial10 records its fulfilled SCRG goal", |engine| {
        engine
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG")
    })?;
    assert!(
        player.engine().object_snapshot(crystal).is_none(),
        "Tutorial10 must sell CRYS before SCRG can be fulfilled"
    );
    assert_eq!(
        player.engine().next_mission().path,
        r"Tutorial.c4f\Tutorial10.c4s",
        "the final tutorial offers a repeat rather than a nonexistent next tutorial"
    );
    assert_eq!(player.engine().next_mission().text, "&Repeat this round");
    assert_eq!(
        player.engine().next_mission().description,
        "Restart this scenario."
    );
    Ok(())
}
