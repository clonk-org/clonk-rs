# Sprite fog performance

The software renderer combines base and owner modulation once per draw and fog
chunk, then unpacks their integer channels once. Texture sampling chooses the
contributing base and owner passes before interpolation; transparent texels
bypass fog sampling. Sky, landscape, sprites,
graphical PXS and fogged text use the prepared data. MOD2 retains its complete
quad all-black decision, owner passes retain native global tint ordering, and
flat shading retains the provoking vertex. Retained GPU capture keeps the raw
vertex colours.

Uniform chunks reuse the final packed colour for finite, clamped coordinates.
Their triangle weights sum to one within a few float ulps, so an integer channel
in 0–255 cannot cross a half-channel rounding boundary. NaN offsets retain the
original interpolation path. Varying chunks preserve the original float
multiply/add order. Channel conversion uses the truncated byte plus an exact
fraction >= 0.5 test with saturation; boundary, nonfinite and uniform-fragment
regressions pin these rules. Prepared fog sampling and nonrecursive fragment consumers are inlined into the
software pixel loops so unused vertex data and intermediate state copies can be
eliminated. The saved release row assembly records the resulting baseline
x86-64 code; no architecture-specific target feature was added.

## Integration with current main

While this change was being validated, clonk-org/clonk-rs#1902 landed the sprite
span optimization for clonk-org/clonk-rs#1860. The conflict was resolved by
sharing the prepared fog vertices with that span path. The original scalar
comparison below remains a separate experiment; its 54.39% gain is not the
incremental gain over the already optimized main revision.

Current-main baseline: `f8f2ddf01efb4b33820c4483401fa7a0f6860f56`.
Integrated candidate source: `d21f765266faad4a0802a6a4e21a516c17290e5a`.
Timing started `2026-10-07T02:48:59.461284+00:00`. Both use the same
diagnostic probe, pinned content, compiler, release profile and span mode.
The protocol is three fresh-process pairs in AB, BA, AB order, each with
200 warmup frames and 300 measured frames per scenario/path, at 1280×720
and `RAYON_NUM_THREADS=1`. No samples are removed.

Software render times in milliseconds; primary values are medians of three
per-process p50s:

| Scenario | Wall before | Wall after | Wall reduction | CPU before | CPU after |
| --- | ---: | ---: | ---: | ---: | ---: |
| SkyBridge | 6.248435 | 6.478526 | -3.68% | 6.233568 | 6.466969 |
| Tower of Magic | 8.095176 | 7.432922 | 8.18% | 8.028915 | 7.422199 |
| Seven Keys | 12.298505 | 12.794600 | -4.03% | 12.262363 | 12.734053 |

Render CPU uses Linux `CLOCK_PROCESS_CPUTIME_ID` around the render interval.
It includes all application threads active in that interval. The untimed
per-frame RGBA digest and capture I/O are outside the interval. Whole-process
CPU additionally includes loading, warmup, updates and both presentation paths.
Retained rows measure CPU lowering, not GPU execution or displayed FPS.

Pooled tails use all 900 software frames per revision and scenario:

| Scenario | Before p95 | After p95 | Before p99 | After p99 |
| --- | ---: | ---: | ---: | ---: |
| SkyBridge | 7.514386 | 7.117187 | 9.939532 | 7.701693 |
| Tower of Magic | 13.945431 | 8.297484 | 17.302890 | 8.726103 |
| Seven Keys | 15.008063 | 13.971009 | 20.090931 | 15.361897 |

Other sessions remained running throughout. Our builds, checks and pixel
captures finished before timing. All individual runs and their host contention
are retained:

| Pair | Revision | Tower p50 ms | Seven Keys p50 ms | SkyBridge p50 ms | One-minute load range | Host CPU busy | CPU pressure some |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | before | 7.847204 | 12.298505 | 6.123077 | 2.83–3.36 | 11.11% | 0.09% |
| 1 | after | 7.432922 | 12.659905 | 6.572516 | 2.78–2.92 | 15.60% | 0.12% |
| 2 | after | 7.515218 | 12.896717 | 6.478526 | 2.89–3.36 | 15.40% | 0.11% |
| 2 | before | 13.258087 | 13.235655 | 6.477644 | 3.28–9.14 | 57.99% | 3.69% |
| 3 | before | 8.095176 | 12.256211 | 6.248435 | 7.16–9.14 | 17.89% | 0.13% |
| 3 | after | 7.418342 | 12.794600 | 6.282471 | 5.11–7.16 | 8.32% | 0.14% |

The observed one-minute load ranged from 2.78 to 9.14 on the 24-CPU host.
Load, runnable tasks, pressure and CPU ticks were sampled about once per
second. The same counter-delta definitions below apply. These measurements
remain subject to changing contention; they do not isolate a causal speedup.
SkyBridge and Seven Keys have higher observed medians in the candidate.
The second baseline process coincided with 57.99% host CPU busy and 3.69%
CPU pressure. Render CPU excludes descheduled time but remains sensitive to
frequency and cache contention; these readings cannot attribute the mixed
results solely to code changes or solely to competing compilation.

Separate capture runs compare **900 integrated frame pairs** and
**3,317,760,000 RGBA bytes** directly with current main. Every byte also
matches the original `6ec60f0f` baseline. No normalization is applied.
The integrated current-tree C++ presentation check passes **46 comparisons
across 23 cases**, using two producer captures under the declared contracts.
The pinned oracle and all accepted fixtures/goldens remain unchanged.

Integrated validation passes: **12,822 workspace tests** (29 opt-in skips),
1,328 frontend tests (two opt-ins), 132 engine-tool tests (one opt-in),
1,038 Python tests (three opt-ins), workspace and profiling-feature clippy
with warnings denied, formatting, engine snapshots, primitive C++ parity,
compatibility and all five development-check steps. Compatibility acquisition
excludes only the owned private baseline checkout through per-process Git
configuration; it does not omit tracked product source or alter shared config.
Presentation capture uses a private network namespace with the current UID
to avoid other sessions occupying its fixed ports.

The original scalar experiment demonstrates the 30.6% bottleneck-share goal.
The integrated experiment measures the additional change after the overlapping
span optimization, and the final Tower p50 remains below 60 ms.

[Integrated raw samples, hashes, harness and verification receipts](../benchmarks/results/sprite-fog-integration.json)
contain 10,800 rows, including 5,400 software rows with render CPU readings.
Reproduce using the named source commits, identical `source.probe_source`,
`source.build_command` and `source.orchestration_source` in independent target
directories. `postchecks_source` records capture and direct byte comparisons.

## Original standalone measurements

Measured 2026-10-07T01:55:09.670957+00:00 on AMD Ryzen 9 9900X, 24 logical CPUs,
Arch Linux 7.2.8-arch1-2, Rust 1.98.1.
Baseline: `6ec60f0f84e5dd5c5d9070f2739543aae3797904`. Candidate source:
`509f399b98504cdd9886b50214323ccaf8920e96`. Content:
`0888b4f3bd10762c976c2fe93aa650d7909c1c6f`.

At 1280×720 with `RAYON_NUM_THREADS=1`, software **render time** in milliseconds:

| Scenario | Before p50 | After p50 | Reduction |
| --- | ---: | ---: | ---: |
| SkyBridge | 29.393929 | 14.306128 | 51.33% |
| Tower of Magic | 83.993694 | 38.309451 | 54.39% |
| Seven Keys | 81.513312 | 38.701283 | 52.52% |

Tail latencies pool all 900 software samples per scenario and revision:

| Scenario | Before p95 | After p95 | Before p99 | After p99 |
| --- | ---: | ---: | ---: | ---: |
| SkyBridge | 30.945389 | 15.235282 | 33.820191 | 17.225943 |
| Tower of Magic | 86.320154 | 40.651274 | 87.685962 | 44.323086 |
| Seven Keys | 82.668722 | 71.829059 | 84.483223 | 76.480530 |

Tower of Magic meets clonk-org/clonk-rs#1859's 30.6% reduction and ≤60 ms p50
requirements in these measurements. These compare a fresh baseline against the
candidate; the issue's historical 83.7 ms is not substituted for today's baseline.

Each scenario and presentation path uses an independent fixture, simulation seed
0, presentation SafeRandom seed 1, 200 warmup frames and 300 measured frames
(201–500). Three fresh-process pairs run sequentially in before/after,
after/before, before/after order. The p50 comparison uses the median of three
per-process percentiles. Each sorts 300 integer nanosecond samples and selects
index `round(299 * fraction)`, matching the Rust probe. Pooled tails sort all
900 samples per scenario and revision and select `round(899 * fraction)`.
No samples are discarded.
All 10,800 rows are included: 5,400 software and 5,400 retained-lowering rows.
Retained results measure CPU scene lowering, not GPU execution or displayed FPS.

## Original standalone contention

Other sessions remained running as requested. The user reported a load average
of 182 during validation; the actual one-minute load during these timing runs
ranged from 2.10 to 9.04. This worktree finished all builds,
checks and pixel captures before timing. Host load, runnable tasks, CPU pressure
and CPU ticks were sampled approximately once per second.

| Pair | Revision | Tower p50 ms | Seven Keys p50 ms | SkyBridge p50 ms | One-minute load range | Host CPU busy | CPU pressure some |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | before | 83.993694 | 81.157797 | 29.307913 | 2.15–3.94 | 8.60% | 0.12% |
| 1 | after | 38.309451 | 38.006514 | 14.059116 | 2.10–2.39 | 12.85% | 0.09% |
| 2 | after | 38.517760 | 65.929202 | 14.329062 | 2.24–9.04 | 38.68% | 2.25% |
| 2 | before | 83.263936 | 81.513312 | 29.393929 | 2.61–7.23 | 8.85% | 0.14% |
| 3 | before | 84.396850 | 81.611952 | 29.547423 | 2.44–7.89 | 26.70% | 1.37% |
| 3 | after | 38.083262 | 38.701283 | 14.306128 | 4.62–5.77 | 11.62% | 0.18% |

Seven Keys' second optimized process measured 65.929202 ms p50, compared with
38.006514 and 38.701283 ms in its other optimized runs. This variation remains
in the table, raw data and pooled tails.

CPU pressure is the fraction of the sampled interval with at least one host task
waiting for CPU, derived from `/proc/pressure/cpu` counter deltas. CPU busy uses
aggregate `/proc/stat` deltas, excluding idle and I/O wait. Raw samples, rolling
pressure averages, start times, process CPU and elapsed time are retained.
These are observed shared-host wall times. Changing contention prevents an
isolated estimate of the optimization's causal speedup. Whole-process CPU time
also includes loading, warmup, updates and both presentation paths; it is not
per-frame rendering CPU time.

## Original standalone exactness and validation

Separate pixel runs compare **900 frame pairs**, or **3,317,760,000 bytes**, as
RGBA8888. Every byte matches the baseline, without normalization. Capture I/O
and these runs are excluded from performance results. Each stream has a SHA-256
receipt and a successful `cmp` result. The final current-tree presentation
verifier passes **46 comparisons across 23 C++ oracle cases**, with two
independent producer captures, under each case's declared pixel/layout
comparison contract. Accepted fixtures
and goldens were not edited.

The standalone revision passed all required gates: **12,799 workspace tests**, 1,314 frontend tests, 132
engine-tool tests and 1,038 Python tests; clippy with warnings denied, formatting,
engine snapshots, primitive C++ parity, compatibility and all five `dev-check`
steps. The 28 workspace, one frontend, one engine-tool and three Python skips
are existing explicit opt-ins. The presentation capture used a private network
namespace with the current user ID because concurrent sessions occupied its
fixed ports. Source and capture inputs were unchanged by that retry.

This change affects presentation only. No simulation, control or synchronized
RNG implementation changed. Primitive parity and rendering checks do not prove
full-scenario simulation parity with the pinned C++ oracle
`7d43b47b7d789b533f32d005e64596e0a07019cd`.

## Reproduction

[Raw samples, executable/source hashes, harness and receipts](../benchmarks/results/sprite-fog-performance.json)
include all three rejected candidate pairs separately from the accepted six runs.
The first regressed; the second improved Tower of Magic to 67.667595 ms but
missed acceptance. The uniform candidate also missed it at 70.645185 ms. Their measurements were
preserved before further changes.

1. Use independent baseline and candidate checkouts at the commits above with
   the pinned content. Extract `source.probe_source` to
   `crates/clonk-app/src/main_tests/scenario_frame_profile.rs` in both checkouts.
   Each checkout must have its own target directory.
2. Run `source.build_command` in each checkout and preserve the listed
   `clonk-app::bin/clonk-app` executable. Both use the same Cargo.lock,
   compiler, release profile and `presentation-profile` feature.
3. Extract `source.orchestration_source`, adjust `ROOT`, `EVIDENCE` and
   `EXECUTABLES`, finish builds and validation, then run it. It writes every
   timing sample and contention reading. `report_source` records completeness
   checks and aggregation.
4. For pixel proof, run each preserved binary with
   `--exact tests::scenario_frame_profile --ignored --nocapture`,
   `RAYON_NUM_THREADS=1`, `LC_FRAME_PROFILE_OUTPUT=<directory>` and
   `LC_FRAME_PROFILE_PIXELS=1`. Extract `pixels.comparison_source`, adjust
   capture/executable paths and run it. It checks all three exact-sized streams
   with `cmp` before hashing them.
