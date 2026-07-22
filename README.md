# clonk-rs

A Rust port of the [LegacyClonk](https://github.com/legacyclonk/LegacyClonk)
engine, developed for bit-for-bit lockstep parity with the original C++
implementation.

The engine, frontend, and tooling form a Cargo workspace of `clonk-*`
packages at the repository root;  `clonk-app` is the game binary. Game data
lives in [`planet/`](planet/) and the [`content/`](content/) submodule.

## Building

```
cargo build --workspace
cargo nextest run --workspace
```

See [`CLAUDE.md`](CLAUDE.md) for engineering constraints,
[`PORT_STATUS.md`](PORT_STATUS.md) for parity status, and
[`REFACTOR_PLAN.md`](REFACTOR_PLAN.md) for the ongoing
decomposition campaign.

## The C++ oracle

The original C++ engine is no longer vendored in this repository. Parity
work reads and runs it from a separate pinned checkout:

- Clone: `~/Documents/code/vendor/legacyclonk-oracle`
  (upstream `legacyclonk/LegacyClonk`; tag `oracle-src-pinned` marks the
  exact source state this repository's parity citations refer to)
- The arm64 capture harness (`build-arm64-native/`, including the
  runnable `clonk.app`) lives in that checkout.

## Licensing

This port is a derived work of LegacyClonk and Clonk. The original license
and trademark terms in [`COPYING`](COPYING), [`TRADEMARK`](TRADEMARK), and
[`licenses/`](licenses/) continue to apply; see [`credits.txt`](credits.txt)
for attribution. Rust dependency licenses are declared in each crate's
metadata (`cargo license`/`cargo about` can enumerate them).
