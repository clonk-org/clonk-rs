//! Contact snaps the fixed position to the whole pixel
//! (clonk-org/clonk-rs#515).

use super::*;
use crate::lib_test_support::{spawn_fixture, EngineTestExt};
use std::collections::HashMap;

/// `C4Object::DoMovement`'s **unattached** horizontal loop steps one pixel at
/// a time toward `ctcox = fixtoi(fix_x += xdir)`, and on contact does
/// (oracle-src-pinned src/C4Movement.cpp:266-281):
///
/// ```cpp
/// if (iContact = ContactCheck(ctx, y))
/// {
///     fAnyContact = true; iContacts |= t_contact;
///     // Abort horizontal movement
///     ctcox = x; fix_x = itofix(x);
///     ...
/// }
/// ```
///
/// `fix_x = itofix(x)` **discards the sub-pixel remainder**. An implementation
/// that left the fraction in place would leave the object on the same whole
/// pixel — so `fixtoi()` agrees and it looks right — while carrying a
/// different raw `C4Fixed` into the next frame's accumulation. That is exactly
/// the "stops one subpixel earlier on one implementation" desync this issue
/// names, and it is invisible unless the raw value is compared, which is why
/// AGENTS.md requires movement diffs to compare raw `C4Fixed` rather than
/// `fixtoi()`.
///
/// The mover carries a deliberate fractional offset *and* a fractional speed,
/// so a surviving remainder cannot coincide with a clean snap.
#[test]
fn a_horizontal_contact_discards_the_subpixel_remainder() {
    let mut definition = test_definition("MOVR", "Mover", "#strict\n");
    // A right-facing vertex is what `ContactCheck` tests against the wall.
    definition.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
    definition.configure_actions(
        Some("Fly".to_owned()),
        HashMap::from([(
            "Fly".to_owned(),
            // FLIGHT does not set `t_attach`, so movement takes the unattached
            // branch this test is about.
            ActionSpec::for_procedure("FLIGHT").with_next("Fly"),
        )]),
    );

    let mut engine = Engine::with_seed(0);
    // A grid whose index 1 is solid granite; `Landscape::flat` alone has no
    // solid material for `grid_write_byte` to place.
    let mut landscape = Landscape::flat(64, 60);
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        64,
        60,
        vec![0_u8; 64 * 60],
        vec![0, 50],
        vec![None, Some("Granite".to_owned())],
        vec![None; 2],
    ));
    landscape.set_world_height(60);
    engine.set_landscape(landscape);
    engine.register_test_definition(definition);

    let object = spawn_fixture!(engine, "MOVR", with_position: Vector2::new(20, 10), with_action: ActionState::new("Fly"), with_mobile: true);
    let index = engine.test_object_index(object);

    // One solid pixel just right of the mover's contact vertex.
    crate::TestValueExt::test_value(engine.landscape.as_mut()).grid_write_byte(23, 10, 1);

    // Start off the whole pixel and move at a speed that never lands on one,
    // so the pre-contact `fix_x` is guaranteed to carry a remainder.
    engine.objects[index].fixed_position.x = itofix_prec(20_500, 1_000);
    engine.objects[index].set_fixed_velocity(FixedVec2::new(
        itofix_prec(7, 3),
        crate::math::C4Fixed::ZERO,
    ));
    assert_ne!(
        engine.objects[index].fixed_position.x,
        itofix(engine.objects[index].state.position.x),
        "the fixture must start with a remainder, or the assertion below is vacuous"
    );

    let definition_id = engine.objects[index].definition_id.clone();
    let action_library = crate::TestValueExt::test_value(engine.definitions.get(&definition_id))
        .action_library()
        .clone();

    for _ in 0..8 {
        crate::TestValueExt::test_value(engine.exec_object_movement(
            index,
            &action_library,
            &definition_id,
            &[],
        ));
        if engine.objects[index].frame_t_contact != 0 {
            let whole = engine.objects[index].state.position.x;
            assert_eq!(
                engine.objects[index].fixed_position.x,
                itofix(whole),
                "`fix_x = itofix(x)` must discard the sub-pixel remainder on contact, \
                 not merely leave the object on the same whole pixel"
            );
            return;
        }
    }
    panic!("the mover never reached the wall");
}
