## Summary

<!-- What changed, and why? -->

## Parity evidence

<!--
For behavioral changes, cite the pinned C++ oracle file:line and describe the
differential evidence. For structural-only changes, write "Not applicable."
-->

## Test evidence

<!-- Name the test observed RED before implementation and GREEN afterward. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `python3 -m unittest discover --buffer -s scripts/tests -p 'test_*.py'`
- [ ] `cargo test -p xtask --features engine-tools --bin xtask-engine-tools --locked`
- [ ] `cargo nextest run --workspace --no-fail-fast`
- [ ] `cargo clippy --profile test --workspace --lib --bins --tests --features xtask/engine-tools --locked -- -D warnings`
- [ ] `cargo xtask engine-snapshots verify`
- [ ] `cargo xtask parity verify`
- [ ] `cargo xtask compat verify`
- [ ] Additional scenario/parity checks, or an explanation of why none apply

## Change classification

- [ ] Structural only
- [ ] Behavioral
- [ ] Structural and behavioral work is split into separate commits

## Public-repository checks

- [ ] No secrets, private player data, or private network captures are included
- [ ] New third-party code and assets include applicable license and attribution
- [ ] Existing copyright and trademark notices remain intact
