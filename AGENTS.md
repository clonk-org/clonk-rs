# clonk-rs — Engineering Constraints & Standards

Deterministic Rust port of LegacyClonk. This file governs all work in the repo
and overrides defaults.

## Prime directive: parity before features

LegacyClonk is **lockstep-deterministic**. The port is worthless unless it
matches C++ **bit-for-bit on simulation state**. The pinned C++ snapshot is
commit `7d43b47b7d789b533f32d005e64596e0a07019cd`, reachable in this repo's Git
history; no C++ source is checked out here. Set `LEGACYCLONK_ORACLE_ROOT` to an
external checkout of that exact commit for live differential work.

- C++ is the golden oracle. When Rust and C++ differ, C++ is right — unless you
  can *prove* a C++ bug that cannot affect determinism.
- **Never** close a parity failure by editing a test to match Rust, by making
  Rust stricter/looser than C++, or by stubbing a determinism-critical path. Log
  gaps you cannot close in `PORT_STATUS.md`; never silently skip one.
- Before changing behavior in a determinism-critical subsystem, first freeze
  current C++ behavior with a differential test, then change.
- Determinism-critical (must be bit-exact): C4Fixed math, C4Random (incl.
  `RandomCount`), movement/physics, landscape & material reactions,
  PXS/mass-mover, weather, particles, the C4Script VM, cross-checks/OCF,
  FindObject ordering, sectors, command AI, control/record.

### Invariants already implemented — do not regress

- **Fixed-point:** `C4Fixed` is the C++ signed 16.16 type. Object position,
  velocity, and rotation are `C4Fixed`; whole-pixel coords are projections at
  interfaces that demand them. Preserve raw fixed values and C++
  rounding/wrapping everywhere. Movement diffs must compare raw `C4Fixed` —
  comparing only `fixtoi()` masks desyncs.
- **RNG:** synchronized `Random()` is the C++ LCG (`*214013 + 2531011`,
  `>>16 % range`) with `RandomHold`, unconditional `RandomCount`, and the
  `FRndBuf3`/`Rnd3` ring. `SafeRandom` and presentation randomness are separate
  unsynchronized streams; never substitute one for another.
- **Differential coverage:** `cargo xtask parity verify` diffs Rust against a
  golden generated from the pinned C++ source (scope in `parity/README.md`). It
  is not proof of full-scenario parity — extend the golden or live shadow-diff
  when changing behavior outside its coverage.

## TDD (Kent Beck) — required workflow

Red → Green → Refactor, one test at a time:

1. Write the simplest failing test pinning a small increment of C++-faithful
   behavior, citing the C++ `file:line` it mirrors in a comment. See it RED.
2. Write the minimum code to make it GREEN.
3. Refactor with tests passing. Run the full (non-long-running) suite each cycle.

Separate **structural** changes (rename/extract/move, no behavior change) from
**behavioral** ones. Never mix them in a commit; land structural first.

## Commits

- **Conventional Commits, no scope, subject line only.** No body, no footer.
  - `fix: preserve raw fixed velocity through attached movement`
  - not `fix(engine): ...`, not a multi-line message.
- Structural changes go under `refactor:`; behavioral work uses
  `feat:`/`fix:`/`perf:`/`test:`/`docs:`/`chore:`/`ci:` as appropriate.
- Only commit when all tests pass and clippy is clean. Small, frequent commits.
- This checkout is shared with concurrent sessions: run `git diff --stat` and
  stage explicit paths — never `git add -A`. Never stage `content/`.

### Local hooks

`lefthook.yml` enforces the two rules above mechanically — rustfmt on staged
`.rs` files, a subject-line-only Conventional Commit, no `content/` in the
index, and rustfmt over the `.rs` files in the commits being pushed. Bootstrap
with:

```sh
lefthook install
```

Two traps, both already hit here:

- If hooks appear to do nothing, check `git config core.hooksPath`. A stale
  absolute path silently disables every hook — and `lefthook install` will
  happily *create* that directory and install into it instead of `.git/hooks`.
- In a worktree, lefthook's own `git status --short` probe fails on the
  `content` symlink (`expected submodule path 'content' not to be a symbolic
  link`). The jobs still run correctly and the error is only noise, but it means
  lefthook cannot detect partially staged files here — so never add a job that
  rewrites files or sets `stage_fixed`.
- No hook may inspect the **working tree** as a whole. `cargo fmt --all --
  --check` on pre-push was tried and reverted: a concurrent session's dirty
  files reject your unrelated push, which just teaches everyone `--no-verify`.
  Check committed content (`git show HEAD:<file>`) over the pushed range
  instead.

`cargo dev-check` is deliberately not a hook: 187s warm, and it exits non-zero
on budget exhaustion. Run it by hand before opening a pull request.

Hooks are advisory (`--no-verify` exists, and agents drive git
non-interactively). CI remains the gate.

## Rust style

- Prefer functional combinators (`map`/`and_then`/`unwrap_or`/`ok_or`/`filter`)
  over `match`/`if let` when they read as clearly.
- Make invalid states unrepresentable: `C4Fixed` for fixed-point state, newtypes
  like `ObjectId` over raw integers, enums + traits over inheritance/RTTI,
  `#[non_exhaustive]` where the C++ enum can grow.
- No `unwrap`/`expect`/`panic!` on paths reachable from script or network input —
  return `Result`. Panics are for genuine invariant violations only.
- Match the surrounding code's naming, comment density, and idioms.

## Done = all required gates green

```sh
cargo nextest run --workspace --no-fail-fast
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

`.github/workflows/rust.yml` additionally runs `cargo fmt --all -- --check`,
`python3 -m unittest discover -s scripts/tests -p 'test_*.py'`, and
`cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked`.

These are requirements, not a claim about the current worktree — run and report
them for the revision you are completing; focused tests do not imply the gates
pass. Engine snapshots are Rust self-consistency checks; `parity verify` is the
C++↔Rust differential. Neither replaces the other. `PORT_STATUS.md` lists extra
focused gates and the narrow accepted skips.

## Test-runner gotchas

- Use `cargo nextest run`, not bare `cargo test` (no doctests exist here).
- `default-members = ["crates/clonk-app"]`, so bare cargo commands only cover
  that crate — pass `--workspace` or `-p <crate>`.
- `clonk-engine`, `clonk-frontend`, and `clonk-logging` set `test = false`; their
  inline `#[cfg(test)]` modules run from the `*-unit-tests` companion crates
  (e.g. `clonk-engine-unit-tests::engine_inline`). `-p clonk-engine` finds zero
  tests.
- `cargo fmt --all -- --check` is **not** currently clean (~851 files drift).
  Never sweep `cargo fmt --all`; format only the hunks you wrote.

## Architecture notes / gotchas

- **Two scripting paths coexist.** `clonk-engine`'s `Engine` supports a
  *command-DSL convenience*: lifecycle callbacks (`Initialize`/`Step`) may return
  a proplist of state deltas that `parse_command` (`lib.rs`) applies. That is an
  additive shortcut for the synthetic snapshot fixtures (`fixtures.rs`). **Real
  C4Script content** mutates state through host-function calls and its callback
  return values are ignored, matching C++. Do not make the engine *require*
  command-proplist returns from real content.
- `clonk-script` is the C4Script VM port (an AST tree-walk; C++ is an 84-opcode
  stack VM). `this` carries the active object/definition context. Preserve C++
  call, conversion, and callback ordering rather than relying on shape.
- `content/` is a submodule and is also the engine's data root — it is read-only
  parity input. `git submodule update --init --recursive` before scenario or
  replay work.

## Useful commands

```sh
cargo dev-check --base origin/main --budget-seconds 60   # fast change-aware loop
cargo dev-check --base origin/main --plan                # inspect without running
cargo build --workspace
cargo nextest run -p <crate>                             # focused
cargo xtask engine-snapshots record|verify               # Rust self-consistency
cargo xtask parity record|verify                         # C++↔Rust differential
```

The live C++ bridge builds in `LEGACYCLONK_ORACLE_ROOT` with
`-DUSE_RUST_ENGINE_VALIDATION` and is driven by `LC_RUST_ENGINE_*`.
