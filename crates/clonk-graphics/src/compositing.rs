//! Software surface compositing: nearest-neighbour stretching, aspect fitting,
//! and the exact `BltAlpha`/`BltAlphaAdd` layer composition C4Surface performs
//! for a non-primary picture cache.
//!
//! These are graphics operations on `Surface`/`Rect`/`Color`, with no notion of
//! a menu, an image asset, or application state — which is why they live here
//! rather than beside the callers that happen to use them. The `ImageData`
//! adapters stay at the GUI-facing layer, so this crate never depends on
//! `clonk-gui`.

use crate::color::Color;
use crate::surface::{BlitMode, PixelFormat, Rect, Surface};

/// The largest rectangle of the source's aspect ratio that fits `destination`,
/// centred on the axis it does not fill.
///
/// `None` for a degenerate source, which has no ratio to preserve.
pub fn aspect_fit_rect(source_width: u32, source_height: u32, destination: Rect) -> Option<Rect> {
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

/// Nearest-neighbour copy used when a native software blit first touches a
/// fully transparent picture cache. `BltAlpha`/`BltAlphaAdd` copy that source
/// pixel verbatim, retaining straight alpha for the later menu/HUD draw.
pub fn copy_stretched(
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

/// Stretch `source` into `destination_rect` and composite it, building the
/// coverage mask the composition needs from the same stretch.
///
/// The mask is what keeps the composition to the pixels the stretch actually
/// wrote: a fully transparent source pixel still counts as covered, which a
/// test on the layer's own alpha would miss.
pub fn blit_stretched(
    destination: &mut Surface,
    source: &Surface,
    source_rect: Rect,
    destination_rect: Rect,
    mode: BlitMode,
) -> Option<()> {
    let mut layer = Surface::new(
        destination.width(),
        destination.height(),
        PixelFormat::Rgba8888,
    );
    copy_stretched(source, source_rect, &mut layer, destination_rect)?;
    let mut coverage_source = Surface::new(
        source_rect.width.max(1),
        source_rect.height.max(1),
        PixelFormat::Rgba8888,
    );
    coverage_source.fill(Color::opaque(255, 255, 255));
    let mut coverage = Surface::new(
        destination.width(),
        destination.height(),
        PixelFormat::Rgba8888,
    );
    copy_stretched(
        &coverage_source,
        Rect::new(0, 0, source_rect.width, source_rect.height),
        &mut coverage,
        destination_rect,
    )?;
    composite_picture_layer(destination, &layer, &coverage, mode)
}

/// Exact `BltAlpha`/`BltAlphaAdd` composition used by non-primary C4Surface
/// picture caches. Rust stores opacity, the inverse of C4's packed alpha byte.
pub fn composite_picture_layer(
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
                        (u16::from(destination) + ((u16::from(source) * alpha) >> 8)).min(255) as u8
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

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(width: u32, height: u32, color: Color) -> Surface {
        let mut surface = Surface::new(width, height, PixelFormat::Rgba8888);
        surface.fill(color);
        surface
    }

    /// The fit keeps the source ratio and centres on the slack axis.
    #[test]
    fn aspect_fit_centres_the_axis_it_does_not_fill() {
        // A 2:1 source in a square box keeps the width and centres vertically.
        let wide = aspect_fit_rect(200, 100, Rect::new(0, 0, 100, 100)).expect("fits");
        assert_eq!((wide.width, wide.height), (100, 50));
        assert_eq!((wide.x, wide.y), (0, 25));

        // A 1:2 source keeps the height and centres horizontally.
        let tall = aspect_fit_rect(100, 200, Rect::new(0, 0, 100, 100)).expect("fits");
        assert_eq!((tall.width, tall.height), (50, 100));
        assert_eq!((tall.x, tall.y), (25, 0));

        // A matching ratio fills the box exactly and moves nothing.
        let exact = aspect_fit_rect(50, 50, Rect::new(4, 6, 100, 100)).expect("fits");
        assert_eq!(exact, Rect::new(4, 6, 100, 100));

        // A degenerate source has no ratio to preserve.
        assert_eq!(aspect_fit_rect(0, 10, Rect::new(0, 0, 10, 10)), None);
        assert_eq!(aspect_fit_rect(10, 0, Rect::new(0, 0, 10, 10)), None);
    }

    /// The stretch is nearest-neighbour, and an empty rectangle is a no-op
    /// rather than a failure.
    #[test]
    fn copy_stretched_replicates_source_pixels_without_blending() {
        let mut source = Surface::new(2, 1, PixelFormat::Rgba8888);
        source
            .set_pixel(0, 0, Color::opaque(255, 0, 0))
            .expect("left");
        source
            .set_pixel(1, 0, Color::opaque(0, 0, 255))
            .expect("right");
        let mut destination = Surface::new(4, 1, PixelFormat::Rgba8888);

        copy_stretched(
            &source,
            Rect::new(0, 0, 2, 1),
            &mut destination,
            Rect::new(0, 0, 4, 1),
        )
        .expect("stretch");

        // Each source pixel is duplicated, never averaged.
        assert_eq!(destination.get_pixel(0, 0), Some(Color::opaque(255, 0, 0)));
        assert_eq!(destination.get_pixel(1, 0), Some(Color::opaque(255, 0, 0)));
        assert_eq!(destination.get_pixel(2, 0), Some(Color::opaque(0, 0, 255)));
        assert_eq!(destination.get_pixel(3, 0), Some(Color::opaque(0, 0, 255)));

        let mut untouched = Surface::new(4, 1, PixelFormat::Rgba8888);
        assert_eq!(
            copy_stretched(
                &source,
                Rect::new(0, 0, 0, 1),
                &mut untouched,
                Rect::new(0, 0, 4, 1)
            ),
            Some(())
        );
    }

    /// `BltAlpha` weights by the source's alpha; `BltAlphaAdd` accumulates and
    /// saturates. A fully transparent destination takes the source verbatim.
    #[test]
    fn composition_matches_the_two_native_blit_modes() {
        let coverage = filled(1, 1, Color::opaque(255, 255, 255));
        let source = filled(1, 1, Color::new(200, 0, 0, 128));

        // Transparent destination: copied straight through, alpha and all.
        let mut fresh = Surface::new(1, 1, PixelFormat::Rgba8888);
        composite_picture_layer(&mut fresh, &source, &coverage, BlitMode::Normal).expect("alpha");
        assert_eq!(fresh.get_pixel(0, 0), Some(Color::new(200, 0, 0, 128)));

        // Alpha over an opaque black background: (200*128 + 0*127) >> 8.
        let mut blended = filled(1, 1, Color::opaque(0, 0, 0));
        composite_picture_layer(&mut blended, &source, &coverage, BlitMode::Normal).expect("alpha");
        assert_eq!(blended.get_pixel(0, 0).map(|pixel| pixel.r), Some(100));

        // Additive over the same background: 0 + ((200*128) >> 8).
        let mut added = filled(1, 1, Color::opaque(0, 0, 0));
        composite_picture_layer(&mut added, &source, &coverage, BlitMode::Additive)
            .expect("additive");
        assert_eq!(added.get_pixel(0, 0).map(|pixel| pixel.r), Some(100));

        // Additive saturates rather than wrapping.
        let bright = filled(1, 1, Color::new(255, 255, 255, 255));
        let mut saturated = filled(1, 1, Color::opaque(200, 200, 200));
        composite_picture_layer(&mut saturated, &bright, &coverage, BlitMode::Additive)
            .expect("additive");
        assert_eq!(saturated.get_pixel(0, 0).map(|pixel| pixel.r), Some(255));
    }

    /// A pixel the coverage mask does not mark is left alone, even where the
    /// layer holds colour — which is what keeps a stretch inside its rectangle.
    #[test]
    fn composition_touches_only_covered_pixels_and_rejects_mismatched_sizes() {
        let mut destination = filled(2, 1, Color::opaque(0, 0, 0));
        let source = filled(2, 1, Color::new(255, 255, 255, 255));
        let mut coverage = Surface::new(2, 1, PixelFormat::Rgba8888);
        coverage
            .set_pixel(0, 0, Color::opaque(255, 255, 255))
            .expect("covered");

        composite_picture_layer(&mut destination, &source, &coverage, BlitMode::Normal)
            .expect("composite");

        // 254 for the same shift-by-eight reason as the blit test above.
        assert_eq!(
            destination.get_pixel(0, 0),
            Some(Color::new(254, 254, 254, 255))
        );
        assert_eq!(
            destination.get_pixel(1, 0),
            Some(Color::opaque(0, 0, 0)),
            "an uncovered pixel keeps its background"
        );

        let wrong = Surface::new(3, 1, PixelFormat::Rgba8888);
        assert_eq!(
            composite_picture_layer(&mut destination, &wrong, &coverage, BlitMode::Normal),
            None,
            "a layer of the wrong size composites nothing"
        );
    }

    /// The stretching blit covers exactly its destination rectangle.
    #[test]
    fn blit_stretched_writes_only_inside_its_rectangle() {
        let source = filled(1, 1, Color::new(255, 0, 0, 255));
        let mut destination = filled(4, 1, Color::opaque(0, 0, 0));

        blit_stretched(
            &mut destination,
            &source,
            Rect::new(0, 0, 1, 1),
            Rect::new(1, 0, 2, 1),
            BlitMode::Normal,
        )
        .expect("blit");

        assert_eq!(destination.get_pixel(0, 0), Some(Color::opaque(0, 0, 0)));
        // 254, not 255: the blend is C4's `(src*a + dst*(255-a)) >> 8`, and a
        // shift by eight is not a divide by 255, so even a fully opaque source
        // over an opaque background loses one. That is the native arithmetic,
        // and rounding it would be the divergence.
        assert_eq!(destination.get_pixel(1, 0).map(|pixel| pixel.r), Some(254));
        assert_eq!(destination.get_pixel(2, 0).map(|pixel| pixel.r), Some(254));
        assert_eq!(destination.get_pixel(3, 0), Some(Color::opaque(0, 0, 0)));
    }
}
