# AST execution materiality

Investigation of clonk-org/clonk-rs#1546.

## Decision rule (recorded before timing)

Treat AST execution as material if its median exclusive elapsed time is at
least 0.278 ms/frame (1% of the 36 Hz frame budget) and at least 5% of elapsed
tick time in either Tower of Magic or Seven Keys. Measure five fresh,
seed-zero runs per scenario, warming up 100 frames and measuring 400 frames.
Exclude snapshots and scenario loading. Report every run, not just the best.

Time nested script execution exclusively so recursive and mixed compiled/AST
calls are not double counted. Native host work inside a script execution
interval remains included: this is an upper bound on interpreter overhead,
not an estimate of the speedup available from lowering. Compare clock-enabled
and clock-disabled elapsed tick times in the same feature-enabled binary to
assess timer overhead. Invocation counters remain enabled in both; this does
not measure their overhead relative to a shipped binary.

The intervals cover initial AST setup (including the function-body clone),
compiled binding/guard checks, direct execution, and resumed execution after
host callbacks. Function argument/environment setup outside those intervals
is excluded, or charged to an enclosing execution interval when nested.
Nested intervals of the same kind are also exclusive, so their sum includes
setup without counting execution twice. All timing is thread-local and off
by default; ordinary builds contain no VM clock reads.

If the threshold is met, open a narrowly scoped follow-up for a blocker family;
do not change C4Script semantics as part of the measurement. Otherwise retain
the negative result without expanding lowering.

## Reproduction

Use the pinned content checkout and a disk-backed target directory. Run the
opt-in probe with the shipped release profile and one test thread:

```sh
flock -w 7200 /tmp/clonk-rs-luna-cargo.lock \
  nice -n 15 ionice -c3 timeout 3600 \
  env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=target \
  cargo nextest run --release --locked \
  -p clonk-engine-integration-tests --test engine_it \
  --features execution-profile,engine-it-shard-3 \
  --run-ignored all --no-capture --test-threads 1 \
  -E 'test(ast_execution_materiality_on_shipped_content)'
```

Retain the raw output with the source revision, content revision, Rust version,
CPU, OS, and the instrumentation diff. Each output row contains whole-window
nanoseconds; divide by 400 to obtain per-frame means. Compute medians across
the five runs. Clock-enabled/disabled pairs alternate order and must retain
identical invocation counts. Run failures are errors, not timing samples.

The initial machine-readable measurement, including every window and source
fingerprints, is in
[`benchmarks/results/script-execution-materiality.json`](../benchmarks/results/script-execution-materiality.json).
Both scenarios exceeded the decision threshold. The next bounded experiment
is loop-control lowering, tracked in clonk-org/clonk-rs#1565. The observed
timer-on tick differences were slightly negative; treat those as noise rather
than evidence that instrumentation makes execution faster.
