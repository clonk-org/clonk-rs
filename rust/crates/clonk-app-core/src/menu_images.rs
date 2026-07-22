//! Software menu-image compositing shared by the in-game menu chrome and the
//! app's picture caches: aspect-fitted copies and the exact
//! `BltAlpha`/`BltAlphaAdd` layer composition.

use clonk_graphics::{BlitMode, Color, PixelFormat, Rect, Surface};
use clonk_gui::ImageData;

pub fn copy_menu_image(surface: &mut Surface, image: &ImageData, destination: Rect) -> Option<()> {
    let source = Surface::from_bytes(
        image.width(),
        image.height(),
        PixelFormat::Rgba8888,
        image.pixels().to_vec(),
    )
    .ok()?;
    copy_stretched_picture(
        &source,
        Rect::new(0, 0, image.width(), image.height()),
        surface,
        destination,
    )
}

pub fn menu_aspect_fit_rect(
    source_width: u32,
    source_height: u32,
    destination: Rect,
) -> Option<Rect> {
    if source_width == 0 || source_height == 0 {
        return None;
    }
    let mut fitted = destination;
    let width_ratio = 100_u64 * u64::from(destination.width) / u64::from(source_width);
    let height_ratio = 100_u64 * u64::from(destination.height) / u64::from(source_height);
    if width_ratio < height_ratio {
        fitted.height = source_height.saturating_mul(destination.width) / source_width;
        fitted.y += destination.height.saturating_sub(fitted.height) as i32 / 2;
    } else if height_ratio < width_ratio {
        fitted.width = source_width.saturating_mul(destination.height) / source_height;
        fitted.x += destination.width.saturating_sub(fitted.width) as i32 / 2;
    }
    Some(fitted)
}

pub fn copy_menu_image_aspect(
    surface: &mut Surface,
    image: &ImageData,
    destination: Rect,
) -> Option<()> {
    let fitted = menu_aspect_fit_rect(image.width(), image.height(), destination)?;
    copy_menu_image(surface, image, fitted)
}

/// Nearest-neighbour copy used when a native software blit first touches a
/// fully transparent picture cache. `BltAlpha`/`BltAlphaAdd` copy that source
/// pixel verbatim, retaining straight alpha for the later menu/HUD draw.
pub fn copy_stretched_picture(
    source: &Surface,
    source_rect: Rect,
    destination: &mut Surface,
    destination_rect: Rect,
) -> Option<()> {
    if source_rect.width == 0
        || source_rect.height == 0
        || destination_rect.width == 0
        || destination_rect.height == 0
    {
        return Some(());
    }
    for row in 0..destination_rect.height {
        let source_y = source_rect.y
            + (u64::from(row) * u64::from(source_rect.height) / u64::from(destination_rect.height))
                as i32;
        let destination_y = destination_rect.y + row as i32;
        for column in 0..destination_rect.width {
            let source_x = source_rect.x
                + (u64::from(column) * u64::from(source_rect.width)
                    / u64::from(destination_rect.width)) as i32;
            let destination_x = destination_rect.x + column as i32;
            if source_x < 0 || source_y < 0 || destination_x < 0 || destination_y < 0 {
                continue;
            }
            let color = source.get_pixel(source_x as u32, source_y as u32)?;
            destination
                .set_pixel(destination_x as u32, destination_y as u32, color)
                .ok()?;
        }
    }
    Some(())
}

pub fn software_blit_menu_image(
    destination: &mut Surface,
    image: &ImageData,
    destination_rect: Rect,
    mode: BlitMode,
) -> Option<()> {
    let source = Surface::from_bytes(
        image.width(),
        image.height(),
        PixelFormat::Rgba8888,
        image.pixels().to_vec(),
    )
    .ok()?;
    let source_rect = Rect::new(0, 0, image.width(), image.height());
    let mut layer = Surface::new(
        destination.width(),
        destination.height(),
        PixelFormat::Rgba8888,
    );
    copy_stretched_picture(&source, source_rect, &mut layer, destination_rect)?;
    let mut coverage_source = Surface::new(image.width(), image.height(), PixelFormat::Rgba8888);
    coverage_source.fill(Color::opaque(255, 255, 255));
    let mut coverage = Surface::new(
        destination.width(),
        destination.height(),
        PixelFormat::Rgba8888,
    );
    copy_stretched_picture(
        &coverage_source,
        source_rect,
        &mut coverage,
        destination_rect,
    )?;
    composite_software_picture_layer(destination, &layer, &coverage, mode)
}

/// Exact `BltAlpha`/`BltAlphaAdd` composition used by non-primary C4Surface
/// picture caches. Rust stores opacity, the inverse of C4's packed alpha byte.
pub fn composite_software_picture_layer(
    destination: &mut Surface,
    source: &Surface,
    coverage: &Surface,
    mode: BlitMode,
) -> Option<()> {
    if destination.width() != source.width()
        || destination.height() != source.height()
        || destination.width() != coverage.width()
        || destination.height() != coverage.height()
    {
        return None;
    }
    let additive = matches!(mode, BlitMode::Additive | BlitMode::Mod2Additive);
    for y in 0..destination.height() {
        for x in 0..destination.width() {
            if coverage.get_pixel(x, y)?.a == 0 {
                continue;
            }
            let foreground = source.get_pixel(x, y)?;
            let background = destination.get_pixel(x, y)?;
            let output = if background.a == 0 {
                foreground
            } else {
                let alpha = u16::from(foreground.a);
                let channel = |source: u8, destination: u8| -> u8 {
                    if additive {
                        (u16::from(destination) + (u16::from(source) * alpha >> 8)).min(255) as u8
                    } else {
                        ((u16::from(source) * alpha + u16::from(destination) * (255 - alpha)) >> 8)
                            as u8
                    }
                };
                Color::new(
                    channel(foreground.r, background.r),
                    channel(foreground.g, background.g),
                    channel(foreground.b, background.b),
                    background.a.saturating_add(foreground.a),
                )
            };
            destination.set_pixel(x, y, output).ok()?;
        }
    }
    Some(())
}
