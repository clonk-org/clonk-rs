# clonk-rs — Engineering Constraints & Standards

Deterministic Rust port of LegacyClonk.

## Parity constraints

LegacyClonk is **lockstep-deterministic**, so simulation state must match C++
bit-for-bit. The oracle is **not** upstream LegacyClonk: it is the instrumented
fork snapshot at commit `7d43b47b7d789b533f32d005e64596e0a07019cd`, reachable in
this repo's Git history and pinned by `parity/oracle/gen_golden.sh`. It carries
the RNG-trace hooks and extracted headers the golden generator needs, none of
which exist upstream. Set `LEGACYCLONK_ORACLE_ROOT` to a checkout of that commit
for live differential work.

- C++ is the golden oracle. When Rust and C++ differ, C++ is right — unless you
  can *prove* a C++ bug that cannot affect determinism.
- **Never** close a parity failure by editing a test to match Rust, by making
  Rust stricter/looser than C++, or by stubbing a determinism-critical path. Log
  gaps you cannot close in `PORT_STATUS.md`; never silently skip one.
- Determinism-critical (must be bit-exact): C4Fixed math, C4Random (incl.
  `RandomCount`), movement/physics, landscape & material reactions,
  PXS/mass-mover, weather, particles, the C4Script VM, cross-checks/OCF,
  FindObject ordering, sectors, command AI, control/record.
- Movement diffs must compare raw `C4Fixed` — comparing only `fixtoi()` masks
  desyncs. `SafeRandom` and presentation randomness are separate unsynchronized
  streams from `Random()`; never substitute one for another.
- `cargo xtask parity verify` is **not** proof of full-scenario parity. The
  golden is ~31 primitive sections, so a change to players, savegames or
  scenario init passes it untouched. Extend the golden when you change behavior
  outside its coverage; `PORT_STATUS.md` holds the open gaps.

## TDD (Kent Beck) — required workflow

Red → Green → Refactor, one test at a time: simplest failing test first, seen
RED; minimum code to GREEN; refactor with tests passing. Run the
non-long-running suite each cycle. A test mirroring C++ cites the C++
`file:line` it pins, in a comment.

Separate **structural** changes (rename/extract/move, no behavior change) from
**behavioral** ones. Never mix them in a commit; land structural first.

## Commits

- **Conventional Commits, no scope, subject line only.** No body, no footer:
  `fix: preserve raw fixed velocity through attached movement` — not
  `fix(engine): ...`, not multi-line.
- Structural changes go under `refactor:`; behavioral work uses
  `feat:`/`fix:`/`perf:`/`test:`/`docs:`/`chore:`/`ci:` as appropriate.
- Only commit when all tests pass and clippy is clean. Small, frequent commits.
- This checkout is shared with concurrent sessions: run `git diff --stat` and
  stage explicit paths — never `git add -A`. Never stage `content/`.
- Most worktrees symlink `content/`, so plain `git status` fails there
  (`expected submodule path 'content' not to be a symbolic link`, exit 128).
  Expected, not corruption: use index-scoped commands (`git diff --cached
  --name-only`, `git diff --stat -- crates/`). Never "fix" it by replacing the
  symlink or running `git submodule update` there — that empties `content/` and
  reds tests that then read as code faults.

### Local hooks

Run `lefthook install` once; `lefthook.yml` then enforces the rules above and
documents its own traps — read it before editing a hook. Three constraints are
not negotiable:

- No hook may inspect the **working tree** as a whole (pre-push `cargo fmt
  --all -- --check` was tried and reverted: a concurrent session's dirty files
  reject your unrelated push, which teaches everyone `--no-verify`). Check
  committed content over the pushed range instead: `git show HEAD:<file>`.
- No hook may rewrite files or set `stage_fixed`: where `content/` is a symlink
  lefthook's own `git status --short` probe exits 128, so it cannot detect
  partially staged files.
- `cargo dev-check` is deliberately not a hook — 187s warm, and it exits
  non-zero on budget exhaustion. Run it by hand before opening a pull request.

If hooks appear to do nothing, check `git config core.hooksPath`: a stale
absolute path silently disables every hook, and `lefthook install` will *create*
that directory and install into it instead of `.git/hooks`.

Hooks are advisory (`--no-verify` exists, and agents drive git
non-interactively). CI remains the gate.

## Pull requests — how work lands

`main` is protected and lands through a **merge queue**. Do not push to `main`.
Admin bypass makes it possible, which is why this is written down: at ~150
commits a day from parallel worktree sessions, a direct push can delete work
that already landed. On 2026-07-29 `fix: install Pillow for CI script tests` was
pushed off `main` within the hour by a session that had merged `main` into an
older branch; it had to be landed twice, with CI red in between.

One pull request per worktree session:

```sh
git push -u origin "$(git branch --show-current)"
gh pr create --repo clonk-org/clonk-rs --base main \
  --head "$(git branch --show-current)" --fill
gh pr merge --repo clonk-org/clonk-rs --auto "$(git branch --show-current)"
```

`--repo/--base/--head` are not optional: an `upstream` remote points at
`legacyclonk/LegacyClonk` and `gh` ranks it above `origin`, so a bare
`gh pr create --fill` opens against the wrong repository (`No commits between
legacyclonk:master and clonk-org:<branch>`). Do **not** use `gh repo
set-default` — it writes into the *shared* `.git/config` every worktree session
reads. With more than one commit `--fill` titles the pull request from the
branch slug; retitle it to a Conventional Commit subject via `gh pr edit`.

`--auto` is the point: the queue rebases your branch onto `main` plus every
entry ahead of you, runs the long gates against that result, and fast-forwards
only if they pass. Because it rebases, your commits land individually — the
structural / behavioural split survives, so keep making it. Branches are **not**
required to be up to date, so never merge `main` into your branch or rebase to
catch up; that is what pushed the 2026-07-29 fix away. Hand-rebase only when the
queue evicts you for a conflict, which it cannot resolve itself.

| | Jobs | When |
|---|---|---|
| Per pull request | formatting, workspace lints, focused feedback, macOS material-order oracles, Windows launcher and installer (+ dependency guard on manifest changes) | every push to the branch, ~14 min |
| In the queue | full parity gate, code coverage, MSVC runtime builds | up to 3 entries build at once, 1 merges at a time |

A green pull request is therefore **not** a green parity gate, and an entry can
be evicted after the pull request itself went green.

- Run the [required gates](#done--all-required-gates-green) locally *before*
  opening the pull request. The queue is a safety net, not your test runner — an
  eviction forces every entry queued behind you to rebuild.
- Expect `--no-verify` on push: `rustfmt-pushed-commits` runs rustfmt over
  `crates/clonk-app/src/main_tests/*.rs`, `include!` fragments that `cargo fmt
  --all` never descends into, so it reports drift the CI gate does not have.
  Confirm with `cargo fmt --all -- --check`, then push through it.
- Pushing straight to `main` is break-glass for a broken queue only. If you take
  it, say so in the pull request or commit that follows.

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
`python3 -m unittest discover -s scripts/tests -p 'test_*.py'`, and `cargo test
-p xtask --features engine-tools --bin xtask-engine-tools --locked`. Run all
three yourself: only the fmt check answers your pull request — the other two
first report from the queue, where a failure evicts you.

These are requirements, not a status report on this worktree — run and report
them for the revision you are completing; focused tests do not imply the gates
pass. Engine snapshots and `parity verify` check different things; neither
replaces the other. `PORT_STATUS.md` lists extra focused gates and the narrow
accepted skips.

## Test-runner gotchas

- Use `cargo nextest run`, not bare `cargo test` (no doctests exist here).
- `default-members = ["crates/clonk-app"]`, so bare cargo commands only cover
  that crate — pass `--workspace` or `-p <crate>`.
- `clonk-engine` and `clonk-frontend` set `[lib] test = false`; their inline
  `#[cfg(test)]` modules are compiled by companion crates
  (`clonk-engine-unit-tests::engine_inline`,
  `clonk-frontend-unit-tests::frontend_inline`), so `-p clonk-engine` finds zero
  tests. `clonk-logging` sets `test = false` but has **no** companion crate:
  `-p clonk-logging` runs its `tests/*.rs` binaries normally, and an inline
  `#[cfg(test)]` module in its `src/lib.rs` would silently never run — put
  logging tests in `tests/`.
- `cargo fmt --all -- --check` is clean at HEAD and is its own CI gate, so any
  drift it reports is **yours**. Never run `cargo fmt --all` or `cargo fmt -p
  <crate>` as a *fixer* — both rewrite every file in scope, including a
  concurrent session's uncommitted work. Format only what you wrote:
  `rustfmt --edition 2021 <file>`.

## Architecture notes / gotchas

- **Two scripting paths coexist.** `clonk-engine`'s `Engine` supports a
  *command-DSL convenience*: lifecycle callbacks (`Initialize`/`Step`) may return
  a proplist of state deltas that `parse_command` (`lib.rs`) applies — an
  additive shortcut for the synthetic snapshot fixtures (`fixtures.rs`). **Real
  C4Script content** mutates state through host-function calls and its callback
  return values are ignored, matching C++. Do not make the engine *require*
  command-proplist returns from real content.
- `clonk-script` is the C4Script VM port (an AST tree-walk; C++ is an 84-opcode
  stack VM). `this` carries the active object/definition context. Preserve C++
  call, conversion, and callback ordering rather than relying on shape.
- `content/` is a submodule and is also the engine's data root — read-only parity
  input. Run `git submodule update --init --recursive` before any test run, not
  just scenario or replay work: `include_bytes!`/`include_str!` sites in
  `#[test]` bodies compile game data in, so an uninitialised `content/` is a
  *compile* error in the test targets. `cargo build --workspace` does not need
  it, and build-only CI jobs deliberately skip the checkout.

## Useful commands

```sh
cargo dev-check --base origin/main --budget-seconds 60   # fast change-aware loop
cargo dev-check --base origin/main --plan                # inspect without running
cargo nextest run -p <crate>                             # focused
cargo xtask engine-snapshots record|verify               # Rust self-consistency
cargo xtask parity record|verify                         # C++↔Rust differential
```

Live full-scenario C++ shadow-diff is **not wired**: no crate exposes a C-ABI
target, so the oracle's `-DUSE_RUST_ENGINE_VALIDATION` bridge in
`LEGACYCLONK_ORACLE_ROOT` links the Rust snapshot bundled at the pinned commit,
not your tree (`parity/README.md`, "Phase 2"). The `LC_RUST_ENGINE_*` env
channel *is* live here, for seed pinning.
