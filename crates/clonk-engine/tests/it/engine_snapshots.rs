use clonk_engine::fixtures::SNAPSHOT_SCENARIOS;
use clonk_engine::{EngineError, Playback, Recording};
use std::env;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;

fn run_snapshot_test<F>(baseline_name: &str, default_frames: usize, generator: F)
where
    F: Fn(usize) -> Result<Recording, EngineError>,
{
    let baseline_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/engine/v1")
        .join(baseline_name);
    let update = env::var_os("UPDATE_ENGINE_SNAPSHOTS").is_some();

    let baseline = match File::open(&baseline_path) {
        Ok(file) => match Recording::from_reader(BufReader::new(file)) {
            Ok(recording) => Some(recording),
            Err(_err) if update => None,
            Err(err) => {
                panic!(
                    "failed to parse baseline {}: {err}",
                    baseline_path.display()
                )
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => panic!("failed to open baseline {}: {err}", baseline_path.display()),
    };

    let actual = generator(default_frames)
        .unwrap_or_else(|err| panic!("failed to generate recording for snapshot: {err}"));

    if update {
        if let Some(parent) = baseline_path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|err| {
                panic!(
                    "failed to ensure snapshot directory {}: {err}",
                    parent.display()
                )
            });
        }
        let mut file = File::create(&baseline_path).unwrap_or_else(|err| {
            panic!(
                "failed to open baseline for update {}: {err}",
                baseline_path.display()
            )
        });
        actual.to_writer(&mut file).unwrap_or_else(|err| {
            panic!(
                "failed to write baseline {}: {err}",
                baseline_path.display()
            )
        });
        return;
    }

    let baseline = baseline.unwrap_or_else(|| {
        panic!(
            "baseline {} is missing; rerun with UPDATE_ENGINE_SNAPSHOTS=1 to generate it",
            baseline_path.display()
        )
    });
    assert_eq!(
        baseline.frames().len(),
        default_frames,
        "baseline {} has a stale frame count",
        baseline_path.display()
    );

    let playback = Playback::from_recording(baseline);
    crate::support::TestValueExt::test_value(playback.validate_sequence(actual.into_frames()));
}

#[test]
fn engine_snapshots_match_baselines() {
    for scenario in SNAPSHOT_SCENARIOS {
        let baseline_name = format!("{}.json", scenario.name);
        run_snapshot_test(&baseline_name, scenario.default_frames, scenario.generator);
    }
}

#[test]
fn real_scenario_snapshot_definition_metadata_matches_uncached_projection() {
    use clonk_engine::{DefinitionLineMetadata, PlayerConfig};
    use std::collections::{BTreeMap, HashMap};

    let mut engine =
        crate::support::real_scenario::load_installed_scenario("Tutorial.c4f/Tutorial01.c4s", 0);
    engine
        .register_player(PlayerConfig::new(0, "viewer"))
        .unwrap();
    // Preserve the pre-cache projection formulas as an independent reference.
    let categories: HashMap<_, _> = engine
        .definition_ids()
        .map(|id| (id.to_owned(), engine.definition(id).unwrap().category()))
        .collect();
    let closed: BTreeMap<_, _> = engine
        .definition_ids()
        .filter_map(|id| {
            let closed = engine.definition(id).unwrap().closed_container();
            (closed != 0).then(|| (id.to_owned(), closed))
        })
        .collect();
    let lines: HashMap<_, _> = engine
        .definition_ids()
        .filter_map(|id| {
            let definition = engine.definition(id).unwrap();
            (definition.line() != 0 || definition.line_intersect() != 0).then(|| {
                (
                    id.to_owned(),
                    DefinitionLineMetadata {
                        line: definition.line(),
                        line_intersect: definition.line_intersect(),
                    },
                )
            })
        })
        .collect();
    assert!(!categories.is_empty());
    assert!(!closed.is_empty());
    assert!(!lines.is_empty());
    for fog in [false, true, false] {
        // Only the definition projection is under test, not the FoW view list.
        let _ = engine.player_mut(0).unwrap().set_fog_of_war(fog);
        let snapshot = engine.snapshot();
        assert_eq!(*snapshot.definition_categories, categories);
        assert_eq!(*snapshot.definition_lines, lines);
        if fog {
            assert_eq!(*snapshot.definition_closed_containers, closed);
        } else {
            assert!(snapshot.definition_closed_containers.is_empty());
        }
    }
}
