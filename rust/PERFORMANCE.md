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
`lc-engine-unit-tests` leaf package at optimization level 0. Test inventories
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

The first local reference baseline was recorded on 2026-07-12 from
`dd32e5d3` with content `67a54d0`, Rust 1.87.0, macOS/Darwin arm64, and an
Apple M4 Max. The representative command was:

```sh
cargo dev-check \
  --changed rust/crates/lc-engine/src/compat.rs \
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

## Reproducing render measurements

Render one explicit replay snapshot:

```sh
LC_DEV_CHECK_SNAPSHOT=target/dev-check/path/to/snapshot-final.json \
LC_DEV_CHECK_FRAME_PNG=target/dev-check/repro/frame-final.png \
LC_DEV_CHECK_RENDER_METRICS=target/dev-check/repro/render-metrics.json \
  cargo nextest run -p lc-frontend --features dev-feedback-render \
  --test dev_feedback_render -- dev_feedback_render --ignored --exact
```

Or let the probe select the newest `snapshot-final.json` recursively:

```sh
LC_DEV_CHECK_ARTIFACT_DIR=target/dev-check \
  cargo nextest run -p lc-frontend --features dev-feedback-render \
  --test dev_feedback_render -- dev_feedback_render --ignored --exact
```

Compare JSON reports only after verifying their input paths/fingerprints and
checksums. The PNG is diagnostic evidence for visual inspection, not a timing
sample.

## CI cache interpretation

The focused and full-parity jobs use separate cache scopes so concurrent jobs
cannot replace a complete parity cache with a smaller focused cache. The cache
is keyed by the Rust dependency/build inputs maintained by the Rust cache
action. `CARGO_INCREMENTAL=0` keeps CI artifacts reproducible and smaller;
local development keeps Cargo's default incremental behavior.

A cache hit is not proof that every crate was reusable. Report cache state with
the observed compile duration, and investigate unexpected rebuilds before
loosening a budget.
