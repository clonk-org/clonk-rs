# Rust development loop

Run Rust commands from this directory (`rust/`). The checked-in content
submodule is part of the test input, so initialize it before running real
scenario or replay checks:

```sh
git submodule update --init --recursive
```

CI uses Rust 1.87.0 and cargo-nextest 0.9.91. Using the same versions makes
local and CI diagnostics comparable.

## Quick, change-aware feedback

Use `dev-check` while editing. It maps changed paths to the smallest useful
compile, unit, replay, and render checks and records each result under
`target/dev-check/`.

```sh
cargo xtask dev-check --base origin/main --budget-seconds 60
```

The 60-second budget limits focused feedback; it is not a performance pass or
a substitute for the full parity gate. Inspect the plan without running it:

```sh
cargo xtask dev-check --base origin/main --plan
```

For an uncommitted or otherwise explicit path, add `--changed` once per path:

```sh
cargo xtask dev-check \
  --changed rust/crates/lc-engine/src/compat.rs \
  --budget-seconds 60
```

Use a focused crate test when its name is already known:

```sh
cargo test -p lc-engine --lib test_name
cargo test -p lc-engine-unit-tests --test unit test_name
cargo test -p lc-engine --test it module_name::test_name
```

Do not run `cargo clean` between feedback cycles. Cargo's local incremental
state is valuable to the edit-test loop. CI disables incremental compilation
because its build cache is reused between clean runners instead.

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
  cargo test -p lc-frontend --features dev-feedback-render \
  --test dev_feedback_render -- \
  --ignored --exact dev_feedback_render
```

By default the probe writes `frame-final.png` and `render-metrics.json` beside
the selected `snapshot-final.json`. Pin every path when reproducing one exact
artifact:

```sh
LC_DEV_CHECK_SNAPSHOT=target/dev-check/path/to/snapshot-final.json \
LC_DEV_CHECK_FRAME_PNG=target/dev-check/repro/frame-final.png \
LC_DEV_CHECK_RENDER_METRICS=target/dev-check/repro/render-metrics.json \
  cargo test -p lc-frontend --features dev-feedback-render \
  --test dev_feedback_render -- \
  --ignored --exact dev_feedback_render
```

`LC_TEST_ARTIFACT_DIR` is also searched when `LC_DEV_CHECK_SNAPSHOT` is not
set. The probe uses empty sprite, cursor, and HUD assets deliberately: it
isolates the deterministic software-render path from machine-specific asset
discovery. It fails if repeated cached renders do not have the same checksum.

GitHub Actions uploads the entire `rust/target/dev-check` tree from the
`focused-feedback` job even when a selected check fails or exceeds its budget.

## Full pre-merge gate

Focused feedback answers "what did this edit most likely break?" The full gate
answers "is the workspace still mergeable?" Run all four commands before
handoff or merge:

```sh
cargo nextest run --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

The workspace test run already includes the focused tutorial and virtual-play
tests; running those filters again in the full gate only duplicates work.
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
