# Rust Port Status

C++ (`../src/`) and `content/` are read-only parity oracles; commits are the
record of completed slices.

## Focus

Engine virtual-play completes Tutorials 01–10. App-keyboard routes complete
01–07 and select Tutorial 08. Tutorial 09 pins seed-zero System-name RNG
placement, breath depletion/refill, the cyan HUD bar, and local/foreign
`DoBreath`. Pinned Gold Rush seed 424242 matches through frame 14,415 after C++
fixed-point script trigonometry; the next mismatch is unknown. Network status
barriers, client lifecycle controls, and authoritative admission cover
fileless/resource-backed players. Alchemy (ALCO+NMGE) intentionally replaces
mana with ingredients. Its seeded bag follows C++
exit→collect→DigDouble→hidden-bag transfer; `ContextMagic`,
MGUP, ABLA aim/release/Airblast, POSE selector/Possession, MFBL→FRBL collection,
native MVLC→FXV1, MTNL terrain opening, FRCS timer audio, and direct CBMU MGUP
casting are pinned. Learned MLGT aims, launches LGTS, and advances its particle
line with C++ wrapping arithmetic; MICS preserves ICEB aim, non-crew cursor,
steering, impact, and Frostwave freeze; FRFS→FSHW→FLAM consumes inflammable
landscape fuel; MQKE consumes IROC, finds ground, launches FXQ1, shakes the
landscape/camera, and expires; MART configures AIR1→LGCN hit artefacts through
its real menus and casts LGCN from an enchanted ROCK impact. Broader combo/spell
effects remain.
In-game left-click MoveTo, 400 ms carryable-object LeftDouble→Get, and >5 px
right-drag crew selection match C++ mouse control. Dragon Rock restores
saved-open entrances; TENT walk+Up is pinned. Sky Race starts with one LOAM
bridge chunk; default deaths announce, and relaunch selects, positions, and
refills the replacement synchronously. Real CLNK ceiling contact, attached
Hangle traversal, auto-stop release, and let-go match C++. Weather uses real
material PXS; Tutorial 07 pins rain cadence, fixed trajectories, and pixels;
lightning has no synthetic launch-frame flash.

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
  movement/collision/attachment, vehicles, lines, containers, and callback
  order; remaining spell effects/combo casts; mouse-context target refill,
  visibility, special cursors, right-drag object/region commands and frame
  rendering, and networking.
- Systems: strict C4Value/save semantics, remaining multiplayer transport and
  resync, exact C4Teams/SafeRandom assignment, configuration/localization, and
  group I/O.

Comparator caveats: presentation RNG is opt-in; fields compare only when both
bridges expose them. Tutorial 07's seed-zero Surface8 is byte-identical; broader
same-seed landscape coverage remains incomplete. Component order is replay-
hashed but not exported by the C++ bridge; unequal-count duplicate IDs remain
an ordered-map model gap.

## Preserve

Preserve fixed-point sync boundaries, shared RNG state/count, reverse
execution, IDs and list ordering, callback/effect timing, movement rules,
script include order, save state, masks, scenario-init RNG, and crew/find ties.
