//! Deterministic forced-volcano profiler for the real Deep Sea FXV1 script.
//!
//! This keeps the shipped script and landscape in the loop while forcing the
//! weather-created object, so a profile does not depend on waiting for the
//! random weather trigger. It is a measurement tool only: the final state is
//! printed as a checksum and no simulation behavior is changed.

use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clonk_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
use clonk_engine::{Engine, Scenario, ScenarioError, SpawnConfig, Vector2};
use clonk_resources::{Group, MaterialLibrary};
use clonk_script::Value;

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

fn load(relative: &Path, seed: u64) -> Engine {
    let content = content_root();
    let content_install = content.parent().expect("content root has a parent");
    let bundled = repository_root();
    let path = [bundled.join(relative), content.join(relative)]
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| bundled.join(relative));
    let scenario = Scenario::load_from_path_with_seed(
        &path,
        &ContentResolver {
            roots: vec![bundled, content.clone()],
        },
        seed,
    )
    .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", path.display()));

    let material_group = Group::open(content.join("Material.c4g")).expect("Material.c4g opens");
    let materials = MaterialLibrary::from_group(&material_group).expect("materials load");
    let system_group =
        Group::open(content_install.join("planet/System.c4g")).expect("planet/System.c4g opens");
    let system_scripts = load_system_scripts(&system_group).expect("system scripts load");
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

fn percentile(sorted: &[Duration], quantile: f64) -> Duration {
    sorted
        .get(((sorted.len().saturating_sub(1)) as f64 * quantile).round() as usize)
        .copied()
        .unwrap_or_default()
}

fn main() {
    let mut args = env::args().skip(1);
    let relative = args
        .next()
        .unwrap_or_else(|| "content/FarWorlds.c4f/Deep.c4s".to_string());
    let frames = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20);
    let seed = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(424_242);
    let size = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);
    assert!(
        args.next().is_none(),
        "usage: volcano_profile [scenario] [frames] [seed] [size]"
    );

    let mut engine = load(Path::new(&relative), seed);
    let (width, height) = engine
        .landscape()
        .and_then(|landscape| landscape.grid_dimensions())
        .expect("exact raster landscape");
    let x = width / 2;
    let y = height - 1;
    let lava = engine
        .materials()
        .id_of("Lava")
        .expect("Deep Sea has Lava")
        .index() as i32;
    let id = engine
        .spawn_object(SpawnConfig::new("FXV1").with_position(Vector2::new(50, 50)))
        .expect("FXV1 spawns");
    let index = engine
        .find_object_index(id)
        .expect("FXV1 remains after spawn");
    engine
        .call_object_function(
            index,
            "Activate",
            vec![
                Value::Int(x),
                Value::Int(y),
                Value::Int(size),
                Value::Int(lava),
                Value::Int(0),
                Value::Int(10_000),
            ],
        )
        .expect("FXV1 activation succeeds");

    let mut samples = Vec::with_capacity(frames);
    for _ in 0..frames {
        let Some(index) = engine.find_object_index(id) else {
            break;
        };
        let started = Instant::now();
        engine
            .call_object_function(index, "Advance", Vec::new())
            .expect("FXV1 Advance succeeds");
        samples.push(started.elapsed());
    }
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let total: Duration = samples.iter().sum();
    let surface32_pixels = engine
        .landscape()
        .and_then(|landscape| landscape.pixel_grid())
        .map(|grid| {
            (0..grid.height() as i32)
                .flat_map(|y| (0..grid.width() as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| grid.surface32_pixel_at(x, y).is_some())
                .count()
        })
        .unwrap_or(0);
    println!("scenario={relative}");
    println!("seed={seed} width={width} height={height} x={x} y={y} size={size}");
    println!("samples={}", samples.len());
    println!(
        "total={total:?} mean={:?}",
        total / samples.len().max(1) as u32
    );
    println!(
        "p50={:?} p95={:?} p99={:?} max={:?}",
        percentile(&sorted, 0.50),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or_default()
    );
    println!("surface32_pixels={surface32_pixels}");
}
