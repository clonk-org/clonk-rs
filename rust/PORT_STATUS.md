# Rust Port Status

C++ (`../src/`) is the simulation oracle. Full parity is not reached; completed
slices are recorded in `git log -- rust/ parity/`.

## Gate

Workspace nextest, strict clippy, snapshots, and parity are green. Scenario
load/apply is 93/93; activation is 92/93 because CTF_DeepSea's delayed animals
match C++; Tutorials 01–10 are warning-free for 1,200 ticks. The next live
comparator divergence after Goldrush frame 410 is not yet pinned.

```sh
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Behavioral slices also require their scenario/audit gate and, when relevant, a
rebuilt pinned live comparison.

## Open parity

- **Simulation:** solid/rotated masks, landscape scan/RNG order, liquids,
  DigFree/BlastFree, fire/effects/explosions, temperature, map post-init, and
  same-callback reads after deferred landscape writes, plus remaining non-C++
  movement damping/clamping/collision behavior.
- **Commands/objects:** pathfinding/Tick35, FLOAT, Exit/Throw, arrival/intervals,
  vehicle/linekit/dig/trade behavior, OCF/sectors/find order, containers,
  initial-placement lifecycle, permanent crew/base state, elimination, and
  rank/version gates.
- **Script/network/state:** strict C4Value conversion/serialization, remaining
  DefCore/ActMap/effect/menu/callback order, object-info/evaluation payloads,
  control synchronization, and protocol/auth/league behavior.
- **Presentation/resources:** scoreboard GUI, renderer/particles, HUD/player and
  startup/options/update menus, activation/saved music, streaming MIDI/WAV,
  controls/config/locale/group I/O, search/scroll/tooltips, and mission access.

## Comparator caveats

Comma expressions are accepted out of context; `mrfScript` uses scenario scope.
Presentation RNG is ignored unless `LC_RUST_ENGINE_COMPARE_PARTICLES=1`.
Message/render fields compare only when both bridges expose them. Same-seed
landscape is 99.66% byte-equal; C++ bakes MCVehic masks while Rust overlays them.

## Preserve

Keep fixed/integer position and rotation separate except at C++ sync points.
Preserve shared RNG, reverse execution, IDs, definition/category/sector/content
order, newest-first contents, callback timing, movement/attachment rules, script
resource/include order, numeric C4ID order, save restore, authoritative landscape
masks, scenario-init RNG, crew/find ties, and effect ordering.

`cargo xtask parity record|verify` reports Rust expected/C++ actual and stops at
the first mismatch. Xtasks skip joins; see xtask help for diagnostics.
