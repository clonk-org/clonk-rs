# Script callback context performance

## Selection

Fresh profiles after the native-query optimization identify callback context
setup and teardown as the largest bounded remaining target: **13.2%** of
simulation tick time in the six-scenario mix, or **10.4% without the stress case**.
This group includes world construction, effect-context construction, TLS
save/restore, outcome extraction, and native work directly in their wrapper;
it excludes the script body executed by that wrapper. World construction alone
accounts for 5.7%. Native queries account for 6.1%, reference bookkeeping 4.9%,
and full snapshots 4.8%. These categories overlap and must not be added.
The profile share estimates the opportunity; it is not the measured speedup.

Two independent sampled windows and two unsampled timing windows per scenario
followed 100 warmup ticks and covered 600 full ticks each. Sampling retained only
`Engine::tick` stacks, excluding loading and the readiness handshake. Aggregate
shares weight per-scenario sample fractions by unsampled mean tick time.
The 19.7% spent copying memory is spread across unrelated operations and is not
one removable cost. The workload mix is an explicit benchmark weighting, not a
measured distribution of player activity.

## Implementation

Engine-backed callback worlds now initialize their resource handles directly.
Previously, the fixture-compatible constructor allocated default tables and
request queues, then a long builder chain immediately replaced them with engine
resources. The direct constructor preserves lazy object/player/landscape reads,
shares the same engine-owned resources, and creates fresh mutable callback
previews. It preserves the merged player-info ID namespace, score filtering,
section-name normalization, list order, and deferred raster previews.

Effect contexts now retain heap ownership through construction, thread-local
save/restore and outcome extraction. Nested callbacks move a pointer instead of
copying the entire large context. The existing guard restores the previous
context on both normal completion and unwinding. No new unsafe code is needed.

The allocation regression was observed failing before implementation: a warmed
object callback that only returns `42` allocated **103 times**. It now allocates
**54 times**, below a 64-allocation budget. Existing callback,
removal, ordering, state-visibility and panic-cleanup tests continue to run.
The results below combine direct construction and context ownership changes.

## Measurements

Measured September 12, 2026 (UTC), on Apple M4 Pro, 48 GiB, macOS 26.3.1,
Rust 1.98.1, AC power, with existing power settings unchanged. Baseline main:
`21235af6181820c92c6649e049e6914fb6bbb963`; candidate source:
`3a8a6896db220415f55ca3d7f606e69efd6d6869`. Content:
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`. Cargo.lock is unchanged.

| Scenario | Before per tick | After per tick | Time reduction |
| --- | ---: | ---: | ---: |
| Tower of Magic | 2.855 ms | 2.712 ms | 5.0% |
| Arso-Morf | 6.832 ms | 6.460 ms | 5.5% |
| Arso-Morf, 1,000 Stippels | 22.088 ms | 20.651 ms | 6.5% |
| ClonkMars: Chaos | 3.336 ms | 3.307 ms | 0.9% |
| Seven Keys | 1.827 ms | 1.787 ms | 2.2% |
| Tutorial 01 | 0.560 ms | 0.553 ms | 1.4% |

The equal-frame mix uses **6.2% less simulation time**, or
**3.8% without the stress case**. These are full headless
`Engine::tick` measurements, including `SimulationSnapshot` generation.
Loading, warmup and rendering are outside the measured window; this is not a
displayed-FPS or GPU result.

Each case ran in three fresh-process pairs with alternating execution order.
Every run enabled recording before initialization, joined two players, warmed
for 100 ticks and measured 600 ticks. All **21,600 tick samples** are retained;
none were removed as outliers. Builds and validation finished before timing;
only one timing process ran at a time. Separate state-capture timings are excluded.
Rows report medians of three per-run means. Aggregate reductions compare medians
of per-repetition sums of scenario means. Raw loading times are retained but do
not constitute a cold-cache startup experiment.

The baseline executable is the retained release driver from
`8f2b204b3c92c99b709a15bafbdad796792ddda9`. A two-endpoint diff confirmed
its simulation sources, manifests and Cargo.lock are identical to baseline main;
intervening changes only affect documentation and CI. Both builds use the same
compiler, engine feature set and harness. Absolute build directories differ;
complete library fingerprints and executable hashes are included in the raw data.

## Correctness and validation

All **42 complete decoded EngineState checkpoint pairs match**, at ticks 101,
200, 300, 400, 500, 600 and 700 for every scenario. Raw fixed-point values,
object/list order and synchronized RNG state are compared. Only JSON object-key
ordering is ignored; no state values are normalized or excluded. File and
canonical hashes are retained.

Passed: **12,178 workspace tests**, 132 engine-tool tests,
639 Python tests, workspace and FFI clippy with warnings denied, formatting,
engine snapshots, primitive C++ parity, compatibility verification and
`cargo dev-check`. The 24 workspace skips are existing explicit opt-ins.
Two workspace tests emitted nextest cleanup diagnostics despite passing their
assertions. Both passed isolated rechecks without cleanup diagnostics; their
names and recheck result are retained in the raw data.
The pinned C++ oracle is `7d43b47b7d789b533f32d005e64596e0a07019cd`.
Primitive parity and matching Rust scenario states are complementary checks;
neither establishes full-scenario C++ parity.

## Reproduction

[Raw data and harness](../benchmarks/results/callback-context-performance.json)
include every sample, selection-profile methodology, orchestration sources,
compiler/power details, library/source/executable hashes and state comparisons.

1. Check out baseline and candidate with their pinned content. Extract
   `harness.source` to `crates/clonk-engine/examples/callback_context_profile.rs`.
2. Build each with `cargo build --release --locked -p clonk-engine --example
   callback_context_profile --features presentation-capture,test-graph`.
   The measured external harness uses optimization level 3, thin LTO and one
   codegen unit; script features are `default`, without VM profiling.
3. After all builds/checks finish, run one process at a time using
   `LC_PROFILE_MODE=tick LC_PROFILE_WARMUP=100` and the arguments below, alternating
   baseline/candidate, candidate/baseline, baseline/candidate.
4. For separate correctness runs, set `LC_PROFILE_STATES` to a fresh directory
   per process and compare every decoded checkpoint without dropping fields.

| Scenario argument | Frames | Seed | Stippel target |
| --- | ---: | ---: | ---: |
| `Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s` | 600 | 0 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | 1000 |
| `ClonkMars.c4f/03_Chaos.c4s` | 600 | 424242 | — |
| `Missions.c4f/SevenKeys.c4s` | 600 | 0 | — |
| `Tutorial.c4f/Tutorial01.c4s` | 600 | 424242 | — |

The stress fixture spawns real ST5B definitions through normal initializers with
identical initial stuck-time grace in both variants.
