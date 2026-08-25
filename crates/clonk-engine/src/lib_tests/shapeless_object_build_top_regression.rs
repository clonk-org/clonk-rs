use super::*;

#[test]
fn a_shapeless_object_takes_the_full_native_build_top_expansion() {
    // C4LArea::Set is fed C4Rect(Left(), Top(), Width(), Height()), and those
    // accessors derive the build-top expansion from Shape.Hgt itself:
    //
    //   addtop() { return max(18 - Shape.Hgt, 0); }
    //   Top()    { return y + Shape.y - addtop(); }
    //   Height() { return Shape.Hgt + addtop(); }
    //
    // (oracle-src-pinned src/C4Object.h:340-344). An object whose definition
    // carries no Shape has Shape.Hgt == 0, so native expands by the full 18
    // and the rect starts at y - 18. C4LArea::Set only substitutes 1 for a
    // zero extent *afterwards*, when it walks the area
    // (src/C4Sector.cpp:249-250).
    //
    // Substituting a 1x1 rect *before* computing the expansion instead yields
    // addtop 17 and a rect starting at y - 17 -- one pixel low, which moves the
    // object across a sector row boundary and shows up as a SectShapeSum
    // difference against a native peer (clonk-org/clonk-rs#1050).
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 200));
    crate::TestValueExt::test_value(engine.register_script_definition(
        "SHPL",
        "Shapeless helper",
        "#strict\n",
    ));
    let id = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("SHPL").with_position(Vector2::new(120, 100))),
    );

    let index = engine
        .find_object_index(id)
        .expect("spawned object is live");
    let object = &engine.objects[index];
    assert!(
        object.current_shape_rect().is_none(),
        "this regression is about the no-Shape path"
    );

    let rect = engine.object_shape_rect(object);
    assert_eq!(rect.y, 100 - 18, "native expands from Shape.Hgt == 0");
    assert_eq!(rect.height, 18, "Shape.Hgt + addtop() == 0 + 18");
}
