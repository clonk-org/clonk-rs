use lc_engine::{fixtures, Playback, Recording};
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

#[test]
fn basic_movement_matches_snapshot() {
    let baseline_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../snapshots/engine/v1/basic_movement.json");
    let file = File::open(&baseline_path)
        .unwrap_or_else(|err| panic!("failed to open baseline {}: {err}", baseline_path.display()));
    let baseline = Recording::from_reader(BufReader::new(file)).unwrap_or_else(|err| {
        panic!(
            "failed to parse baseline {}: {err}",
            baseline_path.display()
        )
    });
    let frames = baseline.frames().len();
    let actual =
        fixtures::basic_movement_recording(frames).expect("scenario recording should succeed");

    if env::var_os("UPDATE_ENGINE_SNAPSHOTS").is_some() {
        let mut file = File::create(&baseline_path).unwrap_or_else(|err| {
            panic!(
                "failed to open baseline for update {}: {err}",
                baseline_path.display()
            )
        });
        actual
            .to_writer(&mut file)
            .unwrap_or_else(|err| panic!("failed to write baseline {}: {err}", baseline_path.display()));
        return;
    }

    let playback = Playback::from_recording(baseline);
    playback
        .validate_sequence(actual.into_frames())
        .expect("engine output should match baseline");
}
