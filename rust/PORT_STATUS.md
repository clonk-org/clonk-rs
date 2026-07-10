# Rust Port Status

> 2026-07-09. C++ (`../src/`) is the oracle; parity means bit-exact simulation
> state. History belongs in `git log -- rust/ parity/`.

## Current state

- Pinned Goldrush (`LC_PIN_SEED=424242`) matches through frame 409; frame 410
  first differs at object 582's integer/subpixel position.
- Foundational motion and RNG gaps are closed: object motion uses signed 16.16
  `C4Fixed`; `Random()` uses the shared C++ LCG/draw ledger.
- Scenarios load/apply 93/93; audit is 92/93. CTF_DeepSea's animal result is
  C++-consistent (water fills after `InitAnimals`). Goldrush runs 1000 headless
  ticks without script warnings.
- Full simulation parity and graphical parity remain incomplete.

## Gates

Every finished slice:

```sh
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Runtime/content validation when relevant:

```sh
cargo xtask scenario-sweep                         # 93/93
cargo xtask scenario-errors Goldrush --ticks 1000  # clean
cargo xtask scenario-audit                         # 92/93; exception above
```

Behavioral sync slices also require a rebuilt/relinked pinned live shadow run
and a recorded new first mismatch.

## Open simulation gaps

- **Movement/masks:** `SetSolidMask` lifetime; rotated-mask attached-object
  pushback and non-grid overlays; remove dormant non-C++ material damping,
  engine-wide 12px xdir clamp, and post-movement collision resolution.
- **Landscape/material:** incremental scan/interleaving, `PRETTY_TEMP_CONV`,
  pixel-exact DigFree/BlastFree and blast/shake/liquid ordering; standalone seed,
  startup-player and MapZoom bracketing; Landscape.txt algorithms,
  `PostInitMap`/`KeepMapCreator`; fixture columns remove segments, not pixels.
- **Effects/fire:** checked-fire timing/returns; engine-incineration check-chain
  interception; negative-priority persistence and exact stop semantics; GLOBAL
  checking/shadowing; attached detach, Tick5 base extinguish, cause mapping,
  smoke, and engine Explosion helpers.
- **Commands:** real C4PathFinder/Tick35 recheck; DFA_FLOAT; contained-target
  Exit; exact UpdateInterval/arrival; FlightControl gates; Acquire default
  timing; Put's live `Ty` rewrite.
- **Controls:** immediate contained throw; vehicle overloads on non-control
  `SetCommand`; linekit DigDouble/DFA_CONNECT; auto-sell/snow-dig; legacy version
  gates/ranks; app trade-menu UI.
- **Objects/find/OCF:** `InMat`/`ClosedContainer`, preview OCF and `At` addtop;
  permanent info; DFA_CONNECT; `Sort_Value` via object value; no-sort FindObject2
  early exit; immediate sector registration for mid-call creations.
- **Players/game/network:** home-base production, asset value, crew persistence;
  elimination presentation/script-player flag; control sync and protocol/auth/
  league completeness.
- **Definitions/values:** `Oversize`, numeric ActMap, GetComponents/CalcDefValue,
  remaining DefCore fields; C4Value interning/serialization and strict host
  conversion.
- **Script host:** partial-error recovery for effects and remaining definition/menu
  seams; foreign fixed-dir precision; FnExit cancel-attach/bounds and
  Ejection/Departure.
- **Weather/config/resources:** remaining sky presentation/LaunchCloud; control
  preferences; names/promotions/locales; group write/gzip/CRC/directory order.
- **Hosts/menus:** exact `CanConcatPictureWith` and object-info payloads; route
  visible engine-created menus (Dragon Rock blocks controls on its mandatory
  hidden choices), AutoContextMenu/CloseCommand, and layout.

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

- Renderer/audio: transforms, GL/shaders, landscape, panning, playlist
  advancement, and exact/looped/streamed MIDI. SMF 0/1 and RMID playback exists;
  scenario music excludes WAV/definition effects.
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
cmake --build build-arm64-native --target clonk -j 8
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

Harness limits: MIDI needs FluidSynth 2.x and a trusted SF2/SF3
(`LC_FLUIDSYNTH_LIBRARY`/`SDL_SOUNDFONTS`; upgrade to 2.5.6+); eager decode caps
at 8 MiB/15 minutes/1M events. `lc-app` exits instead of fail-safe on tick script
errors; xtasks skip player join; Goldrush Talker consumes controls for ~1000
frames; `lc-network`'s `control_sync_and_reconnect_smoke` has a rerunnable TCP
timing flake.
