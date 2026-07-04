# LegacyClonk Rust Port — Status & GAP LIST

> Living document. Last updated 2026-07-03. The C++ engine in `../src/` is the
> **golden oracle**; parity = bit-for-bit match on simulation state. This file
> tracks the CURRENT divergences only — full wave-by-wave history lives in
> `git log` (every behavioral commit cites the C++ file:line it ports).

## Current status

- **Live parity through frame 19 (2026-07-03, pinned seed).** Frames
  1-16 fell in the first arc (see git log); frames 17-19 fell with:
  the exact script PathFree (ForLine Bresenham + GBackSolid), the
  C4SolidMask BAKE-IN (masks live as MCVehic pixels in the plane —
  plane diff 0.34%→0.05%), SetDir gates + TurnAction + foreign
  SetDir, the synchronous FnJump (ObjectComJump), the per-ExecAction
  t_attach latch (phase wraps cannot retro-attach), component-only
  SetXDir/SetYDir fixed writes, and the comparator now checking
  DIRECTION. Current wall: **frame 21** — the
  coach-rider contact-friction class (the horse's DFA_PULL pushes the
  coach identically both sides; C++'s next coach move applies contact
  friction 1.667→0.30 and the riders read the damped value). Ledgers
  are DRAW-EXACT through frame 20 (frame-marked traces; the waterfall
  WTFL InsertMaterial routing was the f18-20 fork). Menu subsystem,
  persistent mass movers, PXS blast/border fidelity, rotated masks,
  the C++-order tick phases, live in-flight locals, crew join with
  fair-crew physicals and the C4-style HUD are all merged.
  Debug: pin runs with `LC_PIN_SEED=424242` (C4GameParameters.cpp,
  env-gated) so C++ runs are reproducible; get the map seed from
  `LC_DEBUG_MAP=1` (rust prints `RUST MAPSEED n`); replay headless with
  `LC_RUST_ENGINE_MAP_SEED=n`. Landscape dumps: `LC_DUMP_LANDSCAPE`
  (bridge), `LC_RUST_ENGINE_DUMP_LANDSCAPE` (runtime),
  `LC_XTASK_DUMP_LANDSCAPE` (xtask). RNG ledger traces:
  `LC_RNG_TRACE=<file>` (C++ temp probe in C4Random.h, re-add when
  needed) + `LC_RUST_RNG_TRACE=<file>` (committed, ledger-gated).
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

- **Exec order** = the C++ main list REVERSED (`Objects.BeginLast`):
  ascending sort-category (StaticBack→Structure→Vehicle→Living→Object), file
  order within a loaded block, later creations after (engine `exec_seq`);
  Line defs exec first. Containers must exec before their contained crew
  (CopyMotion reads post-move state).
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
- Nested-call locals: VM sessions own their locals — nested calls onto an
  in-flight scope read the pre-call snapshot; outer-call errors drop partial
  outcomes (C++ keeps pre-error mutations).
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
| landscape | incremental ExecuteScan/DoScan (batch temperature conversion desyncs scan order) — including DoScan's per-converted-pixel CheckInstabilityRange (C4Landscape.cpp:225): the column conversion has no pixel coordinates to probe; PRETTY_TEMP_CONV; map creation beyond ChunkOZoom; pixel-exact DigFree/BlastFree accounting; blast/shake instability probes run as a post-pass in the C++ scan order (no per-pixel clear/probe interleave until blast/shake are per-pixel); segment- vs pixel-liquid model |
| movement-physics | SetSolidMask update lifetime; attached-object pushback (its DensityProvider reads the put BUFFER for rotated masks, C4SolidMask.cpp:218-227 — the rotated bake buffer already models this); rotated masks stay off in the non-grid mask-rect overlay (fixture worlds only, no C++ counterpart) |
| landscape | incremental ExecuteScan/DoScan (batch temperature conversion desyncs scan order) — including DoScan's per-converted-pixel CheckInstabilityRange (C4Landscape.cpp:225): the column conversion has no pixel coordinates to probe; PRETTY_TEMP_CONV; pixel-exact DigFree/BlastFree accounting; blast/shake instability probes run as a post-pass in the C++ scan order (no per-pixel clear/probe interleave until blast/shake are per-pixel); segment- vs pixel-liquid model. Map creators (C4MapCreator + C4MapCreatorS2) are ported; OPEN there: standalone runs use MapSeed=0 and RandomSeed=0 for the FixRandom bracket instead of the drawn `Random(3133700)`/network seed (the bridge hands the real values via `LC_RUST_ENGINE_MAP_SEED`/`LC_RUST_ENGINE_RANDOM_SEED`, players via `LC_RUST_ENGINE_STARTUP_PLAYERS`); `MapZoom` uses the clamped Std, not the bracket-internal Evaluate draw (no shipped content has a random MapZoom); Landscape.txt `evalFn=`/`drawFn=`/`algo=script` callbacks unsupported (no shipped content uses them, parse errors degrade to the basic creator like C++'s ignored ReadFile return, C4Landscape.cpp:540); C4Landscape::PostInitMap/KeepMapCreator (DrawDefMap) unported |
| effects | Annul/AnnulCalls + FxAdd add-to-other-effect; TempRemove/TempReadd; Fx*Damage DoEnergy modification; builtin fire/helper effects (Splash/Smoke/Explosion/BubbleOut) |
| commands | Tick2/5/35 throttling; MoveTo flight/swim control; Scale/Hangle let-go thresholds |
| player-controls | 2026-07-03 partial: Control<Com> results coerce via C4Value truthiness (static_cast<bool>, C4Object.cpp:3300); EffectCall + by-number effect handles unblock the CLNK Control2Effect chain; contained clonks route non-Special coms to the container's Contained<Com> (C4Object.cpp:3363-3367,3208-3230). STILL OPEN vs C4Object::DirectCom: directional coms never dispatch ControlLeft/Right/Up/Down script overrides (CallControl runs for EVERY com in C++, :3377-3379 — riding/vehicle overloads dead); classic per-procedure fallbacks unported (ObjectComMovement/Up/Dig/DigDouble + PlayerObjectCommand(C4CMD_Throw), :3395-3547); ContainedControl hardcoded fallbacks (COM_Down exit, COM_Throw command, base buy/sell, Take/Take2, :3243-3306); JnR AutoStopDirectCom + ControlUpdate/ContainedControlUpdate callbacks (:3302-3312,3548+); menu com routing/NoCollectDelay decrement/ClearCommands on plain press (:3340-3376) |
| material | column-model fixture worlds keep segment removal where C++ ClearPix/ExtractMaterial act per pixel (grid worlds are C++-faithful) |
| objects-core | OCF computes a subset of the ~30 C++ checks; C4ObjectInfo permanent training/experience unmodeled; DFA_CONNECT uses direct endpoint assignment (the LineConnect wrapping walker is unported) |
| findobject-ocf | Layer never matches — C4FindObjectLayer::Check is `pObj->pLayer == pLayer` (C4FindObject.cpp:671-674), so `Find_Layer(nil)` must match every unlayered object; today System.c4g BlastObjects (Explode.c:93-97) finds nothing and explosions neither damage nor fling objects. Fixing it requires the BlastObject host fn (FnBlastObject, C4Script.cpp:2281-2289 -> C4Object::Blast, C4Object.cpp:1389-1399) + DefCore BlastIncinerate ingest first, or the zero-warning gate regresses. Also: sector-bounds FindMany traversal order; cached sort keys i64 vs C++ i32 wrap |
| game-control-record | control-packet payload serialization; lc-network DoInput/host sync-check broadcast wiring |
| players-crew-teams | team home-base production sync (C4RULE_TeamHombase); CheckElimination; asset value stub; crew infos not persisted in snapshots |
| definitions-id | runtime dispatches on procedure strings not numeric ActMap indices; GetComponents override; CalcDefValue; some DefCore flags unparsed |
| script-values | C++ string-table interning/refcounts; save/load + net sync of values |
| weather-sky | SetSeasonGamma; season Min/Max wrap detail; sky parallax wind/100 vs FIXED100 |
| config-info | GetAName file-based names partial; PromotionUpdate; locale/control-pref defaults |
| resources-groups | no group write/create/gzip-out/CRC32-at-open; directory iteration order may differ from C++ |
| network | password auth, voting, league, NCS_* client status, join-data save/restore, protocol negotiation |
| host-fn backlog | 2026-07-03 mostly CLEARED: ShiftContents full semantics (foreign pObj via the nested seam; fDoCalls ~ControlContents veto + ~Selection + Grab sound, C4Object.cpp:5754-5775 — CanConcatPictureWith still approximated by definition-id equality, menu Refill is presentation); GrabObjectInfo transfers crew flag + portrait + info permanent physicals (C4Object.cpp:5715; name/rank/experience payload unmodeled — no per-object name); SetAction nil targets PRESERVE like `if (pTarget) Action.Target = pTarget` (C4Object.cpp:4123-4125 — was the "HORS action-start target-zero" class); DoDamage/DoEnergy reach foreign targets with caused-by kill tracing (ObjectUpdate.energy_loss_cause; UpdatLastEnergyLossCause guard C4Object.cpp:1369-1378 on host + engine paths; Punch carries the attacker's controller + unguarded post-fling write C4ObjectCom.cpp:749,755,762; DoDamage fires the ~Damage callback C4Object.cpp:1290; nested-fold AssignDeath when a foreign write zeroes energy). STILL OPEN: BlastObject unported (see findobject-ocf); Fx*Damage effect hooks don't dispatch on the HOST scope path (engine change_object_energy/damage has them — see effects row); GetKiller unported; nil-target AddEffect never ran Fx*Start/Timer for System.c4g ShakeEffect in the Goldrush headless probe (Distance/SetViewOffset were unreachable until 2026-07-03) |
| object menus | sim-observable core modeled on `ObjectState.menu` (CreateMenu/GetMenu/CloseMenu/AddMenuItem/SelectMenuItem incl. MenuQueryCancel + OnMenuSelection callbacks, C4Script.cpp:1418-1741, C4ObjectMenu.cpp:56-104); OPEN: engine-internal menus (lc-app Activate/Get/Contents UI) are invisible to GetMenu (C++ returns C4MN_Activate etc.); GetMenuSelection/SetMenuSize/SetMenuDecoration/SetMenuTextProgress unported; MenuCommand execution on user Enter not wired; menus close on Enter/Exit/SyncClearance/SetCommand-fControl in C++ (C4Object.cpp:1531,1571,3818,3922) but not here; menu state is runtime-only (not in ObjectSnapshot save/load — C++ Objects.txt drops menus too) |

## Presentation-layer gaps (non-sync)

| Subsystem | State |
|---|---|
| graphics | ~25%: per-pixel blit only; no transforms/GL/shaders/landscape rendering |
| audio | ~35%: panning math differs; C4Sound/C4MusicSystem high-level layers absent |
| gui-menus | no DrawElement rendering, layout, text progression, portraits |
| startup-launcher | player-selection dialog stub; no update check/first-start UX |
| particles | sim side done (SafeRandom by design, non-sync); Particle.txt gfx loading + draw procs open |
| in-game HUD | C++-faithful chrome done (upper board/logo/title/Game.Time, DrawPlayerInfo fixed items, cursor portrait+rank+name, energy bar, startup keyboard+name, one-line message board; oracles C4UpperBoard.cpp:46-96, C4Viewport.cpp:884-965/1281-1476, C4ObjectInfo.cpp:302-371, C4MessageBoard.cpp:243-306). OPEN: contents/inventory row (Contents.DrawIDList), DrawPlayerControls, rank-name line, board scroll/fade anim, player-colored board lines; lc-app registers players without crew (join_player unused) so GoldRush shows the faithful empty no-crew corner |

## Known harness issues

- **MIDI music cannot play** (`Cannot play music file …: No SoundFonts have
  been requested`, ~1500×/run): the C++ SDL2_mixer fluidsynth backend has no
  GM soundfont configured. Not a port defect. Fix: install a .sf2 and export
  `SDL_SOUNDFONTS`, or set `Music=false` in
  `~/Library/Preferences/legacyclonk.config`.
- lc-app sim-tick script errors still exit the app (event loop) — needs the
  same fail-safe treatment the engine has.
- FLNT `Hit` script errors headless (`script error in Hit of FLNT`) —
  surfaced once flints actually land after the AI/schedule fixes; needs
  the inner error dug out (likely a missing host fn).
- The headless xtask world skips the player join: GoldRush intro-driven
  state (cavalry recruitment draws, coach splash) reproduces only with
  the live pinned harness.
- **14 scenarios hang in a join/tick script loop** now that their worlds
  are real (previously-flat landscapes never exercised these paths):
  Fantasy/Alchemy, FarWorlds/Arctic, Missions/Funnel, Tutorial04/07/10,
  Western/GoldenCanyon + TotemHunt, Worlds/ArcticOcean + Ashlands +
  FoggyCliffs + Mountains + SkyIslands + Tropical (45s join+3-tick
  smoke, 2026-07-03). Two sampled shapes: SkyIslands hangs INSIDE
  join_player in a spawned object's Initialize (nested script frames
  hot-looping GetX/GetY host calls); Tropical hangs in an action
  StartCall loop of FindObject + RemoveObject (suspect: removal not
  visible to the same-call FindObject cursor, C++ Status=0 objects drop
  out immediately). Load+apply is NOT affected (sweep and audit stay
  green); this is a script-VM/world-removal semantics follow-up epic.

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
harness, 92/93 content fidelity, 2026-07-03).
