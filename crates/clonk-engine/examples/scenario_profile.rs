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
//! cargo run --release -p clonk-engine --example scenario_profile -- \
//!     "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MeltMe.c4s" 600
//! ```
//!
//! Set `LC_PROFILE_MODE=tick` (default) to time `Engine::tick` (advance plus the
//! full `SimulationSnapshot` the app consumes), or `LC_PROFILE_MODE=advance` to
//! time only `Engine::tick_without_snapshot`. The difference is the per-frame
//! cost of snapshot construction alone.

use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use clonk_engine::{Engine, JoinPlayerConfig, Scenario, ScenarioError};
use clonk_resources::{Group, MaterialLibrary};

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

fn load(relative_path: &Path, seed: u64) -> Engine {
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
    engine
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
    let snapshot_each_frame = env::var("LC_PROFILE_MODE").as_deref() != Ok("advance");

    let load_started = Instant::now();
    let mut engine = load(Path::new(&relative), seed);
    let load_elapsed = load_started.elapsed();

    // Two players so team scenarios and cross-player code paths stay live.
    let joined: Vec<i32> = ["Profiler A", "Profiler B"]
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            join(&mut engine, name, None)
                .or_else(|| join(&mut engine, name, Some(index as i32 + 1)))
        })
        .collect();

    let mut frame_times = Vec::with_capacity(frames);
    let run_started = Instant::now();
    for _ in 0..frames {
        let frame_started = Instant::now();
        if snapshot_each_frame {
            engine.tick().expect("tick succeeds");
        } else {
            engine
                .tick_without_snapshot()
                .expect("advance-only tick succeeds");
        }
        frame_times.push(frame_started.elapsed());
    }
    let run_elapsed = run_started.elapsed();

    let mut sorted = frame_times.clone();
    sorted.sort_unstable();
    let total: Duration = frame_times.iter().sum();
    let mean = total / frames.max(1) as u32;
    let over_budget = frame_times
        .iter()
        .filter(|elapsed| **elapsed > Duration::from_micros(27_777))
        .count();

    println!("scenario         {relative}");
    println!(
        "mode             {}",
        if snapshot_each_frame {
            "tick (advance + full SimulationSnapshot)"
        } else {
            "advance only (no snapshot)"
        }
    );
    println!("players joined   {joined:?}");
    println!("objects          {}", engine.snapshot().objects.len());
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
