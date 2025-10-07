# LegacyClonk Rust Port Evaluation

## Summary
- The Rust workspace delivers a standalone demo stack (`lc-app`) and validation harnesses, but it is **not** a drop-in replacement for the shipping C++ runtime.
- Rust crates currently mirror only narrow slices of the engine, script VM, GUI, audio, graphics, and networking systems to support deterministic tests and exploratory tooling.
- LegacyClonk still compiles and runs its gameplay, UI, rendering, audio, and networking through the original C++ code paths; Rust code is wired in behind optional validation flags.
- The Rust engine now applies configurable gravity and vertical speed caps each tick, and both demo configs and scenario manifests can override those physics defaults.
- A JSON-backed scenario loader now registers definitions, landscapes, and initial spawns so `lc-app` can bootstrap from external scenario bundles.
- `lc-app` now replays scripted control packets and uses `lc-engine::apply_object_update` to steer the demo object, and an interactive terminal mode captures live keyboard input for the demo bouncer.

## Rust Workspace Scope
- `lc-core`, `lc-resources`, `lc-script`, `lc-engine`, `lc-graphics`, `lc-audio`, `lc-network`, `lc-gui`, `lc-platform`, and `lc-app` provide demo-friendly utilities, parsers, in-memory surfaces, a toy physics loop, and basic networking abstractions.
- `lc-app` drives a self-contained bounce simulation with a bundled AUL script and synthetic audio/graphics output. It can optionally load JSON scenarios and now replays a deterministic control stream, but there is still no viewport management or real player input handling.
- `rust/include` exposes FFI headers so the C++ codebase can record/play back engine snapshots or cross-check groups/GUI layouts when the matching `USE_RUST_*` option is enabled.

## Integration with the C++ Build
- `CMakeLists.txt` keeps all Rust bridges behind opt-in switches (`USE_RUST_CONFIG`, `USE_RUST_GROUP_VALIDATION`, `USE_RUST_ENGINE_VALIDATION`, `USE_RUST_GUI_VALIDATION`). These default to `OFF` and only add validators plus static libraries when explicitly enabled.
- The validation bridges (`src/rust/RustConfigBridge.cpp`, `RustGroupBridge.cpp`, `RustEngineBridge.cpp`, `RustGuiBridge.cpp`) consume Rust FFI helpers to compare C++ output with Rust expectations or to dump JSON recordings. They do not replace runtime logic.
- No executable target links Rust crates as authoritative gameplay systems; the production binary continues to depend on the C++ implementations.

## Component Parity Assessment
- **Simulation / Object System:** `lc-engine` models position/velocity/energy for scripted objects, applies configurable gravity and vertical speed caps each tick, and clamps control updates through that physics layer. A basic JSON scenario loader can register definitions, initial objects, landscapes, and now optional physics overrides, and the `apply_object_update` hook lets scripted or live terminal control streams adjust demo objects. The C++ object status machine, action procs, vertices, effects, command queues, crew ownership, and full scenario/environment management are still absent.
- **Script VM:** `lc-script` parses and executes a subset of AUL with arithmetic, control flow, arrays, and proplists. Engine-call bindings, callback dispatch tables, effect lifecycles, and synchronization with the C++ object model are absent.
- **Graphics:** `lc-graphics` works on CPU-resident RGBA surfaces and hash snapshots. It lacks texture streaming, OpenGL/WGL/SDL integration, blitting catalogs, shader management, viewport compositing, and render thread orchestration present in `StdGL*`.
- **Audio:** `lc-audio` offers a software mixer with optional CPAL output for limited channel playback. The SDL_mixer-based backend, streaming music, positional audio, resampler choices, and sound bank handling from `C4AudioSystem` are not ported.
- **GUI:** `lc-gui` implements a small immediate-mode layout system plus a scenario browser widget. It does not cover the extensive control hierarchy, themes, dialogs, game menus, or input focus logic provided by `C4GUI`.
- **Networking:** `lc-network` covers control packet ordering, a thin lobby model, and decoding of a handful of TCP frame types. Full peer discovery, host migration, HTTP reference handling, UPnP, league integration, and flexible transport backends from `C4Network2` remain in C++.
- **Resources & IO:** `lc-resources` can open groups and enumerate files, useful for validation. Features such as incremental writes, child group mutation, symbol lookup, and tight integration with scenario loading are missing.
- **Core utilities:** `lc-core` includes Rust mirrors for some `Std*` helpers (`StdBuf`, `StdConfig`, scheduler, sync primitives). Many legacy-specific behaviors (platform message loops, window management, legacy threading quirks) stay in the original code.
- **Platform bootstrap:** `lc-platform` only resolves application directories; it does not replace `StdApp*` entry points or SDL/winit startup glue.

## Current Usage Patterns
- Engine parity is evaluated by recording C++ snapshots and comparing them to Rust playback baselines. Failing comparisons raise warnings but do not stop the game.
- Group and GUI validators serve as optional smoke tests during development. They are not enabled in release builds and do not gate asset loading.
- `lc-app` can replay a scripted control sequence or accept live terminal input to drive the demo object through the control hook.

## Major Gaps to Reach Behavior Parity
- Recreate the complete C4 object lifecycle, including action system, physics integration beyond the new baseline gravity/velocity caps, robust scenario/environment management beyond the current JSON loader, real player control pathways beyond the scripted demo hook, and serialization.
- Port AUL runtime features: effect handlers, engine call map, proplist semantics, debugging, and compatibility behaviors relied upon by shipped scripts.
- Implement rendering and audio backends that match the SDL/OpenGL pipeline and mixer behavior across all supported platforms.
- Mirror GUI subsystems (dialogs, HUD, console, editor) and integrate them with input, networking, and engine state.
- Rebuild multiplayer networking (TCP/UDP transports, reference server, lobby flow, download management, UPnP) with deterministic equivalence.
- Provide platform entry points, resource loaders, and tooling parity so the Rust workspace can launch real scenarios instead of the demo harness.

## Recommendations
- Treat the Rust code as validation/helper infrastructure until feature-complete replacements exist for each subsystem.
- Prioritize a detailed migration plan per subsystem with measurable parity tests (engine recordings, script smoke tests, rendering output comparisons, network session captures).
- Expand documentation to track ownership, required features, and blockers for each domain before attempting to switch production builds to Rust.
