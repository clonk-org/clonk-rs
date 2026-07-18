use std::fs;
use std::path::{Path, PathBuf};

use lc_network::{HostInitialResourceSource, NetworkAddress, NetworkProtocol, NETWORK_STATE_LOBBY};
use lc_resources::{Group, LanguagePacks};

#[path = "../src/host_game_resource_sources.rs"]
pub mod host_game_resource_sources;
#[path = "../src/prepared_host_bootstrap.rs"]
pub mod prepared_host_bootstrap;

use prepared_host_bootstrap::{
    prepare_host_bootstrap, prepare_host_bootstrap_with_team_assignment_oracle,
    PrepareHostBootstrapError, PreparedHostBootstrapConfig, PreparedHostBootstrapSpec,
    PreparedHostUseError,
};

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
    let install_roots = vec![content, planet];
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let player_sources = Vec::new();

    let prepared = prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &scenario_path,
        scenario_title: "The First Tutorial",
        install_roots: &install_roots,
        languages: &languages,
        language_packs: &language_packs,
        network_directory: network.path(),
        network_work_path: "Network",
        start_unix_seconds: 1_720_000_122,
        random_seed_unix_seconds: 1_720_000_123,
        group_maker: "FileMaker",
        host_name: "NetworkName",
        host_nick: "NetworkNick",
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
        lc_network::NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 2,
            target_tick: 0,
        }
    );
    let tcp_address = NetworkAddress::new(NetworkProtocol::Tcp, "127.0.0.1:11111".parse().unwrap());
    let reference = prepared
        .initial_host_game_reference(true, &[tcp_address])
        .expect("prepared host builds the exact post-admission reference");
    assert_eq!(reference.summary().title, "The First Tutorial");
    assert_eq!(reference.summary().host_name, "NetworkName");
    assert_eq!(reference.summary().host_nick, "NetworkNick");
    assert_eq!(reference.summary().state, "Lobby");
    assert_eq!(reference.summary().control_mode, 2);
    assert_eq!(reference.summary().start_time, 1_720_000_122);
    assert!(reference.summary().join_allowed);
    assert!(!reference.summary().password_needed);
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
        &lc_engine::PlayerInfoControlData {
            client_id: 0,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
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
        prepared.dynamic_wire_name().as_bytes(),
        b"Network/DynTutorial01_2.c4s"
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
            (5, 2, b"Network/DynTutorial01_2.c4s".to_vec()),
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

    let mut control_clients = lc_engine::ControlPlayerInfoRegistry::default();
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
        .find(|resource| resource.ownership == lc_network::ResourceFileOwnership::Temporary)
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

    let wire = lc_network::encode_join_data_envelope(&lc_network::JoinDataEnvelope {
        client_id: 0,
        start_control_tick: snapshot.dynamic_tick,
        status: host.initial_status,
        dynamic: snapshot.dynamic.clone(),
        parameters: snapshot.parameters.clone(),
    })
    .expect("JoinData encodes");
    let decoded = lc_network::decode_join_data_envelope(&wire).expect("JoinData decodes");
    assert_eq!(
        decoded.parameters.restore_player_infos,
        snapshot.parameters.restore_player_infos
    );
    assert_eq!(
        decoded.parameters.player_infos,
        snapshot.parameters.player_infos
    );

    let mut registry = lc_engine::ControlPlayerInfoRegistry::default();
    let _ = prepared
        .install_initial_host_player_state(&mut registry, |_, _| {})
        .expect("install host restore state");
    assert_eq!(registry.recreation_players(), vec![(0, 3)]);
    let next = registry
        .admit_request(
            lc_engine::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![lc_engine::ControlPlayerInfoEntry::default()],
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
fn unsupported_scenario_and_player_inputs_fail_typed_before_publication() {
    let fixture = minimal_install(None);

    fs::write(
        fixture.scenario_path.join("Parameters.txt"),
        b"[Parameters]\nRandomSeed=7\n",
    )
    .unwrap();
    assert!(matches!(
        prepare(&fixture, &[]),
        Err(PrepareHostBootstrapError::ScenarioParametersUnsupported)
    ));
    fs::remove_file(fixture.scenario_path.join("Parameters.txt")).unwrap();

    fs::write(fixture.scenario_path.join("Game.txt"), b"[Game]\nTime=1\n").unwrap();
    assert!(matches!(
        prepare(&fixture, &[]),
        Err(PrepareHostBootstrapError::ScenarioGameStateUnsupported)
    ));

    // C4Game compiles all Game.txt sections before the initial network save;
    // the compatibility tail only controls what is re-appended afterwards.
    // A runtime section after the first [Player marker must not bypass the
    // supported-subset guard (src/C4Game.cpp:1899-1911,2030-2074).
    fs::write(
        fixture.scenario_path.join("Game.txt"),
        b"[Player1]\nName=Clonk\n\n[Game]\nTime=1\n",
    )
    .unwrap();
    assert!(matches!(
        prepare(&fixture, &[]),
        Err(PrepareHostBootstrapError::ScenarioGameStateUnsupported)
    ));
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
    assert!(matches!(
        prepare(&fixture, &[]),
        Err(PrepareHostBootstrapError::ScenarioPlayerInfosUnsupported)
    ));
    fs::remove_file(fixture.scenario_path.join("PlayerInfos.txt")).unwrap();

    // An active Teams.txt without entries enables generated teams. Keep that
    // localization/process-random surface rejected rather than fabricating a
    // team name or color (pristine 9ffa0a5d src/C4Teams.cpp:605-611;
    // src/C4Network2Players.cpp:189-205).
    fs::write(
        fixture.scenario_path.join("Teams.txt"),
        b"[Teams]\nActive=1\n",
    )
    .unwrap();
    assert!(matches!(
        prepare(
            &fixture,
            &[player_source(PathBuf::from("Alice.c4p"), b"Alice.c4p")],
        ),
        Err(PrepareHostBootstrapError::GeneratedPlayerTeamsUnsupported)
    ));
    fs::remove_file(fixture.scenario_path.join("Teams.txt")).unwrap();

    // CMarkup::StripMarkup consumes `}}` pairs even without an opening tag;
    // accepted names must remain byte-stable through C++ validation
    // (pristine 9ffa0a5d src/C4InputValidation.cpp:97-118;
    // src/StdMarkup.cpp:131-164).
    assert!(matches!(
        prepare_with_names(
            &fixture,
            &[],
            "Bad}}Name",
            "Host Nick",
            "netpuncher.openclonk.org:11115",
        ),
        Err(PrepareHostBootstrapError::UnsupportedText {
            field: "host network name"
        })
    ));

    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture
            .scenario_text
            .replace("[Head]", "[Head]\nSaveGame=1"),
    )
    .unwrap();
    assert!(matches!(
        prepare(&fixture, &[]),
        Err(PrepareHostBootstrapError::SavegameUnsupported)
    ));
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
    assert_eq!(control.flags, lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL);
    assert_eq!(control.by_client, 0);
    assert_eq!(control.players.len(), 1);
    let player = &control.players[0];
    assert_eq!(player.id, 1);
    assert_eq!(player.player_type, lc_engine::PLAYER_INFO_TYPE_USER);
    assert_eq!(player.name.as_bytes(), b"Alice");
    assert_eq!(player.filename.as_bytes(), b"Players.c4f/Alice.c4p");
    assert_eq!(player.color, 0x00fc_f41c);
    assert_eq!(player.original_color, 0x00fc_f41c);
    assert_ne!(player.flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
    let player_core = player.resource.as_ref().expect("player resource core");
    assert_eq!(player_core.resource_type, 3);
    assert_eq!(player_core.id, snapshot.dynamic.id + 1);
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
    let mut registry = lc_engine::ControlPlayerInfoRegistry::default();
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
            lc_engine::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![lc_engine::ControlPlayerInfoEntry::default()],
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
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let scenario_path = content.join("Knights.c4f/Regicide.c4s");
    let install_roots = vec![content, planet];
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    let network = tempfile::tempdir().unwrap();
    let player_directory = tempfile::tempdir().unwrap();
    let player_path = player_directory.path().join("Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(
        player_path.join("Player.txt"),
        b"[Player]\nName=Alice\n\n[Preferences]\nColor=3\nColorDw=0\n",
    )
    .unwrap();
    let player_sources = vec![player_source(player_path, b"Alice.c4p")];
    let mut oracle = RecordingInitialHostTeamAssignmentOracle::default();

    let prepared = prepare_host_bootstrap_with_team_assignment_oracle(
        PreparedHostBootstrapSpec {
            scenario_path: &scenario_path,
            scenario_title: "Regicide",
            install_roots: &install_roots,
            languages: &languages,
            language_packs: &language_packs,
            network_directory: network.path(),
            network_work_path: "Network",
            start_unix_seconds: 1_720_000_122,
            random_seed_unix_seconds: 1_720_000_123,
            group_maker: "FileMaker",
            host_name: "Host",
            host_nick: "Host",
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
            let path = fixture
                .install_roots[0]
                .join(String::from_utf8_lossy(wire_name).as_ref());
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
    let mut registry = lc_engine::ControlPlayerInfoRegistry::default();
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
        control.flags & lc_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
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
        retained.flags & lc_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
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

    let mut installed = Vec::new();
    let mut registry = lc_engine::ControlPlayerInfoRegistry::default();
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
    assert!(game
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
    let languages = vec!["US".to_owned(), "DE".to_owned()];
    let language_packs = LanguagePacks::default();
    prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &fixture.scenario_path,
        scenario_title: "Fixture",
        install_roots: &fixture.install_roots,
        languages: &languages,
        language_packs: &language_packs,
        network_directory: fixture.network.path(),
        network_work_path: "Network",
        start_unix_seconds: 1_699_999_999,
        random_seed_unix_seconds: 1_700_000_000,
        group_maker: "Fixture Maker",
        host_name,
        host_nick,
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
        league: None,
    })
}

fn player_source(path: PathBuf, wire_name: &[u8]) -> HostInitialResourceSource {
    HostInitialResourceSource {
        path,
        wire_name: lc_engine::LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
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
        .join("../../..")
        .canonicalize()
        .unwrap()
}

#[derive(Default)]
struct RecordingInitialHostTeamAssignmentOracle {
    safe_random_ranges: Vec<i32>,
    generated_team_ids: Vec<i32>,
}

impl lc_engine::InitialHostTeamAssignmentOracle for RecordingInitialHostTeamAssignmentOracle {
    fn safe_random(&mut self, range: i32) -> i32 {
        self.safe_random_ranges.push(range);
        0
    }

    fn generate_team(
        &mut self,
        id: i32,
        _existing_teams: &[lc_engine::InitialNetworkTeam],
    ) -> lc_engine::InitialNetworkTeam {
        self.generated_team_ids.push(id);
        lc_engine::InitialNetworkTeam {
            id,
            name: lc_engine::LegacyCString::default(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0,
            icon_spec: lc_engine::LegacyCString::default(),
            max_players: 0,
        }
    }
}
