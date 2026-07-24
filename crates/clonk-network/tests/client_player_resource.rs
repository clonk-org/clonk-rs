use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clonk_engine::LegacyCString;
use clonk_network::{
    publish_client_player_resource, ClientPlayerResourcePublicationSpec, ResourceFileOwnership,
    ResourceTransferBackend,
};
use clonk_resources::{c4group_file_crc, Group, MutableGroup};

#[test]
fn cpp_client_player_uses_assigned_namespace_and_an_optimized_copy() {
    // SetLocalID moves the next resource ID to client_id << 16, AddByFile
    // publishes NRT_Player, and OptimizeStandalone first copies a persistent
    // player before deleting portraits and an oversized root BigIcon.png
    // (pristine 9ffa0a5d src/C4Network2Res.cpp:1168-1205,1361-1385,
    // 1443-1471; src/C4PlayerInfo.cpp:70-104).
    let directory = TestDirectory::new();
    let player = directory.path().join("Alice.c4p");
    let network = directory.path().join("Network");
    let mut original = MutableGroup::new("Alice.c4p");
    original.set_maker("Original Maker");
    original
        .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
        .unwrap();
    original
        .add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 2, false)
        .unwrap();
    original
        .add_file_with_metadata("BigIcon.png", vec![0x42; 20 * 1024 + 1], 3, false)
        .unwrap();
    let original_bytes = original.pack().unwrap();
    fs::write(&player, &original_bytes).unwrap();

    let publication = publish_client_player_resource(ClientPlayerResourcePublicationSpec {
        resource_id: 7 << 16,
        source_path: player.clone(),
        wire_name: LegacyCString::from_bytes(b"Players.c4f/Alice.c4p".to_vec()).unwrap(),
        network_directory: network.clone(),
        group_maker: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
    })
    .unwrap();

    assert_eq!(publication.core.id, 7 << 16);
    assert_eq!(publication.core.resource_type, 3);
    assert_eq!(
        publication.core.filename.as_bytes(),
        b"Players.c4f/Alice.c4p"
    );
    assert!(publication.core.loadable);
    assert_eq!(fs::read(&player).unwrap(), original_bytes);
    assert_ne!(publication.resource_file.path, player);
    assert_eq!(
        publication.resource_file.path.parent(),
        Some(network.as_path())
    );
    assert_eq!(
        publication.resource_file.ownership,
        ResourceFileOwnership::Temporary
    );
    assert!(publication.resource_file.binary_compatible);
    assert_eq!(publication.resource_file.core, publication.core);
    assert_eq!(publication.registration.resource_id, 7 << 16);
    assert!(publication.registration.binary_compatible);
    assert!(!publication.registration.loading);

    let optimized = Group::open(&publication.resource_file.path).unwrap();
    assert_eq!(optimized.read_file("Player.txt").unwrap(), b"player core");
    assert!(!optimized.exists("Portrait.png"));
    assert!(!optimized.exists("BigIcon.png"));

    let mut backend = ResourceTransferBackend::new(7, directory.path().join("backend")).unwrap();
    backend
        .register_hosted_resource(
            publication.resource_file.core.clone(),
            &publication.resource_file.path,
            publication.resource_file.ownership,
            publication.resource_file.binary_compatible,
        )
        .unwrap();
    assert_eq!(backend.core(7 << 16), Some(&publication.core));
    assert_eq!(
        backend.path(7 << 16),
        Some(publication.resource_file.path.as_path())
    );
}

#[test]
fn cpp_client_player_rewrite_preserves_raw_global_maker_bytes() {
    // Config.General.Name is passed directly to C4Group_SetMaker, and closing
    // the player after OptimizeStandalone deletes a portrait copies those
    // legacy bytes into the rewritten header (pristine 9ffa0a5d
    // src/C4Application.cpp:118-121; src/C4Group.cpp:924-935;
    // src/C4Network2Res.cpp:1168-1205; src/C4PlayerInfo.cpp:70-104).
    let directory = TestDirectory::new();
    let player = directory.path().join("RawMaker.c4p");
    let network = directory.path().join("Network");
    let raw_maker = LegacyCString::from_bytes(vec![0xff, b'A', b'B']).unwrap();
    let mut crew = MutableGroup::new("Crew.c4i");
    crew.add_file_with_metadata("ObjectInfo.txt", b"crew".to_vec(), 3, false)
        .unwrap();
    crew.add_file_with_metadata("Portrait.bmp", b"portrait".to_vec(), 4, false)
        .unwrap();
    let mut original = MutableGroup::new("RawMaker.c4p");
    original.set_maker("Original Maker");
    original
        .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
        .unwrap();
    original
        .add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 2, false)
        .unwrap();
    original
        .add_child_with_metadata("Crew.c4i", crew, 5, false)
        .unwrap();
    fs::write(&player, original.pack().unwrap()).unwrap();

    let publication = publish_client_player_resource(ClientPlayerResourcePublicationSpec {
        resource_id: 8 << 16,
        source_path: player.clone(),
        wire_name: LegacyCString::from_bytes(b"Players.c4f/RawMaker.c4p".to_vec()).unwrap(),
        network_directory: network.clone(),
        group_maker: raw_maker.clone(),
    })
    .unwrap();
    let utf8_publication = publish_client_player_resource(ClientPlayerResourcePublicationSpec {
        resource_id: (8 << 16) + 1,
        source_path: player,
        wire_name: LegacyCString::from_bytes(b"Players.c4f/RawMaker.c4p".to_vec()).unwrap(),
        network_directory: network,
        group_maker: LegacyCString::from_bytes(b"?AB".to_vec()).unwrap(),
    })
    .unwrap();

    let optimized_bytes = fs::read(&publication.resource_file.path).unwrap();
    let optimized = Group::open(&publication.resource_file.path).unwrap();
    let optimized_crew = optimized.open_child("Crew.c4i").unwrap();
    assert_eq!(optimized.maker_bytes(), Some(raw_maker.as_bytes()));
    assert_eq!(optimized_crew.maker_bytes(), Some(raw_maker.as_bytes()));
    assert_eq!(
        publication.core.file_crc,
        c4group_file_crc(&optimized_bytes)
    );

    let utf8_bytes = fs::read(&utf8_publication.resource_file.path).unwrap();
    let utf8_optimized = Group::open(&utf8_publication.resource_file.path).unwrap();
    assert_eq!(utf8_optimized.maker_bytes(), Some(b"?AB".as_slice()));
    assert_ne!(optimized_bytes, utf8_bytes);
    assert_ne!(publication.core.file_crc, utf8_publication.core.file_crc);
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "clonk-rust-client-player-resource-{}-{unique}",
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
