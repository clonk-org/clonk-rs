pub mod chrono_util;
/// The C ABI the pinned oracle's `USE_RUST_CONFIG` bridge links against
/// (clonk-org/clonk-rs#1264). Off by default: it is a differential-testing
/// surface, and the crate types it needs are emitted by `cargo xtask ffi`
/// rather than declared in the manifest.
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod legacy_text;
pub mod log_target;
pub mod std_config;
pub mod std_file;
pub mod std_markup;
pub mod version;
