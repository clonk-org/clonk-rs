# LegacyClonk Rust Port Plan

## 1. Existing Architecture Snapshot
- **Core runtime:** `Std*` utility classes, file I/O, threading (`StdApp`, `StdBuf`, `StdThread*`, etc.) under `src/Std*.{h,cpp}` and `src/C4*` foundation headers.
- **Game engine:** Simulation, scenario management, object system (`C4Game*`, `C4Object*`, `C4Def*`, `C4Script*`).
- **Scripting VM:** AUL parser/executor (`C4Aul*`) with bytecode interpreter and debugging support.
- **Graphics stack:** OpenGL/SDL surfaces, texture handling, and GUI (`C4GraphicsSystem`, `StdGL*`, `C4Gui*`).
- **Audio:** SDL2_mixer abstraction (`C4AudioSystem*`).
- **Networking:** P2P control + lobby (`C4Network*`, `C4GameControlNetwork`, miniupnpc integration).
- **Platform glue:** `StdAppWin32.cpp`, `StdAppUnix.cpp`, `StdSdlSubSystem.cpp`, macOS Objective-C++ shim.
- **Tooling/tests:** Catch2 harness in `tests/`, build coordinated by CMake including third-party deps (SDL2, OpenGL, CURL, fmt, Freetype, Iconv, JPEG/PNG, OpenSSL, miniupnpc, X11, Zlib).

## 2. Target Rust Workspace Structure
Use a Cargo workspace with layered crates to isolate concerns and allow incremental porting/testing.

```
legacyclonk-rs/
├── Cargo.toml (workspace)
├── crates/
│   ├── lc-core/          # memory management, math, platform-agnostic utilities (Std* replacements)
│   ├── lc-resources/     # group/file handling, compression, virtual filesystem
│   ├── lc-script/        # AUL parser, bytecode, debugging hooks
│   ├── lc-engine/        # gameplay, simulation loop, objects/definitions
│   ├── lc-graphics/      # rendering, facet/surface abstractions
│   ├── lc-audio/         # audio backend abstraction (cpal/rodio wrappers)
│   ├── lc-network/       # control stream, lobby, HTTP, UPnP
│   ├── lc-gui/           # in-game editor, dialogs, toast notifications
│   ├── lc-platform/      # platform-specific bootstrap (winit/sdl glue, fs helpers)
│   └── lc-app/           # final binary; parity with `LegacyClonk` launcher
└── tools/
    └── migration-scripts # optional transpilation helpers/tests
```

Each crate exposes FFI-safe boundaries to ease testing and permit staged migration from the C++ runner if needed.

## 3. Mapping Highlights
- `StdBuf`, `StdStrBuf`, `StdFile`, `StdSync`, `StdScheduler` → `lc-core` (leverage `Vec<u8>`, `String`, `std::sync` while preserving semantics like reference/view vs owned buffers using enum wrappers).
- `C4Group`, `C4Folder`, `CStdFile` → `lc-resources` (use `flate2`, `zip`, `tar` crates as replacements for custom compression logic, ensure deterministic ordering).
- `C4Aul*` → `lc-script` (Rust parser combinators + bytecode VM using enums/structs; consider `logos`/`lalrpop` or custom parser to replicate grammar).
- `C4Game*`, `C4Object`, `C4Def` → `lc-engine` (entity management with ECS-like data structures; maintain deterministic fixed-step update loop identical to original).
- `StdGL*`, `C4Graphics*`, `C4Surface*` → `lc-graphics` (target `wgpu`/`glow` depending on parity requirements; provide compatibility layer for original fixed pipeline expectations).
- `C4AudioSystem*` → `lc-audio` (wrap SDL2_mixer parity via `sdl2` crate initially, then abstract to cross-platform backend).
- `C4Network*`, `C4GameControlNetwork`, `C4HTTPClient` → `lc-network` (use `tokio`/`async-std` for async networking but keep deterministic message ordering; replicate miniupnpc using `igd` crate).
- `C4Gui*`, `C4Startup*`, dialogs → `lc-gui` (port to Rust immediate-mode GUI using `egui`/custom; ensure identical layout by mirroring original widget tree data).
- Platform entry points (`C4Application`, `StdApp*`, `C4WinMain`, `StdGtkWindow`, `StdSDLWindow`) → `lc-platform` and `lc-app` crates, using `winit` + conditionally compiled modules per OS.

## 4. Porting Strategy
1. **Foundation pass** – replicate low-level utilities (`lc-core`) with comprehensive unit tests derived from existing C++ behavior (use golden tests where available).
   - [x] `StdBuf`, `StdStrBuf`, `StdFile`, `StdConfig`, `StdMarkup` implemented and under test in `lc-core`.
   - [x] `StdSync` ported via `lc-core::std_sync` (reentrant critical sections, manual/auto reset events, shared lock + callback) with concurrency-focused tests.
   - [x] `StdScheduler` implemented with poll-based fd/event handling and cooperative thread runner; Windows path currently uses condvar wake-ups pending native event integration.
2. **Resource & IO layer** – port `C4Group` stack to ensure game data loads. Provide FFI layer to call Rust from existing C++ for verification during transition.
   - [x] `lc-resources::Group` handles directory and packed groups, including header unscrambling, with regression tests covering synthetic archives.
   - [x] Exposed `lc_group_*` C ABI for opening groups, enumerating entries, and reading files so legacy C++ can validate Rust outputs.
   - [x] Tied the Rust `lc_group_*` FFI into the legacy loader via the optional `USE_RUST_GROUP_VALIDATION` flag, running metadata parity checks against every on-disk group open.
3. **Scripting VM** – port AUL parser/executor; validate against existing script test suite and game scenarios.
4. **Engine loop & objects** – migrate game simulation in slices (definitions, object control, landscape handling), comparing frame-by-frame outputs against the C++ build using deterministic seeds.
5. **Rendering & audio** – implement wrapper surfaces and audio mixers; rely on original assets to cross-check rendering results (image snapshots via integration tests).
6. **Networking** – port control streams and lobby after engine parity to avoid divergence; use integration tests between Rust/C++ builds to confirm protocol compatibility.
7. **UI/editor** – migrate GUI components last once core engine validated.
8. **Final integration** – remove C++ harness, produce final Rust binary, update tooling/tests, ensure packaging scripts replaced (cargo xtask).

## 5. Testing & Validation
- Mirror existing Catch2 tests with `cargo test`; translate fixtures.
- Build deterministic comparison harness to record authoritative outputs from C++ build and check Rust port (`snapshots/` stored per version).
- Continuous integration: use `cargo fmt`, `cargo clippy`, OS-specific integration tests via GitHub Actions/CI equivalent.
- Performance regressions tracked via criterion benchmarks for hotspots (script execution, landscape updates).

## 6. Tooling & Automation Ideas
- Source-to-source assistance: write `clang`-based extractor to emit annotated AST -> Rust skeletons for manual filling.
- Create bridging layer (`cxx` crate) for temporary calls into remaining C++ when performing incremental migration to maintain runnable builds throughout port.
- Leverage `bindgen` to wrap remaining C++ until port complete, enabling stepwise replacement.

## 7. Risks / Open Questions
- Precise reproduction of undefined behaviors relied upon in legacy C++ (must audit and encode in tests).
- Rendering parity with legacy fixed pipeline OpenGL when moving to modern Rust graphics stack.
- Multithreading semantics of `StdScheduler`/`StdThreadPool` vs Rust's safety guarantees—requires careful mapping and possibly unsafe code.
- Script engine intricacies, including debugger hooks and platform-specific quirks.
- Large surface area of UI/editor code (GTK/Win32/SDL) needing new cross-platform approach.
