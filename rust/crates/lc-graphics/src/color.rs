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

    /// Apply a packed C4 blit modulation to this opacity-alpha texel. Each RGB
    /// channel is `(a * b) >> 8`; `modulation.a` is C4 transparency, so the
    /// resulting opacity is `self.a.saturating_sub(modulation.a)`. This mirrors
    /// StdGL's texture-transparency addition (`GL_COMBINE_ALPHA = GL_ADD`), after
    /// converting back to Rust's opacity convention. Combining two packed C4
    /// modulation dwords is a different operation and still uses alpha screen.
    ///
    /// The `>> 8` (not `/ 255`) is deliberate: white·white = 254, matching the
    /// engine's modulation channel math bit-for-bit.
    pub fn modulate_clr(self, modulation: Color) -> Color {
        let mul = |a: u8, b: u8| -> u8 { ((a as u16 * b as u16) >> 8) as u8 };
        Color {
            r: mul(self.r, modulation.r),
            g: mul(self.g, modulation.g),
            b: mul(self.b, modulation.b),
            a: self.a.saturating_sub(modulation.a),
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

    /// Live StdGL `LC_MOD2` source preparation. The fragment shader combines
    /// texture and modulation RGB as ADD_SIGNED*2, while deliberately leaving
    /// texture opacity untouched (`src/StdGL.cpp:1071-1076`). This differs by
    /// one RGB unit and by alpha semantics from the packed-color
    /// [`Self::modulate_clr_mod2`] helper used by software surface reads.
    pub(crate) fn modulate_rgb_mod2(self, modulation: Color) -> Color {
        let channel = |source: u8, modulation: u8| -> u8 {
            (2 * i32::from(source) + 2 * i32::from(modulation) - 255)
                .clamp(0, 255) as u8
        };
        Color {
            r: channel(self.r, modulation.r),
            g: channel(self.g, modulation.g),
            b: channel(self.b, modulation.b),
            a: self.a,
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

    /// Framebuffer blend used after the live blit shader. Shader RGB remains
    /// normalized until the RGBA8 store, which rounds the final blend instead
    /// of truncating the individual integer products.
    pub(crate) fn blend_shader_over(self, dest: Color) -> Color {
        let alpha = self.a as u16;
        if alpha == 0 {
            return dest;
        }
        if alpha == 255 {
            return self;
        }

        let inv_alpha = 255 - alpha;
        let blend = |source: u8, destination: u8| -> u8 {
            let numerator = source as u16 * alpha + destination as u16 * inv_alpha;
            ((numerator + 127) / 255) as u8
        };
        Color {
            r: blend(self.r, dest.r),
            g: blend(self.g, dest.g),
            b: blend(self.b, dest.b),
            // Match the renderer's established alpha-channel model; only RGB
            // stays in shader precision through the framebuffer blend.
            a: (alpha + (dest.a as u16 * inv_alpha) / 255).min(255) as u8,
        }
    }

    /// Additive counterpart of [`Self::blend_shader_over`], matching the
    /// rounded RGBA8 store after `GL_SRC_ALPHA, GL_ONE` RGB blending.
    pub(crate) fn blend_shader_additive(self, dest: Color) -> Color {
        let alpha = self.a as u16;
        let add = |source: u8, destination: u8| -> u8 {
            let contribution = (source as u16 * alpha + 127) / 255;
            (destination as u16 + contribution).min(255) as u8
        };
        Color {
            r: add(self.r, dest.r),
            g: add(self.g, dest.g),
            b: add(self.b, dest.b),
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
    fn modulate_clr_matches_c4_blit_channel_math() {
        // RGB = (a*b)>>8, exactly as C++ modulation channel math.
        // White-by-half-grey: (200*128)>>8 = 100.
        assert_eq!(
            Color::new(200, 200, 200, 255).modulate_clr(Color::new(128, 128, 128, 0)),
            Color::new(100, 100, 100, 255)
        );
        // Parity-critical quirk: white·white = (255*255)>>8 = 254 per channel,
        // NOT 255. A zero C4 transparency byte leaves opacity unchanged.
        assert_eq!(
            Color::opaque(255, 255, 255).modulate_clr(Color::new(255, 255, 255, 0)),
            Color::new(254, 254, 254, 255)
        );
        // Identity-ish: modulating by white leaves RGB nearly unchanged but shows
        // the >>8 rounding (254 not 255 for full channels).
        let m = Color::new(255, 0, 128, 255).modulate_clr(Color::new(255, 255, 255, 0));
        assert_eq!(m, Color::new(254, 0, 127, 255));
    }

    #[test]
    fn modulate_clr_converts_c4_transparency_to_opacity() {
        for source_alpha in u8::MIN..=u8::MAX {
            for modulation_alpha in u8::MIN..=u8::MAX {
                let actual = Color::new(40, 80, 120, source_alpha).modulate_clr(Color::new(
                    255,
                    255,
                    255,
                    modulation_alpha,
                ));
                assert_eq!(
                    actual.a,
                    source_alpha.saturating_sub(modulation_alpha),
                    "source opacity {source_alpha}, C4 transparency {modulation_alpha}"
                );
            }
        }
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
