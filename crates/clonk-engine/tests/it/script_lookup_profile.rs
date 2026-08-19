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
//!   answers both; that took the trace to 163,721 lookups.
//!
//!   The resolution that survives — once per call site per invocation — is
//!   the only place left where interning would help, and it is **declined**.
//!   See "Why the handles were not built" below.
//! - `ast_call` is 10%, one resolution per executed call, and shrinks as
//!   compiled coverage widens rather than through interning.
//! - `generic_dispatch` — the host entry point that looks like the obvious
//!   answer — is 0.2%. **Scenario-specific: see the Hazard trace below, where
//!   it is 20%.**
//!
//! 44% remains unattributed and is mostly `local` (all 35,602 probes) and
//! host-function probes from resolution helpers with no span of their own.
//! Locals are already slot-indexed on the compiled path, so what is left there
//! is the AST fallback's scope-chain walk, which the issue scopes to wider
//! bytecode lowering.
//!
//! # Why the handles were not built
//!
//! Four duplicate walks came out of this profile and none of them needed
//! interning; together they removed 13.7% of all identifier lookups. What
//! remains that interning could reach is the compiled prelude's one
//! resolution per call site per invocation — about 14,000 lookups, or 8% of
//! the current total.
//!
//! Caching that resolution needs a way to know the tables have not changed
//! since it was taken, and there is no sound cheap one:
//!
//! - Pointer identity does not work. The engine mutates its tables through
//!   `Arc::make_mut`, which mutates **in place** when the `Arc` is unshared,
//!   so a table can change content at the same address.
//! - A revision counter does work, but it has to be bumped at roughly thirty
//!   mutation sites spread across `engine.rs` — every `Arc::make_mut` on the
//!   host, reference, parameter-type and constant tables, plus the direct
//!   `self.functions` and `self.global_functions` mutations.
//!
//! A single missed bump leaves a stale target that only misfires when a
//! reload happens to change that one name, so it produces a wrong function
//! call rather than a test failure — a desync, invisible to the suite in the
//! general case. That is the failure class this port is least able to absorb,
//! and 8% of identifier lookups (themselves a fraction of tick time) does not
//! buy it. Revisit if a revision counter ever exists for another reason, or
//! if a trace shows the prelude carrying far more than it does here.
//!
//! **No-go for the rest.** `constant`, `definition` and `global` are 1% each;
//! interning them would be risk without a return.
//! `effect_callback` never resolved a name over this trace and builds no keys
//! at runtime — but Tutorial 1 runs almost no effects, so that says more about
//! the scenario than about the family. The Hazard trace below is the
//! confirmation this asked for, and it corrects the second half.
//!
//! # Effect-heavy trace: Hazard tutorial
//!
//! `Hazard.c4f/Tutorial.c4s`, seed 0, one joined player, same 400 steady-state
//! frames, same host, at commit `8a6408b33`:
//!
//! | family | lookups | share | bytes hashed | keys built |
//! |---|---|---|---|---|
//! | `script_function` | 138,491 | 37% | 1,095,837 | 0 |
//! | `host_function` | 137,210 | 37% | 857,914 | 0 |
//! | `local` | 49,886 | 13% | 142,717 | 0 |
//! | `global` | 19,588 | 5% | 136,294 | 0 |
//! | `constant` | 12,272 | 3% | 113,658 | 0 |
//! | `definition` | 11,705 | 3% | 46,820 | 0 |
//! | `effect_callback` | 1,530 | 0.4% | 25,647 | **400** |
//!
//! Totals: 370,682 lookups and 2,418,887 bytes hashed, or roughly 927 lookups
//! per frame against Tutorial 1's 416 — the same shape at 2.2x the volume.
//!
//! | call path | lookups | share | composition |
//! |---|---|---|---|
//! | `reference_query` | 107,946 | 29% | 61,125 script, 46,821 host |
//! | `unattributed` | 93,352 | 25% | 29,467 host, 24,473 local |
//! | `generic_dispatch` | 75,027 | 20% | 25,413 local, 22,795 host |
//! | `ast_call` | 52,960 | 14% | 31,324 script, 21,636 host |
//! | `compiled_prelude` | 40,397 | 10% | 23,791 script, 16,491 host |
//! | `object_call` | 0 | — | `->` is not hot here either |
//!
//! **Two corrections to the Tutorial 1 reading, and no change to the
//! decision.**
//!
//! - `effect_callback` **does** build keys at runtime: exactly 400 over 400
//!   frames, and exactly 200 over 200 — one per `EffectCall`, from the
//!   `format!("Fx{effect_name}{call_fn}")` in `compat::effects`. What
//!   clonk-org/clonk-rs#667 removed was the allocation per effect *dispatch*,
//!   and that property holds: the 1,530 timer-path lookups here build nothing.
//!   The host function that takes a callback name from script still formats
//!   one, because the name genuinely is not known until the call. At 0.4% of
//!   lookups and one allocation per frame it stays a **no-go**, but now on
//!   evidence rather than on a scenario that never ran an effect.
//! - `generic_dispatch` is **20% here against 0.2% in Tutorial 1**. It is a
//!   span, not a site: everything a host entry point reaches is attributed to
//!   it unless a nested span claims it, so the jump measures how much Hazard
//!   drives script *from the engine* rather than any waste of its own. Read
//!   the 0.2% above as a property of Tutorial 1, not of the dispatch path.
//!
//! `reference_query` is 29% here against 25% there — the largest attributed
//! path in both, and after clonk-org/clonk-rs#693 it is one probe per call
//! rather than two. It remains the place to look if the declined handle cache
//! is ever revisited.

use clonk_engine::Engine;
use clonk_script::lookup_profile::{self, LookupFamily};

use crate::support::real_scenario::{join_local_player, load_installed_scenario, load_tutorial};

/// Frames to run after the scenario has initialised. Long enough for effect
/// timers and per-tick script callbacks to dominate one-off load work.
const PROFILED_FRAMES: usize = 400;

#[test]
#[ignore = "manual profiling probe; needs --features lookup-profile to report anything"]
fn script_lookup_profile_over_a_shipped_scenario() {
    let mut engine = load_tutorial(1, 0);
    let _owner = join_local_player(&mut engine, "Lookup profile");
    report_steady_state(&mut engine, "Tutorial 1");
}

/// The same probe over content that actually runs effects.
///
/// clonk-org/clonk-rs#292's `effect_callback` no-go rests on a trace where the
/// family never resolved a name at all, which is weak evidence: Tutorial 1
/// runs almost no effects, so "never reached" there says more about the
/// scenario than about the family. Hazard's tutorial arms crosshairs, weapon
/// timers and death relaunches, so it exercises the `Fx<Name>...` callback
/// names the issue singles out as built at runtime.
#[test]
#[ignore = "manual profiling probe; needs --features lookup-profile to report anything"]
fn script_lookup_profile_over_effect_heavy_content() {
    let mut engine = load_installed_scenario("Hazard.c4f/Tutorial.c4s", 0);
    let _owner = join_local_player(&mut engine, "Lookup profile");
    report_steady_state(&mut engine, "Hazard tutorial");
}

fn report_steady_state(engine: &mut Engine, label: &str) {
    // Scenario init loads and links every definition script, which is a
    // one-off cost with a completely different shape from the steady state.
    // Report the two separately or the steady state disappears into it.
    lookup_profile::reset();
    for _ in 0..PROFILED_FRAMES {
        let _ = engine.tick_without_snapshot();
    }
    let steady_state = lookup_profile::snapshot();

    eprintln!("--- {label}: script identifier lookups over {PROFILED_FRAMES} frames ---");
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
