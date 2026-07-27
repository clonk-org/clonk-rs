# Clonk Rust

Clonk Rust is a Rust port of the
[LegacyClonk](https://github.com/legacyclonk/LegacyClonk) engine, developed
for bit-for-bit lockstep parity with the original C++ implementation.

The engine, frontend, and tooling form a Cargo workspace of `clonk-*`
packages at the repository root;  `clonk-app` is the game binary. Game data
lives in [`planet/`](planet/) and the [`content/`](content/) submodule.

## Building and running

Clone with the game-content submodule, then build and test from the repository
root:

```sh
git clone --recurse-submodules https://github.com/syb0rg/clonk-rs.git
cd clonk-rs
cargo build --workspace
cargo test --workspace --locked
cargo run --release -p clonk-app --bin clonk-app
```

For an existing clone, initialize or repair the content checkout with
`git submodule update --init --recursive` before building. GitHub's generated
“Source code” archives do not populate Git submodules and therefore do not
contain a runnable content tree; use a recursive Git clone for development or
the project release ZIP for a runnable distribution.

## Packaging

Initialize the game-content submodule, then build the distributable archive:

```sh
git submodule update --init --recursive
cargo xtask package
```

By default, the command writes
`target/dist/clonk-rust-<version>-<target-triple>.zip` (or the equivalent path
beneath `CARGO_TARGET_DIR`). The archive contains the `clonk-game` launcher,
the `clonk-app` runtime, the pinned content submodule — base packs plus the
authorized Eke Reloaded and ClonkMars packs — credits, and the project and
content notices. The legacy
`c4group` update utility is optional: packaged builds run without it, while a
copy installed alongside the game is still probed and a broken executable is
reported before startup.

Tracker-music playback loads the optional libxmp 4 runtime at run time. Install
libxmp through the platform package manager, or set `LC_LIBXMP_LIBRARY` to a
compatible library; the game otherwise remains runnable without tracker music.

## Releasing

```sh
scripts/prepare-release.sh
```

The next version comes from the Conventional Commit subjects since the last
`v*` tag. There is one version for the whole workspace — every crate inherits
`version.workspace` and nothing is published to a registry — so releases are a
single tag, not one per crate.

The script bumps that version, refreshes `Cargo.lock` and prepends a
[`CHANGELOG.md`](CHANGELOG.md) section. It stops there; review the result, run
the gates, then commit and tag. Pushing the release commit to `main` builds and
publishes the assets automatically — see below.

See [`AGENTS.md`](AGENTS.md) for engineering constraints,
[`PORT_STATUS.md`](PORT_STATUS.md) for parity status, and
[`docs/REFACTOR_PLAN.md`](docs/REFACTOR_PLAN.md) for the ongoing
decomposition campaign.

## The C++ oracle

The original C++ engine is no longer present in the working tree. Its exact
pinned source snapshot, commit
`7d43b47b7d789b533f32d005e64596e0a07019cd`, remains reachable in this
repository's Git history. `cargo xtask parity record` regenerates the golden
oracle directly from that snapshot; a shallow clone must fetch the missing
history first.

For live differential or capture work, set `LEGACYCLONK_ORACLE_ROOT` to a
separate C++ checkout. `LEGACYCLONK_ORACLE_REVISION` optionally selects a
different revision. See [`parity/README.md`](parity/README.md) for details.

## Licensing

The Rust source packages in this Cargo workspace are available under the ISC
license in [`COPYING`](COPYING), as declared by their Cargo metadata. That
declaration does **not** relicense the bundled game data: graphics, audio,
scripts, text, and other assets under [`planet/`](planet/) and the
[`content/`](content/) submodule remain under their own `COPYING` files,
including the Clonk content license carried by the content submodule.

The Eke Reloaded and ClonkMars packs inside the content submodule are
redistributed under the specific permission and attribution terms recorded in
[`THIRD_PARTY_GAME_CONTENT.md`](THIRD_PARTY_GAME_CONTENT.md), not under the
source or general content licenses.

Third-party Rust dependencies retain their own licenses, as declared by their
Cargo metadata.
