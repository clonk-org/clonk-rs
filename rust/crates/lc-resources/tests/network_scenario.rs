use std::path::PathBuf;

use lc_resources::{combine_network_scenario, Group, MutableGroup};

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
    dynamic
        .add_child("Material.c4g", dynamic_material)
        .unwrap();

    let scenario = Group::from_memory(
        PathBuf::from("Scenario.c4s"),
        scenario.pack().unwrap(),
    )
    .unwrap();
    let dynamic = Group::from_memory(
        PathBuf::from("Dynamic.c4s"),
        dynamic.pack().unwrap(),
    )
    .unwrap();

    let packed = combine_network_scenario(
        &scenario,
        &dynamic,
        "Combined1.c4s",
        "Network Client",
    )
    .unwrap();
    let combined = Group::from_memory(PathBuf::from("Combined1.c4s"), packed).unwrap();

    assert_eq!(combined.read_file("Base.txt").unwrap(), b"scenario-base");
    assert_eq!(combined.read_file("Replace.txt").unwrap(), b"dynamic-new");
    assert_eq!(combined.read_file("New.txt").unwrap(), b"dynamic-only");
    let material = combined.open_child("Material.c4g").unwrap();
    assert_eq!(
        material.read_file("BaseMat.txt").unwrap(),
        b"material-base"
    );
    assert_eq!(
        material.read_file("ReplaceMat.txt").unwrap(),
        b"material-old"
    );
    assert!(!material.exists("NewMat.txt"));
    assert_eq!(combined.maker(), Some("Network Client"));
}
