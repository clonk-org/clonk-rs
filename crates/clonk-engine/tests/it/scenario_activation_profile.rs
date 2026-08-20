//! Manual activation-stage probe for clonk-org/clonk-rs#293.
//!
//! It reports scenario load and apply stages for a large and a small workload
//! without asserting host-dependent timing thresholds. Run:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --run-ignored all --no-capture -E 'test(scenario_activation_profile::)'
//! ```
//!
//! Current conclusion: ClonkMars apply time is dominated by environment
//! placement, while Tutorial 1 is dominated by pre-apply scenario loading.
//! The next useful split belongs inside scenario loading, not materials or
//! system-script setup.

use std::time::Duration;

use crate::support::real_scenario::{prepare_installed_scenario, ScenarioLoadTimings};

/// The load half, which `ActivationTimings` cannot see: it records spans
/// inside `Scenario::apply` and nothing before it.
fn report_load(label: &str, load: ScenarioLoadTimings) {
    let total = load.total();
    let share = |span: Duration| span.as_micros().saturating_mul(100) / total.as_micros().max(1);
    eprintln!("--- {label}: load, before apply begins — {total:?} ---");
    for (name, span) in [
        ("scenario (group I/O + definitions)", load.scenario),
        ("materials (Material.c4g)", load.materials),
        ("system_scripts (planet/System.c4g)", load.system_scripts),
    ] {
        eprintln!("    {name}: {span:?} ({}%)", share(span));
    }
}

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
    let prepared = prepare_installed_scenario("ClonkMars.c4f/03_Chaos.c4s", 0);
    let load = prepared.load_timings();
    let engine = prepared.instantiate();
    let wall = started.elapsed();
    report_load("ClonkMars 03_Chaos", load);
    report("ClonkMars 03_Chaos", wall, engine.activation_timings());
}

/// A small tutorial, for the contrast the issue's benchmark set asks for: the
/// stage that dominates a large scenario need not be the one that dominates a
/// small one, and a single scenario cannot show that.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn scenario_activation_profile_over_a_small_tutorial() {
    let started = std::time::Instant::now();
    let prepared = prepare_installed_scenario("Tutorial.c4f/Tutorial01.c4s", 0);
    let load = prepared.load_timings();
    let engine = prepared.instantiate();
    let wall = started.elapsed();
    report_load("Tutorial 1", load);
    report("Tutorial 1", wall, engine.activation_timings());
}
