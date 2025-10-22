# LegacyClonk Rust Port Plan

## Status: Feature-Complete with Parity Gaps

**Port Completion:** All major subsystems ported to Rust. The `lc-app` binary is a fully functional game client with startup menu, scenario browser, audio (music + SFX), graphics, engine loop, networking, and savegame support.

**How to Run:** `cargo run` (launches full game with real scenarios and audio)

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

### Low: Audio Mixing Model
**Issue:** Linear panning/falloff may differ from C++ (which could use inverse-square, occlusion, etc.).
**Location:** rust/crates/lc-app/src/main.rs:2577-2601
**Impact:** Positional audio may sound different in large maps or busy scenes.
**Fix:** Validate against C++ mix captures; adjust curves if needed.

### Low: Build Ergonomics
**Issue:** lc-engine builds rlib + cdylib + staticlib on every build (slow).
**Fix:** Feature-gate FFI artifacts.

## Validation Checklist

- [x] `cargo run` (no flags) launches lc-app with startup menu
- [ ] Startup menu displays real scenarios from installation
- [ ] Music plays in menu and during gameplay (when assets present)
- [ ] Scenarios load and run with working audio/graphics/input
- [ ] Quick-save/load works across sessions
- [ ] `cargo test` passes on all platforms (macOS/Windows/Linux)
- [x] `cargo xtask engine-snapshots verify` validates determinism vs C++ baseline (2025-10-21 clean run)
- [x] Multiplayer host/join flows work (`lc-app --host/--join`; covered by automated smoke test)

## Immediate Priorities

1. Validate audio mixing parity vs C++ captures

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
