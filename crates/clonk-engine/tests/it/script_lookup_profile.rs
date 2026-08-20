//! Manual identifier-lookup probe for clonk-org/clonk-rs#292.
//!
//! It compares steady-state Tutorial 1 and Hazard workloads with ClonkMars
//! activation, reporting counters without asserting thresholds. Run:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --features lookup-profile --run-ignored all --no-capture \
//!   -E 'test(script_lookup_profile::)'
//! ```
//!
//! Current conclusion: removing duplicate function-table walks was worthwhile,
//! but interning the remaining lookups needs mutation-safe cache invalidation
//! and does not materially affect activation. Runtime effect-callback key
//! construction is also negligible.

use clonk_engine::Engine;
use clonk_script::lookup_profile::{self, LookupFamily};

use crate::support::real_scenario::{join_local_player, load_installed_scenario, load_tutorial};

/// Frames to run after scenario initialization so per-tick callbacks dominate
/// one-off load work.
const PROFILED_FRAMES: usize = 400;

#[test]
#[ignore = "manual profiling probe; needs --features lookup-profile to report anything"]
fn script_lookup_profile_over_a_shipped_scenario() {
    let mut engine = load_tutorial(1, 0);
    let _owner = join_local_player(&mut engine, "Lookup profile");
    report_steady_state(&mut engine, "Tutorial 1");
}

/// Hazard exercises the runtime `Fx<Name>...` callback keys that Tutorial 1
/// rarely reaches.
#[test]
#[ignore = "manual profiling probe; needs --features lookup-profile to report anything"]
fn script_lookup_profile_over_effect_heavy_content() {
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let _owner = join_local_player(&mut engine, "Lookup profile");
    report_steady_state(&mut engine, "Hazard tutorial");
}

/// Measures the activation lookups that steady-state probes reset away.
#[test]
#[ignore = "manual profiling probe; needs --features lookup-profile to report anything"]
fn script_lookup_profile_over_a_scenario_activation() {
    lookup_profile::reset();
    let started = std::time::Instant::now();
    let _engine = load_installed_scenario("ClonkMars.c4f/03_Chaos.c4s", 0);
    let elapsed = started.elapsed();
    let activation = lookup_profile::snapshot();

    eprintln!("--- ClonkMars 03_Chaos: identifier lookups over one activation ---");
    eprintln!("activation wall time: {elapsed:?}");
    if activation.total_lookups() == 0 {
        eprintln!(
            "counters are compiled out; re-run with --features lookup-profile for real numbers"
        );
        return;
    }
    let total = activation.total_lookups();
    for (family, counters) in activation.ranked() {
        let share = counters.lookups.saturating_mul(100) / total.max(1);
        eprintln!(
            "{family}: {share}% of {total} lookups, {} bytes hashed, {} keys built at runtime",
            counters.hashed_bytes, counters.key_allocations,
        );
    }
    eprintln!(
        "totals: {total} lookups, {} bytes hashed, over {elapsed:?}",
        activation.total_hashed_bytes(),
    );
    eprintln!(
        "that is {} lookups per millisecond of activation",
        u128::from(total) / elapsed.as_millis().max(1),
    );
}

fn report_steady_state(engine: &mut Engine, label: &str) {
    lookup_profile::reset();
    for _ in 0..PROFILED_FRAMES {
        let _ = engine.tick_without_snapshot();
    }
    let steady_state = lookup_profile::snapshot();

    eprintln!("--- {label}: script identifier lookups over {PROFILED_FRAMES} frames ---");
    if steady_state.total_lookups() == 0 {
        eprintln!(
            "counters are compiled out; re-run with --features lookup-profile for real numbers"
        );
        return;
    }
    eprintln!("{steady_state}");
    let total = steady_state.total_lookups();
    for (family, counters) in steady_state.ranked() {
        let share = counters.lookups.saturating_mul(100) / total.max(1);
        eprintln!(
            "{family}: {share}% of {total} lookups, {} bytes hashed, {} keys built at runtime",
            counters.hashed_bytes, counters.key_allocations,
        );
    }
    eprintln!(
        "totals: {total} lookups, {} bytes hashed",
        steady_state.total_hashed_bytes(),
    );
    eprintln!("--- by call path ---");
    for (site, counters) in steady_state.ranked_sites() {
        let share = counters.lookups.saturating_mul(100) / total.max(1);
        eprintln!(
            "{site}: {share}% of {total} lookups, {} bytes hashed",
            counters.hashed_bytes,
        );
        for family in LookupFamily::ALL {
            let at = steady_state.family_at(family, site);
            if at.lookups != 0 {
                eprintln!("    {family}: {}", at.lookups);
            }
        }
    }
    for family in LookupFamily::ALL {
        if steady_state.family(family).lookups == 0 {
            eprintln!("{family}: never reached over this trace");
        }
    }
}
