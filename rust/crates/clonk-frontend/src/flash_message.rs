//! Classic fullscreen timed flash-message renderer
//! (`C4GraphicsSystem::DrawFlashMessage`).
//!
//! Message storage, byte truncation, placement timing, and lifetime belong to
//! the caller. This module implements only the final `FontRegular::TextOut`:
//! one markup-aware, white, centered draw over the existing fullscreen image.

use clonk_graphics::clonk_font::{ClonkFont, FontImageProvider, TextAlign};
use clonk_graphics::{GammaRamp, Point, Surface};

/// `CStdDDraw::DEFAULT_MESSAGE_COLOR` (`src/StdDDraw2.h:361`).
const MESSAGE_COLOR: [u8; 4] = [255, 255, 255, 255];

/// Computes the global `TextOut` anchor used by `DrawFlashMessage`.
///
/// `FlashMessageX == -1` selects `Config.Graphics.ResX / 2`. The Rust
/// fullscreen surface has that same whole-screen width; no viewport origin or
/// width participates in the calculation. `y` is the position snapshotted by
/// the caller when the message was installed.
pub fn flash_message_anchor(screen_width: u32, y: i32) -> Point {
    let screen_width = i32::try_from(screen_width).unwrap_or(i32::MAX);
    Point::new(screen_width / 2, y)
}

/// Draws one visible pass of the classic timed flash message.
///
/// C++ uses `FontRegular`, zoom `1.0`, `DEFAULT_MESSAGE_COLOR`, `ACenter`, and
/// the default markup-enabled `TextOut` path (`src/C4GraphicsSystem.cpp:652-664`).
/// `ClonkFont` consequently centers every newline/`|` row independently and
/// carries markup state between rows. No pane or background is drawn.
pub fn render_flash_message(
    surface: &mut Surface,
    font_regular: &ClonkFont,
    display_text: &str,
    y: i32,
    gamma: Option<&GammaRamp>,
    images: &dyn FontImageProvider,
) {
    let anchor = flash_message_anchor(surface.width(), y);
    font_regular.draw_with_gamma_and_images(
        surface,
        anchor.x,
        anchor.y,
        display_text,
        MESSAGE_COLOR,
        TextAlign::Center,
        true,
        gamma,
        images,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::clonk_font::{FontImageRef, GlyphCell};
    use clonk_graphics::{Color, PixelFormat};

    fn test_font() -> ClonkFont {
        let mut font = ClonkFont::new(3);
        for (character, width) in [('A', 3usize), ('B', 5usize)] {
            let mut pixels = vec![Color::opaque(255, 255, 255); width * 4];
            // Shadowed FontRegular cells are one pixel taller than their line
            // advance. Keep that overlap transparent so row-span assertions
            // below isolate each independently centered TextOut row.
            pixels[width * 3..].fill(Color::transparent());
            font.add_glyph(
                character,
                GlyphCell {
                    width: width as i32,
                    pixels,
                },
            );
        }
        font
    }

    fn pixel(surface: &Surface, x: u32, y: u32) -> Color {
        surface
            .get_pixel(x, y)
            .expect("test coordinate is on the surface")
    }

    fn changed_span(surface: &Surface, y: u32, background: Color) -> Option<(u32, u32)> {
        let changed = (0..surface.width())
            .filter(|x| pixel(surface, *x, y) != background)
            .collect::<Vec<_>>();
        changed.first().copied().zip(changed.last().copied())
    }

    struct NoImages;

    impl FontImageProvider for NoImages {
        fn font_image(&self, _tag: &str) -> Option<FontImageRef<'_>> {
            None
        }
    }

    #[test]
    fn render_centers_on_the_whole_surface_with_integer_division() {
        let font = test_font();
        let background = Color::opaque(7, 8, 9);
        let mut surface = Surface::new(11, 12, PixelFormat::Rgba8888);
        surface.fill(background);

        render_flash_message(&mut surface, &font, "A", 2, None, &NoImages);

        assert_eq!(flash_message_anchor(11, 2), Point::new(5, 2));
        assert_eq!(changed_span(&surface, 2, background), Some((4, 6)));
    }

    #[test]
    fn render_centers_newline_and_markup_pipe_rows_independently() {
        let font = test_font();
        let background = Color::opaque(7, 8, 9);
        let mut surface = Surface::new(11, 14, PixelFormat::Rgba8888);
        surface.fill(background);

        render_flash_message(&mut surface, &font, "A\nBB|A", 1, None, &NoImages);

        assert_eq!(changed_span(&surface, 1, background), Some((4, 6)));
        assert_eq!(changed_span(&surface, 4, background), Some((1, 9)));
        assert_eq!(changed_span(&surface, 7, background), Some((4, 6)));
    }

    #[test]
    fn render_uses_white_font_regular_and_honors_color_markup() {
        let font = test_font();
        let mut surface = Surface::new(11, 10, PixelFormat::Rgba8888);

        render_flash_message(&mut surface, &font, "A", 1, None, &NoImages);
        assert_eq!(pixel(&surface, 5, 1), Color::opaque(255, 255, 255));

        let mut marked_up = Surface::new(11, 10, PixelFormat::Rgba8888);
        render_flash_message(
            &mut marked_up,
            &font,
            "<c ffff00>A</c>",
            1,
            None,
            &NoImages,
        );
        assert_eq!(pixel(&marked_up, 5, 1), Color::opaque(254, 254, 0));
    }

    #[test]
    fn render_forwards_the_active_gamma_ramp() {
        let font = test_font();
        let gamma = GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let mut surface = Surface::new(11, 10, PixelFormat::Rgba8888);

        render_flash_message(
            &mut surface,
            &font,
            "<c 000000>A</c>",
            1,
            Some(&gamma),
            &NoImages,
        );

        assert_eq!(pixel(&surface, 5, 1), Color::opaque(17, 33, 49));
    }

    #[test]
    fn render_draws_no_pane_or_background() {
        let font = test_font();
        let background = Color::opaque(21, 34, 55);
        let mut surface = Surface::new(11, 10, PixelFormat::Rgba8888);
        surface.fill(background);

        render_flash_message(&mut surface, &font, "A", 3, None, &NoImages);

        assert_eq!(pixel(&surface, 0, 0), background);
        assert_eq!(pixel(&surface, 10, 9), background);
        assert_eq!(pixel(&surface, 3, 3), background);
        assert_ne!(pixel(&surface, 5, 3), background);
    }
}
