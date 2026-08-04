use super::*;

fn register_player(engine: &mut Engine, player: i32, by_client: i32) {
    engine
        .register_player(PlayerConfig::new(player, format!("Player {player}")))
        .expect("player registers");
    engine
        .player_mut(player)
        .expect("player remains registered")
        .set_at_client(PlayerAtClient::new(by_client));
}

#[test]
fn internal_player_controls_apply_exact_author_and_host_gates() {
    let mut engine = Engine::new();
    register_player(&mut engine, 1, 7);
    register_player(&mut engine, 2, 9);
    engine.set_teams(vec![
        TeamInfo::new(4, "Four", 0x44),
        TeamInfo::new(5, "Five", 0x55),
    ]);

    let hostility = ToggleHostilityControlData {
        opponent: 2,
        player: 1,
        by_client: 7,
    };
    assert!(engine
        .execute_toggle_hostility_control(&hostility)
        .expect("toggle executes"));
    assert!(engine.player(1).unwrap().is_hostile_towards(2));
    assert!(engine
        .execute_toggle_hostility_control(&hostility)
        .expect("second toggle executes"));
    assert!(!engine.player(1).unwrap().is_hostile_towards(2));

    let spoofed = ToggleHostilityControlData {
        by_client: 8,
        ..hostility
    };
    assert!(!engine
        .execute_toggle_hostility_control(&spoofed)
        .expect("spoof is a synchronized no-op"));
    assert!(!engine.player(1).unwrap().is_hostile_towards(2));

    assert!(engine
        .execute_set_player_team_control(&SetPlayerTeamControlData {
            team: 4,
            player: 1,
            by_client: 7,
        })
        .expect("team packet executes"));
    assert_eq!(engine.player(1).unwrap().team(), Some(4));

    assert!(engine
        .execute_set_player_team_control(&SetPlayerTeamControlData {
            team: 99,
            player: 1,
            by_client: 7,
        })
        .expect("invalid team is rejected inside the host function"));
    assert_eq!(engine.player(1).unwrap().team(), Some(4));

    engine.set_league_game(true);
    assert!(engine
        .execute_set_player_team_control(&SetPlayerTeamControlData {
            team: 5,
            player: 1,
            by_client: 7,
        })
        .expect("league rejection is not a packet failure"));
    assert_eq!(engine.player(1).unwrap().team(), Some(4));

    assert!(!engine
        .execute_eliminate_player_control(&EliminatePlayerControlData {
            player: 1,
            by_client: 7,
        })
        .expect("non-host elimination is a no-op"));
    assert_eq!(engine.player(1).unwrap().status(), PlayerStatus::Active);

    assert!(engine
        .execute_eliminate_player_control(&EliminatePlayerControlData {
            player: 2,
            by_client: 0,
        })
        .expect("host may eliminate a remote-owned player"));
    assert_eq!(engine.player(2).unwrap().status(), PlayerStatus::Eliminated);
}

#[test]
fn admitted_joined_player_team_update_applies_add_player_side_effects_only() {
    let old_color = RgbColor::new(0x12, 0x34, 0x56);
    let new_color = 0xff00_c800;
    let mut engine = Engine::new();
    engine.set_teams(vec![
        TeamInfo::new(1, "One", 0x0012_3456).with_player_ids(vec![41]),
        TeamInfo::new(2, "Two", new_color),
    ]);
    engine
        .register_player(
            PlayerConfig::new(4, "Joined")
                .with_player_info_id(41)
                .with_team(Some(1))
                .with_color(Some(old_color)),
        )
        .expect("joined player registers");
    engine
        .register_script_definition("OWND", "Owned", "")
        .expect("owned-object definition registers");
    let owned = engine
        .spawn_object(
            SpawnConfig::new("OWND")
                .with_owner(4)
                .with_color(0xaa12_3456),
        )
        .expect("owned object spawns");

    assert!(engine
        .apply_admitted_player_team_update(41, 2, Some(new_color))
        .expect("live AddPlayer update applies"));

    assert_eq!(engine.player(4).unwrap().team(), Some(2));
    assert_eq!(
        engine.player(4).unwrap().color(),
        Some(RgbColor::new(0, 0xc8, 0))
    );
    assert_eq!(engine.player(4).unwrap().color_dw(), new_color);
    assert!(engine.teams()[0].player_ids.is_empty());
    assert_eq!(engine.teams()[1].player_ids, vec![41]);
    let owned_index = engine
        .find_object_index(owned)
        .expect("owned object remains live");
    assert_eq!(engine.objects[owned_index].state.color, 0xaa00_c800);
    let missing_applied = engine
        .apply_admitted_player_team_update(99, 1, None)
        .expect("a missing joined player is a native-style no-op");
    assert!(!missing_applied);
}

#[test]
fn goal_rule_control_uses_object_scope_and_global_fallback() {
    let mut engine = Engine::new();
    register_player(&mut engine, 3, 7);
    assert_eq!(
        engine.install_global_scripts(&[(
                "RuleFallback.c".to_string(),
                "static RuleFallback; global func Activate(player) { RuleFallback = player; return true; }"
                .to_string(),
        )]),
        1
    );
    engine
        .register_definition(
            Definition::from_script(
                    "RULE",
                    "Rule",
                    "#strict 3\nlocal Marker; func Activate(player) { Marker = player; return true; } func ReadMarker() { return Marker; }",
            )
            .expect("rule definition compiles"),
        )
        .expect("rule definition registers");

    let normal = engine
        .spawn_object(SpawnConfig::new("RULE"))
        .expect("active rule spawns");
    let normal_number = i32::try_from(normal.as_u64()).unwrap();
    assert!(engine
        .execute_activate_game_goal_rule_control(&ActivateGameGoalRuleControlData {
            object: normal_number,
            player: 3,
            by_client: 7,
        })
        .expect("active SafeObjectPointer scope executes"));
    let index = engine.find_object_index(normal).unwrap();
    assert_eq!(
        engine
            .call_object_function(index, "ReadMarker", Vec::new())
            .expect("marker reads"),
        Value::Int(3)
    );

    let inactive = engine
        .spawn_object(SpawnConfig::new("RULE").with_status(ObjectStatus::Inactive))
        .expect("inactive rule spawns");
    let inactive_number = i32::try_from(inactive.as_u64()).unwrap();
    assert!(engine
        .execute_activate_game_goal_rule_control(&ActivateGameGoalRuleControlData {
            object: inactive_number,
            player: 3,
            by_client: 7,
        })
        .expect("inactive SafeObjectPointer scope executes"));
    let index = engine.find_object_index(inactive).unwrap();
    assert_eq!(
        engine
            .call_object_function(index, "ReadMarker", Vec::new())
            .expect("inactive marker reads"),
        Value::Int(3)
    );

    let fallback = engine
        .script_globals
        .borrow()
        .get("RuleFallback")
        .cloned()
        .expect("fallback global exists");
    assert_eq!(*fallback.borrow(), Value::Nil);

    assert!(engine
        .execute_activate_game_goal_rule_control(&ActivateGameGoalRuleControlData {
            object: 999_999,
            player: 3,
            by_client: 7,
        })
        .expect("missing object falls back globally"));
    assert_eq!(*fallback.borrow(), Value::Int(3));

    assert!(!engine
        .execute_activate_game_goal_rule_control(&ActivateGameGoalRuleControlData {
            object: 999_999,
            player: 3,
            by_client: 8,
        })
        .expect("unauthorized packet is a no-op"));
    assert_eq!(*fallback.borrow(), Value::Int(3));
}

#[test]
fn activate_game_goal_menu_builtin_rejects_missing_player_and_queues_valid_local_menu() {
    let mut engine = Engine::new();
    assert_eq!(
        engine
            .direct_exec_script_control_global(
                "ActivateGameGoalMenu(99)",
                "missing player",
                Some(3),
            )
            .expect("missing player call executes"),
        Value::Int(0)
    );
    assert!(engine.take_game_goal_menu_requests().is_empty());

    register_player(&mut engine, 3, 7);
    engine.set_local_players([3]);
    assert_eq!(
        engine
            .direct_exec_script_control_global("ActivateGameGoalMenu(3)", "valid player", Some(3),)
            .expect("valid player call executes"),
        Value::Int(1)
    );
    let requests = engine.take_game_goal_menu_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].player, 3);
    assert!(requests[0].open_menu);
}

#[test]
fn goal_menu_control_evaluates_every_peer_but_marks_only_local_ui() {
    let mut engine = Engine::new();
    register_player(&mut engine, 3, 7);
    let mut goal = Definition::from_script(
        "GOAL",
        "Goal",
        "#strict 3\nfunc IsFulfilled() { return true; }",
    )
    .expect("goal definition compiles");
    goal.set_category(CATEGORY_GOAL);
    engine.register_definition(goal).expect("goal registers");
    engine
        .spawn_object(SpawnConfig::new("GOAL"))
        .expect("goal object spawns");
    let control = ActivateGameGoalMenuControlData {
        player: 3,
        by_client: 7,
    };

    engine.set_local_players([3]);
    assert!(engine
        .execute_activate_game_goal_menu_control(&control)
        .expect("local goal control executes"));
    let requests = engine.take_game_goal_menu_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].player, 3);
    assert_eq!(requests[0].goals, vec!["GOAL".to_string()]);
    assert_eq!(requests[0].fulfilled_goals, vec!["GOAL".to_string()]);
    assert!(requests[0].open_menu);

    engine.set_replay_control(true);
    assert!(engine
        .execute_activate_game_goal_menu_control(&control)
        .expect("replay still evaluates goals"));
    let requests = engine.take_game_goal_menu_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].fulfilled_goals, vec!["GOAL".to_string()]);
    assert!(!requests[0].open_menu);

    engine.set_replay_control(false);
    engine.set_local_players([]);
    assert!(engine
        .execute_activate_game_goal_menu_control(&control)
        .expect("remote peer still evaluates goals"));
    let requests = engine.take_game_goal_menu_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].fulfilled_goals, vec!["GOAL".to_string()]);
    assert!(!requests[0].open_menu);

    assert!(!engine
        .execute_activate_game_goal_menu_control(&ActivateGameGoalMenuControlData {
            by_client: 8,
            ..control
        })
        .expect("spoof is rejected"));
    assert!(engine.take_game_goal_menu_requests().is_empty());
}
