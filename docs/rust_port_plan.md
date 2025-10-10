# LegacyClonk Rust Port Plan

## North Star
- `cargo run -p lc-game` boots the production Startup Menu, runs the full game loop, and exposes every feature the current C++ release ships today.
- The Rust runtime must be behavior-identical across simulation, UI, networking, modding, and tooling; parity is measured against existing regression packs and live builds on all supported platforms.
- Continue shipping C++ binaries until deterministic replays, multiplayer soak, and UI flows stay green with the Rust build.

## Reality Check
- Rust crates (`lc-engine`, `lc-script`, `lc-graphics`, `lc-network`, `lc-audio`, `lc-gui`, `lc-platform`) currently back parity tooling; `C4Game`, `C4GraphicsSystem`, and `C4Gui` still drive the live runtime.
- `lc-app` demo harness retired; workspace packaging now targets the `lc-game` launcher that bridges into the shipping runtime.
- `RustEngineBridge` mirrors frames when `USE_RUST_ENGINE_VALIDATION` is enabled, but the authoritative scheduler, effects, pathfinding, lobby, and UI stacks are still C++.
- Build, packaging, and updater flows are CMake-first; Cargo outputs are never shipped to players or CI artifacts.

## Parity Gaps
- **Boot & Platform:** configuration migration, patcher/updater, localization, logging/crash handling, launcher integration.
- **Assets & Resources:** group parsing, definition loading, scenario discovery, dynamic downloads, string tables, shader/media pipelines.
- **Runtime Authority:** crew lifecycle, scheduler, particles, pathfinding, weather, save/load, deterministic recordings owned by Rust. Bridge action parameters (`SetBridgeActionData`) now processed by `lc-engine`.
- **Script Surface:** full AUL coverage, proplist semantics, callbacks/effects ordering, devmode hooks, debugger/console.
- **Frontend & UI:** renderer, HUD, menus, cutscenes, text layout, input (mouse/keyboard/gamepad/touch), accessibility, editor dialogs.
- **Audio:** device backends, mixing, environmental effects, voice chat plumbing, per-channel controls.
- **Networking:** lobby stack, peer/server protocols, NAT traversal, desync detection/recovery, voting, recordings.
- **Tools & Editor:** scenario editor, particle editor, developer console, replay viewer, script debugger tied to the Rust runtime.
- **Distribution:** CI builds (Win/macOS/Linux), installers, auto-update, telemetry, mod folder layout.

## Porting Strategy
1. **Platform Bootstrap**
   - Status: `lc-game` (Rust) now owns path discovery, prepares user directories, migrates legacy configs into `LC_CONFIG_FILE`, captures runtime stdout/stderr into timestamped logs, launches the shipping C++ runtime so `cargo run -p lc-game` opens the Startup Menu with live input, validates the bundled `c4group` updater, syncs fresh `Clonk*.log` files into the Rust logs directory, records runtime exit status breakdowns, copies crash dumps into the Rust logs, mirrors updater telemetry into the launcher log, emits a structured `launcher-summary.json`, and produces `support-bundle-*.zip` archives with launcher/runtime logs, crash artifacts, and telemetry for support handoff; the new `lc-launcher` crate now feeds the graphical shell with summary/support bundle metadata, manual regeneration support, copy/share helpers, and upload artifact listings while the deprecated `lc-app` demo harness remains retired. `lc-launcher-ui` renders that shell state (summary, logs, telemetry) and wires regeneration, copy/share, and upload staging actions for the support bundle artifacts, and the `lc-launcher-shell` winit host now embeds that diagnostics screen with live regeneration/reveal handling. The diagnostics shell now uses OS-native pickers to stage copies/uploads, surfaces in-window success or error feedback for each action, automatically stages support bundles/artifacts into first-party share/upload drop targets, remembers the last picker destinations for fast follow-up transfers, surfaces first-party provider configuration/status directly in the diagnostics UI, emits submission-request payloads so staged bundles/artifacts drive automated provider workflows, and now records provider submission outcomes into `launcher-summary.json` so later regenerations and exported bundles retain automation history.
   - Next: hydrate persisted provider automation snapshots on launcher boot so the shell surfaces the last recorded submission outcomes without requiring a fresh staging action.
   - **Gate:** `cargo run -p lc-game` opens the shipping Startup Menu with live input routed through Rust scaffolding.

2. **Runtime Authority**
   - Expand `lc-engine` to drive scheduler ticks, object creation/destruction, particles, landscape, weather, pathfinding, and save/load without C++ intervention.
   - Mirror every AUL call/effect hook into Rust; maintain exhaustive replay fixtures for ordering and edge-case validation.
   - **Gate:** headless Rust loop reproduces the deterministic regression pack byte-for-byte against C++ recordings.

3. **Frontend & IO**
   - Implement rendering (software or wgpu/winit) to match C4 graphics, HUD, GUI widgets, and text output; unify font pipelines.
   - Port the full input stack and GUI/dialog system (startup, lobbies, editor, in-game menus); connect audio backends for music/effects parity.
   - **Gate:** Rust frontend renders the Startup Menu and in-game HUD, handles live input/audio, and no longer depends on C++ surfaces.

4. **Networking & Multiplayer**
   - Rebuild the `C4Network2*` stack in Rust (`lc-network`) covering lobby, synchronization, voting, desync recovery, net logging, and NAT traversal.
   - Stand up automated soak tests with mixed-platform clients to verify determinism and reconnection behavior.
   - **Gate:** two Rust clients connect to a Rust host, complete regression scenarios, and pass desync testers.

5. **Tools & Editor**
   - Port scenario/editor tooling, developer console, script debugger, replay viewer, and dev HUD to operate on the Rust runtime.
   - Ensure editor workflows (open/edit/save/play) round-trip with parity.
   - **Gate:** Rust binary launches the editor, edits a scenario, and plays it back identically.

6. **Release Integration**
   - Transition CI/packaging to Cargo-first builds, bundle resources, produce installers, and migrate auto-updater and telemetry.
   - Maintain dual shipping (C++ + Rust) until QA signs off; then retire CMake artifacts.
   - **Gate:** nightly Rust builds ship through the launcher with parity signatures and crash telemetry enabled.

## Validation
- Maintain shared deterministic replay suites, UI screenshot comparisons, and networking soak tests in CI.
- Track parity with a cross-functional feature checklist; no subsystem retires its C++ path until its acceptance gates stay green on every supported platform.
