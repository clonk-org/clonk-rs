# Script call frame and callback state performance

## Changes

Compiled functions retain parameter and hoisted-local layouts. Bytecode binds
these slots by index, so ordinary calls no longer rebuild name tables or hash
local names. Dynamic `VarN` access still resolves through the function layout;
duplicate parameters and parameter/local name collisions retain their existing
precedence.

Named parameters and locals reuse a bounded thread-local pool of 256 slots.
Values stay in the slot until a reference or object-reference tracking needs
shared cells. Promotion moves ownership to those cells. A slot returns to the
pool only after its last frame or continuation alias disappears, and is cleared
first. Escaped references retain their separate cells. A regression verifies
that replacing a promoted string releases the old value immediately.

Callback contexts share global effect lists and reuse object effect lists from
their existing state snapshots. A write detaches the list; read-only `EffectVar`
access does not. Global timer dispatch refreshes its shared view between
callbacks. Public snapshots and save-state formats still contain owned vectors.
Definition metadata clones also share the immutable action-graphics map.

The warmed six-parameter, three-local scalar-call allocation regression fell
from **43 allocations to 7**. Adding 64 untouched global effects previously
raised a no-op callback from **54 allocations to 247**; the regression now
requires constant allocation cost. Separate tests cover global timer scaling,
read-only effect variables, object snapshot reuse and metadata sharing. All
new allocation/sharing regressions were observed failing before their fixes.

## Measurements

Measured September 17, 2026 (UTC), on Apple M4 Max, 128 GiB, macOS 26.6.2,
Rust 1.98.1, on battery power with existing power settings unchanged. Candidate
source: `970abca647197d088cfe587bc6c5ce828fcb6fde`.

| Scenario | Before per tick | After per tick | Time reduction |
| --- | ---: | ---: | ---: |
| Tower of Magic | 1.907 ms | 1.862 ms | 2.3% |
| Arso-Morf | 4.509 ms | 4.399 ms | 2.4% |
| Arso-Morf, 1,000 Stippels | 14.414 ms | 14.017 ms | 2.8% |
| ClonkMars: Chaos | 2.276 ms | 1.966 ms | 13.6% |
| Seven Keys | 1.248 ms | 1.260 ms | -0.9% |
| Tutorial 01 | 0.388 ms | 0.369 ms | 4.8% |

The equal-frame mix uses **3.6% less simulation time**, or **4.6% without the
stress case**. Seven Keys measured about 12 microseconds slower; no samples or
scenarios were removed. The workload mix is an explicit benchmark weighting,
not a measured distribution of player activity.

## Measurement method

Baseline: `4a4e2aa96f52ac8c464c7305ebe07819569bc523`. Content:
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`. The baseline and candidate use the
same release harness, compiler, features and Cargo.lock. The harness enables
recording before initialization, joins two players, warms 100 ticks, then
measures 600 complete `Engine::tick` calls including snapshot generation.

Each of six scenarios runs in three fresh-process pairs, alternating baseline
and candidate order. One timing process runs at a time after builds and checks
finish. Every sample is retained. Results summarize the median of three run
means; aggregate reductions compare medians of per-repetition sums. Loading,
warmup and rendering are excluded, so these results describe simulation cost,
not displayed FPS.

Separate correctness runs compare every decoded `EngineState` field at ticks
101, 200, 300, 400, 500, 600 and 700. Only JSON object-key order is ignored.
This includes raw fixed-point values, object/list order and synchronized RNG
state. Matching Rust checkpoints and the primitive C++ parity gate do not
establish full-scenario parity with the C++ oracle.

All **42 complete checkpoint pairs match**. Passed: **12,191 workspace tests**,
132 engine-tool tests, 639 Python tests, clippy with warnings denied, formatting,
engine snapshots, primitive C++ parity, compatibility verification and all
11 `cargo dev-check` steps. The 24 workspace skips and one engine-tool skip are
existing explicit opt-ins. Three tests emitted nextest cleanup diagnostics
across the initial and final workspace runs; all three passed isolated rechecks
without those diagnostics.

## Reproduction

[Raw data and harness](../benchmarks/results/script-call-frame-performance.json)
include all **21,600 timing samples**, the harness and orchestration source,
environment information, executable/source hashes, state comparisons and
validation results.

1. Check out the baseline and candidate source commits with the pinned content.
   Extract `harness.source` to
   `crates/clonk-engine/examples/call_frame_profile.rs` in each checkout.
2. Build each using `cargo build --release --locked -p clonk-engine --example
   call_frame_profile --features presentation-capture,test-graph`. Preserve the
   resulting executables as `baseline` and `candidate` in a results directory.
3. Extract `orchestration_source` to `run.py`, adjust its `ROOT` and `WORKTREE`
   paths, then run `python3 run.py states` and `python3 run.py timing`. Run timing
   after all builds and validation finish. The script contains all six scenario
   paths, seeds and the optional 1,000-Stippel target.
4. `summary_source` records the aggregation and completeness checks.
   JSON object-key order is the only normalization applied to checkpoint data.
