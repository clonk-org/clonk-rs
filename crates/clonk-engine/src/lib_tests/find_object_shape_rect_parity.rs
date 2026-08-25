//! `FindObject`'s shape tests resolve against the object's live `Shape` rect,
//! and this pins that rect's derivation against the pinned oracle.
//!
//! `C4FindObjectAtPoint::Check`, `C4FindObjectAtRect::Check` and
//! `C4FindObjectOnLine::Check` (`C4FindObject.cpp:550-565`) all read
//! `pObj->Shape` offset by the object's position — they share one rect, so a
//! single derivation covers all three. `C4Object::UpdateShape`
//! (`C4Object.cpp:322-344`) builds it: a `Line` definition keeps the
//! definition shape untouched, construction applies `C4Shape::Stretch` or
//! `C4Shape::Jolt` depending on `GrowthType`, and a rotateable object at a
//! non-zero angle replaces the rect with a bounding square.
//!
//! `compat/profile.json` used to record "shape tests use the vertices bounding
//! box rather than the layered shape" as a divergence
//! (clonk-org/clonk-rs#1095). It is not one — the port derives the same rect
//! from the same inputs — and these cases hold that true.

use crate::{transformed_shape_rect, DefinitionRect, FULL_CON};

/// A definition shape with a non-symmetric offset, so a rule that scales the
/// wrong field cannot pass by coincidence.
const DEF_SHAPE: DefinitionRect = DefinitionRect::new(-5, -8, 10, 16);

#[test]
fn full_construction_without_rotation_keeps_the_definition_rect() {
    // UpdateShape only stretches when `Con != FullCon` and only rotates when
    // `r != 0` (C4Object.cpp:331,338-340).
    assert_eq!(
        transformed_shape_rect(Some(DEF_SHAPE), FULL_CON, true, 1, 0),
        Some(DEF_SHAPE)
    );
}

#[test]
fn growth_construction_stretches_every_edge_like_c4shape_stretch() {
    // C4Shape::Stretch scales x, y, Wdt and Hgt alike (C4Shape.cpp:103-110).
    // Half construction is 50%, and C++ truncates toward zero exactly as Rust
    // does, so -5 * 50 / 100 is -2 on both sides.
    assert_eq!(
        transformed_shape_rect(Some(DEF_SHAPE), FULL_CON / 2, true, 0, 0),
        Some(DefinitionRect::new(-2, -4, 5, 8))
    );
}

#[test]
fn non_growth_construction_jolts_only_the_vertical_edges() {
    // C4Shape::Jolt leaves x and Wdt alone (C4Shape.cpp:119-124) — this is the
    // half of the pair a single "scale the rect" implementation gets wrong.
    assert_eq!(
        transformed_shape_rect(Some(DEF_SHAPE), FULL_CON / 2, false, 0, 0),
        Some(DefinitionRect::new(-5, -4, 10, 8))
    );
}

#[test]
fn rotation_replaces_the_rect_with_c4shape_rotates_bounding_square() {
    // C4Shape::Rotate discards the rect for a square of radius
    // `sqrt(x*x + y*y) + 2` centred on the origin (C4Shape.cpp:87-91).
    // sqrt(25 + 64) is 9.43, truncated to 9, so the radius is 11.
    assert_eq!(
        transformed_shape_rect(Some(DEF_SHAPE), FULL_CON, true, 1, 90),
        Some(DefinitionRect::new(-11, -11, 22, 22))
    );
}

#[test]
fn a_non_rotateable_definition_keeps_its_rect_at_any_angle() {
    // UpdateShape guards the rotation on `Def->Rotateable` (C4Object.cpp:338).
    assert_eq!(
        transformed_shape_rect(Some(DEF_SHAPE), FULL_CON, true, 0, 90),
        Some(DEF_SHAPE)
    );
}

#[test]
fn construction_and_rotation_compose_in_c4objects_order() {
    // UpdateShape stretches first and rotates the stretched rect
    // (C4Object.cpp:331-340), so the radius comes from the scaled offsets:
    // sqrt(4 + 16) is 4.47 -> 4, radius 6.
    assert_eq!(
        transformed_shape_rect(Some(DEF_SHAPE), FULL_CON / 2, true, 1, 45),
        Some(DefinitionRect::new(-6, -6, 12, 12))
    );
}
