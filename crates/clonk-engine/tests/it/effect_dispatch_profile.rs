//! Manual probe: what effect dispatch materialises over an effect-heavy tick.
//!
//! clonk-org/clonk-rs#291 asks whether the transient state built around effect
//! callbacks is still a bottleneck, and requires the dispatch to be
//! instrumented *before* anything is restructured — specifically that shallow
//! `HostWorldContext` cloning be told apart from what a build materialises.
//!
//! It reports numbers and asserts nothing about them, so it is `#[ignore]`d
//! like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!     --run-ignored all --no-capture -E 'test(effect_dispatch_profile::)'
//! ```
//!
//! # Recorded measurement
//!
//! `ClonkMars.c4f/03_Chaos.c4s`, seed 0, one joined player, 200 steady-state
//! frames, on an aarch64 host at commit `343be4f56`:
//!
//! | counter | total | per frame |
//! |---|---|---|
//! | global timer events | 963 | 4.8 |
//! | world context builds | 3,057 | 15.3 |
//! | context base materializations | 3,057 | 15.3 |
//! | object state snapshots | 1,944 | 9.7 |
//!
//! Ticking cost 406ms, or 2.03ms per frame.
//!
//! A throwaway timer around `host_world_context_base` attributed **48.2ms of
//! that 406ms — 11.9% of the whole simulation tick** — to base
//! materialization alone, about 15.7µs per build. The timer is not part of
//! this probe: an `Instant::now()` pair on a path taken 15 times a frame
//! perturbs what it measures, so the figure is recorded here instead.
//!
//! # What that says
//!
//! Every context build materializes its base; none are avoided. The heavy
//! tables inside it are already cached from clonk-org/clonk-rs#228 and #229
//! and cost only an `Rc` clone, so the 11.9% is the pieces that are *not*
//! cached — transfer-zone states, player order, local players, and the
//! player-view object vectors — rebuilt 15 times a frame.
//!
//! That is a real target rather than the "not a bottleneck" outcome
//! clonk-org/clonk-rs#291 also allows for. It also says where not to start:
//! the shallow `HostWorldContext` clone the issue suspected is not the cost,
//! and a build cannot simply be hoisted to once per tick, because
//! `tick_global_effects` folds each event's outcome back into the engine
//! before the next event runs and the next callback must observe it.

use std::time::Instant;

use crate::support::real_scenario::{join_local_player, load_installed_scenario};

/// Frames to run after the scenario has initialised, long enough for effect
/// timers rather than one-off load work to dominate.
const PROFILED_FRAMES: usize = 200;

#[test]
#[ignore = "manual profiling probe; reports numbers and asserts nothing"]
fn effect_dispatch_profile_over_an_effect_heavy_scenario() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/03_Chaos.c4s", 0);
    let _owner = join_local_player(&mut engine, "Effect dispatch profile");

    engine.reset_effect_dispatch_stats();
    let started = Instant::now();
    for _ in 0..PROFILED_FRAMES {
        let _ = engine.tick_without_snapshot();
    }
    let elapsed = started.elapsed();
    let stats = engine.effect_dispatch_stats();

    let frames = PROFILED_FRAMES as f64;
    eprintln!("--- effect dispatch over {PROFILED_FRAMES} frames of 03_Chaos ---");
    eprintln!(
        "global timer events: {} ({:.1}/frame)",
        stats.global_timer_events,
        stats.global_timer_events as f64 / frames,
    );
    eprintln!(
        "world context builds: {} ({:.1}/frame)",
        stats.world_context_builds,
        stats.world_context_builds as f64 / frames,
    );
    eprintln!(
        "context base materializations: {} ({:.1}/frame)",
        stats.context_base_materializations,
        stats.context_base_materializations as f64 / frames,
    );
    eprintln!(
        "object state snapshots: {} ({:.1}/frame)",
        stats.object_state_snapshots,
        stats.object_state_snapshots as f64 / frames,
    );
    eprintln!(
        "wall clock: {:?} total, {:?}/frame",
        elapsed,
        elapsed / PROFILED_FRAMES as u32,
    );
}
