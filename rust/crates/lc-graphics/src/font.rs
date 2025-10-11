use crate::{Color, Surface};
use font8x8::UnicodeFonts;

/// Simple bitmap font backed by the public domain `font8x8` glyph tables.
#[derive(Clone, Copy, Debug, Default)]
pub struct BitmapFont;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub width: f32,
    pub height: f32,
    pub lines: usize,
}

impl BitmapFont {
    const GLYPH_WIDTH: f32 = 8.0;
    const GLYPH_HEIGHT: f32 = 8.0;

    pub const fn new() -> Self {
        Self
    }

    fn scale_for_size(font_size: f32) -> f32 {
        (font_size / Self::GLYPH_HEIGHT).max(0.5)
    }

    fn glyph_advance(scale: f32) -> f32 {
        Self::GLYPH_WIDTH * scale
    }

    fn char_advance(ch: char, scale: f32) -> f32 {
        match ch {
            ' ' => Self::glyph_advance(scale) * 0.5,
            '\t' => Self::glyph_advance(scale) * 2.0,
            _ => Self::glyph_advance(scale) + scale,
        }
    }

    fn line_height(scale: f32) -> f32 {
        Self::GLYPH_HEIGHT * scale
    }

    fn glyph_for(ch: char) -> [u8; 8] {
        font8x8::BASIC_FONTS
            .get(ch)
            .or_else(|| font8x8::BASIC_FONTS.get('?'))
            .unwrap_or([0; 8])
    }

    /// Measures the pixel width and height of the provided text when rendered at the requested font size.
    pub fn measure_text(&self, text: &str, font_size: f32) -> FontMetrics {
        let scale = Self::scale_for_size(font_size);
        let mut lines = 1usize;
        let mut max_width = 0.0f32;
        let mut line_width = 0.0f32;

        for ch in text.chars() {
            if ch == '\n' {
                max_width = max_width.max(line_width);
                line_width = 0.0;
                lines += 1;
                continue;
            }
            line_width += Self::char_advance(ch, scale);
        }

        if text.is_empty() {
            line_width = Self::glyph_advance(scale) * 0.5;
        }

        max_width = max_width.max(line_width);
        let height = Self::line_height(scale) * lines as f32;
        FontMetrics {
            width: max_width,
            height,
            lines,
        }
    }

    /// Renders the supplied text onto the surface starting at `origin`.
    pub fn draw_text(
        &self,
        surface: &mut Surface,
        origin_x: f32,
        origin_y: f32,
        text: &str,
        font_size: f32,
        color: Color,
    ) {
        let scale = Self::scale_for_size(font_size);
        let line_height = Self::line_height(scale);
        let mut cursor_x = origin_x;
        let mut cursor_y = origin_y;

        for ch in text.chars() {
            match ch {
                '\n' => {
                    cursor_x = origin_x;
                    cursor_y += line_height;
                    continue;
                }
                ' ' => {
                    cursor_x += Self::glyph_advance(scale) * 0.5;
                    continue;
                }
                '\t' => {
                    cursor_x += Self::glyph_advance(scale) * 2.0;
                    continue;
                }
                _ => {}
            }

            let glyph = Self::glyph_for(ch);
            self.paint_glyph(surface, cursor_x, cursor_y, glyph, scale, color);
            cursor_x += Self::glyph_advance(scale) + scale;
        }
    }

    fn paint_glyph(
        &self,
        surface: &mut Surface,
        origin_x: f32,
        origin_y: f32,
        glyph: [u8; 8],
        scale: f32,
        color: Color,
    ) {
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..8 {
                if bits & (1 << col) == 0 {
                    continue;
                }
                let x = origin_x + (col as f32) * scale;
                let y = origin_y + (row as f32) * scale;
                self.paint_scaled_pixel(surface, x, y, scale, color);
            }
        }
    }

    fn paint_scaled_pixel(&self, surface: &mut Surface, x: f32, y: f32, scale: f32, color: Color) {
        let mut x0 = x.floor() as i32;
        let mut y0 = y.floor() as i32;
        let mut x1 = (x + scale).ceil() as i32;
        let mut y1 = (y + scale).ceil() as i32;

        if x1 <= x0 {
            x1 = x0 + 1;
        }
        if y1 <= y0 {
            y1 = y0 + 1;
        }

        let width = surface.width() as i32;
        let height = surface.height() as i32;

        x0 = x0.clamp(0, width);
        x1 = x1.clamp(0, width);
        y0 = y0.clamp(0, height);
        y1 = y1.clamp(0, height);

        for yy in y0..y1 {
            for xx in x0..x1 {
                let _ = surface.set_pixel(xx as u32, yy as u32, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PixelFormat, Surface};

    #[test]
    fn measure_text_accounts_for_newlines() {
        let font = BitmapFont::new();
        let metrics = font.measure_text("AB\nCD", 16.0);
        assert_eq!(metrics.lines, 2);
        assert!(metrics.height > 0.0);
        assert!(metrics.width > 0.0);
    }

    #[test]
    fn draw_text_marks_surface_pixels() {
        let font = BitmapFont::new();
        let mut surface = Surface::new(32, 32, PixelFormat::Rgba8888);
        font.draw_text(
            &mut surface,
            0.0,
            0.0,
            "A",
            16.0,
            Color::opaque(255, 255, 255),
        );
        let mut painted = 0usize;
        for y in 0..surface.height() {
            for x in 0..surface.width() {
                if surface.get_pixel(x, y).unwrap().a > 0 {
                    painted += 1;
                }
            }
        }
        assert!(painted > 0);
    }
}
