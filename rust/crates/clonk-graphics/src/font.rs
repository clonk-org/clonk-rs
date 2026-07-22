use crate::{Color, Surface};
use font8x8::UnicodeFonts;
use rusttype::{point, PositionedGlyph, Scale};
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub width: f32,
    pub height: f32,
    pub lines: usize,
}

impl Default for FontMetrics {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            lines: 0,
        }
    }
}

pub trait TextFont: Send + Sync {
    fn measure_text(&self, text: &str, font_size: f32) -> FontMetrics;
    fn draw_text(
        &self,
        surface: &mut Surface,
        origin_x: f32,
        origin_y: f32,
        text: &str,
        font_size: f32,
        color: Color,
    );
}

/// Simple bitmap font backed by the public domain `font8x8` glyph tables.
#[derive(Clone, Copy, Debug, Default)]
pub struct BitmapFont;

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

impl TextFont for BitmapFont {
    fn measure_text(&self, text: &str, font_size: f32) -> FontMetrics {
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

    fn draw_text(
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
}

#[derive(Debug, Error)]
pub enum TrueTypeFontError {
    #[error("invalid font data")]
    InvalidData,
}

pub struct TrueTypeFont {
    font: rusttype::Font<'static>,
}

impl TrueTypeFont {
    pub fn from_bytes(bytes: Arc<[u8]>) -> Result<Self, TrueTypeFontError> {
        rusttype::Font::try_from_vec(bytes.to_vec())
            .map(|font| Self { font })
            .ok_or(TrueTypeFontError::InvalidData)
    }

    fn scale(font_size: f32) -> Scale {
        Scale::uniform(font_size.max(1.0))
    }

    fn line_height(&self, scale: Scale) -> f32 {
        let metrics = self.font.v_metrics(scale);
        (metrics.ascent - metrics.descent + metrics.line_gap).max(1.0)
    }

    fn layout_line<'a>(
        &'a self,
        text: &'a str,
        scale: Scale,
        origin_x: f32,
        origin_y: f32,
    ) -> Vec<PositionedGlyph<'a>> {
        self.font
            .layout(text, scale, point(origin_x, origin_y))
            .collect()
    }

    fn draw_glyph(&self, surface: &mut Surface, glyph: &PositionedGlyph<'_>, color: Color) {
        if let Some(bb) = glyph.pixel_bounding_box() {
            glyph.draw(|x, y, coverage| {
                if coverage <= 0.0 {
                    return;
                }
                let alpha = (color.a as f32 * coverage).round().clamp(0.0, 255.0) as u8;
                if alpha == 0 {
                    return;
                }
                let px = bb.min.x + x as i32;
                let py = bb.min.y + y as i32;
                if px < 0 || py < 0 {
                    return;
                }
                let (px, py) = (px as u32, py as u32);
                if px >= surface.width() || py >= surface.height() {
                    return;
                }
                let blended = Color::new(color.r, color.g, color.b, alpha);
                let _ = surface.blend_pixel(px, py, blended);
            });
        }
    }
}

impl TextFont for TrueTypeFont {
    fn measure_text(&self, text: &str, font_size: f32) -> FontMetrics {
        let scale = Self::scale(font_size);
        let line_height = self.line_height(scale);
        let mut lines = 0usize;
        let mut max_width = 0.0f32;

        for line in text.split('\n') {
            lines += 1;
            let mut line_width = 0.0f32;
            for glyph in self.layout_line(line, scale, 0.0, 0.0) {
                if let Some(bb) = glyph.pixel_bounding_box() {
                    line_width = line_width.max(bb.max.x as f32);
                }
                let advance = glyph.unpositioned().h_metrics().advance_width;
                line_width = line_width.max(glyph.position().x + advance);
            }
            if line.is_empty() {
                line_width = line_width.max(scale.x * 0.5);
            }
            max_width = max_width.max(line_width);
        }

        if lines == 0 {
            lines = 1;
        }

        FontMetrics {
            width: max_width,
            height: line_height * lines as f32,
            lines,
        }
    }

    fn draw_text(
        &self,
        surface: &mut Surface,
        origin_x: f32,
        origin_y: f32,
        text: &str,
        font_size: f32,
        color: Color,
    ) {
        let scale = Self::scale(font_size);
        let line_height = self.line_height(scale);
        let v_metrics = self.font.v_metrics(scale);
        let mut baseline_y = origin_y + v_metrics.ascent;

        for line in text.split('\n') {
            let glyphs = self.layout_line(line, scale, origin_x, baseline_y);
            for glyph in &glyphs {
                self.draw_glyph(surface, glyph, color);
            }
            baseline_y += line_height;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PixelFormat, Surface};
    #[test]
    fn measure_text_accounts_for_newlines_bitmap() {
        let font = BitmapFont::new();
        let metrics = font.measure_text("AB\nCD", 16.0);
        assert_eq!(metrics.lines, 2);
        assert!(metrics.height > 0.0);
        assert!(metrics.width > 0.0);
    }

    #[test]
    fn draw_text_marks_surface_pixels_bitmap() {
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

    #[test]
    fn truetype_font_renders_text() {
        let font_bytes = include_bytes!("../../../../planet/System.c4g/Endeavour.ttf");
        let font = TrueTypeFont::from_bytes(Arc::from(&font_bytes[..])).expect("valid font");
        let mut surface = Surface::new(128, 32, PixelFormat::Rgba8888);
        font.draw_text(
            &mut surface,
            0.0,
            0.0,
            "LC",
            24.0,
            Color::opaque(255, 255, 255),
        );
        assert!(surface
            .pixels()
            .chunks_exact(4)
            .any(|px| px[3] > 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0)));
    }
}
