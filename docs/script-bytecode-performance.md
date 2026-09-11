# C4Script bytecode execution

C4Script now executes exclusively through bytecode, including `eval`/DirectExec
and suspended calls. The AST remains the parser/compiler representation and
supports cache validation. The AST evaluator, task machine, continuation frames,
runtime fallback guards and fallback profiling categories have been removed.

## Measured results

Measured on September 11, 2026, on an Apple M4 Pro running macOS 26.3.1 and
Rust 1.98.1, on AC power. The baseline is
`8885060faa506734317bae070f59c6e1250cb9aa`; the candidate code is
`3f26739be`. Both use content commit
`9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009` and the same dependency lockfile.

Each scenario ran in three fresh-process pairs, alternating baseline/candidate,
candidate/baseline, then baseline/candidate. Each run joined two players, warmed
up for 100 ticks and measured 600 ticks. Recording mode was enabled before
initialization to disable the unsynchronized offline `GetTime` path. Compilation
and tests had finished, and only one benchmark process ran at a time.

The table reports the median of the three per-run mean tick times. Each tick
includes `Engine::tick` and its full `SimulationSnapshot`. Loading, warmup and
rendering are excluded. These measurements establish simulation throughput on
these fixtures, not rendered frame rates.

| Scenario | Before per tick | After per tick | Elapsed change |
| --- | ---: | ---: | ---: |
| Tower of Magic | 6.360 ms | 5.422 ms | −14.7% |
| Arso-Morf | 8.746 ms | 7.827 ms | −10.5% |
| Arso-Morf, 1,000 Stippels | 27.985 ms | 25.100 ms | −10.3% |
| ClonkMars: Chaos | 6.461 ms | 3.919 ms | −39.3% |
| Seven Keys | 2.444 ms | 1.943 ms | −20.5% |
| Tutorial 01 | 1.260 ms | 0.958 ms | −24.0% |

Every candidate run is faster than every baseline run for its scenario. Summing
the six window means within each repetition and comparing the medians gives
**15.1% less simulation time** for this equal-frame workload mix. That weighting
is a benchmark choice, not a measured distribution of player activity. All
21,600 measured frame samples are retained; none were removed as outliers.

There is a loading tradeoff: ClonkMars's median load phase increased from
4.342 s to 4.600 s, **258 ms (+5.9%)**. The other load medians changed by
−2.7% to +0.2%. These are warm-filesystem process starts, not cold-cache startup
measurements. The raw reports include loading time separately from tick time.

## Implementation

The bytecode compiler now covers foreach, reference arguments and returns,
dynamic lvalues, complex assignments, optional/global/inherited/forwarded calls,
legacy parameter lists, deferred parse errors and the remaining operators.
References and iteration state survive suspension and object removal. Value-stack
limits are enforced against live operands as instructions execute, preserving
side effects before an overflow.

Immutable instruction streams are shared across calls, stale plans are rebuilt
when a public function is mutated, and nested value-only arguments no longer
duplicate their child call sites. The measurement covers these changes together
with the interpreter removal; it does not isolate deletion alone.

## Correctness evidence

Separate baseline/candidate runs compared all captured `EngineState` fields at
ticks 101, 200, 300, 400, 500, 600 and 700. **All 42 decoded state checkpoints
match**, including raw fixed-point and RNG state. JSON object key ordering is
irrelevant to that comparison; no state values were normalized or excluded.
Checkpoint-file hashes are retained with the results. Checkpoint runs were not
used as timing evidence.

Two newly generated C++ golden sections pin behavior the former interpreter got
wrong, using the instrumented oracle at
`7d43b47b7d789b533f32d005e64596e0a07019cd`:

- `script_nil_coalescing_assignment` extracts `AB_NilCoalescingIt` from
  C4AulExec.cpp:849–856 and uses real C4Value conversions. Non-nil zero and false
  skip the right-hand side.
- `script_reference_call_increment` uses C4Aul.cpp:285–293 function lookup and
  C4AulExec.cpp:450–454 increment behavior. A script overload's returned reference
  wins over the same-named native slot; without an overload the native slot is
  incremented. C4AulParse.cpp:2808–2832 pins ordinary call selection.

Validation passed: 12,167 workspace tests; full workspace clippy; formatting;
639 Python script tests; 132 engine-tool tests; engine snapshots; primitive C++
parity; and the compatibility-profile contract. Existing manual/live-service
opt-ins remain skipped. The optional execution-profile content probe also passed.
The AST implementation was removed after the intermediate bytecode-coverage
revision passed 901 scripting tests and clippy.

`dev-check` passed replay, render, whitespace, scenario, parity and both engine
unit-test stages, then stopped because its default filter for the manual-only
profiler module selected zero runnable tests. The explicit feature-enabled
profiler probe passed; the full workspace suite covers the remaining script
stages. No assertion failure was suppressed or test disabled.

Primitive golden verification and matching Rust checkpoints are separate pieces
of evidence. Neither establishes full-scenario C++ parity or closes the existing
compatibility-profile readiness gaps.

## Reproduction and raw data

[`benchmarks/results/script-bytecode.json`](../benchmarks/results/script-bytecode.json)
contains every tick sample, per-run load times and object counts, the exact
measurement harness source, source/library/executable hashes, compiler details,
and all checkpoint comparison results.

To reproduce the workload in separate baseline and candidate checkouts:

1. Initialize their pinned content, and extract `harness.source` from the JSON
   into `crates/clonk-engine/examples/bytecode_profile.rs` in each checkout.
2. Build both with
   `cargo build --release --locked -p clonk-app --bin clonk-app -p clonk-engine --example bytecode_profile`.
   The measured libraries used engine features `default`, `presentation-capture`
   and `test-graph`, and script feature `default`; VM profiling was disabled.
   The measured external harness used optimization level 3, thin LTO and one
   codegen unit, matching the release profile.
3. After all builds finish, run the resulting executables one at a time with
   `LC_PROFILE_MODE=tick LC_PROFILE_WARMUP=100`, using the arguments below. Keep
   the same baseline/candidate alternating order for three pairs.
4. For separate state runs, also set `LC_PROFILE_STATES` to a fresh directory
   per run and compare the decoded JSON checkpoints. Exclude these runs from
   timing results.

| Scenario argument | Frames | Seed | Optional Stippel target |
| --- | ---: | ---: | ---: |
| `Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s` | 600 | 0 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | — |
| `EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s` | 600 | 424242 | 1000 |
| `ClonkMars.c4f/03_Chaos.c4s` | 600 | 424242 | — |
| `Missions.c4f/SevenKeys.c4s` | 600 | 0 | — |
| `Tutorial.c4f/Tutorial01.c4s` | 600 | 424242 | — |

The 1,000-Stippel fixture spawns real ST5B definitions with their normal
initializers and grants the same initial stuck-time grace in both variants.
