# Rust Port Status

> 2026-07-09. C++ (`../src/`) is the oracle; parity means bit-exact simulation
> state. History belongs in `git log -- rust/ parity/`.

## Current state

- Pinned Goldrush (`LC_PIN_SEED=424242`) matches through frame **308**.
- First mismatch: frame **309**, FISH #1343 command direction — Rust `Right`,
  C++ `Down`.
- Latest closed wall: frame 219 directly assigned DFA_SWIM facing, bypassing
  `SetDir`, its `TurnAction`, and fixed-position resync. WALK/HANGLE/DIG/SWIM
  now share the C++ path; the production `C4ActionDirection.h` differential
  freezes FISH #1343's exact velocity, action, phase, and fixed coordinates.
- Foundational fixed-point and RNG gaps are closed: object motion is signed
  16.16 `C4Fixed`; `Random()` uses the C++ LCG and global draw ledger.
- Scenario coverage: 93/93 load and apply; content audit 92/93. The remaining
  CTF_DeepSea animal flag is C++-consistent because water is filled after
  `InitAnimals`. Goldrush headless reaches 1000 ticks without script warnings.
- 2026-07-09 content wave merged: the explosion chain (BlastObject/Blast +
  host-path incinerate, Find_Layer, GetDefCoreVal blast entries — the FLNT
  `Hit` error class is gone), the C4Effect protocol (check chain, FxAdd
  merge, TempRemove/Readd brackets, Stop_Deny), Fx*Damage on the host scope
  path, GLOBAL nil-target effect Fx*Start/Timer dispatch (ShakeEffect runs
  end-to-end), MoveTo procedure arms + Data flags + Acquire defaults + live
  GetCommand views, season wrap/sky-scroll bit-exactness, object-menu
  lifecycle + reflection + user-Enter commands, AddEffect missing-priority,
  and Tick35 one-way crew elimination.
- Full parity is not complete. Determinism-critical gaps are listed below;
  graphical rendering remains partial.

## Gates

Every finished slice:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Runtime/content validation when relevant:

```sh
cargo xtask scenario-sweep                  # 93/93
cargo xtask scenario-errors Goldrush        # clean at 120+ ticks
cargo xtask scenario-audit                  # 92/93; exception above
```

Behavioral sync slices also require a rebuilt/relinked pinned live shadow run
and a recorded new first mismatch.

## Open simulation gaps

- **Movement / masks:** `SetSolidMask` lifetime; attached-object pushback
  against rotated mask buffers; rotated non-grid overlays; remove three dormant
  non-C++ paths (`apply_material_interaction` damping/friction overwrite,
  engine-wide 12px xdir clamp, post-movement `resolve_collision`).
- **Landscape / material:** incremental `ExecuteScan`/`DoScan` order and
  per-pixel instability interleaving; `PRETTY_TEMP_CONV`; pixel-exact
  DigFree/BlastFree, blast/shake ordering and liquid segments; standalone
  MapSeed/RandomSeed/startup-player bracketing and random MapZoom (unused by
  shipped content), Landscape.txt script algorithms,
  `PostInitMap`/`KeepMapCreator`; fixture column worlds still remove material
  by segment rather than pixel.
- **Effects:** builtin fire CLOSED 2026-07-09 — fire is a real "Fire"
  C4Effect entry (priority 100/interval 1, vars [FireMode, CausedBy, Blasted,
  IncineratingObj]) on BOTH incinerate paths; the burn executes through the
  effect timer at its list position; Extinguish/RemoveEffect("Fire") kill via
  the effect (engine FnFxFireStop clears OnFire); Incinerate/Extinguish/
  Bubble and FxFireStart/Timer/Stop/Info host fns registered (script
  overloads chain via inherited); AddEffect("Fire") runs the engine start
  synchronously (checked adds defer to the Fx*Effect chain and ignite at
  first execution — timing divergence; interval-0 checked fire never
  ignites); Splash/BubbleOut ported engine-side (synced Random order + FXU1
  cap 150). Fire residuals: other effects' check chain does not intercept
  ENGINE-initiated incineration; attached-object detach (DFA_ATTACH scan),
  Tick5 base extinguish, ValidPlr mapping of the burn's cause, SmokeRate
  smoke/fire particles/sounds (presentation-only); host-seam FxFireTimer
  (inherited chain) omits those same pieces; Smoke() always takes the
  particle path (the no-particle FXS1 fallback unmodeled); Explosion()'s
  engine helper stays unported (System.c4g's global Explode shadows
  FnExplode for all shipped content). Protocol residuals: AddEffect's
  return value cannot reflect deferred check outcomes
  (deny 0 / acceptor number / -2); inactive negative-priority effects are not
  persisted between dispatch sequences (mid-bracket queries, Kill's
  TempAddForRemoval arm); Stop_Deny recovery reinserts at sorted position and
  skips death-clear reasons; stop reasons are strings, not C4FxCall_* ints;
  GLOBAL adds skip the priority check chain, and their no-command-target
  dispatch goes through the first-registered definition's script host where a
  definition-local Fx* name could shadow a same-name global; AddEffect
  arg 6 keeps an explicit-nil fixture timer slot (any value there is rVal1
  like C++).
- **Commands:** the Tick35 `PathChecked` recheck is blocked on a real
  C4PathFinder port (waypoint pushes, fWaypoint easings; `pathfinder.rs` is a
  GetPath-only heightmap approximation); DFA_FLOAT steering arm; contained
  targets do not Exit first; the C++ UpdateInterval countdown/arrival model vs
  the Rust throttle (invented tolerance 5 + dwell); FlightControl's Disabled
  gate and Def->Pathfinder alternative; Acquire defaults apply at Set rather
  than first Execute; Put's live Ty reminder rewrite.
- **Controls:** main gaps CLOSED 2026-07-09 — NoCollectDelay end-to-end
  (ObjectComDrop arms 2 + SetCommand entry decrement travels as a
  CommandOperation, C4ObjectCom.cpp:668-671 / C4Object.cpp:3941-3942);
  wheel-com ShiftContents + COM_Contents target shift with the
  ControlContents/Selection callbacks (C4Object.cpp:3364-3396, 5751-5797);
  C4Object::Base modeled via ExecBase flag assignment / lost-flag clear
  (C4Object.cpp:1000-1031) and the ContainedControl COM_Up/COM_Dig arms
  emit MenuRequestKind::Buy/Sell after the ValidPlr/hostility/BASEFUNC
  checks (C4Object.cpp:3269-3280); DefCore VehicleControl parses and both
  SetCommand ControlCommand overloads run on the control Set path
  (C4Object.cpp:3944-3969); the exact C4Player cursor selection model
  (CursorLeft/Right/Toggle, SelectAllCrew, UpdateSelectionToggleStatus,
  AdjustCursorCommand w/ rank-less hirank, CrewSelection callbacks,
  C4Player.cpp:1235-1365 + C4Object.cpp:5815-5832) replaced the frontend
  approximation. Residuals: contained COM_Throw still executes on the next
  command tick — the immediate ExecuteCommand (C4Object.cpp:3267) needs the
  tick's command-step block extracted into a one-shot; FnSetCommand skips
  the vehicle overloads (C++ runs them for EVERY SetCommand, fControl or
  not); linekit DigDouble line construction (C4ObjectCom.cpp:379-529) is
  blocked on the DFA_CONNECT line model; ExecBase leaves
  BASEFUNC_AutoSellContents and the Tick35 structure snow-dig unported;
  the C4MN_Buy/C4MN_Sell refill menu UI is unbuilt app-side (requests are
  emitted, arms are no-ops); DefCore version gates (fCallSfEarly,
  grab-control 4,9,5,0) still treat every def as modern; crew Info->Rank is
  unmodeled so hirank = first eligible roster entry.
- **Objects / find / OCF:** the SetOCF computation gap is CLOSED 2026-07-09 —
  all 30 C4Object::SetOCF bits compute per C4Object.cpp:526-666 (with new
  DefCore Entrance/RotatedEntrance/Exclusive/Prey/Edible/Chop/
  AttractLightning/NoFight ingest, ActMap ObjectDisabled, GetOCFForPos area
  checks in at_object, and cached-OCF reads for script GetOCF/snapshots);
  residuals: SetOCF's InMat update (InMat/ClosedContainer unmodeled),
  mid-call creation previews stay preview-grade, C4Object::At lacks addtop
  (NoCollectDelay arming/decrement landed 2026-07-09 with the Controls
  work). Sort keys now use the C++ int32 semantics
  (2026-07-09: Distance wraps in i32, Speed is the 0/1 `operator bool` quirk
  of the C4Fixed sum with truncating fixed squares, Mass reads the live
  UpdateMass value; Sort_Value still reads the definition value where C++
  calls C4Object::GetValue — see the definitions bullet's CalcDefValue gap).
  Still open: permanent object info training; DFA_CONNECT line walking;
  sector-bounds FindMany traversal order.
  The engine explosion fallback trio FnExplode/Explosion/Game::BlastObjects +
  FnShakeObjects stays unported (System.c4g overrides make it unreachable for
  shipped content); the host-path incinerate does not add the "Fire" C4Effect
  entry (same builtin-fire gap as the engine path).
- **Players / game / network:** team home-base production, asset value,
  crew-info persistence; elimination is Tick35-gated and one-way now, with
  RetireDelay/sound presentation and the script-player CSPF_NoEliminationCheck
  flag unmodeled; control-packet serialization and sync broadcast;
  auth/voting/league/join-data/protocol completeness.
- **Definitions / script values:** numeric ActMap dispatch, GetComponents and
  CalcDefValue overrides, remaining DefCore flags and runtime creation-number
  skew; C4Value string interning and save/network serialization; strict
  host-parameter conversion.
- **Script host model:** every outer call kind (PSF/timer/host calls,
  Control, Initialize/Construction/Step, menu callbacks, MenuCommand
  DirectExec) now runs on LIVE session local cells like effect callbacks —
  nested calls onto the in-flight object share the storage in both
  directions; outer-call errors keep their pre-error mutations (locals,
  foreign writes, RNG/audio advance) via the `EngineError::Script`
  recovery payload, applied by the call_object_function / DirectExec /
  movement-Hit / control funnels; same-call Enter additions sort into the
  container's contents (Add stContents cluster rules); FnExit threads its
  caller-relative position/dir args incl. the tr==-1 Random(360) draw;
  get_world_object overlays staged owner and whole-pixel dirs and
  re-derives the STAGED OCF bits (container/con/alive/category).
  Residuals: effect-callback ERRORS still restore the rng/audio backups
  and drop the partial outcome (the fail-safe arm predates the recovery
  seam); the Initialize/Construction/Step and menu-entries/-command/
  -callback definition seams attach no recovery (their batch folds drop
  on error), EXCEPT Initialize/Construction which now fold their partial
  outcome on error like call_raw (pre-error creations, burned ids,
  RNG/audio advance and local writes all persist; see the
  creation-number bullet); foreign dir reads stay whole-pixel (snapshot
  task B); energy
  is deliberately not overlaid (active-scope/DoEnergy paths read live
  scope state); FnExit's ObjectComCancelAttach/BoundsCheck arms and the
  Ejection/Departure engine calls stay unmodeled.
- **Weather / config / resources:** weather/sky closed except presentation
  residuals (stale-between-triggers gamma vs the live getter, SetSkyFade,
  disabled-BackClr retention, SkyDef tile fallback; LaunchCloud objects stay
  ledger-replayed stubs); player control preferences/forced style;
  name/promotion data and locale defaults; group create/write/gzip/CRC and
  directory-order fidelity.
- **Host functions / menus:** exact `CanConcatPictureWith`; complete
  GrabObjectInfo identity/rank payloads. Object menus are closed at the engine
  level (close hooks, GetMenuSelection/SetMenuSize/SetMenuTextProgress/
  SetMenuDecoration, user-Enter MenuCommand via the DirectExec port) — the
  remaining gaps are lc-app COM_MenuEnter/Close/Select routing to those
  entries, engine-internal menus staying invisible to `GetMenu`,
  AutoContextMenu/CloseCommand, and presentation layout.

- **Creation-number skew (the numbering epic):** the AH_Predator entry is
  CLOSED — the "preview-vs-final id mismatch" was disproven empirically
  (every Initialize preview id 4..92 materialized verbatim); the door
  zones (owners 53/69/84/86) dropped because `apply_scenario_batch`
  folded the transfer-zone commands BEFORE the same batch's spawns, while
  C++ has the owner live in Game.Objects before any creation callback can
  call SetTransferZone (C4Game.cpp:1115-1131, C4Script.cpp:3145-3149).
  Zones now fold after the spawn loop and land. Also CLOSED: a FAILED
  Construction/Initialize creation callback used to discard its whole
  partial outcome (CommandBatch::default() with the PRE-CALL
  `next_object_id`), erasing pre-error creations and re-minting their
  burned preview ids (AH_Predator: HZCK's Initialize allocated 94 for
  its ABAG, failed on unknown `GetHUD`, and CHOS's InitializePlayer
  re-minted 94 for TIM1); both seams now fold the partial outcome and
  surface the error as a value, so pre-error creations, burned numbers,
  RNG/audio advance and local writes persist like C++ (errors roll
  nothing back, C4AulExec.cpp:1318-1342; `Number =
  ++ObjectEnumerationIndex`, C4Game.cpp:1119) — the join sequence is now
  93 HZCK, 94 ABAG, 95 TIM1. What remains of the epic: the GoldRush
  crosshair skew (cpp 1534-1537 vs rust 1531-1534) stays open and is
  NEITHER mechanism above: GoldRush startup runs warn-free (nothing was
  discarded) and triggers no rain-cloud ledger replay (LaunchCloud is
  not its gap) — three C++ startup creations are missing outright and
  still need to be identified.

## Accepted/comparator-only divergences

- Parser permits comma expressions outside C++'s legal return context.
- Material `mrfScript` resolves against scenario rather than global script.
- Particles use presentation RNG and are outside `C4ControlSyncCheck`; comparison
  is opt-in via `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
- Message-list/render-surface fields compare only when both bridges provide them;
  HUD crew sorting occurs only at the bridge boundary.
- Same-seed landscape bytes are 99.66% equal; residual differences are C++
  solid-mask MCVehic bake-in versus Rust mask overlays.

## Open presentation gaps

- Renderer/audio: transforms, GL/shaders, landscape, panning, MIDI decoding, and
  playlist advancement. Scenario music already excludes WAV/definition effects.
- GUI/particles: generic layout/text/portraits and Particle.txt graphics/procedures.
- Launcher: player/update/first-start/search/CanOpen, icons/files/mission access,
  FolderMap, and long-list scrolling.
- Player menu: pause/abort, goal/rule/player/key/team/admin/network flows,
  MaxPlayers, scrolling, markup, and exact tooltips.
- HUD: inventory/control/mouse rows, board animation/colors, descriptors,
  FlashCom, Base/NeedEnergy, custom ranks, and NO-DIG/NO-CHOP feedback.

## Load-bearing invariants

- Keep integer `x/y/r` separate from `fix_x/fix_y/fix_r`; snapshots expose the
  former and parity probes the latter. Keep raw `r/fix_r/rdir` independent.
- RNG is the shared C++ LCG with global `RandomHold`/`RandomCount`.
- Preserve reverse object-list execution, monotonic unreused IDs, category order
  StaticBack→Structure→Vehicle→Living→Object, loaded order, runtime same-def
  clustering, Line/StaticBack cases, Enter/Exit non-reordering, and
  container-before-content execution.
- Loaded objects restore without callbacks. Fresh creation runs Construction at
  Con 0, then initial DoCon; crossing FullCon runs Completion→Initialize. Only
  pre-insertion action callbacks defer one Start/Abort pair. Otherwise SetAction
  callbacks are synchronous Start→End→Abort; never requeue an update marked
  `callbacks_dispatched`. Phase wrap uses the old action as source.
- SetAction, CopyMotion, ForcePosition, and Exit resync fixed coordinates;
  DoCon/TargetBounds may update only integers. Ordinary integer/fixed splits are
  valid.
- End-of-ExecAction phase rates: WALK/HANGLE `|xdir|*10`, SCALE `|ydir|*14`,
  SWIM `ValByPhysical(160,Swim)*10`, DIG `ValByPhysical(125,Dig)*40`. WALK faces
  by raw fixed sign and captured old action. Default Attach zeroes dirs/mobilizes;
  missing Jump preserves the action.
- Crew lists are newest-first; GetHiRank keeps the first equal rank and join uses
  that cursor. HUD sorting is bridge-only.
- Script registration order is definition resources, folder/scenario defs, pack
  System.c4g, scenario Script.c, scenario System.c4g. Numeric-C4ID InitializeDef
  precedes environment/SyncClearance, scenario Initialize, and queued join (save
  restore skips scenario Initialize); appends precede includes.
- Pixel landscape is authoritative. Solid masks sample active graphics and remain
  per-object carriers even when the definition has no mask.
- Initialization RNG order is Gravity→Season→YearSpeed→Climate→Wind
  (`Random(151)`); preserve omitted C4SVal defaults and C4ID/object-list order.
- C4Aul unary `!` binds only its operand except committed `!x = y`. Fixtures
  default StaticBack; movement fixtures need CATEGORY_OBJECT, mobile state, and
  round-to-nearest `fixtoi`.
- Runtime contents insert newest-first within category; loaded contents stay
  verbatim. Effects run after movement and before fire/life. Enter's internal
  Exit mobilizes/copies motion; Collect does not copy motion.

## Harness

Build/relink and run pinned Goldrush:

```sh
cd rust && cargo xtask ffi --release
cd .. && cmake --build build-arm64-native --target clonk -j 8
cd build-arm64-native
LC_RUST_ENGINE_RUNTIME=1 LC_PIN_SEED=424242 \
  clonk.app/Contents/MacOS/clonk \
  ../content/Western.c4f/Goldrush.c4s ../build/Tyler.c4p /nonetwork /console
```

- Golden: `cargo xtask parity record|verify`. Comparator reports Rust as
  expected/C++ as actual and stops after the first mismatch.
- Object/shape probes: `LC_XTASK_OBJ_DUMP=<ids>`, `LC_XTASK_PROBE_SHAPE`,
  `LC_XTASK_PROBE_SOLID`.
- Landscape: `LC_DUMP_LANDSCAPE`, `LC_RUST_ENGINE_DUMP_LANDSCAPE`,
  `LC_XTASK_DUMP_LANDSCAPE`.
- RNG: `LC_RNG_TRACE=<file>`, `LC_RUST_RNG_TRACE=<file>`; align reseed generations.
- Map replay: `LC_DEBUG_MAP=1`, then `LC_RUST_ENGINE_MAP_SEED=<n>`; standalone
  runs may need `LC_RUST_ENGINE_RANDOM_SEED` and `LC_RUST_ENGINE_STARTUP_PLAYERS`.

- Differential golden: `cargo xtask parity record|verify`.
- The live comparator reports Rust as expected/C++ as actual and disables after
  the first mismatch. Object/shape probes: `LC_XTASK_OBJ_DUMP=<ids>`,
  `LC_XTASK_PROBE_SHAPE`, `LC_XTASK_PROBE_SOLID`.
- Landscape dumps: `LC_DUMP_LANDSCAPE`,
  `LC_RUST_ENGINE_DUMP_LANDSCAPE`, `LC_XTASK_DUMP_LANDSCAPE`.
- RNG ledgers: `LC_RNG_TRACE=<file>`, `LC_RUST_RNG_TRACE=<file>`; align
  FixRandom/reseed generations before comparing counts.
- Map reproduction: `LC_DEBUG_MAP=1`, then
  `LC_RUST_ENGINE_MAP_SEED=<n>`; standalone replay may also need
  `LC_RUST_ENGINE_RANDOM_SEED` and `LC_RUST_ENGINE_STARTUP_PLAYERS`.

## Known harness issues

- MIDI playback needs an SDL SoundFont (`SDL_SOUNDFONTS`); its repeated error is
  unrelated to simulation parity.
- `lc-app` still exits on simulation-tick script errors instead of using the
  engine fail-safe path.
- FLNT `Hit` headless error: resolved 2026-07-09 (the missing `BlastObject`
  host fn); the chain is pinned end-to-end through the real planet scripts.
- Headless xtask worlds skip player join; intro-driven Goldrush state requires
  the live pinned harness.
- Goldrush's intro Talker consumes crew controls for roughly its first 1000
  frames; scripted input tests must run after the dialog stops.
- `lc-network`'s `control_sync_and_reconnect_smoke` has a TCP timing flake;
  rerun it before treating a lone failure as a regression.
