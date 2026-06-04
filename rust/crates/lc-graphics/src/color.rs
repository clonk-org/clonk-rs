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

    /// Modulate this color by `modulation`, mirroring C++ `ModulateClr`
    /// (`src/StdColors.h:159`): each RGB channel is `(a * b) >> 8` and the alpha
    /// channel is the screen combine `min(a + b - ((a * b) >> 8), 0xff)`. This is
    /// the per-pixel application of a blit's `dwModClr` (team/owner tinting, fades,
    /// damage flashes, …). The `>> 8` (not `/ 255`) is deliberate: white·white =
    /// `(255*255)>>8 = 254`, exactly as C++ produces, so modulated output matches
    /// the engine bit-for-bit.
    pub fn modulate_clr(self, modulation: Color) -> Color {
        let mul = |a: u8, b: u8| -> u8 { ((a as u16 * b as u16) >> 8) as u8 };
        let screen = |a: u8, b: u8| -> u8 {
            (a as u16 + b as u16 - ((a as u16 * b as u16) >> 8)).min(0xff) as u8
        };
        Color {
            r: mul(self.r, modulation.r),
            g: mul(self.g, modulation.g),
            b: mul(self.b, modulation.b),
            a: screen(self.a, modulation.a),
        }
    }

    /// Modulate this color by `modulation`, mirroring C++ `ModulateClrMOD2`
    /// (`src/StdColors.h:183`): RGB is `clamp((a + b - 0x7f) * 2, 0, 0xff)` and
    /// alpha is `min(a + b, 0xff)`. Used by the `C4GFXBLIT_MOD2` blit mode
    /// (additive color modulation around a 0x7f mid-grey pivot).
    pub fn modulate_clr_mod2(self, modulation: Color) -> Color {
        let mod2 = |a: u8, b: u8| -> u8 { ((a as i32 + b as i32 - 0x7f) * 2).clamp(0, 0xff) as u8 };
        let add = |a: u8, b: u8| -> u8 { (a as u16 + b as u16).min(0xff) as u8 };
        Color {
            r: mod2(self.r, modulation.r),
            g: mod2(self.g, modulation.g),
            b: mod2(self.b, modulation.b),
            a: add(self.a, modulation.a),
        }
    }

    /// Additive composite of this (already-modulated) source over `dest`,
    /// mirroring the C++ sprite additive path `glBlendFunc(GL_SRC_ALPHA, GL_ONE)`
    /// (`src/StdGL.cpp:908`): `dst + src·srcAlpha`, clamped per channel. Used by
    /// the `C4GFXBLIT_ADDITIVE` blit mode (fire, energy, flashes). Destination
    /// alpha is preserved (the framebuffer stays opaque).
    pub(crate) fn blend_additive(self, dest: Color) -> Color {
        let sa = self.a as u16;
        let add = |s: u8, d: u8| -> u8 { (d as u16 + (s as u16 * sa) / 255).min(255) as u8 };
        Color {
            r: add(self.r, dest.r),
            g: add(self.g, dest.g),
            b: add(self.b, dest.b),
            a: dest.a,
        }
    }

    /// `C4GFXBLIT_MOD2` composite: combine the (modulated) source with the
    /// destination via `modulate_clr_mod2` (additive modulation around 0x7f),
    /// alpha-weighted by the source so transparent pixels leave the destination
    /// unchanged. Destination alpha is preserved. (Full/zero source alpha are
    /// exact; partial alpha lerps toward the combined value.)
    pub(crate) fn blend_mod2(self, dest: Color) -> Color {
        let combined = self.modulate_clr_mod2(dest);
        let sa = self.a as u16;
        if sa == 0 {
            return dest;
        }
        if sa == 255 {
            return Color {
                a: dest.a,
                ..combined
            };
        }
        let inv = 255 - sa;
        let mix = |c: u8, d: u8| -> u8 { ((c as u16 * sa + d as u16 * inv) / 255) as u8 };
        Color {
            r: mix(combined.r, dest.r),
            g: mix(combined.g, dest.g),
            b: mix(combined.b, dest.b),
            a: dest.a,
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

    #[test]
    fn modulate_clr_matches_cpp_modulateclr() {
        // RGB = (a*b)>>8, exactly as C++ ModulateClr (src/StdColors.h:159).
        // White-by-half-grey: (200*128)>>8 = 100.
        assert_eq!(
            Color::new(200, 200, 200, 255).modulate_clr(Color::new(128, 128, 128, 255)),
            Color::new(100, 100, 100, 255)
        );
        // Parity-critical quirk: white·white = (255*255)>>8 = 254 per channel,
        // NOT 255. Alpha screen: min(255+255-254, 255) = 255.
        assert_eq!(
            Color::opaque(255, 255, 255).modulate_clr(Color::opaque(255, 255, 255)),
            Color::new(254, 254, 254, 255)
        );
        // Identity-ish: modulating by white leaves RGB nearly unchanged but shows
        // the >>8 rounding (254 not 255 for full channels).
        let m = Color::new(255, 0, 128, 255).modulate_clr(Color::opaque(255, 255, 255));
        assert_eq!(m, Color::new(254, 0, 127, 255));
    }

    #[test]
    fn blend_additive_matches_gl_src_alpha_one() {
        // dst + src*srcAlpha, clamped. src=(200,100,50,128) over dst=(100,50,0,255):
        // r=100+(200*128/255)=200, g=50+(100*128/255)=100, b=0+(50*128/255)=25, a=dst.a.
        let src = Color::new(200, 100, 50, 128);
        let dst = Color::new(100, 50, 0, 255);
        assert_eq!(src.blend_additive(dst), Color::new(200, 100, 25, 255));
        // Full-alpha bright source clamps to white.
        assert_eq!(
            Color::new(200, 200, 200, 255).blend_additive(Color::opaque(100, 100, 100)),
            Color::opaque(255, 255, 255)
        );
        // Zero-alpha source is a no-op.
        assert_eq!(
            Color::new(255, 255, 255, 0).blend_additive(Color::opaque(10, 20, 30)),
            Color::opaque(10, 20, 30)
        );
    }

    #[test]
    fn modulate_clr_mod2_matches_cpp() {
        // RGB = clamp((a+b-0x7f)*2, 0, 0xff); alpha = min(a+b, 0xff).
        // Mid-grey pivot: (0x7f + 0x7f - 0x7f)*2 = 0xfe.
        assert_eq!(
            Color::new(0x7f, 0x7f, 0x7f, 0).modulate_clr_mod2(Color::new(0x7f, 0x7f, 0x7f, 0)),
            Color::new(0xfe, 0xfe, 0xfe, 0)
        );
        // Below pivot clamps to 0; above clamps to 0xff.
        assert_eq!(
            Color::new(0, 255, 0x7f, 200).modulate_clr_mod2(Color::new(0x40, 255, 0x7f, 200)),
            Color::new(0, 0xff, 0xfe, 0xff)
        );
    }
}
