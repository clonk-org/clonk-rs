# StdConfig Integration Plan

## Immediate Tasks
- Wrap `std_config::Config` behind `extern "C"` functions returning opaque handles (`ConfigHandle`) with create/load/get/set/free operations.
- Introduce a feature flag (e.g. `rust_cfg`) so the C++ build can opt into the Rust implementation while maintaining the current path.
- Expose a comparison helper that accepts the legacy dump and returns newline-delimited differences so debug builds can surface mismatches quickly.
- Use the helper to guard overwriting of C++ config structures; once parity is confirmed, populate General/Graphics/Sound/Network fields directly from the Rust handle before consumers read them. Call `C4Config::SyncRust()` after legacy-only mutations so the Rust handle stays current between saves.

## Staged Rollout
1. **Read-only bridge**: Modify `C4Config` to call into the Rust loader when the feature flag is enabled, but continue writing via the existing C++ mechanism.
2. **Validation**: On boot, load config via both Rust and C++ paths in debug builds and diff the resulting key/value pairs, logging discrepancies for investigation.
3. **Writer swap**: After parity is confirmed, expose a Rust `save` FFI and switch the default writer. Keep the C++ writer accessible via an environment toggle for rollback.
4. **Removal**: Once CI and release builds rely solely on the Rust path, delete the legacy `StdConfig` code and clean up unused headers.

## Supporting Work
- Extend tests to read representative `.cfg` files from `planet/` (including commented sections) and compare against known expected maps.
- Provide utility to convert Rust `Config` entries into `StdBuf`/`StdStrBuf` for compatibility with existing logging and diagnostics.
- Use `Config::set_section_commented` to explicitly mark sections that should round-trip as commented headers (e.g. `#[Audio]`). This keeps serialization aligned with the legacy writer when we migrate C++ callers.
- Ship `rust/include/lc_config_ffi.h` alongside the compiled static/cdylib so C++ can include the declarations without duplicating them.

## Build Integration Notes
- Add `rust/include` to the include path when `USE_RUST_CONFIG` is enabled.
- Link against `lc_core` static or dynamic library produced by Cargo (`cargo build --release --lib`).
- Define `USE_RUST_CONFIG` (e.g., via `target_compile_definitions`) for the C++ targets that should use the bridge.
- Call `RustConfigBridge::LoadConfig` early in initialisation and `Unload` during shutdown to release the handle.
- Create a helper module (e.g. `src/rust/RustConfigBridge.(h|cpp)`) that wraps the FFI in RAII-friendly C++ interfaces; compile it conditionally when the flag is enabled.
- Ensure the consuming target in CMake adds `RustConfigBridge.cpp`, depends on the `rust_build` custom target, includes `${RUST_INCLUDE_DIR}`, and links against the desired `lc_core` artefact.

## Rust integration notes
# - Build Rust with `cargo build --release --lib` when enabling USE_RUST_CONFIG.
# - Set lc_core linkage accordingly for your platform (staticlib/cdylib).
