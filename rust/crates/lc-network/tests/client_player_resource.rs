use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lc_engine::LegacyCString;
use lc_network::{
    publish_client_player_resource, ClientPlayerResourcePublicationSpec, ResourceFileOwnership,
    ResourceTransferBackend,
};
use lc_resources::{Group, MutableGroup};

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
        group_maker: "Alice".to_owned(),
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

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "legacyclonk-client-player-resource-{}-{unique}",
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
