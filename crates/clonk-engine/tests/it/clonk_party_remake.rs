use crate::support::{join_player_config, real_scenario::load_installed_scenario, TestValueExt};
use clonk_engine::JoinPlayerOutcome;
use clonk_script::Value;

const CLONK_PARTY_REMAKE: &str = "Collection.c4f/Fun.c4f/ClonkPartyRemake8.c4s";

/// Clonk Party replaces every lobby crew while loading the chosen section,
/// then creates and positions one minigame crew per player in `NextGameCom`
/// (ClonkPartyRemake8.c4s/Script.c:181-263). C++ keeps the player list intact
/// while replacing the landscape and object lists (C4Game.cpp:4084-4231).
#[test]
fn clonk_party_remake_moves_every_player_into_sudden_death() {
    let mut engine = load_installed_scenario(CLONK_PARTY_REMAKE, 1);
    let players = ["Host", "Guest 1", "Guest 2", "Guest 3"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let outcome = engine
                .join_player(clonk_engine::JoinPlayerConfig {
                    player_info_id: index as i32 + 1,
                    startup_player_count: 4,
                    ..join_player_config(name)
                })
                .test_value();
            let JoinPlayerOutcome::AwaitingTeamSelection { number } = outcome else {
                panic!("Clonk Party asks player {name} to confirm its teamless slot");
            };
            engine.mark_team_selection_pending(number).test_value();
            engine
                .initialize_scenario_player(number, 0)
                .test_value()
                .test_value();
            number
        })
        .collect::<Vec<_>>();

    engine
        .call_scenario_script_function("d", vec![Value::Int(1)])
        .test_value();
    assert_eq!(engine.debug_current_scenario_section(), "SuddenDeath");
    let landscape = engine.landscape().expect("Sudden Death landscape");
    let section_width = landscape.width() as i32;
    let section_height = landscape.estimated_height();
    let section_snapshot = engine.snapshot();
    for &player in &players {
        let crew = engine.crew_members(player);
        let owned = section_snapshot
            .objects
            .iter()
            .filter(|object| object.owner == player)
            .map(|object| (object.id, object.definition_id.as_str(), object.crew_member))
            .collect::<Vec<_>>();
        assert_eq!(
            crew.len(),
            1,
            "player {player} has one section spectator; raw roster={:?}, owned={owned:?}",
            engine.player(player).map(|state| state.crew())
        );
        let spectator = engine
            .object_snapshot(crew[0])
            .expect("the section spectator is live");
        assert_eq!(spectator.definition_id, "SPCT");
        assert_eq!(spectator.position.x, section_width / 2);
        assert!(
            (0..section_height).contains(&spectator.position.y),
            "player {player}'s spectator is inside the target section"
        );
    }
    engine
        .call_scenario_script_function("NextGameCom", Vec::new())
        .test_value();

    let landscape = engine.landscape().expect("Sudden Death landscape");
    let expected_y = landscape.estimated_height() - 30;
    let landscape_width = landscape.width() as i32;
    let snapshot = engine.snapshot();
    for player in players {
        let crew = engine.crew_members(player);
        let owned = snapshot
            .objects
            .iter()
            .filter(|object| object.owner == player)
            .map(|object| {
                (
                    object.id,
                    object.definition_id.as_str(),
                    object.crew_member,
                    object.position,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            crew.len(),
            1,
            "player {player} has one minigame crew; owned objects: {owned:?}"
        );
        let clonk = engine
            .object_snapshot(crew[0])
            .expect("the minigame crew is live");
        assert_eq!(clonk.definition_id, "NCLN");
        assert_eq!(clonk.position.y, expected_y);
        assert!(
            (70..=landscape_width - 70).contains(&clonk.position.x),
            "player {player} spawned at x={} outside the scripted range",
            clonk.position.x
        );
    }
}
