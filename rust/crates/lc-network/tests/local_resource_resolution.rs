use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lc_engine::{LegacyCString, NetworkResourceCore};
use lc_network::{
    resolve_local_resource, LocalResourceResolution, ResourceDiscoverPacket, ResourceFileOwnership,
    ResourcePacket, ResourceTransferBackend,
};
use lc_resources::{c4group_file_crc, MutableGroup};

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
    let core = core(b"Local.c4d", 5, 0xdead_beef, 0x8bd6_88e8, true);

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
