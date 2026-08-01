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

There is no portable measured CI baseline yet. Do not turn timings from an
arbitrary laptop or a shared hosted runner into a blocking threshold.

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

A follow-up on commit `1dd151cfd` modeled the merge queue's more common state:
release libraries restored from the trusted `main` cache, with only the final
application rebuilt. The global profile returned to one codegen unit and only
`clonk-app` varied. Removing the app artifacts between each arm preserved the
same dependency artifacts and shipped `clonk-game`/`c4group` binaries:

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
with status 2 unless at least one refreshed frame was presented, average GPU
graphics-pass time is at most the native 28ms game tick, and
`automatic_graphics_skips=0`. Record the machine line together with the commit,
content revision, display size, hardware, OS, and power state; do not compare
runs with different fingerprints.

Reference run for the L023 retained-GPU landing candidate on 2026-07-21:

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

Landing jobs restore the existing trusted-main `full-parity` and
`windows-runtime-msvc` caches without saving short-lived merge-queue copies.
A post-merge Linux producer compiles the complete locked workspace graph before
it may publish; canceled runs cannot leave an incomplete immutable cache.
Replay/render work restores that ordinary target read-only while instrumented
coverage stays in a different target. Post-merge Windows validation refreshes
smoke, packaging-tool, and exact static-CRT runtime artifacts only after all
three succeed. Recording-host oracles retain their own cache. The keys include
the Rust dependency/build inputs maintained by the Rust cache action.
`CARGO_INCREMENTAL=0` keeps CI artifacts reproducible and smaller; the local
development, play, and test profiles retain incremental behavior.

A cache hit is not proof that every crate was reusable. Report cache state with
the observed compile duration, and investigate unexpected rebuilds before
loosening a budget.
