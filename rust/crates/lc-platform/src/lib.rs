mod paths;

#[cfg(feature = "ffi")]
pub mod ffi;

pub use paths::{AppPaths, PathsError};
