use super::*;
use crate::landscape::PixelGrid;

fn border_engine(bottom_open: bool) -> (Engine, MaterialId) {
    let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100
            Friction=100
            "#,
    )
    .expect("material fixture parses");
    let materials = MaterialSet::from_resource_library(&library);
    let vehicle = materials.id_of("Vehicle").expect("Vehicle exists");
    let grid = PixelGrid::new(7, 3, vec![0; 21], vec![0], vec![None], vec![None]);
    let mut landscape = Landscape::new(7, vec![3; 7]).expect("landscape builds");
    landscape.set_world_height(3);
    landscape.set_pixel_grid(grid);
    landscape.set_border_open(0, 0, true, bottom_open);

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    (engine, vehicle)
}

fn spawn_digger(engine: &mut Engine) -> ObjectId {
    engine
        .register_definition(
            Definition::from_script("DGRR", "Digger", "#strict\n").expect("digger compiles"),
        )
        .expect("digger registers");
    engine
        .spawn_object(SpawnConfig::new("DGRR"))
        .expect("digger spawns")
}

fn shifting_border_engine() -> (Engine, MaterialId) {
    let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100
            BlastShiftTo=Earth
            BlastFree=1

            [Material Earth]
            Name=Earth
            Density=80
            "#,
    )
    .expect("material fixture parses");
    let materials = MaterialSet::from_resource_library(&library);
    let vehicle = materials.id_of("Vehicle").expect("Vehicle exists");
    let grid = PixelGrid::new(
        7,
        3,
        vec![0; 21],
        vec![0, 80],
        vec![None, Some("Earth".to_string())],
        vec![None; 2],
    );
    let mut landscape = Landscape::new(7, vec![3; 7]).expect("landscape builds");
    landscape.set_world_height(3);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    (engine, vehicle)
}

fn shake_border_engine() -> (Engine, MaterialId) {
    let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100

            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            "#,
    )
    .expect("material fixture parses");
    let materials = MaterialSet::from_resource_library(&library);
    let earth = materials.id_of("Earth").expect("Earth exists");
    let grid = PixelGrid::new(
        7,
        3,
        vec![1; 21],
        vec![0, 80],
        vec![None, Some("Earth".to_string())],
        vec![None; 2],
    );
    let mut landscape = Landscape::new(7, vec![0; 7]).expect("landscape builds");
    landscape.set_world_height(3);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    (engine, earth)
}

#[test]
fn blast_circle_counts_closed_bottom_vehicle_without_rng_draws() {
    let (mut engine, vehicle) = border_engine(false);
    let before_rng = engine.rng.count;

    let result = engine
        .blast_circle(Vector2::new(3, 2), 2, None)
        .expect("nonnegative blast runs");

        // C4Landscape::BlastFree includes ycnt=radius. At y=3 the circle
        // probes two closed-bottom pixels; at y=4 it probes one more.
    assert_eq!(result.pixel_count_by_material.get(&vehicle), Some(&3));
    assert_eq!(result.removed_by_material.get(&vehicle), None);
    assert_eq!(engine.rng.count, before_rng, "Vehicle has no BlastShiftTo");
}

#[test]
fn blast_circle_open_bottom_counts_sky_instead_of_vehicle() {
    let (mut engine, vehicle) = border_engine(true);

    let result = engine
        .blast_circle(Vector2::new(3, 2), 2, None)
        .expect("nonnegative blast runs");

    assert_eq!(result.pixel_count_by_material.get(&vehicle), None);
}

#[test]
fn dig_circle_open_bottom_credits_no_vehicle() {
    let (mut engine, vehicle) = border_engine(true);
    let digger = spawn_digger(&mut engine);
    engine.frame = 1;

    engine.apply_landscape_operations(vec![LandscapeOperation::DigCircle {
        center: Vector2::new(3, 2),
        radius: 2,
        requested: false,
        by_object: Some(digger),
    }]);

    let digger_index = engine.find_object_index(digger).expect("digger survives");
    assert_eq!(engine.objects[digger_index].material_content(vehicle), 0);
}

#[test]
fn dig_rect_credits_vehicle_only_at_closed_bottom() {
    for (bottom_open, expected_vehicle) in [(false, 6), (true, 0)] {
        let (mut engine, vehicle) = border_engine(bottom_open);
        let digger = spawn_digger(&mut engine);
        engine.frame = 1;

        engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
            origin: Vector2::new(2, 2),
            width: 3,
            height: 3,
            requested: false,
            by_object: Some(digger),
        }]);

        let digger_index = engine.find_object_index(digger).expect("digger survives");
        assert_eq!(
            engine.objects[digger_index].material_content(vehicle),
            expected_vehicle,
                "bottom_open={bottom_open}"
        );
    }
}

#[test]
fn closed_bottom_vehicle_runs_shift_rng_but_records_no_oob_removal() {
    let (mut engine, vehicle) = shifting_border_engine();
    let mut expected_rng = engine.rng.clone();
    for _ in 0..3 {
        let _ = expected_rng.random(3);
    }

    let result = engine
        .blast_circle(Vector2::new(3, 2), 2, None)
        .expect("nonnegative blast runs");

    assert_eq!(result.pixel_count_by_material.get(&vehicle), Some(&3));
    assert_eq!(result.removed_by_material.get(&vehicle), None);
    assert!(result.affected_columns.is_empty());
    assert_eq!(engine.rng, expected_rng, "one Random(3) per border pixel");
    assert!(
        engine
            .landscape
            .as_ref()
            .unwrap()
            .pixel_grid()
            .unwrap()
            .bytes()
            .iter()
            .all(|byte| *byte == 0),
            "out-of-bounds shifts and clears cannot alter Surface8"
    );
}

#[test]
fn shake_circle_closed_bottom_creates_no_vehicle_pxs() {
    let (mut engine, earth) = shake_border_engine();
    let before_rng = engine.rng.count;

    engine.execute_shake_circle_operation(Vector2::new(3, 2), 2);

    assert_eq!(
        engine.pxs_system.count(),
        7,
            "only the seven in-bounds Earth pixels create PXS"
    );
    assert!(engine.pxs_system.iter().all(|pxs| pxs.mat == earth));
    assert_eq!(engine.rng.count, before_rng);
}
