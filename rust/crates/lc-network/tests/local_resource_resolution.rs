use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use lc_engine::{LegacyCString, NetworkResourceCore};
use lc_network::{
    build_host_resource_core, encode_resource_packet, resolve_local_resource,
    resolve_local_resource_with_group_maker, HostResourceCoreSpec, HostResourceType,
    LocalResourceResolution, ResourceCatalogAction, ResourceDiscoverPacket, ResourceFileOwnership,
    ResourcePacket, ResourceTransferBackend, ResourceTransferEvent, PID_NET_RES_STATUS,
};
use lc_resources::{c4group_file_crc, Group, MutableGroup};

#[test]
fn cpp_set_by_core_accepts_an_exact_plain_file() {
    // SetByFile uses the whole-file CRC as the contents CRC, and GetStandalone
    // checks the official size and CRC (src/C4Network2Res.cpp:373-406,659-695).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("System.ocg");
    fs::write(&candidate, b"local").unwrap();
    let core = core(b"System.ocg", 5, 0x8bd6_88e8, 0x8bd6_88e8, true);

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("exact local file should be selected");
    };
    assert_eq!(local.path(), candidate);
    assert_eq!(
        local.standalone_ownership(),
        Some(ResourceFileOwnership::Persistent)
    );
}

#[test]
fn cpp_set_by_core_separates_group_contents_crc_from_physical_crc_and_ignores_sha() {
    // SetByGroup obtains EntryCRC32, then GetStandalone separately checks the
    // packed file size/CRC; CalculateSHA is not called by either acceptance
    // check (src/C4Network2Res.cpp:409-455,659-715; C4Group.cpp:2181-2194).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("Objects.c4d");
    let mut group = MutableGroup::new("Objects.c4d");
    group
        .add_file_with_metadata("DefCore.txt", b"[DefCore]\n".to_vec(), 1, false)
        .unwrap();
    let packed = group.pack().unwrap();
    fs::write(&candidate, &packed).unwrap();
    let mut core = core(
        b"Objects.c4d",
        packed.len() as u32,
        c4group_file_crc(&packed),
        group.contents_crc(),
        true,
    );
    core.file_sha = Some([0xff; 20]);

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("logically and physically exact C4Group should be selected");
    };
    assert_eq!(local.path(), candidate);
}

#[test]
fn cpp_set_by_core_optimizes_a_local_player_before_the_physical_check() {
    // GetStandalone copies persistent players before recursively deleting
    // Portrait*.* and a root BigIcon.png larger than 20 KiB. Only then does it
    // compare the host-published size and file CRC (src/C4Network2Res.cpp:
    // 653-688,1168-1206).
    let directory = TestDirectory::new();
    let player = directory.path().join("Shared.c4p");
    let host_network = directory.path().join("HostNetwork");
    let mismatch_network = directory.path().join("MismatchNetwork");
    let non_player_network = directory.path().join("NonPlayerNetwork");

    let mut crew = MutableGroup::new("Crew.c4i");
    crew.add_file_with_metadata("ObjectInfo.txt", b"crew".to_vec(), 7, false)
        .unwrap();
    crew.add_file_with_metadata("Portrait.bmp", b"nested portrait".to_vec(), 8, false)
        .unwrap();
    let mut original = MutableGroup::new("Shared.c4p");
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

    // Player-group rewrites stamp the current second. Keep the two rewrites in
    // one second, matching the bounded differential pattern in
    // host_resource_core.rs.
    let mut matched = None;
    for attempt in 0..8 {
        let before = unix_time_now();
        let publication = build_host_resource_core(
            &player,
            &host_network,
            HostResourceCoreSpec::new(
                HostResourceType::Player,
                7,
                LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "Shared Player",
            ),
        )
        .unwrap();
        let local_network = directory.path().join(format!("LocalNetwork{attempt}"));
        let resolution = resolve_local_resource_with_group_maker(
            &publication.core,
            [&player],
            &local_network,
            b"Shared Player",
        )
        .unwrap();
        let after = unix_time_now();
        if before != after {
            continue;
        }
        let LocalResourceResolution::Local(local) = resolution else {
            panic!("contents-identical player should remain local");
        };
        assert!(local.binary_compatible());
        matched = Some((publication, local));
        break;
    }
    let (publication, local) =
        matched.expect("could not complete both player rewrites within one timestamp second");
    let host_standalone = publication.standalone_path.as_ref().unwrap();
    let host_bytes = fs::read(host_standalone).unwrap();
    assert_ne!(host_bytes, original_bytes);
    assert_eq!(local.source_path(), player);
    assert_eq!(
        local.standalone_ownership(),
        Some(ResourceFileOwnership::Temporary)
    );
    assert_eq!(
        fs::read(local.standalone_path().unwrap()).unwrap(),
        host_bytes
    );
    assert_eq!(fs::read(&player).unwrap(), original_bytes);
    let optimized = Group::open(local.standalone_path().unwrap()).unwrap();
    assert!(!optimized.exists("Portrait.png"));
    assert!(!optimized.exists("BigIcon.png"));
    assert!(!optimized
        .open_child("Crew.c4i")
        .unwrap()
        .exists("Portrait.bmp"));

    let mut mismatched_core = publication.core.clone();
    mismatched_core.file_crc ^= u32::MAX;
    let resolution = resolve_local_resource_with_group_maker(
        &mismatched_core,
        [&player],
        &mismatch_network,
        b"Shared Player",
    )
    .unwrap();
    let LocalResourceResolution::Local(local) = resolution else {
        panic!("contents-identical player should remain local after physical mismatch");
    };
    assert!(!local.binary_compatible());
    assert!(fs::read_dir(&mismatch_network).unwrap().next().is_none());

    let mut non_player_core = publication.core.clone();
    non_player_core.resource_type = HostResourceType::Definitions as u8;
    non_player_core.file_size = original_bytes.len() as u32;
    non_player_core.file_crc = c4group_file_crc(&original_bytes);
    let resolution =
        resolve_local_resource(&non_player_core, [&player], &non_player_network).unwrap();
    let LocalResourceResolution::Local(local) = resolution else {
        panic!("raw non-player group should remain local");
    };
    assert!(local.binary_compatible());
    assert_eq!(local.path(), player);
    assert_eq!(
        local.standalone_ownership(),
        Some(ResourceFileOwnership::Persistent)
    );
    assert!(!non_player_network.exists());
}

#[test]
fn cpp_set_by_core_packs_a_player_directory_with_the_local_group_maker() {
    // PackDirectoryTo stamps Config.General.Name before OptimizeStandalone.
    // Even when the player has nothing to strip, the packed bytes therefore
    // use the local process maker rather than MutableGroup's default.
    let directory = TestDirectory::new();
    let player = directory.path().join("Directory.c4p");
    fs::create_dir(&player).unwrap();
    fs::write(player.join("Player.txt"), b"player").unwrap();

    let mut matched = None;
    for attempt in 0..8 {
        let before = unix_time_now();
        let publication = build_host_resource_core(
            &player,
            directory.path().join(format!("HostDirectoryNetwork{attempt}")),
            HostResourceCoreSpec::new(
                HostResourceType::Player,
                8,
                LegacyCString::from_bytes(b"Directory.c4p".to_vec()).unwrap(),
                "Shared Player",
            ),
        )
        .unwrap();
        let resolution = resolve_local_resource_with_group_maker(
            &publication.core,
            [&player],
            directory.path().join(format!("LocalDirectoryNetwork{attempt}")),
            b"Shared Player",
        )
        .unwrap();
        let after = unix_time_now();
        if before != after {
            continue;
        }
        let LocalResourceResolution::Local(local) = resolution else {
            panic!("contents-identical player directory should remain local");
        };
        assert!(local.binary_compatible());
        matched = Some(local);
        break;
    }
    let local = matched.expect("could not pack both player directories in one timestamp second");
    assert!(player.is_dir());
    assert_eq!(
        local.standalone_ownership(),
        Some(ResourceFileOwnership::Temporary)
    );
    assert_eq!(
        Group::open(local.standalone_path().unwrap())
            .unwrap()
            .maker(),
        Some("Shared Player")
    );
}

#[test]
fn cpp_set_by_core_opens_a_nested_child_inside_a_packed_c4_group() {
    // C4Group::Open traces a missing filesystem path back to its real packed
    // mother, then opens the remaining `.c4*` path as nested child groups
    // before SetByCore compares EntryCRC32 (src/C4Group.cpp:656-715,
    // 1792-1816; src/C4Network2Res.cpp:441-458).
    let directory = TestDirectory::new();
    let mother_path = directory.path().join("Easy.c4f");
    let mut child = MutableGroup::new("Castle.c4s");
    child
        .add_file_with_metadata("Scenario.txt", b"[Head]\n".to_vec(), 1, false)
        .unwrap();
    let child_contents_crc = child.contents_crc();
    let mut mother = MutableGroup::new("Easy.c4f");
    mother
        .add_child_with_metadata("Castle.c4s", child, 1, false)
        .unwrap();
    fs::write(&mother_path, mother.pack().unwrap()).unwrap();
    let candidate = mother_path.join("Castle.c4s");
    let core = core(
        b"Easy.c4f/Castle.c4s",
        u32::MAX,
        u32::MAX,
        child_contents_crc,
        false,
    );

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("packed nested child should be selected");
    };
    assert_eq!(local.source_path(), candidate);
    assert!(!local.binary_compatible());
}

#[test]
fn cpp_set_by_core_extracts_a_loadable_nested_child_to_a_real_standalone() {
    // GetStandalone copies a virtual child out of its packed mother before it
    // accepts the official file size/CRC and marks the resource complete
    // (src/C4Network2Res.cpp:633-695; src/C4Group.cpp:129-170).
    let directory = TestDirectory::new();
    let mother_path = directory.path().join("Easy.c4f");
    let mut child = MutableGroup::new("Castle.c4s");
    child
        .add_file_with_metadata("Scenario.txt", b"[Head]\n".to_vec(), 1, false)
        .unwrap();
    let child_contents_crc = child.contents_crc();
    let child_raw = child.pack_raw().unwrap();
    let mut mother = MutableGroup::new("Easy.c4f");
    mother
        .add_child_with_metadata("Castle.c4s", child, 1, false)
        .unwrap();
    fs::write(&mother_path, mother.pack().unwrap()).unwrap();
    let candidate = mother_path.join("Castle.c4s");
    let core = core(
        b"Easy.c4f/Castle.c4s",
        child_raw.len() as u32,
        c4group_file_crc(&child_raw),
        child_contents_crc,
        true,
    );

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("packed nested child should be selected");
    };
    assert!(local.binary_compatible());
    let standalone = local
        .standalone_path()
        .expect("loadable packed child must have a physical standalone");
    assert!(standalone.is_file());
    assert_eq!(fs::read(standalone).unwrap(), child_raw);
}

#[test]
fn cpp_group_contents_crc_trusts_a_new_stored_entry_crc() {
    // CalcCRC32 returns immediately for C4GECS_New rather than hashing entry
    // bytes again (src/C4Group.cpp:2444-2450,2510-2516).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("Stored.c4d");
    let mut group = MutableGroup::new("Stored.c4d");
    group
        .add_file_with_metadata("DefCore.txt", b"bytes".to_vec(), 1, false)
        .unwrap();
    let mut raw = group.pack_raw().unwrap();
    let stored_crc = 0x1234_5678_u32;
    raw[204 + 285..204 + 289].copy_from_slice(&stored_crc.to_le_bytes());
    fs::write(&candidate, &raw).unwrap();
    let core = core(
        b"Stored.c4d",
        raw.len() as u32,
        c4group_file_crc(&raw),
        stored_crc,
        true,
    );

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    assert!(matches!(resolution, LocalResourceResolution::Local(_)));
}

#[test]
fn cpp_set_by_core_packs_a_directory_with_the_stock_group_sort_order() {
    // GetStandalone packs directories through C4Group_PackDirectoryTo, which
    // applies C4CFN_FLS sorting before the byte-size/CRC check
    // (src/C4Network2Res.cpp:588-631; C4Group.cpp:260-320,2366-2382).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("Objects.c4d");
    let standalones = directory.path().join("Network");
    fs::create_dir_all(&candidate).unwrap();
    fs::write(candidate.join("Script.c"), b"func Initialize() {}\n").unwrap();
    fs::write(candidate.join("DefCore.txt"), b"[DefCore]\n").unwrap();

    let mut expected = MutableGroup::new("Objects.c4d");
    for filename in ["Script.c", "DefCore.txt"] {
        let path = candidate.join(filename);
        let modified = fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        expected
            .add_file_with_metadata(filename, fs::read(path).unwrap(), modified, false)
            .unwrap();
    }
    let packed = expected.pack().unwrap();
    let core = core(
        b"Objects.c4d",
        packed.len() as u32,
        c4group_file_crc(&packed),
        expected.contents_crc(),
        true,
    );

    let resolution = resolve_local_resource(&core, [&candidate], &standalones).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("matching directory should produce an exact sorted standalone");
    };
    assert_ne!(local.path(), candidate);
    assert_eq!(
        local.standalone_ownership(),
        Some(ResourceFileOwnership::Temporary)
    );
    assert_eq!(fs::read(local.path()).unwrap(), packed);
}

#[test]
fn cpp_directory_contents_crc_uses_c4group_ignore_but_keeps_legacyclonk() {
    // Folder searches exclude dotfiles (except `.legacyclonk`) and the default
    // `cvs;Thumbs.db` ignore modules before EntryCRC32 folds entries
    // (src/C4Group.cpp:89,121-125,1209-1238,2181-2194).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("Objects.c4d");
    let standalones = directory.path().join("Network");
    fs::create_dir_all(&candidate).unwrap();
    fs::write(candidate.join("Data.txt"), b"data").unwrap();
    fs::write(candidate.join(".cache"), b"ignored").unwrap();
    fs::write(candidate.join("Thumbs.db"), b"ignored").unwrap();
    fs::create_dir(candidate.join("cvs")).unwrap();
    fs::write(candidate.join("cvs/ignored.txt"), b"ignored").unwrap();
    fs::write(candidate.join(".legacyclonk"), b"kept").unwrap();

    let mut expected = MutableGroup::new("Objects.c4d");
    for filename in ["Data.txt", ".legacyclonk"] {
        let path = candidate.join(filename);
        let modified = fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        expected
            .add_file_with_metadata(filename, fs::read(path).unwrap(), modified, false)
            .unwrap();
    }
    let packed = expected.pack().unwrap();
    let core = core(
        b"Objects.c4d",
        packed.len() as u32,
        c4group_file_crc(&packed),
        expected.contents_crc(),
        true,
    );

    let resolution = resolve_local_resource(&core, [&candidate], &standalones).unwrap();

    assert!(matches!(resolution, LocalResourceResolution::Local(_)));
}

#[test]
fn exact_match_registers_directly_with_the_transfer_backend() {
    let directory = TestDirectory::new();
    let candidate = directory.path().join("System.ocg");
    fs::write(&candidate, b"local").unwrap();
    let core = core(b"System.ocg", 5, 0x8bd6_88e8, 0x8bd6_88e8, true);
    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();
    let LocalResourceResolution::Local(local) = resolution else {
        panic!("exact local file should be selected");
    };
    let mut backend = ResourceTransferBackend::new(1, directory.path().join("Network")).unwrap();

    local.register(&mut backend).unwrap();

    assert_eq!(backend.core(core.id), Some(&core));
    assert_eq!(backend.path(core.id), Some(candidate.as_path()));
}

#[test]
fn wrong_physical_crc_only_disables_binary_compatibility_and_sha_is_irrelevant() {
    // SetByCore keeps the contents-identical resource even though GetStandalone
    // rejects its whole-file CRC. SHA is not part of either decision
    // (src/C4Network2Res.cpp:441-458,668-715).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("System.ocg");
    fs::write(&candidate, b"local").unwrap();
    let mut core = core(b"System.ocg", 5, 0xdead_beef, 0x8bd6_88e8, true);
    core.file_sha = Some([0; 20]);

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("contents-identical resource must remain local");
    };
    assert!(!local.binary_compatible());
}

#[test]
fn cpp_logical_match_remains_local_when_standalone_is_not_binary_compatible() {
    // SetByCore copies the remote core and returns true immediately after a
    // contents match. Its GetStandalone result is deliberately ignored
    // (src/C4Network2Res.cpp:441-458).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("Local.c4d");
    fs::write(&candidate, b"local").unwrap();
    let mut core = core(b"Local.c4d", 5, 0xdead_beef, 0x8bd6_88e8, true);
    core.chunk_size = 2;

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("contents-identical resource must remain local");
    };
    assert_eq!(local.source_path(), candidate);
    assert_eq!(local.standalone_path(), None);
    assert!(!local.binary_compatible());

    let mut backend = ResourceTransferBackend::new(1, directory.path().join("Network")).unwrap();
    local.register(&mut backend).unwrap();
    assert_eq!(backend.core(core.id), Some(&core));
    assert_eq!(backend.path(core.id), Some(candidate.as_path()));
    let events = backend.on_timer(0, &mut |_| 0).unwrap();
    let status = events
        .iter()
        .find_map(|event| match event {
            ResourceTransferEvent::Transport(ResourceCatalogAction::Broadcast {
                packet: ResourcePacket::Status(status),
            }) => Some(status),
            _ => None,
        })
        .expect("the dirty logical resource broadcasts its empty status");
    assert_eq!(status.chunks.chunk_count, 0);
    assert!(status.chunks.ranges.is_empty());
    let mut expected = vec![PID_NET_RES_STATUS];
    expected.extend_from_slice(&core.id.to_ne_bytes());
    expected.extend_from_slice(&[0, 0]);
    assert_eq!(
        encode_resource_packet(&ResourcePacket::Status(status.clone())).unwrap(),
        expected
    );
    assert!(backend
        .on_packet(
            2,
            &ResourcePacket::Discover(ResourceDiscoverPacket {
                resource_ids: vec![core.id],
            }),
            0,
            &mut |_| 0,
        )
        .unwrap()
        .is_empty());
}

#[test]
fn missing_non_loadable_resource_is_a_typed_fatal_mismatch() {
    // AddLoad refuses a core marked unloadable, so failure to resolve it
    // locally cannot fall back to transfer (src/C4Network2Res.cpp:1473-1506).
    let directory = TestDirectory::new();
    let core = core(b"System.ocg", u32::MAX, u32::MAX, 0x1234_5678, false);

    let resolution =
        resolve_local_resource::<[&Path; 0], &Path>(&core, [], directory.path()).unwrap();

    let LocalResourceResolution::FatalNonLoadable(mismatch) = resolution else {
        panic!("unloadable missing resource must be fatal");
    };
    assert_eq!(mismatch.resource_id, core.id);
    assert_eq!(mismatch.filename, b"System.ocg");
}

#[test]
fn contents_identical_non_loadable_resource_remains_local() {
    // SetByCore succeeds on contents before GetStandalone rejects a core with
    // no official file metadata; AddLoad is only needed when no local match
    // exists (src/C4Network2Res.cpp:441-458,580-582,1473-1506).
    let directory = TestDirectory::new();
    let candidate = directory.path().join("System.ocg");
    fs::write(&candidate, b"local").unwrap();
    let core = core(b"System.ocg", u32::MAX, u32::MAX, 0x8bd6_88e8, false);

    let resolution = resolve_local_resource(&core, [&candidate], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("contents-identical unloadable resource should remain local");
    };
    assert!(!local.binary_compatible());
    let mut backend = ResourceTransferBackend::new(1, directory.path().join("Network")).unwrap();
    local.register(&mut backend).unwrap();
    assert_eq!(backend.path(core.id), Some(candidate.as_path()));
}

#[test]
fn set_by_core_continues_past_a_logical_mismatch_in_candidate_order() {
    // SetByCore recursively probes candidates until contents CRC matches; it
    // stops at that first logical match regardless of standalone compatibility
    // (src/C4Network2Res.cpp:441-493).
    let directory = TestDirectory::new();
    let wrong = directory.path().join("wrong");
    let exact = directory.path().join("exact");
    fs::write(&wrong, b"other").unwrap();
    fs::write(&exact, b"local").unwrap();
    let core = core(b"System.ocg", 5, 0x8bd6_88e8, 0x8bd6_88e8, true);

    let resolution = resolve_local_resource(&core, [&wrong, &exact], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("second exact candidate should be selected");
    };
    assert_eq!(local.path(), exact);
}

#[cfg(unix)]
#[test]
fn set_by_core_continues_past_an_unreadable_candidate() {
    use std::os::unix::fs::PermissionsExt;

    // A failed SetByFile probe returns false, and SetByCore continues its
    // search instead of aborting the join (src/C4Network2Res.cpp:373-390,
    // 441-493).
    let directory = TestDirectory::new();
    let unreadable = directory.path().join("unreadable.bin");
    let exact = directory.path().join("exact.bin");
    fs::write(&unreadable, b"unreadable").unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o0)).unwrap();
    fs::write(&exact, b"local").unwrap();
    let core = core(b"System.ocg", 5, 0x8bd6_88e8, 0x8bd6_88e8, true);

    let resolution = resolve_local_resource(&core, [&unreadable, &exact], directory.path())
        .expect("an unreadable candidate is only a search miss");

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("the later exact candidate should be selected");
    };
    assert_eq!(local.path(), exact);
}

#[test]
fn set_by_core_stops_at_first_logical_match_even_if_later_bytes_are_exact() {
    // Once EntryCRC32 matches, SetByCore returns true even when its ignored
    // GetStandalone call fails; later search candidates are not considered
    // (src/C4Network2Res.cpp:441-458).
    let directory = TestDirectory::new();
    let first = directory.path().join("first.c4d");
    let later = directory.path().join("later.c4d");
    let mut group = MutableGroup::new("Objects.c4d");
    group
        .add_file_with_metadata("DefCore.txt", b"[DefCore]\n".to_vec(), 1, false)
        .unwrap();
    let raw = group.pack_raw().unwrap();
    let packed = group.pack().unwrap();
    fs::write(&first, raw).unwrap();
    fs::write(&later, &packed).unwrap();
    let core = core(
        b"Objects.c4d",
        packed.len() as u32,
        c4group_file_crc(&packed),
        group.contents_crc(),
        true,
    );

    let resolution = resolve_local_resource(&core, [&first, &later], directory.path()).unwrap();

    let LocalResourceResolution::Local(local) = resolution else {
        panic!("first contents-identical candidate should be retained");
    };
    assert_eq!(local.source_path(), first);
    assert!(!local.binary_compatible());
}

fn core(
    filename: &[u8],
    file_size: u32,
    file_crc: u32,
    contents_crc: u32,
    loadable: bool,
) -> NetworkResourceCore {
    NetworkResourceCore {
        id: 7,
        loadable,
        file_size,
        file_crc,
        contents_crc,
        filename: LegacyCString::from_bytes(filename.to_vec()).unwrap(),
        ..NetworkResourceCore::default()
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
            "legacyclonk-local-resource-{}-{unique}",
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
