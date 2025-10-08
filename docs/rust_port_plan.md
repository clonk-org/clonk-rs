# LegacyClonk Rust Port Evaluation

## Current Status
- Rust crates continue to power the demo harness (`lc-app`) and validation tooling; the shipping runtime still routes gameplay through C++.
- Scenario loading, action/event bridging, float/swim/lift physics, and GUI/input parity are in place for the validation stack.
- Movement profiles now expose walk/float/swim speeds and accelerations, and `ActionProcedure::Walk` mirrors C++ steering, braking, and facing updates.
- Scale, hangle, and dig command handling now mirror the C++ procedures, including configurable climb/hang/dig movement profiles.

## Priority Backlog
1. ✅ Walk procedure command movement parity (Rust engine accelerates/decelerates via per-definition walk profiles and exposes `movement.walk.*` manifest knobs).
2. ✅ Expand grounded procedure parity (Scale/Hangle/Dig command handling) so climbing and digging match C++ behaviour.
3. ✅ Wire the Rust gameplay loop into the main C++ runtime behind a feature toggle to validate parity during live rounds (inputs, HUD, save/load).

## Notes
- Scenario manifests can now provide `movement.walk.speed` and `movement.walk.acceleration` to tune procedures per definition.
- Additional knobs: `movement.scale.*`, `movement.hangle.*`, and `movement.dig.speed` feed the new grounded procedure parity.
- Runtime parity toggle `LC_RUST_ENGINE_RUNTIME` now boots the Rust engine alongside the C++ loop and compares live snapshots per frame, reinitialising with scenario seeds during startup.
