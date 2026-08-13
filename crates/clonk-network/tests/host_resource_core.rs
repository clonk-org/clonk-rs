use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clonk_engine::LegacyCString;
use clonk_network::{
    build_host_resource_core, HostResourceCoreError, HostResourceCoreSpec, HostResourcePublication,
    HostResourceType, ResourceFileOwnership,
};
use clonk_resources::{c4group_file_crc, Group, MutableGroup};
use flate2::write::GzEncoder;
use flate2::Compression;

#[test]
fn calculate_file_sha_prefers_standalone_and_is_idempotent() {
    let directory = TestDirectory::new();
    let source = directory.path().join("source.c4g");
    let standalone = directory.path().join("standalone.c4g");
    fs::write(&source, b"source").unwrap();
    fs::write(&standalone, b"standalone").unwrap();
    let mut publication = HostResourcePublication {
        core: Default::default(),
        source_path: source.clone(),
        standalone_path: Some(standalone.clone()),
        standalone_ownership: Some(ResourceFileOwnership::Temporary),
    };

    publication.calculate_file_sha().unwrap();
    let expected = [
        0x0b, 0x5c, 0xce, 0xaa, 0xfa, 0x4c, 0xc0, 0x72, 0xea, 0x5e, 0x5f, 0x55, 0x8c, 0xd1, 0xe9,
        0x9a, 0x8f, 0x50, 0x3c, 0x2d,
    ];
    assert_eq!(publication.core.file_sha, Some(expected));

    fs::remove_file(source).unwrap();
    fs::remove_file(standalone).unwrap();
    publication.calculate_file_sha().unwrap();
    assert_eq!(publication.core.file_sha, Some(expected));
}

#[test]
fn calculate_file_sha_falls_back_to_source_without_standalone() {
    let directory = TestDirectory::new();
    let source = directory.path().join("System.c4g");
    fs::write(&source, b"abc").unwrap();
    let mut publication = HostResourcePublication {
        core: Default::default(),
        source_path: source,
        standalone_path: None,
        standalone_ownership: None,
    };

    publication.calculate_file_sha().unwrap();
    assert_eq!(
        publication.core.file_sha,
        Some([
            0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
            0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
        ])
    );
}

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
    // 10 KiB, OpenClonk's value, not LegacyClonk's 100 KiB (C++
    // `C4NetResChunkSize`, src/C4Network2Res.h:27). Resource chunks and
    // control share one strictly-ordered reliable-UDP sequence space whenever a
    // peer has no TCP route -- the ordinary internet topology, since NAT
    // punch-through is UDP-only -- so a 100 KiB chunk puts 206 datagrams ahead
    // of every later control packet and one lost fragment withholds all of them
    // until the repair lands; 10 KiB is 21 datagrams. Chunk size travels per
    // resource in the core and is honoured by whoever downloads it, so a stock
    // C++ peer follows this unmodified, and `RESOURCE_MAX_LOADS` is scaled with
    // it to keep C++'s 2 MB of outstanding bulk. Serving code must take each
    // chunk's *length* from the core's own stride, never from the hardcoded
    // 100 KiB of src/C4Network2Res.cpp:1268-1269: that literal is
    // self-consistent only because every core C++ publishes carries
    // `ChunkSize = C4NetResChunkSize`, and copying it made each chunk overlap
    // the following nine -- the host served the file ten times over and the
    // fragment burst stayed 206.
    assert_eq!(publication.core.chunk_size, 10 * 1024);
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
    // An unloadable core keeps C++'s default: nothing will be transferred, and
    // C++ substitutes its compiled-in defaults when decoding one, so a custom
    // chunk size could not round-trip.
    assert_eq!(publication.core.chunk_size, 100 * 1024);
    assert_eq!(publication.core.file_sha, None);
    assert_eq!(publication.standalone_path, None);
    assert_eq!(publication.standalone_ownership, None);
}

#[test]
fn cpp_keeps_a_definition_when_standalone_packing_fails() {
    // AddByFile retains NRT_Definitions when any GetStandalone step fails and
    // fAllowUnloadable is set. Occupy every FindTempResFileName candidate so
    // temporary standalone creation fails deterministically on every platform
    // (src/C4Network2Res.cpp:610-616,1457-1467,1741-1793).
    let directory = TestDirectory::new();
    let definitions = directory.path().join("Objects.c4d");
    fs::create_dir(&definitions).unwrap();
    fs::write(definitions.join("DefCore.txt"), b"[DefCore]\n").unwrap();
    let network = directory.path().join("Network");
    fs::create_dir(&network).unwrap();
    for suffix in 1..=999 {
        let filename = if suffix == 1 {
            "Objects.c4d".to_owned()
        } else {
            format!("Objects_{suffix}.c4d")
        };
        fs::write(network.join(filename), b"occupied").unwrap();
    }

    let publication = build_host_resource_core(
        &definitions,
        &network,
        HostResourceCoreSpec::new(
            HostResourceType::Definitions,
            10,
            LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap(),
            "Host Player",
        )
        .with_standalone_name(LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap()),
    )
    .expect("fAllowUnloadable retains definitions after GetStandalone failure");

    assert!(!publication.core.loadable);
    assert_eq!(publication.core.file_size, u32::MAX);
    assert_eq!(publication.standalone_path, None);
    assert_eq!(publication.source_path, definitions);
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

#[cfg(unix)]
#[test]
fn cpp_dangling_symlink_is_zero_sized_in_unpacked_system_crc() {
    // DirectoryIterator + C4GroupEntry::Set use stat(2). A dangling symlink
    // therefore remains visible with zero-initialized metadata, and
    // CalcCRC32 assigns it CRC zero instead of aborting the whole group
    // (src/C4Group.cpp:586-603,2181-2193,2447-2484).
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let system = directory.path().join("System.c4g");
    fs::create_dir(&system).unwrap();
    fs::write(system.join("C4.c"), b"global func C4() {}\n").unwrap();
    let readable_crc = Group::open(&system).unwrap().contents_crc().unwrap();

    let missing = directory.path().join("cleaned-tmp-oracle/System.c4g");
    symlink(&missing, system.join("stale-oracle")).unwrap();

    let group = Group::open(&system).unwrap();
    let stale = group
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.relative_path == Path::new("stale-oracle"))
        .expect("C++ directory scan retains the dangling entry");
    assert!(!stale.is_directory);
    assert_eq!(stale.size, 0);
    assert_eq!(stale.time, 0);
    assert_eq!(group.contents_crc().unwrap(), readable_crc);

    let publication = build_host_resource_core(
        &system,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::System,
            18,
            LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .expect("a cleaned temporary symlink target cannot abort host preparation");

    assert_eq!(publication.core.contents_crc, readable_crc);
    assert!(!publication.core.loadable);
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
    fs::write(&material, gzip_group_image(&image)).unwrap();

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
fn cpp_player_publication_copies_then_removes_portraits_and_oversize_bigicon() {
    // A persistent player is copied to the network work path before
    // OptimizeStandalone recursively deletes Portrait*.* and deletes a root
    // BigIcon.png only above 20 KiB. The core keeps the original contents CRC
    // and maker but publishes the optimized physical size/CRC
    // (src/C4Network2Res.cpp:570-698,1168-1206;
    // src/C4Network2Res.h:27-36).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let network = directory.path().join("Network");
    let mut crew = MutableGroup::new("Crew.c4i");
    crew.add_file_with_metadata("ObjectInfo.txt", b"crew".to_vec(), 7, false)
        .unwrap();
    crew.add_file_with_metadata("Portrait.bmp", b"nested portrait".to_vec(), 8, false)
        .unwrap();
    let mut original = MutableGroup::new("Player.c4p");
    original.set_maker("Original Maker");
    original
        .add_file_with_metadata("Player.txt", b"player".to_vec(), 5, false)
        .unwrap();
    original
        .add_file_with_metadata("Portrait.png", b"root portrait".to_vec(), 6, false)
        .unwrap();
    original
        .add_file_with_metadata("BigIcon.png", vec![0x42; 20 * 1024 + 1], 9, false)
        .unwrap();
    original
        .add_child_with_metadata("Crew.c4i", crew, 10, false)
        .unwrap();
    let original_bytes = original.pack().unwrap();
    fs::write(&player, &original_bytes).unwrap();

    let publication = build_host_resource_core(
        &player,
        &network,
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            13,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let standalone = publication.standalone_path.as_ref().unwrap();
    let optimized = Group::open(standalone).unwrap();
    let optimized_crew = optimized.open_child("Crew.c4i").unwrap();
    assert_eq!(fs::read(&player).unwrap(), original_bytes);
    assert_ne!(standalone, &player);
    assert_eq!(standalone.parent(), Some(network.as_path()));
    assert!(!optimized.exists("Portrait.png"));
    assert!(!optimized.exists("BigIcon.png"));
    assert!(!optimized_crew.exists("Portrait.bmp"));
    assert_eq!(optimized.read_file("Player.txt").unwrap(), b"player");
    assert_eq!(optimized_crew.read_file("ObjectInfo.txt").unwrap(), b"crew");
    assert_eq!(optimized.maker(), Some("Host Player"));
    assert_eq!(publication.core.author.as_bytes(), b"Original Maker");
    assert_eq!(publication.core.contents_crc, original.contents_crc());
    let optimized_bytes = fs::read(standalone).unwrap();
    assert_eq!(publication.core.file_size, optimized_bytes.len() as u32);
    assert_eq!(
        publication.core.file_crc,
        c4group_file_crc(&optimized_bytes)
    );
    assert_eq!(
        publication.standalone_ownership,
        Some(ResourceFileOwnership::Temporary)
    );
}

#[test]
fn cpp_player_at_bigicon_limit_is_an_exact_collision_safe_copy() {
    // OptimizeStandalone uses a strict `>` 20 KiB comparison. If neither that
    // deletion nor Portrait*.* deletion changes the group, Close does not
    // rewrite it, so its original header/maker and every physical byte remain
    // intact. FindTempResFileName advances to `_2` on a name collision
    // (src/C4Network2Res.cpp:570-698,1168-1206,1741-1792).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let network = directory.path().join("Network");
    fs::create_dir(&network).unwrap();
    fs::write(network.join("Player.c4p"), b"occupied").unwrap();
    let mut original = MutableGroup::new("Player.c4p");
    original.set_maker("Original Maker");
    original
        .add_file_with_metadata("Player.txt", b"player".to_vec(), 5, false)
        .unwrap();
    original
        .add_file_with_metadata("BigIcon.png", vec![0x42; 20 * 1024], 9, false)
        .unwrap();
    let original_bytes = original.pack().unwrap();
    fs::write(&player, &original_bytes).unwrap();

    let publication = build_host_resource_core(
        &player,
        &network,
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            14,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let standalone = publication.standalone_path.as_ref().unwrap();
    assert_eq!(standalone.file_name().unwrap(), "Player_2.c4p");
    assert_eq!(fs::read(standalone).unwrap(), original_bytes);
    let optimized = Group::open(standalone).unwrap();
    assert!(optimized.exists("BigIcon.png"));
    assert_eq!(optimized.maker(), Some("Original Maker"));
    assert_eq!(publication.core.file_size, original_bytes.len() as u32);
    assert_eq!(publication.core.file_crc, c4group_file_crc(&original_bytes));
}

#[test]
fn cpp_player_directory_is_packed_then_optimized_without_touching_source() {
    // GetStandalone packs a persistent directory into the network work path,
    // switches the resource to temporary ownership, and only then runs the
    // same player deletion pass. SetByGroup has already frozen the directory's
    // "Open directory" author and pre-optimization contents CRC
    // (src/C4Network2Res.cpp:409-437,570-698,1168-1206).
    let directory = TestDirectory::new();
    let player = directory.path().join("DirectoryPlayer.c4p");
    let network = directory.path().join("Network");
    fs::create_dir(&player).unwrap();
    fs::write(player.join("Player.txt"), b"player").unwrap();
    fs::write(player.join("Portrait.png"), b"portrait").unwrap();
    let expected_contents_crc = Group::open(&player).unwrap().contents_crc().unwrap();

    let publication = build_host_resource_core(
        &player,
        &network,
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            15,
            LegacyCString::from_bytes(b"DirectoryPlayer.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let standalone = publication.standalone_path.as_ref().unwrap();
    let optimized = Group::open(standalone).unwrap();
    assert!(player.is_dir());
    assert!(player.join("Portrait.png").is_file());
    assert_eq!(standalone.parent(), Some(network.as_path()));
    assert!(!optimized.exists("Portrait.png"));
    assert_eq!(optimized.read_file("Player.txt").unwrap(), b"player");
    assert_eq!(optimized.maker(), Some("Host Player"));
    assert_eq!(publication.core.author.as_bytes(), b"Open directory");
    assert_eq!(publication.core.contents_crc, expected_contents_crc);
    assert_eq!(
        publication.standalone_ownership,
        Some(ResourceFileOwnership::Temporary)
    );
}

#[test]
fn cpp_player_directory_imports_packed_crew_before_recursive_optimization() {
    // PackDirectoryTo passes regular files through C4Group::Add; AddEntryOnDisk
    // recognizes a packed C4Group and embeds its uncompressed image as a child.
    // OptimizeStandalone can then recurse into that child
    // (src/C4Group.cpp:272-300,1446-1495; src/C4Network2Res.cpp:1197).
    let directory = TestDirectory::new();
    let player = directory.path().join("DirectoryPlayer.c4p");
    fs::create_dir(&player).unwrap();
    fs::write(player.join("Player.txt"), b"player").unwrap();
    let mut crew = MutableGroup::new("Crew.c4i");
    crew.add_file_with_metadata("ObjectInfo.txt", b"crew".to_vec(), 7, false)
        .unwrap();
    crew.add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 8, false)
        .unwrap();
    fs::write(player.join("Crew.c4i"), crew.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            18,
            LegacyCString::from_bytes(b"DirectoryPlayer.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    let optimized_crew = optimized.open_child("Crew.c4i").unwrap();
    assert!(!optimized_crew.exists("Portrait.png"));
    assert_eq!(optimized_crew.read_file("ObjectInfo.txt").unwrap(), b"crew");
    assert!(player.join("Crew.c4i").is_file());
}

#[test]
fn cpp_rewritten_child_moves_to_unsorted_mother_tail() {
    // A rewritten child is moved back into its mother and AddEntry appends the
    // replacement. Unknown group extensions have no C4CFN_FLS sort pass, so
    // that tail position is physically significant
    // (src/C4Group.cpp:1018-1024,839-881,2356-2372).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.bin");
    let mut child = MutableGroup::new("Crew.c4i");
    child
        .add_file("Portrait.png", b"portrait".to_vec())
        .unwrap();
    let mut original = MutableGroup::new("Player.bin");
    original.add_file("A.txt", b"a".to_vec()).unwrap();
    original.add_child("Crew.c4i", child).unwrap();
    original.add_file("Z.txt", b"z".to_vec()).unwrap();
    fs::write(&player, original.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            19,
            LegacyCString::from_bytes(b"Player.bin".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    let names = optimized
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.relative_path)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            PathBuf::from("A.txt"),
            PathBuf::from("Z.txt"),
            PathBuf::from("Crew.c4i")
        ]
    );
}

#[test]
fn cpp_recursive_delete_preserves_a_non_raw_child_payload() {
    // OpenAsChild expects the raw nested C4Group image. If a child-marked
    // payload is itself gzip wrapped, recursive Delete simply skips it; a
    // separate root deletion still rewrites the parent and copies it opaquely
    // (src/C4Group.cpp:1544-1556,1792-1908).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let mut opaque_child = MutableGroup::new("Opaque.c4i");
    opaque_child
        .add_file("Portrait.png", b"must remain".to_vec())
        .unwrap();
    let opaque_bytes = opaque_child.pack().unwrap();
    let mut original = MutableGroup::new("Player.c4p");
    original
        .add_packed_child_with_metadata(
            "Opaque.c4i",
            opaque_bytes.clone(),
            opaque_child.contents_crc(),
            7,
            false,
        )
        .unwrap();
    original
        .add_file("Portrait.png", b"remove root".to_vec())
        .unwrap();
    fs::write(&player, original.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            20,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    assert!(!optimized.exists("Portrait.png"));
    assert_eq!(
        optimized.read_entry_bytes("Opaque.c4i").unwrap(),
        opaque_bytes
    );
}

#[test]
fn opaque_child_with_old_crc_state_fails_typed_instead_of_silently_becoming_new() {
    // CalcCRC32 cannot open this child and returns before changing HasCRC.
    // Close ignores EntryCRC32's failure and saves the original state-1 core;
    // emitting state 2 would claim a checksum C++ never established
    // (src/C4Group.cpp:937-944,2434-2458).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let mut opaque_child = MutableGroup::new("Opaque.c4i");
    opaque_child
        .add_file("ObjectInfo.txt", b"opaque".to_vec())
        .unwrap();
    let opaque_bytes = opaque_child.pack().unwrap();
    let mut original = MutableGroup::new("Player.c4p");
    original
        .add_packed_child_with_metadata("Opaque.c4i", opaque_bytes, 0x1234_5678, 7, false)
        .unwrap();
    original
        .add_file("Portrait.png", b"remove root".to_vec())
        .unwrap();
    let mut raw = original.pack_raw().unwrap();
    let mut changed_state = false;
    for index in 0..2 {
        let start = 204 + index * 316;
        let end = raw[start..start + 260]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap();
        if &raw[start..start + end] == b"Opaque.c4i" {
            raw[start + 284] = 1;
            changed_state = true;
        }
    }
    assert!(changed_state);
    fs::write(&player, gzip_group_image(&raw)).unwrap();

    let error = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            24,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        HostResourceCoreError::OpaqueChildCrcStateUnsupported {
            path,
            crc_state: 1
        } if path == Path::new("Opaque.c4i")
    ));
}

#[test]
fn directory_player_packing_shields_nested_opaque_old_crc() {
    // PackDirectoryTo imports a packed Crew.c4i and Close attempts recursive
    // CRC calculation before OptimizeStandalone. An unopenable nested child
    // keeps its old HasCRC core because CalcCRC32 returns early and Close
    // ignores the failure. The parent Crew.c4i core nevertheless receives the
    // child's resulting zero contents CRC as state New, shielding the opaque
    // nested core from the later root-only rewrite
    // (src/C4Group.cpp:272-314,937-944,2434-2458).
    let directory = TestDirectory::new();
    let player = directory.path().join("DirectoryPlayer.c4p");
    fs::create_dir(&player).unwrap();
    fs::write(player.join("Player.txt"), b"player").unwrap();
    fs::write(player.join("Portrait.png"), b"remove root").unwrap();

    let mut opaque = MutableGroup::new("Opaque.c4i");
    opaque
        .add_file("ObjectInfo.txt", b"opaque".to_vec())
        .unwrap();
    let opaque_bytes = opaque.pack().unwrap();
    let mut crew = MutableGroup::new("Crew.c4i");
    crew.add_packed_child_with_metadata("Opaque.c4i", opaque_bytes, 0x1234_5678, 7, false)
        .unwrap();
    let mut crew_raw = crew.pack_raw().unwrap();
    crew_raw[204 + 284] = 1;
    fs::write(player.join("Crew.c4i"), &crew_raw).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            25,
            LegacyCString::from_bytes(b"DirectoryPlayer.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    assert!(!optimized.exists("Portrait.png"));
    assert_eq!(optimized.read_entry_bytes("Crew.c4i").unwrap(), crew_raw);
    assert!(player.is_dir());
    assert!(player.join("Crew.c4i").is_file());
}

#[test]
fn opaque_old_crc_without_a_rewrite_remains_an_exact_copy() {
    // Close checks whether a rewrite is needed before EntryCRC32. With no
    // deletion, duplicate, or modified child, an opaque state-1 core remains
    // physically untouched (src/C4Group.cpp:897-920,2434-2458).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let mut opaque = MutableGroup::new("Opaque.c4i");
    opaque
        .add_file("ObjectInfo.txt", b"opaque".to_vec())
        .unwrap();
    let mut original = MutableGroup::new("Player.c4p");
    original
        .add_packed_child_with_metadata("Opaque.c4i", opaque.pack().unwrap(), 0x1234_5678, 7, false)
        .unwrap();
    let mut raw = original.pack_raw().unwrap();
    raw[204 + 284] = 1;
    let packed = gzip_group_image(&raw);
    fs::write(&player, &packed).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            26,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    assert_eq!(
        fs::read(publication.standalone_path.unwrap()).unwrap(),
        packed
    );
    assert_eq!(publication.core.contents_crc, 0);
}

#[test]
fn new_parent_crc_shields_nested_opaque_old_crc_during_root_rewrite() {
    // CalcCRC32 returns the nested group's zero EntryCRC32 to its successfully
    // opened parent and marks that parent core New. A later root rewrite then
    // trusts the parent CRC and copies its raw payload without changing the
    // nested state-1 core (src/C4Group.cpp:2434-2458).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let mut opaque = MutableGroup::new("Opaque.c4i");
    opaque
        .add_file("ObjectInfo.txt", b"opaque".to_vec())
        .unwrap();
    let mut crew = MutableGroup::new("Crew.c4i");
    crew.add_packed_child_with_metadata(
        "Opaque.c4i",
        opaque.pack().unwrap(),
        0x1234_5678,
        7,
        false,
    )
    .unwrap();
    let mut crew_raw = crew.pack_raw().unwrap();
    crew_raw[204 + 284] = 1;

    let mut original = MutableGroup::new("Player.c4p");
    original
        .add_packed_child_with_metadata("Crew.c4i", crew_raw.clone(), 0, 8, false)
        .unwrap();
    original
        .add_file("Portrait.png", b"remove root".to_vec())
        .unwrap();
    fs::write(&player, original.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            27,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    assert!(!optimized.exists("Portrait.png"));
    assert_eq!(optimized.read_entry_bytes("Crew.c4i").unwrap(), crew_raw);
}

#[test]
fn cpp_duplicate_entry_marks_an_otherwise_unchanged_player_for_rewrite() {
    // OpenRealGrpFile feeds every core through AddEntry. A duplicate marks the
    // earlier core deleted, and Close therefore rewrites even if Delete found
    // no portraits (src/C4Group.cpp:771-784,839-881,897-920).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let mut original = MutableGroup::new("Player.c4p");
    original.set_maker("Original Maker");
    original.add_file("A.txt", b"a".to_vec()).unwrap();
    original.add_file("B.txt", b"b".to_vec()).unwrap();
    let mut raw = original.pack_raw().unwrap();
    let second_name = 204 + 316;
    raw[second_name..second_name + 260].fill(0);
    raw[second_name..second_name + 5].copy_from_slice(b"A.txt");
    fs::write(&player, gzip_group_image(&raw)).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            21,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    assert_eq!(optimized.maker(), Some("Host Player"));
    assert_eq!(optimized.entries().unwrap().len(), 1);
}

#[test]
fn cpp_player_rewrite_preserves_legacy_entry_name_bytes() {
    // C4Group validates filenames in its char buffer and copies those bytes
    // back into rewritten entry cores without a UTF-8 transcode
    // (src/C4Group.cpp:771-784,854-870,955-1015).
    let directory = TestDirectory::new();
    let player = directory.path().join("Player.c4p");
    let mut original = MutableGroup::new("Player.c4p");
    original.add_file("Other.txt", b"keep".to_vec()).unwrap();
    original
        .add_file("Portrait.png", b"remove".to_vec())
        .unwrap();
    let mut raw = original.pack_raw().unwrap();
    let legacy_name = [0xe4, b'.', b't', b'x', b't'];
    let mut replaced = false;
    for index in 0..2 {
        let start = 204 + index * 316;
        let end = raw[start..start + 260]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap();
        if &raw[start..start + end] == b"Other.txt" {
            raw[start..start + 260].fill(0);
            raw[start..start + legacy_name.len()].copy_from_slice(&legacy_name);
            replaced = true;
        }
    }
    assert!(replaced);
    fs::write(&player, gzip_group_image(&raw)).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            22,
            LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            "Host Player",
        ),
    )
    .unwrap();

    let optimized = Group::open(publication.standalone_path.unwrap()).unwrap();
    let retained = optimized.entries().unwrap().pop().unwrap();
    assert_eq!(retained.name_bytes, legacy_name);
}

#[cfg(unix)]
#[test]
fn temporary_directory_pack_staging_failure_preserves_source_contents() {
    // C4Group_PackDirectory writes the packed sibling first and only then
    // renames the source directory; a failure creating that sibling cannot
    // delete source contents (src/C4Group.cpp:319-338).
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new();
    let player = directory.path().join("Temporary.c4p");
    fs::create_dir(&player).unwrap();
    fs::write(player.join("Player.txt"), b"must survive").unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let result = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            23,
            LegacyCString::from_bytes(b"Temporary.c4p".to_vec()).unwrap(),
            "Host Player",
        )
        .with_source_ownership(ResourceFileOwnership::Temporary),
    );

    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err());
    assert_eq!(
        fs::read(player.join("Player.txt")).unwrap(),
        b"must survive"
    );
}

#[test]
fn cpp_temporary_packed_player_is_optimized_in_place() {
    // OptimizeStandalone skips the protective copy when fTempFile is already
    // true, so a temporary packed .c4p is rewritten at its source path and
    // remains temporary (src/C4Network2Res.cpp:570-698,1168-1206).
    let directory = TestDirectory::new();
    let player = directory.path().join("Temporary.c4p");
    let mut original = MutableGroup::new("Temporary.c4p");
    original.set_maker("Original Maker");
    original
        .add_file_with_metadata("Player.txt", b"player".to_vec(), 5, false)
        .unwrap();
    original
        .add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 6, false)
        .unwrap();
    fs::write(&player, original.pack().unwrap()).unwrap();

    let publication = build_host_resource_core(
        &player,
        directory.path().join("Network"),
        HostResourceCoreSpec::new(
            HostResourceType::Player,
            16,
            LegacyCString::from_bytes(b"Temporary.c4p".to_vec()).unwrap(),
            "Host Player",
        )
        .with_source_ownership(ResourceFileOwnership::Temporary),
    )
    .unwrap();

    let optimized = Group::open(&player).unwrap();
    assert!(!optimized.exists("Portrait.png"));
    assert_eq!(optimized.read_file("Player.txt").unwrap(), b"player");
    assert_eq!(optimized.maker(), Some("Host Player"));
    assert_eq!(
        publication.standalone_path.as_deref(),
        Some(player.as_path())
    );
    assert_eq!(
        publication.standalone_ownership,
        Some(ResourceFileOwnership::Temporary)
    );
}

#[test]
fn cpp_oracle_player_optimized_bytes_match_when_available() {
    // This optional differential runs the unmodified c4group oracle against a
    // copy of the same packed player. Both paths perform the recursive
    // Portrait*.* and root BigIcon.png deletions in one open/close cycle; when
    // they execute within one timestamp second, their complete compressed
    // standalone bytes must match (src/C4Network2Res.cpp:1168-1206;
    // src/C4Group.cpp:897-951,955-1025,1516-1562).
    let Ok(oracle) = std::env::var("LC_C4GROUP_ORACLE") else {
        return;
    };
    for attempt in 0..8 {
        let directory = TestDirectory::new();
        let rust_player = directory.path().join(format!("Rust{attempt}.c4p"));
        let oracle_player = directory.path().join(format!("Oracle{attempt}.c4p"));
        let mut crew = MutableGroup::new("Crew.c4i");
        crew.add_file_with_metadata("ObjectInfo.txt", b"crew".to_vec(), 7, false)
            .unwrap();
        crew.add_file_with_metadata("Portrait.bmp", b"nested portrait".to_vec(), 8, false)
            .unwrap();
        let mut original = MutableGroup::new("Player.c4p");
        original.set_maker("Original Maker");
        original
            .add_file_with_metadata("Player.txt", b"player".to_vec(), 5, false)
            .unwrap();
        original
            .add_file_with_metadata("Portrait.png", b"root portrait".to_vec(), 6, false)
            .unwrap();
        original
            .add_file_with_metadata("BigIcon.png", vec![0x42; 20 * 1024 + 1], 9, false)
            .unwrap();
        original
            .add_existing_file_with_metadata(
                "Trusted.bin",
                b"trusted stored CRC".to_vec(),
                0xdead_beef,
                11,
                false,
            )
            .unwrap();
        original
            .add_child_with_metadata("Crew.c4i", crew, 10, false)
            .unwrap();
        let original_bytes = original.pack().unwrap();
        fs::write(&rust_player, &original_bytes).unwrap();
        fs::write(&oracle_player, &original_bytes).unwrap();
        let oracle_home = directory.path().join("OracleHome");
        let oracle_preferences = oracle_home.join("Library/Preferences");
        fs::create_dir_all(&oracle_preferences).unwrap();
        fs::write(
            oracle_preferences.join("legacyclonk.config"),
            b"[General]\nName=\"Host Player\"\n",
        )
        .unwrap();

        let before = unix_time_now();
        let output = Command::new(&oracle)
            .env("HOME", &oracle_home)
            .arg("-r")
            .arg(&oracle_player)
            .args(["-d", "Portrait*.*", "-d", "BigIcon.png"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "c4group failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let publication = build_host_resource_core(
            &rust_player,
            directory.path().join(format!("Network{attempt}")),
            HostResourceCoreSpec::new(
                HostResourceType::Player,
                17,
                LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
                "Host Player",
            ),
        )
        .unwrap();
        let after = unix_time_now();
        if before == after {
            let rust_bytes = fs::read(publication.standalone_path.unwrap()).unwrap();
            let oracle_bytes = fs::read(&oracle_player).unwrap();
            assert_eq!(rust_bytes, oracle_bytes);
            return;
        }
    }
    panic!("could not complete the C++ and Rust rewrites within one timestamp second");
}

fn scramble_group_header(buffer: &mut [u8]) {
    buffer.iter_mut().for_each(|byte| *byte ^= 237);
    for index in (0..buffer.len().saturating_sub(2)).step_by(3) {
        buffer.swap(index, index + 2);
    }
}

fn gzip_group_image(image: &[u8]) -> Vec<u8> {
    let mut compressed = Vec::new();
    let mut encoder = GzEncoder::new(&mut compressed, Compression::default());
    encoder.write_all(image).unwrap();
    encoder.finish().unwrap();
    compressed[..2].copy_from_slice(&[0x1e, 0x8c]);
    compressed
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "clonk-rust-host-resource-core-{}-{unique}",
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

fn unix_time_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
