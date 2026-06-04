# LegacyClonk Rust Port — Status & GAP LIST

> Living document. Last updated 2026-06-04 (item 5: `C4LSectors`/`C4LArea`
> infrastructure and current consumers are wired in Rust: `AtObject`,
> sector-backed bounded `FindObject`/`FindObjects`/`ObjectCount`, and the
> existing collection cross-check slice now use the 50x50 sector lists).
> The C++ engine in `../src/` is the **golden oracle**. Parity = bit-for-bit
> match on simulation. This file tracks every divergence from that goal.

## TL;DR — the port is broadly scaffolded but **not lockstep-parity-capable today**

The Rust port reproduces the *shape* of the engine (structs, enums, command
dispatch, FFI bridges, ~1150 tests) but **two foundational, determinism-critical
subsystems were implemented with entirely different algorithms than C++**, which
makes bit-for-bit lockstep parity architecturally impossible until they are
replaced:

1. **Fixed-point math is now partially introduced, but not fully propagated.** C++ stores object position/velocity as
   `C4Fixed` 16.16 fixed-point (`fix_x, fix_y, xdir, ydir`; `src/Fixed.h:46-219`)
   and accumulates sub-pixel fractions every frame (`src/C4Movement.cpp:627`). As
   of 2026-06-03, Rust has `C4Fixed`/`FixedVec2` in `lc-engine/src/math.rs`, private
   fixed position/velocity storage on live `Object`s, and the central object motion
   step accumulates fixed velocity before projecting snapshots to integer pixels.
   As of 2026-06-03, the **script velocity surface** (`SetXDir`/`SetYDir`/
   `GetXDir`/`GetYDir`) now carries true sub-pixel `C4Fixed` end-to-end:
   `SetXDir(n[,prec])` stores `itofix(n, prec)` (default precision 10) and
   `GetXDir([prec])` returns `fixtoi(xdir, prec)`, plumbed through a new
   `fixed_velocity` field on `ObjectUpdate`/`ObjectDelta` that `apply_delta`
   treats as authoritative (deriving the integer mirror via `fixtoi`). This
   removed the prior 10× desync where the script "tenths" convention was
   misread as whole pixels by the physics layer.
   The **snapshot/save-load round-trip now preserves sub-pixel** too:
   `ObjectSnapshot` carries optional raw `C4Fixed` `fixed_position`/`fixed_velocity`
   (emitted only when they differ from `fixtoi`, so whole-pixel objects are
   unchanged), `Object::snapshot()` records them and `restore_state` reconstructs
   from them — mirroring C++ persisting both `x` and `fix_x`. Verified for the
   in-memory (`restore_snapshot`) and JSON (`capture_state`→`restore_state`) paths.
   **Rotation velocity is now implemented:** live `Object`s carry `rdir`
   (`rotation_velocity`) and the `fix_r` accumulator (`fixed_rotation`), the
   motion step applies `fix_r += rdir * 5` with (-180°,180°] wraparound
   (`C4Movement.cpp:373-436`), `SetRDir`/`GetRDir` store/read `rdir` via
   `itofix(n,prec)`/`fixtoi(rdir,prec)`, and `rdir`/`fix_r` survive save/restore.
   **Post-Theme-C integration is now done for fixed-state transport and rotation
   gating:** raw `C4Fixed` position/velocity/rotation state (`fix_x`, `fix_y`,
   `xdir`, `ydir`, `fix_r`, `rdir`) crosses the C ABI through
   `LcEngineObjectSnapshot`/`LcEngineRuntimeObjectState` and
   `RustEngineBridge.cpp`; `DefCore.txt` `Rotate=` populates `Definition::rotateable`;
   `OCF_Rotate` is set for rotateable definitions; non-rotateable definitions zero
   `rdir`/`fix_r`; and finite `Def->Rotateable > 1` ranges clamp `fix_r` and stop
   `rdir`. **Item 4 is now materially advanced:** central object movement loads
   DefCore vertices/contact metadata, ActMap `Attach`, shape/vertex contact checks,
   force redirection/contact friction, `Shape.Attach` re-snapping, attach-loss action
   transitions (`NoAttachAction`-style Jump/default fallback), border bounds, and
   per-degree rotation rollback for the current landscape density model. **Still
   open:** layer bounds/solid masks, full `UpdateShape`/construction-owned vertices,
   contact callbacks, exact material/vehicle density providers, and sector/object
   contact.

2. **The RNG was the wrong algorithm; the engine RNG is now the C++ LCG.** C++ uses an LCG
   (`RandomHold = RandomHold*214013 + 2531011; (RandomHold>>16) % range`,
   `src/C4Random.h:52-60`) with global `RandomHold`/`RandomCount` state used for
   network sync verification. As of 2026-06-03, `lc-engine` uses `LcgRng`
   (`hold`, `count`, serialized with snapshots/state), script `Random()` consumes
   the C++ formula, engine seeding performs `FixedRandom(seed); Randomize3();`,
   and the old ChaCha proptest was replaced with an LCG parity test.

Until the remaining item-1 movement/physics integration is complete, no
physics-bearing scenario can stay in sync with C++ for more than a few frames.

## Session 2026-06-04 (cont.) — C4Script VM operator parity (item 8)

Five concrete, bit-exact divergences in the `lc-script` expression evaluator were
found by reading `src/C4AulExec.cpp` directly and fixed via TDD (RED test citing
the C++ line → minimal GREEN → full-suite regression run). Each is a foundational
operator that affects *all* script content, so these were desyncing the VM on very
common expressions. Baseline before/after: `cargo test --workspace` green (1217
pass + the documented flaky `lc-network` smoke test), `parity verify` and
`engine-snapshots verify` green.

| Fix | C++ oracle | Was (Rust) | Now (Rust) | Test |
|---|---|---|---|---|
| `x / 0` and `x % 0` yield **0** | `C4AulExec.cpp:504-507,523-526` (`pPar1->Set0()`) | threw `"division by zero"` / `"modulo by zero"` → aborted the script | returns `Int(0)` | `test_division_by_zero.rs` |
| `&&` / `\|\|` return the **operand** value, not a bool | `C4AulExec.cpp:999-1021` (`AB_JUMPAND`/`AB_JUMPOR` leave the operand on the stack) | returned `Bool(...)` → wrong when result flows into arithmetic | returns the surviving operand unchanged | `test_logical_operand_return.rs` |
| binary int ops coerce `nil→0`, `bool→0/1` | every op reads operands via `_getInt()` under `CheckOpPars<C4V_Any,...>`; `C4Value.h:170` (Int/Bool share `Data.Int`, nil's data is 0) | required both operands to be `Int`, else type error | `Value::as_c4_int()` coerces nil/bool; `+`,`-`,`*`,`/`,`%`,`&`,`\|`,`^`,`<<`,`>>`,`<`,`<=`,`>`,`>=` all use it | `test_int_coercion.rs` |
| unary `-` / `~` coerce `nil`/`bool` | `C4AulExec.cpp:460-470` (`SetInt(-_getInt())`, `SetInt(~_getInt())`) | required `Int`, else type error | coerce via `as_c4_int()`; `-` uses `wrapping_neg` (C++-faithful on `i32::MIN`) | `test_int_coercion.rs` |
| `this` yields the current object | C4Script `this` → `C4V_C4Object` (the object the function runs on) | `Expr::This` hardcoded `Nil` → scripts branching on `this` took the wrong path | VM threads a host `this` value (`Vm::with_this`); engine passes `object_reference_value(object_id)` at all 8 object-context call sites | `test_this_context.rs` (lc-script), `object_function_this_is_the_current_object_not_nil` (lc-engine) |
| non-nil string/array/proplist are **truthy** even when empty | `C4Value.h:185`→`:76` (`operator bool` is raw-pointer-nonzero) | `as_bool` used `!is_empty()` → empty `""`/`[]`/`{}` were falsy | `as_bool` returns `true` for any String/Array/Proplist; nil and int/bool 0 stay falsy | `test_truthiness.rs` |
| `==`/`!=` honor the script's `#strict` level | `C4Value::Equals` `C4Value.cpp:823` (NONSTRICT/STRICT1 raw, STRICT2 cross-type numeric, STRICT3 type-checked) | VM ignored `#strict`, always type-checked | `Function` carries its `#strict` level (threaded via `Environment`); `<strict 3` compares Int/Bool/nil by integer value (`0==nil`, `1==true`) | `test_strict_equality.rs` |
| `..` / `..=` concatenation operator | `AB_Concat`/`AB_ConcatIt` `C4AulExec.cpp:594-657`, priority 10 | lexer rejected `..` as an error → content using it failed to parse | lexer emits `Concat`/`ConcatEqual`; parser adds a precedence level between equality and comparison; VM joins string forms (`5..3`=="53"), appends arrays, merges maps | `test_concat_operator.rs` |
| call-depth limit raised 64 → **512** | `MAX_CONTEXT_STACK=512` `C4AulExec.cpp:62` | Rust limit was 64 → scripts recursing 65-511 deep errored where C++ runs them | `MAX_CALL_DEPTH=512`; `stacker::maybe_grow` grows the native stack on demand (this tree-walker uses ~10 KiB/level, overflowing a 2 MiB thread at ~200 without it) | `test_call_depth.rs` |

`Value::as_c4_int()` (in `lc-script/src/value.rs`) is the shared `_getInt()` mirror:
`Int→i`, `Bool→0/1`, `Nil→0`, and `None` for String/Array/Proplist (which have no
deterministic integer value in C++ — their `Data` is a pointer). Integer division
now uses `wrapping_div`/`wrapping_rem` to match C++ 2's-complement wrap instead of
panicking in debug builds.

**Found but deferred (logged, not yet fixed):**
- ~~**Truthiness of empty containers diverges.**~~ **FIXED 2026-06-04** (see fixes
  table above): `as_bool` now returns `true` for any non-nil String/Array/Proplist,
  matching C++ `operator bool` (`C4Value.h:185`→`:76`). All 9 `as_bool` call sites are
  C4Script value-truthiness checks; full suite green, no regressions.
- ~~**`==`/`!=` are strict-level-dependent in C++.**~~ **FIXED 2026-06-04** (see
  fixes table above): each `Function` now carries its script's `#strict` level
  (stamped in `Script::from_ast`, threaded through the `Environment`), and
  `values_equal` applies it — `#strict 3` is type-checked, lower levels compare
  Int/Bool/nil by integer value (`0==nil`, `1==true`, `0==false`). The Rust `Value`
  is a value type with no pointer identity, so NONSTRICT/STRICT1/STRICT2 collapse to
  one "lenient" rule; the only unreachable case is NONSTRICT array/map *identity*
  comparison. Full suite green, no regressions. (Also fixed a latent bug: `C4Id ==
  C4Id` previously fell through to `false`.)
- ~~**`..` string-concat operator is unsupported.**~~ **FIXED 2026-06-04** (see
  fixes table above): `..`/`..=` now lex, parse (priority 10, between equality and
  comparison), and evaluate (string-join / array-append / map-merge). `+` still also
  concatenates strings as a lenient extension (C++'s `+` is integer-only); that
  residual `+`-leniency is harmless for valid content (which uses `..`) and left as-is.
- **Particle host functions / physics (from the port-fidelity workflow).**
  `CastParticles`/`CastBackParticles`/`PushParticles` are unregistered (calling them
  errors) — C++ creates `iAmount` particles and consumes RNG draws (`C4Particles.cpp:421-443`,
  `C4Script.cpp:4881-4923`), so both the particles and the `Random`/`RandomCount`
  stream desync. `ActiveParticle::tick` (`lib.rs:826-835`) is `pos+=vel`, life
  countdown only — no `fxStdExec` gravity/wind-drift/alpha-fade (`C4Particles.cpp:646-671`),
  no `fxSmoke*` (`:521-576`), no per-def data model. These remain `large`-effort and
  are already tracked under the `particles` GAP row (item 12).

## Phase 0 ground-truth: build / test / lint / snapshot reality

Measured 2026-05-30 on a clean `master` (commit `f0c9f7d0`). This **corrects** the
mission brief's "all green / ~1153 tests pass" claim — the suite did not even
compile on arrival.

| Gate | Brief claimed | Actual (arrival) | After this session |
|---|---|---|---|
| `cargo build --workspace` | ✅ | ✅ (5 warnings) | ✅ |
| `cargo test --workspace` | ✅ ~1153 pass | ❌ **did not compile** (then 3 runtime failures + 1 infinite-loop hang once it did) | ✅ **green: 1099 pass, 0 fail** (cargo exit 0) |
| `cargo xtask engine-snapshots verify` | ✅ parity harness | ✅ but **trivial** (3 synthetic Rust-vs-Rust scenarios, 6/6/8 frames) | ✅ (Rust-vs-Rust regression; real C++↔Rust differential now exists: `cargo xtask parity verify`) |
| `cargo clippy --workspace -- -D warnings` | ✅ | ❌ **78 warning/error lines** | 🔶 in progress |

**Known flaky test:** `lc-network` `session::tests::control_sync_and_reconnect_smoke`
passes 10/10 in isolation and on clean full runs, but can intermittently fail
under heavy *parallel* load. Root cause: when the first client calls `shutdown()`,
its closing socket can surface a transient `HostEvent::TransportError` on the host
stream, and `wait_for_host_ready` (`session.rs:1264`) treats *any* transport error
as fatal. This is network-I/O test churn, not a simulation-determinism issue.
Recommended fix: have the `wait_for_*` helpers tolerate (skip) transport errors
from departing clients the way they already skip `ClientLeft`. Logged, not yet
applied.

**Why the test suite was broken on arrival:** the most recent commit
(`e94e5052`, "Construction() + per-object local variable persistence") added a
`local_vars` field to `ObjectSnapshot` but did not update 5 test-fixture
construction sites (`lc-frontend/src/lib.rs:3395`, `lc-app/src/main.rs:9290/9609/9640`,
`lc-app/src/object_menu.rs:1130`). This means the recent commits were pushed
**without running the test suite**, violating the repo's own commit discipline.
Fixed this session.

**Two harnesses now exist; understand the difference.**
`cargo xtask engine-snapshots verify` runs Rust scenario generators and compares
their output against **Rust-recorded** baselines — a determinism *regression*
check, not a parity check.

**Phase 1 differential harness — BUILT (2026-06-03).** `cargo xtask parity verify`
(also `cargo test -p lc-engine parity_differential_matches_cpp_golden`) is a real
C++↔Rust *differential*: it diffs the Rust port's `C4Fixed` math, the LCG RNG
(`Random`/`RandomCount` incl. range-0, `Randomize3`/`Rnd3`), `Sin`/`Cos`, and the
per-frame sub-pixel **accumulation** (`fix += dir`, `ydir += gravity`) byte-for-byte
against a golden generated from the **real** engine code (`src/Fixed.h`,
`src/Fixed.cpp`'s `SineTable`, `src/C4Random.h`). See `parity/README.md`. On
divergence it reports the first mismatch (section/entry/field/values). A negative
control confirms it fails on a corrupted golden. **This gates Theme C.** Regenerate
the golden with `cargo xtask parity record` (or `parity/oracle/gen_golden.sh`).

**Phase 2 (still open) — live full-scenario shadow-diff.** The per-pixel collision
loop, landscape, and materials are not yet covered. The
`USE_RUST_ENGINE_VALIDATION` bridge (`src/rust/RustEngineBridge.cpp`) now compiles
with the `ffi` feature and carries raw `C4Fixed` object state across the C ABI, but
still needs full-scenario shadow execution and per-field divergence reporting. See
`parity/README.md` §Phase 2.

## Changes made this session (Phase 0)

Every item below was a breakage left by an unvalidated recent commit (the suite
did not compile, and once it did, contained a hang and runtime failures). Each
fix was checked against the C++ oracle where behavior was involved.

1. **Fixed test-suite compilation** — 5 `ObjectSnapshot { … }` fixtures missing
   the new `local_vars` field (`lc-frontend/src/lib.rs`, `lc-app/src/main.rs` ×3,
   `lc-app/src/object_menu.rs`). Structural.
2. **`Initialize`-return parity regression** (commit `e94e5052`): `parse_command`
   rejected non-proplist returns from `Initialize`/`Step`
   ("expected proplist or nil, got int"), breaking **all real Clonk content**
   whose `Initialize` returns an int. C++ (`src/C4Object.cpp:1483`) calls
   `Call(PSF_Initialize)` as a bare statement and **discards** the return. Aligned
   Rust to ignore non-command returns. RED→GREEN test
   `initialize_returning_non_proplist_is_ignored_like_cpp` (`lc-engine/src/lib.rs`).
3. **Infinite-loop test hang** — `acquire_attaches_to_grabbable_container`
   (`lc-engine/src/command.rs`) looped forever on `!result.operations.is_empty()`,
   but `CommandStack::step` drains `result.operations` (applies them internally,
   the correct C++-like design), so the condition was never true. The underlying
   behavior was correct (the Get for the contained item *is* pushed). Rewrote the
   test to inspect stack state via `stack.snapshot()` with a bounded frame guard —
   stronger coverage, no hang. This had been blocking the entire suite from
   completing.
4. **Boot/scenario state-machine stranding bug** (`lc-app/src/main.rs`
   `poll_boot_loading`): a scenario started before async boot finished was
   stranded in `Loading` forever, because boot completion unconditionally flipped
   `mode → Menu` and the `Menu` update arm never polls scenario loading. Now boot
   completion yields to an in-flight scenario load. Real bug, not just a test
   issue.
5. **`host_function_registration_matches_expected`** — 12 host functions added by
   recent commits (`Abs, BoundBy, Cos, GameCallEx, GetID, GetPhase, Max, Min, Pow,
   SetPhase, Sin, Sqrt`) were registered but missing from the test's expected
   checklist. Updated the checklist (registration is intended).
6. **2 app-integration tests** (`menu_music_runs_in_menu_cycle`,
   `quick_save_persists_across_sessions`): added a `wait_for_menu` pump so the
   async-boot architecture settles before assertions — mirrors the existing
   `wait_for_running` helper.
7. **Removed 7 stray `*.bak` files** from `lc-engine`, `lc-gui`, `lc-core`.

## Known stray / cleanup items

- `lc-engine/src/compat.rs.bak` (541 KB) and `lc-engine/src/mass_mover.rs.bak` —
  leftover backup files in the source tree; should be deleted (not part of build).
- `random_matches_chacha_stream` proptest — resolved 2026-06-03; replaced with
  `random_matches_cpp_lcg`.
- 117 silent stubs catalogued below (functions that return plausibly but skip
  core C++ logic).

## Test suite: GREEN

`cargo test --workspace` → **1099 passed, 0 failed** (cargo exit 0). All arrival
breakages (above) are resolved. The only residual risk is the documented flaky
network smoke test, which passes in isolation and on clean full runs.

## Clippy: NOT clean — deferred bulk fix (criterion #3)

`cargo clippy --workspace --all-targets` emits **~275 lint lines**. Breakdown:
- **7 errors** `clippy::not_unsafe_ptr_arg_deref` — public FFI functions deref raw
  pointers without `unsafe`. Real; should be the first clippy fix (mark the FFI
  entry points `unsafe fn` — does not change the C ABI).
- ~30 `too_many_arguments` (engine/FFI functions with >7 args) — need
  `#[allow(...)]` or refactor.
- ~6 `type_complexity`, plus ~230 auto-fixable style lints
  (`field_reassign_with_default` ×31, `get_first` ×21, `unwrap_or_else`→default
  ×20, `derivable_impls` ×15, `collapsible_if` ×13, manual char comparison ×10, …).

**Why not auto-fixed this session:** a large share of these lints are in
determinism-critical `lc-engine`/`lc-script`, and several clippy "simplifications"
(`manual_range_contains`→`contains`, clamp-like→`.clamp()`, `if_same_then_else`,
`match_like_matches_macro`) are **not guaranteed behavior-preserving** at edge
cases. Per the CORE PRINCIPLE, determinism-critical behavior must not be changed
without a differential harness to prove equivalence — and that harness does not
yet exist (Phase 1). Recommended order: (1) the 7 FFI `unsafe` errors; (2)
`#[allow(clippy::too_many_arguments)]` / `type_complexity` on the engine/FFI
signatures; (3) bulk `clippy --fix` on the **non-determinism-critical** crates
(launcher/gui/frontend/app/network/audio/graphics/platform/resources/core) with a
full `cargo test` after; (4) `lc-engine`/`lc-script` lints by hand, each verified.

## Graphical parity (empirical, via computer-use, 2026-05-30)

Ran the C++ engine (`build/clonk.app`, fullscreen) and the Rust port
(`lc-app`, windowed) side by side and screenshotted both. **Graphical parity is
NOT achieved** — confirming the code-audit graphics gap empirically:

- **Asset loading/2D blit: parity.** Both render the same `Graphics.c4g` assets —
  the Goldmine loader background and the "Legacy CLONK" logo blit correctly in the
  Rust app.
- **Menu chrome: divergent.** C++ renders large full-width wooden-plank buttons
  ("Start Game", "Start Network Game", …). The Rust port renders small buttons
  inside a blue-bordered dialog panel pinned to the far right, with different
  labels ("Local Game", "Network Game", …). Different layout, sizing, framing.
- **Scenario browser: divergent.** C++ "Start Game" opens a richly GL-rendered 3D
  book (parallax background, animated hands holding the book, paper pages). The
  Rust port has no such chrome (flat 2D — consistent with `lc-graphics` having no
  transforms/GL; see the graphics row in the GAP LIST).
- **In-game (landscape/objects/particles): not yet captured live.** The C++ build
  here is x86_64 (Rosetta) and its scenario list was empty (only `Graphics.c4g`/
  `System.c4g` were linked, not the `content/*.c4f` scenarios). Driving the Rust
  window into a scenario is blocked because `lc-app` is a non-bundled winit binary
  that the computer-use layer cannot grant/drive (no input injection). Per the
  code audit, Rust in-game rendering is ~25% (per-pixel blit only; no landscape
  rendering, object transforms, shaders, or rotation matrices), so in-game parity
  is not expected. To capture it live: link `content/` scenarios for the C++ app,
  and either bundle `lc-app` as a `.app` (so computer-use can drive it) or drive
  the Rust window by hand.

## Parity divergences confirmed by the verification pass

- **C4Script has no general comma operator.** The Rust parser (`lc-script` `parse_comma`)
  accepts comma sequences in any expression context (`var x = (1, 2)`,
  `if ((a, b))`, `while ((a, b))`). C++ does NOT: its `(...)` parser
  (`src/C4AulParse.cpp:2933`) reads exactly one expression; a comma-sequence is
  only legal inside a `return (...)` statement via the `multi_params_hack`
  (`src/C4AulParse.cpp:2069`), and `,` is absent from `C4ScriptOpMap`. Several
  `test_comma_operator.rs` tests therefore pin non-C++ behavior as "valid". This
  predates this session's work; the determinism-critical risk is low (it only
  *accepts* more than C++, and real content uses the `return (...)` form which is
  legal in both), but it should be narrowed to C++ semantics. Tracked here.

---

<!-- The GAP LIST below was produced by a 26-agent parallel audit (workflow
gap-list-analysis, 2026-05-30): each agent read the C++ source and the Rust
equivalent for one subsystem and reported coverage, parity risks, and silent
stubs. The two headline findings (fixed-point, RNG) were independently verified
by hand. -->


The LegacyClonk C++→Rust port is **broadly scaffolded but still determinism-broken**. Of 26 subsystems assessed, **24 are determinism-critical** and only the explicitly listed parity harnesses are verified bit-exact. The original 2026-05-30 audit identified fixed-point math and RNG as the two headline blockers; both have since been partially addressed, but the stateful core that lockstep multiplayer and replay depend on still has major gaps, especially the full inter-object `CrossCheck` behaviors, script values, particles, and the full movement/contact loop.

**The single biggest remaining numeric risk is incomplete propagation of C++'s fixed-point movement loop, not the absence of `C4Fixed` itself.** C++ stores `fix_x`, `fix_y`, `xdir`, `ydir`, `fix_r`, and `rdir` as 16.16 `C4Fixed` values (`Fixed.h:46-219`) and performs per-pixel/per-degree contact-aware movement (`C4Movement.cpp`). Rust now has `C4Fixed`/`FixedVec2`, live fixed position/velocity, script velocity precision, snapshot/save-load preservation, raw C-ABI fixed-state transport, rotateable gate/clamp handling, and a contact-aware shape movement slice for the current landscape density model. The remaining parity blocker is the unported edge behavior around layers/solid masks/action transitions/callbacks plus downstream object/particle systems.

## Determinism-Critical Gaps

Sorted worst-first (missing → stub → partial; within partial, by parity severity).

| Subsystem | Coverage | Key Parity Risk | Rust Location |
|---|---|---|---|
| **sectors-regions-rect** | partial | `C4LSectors`/`C4LArea` infrastructure now exists as `sector.rs`: 50x50 point-sector and shape-overlap lists, `SectorAt()` out-sector behavior, C++-style `C4LArea::Next()` row/pitch iteration including clipped/out-sector edge cases, and Engine membership rebuild/update on landscape set, spawn, restore, movement, script updates, callbacks, and deletion. Current consumers are wired: live `AtObject()` uses point-sector traversal with shape/OCF/exclusive checks; bounded legacy `FindObject`/`FindObjects`/`ObjectCount` use sector candidates with linear fallback; the existing collection cross-check slice uses object-sector area candidates. Broader `C4Region` UI/input rectangles remain absent, and full C++ `CrossCheck()` gameplay is tracked under `objects-core`. | `sector.rs`, `lib.rs`, `compat.rs` |
| **script-values** | **stub (98 LOC vs 2907)** | No reference semantics (`FirstRef/NextRef/AddDataRef`), no `C4ScriptCnvMap` 81-element conversion table, hash derived naively instead of boost-style `hashCombine` (breaks map keys), no `GuessType()`, no string interning. FFI silently drops Array/Proplist (`ffi.rs:157-158`). Save/load + network sync broken. | `lc-script/src/value.rs` |
| **particles** | **stub (420 LOC vs 808)** | `ActiveParticle::tick()` (`lib.rs:812-817`) is `pos += vel; life -= 1` only. No gravity, wind, collision, alpha fade, animation cycling, or `SafeRandom` variation. `Cast()`, `Push()`, all `fx*` procs, `C4ParticleDef::Load()` absent. Any particle scenario desyncs. | `lib.rs:669-860, 12136-12547`; `compat.rs:8355-8539` |
| **findobject-ocf** | **stub (35%, 280 LOC vs 956)** | No `CreateByValue()` condition-tree factory (nested `C4FO_And/Or/Not` fail silently), no `C4SortObject` framework (`C4SO_Random/Speed/Mass/Value` unsorted → desync), no `C4FO_AtRect`/`UseShapes()` condition-tree traversal beyond the legacy host-function rectangle path, and `find_next`/sorted iteration still lacks the full C++ object-list/sort semantics. | `compat.rs:1667-1835, 6784-6931`; `ocf.rs` |
| **fixed-point-math** | partial | `C4Fixed`/`FixedVec2`, `itofix`/`fixtoi`, Sin/Cos, script velocity precision, snapshot/save-load preservation, raw C-ABI transport, rotateable gate/clamp, and current shape movement contact stepping are implemented for current object paths. Remaining risk is incomplete propagation into every legacy subsystem and unported movement edge branches. | `lib.rs`, `math.rs`, `ffi.rs`, `RustEngineBridge.cpp` |
| **movement-physics** | partial | Central motion now accumulates sub-pixel fixed velocity and steps x/y per pixel. For the current landscape density model it consumes DefCore vertices/contact metadata and ActMap `Attach`, runs shape/vertex `ContactCheck`, applies redirect/friction, clamps border bounds, supports `Shape.Attach`, checks attachment even without momentum, forces a Jump/default action on attach loss, and rolls back per-degree rotation on contact. Still missing layer bounds/solid masks, full `UpdateShape`/construction-owned vertices, contact callbacks, exact material/vehicle density providers, and sector/object contact. | `lib.rs` |
| **rng-c4random** | partial | Engine/script RNG now uses the C++ LCG with `RandomHold`, `RandomCount`, `Randomize3`/`Rnd3`, `FixedRandom`, and `SeededRandom` for current callers. Remaining risk is unported consumers such as `SafeRandom`-using subsystems and full network sync-check integration. | `rng.rs`, `compat.rs`, `mass_mover.rs` |
| **script-vm-aul** | partial (5254 LOC vs 13673) | `Expr::This` returns `Nil` unconditionally (`vm.rs:417`) — breaks ALL object-relative code. Div-by-zero throws in Rust vs silent-0 in C++ (`vm.rs:647`). Stack limit 64 vs C++ 512/1024 (8× smaller, exhaustion risk). AST tree-walk vs 84-opcode stack VM; reference semantics are string-mangled stubs (`__local_*`). | `lc-script/src` |
| **objects-core** | partial | `AtObject()` helper exists, and the current collection auto-check now uses sector area candidates. The full `CrossCheck()` (919 LOC inter-object collision loop with Tick3/5/10/35 incineration/fight/collection/hit-damage behavior) is still absent. OCF compute ~8 checks vs C++ ~30 (`ocf.rs:46-76` vs `lib.rs:527-666`), and object-list sorting still differs (Vec vs category/ID-sorted). | `lib.rs`, `ocf.rs`, `compat.rs` |
| **game-control-record** | partial (35%) | No frame-delta varint encoding (`C4Record.cpp:243-264`); JSON full-snapshots instead. No `ControlRate`/`ControlTick` throttling, no `SyncRate` periodic sync-check state machine, no record end-marker `+37` offset (`C4Record.cpp:196`). Control applied without `Prepare()` pre-validation. | `lib.rs`, `control.rs`, `record.rs`, `ffi.rs` |
| **material** | partial (40%) | Reaction *execution* layer entirely missing: `mrfInsertCheck` splash/slide physics (`C4Material.cpp:570-604`), `mrfCorrode` dual `Random(100)` calls (affects RNG sequence), `mrfPoof`. `MaterialReactionKind` enum classifies but never executes (`material.rs:722-767`). No `ExtractMaterial/InsertMaterial`. | `lc-engine/material.rs`, `lc-resources/material.rs` |
| **pxs-massmover** | partial (296 LOC vs 691) | Mass mover silently omits corrosion reactions (`C4MassMover.cpp:127`), `LandscapeInsertThrust` (`:140-151`), `Random(10)` pixel-vs-material choice (`:144`). PXS uses `first_collision_on_line` vs step-by-step `_PathFree`. Execution order reversed (C++ reverse-iterate vs Rust forward swap_remove). | `mass_mover.rs`; `lib.rs:12211-12450+` |
| **landscape** | partial (25%) | Batch `apply_temperature_conversions` vs C++ incremental stateful `ExecuteScan/DoScan` with `ScanX` cursor (scanning order desyncs). No `PRETTY_TEMP_CONV` neighbor validation. No map creation (`ChunkyRandom`, `MapToLandscape`), no `DigFree/BlastFree`, no pixel ops, no Save/Load. Liquid model incompatible (segment vs pixel). | `landscape.rs`, `material.rs` |
| **effects** | partial (35%, 195 LOC vs 921) | Builtin fire effect (300+ lines: particles, rotation, content ejection) missing. Helper effects (Splash/Smoke/Explosion/BubbleOut) missing. No `Check()` priority-conflict, no `TempRemove/TempReadd`. `advance_tick()` uses saturation arithmetic vs C++ modulo `iTime % iIntervall` → frame-timing drift. Dispatch infra exists but never invoked for builtins. | `effect.rs` + `lib.rs:5175+, 5272+` |
| **commands** | partial (55%) | AI behavior determinism: MoveTo lacks JumpControl/FlightControl/Swim. Get missing `Random(15)-7` offset (`C4Command.cpp:1290`) and side-jump (`:1272`). Tick2/5/35 frame-divisor throttling absent → continuous execution breaks tick-sync. Scale/Hangle let-go thresholds missing. | `command.rs` |
| **players-crew-teams** | partial (770 LOC vs 5747) | Wealth clamp divergence (10k in `adjust` vs 100k in `set`, `player.rs:344,349`). Team home-base production sync missing (`C4RULE_TeamHombase`, `C4Player.cpp:1637`) — each player advances independently → desync. No `CheckElimination`, no hostility model, asset value is caller-provided stub. | `player.rs` |
| **weather-sky** | partial (65%) | All updates run every tick vs C++ Tick10/Tick35/Tick1000 gating — changes seasonal/wind temporal signature. Meteorite/earthquake/volcano disaster launching unimplemented (`tick_weather_events` `lib.rs:7811` does lightning only). `Random(60)`/`Random(100)` probability logic replaced by independent `&&`-chained `gen_range`. Sky parallax formula diverges (`wind/100` vs FIXED100). | `lib.rs`, `sky.rs`, `compat.rs` |
| **definitions-id** | partial (4319 LOC) | `CrossMapActMap()` procedure→numeric resolution NOT done (`definition.rs:35` keeps actions as strings) — runtime action behavior diverges. `GetComponents` script-override absent, `CalcDefValue()` dynamic pricing absent. C4ID byte extraction differs (`to_le_bytes` vs explicit shifts, null-handling). Many DefCore flags unparsed. | `lc-resources/definition.rs`; `compat.rs` |
| **config-info** | partial (49%) | `GetAName()` random name selection uses `Random()` — no Rust equivalent. `PromotionUpdate()` rank-scaled energy formula absent. `C4GameParameters` `RandomSeed = time(nullptr)` (`:425`) — determinism tied to external time. Default initialization differs (locale, control prefs). | `lc-core/std_config.rs`, `lc-app/settings.rs`, `scenario.rs` ×2 |
| **resources-groups** | partial (43%) | Read-only subset only: no group write/create (`Save/Add/Move/Delete`, ~800 LOC), no gzip compression, no CRC32 validation at open (`C4Group.cpp:791`). Path normalization via Rust `components()` may differ from C++. WalkDir entry order may differ from C++ `DirectoryIterator`. | `group.rs`, `scenario.rs` |
| **pathfinder-transfer** | **full** but buggy | Ray execution order divergence: C++ LIFO linked-list prepend (newest-first, `C4PathFinder.cpp:655`) vs Rust `insert(0,...)` + ordered snapshot iteration (FIFO-effective) → different path/waypoint sequences. Zone lookup order differs (`sorted_by_key(owner)` vs C++ insertion order). | `pathfinder.rs`, `transfer.rs` |

## Non-Determinism-Critical Gaps

All 26 assessments are flagged `determinism_critical: true`. The following subsystems, while flagged critical, are in practice rendering/presentation layers whose *visual* output diverges but whose impact on lockstep simulation state is secondary. Listed here for completeness and triaged below determinism-priority.

| Subsystem | Coverage | Key Risk (presentation-layer) | Rust Location |
|---|---|---|---|
| **graphics** | partial (25%, 1276 LOC vs 5045) | No transforms/rotation matrices (`CBltTransform`), no texture mgmt/GL, no shaders (`StdGL.cpp:38-1278`), no patterns, no gamma, no landscape rendering. `blit_region` is per-pixel only. Note: C++ texture-chunked blit uses platform float math that itself could diverge if ever made authoritative. | `lc-graphics/src` |
| **audio** | partial (35%) | Panning math fundamentally different (SDL 0–192 vs gain 0–1, `mixer.rs:775`). Entire `C4SoundSystem`/`C4MusicSystem` high-level layers absent (object attachment, audibility falloff, `MaxSoundInstances=20`, `IsNear` radius, wildcard selection). `SetPosition` declared, never implemented. | `lc-audio/src` |
| **gui-menus** | partial (3237 LOC vs 4467) | No rendering (`DrawElement` absent), no `InitLocation` layout, no text progression, no hotkey markup, no portraits. Column-based wrap-around selection replaced by simple modular arithmetic — diverges when `ItemCount % Columns != 0`. | `lc-app/object_menu.rs`, `ingame_menu.rs`, `lc-gui`, `menu_controls.rs` |
| **startup-launcher** | partial (~60%) | Player selection dialog entirely missing (stub status msg, `main.rs:6515`). No file validation, no update check, no first-start UX, no fade transitions. Startup integrated into game loop vs separate modal sequence. | `lc-frontend/startup_*.rs`, `lc-app/main.rs` |
| **network** | partial (3590 LOC vs 8379) | (Straddles both — control-coordination half is determinism-critical, see above.) Missing: password auth (`C4Network2.cpp:281-345`), voting, league integration, client status tracking (NCS_*), save/restore join-data, protocol negotiation (hardcoded `PROTOCOL_VERSION=1`). Client-ID signed/unsigned mismatch (`C4ClientIDHost=0` vs u32). | `lc-network/src` |

## Silent Stubs Inventory

Functions that exist and return plausibly but skip core C++ logic — the landmines that pass review and desync in production.

**sectors-regions-rect**
- `sector.rs` now covers the `C4LSectors`/`C4LArea` foundation: 50x50 sectors,
  point and shape lists, out-sector clipping behavior, and C++ row/pitch area
  iteration. Engine membership is rebuilt or updated on current object lifecycle
  paths.
- Current consumers are now wired: `AtObject()` uses point-sector candidates;
  bounded legacy `FindObject`/`FindObjects`/`ObjectCount` use sector candidates
  with fallback for worlds without a sector map; the existing collection
  cross-check slice uses object-sector area candidates.
- Still open here: implement the separate `C4Region` rectangle/list UI subsystem.
  The full C++ `CrossCheck()` gameplay branches remain in `objects-core`.

**fixed-point-math**
- Historical 2026-05-30 finding is resolved for current object paths: `C4Fixed`/
  `FixedVec2`, conversions, fixed Sin/Cos, live fixed position/velocity, raw C-ABI
  transport, and save/load preservation exist. Remaining fixed-point risk is in
  unported legacy consumers and movement edge branches listed below.

**rng-c4random**
- Historical 2026-05-30 finding is resolved for current callers: script/engine RNG
  uses C++ LCG `RandomHold`/`RandomCount`, `FixedRandom`, `Randomize3`, and the
  `Rnd3` circular buffer. Remaining RNG risk is in unported `SafeRandom` consumers
  and missing full network sync-check integration.

**movement-physics**
- Current shape movement now has per-pixel x/y stepping, shape/vertex
  `ContactCheck`, `RedirectForce`, contacted-vertex friction, `Shape.Attach`, border
  bounds, and per-degree rotation rollback for the current landscape density model.
- Still open: layer bounds/solid masks, full `UpdateShape`/construction-owned
  vertices, contact script callbacks, exact vehicle/material density behavior, and
  sector/object contact.

**landscape**
- `insert_material_at` (`:872-898`) — modifies surface but no `InsertMaterial` pathfinding/velocity/collision.
- `remove_material_at` (`:900-919`) — decrements height; no extraction/object-spawn.
- `incinerate_at` (`:921-926`) — returns early; never sets MNone or triggers effects.
- `can_incinerate` (`:815-835`) — checks inflammable but tracks no in-progress state.
- `blast_circle` (`:697-813`) — simplified; no BlastFree layers/grade depth modulation.

**material**
- `MaterialReactionKind::{Convert, Poof, Corrode, Incinerate, Insert}` (`material.rs:110-121`) — variants defined, zero execution logic.
- `MaterialSet::reaction()` (`:722-767`) and `custom_reaction()` (`:705-720`) — classify and return kind; caller must implement all physics (which doesn't exist).

**pxs-massmover**
- `MassMover::execute()` (`mass_mover.rs:208-251`) — no corrosion check (`C4MassMover.cpp:127`).
- No `LandscapeInsertThrust` (`:239-241`); no `Random(10)` pixel/material choice (`:239`).
- `find_liquid_target()` (`:254-291`) — no reaction callbacks during slide search.
- `tick_material_particles()` (`lib.rs:12259`) — `first_collision_on_line` skips step-by-step reactions.

**script-vm-aul**
- `Expr::This` (`vm.rs:416-419`) — hardcoded `Value::Nil`.
- `AssignmentTarget::{LocalSlot, VarSlot, EffectSlot, MethodSlot, FunctionCall}` (`vm.rs:1072-1158`) — string-mangled (`__local_/__var_`) keys, not array indices / proper dispatch.
- `forward_rest` variadic forwarding — TODO at `vm.rs:464,484`.
- `invoke_host_function` (`vm.rs:200-222`) — calls pointer without param/return validation.
- Call expr dispatch (`vm.rs:493-496`) — by-name, no inheritance/overload chain.

**script-values**
- `Value` enum (`value.rs:5-13`) — no `GetRefVal()`/deref.
- `as_bool()` (`:16-26`) — trivial; ignores Object/C4ID/pC4Value paths.
- `type_name()` (`:28-38`) — "proplist" not "map"; missing object-enum types.
- `LcScriptValue` FFI enum (`ffi.rs:10-17`) — only Nil/Int/Bool/String.
- `rust_value_to_lc()` (`ffi.rs:136-159`) — lines 157-158 return default for Array/Proplist (silent data loss).

**objects-core**
- `reset_action_to_default` (`lib.rs:10629`) — no `SetActionByName` state-machine enforcement.
- `apply_*_procedure` (`lib.rs:10244+`) — don't enforce ObjectCom state transitions.
- `compute_ocf` (`lib.rs:3768`) — does not use the new `AtObject()` for dynamic chop/exclusive checks yet; no `ContactCheck`, no velocity-based HitSpeed, wrong entrance-rotated check.

**effects**
- `advance_tick` (`effect.rs:86`) — timer bool only; no callback invocation / Execute semantics.
- `set_var/var` (`effect.rs:100-112`) — getter/setter, no script callback hooks.
- Effect-callback dispatch infra (`lib.rs:5175+, 5272+`) — exists but never invoked for builtin Fire.

**commands**
- `MoveToState::step()` (`command.rs:6257-6307`) — no `flight_control()`/`jump_control()`.
- `TransferState::step()` (`~10000+`) — no Tick5-gated script execution.
- `RetryState::step()` (`:8890+`) — only decrements interval.
- `HomeState::step()` — no contained/base-owner check.
- `FollowState::step()` — simplified; no Push ComDir copy / Ungrab.
- `PutState` — no command-mode failure-suppression flags.

**players-crew-teams**
- `set_crew()`/`sort_crew()` (`player.rs:467-474, 611-613`) — sorts by ObjectId, no existence/ownership validation.
- `update_asset_value()` (`:386-395`) — accepts pre-computed value, no object iteration.
- `set_home_base_material/production()` (`:476-496`) — filters zeros but no team sync (`SyncHomebaseMaterialToTeam`).
- `advance_home_base_production()` (`:558-589`) — no team-aware logic.
- `set_status()` (`:307-317`) — no crew evacuation / value calc / callbacks.
- Constructor — no Hostility list init.

**definitions-id**
- `Definition::load()` (`definition.rs:35`) — parses fields, no `CrossMapActMap` resolution.
- `parse_act_map()` (`:486-619`) — no procedure→numeric mapping; `next_action` stays a string.

**game-control-record**
- `Recorder::record` (`record.rs:69`) — pushes snapshot to Vec, no binary encoding.
- `Recording::to_writer` (`record.rs:34`) — JSON only, no varint frame-diff chunks.
- `Playback::validate_snapshot` (`record.rs:108-125`) — post-hoc validation, not streaming chunk read.
- `Game::tick` (`lib.rs:7837`) — runs state but doesn't integrate control/record lifecycle.

**findobject-ocf**
- `find_object` (`compat.rs:6784`) — linear/closest only, no factory.
- `find_object_closest` (`:6826`) / `collect_closest_matches` (`:6911`) — basic distance sort, no SortObject infra.
- `ocf compute` (`ocf.rs:46`) — no dynamic state updates.

**network**
- `broadcast_packet` Queue/Sync/Decide path (`session.rs:705-712`) — treats three delivery types identically.
- `run_client_loop` Request handler (`session.rs:1387-1399`) — fulfills resync with no tick-range validation / rate limit.
- `handle_accept` (`session.rs:469-548`) — no password auth.
- `record_packet` (`resync.rs:29-34`) — no monotonic-order / retransmit validation.
- `broadcast_exec_sync` (`session.rs:738-749`) — no host-frozen check.

**particles**
- `create_particle()` (`compat.rs:8355-8485`) — registers command, no execution.
- `ActiveParticle::tick()` (`lib.rs:812-817`) — pos+vel, life-decrement only.
- `apply_particle_commands()` (`lib.rs:12163-12209`) — add/remove only, no def lookup / proc exec.
- `tick_particles()` (`lib.rs:12540-12548`) — steps + removes expired; no environment interaction.

**config-info**
- `AudioOptions::apply_config()` / `DisplayOptions::apply_config()` (`settings.rs:60-105, 331-371`) — load subset, skip validation / Monitor/Gamma/Engine.
- `Config::get_bool()` (`std_config.rs:134`) — `true/1/yes` only, no `off/on/no`.
- `ScenarioObjectives::from_legacy_game()` (`scenario.rs:186-217`) — create/clear only, no parameter compilation.

**audio**
- `SetPosition` (`mixer.rs:313-316`) — declared, never implemented.
- AudioMixer — no wildcard selection, object attachment, `DetachObj`, `GetVolumeByPos`, `MaxSoundInstances`, `IsNear`, `ClearPointers`, audibility falloff.

**graphics**
- `Surface::blit` (`surface.rs:228-230`) / `blit_region` (`:232-323`) — per-pixel only; no texture/chunk/rotation/modulation/gamma/shader.
- `Color::blend_over` (`color.rs:36-57`) — basic alpha-over; no GL_COMBINE/ADD_SIGNED/MODULATE.

**gui-menus**
- `ObjectMenuState::render()` (`object_menu.rs:487-567`) — backdrop/panel only, no items/icons/controls.
- `IngameMenuState` — no rendering at all.
- `ObjectMenuState::handle_command()` (`:427-485`) — returns `None` for all cases, no side effects.

**weather-sky**
- `tick_weather_events` (`lib.rs:7811`) — lightning only; no meteorite/volcano/earthquake/cloud.

**startup-launcher**
- `MainMenuItem::PlayerSelection` handler (`main.rs:6515-6516`) — sets stub status text, no UI logic.

**resources-groups**
- `Group` struct — no write/create/modify; `maker()` getter but header.maker never written.
- `scenario.rs` module — read-only discovery, no saving.

## Theme C — Wire fixed precision through physics/collision/procedure code

These sub-items track action item 1 (C4Fixed migration). The former lossy
`sync_fixed_velocity_components_from_public` round-trip is now eliminated:
`fixed_velocity` is the authoritative physics store, and the public integer
velocity mirror is derived with `fixtoi`.

**Progress 2026-06-04:** Theme C is complete for the currently ported physics paths.
Gravity, friction, collision resolution, command/procedure movement, lift/fight/push/pull,
wind, and physics clamping now write `fixed_velocity` directly and refresh the integer
mirror from `fixtoi`. The remaining parity gap is no longer this sync layer; it is the
larger C++ per-pixel movement/contact loop tracked by action item 4.

### C1 — Gravity accumulation as C4Fixed (DONE 2026-06-04)

**C++ source:** `C4Movement.cpp:643-644`:
```cpp
// Adjust GravAccel once per frame
ydir += GravAccel;
```
`GravAccel = Game.Landscape.Gravity` (`C4Physics.h:27`), set at scenario load to
`FIXED100(Gravity_C4SVal.Evaluate()) / 5` (`C4Landscape.cpp:66`), default
`FIXED100(20)` = raw 16.16 value **13107** (≈0.2 px/frame²).

**Parity golden confirms:** `parity/golden/parity_golden.json` `movement[0]` —
`grav: 13107`, `ydir` grows by 13107 each frame, `fix_y` accumulates the series.

**Status:** Implemented in `PhysicsSettings::gravity_as_c4fixed()`,
`ActionProcedure::gravity_component_fixed()`, and `apply_physics_at_index()`. The
lossy `sync_fixed_velocity_components_from_public` call in the central physics loop
was removed. Test coverage: `gravity_accumulates_as_c4fixed_matching_cpp_golden`
asserts the raw C++ golden `ydir` sequence.

**Former Rust breakage:**

| Location | What it does wrong |
|---|---|
| `lib.rs:955` | `PhysicsSettings.gravity: i32` — stores the C4SVal base integer (e.g. 100), not a C4Fixed |
| `action.rs:180` | `fn gravity_component(self, base_gravity: i32) -> i32` — returns whole-pixel integer |
| `lib.rs:10284` | `object.state.velocity.y = object.state.velocity.y.saturating_add(gravity_component)` — integer addition, loses sub-pixel precision |
| `lib.rs:10365` | `object.sync_fixed_velocity_components_from_public(previous_velocity)` — lossy sync back to fixed; sub-pixel accumulated before the tick is silently discarded |

For the default gravity base=100: Rust applies **+100 px/frame²** to integer velocity;
C++ applies **+0.2 px/frame²** as a sub-pixel C4Fixed accumulation — a ~500× error.

**Implemented fix:**
1. Add `fn gravity_as_c4fixed(&self) -> C4Fixed` to `PhysicsSettings`:
   `C4Fixed::from_raw(itofix(self.gravity, 100).val() / 5)` (mirrors `FIXED100(g)/5`)
2. Add `fn gravity_component_fixed(self, base: C4Fixed) -> C4Fixed` to `ActionProcedure`
   (same halving logic as `gravity_component`, applied to C4Fixed raw values)
3. Replace `lib.rs:10284` with `object.fixed_velocity.y += gravity_c4fixed`
4. Remove the `sync_fixed_velocity_components_from_public` call at `lib.rs:10365`
   (integer mirror stays current because `advance_fixed_position` derives it each tick)

**Test gate:** `gravity_accumulates_as_c4fixed_matching_cpp_golden`
asserting `fixed_velocity.y` after N frames equals the golden's `ydir` field.

---

### C2 — Friction applies to `fixed_velocity` (DONE 2026-06-04)

**C++ source:** `C4Movement.cpp:569` (`ContactVtxFriction`), `C4Material.cpp:570-604`
(slide/splash physics); friction operates on the C4Fixed `xdir/ydir` directly.

**Status:** `apply_material_interaction()`, particle-object collision, and the spawn-time
landscape helper now apply friction to `fixed_velocity.x` with
`apply_horizontal_friction_fixed()` and refresh the integer mirror immediately. The
state-only `apply_landscape(&mut ObjectState)` path was replaced with an object-aware
helper so spawn-time landscape friction can preserve sub-pixel velocity.

**Former Rust breakage:**

| Location | What it does wrong |
|---|---|
| `lib.rs:10011` | Fixed 2026-06-04: state-only `apply_landscape(&mut ObjectState)` was replaced by object-aware fixed-preserving landscape handling |
| `lib.rs:2835` | Fixed 2026-06-04: live `apply_material_interaction` applies `apply_horizontal_friction_fixed(self.fixed_velocity.x, friction)` |
| `lib.rs:12773` | Fixed 2026-06-04: particle-object collision applies fixed friction and refreshes the integer mirror |

**Test gate:** `spawn_landscape_friction_applies_to_fixed_velocity` asserts spawn-time
landscape friction reduces raw `xdir=98304` to `49152` without integer rounding.

---

### C3 — Collision resolution preserves sub-pixel (DONE 2026-06-04)

**C++ source:** After the `while x != ctcox` contact loop (`C4Movement.cpp:266-282`) C++
negates/zeros `xdir/ydir` (C4Fixed) on contact — the sub-pixel fraction is retained or
explicitly zeroed at the fixed level.

**Former Rust breakage:**

| Location | What it does wrong |
|---|---|
| `lib.rs:11523-11524` | Fixed 2026-06-04: `apply_landscape_at_index` now uses fixed-preserving collision resolution |
| `lib.rs:2998-3001` | Fixed 2026-06-04: effect/command landscape resolution now uses fixed-preserving collision resolution |
| `lib.rs:8929-8932` | Fixed 2026-06-04: object-update landscape resolution now uses fixed-preserving collision resolution |

**Implemented fix:** `set_velocity_preserving_subpixel` leaves unchanged axes untouched,
zeros fixed components when collision resolution zeroes an axis, and negates the fixed
component when a sign reversal is reported. All three landscape collision call sites use
`apply_collision_resolution`, and the integer mirror is always `fixtoi(fixed_velocity)`.

**Test gate:** `landscape_collision_preserves_fixed_x_and_zeroes_contact_y` asserts a
collision keeps raw fractional `xdir=300` while zeroing contacted `ydir`.

---

### C4 — Command movement accelerations as C4Fixed (DONE 2026-06-04)

**C++ source:** `C4Object.cpp:4776` (Walk: `xdir -= WalkAccel; if (xdir < -lLimit) xdir = -lLimit`).
`WalkAccel`, `SwimAccel`, `FloatAccel` etc. are `const C4Fixed` (`C4Physics.h:31-32`).

**Status:** The six movement helpers now take `&mut FixedVec2`, convert
`MovementProfile` integer speeds/accelerations to `C4Fixed` at use sites, and the
central physics loop passes `&mut object.fixed_velocity`. Swim receives the fixed
gravity component from C1. The lossy central physics sync call was removed.

**Former Rust breakage:**

| Location | What it does wrong |
|---|---|
| `lib.rs:6007` | `apply_float_command_movement(&mut Vector2, …)` — takes integer velocity, integer acceleration from `MovementProfile` |
| `lib.rs:6041` | `apply_walk_command_movement(&mut Vector2, …)` — same |
| `lib.rs:6070` | `apply_swim_command_movement(&mut Vector2, …, gravity_component: i32)` — same |
| `lib.rs:6105` | `apply_scale_command_movement(&mut Vector2, …)` — same |
| `lib.rs:6142` | `apply_hangle_command_movement(&mut Vector2, …)` — same |
| `lib.rs:6190` | `apply_dig_command_movement(&mut Vector2, …)` — same |
| `lib.rs:10295-10339` | All six call sites pass `&mut object.state.velocity` |
| `lib.rs:10365` | `sync_fixed_velocity_components_from_public` converts back (lossy) |

`MovementProfile` fields (`walk_acceleration`, `walk_speed`, `float_acceleration`,
`float_speed`, `swim_acceleration`, `swim_speed`, `scale_speed`, `hangle_speed`) are
currently `i32`. They should be stored as `C4Fixed` raw values (or converted at call
time) to avoid recurring tenths-style precision loss.

**Implemented fix:**
1. Change the six movement functions to take `&mut FixedVec2` and `C4Fixed` acceleration
2. Pass `&mut object.fixed_velocity` at the six call sites
3. For Swim, pass the `C4Fixed` gravity (`gravity_c4fixed` from C1) instead of the integer
4. Remove the `sync_fixed_velocity_components_from_public` call at `lib.rs:10365`

---

### C5 — Push/pull/fight/lift procedures in C4Fixed (DONE 2026-06-04)

**Former Rust breakage — all used integer `step_toward` then sync:**

| Location | Function |
|---|---|
| `lib.rs:11371-11394` | Fixed 2026-06-04: `update_pull_pair` steps `fixed_velocity.x` and refreshes the integer mirror |
| `lib.rs:11397-11434` | Fixed 2026-06-04: `update_push_pair` steps `fixed_velocity.x` / decelerates fixed `ydir` and refreshes |
| `lib.rs:11255-11285` | Fixed 2026-06-04: `apply_fight_procedure` steps `fixed_velocity.x`, zeroes fixed `ydir`, and refreshes |
| `lib.rs:10505-10521` | Fixed 2026-06-04: `apply_lift_to_target` steps fixed `ydir` and refreshes |

**Implemented fix:** `step_fixed_toward` mirrors the old helper but operates on
`C4Fixed` raw values. Lift, fight, push, and pull derive their integer mirrors from
`fixtoi(fixed_velocity)`, and the six remaining `sync_fixed_velocity_components_from_public`
call sites were removed.

**Test gate:** `lift_procedure_adjusts_target_velocity`,
`fight_procedure_moves_toward_target`, `push_procedure_moves_target_and_pusher`, and
`pull_procedure_moves_target_and_puller` now assert raw fixed velocity values after the
procedure updates.

---

### C6 — Wind applies to `fixed_velocity` (DONE 2026-06-04)

**Former Rust:** `lib.rs:10286-10288`:
```rust
self.environment.apply_to_velocity(&mut object.state.velocity, self.frame);
```
Takes an integer `Vector2`, then syncs at `lib.rs:10365`.

**Implemented fix:** `EnvironmentSettings::apply_to_velocity` now takes `&mut FixedVec2`
and applies wind as `FIXED100(iWind)` via `fixed100(wind_force)`.

---

### Architectural end-state: eliminate `sync_fixed_velocity_components_from_public` (DONE 2026-06-04)

`sync_fixed_velocity_components_from_public` has no callers and was deleted. The
`fixed_velocity` field is now the authoritative store for the currently ported physics
paths, matching C++ semantics where `xdir/ydir` (C4Fixed) are the truth and the integer
velocity is just the `fixtoi` projection.

**Verification:** `cargo test -p lc-engine`, `cargo xtask engine-snapshots verify`, and
`cargo xtask parity verify` pass after the Theme C completion.

---

## Top 15 Prioritized Action Items

Determinism-critical first; each blocks lockstep until done. Foundational items (1–3) gate almost everything else.

1. **PARTIAL 2026-06-04 — Implement `C4Fixed` 16.16 fixed-point type and replace `Vector2` i32/i32 with it.** `C4Fixed`/`FixedVec2` now exist in `math.rs`, live `Object`s carry private fixed position/velocity, and the former integer `position += velocity` update now accumulates fixed velocity. **The script velocity surface is now migrated:** `SetXDir`/`SetYDir` store `itofix(n, prec)` and `GetXDir`/`GetYDir` return `fixtoi(v, prec)` (default precision 10), carried through a new authoritative `fixed_velocity` field on `ObjectUpdate`/`ObjectDelta` (`apply_delta` derives the integer mirror via `fixtoi`); the orphaned `scale_velocity_value` "tenths" path was removed. Verified end-to-end (`compat::tests::set_x_dir_stores_subpixel_fixed_velocity_like_cpp`, `lib::tests::set_x_dir_script_applies_subpixel_velocity_end_to_end`) and the 4 host-fn tests that encoded the buggy tenths convention were corrected to the C++ values. **Sub-pixel now also survives the snapshot/save-load round-trip** (task B): `ObjectSnapshot` carries optional raw `fixed_position`/`fixed_velocity` (`lib.rs`), emitted only when they hold sub-pixel beyond `fixtoi` (so whole-pixel snapshot baselines are unchanged), populated by `Object::snapshot()` and consumed by `restore_state`; verified for both the in-memory `restore_snapshot` path and the JSON `capture_state`→`restore_state` save-game path (`lib::tests::snapshot_round_trip_preserves_sub_pixel_velocity`, `json_save_load_preserves_sub_pixel_velocity`). **Rotation velocity is implemented and gated like C++** (task A2): `Object` carries `rdir`/`fix_r`, the motion step applies `fix_r += rdir * 5` with half-circle wraparound (`C4Movement.cpp:373-436`), `SetRDir`/`GetRDir` use `itofix(n,prec)`/`fixtoi(rdir,prec)`, `rdir`/`fix_r` survive save/restore (`lib::tests::set_r_dir_script_rotates_object_like_cpp`, `snapshot_round_trip_preserves_rotation_velocity`, plus `compat::tests::set_r_dir_*`/`get_r_dir_*`), `Rotate=` is parsed from `DefCore.txt`, `OCF_Rotate` is set for rotateable definitions, non-rotateable objects zero rotation state on movement, and finite `Def->Rotateable > 1` ranges clamp `fix_r`/stop `rdir`. **Theme C is complete for currently ported physics paths:** gravity, friction, collision resolution, walk/swim/float/scale/hangle/dig acceleration, push/pull/fight/lift, wind, and physics clamping now use authoritative `fixed_velocity`; `sync_fixed_velocity_components_from_public` was deleted. **Post-Theme-C FFI transport is complete:** `LcEngineObjectSnapshot`, `LcEngineRuntimeObjectState`, and `RustEngineBridge.cpp` now carry raw `fix_x`/`fix_y`/`xdir`/`ydir`/`fix_r`/`rdir` instead of reconstructing them from whole-pixel mirrors. **Per-pixel movement is now contact-aware for the current landscape density model:** central object movement walks horizontally and vertically one pixel at a time toward the fixed target, initializes object vertices from DefCore shape vertices, consumes `ContactDensity`/`BorderBound`/ActMap `Attach`, runs shape/vertex `ContactCheck`, applies C++-style `RedirectForce`/contact friction, clamps fixed border targets, supports `Shape.Attach` re-snapping, checks attachment even without momentum, forces Jump/default action transitions on attach loss, and rolls back per-degree rotation on contact. Remaining work: layer bounds/solid masks, full `UpdateShape`/construction-owned vertices, contact callbacks, exact material/vehicle density providers, and sector/object contact.

2. **DONE for `lc-engine` 2026-06-03 — Replace ChaCha8 RNG with the C++ LCG.** `LcgRng` now implements `RandomHold`/`RandomCount`, `Random(range)`, `SeededRandom(seed, range)`, engine-start `FixedRandom(seed); Randomize3();`, and serialized RNG state. Script `Random()` and engine random-argument draws now consume the LCG sequence.

3. **DONE for current callers 2026-06-03 — Implement `Randomize3`/`Rnd3` circular buffer.** `LcgRng` owns the 500-entry `FRndBuf3` equivalent and `MassMover` now uses shared `rng.rnd3()` instead of on-demand `gen_range(0..3)-1`.

4. **PARTIAL 2026-06-04 — Port the per-pixel stepping movement loop with sub-pixel accumulation.** The central object motion no longer commits `position += velocity` in one shot when a landscape is present: it computes the fixed target, walks one pixel at a time on x then y, and resets blocked fixed accumulators. The current slice now loads DefCore shape vertices/contact metadata, zero-fills vertex arrays like C++, records ActMap `Attach`, runs shape/vertex `ContactCheck` against the material density provider, applies `RedirectForce` plus contact friction/weight handling, clamps `BorderBound` side/top/bottom fixed targets, performs `Shape.Attach()` vertex re-snapping, checks attachment once even without momentum, forces Jump/default action transitions on attach loss, and steps rotation one degree at a time with rollback on contact. Remaining C++ parity: layer bounds/solid masks, full `UpdateShape`/construction-owned vertices, contact script callbacks, exact vehicle/material density behavior, and sector/object contact.

5. **DONE for sector-map infrastructure and current consumers 2026-06-04 — Build the `C4LSectors`/`C4LArea` spatial-partitioning system from scratch.** `sector.rs` now implements 50x50px object binning, `SectorAt()`-style out-sector behavior, separate point-sector and shape-overlap lists, and `C4LArea::Next()` row/pitch iteration with the C++ `dpitch = Wdt - (x+w-1)/wdt + x/wdt` semantics, including edge cases where clipped areas yield the out-sector last. `Engine` owns an optional sector map and updates/rebuilds membership on landscape set/clear, spawn, restore, movement, script/object updates, action/effect callbacks, deletion, and object-list pruning. Consumers are now wired for the currently ported paths: `AtObject()` uses point-sector candidates with shape/OCF/exclusive checks, bounded legacy `FindObject`/`FindObjects`/`ObjectCount` use sector candidates with linear fallback, and the existing collection cross-check slice uses object-sector area traversal. Separate `C4Region` UI/input rectangles are still absent; full C++ `CrossCheck()` incineration/fight/hit-damage behavior remains item 6.

6. **Implement the remaining `C4GameObjects::CrossCheck()` inter-object collision loop.** `AtObject()` now exists and the current Rust collection auto-check is sector-backed, but the 919-LOC tick-gated C++ incineration/fight/collection/hit-damage loop is not ported. Remaining work: Tick3/5/10/35 scheduling, `RejectFight`/`CatchBlow` callbacks, realistic hit energy/fling, contained-object fight checks, contact incineration, and exact mutation/recheck behavior after callbacks.

7. **Rebuild `script-values` with reference semantics, conversion table, and correct hashing.** Add `FirstRef/NextRef/AddDataRef/DelRef` reference chaining, the `C4ScriptCnvMap` conversion table (`C4Value.cpp:488-598`), and boost-style `hashCombine` (`C4Value.cpp:965-1029`) replacing the derived `Hash` in `value.rs`. Marshal Array/Proplist in `rust_value_to_lc()` (`ffi.rs:157-158`) instead of silently dropping. Save/load + map keys depend on this.

8. **PARTIAL 2026-06-04 — C4Script VM operator parity + `Expr::This`.** DONE this
   session (see "Session 2026-06-04 (cont.)" above): div/mod-by-zero → 0, `&&`/`||`
   operand-return, `nil`/`bool`→int coercion across all binary/unary integer
   operators, and **`Expr::This` now returns the current object context** — the VM
   carries a host-provided `this` value (`Vm::with_this`, `Engine::call_with_locals_and_this`),
   and all 8 object-context call sites in `lib.rs` pass `compat::object_reference_value(object_id)`
   so a script reading `this` gets the object reference `Proplist{"id"}` (was hardcoded
   `Nil`). Also DONE 2026-06-04: strict-level-correct `==`/`!=`, the `..`/`..=`
   concatenation operator, and the call-depth limit (raised 64 → 512 to match
   `MAX_CONTEXT_STACK`, using `stacker` for safe native-stack growth; `cc`/`psm`/
   `stacker` pinned for Rust 1.87, since newer `cc` pulls `ar_archive_writer` which
   needs Rust 1.88). **STILL OPEN (the one remaining item-8 part): array-indexed
   Local/Var storage.** The VM stores numeric `Var(n)`/`Local(n)` slots as separate
   `__var_n`/`__local_n` HashMap keys (`vm.rs` ~1031/1194). In C++ these are *aliases*:
   `FnVar(n)` returns `Caller->NumVars[n].GetRef()` (`C4Script.cpp:3390-3395`) and
   `FnLocal(n)` returns `pObj->Local[n].GetRef()` (`:3408+`), i.e. `Var(0)` is the first
   parameter and `Local(0)` is the definition's first `local` var. **Concrete divergence:**
   `func Test(a) { SetVar(0, 99); return a; }` returns 99 in C++ (Var(0) aliases `a`) but
   5 in Rust (separate `__var_0`). The divergence only manifests when content *mixes*
   numeric `Var(n)`/`Local(n)` and named access to the *same* slot — pure-numeric and
   pure-named usage are each internally consistent, which is why no test currently fails.
   A faithful fix needs reference-semantics local storage (an ordered, index-addressable
   array shared with the named bindings — hoisting `var` decls to the function scope and
   ordering object `local` decls), i.e. a core variable-storage refactor. Left as a
   dedicated effort rather than rushed, since it touches the storage every script uses.

9. **Implement the material reaction execution layer.** Write the `mrf*` handlers behind `MaterialReactionKind` (`material.rs:110-121, 722-767`): `mrfInsertCheck` splash (8× damping) + slide physics (`C4Material.cpp:570-604`), `mrfCorrode` with its two `Random(100)` calls (`:701,724` — RNG-sequence-critical), `mrfPoof`. Wire `ExtractMaterial`/`InsertMaterial` landscape mutations.

10. **Restore mass-mover parity.** Add the corrosion reaction check (`mass_mover.rs:208-251` ← `C4MassMover.cpp:127`), `LandscapeInsertThrust` (`:239-241` ← `:140-151`), and `Random(10)` pixel-vs-material choice (`:239` ← `:144`). Match C++ reverse-iteration execution order (`C4MassMover.cpp:58`).

11. **Implement `CrossMapActMap()` action resolution in definition loading.** In `Definition::load()`/`parse_act_map()` (`definition.rs:35, 486-619`), map procedure names → numeric constants and resolve `next_action` strings → action indices per `C4Def.cpp:773-799`. Currently actions stay as strings, so runtime action behavior diverges across the entire object system.

12. **Implement the full particle physics processor.** Replace `ActiveParticle::tick()` (`lib.rs:812-817`) with `fxStdExec` semantics (`C4Particles.cpp:614-697`): gravity, wind-drift+friction, alpha fade, collision/`CollisionProc`, Delay/Repeats/Reverse animation. Add `Cast()`, `Push()`, the `fx*` proc maps, and `C4ParticleDef::Load()`, using `SafeRandom` (item 2) for variation.

13. **Add frame-tick gating across weather, commands, and control.** Implement Tick10/Tick35/Tick1000 (weather: `lib.rs:7811`/`advance_frame`) and Tick2/5/35 (commands: MoveTo path-recheck, Transfer script throttle) plus `ControlRate`/`ControlTick`/`SyncRate` modulo throttling in the control path (`ffi.rs:451-489`). Add meteorite/earthquake/volcano disaster launching with C++ `Random(60)`/`Random(100)` probability logic. Running everything every-tick changes the temporal signature and breaks tick-sync.

14. **Implement the sync-check state machine and binary record format.** Add `DoSync`/`SyncRate` timing, sync-check queueing + `RemoveOldSyncChecks` (`C4GameControl.cpp:441-468`), the varint frame-diff chunk encoding (`C4Record.cpp:243-264`), and the `+37` end-marker offset (`:196`), replacing JSON snapshots (`record.rs:34,69`). Required for replay + multiplayer desync detection.

15. **Port `FindObject` condition-tree factory and `C4SortObject` framework.** Implement `CreateByValue()` (`C4FindObject.cpp:37-162`) for nested `C4FO_And/Or/Not/ID/OCF/Distance/...`, the `C4SortObject` hierarchy (esp. `Random/Speed/Mass/Value/Func`), full `C4FO_AtRect`/`UseShapes()` semantics beyond the legacy host-function path, and deterministic sorted iteration. Fix `OCF compute` (`ocf.rs:46`) to recompute dynamic state.
