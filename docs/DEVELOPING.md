# Rust development loop

Run Rust commands from this directory (the repo root). The checked-in content
submodule is part of the test input, so initialize it before running real
scenario or replay checks:

```sh
git submodule update --init --recursive
```

The workspace and CI pin Rust 1.97.1; CI pins cargo-nextest 0.9.91. Rustup
selects the checked-in toolchain automatically, which keeps local and CI
diagnostics comparable. Repository script tests require Python 3.11 or newer.

On Debian or Ubuntu, install the native development dependencies used by CI:

```sh
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  libasound2-dev \
  libfreetype6-dev \
  libfluidsynth3 \
  libxmp4 \
  libudev-dev \
  pkg-config
```

Tracker music and its executable IT/MOD/S3M/XM tests require the libxmp 4
runtime (`libxmp4` on Debian/Ubuntu, `libxmp` in Homebrew). MIDI music needs
FluidSynth 2 (`libfluidsynth3` on Debian/Ubuntu, `fluidsynth` on Arch,
`fluid-synth` in Homebrew) plus a General MIDI SoundFont. Set
`LC_LIBXMP_LIBRARY` or `LC_FLUIDSYNTH_LIBRARY` to an explicit shared-library
path when either runtime is not installed in a standard system, executable, or
macOS app-bundle location.

## Quick, change-aware feedback

Use `dev-check` while editing. It maps changed paths to the smallest useful
compile, unit, replay, and render checks and records each result under
`target/dev-check/`. The dedicated alias is intentionally dependency-light, so
planning a check does not first build the engine and resource stack.

```sh
cargo dev-check --base origin/main --budget-seconds 60
```

`cargo xtask dev-check` remains an equivalent compatibility spelling.

The 60-second budget limits focused feedback; it is not a performance pass or
a substitute for the full landing gates. Inspect the plan without running it:

```sh
cargo dev-check --base origin/main --plan
```

For an uncommitted or otherwise explicit path, add `--changed` once per path:

```sh
cargo dev-check \
  --changed crates/clonk-engine/src/compat.rs \
  --budget-seconds 60
```

Use a focused crate test when its name is already known:

```sh
cargo nextest run -p clonk-engine-unit-tests --test engine_inline test_name
cargo nextest run -p clonk-engine-unit-tests --test unit test_name
cargo nextest run -p clonk-engine-integration-tests --test engine_it module_name::test_name
cargo nextest run -p clonk-frontend-unit-tests --test frontend_inline test_name
```

The engine wrappers live in dedicated leaf packages so Cargo can keep their
orchestration code cheap while retaining an optimized production `clonk-engine`
library. The scenario-heavy `engine_it` wrapper uses light optimization; the
larger compile-bound inline wrapper does not. A bare `-p clonk-engine` selection
therefore checks the library package but does not select these test binaries.

The frontend's inline unit surface likewise lives in
`clonk-frontend-unit-tests`; `clonk-frontend` retains the opt-in
`dev_feedback_render` integration target and the optimized production library.

Do not run `cargo clean` between feedback cycles. Cargo's local artifact and
incremental caches are valuable to the edit-test loop. CI disables incremental
compilation because its build cache is reused between clean runners instead.

The default `dev` profile is tuned for cold and ordinary edit/build latency. It
keeps debug assertions, line tables, and incremental reuse, but most workspace
crates are unoptimized and dependencies use only light optimization.
`clonk-scaling` remains level 3 because it is small and hot per pixel. This is
not a representative runtime build. Use the explicit `play` profile when
launching the interactive client; it restores level-3 dependencies and applies
level-3 optimization to the selected rendering, simulation, and script crates:

```sh
cargo run --profile play -p clonk-app
```

On Apple Silicon macOS, Cargo invokes Apple Clang and the system linker through
the checked-in `.cargo/macos-system-clang` shim. Debug and test outputs use
Apple ld's build-time-oriented `-O0` layout; the `play` and `release` profiles
retain the normal linker layout. Rust 1.97.1's bundled Mach-O LLD does not
preserve panic unwinding for every workspace test binary, so do not override
the checked-in linker. Other targets continue using their platform default.

## Artifacts and replay reproduction

Each invocation writes `target/dev-check/<run-id>/` with this stable layout:

```text
manifest.json              changes, selected plan, reproduction commands
timings.json               command durations and imported phase metrics
summary.json               pass/fail, budget state, slowest phase
commands/*.stdout.log      captured command output
commands/*.stderr.log      captured command diagnostics
snapshot-final.json        final replay snapshot, when a replay ran
replay-metrics.json        load/simulation replay metrics, when available
frame-final.png            deterministic frontend render, when available
render-metrics.json        cold/cached render samples and checksums
cpp-rust-diff.json         optional differential result
```

The ignored frontend probe renders the newest `snapshot-final.json` it finds at
a fixed 320x180 resolution:

```sh
LC_DEV_CHECK_ARTIFACT_DIR=target/dev-check \
  cargo nextest run -p clonk-frontend --features dev-feedback-render \
  --test dev_feedback_render -- dev_feedback_render --ignored --exact
```

By default the probe writes `frame-final.png` and `render-metrics.json` beside
the selected `snapshot-final.json`. Pin every path when reproducing one exact
artifact:

```sh
LC_DEV_CHECK_SNAPSHOT=target/dev-check/path/to/snapshot-final.json \
LC_DEV_CHECK_FRAME_PNG=target/dev-check/repro/frame-final.png \
LC_DEV_CHECK_RENDER_METRICS=target/dev-check/repro/render-metrics.json \
  cargo nextest run -p clonk-frontend --features dev-feedback-render \
  --test dev_feedback_render -- dev_feedback_render --ignored --exact
```

`LC_TEST_ARTIFACT_DIR` is also searched when `LC_DEV_CHECK_SNAPSHOT` is not
set. The probe uses empty sprite, cursor, and HUD assets deliberately: it
isolates the deterministic software-render path from machine-specific asset
discovery. It fails if repeated cached renders do not have the same checksum.

The post-merge main validation workflow regenerates deterministic replay
evidence, runs the ignored render probe over that exact snapshot, and uploads
the resulting `target/dev-check` tree. The assertions that decide whether a
tree may land remain in the ordinary Linux test shards.

## Full pre-merge gate

Focused feedback answers "what did this edit most likely break?" The full gate
answers "is the workspace still mergeable?" Run the complete gate before
handoff or merge:

```sh
cargo fmt --all -- --check
python3 -m unittest discover --buffer -s scripts/tests -p 'test_*.py'
cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked
cargo nextest run --workspace --no-fail-fast
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

The Python unittest discovery covers repository scripts, including public-path
portability and the 24-player benchmark harness. The explicit engine-tools
test command exercises the feature-gated packager, archive, and release-dependency
checks that the default workspace feature set does not build. The workspace
test run includes the focused tutorial, virtual-play, snapshot, and C++↔Rust
differential tests. The explicit snapshot and parity commands remain named
completion gates so both baselines are visible independently in local and CI
output. The parity wrapper invokes the pinned cargo-nextest 0.9.91 tool listed
above.
The explicit Clippy target set covers every production library, binary, and
test without rebuilding `test = false` libraries as implicit benchmark
harnesses. The three Criterion benchmarks remain opt-in through their `bench`
features.
Behavior changes can additionally require the relevant scenario sweep/audit and
a rebuilt live C++ comparison against an oracle checkout selected by
`LEGACYCLONK_ORACLE_ROOT`.

`.github/workflows/landing.yml` keeps pull-request admission small, then runs
the exhaustive workspace suite as compile-time shards against the exact merge
queue tree. Twelve application feature selectors cover the exhaustive fragment
inventory, with one route-support fragment shared by selectors 3 and 11; nine
shared harness tests run once in selector 5. Nine application rows distribute
all 12 feature shards, including the two independently compiled netplay
modules. Three engine-integration rows, one combined engine/frontend-unit and
parity row, two disjoint residual-package rows, and dedicated quality and
contract rows complete the 17-row Linux matrix. Two Windows rows keep the slow
network compile independent while the shorter runtime row also runs quality
and NSIS checks. Formatting,
script tests, lints, parity, snapshots, packaging, and Windows checks feed one
fail-closed `Landing gate`.

`.github/workflows/rust.yml` runs slower diagnostic coverage, macOS
recording-host oracles, and Windows release tooling after an ordinary SHA
lands. Release candidates instead run exact-SHA qualification inside their
merge-group `Landing` run, and release publication resolves only those
queue-qualified artifacts. A short, non-preempted trusted-main job publishes
an exact content Git-object cache keyed by
`.gitmodules` and the pinned gitlink. Landing consumers restore it, materialize
the submodule, and verify its exact
revision and clean state. A separate trusted-main Windows producer compiles the
landing test/lint graph before publishing its reusable dependency artifacts as
`windows-runtime-msvc-v2`, leaving shipped-runtime validation downstream.
Selected merge-group rows may preempt the rolling Linux and Windows cache
producers. Release commits use exact-SHA groups and remain isolated from that
preemption. Before post-merge diagnostics fan out, a read-only admission job
checks for an active merge group; a shared concurrency lane also lets any
candidate arriving after that check cancel the diagnostic caller.

After a landing-cache key change lands on `main`, seed it without running
post-merge diagnostics:

```sh
gh workflow run rust.yml --repo clonk-org/clonk-rs --ref main \
  -f cache_only=true
```

The explicit dispatch gives all three producers exact-SHA concurrency lanes
that a busy merge queue or newer push cannot preempt or replace. Ordinary
content publishers still coalesce safely on their rolling lane. Fresh dependent
Linux and Windows jobs must restore both Rust caches before the bootstrap can
report success.

The measured baseline is a 649-second ordinary p50 across 88 successful,
non-release merge-group `Landing` runs ending 2026-08-20; a full 50% reduction
means an ordinary p50 at or below 324.5 seconds (324 seconds when reported as a
whole duration). The first uncontended fully seeded sample, run `32410157185`,
finished in 354 seconds, 45.5% below baseline. The revised 17-plus-two graph
splits that sample's longest application pairs, avoids serializing Windows
network tests, and removes setup scans from Linux rows. It remains a candidate
until a comparable live merge-group sample reaches the strict target; record
queue delay, runner availability, cache state, and content revision while
separating canceled, failed, and release runs.

## Cache and timing hygiene

- Keep one `target/` directory per worktree. Do not point concurrent worktrees
  at the main worktree's target directory.
- Treat a cache miss, toolchain change, or `Cargo.lock` change as a different
  compile workload. Do not compare it with a warm compile.
- Keep test/replay time separate from compile time. A fast cached test does not
  establish a cold-build baseline.
- Record retries separately. Nextest retries a small set of known flaky tests;
  a retry is useful evidence even when the final result passes.
- Preserve `target/dev-check` when reporting a failure; its commands, durations,
  replay snapshot, rendered frame, and checksums make the result reproducible.

See [PERFORMANCE.md](PERFORMANCE.md) for metric definitions, provisional
targets, and the baseline process.
