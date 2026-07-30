//! `planet/System.c4g/BirdFlight.c` — the port's continuous bird-flight
//! controller. It is a deliberate divergence from the shipped
//! `Objects.c4d/Animals.c4d/Bird.c4d/Script.c` steering policy, so no oracle
//! differential covers it and these are the only tests that pin it.

use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::{CommandDirection, Engine, ObjectId};
use clonk_script::Value;

/// Wipfrace declares `Animal=BIRD=10;` (Scenario.txt:56), so InitAnimals
/// places a real flock over generated terrain.
const BIRD_SCENARIO: &str = "Races.c4f/Wipfrace.c4s";
const APPEND: &str = "BirdFlight.c";
/// `[Physical] Float=200` becomes the DFA_FLOAT per-axis velocity bound
/// `FIXED100(200)` = 2.0 px/frame (C4Object.cpp:5284-5285).
const FLOAT_CLAMP_RAW: i32 = 2 * (1 << 16);

fn local_int(engine: &Engine, object: ObjectId, name: &str) -> Option<i32> {
    match engine.object_snapshot(object)?.local_vars.get(name) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

fn birds(engine: &Engine) -> Vec<ObjectId> {
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "BIRD")
        .map(|object| object.id)
        .collect()
}

fn tick(engine: &mut Engine, frames: u32) {
    for _ in 0..frames {
        engine.tick_without_snapshot().expect("the frame executes");
    }
}

/// The controller has to reach birds that arrive as loaded saved objects and
/// never run Initialize, so it installs itself from the shipped per-frame
/// `Survive` PhaseCall and from `Activity` rather than from a constructor.
#[test]
fn bird_flight_controller_installs_itself_on_every_placed_bird() {
    let mut engine = prepare_installed_scenario(BIRD_SCENARIO, 0).instantiate();
    let flock = birds(&engine);
    assert!(
        !flock.is_empty(),
        "Wipfrace places a flock through its [Animals] section"
    );

    tick(&mut engine, 40);

    for bird in flock {
        let object = engine.object_snapshot(bird).expect("the bird remains live");
        assert!(
            object
                .effects
                .iter()
                .any(|effect| effect.name == "BirdFlight"),
            "bird {bird:?} carries the per-frame controller effect"
        );
        assert!(
            local_int(&engine, bird, "flight_agility").unwrap_or(0) > 0,
            "bird {bird:?} is seeded by FxBirdFlightStart"
        );
        assert!(
            matches!(local_int(&engine, bird, "flight_heading"), Some(0..=359)),
            "bird {bird:?} carries a heading in Clonk orientation"
        );
    }
}

/// The point of the whole exercise: the shipped script re-rolls a pure-axis
/// ComDir once per 35-frame timer, so its course is a sawtooth. A flown
/// heading turns by a bounded amount per frame instead.
#[test]
fn bird_heading_turns_continuously_instead_of_snapping_to_an_axis() {
    let mut engine = prepare_installed_scenario(BIRD_SCENARIO, 0).instantiate();
    let bird = *birds(&engine).first().expect("Wipfrace places a bird");

    tick(&mut engine, 40);

    // Cruising steps are small — wander is +-8 degrees per think, separation
    // 12, flee 25. Terrain and contact reflections are deliberately large and
    // can land on the same frame as another, so the invariant is that big
    // steps are the exception, not that they never happen.
    let mut previous = local_int(&engine, bird, "flight_heading").expect("seeded heading");
    let mut samples = 0;
    let mut abrupt = 0;
    let mut axis_aligned = 0;
    for _ in 0..150 {
        tick(&mut engine, 4);
        let Some(heading) = local_int(&engine, bird, "flight_heading") else {
            break;
        };
        let delta = (heading - previous + 540).rem_euclid(360) - 180;
        if delta.abs() > 30 {
            abrupt += 1;
        }
        if heading % 90 == 0 {
            axis_aligned += 1;
        }
        previous = heading;
        samples += 1;
    }

    assert!(samples > 100, "the bird stayed alive long enough to sample");
    assert!(
        abrupt * 4 < samples,
        "{abrupt}/{samples} four-frame steps exceeded 30 degrees; the course \
         should be flown, not snapped"
    );
    // The shipped policy could only ever hold a pure axis; this one should
    // essentially never sit exactly on one.
    assert!(
        axis_aligned * 4 < samples,
        "{axis_aligned}/{samples} samples sat exactly on an axis, which is the \
         shipped sawtooth this replaces"
    );
}

/// The controller writes velocity directly at precision 100 and relies on
/// every write landing inside the DFA_FLOAT clamp of FIXED100(Float) = 200,
/// so that it — and not ExecAction — is the sole velocity authority.
#[test]
fn bird_velocity_stays_inside_the_float_physical_clamp() {
    let mut engine = prepare_installed_scenario(BIRD_SCENARIO, 0).instantiate();
    let flock = birds(&engine);

    for _ in 0..120 {
        tick(&mut engine, 5);
        for &bird in &flock {
            let Some(velocity) = engine
                .object_snapshot(bird)
                .and_then(|object| object.fixed_velocity)
            else {
                continue;
            };
            // Compare raw C4Fixed, not fixtoi: FIXED100(Float) with Float=200
            // is exactly 2.0 px/frame, and the clamp is per axis.
            let (vx, vy) = (velocity.x.val(), velocity.y.val());
            assert!(
                vx.abs() <= FLOAT_CLAMP_RAW && vy.abs() <= FLOAT_CLAMP_RAW,
                "bird {bird:?} raw velocity ({vx}, {vy}) escaped the \
                 FIXED100(200) clamp of {FLOAT_CLAMP_RAW}"
            );
        }
    }
}

/// The controller introduces no synchronized draw *site* of its own: per-bird
/// variation comes from `ObjectNumber()`, a synced integer that survives
/// save/load, and every `Random()` in the file is a shipped draw reproduced in
/// the shipped order under the shipped condition.
///
/// That deliberately is NOT a claim that `RandomCount` tracks the shipped bird
/// forever. `Activity`'s branches read world state the controller changes —
/// `GetXDir`, `GetAction`, `GetCommand` — so the *path* through those shipped
/// draw sites diverges as soon as the flight path does, which is immediately.
/// What is pinned here is the part that scenario assertions actually depend
/// on: the controller installs itself lazily rather than from `Initialize`, so
/// scenario init — worldgen, animal placement, the whole pre-tick ledger — is
/// draw-for-draw identical to the shipped content.
#[test]
fn bird_flight_controller_adds_no_draw_site_and_leaves_scenario_init_untouched() {
    let source = std::fs::read_to_string(
        crate::support::real_scenario::repository_root()
            .join("planet/System.c4g")
            .join(APPEND),
    )
    .expect("the controller source is readable");

    // Every Random() below is one of the shipped draws; the controller's own
    // variation must never reach for the synchronized stream.
    let draws: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter(|line| line.contains("Random(") || line.contains("RandomX("))
        .collect();
    // Eight in Activity and one each in ContactLeft/ContactRight (those two
    // carry a second, short-circuited draw on the same line), matching
    // Bird.c4d/Script.c:28,66,68,75,79,82,85,89,258,267 one for one. CatchBlow
    // keeps its own Random(3) because the controller does not override it.
    assert_eq!(
        draws.len(),
        10,
        "expected exactly the shipped draw sites reproduced from \
         Bird.c4d/Script.c, found: {draws:#?}"
    );

    let prepared = prepare_installed_scenario(BIRD_SCENARIO, 0);
    let with_controller = prepared.instantiate();
    let shipped = prepared.instantiate_without_system_script(APPEND);
    assert!(
        !birds(&with_controller).is_empty(),
        "the A/B needs a flock to exercise"
    );
    assert_eq!(
        with_controller.debug_rng_clone().count,
        shipped.debug_rng_clone().count,
        "scenario init must spend the same draws either way, so every \
         worldgen and placement assertion in a bird-bearing scenario holds"
    );
}

/// Two runs from one seed must agree exactly. This is the guard against a
/// stray unsynchronized draw or client-local state creeping into the
/// controller, both of which would be desyncs rather than test failures.
#[test]
fn bird_flight_is_reproducible_from_a_fixed_seed() {
    let prepared = prepare_installed_scenario(BIRD_SCENARIO, 7);
    let mut first = prepared.instantiate();
    let mut second = prepared.instantiate();

    tick(&mut first, 400);
    tick(&mut second, 400);

    let left = first.snapshot();
    let right = second.snapshot();
    let sample = |snapshot: &clonk_engine::SimulationSnapshot| {
        let mut rows: Vec<(String, i32, i32)> = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "BIRD")
            .map(|object| {
                let fixed = object
                    .fixed_position
                    .expect("a live bird carries fixed position");
                (format!("{:?}", object.id), fixed.x.val(), fixed.y.val())
            })
            .collect();
        rows.sort();
        rows
    };

    assert!(!sample(&left).is_empty(), "birds survive 400 frames");
    assert_eq!(sample(&left), sample(&right));
    assert_eq!(
        first.debug_rng_clone().count,
        second.debug_rng_clone().count
    );
}

/// The shipped `ContactRight` is a verbatim copy of `ContactLeft`
/// (Bird.c4d/Script.c:257-260,266-269): its `COMD_Right + Random(2)*2-1`
/// evaluates to COMD_UpRight (2) or COMD_DownRight (4) — COMD_Right is 3 — and
/// so steers the bird straight back into the wall that raised the callback. It
/// only fires on the one-in-five `!Random(5)` branch, after `TurnLeft()` has
/// already set a correct COMD_Left, which is why the shipped bird grinds a
/// right-facing cliff intermittently rather than always.
///
/// This pins both halves: that the shipped bug is real, and that reflecting
/// off the contact side removes it on every branch.
#[test]
fn contact_right_reflects_away_from_the_wall_instead_of_back_into_it() {
    let prepared = prepare_installed_scenario(BIRD_SCENARIO, 0);

    let mut shipped = prepared.instantiate_without_system_script(APPEND);
    let shipped_bird = *birds(&shipped).first().expect("Wipfrace places a bird");
    let index = shipped
        .find_object_index(shipped_bird)
        .expect("the bird has an index");
    let mut steered_into_the_wall = 0;
    for _ in 0..60 {
        shipped
            .call_object_function(index, "ContactRight", Vec::new())
            .expect("the shipped ContactRight runs");
        if matches!(
            shipped
                .object_snapshot(shipped_bird)
                .expect("the bird remains live")
                .command_direction,
            CommandDirection::UpRight | CommandDirection::DownRight
        ) {
            steered_into_the_wall += 1;
        }
    }
    assert!(
        steered_into_the_wall > 0,
        "the shipped ContactRight is expected to steer back into the wall on \
         its one-in-five branch"
    );

    let mut engine = prepared.instantiate();
    let bird = *birds(&engine).first().expect("Wipfrace places a bird");
    tick(&mut engine, 40);
    let index = engine
        .find_object_index(bird)
        .expect("the bird has an index");
    for call in 0..60 {
        engine
            .call_object_function(index, "ContactRight", Vec::new())
            .expect("the resteered ContactRight runs");
        // Clonk orientation: 0 is up and angles run clockwise, so a heading
        // that clears a right-side wall is strictly between 180 and 360.
        let heading = local_int(&engine, bird, "flight_heading").expect("a driven heading");
        assert!(
            (185..=355).contains(&heading),
            "call {call}: after a right-side contact the heading must point \
             away from the wall, got {heading}"
        );
    }
}
