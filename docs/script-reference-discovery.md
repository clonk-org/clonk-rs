# Shared script-reference discovery

Nested C4Script calls still discover shared global cells on every entry. Hosts
can replace a table entry between calls, so treating the whole table as already
visited would miss live object references during `AssignRemoval`.

Discovery now borrows the thread-local reference index once per table and
borrows the existing cells directly. This avoids a thread-local lookup and
`Rc` → `Weak` → `Rc` conversion for each cell. Scalar values bypass recursive
reference counting and empty-index/frame-pruning work. Overwriting an indexed
object reference with a scalar still removes its old membership. Arrays,
maps, changed cells, escaped references and frame pruning retain their existing
behavior; no simulation callback or RNG draw is removed.

Two regression tests were first observed failing on the old implementation:
128 shared cells acquired the index 128 times instead of once, and a nested
call with 128 scalar globals visited 139 reference values instead of zero.
Both also check that subsequent object removal still clears live references.
The existing replacement, address-reuse, nested-container and escaped-cell
regressions remain enabled.

## Results

Mean elapsed time for each complete simulation tick, averaged across the two
runs per variant:

| Scenario | Before | After | Elapsed change |
| --- | ---: | ---: | ---: |
| Tower of Magic | 6.973 ms | 6.373 ms | −8.6% |
| Arso-Morf | 10.457 ms | 8.872 ms | −15.2% |
| Arso-Morf, 1,000 Stippels | 34.720 ms | 28.948 ms | −16.6% |
| ClonkMars, offline observation | 7.719 ms | 6.525 ms | −15.5% |

Both repetitions improved in every workload. For the dense fixture, baseline
run means were 34.748 and 34.692 ms; candidate means were 29.233 and 28.662 ms.
Its per-run p95 decreased from 66.530/66.159 ms to 54.552/53.172 ms.
The remaining cost still exceeds the ordinary offline tick budget in this
stress fixture; this change does not eliminate every slow frame.

Pooled latency across 1,200 measured ticks per variant (p50 is the median;
p95/p99 select the nearest sample):

| Scenario | p50 before → after | p95 before → after | p99 before → after |
| --- | ---: | ---: | ---: |
| Tower of Magic | 5.054 → 4.503 ms | 12.014 → 10.635 ms | 36.266 → 34.945 ms |
| Arso-Morf | 10.129 → 8.516 ms | 12.080 → 10.452 ms | 15.751 → 13.998 ms |
| 1,000 Stippels | 28.971 → 24.387 ms | 66.429 → 54.103 ms | 92.076 → 72.963 ms |

The shared-global microbenchmark reports the median per entry across both
runs, each with nine samples of 1,000 entries. Each entry makes 32 nested calls:

| Shared globals | Before | After | Elapsed change |
| --- | ---: | ---: | ---: |
| 0 (control) | 55.595 µs | 54.936 µs | −1.2% |
| 128 | 117.332 µs | 78.237 µs | −33.3% |
| 512 | 318.201 µs | 154.873 µs | −51.3% |

The small control difference does not establish a useful speedup without
shared globals. The larger microbenchmark reductions describe script-call
overhead, not complete app frame-rate gains.

Raw tick samples, microbenchmark samples, window summaries, state-comparison
hashes, harness sources, build commands and binary/source hashes are retained
in [`benchmarks/results/script-reference-discovery.json`](../benchmarks/results/script-reference-discovery.json).
No timing outliers are removed.

All 12 attempted windowed trials were occluded/offscreen and produced zero
presentation submissions, including after early activation, activation after
scenario load, and a temporary display-awake assertion. They are retained as
**invalid presentation measurements**; no visible FPS or GPU improvement is
claimed. The offscreen dense app did advance about 31.38 → 37.56 simulation
ticks/s, but that excludes presentation work and is not a visible-gameplay
throughput result. The complete fixed-work simulation measurements above are
the primary evidence.

## Measurement method

The baseline is `8d85fbf0ba3530ef7265f75f2c62760a18c459eb`. Both variants use
content `9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`, Rust 1.98.1, the release
profile, and the same Apple M4 Pro with 48 GiB on macOS 26.3.1. These new paired
runs use battery power with the existing power settings. They should not be
compared directly with earlier measurements taken under different conditions.

Each workload runs baseline, candidate, candidate, baseline, sequentially,
with compilation and sampling disabled during timing. Headless runs load real
content, join two players, warm up 100 ticks, and time 600 complete
`Engine::tick` calls, including snapshot construction. The dense fixture
creates exactly 1,000 ST5B objects before warmup using their real definition
and initialization callbacks. Object counts are retained with each run.

Windowed runs use 1280×720 Metal, automatic frame skipping, two seconds of
warmup and a 20-second measurement window. Ordinary gameplay is tick-paced,
so lower CPU cost need not raise its simulation FPS. Occluded windows do not
qualify for presentation comparisons. GPU timestamp readback limitations are
reported separately from CPU frame timing.

## Determinism checks

Separate runs serialize the complete persisted engine state after 101, 200,
300, 400, 500, 600 and 700 ticks. Canonicalization sorts JSON object keys only;
array and execution order remain intact. All 21 checkpoints match exactly for
Tower of Magic, Arso-Morf and the 1,000-Stippel fixture, including raw fixed-point
motion, object order, landscape, effects, globals and synchronized RNG state.

ClonkMars offline play is sensitive to elapsed real time:
`content/ClonkMars.c4d/System.c4g/TIME.c:38-41` calls native `GetTime()` when
calculating temperature. The native host deliberately exposes that clock
outside network/replay/recording mode (`compat/world.rs`, `get_time`, mirroring
`C4Script.cpp:4647-4652`). A second run of the unchanged baseline reproduces
weather, landscape and mass-mover differences. The candidate and baseline still
match object state and the complete synchronized RNG at every checkpoint.
ClonkMars timing is therefore an observational offline comparison, not a
fixed-work deterministic comparison or full-state equivalence result.

These checks supplement the required C++ primitive parity and engine snapshot
gates; they do not establish full-scenario C++ parity.

## Reproduction

The permanent shared-global benchmark exercises 32 nested calls with 0, 128 or
512 globals, including one live object reference per 16 globals:

```sh
cargo bench --locked -p clonk-script --features bench --bench script_execution -- \
  script_nested_calls_with_shared_globals
```
