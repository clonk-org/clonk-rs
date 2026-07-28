//! Combat-load profiler: boots MeltMe with real crew and drives the
//! per-frame fire control for N players, then reports frame-time percentiles.
//!
//! Measurement tool only: it never asserts simulation state.
//!
//! ```text
//! LC_PLAYERS=3 cargo run --release -p clonk-engine --example combat_profile -- \
//!     "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MeltMe.c4s" 600 424242
//! ```

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clonk_engine::player_file::CrewInfo;
use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use clonk_engine::{
    CommandKind, ControlCommand, Engine, JoinPlayerConfig, LegacyCString, Scenario, ScenarioError,
    ScriptControlData, ScriptControlPolicy, ScriptStrictness, SCRIPT_SCOPE_CONSOLE,
};
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

fn crew_entry(name: &str) -> CrewInfo {
    CrewInfo {
        id: "SF5B".to_owned(),
        name: name.to_owned(),
        rank_name: "Clonk".to_owned(),
        ..Default::default()
    }
}

fn join(engine: &mut Engine, name: &str, color: u32, with_crew: bool) -> Option<i32> {
    let config = |team: Option<i32>| JoinPlayerConfig {
        name: name.to_owned(),
        player_info_id: 0,
        score: 0,
        rounds: 0,
        rounds_won: 0,
        rounds_lost: 0,
        total_playing_time: 0,
        team,
        color_dw: color,
        pref_color: 0,
        pref_position: 0,
        crew: if with_crew {
            vec![crew_entry(name)]
        } else {
            Vec::new()
        },
        control_style: false,
        auto_context_menu: false,
        startup_player_count: 1,
    };
    engine
        .join_player(config(None))
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

fn object_census(engine: &Engine) -> BTreeMap<String, usize> {
    let mut census = BTreeMap::new();
    for object in engine.snapshot().objects.iter() {
        *census.entry(object.definition_id.to_string()).or_insert(0) += 1;
    }
    census
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
    let players: usize = env::var("LC_PLAYERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let fire = env::var("LC_FIRE").as_deref() != Ok("0");
    let snapshot_each_frame = env::var("LC_PROFILE_MODE").as_deref() != Ok("advance");
    let warmup: usize = env::var("LC_WARMUP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(36);

    let mut engine = load(Path::new(&relative), seed);

    let colors = [
        0xff_00_00u32,
        0x00_ff_00,
        0x00_00_ff,
        0xff_ff_00,
        0xff_00_ff,
    ];
    let joined: Vec<i32> = (0..players)
        .filter_map(|index| {
            join(
                &mut engine,
                &format!("Profiler {index}"),
                colors[index % colors.len()],
                true,
            )
        })
        .collect();

    println!("players joined   {joined:?}");
    println!("objects at join  {}", engine.snapshot().objects.len());

    for _ in 0..warmup {
        engine.tick().expect("warmup tick succeeds");
    }
    // Optional: remove every carried definition named in LC_STRIP so the next
    // item becomes Contents(0) and the fire control drives that weapon instead.
    if let Ok(strip) = env::var("LC_STRIP") {
        for id in strip.split(',').filter(|id| !id.is_empty()) {
            let script = format!("var o; while (o = FindObject({id})) RemoveObject(o);");
            let control = ScriptControlData {
                target_object: SCRIPT_SCOPE_CONSOLE,
                strictness: ScriptStrictness::Strict2,
                script: LegacyCString::from_bytes(script.into_bytes()).expect("no interior NUL"),
                by_client: 0,
            };
            let policy = ScriptControlPolicy {
                is_replay: false,
                console_active: true,
                allow_scripting_in_replays: false,
            };
            let result = engine.execute_script_control(&control, policy);
            println!("strip {id}         {result:?}");
        }
        println!("census stripped  {:?}", object_census(&engine));
    }
    println!("objects warm     {}", engine.snapshot().objects.len());
    println!("census warm      {:?}", object_census(&engine));

    let boom_every: usize = env::var("LC_BOOM")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    // Steady-state flame population, independent of the weapon's rate of fire.
    let flame_target: usize = env::var("LC_FLAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let flame_id = env::var("LC_FLAME_ID").unwrap_or_else(|_| "SH5B".to_owned());
    let boom_policy = ScriptControlPolicy {
        is_replay: false,
        console_active: true,
        allow_scripting_in_replays: false,
    };
    let mut frame_times = Vec::with_capacity(frames);
    let mut control_errors = 0usize;
    let mut controls_handled = 0usize;
    let run_started = Instant::now();
    for frame in 0..frames {
        let frame_started = Instant::now();
        if fire {
            // Hold the fire control down: press on even frames, release on odd,
            // which is what a player holding the throw key produces.
            let kind = if frame % 2 == 0 {
                CommandKind::Press
            } else {
                CommandKind::Release
            };
            for owner in &joined {
                match engine.handle_control_command(*owner, ControlCommand::Throw, kind) {
                    Ok(true) => controls_handled += 1,
                    Ok(false) => {}
                    Err(_) => control_errors += 1,
                }
            }
        }
        // Hold a steady-state flame population. The weapon's own rate of fire
        // plateaus near 40 live flames, which is far below a real firefight;
        // topping the census back up each frame lets the flame cost be swept
        // independently of how fast the shipped weapon can emit.
        if flame_target > 0 {
            let live = engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == flame_id)
                .count();
            for index in live..flame_target {
                let script = format!(
                    "CreateObject({flame_id}, {}, {}, -1);",
                    120 + (index % 48) * 58,
                    140 + (index % 7) * 30,
                );
                let control = ScriptControlData {
                    target_object: SCRIPT_SCOPE_CONSOLE,
                    strictness: ScriptStrictness::Strict2,
                    script: LegacyCString::from_bytes(script.into_bytes()).expect("no NUL"),
                    by_client: 0,
                };
                if engine
                    .execute_script_control(&control, boom_policy)
                    .is_err()
                {
                    control_errors += 1;
                }
            }
        }
        if boom_every > 0 && frame % boom_every == 0 {
            let script = format!(
                "Explode(50, CreateObject(FXP1, {}, {}, -1));",
                200 + (frame % 40) * 60,
                200
            );
            let control = ScriptControlData {
                target_object: SCRIPT_SCOPE_CONSOLE,
                strictness: ScriptStrictness::Strict2,
                script: LegacyCString::from_bytes(script.into_bytes()).expect("no NUL"),
                by_client: 0,
            };
            if engine
                .execute_script_control(&control, boom_policy)
                .is_err()
            {
                control_errors += 1;
            }
        }
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
    println!("fire             {fire}");
    println!("seed             {seed}");
    println!("controls handled {controls_handled} (errors {control_errors})");
    println!("objects end      {}", engine.snapshot().objects.len());
    println!("census end       {:?}", object_census(&engine));
    println!("frames           {frames}");
    println!("wall             {run_elapsed:?}");
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
    let spikes: Vec<(usize, Duration)> = frame_times
        .iter()
        .enumerate()
        .filter(|(_, elapsed)| **elapsed > Duration::from_millis(10))
        .map(|(index, elapsed)| (index, *elapsed))
        .collect();
    println!("spikes >10ms     {spikes:?}");
}
