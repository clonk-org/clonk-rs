# Rendering and script performance follow-ups

These changes build on `190d17e30d1a4549b2aa78895213c66dbc35fce6`.

## Measured results

Measured on an Apple M4 Pro, macOS 26.3.1, with Rust 1.98.0 and the release
profile on September 10–11, 2026. Both variants used identical fixtures,
`Cargo.lock`, and content commit `9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`.
Compilation finished before timing. Each component ran in the order baseline,
candidate, candidate, baseline, with one benchmark process at a time.

The table reports the median sample time per iteration pooled across the two
trials. Negative elapsed changes mean less time. These are component timings,
not complete game frame rates.

| Workload | Before | After | Elapsed change |
| --- | ---: | ---: | ---: |
| Software frame, 1,000 object faces | 11.475 ms | 9.694 ms | −15.5% |
| Translucent 1280×16 HUD strip | 447.554 µs | 157.893 µs | −64.7% |
| Value-only method call | 4.998 µs | 3.979 µs | −20.4% |
| Sparse landscape, two distant edits | 1.095 ms | 0.525 ms | −52.1% |
| Dense landscape edit control | 1.063 ms | 1.071 ms | +0.7% |
| Unchanged landscape control | 310.315 µs | 310.155 µs | −0.05% |

The GPU fixture uses a 1024×1024 landscape, composition detail two, populated
primary/overlay material patterns, and a 1024×1024 offscreen target. Wall time
includes cloning the pending plan, CPU encoding, submission, and waiting for
GPU completion. Sparse edits reduce the upload from 984,064 bytes in one call
to two bytes in two calls, and composition from 3,936,256 texels to eight.
Dense edits retain one full-plane upload. Neither control establishes a useful
speedup; the dense control is slightly slower.

A separate device measured the shader-landscape pass with GPU timestamps.
After four warm frames, each trial retained 32 samples with verified frame IDs
and valid timestamps. The pooled sparse-pass median was 469.687 µs before and
35.604 µs after. These short windows show substantial GPU clock variation:
the dense-pass medians range from 485.875 to 706.626 µs before and 531.688 to
659.125 µs after. Use the completed-frame measurements above for the primary
comparison. Timestamp instrumentation was disabled on that device.

Raw Criterion samples and estimates, GPU timestamps, scenario windows, source
fingerprints, and executable hashes are retained in
[`benchmarks/results/performance-followups.json`](../benchmarks/results/performance-followups.json).
All trials are included; no outliers are removed from the reported medians.
The data is specific to this machine and these fixtures.

The real-content probe was rebuilt from final source commit
`a09c4711c5386ad9b13f1ff4e31609b4ef1afab2` and compared with the baseline
in the same reversed order. Each trial contains five fresh seed-zero runs per
scenario, with 100 warmup frames and 400 measured frames. The following values
are medians of the ten window means per variant with VM elapsed clocks
disabled. Invocation counters remain enabled. Loading, snapshots, and rendering
are excluded.

| Scenario | Before per tick | After per tick | Elapsed change | AST invocations per 400 frames |
| --- | ---: | ---: | ---: | ---: |
| Tower of Magic | 6.037 ms | 5.871 ms | −2.76% | 23,387 → 16,910 |
| Seven Keys | 1.745 ms | 1.766 ms | +1.25% | 4,387 → 4,381 |

Tower of Magic moves 6,477 invocations to compiled execution; Seven Keys moves
only six. Total invocation counts and runtime-guard counts are unchanged.
Seven Keys is slightly slower in both trial comparisons, so this change does
not establish a benefit there. The microbenchmark's 20.4% reduction must not
be generalized to all scripts or scenarios. Earlier exploratory measurements
made before the final diagnostic-classification cleanup are excluded from
these final-source comparisons.

## Changes

- Software object faces borrow clipped destination rows once per draw. Straight,
  transformed, and adjusted faces retain the existing sampling, modulation,
  gamma, and blending calculations. This removes per-pixel surface mutation
  bookkeeping and copy-on-write checks.
- Translucent HUD strips use the same row access. Regression coverage compares
  every alpha value against the original compositor, with and without independent
  RGB gamma ramps. GPU command capture continues to avoid allocating a software
  pixel plane.
- Value-only, non-forwarded method calls, including optional arrow calls, can
  execute in a compiled script plan. Reference-sensitive calls retain the AST
  path. Receiver dispatch uses the existing method implementation; suspension
  preserves the receiver and ten argument slots required by the C++ executor.
- Retained shader landscapes split sufficiently sparse changes into at most
  eight rectangles per plane. Uploads update only those cached spans, and
  composition draws the corresponding scissors. Dense edits keep a single
  upload. Material, atlas, extent, and output invalidation still force the
  appropriate full refresh. Draw-call statistics count the actual draws.

The sparse GPU regression changes two distant pixels in a 128×128 landscape.
It uploads two bytes and recomposes two texels, compared with the previous
15,876-byte, 15,876-texel covering rectangle. Incremental and fresh GPU readbacks
are identical. This is a work-volume measurement, not a GPU frame-time claim.

## Reproduction

Build both revisions with the same benchmark files, release configuration, and
content checkout. Complete compilation before timing; run one benchmark process
at a time on an otherwise idle machine.

```sh
cargo bench --locked -p clonk-frontend --features bench \
  --bench object_capture -- software_
cargo bench --locked -p clonk-script --features bench \
  --bench script_execution -- script_value_method_call
LC_GPU_TIMESTAMP_QUERIES=1 cargo bench --locked -p clonk-app-render --features bench \
  --bench landscape_render -- landscape_sparse
cargo nextest run --release --locked \
  -p clonk-engine-integration-tests --test engine_it \
  --features execution-profile,engine-it-shard-3 \
  --run-ignored all --no-capture --test-threads 1 \
  -E 'test(ast_execution_materiality_on_shipped_content)'
```

The software fixture renders 1,000 mixed straight/transformed ST5B faces at
800×600. The HUD fixture blends a 1280×16 translucent strip with standard gamma.
The script microbenchmark calls a value-only method returning an integer.
These isolate the changed paths; they do not measure complete game frame rates.

The real-content script probe uses seed zero, 100 warmup frames and 400 measured
frames, with five fresh runs each for Tower of Magic and Seven Keys. Its timing
instrumentation is optional and absent from ordinary builds. Interpret its
results using the limitations in [AST execution materiality](script-execution-materiality.md).

For the component comparisons, build and preserve each executable first, then
invoke it with `--bench <filter> --save-baseline before_1` (and `after_1`,
`after_2`, `before_2`) under a shared `CRITERION_HOME`. Divide each Criterion
sample's `times` by its `iters`, pool the two trials per variant, and take the
median. Retain the original estimates as well; the median reported here is
different from Criterion's regression-slope estimate. Renderer fixtures use
20 samples, two seconds of warmup, and five seconds of measurement per trial;
the method fixture uses 100 samples and three seconds of warmup.

Build the GPU baseline with the new benchmark fixture and the original
renderer. The GPU candidate differs only in `gpu_renderer.rs`; it does not
need the software-rendering or VM changes. Both CPU benchmark variants also
use the same new fixture. The scenario probe already exists at the baseline.
