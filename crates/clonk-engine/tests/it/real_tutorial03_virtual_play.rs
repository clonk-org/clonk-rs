#![allow(dead_code)]

use std::error::Error;

use crate::support::real_scenario::load_tutorial;
use crate::support::virtual_player::VirtualPlayer;
use clonk_engine::{
    ocf, Engine, JoinPlayerConfig, ObjectId, CATEGORY_VEHICLE, COM_DIG, COM_DOWN, COM_LEFT,
    COM_RIGHT, COM_THROW, COM_UP,
};

fn load_tutorial03() -> (Engine, i32) {
    let mut engine = load_tutorial(3, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 3 virtual player".to_owned(),
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
        .expect("local Tutorial03 virtual player joins")
        .number();
    (engine, owner)
}

fn object_with_definition(engine: &Engine, definition: &str) -> Option<ObjectId> {
    engine.first_object_for_definition(definition)
}

fn object_with_definition_near_x(
    engine: &Engine,
    definition: &str,
    expected_x: i32,
) -> Option<ObjectId> {
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == definition)
        .min_by_key(|object| (object.position.x - expected_x).abs())
        .map(|object| object.id)
}

fn object_menu_identification(engine: &Engine, owner: i32) -> Option<clonk_script::Value> {
    engine
        .cursor_object_menu(owner)
        .map(|(_, menu)| menu.identification.clone())
}

fn tutorial_message_contains(engine: &Engine, needle: &str) -> bool {
    engine.message_line_contains(needle)
}

#[test]
fn tutorial03_virtual_player_completes_the_real_tutorial_route() -> Result<(), Box<dyn Error>> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let (mut engine, owner) = load_tutorial03();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial03 joins one selected CLNK");
    let hut = object_with_definition(&engine, "HUT3").expect("Tutorial03 creates HUT3");
    let sawmill = object_with_definition(&engine, "SAWM").expect("Tutorial03 creates SAWM");
    let foundry = object_with_definition(&engine, "FNDR").expect("Tutorial03 creates FNDR");
    let tree = object_with_definition_near_x(&engine, "TRE2", 167)
        .expect("Tutorial03 saves its first full TRE2 at x=167");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("the ready base and Clonk finish joining", 120, |engine| {
        engine
            .object_snapshot(hut)
            .is_some_and(|object| object.base == owner)
            && engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
    })?;
    player.wait_until("Tutorial03 asks the Clonk to enter HUT3", 240, |engine| {
        tutorial_message_contains(engine, "Enter your homebase")
    })?;

    // HUT3's DefCore entrance begins two pixels right of its center. Under
    // Jump'n'Run controls, Up tries ObjectComUp's entrance path before Jump
    // (Objects.c4d/Structures.c4d/Hut3.c4d/DefCore.txt:18;
    // src/C4ObjectCom.cpp:335-350).
    player.hold_until(
        COM_RIGHT,
        "the Clonk reaches HUT3's entrance",
        20,
        |engine| {
            let hut = engine.object_snapshot(hut);
            let clonk = engine.object_snapshot(clonk);
            hut.zip(clonk)
                .is_some_and(|(hut, clonk)| clonk.position.x >= hut.position.x + 2)
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the Clonk enters HUT3", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container == Some(hut))
    })?;
    player.wait_until("Tutorial03 asks for the Buy menu", 240, |engine| {
        tutorial_message_contains(engine, "Select option 'Buy'")
    })?;

    // Tutorial03 Script91..126 teaches the real permanent-menu sequence
    // C4MN_Context -> C4MN_Buy -> C4MN_Contents and activation of LORY
    // (Tutorial03.c4s/Script.c:106-185; src/C4ObjectMenu.cpp:207-435).
    player.wait_until("HUT3 opens its auto-context menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Buy")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its Buy menu", 10, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(4))
    })?;
    player.wait_until("Tutorial03 asks the player to buy LORY", 240, |engine| {
        tutorial_message_contains(engine, "Buy a lorry")
    })?;
    let buy_lorry_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "LORY"))
        .expect("Tutorial03 home-base material offers LORY");
    player.menu_navigate_to_index(buy_lorry_index)?;
    player.menu_enter()?;
    let lorry = player
        .wait_until("the bought LORY enters HUT3", 20, |engine| {
            object_with_definition(engine, "LORY").is_some_and(|lorry| {
                engine
                    .object_snapshot(lorry)
                    .is_some_and(|object| object.container == Some(hut))
            })
        })
        .map(|_| object_with_definition(player.engine(), "LORY").expect("bought LORY"))?;

    player.wait_until("Tutorial03 asks to close the Buy menu", 240, |engine| {
        tutorial_message_contains(engine, "close the buy menu")
    })?;
    player.menu_close()?;
    player.wait_until("HUT3 restores its context menu", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.wait_until("Tutorial03 asks for HUT3 Contents", 240, |engine| {
        tutorial_message_contains(engine, "select 'Contents'")
    })?;
    player.menu_navigate_to_caption("Contents")?;
    player.menu_enter()?;
    player.wait_until("HUT3 opens its Contents menu", 10, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(18))
    })?;
    player.wait_until(
        "Tutorial03 asks the player to activate LORY",
        240,
        |engine| tutorial_message_contains(engine, "Activate the lorry"),
    )?;
    let contents_lorry_index = player
        .engine()
        .cursor_object_menu(owner)
        .and_then(|(_, menu)| menu.items.iter().position(|item| item.item_id == "LORY"))
        .expect("HUT3 Contents contains the bought LORY");
    player.menu_navigate_to_index(contents_lorry_index)?;
    player.menu_enter()?;
    player.wait_until("Contents activation exits LORY from HUT3", 40, |engine| {
        engine
            .object_snapshot(lorry)
            .is_some_and(|object| object.container.is_none())
    })?;

    player.wait_until("Tutorial03 asks the Clonk to leave HUT3", 240, |engine| {
        tutorial_message_contains(engine, "exit the hut")
    })?;
    player.menu_close()?;
    player.wait_until("HUT3 restores context after activation", 20, |engine| {
        object_menu_identification(engine, owner) == Some(clonk_script::Value::Int(14))
    })?;
    player.menu_navigate_to_caption("Exit")?;
    player.menu_enter()?;
    player.wait_until("the Clonk exits HUT3", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
    })?;

    player.wait_until(
        "Tutorial03 asks for the Lorry to be grabbed",
        180,
        |engine| tutorial_message_contains(engine, "once to grab the lorry"),
    )?;

    // Jump'n'Run Down immediately takes the nearby grab target; while in
    // DFA_PUSH, horizontal controls move both the Clonk and LORY. The script
    // recognizes the cart once its shape overlaps (206,267)
    // (Tutorial03.c4s/Script.c:202-221; src/C4ObjectCom.cpp:247-258).
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs LORY", 40, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    })?;
    player.hold_until(COM_LEFT, "LORY reaches the sawmill chute", 240, |engine| {
        engine.object_snapshot(lorry).is_some_and(|lorry| {
            (194..=218).contains(&lorry.position.x) && (257..=277).contains(&lorry.position.y)
        })
    })?;
    player.wait_until("Tutorial03 asks to release LORY", 180, |engine| {
        tutorial_message_contains(engine, "again to let go of the lorry")
    })?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk releases LORY", 40, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;

    player.wait_until(
        "Tutorial03 asks the Clonk to approach TRE2",
        180,
        |engine| tutorial_message_contains(engine, "first tree on the left"),
    )?;
    player.hold_until(
        COM_LEFT,
        "the Clonk stands inside the first TRE2 shape",
        120,
        |engine| {
            let tree = engine.object_snapshot(tree);
            let clonk = engine.object_snapshot(clonk);
            tree.zip(clonk).is_some_and(|(tree, clonk)| {
                (tree.position.x - 20..=tree.position.x + 20).contains(&clonk.position.x)
                    && (tree.position.y - 28..=tree.position.y + 28).contains(&clonk.position.y)
            })
        },
    )?;
    player.wait_until("Tutorial03 asks for a double Dig at TRE2", 180, |engine| {
        tutorial_message_contains(engine, "twice quickly to start chopping")
    })?;

    // C++ turns the full tree from StaticBack into Vehicle after Chop's
    // repeated damage crosses TreeStrength, at which point the Clonk can grab
    // it (Tree.c4d/Script.c:89-103,116-131; C4Object.cpp:5202-5221).
    let tree_before_chop = player
        .engine()
        .object_snapshot(tree)
        .expect("TRE2 before Chop");
    assert_ne!(
        tree_before_chop.ocf & ocf::CHOP,
        0,
        "the real full TRE2 must expose C++ OCF_Chop at the taught position"
    );
    player.double_tap(COM_DIG)?;
    player.wait_until("the Clonk starts the real Chop action", 80, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Chop")
    })?;
    player.wait_until("TRE2 is chopped into a vehicle", 800, |engine| {
        engine
            .object_snapshot(tree)
            .is_some_and(|object| object.category & CATEGORY_VEHICLE != 0)
    })?;

    player.wait_until(
        "Tutorial03 asks the Clonk to grab the felled tree",
        180,
        |engine| {
            tutorial_message_contains(engine, "grab the felled tree")
                && engine
                    .object_snapshot(clonk)
                    .is_some_and(|object| object.action.name == "Walk")
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs the real felled TRE2", 80, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(tree)
        })
    })?;
    player.wait_until("Tutorial03 asks for TRE2 at SAWM", 180, |engine| {
        tutorial_message_contains(engine, "Push the tree over to the sawmill")
    })?;

    // Script51 uses FindObject(SAWM, GetX(pTree), GetY(pTree)); the taught
    // gate spans the saved SAWM shape around (249,266). Up while pushing
    // invokes ObjectComEnter on TRE2, exactly like the C++ DFA_PUSH control
    // path (Tutorial03.c4s/Script.c:75-94; C4Object.cpp:3506-3536).
    player.hold_until(
        COM_RIGHT,
        "the felled TRE2 center reaches SAWM's entrance",
        240,
        |engine| {
            engine.object_snapshot(tree).is_some_and(|tree| {
                (239..=259).contains(&tree.position.x) && (254..=279).contains(&tree.position.y)
            })
        },
    )?;
    player.wait_until("Tutorial03 asks to push TRE2 into SAWM", 180, |engine| {
        tutorial_message_contains(engine, "press 'up' to push it into the sawmill")
    })?;
    player.tap(COM_UP)?;
    player.wait_until("SAWM consumes the real TRE2", 240, |engine| {
        engine.object_snapshot(tree).is_none()
    })?;
    player.wait_until("SAWM's five WOOD enter the real LORY", 600, |engine| {
        engine
            .snapshot()
            .objects
            .into_iter()
            .filter(|object| object.definition_id == "WOOD" && object.container == Some(lorry))
            .count()
            >= 5
    })?;

    // Script85 physically transfers every sawmill result into LORY
    // (Tutorial03.c4s/Script.c:99-105). C++ stContents insertion keeps the
    // five same-ID objects in one chunk, then DrawIDList's iterator emits one
    // picture stack whose count is five when CanConcatPictureWith succeeds
    // (C4ObjectList.cpp:144-173,343-372,849-903;
    // C4Object.cpp:6173-6213).
    let lorry_after_sawmill = player
        .engine()
        .object_snapshot(lorry)
        .expect("the loaded Tutorial03 LORY remains live");
    let wood_stack = lorry_after_sawmill
        .contents
        .iter()
        .map(|item| {
            player
                .engine()
                .object_snapshot(*item)
                .expect("every LORY content remains live")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        wood_stack
            .iter()
            .map(|item| item.definition_id.as_str())
            .collect::<Vec<_>>(),
        vec!["WOOD"; 5],
        "the real Tutorial03 production flow loads exactly five WOOD objects"
    );
    let representative = &wood_stack[0];
    assert!(
        wood_stack[1..].iter().all(|item| {
            player
                .engine()
                .can_concat_picture_with(representative, item)
                && player
                    .engine()
                    .can_concat_picture_with(item, representative)
        }),
        "all real Tutorial03 WOOD pictures belong to one C++ picture stack"
    );
    let cpp_stack_count = 1 + wood_stack[1..]
        .iter()
        .filter(|item| {
            player
                .engine()
                .can_concat_picture_with(item, representative)
        })
        .count();
    assert_eq!(
        cpp_stack_count, 5,
        "C4ObjectListIterator supplies DrawIDList one five-item WOOD stack"
    );

    player.wait_until("Tutorial03 creates and points to ORE1", 180, |engine| {
        tutorial_message_contains(engine, "dig out the chunk of ore")
            && object_with_definition(engine, "ORE1").is_some()
    })?;
    let ore = object_with_definition(player.engine(), "ORE1").expect("Tutorial03 ORE1");
    player.hold_until(
        COM_RIGHT,
        "the Clonk walks to the ORE1 digging face",
        600,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 480)
        },
    )?;

    // The same real Jump'n'Run sequence used by Tutorial01 starts Dig,
    // then held Down+Right supplies the diagonal tunnel direction. ORE1 is
    // collected only by the normal CrossCheck path; the route never edits
    // landscape, position, or containment state.
    player.tap(COM_DIG)?;
    player.wait_until("the Clonk starts digging toward ORE1", 30, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Dig")
    })?;
    player.press(COM_DOWN)?;
    player.press(COM_RIGHT)?;
    let ore_pickup = player.wait_until("the real dig tunnel collects ORE1", 300, |engine| {
        engine
            .object_snapshot(ore)
            .is_some_and(|object| object.container == Some(clonk))
    });
    player.release(COM_DOWN)?;
    player.release(COM_RIGHT)?;
    ore_pickup?;
    player.wait_until("the ORE1-carrying Clonk finishes digging", 80, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;

    player.wait_until("Tutorial03 asks for ORE1 in LORY", 180, |engine| {
        tutorial_message_contains(engine, "Throw the chunk of ore into the lorry")
    })?;
    player.hold_until(
        COM_LEFT,
        "the ORE1-carrying Clonk returns to LORY's right side",
        800,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| {
                    clonk.position.x >= lorry.position.x + 40
                        && clonk.position.x <= lorry.position.x + 42
                })
        },
    )?;
    player.tap(COM_THROW)?;
    player.wait_until("the real Throw puts ORE1 into LORY", 180, |engine| {
        engine
            .object_snapshot(ore)
            .is_some_and(|object| object.container == Some(lorry))
    })?;

    player.wait_until("Tutorial03 asks for LORY at the foundry", 240, |engine| {
        tutorial_message_contains(
            engine,
            "grab the lorry and push it into the gate of the foundry",
        )
    })?;
    player.wait_until("the Clonk finishes the real Throw action", 80, |engine| {
        engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.action.name == "Walk")
    })?;
    player.hold_until(
        COM_LEFT,
        "the Clonk returns to LORY's grab area",
        160,
        |engine| {
            engine
                .object_snapshot(clonk)
                .zip(engine.object_snapshot(lorry))
                .is_some_and(|(clonk, lorry)| clonk.position.x <= lorry.position.x + 10)
        },
    )?;
    player.tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs the loaded LORY", 60, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    })?;

    // Script315 points to the foundry gate at (368,264). As with the
    // sawmill, Up while pushing runs the real ObjectComEnter path; LORY's
    // Entrance callback then transfers its ore and wood with GrabContents
    // (Tutorial03.c4s/Script.c:263-275; Lorry.c4d/Script.c:82-90).
    player.hold_until(
        COM_RIGHT,
        "the loaded LORY reaches the foundry gate",
        400,
        |engine| {
            engine.object_snapshot(lorry).is_some_and(|lorry| {
                (356..=376).contains(&lorry.position.x) && (253..=279).contains(&lorry.position.y)
            })
        },
    )?;
    player.tap(COM_UP)?;
    player.wait_until("the loaded LORY enters FNDR", 120, |engine| {
        engine
            .object_snapshot(lorry)
            .is_some_and(|object| object.container == Some(foundry))
    })?;
    player.wait_until("Tutorial03 observes LORY inside FNDR", 240, |engine| {
        tutorial_message_contains(engine, "foundry processes ore and fuel into metal")
    })?;
    player.wait_until(
        "FNDR produces real METL from ORE1 and WOOD",
        600,
        |engine| object_with_definition(engine, "METL").is_some(),
    )?;
    player.wait_until("Tutorial03 explains the produced METL", 240, |engine| {
        tutorial_message_contains(engine, "Metal can be used to build vehicles")
    })?;
    player.wait_until("Tutorial03 selects Tutorial04", 240, |engine| {
        engine.next_mission().path == r"Tutorial.c4f\Tutorial04.c4s"
    })?;
    player.wait_until(
        "Tutorial03 fulfilled goal reaches GameOver",
        320,
        Engine::is_game_over,
    )?;
    let completed = player.engine().snapshot();
    assert!(
        completed
            .round_results
            .fulfilled_goals
            .iter()
            .any(|goal| goal == "SCRG"),
        "Tutorial03 must fulfill its real SCRG before selecting Tutorial04"
    );

    let _ = sawmill;
    Ok(())
}
