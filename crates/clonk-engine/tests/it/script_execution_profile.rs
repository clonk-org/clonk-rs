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
    eprintln!("fallback reason counts overlap when one function has several blockers");
}
