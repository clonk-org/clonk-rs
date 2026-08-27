use super::*;

#[test]
fn restore_reputs_overlapping_masks_in_master_object_order() {
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(20, vec![0; 20]));
    landscape.set_world_height(20);
    landscape.set_pixel_grid(landscape::PixelGrid::new(
        20,
        20,
        vec![0; 400],
        vec![0, 100, 100],
        vec![None, Some("Earth".into()), Some("Vehicle".into())],
        vec![None; 3],
    ));
    landscape.grid_write_byte(10, 10, 1);

    let mut mask = test_definition("OMSK", "Ordered mask", "");
    mask.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
    mask.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
    mask.set_sprite_image(Some(DefinitionSpriteImage {
        width: 1,
        height: 1,
        pixels: Arc::from([0, 0, 0, 255]),
        color_mask: None,
    }));

    let mut engine = Engine::with_seed(32);
    engine.set_landscape(landscape);
    crate::TestValueExt::test_value(engine.register_definition(mask));
    let first = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("OMSK")
                .with_position(Vector2::new(10, 10))
                .with_loaded(true),
        ),
    );
    let second = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("OMSK")
                .with_position(Vector2::new(10, 10))
                .with_loaded(true),
        ),
    );
    for id in [first, second] {
        let index = crate::TestValueExt::test_value(engine.find_object_index(id));
        engine.update_solid_mask(index);
    }

    let state = engine.capture_state();
    assert_eq!(state.object_order, vec![first, second]);
    assert_eq!(
        state
            .landscape
            .as_ref()
            .expect("clean landscape captured")
            .grid_byte_at(10, 10),
        Some(1)
    );

    crate::TestValueExt::test_value(engine.restore_state(&state));
    assert_eq!(engine.execution.exec_list, vec![first, second]);
    let first_index = crate::TestValueExt::test_value(engine.find_object_index(first));
    let first_bake =
        crate::TestValueExt::test_value(engine.objects[first_index].solid_mask_bake.as_ref());
    let second_index = crate::TestValueExt::test_value(engine.find_object_index(second));
    let second_bake =
        crate::TestValueExt::test_value(engine.objects[second_index].solid_mask_bake.as_ref());
    assert_eq!(
        second_bake.buffer,
        vec![1],
        "master First->Next starts with the reverse of Rust exec order"
    );
    assert_eq!(
        first_bake.buffer,
        vec![2],
        "the later overlapping mask records MCVehic as an unused slot"
    );
}
