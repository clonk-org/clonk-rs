use std::path::PathBuf;

use clonk_resources::{
    combine_network_scenario, combine_network_scenario_with_maker_bytes, Group, MutableGroup,
};

#[test]
fn retrieve_scenario_preserves_native_maker_bytes() {
    // C4Group::SetMaker copies the process-global native General.Name bytes
    // into each rewritten group header (src/C4Application.cpp:95-120;
    // src/C4Group.cpp:104-108,938-945).
    let scenario = MutableGroup::new("Scenario.c4s");
    let dynamic = MutableGroup::new("Dynamic.c4s");
    let scenario =
        Group::from_memory(PathBuf::from("Scenario.c4s"), scenario.pack().unwrap()).unwrap();
    let dynamic =
        Group::from_memory(PathBuf::from("Dynamic.c4s"), dynamic.pack().unwrap()).unwrap();

    let packed = combine_network_scenario_with_maker_bytes(
        &scenario,
        &dynamic,
        "Combined1.c4s",
        b"M\x81ker",
    )
    .unwrap();
    let combined = Group::from_memory(PathBuf::from("Combined1.c4s"), packed).unwrap();

    assert_eq!(combined.maker_bytes(), Some(&b"M\x81ker"[..]));
}

#[test]
fn cpp_retrieve_scenario_overlays_dynamic_top_level_and_keeps_existing_material_group() {
    // RetrieveScenario unpacks both Material.c4g files before C4Group::Merge.
    // The folder-target Merge moves ordinary dynamic entries over the
    // scenario, while its attempted directory replacement leaves the existing
    // scenario Material.c4g tree intact (src/C4Network2.cpp:619-671;
    // src/C4Group.cpp:1421-1503). This edge case is frozen against the real
    // C++ C4Group objects by the development oracle harness.
    let mut scenario_material = MutableGroup::new("Material.c4g");
    scenario_material
        .add_file("BaseMat.txt", b"material-base".to_vec())
        .unwrap();
    scenario_material
        .add_file("ReplaceMat.txt", b"material-old".to_vec())
        .unwrap();
    let mut scenario = MutableGroup::new("Scenario.c4s");
    scenario
        .add_file("Base.txt", b"scenario-base".to_vec())
        .unwrap();
    scenario
        .add_file("Replace.txt", b"scenario-old".to_vec())
        .unwrap();
    scenario
        .add_child("Material.c4g", scenario_material)
        .unwrap();

    let mut dynamic_material = MutableGroup::new("Material.c4g");
    dynamic_material
        .add_file("ReplaceMat.txt", b"material-new".to_vec())
        .unwrap();
    dynamic_material
        .add_file("NewMat.txt", b"material-dynamic".to_vec())
        .unwrap();
    let mut dynamic = MutableGroup::new("Dynamic.c4s");
    dynamic
        .add_file("Replace.txt", b"dynamic-new".to_vec())
        .unwrap();
    dynamic
        .add_file("New.txt", b"dynamic-only".to_vec())
        .unwrap();
    dynamic.add_child("Material.c4g", dynamic_material).unwrap();

    let scenario =
        Group::from_memory(PathBuf::from("Scenario.c4s"), scenario.pack().unwrap()).unwrap();
    let dynamic =
        Group::from_memory(PathBuf::from("Dynamic.c4s"), dynamic.pack().unwrap()).unwrap();

    let packed =
        combine_network_scenario(&scenario, &dynamic, "Combined1.c4s", "Network Client").unwrap();
    let combined = Group::from_memory(PathBuf::from("Combined1.c4s"), packed).unwrap();

    assert_eq!(combined.read_file("Base.txt").unwrap(), b"scenario-base");
    assert_eq!(combined.read_file("Replace.txt").unwrap(), b"dynamic-new");
    assert_eq!(combined.read_file("New.txt").unwrap(), b"dynamic-only");
    let material = combined.open_child("Material.c4g").unwrap();
    assert_eq!(material.read_file("BaseMat.txt").unwrap(), b"material-base");
    assert_eq!(
        material.read_file("ReplaceMat.txt").unwrap(),
        b"material-old"
    );
    assert!(!material.exists("NewMat.txt"));
    assert_eq!(combined.maker(), Some("Network Client"));
}

#[test]
fn retrieve_scenario_preserves_legacy_names_metadata_and_opaque_children() {
    const LEGACY_SCENARIO: &[u8] = b"Replace\xfc.bin";
    const LEGACY_DYNAMIC: &[u8] = b"rEPLACE\xfc.BIN";
    const LEGACY_DISTINCT: &[u8] = b"Replace\xf6.bin";
    const OPAQUE_NAME: &[u8] = b"Opaque\xfc.c4g";

    let mut opaque = MutableGroup::new_bytes(OPAQUE_NAME.to_vec());
    opaque.set_maker_bytes(b"Opaque Legacy Maker");
    opaque
        .add_file_bytes_with_metadata(
            b"Zulu\xfc.bin".to_vec(),
            b"opaque-zulu".to_vec(),
            0x1020_3040,
            true,
        )
        .unwrap();
    opaque
        .add_file_bytes_with_metadata(
            b"Alpha.bin".to_vec(),
            b"opaque-alpha".to_vec(),
            0x5060_7080,
            false,
        )
        .unwrap();
    let mut opaque_raw = opaque.pack_raw().unwrap();
    stamp_reserved_header_bytes(&mut opaque_raw, 0xa5, 0x5a);

    // Material.c4g is deliberately written without its normal sort list and
    // with a distinctive header. RetrieveScenario unpacks this one colliding
    // child, so the result must rebuild it rather than preserve these bytes.
    let mut scenario_material = MutableGroup::new("Unsorted.bin");
    scenario_material.set_maker("Scenario Material Maker");
    scenario_material
        .add_file_with_metadata("Earth.c4m", b"scenario-earth".to_vec(), 0x1122_3344, false)
        .unwrap();
    scenario_material
        .add_file_with_metadata("TexMap.txt", b"scenario-map".to_vec(), 0x5566_7788, false)
        .unwrap();
    let mut scenario_material_raw = scenario_material.pack_raw().unwrap();
    stamp_reserved_header_bytes(&mut scenario_material_raw, 0xc3, 0x3c);

    let mut dynamic_material = MutableGroup::new("Material.c4g");
    dynamic_material
        .add_file("Earth.c4m", b"dynamic-earth".to_vec())
        .unwrap();
    dynamic_material
        .add_file("Dynamic.c4m", b"dynamic-only".to_vec())
        .unwrap();
    let dynamic_material_raw = dynamic_material.pack_raw().unwrap();

    let mut scenario = MutableGroup::new("Scenario.c4s");
    scenario
        .add_file_bytes_with_metadata(
            LEGACY_SCENARIO.to_vec(),
            b"scenario-old".to_vec(),
            0x0102_0304,
            false,
        )
        .unwrap();
    scenario
        .add_file_bytes_with_metadata(
            LEGACY_DISTINCT.to_vec(),
            b"scenario-distinct".to_vec(),
            0x0506_0708,
            false,
        )
        .unwrap();
    add_raw_child(
        &mut scenario,
        OPAQUE_NAME,
        opaque_raw.clone(),
        0x90a0_b0c0,
        true,
    );
    add_raw_child(
        &mut scenario,
        b"Material.c4g",
        scenario_material_raw.clone(),
        0x2233_4455,
        false,
    );

    let mut dynamic = MutableGroup::new("Dynamic.c4s");
    dynamic
        .add_file_bytes_with_metadata(
            LEGACY_DYNAMIC.to_vec(),
            b"dynamic-new".to_vec(),
            0x0a0b_0c0d,
            false,
        )
        .unwrap();
    dynamic
        .add_file_bytes_with_metadata(
            b"Added\xfe.bin".to_vec(),
            b"dynamic-added".to_vec(),
            0x0e0f_1011,
            false,
        )
        .unwrap();
    add_raw_child(
        &mut dynamic,
        b"Material.c4g",
        dynamic_material_raw,
        0x6677_8899,
        false,
    );

    let scenario =
        Group::from_memory(PathBuf::from("Scenario.c4s"), scenario.pack().unwrap()).unwrap();
    let dynamic =
        Group::from_memory(PathBuf::from("Dynamic.c4s"), dynamic.pack().unwrap()).unwrap();
    let packed =
        combine_network_scenario(&scenario, &dynamic, "CombinedRaw.c4s", "Network Client").unwrap();
    let combined = Group::from_memory(PathBuf::from("CombinedRaw.c4s"), packed).unwrap();

    assert_eq!(
        read_exact_entry(&combined, LEGACY_DYNAMIC),
        b"dynamic-new",
        "the dynamic raw-byte key must replace the matching scenario key"
    );
    assert!(
        combined
            .entries()
            .unwrap()
            .into_iter()
            .all(|entry| entry.name_bytes != LEGACY_SCENARIO),
        "the replacement must retain the dynamic entry's original byte spelling"
    );
    assert_eq!(
        read_exact_entry(&combined, LEGACY_DISTINCT),
        b"scenario-distinct",
        "different non-UTF-8 bytes must not collapse through lossy Unicode"
    );
    assert_eq!(
        read_exact_entry(&combined, b"Added\xfe.bin"),
        b"dynamic-added"
    );

    let opaque_entry = exact_entry(&combined, OPAQUE_NAME);
    assert_eq!(opaque_entry.time, 0x90a0_b0c0);
    assert_eq!(
        opaque_entry.executable,
        cfg!(target_os = "linux"),
        "extract/repack retains the ordinary child's outer executable core only on Linux"
    );
    let combined_opaque_raw = combined.read_entry_bytes_exact(&opaque_entry).unwrap();
    assert_eq!(
        combined_opaque_raw, opaque_raw,
        "an ordinary child must retain its complete raw image, including header and cores"
    );
    let combined_opaque =
        Group::from_raw_memory(PathBuf::from("OpaqueLegacy.c4g"), combined_opaque_raw).unwrap();
    let opaque_entries = combined_opaque.entries().unwrap();
    assert_eq!(
        opaque_entries
            .iter()
            .map(|entry| entry.name_bytes.as_slice())
            .collect::<Vec<_>>(),
        vec![b"Zulu\xfc.bin".as_slice(), b"Alpha.bin".as_slice()]
    );
    assert_eq!(
        opaque_entries
            .iter()
            .map(|entry| (entry.time, entry.executable))
            .collect::<Vec<_>>(),
        vec![(0x1020_3040, true), (0x5060_7080, false)]
    );
    assert_eq!(
        read_exact_entry(&combined_opaque, b"Zulu\xfc.bin"),
        b"opaque-zulu"
    );
    assert_eq!(
        read_exact_entry(&combined_opaque, b"Alpha.bin"),
        b"opaque-alpha"
    );

    let material_entry = exact_entry(&combined, b"Material.c4g");
    let combined_material_raw = combined.read_entry_bytes_exact(&material_entry).unwrap();
    assert_ne!(
        combined_material_raw, scenario_material_raw,
        "the colliding Material.c4g is the exceptional child that C++ unpacks and rebuilds"
    );
    let combined_material =
        Group::from_raw_memory(PathBuf::from("Material.c4g"), combined_material_raw).unwrap();
    assert_eq!(combined_material.maker(), Some("Network Client"));
    assert_eq!(
        combined_material.read_file("Earth.c4m").unwrap(),
        b"scenario-earth",
        "the failed folder-target replacement retains scenario Material contents"
    );
    assert_eq!(
        combined_material.read_file("TexMap.txt").unwrap(),
        b"scenario-map"
    );
    assert!(!combined_material.exists("Dynamic.c4m"));
    assert_eq!(
        combined_material
            .entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.name_bytes)
            .collect::<Vec<_>>(),
        vec![b"TexMap.txt".to_vec(), b"Earth.c4m".to_vec()],
        "the rebuilt Material child receives its native standard ordering"
    );
}

#[test]
fn cpp_retrieve_scenario_opens_standalone_material_files_without_child_cores() {
    let mut scenario_material = MutableGroup::new("Material.c4g");
    scenario_material
        .add_file("Earth.c4m", b"scenario-earth".to_vec())
        .unwrap();
    let mut scenario = MutableGroup::new("Scenario.c4s");
    scenario
        .add_file("Material.c4g", scenario_material.pack().unwrap())
        .unwrap();

    let mut dynamic_material = MutableGroup::new("Material.c4g");
    dynamic_material
        .add_file("Earth.c4m", b"dynamic-earth".to_vec())
        .unwrap();
    let mut dynamic = MutableGroup::new("Dynamic.c4s");
    dynamic
        .add_file("Material.c4g", dynamic_material.pack().unwrap())
        .unwrap();

    let scenario =
        Group::from_memory(PathBuf::from("Scenario.c4s"), scenario.pack().unwrap()).unwrap();
    let dynamic =
        Group::from_memory(PathBuf::from("Dynamic.c4s"), dynamic.pack().unwrap()).unwrap();
    assert!(!exact_entry(&scenario, b"Material.c4g").is_directory);
    assert!(!exact_entry(&dynamic, b"Material.c4g").is_directory);

    let combined = combine_network_scenario(
        &scenario,
        &dynamic,
        "CombinedStandalone.c4s",
        "Network Client",
    )
    .unwrap();
    let combined = Group::from_memory(PathBuf::from("CombinedStandalone.c4s"), combined).unwrap();
    let material = combined.open_child("Material.c4g").unwrap();
    assert_eq!(material.maker(), Some("Network Client"));
    assert_eq!(material.read_file("Earth.c4m").unwrap(), b"scenario-earth");
}

#[test]
fn retrieve_scenario_classifies_extracted_group_files_like_top_level_cpp() {
    let directory = tempfile::tempdir().unwrap();
    let scenario_path = directory.path().join("FolderScenario.c4s");
    std::fs::create_dir(&scenario_path).unwrap();

    let mut raw_child = MutableGroup::new("Raw.c4g");
    raw_child
        .add_file("Inside.txt", b"raw child sentinel".to_vec())
        .unwrap();
    let raw_image = raw_child.pack_raw().unwrap();

    let mut wrapped_child = MutableGroup::new("Wrapped.c4g");
    wrapped_child
        .add_file("Inside.txt", b"wrapped child sentinel".to_vec())
        .unwrap();
    let wrapped_image = wrapped_child.pack().unwrap();

    std::fs::write(scenario_path.join("Raw.c4g"), &raw_image).unwrap();
    std::fs::write(scenario_path.join("Wrapped.c4g"), &wrapped_image).unwrap();
    let directory_scenario = Group::open(&scenario_path).unwrap();

    let mut packed_source = MutableGroup::new("PackedScenario.c4s");
    packed_source
        .add_file("Raw.c4g", raw_image.clone())
        .unwrap();
    packed_source
        .add_file("Wrapped.c4g", wrapped_image)
        .unwrap();
    let packed_scenario = Group::from_memory(
        PathBuf::from("PackedScenario.c4s"),
        packed_source.pack().unwrap(),
    )
    .unwrap();
    let dynamic = Group::from_memory(
        PathBuf::from("Dynamic.c4s"),
        MutableGroup::new("Dynamic.c4s").pack().unwrap(),
    )
    .unwrap();

    for (label, scenario) in [
        ("directory", directory_scenario),
        ("packed", packed_scenario),
    ] {
        let output_filename = format!("Combined-{label}.c4s");
        let combined =
            combine_network_scenario(&scenario, &dynamic, &output_filename, "Network Client")
                .unwrap();
        let combined = Group::from_memory(PathBuf::from(output_filename), combined).unwrap();

        let raw_entry = exact_entry(&combined, b"Raw.c4g");
        assert!(!raw_entry.is_directory, "{label} raw-image source");
        assert_eq!(
            combined.read_entry_bytes_exact(&raw_entry).unwrap(),
            raw_image,
            "{label} raw-image bytes"
        );
        combined
            .open_child_entry_exact(&raw_entry)
            .expect_err("raw image must retain its ordinary-file flag");

        let wrapped_entry = exact_entry(&combined, b"Wrapped.c4g");
        assert!(wrapped_entry.is_directory, "{label} wrapped source");
        assert_eq!(
            combined
                .open_child_entry_exact(&wrapped_entry)
                .unwrap()
                .read_file("Inside.txt")
                .unwrap(),
            b"wrapped child sentinel",
            "{label} wrapped child contents"
        );
    }
}

#[test]
fn retrieve_scenario_rejects_raw_unwrapped_material_files() {
    let mut raw_material = MutableGroup::new("Material.c4g");
    raw_material
        .add_file("Earth.c4m", b"raw material".to_vec())
        .unwrap();
    let raw_image = raw_material.pack_raw().unwrap();

    let directory = tempfile::tempdir().unwrap();
    let directory_scenario_path = directory.path().join("FolderScenario.c4s");
    std::fs::create_dir(&directory_scenario_path).unwrap();
    std::fs::write(directory_scenario_path.join("Material.c4g"), &raw_image).unwrap();
    let directory_scenario = Group::open(directory_scenario_path).unwrap();

    let mut packed_source = MutableGroup::new("PackedScenario.c4s");
    packed_source
        .add_file("Material.c4g", raw_image.clone())
        .unwrap();
    let packed_scenario = Group::from_memory(
        PathBuf::from("PackedScenario.c4s"),
        packed_source.pack().unwrap(),
    )
    .unwrap();

    let mut dynamic = MutableGroup::new("Dynamic.c4s");
    dynamic.add_file("Material.c4g", raw_image).unwrap();
    let dynamic =
        Group::from_memory(PathBuf::from("Dynamic.c4s"), dynamic.pack().unwrap()).unwrap();

    for scenario in [directory_scenario, packed_scenario] {
        let error = combine_network_scenario(&scenario, &dynamic, "Combined.c4s", "Network Client")
            .expect_err("C4Group_UnpackDirectory rejects raw Material.c4g files");
        assert_eq!(
            error.to_string(),
            "both network resources contain Material.c4g, but one is not a child group"
        );
    }
}

fn add_raw_child(
    parent: &mut MutableGroup,
    name: &[u8],
    raw: Vec<u8>,
    time: u32,
    executable: bool,
) {
    let child = Group::from_raw_memory(PathBuf::from("raw-child.c4g"), raw.clone()).unwrap();
    parent
        .add_packed_child_bytes_with_metadata(
            name.to_vec(),
            raw,
            child.contents_crc().unwrap(),
            time,
            executable,
        )
        .unwrap();
}

fn exact_entry(group: &Group, name: &[u8]) -> clonk_resources::GroupEntry {
    group
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name_bytes == name)
        .unwrap_or_else(|| panic!("missing raw C4Group entry {name:?}"))
}

fn read_exact_entry(group: &Group, name: &[u8]) -> Vec<u8> {
    let entry = exact_entry(group, name);
    group.read_entry_bytes_exact(&entry).unwrap()
}

fn stamp_reserved_header_bytes(image: &mut [u8], password: u8, reserved: u8) {
    let mut header: [u8; 204] = image[..204].try_into().unwrap();
    mem_unscramble(&mut header);
    header[72..104].fill(password);
    header[112..204].fill(reserved);
    mem_unscramble(&mut header);
    image[..204].copy_from_slice(&header);
}

fn mem_unscramble(buffer: &mut [u8]) {
    buffer.iter_mut().for_each(|byte| *byte ^= 237);
    for index in (0..buffer.len().saturating_sub(2)).step_by(3) {
        buffer.swap(index, index + 2);
    }
}
