# Developer-feedback and runtime performance

Performance data has two different purposes in this port:

- developer feedback measures how long a change takes to compile and validate;
- runtime performance measures scenario load, simulation, and software render.

Do not combine them into one number. A warm Cargo cache can improve feedback
without changing the game, while an engine optimization can improve simulation
without changing compile time.

## Measurement artifacts

`cargo dev-check` writes each run below
`target/dev-check/<run-id>/`. Keep the whole directory so a result retains its
selected commands, outcomes, timings, and replay evidence:

| Artifact | Contents |
| --- | --- |
| `manifest.json` | Changed paths, selected plan, and reproduction commands. |
| `timings.json` | Command durations plus imported load/simulation/render metrics. |
| `summary.json` | Overall pass/fail, budget state, and slowest phase. |
| `commands/*.stdout.log`, `*.stderr.log` | Captured output for every command. |
| `snapshot-final.json` | Final replay snapshot, when a replay ran. |
| `replay-metrics.json` | Replay load/simulation metrics, when available. |
| `frame-final.png`, `render-metrics.json` | Deterministic render and its samples. |
| `cpp-rust-diff.json` | Optional C++/Rust differential result. |

Treat the command duration, status, budget state, and slowest phase in the
machine-readable reports as distinct fields. Each command records
`cargo_build_ms` parsed from Cargo's own completion line and
`execution_after_build_ms` as the remaining wall time. `summary.json` records
the total, total reported Cargo build time, and the slowest command. A command
that passes after the budget has expired is still a pass, but the summary must
preserve that the feedback target was missed.

The ignored frontend render probe writes valid JSON with these fields:

| Field | Meaning |
| --- | --- |
| `schema_version` | Metrics format version. |
| `snapshot` | Exact `SimulationSnapshot` JSON used as input. |
| `width`, `height` | Fixed output dimensions; currently 320x180. |
| `cold_render_ns` | First `GraphicsSystem::render_frame` call. |
| `cold_checksum` | FNV surface checksum after the cold call. |
| `cached_render_ns` | Ten repeated-render samples after one settling pass. |
| `cached_checksum` | Required checksum for every cached sample. |
| `cached_samples` | Number of cached samples in the report. |

The paired `frame-final.png` is the final cached surface. A checksum mismatch is
a correctness failure, not a timing regression.

## Comparable runs

Only compare measurements whose fingerprints match:

- commit and content-submodule revision;
- Rust version and `Cargo.lock`;
- operating system, architecture, CPU, and runner class;
- Cargo profile and incremental/cache state;
- selected scenario/replay, seed, frame count, and render dimensions.

Separate cold, warm, and incremental compiles. A cold build must use a fresh
`CARGO_TARGET_DIR`; do not destroy the normal developer cache to measure it.
Network fetch time is a separate metric from compilation.

For runtime work, report phases independently:

1. load: group I/O, scenario/definition/script parsing, apply/init, and asset
   preparation;
2. simulation: fixed-seed warmup plus measured engine ticks, including the
   snapshot creation used by the app;
3. render: first frame, unchanged cached frames, changed-landscape frame, and
   output copy/upscale;
4. end-to-end frame: simulation plus render, used to assess the in-game frame
   deadline.

Use medians for compile/load and p50/p95/p99 for repeated simulation/render
samples. Always retain sample counts and raw samples; a single mean hides stalls.

## Baselines and regression gates

There is no portable compile or runtime baseline. Timings from an arbitrary
laptop or shared hosted runner are evidence about that fingerprint, not a
threshold for every machine. Keep cold, warm, and incremental workloads
separate and never compare results whose fingerprints differ.

Before making a timing blocking, collect at least 20 successful default-branch
runs with an unchanged runner class, toolchain, workload, content revision,
and cache classification. Reset collection after any fingerprint change and
retain the raw samples and machine-readable reports with the proposed
baseline.

Dated optimization journals and result narratives belong in benchmark
artifacts, pull requests, and git history, not in this live methodology.

For a proposed regression gate:

- warn when a comparable median rises by more than 10%;
- block only when it rises by more than 15% and also exceeds an absolute noise
  floor selected from the collected samples;
- require an explicit, reviewed baseline update for a deliberate tradeoff;
- keep shared-runner microbenchmarks informational unless their observed noise
  supports enforcement.

This dual relative/absolute rule prevents a tiny metric from failing because
of sub-millisecond scheduler noise.

## Provisional targets

These are planning targets, not measured baselines:

| Metric | Provisional target | Enforcement |
| --- | --- | --- |
| Focused `dev-check` work | Start no new ordinary check after its 60s budget | Enforced by the developer-feedback command; a required render diagnostic may still run. |
| Cached full Rust test execution | Local reference below 60s; CI p50 below 2 minutes | Report until 20 comparable default-branch runs exist. |
| In-game end-to-end frame | p99 below 25ms on a stable performance runner | Report on hosted runners; enforce only on stable hardware. |
| Hard in-game cadence | Never budget above 28ms | Architectural limit from `INGAME_FRAME_INTERVAL`, not a measured baseline. |

The 25ms planning target leaves 3ms of headroom beneath the 28ms in-game
cadence. Do not split it into arbitrary simulation/render limits before phase
measurements show where time is spent.

### Arso-Morf 1,000-Stippel simulation profile

Run the checked-in Arso-Morf save with exactly 1,000 real-content ST5B objects:

```sh
LC_PROFILE_MODE=split \
  cargo run --release --offline --locked -p clonk-engine \
    --example scenario_profile -- \
    "EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s" 600 424242 1000
```

The saved object section contains 20 Stippels and 1,041 non-Stippel objects;
scenario initialization creates two more, so the applied scenario reports 20
ST5B and 1,043 other objects. The profiler then joins both players before
population so all `InitializePlayer` crew/content creation is also outside the
measured frames (the smoke-run timed census has 1,056 non-ST5B objects). Setup
creates the remaining fresh ST5B objects through the loaded definition and its
normal `Initialize` callback, then verifies an exact 1,000-object ST5B census
before timing begins. The added objects use 49 deterministic, unique horizontal
offsets around each of the 20 saved-object anchors; fresh-construction vertical
movement is compensated so their centers remain on the same occupied ground
bands. They retain the real `LifeCycle` effect and receive only initial stuck
grace. Content may therefore remove a genuinely stuck Stippel later: the timed
census is exact, while the separately reported final census makes natural
attrition visible. Split mode reports simulation advance and
`SimulationSnapshot` projection samples independently as well as the combined
frame distribution. The output also includes the seed and resolved content
paths.

This is a reproducible measurement path, not a portable wall-clock assertion.
Compare release runs only with matching commit/content/toolchain/machine
fingerprints, and retain the raw output for both the parent and candidate.

### Arso-Morf 1,000-Stippel network presentation acceptance harness

Build both release executables, then run the same population through the real
windowed app, simulation, single-host network, viewport, and GPU submission
paths:

```sh
cargo build --release --offline --locked \
  -p clonk-app --bin clonk-app \
  -p clonk-engine --example arso_morf_stippel_fixture
scripts/run_arso_morf_stippel_gpu_benchmark.py 20
```

The runner copies checked-in Arso-Morf below the platform temporary directory.
The fixture builder refuses an unmarked destination or any path below installed
content, creates the other 980 objects through the real ST5B `Initialize`
callback, verifies `LifeCycle` on all 1,000, and serializes the disposable save.
It uses the same deterministic distribution around the 20 saved anchors as the
headless profile and verifies that the source `Objects.txt` hash did not change.

The app starts immediately as a real `/network /nosignup` host with one embedded
player. A private config uses separately probed TCP, UDP, and reference ports
and disables discovery, master-server signup, and UPnP. This is a component
acceptance harness for one playing network host, not a multi-peer transport or
network-throughput benchmark.

The run fails closed unless all of these conditions hold:

- fixture preparation and serialization each contain exactly 1,000 real ST5B
  objects;
- exactly one network-inspection line reports `inspection_status=ok` and host
  `local_client_id=0`, while runtime player infos match the authentic scenario
  player count, at least one player has one live SF5B crew, and no non-host
  client is activated;
- the active ST5B census is at least 990 at both measurement edges. Preparation
  and serialization are still exactly 1,000, but real `LifeCycle` can naturally
  remove stuck creatures during the two-second warmup and measured window. Both
  exact runtime counts remain in the result; losing more than 1% rejects a
  collapsed workload;
- simulation frames, refreshed frames, and successful presentation submissions
  each reach `floor(elapsed / 28ms)`, average graphics-pass time is at most
  28ms, and automatic graphics skips remain zero.

The two-second warmup precedes the requested measurement window, and a process
timeout also fails closed. Run from an interactive visible desktop on otherwise
idle hardware. A background automation session that supplies no refreshed GPU
frames is intentionally rejected rather than reported as a zero-FPS benchmark.
For an A/B comparison, reuse identical serialized fixture bytes, config, seed,
window size, and hardware, and add only the measurement instrumentation to the
baseline app.

Four deterministic microbenchmarks isolate object and particle capture from
retained-renderer submission:

```sh
cargo bench -p clonk-frontend --features bench --bench object_capture --locked
cargo bench -p clonk-frontend --features bench --bench particle_capture --locked
cargo bench -p clonk-app-render --features bench --bench object_sprite_render --locked
cargo bench -p clonk-app-render --features bench --bench particle_render --locked
```

The two object suites retain the original 1,000 ST5B-shaped faces (15x15, 20
phases, alternating transforms and sampling, unique modulation) and add 1,000
owner-colored crew faces. The owner capture fixture uses HZCK's 16x20 face and
256x420 sheet dimensions, 15 deterministic phases, distinct live object colors
and a full-RGBA owner overlay. Its four-pixel source offset makes phases 3, 7
and 11 cross a 64-pixel fog boundary. Both fixtures use explicit reverse
painter order.

`object_capture` runs each fixture unfogged and fogged. Its scoped counting
allocator compares one object with 1,000 after warming the same retained
ordering maps, sets, phase lists, texture resources and capture storage.
Equality of both allocation calls and bytes is the gate, so allocations may
remain per frame/resource run but cannot scale with represented objects or
fog-expanded chunks. The scope starts after `ObjectSnapshot` and render-order
construction: engine snapshot cloning intentionally remains outside this
presentation-only claim. The raw owner line reports the mask form, one/1,000
allocation calls and bytes, compact instance/upload bytes, captured commands,
generic-quad count, and fog-expanded instances. The unfogged structural gate
is exactly 2,000 base/owner instances in
`base1, owner1, base2, owner2, ...` order, zero generic quads and 176,000 bytes,
which is below the 176 KiB acceptance ceiling.

`object_sprite_render` submits the same ordered single-layer source once through
the compact path and once as generic quads, then repeats the comparison with an
ordered base/owner texture pair. It waits for each GPU completion and reports
all four `GpuRendererStats` records. The original structural gates remain an
88-byte object instance, one compact resource-run scene draw and 88,000 upload
bytes, versus 1,000 generic scene draws and 232,000 generic upload bytes. The
owner-pair gates require one scene draw containing 2,000 ordered 88-byte compact
instances and 176,000 upload bytes, versus 2,000 alternating base/owner generic
draws and 464,000 generic upload bytes. Including the fixed final presentation
pass, the corresponding totals are 2 and 2,001. These are representation
counts, not a substitute for the visible real-content presentation cadence
below.

Run the structural and allocation gates once without collecting Criterion
timing samples by appending `-- --test` to either object benchmark command.

The object renderer has `compact_1000_st5b_amortized` and
`compact_1000_owner_pairs_amortized` throughput cases. Each performs the same
retained build, upload, encoding, submission and statistics work for 16 frames,
but waits for device completion only after the batch. The particle renderer's
`2000_fire_and_fire2_amortized` case uses the same cadence for 1,000 normal and
1,000 additive particle sprites. These are steady submission-throughput
measurements, not GPU-only timing or single-frame latency; keep the original
completion-per-frame cases when diagnosing latency.

To measure the default, timestamp-disabled instrumentation overhead, build
separate retained baseline and candidate executables and explicitly remove
`LC_GPU_TIMESTAMP_QUERIES`. Force wgpu's opt-in no-op adapter with
`WGPU_NOOP_BACKEND=1 WGPU_BACKEND=noop`; this runs the same host validation,
packing, encoding, submission and statistics path without conflating desktop
GPU-completion load with the instrumentation cost. It is not rendering-
throughput or GPU-performance evidence. Every renderer-benchmark log must name
the selected adapter backend as `Noop`. Use these exact selectors:

- `object_capture/unfogged_1000_st5b`;
- `particle_capture/2000_fire_and_fire2`;
- `object_sprite_render/compact_1000_st5b_amortized`;
- `particle_render/2000_fire_and_fire2_amortized`.

Run three baseline-candidate-candidate-baseline quartets followed by three
candidate-baseline-baseline-candidate quartets for each case on the same idle,
AC-powered machine. Preserve every raw Criterion estimate and load/thermal log.
Reject a whole quartet if `max/min` exceeds 1.05 for either arm's two
`median.point_estimate` values, any run's Criterion
`std_dev.point_estimate / mean.point_estimate` exceeds 0.10, AC/thermal state
changes, or external load exceeds the predeclared limit. Collect exactly six
valid quartets per case without stopping early.

For each valid quartet compute
`d = (ln(C1) + ln(C2) - ln(B1) - ln(B2)) / 2`, using those four median point
estimates. From its six `d` values, compute the one-sided 95% Student-t upper bound
`exp(mean(d) + t(0.95, 5) * sample_stddev(d) / sqrt(6)) - 1`. Treat the overhead
as negligible only when that bound is below 2%; report the absolute duration as
well as the percentage. Apply any benchmark-only harness change identically to
both source trees and record its patch hash with the artifacts.

The runner can enforce and retain that paired comparison while leaving the
single-binary invocation above unchanged. Build an instrumented `origin/main`
app in a separate worktree, then pass both exact executables and a new artifact
directory:

```sh
scripts/run_arso_morf_stippel_gpu_benchmark.py 20 \
  --baseline-app-binary /path/to/origin-main/target/release/clonk-app \
  --baseline-source-root /path/to/origin-main \
  --candidate-app-binary target/release/clonk-app \
  --paired-artifact-dir /path/to/new/st5b-ab-artifacts
```

`--baseline-source-root` may be omitted when the binary still lives below its
Git worktree and the runner can discover it. The candidate source root defaults
to the current workspace and can be overridden with `--candidate-source-root`.

Paired mode generates the fixture and one canonical config inside the artifact
directory, then runs the baseline followed by the candidate against the same
fixture and byte-identical per-arm config copies. (The app normally saves its
config at shutdown, so sharing one writable copy would contaminate the second
arm.) The runner hashes every fixture file and the canonical config after
generation, verifies that combined fingerprint before and after each arm, and
verifies each writable copy against it immediately before launch. Canonical or
fixture byte drift fails the run; post-run config hashes record normal app
writes. The artifact directory must not already exist and is never removed.

`manifest.json` records the source and content revisions and dirty-input hashes,
Cargo lock and runner inputs, fixture-builder and app binary sizes and hashes,
Rust/Cargo/Python versions, machine, display, power, window, ports, environment,
commands, exact census evidence, and the A/B metric summary. The generated
fixture, `config.ini`, input fingerprint, fixture-builder logs, and each arm's
writable config, verbatim `stdout.log`, `stderr.log`, and parsed `report.json`
remain beside it. The reports and manifest preserve the complete
`graphics_pass_samples_ns` arrays rather than only percentiles.

Both arms must produce a real nonzero presentation plus valid network, player,
and at-least-99%-retained ST5B evidence. A baseline budget miss is recorded and
does not prevent the candidate run—the old renderer is the control, not the
acceptance target. The candidate must still satisfy every native-cadence,
28-ms-average, zero-auto-skip, and process-result assertion. Paired mode must
therefore also run from an interactive visible desktop; it does not make a
headless automation session valid presentation evidence.

#### Retained-renderer stage evidence

The instrumented candidate emits one compact JSON object prefixed by
`LC_APP_RETAINED_GPU_PROFILE`. Paired mode requires that record from the
candidate but keeps it optional for an older baseline binary. It preserves the
raw record and its SHA-256 in the arm report instead of reducing it to a few
averages. The record is tied to the exact adapter, enabled device features,
surface and buffer formats/extents, renderer switches, frontend raster
switches, and presentation scale/crop. The compact line can exceed live-console
display limits; use the retained `stdout.log` and parsed arm report as the
artifact rather than copied console output.

Arso-Morf is the mixed-content procedure for this evidence: the ordinary real
scene contributes landscape, fog, compact objects, generic UI/text sprites,
solids and compositor passes in addition to the 1,000 ST5B workload. Inspect
the structural counters before comparing timings; a missing draw family means
the two runs did not profile the same effective content.

Each successful retained presentation has one raw CPU sample. Its named stages
are frame preparation, renderer validation, texture synchronization, stream
packing/upload, retained pass encoding plus command-buffer finalization,
drawable acquisition, the CPU call to
`Queue::submit`, and the CPU call to present the drawable. These intervals are
host wall time. In particular, queue submission and presentation are **not**
GPU execution time. The existing end-to-end graphics duration remains the
governor input; every profile records an unclassified residual or an overrun so
the named intervals reconcile exactly without moving that endpoint or hiding
overlap.

The same per-frame sample records structural work rather than inferring it from
time: every retained draw kind, fixed compositor pass, compatible-resource run,
vertex/instance stream count and upload byte count, source-texture write call
and byte count (including mip levels), composition recreation, expanded fog
chunk, and successful generic-sprite fallback reason. Fallback reasons are
non-exclusive, so their sum may exceed the fallback total.

Retained profile schema 2 adds the compact landscape-instance count and upload
bytes and requires `landscape_instance_upload_bytes == landscape_instances *
72` for every frame. The paired-run parser still accepts schema 1 from a legacy
baseline, where that stream did not exist, but the instrumented candidate emits
schema 2 and cannot omit either counter.

GPU pass timing is separately opt-in. The runner removes any ambient setting,
leaves the baseline uninstrumented, and launches the candidate with:

```sh
LC_GPU_TIMESTAMP_QUERIES=1
```

If the selected adapter does not advertise `TIMESTAMP_QUERY`, the device's
optional feature set remains empty, every CPU frame carries a null timestamp
ID, and the GPU frame list is empty. If it is supported, the renderer allocates
a bounded asynchronous query pool and identifies the actual shader-landscape,
scene, monitor-gamma, and presentation passes encoded for each frame. Normal
rendering only polls; it never waits for the GPU. The benchmark performs one
bounded drain after the measurement window, then requires an exact one-to-one
frame-ID correlation. Raw ticks, the queue's timestamp period, pass name, and
derived nanoseconds are all retained. Counter rollover and invalid periods or
durations remain visible as raw invalid samples and fail candidate validation,
as do missing/duplicate IDs, dropped frames, readback errors, device
discontinuities, nonzero telemetry, or a pass set that disagrees with the
structural counters.

The profile parser is deliberately fail closed for the candidate: CPU sample
count must equal retained submissions, each end-to-end duration must equal the
corresponding legacy `graphics_pass_samples_ns` entry, all integer fields reject
JSON booleans/floats, and CPU reconciliation must hold exactly. A legacy
baseline without the new prefix is still valid A/B input; a partial, duplicate,
or malformed candidate record is not.

### Fogged-landscape capture and renderer microbenchmark

Run production fog capture and the retained renderer at both 800x600 and
3840x2160:

```sh
cargo bench -p clonk-app-render --features bench \
  --bench landscape_render --locked
```

`landscape_capture/fogged_800x600_130_chunks` and
`landscape_capture/fogged_4k_2040_chunks` call the real `GraphicsSystem` fogged
landscape lowering. Each dedicated system retains its synthetic landscape cache,
fog map and recorder capacity across warmups, so the Criterion samples exclude
fixture construction. Before wgpu is initialized, a scoped counting allocator
warms separate one-chunk, 130-chunk and 2,040-chunk systems three times and
measures their fourth capture. The benchmark fails unless all three allocation
call counts are equal. Allocation bytes may scale with the output scene; the
gate is specifically that command count cannot create additional allocation
calls.

The two normal renderer arms consume those production-captured scenes. A
separate 4K `NoBoxFades` arm retains the two flat-shaded triangle commands per
chunk, for 4,080 ordered landscape commands. The compact record is 72 bytes.
The 800x600 arm therefore uploads 130 records (9,360 bytes), while the normal
4K arm uploads 2,040 records (146,880 bytes) and no generic vertices; the 4K
byte count must remain at or below 196 KiB. Every renderer arm requires one
scene draw and two total draws including the fixed presentation pass. The raw
evidence line reports source commands, instance and generic-stream bytes,
scene/total draws, and `cpu_stages.stream_packing_upload`. That last duration is
host packing/upload time, not GPU execution.

The Criterion `landscape_render/*` wall-time samples still combine retained
processing, command encoding, queue submission and a device-completion wait.
For a named GPU interval, opt in to clonk-org/clonk-rs#267's timestamp path:

```sh
LC_GPU_TIMESTAMP_QUERIES=1 cargo bench -p clonk-app-render --features bench \
  --bench landscape_render --locked
```

The ordinary Criterion device always requests an empty optional feature set.
When the adapter supports `TIMESTAMP_QUERY`, the preflight creates a separate
device requesting only that feature and records eight named `Scene` samples per
workload. It reports the median valid duration plus validity counts and fails if
all eight samples are invalid. Counter rollover and other invalid samples remain
visible rather than being converted into durations. When the request is disabled
or unsupported, no timestamp device is created, both effective optional feature
sets stay empty, and the raw duration is explicitly `unavailable`.

### Hazard 24-player owner-color A/B benchmark

The visible owner-color acceptance runner compares two already-built app
executables on the real Hazard `DM_Baldoon` scenario:

```sh
python3 scripts/run_hazard_24_player_gpu_benchmark.py \
  --baseline-binary /absolute/base/clonk-app \
  --candidate-binary /absolute/candidate/clonk-app \
  --baseline-source-root /absolute/base/source \
  --candidate-source-root "$PWD" \
  --clients 1
```

The default is 12 rendered clients; `--clients 1` assigns all 24 ordered player
profiles to one visible client to avoid desktop window-occlusion throttling.
Both layouts still require exactly 24 synchronized players, 24 players with
live crew, and 24 live crew objects.
The byte-hashed scenario contract pins `Crew=HZCK=1` for all four player
templates, so that runtime census establishes 24 live HZCK crew even though the
runtime report does not expose definition IDs.

The runner never builds either executable. It records each executable's bytes,
each source tree, Cargo.lock, harnesses, scenario/runtime data, settings, and
the normalized retained-GPU adapter/config fingerprint independently. It
requires initial and final provenance to match and permits only the executable
identity to differ between the paired input fingerprints. This is exact
artifact and source provenance, but it is not a cryptographic attestation that
an arbitrary supplied executable was built from the accompanying source root.

Both arms request timestamp queries. Unsupported adapters remain a valid,
explicit `unavailable` result; supported arms must have the same normalized
GPU fingerprint. Hazard opts into the retained-profile validator's raw Metal
timestamp policy: dropped frames and device discontinuities still fail, every
raw tick and disposition is validated and retained, and each rendered pass
must have at least one valid sample. Counter-rollover samples are excluded only
from timing distributions. The ordinary Arso-Morf validator remains strict.

### Deep Sea retained-GPU presentation benchmark

Build once, then run the real Deep Sea scenario through the windowed GPU path:

```sh
# Run from the repository root.
cargo build --release --offline --locked -p clonk-app --bin clonk-app
scripts/run-deep-sea-gpu-benchmark.sh 20
```

The wrapper creates and removes a self-contained fixture below the platform
temporary directory. On macOS and Linux, set `TMPDIR` to place that disposable
state somewhere else. It excludes only Hazard's three duplicate process-global
GUI sheets and removes the copied scenario's `Origin` redirect; all Deep Sea
world and gameplay resources remain the real checked-in content.

The app warms up for two seconds, measures for 20 seconds, prints one
`LC_APP_PRESENTATION_BENCHMARK` machine line, and exits. The assertion exits
with status 2 unless both refreshed frames and successful presentation
submissions reach `floor(elapsed / 28ms)`, average GPU graphics-pass time is at
most the native 28ms game tick, and `automatic_graphics_skips=0`. Record the
machine line together with the commit, content revision, display size, hardware,
OS, and power state; do not compare runs with different fingerprints.

### HarpoonRace 24-player process benchmark

The other opt-in end-to-end benchmark starts a real classic
`/network /lobby /console` host and a 24-client rendered fleet, then measures
every client against a per-player mesh and frame-cadence contract.
`scripts/HARPOONRACE_24_PLAYER_BENCHMARK.md` documents the route, the
acceptance gates, and the overrides; it needs an interactive desktop session on
otherwise idle hardware.

The transport-only companion has a repeated, fingerprint-checked runner in
`scripts/run_network_load_benchmark.py`. See
`scripts/NETWORK_LOAD_BENCHMARK.md` for its optimized build-once workflow,
provenance-bound binaries, raw artifacts, and baseline/candidate comparison.
Only a direct same-runner experiment with an exact, predeclared 20-pair,
randomized ten-AB/ten-BA schedule can establish its target; separately collected
cohorts are exploratory. The authoritative decision uses a distribution-free
paired-median interval and exact sign evidence, while bootstrap intervals are
descriptive. Its target metrics are control-completion wait and the microsecond
application ReadyCheck round trip from a fresh, warmed one-host/one-client
session created only after the loaded 24-player session shuts down cleanly.
The loaded 24-client application fanout and native integer-millisecond ping are
diagnostic; the latter's quantization cannot establish a 50% latency change
near zero. The runner recomputes every report percentile from raw samples and
checks measured counts, route topology, and both cleanup gates independently
of the harness's own overall result.

## Reproducing render measurements

Render one explicit replay snapshot:

```sh
LC_DEV_CHECK_SNAPSHOT=target/dev-check/path/to/snapshot-final.json \
LC_DEV_CHECK_FRAME_PNG=target/dev-check/repro/frame-final.png \
LC_DEV_CHECK_RENDER_METRICS=target/dev-check/repro/render-metrics.json \
  cargo nextest run -p clonk-frontend --features dev-feedback-render \
  --test dev_feedback_render -- dev_feedback_render --ignored --exact
```

Or let the probe select the newest `snapshot-final.json` recursively:

```sh
LC_DEV_CHECK_ARTIFACT_DIR=target/dev-check \
  cargo nextest run -p clonk-frontend --features dev-feedback-render \
  --test dev_feedback_render -- dev_feedback_render --ignored --exact
```

Compare JSON reports only after verifying their input paths/fingerprints and
checksums. The PNG is diagnostic evidence for visual inspection, not a timing
sample.

## CI cache interpretation

A cache hit is not proof that every crate was reusable. Report cache state with
the observed compile duration, and treat a cache-key, toolchain, manifest,
content, or runner change as a different compile workload. Merge-group jobs may
consume artifacts produced from trusted default-branch trees but must not
publish reusable caches from an untrusted candidate.

The workflows are the source of truth for current cache names, producers,
consumers, and concurrency. After a cache-key change lands, use the documented
`cache_only` dispatch in `docs/DEVELOPING.md`, then confirm from the workflow
report that every required producer and consumer used the intended exact key.
