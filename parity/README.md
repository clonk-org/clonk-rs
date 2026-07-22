# C++↔Rust Differential Parity Harness (Phase 1)

This harness verifies that the Rust port (`crates/clonk-engine`) reproduces the
determinism-critical C++ primitives **bit-for-bit**. It is a true *differential*
against the pinned C++ golden oracle (`vendor/legacyclonk-oracle` under the
workspace's `code/` directory), not a Rust-vs-Rust regression check (that is
`cargo xtask engine-snapshots verify`).

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
| `dig2object_rng` | complete `C4Object::DigOutMaterialCast` body | Dig2Object shape-bottom spawn and `Random(360)` plus the next 20 ledger draws |
| `material_corrode_rng` | `src/C4Material.cpp` corrosion branches | material reaction execution RNG ordering |
| `mass_mover_transfer_rng` | `src/C4MassMover.cpp` transfer calls | `Random(10)` before `Rnd3()` immediate-execution decision |
| `script_value_hash` | `src/C4Value.cpp` `hashCombine` / `std::hash<C4Value>` | map-key lookup for nested script values |
| `script_value_convert` | `src/C4Value.cpp:488-598` `C4ScriptCnvMap` + `ConvertTo` | type-coercion rules for `getInt`/`getStr`/… and parameter marshaling |
| `script_killer` | `src/C4ScriptKiller.h`, called by `src/C4Script.cpp:1333-1347` | GetKiller/SetKiller fallback target, player validation, direct assignment, foreign/arrow targeting |
| `landscape_path` | `src/C4LandscapePath.h`, called by `src/C4Landscape.cpp:890-915` | 17×15 PixCnt traversal and authoritative pixel-plane occupancy at cell edges |
| `action_direction` | `src/C4ActionDirection.h`, called by `C4Object::ExecAction`/`SetDir` | raw-C4Fixed facing, TurnAction fixed-position resync, and stale pre-transition phase ordering |
| `action_swim_direction` | `src/C4ActionDirection.h`, called by DFA_SWIM/`SetDir` | SwimAccel facing changes, TurnAction two-axis fixed-position resync, and stale Swim phase ordering |
| `action_callbacks` | `src/C4ActionCallbacks.h`, called by `C4Object::SetAction` | synchronous callback count and Start-before-End/Abort ordering |
| `connect_missing_target_removal` | mechanically extracted `C4Object.cpp` DFA_CONNECT missing-target branch | `LineBreak(true)` before `AssignRemoval`/`Destruction`, with final deleted status |
| `connect_geometry_break_removal` | mechanically extracted `C4Shape::LineConnect` vertex guard + later DFA_CONNECT break branch | zero-argument `LineBreak()` before the same removal lifecycle |
| `solid_mask_graphics` | `src/C4SolidMaskBitmap.h`, called by `C4SolidMask` | active/default graphics selection and transparent/solid mask sampling after `SetGraphics` |
| `shake_objects` | complete `C4Game::ShakeObjects` + `C4Object::Fling` bodies | master-order gates, `Random(3)`/`Rnd3()` consumption, attachment material identity, and raw Fling fallback |
| `blast_free` | complete `C4Landscape::ClearPix`, `BlastFreePix`, and `BlastFree` bodies | exact circle scan, pre-mutation material counts, duplicate-slot BlastShiftTo/DefaultMatTex byte selection, IFT preservation, and RNG order |
| `contact_action_bottom_flight` | complete bottom `DFA_FLIGHT` arm of `C4Object::ContactAction` + action helpers | the `(OCF_HitSpeed4 \|\| fDisabled)` FlatUp gate, including low-speed disabled actions |
| `contact_action_top_side_flight` | complete top/left/right `DFA_FLIGHT` arms + action helpers + unresolved-flight tail | the `(OCF_HitSpeed3 \|\| fDisabled)` Tumble gates, exact transient wall kicks, enabled Hangle/Scale controls, and final slide-free state |
| `movement` | `src/C4Movement.cpp:260,627` accumulation | the Theme-C core: `fix += dir`, `ydir += gravity` |

**Out of scope (Phase 2):** the C++ per-pixel collision/contact *detection* loop
(`C4Movement.cpp` `while (x != ctcox)` with `ContactCheck`/friction/redirection,
item 4) and evolving landscape/material state beyond the isolated `_PathFree`
and `BlastFree` fixtures above. The isolated flight `ContactAction`
transitions after detection are covered. Validating the remaining loop requires
running the full C++ engine on a content scenario via the `RustEngineBridge`
live shadow-diff — see "Phase 2" below.

## Accepted safety divergences

- **PXS `SyncClearance` gap compaction:** C++ copies a surviving chunk pointer
  downward without clearing the moved-from slot (C4PXS.cpp:406-424). If an
  empty lower chunk precedes a live one, two slots alias the same allocation;
  subsequent execution can process it twice and cleanup can `delete[]` it
  twice. Rust intentionally transfers unique ownership and clears the tail.
  Golden or live-shadow equality is therefore not expected at this undefined-
  behavior boundary; Rust's single-copy survivor order is authoritative.
- **S2 map-generator terminal parameters:** a negative Mandel alpha becomes a
  huge `uint32_t` iteration budget in C++, Gradient with `Wdt=0` performs
  integer division by zero, and Random with `alpha=-2` performs remainder by
  zero (C4MapCreatorS2.cpp:1357-1361,1422-1447). Rust bounds negative Mandel
  alpha to ten iterations, substitutes a denominator of one for Gradient's
  zero width, and returns false from Random's raw algorithm (before normal
  overlay inversion). These inputs are excluded from C++ differential runs.
  Mandel zero width or height is not excluded: its floating division is
  emulated with the same IEEE-754 inf/NaN propagation, and safe parameters
  remain formula-identical.

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
- `dig2object_rng` mechanically extracts the complete production
  `C4Object::DigOutMaterialCast` body. C++ records its `CreateObject` arguments
  and twenty following `Random` draws; Rust digs an identical one-pixel
  `Dig2ObjectRatio` material and compares the same spawn and ledger.
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
- `landscape_path` calls the production `C4LandscapePath.h` traversal used by
  `_PathFree`. Its edge-water input is the minimized Goldrush frame-143 live
  divergence; Rust runs the same density plane through a real `PixelGrid`.
- `action_direction` calls the production `C4ActionDirection.h` raw-xdir and
  TurnAction decisions used by `C4Object`. Its input is the minimized Goldrush
  frame-170 WIPF state; Rust runs the same Walk/Turn ActMap through a real
  engine frame and compares raw velocity/position plus action, facing, phase,
  and time.
- `action_swim_direction` drives the same production direction/TurnAction
  decisions with the minimized Goldrush frame-219 FISH state. Rust runs a real
  Swim/Turn ActMap frame and compares raw velocity/position plus action, facing,
  phase, and time; the decisive fixed-y snap is observable only when internal
  DFA_SWIM facing goes through `SetDir`.
- `action_callbacks` calls the production `C4ActionCallbacks.h` dispatcher
  used by `C4Object::SetAction`. Its Start-only case is the minimized Goldrush
  frame-192 WIPF double-`Sitting` divergence; real Rust script fixtures also
  cover script Start/Abort and natural Start/End ordering.
- `connect_missing_target_removal` compiles the exact production target-check
  and `if (fBroke)` block lifted from DFA_CONNECT. A minimal C++ lifecycle
  scaffold records `LineBreak(true)`, `AssignRemoval`'s `Destruction`, and final
  status. `connect_geometry_break_removal` additionally compiles the exact
  `C4Shape::LineConnect` one-vertex failure guard and the later DFA_CONNECT
  `LineBreak()`/removal block. Rust drives both through the real
  `Engine::exec_connect_line` method and inspects each deleted line before frame
  cleanup. These focused fixtures do not model the rest of `AssignRemoval`
  (contents, effects, or pointer clearing), nor LineConnect's
  landscape-dependent path and bend search after its vertex-count guard.
- `solid_mask_graphics` calls the production `C4SolidMaskBitmap.h` helpers used
  by `C4SolidMask`. Its decisive `(219,86)` input is the minimized Goldrush
  frame-184 CTWR Graphics2/SNKE contact: default graphics are transparent,
  Graphics2 is opaque. Rust runs that selection through a real mask bake and
  also tests cross-definition `SetGraphics` plus immediate remove/re-put.
- `shake_objects` mechanically extracts and compiles the complete production
  `C4Game::ShakeObjects` and `C4Object::Fling` bodies. Minimal object/action
  stubs force the raw fallback while preserving the real selection and RNG
  order; Rust drives the registered script host function over the same master
  list and compares every resulting velocity, attachment, mobile, and cause.
- `blast_free` mechanically extracts and compiles the complete production
  `C4Landscape::ClearPix`, `BlastFreePix`, and `BlastFree` bodies. A 7×7
  Surface8 fixture mixes Earth and Granite with/without IFT; Granite shifts to
  an explicit second Rock texture while Earth clears to sky or Tunnel's
  second/default texture+IFT. Rust blasts an identical real
  `PixelGrid` and compares pre-mutation `BlastMatCount`, every final byte, and
  `RandomHold`/`RandomCount`/`FRndPtr3` before and after the scan. A second
  mechanically extracted call pins the inclusive radius-zero center clear.
- `contact_action_bottom_flight` mechanically extracts the complete first
  `DFA_FLIGHT` switch arm from `C4Object::ContactAction` and the production
  `ObjectActionWalk`, `ObjectActionKneel`, and `ObjectActionFlat` helpers. Its
  three-case OR-gate matrix proves that low-speed `ObjectDisabled=1` reaches
  FlatUp exactly like `OCF_HitSpeed4`, while enabled low-speed flight walks.
  Rust drives the matching ActMaps through `Engine::exec_contact_action` and
  compares action, direction, and raw fixed velocities.
- `contact_action_top_side_flight` mechanically extracts the ceiling and both
  wall `DFA_FLIGHT` arms, `ObjectActionTumble`/`Scale`/`Hangle`, and the shared
  unresolved-flight tail. Enabled controls enter Hangle/Scale; matching
  low-speed disabled cases enter Tumble, bypass those fallbacks, and compare
  the pre-tail raw Tumble velocity plus final action, direction, position, and
  raw fixed velocity after slide-free.

If a divergence is ever a *bug in the golden* rather than the Rust port, fix the
C++ source and regenerate.

## Usage

```sh
# Verify:
cargo nextest run -p clonk-engine-unit-tests --test engine_inline \
  -E 'test(parity_differential_matches_cpp_golden)'
#   or, via the xtask wrapper:
cargo xtask parity verify

# Regenerate the golden after changing the C++ primitives or oracle coverage
# (requires a C++20 compiler; honours $CXX, defaults to clang++):
parity/oracle/gen_golden.sh
#   or:
cargo xtask parity record
```

The generator defaults to the pinned sibling checkout at
`../../vendor/legacyclonk-oracle` relative to this repository and archives the
`oracle-src-pinned` tag into its disposable `.gen` directory before extraction
and compilation. Set `LEGACYCLONK_ORACLE_ROOT` to use another repository, or
`LEGACYCLONK_ORACLE_REVISION` for an intentional source revision override.

The Rust checker is `crates/clonk-engine/src/parity_differential.rs`. On any
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
