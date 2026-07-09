# C++↔Rust Differential Parity Harness (Phase 1)

This harness verifies that the Rust port (`rust/crates/lc-engine`) reproduces the
determinism-critical C++ primitives **bit-for-bit**. It is a true *differential*
against the C++ golden oracle (`../src`), not a Rust-vs-Rust regression check
(that is `cargo xtask engine-snapshots verify`).

It exists to **gate Theme C** (wiring fixed-point precision through the physics /
collision / procedure code): the sub-pixel accumulation it covers is exactly the
arithmetic that movement physics extends, so any divergence introduced while
porting physics is caught immediately, with the first mismatch pinpointed.

## What it covers

The golden (`golden/parity_golden.json`) is generated from the **real engine
code** and the Rust side runs identical inputs and asserts byte-exact equality:

| Section | C++ source of truth | Why it matters |
|---|---|---|
| `itofix` / `fixtoi` | `src/Fixed.h` C4Fixed | velocity/gravity precision (`SetXDir`, `FIXED256`) |
| `arith` | `src/Fixed.h` operators | velocity scaling, force redirection |
| `trig` | `src/Fixed.h` + `src/Fixed.cpp` `SineTable` | rotation, `SimFlight` |
| `rng_random` | `src/C4Random.h` LCG | network sync (`RandomHold`/`RandomCount`, incl. range-0) |
| `rng_randomize3` | `src/C4Random.cpp` `FRndBuf3` | mass-mover / `Rnd3` |
| `material_corrode_rng` | `src/C4Material.cpp` corrosion branches | material reaction execution RNG ordering |
| `mass_mover_transfer_rng` | `src/C4MassMover.cpp` transfer calls | `Random(10)` before `Rnd3()` immediate-execution decision |
| `script_value_hash` | `src/C4Value.cpp` `hashCombine` / `std::hash<C4Value>` | map-key lookup for nested script values |
| `script_value_convert` | `src/C4Value.cpp:488-598` `C4ScriptCnvMap` + `ConvertTo` | type-coercion rules for `getInt`/`getStr`/… and parameter marshaling |
| `script_killer` | `src/C4ScriptKiller.h`, called by `src/C4Script.cpp:1333-1347` | GetKiller/SetKiller fallback target, player validation, direct assignment, foreign/arrow targeting |
| `movement` | `src/C4Movement.cpp:260,627` accumulation | the Theme-C core: `fix += dir`, `ydir += gravity` |

**Out of scope (Phase 2):** the C++ per-pixel collision/contact loop
(`C4Movement.cpp` `while (x != ctcox)` with `ContactCheck`/friction/redirection,
item 4) and landscape/material state. Validating those requires running the full
C++ engine on a content scenario via the `RustEngineBridge` live shadow-diff —
see "Phase 2" below.

## How the oracle stays honest

`oracle/gen_golden.sh` uses the actual engine code, not a hand-rewrite:

- `oracle_fixed.h` is **mechanically stripped** from `src/Fixed.h` (only the
  `StdCompiler`/`StdAdaptors` includes and the serialization `CompileFunc` are
  removed — the `C4Fixed` arithmetic is byte-identical).
- `SineTable` is lifted verbatim from `src/Fixed.cpp`.
- `src/C4Random.h` is included unmodified (its sole heavy include, `C4Record.h`,
  is `#ifdef DEBUGREC`, which the oracle does not define).
- `Randomize3`/`Rnd3` are reproduced verbatim from `src/C4Random.cpp` (10 trivial
  lines around the real `Random()`).
- Material corrosion and mass-mover transfer sections are small source-aligned
  RNG traces copied from the branch order in `src/C4Material.cpp` and
  `src/C4MassMover.cpp`; they intentionally avoid full engine setup while still
  pinning sync-critical `Random()` call order.
- `script_value_hash` is a source-aligned standalone copy of the small
  `hashCombine` / recursive `std::hash<C4Value>` path in `src/C4Value.cpp`.
- `script_value_convert` transcribes the 9×9 `C4ScriptCnvMap` table and the
  `ConvertTo` dispatch (`src/C4Value.cpp:431-598`) cell-for-cell — the real table
  is a private static of function pointers that cannot be linked without all of
  Game/C4Object. The oracle's copy and the Rust port's are *independent*, so a
  transcription slip on either side surfaces as a divergence. The Game-dependent
  `FnCnvGuess`/`GuessType` branch only runs for a non-zero `C4V_Any`; every input
  is a concrete type or nil, so no engine setup is needed.
- `script_killer` calls the production `C4ScriptKiller.h` helper verbatim.
  `C4Script.cpp` delegates both static engine functions to this same helper, so
  the oracle can vary context/target pointers and the player-validity predicate
  without copying the decision logic or linking the full game executable. The
  Rust checker drives its registered host functions through real C4Script calls,
  including explicit foreign and arrow targets plus a context-free call.

If a divergence is ever a *bug in the golden* rather than the Rust port, fix the
C++ source and regenerate.

## Usage

```sh
# Verify (runs in CI as part of `cargo test --workspace`):
cargo test -p lc-engine --lib parity_differential_matches_cpp_golden
#   or, via the xtask wrapper:
cargo xtask parity verify

# Regenerate the golden after changing the C++ primitives or oracle coverage
# (requires a C++20 compiler; honours $CXX, defaults to clang++):
parity/oracle/gen_golden.sh
#   or:
cargo xtask parity record
```

The Rust checker is `rust/crates/lc-engine/src/parity_differential.rs`. On any
mismatch it panics with `PARITY DIVERGENCE in <section> entry <i> field <f>:
C++ golden = <x>, Rust = <y>` — i.e. the first divergence, fully localized.

## Layout

```
parity/
  oracle/
    oracle_main.cpp     # the golden generator (emits JSON)
    gen_golden.sh       # strips src/ headers, compiles, runs -> golden
    .gen/               # generated build inputs (oracle_fixed.h, sine_table.cpp) — disposable
  golden/
    parity_golden.json  # committed C++ golden output (the oracle)
```

## Phase 2 (future): live full-scenario shadow-diff

`src/rust/RustEngineBridge.cpp` already runs the Rust engine alongside C++ each
frame (`OnFrame`, gated by `USE_RUST_ENGINE_VALIDATION`, controlled by the
`LC_RUST_ENGINE_*` env vars). To extend the differential to full scenarios
(collision, landscape, materials):

1. Fix the `ffi` cargo feature so `cargo xtask ffi` builds (`B2`); it is currently
   pre-existing-broken on unrelated fields (`ControlEvent`, `construction`,
   `messages`, `ObjectBaseGraphics`).
2. Extend the C-ABI snapshot (`LcEngineObjectSnapshot`) to carry **raw `C4Fixed`**
   `fix_x/fix_y/fix_r/xdir/ydir/rdir` (it currently transports whole pixels via
   `fixtoi`, masking sub-pixel desync).
3. Make the bridge's comparison report the **first per-field divergence** (it
   currently logs a single "parity mismatch") and dump both states.
