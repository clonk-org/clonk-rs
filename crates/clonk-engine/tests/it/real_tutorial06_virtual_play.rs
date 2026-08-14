use std::error::Error;

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::{
    Direction, Engine, JoinPlayerConfig, ObjectId, COM_CURSOR_RIGHT, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RIGHT, COM_THROW, COM_UP,
};

fn load_tutorial06() -> (Engine, i32) {
    let mut engine = load_tutorial(6, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 6 virtual player".to_owned(),
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
            auto_context_menu: true,
            startup_player_count: 1,
        })
        .expect("local Tutorial06 virtual player joins")
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

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

#[test]
fn tutorial06_virtual_player_completes_real_scenario_with_autostop_endgame(
) -> Result<(), Box<dyn Error>> {
    let (mut engine, owner) = load_tutorial06();
    let crystal =
        object_with_definition(&engine, "CRYS").expect("Tutorial06 creates the collectible CRYS");
    let first_clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial06 starts with a crew cursor");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    // The scenario first waits for its real CRYS to become contained before
    // advancing (Tutorial06/Script.c:21-32). A C++ crew member collects a
    // carryable object by entering it into Contents, with Collection2,
    // Entrance, and Collection callbacks in that order
    // (C4Object.cpp:1566-1634,5693-5710).
    player.wait_until(
        "Tutorial06 asks the first CLNK to collect CRYS",
        400,
        |engine| tutorial_message_contains(engine, "collect the crystal"),
    )?;
    player.hold_until(
        COM_LEFT,
        "the first CLNK naturally collects the real CRYS",
        800,
        |engine| clonk_carries(engine, first_clonk, "CRYS"),
    )?;
    player.assert_milestone("CRYS enters the first CLNK's inventory", |engine| {
        engine
            .object_snapshot(crystal)
            .is_some_and(|object| object.container == Some(first_clonk))
    })?;
    // Classic controls retain the last procedure direction after key-up;
    // Down changes that lingering Left to Stop without dropping contents
    // (C4Player.cpp:1490-1554; Clonk.c4d/Script.c:175-183).
    player.tap(COM_DOWN)?;
    player.wait_until(
        "Tutorial06 launches its scripted earthquake",
        120,
        |engine| tutorial_message_contains(engine, "area seems to be unstable"),
    )?;
    // ShakeFree visits every pixel in its radius and clears DigFree material
    // in place before creating PXS (C4Landscape.cpp:928-938,999-1010).
    // Tutorial06's (60,150) Earth pixel lies inside ShakeFree(60,160,50).
    player.assert_milestone("Tutorial06 ShakeFree opens its surface pit", |engine| {
        !engine.debug_landscape_is_solid(60, 150)
    })?;
    // The carrier is still tumbling through the new pit when the scripted
    // message appears. Do not arm a horizontal control until it has reached
    // a stable lower-cavern contact: C4Player::InCom retains held controls,
    // so an early Right would become active on a transient Walk frame
    // (C4Player.cpp:1490-1554; C4Object.cpp:3419-3449).
    let mut stable_ticks = 0;
    player.wait_until(
        "the CRYS carrier lands safely in the lower cavern",
        800,
        |engine| {
            let stable = object_with_definition(engine, "FXQ1").is_none()
                && engine.object_snapshot(first_clonk).is_some_and(|object| {
                    object.action.name == "Walk"
                        && object.position.y >= 250
                        && object.velocity.x == 0
                        && object.velocity.y == 0
                        && clonk_carries(engine, first_clonk, "CRYS")
                });
            stable_ticks = if stable { stable_ticks + 1 } else { 0 };
            stable_ticks >= 3
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the CRYS-carrying CLNK reaches the trapped cavern",
        800,
        |engine| {
            engine.object_snapshot(first_clonk).is_some_and(|object| {
                object.action.name == "Walk"
                    && object.position.x >= 160
                    && object.position.y >= 350
                    && clonk_carries(engine, first_clonk, "CRYS")
            })
        },
    )?;

    player.wait_until(
        "Tutorial06 asks the other CLNK to build ELEV",
        2_400,
        |engine| tutorial_message_contains(engine, "With the other clonk"),
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    let builder = player
        .engine()
        .crew_cursor(owner)
        .expect("CursorRight selects Tutorial06's surface CLNK");
    player.assert_milestone("CursorRight leaves the crystal-bearing CLNK", |_| {
        builder != first_clonk
    })?;
    let hut = object_with_definition(player.engine(), "HUT3")
        .expect("Tutorial06 creates the player's HUT3");

    // Jump/Up checks a nearby entrance before jumping, and a base with
    // AutoContextMenu opens C4MN_Context on entry (C4ObjectCom.cpp:335-350;
    // C4Object.cpp:1654-1681; C4Player.cpp:1502-1513).
    player.tap(COM_UP)?;
    player.wait_until("the surface CLNK enters HUT3", 80, |engine| {
        engine
            .object_snapshot(builder)
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
    let linekit = object_with_definition(player.engine(), "LNKT")
        .expect("Tutorial06 creates the line kit in HUT3");
    let linekit_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "LNKT"))
        .expect("Tutorial06 puts LNKT in HUT3");
    player.menu_navigate_to_index(linekit_index)?;
    player.menu_enter()?;
    player.wait_until("the surface CLNK takes LNKT", 80, |engine| {
        engine
            .object_snapshot(linekit)
            .is_some_and(|object| object.container == Some(builder))
    })?;
    player.menu_close()?;
    player.wait_until("HUT3 restores its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the LNKT-carrying CLNK exits HUT3", 80, |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.container.is_none()
                && object.action.name == "Walk"
                && clonk_carries(engine, builder, "LNKT")
        })
    })?;
    player.hold_until(
        COM_RIGHT,
        "the builder carries LNKT to the future elevator site",
        100,
        |engine| {
            engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Walk"
                    && (329..=333).contains(&object.position.x)
                    && clonk_carries(engine, builder, "LNKT")
            })
        },
    )?;
    // Classic movement retains COMD_Right after the physical key is released.
    // Stop in Walk before throwing LNKT east while the route is unobstructed;
    // Throw is rejected outside DFA_WALK (C4Object.cpp:3419-3449;
    // C4Command.cpp:981-984; C4ObjectCom.cpp:625-637).
    player.tap(COM_DOWN)?;
    player.wait_until(
        "the LNKT carrier stops at the future elevator",
        40,
        |engine| {
            engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Walk"
                    && object.velocity.x == 0
                    && object.velocity.y == 0
                    && object.fixed_velocity.is_none()
                    && clonk_carries(engine, builder, "LNKT")
            })
        },
    )?;
    let linekit_launch_x = player
        .engine()
        .object_snapshot(builder)
        .expect("the LNKT carrier remains live")
        .position
        .x;
    player.tap(COM_THROW)?;
    player.wait_until("LNKT settles east of the future elevator", 120, |engine| {
        engine.object_snapshot(linekit).is_some_and(|object| {
            object.container.is_none()
                && object.position.x > linekit_launch_x
                && object.velocity.x == 0
                && object.velocity.y == 0
                && object.fixed_velocity.is_none()
                && engine
                    .object_snapshot(builder)
                    .is_some_and(|builder| builder.action.name == "Walk")
        })
    })?;
    player.hold_until(
        COM_LEFT,
        "the builder returns to HUT3 before constructing ELEV",
        100,
        |engine| {
            engine
                .object_snapshot(builder)
                .zip(engine.object_snapshot(hut))
                .is_some_and(|(builder, hut)| {
                    object_with_definition(engine, "ELEV").is_none()
                        && builder.action.name == "Walk"
                        && builder.position.x <= hut.position.x + 4
                })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the surface CLNK re-enters HUT3", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("HUT3 reopens its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 reopens its real Contents menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
    })?;
    let conkit_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "CNKT"))
        .expect("Tutorial06 puts CNKT in HUT3");
    player.menu_navigate_to_index(conkit_index)?;
    player.menu_enter()?;
    player.wait_until("the surface CLNK takes CNKT", 80, |engine| {
        clonk_carries(engine, builder, "CNKT")
    })?;
    player.menu_close()?;
    player.wait_until("HUT3 restores its context menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the CNKT-carrying CLNK exits HUT3", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
    })?;

    // CNKT::Activate creates the known ELEV construction at the CLNK's
    // feet; double Dig activates the first carried object
    // (Conkit.c4d/Script.c:5-39; C4ObjectCom.cpp:531-539).
    player.hold_until(
        COM_RIGHT,
        "the builder reaches the space between HUT3 and POWR",
        100,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| (329..=333).contains(&object.position.x))
        },
    )?;
    player.double_tap(COM_DIG)?;
    player.wait_until("CNKT opens its ELEV construction menu", 30, |engine| {
        object_menu_identification(engine, owner)
            == Some(clonk_script::Value::C4Id("CXCN".to_owned()))
    })?;
    let elevator_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "ELEV"))
        .expect("Tutorial06's CNKT menu contains ELEV");
    player.menu_navigate_to_index(elevator_index)?;
    player.menu_enter()?;
    let elevator = player
        .wait_until("the real ELEV construction is created", 30, |engine| {
            object_with_definition(engine, "ELEV").is_some()
        })
        .map(|_| object_with_definition(player.engine(), "ELEV").expect("ELEV exists"))?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "ELEV receives every component before LNKT occupies the inventory",
        2_400,
        |engine| {
            engine.object_snapshot(elevator).is_some_and(|object| {
                object.construction < 100_000
                    && object.components.get("WOOD").copied() == Some(4)
                    && object.components.get("METL").copied() == Some(2)
            })
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until(
        "the builder pauses ELEV before fetching LNKT",
        80,
        |engine| {
            engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Walk" && object.command_stack.is_empty()
            })
        },
    )?;

    // A Clonk can carry only one ordinary item. LNKT was staged before ELEV
    // existed, so invest every WOOD/METL first and only then collect it east of
    // the site. Resuming Build cannot put the kit away for another component,
    // and completion queues Energy without the blocked post-ELEV HUT3 trip
    // (Clonk.c4d/Script.c:738-764; Elevator.c4d/DefCore.txt:16;
    // C4Object.cpp:1682-1747,3395-3397,3504-3508;
    // C4Command.cpp:823-861,2246-2311).
    player.hold_until(
        COM_RIGHT,
        "the builder collects the staged LNKT",
        160,
        |engine| {
            engine
                .object_snapshot(linekit)
                .is_some_and(|object| object.container == Some(builder))
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the LNKT-carrying builder returns to ELEV",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .zip(engine.object_snapshot(elevator))
                .is_some_and(|(builder, elevator)| {
                    builder.action.name == "Walk" && builder.position.x <= elevator.position.x + 3
                })
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("ELEV finishes and creates ELEC", 3_000, |engine| {
        object_with_definition(engine, "ELEC").is_some()
            && engine
                .object_snapshot(elevator)
                .is_some_and(|object| object.construction == 100_000)
    })?;

    let power_plant = object_with_definition(player.engine(), "POWR")
        .expect("Tutorial06 creates POWR before play starts");
    // Energy first creates an intermediate POWR-to-LNKT line, then replaces
    // the kit endpoint with ELEV only after the return trip
    // (C4Command.cpp:2273-2311).
    player.wait_until(
        "the automatic Energy command connects ELEV to POWR",
        1_200,
        |engine| {
            engine.snapshot().objects.into_iter().any(|object| {
                object.definition_id == "PWRL"
                    && object.action.name == "Connect"
                    && object.action.target == Some(power_plant)
                    && object.action.target2 == Some(elevator)
            })
        },
    )?;
    player.wait_until("Tutorial06 points at the surface coal", 300, |engine| {
        tutorial_message_contains(engine, "dig out a few pieces of coal")
    })?;
    player.hold_until(
        COM_RIGHT,
        "the surface CLNK reaches the coal seam",
        300,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x >= 440)
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the surface CLNK steps off the coal wall",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 420)
        },
    )?;
    player.tap(COM_RIGHT)?;
    player.tap(COM_DIG)?;
    player.wait_until("the surface CLNK starts digging coal", 30, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.hold_until(
        COM_RIGHT,
        "the real coal seam yields three chunks",
        300,
        |engine| {
            engine
                .snapshot()
                .objects
                .into_iter()
                .filter(|object| object.definition_id == "COAL")
                .count()
                >= 3
        },
    )?;
    player.wait_until("Tutorial06 asks for coal in POWR", 300, |engine| {
        tutorial_message_contains(engine, "Throw the coal chunks")
    })?;
    // Releasing the horizontal key leaves classic DFA_DIG active. The
    // ordinary Down control invokes ObjectComStop before the miner walks
    // back to POWR (C4Object.cpp:3481-3489).
    player.tap(COM_DOWN)?;
    player.wait_until("the coal miner returns to Walk", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.assert_milestone("the coal miner carries one real COAL", |engine| {
        clonk_carries(engine, builder, "COAL")
    })?;
    player.hold_until(
        COM_LEFT,
        "the COAL-carrying CLNK reaches POWR's chute",
        160,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= 424)
        },
    )?;
    player.tap(COM_DOWN)?;
    let thrown_coal = player
        .engine()
        .object_snapshot(builder)
        .and_then(|object| {
            object.contents.into_iter().find(|content| {
                player
                    .engine()
                    .object_snapshot(*content)
                    .is_some_and(|content| content.definition_id == "COAL")
            })
        })
        .expect("the miner carries the coal thrown into POWR");
    player.tap(COM_THROW)?;
    player.wait_until("POWR receives the thrown COAL", 180, |engine| {
        engine
            .object_snapshot(thrown_coal)
            .is_some_and(|object| object.container == Some(power_plant))
    })?;
    let elevator_case = object_with_definition(player.engine(), "ELEC")
        .expect("the completed Tutorial06 elevator creates ELEC");
    player.hold_until(
        COM_LEFT,
        "the builder returns to the elevator shaft",
        180,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= 340)
        },
    )?;
    player.tap(COM_DOWN)?;
    player.ticks(12)?;
    player.wait_until("ELEC returns to the waiting builder", 600, |engine| {
        engine
            .object_snapshot(builder)
            .zip(engine.object_snapshot(elevator_case))
            .is_some_and(|(builder, elevator_case)| {
                elevator_case.action.name == "Wait"
                    && (builder.position.y - elevator_case.position.y).abs() <= 24
            })
    })?;
    player.tap(COM_UP)?;
    player.hold_until(
        COM_LEFT,
        "the builder jumps onto the center of ELEC",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= 331)
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the centered builder lands on ELEC", 80, |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.action.name == "Walk" && (327..=333).contains(&object.position.x)
        })
    })?;
    player.ticks(12)?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the builder grabs ELEC", 100, |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;

    // Classic ELEC drilling is its repeated Dig control. Keeping the second
    // physical press held mirrors ControlDigDouble/ControlDigReleased; the
    // shaft is excavated only while that key remains down
    // (Elevator/Case/Script.c:346-359,612-631).
    player.tap(COM_DIG)?;
    player.press(COM_DIG)?;
    // POWR consumes fuel only while its connected chain requests energy
    // (PowerPlant.c4d/Script.c:63-85). The first physical drill attempt
    // supplies that demand but ELEC rejects it while NoEnergy is true
    // (Elevator/Case/Script.c:346-355,503-508). Wait for Burning to deliver
    // the requested energy, then repeat the ordinary double-Dig.
    player.wait_until("POWR starts burning the thrown COAL", 180, |engine| {
        engine
            .object_snapshot(power_plant)
            .is_some_and(|object| object.action.name == "Burning")
    })?;
    player.wait_until("POWR supplies ELEC's requested energy", 180, |engine| {
        engine
            .object_snapshot(elevator)
            .is_some_and(|object| object.energy >= 12_500)
    })?;
    player.release(COM_DIG)?;
    player.wait_out_double_click()?;
    player.tap(COM_DIG)?;
    player.press(COM_DIG)?;
    player.wait_until("ELEC starts drilling the real shaft", 80, |engine| {
        engine
            .object_snapshot(elevator_case)
            .is_some_and(|object| object.action.name == "Drill")
    })?;
    player.wait_until(
        "Tutorial06 asks the builder to drill the elevator shaft",
        300,
        |engine| tutorial_message_contains(engine, "drill an elevator shaft"),
    )?;
    player.ticks(1)?;
    player.assert_milestone("the centered builder stays on drilling ELEC", |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    let shaft_bottom = player.wait_until(
        "ELEC carries the builder to Tutorial06's lower cavern",
        1_200,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.y >= 300)
        },
    );
    player.release(COM_DIG)?;
    shaft_bottom?;
    player.wait_until("Tutorial06 introduces the flooded passage", 600, |engine| {
        tutorial_message_contains(engine, "get the water out of the way")
    })?;
    assert!(
        player.engine().debug_landscape_is_liquid(220, 220),
        "the default-seed upper-passage probe starts in Water"
    );

    // Isolate the user-facing default Jump'n'Run/AutoStop route at the
    // Tutorial06 endgame checkpoint. Resetting the input ledger models a
    // player loading this live state with no keys held; every subsequent
    // movement still uses physical press/hold/release through InCom.
    player.reset_input_ledger_with_control_style(true)?;

    // Continue to the flooded shelf before releasing the case. From this
    // ledge the Clonk is already in contact with the Earth wall, so a live
    // up-left dig can cut a body-sized diagonal passage into the basin.
    player.press(COM_DIG)?;
    player.wait_until(
        "ELEC resumes drilling below the flooded shelf",
        80,
        |engine| {
            engine
                .object_snapshot(elevator_case)
                .is_some_and(|object| object.action.name == "Drill")
        },
    )?;
    let drainage_shaft =
        player.wait_until("ELEC drills to the drainage-tunnel level", 600, |engine| {
            engine
                .object_snapshot(elevator_case)
                .is_some_and(|object| object.position.y >= 325)
        });
    player.release(COM_DIG)?;
    drainage_shaft?;
    player.double_tap(COM_DOWN)?;
    player.wait_until(
        "the builder releases ELEC in the lower cavern",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;

    player.tap(COM_DIG)?;
    player.wait_until(
        "the builder starts the passage above the water shelf",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig")
        },
    )?;
    player.press(COM_LEFT)?;
    player.press(COM_DOWN)?;
    let dry_approach =
        player.wait_until("the dry approach reaches the basin wall", 100, |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= 320)
        });
    player.release(COM_DOWN)?;
    player.release(COM_LEFT)?;
    dry_approach?;
    player.tap(COM_DOWN)?;
    player.wait_until("the builder stops at the dry basin wall", 40, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.tap(COM_DIG)?;
    player.wait_until(
        "the builder starts the diagonal basin passage",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig")
        },
    )?;
    // AutoStop derives the live dig direction from the physically held
    // direction keys. Hold up-left until the dry upper route is open.
    player.press(COM_LEFT)?;
    player.press(COM_UP)?;
    let upper_passage = player.wait_until(
        "the builder pre-clears the dry upper passage",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 290)
        },
    );
    player.release(COM_UP)?;
    player.release(COM_LEFT)?;
    upper_passage?;
    player.wait_until(
        "the builder stops before breaching the upper passage",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the builder returns from the dry upper passage",
        140,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x >= 327)
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the builder aligns with the lower basin wall",
        100,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x <= 330)
        },
    )?;
    player.wait_until(
        "the lower-shelf builder comes to a full stop",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.velocity.x == 0)
        },
    )?;

    player.tap(COM_DIG)?;
    player.wait_until("the builder starts the lower basin drain", 40, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_LEFT)?;
    player.press(COM_DOWN)?;
    let lower_approach = player.wait_until(
        "the lower drain reaches the dry basin wall",
        100,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= 305)
        },
    );
    player.release(COM_DOWN)?;
    player.release(COM_LEFT)?;
    lower_approach?;
    player.wait_until(
        "the builder stops before opening the lower drain",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.tap(COM_DIG)?;
    player.wait_until("the builder resumes the lower basin drain", 40, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_LEFT)?;
    let diagonal_turn =
        player.wait_until("the lower drain reaches its diagonal turn", 80, |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 298)
        });
    diagonal_turn?;
    // Turn the live dig up-left while CNAT_Bottom is still available. A
    // straight horizontal cut leaves a body-blocking Earth lip; C++ stops
    // DFA_DIG as soon as that support is gone (C4Object.cpp:4906-4911).
    player.press(COM_UP)?;
    let opened_drain = player.wait_until(
        "the diagonal passage opens the basin drain",
        240,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Swim")
        },
    );
    player.release(COM_UP)?;
    player.release(COM_LEFT)?;
    opened_drain?;
    // Water starts draining through the new lower cut as soon as Dig changes
    // to Swim. The earlier physical cut leaves a short vertical connection
    // into the upper passage on the seed-zero landscape.
    player.hold_until(
        COM_UP,
        "the rescuer rises into the pre-cleared upper passage",
        160,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.y <= 316)
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the rescuer exits the flowing basin drain",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x >= 310)
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the rescuer reaches the dry cavern wall",
        180,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x >= 380)
        },
    )?;
    player.hold_until(
        COM_UP,
        "the rescuer rises to the cavern-wall handhold",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.y <= 318)
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the rescuer grabs the dry cavern wall",
        60,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Scale")
        },
    )?;
    player.hold_until(
        COM_UP,
        "the rescuer scales out of the flooded lower passage",
        240,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    // Keep incidental COAL carried until the rescuer has left the drain.
    // Dropping it beside CRYS made the later pickup depend on ELEC's old
    // two-frame MoveTo overshoot: at the C++-exact stop position COAL is
    // collected first and fills the single collection slot.
    if clonk_carries(player.engine(), builder, "COAL") {
        player.tap(COM_THROW)?;
        player.wait_until(
            "the rescuer drops incidental COAL outside the drain",
            60,
            |engine| !clonk_carries(engine, builder, "COAL"),
        )?;
    }
    player.tap(COM_DOWN)?;
    // The default-seed upper-passage probe at (220,220) changes from Water
    // to non-liquid after this physical outlet is opened. This is the
    // behavior required by Script40's instruction to get the water out of
    // the way; C4MassMover::Execute transfers each liquid pixel along
    // FindMatPath (Tutorial06/StringTblUS.txt:8; C4MassMover.cpp:119-158).
    player.wait_until(
        "the lower outlet drains the upper passage",
        1_200,
        |engine| !engine.debug_landscape_is_liquid(220, 220),
    )?;
    player.wait_until(
        "the rescuer stands above the drained passage",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk" && object.velocity.x == 0)
        },
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight selects the CRYS-carrying CLNK", |engine| {
        engine.crew_cursor(owner) == Some(first_clonk)
    })?;
    player.tap(COM_RIGHT)?;
    player.tap(COM_DOWN)?;
    player.tap(COM_DIG)?;
    player.wait_until("the trapped CLNK starts an escape tunnel", 40, |engine| {
        engine
            .object_snapshot(first_clonk)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_RIGHT)?;
    player.press(COM_UP)?;
    let escaped_tunnel = player.wait_until(
        "the trapped CLNK's escape tunnel reaches the lower cavern",
        400,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.action.name == "Walk" && object.position.x >= 200)
        },
    );
    player.release(COM_UP)?;
    player.release(COM_RIGHT)?;
    escaped_tunnel?;
    player.hold_until(
        COM_RIGHT,
        "the escaped CLNK reaches the lower-cavern wall",
        80,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.action.name == "Scale")
        },
    )?;
    player.tap(COM_LEFT)?;
    player.wait_until("the escaped CLNK releases the cavern wall", 60, |engine| {
        engine
            .object_snapshot(first_clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.tap(COM_DIG)?;
    player.wait_until("the escaped CLNK resumes digging east", 40, |engine| {
        engine
            .object_snapshot(first_clonk)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_RIGHT)?;
    player.press(COM_UP)?;
    let reached_basin = player.wait_until(
        "the escaped CLNK reaches the flooded basin",
        400,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.action.name == "Swim")
        },
    );
    player.release(COM_UP)?;
    player.release(COM_RIGHT)?;
    reached_basin?;
    player.hold_until(
        COM_RIGHT,
        "the CRYS carrier reaches the upper passage or its wall",
        120,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.position.x >= 278 || object.action.name == "Scale")
        },
    )?;
    let carrier = player
        .engine()
        .object_snapshot(first_clonk)
        .expect("the CRYS carrier survives the drained basin");
    if carrier.position.x < 278 {
        // Some material-motion layouts attach the swimmer to the x=226 wall.
        // The first opposite Scale edge lets go; pressing that edge again
        // after Scale changes to Jump deliberately recomputes horizontal
        // steering. Hold it until the swimmer moves beyond the five-pixel
        // attachment search, then use UpRight to reach the channel. Launch
        // momentum alone need not survive the transition to Swim. Stop then
        // clears the diagonal swim velocity before Right resumes the crossing
        // (C4Object.cpp:3595-3632,3654-3664,3743-3755,4823-4855,4938-4963;
        // C4ObjectCom.cpp:310-314; C4Physics.h:24-25).
        let wall_x = carrier.position.x;
        let wall_direction = carrier.direction;
        let let_go = if carrier.direction == Direction::Right {
            COM_LEFT
        } else {
            COM_RIGHT
        };
        player.tap(let_go)?;
        player.hold_until(
            let_go,
            "the CRYS carrier releases the basin wall",
            80,
            |engine| {
                engine.object_snapshot(first_clonk).is_some_and(|object| {
                    object.action.name == "Swim"
                        && if wall_direction == Direction::Right {
                            object.position.x <= wall_x - 6
                        } else {
                            object.position.x >= wall_x + 6
                        }
                })
            },
        )?;
        player.press(COM_RIGHT)?;
        let reached_channel = player.hold_until(
            COM_UP,
            "the CRYS carrier returns to the drained transfer channel",
            180,
            |engine| {
                engine
                    .object_snapshot(first_clonk)
                    .is_some_and(|object| object.action.name == "Swim" && object.position.y <= 315)
            },
        );
        player.release(COM_RIGHT)?;
        reached_channel?;
        player.wait_until(
            "the CRYS carrier comes to rest in the transfer channel",
            120,
            |engine| {
                engine.object_snapshot(first_clonk).is_some_and(|object| {
                    object.velocity.x == 0
                        && object.velocity.y == 0
                        && object.fixed_velocity.is_none()
                })
            },
        )?;
        player.hold_until(
            COM_RIGHT,
            "the CRYS carrier reaches or stops beside the upper-passage wall",
            120,
            |engine| {
                engine.object_snapshot(first_clonk).is_some_and(|object| {
                    object.position.x >= 278
                        || (object.position.x >= 270
                            && object.velocity.x == 0
                            && object.fixed_velocity.is_none())
                })
            },
        )?;
        if player
            .engine()
            .object_snapshot(first_clonk)
            .is_some_and(|object| object.position.x < 278)
        {
            player.press(COM_RIGHT)?;
            let reached_wall = player.hold_until(
                COM_DOWN,
                "the CRYS carrier aligns beside the upper-passage wall",
                120,
                |engine| {
                    engine.object_snapshot(first_clonk).is_some_and(|object| {
                        object.position.x >= 278
                            || (object.position.x >= 272
                                && object.velocity.x == 0
                                && object.fixed_velocity.is_none())
                    })
                },
            );
            player.release(COM_RIGHT)?;
            reached_wall?;
        }
    }
    // Current seed-zero terrain leaves a lip between the two Clonks. Bring
    // the empty rescuer alongside the carrier before using the physical
    // Throw-key Drop, so Collection transfers CRYS instead of letting it
    // settle below the lip (C4ObjectCom.cpp:650-671).
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight returns to the elevator builder", |engine| {
        engine.crew_cursor(owner) == Some(builder)
    })?;
    if player
        .engine()
        .object_snapshot(builder)
        .is_some_and(|object| object.action.name == "Scale")
    {
        player.tap(COM_RIGHT)?;
        player.wait_until(
            "the builder releases the eastern cavern wall",
            80,
            |engine| {
                engine
                    .object_snapshot(builder)
                    .is_some_and(|object| object.action.name != "Scale")
            },
        )?;
    }
    player.hold_until(
        COM_LEFT,
        "the builder grabs the eastern drain wall",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Scale")
        },
    )?;
    player.hold_until(
        COM_DOWN,
        "the builder descends beside the drain",
        180,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk" || object.action.name == "Swim")
        },
    )?;
    player.hold_until(COM_LEFT, "the builder re-enters the drain", 100, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Swim")
    })?;
    player.hold_until(
        COM_DOWN,
        "the builder dives to the CRYS transfer depth",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.y >= 325)
        },
    )?;
    player.hold_until(
        COM_LEFT,
        "the builder reaches the CRYS transfer point or its wall",
        180,
        |engine| {
            engine.object_snapshot(builder).is_some_and(|object| {
                object.position.x <= 280
                    || (object.position.x <= 283 && object.action.name == "Scale")
                    || (object.position.x <= 283
                        && matches!(object.action.name.as_str(), "Walk" | "Swim")
                        && object.velocity.x == 0
                        && object.velocity.y == 0
                        && object.fixed_velocity.is_none())
            })
        },
    )?;
    if player
        .engine()
        .object_snapshot(builder)
        .is_some_and(|object| object.action.name == "Scale")
    {
        // The drained layout can attach at x=282, two pixels east of the old
        // incidental waypoint. Descend to a stable Walk/Swim contact there;
        // the following real Collection is the transfer acceptance
        // (C4Object.cpp:4823-4855).
        player.hold_until(
            COM_DOWN,
            "the builder descends from the CRYS transfer wall",
            100,
            |engine| {
                engine.object_snapshot(builder).is_some_and(|object| {
                    matches!(object.action.name.as_str(), "Walk" | "Swim")
                        && object.position.x <= 283
                        && object.velocity.x == 0
                        && object.velocity.y == 0
                        && object.fixed_velocity.is_none()
                })
            },
        )?;
    }
    player.wait_until(
        "the builder steadies at the CRYS transfer point",
        80,
        |engine| {
            engine.object_snapshot(builder).is_some_and(|object| {
                matches!(object.action.name.as_str(), "Walk" | "Swim")
                    && object.position.x <= 283
                    && object.velocity.x == 0
                    && object.velocity.y == 0
                    && object.fixed_velocity.is_none()
            })
        },
    )?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight returns to the CRYS carrier", |engine| {
        engine.crew_cursor(owner) == Some(first_clonk)
    })?;
    // Drop at rest so ObjectComDrop uses no directional throw force. The
    // nearby builder can collect CRYS without it flying past the transfer
    // point (C4ObjectCom.cpp:640-671).
    player.tap(COM_THROW)?;
    player.wait_until(
        "the escaped CLNK drops CRYS beside its rescuer",
        60,
        |engine| !clonk_carries(engine, first_clonk, "CRYS"),
    )?;
    player.wait_until("the builder collects the transferred CRYS", 80, |engine| {
        clonk_carries(engine, builder, "CRYS")
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight selects the CRYS-carrying builder", |engine| {
        engine.crew_cursor(owner) == Some(builder)
    })?;
    let elevator_x = player
        .engine()
        .object_snapshot(elevator_case)
        .expect("Tutorial06 keeps ELEC")
        .position
        .x;
    // A submerged bottom contact cannot turn Swim into Walk. Climb out at the
    // eastern wall, then launch a real UpLeft jump across its lip and release
    // directly over ELEC so the physical fall lands on its solid mask
    // (C4Object.cpp:3743-3755,4332-4379,4823-4855,4967-4974;
    // C4ObjectCom.cpp:220-235,280-307).
    player.hold_until(
        COM_RIGHT,
        "the CRYS-carrying builder reaches the eastern pool wall",
        160,
        |engine| {
            engine.object_snapshot(builder).is_some_and(|object| {
                object.action.name == "Scale" && object.position.x > elevator_x
            })
        },
    )?;
    player.hold_until(
        COM_UP,
        "the CRYS-carrying builder climbs out at the eastern pool wall",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk" && !object.in_liquid)
        },
    )?;
    player.press(COM_LEFT)?;
    let crossed_lip = player.hold_until(
        COM_UP,
        "the CRYS-carrying builder jumps back over ELEC",
        160,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= elevator_x + 3)
        },
    );
    player.release(COM_LEFT)?;
    crossed_lip?;
    player.wait_until("the CRYS carrier lands on ELEC", 120, |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.action.name == "Walk"
                && ((elevator_x - 3)..=(elevator_x + 3)).contains(&object.position.x)
                && object.velocity.x == 0
                && object.velocity.y == 0
                && object.fixed_velocity.is_none()
        })
    })?;
    player.wait_out_double_click()?;
    // Walk Down runs ObjectComDownDouble and grabs the nearby ELEC in C++
    // (C4Object.cpp:3582-3586; C4ObjectCom.cpp:573-589).
    player.double_tap(COM_DOWN)?;
    player.wait_until("the CRYS-carrying builder grabs ELEC", 100, |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.hold_until(
        COM_UP,
        "ELEC carries CRYS back to the surface",
        600,
        |engine| {
            engine
                .object_snapshot(elevator_case)
                .is_some_and(|object| object.position.y <= 160)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the CRYS-carrying builder releases ELEC", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    let hut_x = player
        .engine()
        .object_snapshot(hut)
        .expect("Tutorial06 keeps HUT3")
        .position
        .x;
    player.hold_until(
        COM_LEFT,
        "the CRYS-carrying builder returns to HUT3",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x <= hut_x + 4)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the CRYS-carrying builder enters HUT3", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("Tutorial06 asks the player to sell CRYS", 240, |engine| {
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
    player.wait_until("Tutorial06 selects Tutorial07", 320, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial07.c4s"
    })?;
    player.wait_until(
        "Tutorial06 fulfilled goal reaches GameOver",
        320,
        Engine::is_game_over,
    )?;
    player.assert_milestone("Tutorial06 records its fulfilled SCRG goal", |engine| {
        engine
            .snapshot()
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG")
    })?;
    assert!(
        player.engine().object_snapshot(crystal).is_none(),
        "Tutorial06's CRYS must be sold before SCRG is fulfilled"
    );
    Ok(())
}
