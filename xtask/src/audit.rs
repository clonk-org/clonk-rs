//! `cargo xtask scenario-audit` — content-fidelity scoreboard.
//!
//! The scenario-sweep only proves load+apply do not ERROR; this audit checks
//! that the APPLIED world actually resembles what the C++ engine builds:
//! a real landscape (pixel grid, several materials), the scenario's initial
//! objects, and the `[Landscape] Vegetation=/InEarth=`, `[Animals]`,
//! `[Environment] Objects=`, `[Game] Goals=/Rules=` placements
//! (C4Game::InitVegetation/InitInEarth/InitAnimals/InitEnvironment/
//! InitRules/InitGoals, src/C4Game.cpp:2493-2503).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// What Scenario.txt promises the world will contain, parsed independently
/// of the engine loader so loader gaps cannot hide audit findings.
#[derive(Debug, Default, Clone)]
struct ScenarioExpectations {
    no_initialize: bool,
    vegetation: Vec<String>,
    vegetation_level_std: i32,
    in_earth: Vec<String>,
    in_earth_level_std: i32,
    animals: Vec<String>,
    nests: Vec<String>,
    environment_objects: Vec<String>,
    goals: Vec<String>,
    rules: Vec<String>,
    /// Landscape source: `static` (Map.bmp/Landscape.bmp), `exmap`
    /// (Landscape.txt), `dynamic` (C4MapCreator by [Landscape] keys) or
    /// `exact` (ExactLandscape).
    landscape_source: &'static str,
}

/// C4IDList entries with count > 0 — zero-weight entries never place
/// anything in C++ (ListExpandValids repeats each id `count` times,
/// C4Game.cpp:2929-2947), so they carry no expectation.
fn parse_id_list(raw: &str) -> Vec<String> {
    raw.split(';')
        .filter_map(|token| {
            let mut parts = token.trim().splitn(2, '=');
            let id = parts.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let count: i32 = parts
                .next()
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
            (count > 0).then(|| id.to_ascii_uppercase())
        })
        .collect()
}

fn parse_scenario_txt(text: &str) -> BTreeMap<String, Vec<(String, String)>> {
    let mut sections: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_ascii_lowercase();
            sections.entry(current.clone()).or_default();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            sections
                .entry(current.clone())
                .or_default()
                .push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    sections
}

fn section_value<'a>(
    sections: &'a BTreeMap<String, Vec<(String, String)>>,
    section: &str,
    key: &str,
) -> Option<&'a str> {
    sections
        .get(section)?
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn expectations_for(path: &Path) -> Result<ScenarioExpectations> {
    let text = std::fs::read(path.join("Scenario.txt"))
        .with_context(|| format!("reading {}/Scenario.txt", path.display()))?;
    let text = String::from_utf8_lossy(&text);
    let sections = parse_scenario_txt(&text);

    let c4sval_std = |raw: Option<&str>, default: i32| -> i32 {
        raw.and_then(|raw| raw.split(',').next())
            .and_then(|std| std.trim().parse().ok())
            .unwrap_or(default)
    };

    let exact = section_value(&sections, "landscape", "exactlandscape")
        .map(|v| v.trim() != "0")
        .unwrap_or(false);
    let landscape_source = if exact {
        "exact"
    } else if path.join("Map.bmp").exists() || path.join("Landscape.bmp").exists() {
        "static"
    } else if path.join("Landscape.txt").exists() {
        "exmap"
    } else {
        "dynamic"
    };

    Ok(ScenarioExpectations {
        no_initialize: section_value(&sections, "head", "noinitialize")
            .map(|v| v.trim() != "0")
            .unwrap_or(false),
        vegetation: parse_id_list(section_value(&sections, "landscape", "vegetation").unwrap_or("")),
        // C4SVal(50, 30, 0, 100) — C4Scenario.cpp:340.
        vegetation_level_std: c4sval_std(
            section_value(&sections, "landscape", "vegetationlevel"),
            50,
        ),
        in_earth: parse_id_list(section_value(&sections, "landscape", "inearth").unwrap_or("")),
        // C4SVal(50, 0, 0, 100) — C4Scenario.cpp:342.
        in_earth_level_std: c4sval_std(section_value(&sections, "landscape", "inearthlevel"), 50),
        animals: parse_id_list(section_value(&sections, "animals", "animal").unwrap_or("")),
        nests: parse_id_list(section_value(&sections, "animals", "nest").unwrap_or("")),
        environment_objects: parse_id_list(
            section_value(&sections, "environment", "objects").unwrap_or(""),
        ),
        goals: parse_id_list(section_value(&sections, "game", "goals").unwrap_or("")),
        // InitRules places max(count, 1) per listed rule (C4Game.cpp:4004)
        // — zero-weight rule entries still place one.
        rules: section_value(&sections, "game", "rules")
            .unwrap_or("")
            .split(';')
            .filter_map(|token| token.trim().split('=').next())
            .filter(|id| !id.is_empty())
            .map(|id| id.to_ascii_uppercase())
            .collect(),
        landscape_source,
    })
}

/// The applied world, measured through the engine's public surface.
#[derive(Debug, Default, Clone)]
struct WorldReport {
    landscape: Option<(u32, u32)>,
    pixel_grid: bool,
    /// (material name, pixel count) sorted by count, descending; sky
    /// (density 0 slots / byte 0) excluded.
    materials: Vec<(String, u64)>,
    sky: bool,
    objects_by_def: BTreeMap<String, usize>,
    /// Registered definition ids — an expectation on an id the loader
    /// never registered is a C++-consistent miss (C4Id2Def -> nullptr),
    /// not a placement gap.
    known_defs: std::collections::BTreeSet<String>,
    error: Option<String>,
}

fn measure_world(
    scenario_path: &Path,
    repo_root: &Path,
    content_root: &Path,
) -> Result<WorldReport> {
    let global_material_library = clonk_resources::MaterialLibrary::from_group(
        &clonk_resources::Group::open(content_root.join("Material.c4g"))
            .context("opening content/Material.c4g")?,
    )
    .map_err(|error| anyhow!("loading material library: {error}"))?;
    let local_material_library = clonk_resources::Group::open(scenario_path.join("Material.c4g"))
        .ok()
        .and_then(|group| clonk_resources::MaterialLibrary::from_group(&group).ok());
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

    let roots = vec![content_root.to_path_buf(), repo_root.to_path_buf()];
    let resolver = crate::SweepResolver { roots };

    let scenario = clonk_engine::Scenario::load_from_path_with(scenario_path, &resolver)
        .map_err(|error| anyhow!("load failed: {error}"))?;
    let mut engine = clonk_engine::Engine::new();
    engine.configure_materials_from_library(&material_library);
    engine.install_global_scripts(&system_scripts);
    scenario
        .apply(&mut engine)
        .map_err(|error| anyhow!("apply failed: {error}"))?;

    let mut report = WorldReport {
        sky: scenario.sky().is_some(),
        ..WorldReport::default()
    };
    if let Some(landscape) = engine.landscape() {
        report.landscape = Some((landscape.width(), landscape.estimated_height().max(0) as u32));
        if let Some(grid) = landscape.pixel_grid() {
            report.pixel_grid = true;
            let mut by_slot = [0u64; 128];
            for &byte in grid.bytes() {
                by_slot[(byte & 0x7f) as usize] += 1;
            }
            let names = grid.material_names();
            let mut by_material: BTreeMap<String, u64> = BTreeMap::new();
            for (slot, &count) in by_slot.iter().enumerate().skip(1) {
                if count == 0 {
                    continue;
                }
                let name = names
                    .get(slot)
                    .and_then(|name| name.clone())
                    .unwrap_or_else(|| format!("slot{slot}"));
                *by_material.entry(name).or_default() += count;
            }
            let mut materials: Vec<(String, u64)> = by_material.into_iter().collect();
            materials.sort_by(|a, b| b.1.cmp(&a.1));
            report.materials = materials;
        }
    }
    for object in engine.snapshot().objects {
        *report
            .objects_by_def
            .entry(object.definition_id.clone())
            .or_default() += 1;
    }
    report.known_defs = engine.definition_ids().map(str::to_string).collect();
    Ok(report)
}

fn flags_for(expect: &ScenarioExpectations, world: &WorldReport) -> Vec<String> {
    let mut flags = Vec::new();
    if world.error.is_some() {
        flags.push("apply-error".to_string());
        return flags;
    }
    if world.landscape.is_none() {
        flags.push("no-landscape".to_string());
    }
    if !world.pixel_grid {
        flags.push("no-pixel-grid".to_string());
    }
    // A generated/static world with at most one material is the flat
    // placeholder, not a real map (exact landscapes excluded: they load
    // the pixel plane directly).
    if world.pixel_grid && world.materials.len() <= 1 && expect.landscape_source != "exact" {
        flags.push("single-material".to_string());
    }
    if world.objects_by_def.is_empty() {
        flags.push("no-objects".to_string());
    }
    // A placement expectation only stands when at least one listed id is
    // a REGISTERED definition (C4Id2Def semantics); ids absent from the
    // loaded packs fail in C++ too.
    let has_any = |ids: &[String]| {
        let known: Vec<&String> = ids.iter().filter(|id| world.known_defs.contains(*id)).collect();
        known.is_empty()
            || known
                .iter()
                .any(|id| world.objects_by_def.get(*id).copied().unwrap_or(0) > 0)
    };
    if !expect.no_initialize {
        if !expect.vegetation.is_empty()
            && expect.vegetation_level_std > 0
            && !has_any(&expect.vegetation)
        {
            flags.push("veg-missing".to_string());
        }
        if !expect.in_earth.is_empty()
            && expect.in_earth_level_std > 0
            && !has_any(&expect.in_earth)
        {
            flags.push("inearth-missing".to_string());
        }
        if !expect.animals.is_empty() && !has_any(&expect.animals) {
            flags.push("animals-missing".to_string());
        }
        if !expect.nests.is_empty() && !has_any(&expect.nests) {
            flags.push("nests-missing".to_string());
        }
        if !expect.environment_objects.is_empty() && !has_any(&expect.environment_objects) {
            flags.push("environment-missing".to_string());
        }
        if !expect.goals.is_empty() && !has_any(&expect.goals) {
            flags.push("goals-missing".to_string());
        }
        if !expect.rules.is_empty() && !has_any(&expect.rules) {
            flags.push("rules-missing".to_string());
        }
    }
    flags
}

pub fn scenario_audit_command(args: &[String]) -> Result<()> {
    let mut filter: Option<String> = None;
    let mut verbose = false;
    for arg in args {
        match arg.as_str() {
            "--verbose" | "-v" => verbose = true,
            "--help" | "-h" => {
                tracing::info!(
                    "Usage: cargo xtask scenario-audit [filter] [--verbose]\n  Loads + applies content/**/*.c4s and audits world fidelity (landscape materials, objects, vegetation/in-earth/animal/environment/goal/rule placement)."
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
    if scenario_paths.is_empty() {
        bail!("no scenario matches the filter");
    }

    let mut rows: Vec<(String, ScenarioExpectations, WorldReport)> = Vec::new();
    for path in &scenario_paths {
        let label = path
            .strip_prefix(&content_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let expect = expectations_for(path)?;

        // Watchdog thread, like scenario-sweep: a hang is a finding, not a
        // harness failure.
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_path = path.clone();
        let worker_repo = repo_root.clone();
        let worker_content = content_root.clone();
        std::thread::spawn(move || {
            let report = measure_world(&worker_path, &worker_repo, &worker_content)
                .unwrap_or_else(|error| WorldReport {
                    error: Some(error.to_string()),
                    ..WorldReport::default()
                });
            let _ = sender.send(report);
        });
        let world = receiver
            .recv_timeout(std::time::Duration::from_secs(180))
            .unwrap_or_else(|_| WorldReport {
                error: Some("HUNG: apply did not finish within 180s".to_string()),
                ..WorldReport::default()
            });
        rows.push((label, expect, world));
    }

    // Detail lines (single scenario or --verbose).
    let detail = verbose || rows.len() == 1;
    let mut report = String::new();
    let mut flag_totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut flagged: Vec<(usize, String)> = Vec::new();
    for (label, expect, world) in &rows {
        let flags = flags_for(expect, world);
        for flag in &flags {
            *flag_totals.entry(flag.clone()).or_default() += 1;
        }
        if detail {
            report.push_str(&format!(
                "\n=== {label} [{}{}]\n",
                expect.landscape_source,
                if expect.no_initialize {
                    ", NoInitialize"
                } else {
                    ""
                }
            ));
            match (&world.error, world.landscape) {
                (Some(error), _) => report.push_str(&format!("  ERROR: {error}\n")),
                (None, Some((width, height))) => {
                    let materials: Vec<String> = world
                        .materials
                        .iter()
                        .take(8)
                        .map(|(name, count)| {
                            let pct = *count as f64 * 100.0 / (width as f64 * height as f64);
                            format!("{name} {pct:.1}%")
                        })
                        .collect();
                    report.push_str(&format!(
                        "  landscape: {width}x{height} pixel-grid={} materials({}): {}\n",
                        world.pixel_grid,
                        world.materials.len(),
                        materials.join(", ")
                    ));
                }
                (None, None) => report.push_str("  landscape: NONE\n"),
            }
            let total: usize = world.objects_by_def.values().sum();
            let mut defs: Vec<(&String, &usize)> = world.objects_by_def.iter().collect();
            defs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let listing: Vec<String> = defs
                .iter()
                .take(16)
                .map(|(id, count)| format!("{id}={count}"))
                .collect();
            report.push_str(&format!(
                "  objects: {total} — {}{}\n  sky: {}\n",
                listing.join(" "),
                if defs.len() > 16 { " …" } else { "" },
                world.sky
            ));
            if !flags.is_empty() {
                report.push_str(&format!("  FLAGS: {}\n", flags.join(", ")));
            }
        }
        if !flags.is_empty() {
            flagged.push((flags.len(), format!("{label}: {}", flags.join(", "))));
        }
    }

    report.push_str(&format!(
        "\nscenario audit: {} scenarios, {} flagged\n",
        rows.len(),
        flagged.len()
    ));
    if !flag_totals.is_empty() {
        report.push_str("flag totals:\n");
        let mut totals: Vec<_> = flag_totals.into_iter().collect();
        totals.sort_by(|a, b| b.1.cmp(&a.1));
        for (flag, count) in totals {
            report.push_str(&format!("  {count:3}x {flag}\n"));
        }
        flagged.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        report.push_str("worst offenders:\n");
        for (_, line) in flagged.iter().take(25) {
            report.push_str(&format!("  {line}\n"));
        }
    }
    tracing::info!("{report}");
    Ok(())
}
