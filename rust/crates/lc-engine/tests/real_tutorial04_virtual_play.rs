#[allow(dead_code)]
mod support;

use std::error::Error;

use lc_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT, COM_UP,
};
use support::real_scenario::load_tutorial;
use support::virtual_player::VirtualPlayer;

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

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine
        .snapshot()
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
}

#[test]
fn tutorial04_virtual_player_reaches_the_real_gold_tunnel() -> Result<(), Box<dyn Error>> {
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
            && engine.object_snapshot(clonk).is_some_and(|object| {
                object.container.is_none() && object.action.name == "Walk"
            })
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
    player.wait_until("HUT2 restores its context menu after taking CNKT", 30, |engine| {
        object_menu_identification(engine, owner) == Some(lc_script::Value::Int(14))
    })?;
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
    player.wait_until("Tutorial04 asks for an ELEV construction site", 240, |engine| {
        tutorial_message_contains(engine, "Create an elevator construction site")
    })?;
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
    player.wait_until("ELEV finishes and creates ELEC", 720, |engine| {
        object_with_definition(engine, "ELEC").is_some()
            && engine
                .object_snapshot(elevator)
                .is_some_and(|object| object.construction == 100_000)
    })?;
    let elevator_case =
        object_with_definition(player.engine(), "ELEC").expect("ELEV creates ELEC");
    player.wait_until("Tutorial04 asks the Clonk to grab ELEC", 240, |engine| {
        tutorial_message_contains(engine, "Grab the elevator case")
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs ELEC", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(elevator_case)
        })
    })?;
    player.wait_until("Tutorial04 asks the Clonk to drill the shaft", 240, |engine| {
        tutorial_message_contains(engine, "Hold down the 'dig' key")
    })?;
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
    player.wait_until("Tutorial04 asks the Clonk to let go of ELEC", 240, |engine| {
        tutorial_message_contains(engine, "Let go of the elevator case")
    })?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk lets go of ELEC", 60, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name != "Push")
    })?;
    player.wait_until("Tutorial04 sends the Clonk to collect TFLN", 240, |engine| {
        tutorial_message_contains(engine, "Walk back to the cabin")
            && object_with_definition(engine, "TFLN").is_some()
    })?;
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
    player.assert_milestone("the Clonk naturally collects TFLN before its fuse expires", |engine| {
        clonk_carries(engine, clonk, "TFLN")
    })?;
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
    player.wait_until("Tutorial04 sends the TFLN-carrying Clonk down", 240, |engine| {
        tutorial_message_contains(engine, "Ride back down into the mine")
            && clonk_carries(engine, clonk, "TFLN")
    })?;
    player.hold_until(
        COM_LEFT,
        "the TFLN-carrying Clonk returns to ELEC",
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

    Ok(())
}
