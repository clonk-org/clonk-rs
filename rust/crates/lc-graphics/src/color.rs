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
