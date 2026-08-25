use super::*;
use crate::landscape::{
    LandscapeRasterState, PixelGrid, RuntimeTexMapMaterial, RuntimeTexMapState,
};

fn blast_engine(copy_to_lower_slot: bool, blast_shift_to: &str) -> (Engine, MaterialId) {
    let source = format!(
        r#"
            [Material Rock]
            Name=Rock
            Density=80
            BlastShiftTo={blast_shift_to}

            [Material Target]
            Name=Target
            Density=30
            "#
    );
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(&source));
    let materials = MaterialSet::from_resource_library(&library);
    let rock = crate::TestValueExt::test_value(materials.id_of("Rock"));

    let mut densities = vec![0; 128];
    densities[10] = 80;
    densities[40] = 30;
    let mut material_names = vec![None; 128];
    material_names[10] = Some("Rock".to_string());
    material_names[40] = Some("Target".to_string());
    let mut texture_names = vec![None; 128];
    texture_names[10] = Some("Rough".to_string());
    texture_names[40] = Some("Smooth".to_string());
    let grid = PixelGrid::new(
        5,
        5,
        {
            let mut bytes = vec![0; 25];
            bytes[12] = 10 | 0x80;
            bytes
        },
        densities.clone(),
        material_names.clone(),
        texture_names.clone(),
    );
    let runtime_materials = materials
        .iter()
        .map(|material| RuntimeTexMapMaterial {
            name: material.name().to_string(),
            density: material.density(),
            shape: crate::chunky::ChunkShape::Smooth,
        })
        .collect();
    let texmap = RuntimeTexMapState {
        densities,
        material_names,
        texture_names: texture_names.clone(),
        match_texture_names: texture_names,
        shapes: vec![None; 128],
        materials: runtime_materials,
        texture_inventory: vec!["Rough".to_string(), "Smooth".to_string()],
        default_material_entries: vec![("Rock".to_string(), 10), ("Target".to_string(), 40)],
        material_crossmap_entries: vec![40],
        ..Default::default()
    };
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(5, vec![5; 5]));
    landscape.set_world_height(5);
    landscape.set_pixel_grid(grid);
    landscape.refresh_all_raster_columns();
    landscape.set_raster_state(LandscapeRasterState::new(1, 0, texmap));

    if copy_to_lower_slot {
        let mut moved_texmap = crate::TestValueExt::test_value(landscape.raster_state())
            .texmap()
            .clone();
        let (success, indices) = moved_texmap.set_texture_index("Target-Smooth", 5, false);
        assert!(success);
        assert_eq!(indices, Some((40, 5)));
        assert!(landscape.apply_runtime_texture_index_move(moved_texmap, 40, 5));
    }

    let mut engine = Engine::with_seed(17);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    (engine, rock)
}

#[test]
fn blast_shift_uses_frozen_crossmap_slot_after_lower_index_copy() {
    let (mut baseline, baseline_rock) = blast_engine(false, "Target-Smooth");
    let (mut moved, moved_rock) = blast_engine(true, "Target-Smooth");
    let mut expected_baseline_rng = baseline.rng.clone();
    let mut expected_moved_rng = moved.rng.clone();
    let _ = expected_baseline_rng.random(1);
    let _ = expected_moved_rng.random(1);

    let baseline_result =
        crate::TestValueExt::test_value(baseline.blast_circle(Vector2::new(2, 2), 2, None));
    let moved_result =
        crate::TestValueExt::test_value(moved.blast_circle(Vector2::new(2, 2), 2, None));

    assert_eq!(baseline_result.pixel_count_by_material[&baseline_rock], 1);
    assert_eq!(moved_result.pixel_count_by_material[&moved_rock], 1);
    assert_eq!(
        baseline.landscape().unwrap().grid_byte_at(2, 2),
        Some(40 | 0x80)
    );
    assert_eq!(
        moved.landscape().unwrap().grid_byte_at(2, 2),
        Some(40 | 0x80)
    );
    assert_ne!(
        moved.landscape().unwrap().grid_byte_at(2, 2),
        Some(5 | 0x80)
    );
    assert_eq!(baseline.rng, expected_baseline_rng);
    assert_eq!(moved.rng, expected_moved_rng);
    let texmap = crate::TestValueExt::test_value(
        crate::TestValueExt::test_value(moved.landscape()).raster_state(),
    )
    .texmap();
    assert_eq!(texmap.material_names[5].as_deref(), Some("Target"));
    assert_eq!(texmap.material_names[40].as_deref(), Some("Target"));
    assert_eq!(texmap.material_crossmap_entries, vec![40]);
}

#[test]
fn frozen_zero_crossmap_does_not_re_resolve_a_later_pair() {
    let (mut engine, rock) = blast_engine(true, "Target-Smooth");
    crate::TestValueExt::test_value(engine.landscape.as_mut().unwrap().raster_state_mut())
        .texmap_mut()
        .material_crossmap_entries[0] = 0;
    let before_rng = engine.rng.clone();

    let result = crate::TestValueExt::test_value(engine.blast_circle(Vector2::new(2, 2), 2, None));

    assert_eq!(result.pixel_count_by_material[&rock], 1);
    assert_eq!(
        engine.landscape().unwrap().grid_byte_at(2, 2),
        Some(10 | 0x80)
    );
    assert_eq!(engine.rng, before_rng);
}

#[test]
fn blast_shift_without_texture_keeps_frozen_default_slot() {
    let (mut engine, rock) = blast_engine(true, "Target");
    let mut expected_rng = engine.rng.clone();
    let _ = expected_rng.random(1);

    let result = crate::TestValueExt::test_value(engine.blast_circle(Vector2::new(2, 2), 2, None));

    assert_eq!(result.pixel_count_by_material[&rock], 1);
    assert_eq!(
        engine.landscape().unwrap().grid_byte_at(2, 2),
        Some(40 | 0x80)
    );
    assert_eq!(engine.rng, expected_rng);
}

#[test]
fn raster_blast_resolves_each_material_shift_once() {
    // C4Landscape::BlastFree counts the circle before its second raster walk,
    // while C4Landscape::BlastFreePix consumes the already-crossmapped
    // material properties for each pixel (C4Landscape.cpp:941-970,1022-1063;
    // C4Material.cpp:474-479). Resolving that immutable property repeatedly
    // must not add work or alter the row-major draw sequence.
    let (mut engine, rock) = blast_engine(false, "Target-Smooth");
    for y in 0..5 {
        for x in 0..5 {
            engine
                .landscape
                .as_mut()
                .expect("raster landscape")
                .grid_write_byte(x, y, 10 | 0x80);
        }
    }
    BLAST_SHIFT_BYTE_RESOLUTIONS.with(|count| count.set(0));

    let result = crate::TestValueExt::test_value(engine.blast_circle(Vector2::new(2, 2), 2, None));

    assert!(result.pixel_count_by_material[&rock] > 1);
    assert_eq!(
        BLAST_SHIFT_BYTE_RESOLUTIONS.with(Cell::get),
        1,
        "the crossmapped shift byte is invariant for the complete blast"
    );
}
