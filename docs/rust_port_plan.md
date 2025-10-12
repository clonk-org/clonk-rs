# LegacyClonk Rust Port Plan

## Current Snapshot
- `lc-game` still defers to the shipping C++ runtime; Rust binaries remain developer-only.
- `lc-app` opens a winit/pixels window, lists real scenarios from install/user roots, runs them through `lc_engine`/`lc_frontend`, and now streams background music via `lc_audio` (falling back to a synthetic loop when no track is packaged).
- Rust subsystems (engine, script VM, graphics, audio, networking, resources, GUI) are unit/snapshot tested in isolation; only the preview harness stitches them together.

## Parity Gaps
- Startup/menu flow still lacks HUD polish, scripted metadata coverage, and production-ready UI polish; quick-save support now exists but broader UI parity remains.
- Gameplay loop lacks SFX mixing, scripted audio hooks, persistent settings, and integration with networking/editor toolchains; C++ runtime remains authoritative for shipped builds.
- CI still misses smoke/parity runs that boot the Rust runtime and compare against C++ recordings.

## Immediate Priorities
1. Standalone Rust client parity
   - [x] Boot window + scenario browser + deterministic engine loop in `lc-app`.
   - [x] Loop background music via `lc_audio` (real scenario tracks when present, sandbox fallback otherwise).
   - [ ] Promote Rust UI/input/audio to production fidelity
     - [x] Save/load parity via quick-save `.lcsave` snapshots in user data.
     - [ ] HUD overlays, menu integration, and scripted metadata polish.
     - [ ] SFX mixer wiring and scripted audio hooks.
2. Launcher parity: retire the C++ delegation for updates/support bundles and keep all prelaunch flows in Rust.
3. Automated parity harness: record canonical scenarios from the C++ build, replay them through Rust headlessly, and gate CI on the comparison.

## Validation Targets
- `cargo run -p lc-app` enters the startup menu, launches scenarios, and keeps music running without runtime warnings.
- `cargo test` and `cargo xtask engine-snapshots verify` stay green across macOS/Windows/Linux.
