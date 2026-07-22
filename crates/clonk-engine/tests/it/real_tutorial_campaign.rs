use std::env;
use std::path::PathBuf;

use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{
    CommandDirection, Engine, JoinPlayerConfig, Scenario, ScenarioError, COM_DOWN, COM_UP,
};
use clonk_resources::Group;

struct ContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let path = self.root.join(identifier.replace('\\', "/"));
        Group::open(path)
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content"))
}

fn load_tutorial(number: u8) -> (Engine, i32) {
    let content = content_root();
    let path = content.join(format!("Tutorial.c4f/Tutorial{number:02}.c4s"));
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&path, &resolver)
        .unwrap_or_else(|error| panic!("Tutorial{number:02} loads: {error}"));
    let mut engine = Engine::with_seed(0);
    scenario
        .apply(&mut engine)
        .unwrap_or_else(|error| panic!("Tutorial{number:02} applies: {error}"));
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial campaign".to_string(),
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
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .unwrap_or_else(|error| panic!("Tutorial{number:02} player joins: {error}"));
    (engine, joined.number())
}

#[test]
fn tutorial02_ready_balloon_exits_the_first_base() {
    // C4Player::PlaceReadyVehic creates each ready vehicle, enters it into the
    // first base, then immediately replaces its command stack with Exit
    // (C4Player.cpp:619-640, especially :631-636).
    let (mut engine, _) = load_tutorial(2);
    let balloon = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "BALN")
        .expect("Tutorial02 places BALN");
    let hut = engine
        .object_snapshot(balloon.container.expect("BALN starts in the first base"))
        .expect("BALN container exists");
    assert_eq!(hut.definition_id, "HUT3");
    assert_eq!(
        balloon.command_stack.command_names(),
        vec!["Exit".to_string()],
        "PlaceReadyVehic must queue the C++ Exit command"
    );

    for _ in 0..80 {
        if engine
            .object_snapshot(balloon.id)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        engine.tick_without_snapshot().expect("ready vehicle Exit frame");
    }
    assert_eq!(
        engine
            .object_snapshot(balloon.id)
            .expect("BALN survives its Exit")
            .container,
        None,
        "the queued Exit must move BALN out of HUT3"
    );
}

#[test]
fn tutorial02_ready_crew_exits_the_first_base() {
    // C4Player::PlaceReadyCrew enters each newly created crew member into the
    // first base and immediately replaces its command stack with Exit before
    // Recruitment runs (C4Player.cpp:551-564, especially :557-558).
    let (mut engine, player) = load_tutorial(2);
    let clonk = engine
        .crew_cursor(player)
        .expect("Tutorial02 joins one selected CLNK");
    let joined = engine
        .object_snapshot(clonk)
        .expect("Tutorial02 ready crew exists");
    let hut = engine
        .object_snapshot(joined.container.expect("CLNK starts in the first base"))
        .expect("CLNK container exists");
    assert_eq!(hut.definition_id, "HUT3");
    assert_eq!(
        joined.command_stack.command_names(),
        vec!["Exit".to_string()],
        "PlaceReadyCrew must queue the C++ Exit command"
    );

    for _ in 0..80 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        engine.tick_without_snapshot().expect("ready crew Exit frame");
    }
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK survives its Exit")
            .container,
        None,
        "the queued Exit must move the CLNK out of HUT3"
    );
}

#[test]
fn tutorial02_ready_objects_exit_then_the_clonk_controls_the_real_balloon_up() {
    // The first Tutorial02 lesson requires the normal classic-control route:
    // a repeated Down becomes COM_Down_D and queues Grab
    // (C4Player.cpp:1532-1533; C4ObjectCom.cpp:573-588), then the completed
    // Grab switches the CLNK to Push (C4ObjectCom.cpp:247-259). The Push
    // procedure gives the target first refusal on Up
    // (C4Object.cpp:3520-3537); BALN::ControlUp then selects COMD_Up
    // (Objects.c4d/Vehicles.c4d/Balloon.c4d/Script.c:14-29).
    let (mut engine, player) = load_tutorial(2);
    let clonk = engine
        .crew_cursor(player)
        .expect("Tutorial02 joins one selected CLNK");
    let balloon = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "BALN")
        .expect("Tutorial02 places BALN")
        .id;

    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
            && engine
                .object_snapshot(balloon)
                .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        engine.tick_without_snapshot().expect("ready object Exit frame");
    }
    let ready_clonk = engine.object_snapshot(clonk).expect("ready CLNK survives");
    let ready_balloon = engine
        .object_snapshot(balloon)
        .expect("ready BALN survives");
    assert_eq!(ready_clonk.container, None);
    assert_eq!(ready_clonk.action.name, "Walk");
    assert_eq!(ready_balloon.container, None);
    engine
        .player_in_com(player, COM_DOWN, 0)
        .expect("first classic Down press");
    engine
        .player_in_com(player, COM_DOWN, 0)
        .expect("second classic Down press");
    for _ in 0..80 {
        if engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon)
        }) {
            break;
        }
        engine.tick_without_snapshot().expect("Grab command frame");
    }
    let pushing = engine.object_snapshot(clonk).expect("CLNK survives Grab");
    assert_eq!(pushing.action.name, "Push");
    assert_eq!(pushing.action.target, Some(balloon));

    engine
        .player_in_com(player, COM_UP, 0)
        .expect("normal Up while pushing BALN");
    assert_eq!(
        engine
            .object_snapshot(balloon)
            .expect("BALN after ControlUp")
            .command_direction,
        CommandDirection::Up,
        "BALN::ControlUp must complete instead of aborting at GetObjHeight"
    );
}
