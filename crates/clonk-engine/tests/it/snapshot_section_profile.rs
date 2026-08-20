//! Manual snapshot-section probe for clonk-org/clonk-rs#294.
//!
//! It reports the worst of repeated projections for a large and a small
//! workload without asserting host-dependent timing thresholds. Run:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --run-ignored all --no-capture -E 'test(snapshot_section_profile::)'
//! ```
//!
//! Current conclusion: no section approaches the 2 ms materiality threshold.
//! Landscape cost scales with map width because per-column vectors are not
//! shared; dirtiness does not affect clone cost, so no broad optimization is
//! justified by the current workloads.

use crate::support::real_scenario::load_installed_scenario;

/// Snapshots taken per workload. The projection is pure with respect to the
/// engine, so repeats measure the same work — enough of them to see past a
/// cold first call without turning the probe into a benchmark.
const SNAPSHOTS: usize = 20;

fn report(label: &str, engine: &clonk_engine::Engine) {
    // Warm once: the first projection pays for allocations every later one
    // reuses, and reporting that as the section cost would overstate it.
    let _ = engine.snapshot();
    let mut worst = clonk_engine::SnapshotTimings::default();
    for _ in 0..SNAPSHOTS {
        let _ = engine.snapshot();
        let timings = engine.snapshot_timings();
        if timings.total > worst.total {
            worst = timings;
        }
    }

    let total = worst.total;
    eprintln!("--- {label}: SimulationSnapshot projection, worst of {SNAPSHOTS} — {total:?} ---");
    for (name, span) in worst.ranked() {
        let share = span
            .as_nanos()
            .saturating_mul(100)
            .checked_div(total.as_nanos().max(1))
            .unwrap_or(0);
        eprintln!("    {name}: {span:?} ({share}%)");
    }
    eprintln!(
        "    sectioned: {:?}, unattributed: {:?}",
        worst.sectioned(),
        worst.unattributed()
    );
}

/// A real scenario with a populated world.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn snapshot_section_profile_over_clonkmars_chaos() {
    let engine = load_installed_scenario("ClonkMars.c4f/03_Chaos.c4s", 0);
    report("ClonkMars 03_Chaos", &engine);
}

/// A small scenario, so a section that dominates a populated world is not
/// mistaken for one that always dominates.
#[test]
#[ignore = "manual profiling probe; reports timings and asserts nothing"]
fn snapshot_section_profile_over_a_small_tutorial() {
    let engine = load_installed_scenario("Tutorial.c4f/Tutorial01.c4s", 0);
    report("Tutorial 1", &engine);
}
