# LegacyClonk Rust Port Evaluation

## Current Status
- Rust crates continue to power the demo harness (`lc-app`) and validation tooling; the shipping runtime still routes gameplay through C++.
- Scenario loading, action/event bridging, float/swim/lift physics, and GUI/input parity are in place for the validation stack.
- Movement profiles now expose walk/float/swim speeds and accelerations, and `ActionProcedure::Walk` mirrors C++ steering, braking, and facing updates.
- Scale, hangle, and dig command handling now mirror the C++ procedures, including configurable climb/hang/dig movement profiles.
- Parity runs can capture per-frame Rust engine snapshots by setting `LC_RUST_ENGINE_RUNTIME_SNAPSHOT` while the validation toggle is enabled.
- Push procedure parity keeps the pusher aligned with the target, imparts horizontal velocity based on command direction, and cleanly reverts when the target is unavailable.
- Pull procedure parity now mirrors the C++ towing offsets, range checks, and velocity handling for validation runs.
- `std_markup::strip_markup` now handles unterminated inline image tags exactly like the legacy C++ path, removing stray `{{` brace pairs instead of leaving them in Rust output.

## Priority Backlog

- (none; markup parity gap resolved)

## Notes
- Scenario manifests can now provide `movement.walk.speed` and `movement.walk.acceleration` to tune procedures per definition.
- Additional knobs: `movement.scale.*`, `movement.hangle.*`, and `movement.dig.speed` feed the new grounded procedure parity.
- Runtime parity toggle `LC_RUST_ENGINE_RUNTIME` now boots the Rust engine alongside the C++ loop and compares live snapshots per frame, reinitialising with scenario seeds during startup.
- Setting `LC_RUST_ENGINE_RUNTIME_SNAPSHOT=/path/to/log.ndjson` streams Rust runtime snapshots (one JSON object per frame) to aid diffing during live parity sessions.
