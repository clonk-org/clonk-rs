mod audit;
mod dependency_licenses;

use anyhow::{anyhow, bail, Context, Result};
use clonk_engine::fixtures::SNAPSHOT_SCENARIOS;
use clonk_engine::{Playback, Recording};
use std::env;
use std::fs;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;
use xtask::dev_check;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

// The authorized classic packs ship inside the content submodule, which is the
// engine's data root (`General.ExePath`). Packing them beside it instead would
// leave them outside every definition and scenario search path.
const CONTENT_GAME_PACKS: [&str; 4] = [
    "EkeReloaded.c4d",
    "EkeReloaded.c4f",
    "ClonkMars.c4d",
    "ClonkMars.c4f",
];

const MACOS_APP_NAME: &str = "Clonk Rust.app";
const MACOS_ICON_STEM: &str = "ClonkRust";
const MACOS_ICON_SOURCE: &str = "planet/Graphics.c4g/Logo.png";
/// Everything the staged payload keeps outside `bin/`, relocated into
/// `Contents/Resources` so the bundle stays self-contained.
const MACOS_BUNDLED_RESOURCES: [&str; 8] = [
    "planet",
    "content",
    "licenses",
    "COPYING",
    "TRADEMARK",
    "README.md",
    "credits.txt",
    "THIRD_PARTY_GAME_CONTENT.md",
];

fn main() -> Result<()> {
    clonk_logging::init();

    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some("package") => {
            if let Some(arg) = args.next() {
                bail!("unexpected argument `{}` for `package` command", arg);
            }
            package()
        }
        Some("engine-snapshots") => {
            let tail: Vec<String> = args.collect();
            engine_snapshots_command(&tail)
        }
        Some("dev-check") => {
            let tail: Vec<String> = args.collect();
            dev_check::command(&tail)
        }
        Some("parity") => {
            let tail: Vec<String> = args.collect();
            parity_command(&tail)
        }
        Some("scenario-sweep") => {
            let tail: Vec<String> = args.collect();
            scenario_sweep_command(&tail)
        }
        Some("scenario-errors") => {
            let tail: Vec<String> = args.collect();
            scenario_errors_command(&tail)
        }
        Some("scenario-audit") => {
            let tail: Vec<String> = args.collect();
            audit::scenario_audit_command(&tail)
        }
        Some(cmd) => bail!("unknown command `{}` (try `cargo xtask --help`)", cmd),
    }
}

fn print_usage() {
    tracing::info!(
        "Usage:\n  cargo xtask package                 Build the Rust port and bundle a distributable archive.\n  cargo xtask dev-check [options]     Run the change-aware sub-60-second developer feedback loop.\n  cargo xtask engine-snapshots record Regenerate engine snapshot baselines.\n  cargo xtask engine-snapshots verify Check Rust engine output against recorded baselines.\n  cargo xtask parity record|verify    C++↔Rust differential parity harness (see parity/README.md).\n  cargo xtask scenario-sweep [filter] [--verbose]  Load+apply every real scenario in content/; the scenario-load parity scoreboard.\n  cargo xtask scenario-audit [filter] [--verbose]  Audit applied-world fidelity (landscape materials, objects, init placements)."
    );
}

// ── scenario-sweep: the scenario-load parity scoreboard ─────────────────────
//
// Loads and applies every `*.c4s` under `content/` with real definitions and
// materials, then reports failures grouped by error class. C++ load parity is
// reached when every scenario the C++ engine can start also loads+applies
// here.

pub(crate) struct SweepResolver {
    pub(crate) roots: Vec<PathBuf>,
}

impl clonk_engine::scenario::LegacyDefinitionResolver for SweepResolver {
    fn resolve_definition_groups(
        &self,
        _scenario: &clonk_resources::Group,
        identifier: &str,
    ) -> std::result::Result<Vec<clonk_resources::Group>, clonk_engine::ScenarioError> {
        let mut groups: Vec<clonk_resources::Group> = Vec::new();
        let normalized = identifier.replace('\\', "/");
        let path = Path::new(&normalized);

        // DefinitionFilenames are opened from executable-data roots. Folder
        // and scenario-local definitions are appended by clonk-engine's
        // separate InitDefs passes (C4Game.cpp:81-103, 184-213).
        for root in &self.roots {
            let candidate = root.join(path);
            if !candidate.exists() {
                continue;
            }
            let group = clonk_resources::Group::open(&candidate)?;
            if groups
                .iter()
                .all(|existing| existing.root() != group.root())
            {
                groups.push(group);
            }
        }

        if groups.is_empty() {
            return Err(clonk_engine::ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            });
        }
        Ok(groups)
    }

    fn resolve_material_groups(
        &self,
        scenario: &clonk_resources::Group,
    ) -> std::result::Result<Vec<clonk_resources::Group>, clonk_engine::ScenarioError> {
        let mut groups = Vec::new();
        let mut candidates = scenario
            .root()
            .ancestors()
            .map(|root| root.join("Material.c4g"))
            .collect::<Vec<_>>();
        candidates.extend(self.roots.iter().map(|root| root.join("Material.c4g")));
        for candidate in candidates {
            let Ok(group) = clonk_resources::Group::open(&candidate) else {
                continue;
            };
            if groups
                .iter()
                .all(|existing: &clonk_resources::Group| existing.root() != group.root())
            {
                groups.push(group);
            }
        }
        Ok(groups)
    }
}

/// Collapses error messages into bucket keys: backtick-quoted names and
/// numbers vary per scenario, the remaining text is the failure class.
fn classify_error(message: &str) -> String {
    let mut class = String::with_capacity(message.len());
    let mut in_quote = false;
    for ch in message.chars() {
        match ch {
            '`' => {
                in_quote = !in_quote;
                if !in_quote {
                    class.push('*');
                }
            }
            _ if in_quote => {}
            '0'..='9' => {
                if !class.ends_with('#') {
                    class.push('#');
                }
            }
            _ => class.push(ch),
        }
    }
    class
}

fn scenario_sweep_command(args: &[String]) -> Result<()> {
    let mut filter: Option<String> = None;
    let mut verbose = false;
    for arg in args {
        match arg.as_str() {
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => {
                tracing::info!(
                    "Usage: cargo xtask scenario-sweep [filter] [--verbose]\n  Loads + applies every content/**/*.c4s; reports failures by class."
                );
                return Ok(());
            }
            other => filter = Some(other.to_string()),
        }
    }

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("cannot locate repo root from xtask manifest dir"))?;
    let content_root = repo_root.join("content");
    if !content_root.exists() {
        bail!("content directory not found at {}", content_root.display());
    }

    let material_library = clonk_resources::MaterialLibrary::from_group(
        &clonk_resources::Group::open(content_root.join("Material.c4g"))
            .context("opening content/Material.c4g")?,
    )
    .map_err(|error| anyhow!("loading material library: {error}"))?;

    // System.c4g global scripts (Game.ScriptEngine in C++).
    let system_scripts = clonk_resources::Group::open(repo_root.join("planet/System.c4g"))
        .ok()
        .and_then(|group| clonk_engine::scenario::load_system_scripts(&group).ok())
        .unwrap_or_default();
    if system_scripts.is_empty() {
        tracing::warn!("no System.c4g scripts found; global functions unavailable");
    }

    let mut scenario_paths: Vec<PathBuf> = WalkDir::new(&content_root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case("c4s"))
                .unwrap_or(false)
        })
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| {
            filter
                .as_ref()
                .map(|needle| path.to_string_lossy().contains(needle.as_str()))
                .unwrap_or(true)
        })
        .collect();
    scenario_paths.sort();

    let total = scenario_paths.len();
    let mut loaded = 0usize;
    let mut applied = 0usize;
    let mut load_failures: Vec<(String, String)> = Vec::new();
    let mut apply_failures: Vec<(String, String)> = Vec::new();

    enum SweepOutcome {
        Applied,
        LoadFailed(String),
        ApplyFailed(String),
    }

    for path in &scenario_paths {
        let label = path
            .strip_prefix(&content_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        // Explicit DefinitionFilenames resolve from executable-data roots;
        // folder-local packs are a distinct engine loading phase.
        let roots = vec![content_root.clone(), repo_root.clone()];

        // Each scenario runs on a watchdog thread: a script whose loop
        // depends on engine state we model differently can hang forever —
        // a timeout makes that a reportable failure class instead.
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        let worker_library = material_library.clone();
        let worker_system_scripts = system_scripts.clone();
        std::thread::spawn(move || {
            let resolver = SweepResolver { roots };
            let outcome = match clonk_engine::Scenario::load_from_path_with(&worker_path, &resolver)
            {
                Ok(scenario) => {
                    let mut engine = clonk_engine::Engine::new();
                    engine.configure_materials_from_library(&worker_library);
                    engine.install_global_scripts(&worker_system_scripts);
                    match scenario.apply(&mut engine) {
                        Ok(_) => SweepOutcome::Applied,
                        Err(error) => SweepOutcome::ApplyFailed(error.to_string()),
                    }
                }
                Err(error) => SweepOutcome::LoadFailed(error.to_string()),
            };
            let _ = sender.send(outcome);
        });

        match receiver.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(SweepOutcome::Applied) => {
                loaded += 1;
                applied += 1;
                if verbose {
                    tracing::info!("OK        {label}");
                }
            }
            Ok(SweepOutcome::ApplyFailed(error)) => {
                loaded += 1;
                if verbose {
                    tracing::info!("APPLY FAIL {label}: {error}");
                }
                apply_failures.push((label, error));
            }
            Ok(SweepOutcome::LoadFailed(error)) => {
                if verbose {
                    tracing::info!("LOAD FAIL {label}: {error}");
                }
                load_failures.push((label, error));
            }
            Err(_) => {
                if verbose {
                    tracing::info!("HUNG      {label}");
                }
                apply_failures.push((label, "HUNG: apply did not finish within 120s".to_string()));
                // The worker thread is abandoned; the sweep process exits at
                // the end regardless.
            }
        }
    }

    let mut report = String::new();
    report.push_str(&format!(
        "\nscenario sweep: {total} scenarios — {loaded} load ({load_pct}%), {applied} apply ({apply_pct}%)\n",
        load_pct = (loaded * 100).checked_div(total).unwrap_or(0),
        apply_pct = (applied * 100).checked_div(total).unwrap_or(0),
    ));
    for (title, failures) in [
        ("LOAD failures", &load_failures),
        ("APPLY failures", &apply_failures),
    ] {
        if failures.is_empty() {
            continue;
        }
        report.push_str(&format!("\n{title} ({}):\n", failures.len()));
        let mut by_class: std::collections::BTreeMap<String, Vec<&str>> =
            std::collections::BTreeMap::new();
        for (label, error) in failures {
            by_class
                .entry(classify_error(error))
                .or_default()
                .push(label.as_str());
        }
        let mut classes: Vec<_> = by_class.into_iter().collect();
        classes.sort_by_key(|entry| std::cmp::Reverse(entry.1.len()));
        for (class, scenarios) in classes {
            report.push_str(&format!("  {:3}x {class}\n", scenarios.len()));
            for sample in scenarios.iter().take(3) {
                report.push_str(&format!("       e.g. {sample}\n"));
            }
        }
    }
    tracing::info!("{report}");
    if !load_failures.is_empty() || !apply_failures.is_empty() {
        bail!(
            "scenario sweep failed: {} load failures, {} apply failures",
            load_failures.len(),
            apply_failures.len()
        );
    }
    Ok(())
}

/// Loads + applies one scenario the way clonk-app does (materials, System.c4g,
/// player registration, simulation ticks) so every script-error warning the
/// C++ engine would not produce becomes visible headlessly. The C++ engine
/// runs official content without script errors; each distinct warning this
/// prints is a parity gap.
fn scenario_errors_command(args: &[String]) -> Result<()> {
    let filter = args
        .first()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .ok_or_else(|| anyhow!("Usage: cargo xtask scenario-errors <filter> [--ticks N]"))?;
    let ticks: u32 = args
        .iter()
        .position(|arg| arg == "--ticks")
        .and_then(|index| args.get(index + 1))
        .map(|value| value.parse())
        .transpose()
        .context("parsing --ticks")?
        .unwrap_or(120);
    // `--defs FXU1,CPFR`: log per-definition object counts at each stage —
    // the shadow-diff histogram's headless counterpart.
    let watched_defs: Vec<String> = args
        .iter()
        .position(|arg| arg == "--defs")
        .and_then(|index| args.get(index + 1))
        .map(|value| value.split(',').map(str::to_string).collect())
        .unwrap_or_default();

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("cannot locate repo root from xtask manifest dir"))?;
    let content_root = repo_root.join("content");

    let scenario_path = WalkDir::new(&content_root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().to_path_buf())
        .find(|path| {
            path.extension()
                .map(|ext| ext.eq_ignore_ascii_case("c4s"))
                .unwrap_or(false)
                && path.to_string_lossy().contains(filter.as_str())
        })
        .ok_or_else(|| anyhow!("no content/**/*.c4s matches `{filter}`"))?;
    tracing::info!("scenario-errors: {}", scenario_path.display());

    let global_material_library = clonk_resources::MaterialLibrary::from_group(
        &clonk_resources::Group::open(content_root.join("Material.c4g"))
            .context("opening content/Material.c4g")?,
    )
    .map_err(|error| anyhow!("loading material library: {error}"))?;
    // Scenario-local materials load FIRST, the global set after
    // (C4Game::InitMaterialTexture, C4Game.cpp:882-960); each load
    // prepends new names (C4Material.cpp:263-299).
    let local_material_library = clonk_resources::Group::open(scenario_path.join("Material.c4g"))
        .ok()
        .and_then(|group| clonk_resources::MaterialLibrary::from_group(&group).ok());
    let local_material_library = if std::env::var("LC_XTASK_GLOBAL_MATS_ONLY").is_ok() {
        None
    } else {
        local_material_library
    };
    let material_library = match &local_material_library {
        Some(local) => clonk_resources::MaterialLibrary::from_overloaded_loads(&[
            local,
            &global_material_library,
        ])
        .map_err(|error| anyhow!("merging material libraries: {error}"))?,
        None => global_material_library,
    };
    let system_scripts = clonk_resources::Group::open(repo_root.join("planet/System.c4g"))
        .ok()
        .and_then(|group| clonk_engine::scenario::load_system_scripts(&group).ok())
        .unwrap_or_default();

    let mut roots: Vec<PathBuf> = scenario_path
        .ancestors()
        .skip(1)
        .take_while(|ancestor| ancestor.starts_with(&content_root))
        .map(Path::to_path_buf)
        .collect();
    roots.push(content_root.clone());
    roots.push(repo_root.clone());
    let resolver = SweepResolver { roots };

    let scenario = clonk_engine::Scenario::load_from_path_with(&scenario_path, &resolver)
        .map_err(|error| anyhow!("load failed: {error}"))?;
    let mut engine = clonk_engine::Engine::new();
    engine.configure_materials_from_library(&material_library);
    engine.install_global_scripts(&system_scripts);
    // Game.Names: the standard clonk names live next to the System.c4g
    // scripts (C4Game::InitScriptEngine, C4Game.cpp:2772).
    engine.set_standard_names(
        clonk_resources::Group::open(repo_root.join("planet/System.c4g"))
            .ok()
            .and_then(|group| group.read_file("Names.txt").ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
    );
    scenario
        .apply_before_players(&mut engine)
        .map_err(|error| anyhow!("apply failed: {error}"))?;
    engine
        .initialize_scenario_script()
        .map_err(|error| anyhow!("scenario Initialize failed: {error}"))?;

    let log_watched = |engine: &clonk_engine::Engine, stage: &str| {
        if watched_defs.is_empty() {
            return;
        }
        let snapshot = engine.snapshot();
        let counts: Vec<String> = watched_defs
            .iter()
            .map(|id| {
                let count = snapshot
                    .objects
                    .iter()
                    .filter(|object| object.definition_id == *id)
                    .count();
                let detail: Vec<String> = snapshot
                    .objects
                    .iter()
                    .filter(|object| object.definition_id == *id)
                    .map(|object| {
                        format!(
                            "#{} {},{} act:{} ph{} t{}",
                            object.id,
                            object.position.x,
                            object.position.y,
                            object.action.name,
                            object.action.phase,
                            object.action.ticks
                        )
                    })
                    .collect();
                let known = engine.definition_ids().any(|known| known == id);
                format!(
                    "{id}={count}{} [{}]",
                    if known { "" } else { " (def missing)" },
                    detail.join(" | ")
                )
            })
            .collect();
        tracing::info!(stage, counts = counts.join(" "), "watched defs");
        if let Some(landscape) = engine.landscape() {
            tracing::info!(
                liquid_750_622 = landscape.is_liquid_at(750, 622),
                solid_750_622 = landscape.is_solid_at(750, 622),
                "probe"
            );
            // `LC_XTASK_PROBE=x,y;x,y`: solidity/liquid at arbitrary
            // pixels — the headless stand-in for GBackSolid spot checks.
            for spec in std::env::var("LC_XTASK_PROBE")
                .unwrap_or_default()
                .split(';')
                .filter(|spec| !spec.is_empty())
            {
                if let Some((x, y)) = spec.split_once(',').and_then(|(x, y)| {
                    x.trim()
                        .parse::<i32>()
                        .ok()
                        .zip(y.trim().parse::<i32>().ok())
                }) {
                    tracing::info!(
                        x,
                        y,
                        solid = landscape.is_solid_at(x, y),
                        liquid = landscape.is_liquid_at(x, y),
                        byte = ?engine.debug_landscape_byte(x, y),
                        material = ?engine.debug_landscape_material_name(x, y),
                        "probe"
                    );
                }
            }
        }
    };

    tracing::info!(
        objects = engine.snapshot().objects.len(),
        "scenario-errors: applied"
    );
    if let Ok(raw) = std::env::var("LC_XTASK_OBJ_DUMP") {
        for id in raw.split(',').filter_map(|s| s.trim().parse::<u64>().ok()) {
            println!("OBJDUMP applied {id} {:?}", engine.debug_object_by_id(id));
        }
    }

    // Join like the real game does (CID_JoinPlr -> C4Game::JoinPlayer ->
    // ScenarioInit): crew and player-owned objects arrive here, then
    // InitializePlayer runs. Tries build/Tyler.c4p for the crew roster.
    let player_file = repo_root.join("build/Tyler.c4p");
    let (name, color_dw, pref_color, pref_position, control_style, auto_context_menu, crew) =
        match player_file.exists() {
            true => match clonk_engine::player_file::PlayerFile::load_from_path(&player_file) {
                Ok(file) => (
                    file.name,
                    file.pref_color_dw & 0xffffff,
                    file.pref_color,
                    file.pref_position,
                    file.pref_control_style,
                    file.pref_auto_context_menu,
                    file.crew,
                ),
                Err(error) => {
                    tracing::warn!(%error, "Tyler.c4p failed to load; joining a default player");
                    (
                        "Tester".to_string(),
                        0xff0000,
                        0,
                        0,
                        false,
                        false,
                        Vec::new(),
                    )
                }
            },
            false => (
                "Tester".to_string(),
                0xff0000,
                0,
                0,
                false,
                false,
                Vec::new(),
            ),
        };
    if std::env::var("LC_XTASK_CREW_DUMP").is_ok() {
        for info in &crew {
            println!(
                "CREWDUMP id={} name={} rank={} exp={}",
                info.id, info.name, info.rank, info.experience
            );
        }
    }
    let joined = engine
        .join_player(clonk_engine::JoinPlayerConfig {
            name,
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw,
            pref_color,
            pref_position,
            crew,
            startup_player_count: 1,
            control_style,
            auto_context_menu,
        })
        .map_err(|error| anyhow!("join_player failed: {error}"))?
        .initialized()
        .ok_or_else(|| anyhow!("join_player is awaiting team selection"))?;
    tracing::info!(
        objects = engine.snapshot().objects.len(),
        number = joined.number,
        start_x = joined.start_x,
        start_y = joined.start_y,
        "scenario-errors: player joined"
    );
    // Creation-order forensics for the numbering-skew epic: dump the
    // post-load id -> definition table (LC_XTASK_SPAWN_DUMP=<min id>).
    if let Ok(min_id) = std::env::var("LC_XTASK_SPAWN_DUMP") {
        let min_id: u64 = min_id.parse().unwrap_or(0);
        let mut rows: Vec<_> = engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.id.as_u64() >= min_id)
            .map(|object| (object.id.as_u64(), object.definition_id.clone()))
            .collect();
        rows.sort();
        for (id, definition) in rows {
            println!("SPAWN {id} {definition}");
        }
    }
    if std::env::var("LC_XTASK_PROBE_MAT").is_ok() {
        for (x, y) in [
            (1998, 556),
            (1998, 557),
            (1998, 558),
            (1998, 559),
            (1998, 560),
            (1996, 557),
            (2000, 559),
        ] {
            println!(
                "MATPROBE ({x},{y}) byte={:?} density={:?} material={:?} in_liquid_probe={:?}",
                engine.debug_landscape_byte(x, y),
                engine.debug_landscape_density(x, y),
                engine.debug_landscape_material_name(x, y),
                engine.debug_landscape_is_liquid(x, y),
            );
        }
    }
    if let Ok(dump) = std::env::var("LC_XTASK_DUMP_LANDSCAPE") {
        if let Some((width, height, bytes)) = engine.debug_landscape_plane() {
            let mut out = Vec::with_capacity(8 + bytes.len());
            out.extend_from_slice(&(width as i32).to_le_bytes());
            out.extend_from_slice(&(height as i32).to_le_bytes());
            out.extend_from_slice(&bytes);
            std::fs::write(&dump, out).expect("landscape dump writes");
            println!("LANDDUMP wrote {width}x{height} to {dump}");
        } else {
            println!("LANDDUMP no pixel grid");
        }
    }
    if std::env::var("LC_XTASK_PROBE_SOLID").is_ok() {
        for y in 255..=272 {
            println!(
                "SOLID 1171,{y} = {}",
                engine.debug_landscape_is_solid(1171, y)
            );
        }
    }
    if std::env::var("LC_XTASK_PROBE_SHAPE").is_ok() {
        for id in ["COAC", "NOPC", "NDWA", "_TLK"] {
            println!("SHAPE {id} {:?}", engine.debug_definition_shape(id));
        }
    }
    log_watched(&engine, "joined");
    // LC_XTASK_OBJ_DUMP=3,42: print per-object forensics after the join
    // and every 5 frames.
    let obj_dump: Vec<u64> = std::env::var("LC_XTASK_OBJ_DUMP")
        .ok()
        .map(|raw| {
            raw.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default();
    for id in &obj_dump {
        println!("OBJDUMP joined {id} {:?}", engine.debug_object_by_id(*id));
        if let Some(object) = engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.id.as_u64() == *id)
        {
            let effects: Vec<String> = object.effects.iter().map(|e| e.name.clone()).collect();
            println!(
                "OBJDUMP effects {id} {effects:?} owner={} alive={} def={} crew={}",
                object.owner, object.alive, object.definition_id, object.crew_member
            );
        }
    }
    for frame in 0..ticks {
        let started = std::time::Instant::now();
        match engine.tick() {
            Ok(snapshot) => {
                if !obj_dump.is_empty() {
                    for id in &obj_dump {
                        println!(
                            "OBJDUMP f{frame} {id} {:?} motion {:?}",
                            engine.debug_object_by_id(*id),
                            engine.debug_object_motion(*id)
                        );
                        if let Some(object) = snapshot
                            .objects
                            .iter()
                            .find(|object| object.id.as_u64() == *id)
                        {
                            let effects: Vec<&str> =
                                object.effects.iter().map(|e| e.name.as_str()).collect();
                            println!(
                                "OBJDUMP f{frame} {id} effects {effects:?} commands {:?} pos {:?} act {} ph {} dir {:?} comdir {:?} mobile {} vel {:?} cont {:?} verts {:?}",
                                object.command_stack,
                                object.position,
                                object.action.name,
                                object.action.phase,
                                object.direction,
                                object.command_direction,
                                object.mobile,
                                object.velocity,
                                object.container,
                                object.vertices.first()
                            );
                        }
                    }
                }
                if frame % 10 == 0 || started.elapsed().as_millis() > 500 {
                    tracing::info!(
                        frame,
                        objects = snapshot.objects.len(),
                        ms = started.elapsed().as_millis(),
                        "tick"
                    );
                }
            }
            Err(error) => {
                let chain: Vec<String> =
                    std::iter::successors(Some(&error as &dyn std::error::Error), |err| {
                        err.source()
                    })
                    .map(|err| err.to_string())
                    .collect();
                tracing::error!(frame, error = chain.join(" <- "), "tick failed");
                break;
            }
        }
    }
    log_watched(&engine, "ticked");
    tracing::info!("scenario-errors: done ({ticks} ticks)");
    Ok(())
}

fn engine_snapshots_command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_engine_snapshots_usage();
        return Ok(());
    }

    match args[0].as_str() {
        "record" => {
            if args.len() > 1 {
                bail!("`engine-snapshots record` does not take additional arguments");
            }
            record_engine_snapshots()
        }
        "verify" => {
            if args.len() > 1 {
                bail!("`engine-snapshots verify` does not take additional arguments");
            }
            verify_engine_snapshots()
        }
        other => bail!(
            "unknown `engine-snapshots` subcommand `{}` (try `cargo xtask engine-snapshots --help`)",
            other
        ),
    }
}

fn print_engine_snapshots_usage() {
    tracing::info!(
        "Usage:\n  cargo xtask engine-snapshots record\n  cargo xtask engine-snapshots verify"
    );
}

/// `cargo xtask parity record|verify` — the C++↔Rust differential parity harness
/// (see `parity/README.md`). `record` regenerates the C++ golden oracle from the
/// real engine primitives; `verify` runs the Rust differential check.
fn parity_command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        tracing::info!(
            "Usage:\n  cargo xtask parity record   Regenerate the C++ golden oracle (parity/golden).\n  cargo xtask parity verify   Run the Rust differential check against the golden."
        );
        return Ok(());
    }
    let paths = WorkspacePaths::detect()?;
    match args[0].as_str() {
        "record" => {
            if args.len() > 1 {
                bail!("`parity record` does not take additional arguments");
            }
            let script = paths.repo_root.join("parity/oracle/gen_golden.sh");
            let status = Command::new("bash")
                .arg(&script)
                .status()
                .with_context(|| format!("failed to run {}", script.display()))?;
            if !status.success() {
                bail!("parity golden generation failed ({status})");
            }
            Ok(())
        }
        "verify" => {
            if args.len() > 1 {
                bail!("`parity verify` does not take additional arguments");
            }
            let status = Command::new("cargo")
                .current_dir(&paths.workspace_dir)
                .args([
                    "nextest",
                    "run",
                    "-p",
                    "clonk-engine-unit-tests",
                    "--test",
                    "engine_inline",
                    "-E",
                    "test(parity_differential_matches_cpp_golden)",
                ])
                .status()
                .context("failed to run cargo nextest for parity verify")?;
            if !status.success() {
                bail!("parity differential check failed ({status})");
            }
            Ok(())
        }
        other => bail!(
            "unknown `parity` subcommand `{}` (try `cargo xtask parity --help`)",
            other
        ),
    }
}

fn record_engine_snapshots() -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    let snapshot_dir = engine_snapshot_dir(&paths);
    fs::create_dir_all(&snapshot_dir)
        .with_context(|| format!("failed to create {}", snapshot_dir.display()))?;

    for scenario in SNAPSHOT_SCENARIOS {
        let recording = (scenario.generator)(scenario.default_frames)
            .with_context(|| format!("failed to record scenario `{}`", scenario.name))?;
        let path = snapshot_dir.join(format!("{}.json", scenario.name));
        let file =
            File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
        recording
            .to_writer(file)
            .map_err(|error| anyhow!(error))
            .with_context(|| format!("failed to serialize recording for `{}`", scenario.name))?;
        let display_path = match path.strip_prefix(&paths.repo_root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => path.clone(),
        };
        tracing::info!(
            path = %display_path.display(),
            frames = scenario.default_frames,
            "wrote engine snapshot"
        );
    }

    Ok(())
}

fn verify_engine_snapshots() -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    let snapshot_dir = engine_snapshot_dir(&paths);

    for scenario in SNAPSHOT_SCENARIOS {
        let path = snapshot_dir.join(format!("{}.json", scenario.name));
        let baseline = load_recording(&path)
            .with_context(|| format!("failed to load baseline {}", path.display()))?;
        let frames = baseline.frames().len();
        if frames != scenario.default_frames {
            bail!(
                "baseline {} contains {} frames but scenario expects {}",
                path.display(),
                frames,
                scenario.default_frames
            );
        }
        let playback = Playback::from_recording(baseline);
        let actual = (scenario.generator)(scenario.default_frames)
            .with_context(|| format!("failed to run scenario `{}`", scenario.name))?;
        playback
            .validate_sequence(actual.into_frames())
            .map_err(|error| anyhow!(error))
            .with_context(|| format!("snapshot mismatch for `{}`", scenario.name))?;
        tracing::info!(
            scenario = scenario.name,
            frames = scenario.default_frames,
            "validated engine snapshot"
        );
    }

    Ok(())
}

fn engine_snapshot_dir(paths: &WorkspacePaths) -> PathBuf {
    paths
        .workspace_dir
        .join("snapshots")
        .join("engine")
        .join("v1")
}

fn load_recording(path: &Path) -> Result<Recording> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    Recording::from_reader(BufReader::new(file)).map_err(|error| anyhow!(error))
}

fn package() -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    build_runtime_binaries(&paths)?;
    audit_release_dependencies(&paths)?;
    dependency_licenses::validate_runtime_dependency_notices(
        &paths.workspace_dir,
        &paths
            .repo_root
            .join("licenses/RUST_THIRD_PARTY_LICENSES.txt"),
    )?;
    let package_dir = assemble_package_layout(&paths)?;
    if paths.target_triple.contains("apple-darwin") {
        let app_dir = assemble_macos_app_bundle(&paths, &package_dir)?;
        let image = create_dmg(&paths, &app_dir)?;
        tracing::info!(path = %image.display(), "packaged Rust port");
        return Ok(());
    }
    let archive = create_archive(&paths, &package_dir)?;
    tracing::info!(path = %archive.display(), "packaged Rust port");
    Ok(())
}

fn build_runtime_binaries(paths: &WorkspacePaths) -> Result<()> {
    tracing::info!("building clonk-game and clonk-app (release)");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(&cargo)
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "clonk-game",
            "-p",
            "clonk-app",
        ])
        .arg("--target-dir")
        .arg(&paths.target_dir)
        .current_dir(&paths.workspace_dir)
        .status()
        .context("failed to invoke cargo build")?;
    if !status.success() {
        bail!("cargo build failed with status {:?}", status.code());
    }
    Ok(())
}

fn assemble_package_layout(paths: &WorkspacePaths) -> Result<PathBuf> {
    let content_src = paths.repo_root.join("content");
    if !content_src.join("Objects.c4d").is_dir() {
        bail!(
            "required game content was not found at {}; initialize the content submodule with `git submodule update --init --recursive`",
            content_src.display()
        );
    }

    let dist_dir = paths.target_dir.join("dist");
    let package_dir = dist_dir.join("clonk-rust");

    if package_dir.exists() {
        fs::remove_dir_all(&package_dir)
            .with_context(|| format!("failed to remove {}", package_dir.display()))?;
    }
    fs::create_dir_all(&package_dir)
        .with_context(|| format!("failed to create {}", package_dir.display()))?;

    let bin_dir = package_dir.join("bin");
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;

    for binary_name in ["clonk-game", "clonk-app"] {
        let exe_name = executable_name(binary_name, &paths.target_triple);
        let built_binary = paths.release_dir.join(&exe_name);
        if !built_binary.exists() {
            bail!(
                "expected {binary_name} binary at {}",
                built_binary.display()
            );
        }
        let packaged_binary = bin_dir.join(&exe_name);
        fs::copy(&built_binary, &packaged_binary).with_context(|| {
            format!(
                "failed to copy {} to {}",
                built_binary.display(),
                packaged_binary.display()
            )
        })?;
        set_executable(&packaged_binary)?;
    }

    copy_file(
        &paths.repo_root.join("COPYING"),
        &package_dir.join("COPYING"),
    )?;
    copy_file(
        &paths.repo_root.join("TRADEMARK"),
        &package_dir.join("TRADEMARK"),
    )?;
    copy_file(
        &paths.repo_root.join("README.md"),
        &package_dir.join("README.md"),
    )?;
    copy_file(
        &paths.repo_root.join("credits.txt"),
        &package_dir.join("credits.txt"),
    )?;
    copy_file(
        &paths.repo_root.join("THIRD_PARTY_GAME_CONTENT.md"),
        &package_dir.join("THIRD_PARTY_GAME_CONTENT.md"),
    )?;

    let planet_dst = package_dir.join("planet");
    copy_tracked_directory(&paths.repo_root, Path::new("planet"), &planet_dst)?;

    let content_dst = package_dir.join("content");
    copy_tracked_directory(&content_src, Path::new(""), &content_dst)?;

    for pack in CONTENT_GAME_PACKS {
        let destination = content_dst.join(pack);
        if !directory_contains_file(&destination)? {
            bail!(
                "required authorized game pack {pack} did not reach the package; it must be tracked in the content submodule"
            );
        }
    }

    let licenses_dst = package_dir.join("licenses");
    copy_tracked_directory(&paths.repo_root, Path::new("licenses"), &licenses_dst)?;
    for relative in [
        "RUST_THIRD_PARTY_LICENSES.txt",
        "third_party/freetype/FTL.TXT",
        "third_party/libpng/LICENSE",
        "third_party/minimp3/LICENSE",
        "third_party/zlib/LICENSE",
    ] {
        if !licenses_dst.join(relative).is_file() {
            bail!(
                "required dependency license licenses/{relative} was not included; ensure it is tracked by Git"
            );
        }
    }

    Ok(package_dir)
}

/// Restructure the staged payload into a macOS application bundle.
///
/// `clonk-app` is the bundle executable rather than the `clonk-game` launcher:
/// the launcher stages `System.c4g`/`Graphics.c4g` next to the bundle, which
/// fails when the app runs from a read-only disk image. It still ships inside
/// `Contents/MacOS` for terminal use.
fn assemble_macos_app_bundle(paths: &WorkspacePaths, package_dir: &Path) -> Result<PathBuf> {
    let app_dir = package_dir.join(MACOS_APP_NAME);
    let contents = app_dir.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources = contents.join("Resources");
    for directory in [&macos_dir, &resources] {
        fs::create_dir_all(directory)
            .with_context(|| format!("failed to create {}", directory.display()))?;
    }

    let bin_dir = package_dir.join("bin");
    for binary_name in ["clonk-game", "clonk-app"] {
        let staged = bin_dir.join(binary_name);
        let bundled = macos_dir.join(binary_name);
        fs::rename(&staged, &bundled).with_context(|| {
            format!(
                "failed to move {} into {}",
                staged.display(),
                bundled.display()
            )
        })?;
        set_executable(&bundled)?;
    }
    fs::remove_dir_all(&bin_dir)
        .with_context(|| format!("failed to remove {}", bin_dir.display()))?;

    for entry in MACOS_BUNDLED_RESOURCES {
        let staged = package_dir.join(entry);
        let bundled = resources.join(entry);
        fs::rename(&staged, &bundled).with_context(|| {
            format!(
                "failed to move {} into {}",
                staged.display(),
                bundled.display()
            )
        })?;
    }

    write_macos_icon(paths, &resources.join(format!("{MACOS_ICON_STEM}.icns")))?;
    fs::write(contents.join("Info.plist"), macos_info_plist())
        .with_context(|| format!("failed to write {}", contents.join("Info.plist").display()))?;
    fs::write(contents.join("PkgInfo"), b"APPL????")
        .with_context(|| format!("failed to write {}", contents.join("PkgInfo").display()))?;

    Ok(app_dir)
}

fn macos_info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>Clonk Rust</string>
	<key>CFBundleExecutable</key>
	<string>clonk-app</string>
	<key>CFBundleIconFile</key>
	<string>{MACOS_ICON_STEM}</string>
	<key>CFBundleIdentifier</key>
	<string>io.github.syb0rg.clonk-rust</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Clonk Rust</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.games</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
</dict>
</plist>
"#,
        version = env!("CARGO_PKG_VERSION")
    )
}

/// Render the project logo into an `.icns` via a temporary iconset.
///
/// The logo is wider than it is tall, so it is composited onto a transparent
/// square first; padding with `sips` would force an opaque background.
fn write_macos_icon(paths: &WorkspacePaths, destination: &Path) -> Result<()> {
    let logo_path = paths.repo_root.join(MACOS_ICON_SOURCE);
    let logo = image::open(&logo_path)
        .with_context(|| format!("failed to read icon source {}", logo_path.display()))?
        .to_rgba8();
    let side = logo.width().max(logo.height());
    let mut square = image::RgbaImage::from_pixel(side, side, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut square,
        &logo,
        i64::from((side - logo.width()) / 2),
        i64::from((side - logo.height()) / 2),
    );

    let iconset_dir = destination.with_extension("iconset");
    if iconset_dir.exists() {
        fs::remove_dir_all(&iconset_dir)
            .with_context(|| format!("failed to remove {}", iconset_dir.display()))?;
    }
    fs::create_dir_all(&iconset_dir)
        .with_context(|| format!("failed to create {}", iconset_dir.display()))?;

    // `iconutil` requires exactly these names, and rejects an incomplete set.
    for (size, name) in [
        (16, "icon_16x16.png"),
        (32, "icon_16x16@2x.png"),
        (32, "icon_32x32.png"),
        (64, "icon_32x32@2x.png"),
        (128, "icon_128x128.png"),
        (256, "icon_128x128@2x.png"),
        (256, "icon_256x256.png"),
        (512, "icon_256x256@2x.png"),
        (512, "icon_512x512.png"),
        (1024, "icon_512x512@2x.png"),
    ] {
        let scaled =
            image::imageops::resize(&square, size, size, image::imageops::FilterType::Lanczos3);
        let path = iconset_dir.join(name);
        scaled
            .save(&path)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }

    let status = Command::new("iconutil")
        .args(["--convert", "icns", "--output"])
        .arg(destination)
        .arg(&iconset_dir)
        .status()
        .context("failed to invoke iconutil")?;
    if !status.success() {
        bail!("iconutil failed with status {:?}", status.code());
    }
    fs::remove_dir_all(&iconset_dir)
        .with_context(|| format!("failed to remove {}", iconset_dir.display()))?;
    Ok(())
}

/// Wrap the bundle in a compressed disk image with the conventional
/// drag-to-Applications layout.
fn create_dmg(paths: &WorkspacePaths, app_dir: &Path) -> Result<PathBuf> {
    let dist_dir = paths.target_dir.join("dist");
    let staging = dist_dir.join("dmg-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .with_context(|| format!("failed to remove {}", staging.display()))?;
    }
    fs::create_dir_all(&staging)
        .with_context(|| format!("failed to create {}", staging.display()))?;
    let staged_app = staging.join(MACOS_APP_NAME);
    fs::rename(app_dir, &staged_app).with_context(|| {
        format!(
            "failed to move {} into {}",
            app_dir.display(),
            staged_app.display()
        )
    })?;
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .context("failed to create the /Applications shortcut")?;

    let image_path = dist_dir.join(format!(
        "clonk-rust-{}-{}.dmg",
        env!("CARGO_PKG_VERSION"),
        paths.target_triple
    ));
    if image_path.exists() {
        fs::remove_file(&image_path)
            .with_context(|| format!("failed to remove {}", image_path.display()))?;
    }
    let status = Command::new("hdiutil")
        .args(["create", "-volname", "Clonk Rust", "-srcfolder"])
        .arg(&staging)
        .args(["-fs", "HFS+", "-format", "UDZO", "-quiet"])
        .arg(&image_path)
        .status()
        .context("failed to invoke hdiutil")?;
    if !status.success() {
        bail!("hdiutil failed with status {:?}", status.code());
    }
    fs::remove_dir_all(&staging)
        .with_context(|| format!("failed to remove {}", staging.display()))?;
    Ok(image_path)
}

fn executable_name(binary_name: &str, target_triple: &str) -> String {
    // The host suffix is wrong whenever `CARGO_BUILD_TARGET` cross-compiles the
    // release binaries, so the extension follows the target triple instead.
    let suffix = if target_triple.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("{binary_name}{suffix}")
}

fn directory_contains_file(path: &Path) -> Result<bool> {
    for entry in WalkDir::new(path) {
        if entry?.file_type().is_file() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn create_archive(paths: &WorkspacePaths, package_dir: &Path) -> Result<PathBuf> {
    let dist_dir = paths.target_dir.join("dist");
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;
    let archive_path = dist_dir.join(archive_file_name(
        env!("CARGO_PKG_VERSION"),
        &paths.target_triple,
    ));
    if archive_path.exists() {
        fs::remove_file(&archive_path)
            .with_context(|| format!("failed to remove {}", archive_path.display()))?;
    }

    let file = File::create(&archive_path)
        .with_context(|| format!("unable to create archive {}", archive_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let base_name = package_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "package".to_string());
    let dir_options = FileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);
    zip.add_directory(format!("{}/", base_name), dir_options)?;

    let file_options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut entries = WalkDir::new(package_dir)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| {
        entry
            .path()
            .strip_prefix(package_dir)
            .map(path_to_zip_string)
            .unwrap_or_default()
    });

    for entry in entries {
        let rel_path = entry.path().strip_prefix(package_dir).unwrap();
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        let mut zip_path = PathBuf::from(&base_name);
        zip_path.push(rel_path);
        let zip_path_str = path_to_zip_string(&zip_path);

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let options = dir_options;
            zip.add_directory(format!("{}/", zip_path_str), options)?;
            continue;
        }

        if metadata.is_file() {
            let mut options = file_options;
            if zip_path.components().nth(1).map(|c| c.as_os_str())
                == Some(std::ffi::OsStr::new("bin"))
            {
                options = options.unix_permissions(0o755);
            } else {
                options = options.unix_permissions(0o644);
            }
            zip.start_file(&zip_path_str, options)?;
            let mut src = File::open(entry.path())?;
            io::copy(&mut src, &mut zip)?;
        }
    }

    zip.finish()?;
    Ok(archive_path)
}

fn archive_file_name(version: &str, target_triple: &str) -> String {
    format!("clonk-rust-{version}-{target_triple}.zip")
}

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        bail!("required file {} was not found", src.display());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    Ok(())
}

fn copy_directory(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        bail!("directory {} does not exist", src.display());
    }
    fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in WalkDir::new(src)
        .into_iter()
        .filter_entry(is_runtime_package_entry)
    {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create {}", target.display()))?;
        } else if metadata.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(entry.path(), &target).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }

    Ok(())
}

fn copy_tracked_directory(repository: &Path, directory: &Path, dst: &Path) -> Result<()> {
    let src = repository.join(directory);
    if !repository.join(".git").exists() {
        // Source archives do not carry Git metadata and are already expected
        // to contain only published content.
        return copy_directory(&src, dst);
    }

    let pathspec = if directory.as_os_str().is_empty() {
        Path::new(".")
    } else {
        directory
    };
    let diff_status = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["diff", "--quiet", "--no-ext-diff", "--"])
        .arg(pathspec)
        .status()
        .with_context(|| format!("failed to inspect tracked files under {}", src.display()))?;
    match diff_status.code() {
        Some(0) => {}
        Some(1) => bail!(
            "tracked files under {} differ from the Git index; stage or discard those changes before packaging",
            src.display()
        ),
        _ => bail!(
            "git diff failed while inspecting {} with status {:?}",
            src.display(),
            diff_status.code()
        ),
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "-z", "--"])
        .arg(pathspec)
        .output()
        .with_context(|| format!("failed to list tracked files under {}", src.display()))?;
    if !output.status.success() {
        bail!(
            "failed to list tracked files under {}: git exited with status {:?}",
            src.display(),
            output.status.code()
        );
    }

    fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;
    for raw_path in output.stdout.split(|byte| *byte == 0) {
        if raw_path.is_empty() {
            continue;
        }
        let repository_relative_text = std::str::from_utf8(raw_path)
            .with_context(|| format!("tracked path under {} is not UTF-8", repository.display()))?;
        let repository_relative = Path::new(repository_relative_text);
        let relative = if directory.as_os_str().is_empty() {
            repository_relative
        } else {
            repository_relative
                .strip_prefix(directory)
                .with_context(|| {
                    format!(
                        "tracked path {} is outside {}",
                        repository_relative.display(),
                        src.display()
                    )
                })?
        };
        if !is_safe_relative_path(relative) || !is_runtime_package_path(relative) {
            continue;
        }

        let source = repository.join(repository_relative);
        let metadata = fs::symlink_metadata(&source)
            .with_context(|| format!("failed to inspect tracked file {}", source.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        copy_file(&source, &dst.join(relative))?;
    }
    Ok(())
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn is_runtime_package_path(path: &Path) -> bool {
    path.components().all(|component| {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        !matches!(
            name.to_str(),
            Some(
                ".git"
                    | ".github"
                    | ".gitignore"
                    | ".gitattributes"
                    | ".editorconfig"
                    | ".DS_Store"
            )
        )
    })
}

fn is_runtime_package_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if entry.file_type().is_symlink() {
        return false;
    }
    is_runtime_package_path(Path::new(entry.file_name()))
}

fn path_to_zip_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn rustc_host_target(workspace_dir: &Path) -> Result<String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(&rustc)
        .arg("-vV")
        .current_dir(workspace_dir)
        .output()
        .context("failed to query the rustc host target")?;
    if !output.status.success() {
        bail!("rustc -vV failed with status {:?}", output.status.code());
    }
    let stdout = String::from_utf8(output.stdout).context("rustc -vV output was not UTF-8")?;
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .context("rustc -vV did not report a host target")
}

fn audit_release_dependencies(paths: &WorkspacePaths) -> Result<()> {
    for binary_name in ["clonk-game", "clonk-app"] {
        let binary = paths
            .release_dir
            .join(executable_name(binary_name, &paths.target_triple));
        if paths.target_triple.contains("apple-darwin") {
            audit_macos_release_binary(&binary)?;
        } else if paths.target_triple.contains("linux") {
            audit_linux_release_binary(&binary, paths.target_triple == paths.host_triple)?;
        }
    }
    Ok(())
}

fn audit_macos_release_binary(binary: &Path) -> Result<()> {
    let output = Command::new("otool")
        .arg("-L")
        .arg(binary)
        .output()
        .with_context(|| format!("failed to audit dynamic libraries for {}", binary.display()))?;
    if !output.status.success() {
        bail!(
            "otool -L failed for {} with status {:?}",
            binary.display(),
            output.status.code()
        );
    }
    let stdout = String::from_utf8(output.stdout).context("otool -L output was not UTF-8")?;
    validate_macos_dependency_output(binary, &stdout)
}

fn audit_linux_release_binary(binary: &Path, resolve_dependencies: bool) -> Result<()> {
    let dynamic = Command::new("readelf")
        .args(["-d"])
        .arg(binary)
        .output()
        .with_context(|| format!("failed to inspect ELF metadata for {}", binary.display()))?;
    if !dynamic.status.success() {
        bail!(
            "readelf -d failed for {} with status {:?}",
            binary.display(),
            dynamic.status.code()
        );
    }
    let dynamic_stdout =
        String::from_utf8(dynamic.stdout).context("readelf -d output was not UTF-8")?;
    validate_linux_elf_output(binary, &dynamic_stdout)?;

    if resolve_dependencies {
        let linked = Command::new("ldd")
            .arg(binary)
            .env_remove("LD_LIBRARY_PATH")
            .output()
            .with_context(|| format!("failed to resolve ELF libraries for {}", binary.display()))?;
        if !linked.status.success() {
            bail!(
                "ldd failed for {} with status {:?}",
                binary.display(),
                linked.status.code()
            );
        }
        let linked_stdout = String::from_utf8(linked.stdout).context("ldd output was not UTF-8")?;
        validate_linux_dependency_output(binary, &linked_stdout)?;
    }
    Ok(())
}

fn validate_macos_dependency_output(binary: &Path, output: &str) -> Result<()> {
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.ends_with(':') {
            continue;
        }
        let dependency = line.split_whitespace().next().unwrap_or_default();
        if dependency.starts_with("/System/Library/")
            || dependency.starts_with("/usr/lib/")
            || dependency.starts_with("@loader_path/")
            || dependency.starts_with("@executable_path/")
        {
            continue;
        }
        bail!(
            "{} has non-relocatable dynamic dependency {dependency}",
            binary.display()
        );
    }
    Ok(())
}

fn validate_linux_dependency_output(binary: &Path, output: &str) -> Result<()> {
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.contains("=> not found") {
            bail!("{} has an unresolved dependency: {line}", binary.display());
        }
        let resolved = line
            .split_once("=>")
            .map(|(_, path)| path.trim())
            .unwrap_or(line)
            .split_whitespace()
            .next()
            .unwrap_or_default();
        if !resolved.starts_with('/') {
            continue;
        }
        if resolved.starts_with("/lib/")
            || resolved.starts_with("/lib64/")
            || resolved.starts_with("/usr/lib/")
            || resolved.starts_with("/usr/lib64/")
        {
            continue;
        }
        bail!(
            "{} has non-relocatable dynamic dependency {resolved}",
            binary.display()
        );
    }
    Ok(())
}

fn validate_linux_elf_output(binary: &Path, output: &str) -> Result<()> {
    for line in output
        .lines()
        .filter(|line| line.contains("(RPATH)") || line.contains("(RUNPATH)"))
    {
        let Some((_, paths)) = line.split_once('[') else {
            continue;
        };
        let paths = paths.split_once(']').map_or(paths, |(paths, _)| paths);
        for path in paths.split(':').filter(|path| !path.is_empty()) {
            if path == "$ORIGIN"
                || path.starts_with("$ORIGIN/")
                || path.starts_with("/lib/")
                || path.starts_with("/lib64/")
                || path.starts_with("/usr/lib/")
                || path.starts_with("/usr/lib64/")
            {
                continue;
            }
            bail!(
                "{} has non-relocatable ELF search path {path}",
                binary.display()
            );
        }
    }
    Ok(())
}

struct WorkspacePaths {
    workspace_dir: PathBuf,
    repo_root: PathBuf,
    target_dir: PathBuf,
    release_dir: PathBuf,
    host_triple: String,
    target_triple: String,
}

impl WorkspacePaths {
    fn detect() -> Result<Self> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_dir = manifest_dir
            .parent()
            .context("xtask manifest is missing parent directory")?
            .to_path_buf();
        // The workspace was hoisted to the repository root.
        let repo_root = workspace_dir.clone();
        let target_dir = env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    workspace_dir.join(path)
                }
            })
            .unwrap_or_else(|| workspace_dir.join("target"));
        let host_triple = rustc_host_target(&workspace_dir)?;
        let explicit_target = env::var("CARGO_BUILD_TARGET")
            .ok()
            .filter(|target| !target.is_empty());
        let target_triple = explicit_target
            .clone()
            .unwrap_or_else(|| host_triple.clone());
        if !target_triple
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            bail!(
                "CARGO_BUILD_TARGET must be a target triple suitable for an archive name, got `{target_triple}`"
            );
        }
        let release_dir = explicit_target.map_or_else(
            || target_dir.join("release"),
            |target| target_dir.join(target).join("release"),
        );
        Ok(Self {
            workspace_dir,
            repo_root,
            target_dir,
            release_dir,
            host_triple,
            target_triple,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const FIXTURE_TARGET: &str = "test-target";

    fn write_fixture(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().expect("fixture path has a parent"))
            .expect("create fixture parent");
        fs::write(path, contents).expect("write fixture");
    }

    fn package_fixture() -> (TempDir, WorkspacePaths) {
        let temp = TempDir::new().expect("temporary workspace");
        let root = temp.path();

        for name in [
            "COPYING",
            "TRADEMARK",
            "README.md",
            "credits.txt",
            "THIRD_PARTY_GAME_CONTENT.md",
        ] {
            write_fixture(&root.join(name), name.as_bytes());
        }
        write_fixture(&root.join("licenses/dependency.txt"), b"dependency license");
        for (relative, contents) in [
            (
                "licenses/RUST_THIRD_PARTY_LICENSES.txt",
                b"Rust dependency licenses".as_slice(),
            ),
            (
                "licenses/third_party/freetype/FTL.TXT",
                b"FreeType license".as_slice(),
            ),
            (
                "licenses/third_party/libpng/LICENSE",
                b"libpng license".as_slice(),
            ),
            (
                "licenses/third_party/minimp3/LICENSE",
                b"minimp3 license".as_slice(),
            ),
            (
                "licenses/third_party/zlib/LICENSE",
                b"zlib license".as_slice(),
            ),
        ] {
            write_fixture(&root.join(relative), contents);
        }
        write_fixture(&root.join("planet/System.c4g/C4.c"), b"system");
        write_fixture(&root.join("planet/Graphics.c4g/Logo.png"), b"logo");
        write_fixture(&root.join("content/Objects.c4d/DefCore.txt"), b"objects");
        write_fixture(
            &root.join("content/Worlds.c4f/Test.c4s/Scenario.txt"),
            b"scenario",
        );
        write_fixture(
            &root.join("content/EkeReloaded.c4d/DefCore.txt"),
            b"eke definitions",
        );
        write_fixture(
            &root.join("content/EkeReloaded.c4f/HarpoonRace.c4s/Scenario.txt"),
            b"harpoon race",
        );
        write_fixture(
            &root.join("content/ClonkMars.c4d/DefCore.txt"),
            b"mars definitions",
        );
        write_fixture(
            &root.join("content/ClonkMars.c4f/Test.c4s/Scenario.txt"),
            b"mars scenario",
        );
        write_fixture(
            &root
                .join("workspace-target/release")
                .join(executable_name("clonk-game", FIXTURE_TARGET)),
            b"launcher",
        );
        write_fixture(
            &root
                .join("workspace-target/release")
                .join(executable_name("clonk-app", FIXTURE_TARGET)),
            b"runtime",
        );

        let paths = WorkspacePaths {
            workspace_dir: root.to_path_buf(),
            repo_root: root.to_path_buf(),
            target_dir: root.join("workspace-target"),
            release_dir: root.join("workspace-target/release"),
            host_triple: FIXTURE_TARGET.to_string(),
            target_triple: FIXTURE_TARGET.to_string(),
        };
        (temp, paths)
    }

    #[test]
    fn executable_name_follows_the_target_triple_not_the_host() {
        assert_eq!(
            executable_name("clonk-app", "x86_64-pc-windows-gnu"),
            "clonk-app.exe"
        );
        assert_eq!(
            executable_name("clonk-app", "x86_64-pc-windows-msvc"),
            "clonk-app.exe"
        );
        assert_eq!(
            executable_name("clonk-app", "x86_64-unknown-linux-gnu"),
            "clonk-app"
        );
        assert_eq!(
            executable_name("clonk-game", "aarch64-apple-darwin"),
            "clonk-game"
        );
    }

    #[test]
    fn package_layout_contains_both_binaries_content_and_legal_files() {
        let (_temp, paths) = package_fixture();

        let package_dir = assemble_package_layout(&paths).expect("assemble package");

        for relative in [
            PathBuf::from("bin").join(executable_name("clonk-game", FIXTURE_TARGET)),
            PathBuf::from("bin").join(executable_name("clonk-app", FIXTURE_TARGET)),
            PathBuf::from("COPYING"),
            PathBuf::from("TRADEMARK"),
            PathBuf::from("README.md"),
            PathBuf::from("credits.txt"),
            PathBuf::from("THIRD_PARTY_GAME_CONTENT.md"),
            PathBuf::from("licenses/dependency.txt"),
            PathBuf::from("licenses/RUST_THIRD_PARTY_LICENSES.txt"),
            PathBuf::from("licenses/third_party/freetype/FTL.TXT"),
            PathBuf::from("licenses/third_party/libpng/LICENSE"),
            PathBuf::from("licenses/third_party/minimp3/LICENSE"),
            PathBuf::from("licenses/third_party/zlib/LICENSE"),
            PathBuf::from("planet/System.c4g/C4.c"),
            PathBuf::from("planet/Graphics.c4g/Logo.png"),
            PathBuf::from("content/Objects.c4d/DefCore.txt"),
            PathBuf::from("content/Worlds.c4f/Test.c4s/Scenario.txt"),
            PathBuf::from("content/EkeReloaded.c4d/DefCore.txt"),
            PathBuf::from("content/EkeReloaded.c4f/HarpoonRace.c4s/Scenario.txt"),
            PathBuf::from("content/ClonkMars.c4d/DefCore.txt"),
            PathBuf::from("content/ClonkMars.c4f/Test.c4s/Scenario.txt"),
        ] {
            assert!(
                package_dir.join(&relative).is_file(),
                "package is missing {}",
                relative.display()
            );
        }
    }

    #[test]
    fn archive_identity_includes_version_and_target_triple() {
        assert_eq!(
            archive_file_name("1.2.3", "aarch64-apple-darwin"),
            "clonk-rust-1.2.3-aarch64-apple-darwin.zip"
        );
    }

    #[test]
    fn archive_entries_are_sorted_and_repeatable() {
        let (_temp, paths) = package_fixture();
        let package_dir = assemble_package_layout(&paths).expect("assemble package");

        let archive = create_archive(&paths, &package_dir).expect("create archive");
        let first_bytes = fs::read(&archive).expect("read first archive");
        let file = File::open(&archive).expect("open archive");
        let mut zip = zip::ZipArchive::new(file).expect("open zip");
        let names = (0..zip.len())
            .map(|index| {
                zip.by_index(index)
                    .expect("read zip entry")
                    .name()
                    .to_string()
            })
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "archive entries are not path-sorted");

        let extracted = TempDir::new().expect("temporary extraction directory");
        zip.extract(extracted.path())
            .expect("extract package archive");
        for relative in [
            PathBuf::from("clonk-rust/bin").join(executable_name("clonk-game", FIXTURE_TARGET)),
            PathBuf::from("clonk-rust/bin").join(executable_name("clonk-app", FIXTURE_TARGET)),
            PathBuf::from("clonk-rust/content/EkeReloaded.c4f/HarpoonRace.c4s/Scenario.txt"),
            PathBuf::from("clonk-rust/licenses/RUST_THIRD_PARTY_LICENSES.txt"),
        ] {
            assert!(
                extracted.path().join(&relative).is_file(),
                "extracted package is missing {}",
                relative.display()
            );
        }
        drop(zip);

        create_archive(&paths, &package_dir).expect("recreate archive");
        let second_bytes = fs::read(&archive).expect("read second archive");
        assert_eq!(
            first_bytes, second_bytes,
            "archive bytes changed on rebuild"
        );
    }

    #[test]
    fn macos_dependency_audit_rejects_homebrew_linkage() {
        let binary = Path::new("/package/bin/clonk-app");
        let system_only = "\
/package/bin/clonk-app:\n\
\t/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit\n\
\t/usr/lib/libSystem.B.dylib\n";
        validate_macos_dependency_output(binary, system_only)
            .expect("system libraries are portable");

        let homebrew = "\
/package/bin/clonk-app:\n\
\t/opt/homebrew/opt/freetype/lib/libfreetype.6.dylib\n";
        let error = validate_macos_dependency_output(binary, homebrew)
            .expect_err("Homebrew dependency must fail packaging");
        assert!(
            error.to_string().contains("/opt/homebrew"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn linux_dependency_audit_rejects_missing_or_user_local_libraries() {
        let binary = Path::new("/package/bin/clonk-app");
        let system_only = "\
\tlinux-vdso.so.1 (0x00007fff)\n\
\tlibz.so.1 => /lib/x86_64-linux-gnu/libz.so.1 (0x00007fff)\n";
        validate_linux_dependency_output(binary, system_only)
            .expect("system libraries are portable");

        for nonportable in [
            "\tlibfreetype.so.6 => not found\n",
            "\tlibfreetype.so.6 => /home/user/local/libfreetype.so.6 (0x00007fff)\n",
        ] {
            assert!(
                validate_linux_dependency_output(binary, nonportable).is_err(),
                "nonportable dependency was accepted: {nonportable}"
            );
        }
    }

    #[test]
    fn elf_dependency_audit_allows_origin_but_rejects_host_search_paths() {
        let binary = Path::new("/package/bin/clonk-app");
        validate_linux_elf_output(
            binary,
            "0x000000000000001d (RUNPATH) Library runpath: [$ORIGIN/lib]",
        )
        .expect("$ORIGIN is relocatable");

        let error = validate_linux_elf_output(
            binary,
            "0x000000000000001d (RUNPATH) Library runpath: [/opt/homebrew/lib]",
        )
        .expect_err("host search path must fail packaging");
        assert!(
            error.to_string().contains("/opt/homebrew"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn package_layout_rejects_an_uninitialized_content_submodule() {
        let (_temp, paths) = package_fixture();
        fs::remove_dir_all(paths.repo_root.join("content/Objects.c4d"))
            .expect("remove content sentinel");

        let error = assemble_package_layout(&paths).expect_err("uninitialized content must fail");

        assert!(
            error
                .to_string()
                .contains("initialize the content submodule"),
            "unexpected error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn package_layout_excludes_repository_metadata_and_external_symlinks() {
        use std::os::unix::fs::symlink;

        let (_temp, paths) = package_fixture();
        write_fixture(
            &paths.repo_root.join("content/.github/workflows/build.yml"),
            b"workflow",
        );
        let private_file = paths.repo_root.join("private.txt");
        write_fixture(&private_file, b"must not be packaged");
        symlink(
            &private_file,
            paths.repo_root.join("content/external-private-link"),
        )
        .expect("create external fixture symlink");

        let package_dir = assemble_package_layout(&paths).expect("assemble package");

        for relative in ["content/.github", "content/external-private-link"] {
            assert!(
                !package_dir.join(relative).exists(),
                "development-only path leaked into package: {relative}"
            );
        }
    }

    #[test]
    fn package_layout_excludes_untracked_content_from_a_submodule_checkout() {
        let (_temp, paths) = package_fixture();
        let content = paths.repo_root.join("content");
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&content)
            .status()
            .expect("run git init");
        assert!(init.success(), "git init failed");
        let add = Command::new("git")
            .args([
                "add",
                "Objects.c4d",
                "Worlds.c4f",
                "EkeReloaded.c4d",
                "EkeReloaded.c4f",
                "ClonkMars.c4d",
                "ClonkMars.c4f",
            ])
            .current_dir(&content)
            .status()
            .expect("run git add");
        assert!(add.success(), "git add failed");
        write_fixture(
            &content.join("EkeReloaded.c4f/Secret.txt"),
            b"untracked private content",
        );

        let package_dir = assemble_package_layout(&paths).expect("assemble package");

        assert!(
            package_dir
                .join("content/EkeReloaded.c4f/HarpoonRace.c4s/Scenario.txt")
                .is_file(),
            "tracked Eke Reloaded content must reach the package"
        );
        assert!(
            !package_dir
                .join("content/EkeReloaded.c4f/Secret.txt")
                .exists(),
            "untracked Eke Reloaded content leaked into the package"
        );
    }

    #[test]
    fn package_layout_excludes_untracked_planet_assets_from_a_checkout() {
        let (_temp, paths) = package_fixture();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git init");
        assert!(init.success(), "git init failed");
        let add = Command::new("git")
            .args(["add", "planet", "licenses", "content"])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git add");
        assert!(add.success(), "git add failed");
        write_fixture(
            &paths.repo_root.join("planet/PrivateAsset.c4g/Secret.txt"),
            b"untracked private asset",
        );

        let package_dir = assemble_package_layout(&paths).expect("assemble package");

        assert!(
            !package_dir.join("planet/PrivateAsset.c4g").exists(),
            "untracked planet asset leaked into the package"
        );
    }

    #[test]
    fn package_layout_rejects_an_untracked_dependency_notice() {
        let (_temp, paths) = package_fixture();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git init");
        assert!(init.success(), "git init failed");
        let add = Command::new("git")
            .args([
                "add",
                "planet",
                "licenses/dependency.txt",
                "licenses/third_party",
                "content",
            ])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git add");
        assert!(add.success(), "git add failed");

        let error =
            assemble_package_layout(&paths).expect_err("untracked notice must fail packaging");

        assert!(
            error.to_string().contains("RUST_THIRD_PARTY_LICENSES.txt"),
            "unexpected error: {error:#}"
        );
        assert!(
            error.to_string().contains("tracked by Git"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn package_layout_rejects_modified_tracked_planet_bytes() {
        let (_temp, paths) = package_fixture();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git init");
        assert!(init.success(), "git init failed");
        let add = Command::new("git")
            .args(["add", "planet", "licenses", "content"])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git add");
        assert!(add.success(), "git add failed");
        write_fixture(
            &paths.repo_root.join("planet/System.c4g/C4.c"),
            b"modified after staging",
        );

        let error = assemble_package_layout(&paths)
            .expect_err("modified tracked asset must fail packaging");

        assert!(
            error.to_string().contains("differ from the Git index"),
            "unexpected error: {error:#}"
        );
    }
}
