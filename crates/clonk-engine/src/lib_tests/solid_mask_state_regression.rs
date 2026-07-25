use super::*;

#[test]
fn restore_reputs_overlapping_masks_in_master_object_order() {
    let mut landscape = Landscape::new(20, vec![0; 20]).expect("landscape builds");
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

    let mut mask =
        Definition::from_script("OMSK", "Ordered mask", "").expect("mask definition compiles");
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
    engine.register_definition(mask).expect("mask registers");
    let first = engine
        .spawn_object(
            SpawnConfig::new("OMSK")
                .with_position(Vector2::new(10, 10))
                .with_loaded(true),
        )
        .expect("first mask spawns");
    let second = engine
        .spawn_object(
            SpawnConfig::new("OMSK")
                .with_position(Vector2::new(10, 10))
                .with_loaded(true),
        )
        .expect("second mask spawns");
    for id in [first, second] {
        let index = engine.find_object_index(id).expect("mask exists");
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

    engine.restore_state(&state).expect("state restores");
    assert_eq!(engine.exec_list, vec![first, second]);
    let first_index = engine.find_object_index(first).expect("first restores");
    let first_bake = engine.objects[first_index]
        .solid_mask_bake
        .as_ref()
        .expect("first mask re-put");
    let second_index = engine.find_object_index(second).expect("second restores");
    let second_bake = engine.objects[second_index]
        .solid_mask_bake
        .as_ref()
        .expect("second mask re-put");
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
