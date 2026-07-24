# Contributing to Clonk Rust

Thank you for helping improve Clonk Rust. This project is a deterministic Rust
port of LegacyClonk, so matching the pinned C++ implementation takes priority
over adding new behavior.

## Before you start

Read [`CLAUDE.md`](CLAUDE.md) for the engineering rules and
[`PORT_STATUS.md`](PORT_STATUS.md) for known parity gaps. For substantial
changes, open an issue first so the proposed scope and parity evidence can be
agreed on before implementation.

Initialize the game-content submodule after cloning:

```sh
git submodule update --init --recursive
```

The checked-in `rust-toolchain.toml` selects the supported compiler and tools.
Platform dependencies and the fast local feedback loop are documented in
[`DEVELOPING.md`](DEVELOPING.md).

## Development process

- Treat the pinned LegacyClonk C++ checkout as the behavioral oracle. A Rust
  implementation may intentionally differ only when the C++ behavior is proven
  defective and the difference cannot affect lockstep determinism.
- Follow red-green-refactor. Add the smallest test that demonstrates the gap,
  observe it fail, implement the minimum parity fix, and rerun the relevant
  focused tests.
- Cite the corresponding C++ source file and line in tests for
  determinism-critical behavior.
- Keep structural changes separate from behavioral changes. State
  `[structural]` or `[behavioral]` in the commit subject.
- Do not hide parity gaps by weakening assertions, accepting broader behavior,
  or silently skipping tests. Record unresolved gaps in `PORT_STATUS.md`.

Run the complete required gate before requesting review:

```sh
cargo fmt --all -- --check
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked
cargo test --workspace --locked
cargo clippy --profile test --workspace --lib --bins --tests \
  --features xtask/engine-tools --locked -- -D warnings
cargo xtask engine-snapshots verify
cargo xtask parity verify
```

Behavioral changes can require additional scenario sweeps and live C++↔Rust
comparisons; follow the subsystem guidance in `PORT_STATUS.md`.

## Pull requests

Keep pull requests focused and include:

- the behavior or structural change and why it is needed;
- C++ oracle references and differential evidence for parity work;
- the test that was observed failing before the implementation;
- every validation command run, including any failure or skipped check;
- screenshots or recordings for visible changes; and
- attribution and license information for new assets or third-party code.

Never include credentials, private player data, private network captures, or
other secrets in an issue, commit, test fixture, or CI artifact. Report
security-sensitive findings as described in [`SECURITY.md`](SECURITY.md).

## Licensing and trademarks

Repository source is distributed under the ISC terms in
[`COPYING`](COPYING), except where a file states otherwise. Only contribute
material you have the right to submit under the repository's applicable terms,
and preserve existing copyright, license, and source attribution. Additional
content and dependency terms are retained under [`licenses/`](licenses/).

Clonk Rust is derived from LegacyClonk and Clonk. **“Clonk” is a registered
trademark of Matthes Bender.** The complete applicable notice is in
[`TRADEMARK`](TRADEMARK); do not remove or weaken it.
