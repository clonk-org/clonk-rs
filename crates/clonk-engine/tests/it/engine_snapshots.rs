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
