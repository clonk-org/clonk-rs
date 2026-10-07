# Retained CPU presentation performance

The software presenter executes the same painter-ordered `GpuScene` lowered for
the GPU presenter. The CPU executor bins commands into 64×64 screen tiles,
hashes commands and referenced resource revisions, and reuses unchanged tile
pixels. Changed tiles retain command order and use exact native, GUI, landscape,
solid and font sampling rules. Row strips run through Rayon. The immediate
renderer remains a test oracle.

Capture storage and immutable texture/gamma resources are reused through bounded
caches. Steady-state output buffers persist. Specialized spans preserve the
generic renderer's floating-point operation order and actual per-draw gamma LUT;
unsupported geometry and cold LUT misses use the generic executor.

Prepared ramps retain exact raw `u16 / 257.0` float values for shader blending.
Bulk sky copies can use a clamped integer transform only after verifying every
encoded LUT entry. SSE2 kernels perform these copies and ordered source-over
blends on x86-64, with scalar fallbacks on other architectures.

## Measurement contract

This fixes clonk-org/clonk-rs#1861. Measurements use the final candidate's release
test executable with `presentation-profile`, pinned content, 1280×720 output,
200 warmup frames and 300 measured frames in each fresh process. Each renderer
uses its own scenario fixture. Three process pairs use AB, BA, AB order for the
immediate comparison renderer and cached CPU executor. Forced redraw and
retained GPU lowering use separate processes. Scroll and lightning each have
three repetitions. Both one and 24 Rayon threads are measured. All 25,200 rows
are retained; no samples are removed.

Primary p50 is the median of three per-process p50s. Each percentile sorts
integer nanoseconds and selects `floor((n-1)*fraction+0.5)`, matching the Rust
probe. P95 and p99 pool all 900 samples for the scenario, path and thread count.
Linux `CLOCK_PROCESS_CPUTIME_ID` measures process CPU during rendering, including
all application threads. Wall time is the acceptance measurement. Update and
snapshot work are measured separately.

The render interval includes scene capture, CPU command lowering/execution and
copying into the caller's logical output buffer. It excludes physical window
presentation, simulation update, capture I/O and the untimed comparison/digest.
These numbers are not displayed FPS. GPU rows measure CPU lowering only.

The immediate comparison uses the same candidate executable, including shared
frontend cache changes. It is not a pristine-main baseline. The issue's historical
83.7 ms figure is not substituted for the current measured comparison.

Forced redraw invalidates every tile before rendering. The scroll probe
alternates the unclamped observer origin by 64 pixels after each update;
recorded camera coordinates prove it moved. Lightning alternates presentation
gamma ramp 5 between 96 and 0. These probes mutate presentation inputs and do
not alter engine weather or synchronized RNG.

Allocator instrumentation counts requested allocation and reallocation bytes,
including temporary work, rather than live retained memory. A frame-sized request
is at least 1280×720×4 = 3,686,400 bytes. Initial frame allocation and warmup are
excluded. Small steady-state allocations remain and are reported explicitly.

Builds and pixel checks finish before timing. Other user processes and sessions
remain running. Host load, runnable tasks, CPU ticks and CPU pressure are sampled
about once per second and included with every process. CPU busy excludes idle
and I/O wait; CPU pressure uses the `some total` counter delta over elapsed time.
Frequency and affinity are not controlled, so shared-host results do not isolate
a causal speedup.

## Results

Host: AMD Ryzen 9 9900X (12 cores, 24 logical CPUs), Arch Linux 7.2.8,
Rust 1.98.1 (`48a229cea`, 2026-09-01). Resolution is 1280×720. Each row has
900 measured frames in three fresh processes. Times are milliseconds; table
allocation means are rounded to two decimal bytes. Raw CSV integers are
preserved in the evidence.

### 1 Rayon worker

| Scenario | Path | Wall p50 | Wall p95 | Wall p99 | CPU p50 | Mean B/frame |
|---|---|---:|---:|---:|---:|---:|
| Tower of Magic | GPU lowering only | 0.735895 | 0.790339 | 0.878808 | 0.733937 | 5491983.54 |
| Tower of Magic | Cached CPU | 3.112704 | 4.177878 | 4.534339 | 3.108088 | 152881.22 |
| Tower of Magic | CPU lightning | 7.772663 | 8.382047 | 8.697930 | 7.753671 | 152780.69 |
| Tower of Magic | CPU forced redraw | 7.654206 | 8.159672 | 8.732185 | 7.639809 | 152881.22 |
| Tower of Magic | CPU scroll | 7.799975 | 8.316381 | 8.655800 | 7.774975 | 135202.77 |
| Tower of Magic | Immediate comparison | 6.901639 | 7.664176 | 8.131477 | 6.889338 | 13617252.89 |
| Seven Keys | GPU lowering only | 0.645142 | 0.706399 | 0.758508 | 0.643968 | 2645943.21 |
| Seven Keys | Cached CPU | 3.269663 | 3.923031 | 4.549377 | 3.266048 | 394185.45 |
| Seven Keys | CPU forced redraw | 7.306963 | 7.746694 | 8.033491 | 7.294018 | 394185.45 |
| Seven Keys | Immediate comparison | 12.098013 | 12.926405 | 13.735119 | 12.074495 | 7361652.59 |
| SkyBridge | GPU lowering only | 0.393371 | 0.508892 | 0.530623 | 0.392656 | 4665494.63 |
| SkyBridge | Cached CPU | 4.412045 | 5.451470 | 5.660109 | 4.402665 | 225023.24 |
| SkyBridge | CPU forced redraw | 6.039032 | 6.758276 | 6.898283 | 6.028748 | 225023.24 |
| SkyBridge | Immediate comparison | 6.064912 | 6.614291 | 6.945353 | 6.058248 | 7491628.11 |

### 24 Rayon workers

| Scenario | Path | Wall p50 | Wall p95 | Wall p99 | CPU p50 | Mean B/frame |
|---|---|---:|---:|---:|---:|---:|
| Tower of Magic | GPU lowering only | 0.739642 | 0.804556 | 0.879169 | 0.737914 | 5491983.54 |
| Tower of Magic | Cached CPU | 2.279383 | 2.716518 | 2.965403 | 4.530306 | 152881.22 |
| Tower of Magic | CPU lightning | 3.086745 | 3.427035 | 3.618270 | 11.928884 | 152780.69 |
| Tower of Magic | CPU forced redraw | 3.040396 | 3.427475 | 3.669127 | 11.710083 | 152881.22 |
| Tower of Magic | CPU scroll | 2.054574 | 2.426024 | 2.633730 | 11.142721 | 135202.77 |
| Tower of Magic | Immediate comparison | 5.511595 | 6.350457 | 6.751303 | 15.071526 | 13617252.89 |
| Seven Keys | GPU lowering only | 0.645954 | 0.692553 | 0.764259 | 0.645269 | 2645943.21 |
| Seven Keys | Cached CPU | 2.144085 | 2.560920 | 2.697451 | 4.920807 | 394185.45 |
| Seven Keys | CPU forced redraw | 2.815998 | 3.185092 | 3.353093 | 10.682197 | 394185.45 |
| Seven Keys | Immediate comparison | 4.568063 | 5.088106 | 5.309820 | 27.880858 | 7361652.59 |
| SkyBridge | GPU lowering only | 0.392841 | 0.510174 | 0.546934 | 0.391774 | 4665494.63 |
| SkyBridge | Cached CPU | 2.879579 | 3.391196 | 3.594354 | 6.746822 | 225023.24 |
| SkyBridge | CPU forced redraw | 3.464045 | 3.948741 | 4.136870 | 9.447785 | 225023.24 |
| SkyBridge | Immediate comparison | 3.588182 | 4.104158 | 4.351400 | 14.722371 | 7491628.11 |

The Tower of Magic target passes at the primary median for cached rendering,
forced redraw, scrolling and lightning. Every individual process median for
these four paths is also below 8 ms. The target is a median condition: p95 and
p99 remain above 8 ms for redraw, scrolling and lightning.

Forced redraw is 10.904178% slower than the same-executable immediate
comparison in the single-worker Tower run. Tile reuse makes normal presentation
faster; these measurements do not establish a per-pixel speedup over the current
immediate renderer. The current comparison already includes main's sprite-span
and fog improvements and the candidate's shared frontend cache changes.

Every measured CPU path has zero frame-sized allocation requests. Tower cached
and forced redraw request 152,881.22 B/frame, versus 13,617,252.89 B/frame in the
immediate comparison. Small allocations remain. Cached Tower rasterizes an
average of 33.483333 tiles and reuses 206.516667; forced redraw and lightning
rasterize all 240; scrolling rasterizes 220 and reuses 20. Scroll camera Y spans
442 through 506, confirming that the observer moved.

## Exactness and validation

The separate release probe compared 500 frame pairs from each of Seven Keys,
SkyBridge and Tower of Magic. All 1,500 pairs matched every RGBA byte, totaling
5,529,600,000 compared bytes, with equivalent input snapshots and forced tile
invalidation every fifth frame. This comparison is outside the timing interval.

Graphics regression tests exercise generic-versus-specialized sampling and
blending, all integer source/alpha/destination combinations for the shader blend
kernel, fractional gamma entries, SIMD alignment and tails, widget LUT changes,
COW resource mutation, painter order, cached tile invalidation, and fail-closed
validation. Frontend and application regressions cover ordinary captures,
scroll/lightning equality, background thumbnail publication and presenter
handoffs. Tests compare raw bytes rather than perceptual similarity.

Required workspace gates and the current-tree C++ presentation check are
separate checks. The latter captures the committed Rust producer twice and
compares it with the accepted instrumented C++ references; it does not replace
the scenario byte comparison above.


The change affects presentation. Primitive parity and the presentation verifier
do not prove full-scenario simulation parity with the pinned instrumented C++
oracle at `7d43b47b7d789b533f32d005e64596e0a07019cd`.

## Reproduction

The [raw evidence](../benchmarks/results/retained-cpu-presentation.json) contains
the complete runner and profiling source, executable and source SHA-256 values,
compiler/CPU/content fingerprints, raw CSV rows, command outputs and host samples.
Build the revision containing this evidence with the pinned content and the
recorded compiler, and check its source files against `source_sha256`:

```sh
cargo nextest list -p clonk-app --release --features presentation-profile \
  --message-format json > /tmp/retained-cpu-executables.json
```

Resolve `clonk-app::bin/clonk-app` from that JSON. Extract
`measurement.runner_source` from the evidence to a script and pass the resolved
executable plus a fresh output directory. Each checkout must own its target
directory. Do not build or run other checks concurrently with timing.

Run the separate exact comparison with the same executable:

```sh
RAYON_NUM_THREADS=1 <executable> --ignored --exact \
  tests::scenario_cpu_production_capture_profile --nocapture --test-threads=1
```

The current-tree C++ presentation check requires clean committed source:

```sh
cargo xtask presentation verify-current --profile release \
  --output-dir /absolute/fresh/presentation-current
```
