use super::real_scenario::{prepare_installed_scenario, PreparedInstalledScenario};
use clonk_engine::player_file::CrewInfo;
use clonk_engine::{Engine, JoinPlayerConfig, SimulationSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const REPLAY_SCHEMA_VERSION: u32 = 1;
/// Version 2 removes Rust-only Surface8 presentation-cache lineage from the
/// checkpoint projection while retaining every authoritative landscape byte.
pub const SNAPSHOT_HASH_VERSION: u32 = 2;
const LEGACY_SNAPSHOT_HASH_VERSION: u32 = 1;
const DEFAULT_DIFF_LIMIT: usize = 64;
const MAX_RECORDED_INPUTS: usize = 4_096;
const SNAPSHOT_SAMPLE_INTERVAL: u64 = 32;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug)]
pub struct DevFeedbackError(String);

impl DevFeedbackError {
    fn message(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for DevFeedbackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DevFeedbackError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioReplayV1 {
    pub schema_version: u32,
    /// Independently versioned because the input tape remains replay schema
    /// v1 while its deterministic-state projection can be migrated safely.
    #[serde(default = "legacy_snapshot_hash_version")]
    pub snapshot_hash_version: u32,
    pub scenario: String,
    pub seed: u64,
    #[serde(default)]
    pub joins: Vec<ReplayJoinV1>,
    #[serde(default)]
    pub inputs: Vec<ReplayInputV1>,
    pub render: ReplayRenderConfigV1,
    pub stop_frame: u64,
    #[serde(default)]
    pub checkpoints: Vec<ReplayCheckpointV1>,
}

impl ScenarioReplayV1 {
    pub fn new(
        scenario: impl Into<String>,
        seed: u64,
        stop_frame: u64,
    ) -> Result<Self, DevFeedbackError> {
        let replay = Self {
            schema_version: REPLAY_SCHEMA_VERSION,
            snapshot_hash_version: SNAPSHOT_HASH_VERSION,
            scenario: normalize_scenario_path(&scenario.into())?,
            seed,
            joins: Vec::new(),
            inputs: Vec::new(),
            render: ReplayRenderConfigV1::headless(640, 480),
            stop_frame,
            checkpoints: Vec::new(),
        };
        replay.validate()?;
        Ok(replay)
    }

    pub fn from_json(json: &str) -> Result<Self, DevFeedbackError> {
        let replay: Self = serde_json::from_str(json)
            .map_err(|error| DevFeedbackError::message(format!("parse replay JSON: {error}")))?;
        replay.validate()?;
        Ok(replay)
    }

    pub fn from_path(path: &Path) -> Result<Self, DevFeedbackError> {
        let json = fs::read_to_string(path).map_err(|error| {
            DevFeedbackError::message(format!("read replay `{}`: {error}", path.display()))
        })?;
        Self::from_json(&json)
    }

    pub fn canonical_json(&self) -> Result<String, DevFeedbackError> {
        let mut canonical = self.clone();
        canonical.scenario = normalize_scenario_path(&canonical.scenario)?;
        canonical
            .joins
            .sort_by_key(|join| (join.frame, join.ordinal));
        canonical
            .inputs
            .sort_by_key(|input| (input.frame, input.ordinal));
        canonical.render.capture_frames.sort_unstable();
        canonical.render.capture_frames.dedup();
        canonical
            .checkpoints
            .sort_by_key(|checkpoint| checkpoint.frame);
        for checkpoint in &mut canonical.checkpoints {
            checkpoint.snapshot_hash.make_ascii_lowercase();
        }
        canonical.validate()?;
        let json = serde_json::to_string_pretty(&canonical).map_err(|error| {
            DevFeedbackError::message(format!("serialize canonical replay: {error}"))
        })?;
        Ok(format!("{json}\n"))
    }

    pub fn validate(&self) -> Result<(), DevFeedbackError> {
        if self.schema_version != REPLAY_SCHEMA_VERSION {
            return Err(DevFeedbackError::message(format!(
                "unsupported replay schema version {} (expected {})",
                self.schema_version, REPLAY_SCHEMA_VERSION
            )));
        }
        let normalized = normalize_scenario_path(&self.scenario)?;
        if normalized != self.scenario {
            return Err(DevFeedbackError::message(format!(
                "scenario path `{}` is not canonical; use `{normalized}`",
                self.scenario
            )));
        }
        if self.render.width == 0 || self.render.height == 0 {
            return Err(DevFeedbackError::message(
                "replay render dimensions must be non-zero",
            ));
        }
        if ![LEGACY_SNAPSHOT_HASH_VERSION, SNAPSHOT_HASH_VERSION]
            .contains(&self.snapshot_hash_version)
        {
            return Err(DevFeedbackError::message(format!(
                "unsupported snapshot hash version {} (expected {} or {})",
                self.snapshot_hash_version, LEGACY_SNAPSHOT_HASH_VERSION, SNAPSHOT_HASH_VERSION
            )));
        }
        if let Some(frame) = self
            .render
            .capture_frames
            .iter()
            .find(|&&frame| frame > self.stop_frame)
        {
            return Err(DevFeedbackError::message(format!(
                "render capture frame {frame} exceeds stop frame {}",
                self.stop_frame
            )));
        }
        let mut event_keys = BTreeSet::new();
        for join in &self.joins {
            validate_event_key(
                &mut event_keys,
                join.frame,
                join.ordinal,
                self.stop_frame,
                "join",
            )?;
        }
        for input in &self.inputs {
            validate_event_key(
                &mut event_keys,
                input.frame,
                input.ordinal,
                self.stop_frame,
                "input",
            )?;
        }
        let mut checkpoint_frames = BTreeSet::new();
        for checkpoint in &self.checkpoints {
            if checkpoint.frame > self.stop_frame {
                return Err(DevFeedbackError::message(format!(
                    "checkpoint frame {} exceeds stop frame {}",
                    checkpoint.frame, self.stop_frame
                )));
            }
            if !checkpoint_frames.insert(checkpoint.frame) {
                return Err(DevFeedbackError::message(format!(
                    "duplicate checkpoint frame {}",
                    checkpoint.frame
                )));
            }
            if checkpoint.snapshot_hash.len() != 16
                || !checkpoint
                    .snapshot_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(DevFeedbackError::message(format!(
                    "checkpoint frame {} has invalid 64-bit hex hash `{}`",
                    checkpoint.frame, checkpoint.snapshot_hash
                )));
            }
        }
        Ok(())
    }
}

fn validate_event_key(
    keys: &mut BTreeSet<(u64, u64)>,
    frame: u64,
    ordinal: u64,
    stop_frame: u64,
    kind: &str,
) -> Result<(), DevFeedbackError> {
    if frame > stop_frame {
        return Err(DevFeedbackError::message(format!(
            "{kind} frame {frame} exceeds stop frame {stop_frame}"
        )));
    }
    if !keys.insert((frame, ordinal)) {
        return Err(DevFeedbackError::message(format!(
            "duplicate replay event key frame={frame} ordinal={ordinal}"
        )));
    }
    Ok(())
}

fn normalize_scenario_path(path: &str) -> Result<String, DevFeedbackError> {
    let path = path.replace('\\', "/");
    if path.is_empty() || Path::new(&path).is_absolute() {
        return Err(DevFeedbackError::message(format!(
            "scenario path `{path}` must be repository-relative"
        )));
    }
    if Path::new(&path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(DevFeedbackError::message(format!(
            "scenario path `{path}` may not contain `.` or `..` components"
        )));
    }
    Ok(path)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayCrewInfoV1 {
    pub id: String,
    pub name: String,
    pub rank: i32,
    pub experience: i32,
    pub participation: i32,
    pub in_action: bool,
    pub has_died: bool,
}

impl ReplayCrewInfoV1 {
    fn to_engine(&self) -> CrewInfo {
        CrewInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            rank: self.rank,
            experience: self.experience,
            participation: self.participation,
            in_action: self.in_action,
            has_died: self.has_died,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayJoinV1 {
    pub frame: u64,
    pub ordinal: u64,
    pub expected_owner: i32,
    pub name: String,
    pub player_info_id: i32,
    pub score: i32,
    pub total_playing_time: i32,
    pub team: Option<i32>,
    pub color_dw: u32,
    pub pref_color: i32,
    pub pref_position: i32,
    #[serde(default)]
    pub crew: Vec<ReplayCrewInfoV1>,
    pub control_style: bool,
    pub auto_context_menu: bool,
    pub startup_player_count: i32,
}

impl ReplayJoinV1 {
    pub fn local(frame: u64, ordinal: u64, name: impl Into<String>) -> Self {
        Self {
            frame,
            ordinal,
            expected_owner: 0,
            name: name.into(),
            player_info_id: 0,
            score: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: true,
            auto_context_menu: true,
            startup_player_count: 1,
        }
    }

    fn to_engine(&self) -> JoinPlayerConfig {
        JoinPlayerConfig {
            name: self.name.clone(),
            player_info_id: self.player_info_id,
            score: self.score,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: self.total_playing_time,
            team: self.team,
            color_dw: self.color_dw,
            pref_color: self.pref_color,
            pref_position: self.pref_position,
            crew: self.crew.iter().map(ReplayCrewInfoV1::to_engine).collect(),
            control_style: self.control_style,
            auto_context_menu: self.auto_context_menu,
            startup_player_count: self.startup_player_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayInputV1 {
    pub frame: u64,
    pub ordinal: u64,
    pub owner: i32,
    pub command: u8,
    pub data: i32,
}

impl ReplayInputV1 {
    pub fn new(frame: u64, ordinal: u64, owner: i32, command: u8, data: i32) -> Self {
        Self {
            frame,
            ordinal,
            owner,
            command,
            data,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayRenderConfigV1 {
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub capture_frames: Vec<u64>,
}

impl ReplayRenderConfigV1 {
    pub fn headless(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            capture_frames: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayCheckpointV1 {
    pub frame: u64,
    pub snapshot_hash: String,
}

impl ReplayCheckpointV1 {
    pub fn new(frame: u64, snapshot_hash: impl Into<String>) -> Self {
        Self {
            frame,
            snapshot_hash: snapshot_hash.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDiffEntry {
    pub path: String,
    pub expected: Value,
    pub actual: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub entries: Vec<SnapshotDiffEntry>,
    pub truncated: bool,
}

pub fn snapshot_diff(
    expected: &SimulationSnapshot,
    actual: &SimulationSnapshot,
    limit: usize,
) -> Option<SnapshotDiff> {
    if expected == actual {
        return None;
    }
    let expected = serde_json::to_value(expected).ok()?;
    let actual = serde_json::to_value(actual).ok()?;
    Some(value_diff(&expected, &actual, limit))
}

fn value_diff(expected: &Value, actual: &Value, limit: usize) -> SnapshotDiff {
    let mut diff = SnapshotDiff {
        entries: Vec::new(),
        truncated: false,
    };
    collect_value_diff(expected, actual, "", limit, &mut diff);
    diff
}

fn collect_value_diff(
    expected: &Value,
    actual: &Value,
    path: &str,
    limit: usize,
    diff: &mut SnapshotDiff,
) {
    if expected == actual {
        return;
    }
    if diff.entries.len() >= limit {
        diff.truncated = true;
        return;
    }
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                let child_path = format!("{path}/{}", escape_json_pointer(key));
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        collect_value_diff(expected, actual, &child_path, limit, diff)
                    }
                    (Some(expected), None) => {
                        push_diff(diff, limit, child_path, expected.clone(), Value::Null)
                    }
                    (None, Some(actual)) => {
                        push_diff(diff, limit, child_path, Value::Null, actual.clone())
                    }
                    (None, None) => {}
                }
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            for index in 0..expected.len().max(actual.len()) {
                let child_path = format!("{path}/{index}");
                match (expected.get(index), actual.get(index)) {
                    (Some(expected), Some(actual)) => {
                        collect_value_diff(expected, actual, &child_path, limit, diff)
                    }
                    (Some(expected), None) => {
                        push_diff(diff, limit, child_path, expected.clone(), Value::Null)
                    }
                    (None, Some(actual)) => {
                        push_diff(diff, limit, child_path, Value::Null, actual.clone())
                    }
                    (None, None) => {}
                }
            }
        }
        _ => push_diff(
            diff,
            limit,
            path.to_owned(),
            expected.clone(),
            actual.clone(),
        ),
    }
}

fn push_diff(diff: &mut SnapshotDiff, limit: usize, path: String, expected: Value, actual: Value) {
    if diff.entries.len() >= limit {
        diff.truncated = true;
    } else {
        diff.entries.push(SnapshotDiffEntry {
            path,
            expected,
            actual,
        });
    }
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaySnapshotSummaryV1 {
    pub frame: u64,
    pub object_count: usize,
    pub player_count: usize,
    pub global_effect_count: usize,
    pub particle_count: usize,
}

impl ReplaySnapshotSummaryV1 {
    fn from_snapshot(snapshot: &SimulationSnapshot) -> Self {
        Self {
            frame: snapshot.frame,
            object_count: snapshot.objects.len(),
            player_count: snapshot.players.len(),
            global_effect_count: snapshot.global_effects.len(),
            particle_count: snapshot.particles.len(),
        }
    }
}

/// Wall-clock fields intentionally live outside `ScenarioReplayV1`, so the
/// canonical replay stays byte-stable across machines while artifacts can
/// still separate loader, join, and simulation costs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayRunMetricsV1 {
    pub schema_version: u32,
    pub load_micros: u128,
    pub join_micros: u128,
    pub simulation_micros: u128,
    pub start_frame: u64,
    pub stop_frame: u64,
    pub frames: u64,
    pub observed_frames: usize,
    pub ticks: u64,
    pub final_snapshot_hash: String,
    pub final_summary: ReplaySnapshotSummaryV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayMetricsFileV1 {
    pub schema_version: u32,
    #[serde(default = "legacy_snapshot_hash_version")]
    pub snapshot_hash_version: u32,
    pub runs: Vec<ReplayRunMetricsV1>,
}

#[derive(Clone, Debug)]
struct ReplayCapture {
    observations: BTreeMap<u64, SimulationSnapshot>,
    metrics: ReplayRunMetricsV1,
}

#[derive(Clone, Debug)]
pub struct ReplayRunReport {
    pub checkpoints: Vec<ReplayCheckpointV1>,
    pub metrics: ReplayMetricsFileV1,
    pub artifact_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayCheckpointPolicy {
    /// Require the first run to match the hashes stored in the replay fixture.
    Committed,
    /// Compare both runs while projecting the first run's native-host hashes
    /// into retained artifacts.
    SameHost,
}

#[derive(Debug)]
pub struct ReplayRunError {
    message: String,
    artifact_dir: Option<PathBuf>,
    artifact_warning: Option<String>,
}

impl ReplayRunError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            artifact_dir: None,
            artifact_warning: None,
        }
    }

    pub fn artifact_dir(&self) -> Option<&Path> {
        self.artifact_dir.as_deref()
    }
}

impl fmt::Display for ReplayRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(path) = &self.artifact_dir {
            write!(formatter, "; artifacts: {}", path.display())?;
        }
        if let Some(warning) = &self.artifact_warning {
            write!(formatter, "; artifact capture failed: {warning}")?;
        }
        Ok(())
    }
}

impl Error for ReplayRunError {}

pub fn run_replay_twice(
    replay: &ScenarioReplayV1,
    artifact_label: &str,
) -> Result<ReplayRunReport, ReplayRunError> {
    run_replay_twice_with_policy(replay, artifact_label, ReplayCheckpointPolicy::Committed)
}

/// Runs two fresh engines and selects whether the first run must also match the
/// fixture's recording-host hashes.
pub fn run_replay_twice_with_policy(
    replay: &ScenarioReplayV1,
    artifact_label: &str,
    checkpoint_policy: ReplayCheckpointPolicy,
) -> Result<ReplayRunReport, ReplayRunError> {
    replay
        .validate()
        .map_err(|error| ReplayRunError::new(error.to_string()))?;
    let prepare_started = Instant::now();
    let scenario = prepare_installed_scenario(&replay.scenario, replay.seed);
    let prepare_elapsed = prepare_started.elapsed();
    // Parsing and I/O are immutable preparation shared by the two fresh
    // engines. Attribute that measured cost to each run's logical load phase
    // so `load_micros` remains comparable with pre-reuse replay artifacts.
    let first = run_replay_once(replay, &scenario, prepare_elapsed).map_err(ReplayRunError::new)?;
    let second =
        run_replay_once(replay, &scenario, prepare_elapsed).map_err(ReplayRunError::new)?;
    let metrics = ReplayMetricsFileV1 {
        schema_version: REPLAY_SCHEMA_VERSION,
        snapshot_hash_version: replay.snapshot_hash_version,
        runs: vec![first.metrics.clone(), second.metrics.clone()],
    };
    let actual_checkpoints = replay
        .checkpoints
        .iter()
        .map(|checkpoint| {
            let snapshot =
                crate::support::TestValueExt::test_value(first.observations.get(&checkpoint.frame));
            ReplayCheckpointV1::new(
                checkpoint.frame,
                snapshot_hash(snapshot, replay.snapshot_hash_version),
            )
        })
        .collect::<Vec<_>>();
    let same_host_replay =
        (checkpoint_policy == ReplayCheckpointPolicy::SameHost).then(|| ScenarioReplayV1 {
            checkpoints: actual_checkpoints.clone(),
            ..replay.clone()
        });
    let artifact_replay = same_host_replay.as_ref().unwrap_or(replay);

    for (&frame, expected) in &first.observations {
        let Some(actual) = second.observations.get(&frame) else {
            let mut error =
                ReplayRunError::new(format!("second replay omitted observed frame {frame}"));
            attach_replay_failure_artifacts(
                &mut error,
                ReplayFailureArtifacts {
                    replay: artifact_replay,
                    label: artifact_label,
                    metrics: &metrics,
                    first: &first,
                    second: &second,
                    frame,
                    diff: None,
                },
            );
            return Err(error);
        };
        if expected != actual {
            let diff = snapshot_diff(expected, actual, DEFAULT_DIFF_LIMIT);
            let first_path = diff
                .as_ref()
                .and_then(|diff| diff.entries.first())
                .map(|entry| entry.path.as_str())
                .unwrap_or("<unknown>");
            let mut error = ReplayRunError::new(format!(
                "replay diverged at frame {frame}, first difference {first_path}"
            ));
            attach_replay_failure_artifacts(
                &mut error,
                ReplayFailureArtifacts {
                    replay: artifact_replay,
                    label: artifact_label,
                    metrics: &metrics,
                    first: &first,
                    second: &second,
                    frame,
                    diff,
                },
            );
            return Err(error);
        }
    }

    if checkpoint_policy == ReplayCheckpointPolicy::Committed {
        for (expected, actual) in replay.checkpoints.iter().zip(&actual_checkpoints) {
            if expected != actual {
                let actuals = actual_checkpoints
                    .iter()
                    .map(|checkpoint| format!("{}={}", checkpoint.frame, checkpoint.snapshot_hash))
                    .collect::<Vec<_>>()
                    .join(", ");
                let diff = SnapshotDiff {
                    entries: vec![SnapshotDiffEntry {
                        path: "/snapshot_hash".to_owned(),
                        expected: json!(expected.snapshot_hash),
                        actual: json!(actual.snapshot_hash),
                    }],
                    truncated: false,
                };
                let mut error = ReplayRunError::new(format!(
                    "checkpoint hash mismatch at frame {}: expected {}, actual {}; actual checkpoints: [{}]",
                    expected.frame, expected.snapshot_hash, actual.snapshot_hash, actuals
                ));
                attach_replay_failure_artifacts(
                    &mut error,
                    ReplayFailureArtifacts {
                        replay,
                        label: artifact_label,
                        metrics: &metrics,
                        first: &first,
                        second: &second,
                        frame: expected.frame,
                        diff: Some(diff),
                    },
                );
                return Err(error);
            }
        }
    }

    let (kind, message) = match checkpoint_policy {
        ReplayCheckpointPolicy::Committed => ("passed", "replay completed without mismatches"),
        ReplayCheckpointPolicy::SameHost => (
            "same_host_passed",
            "replay repeated without same-host mismatches",
        ),
    };
    let artifact_dir = if keep_pass_artifacts() {
        write_replay_bundle(ReplayBundleRequest {
            replay: artifact_replay,
            kind,
            label: artifact_label,
            message,
            metrics: &metrics,
            first: &first,
            second: &second,
            frame: replay.stop_frame,
            diff: None,
        })
        .map_err(|error| ReplayRunError::new(format!("write passing replay artifact: {error}")))?
    } else {
        None
    };
    Ok(ReplayRunReport {
        checkpoints: actual_checkpoints,
        metrics,
        artifact_dir,
    })
}

fn run_replay_once(
    replay: &ScenarioReplayV1,
    scenario: &PreparedInstalledScenario,
    prepare_elapsed: Duration,
) -> Result<ReplayCapture, String> {
    let load_started = Instant::now();
    let mut engine = scenario.instantiate();
    let load_elapsed = prepare_elapsed.saturating_add(load_started.elapsed());
    let simulation_started = Instant::now();
    let mut join_elapsed = Duration::ZERO;
    let mut observations = BTreeMap::new();
    let observation_frames: BTreeSet<u64> = replay
        .checkpoints
        .iter()
        .map(|checkpoint| checkpoint.frame)
        .chain([0, replay.stop_frame])
        .collect();

    enum Event<'a> {
        Join(&'a ReplayJoinV1),
        Input(&'a ReplayInputV1),
    }

    for frame in 0..=replay.stop_frame {
        let mut events = replay
            .joins
            .iter()
            .filter(|join| join.frame == frame)
            .map(|join| (join.ordinal, Event::Join(join)))
            .chain(
                replay
                    .inputs
                    .iter()
                    .filter(|input| input.frame == frame)
                    .map(|input| (input.ordinal, Event::Input(input))),
            )
            .collect::<Vec<_>>();
        events.sort_by_key(|(ordinal, _)| *ordinal);
        for (_, event) in events {
            match event {
                Event::Join(join) => {
                    let started = Instant::now();
                    let joined = engine.join_player(join.to_engine()).map_err(|error| {
                        format!("join `{}` at frame {frame}: {error}", join.name)
                    })?;
                    join_elapsed += started.elapsed();
                    if joined.number() != join.expected_owner {
                        return Err(format!(
                            "join `{}` at frame {frame} expected owner {}, got {}",
                            join.name,
                            join.expected_owner,
                            joined.number()
                        ));
                    }
                }
                Event::Input(input) => engine
                    .player_in_com(input.owner, input.command, input.data)
                    .map_err(|error| {
                        format!(
                            "input frame={frame} ordinal={} owner={} command={} data={}: {error}",
                            input.ordinal, input.owner, input.command, input.data
                        )
                    })?,
            }
        }
        if observation_frames.contains(&frame) {
            observations.insert(frame, engine.snapshot());
        }
        if frame != replay.stop_frame {
            engine
                .tick_without_snapshot()
                .map_err(|error| format!("tick from replay frame {frame}: {error}"))?;
        }
    }

    let simulation_elapsed = simulation_started.elapsed().saturating_sub(join_elapsed);
    let final_snapshot =
        crate::support::TestValueExt::test_value(observations.get(&replay.stop_frame));
    let metrics = ReplayRunMetricsV1 {
        schema_version: REPLAY_SCHEMA_VERSION,
        load_micros: load_elapsed.as_micros(),
        join_micros: join_elapsed.as_micros(),
        simulation_micros: simulation_elapsed.as_micros(),
        start_frame: 0,
        stop_frame: replay.stop_frame,
        frames: replay.stop_frame.saturating_add(1),
        observed_frames: observations.len(),
        ticks: replay.stop_frame,
        final_snapshot_hash: snapshot_hash(final_snapshot, replay.snapshot_hash_version),
        final_summary: ReplaySnapshotSummaryV1::from_snapshot(final_snapshot),
    };
    Ok(ReplayCapture {
        observations,
        metrics,
    })
}

struct ReplayFailureArtifacts<'a> {
    replay: &'a ScenarioReplayV1,
    label: &'a str,
    metrics: &'a ReplayMetricsFileV1,
    first: &'a ReplayCapture,
    second: &'a ReplayCapture,
    frame: u64,
    diff: Option<SnapshotDiff>,
}

fn attach_replay_failure_artifacts(
    error: &mut ReplayRunError,
    failure: ReplayFailureArtifacts<'_>,
) {
    let message = error.message.clone();
    match write_replay_bundle(ReplayBundleRequest {
        replay: failure.replay,
        kind: "replay_mismatch",
        label: failure.label,
        message: &message,
        metrics: failure.metrics,
        first: failure.first,
        second: failure.second,
        frame: failure.frame,
        diff: failure.diff,
    }) {
        Ok(path) => error.artifact_dir = path,
        Err(capture_error) => error.artifact_warning = Some(capture_error.to_string()),
    }
}

struct ReplayBundleRequest<'a> {
    replay: &'a ScenarioReplayV1,
    kind: &'a str,
    label: &'a str,
    message: &'a str,
    metrics: &'a ReplayMetricsFileV1,
    first: &'a ReplayCapture,
    second: &'a ReplayCapture,
    frame: u64,
    diff: Option<SnapshotDiff>,
}

fn write_replay_bundle(
    request: ReplayBundleRequest<'_>,
) -> Result<Option<PathBuf>, DevFeedbackError> {
    let Some(root) = artifact_root() else {
        return Ok(None);
    };
    create_bundle(&root, request.label, |directory| {
        fs::write(
            directory.join("replay.json"),
            request.replay.canonical_json()?,
        )
        .map_err(|error| io_error("write replay.json", error))?;
        write_json(directory.join("replay-metrics.json"), request.metrics)?;
        let first_snapshot = request
            .first
            .observations
            .values()
            .next()
            .ok_or_else(|| DevFeedbackError::message("first replay has no snapshots"))?;
        let final_snapshot = request
            .first
            .observations
            .get(&request.replay.stop_frame)
            .ok_or_else(|| DevFeedbackError::message("first replay has no final snapshot"))?;
        write_json(directory.join("first.json"), first_snapshot)?;
        write_json(directory.join("final.json"), final_snapshot)?;
        let actual = request
            .second
            .observations
            .get(&request.frame)
            .or_else(|| request.second.observations.values().next_back())
            .ok_or_else(|| DevFeedbackError::message("second replay has no snapshots"))?;
        let before = request
            .first
            .observations
            .range(..request.frame)
            .next_back()
            .map(|(_, snapshot)| snapshot)
            .unwrap_or(first_snapshot);
        write_json(directory.join("before.json"), before)?;
        write_json(directory.join("failing.json"), actual)?;
        write_json(
            directory.join("diff.json"),
            &request.diff.unwrap_or(SnapshotDiff {
                entries: Vec::new(),
                truncated: false,
            }),
        )?;
        write_json(
            directory.join("failure.json"),
            &json!({
                "schema_version": REPLAY_SCHEMA_VERSION,
                "kind": request.kind,
                "frame": request.frame,
                "label": request.label,
                "message": request.message,
            }),
        )?;
        write_logs(
            directory,
            &json!({
                "frame": request.frame,
                "level": if matches!(request.kind, "passed" | "same_host_passed") {
                    "info"
                } else {
                    "error"
                },
                "target": "dev_feedback_replay",
                "message": request.message,
                "label": request.label,
            }),
        )?;
        write_readme(directory, &request.replay.scenario)?;
        Ok(())
    })
    .map(Some)
}

pub struct DevFeedbackCapture {
    replay: Option<ScenarioReplayV1>,
    label: String,
    recorded_inputs: Vec<ReplayInputV1>,
    dropped_inputs: usize,
    next_ordinal: u64,
    first_snapshot: SimulationSnapshot,
    before_snapshot: SimulationSnapshot,
    started: Instant,
}

impl DevFeedbackCapture {
    pub fn new(replay: ScenarioReplayV1, label: impl Into<String>, engine: &Engine) -> Self {
        Self::new_inner(Some(replay), label, engine)
    }

    /// Build a bounded state/input capture for a virtual-player route that
    /// does not have a committed replay fixture. The resulting artifact is a
    /// divergence trace rather than a replay that claims to be runnable.
    pub fn new_virtual_player(label: impl Into<String>, engine: &Engine) -> Self {
        Self::new_inner(None, label, engine)
    }

    /// Enable route diagnostics only when CI explicitly requests them and has
    /// supplied an artifact root. Ordinary local runs keep the allocation-free
    /// `VirtualPlayer::new` path.
    pub fn from_virtual_player_env(engine: &Engine, owner: i32) -> Option<Self> {
        let enabled = std::env::var("LC_CAPTURE_VIRTUAL_PLAYER").is_ok_and(|value| {
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        });
        (enabled && artifact_root().is_some())
            .then(|| Self::new_virtual_player(format!("virtual-player-owner-{owner}"), engine))
    }

    fn new_inner(
        replay: Option<ScenarioReplayV1>,
        label: impl Into<String>,
        engine: &Engine,
    ) -> Self {
        let next_ordinal = replay
            .as_ref()
            .iter()
            .flat_map(|replay| replay.joins.iter())
            .map(|join| join.ordinal)
            .chain(
                replay
                    .as_ref()
                    .iter()
                    .flat_map(|replay| replay.inputs.iter())
                    .map(|input| input.ordinal),
            )
            .max()
            .map_or(0, |ordinal| ordinal + 1);
        let first_snapshot = engine.snapshot();
        Self {
            replay,
            label: label.into(),
            recorded_inputs: Vec::new(),
            dropped_inputs: 0,
            next_ordinal,
            before_snapshot: first_snapshot.clone(),
            first_snapshot,
            started: Instant::now(),
        }
    }

    pub fn record_input(&mut self, frame: u64, owner: i32, command: u8, data: i32) {
        // Committed replay diagnostics must preserve their complete tape;
        // only the open-ended virtual-player trace uses the bound.
        if self.replay.is_some() || self.recorded_inputs.len() < MAX_RECORDED_INPUTS {
            self.recorded_inputs.push(ReplayInputV1::new(
                frame,
                self.next_ordinal,
                owner,
                command,
                data,
            ));
        } else {
            self.dropped_inputs += 1;
        }
        self.next_ordinal += 1;
    }

    pub fn recorded_inputs(&self) -> &[ReplayInputV1] {
        &self.recorded_inputs
    }

    pub fn before_tick(&mut self, engine: &Engine) {
        // Keep one recent state sample rather than cloning the full snapshot
        // on every open-ended route tick. Committed replay captures retain
        // their historical per-tick boundary, while the failing snapshot is
        // always captured at the failure boundary for either mode.
        let sample_due = self.replay.is_some()
            || engine.frame().saturating_sub(self.before_snapshot.frame)
                >= SNAPSHOT_SAMPLE_INTERVAL;
        if sample_due {
            self.before_snapshot = engine.snapshot();
        }
    }

    pub fn capture_timeout(
        &self,
        engine: &Engine,
        milestone: &str,
        max_ticks: u32,
        diagnostics: &str,
    ) -> Result<Option<PathBuf>, DevFeedbackError> {
        self.capture_failure_artifact(engine, milestone, Some(max_ticks), diagnostics)
    }

    pub fn capture_milestone(
        &self,
        engine: &Engine,
        milestone: &str,
        diagnostics: &str,
    ) -> Result<Option<PathBuf>, DevFeedbackError> {
        self.capture_failure_artifact(engine, milestone, None, diagnostics)
    }

    fn capture_failure_artifact(
        &self,
        engine: &Engine,
        milestone: &str,
        max_ticks: Option<u32>,
        diagnostics: &str,
    ) -> Result<Option<PathBuf>, DevFeedbackError> {
        let Some(root) = artifact_root() else {
            return Ok(None);
        };
        let failure_kind =
            max_ticks.map_or("virtual_player_milestone", |_| "virtual_player_timeout");
        let failing = engine.snapshot();
        let diff = snapshot_diff(&self.before_snapshot, &failing, DEFAULT_DIFF_LIMIT).unwrap_or(
            SnapshotDiff {
                entries: Vec::new(),
                truncated: false,
            },
        );
        let Some(replay) = self.replay.as_ref() else {
            return create_bundle(&root, &self.label, |directory| {
                write_json(
                    directory.join("divergence-trace.json"),
                    &json!({
                        "schema_version": REPLAY_SCHEMA_VERSION,
                        "kind": failure_kind,
                        "label": self.label,
                        "seed": engine.random_seed(),
                        "frame": failing.frame,
                        "milestone": milestone,
                        "max_ticks": max_ticks,
                        "diagnostics": diagnostics,
                        "inputs": self.recorded_inputs,
                        "dropped_inputs": self.dropped_inputs,
                    }),
                )?;
                write_json(directory.join("first.json"), &self.first_snapshot)?;
                write_json(directory.join("before.json"), &self.before_snapshot)?;
                write_json(directory.join("failing.json"), &failing)?;
                write_json(directory.join("diff.json"), &diff)?;
                write_json(
                    directory.join("failure.json"),
                    &json!({
                        "schema_version": REPLAY_SCHEMA_VERSION,
                        "kind": failure_kind,
                        "frame": failing.frame,
                        "milestone": milestone,
                        "max_ticks": max_ticks,
                        "diagnostics": diagnostics,
                    }),
                )?;
                write_logs(
                    directory,
                    &json!({
                        "frame": failing.frame,
                        "level": "error",
                        "target": "virtual_player",
                        "message": diagnostics,
                        "milestone": milestone,
                    }),
                )?;
                write_route_readme(directory)?;
                Ok(())
            })
            .map(Some);
        };
        let mut replay = replay.clone();
        replay.inputs.extend(self.recorded_inputs.clone());
        replay.stop_frame = engine.frame().max(replay.stop_frame);
        replay
            .inputs
            .sort_by_key(|input| (input.frame, input.ordinal));
        replay.validate()?;
        let metrics = ReplayMetricsFileV1 {
            schema_version: REPLAY_SCHEMA_VERSION,
            snapshot_hash_version: replay.snapshot_hash_version,
            runs: vec![ReplayRunMetricsV1 {
                schema_version: REPLAY_SCHEMA_VERSION,
                load_micros: 0,
                join_micros: 0,
                simulation_micros: self.started.elapsed().as_micros(),
                start_frame: self.first_snapshot.frame,
                stop_frame: failing.frame,
                frames: failing
                    .frame
                    .saturating_sub(self.first_snapshot.frame)
                    .saturating_add(1),
                observed_frames: failing
                    .frame
                    .saturating_sub(self.first_snapshot.frame)
                    .saturating_add(1) as usize,
                ticks: failing.frame.saturating_sub(self.first_snapshot.frame),
                final_snapshot_hash: snapshot_hash(&failing, replay.snapshot_hash_version),
                final_summary: ReplaySnapshotSummaryV1::from_snapshot(&failing),
            }],
        };
        create_bundle(&root, &self.label, |directory| {
            fs::write(directory.join("replay.json"), replay.canonical_json()?)
                .map_err(|error| io_error("write replay.json", error))?;
            write_json(directory.join("replay-metrics.json"), &metrics)?;
            write_json(directory.join("first.json"), &self.first_snapshot)?;
            write_json(directory.join("before.json"), &self.before_snapshot)?;
            write_json(directory.join("failing.json"), &failing)?;
            write_json(directory.join("diff.json"), &diff)?;
            write_json(
                directory.join("failure.json"),
                &json!({
                    "schema_version": REPLAY_SCHEMA_VERSION,
                    "kind": failure_kind,
                    "frame": failing.frame,
                    "milestone": milestone,
                    "max_ticks": max_ticks,
                    "diagnostics": diagnostics,
                }),
            )?;
            write_logs(
                directory,
                &json!({
                    "frame": failing.frame,
                    "level": "error",
                    "target": "virtual_player",
                    "message": diagnostics,
                    "milestone": milestone,
                }),
            )?;
            write_readme(directory, &replay.scenario)?;
            Ok(())
        })
        .map(Some)
    }
}

fn artifact_root() -> Option<PathBuf> {
    std::env::var_os("LC_TEST_ARTIFACT_DIR")
        .or_else(|| std::env::var_os("LC_DEV_CHECK_ARTIFACT_DIR"))
        .map(PathBuf::from)
}

fn keep_pass_artifacts() -> bool {
    std::env::var("LC_KEEP_PASS_ARTIFACTS").is_ok_and(|value| {
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

fn create_bundle(
    root: &Path,
    label: &str,
    write_contents: impl FnOnce(&Path) -> Result<(), DevFeedbackError>,
) -> Result<PathBuf, DevFeedbackError> {
    static NEXT_BUNDLE: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(root).map_err(|error| io_error("create artifact root", error))?;
    let sequence = NEXT_BUNDLE.fetch_add(1, Ordering::Relaxed);
    let name = format!(
        "{}-{}-{sequence}",
        sanitize_label(label),
        std::process::id()
    );
    let final_path = root.join(&name);
    let temporary_path = root.join(format!(".{name}.tmp"));
    fs::create_dir(&temporary_path)
        .map_err(|error| io_error("create temporary artifact bundle", error))?;
    if let Err(error) = write_contents(&temporary_path) {
        let _ = fs::remove_dir_all(&temporary_path);
        return Err(error);
    }
    fs::rename(&temporary_path, &final_path)
        .map_err(|error| io_error("publish artifact bundle", error))?;
    Ok(final_path)
}

fn sanitize_label(label: &str) -> String {
    let sanitized = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "replay".to_owned()
    } else {
        sanitized.chars().take(80).collect()
    }
}

fn write_json(path: PathBuf, value: &impl Serialize) -> Result<(), DevFeedbackError> {
    let mut file = fs::File::create(&path)
        .map_err(|error| io_error(&format!("create {}", path.display()), error))?;
    serde_json::to_writer_pretty(&mut file, value)
        .map_err(|error| DevFeedbackError::message(format!("write {}: {error}", path.display())))?;
    file.write_all(b"\n")
        .map_err(|error| io_error(&format!("finish {}", path.display()), error))
}

fn write_logs(directory: &Path, value: &Value) -> Result<(), DevFeedbackError> {
    let line = serde_json::to_string(value)
        .map_err(|error| DevFeedbackError::message(format!("serialize log line: {error}")))?;
    fs::write(directory.join("logs.ndjson"), format!("{line}\n"))
        .map_err(|error| io_error("write logs.ndjson", error))
}

fn write_readme(directory: &Path, scenario: &str) -> Result<(), DevFeedbackError> {
    let contents = format!(
        "Deterministic Clonk Rust replay artifact\n\nScenario: {scenario}\nReplay: replay.json\nMetrics: replay-metrics.json\n\nReproduce from the repository root:\n  LC_CONTENT_ROOT=/path/to/content LC_REPLAY_PATH=/path/to/artifact/replay.json cargo nextest run -p clonk-engine-integration-tests --test engine_it -- dev_feedback_replay::replay_artifact_from_env_repeats --ignored --exact\n"
    );
    fs::write(directory.join("README.txt"), contents)
        .map_err(|error| io_error("write README.txt", error))
}

fn write_route_readme(directory: &Path) -> Result<(), DevFeedbackError> {
    let contents = "Virtual player divergence trace\n\nTrace: divergence-trace.json\nState snapshots: first.json, before.json, failing.json\nState diff: diff.json\nFailure metadata: failure.json\nLogs: logs.ndjson\n\nThe input tape is bounded and records the controls observed before the failure.\nThis trace is diagnostic evidence for a route failure; it is not a standalone replay.\n";
    fs::write(directory.join("README.txt"), contents)
        .map_err(|error| io_error("write README.txt", error))
}

fn snapshot_hash(snapshot: &SimulationSnapshot, version: u32) -> String {
    // Debug-draw sidecars are frame-local presentation data. Keep them in
    // serialized snapshots and artifacts, but exclude them from deterministic
    // engine replay checkpoints so enabling diagnostics cannot change a run's
    // identity.
    let mut deterministic = snapshot.clone();
    deterministic.pathfinder_debug = Default::default();
    for object in &mut deterministic.objects {
        object.vertex_contacts.clear();
        object.solid_mask_override = None;
    }
    let mut value = crate::support::TestValueExt::test_value(serde_json::to_value(&deterministic));
    if version == SNAPSHOT_HASH_VERSION {
        exclude_surface8_render_cache_lineage(&mut value);
    }
    let canonical = canonicalize_json(value);
    let bytes = crate::support::TestValueExt::test_value(serde_json::to_vec(&canonical));
    let hash = bytes.into_iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    });
    format!("{hash:016x}")
}

fn exclude_surface8_render_cache_lineage(snapshot: &mut Value) {
    // These fields are a Rust frontend cache protocol. Native C4Landscape has
    // Surface8 and bounded Relights, but no persistent revision/token/history
    // state (C4Landscape.h:48-83); mask writes touch Surface8 only
    // (C4Landscape.cpp:846-849).
    let Some(pixels) = snapshot
        .pointer_mut("/landscape/pixels")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for field in ["revision", "render_token", "dirty_generations"] {
        pixels.remove(field);
    }
}

const fn legacy_snapshot_hash_version() -> u32 {
    LEGACY_SNAPSHOT_HASH_VERSION
}

#[cfg(any(not(feature = "engine-it-sharded"), feature = "engine-it-shard-1",))]
#[test]
fn snapshot_hash_ignores_serialized_debug_draw_sidecars() {
    let mut engine = Engine::with_seed(0);
    crate::support::TestValueExt::test_value(engine.register_definition(
        crate::support::TestValueExt::test_value(clonk_engine::Definition::from_script(
            "DBUG", "Debug", "",
        )),
    ));
    crate::support::TestValueExt::test_value(
        engine.spawn_object(clonk_engine::SpawnConfig::new("DBUG")),
    );
    let baseline = engine.snapshot();
    let mut with_debug = baseline.clone();
    with_debug.objects[0].vertex_contacts = vec![17];
    with_debug.objects[0].solid_mask_override =
        Some(clonk_engine::DefinitionTargetRect::new(1, 2, 3, 4, 5, 6));
    with_debug
        .pathfinder_debug
        .rays
        .push(clonk_engine::PathfinderDebugRay::default());

    let encoded = crate::support::TestValueExt::test_value(serde_json::to_string(&with_debug));
    let restored: SimulationSnapshot =
        crate::support::TestValueExt::test_value(serde_json::from_str(&encoded));
    assert_eq!(restored.objects[0].vertex_contacts, vec![17]);
    assert_eq!(
        restored.objects[0].solid_mask_override,
        with_debug.objects[0].solid_mask_override
    );
    assert_eq!(restored.pathfinder_debug, with_debug.pathfinder_debug);
    assert_eq!(
        snapshot_hash(&baseline, SNAPSHOT_HASH_VERSION),
        snapshot_hash(&restored, SNAPSHOT_HASH_VERSION)
    );
}

#[cfg(any(not(feature = "engine-it-sharded"), feature = "engine-it-shard-1",))]
#[test]
fn snapshot_hash_ignores_surface8_render_cache_lineage() {
    // A C4SolidMask put/remove cycle writes MCVehic and restores the saved
    // Surface8 byte (C4SolidMask.cpp:323-360; C4Landscape.cpp:846-849). C++
    // carries no revision/token/dirty-generation history for those writes;
    // equal final landscape bytes are therefore equal replay state.
    let mut landscape =
        crate::support::TestValueExt::test_value(clonk_engine::Landscape::new(2, vec![2; 2]));
    landscape.set_pixel_grid(clonk_engine::landscape::PixelGrid::new(
        2,
        2,
        vec![1; 4],
        vec![0; 128],
        vec![None; 128],
        vec![None; 128],
    ));
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(landscape);
    let baseline = engine.snapshot();
    let mut after_mask_cycle = baseline.clone();
    let cycled = crate::support::TestValueExt::test_value(after_mask_cycle.landscape.as_mut());
    cycled.grid_write_mask_byte(0, 0, 2);
    cycled.grid_write_mask_byte(0, 0, 1);

    assert_eq!(
        baseline
            .landscape
            .as_ref()
            .and_then(clonk_engine::Landscape::pixel_grid)
            .expect("baseline pixel grid")
            .bytes(),
        cycled.pixel_grid().expect("cycled pixel grid").bytes(),
        "the mask cycle restores authoritative Surface8 byte-for-byte"
    );
    assert_ne!(
        snapshot_hash(&baseline, LEGACY_SNAPSHOT_HASH_VERSION),
        snapshot_hash(&after_mask_cycle, LEGACY_SNAPSHOT_HASH_VERSION),
        "legacy hashes include the Rust-only cache lineage"
    );
    assert_eq!(
        snapshot_hash(&baseline, SNAPSHOT_HASH_VERSION),
        snapshot_hash(&after_mask_cycle, SNAPSHOT_HASH_VERSION)
    );

    let mut changed = baseline.clone();
    crate::support::TestValueExt::test_value(changed.landscape.as_mut())
        .grid_write_mask_byte(0, 0, 2);
    assert_ne!(
        snapshot_hash(&baseline, SNAPSHOT_HASH_VERSION),
        snapshot_hash(&changed, SNAPSHOT_HASH_VERSION),
        "an authoritative Surface8 byte change must remain replay-visible"
    );
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(entries) => {
            let sorted = entries
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            let mut output = Map::new();
            for (key, value) in sorted {
                output.insert(key, value);
            }
            Value::Object(output)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        value => value,
    }
}

fn io_error(action: &str, error: std::io::Error) -> DevFeedbackError {
    DevFeedbackError::message(format!("{action}: {error}"))
}
