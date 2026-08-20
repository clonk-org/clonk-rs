use crate::far_worlds_deep_lorry_acquire::deep_hydroclonk_finds_coral_inside_a_submerged_lorry;
use crate::support::real_scenario::{prepare_installed_scenario, PreparedInstalledScenario};
use crate::support::PreparedScenarioSubcase;
use crate::wagon_grab_put_get::deep_hydroclonk_reaches_wagon_cargo_through_its_entrance;
use clonk_engine::landscape::PixelGrid;
use clonk_engine::{Landscape, MaterialId, SpawnConfig, Vector2};

const WIDTH: u32 = 101;
const HEIGHT: u32 = 120;
const AIRLOCK_X: i32 = 50;
const REQUESTED_AIRLOCK_Y: i32 = 80;

fn pumping_fixture(airlock: Vector2) -> Landscape {
    let source_top = airlock.y + 13;
    let source_bottom = airlock.y + 15;
    let outlet_floor = airlock.y - 49;
    let bytes = (0..HEIGHT as i32)
        .flat_map(|y| {
            (0..WIDTH as i32).map(move |x| {
                if y == outlet_floor {
                    2 // Granite floor below AIRL's (0,-50) outlet.
                } else if x == airlock.x && (source_top..=source_bottom).contains(&y) {
                    1 // Water at the three RandomX(13,15) source pixels.
                } else {
                    0
                }
            })
        })
        .collect();
    let grid = PixelGrid::new(
        WIDTH,
        HEIGHT,
        bytes,
        vec![0, 25, 50],
        vec![None, Some("Water".to_string()), Some("Granite".to_string())],
        vec![None; 3],
    );
    let mut landscape = Landscape::flat(WIDTH, HEIGHT as i32);
    landscape.set_pixel_grid(grid);
    landscape.set_world_height(HEIGHT as i32);
    landscape
}

fn live_material_count(engine: &clonk_engine::Engine, material: MaterialId) -> usize {
    let landscape = crate::support::TestValueExt::test_value(engine.landscape())
        .material_pixel_count(material, None) as usize;
    let pxs = engine
        .pxs_system
        .iter()
        .filter(|pixel| pixel.mat == material)
        .count();
    landscape + pxs
}

#[test]
fn far_worlds_deep_shared_scenario_subcases() {
    let prepared = prepare_installed_scenario("FarWorlds.c4f/Deep.c4s", 0);
    let subcases: &[PreparedScenarioSubcase] = &[
        (
            "deep_sea_airlock_pumping_does_not_duplicate_repeatedly_sampled_liquid",
            deep_sea_airlock_pumping_does_not_duplicate_repeatedly_sampled_liquid,
        ),
        (
            "deep_hydroclonk_finds_coral_inside_a_submerged_lorry",
            deep_hydroclonk_finds_coral_inside_a_submerged_lorry,
        ),
        (
            "deep_hydroclonk_reaches_wagon_cargo_through_its_entrance",
            deep_hydroclonk_reaches_wagon_cargo_through_its_entrance,
        ),
    ];
    let mut failures = Vec::new();

    for &(name, subcase) in subcases {
        eprintln!("running shared Deep Sea subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subcase(&prepared))).is_err() {
            eprintln!("shared Deep Sea subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} shared Deep Sea subcase(s) failed: {}",
            failures.len(),
            failures.join(", ")
        );
    }
}

fn deep_sea_airlock_pumping_does_not_duplicate_repeatedly_sampled_liquid(
    prepared: &PreparedInstalledScenario,
) {
    // AIRL::Pumping performs twenty paired
    // InsertMaterial(ExtractLiquid(0, RandomX(13,15)), 0, -50) calls
    // (FarWorlds.c4d/Deep.c4d/Structures.c4d/Airlock.c4d/Script.c:53-65).
    // C++ FnExtractLiquid removes the pixel before returning its material,
    // so every later call in the same callback observes that mutation before
    // FnInsertMaterial runs (src/C4Script.cpp:2194-2204). A stale host-world
    // copy instead returns the same three source pixels all twenty times and
    // creates liquid that never existed.
    let mut engine = prepared.instantiate();
    let airlock = crate::support::TestValueExt::test_value(engine.spawn_object(
        SpawnConfig::new("AIRL").with_position(Vector2::new(AIRLOCK_X, REQUESTED_AIRLOCK_Y)),
    ));
    assert_eq!(
        engine.debug_definition_has_function("AIRL", "Pumping"),
        Some(true),
        "the spawned AIRL uses the shipped Deep Sea script"
    );
    let airlock_position = crate::support::TestValueExt::test_value(
        engine
            .object_snapshot(airlock)
            .map(|object| object.position),
    );
    assert_eq!(airlock_position.x, AIRLOCK_X);
    let source = airlock_position.y + 13..=airlock_position.y + 15;

    // Install the controlled plane after AIRL::Initialize so its real closed
    // solid mask cannot obscure the three liquid probes. The definition,
    // script, material library, RNG, and callback path remain the shipped
    // Deep Sea scenario's.
    engine.set_landscape(pumping_fixture(airlock_position));
    let water = crate::support::TestValueExt::test_value(
        engine
            .landscape()
            .and_then(|landscape| landscape.material_at(AIRLOCK_X, *source.start())),
    );
    assert_eq!(live_material_count(&engine, water), 3);

    let airlock_index = crate::support::TestValueExt::test_value(engine.find_object_index(airlock));
    crate::support::TestValueExt::test_value(engine.call_object_function(
        airlock_index,
        "Pumping",
        Vec::new(),
    ));

    let remaining = source
        .filter(|&y| engine.debug_landscape_is_liquid(AIRLOCK_X, y))
        .collect::<Vec<_>>();
    assert!(
        remaining.is_empty(),
        "twenty deterministic probes left source rows {remaining:?} wet"
    );
    assert_eq!(
        live_material_count(&engine, water),
        3,
        "synchronous ExtractLiquid/InsertMaterial conserves the three water pixels"
    );
}
