#[allow(dead_code)]
mod support;

use std::error::Error;

use lc_engine::{
    Engine, JoinPlayerConfig, ObjectId, COM_CURSOR_RIGHT, COM_DIG, COM_DOWN, COM_LEFT, COM_RIGHT,
    COM_UP,
};
use support::real_scenario::load_tutorial;
use support::virtual_player::VirtualPlayer;

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
fn tutorial06_virtual_player_builds_the_real_elevator() -> Result<(), Box<dyn Error>> {
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
    Ok(())
}
