# Rust Port — Engineering Constraints & Standards

This file governs work in `rust/`. It overrides defaults. Read `PORT_STATUS.md`
for the current GAP LIST and the two foundational determinism breaks.

## Prime directive: parity before features

LegacyClonk is a **lockstep-deterministic** engine. The Rust port is worthless
unless it matches the C++ engine in `../src/` **bit-for-bit on simulation state**.

- The C++ engine in `../src/` is the **golden oracle**. When Rust and C++ differ,
  C++ is right — unless you can *prove* a C++ bug that does not affect determinism.
- **Never** make a parity failure go away by editing a test to match Rust, by
  making Rust stricter/looser than C++, or by stubbing a determinism-critical path.
  If you cannot close a gap, log it in `PORT_STATUS.md` — never silently skip it.
- Before changing or adding behavior in a determinism-critical subsystem, **freeze
  current C++ behavior with a differential test first** (same inputs through both
  engines, assert identical output), then change.
- Determinism-critical subsystems (must be bit-exact): C4Fixed fixed-point math,
  C4Random RNG (incl. `RandomCount`), movement/physics, landscape & material
  reactions, PXS/mass-mover, weather, particles, the C4Script VM, object
  cross-checks/OCF, FindObject ordering, sectors, command AI, control/record.

### Two standing parity violations (fix at the source, do not build on top)
1. **Fixed-point:** object position/velocity must become `C4Fixed` 16.16, not
   `i32`. Until then physics cannot be parity-correct. See action item 1.
2. **RNG:** `Random()` must be the C++ LCG (`*214013 + 2531011`, `>>16 % range`)
   with global `RandomHold`/`RandomCount`, not `ChaCha8Rng`. See action item 2.
   When you land this, **delete** the `random_matches_chacha_stream` anti-test
   (`compat.rs:13171`) — it currently enshrines the wrong behavior.

## TDD (Kent Beck) — required workflow

Red → Green → Refactor, one test at a time:
1. Write the simplest failing test that pins a small increment of C++-faithful
   behavior (cite the C++ `file:line` it mirrors in a comment). Run it; see it RED.
2. Write the minimum code to make it GREEN.
3. Refactor with tests passing. Run the full (non-long-running) suite each cycle.

Separate **structural** changes (rename/extract/move — no behavior change) from
**behavioral** changes; never mix them in one commit; do structural first. Only
commit when all tests pass and clippy is clean. Small, frequent commits; the
commit message states whether it is structural or behavioral.

## Rust style

- Prefer functional combinators (`map`/`and_then`/`unwrap_or`/`ok_or`/`filter`)
  over `match`/`if let` when they read as clearly.
- Model the domain so invalid states are unrepresentable: newtypes over raw ints
  (e.g. a real `C4Fixed` type, `ObjectId`, owner/material newtypes), enums + traits
  over inheritance/RTTI, `#[non_exhaustive]` where the C++ enum can grow.
- Use `cfg`/features over C++ `#ifdef`. Keep FFI (`#[cfg(feature = "ffi")]`) thin.
- No `unwrap`/`expect`/`panic!` on paths reachable from script or network input —
  return `Result`. Reserve panics for genuine invariant violations.
- Match the surrounding code's naming, comment density, and idioms.

## Done = all three green (Phase 1 definition of done)

```
cargo test --workspace
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
```

Today none of these fully hold (see `PORT_STATUS.md`): the suite has 2 failing
app-integration tests, clippy emits ~78 issues, and the snapshot harness only
checks Rust-vs-Rust self-consistency (a real C++↔Rust differential harness must be
built — `USE_RUST_ENGINE_VALIDATION` + `LC_RUST_ENGINE_*` env vars in
`../src/rust/RustEngineBridge.cpp` are the starting point).

## Architecture notes / gotchas

- **Two scripting paths coexist.** `clonk-engine`'s `Engine` runs scripts via a
  *command-DSL convenience*: lifecycle callbacks (`Initialize`/`Step`) may *return*
  a proplist of state deltas which `parse_command` (`lib.rs`) applies. This is an
  additive shortcut for the synthetic snapshot fixtures (`fixtures.rs`). **Real
  C4Script content** mutates state through host-function calls and its callback
  return values are ignored (matching C++). Do not make the engine *require*
  command-proplist returns from real content.
- `clonk-script` is the actual C4Script VM port (AST tree-walk today; C++ is an
  84-opcode stack VM). `Expr::This` currently returns `Nil` (`vm.rs:417`) — a
  major correctness bug for object-relative code.
- Snapshots/FFI carry integer positions because C++ converts via `fixtoi()` at the
  boundary; this hides the fixed-point gap. A real differential harness must
  compare **pre-conversion** fixed-point state, or it will mask desyncs.
- Stray `*.bak` files in `clonk-engine/src/` are not part of the build — ignore/remove.

## Useful commands

```
cargo build --workspace
cargo test -p <crate>                       # focused
cargo test --workspace
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots record|verify  # Rust self-consistency snapshots
cargo xtask ffi [--release] [-p <crate>]    # build staticlib/cdylib for C++
```

C++ oracle build (for differential work) lives in `../` (CMake); the bridge is
compiled with `-DUSE_RUST_ENGINE_VALIDATION` and driven by `LC_RUST_ENGINE_*`.
