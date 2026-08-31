//! `planet/System.c4g/BirdFlight.c` — the port's continuous bird-flight
//! controller. It is a deliberate divergence from the shipped
//! `Objects.c4d/Animals.c4d/Bird.c4d/Script.c` steering policy, so no oracle
//! differential covers it and these are the only tests that pin it.

use crate::support::real_scenario::{join_local_player, prepare_installed_scenario};
use crate::support::EngineTestExt;
use clonk_core::log_target::SCRIPT_DEBUG_LOG_TARGET;
use clonk_engine::{
    CommandDirection, EffectState, Engine, ObjectId, ObjectUpdate, SpawnConfig, Vector2,
};
use clonk_script::Value;
use std::fmt;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{subscriber, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::Registry;

/// Wipfrace declares `Animal=BIRD=10;` (Scenario.txt:56), so InitAnimals
/// places a real flock over generated terrain.
const BIRD_SCENARIO: &str = "Races.c4f/Wipfrace.c4s";
/// S2Stylands overloads `BIRD` with a folder-local definition whose Activity
/// callback owns combat and energy behavior rather than stock bird steering.
const CUSTOM_BIRD_SCENARIO: &str = "Collection.c4f/Knights.c4f/S2Stylands.c4s";
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
}

#[derive(Clone)]
struct ScriptWarningLayer {
    messages: Arc<Mutex<Vec<String>>>,
}

impl<S> Layer<S> for ScriptWarningLayer
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN
            || event.metadata().target() != SCRIPT_DEBUG_LOG_TARGET
        {
            return;
        }
        let mut visitor = ScriptWarningVisitor::default();
        event.record(&mut visitor);
        if let Some(message) = visitor.message {
            crate::support::TestValueExt::test_value(self.messages.lock()).push(message);
        }
    }
}

#[derive(Default)]
struct ScriptWarningVisitor {
    message: Option<String>,
}

impl Visit for ScriptWarningVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        }
    }
}

fn capture_script_warnings<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = Registry::default().with(ScriptWarningLayer {
        messages: Arc::clone(&messages),
    });
    let result = subscriber::with_default(subscriber, run);
    let captured = crate::support::TestValueExt::test_value(messages.lock()).clone();
    (result, captured)
}

#[test]
fn folder_local_bird_activity_runs_without_stock_flight_steering() {
    let prepared = prepare_installed_scenario(CUSTOM_BIRD_SCENARIO, 0);
    let mut engine = prepared.instantiate();
    let mut authored = prepared.instantiate_without_system_script(APPEND);
    join_local_player(&mut engine, "S2Stylands bird callback");
    join_local_player(&mut authored, "S2Stylands bird callback");
    let bird = *crate::support::TestValueExt::test_value(birds(&engine).first());
    let authored_bird = *crate::support::TestValueExt::test_value(birds(&authored).first());
    let index = engine.test_object_index(bird);
    let authored_index = authored.test_object_index(authored_bird);
    let rng_before = engine.debug_rng_clone().count;
    let authored_rng_before = authored.debug_rng_clone().count;

    engine.call_test_object_function(index, "Activity", Vec::new());
    authored.call_test_object_function(authored_index, "Activity", Vec::new());

    assert_eq!(
        local_int(&engine, bird, "actCnt"),
        local_int(&authored, authored_bird, "actCnt"),
        "the folder-local BIRD::Activity callback must remain in the overload chain"
    );
    assert_eq!(local_int(&engine, bird, "actCnt"), Some(1));
    assert!(
        engine
            .test_object_snapshot(bird)
            .effects
            .iter()
            .all(|effect| effect.name != "BirdFlight"),
        "the stock controller must not attach to an incompatible BIRD"
    );
    assert_eq!(
        engine.debug_rng_clone().count - rng_before,
        authored.debug_rng_clone().count - authored_rng_before,
        "rejecting an incompatible bird must not consume stock steering draws"
    );

    let (_, warnings) = capture_script_warnings(|| {
        tick(&mut engine, 600);
        tick(&mut authored, 600);
    });
    assert!(
        warnings
            .iter()
            .all(|warning| !warning.contains("BirdFlight")),
        "the delegated folder-local callbacks must not fail over 600 frames: {warnings:?}"
    );
    assert_eq!(
        local_int(&engine, bird, "actCnt"),
        local_int(&authored, authored_bird, "actCnt"),
        "the authored periodic callback state stays aligned with the control"
    );
    assert!(
        local_int(&engine, bird, "actCnt").is_some_and(|count| count > 1),
        "the 600-frame window exercises the authored periodic Activity callback"
    );
    assert_eq!(
        engine.debug_rng_clone().count,
        authored.debug_rng_clone().count,
        "the compatibility append must not perturb the scenario RNG ledger"
    );
    let controlled = engine.test_object_snapshot(bird);
    let authored = authored.test_object_snapshot(authored_bird);
    assert_eq!(controlled.position, authored.position);
    assert_eq!(controlled.fixed_velocity, authored.fixed_velocity);
    assert_eq!(controlled.energy, authored.energy);
}

#[test]
fn folder_local_bird_rejects_new_and_removes_restored_stock_flight_effects() {
    let prepared = prepare_installed_scenario(CUSTOM_BIRD_SCENARIO, 0);
    let mut engine = prepared.instantiate();
    join_local_player(&mut engine, "S2Stylands stale bird controller");
    let bird = *crate::support::TestValueExt::test_value(birds(&engine).first());
    let index = engine.test_object_index(bird);

    assert_eq!(
        engine.call_test_object_function(
            index,
            "FxBirdFlightStart",
            vec![Value::Object(bird.as_u64()), Value::Int(1), Value::Int(1),],
        ),
        Value::Int(1),
        "temporary Start remains a no-op during effect suspension and restore"
    );
    assert_eq!(
        local_int(&engine, bird, "flight_compatible"),
        None,
        "temporary Start must not materialize the definition classification"
    );
    assert_eq!(
        engine.call_test_object_function(
            index,
            "FxBirdFlightStart",
            vec![Value::Object(bird.as_u64()), Value::Int(1), Value::Int(0),],
        ),
        Value::Int(-1),
        "a fresh effect must be denied before it can seed stock steering state"
    );
    assert_eq!(
        local_int(&engine, bird, "flight_compatible"),
        Some(-1),
        "the folder-local definition classification is cached on the bird"
    );

    // Older saves can carry a BirdFlight effect without the compatibility
    // cache. Restore that exact shape: loaded effects have already completed
    // Start, so Timer is the last line of defence against stale steering.
    let mut state = engine.capture_state();
    let saved_bird = crate::support::TestValueExt::test_value(
        state
            .objects
            .iter_mut()
            .find(|object| object.snapshot.id == bird),
    );
    saved_bird.snapshot.local_vars.remove("flight_compatible");
    let mut stale = EffectState::new("BirdFlight")
        .with_priority(1)
        .with_interval(1)
        .with_command_target(Some(bird.as_u64() as i32))
        .with_command_id(Some("BIRD"));
    stale.start_dispatched = true;
    saved_bird.snapshot.effects.push(stale);
    crate::support::TestValueExt::test_value(engine.restore_state(&state));
    assert!(
        engine
            .test_object_snapshot(bird)
            .effects
            .iter()
            .any(|effect| effect.name == "BirdFlight"),
        "the restored pre-fix save starts with the stale controller"
    );

    tick(&mut engine, 1);

    assert_eq!(
        engine
            .test_object_snapshot(bird)
            .effects
            .iter()
            .find(|effect| effect.name == "BirdFlight")
            .map(|effect| effect.priority),
        Some(0),
        "the first timer callback marks the stale incompatible effect dead"
    );
    assert_eq!(local_int(&engine, bird, "flight_compatible"), Some(-1));

    tick(&mut engine, 1);
    assert!(
        engine
            .test_object_snapshot(bird)
            .effects
            .iter()
            .all(|effect| effect.name != "BirdFlight"),
        "the next live-list pass unlinks the dead stale effect"
    );
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
        let object = engine.test_object_snapshot(bird);
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
    let bird = *crate::support::TestValueExt::test_value(birds(&engine).first());

    tick(&mut engine, 40);

    // Cruising steps are small — wander is +-8 degrees per think, separation
    // 12, flee 25. Terrain and contact reflections are deliberately large and
    // can land on the same frame as another, so the invariant is that big
    // steps are the exception, not that they never happen.
    let mut previous =
        crate::support::TestValueExt::test_value(local_int(&engine, bird, "flight_heading"));
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
    let source = crate::support::TestValueExt::test_value(std::fs::read_to_string(
        crate::support::real_scenario::repository_root()
            .join("planet/System.c4g")
            .join(APPEND),
    ));

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
                let fixed = crate::support::TestValueExt::test_value(object.fixed_position);
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
    let shipped_bird = *crate::support::TestValueExt::test_value(birds(&shipped).first());
    let index = shipped.test_object_index(shipped_bird);
    let mut steered_into_the_wall = 0;
    for _ in 0..60 {
        shipped.call_test_object_function(index, "ContactRight", Vec::new());
        if matches!(
            shipped.test_object_snapshot(shipped_bird).command_direction,
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
    let bird = *crate::support::TestValueExt::test_value(birds(&engine).first());
    tick(&mut engine, 40);
    let index = engine.test_object_index(bird);
    for call in 0..60 {
        engine.call_test_object_function(index, "ContactRight", Vec::new());
        // Clonk orientation: 0 is up and angles run clockwise, so a heading
        // that clears a right-side wall is strictly between 180 and 360.
        let heading =
            crate::support::TestValueExt::test_value(local_int(&engine, bird, "flight_heading"));
        assert!(
            (185..=355).contains(&heading),
            "call {call}: after a right-side contact the heading must point \
             away from the wall, got {heading}"
        );
    }
}

/// Startle: the shipped bird has no notion of a player at all — it never
/// searches for a Clonk and never reacts to one. A bird that scatters when you
/// walk under it is most of what makes a flock read as alive, so this pins
/// that the flee arm actually fires and actually moves the bird away.
#[test]
fn birds_startle_and_flee_when_a_clonk_comes_close() {
    let mut engine = prepare_installed_scenario(BIRD_SCENARIO, 0).instantiate();
    let owner = join_local_player(&mut engine, "Bird startle");
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // Pick a bird that is airborne and settled, then put the crew right under
    // it. `Placement=2` spawns birds in open air, so this stays clear of the
    // terrain arm, which would otherwise win the priority order.
    let bird = *crate::support::TestValueExt::test_value(birds(&engine).first());
    tick(&mut engine, 40);
    let perch = engine.test_object_snapshot(bird).position;

    assert_eq!(
        local_int(&engine, bird, "flight_alarm").unwrap_or(0),
        0,
        "an undisturbed bird carries no alarm"
    );

    // Hold the Clonk under the bird for a few think intervals; it is in free
    // fall, so it has to be re-placed each frame to stay inside the radius.
    let mut alarmed = 0;
    for _ in 0..24 {
        crate::support::TestValueExt::test_value(engine.apply_object_update(
            clonk,
            ObjectUpdate::new().with_position(Vector2::new(perch.x, perch.y + 20)),
        ));
        tick(&mut engine, 1);
        alarmed = alarmed.max(local_int(&engine, bird, "flight_alarm").unwrap_or(0));
    }

    assert!(
        alarmed > 0,
        "the bird never noticed a Clonk inside its startle radius"
    );

    // Alarm raises cruise to 190 and agility to 6, so a startled bird should
    // put real distance between itself and where the Clonk was.
    let before = engine.test_object_snapshot(bird).position;
    tick(&mut engine, 60);
    let after = engine.test_object_snapshot(bird).position;
    let near = (before.x - perch.x).pow(2) + (before.y - perch.y).pow(2);
    let far = (after.x - perch.x).pow(2) + (after.y - perch.y).pow(2);
    assert!(
        far > near,
        "a startled bird should leave: {near} -> {far} squared pixels from the \
         Clonk"
    );
}

/// Flocking: the shipped bird never looks for another bird, so N birds are N
/// independent random walks that routinely overlap pixel for pixel. The
/// controller adds separation with weak alignment (and deliberately no
/// cohesion, which is the term that collapses a flock).
#[test]
fn birds_separate_from_neighbours_that_start_on_top_of_each_other() {
    let mut engine = prepare_installed_scenario(BIRD_SCENARIO, 0).instantiate();
    let seed_bird = *crate::support::TestValueExt::test_value(birds(&engine).first());
    tick(&mut engine, 40);
    let origin = engine.test_object_snapshot(seed_bird).position;

    // Stack three fresh birds within a couple of pixels of each other, well
    // inside the 90px separation radius.
    let stacked: Vec<ObjectId> = (0..3)
        .map(|offset| {
            engine.spawn_test_object(
                SpawnConfig::new("BIRD")
                    .with_position(Vector2::new(origin.x + offset, origin.y + offset)),
            )
        })
        .collect();

    let spread = |engine: &Engine| -> i32 {
        let points: Vec<Vector2> = stacked
            .iter()
            .filter_map(|&bird| engine.object_snapshot(bird).map(|object| object.position))
            .collect();
        let mut worst = 0;
        for (index, left) in points.iter().enumerate() {
            for right in &points[index + 1..] {
                worst = worst.max((left.x - right.x).pow(2) + (left.y - right.y).pow(2));
            }
        }
        worst
    };

    assert!(spread(&engine) <= 32, "the birds start stacked");
    tick(&mut engine, 120);
    assert_eq!(
        stacked
            .iter()
            .filter(|&&bird| engine.object_snapshot(bird).is_some())
            .count(),
        3,
        "all three birds survive the separation window"
    );
    assert!(
        spread(&engine) > 32,
        "stacked birds should push apart, widest separation stayed at {}",
        spread(&engine)
    );
}
