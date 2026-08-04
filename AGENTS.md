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

## Issues — claim one before you work it

When the task is a GitHub issue, claim it on GitHub *before* you start. At ~150
commits a day from parallel worktree sessions the alternative is two sessions
fixing the same bug, and one of them throwing the work away.

```sh
gh issue view <n> --repo clonk-org/clonk-rs --json assignees,state
gh pr list --repo clonk-org/clonk-rs --state open --search "<n>"  # already claimed?
gh issue edit <n> --repo clonk-org/clonk-rs --add-assignee @me
```

Then write `Fixes #<n>` in the **pull request body** — not in a commit, whose
subject-only rule leaves no room for a footer. That registers a closing
reference, so the issue lists the pull request under Development and GitHub
closes it when the queue lands the branch. Confirm it took with `gh pr view
<pr> --repo clonk-org/clonk-rs --json closingIssuesReferences`; the close runs
off that link, not off commit text, which matters because the queue squashes
and the pull request body never reaches `main`.

Assignee plus linked pull request is the whole mechanism. The two things that
look like alternatives are not available here:

- **Projects.** Its status field is the GitHub-native "In Progress", but the
  `gh` token in these sessions carries only `gist, read:org, repo`, and
  `projectsV2` needs `read:project` — it fails `INSUFFICIENT_SCOPES` on reads
  as well as writes. An agent cannot drive a board here.
- **A `status: in progress` label.** The repo has only GitHub's defaults plus
  the dependabot labels. A new one buys a second source of truth that must be
  cleared by hand and says less than the linked pull request already does.

Every session authenticates as the same account, so the assignee does **not**
tell two sessions apart — the linked pull request and its branch do. Read a
claim that way: assigned *with* an open linked pull request is someone working
now; assigned with no linked pull request and no recent branch is stale, and is
yours to take.

**Release a claim you are not finishing.** A worktree deleted mid-task leaves
the issue assigned forever and the next session skips it. If you stop without
landing, say so on the issue and `gh issue edit <n> --repo clonk-org/clonk-rs
--remove-assignee @me`.

**Cite issues as `owner/repo#N`** in comments, docs and `PORT_STATUS.md` — this
repository is public, a bare `#28` is ambiguous between `clonk-org/clonk-rs` and
`legacyclonk/LegacyClonk`, and only the qualified form renders as a link.
`workspace quality` greps for the bare `issue #N` spelling and for the retired
private `CLO-` tracker, whose ids no reader of this repository can resolve. It
deliberately does not reject bare `#N` in general: object and definition numbers
(`WIPF #564`, `KING #5129`) are spelled the same way.

## Pull requests — how work lands

`main` is protected and lands through a **merge queue**. Do not push to `main`.
Admin bypass makes it possible, which is why this is written down: at ~150
commits a day from parallel worktree sessions, a direct push can delete work
that already landed. On 2026-07-29 `fix: install Pillow for CI script tests` was
pushed off `main` within the hour by a session that had merged `main` into an
older branch; it had to be landed twice, with CI red in between.

**Every change ships as a pull request, and landing it is part of the task.** A
local commit that never opens one is unshipped work: it is invisible to every
other session, it is not on anyone's review queue, and the worktree it sits in
is routinely deleted. Do not stop at "committed locally" and do not wait to be
asked — once the [required gates](#done--all-required-gates-green) pass, open the
pull request and shepherd it to a merge. The only reasons to hold back are an
explicit instruction to, or work you know is incomplete; say which one applies.

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
branch slug; **retitle it to a Conventional Commit subject via `gh pr edit`.**
That title is not cosmetic: the queue squashes, so it becomes the commit subject
on `main` verbatim, and it is the only subject `git-cliff` ever reads. Because
`cliff.toml` sets `filter_unconventional = true`, a branch-slug title is dropped
*silently* — the change ships with no changelog entry and earns no version bump.
The `Pull request title` job checks this on every pull request.

`--auto` is the point: the queue squashes your branch onto `main` plus every
entry ahead of you, runs the long gates against that result, and fast-forwards
only if they pass. Branches are **not** required to be up to date, so never
merge `main` into your branch or rebase to catch up; that is what pushed the
2026-07-29 fix away. Hand-rebase only when the queue evicts you for a conflict,
which it cannot resolve itself.

The queue merges with **squash**, deliberately: it is the only method that
leaves `main` verified while keeping history linear. GitHub cannot sign a commit
it rewrites — it does not hold your key — so the previous `REBASE` method
stripped the signature off every commit it landed, and `main` accumulated 234
`Unverified` commits. Squashed commits are created and signed by GitHub's
`web-flow` key instead. Do not switch the `main merge queue` ruleset back to
`REBASE` to recover per-commit history on `main`; that trade was made knowingly
on 2026-07-30.

Keep making the structural / behavioural split anyway. It no longer survives as
separate commits on `main` — that is what was traded for verification — but
`squash_merge_commit_message` is `COMMIT_MESSAGES`, so every subject you wrote
is preserved in the body of the squashed commit, and the split is still how the
pull request is reviewed.

| | Jobs | When |
|---|---|---|
| Per pull request | pull-request title admission (+ dependency guard on manifest changes) | every push to the branch, normally under 1 min |
| In the queue | exhaustive compile-time Linux test shards, formatting/scripts, workspace lints, parity/snapshots/package tests, Windows smoke tests and the shipped MSVC runtime | 1 entry builds at a time; target at or below 5 min |
| After landing | code coverage, macOS material-order oracles and Windows release tooling | exact landed SHA; blocks releases, not the next merge |

A green pull request has passed admission, not the landing gate. The fail-closed
`Landing gate` is the sole required queue result and rejects any failed,
cancelled or unexpectedly skipped child job. An entry can therefore still be
evicted after its pull request first goes green.

- Run the [required gates](#done--all-required-gates-green) locally *before*
  opening the pull request. The queue is a safety net, not your test runner — an
  eviction forces every entry queued behind you to rebuild.
- Expect `--no-verify` on push: `rustfmt-pushed-commits` runs rustfmt over
  `crates/clonk-app/src/main_tests/*.rs`, `include!` fragments that `cargo fmt
  --all` never descends into, so it reports drift the CI gate does not have.
  Confirm with `cargo fmt --all -- --check`, then push through it.
- Pushing straight to `main` is break-glass for a broken queue only. If you take
  it, say so in the pull request or commit that follows.

### Shepherding an entry to landing

`--auto` enqueues the entry; it does not babysit it. Stay with the pull request
until it merges:

```sh
gh pr checks <n> --repo clonk-org/clonk-rs --watch     # admission
gh pr view <n> --repo clonk-org/clonk-rs --json state,mergeStateStatus,autoMergeRequest
gh run list --repo clonk-org/clonk-rs --branch "gh-readonly-queue/main/pr-<n>-<base-sha>"
```

Poll on the timings in the table above — admission inside a minute, the queue
around five. Do not spin on it faster than that.

- **`autoMergeRequest: null` means the entry is *in* the queue, not that it was
  evicted.** GitHub consumes the auto-merge request when it enqueues, so the null
  is what success looks like on the way in. Read the `Landing` run on the
  `gh-readonly-queue/main/pr-<n>-*` branch and a `removed_from_merge_queue` event
  on the pull request instead; `state: MERGED` is the only thing that means
  landed.
- An eviction is yours to fix, and it is urgent — it forces every entry queued
  behind you to rebuild. Read the failing queue job, reproduce it locally, push
  the fix, and re-enable `--auto`; the enqueue is not restored automatically. Do
  **not** merge `main` into the branch to "refresh" it. Hand-rebase only for a
  conflict the queue cannot resolve itself.
- A queue failure in code you did not touch is still yours to triage before
  re-queueing. Establish whether it is your change, a flake, or already broken on
  `main` — re-queueing blind burns a build slot for everyone.
- Report where it actually got to. "Opened the pull request" is not "landed", and
  a green pull request has passed admission, not the landing gate. If you stop
  before it merges — the queue is jammed, a failure needs a decision that is not
  yours, the session ends — say so, and say exactly what state you left it in.

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

`.github/workflows/landing.yml` additionally runs `cargo fmt --all -- --check`,
`python3 -m unittest discover --buffer -s scripts/tests -p 'test_*.py'`, and `cargo test
-p xtask --features engine-tools --bin xtask-engine-tools --locked`. Run all
three yourself: they first report against the merge-group tree, where any
failure rejects the `Landing gate` and evicts the entry.

These are requirements, not a status report on this worktree — run and report
them for the revision you are completing; focused tests do not imply the gates
pass. Engine snapshots and `parity verify` check different things; neither
replaces the other. `PORT_STATUS.md` lists extra focused gates and the narrow
accepted skips.

Green gates are what let you *open* the pull request, not the end of the task.
The change is done when it has [landed](#pull-requests--how-work-lands).

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
