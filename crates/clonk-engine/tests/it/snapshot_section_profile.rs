//! Manual probe: which section of `Engine::snapshot` costs what.
//!
//! clonk-org/clonk-rs#294 asks for a per-section split of the
//! `SimulationSnapshot` projection, read against a materiality threshold
//! recorded in advance. The threshold lives with the profiler that measures
//! the total (`examples/scenario_profile.rs`): **material if it exceeds either
//! 10% of the combined frame or 2 ms absolute.** Neither of the two workloads
//! measured there crosses it, and this says *why* — a total alone cannot tell
//! one expensive section from a projection that is simply proportional to the
//! world.
//!
//! It reports numbers and asserts nothing about them, so it is `#[ignore]`d
//! like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!     --run-ignored all --no-capture -E 'test(snapshot_section_profile::)'
//! ```
//!
//! # Recorded measurement
//!
//! Worst of 20 projections after a warm-up, aarch64.
//!
//! | section | `03_Chaos` | Tutorial 1 |
//! |---|---|---|
//! | `landscape` | **39.0 µs (38%)** | 12.4 µs (26%) |
//! | `objects` | 36.8 µs (36%) | 14.5 µs (30%) |
//! | `effects_globals` | 14.1 µs (13%) | 0.4 µs (0%) |
//! | `definitions` | 9.8 µs (9%) | 6.9 µs (14%) |
//! | `object_sort` | 0.1 µs (0%) | 12.4 µs (26%) |
//! | `players` / `particles` / `environment` / `debug` | ~0 | ~0 |
//! | **total** | **100.7 µs** | **47.1 µs** |
//!
//! Unattributed is 292 ns in both: the sections account for the projection.
//!
//! **No section is anywhere near the threshold**, and the totals are
//! themselves an order of magnitude under the 2 ms absolute limit. There is no
//! single expensive section to attack — the two that lead are `landscape` and
//! `objects`, and both are the projection being proportional to the world
//! rather than doing anything avoidable.
//!
//! Two things worth carrying forward rather than concluding from:
//!
//! - **`landscape` leads on the populated scenario at 38%, and it scales with
//!   map width rather than with dirtiness.** See below; this is the section
//!   clonk-org/clonk-rs#294 singles out, and the answer is not the one the
//!   issue expects.
//! - **`object_sort` is 26% on Tutorial 1 and 0% on `03_Chaos`**, on the
//!   scenario with *fewer* objects. Read that as an artefact, not a finding:
//!   "worst of 20" selects the run with the largest total and reports that
//!   run's sections, so one noisy sample carries its whole row. A section this
//!   small needs a distribution, not a maximum, before anything is claimed
//!   about it.
//!
//! # What the `landscape` section actually costs
//!
//! clonk-org/clonk-rs#294 reasons that `PixelGrid.bytes` and `surface32_pixels`
//! are `Arc`-backed, "so unchanged-landscape sharing is partly implemented",
//! and lists a **large dirty landscape** as the fixture that would stress it.
//! Both halves of that are right about the pixels and wrong about the cost.
//!
//! `Landscape` also holds `surface: Vec<i32>`, `liquids: Vec<LiquidColumn>`,
//! `solid_materials: Vec<Option<MaterialId>>` and a `tunnels` map. **None are
//! shared**, so `landscape.clone()` deep-copies one entry per column every
//! time, whatever the pixels do. The measurements say exactly that:
//!
//! | scenario | `MapWidth` | landscape px | `landscape` span |
//! |---|---|---|---|
//! | Tutorial 1 | 64 | 512 | 12.4 µs |
//! | `03_Chaos` | 197 | 1,576 | 39.0 µs |
//!
//! **3.08x the width, 3.15x the cost** — proportional to a column count, which
//! is what an unshared per-column `Vec` costs and is not what an `Arc` bump
//! costs.
//!
//! Two consequences:
//!
//! - **Dirtiness is irrelevant here.** Copy-on-write diverges on *write*;
//!   `clone` never consults it. A large *dirty* landscape projects for the
//!   same cost as a large clean one, so that fixture would measure width, not
//!   sharing. Worth building for the other sections, not for this one.
//! - **This section cannot reach the absolute threshold on a real map.** At
//!   ~24.7 ns per column it needs roughly 81,000 columns to reach 2 ms, which
//!   is a `MapWidth` around 10,000 — two orders of magnitude beyond anything
//!   shipped. If the section is ever worth attacking it will be for its share,
//!   not its absolute cost, and the fix is to share the per-column vectors the
//!   way the pixels already are.

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
