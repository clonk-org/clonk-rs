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

## Loop-control lowering (clonk-org/clonk-rs#1565)

`break` and `continue` now lower into the compiled plan's `while` and
classic-`for` forms. The probe above was re-run against the change and against
its parent commit back to back in one session, so only
`crates/clonk-script/src/vm.rs` differs between the two sets. Do not compare
against the numbers in the previous section: they predate later landed
performance work, and this session measured the parent commit at
2.0719 ms/frame where that section recorded 2.1779.

| Scenario | Newly compiled invocations / 400 frames | AST invocations | Median AST ms/frame | Median tick ms/frame (timed) | Median tick ms/frame (untimed) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Tower of Magic | +117 | 23,504 → 23,387 (−0.50%) | 2.0719 → 2.0475 (−1.18%) | 4.3703 → 4.3204 (−1.14%) | 4.3068 → 4.3184 (+0.27%) |
| Seven Keys | +57 | 4,444 → 4,387 (−1.28%) | 0.4556 → 0.4419 (−3.00%) | 1.3145 → 1.2922 (−1.69%) | 1.3334 → 1.2861 (−3.55%) |

Total invocation counts are unchanged in both scenarios, so the moved
invocations are the same work on a different executor. Coverage grew by less
than the 7,563 and 183 loop-control blocker counts suggested, because those
counts overlap: most of those functions have another blocker that still keeps
them interpreted. The elapsed differences span −3.55% to +0.27%, inside the
noise band this document already recorded for identical binaries (−1.99% and
−0.72%), so no elapsed benefit is resolvable at this sample size.

Every window is in
[`benchmarks/results/loop-control-lowering.json`](../benchmarks/results/loop-control-lowering.json).
The remaining mass in Tower of Magic is `special_or_forwarded_call` (12,750)
and `method_or_optional_call` (10,765); measure a family that large before
lowering another one.
