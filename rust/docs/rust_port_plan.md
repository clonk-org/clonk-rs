# LegacyClonk Rust Port Plan

## Current State (April 2025)
- `lc-game` is a Rust launcher that prepares config/logging then delegates to the legacy C++ binary (`crates/lc-game/src/main.rs`), so the shipped gameplay loop is still C++.
- Core subsystems (engine, script VM, graphics surfaces, audio mixer, networking control layer, resource loader) exist as Rust libraries with unit/snapshot tests (`crates/lc-engine/tests`, `crates/lc-script`, etc.) but run only in isolation or via FFI.
- GUI and launcher tooling focus on diagnostics (`crates/lc-launcher*`) and do not construct an in-engine gameplay window.

## Behavior Parity Gaps
- No Rust binary boots the real game UI; `cargo run` offers only tooling/launcher bins and cannot show the startup menu or scenarios.
- Engine/graphics/audio subsystems are not wired together into a frame loop with real assets, input, and sound.
- Scenario browser, network lobby, and savegame/playback paths have no end-to-end parity coverage with legacy builds.
- QA story relies on synthetic recordings; there is no automated parity check that boots the game and exercises standard scenarios.

## Immediate Priorities
1. Ship a new top-level `lc-app` (or repurpose `lc-game`) that:
   - Loads real installation assets (`System.c4g`, etc.), initialises `lc_engine`, `lc_graphics`, `lc_audio`, and `lc_gui`, and opens a window with winit/pixels (or GPU backend).
   - Presents the startup menu and scenario browser backed by `lc_gui::ScenarioBrowser`.
   - Drives the engine tick/render pipeline with proper input mapping and audio playback.
2. Bridge launcher functionality so updates/support bundles work without calling the C++ runtime.
3. Implement deterministic parity harnesses:
   - Golden recordings from the C++ build for canonical scenarios.
   - Smoke tests that launch the Rust app headlessly (or with render captures) to ensure menu → scenario → player control flows succeed.
4. Harden platform support (paths, config migration, feature flags) and replace any FFI shims still expected by the old codebase.

## Validation Checklist (per release)
- `cargo run` (or `cargo run --bin lc-game`) opens the main menu with working audio, input, and scenario selection.
- Standard scenarios play through to completion with matching engine snapshots vs. legacy baselines.
- Network lobby join/host succeeds with Rust peers; regression tests cover reconnect and resync paths.
- Launcher diagnostics/support bundles operate entirely in Rust.
- CI matrix builds and runs parity smoke tests on Windows/macOS/Linux with real assets.
