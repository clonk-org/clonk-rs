# Next Steps: Porting StdMarkup / StdConfig

## Overview
With `StdBuf`, `StdStrBuf`, and `StdFile` now available in Rust, the next logical porting target is the markup/configuration layer that ingests text files into structured representations. The C++ implementation focuses on:

- Tokenizing simple key/value pairs and nested sections (`StdMarkup.h`, `StdConfig.h`).
- Handling comments and escape sequences, relying heavily on `StdStrBuf` utilities.
- Building typed accessors that feed into engine initialisation and scenario parsing.

## Proposed Rust Modules
- `std_markup`: Parser utilities around newline-delimited key/value syntax, whitespace handling, and comment stripping.
- `std_config`: Structured wrapper offering typed getters/setters and file IO glue.

## Dependencies & Interfaces
- Depends on new Rust `std_file` for line iteration and path handling.
- Reuses `std_buf::StdStrBuf` for string operations and escaping.
- Should expose idiomatic iterators over sections to allow incremental migration of callers (e.g., `C4Config`, scenario loaders).

## Migration Strategy
1. Translate `StdMarkup::Read/Write` logic into Rust iterators, backed by tests using existing config fixtures.
2. Implement the `StdConfig` wrapper with BTreeMap-based storage, keeping but constraining string interning to preserve ordering where required.
3. Offer FFI shims so remaining C++ code can call the Rust parser during staged migration.
4. Update config-consuming subsystems incrementally, beginning with command-line/config bootstrapping, before moving to scenario and player configuration files.

