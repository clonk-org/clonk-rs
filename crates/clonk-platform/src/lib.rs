pub mod alloc_console;
mod console;
#[cfg(unix)]
pub mod crash;
pub mod crash_win32;
/// The C ABI the pinned oracle's `USE_RUST_PLATFORM_PATHS` bridge links
/// against (clonk-org/clonk-rs#1267). Off by default: it is a
/// differential-testing surface, and the crate types it needs are emitted by
/// `cargo xtask ffi` rather than declared in the manifest.
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod file_classes;
pub mod file_monitor;
mod paths;
pub mod privileges;
pub mod startup_dialog;
pub mod taskbar_progress;

pub use console::attach_parent_console;
pub use paths::{
    discover_unvalidated_install_root, AppPaths, PathsError, CONSOLE_CAPTION, ENGINE_CAPTION,
    PRODUCT_COMPACT_NAME, PRODUCT_NAME, PRODUCT_SLUG,
};
#[cfg(target_os = "macos")]
pub use paths::{
    establish_macos_bundle_working_directory, macos_bundle_working_directory,
    non_translocated_executable,
};
