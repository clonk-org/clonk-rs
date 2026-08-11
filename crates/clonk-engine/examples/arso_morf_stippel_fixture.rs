//! Build a disposable Arso-Morf fixture with exactly 1,000 real ST5B objects.
//!
//! This executable refuses to operate without the marker created by
//! `scripts/run_arso_morf_stippel_gpu_benchmark.py`. It loads the copied
//! scenario and definitions through the normal engine, creates each added
//! Stippel through `Engine::spawn_object` (including `Initialize`), then uses
//! the live C4 serializer to update only the disposable copy.

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use clonk_engine::{
    Engine, LiveC4ComponentHost, LiveC4SavePolicy, LiveC4SaveSpec, Scenario, ScenarioError,
    SpawnConfig, Vector2,
};
use clonk_resources::{Group, MaterialLibrary};
use clonk_script::Value;

const STIPPEL_ID: &str = "ST5B";
const SOURCE_STIPPELS: usize = 20;
// The 1,061 serialized rows plus the scientist and key created by the normal
// scenario Initialize callback.
const SOURCE_OBJECTS: usize = 1_063;
const TARGET_STIPPELS: usize = 1_000;
const TARGET_OBJECTS: usize = SOURCE_OBJECTS + TARGET_STIPPELS - SOURCE_STIPPELS;
const DEFAULT_SEED: u64 = 424_242;
const FIXTURE_MARKER: &str = ".clonk-rs-disposable-stippel-benchmark";

#[derive(Clone, Copy)]
struct ObjectCensus {
    total: usize,
    stippels: usize,
    lifecycle_stippels: usize,
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

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn validate_disposable_path(scenario_path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let canonical = scenario_path.canonicalize()?;
    if !canonical.join(FIXTURE_MARKER).is_file() {
        return Err(invalid_data(format!(
            "refusing to modify `{}` without disposable fixture marker `{FIXTURE_MARKER}`",
            canonical.display()
        ))
        .into());
    }
    let installed_content = content_root().canonicalize()?;
    if canonical.starts_with(&installed_content) {
        return Err(invalid_data(format!(
            "refusing to modify checked-in content under `{}`",
            installed_content.display()
        ))
        .into());
    }
    Ok(canonical)
}

fn load_engine(scenario_path: &Path, seed: u64) -> Result<Engine, Box<dyn Error>> {
    let content = content_root();
    let content_install = content
        .parent()
        .ok_or_else(|| invalid_data("content root has no parent"))?;
    let bundled = repository_root();
    let scenario = Scenario::load_from_path_with_seed(
        scenario_path,
        &ContentResolver {
            roots: vec![bundled, content.clone()],
        },
        seed,
    )?;

    let material_group = Group::open(content.join("Material.c4g"))?;
    let materials = MaterialLibrary::from_group(&material_group)?;
    let system_group = Group::open(content_install.join("planet/System.c4g"))?;
    let system_scripts = load_system_scripts(&system_group)?;
    let standard_names = system_group
        .read_file("Names.txt")
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    let mut engine = Engine::with_seed(seed);
    engine.configure_materials_from_library(&materials);
    engine.install_global_scripts(&system_scripts);
    engine.set_standard_names(standard_names);
    scenario.apply(&mut engine)?;
    Ok(engine)
}

fn object_census(engine: &Engine) -> ObjectCensus {
    let snapshot = engine.snapshot();
    let stippels = snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == STIPPEL_ID)
        .collect::<Vec<_>>();
    ObjectCensus {
        total: snapshot.objects.len(),
        stippels: stippels.len(),
        lifecycle_stippels: stippels
            .iter()
            .filter(|object| {
                object
                    .effects
                    .iter()
                    .any(|effect| effect.name == "LifeCycle")
            })
            .count(),
    }
}

fn stippel_spawn_position(anchor: Vector2, occurrence: usize) -> Vector2 {
    // Fresh FullCon construction moves ST5B's center eight pixels upward.
    // Compensate so the serialized center stays on the checked-in ground band.
    const FRESH_CENTER_Y_ADJUSTMENT: i32 = 8;
    let offset = i32::try_from(occurrence).unwrap_or_default() - 24;
    let offset = if offset >= 0 { offset + 1 } else { offset };
    Vector2::new(anchor.x + offset, anchor.y + FRESH_CENTER_Y_ADJUSTMENT)
}

fn stippel_spawn_config(position: Vector2) -> SpawnConfig {
    // Preserve Initialize and the complete LifeCycle workload while giving the
    // dense fresh population enough initial grace to reach measurement start.
    // LifeCycle may reset this value and naturally remove stuck ST5Bs later.
    SpawnConfig::new(STIPPEL_ID)
        .with_position(position)
        .with_local_vars(HashMap::from([(
            "stuckTime".to_owned(),
            Value::Int(-1_000),
        )]))
}

fn set_initial_stippel_stuck_grace(engine: &mut Engine) -> Result<(), Box<dyn Error>> {
    let mut state = engine.capture_state();
    for object in &mut state.objects {
        if object.snapshot.definition_id == STIPPEL_ID {
            object
                .snapshot
                .local_vars
                .insert("stuckTime".to_owned(), Value::Int(-1_000));
        }
    }
    engine.restore_state(&state)?;
    Ok(())
}

fn populate_stippels(engine: &mut Engine) -> Result<(), Box<dyn Error>> {
    let anchors = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == STIPPEL_ID)
        .map(|object| object.position)
        .collect::<Vec<Vector2>>();
    if anchors.len() != SOURCE_STIPPELS {
        return Err(invalid_data(format!(
            "source fixture contains {} ST5B objects; expected {SOURCE_STIPPELS}",
            anchors.len()
        ))
        .into());
    }

    for index in anchors.len()..TARGET_STIPPELS {
        let anchor_index = index % anchors.len();
        let occurrence = (index - anchors.len()) / anchors.len();
        let position = stippel_spawn_position(anchors[anchor_index], occurrence);
        engine.spawn_object(stippel_spawn_config(position))?;
    }
    set_initial_stippel_stuck_grace(engine)?;
    Ok(())
}

fn count_serialized_rows(bytes: &[u8], row: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| line.strip_suffix(b"\r").unwrap_or(line) == row)
        .count()
}

fn write_fixture_components(
    scenario_path: &Path,
    objects_txt: &[u8],
    strings_txt: &[u8],
    game_txt: &[u8],
) -> Result<(), Box<dyn Error>> {
    fs::write(scenario_path.join("Objects.txt"), objects_txt)?;
    fs::write(scenario_path.join("Strings.txt"), strings_txt)?;
    fs::write(scenario_path.join("Game.txt"), game_txt)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let scenario_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| invalid_data("usage: arso_morf_stippel_fixture SCENARIO [SEED]"))?;
    let seed = args
        .next()
        .map(|value| value.to_string_lossy().parse::<u64>())
        .transpose()?
        .unwrap_or(DEFAULT_SEED);
    if args.next().is_some() {
        return Err(invalid_data("usage: arso_morf_stippel_fixture SCENARIO [SEED]").into());
    }
    let scenario_path = validate_disposable_path(&scenario_path)?;
    let mut engine = load_engine(&scenario_path, seed)?;
    let source = object_census(&engine);
    if source.total != SOURCE_OBJECTS
        || source.stippels != SOURCE_STIPPELS
        || source.lifecycle_stippels != SOURCE_STIPPELS
    {
        return Err(invalid_data(format!(
            "source Arso-Morf census drifted: objects={} ST5B={} ST5B-with-LifeCycle={}; expected objects={SOURCE_OBJECTS} ST5B={SOURCE_STIPPELS} ST5B-with-LifeCycle={SOURCE_STIPPELS}",
            source.total, source.stippels, source.lifecycle_stippels
        ))
        .into());
    }

    populate_stippels(&mut engine)?;
    let prepared = object_census(&engine);
    if prepared.total != TARGET_OBJECTS
        || prepared.stippels != TARGET_STIPPELS
        || prepared.lifecycle_stippels != TARGET_STIPPELS
    {
        return Err(invalid_data(format!(
            "prepared census is objects={} ST5B={} ST5B-with-LifeCycle={}; expected objects={TARGET_OBJECTS} ST5B={TARGET_STIPPELS} ST5B-with-LifeCycle={TARGET_STIPPELS}",
            prepared.total, prepared.stippels, prepared.lifecycle_stippels
        ))
        .into());
    }

    let definition_modules = vec!["Objects.c4d".to_owned(), "EkeReloaded.c4d".to_owned()];
    let components = engine.serialize_live_c4_save_with_policy(
        LiveC4SaveSpec {
            title: "Arso-Morf 1,000-ST5B GPU benchmark",
            definition_modules: &definition_modules,
            definition_executable_path: "",
            definition_path: "",
            origin: "",
            music_enabled: false,
            copied_material_group_is_file: scenario_path.join("Material.c4g").is_file(),
            title_component: LiveC4ComponentHost::Unmodified,
            info_component: LiveC4ComponentHost::Unmodified,
            script_component: LiveC4ComponentHost::Unmodified,
        },
        LiveC4SavePolicy::Scenario {
            force_exact_landscape: false,
        },
    )?;
    let serialized_stippels = count_serialized_rows(&components.objects_txt, b"id=ST5B");
    let serialized_objects = count_serialized_rows(&components.objects_txt, b"[Object]");
    if serialized_stippels != TARGET_STIPPELS || serialized_objects != TARGET_OBJECTS {
        return Err(invalid_data(format!(
            "serialized census is objects={serialized_objects} ST5B={serialized_stippels}; expected objects={TARGET_OBJECTS} ST5B={TARGET_STIPPELS}"
        ))
        .into());
    }
    let strings_txt = components.strings_txt.ok_or_else(|| {
        invalid_data("live serializer omitted Strings.txt required by saved object values")
    })?;
    write_fixture_components(
        &scenario_path,
        &components.objects_txt,
        &strings_txt,
        &components.game_txt,
    )?;

    println!(
        "LC_ARSO_MORF_STIPPEL_FIXTURE source_stippels={} prepared_stippels={} source_lifecycle_stippels={} prepared_lifecycle_stippels={} serialized_stippels={serialized_stippels} source_objects={} serialized_objects={serialized_objects} seed={seed}",
        source.stippels,
        prepared.stippels,
        source.lifecycle_stippels,
        prepared.lifecycle_stippels,
        source.total,
    );
    Ok(())
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
