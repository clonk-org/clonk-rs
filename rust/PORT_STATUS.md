# Rust Port Status

> 2026-07-09. C++ (`../src/`) is the oracle; parity is bit-exact simulation state.
> History: `git log -- rust/ parity/`.

## Current state

- Pinned Goldrush (`LC_PIN_SEED=424242`) matches through frame 409; frame 410
  first differs at object 582's integer/subpixel position.
- Motion is signed 16.16 `C4Fixed`; RNG is the shared C++ LCG/draw ledger.
- Load/apply: 93/93. Audit: 92/93; CTF_DeepSea's animal result matches C++ after
  water fills. Goldrush: 1000 clean headless ticks. Full parity remains incomplete.

## Gates

Every finished slice:

```sh
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

When relevant: `cargo xtask scenario-sweep`, `cargo xtask scenario-audit`, and
`cargo xtask scenario-errors Goldrush --ticks 1000`. Behavioral sync slices also
require a rebuilt/relinked pinned live run and its new first mismatch.

## Open parity gaps

- **Movement/masks:** `SetSolidMask` lifetime; rotated attached-mask pushback;
  non-grid overlays; delete non-C++ damping, 12px xdir clamp, post-move collision.
- **Landscape/material:** scan/interleaving, `PRETTY_TEMP_CONV`, pixel-exact
  DigFree/BlastFree, blast/shake/liquid order, seed/player/MapZoom bracketing,
  Landscape.txt, `PostInitMap`/`KeepMapCreator`; fixture columns remove segments.
- **Effects/fire:** checked-fire timing/returns, engine-incineration check-chain;
  negative-priority persistence/stop; GLOBAL checks/shadowing; detach, Tick5
  extinguish, causes, smoke, Explosion helpers.
- **Commands/controls:** C4PathFinder/Tick35, DFA_FLOAT, contained-target Exit,
  UpdateInterval/arrival, FlightControl, Acquire timing, Put `Ty`; contained throw,
  non-control vehicle `SetCommand`, linekit DigDouble/DFA_CONNECT,
  auto-sell/snow-dig, version/rank gates, trade UI.
- **Objects/find/OCF:** `InMat`/`ClosedContainer`, preview OCF/`At` addtop,
  permanent info, DFA_CONNECT, `Sort_Value` by object value, no-sort FindObject2
  early exit, immediate sector registration during calls.
- **Players/game/network:** base production, asset value, crew persistence,
  elimination presentation/script-player flag, control sync, protocol/auth/league.
- **Definitions/values/script:** `Oversize`, numeric ActMap,
  GetComponents/CalcDefValue, DefCore fields, C4Value intern/serialization/strict
  conversion, effect error recovery, definition/menu seams, fixed-dir precision,
  FnExit cancel-attach/bounds/Ejection/Departure.
- **Resources/config:** sky/LaunchCloud, control prefs, names/promotions/locales,
  group write/gzip/CRC/directory order.
- **Hosts/menus:** `CanConcatPictureWith`, object-info payloads, data-bearing app
  network `MenuSelect`, async text-progress conversion, scenario callbacks,
  AutoContextMenu/CloseCommand, and PlayerMenu ordering.
- **Presentation:** renderer transforms/GL/shaders/landscape/panning; playlist,
  looped/streamed MIDI, WAV/definition effects; GUI layout/text/portraits and
  Particle.txt graphics/procedures; launcher Network/Player/Options/About input,
  player/update/first-start/search/CanOpen/icons/files/mission access/scrolling;
  player-menu pause/abort/
  goal/rule/player/key/team/admin/network/MaxPlayers/scrolling/markup/tooltips;
  HUD inventory/control/mouse rows, board animation/colors, descriptors, FlashCom,
  Base/NeedEnergy, custom ranks, NO-DIG/NO-CHOP feedback.

## Deliberate/comparator-only divergences

- Parser accepts out-of-context comma expressions; `mrfScript` resolves against
  scenario, not global script.
- Particles use presentation RNG outside `C4ControlSyncCheck`; opt in with
  `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
- Message-list/render-surface fields compare only when both bridges provide them;
  HUD crew sorting is bridge-only.
- Same-seed landscape is 99.66% byte-equal; C++ bakes MCVehic masks, Rust overlays.

## Load-bearing invariants

- Keep integer `x/y/r` separate from `fix_x/fix_y/fix_r`; snapshots expose the
  former, parity probes the latter. Keep raw `r/fix_r/rdir` independent.
- RNG is the shared C++ LCG with global `RandomHold`/`RandomCount`.
- Preserve reverse object-list execution, monotonic unreused IDs, category order
  StaticBack→Structure→Vehicle→Living→Object, loaded order, runtime same-def
  clustering, Line/StaticBack cases, Enter/Exit non-reordering,
  container-before-content execution.
- Loaded objects restore without callbacks. Fresh creation runs Construction at
  Con 0 then initial DoCon; crossing FullCon runs Completion→Initialize. Only
  pre-insertion action callbacks defer one Start/Abort pair; otherwise SetAction
  callbacks synchronously run Start→End→Abort. Never requeue an update marked
  `callbacks_dispatched`; phase wrap uses the old action.
- SetAction, CopyMotion, ForcePosition, and Exit resync fixed coordinates;
  DoCon/TargetBounds change only integers. Other integer/fixed splits are valid.
- End-of-ExecAction rates: WALK/HANGLE `|xdir|*10`, SCALE `|ydir|*14`, SWIM
  `ValByPhysical(160,Swim)*10`, DIG `ValByPhysical(125,Dig)*40`. WALK faces by raw
  fixed sign/captured old action. Default Attach zeroes dirs/mobilizes; missing Jump
  preserves the action.
- Crew lists are newest-first; GetHiRank keeps the first equal rank and join uses
  that cursor. HUD sorting is bridge-only.
- Script order: definition resources, folder/scenario defs, pack System.c4g,
  scenario Script.c, scenario System.c4g; appends before includes. Numeric-C4ID
  InitializeDef precedes environment/SyncClearance, scenario Initialize, queued
  join; save restore skips scenario Initialize.
- Pixel landscape is authoritative. Solid masks sample active graphics and remain
  per-object carriers without a definition mask.
- Initialization RNG order is Gravity→Season→YearSpeed→Climate→Wind
  (`Random(151)`); preserve omitted C4SVal defaults and C4ID/object-list order.
- C4Aul unary `!` binds only its operand except committed `!x = y`. Fixtures
  default StaticBack; movement fixtures need CATEGORY_OBJECT, mobile state, and
  round-to-nearest `fixtoi`.
- Runtime contents insert newest-first per category; loaded contents stay verbatim.
  Effects run after movement and before fire/life. Enter's Exit mobilizes/copies
  motion; Collect does not.

## Harness

```sh
cmake --build build-arm64-native --target clonk -j 8
cd build-arm64-native
LC_RUST_ENGINE_RUNTIME=1 LC_PIN_SEED=424242 \
  clonk.app/Contents/MacOS/clonk \
  ../content/Western.c4f/Goldrush.c4s ../build/Tyler.c4p /nonetwork /console
```

`cargo xtask parity record|verify` labels Rust expected/C++ actual and stops at the
first mismatch. Diagnostic environment variables are documented by xtask help.
Live caveats: xtasks skip joins; Goldrush Talker consumes controls for ~1000 frames.
