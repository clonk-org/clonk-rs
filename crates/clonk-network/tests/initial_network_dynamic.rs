use std::env;
use std::path::PathBuf;

use clonk_engine::scenario::LegacyDefinitionResolver;
use clonk_engine::{
    parse_initial_network_game_data, ClientCoreControlData, InitialNetworkGameData, LegacyCString,
    NetworkResourceCore, Scenario, ScenarioError,
};
use clonk_network::{
    compose_initial_network_dynamic, InitialNetworkDynamicError, InitialNetworkDynamicSpec,
    InitialNetworkParametersError, InitialNetworkScenarioDefaults, JoinClientRegistrySnapshot,
    JoinDataC4Id, JoinDataIdListEntry, JoinGameParametersEnvelope, JoinTeamListSnapshot,
    PlayerInfoListSnapshot,
};
use clonk_resources::{c4group_file_crc, Group};

#[test]
fn pristine_tutorial01_initial_dynamic_matches_cpp_component_oracle() {
    // Clean payload/entry checks frozen from the unmodified C++ initial
    // C4GameSaveNetwork path: SaveCore adds Parameters.txt and Scenario.txt,
    // SaveData adds Game.txt, then C4FLS_Scenario orders the final group
    // (src/C4GameSave.cpp:58-108,465-515,612-617;
    // src/C4Game.cpp:2055-2104).
    let content = content_root();
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with_seed(&scenario_path, &resolver, 424_242)
        .expect("pristine Tutorial01 loads");
    let definitions = vec![
        content.join("Objects.c4d").to_string_lossy().into_owned(),
        content.join("Tutorial.c4f").to_string_lossy().into_owned(),
    ];
    let definition_executable_path = format!(
        "{}{sep}",
        content.display(),
        sep = std::path::MAIN_SEPARATOR
    );
    let game = InitialNetworkGameData::default();
    let parameters = tutorial_parameters();
    let scenario_defaults = InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 1,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: vec![id_entry(*b"SURR", 1)],
        goals: Vec::new(),
    };

    let dynamic = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: "DynTutorial01.c4s",
        maker: b"OracleHost",
        scenario: &scenario,
        scenario_title: "A Clonk",
        definition_modules: &definitions,
        definition_executable_path: &definition_executable_path,
        definition_path: "",
        scenario_origin: "Tutorial.c4f/Tutorial01.c4s",
        game: &game,
        original_game_text: None,
        parameters: &parameters,
        scenario_defaults: &scenario_defaults,
    })
    .expect("initial network dynamic composes");

    assert_eq!(
        dynamic
            .entries
            .iter()
            .map(|entry| (entry.name, entry.payload.len(), entry.contents_crc))
            .collect::<Vec<_>>(),
        vec![
            ("Scenario.txt", 1302, 0xd227_6159),
            ("Game.txt", 94, 0xe58e_8d18),
            ("Parameters.txt", 170, 0xbee7_f618),
        ]
    );
    assert_eq!(dynamic.contents_crc, 0x894e_1a59);
    assert_eq!(dynamic.group_filename, "DynTutorial01.c4s");
    assert_eq!(dynamic.maker, b"OracleHost");
    assert_eq!(dynamic.file_size as usize, dynamic.packed_bytes.len());
    assert_eq!(dynamic.file_crc, c4group_file_crc(&dynamic.packed_bytes));

    let packed = Group::from_memory(
        PathBuf::from("DynTutorial01.c4s"),
        dynamic.packed_bytes.clone(),
    )
    .expect("composed bytes are a valid C4Group");
    assert_eq!(packed.maker_bytes(), Some(b"OracleHost".as_slice()));
    assert_eq!(
        packed
            .entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect::<Vec<_>>(),
        ["Scenario.txt", "Game.txt", "Parameters.txt"]
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn savegame_runtime_sections_contribute_exact_game_entry_crc() {
    let content = content_root();
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with_seed(&scenario_path, &resolver, 424_242)
        .expect("pristine Tutorial01 loads");
    let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];
    let game_text = b"[Sky]\r\nX=65536\r\n\r\n\
[Effects]\r\nGlobalEffects=Fog(1,100,7,3,0,FOGG)\r\n\r\n\
[Scoreboard]\r\nRows=1\r\nCols=1\r\nCell0_0String=\"Scores\"\r\nCell0_0Value=-1\r\n";
    let game = parse_initial_network_game_data(game_text);
    let parameters = tutorial_parameters();
    let defaults = tutorial_defaults();

    let dynamic = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: "DynTutorial01.c4s",
        maker: b"OracleHost",
        scenario: &scenario,
        scenario_title: "A Clonk",
        definition_modules: &definitions,
        definition_executable_path: "",
        definition_path: "",
        scenario_origin: "Tutorial.c4f/Tutorial01.c4s",
        game: &game,
        original_game_text: None,
        parameters: &parameters,
        scenario_defaults: &defaults,
    })
    .expect("savegame initial network dynamic composes");

    let game_entry = dynamic
        .entries
        .iter()
        .find(|entry| entry.name == "Game.txt")
        .expect("opaque runtime sections keep Game.txt present");
    assert_eq!(game_entry.payload, game_text);
    // C4Group file entries chain CRC32(payload) with CRC32("Game.txt").
    assert_eq!(game_entry.contents_crc, 0x7a00_9287);
}

#[test]
fn composition_rejects_non_scenario_group_names_without_a_sort_fallback() {
    let content = content_root();
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&scenario_path, &resolver).unwrap();
    let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];
    let game = InitialNetworkGameData::default();
    let parameters = tutorial_parameters();
    let defaults = tutorial_defaults();

    let error = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: "Dynamic.bin",
        maker: b"OracleHost",
        scenario: &scenario,
        scenario_title: "A Clonk",
        definition_modules: &definitions,
        definition_executable_path: "",
        definition_path: "",
        scenario_origin: "Tutorial.c4f/Tutorial01.c4s",
        game: &game,
        original_game_text: None,
        parameters: &parameters,
        scenario_defaults: &defaults,
    })
    .unwrap_err();

    assert!(matches!(
        error,
        InitialNetworkDynamicError::InvalidGroupFilename(filename)
            if filename == "Dynamic.bin"
    ));
}

#[test]
fn composition_propagates_parameter_errors_without_building_a_partial_group() {
    let content = content_root();
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&scenario_path, &resolver).unwrap();
    let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];
    let game = InitialNetworkGameData::default();
    let mut parameters = tutorial_parameters();
    parameters
        .clients
        .clients
        .push(parameters.clients.clients[0].clone());
    let defaults = tutorial_defaults();

    let error = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: "DynTutorial01.c4s",
        maker: b"OracleHost",
        scenario: &scenario,
        scenario_title: "A Clonk",
        definition_modules: &definitions,
        definition_executable_path: "",
        definition_path: "",
        scenario_origin: "Tutorial.c4f/Tutorial01.c4s",
        game: &game,
        original_game_text: None,
        parameters: &parameters,
        scenario_defaults: &defaults,
    })
    .unwrap_err();

    assert!(matches!(
        error,
        InitialNetworkDynamicError::Parameters(InitialNetworkParametersError::DuplicateClientId(0))
    ));
}

#[test]
fn cpp_omits_game_entry_when_initial_game_component_is_all_default() {
    // C4Game::SaveData deletes Game.txt when decompilation is empty
    // (src/C4Game.cpp:2091-2104); composition must not fabricate a payload.
    let content = content_root();
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&scenario_path, &resolver).unwrap();
    let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];
    let mut game = InitialNetworkGameData::default();
    game.message_board_commands.clear();
    game.environment.no_gamma = false;
    let parameters = tutorial_parameters();
    let defaults = tutorial_defaults();

    let dynamic = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: "DynTutorial01.c4s",
        maker: b"OracleHost",
        scenario: &scenario,
        scenario_title: "A Clonk",
        definition_modules: &definitions,
        definition_executable_path: "",
        definition_path: "",
        scenario_origin: "Tutorial.c4f/Tutorial01.c4s",
        game: &game,
        original_game_text: None,
        parameters: &parameters,
        scenario_defaults: &defaults,
    })
    .unwrap();

    assert_eq!(
        dynamic
            .entries
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        vec!["Scenario.txt", "Parameters.txt"]
    );
}

/// An empty `Config.General.Name` leaves the new group's native default in the
/// header, because `C4Group::Close` copies the process maker only when its first
/// byte is nonzero (`src/C4Group.cpp:955`). The composed metadata must describe
/// the header that is actually packed, not the process maker that was offered.
#[test]
fn initial_dynamic_reports_the_maker_its_packed_bytes_carry() {
    let content = content_root();
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let resolver = ContentResolver {
        root: content.clone(),
    };
    let scenario = Scenario::load_from_path_with(&scenario_path, &resolver).unwrap();
    let definitions = vec!["Objects.c4d".to_owned(), "Tutorial.c4f".to_owned()];
    let game = InitialNetworkGameData::default();
    let parameters = tutorial_parameters();
    let defaults = tutorial_defaults();

    let dynamic = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: "DynTutorial01.c4s",
        maker: b"",
        scenario: &scenario,
        scenario_title: "A Clonk",
        definition_modules: &definitions,
        definition_executable_path: "",
        definition_path: "",
        scenario_origin: "Tutorial.c4f/Tutorial01.c4s",
        game: &game,
        original_game_text: None,
        parameters: &parameters,
        scenario_defaults: &defaults,
    })
    .unwrap();

    let packed = Group::from_memory(
        PathBuf::from("DynTutorial01.c4s"),
        dynamic.packed_bytes.clone(),
    )
    .unwrap();
    assert_eq!(packed.maker_bytes(), Some(dynamic.maker.as_slice()));
}

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

fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content"))
}

fn tutorial_parameters() -> JoinGameParametersEnvelope {
    let empty_players = PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    };
    let host = LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap();
    JoinGameParametersEnvelope {
        random_seed: 424_242,
        startup_player_count: 0,
        max_players: 1,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        allow_debug: true,
        is_network_game: true,
        control_rate: 2,
        auto_frame_skip: true,
        rules: vec![id_entry(*b"SURR", 1)],
        goals: Vec::new(),
        league: LegacyCString::default(),
        league_address: LegacyCString::default(),
        title: LegacyCString::from_bytes(b"A Clonk".to_vec()).unwrap(),
        scenario: NetworkResourceCore::default(),
        game_resources: Vec::new(),
        player_infos: empty_players.clone(),
        restore_player_infos: empty_players,
        teams: JoinTeamListSnapshot {
            active: 1,
            custom: 0,
            allow_hostility_change: 1,
            allow_team_switch: 0,
            auto_generate_teams: 1,
            last_team_id: 0,
            team_distribution: 0,
            team_colors: 0,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        },
        clients: JoinClientRegistrySnapshot {
            clients: vec![ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: host.clone(),
                nick: host,
                lobby_ready: false,
            }],
            local_client_id: Some(0),
        },
    }
}

fn tutorial_defaults() -> InitialNetworkScenarioDefaults {
    InitialNetworkScenarioDefaults {
        random_seed: 0,
        max_players: 1,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: vec![id_entry(*b"SURR", 1)],
        goals: Vec::new(),
    }
}

fn id_entry(id: [u8; 4], count: i32) -> JoinDataIdListEntry {
    JoinDataIdListEntry {
        id: JoinDataC4Id::from_bytes(id).unwrap(),
        count,
    }
}
