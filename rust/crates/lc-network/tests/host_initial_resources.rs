use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lc_engine::{LegacyCString, NetworkResourceCore};
use lc_network::{
    publish_host_initial_resources, HostConfig, HostInitialResourcePublicationSpec,
    HostInitialResourceSource, InitialNetworkDynamic, InitialNetworkDynamicEntry,
    JoinClientRegistrySnapshot, JoinGameParametersEnvelope, JoinTeamListSnapshot,
    PlayerInfoListSnapshot, ResourceDiscoverPacket, ResourceFileOwnership, ResourcePacket,
    ResourceTransferBackend,
};
use lc_resources::{c4group_file_crc, MutableGroup};

#[test]
fn cpp_host_publication_assigns_ids_fills_join_data_and_registers_system_logically() {
    // InitHost publishes Scenario then the ordered GameRes list and finally
    // Dynamic; nextResID starts at host namespace zero. GameRes is built as
    // Definitions*, System, Material* (pristine 9ffa0a5d
    // src/C4Network2.cpp:222-250,1945-1971;
    // src/C4GameParameters.cpp:192-224,237-246,539-550;
    // src/C4Network2Res.cpp:373-424,1332-1385,1431-1470,1741-1792).
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    fs::create_dir_all(&network).unwrap();
    let scenario = packed_source(
        &sources,
        "actual-scenario.bin",
        "Scenario Maker",
        b"scenario",
    );
    let definition_a = packed_source(&sources, "actual-def-a.bin", "Def A", b"def a");
    let definition_b = packed_source(&sources, "actual-def-b.bin", "Def B", b"def b");
    let system = packed_source(&sources, "actual-system.bin", "System Maker", b"system");
    let material_a = packed_source(&sources, "actual-mat-a.bin", "Mat A", b"mat a");
    let material_b = packed_source(&sources, "actual-mat-b.bin", "Mat B", b"mat b");
    let dynamic = composed_dynamic();
    let expected_dynamic = dynamic.packed_bytes.clone();
    let collision = network.join("DynScenario.c4s");
    fs::write(&collision, b"keep").unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network.clone(),
        group_maker: "OracleHost".to_owned(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario.clone(), b"Missions/Scenario.c4s"),
        definitions: vec![
            source(definition_a.clone(), b"Objects.c4d"),
            source(definition_b.clone(), b"Tutorial.c4f"),
        ],
        system: source(system.clone(), b"System.c4g"),
        materials: vec![
            source(material_a.clone(), b"Folder/Material.c4g"),
            source(material_b.clone(), b"Material.c4g"),
        ],
        dynamic,
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/DynScenario.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 7,
    })
    .unwrap();

    let cores = publication
        .resource_files
        .iter()
        .map(|resource| &resource.core)
        .collect::<Vec<_>>();
    assert_eq!(
        cores
            .iter()
            .map(|core| (core.id, core.resource_type, core.filename.as_bytes()))
            .collect::<Vec<_>>(),
        vec![
            (0, 1, b"Missions/Scenario.c4s".as_slice()),
            (1, 4, b"Objects.c4d".as_slice()),
            (2, 4, b"Tutorial.c4f".as_slice()),
            (3, 5, b"System.c4g".as_slice()),
            (4, 6, b"Folder/Material.c4g".as_slice()),
            (5, 6, b"Material.c4g".as_slice()),
            (6, 2, b"Network/DynScenario_2.c4s".as_slice()),
        ]
    );
    assert!(!cores[3].loadable);
    assert!(cores
        .iter()
        .enumerate()
        .all(|(index, core)| index == 3 || core.loadable));
    assert_eq!(
        publication
            .resource_registrations
            .iter()
            .map(|registration| {
                (
                    registration.resource_id,
                    registration.binary_compatible,
                    registration.chunk_count,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (0, true, 1),
            (1, true, 1),
            (2, true, 1),
            (3, false, 0),
            (4, true, 1),
            (5, true, 1),
            (6, true, 1),
        ]
    );

    assert_eq!(publication.join_snapshot.parameters.scenario.id, 0);
    assert_eq!(
        publication
            .join_snapshot
            .parameters
            .game_resources
            .iter()
            .map(|core| core.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(publication.join_snapshot.dynamic.id, 6);
    assert_eq!(publication.join_snapshot.dynamic_tick, 7);

    let dynamic_file = &publication.resource_files[6];
    assert_eq!(dynamic_file.ownership, ResourceFileOwnership::Temporary);
    assert_eq!(dynamic_file.path.file_name().unwrap(), "DynScenario_2.c4s");
    assert_eq!(fs::read(&dynamic_file.path).unwrap(), expected_dynamic);
    assert_eq!(fs::read(collision).unwrap(), b"keep");

    let mut backend = ResourceTransferBackend::new(0, directory.path().join("backend")).unwrap();
    for resource in &publication.resource_files {
        backend
            .register_hosted_resource(
                resource.core.clone(),
                &resource.path,
                resource.ownership,
                resource.binary_compatible,
            )
            .unwrap();
    }
    assert_eq!(
        backend.catalog().discovery_packet().resource_ids,
        vec![6, 5, 4, 3, 2, 1, 0]
    );
    assert_eq!(backend.core(3), Some(cores[3]));
    assert_eq!(backend.path(3), Some(system.as_path()));
    assert!(backend.catalog().contains_resource(3));
    let events = backend
        .on_packet(
            2,
            &ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![3],
            }),
            0,
            &mut |_| 0,
        )
        .unwrap();
    assert!(
        events.is_empty(),
        "unloadable System is logical, not served"
    );

    let mut host = HostConfig::default();
    publication.apply_to(&mut host);
    assert_eq!(host.resource_directory.as_deref(), Some(network.as_path()));
    assert_eq!(host.resource_files.len(), 7);
    assert_eq!(host.initial_join_snapshot.as_ref().unwrap().dynamic.id, 6);
}

fn source(path: PathBuf, wire_name: &[u8]) -> HostInitialResourceSource {
    HostInitialResourceSource {
        path,
        wire_name: LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
    }
}

fn packed_source(directory: &Path, filename: &str, maker: &str, payload: &[u8]) -> PathBuf {
    let path = directory.join(filename);
    let mut group = MutableGroup::new(filename);
    group.set_maker(maker);
    group
        .add_file_with_metadata("Data.bin", payload.to_vec(), 1, false)
        .unwrap();
    fs::write(&path, group.pack().unwrap()).unwrap();
    path
}

fn composed_dynamic() -> InitialNetworkDynamic {
    let payloads = [
        ("Scenario.txt", b"scenario".as_slice()),
        ("Game.txt", b"game".as_slice()),
        ("Parameters.txt", b"parameters".as_slice()),
    ];
    let mut group = MutableGroup::new("DynScenario.c4s");
    group.set_maker("OracleHost");
    for (name, payload) in payloads {
        group
            .add_file_with_metadata(name, payload.to_vec(), 1, false)
            .unwrap();
    }
    let entries = payloads
        .into_iter()
        .map(|(name, payload)| InitialNetworkDynamicEntry {
            name,
            payload: payload.to_vec(),
            contents_crc: group.entry_crc(name).unwrap(),
        })
        .collect();
    let contents_crc = group.contents_crc();
    let packed_bytes = group.pack().unwrap();
    InitialNetworkDynamic {
        group_filename: "DynScenario.c4s".to_owned(),
        maker: b"OracleHost".to_vec(),
        file_size: packed_bytes.len() as u32,
        file_crc: c4group_file_crc(&packed_bytes),
        packed_bytes,
        contents_crc,
        entries,
    }
}

fn base_parameters() -> JoinGameParametersEnvelope {
    let empty_players = PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    };
    JoinGameParametersEnvelope {
        random_seed: 123,
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
        league_address: LegacyCString::default(),
        title: LegacyCString::from_bytes(b"Scenario".to_vec()).unwrap(),
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
            clients: Vec::new(),
            local_client_id: Some(0),
        },
    }
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "legacyclonk-host-initial-resources-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
