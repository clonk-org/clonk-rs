//! Manual effect-dispatch instrumentation for clonk-org/clonk-rs#291.
//!
//! It reports transient world-context work over an effect-heavy trace without
//! asserting timing thresholds. Run:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --run-ignored all --no-capture -E 'test(effect_dispatch_profile::)'
//! ```
//!
//! Current conclusion: C4Script execution dominates effect dispatch; context
//! materialization is not the bottleneck. A context cannot be hoisted across
//! callbacks because each callback must observe preceding mutations.

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
