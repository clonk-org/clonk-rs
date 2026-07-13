use lc_engine::{LegacyCString, NetworkResourceCore};
use lc_network::{ChunkWriteOutcome, ResourceFileOwnership, ResourceFileStore};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn core(id: i32, filename: &[u8], file_size: u32, chunk_size: u32) -> NetworkResourceCore {
    NetworkResourceCore {
        id,
        loadable: true,
        file_size,
        chunk_size,
        filename: LegacyCString::from_bytes(filename.to_vec()).unwrap(),
        ..NetworkResourceCore::default()
    }
}

fn core_with_crc(
    id: i32,
    filename: &[u8],
    file_size: u32,
    chunk_size: u32,
    file_crc: u32,
) -> NetworkResourceCore {
    NetworkResourceCore {
        file_crc,
        ..core(id, filename, file_size, chunk_size)
    }
}

#[test]
fn cpp_remote_temp_creation_sanitizes_basename_and_never_clobbers_a_collision() {
    // FindTempResFileName keeps ASCII alnum, dot, slash; replaces other bytes,
    // takes the basename, uses exclusive `fopen("wxb")`, then tries `_2`
    // through `_999` before the extension (src/C4Network2Res.cpp:1741-1792).
    let directory = TestDirectory::new();
    let collision = directory.path().join("Bad_name_.c4s");
    fs::write(&collision, b"keep me").unwrap();
    let mut store = ResourceFileStore::new(directory.path()).unwrap();

    let path = store
        .create_remote(&core(7, b"folder/Bad name!.c4s", 1, 1))
        .unwrap();

    assert_eq!(path.file_name().unwrap(), "Bad_name__2.c4s");
    assert_eq!(fs::read(collision).unwrap(), b"keep me");
    assert_eq!(fs::metadata(path).unwrap().len(), 0);
}

#[test]
fn cpp_local_complete_registration_retains_non_temporary_file() {
    // SetByFile records whether the source is temporary and marks it complete;
    // Clear deletes only fTempFile paths (src/C4Network2Res.cpp:373-406,
    // 983-1002). A binary-compatible standalone has Core.FileSize bytes
    // (src/C4Network2Res.cpp:659-695).
    let directory = TestDirectory::new();
    let path = directory.path().join("local.c4s");
    fs::write(&path, b"local").unwrap();
    {
        let mut store = ResourceFileStore::new(directory.path()).unwrap();
        store
            .register_local_complete(
                &core_with_crc(8, b"local.c4s", 5, 4, 0x8bd6_88e8),
                &path,
                ResourceFileOwnership::Persistent,
            )
            .unwrap();
        assert!(store.is_complete(8));
        assert_eq!(store.path(8), Some(path.as_path()));
    }

    assert_eq!(fs::read(path).unwrap(), b"local");
}

#[test]
fn cpp_chunk_reads_use_core_offset_but_fixed_100k_length_cap() {
    // C4Network2ResChunk::Set offsets by Core.ChunkSize, but sizes with
    // min(FileSize-offset, C4NetResChunkSize), the fixed 100 KiB constant
    // (src/C4Network2Res.cpp:1230-1260; constant at C4Network2Res.h:27).
    let directory = TestDirectory::new();
    let path = directory.path().join("local.bin");
    fs::write(&path, b"0123456789").unwrap();
    let mut store = ResourceFileStore::new(directory.path()).unwrap();
    store
        .register_local_complete(
            &core_with_crc(9, b"local.bin", 10, 4, 0xa684_c7c6),
            &path,
            ResourceFileOwnership::Persistent,
        )
        .unwrap();

    assert_eq!(store.read_chunk(9, 0).unwrap(), b"0123456789");
    assert_eq!(store.read_chunk(9, 1).unwrap(), b"456789");
    assert_eq!(store.read_chunk(9, 2).unwrap(), b"89");
    assert!(store.read_chunk(9, 3).is_err());
}

#[test]
fn cpp_chunk_writes_accept_short_payloads_and_complete_by_chunk_presence() {
    // AddTo checks only offset+DataSize <= FileSize, writes at
    // chunk*Core.ChunkSize, then marks that chunk present. EndLoad performs no
    // final CRC/SHA or hole validation (src/C4Network2Res.cpp:911-940,
    // 1263-1318; EndLoad at 1113-1122).
    let directory = TestDirectory::new();
    let mut store = ResourceFileStore::new(directory.path()).unwrap();
    let path = store
        .create_remote(&core(10, b"remote.bin", 10, 4))
        .unwrap();

    assert_eq!(
        store.write_chunk(10, 1, b"AB").unwrap(),
        ChunkWriteOutcome::Stored {
            newly_received: true,
            complete: false,
        }
    );
    assert_eq!(
        store.write_chunk(10, 0, b"0123").unwrap(),
        ChunkWriteOutcome::Stored {
            newly_received: true,
            complete: false,
        }
    );
    assert_eq!(
        store.write_chunk(10, 2, b"89").unwrap(),
        ChunkWriteOutcome::Stored {
            newly_received: true,
            complete: true,
        }
    );

    assert!(store.is_complete(10));
    assert_eq!(fs::read(path).unwrap(), b"0123AB\0\089");
}

#[test]
fn cpp_remove_deletes_owned_temp_files_but_preserves_persistent_local_files() {
    // Clear removes szFile only when fTempFile is set; persistent SetByFile
    // registrations survive resource removal (src/C4Network2Res.cpp:983-1002).
    let directory = TestDirectory::new();
    let persistent = directory.path().join("persistent.bin");
    let temporary = directory.path().join("temporary.bin");
    fs::write(&persistent, b"P").unwrap();
    fs::write(&temporary, b"T").unwrap();
    let mut store = ResourceFileStore::new(directory.path()).unwrap();
    store
        .register_local_complete(
            &core_with_crc(11, b"persistent.bin", 1, 1, 0xb969_be79),
            &persistent,
            ResourceFileOwnership::Persistent,
        )
        .unwrap();
    store
        .register_local_complete(
            &core_with_crc(12, b"temporary.bin", 1, 1, 0xbe04_7a60),
            &temporary,
            ResourceFileOwnership::Temporary,
        )
        .unwrap();
    let remote = store.create_remote(&core(13, b"remote.bin", 1, 1)).unwrap();

    store.remove(11).unwrap();
    store.remove(12).unwrap();
    store.remove(13).unwrap();

    assert!(persistent.exists());
    assert!(!temporary.exists());
    assert!(!remote.exists());
}

#[test]
fn failed_final_write_does_not_complete_but_crc_and_sha_are_not_checked() {
    // AddTo rejects only writes beyond FileSize. After all valid chunk indices
    // are present, EndLoad performs no FileCRC or FileSHA verification
    // (src/C4Network2Res.cpp:1269-1318,911-940,1113-1122).
    let directory = TestDirectory::new();
    let mut remote_core = core(14, b"unchecked.bin", 5, 4);
    remote_core.file_crc = 0xdead_beef;
    remote_core.file_sha = Some([0xa5; 20]);
    let mut store = ResourceFileStore::new(directory.path()).unwrap();
    store.create_remote(&remote_core).unwrap();
    store.write_chunk(14, 0, b"1234").unwrap();

    assert!(store.write_chunk(14, 1, b"XX").is_err());
    assert!(!store.is_complete(14));
    assert_eq!(
        store.write_chunk(14, 1, b"5").unwrap(),
        ChunkWriteOutcome::Stored {
            newly_received: true,
            complete: true,
        }
    );
    assert!(store.is_complete(14));
    assert!(store.write_chunk(14, 1, b"5").is_err());
}

#[test]
fn cpp_local_binary_compatibility_checks_whole_file_crc_but_not_sha() {
    // GetStandalone checks exact FileSize and FileCRC against the remote core;
    // it does not compare FileSHA (src/C4Network2Res.cpp:659-695,700-715).
    let directory = TestDirectory::new();
    let path = directory.path().join("local.bin");
    fs::write(&path, b"local").unwrap();
    let mut wrong = core_with_crc(15, b"local.bin", 5, 4, 0xdead_beef);
    wrong.file_sha = Some([0; 20]);
    let mut store = ResourceFileStore::new(directory.path()).unwrap();
    assert!(store
        .register_local_complete(&wrong, &path, ResourceFileOwnership::Persistent)
        .is_err());

    let mut exact = core_with_crc(15, b"local.bin", 5, 4, 0x8bd6_88e8);
    exact.file_sha = Some([0xff; 20]);
    store
        .register_local_complete(&exact, &path, ResourceFileOwnership::Persistent)
        .unwrap();
}

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let unique = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "legacyclonk-resource-store-{}-{unique}",
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
