# Local patches

This directory contains the published `winit` 0.30.13 source from crates.io.

Upstream's Wayland keyboard handler `remove`s the key-repeat `calloop::timer`
on every press, release, and focus change, then inserts a new one. calloop
reuses the slab slot (same id, new version). If that timer already expired
in the current `poll`, dispatch still holds the old token and warns:

`Received an event for non-existence source: TokenInner { id: 3, ... }`

Those warnings showed up in ordinary play (clonk-org/clonk-rs#311). Muting
`calloop::loop_logic` hid them; it did not stop the race.

Local changes:

- keep one repeat timer per seat;
- `disable` it on cancel so a leftover timeout is a no-op instead of a
  vacated slot;
- `update` the same source when the delay needs to change;
- `remove` only when the keyboard itself is dropped.

Upstream tracking: the race is the documented calloop token-reuse path
combined with winit's remove+insert in
`src/platform_impl/linux/wayland/seat/keyboard/mod.rs`.
