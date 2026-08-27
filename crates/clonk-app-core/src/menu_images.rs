//! `ImageData` adapters over the software compositing primitives.
//!
//! The primitives themselves — aspect fitting, nearest-neighbour stretching and
//! the exact `BltAlpha`/`BltAlphaAdd` layer composition — belong to
//! [`clonk_graphics::compositing`], which knows nothing about menus or image
//! assets. What is left here is the one thing that does: turning a GUI
//! `ImageData` into a `Surface` the primitives can read. That boundary is why
//! `clonk-graphics` needs no dependency on `clonk-gui`.

use clonk_graphics::compositing::{aspect_fit_rect, blit_stretched, copy_stretched};
use clonk_graphics::{BlitMode, PixelFormat, Rect, Surface};
use clonk_gui::ImageData;

/// A `Surface` view of one image's pixels.
fn image_surface(image: &ImageData) -> Option<Surface> {
    Surface::from_bytes(
        image.width(),
        image.height(),
        PixelFormat::Rgba8888,
        image.pixels().to_vec(),
    )
    .ok()
}

pub fn copy_menu_image(surface: &mut Surface, image: &ImageData, destination: Rect) -> Option<()> {
    let source = image_surface(image)?;
    copy_stretched(
        &source,
        Rect::new(0, 0, image.width(), image.height()),
        surface,
        destination,
    )
}

pub fn copy_menu_image_aspect(
    surface: &mut Surface,
    image: &ImageData,
    destination: Rect,
) -> Option<()> {
    let fitted = aspect_fit_rect(image.width(), image.height(), destination)?;
    copy_menu_image(surface, image, fitted)
}

pub fn software_blit_menu_image(
    destination: &mut Surface,
    image: &ImageData,
    destination_rect: Rect,
    mode: BlitMode,
) -> Option<()> {
    let source = image_surface(image)?;
    let source_rect = Rect::new(0, 0, image.width(), image.height());
    blit_stretched(destination, &source, source_rect, destination_rect, mode)
}
