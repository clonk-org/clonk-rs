use super::*;
use crate::landscape::PixelGrid;

fn splash_engine() -> Engine {
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Negative]
            Name=Negative
            Density=25
            SplashRate=-5

            [Material Zero]
            Name=Zero
            Density=25
            SplashRate=0

            [Material Positive]
            Name=Positive
            Density=25
            SplashRate=1
            "#,
    ));
    let mut engine = Engine::with_seed(91);
    engine.set_materials(MaterialSet::from_resource_library(&library));
    engine
}

fn run_insert_check(engine: &mut Engine, material: MaterialId) -> bool {
    let (mut x, mut y) = (0, 0);
    let (mut xdir, mut ydir) = (C4Fixed::ZERO, itofix(2));
    let mut pos_changed = false;
    engine.mrf_insert_check(
        &mut x,
        &mut y,
        &mut xdir,
        &mut ydir,
        material,
        None,
        &mut pos_changed,
    )
}

#[test]
fn signed_splash_rate_preserves_cpp_rng_consumption() {
    let mut negative_engine = splash_engine();
    let negative = crate::TestValueExt::test_value(negative_engine.materials.id_of("Negative"));
    let mut expected = negative_engine.rng.clone();
    let splash = expected.random(-5) == 0;
    if splash {
        let _ = expected.random(200);
    }
    assert_eq!(run_insert_check(&mut negative_engine, negative), !splash);
    assert_eq!(
        negative_engine.rng, expected,
        "negative range is converted to unsigned for modulo after consuming one draw"
    );

    let mut zero_engine = splash_engine();
    let zero = crate::TestValueExt::test_value(zero_engine.materials.id_of("Zero"));
    let expected = zero_engine.rng.clone();
    assert!(run_insert_check(&mut zero_engine, zero));
    assert_eq!(zero_engine.rng, expected, "zero skips Random entirely");

    let mut positive_engine = splash_engine();
    let positive = crate::TestValueExt::test_value(positive_engine.materials.id_of("Positive"));
    let mut expected = positive_engine.rng.clone();
    assert_eq!(expected.random(1), 0);
    let _ = expected.random(200);
    assert!(!run_insert_check(&mut positive_engine, positive));
    assert_eq!(positive_engine.rng, expected);
}

fn conversion_engine(below_matches: bool) -> Engine {
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Natural]
            Name=Natural
            Density=30
            InMatConvert=Rock
            InMatConvertTo=Water
            InMatConvertDepth=-2

            [Material Custom]
            Name=Custom
            Density=30

            [Reaction]
            Type=Convert
            TargetSpec=Rock
            ConvertMat=Water
            Depth=-2
            CheckSlide=0

            [Material Rock]
            Name=Rock
            Density=80

            [Material Wrong]
            Name=Wrong
            Density=80

            [Material Water]
            Name=Water
            Density=25
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let below_byte = if below_matches { 1 } else { 2 };
    // y=0 is Rock, so an accidental absolute-value/sign-flipped depth
    // succeeds; the native negative depth probes y=4 instead.
    let grid = PixelGrid::new(
        1,
        5,
        vec![1, 0, 1, 0, below_byte],
        vec![0, 80, 80],
        vec![None, Some("Rock".into()), Some("Wrong".into())],
        vec![None; 3],
    );
    let mut landscape = crate::TestValueExt::test_value(Landscape::new(1, vec![5]));
    landscape.set_world_height(5);
    landscape.set_pixel_grid(grid);

    let mut engine = Engine::with_seed(17);
    engine.set_materials(materials);
    engine.set_landscape(landscape);
    engine
}

fn run_depth_conversion(engine: &mut Engine, source_name: &str) -> (MaterialId, bool) {
    let source = crate::TestValueExt::test_value(engine.materials.id_of(source_name));
    let rock = crate::TestValueExt::test_value(engine.materials.id_of("Rock"));
    let reaction = engine.materials.reaction_for_event(
        Some(source),
        Some(rock),
        MaterialInteractionEvent::PxsPos,
    );
    let mut pixel = pxs::Pxs {
        mat: source,
        x: itofix(0),
        y: itofix(2),
        xdir: itofix(1),
        ydir: itofix(1),
    };
    let (mut x, mut y) = (0, 2);
    let mut pos_changed = false;
    assert!(!engine.execute_pxs_reaction(
        reaction,
        &mut x,
        &mut y,
        0,
        2,
        &mut pixel,
        Some(rock),
        MaterialInteractionEvent::PxsPos,
        &mut pos_changed,
    ));
    (pixel.mat, pos_changed)
}

#[test]
fn negative_conversion_depths_probe_below_for_builtin_and_custom_reactions() {
    for source in ["Natural", "Custom"] {
        let mut blocked = conversion_engine(false);
        let source_id = crate::TestValueExt::test_value(blocked.materials.id_of(source));
        assert_eq!(
            run_depth_conversion(&mut blocked, source),
            (source_id, false),
            "{source} must reject a nonmatching material below"
        );

        let mut converted = conversion_engine(true);
        let water = crate::TestValueExt::test_value(converted.materials.id_of("Water"));
        assert_eq!(
            run_depth_conversion(&mut converted, source),
            (water, true),
            "{source} must accept the matching material below"
        );
    }
}

#[test]
fn negative_dig_ratio_casts_immediately_and_clears_with_one_rotation_draw() {
    let library = crate::TestValueExt::test_value(clonk_resources::MaterialLibrary::parse(
        r#"
            [Material Negative]
            Name=Negative
            Dig2Object=GEM_
            Dig2ObjectRatio=-2

            [Material Zero]
            Name=Zero
            Dig2Object=GEM_
            Dig2ObjectRatio=0

            [Material Positive]
            Name=Positive
            Dig2Object=GEM_
            Dig2ObjectRatio=2
            "#,
    ));
    let materials = MaterialSet::from_resource_library(&library);
    let negative = crate::TestValueExt::test_value(materials.id_of("Negative"));
    let zero = crate::TestValueExt::test_value(materials.id_of("Zero"));
    let positive = crate::TestValueExt::test_value(materials.id_of("Positive"));

    let mut engine = Engine::with_seed(23);
    engine.set_materials(materials);
    crate::TestValueExt::test_value(engine.register_script_definition(
        "DGRR",
        "Digger",
        "#strict\n",
    ));
    crate::TestValueExt::test_value(engine.register_script_definition("GEM_", "Gem", "#strict\n"));
    let digger = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("DGRR")));
    let digger_index = crate::TestValueExt::test_value(engine.find_object_index(digger));
    for material in [negative, zero, positive] {
        engine.objects[digger_index].set_material_content(material, 1);
    }

    let mut expected_rng = engine.rng.clone();
    let _ = expected_rng.random(360);
    engine.process_dig_material_conversions(digger_index, false);

    let digger_index = crate::TestValueExt::test_value(engine.find_object_index(digger));
    assert_eq!(engine.objects[digger_index].material_content(negative), 0);
    assert_eq!(engine.objects[digger_index].material_content(zero), 1);
    assert_eq!(engine.objects[digger_index].material_content(positive), 1);
    assert_eq!(
        engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
            .count(),
        1
    );
    assert_eq!(engine.rng, expected_rng);
}
