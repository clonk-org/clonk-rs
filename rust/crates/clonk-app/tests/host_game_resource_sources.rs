use std::fs;
use std::path::{Path, PathBuf};

#[path = "../src/host_game_resource_sources.rs"]
pub mod host_game_resource_sources;

use host_game_resource_sources::{
    freeze_host_definition_resource_sources, resolve_host_game_resource_sources,
    validate_host_group_resource_source, HostGameResourceSourceError, HostGameResourceSourceKind,
};
use clonk_engine::LegacyCString;
use clonk_network::HostInitialResourceSource;
use clonk_resources::MutableGroup;

#[test]
fn pristine_tutorial01_resolves_cpp_game_resource_order_and_wire_names() {
    // The caller supplies OpenScenario's already-resolved module/local vector;
    // GameRes then emits Definitions*, System, ancestor Material*, and global
    // Material in that order (pristine 9ffa0a5d
    // src/C4Game.cpp:179-213,3936-3969;
    // src/C4GameParameters.cpp:192-224).
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let scenario = content.join("Tutorial.c4f/Tutorial01.c4s");
    let definitions = vec![content.join("Objects.c4d"), content.join("Tutorial.c4f")];
    let definition_resources = freeze_host_definition_resource_sources(
        &definitions,
        &scenario,
        &["Objects.c4d".to_owned()],
        false,
        &content,
        "",
    )
    .unwrap();

    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[content.clone(), planet.clone()],
        &definition_resources,
        &content,
    )
    .unwrap();

    assert_eq!(
        source_pairs(&sources.definitions),
        vec![
            (content.join("Objects.c4d"), b"Objects.c4d".to_vec()),
            (content.join("Tutorial.c4f"), b"Tutorial.c4f".to_vec()),
        ]
    );
    assert_eq!(sources.system.path, planet.join("System.c4g"));
    assert_eq!(sources.system.wire_name.as_bytes(), b"System.c4g");
    assert_eq!(
        source_pairs(&sources.materials),
        vec![(content.join("Material.c4g"), b"Material.c4g".to_vec(),)]
    );
}

#[test]
fn nested_folders_keep_repeated_defs_and_publish_materials_inner_to_outer() {
    // FoldersWithLocalsDefs scans path prefixes from outer to inner. GroupSet
    // assigns increasing folder priority and FindGroup walks highest priority
    // first, so material resources run inner to outer; GameRes then appends the
    // global Material. The scenario's own Material is explicitly skipped
    // (pristine 9ffa0a5d src/C4Game.cpp:209-213,3936-3969;
    // src/C4GroupSet.cpp:87-100,161-178,238-318;
    // src/C4GameParameters.cpp:192-224).
    let fixture = tempfile::tempdir().unwrap();
    let content = fixture.path().join("content");
    let planet = fixture.path().join("planet");
    let outer = content.join("Outer.c4f");
    let empty = outer.join("Empty.c4f");
    let inner = empty.join("Inner.c4f");
    let scenario = inner.join("Nested.c4s");
    for path in [
        content.join("Objects.c4d"),
        content.join("Material.c4g"),
        planet.join("System.c4g"),
        outer.join("OuterOnly.c4d"),
        outer.join("Material.c4g"),
        inner.join("InnerOnly.c4d"),
        inner.join("Material.c4g"),
        scenario.join("Material.c4g"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(outer.join("OuterOnly.c4d/DefCore.txt"), b"[DefCore]").unwrap();
    fs::write(inner.join("InnerOnly.c4d/DefCore.txt"), b"[DefCore]").unwrap();
    let definitions = vec![
        content.join("Objects.c4d"),
        content.join("Objects.c4d"),
        outer.clone(),
        inner.clone(),
    ];
    let definition_resources = freeze_host_definition_resource_sources(
        &definitions,
        &scenario,
        &["Objects.c4d".to_owned(), "Objects.c4d".to_owned()],
        false,
        &content,
        "",
    )
    .unwrap();

    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[content.clone(), planet.clone()],
        &definition_resources,
        &content,
    )
    .unwrap();

    assert_eq!(
        source_pairs(&sources.definitions),
        vec![
            (content.join("Objects.c4d"), b"Objects.c4d".to_vec()),
            (content.join("Objects.c4d"), b"Objects.c4d".to_vec()),
            (outer.clone(), b"Outer.c4f".to_vec()),
            (inner.clone(), b"Outer.c4f/Empty.c4f/Inner.c4f".to_vec(),),
        ]
    );
    assert_eq!(
        source_pairs(&sources.materials),
        vec![
            (
                inner.join("Material.c4g"),
                b"Outer.c4f/Empty.c4f/Inner.c4f/Material.c4g".to_vec(),
            ),
            (
                outer.join("Material.c4g"),
                b"Outer.c4f/Material.c4g".to_vec(),
            ),
            (content.join("Material.c4g"), b"Material.c4g".to_vec(),),
        ]
    );
    assert!(!sources
        .materials
        .iter()
        .any(|source| source.path.starts_with(&scenario)));
}

#[test]
fn exact_effective_path_is_not_re_resolved_through_later_roots() {
    // C4GameRes opens every final DefinitionFilenames entry exactly once in
    // list order (pristine 9ffa0a5d src/C4GameParameters.cpp:192-224). Host
    // publication therefore consumes the staged physical path directly.
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first");
    let second = fixture.path().join("second");
    let scenario = first.join("Scenario.c4s");
    for root in [&first, &second] {
        for name in ["Objects.c4d", "System.c4g", "Material.c4g"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
    }
    fs::create_dir_all(&scenario).unwrap();

    let definition_resources = freeze_host_definition_resource_sources(
        &[first.join("Objects.c4d")],
        &scenario,
        &["Objects.c4d".to_owned()],
        false,
        &first,
        "",
    )
    .unwrap();
    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[first.clone(), second],
        &definition_resources,
        &first,
    )
    .unwrap();

    assert_eq!(sources.definitions[0].path, first.join("Objects.c4d"));
    assert_eq!(sources.system.path, first.join("System.c4g"));
    assert_eq!(sources.materials[0].path, first.join("Material.c4g"));
}

#[test]
fn definition_wire_names_strip_only_the_executable_root() {
    let fixture = tempfile::tempdir().unwrap();
    let content = fixture.path().join("content");
    let scenario_root = content.join("Scenarios");
    let planet = fixture.path().join("planet");
    let scenario = scenario_root.join("Round.c4s");
    let installed = content.join("Defs/Installed.c4d");
    let external = fixture.path().join("External.c4d");
    for path in [
        &scenario,
        &installed,
        &external,
        &content.join("Material.c4g"),
        &planet.join("System.c4g"),
    ] {
        fs::create_dir_all(path).unwrap();
    }

    let definition_resources = freeze_host_definition_resource_sources(
        &[installed.clone(), external.clone()],
        &scenario,
        &[
            "Defs/installed.c4d".to_owned(),
            external.to_string_lossy().into_owned(),
        ],
        false,
        &content,
        "",
    )
    .unwrap();
    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[scenario_root, content.clone(), planet],
        &definition_resources,
        &content,
    )
    .unwrap();

    assert_eq!(sources.definitions[0].path, installed);
    assert_eq!(
        sources.definitions[0].lookup_name.as_bytes(),
        b"Defs/installed.c4d"
    );
    assert_eq!(
        sources.definitions[0].opened_name.as_bytes(),
        if !cfg!(windows) && content.join("Defs/installed.c4d").exists() {
            b"Defs/installed.c4d"
        } else {
            b"Defs/Installed.c4d"
        }
    );
    assert_eq!(
        sources.definitions[0].wire_name.as_bytes(),
        b"Defs/installed.c4d"
    );
    assert_eq!(sources.definitions[1].path, external);
    assert_eq!(
        sources.definitions[1].lookup_name.as_bytes(),
        sources.definitions[1].opened_name.as_bytes()
    );
    assert_eq!(
        sources.definitions[1].wire_name.as_bytes(),
        sources.definitions[1]
            .path
            .to_string_lossy()
            .replace('\\', "/")
            .as_bytes()
    );
}

#[test]
fn folder_local_wire_name_retains_the_selected_scenario_prefix_spelling() {
    let fixture = tempfile::tempdir().unwrap();
    let content = fixture.path().join("content");
    let physical_folder = content.join("Outer.c4f");
    fs::create_dir_all(physical_folder.join("Local.c4d")).unwrap();
    fs::create_dir_all(physical_folder.join("Scenario.c4s")).unwrap();
    let selected_scenario = content.join("Out?r.c4f/Scenario.c4s");

    let definitions = freeze_host_definition_resource_sources(
        std::slice::from_ref(&physical_folder),
        &selected_scenario,
        &[],
        false,
        &content,
        "",
    )
    .unwrap();

    assert_eq!(definitions[0].path, physical_folder);
    assert_eq!(definitions[0].lookup_name.as_bytes(), b"Out?r.c4f");
    assert_eq!(definitions[0].opened_name.as_bytes(), b"Outer.c4f");
    assert_eq!(definitions[0].wire_name.as_bytes(), b"Out?r.c4f");
}

#[test]
fn selected_definition_wire_name_retains_native_high_bytes() {
    let fixture = tempfile::tempdir().unwrap();
    let physical = fixture.path().join("Installed.c4d");
    let expected_wire_name = b"Obj\xe4cts.c4d";
    let selected_module = clonk_script::c4_string_from_bytes(expected_wire_name);
    assert_eq!(
        clonk_script::c4_string_bytes(&selected_module),
        expected_wire_name
    );

    let definitions = freeze_host_definition_resource_sources(
        std::slice::from_ref(&physical),
        &fixture.path().join("Scenario.c4s"),
        &[selected_module],
        false,
        fixture.path(),
        "",
    )
    .unwrap();

    assert_eq!(definitions[0].path, physical);
    assert_eq!(definitions[0].wire_name.as_bytes(), expected_wire_name);
}

#[test]
fn selected_definition_wire_name_retains_separator_spelling() {
    let fixture = tempfile::tempdir().unwrap();
    let physical = fixture.path().join("Objects.c4d");
    let selected_module = r".\Objects.c4d".to_owned();

    let definitions = freeze_host_definition_resource_sources(
        std::slice::from_ref(&physical),
        &fixture.path().join("Scenario.c4s"),
        &[selected_module],
        false,
        fixture.path(),
        "",
    )
    .unwrap();

    assert_eq!(definitions[0].lookup_name.as_bytes(), br".\Objects.c4d");
    assert_eq!(
        definitions[0].opened_name.as_bytes(),
        if cfg!(windows) {
            br".\Objects.c4d"
        } else {
            b"./Objects.c4d"
        }
    );
    assert_eq!(definitions[0].wire_name.as_bytes(), br".\Objects.c4d");
}

#[test]
fn definition_resource_count_mismatch_counts_rooted_and_original_blocks() {
    let fixture = tempfile::tempdir().unwrap();
    let error = freeze_host_definition_resource_sources(
        &[fixture.path().join("OnlyOne.c4d")],
        &fixture.path().join("Scenario.c4s"),
        &["First.c4d".to_owned(), "Second.c4d".to_owned()],
        true,
        fixture.path(),
        "Custom/",
    )
    .unwrap_err();

    assert!(matches!(
        error,
        HostGameResourceSourceError::DefinitionResourceCountMismatch {
            actual: 1,
            expected: 4,
        }
    ));
}

#[test]
fn packed_nested_host_groups_are_snapshotted_for_network_publication() {
    fn child_group(name: &str, marker: &str) -> MutableGroup {
        let mut group = MutableGroup::new(name);
        group
            .add_file_with_metadata("Marker.txt", marker.as_bytes().to_vec(), 1, false)
            .unwrap();
        group
    }

    let fixture = tempfile::tempdir().unwrap();
    let content = fixture.path().join("content");
    let planet = fixture.path().join("planet");
    fs::create_dir_all(content.join("Material.c4g")).unwrap();
    fs::create_dir_all(planet.join("System.c4g")).unwrap();

    let mut scenario = MutableGroup::new("Nested.c4s");
    scenario
        .add_file_with_metadata(
            "Scenario.txt",
            b"[Head]\nTitle=Packed host\n".to_vec(),
            1,
            false,
        )
        .unwrap();
    let mut inner = MutableGroup::new("Inner.c4f");
    inner
        .add_child_with_metadata(
            "InnerOnly.c4d",
            child_group("InnerOnly.c4d", "inner definition"),
            1,
            false,
        )
        .unwrap();
    inner
        .add_child_with_metadata(
            "Material.c4g",
            child_group("Material.c4g", "inner material"),
            1,
            false,
        )
        .unwrap();
    inner
        .add_child_with_metadata("Nested.c4s", scenario, 1, false)
        .unwrap();
    let mut outer = MutableGroup::new("Outer.c4f");
    outer
        .add_child_with_metadata(
            "OuterOnly.c4d",
            child_group("OuterOnly.c4d", "outer definition"),
            1,
            false,
        )
        .unwrap();
    outer
        .add_child_with_metadata(
            "Material.c4g",
            child_group("Material.c4g", "outer material"),
            1,
            false,
        )
        .unwrap();
    outer
        .add_child_with_metadata("Inner.c4f", inner, 1, false)
        .unwrap();
    let outer_path = content.join("Outer.c4f");
    fs::create_dir_all(&content).unwrap();
    fs::write(&outer_path, outer.pack().unwrap()).unwrap();

    let inner_path = outer_path.join("Inner.c4f");
    let scenario_path = inner_path.join("Nested.c4s");
    let definition_resources = freeze_host_definition_resource_sources(
        &[outer_path.clone(), inner_path.clone()],
        &scenario_path,
        &[],
        false,
        &content,
        "",
    )
    .unwrap();
    let sources = resolve_host_game_resource_sources(
        &scenario_path,
        &[content.clone(), planet],
        &definition_resources,
        &content,
    )
    .unwrap();

    assert!(sources.definitions[0].virtual_group_bytes.is_none());
    assert!(sources.definitions[1].virtual_group_bytes.is_some());
    assert_eq!(sources.definitions[1].path, inner_path);
    assert_eq!(
        sources.definitions[1].wire_name.as_bytes(),
        b"Outer.c4f/Inner.c4f"
    );
    assert_eq!(
        sources
            .materials
            .iter()
            .map(|source| (
                source.wire_name.as_bytes(),
                source.virtual_group_bytes.is_some()
            ))
            .collect::<Vec<_>>(),
        vec![
            (b"Outer.c4f/Inner.c4f/Material.c4g".as_slice(), true),
            (b"Outer.c4f/Material.c4g".as_slice(), true),
            (b"Material.c4g".as_slice(), false),
        ]
    );

    let scenario_source = validate_host_group_resource_source(
        HostGameResourceSourceKind::Scenario,
        HostInitialResourceSource {
            path: scenario_path,
            lookup_name: LegacyCString::from_bytes(b"Outer.c4f/Inner.c4f/Nested.c4s".to_vec())
                .unwrap(),
            opened_name: LegacyCString::from_bytes(b"Outer.c4f/Inner.c4f/Nested.c4s".to_vec())
                .unwrap(),
            wire_name: LegacyCString::from_bytes(b"Outer.c4f/Inner.c4f/Nested.c4s".to_vec())
                .unwrap(),
            virtual_group_bytes: None,
        },
    )
    .unwrap();
    assert!(scenario_source.virtual_group_bytes.is_some());
}

#[test]
fn folder_material_identity_uses_the_opened_parent_spelling() {
    let fixture = tempfile::tempdir().unwrap();
    let content = fixture.path().join("content");
    let planet = fixture.path().join("planet");
    let actual_folder = content.join("Outer.c4f");
    let actual_scenario = actual_folder.join("Round.c4s");
    for path in [
        &actual_scenario,
        &actual_folder.join("Material.c4g"),
        &content.join("Material.c4g"),
        &planet.join("System.c4g"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    let selected_scenario = content.join("Out?r.c4f/Round.c4s");
    let sources = resolve_host_game_resource_sources(
        &selected_scenario,
        &[content.clone(), planet],
        &[],
        &content,
    )
    .unwrap();

    let folder_material = &sources.materials[0];
    assert_eq!(folder_material.path, actual_folder.join("Material.c4g"));
    assert_eq!(
        folder_material.lookup_name.as_bytes(),
        path_bytes(&actual_folder.join("Material.c4g"))
    );
    assert_eq!(
        folder_material.opened_name.as_bytes(),
        path_bytes(&actual_folder.join("Material.c4g"))
    );
    assert_eq!(
        folder_material.wire_name.as_bytes(),
        b"Outer.c4f/Material.c4g"
    );
}

#[cfg(windows)]
#[test]
fn global_resource_opened_names_use_win32_disk_case() {
    let fixture = tempfile::tempdir().unwrap();
    let content = fixture.path().join("content");
    let scenario = content.join("Scenario.c4s");
    for path in [
        &scenario,
        &content.join("system.c4g"),
        &content.join("material.c4g"),
    ] {
        fs::create_dir_all(path).unwrap();
    }

    let sources = resolve_host_game_resource_sources(
        &scenario,
        std::slice::from_ref(&content),
        &[],
        &content,
    )
    .unwrap();

    assert_eq!(sources.system.lookup_name.as_bytes(), b"System.c4g");
    assert_eq!(sources.system.opened_name.as_bytes(), b"system.c4g");
    assert_eq!(sources.system.wire_name.as_bytes(), b"System.c4g");
    let global_material = sources.materials.last().unwrap();
    assert_eq!(global_material.lookup_name.as_bytes(), b"Material.c4g");
    assert_eq!(global_material.opened_name.as_bytes(), b"material.c4g");
    assert_eq!(global_material.wire_name.as_bytes(), b"Material.c4g");
}

#[test]
fn corrupt_first_root_source_fails_typed_without_falling_through() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first");
    let second = fixture.path().join("second");
    let scenario = first.join("Scenario.c4s");
    for path in [
        &scenario,
        &first.join("System.c4g"),
        &first.join("Material.c4g"),
        &second.join("Objects.c4d"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    let corrupt = first.join("Objects.c4d");
    fs::write(&corrupt, b"not a C4Group").unwrap();

    let definition_resources = freeze_host_definition_resource_sources(
        std::slice::from_ref(&corrupt),
        &scenario,
        &["Objects.c4d".to_owned()],
        false,
        fixture.path(),
        "",
    )
    .unwrap();
    let error = resolve_host_game_resource_sources(
        &scenario,
        &[first, second],
        &definition_resources,
        fixture.path(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        HostGameResourceSourceError::ResourceGroup {
            kind: HostGameResourceSourceKind::Definition,
            path,
            ..
        } if path == corrupt
    ));
}

fn source_pairs(sources: &[clonk_network::HostInitialResourceSource]) -> Vec<(PathBuf, Vec<u8>)> {
    sources
        .iter()
        .map(|source| (source.path.clone(), source.wire_name.as_bytes().to_vec()))
        .collect()
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    clonk_script::c4_string_bytes(path.to_string_lossy().as_ref())
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}
