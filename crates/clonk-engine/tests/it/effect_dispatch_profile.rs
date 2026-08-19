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
//! # What the counts say
//!
//! Every context build materializes its base; none are avoided. A build is
//! therefore roughly three times as frequent as a global timer event, because
//! object dispatch builds one too.
//!
//! The caches inside the base work. A throwaway counter on
//! `solid_mask_metadata_table` recorded **one miss in 3,057 calls**, so the
//! definition and solid-mask tables really do cost only an `Rc` clone per
//! build, and "the cached tables are being rebuilt" is not the explanation
//! for anything here.
//!
//! # What no measurement here supports
//!
//! An earlier revision of this comment claimed 11.9% of the tick for base
//! materialization and attributed it to the uncached transfer-zone, player
//! order and player-view vectors. **Both claims are withdrawn.** Timing at
//! that granularity with `Instant::now()` did not survive its own check: two
//! timers wrapped around the same `solid_mask_metadata_table` call disagreed
//! by more than two orders of magnitude (0.045ms from inside the function
//! against 21.35ms from immediately outside it), and that discrepancy is
//! unexplained. When the instrument contradicts itself the reading is not
//! evidence, whichever number would have been more convenient.
//!
//! Per-piece cost inside the base is therefore still unknown. A sampling
//! profiler is the right tool for it; hand-placed timers on a path taken 15
//! times a frame are not.
//!
//! # What still holds regardless
//!
//! Two structural facts constrain any fix and come from reading the code
//! rather than from timing:
//!
//! - A build cannot be hoisted to once per tick. `tick_global_effects` folds
//!   each event's outcome back into the engine before the next event runs,
//!   and the next callback must observe those mutations.
//! - `with_solid_mask_instance_sequences` deep-clones its `HashMap` on every
//!   build *because* a callback may mutate the resulting `RefCell` and that
//!   mutation must not leak back. That is eager copy-on-write, and making it
//!   lazy is a real change rather than a caching one.

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
