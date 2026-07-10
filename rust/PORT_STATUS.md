# Rust Port Status

> C++ (`../src/`) is the bit-exact simulation oracle. History: `git log --
> rust/ parity/`. Full parity is not reached.

## Baseline and gates

- Motion is signed 16.16 `C4Fixed`; RNG is the shared C++ LCG/draw ledger.
- Last full gate (`41e49529`): nextest 2,255 green here (2,256 with populated
  content), workspace clippy/snapshots/parity green. Current engine (`f90a60af`):
  1,305 green plus clippy/snapshots/parity. Load/apply is 93/93, activation audit
  92/93 (CTF_DeepSea's delayed animals match C++), Goldrush 1000 headless ticks.
- The pinned Goldrush frame-410 MONS #582 mismatch is fixed (`b1188cf4`). Run the
  live comparator again and record the next divergence; no later first mismatch
  is pinned yet.

Every completed slice:

```sh
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Also run `scenario-sweep`, `scenario-audit`, relevant `scenario-errors`, and a
rebuilt/relinked pinned live comparison when behavior changes.

## Active gaps

- **Movement/masks:** solid-mask lifetime, rotated attached-mask pushback,
  non-grid overlays, and removal of non-C++ damping/xdir clamp/post-move collision.
- **Landscape/material/fire:** scan/RNG interleaving, `PRETTY_TEMP_CONV`, exact
  DigFree/BlastFree, blast/shake/liquid and seed/player/MapZoom order,
  Landscape.txt, `PostInitMap`/`KeepMapCreator`, fixture segment removal;
  checked-fire timing/returns, incineration chain, negative/global effects,
  detach/extinguish/causes/smoke/explosion helpers.
- **Commands/controls:** C4PathFinder/Tick35, DFA_FLOAT, contained-target Exit,
  UpdateInterval/arrival, FlightControl/Acquire/Put-Ty, contained Throw,
  non-control vehicle commands, linekit connect/dig, auto-sell/snow-dig,
  version/rank gates and trade UI.
- **Objects/players:** `InMat`/closed containers, preview OCF/addtop, permanent
  info, DFA_CONNECT, object-value/no-sort finds, immediate sector registration;
  base production/value, crew persistence, elimination presentation and
  script-player/control-sync behavior; protocol/auth/league.
- **Script/content:** C4Value strict conversion/interning/serialization, named
  values and loaded multi-direction `Dir`, `Oversize`, numeric ActMap,
  GetComponents/CalcDefValue/DefCore fields, effect recovery, definition/menu
  seams, `FnExit` edge cases; `CanConcatPictureWith`, object-info payloads,
  data-bearing network `MenuSelect`, async text progress, scenario callbacks,
  AutoContextMenu/CloseCommand/PlayerMenu ordering. Tutorial blockers:
  `SetPlrShowControl`, `ExecuteCommand`, `PlaceVegetation`, `CheckEnergyNeedChain`,
  `CastPXS`, `IsNetwork`, C4ID `CustomMessage`; localized omitted
  `SetNextMission` labels and next-mission UI/launch remain.
- **Music/audio:** `Music`/`MusicLevel` host calls work; C++ activation order and
  app/engine enabled state do not. `MusicEnabled`/`MusicLevel`/`PlayList`
  save/restore, random continuation, streaming/looped MIDI and definition WAVs remain.
- **Resources/presentation:** sky, control/config/name/locale/group
  I/O; renderer transforms/GL/shaders/panning; particle graphics; tutorial
  difficulty/character-choice GUI, HUD/board/player menus; remaining startup,
  player, network, option, update/mission-access, search/scroll and tooltip flows.

## Comparator-only divergences

- Parser accepts out-of-context comma expressions; `mrfScript` resolves against
  scenario rather than global script.
- Particles use presentation RNG outside `C4ControlSyncCheck`; opt in with
  `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
- Message/render fields compare only when both bridges provide them; HUD crew
  sorting is bridge-only.
- Same-seed landscape is 99.66% byte-equal: C++ bakes MCVehic masks, Rust overlays.

## Load-bearing invariants

- Keep integer `x/y/r` apart from probed `fix_x/fix_y/fix_r`, and raw
  `r/fix_r/rdir` independent. Only SetAction, CopyMotion, ForcePosition, and Exit
  resync fixed coordinates; DoCon/TargetBounds change integers only.
- Preserve shared LCG `RandomHold`/`RandomCount`, reverse execution, monotonic IDs,
  StaticBack→Structure→Vehicle→Living→Object, loaded/runtime same-def order,
  Line/StaticBack cases, Enter/Exit stability, container-before-content, and
  newest-first runtime contents.
- Loaded objects run no callbacks. Fresh: Construction at Con 0, initial DoCon,
  then Completion→Initialize at FullCon. Only pre-insertion actions defer one
  Start/Abort pair; otherwise SetAction is synchronous Start→End→Abort. Never
  requeue `callbacks_dispatched`; phase wrap uses the old action.
- End-of-ExecAction: WALK/HANGLE `|xdir|*10`, SCALE `|ydir|*14`, SWIM
  `ValByPhysical(160,Swim)*10`, DIG `ValByPhysical(125,Dig)*40`. WALK faces by raw
  fixed sign/old action; default Attach zeroes dirs/mobilizes; missing Jump keeps
  the action.
- Script order: definition resources, folder/scenario defs, pack System.c4g,
  scenario Script.c, scenario System.c4g; appends before includes. Numeric-C4ID
  InitializeDef precedes environment/SyncClearance, scenario Initialize, queued
  join; save restore skips scenario Initialize.
- Pixel landscape is authoritative; masks sample active graphics and remain
  per-object carriers. Init RNG: Gravity→Season→YearSpeed→Climate→Wind (`Random(151)`);
  preserve omitted C4SVal defaults and C4ID/list order.
- Unary `!` binds only its operand except committed `!x = y`. Fixtures default
  StaticBack; movement needs CATEGORY_OBJECT, mobile state, nearest `fixtoi`.
  Crew is newest-first; GetHiRank keeps the first tie. Effects follow movement
  before fire/life; Enter's Exit mobilizes/copies motion, Collect does not.

`cargo xtask parity record|verify` labels Rust expected/C++ actual and stops at
the first mismatch. Xtasks skip joins; Goldrush Talker consumes controls for
about 1000 frames. See xtask help for diagnostic environment variables.
