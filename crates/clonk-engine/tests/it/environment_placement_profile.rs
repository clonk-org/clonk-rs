//! Manual probe: which legacy environment placement costs the activation.
//!
//! clonk-org/clonk-rs#732 exists because `run_legacy_init_placements` measured
//! **98% of ClonkMars `03_Chaos`'s apply interval** — 12.1 s of 12.3 s — and
//! nothing had ever looked inside it. It covers six `C4Game::InitGame` phases
//! (C4Game.cpp:2493-2503), and an aggregate that large says only that the
//! answer is in there somewhere.
//!
//! This splits the pass by phase, and reports each phase's placement count
//! beside its span so the cost can be read per placement rather than in total.
//!
//! It reports numbers and asserts nothing about them, so it is `#[ignore]`d
//! like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!     --run-ignored all --no-capture -E 'test(environment_placement_profile::)'
//! ```
//!
//! # Recorded measurement
//!
//! Warm process and filesystem cache, seed 0, aarch64 host.
//!
//! ## ClonkMars `03_Chaos` — 11.977 s in the pass
//!
//! | phase | span | share | placements | each |
//! |---|---|---|---|---|
//! | `environment` | 11.933 s | **99%** | **11** | **1.085 s** |
//! | `in_earth` | 42.9 ms | 0% | 85 | 504 µs |
//! | `goals` | 737 µs | 0% | 7 | 105 µs |
//! | `rules` | 278 µs | 0% | 6 | 46 µs |
//! | `nests` | 8.9 µs | 0% | 1 | 8.9 µs |
//! | `vegetation` | 125 ns | 0% | 0 | — |
//! | `animals` | 41 ns | 0% | 0 | — |
//!
//! Unattributed: 3.5 µs. The phases account for the whole pass.
//!
//! **This is not a scaling problem.** The candidates clonk-org/clonk-rs#732
//! listed — placement-loop overhead, a superlinear term in the number already
//! placed, landscape probes per candidate position — all predict cost that
//! grows with *placements*. `environment` makes **eleven** of them and takes
//! **1.08 seconds each**. Vegetation, the phase that actually loops, places
//! nothing here at all.
//!
//! The eleven are `Objects=TIME=10;TEMP=1` in the scenario's `[Environment]`
//! section, so the whole 12-second activation is eleven objects' `Initialize`
//! callbacks. Which of `TIME` and `TEMP` carries it, and what inside the
//! callback costs a second, is the next thing to find — but it is a script
//! question now, not a placement-loop one.
//!
//! `in_earth` is worth a second look on its own terms: 504 µs per placement is
//! slow for something that draws a position and probes the landscape, and it
//! *is* the shape that scales. It is invisible here only because 85 placements
//! is a small number.
//!
//! ## Tutorial 1 — 10.75 µs in the pass
//!
//! One rule object and nothing else. The same pass is six orders of magnitude
//! cheaper, which is why a threshold taken from one scenario would be wrong.

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
