//! Manual compiled-vs-AST C4Script probe over shipped content.
//!
//! Run:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!   --features execution-profile --run-ignored all --no-capture \
//!   -E 'test(script_execution_profile::)'
//! ```

use clonk_script::execution_profile;

use crate::support::real_scenario::{join_local_player, load_installed_scenario};

const PROFILED_FRAMES: usize = 400;

/// Decision rule and interpretation: docs/script-execution-materiality.md.
#[test]
#[ignore = "manual release timing probe; requires execution-profile"]
#[cfg(feature = "execution-profile")]
fn ast_execution_materiality_on_shipped_content() {
    use std::time::Instant;

    for scenario in [
        "Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s",
        "Missions.c4f/SevenKeys.c4s",
    ] {
        for run in 0..5 {
            let mut expected_profile = None;
            // Alternate pair order to reduce warm-cache and scheduling bias.
            for enabled in [run % 2 == 0, run % 2 != 0] {
                let mut engine = load_installed_scenario(scenario, 0);
                join_local_player(&mut engine, "Execution timing");
                for _ in 0..100 {
                    engine.tick_without_snapshot().expect("warmup tick");
                }
                execution_profile::reset();
                execution_profile::set_timing_enabled(enabled);
                let started = Instant::now();
                for _ in 0..PROFILED_FRAMES {
                    engine.tick_without_snapshot().expect("measured tick");
                }
                let elapsed = started.elapsed();
                let timing = execution_profile::timing_snapshot();
                execution_profile::set_timing_enabled(false);
                let profile = execution_profile::snapshot();
                if let Some(expected) = expected_profile {
                    assert_eq!(profile, expected, "timing must preserve invocation counts");
                } else {
                    expected_profile = Some(profile);
                }
                eprintln!(
                    "scenario={scenario} run={run} timing={enabled} frames={PROFILED_FRAMES} tick_ns={} ast_ns={} compiled_ns={} compiled={} ast={} guards={}",
                    elapsed.as_nanos(), timing.ast_ns, timing.compiled_ns,
                    profile.compiled, profile.ast_without_plan, profile.ast_after_runtime_guard,
                );
                if enabled {
                    eprintln!("{profile}");
                    for (reason, ns) in timing.ranked_ast_sole_blocker_ns() {
                        eprintln!(
                            "sole {reason}: {ns} ns ({}% of ast_ns)",
                            ns.saturating_mul(100) / timing.ast_ns.max(1)
                        );
                    }
                    assert!(timing.ast_ns > 0, "scenario must exercise AST execution");
                }
            }
        }
    }
}

#[test]
#[ignore = "manual profiling probe; needs --features execution-profile for real counters"]
fn compiled_vs_ast_over_effect_heavy_shipped_content() {
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let _owner = join_local_player(&mut engine, "Execution profile");
    execution_profile::reset();

    for _ in 0..PROFILED_FRAMES {
        let _ = engine.tick_without_snapshot();
    }

    let profile = execution_profile::snapshot();
    eprintln!("--- Hazard tutorial: C4Script execution over {PROFILED_FRAMES} frames ---");
    if profile.total_invocations() == 0 {
        eprintln!(
            "counters are compiled out; re-run with --features execution-profile for real numbers"
        );
        return;
    }
    eprintln!("{profile}");
    let ast = profile
        .ast_without_plan
        .saturating_add(profile.ast_after_runtime_guard);
    eprintln!(
        "AST share: {}% ({ast}/{})",
        ast.saturating_mul(100) / profile.total_invocations().max(1),
        profile.total_invocations(),
    );
    eprintln!(
        "fallback reason counts overlap when one function has several blockers; the `sole` lines count invocations that one family alone kept on the AST VM"
    );
}
