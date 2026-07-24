mod audit;

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
                apply_failures
                    .push((label, "HUNG: apply did not finish within 120s".to_string()));
                // The worker thread is abandoned; the sweep process exits at
                // the end regardless.
            }
        }
    }

    let mut report = String::new();
    report.push_str(&format!(
        "\nscenario sweep: {total} scenarios — {loaded} load ({load_pct}%), {applied} apply ({apply_pct}%)\n",
        load_pct = if total > 0 { loaded * 100 / total } else { 0 },
        apply_pct = if total > 0 { applied * 100 / total } else { 0 },
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
        classes.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
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
            tracing::info!(liquid_750_622 = landscape.is_liquid_at(750, 622), solid_750_622 = landscape.is_solid_at(750, 622), "probe");
            // `LC_XTASK_PROBE=x,y;x,y`: solidity/liquid at arbitrary
            // pixels — the headless stand-in for GBackSolid spot checks.
            for spec in std::env::var("LC_XTASK_PROBE")
                .unwrap_or_default()
                .split(';')
                .filter(|spec| !spec.is_empty())
            {
                if let Some((x, y)) = spec.split_once(',').and_then(|(x, y)| {
                    x.trim().parse::<i32>().ok().zip(y.trim().parse::<i32>().ok())
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
        for (x, y) in [(1998, 556), (1998, 557), (1998, 558), (1998, 559), (1998, 560), (1996, 557), (2000, 559)] {
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
        .map(|raw| raw.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_default();
    for id in &obj_dump {
        println!("OBJDUMP joined {id} {:?}", engine.debug_object_by_id(*id));
        if let Some(object) = engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.id.as_u64() == *id)
        {
            let effects: Vec<String> =
                object.effects.iter().map(|e| e.name.clone()).collect();
            println!("OBJDUMP effects {id} {effects:?} owner={} alive={} def={} crew={}", object.owner, object.alive, object.definition_id, object.crew_member);
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
                let chain: Vec<String> = std::iter::successors(
                    Some(&error as &dyn std::error::Error),
                    |err| err.source(),
                )
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
    build_clonk_game(&paths)?;
    let package_dir = assemble_package_layout(&paths)?;
    let archive = create_archive(&paths, &package_dir)?;
    tracing::info!(path = %archive.display(), "packaged Rust port");
    Ok(())
}

fn build_clonk_game(paths: &WorkspacePaths) -> Result<()> {
    tracing::info!("building clonk-game (release)");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "clonk-game"])
        .current_dir(&paths.workspace_dir)
        .status()
        .context("failed to invoke cargo build")?;
    if !status.success() {
        bail!("cargo build failed with status {:?}", status.code());
    }
    Ok(())
}

fn assemble_package_layout(paths: &WorkspacePaths) -> Result<PathBuf> {
    let dist_dir = paths.workspace_dir.join("target").join("dist");
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

    let exe_name = format!("clonk-game{}", env::consts::EXE_SUFFIX);
    let built_binary = paths
        .workspace_dir
        .join("target")
        .join("release")
        .join(&exe_name);
    if !built_binary.exists() {
        bail!("expected clonk-game binary at {}", built_binary.display());
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

    copy_file(
        &paths.repo_root.join("COPYING"),
        &package_dir.join("COPYING"),
    )?;
    copy_file(
        &paths.repo_root.join("README.md"),
        &package_dir.join("README.md"),
    )?;
    copy_file(
        &paths.repo_root.join("credits.txt"),
        &package_dir.join("credits.txt"),
    )?;

    let planet_src = paths.repo_root.join("planet");
    let planet_dst = package_dir.join("planet");
    copy_directory(&planet_src, &planet_dst)?;

    Ok(package_dir)
}

fn create_archive(paths: &WorkspacePaths, package_dir: &Path) -> Result<PathBuf> {
    let dist_dir = paths.workspace_dir.join("target").join("dist");
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;
    let archive_path = dist_dir.join("clonk-rust.zip");
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

    for entry in WalkDir::new(package_dir) {
        let entry = entry?;
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

    for entry in WalkDir::new(src) {
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

struct WorkspacePaths {
    workspace_dir: PathBuf,
    repo_root: PathBuf,
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
        Ok(Self {
            workspace_dir,
            repo_root,
        })
    }

}
