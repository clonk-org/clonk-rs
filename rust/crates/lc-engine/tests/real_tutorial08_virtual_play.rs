#[allow(dead_code)]
mod support;

use std::error::Error;

use lc_engine::{Engine, JoinPlayerConfig, ObjectId, COM_DOWN, COM_RIGHT};
use support::real_scenario::load_tutorial;
use support::virtual_player::VirtualPlayer;

fn load_tutorial08() -> (Engine, i32) {
    let mut engine = load_tutorial(8, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 8 virtual player".to_owned(),
            player_info_id: 0,
            score: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("local Tutorial08 virtual player joins")
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

fn contained_wipfs(engine: &Engine, container: ObjectId) -> usize {
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "WIPF" && object.container == Some(container))
        .count()
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
fn tutorial08_virtual_player_catches_wipfs_with_real_lorry_controls(
) -> Result<(), Box<dyn Error>> {
    let (mut engine, owner) = load_tutorial08();
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial08 joins one selected CLNK");
    let lorry = object_with_definition(&engine, "LORY").expect("Tutorial08 creates LORY");
    let mut player = VirtualPlayer::new(&mut engine, owner);

    player.wait_until("Tutorial08 teaches catching WIPFs", 500, |engine| {
        tutorial_message_contains(engine, "catch them either by hand or with the lorry")
    })?;

    // C++ creates HUT3's BAS4 during HUT3::Construction while the hut is
    // still at its raw Con=0 position. This route first proves the joined
    // CLNK is not embedded in that basement, then uses only normal classic
    // controls to grab and drive the real LORY through a WIPF.
    player.hold_until(
        COM_RIGHT,
        "the Clonk reaches LORY's initial grab area",
        40,
        |engine| {
            engine
                .object_snapshot(clonk)
                .is_some_and(|object| object.position.x >= 710)
        },
    )?;
    player.double_tap(COM_DOWN)?;
    player.wait_until("the Clonk grabs LORY", 80, |engine| {
        engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(lorry)
        })
    })?;
    player.hold_until(
        COM_RIGHT,
        "LORY reaches the east surface edge",
        900,
        |engine| {
            engine
                .object_snapshot(lorry)
                .is_some_and(|object| object.position.x >= 782)
        },
    )?;
    player.wait_until("LORY catches a real WIPF", 80, |engine| {
        contained_wipfs(engine, lorry) > 0
    })?;

    Ok(())
}
