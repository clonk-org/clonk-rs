# Rust Port Status

C++ (`../src/`) and `content/` are read-only parity oracles; commits are the
record of completed slices.

## Focus

Engine virtual-play completes Tutorials 01–10. App-keyboard routes complete
01–07 and select Tutorial 08. Pinned Gold Rush seed 424242 now matches through
frame 14,415 after fixing C++ fixed-point script trigonometry. The live run was
capped there, so the next mismatch is unknown. Network status barriers, client
lifecycle controls, and authoritative admission cover fileless/resource-backed
players. Alchemy (ALCO+NMGE) intentionally has no mana bar and an
ingredient-only footer. Player right-up opens classic `ContextMagic`; MGUP,
ABLA aim/release/Airblast, POSE selector/Possession, MFBL→FRBL collection, and
native MVLC→FXV1 are pinned. CBMU accepts its first class key; full combo casts
and broader spell effects remain.

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
  visibility, right-drag, click placement, and networking.
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
