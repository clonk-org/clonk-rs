//! Shipped sailboat parity for a definition `SolidMask` across `ChangeDef`.
//!
//! `Sailing.c4d` (`SLBS`) and its parent `Sailboat.c4d` (`SLBT`) both declare
//! `SolidMask=0,42,36,2,0,42`. `RaiseSail` ends in `LandOn`, which calls
//! `ChangeDef(SLBT)`, and because the adopted definition carries the same mask
//! the hull stays in the landscape and crew standing in the boat stay aboard.
//!
//! The engine behaviour under test is the same either way: C++
//! `C4Object::ChangeDef` removes the live mask and puts the new definition's
//! (C4Object.cpp:1220-1240), and `InLiquidAction` (C4Object.cpp:4758-4763)
//! returns the floating boat to `SLBS`. What changed is the content — `SLBT`
//! declared no mask until clonk-org/clonk-rs-content#21 gave it one, so
//! raising the sails used to drop the hull and tip the crew into the water.
//! This pins the corrected shape: adoption, not loss.

use crate::support::real_scenario::{join_local_player, load_tutorial};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, ObjectId, ObjectUpdate, SpawnConfig, Vector2};

/// `SLBS` hull row relative to the object's position: C4SolidMask::Put targets
/// `y + Def->Shape.y + SolidMask.ty` (C4SolidMask.cpp:67-68), and the shipped
/// definition pairs `Offset=-18,-22` with `ty=42`.
const HULL_MASK_OFFSET_Y: i32 = -22 + 42;

/// Tutorial07 ships its return sailboat afloat; settle the scenario so the
/// boat has reached its `Sailing` action before the hull is sampled.
fn settled_sailboat(engine: &mut Engine) -> (ObjectId, Vector2) {
    for _ in 0..30 {
        engine.tick().expect("tutorial07 ticks");
    }
    let boat = engine
        .first_object_for_definition("SLBS")
        .expect("Tutorial07 ships a sailing sailboat");
    let snapshot = engine.test_object_snapshot(boat);
    assert_eq!(snapshot.action.name, "Sailing");
    (boat, snapshot.position)
}

fn crew_on_deck(engine: &mut Engine, owner: i32, at: Vector2) -> ObjectId {
    engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_owner(owner)
            .with_position(Vector2::new(at.x, at.y + 8)),
    )
}

fn hull_is_put(engine: &Engine, at: Vector2) -> bool {
    engine
        .landscape()
        .expect("tutorial07 landscape")
        .is_solid_at(at.x, at.y + HULL_MASK_OFFSET_Y)
}

#[test]
fn the_sailing_hull_mask_carries_crew_standing_in_the_boat() {
    let mut engine = load_tutorial(7, 0);
    let owner = join_local_player(&mut engine, "sailboat hull parity");
    let (_, at) = settled_sailboat(&mut engine);
    assert!(
        hull_is_put(&engine, at),
        "SLBS puts its DefCore SolidMask into the landscape"
    );

    let clonk = crew_on_deck(&mut engine, owner, at);
    for _ in 0..60 {
        engine.tick().expect("sailing ticks");
    }

    let standing = engine.test_object_snapshot(clonk);
    assert_eq!(standing.action.name, "Walk");
    assert!(
        standing.position.y < at.y + HULL_MASK_OFFSET_Y,
        "crew rests above the hull mask, not in the water below it"
    );
}

#[test]
fn raising_the_sails_keeps_the_hull_mask_and_the_crew_aboard() {
    let mut engine = load_tutorial(7, 0);
    let owner = join_local_player(&mut engine, "sailboat hull parity");
    let (boat, at) = settled_sailboat(&mut engine);
    let clonk = crew_on_deck(&mut engine, owner, at);
    for _ in 0..60 {
        engine.tick().expect("sailing ticks");
    }
    assert_eq!(engine.test_object_snapshot(clonk).action.name, "Walk");

    // RaiseSail runs ten phases at Delay=1 and calls LandOn from its EndCall.
    engine
        .apply_object_update(boat, ObjectUpdate::new().with_action("RaiseSail"))
        .expect("the sailing boat accepts RaiseSail");
    for _ in 0..30 {
        engine.tick().expect("raised-sail ticks");
    }

    let landed = engine.test_object_snapshot(boat);
    assert_eq!(landed.definition_id, "SLBT");
    assert_eq!(landed.action.name, "JustLanded");
    assert!(
        hull_is_put(&engine, at),
        "ChangeDef adopts SLBT's matching SolidMask, so the hull survives"
    );
    assert_eq!(
        engine.test_object_snapshot(clonk).action.name,
        "Walk",
        "crew stay on a hull that never left the landscape"
    );

    // JustLanded holds for 150 ticks; OnLand's InLiquidAction then runs
    // Floating, which changes the upright boat back to SLBS.
    for _ in 0..180 {
        engine.tick().expect("re-floating ticks");
    }

    let afloat = engine.test_object_snapshot(boat);
    assert_eq!(afloat.definition_id, "SLBS");
    assert!(
        hull_is_put(&engine, at),
        "and the sailing definition's own mask is in place once it floats"
    );
}
