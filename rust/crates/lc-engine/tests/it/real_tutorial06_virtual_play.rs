#[allow(dead_code)]

use std::error::Error;

use lc_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_CURSOR_RIGHT, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT,
    COM_THROW, COM_UP,
};
use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;

fn load_tutorial06() -> (Engine, i32) {
    let mut engine = load_tutorial(6, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 6 virtual player".to_owned(),
            player_info_id: 0,
            score: 0,
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

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine
        .snapshot()
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<lc_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

#[test]
fn tutorial06_virtual_player_completes_the_real_scenario() -> Result<(), Box<dyn Error>> {
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
    player.hold_until(
        COM_RIGHT,
        "the CRYS-carrying CLNK reaches the trapped cavern",
        800,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.position.x >= 160 && object.position.y >= 350)
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
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its real Contents menu", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(18))
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
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
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
    // (Conkit.c4d/Script.c:5-33; C4ObjectCom.cpp:531-539).
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
        object_menu_identification(engine, owner) == Some(lc_script::Value::C4Id("CXCN".to_owned()))
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
    player.wait_until("ELEV finishes and creates ELEC", 3_000, |engine| {
        object_with_definition(engine, "ELEC").is_some()
            && engine
                .object_snapshot(elevator)
                .is_some_and(|object| object.construction == 100_000)
    })?;

    player.wait_until(
        "the automatic Energy command connects ELEV to POWR",
        1_200,
        |engine| object_with_definition(engine, "PWRL").is_some(),
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
    player.tap(COM_THROW)?;
    let power_plant = object_with_definition(player.engine(), "POWR")
        .expect("Tutorial06 creates POWR before play starts");
    player.wait_until("POWR starts burning the thrown COAL", 180, |engine| {
        engine
            .object_snapshot(power_plant)
            .is_some_and(|object| object.action.name == "Burning")
    })?;
    player.wait_until(
        "Tutorial06 asks the builder to drill the elevator shaft",
        300,
        |engine| tutorial_message_contains(engine, "drill an elevator shaft"),
    )?;
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
    player.wait_until("ELEC starts drilling the real shaft", 80, |engine| {
        engine
            .object_snapshot(elevator_case)
            .is_some_and(|object| object.action.name == "Drill")
    })?;
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

    // Continue to the flooded shelf before releasing the case. From this
    // ledge the Clonk is already in contact with the Earth wall, so a live
    // up-left dig can cut a body-sized diagonal passage into the basin.
    player.tap(COM_DIG)?;
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
    player.wait_until("the dry approach reaches the basin wall", 100, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.position.x <= 320)
    })?;
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
    // Dig begins down-left. Left rotates it horizontal; an ignored Up press
    // clears the double-click buffer so the second Left is another ordinary
    // direction step, rotating the live dig to up-left without an 11-frame
    // detour into the basin floor.
    player.tap(COM_LEFT)?;
    player.tap(COM_UP)?;
    player.tap(COM_LEFT)?;
    player.wait_until(
        "the builder pre-clears the dry upper passage",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 296)
        },
    )?;
    player.tap(COM_DOWN)?;
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
    player.tap(COM_DOWN)?;
    player.wait_until(
        "the lower-shelf builder comes to a full stop",
        40,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.velocity.x == 0)
        },
    )?;
    player.tap(COM_LEFT)?;
    player.tap(COM_DOWN)?;

    player.tap(COM_DIG)?;
    player.wait_until("the builder starts the lower basin drain", 40, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.wait_until(
        "the lower drain reaches the dry basin wall",
        100,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 305)
        },
    )?;
    player.tap(COM_DOWN)?;
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
    player.tap(COM_LEFT)?;
    player.wait_until("the lower drain reaches its diagonal turn", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Dig" && object.position.x <= 298)
    })?;
    // Turn the live dig up-left while CNAT_Bottom is still available. A
    // straight horizontal cut leaves a body-blocking Earth lip; C++ stops
    // DFA_DIG as soon as that support is gone (C4Object.cpp:4906-4911).
    player.tap(COM_LEFT)?;
    player.wait_until(
        "the diagonal passage opens the basin drain",
        240,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Swim")
        },
    )?;
    if clonk_carries(player.engine(), builder, "COAL") {
        player.tap(COM_THROW)?;
        player.wait_until("the rescuer drops its incidental COAL", 60, |engine| {
            !clonk_carries(engine, builder, "COAL")
        })?;
    }
    player.hold_until(
        COM_UP,
        "the rescuer rises through the pre-cleared upper passage",
        160,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the rescuer exits the flowing basin drain",
        120,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x >= 310 && object.action.name != "Swim")
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until(
        "the lower outlet drains the upper passage",
        1_200,
        |engine| !engine.debug_landscape_is_liquid(290, 310),
    )?;
    player.wait_until(
        "the rescuer stands above the drained outlet",
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
    player.tap(COM_RIGHT)?;
    player.tap(COM_UP)?;
    player.tap(COM_RIGHT)?;
    player.wait_until(
        "the trapped CLNK reaches the drained basin",
        400,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.action.name == "Swim")
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the CRYS carrier swims through the opened passage",
        240,
        |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.position.x >= 300)
        },
    )?;
    // ELEC blocks the narrow shaft while parked below. Transfer CRYS through
    // ordinary Drop/Collection first, then let the surface Clonk ride it up.
    for _ in 0..2 {
        if !clonk_carries(player.engine(), first_clonk, "CRYS") {
            break;
        }
        let previous_count = player
            .engine()
            .object_snapshot(first_clonk)
            .map_or(0, |object| object.contents.len());
        player.tap(COM_THROW)?;
        player.wait_until("the escaped CLNK drops one carried object", 60, |engine| {
            engine
                .object_snapshot(first_clonk)
                .is_some_and(|object| object.contents.len() < previous_count)
        })?;
        player.ticks(12)?;
    }
    player.assert_milestone("the escaped CLNK releases CRYS to its rescuer", |engine| {
        !clonk_carries(engine, first_clonk, "CRYS")
    })?;
    player.tap(COM_CURSOR_RIGHT)?;
    player.assert_milestone("CursorRight returns to the elevator builder", |engine| {
        engine.crew_cursor(owner) == Some(builder)
    })?;
    player.hold_until(COM_LEFT, "the builder re-enters the drain", 80, |engine| {
        engine
            .object_snapshot(builder)
            .is_some_and(|object| object.action.name == "Swim")
    })?;
    player.hold_until(
        COM_DOWN,
        "the builder dives to the dropped CRYS",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.y >= 325)
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the builder retrieves CRYS from the drain",
        180,
        |engine| clonk_carries(engine, builder, "CRYS"),
    )?;
    player.hold_until(
        COM_UP,
        "the CRYS-carrying builder surfaces by ELEC",
        180,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.action.name == "Walk" && object.position.y <= 316)
        },
    )?;
    player.hold_until(
        COM_RIGHT,
        "the CRYS-carrying builder centers over ELEC",
        80,
        |engine| {
            engine
                .object_snapshot(builder)
                .is_some_and(|object| object.position.x >= 329)
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the CRYS-carrying builder settles on ELEC", 80, |engine| {
        engine.object_snapshot(builder).is_some_and(|object| {
            object.action.name == "Walk" && (327..=333).contains(&object.position.x)
        })
    })?;
    player.ticks(12)?;
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
    player.wait_until("Tutorial06 selects Tutorial07", 320, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial07.c4s"
    })?;
    player.wait_until(
        "Tutorial06 fulfilled goal reaches GameOver",
        320,
        |engine| engine.snapshot().game_over,
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
