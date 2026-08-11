//! Headless simulation profiler for an installed scenario.
//!
//! Boots real content the same way the app does (installed material library and
//! `planet/System.c4g` before scenario definition/script loading) and advances a
//! fixed number of frames, reporting where per-frame wall time goes.
//!
//! This is a measurement tool, not a parity gate: it never asserts simulation
//! state, so it cannot mask a determinism regression.
//!
//! ```text
//! LC_PROFILE_MODE=split \
//!   cargo run --release --offline --locked -p clonk-engine \
//!     --example scenario_profile -- \
//!     "EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s" 600 424242 1000
//! ```
//!
//! Set `LC_PROFILE_MODE=tick` (default) to time `Engine::tick` (advance plus the
//! full `SimulationSnapshot` the app consumes), or `LC_PROFILE_MODE=advance` to
//! time only `Engine::tick_without_snapshot`. `LC_PROFILE_MODE=split` times the
//! advance and the following `Engine::snapshot` projection independently in
//! the same run. An optional fourth positional argument is an exact ST5B
//! target. The checked-in Arso-Morf save starts with 20 Stippels; the profiler
//! creates the remainder with the real loaded ST5B definition and its normal
//! `Initialize` callback before any measured frame.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use clonk_engine::{Engine, JoinPlayerConfig, Scenario, ScenarioError, SpawnConfig, Vector2};
use clonk_resources::{Group, MaterialLibrary};
use clonk_script::Value;

const STIPPEL_ID: &str = "ST5B";

#[derive(Clone, Copy)]
enum ProfileMode {
    Tick,
    Advance,
    Split,
}

impl ProfileMode {
    fn from_environment() -> Self {
        match env::var("LC_PROFILE_MODE").as_deref() {
            Ok("advance") => Self::Advance,
            Ok("split") => Self::Split,
            Ok("tick") | Err(_) => Self::Tick,
            Ok(other) => panic!(
                "unsupported LC_PROFILE_MODE `{other}`; expected `tick`, `advance`, or `split`"
            ),
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Tick => "tick (advance + full SimulationSnapshot)",
            Self::Advance => "advance only (no snapshot)",
            Self::Split => "split (advance + separately timed snapshot projection)",
        }
    }
}

#[derive(Clone, Copy)]
struct ObjectCensus {
    total: usize,
    stippels: usize,
}

impl ObjectCensus {
    const fn other(self) -> usize {
        self.total - self.stippels
    }
}

struct ContentResolver {
    roots: Vec<PathBuf>,
}

impl LegacyDefinitionResolver for ContentResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let relative = identifier.replace('\\', "/");
        self.roots
            .iter()
            .map(|root| root.join(&relative))
            .find(|candidate| candidate.exists())
            .map(Group::open)
            .transpose()
            .map_err(ScenarioError::Resources)?
            .map(|group| vec![group])
            .ok_or_else(|| ScenarioError::LegacyDefinitionNotFound { path: relative })
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn content_root() -> PathBuf {
    env::var_os("LC_CONTENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root().join("content"))
}

fn load(relative_path: &Path, seed: u64) -> (Engine, PathBuf, PathBuf) {
    let content = content_root();
    let content_install = content.parent().expect("content root has a parent");
    let bundled = repository_root();
    let scenario_path = [bundled.join(relative_path), content.join(relative_path)]
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| bundled.join(relative_path));
    let scenario = Scenario::load_from_path_with_seed(
        &scenario_path,
        &ContentResolver {
            roots: vec![bundled, content.clone()],
        },
        seed,
    )
    .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", scenario_path.display()));

    let material_group = Group::open(content.join("Material.c4g")).expect("Material.c4g opens");
    let materials = MaterialLibrary::from_group(&material_group).expect("Material.c4g loads");
    let system_group =
        Group::open(content_install.join("planet/System.c4g")).expect("System opens");
    let system_scripts = load_system_scripts(&system_group).expect("System scripts load");
    let standard_names = system_group
        .read_file("Names.txt")
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    let mut engine = Engine::with_seed(seed);
    engine.configure_materials_from_library(&materials);
    engine.install_global_scripts(&system_scripts);
    engine.set_standard_names(standard_names);
    scenario
        .apply(&mut engine)
        .unwrap_or_else(|error| panic!("scenario applies: {error}"));
    (engine, scenario_path, content)
}

fn object_census(engine: &Engine) -> ObjectCensus {
    let snapshot = engine.snapshot();
    let stippels = snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == STIPPEL_ID)
        .count();
    ObjectCensus {
        total: snapshot.objects.len(),
        stippels,
    }
}

fn stippel_spawn_position(anchor: Vector2, occurrence: usize) -> Vector2 {
    // Fresh FullCon construction moves ST5B's center eight pixels upward.
    // Compensate so the timed center starts on the checked-in ground band.
    const FRESH_CENTER_Y_ADJUSTMENT: i32 = 8;
    let offset = i32::try_from(occurrence).unwrap_or_default() - 24;
    let offset = if offset >= 0 { offset + 1 } else { offset };
    Vector2::new(anchor.x + offset, anchor.y + FRESH_CENTER_Y_ADJUSTMENT)
}

fn stippel_spawn_config(position: Vector2) -> SpawnConfig {
    // Keep the real LifeCycle cost while giving the dense fresh population
    // enough initial grace to reach measurement start. LifeCycle may reset the
    // value and naturally remove stuck ST5Bs during the profile window.
    SpawnConfig::new(STIPPEL_ID)
        .with_position(position)
        .with_local_vars(HashMap::from([(
            "stuckTime".to_owned(),
            Value::Int(-1_000),
        )]))
}

fn set_initial_stippel_stuck_grace(engine: &mut Engine) {
    let mut state = engine.capture_state();
    for object in &mut state.objects {
        if object.snapshot.definition_id == STIPPEL_ID {
            object
                .snapshot
                .local_vars
                .insert("stuckTime".to_owned(), Value::Int(-1_000));
        }
    }
    engine
        .restore_state(&state)
        .expect("fixed ST5B lifecycle fixture restores");
}

fn populate_stippels(engine: &mut Engine, target: usize) {
    let snapshot = engine.snapshot();
    let anchors = snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == STIPPEL_ID)
        .map(|object| object.position)
        .collect::<Vec<Vector2>>();
    assert!(
        !anchors.is_empty(),
        "ST5B target requires at least one real loaded Stippel placement"
    );
    assert!(
        anchors.len() <= target,
        "loaded scenario already has {} ST5B objects, above requested target {target}",
        anchors.len()
    );

    for index in anchors.len()..target {
        let anchor_index = index % anchors.len();
        let occurrence = (index - anchors.len()) / anchors.len();
        let position = stippel_spawn_position(anchors[anchor_index], occurrence);
        engine
            .spawn_object(stippel_spawn_config(position))
            .unwrap_or_else(|error| panic!("fresh real-content ST5B #{index} spawns: {error}"));
    }
    set_initial_stippel_stuck_grace(engine);

    let census = object_census(engine);
    assert_eq!(
        census.stippels, target,
        "ST5B prepopulation must reach the exact requested census before timing"
    );
}

fn join(engine: &mut Engine, name: &str, team: Option<i32>) -> Option<i32> {
    engine
        .join_player(JoinPlayerConfig {
            name: name.to_owned(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .ok()?
        .initialized()
        .map(|player| player.number)
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn print_timing_row(label: &str, samples: &[Duration]) {
    if samples.is_empty() {
        return;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let total: Duration = samples.iter().sum();
    let mean = total / samples.len().max(1) as u32;
    println!(
        "{label:<18} {:>10.3?} {:>10.3?} {:>10.3?} {:>10.3?} {:>10.3?}",
        mean,
        percentile(&sorted, 0.50),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or_default(),
    );
}

fn main() {
    let mut args = env::args().skip(1);
    let relative = args
        .next()
        .unwrap_or_else(|| "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MeltMe.c4s".to_owned());
    let frames: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);
    let seed: u64 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(424_242);
    let stippel_target = args.next().map(|value| {
        value
            .parse::<usize>()
            .unwrap_or_else(|error| panic!("ST5B target `{value}` is not a count: {error}"))
    });
    assert!(
        args.next().is_none(),
        "usage: scenario_profile [scenario] [frames] [seed] [ST5B target]"
    );
    let mode = ProfileMode::from_environment();

    let load_started = Instant::now();
    let (mut engine, scenario_path, content) = load(Path::new(&relative), seed);
    let load_elapsed = load_started.elapsed();
    let loaded_census = object_census(&engine);

    // Join before target population so InitializePlayer and crew/content
    // creation finish before the exact census and measured frames. Two players
    // keep team scenarios and cross-player code paths live.
    let joined: Vec<i32> = ["Profiler A", "Profiler B"]
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            join(&mut engine, name, None)
                .or_else(|| join(&mut engine, name, Some(index as i32 + 1)))
        })
        .collect();

    if let Some(target) = stippel_target {
        populate_stippels(&mut engine, target);
    }
    let prepared_census = object_census(&engine);

    let mut frame_times = Vec::with_capacity(frames);
    let mut advance_times = Vec::with_capacity(frames);
    let mut snapshot_times = Vec::with_capacity(frames);
    let run_started = Instant::now();
    for _ in 0..frames {
        let frame_started = Instant::now();
        match mode {
            ProfileMode::Tick => {
                std::hint::black_box(engine.tick().expect("tick succeeds"));
            }
            ProfileMode::Advance => {
                let started = Instant::now();
                engine
                    .tick_without_snapshot()
                    .expect("advance-only tick succeeds");
                advance_times.push(started.elapsed());
            }
            ProfileMode::Split => {
                let started = Instant::now();
                engine
                    .tick_without_snapshot()
                    .expect("split-profile advance succeeds");
                advance_times.push(started.elapsed());

                let started = Instant::now();
                std::hint::black_box(engine.snapshot());
                snapshot_times.push(started.elapsed());
            }
        }
        frame_times.push(frame_started.elapsed());
    }
    let run_elapsed = run_started.elapsed();
    let final_census = object_census(&engine);

    let mut sorted = frame_times.clone();
    sorted.sort_unstable();
    let total: Duration = frame_times.iter().sum();
    let mean = total / frames.max(1) as u32;
    let over_budget = frame_times
        .iter()
        .filter(|elapsed| **elapsed > Duration::from_micros(27_777))
        .count();

    println!("scenario         {relative}");
    println!("scenario path    {}", scenario_path.display());
    println!("content root     {}", content.display());
    println!("seed             {seed}");
    println!("mode             {}", mode.description());
    println!("players joined   {joined:?}");
    println!(
        "loaded census    total={} ST5B={} other={}",
        loaded_census.total,
        loaded_census.stippels,
        loaded_census.other(),
    );
    println!(
        "timed census     total={} ST5B={} other={}",
        prepared_census.total,
        prepared_census.stippels,
        prepared_census.other(),
    );
    println!(
        "final census     total={} ST5B={} other={}",
        final_census.total,
        final_census.stippels,
        final_census.other(),
    );
    println!("load             {load_elapsed:?}");
    println!("frames           {frames}");
    println!("wall             {run_elapsed:?}");
    println!(
        "effective fps    {:.1}",
        frames as f64 / run_elapsed.as_secs_f64()
    );
    println!("mean/frame       {mean:?}");
    println!("p50              {:?}", percentile(&sorted, 0.50));
    println!("p95              {:?}", percentile(&sorted, 0.95));
    println!("p99              {:?}", percentile(&sorted, 0.99));
    println!(
        "max              {:?}",
        sorted.last().copied().unwrap_or_default()
    );
    println!(
        "frames > 27.7ms  {over_budget} ({:.1}%)",
        over_budget as f64 / frames as f64 * 100.0
    );
    if !advance_times.is_empty() || !snapshot_times.is_empty() {
        println!("\nphase                    mean        p50        p95        p99        max");
        print_timing_row("advance", &advance_times);
        print_timing_row("snapshot projection", &snapshot_times);
        print_timing_row("combined frame", &frame_times);
    }

    // Degradation trend. Aggregates cannot distinguish a uniformly slow run
    // from one that starts fast and decays, which is what a leak looks like.
    let segments = 10;
    let per_segment = frames / segments;
    if per_segment > 0 {
        println!("\nsegment    frames        mean      p95");
        for segment in 0..segments {
            let window = &frame_times[segment * per_segment..(segment + 1) * per_segment];
            let mut sorted_window = window.to_vec();
            sorted_window.sort_unstable();
            let window_mean: Duration =
                window.iter().sum::<Duration>() / window.len().max(1) as u32;
            println!(
                "{:>7}  {:>6}  {:>10.3?}  {:>7.3?}",
                segment,
                segment * per_segment,
                window_mean,
                percentile(&sorted_window, 0.95)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn added_stippels_use_unique_small_offsets_on_each_anchor_band() {
        let anchor = Vector2::new(1_000, 600);
        let positions = (0..49)
            .map(|occurrence| stippel_spawn_position(anchor, occurrence))
            .map(|position| (position.x, position.y))
            .collect::<HashSet<_>>();

        assert_eq!(positions.len(), 49);
        assert!(positions
            .iter()
            .all(|(x, y)| (976..=1_025).contains(x) && *y == 608));
        assert!(!positions.contains(&(anchor.x, anchor.y)));
    }

    #[test]
    fn added_stippels_keep_the_real_lifecycle_with_initial_stuck_grace() {
        let config = stippel_spawn_config(Vector2::new(1_000, 600));

        assert_eq!(
            config.local_vars.get("stuckTime"),
            Some(&Value::Int(-1_000))
        );
    }
}
