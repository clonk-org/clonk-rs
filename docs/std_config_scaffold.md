# StdConfig Migration Scaffold

## Goal
Provide a Rust-native configuration reader/writer that mirrors the behaviour of `StdConfig` and friends. It should parse key/value pairs, preserve ordering, and integrate with `std_markup` for comment/markup handling.

## Proposed Components
- `std_config::Entry` – holds key, value, and optional metadata (comments, source line).
- `std_config::Config` – map-like structure backed by `indexmap::IndexMap` to preserve iteration order.
- `std_config::parser` – line-oriented loader using `std_file::read_file_line` and `Markup::strip_markup` to sanitise inputs.
- `std_config::writer` – serialises entries back to the canonical `.cfg` format.

## Next Implementation Steps
1. Define `Config` struct with typed accessors (`get_bool`, `get_int`, `get_string`) returning `Option` plus convenience defaults.
2. Implement parser using the new `std_markup::Markup::strip_markup` to remove inline colour codes before tokenisation.
3. Add round-trip tests using existing configuration fixtures (e.g. `planet/Objects.ocd/Definitions.txt`).
4. Provide feature-flagged C++ FFI shim so legacy components can transition incrementally.

## Integration Strategy
- Expose the Rust config reader behind a C ABI (`extern "C"`) returning owned buffers so existing C++ `StdConfig` wrappers can gradually delegate.
- Start with read-only adoption inside `C4Config` compile functions, keeping the old writer until parity is confirmed.
- Once all call sites use the Rust reader, flip the default to Rust while leaving the C++ parser available under a compile flag for fallback.
