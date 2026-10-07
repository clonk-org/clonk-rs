# Software sprite and fog span performance

This change addresses clonk-org/clonk-rs#1860. The acceptance measurement is
Tower of Magic's complete software render at 1280x720, with one Rayon worker;
its target is p50 at or below 8 ms. Sprite-loop throughput is a separate
component measurement and does not establish a whole-game FPS multiplier.

## Measured results

Measured October 7, 2026 UTC, on an AMD Ryzen 9 9900X (24 logical CPUs),
Arch Linux kernel 7.2.8, Rust 1.98.1. Power settings are unchanged; the governor
is `powersave`, with boost enabled. The source baseline is
`6ec60f0f84e5dd5c5d9070f2739543aae3797904`; pinned content is
`0888b4f3bd10762c976c2fe93aa650d7909c1c6f`. Candidate source and executable
SHA-256 fingerprints are retained with the raw measurements.

| Software scenario | Scalar p50 | Span p50 | Span p95 | Span p99 | Render speedup |
| --- | ---: | ---: | ---: | ---: | ---: |
| Tower of Magic | 83.818890 ms | 7.874007 ms | 8.524830 ms | 8.965680 ms | 10.645x |
| Seven Keys | 79.724953 ms | 11.915355 ms | 12.200140 ms | 13.356568 ms | 6.691x |
| SkyBridge | 28.760626 ms | 6.208917 ms | 7.080200 ms | 7.337631 ms | 4.632x |

These are the original, fixed three-pair batch's medians of per-process
percentiles. Tower's individual optimized p50s are 7.773974, 7.874007, and
8.890129 ms. Two meet 8 ms; one misses it. Pooling all 900 original optimized
Tower samples gives 8.128552 ms p50. Every sample is retained, including that
slower run. A fixed follow-up batch checks this variation separately; it does
not replace or discard the original results.

The follow-up's optimized p50s are 7.544160, 7.807263, and 8.045839 ms. Its
900 optimized frames have a pooled p50 of 7.747128 ms. Across both batches,
all 1,800 optimized Tower frames have **7.902371 ms pooled p50**, 10.747960 ms
p95, and 16.052759 ms p99. This pooled median meets 8 ms while retaining both
individual runs whose medians exceeded it. These results establish the target
on this workload and machine; individual process medians vary.

Retained lowering, median of three per-process p50s: Seven Keys
0.862337 -> 0.834694 ms; SkyBridge 0.481239 -> 0.489545 ms;
Tower of Magic 0.898365 -> 0.845514 ms. The small differences and the observed
variation do not establish a retained-GPU speedup. Raw update and process-CPU
timings accompany the render measurements.

| Sprite mode | Scalar p50 ns/pixel | Span p50 ns/pixel |
| --- | ---: | ---: |
| Opaque | 44.768904 | 0.875435 |
| Alpha-over | 56.348509 | 2.408800 |
| Additive | 45.202048 | 1.451100 |
| MOD2 | 50.739306 | 1.871783 |
| MOD2-additive | 49.137789 | 1.452928 |

All five component modes meet 4 ns/pixel at p50, with complete output-buffer
equality. These use baseline x86-64 instructions, without a native-CPU target.

## Implementation

Nearest, untransformed sprites resolve clipping and horizontal source coordinates
before entering the rows. Separate loops handle ordinary alpha composition,
additive composition, MOD2, and MOD2-additive. Opaque texels copy encoded RGB
directly. Baseline x86-64 SSE2 processes two legacy alpha-over pixels in eight
integer lanes, preserving the existing floor division and alpha equation.
Other architectures retain equivalent scalar arithmetic.

Shader modulation and gamma become bounded, per-thread color palettes. Their
keys include the complete renderer configuration and gamma-ramp contents;
32 palette entries and four gamma entries prevent unbounded retention. Partial
shader opacity retains the original f32 operations and rounding through SSE2.
An integer approximation of the gamma/blend equation would change bytes near
rounding boundaries, so it is deliberately not used.

Fog vertices combine once per draw. Source-chunk decisions remain authoritative
for MOD2, including black fragments inside a nonblack quad. Vertex interpolation
keeps the original product/addition order. Uniform quads reuse color palettes.
Cached sky and ground texels use the same specialized fragment rules; fully
opaque black sky spans can fill or blend pairs without fetching source RGB.
Animated liquids retain their original preparation.

Transformed and filtered sprites retain their original texture sampling and
owner-color layer ordering. Transformed sprites without active owner-color
layers reuse precombined fog vertices, and the generic compositor caches gamma
and uses exact SIMD float composition. GPU capture takes its existing path.
Object drawing still records `rendered_object_audibility_calls` at its original
call sites. Simulation, control, and synchronized RNG are untouched.

The specialization adds temporary source-coordinate, fog-axis, and quad vectors
per draw. In Tower of Magic's first final pair, mean render allocation calls
increase from 3,084 to 6,630 and allocated bytes from 15,414,898 to 15,846,691
(2.8%). The measured render-time improvement includes this setup cost; the
large-span component probe alone does not characterize tiny-draw overhead.

## Measurement method

Final results are recorded in
[`benchmarks/results/software-sprite-spans.json`](../benchmarks/results/software-sprite-spans.json).
It retains every sample in nanoseconds, the execution order, complete logs,
source and executable hashes, compiler/platform details, and workload hashes.

The comparison uses a single release executable built from the candidate.
The `presentation-profile` feature enables an opt-in scalar-reference switch
that bypasses each new specialization. Both variants use the same compiler,
content, game state, allocation instrumentation, and executable. Each fresh
process warms 200 frames and measures 300. Timing processes run sequentially,
with `RAYON_NUM_THREADS=1` and all threads pinned to logical CPU 23. Each scenario
runs three software pairs, alternating reference/optimized, optimized/reference,
then reference/optimized. No samples are removed. Three retained-render pairs
check capture/lowering; they do not measure GPU display. The first retained pair
showed substantial variation alongside changes in unchanged simulation timing,
so two additional alternating pairs were specified before the follow-up.
Tower also runs a separate fixed set of three additional alternating software
pairs after the component probe, following the original batch's missed run.
The original three-pair summaries remain the primary scenario comparison;
the pooled Tower results explicitly include both batches.

Wall-clock render time is the acceptance metric. Linux process CPU time is
retained as a secondary measure and includes the Rayon worker. The RGBA
fingerprint is computed outside the timer. Percentiles select the sorted sample
at `round((n - 1) * fraction)`, matching `scenario_frame_profile`; each aggregate
is the median of the three per-process percentiles.

A separate correctness run sets `CLONK_FRAME_PROFILE_COMPARE_RGBA=1` and compares
all 3,686,400 output bytes after each of 300 frames per scenario. Those runs
perform an additional render and are excluded from timing results.

The sprite component probe stretches a deterministic 64x64 RGBA texture over
1280x720 with standard gamma. Each mode runs 200 warmup draws and 300 measured
draws through each implementation, then compares complete output buffers.
This is a warm-cache, single large-draw probe, not a distribution of game sprites.

## Reproduction

Initialize the pinned content before compiling test targets. Build:

```sh
CARGO_BUILD_JOBS=4 cargo nextest run --release -p clonk-app \
  --features presentation-profile,app-test-shard-12 --no-run
```

Run the resulting test executable from the checkout root:

```sh
RAYON_NUM_THREADS=1 CLONK_FRAME_PROFILE_PATH=software \
CLONK_FRAME_PROFILE_SCENARIO=Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s \
CLONK_FRAME_PROFILE_OUTPUT_DIR=/tmp/sprite-span-optimized \
taskset -c 23 <test-executable> --ignored --exact \
  tests::scenario_frame_profile --nocapture
```

Add `CLONK_FRAME_PROFILE_SPRITE_REFERENCE=1` for the scalar run. Use fresh output
directories, and finish builds and validation before measuring. For the separate
byte-comparison run, set `CLONK_FRAME_PROFILE_COMPARE_RGBA=1`. Omit that setting
from timing runs.

For per-pixel throughput, build the frontend companion test crate in release
mode and run its ignored `sprite_spans::tests::sprite_span_profile` test, setting
`CLONK_SPRITE_SPAN_PROFILE_OUTPUT_DIR` to retain the 300 draw times per mode.

## Correctness coverage

The strict unit comparisons cover clipping, fractional source rectangles,
scaling, flips, malformed images, gamma ramps, gradient/uniform fog, flat-shading
vertices, all blend modes, renderer alpha flags, filtered fractional fragments,
and owner-color layers. Legacy pair blending exhausts all 16,777,216 combinations
of source channel, destination channel, and opacity; black shader pair blending
exhausts all 65,536 opacity/destination combinations. Transformed-fog preparation
compares shader channels as raw f32 bits. No byte tolerance is introduced.

Primitive C++ parity, engine snapshots, and full-render scalar comparisons are
different checks. The primitive golden does not prove full-scenario C++ parity;
this optimization preserves the existing Rust presentation oracle.

Local gates pass: 12,802 workspace tests (29 explicit opt-ins), workspace clippy
with warnings denied, formatting, four engine snapshots, primitive C++ parity,
132 engine-tool tests (one manual opt-in), the Python suite (1,038 tests, three
skips), measurement-feature clippy, and all six development-check commands.
Compatibility verification requires a clean committed source identity and is
run after committing these artifacts, before opening the pull request.
