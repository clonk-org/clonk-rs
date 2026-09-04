#![allow(dead_code)] // The writer is path-included until its public integration slice.

#[path = "../src/group_writer.rs"]
mod group_writer;

use clonk_resources::{Group, MutableGroup, MutableGroupChildMut, MutableGroupError};
use group_writer::{c4group_entry_crc, c4group_file_crc, compress_c4group_for_test};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

#[cfg(unix)]
fn set_mtime(path: &Path, seconds: i64) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).expect("path has no NUL");
    let times = [
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: 0,
        },
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: 0,
        },
    ];
    assert_eq!(
        unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), 0) },
        0,
        "set source directory mtime"
    );
}

#[test]
fn cpp_nonempty_file_crc_extends_data_crc_with_entry_filename() {
    // CalcCRC32 first calls zlib crc32 over the bytes, then continues the CRC
    // over FileName (src/C4Group.cpp:2473-2512).
    let mut group = MutableGroup::new("Test.c4g");
    group.add_file("file.txt", b"hello".to_vec()).unwrap();

    assert_eq!(group.entry_crc("file.txt"), Some(0x7c6f_3391));
    assert_eq!(group.contents_crc(), 0x7c6f_3391);
}

#[test]
fn cpp_whole_file_crc_is_the_zlib_crc32_stream() {
    // C4Group_GetFileCRC starts at zero and chains zlib crc32 over every file
    // chunk (src/C4Group.cpp:429-469).
    assert_eq!(c4group_file_crc(b"123456789"), 0xcbf4_3926);
}

#[test]
fn cpp_empty_file_crc_is_zero_and_omits_its_filename() {
    // The zero-size branch assigns zero and bypasses filename extension
    // (src/C4Group.cpp:2470-2472,2510-2512).
    let mut group = MutableGroup::new("Test.c4g");
    group.add_file("name-is-ignored.txt", Vec::new()).unwrap();

    assert_eq!(group.entry_crc("name-is-ignored.txt"), Some(0));
    assert_eq!(group.contents_crc(), 0);
}

#[test]
fn cpp_child_crc_is_recursive_xor_and_omits_child_entry_name() {
    // Child entries take Child.EntryCRC32 directly; EntryCRC32 XORs every
    // child entry CRC. The child group entry name is never appended
    // (src/C4Group.cpp:2181-2193,2449-2471).
    let mut child = MutableGroup::new("Test.c4g");
    child.add_file("a.txt", b"abc".to_vec()).unwrap();
    child.add_file("b.txt", b"xyz".to_vec()).unwrap();
    assert_eq!(child.contents_crc(), 0x6554_cbb7);

    let mut first_parent = MutableGroup::new("Test.c4g");
    first_parent.add_child("First.c4g", child.clone()).unwrap();
    let mut renamed_parent = MutableGroup::new("Test.c4g");
    renamed_parent.add_child("Renamed.c4g", child).unwrap();

    assert_eq!(first_parent.entry_crc("First.c4g"), Some(0x6554_cbb7));
    assert_eq!(first_parent.contents_crc(), renamed_parent.contents_crc());
}

#[test]
fn mutable_group_rename_is_collision_safe_and_preserves_entry_metadata() {
    let mut child = MutableGroup::new("Old.bin");
    child.add_file("z.raw", b"tail".to_vec()).unwrap();
    child.add_file("Scenario.txt", b"core".to_vec()).unwrap();

    let mut parent = MutableGroup::new("Parent.c4g");
    parent
        .add_child_with_metadata("Old.bin", child, 0xa0b0_c0d0, true)
        .unwrap();
    parent.add_file("Taken.c4s", b"taken".to_vec()).unwrap();

    assert_eq!(
        parent.rename_entry_checked("Old.bin", "taken.C4S"),
        Err(MutableGroupError::EntryAlreadyExists(
            "taken.C4S".to_string()
        ))
    );
    assert_eq!(parent.entry_names(), ["Old.bin", "Taken.c4s"]);

    assert!(parent
        .rename_entry_checked("old.BIN", "Renamed.c4s")
        .unwrap());
    assert_eq!(parent.entry_names(), ["Renamed.c4s", "Taken.c4s"]);

    let packed = parent.pack_raw().unwrap();
    let group = Group::from_memory(PathBuf::from("Parent.c4g"), packed).unwrap();
    let renamed = group
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.relative_path == std::path::Path::new("Renamed.c4s"))
        .unwrap();
    assert_eq!(renamed.time, 0xa0b0_c0d0);
    assert!(renamed.executable);

    let child = group.open_child("Renamed.c4s").unwrap();
    assert_eq!(child.read_file("Scenario.txt").unwrap(), b"core");
    assert_eq!(child.read_file("z.raw").unwrap(), b"tail");
    assert_eq!(
        packed_entry_names(&child.raw_image().unwrap()),
        ["Scenario.txt", "z.raw"],
        "the renamed child's filename selects the scenario sort list"
    );
}

#[test]
fn mutable_group_rename_preserves_cpp_cached_new_crc_for_imported_file() {
    const OLD_FILE_NAME: &str = "Old.txt";
    const NEW_FILE_NAME: &str = "New.txt";
    const PAYLOAD: &[u8] = b"payload";

    let stored_data_crc = 0x1122_3344;
    let cached_file_crc = 0xa1b2_c3d4;
    for (crc_state, stored_crc, expected_crc) in [
        (
            0,
            0x0102_0304,
            c4group_entry_crc(PAYLOAD, NEW_FILE_NAME.as_bytes()),
        ),
        (
            1,
            stored_data_crc,
            crc32_update(stored_data_crc, NEW_FILE_NAME.as_bytes()),
        ),
        (2, cached_file_crc, cached_file_crc),
        (
            7,
            0x5566_7788,
            c4group_entry_crc(PAYLOAD, NEW_FILE_NAME.as_bytes()),
        ),
    ] {
        let mut source = MutableGroup::new("Files.c4g");
        source.add_file(OLD_FILE_NAME, PAYLOAD.to_vec()).unwrap();
        let source = Group::from_memory(
            PathBuf::from("Files.c4g"),
            with_first_entry_crc(source.pack_raw().unwrap(), crc_state, stored_crc),
        )
        .unwrap();
        let mut rewritten = MutableGroup::from_group(&source).unwrap();

        assert!(rewritten
            .rename_entry_checked(OLD_FILE_NAME, NEW_FILE_NAME)
            .unwrap());
        assert_eq!(rewritten.entry_crc(NEW_FILE_NAME), Some(expected_crc));

        let reopened =
            Group::from_memory(PathBuf::from("Files.c4g"), rewritten.pack_raw().unwrap()).unwrap();
        let renamed = reopened.entries().unwrap().remove(0);
        assert_eq!(renamed.crc_state, 2, "source CRC state {crc_state}");
        assert_eq!(
            renamed.stored_crc, expected_crc,
            "source CRC state {crc_state}"
        );
        assert_eq!(reopened.contents_crc().unwrap(), expected_crc);
    }

    let cached_empty_crc = 0xdec0_adde;
    for (crc_state, stored_crc, expected_crc) in [
        (0, 0x3141_5926, 0),
        (1, 0x2718_2818, 0),
        (2, cached_empty_crc, cached_empty_crc),
        (7, 0x1618_0339, 0),
    ] {
        let mut source = MutableGroup::new("Empty.c4g");
        source.add_file(OLD_FILE_NAME, Vec::new()).unwrap();
        let source = Group::from_memory(
            PathBuf::from("Empty.c4g"),
            with_first_entry_crc(source.pack_raw().unwrap(), crc_state, stored_crc),
        )
        .unwrap();
        let mut rewritten = MutableGroup::from_group(&source).unwrap();

        assert!(rewritten
            .rename_entry_checked(OLD_FILE_NAME, NEW_FILE_NAME)
            .unwrap());
        let reopened =
            Group::from_memory(PathBuf::from("Empty.c4g"), rewritten.pack_raw().unwrap()).unwrap();
        let renamed = reopened.entries().unwrap().remove(0);
        assert_eq!(renamed.crc_state, 2, "empty source CRC state {crc_state}");
        assert_eq!(
            renamed.stored_crc, expected_crc,
            "empty source CRC state {crc_state}"
        );
        assert_eq!(reopened.contents_crc().unwrap(), expected_crc);
    }

    let mut child = MutableGroup::new("Old.c4g");
    child
        .add_file("Inside.txt", b"child payload".to_vec())
        .unwrap();
    let child_crc = child.contents_crc();
    let cached_child_crc = 0xcafe_babe;
    for (crc_state, stored_crc, expected_crc) in [
        (0, 0x1020_3040, child_crc),
        (1, 0x5060_7080, child_crc),
        (2, cached_child_crc, cached_child_crc),
        (7, 0x90a0_b0c0, child_crc),
    ] {
        let mut source = MutableGroup::new("Parent.c4g");
        source.add_child("Old.c4g", child.clone()).unwrap();
        let source = Group::from_memory(
            PathBuf::from("Parent.c4g"),
            with_first_entry_crc(source.pack_raw().unwrap(), crc_state, stored_crc),
        )
        .unwrap();
        let mut rewritten = MutableGroup::from_group(&source).unwrap();
        let before_rename = rewritten.entry_crc("Old.c4g");

        assert!(rewritten
            .rename_entry_checked("Old.c4g", "Renamed.c4g")
            .unwrap());
        assert_eq!(rewritten.entry_crc("Renamed.c4g"), before_rename);

        let reopened =
            Group::from_memory(PathBuf::from("Parent.c4g"), rewritten.pack_raw().unwrap()).unwrap();
        let renamed = reopened.entries().unwrap().remove(0);
        assert_eq!(renamed.crc_state, 2, "child source CRC state {crc_state}");
        assert_eq!(
            renamed.stored_crc, expected_crc,
            "child source CRC state {crc_state}"
        );
        assert_eq!(reopened.contents_crc().unwrap(), expected_crc);
    }
}

#[test]
fn rewrite_preserves_unopenable_child_when_crc_calculation_fails() {
    const CHILD_NAME: &str = "Broken.c4g";
    const CHILD_TIME: u32 = 0x1234_5678;
    const FIRST_CORE: usize = 204;
    const CORE_SIZE: usize = 316;
    const ENTRY_COUNT: usize = 2;

    let invalid_child = vec![0xa5; 211];
    for (crc_state, stored_crc) in [(0, 0x1020_3040), (1, 0x5060_7080)] {
        let mut source = MutableGroup::new("Opaque.bin");
        source
            .add_packed_child_with_metadata(
                CHILD_NAME,
                invalid_child.clone(),
                0xdead_beef,
                CHILD_TIME,
                true,
            )
            .unwrap();
        source
            .add_file_with_metadata("Sibling.txt", b"old".to_vec(), 7, false)
            .unwrap();
        let mut source_image = source.pack_raw().unwrap();
        set_entry_crc(&mut source_image, 0, crc_state, stored_crc);
        let original_core = source_image[FIRST_CORE..FIRST_CORE + CORE_SIZE].to_vec();

        let source = Group::from_memory(PathBuf::from("Opaque.bin"), source_image).unwrap();
        let broken = source
            .entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.name_bytes == CHILD_NAME.as_bytes())
            .unwrap();
        assert_eq!(
            source.read_entry_bytes_exact(&broken).unwrap(),
            invalid_child,
            "state {crc_state} has a complete physical payload"
        );
        assert!(
            source.open_child(CHILD_NAME).is_err(),
            "state {crc_state} payload must remain unopenable"
        );

        let mut rewritten = MutableGroup::from_group(&source).unwrap();
        assert!(matches!(
            rewritten.child_mut(CHILD_NAME),
            Err(MutableGroupError::SourceGroup(_))
        ));
        rewritten
            .add_file_with_metadata("Sibling.txt", b"changed".to_vec(), 9, false)
            .unwrap();
        assert_eq!(rewritten.entry_crc(CHILD_NAME), Some(0));
        assert_eq!(rewritten.contents_crc(), 0);

        let rewritten_image = rewritten.pack_raw().unwrap();
        assert_eq!(
            &rewritten_image[FIRST_CORE..FIRST_CORE + CORE_SIZE],
            original_core.as_slice(),
            "state {crc_state} child core"
        );
        let payload_start = FIRST_CORE + ENTRY_COUNT * CORE_SIZE;
        assert_eq!(
            &rewritten_image[payload_start..payload_start + invalid_child.len()],
            invalid_child.as_slice(),
            "state {crc_state} child payload"
        );
        assert_eq!(
            entry_crc_core(&rewritten_image, 1),
            (0, 0),
            "CRC traversal stops before the replacement sibling"
        );

        let reopened = Group::from_memory(PathBuf::from("Opaque.bin"), rewritten_image).unwrap();
        let entries = reopened.entries().unwrap();
        let broken = entries
            .iter()
            .find(|entry| entry.name_bytes == CHILD_NAME.as_bytes())
            .unwrap();
        assert_eq!(
            (broken.crc_state, broken.stored_crc),
            (crc_state, stored_crc)
        );
        assert_eq!(broken.time, CHILD_TIME);
        assert!(broken.executable);
        assert_eq!(reopened.read_file("Sibling.txt").unwrap(), b"changed");
    }
}

#[test]
fn rewrite_rejects_truncated_unopenable_child_payload() {
    let mut source = MutableGroup::new("Truncated.bin");
    source
        .add_packed_child_with_metadata("Broken.c4g", vec![0xa5; 211], 0xdead_beef, 7, false)
        .unwrap();
    let mut source_image = source.pack_raw().unwrap();
    set_entry_crc(&mut source_image, 0, 0, 0x1020_3040);
    source_image.pop();

    let source = Group::from_memory(PathBuf::from("Truncated.bin"), source_image).unwrap();
    let entry = source.entries().unwrap().remove(0);
    assert!(source.read_entry_bytes_exact(&entry).is_err());
    assert!(matches!(
        MutableGroup::from_group(&source),
        Err(MutableGroupError::SourceGroup(message)) if message.contains("exceeds group bounds")
    ));
}

#[test]
fn rewrite_promotes_openable_child_with_nested_crc_failure_to_zero() {
    let mut child = MutableGroup::new("Direct.c4g");
    child
        .add_packed_child_with_metadata("Nested.c4g", vec![0x5a; 204], 0x1111_2222, 3, false)
        .unwrap();
    let mut child_image = child.pack_raw().unwrap();
    set_entry_crc(&mut child_image, 0, 0, 0x3333_4444);

    let mut parent = MutableGroup::new("Parent.bin");
    parent
        .add_packed_child_with_metadata("Direct.c4g", child_image, 0x5555_6666, 5, false)
        .unwrap();
    let sibling_payload = b"sibling";
    parent
        .add_file("Sibling.txt", sibling_payload.to_vec())
        .unwrap();
    let mut parent_image = parent.pack_raw().unwrap();
    set_entry_crc(&mut parent_image, 0, 1, 0x7777_8888);
    set_entry_crc(&mut parent_image, 1, 0, 0x9999_aaaa);

    let source = Group::from_memory(PathBuf::from("Parent.bin"), parent_image).unwrap();
    let rewritten = MutableGroup::from_group(&source).unwrap();
    let sibling_crc = c4group_entry_crc(sibling_payload, b"Sibling.txt");
    assert_eq!(rewritten.entry_crc("Direct.c4g"), Some(0));
    assert_eq!(rewritten.contents_crc(), sibling_crc);
    let rewritten_image = rewritten.pack_raw().unwrap();
    assert_eq!(entry_crc_core(&rewritten_image, 0), (2, 0));
    assert_eq!(entry_crc_core(&rewritten_image, 1), (2, sibling_crc));
}

#[test]
fn rewrite_crc_pass_stops_after_first_unopenable_child() {
    let mut source = MutableGroup::new("Player.c4p");
    source
        .add_packed_child_with_metadata("Broken.c4i", vec![0xa5; 211], 0x1111_2222, 2, false)
        .unwrap();
    source.add_file("After.raw", b"after".to_vec()).unwrap();
    source.add_file("Player.txt", b"old".to_vec()).unwrap();
    let mut source_image = source.pack_raw().unwrap();
    set_entry_crc(&mut source_image, 0, 0, 0x0102_0304);
    set_entry_crc(&mut source_image, 1, 1, 0x1112_1314);
    set_entry_crc(&mut source_image, 2, 1, 0x2122_2324);

    let source = Group::from_memory(PathBuf::from("Player.c4p"), source_image).unwrap();
    let mut rewritten = MutableGroup::from_group(&source).unwrap();
    let replacement = b"replacement";
    rewritten
        .add_file("Player.txt", replacement.to_vec())
        .unwrap();
    let rewritten_image = rewritten.pack_raw().unwrap();

    assert_eq!(
        entry_crc_core(&rewritten_image, 0),
        (2, c4group_entry_crc(replacement, b"Player.txt")),
        "Player.txt is visited before the earlier-inserted *.c4i after stock sorting"
    );
    assert_eq!(entry_crc_core(&rewritten_image, 1), (1, 0x1112_1314));
    assert_eq!(entry_crc_core(&rewritten_image, 2), (1, 0x2122_2324));
}

#[test]
fn rewrite_cached_unopenable_child_does_not_stop_crc_pass() {
    let cached_crc = 0xcafe_babe;
    let sibling_payload = b"sibling";
    let sibling_crc = c4group_entry_crc(sibling_payload, b"Sibling.txt");
    let mut source = MutableGroup::new("Cached.bin");
    source
        .add_packed_child_with_metadata("Broken.c4g", vec![0xa5; 211], cached_crc, 4, false)
        .unwrap();
    source
        .add_file("Sibling.txt", sibling_payload.to_vec())
        .unwrap();
    let mut source_image = source.pack_raw().unwrap();
    set_entry_crc(&mut source_image, 1, 0, 0x0102_0304);

    let source = Group::from_memory(PathBuf::from("Cached.bin"), source_image).unwrap();
    let mut rewritten = MutableGroup::from_group(&source).unwrap();
    assert_eq!(rewritten.entry_crc("Broken.c4g"), Some(cached_crc));
    assert_eq!(rewritten.contents_crc(), cached_crc ^ sibling_crc);
    assert!(matches!(
        rewritten.child_mut("Broken.c4g"),
        Err(MutableGroupError::SourceGroup(_))
    ));

    let rewritten_image = rewritten.pack_raw().unwrap();
    assert_eq!(entry_crc_core(&rewritten_image, 0), (2, cached_crc));
    assert_eq!(entry_crc_core(&rewritten_image, 1), (2, sibling_crc));
}

#[test]
fn rewrite_child_crc_uses_open_as_child_wildcard_rejection() {
    let mut valid_child = MutableGroup::new("Wildcard.c4g");
    valid_child
        .add_file("Inside.txt", b"valid".to_vec())
        .unwrap();
    let valid_crc = valid_child.contents_crc();
    let mut source = MutableGroup::new("Nested.bin");
    source
        .add_packed_child_with_metadata(
            "A*.c4g",
            valid_child.pack_raw().unwrap(),
            valid_crc,
            6,
            false,
        )
        .unwrap();
    let mut source_image = source.pack_raw().unwrap();
    set_entry_crc(&mut source_image, 0, 0, 0x1234_5678);

    let source = Group::from_raw_memory(PathBuf::from("Nested.bin"), source_image).unwrap();
    let rewritten = MutableGroup::from_group(&source).unwrap();
    assert_eq!(rewritten.entry_crc("A*.c4g"), Some(0));
    assert_eq!(
        entry_crc_core(&rewritten.pack_raw().unwrap(), 0),
        (0, 0x1234_5678)
    );
}

#[test]
fn rewrite_nested_child_crc_uses_open_as_child_wildcard_rejection() {
    let mut leaf = MutableGroup::new("Leaf.c4g");
    leaf.add_file("Inside.txt", b"valid".to_vec()).unwrap();
    let leaf_crc = leaf.contents_crc();

    let mut middle = MutableGroup::new("Middle.c4g");
    middle
        .add_packed_child_with_metadata("A*.c4g", leaf.pack_raw().unwrap(), leaf_crc, 6, false)
        .unwrap();
    let mut middle_image = middle.pack_raw().unwrap();
    set_entry_crc(&mut middle_image, 0, 0, 0x1234_5678);
    let opened_middle =
        Group::from_raw_memory(PathBuf::from("Middle.c4g"), middle_image.clone()).unwrap();
    assert!(opened_middle.contents_crc().is_err());
    assert_eq!(opened_middle.contents_crc_or_zero(), 0);

    let sibling_payload = b"sibling";
    let sibling_crc = c4group_entry_crc(sibling_payload, b"Sibling.txt");
    let mut parent = MutableGroup::new("Parent.bin");
    parent
        .add_packed_child_with_metadata("Middle.c4g", middle_image, 0x8765_4321, 7, false)
        .unwrap();
    parent
        .add_file("Sibling.txt", sibling_payload.to_vec())
        .unwrap();
    let mut parent_image = parent.pack_raw().unwrap();
    set_entry_crc(&mut parent_image, 0, 0, 0x1122_3344);
    set_entry_crc(&mut parent_image, 1, 0, 0x5566_7788);

    let source = Group::from_memory(PathBuf::from("Parent.bin"), parent_image).unwrap();
    let rewritten = MutableGroup::from_group(&source).unwrap();
    assert_eq!(rewritten.entry_crc("Middle.c4g"), Some(0));
    assert_eq!(rewritten.contents_crc(), sibling_crc);
    let rewritten_image = rewritten.pack_raw().unwrap();
    assert_eq!(entry_crc_core(&rewritten_image, 0), (2, 0));
    assert_eq!(entry_crc_core(&rewritten_image, 1), (2, sibling_crc));
}

#[test]
fn materialized_child_crc_uses_child_close_sort_for_question_match() {
    let mut pattern_child = MutableGroup::new("A?.c4g");
    pattern_child
        .add_file("Inside.txt", b"valid".to_vec())
        .unwrap();

    let mut player = MutableGroup::new("Player.c4p");
    player
        .add_child("A?.c4g", pattern_child)
        .expect("question-mark child is accepted inside a nested group");
    player
        .add_file("A0.c4g", b"plain first match after sorting".to_vec())
        .unwrap();

    let mut parent = MutableGroup::new("Parent.bin");
    parent.add_child("Player.c4p", player).unwrap();
    let parent_image = parent.pack_raw().unwrap();
    assert_eq!(entry_crc_core(&parent_image, 0), (2, 0));

    let opened_parent = Group::from_memory(PathBuf::from("Parent.bin"), parent_image).unwrap();
    let opened_player = opened_parent.open_child("Player.c4p").unwrap();
    let entries = opened_player.entries().unwrap();
    assert_eq!(entries[0].name_bytes, b"A0.c4g");
    assert_eq!(entries[0].crc_state, 2);
    assert_eq!(entries[1].name_bytes, b"A?.c4g");
    assert_eq!(entries[1].crc_state, 0);
    assert!(opened_player.contents_crc().is_err());
    assert_eq!(opened_player.contents_crc_or_zero(), 0);
}

fn with_first_entry_crc(mut image: Vec<u8>, crc_state: u8, stored_crc: u32) -> Vec<u8> {
    set_entry_crc(&mut image, 0, crc_state, stored_crc);
    image
}

fn set_entry_crc(image: &mut [u8], index: usize, crc_state: u8, stored_crc: u32) {
    const FIRST_ENTRY_CORE: usize = 204;
    const ENTRY_CORE_SIZE: usize = 316;
    const CRC_STATE_OFFSET: usize = 284;
    const STORED_CRC_OFFSET: usize = 285;

    let core = FIRST_ENTRY_CORE + index * ENTRY_CORE_SIZE;
    image[core + CRC_STATE_OFFSET] = crc_state;
    image[core + STORED_CRC_OFFSET..core + STORED_CRC_OFFSET + 4]
        .copy_from_slice(&stored_crc.to_le_bytes());
}

fn entry_crc_core(image: &[u8], index: usize) -> (u8, u32) {
    const FIRST_ENTRY_CORE: usize = 204;
    const ENTRY_CORE_SIZE: usize = 316;
    const CRC_STATE_OFFSET: usize = 284;
    const STORED_CRC_OFFSET: usize = 285;

    let core = FIRST_ENTRY_CORE + index * ENTRY_CORE_SIZE;
    (
        image[core + CRC_STATE_OFFSET],
        u32::from_le_bytes(
            image[core + STORED_CRC_OFFSET..core + STORED_CRC_OFFSET + 4]
                .try_into()
                .unwrap(),
        ),
    )
}

fn crc32_update(initial: u32, data: &[u8]) -> u32 {
    let mut crc = initial ^ u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc ^ u32::MAX
}

#[test]
fn group_original_marker_is_exact_and_directories_are_not_original() {
    let mut mutable = MutableGroup::new("Original.c4g");
    mutable.make_original(true);
    let original =
        Group::from_memory(PathBuf::from("Original.c4g"), mutable.pack_raw().unwrap()).unwrap();
    assert!(original.is_original());

    mutable.make_original(false);
    let ordinary =
        Group::from_memory(PathBuf::from("Ordinary.c4g"), mutable.pack_raw().unwrap()).unwrap();
    assert!(!ordinary.is_original());

    let directory = tempdir().unwrap();
    let unpacked = directory.path().join("Unpacked.c4g");
    std::fs::create_dir(&unpacked).unwrap();
    assert!(!Group::open(unpacked).unwrap().is_original());
}

/// A group whose filename selects no stock sort list keeps the order its
/// entries were added in, all the way through `pack` and back out of `Group`.
///
/// This is the property clonk-org/clonk-rs#382 needs. `C4Group`'s folder scan
/// is unsorted and `C4MaterialMap::Load` takes material slots straight from it
/// (`C4Material.cpp:263-299`), so a shipped *unpacked* `Material.c4g` makes
/// material indices depend on the host filesystem. Packing fixes an order into
/// the archive — but `Material.c4g` matches `C4FLS_MATERIAL` in the stock
/// `C4CFN_FLS` table, so packing under that name applies the stock sort and
/// moves every index. Building the archive under a name that matches no
/// pattern is what allows a *chosen* order — a recording host's `readdir`
/// order, say — to be pinned without moving anything.
#[test]
fn a_group_outside_the_stock_sort_table_packs_in_insertion_order() {
    // Neither alphabetical nor reverse, and deliberately the kind of order a
    // `readdir` produces rather than one any sort would.
    let names = ["zulu.txt", "alpha.txt", "mike.txt", "bravo.txt"];

    let mut pinned = MutableGroup::new("Pinned.c4g");
    for name in names {
        pinned.add_file(name, name.as_bytes().to_vec()).unwrap();
    }
    assert_eq!(pinned.entry_names(), names, "added in the order given");

    let directory = tempdir().unwrap();
    let path = directory.path().join("Material.c4g");
    std::fs::write(&path, pinned.pack().unwrap()).unwrap();

    // Read back under the name that *does* select a sort list: the archive
    // carries no filename, so nothing re-sorts on load. Sorting happens in
    // `Close`/`Save`, never in `Load`.
    let group = Group::open(&path).unwrap();
    let read_back = group
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.relative_path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(read_back, names, "the packed order survives the round trip");
    for name in names {
        assert_eq!(group.read_file(name).unwrap(), name.as_bytes());
    }

    // The contrast that makes the choice of name load-bearing: the same
    // entries under `Material.c4g` come back in the stock sort's order.
    let mut sorted = MutableGroup::new("Material.c4g");
    for name in names {
        sorted.add_file(name, name.as_bytes().to_vec()).unwrap();
    }
    let sorted_path = directory.path().join("Sorted.c4g");
    std::fs::write(&sorted_path, sorted.pack().unwrap()).unwrap();
    let sorted_back = Group::open(&sorted_path)
        .unwrap()
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.relative_path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_ne!(
        sorted_back, read_back,
        "packing under a stock-sorted name must not preserve insertion order, \
         or this test proves nothing about the name mattering"
    );
}

#[test]
fn cpp_sort_uses_first_matching_rank_then_case_insensitive_filename() {
    // SortRank returns the first matching segment's descending rank. Sort then
    // orders higher ranks first and equal ranks by stricmp filename, retaining
    // insertion order only when both compare equal
    // (src/C4Group.cpp:2288-2337; wildcard rules at src/StdFile.cpp:337-368).
    let mut group = MutableGroup::new("Test.c4g");
    for name in ["z.raw", "b.txt", "a.bin", "scenario.txt", "A.txt", "c.raw"] {
        group.add_file(name, vec![1]).unwrap();
    }

    assert!(group.sort("Scenario.txt|*.txt|*.bin"));
    assert_eq!(
        group.entry_names(),
        ["scenario.txt", "A.txt", "b.txt", "a.bin", "c.raw", "z.raw",]
    );
}

#[test]
fn cpp_pack_automatically_applies_the_scenario_file_sort_list() {
    // C4Application installs C4CFN_FLS, and Close selects the *.c4s entry and
    // calls Sort before Save (src/C4Application.cpp:118-122;
    // src/C4Group.cpp:54-72,947-951,2366-2382).
    let mut group = MutableGroup::new("Melee.c4s");
    for name in ["z.raw", "Script.c", "Landscape.png", "Scenario.txt"] {
        group.add_file(name, vec![1]).unwrap();
    }

    assert_eq!(
        packed_entry_names(&group.pack_raw().unwrap()),
        ["Scenario.txt", "Landscape.png", "Script.c", "z.raw"]
    );
}

#[test]
fn cpp_pack_selects_every_standard_c4cfn_fls_group_family() {
    // C4CFN_FLS maps these exact/group-wildcard names to their load sequence;
    // SortByList uses the first filename match (src/C4Group.cpp:54-72,2366-2382).
    for (filename, ranked_entry) in [
        ("System.c4g", "Names.txt"),
        ("Mouse.c4f", "Tutorial01.c4s"),
        ("Keyboard.c4f", "Tutorial10.c4s"),
        ("Easy.c4f", "Castle.c4s"),
        ("Material.c4g", "TexMap.txt"),
        ("Graphics.c4g", "Magic.png"),
        ("Western.c4f", "Folder.txt"),
        ("Objects.c4d", "DefCore.txt"),
        ("Player.c4p", "Player.txt"),
        ("Crew.c4i", "ObjectInfo.txt"),
        ("Melee.c4s", "Scenario.txt"),
        ("Missions.c4f", "Folder.txt"),
        ("Sect001.c4g", "Game.txt"),
        ("Music.c4g", "Frontend.ogg"),
    ] {
        let mut group = MutableGroup::new(filename);
        group.add_file("000.unmatched", vec![1]).unwrap();
        group.add_file(ranked_entry, vec![1]).unwrap();

        assert_eq!(
            packed_entry_names(&group.pack_raw().unwrap())[0],
            ranked_entry,
            "standard sort list for {filename}"
        );
    }
}

#[test]
fn cpp_pack_sorts_a_child_by_its_parent_entry_name() {
    // Recursive directory packing selects C4CFN_FLS with the logical source
    // directory/entry name, not the temporary output path
    // (src/C4Group.cpp:274-326,1118-1134).
    let mut child = MutableGroup::new("Test.c4g");
    child.add_file("000.unmatched", vec![1]).unwrap();
    child.add_file("Scenario.txt", vec![1]).unwrap();
    let mut parent = MutableGroup::new("Test.c4g");
    parent.add_child("Child.c4s", child).unwrap();

    let image = parent.pack_raw().unwrap();
    assert_eq!(
        packed_entry_names(&image[204 + 316..]),
        ["Scenario.txt", "000.unmatched"]
    );
}

#[test]
fn cpp_case_insensitive_replacement_appends_the_new_entry() {
    // AddEntry marks an existing same-name entry deleted, then appends the new
    // entry at list tail (src/C4Group.cpp:849-891; lookup at 896-904).
    let mut group = MutableGroup::new("Test.c4g");
    group.add_file("Same.txt", b"old".to_vec()).unwrap();
    group.add_file("other.txt", b"other".to_vec()).unwrap();
    group.add_file("same.TXT", b"new".to_vec()).unwrap();

    assert_eq!(group.entry_names(), ["other.txt", "same.TXT"]);
    assert_eq!(group.entry_crc("SAME.txt"), group.entry_crc("same.TXT"));
}

#[test]
fn mutable_group_rename_is_case_insensitive_and_preserves_packed_child_metadata() {
    let mut crew = MutableGroup::new("Veteran.c4i");
    crew.add_file("ObjectInfo.txt", b"crew core".to_vec())
        .unwrap();
    let crew_crc = crew.contents_crc();
    let crew_image = crew.pack_raw().unwrap();

    let mut player = MutableGroup::new("Player.c4p");
    player
        .add_packed_child_with_metadata(
            "Veteran.c4i",
            crew_image.clone(),
            crew_crc,
            0x1234_5678,
            true,
        )
        .unwrap();
    let source = Group::from_memory(PathBuf::from("Player.c4p"), player.pack_raw().unwrap())
        .expect("open source player");
    let source_entry = source.entries().unwrap().remove(0);
    let mut rewritten = MutableGroup::from_group(&source).unwrap();

    assert!(rewritten.rename_entry("vEtErAn.C4I", "Captain.c4i"));

    let reopened = Group::from_memory(PathBuf::from("Player.c4p"), rewritten.pack_raw().unwrap())
        .expect("open renamed player");
    assert!(!reopened.exists("Veteran.c4i"));
    assert!(reopened.exists("CAPTAIN.C4I"));
    assert_eq!(
        reopened.read_entry_bytes("captain.c4i").unwrap(),
        crew_image
    );
    let renamed = reopened.entries().unwrap().remove(0);
    assert_eq!(renamed.time, source_entry.time);
    assert_eq!(renamed.executable, source_entry.executable);
    assert_eq!(renamed.crc_state, source_entry.crc_state);
    assert_eq!(renamed.stored_crc, source_entry.stored_crc);
}

#[test]
fn mutable_group_rename_rejects_a_distinct_case_insensitive_target() {
    let mut group = MutableGroup::new("Player.c4p");
    group.add_file("Alpha.c4i", b"alpha".to_vec()).unwrap();
    group.add_file("Bravo.c4i", b"bravo".to_vec()).unwrap();

    assert!(!group.rename_entry("missing.c4i", "Renamed.c4i"));
    assert!(!group.rename_entry("Alpha.c4i", ""));
    assert!(!group.rename_entry("Alpha.c4i", "Bad\0Name.c4i"));
    assert!(!group.rename_entry("ALPHA.C4I", "brAVo.C4i"));
    assert_eq!(group.entry_names(), ["Alpha.c4i", "Bravo.c4i"]);
    assert!(group.rename_entry("alpha.c4i", "ALPHA.C4I"));
    assert_eq!(group.entry_names(), ["ALPHA.C4I", "Bravo.c4i"]);
}

#[test]
fn mutable_group_rename_updates_an_expanded_childs_sort_filename() {
    let mut child = MutableGroup::new("Unsorted.bin");
    child.add_file("z.raw", b"last".to_vec()).unwrap();
    child.add_file("ObjectInfo.txt", b"first".to_vec()).unwrap();
    let mut player = MutableGroup::new("Player.c4p");
    player.add_child("Unsorted.bin", child).unwrap();

    assert!(player.rename_entry("unsorted.BIN", "Veteran.c4i"));

    let packed = Group::from_memory(PathBuf::from("Player.c4p"), player.pack_raw().unwrap())
        .expect("open player");
    let crew = packed.open_child("VETERAN.C4I").expect("open renamed crew");
    assert_eq!(
        crew.entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.name_bytes)
            .collect::<Vec<_>>(),
        [b"ObjectInfo.txt".to_vec(), b"z.raw".to_vec()]
    );
}

#[test]
fn cpp_add_entry_truncates_to_the_platform_filename_limit() {
    // AddEntry copies through SCopy(..., _MAX_FNAME): NAME_MAX (255) on Unix
    // and the CRT's 256-byte _MAX_FNAME on Windows. The 260-byte core remains
    // zero-padded (src/C4Group.cpp:864-866; src/C4Group.h:105-116).
    let mut group = MutableGroup::new("Test.c4g");
    let source = vec![b'a'; 300];
    let expected_length = if cfg!(windows) { 256 } else { 255 };
    let expected = "a".repeat(expected_length);

    group
        .add_file_bytes_with_metadata(source, b"data".to_vec(), 1, false)
        .unwrap();

    assert_eq!(group.entry_names(), [expected.as_str()]);
    let image = group.pack_raw().unwrap();
    assert_eq!(&image[204..204 + expected_length], expected.as_bytes());
    assert!(image[204 + expected_length..204 + 260]
        .iter()
        .all(|byte| *byte == 0));
    assert_eq!(packed_entry_names(&image), [expected]);
}

#[test]
fn cpp_overlong_duplicate_lookup_precedes_truncation_and_removes_only_the_first_match() {
    // AddEntry calls GetEntry with the full input before SCopy truncates the
    // stored core. Repeated long inputs therefore coexist. A later exact
    // truncated input removes only the first match, then appends at the tail
    // (src/C4Group.cpp:851-888,896-904).
    let mut group = MutableGroup::new("Test.c4g");
    let source = vec![b'b'; 300];
    let expected_length = if cfg!(windows) { 256 } else { 255 };
    let stored = vec![b'b'; expected_length];

    group
        .add_file_bytes_with_metadata(source.clone(), vec![1], 1, false)
        .unwrap();
    group
        .add_file_bytes_with_metadata(source, vec![2], 1, false)
        .unwrap();
    assert_eq!(group.entry_names().len(), 2);

    group
        .add_file_bytes_with_metadata(stored, vec![3], 1, false)
        .unwrap();
    assert_eq!(group.entry_names().len(), 2);
    let image = group.pack_raw().unwrap();
    assert_eq!(&image[204 + 2 * 316..], &[2, 3]);
}

#[test]
fn cpp_add_entry_accepts_empty_names_and_zero_pads_the_core() {
    // Packed-group AddEntry accepts an explicit empty name. The zero-initialized
    // core therefore retains 260 zero filename bytes. A leading NUL is the
    // same empty C string and replaces it (src/C4Group.cpp:849-866;
    // src/C4Group.h:105-116).
    let mut group = MutableGroup::new("Test.c4g");

    group
        .add_file_with_metadata("", b"old".to_vec(), 1, false)
        .unwrap();
    group
        .add_file_bytes_with_metadata(b"\0ignored".to_vec(), b"empty".to_vec(), 1, false)
        .unwrap();

    assert_eq!(group.entry_names(), [""]);
    let image = group.pack_raw().unwrap();
    assert!(image[204..204 + 260].iter().all(|byte| *byte == 0));
    assert_eq!(packed_entry_names(&image), [""]);
    assert_eq!(&image[204 + 316..], b"empty");
}

#[test]
fn cpp_add_entry_stops_at_nul_and_replaces_the_prefix() {
    // SCopy and every const-char lookup stop at the first NUL. Rust byte-name
    // entrypoints therefore store and replace by that prefix instead of
    // rejecting the input (src/C4Strings.cpp:65-80).
    let mut group = MutableGroup::new("Test.c4g");
    group
        .add_file_with_metadata("prefix", vec![1], 1, false)
        .unwrap();
    group
        .add_file_bytes_with_metadata(b"prefix\0ignored".to_vec(), vec![2], 1, false)
        .unwrap();
    let mut expected = MutableGroup::new("Test.c4g");
    expected
        .add_file_with_metadata("prefix", vec![2], 1, false)
        .unwrap();

    assert_eq!(group.entry_names(), ["prefix"]);
    assert_eq!(group.entry_crc("prefix"), expected.entry_crc("prefix"));
    let image = group.pack_raw().unwrap();
    assert_eq!(packed_entry_names(&image), ["prefix"]);
    assert_eq!(&image[204 + 316..], &[2]);
}

#[test]
fn cpp_nul_terminated_child_name_uses_its_prefix_for_sort_and_storage() {
    let mut child = MutableGroup::new("Test.c4g");
    child.add_file("000.unmatched", vec![1]).unwrap();
    child.add_file("Scenario.txt", vec![1]).unwrap();
    let mut parent = MutableGroup::new("Test.c4g");
    parent
        .add_child_bytes(b"Child.c4s\0ignored".to_vec(), child)
        .unwrap();

    assert_eq!(
        packed_entry_names(&parent.pack_raw().unwrap()),
        ["Child.c4s"]
    );
    assert_eq!(
        packed_entry_names(&parent.pack_raw().unwrap()[204 + 316..]),
        ["Scenario.txt", "000.unmatched"]
    );
}

#[test]
fn minimal_raw_group_pack_round_trips_through_the_stock_layout_reader() {
    // Save writes a scrambled 204-byte C4GroupHeader, then packed 316-byte
    // C4GroupEntryCore records, then entry payloads in list order
    // (src/C4Group.h:84-118; src/C4Group.cpp:965-1025).
    let mut mutable = MutableGroup::new("Test.c4g");
    mutable.add_file("hello.txt", b"world".to_vec()).unwrap();

    let image = mutable.pack_raw().unwrap();
    assert_eq!(image.len(), 204 + 316 + 5);

    let group = Group::from_memory(PathBuf::from("minimal.c4g"), image).unwrap();
    assert_eq!(group.maker(), Some("New C4Group"));
    assert_eq!(group.read_file("hello.txt").unwrap(), b"world");
}

#[test]
fn nested_raw_group_pack_sets_child_core_crc_and_round_trips() {
    // Save copies each C4GroupEntryCore before concatenating entry payloads;
    // child payloads are their uncompressed group image and ChildGroup is set
    // (src/C4Group.cpp:972-1025,1085-1165; core layout in
    // src/C4Group.h:105-116).
    let mut child = MutableGroup::new("Test.c4g");
    child.add_file("inside.txt", b"nested".to_vec()).unwrap();
    let expected_child_crc = child.contents_crc();

    let mut parent = MutableGroup::new("Test.c4g");
    parent.add_child("Child.c4g", child).unwrap();
    parent.add_file("root.txt", b"root".to_vec()).unwrap();
    let image = parent.pack_raw().unwrap();

    let first_core = 204;
    assert_eq!(
        i32::from_le_bytes(
            image[first_core + 264..first_core + 268]
                .try_into()
                .unwrap()
        ),
        1
    );
    assert_eq!(image[first_core + 284], 2);
    assert_eq!(
        u32::from_le_bytes(
            image[first_core + 285..first_core + 289]
                .try_into()
                .unwrap()
        ),
        expected_child_crc
    );

    let group = Group::from_memory(PathBuf::from("parent.c4g"), image).unwrap();
    let child = group.open_child("Child.c4g").unwrap();
    assert_eq!(child.read_file("inside.txt").unwrap(), b"nested");
    assert_eq!(group.read_file("root.txt").unwrap(), b"root");
}

#[test]
fn cpp_child_entries_retain_timestamp_and_executable_metadata() {
    // Add(..., fChild, ..., iTime, fExecutable) forwards both fields into the
    // child entry core (src/C4Group.cpp:864-873,2095-2108).
    let mut child = MutableGroup::new("Test.c4g");
    child.add_file("inside.txt", b"nested".to_vec()).unwrap();
    let mut parent = MutableGroup::new("Test.c4g");
    parent
        .add_child_with_metadata("Child.c4g", child, 0x1234_5678, true)
        .unwrap();

    let image = parent.pack_raw().unwrap();
    assert_eq!(
        u32::from_le_bytes(image[204 + 280..204 + 284].try_into().unwrap()),
        0x1234_5678
    );
    assert_eq!(image[204 + 289], 1);
}

#[test]
fn cpp_zero_child_timestamp_defaults_to_current_time() {
    // The same zero-time substitution applies when fChild is true
    // (src/C4Group.cpp:2095-2108).
    let before = unix_time_now();
    let mut parent = MutableGroup::new("Test.c4g");
    parent
        .add_child("Child.c4g", MutableGroup::new("Test.c4g"))
        .unwrap();
    let image = parent.pack_raw().unwrap();
    let after = unix_time_now();

    let entry_time = u32::from_le_bytes(image[204 + 280..204 + 284].try_into().unwrap());
    assert!((before..=after).contains(&entry_time));
}

#[test]
fn cpp_explicit_zero_child_timestamp_also_defaults_to_current_time() {
    // Zero remains the current-time sentinel for the metadata-bearing child
    // Add overload (src/C4Group.cpp:2095-2108).
    let before = unix_time_now();
    let mut parent = MutableGroup::new("Test.c4g");
    parent
        .add_child_with_metadata("Child.c4g", MutableGroup::new("Test.c4g"), 0, false)
        .unwrap();
    let image = parent.pack_raw().unwrap();
    let after = unix_time_now();

    let entry_time = u32::from_le_bytes(image[204 + 280..204 + 284].try_into().unwrap());
    assert!((before..=after).contains(&entry_time));
}

#[test]
fn packed_group_uses_c4_gzip_magic_and_round_trips() {
    // CStdFile creates groups through StdGzCompressedFile::Write, whose gzip
    // stream replaces 1f 8b with LegacyClonk's 1e 8c magic
    // (src/C4Group.cpp:997-1008; src/StdGzCompressedFile.cpp:227-327).
    let mut mutable = MutableGroup::new("Test.c4g");
    mutable.add_file("hello.txt", b"world".to_vec()).unwrap();

    let packed = mutable.pack().unwrap();
    assert_eq!(&packed[..2], &[0x1e, 0x8c]);

    let group = Group::from_memory(PathBuf::from("minimal.c4g"), packed).unwrap();
    assert_eq!(group.read_file("hello.txt").unwrap(), b"world");
}

#[cfg(target_os = "macos")]
#[test]
fn cpp_darwin_zlib_stream_and_physical_file_crc_are_byte_exact() {
    // StdGzCompressedFile uses deflateInit2(9, Z_DEFLATED, 31, 2,
    // Z_DEFAULT_STRATEGY); Darwin zlib emits OS code 19. This fixture and its
    // physical CRC come from that unmodified C++ parameter sequence
    // (src/StdGzCompressedFile.cpp:227-247,278-327;
    // src/C4Group.cpp:429-463).
    let mut image = Vec::new();
    for _ in 0..4 {
        image.extend_from_slice(b"LegacyClonk zlib parity");
        image.extend_from_slice(&[0, 0xff]);
    }
    let compressed = compress_c4group_for_test(&image).unwrap();
    let expected = [
        0x1e, 0x8c, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x13, 0xf3, 0x49, 0x4d, 0x4f, 0x4c,
        0xae, 0x74, 0xce, 0xc9, 0xcf, 0xcb, 0x56, 0xa8, 0xca, 0xc9, 0x4c, 0x52, 0x28, 0x48, 0x2c,
        0xca, 0x2c, 0xa9, 0x64, 0xf8, 0xef, 0x43, 0x3d, 0x09, 0x00, 0x43, 0x58, 0x4f, 0x97, 0x64,
        0x00, 0x00, 0x00,
    ];

    assert_eq!(compressed, expected);
    assert_eq!(compressed[9], 19);
    assert_eq!(c4group_file_crc(&compressed), 0xf266_608c);
}

#[test]
fn cpp_oracle_compressed_bytes_and_physical_crc_match_when_available() {
    // Recompress the unmodified oracle's own raw group image, removing all
    // timestamp/maker variables from the comparison while retaining its exact
    // StdGzCompressedFile byte stream (src/StdGzCompressedFile.cpp:227-327).
    let Ok(oracle) = std::env::var("LC_C4GROUP_ORACLE") else {
        return;
    };
    let _oracle_guard = oracle_lock().lock().unwrap();
    let directory = tempdir().unwrap();
    let group_path = directory.path().join("oracle.c4g");
    std::fs::create_dir(&group_path).unwrap();
    std::fs::write(group_path.join("file.txt"), b"compression parity").unwrap();
    let output = Command::new(oracle)
        .arg(&group_path)
        .arg("-p")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "c4group failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let oracle_packed = std::fs::read(&group_path).unwrap();
    let mut standard_gzip = oracle_packed.clone();
    standard_gzip[..2].copy_from_slice(&[0x1f, 0x8b]);
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&standard_gzip[..])
        .read_to_end(&mut raw)
        .unwrap();
    let rust_packed = compress_c4group_for_test(&raw).unwrap();

    assert_eq!(rust_packed, oracle_packed);
    assert_eq!(
        c4group_file_crc(&rust_packed),
        c4group_file_crc(&oracle_packed)
    );
}

#[test]
fn cpp_header_and_entry_metadata_use_packed_native_layout() {
    // SetMaker copies at most 30 bytes; MakeOriginal writes sentinel 1234567.
    // AddEntry stores uint32 time and executable directly in the packed core
    // (src/C4Group.cpp:864-884,2272-2276,2432-2441; layout at
    // src/C4Group.h:84-118).
    let mut mutable = MutableGroup::new("Test.c4g");
    mutable.set_maker("Oracle Maker");
    mutable.make_original(true);
    mutable
        .add_file_with_metadata("run.bin", vec![1, 2, 3], 0xa0b0_c0d0, true)
        .unwrap();
    let image = mutable.pack_raw().unwrap();

    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    mem_unscramble(&mut header);
    assert_eq!(&header[40..53], b"Oracle Maker\0");
    assert_eq!(
        i32::from_le_bytes(header[108..112].try_into().unwrap()),
        1_234_567
    );

    assert_eq!(
        u32::from_le_bytes(image[204 + 280..204 + 284].try_into().unwrap()),
        0xa0b0_c0d0
    );
    assert_eq!(image[204 + 289], 1);
}

#[test]
fn cpp_rewrite_maker_copy_preserves_bytes_after_the_new_nul() {
    // C4Group::Close applies the process maker with SCopy. SCopy writes only
    // through the new NUL and does not clear the rest of Head.Maker, so those
    // tail bytes remain part of the physical standalone CRC
    // (src/C4Group.cpp:929-951; src/C4Strings.cpp:65-81).
    let mut mutable = MutableGroup::new("Player.c4p");
    mutable.set_maker("Original Maker");
    mutable.set_maker("Host Player");
    let image = mutable.pack_raw().unwrap();
    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    mem_unscramble(&mut header);

    assert_eq!(&header[40..55], b"Host Player\0er\0");
}

#[test]
fn cpp_rewrite_retains_password_and_reserved_header_bytes() {
    // OpenRealGrpFile retains the complete Head, and Close changes only the
    // version, entry count, creation, original flag, and optional maker before
    // Save. Password and reserved bytes therefore survive a deletion rewrite
    // (src/C4Group.cpp:762-798,897-951,955-1004).
    let original = MutableGroup::new("Player.c4p").pack_raw().unwrap();
    let mut template: [u8; 204] = original[..204].try_into().unwrap();
    mem_unscramble(&mut template);
    template[72..104].fill(0xa5);
    template[108..112].copy_from_slice(&1_234_567_i32.to_le_bytes());
    template[112..204].fill(0x5a);

    let mut rewritten = MutableGroup::new("Player.c4p");
    rewritten.set_rewrite_header_template(&template);
    rewritten.set_maker("Host Player");
    let image = rewritten.pack_raw().unwrap();
    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    mem_unscramble(&mut header);

    assert_eq!(&header[72..104], &[0xa5; 32]);
    assert_eq!(&header[112..204], &[0x5a; 92]);
    assert_eq!(i32::from_le_bytes(header[108..112].try_into().unwrap()), 0);
}

#[test]
fn cpp_group_reader_preserves_legacy_entry_name_bytes_for_rewrite() {
    // OpenRealGrpFile and AddEntry operate on the 260-byte char filename field;
    // no text transcoding occurs before Save writes it again
    // (src/C4Group.cpp:771-784,854-870,955-1015).
    let legacy_name = vec![0xe4, b'.', b't', b'x', b't'];
    let mut mutable = MutableGroup::new("Player.c4p");
    mutable
        .add_existing_file_bytes_with_metadata(
            legacy_name.clone(),
            b"legacy".to_vec(),
            0x1234_5678,
            7,
            false,
        )
        .unwrap();

    let group =
        Group::from_memory(PathBuf::from("Player.c4p"), mutable.pack_raw().unwrap()).unwrap();
    assert_eq!(group.entries().unwrap()[0].name_bytes, legacy_name);
}

#[test]
fn cpp_group_reader_exposes_the_raw_uncompressed_image() {
    // Child entries are copied from CStdFile's decompressed stream, so their
    // payload is the complete raw group image rather than the outer gzip bytes
    // (src/C4Group.cpp:1075-1143,1446-1495).
    let mut mutable = MutableGroup::new("Crew.c4i");
    mutable
        .add_file("ObjectInfo.txt", b"crew".to_vec())
        .unwrap();
    // Wrapped from the one image rather than packed a second time: each pack
    // restamps Head.Creation with the current time, so two of them differ
    // whenever they straddle a second (src/C4Group.cpp:937-939).
    let raw = mutable.pack_raw().unwrap();
    let packed = compress_c4group_for_test(&raw).unwrap();
    let group = Group::from_memory(PathBuf::from("Crew.c4i"), packed).unwrap();

    assert_eq!(group.raw_image().unwrap(), raw);
}

#[test]
fn mutable_group_from_group_preserves_existing_material_child_contents_and_metadata() {
    let mut nested = MutableGroup::new("Nested.c4g");
    nested.add_file("inside.bin", b"nested".to_vec()).unwrap();

    let mut material = MutableGroup::new("Material.c4g");
    material.set_maker("Material Maker");
    material
        .add_file_with_metadata("Sentinel.bin", b"keep me".to_vec(), 0x1234_5678, true)
        .unwrap();
    material
        .add_child_with_metadata("Nested.c4g", nested, 0x8765_4321, false)
        .unwrap();
    material
        .add_file("TexMap.txt", b"1=Old-Texture\r\n".to_vec())
        .unwrap();

    let mut source_image = material.pack_raw().unwrap();
    let mut source_header: [u8; 204] = source_image[..204].try_into().unwrap();
    mem_unscramble(&mut source_header);
    source_header[72..104].fill(0xa5);
    source_header[112..204].fill(0x5a);
    mem_unscramble(&mut source_header);
    source_image[..204].copy_from_slice(&source_header);

    let source = Group::from_memory(PathBuf::from("Material.c4g"), source_image).unwrap();
    let source_entries = source.entries().unwrap();
    let source_sentinel = source_entries
        .iter()
        .find(|entry| entry.name_bytes.eq_ignore_ascii_case(b"Sentinel.bin"))
        .unwrap();
    let source_sentinel_crc = source_sentinel.stored_crc;

    let mut rewritten = MutableGroup::from_group(&source).unwrap();
    rewritten
        .add_file("TexMap.txt", b"1=New-Texture\r\n".to_vec())
        .unwrap();
    let rewritten_image = rewritten.pack_raw().unwrap();
    let reopened =
        Group::from_memory(PathBuf::from("Material.c4g"), rewritten_image.clone()).unwrap();

    assert_eq!(reopened.read_file("Sentinel.bin").unwrap(), b"keep me");
    assert_eq!(
        reopened.read_file("TexMap.txt").unwrap(),
        b"1=New-Texture\r\n"
    );
    assert_eq!(
        reopened
            .open_child("Nested.c4g")
            .unwrap()
            .read_file("inside.bin")
            .unwrap(),
        b"nested"
    );

    let entries = reopened.entries().unwrap();
    let sentinel = entries
        .iter()
        .find(|entry| entry.name_bytes.eq_ignore_ascii_case(b"Sentinel.bin"))
        .unwrap();
    assert_eq!(sentinel.time, 0x1234_5678);
    assert!(sentinel.executable);
    assert_eq!(sentinel.stored_crc, source_sentinel_crc);
    let nested = entries
        .iter()
        .find(|entry| entry.name_bytes.eq_ignore_ascii_case(b"Nested.c4g"))
        .unwrap();
    assert!(nested.is_directory);
    assert_eq!(nested.time, 0x8765_4321);

    let mut rewritten_header: [u8; 204] = rewritten_image[..204].try_into().unwrap();
    mem_unscramble(&mut rewritten_header);
    assert_eq!(&rewritten_header[40..55], b"Material Maker\0");
    assert_eq!(&rewritten_header[72..104], &[0xa5; 32]);
    assert_eq!(&rewritten_header[112..204], &[0x5a; 92]);
}

#[test]
fn mutable_group_child_mut_opens_imported_child_case_insensitively() {
    let mut material = MutableGroup::new("Material.c4g");
    material
        .add_file("Sentinel.bin", b"preserved".to_vec())
        .unwrap();
    let mut scenario = MutableGroup::new("Scenario.c4s");
    scenario.add_child("Material.c4g", material).unwrap();
    scenario
        .add_file("Ordinary.c4g", b"not a child".to_vec())
        .unwrap();
    let source =
        Group::from_memory(PathBuf::from("Scenario.c4s"), scenario.pack_raw().unwrap()).unwrap();

    let mut rewritten = MutableGroup::from_group(&source).unwrap();
    assert!(matches!(
        rewritten.child_mut("missing.c4g").unwrap(),
        MutableGroupChildMut::Missing
    ));
    assert!(matches!(
        rewritten.child_mut("ordinary.C4G").unwrap(),
        MutableGroupChildMut::File
    ));
    let MutableGroupChildMut::Child(material) = rewritten.child_mut("mAtErIaL.C4g").unwrap() else {
        panic!("Material.c4g must remain a child")
    };
    material
        .add_file("TexMap.txt", b"1=Earth-Rough\r\n".to_vec())
        .unwrap();

    let reopened =
        Group::from_memory(PathBuf::from("Scenario.c4s"), rewritten.pack_raw().unwrap()).unwrap();
    let material = reopened.open_child("Material.c4g").unwrap();
    assert_eq!(material.read_file("Sentinel.bin").unwrap(), b"preserved");
    assert_eq!(
        material.read_file("TexMap.txt").unwrap(),
        b"1=Earth-Rough\r\n"
    );
}

#[test]
fn mutable_group_from_directory_keeps_raw_unwrapped_group_image_as_file() {
    let directory = tempdir().unwrap();
    let scenario_path = directory.path().join("FolderScenario.c4s");
    std::fs::create_dir(&scenario_path).unwrap();

    let mut raw_child = MutableGroup::new("Raw.c4g");
    raw_child
        .add_file("Inside.txt", b"raw child sentinel".to_vec())
        .unwrap();
    let raw_image = raw_child.pack_raw().unwrap();
    std::fs::write(scenario_path.join("Raw.c4g"), &raw_image).unwrap();

    let mut wrapped_child = MutableGroup::new("Wrapped.c4g");
    wrapped_child
        .add_file("Inside.txt", b"wrapped child sentinel".to_vec())
        .unwrap();
    std::fs::write(
        scenario_path.join("Wrapped.c4g"),
        wrapped_child.pack().unwrap(),
    )
    .unwrap();

    let source = Group::open(&scenario_path).unwrap();
    assert_eq!(
        source
            .open_child("Raw.c4g")
            .expect("direct child opens retain raw-image support")
            .read_file("Inside.txt")
            .unwrap(),
        b"raw child sentinel"
    );

    let rewritten = MutableGroup::from_group(&source).unwrap();
    let mut expected_file = MutableGroup::new("FolderScenario.c4s");
    expected_file
        .add_file("Raw.c4g", raw_image.clone())
        .unwrap();
    assert_eq!(
        rewritten.entry_crc("Raw.c4g"),
        expected_file.entry_crc("Raw.c4g"),
        "ordinary-file CRC includes the outer entry name"
    );

    let reopened = Group::from_memory(
        PathBuf::from("FolderScenario.c4s"),
        rewritten.pack_raw().unwrap(),
    )
    .unwrap();
    let raw_entry = reopened
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name_bytes == b"Raw.c4g")
        .unwrap();
    assert!(!raw_entry.is_directory);
    assert_eq!(
        reopened.read_entry_bytes_exact(&raw_entry).unwrap(),
        raw_image
    );
    reopened
        .open_child("Raw.c4g")
        .expect_err("the packed result must retain the ordinary-file flag");

    let wrapped_entry = reopened
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name_bytes == b"Wrapped.c4g")
        .unwrap();
    assert!(wrapped_entry.is_directory);
    assert_eq!(
        reopened
            .open_child_entry_exact(&wrapped_entry)
            .unwrap()
            .read_file("Inside.txt")
            .unwrap(),
        b"wrapped child sentinel"
    );
}

#[cfg(unix)]
#[test]
// `C4Group_PackDirectoryTo` packs a child to a temporary file and moves that
// file into its parent, so the outer core receives the temporary file metadata
// (`C4Group.cpp:281-307,1459-1495`).
fn mutable_group_from_directory_uses_temporary_child_metadata() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("Objects.c4d");
    let child = root.join("Child.c4d");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(child.join("Inner.txt"), b"inner").unwrap();
    set_mtime(&child, 1);

    let source = Group::open(&root).unwrap();
    let before = unix_time_now();
    let rewritten = MutableGroup::from_directory(&source).unwrap();
    let image = rewritten.pack_raw().unwrap();
    let after = unix_time_now();
    let reopened = Group::from_memory(PathBuf::from("Objects.c4d"), image).unwrap();
    let child = reopened
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name_bytes == b"Child.c4d")
        .unwrap();

    assert!((before..=after).contains(&child.time));
    assert!(!child.executable);
}

#[test]
fn mutable_group_keeps_directory_packed_child_image_opaque_until_mutated() {
    let directory = tempdir().unwrap();
    let scenario_path = directory.path().join("FolderScenario.c4s");
    std::fs::create_dir(&scenario_path).unwrap();

    let mut material = MutableGroup::new("Material.c4g");
    material
        .add_file("Earth.c4m", b"child sentinel".to_vec())
        .unwrap();
    let mut material_raw = material.pack_raw().unwrap();
    let mut material_header: [u8; 204] = material_raw[..204].try_into().unwrap();
    mem_unscramble(&mut material_header);
    material_header[104..108].copy_from_slice(&0x1234_5678_u32.to_le_bytes());
    material_header[112..204].fill(0xa7);
    mem_unscramble(&mut material_header);
    material_raw[..204].copy_from_slice(&material_header);
    std::fs::write(
        scenario_path.join("Material.c4g"),
        compress_c4group_for_test(&material_raw).unwrap(),
    )
    .unwrap();

    let source = Group::open(&scenario_path).unwrap();
    let rewritten = MutableGroup::from_group(&source).unwrap();
    let reopened = Group::from_memory(
        PathBuf::from("FolderScenario.c4s"),
        rewritten.pack_raw().unwrap(),
    )
    .unwrap();

    assert_eq!(
        reopened
            .open_child("Material.c4g")
            .unwrap()
            .raw_image()
            .unwrap(),
        material_raw,
        "an untouched packed file child must retain its exact raw group image"
    );
}

#[test]
fn mutable_group_rewritten_packed_child_refreshes_outer_metadata() {
    let mut material = MutableGroup::new("Material.c4g");
    material
        .add_file("Earth.c4m", b"child sentinel".to_vec())
        .unwrap();
    let mut scenario = MutableGroup::new("Scenario.c4s");
    scenario
        .add_child_with_metadata("Material.c4g", material, 0x1234_5678, true)
        .unwrap();
    let source =
        Group::from_memory(PathBuf::from("Scenario.c4s"), scenario.pack_raw().unwrap()).unwrap();

    let mut rewritten = MutableGroup::from_group(&source).unwrap();
    let before = unix_time_now();
    let MutableGroupChildMut::Child(material) = rewritten.child_mut("Material.c4g").unwrap() else {
        panic!("packed Material.c4g must open as a mutable child")
    };
    material
        .add_file("TexMap.txt", b"1=Earth-Rough\r\n".to_vec())
        .unwrap();
    let rewritten_image = rewritten.pack_raw().unwrap();
    let after = unix_time_now();
    let reopened = Group::from_memory(PathBuf::from("Scenario.c4s"), rewritten_image).unwrap();
    let material_entry = reopened
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name_bytes.eq_ignore_ascii_case(b"Material.c4g"))
        .unwrap();

    assert!((before..=after).contains(&material_entry.time));
    assert!(!material_entry.executable);
}

#[test]
fn mutable_group_from_directory_recognizes_packed_material_child_file() {
    let directory = tempdir().unwrap();
    let scenario_path = directory.path().join("FolderScenario.c4s");
    std::fs::create_dir(&scenario_path).unwrap();
    std::fs::write(scenario_path.join("Root.txt"), b"root sentinel").unwrap();

    let mut material = MutableGroup::new("Material.c4g");
    material
        .add_file("Earth.c4m", b"child sentinel".to_vec())
        .unwrap();
    material
        .add_file("TexMap.txt", b"old map".to_vec())
        .unwrap();
    std::fs::write(scenario_path.join("Material.c4g"), material.pack().unwrap()).unwrap();

    let source = Group::open(&scenario_path).unwrap();
    let mut rewritten = MutableGroup::from_group(&source).unwrap();
    let MutableGroupChildMut::Child(material) = rewritten.child_mut("material.C4G").unwrap() else {
        panic!("packed Material.c4g file must import as a child")
    };
    material
        .add_file("TexMap.txt", b"new map".to_vec())
        .unwrap();

    let reopened = Group::from_memory(
        PathBuf::from("FolderScenario.c4s"),
        rewritten.pack_raw().unwrap(),
    )
    .unwrap();
    assert_eq!(reopened.read_file("Root.txt").unwrap(), b"root sentinel");
    let material = reopened.open_child("Material.c4g").unwrap();
    assert_eq!(material.read_file("Earth.c4m").unwrap(), b"child sentinel");
    assert_eq!(material.read_file("TexMap.txt").unwrap(), b"new map");
}

#[test]
fn cpp_duplicate_entry_requires_close_rewrite() {
    // AddEntry marks an earlier duplicate deleted, which is itself sufficient
    // for Close's rewrite check (src/C4Group.cpp:839-881,897-920).
    let mut mutable = MutableGroup::new("Player.c4p");
    mutable.add_file("A.txt", b"a".to_vec()).unwrap();
    mutable.add_file("B.txt", b"b".to_vec()).unwrap();
    let mut raw = mutable.pack_raw().unwrap();
    let second_name = 204 + 316;
    raw[second_name..second_name + 260].fill(0);
    raw[second_name..second_name + 5].copy_from_slice(b"A.txt");

    let group = Group::from_memory(PathBuf::from("Player.c4p"), raw).unwrap();
    assert!(group.requires_rewrite());
    assert_eq!(group.entries().unwrap().len(), 1);
}

#[test]
fn cpp_zero_file_timestamp_defaults_to_current_time() {
    // C4Group::Add substitutes time(nullptr) when iTime is zero before storing
    // the entry core (src/C4Group.cpp:2095-2108).
    let before = unix_time_now();
    let mut mutable = MutableGroup::new("Test.c4g");
    mutable.add_file("now.txt", Vec::new()).unwrap();
    let image = mutable.pack_raw().unwrap();
    let after = unix_time_now();

    let entry_time = u32::from_le_bytes(image[204 + 280..204 + 284].try_into().unwrap());
    assert!((before..=after).contains(&entry_time));
}

#[test]
fn cpp_explicit_zero_file_timestamp_also_defaults_to_current_time() {
    // The iTime parameter itself uses zero as the current-time sentinel
    // (src/C4Group.cpp:2095-2108).
    let before = unix_time_now();
    let mut mutable = MutableGroup::new("Test.c4g");
    mutable
        .add_file_with_metadata("now.txt", Vec::new(), 0, true)
        .unwrap();
    let image = mutable.pack_raw().unwrap();
    let after = unix_time_now();

    let entry_time = u32::from_le_bytes(image[204 + 280..204 + 284].try_into().unwrap());
    assert!((before..=after).contains(&entry_time));
}

#[test]
fn cpp_new_group_header_is_stamped_when_packed() {
    // Every rewritten group gets Head.Creation = time(nullptr) immediately
    // before Save (src/C4Group.cpp:938-939).
    let before = unix_time_now();
    let image = MutableGroup::new("Test.c4g").pack_raw().unwrap();
    let after = unix_time_now();

    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    mem_unscramble(&mut header);
    let creation = u32::from_le_bytes(header[104..108].try_into().unwrap());
    assert!((before..=after).contains(&creation));
}

#[test]
fn cpp_oracle_opens_the_rust_packed_group_when_available() {
    // C4Group::OpenRealGrpFile reads the same compressed header/core/payload
    // sequence (src/C4Group.cpp:762-798). This optional differential check
    // executes an existing read-only c4group oracle binary against a temp file.
    let Ok(oracle) = std::env::var("LC_C4GROUP_ORACLE") else {
        return;
    };
    let _oracle_guard = oracle_lock().lock().unwrap();
    let mut mutable = MutableGroup::new("Test.c4g");
    mutable.add_file("hello.txt", b"world".to_vec()).unwrap();
    let directory = tempdir().unwrap();
    let path = directory.path().join("rust.c4g");
    std::fs::write(&path, mutable.pack().unwrap()).unwrap();

    let output = Command::new(oracle)
        .arg(&path)
        .args(["-l", "*"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "c4group failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello.txt"));
}

#[test]
fn cpp_oracle_writes_the_same_entry_crc_when_available() {
    // This differential check lets unmodified C4Group::CalcCRC32 serialize the
    // oracle value, then compares it to the Rust calculation
    // (src/C4Group.cpp:2444-2516).
    let Ok(oracle) = std::env::var("LC_C4GROUP_ORACLE") else {
        return;
    };
    let _oracle_guard = oracle_lock().lock().unwrap();
    let directory = tempdir().unwrap();
    let group_path = directory.path().join("oracle.c4g");
    std::fs::create_dir(&group_path).unwrap();
    std::fs::write(group_path.join("file.txt"), b"hello").unwrap();
    let output = Command::new(oracle)
        .arg(&group_path)
        .arg("-p")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "c4group failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut compressed = std::fs::read(&group_path).unwrap();
    compressed[..2].copy_from_slice(&[0x1f, 0x8b]);
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&compressed[..])
        .read_to_end(&mut raw)
        .unwrap();
    let oracle_crc = u32::from_le_bytes(raw[204 + 285..204 + 289].try_into().unwrap());

    let mut rust = MutableGroup::new("Test.c4g");
    rust.add_file("file.txt", b"hello".to_vec()).unwrap();
    assert_eq!(rust.entry_crc("file.txt"), Some(oracle_crc));
}

fn mem_unscramble(buffer: &mut [u8]) {
    buffer.iter_mut().for_each(|byte| *byte ^= 237);
    for index in (0..buffer.len().saturating_sub(2)).step_by(3) {
        buffer.swap(index, index + 2);
    }
}

fn unix_time_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

fn packed_entry_names(image: &[u8]) -> Vec<String> {
    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    mem_unscramble(&mut header);
    let count = i32::from_le_bytes(header[36..40].try_into().unwrap()) as usize;
    (0..count)
        .map(|index| {
            let start = 204 + index * 316;
            let core = &image[start..start + 316];
            let length = core.iter().position(|byte| *byte == 0).unwrap();
            String::from_utf8(core[..length].to_vec()).unwrap()
        })
        .collect()
}

fn oracle_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

/// A new group carries C4Group's native default until something overwrites it,
/// and `pack` writes whatever `maker_bytes` reports, so the accessor is the
/// authority on what a reader will see in the packed header.
#[test]
fn maker_bytes_reports_what_pack_writes() {
    let mut group = MutableGroup::new("Report.c4g");
    group.add_file("Info.txt", b"payload".to_vec()).unwrap();
    assert_eq!(group.maker_bytes(), b"New C4Group");

    group.set_maker_bytes(b"Overwritten");
    assert_eq!(group.maker_bytes(), b"Overwritten");

    let packed = Group::from_memory(PathBuf::from("Report.c4g"), group.pack().unwrap()).unwrap();
    assert_eq!(packed.maker_bytes(), Some(group.maker_bytes()));
}
