use std::env;
use std::path::PathBuf;

use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{
    CommandDirection, Engine, JoinPlayerConfig, Scenario, ScenarioError, COM_DOWN, COM_UP,
};
use lc_resources::Group;

struct ContentResolver {
    root: PathBuf,
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        Group::open(self.root.join(identifier.replace('\\', "/")))
            .map(|group| vec![group])
            .map_err(ScenarioError::Resources)
    }
}

fn load_tutorial02() -> (Engine, i32) {
    let content = env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../content"));
    let scenario = Scenario::load_from_path_with(
        content.join("Tutorial.c4f/Tutorial02.c4s"),
        &ContentResolver {
            root: content.clone(),
        },
    )
    .unwrap_or_else(|error| panic!("Tutorial02 loads: {error}"));
    let mut engine = Engine::with_seed(0);
    scenario
        .apply(&mut engine)
        .unwrap_or_else(|error| panic!("Tutorial02 applies: {error}"));
    let joined = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 2 balloon platform".to_string(),
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
        .unwrap_or_else(|error| panic!("Tutorial02 player joins: {error}"));
    (engine, joined.number)
}

#[test]
fn tutorial02_balloon_lift_keeps_the_pushing_clonk_on_its_platform() {
    let (mut engine, player) = load_tutorial02();
    let clonk = engine
        .crew_cursor(player)
        .expect("Tutorial02 joins one selected CLNK");
    let balloon = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "BALN")
        .expect("Tutorial02 places BALN");

    // C4Player::PlaceReadyCrew/PlaceReadyVehic put both objects into the
    // first base and replace their command stacks with Exit before play
    // begins (C4Player.cpp:551-564,619-640).
    let clonk_at_spawn = engine.object_snapshot(clonk).expect("spawned CLNK");
    assert_eq!(clonk_at_spawn.container, balloon.container);
    assert!(clonk_at_spawn.container.is_some());
    assert_eq!(clonk_at_spawn.command_stack.command_names(), ["Exit"]);
    assert_eq!(balloon.command_stack.command_names(), ["Exit"]);

    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
            && engine
                .object_snapshot(balloon.id)
                .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        engine.tick().expect("startup Exit frame");
    }
    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("exited CLNK")
            .container,
        None
    );
    assert_eq!(
        engine
            .object_snapshot(balloon.id)
            .expect("exited BALN")
            .container,
        None
    );

    // A repeated classic Down becomes COM_Down_D and queues Grab; the
    // completed Grab enters DFA_PUSH (C4Player.cpp:1522-1536;
    // C4ObjectCom.cpp:247-259,573-588).
    engine
        .player_in_com(player, COM_DOWN, 0)
        .expect("first Down press");
    engine
        .player_in_com(player, COM_DOWN, 0)
        .expect("second Down press");
    for _ in 0..80 {
        if engine.object_snapshot(clonk).is_some_and(|object| {
            object.action.name == "Push" && object.action.target == Some(balloon.id)
        }) {
            break;
        }
        engine.tick().expect("Grab command frame");
    }
    let pushing = engine.object_snapshot(clonk).expect("CLNK after Grab");
    let balloon_before_lift = engine.object_snapshot(balloon.id).expect("BALN after Grab");
    assert_eq!(pushing.action.name, "Push");
    assert_eq!(pushing.action.target, Some(balloon.id));
    let platform_delta_y = pushing.position.y - balloon_before_lift.position.y;

    // While DFA_PUSH is active, the target receives Up first
    // (C4Object.cpp:3520-3537). BALN::ControlUp selects COMD_Up, and its
    // Float procedure accelerates upward. DFA_PUSH follows the target and
    // sets CNAT_Bottom (C4Object.cpp:5058-5114). When BALN's DoMotion removes
    // its solid mask, C++ backs up every object contacting that mask, then
    // Put moves each one by BALN's dx/dy before its own attachment pass
    // (C4Movement.cpp:121-126,443-445; C4SolidMask.cpp:178-195,276-305).
    engine
        .player_in_com(player, COM_UP, 0)
        .expect("Up while pushing BALN");
    assert_eq!(
        engine
            .object_snapshot(balloon.id)
            .expect("controlled BALN")
            .command_direction,
        CommandDirection::Up
    );

    for lift_frame in 1..=60 {
        engine.tick().expect("controlled BALN lift frame");
        let clonk_now = engine.object_snapshot(clonk).expect("CLNK during lift");
        let balloon_now = engine
            .object_snapshot(balloon.id)
            .expect("BALN during lift");
        assert_eq!(
            (clonk_now.action.name.as_str(), clonk_now.action.target),
            ("Push", Some(balloon.id)),
            "DFA_PUSH must retain the moving BALN on lift frame {lift_frame}; \
             clonk={clonk_now:?}, balloon={balloon_now:?}"
        );
        assert!(
            (clonk_now.position.y - balloon_now.position.y - platform_delta_y).abs() <= 1,
            "CLNK must remain vertically attached to BALN's platform on lift frame \
             {lift_frame}; initial delta={platform_delta_y}, clonk={clonk_now:?}, \
             balloon={balloon_now:?}"
        );
    }

    let balloon_after_lift = engine.object_snapshot(balloon.id).expect("lifted BALN");
    assert!(
        balloon_after_lift.position.y < balloon_before_lift.position.y,
        "BALN::ControlUp must move the platform upward"
    );
}
