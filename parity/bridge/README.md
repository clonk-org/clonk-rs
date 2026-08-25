# Shadow-diff bridge ABI

`lc_engine_ffi.h` is the C ABI the pinned oracle's `USE_RUST_ENGINE_VALIDATION`
bridge (`src/rust/RustEngineBridge.cpp`) calls, copied verbatim from the oracle
commit `7d43b47b7d789b533f32d005e64596e0a07019cd`
(`rust/include/lc_engine_ffi.h`). It is vendored here so the contract is
versioned next to the implementation instead of living only in a C++ checkout —
`crates/clonk-engine/src/ffi.rs` (feature `ffi`) is what satisfies it.

This is Phase 2 of the parity harness, clonk-org/clonk-rs#585.

## Building the artifacts

```sh
cargo xtask ffi --profile debug     # or --release
```

That emits `target/<profile>/libclonk_engine.a` and the matching dynamic
library — the exact paths `CMakeLists.txt:97-107` imports and
`:404-406` links for `USE_RUST_ENGINE_VALIDATION`. The oracle runs the same
command itself (`CMakeLists.txt:138-143`).

The crate types are emitted by that command rather than declared in
`clonk-engine`'s manifest, as the pinned tree did it: a `crate-type` entry
would make every ordinary build pay the staticlib archive and the cdylib link.

## What is and is not wired

The Rust side of the ABI is present, compiles, and exports all 32
`lc_engine_*` symbols the header declares. The loop is **not** wired yet:

- Nothing rebuilds the oracle with `-DUSE_RUST_ENGINE_VALIDATION` against this
  tree, so no gate exercises it and none of the four scenario classes #585 asks
  for run. That rebuild is the step that actually validates the ported
  comparison semantics.
- The other bridges (`USE_RUST_CONFIG`, `USE_RUST_GROUP_VALIDATION`,
  `USE_RUST_GUI_VALIDATION`, `USE_RUST_PLATFORM_PATHS`) link their own
  `lc_*` libraries and need the `ffi` modules of the other crates, which are
  not restored. They are off by default, so engine validation does not wait on
  them.

Until both exist, `parity verify` remains the primitive-section differential and
this ABI is inert. Do not read its presence as evidence of full-scenario parity;
`../README.md` is explicit that the historical report used the Rust snapshot
bundled at the oracle commit, not the current tree.

## Faithfulness

`ffi.rs` is a port-forward of the pinned implementation rather than a rewrite,
deliberately: `OnFrame` hands the Rust side a complete `SnapshotBuffer`
collected from `C4Game`, and the Rust side decides what counts as a divergence,
so the comparison semantics are only correct if they match the ones the bridge
was written against. Reconciling drift against the current engine took two
changes:

- `ObjectSnapshot::components` is a `ComponentList` now (`C4IDList` ordering)
  rather than a `HashMap`.
- `tracing-subscriber` is an optional dependency behind the `ffi` feature; the
  pinned tree took it the same way.

Keep it that way. A divergence introduced here is indistinguishable from an
engine divergence when the loop finally runs.
