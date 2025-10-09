# LegacyClonk Rust Port Plan

## Goal
- Ship LegacyClonk with a Rust-based client/server runtime that is behavior-identical to the current C++ build across simulation, rendering, networking, UI, and modding, running the production game rather than demos.

## Current State Snapshot
- Rust crates (`lc-core`, `lc-engine`, `lc-script`, `lc-graphics`, etc.) back the `lc-app` demo and parity tooling; the live game loop, rendering, and UI remain C++ (`C4Game`, `C4GraphicsSystem`, `C4Gui`).
- `RustEngineBridge` can mirror frames alongside the C++ loop for validation, but authoritative physics, control queues, and object lifetimes are still driven by the C++ engine.
- `LC_RUST_ENGINE_RUNTIME` snapshots now bundle per-frame player controls alongside particles and HUD state, giving the recorder/playback harness full I/O context for parity runs.
- Graphics/audio/platform crates operate on CPU surfaces or null backends for comparisons and do not yet present a window, swap chain, input handling, or OS event loop.
- The Rust AUL VM runs scripted procedures inside controlled host contexts, yet large parts of the C4 API surface (effect callbacks, proplist mutation, object enumerators, particles, menus, cutscenes) still rely on C++ glue.
- Build, packaging, and installer flows are CMake/C++ centric; Cargo artifacts are not integrated into CI releases or launcher updates.

## Blocking Gaps For Full Game Runtime
- Authoritative game loop: C++ still owns object creation/destruction, crew control, scheduler ticks, particles, pathfinding, viewport syncing, and landscape updates; the Rust engine lacks coverage for many procedures beyond the demo set.
- Script and engine API coverage: Hundreds of AUL functions, proplist operations, callback hooks, effect priorities, overlay rendering, and global state mutations need Rust equivalents with identical call ordering and edge cases.
- Frontend and IO parity: Window management, software/OpenGL render paths, HUD/GUI widgets, font and text layout, input devices (mouse, keyboard, gamepad), and platform-specific integrations live only in the C++ frontend.
- Networking and concurrency: Lobby discovery, control packet resync, league/master-server protocols, voice/chat relays, and host migration logic are handled by C++ systems beyond the current `lc-network` framing.
- Toolchain compatibility: Scenario/editor workflows, savegames, replay files, localization, diagnostics, and mod packaging expect existing C++ utilities and file formats tied to legacy serialization.

## Port Roadmap (Real Game Focus)
- **Phase 0 · Parity Harness Expansion**
  - Drive the shipping client through `LC_RUST_ENGINE_RUNTIME` for full matches, capturing snapshots, I/O, particles, and HUD state. (Snapshots now embed per-frame particle state alongside I/O, HUD capture records per-owner focus/crew panels, and the Rust parity harness now fails when control logs diverge.)
  - Record canonical replays and savegames from C++ and ensure the Rust engine can import them losslessly.
  - Extend automated diff tooling to compare network traffic, HUD buffers, and rendered surfaces frame-by-frame.
- **Phase 1 · Simulation Authority Flip**
  - Close feature gaps in `lc-engine` (all action procedures, crew AI, physics edge cases, object enumerators, global effects) until the C++ loop can defer object ticking to Rust.
  - Expose every required engine/AUL entry point through FFI so C++ only marshals inputs/outputs while Rust advances world state deterministically.
  - Promote the Rust VM to the primary script runtime, running scenario/system scripts in Rust while shadow-running the C++ VM for audit until clean.
- **Phase 2 · Frontend and Platform Port**
  - Replace `C4GraphicsSystem`, GUI, and input handling with Rust implementations (SDL/OpenGL or wgpu) that reproduce batching, overlay composition, and device quirks.
  - Port HUD/menus/console to Rust (`lc-gui` or successor) and ensure layout, focus, and animations match frame-perfect with legacy recordings.
  - Wire audio mixing to `lc-audio` using a real backend (e.g., cpal/SDL) with streaming music, positional effects, and identical volume curves.
- **Phase 3 · Networking and Services**
  - Extend `lc-network` to handle league lobby, NAT traversal, replay upload, and peer reconnect semantics; let Rust host authoritative control arbitration.
  - Port platform services (patcher, updater, telemetry, crash reporting) or provide Rust bindings to the existing implementations.
  - Validate mixed C++/Rust multiplayer sessions before fully retiring the legacy netcode.
- **Phase 4 · Release Integration**
  - Integrate Cargo builds into CI, produce signed installers, and run automated parity suites on canonical replays before each release.
  - Remove unused C++ modules once Rust reaches feature lock, keeping a fallback branch for hotfix builds until the Rust client proves stable in the wild.

## Validation & Tooling Requirements
- Maintain deterministic replays across both runtimes, gating merges on replay hashes and rendered frame hashes.
- Add exhaustive property-based and fixture-driven tests for AUL builtins, scenario loading, particle systems, and network state machines. (DoEnergy energy delta clamping now covered via proptest in `lc-engine/src/compat.rs`; expand to remaining APIs and subsystems.)
- Provide developer toggles to dump cross-runtime diffs (state, HUD layers, audio mix) and integrate them into CI dashboards.
- Establish performance baselines comparing CPU/GPU usage so regressions surface before release candidates.
