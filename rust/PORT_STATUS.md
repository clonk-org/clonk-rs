# LegacyClonk Rust Port — Current Status

> Last updated 2026-07-09. C++ in `../src/` is the golden oracle; parity means
> bit-exact simulation state. This file tracks current state only. Closed-wall
> forensics and implementation history live in `git log -- rust/ parity/`.

## Status

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
- Full parity is not complete. Determinism-critical gaps are listed below;
  graphical rendering remains partial.

## Required gates

Every finished slice must pass:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
```

Runtime/content validation required for parity slices:

```sh
cargo xtask parity verify
cargo xtask scenario-sweep                 # 93/93
cargo xtask scenario-errors Goldrush       # clean over 120+ ticks
cargo xtask scenario-audit                 # 92/93, exception above
```

Behavioral sync slices must also rebuild/relink and rerun the pinned live
shadow, recording the new first mismatch.

## Open determinism-critical gaps

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
- **Effects:** Annul/AnnulCalls and add-to-other-effect; TempRemove/Readd;
  builtin fire/splash/smoke/explosion/bubble helpers.
- **Commands:** Tick2/5/35 throttling; MoveTo flight/swim and Data flags;
  Scale/Hangle release thresholds; Acquire defaults; live `GetCommand` fields
  and restored command-stack element views.
- **Controls:** contents-wheel shifting; `NoCollectDelay`; base buy/sell menus
  and immediate contained throw; DefCore version gates; VehicleControl
  overload; exact crew cycling; linekit DigDouble construction.
- **Objects / find / OCF:** full OCF computation; permanent object info
  training; DFA_CONNECT line walking; sector traversal and i32 sort-key wrap.
  Port `BlastObject` plus `BlastIncinerate` before enabling
  `Find_Layer(nil)` or the zero-warning gate regresses.
- **Players / game / network:** team home-base production, elimination, asset
  value, crew-info persistence; control-packet serialization and sync broadcast;
  auth/voting/league/join-data/protocol completeness.
- **Definitions / script values:** numeric ActMap dispatch, GetComponents and
  CalcDefValue overrides, remaining DefCore flags and runtime creation-number
  skew; C4Value string interning and save/network serialization; strict
  host-parameter conversion.
- **Script host model:** non-effect outer calls still expose stale locals to
  nested calls; outer-call errors drop pre-error mutations; same-call Enter does
  not append contents, nested Exit drops relative args, and velocity/owner/OCF
  are not fully overlaid.
- **Weather / config / resources:** SetSeasonGamma and season wrap details; sky
  parallax arithmetic; player control preferences/forced style; name/promotion
  data and locale defaults; group create/write/gzip/CRC and directory-order
  fidelity.
- **Host functions / menus:** AddEffect missing-priority behavior, Fx*Damage on
  host paths, nil/global AddEffect Start/Timer dispatch, exact
  `CanConcatPictureWith`, complete GrabObjectInfo identity/rank payloads,
  remaining object-menu queries/sizing/commands and C++ menu-close events;
  engine menus remain invisible to `GetMenu`.

## Accepted or comparator-only divergences

- The parser accepts comma expressions outside C++'s legal return context.
- Material `mrfScript` callbacks resolve against scenario script rather than the
  global engine.
- Particle equality is skipped by default because particles use presentation
  RNG and are absent from `C4ControlSyncCheck`; enable with
  `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
- The bridge exports neither C++ message-list nor Rust render-surface symmetry;
  those fields compare only when both sides provide them. HUD crew is sorted at
  the bridge boundary.
- Same-seed landscape bytes are 99.66% equal; the residual is C++ solid-mask
  MCVehic bake-in versus Rust mask overlays.

## Open presentation gaps

- **Renderer/audio:** transforms, GL/shaders, landscape and audio panning remain
  partial. Scenario music now excludes WAV/definition effects and finds direct
  `Music.c4g`; MIDI decoding and playlist advancement are missing.
- **GUI/particles:** generic DrawElement/layout/text/portraits and Particle.txt
  graphics/draw procedures are missing.
- **Launcher:** player selection, update/first-start flows, search input,
  CanOpen state, custom icons, file operations, mission access, FolderMap and
  long-list scrolling.
- **Player menu:** native abort/pause semantics, goal/rule info, player-file
  join and key binding, hostility/admin/team pages, MaxPlayers,
  display/network actions, scrolling, markup and exact tooltips.
- **HUD:** inventory/control/mouse rows, board animation/colors, Method/append
  descriptors, FlashCom, Base/NeedEnergy icons and custom Rank.txt names.
  NO-DIG/NO-CHOP feedback is also missing.

## Load-bearing invariants

- Preserve integer `x/y/r` separately from `fix_x/fix_y/fix_r`; snapshots expose
  integer mirrors while parity probes compare raw fixed state. Keep raw
  `r/fix_r/rdir` independent; only GetR projects the angle.
- RNG is the shared C++ LCG with `RandomHold`/`RandomCount`; never substitute a
  per-system generator.
- Execution uses the persistent reverse C++ object-list order. Object IDs are
  monotonic and never reused. Preserve StaticBack→Structure→Vehicle→Living→
  Object ordering, loaded file order, runtime same-definition clustering,
  Line/StaticBack special cases, Enter/Exit non-reordering and
  container-before-content execution.
- Loaded objects restore action/state without callbacks. Fresh creation runs
  Construction before Initialize; pre-insertion action callbacks retain one
  deferred Start/Abort pair because the object is not world-visible yet.
- `SetAction` callbacks are synchronous and ordered Start→End→Abort. Outside the
  pre-insertion exception above, never queue an update whose
  `callbacks_dispatched` flag is true. Phase wrap retains the pre-transition
  action as its phase source.
- SetAction, CopyMotion, ForcePosition and Exit resync fixed coordinates; ordinary
  integer/fixed splits are legitimate state. DoCon/TargetBounds may update only
  the integer side.
- Phase advance runs at the end of ExecAction: WALK/HANGLE use `|xdir|*10`,
  SCALE `|ydir|*14`, SWIM `ValByPhysical(160,Swim)*10`, DIG
  `ValByPhysical(125,Dig)*40`. WALK facing uses raw fixed-point sign and retains
  the captured old action. Default Attach zeroes dirs/mobilizes instead of
  gravity; a missing Jump action leaves the current action intact.
- Crew lists are newest-first; GetHiRank keeps the first equal rank and join
  selects that cursor. HUD sorting is bridge-only.
- Script host registration order is definition resources, folder/scenario defs,
  pack System.c4g, scenario Script.c, then scenario System.c4g; numeric-C4ID
  InitializeDef precedes environment/SyncClearance, scenario Initialize and
  queued join (save restore skips scenario Initialize); appends resolve before
  includes.
- Pixel-grid landscape data is authoritative when present. Solid masks sample
  active graphics, and per-object masks remain carriers even when the definition
  has no mask.
- Initialization RNG order is Gravity→Season→YearSpeed→Climate→Wind
  (`Random(151)`); preserve C4SVal defaults for omitted fields. C4ID/object-list
  ordering is also sync state and must not be normalized.
- C4Aul unary `!` binds only its operand except the committed `!x = y` parse.
  Fixture objects default StaticBack; movement fixtures need CATEGORY_OBJECT,
  explicit mobile state and round-to-nearest `fixtoi` expectations.
- Runtime contents insert newest-first within equal category; loaded contents
  stay verbatim. Effect timers run after movement and before fire/life. Enter's
  internal Exit mobilizes and copy-motion snaps to the new container; Collect
  does not copy motion.

## Harnesses

Build and relink the live bridge:

```sh
cd rust && cargo xtask ffi --release
cd .. && cmake --build build-arm64-native --target clonk -j 8
```

Pinned Goldrush shadow run:

```sh
cd build-arm64-native
LC_RUST_ENGINE_RUNTIME=1 LC_PIN_SEED=424242 \
  clonk.app/Contents/MacOS/clonk \
  ../content/Western.c4f/Goldrush.c4s ../build/Tyler.c4p \
  /nonetwork /console
```

Useful probes:

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
- FLNT `Hit` currently reports a headless script error.
- Headless xtask worlds skip player join; intro-driven Goldrush state requires
  the live pinned harness.
- Goldrush's intro Talker consumes crew controls for roughly its first 1000
  frames; scripted input tests must run after the dialog stops.
- `lc-network`'s `control_sync_and_reconnect_smoke` has a TCP timing flake;
  rerun it before treating a lone failure as a regression.
