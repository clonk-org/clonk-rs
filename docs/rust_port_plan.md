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
- Rust launcher now loads `Language*.txt` packs and renders every `lc-launcher-ui` string via the shared localization tables.

## Parity Gaps
- **Boot & Platform:** patcher/updater, localization, logging/crash handling, launcher integration; configuration migration now handled by `lc-game` (copies legacy configs and honours `LC_CONFIG_FILE`).
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
  - Status: `lc-game` handles config migration, per-run logging, crash/telemetry harvesting, and support bundle generation before delegating to the shipping C++ runtime. We now materialise `System.c4g`/`Graphics.c4g` into the repo root, the macOS bundle root, and `clonk.app/Contents/MacOS` so `cargo run -p lc-game` no longer dies on missing system groups; the launcher reaches SDL initialisation and forces windowed mode on headless hosts to keep CI boots stable.
  - Localization: `lc-launcher` loads language packs from `System.c4g` and the Rust UI uses them for all visible labels, prompts, and status messages.

2. **Runtime Authority**
  - Status: `lc-engine` drives the deterministic tick loop, object lifecycle, particles, landscape physics, and replay IO in Rust (`lc-engine/tests` keep regression fixtures honest). Landscape batches now cover per-column liquid placement/clearing so scripted fluid edits run entirely on the Rust side, the new `queued_commands` snapshot locks down spawn/destruction and particle parity, and weather drift (wind targets, time-of-day) now runs in `EnvironmentSettings` without touching `C4Weather`.
  - Expand `lc-engine` to drive scheduler ticks, pathfinding, and save/load without C++ intervention.
  - Mirror every AUL call/effect hook into Rust; maintain exhaustive replay fixtures for ordering and edge-case validation.
  - **Gate:** headless Rust loop reproduces the deterministic regression pack byte-for-byte against C++ recordings.

3. **Frontend & IO**
  - Progress: `lc-frontend` renders the HUD overlay, keeps the camera locked to the focus object, now drives crew command input through the shared `PlayerInputState`, the software renderer plots object silhouettes from engine vertex data, and GUI text flows through the shared bitmap font pipeline; next up is replacing remaining SDL/GDI surfaces.
  - Implement rendering (software or wgpu/winit) to match C4 graphics, HUD, GUI widgets, and text output; unify font pipelines. Software overlay now respects engine vertex meshes and Rust-side bitmap font layout; sprite atlas parity and lighting still pending.
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
