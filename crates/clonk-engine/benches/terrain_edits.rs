//! Complete script terrain callbacks and effect ticks; setup and projection
//! are outside the timed interval. Run with `cargo bench -p clonk-engine
//! --bench terrain_edits`. The byte fingerprint and RNG count must match
//! between revisions. Sizes include the existing Alchemy-sized fixture.
use clonk_engine::landscape::PixelGrid;
use clonk_engine::{Definition, Engine, Landscape, SpawnConfig};
use clonk_resources::MaterialLibrary;
use clonk_script::Value;
use std::hint::black_box;
use std::time::Instant;

const CALLBACKS: i32 = 200;
const SCRIPT: &str = r#"
#strict 2
func Probe(x, y) { DigFree(x, y, 2); return GetMaterial(x, y); }
func SeedDiggers(y) {
    for (var x = 0; x < 200; x++) AddEffect("DigEarth", this, 10, 1, this, 0, 8 + 2*x, y);
}
func FxDigEarthStart(target, number, temporary, x, y) {
    if (!temporary) { EffectVar(0, target, number) = x; EffectVar(1, target, number) = y; }
}
func FxDigEarthTimer(target, number) {
    var y = EffectVar(1, target, number);
    DigFree(EffectVar(0, target, number), y, 2);
    EffectVar(1, target, number) = y + 4;
    return 1;
}
"#;

fn setup(width: u32, height: u32, effects: bool) -> Engine {
    let mut engine = Engine::with_seed(0);
    let library =
        MaterialLibrary::parse("[Material Earth]\nName=Earth\nDensity=100\nDigFree=1\n").unwrap();
    engine.configure_materials_from_library(&library);
    let top = height / 2;
    let bytes = (0..height)
        .flat_map(|y| std::iter::repeat_n(u8::from(y >= top), width as usize))
        .collect();
    let grid = PixelGrid::new(
        width,
        height,
        bytes,
        vec![0, 100],
        vec![None, Some("Earth".into())],
        vec![None, None],
    );
    let mut landscape = Landscape::flat(width, top as i32);
    landscape.set_world_height(height as i32);
    landscape.set_no_scan(true);
    landscape.set_pixel_grid(grid);
    engine.set_landscape(landscape);
    let mut definition = Definition::from_script("TEST", "Digger", SCRIPT).unwrap();
    definition.set_c4_callback_convention(true);
    engine.register_definition(definition).unwrap();
    engine.spawn_object(SpawnConfig::new("TEST")).unwrap();
    if effects {
        engine
            .call_object_function(0, "SeedDiggers", vec![Value::Int(top as i32)])
            .unwrap();
    }
    engine
}

fn main() {
    for (width, height) in [(512, 256), (1488, 1536), (4096, 4096)] {
        for (warm, retain_snapshot) in [(false, false), (true, false), (true, true)] {
            for effects in [false, true] {
                let mut samples = Vec::new();
                let mut fingerprint = None;
                let mut random_count = 0;
                for sample in 0..12 {
                    let mut engine = setup(width, height, effects);
                    if warm {
                        if effects {
                            engine.tick_without_snapshot().unwrap();
                        } else {
                            for index in 0..CALLBACKS {
                                engine
                                    .call_object_function(
                                        0,
                                        "Probe",
                                        vec![
                                            Value::Int(8 + 2 * index),
                                            Value::Int(height as i32 / 2),
                                        ],
                                    )
                                    .unwrap();
                            }
                        }
                    }
                    let snapshot = retain_snapshot.then(|| engine.snapshot());
                    let edit_y = height as i32 / 2 + if warm { 4 } else { 0 };
                    let start = Instant::now();
                    if effects {
                        engine.tick_without_snapshot().unwrap();
                    } else {
                        for index in 0..CALLBACKS {
                            black_box(
                                engine
                                    .call_object_function(
                                        0,
                                        "Probe",
                                        vec![Value::Int(8 + 2 * index), Value::Int(edit_y)],
                                    )
                                    .unwrap(),
                            );
                        }
                    }
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    random_count = engine.sync_check(0).random_count;
                    let grid = engine.landscape().unwrap().pixel_grid().unwrap();
                    assert_eq!(grid.byte_at(8, edit_y), Some(0));
                    if let Some(snapshot) = snapshot.as_ref() {
                        assert_eq!(
                            snapshot.landscape.as_ref().unwrap().grid_byte_at(8, edit_y),
                            Some(1)
                        );
                    }
                    let hash = grid
                        .bytes()
                        .iter()
                        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
                            (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
                        });
                    assert_eq!(*fingerprint.get_or_insert(hash), hash);
                    if sample >= 3 {
                        samples.push(elapsed);
                    }
                }
                samples.sort_by(f64::total_cmp);
                println!(
                    "{}",
                    serde_json::json!({
                        "size": [width, height], "mode": if effects { "effect_tick" } else { "callbacks" },
                        "warm": warm, "retained_snapshot": retain_snapshot, "callbacks": CALLBACKS, "median_ms": samples[samples.len()/2],
                        "samples_ms": samples, "pixels_fnv1a": format!("{:016x}", fingerprint.unwrap()),
                        "random_count": random_count,
                    })
                );
            }
        }
    }
    let engine = setup(1488, 1536, false);
    let original = engine.landscape().unwrap().pixel_grid().unwrap();
    let mut points = Vec::with_capacity(1_000_000);
    let mut random = 12345u32;
    for _ in 0..1_000_000 {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        points.push(((random % 1488) as i32, ((random / 1488) % 1536) as i32));
    }
    for changed in [false, true] {
        let mut grid = original.clone();
        if changed {
            grid.write_byte(700, 800, 0);
        }
        let mut samples = Vec::new();
        let mut checksum = 0u64;
        for sample in 0..12 {
            let start = Instant::now();
            let sum: u64 = points
                .iter()
                .map(|&(x, y)| u64::from(black_box(&grid).byte_at(x, y).unwrap()))
                .sum();
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            checksum = black_box(sum);
            if sample >= 3 {
                samples.push(elapsed);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "{}",
            serde_json::json!({"size": [1488, 1536],
            "mode": if changed { "shared_pixel_reads" } else { "flat_pixel_reads" },
            "operations": points.len(), "median_ms": samples[samples.len()/2],
            "samples_ms": samples, "checksum": checksum})
        );
    }
}
