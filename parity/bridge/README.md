# Shadow-diff bridge ABI

`lc_engine_ffi.h` is the C ABI the pinned oracle's `USE_RUST_ENGINE_VALIDATION`
bridge (`src/rust/RustEngineBridge.cpp`) calls, copied verbatim from the oracle
commit `7d43b47b7d789b533f32d005e64596e0a07019cd`
(`rust/include/lc_engine_ffi.h`). It is vendored here so the contract is
versioned next to the implementation instead of living only in a C++ checkout —
`crates/clonk-engine/src/ffi.rs` (feature `ffi`) is what satisfies it.

This is Phase 2 of the parity harness, clonk-org/clonk-rs#585.

## What is and is not wired

The Rust side of the ABI is present and compiles. The loop is **not** wired yet:

- The oracle links `rust/target/<profile>/liblc_core.a`
  (`CMakeLists.txt:71-76`), an aggregating `staticlib`/`cdylib` target that
  re-exports every `ffi` module — engine, audio, config, gui, platform,
  resources, script. That target was never committed at the pin either; it was
  part of the "diagnostic-only bridge/ABI patch" `../README.md` credits for the
  historical Gold Rush run. Only `clonk-engine`'s surface is restored here.
- Nothing rebuilds the oracle with `-DUSE_RUST_ENGINE_VALIDATION` against this
  tree, so no gate exercises it.

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
