//! Classic fullscreen F1 help overlay (`C4GraphicsSystem::DrawHelp`).
//!
//! The C++ caller resolves localized labels and keyboard bindings before it
//! draws. This module therefore accepts the two fully formatted column
//! strings, including their blank lines and balanced `<c ...>` markup.

use clonk_graphics::clonk_font::{ClonkFont, FontImageProvider, TextAlign};
use clonk_graphics::{GammaRamp, Point, Rect, Surface};

/// `CStdDDraw::DEFAULT_MESSAGE_COLOR` (`src/StdDDraw2.h:361`).
const MESSAGE_COLOR: [u8; 4] = [255, 255, 255, 255];

/// The two left-aligned `TextOut` anchors used by the F1 help overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeHelpLayout {
    pub left_anchor: Point,
    pub right_anchor: Point,
}

/// Computes `C4GraphicsSystem::DrawHelp`'s column anchors.
///
/// `ViewportArea` supplies only the origin and width. The original renderer
/// neither clips the text to this rectangle nor draws a panel behind it.
pub fn runtime_help_layout(viewport_area: Rect) -> RuntimeHelpLayout {
    let width = i32::try_from(viewport_area.width).unwrap_or(i32::MAX);
    let y = viewport_area.y.saturating_add(64);
    RuntimeHelpLayout {
        left_anchor: Point::new(viewport_area.x.saturating_add(128), y),
        right_anchor: Point::new(
            viewport_area.x.saturating_add(width / 2).saturating_add(64),
            y,
        ),
    }
}

/// Draws the already-resolved left and right F1 help columns.
///
/// The strings are passed to one markup-aware `CStdFont::DrawText` equivalent
/// per column. Consequently `\n` advances by exactly `FontRegular`'s line
/// height, blank lines are retained, and markup state follows the same
/// multiline behavior as the C++ `TextOut` calls.
#[allow(clippy::too_many_arguments)]
pub fn render_runtime_help(
    surface: &mut Surface,
    font_regular: &ClonkFont,
    viewport_area: Rect,
    left_display_lines: &str,
    right_display_lines: &str,
    gamma: Option<&GammaRamp>,
    images: &dyn FontImageProvider,
) {
    let layout = runtime_help_layout(viewport_area);
    font_regular.draw_with_gamma_and_images(
        surface,
        layout.left_anchor.x,
        layout.left_anchor.y,
        left_display_lines,
        MESSAGE_COLOR,
        TextAlign::Left,
        true,
        gamma,
        images,
    );
    font_regular.draw_with_gamma_and_images(
        surface,
        layout.right_anchor.x,
        layout.right_anchor.y,
        right_display_lines,
        MESSAGE_COLOR,
        TextAlign::Left,
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

    fn test_font(line_height: i32) -> ClonkFont {
        let mut font = ClonkFont::new(line_height);
        for character in ['A', 'B'] {
            font.add_glyph(
                character,
                GlyphCell {
                    width: 1,
                    pixels: vec![
                        Color::opaque(255, 255, 255);
                        usize::try_from(font.cell_height).expect("positive cell height")
                    ],
                },
            );
        }
        font
    }

    fn pixel(surface: &Surface, x: i32, y: i32) -> Color {
        surface
            .get_pixel(x as u32, y as u32)
            .expect("test coordinate is on the surface")
    }

    struct NoImages;

    impl FontImageProvider for NoImages {
        fn font_image(&self, _tag: &str) -> Option<FontImageRef<'_>> {
            None
        }
    }

    #[test]
    fn layout_uses_viewport_origin_and_truncating_half_width() {
        let layout = runtime_help_layout(Rect::new(-10, 7, 641, 333));

        assert_eq!(layout.left_anchor, Point::new(118, 71));
        assert_eq!(layout.right_anchor, Point::new(374, 71));
    }

    #[test]
    fn render_uses_both_anchors_and_preserves_blank_lines() {
        let font = test_font(2);
        let area = Rect::new(10, 20, 400, 200);
        let layout = runtime_help_layout(area);
        let background = Color::opaque(9, 10, 11);
        let mut surface = Surface::new(500, 160, PixelFormat::Rgba8888);
        surface.fill(background);

        render_runtime_help(&mut surface, &font, area, "A\n\nA", "B\nB", None, &NoImages);

        assert_eq!(
            pixel(&surface, layout.left_anchor.x, layout.left_anchor.y),
            Color::opaque(255, 255, 255)
        );
        assert_eq!(
            pixel(&surface, layout.left_anchor.x, layout.left_anchor.y + 3),
            background
        );
        assert_eq!(
            pixel(&surface, layout.left_anchor.x, layout.left_anchor.y + 4),
            Color::opaque(255, 255, 255)
        );
        assert_eq!(
            pixel(&surface, layout.right_anchor.x, layout.right_anchor.y),
            Color::opaque(255, 255, 255)
        );
        assert_eq!(
            pixel(&surface, layout.right_anchor.x, layout.right_anchor.y + 2),
            Color::opaque(255, 255, 255)
        );

        // DrawHelp has no pane or background of its own.
        assert_eq!(pixel(&surface, area.x, area.y), background);
    }

    #[test]
    fn render_keeps_white_base_text_and_honors_key_markup() {
        let font = test_font(2);
        let area = Rect::new(0, 0, 400, 120);
        let layout = runtime_help_layout(area);
        let mut surface = Surface::new(400, 100, PixelFormat::Rgba8888);

        render_runtime_help(
            &mut surface,
            &font,
            area,
            "<c ffff00>A</c>",
            "B",
            None,
            &NoImages,
        );

        assert_eq!(
            pixel(&surface, layout.left_anchor.x, layout.left_anchor.y),
            Color::opaque(254, 254, 0)
        );
        assert_eq!(
            pixel(&surface, layout.right_anchor.x, layout.right_anchor.y),
            Color::opaque(255, 255, 255)
        );
    }

    #[test]
    fn render_forwards_the_active_gamma_ramp() {
        let font = test_font(2);
        let area = Rect::new(0, 0, 400, 120);
        let layout = runtime_help_layout(area);
        let gamma = GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);
        let mut surface = Surface::new(400, 100, PixelFormat::Rgba8888);

        render_runtime_help(
            &mut surface,
            &font,
            area,
            "<c 000000>A</c>",
            "",
            Some(&gamma),
            &NoImages,
        );

        assert_eq!(
            pixel(&surface, layout.left_anchor.x, layout.left_anchor.y),
            Color::opaque(17, 33, 49)
        );
    }
}
