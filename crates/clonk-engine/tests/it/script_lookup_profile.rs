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
//! Acting on the two duplicate walks this profile exposed took the same trace
//! to **163,721 lookups and 1,528,432 bytes hashed — 17.9% fewer lookups and
//! 18.7% fewer hashed bytes**, with no interning, no change to how a function
//! is selected, and `parity verify` and `engine-snapshots verify` unmoved.
//!
//! **The figures above under-count.** `has_host_function` probes the value
//! table and only falls through to the reference table on a miss, so `||`
//! short-circuited past the one recording site and every *successful* host
//! probe went uncounted. With that fixed the same tree measures 192,846
//! lookups, and it immediately showed a third duplicate: the predicate
//! deciding whether a call yields a reference walked the host tables for every
//! name before the cheap test of whether the name is one of the seven builtins
//! a host registration can affect. Testing the name first took the trace to
//! **177,831 lookups and 1,688,613 hashed bytes, 7.8% fewer**. Totals from
//! here on are not comparable with the ones above.
//!
//! A fourth followed from the same corrected numbers: `build_call_args`
//! resolved the host callee *inside* its per-argument loop, so a
//! three-argument call walked the host reference table three times for an
//! answer whose lookup does not vary with the argument index. Hoisting it out
//! took the trace to **166,349 lookups and 1,553,339 hashed bytes**, 6.5%
//! fewer again.
//!
//! Split by the call path that issued each probe, over the same trace:
//!
//! | call path | lookups | share | composition |
//! |---|---|---|---|
//! | `unattributed` | 88,008 | 44% | 35,602 local, 33,864 host, 9,702 script |
//! | `reference_query` | 51,266 | 25% | 50,992 script |
//! | `compiled_prelude` | 39,492 | 19% | 15,630 script, 23,645 host |
//! | `ast_call` | 20,265 | 10% | 15,040 script, 5,225 host |
//! | `generic_dispatch` | 394 | 0% | the host entry point's own dispatch |
//! | `object_call` | 0 | — | `->` is not hot in this scenario |
//!
//! # Go/no-go
//!
//! **Go on the family — and the first fix is not interning.** Script and host
//! function names are 76% of all lookups after clonk-org/clonk-rs#207 and
//! #259, so the family is still material and the issue's premise survives its
//! own staleness warning. Where that cost sits is the surprise:
//!
//! - `reference_query` was **25% of every identifier lookup in the VM and 56%
//!   of all script-function resolutions** (50,992 of 91,393). It is waste, not
//!   work: `direct_value_call_has_materialized_result` asked
//!   `call_expression_returns_reference` whether the result is a reference and
//!   then resolved the same callee again to decide whether it is materialized.
//!   **Fixed:** one resolution now answers both, which took the trace from
//!   199,415 lookups and 1,879,374 hashed bytes to 174,835 and 1,617,477 —
//!   12.3% fewer lookups and 13.9% fewer hashed bytes, with no interning and
//!   no change to how a function is selected. One probe per call remains, on
//!   `set_no_ref_keeps_reference`'s separate evaluator entry point.
//! - `compiled_prelude` was 19%. It re-resolves every call site in a function
//!   body on each entry, which `function_name_lookups_do_not_scale_with_the
//!   _work_a_call_does` pins as per invocation rather than per executed call.
//!   **Partly fixed:** it also walked the host tables twice per host call
//!   site, once for the reference guard and once for the value target.
//!   Registration keeps a name out of the table it is not in, so one walk
//!   answers both; that took the trace to 163,721 lookups. Attaching stable
//!   handles to the surviving per-entry resolution is the next step, and the
//!   one that actually needs interning.
//! - `ast_call` is 10%, one resolution per executed call, and shrinks as
//!   compiled coverage widens rather than through interning.
//! - `generic_dispatch` — the host entry point that looks like the obvious
//!   answer — is 0.2%.
//!
//! 44% remains unattributed and is mostly `local` (all 35,602 probes) and
//! host-function probes from resolution helpers with no span of their own.
//! Locals are already slot-indexed on the compiled path, so what is left there
//! is the AST fallback's scope-chain walk, which the issue scopes to wider
//! bytecode lowering.
//!
//! **No-go for the rest.** `constant`, `definition` and `global` are 1% each;
//! interning them would be risk without a return.
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
    eprintln!("--- by call path ---");
    for (site, counters) in steady_state.ranked_sites() {
        let share = counters.lookups.saturating_mul(100) / total.max(1);
        eprintln!(
            "{site}: {share}% of {total} lookups, {} bytes hashed",
            counters.hashed_bytes,
        );
        for family in LookupFamily::ALL {
            let at = steady_state.family_at(family, site);
            if at.lookups != 0 {
                eprintln!("    {family}: {}", at.lookups);
            }
        }
    }
    for family in LookupFamily::ALL {
        if steady_state.family(family).lookups == 0 {
            eprintln!("{family}: never reached over this trace");
        }
    }
}
