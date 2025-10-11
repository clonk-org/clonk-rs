use lc_engine::{fixtures, EngineError, Playback, Recording};
use std::env;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::PathBuf;

fn run_snapshot_test<F>(baseline_name: &str, default_frames: usize, generator: F)
where
    F: Fn(usize) -> Result<Recording, EngineError>,
{
    let baseline_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../snapshots/engine/v1")
        .join(baseline_name);
    let update = env::var_os("UPDATE_ENGINE_SNAPSHOTS").is_some();

    let baseline = match File::open(&baseline_path) {
        Ok(file) => Some(
            Recording::from_reader(BufReader::new(file)).unwrap_or_else(|err| {
                panic!(
                    "failed to parse baseline {}: {err}",
                    baseline_path.display()
                )
            }),
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => panic!("failed to open baseline {}: {err}", baseline_path.display()),
    };

    let frames = baseline
        .as_ref()
        .map(|recording| recording.frames().len())
        .unwrap_or(default_frames);

    let actual = generator(frames)
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

    let playback = Playback::from_recording(baseline);
    playback
        .validate_sequence(actual.into_frames())
        .expect("engine output should match baseline");
}

#[test]
fn basic_movement_matches_snapshot() {
    run_snapshot_test("basic_movement.json", 6, |frames| {
        fixtures::basic_movement_recording(frames)
    });
}

#[test]
fn queued_commands_match_snapshot() {
    run_snapshot_test("queued_commands.json", 6, |frames| {
        fixtures::queued_command_recording(frames)
    });
}
