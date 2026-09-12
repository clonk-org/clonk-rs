# Script and host bookkeeping performance

The follow-up to bytecode-only execution reduces repeated reference discovery,
object-query materialization and callback-context construction. These are combined
A/B measurements of the changes; they do not isolate each optimization's contribution.

## Measured results

Measured September 12, 2026 (UTC), on an Apple M4 Pro with 48 GiB, macOS 26.3.1,
Rust 1.98.1 and AC power, with the existing power settings unchanged. Baseline:
`e4be415fc82925376e4155b7c5afa02b671372ce`. Candidate code:
`5d6143404dac74f343fb7bd07bd123793c9fa648`. Both use content
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009` and the same Cargo.lock.

| Scenario | Before per tick | After per tick | Time reduction |
| --- | ---: | ---: | ---: |
| Tower of Magic | 5.382 ms | 3.122 ms | 42.0% |
| Arso-Morf | 7.811 ms | 7.017 ms | 10.2% |
| Arso-Morf, 1,000 Stippels | 25.054 ms | 22.713 ms | 9.3% |
| ClonkMars: Chaos | 3.902 ms | 3.371 ms | 13.6% |
| Seven Keys | 1.920 ms | 1.865 ms | 2.9% |
| Tutorial 01 | 0.960 ms | 0.575 ms | 40.1% |

The six-scenario equal-frame mix uses **14.1% less simulation time**.
Excluding the 1,000-Stippel stress case gives **20.1% less time**.
These weights describe the benchmark, not a measured distribution of player activity.
Every candidate run was faster than every baseline run for its scenario.

Each scenario ran in three fresh-process pairs, alternating baseline/candidate,
candidate/baseline, then baseline/candidate. Every run joined two players, warmed
up for 100 ticks and measured 600 complete `Engine::tick` calls, including the
full `SimulationSnapshot`. Recording was enabled before initialization to disable
the unsynchronized offline `GetTime` path. All builds and validation finished before
timing; one benchmark process ran at a time. All 21,600 tick samples are retained,
with no outlier removal. Checkpoint runs are separate and excluded from timings.

Rows report the median of three per-run means. Aggregate reductions compare the
medians of per-repetition sums of the selected scenario means. These are headless
simulation measurements, with loading, warmup and rendering outside the timed
window; they do not establish a displayed FPS or GPU improvement.

Loading is retained separately. These are warm-filesystem process starts, not
cold-cache startup measurements:

| Scenario | Before loading | After loading | Change |
| --- | ---: | ---: | ---: |
| Tower of Magic | 0.791 s | 0.772 s | -2.4% |
| Arso-Morf | 0.373 s | 0.360 s | -3.5% |
| Arso-Morf, 1,000 Stippels | 0.373 s | 0.366 s | -1.8% |
| ClonkMars: Chaos | 4.511 s | 4.191 s | -7.1% |
| Seven Keys | 0.972 s | 0.948 s | -2.5% |
| Tutorial 01 | 0.220 s | 0.212 s | -3.8% |

## Changes

- Named global and constant tables carry a mutation generation. Nested calls
  reuse the current registration while that generation is unchanged. Replacing
  an existing key invalidates the registration even when the table length is
  unchanged; generation exhaustion disables caching. The registry retains the
  exact table identity and releases it with its owning frame. Scalar discovery
  avoids an extra membership lookup and weak-reference upgrade; already indexed
  cells can still be discovered while mutably borrowed.
- Legacy rectangular searches check coordinates through the existing scalar
  candidate path. Closest searches read positions without copying full host
  objects. Master/sector iteration order, arithmetic and distance tie handling
  are unchanged. Pending and scoped object changes still use their callback-local
  state. Both new search fixtures reduce host-object materializations from 66 to 1.
- Engine callback contexts initialize 13 immutable definition-table handles from
  their shared cache, avoiding disposable empty allocations. Attaching the same
  definition-script handles reuses the compiled function namespace and order;
  replacing a handle under the same ID rebuilds them. Mutable callback state
  remains independent.

The reference registry still starts afresh for each outermost call, and numbered
slots still undergo discovery. This does not remove all script bookkeeping or all
object copies. `GlobalVariables` now wraps a versioned table; embedding code uses
`new_global_variables()` and its existing `borrow`/`borrow_mut` interface. Cell
writes continue to use `set_value_cell` to maintain immediate removal tracking.

## Correctness and validation

All **42 complete decoded EngineState checkpoints match** at ticks 101, 200, 300,
400, 500, 600 and 700 across all six scenarios. This includes raw fixed-point,
object ordering and synchronized RNG state. Only JSON object-key ordering is
ignored; no state values are normalized or excluded. File and canonical hashes
are retained with the raw data.

Each optimization began with an observed failing regression test. Coverage includes
unchanged and replaced globals, scalar-to-object writes followed
by removal, discovery of a mutably borrowed registered cell, query materialization,
namespace replacement and direct construction from shared definition tables.
Existing removal, nested-call, query-order and pending-mutation coverage stays active.

Validation: 12,175 workspace tests; 132 engine-tool tests; 639 Python script tests;
workspace and FFI clippy with warnings denied; formatting; engine snapshots;
primitive C++ parity; compatibility-profile verification; and the development check.
Existing manual/live-service opt-ins remain skipped. The pinned C++ oracle is
`7d43b47b7d789b533f32d005e64596e0a07019cd`. Primitive parity and matching Rust state
checkpoints provide complementary evidence; neither proves full-scenario C++ parity.

## Reproduction

[`benchmarks/results/script-bookkeeping.json`](../benchmarks/results/script-bookkeeping.json)
contains every tick sample, per-run loading/object counts, harness and orchestration
sources, compiler/power details, source/library/executable hashes, and all state
comparison results.

1. Create baseline and candidate checkouts with their pinned content initialized.
   Extract `harness.source` into `crates/clonk-engine/examples/bookkeeping_profile.rs`
   in each checkout.
2. Build each with `cargo build --release --locked -p clonk-engine --example bookkeeping_profile --features presentation-capture,test-graph`. The measured
   libraries used engine features `default`, `presentation-capture`, `test-graph`
   and script feature `default`. VM profiling was disabled. The common external
   driver used optimization level 3, thin LTO and one codegen unit, matching release.
3. After builds and checks finish, run one executable at a time with
   `LC_PROFILE_MODE=tick LC_PROFILE_WARMUP=100`, using the arguments below and the
   alternating order above.
4. For separate state runs, also set `LC_PROFILE_STATES` to a fresh directory for
   each process, then compare every decoded checkpoint without dropping fields.

| Scenario argument | Frames | Seed | Stippel target |
| --- | ---: | ---: | ---: |
| `Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s` | 600 | 0 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | 1000 |
| `ClonkMars.c4f/03_Chaos.c4s` | 600 | 424242 | — |
| `Missions.c4f/SevenKeys.c4s` | 600 | 0 | — |
| `Tutorial.c4f/Tutorial01.c4s` | 600 | 424242 | — |

The stress fixture spawns real ST5B definitions with their normal initializers and
applies identical initial stuck-time grace to both variants.
