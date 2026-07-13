use std::fs;
use std::path::{Path, PathBuf};

use lc_network::{HostInitialResourceSource, NetworkAddress, NetworkProtocol, NETWORK_STATE_LOBBY};
use lc_resources::Group;

#[path = "../src/host_game_resource_sources.rs"]
pub mod host_game_resource_sources;
#[path = "../src/prepared_host_bootstrap.rs"]
pub mod prepared_host_bootstrap;

use prepared_host_bootstrap::{
    prepare_host_bootstrap, PrepareHostBootstrapError, PreparedHostBootstrapConfig,
    PreparedHostBootstrapSpec, PreparedHostUseError,
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
    let player_sources = Vec::new();

    let prepared = prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &scenario_path,
        scenario_title: "The First Tutorial",
        install_roots: &install_roots,
        languages: &languages,
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
            fair_crew: true,
            fair_crew_strength: 4_321,
            auto_frame_skip: true,
            max_load_file_size: 100 * 1024 * 1024,
            no_runtime_join: true,
        },
    })
    .unwrap();

    let host = prepared.host_config();
    assert!(!host.allow_join);
    assert_eq!(host.start_tick, 0);
    assert_eq!(prepared.start_time(), 1_720_000_122);
    assert_eq!(host.max_players, 1);
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
    assert_eq!(
        reference.summary().tcp_addresses,
        vec![tcp_address.endpoint]
    );
    assert_eq!(reference.metadata().icon, 2);
    assert_eq!(reference.metadata().comment.as_bytes(), b" Host comment ");
    assert_eq!(reference.metadata().addresses, vec![tcp_address]);
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
        b"[PlayerInfoList]\n",
    )
    .unwrap();
    assert!(matches!(
        prepare(&fixture, &[]),
        Err(PrepareHostBootstrapError::RestorePlayerInfosUnsupported)
    ));
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

    assert!(matches!(
        prepare(
            &fixture,
            &[
                player_source(PathBuf::from("Alice.c4p"), b"Alice.c4p"),
                player_source(PathBuf::from("Bob.c4p"), b"Bob.c4p"),
            ],
        ),
        Err(PrepareHostBootstrapError::MultipleLocalPlayerFilesUnsupported { count: 2 })
    ));

    // An active Teams.txt selects C4TeamList's custom-team path; keep that
    // larger player/team assignment surface outside this bounded slice rather
    // than silently producing different assignments (pristine
    // 9ffa0a5d src/C4Team.cpp:667-720; src/C4Network2Players.cpp:78-123).
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
        Err(PrepareHostBootstrapError::LocalPlayerTeamsUnsupported)
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
fn rejected_local_player_admission_removes_published_temporary_files() {
    // AssignPlayerIDs removes a requested player when MaxPlayers has no free
    // startup slot; failed host initialization then closes its resources
    // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:781-817;
    // src/C4Game.cpp:3867-3876; src/C4Network2Res.cpp:360-371).
    let fixture = minimal_install(None);
    fs::write(
        fixture.scenario_path.join("Scenario.txt"),
        fixture.scenario_text.replace("MaxPlayer=2", "MaxPlayer=0"),
    )
    .unwrap();
    let player_path = fixture.install_roots[0].join("Players.c4f/Alice.c4p");
    fs::create_dir_all(&player_path).unwrap();
    fs::write(player_path.join("Player.txt"), b"[Player]\nName=Alice\n").unwrap();

    assert!(matches!(
        prepare(
            &fixture,
            &[player_source(player_path, b"Players.c4f/Alice.c4p")],
        ),
        Err(PrepareHostBootstrapError::LocalPlayerAdmissionRejected)
    ));
    assert_eq!(
        fs::read_dir(fixture.network.path()).unwrap().count(),
        0,
        "failed preparation must not leak optimized player/dynamic files"
    );
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
    prepare_host_bootstrap(PreparedHostBootstrapSpec {
        scenario_path: &fixture.scenario_path,
        scenario_title: "Fixture",
        install_roots: &fixture.install_roots,
        languages: &languages,
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
            fair_crew: false,
            fair_crew_strength: 0,
            auto_frame_skip: false,
            max_load_file_size: 100 * 1024 * 1024,
            no_runtime_join: true,
        },
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
