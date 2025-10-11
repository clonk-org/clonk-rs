#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }

    pub const fn transparent() -> Self {
        Self::new(0, 0, 0, 0)
    }

    pub fn modulate(self, factor: f32) -> Self {
        let clamped = factor.max(0.0);
        let scale_channel = |component: u8| -> u8 {
            let scaled = (component as f32 * clamped).round();
            scaled.clamp(0.0, 255.0) as u8
        };
        Self {
            r: scale_channel(self.r),
            g: scale_channel(self.g),
            b: scale_channel(self.b),
            a: self.a,
        }
    }

    pub(crate) fn blend_over(self, dest: Color) -> Color {
        let alpha = self.a as u16;
        if alpha == 0 {
            return dest;
        }
        if alpha == 255 {
            return self;
        }

        let inv_alpha = 255u16 - alpha;
        let r = (self.r as u16 * alpha + dest.r as u16 * inv_alpha) / 255;
        let g = (self.g as u16 * alpha + dest.g as u16 * inv_alpha) / 255;
        let b = (self.b as u16 * alpha + dest.b as u16 * inv_alpha) / 255;
        let a = alpha + (dest.a as u16 * inv_alpha) / 255;

        Color {
            r: r as u8,
            g: g as u8,
            b: b as u8,
            a: a.min(255) as u8,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::transparent()
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn modulate_scales_rgb_channels() {
        let color = Color::new(100, 150, 200, 255);
        let darker = color.modulate(0.5);
        assert_eq!(darker, Color::new(50, 75, 100, 255));

        let brighter = color.modulate(1.5);
        assert_eq!(brighter, Color::new(150, 225, 255, 255));
    }

    #[test]
    fn modulate_clamps_negative_factors() {
        let color = Color::opaque(30, 60, 90);
        assert_eq!(color.modulate(-2.0), Color::opaque(0, 0, 0));
    }
}
