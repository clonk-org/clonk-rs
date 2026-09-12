# Native object-query performance

## Why this target

After the script-bookkeeping changes, native object queries were the largest
specific optimization target in the six-scenario CPU profile: **9.4% of tick
samples**, weighted by unsampled execution time, or **8.8% without the stress
case**. Host-context construction was 5.7%, full snapshots 4.8%, and reference
bookkeeping 4.4%. These inclusive categories can overlap; they must not be added.
Script predicates invoked by queries account for another 5.8% and are excluded
from the native-query number. Broad bytecode and memory-copy totals include many
other operations and do not represent one removable cost.

The selection profile used two independent 600-tick CPU-sampling runs and two
unsampled timing runs per scenario. It reused the preceding optimization's
release driver, whose linked simulation sources were unchanged through baseline
`4a9c3d2e613cba0cad7e9f94ec0a24806a1953bc`; the intervening changes affected app/platform
captions. The A/B measurements below use freshly built baseline and candidate
engine libraries. The 9.4% profile share is an opportunity estimate, not the
claimed speedup.

## Changes

Native searches previously resolved every master-list ID and read every status
before resolving candidates again to evaluate the predicate. The engine now
supplies the frozen execution IDs in forward master order; native predicates
check active-list membership alongside their other fields. This removes the
preliminary lookup/status pass and its temporary status and exclusion tables.

Legacy searches with `FindNext`, reentrant `Find_Func`/script-sort views, and
list-inspection APIs keep the exact filtered master list. Callback order previews,
pending objects, live scope overlays and sector ordering remain authoritative.
The new provider reads only the execution-list field; object lookup retains its
existing generation and identity validation.

Callback-local object, storage-index and removal lookups, plus the master-list
status/exclusion tables, now use the same fast integer-key hashing already used
by the engine object-index cache. Traversal follows explicit master/storage
sequences; bulk materialization sorts by unique storage indices before exposing
its order. Hash-table bucket order never selects a query result.

Results below combine the scan and lookup changes; they do not isolate each
optimization's contribution.

Ensured `ObjectCount2` queries use a scalar active-status predicate instead of
materializing every object's full script state. This preserves the distinction
between `IsEnsured` and `Check`, including the special `Category(0)` behavior.

Regression fixtures observed the original failures before implementation:
64 preliminary status reads become zero, and an unconditional count needs one
caller snapshot instead of 65 object projections. Tests also cover inactive and
deleted candidates, absent inactive continuation markers, and ensured counts.
Existing nested-callback, removal, ordering, closest-tie and pending-write tests
remain active.

## Measured results

Measured September 12, 2026 (UTC), on an Apple M4 Pro with 48 GiB, macOS 26.3.1,
Rust 1.98.1 and AC power, with existing power settings unchanged. Baseline:
`4a9c3d2e613cba0cad7e9f94ec0a24806a1953bc`. Candidate code:
`8f2b204b3c92c99b709a15bafbdad796792ddda9`. Both use content
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009` and the same Cargo.lock.

| Scenario | Before per tick | After per tick | Time reduction |
| --- | ---: | ---: | ---: |
| Tower of Magic | 3.110 ms | 2.825 ms | 9.2% |
| Arso-Morf | 7.023 ms | 6.780 ms | 3.5% |
| Arso-Morf, 1,000 Stippels | 22.674 ms | 22.080 ms | 2.6% |
| ClonkMars: Chaos | 3.392 ms | 3.331 ms | 1.8% |
| Seven Keys | 1.876 ms | 1.831 ms | 2.4% |
| Tutorial 01 | 0.571 ms | 0.563 ms | 1.4% |

The equal-frame six-scenario mix uses **3.3% less simulation time**;
without the 1,000-Stippel stress case, the reduction is **4.0%**.
These are benchmark weights, not a measured distribution of player activity.
They measure complete headless `Engine::tick` calls, including the full
`SimulationSnapshot`; loading, warmup and rendering are outside the timed window.
They do not establish a displayed FPS or GPU improvement.

Each scenario ran in three fresh-process pairs, alternating baseline/candidate,
candidate/baseline, then baseline/candidate. Every process joined two players,
enabled recording before initialization to disable unsynchronized offline
`GetTime`, warmed up for 100 ticks, and measured 600 ticks. All builds and
validation finished before timing, and only one timing process ran at a time.
During development, an early timing batch was discarded in full because
compatibility verification was still running. All reported final runs took place
after validation exited.
All 21,600 final tick samples are retained without outlier removal. State captures ran
separately and their timings are excluded.

Rows show the median of three per-run means. Aggregate reductions compare the
medians of per-repetition sums of scenario means. Raw data includes every run,
loading times and object counts. Loading represents warm-filesystem process
starts, not a cold-cache startup experiment.

## Correctness and validation

All **42 complete decoded EngineState checkpoints match** at ticks 101, 200, 300,
400, 500, 600 and 700 across the six scenarios. This includes raw fixed-point,
object ordering and synchronized RNG state. Only JSON object-key ordering is
ignored; no values are normalized or excluded. Original file hashes and canonical
hashes are retained with the comparison results.

Validation passed: 12,177 workspace tests, 132 engine-tool tests, 639 Python tests,
workspace and FFI clippy with warnings denied, formatting, engine snapshots,
primitive C++ parity, compatibility verification, and the development check.
The 24 workspace skips are existing manual/live-service opt-ins. One workspace
run reported a child-process cleanup diagnostic; the affected UI test passed
an isolated recheck without that diagnostic. The pinned C++
oracle is `7d43b47b7d789b533f32d005e64596e0a07019cd`. Primitive parity and matching
Rust scenario states are complementary checks; neither proves full-scenario C++
parity.

## Reproduction

[`benchmarks/results/native-query-performance.json`](../benchmarks/results/native-query-performance.json)
contains the selection-profile summary, every tick sample, common harness,
orchestration sources, compiler/power details, source/library/executable hashes,
and all checkpoint comparisons.

1. Check out the baseline and candidate commits with their pinned content.
   Extract `harness.source` to `crates/clonk-engine/examples/native_query_profile.rs`
   in each checkout.
2. Build each with `cargo build --release --locked -p clonk-engine --example
   native_query_profile --features presentation-capture,test-graph`. Engine
   features are `default`, `presentation-capture`, `test-graph`; script uses
   `default`, with VM profiling disabled. The measured external driver used
   optimization level 3, thin LTO and one codegen unit. Common non-engine
   libraries were byte-identical between the measured builds.
3. After builds/checks finish, run one executable at a time with
   `LC_PROFILE_MODE=tick LC_PROFILE_WARMUP=100`, following the alternating order
   above and the arguments below.
4. For separate state runs, also set `LC_PROFILE_STATES` to a fresh directory
   per process, then compare every decoded checkpoint without dropping fields.

| Scenario argument | Frames | Seed | Stippel target |
| --- | ---: | ---: | ---: |
| `Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s` | 600 | 0 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | 1000 |
| `ClonkMars.c4f/03_Chaos.c4s` | 600 | 424242 | — |
| `Missions.c4f/SevenKeys.c4s` | 600 | 0 | — |
| `Tutorial.c4f/Tutorial01.c4s` | 600 | 424242 | — |

The stress fixture spawns real ST5B definitions with their normal initializers
and applies identical initial stuck-time grace to both variants.
