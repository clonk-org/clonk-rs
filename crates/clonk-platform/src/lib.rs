mod console;
#[cfg(unix)]
pub mod crash;
mod paths;

pub use console::attach_parent_console;
#[cfg(target_os = "macos")]
pub use paths::{
    establish_macos_bundle_working_directory, macos_bundle_working_directory,
    non_translocated_executable,
};
pub use paths::{AppPaths, PathsError, PRODUCT_COMPACT_NAME, PRODUCT_NAME, PRODUCT_SLUG};
