use super::*;

fn joining_player(name: &str) -> JoinPlayerConfig {
    JoinPlayerConfig {
        name: name.to_owned(),
        player_info_id: 0,
        score: 0,
        rounds: 0,
        rounds_won: 0,
        rounds_lost: 0,
        total_playing_time: 0,
        team: None,
        color_dw: 0,
        pref_color: 0,
        pref_position: 0,
        crew: Vec::new(),
        control_style: false,
        auto_context_menu: false,
        startup_player_count: 1,
    }
}

fn register_joining_player(engine: &mut Engine, name: &str) -> i32 {
    engine.register_joining_player(
        &joining_player(name),
        PlayerAtClient::HOST,
        "Local",
        PlayerRuntimeControl::NONE,
        false,
        None,
        None,
        None,
    )
}

#[test]
fn replay_player_info_counter_install_preserves_exact_persisted_value() {
    let mut engine = Engine::new();
    engine
        .register_player(PlayerConfig::new(0, "Existing").with_player_info_id(41))
        .expect("existing player registers");
    assert_eq!(engine.last_player_info_id(), 41);

    engine.set_last_player_info_id(12);
    assert_eq!(engine.last_player_info_id(), 12);
}

fn crew_info(name: &str) -> player_file::CrewInfo {
    player_file::CrewInfo {
        id: "CLNK".to_string(),
        name: name.to_string(),
        ..Default::default()
    }
}

#[test]
fn loaded_crew_info_list_prepends_group_entries_like_cpp() {
    let mut engine = Engine::new();
    let mut config = joining_player("Roster");
    config.crew = vec![
        crew_info("First in group"),
        crew_info("Second in group"),
        crew_info("Third in group"),
    ];

    let player = engine.register_joining_player(
        &config,
        PlayerAtClient::HOST,
        "Local",
        PlayerRuntimeControl::NONE,
        false,
        None,
        None,
        None,
    );
    let state = engine.capture_state();

    assert_eq!(state.crew_info_order[&player], vec![2, 1, 0]);
    assert_eq!(state.crew_info_rosters[&player][0].name, "First in group");
}

#[test]
fn player_list_order_preserves_native_id_reuse_recheck_edges() {
    let mut two_players = Engine::new();
    assert_eq!(register_joining_player(&mut two_players, "Zero"), 0);
    assert_eq!(register_joining_player(&mut two_players, "One"), 1);
    two_players.remove_player(0).expect("remove player zero");
    assert_eq!(register_joining_player(&mut two_players, "Zero again"), 0);
    assert_eq!(two_players.first_player_id(), Some(1));
    assert_eq!(
        two_players.players().map(Player::id).collect::<Vec<_>>(),
        vec![1, 0],
        "public iteration follows C4PlayerList links"
    );
    let snapshot = two_players.snapshot();
    assert_eq!(
        snapshot
            .players
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![1, 0],
        "simulation players follow C4PlayerList links"
    );
    assert_eq!(
        snapshot
            .hud
            .players
            .iter()
            .map(|player| player.owner)
            .collect::<Vec<_>>(),
        vec![1, 0],
        "HUD live-player prefix follows C4PlayerList links"
    );
    assert_eq!(snapshot.hud.local_players, vec![1, 0]);
    assert_eq!(
        two_players.host_world_context().player_ids(),
        &[1, 0],
        "live indexed script natives follow C4PlayerList links"
    );
    assert_eq!(
        host_world_context_from_snapshot(&snapshot).player_ids(),
        &[1, 0],
        "snapshot indexed script natives follow serialized list order"
    );
    assert_eq!(
        two_players
            .capture_state()
            .players
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![1, 0],
        "native's two-node scan reaches the appended player and returns"
    );

    let state = two_players.capture_state();
    let mut restored = Engine::new();
    restored
        .restore_state(&state)
        .expect("restore player order");
    assert_eq!(restored.first_player_id(), Some(1));
    restored.retain_restored_players([0]);
    assert_eq!(restored.first_player_id(), Some(0));

    let mut three_players = Engine::new();
    for number in 0..3 {
        three_players
            .register_player(PlayerConfig::new(number, format!("Player {number}")))
            .expect("register player");
    }
    three_players
        .remove_player(0)
        .expect("remove first of three players");
    three_players
        .register_player(PlayerConfig::new(0, "Zero again"))
        .expect("reuse player zero");
    assert_eq!(three_players.first_player_id(), Some(0));
    assert_eq!(
        three_players
            .capture_state()
            .players
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "the intervening higher number makes native move zero to the head"
    );

    let mut legacy_fixture = Engine::new();
    legacy_fixture
        .players
        .insert(4, PlayerConfig::new(4, "Four").build());
    legacy_fixture
        .players
        .insert(2, PlayerConfig::new(2, "Two").build());
    assert_eq!(legacy_fixture.first_player_id(), Some(2));
    assert_eq!(
        legacy_fixture
            .capture_state()
            .players
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![2, 4],
        "legacy direct-map fixtures append untracked players deterministically"
    );

    let mut partial_ledger_fixture = Engine::new();
    partial_ledger_fixture
        .register_player(PlayerConfig::new(1, "Tracked"))
        .expect("register tracked player");
    partial_ledger_fixture
        .players
        .insert(4, PlayerConfig::new(4, "Direct map addition four").build());
    partial_ledger_fixture
        .players
        .insert(2, PlayerConfig::new(2, "Direct map addition two").build());
    assert_eq!(
        partial_ledger_fixture
            .players()
            .map(Player::id)
            .collect::<Vec<_>>(),
        vec![1, 2, 4],
        "public iteration must not hide an untracked direct-map addition"
    );
    partial_ledger_fixture.players.remove(&4);
    partial_ledger_fixture.players.remove(&1);
    assert_eq!(
        partial_ledger_fixture
            .players()
            .map(Player::id)
            .collect::<Vec<_>>(),
        vec![2],
        "a stale same-length ledger must still expose every live map player"
    );
}

#[test]
fn clear_pointer_callbacks_follow_native_player_list_order() {
    let mut engine = Engine::new();
    engine
        .register_player(PlayerConfig::new(1, "First"))
        .expect("register first player");
    engine
        .register_player(PlayerConfig::new(0, "Appended lower number"))
        .expect("register appended lower-number player");
    assert_eq!(
        engine.players().map(Player::id).collect::<Vec<_>>(),
        vec![1, 0]
    );

    let mut definition = Definition::from_script(
        "PORD",
        "Player-order callback probe",
        r#"#strict 2
static callback_log;

func CrewSelection(bool unselect, bool cursor)
{
    if (!unselect) callback_log = callback_log * 10 + GetOwner() + 1;
    return true;
}

func ResetCallbackLog() { callback_log = 0; return true; }
func ReadCallbackLog() { return callback_log; }
"#,
    )
    .expect("compile callback-order probe");
    definition.set_crew_member(true);
    engine
        .register_definition(definition)
        .expect("register callback-order probe");

    let target = engine
        .spawn_object(
            SpawnConfig::new("PORD")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )
        .expect("spawn shared cursor target");
    let replacement_one = engine
        .spawn_object(
            SpawnConfig::new("PORD")
                .with_owner(1)
                .with_alive(true)
                .with_crew_member(true),
        )
        .expect("spawn player-one replacement");
    let replacement_zero = engine
        .spawn_object(
            SpawnConfig::new("PORD")
                .with_owner(0)
                .with_alive(true)
                .with_crew_member(true),
        )
        .expect("spawn player-zero replacement");
    engine
        .players
        .get_mut(&1)
        .expect("player one remains")
        .set_crew(vec![target, replacement_one]);
    engine
        .players
        .get_mut(&0)
        .expect("player zero remains")
        .set_crew(vec![target, replacement_zero]);
    engine.crew_selection.insert(
        1,
        CrewSelection {
            cursor: Some(target),
        },
    );
    engine.crew_selection.insert(
        0,
        CrewSelection {
            cursor: Some(target),
        },
    );

    let replacement_index = engine
        .find_object_index(replacement_one)
        .expect("replacement remains live");
    engine
        .call_object_function(replacement_index, "ResetCallbackLog", Vec::new())
        .expect("reset callback order");
    engine
        .clear_object_references_for_removal(target)
        .expect("clear shared cursor in player-list order");
    assert_eq!(
        engine
            .call_object_function(replacement_index, "ReadCallbackLog", Vec::new())
            .expect("read callback order"),
        Value::Int(21),
        "player 1 callback must precede player 0 after the native reuse edge"
    );
}

#[test]
fn player_list_remove_snapshots_without_running_player_evaluation() {
    let mut engine = Engine::new();
    engine
        .register_player(
            PlayerConfig::new(7, "Departing")
                .with_player_info_id(41)
                .with_score(123)
                .with_rounds(8, 3, 5)
                .with_total_playing_time(456),
        )
        .expect("player registers");

    let removed = engine.remove_player(7).expect("player removes");
    let removed_state = removed.to_state();
    assert!(!removed_state.evaluated);
    assert_eq!(removed_state.score, 123);
    assert_eq!(removed_state.rounds, 8);
    assert_eq!(removed_state.total_playing_time, 456);
    assert_eq!(engine.round_results.players.len(), 1);
    let result = &engine.round_results.players[0];
    assert_eq!(result.player_info_id, 41);
    assert_eq!(result.total_playing_time, 456);
    assert_eq!(result.score_old, 123);
    assert_eq!(result.score_new, None);
}

#[test]
fn hard_abort_removes_local_then_remote_without_callbacks_or_crew_removal() {
    let mut engine = Engine::new();
    engine.set_teams(vec![TeamInfo::new(1, "One", 0)]);
    for (number, client) in [
        (0, PlayerAtClient::HOST),
        (1, PlayerAtClient::new(7)),
        (2, PlayerAtClient::UNKNOWN),
        (3, PlayerAtClient::UNKNOWN),
        (4, PlayerAtClient::UNKNOWN),
    ] {
        engine
            .register_player(
                PlayerConfig::new(number, format!("Player {number}"))
                    .with_player_info_id(100 + number)
                    .with_team(Some(1))
                    .with_score(10 + number)
                    .with_rounds(7, 3, 4)
                    .with_total_playing_time(20 + number),
            )
            .expect("player registers");
        engine
            .player_mut(number)
            .expect("player remains")
            .set_at_client(client);
    }
    engine
        .player_mut(4)
        .expect("unknown-client player remains")
        .set_script_player(true);

    // A direct legacy fixture supplies the GetInfo()==nullptr case. It
    // is remote in the second pass and must not create a result row.
    let mut no_info = PlayerConfig::new(5, "No info").build();
    no_info.set_at_client(PlayerAtClient::new(8));
    engine.players.insert(5, no_info);
    engine.player_order = vec![3, 1, 4, 2, 0, 5];
    // Explicit LocalControl is authoritative over the InitControl
    // derivation: player 1 is local despite AtClient=7, while player 2
    // is non-local despite being a user at the replay client id.
    engine.set_local_players([3, 1]);

    let mut crew_definition =
        Definition::from_script("CLNK", "Crew", "").expect("crew definition compiles");
    crew_definition.set_crew_member(true);
    engine
        .register_definition(crew_definition)
        .expect("crew definition registers");
    engine
        .register_script_definition("OWND", "Owned", "")
        .expect("owned definition registers");
    engine
        .load_scenario_script_with_convention(
            "AbortCallbacks",
            "#strict 3\n\
                 static RemoveCalls, GameOverCalls;\n\
                 func Initialize() { RemoveCalls = 0; GameOverCalls = 0; }\n\
                 func RemovePlayer() { RemoveCalls = RemoveCalls + 1; }\n\
                 func OnGameOver() { GameOverCalls = GameOverCalls + 1; }\n\
                 func ReadAbortCalls() { return RemoveCalls * 10 + GameOverCalls; }",
            true,
        )
        .expect("callback probe loads");

    let crew = engine
        .spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(3)
                .with_alive(true)
                .with_crew_member(false)
                .with_loaded(true),
        )
        .expect("crew object spawns");
    engine.crew_rosters.insert(3, vec![crew_info("Retained")]);
    engine.crew_info_order.insert(3, vec![0]);
    engine.remember_legacy_object_info(crew, Some("Retained".to_string()));
    engine
        .initialize_scenario_script()
        .expect("crew info attaches and callback counters initialize");
    assert!(engine.crew_object_info(crew).is_some());

    let owned = engine
        .spawn_object(SpawnConfig::new("OWND").with_owner(3))
        .expect("owned object spawns");
    let removed = engine
        .abort_players_without_callbacks(-1)
        .expect("hard abort succeeds");

    assert_eq!(
        removed.iter().map(Player::id).collect::<Vec<_>>(),
        vec![3, 1, 0, 5],
        "RemoveLocal completes before RemoveAtRemoteClient"
    );
    assert!(removed.iter().all(|player| !player.to_state().evaluated));
    assert_eq!(
        engine.players().map(Player::id).collect::<Vec<_>>(),
        vec![4, 2]
    );
    assert!(!engine.is_game_over());
    assert_eq!(
        engine
            .call_scenario_script_value("ReadAbortCalls", &[])
            .expect("callback counter reads"),
        Some(Value::Int(0))
    );

    let crew_index = engine.find_object_index(crew).expect("crew object remains");
    assert!(!engine.objects[crew_index].destroyed);
    assert_eq!(engine.objects[crew_index].state.owner, OWNER_NONE);
    assert!(engine.objects[crew_index].state.crew_member);
    assert_eq!(engine.objects[crew_index].state.info_physical, None);
    assert!(engine.crew_object_info(crew).is_none());
    assert!(!engine.crew_info_links.contains_key(&crew));

    let owned_index = engine
        .find_object_index(owned)
        .expect("owned object remains");
    assert_eq!(engine.objects[owned_index].state.owner, OWNER_NONE);
    assert_ne!(
        engine.objects[owned_index].state.owner, 4,
        "NotifyOwnedObjects must not transfer to the preserved teammate"
    );

    assert_eq!(
        engine
            .round_results
            .players
            .iter()
            .map(|result| result.player_info_id)
            .collect::<Vec<_>>(),
        vec![103, 101, 100]
    );
    assert!(engine.round_results.players.iter().all(|result| {
        result.score_new.is_none() && result.score_old == result.player_info_id - 90
    }));
}

#[test]
fn hard_abort_derives_local_control_when_no_projection_is_installed() {
    let mut engine = Engine::new();
    for number in 0..3 {
        engine
            .register_player(
                PlayerConfig::new(number, format!("Player {number}"))
                    .with_player_info_id(20 + number),
            )
            .expect("player registers");
    }
    engine
        .player_mut(0)
        .expect("user player remains")
        .set_at_client(PlayerAtClient::UNKNOWN);
    engine
        .player_mut(1)
        .expect("script player remains")
        .set_at_client(PlayerAtClient::UNKNOWN);
    engine
        .player_mut(1)
        .expect("script player remains")
        .set_script_player(true);
    engine
        .player_mut(2)
        .expect("remote player remains")
        .set_at_client(PlayerAtClient::new(7));
    engine.player_order = vec![2, 1, 0];

    let removed = engine
        .abort_players_without_callbacks(-1)
        .expect("derived hard abort succeeds");
    assert_eq!(
        removed.iter().map(Player::id).collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(
        engine.players().map(Player::id).collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn initialized_join_player_can_repeat_scenario_and_team_init() {
    let mut engine = Engine::new();
    engine.set_landscape(Landscape::flat(100, 100));
    engine.set_teams(vec![TeamInfo::new(1, "One", 0), TeamInfo::new(2, "Two", 0)]);
    engine
        .load_scenario_script_with_convention(
            "RepeatedScenarioInit",
            "#strict 3\n\
                 static InitCalls;\n\
                 func Initialize() { InitCalls = 0; }\n\
                 func InitializePlayer() { InitCalls = InitCalls + 1; }\n\
                 func RepeatInit(plr, team) { return InitScenarioPlayer(plr, team); }\n\
                 func ReadInitCalls() { return InitCalls; }",
            true,
        )
        .expect("scenario probe loads");
    engine
        .initialize_scenario_script()
        .expect("scenario probe initializes");

    let mut config = joining_player("Repeatable");
    config.player_info_id = 41;
    config.team = Some(1);
    let number = engine
        .join_player(config)
        .expect("initial join succeeds")
        .number();
    assert_eq!(
        engine
            .call_scenario_script_value("RepeatInit", &[Value::Int(number), Value::Int(2)],)
            .expect("script-host repeated init succeeds"),
        Some(Value::Bool(true))
    );
    assert_eq!(engine.player(number).and_then(Player::team), Some(2));
    assert_eq!(engine.pending_player_joins[&number].team, Some(2));
    assert!(engine
        .initialize_scenario_player(number, 1)
        .expect("second repeated init succeeds")
        .is_some());
    assert_eq!(engine.player(number).and_then(Player::team), Some(1));
    assert_eq!(engine.pending_player_joins[&number].team, Some(1));
    assert_eq!(
        engine
            .call_scenario_script_value("ReadInitCalls", &[])
            .expect("init counter reads"),
        Some(Value::Int(3))
    );

    let no_info_number = 9;
    engine.players.insert(
        no_info_number,
        PlayerConfig::new(no_info_number, "No info").build(),
    );
    engine.player_order.push(no_info_number);
    engine
        .pending_player_joins
        .insert(no_info_number, joining_player("No info"));
    assert!(engine
        .initialize_scenario_player(no_info_number, 1)
        .expect("missing C4PlayerInfo is a clean false return")
        .is_none());
}

#[test]
fn control_game_over_request_does_not_remove_a_player() {
    let mut engine = Engine::new();
    engine
        .register_player(PlayerConfig::new(0, "Replay").with_player_info_id(17))
        .expect("player registers");
    engine
        .load_scenario_script_with_convention(
            "ControlGameOver",
            "#strict 3\n\
                 static Calls;\n\
                 func Initialize() { Calls = 0; }\n\
                 func OnGameOver() { Calls = Calls + 1; }\n\
                 func ReadCalls() { return Calls; }",
            true,
        )
        .expect("game-over probe loads");
    engine
        .initialize_scenario_script()
        .expect("game-over probe initializes");

    assert!(engine
        .request_game_over_from_control()
        .expect("first request triggers game over"));
    assert!(!engine
        .request_game_over_from_control()
        .expect("repeat request is idempotent"));
    assert!(engine.player(0).is_some());
    assert!(engine.player(0).unwrap().to_state().won);
    assert_eq!(
        engine
            .call_scenario_script_value("ReadCalls", &[])
            .expect("game-over counter reads"),
        Some(Value::Int(1))
    );
}
