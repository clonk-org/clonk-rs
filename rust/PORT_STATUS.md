# LegacyClonk Rust Port — Status & GAP LIST

> Living document. Last updated 2026-06-11. The C++ engine in `../src/` is the
> **golden oracle**; parity = bit-for-bit match on simulation state. This file
> tracks every divergence from that goal.

## Scenario-load parity epic (2026-06-11, ACTIVE)

Goal: every real scenario in `content/` loads + applies like C++.
Scoreboard: `cargo xtask scenario-sweep` (per-scenario watchdog; slow in
debug — per-spawn snapshot cost). **93 scenarios: 93 load (100%), 93 apply
(100%)** — baseline before the epic was 60 load / 6 apply. GoldRush
additionally runs load → apply → register_player → 60 ticks with ZERO
script-error warnings (`cargo xtask scenario-errors Goldrush`), the
zero-spurious-error bar the C++ engine sets on official content. Landed:
- Fail-safe script errors everywhere C++ is fail-safe (`tolerate_script_error`):
  def-script load failures register script-less (C4Def.cpp:632);
  Construction/Initialize/scenario-Initialize/action StartCall-family
  log-and-continue (C4AulExec.cpp:1318-1342). Unknown Objects.txt defs skip
  the object like C4Id2Def. OPEN: the lc-app sim-tick script error still
  exits the app (event loop) — same treatment needed.
- Loader leniency: strtol-style numbers (trailing junk ok), MapZoom
  C4SVal(10,0,5,15) default+bounds, `Clonks=` is the C4SVal crew COUNT with
  `StandardCrew=` the native def, out-of-enum Dir/ComDir warn-keep-default
  (the engine Dir model is still two-way — multi-directional Dir pending),
  Objects.txt Latin-1 fallback.
- Parser: `??`/`??=` (priority 3, nil-only, short-circuit), contextual
  keyword identifiers (params + expression position).
- `itofix`/`itofix_prec` wrap like C++ int32 (Objects.txt `Size=100000`
  panicked debug builds).
- The 292-entry C4ScriptConstMap constants table (script_constants.rs) with
  VM identifier fallback; System.c4g global scripts on `Game.ScriptEngine`
  semantics (`Engine::install_global_scripts`, own-def → global → host
  resolution, shared Arc table); call arity pads missing args with nil
  (C4AulParSet).
- 2026-06-11 second wave (GoldRush zero-error epic): C4Aul varargs
  (`func F(...)` ends the param list, `G(...)` forwards slots past the
  named params, `Par(i)` reads them — C4AulParse.cpp:1642,2293,
  C4AulExec.cpp:1127); `inherited`/`_inherited` via the OwnerOverloaded
  chain recorded on name collisions (add_script/merge_from/global installs);
  keywords as `var` names; `nil++` converts to 0 (CheckOpPar); C++ callback
  argument convention for real content (no params; AbortCall gets
  iLastPhase — C4Object.cpp:4154-4182) while command-DSL fixtures keep
  (state, ...); fail-safe Pre/InitializePlayer (C4Player.cpp:769);
  scenario-local System.c4g joins the global script engine
  (C4Game::LoadScenarioScripts, C4Game.cpp:3317-3343); C++ parameter
  coercion at the host boundary (unfilled/nil/bool → 0; falsy resets to
  nil before the type check — C4AulExec.cpp:1364-1396; CreateObject takes
  ids); host fns CreateContents, GetActMapVal, GetObjectVal, LocalN (VM
  builtin, self form), ActIdle, NoContainer/AnyContainer,
  SetEntrance/SetColorDw/SetShape/SetVertex; folder-chain (.c4f ancestor)
  definition sources; Initialize/Construction may self-remove; unknown
  spawns skip like C4Id2Def → nullptr.
REMAINING (documented gaps): LocalN cross-object form (WaterTower
`LocalN("iWater", pObj) += x`) needs world-object named-locals exposure;
GetObjectVal Width/Height serve the definition shape and do not reflect
SetShape overrides yet; GetActMapVal serves the ActionSpec subset (no
Facet/Directions/FlipDir); tick performance (see below); full-fidelity
definition apply (from_resource path) + DefCore key fixes (#15); #appendto
linking (#16).

### Tick performance (tasks #18, #19 DONE; follow-up #20)

GoldRush ticks: 37s → 2.2s (8168a0cb) → 0.20s (50775f1d) → **~0.12s**
(c0d81626). The #19 finding was a real LOCKSTEP fix, not just perf: area
queries returned candidates in an invented GLOBAL rank sort, while C++
enumerates area sectors row-major with the outside-sector last
(C4LArea::Next, C4Sector.cpp:264-277), each list rank-ordered within,
first-encounter Marker dedup (C4GameObjects.cpp:155-165,
C4FindObject.cpp:325-353) — affecting CrossCheck pair order (→ RNG
stream) AND script-visible FindObjects result arrays. Fixed + pinned by
tests (one old test had enshrined the invented order and was re-pinned
to C++). CrossCheck inner loops also reuse obj1*s stable index instead
of re-resolving per candidate.

Task #20 DONE (682b99b6): ObjectState.ocf is the cached mask, refreshed
exactly at the C++ SetOCF/UpdateOCF points — spawn Init
(C4Object.cpp:215), Execute-start (1058), host-driven updates
(SetAlive C4Object.h:361, DoCon 1417, status 4139, death 1177),
Enter/Exit both sides (1518-1597), Incinerate, snapshot-restore
recompute (2863). Raw state pokes stay stale until the next frame like
C++ (pinned by test). NOT modeled: NoCollectDelay/FnCollect refresh
(C4Script.cpp:395-400), and the SetOCF-vs-UpdateOCF bit split is moot
until the situational bits (HitSpeed1-4, In*, Chop, Entrance dynamics,
FightReady action-gating) exist in compute_ocf — those are the next OCF
gaps. GoldRush ~90ms/tick (from 37s at the epic start, ~400x).

## State: broadly scaffolded, not yet lockstep-parity-capable

The port reproduces the engine's *shape* (structs, enums, command dispatch, FFI,
~1300 tests) and the two original headline determinism breaks — fixed-point math
and the RNG — are now correct for the **currently ported paths**. All Top-15
action items are done or partial-with-documented-remainder; the physicals model
and GBackWind/IFT epics are complete. **Host→VM reentrancy CORE LANDED
(2026-06-10):** host functions can run script functions on other objects
mid-VM-call (`compat::call_world_object_function` — scope-stack swap inside the
effect context, outcomes folded into `EffectContextOutcome::other_objects` in
first-call order and applied by `Engine::apply_nested_object_outcomes`);
Find_Func/Sort_Func are the first consumers. Known seam divergences (documented,
all locals-related or snapshot-related): VM sessions own their locals, so nested
calls onto an IN-FLIGHT scope (self/dormant) read the pre-call locals snapshot
and their local writes are discarded; Func-criterion finds read a snapshot view
(mid-search mutations to non-target state and callback-spawned objects are not
re-read; C++ reads live state); when the OUTER call errors, the partial outcome
is dropped (pre-existing — C++ keeps mutations made before the error).
**Call family DONE (2026-06-10, second leg):** `Call`/`ObjectCall`/
`ProtectedCall`/`PrivateCall`/`DefinitionCall`/`GameCall`/`GameCallEx`
registered with C++-exact semantics (C4Script.cpp:3424-3534): script-only
owner-scoped resolution (engine functions never found), access levels are
log-only in C++ so the three object variants share one implementation,
`~`-failsafe only silences logging (miss → C4VNull either way; strip ≤2
leading `~`), DefinitionCall/GameCall run with the active scope PARKED
(Obj=nullptr — host functions see no object context), GameCallEx broadcasts
to live Goal/Rule/Environment objects (results discarded) then returns the
scenario result; the scenario script rides `HostWorldContext` as an
`Arc<ScriptEngine>`. mrfScript by-ref write-back DONE via
`ScriptEngine::call_with_ref_args` (reference-cell args, C4AulParSet GetRef
pattern). NOT ported (joins the documented CheckConvertFunctionParameters
gap): the per-call par-conversion flag matrix (CalledWithStrictNil →
falsy-par Set0 for non-strict3 callees, nil→0/false for strict3 callees,
the FnProtectedCall/FnPrivateCall 4-arg Exec quirk).
**Mass-mover script reactions DONE (2026-06-10, third leg):** the mover loop
runs through `&mut Engine` (`tick_mass_movers` takes the set out for the
frame), so `Type=Script` reactions dispatch at the exact C++ corrosion-check
position (C4MassMover.cpp:163-167: xdir=ydir=Fix0, write-backs discarded,
truthy return consumes the material) — RNG draw order pinned by the
migrated mover tests. The mrfScript epic is COMPLETE except the
global-script-engine resolution stand-in (scenario script).
Lockstep parity is still blocked by: C++ string interning + full save/load
+ binary packet encoding, the command-AI per-frame rework (incl. Throw
ejection + the C4ObjectInfo model) — and proven only by the live C++↔Rust
full-scenario shadow-diff (see Parity harnesses).

### The two foundational breaks — current status

1. **Fixed-point math — DONE for current paths.** `C4Fixed`/`FixedVec2` in
   `math.rs` (16.16, `itofix`/`fixtoi`, Sin/Cos). Live objects carry private fixed
   position/velocity/rotation; the motion step accumulates fixed velocity and
   projects to integer pixels. Script velocity surface (`SetXDir`/`GetXDir`, prec
   10) and rotation (`SetRDir`/`GetRDir`, `fix_r += rdir*5`, half-circle wrap,
   `C4Movement.cpp:373-436`) carry true sub-pixel. Snapshot + JSON save/load
   preserve raw `C4Fixed` (emitted only when beyond `fixtoi`). Raw `fix_x/fix_y/
   xdir/ydir/fix_r/rdir` cross the C ABI (`LcEngine*`/`RustEngineBridge.cpp`).
   `Rotate=` → `Definition::rotateable`, `OCF_Rotate`, non-rotateable zeroing,
   `Rotateable>1` clamp. **Theme C complete:** gravity, friction, collision,
   walk/swim/float/scale/hangle/dig accel, push/pull/fight/lift, wind, and
   physics clamping all write authoritative `fixed_velocity`;
   `sync_fixed_velocity_components_from_public` deleted. **Open:** residual
   movement systems outside item 4 (notably rotated/update-lifetime solid masks).

2. **RNG — DONE for current callers.** `LcgRng` is the C++ LCG
   (`RandomHold*214013 + 2531011; (RandomHold>>16)%range`, `C4Random.h:52-60`) with
   `RandomHold`/`RandomCount`, `FixedRandom`, `SeededRandom`, `Randomize3`/`Rnd3`
   (500-entry buffer), serialized with state. Engine seeds `FixedRandom(seed);
   Randomize3();`. The old ChaCha proptest is replaced by an LCG parity test.
   **Open:** `SafeRandom` consumers, full network sync-check integration.

## Parity harnesses

- **`cargo xtask parity verify`** (also `cargo test -p lc-engine
  parity_differential_matches_cpp_golden`) — the **real C++↔Rust differential**:
  diffs `C4Fixed` math, LCG (`Random`/`RandomCount` incl. range-0, `Randomize3`/
  `Rnd3`), `Sin`/`Cos`, per-frame sub-pixel accumulation (`fix += dir`,
  `ydir += gravity`), the `C4Value` map-key hash (`script_value_hash`), and the
  `C4ScriptCnvMap` conversion table + `ConvertTo` dispatch (`script_value_convert`)
  byte-for-byte against a golden from the real engine
  (`src/Fixed.{h,cpp}`, `src/C4Random.h`, `src/C4Value.cpp`). Reports first mismatch; negative
  control confirms it fails on a corrupted golden. **Gates Theme C.** Regenerate:
  `cargo xtask parity record`. See `parity/README.md`.
- **`cargo xtask engine-snapshots verify`** — Rust-vs-Rust determinism
  *regression* check only (NOT a parity check).
- **Phase 2 LIVE (2026-06-11, commit 4b5ee060)** — the shadow-diff runs
  end-to-end: `LC_RUST_ENGINE_RUNTIME=1 build-x86/.../clonk <scenario>
  <player.c4p>` shadows a live Rust runtime per frame and logs the first
  divergence to Clonk.log. Diagnostics: `LC_RUST_ENGINE_LOG=<filter>`
  installs a stderr tracing subscriber in the embedded runtime;
  `LC_RUST_ENGINE_CONTROL_DUMP=<path>` makes the bridge dump every
  serialised control frame (the ground truth for the Rust INI parser).
- **Player-join pipeline LANDED (task #23, 2026-06-11)** — the live
  GoldRush session executes CID_PlrInfo/CID_JoinPlr end-to-end:
  Engine::join_player ports C4Player::ScenarioInit (synced RNG ledger,
  PlaceReady*, crew GetIdle/New with ClonkNames draws, Recruitment
  callback, InitializePlayer args), the FFI runtime loads the .c4p
  (gz-wrapped C4Group support landed) and joins at frame 0 BEFORE that
  frame's tick (advance_to_frame was off by one for ALL control).
  Divergence moved 737-vs-810 -> 731-vs-810: the remaining gap is the
  InitializePlayer cascade (GoldRush's DoInitialize aborts at
  pObj->SetAI, defined in Locals.c4d/AI.c4d via '#appendto CLNK' — task
  #16), plus full-fidelity defs (#15). Known join gaps (documented in
  code): power-line auto-connections in PlaceReadyBase, base-exit
  commands, team start-index/hostility, the Magic list, the NativeCrew
  flag for empty-id GetIdle, GetAName file-based names, CrewDisabled for
  GetHiRank, StartupPlayerCount approximation (infos seen so far), and
  crew infos are not yet persisted in snapshots.
- **#appendto + statics + def-globals LANDED (task #16, 2026-06-11)** —
  Engine::resolve_appends ports C4AulScript::ResolveAppends/AppendTo
  (C4AulLink.cpp:29-64,114-141): definition and System.c4g scripts with
  #appendto copy their non-global functions into the targets as
  overrides (inherited reaches the original), system hosts first then
  defs in load order; includes stop copying global funcs (:127). Engine
  -global `static` table (GlobalNamed) shared across every script host
  (NOT yet persisted in snapshots); `global func` declarations in
  definition scripts register engine-wide (Time.c4d IsNight, MainTipi
  GetClan). Mid-call spawns carry a callable preview scope (C++ creates
  objects live during the call) and scenario-batch nested outcomes fold
  after spawns. Host fns: GetComponent, Enter/Exit (foreign subjects via
  the seam), ObjectSetAction, Material, Smoke, InLiquid (landscape
  approximation), SetPortrait/SetVisibility/SetClrModulation acks,
  GetHiRank, FindObjectOwner. Live divergence now 734-vs-810; GoldRush
  warnings 282 -> ~200. NEXT cascade blocker: the cross-object LocalN
  lvalue (`LocalN(name, pObj) = v`, GoldRush DoInitialize WSKI loop) —
  after it, the remaining unknown host fns (GetDefCoreVal, SetGamma,
  SetSkyParallax, ...) and per-object compare detail (#15). NOTE: the
  GoldRush

  zero-script-error claim (#17) does not hold at current HEAD — the
  baseline already showed ~210 warnings (unknown host fns GetComponent/
  InLiquid/SetClrModulation/..., #appendto-related Construction errors);
  the headless join harness (`cargo xtask scenario-errors`, now joining
  via Tyler.c4p when present) is the triage scoreboard. Previous recon
  (now superseded) — the
  bridge ALREADY implements live shadow execution AND divergence reporting:
  `LC_RUST_ENGINE_RUNTIME=1` advances a Rust engine per frame and
  `lc_engine_runtime_compare_snapshot` diffs every snapshot field, logging
  'Rust runtime parity mismatch' (RustEngineBridge.cpp OnFrame, :1934+;
  record/playback/authoritative modes too). What remains: rebuild both
  sides native arm64 (build/clonk.app is a stale x86_64 binary from Oct
  2025; `cargo xtask ffi --release` works again after the ffi.rs snapshot
  catch-up — lc_core + lc_resources staticlibs also needed per
  CMakeLists.txt:73-107), a headless scenario driver, then harvest the
  first mismatch per scenario as the divergence worklist. The C ABI
  snapshot lacks the new fire/physicals/breath/pxs_fixed fields (defaults
  on conversion); per-pixel collision, landscape and materials remain
  uncovered by the snapshot set.

## Gates

- **`cargo test --workspace`: GREEN** (~1240 pass, cargo exit 0). The old
  lc-network flake is FIXED (2026-06-10): `wait_for_host_ready` tolerates
  departing-client `TransportError` like `ClientLeft`.
- **`cargo clippy --workspace --all-targets -- -D warnings`: CLEAN**
  (verified 2026-06-10; the previous ~275-line backlog was resolved).
- **Graphical parity: NOT achieved** (presentation layer). Asset loading/2D blit
  matches, but menu chrome, the GL 3D scenario book, and in-game rendering all
  diverge — `lc-graphics` is ~25% (per-pixel blit only; no transforms/GL/shaders/
  landscape rendering). Live in-game capture is blocked (x86_64 Rosetta C++ build
  had no linked scenarios; `lc-app` is a non-bundled winit binary computer-use
  can't drive).

## Known accepted divergence

- **No general comma operator in C++.** Rust's `lc-script` `parse_comma` accepts
  comma sequences in any expression context; C++ only allows them inside `return
  (...)` via `multi_params_hack` (`C4AulParse.cpp:2069`); `,` is absent from
  `C4ScriptOpMap`. Rust only *accepts* more, and real content uses the legal
  `return (...)` form, so risk is low — but should be narrowed to C++ semantics.

---

## Determinism-Critical GAP LIST

Sorted worst-first (stub → partial; within partial, by severity). 24 of 26
audited subsystems are determinism-critical.

| Subsystem | Coverage | Key Parity Risk | Rust Location |
|---|---|---|---|
| **script-values** | partial | **Done:** `C4ScriptCnvMap` 81-cell conversion table + `ConvertTo` dispatch (differential-locked `script_value_convert`); boost `hashCombine` + libc++ `std::hash<C4Value>` map-key hash (`script_value_hash`); typed `C4V_C4Object` identity as `Value::Object(u64)` through VM equality/truthiness/type/hash, FFI, host `this`, object-returning helpers, and effect vars; recursive FFI marshalling for `C4Id`/`Array`/`Proplist` through `LcScriptValueKind` + `LcScriptMapEntry`; VM-visible reference semantics for `&` params, `func &` returns, Local/Var slots, and array/map element refs. **Open:** `GuessType()` data-nonzero path (unreachable in the eager Rust value model — types are always known), C++ string-table interning/refcounts. Save/load + net sync still incomplete. | `lc-script/src/value.rs`, `lc-script/src/vm.rs`, `lc-engine/src/compat.rs` |
| **particles** | mostly done (sim side) | **Corrected risk model:** C4Particles is *non-sync-relevant by design* (C4Particles.h:18-27) — every draw is `SafeRandom` (wall-clock-seeded libc `rand()`, C4Random.h:35,71-75), never the synced LCG, and counts scale with local `SmokeLevel`. The real parity risk was script-visible behavior: `CastParticles`/`CastBackParticles`/`PushParticles` were unregistered (script abort vs C++ `true`). **Done:** `particles.rs` ports `C4ParticleDefCore`+`Load` adjustments, def registry/overload, `Create` (room check, Attach offset), `Cast` draw structure, `Push`, `fxStdInit/Exec` (move/collision/RByV/gravity/WindDrift/AlphaFade/delay-phase/fadeout/offscreen), `fxSmokeInit/Exec`, Bounce/BounceY/Stop/Die; host fns with C++ return semantics incl. GetDef-failure → false; engine exec order object Back→Front then Global; snapshot/restore. **Open:** Particle.txt group loading (lc-resources) + gfx length/aspect from Graphics.png, draw procs (presentation), position-dependent `GBackWind`, clearing object-layer particles on object death. | `lc-engine/src/particles.rs`; `lib.rs`, `compat.rs` |
| **findobject-ocf** | mostly done (2026-06-10) | **Done:** the full `CreateByValue()` condition-tree factory + `C4SortObject` with C++ cache semantics (see item 15); **Find_Func/Sort_Func via the host→VM reentrancy seam (2026-06-10)** — per-candidate nested calls with raw-truthiness Check / getInt sort values, FindSameNameFunc-style own-def-then-host resolution, fPassErrors=true error passthrough, IsImpossible only when the name is unknown everywhere, Not swaps impossible/ensured, criteria parsing stops at the first nil par (C4Script.cpp:1996), single-result Find-with-sort now uses the UNCACHED pairwise `Compare(candidate, best)` (per-comparison value calls/Random draws, C4FindObject.cpp:186-199), post-sort destroyed objects keep their slot as Nil. **Open:** `Controller` compares owner, `Layer` never matches, sector-bounds FindMany traversal order; cached sort keys compare as i64 while C++ wraps i32 (`values[j]-values[i]` — divergent only for |values| ≥ 2^31 spreads); C++ stable_sort internals not mirrored for non-total comparators. | `compat.rs`; `ocf.rs` |
| **movement-physics** | partial | Central motion accumulates sub-pixel fixed velocity, steps x/y per pixel, consumes DefCore/current owned vertices and `StretchGrowth`/Jolt construction shape updates, runs shape/vertex `ContactCheck`, dispatches ContactLeft/Right/Top/Bottom and Hit/Hit2/Hit3 in C++ order, applies redirect/friction, clamps landscape and layer `TargetBounds`, overlays active DefCore solid masks as `MCVehic` contact density with sprite-alpha bitmap transparency, supports `Shape.Attach`, forces Jump/default on attach loss, rolls back per-degree rotation, and uses C++ density levels for background/material/vehicle contact checks (`C4M_Background=0`, material `Density`, closed side bounds and solid masks `C4M_Vehicle=100`). **Missing:** rotated solid-mask put-buffer semantics, `SetSolidMask`/solid-mask update lifetime, attached-object pushback. | `lib.rs`, `landscape.rs` |
| **objects-core** | mostly done (2026-06-09) | **Done:** full `CrossCheck()` — all three passes (Tick5 hostile fight + Tick35 contact incineration, every-frame hit-damage/fling + Tick3 collection, Tick10 contained fight; see Completed); fire model + `AssignDeath`; **C4PhysicalInfo physicals** (`[Physical]` parsing, `GetPhysical` override→def fallback, `TrainPhysical`, `ValByPhysical`, DoEnergy Energy-ceiling clamp) + the C++ DFA_FIGHT exec (see item 6). **Open:** OCF computes a subset of the ~30 C++ checks (`ocf.rs` vs `lib.rs:527-666`); object list is `Vec` vs category/ID-sorted; C4ObjectInfo (permanent training/experience) unmodeled. | `lib.rs`, `ocf.rs`, `compat.rs` |
| **game-control-record** | mostly done (2026-06-09) | **Done:** the real C4ControlSyncCheck digest (Random3/RandomCount/AllCrewPosX/SectShapeSum/MassMoverIndex via `CreatePtr` slots), `ControlRate`/`ControlTick`/`SyncRate` state machine, `BinaryControlRecord` 2-byte chunk-head stream with `RCT_Frame` fillers and the `frame+37` `RCT_End` (see item 14). **Open:** control-packet payload serialization (`DecompileToBuf<StdCompilerBinWrite>`), lc-network DoInput/queue wiring for host sync-check broadcast, `Prepare()` pre-validation. | `lib.rs`, `control.rs`, `record.rs`, `ffi.rs` |
| **material** | partial (65%) | **User-defined reaction parity DONE (2026-06-10):** reaction-table entries carry `fUserDefined`/`CheckSlide`; unknown/absent `Type=` (incl. "Incinerate", not user-nameable) installs a NoReaction that OVERRIDES the hardcoded default (ReactionFuncMap nullptr sentinel, C4Material.cpp:38-46); `mrfUserCheck` prologue (CheckSlide-gated splash/slide on PXSMove) with `!fUserDefined`-gated body checks; user Convert fires on PXSMove (C4Material.cpp:629-634). **mrfScript PXS path DONE (2026-06-10):** `Type=Script`/`ScriptFunc=` parsed into a `Script{func}` kind + name table, resolved lazily against the scenario script (C++ uses the GLOBAL engine — stand-in documented), called via `ScenarioScript::call_value` (raw return value, fail-safe exec: errors log + Nil, side effects fold via ScenarioBatch) with the C++ 9-int params (fixtoi(dir,100), MNone=-1, event index); truthy return kills the PXS. **Open:** by-ref write-back of X/Y/XDir/YDir/PxsMat after the call (needs lc-script reference-argument API; C4Material.cpp:814-832), meeMassMove script reactions (mass-mover loop has no VM access; RNG-order constrained lift onto Engine), mass-move `Convert` → `PXS.Create` handoff (C4Material.cpp:654-657), full `ExtractMaterial/InsertMaterial` semantics. | `lc-engine/material.rs`, `lc-resources/material.rs` |
| **pxs-massmover** | mostly done (2026-06-09) | **Done:** full `C4PXS::Execute` port — `pxs.rs` chunk/slot storage with `New()` lowest-free-slot reuse and chunk-major execution order (`C4PXS.cpp:175-234`), out-of-bounds rules, meePXSPos/meePXSMove reaction dispatch (`execute_pxs_reaction` mirrors mrfConvert/Poof/Corrode/Incinerate/Insert incl. depth-checked conversion and `Landscape.Incinerate`-at-position semantics), free-fall wind drift with the synced `Random(1200)` pair and `WindDrift_Factor`, coarse `_PathFree` (17×15 `PixCnt` cells, on-demand occupancy), step-loop with `fStopMovement` snap; `PXS.Cast` draw order (`r2` before `r1`, C4PXS.cpp:303-316) wired into blast (`level=60`, C4Landscape.cpp:1075-1078); dig spill as zero-velocity `PXS.Create`; raw-fixed save/load (`ParticleSnapshot.pxs_fixed`). Mass-mover side: down/L/R corrosion, two-pass reverse exec, `Random(10)` before `Rnd3()`. **Open:** mass-move `Convert` → `PXS.Create` handoff (C4Material.cpp:654-657; `MaterialReactionExecution::Converted` is produced but unconsumed), exact `BlastFree` material accounting around the cast. PXS wind drift is now position-dependent via the IFT tunnel overlay (2026-06-09). The invented PXS→object friction coupling was REMOVED (C++ PXS never touch objects). | `pxs.rs`, `mass_mover.rs`, `lib.rs`, `landscape.rs` |
| **landscape** | partial (25%) | Batch `apply_temperature_conversions` vs C++ incremental `ExecuteScan/DoScan` with `ScanX` cursor (scan order desyncs). No `PRETTY_TEMP_CONV`, no map creation (`ChunkyRandom`/`MapToLandscape`), no `DigFree/BlastFree`, no pixel ops, no Save/Load. Liquid model is segment- vs pixel-based. | `landscape.rs`, `material.rs` |
| **effects** | partial (timer semantics DONE 2026-06-10) | DONE: C++ modulo timer (monotonic iTime, zero interval never fires, verbatim iIntervall/iTime incl. AddEffect default 0), kill on C4Fx_Execute_Kill and on elapsed timerless intervals (Stop callback runs), FxStart C4Fx_Start_Deny (dead without Stop), list order ascending by |priority| (C4Effect.cpp:80-94). DONE 2026-06-10 second pass: command-target def resolution (C4Effect::GetCallbackScript - cross-def Fx* callbacks run in the right script), the Fx*Effect Check chain with C4Fx_Effect_Deny (priority-1 exemption). Open: Annul/AnnulCalls + FxAdd add-to-other-effect (C4Effect.cpp:191-210), TempRemove/TempReadd, Fx*Damage (DoEnergy modification, C4Object.cpp:1355-1359), builtin fire/helper effects (Splash/Smoke/Explosion/BubbleOut), the C++ global script engine (host-def fallback used). | `effect.rs`, `lib.rs` |
| **commands** | partial (55%) | AI determinism: MoveTo lacks Jump/Flight/Swim control; Get missing `Random(15)-7` offset (`C4Command.cpp:1290`) + side-jump (`:1272`). Tick2/5/35 throttling absent → continuous exec breaks tick-sync. Scale/Hangle let-go thresholds missing. | `command.rs` |
| **players-crew-teams** | partial (770 vs 5747) | Wealth clamps VERIFIED C++-faithful (the 10k-adjust/100k-set asymmetry matches DoWealth vs FnSetWealth, C4Player.cpp:905/C4Script.cpp:2764); SetWealth host fn registered (2026-06-09). Team home-base production sync missing (`C4RULE_TeamHombase`, `C4Player.cpp:1637`) → players advance independently. No `CheckElimination`, asset value is a caller stub. Hostility model DONE (2026-06-09): `PlayerState.hostility` + `C4PlayerList::Hostile` one-way-counts-both-ways for the CrossCheck fight pass. | `player.rs` |
| **definitions-id** | partial (4319 LOC) | `CrossMapActMap()` load-time mapping DONE (2026-06-09, item 11) but the engine runtime still dispatches on procedure strings, not the numeric indices. `[Physical]` section + ContactIncinerate/NoBurnDecay/NoBurnDamage/BurnTurnTo/IncompleteActivity now parsed (2026-06-09). No `GetComponents` override, no `CalcDefValue()`. C4ID byte extraction differs. Other DefCore flags still unparsed. | `lc-resources/definition.rs`; `compat.rs` |
| **weather-sky** | mostly done (2026-06-09) | **Done:** the full Tick10 disaster block (meteor/lightning/earthquake/volcano, exact synced draw order — see item 13); stateful weather per `C4Weather::Execute` (C4Weather.cpp:72-101): Tick35 season/temperature, Tick1000 `TargetWind = C4SVal::Evaluate` (ONE synced `Random(2*Rnd+1)` draw, `BoundBy(Std+…−Rnd, Min, Max)` with the C4S Wind defaults `(0, 70, −100, 100)`), Tick10 ±1 wind step — replacing both the per-frame `gen_range` interval model and the invented sinusoidal `wind_force(frame)` (now a stateful-wind accessor). Engine snapshots regenerated. **GBackWind DONE (2026-06-09):** Landscape tunnel(IFT) overlay + `Engine::wind_at` + positional `GetWind`; the invented object-wind application was REMOVED (C++ wind reaches only PXS/particles, C4Wrappers.h:189-192) — goldens regenerated. **Open:** IFT population from Landscape.txt (needs the pixel landscape), `SetSeasonGamma`, season Min/Max wrap from scenario StartSeason (mod-100 wrap used), sky parallax `wind/100` vs FIXED100. | `lib.rs`, `sky.rs` |
| **config-info** | partial (49%) | `GetAName()` random name uses `Random()` — no Rust equivalent. No `PromotionUpdate()`. `RandomSeed = time(nullptr)` (`:425`) ties determinism to wall-clock. Default init differs (locale, control prefs). | `lc-core/std_config.rs`, `lc-app/settings.rs`, `scenario.rs` |
| **resources-groups** | partial (43%) | Read-only: no group write/create (`Save/Add/Move/Delete`), no gzip, no CRC32 at open (`C4Group.cpp:791`). Path normalization (Rust `components()`) and WalkDir order may differ from C++ `DirectoryIterator`. | `group.rs`, `scenario.rs` |
| **sectors-regions-rect** | partial | `C4LSectors`/`C4LArea` done in `sector.rs`: 50×50 point/shape lists, `SectorAt()` out-sector behavior, `C4LArea::Next()` row/pitch iteration with clipped edge cases; membership rebuilds on all current object-lifecycle paths. Consumers wired: `AtObject()`, bounded `FindObject`/`FindObjects`/`ObjectCount`, collection cross-check. **Open:** separate `C4Region` UI/input rectangles. | `sector.rs`, `lib.rs`, `compat.rs` |
| **pathfinder-transfer** | full (order verified 2026-06-09) | Ray exec order VERIFIED C++-faithful (snapshot iteration over the newest-first active list = the C++ Next-pointer walk past prepends). Transfer-zone traversal order FIXED: ordered Vec, newest-first insert, in-place update (C4TransferZone.cpp:83-108). | `pathfinder.rs`, `transfer.rs` |

## Non-Determinism-Critical (presentation-layer) Gaps

Flagged critical in the audit but in practice their *visual* output diverges
while simulation impact is secondary.

| Subsystem | Coverage | Key Risk | Rust Location |
|---|---|---|---|
| **graphics** | partial (25%, 1276 vs 5045) | No transforms/rotation (`CBltTransform`), texture mgmt/GL, shaders (`StdGL.cpp`), patterns, gamma, or landscape rendering. `blit_region` per-pixel only. | `lc-graphics/src` |
| **audio** | partial (35%) | Panning math differs (SDL 0–192 vs gain 0–1, `mixer.rs:775`). `C4SoundSystem`/`C4MusicSystem` high-level layers absent (object attach, falloff, `MaxSoundInstances`, `IsNear`, wildcard). `SetPosition` declared, never implemented. | `lc-audio/src` |
| **gui-menus** | partial (3237 vs 4467) | No rendering (`DrawElement`), `InitLocation` layout, text progression, hotkey markup, or portraits. Column wrap-around → modular arithmetic (diverges when `ItemCount % Columns != 0`). | `lc-app/object_menu.rs`, `ingame_menu.rs`, `lc-gui` |
| **startup-launcher** | partial (~60%) | Player-selection dialog missing (stub msg, `main.rs:6515`). No file validation, update check, first-start UX, or fades. Startup folded into game loop vs separate modal. | `lc-frontend/startup_*.rs`, `lc-app/main.rs` |
| **network** | partial (3590 vs 8379) | Control-coordination half is determinism-critical (above). Missing: password auth (`C4Network2.cpp:281-345`), voting, league, client status (NCS_*), save/restore join-data, protocol negotiation (`PROTOCOL_VERSION=1` hardcoded). Client-ID signed/unsigned mismatch. | `lc-network/src` |

## Silent Stubs Inventory

Functions that return plausibly but skip core C++ logic — the landmines that pass
review and desync in production. (Resolved subsystems — fixed-point, RNG,
sectors — omitted; see status above.)

**script-vm-aul** — `AssignmentTarget::{LocalSlot,VarSlot,EffectSlot,MethodSlot,
FunctionCall}` (`vm.rs:1072-1158`) string-mangled (`__local_/__var_`) keys, not
array indices/dispatch; `forward_rest` variadic TODO (`vm.rs:464,484`);
`invoke_host_function` (`:200-222`) no param/return validation; call dispatch
(`:493-496`) by-name, no inheritance/overload chain. (`Expr::This`, div-by-zero,
slots, stack limit — fixed; see Completed.)
**Host-call par conversion (`CheckConvertFunctionParameters`,
C4AulExec.cpp:1364-1396) unported:** pre-#strict-3 callers get falsy pars
`Set0()`d to nil before the declared-type conversion, so real content legally
passes `0` where `C4String*`/object params are expected (found live: CLNK
`Control2Effect` crashed the app via `GetEffectCount(0, this())`,
Clonk.c4d Script.c:863). Emulated ONLY for the effect-name params
(`effect_name_filter`, compat.rs — AddEffect/RemoveEffect/GetEffect/
GetEffectCount); every other host fn still rejects falsy ints where C++
converts — content-crash landmines until the conversion layer exists.
Related app gap: control-path script errors now log-and-continue like C++
(`control_script_error_to_status`, lc-app main.rs), but a script error during
the simulation tick (`app.update()` in the event loop) still EXITS the app
where C++ shows it and keeps running.

**script-values** — `C4ScriptCnvMap`/`ConvertTo`, the map-key hash, typed
`C4V_C4Object` identity, VM-visible references for `&` params/returns and
container lvalues, and C4Id/Array/Proplist/Object FFI marshalling are no longer
stubs. Still stubbed: strings are owned Rust strings rather than C++ string-table
entries.

**objects-core** — `reset_action_to_default` (`lib.rs:10629`) no `SetActionByName`
enforcement; `apply_*_procedure` (`:10244+`) no ObjectCom transitions;
`compute_ocf` (`:3768`) no `ContactCheck`, no velocity HitSpeed, wrong
entrance-rotated check.

**movement-physics** — current slice has per-pixel x/y, `ContactCheck`,
`RedirectForce`, friction, `Shape.Attach`, border bounds, per-degree rotation
rollback, current-model density provider parity for background/material/vehicle
contact density, Contact*/Hit callbacks, and C++ `UpdateShape` construction
paths for definition/owned vertices plus `StretchGrowth`. Layer bounds now use
the C++ `TargetBounds` clamp path; active DefCore solid masks are parsed from
`SolidMask=`, sampled as `MCVehic`, and respect sprite-alpha transparency during
contact/attach/rotation checks. **Open:** rotated solid-mask put-buffer
semantics, `SetSolidMask`/solid-mask update lifetime, attached-object pushback.

**landscape** — `insert_material_at` (`:872-898`) no pathfinding/velocity/
collision; `remove_material_at` (`:900-919`) no extraction/spawn; `incinerate_at`
(`:921-926`) returns early; `blast_circle` (`:697-813`) no BlastFree layers/grade.

**material** — `MaterialReactionKind::{Convert,Poof,Corrode,Incinerate,Insert}`
(`material.rs:110-121`) variants defined; `reaction()`/`custom_reaction()`
(`:705-767`) classify only — non-mass-mover callers must implement physics.

**pxs-massmover** — RESOLVED for the PXS core (see GAP LIST row): the old
`tick_material_particles` float-jitter loop and its `first_collision_on_line`
shortcut are replaced by the faithful `C4PXS::Execute` step loop. Still
stubbed: `find_liquid_target()` (`mass_mover.rs`) reaction callbacks during
slide; mass-move `Convert` → `PXS.Create` handoff.

**effects** — `advance_tick` (`effect.rs:86`) timer bool only; `set_var/var`
(`:100-112`) no callbacks; dispatch infra (`lib.rs:5175+,5272+`) never invoked for
builtin Fire.

**commands** — `MoveToState::step()` (`command.rs:6257-6307`) no flight/jump
control; `TransferState` no Tick5 throttle; `RetryState` (`:8890+`) decrement
only; `HomeState` no base-owner check; `FollowState` no Push/Ungrab; `PutState` no
failure-suppression flags.

**players-crew-teams** — `set_crew()/sort_crew()` (`player.rs:467-474,611-613`) by
ObjectId, no validation; `update_asset_value()` (`:386-395`) accepts pre-computed
value; `set_home_base_*()` (`:476-496`) no team sync; `advance_home_base_
production()` (`:558-589`) no team logic; `set_status()` (`:307-317`) no
evacuation/callbacks; ctor no Hostility init.

**definitions-id** — `Definition::load()` (`definition.rs:35`) no `CrossMapActMap`;
`parse_act_map()` (`:486-619`) no procedure→numeric, `next_action` stays string.

**game-control-record** — `Recorder::record` (`record.rs:69`) Vec push, no binary;
`Recording::to_writer` (`:34`) JSON only; `Playback::validate_snapshot`
(`:108-125`) post-hoc not streaming; `Game::tick` (`lib.rs:7837`) no
control/record lifecycle.

**findobject-ocf** — `find_object` (`compat.rs:6784`) linear/closest, no factory;
`find_object_closest`/`collect_closest_matches` (`:6826,6911`) distance sort, no
SortObject; `ocf compute` (`ocf.rs:46`) no dynamic updates.

**weather-sky** — `tick_weather_events` (`lib.rs:7811`) lightning only.

**particles** — RESOLVED for the sim side (see GAP LIST row): full
`C4ParticleSystem` port in `particles.rs`, host functions registered with C++
return semantics, def-based exec wired into the engine tick. Remaining:
Particle.txt group loading, draw procs (presentation), position-dependent
wind, object-death particle cleanup.

**config-info** — `Audio/DisplayOptions::apply_config()` (`settings.rs:60-105,
331-371`) load subset, skip validation; `Config::get_bool()` (`std_config.rs:134`)
`true/1/yes` only; `ScenarioObjectives::from_legacy_game()` (`scenario.rs:186-217`)
create/clear only.

**presentation** — audio `SetPosition` (`mixer.rs:313-316`) declared only;
graphics `Surface::blit/blit_region` (`surface.rs:228-323`) per-pixel only;
`Color::blend_over` (`color.rs:36-57`) basic alpha; gui `ObjectMenuState::render/
handle_command` (`object_menu.rs:427-567`) backdrop only, returns `None`;
`IngameMenuState` no rendering; startup `PlayerSelection` (`main.rs:6515`) stub
text; resources `Group` no write/create.

**network** — `broadcast_packet` (`session.rs:705-712`) treats Queue/Sync/Decide
identically; Request handler (`:1387-1399`) no tick-range/rate limit;
`handle_accept` (`:469-548`) no password; `record_packet` (`resync.rs:29-34`) no
order/retransmit validation; `broadcast_exec_sync` (`:738-749`) no host-frozen
check.

---

## Top 15 Action Items

Determinism-critical first; items 1–3 gate almost everything. Status inline.

1. **PARTIAL** — `C4Fixed` type + replace `Vector2`. Core done (see foundational
   break #1 above). Remaining = residual non-item-4 movement systems plus other
   stateful subsystems.
2. **DONE** (lc-engine) — Replace ChaCha8 with the C++ LCG. (Break #2 above.)
3. **DONE** (current callers) — `Randomize3`/`Rnd3` circular buffer.
4. **DONE (requested contact-loop slices)** — Per-pixel stepping movement loop
   with sub-pixel accumulation.
   Done for current density model: DefCore vertices/`Attach`, shape/vertex
   `ContactCheck`, `RedirectForce`+friction, `BorderBound` clamp, `Shape.Attach`,
   Jump/default on attach loss, per-degree rotation rollback, and
   background/material/vehicle `GetDensity` levels for contact checks
   (`vehicle_density_boundary_below_contact_density_allows_motion_like_cpp`
   mirrors `C4Movement.cpp:260-281`, `C4Shape.cpp:389`,
   `C4Landscape.h:144-150`, `C4Material.h:200`), plus layer `TargetBounds`
   (`layer_border_bound_clamps_horizontal_target_like_cpp` mirrors
   `C4Movement.cpp:185-196` and `C4Movement.cpp:147-155`), and active
   solid-mask vehicle-density contact
   (`solid_mask_vehicle_density_blocks_per_pixel_contact_like_cpp` mirrors
   `C4Movement.cpp:260-282`, `C4SolidMask.cpp:66-104`, `C4Material.h:200`,
   `C4Movement.cpp:277`; resource parsing covered by
   `parse_def_core_solid_mask_target_rect`), solid-mask bitmap transparency
   (`solid_mask_transparent_bitmap_pixel_allows_motion_like_cpp` mirrors
   `C4SolidMask.cpp:80-104,401-411`), Contact*/Hit callback ordering, and full
   `UpdateShape` construction shape refresh for definition vertices, owned
   vertices across restore, and `StretchGrowth`
   (`construction_jolt_updates_vertices_and_preserves_bottom_like_cpp`,
   `construction_owned_vertices_survive_restore_like_cpp`,
   `construction_stretch_growth_scales_x_axis_like_cpp`). Residual movement gap
   tracked above: rotated solid-mask put-buffer/update lifetime and attached
   object pushback.
5. **DONE** (infra + current consumers) — `C4LSectors`/`C4LArea` (see GAP LIST).
   Open: separate `C4Region` UI rectangles.
6. **PARTIAL (pass 2 DONE 2026-06-09)** — `CrossCheck()` inter-object loop
   (C4GameObjects.cpp:92-230). **Done:** the reverse area check (pass 2,
   :140-197) as `Engine::cross_check`, run once per frame after object
   execution like C++ ExecObjects: OCF_Alive victims take OCF_HitSpeed2 hits
   from C4D_Object projectiles inside their shape every frame — QueryCatchBlow
   veto, hit energy `fixtoi((dX²+dY²)*Mass/5)` reduced `/3` (min 1),
   `DoEnergy(-e/5)`, `Fling(xdir*50/tmass, -|ydir/2|*50/tmass)` with the
   Tick3/DFA_FLIGHT gate and the Tumble→Jump→raw-velocity chain
   (C4Object.cpp:1612-1625, C4ObjectCom.cpp:48-80), CatchBlow callback, and the
   exact tamper rechecks; collection moved onto the Tick3 gate (Collection
   rect, marker dedup, per-candidate scan order). HitSpeed1-4 OCF bits now
   computed from fixed speed in `object_ocf_at_index` (SetOCF
   C4Object.cpp:588-592). **Pass 1 fight + pass 3 DONE (2026-06-09):** Tick5
   AtObject fight with `C4PlayerList::Hostile` (one-way declarations count
   both ways, C4PlayerList.cpp:82-92; `Player.hostility` persisted sorted in
   `PlayerState`), RejectFight vetoes on both sides, `ObjectActionFight` =
   SetActionByName("Fight", target); Tick10 contained fight (no RejectFight,
   C4GameObjects.cpp:199-230) with tamper rechecks. **Fire model + Tick35
   incineration arm DONE (2026-06-09):** object `on_fire`/`fire_phase`/
   `fire_caused_by` state (snapshot-persisted); `ContactIncinerate`/
   `NoBurnDecay`/`NoBurnDamage` DefCore fields; `Engine::incinerate_object` =
   C4Object::Incinerate + the deterministic fxFireStart core (already-burning/
   dead-living refusals, extinguisher-material check BEFORE the
   `FirePhase = Random(MaxFirePhase)` draw, Incineration callback);
   `exec_object_fire` = ExecFire (phase mod 15, every-frame `DoCon(-100)`
   decay with burn-away removal, Tick10 +2 damage, Tick5 −1 energy, Tick5
   background extinguish + the `Random(3)` landscape-inflame draw over valid
   material) run post-movement like the C++ fire effect timer;
   `OCF_OnFire`/`OCF_Inflammable` per SetOCF (dead livings excluded); the
   Tick35 arm consumes `Random(ContactIncinerate)` whenever the OCF pair
   matches and attributes via GetFireCausePlr's ValidPlr filter
   (C4Object.cpp:6193-6203). **Trimmings + death model DONE (2026-06-09):**
   BurnTurnTo ChangeDef (minimal `change_object_def`: def swap, default
   action, shape template/vertices refresh, rotation reset for
   non-rotateables, C4Object.cpp:1180-1228), contents ejection at fire start
   (into the container when contained, C4Effect.cpp:586-594, honoring
   IncompleteActivity/NoBurnDecay — both now DefCore-parsed), IncinerationEx
   for blasted-in-extinguisher; `AssignDeath` core (Dead action, command
   clear, contents ejection, Death callback with the tracked
   LastEnergyLossCausePlayer) fired by DoEnergy on first-zero energy.
   **Physicals model DONE (2026-06-09):** `C4PhysicalInfo` (21 fields,
   C4InfoCore.h:34-63) parsed from the DefCore `[Physical]` section via the
   `C4PhysInfoNameMap` names into `DefCore.physical` (defaults all zero);
   `ValByPhysical` = `itofix(physical*(percent/5), C4MaxPhysical*20)` with
   integer `percent/5` (C4InfoCore.h:224-227) + `Towards` snap-within-step
   (C4Object.cpp:4561-4566) in `math.rs`; `Engine::object_physical` =
   GetPhysical's override→definition fallback (C4Object.cpp:2118-2134);
   `train_physical`/`TrainValue` only-nonzero/cap/never-decrease
   (C4InfoCore.cpp:279-285) cloning the definition physicals on first
   training; DoEnergy now clamps to the physical Energy ceiling
   (C4Object.cpp:1361; zero-physical fixture definitions keep the legacy
   unclamped ceiling — documented deviation); **DFA_FIGHT exec**
   (C4Object.cpp:5200-5241): target-valid checks, Tick5
   `TrainPhysical(Fight,1,C4MaxPhysical)`, facing by target x, stand-beside
   at `target.x ± (Shape.Wdt/2+2)` with `lLimit = ValByPhysical(95, Walk)`
   `Towards` stepping, own-shape distance check after the approach, grounded
   `ydir=0`. **Procedure speed limits DONE (2026-06-09):** Walk/Scale/
   Hangle/Swim/Dig/Float ComDir movement follows the C++ physical model
   whenever the relevant `[Physical]` value is nonzero — `WalkAccel`/
   `SwimAccel`/`FloatAccel` constants (C4Movement.cpp:31-34), per-branch
   clamps to `ValByPhysical(280/200/160/160/125, …)` resp. `FIXED100(Float)`
   (C4Object.cpp:4771-5286), Scale/Hangle Tick5 + Swim Tick10 at-limit
   training, no gravity for Swim/Float, facing by xdir sign; physical-less
   fixture definitions keep the legacy `MovementProfile` paths (documented
   deviation). **Two-layer physicals + script API DONE (2026-06-09):**
   object physical state split into `info_physical` (C4ObjectInfo::Physical
   surrogate for crew members, lazily cloned from the definition),
   `temporary_physical` + `physical_changes` (PhysicalTemporary/
   TemporaryPhysical with the C4TempPhysicalInfo change stack); GetPhysical
   resolves temporary→info→definition (C4Object.cpp:2118-2134);
   TrainPhysical trains the temp set incl. stacked previous values and the
   crew info — an object with neither trains NOTHING (C4Object.cpp:
   2136-2146; C4InfoCore.cpp:309-317); host fns `GetPhysical`/`SetPhysical`/
   `TrainPhysical`/`ResetPhysical` with all PHYS_* modes
   (C4Script.cpp:552-688, fair crew off), state carried through script
   scopes and applied wholesale via `ObjectUpdate.physicals`; all three
   fields + `last_energy_loss_cause` + `breath` snapshot-persisted
   (C4Object.cpp:2738-2801). **ExecLife breathing DONE (2026-06-09):**
   Tick5 supply check at the mouth, breath −2*C4MaxPhysical/100 → at zero
   DoEnergy(−1) asphyxiation with cause attribution, synced `Random(5)`
   BubbleOut x draw, Breath training, one-gulp restore + DeepBreath
   callback (C4Object.cpp:878-919); NoBreath DefCore-parsed; breath fills
   from physicals at birth (:193). **ALSO FIXED:** the tick loop ran the
   fire effect TWICE per object per frame (both sites from the original
   fire commit; direct-call fire tests missed it) — double DoCon decay,
   double damage gates, double inflame draws; now once-per-frame with a
   tick-level pin (C4Object.cpp:1073-1077). **Open:** attach detach at fire
   start (needs the DFA_ATTACH action scan); fire modes/sounds; Tick5 base
   extinguish (base model); SmokeRate smoke (visual); Push/Pull force
   `ValByPhysical(250, Push)` + walk limit (C4Object.cpp:5048-5129),
   `ObjectComJump` Con-scaled Walk/Jump physicals (C4ObjectCom.cpp:287-288),
   Throw `pthrow = ValByPhysical(400, Throw)` (C4ObjectCom.cpp:127); swim
   InLiquid exit/surface checks (need the liquid model); MVehic forcefield
   breathe arm (needs the solid-mask material layer); FXB1 bubble object;
   corrosion/InMat-incineration ExecLife arms (need InMat tracking); the
   C4ObjectInfo model (permanent training storage, DoExperience — the fight
   exec skips the Tick35 `DoExperience(+2)`, fair crew); foreign-object
   physicals reads/writes in the host fns (this-object only, like
   DoEnergy); PHYS_* script constants (no constant registration mechanism
   yet); effect ClearAll revival abort and player pointer/view cleanup in
   AssignDeath.
7. **PARTIAL** — `script-values`. **Done:** `C4ScriptCnvMap` 81-cell table +
   `ConvertTo` dispatch (`C4Value.cpp:431-598`; differential-locked
   `script_value_convert` — 81-cell grid + per-(value,target,#strict) result);
   boost `hashCombine` + `std::hash<C4Value>` (`:923-1029`; `script_value_hash`);
   recursive C4Id/Array/Proplist FFI marshalling in `rust_value_to_lc()` +
   `lc_value_to_rust()` (`ffi.rs`); VM-visible reference semantics for `&`
   params, `func &` returns, Local/Var slots, and array/map element refs; typed
   `C4V_C4Object` identity as `Value::Object(u64)` through VM/FFI/host helpers.
   **Remaining:** C++ string-table interning/refcounts, full save/load + net
   sync wiring.
8. **DONE** — C4Script VM operator parity + `Expr::This` + Var/Local slots (see
   Completed).
9. **PARTIAL (splash/slide DONE 2026-06-09)** — Material reaction execution.
   Mass-mover path runs `MaterialReactionKind` with event masks, `mrfCorrode`
   `Random(100)` ordering + effect RNG, `mrfPoof` `Rnd3()`, shared
   `ExtractMaterial`/`InsertMaterial`. **New:** `mrfInsertCheck`
   (`C4Material.cpp:567-610`) ported as `Engine::mrf_insert_check` — splash
   (`-fYDir/8`, `fXDir/8 + FIXED100(Random(200)-100)`, exact Random order),
   incendiary `Random(25)`+`Rnd3()` smoke, `FindMatSlide`
   (`C4Landscape.cpp:1260-1290`, exact left-first/clog rules, on `Landscape`),
   same-mat absorb, slide accel `(fXDir*10+Sign)/11 + FIXED10(Random(5)-2)`,
   in-range jump + `fYDir<=0` zeroing — wired into the PXS-move
   Insert/Poof/Corrode/Incinerate arms with the C++ contact-adjacent check
   position (`C4PXS.cpp:96-117`). Remaining: script reactions (`mrfScript`),
   full fixed-point `C4PXS::Execute` step loop (item 10).
10. **DONE (PXS core; 2026-06-09)** — `C4PXS::Execute` + `C4PXSSystem`
    chunk/slot storage with exact `New()` slot reuse and execution order,
    reaction dispatch on both PXS events, `_PathFree`, `PXS.Cast`/`Create`
    at blast/dig sites, fixed-point state with lossless save/load. See the
    pxs-massmover GAP row for the short open list (mass-move Convert→PXS
    handoff, position-dependent wind, BlastFree accounting).
11. **DONE (load-time mapping; 2026-06-09)** — `CrossMapActMap()` in
    definition loading per `C4Def.cpp:773-799`: `ActionMap.actions` is now an
    ordered Vec keeping duplicates (C++ array semantics, first-match `get()`
    like `SetActionByName`); `procedure_index` resolves case-SENSITIVELY
    against the `ProcedureName` table (C4Def.cpp:38-58, miss → `DFA_NONE`);
    `next_action_index` maps "Hold"→`ACT_HOLD` case-insensitively, else
    case-sensitive name→index with last-duplicate-wins (overwrite loop
    :789-791), default `ACT_IDLE`. Remaining: engine runtime still dispatches
    on the procedure *string* (`ActionSpec`); switch dispatch + `next_action`
    transitions to the numeric indices.
12. **DONE (sim side; 2026-06-09)** — Full particle physics processor in
    `particles.rs`: `fxStdExec`/`fxSmokeExec`/collision procs, `Cast()`,
    `Push()`, proc maps, `Load` adjustments, `SafeRandom` stand-in (`SafeRng`).
    NOTE: corrected audit — C4Particles is non-sync-relevant by C++ design;
    the determinism-critical part was host-function registration/returns.
    Remaining: Particle.txt group loading + Graphics.png-derived length/aspect,
    draw procs, position-dependent wind, object-death cleanup.
13. **PARTIAL (disaster block DONE 2026-06-09)** — Frame-tick gating. **Done:**
    the C4Weather Tick10 disaster launch (C4Weather.cpp:104-148) with the
    exact synced draw order — gates `Random(60)`/`Random(35)`/`Random(50)`/
    `Random(60)` drawn unconditionally (levels only gate the follow-up
    `Random(100)`), forced argument-evaluation order for the meteor
    (`Random(101)` then `Random(GBackWdt)`), earthquake
    (`Random(GBackHgt)` then `Random(GBackWdt)`), and volcano (`Random(10)`
    then `Random(GBackWdt)`, size `BoundBy(15*GBackHgt/500+r2,10,60)`);
    launches spawn METO (fixed xdir `itofix(r2-50)/10`, rdir `itofix(1)/5`),
    FXQ1 + `Activate()`, FXV1 + `Activate(x,y,size,mat)`. New
    `WeatherEvent::{Meteorite,Earthquake,Volcano}` variants. **Stateful
    weather DONE (2026-06-09):** Tick35 season/temperature, Tick1000
    `TargetWind = C4SVal::Evaluate`, Tick10 ±1 wind step (see the
    weather-sky GAP row). `ControlRate`/`ControlTick`/`SyncRate` DONE with
    item 14. **Remaining:** command Tick2/5/35 throttles (`PathChecked`
    Tick35 reset C4Command.cpp:255, swim-steer Tick2 :372, Transfer Tick5
    :1931 — blocked on the larger command-AI rework, the Rust interval model
    is structurally different); meteor cave-landscape y offset needs the
    scenario `TopOpen` flag carried onto the engine landscape.
14. **MOSTLY DONE (2026-06-09)** — Sync-check state machine + binary record.
    **Done:** the C4ControlSyncCheck digest now carries the real C++ fields
    (`Random3` = the Rnd3 ring pointer, `RandomCount`, `AllCrewPosX` =
    `fixtoi(fix_x, 100)` centipixels over the players' crew lists,
    `SectShapeSum` = sector shape-list sum via `C4LSectors::getShapeSum` —
    replacing the invented FNV rng hash / whole-pixel / landscape-sum
    digest); `ControlTick` advances every `ControlRate` frames and `DoSync`
    fires every `SyncRate` (100) frames in `control_ticks()`
    (C4GameControl.cpp:326-332) with `do_sync_check()` closing each frame
    (C4Game.cpp:829); local queue + `get_sync_check` + strict-cutoff
    `remove_old_sync_checks` (keep 50) + `register_remote_sync_check`
    comparison for the network layer (C4Control.cpp:469-525). Binary record:
    `BinaryControlRecord` with the exact 2-byte `C4RecordChunkHead` stream,
    `RCT_Frame` filler chunks past 0xff frame diffs, no-rewind diff clamp,
    and the truncated `frame + 37` `RCT_End` marker (C4Record.cpp:194-264).
    `MassMoverIndex` now reports the real `CreatePtr` cursor over the
    fixed-slot C4MassMoverSet model (2026-06-09). **Open:** control-packet
    payload serialization into the binary stream
    (`DecompileToBuf<StdCompilerBinWrite>` packet encoding) and the
    lc-network DoInput/queue wiring for host sync-check broadcast.
15. **MOSTLY DONE (2026-06-09)** — `FindObject` condition-tree factory
    (`CreateByValue()`, C4FindObject.cpp:37-162) + `C4SortObject`
    (C4FindObject.cpp:683-932) ported into `compat.rs`: full condition set
    (Not/And/Or with null-filtering and trivial unwrap, Exclude, ID, InRect,
    AtPoint/AtRect/OnLine on definition-shape bounds, Distance, OCF, Category,
    Action, ActionTarget with 0..=1 clamp, Container, AnyContainer, Owner),
    IsImpossible/IsEnsured pruning, sorts Reverse/Multiple/Distance/Random/
    Speed/Mass/Value with C++ cache semantics — `C4SO_Random` draws the synced
    `Random(1<<16)` exactly once per object in collection order, then a stable
    ascending sort. Host fns `FindObject2`/`ObjectCount2` registered;
    `FindObjects` dispatches array-first-arg → C++ criteria form, else the
    legacy fixture form. CreateCriterionsFromPars AND-merging + no-criterion
    script error (C4Script.cpp:1985-2060). **Find_Func/Sort_Func DONE
    (2026-06-10)** via the host→VM reentrancy seam (see findobject-ocf GAP
    row for details and residual caveats). **Open:** `Controller` compares
    owner (no controller model); `Layer` never matches (host objects carry
    no layer); the sector-bounds traversal (and its sector-order FindMany
    result ordering) — the main list is always walked, matching the C++
    unbounded path.

---

## Completed (changelog)

**Host→VM reentrancy seam + Find_Func/Sort_Func + material user-reactions
(2026-06-10).** First slice of the reentrancy epic:
- Structural: `Definition.script` is `Arc<ScriptEngine>`; `HostWorldContext`
  carries `definition_scripts` (Arc clones) + per-object `Rc<ObjectState>`
  full snapshots; `DefinitionMetadata.action_library`.
- The seam: `compat::call_world_object_function(target, fn, args)` —
  three-phase borrow discipline (prepare under borrow → run the target def's
  VM borrow-free → restore under borrow); scope STACK on the context
  (`object` = active, `dormant_scopes` = suspended levels) with
  move-by-identity so one object never has two scopes (no double-apply);
  completed nested scopes + VM-final locals kept per target for resumption;
  folded into `EffectContextOutcome::other_objects` (ordered) and applied by
  `Engine::apply_nested_object_outcomes` (update/destroy/commands/effects +
  effect events per object); threaded through `CommandBatch`/`ScenarioBatch`
  for the DSL/scenario paths. Resolution = target def's script function,
  host functions as engine fallback, miss → silent None
  (FindSameNameFunc, C4Aul.cpp:130-148).
- Find_Func (C4FindObject.cpp:124-136,653-662): name+pars captured (slot 2 →
  par 0, 10-par cap), raw-truthiness Check, errors rethrown
  (fPassErrors=true), IsImpossible = name unknown everywhere, Not swaps
  impossible/ensured, Func-mode finds run on a borrow-free snapshot view;
  Status re-checks: pre-sort erase + post-sort Nil slots
  (C4FindObject.cpp:217-223,372-375). Criteria parsing stops at the first
  nil par (C4Script.cpp:1996; was: skipped).
- Sort_Func (C4FindObject.cpp:934-956): cached once-per-object values in
  find order (PrepareCache), getInt() conversion, stable ascending;
  single-result Find-with-sort switched to the UNCACHED pairwise
  `Compare(candidate, best)` with obj1-then-obj2 evaluation order — fixes
  Random-draw-count parity for ALL sorts on FindObject2
  (C4FindObject.cpp:186-199,834-842).
- Material user-reactions: `MaterialReaction { kind, user_defined,
  insertion_check }` table entries; unknown/absent Type installs a
  default-overriding NoReaction ("Incinerate" is not user-nameable;
  mrfIncinerate asserts !fUserDefined); mrfUserCheck prologue with
  CheckSlide gate; user Convert fires on PXSMove (C4Material.cpp:38-46,
  612-634,683-787).
- Gates: lc-network flake fixed; clippy backlog found already clean
  (-D warnings passes workspace-wide).

**DFA_PUSH/DFA_PULL + jump physicals + GBackWind/IFT (2026-06-09).**
`C4Object::Push` force model (C4Object.cpp:1758-1808: OCF_Grab/Grab=2 gates —
Grab now DefCore-parsed and feeding OCF_Grab — force×100/Mass, close-enough-set
Towards, RotateAccel straightening); DFA_PUSH (:5040-5097) and DFA_PULL
(:5099-5170) exec with ValByPhysical(280, Walk)/(250, Push), got-hold/pulling
ranges with the GrabLost callback, ComDir transfer onto walking targets;
ObjectComJump launch velocities (C4ObjectCom.cpp:284-296, Con-scaled Walk/Jump).
GBackWind/IFT: the invented object-wind application REMOVED (C++ wind reaches
only PXS/particles, C4Wrappers.h:189-192; goldens regenerated), Landscape
tunnel(IFT) overlay + `Engine::wind_at`, position-dependent PXS drift,
positional `GetWind` host form. Remaining wind/physicals opens: IFT from
Landscape.txt (pixel landscape), Throw ejection + C4ObjectInfo (command epic).

**C4PhysicalInfo physicals model + DFA_FIGHT exec (2026-06-09).** First of the
five remaining epics. `lc-resources`: `PhysicalInfo` (21 i32 fields,
C4InfoCore.h:34-63) with `C4PhysInfoNameMap` name lookup, parsed from the
DefCore `[Physical]` section (defaults zero, C4InfoCore.cpp:181-205), and
`TrainValue` only-nonzero/cap/never-decrease (C4InfoCore.cpp:279-285).
`lc-engine/math.rs`: `val_by_physical` = `itofix(physical*(percent/5),
C4MaxPhysical*20)` with integer `percent/5` (C4InfoCore.h:224-227);
`towards` snap-within-step (C4Object.cpp:4561-4566). Engine:
`Definition.physical`, `Object.physical_override`, `object_physical` =
GetPhysical override→definition fallback (C4Object.cpp:2118-2134),
`train_physical` cloning the definition physicals on first training
(C4Object.cpp:2136-2146); `change_object_energy` clamps to the physical
Energy ceiling (C4Object.cpp:1361) scaled to percent points — zero-physical
fixture definitions keep the legacy unclamped ceiling (documented deviation).
DFA_FIGHT exec rewritten to C4Object.cpp:5200-5241: Tick5 fight training,
facing by target x, stand-beside `target.x ± (Shape.Wdt/2+2)` approach with
`lLimit = ValByPhysical(95, Walk)` `Towards` stepping (replacing the invented
MovementProfile-based approach), own-shape distance check after the approach,
grounded `ydir=0`; the Tick35 `DoExperience(+2)` waits on the C4ObjectInfo
model. **Second slice:** Walk/Scale/Hangle/Swim/Dig/Float ComDir movement
follows the C++ physical model when the relevant physical is nonzero —
`WalkAccel = FIXED100(50)`, `SwimAccel = FIXED100(20)`, `FloatAccel =
FIXED100(10)` (C4Movement.cpp:31-34), per-branch limit clamps
`ValByPhysical(280, Walk)`/`(200, Scale)`/`(160, Hangle)`/`(160, Swim)`/
`(125, Dig)`/`FIXED100(Float)` (C4Object.cpp:4771-5286), Scale/Hangle Tick5
and Swim Tick10 at-limit training, no gravity for Swim/Float (no DoGravity
call in either case), Swim faces by xdir sign. Physical-less fixture
definitions keep the legacy `MovementProfile` paths. Opens tracked in item 6.

**Particle system — item 12 (sim side) (2026-06-09).** Ported C4Particles into
`lc-engine/src/particles.rs` and corrected the audit's risk model: the C++
header declares the system "everything, that is not sync-relevant"
(C4Particles.h:18-27); all randomness is `SafeRandom` (wall-clock-seeded libc
`rand()`, C4Random.h:35,71-75) and `Create` scales MaxCount by the *local*
`Config.Graphics.SmokeLevel` (C4Particles.cpp:389), so particles cannot desync
the simulation. The genuine parity bug was script-visible: the cast/push host
functions were unregistered, aborting scripts where C++ returns `true`.
- `ParticleDefCore` (ctor + CompileFunc defaults), `ParticleDef` with
  `C4ParticleDef::Load` derivations (FadeOutLen length clamp, FadeOutDelay
  default, single-phase Reverse reset), def registry with GetDef order and
  particle-overload replacement, proc maps with GetProc-failure load errors.
- `Create` (C4Particles.cpp:378-419): SmokeLevel-scaled MaxCount, room check +
  SafeRandom rejection, Attach offset, init-proc dispatch, per-def counts.
- `Cast` (:421-443): exact per-particle draw order (xdir, ydir, a, 4 b-bytes),
  a-range ×100 swap, byte-split b-delta. `Push` (:494-519) with def filter.
- `fxStdInit`/`fxStdExec` (:600-697): vertex collision → collision proc
  (Bounce/BounceY/Stop/Die), RByV=2 no-move, gravity `fixtof(GravAccel*acc)/100`,
  WindDrift relaxation `/800`, AlphaFade (incl. negative periodic + the C++
  `iAlpha += AlphaFade` quirk), delay-phase lifetime with fade-out decay
  (post-decrement compare), off-landscape kill rules — pinned by an exact
  C++-derived trace. `fxSmokeInit`/`fxSmokeExec` (:521-576) with high-word
  init-status, `LightenClrBy` color ramp, wind/float behavior — exact trace.
- `SafeRng`: structural stand-in documented as deliberately unsynced.
- Engine: `particle_system` field; exec order object Back→Front then Global
  (C4Object.cpp:1071-1072, C4Game.cpp:814); snapshot/save-load round trip;
  def registry exposed to host contexts (script + scenario paths).
- Host fns: `CastParticles`/`CastBackParticles`/`PushParticles` registered;
  `CreateParticle`/`ClearParticles` get GetDef-failure → false when a registry
  is attached (C4Script.cpp:4874,4893,4917,4932); legacy def-less fixture path
  preserved and documented.

**Script value object identity — item 7 (partial) (2026-06-05).** Replaced the
old object-reference proplist shim as the primary representation with typed
object values:
- Added `Value::Object(u64)` for `C4V_C4Object`, including VM equality,
  truthiness, `type_name() == "object"`, `c4v_type()`, and deterministic
  `std::hash<C4Value>` parity hashing under the object type tag.
- Extended FFI with `LcScriptValueKind::Object` and an object-id payload, while
  preserving the existing Array/Proplist marshalling fields.
- Changed engine object-returning helpers (`object_reference_value`, contents,
  find-object/content helpers, action targets, `Contained`, etc.) to return typed
  objects; host parsers accept typed objects and the legacy `{id = ...}` proplist
  shim for compatibility.
- Effect variables now preserve object identity as `EffectVarValue::Object(u64)`
  instead of collapsing objects into lossy integers.
- Covered by `this` identity tests, FFI nested round trips, object-returning host
  helper tests, effect-var object round trips, full `lc-script --features ffi`,
  and full `lc-engine`.

**Script value references — item 7 (partial) (2026-06-05).** Replaced the VM's
synthetic `__funcref_*` placeholder with internal lvalue handles:
- Environment bindings now use shared cells, so reference parameters alias caller
  variables/slots/elements instead of receiving value copies.
- `func &` returns now return an internal lvalue for variables, `Local()`/`Var()`,
  array indexes, map/proplist properties, and chained reference-returning calls.
- Nested script calls share object-local named storage and `Local(n)` slots during
  a call session, matching the C++ object-local shape that `C4Value` references
  target.
- `type_name()` now reports `"map"` for `Value::Proplist`, matching
  `GetC4VName(C4V_Map)`.
- Covered by runtime tests for `&` param mutation, `func &` Local-slot mutation,
  and array/map element mutation through both reference paths.

**Script value FFI Array/Proplist marshalling — item 7 (partial) (2026-06-05).**
Extended the C ABI value shape beyond Nil/Int/Bool/String:
- Added `LcScriptValueKind::{C4Id, Array, Proplist}` and `LcScriptMapEntry`;
  `rust_value_to_lc()` recursively exports nested arrays/proplists with sorted
  proplist keys for deterministic ABI order.
- `lc_value_to_rust()` imports the same nested structures, and
  `lc_script_value_free()` now recursively frees strings, arrays, map-entry keys,
  and nested values.
- Covered by focused ffi-feature roundtrip tests for nested arrays/proplists,
  empty containers, and C4Id values.

**`C4ScriptCnvMap` conversion table — item 7 (partial) (2026-06-04).** Ported the
9×9 type-conversion table and `ConvertTo` dispatch from `C4Value.cpp:431-598`:
- `C4VType` (the `C4V_Type` tag, C4Value.h:37-54), `CnvFn` (the six converter
  classes from the C4Value.cpp:481-486 macros; `Warn` derived since it is a pure
  function of the class), and the table transcribed cell-for-cell into
  `value.rs` (`C4_SCRIPT_CNV_MAP`).
- `Value::c4v_type()` (the eager Rust model only maps `Nil`→`C4V_Any`;
  `C4V_pC4Value` remains internal VM reference state) and
  `Value::convert_to(to, strict)` mirroring `ConvertTo`: `CnvOK`→true,
  `FnCnvError`→false,
  `FnCnvDirectOld`→`!strict`, `FnCnvInt2Id`→int in `0..=9999`, `FnCnvGuess`→true
  (nil "is every type except a reference"; the Game-dependent `GuessType`
  data-nonzero path is unreachable because types are always known),
  `FnCnvDeref`→unreachable (no Rust reference value).
- Locked by a new differential section `script_value_convert` (oracle
  `oracle_main.cpp` transcribes the table independently; golden regenerated):
  the full 81-cell classification grid + 216 per-(value, target type, #strict)
  `ConvertTo` results. Negative control verified (corrupting one cell fails with
  `cell [1][3]`). RED→GREEN per increment; `cargo test --workspace` +
  `cargo clippy -p lc-script -p lc-engine` green.

**Theme C — fixed precision through physics (2026-06-04).** All ported physics
paths write authoritative `fixed_velocity`, integer mirror derived via `fixtoi`;
`sync_fixed_velocity_components_from_public` deleted. Covered: gravity
(`ydir += GravAccel`, `FIXED100(20)/5` = raw 13107, `C4Movement.cpp:643`),
friction (`ContactVtxFriction`, `:569`), collision resolution (sub-pixel retained/
zeroed at fixed level, `:266-282`), walk/swim/float/scale/hangle/dig accel
(`C4Object.cpp:4776`, accels are `const C4Fixed`), push/pull/fight/lift, wind
(`FIXED100(iWind)`). Gated by `parity verify` + targeted tests.

**C4Script VM operator parity — item 8 (2026-06-04).** Bit-exact fixes from
reading `C4AulExec.cpp` directly (each RED→GREEN, full suite green):
- `x/0`, `x%0` → `Int(0)` (`:504-526`, was: threw).
- `&&`/`||` return the surviving **operand**, not a bool (`:999-1021`).
- Binary/unary int ops coerce `nil→0`, `bool→0/1` via `Value::as_c4_int()`
  (the `_getInt()` mirror; `None` for String/Array/Proplist — pointer data);
  `-` uses `wrapping_neg`, div/mod use `wrapping_div/rem` (`:460-470`, `C4Value.h:170`).
- `this` → current object: VM threads a host `this` (`Vm::with_this`); all 8
  object-context call sites pass `object_reference_value(object_id)` (was `Nil`).
- Non-nil String/Array/Proplist are truthy even when empty (`C4Value.h:185`→`:76`).
- `==`/`!=` honor `#strict` (each `Function` carries its level; `<strict 3`
  compares Int/Bool/nil by value: `0==nil`, `1==true`) (`C4Value.cpp:823`).
- `..`/`..=` concat operator: lex + parse (priority 10, between equality and
  comparison) + eval (string-join/array-append/map-merge) (`:594-657`).
- Call-depth 64 → **512** (`MAX_CONTEXT_STACK`, `:62`) via `stacker::maybe_grow`
  (~10 KiB/level; `cc`/`psm`/`stacker` pinned for Rust 1.87).
- `Var(n)`/`Local(n)` numeric slots wired: dedicated function-scoped
  `var_slots`/`local_slots`; reads routed; negative index clamped to 0; `Local(n)`
  persists via object `local_vars` (`"__local_{n}"`), `Var(n)` per-call like
  `NumVars` (`C4Script.cpp:3390,3408`). (Note: `Var/Local` are separate scratch
  arrays, NOT aliases of named storage — the earlier "aliasing" analysis was a
  misread.)

**Phase 0 arrival fixes (2026-05-30).** The suite didn't compile on arrival
(unvalidated commit `e94e5052` added `local_vars` to `ObjectSnapshot` without
updating 5 fixtures). Fixed: fixture compilation; `Initialize` non-proplist return
now discarded like C++ (`C4Object.cpp:1483`); an infinite-loop test hang
(`command.rs`); a boot/scenario state-machine stranding bug (`main.rs
poll_boot_loading`); host-function checklist; 2 app-integration test pumps;
removed stray `*.bak` files.
