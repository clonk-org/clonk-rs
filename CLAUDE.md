# Rust Port — Engineering Constraints & Standards

This file governs work in the repo root. It overrides defaults. Read
`PORT_STATUS.md` for the current focus, completion gates, and open parity gaps.

## Prime directive: parity before features

LegacyClonk is a **lockstep-deterministic** engine. The Rust port is worthless
unless it matches the C++ engine **bit-for-bit on simulation state**. The
pinned C++ source snapshot is commit
`7d43b47b7d789b533f32d005e64596e0a07019cd`, which remains reachable in this
repository's Git history. Set `LEGACYCLONK_ORACLE_ROOT` to use a separate C++
checkout for live differential work.

- The C++ engine in the oracle checkout is the **golden oracle**. When Rust and C++ differ,
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

### Foundational determinism state (implemented; do not regress)

1. **Fixed-point:** `C4Fixed` is the C++-compatible signed 16.16 type. Object
   fixed position, velocity, and rotation use it; whole-pixel coordinates are
   projections at interfaces that require them. Preserve raw fixed values and
   C++ rounding/wrapping behavior through every simulation path.
2. **RNG:** synchronized `Random()` uses the C++ LCG
   (`*214013 + 2531011`, `>>16 % range`) with `RandomHold`, unconditional
   `RandomCount`, and the `FRndBuf3`/`Rnd3` ring. `SafeRandom` and presentation
   randomness are separate unsynchronized streams; never substitute one for
   another.
3. **Differential coverage:** `cargo xtask parity verify` is a real
   C++↔Rust differential against a golden generated from the pinned C++ source.
   It covers `C4Fixed`, the LCG/Rnd3 ledger, sub-pixel accumulation, and the
   source-aligned subsystems listed in `parity/README.md`. It is not proof that
   every full scenario is at parity; extend the golden or live shadow-diff when
   changing behavior outside its current coverage.

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
- Model the domain so invalid states are unrepresentable: reuse `C4Fixed` for
  fixed-point state, prefer newtypes such as `ObjectId` and owner/material IDs
  over raw integers, use enums + traits over inheritance/RTTI, and use
  `#[non_exhaustive]` where the C++ enum can grow.
- No `unwrap`/`expect`/`panic!` on paths reachable from script or network input —
  return `Result`. Reserve panics for genuine invariant violations.
- Match the surrounding code's naming, comment density, and idioms.

## Done = all required gates green

```
cargo nextest run --workspace --no-fail-fast
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

These are requirements, not a claim about the current worktree. Run and report
the gates for the revision being completed; focused tests do not imply that the
full gates pass. `PORT_STATUS.md` lists additional focused gates and the narrow
accepted over-constraint skips. Engine snapshots are Rust self-consistency
checks, while `parity verify` is the C++↔Rust differential; neither replaces
the other.

## Architecture notes / gotchas

- **Two scripting paths coexist.** `clonk-engine`'s `Engine` runs scripts via a
  *command-DSL convenience*: lifecycle callbacks (`Initialize`/`Step`) may *return*
  a proplist of state deltas which `parse_command` (`lib.rs`) applies. This is an
  additive shortcut for the synthetic snapshot fixtures (`fixtures.rs`). **Real
  C4Script content** mutates state through host-function calls and its callback
  return values are ignored (matching C++). Do not make the engine *require*
  command-proplist returns from real content.
- `clonk-script` is the actual C4Script VM port (an AST tree-walk today; C++ is
  an 84-opcode stack VM). `this` carries the active object/definition context.
  Preserve C++ call, conversion, and callback ordering rather than relying on
  implementation shape.
- Engine state and snapshots retain raw fixed-point fields alongside
  whole-pixel projections. Differential comparisons of movement must use the
  raw `C4Fixed` values; comparing only `fixtoi()` output can mask a desync.
- The committed golden differential is already active. Full-scenario live
  shadow coverage remains incremental: use the C++ bridge for gaps that the
  source-extracted golden does not yet cover, and record the first raw-state
  divergence.

## Useful commands

```
cargo build --workspace
cargo test -p <crate>                       # focused
cargo test --workspace
cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots record|verify  # Rust self-consistency snapshots
cargo xtask parity record|verify            # C++↔Rust differential
```

For a live C++ differential build, set `LEGACYCLONK_ORACLE_ROOT` to its
checkout. The bridge is compiled there with `-DUSE_RUST_ENGINE_VALIDATION` and
driven by `LC_RUST_ENGINE_*`.
