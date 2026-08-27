//! The `ImageData` menu-image adapters, under the path the menu chrome has
//! always used.
//!
//! The generic primitives beneath them belong to
//! [`clonk_graphics::compositing`] and are reached from there directly; only
//! the adapters are re-exported here, and only the ones this crate offers.

pub use clonk_app_core::menu_images::{
    copy_menu_image, copy_menu_image_aspect, software_blit_menu_image,
};
