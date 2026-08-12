use super::*;

fn engine_with_push_pull_plane(
    enabled: bool,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
) -> (Engine, MaterialId) {
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Water]
            Name=Water
            Density=25
            MaxSlide=1
            Instable=1

            [Material Earth]
            Name=Earth
            Density=100
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let water = crate::TestValueExt::test_value(materials.id_of("Water"));
    let mut landscape =
        crate::TestValueExt::test_value(Landscape::new(width, vec![height as i32; width as usize]));
    landscape.set_world_height(height as i32);
    landscape.set_pixel_grid(landscape::PixelGrid::new(
        width,
        height,
        bytes,
        vec![0, 25, 100],
        vec![None, Some("Water".into()), Some("Earth".into())],
        vec![None; 3],
    ));
    landscape.set_border_open(0, 0, false, false);
    landscape.resolve_grid_materials(|name| materials.id_of(name));

    let mut engine = Engine::with_seed(0x4c_30_32_33);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    engine.set_scenario_values(
        scenario::ScenarioValueStore::with_landscape_push_pull_for_test(enabled),
    );
    (engine, water)
}

#[test]
fn landscape_push_pull_routes_insert_material_and_matches_cpp_surface8_fixture() {
    const WIDTH: u32 = 7;
    const HEIGHT: u32 = 5;
    let mut initial = vec![2; WIDTH as usize * HEIGHT as usize];
    for x in 2..=4 {
        initial[WIDTH as usize + x] = 1;
    }
    initial[WIDTH as usize + 1] = 0;
    for x in 3..=5 {
        initial[3 * WIDTH as usize + x] = 1;
    }
    initial[3 * WIDTH as usize + 6] = 0;

    let (mut disabled, water) = engine_with_push_pull_plane(false, WIDTH, HEIGHT, initial.clone());
    assert!(
        !disabled.insert_material(water, 3, 1, 0, 0),
        "the default upward path stops in the denser ceiling"
    );
    assert_eq!(
        disabled
            .debug_landscape_plane()
            .expect("disabled plane remains")
            .2,
        initial
    );

    let (mut enabled, water) = engine_with_push_pull_plane(true, WIDTH, HEIGHT, initial.clone());
    assert!(enabled.insert_material(water, 3, 1, 0, 0));
    assert!(enabled.insert_material(water, 4, 3, 0, 0));
    assert!(
        !enabled.insert_material(water, 0, 0, 0, 0),
        "the closed higher-density corner has no push path"
    );

    // Frozen Surface8 bytes from the read-only C++ FindMatPathPush body:
    // the two equal-density border walks fill their sole exits; the
    // sealed third insertion makes no change. No call consumes RNG.
    let mut cpp_surface8 = initial;
    cpp_surface8[WIDTH as usize + 1] = 1;
    cpp_surface8[3 * WIDTH as usize + 6] = 1;
    assert_eq!(
        enabled
            .debug_landscape_plane()
            .expect("push-pull plane remains")
            .2,
        cpp_surface8
    );
    assert_eq!(enabled.rng, LcgRng::seed_from_u64(0x4c_30_32_33));
}
