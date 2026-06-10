//! The engine gamma ramp (C++ `CGammaControl`, StdDDraw2.{h,cpp}).
//!
//! The C++ engine pushes a 256-entry 16-bit per-channel ramp into 1D textures
//! and applies it to every fragment in the blit shader
//! (`StdGL.cpp:1082-1086`, enabled by default via `UseShaderGamma`,
//! C4Config.cpp:504). With the default control points (0x000000, 0x808080,
//! 0xffffff; `CGammaControl::Default`, StdDDraw2.h:172) the ramp is identity
//! except that the `MinGamma = 0x100` clamp (StdDDraw2.cpp:240) lifts input 0
//! to output 1.

use crate::Surface;

/// One 8-bit-in / 8-bit-out gamma lookup per colour channel, plus the float
/// fragment-shader semantics for filtered (non-integer) colour values.
#[derive(Clone, Debug, PartialEq)]
pub struct GammaRamp {
    red: [u8; 256],
    green: [u8; 256],
    blue: [u8; 256],
    ramp16: [u16; 256],
}

/// Builds one channel's 16-bit ramp from three control values, then converts
/// to 8-bit framebuffer output.
///
/// Mirrors `CGammaControl::SetClrChannel` (StdDDraw2.cpp:237-271) with
/// size=256 and no ref ramp; the shader samples the 16-bit value and the
/// 8-bit framebuffer store rounds `v * 255 / 65535` = `v / 257`.
fn ramp_channel(c1: u8, c2: u8, c3: u8) -> ([u8; 256], [u16; 256]) {
    let (c1, c2) = (i64::from(c1), i64::from(c2));
    let c3 = i64::from(c3) + 1; // "adjust clr3-value" (StdDDraw2.cpp:242)
    let size: i64 = 256;
    let size1 = size - 1;
    let denom = size1 * size1 / 512;
    let mut ramp16 = [0u16; 256];
    for i in 0..size / 2 {
        let i2 = size / 2 - i;
        let bend = (2 * c2 - c1 - c3) * 2 * i * i2;
        // MinGamma clamp per SetClrChannel (StdDDraw2.cpp:240,267).
        ramp16[i as usize] =
            (((c1 * i2 + c2 * i) * size1 + bend) / denom).clamp(0x100, 0xffff) as u16;
        ramp16[(i + size / 2) as usize] =
            (((c2 * i2 + c3 * i) * size1 + bend) / denom).clamp(0x100, 0xffff) as u16;
    }
    let mut out = [0u8; 256];
    for (entry, v) in out.iter_mut().zip(ramp16) {
        // 16-bit ramp value -> 8-bit framebuffer store rounding.
        *entry = (f32::from(v) / 257.0).round() as u8;
    }
    (out, ramp16)
}

impl GammaRamp {
    /// The default ramp (control points 0x000000, 0x808080, 0xffffff).
    ///
    /// Note the C++ feeds channels through `GetBValue`/`GetRValue` swapped
    /// (StdDDraw2.cpp:284-286); with the default grey control points every
    /// channel is identical, so the swap is unobservable here.
    pub fn standard() -> Self {
        let (lut, ramp16) = ramp_channel(0x00, 0x80, 0xff);
        Self {
            red: lut,
            green: lut,
            blue: lut,
            ramp16,
        }
    }

    /// Applies the ramp to a *filtered* (fractional) colour value the way the
    /// blit shader does: the 1D gamma texture is sampled with GL_NEAREST
    /// (StdGL.cpp:1254), so the texel index is `floor(c * 256)` of the
    /// normalized fragment colour `x/255` — half-values round down below 127
    /// and up above — then the 16-bit texel is stored to the 8-bit
    /// framebuffer with rounding.
    pub fn encode_float(&self, x: f32) -> u8 {
        let index = ((x.clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255);
        (f32::from(self.ramp16[index]) / 257.0).round() as u8
    }

    /// Applies the ramp to every pixel of `surface`, like the blit shader
    /// does per fragment.
    pub fn apply_to_surface(&self, surface: &mut Surface) {
        for px in surface.pixels_mut().chunks_exact_mut(4) {
            px[0] = self.red[px[0] as usize];
            px[1] = self.green[px[1] as usize];
            px[2] = self.blue[px[2] as usize];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PixelFormat;

    #[test]
    fn standard_ramp_is_identity_except_black_floor() {
        let ramp = GammaRamp::standard();
        // MinGamma 0x100 lifts black: round(256/257) = 1.
        assert_eq!(ramp.red[0], 1);
        // Everything else is identity for the default control points.
        for c in [1usize, 63, 64, 127, 128, 129, 200, 254, 255] {
            assert_eq!(ramp.red[c], c as u8, "channel value {c}");
        }
        assert_eq!(ramp.red, ramp.green);
        assert_eq!(ramp.green, ramp.blue);
    }

    #[test]
    fn encode_float_floors_via_nearest_texel_lookup() {
        let ramp = GammaRamp::standard();
        // Integer values stay identity (with the black floor).
        assert_eq!(ramp.encode_float(0.0), 1);
        assert_eq!(ramp.encode_float(64.0), 64);
        assert_eq!(ramp.encode_float(255.0), 255);
        // Half-values: floor(x*256/255) rounds down below 127...
        assert_eq!(ramp.encode_float(10.5), 10);
        assert_eq!(ramp.encode_float(126.5), 126);
        // ...and up from 127.5 (127.5*256/255 = 128.0).
        assert_eq!(ramp.encode_float(127.5), 128);
        assert_eq!(ramp.encode_float(200.5), 201);
    }

    #[test]
    fn apply_to_surface_lifts_black_and_keeps_alpha() {
        let ramp = GammaRamp::standard();
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface
            .set_pixel(0, 0, crate::Color::new(0, 128, 255, 7))
            .unwrap();
        ramp.apply_to_surface(&mut surface);
        assert_eq!(
            surface.get_pixel(0, 0),
            Some(crate::Color::new(1, 128, 255, 7))
        );
    }
}
