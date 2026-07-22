//! Software menu-image compositing shared by the in-game menu chrome and the
//! app's picture caches. The implementations moved to `clonk_app_core` with
//! the picture pipeline; this module re-exports them under their old paths.

pub use clonk_app_core::menu_images::{
    composite_software_picture_layer, copy_menu_image, copy_menu_image_aspect,
    copy_stretched_picture, menu_aspect_fit_rect, software_blit_menu_image,
};
