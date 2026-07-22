use clonk_engine::{ClientCoreControlData, LegacyCString, NetworkResourceCore};
use clonk_network::{
    serialize_initial_network_parameters, InitialNetworkParametersError,
    InitialNetworkScenarioDefaults, JoinClientRegistrySnapshot, JoinDataC4Id, JoinDataIdListEntry,
    JoinGameParametersEnvelope, JoinTeamListSnapshot, PlayerInfoListSnapshot,
};

#[test]
fn cpp_oracle_serializes_initial_network_parameters_exactly() {
    // Frozen from a clean build of the unmodified pre-port merge-base
    // 9ffa0a5d using an external copied scenario with an ordinary
    // Parameters.txt seed, then checked against C4GameParameters::Save/
    // CompileFunc and the nested client compiler (src/C4GameParameters.cpp:528-587;
    // src/C4Client.cpp:75-83,353-371; src/StdCompiler.cpp:248-479).
    let parameters = oracle_parameters();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert_eq!(
        encoded,
        concat!(
            "[Parameters]\r\n",
            "RandomSeed=424242\r\n",
            "IsNetworkGame=true\r\n",
            "ControlRate=2\r\n",
            "AutoFrameSkip=true\r\n",
            "\r\n",
            "  [Client]\r\n",
            "  ID=0\r\n",
            "  Activated=true\r\n",
            "  Name=\"OracleHost\"\r\n",
            "  Nick=\"OracleHost\"\r\n",
        )
        .as_bytes()
    );
}

#[test]
fn cpp_ini_writer_cannot_materialize_an_empty_id_list_over_a_nonempty_default() {
    // An empty C4IDList never asks StdCompilerINIWrite to prepare a value, so
    // NameEnd drops the pending `Rules` name. This matches the writer's stated
    // inability to distinguish an empty section from a missing one
    // (src/C4IDList.cpp:256-260; src/StdCompiler.cpp:248-280).
    let mut parameters = oracle_parameters();
    parameters.random_seed = 0;
    parameters.is_network_game = false;
    parameters.control_rate = -1;
    parameters.auto_frame_skip = false;
    parameters.clients.clients.clear();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: vec![id_entry(*b"RULE", 1)],
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert!(encoded.is_empty());
}

#[test]
fn cpp_serializes_all_parameter_fields_in_compile_order() {
    // The order and each field's scenario-independent default are fixed by
    // C4GameParameters::CompileFunc (src/C4GameParameters.cpp:555-571).
    let mut parameters = oracle_parameters();
    parameters.random_seed = -7;
    parameters.startup_player_count = 3;
    parameters.max_players = 9;
    parameters.use_fair_crew = true;
    parameters.fair_crew_forced = true;
    parameters.fair_crew_strength = 123;
    parameters.allow_debug = false;
    parameters.is_network_game = true;
    parameters.control_rate = 0;
    parameters.auto_frame_skip = true;
    parameters.rules = vec![id_entry(*b"ABCD", 1), id_entry(*b"EFGH", 0)];
    parameters.goals = vec![id_entry(*b"GOAL", -2)];
    parameters.league = LegacyCString::from_bytes(b"Cup".to_vec()).unwrap();
    parameters.clients.clients.clear();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert_eq!(
        encoded,
        concat!(
            "[Parameters]\r\n",
            "RandomSeed=-7\r\n",
            "StartupPlayerCount=3\r\n",
            "MaxPlayers=9\r\n",
            "UseFairCrew=true\r\n",
            "FairCrewForced=true\r\n",
            "FairCrewStrength=123\r\n",
            "AllowDebug=false\r\n",
            "IsNetworkGame=true\r\n",
            "ControlRate=0\r\n",
            "AutoFrameSkip=true\r\n",
            "Rules=ABCD=1;EFGH=0\r\n",
            "Goals=GOAL=-2\r\n",
            "League=\"Cup\"\r\n",
        )
        .as_bytes()
    );
}

#[test]
fn cpp_serializes_sorted_clients_with_field_defaults_and_exact_escaping() {
    // C4ClientList stores clients by ascending ID and emits repeated Client
    // sections; C4ClientCore uses the listed field order/defaults. Escaped
    // strings use octal and protect a digit following an octal escape
    // (src/C4Client.cpp:75-83,150-176,353-371;
    // src/StdCompiler.cpp:423-460).
    let mut parameters = oracle_parameters();
    parameters.random_seed = 0;
    parameters.is_network_game = false;
    parameters.control_rate = -1;
    parameters.auto_frame_skip = false;
    parameters.clients.clients = vec![
        ClientCoreControlData {
            client_id: 2,
            activated: false,
            observer: true,
            name: LegacyCString::from_bytes(b"Two".to_vec()).unwrap(),
            nick: LegacyCString::default(),
            lobby_ready: true,
        },
        ClientCoreControlData {
            client_id: -1,
            activated: false,
            observer: false,
            name: LegacyCString::default(),
            nick: LegacyCString::default(),
            lobby_ready: false,
        },
        ClientCoreControlData {
            client_id: 0,
            activated: true,
            observer: false,
            name: LegacyCString::from_bytes(b"Line\n\"\\\x011".to_vec()).unwrap(),
            nick: LegacyCString::from_bytes(b"Zero".to_vec()).unwrap(),
            lobby_ready: false,
        },
    ];
    parameters.clients.local_client_id = Some(2);
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert_eq!(
        encoded,
        concat!(
            "[Parameters]\r\n",
            "\r\n",
            "  [Client]\r\n",
            "  ID=0\r\n",
            "  Activated=true\r\n",
            "  Name=\"Line\\n\\\"\\\\\\1\\61\"\r\n",
            "  Nick=\"Zero\"\r\n",
            "\r\n",
            "  [Client]\r\n",
            "  ID=2\r\n",
            "  Observer=true\r\n",
            "  Name=\"Two\"\r\n",
            "  LobbyReady=true\r\n",
        )
        .as_bytes()
    );
}

#[test]
fn cpp_string_writer_preserves_non_utf8_and_utf8_bytes_as_octal() {
    // WriteEscaped operates on unsigned bytes. Under the deterministic ASCII
    // printability contract, every non-ASCII byte is preserved as octal rather
    // than decoded or replaced (src/StdCompiler.cpp:423-460).
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };
    let mut non_utf8 = oracle_parameters();
    non_utf8.random_seed = 0;
    non_utf8.is_network_game = false;
    non_utf8.control_rate = -1;
    non_utf8.auto_frame_skip = false;
    non_utf8.clients.clients.clear();
    non_utf8.league = LegacyCString::from_bytes(vec![0xff, b'1']).unwrap();
    assert_eq!(
        serialize_initial_network_parameters(&non_utf8, &defaults).unwrap(),
        b"[Parameters]\r\nLeague=\"\\377\\61\"\r\n"
    );

    let mut utf8 = non_utf8;
    utf8.league = LegacyCString::from_bytes("é".as_bytes().to_vec()).unwrap();
    assert_eq!(
        serialize_initial_network_parameters(&utf8, &defaults).unwrap(),
        b"[Parameters]\r\nLeague=\"\\303\\251\"\r\n"
    );
}

#[test]
fn duplicate_client_ids_are_a_typed_unrepresentable_list_error() {
    // C4ClientList::Add asserts ID uniqueness before linked-list insertion
    // (src/C4Client.cpp:155-176), so no valid C++ Parameters value can contain
    // two Client sections with the same ID.
    let mut parameters = oracle_parameters();
    parameters
        .clients
        .clients
        .push(parameters.clients.clients[0].clone());
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let error = serialize_initial_network_parameters(&parameters, &defaults).unwrap_err();

    assert_eq!(error, InitialNetworkParametersError::DuplicateClientId(0));
}

#[test]
fn cpp_string_writer_uses_all_named_escapes_and_unpadded_octal() {
    // StdCompilerINIWrite::WriteEscaped has named escapes for the seven C
    // controls, quote, and slash; remaining bytes use unpadded octal and a
    // following digit must itself be octal-escaped (src/StdCompiler.cpp:423-460).
    let mut parameters = oracle_parameters();
    parameters.random_seed = 0;
    parameters.is_network_game = false;
    parameters.control_rate = -1;
    parameters.auto_frame_skip = false;
    parameters.clients.clients.clear();
    parameters.league =
        LegacyCString::from_bytes(b"\x07\x08\x0c\n\r\t\x0b\"\\\x7f9".to_vec()).unwrap();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert_eq!(
        encoded,
        b"[Parameters]\r\nLeague=\"\\a\\b\\f\\n\\r\\t\\v\\\"\\\\\\177\\71\"\r\n"
    );
}

#[test]
fn save_with_scenario_does_not_inspect_the_omitted_savegame_block() {
    // The non-null pScenario branch skips LeagueAddress, Title, resources,
    // player infos, and teams completely (src/C4GameParameters.cpp:573-585).
    let mut parameters = oracle_parameters();
    parameters.league_address = LegacyCString::from_bytes(vec![0xff]).unwrap();
    parameters.title = LegacyCString::from_bytes(vec![0xfe]).unwrap();
    parameters.scenario.filename = LegacyCString::from_bytes(vec![0xfd]).unwrap();
    parameters.game_resources[0].author = LegacyCString::from_bytes(vec![0xfc]).unwrap();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert!(encoded.starts_with(b"[Parameters]\r\nRandomSeed=424242\r\n"));
    assert!(!encoded.windows(5).any(|window| window == b"Title"));
}

#[test]
fn cpp_omits_the_root_section_when_every_value_is_defaulted() {
    // StdCompilerINIWrite delays section names until a child emits a value;
    // therefore a fully defaulted Parameters object produces an empty buffer
    // (src/StdCompiler.cpp:248-280,398-479).
    let mut parameters = oracle_parameters();
    parameters.is_network_game = false;
    parameters.control_rate = -1;
    parameters.auto_frame_skip = false;
    parameters.clients.clients.clear();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 424_242,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert!(encoded.is_empty());
}

#[test]
fn cpp_elides_nontrivial_scenario_fair_crew_and_id_list_defaults() {
    // The default adaptors compare against the supplied scenario Head/Game
    // values, not hardcoded zeroes (src/C4GameParameters.cpp:555-569).
    let mut parameters = oracle_parameters();
    parameters.random_seed = 77;
    parameters.max_players = 12;
    parameters.use_fair_crew = true;
    parameters.fair_crew_forced = true;
    parameters.fair_crew_strength = 20_000;
    parameters.rules = vec![id_entry(*b"RULE", 1)];
    parameters.goals = vec![id_entry(*b"GOAL", 2)];
    parameters.is_network_game = false;
    parameters.control_rate = -1;
    parameters.auto_frame_skip = false;
    parameters.clients.clients.clear();
    let defaults = InitialNetworkScenarioDefaults {
        random_seed: 77,
        max_players: 12,
        use_fair_crew: true,
        fair_crew_forced: true,
        fair_crew_strength: 20_000,
        rules: parameters.rules.clone(),
        goals: parameters.goals.clone(),
    };

    let encoded = serialize_initial_network_parameters(&parameters, &defaults).unwrap();

    assert!(encoded.is_empty());
}

fn oracle_parameters() -> JoinGameParametersEnvelope {
    let empty_players = PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    };
    let oracle_host = LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap();
    JoinGameParametersEnvelope {
        random_seed: 424_242,
        startup_player_count: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        allow_debug: true,
        is_network_game: true,
        control_rate: 2,
        auto_frame_skip: true,
        rules: Vec::new(),
        goals: Vec::new(),
        league: LegacyCString::default(),
        // Save(pScenario) must ignore all fields in this block.
        league_address: LegacyCString::from_bytes(b"ignored league address".to_vec()).unwrap(),
        title: LegacyCString::from_bytes(b"ignored title".to_vec()).unwrap(),
        scenario: NetworkResourceCore {
            resource_type: 1,
            id: 99,
            ..NetworkResourceCore::default()
        },
        game_resources: vec![NetworkResourceCore {
            resource_type: 4,
            id: 100,
            ..NetworkResourceCore::default()
        }],
        player_infos: empty_players.clone(),
        restore_player_infos: empty_players,
        teams: JoinTeamListSnapshot {
            active: 1,
            custom: 1,
            allow_hostility_change: 1,
            allow_team_switch: 1,
            auto_generate_teams: 1,
            last_team_id: 7,
            team_distribution: 2,
            team_colors: 1,
            max_script_players: 3,
            script_player_names: LegacyCString::from_bytes(b"ignored".to_vec()).unwrap(),
            random_team_count: 2,
            teams: Vec::new(),
        },
        clients: JoinClientRegistrySnapshot {
            clients: vec![ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: oracle_host.clone(),
                nick: oracle_host,
                lobby_ready: false,
            }],
            local_client_id: Some(0),
        },
    }
}

fn id_entry(id: [u8; 4], count: i32) -> JoinDataIdListEntry {
    JoinDataIdListEntry {
        id: JoinDataC4Id::from_bytes(id).unwrap(),
        count,
    }
}
