# Rust Port Status

C++ (`../src/`) and `content/` are read-only parity oracles; commits are the
record of completed slices.

## Focus

Engine virtual-play completes 01–03, 05, and 07–10; exact saved-local RNG now
invalidates the 04/06 route checkpoints and app 04/06/07 wrappers. Other app
keyboard routes and selected 08 are covered. Tutorial 09 pins seed-zero
System-name RNG placement, breath depletion/refill, the cyan HUD bar, and local/foreign
`DoBreath`. Pinned Gold Rush seed 424242 matches through frame 14,415 after C++
fixed-point script trigonometry; the next mismatch is unknown. Network status
barriers, admission/lifecycle, prepared observer hosts, selected-player/resource
publication, exact advertised references, C4Group cores, and initial
parameter/scenario/game/dynamic serialization cover fileless and resource-backed
players; the retained network dialog feeds recursive host selection, and typed
lobby countdown/ready-check traffic is live. Scenario definition lists use
classic quoted/numbered parsing and load
explicit global packs before ancestor-local packs. Alchemy (ALCO+NMGE)
intentionally replaces mana with ingredients. Its seeded bag follows C++
exit→collect→DigDouble→hidden-bag transfer; `ContextMagic`, MGUP/MGDW
global-effect merging, ABLA aim/release/Airblast, POSE selector/Possession,
MFBL→FRBL collection, MFFW's seven linked FCWS segments with synchronous
stuck-crew ejection, phase-mask rebakes, and damage/timer expiry,
native MVLC→FXV1, MWP2 paired portals/base transfer, MTNL terrain opening,
FRCS timer audio, and direct CBMU MGUP casting are pinned. Learned MLGT aims,
launches LGTS, and advances its particle line with C++ wrapping arithmetic;
MICS preserves ICEB aim, non-crew cursor,
steering, impact, and Frostwave freeze; FRFS→FSHW→FLAM consumes inflammable
landscape fuel; MQKE consumes IROC, finds ground, launches FXQ1, shakes the
landscape/camera, and expires; MART configures AIR1→LGCN hit artefacts through
its real menus and casts LGCN from an enchanted ROCK impact; XCRS consumes its
recipe, sacrifices energy, and intercepts `AssignDeath` into delayed burning
reincarnation. Learned GGHG sustains Magic and heals nearby crew;
definition-owned effect `FindObject` uses global coordinates. Broader
combo/spell effects remain.
`Set/GetVisibility`, saved `Visibility=`/numbered `Locals=`, all C++ masks,
layers/local bits,
base/object-overlay rendering (including contained overlay-only targets and
`TargetPos` parallax/top faces), mouse picking, and target-message suppression
are live; shipped MINV pins start/stop restoration and native `ModulateColor` math.
In-game mouse matches left MoveTo, 400 ms carryable LeftDouble→Get, >5 px
landscape frames with `CRed`/candidate marks, 20-item carryable Drop/Throw,
Control-container Put, Grab=1 vehicle PushTo, HUD-region right-up, and
inventory-region same-ID Set→Append ordering. Dragon Rock restores saved-open
entrances; TENT walk+Up is pinned. Sky Race starts with one LOAM
bridge chunk; deaths/relaunch, 100% progress, rivalry elimination/retirement,
GOAL-delayed game over, and winner evaluation are pinned. Real CLNK ceiling
contact, attached Hangle traversal, auto-stop release, and let-go match C++.
Tutorial 05's real CATA follows its launched payload through `SetPlrView`; the
next regular non-menu press resets the camera to ViewCursor/Cursor like C++.
Weather uses real material PXS; Tutorial 07 pins rain cadence, fixed
trajectories, and pixels;
lightning has no synthetic launch-frame flash. Regular CONNECT lines use the
C++ PathFree walk, 4/8/12-pixel terrain-bend search, and old-endpoint
PathFreeIgnoreVehicle fallback across solid masks and closed borders.
Power/source/drain/rope/colored/vertex rendering uses absolute live vertices,
C4.PAL colors/locals, and half-open start-marked segments; Lightning
`DrawBolt` presentation RNG remains open.

Menu parity is tracked recursively in `MENU_PARITY.md`. It covers every C++
startup/in-game/object/script/modal screen and nested transition found in the
source and shipped content; top-level visual similarity is not treated as full
menu parity.

## Gates

```sh
cargo nextest run -p lc-engine -E 'test(/^(real_tutorial(0[1-9]|10)_(virtual_play|route)|real_tutorial02_balloon_platform)::/)'
cargo nextest run -p lc-engine -E 'test(/^virtual_player_harness::/)'
cargo nextest run -p lc-app -E 'test(/app_virtual_keyboard_(completes|flings|opens)/)'
cargo nextest run --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Behavior changes also require the relevant scenario sweep/audit and rebuilt
live comparison.

## Open

- Tutorial/UI: exact menus, HUD, evaluation, audio, and startup/options; 2×/3×
  startup-main text is native, while fractional/other scaled text remains blurred.
- Gameplay: exact landscape/material/PXS behavior, liquids, blasts, weather,
  movement/collision/attachment, vehicles, containers, and callback order;
  remaining spell effects/combo casts;
  mouse-context target refill, special cursors, and networking.
- Systems: strict C4Value/save semantics, remaining multiplayer transport and
  resync, exact C4Teams/SafeRandom assignment, configuration/localization, and
  group I/O.

Comparator caveats: presentation RNG is opt-in; fields compare only when both
bridges expose them (the C++ bridge omits layer/visibility/player hostility).
Tutorial 07's seed-zero Surface8 is byte-identical; broader
same-seed landscape coverage remains incomplete. Component order is replay-
hashed but not exported by the C++ bridge; unequal-count duplicate IDs remain
an ordered-map model gap.

## Preserve

Preserve fixed-point sync boundaries, shared RNG state/count, reverse
execution, IDs and list ordering, callback/effect timing, movement rules,
script include order, save state, masks, scenario-init RNG, and crew/find ties.
