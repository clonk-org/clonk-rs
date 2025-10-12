# LegacyClonk Rust Port Plan

## Current Snapshot
- `lc-game` still defers to the shipping C++ runtime; Rust binaries remain developer-only.
- `lc-app` opens a winit/pixels window, lists real scenarios from install/user roots, runs them through `lc_engine`/`lc_frontend`, streams background music via `lc_audio`, and resolves SFX from scenario/global sound packs (synthetic fallback only when assets are missing).
- Rust subsystems (engine, script VM, graphics, audio, networking, resources, GUI) are unit/snapshot tested in isolation; only the preview harness stitches them together.
- Startup/menu flow now renders HUD overlays sourced from engine HUD metadata, highlighting crew focus while broader UI polish continues; quick-save parity is complete.

## Parity Gaps
- Startup/menu flow still needs final production polish and richer menu presentation beyond the new HUD overlays; quick-save support remains complete.
- Gameplay loop lacks persistent settings and integration with networking/editor toolchains; C++ runtime remains authoritative for shipped builds.
- CI still misses smoke/parity runs that boot the Rust runtime and compare against C++ recordings.

## Immediate Priorities
1. Standalone Rust client parity
   - [x] Boot window + scenario browser + deterministic engine loop in `lc-app`.
   - [x] Loop background music via `lc_audio` (real scenario tracks when present, sandbox fallback otherwise).
   - [x] Promote Rust UI/input/audio to production fidelity
     - [x] Save/load parity via quick-save `.lcsave` snapshots in user data.
     - [x] HUD overlays, menu integration, and scripted metadata polish.
     - [x] SFX mixer wiring, scripted audio hooks, and asset resolution via registered sound groups.
2. Launcher parity: retire the C++ delegation for updates/support bundles and keep all prelaunch flows in Rust.
3. Automated parity harness: record canonical scenarios from the C++ build, replay them through Rust headlessly, and gate CI on the comparison.

## Validation Targets
- `cargo run -p lc-app` enters the startup menu, launches scenarios, and keeps music running without runtime warnings.
- `cargo test` and `cargo xtask engine-snapshots verify` stay green across macOS/Windows/Linux.
