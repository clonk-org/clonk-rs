# Object view performance

## Changes

Unchanged construction, mass, layer and base overlays retain the existing shared
`ObjectState` instead of invoking a full copy before checking whether a field
changed.

Coordinate, distance, fixed-point velocity, owner, controller, damage, direction,
action-time and phase getters read callback scopes or shared object views
without copying the complete host object. Active and foreign reads retain their
existing visibility rules for pending updates, missing objects and nested calls.
The coordinate regression drops from four object copies to zero; the broader
scalar-field regression drops from eight to zero. Velocity tests expose raw
fixed-point bits at precision 65,536, including negative values.

Foreign object views defer their complete script-state snapshot until first use.
Clones share that first snapshot and detach on writes. Controller-only reads do
not force a full snapshot. Active and newly spawned objects keep immediate owned
snapshots. Deferred views use the existing synchronous paused-engine lifetime
contract and cannot outlive their frozen source object. Every full-state mutation
materializes its backing before applying the existing update.

Six regression tests were observed failing before their fixes. The full engine
unit suite passed after every implementation cycle.

## Measurements

Measured 2026-09-19 (UTC), on Apple M4 Max, 128 GiB, macOS 26.6.2,
Rust 1.98.1, on AC power with existing power settings unchanged. Candidate source:
`be443539311311eb0fea7f820120958bf44efca3`.

| Scenario | Before per tick | After per tick | Time reduction |
| --- | ---: | ---: | ---: |
| Tower of Magic | 1.799 ms | 1.771 ms | 1.6% |
| Arso-Morf | 4.068 ms | 4.003 ms | 1.6% |
| Arso-Morf, 1,000 Stippels | 13.332 ms | 13.158 ms | 1.3% |
| ClonkMars: Chaos | 1.896 ms | 1.850 ms | 2.5% |
| Seven Keys | 1.198 ms | 1.184 ms | 1.2% |
| Tutorial 01 | 0.350 ms | 0.352 ms | -0.4% |

The equal-frame mix uses **1.3% less simulation time**,
or **1.1% without the stress case**. This is an explicit
benchmark weighting, not a measured distribution of player activity. Rendering
is excluded, so these results describe simulation time rather than FPS.
Tutorial 01 was essentially flat (0.4% more time); individual run variation is
larger than some per-scenario differences. These are measured point estimates,
not a promise of the same gain in every session.

## Method and validation

Baseline: `e8706c6d6cacc7496a01c3657559d925dad16be3`. Content:
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`. The retained baseline executable was built from
`bc8b26520126a017caf3ce31cba24e42ab9b374f`; its runtime sources, manifests,
lockfile and bundled assets are identical to the baseline merge commit.
Both executables use the same harness, compiler, features and Cargo.lock.

Recording starts before initialization. Two players join before 100 warmup ticks
and 600 measured ticks. Each sample includes the complete `Engine::tick` and its
simulation snapshot; loading, warmup and rendering are excluded. Three
fresh-process pairs per scenario alternate build order. Timing runs sequentially
after compilation, validation and state comparisons finish. Scenario results use
the median of three run means; aggregates compare medians of per-repetition
sums. No outliers are removed. Individual run means are retained in the artifact.

All **42 full-state checkpoint pairs match** at ticks 101, 200, 300, 400, 500,
600 and 700 across six workloads. Comparisons include raw fixed values,
synchronized RNG state and ordered lists. Canonical JSON hashes also match.
These Rust before/after comparisons and the primitive C++ parity gate do not
establish full-scenario parity with the C++ oracle.

Passed: **12,207 workspace tests**, 132 engine-tool tests, 639 Python tests,
workspace and FFI clippy with warnings denied, formatting, engine snapshots,
primitive C++ parity, compatibility verification and all six `cargo dev-check`
steps. The 24 workspace skips and one engine-tool skip are existing explicit
opt-ins. Four nextest process-cleanup diagnostics from the workspace run did not
recur when those tests were rerun serially; all test assertions passed in both
runs.

## Reproduction

[Raw results and harness](../benchmarks/results/object-view-performance.json)
include all **21,600 timing samples**, source and executable hashes, environment
information, checkpoint comparisons and validation results.

1. Check out the baseline and candidate source commits with pinned content.
   Extract `harness.source` to
   `crates/clonk-engine/examples/callback_lifecycle_profile.rs` in each checkout.
2. Build using `cargo build --release --locked -p clonk-engine --example
   callback_lifecycle_profile --features presentation-capture,test-graph` and
   retain each executable as `baseline` or `candidate` in a results directory.
3. Extract `orchestration_source` to `run.py`, adjust `ROOT` and `WORKTREE`, then
   run `python3 run.py states` and `python3 run.py timing`. Finish compilation and
   checks before timing. Scenario paths, seeds and stress population are in the
   script.
4. `summary_source` records aggregation and completeness checks. Compare the
   canonical state hashes as well as all decoded fields.
