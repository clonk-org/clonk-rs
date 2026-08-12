use super::*;
use crate::landscape::PixelGrid;

#[test]
fn dig_free_circle_closed_bottom_credits_vehicle_without_side_effects() {
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100
            Friction=100
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let vehicle = crate::TestValueExt::test_value(materials.id_of("Vehicle"));

    let grid = PixelGrid::new(9, 5, vec![0; 45], vec![0], vec![None], vec![None]);
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(9, vec![0; 9]));
    landscape.set_world_height(5);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    crate::TestValueExt::test_value(engine.register_script_definition(
        "DGRR",
        "Digger",
        "#strict\n",
    ));
    let digger = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("DGRR")));
    let before: Vec<_> = (0..5)
        .flat_map(|y| (0..9).map(move |x| (x, y)))
        .map(|(x, y)| {
            crate::TestValueExt::test_value(engine.landscape.as_ref().unwrap().pixel_grid())
                .byte_at(x, y)
        })
        .collect();

    engine.frame = 1;
    engine.apply_landscape_operations(vec![LandscapeOperation::DigCircle {
        center: Vector2::new(4, 4),
        radius: 3,
        requested: false,
        by_object: Some(digger),
    }]);

    // The main-loop rows at y=5 and y=6 are past the closed bottom.
    // Each has line width two, so C++ credits 4 + 4 Vehicle probes.
    let digger_index = crate::TestValueExt::test_value(engine.find_object_index(digger));
    assert_eq!(engine.objects[digger_index].material_content(vehicle), 8);
    let after: Vec<_> = (0..5)
        .flat_map(|y| (0..9).map(move |x| (x, y)))
        .map(|(x, y)| {
            crate::TestValueExt::test_value(engine.landscape.as_ref().unwrap().pixel_grid())
                .byte_at(x, y)
        })
        .collect();
    assert_eq!(after, before, "closed-border probes clear no pixels");
    assert_eq!(engine.pxs_system.count(), 0);
    assert_eq!(engine.mass_movers.live_movers(), 0);
}

#[test]
fn dig_out_material_cast_waits_for_tick5_and_retains_contents() {
    // C4Landscape::DigFreeRect always accumulates pixels, but calls
    // C4Object::DigOutMaterialCast only on !Tick5 (C4Landscape.cpp:986-996).
    // Repeating the already-cleared rectangle on frame 5 also proves the
    // cast check is not accidentally conditional on this dig removing more.
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=2
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let earth = crate::TestValueExt::test_value(materials.id_of("Earth"));

    let grid = PixelGrid::new(
        2,
        1,
        vec![1, 1],
        vec![0, 80],
        vec![None, Some("Earth".to_string())],
        vec![None; 2],
    );
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(2, vec![1; 2]));
    landscape.set_world_height(1);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    crate::TestValueExt::test_value(engine.register_script_definition(
        "DGRR",
        "Digger",
        "#strict\n",
    ));
    crate::TestValueExt::test_value(engine.register_script_definition("GEM_", "Gem", "#strict\n"));
    let digger = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("DGRR")));
    let dig = LandscapeOperation::DigRect {
        origin: Vector2::ZERO,
        width: 2,
        height: 1,
        requested: false,
        by_object: Some(digger),
    };

    engine.frame = 1;
    engine.apply_landscape_operations(vec![dig.clone()]);
    let digger_index = crate::TestValueExt::test_value(engine.find_object_index(digger));
    assert_eq!(engine.objects[digger_index].material_content(earth), 2);
    assert!(
        engine
            .objects
            .iter()
            .all(|object| object.definition_id != "GEM_"),
        "off-Tick5 dig must retain contents without spawning"
    );

    engine.frame = 5;
    engine.apply_landscape_operations(vec![dig]);
    assert_eq!(
        engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
            .count(),
        1,
        "Tick5 dig casts the retained contents even when no new pixel clears"
    );
    let digger_index = crate::TestValueExt::test_value(engine.find_object_index(digger));
    assert_eq!(engine.objects[digger_index].material_content(earth), 0);
}

#[test]
fn continuous_dig_replay_matches_cpp_per_frame_spawn_census() {
    // Frozen from the unmodified C++ ordering: C4Game::Ticks advances
    // iTick5 before object execution (C4Game.cpp:1906), and every
    // C4Landscape::DigFreeRect call accumulates first but casts only on
    // !Tick5 (C4Landscape.cpp:986-996). With ratio 3 and one fresh Earth
    // pixel per frame, the bucket reaches 4 before each Tick5 cast.
    const CPP_CUMULATIVE_SPAWNS: [usize; 10] = [0, 0, 0, 0, 1, 1, 1, 1, 1, 2];
    const CPP_RETAINED_CONTENTS: [i32; 10] = [1, 2, 3, 4, 0, 1, 2, 3, 4, 0];

    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=3
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let earth = crate::TestValueExt::test_value(materials.id_of("Earth"));

    let grid = PixelGrid::new(
        10,
        1,
        vec![1; 10],
        vec![0, 80],
        vec![None, Some("Earth".to_string())],
        vec![None; 2],
    );
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(10, vec![1; 10]));
    landscape.set_world_height(1);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    crate::TestValueExt::test_value(engine.register_script_definition(
        "DGRR",
        "Digger",
        "#strict\n",
    ));
    crate::TestValueExt::test_value(engine.register_script_definition("GEM_", "Gem", "#strict\n"));
    let digger = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("DGRR")));

    for frame_index in 0..10 {
        engine.frame = (frame_index + 1) as u64;
        engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
            origin: Vector2::new(frame_index as i32, 0),
            width: 1,
            height: 1,
            requested: false,
            by_object: Some(digger),
        }]);

        let spawns = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
            .count();
        assert_eq!(
            spawns,
            CPP_CUMULATIVE_SPAWNS[frame_index],
            "C++ cumulative Dig2Object census diverged on frame {}",
            frame_index + 1
        );
        let digger_index = crate::TestValueExt::test_value(engine.find_object_index(digger));
        assert_eq!(
            engine.objects[digger_index].material_content(earth),
            CPP_RETAINED_CONTENTS[frame_index],
            "C++ retained material contents diverged on frame {}",
            frame_index + 1
        );
    }
}
