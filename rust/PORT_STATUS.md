# LegacyClonk Rust Port — Status & GAP LIST

> Living document. Last updated 2026-07-09. The C++ engine in `../src/` is the
> **golden oracle**; parity = bit-for-bit match on simulation state. This file
> tracks the CURRENT divergences only — full wave-by-wave history lives in
> `git log` (every behavioral commit cites the C++ file:line it ports).

## Current status

- **Pinned live shadow matches through frame 191 (2026-07-09, seed 424242).**
  The frame-184 SNKE #571 wall was its left vertex contacting CTWR #1351's
  alternate `Graphics2` solid mask. At mask source pixel `(219,86)`, the
  default `Graphics.png` is transparent and `Graphics2.png` is opaque. C++
  constructs `C4SolidMask` from `pForObject->GetGraphics()->GetBitmap()`;
  Rust always decoded the owning definition's default sprite and keyed its
  cache only by mask rect, so the snake missed `ContactLeft -> TurnRight`.
  Rust now resolves mask alpha through `ObjectBaseGraphics` (source definition
  plus named variant), separates variant cache entries, and immediately
  removes/re-puts masks when `SetGraphics` changes. The production
  `C4SolidMaskBitmap.h` differential freezes default=transparent versus
  Graphics2=opaque, and a real script-host test covers cross-definition
  selection and reset. The current live wall is **frame 192**: several animals
  diverge together in Walk/Turn/Jump state; the first report is object 577,
  Rust Jump/Right with velocity `(91750,-196608)` versus C++ Turn/Left at rest.
  The frame-170 WIPF #566 wall was `DFA_WALK`/`SetDir` ordering, not contact:
  Right steering changes raw xdir `-52430 -> -19662`, which still has a
  negative C4Fixed sign even though `fixtoi` rounds it to zero. C++ therefore
  calls `SetDir(Left)`, fires Walk's `TurnAction=Turn`, snaps fix_x to pixel
  541, then retains the old Walk `pAction` for phase advance; Rust tested its
  rounded velocity and assigned direction directly. Rust now uses the raw
  sign, full SetDir/TurnAction + fixed-resync semantics, and the pre-transition
  phase source. The production `C4ActionDirection.h` oracle freezes the exact
  result (Turn/Left, phase 1, time 0, fix_x 35435314).
  The preceding frame-157 animal mismatch was a downstream RNG symptom from
  frame 143: C++ saw the authoritative Water pixel at coarse cell `(52,24)`
  and consumed Water's `Random(10)`, while Rust consulted only its lossy column
  model. Grid-first occupancy is frozen by `C4LandscapePath.h` with one
  density-25 edge pixel in a 17x15 cell.
  Menu subsystem, persistent mass movers, PXS blast/border fidelity, rotated
  masks, the C++-order tick phases, live in-flight locals, crew join with
  fair-crew physicals and the C4-style HUD are all merged.
  Debug: pin runs with `LC_PIN_SEED=424242` (C4GameParameters.cpp,
  env-gated) so C++ runs are reproducible; get the map seed from
  `LC_DEBUG_MAP=1` (rust prints `RUST MAPSEED n`); replay headless with
  `LC_RUST_ENGINE_MAP_SEED=n`. Landscape dumps: `LC_DUMP_LANDSCAPE`
  (bridge), `LC_RUST_ENGINE_DUMP_LANDSCAPE` (runtime),
  `LC_XTASK_DUMP_LANDSCAPE` (xtask). RNG ledger traces:
  `LC_RNG_TRACE=<file>` (C++ temp probe in C4Random.h, re-add when
  needed) + `LC_RUST_RNG_TRACE=<file>` (committed, ledger-gated).
- **Script host order, DFA_WALK rotation, and raw rotation state CLOSED
  (2026-07-09).**
  Rotateable walkers now run the internal `AdjustWalkRotation(20,20,100)`
  ground-slope steering every WALK frame, including the offset-vertex gate
  and unconditional `rdir=0` fallback (C4Object.cpp:4817-4821,6031-6097).
  Definition resources, folder-local defs, scenario defs, pack System.c4g,
  scenario Script.c, and scenario System.c4g now register in C++ host order;
  constants, global-function overloads, and `#appendto` use that same order.
  Superseded defs retain preparser statics but lose functions/appends, and
  only `global func` exports from System hosts (C4Def.cpp:927-968;
  C4Game.cpp:81-103,2606-2622,3336-3355; C4AulLink.cpp:27-64).
  Loaded objects are followed by `InitializeDef` in numeric C4ID order with
  its no-object definition scope, then environment placement/weather,
  SyncClearance/Synchronize, scenario Initialize, and finally the queued
  startup-player join; save restore skips scenario Initialize
  (C4Game.cpp:112,456-483,2505-2520,2731-2734; C4AulExec.cpp:343-352).
  `static const` declarations share an immutable registry, accept signed
  literals, and overwrite existing cells like `RegisterGlobalConstant`
  (C4Aul.cpp:484-492; C4AulParse.cpp:639-650,3402-3422). Snapshots/FFI now retain
  raw signed `r`, `fix_r`, and `rdir` independently; `GetR` alone projects to
  `[-180,180]` like C++ (C4Object.cpp:2769,2789,2792;
  C4Script.cpp:1181-1188). Bare `CreateArray()` also follows missing-nil to
  zero-size conversion (C4Script.cpp:3807-3810).
- **Frame-30 bandit LoadRifle wall CLOSED (2026-07-06).** The GoldRush
  bandits now walk the full OrderDefend rifle-load chain at the first
  timer tick (FxOrderDefendTimer -> ExecuteWatch -> WINC::ControlThrow ->
  FireRifle -> CheckAmmo -> LoadRifle; Cowboy.c4d/Script.c:641-703,
  436-456, 499-504; Winchester.c4d/Script.c:7-31,289-299): action
  LoadRifle, rifles removed, WCHR crosshairs created, matching the C++
  f30 state. Five behavioral fixes landed: (1) Enter sorts contents like
  C4ObjectList::Add stContents (C4ObjectList.cpp:110-176 — Contents(0)
  is the newest equal-category item; loads stay verbatim); (2)
  cross-object numbered Local(i, pObj) by reference (FnLocal,
  C4Script.cpp:3423-3433); (3) effect callbacks fold nested-call
  outcomes to OTHER objects (run_effect_events_for_object dropped them)
  and get_world_object overlays the in-flight scope's action/damage;
  (4) effect callbacks run on LIVE session cells (nested write-backs and
  the outer call share storage chronologically); (5) newly-reached host
  fns: GetDamage, GetPlrColorDw, eval-as-DirectExec
  (C4AulExec.cpp:1658-1707) + int RemoveEffect fDoNoCalls. Goldrush
  headless 1000 ticks: zero warnings. Object-id skew (cpp 1534-1537 vs
  rust 1531-1534 for the crosshairs) is the separate numbering epic.
  2026-07-06 residuals closed: SetAction now zeroes Action.Phase
  unconditionally (C4Object.cpp:4132 — FireRifle's SetPhase(6) leaked
  into LoadRifle, the 'phase rust 6 cpp 0' wall), and GetDir honors an
  explicit target with NO object context (FnGetDir prologue,
  C4Script.cpp:1118-1122 — WINC->ActualizePhase is a definition call;
  the Nil bail flipped every crosshair vertex to +40, the 'object 1536
  vertices' wall: a real dir bug, not the id skew — the live comparator
  pairing was consistent).
  2026-07-06 f32 residual closed: the per-object script effect TIMER
  batch now executes where C++ runs pEffects->Execute — after ExecAction
  and ExecMovement, before fire/ExecLife (C4Object::Execute order,
  C4Object.cpp:1069-1090) — so an action set inside a timer callback
  gets its first PhaseDelay increment the NEXT frame
  (C4Object.cpp:5458-5466) and timer callbacks read post-movement state.
  2026-07-07 f60 residual closed: apply_container_change now carries
  C4Object::Enter's runtime semantics — a transfer's internal Exit
  mobilizes (C4Object.cpp:1579,1540-1541) and fCopyMotion snaps the
  entering object to the NEW container's position/velocity immediately
  (:1598-1606) — the reloaded CSHO cartridges sit at the crosshair with
  Mobile=1 the same frame like cpp; collect keeps its fCopyMotion=false
  stand-in (:5698).
- **Frame-21 rider-xdir wall forensics (2026-07-03).** The coach movement
  friction chain is now PINNED bit-exact against C4Movement.cpp and cannot
  be the wall: `pushed_wagon_loses_xdir_by_wheel_friction_quanta_*`,
  `wagon_hitting_a_step_redirects_half_pixel_*` and
  `pull3_walk_physical_yields_the_goldrush_pull_forces` (lib.rs tests)
  verify ApplyFriction (`FFriction*percent/100`, :50-56), first-contacted-
  vertex friction (:89-96), the vertical-contact chain (:297-317), the
  horizontal abort + RedirectForce(FIXED100(50)) + fix_x snap (:266-282),
  and the Pull3 forces (Pulling3 sets Walk=130000 → fWalk=238551, txdir
  = fWalk + fWalk*(-5)/10 = 119276 raw at BoundBy=-5, dforce =
  ValByPhysical(250,100000)*100/150 = 109226 raw). Rider CopyMotion
  timing also matches C++: contents (C4D_Object) exec AFTER the vehicles
  region in both engines (C4ObjectList stMain id-cluster inserts keep new
  contents inside the items region; C4Game::ExecObjects BeginLast ⇒
  ascending category), so at the riders' exec the coach xdir is its
  END-OF-FRAME value. NOTE for the next probe round: the reported cpp
  values (riders 21933 at end-f21, COACH-POST f21=0 → push 0→109226 →
  COACH-POST f22=19967) are mutually inconsistent with C4Object.cpp —
  towards(0,119276,109226)=109226 stands until frame end, and one
  DoMovement can cut at most 32768+19660+32768 (raw) so 109226→19967 is
  unreachable; but the values close into an exact per-frame cycle
  21933 −1966(wheel quantum)= 19967 −109226(push toward a txdir<−89259)=
  −89259 +1966 = −87293 +109226(push toward +119276) = 21933 — i.e. the
  cpp coach oscillated ±1.3px/f under a SIGN-ALTERNATING push txdir,
  which is impossible under a steady COMD_Right (txdir ≥ 0 with the
  BoundBy clamp) and impossible for COMD_Left at the rig's geometry
  (b clamps to +10 → txdir=0). Suspects to probe on the C++ side: (a) the horse's ComDir/Action.Dir timeline at
  f19-f23 (ContactRight→TurnLeft flip?), (b) whether cpp #1455-1497 are
  really coach-1450 contents (numbering skew), (c) the coach's xdir
  logged INSIDE the first content's CopyMotion at f21, (d) cpp Push
  post-value (CPPPULL logged args only).
- The two original foundational breaks (C4Fixed positions, ChaCha RNG) are
  long fixed: positions/velocities are 16.16 `C4Fixed`, `Random()` is the C++
  LCG with a shared ledger, and the join/init draw sequences are
  draw-for-draw identical.
- Scenario sweep: **93/93 load, 93/93 apply**. GoldRush headless runs 1000
  ticks with ZERO script-error warnings (including the intro movie).
- **Scenario-worlds epic (2026-07-03): 92/93 on the content-fidelity
  audit** (`cargo xtask scenario-audit` — landscape material histogram,
  objects by def, init-placement expectations; baseline was 71/93
  flagged). Dynamic maps generate: C4MapCreator (basic sine/liquid/layers,
  C4Map.cpp:73-167) and C4MapCreatorS2 (Landscape.txt exmap
  parser+renderer, C4MapCreatorS2.cpp) draw byte-exact on the FixRandom
  bracket (C4Landscape.cpp:578,734) and feed the classified ChunkOZoom
  plane. The C4Game init placements run at apply (InitVegetation/
  InitInEarth/InitAnimals/InitEnvironment/InitRules/InitGoals,
  C4Game.cpp:2493-2503) between the Gravity draw and Weather.Init's —
  the VegLevel/InEarthLevel evaluates draw even with empty id lists.
  Skies of Fire, Crystal Valley, Goldmine + all 27 formerly-flat dynamic
  scenarios build their real worlds (PNG-verified). The one remaining
  audit flag (CTF_DeepSea animals) is C++-consistent: its water is
  script-filled by _REF refillers AFTER InitAnimals, so C++ places no
  sharks at init either.
- Startup menus: all 6 screens pixel-exact vs C++ (95.4–99.8%, ±1 LSB).
  Scenario book content parity (2026-07-06): Title.txt language resolution
  (C4ComponentHost "XX:" lines, Title{code}.txt|Title.txt candidates,
  LanguageEx-style US,DE fallback), CreateEntryForFile extension rules
  (no more .c4d/.c4g phantom folders/duplicates), right page populated
  per UpdateSelection (Title.png in the ScenSelTitleOv frame, Desc??.rtf
  via a C4RTFFile::GetPlainText port, author/version lines), Open/Start
  button text, Choose-definitions checkbox rule, folder captions,
  Left/Right/Esc keys, double-click DoOK.
- Graphical in-game parity: NOT attempted (presentation layer, see below).

## Gates (definition of done per increment)

1. `cargo test --workspace` green (lc-network
   `control_sync_and_reconnect_smoke` is a known TCP-race flake; rerun).
2. `cargo clippy --workspace --all-targets -- -D warnings` clean.
3. `cargo xtask engine-snapshots verify` (Rust-vs-Rust determinism regression).
4. `cargo xtask scenario-sweep` 93/93.
5. `cargo xtask scenario-errors Goldrush` clean over 120 ticks.
6. `cargo xtask scenario-audit` 92/93 (the CTF_DeepSea animals flag is
   C++-consistent — see Current status).
7. Live re-measure: `scratchpad/shadow_measure_arm64.sh 45` (see Harnesses).

## Harnesses & debugging quick reference

- **Live shadow compare**: the Rust engine is statically linked into `clonk`
  (`build-arm64-native/`); per-frame `lc_engine_runtime_compare_snapshot`,
  'rust' = expected, 'cpp' = actual. The compare DISABLES after the first
  divergent frame — every histogram datum is from that one frame
  (frame-stamped). Build loop: `cd rust && cargo xtask ffi --release &&
  cd .. && cmake --build build-arm64-native --target clonk -j 8`.
  Measure: scratchpad `shadow_measure_arm64.sh <secs>` (variants: `_rng`,
  `_info`, `_dbg`, `_ctl`, `_rngtrace`).
- **C++↔Rust differential**: `cargo xtask parity verify` — C4Fixed math, LCG,
  Sin/Cos, sub-pixel accumulation, C4Value hash/convert vs a recorded golden
  (`cargo xtask parity record` to regenerate; see `parity/README.md`).
- **Headless forensics**: `LC_XTASK_OBJ_DUMP=<ids>` (per-frame object dumps),
  `LC_XTASK_PROBE_SHAPE`, `LC_XTASK_PROBE_SOLID` on
  `cargo xtask scenario-errors Goldrush --ticks N`.
- **RNG ledger tracing**: temp `LC_RNG_TRACE` probes in `C4Random.h` +
  `rng.rs` printing count/range/value windows; align reseed GENERATIONS
  (count resets at every FixRandom) before comparing draws.
- **Temp C++ probes** (spdlog in C4Object/C4Movement/C4Game) are the fastest
  oracle for semantics questions — always `git checkout src/*.cpp` before
  committing.

## Load-bearing engine semantics (discovered the hard way; easy to regress)

- **Exec order** = the C++ main list REVERSED (`Objects.BeginLast`), kept as
  a persistent `exec_list` maintained by `insert_into_exec_list`
  (C4ObjectList::Add stMain, C4ObjectList.cpp:110-216): ascending
  sort-category (StaticBack→Structure→Vehicle→Living→Object), file order
  within a loaded block (the save is written back-to-front and re-added
  stReverse); a runtime creation of an EXISTING def executes right after
  the last-executing member of its same-category def CLUSTER (Add pass 1 —
  the GoldRush intro _TLK execs before the later-joined player and reads
  its previous-frame ride position), a new def at its bracket's end;
  StaticBack skips clustering; Line defs exec first, newest first.
  Enter/Exit never reorder (C4Object.cpp:1513-1615 only move Contents).
  Containers must exec before their contained crew (CopyMotion reads
  post-move state).
- **Object ids never rewind**: C4Game::NewObj's `++ObjectEnumerationIndex`
  is strictly monotonic. Script-world counters written back from snapshots
  fold through `sync_next_object_id` (max), and snapshot-built worlds seed
  from the engine allocator, never `max(live ids)+1` — a burned same-frame
  id (GoldRush FXU1 flash) must not be re-minted for the next creation.
- **x/fix splits are legitimate**: DoCon's initial adjust writes int y only;
  `TargetBounds` clamps the int step target only (map-edge objects keep the
  outside fixed coord); `SetAction` resyncs fix (C4Object.cpp:4144);
  CopyMotion/ForcePosition/Exit resync. The snapshot integer position is the
  sim-state x/y, never `fixtoi(fix)`.
- **Phase advance** runs at the END of ExecAction, after procedure steering
  (reads THIS frame's velocity): WALK `|xdir|*10`, SCALE `|ydir|*14`, HANGLE
  `|xdir|*10`, SWIM `ValByPhysical(160,Swim)*10` (the PHYSICAL, not the
  velocity), DIG `ValByPhysical(125,Dig)*40`. Same-name NextAction chains
  fire EndCall+StartCall (no same-name gate, C4Object.cpp:5462).
- **SetAction callbacks run synchronously** inside the script call; the
  deferred event queue must never double-fire (`callbacks_dispatched`).
  Backstops: sync recursion depth >16, 32-event drain cap (log, not freeze).
- The ExecAction **default case with an ActMap `Attach`** zeroes dirs and
  mobilizes INSTEAD of gravity (C4Object.cpp:5426-5437); NoAttachAction's
  failed Jump (`SetActionByName("Jump")` miss) keeps the current action.
- **Crew order**: engine-internal newest-first (Add stMain); the bridge
  std::sort's HUD crew ascending — only the snapshot HUD mirrors that
  normalization. GetHiRank = first of equal ranks in newest-first order.
  Join runs AdjustCursorCommand (cursor = hi-rank, selected).
- **Appends resolve BEFORE includes** (SetAI appended to CLNK reaches BNDT
  through `#include COWB` → CLNK).
- **Init ledger draw order**: Landscape.ScenarioInit Gravity draw → Season →
  YearSpeed → Climate → Wind (`Random(151)`) → … C4SVal partial specs keep
  the C++ struct defaults for unspecified fields (Wind Min/Max = −100/100).
- **Per-object SolidMask overrides** decode their own sprite region; the
  mask-carrier filter must include objects whose DEF has no mask line.
- C4Aul precedence: unary `!` binds its operand only (`!A && B` =
  `(!A) && B`); the `!x = y` → `!(x = y)` speculative parse commits ONLY for
  actual assignments.
- Fixture rules: bare defs are StaticBack → movement tests need
  CATEGORY_OBJECT; direct fixed_velocity writes must arm `state.mobile`;
  `fixtoi` rounds-to-nearest.

## Comparator normalizations (bridge asymmetries, not sim gaps)

- The bridge exports no `C4GameMessageList` — hud messages compare only when
  both sides carry some.
- The Rust runtime exports no render surfaces — surface hashes compare only
  when both sides do.
- The bridge sorts HUD crew ascending before export.

## Accepted divergences

- `lc-script` accepts comma sequences in any expression context; C++ only
  inside `return (...)`. Rust accepts strictly more; real content uses the
  legal form.
- Nested-call locals: EFFECT callbacks now run on live session cells
  (2026-07-06) — nested calls and cross-object LocalN/Local references
  onto the effect's command target share its storage like C++. Other
  outer-call kinds (object callbacks) still hand nested calls a pre-call
  snapshot, and outer-call errors drop partial outcomes (C++ keeps
  pre-error mutations).
- Host-call par conversion (`CheckConvertFunctionParameters` strict-level
  matrix) is not ported; padding/nil rules cover real content.
- mrfScript material reactions resolve against the scenario script (C++ uses
  the global engine).
- The comparator skips particle-state equality (C4Particle is
  SafeRandom presentation, absent from C4ControlSyncCheck); opt back in
  with `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
- Same-seed landscape planes match C++ at 99.66% — the remaining 0.34%
  is C4SolidMask::Put's MCVehic bake-in (rust overlays masks instead of
  writing pixels; movement combines them at the density probes).

## Determinism-critical OPEN gaps

| Subsystem | Open items |
|---|---|
| movement-physics | SetSolidMask update lifetime; attached-object pushback (its DensityProvider reads the put BUFFER for rotated masks, C4SolidMask.cpp:218-227 — the rotated bake buffer already models this); rotated masks stay off in the non-grid mask-rect overlay (fixture worlds only, no C++ counterpart); three DORMANT non-C++ paths flagged 2026-07-03 (inert on pixel landscapes/GoldRush speeds, but no oracle counterpart): `apply_material_interaction` multiplicative xdir damping + vertex-friction overwrite (lib.rs, heightmap resolve_collision only), the engine-wide 12px `clamp_fixed_velocity` horizontal clamp (C++ DFA_PULL/updates never clamp), and `apply_landscape_at_index`'s post-movement resolve_collision |
| landscape | incremental ExecuteScan/DoScan (batch temperature conversion desyncs scan order) — including DoScan's per-converted-pixel CheckInstabilityRange (C4Landscape.cpp:225): the column conversion has no pixel coordinates to probe; PRETTY_TEMP_CONV; pixel-exact DigFree/BlastFree accounting; blast/shake instability probes run as a post-pass in the C++ scan order (no per-pixel clear/probe interleave until blast/shake are per-pixel); segment- vs pixel-liquid model. Map creators (C4MapCreator + C4MapCreatorS2) are ported; OPEN there: standalone runs use MapSeed=0 and RandomSeed=0 for the FixRandom bracket instead of the drawn `Random(3133700)`/network seed (the bridge hands the real values via `LC_RUST_ENGINE_MAP_SEED`/`LC_RUST_ENGINE_RANDOM_SEED`, players via `LC_RUST_ENGINE_STARTUP_PLAYERS`); `MapZoom` uses the clamped Std, not the bracket-internal Evaluate draw (no shipped content has a random MapZoom); Landscape.txt `evalFn=`/`drawFn=`/`algo=script` callbacks unsupported (no shipped content uses them, parse errors degrade to the basic creator like C++'s ignored ReadFile return, C4Landscape.cpp:540); C4Landscape::PostInitMap/KeepMapCreator (DrawDefMap) unported |
| effects | 2026-07-09 CLOSED: Annul/AnnulCalls + FxAdd add-to-other-effect (C4Effect.cpp:287-313 — the LAST checker answering -2/-3 wins; the new effect dies without Start/Stop; the acceptor's Fx*Add gets name/interval/rVal1-4; -1 from Add gives the acceptor a full Kill incl. Stop; AnnulCalls temp-brackets the Add with the acceptor's uppers, :297-304); TempRemove/TempReadd (a validating Fx*Start is bracketed by Fx*Stop(C4FxCall_Temp,fTemp=true) high→low + Fx*Start(C4FxCall_Temp) low→high, C4Effect.cpp:118-133/473-510; same bracket around Kill :365-405; ClearAll never brackets :407-425; prio-1 anchors/uppers skip callbacks :477,489,505); Stop_Deny recovery (Fx*Stop returning -1 on Kill keeps the effect, C4Effect.cpp:389-396); check-chain fidelity (deny short-circuits the walk :283-285, same-name peers ARE asked + all bookkeeping keyed by iNumber identity :278-282/76-78, Fx*Effect receives the pending rVal1-4 :282). STILL OPEN: AddEffect's return value cannot reflect deferred check outcomes — C++ returns 0 on deny, the acceptor number on merge, -2 when the acceptor killed itself (C4Effect.cpp:108-115,306-313), but the deferred event loop runs checks AFTER the host fn already returned the new number (only the synchronous prio-1 path matches); inactive (negative-priority) effects are not persisted between dispatch sequences — mid-bracket GetEffect priority queries and Kill's inactive arm (Fx*Start(C4FxCall_TempAddForRemoval), C4Effect.cpp:377-387, reachable only when a callback removes a temp-removed upper mid-bracket) are unmodeled, ditto removals landing INSIDE a bracket dispatch synchronously in C++ but queue behind the readds here; Stop_Deny recovery reinserts at the sorted position (equal-priority original order may shift) and applies only to Kill-removals — death-clear reasons (C4FxCall_RemoveDeath "return -1 to avoid removal", C4Effects.h:50) unmodeled (mark_destroyed drains as Destroyed without recovery); stop reasons stay the deferred string convention ("removed"/"temp"/…) vs C++ int C4FxCall_* — content comparing iReason ints diverges (pre-existing); Fx*Damage DoEnergy/DoDamage modification on the HOST scope path CLOSED 2026-07-09 (dispatch_effects_do_damage — the C4Effect::DoDamage do-while on the seam: first effect asked even at zero, iNumber-keyed live effects, pFnDamage existence gate leaves the value, errors fold to 0 like fPassErrors=false, mid-chain target removal aborts; wired into do_damage non-living C4Object.cpp:1282-1286, do_energy living post-scale :1347/:1355-1359 incl. the zero-outcome early return, and both BlastObject legs); builtin fire/helper effects (Splash/Smoke/Explosion/BubbleOut); 2026-07-09 CLOSED: GLOBAL (nil-target) effects dispatch their Fx* callbacks — tick_global_effects mirrors pGlobalEffects->Execute(nullptr) (C4Game.cpp:830-831; C4Effect::Execute C4Effect.cpp:319-363): elapsed intervals fire Fx*Timer(nil, iNumber, iTime) resolved per DoCall (command target script → command-id def script → engine-global fn table, C4Effect.cpp:439-456); C4Fx_Execute_Kill (-1) and the no-timer-function arm kill via C4Effect::Kill with Fx*Stop(nil, iNumber) (:342-357, :389-392), Stop_Deny recovers (:389-396), and kills temp-bracket upper effects (:365-405); AddEffect's GLOBAL scope runs Fx*Start(nil, iNumber, 0, rVal1-4) synchronously (ctor :128-131; Start_Deny removes without Stop); callback outcomes fold at the tick site like the object timer batch (RNG threads through in phase order). RESIDUAL (global effects): the priority check chain (Fx*Effect/Fx*Add, ctor :97-116) is NOT generated for global adds, and the no-command-target fallback dispatches through the first-registered definition's script host (which shares the engine-global fn table) — a definition-LOCAL Fx* name could shadow a same-name global there, whereas C++ reaches only Game.ScriptEngine |
| commands | 2026-07-09 MoveTo/Acquire/GetCommand pass CLOSED: MoveTo procedure arms ported — DFA_SWIM Tick2 horizontal / !Tick2 vertical steering (C4Command.cpp:370-382), DFA_SCALE vertical steer + let-go thresholds (LetGoRange1=7/LetGoRange2=30 target jump-off, any-contact let-go after Action.Time>2; :335-368), DFA_HANGLE horizontal steer + LetGoHangleAngle 110 drop (Angle is 0..360 so the C++ Abs is inert; :384-391), DFA_FLIGHT FlightControl-only no-steer arm (:414-417), DFA_PUSH/PULL horizontal steer from the pushed vehicle's position (:271-277, :329-333); ObjectComLetGo modeled as Jump action + fixed xdir launch (C4ObjectCom.cpp:310-314). MoveTo InitEvaluation ported (:1634-1643 via Execute :1555): first Execute consumes the frame, Target absorbs into Tx/Ty ONCE and clears (:1637), AdjustMoveToTarget grounds the destination (FreeMoveTo float/CanFly, :94-124) unless Data & C4CMD_MoveTo_NoPosAdjust; DFA_PUSH lets go (UnGrab + Evaluated reset :257-265) on Grab=2 targets or Data without C4CMD_MoveTo_PushTarget (C4Command.h:68-69); Enter forwards its PushTarget flag onto the entrance MoveTo (:615). Acquire InitEvaluation Tx/Ty defaults confirmed 500/250 replacing ZERO only — the invented .abs() dropped so negative ranges match nothing like Inside(cx-px,-Tx,+Tx) (:1666-1670, :2115-2116). GetCommand element views now read the LIVE fields (request base + MoveTo/Acquire/Construct live-state overrides) and the creating request persists through CommandSnapshot — frame-start world-context views and save/load restores keep their elements (pre-fix saves degrade to name-only). Transfer's ControlTransfer call was already Tick5-gated (frame%5==0, :1931). REMAINING GAPS: the Tick35 PathChecked recheck (:255) needs MoveTo's whole path-check arm (:228-253 — Game.PathFinder.Find + ObjectAddWaypoint MoveTo/Transfer waypoint pushes with Data propagation and fEvaluated=false, :189-209) which is blocked on a real C4PathFinder port (rust pathfinder.rs is a surface-heightmap approximation used only by GetPath); with no waypoint stack the fWaypoint easings are unmodeled (push position-override skip :273, crew waypoint range factors 3/3/2 :294-299); MoveTo contained→Exit (:222-226) and the DFA_FLOAT C4Fixed steering arm (:393-412) unported; MoveTo Idle fail (:310-314) and the C++ UpdateInterval countdown Finish(true) (Execute :1544-1552) still diverge from the rust every-N-frames throttle/arrival model (crew arrival uses invented tolerance 5 + 2-frame dwell instead of Shape.Wdt/5 + range factors :285-308); FlightControl's ActMap Disabled gate (:1823-1827) and Def->Pathfinder alternative to OCF_CrewMember unmodeled; Acquire defaults apply at Set (C++ waits for the first Execute) so a GetCommand between AddCommand and Execute already reads 500/250; Put's live Ty reminder-flag rewrite (:1384) unmodeled |
| player-controls | 2026-07-06 classic DirectCom chain PORTED (direct_com.rs): C4Player::InCom single/double synthesis + PressedComs + LastCom timeout in the player execute (C4Player.cpp:1490-1554, 1215-1232); CallControl for EVERY com incl. directional (C4Object.cpp:3385-3389); classic per-procedure fallbacks — ObjectComMovement/Stop/Up/Dig/DigDouble/DownDouble/LetGo/UnGrab, PlayerObjectCommand throw→drop conversion (C4Object.cpp:3406-3556, C4ObjectCom.cpp); ContainedControl script-early + hardcoded Down-exit/Throw/Take/Take2 (C4Object.cpp:3219-3305); JnR AutoStopDirectCom + ControlUpdate/ContainedControlUpdate (C4Object.cpp:3559-3741) gated on PlayerControlState.control_style (default classic; player-file AutoStopControl pref not wired yet); CursorFlash/SelectFlash timers ported. Enter/Grab commands now use the C++ At() point-in-shape test without the invented aliveness gate (C4Command.cpp:586-588, 689-691). REMAINING GAPS: wheel-com ShiftContents + COM_Contents shift stay app/menu-side (C4Object.cpp:3364-3396); NoCollectDelay decrement unmodelled (:3359-3362); ContainedControl base buy/sell menus (:3269-3280) and immediate ExecuteCommand on contained throw (:3267) missing; def-version gates (fCallSfEarly >=4,9,1,3 / grab overload >=4,9,5,0) assume modern defs — DefCore Version unparsed; VehicleControl inside-control overload in SetCommand (C4Object.cpp:3944-3957) missing; cursor-com crew cycling still frontend-approximated (C4Player.cpp:1240-1330 selection model incl. UpdateSelectionToggleStatus unported); ObjectComDigDouble linekit line construction (C4ObjectCom.cpp:379-529) unported; IDS_OBJ_NODIG/NOCHOP messages display-only missing. 2026-07-06 follow-up: SAME-SCRIPT function redefinitions now chain to the earlier definition via inherited (C4AulParse.cpp:1404-1406; the Coach.c4d `[$TxtGetoff$] return(inherited(...))` menu wrappers) — GoldRush double-Down dismount verified end-to-end on real content (Ride→Jump→Walk) via the app harness (`LC_APP_TEST_INPUT` + per-frame cursor action-transition log). 2026-07-07: DoMagicEnergy ported (see host-fn backlog row) — the inherited chain to engine fns is closed for it. Coach-dismount re-verified on the current tip: double-Down from the parked coach is clean (RideStill→Walk at frame ~1106, NO Ride re-entry over 150 further frames); the earlier 'instant remount by landing on the coach' report is NOT reproducible, and the C++ oracle has NO landing-based remount path — Ride/RideStill are ATTACH actions set exclusively by script (Coach ContainedUp `ObjectSetAction(pByObject,"Ride",this())`, Coach.c4d/Script.c:47-58; C4Object::ContactAction C4Object.cpp:4321+ maps contacts to coms and never sets ATTACH), so any future remount-on-landing would be a rust divergence. Verification gotcha: the GoldRush intro movie swallows ALL crew coms for its full ~1000-frame run via the Talker CLNK append (`if(bTalking) return(1)`, Talker.c4d/Append.c4d — every Control* handler) — correct C++ behavior; scripted dismount input must come after MovIntro9's StopDialog. STILL OPEN: C4Game::DrawCursors name text (red rank|name above the flashing cursor, C4Game.cpp:1874-1888) not drawn |
| material | column-model fixture worlds keep segment removal where C++ ClearPix/ExtractMaterial act per pixel (grid worlds are C++-faithful) |
| objects-core | OCF computes a subset of the ~30 C++ checks; C4ObjectInfo permanent training/experience unmodeled; DFA_CONNECT uses direct endpoint assignment (the LineConnect wrapping walker is unported) |
| findobject-ocf | 2026-07-09 explosion epic CLOSED the Layer class: Find_Layer carries Data[1]'s object and compares `pObj->pLayer == pLayer` (C4FindObject.cpp:671-674, nil = unlayered world), BlastObject is ported (FnBlastObject C4Script.cpp:2281-2289 -> C4Object::Blast C4Object.cpp:1414-1424: staged DoDamage + synchronous ~Damage, alive −level/3 energy percent w/ EngBlast kill trace, LIVE post-callback Damage vs Def->BlastIncinerate) with the host-path incinerate mirroring incinerate_object_inner (refusals, extinguisher-before-draw, BurnTurnTo changedef, contents ejection, ONE FirePhase=Random(15) mid-call ledger draw, Incineration/IncinerationEx fail-safe; fire bits ride ObjectUpdate::fire to both folds), DefCore BlastIncinerate/ContainBlast/HorizontalFix ingest + GetDefCoreVal Grab/HorizontalFix/ContainBlast/BlastIncinerate/ContactIncinerate entries (the GetXVal.c wrappers BlastObjectsShockwaveCheck + DoExplosion read) — System.c4g BlastObjects finds, damages, and flings explosion victims. STILL OPEN: the ENGINE fallback trio FnExplode/Explosion/Game::BlastObjects + FnShakeObjects (C4Effect.cpp:877-925, C4Game.cpp:1243-1330) is unported — unreachable by shipped content (System.c4g `global func Explode`/`BlastObjects` override the engine fns and never call inherited; C4Object::Explode is "called by FnExplode only"), so it matters for fixture worlds/inherited chains only; host-path incinerate does not add the "Fire" C4Effect entry (same builtin-fire-effect gap as the engine path, see effects row); sector-bounds FindMany traversal order; cached sort keys i64 vs C++ i32 wrap |
| game-control-record | control-packet payload serialization; lc-network DoInput/host sync-check broadcast wiring |
| players-crew-teams | team home-base production sync (C4RULE_TeamHombase); CheckElimination CLOSED 2026-07-09 (Tick35-gated, one-way like C4Player::Execute -> CheckElimination -> Eliminate, C4Player.cpp:225-235/1680-1690/2015-2017 — the instant recompute + resurrect-on-new-crew divergence is gone; residuals: RetireDelay/sound/log presentation not modeled, script-player CSPF_NoEliminationCheck unmodeled since script players are unported, PS_Normal gate approximated by PlayerStatus::Active); asset value stub; crew infos not persisted in snapshots |
| definitions-id | runtime dispatches on procedure strings not numeric ActMap indices; GetComponents override; CalcDefValue; some DefCore flags unparsed |
| script-values | C++ string-table interning/refcounts; save/load + net sync of values |
| weather-sky | 2026-07-09 epic CLOSED (all three items): (1) SetSeasonGamma is PRESENTATION-ONLY — there is NO FnSetSeasonGamma host fn (C4Script.cpp has only SetGamma/ResetGamma); C4Weather::SetSeasonGamma (C4Weather.cpp:259-285) only writes Game.GraphicsSystem.SetGamma(..., C4GRI_SEASON) and nothing sim-side ever reads gamma back (no script getter; the dwGamma block in C4Weather::CompileFunc :302-309 is display restore). The ramp math is ported as the derived `EnvironmentSettings::season_gamma()` getter, pinned by tests to hand-computed oracle values (truncating blends, Temperature/2 winter shift, Season=100≡0, NoGamma gate). NUANCE (visual only, not modeled): C++ refreshes the APPLIED ramp only at Init/season-advance/SetSeason/SetTemperature/SetClimate, so between triggers C++ shows a stale ramp vs the live-recomputed Rust getter. SetGamma/ResetGamma now registered as accepted no-ops (C4Script.cpp:5004-5013 → C4GraphicsSystem.cpp:772-786) — the unknown-function error was ABORTING the rest of Initialize in Tutorial06/07/10, ArcticOcean, Chasm, Clepal, PolarNight; renderer ramp still unmodeled. (2) Season advance wraps exactly like C4Weather::Execute (C4Weather.cpp:77-85): delay resets to ZERO (not modulo), ONE step per Tick35 regardless of YearSpeed overshoot, no YearSpeed==0 gate (preloaded delay still advances), no negative-delay regression; wrap is `Season > StartSeason.Max → Season = StartSeason.Min` on new `season_min`/`season_max` ingested from the scenario C4SVal (season 100 reachable under default 0/100 bounds; Max<Min pins at Min); Init drops the invented 0..100 clamp (C4Weather.cpp:41 — Evaluate is already Min/Max-bounded). Temperature step now uses the C++ `Climate - int32(TemperatureRange*cos(6.28*Season/100.0))` double-cos TRUNCATION (literal 6.28, not tau; round() was off by one degree over half the season range) with no TemperatureRange gate (:88-93). SetSeason/GetSeason host fns registered (C4Script.cpp:3025-3033; BoundBy 0..100 C4Weather.cpp:229-233). (3) Sky scroll is bit-exact C4Sky::Execute fixed-point (C4Sky.cpp:193-204): SkyState holds C4Fixed x/y/xdir/ydir; NO advance without a surface (:196); position moves by the PREVIOUS frame's dirs — the wind refresh to FIXED100(Wind) happens AFTER the move (:198 vs :203, replacing the f32 wind/100 projection with the truncated 16.16 quotient); single-subtraction wrap only at `>= itofix(size)`, never upward (:200-201). SkyFrame carries the raw fixed [x,y,xdir,ydir] beside the float projections the renderer reads, and EngineState persists/restores it (C4Sky::CompileFunc :248-251 mkCastIntAdapt raw bits; savegame Init keeps loaded values :77-80). SetSkyParallax ported (C4Script.cpp:4955-4970: SkyPar_KEEP=-163764, nil/missing args are 0 and ZERO the slots, mode assigns only Inside 0..1, zero ParX/ParY ignored as Draw divisors, xdir/ydir/x/y itofix) routed as a LandscapeOperation (Sky is a C4Landscape member) into SkyState::apply_parallax. SetSkyAdjust ported (C4Script.cpp:4626-4630 → C4Sky::SetModulation C4Sky.cpp:238-244, alpha-gated BackClrEnabled as the back_color Option). STILL OPEN (all presentation): sky x/y feed only C4Sky::Draw (serialized deterministic state, in no sync hash); SetSkyFade unported (0 content users; needs GetClrModulation); C++ keeps a DISABLED BackClr around where the Option drops it (scripts always set both in one call); the SkyDef tile list draw (SeededRandom pick, C4Sky.cpp:88-105) still falls back to the fade gradient |
| config-info | GetAName file-based names partial; PromotionUpdate; locale/control-pref defaults |
| resources-groups | no group write/create/gzip-out/CRC32-at-open; directory iteration order may differ from C++ |
| network | password auth, voting, league, NCS_* client status, join-data save/restore, protocol negotiation |
| host-fn backlog | 2026-07-03 mostly CLEARED: ShiftContents full semantics (foreign pObj via the nested seam; fDoCalls ~ControlContents veto + ~Selection + Grab sound, C4Object.cpp:5754-5775 — CanConcatPictureWith still approximated by definition-id equality, menu Refill is presentation); GrabObjectInfo transfers crew flag + portrait + info permanent physicals (C4Object.cpp:5715; name/rank/experience payload unmodeled — no per-object name); SetAction nil targets PRESERVE like `if (pTarget) Action.Target = pTarget` (C4Object.cpp:4123-4125 — was the "HORS action-start target-zero" class); DoDamage/DoEnergy reach foreign targets with caused-by kill tracing (ObjectUpdate.energy_loss_cause; UpdatLastEnergyLossCause guard C4Object.cpp:1369-1378 on host + engine paths; Punch carries the attacker's controller + unguarded post-fling write C4ObjectCom.cpp:749,755,762; DoDamage fires the ~Damage callback C4Object.cpp:1290; nested-fold AssignDeath when a foreign write zeroes energy). 2026-07-07 SkiesOfFire residue CLEARED (200-tick headless run free of all five warning classes): FindConstructionSite host fn stages through the CALLER's Var slots — new lc-script seam `caller_var_slots()` exposes the calling env's slot table to host fns (cthr->Caller->NumVars, FnFindConstructionSite C4Script.cpp:1958-1981), wired to the already-ported FindConSiteSpot (C4Landscape.cpp:1987-2043) with the Game.OverlapObject veto (C4Game.cpp:1298-1313, C4Rect::Overlap); pre-#strict-2 constant CALLS (`MCLK_ComboExtraDataName()`) resolve script `static const`s via a shared engine-global constant registry (RegisterGlobalConstant C4Aul.cpp:484; old-style usage C4AulParse.cpp:2834-2864, parameters error like Match(ATT_BCLOSE)); DoMagicEnergy/GetMagicEnergy ported (C4Script.cpp:517-550, MagicPhysicalFactor 1000 C4Object.h:81) with C4Object::MagicEnergy on the object model (scope overlay + SpawnConfig + Objects.txt MagicEnergy=, C4Object.cpp:2768) so NoMagicEnergy.c4d's global overrides chain to the engine fn via inherited; Get/SetPlrExtraData (C4Script.cpp:4692-4747; IsIdentifier name gate StdCompiler.cpp:92-100; nil/int/bool/id payloads only) for MAGE Recruitment; SetTransferZone resolves the in-flight object mid-Initialize (pObj->x/y off the live object, C4Script.cpp:3151-3156) and spawn-time zone commands DEFER until the object joins self.objects (C++ adds it to Game.Objects BEFORE Construction/Initialize fire, C4Game.cpp:1115-1131 — the WZKP homebase no longer aborts join_player). 2026-07-09: System/scenario `static const` declarations register into the shared constant table and duplicate declarations overwrite the existing shared cell like C++ RegisterGlobalConstant. 2026-07-07 CLEARED: GOAL CheckTime's `curr_goal->LocalN("missionPassword")` (Goal.c4d/Script.c) — the ARROW-form READ of the global by-reference engine fns LocalN/Local now resolves the TARGET object's named/numbered local through the cross-object cell hook instead of world method dispatch (vm.rs invoke_property_call; only the bare Expr::Variable form was special-cased before, so `obj->LocalN(..)`/`obj->Local(n)` fell through to "No function LocalN in object N"; content has 14 `->LocalN` + 2 `->Local` sites). Verified end-to-end: SkiesOfFire 600-frame headless run (`--integration-test`) fires MsgTooFewPlayers/CheckTime with ZERO script-error warns. GetPlrJumpAndRunControl registered (FnGetPlrJumpAndRunControl C4Script.cpp:2579-2583 → ControlStyle, else -1; fixes MAGE ControlUp `unknown function`). GAP (pre-existing, unreachable by shipped content): the getter reads player.control.control_style (player-file AutoStopControl pref, a bool) and does NOT apply the scenario `[Head] ForcedAutoStopControl` override (parsed to head.forced_control_style scenario.rs:1246/1895 but applied to no player) — 0 shipped scenarios set ForcedAutoStopControl and AutoStopControl is a 0/1 checkbox, so no divergence today, but a forced/out-of-domain control style would differ from C++'s live int32 ControlStyle. 2026-07-09: GetKiller/SetKiller ported with default-self targeting, ValidPlr/NO_OWNER validation, direct foreign/arrow writes, and same-call read-after-write visibility (C4Script.cpp:1333-1347). 2026-07-09 CLEARED: AddEffect nil-fills a MISSING priority to 0 and returns 0 without creating like `if (!iPrio) return 0` (C4Script.cpp:5449; the command-DSL fixtures now pass explicit priorities), and a missing pTarget slot no longer panics (nil = global scope). 2026-07-09 CLEARED: BlastObject + Find_Layer + the GetDefCoreVal blast entries (see findobject-ocf). 2026-07-09 also CLEARED: the Fx*Damage host-scope dispatch (see effects row — do_damage/do_energy/BlastObject all ask the target's effects like C4Object.cpp:1282/1355; script DoDamage now threads its iDmgType arg through instead of discarding it); nil-target AddEffect never ran Fx*Start/Timer for System.c4g ShakeEffect in the Goldrush headless probe (Distance/SetViewOffset were unreachable until 2026-07-03) — CLEARED 2026-07-09: GLOBAL effects now dispatch the full Fx* lifecycle (see effects row; ShakeViewPort pinned end-to-end in scenario.rs with the real Explode.c — Start inside AddEffect, Timer every frame, kill at iTime 29 for level 100) |
| object menus | sim-observable core modeled on `ObjectState.menu` (CreateMenu/GetMenu/CloseMenu/AddMenuItem/SelectMenuItem incl. MenuQueryCancel + OnMenuSelection callbacks, C4Script.cpp:1418-1741, C4ObjectMenu.cpp:56-104). 2026-07-09 CLEARED: menus close on ALL FOUR C++ lifecycle events — Enter/Exit force-close at the moment of the container move (CloseMenu(true), C4Object.cpp:1555/:1594; script scopes stage the close in `set_container` so same-call GetMenu sees it and a LATER same-call CreateMenu survives; engine-internal movers close at both container folds — apply_delta + apply_object_update — guarded by an explicit menu write in the same delta), SyncClearance (C4Object.cpp:3842, game_start_synchronize), and control SetCommand's SOFT close `if (!CloseMenu(false)) return;` (C4Object.cpp:3944-3946 — Engine::close_object_menu asks MenuQueryCancel with the shared CloseQuerying guard; denial keeps the menu and aborts ObjectCommand2Obj's Set arm with the stack already cleared). GetMenuSelection (C4Script.cpp:4310-4316), SetMenuSize (C4Script.cpp:4483-4492 → C4Menu::SetSize keep-zero-axis, BoundBy 0..50; columns/lines on ObjectMenuState), SetMenuTextProgress (C4Script.cpp:1750-1754, NO cthr->Obj fallback; text_progress state) and SetMenuDecoration (C4Script.cpp:1737-1748, NO fallback, gated on a known deco def; decoration state) ported. MenuCommand execution on user Enter ported as `Engine::menu_user_enter` (C4Menu::Enter, C4Menu.cpp:498-523: Style_Info refuses, no-selection keeps non-dialogs/soft-closes dialogs, Command2 on right enter, non-permanent closes BEFORE the exec; C4ObjectMenu::MenuCommand CB_Object DirectExecs the command string on the command object — new lc-script `direct_exec_with_locals_and_this`, C4AulExec.cpp:1658-1707, parse errors yield nil, runtime errors fail-safe). OPEN: lc-app renders only its own Activate/Get/Contents UI — script menus are NOT drawn and no input path routes COM_MenuEnter(All)/COM_MenuClose/COM_MenuSelect to menu_user_enter/close_object_menu/SelectMenuItem (C++ queues them via Game.Input, C4ObjectMenu.cpp:461-477), and those engine-internal app menus stay invisible to GetMenu (C++ returns C4MN_Activate etc.); CB_Scenario menus (command_object None): MenuCommand on Enter logs+skips and the ENGINE-side soft close skips the MenuQueryCancel query (host-fn close path handles both, C4ObjectMenu.cpp:64-70/523-526); AutoContextMenu tail after MenuCommand (C4ObjectMenu.cpp:528) + CloseCommand invocation on TryClose(fControl) need Def->AutoContextMenu/player prefs — unported; contents ejected by container REMOVAL exit via a direct apply_container_change and keep an open menu (C++ Exit closes, fringe); Enter into the SAME container skips the re-close (C++ exits+re-enters, both close); presentation not modeled: menu layout from columns/lines, per-item TextDisplayProgress distribution (C4Menu.cpp:1085-1108), FrameDeco facet/border queries + SetByDef's script-not-ready failure arm (C4GuiDialogs.cpp:115-116); menu state is runtime-only (not in ObjectSnapshot save/load — C++ Objects.txt drops menus too) |

## Presentation-layer gaps (non-sync)

| Subsystem | State |
|---|---|
| graphics | ~25%: per-pixel blit only; no transforms/GL/shaders/landscape rendering |
| audio | ~35%: panning math differs; C4Sound/C4MusicSystem high-level layers absent |
| gui-menus | no DrawElement rendering, layout, text progression, portraits |
| startup-launcher | player-selection dialog stub; no update check/first-start UX. Scenario book (C4StartupScenSelDlg) data+interaction parity done 2026-07-06; OPEN: search-bar typing (no ReceivedCharacter plumbing at all in lc-app), CanOpen greying of rows (needs Participants count + C4Scenario::GetMinPlayer incl. IsMelee), custom Icon.png/legacy Icon.bmp list icons, F2 rename/Del delete/F5 refresh/Alt+M mission access, FolderMap.txt map-style folders, list scrolling past one page |
| player menu (C4MainMenu) | 2026-07-06: Escape menu is now the faithful C4MainMenu port (lc-app ingame_menu.rs): entry lists/captions/icons/order per ActivateMain/Options/Display/Savegame/Surrender/Goals/Rules/NewPlayer (C4MainMenu.cpp:643-715 etc., LanguageUS.txt strings), classic C4Menu context-style furniture (0x5f bg + 3D frame, GUICaption wooden bar, CRed #c80000 selection, 35px left-bottom alignment, GfxR facets from Menu/Options/Control/GUIIcons/Player.png, bottom command-key bar, 90-frame info tooltip; C4Menu.cpp:642-880), C4Menu navigation (wrap, permanent menus, close commands — Escape in a submenu returns to Main). Save slots map ScenName{N}.c4s → sanitized `{title}{N}.lcsave` via the existing JSON save path; named save/load browsers (rust extras) moved to F6/F7, F5/F9 quick save/load kept. Wired for real: Sound/Music toggles (audio options), Mouse control (gates mouse gameplay drags), Portraits (HUD crew portraits), Commands/Keys (menu command bar), Surrender (local rounds: set_player_surrendered → engine game-over), Abort/Restart (menu-page approximation of C4AbortGameDialog; restart relaunches the active scenario). OPEN gaps: C++ binds Escape to C4AbortGameDialog and the player menu to COM_PlayerMenu (rust PlayerMenu key opens the object menu first, Escape opens the player menu; the abort dialog is a menu page, no C4GUI dialog, no Game.HaltCount pause); Goals lack the fulfilled-captain overlay (C4RoundResults::EvaluateGoals) and goal/rule Enter doesn't open the info menu (CID_ActivateGameGoalRule); Join player lists nothing (.c4p discovery/runtime join unported — shows the C++ IDS_MENU_NOPLRFILES empty caption) and JoinPlayer only logs; Hostility page, host kick list and the observer page reopen Main with a status note; team switch never shows (Teams.txt unported); MaxPlayers uses the C4S default 12 (Scenario.txt Head not surfaced to lc-app); Display toggles for player/clonk names, title-board modes, FPS, clock, white chat flip session flags + icons only (no renderer effect, not persisted to config); networked Surrender/Part blocked with a status note (control-queue routing missing); no scrollbar graphic on overflow (selection auto-scrolls); item-caption markup (`<c>`) not interpreted; tooltip uses FontRegular + greedy word wrap (C++ TooltipFont/BreakMessage) |
| startup-launcher | player-selection dialog stub; no update check/first-start UX |
| particles | sim side done (SafeRandom by design, non-sync); Particle.txt gfx loading + draw procs open |
| in-game HUD | C++-faithful chrome done (upper board/logo/title/Game.Time, DrawPlayerInfo fixed items, cursor portrait+rank+name, energy bar, startup keyboard+name, one-line message board; oracles C4UpperBoard.cpp:46-96, C4Viewport.cpp:884-965/1281-1476, C4ObjectInfo.cpp:302-371, C4MessageBoard.cpp:243-306). 2026-07-07: viewport COMMAND ROWS ported (DrawCursorInfo commands C4Viewport.cpp:947-962 -> C4Object::DrawCommands/DrawCommand C4Object.cpp:2940-3098/4018-4078): bottom bar right-to-left + side strip bottom-up in 2*C4SymbolSize/3 cells (TruncateSection C4Facet.cpp:182-215), fctKey cap + fctCommand phase (Com2Control x / COM_Double row y) + FontTiny key labels under ShowCommandKeys, image cells (def picture / 85% picture+fctBuild/fctHand composites / fctExit / DrawMenuSymbol Buy-Sell); decision tree in lc-app draw_commands.rs (build at OCF_Construct via the AtObject enclose test incl. 18px build-top, grab Control<Com>+let-go/put/get on the new GrabPutGet DefCore bitfield, contained Contained<Com>/exit/take/put with CON_Down/Left/Right overrides, contents-vs-self Activate, ComOrder 6,7,14,15,22,23 specials); function [Image=ID:phase or Contents] descriptors extracted textually from the retained def script source across the #include chain (lc-script drops descriptor metadata). Cursor rank-name label ported (C4Game::DrawCursors C4Game.cpp:1873-1888: red 0xffff0000 rank line + name centered above the flashing mark; DEFRANKS table names). Floating energy/magic bars + persistent bolt REMOVED from the world draw (non-C++; energy lives in the HUD corner, C4Viewport.cpp:920-945). OPEN: contents/inventory row (Contents.DrawIDList), DrawPlayerControls, DrawMouseButtons + keyboard PlayerMenu key (C4Viewport.cpp:856-880), board scroll/fade anim, player-colored board lines; command-row residuals: Method= descriptors (unused by base content), #appendto'd control-function descriptors, FlashCom blink (Tick35>15, C4Object.cpp:4071 — FlashCom unmodeled), contained Buy/Sell needs C4Object::Base (unmodeled), NeedEnergy world bolt (Tick35>12 blink, C4Object.cpp:2505-2510) needs engine NeedEnergy modeling, def-custom Rank.txt rank-name overloads not loaded |

## Known harness issues

- **MIDI music cannot play** (`Cannot play music file …: No SoundFonts have
  been requested`, ~1500×/run): the C++ SDL2_mixer fluidsynth backend has no
  GM soundfont configured. Not a port defect. Fix: install a .sf2 and export
  `SDL_SOUNDFONTS`, or set `Music=false` in
  `~/Library/Preferences/legacyclonk.config`.
- lc-app sim-tick script errors still exit the app (event loop) — needs the
  same fail-safe treatment the engine has.
- FLNT `Hit` script errors headless: RESOLVED 2026-07-09 — the inner
  error was the unported `BlastObject` host fn (Explode ->
  DoExplosion -> BlastObjects -> BlastObject, Explode.c). The whole
  chain is pinned end-to-end through the REAL planet/System.c4g scripts
  (`real_system_scripts_explode_blasts_bystanders_end_to_end`,
  scenario.rs): self-removal, direct-hit Damage, shockwave branch, zero
  script errors; Goldrush 1000-tick headless stays clean with the
  explosion path live.
- The headless xtask world skips the player join: GoldRush intro-driven
  state (cavalry recruitment draws, coach splash) reproduces only with
  the live pinned harness.
- **14-scenario script-loop hang epic: RESOLVED 2026-07-06.** All 14
  formerly-hanging scenarios now join + run 100 ticks headless with zero
  tick failures. Root cause was one class — the copy-in/copy-out host
  seam did not read back mid-call staged mutations, so script loops that
  terminate in C++ (which mutates the live C4Object) never saw their own
  progress. Three shapes, all fixed at the staging boundary
  (`EffectHostContext::get_world_object` + the writer host fns):
  - FindObject + RemoveObject dedup (`Time.c4d`/`Driftwood.c4d`
    `Initialized`: `while(pOther=FindObject(GetID()))
    RemoveObject(pOther)`) — C++ AssignRemoval sets Status=0 IMMEDIATELY
    (C4Object.cpp:282) and C4Game::FindObject skips Status==0
    (C4Game.cpp:1360-1365); nested-scope destroys now overlay as Deleted
    status. Fixed: Tropical, Alchemy, Funnel, Ashlands (TIME);
    GoldenCanyon, ArcticOcean, FarWorlds/Arctic (DFTW).
  - Foreign SetPosition + GetX/GetY/Stuck loop (`Basement72.c4d` BAS7
    `MoveOutClonk`: `while(Stuck(pObj) && Inside(...))
    SetPosition(...,pObj)`) — FnSetPosition force-positions ANY pObj
    (C4Script.cpp:462-477); the Rust host fn silently no-opped on
    foreign targets and reads came from the stale snapshot. Foreign
    writes now land in the target's scope; position/vertices read
    through it (FnStuck/FnGetX/FnGetY, C4Script.cpp:1197,1292,1858).
    Fixed: SkyIslands, Tutorial04/07/10, FoggyCliffs, Mountains.
  - Contents + Exit eject loop (TotemHunt `_PLO.DoPlrLaunch`:
    `while(Contents()) Exit(Contents())`) — C4Object::Exit removes from
    the container's Contents IMMEDIATELY (C4Object.cpp:1529-1533); the
    contents list now re-checks each child's live container/status.
    Fixed: TotemHunt.
  Remaining same-call staging divergences (non-hang, documented):
  same-call Enter does not APPEND to the container's contents list
  (removals are visible, additions are not); the nested-seam Exit drops
  FnExit's caller-relative position/dir args (C4Script.cpp:372-388);
  velocity/owner/OCF are not yet overlaid in `get_world_object`.

## Changelog

Full history in `git log --oneline -- rust/` — every behavioral commit
documents the C++ oracle (file:line) it ports. Major epics in order:
foundations (C4Fixed/LCG/sectors) → CrossCheck/physicals → weather/disasters
→ PXS/mass-mover → host→VM reentrancy + Find_Func → scenario-load epic
(93/93) → player-join pipeline → startup-menu pixel parity → per-pixel
landscape → Tick10 mobile gate → frame-1 live-parity epic (393→0, complete
2026-07-03) → persistent-slot mass mover (incremental CheckInstability
creation at every C++ call site, no surface re-seek, C++ Count/CreatePtr
ledger, 2026-07-03) → scenario-worlds epic (C4MapCreator +
C4MapCreatorS2 dynamic maps, C4Game init placements, scenario-audit
harness, 92/93 content fidelity, 2026-07-03) → same-call staging
read-through (host fns see mid-call RemoveObject/SetPosition/Exit like
the live C4Object; 14 scenario hangs resolved, 2026-07-06).
harness, 92/93 content fidelity, 2026-07-03) → C4MainMenu player menu
(faithful entries + classic C4Menu rendering replacing the modern
Escape panel, 2026-07-06).
harness, 92/93 content fidelity, 2026-07-03) → Fantasy-dragon host-fn epic
(2026-07-06: command fns take the exact C++ argument orders — SetCommand has
NO interval slot, AppendCommand leads with the object, Add/Append default
mode 0=SilentSub, C4CMD_Wait duration from Data else Tx; ~60-wrapper sweep so
missing args nil-fill like C4AulExec instead of hard-erroring; GetR returns
the raw signed r (C4Movement.cpp:434-435); shape attach bookkeeping
(AttachMat/iAttachX/iAttachY/iAttachVtx) + FnAdjustWalkRotation
(C4Object.cpp:6019-6086); FnAngle; FnGetCommand elements 1-5;
FnGetScenarioVal Landscape border keys; FnGetLeague nil stub. SkiesOfFire
600-frame headless: ZERO DRGN warnings).
→ viewport presentation parity (2026-07-07: DrawCursorInfo command rows +
C4Object::DrawCommands/DrawCommand decision tree with textual Image=
descriptor extraction, C4Game::DrawCursors red rank|name cursor label,
non-C++ floating energy/magic bars removed from the world draw; see the
in-game HUD gap row for residuals).
→ explosion epic (2026-07-09: BlastObject/C4Object::Blast on the host seam
with the staged incinerate mirror + ObjectUpdate::fire channel, Find_Layer
carries its layer object, DefCore BlastIncinerate/ContainBlast/HorizontalFix
ingest + GetDefCoreVal blast entries; System.c4g explosions damage, fling,
and ignite again — pinned end-to-end through the real planet scripts; the
former FLNT Hit harness error resolved; see findobject-ocf for the engine
fallback residual).
