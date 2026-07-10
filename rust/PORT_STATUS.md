# Rust Port Status

C++ (`../src/`) is the bit-exact simulation oracle; full parity is not reached.
History is in `git log -- rust/ parity/`.

## Baseline

- Signed 16.16 `C4Fixed`, the C++ LCG, and its shared draw ledger are in place.
- Current full gate: 2,300 nextest tests, workspace clippy, snapshots, and parity
  green; scenario load/apply 93/93; activation audit 92/93 because CTF_DeepSea's
  delayed animals match C++; Tutorials 01-10 are warning-free for 1,200 ticks.
- Goldrush's frame-410 MONS #582 mismatch is fixed. The next live-comparator
  divergence has not been pinned.

For every completed slice, run:

```sh
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Behavior changes also require the relevant scenario sweep/audit/error run and,
when applicable, a rebuilt pinned live comparison.

## Remaining gaps

- **Movement/masks:** solid-mask lifetime, rotated attached-mask pushback,
  non-grid overlays, and removal of non-C++ damping/clamping/collision behavior.
- **Landscape/fire:** exact scan/RNG ordering, temperature conversion,
  DigFree/BlastFree, liquids/seeds/MapZoom/Landscape.txt, map post-init, fixture
  removal, fire/effect timing, causes, smoke, and explosion helpers.
- **Commands:** pathfinding/Tick35, FLOAT, contained Exit/Throw, command arrival
  and intervals, vehicle/linekit/dig/trade behavior, and version/rank gates.
- **Objects/players/network:** containers/OCF/sectors/find ordering, permanent
  crew/base state, elimination/control sync, protocol/auth/league, and
  data-bearing network menus.
- **Script/content:** strict C4Value conversion/serialization, named values and
  multi-direction `Dir`, remaining DefCore/ActMap/effect/menu/callback ordering,
  object-info payloads, async progress, and localized `SetNextMission` labels.
- **Audio/presentation/resources:** activation and saved music state, streaming
  MIDI/definition WAVs, sky/control/config/locale/group I/O, exact renderer and
  particles, difficulty/character-choice/HUD/player menus, and remaining startup,
  option, update, mission-access, search/scroll, and tooltip flows.

## Comparator limits

- Out-of-context comma expressions are accepted; `mrfScript` uses scenario scope.
- Presentation particles use untracked RNG unless
  `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
- Message/render fields compare only when both bridges provide them; HUD crew
  sorting is bridge-only.
- Same-seed landscape is 99.66% byte-equal; C++ bakes MCVehic masks while Rust
  overlays them.

## Preserve while porting

- Keep integer and fixed `x/y/r` independent; only the C++ synchronization points
  may copy between them. Preserve shared RNG state, reverse execution, monotonic
  IDs, category/definition order, sector/content order, and newest-first contents.
- Loaded objects run no callbacks. Fresh construction, completion, initialization,
  action start/end/abort, and deferred callbacks must retain C++ order.
- Preserve exact WALK/HANGLE/SCALE/SWIM/DIG speed rules, attachment behavior, raw
  facing sign, and missing-Jump behavior.
- Preserve script-resource/include order, numeric-C4ID initialization order, save
  restore behavior, authoritative pixel landscape/masks, and scenario-init RNG.
- Preserve C4Script precedence, category/mobile/rounding movement requirements,
  crew/find tie order, and effect ordering relative to movement, fire, and life.

`cargo xtask parity record|verify` reports Rust expected/C++ actual and stops at
the first mismatch. Xtasks skip joins; see xtask help for diagnostic variables.
