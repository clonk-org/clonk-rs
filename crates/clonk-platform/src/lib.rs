pub mod alloc_console;
mod console;
#[cfg(unix)]
pub mod crash;
pub mod crash_win32;
mod paths;
pub mod privileges;
pub mod taskbar_progress;

pub use console::attach_parent_console;
#[cfg(target_os = "macos")]
pub use paths::{
    establish_macos_bundle_working_directory, macos_bundle_working_directory,
    non_translocated_executable,
};
pub use paths::{
    AppPaths, PathsError, CONSOLE_CAPTION, ENGINE_CAPTION, PRODUCT_COMPACT_NAME, PRODUCT_NAME,
    PRODUCT_SLUG,
};
