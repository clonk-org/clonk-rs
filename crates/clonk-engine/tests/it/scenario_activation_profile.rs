//! Manual probe: where a scenario activation's wall time goes.
//!
//! clonk-org/clonk-rs#293 exists because activation is a major user-visible
//! cost with no stage-level tracker. The evidence it was opened on is a
//! sampled profile in which `Scenario::apply` was 51% of samples and 99% of
//! *that subtree* was object-placement callbacks — a share of a subtree, with
//! no denominator, which is why the issue says outright that it "does not
//! establish that callbacks are 99% of every activation stage".
//!
//! `ActivationTimings` records six stages inside the apply interval. This
//! reports them against the interval they sit in, so each one becomes a share
//! and the remainder no stage claims becomes visible.
//!
//! It reports numbers and asserts nothing about them, so it is `#[ignore]`d
//! like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!     --run-ignored all --no-capture -E 'test(scenario_activation_profile::)'
//! ```
//!
//! # Recorded measurement
//!
//! Warm process and warm filesystem cache, seed 0, one aarch64 host at commit
//! `8a6408b33`. Cold-cache numbers are a separate acceptance criterion and are
//! not claimed here.
//!
//! ## ClonkMars `03_Chaos` — the scenario the issue names
//!
//! User-visible interval **12.90 s**, of which `apply` is **12.26 s** (95%).
//!
//! | stage | span | share of apply |
//! |---|---|---|
//! | `environment_placement` | 12.097 s | **98%** |
//! | `definition_registration` | 56.7 ms | 0% |
//! | `scenario_script` | 18.9 ms | 0% |
//! | `landscape` | 2.8 ms | 0% |
//! | `materials` | 172 µs | 0% |
//! | `definition_scripts` | 28.8 µs | 0% |
//! | `object_placement` | 27.4 µs | 0% |
//! | `post_init_map_callbacks` | 84 ns | 0% |
//! | unattributed | 89.6 ms | 0% |
//!
//! **Activation is `run_legacy_init_placements`.** InitVegetation, InitInEarth,
//! InitAnimals, InitEnvironment, InitRules and InitGoals — and every
//! `Initialize` the objects they place run — are 98% of the apply interval
//! (C4Game.cpp:2493-2503).
//!
//! This confirms the *shape* of the sampled profile the issue was opened on
//! and corrects what it names. Placement callbacks do dominate, but not the
//! ones `ObjectPlacement` covers: `Objects.txt` placement is **27 µs**, five
//! orders of magnitude below the environment placers. An optimization aimed at
//! the stage the old reading named would have moved nothing.
//!
//! Before `environment_placement` existed, the six recorded stages accounted
//! for 0.18% of this interval and 99% was unattributed — which is exactly what
//! the remainder is for.
//!
//! ## Tutorial 1 — the contrast
//!
//! User-visible interval **424 ms**, of which `apply` is **81 ms (19%)**.
//! Inside apply the split is `scenario_script` 33.9 ms (41%) and
//! `definition_registration` 30.1 ms (37%); `environment_placement` is 3.5 µs.
//!
//! **The stage that dominates a large scenario does not dominate a small one,
//! and neither is where a small scenario's time goes at all**: 81% of
//! Tutorial 1's activation is in *load*, before `apply` begins. `Scenario::load`
//! has no stage instrumentation, so that share is currently unattributable —
//! it is the next thing to measure, and clonk-org/clonk-rs#293's criteria list
//! group I/O, definition traversal and script parse/link as the stages it wants
//! there.

use std::time::Duration;

use crate::support::real_scenario::{load_installed_scenario, load_tutorial};

fn report(label: &str, wall: Duration, timings: clonk_engine::ActivationTimings) {
    let total = timings.total;
    eprintln!(
        "--- {label}: the user-visible interval is {wall:?}, of which apply reports {total:?} ---"
    );
    let share = |span: Duration| {
        let micros = total.as_micros().max(1);
        span.as_micros().saturating_mul(100) / micros
    };
    eprintln!("--- {label}: activation stages against the interval they sit in ---");
    eprintln!("total: {total:?}");
    for (name, span) in [
        ("definition_registration", timings.definition_registration),
        ("materials", timings.materials),
        ("landscape", timings.landscape),
        ("object_placement", timings.object_placement),
        ("definition_scripts", timings.definition_scripts),
        ("environment_placement", timings.environment_placement),
        ("post_init_map_callbacks", timings.post_init_map_callbacks),
        ("scenario_script", timings.scenario_script),
    ] {
        eprintln!("    {name}: {span:?} ({}%)", share(span));
    }
    eprintln!(
        "    staged: {:?} ({}%)",
        timings.staged(),
        share(timings.staged())
    );
    eprintln!(
        "    unattributed: {:?} ({}%)  <- no stage covers this",
        timings.unattributed(),
        share(timings.unattributed())
    );
}

/// The scenario clonk-org/clonk-rs#293 names as its reference measurement.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn scenario_activation_profile_over_clonkmars_chaos() {
    let started = std::time::Instant::now();
    let engine = load_installed_scenario("ClonkMars.c4f/03_Chaos.c4s", 0);
    let wall = started.elapsed();
    report("ClonkMars 03_Chaos", wall, engine.activation_timings());
}

/// A small tutorial, for the contrast the issue's benchmark set asks for: the
/// stage that dominates a large scenario need not be the one that dominates a
/// small one, and a single scenario cannot show that.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn scenario_activation_profile_over_a_small_tutorial() {
    let started = std::time::Instant::now();
    let engine = load_tutorial(1, 0);
    let wall = started.elapsed();
    report("Tutorial 1", wall, engine.activation_timings());
}
