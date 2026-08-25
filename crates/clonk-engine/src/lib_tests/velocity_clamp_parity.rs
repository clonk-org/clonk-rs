//! Ordinary content must not be subject to a terminal-speed bound, because the
//! pinned oracle has none.
//!
//! `C4Object::DoMovement` (`C4Movement.cpp:215-226`) states its only
//! restriction — `if (Def->NoHorizontalMove) xdir = 0;` — and then adds `xdir`
//! and `ydir` to the position unchanged. Gravity is applied unconditionally
//! (`C4Movement.cpp:649`, `C4Object.cpp:4674`), and nothing bounds the result:
//! `MaxFallSpeed`, `MaxSpeed` and `SpeedLimit` appear nowhere in the pin except
//! one comment saying MaxSpeed *is ignored* (`C4PXS.cpp:77`).
//!
//! The port's `PhysicsSettings` bounds are a synthetic-fixture knob — they come
//! from `PhysicsManifest`, a serde struct with snake_case fields rather than a
//! C4 `Scenario.txt` `[Landscape]` section — and real content, which never sets
//! them, inherited the fixture defaults. A falling object stopped at exactly
//! `itofix(12)` while C++ accelerated past it, which desynced `Goldrace`,
//! `Skyrace` and `Canyon` in the live shadow diff (clonk-org/clonk-rs#1112).

use crate::PhysicsSettings;
use clonk_engine_core::math::{itofix, FixedVec2};

/// Faster than any bound the fixtures impose, in every axis and both signs.
const FAST: i32 = 40;

fn clamped(settings: PhysicsSettings, velocity: FixedVec2) -> FixedVec2 {
    let mut velocity = velocity;
    settings.clamp_fixed_velocity(&mut velocity);
    velocity
}

#[test]
fn default_physics_leaves_a_falling_velocity_untouched() {
    let falling = FixedVec2::new(itofix(FAST), itofix(FAST));
    assert_eq!(clamped(PhysicsSettings::default(), falling), falling);
}

#[test]
fn default_physics_leaves_a_rising_velocity_untouched() {
    let rising = FixedVec2::new(itofix(-FAST), itofix(-FAST));
    assert_eq!(clamped(PhysicsSettings::default(), rising), rising);
}

#[test]
fn a_fixture_that_asks_for_a_bound_still_gets_one() {
    // The synthetic manifest keeps the knob; only the default changed.
    let bounded = PhysicsSettings::new(1, 12, -20);
    let falling = FixedVec2::new(itofix(FAST), itofix(FAST));
    assert_eq!(clamped(bounded, falling).y, itofix(12));
}

#[test]
fn a_fixture_bound_still_applies_to_rising_and_horizontal_motion() {
    let bounded = PhysicsSettings::new(1, 12, -20);
    let rising = FixedVec2::new(itofix(-FAST), itofix(-FAST));
    assert_eq!(clamped(bounded, rising).y, itofix(-20));
    assert_eq!(
        clamped(bounded, rising).x,
        itofix(-PhysicsSettings::DEFAULT_MAX_HORIZONTAL_SPEED)
    );
}
