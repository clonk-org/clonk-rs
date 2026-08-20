//! Manual environment-placement probe for clonk-org/clonk-rs#732.
//!
//! It reports each legacy placement phase and its placement count for a large
//! and a small workload without asserting timing thresholds. Run:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --run-ignored all --no-capture -E 'test(environment_placement_profile::)'
//! ```
//!
//! Current conclusion: ClonkMars cost is concentrated in the Initialize
//! callbacks of eleven environment objects, not placement-loop scaling.
//! In-earth placement remains a separate profiling follow-up.

use crate::support::real_scenario::load_installed_scenario;

fn report(label: &str, timings: clonk_engine::PlacementTimings) {
    let total = timings.total;
    eprintln!("--- {label}: legacy environment placement, {total:?} total ---");
    for (name, phase) in timings.ranked() {
        let share = phase
            .span
            .as_micros()
            .saturating_mul(100)
            .checked_div(total.as_micros().max(1))
            .unwrap_or(0);
        let each = phase
            .span
            .checked_div(phase.placements.max(1))
            .unwrap_or_default();
        eprintln!(
            "    {name}: {:?} ({share}%) over {} placements, {each:?} each",
            phase.span, phase.placements
        );
    }
    eprintln!(
        "    phased: {:?}, unattributed: {:?}",
        timings.phased(),
        timings.unattributed()
    );
}

/// The scenario whose activation clonk-org/clonk-rs#732 is about.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn environment_placement_profile_over_clonkmars_chaos() {
    let engine = load_installed_scenario("ClonkMars.c4f/03_Chaos.c4s", 0);
    report("ClonkMars 03_Chaos", engine.placement_timings());
}

/// A scenario where the same pass costs almost nothing, so the phase that
/// dominates one is not mistaken for the phase that always dominates.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn environment_placement_profile_over_a_small_tutorial() {
    let engine = load_installed_scenario("Tutorial.c4f/Tutorial01.c4s", 0);
    report("Tutorial 1", engine.placement_timings());
}
