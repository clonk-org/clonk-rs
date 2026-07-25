mod console;
mod paths;

pub use console::attach_parent_console;
pub use paths::{AppPaths, PathsError, PRODUCT_COMPACT_NAME, PRODUCT_NAME, PRODUCT_SLUG};
