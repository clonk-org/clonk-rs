//! Manual probe: where the C4Script VM still spends string-keyed lookups.
//!
//! clonk-org/clonk-rs#292 asks whether the runtime's owned-`String` identifier
//! tables are still worth interning. The percentages that motivated it predate
//! the compiled executor, so the issue requires a fresh measurement by lookup
//! family before any interning is chosen. This is that measurement over a real
//! shipped scenario rather than a synthetic loop.
//!
//! It reports numbers and asserts nothing about them, so it is `#[ignore]`d
//! like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!     --features lookup-profile --run-ignored all --no-capture \
//!     -E 'test(script_lookup_profile::)'
//! ```
//!
//! Without `--features lookup-profile` the counters are compiled out and every
//! family reports zero, which the probe says out loud rather than presenting
//! an empty table as a result.
//!
//! # Recorded measurement
//!
//! Tutorial 1, seed 0, one joined player, 400 steady-state frames after
//! scenario init, on an aarch64 host at commit `be960c8cb`:
//!
//! | family | lookups | share | bytes hashed |
//! |---|---|---|---|
//! | `script_function` | 91,393 | 45% | 939,534 |
//! | `host_function` | 63,334 | 31% | 641,145 |
//! | `local` | 35,612 | 17% | 232,446 |
//! | `constant` | 3,125 | 1% | 27,933 |
//! | `definition` | 3,016 | 1% | 12,064 |
//! | `global` | 2,935 | 1% | 26,252 |
//! | `effect_callback` | 0 | — | never reached |
//!
//! Totals: 199,415 lookups and 1,879,374 bytes hashed over 400 frames, or
//! roughly 500 lookups and 4.7 KiB hashed per frame.
//!
//! Throwaway sub-counts over the same trace attributed two of the call paths
//! that issue those 154,727 function lookups:
//!
//! - The compiled executor was entered 14,299 times, ran 13,098 of those, and
//!   resolved 14,236 call sites in its per-invocation prelude (4,516 script,
//!   9,720 host). At one script probe and two host probes per site that is
//!   roughly 28,000 lookups, about 18%.
//! - The AST interpreter evaluated 15,117 call expressions, each resolving its
//!   callee by name on every executed call. With its host fallback that is
//!   roughly 30,000 lookups, about another 20%.
//!
//! That leaves about 60% unattributed, spread over the other resolution sites
//! and the host-initiated entry points. Attributing it needs per-site counters
//! rather than more throwaway atomics, which is the next step below.
//!
//! # Go/no-go
//!
//! **Go on the family, not yet on a site.** Script and host function names are
//! 76% of all lookups after clonk-org/clonk-rs#207 and #259, so the family is
//! still material and the issue's premise survives its own staleness warning.
//! But the obvious target is the wrong one: the compiled executor's prelude
//! re-resolves every call site each time a function is entered — which
//! `function_name_lookups_do_not_scale_with_the_work_a_call_does` pins as per
//! invocation rather than per executed call — and it is only ~18% of the cost.
//! Interning it would move a fifth of the lookups and leave the majority
//! untouched.
//!
//! The next step is therefore to extend this instrument with per-call-site
//! attribution and find the remaining ~60% *before* choosing where handles
//! attach. Implementing against the numbers known today would optimise a
//! minority path.
//!
//! Three traps this measurement caught, each of which would have sent the work
//! somewhere it does not belong:
//!
//! - A synthetic loop that stays inside one long-running function reports
//!   `local` three orders of magnitude heavier than the function families and
//!   ranks them last. Real content makes many short calls.
//! - The compiled prelude looks like the hot path from reading the code, and
//!   is not.
//! - A family total does not name a call site. 76% in one family still split
//!   across at least three paths with different fixes.
//!
//! **No-go for the rest.** `constant`, `definition` and `global` are 1% each;
//! interning them would be risk without a return. `local` is 17% but is
//! already slot-indexed on the compiled path — what remains is the AST
//! fallback, which belongs with wider bytecode lowering rather than here.
//! `effect_callback` never resolved a name over this trace and builds no keys
//! at runtime, which is consistent with the per-dispatch allocation removed by
//! clonk-org/clonk-rs#667; a trace over effect-heavy content should confirm
//! that before anyone acts on it.

use clonk_script::lookup_profile::{self, LookupFamily};

use crate::support::real_scenario::{join_local_player, load_tutorial};

/// Frames to run after the scenario has initialised. Long enough for effect
/// timers and per-tick script callbacks to dominate one-off load work.
const PROFILED_FRAMES: usize = 400;

#[test]
#[ignore = "manual profiling probe; needs --features lookup-profile to report anything"]
fn script_lookup_profile_over_a_shipped_scenario() {
    let mut engine = load_tutorial(1, 0);
    let _owner = join_local_player(&mut engine, "Lookup profile");

    // Scenario init loads and links every definition script, which is a
    // one-off cost with a completely different shape from the steady state.
    // Report the two separately or the steady state disappears into it.
    lookup_profile::reset();
    for _ in 0..PROFILED_FRAMES {
        let _ = engine.tick_without_snapshot();
    }
    let steady_state = lookup_profile::snapshot();

    eprintln!("--- script identifier lookups over {PROFILED_FRAMES} frames ---");
    if steady_state.total_lookups() == 0 {
        eprintln!(
            "counters are compiled out; re-run with --features lookup-profile for real numbers"
        );
        return;
    }
    eprintln!("{steady_state}");
    let total = steady_state.total_lookups();
    for (family, counters) in steady_state.ranked() {
        // Integer percent avoids implying more precision than one run has.
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
    for family in LookupFamily::ALL {
        if steady_state.family(family).lookups == 0 {
            eprintln!("{family}: never reached over this trace");
        }
    }
}
