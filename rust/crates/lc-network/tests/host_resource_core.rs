use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lc_engine::LegacyCString;
use lc_network::{
    build_host_resource_core, HostResourceCoreError, HostResourceCoreSpec, HostResourceType,
    ResourceFileOwnership,
};
use lc_resources::{c4group_file_crc, Group, MutableGroup};

#[test]
fn cpp_publishes_a_packed_scenario_with_separate_content_and_file_checksums() {
    // SetByGroup publishes EntryCRC32 and the header maker; GetStandalone then
    // publishes the physical file size/CRC with the stock 100 KiB chunk size
    // (src/C4Network2Res.cpp:409-437,570-698; src/C4Network2Res.h:27).
    let directory = TestDirectory::new();
    let scenario = directory.path().join("Arena.c4s");
    let mut group = MutableGroup::new("Arena.c4s");
    group.set_maker("Scenario Maker");
    group
        .add_file_with_metadata("Scenario.txt", b"[Head]\n".to_vec(), 1, false)
        .unwrap();
    let packed = group.pack().unwrap();
    fs::write(&scenario, &packed).unwrap();
    let filename = LegacyCString::from_bytes(b"Missions.c4f/Arena.c4s".to_vec()).unwrap();

    let publication = build_host_resource_core(
        &scenario,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Scenario,
            7,
            filename.clone(),
            "Host Player",
        ),
    )
    .unwrap();

    assert_eq!(publication.core.resource_type, 1);
    assert_eq!(publication.core.id, 7);
    assert_eq!(publication.core.derived_id, -1);
    assert_eq!(publication.core.filename, filename);
    assert_eq!(publication.core.author.as_bytes(), b"Scenario Maker");
    assert_eq!(publication.core.contents_crc, group.contents_crc());
    assert!(publication.core.loadable);
    assert_eq!(publication.core.file_size, packed.len() as u32);
    assert_eq!(publication.core.file_crc, c4group_file_crc(&packed));
    assert_eq!(publication.core.chunk_size, 100 * 1024);
    assert_eq!(publication.core.file_sha, None);
    assert_eq!(
        publication.standalone_path.as_deref(),
        Some(scenario.as_path())
    );
    assert_eq!(
        publication.standalone_ownership,
        Some(ResourceFileOwnership::Persistent)
    );
}

#[test]
fn cpp_packs_a_scenario_directory_into_a_temporary_standalone() {
    // A non-temporary directory is opened with maker "Open directory", then
    // GetStandalone packs it to the network directory. C4Group::Close applies
    // the process-wide player maker to the packed header
    // (src/C4Network2Res.cpp:409-425,588-637; src/C4Group.cpp:260-320,938-951).
    let directory = TestDirectory::new();
    let scenario = directory.path().join("Arena.c4s");
    let network = directory.path().join("Network");
    fs::create_dir_all(&scenario).unwrap();
    fs::write(scenario.join("Scenario.txt"), b"[Head]\n").unwrap();
    fs::create_dir(scenario.join("Objects.c4d")).unwrap();
    fs::write(scenario.join("Objects.c4d/DefCore.txt"), b"[DefCore]\n").unwrap();
    let expected_contents_crc = Group::open(&scenario).unwrap().contents_crc().unwrap();
    let filename = LegacyCString::from_bytes(b"Arena.c4s".to_vec()).unwrap();

    let publication = build_host_resource_core(
        &scenario,
        &network,
        HostResourceCoreSpec::new(HostResourceType::Scenario, 8, filename, "Host Player"),
    )
    .unwrap();

    let standalone = publication.standalone_path.as_ref().unwrap();
    let bytes = fs::read(standalone).unwrap();
    assert!(scenario.is_dir());
    assert_eq!(standalone.parent(), Some(network.as_path()));
    assert_eq!(publication.core.author.as_bytes(), b"Open directory");
    assert_eq!(publication.core.contents_crc, expected_contents_crc);
    assert_eq!(publication.core.file_size, bytes.len() as u32);
    assert_eq!(publication.core.file_crc, c4group_file_crc(&bytes));
    assert_eq!(publication.core.file_sha, None);
    assert_eq!(
        publication.standalone_ownership,
        Some(ResourceFileOwnership::Temporary)
    );
    assert_eq!(
        Group::open(standalone).unwrap().maker_bytes(),
        Some(b"Host Player".as_slice())
    );
}

#[test]
fn cpp_keeps_an_oversize_definition_logical_but_unloadable() {
    // C4GameRes::Publish alone opts NRT_Definitions into fAllowUnloadable;
    // GetStandalone clears the candidate when it exceeds MaxLoadFileSize, and
    // AddByFile retains the SetByGroup core with its non-loadable sentinels
    // (src/C4GameParameters.cpp:96-109; src/C4Network2Res.cpp:638-669,1459-1468).
    let directory = TestDirectory::new();
    let definitions = directory.path().join("Objects.c4d");
    let mut group = MutableGroup::new("Objects.c4d");
    group
        .add_file_with_metadata("DefCore.txt", b"[DefCore]\n".to_vec(), 1, false)
        .unwrap();
    fs::write(&definitions, group.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &definitions,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Definitions,
            9,
            LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap(),
            "Host Player",
        )
        .with_max_load_file_size(1),
    )
    .unwrap();

    assert_eq!(publication.core.resource_type, 4);
    assert_eq!(publication.core.contents_crc, group.contents_crc());
    assert!(!publication.core.loadable);
    assert_eq!(publication.core.file_size, u32::MAX);
    assert_eq!(publication.core.file_crc, u32::MAX);
    assert_eq!(publication.core.chunk_size, 100 * 1024);
    assert_eq!(publication.core.file_sha, None);
    assert_eq!(publication.standalone_path, None);
    assert_eq!(publication.standalone_ownership, None);
}

#[test]
fn cpp_definition_directory_size_limit_counts_files_group_packing_would_ignore() {
    // DirSizeHelper runs before C4Group_PackDirectoryTo and walks physical
    // files directly, so even a dotfile later excluded by C4Group_TestIgnore
    // can make a definition unloadable (src/C4Network2Res.cpp:32-62,588-607).
    let directory = TestDirectory::new();
    let definitions = directory.path().join("Objects.c4d");
    let network = directory.path().join("Network");
    fs::create_dir(&definitions).unwrap();
    fs::write(definitions.join("DefCore.txt"), b"").unwrap();
    fs::write(definitions.join(".cache"), b"too large").unwrap();

    let publication = build_host_resource_core(
        &definitions,
        &network,
        HostResourceCoreSpec::new(
            HostResourceType::Definitions,
            14,
            LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap(),
            "Host Player",
        )
        .with_max_load_file_size(1),
    )
    .unwrap();

    assert!(!publication.core.loadable);
    assert_eq!(publication.standalone_path, None);
    assert!(!network.exists());
}

#[test]
fn cpp_system_publication_never_creates_a_loadable_standalone() {
    // AddByFile explicitly skips GetStandalone for NRT_System, retaining
    // SetByGroup's contents CRC/author and the Set defaults for file metadata
    // (src/C4Network2Res.cpp:409-437,1443-1468).
    let directory = TestDirectory::new();
    let system = directory.path().join("System.c4g");
    let mut group = MutableGroup::new("System.c4g");
    group.set_maker("System Maker");
    group
        .add_file_with_metadata("C4.c", b"global func C4() {}\n".to_vec(), 1, false)
        .unwrap();
    fs::write(&system, group.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &system,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::System,
            10,
            LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    assert_eq!(publication.core.resource_type, 5);
    assert_eq!(publication.core.author.as_bytes(), b"System Maker");
    assert_eq!(publication.core.contents_crc, group.contents_crc());
    assert!(!publication.core.loadable);
    assert_eq!(publication.core.file_size, u32::MAX);
    assert_eq!(publication.core.file_crc, u32::MAX);
    assert_eq!(publication.core.file_sha, None);
    assert_eq!(publication.standalone_path, None);
}

#[test]
fn cpp_plain_dynamic_uses_the_whole_file_crc_for_both_checksums() {
    // When C4Group::Open fails, SetByFile uses C4Group_GetFileCRC as the
    // contents CRC; GetStandalone hashes the same physical file and does not
    // populate SHA (src/C4Network2Res.cpp:373-406,659-715).
    let directory = TestDirectory::new();
    let dynamic = directory.path().join("ArenaDyn.c4s");
    fs::write(&dynamic, b"not a group").unwrap();
    let expected_crc = c4group_file_crc(b"not a group");

    let publication = build_host_resource_core(
        &dynamic,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Dynamic,
            11,
            LegacyCString::from_bytes(b"ArenaDyn.c4s".to_vec()).unwrap(),
            "Host Player",
        )
        .with_source_ownership(ResourceFileOwnership::Temporary),
    )
    .unwrap();

    assert_eq!(publication.core.resource_type, 2);
    assert_eq!(publication.core.contents_crc, expected_crc);
    assert_eq!(publication.core.file_crc, expected_crc);
    assert_eq!(publication.core.file_size, 11);
    assert_eq!(publication.core.author.as_bytes(), b"");
    assert_eq!(publication.core.file_sha, None);
    assert_eq!(
        publication.standalone_path.as_deref(),
        Some(dynamic.as_path())
    );
    assert_eq!(
        publication.standalone_ownership,
        Some(ResourceFileOwnership::Temporary)
    );
}

#[test]
fn cpp_network_core_preserves_non_utf8_group_maker_bytes() {
    // StdStrBuf copies the C4Group maker byte-for-byte into Author and the
    // network compiler later serializes that byte string unchanged
    // (src/C4Network2Res.cpp:409-425,113-142).
    let directory = TestDirectory::new();
    let material = directory.path().join("Material.c4g");
    let mut group = MutableGroup::new("Material.c4g");
    group
        .add_file_with_metadata("TexMap.txt", b"Earth=1\n".to_vec(), 1, false)
        .unwrap();
    let mut image = group.pack_raw().unwrap();
    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    scramble_group_header(&mut header);
    header[40..44].copy_from_slice(&[0xff, b'A', b'B', 0]);
    scramble_group_header(&mut header);
    image[..204].copy_from_slice(&header);
    fs::write(&material, image).unwrap();

    let publication = build_host_resource_core(
        &material,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Material,
            12,
            LegacyCString::from_bytes(b"Material.c4g".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    assert_eq!(publication.core.author.as_bytes(), &[0xff, b'A', b'B']);
    assert_eq!(publication.core.file_sha, None);
}

#[test]
fn player_publication_fails_typed_instead_of_skipping_cpp_optimization() {
    // OptimizeStandalone always applies the player copy/delete policy before
    // file metadata is published (src/C4Network2Res.cpp:1168-1206).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    fs::write(&player, b"player").unwrap();

    let error = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            13,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        HostResourceCoreError::PlayerOptimizationUnsupported
    ));
}

fn scramble_group_header(buffer: &mut [u8]) {
    buffer.iter_mut().for_each(|byte| *byte ^= 237);
    for index in (0..buffer.len().saturating_sub(2)).step_by(3) {
        buffer.swap(index, index + 2);
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
            "legacyclonk-host-resource-core-{}-{unique}",
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
