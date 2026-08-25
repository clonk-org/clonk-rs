use super::*;

/// `C4Object::ExecLife`'s breathing supply check reads the mouth pixel with
/// `GBackMat`, and its first arm is "forcefields are breathable"
/// (oracle-src-pinned `src/C4Object.cpp:883-899`):
///
/// ```cpp
/// if (GBackMat(x, y + Shape.y / 2) == MVehic) Breathe = true;
/// ```
///
/// `GBackMat` is `Pix2Mat[GetPix]`, and `GetPix` answers **MCVehic** past a
/// closed border (`src/C4Landscape.h:144-161`). So an object off the top of a
/// closed map reads the Vehicle material and breathes.
///
/// The port read the mouth with `material_at`, which has no border rule and
/// answers `None` out of bounds. The vehicle arm therefore missed, the chain
/// fell through to the semi-solid arm — where `is_solid_at` *does* apply the
/// border and reports solid — and the object suffocated instead of breathing.
///
/// That is not only a state difference: the `!Breathe` arm spends
/// `Random(5)` on the bubble, so the port drew where a stock C++ client did
/// not and the two ledgers parted (clonk-org/clonk-rs#1100, frame 5 of
/// `Fantasy.c4f/Alchemy.c4s`).
#[test]
fn an_object_past_a_closed_border_breathes_the_vehicle_material() {
    let mut engine = Engine::with_seed(0x1100);
    // MVehic is resolved from the material set by name, exactly as
    // C4Game::InitMaterialTexture does (src/C4Game.cpp:1669).
    let library = clonk_resources::MaterialLibrary::parse(
        // Earth first, so it -- not Vehicle -- is the default ground material.
        // With Vehicle as the only material the two reads coincide and the
        // test cannot tell the border rule from the column fallback.
        "[Material]\nName=Earth\nDensity=50\n\n[Material]\nName=Vehicle\n",
    )
    .expect("the vehicle material library parses");
    engine.materials = crate::material::MaterialSet::from_resource_library(&library);
    let vehicle = engine.materials.id_of("Vehicle");
    assert!(vehicle.is_some(), "the Vehicle material is registered");

    let mut landscape = Landscape::flat(200, 120);
    landscape.set_vehicle_material(vehicle);
    // A closed top is what makes GetPix answer MCVehic above the map; with the
    // default open top it answers sky and the third arm breathes instead
    // (C4Landscape.cpp:67-71 sets these from Scenario.txt).
    landscape.set_border_open(0, 0, false, false);
    engine.set_landscape(landscape);

    crate::TestValueExt::test_value(engine.register_script_definition(
        "BRTH",
        "Breather",
        "#strict\n",
    ));
    // Above the top border. `Landscape::flat` leaves the top closed, so native
    // reads MCVehic here rather than sky.
    let id = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("BRTH").with_position(Vector2::new(100, -10))),
    );
    let index = engine.find_object_index(id).expect("the object is live");
    // The premise: the mouth pixel is past the closed top, where native's
    // GetPix answers MCVehic and GBackMat therefore answers the Vehicle
    // material.
    assert_eq!(engine.objects[index].state.position.y, -10);
    assert_eq!(
        engine
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.border_material_at(100, -10)),
        vehicle,
        "the closed top border reads as the Vehicle material"
    );
    assert_eq!(
        engine
            .landscape
            .as_ref()
            .map(|l| (l.is_solid_at(100, -10), l.is_liquid_at(100, -10))),
        Some((true, false)),
        "the closed border is solid to the semi-solid arm"
    );
    engine.objects[index].state.alive = true;

    // The breathing block is gated on `!Tick5`, so frame 5 runs it. What is
    // pinned is the *draw*, not the breath: taking the supply arm leaves
    // breath at the object's maximum, which is zero without physicals, so a
    // breath assertion cannot tell the two arms apart. The `Random(5)` can --
    // only the `!Breathe` arm spends it, and spending it out of step with a
    // stock C++ client is the whole defect.
    let before = engine.rng.count;
    crate::TestValueExt::test_value(engine.exec_object_life(index, 5).ok());

    assert_eq!(
        engine.rng.count, before,
        "the vehicle arm breathes, so the bubble's Random(5) is never reached"
    );
}
