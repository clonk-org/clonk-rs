#![allow(dead_code)] // The writer is path-included until its public integration slice.

#[path = "../src/group_writer.rs"]
mod group_writer;

use group_writer::{c4group_file_crc, compress_c4group_for_test};
use lc_resources::{Group, MutableGroup, MutableGroupChildMut};
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

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
    let raw = mutable.pack_raw().unwrap();
    let packed = mutable.pack().unwrap();
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
