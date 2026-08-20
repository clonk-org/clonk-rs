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

## Baseline collection

There is no portable compile or runtime CI baseline yet. Do not turn timings
from an arbitrary laptop or a shared hosted runner into a blocking threshold.
The merge-queue service baseline below is deliberately scoped to comparable
hosted `Landing` runs rather than presented as a portable machine benchmark.

### Release codegen parallelism

On 2026-07-31, commit `b1d71339c` and content revision `e82d6d275` were built
on the Apple M4 Max reference machine with Rust 1.97.1. Thin LTO stayed enabled
while release codegen units varied. Each shipped-binary build used a fresh
target, the locked offline graph, and the real release inventory:

```sh
cgu=8
cold_target="$(mktemp -d "${TMPDIR:-/tmp}/clonk-release-cgu.XXXXXX")"
caffeinate -dimsu /usr/bin/time -lp env \
  -u CARGO_INCREMENTAL \
  -u CARGO_BUILD_JOBS \
  -u RUSTC_WRAPPER \
  -u RUSTC_WORKSPACE_WRAPPER \
  CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$cgu" \
  CARGO_TARGET_DIR="$cold_target" \
  cargo build --release -p clonk-app -p clonk-game -p clonk-c4group \
    --locked --offline --timings --quiet
```

| Release codegen units | Cold wall | Build process-tree user CPU | System CPU | Target size | Three binaries |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 208.49s | 590.58s | 20.37s | 1,080,324 KiB | 43,491,408 bytes |
| **8** | **72.38s** | 658.72s | 26.34s | 1,165,852 KiB | 54,104,384 bytes |
| 16 | 75.29s | 694.20s | 28.21s | 1,182,436 KiB | 55,945,920 bytes |
| 64 | 75.97s | 727.35s | 35.61s | 1,206,852 KiB | 57,716,304 bytes |

The runtime control was the fixed-seed, 6,000-frame `03_Chaos` workload named
below as the low-power regression scenario. Each arm reused its corresponding
release target, and the three primary arms ran in balanced orders
`1/8/16`, `16/8/1`, and `8/1/16`:

```sh
CARGO_TARGET_DIR="$cold_target" \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$cgu" \
  cargo build --release -p clonk-engine --example scenario_profile \
    --locked --offline --quiet
/usr/bin/time -lp env LC_PROFILE_MODE=tick \
  "$cold_target/release/examples/scenario_profile" \
  "ClonkMars.c4f/03_Chaos.c4s" 6000 424242
```

| Release codegen units | Samples | Simulation wall median | Mean/frame | p50 | p95 | p99 | Mean delta |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 3 | 12.054256s | 2.009012ms | 1.738625ms | 3.188125ms | 5.491166ms | baseline |
| **8** | 3 | 12.285615s | 2.047573ms | 1.762416ms | 3.279708ms | 5.649625ms | +1.9% |
| 16 | 3 | 12.390027s | 2.064979ms | 1.781958ms | 3.379583ms | 5.730750ms | +2.8% |

Two additional reversed codegen-unit-1/64 pairs put the 64-unit mean-frame
regression between 2.9% and 6.9%. Every runtime sample joined players `[0, 1]`,
ended with 122 objects after 6,000 frames, and recorded no frame above the
27.7ms native-tick budget. The profiler does not hash the final snapshot, so
these matching outputs are a workload sanity check rather than a parity gate.

Eight global codegen units were the initial selection because they reduced this
cold build by 65.3% while finishing faster, using less CPU, producing smaller
binaries, and regressing runtime less than 16 or 64. The final binaries were
24.4% larger than the one-unit build; that size and the 1.9% mean-frame cost
were the accepted tradeoff for the 136.11s local build reduction.

A follow-up on commit `1dd151cfd` modeled a warm-library build, with only the
final application rebuilt. The global profile returned to one codegen unit and
only `clonk-app` varied. Removing the app artifacts between each arm preserved
the same dependency artifacts and shipped `clonk-game`/`c4group` binaries:

| App codegen units | Warm app wall | Build user CPU | App binary |
| ---: | ---: | ---: | ---: |
| 1 | 106.96s | 261.58s | 40,235,120 bytes |
| **8** | **59.45s** | 264.36s | 43,185,232 bytes |
| 16 | 59.66s | 271.21s | 43,606,736 bytes |

A four-Cargo-job proxy for the hosted Windows runner kept eight ahead of four
(69.38s versus 77.61s) and sixteen (73.34s). Cargo rebuilt only `clonk-app`
when moving from global-one to app-eight, confirming that the narrow override
preserves the cached libraries. The shipped profile therefore keeps one unit
globally and grants eight only to the app. This retains the baseline codegen
for the simulation libraries while cutting the measured cache-warm final tail
by 44.4%. Because Cargo package overrides inherit into child profiles, the test
profile repeats its explicit 256-unit app setting instead of silently
inheriting eight.

On MSVC, rustc 1.97.1 requests a PDB even though the release profile has no
debug information, while the published archives do not contain that PDB. The
post-merge and release builds therefore share one configuration
script that selects rustc's bundled LLD and passes `/DEBUG:NONE`,
`/OPT:REF,ICF`, `/TIME`, and `/Brepro` explicitly. The same script enables
linker-plugin LTO and a bounded 512 MiB ThinLTO cache while retaining the static
CRT. It clears inherited `LINK` and `_LINK_` values before exporting the
fingerprinted Rust flags, so runner-image defaults cannot silently override the
contract. If release symbols are published in the future, remove
`/DEBUG:NONE` from both paths and ship the matching PDB rather than
silently producing an unused one.

Hosted run `30691633087` restored an 855 MiB Windows dependency cache and took
12m49s for the job: 11m50s in the runtime-build step and 11m10s reported by
Cargo. An offline registry miss accounted for another 38.8s. Linker `/TIME`
reported only 1.390s for the final `clonk-app.exe` link and 3.935s across every
native link in the build; the roughly 8m40s final application tail is rustc
frontend, codegen, and ThinLTO work rather than MSVC linking. A later hosted
toolchain probe accidentally changed `CARGO_HOME` from the producer's native
Windows path to an MSYS path, producing a cache identity that the restore-only
landing job could never seed. Windows smoke therefore uses the same pinned
toolchain action as the trusted post-merge cache producer.

Later native-MSVC measurements isolated the remaining warm path. Cold hosted
run `30698792424` used linker-plugin ThinLTO and finished in 16m48s;
Cargo reported 14m43s and the application LTO phase alone took 331.775s. It
published a 27.6 MiB LLD cache and a 496 MiB Rust dependency cache. Run
`30699399606` then measured the same revision twice. The first attempt's only
exact hit was the LLD cache; its dependency inventory differed, and it finished
in 11m13s. The second hit both exact caches and
finished in 8m19s: 7m11s in the build step and 6m31s reported by Cargo. Its
observable final application tail was 4m40.219s, while all three native links
together took 3.809s and their LTO work took 1.243s. The three executable
hashes matched both earlier ThinLTO builds.

The comparison arm disabled LTO only for these MSVC builds. Cold run
`30700375081` finished in 14m14s, with a 13m19s build step, 12m23s reported by
Cargo, and 5.480s across all links. An exact dependency-cache rerun was still
compiling after 10m11s and was stopped before the final application link. This
arm is rejected: disabling LTO made the relevant warm path slower while also
changing the shipped optimization contract. Normalizing the runner to the one
pinned Rust toolchain keeps future dependency-cache identities stable, and
trusted `main` alone publishes both dependency and ThinLTO caches. Release jobs
restore them without writing short-lived copies.

Production-plumbing run `30701547831` deliberately began with neither exact
cache and finished green in 12m55s. Cargo reported 11m32s; the application
rustc span was 8m08.050s and its cold LTO link consumed 4m03.251s. Validation
found a 119 MiB reusable ThinLTO cache, no dynamic CRT or PDB dependency, and
working executables. All three hashes matched the earlier ThinLTO builds. The
separate Windows smoke job pinned NSIS 3.12, compiled the stand-in installer,
and finished in 3m56s.

These samples put the standard four-vCPU Windows runner above the five-minute
landing target even with both caches exact. The remaining cost is application
frontend and code generation, not linking. The bounded queue therefore retains
Windows tests, path linting, and an NSIS installer smoke compile, while trusted
post-merge validation performs the exact static-CRT release build and runtime
inspection. Release remains fail-closed on the merge-group `Landing` run,
whose release path performs exact-SHA qualification before the commit may
land; post-merge validation is diagnostic only.

Two exhaustive standard-runner Linux samples of the predecessor graph, runs
`30693625838` and `30693995330`, passed every row, but shared-runner execution
varied enough that one and four rows respectively reached five minutes. The
latter sample ranged from 2m28s to 5m16s per row; its slow application commands
spent 3m34s--3m41s compiling and 28--39s executing tests. A controlled four-job
shard-6 probe kept the current app test profile: opt-level 1, opt-level 0, and
512 codegen units were each about 8% slower end to end than opt-level 2 with
256 units. These samples prove the predecessor partitions exhaustive and green,
not a robust five-minute latency bound on four-vCPU hosted runners.

The candidate queue graph uses 16 Linux rows and three parallel Windows rows.
Seven application rows cover all 12 compile-time feature selectors, including
the two netplay modules. Three engine-integration rows, separate engine and
frontend unit rows, two residual-workspace rows, and the quality and contract
rows complete the Linux matrix. Windows runtime tests, network tests, and the
quality/NSIS checks run independently instead of sharing one serial critical
path. The ordinary unsharded suite remains the coverage reference.

Hosted workflow-dispatch run `30702040649` exercised the predecessor 18-row
Linux partition at commit `6dd2b490c`. All 18 jobs started within 20 seconds
and passed; workflow creation to the last Linux completion was 4m55s. The
slowest row was remaining workspace 1/2 at 4m51s. Moving the eight `bird_flight`
cases between the existing engine selectors left their paired rows at 4m00s
and 3m46s instead of the predecessor's 4m56s critical row. Every row retained
the trusted-main Rust cache identity, though each restored a compatible prefix
rather than an exact file-hash key. This is a shared-runner sample with five
seconds of end-to-end margin, not a measurement of the candidate graph or a
portable timing guarantee.

Across 88 successful ordinary, non-release merge-group `Landing` runs ending
2026-08-20, workflow creation-to-completion had a p50 of 649 seconds. A full
50% reduction therefore requires an ordinary p50 at or below 324.5 seconds
(324 seconds when reported as a whole duration). The
16-plus-three topology and cache changes are a projection toward that target;
they have not yet produced a live merge-group sample, so do not report the
target as achieved before a comparable live trial. Record queue delay, runner
availability, cache state, and the exact content revision with that trial, and
keep canceled, failed, and release runs separate from the ordinary sample.

Because the merge queue admits one candidate at a time, one Linux row and the
Windows runtime row claim the rolling landing-cache lanes. Required queue work
therefore preempts stale trusted-main producers, while the other rows use
run-scoped lanes; release pushes use SHA-specific groups and are unaffected.

The build values are single sequential directional samples from fresh Cargo
targets; later arms benefited from warmer filesystem caches. The runtime
samples were interleaved, but the desktop session was active and on battery.
Re-measure on the target CI and release platforms rather than treating these
M4 measurements as portable thresholds.

### Compile-first development profile

On 2026-07-30, the Apple M4 Max reference machine used fresh target directories
to compare the ordinary full-workspace development build at `d4060ca0e` with a
profile-only candidate. The content revision was `963e8cb4458b`, the compiler
was Rust 1.97.1, and every build used the locked, offline dependency graph:

```sh
cold_target="$(mktemp -d "${TMPDIR:-/tmp}/clonk-cold-build.XXXXXX")"
caffeinate -dimsu /usr/bin/time -lp env \
  -u CARGO_INCREMENTAL \
  -u CARGO_BUILD_JOBS \
  -u RUSTC_WRAPPER \
  -u RUSTC_WORKSPACE_WRAPPER \
  CARGO_TARGET_DIR="$cold_target" \
  cargo build --workspace --locked --offline --timings
```

After preserving the Cargo timing report and recording the target size, remove
only the directory returned by `mktemp`:

```sh
find "$cold_target" -depth -delete
```

The baseline is a single directional sample. The candidate result is the
median of five independent fresh-target builds; its raw wall samples were
46.63s, 46.01s, 45.87s, 46.04s, and 45.90s. The corresponding raw user CPU
samples were 321.80s, 321.59s, 324.68s, 327.44s, and 329.91s; system CPU was
21.63s, 21.30s, 21.33s, 21.62s, and 21.68s. Target sizes were 3,779,220,
3,778,812, 3,780,428, 3,760,324, and 3,777,788 KiB.

| Development profile | Samples | Cold wall | Build process-tree user CPU | System CPU | Target size |
| --- | ---: | ---: | ---: | ---: | ---: |
| Level-1 workspace default, level-3 dependencies | 1 | 105.10s | 863.40s | 34.40s | 5.2 GiB |
| Level-0 workspace default, level-1 dependencies | 5 | **46.01s median** | 324.68s median | 21.62s median | 3.60 GiB median |

The candidate reduced the observed cold wall time by 59.09s (56.2%) and build
process-tree user CPU by 538.72s (62.4%). It retains debug assertions, line
tables, incremental reuse, and the same full-workspace default build targets
and enabled features. The combined A/B reduced development-only optimization
and debug data and disabled thin-local LTO; it does not isolate the
contribution of each setting. No production source or feature gate changed.
`clonk-scaling` remains at level 3 in both configurations.

The `play` profile explicitly restores the preceding level-1 workspace,
level-3 dependency, prior debug, incremental, and thin-local-LTO settings
before applying its existing level-3 hot-crate overrides. The shipped
`release` and the behavior-verification `test` profiles are unchanged.

The candidate passed the full workspace nextest and `-D warnings` clippy gates,
all three engine snapshots, and the pinned C++ parity golden. The full
workspace also checked successfully under the restored `play` profile.

These measurements are deliberately not promoted to a portable reference
baseline. The desktop session was active, and the macOS 26.5.2 AC power reports
disagreed at measurement time: `pmset -g live` returned `powermode 2`, while
`system_profiler SPPowerDataType` reported Low Power Mode enabled. Neither was
sampled continuously through every build, so the actual build-time power state
is indeterminate. The five-sample 0.76s range and 46.01s median establish the
local one-minute target with 13.99 seconds of margin; future cross-revision
comparisons still require a stable, idle power fingerprint.

On 2026-07-19, a same-checkout profile A/B used fresh target directories on
the Apple M4 Max reference machine with Rust 1.87.0. Both builds used
`cargo test --workspace --no-run --locked --offline --timings`; execution used
the full nextest workspace gate. Explicitly disabling thin-local LTO while
raising workspace test optimization from level 2 to level 3 produced the best
compile/runtime balance:

| Test profile | Cold build | Compiler user CPU | Full suite | Build + suite | Target size |
| --- | ---: | ---: | ---: | ---: | ---: |
| opt 2, implicit thin-local LTO | 213.40s | 1523.80s | 86.238s | 299.638s | 4.6 GiB |
| opt 3, LTO off | 107.41s | 1121.22s | 90.552s | 197.962s | 3.7 GiB |

This is a 49.7% cold-build reduction and a 33.9% end-to-end reduction on that
machine, with a 4.3s warm-suite cost. Re-measure rather than extrapolating the
result to a different toolchain or runner.

### Test-harness and scheduling follow-up

The next feedback-loop pass kept production engine code at test-profile
optimization while moving all three engine test wrappers into the
`clonk-engine-unit-tests` leaf package at optimization level 0. Test inventories
were compared before and after the move; the 1,951 inline tests were
byte-identical, and the unchanged integration source contributed another 266
tests. On the same M4 Max/Rust 1.87.0 checkout:

| Isolated workload | Result |
| --- | ---: |
| Inline engine wrapper, old level-3 clean compile | 86.87s |
| Inline engine wrapper, leaf level-0 clean compile | 26.89s |
| Integration wrapper, leaf level-0 compile with dependencies ready | 7.66s |
| All three engine binaries, 3,256 tests | 36.948s |
| Full app binary, 1,385 tests | 48.794s |
| Lightweight `dev-check` dispatcher, cold / warm | 1.78s / about 0.35s |

The exact inline-wrapper A/B removes 59.98s (69.0%) of compiler work. The
engine and app suite figures are component measurements, not numbers to add:
nextest schedules the two surfaces concurrently in the workspace gate.

The same pass also:

- keeps 256 test-profile codegen units when CI sets `CARGO_INCREMENTAL=0`;
- starts real-scenario tests before short unit work to avoid a serial tail;
- uses synthetic GUI fixtures by default in app tests, retaining classic
  resources only for pixel/resource-sensitive cases;
- splits dependency-light `dev-check` planning from engine-backed xtask tools;
- upgrades CPAL to the bindgen-free CoreAudio backend and removes that cold
  native binding-generation step; and
- treats snapshot and differential checks as part of the workspace nextest
  run instead of executing them a second time afterward.

The final rebased landing gate compiled its remaining warm-cache work in 9.75s
and passed 7,357 tests in 95.772s. Several other workers were active and the
base gained 29 tests during the pass, so this is retained as correctness and
warm-build evidence rather than promoted to a comparable execution baseline.
Use an idle machine for the final cached and cold workspace measurements.

### Cached-runtime and leaf-profile follow-up

The next pass added an opt-in nextest `profiling` profile with JUnit output.
One deliberately contended diagnostic run passed 7,359 tests (24 skipped) in
134.203s after 16.60s of compilation. The sum of individual testcase durations
was 2,138.4s; that sum measures work across concurrent processes, not elapsed
feedback-loop time. It identified the app and engine scenario binaries as the
dominant remaining surfaces and should not be compared with an idle gate.

The resulting changes keep compile-bound wrappers cheap while optimizing only
the runtime-heavy leaves:

- the 666-test frontend inline surface moved to `clonk-frontend-unit-tests` at
  level 1; its isolated execution fell from 12.798s at level 0 to 3.409s;
- `engine_it` moved to `clonk-engine-integration-tests` at level 1, while the much
  larger `engine_inline` and `unit` wrappers remain at level 0;
- both engine leaf packages request the app's `test-graph` feature, preventing
  a focused engine-then-app cycle from producing two optimized engine copies;
- test-profile engine line tables are disabled. The previous engine rlib held
  about 36 MiB of debug sections that every large test link had to process;
- silent/pathless app fixtures skip install audio discovery and a guaranteed
  empty boot worker; and
- immutable definition/script tables, command metadata, object indices, and
  fallback landscape height are reused across hot simulation queries.

A cached isolated app sample after these changes passed 1,409 tests (5
skipped) in 44.770s (`n = 1`). The earlier 48.794s component reference had
1,385 tests, so the newer result is useful feedback-loop evidence but not a
strict same-inventory A/B. An exact landscape hot-path sample retired 308.3
million instructions after caching the fallback height, versus 345.3 million
before it (`n = 1`, 10.7% less instruction work).

A same-machine, same-test, single-sample (`n = 1`) binary A/B used hardware
work counters because other workers made wall time non-comparable:

| Binary | Instructions | Cycles |
| --- | ---: | ---: |
| Earlier level-0 integration binary | 213,410,300,013 | 42,184,413,414 |
| Level-1 binary plus engine hot-path changes | 137,649,905,676 | 27,927,766,891 |

The complete Tutorial09 virtual route therefore used 35.5% less instruction
work and 33.8% fewer cycles. The command shape was
`/usr/bin/time -lp target/debug/deps/engine_it-<hash> --exact real_tutorial09_virtual_play::tutorial09_virtual_player_completes_the_real_tutorial_route --nocapture`.
Promote an elapsed speedup only after a repeated idle full-workspace
measurement.

### Serialized-fleet and scenario-batching follow-up

The next pass separated host contention from Cargo and test costs. Multiple
worktrees had independently compiled the 162k-line `clonk-app` harness while a
workspace gate was running; one observed contended gate took 50.92s to compile
and 73.736s to execute, with host load around 25--30 on 16 logical cores. That
sample is evidence of contention, not a profile baseline. The shared worker
protocol uses `queue/merge.lock` as its only build slot. Every Cargo, rustc,
and nextest invocation acquires it with 5 ms polling and releases it promptly,
while source inspection and editing remain lock-free. The authoritative
rebase, workspace gate, and fast-forward hold the same lock as one operation,
and the redundant whole-crate working-phase suite remains omitted.

A live follow-up exposed the remaining release/acquire race: scoped work could
start just after one landing released the lock while the next landing acquired
it, causing both to run together. One nearly cached 8,120-test gate then took
129.299s to execute, versus 61.360s for a nearby 8,112-test gate; the samples
differ by eight tests and are directional contention evidence, not a test-code
A/B. After acquiring the existing merge lock, a landing worker also polls at
5 ms until pre-existing Cargo, rustc, and nextest processes drain. This is a
transition guard for commands launched by workers following an older protocol;
the held lock prevents new compliant diagnostics from starting without a
second lock or a redundant test pass.

With the host build slot isolated, a same-source, one-line incremental app
change compared warmed test-profile artifacts (`n = 1` each):

| `clonk-app` test optimization | Compile | 1,463-test suite | Compile + suite |
| --- | ---: | ---: | ---: |
| level 3 | 30.98s | 34.025s | 65.005s |
| level 2 | 29.02s | 32.075s | 61.095s |

Level 2 reduced this measured app-change loop by 3.910s (6.0%) without a
runtime penalty in the sample. Level 1 was rejected after a slower directional
run (34.08s compile plus 35.256s execution). These are local single samples,
not regression thresholds; the final workspace gate remains authoritative.

The real Alchemy integration surface also repeated the same immutable scenario
parse in 17 nextest processes. It now uses four balanced nextest-visible
batches. Each batch prepares the scenario once, instantiates a fresh `Engine`
for every one of the 17 unchanged assertion bodies, continues after caught
subcase panics, and reports all failed subcase names. This preserves fresh
simulation state and four-way parallelism while reducing scenario parses from
17 to four. The uncontended focused result compiled in 2.80s and passed all
four batches in 7.936s (11.53s command wall time); batch durations ranged from
5.844s to 7.935s. Earlier 17-test and single-batch runs were contended, so they
are retained only as diagnostics rather than claimed as a formal elapsed A/B.

The rebased workspace gate for that pass compiled in 51.09s, passed 7,437
tests (20 skipped) in 64.997s, and took 118.98s wall time. This is the
pre-follow-up reference for the next scenario-batching pass, not a portable
threshold.

Five app-level Alchemy mouse tests also repeated the same catalog discovery
and scenario parse. Two nextest-visible batches now prepare that immutable
input once apiece and create a fresh temporary user-data tree, `GameApp`, and
`Engine` for each unchanged assertion body. The focused result passed both
batches in 6.974s after a 31.80s app-harness compile; the individual batches
took 6.719s and 6.965s. The five earlier same-profile testcase samples totaled
41.723s and had a 10.707s slowest case, but they were collected inside a full
app run, so the reduction is directional evidence rather than a strict
same-command A/B.

Drachenfels and Goldrush also repeated scenario parsing across 17 standalone
integration tests. Three Drachenfels batches and two Goldrush batches provide
five-way process parallelism, share only immutable preparation, and instantiate
a fresh seeded engine for every subcase. An uncontended focused run compiled
the integration wrapper in 3.48s and passed all five batches in 5.411s (9.89s
command wall time); batch durations ranged from 3.048s to 5.410s. No comparable
17-process timing was retained, so this records the new reference without
claiming an elapsed speedup.

A same-source directional compile probe also set the app package's test-profile
codegen units from 256 to 128. The app test harness compiled in 36.07s, versus
31.80s for the preceding 256-unit build (`n = 1` each; cache states were not
controlled). The probe showed no benefit, so the override was removed;
preserving 256 units keeps more LLVM parallelism and finer-grained incremental
reuse for this unusually large test target.

After those batches landed, the complete workspace gate compiled in 38.36s,
passed 7,440 tests (20 skipped) in 62.517s, and took 104.18s wall time. The
execution phase was 2.480s faster than the preceding 64.997s reference but
remained 2.517s above the sub-minute target.

A follow-up profile found ten more seed-zero Goldrush tests across six
integration modules, each independently parsing the same installed scenario.
Their ten testcase durations totaled 73.696s and the slowest completed in
9.900s. Three balanced nextest-visible batches now prepare Goldrush once per
process and instantiate a fresh engine for every unchanged assertion body. The
focused comparison passed all three batches in 6.049s; their durations totaled
14.508s (2.959s, 5.501s, and 6.048s). That single-sample comparison reduces the
family's elapsed span by 3.851s (38.9%) and aggregate process time by 59.188s
(80.3%) while preserving three-way parallel execution.

### Linker, development-profile, and feature-graph follow-up

On 2026-07-20, the Apple M4 Max/Rust 1.87.0 reference machine used fresh
target directories to measure the compile side of the complete feedback loop.
All commands were offline and lockfile-pinned; each result is a single local
sample, not a portable regression threshold.

| Workload | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Cold `cargo build --workspace` | 111.95s | 93.38s | 18.57s (16.6%) |
| Cold `cargo test --workspace --no-run` | 103.94s | 91.30s | 12.64s (12.2%) |
| Focused Alchemy compile after a workspace build | 57.97s | 21.55s | 36.42s (62.8%) |
| Focused compile plus two Alchemy batches | 64.10s | 26.87s | 37.23s (58.1%) |

The default development profile now keeps workspace crates at level 1. An
explicit `play` profile applies level 3 only when launching representative
gameplay, so interactive runtime requirements no longer lengthen every normal
edit/build cycle. On Apple Silicon, a checked-in Clang-driver wrapper selects
the LLVM-matched `ld64.lld` shipped with the active Rust toolchain. A direct
same-source linker probe reduced the cold test build from 103.94s to 99.21s
(4.5%); the lower 91.30s final sample also includes the other changes and host
variation, so it is not attributed to the linker alone.

The first focused integration command after a complete workspace build
unexpectedly rebuilt the engine dependency chain. Comparing Cargo feature
graphs found focused-only resolver-v2 fingerprints in `libc`, `rustix`,
`memchr`, `smallvec`, and host-side `syn`. The engine, frontend, and network
test entry points now anchor the feature sets already enabled by the complete
workspace. A post-change `cargo tree` comparison found zero focused-only
feature fingerprints for all five entry points, and the exact rerun rebuilt
only the integration wrapper, producing the 58.1% command reduction above.

Two remaining standalone Alchemy visibility tests now run inside existing
prepared-scenario batches. Every subcase still instantiates a fresh engine and
the batch still catches and aggregates each named failure, while two redundant
scenario parses and two nextest processes disappear. A priority-20 nextest
tier also starts the five network dynamic-parameter tests and two xtask Git
tests before ordinary priority-zero work; these seven tests repeatedly formed
the cached suite's final tail, while all real-scenario tiers retain priority
60 or higher.

Two alternatives were rejected by measurement. Limiting Rayon to two threads
made the exact 7,921-test cached suite slower (59.601s versus 57.976s), and a
single-process libtest run immediately exposed races in process-global app
state. Nextest process isolation therefore remains part of the correctness
model rather than an interchangeable scheduling choice.

### Debug-payload and scenario-preparation follow-up

Two compile passes removed additional test-profile debug data without changing
generated code or test runtime. Frontend, resources, and script accounted for
18,365,488 bytes (17.515 MiB) of object-file DWARF. A post-change artifact
audit found another 8,343,469 bytes across the remaining workspace rlibs,
expanded to 43,125,856 bytes after downstream-link fanout, plus 25,171,978
bytes in loose debug-map objects. The follow-up therefore made `debug = false`
the workspace-member test-profile default instead of accumulating package
exceptions. The later cold-profile follow-up below removed the dependency
wildcard entirely by inheriting the release profile, while retaining explicit
test assertions, overflow checks, and local incremental compilation.
Panic-site text and function symbols remain available, while the development
profile retains debug information for interactive use. Set
`CARGO_PROFILE_TEST_DEBUG=line-tables-only` for a line-symbolized test build.

The same pass eliminated four redundant installed-scenario preparations while
preserving a fresh `Engine` and player for every assertion body. Tutorial05's
long catapult test now prepares its immutable input once for both the original
and restored engines, and four shorter Tutorial05 cases run in two balanced,
failure-aggregating batches. The Arctic harpoon/drop test likewise prepares
its immutable scenario once and instantiates two independent engines. Retained
pre-change profiling samples totaled 14.293s across the five active Tutorial05
cases and 7.112s for the Arctic case; those figures identify repeated work and
are not presented as a same-command elapsed A/B.

### Clippy targets and Tutorial01 preparation follow-up

The full lint command now names production libraries, binaries, and tests
instead of using Cargo's `--all-targets` expansion. The workspace has no
examples, and both explicit Criterion benches require opt-in `bench` features,
so lint coverage is unchanged. The old expansion additionally created five
implicit benchmark-mode roots across targets that disable ordinary test
harnesses: `clonk-engine`, `clonk-frontend`, `clonk-logging`, and both `xtask` binaries.
Those roots span 98,217 source lines; engine and frontend account for 96,512
of them. The explicit target set avoids that duplicate codegen while retaining
every production and test target.

Four remaining Tutorial01 app assertions also prepared the same immutable
installed scenario in separate nextest processes. Two failure-aggregating
batches now prepare it once for the two message-render assertions and once for
the two save/music assertions. Each assertion still receives a fresh app,
engine, environment guard, and temporary user-data tree, while the 9.178s
physical Tutorial01 route stays standalone. Retained pre-change samples for
the four batched assertions totaled 20.428s; that aggregate identifies the
duplicated work and is not claimed as a same-command elapsed speedup.

### Remaining two-case scenario preparation follow-up

Four more two-case families loaded the same immutable installed scenario in
separate nextest processes. Jungle amulets, the Deep Sea airlock/lorry pair,
the Arctic kayak pair, and two Tutorial09 app assertions now prepare their
scenario once per family and instantiate a fresh engine or app for every
unchanged assertion body. Each batch catches subcase panics and reports every
failed subcase name; the Arctic batch retains the kayak-rowing regression's
executable test ID. The batches also start in the existing real-scenario
priority tiers so their sequential subcases do not become a late suite tail.

Retained pre-change samples total 27.360s for seven of the eight assertions:
8.135s for Jungle, 8.009s for Deep Sea, 7.116s for Tutorial09, and 4.100s for
the Arctic cargo case. The newer Arctic rowing regression has no retained
pre-change profile sample. These totals identify four redundant preparations;
they are not an elapsed A/B because the old test processes ran concurrently.

The same pass tested a package-only `clonk-engine` test-profile reduction from
optimization level 3 to 2 in an isolated target. Its two unit samples took
33.80s and 37.36s, versus a 35.56s level-3 workspace reference; fleet overlap
invalidated the requested brackets, so none is promoted to a clean A/B. Even
the favorable sample saved only 1.76s, below the 3s acceptance threshold, and
the candidate rlib was only 0.63% smaller. Since level 2 lacked evidence of a
material compile win and could slow simulation-heavy tests, the override was
not adopted and no runtime tradeoff was taken.

### Tutorial07 scheduling and host-state follow-up

Five seed-zero Tutorial07 integration assertions repeated the same immutable
scenario preparation, but an exact same-host A/B rejected batching them. The
five parallel processes passed in 2.299s with 7.112s of aggregate testcase
work; two batches passed in 2.448s with 4.086s aggregate. Their wrapper builds
were equivalent at 6.25s and 6.18s. The 42.5% aggregate-work reduction did not
offset the lost process parallelism, so the measured 6.5% elapsed regression
keeps all five original process and test-ID boundaries.

Those five Tutorial07 tests and four real-resource frontend tests previously
started near the end of the cached suite. They now share the priority-60
scenario tier. This changes only start order and retains every independent
process and exact test ID.

A further Goldrush/Drachenfels consolidation was rejected by an exact
same-host, same-cache A/B. The parent configuration's eight processes passed
in 9.201s after a 6.05s wrapper build; five merged processes took 11.491s after
an equivalent 6.06s build. Drachenfels' own slowest process also rose from
6.873s to 7.110s. The measured 24.9% family regression outweighed the removed
scenario preparations, so those process boundaries remain unchanged.

### Script-call allocation and native-catalog follow-up

A natural incremental `cargo test --workspace --no-run --timings` build took
47.22s. Cargo's critical path was the 40.96s `clonk-app` binary test harness;
`engine_inline` took 13.22s and `frontend_inline` took 9.53s in parallel. This
confirms that further scheduling changes cannot materially shorten the compile
half of the loop without reducing the monolithic app harness or compiler work.

A three-pair, interleaved same-source binary A/B used the real Tutorial03
virtual-player route and macOS retired-instruction/cycle counters. The first
candidate removes repeated `Vec` growth while balancing the ten script-call
slots, moves diagnostic argument values into their frame with a compact
reference mask, and borrows ordinary UTF-8 bytes instead of allocating native
byte projections. The second candidate also shares the immutable 457-function,
292-constant native registration catalog copy-on-write across script hosts;
per-host world dispatch hooks remain independent.

| Route binary | Mean wall | Retired instructions | CPU cycles | Peak memory |
| --- | ---: | ---: | ---: | ---: |
| Parent | 6.197s | 118.284B | 23.126B | 164.12 MB |
| Call/string allocation changes | 5.580s | 109.965B | 21.481B | 163.96 MB |
| Plus native registration cache | 5.287s | 105.530B | 20.561B | 82.86 MB |

The combined binary retired 10.78% fewer instructions and 11.09% fewer cycles,
used 49.51% less peak memory, and completed 14.68% faster in this route. Every
pair used separately retained executables and the same test/content inputs, so
source rebuilds and fleet contention are outside the measured interval.

Five Tutorial01 real-Clonk assertions now share one immutable scenario parse
while preserving a fresh engine and failure label for every subcase. Their
retained independent-process samples totaled 10.248--11.287s of testcase work;
the batched process took 2.937--3.490s. Because the old processes overlapped,
this is an aggregate-work reduction rather than an isolated elapsed claim. The
batch starts in the priority-60 real-scenario tier so it cannot become a late
serial tail.

The same source was then compiled from two empty target directories with
`CARGO_INCREMENTAL=0`, offline dependencies, and a locked dependency graph.
The baseline test profile inherited the development dependency wildcard; the
candidate inherits release and lets Cargo's default host build-dependency
profile apply, while the checked-in profile explicitly re-enables incremental
compilation for normal local rebuilds.

| Cold workspace test build | Wall | User CPU | System CPU | Target size |
| --- | ---: | ---: | ---: | ---: |
| Optimized host wildcard | 112.61s | 862.57s | 30.16s | 1,106,980 KiB |
| Cargo host fast path | 103.17s | 765.25s | 27.53s | 1,124,792 KiB |

The candidate saved 9.44s wall (8.4%) and 97.32 user CPU-seconds (11.3%) for a
17,812 KiB (1.6%) target-size increase. Unit timings provide a source-local
control against second-run filesystem caching: `syn` fell from 10.9s to 2.1s,
`clap_derive` from 5.6s to 0.9s, `serde_derive` from 5.0s to 1.4s, and
`tracing-attributes` from 4.4s to 0.8s. Normal runtime dependencies remain at
the test profile's level 3; only host build scripts, procedural macros, and
their host-only dependencies regain Cargo's compilation-oriented settings.

### Toolchain and Darwin-linker follow-up

On 2026-07-21, commit `11f43e7eb` was compiled from empty target directories
on the same AC-powered Apple M4 Max. Each `cargo test --workspace --no-run`
sample used the locked, offline dependency graph and the checked-in test
profile. These are sequential single samples; later runs had a warmer OS file
cache, so the exact delta is not a portable regression threshold.

| Rust and Darwin linker | Wall | User CPU | System CPU |
| --- | ---: | ---: | ---: |
| Rust 1.87.0 + bundled Mach-O LLD | 115.20s | 794.90s | 35.58s |
| Rust 1.97.1 + bundled Mach-O LLD | 105.08s | 765.56s | 32.39s |
| Rust 1.97.1 + Apple system linker | 90.67s | 749.43s | 29.79s |

The adopted toolchain/linker pair reduced this same-source cold sample by
24.53s (21.3%) and 45.47 user CPU-seconds (5.7%). Rust 1.97.1 also includes
the upstream fix for an LLVM miscompilation present since at least Rust 1.87.
The workspace pins the compiler so local workers and CI share the same cache
fingerprint.

A follow-up compared Apple ld modes from fresh targets on the same source. The
ordinary linker took 93.09s wall, 751.64s user, and 30.43s system CPU;
`-no_deduplicate` took 92.42s and was rejected as noise. Apple's debug-build
`-O0` mode took 87.28s wall, 742.05s user, and 27.61s system CPU: 5.81s (6.2%)
less wall time and 9.59 fewer user CPU-seconds than the direct control. The
checked-in linker shim applies that mode only when rustc's output is under the
`debug` profile directory; `play` and `release` retain the normal layout.

The candidate target then passed all 8,348 selected workspace tests, with 10
skipped, in a 57.486s nextest phase. Apple documents `-O0` as disabling linker
optimizations and layout algorithms specifically to speed debug incremental
development; it does not change Rust or LLVM optimization levels.

### Dynamic Darwin test-root follow-up

Rust's standard library is also available as a toolchain dylib. On Darwin,
test commands opt into `.cargo/rustc-test-wrapper` through the
`RUSTC_WORKSPACE_WRAPPER` environment variable. It selects
`-Cprefer-dynamic -Crpath` solely for `--test` roots. Workspace libraries,
ordinary binaries, non-test `play`/`release` builds, and non-Darwin targets
retain static linkage. Keeping this out of global Cargo configuration also
prevents native Windows Cargo from trying to execute a POSIX script. From the
the repo root directory, the opt-in is:

```bash
RUSTC_WORKSPACE_WRAPPER="$PWD/.cargo/rustc-test-wrapper" cargo nextest run ...
```

Cargo includes the wrapper in artifact identity, so use the same absolute
wrapper path on every Darwin Cargo command instead of alternating wrapped and
unwrapped commands into parallel caches. The worker protocol does this.

The output-relative Mach-O rpath keeps direct test-binary execution working
from its original target directory without modifying `DYLD_LIBRARY_PATH`.
These are local artifacts, not portable binaries: moving them or removing
their exact Rust toolchain requires rebuilding them.

On the same M4 Max and source, empty-target locked/offline builds and two
counterbalanced cached-suite pairs produced these single-sample results:

| Workload | Static std | Dynamic std | Delta |
| --- | ---: | ---: | ---: |
| Cold workspace test build | 92.15s | 93.35s | +1.20s |
| Full suite, pair 1 (8,375 passed) | 60.189s | 56.300s | -3.889s |
| Full suite, reversed pair 2 (8,375 passed) | 58.516s | 54.564s | -3.952s |
| Same-line app code edit/relink | 26.34s | 25.91s | -0.43s |
| Same-line no-code-change relink | 7.97s | 6.31s | -1.66s |

Using the reversed suite pair, the representative code-edit plus full-test
loop fell from 84.856s to 80.474s, a 4.382s (5.2%) reduction. Dynamic linkage
also reduced aggregate testcase time from 912.797s to 859.994s in that pair;
the app harness accounted for most of the reduction. The cold build alone was
1.3% slower, but cold build plus the first suite still fell by 2.689s.

A deliberately pathological edit that inserted a line near the top of the
200k-line app harness rebuilt in 40.97s dynamically versus 33.47s statically;
changing an existing code line instead produced the representative result
above. Keep both tradeoffs visible when comparing future compiler behavior.
All dynamic samples retained normal panic unwinding, ad-hoc Mach-O signatures,
and direct execution; both complete nextest runs passed the full inventory.

The faster-looking LLD `-no_deduplicate` experiment was rejected despite a
97.38s build: an isolated panic probe passed, but the fresh full-workspace
binary aborted while unwinding the same `#[should_panic]` test. The unmodified
1.97 LLD binary failed identically. Apple Clang's system linker passed the
probe from the full-workspace target and avoids relying on layout-sensitive
unwind behavior. After rebasing, the exact checked-in configuration passed all
8,351 tests in 68.359s. Their 1,092.605 aggregate testcase-seconds imply a
68.288s ideal 16-way floor, so the runner remained saturated rather than
exposing a linker-related execution regression.

The preceding rebased workspace gate's 127s compile, 176.742s nextest phase,
and 310.20s command wall time are not a code-performance baseline. It ran with
AC Low Power Mode active during a display-off dark wake and sustained elevated
thermal pressure. No competing worktree produced Rust artifacts in the gate
window, its inventory grew by only 0.14%, and its profiles, feature graph,
linker, and artifact sizes did not regress. Authoritative gates now run under
`caffeinate` and retain nextest JUnit timings through the `profiling` profile;
power-mode state must still be recorded before comparing samples.

The first local reference baseline was recorded on 2026-07-12 from
`dd32e5d3` with content `67a54d0`, Rust 1.87.0, macOS/Darwin arm64, and an
Apple M4 Max. The representative command was:

```sh
cargo dev-check \
  --changed crates/clonk-engine/src/compat.rs \
  --budget-seconds 60 --keep-going
```

It selected deterministic replay, render capture, whitespace hygiene, both
engine unit-test surfaces, and the real-scenario smoke family. All six checks
passed in 20.304s inside `dev-check`; rebuilding the modified xtask wrapper
before it started took another 5.09s. Cargo-reported build time inside the
checks was 12.250s, almost entirely the external engine unit binary (12.030s),
making compilation the measured bottleneck. The retained runtime samples were:

| Phase | Local reference |
| --- | ---: |
| Tutorial01 scenario load | 245.707–257.887ms |
| Player join | 1.179–1.184ms |
| Three simulated ticks | 3.862–3.914ms |
| First 320×180 render | 0.670ms |
| Cached 320×180 render median | 0.614ms |

These numbers prove the representative local gameplay loop is below the
60-second target and identify its current bottleneck. They are not used as a
cross-machine relative gate; CI enforces the portable 60-second focused-loop
ceiling and retains its own reports while a comparable hosted-runner history
is collected.

Collect at least 20 successful default-branch runs with an unchanged runner,
toolchain, workload, and cache classification. Then review and commit a baseline
with its complete fingerprint. Reset collection after any fingerprint change.

For a proposed regression gate:

- warn when a comparable median rises by more than 10%;
- block only when it rises by more than 15% and also exceeds an absolute noise
  floor chosen from the collected samples;
- require an explicit, reviewed baseline update when a deliberate tradeoff is
  accepted;
- keep shared-runner microbenchmarks informational unless their observed noise
  supports enforcement.

This dual relative/absolute rule prevents a tiny metric from failing because of
sub-millisecond scheduler noise.

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

The 2026-08-14 assertion run (`cargo bench ... -- --test`) recorded 2,000
unfogged owner instances and 2,400 fog-expanded owner instances. Unfogged
capture made 6 allocation calls totaling 182,936 bytes for both one and 1,000
objects; fogged capture made 8 calls totaling 363,848 bytes for both the
one-object/one-chunk probe and the 1,000-object/1,200-chunk workload. These are
warm capture allocations, not instance-upload bytes; the separately asserted
uploads were 176,000 and 211,200 bytes.

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
The 2026-08-14 Metal assertion run reported one owner-pair scene draw, 2,000
compact instances and 176,000 upload bytes, versus 2,000 generic scene draws,
2,000 generic instances and 464,000 upload bytes. Total draws including final
presentation were 2 and 2,001.

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
well as the percentage. When comparing the pre-amortized baseline at
`346b776cf23f9fe632d1868560121b31368b38bf`, apply the identical benchmark-only
object/particle harness patch to both source trees; its unified-diff SHA-256 is
`108e537372dda870b3b8cb2c6312dca4fda478a10d5da80eb5a84660c5bfa96b`.

The 2026-08-14 control used that base and candidate on an AC-powered Apple M4
Max running macOS 26.5.2. All renderer arms reported `backend: Noop`; Criterion
used a two-second warmup, five-second measurement and 20 samples. The table
reports the geometric candidate/baseline effect, arithmetic mean signed
candidate-minus-baseline duration, and preregistered one-sided Student-t bound:

| Case | Valid/attempted quartets | Geometric effect | Mean C−B duration | 95% upper bound |
| --- | ---: | ---: | ---: | ---: |
| Object capture | 6/42 | +1.037% | +2,344 ns | +1.980% |
| Particle capture | 6/6 | +0.319% | +77 ns | +0.890% |
| Compact object submission | 6/14 | +0.788% | +194 ns | +1.113% |
| Particle submission | 6/19 | +1.162% | +273 ns | +1.848% |

Every bound is below 2%. The object-capture result is deliberately reported at
its narrow 0.020-percentage-point margin. Of 57 rejected quartets, 55 tripped
the predeclared external-load screen; contemporaneous snapshots repeatedly
showed unrelated compiler/test activity. All 57 also failed the same-arm spread
screen, and quartets were not replaced based on effect direction.
An earlier particle-submission campaign that reached only one valid quartet
before its fixed attempt cap was excluded wholesale. After recording the
aggregate results and hashes above, the local
1.9 GiB raw-log bundle and its disposable benchmark worktrees were removed to
recover disk space; the raw estimates are therefore not retained in the
repository. Exploratory Metal runs were nonstationary under concurrent desktop
GPU load and are not presented as latency evidence.

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
profiles to one visible client and was used for the reference below to avoid
desktop window-occlusion throttling. Both layouts still require exactly 24
synchronized players, 24 players with live crew, and 24 live crew objects.
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

The 2026-08-14 reference used a single frontmost 800x600 client on an
AC-powered Apple M4 Max running macOS 26.5.2 and Metal. Both named source roots
were at commit `be647b87b7b6d3b38b84fce2bfe2a40977cfe8ab`; the candidate additionally
had tracked patch SHA-256
`c57c18360e523d3de75f22592cdd5a320cc1d04396873b6e021a34e01e53a21b`.
The paired shared-input fingerprint was
`7e17d2c8af0e4c042db6e0cc265daab1257b1b63296ca6e66cd17fca1533b5c1`,
and both arms' normalized GPU fingerprint was
`dc4386fb78b5ee64d354ca102a8a10c51a4976f0884a0b9bd8ec51ab7d9aa213`.
Each arm completed 2,308 simulation, refreshed, and presented frames in about
60.0 seconds: 38.462 baseline FPS and 38.461 candidate FPS, with zero automatic
skips, network lag, ping, or loss. All 5,760 input probes succeeded in each arm;
candidate p99 input latency was 73.614ms against 75.643ms for baseline.

The retained per-frame distributions were:

| Metric | Baseline p50 / p95 / p99 | Candidate p50 / p95 / p99 |
| --- | ---: | ---: |
| End-to-end graphics CPU | 9.677 / 11.857 / 12.249ms | 9.743 / 10.168 / 10.858ms |
| Stream packing/upload CPU | 0.431 / 0.487 / 0.573ms | 0.435 / 0.482 / 0.524ms |
| Valid GPU `Scene` pass | 0.544 / 0.930 / 1.090ms | 0.547 / 0.919 / 1.032ms |
| Scene draw calls | 2,078 / 2,100 / 2,111 | 1,936 / 1,950 / 1,955 |
| Total draw calls | 2,080 / 2,102 / 2,113 | 1,938 / 1,952 / 1,957 |
| Object draw calls | 366 / 372 / 372 | 398 / 400 / 400 |
| Generic-quad draw calls | 1,647 / 1,669 / 1,679 | 1,451 / 1,465 / 1,470 |
| Object-instance upload | 85,536 / 86,592 / 87,296 B | 96,712 / 97,064 / 97,064 B |
| Generic-quad upload | 775,808 / 800,632 / 804,344 B | 745,184 / 759,800 / 763,976 B |
| Generic sprite fallbacks | 512 / 599 / 617 | 497 / 548 / 565 |
| Owner-mask fallbacks | 94 / 94 / 94 | 0 / 0 / 0 |

At the median, owner-mask fallback fell 100%, scene draws fell 6.83%, total
draws fell 6.83%, and generic-quad upload fell 3.95%. Compact object upload
rose 13.07% because the formerly generic owner layers moved into the 88-byte
object stream. Median end-to-end, packing, and valid `Scene` timings changed by
+0.68%, +0.90%, and +0.57%; their tails improved, but this single sequential
campaign ran alongside unrelated desktop activity and is not a statistically
controlled performance-improvement claim. It proves the structural conversion,
the observed zero owner-mask fallback result, painter-visible presentation, and
native-tick budget.

Metal retained 2,278/2,308 valid baseline and 2,279/2,308 valid candidate
`Scene` samples. The corresponding monitor-gamma counts were 2,140 and 2,204;
presentation counts were 82 and 99. Every other sample was an explicit
`counter_rollover`; readback-error telemetry was 3,755 and 3,680, with zero
dropped frames and discontinuities. Four setup campaigns were rejected before
this result: the 12-window run was occlusion-starved, the first one-client run
was not foreground, a partial-foreground run exposed the Metal rollover
handling, and one candidate launch lost an activation race. None contributes
to the table.

The final `summary.json`, baseline evidence, and candidate evidence SHA-256
values were respectively
`c9354927f53b24fe5c8f4ee7bb7060f1a306f7a3165e17bbc4ea8ded9c64bdf6`,
`7dd41934dff220e13e032d980aee9bc1826c2b4778ebbb894f0f385574236513`,
and `2636c54e30f6483e19b5a8a7cd0034c92895853024c15f5827aeeb7520343681`.
The local raw bundle is retained through review and landing, then intentionally
deleted after these aggregates and hashes are recorded to conserve disk space.

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

Reference run for the retained-GPU scene-composition landing candidate on
2026-07-21:

- content revision `67a54d0e662bda3aa0202134efc065d7bc420872`;
- Apple M4 Max, Metal, arm64, 128 GiB memory;
- macOS 26.5.2 (25F84), AC power at 100%;
- 800x600 windowed output, 100% scale, immediate/no-vsync presentation;
- 20.002 seconds measured after warmup, 1,077 successful presentation
  submissions and refreshed frames, and 714 simulation frames.

The machine result was 53.845 presentation-submission FPS, 35.697 simulation
FPS, 5.794ms average and 9.202ms maximum graphics-pass time, with zero
automatic graphics skips. This is a passing native-tick-budget reference, not
a claim that the current path sustains 60 FPS.

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

Landing jobs restore the trusted-main `full-parity` and
`windows-runtime-msvc-v2` caches without saving short-lived merge-queue copies.
The short, non-preempted trusted-main content publisher stores
`.git/modules/content` under an exact key containing the runner OS,
`.gitmodules` hash, and pinned content gitlink. Landing restores that Git-object
cache, materializes the submodule, and verifies both its exact revision and
clean state; a miss may fetch the pinned object but cannot publish from an
untrusted merge-group tree. The Linux producer compiles the complete locked
workspace graph before publishing its Rust cache, so canceled runs cannot
leave an incomplete immutable entry.
Replay/render work restores that ordinary target read-only while instrumented
coverage stays in a different target.

An isolated trusted-main Windows landing-cache producer compiles the Windows
test and lint graph before publishing its reusable dependency artifacts as
`windows-runtime-msvc-v2`.
Windows release tooling follows separately and owns the shipped-runtime
dependency and bounded ThinLTO caches, so its longer static-CRT build is not on
the landing-cache producer's critical path. Release restores the applicable
trusted caches, and recording-host oracles retain their own cache. The Rust
cache keys include the dependency/build inputs maintained by the cache action;
the ThinLTO key additionally pins the Rust and LLVM versions plus manifests and
the shared configuration script.

After a landing-cache key change lands, dispatch `rust.yml` on `main` with
`cache_only=true`. That mode gives the content, Linux, and Windows producers
exact-SHA concurrency lanes and skips the release tooling and post-merge
diagnostics, so a busy queue cannot preempt the bootstrap and a newer push
cannot replace its pending content prerequisite. Ordinary content publishers
continue to coalesce on one non-preempted rolling lane. Fresh dependent Linux
and Windows jobs must restore the two published Rust entries before the
bootstrap reports success.
`CARGO_INCREMENTAL=0` keeps CI artifacts reproducible and smaller; the local
development, play, and test profiles retain incremental behavior.

A cache hit is not proof that every crate was reusable. Report cache state with
the observed compile duration, and investigate unexpected rebuilds before
loosening a budget.
