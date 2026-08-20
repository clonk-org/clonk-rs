//! Shipped catapult payload launch (`Objects.c4d/Vehicles.c4d/Catapult.c4d`).
//!
//! `Projectile()` (`Catapult.c4d/Script.c:51-77`) is the synchronized half of
//! firing: it hands the payload the firing player as controller and killer,
//! `Exit`s it with a computed offset, rotation and velocity, then overwrites
//! both velocity components through `SetXDir`/`SetYDir` at precision 100.
//!
//! Two things about that tail are easy to get wrong and invisible in a
//! fixtoi-only comparison:
//!
//! * the script draws exactly twice from the synchronized stream — `Random(360)`
//!   for the rotation, then one draw inside `RandomX` for the deviation — and in
//!   that order; and
//! * it adds the SAME `iDeviation` to both axes, so the deviation cancels in
//!   `xdir - ydir`. That difference is therefore seed-independent and pins the
//!   velocity arithmetic without pinning the draw values themselves.

use crate::support::real_scenario::{join_local_player, load_tutorial};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, ObjectId, SpawnConfig};
use clonk_script::Value;

/// The shipped `Fire` action is `Length=7, Delay=1` with `EndCall=Projectile`
/// (`Catapult.c4d/ActMap.txt:23-32`), so the payload leaves within a frame or
/// two of the animation ending.
const LAUNCH_FRAMES: usize = 24;

/// One frame of the scenario's default gravity, in the hundredths
/// `SetXDir`/`SetYDir` are denominated in (`C4Physics.h` `GravAccel`, applied
/// once by the movement pass that runs in the same frame as the `EndCall`).
const GRAVITY_HUNDREDTHS_PER_FRAME: i32 = 20;

fn armed_catapult(engine: &mut Engine, owner: i32) -> (ObjectId, ObjectId) {
    let catapult = engine
        .first_object_for_definition("CATA")
        .expect("Tutorial05 ships a catapult");
    let index = engine.test_object_index(catapult);
    engine.objects[index].state.controller = owner;

    let payload = engine.spawn_test_object(
        SpawnConfig::new("ROCK")
            .with_container(catapult)
            .with_owner(owner),
    );
    (catapult, payload)
}

fn contained_in(engine: &Engine, object: ObjectId) -> Option<ObjectId> {
    engine.test_object_snapshot(object).container
}

#[test]
fn firing_hands_the_payload_the_controller_and_leaves_the_catapult() {
    let mut engine = load_tutorial(5, 0);
    let owner = join_local_player(&mut engine, "catapult payload parity");
    let (catapult, payload) = armed_catapult(&mut engine, owner);
    assert_eq!(
        contained_in(&engine, payload),
        Some(catapult),
        "the payload starts inside the catapult"
    );

    let catapult_index = engine.test_object_index(catapult);
    assert_eq!(
        engine.call_test_object_function(catapult_index, "Fire", vec![Value::Bool(true)]),
        Value::Int(1),
        "Fire returns 1 (Catapult.c4d/Script.c:48)"
    );

    for _ in 0..LAUNCH_FRAMES {
        engine.tick().expect("tutorial05 ticks");
        if contained_in(&engine, payload).is_none() {
            break;
        }
    }

    assert_eq!(
        contained_in(&engine, payload),
        None,
        "Exit takes the payload out of the catapult (Catapult.c4d/Script.c:66)"
    );
    assert_eq!(
        engine.test_object_snapshot(payload).controller,
        owner,
        "the payload carries the firing player (Catapult.c4d/Script.c:65)"
    );
}

#[test]
fn the_launch_draws_twice_and_deviates_both_axes_by_the_same_amount() {
    let mut engine = load_tutorial(5, 0);
    let owner = join_local_player(&mut engine, "catapult payload parity");
    let (catapult, payload) = armed_catapult(&mut engine, owner);

    let catapult_index = engine.test_object_index(catapult);
    engine.call_test_object_function(catapult_index, "Fire", vec![Value::Bool(true)]);

    // Tick to the frame the EndCall fires on, sampling the ledger either side
    // of it so the launch's own draws are isolated from the scenario's.
    let mut before = None;
    for _ in 0..LAUNCH_FRAMES {
        let count = engine.rng.count;
        engine.tick().expect("tutorial05 ticks");
        if contained_in(&engine, payload).is_none() {
            before = Some(count);
            break;
        }
    }
    let before = before.expect("the payload leaves within the Fire animation");

    // `Random(360)` then `RandomX(-iPhase * 10 - 20, iPhase * 10 + 20)`, which
    // is one further draw (`Script.c:57,71`). Anything else in the frame would
    // show up here.
    assert_eq!(
        engine.rng.count - before,
        2,
        "the launch frame draws exactly the rotation and the deviation"
    );

    // `SetXDir(iXDir * 100 + iDeviation, ..., 100)` and
    // `SetYDir(iYDir * 100 + iDeviation, ..., 100)` (Script.c:73-74) add the
    // SAME deviation to both axes, so it cancels here — a seed-independent
    // check on the raw C4Fixed values rather than their fixtoi.
    let snapshot = engine.test_object_snapshot(payload);
    let velocity = snapshot
        .fixed_velocity
        .expect("a launched payload carries raw fixed velocity");
    // `iXDir` and `iYDir` are whole pixels per frame before the * 100, so once
    // the shared deviation cancels the difference is a whole multiple of a
    // hundred hundredths. Read at precision 100, which is the denomination
    // SetXDir/SetYDir were given.
    // Read at precision 100, the denomination SetXDir/SetYDir were given.
    let x_hundredths = clonk_engine::math::fixtoi_prec(velocity.x, 100);
    let y_hundredths = clonk_engine::math::fixtoi_prec(velocity.y, 100);
    assert_eq!(
        y_hundredths - x_hundredths,
        GRAVITY_HUNDREDTHS_PER_FRAME,
        "a catapult at rest gives both axes only the deviation, so by the time \
         the payload is sampled they differ by exactly the launch frame's \
         gravity (got {x_hundredths} and {y_hundredths})"
    );
}
