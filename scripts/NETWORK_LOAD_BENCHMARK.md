# HarpoonRace-shaped network-load benchmark

This runner turns the opt-in 24-player socket harness into a repeated benchmark
suite. It preserves every process report and verifies the workload, binary,
build inputs, and statistics before comparing a baseline with a candidate.

There are two distinct modes:

- a direct, randomized paired comparison of two provenance-bound prebuilt
  binaries is the only design that can establish the 50% target;
- a single `run`, or a later comparison of separately collected cohort
  directories, is exploratory and always reports `statistically_valid=false`
  and `meets_target=null`.

Run measurements on otherwise idle hardware with a stable power policy. The
default workload is full-mesh reliable UDP with a 36-control-tick warmup and a
60-second measurement window.

## Prepare provenance-bound binaries

New builds use `cargo test --locked --profile release --no-run` and discover the
exact integration-test executable from Cargo's JSON output. The runner writes
an adjacent `<executable>.network-load-provenance.json` sidecar before using the
binary as prebuilt evidence.

Prepare each arm in a clean checkout using the same version of this runner and
the same benchmark harness. A one-run, one-second smoke cohort is a convenient
way to build the executable and sidecar without collecting an exploratory
20-minute cohort:

```sh
# Run once in the clean baseline checkout and once in the clean candidate
# checkout. The smoke report is not target evidence.
python3 scripts/run_network_load_benchmark.py run \
  --runs 1 \
  --measurement-seconds 1 \
  --label provenance-smoke \
  --output target/network-load-provenance-smoke
```

`cohort-metadata.json` records the original paths as `build.binary.path` and
`build.provenance_sidecar`. Copy both files together if the Cargo target will
not remain available. The copied sidecar name must be formed from the copied
binary name:

```text
baseline-integration-test
baseline-integration-test.network-load-provenance.json
candidate-integration-test
candidate-integration-test.network-load-provenance.json
```

The sidecar, not the harness's runtime Git probe, is authoritative for build
identity. It binds the executable hash and size to:

- the source commit/tree plus tracked-patch and untracked-input hashes;
- the content checkout commit/tree/gitlink and its dirty-state hashes;
- `Cargo.lock`, all manifests, Rust/Cargo configuration from every effective
  origin, and the exact `tests/network_load_24.rs` contract;
- `rustc -Vv`, Cargo version, target, wrappers, compiler/linker variables, and
  every present `CARGO_PROFILE_*`, `CARGO_BUILD_*`, and `CARGO_TARGET_*` value;
- the selected profile, canonical workspace profile tables, and Cargo's exact
  compiler-artifact profile;
- the SHA-256 of the exact runner script that created the sidecar.

An explicit `--cargo-profile` must match both sidecars. A profile name alone is
not provenance. The runner rejects different effective profile settings,
Cargo.lock files, manifests, toolchains, target/linker settings, Cargo
configuration, content, or benchmark-contract hashes.

Source revisions may differ because that is the code under test. For an exact
20-pair authoritative comparison, both source and content must be clean. The
current sidecar records dirty-input hashes but does not archive the corresponding
patch and untracked bytes, so dirty builds are deliberately ineligible for an
authoritative claim. Content must also be recorded by the parent tree as a real
`160000 commit` gitlink, and its checked-out HEAD must equal that gitlink.

The harness's `fingerprint.cargo_profile` is a diagnostic label, not Cargo's
selected profile name. The runner derives its required value from Cargo's
authoritative compiler-artifact `debug_assertions`: `test` when false and
`test-with-debug-assertions` when true.

Copied binaries remain usable after their build checkout is moved or deleted.
In that case report fields such as runtime `source_commit`, `content_revision`,
or `rustc` may be null or stale; they remain diagnostics and do not override the
binary-bound sidecar.

## Authoritative paired comparison

Pass the two exact binaries to one direct `compare` invocation. State the
pre-specified 20-pair count explicitly and omit `--measurement-seconds` for the
default 60-second workload:

```sh
# This runs 40 processes and takes at least 40 minutes plus setup.
python3 scripts/run_network_load_benchmark.py compare \
  /absolute/path/to/baseline-integration-test \
  /absolute/path/to/candidate-integration-test \
  --runs 20 \
  --topology udp \
  --cargo-profile release \
  --output /absolute/path/to/network-load/paired-comparison
```

Before the first process starts, the runner creates
`experiment-manifest.json` with a random experiment ID and recorded 128-bit
seed. The manifest predeclares exactly 20 pairs, randomly shuffles an exactly
balanced ten-AB/ten-BA order, and expands it into a 40-process global sequence.
Every cohort, summary, and execution is bound to the manifest hash and
experiment ID. Each execution records its pair index, AB/BA order, position,
and global sequence number. Retained comparison reconstructs the schedule from
the seed and rejects a forged schedule string, unrelated cohorts, path escapes,
symlinked or duplicate run directories, and inconsistent bindings. The
manifest records the runner-script SHA-256; both binary sidecars, both cohorts,
the manifest, and the executing runner must name the same exact script bytes.

One runner process executes the complete interleaved schedule, which is the
basis for same-host authority. The runtime-machine fingerprint and captured
CPU affinity, load, logical resource, and available power observations are
descriptive context; matching descriptive strings alone cannot turn separately
collected cohorts into a paired experiment.

Each cohort keeps its own copied executable and embedded build provenance. The
runner hashes the executable before and after every process and again at cohort
finalization. A changed, missing, symlinked, or unverifiable retained binary
invalidates the evidence.

## Exploratory cohorts

Use `run` for smoke tests, diagnostics, or exploratory distributions:

```sh
python3 scripts/run_network_load_benchmark.py run \
  --label candidate \
  --output target/network-load-benchmark/candidate
```

`--label` is restricted to one 1-64 character ASCII filename component that
starts with a letter or digit. The runner rejects separators, `..`, absolute
paths, and any resolved artifact path outside its newly created cohort.

For shorter orchestration checks, pass `--runs` and
`--measurement-seconds`. Durations below 60 seconds carry
`authoritative_duration=false`. Durations of 60 seconds or more satisfy the
harness duration gate, but comparisons still apply only to the exact requested
duration and topology shared by both arms.

Two retained cohort directories may also be compared without rerunning them:

```sh
python3 scripts/run_network_load_benchmark.py compare \
  /absolute/path/to/network-load/baseline \
  /absolute/path/to/network-load/candidate \
  --runs 20 \
  --topology udp \
  --cargo-profile release \
  --output /absolute/path/to/network-load/exploratory-comparison
```

Repeat any non-default `--runs`, `--measurement-seconds`,
`--timeout-seconds`, `--topology`, and `--cargo-profile` values used to collect
the cohorts. These directory comparisons are useful diagnostics, but temporal
drift and collection order are uncontrolled; they cannot set
`statistically_valid` or establish the target.

## Statistical contract

One complete test process is one independent observation. For each metric, the
runner recomputes that process's p50 from raw samples. It reports all per-run
p50 values, their median and median absolute deviation, descriptive pooled
percentiles, and a deterministic bootstrap interval. Pooled samples share a
process, scheduler, and protocol history and are never substituted for the
independent-run count.

The authoritative paired statistic is the median of the 20 within-pair ratios:

```text
candidate per-run p50 / baseline per-run p50
```

Its decision interval is the exact, distribution-free binomial order-statistic
interval for the paired-ratio median. With 20 pairs, the sixth and fifteenth
ordered ratios form a 95.861% interval. The runner also records the exact
one-sided sign-test tail probability and requires at least 15 of 20 ratios to
be at or below `0.5`. Bootstrap intervals remain descriptive and do not decide
the target.

The two target metrics are:

- `control_completion_wait`, in microseconds;
- `client_to_host_isolated_application_round_trip`, in microseconds. Only after
  the loaded 24-player session shuts down cleanly, the harness creates a fresh
  same-topology one-host/one-client session, completes its join/status
  handshake, performs 128 warmup exchanges, and measures 256 sequential
  two-message exchanges. Each exchange sends host
  `ReadyCheck(Other(index+2))`, receives client
  `ActivationRequest(index+2)`, and stops at the matching host receipt.

For each target metric:

- `met` requires the exact interval's upper bound to be at most `0.5` and at
  least 15 successful pairs;
- `not-met` requires the interval's lower bound to be greater than `0.5`;
- an interval crossing `0.5` is `indeterminate`, even when the point estimate
  is worse than `0.5`.

The overall target is met only when both target metrics are met. A zero
baseline makes the corresponding ratio and target result indeterminate.

The loaded `client_to_host_application_round_trip` and its 24 per-client series
remain diagnostics. They measure eight sequential ReadyCheck fanout round trips
per client while the 24-player session is loaded, so unrelated fanout and
scheduler costs make them unsuitable for the isolated ping target. Native
`client_to_host_round_trip` remains a compatibility diagnostic in whole
milliseconds; quantization, especially a zero baseline median, also makes it
incapable of proving a 50% change. Both diagnostic metrics are excluded from
`meets_target`.

## Validation and artifacts

Runner artifacts use contract schema 5; the embedded harness reports use schema
6. The runner independently checks the exact workload strings, 24 joined
players, 25 control participants, selected route topology, 36 warmup ticks,
requested duration and wall-time tolerance, measured tick and ready-delivery
counts, all required harness assertions, and the exact diagnostic 24-by-8
loaded application-RTT shape. It also requires the exact isolated sequence, 128
warmup count, 256 measured microsecond samples, client ID 1, the two directed
host/client preferred-message routes, and separate passing
`loaded-session-clean-shutdown` and `isolated-ping-clean-shutdown` assertions.
Runtime telemetry must consist of complete, ordered host-plus-24-client groups
with valid nonnegative telemetry. Runtime `route_count` is diagnostic because
it can include data routes; the final preferred-message route map supplies the
exact topology evidence. Each of those 25 process samples contributes 25
native-control state waits, so the runner requires exactly
`25 * len(runtime_samples)` native-control-wait values. It recomputes every
MetricSeries summary from raw samples and requires aggregate native and loaded
application RTT samples to be the exact client-order concatenations.

Every cohort contains:

- `cohort-metadata.json`, including configuration, runtime context, binary,
  provenance, and experiment binding;
- the copied benchmark executable and `build-provenance.json`;
- `run-NNN/report.json`, `execution.json`, `stdout.log`, and `stderr.log`;
- `benchmark-summary.json`, with per-run and descriptive pooled results.

A failed or timed-out process is retained and does not stop later runs. An
exit-zero process with no report is a failure. Exact requested report counts
are mandatory, and no failed report is replaced with a synthetic sample.
Retained comparison validates schemas, counts, unique contained paths,
report/execution hashes, provenance hashes, experiment bindings, and pre/post
binary hashes, then recomputes the statistics from admitted raw reports.

Comparison exit statuses are:

- `0`: valid authoritative comparison and both targets met;
- `1`: valid comparison, but the target is unmet or indeterminate, including
  all exploratory cohort comparisons;
- `2`: malformed, failed, incomparable, or otherwise invalid evidence.

The loaded Rust session fixes warmup at 36 control ticks, about two seconds.
That may not exhaust reliable UDP's 128-fragment adaptation. The fresh isolated
session separately warms exactly 128 request/response exchanges before its 256
measurements. Results establish only the recorded loopback workloads under
those warmups; the runner does not silently change the harness's semantics.
