# Callback lifecycle performance

## Changes

Callback object lookups and nested-call scratch collections reuse cleared,
bounded thread-local buffers. Object buffers return only after their final
shared owner disappears. Pools retain at most eight entries; object collections
larger than 4,096 slots and scope collections larger than 256 slots are discarded.
Tests verify that recycling releases object and cell ownership and preserves
values held by escaped references.

Player registries, player ordering, local-player membership, fog-of-war lists,
league scores and scenario-section tables are projected only when read. Lazy
projections use the existing synchronous paused-engine lifetime contract and
borrow individual fields. Initialized snapshots share their backing across
clones and detach on mutation. Teams and solid-mask instance ages share their
existing backing until a callback writes them.

Same-object nested calls reuse live local cells instead of seeding and copying
local snapshots. Return, error, reference and continuation paths keep the existing
scope-restoration behavior. Calls that need to fold a separate object's scope
still capture its changes.

A warmed no-op callback fell from **54 allocations to 42**. Adding untouched team
rosters or unused players no longer builds their collections on callback entry.
The nested-call regression eliminates **1,024 array copies** across eight
re-entries with 64 unused array locals. VM name and identity bindings still incur
allocation costs; this change does not remove all nested-call allocations.
Allocation regressions were observed failing before their fixes.

## Measurements

Measured September 18, 2026 (UTC), on Apple M4 Max, 128 GiB, macOS 26.6.2,
Rust 1.98.1, on AC power with existing power settings unchanged. Candidate source:
`bc8b26520126a017caf3ce31cba24e42ab9b374f`.

| Scenario | Before per tick | After per tick | Time reduction |
| --- | ---: | ---: | ---: |
| Tower of Magic | 1.924 ms | 1.893 ms | 1.6% |
| Arso-Morf | 4.572 ms | 4.471 ms | 2.2% |
| Arso-Morf, 1,000 Stippels | 14.625 ms | 14.355 ms | 1.8% |
| ClonkMars: Chaos | 2.020 ms | 2.007 ms | 0.7% |
| Seven Keys | 1.276 ms | 1.267 ms | 0.7% |
| Tutorial 01 | 0.381 ms | 0.377 ms | 1.0% |

The equal-frame mix uses **2.0% less simulation time**,
or **1.5% without the stress case**. This mix is an explicit
benchmark weighting, not a measured distribution of player activity. Every
sample and scenario is retained, including the variation between stress runs.

## Method and validation

Baseline: `0736e943fffea168703a62dc4aa6f6383d4579f1`. Content:
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`. Both executables use identical harness source,
compiler, features and Cargo.lock. Recording is enabled before initialization;
two players join before 100 warmup ticks and 600 measured ticks. Each sample
includes the complete `Engine::tick` and its simulation snapshot. Loading,
warmup and rendering are excluded, so these are simulation timings, not FPS.

Each of six scenarios runs in three fresh-process pairs with alternating build
order. Timing runs sequentially after compilation and validation finish.
Scenario results use the median of three run means; aggregate results compare
the medians of per-repetition sums. No outliers are removed.

Separate runs compare all decoded `EngineState` fields at ticks 101, 200, 300,
400, 500, 600 and 700. All **42 checkpoint pairs match**, including raw fixed
values, synchronized RNG state and ordered lists. Only JSON object-key order is
ignored. This comparison and the primitive C++ parity gate do not establish
full-scenario parity with the C++ oracle.

Passed: **12,201 workspace tests**, 132 engine-tool tests, 639 Python tests,
clippy with warnings denied, formatting, engine snapshots, primitive C++ parity,
compatibility verification and all seven `cargo dev-check` steps. The 24
workspace skips and one engine-tool skip are existing explicit opt-ins. An
initial nextest cleanup warning did not recur in its isolated recheck or the
final complete workspace run.

## Reproduction

[Raw results and harness](../benchmarks/results/callback-lifecycle-performance.json)
contain all **21,600 timing samples**, source and executable hashes, environment
information, checkpoint comparisons and validation results.

1. Check out the baseline and candidate source commits with the pinned content.
   Extract `harness.source` to
   `crates/clonk-engine/examples/callback_lifecycle_profile.rs` in both checkouts.
2. Build with `cargo build --release --locked -p clonk-engine --example
   callback_lifecycle_profile --features presentation-capture,test-graph` and
   preserve the executables as `baseline` and `candidate` in a results directory.
3. Extract `orchestration_source` to `run.py`, adjust `ROOT` and `WORKTREE`, then
   run `python3 run.py states` and `python3 run.py timing`. Finish all compilation
   and checks before timing. Scenario paths, seeds and the stress population
   target are included in the script.
4. `summary_source` records aggregation and completeness checks. JSON object-key
   order is the only normalization applied to state comparisons.
