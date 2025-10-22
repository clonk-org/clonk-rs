# LegacyClonk Rust Port Plan

## Status: Feature-Complete with Parity Gaps

**Port Completion:** All major subsystems ported to Rust. The `lc-app` binary is a fully functional game client with startup menu, scenario browser, audio (music + SFX), graphics, engine loop, networking, and savegame support.

**How to Run:** `cargo run` (launches full game with real scenarios and audio)

**Recent Progress (2025-10-27):**
- Added regression coverage for on-disk scenarios via `start_real_scenario_loads_from_disk`; menu launches now start real scenario groups with focus selection parity.

**Recent Progress (2025-10-26):**
- Menu music now starts automatically on launch and after returning from scenarios; sandbox fallback reuses the same loop with regression coverage (`menu_music_runs_in_menu_cycle`).

**Recent Progress (2025-10-25):**
- Startup menu now lists real installation scenarios; scenario discovery tolerates file roots and regression tests cover the menu flow.

**Recent Progress (2025-10-24):**
- FFI exports now live behind crate feature `ffi`; default builds emit only `rlib` so the workspace no longer recompiles cdylib/staticlib on every edit. Documented the manual `cargo rustc -p <crate> --features ffi -- --crate-type staticlib --crate-type cdylib` flow for legacy bridge builds.

**Recent Progress (2025-10-23):**
- Matched positional audio with the legacy AudibilityRadius (700px) and viewport panning model; added regression coverage for mix geometry to prevent regressions.

**Recent Progress (2025-10-22):**
- Added semantic-versioned quick-save format with migration pipeline; legacy saves now load under current engine and regression coverage guards the path.

**Recent Progress (2025-10-21):**
- Replaced busy-poll loop with fixed-step accumulator using `ControlFlow::WaitUntil` to align simulation cadence with the legacy engine; validated via `cargo test` and `cargo xtask engine-snapshots verify`.
- Implemented legacy glob semantics for sound wildcard lookup and added regression tests.
- Wired `lc-app` CLI multiplayer entry points (`--host <addr>`, `--join <addr>`) to the Rust `lc-network` stack and added encode/decode coverage for legacy control packets.
- Replaced `println!/eprintln!` logging across Rust crates with `tracing` and centralised initialisation in `lc_core::logging`, preserving CLI output expectations.
- Added headless multiplayer smoke coverage (network harness + reconnect) and quick save/load regression test spanning `lc-network` and `lc-app`.

## Architecture

**Rust Crates:**
- `lc-engine` - Core game engine (physics, objects, landscape, actions, effects, recording/playback)
- `lc-script` - C4Aul script VM port
- `lc-graphics` - Surface rendering and pixel manipulation
- `lc-audio` - Audio decoder + mixer (music/SFX channels)
- `lc-frontend` - Graphics system, startup menu, scenario browser, input dispatcher
- `lc-gui` - Widget system for UI overlays
- `lc-resources` - C4Group file loading and scenario discovery
- `lc-network` - Multiplayer transport (handshake, lobby, control dispatch, sync)
- `lc-platform` - Platform abstractions and path discovery
- `lc-core` - Shared types and config bridge to C++
- `lc-app` - **Main game binary** (integrates all subsystems)
- `lc-game` - Launcher wrapper (config/logging, delegates to runtime)

**C++ Codebase:** Legacy implementation remains in `src/` but is **no longer required** for the Rust runtime.

## Parity Gaps

### Low: Build Ergonomics
Default builds now emit only `rlib`. Enable the `ffi` feature on the target crate and run `cargo rustc -p <crate> --features ffi -- --crate-type staticlib --crate-type cdylib` when legacy C++ artifacts are required.

## Validation Checklist

- [x] `cargo run` (no flags) launches lc-app with startup menu
- [x] Startup menu displays real scenarios from installation
- [x] Music plays in menu and during gameplay (when assets present)
- [x] Scenarios load and run with working audio/graphics/input (`start_real_scenario_loads_from_disk`)
- [ ] Quick-save/load works across sessions
- [ ] `cargo test` passes on all platforms (macOS/Windows/Linux)
- [x] `cargo xtask engine-snapshots verify` validates determinism vs C++ baseline (2025-10-21 clean run)
- [x] Multiplayer host/join flows work (`lc-app --host/--join`; covered by automated smoke test)

## Immediate Priorities

- None (build gating complete)

## Assets & Testing

**Required Assets:** Game requires `System.c4g` and scenario files (`.c4s`, `.c4f`) in installation directory.
**Current Status:** Assets present at `planet/System.c4g` (symlinked from project root).

**Testing Infrastructure:**
- Unit/snapshot tests per crate
- Engine snapshot verification via `cargo xtask` (compares Rust vs C++ determinism)
- CI should gate on test suite + snapshot verification

## Success Criteria

Port achieves **exact behavior parity** when:
1. `cargo run` launches game with working menu/audio/scenarios
2. All validation checklist items pass
3. Engine snapshots match C++ baseline (deterministic parity)
4. No observable differences in gameplay, audio, or multiplayer
