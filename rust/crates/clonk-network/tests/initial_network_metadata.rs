use clonk_engine::{
    InitialNetworkScenarioMetadata, InitialNetworkTeam, InitialNetworkTeamDistribution,
    InitialNetworkTeamMetadata, LegacyCString, NetworkResourceCore, ScenarioIdListEntry,
};
use clonk_network::{
    fill_scenario_derived_join_parameters, initial_network_scenario_defaults,
    join_team_list_snapshot, JoinClientRegistrySnapshot, JoinDataC4Id, JoinDataIdListEntry,
    JoinGameParametersEnvelope, JoinTeamListSnapshot, JoinTeamSnapshot, PlayerInfoListSnapshot,
};

#[test]
fn cpp_scenario_metadata_adapts_defaults_and_ordered_id_lists() {
    // C4GameParameters::CompileFunc takes these defaults directly from the
    // loaded scenario and preserves C4IDList entry order/counts (pristine
    // 9ffa0a5d src/C4GameParameters.cpp:553-568).
    let metadata = InitialNetworkScenarioMetadata {
        icon: 0,
        definition_modules: vec!["Objects.c4d".to_owned()],
        random_seed: -123,
        max_players: 7,
        use_fair_crew: true,
        fair_crew_forced: true,
        fair_crew_strength: 20_000,
        rules: vec![
            ScenarioIdListEntry::new("RULE", 0),
            ScenarioIdListEntry::new("R2_3", -7),
        ],
        goals: vec![ScenarioIdListEntry::new("GOAL", 4)],
    };

    let defaults = initial_network_scenario_defaults(&metadata).unwrap();

    assert_eq!(defaults.random_seed, -123);
    assert_eq!(defaults.max_players, 7);
    assert!(defaults.use_fair_crew);
    assert!(defaults.fair_crew_forced);
    assert_eq!(defaults.fair_crew_strength, 20_000);
    assert_eq!(
        defaults.rules,
        vec![id_entry(*b"RULE", 0), id_entry(*b"R2_3", -7)]
    );
    assert_eq!(defaults.goals, vec![id_entry(*b"GOAL", 4)]);
}

#[test]
fn cpp_team_metadata_moves_bytes_and_canonicalizes_bools_without_reordering() {
    // C4TeamList and nested C4Team compile every field in this order; their
    // bool fields originate as ordinary C++ bools and therefore enter a new
    // binary snapshot canonically as 0/1 (pristine 9ffa0a5d
    // src/C4Teams.cpp:138-150,556-603).
    let metadata = InitialNetworkTeamMetadata {
        active: true,
        custom: false,
        allow_hostility_change: true,
        allow_team_switch: false,
        auto_generate_teams: true,
        last_team_id: 12,
        team_distribution: InitialNetworkTeamDistribution::RandomInvisible,
        team_colors: true,
        max_script_players: 3,
        script_player_names: legacy(&[b'A', 0x80, b';', b'B']),
        random_team_count: 2,
        teams: vec![
            InitialNetworkTeam {
                id: 7,
                name: legacy(&[b'R', 0x81]),
                player_start_index: -1,
                player_ids: vec![19, -5],
                color: 0xff12_3456,
                icon_spec: legacy(&[0x82, b':', b'1']),
                max_players: 4,
            },
            InitialNetworkTeam {
                id: 2,
                name: legacy(b"Blue"),
                player_start_index: 6,
                player_ids: vec![3],
                color: 0x0011_2233,
                icon_spec: LegacyCString::default(),
                max_players: 1,
            },
        ],
    };

    let snapshot = join_team_list_snapshot(metadata);

    assert_eq!(
        snapshot,
        JoinTeamListSnapshot {
            active: 1,
            custom: 0,
            allow_hostility_change: 1,
            allow_team_switch: 0,
            auto_generate_teams: 1,
            last_team_id: 12,
            team_distribution: 4,
            team_colors: 1,
            max_script_players: 3,
            script_player_names: legacy(&[b'A', 0x80, b';', b'B']),
            random_team_count: 2,
            teams: vec![
                JoinTeamSnapshot {
                    id: 7,
                    name: legacy(&[b'R', 0x81]),
                    player_start_index: -1,
                    player_ids: vec![19, -5],
                    color: 0xff12_3456,
                    icon_spec: legacy(&[0x82, b':', b'1']),
                    max_players: 4,
                },
                JoinTeamSnapshot {
                    id: 2,
                    name: legacy(b"Blue"),
                    player_start_index: 6,
                    player_ids: vec![3],
                    color: 0x0011_2233,
                    icon_spec: LegacyCString::default(),
                    max_players: 1,
                },
            ],
        }
    );
}

#[test]
fn cpp_parameter_fill_preserves_runtime_values_and_applies_fair_crew_sequence() {
    // Load first compiles scenario defaults, then replaces RandomSeed with
    // time(), applies config FairCrew only when the scenario leaves it free,
    // and fills a zero strength only when fair crew is enabled (pristine
    // 9ffa0a5d src/C4GameParameters.cpp:411-440,553-585).
    let cases = [
        ("free", false, false, 0, true, 111, true, 111),
        ("forced fair", true, true, 0, false, 222, true, 222),
        ("forced no fair", false, true, 0, true, 333, false, 0),
        ("scenario strength", false, true, 444, true, 333, false, 444),
    ];

    for (
        label,
        metadata_use,
        metadata_forced,
        metadata_strength,
        caller_use,
        caller_strength,
        expected_use,
        expected_strength,
    ) in cases
    {
        let metadata = InitialNetworkScenarioMetadata {
            icon: 0,
            definition_modules: vec!["ignored-by-parameters.c4d".to_owned()],
            random_seed: -77,
            max_players: 6,
            use_fair_crew: metadata_use,
            fair_crew_forced: metadata_forced,
            fair_crew_strength: metadata_strength,
            rules: vec![ScenarioIdListEntry::new("RULE", 2)],
            goals: vec![ScenarioIdListEntry::new("GOAL", 3)],
        };
        let teams = helper_team_metadata();
        let mut parameters = base_parameters();
        parameters.use_fair_crew = caller_use;
        parameters.fair_crew_strength = caller_strength;
        let before = parameters.clone();

        let defaults =
            fill_scenario_derived_join_parameters(&mut parameters, &metadata, teams).unwrap();

        assert_caller_owned_unchanged(&before, &parameters, label);
        assert_eq!(parameters.max_players, 6, "{label}");
        assert_eq!(parameters.use_fair_crew, expected_use, "{label}");
        assert_eq!(parameters.fair_crew_forced, metadata_forced, "{label}");
        assert_eq!(parameters.fair_crew_strength, expected_strength, "{label}");
        assert_eq!(parameters.rules, vec![id_entry(*b"RULE", 2)], "{label}");
        assert_eq!(parameters.goals, vec![id_entry(*b"GOAL", 3)], "{label}");
        assert_eq!(parameters.teams.active, 0, "{label}");
        assert_eq!(parameters.teams.team_distribution, 1, "{label}");
        assert_eq!(defaults.random_seed, -77, "{label}");
    }
}

fn id_entry(id: [u8; 4], count: i32) -> JoinDataIdListEntry {
    JoinDataIdListEntry {
        id: JoinDataC4Id::from_bytes(id).unwrap(),
        count,
    }
}

fn legacy(bytes: &[u8]) -> LegacyCString {
    LegacyCString::from_bytes(bytes.to_vec()).unwrap()
}

fn helper_team_metadata() -> InitialNetworkTeamMetadata {
    InitialNetworkTeamMetadata {
        active: false,
        custom: true,
        allow_hostility_change: false,
        allow_team_switch: true,
        auto_generate_teams: false,
        last_team_id: 9,
        team_distribution: InitialNetworkTeamDistribution::Host,
        team_colors: false,
        max_script_players: 0,
        script_player_names: legacy(&[0x80]),
        random_team_count: 1,
        teams: Vec::new(),
    }
}

fn base_parameters() -> JoinGameParametersEnvelope {
    let players = PlayerInfoListSnapshot {
        last_player_id: 41,
        clients: Vec::new(),
    };
    JoinGameParametersEnvelope {
        random_seed: 424_242,
        startup_player_count: 5,
        max_players: 99,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        allow_debug: false,
        is_network_game: true,
        control_rate: 3,
        auto_frame_skip: true,
        rules: Vec::new(),
        goals: Vec::new(),
        league: legacy(b"League"),
        league_address: legacy(b"Address"),
        title: legacy(b"Title"),
        scenario: NetworkResourceCore {
            id: 10,
            ..NetworkResourceCore::default()
        },
        game_resources: vec![NetworkResourceCore {
            id: 11,
            ..NetworkResourceCore::default()
        }],
        player_infos: players.clone(),
        restore_player_infos: players,
        teams: JoinTeamListSnapshot {
            active: 1,
            custom: 0,
            allow_hostility_change: 1,
            allow_team_switch: 0,
            auto_generate_teams: 1,
            last_team_id: 1,
            team_distribution: 0,
            team_colors: 1,
            max_script_players: 2,
            script_player_names: legacy(b"old"),
            random_team_count: 3,
            teams: Vec::new(),
        },
        clients: JoinClientRegistrySnapshot {
            clients: Vec::new(),
            local_client_id: Some(7),
        },
    }
}

fn assert_caller_owned_unchanged(
    before: &JoinGameParametersEnvelope,
    after: &JoinGameParametersEnvelope,
    label: &str,
) {
    assert_eq!(after.random_seed, before.random_seed, "{label}");
    assert_eq!(
        after.startup_player_count, before.startup_player_count,
        "{label}"
    );
    assert_eq!(after.allow_debug, before.allow_debug, "{label}");
    assert_eq!(after.is_network_game, before.is_network_game, "{label}");
    assert_eq!(after.control_rate, before.control_rate, "{label}");
    assert_eq!(after.auto_frame_skip, before.auto_frame_skip, "{label}");
    assert_eq!(after.league, before.league, "{label}");
    assert_eq!(after.league_address, before.league_address, "{label}");
    assert_eq!(after.title, before.title, "{label}");
    assert_eq!(after.scenario, before.scenario, "{label}");
    assert_eq!(after.game_resources, before.game_resources, "{label}");
    assert_eq!(after.player_infos, before.player_infos, "{label}");
    assert_eq!(
        after.restore_player_infos, before.restore_player_infos,
        "{label}"
    );
    assert_eq!(after.clients, before.clients, "{label}");
}
