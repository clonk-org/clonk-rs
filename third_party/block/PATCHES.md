# Local patches

This directory contains the published `block` 0.1.6 source. It is patched
locally because 0.1.6 is still the latest release and its upstream source
triggers Rust's `uninhabited_static` future-compatibility lint.

Local changes:

- model the opaque `_NSConcreteStackBlock` class as an inhabited zero-sized
  C-compatible struct;
- spell the crate's existing default C ABI explicitly;
- declare a nested workspace boundary so this vendored crate can be tested
  independently; and
- resolve the published package's test helper from crates.io rather than the
  repository-relative development path.

Upstream tracking issue: <https://github.com/SSheldon/rust-block/issues/21>
