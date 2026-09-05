use crate::support::dev_feedback::{
    run_replay_twice, run_replay_twice_with_policy, snapshot_diff, DevFeedbackCapture,
    ReplayCheckpointPolicy, ReplayCheckpointV1, ReplayInputV1, ReplayJoinV1, ReplayRenderConfigV1,
    ScenarioReplayV1, SNAPSHOT_HASH_VERSION,
};
use crate::support::virtual_player::{VirtualPlayer, VirtualPlayerError};
use clonk_engine::{Definition, Engine, PlayerConfig, SpawnConfig, COM_RIGHT};
use serde_json::json;
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn replay_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/dev-replays")
        .join(name)
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    crate::support::TestValueExt::test_value(LOCK.get_or_init(|| Mutex::new(())).lock())
}

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn synthetic_player_fixture() -> Result<Engine, Box<dyn Error>> {
    let mut engine = Engine::with_seed(11);
    let mut definition = Definition::from_script(
        "CLNK",
        "Dev feedback probe",
        "#strict 2\nprotected func ControlRight() { return(1); }\nprotected func ControlRightReleased() { return(1); }",
    )?;
    definition.set_crew_member(true);
    engine.register_definition(definition)?;
    engine.register_player(PlayerConfig::new(1, "Replay probe"))?;
    let crew = engine.spawn_object(
        SpawnConfig::new("CLNK")
            .with_owner(1)
            .with_crew_member(true),
    )?;
    engine.select_crew(1, [crew])?;
    engine.set_crew_cursor(1, Some(crew))?;
    Ok(engine)
}

fn synthetic_replay() -> ScenarioReplayV1 {
    ScenarioReplayV1 {
        schema_version: 1,
        snapshot_hash_version: SNAPSHOT_HASH_VERSION,
        scenario: "Tutorial.c4f/Tutorial01.c4s".to_owned(),
        seed: 11,
        joins: vec![ReplayJoinV1::local(0, 0, "Replay probe")],
        inputs: vec![
            ReplayInputV1::new(2, 1, 0, COM_RIGHT, 0),
            ReplayInputV1::new(2, 2, 0, COM_RIGHT + 16, 0),
        ],
        render: ReplayRenderConfigV1::headless(640, 480),
        stop_frame: 3,
        checkpoints: vec![ReplayCheckpointV1::new(0, "0000000000000000")],
    }
}

#[test]
fn replay_hash_version_defaults_to_legacy_and_rejects_unknown_versions() {
    let mut value =
        crate::support::TestValueExt::test_value(serde_json::to_value(synthetic_replay()));
    crate::support::TestValueExt::test_value(value.as_object_mut()).remove("snapshot_hash_version");
    let legacy = crate::support::TestValueExt::test_value(ScenarioReplayV1::from_json(
        &crate::support::TestValueExt::test_value(serde_json::to_string(&value)),
    ));
    assert_eq!(legacy.snapshot_hash_version, 1);

    value["snapshot_hash_version"] = serde_json::json!(SNAPSHOT_HASH_VERSION + 1);
    assert!(ScenarioReplayV1::from_json(
        &serde_json::to_string(&value).expect("unknown-version replay serializes")
    )
    .is_err());
}

#[test]
fn replay_json_is_canonical_and_repository_relative() -> Result<(), Box<dyn Error>> {
    let mut replay = synthetic_replay();
    replay.inputs.reverse();
    let first = replay.canonical_json()?;
    let second = ScenarioReplayV1::from_json(&first)?.canonical_json()?;

    assert_eq!(first, second);
    assert!(!first.contains(env!("CARGO_MANIFEST_DIR")));
    assert!(first.find("\"ordinal\": 1").unwrap() < first.find("\"ordinal\": 2").unwrap());
    assert!(ScenarioReplayV1::new("/absolute/scenario.c4s", 0, 0).is_err());
    assert!(ScenarioReplayV1::new("../outside.c4s", 0, 0).is_err());
    Ok(())
}

#[test]
fn snapshot_diff_has_stable_structured_paths() {
    let expected = Engine::with_seed(0).snapshot();
    let mut actual = expected.clone();
    actual.frame = 4;
    actual.game_time = 2;

    let diff = crate::support::TestValueExt::test_value(snapshot_diff(&expected, &actual, 16));
    assert_eq!(
        diff.entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/frame", "/game_time"]
    );
    assert_eq!(diff.entries[0].expected, json!(0));
    assert_eq!(diff.entries[0].actual, json!(4));
    assert!(!diff.truncated);
}

#[test]
fn committed_real_scenario_replays_are_deterministic() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    for name in [
        "tutorial01-idle.json",
        "tutorial01-right-tap.json",
        "tutorial01-held-right.json",
    ] {
        let path = replay_fixture(name);
        let replay = ScenarioReplayV1::from_path(&path)?;
        assert_eq!(
            fs::read_to_string(&path)?,
            replay.canonical_json()?,
            "fixture {name} must remain byte-canonical"
        );
        let report = run_replay_twice(&replay, name)?;
        assert_eq!(report.checkpoints, replay.checkpoints, "fixture {name}");
        assert_eq!(report.metrics.runs.len(), 2, "fixture {name}");
        for metrics in &report.metrics.runs {
            assert_eq!(metrics.ticks, replay.stop_frame, "fixture {name}");
            assert_eq!(metrics.start_frame, 0, "fixture {name}");
            assert_eq!(metrics.stop_frame, replay.stop_frame, "fixture {name}");
            assert_eq!(metrics.frames, replay.stop_frame + 1, "fixture {name}");
            assert_eq!(
                metrics.final_summary.frame, replay.stop_frame,
                "fixture {name}"
            );
            assert_eq!(
                metrics.final_snapshot_hash,
                replay.checkpoints.last().unwrap().snapshot_hash,
                "fixture {name}"
            );
        }
        if let Some(bundle) = report.artifact_dir {
            assert!(bundle.join("replay-metrics.json").is_file());
        }
    }
    Ok(())
}

#[test]
fn real_scenario_replays_repeat_with_native_group_order() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    for name in [
        "tutorial01-idle.json",
        "tutorial01-right-tap.json",
        "tutorial01-held-right.json",
    ] {
        let path = replay_fixture(name);
        let replay = ScenarioReplayV1::from_path(&path)?;
        assert_eq!(
            fs::read_to_string(&path)?,
            replay.canonical_json()?,
            "fixture {name} must remain byte-canonical"
        );
        let report = run_replay_twice_with_policy(&replay, name, ReplayCheckpointPolicy::SameHost)?;
        assert_eq!(
            report
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.frame)
                .collect::<Vec<_>>(),
            replay
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.frame)
                .collect::<Vec<_>>(),
            "fixture {name}"
        );
        let final_hash =
            &crate::support::TestValueExt::test_value(report.checkpoints.last()).snapshot_hash;
        assert!(
            report
                .metrics
                .runs
                .iter()
                .all(|metrics| &metrics.final_snapshot_hash == final_hash),
            "fixture {name}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "set LC_REPLAY_PATH to a replay artifact before running"]
fn replay_artifact_from_env_repeats() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(std::env::var_os("LC_REPLAY_PATH").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "LC_REPLAY_PATH must name the artifact replay.json",
        )
    })?);
    let replay = ScenarioReplayV1::from_path(&path)?;
    let label = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("artifact-replay");
    run_replay_twice(&replay, label)?;
    Ok(())
}

#[test]
fn virtual_player_timeout_writes_replay_bundle_after_held_key_release() -> Result<(), Box<dyn Error>>
{
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", temp.path());
    let mut engine = synthetic_player_fixture()?;
    let replay = synthetic_replay();
    let mut player = VirtualPlayer::with_dev_feedback(&mut engine, 1, replay, "held-timeout");

    let error = player
        .hold_until(COM_RIGHT, "never reached", 2, |_| false)
        .expect_err("the synthetic milestone is impossible");
    assert!(matches!(error, VirtualPlayerError::Timeout { .. }));
    let inputs = crate::support::TestValueExt::test_value(player.recorded_inputs());
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].command, COM_RIGHT);
    assert_eq!(inputs[1].command, COM_RIGHT + 16);

    let bundle = crate::support::TestValueExt::test_value(error.artifact_dir());
    assert!(bundle.starts_with(temp.path()));
    for file in [
        "replay.json",
        "replay-metrics.json",
        "failure.json",
        "logs.ndjson",
        "first.json",
        "before.json",
        "failing.json",
        "diff.json",
        "README.txt",
    ] {
        assert!(bundle.join(file).is_file(), "missing {file}");
    }
    let replay_json = fs::read_to_string(bundle.join("replay.json"))?;
    assert!(replay_json.contains(&format!("\"command\": {}", COM_RIGHT + 16)));
    assert!(error.to_string().contains(&bundle.display().to_string()));
    Ok(())
}

#[test]
fn virtual_player_timeout_writes_a_bounded_divergence_trace_when_enabled(
) -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", temp.path());
    let _capture = EnvRestore::set("LC_CAPTURE_VIRTUAL_PLAYER", "1");
    let mut engine = synthetic_player_fixture()?;
    let mut player = VirtualPlayer::new(&mut engine, 1);

    let error = player
        .hold_until(COM_RIGHT, "never reached", 2, |_| false)
        .expect_err("the synthetic milestone is impossible");
    assert!(matches!(error, VirtualPlayerError::Timeout { .. }));

    let bundle = crate::support::TestValueExt::test_value(error.artifact_dir());
    let trace: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("divergence-trace.json"))?)?;
    assert_eq!(trace["kind"], "virtual_player_timeout");
    assert_eq!(trace["inputs"].as_array().map(Vec::len), Some(2));
    assert!(bundle.join("first.json").is_file());
    assert!(bundle.join("before.json").is_file());
    assert!(bundle.join("failing.json").is_file());
    assert!(bundle.join("failure.json").is_file());
    assert!(bundle.join("logs.ndjson").is_file());
    Ok(())
}

#[test]
fn virtual_player_assertion_writes_a_divergence_trace_when_enabled() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", temp.path());
    let _capture = EnvRestore::set("LC_CAPTURE_VIRTUAL_PLAYER", "1");
    let mut engine = synthetic_player_fixture()?;
    let player = VirtualPlayer::new(&mut engine, 1);

    let error = player
        .assert_milestone("never reached", |_| false)
        .expect_err("the synthetic milestone is impossible");
    assert!(matches!(
        error,
        VirtualPlayerError::MilestoneNotReached { .. }
    ));

    let bundle = crate::support::TestValueExt::test_value(error.artifact_dir());
    let trace: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("divergence-trace.json"))?)?;
    assert_eq!(trace["kind"], "virtual_player_milestone");
    assert_eq!(trace["inputs"].as_array().map(Vec::len), Some(0));
    assert!(bundle.join("failure.json").is_file());
    Ok(())
}

#[test]
fn virtual_player_divergence_trace_bounds_recorded_inputs() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", temp.path());
    let engine = synthetic_player_fixture()?;
    let mut capture = DevFeedbackCapture::new_virtual_player("bounded-inputs", &engine);
    for frame in 0..4_097 {
        capture.record_input(frame, 1, COM_RIGHT, 0);
    }

    let bundle = crate::support::TestValueExt::test_value(capture.capture_timeout(
        &engine,
        "bounded input probe",
        4_097,
        "input tape bound",
    )?);
    let trace: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("divergence-trace.json"))?)?;
    assert_eq!(trace["inputs"].as_array().map(Vec::len), Some(4_096));
    assert_eq!(trace["dropped_inputs"], 1);
    Ok(())
}

#[test]
fn passing_replay_is_retained_with_metrics_when_requested() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", temp.path());
    let _keep = EnvRestore::set("LC_KEEP_PASS_ARTIFACTS", "1");
    let mut replay = ScenarioReplayV1::from_path(&replay_fixture("tutorial01-idle.json"))?;
    replay.checkpoints[0].snapshot_hash = "0000000000000000".to_owned();

    let report =
        run_replay_twice_with_policy(&replay, "retained-pass", ReplayCheckpointPolicy::SameHost)?;
    let bundle = crate::support::TestValueExt::test_value(report.artifact_dir);
    for file in [
        "replay.json",
        "replay-metrics.json",
        "first.json",
        "final.json",
        "failure.json",
        "logs.ndjson",
        "README.txt",
    ] {
        assert!(bundle.join(file).is_file(), "missing {file}");
    }
    let metrics: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("replay-metrics.json"))?)?;
    assert_eq!(metrics["schema_version"], 1);
    assert_eq!(metrics["snapshot_hash_version"], SNAPSHOT_HASH_VERSION);
    assert_eq!(metrics["runs"].as_array().unwrap().len(), 2);
    assert_eq!(metrics["runs"][0]["ticks"], replay.stop_frame);
    let retained_replay = ScenarioReplayV1::from_path(&bundle.join("replay.json"))?;
    assert_eq!(retained_replay.checkpoints, report.checkpoints);
    assert_ne!(retained_replay.checkpoints, replay.checkpoints);
    assert_eq!(
        metrics["runs"][0]["final_snapshot_hash"],
        report.checkpoints.last().unwrap().snapshot_hash
    );
    let failure: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("failure.json"))?)?;
    assert_eq!(failure["kind"], "same_host_passed");
    let logs = fs::read_to_string(bundle.join("logs.ndjson"))?;
    let log: serde_json::Value = serde_json::from_str(crate::support::TestValueExt::test_value(
        logs.lines().next(),
    ))?;
    assert_eq!(log["level"], "info");
    let readme = fs::read_to_string(bundle.join("README.txt"))?;
    assert!(readme.contains("LC_REPLAY_PATH=/path/to/artifact/replay.json"));
    assert!(readme.contains("dev_feedback_replay::replay_artifact_from_env_repeats"));
    Ok(())
}

#[test]
fn stale_checkpoint_writes_before_failing_diff_and_metrics() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", temp.path());
    let mut replay = ScenarioReplayV1::from_path(&replay_fixture("tutorial01-idle.json"))?;
    replay.checkpoints = run_replay_twice_with_policy(
        &replay,
        "native-checkpoints",
        ReplayCheckpointPolicy::SameHost,
    )?
    .checkpoints;
    let current_hash = std::mem::replace(
        &mut crate::support::TestValueExt::test_value(replay.checkpoints.first_mut()).snapshot_hash,
        "0000000000000000".to_owned(),
    );

    let error = run_replay_twice(&replay, "stale-checkpoint")
        .expect_err("a stale committed checkpoint must fail");
    assert!(error.to_string().contains("checkpoint hash mismatch"));
    let bundle = crate::support::TestValueExt::test_value(error.artifact_dir());
    for file in [
        "replay.json",
        "replay-metrics.json",
        "first.json",
        "final.json",
        "before.json",
        "failing.json",
        "diff.json",
        "failure.json",
        "logs.ndjson",
        "README.txt",
    ] {
        assert!(bundle.join(file).is_file(), "missing {file}");
    }
    let diff: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("diff.json"))?)?;
    assert_eq!(diff["entries"][0]["path"], "/snapshot_hash");
    assert_eq!(diff["entries"][0]["expected"], "0000000000000000");
    assert_eq!(diff["entries"][0]["actual"], current_hash);
    Ok(())
}

#[test]
fn artifact_write_failure_never_replaces_the_timeout() -> Result<(), Box<dyn Error>> {
    let _guard = env_lock();
    let temp = tempfile::tempdir()?;
    let not_a_directory = temp.path().join("file");
    fs::write(&not_a_directory, b"occupied")?;
    let _artifact_root = EnvRestore::set("LC_TEST_ARTIFACT_DIR", &not_a_directory);
    let mut engine = synthetic_player_fixture()?;
    let mut player =
        VirtualPlayer::with_dev_feedback(&mut engine, 1, synthetic_replay(), "unwritable-timeout");

    let error = player
        .wait_until("still impossible", 1, |_| false)
        .expect_err("timeout remains the primary error");
    assert!(matches!(
        error,
        VirtualPlayerError::Timeout {
            ref milestone,
            max_ticks: 1,
            ..
        } if milestone == "still impossible"
    ));
    assert!(error.artifact_dir().is_none());
    assert!(error.to_string().contains("artifact capture failed"));
    Ok(())
}

#[test]
fn replay_fixture_paths_exist() {
    for name in [
        "tutorial01-idle.json",
        "tutorial01-right-tap.json",
        "tutorial01-held-right.json",
    ] {
        assert!(Path::new(&replay_fixture(name)).is_file(), "missing {name}");
    }
}
