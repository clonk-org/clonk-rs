use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clonk_engine::{LegacyCString, NetworkResourceCore};
use clonk_network::{
    publish_host_initial_resources, HostConfig, HostInitialResourcePublicationSpec,
    HostInitialResourceSource, HostResourceType, InitialNetworkDynamic, InitialNetworkDynamicEntry,
    JoinClientRegistrySnapshot, JoinGameParametersEnvelope, JoinTeamListSnapshot,
    PlayerInfoListSnapshot, ResourceDiscoverPacket, ResourceFileOwnership, ResourcePacket,
    ResourceTransferBackend,
};
use clonk_resources::{c4group_file_crc, MutableGroup};
use sha1::{Digest, Sha1};

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
    let player = packed_source(&sources, "actual-player.c4p", "Player Maker", b"player");
    let original_player_bytes = fs::read(&player).unwrap();
    let dynamic = composed_dynamic();
    let expected_dynamic = dynamic.packed_bytes.clone();
    let collision = network.join("DynScenario.c4s");
    fs::write(&collision, b"keep").unwrap();
    let mut parameters = base_parameters();
    parameters.league = LegacyCString::from_bytes(b"Display only".to_vec()).unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network.clone(),
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
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
        players: vec![source(player.clone(), b"Players.c4f/Alice.c4p")],
        dynamic,
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/DynScenario.c4s".to_vec()).unwrap(),
        parameters,
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
            (7, 3, b"Players.c4f/Alice.c4p".as_slice()),
        ]
    );
    assert!(!cores[3].loadable);
    assert!(cores
        .iter()
        .enumerate()
        .all(|(index, core)| index == 3 || core.loadable));
    assert!(cores.iter().all(|core| core.file_sha.is_none()));
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
            (7, true, 1),
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
    assert_eq!(publication.player_cores.len(), 1);
    assert_eq!(publication.player_cores[0].id, 7);
    assert_eq!(publication.player_cores[0].resource_type, 3);
    assert!(!publication
        .join_snapshot
        .parameters
        .game_resources
        .iter()
        .any(|core| core.id == 7));

    let dynamic_file = &publication.resource_files[6];
    assert_eq!(dynamic_file.ownership, ResourceFileOwnership::Temporary);
    assert_eq!(dynamic_file.path.file_name().unwrap(), "DynScenario_2.c4s");
    assert_eq!(fs::read(&dynamic_file.path).unwrap(), expected_dynamic);
    assert_eq!(fs::read(collision).unwrap(), b"keep");
    let player_file = &publication.resource_files[7];
    assert_eq!(player_file.ownership, ResourceFileOwnership::Temporary);
    assert_ne!(player_file.path, player);
    assert_eq!(fs::read(&player).unwrap(), original_player_bytes);

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
        vec![7, 6, 5, 4, 3, 2, 1, 0]
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
    assert_eq!(host.resource_files.len(), 8);
    assert_eq!(host.initial_join_snapshot.as_ref().unwrap().dynamic.id, 6);
    assert_eq!(
        host.player_resource_sources,
        vec![(player, host.resource_files[7].core.clone())]
    );
}

#[test]
fn cpp_host_publication_reuses_network_core_for_repeated_game_resource_file() {
    // C4GameResList preserves repeated logical entries, but AddByFile returns
    // the already-published resource for the same file before allocating a new
    // ID (pristine 9ffa0a5d src/C4GameParameters.cpp:192-224,237-246;
    // src/C4Network2Res.cpp:1414-1419,1443-1449).
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let definition = packed_source(&sources, "definitions.bin", "Defs", b"definitions");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let material = packed_source(&sources, "material.bin", "Material", b"material");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![
            source(definition.clone(), b"Objects.c4d"),
            source(definition, b"Objects.c4d"),
        ],
        system: source(system, b"System.c4g"),
        materials: vec![
            source(material.clone(), b"Material.c4g"),
            source(material, b"Material.c4g"),
        ],
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(
        publication
            .resource_files
            .iter()
            .map(|resource| resource.core.id)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(
        publication
            .join_snapshot
            .parameters
            .game_resources
            .iter()
            .map(|core| core.id)
            .collect::<Vec<_>>(),
        vec![1, 1, 2, 3, 3]
    );
    assert_eq!(publication.join_snapshot.dynamic.id, 4);
}

#[test]
fn packed_cross_type_source_reuses_the_definition_core() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let shared = packed_source(&sources, "shared.bin", "Shared", b"shared");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![source(shared.clone(), b"System.c4g")],
        system: source(shared, b"System.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    let resources = &publication.join_snapshot.parameters.game_resources;
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].id, resources[1].id);
    assert!(resources
        .iter()
        .all(|core| core.resource_type == HostResourceType::Definitions as u8));
}

#[test]
fn packed_physical_directory_rewrites_the_reuse_key() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let shared = sources.join("Shared.c4g");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("Payload.txt"), b"directory payload").unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![
            source(shared.clone(), b"Shared.c4g"),
            source(shared.clone(), b"Shared.c4g"),
            source_with_names(
                shared.clone(),
                b"Network/Shared.c4g",
                b"Network/Shared.c4g",
                b"LogicalAlias.c4g",
            ),
        ],
        system: source(shared, b"Shared.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(
        publication
            .join_snapshot
            .parameters
            .game_resources
            .iter()
            .map(|core| (core.id, core.resource_type))
            .collect::<Vec<_>>(),
        vec![
            (1, HostResourceType::Definitions as u8),
            (2, HostResourceType::Definitions as u8),
            (1, HostResourceType::Definitions as u8),
            (3, HostResourceType::System as u8),
        ]
    );
}

#[test]
fn post_pack_over_limit_directory_retains_logical_key_and_temporary_file() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("cache");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let definition = sources.join("Empty.c4d");
    fs::create_dir_all(&definition).unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 1,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![
            source(definition.clone(), b"Empty.c4d"),
            source_with_names(
                definition,
                b"Network/Empty.c4d",
                b"Network/Empty.c4d",
                b"Alias.c4d",
            ),
        ],
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    let resources = &publication.join_snapshot.parameters.game_resources;
    assert_eq!(resources[0].id, resources[1].id);
    assert!(!resources[0].loadable);
    let hosted = publication
        .resource_files
        .iter()
        .find(|resource| resource.core.id == resources[0].id)
        .expect("unloadable packed directory remains hosted for lifetime cleanup");
    assert_eq!(hosted.ownership, ResourceFileOwnership::Temporary);
    assert!(!hosted.binary_compatible);
    assert!(hosted.path.exists());
}

#[test]
fn same_source_path_with_a_distinct_wire_name_publishes_a_distinct_core() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let definition = packed_source(&sources, "definitions.bin", "Defs", b"definitions");
    let system = packed_source(&sources, "system.bin", "System", b"system");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![
            source(definition.clone(), b"First.c4d"),
            source(definition.clone(), b"First.c4d"),
            source(definition, b"Alias.c4d"),
        ],
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(
        publication
            .resource_files
            .iter()
            .map(|resource| (resource.core.id, resource.core.filename.as_bytes()))
            .collect::<Vec<_>>(),
        vec![
            (0, b"Scenario.c4s".as_slice()),
            (1, b"First.c4d".as_slice()),
            (2, b"Alias.c4d".as_slice()),
            (3, b"System.c4g".as_slice()),
            (4, b"Network/DynScenario.c4s".as_slice()),
        ]
    );
    assert_eq!(
        publication
            .join_snapshot
            .parameters
            .game_resources
            .iter()
            .map(|core| core.id)
            .collect::<Vec<_>>(),
        vec![1, 1, 2, 3]
    );
}

#[test]
fn absolute_opened_source_and_relative_alias_publish_distinct_cores_with_same_wire_name() {
    // AddByFile compares the incoming spelling with each earlier resource's
    // retained group filename. An absolute first spelling therefore does not
    // absorb a later relative alias, even when both open the same file and
    // publish the same core filename. Once the relative spelling is retained,
    // an exact repeat reuses that second core.
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let definition = packed_source(&sources, "definitions.bin", "Defs", b"definitions");
    let absolute_name = definition.to_string_lossy().into_owned().into_bytes();
    let system = packed_source(&sources, "system.bin", "System", b"system");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![
            source_with_names(
                definition.clone(),
                &absolute_name,
                &absolute_name,
                b"Objects.c4d",
            ),
            source(definition.clone(), b"Objects.c4d"),
            source(definition, b"Objects.c4d"),
        ],
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(
        publication
            .resource_files
            .iter()
            .map(|resource| (resource.core.id, resource.core.filename.as_bytes()))
            .collect::<Vec<_>>(),
        vec![
            (0, b"Scenario.c4s".as_slice()),
            (1, b"Objects.c4d".as_slice()),
            (2, b"Objects.c4d".as_slice()),
            (3, b"System.c4g".as_slice()),
            (4, b"Network/DynScenario.c4s".as_slice()),
        ]
    );
    assert_eq!(
        publication
            .join_snapshot
            .parameters
            .game_resources
            .iter()
            .map(|core| core.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 2, 3]
    );
}

#[test]
fn player_lookup_reuses_an_earlier_cross_type_resource_core() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let shared = packed_source(&sources, "shared.bin", "Shared", b"shared");
    let system = packed_source(&sources, "system.bin", "System", b"system");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![source(shared.clone(), b"Shared.c4p")],
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: vec![source(shared.clone(), b"Shared.c4p")],
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(publication.player_cores.len(), 1);
    assert_eq!(publication.player_cores[0].id, 1);
    assert_eq!(
        publication.player_cores[0].resource_type,
        HostResourceType::Definitions as u8
    );
    assert_eq!(
        publication.player_cores[0].filename.as_bytes(),
        b"Shared.c4p"
    );
    assert_eq!(
        publication.player_resource_sources,
        vec![(shared, publication.player_cores[0].clone())]
    );
    assert_eq!(
        publication
            .resource_files
            .iter()
            .map(|resource| resource.core.id)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
}

#[test]
fn exact_player_name_reuses_an_earlier_alias_opened_core() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let player = packed_source(&sources, "player.bin", "Player", b"player");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: Vec::new(),
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: vec![
            source_with_names(
                player.clone(),
                b"Players/P?ayer.c4p",
                b"Players/Player.c4p",
                b"Players/P?ayer.c4p",
            ),
            source(player, b"Players/Player.c4p"),
        ],
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(publication.player_cores.len(), 2);
    assert_eq!(
        publication.player_cores[0].id,
        publication.player_cores[1].id
    );
    assert_eq!(
        publication.player_cores[0].filename.as_bytes(),
        b"Players/P?ayer.c4p"
    );
}

#[test]
fn virtual_group_materialization_sanitizes_the_opened_basename() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let mut virtual_group = MutableGroup::new("Virtual.c4d");
    virtual_group
        .add_file_with_metadata("Data.bin", b"virtual".to_vec(), 1, false)
        .unwrap();
    let virtual_group_bytes = virtual_group.pack().unwrap();
    let virtual_path = sources.join("Parent.c4f/Virtual.c4d");
    let raw_wire_name = b"Folder\\Raw\xff.c4d";
    let opened_name = b"Folder/Opened\xfe.c4d";

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![HostInitialResourceSource {
            path: virtual_path,
            lookup_name: LegacyCString::from_bytes(raw_wire_name.to_vec()).unwrap(),
            opened_name: LegacyCString::from_bytes(opened_name.to_vec()).unwrap(),
            wire_name: LegacyCString::from_bytes(raw_wire_name.to_vec()).unwrap(),
            virtual_group_bytes: Some(virtual_group_bytes.clone()),
        }],
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    let definition = &publication.resource_files[1];
    assert_eq!(definition.core.filename.as_bytes(), raw_wire_name);
    assert_eq!(definition.path.file_name().unwrap(), "Opened_.c4d");
    assert_eq!(definition.ownership, ResourceFileOwnership::Temporary);
    assert_eq!(fs::read(&definition.path).unwrap(), virtual_group_bytes);
}

#[test]
fn virtual_group_materialization_flattens_native_backslashes_before_basename_selection() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let mut virtual_group = MutableGroup::new("Virtual.c4d");
    virtual_group
        .add_file_with_metadata("Data.bin", b"virtual".to_vec(), 1, false)
        .unwrap();
    let virtual_group_bytes = virtual_group.pack().unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![HostInitialResourceSource {
            path: sources.join("Parent.c4f/Virtual.c4d"),
            lookup_name: LegacyCString::from_bytes(b"Lookup.c4d".to_vec()).unwrap(),
            opened_name: LegacyCString::from_bytes(b"C:\\Root\\Opened.c4d".to_vec()).unwrap(),
            wire_name: LegacyCString::from_bytes(b"Lookup.c4d".to_vec()).unwrap(),
            virtual_group_bytes: Some(virtual_group_bytes),
        }],
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: Vec::new(),
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(
        publication.resource_files[1].path.file_name().unwrap(),
        "C__Root_Opened.c4d"
    );
}

#[test]
fn league_initial_publication_hashes_scenario_and_all_game_resources_only() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let definition = packed_source(&sources, "definitions.bin", "Defs", b"definitions");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let material = packed_source(&sources, "material.bin", "Material", b"material");
    let player = packed_source(&sources, "player.c4p", "Player", b"player");
    let mut parameters = base_parameters();
    parameters.league_address =
        LegacyCString::from_bytes(b"https://league.invalid/".to_vec()).unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: vec![source(definition, b"Objects.c4d")],
        system: source(system, b"System.c4g"),
        materials: vec![source(material, b"Material.c4g")],
        players: vec![source(player, b"Player.c4p")],
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters,
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(publication.resource_files.len(), 6);
    for resource in &publication.resource_files[..4] {
        assert_eq!(resource.core.file_sha, Some(file_sha(&resource.path)));
    }
    assert_eq!(publication.resource_files[4].core.file_sha, None);
    assert_eq!(publication.resource_files[5].core.file_sha, None);

    assert_eq!(
        publication.join_snapshot.parameters.scenario,
        publication.resource_files[0].core
    );
    assert_eq!(
        publication.join_snapshot.parameters.game_resources,
        publication.resource_files[1..4]
            .iter()
            .map(|resource| resource.core.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        publication.join_snapshot.dynamic,
        publication.resource_files[4].core
    );
    assert_eq!(
        publication.player_cores,
        vec![publication.resource_files[5].core.clone()]
    );
}

#[test]
fn failed_player_publication_consumes_its_reserved_resource_id() {
    // AddByFile increments nextResID before SetByFile/GetStandalone can fail;
    // C4ClientPlayerInfos then drops only that player and continues with the
    // next module (pristine 9ffa0a5d src/C4Network2Res.cpp:1451-1465;
    // src/C4PlayerInfo.cpp:377-395).
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let valid_player = packed_source(&sources, "valid.c4p", "Player", b"player");
    let missing_player = sources.join("missing.c4p");

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: Vec::new(),
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: vec![
            source(missing_player, b"Missing.c4p"),
            source(valid_player, b"Valid.c4p"),
        ],
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(publication.join_snapshot.dynamic.id, 2);
    assert_eq!(publication.player_cores.len(), 1);
    assert_eq!(publication.player_cores[0].id, 4);
    assert_eq!(
        publication
            .resource_files
            .iter()
            .map(|resource| resource.core.id)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 4]
    );
}

#[test]
fn failed_virtual_player_materialization_is_skipped_with_an_id_hole() {
    let directory = TestDirectory::new();
    let sources = directory.path().join("sources");
    let network = directory.path().join("Network");
    fs::create_dir_all(&sources).unwrap();
    fs::create_dir_all(&network).unwrap();
    for suffix in 1..=999 {
        let filename = if suffix == 1 {
            "Packed.c4p".to_owned()
        } else {
            format!("Packed_{suffix}.c4p")
        };
        fs::write(network.join(filename), b"occupied").unwrap();
    }
    let scenario = packed_source(&sources, "scenario.bin", "Scenario", b"scenario");
    let system = packed_source(&sources, "system.bin", "System", b"system");
    let valid_player = packed_source(&sources, "valid.c4p", "Player", b"player");
    let mut packed_player = MutableGroup::new("Packed.c4p");
    packed_player
        .add_file("Player.txt", b"[Player]\nName=Packed\n".to_vec())
        .unwrap();

    let publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"OracleHost".to_vec()).unwrap(),
        max_load_file_size: 100 * 1024 * 1024,
        scenario: source(scenario, b"Scenario.c4s"),
        definitions: Vec::new(),
        system: source(system, b"System.c4g"),
        materials: Vec::new(),
        players: vec![
            HostInitialResourceSource {
                path: sources.join("Parent.c4f/Packed.c4p"),
                lookup_name: LegacyCString::from_bytes(b"Packed.c4p".to_vec()).unwrap(),
                opened_name: LegacyCString::from_bytes(b"Packed.c4p".to_vec()).unwrap(),
                wire_name: LegacyCString::from_bytes(b"Packed.c4p".to_vec()).unwrap(),
                virtual_group_bytes: Some(packed_player.pack().unwrap()),
            },
            source(valid_player, b"Valid.c4p"),
        ],
        dynamic: composed_dynamic(),
        dynamic_wire_name: LegacyCString::from_bytes(b"Network/Dynamic.c4s".to_vec()).unwrap(),
        parameters: base_parameters(),
        dynamic_tick: 0,
    })
    .unwrap();

    assert_eq!(publication.player_cores.len(), 1);
    assert_eq!(publication.player_cores[0].id, 4);
    assert_eq!(
        publication
            .resource_files
            .iter()
            .map(|resource| resource.core.id)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 4]
    );
}

fn file_sha(path: &Path) -> [u8; 20] {
    Sha1::digest(fs::read(path).unwrap()).into()
}

fn source(path: PathBuf, wire_name: &[u8]) -> HostInitialResourceSource {
    source_with_names(path, wire_name, wire_name, wire_name)
}

fn source_with_names(
    path: PathBuf,
    lookup_name: &[u8],
    opened_name: &[u8],
    wire_name: &[u8],
) -> HostInitialResourceSource {
    HostInitialResourceSource {
        path,
        lookup_name: LegacyCString::from_bytes(lookup_name.to_vec()).unwrap(),
        opened_name: LegacyCString::from_bytes(opened_name.to_vec()).unwrap(),
        wire_name: LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
        virtual_group_bytes: None,
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
