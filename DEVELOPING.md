# Rust development loop

Run Rust commands from this directory (the repo root). The checked-in content
submodule is part of the test input, so initialize it before running real
scenario or replay checks:

```sh
git submodule update --init --recursive
```

The workspace and CI pin Rust 1.97.1; CI pins cargo-nextest 0.9.91. Rustup
selects the checked-in toolchain automatically, which keeps local and CI
diagnostics comparable. Repository script tests require Python 3.10 or newer.

On Debian or Ubuntu, install the native development dependencies used by CI:

```sh
sudo apt-get update
sudo apt-get install --yes --no-install-recommends \
  libasound2-dev \
  libfreetype6-dev \
  libxmp4 \
  libudev-dev \
  pkg-config
```

Tracker music and its executable IT/MOD/S3M/XM tests require the libxmp 4
runtime (`libxmp4` on Debian/Ubuntu, `libxmp` in Homebrew). Set
`LC_LIBXMP_LIBRARY` to an explicit shared-library path when it is not installed
in a standard system, executable, or macOS app-bundle location.

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
a substitute for the full parity gate. Inspect the plan without running it:

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

Do not run `cargo clean` between feedback cycles. Cargo's local incremental
state is valuable to the edit-test loop. CI disables incremental compilation
because its build cache is reused between clean runners instead.

The default `dev` profile is tuned for the edit/build loop. Use the explicit
`play` profile when launching the interactive client; it keeps debug assertions
but applies level-3 optimization to the rendering, simulation, and script crates
whose level-1 runtime is too slow for representative gameplay:

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

GitHub Actions uploads the entire `target/dev-check` tree from the
`focused-feedback` job even when a selected check fails or exceeds its budget.

## Full pre-merge gate

Focused feedback answers "what did this edit most likely break?" The full gate
answers "is the workspace still mergeable?" Run the complete gate before
handoff or merge:

```sh
cargo fmt --all -- --check
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked
cargo test --workspace --locked
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

The Python unittest discovery covers repository scripts, including public-path
portability and the 24-player benchmark harness. The explicit engine-tools
test command exercises the feature-gated packager, archive, release-dependency,
and release-license checks that the default workspace feature set does not
build. The workspace
test run includes the focused tutorial, virtual-play, snapshot, and C++↔Rust
differential tests. The explicit snapshot and parity commands remain named
completion gates so both baselines are visible independently in local and CI
output. The parity wrapper invokes the pinned cargo-nextest 0.9.91 tool listed
above.
The explicit Clippy target set covers every production library, binary, and
test without rebuilding `test = false` libraries as implicit benchmark
harnesses. The two Criterion benchmarks remain opt-in through their `bench`
features.
Behavior changes can additionally require the relevant scenario sweep/audit
and rebuilt live C++ comparison described in `PORT_STATUS.md`.

The `.github/workflows/rust.yml` workflow runs focused feedback and the full
parity gate as separate jobs. Both check out recursive submodules and restore
their own Rust cache. New pushes cancel obsolete runs for the same pull request
or branch.

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
