# Rust Port Status

C++ (`../src/`) and `content/` are read-only parity oracles. Completed slices
are in `git log -- rust/ parity/`.

## Current

Tutorials 01–10 complete their real victory flows through player controls and
live menus. Tutorial 02 separately pins the Clonk to BALN's moving platform,
including a real-scenario app replay through physical X/X/S keys. App tests
also replay Tutorial 03 through physical C/S/X/A keys into its localized,
mouse-disabled context and C++-style Buy menu. S/Z/X/C mapping, unbound arrows,
jumping, and classic/AutoStop release behavior are pinned.

Next: close tutorial presentation/UI parity, then resume Goldrush at its pinned
first mismatch: frame 3327, object #1451, Decay/DoCon Y synchronization; RNG is
still aligned.

## Gates

```sh
cargo nextest run -p lc-engine -E 'test(/^(real_tutorial(0[1-9]|10)_(virtual_play|route)|real_tutorial02_balloon_platform)::/)'
cargo nextest run -p lc-engine -E 'test(/^virtual_player_harness::/)'
cargo nextest run -p lc-app -E 'test(/app_virtual_keyboard_(keeps_tutorial02|enters_tutorial03)/)'
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Behavior changes also require the relevant scenario sweep/audit and rebuilt
live comparison.

## Remaining parity

- Tutorial presentation: C++-exact menus, HUD, evaluation screens, rendering,
  audio, and startup/options interaction.
- Simulation: landscape/material/RNG order, liquids, masks, fire/effects,
  movement/collision/attachment, pathfinding, vehicles, lines, digging,
  OCF/sectors/find order, containers, crew/base state, and callback timing.
- Script/state/network/resources: strict C4Value/save semantics, effect/menu
  ordering, synchronized controls/protocols, renderer/particles, audio,
  configuration/localization, and group I/O.

Comparator caveats: presentation RNG is opt-in; message/render fields compare
only when both bridges expose them; same-seed landscape is 99.66% byte-equal
because C++ bakes MCVehic masks while Rust overlays them.

## Preserve

Keep fixed-point state separate from integer projections except at C++ sync
points. Preserve shared LCG state/count, reverse execution, IDs and all
definition/category/sector/content/C4ID ordering, newest-first contents,
callback/effect timing, movement/attachment rules, script include order, save
state, authoritative landscape masks, scenario-init RNG, and crew/find ties.
