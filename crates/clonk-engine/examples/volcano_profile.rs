//! Deterministic forced-volcano profiler for the real Deep Sea FXV1 script.
//!
//! This keeps the shipped script and landscape in the loop while forcing the
//! weather-created object, so a profile does not depend on waiting for the
//! random weather trigger. It is a measurement tool only: the final state is
//! printed as a checksum and no simulation behavior is changed.
//!
//! The elapsed span is reported twice: `advance` covers the FXV1 `Advance`
//! calls alone, and the unqualified figures add the render dirty-rect scan that
//! follows them. Keeping them apart is what settles whether a frame cost is
//! simulation or presentation — for clonk-org/clonk-rs#497 the two are equal to
//! within a microsecond, which places the whole cost in the script.

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

fn surface32_state(engine: &Engine) -> (usize, u64) {
    let Some(grid) = engine
        .landscape()
        .and_then(|landscape| landscape.pixel_grid())
    else {
        return (0, 0);
    };
    let mut count = 0;
    let mut checksum = 0xcbf2_9ce4_8422_2325;
    for y in 0..grid.height() as i32 {
        for x in 0..grid.width() as i32 {
            let Some(color) = grid.surface32_pixel_at(x, y) else {
                continue;
            };
            count += 1;
            for value in [x as u32, y as u32, color] {
                checksum ^= u64::from(value);
                checksum = checksum.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    (count, checksum)
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
    let volcanoes = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    assert!(
        args.next().is_none(),
        "usage: volcano_profile [scenario] [frames] [seed] [size] [volcanoes]"
    );
    assert!(volcanoes > 0, "volcanoes must be positive");

    let mut engine = load(Path::new(&relative), seed);
    let (width, height) = engine
        .landscape()
        .and_then(|landscape| landscape.grid_dimensions())
        .expect("exact raster landscape");
    let center_x = width / 2;
    let y = height - 1;
    let lava = engine
        .materials()
        .id_of("Lava")
        .expect("Deep Sea has Lava")
        .index() as i32;
    let mut ids = Vec::with_capacity(volcanoes);
    let spacing = size.max(20) + 20;
    for volcano_index in 0..volcanoes {
        let offset = (volcano_index as i32 - volcanoes as i32 / 2) * spacing;
        let x = (center_x + offset).clamp(0, width.saturating_sub(1));
        let id = engine
            .spawn_object(SpawnConfig::new("FXV1").with_position(Vector2::new(50 + offset, 50)))
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
        ids.push(id);
    }

    let mut samples = Vec::with_capacity(frames);
    let mut advance_samples = Vec::with_capacity(frames);
    let mut render_anchor = engine
        .landscape()
        .and_then(|landscape| landscape.pixel_grid())
        .map(|grid| grid.render_anchor());
    let mut dirty_rects = 0usize;
    let mut dirty_area = 0u64;
    let mut dirty_full_rebuilds = 0usize;
    let mut dirty_max = 0usize;
    for _ in 0..frames {
        let active_ids: Vec<_> = ids
            .iter()
            .copied()
            .filter(|id| {
                engine
                    .object_snapshot(*id)
                    .is_some_and(|object| object.status.is_active())
            })
            .collect();
        if active_ids.is_empty() {
            break;
        }
        let started = Instant::now();
        for id in active_ids {
            let Some(index) = engine.find_object_index(id) else {
                continue;
            };
            engine
                .call_object_function(index, "Advance", Vec::new())
                .expect("FXV1 Advance succeeds");
        }
        let advanced = started.elapsed();
        let grid = engine
            .landscape()
            .and_then(|landscape| landscape.pixel_grid())
            .expect("forced volcano has an exact raster landscape");
        if let Some(previous) = render_anchor.take() {
            match grid.render_dirty_rects_since_anchor(&previous) {
                Some(rects) => {
                    dirty_max = dirty_max.max(rects.len());
                    dirty_rects = dirty_rects.saturating_add(rects.len());
                    dirty_area = dirty_area.saturating_add(rects.iter().fold(0, |area, rect| {
                        area.saturating_add(u64::from(rect.width()) * u64::from(rect.height()))
                    }));
                }
                None => dirty_full_rebuilds = dirty_full_rebuilds.saturating_add(1),
            }
        }
        render_anchor = Some(grid.render_anchor());
        advance_samples.push(advanced);
        samples.push(started.elapsed());
    }
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let total: Duration = samples.iter().sum();
    let (surface32_pixels, surface32_checksum) = surface32_state(&engine);
    println!("scenario={relative}");
    println!(
        "seed={seed} width={width} height={height} x={center_x} y={y} size={size} volcanoes={volcanoes}"
    );
    println!("samples={}", samples.len());
    println!(
        "dirty_rects={dirty_rects} dirty_area={dirty_area} dirty_max={dirty_max} dirty_full_rebuilds={dirty_full_rebuilds}"
    );
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
    let mut advance_sorted = advance_samples.clone();
    advance_sorted.sort_unstable();
    let advance_total: Duration = advance_samples.iter().sum();
    println!(
        "advance mean={:?} p50={:?} p95={:?} p99={:?} max={:?}",
        advance_total / advance_samples.len().max(1) as u32,
        percentile(&advance_sorted, 0.50),
        percentile(&advance_sorted, 0.95),
        percentile(&advance_sorted, 0.99),
        advance_sorted.last().copied().unwrap_or_default()
    );
    println!("surface32_pixels={surface32_pixels} surface32_checksum=0x{surface32_checksum:016x}");
}
