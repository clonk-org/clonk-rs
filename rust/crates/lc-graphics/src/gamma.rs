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

/// A colour component of the C++ gamma ramp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GammaChannel {
    Red,
    Green,
    Blue,
}

impl GammaChannel {
    const fn index(self) -> usize {
        match self {
            Self::Red => 0,
            Self::Green => 1,
            Self::Blue => 2,
        }
    }
}

/// Three exact 16-bit gamma lookups, plus the 8-bit framebuffer and filtered
/// fragment-shader encoders derived from them.
#[derive(Clone, Debug, PartialEq)]
pub struct GammaRamp {
    channels: [[u16; 256]; 3],
    /// Mathematical identity is also the sentinel for bypassing the shader's
    /// nearest-sampled 1D lookup. A literal identity texture would quantize
    /// filtered fractional RGB before blending, unlike the native fixed path.
    passthrough: bool,
}

/// Builds the uncorrected curve for one channel from three control values.
///
/// Mirrors `CGammaControl::SetClrChannel` (StdDDraw2.cpp:237-271) with
/// size=256 and no ref ramp; the shader samples the 16-bit value and the
/// 8-bit framebuffer store rounds `v * 255 / 65535` = `v / 257`.
fn raw_ramp_channel(c1: u8, c2: u8, c3: u8) -> [u16; 256] {
    let (c1, c2) = (i64::from(c1), i64::from(c2));
    let c3 = i64::from(c3) + 1; // "adjust clr3-value" (StdDDraw2.cpp:242)
    let size: i64 = 256;
    let size1 = size - 1;
    let denom = size1 * size1 / 512;
    let mut ramp16 = [0u16; 256];
    for i in 0..size / 2 {
        let i2 = size / 2 - i;
        let bend = (2 * c2 - c1 - c3) * 2 * i * i2;
        ramp16[i as usize] = (((c1 * i2 + c2 * i) * size1 + bend) / denom).clamp(0, 0xffff) as u16;
        ramp16[(i + size / 2) as usize] =
            (((c2 * i2 + c3 * i) * size1 + bend) / denom).clamp(0, 0xffff) as u16;
    }
    ramp16
}

fn ramp_channel(c1: u8, c2: u8, c3: u8) -> [u16; 256] {
    let raw = raw_ramp_channel(c1, c2, c3);
    let mut reference = raw_ramp_channel(0x00, 0x80, 0xff);
    reference
        .iter_mut()
        .for_each(|value| *value = (*value).max(0x100));

    std::array::from_fn(|i| {
        // `CStdDDraw::SetGamma` supplies `DefRamp` as `ref`. C++ applies this
        // correction before the MinGamma clamp (StdDDraw2.cpp:254-267).
        let linear = 0x10000_i64 * i as i64 / 255;
        (i64::from(raw[i]) + i64::from(reference[i]) - linear).clamp(0x100, 0xffff) as u16
    })
}

const fn framebuffer_channel(value: u16) -> u8 {
    // A normalized 16-bit texture sample written to an 8-bit framebuffer is
    // rounded to the nearest of the 256 representable values.
    ((value as u32 + 128) / 257) as u8
}

impl GammaRamp {
    /// Whether this ramp is the continuous identity sentinel used when
    /// native gamma correction is disabled or belongs to a later monitor
    /// postpass. Retained command recorders use this to avoid quantizing
    /// fractional fragment colours through a literal identity lookup.
    pub const fn is_passthrough(&self) -> bool {
        self.passthrough
    }

    /// Copy the exact lookup table for a retained graphics backend.
    pub fn channels(&self) -> [[u16; 256]; 3] {
        self.channels
    }

    /// Stable content revision used by the retained gamma texture cache.
    pub fn gpu_revision(&self) -> u64 {
        let mut hash = crate::snapshot::FNV_OFFSET;
        for channel in &self.channels {
            for value in channel {
                hash = crate::snapshot::checksum_update(hash, &value.to_le_bytes());
            }
        }
        u64::from(hash)
    }

    /// A byte-for-byte linear ramp used when native gamma correction is
    /// disabled. Passing this through an in-process renderer is visibly
    /// equivalent to C++ skipping both its shader lookup and monitor ramp,
    /// without mutating process-global display state.
    pub fn identity() -> Self {
        let channel = std::array::from_fn(|index| (index as u16) * 257);
        Self {
            channels: [channel; 3],
            passthrough: true,
        }
    }

    /// The default ramp (control points 0x000000, 0x808080, 0xffffff).
    ///
    /// Note the C++ feeds channels through `GetBValue`/`GetRValue` swapped
    /// (StdDDraw2.cpp:284-286); with the default grey control points every
    /// channel is identical, so the swap is unobservable here.
    pub fn standard() -> Self {
        Self::from_control_points([0x000000, 0x808080, 0xffffff])
    }

    /// Builds the exact three-channel C++ ramp from packed `0xRRGGBB` control
    /// points (`CGammaControl::Set`, StdDDraw2.cpp:273-286).
    ///
    /// The apparently reversed extractors are intentional: LegacyClonk's
    /// `GetBValue` reads the high byte and feeds the red table, while
    /// `GetRValue` reads the low byte and feeds the blue table.
    pub fn from_control_points(control_points: [u32; 3]) -> Self {
        let [c1, c2, c3] = control_points;
        Self {
            channels: [
                ramp_channel((c1 >> 16) as u8, (c2 >> 16) as u8, (c3 >> 16) as u8),
                ramp_channel((c1 >> 8) as u8, (c2 >> 8) as u8, (c3 >> 8) as u8),
                ramp_channel(c1 as u8, c2 as u8, c3 as u8),
            ],
            passthrough: false,
        }
    }

    /// Returns the exact normalized 16-bit texture entry for an integer input.
    pub fn encode_channel_u16(&self, channel: GammaChannel, x: u8) -> u16 {
        self.channels[channel.index()][usize::from(x)]
    }

    /// Applies one channel's ramp to an integer input and converts the texture
    /// sample to its rounded 8-bit framebuffer representation.
    pub fn encode_channel(&self, channel: GammaChannel, x: u8) -> u8 {
        framebuffer_channel(self.encode_channel_u16(channel, x))
    }

    /// Applies one channel's ramp to a filtered (fractional) colour value with
    /// the shader's `GL_NEAREST` texture-coordinate semantics.
    pub fn encode_channel_float(&self, channel: GammaChannel, x: f32) -> u8 {
        if self.passthrough {
            return x.round().clamp(0.0, 255.0) as u8;
        }
        let index = Self::sample_index(x);
        framebuffer_channel(self.channels[channel.index()][index])
    }

    /// Samples one normalized R16 gamma texel as a framebuffer-scale float.
    ///
    /// The C++ shader consumes `GL_R16` through a normalized sampler and the
    /// blend stage receives that unrounded float (`StdGL.cpp:1081-1087,
    /// 1246-1255`). Callers performing alpha or additive blending must keep
    /// this value in float and round only when storing the final pixel.
    #[inline]
    pub fn sample_channel_float(&self, channel: GammaChannel, x: f32) -> f32 {
        if self.passthrough {
            return x.clamp(0.0, 255.0);
        }
        let index = Self::sample_index(x);
        // `u16 / 65535 * 255` is exactly `u16 / 257`.
        f32::from(self.channels[channel.index()][index]) / 257.0
    }

    #[inline]
    fn sample_index(x: f32) -> usize {
        ((x.clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255)
    }

    /// Applies the ramp to a *filtered* (fractional) colour value the way the
    /// blit shader does: the 1D gamma texture is sampled with GL_NEAREST
    /// (StdGL.cpp:1254), so the texel index is `floor(c * 256)` of the
    /// normalized fragment colour `x/255` — half-values round down below 127
    /// and up above — then the 16-bit texel is stored to the 8-bit
    /// framebuffer with rounding.
    pub fn encode_float(&self, x: f32) -> u8 {
        self.encode_channel_float(GammaChannel::Red, x)
    }

    /// Applies the ramp to every complete RGBA pixel, preserving alpha.
    pub fn apply_to_rgba_bytes(&self, pixels: &mut [u8]) {
        for px in pixels.chunks_exact_mut(4) {
            px[0] = self.encode_channel(GammaChannel::Red, px[0]);
            px[1] = self.encode_channel(GammaChannel::Green, px[1]);
            px[2] = self.encode_channel(GammaChannel::Blue, px[2]);
        }
    }

    /// Applies the ramp to every pixel of `surface`, like a monitor ramp
    /// becoming visible after the complete framebuffer has been composed.
    pub fn apply_to_surface(&self, surface: &mut Surface) {
        self.apply_to_rgba_bytes(surface.pixels_mut());
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
        assert_eq!(ramp.encode_channel(GammaChannel::Red, 0), 1);
        // Everything else is identity for the default control points.
        for c in [1usize, 63, 64, 127, 128, 129, 200, 254, 255] {
            assert_eq!(
                ramp.encode_channel(GammaChannel::Red, c as u8),
                c as u8,
                "channel value {c}"
            );
        }
        assert_eq!(ramp.channels[0], ramp.channels[1]);
        assert_eq!(ramp.channels[1], ramp.channels[2]);
    }

    #[test]
    fn tutorial_six_ramp_preserves_exact_cpp_16_bit_samples() {
        // `CGammaControl::Set` + `SetClrChannel` (StdDDraw2.cpp:237-286).
        let ramp = GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let expected = [
            (0, 0x0100, 1),
            (64, 0x31f1, 50),
            (128, 0x6465, 100),
            (192, 0x96d8, 150),
            (255, 0xc8fc, 200),
        ];

        for (input, raw, framebuffer) in expected {
            for channel in [GammaChannel::Red, GammaChannel::Green, GammaChannel::Blue] {
                assert_eq!(ramp.encode_channel_u16(channel, input), raw);
                assert_eq!(ramp.encode_channel(channel, input), framebuffer);
            }
        }
    }

    #[test]
    fn identity_bypasses_fractional_shader_lookup_quantization() {
        let ramp = GammaRamp::identity();
        assert!(ramp.is_passthrough());
        assert!(!GammaRamp::standard().is_passthrough());
        assert_eq!(ramp.sample_channel_float(GammaChannel::Red, 127.25), 127.25);
        assert_eq!(
            ramp.sample_channel_float(GammaChannel::Green, 200.75),
            200.75
        );
        assert_eq!(ramp.encode_channel_float(GammaChannel::Blue, 127.49), 127);
    }

    #[test]
    fn packed_control_points_feed_the_cpp_red_green_blue_channels() {
        // `GetBValue` reads the high packed byte and `GetRValue` the low byte;
        // `CGammaControl::Set` deliberately uses them for red and blue in that
        // order (StdColors.h:43-45; StdDDraw2.cpp:283-286).
        let ramp = GammaRamp::from_control_points([0x102030, 0x405060, 0x708090]);

        assert_eq!(ramp.encode_channel_u16(GammaChannel::Red, 0), 0x1110);
        assert_eq!(ramp.encode_channel_u16(GammaChannel::Green, 0), 0x2120);
        assert_eq!(ramp.encode_channel_u16(GammaChannel::Blue, 0), 0x3130);
        assert_eq!(ramp.encode_channel_float(GammaChannel::Red, 0.0), 17);
        assert_eq!(ramp.encode_channel_float(GammaChannel::Green, 0.0), 33);
        assert_eq!(ramp.encode_channel_float(GammaChannel::Blue, 0.0), 49);

        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface
            .set_pixel(0, 0, crate::Color::new(0, 0, 0, 7))
            .unwrap();
        ramp.apply_to_surface(&mut surface);
        assert_eq!(
            surface.get_pixel(0, 0),
            Some(crate::Color::new(17, 33, 49, 7))
        );
    }

    #[test]
    fn shader_gamma_encodes_source_before_alpha_blending() {
        // The fragment shader samples gamma before emitting its colour
        // (StdGL.cpp:1081-1087); fixed-function source-alpha blending follows
        // (StdGL.cpp:908). Applying gamma to the composited surface is not
        // equivalent.
        let ramp = GammaRamp::from_control_points([0x000000, 0x646464, 0xc8c8c8]);
        let source = crate::Color::new(64, 128, 192, 128);
        let encoded = crate::Color::new(
            ramp.encode_channel_float(GammaChannel::Red, f32::from(source.r)),
            ramp.encode_channel_float(GammaChannel::Green, f32::from(source.g)),
            ramp.encode_channel_float(GammaChannel::Blue, f32::from(source.b)),
            source.a,
        );
        assert_eq!(encoded, crate::Color::new(50, 100, 150, 128));

        let alpha = f32::from(source.a) / 255.0;
        // GL_R16 is normalized to float in the shader. Fixed-function
        // blending consumes that unrounded sample and the framebuffer rounds
        // only once on store (StdGL.cpp:1081-1087,908; 1250-1255).
        let samples = [
            ramp.sample_channel_float(GammaChannel::Red, f32::from(source.r)),
            ramp.sample_channel_float(GammaChannel::Green, f32::from(source.g)),
            ramp.sample_channel_float(GammaChannel::Blue, f32::from(source.b)),
        ];
        assert!((samples[0] - f32::from(0x31f1_u16) / 257.0).abs() < f32::EPSILON);
        assert!((samples[1] - f32::from(0x6465_u16) / 257.0).abs() < f32::EPSILON);
        assert!((samples[2] - f32::from(0x96d8_u16) / 257.0).abs() < f32::EPSILON);

        let over_opaque_200 =
            |value: f32| (value * alpha + 200.0 * (1.0 - alpha)).round() as u8;
        let pre_blend = crate::Color::new(
            over_opaque_200(samples[0]),
            over_opaque_200(samples[1]),
            over_opaque_200(samples[2]),
            255,
        );
        assert_eq!(pre_blend, crate::Color::new(125, 150, 175, 255));

        let raw_blend = crate::Color::new(
            over_opaque_200(f32::from(source.r)),
            over_opaque_200(f32::from(source.g)),
            over_opaque_200(f32::from(source.b)),
            255,
        );
        let post_blend = crate::Color::new(
            ramp.encode_channel(GammaChannel::Red, raw_blend.r),
            ramp.encode_channel(GammaChannel::Green, raw_blend.g),
            ramp.encode_channel(GammaChannel::Blue, raw_blend.b),
            255,
        );
        assert_ne!(post_blend, pre_blend);
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
