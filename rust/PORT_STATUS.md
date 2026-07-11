# Rust Port Status

C++ (`../src/`) and `content/` are read-only parity oracles; commits are the
record of completed slices.

## Focus

Tutorial virtual-play passes 01–03, 05–06, and 08–10. Tutorial 04 is blocked
while scaling beside ELEC; 07 is blocked while grabbing ELEC with GOLD. App
routes complete 01–03 and reach Tutorial 04's ELEV construction. Finish all
tutorial behavior and presentation before resuming Goldrush at its pinned
frame-3327 Decay/DoCon mismatch.

## Gates

```sh
cargo nextest run -p lc-engine -E 'test(/^(real_tutorial(0[1-9]|10)_(virtual_play|route)|real_tutorial02_balloon_platform)::/)'
cargo nextest run -p lc-engine -E 'test(/^virtual_player_harness::/)'
cargo nextest run -p lc-app -E 'test(/app_virtual_keyboard_(completes_real_tutorial0[1-3]|builds_tutorial04)/)'
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Behavior changes also require the relevant scenario sweep/audit and rebuilt
live comparison.

## Open

- Tutorial/UI: C++-exact menus, HUD, evaluation, rendering, audio, and
  startup/options interaction.
- Gameplay: exact landscape/material/PXS behavior, liquids, blasts, weather,
  movement/collision/attachment, vehicles, lines, containers, and callback
  order.
- Systems: strict C4Value/save semantics, synchronized controls/networking,
  configuration/localization, and group I/O.

Comparator caveats: presentation RNG is opt-in; fields compare only when both
bridges expose them; same-seed landscape is 99.66% byte-equal because C++ bakes
MCVehic masks while Rust overlays them.

## Preserve

Preserve fixed-point sync boundaries, shared RNG state/count, reverse
execution, IDs and list ordering, callback/effect timing, movement rules,
script include order, save state, masks, scenario-init RNG, and crew/find ties.
