use std::fs;
use std::path::{Path, PathBuf};

use lc_engine::InitialNetworkScenarioMetadata;

#[path = "../src/host_game_resource_sources.rs"]
pub mod host_game_resource_sources;

use host_game_resource_sources::{
    resolve_host_game_resource_sources, HostGameResourceSourceError, HostGameResourceSourceKind,
};

#[test]
fn pristine_tutorial01_resolves_cpp_game_resource_order_and_wire_names() {
    // OpenScenario retains scenario modules and appends folder-local definition
    // groups; GameRes then emits Definitions*, System, ancestor Material*, and
    // global Material in that order (pristine 9ffa0a5d
    // src/C4Game.cpp:179-213,3936-3969;
    // src/C4GameParameters.cpp:192-224).
    let repository = repository_root();
    let content = repository.join("content");
    let planet = repository.join("planet");
    let scenario = content.join("Tutorial.c4f/Tutorial01.c4s");
    let metadata = metadata(vec!["Objects.c4d"]);

    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[content.clone(), planet.clone()],
        &metadata,
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
    let metadata = metadata(vec!["Objects.c4d", "Objects.c4d"]);

    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[content.clone(), planet.clone()],
        &metadata,
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
fn explicit_install_roots_shadow_by_caller_order() {
    // C4GameRes opens each logical filename exactly once in list order
    // (pristine 9ffa0a5d src/C4GameParameters.cpp:192-224). The Rust install
    // layout is an explicit ordered overlay of those assembled-root names, so
    // the first existing physical source must shadow later roots.
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

    let sources = resolve_host_game_resource_sources(
        &scenario,
        &[first.clone(), second],
        &metadata(vec!["Objects.c4d"]),
    )
    .unwrap();

    assert_eq!(sources.definitions[0].path, first.join("Objects.c4d"));
    assert_eq!(sources.system.path, first.join("System.c4g"));
    assert_eq!(sources.materials[0].path, first.join("Material.c4g"));
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

    let error = resolve_host_game_resource_sources(
        &scenario,
        &[first, second],
        &metadata(vec!["Objects.c4d"]),
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

fn metadata(definitions: Vec<&str>) -> InitialNetworkScenarioMetadata {
    InitialNetworkScenarioMetadata {
        icon: 0,
        definition_modules: definitions.into_iter().map(str::to_owned).collect(),
        random_seed: 0,
        max_players: 8,
        use_fair_crew: false,
        fair_crew_forced: false,
        fair_crew_strength: 0,
        rules: Vec::new(),
        goals: Vec::new(),
    }
}

fn source_pairs(sources: &[lc_network::HostInitialResourceSource]) -> Vec<(PathBuf, Vec<u8>)> {
    sources
        .iter()
        .map(|source| (source.path.clone(), source.wire_name.as_bytes().to_vec()))
        .collect()
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}
