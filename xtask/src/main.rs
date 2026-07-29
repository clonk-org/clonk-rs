mod audit;
mod chaos;
mod components;
mod manifest;

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
const MACOS_BUNDLED_RESOURCES: [&str; 5] =
    ["planet", "content", "COPYING", "README.md", "credits.txt"];

/// What a macOS release names itself, and the directory its fused binaries land
/// in. Not a rustc target — `lipo` produces it, cargo never does.
const MACOS_UNIVERSAL_TRIPLE: &str = "universal-apple-darwin";

/// The real architectures a universal build fuses, in `lipo` argument order.
///
/// One `.app` for both cuts the macOS release in half: a per-architecture disk
/// image is ~340 MB of identical game data wrapped around ~20 MB of different
/// executables.
const MACOS_UNIVERSAL_ARCHES: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

/// The executables a release ships, in every layout.
const RUNTIME_BINARIES: [&str; 2] = ["clonk-game", "clonk-app"];

fn main() -> Result<()> {
    clonk_logging::init();
    clonk_logging::install_panic_hook();

    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some("package") => package(PackageOptions::parse(args)?),
        Some("engine-snapshots") => {
            let tail: Vec<String> = args.collect();
            engine_snapshots_command(&tail)
        }
        Some("dev-check") => {
            let tail: Vec<String> = args.collect();
            dev_check::command(&tail)
        }
        Some("chaos") => {
            let tail: Vec<String> = args.collect();
            chaos::command(&tail)
        }
        Some("parity") => {
            let tail: Vec<String> = args.collect();
            parity_command(&tail)
        }
        Some("update-manifest") => {
            let tail: Vec<String> = args.collect();
            update_manifest_command(&tail)
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
        "Usage:\n  cargo xtask package                 Build the Rust port and bundle a distributable archive.\n  cargo xtask dev-check [options]     Run the change-aware sub-60-second developer feedback loop.\n  cargo xtask engine-snapshots record Regenerate engine snapshot baselines.\n  cargo xtask engine-snapshots verify Check Rust engine output against recorded baselines.\n  cargo xtask parity record|verify    C++↔Rust differential parity harness (see parity/README.md).\n  cargo xtask update-manifest generate --version <X.Y.Z> --released-at <RFC3339> --components <dir> --out-dir <dir> --content-commit <sha> --content-sha256 <hex> --content-size <bytes>  Describe the update components for in-app updating.\n  cargo xtask chaos run|record|verify Potato-on-a-bad-link regression harness (report-only).\n  cargo xtask scenario-sweep [filter] [--verbose]  Load+apply every real scenario in content/; the scenario-load parity scoreboard.\n  cargo xtask scenario-audit [filter] [--verbose]  Audit applied-world fidelity (landscape materials, objects, init placements)."
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

/// `cargo xtask update-manifest generate|--help`.
///
/// Runs in the publishing job once every platform has built, from the binary
/// that job downloads rather than a fresh compile: the manifest only describes
/// bytes that already exist.
fn update_manifest_command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        tracing::info!(
            "Usage:\n  cargo xtask update-manifest generate --version <X.Y.Z> --released-at <RFC3339> --components <dir> --out-dir <dir> --content-commit <sha> --content-sha256 <hex> --content-size <bytes>"
        );
        return Ok(());
    }
    match args[0].as_str() {
        "generate" => generate_update_manifest(GenerateManifestOptions::parse(&args[1..])?),
        other => bail!(
            "unknown `update-manifest` subcommand `{}` (try `cargo xtask update-manifest --help`)",
            other
        ),
    }
}

/// The triples a release ships, and therefore the ones every component must
/// offer an archive for.
///
/// Six entries for three builds: `x86_64-pc-windows-gnu` and the two macOS
/// architecture triples are all served through [`UPDATE_TRIPLE_ALIASES`] rather
/// than being builds of their own.
const UPDATE_TARGET_TRIPLES: [&str; 6] = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "x86_64-pc-windows-gnu",
    MACOS_UNIVERSAL_TRIPLE,
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
];

/// `(the triple a client reports, the triple whose archive it is served)`.
///
/// A client can only report the triple *cargo* built it for: `build.rs` reads
/// `TARGET`, and nothing else in the toolchain knows. Dropping a key here
/// therefore leaves every install that reports it with no entry for itself —
/// told there is no update, for ever, with nothing to notice it, because
/// "nothing offered for my triple" is indistinguishable from "up to date".
///
/// The two aliases are not the same kind of thing:
///
/// * **Windows** releases were cross-built from Linux as `x86_64-pc-windows-gnu`
///   until the build moved to a native MSVC runner. That alias is a *migration*
///   and drains: the `engine` component replaces the executables wholesale, so
///   applying it turns a gnu install into an MSVC one and the next check
///   reports the new triple.
/// * **macOS** ships one universal `.app`, but each slice of it is compiled for
///   a real architecture triple, so a universal install still reports
///   `aarch64-apple-darwin` or `x86_64-apple-darwin` *after* updating. Those two
///   aliases are permanent — they are how every Mac resolves, not a bridge from
///   an older layout — and removing either one strands that architecture.
///
/// Only `engine` needs an alias; the shared components are offered to every
/// triple in [`UPDATE_TARGET_TRIPLES`] already.
const UPDATE_TRIPLE_ALIASES: [(&str, &str); 3] = [
    ("x86_64-pc-windows-gnu", "x86_64-pc-windows-msvc"),
    ("aarch64-apple-darwin", MACOS_UNIVERSAL_TRIPLE),
    ("x86_64-apple-darwin", MACOS_UNIVERSAL_TRIPLE),
];

/// The repository that builds and publishes the `content` archive.
///
/// It is the repository the game data lives in, and the *only* builder of those
/// bytes. This one used to build them too and re-upload 225 MB on every daily
/// release; two content-addressed producers would have to agree byte for byte
/// forever, so it stopped and references the published artifact instead. See
/// [`components::ComponentId::BUILT`].
const CONTENT_REPOSITORY: &str = "syb0rg/clonk-rs-content";

/// The asset name that repository publishes.
const CONTENT_ARCHIVE: &str = "content.zip";

/// The tag naming a content commit's release, which is how the `content`
/// submodule pin resolves to an artifact with no lookup table in between.
fn content_release_tag(commit: &str) -> String {
    format!("content-{commit}")
}

/// The triple recorded against a shared archive.
///
/// `content` and `planet` are byte-identical everywhere, so `build_manifest`
/// offers them to every triple and never reads this field. Deliberately empty
/// rather than a real triple: were that ever to change, the manifest would
/// carry a visibly empty key — which the coverage check below rejects —
/// instead of quietly claiming `content` was built for Linux alone.
const SHARED_ARCHIVE_TRIPLE: &str = "";

/// Arguments for `cargo xtask update-manifest generate`.
///
/// The version and the release timestamp are passed in rather than read from
/// the tree or the clock: the manifest must describe the commit that was
/// built, and the publishing job runs after the builds, on another machine.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GenerateManifestOptions {
    /// The release version, as the workflow resolved it. Also the version the
    /// engine archives must carry in their names.
    version: String,
    /// RFC 3339, and the client's replay guard: a client refuses a manifest
    /// older than the one it already applied.
    released_at: String,
    /// Where the emitted component archives were downloaded to.
    components_dir: PathBuf,
    /// Where to write `manifest.json`. A directory rather than a file path
    /// because the name is part of the client contract — clients fetch
    /// `releases/latest/download/manifest.json` and nothing else.
    out_dir: PathBuf,
    /// The `content` submodule pin, which names the release in
    /// [`CONTENT_REPOSITORY`] that publishes the archive this release points at.
    content_commit: String,
    /// The digest that release published, as its `content.sha256` sidecar
    /// records it. Copied rather than recomputed: recomputing would mean
    /// downloading 225 MB into the publishing job to learn something the
    /// producer already stated, and a mismatch is caught by the client anyway.
    content_sha256: String,
    /// The published asset's size, used for the client's disk-space check.
    content_size: u64,
}

impl GenerateManifestOptions {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut version = None;
        let mut released_at = None;
        let mut components_dir = None;
        let mut out_dir = None;
        let mut content_commit = None;
        let mut content_sha256 = None;
        let mut content_size = None;

        let mut index = 0;
        while index < arguments.len() {
            // Both `--name value` and `--name=value`: the workflow writes the
            // former, humans reach for the latter.
            let (name, value) = match arguments[index].split_once('=') {
                Some((name, value)) => {
                    index += 1;
                    (name.to_string(), value.to_string())
                }
                None => {
                    let name = arguments[index].clone();
                    let value = arguments
                        .get(index + 1)
                        .cloned()
                        .ok_or_else(|| anyhow!("`{name}` needs a value"))?;
                    index += 2;
                    (name, value)
                }
            };
            match name.as_str() {
                "--version" => version = Some(value),
                "--released-at" => released_at = Some(value),
                "--components" => components_dir = Some(PathBuf::from(value)),
                "--out-dir" => out_dir = Some(PathBuf::from(value)),
                "--content-commit" => content_commit = Some(value),
                "--content-sha256" => content_sha256 = Some(value),
                "--content-size" => content_size = Some(value),
                other => bail!("unexpected argument `{other}` for `update-manifest generate`"),
            }
        }

        // Every field is required: defaulting any of them would publish a
        // manifest that quietly describes the wrong release. Resolved in the
        // order they are documented, so an omitted argument is reported as
        // itself rather than as whichever check happened to run first.
        let version = version.ok_or_else(|| missing_manifest_argument("--version"))?;
        let released_at = released_at.ok_or_else(|| missing_manifest_argument("--released-at"))?;
        let components_dir =
            components_dir.ok_or_else(|| missing_manifest_argument("--components"))?;
        let out_dir = out_dir.ok_or_else(|| missing_manifest_argument("--out-dir"))?;

        // The `content` arguments especially: that archive is not in the
        // components directory to be missed, so nothing downstream would
        // notice its absence.
        let content_commit = content_commit
            .filter(|commit| is_commit_sha(commit))
            .ok_or_else(|| {
                anyhow!(
                    "`update-manifest generate` needs `--content-commit <40 hex digits>`, the \
                     `content` submodule pin whose release publishes {CONTENT_ARCHIVE}"
                )
            })?;
        let content_sha256 = content_sha256
            .filter(|digest| is_sha256_hex(digest))
            .ok_or_else(|| {
                anyhow!(
                    "`update-manifest generate` needs `--content-sha256 <64 hex digits>`; with no \
                     manifest signature that digest is the whole integrity story for {CONTENT_ARCHIVE}"
                )
            })?;
        let content_size = content_size
            .and_then(|size| size.parse::<u64>().ok())
            .filter(|size| *size > 0)
            .ok_or_else(|| {
                anyhow!("`update-manifest generate` needs `--content-size <bytes>`, above zero")
            })?;

        Ok(Self {
            version,
            released_at,
            components_dir,
            out_dir,
            content_commit,
            content_sha256,
            content_size,
        })
    }
}

/// A full Git object name, which is what a submodule pin always is.
///
/// Lowercase, like [`is_sha256_hex`] and for the same reason: this pin becomes
/// the release tag `content-<sha>`, and GitHub matches a tag byte for byte. An
/// uppercase pin would name a release that does not exist while looking
/// perfectly valid here. Git renders object names in lowercase, so the stricter
/// rule turns nothing legitimate away.
fn is_commit_sha(text: &str) -> bool {
    is_lowercase_hex(text, 40)
}

/// Lowercase hex SHA-256, the form the manifest records and a client compares.
fn is_sha256_hex(text: &str) -> bool {
    is_lowercase_hex(text, 64)
}

/// Exactly `digits` lowercase hex characters, and nothing else.
///
/// Deliberately not `is_ascii_hexdigit`: every hex string in a manifest is
/// compared as text by something downstream that will not case-fold it.
fn is_lowercase_hex(text: &str, digits: usize) -> bool {
    text.len() == digits
        && text
            .chars()
            .all(|digit| digit.is_ascii_digit() || matches!(digit, 'a'..='f'))
}

fn missing_manifest_argument(name: &str) -> anyhow::Error {
    anyhow!("`update-manifest generate` needs `{name}`")
}

/// Which component an emitted archive is, and which triple it was built for.
///
/// `None` means every triple: `planet` is prefix-free, so its bytes are
/// identical on all four. `engine` is the reason the manifest is keyed by
/// triple at all, and its filename is the only place that triple survives —
/// the archives arrive in the publishing job as a flat directory.
fn archive_identity(
    name: &str,
    version: &str,
) -> Result<(components::ComponentId, Option<String>)> {
    // A content archive here would be a second builder of bytes that must stay
    // identical forever, which is exactly what moving the build away removed.
    if name == CONTENT_ARCHIVE || name.starts_with("content-") {
        bail!(
            "`{name}` is a content archive, and content is built and published by \
             {CONTENT_REPOSITORY}; this release references that artifact rather than \
             uploading one of its own"
        );
    }
    components::ComponentId::BUILT
        .into_iter()
        .filter(|component| component.is_platform_independent())
        .find(|component| name.starts_with(&format!("{}-", component.name())))
        .map(|component| Ok((component, None)))
        .unwrap_or_else(|| {
            engine_archive_triple(name, version)
                .map(|triple| (components::ComponentId::Engine, Some(triple)))
        })
}

/// The triple an engine archive was built for, read out of
/// `clonk-rust-<version>-engine-<triple>.zip`.
///
/// The version is matched rather than skipped, so an archive left over from an
/// earlier release cannot enter this manifest: it would name an asset this
/// release never uploads, and every client on that triple would fail to fetch.
fn engine_archive_triple(name: &str, version: &str) -> Result<String> {
    name.strip_prefix(&format!("clonk-rust-{version}-engine-"))
        .and_then(|rest| rest.strip_suffix(".zip"))
        .filter(|triple| !triple.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            if name.contains("-engine-") {
                anyhow!("`{name}` is an engine archive from another release, not {version}")
            } else {
                anyhow!(
                    "`{name}` matches no update component; refusing to publish a partial manifest"
                )
            }
        })
}

/// Reads back the component archives a release run emitted, hashing each one
/// and pairing it with the triple it belongs to.
///
/// The digests are recomputed here rather than carried from the build jobs:
/// the manifest must describe the bytes that are about to be uploaded, not
/// what a different machine reported writing.
fn scan_emitted_components(
    directory: &Path,
    version: &str,
) -> Result<Vec<(String, components::EmittedComponent)>> {
    let mut archives: Vec<PathBuf> = fs::read_dir(directory)
        .with_context(|| {
            format!(
                "failed to read components directory {}",
                directory.display()
            )
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()
        .with_context(|| format!("failed to list {}", directory.display()))?
        .into_iter()
        // Only zips claim to be components; the publishing job assembles
        // release notes and checksums in the same tree.
        .filter(|path| path.extension().is_some_and(|extension| extension == "zip"))
        .collect();
    // Sorted so the manifest does not depend on directory iteration order.
    archives.sort();

    let scanned = archives
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            // An archive nobody recognises is a hard error: dropping it would
            // publish a manifest that omits a shipped component, and clients
            // would never learn the component existed.
            let (id, triple) = archive_identity(&name, version)?;
            let size = fs::metadata(path)
                .with_context(|| format!("failed to stat {}", path.display()))?
                .len();
            Ok((
                triple.unwrap_or_else(|| SHARED_ARCHIVE_TRIPLE.to_string()),
                components::EmittedComponent {
                    id,
                    path: path.clone(),
                    sha256: components::hex_digest(&components::sha256_file(path)?),
                    size,
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    if scanned.is_empty() {
        bail!(
            "no component archives in {}; a manifest describing nothing would tell every \
             client there is no update",
            directory.display()
        );
    }
    Ok(scanned)
}

/// Refuses a manifest that would leave some client with nothing to fetch.
///
/// The publishing job runs this *before* the tag: a release whose macOS pass
/// never uploaded its engine archive has to fail loudly while the version can
/// still be re-released, rather than publish a document that silently tells
/// every Mac there is no engine for it.
fn verify_every_triple_is_covered(manifest: &manifest::Manifest) -> Result<()> {
    let missing: Vec<String> = components::ComponentId::ALL
        .into_iter()
        .flat_map(|component| {
            let entry = manifest
                .components
                .iter()
                .find(|entry| entry.name == component.name());
            UPDATE_TARGET_TRIPLES
                .into_iter()
                .filter(move |triple| {
                    entry.is_none_or(|entry| !entry.targets.contains_key(*triple))
                })
                .map(move |triple| format!("{}/{triple}", component.name()))
        })
        .collect();
    if !missing.is_empty() {
        bail!(
            "the update manifest offers nothing for {}; refusing to publish a release those \
             clients could not complete",
            missing.join(", ")
        );
    }

    // The mirror image: an archive built for a triple this release does not
    // ship would appear under no other component, so a client reading it could
    // never assemble a complete install.
    let unshipped: Vec<String> = manifest
        .components
        .iter()
        .flat_map(|entry| {
            entry
                .targets
                .keys()
                .filter(|triple| !UPDATE_TARGET_TRIPLES.contains(&triple.as_str()))
                .map(move |triple| format!("{}/{triple}", entry.name))
        })
        .collect();
    if !unshipped.is_empty() {
        bail!(
            "the update manifest describes {}, which this release does not ship for every \
             component",
            unshipped.join(", ")
        );
    }
    Ok(())
}

/// Offers each retired triple the archive of the triple that replaced it.
///
/// A second entry pointing at the *same* archive, rather than a second build:
/// nothing is copied or re-hashed, so the release uploads one Windows engine
/// archive and the manifest names it twice. See [`UPDATE_TRIPLE_ALIASES`].
fn serve_retired_triples(
    scanned: &[(String, components::EmittedComponent)],
) -> Vec<(String, components::EmittedComponent)> {
    let aliased = scanned.iter().flat_map(|(built_for, component)| {
        UPDATE_TRIPLE_ALIASES
            .into_iter()
            // Shared components already reach every triple; aliasing them
            // would record the same archive twice under one key.
            .filter(move |(_, served_by)| {
                !component.id.is_platform_independent() && served_by == built_for
            })
            .map(move |(retired, _)| (retired.to_string(), component.clone()))
    });
    scanned.iter().cloned().chain(aliased).collect()
}

/// The `content` entry, describing an archive this repository does not build.
///
/// Everything about it comes from the content repository's release: the tag
/// derives from the submodule pin, and the digest and size are what that
/// release published. Nothing is recomputed, because recomputing would require
/// the second builder this arrangement exists to remove.
fn referenced_content(options: &GenerateManifestOptions) -> manifest::ReferencedComponent {
    manifest::ReferencedComponent {
        id: components::ComponentId::Content,
        source: manifest::ArchiveSource {
            repo: CONTENT_REPOSITORY.to_string(),
            tag: content_release_tag(&options.content_commit),
        },
        archive: CONTENT_ARCHIVE.to_string(),
        sha256: options.content_sha256.clone(),
        size: options.content_size,
    }
}

fn generate_update_manifest(options: GenerateManifestOptions) -> Result<()> {
    let scanned = scan_emitted_components(&options.components_dir, &options.version)?;
    let emitted = serve_retired_triples(&scanned);
    let referenced = [referenced_content(&options)];
    let manifest = manifest::build_manifest(
        &options.version,
        clonk_core::version::ENGINE_VERSION,
        &options.released_at,
        &emitted,
        &referenced,
        &UPDATE_TARGET_TRIPLES,
    );
    // Checked before anything is written, so a refused manifest leaves no
    // half-valid document for a later step to pick up.
    verify_every_triple_is_covered(&manifest)?;

    fs::create_dir_all(&options.out_dir)
        .with_context(|| format!("failed to create {}", options.out_dir.display()))?;
    manifest::write_manifest(&options.out_dir, &manifest)?;

    tracing::info!(
        path = %options.out_dir.join("manifest.json").display(),
        version = %manifest.version,
        components = manifest.components.len(),
        // The archives that exist, not the manifest entries: an alias adds an
        // entry without adding a file.
        archives = scanned.len(),
        "wrote update manifest"
    );
    Ok(())
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
        .join("testdata")
        .join("engine")
        .join("v1")
}

fn load_recording(path: &Path) -> Result<Recording> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    Recording::from_reader(BufReader::new(file)).map_err(|error| anyhow!(error))
}

/// Command-line options for `cargo xtask package`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PackageOptions {
    /// Whether to compress the staged layout into a release archive.
    archive: bool,
    /// Which update components to emit alongside it.
    components: ComponentSelection,
}

/// Which per-component update archives a packaging run produces.
///
/// Defaults to `None` so a local `cargo xtask package` stays exactly what it
/// was. The shared components are large and only the Linux release pass needs
/// to build them, since their bytes are identical everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComponentSelection {
    None,
    /// Just this platform's binaries.
    Engine,
    /// Binaries plus the platform-independent `planet`. Not `content`: that
    /// archive is published by [`CONTENT_REPOSITORY`], and building a second
    /// one here is precisely what must not happen.
    All,
}

impl ComponentSelection {
    fn includes_shared(self) -> bool {
        matches!(self, ComponentSelection::All)
    }

    fn includes_engine(self) -> bool {
        matches!(self, ComponentSelection::Engine | ComponentSelection::All)
    }
}

impl PackageOptions {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self> {
        let mut options = Self {
            archive: true,
            components: ComponentSelection::None,
        };
        for argument in arguments {
            match argument.as_str() {
                // The Windows installer consumes the staged directory itself,
                // so the archive step would only produce a file to discard.
                "--no-archive" => options.archive = false,
                "--components=none" => options.components = ComponentSelection::None,
                "--components=engine" => options.components = ComponentSelection::Engine,
                "--components=all" => options.components = ComponentSelection::All,
                other => bail!("unexpected argument `{other}` for `package` command"),
            }
        }
        Ok(options)
    }
}

/// What `package` writes once the payload has been staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageOutput {
    /// The staged tree only; the caller wraps it (the Windows installer does).
    StagedOnly,
    Archive,
    DiskImage,
}

fn package_output(target_triple: &str, archive: bool) -> PackageOutput {
    if !archive {
        return PackageOutput::StagedOnly;
    }
    if target_triple.contains("apple-darwin") {
        return PackageOutput::DiskImage;
    }
    PackageOutput::Archive
}

fn package(options: PackageOptions) -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    build_runtime_binaries(&paths)?;
    audit_release_dependencies(&paths)?;
    let package_dir = assemble_package_layout(&paths)?;
    // Runs on the real tree, not a fixture: an entry that belongs to no update
    // component would ship in the installer but never reach a client updating
    // in place, and nothing else would notice.
    components::verify_components_cover_layout(&package_dir)?;
    let output = package_output(&paths.target_triple, options.archive);
    let components_dir = paths.target_dir.join("dist").join("components");

    // Shared components are emitted from the flat layout, before the macOS
    // bundle relocates `planet` and `content` into `Contents/Resources`, and
    // before any platform prefix could reach their entry names. `content` is
    // not among them: the content repository builds and publishes that archive.
    if options.components.includes_shared() {
        for component in components::ComponentId::BUILT
            .into_iter()
            .filter(|component| component.is_platform_independent())
        {
            let emitted = emit_update_component(component, &package_dir, &components_dir, &paths)?;
            tracing::info!(
                path = %emitted.path.display(),
                sha256 = %emitted.sha256,
                "emitted update component"
            );
        }
    }

    // The bundle is the macOS staged layout, so it is assembled even when no
    // disk image is requested.
    let staged = if paths.target_triple.contains("apple-darwin") {
        assemble_macos_app_bundle(&paths, &package_dir)?
    } else {
        package_dir
    };

    // The engine component is emitted last on macOS because `Info.plist`,
    // `PkgInfo` and the icon only exist once the bundle has been assembled.
    if options.components.includes_engine() {
        let emitted = emit_update_component(
            components::ComponentId::Engine,
            &staged,
            &components_dir,
            &paths,
        )?;
        tracing::info!(
            path = %emitted.path.display(),
            sha256 = %emitted.sha256,
            "emitted update component"
        );
    }

    match output {
        PackageOutput::StagedOnly => {
            tracing::info!(path = %staged.display(), "staged Rust port without an archive");
        }
        PackageOutput::DiskImage => {
            let image = create_dmg(&paths, &staged)?;
            tracing::info!(path = %image.display(), "packaged Rust port");
        }
        PackageOutput::Archive => {
            let archive = create_archive(&paths, &staged)?;
            tracing::info!(path = %archive.display(), "packaged Rust port");
        }
    }
    Ok(())
}

fn build_runtime_binaries(paths: &WorkspacePaths) -> Result<()> {
    if paths.target_triple == MACOS_UNIVERSAL_TRIPLE {
        return build_universal_macos_binaries(paths);
    }
    cargo_build_runtime(paths, None)
}

/// One `cargo build --release` of the two shipped executables.
///
/// `target` is passed on the command line rather than through the environment
/// so a single run can build both macOS architectures; when it is `None` the
/// inherited `CARGO_BUILD_TARGET` still decides, exactly as before.
fn cargo_build_runtime(paths: &WorkspacePaths, target: Option<&str>) -> Result<()> {
    tracing::info!(target, "building clonk-game and clonk-app (release)");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(&cargo);
    command.args([
        "build",
        "--release",
        "--locked",
        "-p",
        "clonk-game",
        "-p",
        "clonk-app",
    ]);
    if let Some(target) = target {
        command.args(["--target", target]);
    }
    let status = command
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

/// Builds every macOS architecture and fuses each executable into one fat file.
///
/// Everything downstream — the audit, the staged layout, the bundle, the engine
/// component, the disk image — then reads a single `release_dir`, so exactly
/// one `.app` is assembled and signed.
fn build_universal_macos_binaries(paths: &WorkspacePaths) -> Result<()> {
    for arch in MACOS_UNIVERSAL_ARCHES {
        cargo_build_runtime(paths, Some(arch))?;
    }
    fs::create_dir_all(&paths.release_dir)
        .with_context(|| format!("failed to create {}", paths.release_dir.display()))?;
    for binary_name in RUNTIME_BINARIES {
        lipo_create(
            &macos_universal_slices(paths, binary_name),
            &paths.release_dir.join(binary_name),
        )?;
    }
    Ok(())
}

/// Where cargo left each architecture's build of one executable.
///
/// Explicitly per-target, never `target/release`: a slice read from the host's
/// default output directory would be whichever architecture was built last, and
/// `lipo` would happily fuse a binary with itself.
fn macos_universal_slices(paths: &WorkspacePaths, binary_name: &str) -> Vec<PathBuf> {
    MACOS_UNIVERSAL_ARCHES
        .iter()
        .map(|arch| {
            paths
                .target_dir
                .join(arch)
                .join("release")
                .join(binary_name)
        })
        .collect()
}

/// Fuses architecture slices into one universal executable.
///
/// The result carries no valid signature: the linker ad-hoc signs each slice,
/// and `lipo` rebuilds the file around them. That is why signing has to happen
/// after this, on the bundle these binaries end up in.
fn lipo_create(slices: &[PathBuf], destination: &Path) -> Result<()> {
    if let Some(missing) = slices.iter().find(|slice| !slice.exists()) {
        bail!(
            "expected a macOS architecture slice at {}; `rustup target add` each of {}",
            missing.display(),
            MACOS_UNIVERSAL_ARCHES.join(", ")
        );
    }
    let status = Command::new("lipo")
        .arg("-create")
        .args(slices)
        .arg("-output")
        .arg(destination)
        .status()
        .with_context(|| format!("failed to invoke lipo for {}", destination.display()))?;
    if !status.success() {
        bail!(
            "lipo -create failed for {} with status {:?}",
            destination.display(),
            status.code()
        );
    }
    set_executable(destination)?;
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

    for binary_name in RUNTIME_BINARIES {
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
        &paths.repo_root.join("README.md"),
        &package_dir.join("README.md"),
    )?;
    copy_file(
        &paths.repo_root.join("credits.txt"),
        &package_dir.join("credits.txt"),
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
    for binary_name in RUNTIME_BINARIES {
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

    sign_macos_bundle(&app_dir, &macos_dir)?;

    Ok(app_dir)
}

/// Ad-hoc sign the finished bundle and prove the signature validates.
///
/// The linker already ad-hoc signs each executable, but a `.app` whose
/// `Contents/_CodeSignature` is absent fails validation as a bundle, and macOS
/// reports a quarantined copy of it as "damaged and can't be opened" rather
/// than as merely unsigned. Signing seals `Info.plist` and `Resources`.
///
/// This is not a substitute for Developer ID signing and notarization: the
/// download still needs the quarantine flag cleared before it will launch.
fn sign_macos_bundle(app_dir: &Path, macos_dir: &Path) -> Result<()> {
    // The launcher is nested code and must be signed before the bundle that
    // seals it; `clonk-app` is the bundle executable and is covered below.
    codesign(&["--force", "--sign", "-"], &macos_dir.join("clonk-game"))?;
    codesign(&["--force", "--sign", "-"], app_dir)?;
    // Packaging must fail loudly rather than ship an unopenable bundle.
    codesign(&["--verify", "--deep", "--strict"], app_dir)
        .context("the packaged application bundle does not carry a valid signature")?;
    Ok(())
}

fn codesign(arguments: &[&str], target: &Path) -> Result<()> {
    let status = Command::new("codesign")
        .args(arguments)
        .arg(target)
        .status()
        .with_context(|| format!("failed to invoke codesign for {}", target.display()))?;
    if !status.success() {
        bail!(
            "codesign {} failed for {} with status {:?}",
            arguments.join(" "),
            target.display(),
            status.code()
        );
    }
    Ok(())
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

/// The drag-to-Applications shortcut a disk image opens with.
///
/// Only ever reached on macOS — `hdiutil` exists nowhere else — but this tool
/// now also *compiles* on Windows, where the release runs natively, and
/// `std::os::unix` does not exist there.
#[cfg(unix)]
fn link_applications_shortcut(staging: &Path) -> Result<()> {
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .context("failed to create the /Applications shortcut")
}

#[cfg(not(unix))]
fn link_applications_shortcut(_staging: &Path) -> Result<()> {
    bail!("macOS disk images can only be built on a unix host")
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
    link_applications_shortcut(&staging)?;

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
        // APFS preserves filenames byte-for-byte. HFS+ normalizes Unicode, so
        // content paths such as `Überladungen.c4d` would arrive under a
        // different encoding than the code signature sealed, and every such
        // resource would read back as missing.
        .args(["-fs", "APFS", "-format", "UDZO", "-quiet"])
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

    let base_name = package_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "package".to_string());

    write_deterministic_zip(
        &archive_path,
        package_dir,
        Some(&base_name),
        &|_| true,
        &executable_bit_for_bin_directory,
        true,
    )?;
    Ok(archive_path)
}

/// Emits one update component archive from an already-staged layout.
///
/// On macOS the engine component is taken from the assembled `.app`, whose
/// `Contents/Resources` also holds the shared components and whose
/// `_CodeSignature` seals whatever is present — neither belongs in an engine
/// archive, and the seal would be stale by the time a client applied it.
fn emit_update_component(
    component: components::ComponentId,
    source_dir: &Path,
    output_dir: &Path,
    paths: &WorkspacePaths,
) -> Result<components::EmittedComponent> {
    let is_bundle = component == components::ComponentId::Engine
        && paths.target_triple.contains("apple-darwin");

    let write = |archive: &Path, root: &Path, include: &dyn Fn(&Path) -> bool| -> Result<()> {
        let combined = |relative: &Path| -> bool {
            // Inside a bundle the only top-level entry is `Contents`, which the
            // staged-layout predicate does not recognise; membership is defined
            // by exclusion there instead.
            if is_bundle {
                return !bundle_path_belongs_to_another_component(relative);
            }
            include(relative)
        };
        write_deterministic_zip(
            archive,
            root,
            None,
            &combined,
            &executable_bit_for_component,
            false,
        )
    };

    components::emit_component(
        component,
        source_dir,
        output_dir,
        env!("CARGO_PKG_VERSION"),
        &paths.target_triple,
        &write,
    )
}

/// Paths inside an assembled `.app` that the engine component must not carry.
fn bundle_path_belongs_to_another_component(relative: &Path) -> bool {
    let text = path_to_zip_string(relative);
    text.starts_with("Contents/Resources/planet/")
        || text.starts_with("Contents/Resources/content/")
        || text.starts_with("Contents/_CodeSignature/")
}

/// Executables live in `bin/` off the bundle and `Contents/MacOS/` inside it.
fn executable_bit_for_component(relative: &Path) -> u32 {
    let text = path_to_zip_string(relative);
    if text.starts_with("bin/") || text.starts_with("Contents/MacOS/") {
        0o755
    } else {
        0o644
    }
}

/// `bin/` ships executables; everything else is data.
fn executable_bit_for_bin_directory(relative: &Path) -> u32 {
    if relative.components().next().map(|c| c.as_os_str()) == Some(std::ffi::OsStr::new("bin")) {
        0o755
    } else {
        0o644
    }
}

/// Writes a byte-reproducible zip of `source_root`.
///
/// Every axis that could otherwise vary between builds is pinned here rather
/// than inherited from the environment: entry order, timestamps, permissions
/// and compression. Component archives are named after their own digest, so
/// any drift would defeat deduplication and re-upload hundreds of megabytes of
/// unchanged data.
///
/// `mode_for` receives the path relative to `source_root`, so it is unaffected
/// by whichever `entry_prefix` the caller chose.
fn write_deterministic_zip(
    archive_path: &Path,
    source_root: &Path,
    entry_prefix: Option<&str>,
    include: &dyn Fn(&Path) -> bool,
    mode_for: &dyn Fn(&Path) -> u32,
    include_directory_entries: bool,
) -> Result<()> {
    let file = File::create(archive_path)
        .with_context(|| format!("unable to create archive {}", archive_path.display()))?;
    let mut zip = ZipWriter::new(file);

    // `FileOptions::default()` reads the wall clock when `zip`'s `time` feature
    // is enabled, which any dependency could turn on through feature
    // unification. Release archives must not depend on that.
    let epoch = zip::DateTime::default();
    let dir_options = FileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(epoch)
        .unix_permissions(0o755);
    let file_options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(epoch);

    let entry_name = |relative: &Path| match entry_prefix {
        Some(prefix) => {
            let mut prefixed = PathBuf::from(prefix);
            prefixed.push(relative);
            path_to_zip_string(&prefixed)
        }
        None => path_to_zip_string(relative),
    };

    if include_directory_entries {
        if let Some(prefix) = entry_prefix {
            zip.add_directory(format!("{prefix}/"), dir_options)?;
        }
    }

    let mut entries = WalkDir::new(source_root)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| {
        entry
            .path()
            .strip_prefix(source_root)
            .map(&entry_name)
            .unwrap_or_default()
    });

    for entry in entries {
        let relative = entry.path().strip_prefix(source_root).unwrap();
        if relative.as_os_str().is_empty() || !include(relative) {
            continue;
        }
        let zip_path_str = entry_name(relative);

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            if include_directory_entries {
                zip.add_directory(format!("{zip_path_str}/"), dir_options)?;
            }
            continue;
        }

        if metadata.is_file() {
            let options = file_options.unix_permissions(mode_for(relative));
            zip.start_file(&zip_path_str, options)?;
            let mut src = File::open(entry.path())?;
            io::copy(&mut src, &mut zip)?;
        }
    }

    zip.finish()?;
    Ok(())
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
    for binary_name in RUNTIME_BINARIES {
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

/// The triple a packaging run names every artifact after.
///
/// A macOS host with no target named builds *both* architectures and fuses
/// them, so the run is not for the host triple at all. Naming an explicit
/// target keeps the single-architecture behaviour: cargo can only be asked for
/// one, and an artifact that claims to be fat when it is not would be served to
/// Macs it cannot run on.
fn packaging_triple(host_triple: &str, explicit_target: Option<&str>) -> String {
    explicit_target.map(str::to_string).unwrap_or_else(|| {
        if host_triple.contains("apple-darwin") {
            MACOS_UNIVERSAL_TRIPLE.to_string()
        } else {
            host_triple.to_string()
        }
    })
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
        let target_triple = packaging_triple(&host_triple, explicit_target.as_deref());
        if !target_triple
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            bail!(
                "CARGO_BUILD_TARGET must be a target triple suitable for an archive name, got `{target_triple}`"
            );
        }
        // A universal run owns a directory of its own, because `target/release`
        // holds whichever single architecture the host builds by default.
        let release_dir = explicit_target
            .as_deref()
            .or((target_triple == MACOS_UNIVERSAL_TRIPLE).then_some(MACOS_UNIVERSAL_TRIPLE))
            .map_or_else(
                || target_dir.join("release"),
                |directory| target_dir.join(directory).join("release"),
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

        for name in ["COPYING", "README.md", "credits.txt"] {
            write_fixture(&root.join(name), name.as_bytes());
        }
        write_fixture(&root.join("planet/System.c4g/C4.c"), b"system");
        write_fixture(&root.join("planet/Graphics.c4g/Logo.png"), b"logo");
        write_fixture(
            &root.join("content/THIRD_PARTY_GAME_CONTENT.md"),
            b"redistribution permission",
        );
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
    fn package_options_default_to_writing_an_archive() {
        assert_eq!(
            PackageOptions::parse(Vec::new().into_iter()).expect("no arguments parses"),
            PackageOptions {
                archive: true,
                components: ComponentSelection::None
            }
        );
    }

    #[test]
    fn package_options_can_stop_before_the_archive() {
        // The Windows installer wraps the staged layout directly, so building a
        // ~270 MB zip only to discard it is pure release latency.
        assert_eq!(
            PackageOptions::parse(["--no-archive".to_string()].into_iter())
                .expect("--no-archive parses"),
            PackageOptions {
                archive: false,
                components: ComponentSelection::None
            }
        );
    }

    #[test]
    fn component_selection_defaults_to_none_and_parses_each_level() {
        // A local `cargo xtask package` must stay exactly what it was; only
        // the release workflow asks for components.
        assert_eq!(
            PackageOptions::parse(Vec::new().into_iter())
                .expect("parses")
                .components,
            ComponentSelection::None
        );
        for (argument, expected) in [
            ("--components=none", ComponentSelection::None),
            ("--components=engine", ComponentSelection::Engine),
            ("--components=all", ComponentSelection::All),
        ] {
            assert_eq!(
                PackageOptions::parse([argument.to_string()].into_iter())
                    .expect("parses")
                    .components,
                expected,
                "{argument}"
            );
        }
    }

    #[test]
    fn only_the_all_selection_emits_the_shared_components() {
        // The shared archives are byte-identical everywhere, so exactly one
        // platform pass should pay ~265 MB to build them.
        assert!(!ComponentSelection::Engine.includes_shared());
        assert!(ComponentSelection::Engine.includes_engine());
        assert!(ComponentSelection::All.includes_shared());
        assert!(!ComponentSelection::None.includes_engine());
    }

    #[test]
    fn the_bundle_engine_component_excludes_data_and_the_stale_seal() {
        // `Contents/Resources` also holds the shared components, and
        // `_CodeSignature` seals whatever was present at packaging time — a
        // seal that is stale the moment a client applies an update.
        for excluded in [
            "Contents/Resources/planet/System.c4g/C4.c",
            "Contents/Resources/content/Objects.c4d/DefCore.txt",
            "Contents/_CodeSignature/CodeResources",
        ] {
            assert!(
                bundle_path_belongs_to_another_component(Path::new(excluded)),
                "{excluded} must not ride along in the engine component"
            );
        }
        for included in [
            "Contents/MacOS/clonk-app",
            "Contents/Info.plist",
            "Contents/PkgInfo",
            "Contents/Resources/ClonkRust.icns",
            "Contents/Resources/COPYING",
        ] {
            assert!(
                !bundle_path_belongs_to_another_component(Path::new(included)),
                "{included} belongs to the engine component"
            );
        }
    }

    #[test]
    fn executables_are_marked_in_both_the_flat_and_bundle_layouts() {
        assert_eq!(
            executable_bit_for_component(Path::new("bin/clonk-app")),
            0o755
        );
        assert_eq!(
            executable_bit_for_component(Path::new("Contents/MacOS/clonk-app")),
            0o755
        );
        assert_eq!(executable_bit_for_component(Path::new("COPYING")), 0o644);
        assert_eq!(
            executable_bit_for_component(Path::new("Contents/Resources/ClonkRust.icns")),
            0o644
        );
    }

    #[test]
    fn no_archive_stops_after_staging_on_every_platform() {
        // The macOS branch used to return before the `--no-archive` check ran,
        // so the flag was silently ignored and a disk image was always built.
        assert_eq!(
            package_output("aarch64-apple-darwin", false),
            PackageOutput::StagedOnly
        );
        assert_eq!(
            package_output("x86_64-pc-windows-gnu", false),
            PackageOutput::StagedOnly
        );
    }

    #[test]
    fn packaging_produces_a_disk_image_on_macos_and_an_archive_elsewhere() {
        assert_eq!(
            package_output("universal-apple-darwin", true),
            PackageOutput::DiskImage
        );
        assert_eq!(
            package_output("x86_64-apple-darwin", true),
            PackageOutput::DiskImage
        );
        assert_eq!(
            package_output("x86_64-unknown-linux-gnu", true),
            PackageOutput::Archive
        );
    }

    #[test]
    fn a_macos_host_packages_one_universal_build_for_both_architectures() {
        // Every artifact name — the disk image and the engine component alike —
        // is derived from this triple, so naming the run `universal-apple-darwin`
        // is what collapses the two macOS passes into one.
        for host in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
            assert_eq!(packaging_triple(host, None), "universal-apple-darwin");
        }
    }

    #[test]
    fn a_named_target_still_packages_only_that_architecture() {
        // The escape hatch: `CARGO_BUILD_TARGET` names one real triple, and a
        // build that was asked for one architecture must not claim to be fat.
        assert_eq!(
            packaging_triple("aarch64-apple-darwin", Some("x86_64-apple-darwin")),
            "x86_64-apple-darwin"
        );
        assert_eq!(
            packaging_triple("x86_64-unknown-linux-gnu", Some("x86_64-pc-windows-msvc")),
            "x86_64-pc-windows-msvc"
        );
    }

    #[test]
    fn a_non_macos_host_packages_for_itself() {
        // Only macOS has `lipo`; everywhere else the host triple is the whole
        // story and this must stay exactly what it was.
        assert_eq!(
            packaging_triple("x86_64-unknown-linux-gnu", None),
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            packaging_triple("x86_64-pc-windows-msvc", None),
            "x86_64-pc-windows-msvc"
        );
    }

    #[test]
    fn a_universal_build_is_fused_from_both_architecture_slices() {
        // Read from cargo's per-target directories, never `target/release`: a
        // slice picked up from the host's default output would be single-arch
        // and `lipo` would happily fuse a binary with itself.
        let (_temp, mut paths) = package_fixture();
        paths.target_triple = MACOS_UNIVERSAL_TRIPLE.to_string();

        assert_eq!(
            macos_universal_slices(&paths, "clonk-app"),
            [
                paths
                    .target_dir
                    .join("aarch64-apple-darwin/release/clonk-app"),
                paths
                    .target_dir
                    .join("x86_64-apple-darwin/release/clonk-app"),
            ]
        );
    }

    #[test]
    fn package_options_reject_unknown_arguments() {
        let error = PackageOptions::parse(["--zip-it".to_string()].into_iter())
            .expect_err("unknown arguments are rejected");
        assert!(
            error.to_string().contains("--zip-it"),
            "error names the offending argument: {error}"
        );
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
            PathBuf::from("README.md"),
            PathBuf::from("credits.txt"),
            PathBuf::from("content/THIRD_PARTY_GAME_CONTENT.md"),
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
    fn archive_entries_carry_a_pinned_timestamp() {
        // Reproducibility currently rests on `zip`'s `time` feature being off:
        // `FileOptions::default()` reads the wall clock when it is on, and the
        // repeatability test above cannot see the difference because DOS
        // timestamps have two-second granularity. Pin it so a version bump or
        // feature unification elsewhere in the workspace cannot silently make
        // releases non-reproducible.
        let (_temp, paths) = package_fixture();
        let package_dir = assemble_package_layout(&paths).expect("assemble package");
        let archive = create_archive(&paths, &package_dir).expect("create archive");

        let file = File::open(&archive).expect("open archive");
        let mut zip = zip::ZipArchive::new(file).expect("open zip");
        let epoch = zip::DateTime::default();
        for index in 0..zip.len() {
            let entry = zip.by_index(index).expect("read zip entry");
            let stamp = entry.last_modified();
            assert_eq!(
                (
                    stamp.year(),
                    stamp.month(),
                    stamp.day(),
                    stamp.hour(),
                    stamp.minute(),
                    stamp.second()
                ),
                (
                    epoch.year(),
                    epoch.month(),
                    epoch.day(),
                    epoch.hour(),
                    epoch.minute(),
                    epoch.second()
                ),
                "entry {} carries a wall-clock timestamp",
                entry.name()
            );
        }
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
    fn manifest_options_accept_the_arguments_the_release_workflow_passes() {
        assert_eq!(
            GenerateManifestOptions::parse(&[
                "--version".to_string(),
                "0.4.0".to_string(),
                "--released-at".to_string(),
                "2026-07-28T10:00:00Z".to_string(),
                "--components".to_string(),
                "components".to_string(),
                "--out-dir".to_string(),
                "release".to_string(),
                "--content-commit".to_string(),
                CONTENT_COMMIT_FIXTURE.to_string(),
                "--content-sha256".to_string(),
                CONTENT_SHA256_FIXTURE.to_string(),
                "--content-size".to_string(),
                "236117973".to_string(),
            ])
            .expect("the release workflow's arguments parse"),
            GenerateManifestOptions {
                version: "0.4.0".to_string(),
                released_at: "2026-07-28T10:00:00Z".to_string(),
                components_dir: PathBuf::from("components"),
                out_dir: PathBuf::from("release"),
                content_commit: CONTENT_COMMIT_FIXTURE.to_string(),
                content_sha256: CONTENT_SHA256_FIXTURE.to_string(),
                content_size: 236_117_973,
            }
        );
    }

    #[test]
    fn manifest_options_name_the_argument_they_are_missing() {
        // Defaulting any of these would publish a manifest that describes a
        // release other than the one that was built, and every client would
        // then act on it.
        let error = GenerateManifestOptions::parse(&[
            "--version".to_string(),
            "0.4.0".to_string(),
            "--components".to_string(),
            "components".to_string(),
            "--out-dir".to_string(),
            "release".to_string(),
        ])
        .expect_err("a manifest without a release timestamp must not be generated");
        assert!(
            error.to_string().contains("--released-at"),
            "error names the missing argument: {error}"
        );
    }

    #[test]
    fn a_manifest_without_the_published_content_release_is_refused() {
        // The content archive is not in the components directory to be found,
        // so nothing else would notice it missing: the manifest would simply
        // ship without game data and every client would stop receiving it.
        for missing in ["--content-commit", "--content-sha256", "--content-size"] {
            let arguments: Vec<String> = [
                "--version",
                "0.4.0",
                "--released-at",
                "2026-07-28T10:00:00Z",
                "--components",
                "components",
                "--out-dir",
                "release",
                "--content-commit",
                CONTENT_COMMIT_FIXTURE,
                "--content-sha256",
                CONTENT_SHA256_FIXTURE,
                "--content-size",
                "236117973",
            ]
            .chunks(2)
            .filter(|pair| pair[0] != missing)
            .flatten()
            .map(|argument| argument.to_string())
            .collect();

            let error = GenerateManifestOptions::parse(&arguments)
                .expect_err("a manifest that cannot reach content must not be generated");
            assert!(
                error.to_string().contains(missing),
                "error names the missing argument: {error}"
            );
        }
    }

    #[test]
    fn a_content_release_that_cannot_be_verified_is_refused() {
        // The recorded digest is the whole integrity story for 225 MB with no
        // signature behind it. A malformed one fails every client *after* the
        // download, so it has to fail here instead.
        for (commit, sha256, size) in [
            ("not-a-commit", CONTENT_SHA256_FIXTURE, "236117973"),
            (CONTENT_COMMIT_FIXTURE, "abc", "236117973"),
            (CONTENT_COMMIT_FIXTURE, &CONTENT_SHA256_FIXTURE[..63], "1"),
            (CONTENT_COMMIT_FIXTURE, CONTENT_SHA256_FIXTURE, "0"),
            (CONTENT_COMMIT_FIXTURE, CONTENT_SHA256_FIXTURE, "huge"),
        ] {
            assert!(
                GenerateManifestOptions::parse(&[
                    "--version".to_string(),
                    "0.4.0".to_string(),
                    "--released-at".to_string(),
                    "2026-07-28T10:00:00Z".to_string(),
                    "--components".to_string(),
                    "components".to_string(),
                    "--out-dir".to_string(),
                    "release".to_string(),
                    "--content-commit".to_string(),
                    commit.to_string(),
                    "--content-sha256".to_string(),
                    sha256.to_string(),
                    "--content-size".to_string(),
                    size.to_string(),
                ])
                .is_err(),
                "commit {commit:?} / digest {sha256:?} / size {size:?} must be refused"
            );
        }
    }

    #[test]
    fn hex_arguments_are_lowercase_or_refused() {
        // Both are lowercase hex or nothing, and for the same reason: each is
        // compared as text somewhere that will not case-fold it. The pin
        // becomes the release tag `content-<sha>`, which GitHub matches byte
        // for byte, so an uppercase pin would pass validation and then resolve
        // to no release at all; the digest is compared against the client's own
        // lowercase rendering. Git and `sha256sum` only ever emit lowercase, so
        // nothing legitimate is turned away.
        for (commit, sha256) in [
            (
                CONTENT_COMMIT_FIXTURE.to_uppercase(),
                CONTENT_SHA256_FIXTURE.to_string(),
            ),
            (
                CONTENT_COMMIT_FIXTURE.to_string(),
                CONTENT_SHA256_FIXTURE.to_uppercase(),
            ),
        ] {
            assert!(
                GenerateManifestOptions::parse(&[
                    "--version".to_string(),
                    "0.4.0".to_string(),
                    "--released-at".to_string(),
                    "2026-07-28T10:00:00Z".to_string(),
                    "--components".to_string(),
                    "components".to_string(),
                    "--out-dir".to_string(),
                    "release".to_string(),
                    "--content-commit".to_string(),
                    commit.clone(),
                    "--content-sha256".to_string(),
                    sha256.clone(),
                    "--content-size".to_string(),
                    "236117973".to_string(),
                ])
                .is_err(),
                "uppercase hex must be refused: commit {commit:?} / digest {sha256:?}"
            );
        }
    }

    /// The four archives a complete release run leaves in
    /// `target/dist/components`: one engine build per shipped *build*, plus
    /// `planet`.
    ///
    /// `content` is absent because this repository no longer builds it — it is
    /// referenced from the content repository's own release instead.
    ///
    /// Three engine archives for five shipped triples: macOS ships one
    /// universal build for both of its triples, and Windows serves its retired
    /// gnu triple from the msvc archive. See [`UPDATE_TRIPLE_ALIASES`].
    ///
    /// Every archive carries different bytes, so a manifest that mixed two of
    /// them up cannot pass by coincidence.
    fn release_components_fixture() -> TempDir {
        let temp = TempDir::new().expect("temporary components directory");
        for (name, contents) in RELEASE_COMPONENTS_FIXTURE {
            write_fixture(&temp.path().join(name), contents.as_bytes());
        }
        temp
    }

    /// `(archive name, bytes)`. The digests asserted below were taken from
    /// `shasum -a 256` over these exact strings, not from the code under test.
    const RELEASE_COMPONENTS_FIXTURE: [(&str, &str); 4] = [
        (
            "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip",
            "linux engine",
        ),
        (
            "clonk-rust-0.4.0-engine-x86_64-pc-windows-msvc.zip",
            "windows engine",
        ),
        (
            "clonk-rust-0.4.0-engine-universal-apple-darwin.zip",
            "universal macos engine",
        ),
        (
            "planet-ffeeddccbbaa99887766554433221100.zip",
            "shared planet",
        ),
    ];

    /// The `content` submodule pin the publishing job would resolve.
    const CONTENT_COMMIT_FIXTURE: &str = "d34d385591134ce6c262b8c9ed53faaa6229cc6b";
    /// The digest that commit's content release published.
    const CONTENT_SHA256_FIXTURE: &str =
        "9cf12dcd98c461a96039ca6ed9be926301ddaf457d572b8a82981fe567819c2b";

    /// The options the publishing job passes, with the content release the
    /// submodule pin resolves to.
    fn manifest_options(components_dir: &Path, out_dir: &Path) -> GenerateManifestOptions {
        GenerateManifestOptions {
            version: "0.4.0".to_string(),
            released_at: "2026-07-28T10:00:00Z".to_string(),
            components_dir: components_dir.to_path_buf(),
            out_dir: out_dir.to_path_buf(),
            content_commit: CONTENT_COMMIT_FIXTURE.to_string(),
            content_sha256: CONTENT_SHA256_FIXTURE.to_string(),
            content_size: 236_117_973,
        }
    }

    /// Generates a manifest from `components` and reads back the published
    /// bytes, so the assertions are about the document a client downloads.
    fn generated_manifest(components: &TempDir) -> manifest::Manifest {
        let out = TempDir::new().expect("temporary output directory");
        // Nested, because the publishing job writes the manifest beside assets
        // it has only just downloaded.
        let out_dir = out.path().join("release");
        generate_update_manifest(manifest_options(components.path(), &out_dir))
            .expect("generate the manifest");

        let bytes = fs::read(out_dir.join("manifest.json")).expect("read the manifest");
        serde_json::from_slice(&bytes).expect("the manifest parses")
    }

    fn component_entry<'a>(
        manifest: &'a manifest::Manifest,
        name: &str,
    ) -> &'a manifest::ComponentEntry {
        manifest
            .components
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("manifest has no `{name}` component"))
    }

    #[test]
    fn every_triple_resolves_to_its_own_engine_archive() {
        // The reason the manifest is keyed by triple at all: three different
        // engine builds ship under one component, and a client has nothing but
        // this map to tell which of them is its own.
        let components = release_components_fixture();
        let engine = component_entry(&generated_manifest(&components), "engine").clone();

        for (triple, expected) in [
            (
                "x86_64-unknown-linux-gnu",
                (
                    "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip",
                    "58779b29d498bd1ff0b984a31c41072c1dadb69af13f75fe9360f6c17d7c0b4e",
                ),
            ),
            (
                "x86_64-pc-windows-msvc",
                (
                    "clonk-rust-0.4.0-engine-x86_64-pc-windows-msvc.zip",
                    "4f5e971ed560a857e53dfa16525c9a7aeb58d02a61a18b75c403f1eae333b7dd",
                ),
            ),
            (
                "universal-apple-darwin",
                (
                    "clonk-rust-0.4.0-engine-universal-apple-darwin.zip",
                    "6da2dcb44809e63fda53cf66b9cd958585a9b6453b7b06e283c38cee2eed014a",
                ),
            ),
        ] {
            let target = engine
                .targets
                .get(triple)
                .unwrap_or_else(|| panic!("engine has no archive for {triple}"));
            assert_eq!(
                target.archive, expected.0,
                "wrong engine archive for {triple}"
            );
            // Known-answer digests: the manifest must record the hash of the
            // bytes on disk, not merely something self-consistent.
            assert_eq!(
                target.sha256, expected.1,
                "wrong engine digest for {triple}"
            );
            assert_eq!(
                target.install, "",
                "the engine archive carries its own layout"
            );
        }

        let digests: std::collections::BTreeSet<&str> = engine
            .targets
            .values()
            .map(|target| target.sha256.as_str())
            .collect();
        assert_eq!(
            digests.len(),
            3,
            "three separate builds must not share a digest"
        );
    }

    #[test]
    fn both_macos_triples_are_served_the_universal_engine_archive() {
        // macOS ships one universal `.app` for both architectures, but a binary
        // can only report the triple *cargo* built it for — `build.rs` reads
        // `TARGET`, and each slice of a `lipo` build is compiled for a real
        // arch triple. So no Mac ever asks for `universal-apple-darwin`: the
        // arm64 slice asks as `aarch64-apple-darwin` and the Intel slice as
        // `x86_64-apple-darwin`, both before *and* after they update.
        //
        // Losing either key would tell every Mac there is no update, for ever
        // and silently, because "nothing offered for my triple" is
        // indistinguishable from "up to date".
        let components = release_components_fixture();
        let engine = component_entry(&generated_manifest(&components), "engine").clone();

        for triple in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
            let target = engine
                .targets
                .get(triple)
                .unwrap_or_else(|| panic!("engine has no archive for {triple}"));
            assert_eq!(
                target.archive, "clonk-rust-0.4.0-engine-universal-apple-darwin.zip",
                "{triple} must be offered the universal build"
            );
            // The whole target, not just the name: a client verifies the digest
            // it was given, so an alias that agreed on the filename alone would
            // fail every Mac at the integrity check instead of at the manifest.
            assert_eq!(
                target, &engine.targets["universal-apple-darwin"],
                "{triple} must resolve to the same bytes, not merely the same name"
            );
            assert_eq!(
                target.install, "",
                "the engine archive carries its own layout"
            );
        }
    }

    #[test]
    fn a_windows_gnu_client_is_served_the_msvc_engine_archive() {
        // Windows shipped as `x86_64-pc-windows-gnu` — cross-built from Linux
        // with mingw — until the release moved to a native MSVC runner. A
        // client reports the triple it was *built* for, so dropping the gnu key
        // would leave every Windows install that already exists with no entry
        // for itself: told there is no update, for ever, and silently, because
        // "nothing offered for my triple" is indistinguishable from "up to
        // date". The `engine` component replaces the executables wholesale, so
        // handing those clients the MSVC archive is the migration itself.
        let components = release_components_fixture();
        let engine = component_entry(&generated_manifest(&components), "engine").clone();

        assert_eq!(
            engine.targets["x86_64-pc-windows-gnu"].archive,
            "clonk-rust-0.4.0-engine-x86_64-pc-windows-msvc.zip",
            "a gnu client must be offered the archive that replaced its build"
        );
        // The whole target, not just the name: a client verifies the digest it
        // was given, so an alias that agreed on the filename alone would fail
        // every gnu install at the integrity check instead of at the manifest.
        assert_eq!(
            engine.targets["x86_64-pc-windows-gnu"], engine.targets["x86_64-pc-windows-msvc"],
            "the retired triple must resolve to the same bytes, not merely the same name"
        );
    }

    #[test]
    fn content_is_referenced_from_the_repository_that_publishes_it() {
        // This repository no longer builds `content.zip`. It records where the
        // content repository published it, the digest that release declared,
        // and its size — so a client fetches the same bytes from the place they
        // were produced instead of a daily re-upload of unchanged data.
        let components = release_components_fixture();
        let content = component_entry(&generated_manifest(&components), "content").clone();

        let linux = &content.targets["x86_64-unknown-linux-gnu"];
        assert_eq!(linux.archive, "content.zip");
        let source = linux
            .source
            .as_ref()
            .expect("content must name the release that publishes it");
        assert_eq!(source.repo, "syb0rg/clonk-rs-content");
        // The tag names the exact commit the `content` submodule pins, so the
        // pin resolves to an artifact with no lookup table in between.
        assert_eq!(source.tag, format!("content-{CONTENT_COMMIT_FIXTURE}"));
        assert_eq!(linux.sha256, CONTENT_SHA256_FIXTURE);
        assert_eq!(linux.size, 236_117_973);
        assert_eq!(linux.install, "content");

        // Prefix-free and identical everywhere; only the destination differs.
        let macos = &content.targets["aarch64-apple-darwin"];
        assert_eq!(macos.source, linux.source);
        assert_eq!(macos.sha256, linux.sha256);
        assert_eq!(macos.install, "Contents/Resources/content");
    }

    #[test]
    fn only_this_repository_s_own_components_omit_a_source() {
        // Absence is the instruction to resolve against the clonk-rs release,
        // so an engine archive that grew one would send clients elsewhere.
        let components = release_components_fixture();
        let manifest = generated_manifest(&components);
        for name in ["engine", "planet"] {
            assert!(
                component_entry(&manifest, name)
                    .targets
                    .values()
                    .all(|target| target.source.is_none()),
                "{name} is built and published here"
            );
        }
    }

    #[test]
    fn a_content_archive_left_in_the_components_directory_is_refused() {
        // Uploading one here would restore the second builder this change
        // exists to remove, and the two would silently drift.
        let components = release_components_fixture();
        write_fixture(
            &components
                .path()
                .join("content-00112233445566778899aabb.zip"),
            b"stale content",
        );

        let error = scan_emitted_components(components.path(), "0.4.0")
            .expect_err("this repository must not publish a content archive");
        assert!(
            error.to_string().contains(CONTENT_REPOSITORY),
            "error points at the repository that owns content: {error}"
        );
    }

    #[test]
    fn shared_components_are_offered_to_every_triple_as_the_same_bytes() {
        // `planet` is prefix-free and identical everywhere; only where it
        // unpacks differs.
        let components = release_components_fixture();
        let manifest = generated_manifest(&components);

        let planet = component_entry(&manifest, "planet");
        assert_eq!(
            planet.targets["x86_64-unknown-linux-gnu"].sha256,
            "9d91f5cc39abfdeda7c0fc532504b8b9bdcb8db7d14aedeeb9489abfdbf1ecd9"
        );
        assert_eq!(planet.targets["x86_64-pc-windows-gnu"].install, "planet");
    }

    #[test]
    fn a_generated_manifest_carries_the_release_and_the_engine_tuple() {
        let components = release_components_fixture();
        let manifest = generated_manifest(&components);

        assert_eq!(manifest.schema, manifest::MANIFEST_SCHEMA);
        assert_eq!(manifest.version, "0.4.0");
        assert_eq!(manifest.released_at, "2026-07-28T10:00:00Z");
        // The C4XVer tuple, not the release version: a client whose engine is
        // older must refuse `content` rather than have its definitions pruned.
        assert_eq!(manifest.engine_version, clonk_core::version::ENGINE_VERSION);
        assert_eq!(
            manifest
                .components
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>(),
            ["content", "planet", "engine"],
            "data first, executables last"
        );
    }

    #[test]
    fn the_component_scan_ignores_everything_that_is_not_an_archive() {
        // The publishing job assembles release notes and checksums in the same
        // tree; only the zips are components.
        let components = release_components_fixture();
        write_fixture(&components.path().join("SHA256SUMS.txt"), b"sums");
        write_fixture(&components.path().join("release-notes.md"), b"notes");

        assert_eq!(
            scan_emitted_components(components.path(), "0.4.0")
                .expect("scan the components")
                .len(),
            4
        );
    }

    #[test]
    fn an_unrecognised_archive_stops_the_manifest() {
        // Publishing a manifest that omits a shipped component is worse than
        // failing: clients would never learn the component existed.
        let components = release_components_fixture();
        write_fixture(&components.path().join("shaders-0.4.0.zip"), b"new");

        let error = scan_emitted_components(components.path(), "0.4.0")
            .expect_err("an unmapped archive must fail the manifest");
        assert!(
            error.to_string().contains("shaders-0.4.0.zip"),
            "error names the offending archive: {error}"
        );
    }

    #[test]
    fn a_release_missing_one_platforms_engine_is_refused() {
        // The macOS build pass uploading nothing must fail the release, not
        // publish a manifest that tells every Mac there is no engine for it —
        // before the tag, which is what makes the failure recoverable.
        let components = TempDir::new().expect("temporary components directory");
        for (name, contents) in RELEASE_COMPONENTS_FIXTURE {
            if !name.contains("apple-darwin") {
                write_fixture(&components.path().join(name), contents.as_bytes());
            }
        }
        let out = TempDir::new().expect("temporary output directory");

        let error = generate_update_manifest(manifest_options(components.path(), out.path()))
            .expect_err("a manifest that strands a platform must not be published");

        // Every Mac triple, not just the one that was not built: the aliases
        // are what the two architecture triples resolve through, so a missing
        // universal archive strands all three at once.
        for triple in [
            "universal-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ] {
            assert!(
                error.to_string().contains(&format!("engine/{triple}")),
                "error names the component and triple that would be missing: {error}"
            );
        }
        assert!(
            !out.path().join("manifest.json").exists(),
            "nothing may be written when the manifest is refused"
        );
    }

    #[test]
    fn an_engine_archive_from_another_release_is_refused() {
        // Artifacts from a re-run or a previous version would name assets this
        // release never uploads; the client would fetch a 404.
        let components = release_components_fixture();
        write_fixture(
            &components
                .path()
                .join("clonk-rust-0.3.9-engine-x86_64-unknown-linux-gnu.zip"),
            b"stale engine",
        );

        let error = scan_emitted_components(components.path(), "0.4.0")
            .expect_err("an archive from another release must fail the manifest");
        assert!(
            error.to_string().contains("clonk-rust-0.3.9-engine"),
            "error names the stale archive: {error}"
        );
    }

    #[test]
    fn an_engine_archive_for_an_unshipped_triple_is_refused() {
        // Its triple appears under no other component, so a client reading it
        // could never assemble a complete install.
        let components = release_components_fixture();
        write_fixture(
            &components
                .path()
                .join("clonk-rust-0.4.0-engine-riscv64gc-unknown-linux-gnu.zip"),
            b"unshipped engine",
        );
        let out = TempDir::new().expect("temporary output directory");

        let error = generate_update_manifest(manifest_options(components.path(), out.path()))
            .expect_err("a triple the release does not ship must not reach the manifest");
        assert!(
            error.to_string().contains("riscv64gc-unknown-linux-gnu"),
            "error names the unshipped triple: {error}"
        );
    }

    #[test]
    fn an_empty_components_directory_is_refused() {
        // A manifest describing nothing reads to every client as "no update",
        // which is indistinguishable from a healthy release.
        let empty = TempDir::new().expect("temporary components directory");
        assert!(scan_emitted_components(empty.path(), "0.4.0").is_err());
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
            .args(["add", "planet", "content"])
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
    fn package_layout_rejects_modified_tracked_planet_bytes() {
        let (_temp, paths) = package_fixture();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&paths.repo_root)
            .status()
            .expect("run git init");
        assert!(init.success(), "git init failed");
        let add = Command::new("git")
            .args(["add", "planet", "content"])
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
