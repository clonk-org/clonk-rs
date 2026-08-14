use std::fs;
use std::path::{Path, PathBuf};

use clonk_network::{
    HostInitialResourceSource, NetworkAddress, NetworkProtocol, NETWORK_STATE_LOBBY,
};
use clonk_resources::{Group, LanguagePacks, MutableGroup};

use crate::host_game_resource_sources::freeze_host_definition_resource_sources;

use crate::prepared_host_bootstrap;
use crate::prepared_host_bootstrap::{
    prepare_host_bootstrap, prepare_host_bootstrap_with_team_assignment_oracle,
    PrepareHostBootstrapError, PreparedHostBootstrapConfig, PreparedHostBootstrapSpec,
    PreparedHostPlayerIdentity, PreparedHostPlayerSource, PreparedHostUseError,
    PreparedLeagueHostConfig,
};

fn prepare_harpoonrace_host(
    random_seed_unix_seconds: i64,
    league: Option<&PreparedLeagueHostConfig>,
) -> (
    prepared_host_bootstrap::PreparedHostBootstrap,
    tempfile::TempDir,
) {
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let scenario_path = content.join("EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s");
    let install_roots = vec![repository, content.clone(), planet];
    prepare_harpoonrace_host_from_paths(
        random_seed_unix_seconds,
        league,
        &scenario_path,
        &content,
        &install_roots,
    )
}

fn prepare_harpoonrace_host_from_paths(
    random_seed_unix_seconds: i64,
    league: Option<&PreparedLeagueHostConfig>,
    scenario_path: &Path,
    content: &Path,
    install_roots: &[PathBuf],
) -> (
    prepared_host_bootstrap::PreparedHostBootstrap,
    tempfile::TempDir,
) {
    let definition_resource_paths =
        vec![content.join("Objects.c4d"), content.join("EkeReloaded.c4d")];
    let effective_definition_modules = vec!["Objects.c4d".to_owned(), "EkeReloaded.c4d".to_owned()];
    let definition_resources = freeze_host_definition_resource_sources(
        &definition_resource_paths,
        scenario_path,
        &effective_definition_modules,
        false,
        content,
        "",
    )
    .expect("freeze HarpoonRace definitions");
    let definition_executable_path = format!("{}{}", content.display(), std::path::MAIN_SEPARATOR);
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let network = tempfile::tempdir().expect("isolated host resources");

    let prepared = prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path,
        install_roots,
        definition_resources: &definition_resources,
        effective_definition_modules: &effective_definition_modules,
        initial_definition_modules: &[],
        fixed_definition_modules: None,
        selector_definition_root: None,
        definition_executable_path: &definition_executable_path,
        definition_path: "",
        languages: &languages,
        language_packs: &language_packs,
        network_directory: network.path(),
        network_work_path: "Network",
        start_unix_seconds: 1_784_903_469,
        random_seed_unix_seconds,
        group_maker: "Worldgen test",
        host_name: "Host",
        host_nick: "Host",
        network_password: "",
        network_comment: "",
        netpuncher_address: "puncher.invalid:11115",
        player_sources: &[],
        config: PreparedHostBootstrapConfig {
            control_mode: 0,
            control_rate: 2,
            async_max_wait: 2,
            fair_crew: true,
            fair_crew_strength: 1_000,
            auto_frame_skip: true,
            max_load_file_size: 100 * 1024 * 1024,
            no_runtime_join: true,
            enable_upnp: false,
            network_tcp_port: 0,
            network_udp_port: 0,
        },
        league,
    })
    .expect("prepare HarpoonRace host");

    (prepared, network)
}

#[test]
fn harpoonrace_host_retries_before_publishing_the_random_seed() {
    let (prepared, _network) = prepare_harpoonrace_host(1_784_903_470, None);
    let join = prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData");
    assert_eq!(
        join.parameters.random_seed, 1_784_903_471,
        "the known-invalid seed is advanced before JoinData publication"
    );
    let dynamic_path = prepared
        .host_config()
        .resource_files
        .iter()
        .find(|resource| resource.core.id == join.dynamic.id)
        .map(|resource| resource.path.as_path())
        .expect("published Dynamic resource");
    let dynamic = Group::open(dynamic_path).expect("open Dynamic resource");
    let parameters = dynamic
        .read_file("Parameters.txt")
        .expect("read Dynamic Parameters");
    assert!(
        parameters
            .windows(b"RandomSeed=1784903471".len())
            .any(|window| window == b"RandomSeed=1784903471"),
        "Dynamic and JoinData must publish the same accepted seed"
    );
    let scenario = prepared
        .claim_scenario()
        .expect("claim accepted host scenario");
    assert!(
        !scenario.generated_landscape_requires_seed_retry(),
        "the retained host scenario must match the accepted seed"
    );
}

#[test]
fn harpoonrace_live_signup_rejects_an_invalid_seed_after_signup_off_prepare() {
    // Native applies the league Start seed before Landscape.Init
    // (oracle src/C4Network2.cpp:2378-2431; src/C4Game.cpp:2660-2672).
    // Rust cannot advance that externally authoritative seed, but it must not
    // publish or launch a known-invalid generated landscape.
    let league = PreparedLeagueHostConfig {
        endpoint: "https://league.invalid/".to_owned(),
        transport: clonk_network::LeagueHttpTransportConfig::default(),
        update_period_secs: 120,
        league_server_signup: false,
    };
    let (mut prepared, _network) = prepare_harpoonrace_host(1_784_903_471, Some(&league));

    let error = prepared
        .apply_league_start_response(&clonk_network::LeagueStartResponse {
            seed: Some(1_784_903_470),
            ..clonk_network::LeagueStartResponse::default()
        })
        .expect_err("the league-assigned seed exposes the SkyParcour water fill");

    assert!(matches!(
        error,
        PrepareHostBootstrapError::LeagueGeneratedLandscapeInvalid {
            random_seed: 1_784_903_470
        }
    ));
    assert_eq!(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .random_seed,
        1_784_903_471,
        "a rejected Start reply must not partially mutate JoinData"
    );
}

#[test]
fn harpoonrace_save_game_keeps_its_retained_landscape_for_a_league_seed() {
    // Saved games restore their serialized landscape instead of regenerating
    // it from the league Start seed (src/C4Game.cpp:2455-2462,2642-2672).
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let source_scenario =
        content.join("EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s");
    let isolated = tempfile::tempdir().expect("isolated savegame source");
    let isolated_content = isolated.path().join("content");
    let isolated_scenario = isolated_content.join("HarpoonRace.c4s");
    fs::create_dir_all(&isolated_scenario).expect("create isolated savegame");
    for entry in fs::read_dir(&source_scenario).expect("read HarpoonRace source") {
        let entry = entry.expect("read HarpoonRace source entry");
        assert!(
            entry.file_type().expect("read source entry kind").is_file(),
            "the HarpoonRace fixture copy expects only direct files"
        );
        fs::copy(entry.path(), isolated_scenario.join(entry.file_name()))
            .expect("copy isolated HarpoonRace entry");
    }
    let scenario_core_path = isolated_scenario.join("Scenario.txt");
    let scenario_core =
        fs::read_to_string(&scenario_core_path).expect("read isolated Scenario core");
    let savegame_core = scenario_core.replacen("[Head]", "[Head]\nSaveGame=1", 1);
    assert_ne!(
        savegame_core, scenario_core,
        "the fixture must expose a Head section"
    );
    fs::write(&scenario_core_path, savegame_core).expect("mark isolated scenario as a savegame");
    let install_roots = vec![isolated_content, repository, content.clone(), planet];
    let league = PreparedLeagueHostConfig {
        endpoint: "https://league.invalid/".to_owned(),
        transport: clonk_network::LeagueHttpTransportConfig::default(),
        update_period_secs: 120,
        league_server_signup: false,
    };
    let (mut prepared, _network) = prepare_harpoonrace_host_from_paths(
        1_784_903_471,
        Some(&league),
        &isolated_scenario,
        &content,
        &install_roots,
    );

    prepared
        .apply_league_start_response(&clonk_network::LeagueStartResponse {
            seed: Some(1_784_903_470),
            ..clonk_network::LeagueStartResponse::default()
        })
        .expect("the Start seed must not regenerate a saved landscape");

    assert_eq!(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .random_seed,
        1_784_903_470
    );
    let scenario = prepared
        .claim_scenario()
        .expect("claim retained savegame scenario");
    assert!(
        !scenario.generated_landscape_requires_seed_retry(),
        "the valid serialized landscape must remain retained"
    );
}

#[test]
fn harpoonrace_league_reloads_the_retained_map_for_a_valid_assigned_seed() {
    // The Start seed is installed before native creates the landscape
    // (oracle src/C4Network2.cpp:2430-2432; src/C4Game.cpp:2660-2672).
    let league = PreparedLeagueHostConfig {
        endpoint: "https://league.invalid/".to_owned(),
        transport: clonk_network::LeagueHttpTransportConfig::default(),
        update_period_secs: 120,
        league_server_signup: true,
    };
    let (mut prepared, _network) = prepare_harpoonrace_host(1_784_903_470, Some(&league));

    prepared
        .apply_league_start_response(&clonk_network::LeagueStartResponse {
            seed: Some(1_784_903_471),
            ..clonk_network::LeagueStartResponse::default()
        })
        .expect("the assigned seed produces a valid SkyParcour landscape");

    assert_eq!(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .random_seed,
        1_784_903_471
    );
    let scenario = prepared
        .claim_scenario()
        .expect("claim league-seeded host scenario");
    assert!(
        !scenario.generated_landscape_requires_seed_retry(),
        "the host must launch the map generated from the published Start seed"
    );
}

#[test]
fn harpoonrace_league_seed_reload_uses_the_published_scenario_bytes() {
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let source_scenario =
        content.join("EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s");
    let isolated = tempfile::tempdir().expect("isolated scenario source");
    let isolated_content = isolated.path().join("content");
    let isolated_scenario = isolated_content.join("HarpoonRace.c4s");
    fs::create_dir_all(&isolated_scenario).expect("create isolated scenario");
    for entry in fs::read_dir(&source_scenario).expect("read HarpoonRace source") {
        let entry = entry.expect("read HarpoonRace source entry");
        assert!(
            entry.file_type().expect("read source entry kind").is_file(),
            "the HarpoonRace fixture copy expects only direct files"
        );
        fs::copy(entry.path(), isolated_scenario.join(entry.file_name()))
            .expect("copy isolated HarpoonRace entry");
    }
    let install_roots = vec![isolated_content, repository, content.clone(), planet];
    let league = PreparedLeagueHostConfig {
        endpoint: "https://league.invalid/".to_owned(),
        transport: clonk_network::LeagueHttpTransportConfig::default(),
        update_period_secs: 120,
        league_server_signup: false,
    };
    let (mut prepared, _network) = prepare_harpoonrace_host_from_paths(
        1_784_903_471,
        Some(&league),
        &isolated_scenario,
        &content,
        &install_roots,
    );

    let landscape_path = isolated_scenario.join("Landscape.txt");
    let source = fs::read_to_string(&landscape_path).expect("read isolated Landscape");
    let changed = source.replacen("mat=Water", "mat=Earth", 1);
    assert_ne!(
        changed, source,
        "the fixture must contain its water overlay"
    );
    fs::write(&landscape_path, changed).expect("mutate only the isolated source");

    let error = prepared
        .apply_league_start_response(&clonk_network::LeagueStartResponse {
            seed: Some(1_784_903_470),
            ..clonk_network::LeagueStartResponse::default()
        })
        .expect_err("the published Landscape bytes still expose water for the known-invalid seed");
    assert!(matches!(
        error,
        PrepareHostBootstrapError::LeagueGeneratedLandscapeInvalid {
            random_seed: 1_784_903_470
        }
    ));
}

#[test]
fn tutorial01_builds_the_exact_supported_initial_host_bootstrap() {
    // The builder follows OpenScenario -> Parameters::Load -> InitHost ->
    // CreateDynamic, then adds the empty local Initial player packet before
    // opening admission (pristine 9ffa0a5d src/C4Game.cpp:123-278,3847-3876;
    // src/C4GameParameters.cpp:362-442,553-585;
    // src/C4Network2.cpp:222-278,1945-1971;
    // src/C4PlayerInfo.cpp:357-397,834-880).
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let scenario_path = content.join("Tutorial.c4f/Tutorial01.c4s");
    let network = tempfile::tempdir().unwrap();
    fs::write(network.path().join("DynTutorial01.c4s"), b"collision").unwrap();
    let definition_resource_paths = vec![content.join("Objects.c4d"), content.join("Tutorial.c4f")];
    let effective_definition_modules = vec!["Objects.c4d".to_owned()];
    let definition_resources = freeze_host_definition_resource_sources(
        &definition_resource_paths,
        &scenario_path,
        &effective_definition_modules,
        false,
        &content,
        "",
    )
    .unwrap();
    let definition_executable_path = format!("{}{}", content.display(), std::path::MAIN_SEPARATOR);
    let install_roots = vec![content, planet];
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let player_sources = Vec::new();

    let prepared = prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &scenario_path,
        install_roots: &install_roots,
        definition_resources: &definition_resources,
        effective_definition_modules: &effective_definition_modules,
        initial_definition_modules: &[],
        fixed_definition_modules: None,
        selector_definition_root: None,
        definition_executable_path: &definition_executable_path,
        definition_path: "",
        languages: &languages,
        language_packs: &language_packs,
        network_directory: network.path(),
        network_work_path: "Network",
        start_unix_seconds: 1_720_000_122,
        random_seed_unix_seconds: 1_720_000_123,
        group_maker: "FileMaker",
        host_name: "NetworkName",
        host_nick: "NetworkNick",
        network_password: "round secret",
        network_comment: " Host comment ",
        netpuncher_address: "puncher.invalid:11115",
        player_sources: &player_sources,
        config: PreparedHostBootstrapConfig {
            control_mode: 2,
            control_rate: 3,
            async_max_wait: 7,
            fair_crew: true,
            fair_crew_strength: 4_321,
            auto_frame_skip: true,
            max_load_file_size: 100 * 1024 * 1024,
            no_runtime_join: true,
            enable_upnp: true,
            network_tcp_port: 11_112,
            network_udp_port: 11_113,
        },
        league: None,
    })
    .unwrap();

    let host = prepared.host_config();
    assert!(!host.allow_join);
    assert_eq!(host.start_tick, 0);
    assert_eq!(host.async_max_wait_frames, 7);
    // HandlePlayerInfo::LoadResources uses the same installed resource roots
    // after the initial JoinData bootstrap (pristine 9ffa0a5d
    // src/C4Network2Players.cpp:245-260; src/C4Network2Res.cpp:1473-1516).
    assert_eq!(host.local_resource_roots, install_roots);
    assert_eq!(prepared.start_time(), 1_720_000_122);
    assert_eq!(host.max_players, 1);
    assert!(host.enable_upnp);
    assert_eq!(host.configured_tcp_port, Some(11_112));
    assert_eq!(host.configured_udp_port, Some(11_113));
    assert_eq!(
        host.initial_status,
        clonk_network::NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 2,
            target_tick: 0,
        }
    );
    let tcp_address = NetworkAddress::new(NetworkProtocol::Tcp, "127.0.0.1:11111".parse().unwrap());
    let reference = prepared
        .initial_host_game_reference(true, &[tcp_address])
        .expect("prepared host builds the exact post-admission reference");
    assert_eq!(
        reference.summary().title,
        "A Clonk",
        "C++ reloads the selected scenario Title component instead of trusting caller display text"
    );
    assert_eq!(reference.summary().host_name, "NetworkName");
    assert_eq!(reference.summary().host_nick, "NetworkNick");
    assert_eq!(reference.summary().state, "Lobby");
    assert_eq!(reference.summary().control_mode, 2);
    assert_eq!(reference.summary().start_time, 1_720_000_122);
    assert!(reference.summary().join_allowed);
    assert!(reference.summary().password_needed);
    assert_eq!(host.password.as_bytes(), b"round secret");
    assert_eq!(reference.summary().max_players, 1);
    // InitLocal copies every local net-client address into the reference's
    // canonical Addrs container (pristine 9ffa0a5d
    // src/C4Network2Reference.cpp:81-85).
    assert_eq!(reference.summary().addresses, vec![tcp_address]);
    assert_eq!(
        reference.summary().tcp_addresses,
        vec![tcp_address.endpoint]
    );
    assert_eq!(reference.metadata().icon, 2);
    assert_eq!(reference.metadata().comment.as_bytes(), b" Host comment ");
    assert_eq!(reference.metadata().addresses, vec![tcp_address]);
    // InitLocal copies the live puncher metadata into the same reference that
    // clients consume before connection setup (pristine 9ffa0a5d
    // src/C4Network2Reference.cpp:77-78;
    // src/C4Network2.cpp:292-293).
    assert_eq!(
        reference.summary().netpuncher_address,
        "puncher.invalid:11115"
    );
    assert_eq!(
        reference.metadata().netpuncher_address.as_bytes(),
        b"puncher.invalid:11115"
    );
    assert_eq!(
        reference.parameters(),
        &host
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
    );
    assert_eq!(host.local_core.client_id, 0);
    assert!(host.local_core.activated);
    assert!(!host.local_core.observer);
    assert!(!host.local_core.lobby_ready);
    assert_eq!(host.local_core.name.as_bytes(), b"NetworkName");
    assert_eq!(host.local_core.nick.as_bytes(), b"NetworkNick");

    let snapshot = host.initial_join_snapshot.as_ref().unwrap();
    assert_eq!(snapshot.dynamic_tick, 0);
    assert_eq!(snapshot.parameters.random_seed, 1_720_000_123);
    assert_eq!(snapshot.parameters.control_rate, 3);
    assert!(snapshot.parameters.use_fair_crew);
    assert_eq!(snapshot.parameters.fair_crew_strength, 4_321);
    assert!(snapshot.parameters.auto_frame_skip);
    assert!(snapshot.parameters.is_network_game);
    assert_eq!(snapshot.parameters.max_players, 1);
    assert_eq!(snapshot.parameters.clients.local_client_id, Some(0));
    assert_eq!(
        snapshot.parameters.clients.clients,
        vec![host.local_core.clone()]
    );
    assert_eq!(snapshot.parameters.player_infos.last_player_id, 0);
    assert_eq!(snapshot.parameters.player_infos.clients.len(), 1);
    let initial_players = &snapshot.parameters.player_infos.clients[0];
    assert_eq!(initial_players.client_id, 0);
    assert_eq!(initial_players.flags, 1 << 2);
    assert!(initial_players.players.is_empty());
    assert!(snapshot.parameters.restore_player_infos.clients.is_empty());
    assert_eq!(
        prepared.initial_host_player_info_control(),
        &clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: Vec::new(),
            by_client: 0,
        },
        "Players.Init executes this direct control before AllowJoin(true)"
    );

    assert_eq!(
        prepared.scenario_wire_name().as_bytes(),
        b"Tutorial.c4f/Tutorial01.c4s"
    );
    assert_eq!(prepared.scenario_origin(), "Tutorial.c4f/Tutorial01.c4s");
    assert_eq!(
        prepared.dynamic_filename_seed(),
        format!("Network{}DynTutorial01.c4s", std::path::MAIN_SEPARATOR)
    );
    let expected_dynamic_wire = if cfg!(windows) {
        b"Network\\Network_DynTutorial01.c4s".as_slice()
    } else {
        b"Network/DynTutorial01_2.c4s".as_slice()
    };
    assert_eq!(
        prepared.dynamic_wire_name().as_bytes(),
        expected_dynamic_wire
    );
    assert_eq!(
        host.resource_files
            .iter()
            .map(|resource| {
                (
                    resource.core.id,
                    resource.core.resource_type,
                    resource.core.filename.as_bytes().to_vec(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (0, 1, b"Tutorial.c4f/Tutorial01.c4s".to_vec()),
            (1, 4, b"Objects.c4d".to_vec()),
            (2, 4, b"Tutorial.c4f".to_vec()),
            (3, 5, b"System.c4g".to_vec()),
            (4, 6, b"Material.c4g".to_vec()),
            (5, 2, expected_dynamic_wire.to_vec()),
        ]
    );
    assert_eq!(snapshot.dynamic.id, 5);
    let dynamic = Group::open(&host.resource_files[5].path).unwrap();
    assert_eq!(dynamic.maker_bytes(), Some(b"FileMaker".as_slice()));
    assert_eq!(
        Group::open(&host.resource_files[0].path)
            .unwrap()
            .maker_bytes(),
        Some(b"FileMaker".as_slice())
    );
    assert_eq!(
        fs::read(network.path().join("DynTutorial01.c4s")).unwrap(),
        b"collision"
    );
    let dynamic_scenario = dynamic.read_file("Scenario.txt").unwrap();
    assert!(dynamic_scenario
        .windows(b"Definitions=\"Objects.c4d\",\"Tutorial.c4f\"".len())
        .any(|line| line == b"Definitions=\"Objects.c4d\",\"Tutorial.c4f\""));
    let parameters = dynamic.read_file("Parameters.txt").unwrap();
    assert!(parameters
        .windows(b"RandomSeed=1720000123".len())
        .any(|line| { line == b"RandomSeed=1720000123" }));
    assert!(parameters
        .windows(b"Name=\"NetworkName\"".len())
        .any(|line| { line == b"Name=\"NetworkName\"" }));
    assert!(!parameters
        .windows(b"FileMaker".len())
        .any(|line| line == b"FileMaker"));

    let mut control_clients = clonk_engine::ControlPlayerInfoRegistry::default();
    let admission_ready = prepared
        .install_initial_host_player_state(&mut control_clients, |_, _| {
            panic!("observer host has no local player resource")
        })
        .expect("Initial PlayerInfo is installed exactly once");
    assert_eq!(
        prepared
            .install_initial_host_player_state(&mut control_clients, |_, _| {})
            .expect_err("the admission capability cannot be minted twice"),
        PreparedHostUseError::InitialPlayerInfoAlreadyInstalled
    );
    assert!(control_clients.contains_client(0));
    assert_eq!(control_clients.player_count(), 0);
    assert_eq!(prepared.admission().max_players(), 1);
    assert!(admission_ready.lobby_join_allowed());
    assert!(!prepared.admission().runtime_join_allowed());

    let launch = prepared
        .claim_host_config()
        .expect("the prepared resources have one launch owner");
    assert_eq!(
        prepared
            .claim_host_config()
            .expect_err("a prepared resource set cannot launch twice"),
        PreparedHostUseError::HostAlreadyLaunched
    );
    let temporary_path = launch
        .resource_files
        .iter()
        .find(|resource| resource.ownership == clonk_network::ResourceFileOwnership::Temporary)
        .expect("prepared dynamic is temporary")
        .path
        .clone();
    let retained = prepared.clone();
    drop(prepared);
    assert!(
        temporary_path.exists(),
        "a retained launch keeps resources alive"
    );
    drop(retained);
    assert!(
        !temporary_path.exists(),
        "the last prepared owner cleans an unlaunched temporary"
    );
}

#[test]
fn prepared_clones_share_one_claim_of_the_loaded_scenario() {
    // C4Game owns one C4S member: OpenScenario loads it before InitNetworkHost,
    // and the same loaded value survives the lobby and is consumed by InitGame
    // (pristine 9ffa0a5d src/C4Game.h:107;
    // src/C4Game.cpp:421-456; src/C4Game.cpp:3847-3888).
    let fixture = minimal_install(None);
    let prepared = prepare(&fixture, &[]).expect("prepare the host scenario once");
    let retained = prepared.clone();

    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture.scenario_text.replace("MaxPlayer=2", "MaxPlayer=7"),
    )
    .unwrap();

    let scenario = retained
        .claim_scenario()
        .expect("a prepared launch owns the already-loaded scenario");
    assert_eq!(
        scenario
            .initial_network_scenario_metadata()
            .unwrap()
            .max_players,
        2,
        "claiming must not reopen changed source content"
    );
    assert_eq!(
        prepared
            .claim_scenario()
            .expect_err("all prepared clones share one launch scenario"),
        PreparedHostUseError::ScenarioAlreadyClaimed
    );
}

#[test]
fn prepared_scenario_load_ticket_preserves_the_single_claim() {
    // C4Game owns one opened scenario across the lobby and InitGame; moving
    // the post-lobby work to a loader worker must not create a second launch
    // claim (src/C4Game.h:107; src/C4Game.cpp:421-457).
    let fixture = minimal_install(None);
    let prepared = prepare(&fixture, &[]).expect("prepare the host scenario once");
    let retained = prepared.clone();

    let ticket = retained
        .claim_scenario_load()
        .expect("claim the one post-lobby scenario load");
    assert_eq!(
        ticket
            .retained()
            .initial_network_scenario_metadata()
            .unwrap()
            .max_players,
        2
    );
    assert_eq!(
        prepared
            .claim_scenario()
            .expect_err("the load ticket consumes the shared scenario claim"),
        PreparedHostUseError::ScenarioAlreadyClaimed
    );
}

#[test]
fn map_player_extend_ticket_reloads_with_the_post_lobby_player_count() {
    // Preload deliberately leaves MapPlayerExtend landscape creation to the
    // foreground InitGame pass, after StartupPlayerCount has been recomputed
    // from the final roster (src/C4Game.cpp:2455-2462,2642-2649).
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        format!(
            "{}\n[Landscape]\nMapWidth=20,0,1,20\nMapHeight=10,0,1,10\n\
             MapZoom=5\nMapPlayerExtend=1\nMaterial=Earth\n",
            fixture.scenario_text
        ),
    )
    .expect("write MapPlayerExtend scenario");
    let materials = fixture.install_roots[0].join("Material.c4g");
    fs::write(materials.join("TexMap.txt"), b"1=Earth-Smooth\n").expect("write texture map");
    fs::write(
        materials.join("Earth.c4m"),
        b"[Material]\nName=Earth\nDensity=100\n",
    )
    .expect("write material");

    let prepared = prepare(&fixture, &[]).expect("prepare the host scenario");
    let random_seed = u64::from(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .random_seed as u32,
    );
    let ticket = prepared
        .claim_scenario_load()
        .expect("claim the post-lobby load");
    assert!(ticket.retained().uses_map_player_extend());
    let mut preloaded = clonk_engine::Engine::with_seed(random_seed);
    ticket
        .retained()
        .apply(&mut preloaded)
        .expect("apply preparation-time scenario");

    let loaded = ticket
        .load_with_progress(random_seed, 3, |_, _| {})
        .expect("reload with the final roster");
    let mut post_lobby = clonk_engine::Engine::with_seed(random_seed);
    loaded
        .apply(&mut post_lobby)
        .expect("apply post-lobby scenario");

    assert_eq!(
        (
            preloaded.landscape().expect("preloaded landscape").width(),
            post_lobby
                .landscape()
                .expect("post-lobby landscape")
                .width(),
        ),
        (100, 300)
    );
}

#[test]
fn restore_infos_seed_host_ids_and_recreate_script_players_in_join_data() {
    // Parameters::Load localizes SavePlayerInfos, transfers its raw ID
    // counter to the live PlayerInfos list, and Network2Players::Init copies
    // every unclaimed script restore row into the host packet before opening
    // admission (src/C4GameParameters.cpp:379-389;
    // src/C4Network2Players.cpp:38-69;
    // src/C4PlayerInfo.cpp:1325-1358).
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("StringTblUS.txt"),
        b"SavedBot=Localized Script\n",
    )
    .unwrap();
    fs::write(
        fixture.scenario_path.join("SavePlayerInfos.txt"),
        b"[PlayerInfoList]\n\
LastPlayerID=9\n\
\n\
\x20\x20[Client]\n\
\x20\x20ID=5\n\
\n\
\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20Name=\"$SavedBot$\"\n\
\x20\x20\x20\x20ForcedName=\"Raw\x81\"\n\
\x20\x20\x20\x20Flags=Joined|AttributesFixed|NoScenarioInit|NoEliminationCheck\n\
\x20\x20\x20\x20ID=3\n\
\x20\x20\x20\x20Type=Script\n\
\x20\x20\x20\x20Color=65535\n\
\x20\x20\x20\x20OriginalColor=65535\n\
\x20\x20\x20\x20GameNumber=10\n\
\x20\x20\x20\x20Team=2\n",
    )
    .unwrap();
    fs::write(
        fixture.scenario_path.join("Teams.txt"),
        b"[Teams]\nLastTeamID=2\n\
\n\
\x20\x20[Team]\n\x20\x20id=1\n\x20\x20Name=One\n\
\n\
\x20\x20[Team]\n\x20\x20id=2\n\x20\x20Name=Two\n\x20\x20PlayerCount=1\n\x20\x20Players=3\n",
    )
    .unwrap();

    let prepared = prepare(&fixture, &[]).expect("ordinary restore scenario is hostable");
    let host = prepared.host_config();
    let snapshot = host
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .clone();
    let restore = &snapshot.parameters.restore_player_infos;
    assert_eq!(restore.last_player_id, 9);
    assert_eq!(restore.clients.len(), 1);
    assert_eq!(restore.clients[0].client_id, 5);
    assert_eq!(
        restore.clients[0].players[0].name.as_bytes(),
        b"Localized Script"
    );
    assert_eq!(
        restore.clients[0].players[0].forced_name.as_bytes(),
        b"Raw\x81",
        "localization preserves undefined native bytes"
    );
    assert_eq!(restore.clients[0].players[0].savegame_player, 0);

    assert_eq!(snapshot.parameters.player_infos.last_player_id, 9);
    let host_script = &snapshot.parameters.player_infos.clients[0].players[0];
    assert_eq!((host_script.id, host_script.savegame_player), (3, 3));
    assert_eq!(host_script.name.as_bytes(), b"Localized Script");
    assert!(host_script.is_script_player());
    assert!(host_script.is_joined());
    assert_eq!(
        snapshot
            .parameters
            .teams
            .teams
            .iter()
            .find(|team| team.id == 2)
            .expect("restore team")
            .player_ids,
        vec![3]
    );

    let wire = clonk_network::encode_join_data_envelope(&clonk_network::JoinDataEnvelope {
        client_id: 0,
        start_control_tick: snapshot.dynamic_tick,
        status: host.initial_status,
        dynamic: snapshot.dynamic.clone(),
        parameters: snapshot.parameters.clone(),
    })
    .expect("JoinData encodes");
    let decoded = clonk_network::decode_join_data_envelope(&wire).expect("JoinData decodes");
    assert_eq!(
        decoded.parameters.restore_player_infos,
        snapshot.parameters.restore_player_infos
    );
    assert_eq!(
        decoded.parameters.player_infos,
        snapshot.parameters.player_infos
    );

    let mut registry = clonk_engine::ControlPlayerInfoRegistry::default();
    let _ = prepared
        .install_initial_host_player_state(&mut registry, |_, _| {})
        .expect("install host restore state");
    assert_eq!(registry.recreation_players(), vec![(0, 3)]);
    let next = registry
        .admit_request(
            clonk_engine::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
            },
            2,
        )
        .expect("one ordinary player slot remains");
    assert_eq!(
        next.players[0].id, 10,
        "restore LastPlayerID seeds allocation"
    );
}

#[test]
fn old_save_definition_files_override_remains_a_typed_host_boundary() {
    let fixture = minimal_install(Some(b"[DefinitionFiles]\nDefinition1=Historical.c4d\n"));
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replacen("[Head]\n", "[Head]\nSaveGame=1\n", 1),
    )
    .unwrap();

    let error = prepare(&fixture, &[]).expect_err("old DefinitionFiles must not publish raw defs");
    assert!(matches!(
        error,
        PrepareHostBootstrapError::SavegameDefinitionOverrideUnsupported
    ));
}

#[test]
fn embedded_parameters_and_ignored_rosters_prepare_before_typed_runtime_guards() {
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("MaxPlayer=2", "MaxPlayer=2\nMaxPlayerLeague=5"),
    )
    .unwrap();

    fs::write(
        fixture.scenario_path.join("Parameters.txt"),
        b"[Parameters]\n\
RandomSeed=7\n\
StartupPlayerCount=3\n\
MaxPlayers=5\n\
UseFairCrew=1\n\
FairCrewForced=1\n\
FairCrewStrength=1234\n\
AllowDebug=0\n\
IsNetworkGame=1\n\
ControlRate=4\n\
AutoFrameSkip=1\n\
Rules=RULE=2\n\
Goals=GOAL=3\n\
League=Embedded Cup\n\
\n\
\x20\x20[Client]\n\
\x20\x20ID=9\n\
\x20\x20Activated=1\n\
\x20\x20Name=Saved Client\n",
    )
    .unwrap();
    let embedded = prepare(&fixture, &[]).expect("embedded Parameters.txt compiles");
    let parameters = &embedded
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters;
    assert_eq!(parameters.random_seed, 7);
    assert_eq!(parameters.startup_player_count, 3);
    assert_eq!(parameters.max_players, 5);
    assert!(parameters.use_fair_crew);
    assert!(parameters.fair_crew_forced);
    assert_eq!(parameters.fair_crew_strength, 1234);
    assert!(!parameters.allow_debug);
    assert!(parameters.is_network_game);
    assert_eq!(parameters.control_rate, 4);
    assert!(parameters.auto_frame_skip);
    assert_eq!(parameters.rules[0].id.as_bytes(), b"RULE");
    assert_eq!(parameters.rules[0].count, 2);
    assert_eq!(parameters.goals[0].id.as_bytes(), b"GOAL");
    assert_eq!(parameters.goals[0].count, 3);
    assert!(
        parameters.league.is_empty(),
        "InitLeague clears embedded League"
    );
    assert_eq!(parameters.clients.clients.len(), 1);
    assert_eq!(parameters.clients.clients[0].client_id, 0);
    assert_eq!(parameters.clients.clients[0].name.as_bytes(), b"Host Name");

    let dynamic_file = embedded
        .host_config()
        .resource_files
        .last()
        .expect("embedded dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).expect("open embedded dynamic");
    let dynamic_parameters = dynamic.read_file("Parameters.txt").unwrap();
    assert!(dynamic_parameters
        .windows(b"League=\"Embedded Cup\"".len())
        .any(|window| window == b"League=\"Embedded Cup\""));
    assert!(!dynamic_parameters
        .windows(b"Saved Client".len())
        .any(|window| window == b"Saved Client"));
    fs::remove_file(fixture.scenario_path.join("Parameters.txt")).unwrap();

    fs::write(fixture.scenario_path.join("Parameters.txt"), b"").unwrap();
    let empty = prepare(&fixture, &[]).expect("zero-byte Parameters.txt is absent to C++");
    let parameters = &empty
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .unwrap()
        .parameters;
    assert_eq!(parameters.random_seed, 1_700_000_000);
    assert_eq!(parameters.control_rate, 1);
    assert!(parameters.is_network_game);
    assert_eq!(parameters.max_players, 2);
    fs::remove_file(fixture.scenario_path.join("Parameters.txt")).unwrap();
    fs::write(fixture.scenario_path.join("Game.txt"), b"[Game]\nTime=1\n").unwrap();
    let runtime = prepare(&fixture, &[]).expect("ordinary Game.txt runtime compiles");
    assert!(runtime.has_initial_game_data());
    assert_eq!(runtime.initial_game_data().time, 1);

    // C4Game compiles all Game.txt sections before the initial network save;
    // the compatibility tail only controls what is re-appended afterwards.
    // A runtime section after the first [Player marker is still compiled;
    // the initial-save hack merely re-appends the complete player tail.
    fs::write(
        fixture.scenario_path.join("Game.txt"),
        b"[Player1]\nName=Clonk\n\n[Game]\nTime=1\n",
    )
    .unwrap();
    let runtime_after_player =
        prepare(&fixture, &[]).expect("runtime section after Player tail compiles");
    assert!(runtime_after_player.has_initial_game_data());
    assert_eq!(runtime_after_player.initial_game_data().time, 1);

    fs::remove_file(fixture.scenario_path.join("Game.txt")).unwrap();

    fs::write(
        fixture.scenario_path.join("SavePlayerInfos.txt"),
        b"[Wrong]\nLastPlayerID=7\n",
    )
    .unwrap();
    let malformed_restore =
        prepare(&fixture, &[]).expect("native logs malformed restore infos and continues");
    let restore = &malformed_restore
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters
        .restore_player_infos;
    assert_eq!((restore.last_player_id, restore.clients.len()), (0, 0));
    fs::remove_file(fixture.scenario_path.join("SavePlayerInfos.txt")).unwrap();

    fs::write(
        fixture.scenario_path.join("PlayerInfos.txt"),
        b"[PlayerInfoList]\n",
    )
    .unwrap();
    prepare(&fixture, &[]).expect("non-replay startup ignores PlayerInfos.txt");
    fs::remove_file(fixture.scenario_path.join("PlayerInfos.txt")).unwrap();

    // An active Teams.txt without entries enables C++'s generated-team path.
    // Initial local players must run the real injected assignment oracle.
    fs::write(
        fixture.scenario_path.join("Teams.txt"),
        b"[Teams]\nActive=1\n",
    )
    .unwrap();
    let player_path = fixture.install_roots[0].join("Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(player_path.join("Player.txt"), b"[Player]\nName=Alice\n").unwrap();
    let generated = prepare(&fixture, &[player_source(player_path, b"Alice.c4p")])
        .expect("generated teams admit configured participants");
    let parameters = &generated
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .unwrap()
        .parameters;
    assert_eq!(parameters.player_infos.clients[0].players[0].team, 1);
    assert_eq!(parameters.teams.teams[0].id, 1);
    fs::remove_file(fixture.scenario_path.join("Teams.txt")).unwrap();

    // The app hands preparation final C4ClientCore identity bytes. Enforce the
    // resulting fixed buffer, but do not demand another validation pass:
    // malformed nested markup can legitimately survive C++'s finite passes.
    assert!(matches!(
        prepare_with_names(
            &fixture,
            &[],
            "1234567890123456789012345678901",
            "Host Nick",
            "netpuncher.openclonk.org:11115",
        ),
        Err(PrepareHostBootstrapError::UnsupportedText {
            field: "host network name"
        })
    ));
    let literal_unknown_tag = prepare_with_names(
        &fixture,
        &[],
        "Literal<future>",
        "Host Nick",
        "netpuncher.openclonk.org:11115",
    )
    .expect("unknown opening markup remains a canonical literal");
    assert_eq!(
        literal_unknown_tag.host_config().local_core.name.as_bytes(),
        b"Literal<future>"
    );

    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("[Head]", "[Head]\nSaveGame=1"),
    )
    .unwrap();
    prepare(&fixture, &[]).expect("savegame head is hostable");
}

#[test]
fn embedded_league_name_is_display_only_without_configured_signup() {
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("MaxPlayer=2", "MaxPlayer=7\nMaxPlayerLeague=3"),
    )
    .unwrap();
    fs::write(
        fixture.scenario_path.join("Teams.txt"),
        b"[Teams]\nActive=1\nAllowTeamSwitch=1\n",
    )
    .unwrap();
    fs::write(
        fixture.scenario_path.join("Parameters.txt"),
        b"[Parameters]\n\
MaxPlayers=6\n\
UseFairCrew=0\n\
FairCrewForced=0\n\
FairCrewStrength=321\n\
AllowDebug=1\n\
League=Embedded Cup\n",
    )
    .unwrap();

    let prepared =
        prepare(&fixture, &[]).expect("embedded display league loads without configured signup");
    let snapshot = prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData");
    let parameters = &snapshot.parameters;
    assert_eq!(parameters.max_players, 6);
    assert!(parameters.allow_debug);
    assert!(!parameters.use_fair_crew);
    assert!(!parameters.fair_crew_forced);
    assert_eq!(parameters.fair_crew_strength, 321);
    assert_eq!(parameters.teams.allow_team_switch, 1);
    assert!(
        parameters.league.is_empty(),
        "InitLeague clears the display league after CreateDynamic"
    );
    assert!(parameters.league_address.is_empty());
    assert_eq!(prepared.host_config().max_players, 6);
    assert_eq!(prepared.admission().max_players(), 6);

    let dynamic_file = prepared
        .host_config()
        .resource_files
        .last()
        .expect("dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).expect("open dynamic");
    let source = dynamic.read_file("Parameters.txt").unwrap();
    for expected in [
        b"MaxPlayers=6\r\n".as_slice(),
        b"FairCrewStrength=321\r\n".as_slice(),
        b"League=\"Embedded Cup\"\r\n".as_slice(),
    ] {
        assert!(
            source
                .windows(expected.len())
                .any(|window| window == expected),
            "initial dynamic is missing {}",
            String::from_utf8_lossy(expected).trim()
        );
    }
    for forbidden in [
        b"MaxPlayers=3\r\n".as_slice(),
        b"UseFairCrew=true\r\n".as_slice(),
        b"FairCrewForced=true\r\n".as_slice(),
        b"FairCrewStrength=20000\r\n".as_slice(),
        b"AllowDebug=false\r\n".as_slice(),
    ] {
        assert!(
            !source
                .windows(forbidden.len())
                .any(|window| window == forbidden),
            "embedded display League must not add {}",
            String::from_utf8_lossy(forbidden).trim()
        );
    }
}

#[test]
fn embedded_league_name_preserves_all_compiled_parameter_values() {
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("MaxPlayer=2", "MaxPlayer=8\nMaxPlayerLeague=4"),
    )
    .unwrap();
    fs::write(
        fixture.scenario_path.join("Parameters.txt"),
        b"[Parameters]\n\
MaxPlayers=7\n\
UseFairCrew=0\n\
FairCrewForced=1\n\
FairCrewStrength=777\n\
AllowDebug=1\n\
League=Embedded Cup\n",
    )
    .unwrap();

    let prepared = prepare(&fixture, &[])
        .expect("embedded parameter values survive a display-only league name");
    let parameters = &prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters;
    assert_eq!(parameters.max_players, 7);
    assert!(parameters.allow_debug);
    assert!(!parameters.use_fair_crew);
    assert!(parameters.fair_crew_forced);
    assert_eq!(parameters.fair_crew_strength, 777);
    assert!(parameters.league.is_empty());

    let dynamic_file = prepared
        .host_config()
        .resource_files
        .last()
        .expect("dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).expect("open dynamic");
    let source = dynamic.read_file("Parameters.txt").unwrap();
    assert!(source
        .windows(b"League=\"Embedded Cup\"".len())
        .any(|window| window == b"League=\"Embedded Cup\""));
    assert!(source
        .windows(b"FairCrewStrength=777".len())
        .any(|window| window == b"FairCrewStrength=777"));
    assert!(source
        .windows(b"MaxPlayers=7".len())
        .any(|window| window == b"MaxPlayers=7"));
    assert!(!source
        .windows(b"UseFairCrew=true".len())
        .any(|window| window == b"UseFairCrew=true"));
    assert!(!source
        .windows(b"AllowDebug=false".len())
        .any(|window| window == b"AllowDebug=false"));
}

#[test]
fn savegame_zero_max_uses_restore_count_and_canonicalizes_runtime_game_text() {
    let fixture = minimal_install(Some(
        b"[Game]\nTime=123\nFrame=41\nControlTick=37\nTick2=1\nTick3=2\nTick5=1\nTick10=1\nTick35=6\nTick255=41\nTick500=41\nTick1000=41\n\n\
[Script]\nGlobals=1;i17\nGlobalNamed=1;saved=i23\n\n\
[Sky]\nX=65536\nParX=12\nParY=13\nParMode=1\n\n\
[Effects]\nGlobalEffects=Fog(1,100,7,3,0,FOGG)[1;i5]\n\n\
[Scoreboard]\nRows=1\nCols=1\nDlgShow=1\nCell0_0String=\"Scores\"\nCell0_0Value=-1\n",
    ));
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("MaxPlayer=2", "SaveGame=1\nMinPlayer=3\nMaxPlayer=0"),
    )
    .unwrap();
    fs::write(
        fixture.scenario_path.join("SavePlayerInfos.txt"),
        b"[PlayerInfoList]\n\
LastPlayerID=2\n\
\n\
\x20\x20[Client]\n\
\x20\x20ID=5\n\
\n\
\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20Name=One\n\
\x20\x20\x20\x20ID=1\n\
\x20\x20\x20\x20Type=User\n\
\n\
\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20Name=Two\n\
\x20\x20\x20\x20ID=2\n\
\x20\x20\x20\x20Type=User\n",
    )
    .unwrap();

    let prepared = prepare(&fixture, &[]).expect("savegame restore rows prepare");
    let snapshot = prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData");
    assert_eq!(
        snapshot.parameters.restore_player_infos.clients[0]
            .players
            .len(),
        2
    );
    assert_eq!(snapshot.parameters.max_players, 2);
    assert_eq!(prepared.host_config().max_players, 2);
    assert_eq!(prepared.admission().max_players(), 2);
    assert_eq!(snapshot.dynamic_tick, 37);
    assert_eq!(prepared.host_config().start_tick, 37);
    assert_eq!(prepared.host_config().initial_status.target_tick, 37);
    let reference = prepared
        .initial_host_game_reference(true, &[])
        .expect("savegame reference");
    assert_eq!(reference.metadata().time, 123);
    assert_eq!(reference.metadata().frame, 41);

    let dynamic_file = prepared
        .host_config()
        .resource_files
        .last()
        .expect("dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).expect("open dynamic");
    let game = dynamic.read_file("Game.txt").unwrap();
    assert!(game
        .windows(b"Time=123\r\n".len())
        .any(|window| window == b"Time=123\r\n"));
    assert!(game
        .windows(b"Globals=1;i17\r\n".len())
        .any(|window| window == b"Globals=1;i17\r\n"));
    assert!(game
        .windows(b"GlobalEffects=Fog(1,100,7,3,0,FOGG)[1;i5]\r\n".len())
        .any(|window| window == b"GlobalEffects=Fog(1,100,7,3,0,FOGG)[1;i5]\r\n"));
    assert!(game
        .windows(b"Cell0_0String=\"Scores\"\r\n".len())
        .any(|window| window == b"Cell0_0String=\"Scores\"\r\n"));
    assert!(!game
        .windows(b"MessageBoardCommands=".len())
        .any(|window| window == b"MessageBoardCommands="));
    assert_ne!(game, b"[Game]\nTime=123\n");
}

#[test]
fn old_style_savegame_player_files_restore_rows_and_capacity() {
    let game = b"[PlayerFiles]\r\n\
Player1=Old.c4p\r\n\
Player2=Missing.c4p\r\n\
\r\n\
[Player1]\r\n\
Index=4\r\n";
    let fixture = minimal_install(Some(game));
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("MaxPlayer=2", "SaveGame=1\nMaxPlayer=0"),
    )
    .unwrap();
    let old_player = fixture.scenario_path.join("Old.c4p");
    fs::create_dir_all(&old_player).unwrap();
    fs::write(
        old_player.join("Player.txt"),
        b"[Player]\nName=Old Player\n\n[Preferences]\nColorDw=1193046\n",
    )
    .unwrap();

    let prepared = prepare(&fixture, &[]).expect("old-style savegame prepares");
    let snapshot = prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("old-style JoinData");
    assert_eq!(snapshot.parameters.max_players, 1);
    assert_eq!(prepared.admission().max_players(), 1);
    assert_eq!(snapshot.parameters.restore_player_infos.last_player_id, 1);
    let restore = &snapshot.parameters.restore_player_infos.clients[0];
    assert_eq!(restore.client_id, -1);
    assert_eq!(restore.players.len(), 1, "unreadable old player is skipped");
    assert_eq!(restore.players[0].id, 1);
    assert_eq!(restore.players[0].name.as_bytes(), b"Old Player");
    assert_eq!(restore.players[0].game_number, 4);
    assert!(restore.players[0].is_joined());
    assert_eq!(restore.players[0].color, 0x0012_3456);
}

#[test]
fn non_ascii_localized_scenario_title_prepares_as_native_c4_bytes() {
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("TitleUS.txt"),
        b"US:S\xe4uresee\n",
    )
    .unwrap();
    let prepared = prepare(&fixture, &[]).expect("CP1252 localized title prepares");
    let host = prepared.host_config();
    let snapshot = host
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .clone();
    assert_eq!(snapshot.parameters.title.as_bytes(), b"S\xe4uresee");

    let wire = clonk_network::encode_join_data_envelope(&clonk_network::JoinDataEnvelope {
        client_id: 0,
        start_control_tick: snapshot.dynamic_tick,
        status: host.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    })
    .expect("JoinData encodes");
    let decoded = clonk_network::decode_join_data_envelope(&wire).expect("JoinData decodes");
    assert_eq!(decoded.parameters.title.as_bytes(), b"S\xe4uresee");

    let dynamic_file = host.resource_files.last().expect("dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).expect("open dynamic");
    let scenario = dynamic.read_file("Scenario.txt").unwrap();
    assert!(scenario
        .windows(b"Title=S\xe4uresee\r\n".len())
        .any(|window| window == b"Title=S\xe4uresee\r\n"));
    assert!(!scenario
        .windows(b"Title=S\xc3\xa4uresee\r\n".len())
        .any(|window| window == b"Title=S\xc3\xa4uresee\r\n"));
}

#[test]
fn non_ascii_scenario_head_fallback_prepares_as_native_c4_bytes() {
    let fixture = minimal_install(None);
    let (prefix, suffix) = fixture
        .scenario_text
        .split_once("Title=Fixture")
        .expect("fixture title");
    let mut scenario = prefix.as_bytes().to_vec();
    scenario.extend_from_slice(b"Title=S\xe4uresee");
    scenario.extend_from_slice(suffix.as_bytes());
    fs::write(fixture.scenario_path.join("Scenario.txt"), scenario).unwrap();

    let prepared = prepare(&fixture, &[]).expect("CP1252 Head.Title prepares");
    let host = prepared.host_config();
    let snapshot = host
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData");
    assert_eq!(snapshot.parameters.title.as_bytes(), b"S\xe4uresee");

    let dynamic_file = host.resource_files.last().expect("dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).expect("open dynamic");
    let scenario = dynamic.read_file("Scenario.txt").unwrap();
    assert!(scenario
        .windows(b"Title=S\xe4uresee\r\n".len())
        .any(|window| window == b"Title=S\xe4uresee\r\n"));
}

#[test]
fn native_host_metadata_and_player_filename_prepare_as_c4_bytes() {
    let fixture = minimal_install(None);
    let player_path = fixture.install_roots[0].join("NativePlayer.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(player_path.join("Player.txt"), b"[Player]\nName=Andr\xe9\n").unwrap();
    let player_sources = [PreparedHostPlayerSource::from(player_source(
        player_path,
        b"Spieler-\xe4.c4p",
    ))];
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let definition_resource_paths = vec![fixture.install_roots[0].join("Defs.c4d")];
    let effective_definition_modules = vec![clonk_script::c4_string_from_bytes(b"D\xe4fs.c4d")];
    let definition_resources = freeze_host_definition_resource_sources(
        &definition_resource_paths,
        &fixture.scenario_path,
        &effective_definition_modules,
        false,
        &fixture.install_roots[0],
        "",
    )
    .unwrap();
    let definition_executable_path = format!(
        "{}{}",
        fixture.install_roots[0].display(),
        std::path::MAIN_SEPARATOR
    );

    let prepared = prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &fixture.scenario_path,
        install_roots: &fixture.install_roots,
        definition_resources: &definition_resources,
        effective_definition_modules: &effective_definition_modules,
        initial_definition_modules: &[],
        fixed_definition_modules: Some(&effective_definition_modules),
        selector_definition_root: None,
        definition_executable_path: &definition_executable_path,
        definition_path: "",
        languages: &languages,
        language_packs: &language_packs,
        network_directory: fixture.network.path(),
        network_work_path: "Network",
        start_unix_seconds: 1_699_999_999,
        random_seed_unix_seconds: 1_700_000_000,
        group_maker: "Mäker",
        host_name: "Höst",
        host_nick: "Nïck",
        network_password: "",
        network_comment: "Grüße",
        netpuncher_address: "netpuncher.openclonk.org:11115",
        player_sources: &player_sources,
        config: PreparedHostBootstrapConfig {
            control_mode: 0,
            control_rate: 1,
            async_max_wait: 2,
            fair_crew: false,
            fair_crew_strength: 0,
            auto_frame_skip: false,
            max_load_file_size: 100 * 1024 * 1024,
            no_runtime_join: true,
            enable_upnp: true,
            network_tcp_port: 11_112,
            network_udp_port: 11_113,
        },
        league: None,
    })
    .expect("native host metadata prepares");

    assert_eq!(prepared.host_config().group_maker.as_bytes(), b"M\xe4ker");
    assert_eq!(
        prepared.host_config().local_core.name.as_bytes(),
        b"H\xf6st"
    );
    assert_eq!(
        prepared.host_config().local_core.nick.as_bytes(),
        b"N\xefck"
    );
    assert_eq!(
        prepared.initial_host_player_info_control().players[0]
            .filename
            .as_bytes(),
        b"Spieler-\xe4.c4p"
    );
    let reference = prepared
        .initial_host_game_reference(true, &[])
        .expect("native reference");
    assert_eq!(reference.metadata().comment.as_bytes(), b"Gr\xfc\xdfe");
    let dynamic_resource = prepared
        .host_config()
        .resource_files
        .iter()
        .find(|resource| {
            resource.core.resource_type == clonk_network::HostResourceType::Dynamic as u8
        })
        .expect("dynamic resource");
    let dynamic = Group::open(&dynamic_resource.path).expect("open native-maker dynamic");
    assert_eq!(dynamic.maker_bytes(), Some(b"M\xe4ker".as_slice()));
    assert!(dynamic
        .read_file("Scenario.txt")
        .unwrap()
        .windows(b"Definitions=\"D\xe4fs.c4d\"".len())
        .any(|window| window == b"Definitions=\"D\xe4fs.c4d\""));
    assert_eq!(
        prepared
            .host_config()
            .resource_files
            .iter()
            .find(|resource| {
                resource.core.resource_type == clonk_network::HostResourceType::Definitions as u8
            })
            .unwrap()
            .core
            .filename
            .as_bytes(),
        b"D\xe4fs.c4d"
    );
}

#[test]
fn explicit_empty_netpuncher_address_remains_empty_in_the_reference() {
    // CompileFunc applies DefaultPuncherServer only when the key is absent;
    // InitHost and InitLocal preserve an explicitly empty configured value
    // (pristine 9ffa0a5d src/C4Config.cpp:265;
    // src/C4Network2.cpp:237-238; src/C4Network2Reference.cpp:77-78).
    let fixture = minimal_install(None);
    let prepared = prepare_with_names(&fixture, &[], "Host Name", "Host Nick", "")
        .expect("explicit empty puncher address is representable");
    let reference = prepared
        .initial_host_game_reference(true, &[])
        .expect("empty puncher address builds a reference");

    assert!(reference.metadata().netpuncher_address.is_empty());
}

#[test]
fn one_selected_player_is_published_after_dynamic_and_installed_before_admission() {
    // Players.Init loads local participants only after InitHost created the
    // Dynamic resource, publishes NRT_Player, assigns ID 1, and directly
    // executes Initial PlayerInfo before AllowJoin(true) (pristine 9ffa0a5d
    // src/C4Game.cpp:3867-3876; src/C4Network2.cpp:241-250;
    // src/C4Network2Players.cpp:38-49,78-123,160-239;
    // src/C4PlayerInfo.cpp:70-104,357-395,781-817).
    let fixture = minimal_install(None);
    let player_path = fixture.install_roots[0].join("Players.c4f/Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=Alice\n\n[Preferences]\nColor=3\nColorDw=0\n",
    )
    .unwrap();
    let source = player_source(player_path.clone(), b"Players.c4f/Alice.c4p");

    let prepared = prepare(&fixture, &[source]).expect("one local player is supported");
    let host = prepared.host_config();
    let snapshot = host
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData");
    let control = prepared.initial_host_player_info_control();
    assert_eq!(control.client_id, 0);
    assert_eq!(control.flags, clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL);
    assert_eq!(control.by_client, 0);
    assert_eq!(control.players.len(), 1);
    let player = &control.players[0];
    assert_eq!(player.id, 1);
    assert_eq!(player.player_type, clonk_engine::PLAYER_INFO_TYPE_USER);
    assert_eq!(player.name.as_bytes(), b"Alice");
    assert_eq!(player.filename.as_bytes(), b"Players.c4f/Alice.c4p");
    assert_eq!(player.color, 0x00fc_f41c);
    assert_eq!(player.original_color, 0x00fc_f41c);
    assert_ne!(
        player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );
    let player_core = player.resource.as_ref().expect("player resource core");
    assert_eq!(player_core.resource_type, 3);
    assert_eq!(player_core.id, snapshot.dynamic.id + 1);
    assert_eq!(
        prepared
            .local_player_alternate_colors_by_resource()
            .get(&player_core.id),
        Some(&0),
        "the host retains the local C4PlayerInfo alternate-color default"
    );
    assert_eq!(snapshot.parameters.startup_player_count, 0);
    assert_eq!(snapshot.parameters.player_infos.last_player_id, 1);
    assert_eq!(snapshot.parameters.player_infos.clients.len(), 1);
    assert_eq!(
        snapshot.parameters.player_infos.clients[0].players,
        control.players
    );
    assert!(!snapshot
        .parameters
        .game_resources
        .iter()
        .any(|core| core.id == player_core.id));

    let hosted_player = host
        .resource_files
        .iter()
        .find(|resource| resource.core.id == player_core.id)
        .expect("optimized player standalone");
    assert_ne!(hosted_player.path, player_path);
    let mut installed_resources = Vec::new();
    let mut registry = clonk_engine::ControlPlayerInfoRegistry::default();
    let ready = prepared
        .install_initial_host_player_state(&mut registry, |core, path| {
            installed_resources.push((core.clone(), path.to_path_buf()));
        })
        .expect("resources and Initial PlayerInfo install once");
    assert_eq!(
        installed_resources,
        vec![(player_core.clone(), player_path)]
    );
    assert!(registry.contains_client(0));
    assert_eq!(registry.player_count(), 1);
    assert!(ready.lobby_join_allowed());

    let admitted = registry
        .admit_request(
            clonk_engine::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
            },
            2,
        )
        .expect("the second scenario slot accepts a remote player");
    assert_eq!(
        admitted.players[0].id, 2,
        "runtime assignment must continue after the installed host player"
    );
}

#[test]
fn packed_parent_player_child_is_snapshotted_and_published() {
    let fixture = minimal_install(None);
    let mut player = MutableGroup::new("Alice.c4p");
    player
        .add_file(
            "Player.txt",
            b"[Player]\nName=Packed Alice\n\n[Preferences]\nColor=3\nColorDw=0\n".to_vec(),
        )
        .unwrap();
    let mut parent = MutableGroup::new("Players.c4f");
    parent.add_child("Alice.c4p", player).unwrap();
    let parent_path = fixture.install_roots[0].join("Players.c4f");
    fs::write(&parent_path, parent.pack().unwrap()).unwrap();
    let virtual_player_path = parent_path.join("Alice.c4p");
    assert!(!virtual_player_path.exists());

    let prepared = prepare(
        &fixture,
        &[player_source(
            virtual_player_path.clone(),
            b"Players.c4f/Alice.c4p",
        )],
    )
    .expect("packed player child prepares");

    let player_info = &prepared.initial_host_player_info_control().players[0];
    assert_eq!(player_info.name.as_bytes(), b"Packed Alice");
    let core = player_info
        .resource
        .as_ref()
        .expect("published player core");
    assert_eq!(
        core.resource_type,
        clonk_network::HostResourceType::Player as u8
    );
    let hosted = prepared
        .host_config()
        .resource_files
        .iter()
        .find(|resource| resource.core.id == core.id)
        .expect("materialized packed child");
    assert!(hosted.path.exists());
    assert_ne!(hosted.path, virtual_player_path);
    assert!(hosted.binary_compatible);
    let hosted_path = hosted.path.clone();
    let mut installed = Vec::new();
    let mut registry = clonk_engine::ControlPlayerInfoRegistry::default();
    let _ready = prepared
        .install_initial_host_player_state(&mut registry, |core, path| {
            installed.push((core.id, path.to_path_buf()));
        })
        .expect("install packed child resource and player info");
    assert_eq!(installed, [(core.id, hosted_path.clone())]);
    assert!(hosted_path.exists());
    Group::open(&hosted_path).expect("installed packed child is loadable");
    assert_eq!(registry.player_count(), 1);
}

#[test]
fn configured_player_identity_reaches_initial_host_info_without_generic_reparse() {
    // C4PlayerInfoCore caps PrefName at 30 bytes before stripping markup and
    // uses exact-case INI lookup. The generic PlayerFile representation keeps
    // different raw values, so preparation must carry the configured loader's
    // already-normalized name and default color through NRT_Player
    // publication (pristine 9ffa0a5d src/C4InfoCore.cpp:90-125,146-173;
    // src/C4PlayerInfo.cpp:70-104,357-395).
    let fixture = minimal_install(None);
    let player_path = fixture.install_roots[0].join("Players.c4f/Marked.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=<i>ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789</i>\n\
[Preferences]\ncolor=3\ncolordw=1193046\n",
    )
    .unwrap();
    let selected = PreparedHostPlayerSource {
        resource: player_source(player_path, b"Players.c4f/Marked.c4p"),
        identity: Some(PreparedHostPlayerIdentity {
            // The first 30 source bytes are `<i>` plus 27 visible bytes; the
            // configured loader then strips the opening markup tag.
            player_name: clonk_engine::LegacyCString::from_bytes(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0".to_vec(),
            )
            .unwrap(),
            // Lowercase Color/ColorDw keys are ignored by C++.
            network_color: 0xff,
            alternate_color: 0,
        }),
    };

    let prepared = prepare_typed(&fixture, &[selected]).expect("configured identity prepares");
    let player = &prepared.initial_host_player_info_control().players[0];
    assert_eq!(player.name.as_bytes(), b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0");
    assert_eq!(player.color, 0xff);
    assert_eq!(player.original_color, 0xff);
    assert_eq!(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .player_infos
            .clients[0]
            .players[0]
            .name
            .as_bytes(),
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0"
    );
}

#[test]
fn master_server_only_host_defers_local_player_until_start_response() {
    // C4Network2::InitLeague creates a Start request for master-server signup
    // as well as league signup. Local PlayerInfo admission therefore waits
    // until that response path has had its chance to change MaxPlayers, even
    // when league authentication itself is disabled.
    let fixture = minimal_install(None);

    let player_path = fixture.install_roots[0].join("Players.c4f/Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=Alice\n[Preferences]\nColorDw=1193046\n",
    )
    .unwrap();
    let players = [PreparedHostPlayerSource::from(player_source(
        player_path,
        b"Players.c4f/Alice.c4p",
    ))];
    let league = PreparedLeagueHostConfig {
        endpoint: "https://master.invalid/league.php".to_string(),
        transport: clonk_network::LeagueHttpTransportConfig::default(),
        update_period_secs: 120,
        league_server_signup: false,
    };
    let mut prepared = prepare_typed_with_names_and_league_impl(
        &fixture,
        &players,
        "Host Name",
        "Host Nick",
        "netpuncher.openclonk.org:11115",
        Some(&league),
    )
    .expect("master-server-only host prepares");

    assert!(prepared
        .initial_host_player_info_control()
        .players
        .is_empty());
    let pending = prepared
        .pending_initial_league_players()
        .expect("Start response still gates local admission")
        .to_vec();
    assert_eq!(pending.len(), 1);
    let mut oracle = RecordingInitialHostTeamAssignmentOracle::default();
    assert!(prepared
        .finalize_initial_league_players(pending, &mut oracle, |_| true)
        .expect("Start response finalizes the local player"));
    assert_eq!(prepared.initial_host_player_info_control().players.len(), 1);
    assert_eq!(prepared.initial_host_player_info_control().players[0].id, 1);
    assert!(prepared.pending_initial_league_players().is_none());
}

#[test]
fn selected_native_player_name_is_preserved_in_the_initial_host_packet() {
    let fixture = minimal_install(None);
    let player_path = fixture.install_roots[0].join("Players.c4f/Andre.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(
        player_path.join("Player.txt"),
        [b"[Player]\nName=Andr".as_slice(), &[0xe9], b"\n"].concat(),
    )
    .unwrap();

    let prepared = prepare(
        &fixture,
        &[player_source(player_path, b"Players.c4f/Andre.c4p")],
    )
    .expect("native C4 player names remain valid for the host");
    let player = &prepared.initial_host_player_info_control().players[0];
    assert_eq!(player.name.as_bytes(), b"Andr\xe9");
    assert_eq!(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .player_infos
            .clients[0]
            .players[0]
            .name
            .as_bytes(),
        b"Andr\xe9"
    );
}

#[test]
fn regicide_assigns_the_initial_host_player_before_publishing_join_data() {
    // C++ loads Teams.txt before parameters, snapshots Dynamic before local
    // players exist, then allocates player IDs and uses process-global
    // SafeRandom while assigning the least-used team. Team assignment changes
    // the live color but preserves OriginalColor (pristine 9ffa0a5d
    // src/C4GameParameters.cpp:403-410; src/C4Network2.cpp:249-250;
    // src/C4Network2Players.cpp:189-205; src/C4Teams.cpp:53-81,446-539).
    let fixture = minimal_install(None);
    // Team parsing and assignment depend on the shipped Teams.txt, not on
    // compressing the unrelated Objects/Knights definition trees.
    fs::copy(
        repository_root().join("content/Knights.c4f/Regicide.c4s/Teams.txt"),
        fixture.scenario_path.join("Teams.txt"),
    )
    .unwrap();
    let definition_resource_paths = vec![fixture.install_roots[0].join("Defs.c4d")];
    let effective_definition_modules = vec!["Defs.c4d".to_owned()];
    let definition_resources = freeze_host_definition_resource_sources(
        &definition_resource_paths,
        &fixture.scenario_path,
        &effective_definition_modules,
        false,
        &fixture.install_roots[0],
        "",
    )
    .unwrap();
    let definition_executable_path = format!(
        "{}{}",
        fixture.install_roots[0].display(),
        std::path::MAIN_SEPARATOR
    );
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let player_directory = tempfile::tempdir().unwrap();
    let player_path = player_directory.path().join("Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=Alice\n\n[Preferences]\nColor=3\nColorDw=0\n",
    )
    .unwrap();
    let player_sources = vec![PreparedHostPlayerSource::from(player_source(
        player_path,
        b"Alice.c4p",
    ))];
    let mut oracle = RecordingInitialHostTeamAssignmentOracle::default();

    let prepared = prepare_host_bootstrap_with_team_assignment_oracle(
        PreparedHostBootstrapSpec {
            scenario_path: &fixture.scenario_path,
            install_roots: &fixture.install_roots,
            definition_resources: &definition_resources,
            effective_definition_modules: &effective_definition_modules,
            initial_definition_modules: &[],
            fixed_definition_modules: None,
            selector_definition_root: None,
            definition_executable_path: &definition_executable_path,
            definition_path: "",
            languages: &languages,
            language_packs: &language_packs,
            network_directory: fixture.network.path(),
            network_work_path: "Network",
            start_unix_seconds: 1_720_000_122,
            random_seed_unix_seconds: 1_720_000_123,
            group_maker: "FileMaker",
            host_name: "Host",
            host_nick: "Host",
            network_password: "",
            network_comment: "",
            netpuncher_address: "",
            player_sources: &player_sources,
            config: PreparedHostBootstrapConfig {
                control_mode: 0,
                control_rate: 1,
                async_max_wait: 2,
                fair_crew: false,
                fair_crew_strength: 0,
                auto_frame_skip: false,
                max_load_file_size: 100 * 1024 * 1024,
                no_runtime_join: true,
                enable_upnp: true,
                network_tcp_port: 11_112,
                network_udp_port: 11_113,
            },
            league: None,
        },
        &mut oracle,
    )
    .expect("shipped explicit teams support an initial local host player");

    assert_eq!(oracle.safe_random_ranges, vec![2]);
    assert!(oracle.generated_team_ids.is_empty());
    let control = prepared.initial_host_player_info_control();
    assert_eq!(control.players.len(), 1);
    assert_eq!(control.players[0].id, 1);
    assert_eq!(control.players[0].team, 2);
    assert_eq!(control.players[0].color, 0x0000_c800);
    assert_eq!(control.players[0].original_color, 0x00fc_f41c);
    let snapshot = &prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .unwrap()
        .parameters;
    assert_eq!(snapshot.player_infos.clients[0].players, control.players);
    assert_eq!(snapshot.teams.teams[0].player_ids, Vec::<i32>::new());
    assert_eq!(snapshot.teams.teams[1].player_ids, vec![1]);
}

#[test]
fn selected_players_are_published_and_admitted_in_module_order() {
    // C4ClientPlayerInfos walks every module in PlayerFilenames in order;
    // Network2Players then publishes and admits every successfully loaded
    // entry in that same Initial packet (pristine 9ffa0a5d
    // src/C4PlayerInfo.cpp:357-395;
    // src/C4Network2Players.cpp:38-49,78-123).
    let fixture = minimal_install(None);
    let players = [
        ("Alice", b"Players.c4f/Alice.c4p".as_slice()),
        ("Bob", b"Players.c4f/Bob.c4p".as_slice()),
    ];
    let sources = players
        .iter()
        .map(|(name, wire_name)| {
            let path = fixture.install_roots[0].join(String::from_utf8_lossy(wire_name).as_ref());
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("Player.txt"),
                format!("[Player]\nName={name}\n").as_bytes(),
            )
            .unwrap();
            player_source(path, wire_name)
        })
        .collect::<Vec<_>>();

    let prepared = prepare(&fixture, &sources).expect("all selected players are supported");
    let control = prepared.initial_host_player_info_control();
    assert_eq!(
        control
            .players
            .iter()
            .map(|player| (
                player.id,
                player.name.as_bytes(),
                player.filename.as_bytes(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, b"Alice".as_slice(), b"Players.c4f/Alice.c4p".as_slice()),
            (2, b"Bob".as_slice(), b"Players.c4f/Bob.c4p".as_slice()),
        ]
    );
    let snapshot = &prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters
        .player_infos;
    assert_eq!(snapshot.last_player_id, 2);
    assert_eq!(snapshot.clients[0].players, control.players);

    let mut installed = Vec::new();
    let mut registry = clonk_engine::ControlPlayerInfoRegistry::default();
    let _ready = prepared
        .install_initial_host_player_state(&mut registry, |core, path| {
            installed.push((core.id, path.to_path_buf()));
        })
        .expect("all host players install before admission");
    assert_eq!(
        installed,
        sources
            .iter()
            .zip(control.players.iter())
            .map(|(source, player)| {
                (
                    player.resource.as_ref().expect("published player").id,
                    source.path.clone(),
                )
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(registry.player_count(), 2);
}

#[test]
fn selected_players_resolve_duplicate_names_in_the_initial_host_packet() {
    // HandlePlayerInfoUpdRequest assigns initial teams before resolving the
    // complete packet. Equal-priority players are visited in packet order, so
    // the first duplicate takes the forced-name suffix while the second keeps
    // its original name (src/C4Network2Players.cpp:189-205;
    // src/C4PlayerInfoConflicts.cpp:322-344).
    let fixture = minimal_install(None);
    let players = [
        (b"Players.c4f/First.c4p".as_slice(), 15_990_784),
        (b"Players.c4f/Second.c4p".as_slice(), 244),
    ];
    let sources = players
        .iter()
        .map(|(wire_name, color)| {
            let path = fixture.install_roots[0].join(String::from_utf8_lossy(wire_name).as_ref());
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join("Player.txt"),
                format!("[Player]\nName=Same\n\n[Preferences]\nColorDw={color}\n"),
            )
            .unwrap();
            player_source(path, wire_name)
        })
        .collect::<Vec<_>>();

    let prepared = prepare(&fixture, &sources).expect("duplicate names are resolved");
    let control = prepared.initial_host_player_info_control();
    assert_eq!(control.players.len(), 2);
    assert_ne!(
        control.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
        0,
        "the direct control retains native's transient update marker"
    );
    assert_eq!(control.players[0].name.as_bytes(), b"Same");
    assert_eq!(control.players[0].forced_name.as_bytes(), b"Same (2)");
    assert_eq!(control.players[1].name.as_bytes(), b"Same");
    assert!(control.players[1].forced_name.is_empty());
    let retained = &prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters
        .player_infos
        .clients[0];
    assert_eq!(retained.players, control.players);
    assert_eq!(
        retained.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
        0,
        "retained JoinData clears the transient update marker"
    );
}

#[test]
fn unreadable_selected_player_does_not_hide_later_valid_players() {
    // C4ClientPlayerInfos deletes only the C4PlayerInfo whose module fails
    // LoadFromLocalFile, then continues SGetModule with the next index
    // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:377-395).
    let fixture = minimal_install(None);
    let valid_path = fixture.install_roots[0].join("Players.c4f/Bob.c4p");
    fs::create_dir_all(&valid_path).unwrap();
    fs::write(valid_path.join("Player.txt"), b"[Player]\nName=Bob\n").unwrap();
    let sources = [
        player_source(
            fixture.install_roots[0].join("Players.c4f/Missing.c4p"),
            b"Players.c4f/Missing.c4p",
        ),
        player_source(valid_path.clone(), b"Players.c4f/Bob.c4p"),
    ];

    let prepared = prepare(&fixture, &sources).expect("later valid player remains joinable");
    let players = &prepared.initial_host_player_info_control().players;
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].id, 1);
    assert_eq!(players[0].name.as_bytes(), b"Bob");
    assert_eq!(players[0].filename.as_bytes(), b"Players.c4f/Bob.c4p");
    let dynamic_id = prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .dynamic
        .id;
    assert_eq!(
        players[0].resource.as_ref().expect("published Bob").id,
        dynamic_id + 1,
        "LoadFromLocalFile rejects the missing module before AddByFile allocates an ID"
    );

    let mut installed = Vec::new();
    let mut registry = clonk_engine::ControlPlayerInfoRegistry::default();
    let _ready = prepared
        .install_initial_host_player_state(&mut registry, |_, path| {
            installed.push(path.to_path_buf());
        })
        .expect("valid player installs before admission");
    assert_eq!(installed, vec![valid_path]);
    assert_eq!(registry.player_count(), 1);
}

#[test]
fn initial_host_keeps_the_first_players_that_fit_available_slots() {
    // AssignPlayerIDs removes each excess entry in place, but an Initial
    // packet continues as long as preparation itself succeeded; only an empty
    // AddPlayers packet is rejected (pristine 9ffa0a5d
    // src/C4PlayerInfo.cpp:781-807;
    // src/C4Network2Players.cpp:160-194).
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture.scenario_text.replace("MaxPlayer=2", "MaxPlayer=1"),
    )
    .unwrap();
    let sources = ["Alice", "Bob"]
        .into_iter()
        .map(|name| {
            let wire_name = format!("Players.c4f/{name}.c4p");
            let path = fixture.install_roots[0].join(&wire_name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("Player.txt"), format!("[Player]\nName={name}\n")).unwrap();
            player_source(path, wire_name.as_bytes())
        })
        .collect::<Vec<_>>();

    let prepared = prepare(&fixture, &sources).expect("excess initial players are pruned");
    let players = &prepared.initial_host_player_info_control().players;
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].id, 1);
    assert_eq!(players[0].name.as_bytes(), b"Alice");
    assert_eq!(
        prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
            .player_infos
            .last_player_id,
        1
    );
}

#[test]
fn zero_player_slots_keep_the_empty_initial_packet() {
    // AssignPlayerIDs removes the requested player, but HandlePlayerInfoUpdRequest
    // rejects an empty packet only for CIF_AddPlayers. The host's Initial
    // packet still executes and leaves the local client observing
    // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:781-807;
    // src/C4Network2Players.cpp:160-194,239-243).
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture.scenario_text.replace("MaxPlayer=2", "MaxPlayer=0"),
    )
    .unwrap();
    let player_path = fixture.install_roots[0].join("Players.c4f/Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(player_path.join("Player.txt"), b"[Player]\nName=Alice\n").unwrap();

    let prepared = prepare(
        &fixture,
        &[player_source(player_path, b"Players.c4f/Alice.c4p")],
    )
    .expect("empty Initial player packet remains valid");
    assert!(prepared
        .initial_host_player_info_control()
        .players
        .is_empty());
    let snapshot = &prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters
        .player_infos;
    assert_eq!(snapshot.last_player_id, 0);
    assert!(snapshot.clients[0].players.is_empty());
}

#[test]
fn player_section_tail_is_the_only_original_game_text_preserved() {
    let fixture = minimal_install(Some(b" \r\n[Player1]\r\nName=Clonk\r\n"));
    let prepared = prepare(&fixture, &[]).unwrap();
    let dynamic_file = prepared
        .host_config()
        .resource_files
        .last()
        .expect("dynamic resource");
    let dynamic = Group::open(&dynamic_file.path).unwrap();
    let game = dynamic.read_file("Game.txt").unwrap();

    assert!(game.ends_with(b"[Player1]\r\nName=Clonk\r\n"));
    assert!(!game
        .windows(b"SetGameSpeed(%d)".len())
        .any(|line| { line == b"SetGameSpeed(%d)" }));
}

#[test]
fn preparation_reloads_scenario_metadata_from_the_published_path() {
    let fixture = minimal_install(None);
    let initial = prepare(&fixture, &[]).unwrap();
    assert_eq!(initial.admission().max_players(), 2);

    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture.scenario_text.replace("MaxPlayer=2", "MaxPlayer=7"),
    )
    .unwrap();
    let reloaded = prepare(&fixture, &[]).unwrap();
    assert_eq!(reloaded.admission().max_players(), 7);
    assert_eq!(reloaded.host_config().max_players, 7);
    assert_eq!(
        reloaded
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .unwrap()
            .parameters
            .max_players,
        7
    );
}

fn prepare(
    fixture: &MinimalInstall,
    player_sources: &[HostInitialResourceSource],
) -> Result<prepared_host_bootstrap::PreparedHostBootstrap, PrepareHostBootstrapError> {
    prepare_with_names(
        fixture,
        player_sources,
        "Host Name",
        "Host Nick",
        "netpuncher.openclonk.org:11115",
    )
}

fn prepare_with_names(
    fixture: &MinimalInstall,
    player_sources: &[HostInitialResourceSource],
    host_name: &str,
    host_nick: &str,
    netpuncher_address: &str,
) -> Result<prepared_host_bootstrap::PreparedHostBootstrap, PrepareHostBootstrapError> {
    prepare_with_names_impl(
        fixture,
        player_sources,
        host_name,
        host_nick,
        netpuncher_address,
    )
}

fn prepare_with_names_impl(
    fixture: &MinimalInstall,
    player_sources: &[HostInitialResourceSource],
    host_name: &str,
    host_nick: &str,
    netpuncher_address: &str,
) -> Result<prepared_host_bootstrap::PreparedHostBootstrap, PrepareHostBootstrapError> {
    let player_sources = player_sources
        .iter()
        .cloned()
        .map(PreparedHostPlayerSource::from)
        .collect::<Vec<_>>();
    prepare_typed_with_names_impl(
        fixture,
        &player_sources,
        host_name,
        host_nick,
        netpuncher_address,
    )
}

fn prepare_typed(
    fixture: &MinimalInstall,
    player_sources: &[PreparedHostPlayerSource],
) -> Result<prepared_host_bootstrap::PreparedHostBootstrap, PrepareHostBootstrapError> {
    prepare_typed_with_names_impl(
        fixture,
        player_sources,
        "Host Name",
        "Host Nick",
        "netpuncher.openclonk.org:11115",
    )
}

fn prepare_typed_with_names_impl(
    fixture: &MinimalInstall,
    player_sources: &[PreparedHostPlayerSource],
    host_name: &str,
    host_nick: &str,
    netpuncher_address: &str,
) -> Result<prepared_host_bootstrap::PreparedHostBootstrap, PrepareHostBootstrapError> {
    prepare_typed_with_names_and_league_impl(
        fixture,
        player_sources,
        host_name,
        host_nick,
        netpuncher_address,
        None,
    )
}

fn prepare_typed_with_names_and_league_impl(
    fixture: &MinimalInstall,
    player_sources: &[PreparedHostPlayerSource],
    host_name: &str,
    host_nick: &str,
    netpuncher_address: &str,
    league: Option<&PreparedLeagueHostConfig>,
) -> Result<prepared_host_bootstrap::PreparedHostBootstrap, PrepareHostBootstrapError> {
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let definition_resource_paths = vec![fixture.install_roots[0].join("Defs.c4d")];
    let effective_definition_modules = vec!["Defs.c4d".to_owned()];
    let definition_resources = freeze_host_definition_resource_sources(
        &definition_resource_paths,
        &fixture.scenario_path,
        &effective_definition_modules,
        false,
        &fixture.install_roots[0],
        "",
    )
    .unwrap();
    let definition_executable_path = format!(
        "{}{}",
        fixture.install_roots[0].display(),
        std::path::MAIN_SEPARATOR
    );
    prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &fixture.scenario_path,
        install_roots: &fixture.install_roots,
        definition_resources: &definition_resources,
        effective_definition_modules: &effective_definition_modules,
        initial_definition_modules: &[],
        fixed_definition_modules: None,
        selector_definition_root: None,
        definition_executable_path: &definition_executable_path,
        definition_path: "",
        languages: &languages,
        language_packs: &language_packs,
        network_directory: fixture.network.path(),
        network_work_path: "Network",
        start_unix_seconds: 1_699_999_999,
        random_seed_unix_seconds: 1_700_000_000,
        group_maker: "Fixture Maker",
        host_name,
        host_nick,
        network_password: "",
        network_comment: "",
        netpuncher_address,
        player_sources,
        config: PreparedHostBootstrapConfig {
            control_mode: 0,
            control_rate: 1,
            async_max_wait: 2,
            fair_crew: false,
            fair_crew_strength: 0,
            auto_frame_skip: false,
            max_load_file_size: 100 * 1024 * 1024,
            no_runtime_join: true,
            enable_upnp: true,
            network_tcp_port: 11_112,
            network_udp_port: 11_113,
        },
        league,
    })
}

fn player_source(path: PathBuf, wire_name: &[u8]) -> HostInitialResourceSource {
    HostInitialResourceSource {
        path,
        lookup_name: clonk_engine::LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
        opened_name: clonk_engine::LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
        wire_name: clonk_engine::LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
        virtual_group_bytes: None,
    }
}

struct MinimalInstall {
    _root: tempfile::TempDir,
    network: tempfile::TempDir,
    scenario_path: PathBuf,
    install_roots: Vec<PathBuf>,
    scenario_text: String,
}

fn minimal_install(game: Option<&[u8]>) -> MinimalInstall {
    let root = tempfile::tempdir().unwrap();
    let network = tempfile::tempdir().unwrap();
    let content = root.path().join("content");
    let planet = root.path().join("planet");
    let scenario_path = content.join("Fixture.c4s");
    let definition = content.join("Defs.c4d/Good.c4d");
    fs::create_dir_all(&scenario_path).unwrap();
    fs::create_dir_all(&definition).unwrap();
    fs::create_dir_all(content.join("Material.c4g")).unwrap();
    fs::create_dir_all(planet.join("System.c4g")).unwrap();
    fs::write(
        definition.join("DefCore.txt"),
        b"[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
    )
    .unwrap();
    fs::write(definition.join("Script.c"), b"// fixture\n").unwrap();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))
        .save(definition.join("Graphics.png"))
        .unwrap();
    let scenario_text = "[Head]\nTitle=Fixture\nMaxPlayer=2\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=GOOD=1\n"
        .to_owned();
    fs::write(scenario_path.join("Scenario.txt"), &scenario_text).unwrap();
    if let Some(game) = game {
        fs::write(scenario_path.join("Game.txt"), game).unwrap();
    }
    MinimalInstall {
        _root: root,
        network,
        scenario_path,
        install_roots: vec![content, planet],
        scenario_text,
    }
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[derive(Default)]
struct RecordingInitialHostTeamAssignmentOracle {
    safe_random_ranges: Vec<i32>,
    generated_team_ids: Vec<i32>,
}

impl clonk_engine::InitialHostTeamAssignmentOracle for RecordingInitialHostTeamAssignmentOracle {
    fn safe_random(&mut self, range: i32) -> i32 {
        self.safe_random_ranges.push(range);
        0
    }

    fn generate_team(
        &mut self,
        id: i32,
        _existing_teams: &[clonk_engine::InitialNetworkTeam],
    ) -> clonk_engine::InitialNetworkTeam {
        self.generated_team_ids.push(id);
        clonk_engine::InitialNetworkTeam {
            id,
            name: clonk_engine::LegacyCString::default(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0,
            icon_spec: clonk_engine::LegacyCString::default(),
            max_players: 0,
        }
    }
}
