use super::*;

#[cfg(feature = "presentation-profile")]
static SPRITE_SPAN_REFERENCE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Manual profiling only: compare both implementations in the same executable.
#[cfg(feature = "presentation-profile")]
pub fn set_software_sprite_span_reference(reference: bool) {
    SPRITE_SPAN_REFERENCE.store(reference, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) struct SpanCompositor {
    gamma: std::rc::Rc<SpanGamma>,
    legacy_gamma: bool,
}

impl SpanCompositor {
    pub(crate) fn new(gamma: Option<&clonk_graphics::GammaRamp>) -> Option<Self> {
        #[cfg(feature = "presentation-profile")]
        if SPRITE_SPAN_REFERENCE.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        Some(Self {
            gamma: span_gamma(gamma),
            legacy_gamma: gamma.is_some(),
        })
    }

    #[inline]
    pub(crate) fn composite(
        &self,
        fragment: PreparedSpriteFragment,
        destination: Color,
        blit: SpriteBlitState,
    ) -> Color {
        if let PreparedSpriteFragment::Layers { base, overlay } = fragment {
            let destination = self.composite(base.into_fragment(), destination, blit);
            return self.composite(overlay.into_fragment(), destination, blit);
        }
        let additive = blit.mode & C4GFXBLIT_ADDITIVE != 0;
        let (rgb, opacity, legacy) = match fragment {
            PreparedSpriteFragment::Legacy(source) => {
                if !self.legacy_gamma && !additive {
                    return blend_colors(source, destination);
                }
                (
                    [
                        f32::from(source.r),
                        f32::from(source.g),
                        f32::from(source.b),
                    ],
                    f32::from(source.a),
                    true,
                )
            }
            PreparedSpriteFragment::Shader { rgb, alpha } => (rgb, alpha, false),
            PreparedSpriteFragment::Layers { .. } => unreachable!("layers were split"),
        };
        if opacity == 0.0 {
            return destination;
        }
        let rgb = std::array::from_fn(|channel| {
            let value = rgb[channel].clamp(0.0, 255.0);
            if self.gamma.quantized {
                let index = ((value * 256.0 / 255.0) as usize).min(255);
                self.gamma.rgb[channel][index]
            } else {
                value
            }
        });
        let background = [destination.r, destination.g, destination.b, destination.a];
        let mut output = if additive {
            blend_float_span::<true>(rgb, opacity, background)
        } else {
            blend_float_span::<false>(rgb, opacity, background)
        };
        if legacy && !additive {
            let alpha = opacity as u16;
            output[3] = (alpha + u16::from(destination.a) * (255 - alpha) / 255) as u8;
        }
        Color::new(output[0], output[1], output[2], output[3])
    }
}

#[cfg(test)]
fn constant_blit_for_span(
    mut blit: SpriteBlitState,
    sampler: &FogSpriteSampler,
) -> Option<SpriteBlitState> {
    if blit.fog_modulation.is_some() {
        return None;
    }
    let base = blit.modulation.unwrap_or(0x00ff_ffff);
    let combine = |fog| {
        if base == 0 {
            0
        } else {
            modulate_c4_colors(base, fog)
        }
    };
    let modulation = combine(sampler.quads.first()?.modulation[0]);
    if !sampler.quads.iter().all(|quad| {
        quad.modulation
            .iter()
            .all(|fog| combine(*fog) == modulation)
    }) {
        return None;
    }
    // Equal integer vertex channels stay far from a half-integer throughout
    // triangle interpolation. MOD2's decision still belongs to the quad.
    blit.modulation = Some(modulation);
    if modulation == 0 {
        blit.mode &= !C4GFXBLIT_MOD2;
    }
    Some(blit)
}

#[derive(Clone, Copy)]
struct SpanAlpha {
    opacity: u8,
    factor: f32,
    inverse: f32,
}

struct SpanPalette {
    rgb: [[f32; 256]; 3],
    opaque_rgb: [[u8; 256]; 3],
    alpha: [SpanAlpha; 256],
}

struct CachedSpanPalette {
    state: (u32, Option<u32>, AdvancedRendererConfig),
    ramp: Option<clonk_graphics::GammaRamp>,
    data: Arc<SpanPalette>,
}

thread_local! {
    static SPAN_PALETTE_CACHE: std::cell::RefCell<std::collections::VecDeque<CachedSpanPalette>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

fn span_palette(
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
) -> Arc<SpanPalette> {
    let state = (blit.mode, blit.modulation, blit.renderer_config);
    SPAN_PALETTE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache
            .iter()
            .rposition(|entry| entry.state == state && entry.ramp.as_ref() == gamma)
        {
            let entry = cache
                .remove(index)
                .unwrap_or_else(|| unreachable!("cache index exists"));
            let data = entry.data.clone();
            cache.push_back(entry);
            return data;
        }
        let data = Arc::new(SpanPalette::new(blit, gamma));
        if cache.len() == 32 {
            cache.pop_front();
        }
        cache.push_back(CachedSpanPalette {
            state,
            ramp: gamma.cloned(),
            data: data.clone(),
        });
        data
    })
}

fn nearest_gamma_modulation_index(source: u8, modulation: u8) -> u8 {
    ((u32::from(source) * u32::from(modulation) * 256 / 65_025).min(255)) as u8
}

pub(crate) fn blend_black_shader_pair(opacity: [u8; 2], destination: [[u8; 4]; 2]) -> [[u8; 4]; 2] {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        let packed = u64::from(u32::from_le_bytes(destination[0]))
            | (u64::from(u32::from_le_bytes(destination[1])) << 32);
        // RGB is zero before blending. For integral opacity, the odd 255
        // denominator keeps every exact result at least 1/510 from a tie;
        // f32 error cannot change the oracle's rounded byte. Alpha uses the
        // same equation with 255 as its source channel.
        let bytes = unsafe {
            let zero = _mm_setzero_si128();
            let words = _mm_unpacklo_epi8(_mm_cvtsi64_si128(packed as i64), zero);
            let a = i16::from(opacity[0]);
            let b = i16::from(opacity[1]);
            let alpha = _mm_set_epi16(b, b, b, b, a, a, a, a);
            let inverse = _mm_sub_epi16(_mm_set1_epi16(255), alpha);
            let source = _mm_mullo_epi16(alpha, _mm_set_epi16(255, 0, 0, 0, 255, 0, 0, 0));
            let numerator = _mm_add_epi16(
                _mm_add_epi16(source, _mm_mullo_epi16(words, inverse)),
                _mm_set1_epi16(127),
            );
            let quotient = _mm_srli_epi16(
                _mm_add_epi16(
                    _mm_add_epi16(numerator, _mm_set1_epi16(1)),
                    _mm_srli_epi16(numerator, 8),
                ),
                8,
            );
            (_mm_cvtsi128_si64(_mm_packus_epi16(quotient, zero)) as u64).to_le_bytes()
        };
        [
            [bytes[0], bytes[1], bytes[2], bytes[3]],
            [bytes[4], bytes[5], bytes[6], bytes[7]],
        ]
    }
    #[cfg(not(target_arch = "x86_64"))]
    std::array::from_fn(|index| {
        std::array::from_fn(|channel| {
            let alpha = u16::from(opacity[index]);
            let source = if channel == 3 { alpha * 255 } else { 0 };
            ((source + u16::from(destination[index][channel]) * (255 - alpha) + 127) / 255) as u8
        })
    })
}

#[inline]
fn blend_legacy_pair(source: [[u8; 4]; 2], destination: [[u8; 4]; 2]) -> [[u8; 4]; 2] {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        let packed = |pixels: [[u8; 4]; 2]| {
            u64::from(u32::from_le_bytes(pixels[0]))
                | (u64::from(u32::from_le_bytes(pixels[1])) << 32)
        };
        // x86-64 guarantees SSE2. Both inputs fit in the low eight bytes;
        // arithmetic stays below 65536 and no pointers cross this boundary.
        let bytes = unsafe {
            let zero = _mm_setzero_si128();
            let source_words = _mm_unpacklo_epi8(_mm_cvtsi64_si128(packed(source) as i64), zero);
            let destination_words =
                _mm_unpacklo_epi8(_mm_cvtsi64_si128(packed(destination) as i64), zero);
            let alpha = _mm_shufflehi_epi16(_mm_shufflelo_epi16(source_words, 0xff), 0xff);
            let inverse = _mm_sub_epi16(_mm_set1_epi16(255), alpha);
            let numerator = _mm_add_epi16(
                _mm_mullo_epi16(source_words, alpha),
                _mm_mullo_epi16(destination_words, inverse),
            );
            // Exact floor(n/255) for 0 <= n <= 65025, in eight u16 lanes.
            let quotient = _mm_srli_epi16(
                _mm_add_epi16(
                    _mm_add_epi16(numerator, _mm_set1_epi16(1)),
                    _mm_srli_epi16(numerator, 8),
                ),
                8,
            );
            (_mm_cvtsi128_si64(_mm_packus_epi16(quotient, zero)) as u64).to_le_bytes()
        };
        let mut result = [
            [bytes[0], bytes[1], bytes[2], bytes[3]],
            [bytes[4], bytes[5], bytes[6], bytes[7]],
        ];
        for index in 0..2 {
            let alpha = u16::from(source[index][3]);
            result[index][3] =
                (alpha + u16::from(destination[index][3]) * (255 - alpha) / 255) as u8;
        }
        result
    }
    #[cfg(not(target_arch = "x86_64"))]
    std::array::from_fn(|index| {
        let source = source[index];
        let destination = destination[index];
        let color = blend_colors(
            Color::new(source[0], source[1], source[2], source[3]),
            Color::new(
                destination[0],
                destination[1],
                destination[2],
                destination[3],
            ),
        );
        [color.r, color.g, color.b, color.a]
    })
}

impl SpanPalette {
    fn new(blit: SpriteBlitState, gamma: Option<&clonk_graphics::GammaRamp>) -> Self {
        let mut result = Self {
            rgb: [[0.0; 256]; 3],
            opaque_rgb: [[0; 256]; 3],
            alpha: [SpanAlpha {
                opacity: 0,
                factor: 0.0,
                inverse: 1.0,
            }; 256],
        };
        use clonk_graphics::gamma::GammaChannel::{Blue, Green, Red};
        for value in 0..256 {
            let color = Color::new(value as u8, value as u8, value as u8, value as u8);
            let fragment = prepare_sprite_fragment(color, None, None, blit);
            let (rgb, opacity) = match fragment {
                PreparedSpriteFragment::Legacy(color) => ([f32::from(color.r); 3], color.a),
                PreparedSpriteFragment::Shader { rgb, alpha } => (rgb, alpha as u8),
                PreparedSpriteFragment::Layers { .. } => unreachable!("owner layers are excluded"),
            };
            for (channel, gamma_channel) in [Red, Green, Blue].into_iter().enumerate() {
                let encoded = sample_channel(gamma, gamma_channel, rgb[channel]);
                result.rgb[channel][value] = encoded;
                result.opaque_rgb[channel][value] = store_channel(encoded);
            }
            let factor = f32::from(opacity) / 255.0;
            result.alpha[value] = SpanAlpha {
                opacity,
                factor,
                inverse: 1.0 - factor,
            };
        }
        result
    }
}

/// Resolve clipping and nearest source coordinates before entering a row.
/// Shader modulation and gamma are one 256-entry palette per draw; fragments
/// retain the original float blend and RGBA8 rounding only when translucent.
pub(crate) fn draw_nearest_sprite_span(
    surface: &mut Surface,
    image: &ImageData,
    source: &FloatSourceRect,
    destination: SurfaceRect,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    flip_x: bool,
    fog: Option<(&FogSpriteSampler, &GuiRect)>,
) -> bool {
    #[cfg(feature = "presentation-profile")]
    if SPRITE_SPAN_REFERENCE.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    if !source.is_valid()
        // Larger images can end in a smaller native texture that emits no
        // blit chunks. Keep their physical-tile handling on the scalar path.
        || image.width() > 4_096 || image.height() > 4_096
        || u64::from(image.width())
            .checked_mul(u64::from(image.height()))
            .and_then(|len| len.checked_mul(4))
            != Some(image.pixels().len() as u64)
        || destination.width == 0
        || destination.height == 0
        || destination.width > i32::MAX as u32
        || destination.height > i32::MAX as u32
        || destination.x.checked_add(destination.width as i32).is_none()
        || destination.y.checked_add(destination.height as i32).is_none()
        || source.x < 0.0
        || source.y < 0.0
        || source.x + source.width > image.width() as f32
        || source.y + source.height > image.height() as f32
        || blit.fog_modulation.is_some()
        || blit.renderer_config.has_adjusted_quad_geometry()
        || surface.is_gpu_scene_capture_active()
    {
        return false;
    }
    let Some(region) = destination
        .intersection(surface.bounds())
        .and_then(|region| {
            surface
                .clip()
                .map_or(Some(region), |clip| region.intersection(clip))
        })
    else {
        return true;
    };
    let source_x = (region.x..region.x + region.width as i32)
        .map(|x| {
            let normalized = ((x - destination.x) as f32 + 0.5) / destination.width as f32;
            let (x, _) = source.source_edge(normalized, 0.0, flip_x);
            (x >= 0.0 && x < image.width() as f32).then(|| x.floor() as usize * 4)
        })
        .collect::<Option<Vec<_>>>();
    let Some(source_x) = source_x else {
        return false;
    };
    for y in [region.y, region.y + region.height as i32 - 1] {
        let normalized = ((y - destination.y) as f32 + 0.5) / destination.height as f32;
        let (_, source_y) = source.source_edge(0.0, normalized, false);
        if source_y < 0.0 || source_y >= image.height() as f32 {
            return false;
        }
    }
    if let Some((sampler, rect)) = fog {
        return if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            rasterize_fog_span::<true>(
                surface,
                image,
                source,
                destination,
                region,
                &source_x,
                blit,
                gamma,
                sampler,
                rect,
            )
        } else {
            rasterize_fog_span::<false>(
                surface,
                image,
                source,
                destination,
                region,
                &source_x,
                blit,
                gamma,
                sampler,
                rect,
            )
        };
    }
    let shader = blit.modulation.is_some() || blit.mode & C4GFXBLIT_MOD2 != 0;
    if !shader && gamma.is_none() && blit.mode & C4GFXBLIT_ADDITIVE == 0 {
        return surface.rasterize_rgba_rows(region, |_, y, destination_pixels| {
            let normalized = ((y as i32 - destination.y) as f32 + 0.5) / destination.height as f32;
            let (_, source_y) = source.source_edge(0.0, normalized, false);
            let row = source_y.floor() as usize * image.width() as usize * 4;
            let source_pixel = |column: usize| {
                let offset = row + source_x[column];
                let rgba = &image.pixels()[offset..offset + 4];
                [rgba[0], rgba[1], rgba[2], rgba[3]]
            };
            let (pairs, tail) = destination_pixels.as_chunks_mut::<2>();
            for (index, pair) in pairs.iter_mut().enumerate() {
                *pair = blend_legacy_pair(
                    [source_pixel(index * 2), source_pixel(index * 2 + 1)],
                    *pair,
                );
            }
            if let Some(pixel) = tail.first_mut() {
                let rgba = source_pixel(pairs.len() * 2);
                let color = blend_colors(
                    Color::new(rgba[0], rgba[1], rgba[2], rgba[3]),
                    Color::new(pixel[0], pixel[1], pixel[2], pixel[3]),
                );
                *pixel = [color.r, color.g, color.b, color.a];
            }
        });
    }
    let palette = span_palette(blit, gamma);
    if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
        rasterize_palette::<true>(
            surface,
            image,
            source,
            destination,
            region,
            &source_x,
            &palette,
            shader,
        )
    } else {
        rasterize_palette::<false>(
            surface,
            image,
            source,
            destination,
            region,
            &source_x,
            &palette,
            shader,
        )
    }
}

fn rasterize_palette<const ADDITIVE: bool>(
    surface: &mut Surface,
    image: &ImageData,
    source: &FloatSourceRect,
    destination: SurfaceRect,
    region: SurfaceRect,
    source_x: &[usize],
    palette: &SpanPalette,
    shader: bool,
) -> bool {
    surface.rasterize_rgba_rows(region, |_, y, destination_pixels| {
        let normalized = ((y as i32 - destination.y) as f32 + 0.5) / destination.height as f32;
        let (_, source_y) = source.source_edge(0.0, normalized, false);
        let row = source_y.floor() as usize * image.width() as usize * 4;
        for (pixel, offset) in destination_pixels.iter_mut().zip(source_x) {
            let rgba = &image.pixels()[row + offset..row + offset + 4];
            let alpha = palette.alpha[rgba[3] as usize];
            if alpha.opacity == 0 {
                continue;
            }
            if alpha.opacity == 255 {
                for channel in 0..3 {
                    let color = palette.opaque_rgb[channel][rgba[channel] as usize];
                    pixel[channel] = if ADDITIVE {
                        pixel[channel].saturating_add(color)
                    } else {
                        color
                    };
                }
                if !ADDITIVE {
                    pixel[3] = 255;
                }
                continue;
            }
            let rgb = std::array::from_fn(|channel| palette.rgb[channel][rgba[channel] as usize]);
            let old_alpha = pixel[3];
            *pixel = blend_shader_span::<ADDITIVE>(rgb, alpha, *pixel);
            if !ADDITIVE && !shader {
                pixel[3] = (u16::from(alpha.opacity)
                    + u16::from(old_alpha) * (255 - u16::from(alpha.opacity)) / 255)
                    as u8;
            }
        }
    })
}

#[derive(Clone)]
struct SpanGamma {
    quantized: bool,
    rgb: [[f32; 256]; 3],
    opaque: [[u8; 256]; 3],
}

struct CachedSpanGamma {
    ramp: Option<clonk_graphics::GammaRamp>,
    data: std::rc::Rc<SpanGamma>,
}

thread_local! {
    static SPAN_GAMMA_CACHE: std::cell::RefCell<std::collections::VecDeque<CachedSpanGamma>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

fn span_gamma(gamma: Option<&clonk_graphics::GammaRamp>) -> std::rc::Rc<SpanGamma> {
    SPAN_GAMMA_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(index) = cache.iter().rposition(|entry| entry.ramp.as_ref() == gamma) {
            // Retain complete ramp content, including the continuous identity
            // sentinel. A new ramp at an old address cannot reuse stale colors.
            let entry = cache
                .remove(index)
                .unwrap_or_else(|| unreachable!("cache index exists"));
            let result = entry.data.clone();
            cache.push_back(entry);
            return result;
        }
        let mut result = SpanGamma {
            quantized: gamma.is_some_and(|gamma| !gamma.is_passthrough()),
            rgb: [[0.0; 256]; 3],
            opaque: [[0; 256]; 3],
        };
        use clonk_graphics::gamma::GammaChannel::{Blue, Green, Red};
        for (channel, gamma_channel) in [Red, Green, Blue].into_iter().enumerate() {
            for value in 0..256 {
                let encoded = sample_channel(gamma, gamma_channel, value as f32);
                result.rgb[channel][value] = encoded;
                result.opaque[channel][value] = store_channel(encoded);
            }
        }
        let result = std::rc::Rc::new(result);
        if cache.len() == 4 {
            cache.pop_front();
        }
        cache.push_back(CachedSpanGamma {
            ramp: gamma.cloned(),
            data: result.clone(),
        });
        result
    })
}

fn modulation_indices() -> &'static [u8] {
    static INDICES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    INDICES.get_or_init(|| {
        (0..65_536)
            .map(|index| nearest_gamma_modulation_index(index as u8, (index >> 8) as u8))
            .collect()
    })
}

fn span_alphas() -> &'static [SpanAlpha; 256] {
    static ALPHAS: std::sync::OnceLock<[SpanAlpha; 256]> = std::sync::OnceLock::new();
    ALPHAS.get_or_init(|| {
        std::array::from_fn(|opacity| {
            let factor = opacity as f32 / 255.0;
            SpanAlpha {
                opacity: opacity as u8,
                factor,
                inverse: 1.0 - factor,
            }
        })
    })
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn rounded_packed_channels(value: std::arch::x86_64::__m128) -> u32 {
    use std::arch::x86_64::*;
    // Positive, bounded shader channels. Truncate and compare the exact
    // fractional part: adding 0.5 can misround a value just below a tie.
    unsafe {
        let value = _mm_min_ps(_mm_max_ps(value, _mm_setzero_ps()), _mm_set1_ps(255.0));
        let integer = _mm_cvttps_epi32(value);
        let fraction = _mm_sub_ps(value, _mm_cvtepi32_ps(integer));
        let increment = _mm_and_si128(
            _mm_castps_si128(_mm_cmpge_ps(fraction, _mm_set1_ps(0.5))),
            _mm_set1_epi32(1),
        );
        let rounded = _mm_add_epi32(integer, increment);
        _mm_cvtsi128_si32(_mm_packus_epi16(
            _mm_packs_epi32(rounded, _mm_setzero_si128()),
            _mm_setzero_si128(),
        )) as u32
    }
}

#[inline]
fn interpolate_span_modulation(quad: &PreparedFogSpanQuad, weights: [f32; 4]) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SSE2 evaluates the same four products and ordered additions as the
        // scalar oracle; it does not reassociate or fuse the float operations.
        unsafe {
            let mut value = _mm_setzero_ps();
            for (channels, weight) in quad.float_colors.into_iter().zip(weights) {
                value = _mm_add_ps(value, _mm_mul_ps(channels, _mm_set1_ps(weight)));
            }
            rounded_packed_channels(value)
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut result = 0;
        for shift in [0, 8, 16, 24] {
            let mut value = 0.0;
            for (color, weight) in quad.colors.into_iter().zip(weights) {
                value += ((color >> shift) & 255) as f32 * weight;
            }
            result |= u32::from(store_channel(value)) << shift;
        }
        result
    }
}

#[inline]
fn blend_shader_span<const ADDITIVE: bool>(
    rgb: [f32; 3],
    alpha: SpanAlpha,
    destination: [u8; 4],
) -> [u8; 4] {
    blend_float_span_with_alpha::<ADDITIVE>(
        rgb,
        f32::from(alpha.opacity),
        alpha.factor,
        alpha.inverse,
        destination,
    )
}

#[inline]
fn blend_float_span<const ADDITIVE: bool>(
    rgb: [f32; 3],
    opacity: f32,
    destination: [u8; 4],
) -> [u8; 4] {
    let factor = (opacity / 255.0).clamp(0.0, 1.0);
    blend_float_span_with_alpha::<ADDITIVE>(rgb, opacity, factor, 1.0 - factor, destination)
}

#[inline]
fn blend_float_span_with_alpha<const ADDITIVE: bool>(
    rgb: [f32; 3],
    opacity: f32,
    factor: f32,
    inverse: f32,
    destination: [u8; 4],
) -> [u8; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // x86-64's baseline SSE2 uses the oracle's f32 precision throughout.
        unsafe {
            let zero = _mm_setzero_si128();
            let destination_words = _mm_unpacklo_epi8(
                _mm_cvtsi32_si128(u32::from_le_bytes(destination) as i32),
                zero,
            );
            let destination_float = _mm_cvtepi32_ps(_mm_unpacklo_epi16(destination_words, zero));
            let source = _mm_mul_ps(_mm_set_ps(0.0, rgb[2], rgb[1], rgb[0]), _mm_set1_ps(factor));
            let output = if ADDITIVE {
                _mm_add_ps(destination_float, source)
            } else {
                let source = _mm_add_ps(source, _mm_set_ps(opacity, 0.0, 0.0, 0.0));
                _mm_add_ps(source, _mm_mul_ps(destination_float, _mm_set1_ps(inverse)))
            };
            rounded_packed_channels(output).to_le_bytes()
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let mut output = destination;
        for channel in 0..3 {
            output[channel] = store_channel(if ADDITIVE {
                f32::from(destination[channel]) + rgb[channel] * factor
            } else {
                rgb[channel] * factor + f32::from(destination[channel]) * inverse
            });
        }
        if !ADDITIVE {
            output[3] = store_channel(opacity + f32::from(destination[3]) * inverse);
        }
        output
    }
}

struct PreparedFogSpanQuad {
    colors: [u32; 4],
    #[cfg(target_arch = "x86_64")]
    float_colors: [std::arch::x86_64::__m128; 4],
    uniform: bool,
    mod2: bool,
    transparent: bool,
    palette: Option<Arc<SpanPalette>>,
}

pub(crate) struct SpanFogBlit {
    blit: SpriteBlitState,
    quads: Vec<PreparedFogSpanQuad>,
}

impl SpanFogBlit {
    pub(crate) fn new(blit: SpriteBlitState, sampler: &FogSpriteSampler) -> Option<Self> {
        #[cfg(feature = "presentation-profile")]
        if SPRITE_SPAN_REFERENCE.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        blit.fog_modulation.is_none().then(|| Self {
            blit,
            quads: prepare_fog_span_quads(blit, sampler),
        })
    }

    #[inline]
    pub(crate) fn at(&self, sampler: &FogSpriteSampler, x: f32, y: f32) -> SpriteBlitState {
        let (x, y) = (sampler.x_axis_at(x), sampler.y_axis_at(y));
        let quad = &self.quads[y.chunk() * sampler.columns + x.chunk()];
        let modulation = if quad.uniform {
            quad.colors[0]
        } else {
            let (u, v) = (x.offset(), y.offset());
            let weights = if u + v <= 1.0 {
                [1.0 - u - v, u, v, 0.0]
            } else {
                [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
            };
            if self.blit.renderer_config.no_box_fades {
                quad.colors[if weights[3] > 0.0 { 3 } else { 2 }]
            } else {
                interpolate_span_modulation(quad, weights)
            }
        };
        // A black fragment can still belong to a nonblack MOD2 quad. The
        // original preparer needs that quad to avoid disabling MOD2 locally.
        if modulation == 0 && quad.mod2 {
            return sampler.blit_at_axes(self.blit, x, y);
        }
        SpriteBlitState {
            modulation: Some(modulation),
            mode: if quad.mod2 {
                self.blit.mode
            } else {
                self.blit.mode & !C4GFXBLIT_MOD2
            },
            ..self.blit
        }
    }
}

pub(crate) struct SpanRowShader {
    blit: SpriteBlitState,
    gamma: SpanGamma,
    columns: usize,
    quads: Vec<PreparedFogSpanQuad>,
    opaque_destination: bool,
}

impl SpanRowShader {
    pub(crate) fn new(
        blit: SpriteBlitState,
        gamma: Option<&clonk_graphics::GammaRamp>,
        sampler: &FogSpriteSampler,
    ) -> Option<Self> {
        #[cfg(feature = "presentation-profile")]
        if SPRITE_SPAN_REFERENCE.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        if blit.fog_modulation.is_some() {
            return None;
        }
        let mut quads = prepare_fog_span_quads(blit, sampler);
        for quad in quads.iter_mut().filter(|quad| quad.uniform) {
            let uniform = SpriteBlitState {
                modulation: Some(quad.colors[0]),
                mode: if quad.mod2 {
                    blit.mode
                } else {
                    blit.mode & !C4GFXBLIT_MOD2
                },
                ..blit
            };
            quad.palette = Some(span_palette(uniform, gamma));
        }
        Some(Self {
            blit,
            gamma: (*span_gamma(gamma)).clone(),
            columns: sampler.columns,
            quads,
            opaque_destination: false,
        })
    }

    pub(crate) fn with_opaque_destination(mut self) -> Self {
        // Sky/ground rows historically evaluate opaque fragments against a
        // transparent destination, including their additive configuration.
        // Preserve that established CPU oracle while specializing the loop.
        self.opaque_destination = true;
        self
    }

    pub(crate) fn opaque_fill(&self, x: FogAxisSample, y: FogAxisSample) -> Option<[u8; 4]> {
        let quad = &self.quads[y.chunk() * self.columns + x.chunk()];
        let preserves_alpha =
            !self.blit.renderer_config.shader && self.blit.renderer_config.no_alpha_add;
        (self.blit.mode & C4GFXBLIT_ADDITIVE == 0
            && quad.uniform
            && !quad.mod2
            && quad.colors[0] & 0x00ff_ffff == 0
            && (quad.colors[0] >> 24 == 0 || preserves_alpha))
            .then(|| {
                [
                    self.gamma.opaque[0][0],
                    self.gamma.opaque[1][0],
                    self.gamma.opaque[2][0],
                    255,
                ]
            })
    }

    #[inline]
    pub(crate) fn black_opacity(&self, x: FogAxisSample, y: FogAxisSample) -> Option<u8> {
        let quad = &self.quads[y.chunk() * self.columns + x.chunk()];
        let preserves_alpha =
            !self.blit.renderer_config.shader && self.blit.renderer_config.no_alpha_add;
        (!self.gamma.quantized
            && self.blit.mode & C4GFXBLIT_ADDITIVE == 0
            && quad.uniform
            && !quad.mod2
            && quad.colors[0] & 0x00ff_ffff == 0)
            .then(|| {
                if preserves_alpha {
                    255
                } else {
                    255 - (quad.colors[0] >> 24) as u8
                }
            })
    }

    #[inline(always)]
    pub(crate) fn shade(
        &self,
        color: Color,
        destination: &mut [u8; 4],
        x: FogAxisSample,
        y: FogAxisSample,
    ) -> u8 {
        if color.a == 0 {
            return 0;
        }
        let quad = &self.quads[y.chunk() * self.columns + x.chunk()];
        if let Some(palette) = quad.palette.as_ref() {
            let opacity = palette.alpha[usize::from(color.a)].opacity;
            let additive = self.blit.mode & C4GFXBLIT_ADDITIVE != 0;
            if self.opaque_destination && additive && opacity == 255 {
                *destination = [0; 4];
            }
            return if additive {
                shade_uniform_palette::<true>(color, destination, palette)
            } else {
                shade_uniform_palette::<false>(color, destination, palette)
            };
        }
        let modulation = if quad.uniform {
            quad.colors[0]
        } else {
            let (u, v) = (x.offset(), y.offset());
            let weights = if u + v <= 1.0 {
                [1.0 - u - v, u, v, 0.0]
            } else {
                [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
            };
            if self.blit.renderer_config.no_box_fades {
                quad.colors[if weights[3] > 0.0 { 3 } else { 2 }]
            } else {
                interpolate_span_modulation(quad, weights)
            }
        };
        let preserve_alpha = if quad.mod2 {
            self.blit.renderer_config.shader
        } else {
            !self.blit.renderer_config.shader && self.blit.renderer_config.no_alpha_add
        };
        let alpha = if preserve_alpha {
            color.a
        } else {
            color.a.saturating_sub((modulation >> 24) as u8)
        };
        if alpha == 0 {
            return 0;
        }
        let additive = self.blit.mode & C4GFXBLIT_ADDITIVE != 0;
        if self.opaque_destination && additive && alpha == 255 {
            *destination = [0; 4];
        }
        if !quad.mod2 && modulation & 0x00ff_ffff == 0 && alpha == 255 {
            for (channel, destination) in destination.iter_mut().take(3).enumerate() {
                let value = self.gamma.opaque[channel][0];
                *destination = if additive {
                    destination.saturating_add(value)
                } else {
                    value
                };
            }
            if !additive {
                destination[3] = 255;
            }
            return alpha;
        }
        let source = [color.r, color.g, color.b, color.a];
        let indices = modulation_indices();
        let alphas = span_alphas();
        match (additive, quad.mod2) {
            (false, false) => shade_fog_span::<false, false>(
                source,
                destination,
                modulation,
                &self.gamma,
                preserve_alpha,
                indices,
                alphas,
            ),
            (false, true) => shade_fog_span::<false, true>(
                source,
                destination,
                modulation,
                &self.gamma,
                preserve_alpha,
                indices,
                alphas,
            ),
            (true, false) => shade_fog_span::<true, false>(
                source,
                destination,
                modulation,
                &self.gamma,
                preserve_alpha,
                indices,
                alphas,
            ),
            (true, true) => shade_fog_span::<true, true>(
                source,
                destination,
                modulation,
                &self.gamma,
                preserve_alpha,
                indices,
                alphas,
            ),
        }
        alpha
    }
}

#[inline]
fn shade_uniform_palette<const ADDITIVE: bool>(
    color: Color,
    destination: &mut [u8; 4],
    palette: &SpanPalette,
) -> u8 {
    let alpha = palette.alpha[usize::from(color.a)];
    if alpha.opacity == 0 {
        return 0;
    }
    let indices = [
        usize::from(color.r),
        usize::from(color.g),
        usize::from(color.b),
    ];
    if alpha.opacity == 255 {
        let rgb =
            std::array::from_fn::<_, 3, _>(|channel| palette.opaque_rgb[channel][indices[channel]]);
        if ADDITIVE {
            for channel in 0..3 {
                destination[channel] = destination[channel].saturating_add(rgb[channel]);
            }
        } else {
            *destination = [rgb[0], rgb[1], rgb[2], 255];
        }
    } else {
        let rgb = std::array::from_fn(|channel| palette.rgb[channel][indices[channel]]);
        *destination = blend_shader_span::<ADDITIVE>(rgb, alpha, *destination);
    }
    alpha.opacity
}

fn prepare_fog_span_quads(
    blit: SpriteBlitState,
    sampler: &FogSpriteSampler,
) -> Vec<PreparedFogSpanQuad> {
    let base = blit.modulation.unwrap_or(0x00ff_ffff);
    sampler
        .quads
        .iter()
        .map(|quad| {
            let colors = quad.modulation.map(|fog| {
                if base == 0 {
                    0
                } else {
                    modulate_c4_colors(base, fog)
                }
            });
            PreparedFogSpanQuad {
                #[cfg(target_arch = "x86_64")]
                float_colors: colors.map(|color| {
                    use std::arch::x86_64::*;
                    unsafe {
                        let zero = _mm_setzero_si128();
                        let words = _mm_unpacklo_epi8(_mm_cvtsi32_si128(color as i32), zero);
                        _mm_cvtepi32_ps(_mm_unpacklo_epi16(words, zero))
                    }
                }),
                uniform: colors.iter().all(|color| *color == colors[0]),
                mod2: blit.mode & C4GFXBLIT_MOD2 != 0 && colors.iter().any(|color| *color != 0),
                transparent: colors.iter().all(|color| color >> 24 == 255),
                palette: None,
                colors,
            }
        })
        .collect()
}

#[inline]
fn shade_fog_span<const ADDITIVE: bool, const MOD2: bool>(
    source: [u8; 4],
    destination: &mut [u8; 4],
    modulation: u32,
    gamma: &SpanGamma,
    preserve_alpha: bool,
    indices: &[u8],
    alphas: &[SpanAlpha; 256],
) {
    let [red, green, blue, transparency] = split_c4_color(modulation);
    let alpha = if preserve_alpha {
        source[3]
    } else {
        source[3].saturating_sub(transparency)
    };
    if alpha == 0 {
        return;
    }
    let modulation = [red, green, blue];
    let index = |channel| {
        if MOD2 {
            (2 * i32::from(source[channel]) + 2 * i32::from(modulation[channel]) - 255)
                .clamp(0, 255) as usize
        } else {
            usize::from(
                indices[usize::from(modulation[channel]) * 256 + usize::from(source[channel])],
            )
        }
    };
    if alpha == 255 {
        for channel in 0..3 {
            let color = if gamma.quantized || MOD2 {
                gamma.opaque[channel][index(channel)]
            } else {
                ((u16::from(source[channel]) * u16::from(modulation[channel]) + 127) / 255) as u8
            };
            destination[channel] = if ADDITIVE {
                destination[channel].saturating_add(color)
            } else {
                color
            };
        }
        if !ADDITIVE {
            destination[3] = 255;
        }
        return;
    }
    let rgb = std::array::from_fn(|channel| {
        if gamma.quantized || MOD2 {
            gamma.rgb[channel][index(channel)]
        } else {
            f32::from(source[channel]) * f32::from(modulation[channel]) / 255.0
        }
    });
    *destination = blend_shader_span::<ADDITIVE>(rgb, alphas[usize::from(alpha)], *destination);
}

fn rasterize_fog_span<const ADDITIVE: bool>(
    surface: &mut Surface,
    image: &ImageData,
    source: &FloatSourceRect,
    destination: SurfaceRect,
    region: SurfaceRect,
    source_x: &[usize],
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    sampler: &FogSpriteSampler,
    rect: &GuiRect,
) -> bool {
    let base = blit.modulation.unwrap_or(0x00ff_ffff);
    let mut quads = sampler
        .quads
        .iter()
        .map(|quad| {
            let colors = quad.modulation.map(|fog| {
                if base == 0 {
                    0
                } else {
                    modulate_c4_colors(base, fog)
                }
            });
            PreparedFogSpanQuad {
                #[cfg(target_arch = "x86_64")]
                float_colors: colors.map(|color| {
                    use std::arch::x86_64::*;
                    // Vertex-byte conversion belongs to the draw, not each
                    // fragment. Zero extension keeps C4 transparency unsigned.
                    unsafe {
                        let zero = _mm_setzero_si128();
                        let words = _mm_unpacklo_epi8(_mm_cvtsi32_si128(color as i32), zero);
                        _mm_cvtepi32_ps(_mm_unpacklo_epi16(words, zero))
                    }
                }),
                uniform: colors.iter().all(|color| *color == colors[0]),
                mod2: blit.mode & C4GFXBLIT_MOD2 != 0 && colors.iter().any(|color| *color != 0),
                transparent: colors.iter().all(|color| color >> 24 == 255),
                palette: None,
                colors,
            }
        })
        .collect::<Vec<_>>();
    for quad in quads.iter_mut().filter(|quad| quad.uniform) {
        quad.palette = Some(span_palette(
            SpriteBlitState {
                modulation: Some(quad.colors[0]),
                mode: if quad.mod2 {
                    blit.mode
                } else {
                    blit.mode & !C4GFXBLIT_MOD2
                },
                ..blit
            },
            gamma,
        ));
    }
    let x_axes = (region.x..region.x + region.width as i32)
        .map(|x| sampler.x_axis_at((x as f32 + 0.5 - rect.origin.x) / rect.size.width))
        .collect::<Vec<_>>();
    let mut runs = Vec::new();
    let mut start = 0;
    while start < x_axes.len() {
        let chunk = x_axes[start].chunk();
        let mut end = start + 1;
        while end < x_axes.len() && x_axes[end].chunk() == chunk {
            end += 1;
        }
        runs.push((start, end, chunk));
        start = end;
    }
    let gamma = span_gamma(gamma);
    let indices = modulation_indices();
    let alphas = span_alphas();
    surface.rasterize_rgba_rows(region, |_, y, pixels| {
        let normalized = ((y as i32 - destination.y) as f32 + 0.5) / destination.height as f32;
        let (_, source_y) = source.source_edge(0.0, normalized, false);
        let row = source_y.floor() as usize * image.width() as usize * 4;
        let y_axis = sampler.y_axis_at((y as f32 + 0.5 - rect.origin.y) / rect.size.height);
        for &(start, end, chunk) in &runs {
            let quad = &quads[y_axis.chunk() * sampler.columns + chunk];
            let preserve_alpha = if quad.mod2 {
                blit.renderer_config.shader
            } else {
                !blit.renderer_config.shader && blit.renderer_config.no_alpha_add
            };
            if quad.transparent && !preserve_alpha {
                continue;
            }
            if let Some(palette) = quad.palette.as_ref() {
                for column in start..end {
                    let offset = row + source_x[column];
                    let rgba = &image.pixels()[offset..offset + 4];
                    shade_uniform_palette::<ADDITIVE>(
                        Color::new(rgba[0], rgba[1], rgba[2], rgba[3]),
                        &mut pixels[column],
                        palette,
                    );
                }
                continue;
            }
            for column in start..end {
                let offset = row + source_x[column];
                let rgba = &image.pixels()[offset..offset + 4];
                if rgba[3] == 0 {
                    continue;
                }
                let modulation = if quad.uniform {
                    quad.colors[0]
                } else {
                    let u = x_axes[column].offset();
                    let v = y_axis.offset();
                    let weights = if u + v <= 1.0 {
                        [1.0 - u - v, u, v, 0.0]
                    } else {
                        [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
                    };
                    if blit.renderer_config.no_box_fades {
                        quad.colors[if weights[3] > 0.0 { 3 } else { 2 }]
                    } else {
                        interpolate_span_modulation(quad, weights)
                    }
                };
                let source = [rgba[0], rgba[1], rgba[2], rgba[3]];
                if quad.mod2 {
                    shade_fog_span::<ADDITIVE, true>(
                        source,
                        &mut pixels[column],
                        modulation,
                        &gamma,
                        preserve_alpha,
                        indices,
                        alphas,
                    );
                } else {
                    shade_fog_span::<ADDITIVE, false>(
                        source,
                        &mut pixels[column],
                        modulation,
                        &gamma,
                        preserve_alpha,
                        indices,
                        alphas,
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_fog_blit_preserves_filtered_shader_channels_and_quad_mod2() {
        // Combine C4 colors at the vertices before interpolation and decide
        // MOD2 from the entire quad (src/StdGL.cpp:471-503).
        let mut sampler = FogSpriteSampler {
            source_width: 8.0,
            source_height: 8.0,
            columns: 1,
            x_ranges: vec![(0.0, 8.0)],
            y_ranges: vec![(0.0, 8.0)],
            quads: vec![FogColorQuad {
                x: (0.0, 8.0),
                y: (0.0, 8.0),
                modulation: [0; 4],
            }],
        };
        for colors in [
            [0; 4],
            [0x4578_9abc; 4],
            [0, 0x45789abc, 0x89def012, 0xcf345678],
            [0x0012_3456, 0x0078_9abc, 0, 0],
        ] {
            sampler.quads[0].modulation = colors;
            for modulation in [None, Some(0), Some(0x7912_9be7)] {
                for mode in 0..4 {
                    for flags in 0..8 {
                        let blit = SpriteBlitState {
                            mode,
                            modulation,
                            renderer_config: AdvancedRendererConfig {
                                shader: flags & 1 != 0,
                                no_alpha_add: flags & 2 != 0,
                                no_box_fades: flags & 4 != 0,
                                ..AdvancedRendererConfig::default()
                            },
                            ..SpriteBlitState::normal()
                        };
                        let prepared = SpanFogBlit::new(blit, &sampler).unwrap();
                        for x in 0..8 {
                            for y in 0..8 {
                                let (x, y) = ((x as f32 + 0.5) / 8.0, (y as f32 + 0.5) / 8.0);
                                let source = [37.125, 129.375, 243.625, 151.875];
                                let expected = prepare_filtered_sprite_fragment(
                                    source,
                                    None,
                                    None,
                                    sampler.blit_at(blit, x, y),
                                );
                                let actual = prepare_filtered_sprite_fragment(
                                    source,
                                    None,
                                    None,
                                    prepared.at(&sampler, x, y),
                                );
                                match (expected, actual) {
                                    (
                                        PreparedSpriteFragment::Shader {
                                            rgb: expected,
                                            alpha: expected_alpha,
                                        },
                                        PreparedSpriteFragment::Shader {
                                            rgb: actual,
                                            alpha: actual_alpha,
                                        },
                                    ) => {
                                        assert_eq!(
                                            expected.map(f32::to_bits),
                                            actual.map(f32::to_bits)
                                        );
                                        assert_eq!(
                                            expected_alpha.to_bits(),
                                            actual_alpha.to_bits()
                                        );
                                    }
                                    _ => panic!("fog fragments must retain shader precision"),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn black_shader_pair_matches_scalar_for_every_opacity_and_destination() {
        // Shader alpha uses GL's rounded non-separate framebuffer equation
        // (src/StdGL.cpp:908,1072-1087).
        for opacity in 0..=255_u8 {
            for value in 0..=255_u8 {
                let destination = Color::new(value, 255 - value, value, 255 - value);
                let expected = [opacity, 255 - opacity].map(|alpha| {
                    let color = composite_sprite_fragment(
                        PreparedSpriteFragment::Shader {
                            rgb: [0.0; 3],
                            alpha: f32::from(alpha),
                        },
                        destination,
                        SpriteBlitState::normal(),
                        None,
                    );
                    [color.r, color.g, color.b, color.a]
                });
                assert_eq!(
                    blend_black_shader_pair(
                        [opacity, 255 - opacity],
                        [[destination.r, destination.g, destination.b, destination.a]; 2]
                    ),
                    expected
                );
            }
        }
    }

    #[test]
    fn uniform_black_fog_can_fill_an_opaque_span_after_gamma() {
        // A black, alpha-preserving modulation quad disables MOD2
        // (src/StdGL.cpp:471-472); gamma follows shader modulation.
        let sampler = FogSpriteSampler {
            source_width: 8.0,
            source_height: 8.0,
            columns: 1,
            x_ranges: vec![(0.0, 8.0)],
            y_ranges: vec![(0.0, 8.0)],
            quads: vec![FogColorQuad {
                x: (0.0, 8.0),
                y: (0.0, 8.0),
                modulation: [0; 4],
            }],
        };
        let gamma = clonk_graphics::GammaRamp::standard();
        let shader = SpanRowShader::new(SpriteBlitState::normal(), Some(&gamma), &sampler).unwrap();
        let (x, y) = (sampler.x_axis_at(0.5), sampler.y_axis_at(0.5));
        assert_eq!(shader.opaque_fill(x, y), Some([1, 1, 1, 255]));
        for color in [
            Color::opaque(0, 0, 0),
            Color::opaque(17, 129, 243),
            Color::opaque(255, 255, 255),
        ] {
            let mut actual = [31, 63, 95, 127];
            shader.shade(color, &mut actual, x, y);
            assert_eq!(Some(actual), shader.opaque_fill(x, y));
        }
    }

    #[test]
    fn row_shader_matches_scalar_fog_for_opaque_and_translucent_texels() {
        // Fog combines vertex colors before interpolation (StdGL.cpp:471-503),
        // followed by the original gamma and framebuffer pipeline.
        let mut sampler = FogSpriteSampler {
            source_width: 8.0,
            source_height: 8.0,
            columns: 1,
            x_ranges: vec![(0.0, 8.0)],
            y_ranges: vec![(0.0, 8.0)],
            quads: vec![FogColorQuad {
                x: (0.0, 8.0),
                y: (0.0, 8.0),
                modulation: [0x00123456, 0x45789abc, 0x89def012, 0xcf345678],
            }],
        };
        let ramps = [
            clonk_graphics::GammaRamp::identity(),
            clonk_graphics::GammaRamp::standard(),
            clonk_graphics::GammaRamp::from_control_points([0x102030, 0x507080, 0xd0e0f0]),
        ];
        for colors in [
            [0; 4],
            [0x00ff_ffff; 4],
            [0x4578_9abc; 4],
            [0xff78_9abc; 4],
            [0x00123456, 0x45789abc, 0x89def012, 0xcf345678],
        ] {
            sampler.quads[0].modulation = colors;
            for gamma in [None, Some(&ramps[0]), Some(&ramps[1]), Some(&ramps[2])] {
                for mode in 0..4 {
                    for flags in 0..8 {
                        for modulation in [None, Some(0), Some(0x7912_9be7)] {
                            let blit = SpriteBlitState {
                                mode,
                                modulation,
                                renderer_config: AdvancedRendererConfig {
                                    shader: flags & 1 != 0,
                                    no_alpha_add: flags & 2 != 0,
                                    no_box_fades: flags & 4 != 0,
                                    ..AdvancedRendererConfig::default()
                                },
                                ..SpriteBlitState::normal()
                            };
                            for opaque_destination in [false, true] {
                                let shader = SpanRowShader::new(blit, gamma, &sampler).unwrap();
                                let shader = if opaque_destination {
                                    shader.with_opaque_destination()
                                } else {
                                    shader
                                };
                                for x in 0..8 {
                                    for y in 0..8 {
                                        for alpha in [0, 71, 151, 255] {
                                            let color = Color::new(37, 129, 243, alpha);
                                            let background = Color::new(33, 65, 97, 131);
                                            let pixel_blit = sampler.blit_at(
                                                blit,
                                                (x as f32 + 0.5) / 8.0,
                                                (y as f32 + 0.5) / 8.0,
                                            );
                                            let fragment = prepare_sprite_fragment(
                                                color, None, None, pixel_blit,
                                            );
                                            let expected = composite_sprite_fragment(
                                                fragment,
                                                if opaque_destination && fragment.alpha() == 255.0 {
                                                    Color::new(0, 0, 0, 0)
                                                } else {
                                                    background
                                                },
                                                pixel_blit,
                                                gamma,
                                            );
                                            let mut actual = [
                                                background.r,
                                                background.g,
                                                background.b,
                                                background.a,
                                            ];
                                            shader.shade(
                                                color,
                                                &mut actual,
                                                sampler.x_axis_at((x as f32 + 0.5) / 8.0),
                                                sampler.y_axis_at((y as f32 + 0.5) / 8.0),
                                            );
                                            assert_eq!(
                                                actual,
                                                [expected.r, expected.g, expected.b, expected.a]
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn span_palette_reuses_draw_state_and_invalidates_changed_gamma_content() {
        let blit = SpriteBlitState::normal();
        let ramp = clonk_graphics::GammaRamp::standard();
        let first = span_palette(blit, Some(&ramp));
        let same_content = ramp.clone();
        let second = span_palette(blit, Some(&same_content));
        assert!(Arc::ptr_eq(&first, &second));
        let changed =
            clonk_graphics::GammaRamp::from_control_points([0x102030, 0x507080, 0xd0e0f0]);
        let third = span_palette(blit, Some(&changed));
        assert!(!Arc::ptr_eq(&first, &third));
        assert_ne!(first.opaque_rgb, third.opaque_rgb);
    }

    #[test]
    #[ignore = "manual release-mode sprite throughput measurement"]
    fn sprite_span_profile() {
        let source = FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
        };
        let destination = SurfaceRect::new(0, 0, 1280, 720);
        let gamma = clonk_graphics::GammaRamp::standard();
        for (label, mode, opaque, modulation) in [
            ("opaque", 0, true, None),
            ("alpha", 0, false, None),
            ("additive", C4GFXBLIT_ADDITIVE, false, None),
            ("mod2", C4GFXBLIT_MOD2, false, Some(0x0035_79bd)),
            (
                "mod2_additive",
                C4GFXBLIT_MOD2 | C4GFXBLIT_ADDITIVE,
                false,
                Some(0x0035_79bd),
            ),
        ] {
            let image = ImageData::new(
                64,
                64,
                (0_u32..4096)
                    .flat_map(|i| {
                        [
                            i as u8,
                            (i * 37) as u8,
                            (i * 71) as u8,
                            if opaque { 255 } else { (i * 113) as u8 },
                        ]
                    })
                    .collect(),
            );
            let blit = SpriteBlitState {
                mode,
                modulation,
                ..SpriteBlitState::normal()
            };
            let mut outputs = Vec::new();
            for reference in [true, false] {
                let mut surface = Surface::new(1280, 720, PixelFormat::Rgba8888);
                surface.fill(Color::new(71, 113, 179, 137));
                let mut samples = Vec::new();
                for iteration in 0..500 {
                    let started = std::time::Instant::now();
                    if reference {
                        rasterize_sprite_region(&mut surface, destination, Some(&gamma), |x, y| {
                            let (sx, sy) = source.source_edge(
                                (x as f32 + 0.5) / 1280.0,
                                (y as f32 + 0.5) / 720.0,
                                false,
                            );
                            prepare_runtime_sprite_sample(
                                &image,
                                None,
                                &source,
                                false,
                                sx,
                                sy,
                                BlitSampling::Nearest,
                                None,
                                blit,
                            )
                            .map(|fragment| (fragment, blit))
                        });
                    } else {
                        assert!(draw_nearest_sprite_span(
                            &mut surface,
                            &image,
                            &source,
                            destination,
                            blit,
                            Some(&gamma),
                            false,
                            None
                        ));
                    }
                    let elapsed = started.elapsed();
                    if iteration >= 200 {
                        samples.push(elapsed);
                    }
                    std::hint::black_box(surface.pixels());
                }
                if let Some(directory) = std::env::var_os("CLONK_SPRITE_SPAN_PROFILE_OUTPUT_DIR") {
                    let directory = std::path::PathBuf::from(directory);
                    std::fs::create_dir_all(&directory).unwrap();
                    let report = serde_json::json!({
                        "mode": label,
                        "reference": reference,
                        "pixels": 921600,
                        "warmup": 200,
                        "samples": samples.iter().map(|sample| sample.as_nanos() as u64).collect::<Vec<_>>(),
                    });
                    std::fs::write(
                        directory.join(format!("{label}-{reference}.json")),
                        serde_json::to_vec_pretty(&report).unwrap(),
                    )
                    .unwrap();
                }
                samples.sort_unstable();
                eprintln!("sprite_span_profile mode={label} reference={reference} pixels=921600 samples=300 warmup=200 p50_ns_per_pixel={:.6} p95_ns_per_pixel={:.6}", samples[150].as_secs_f64() * 1e9 / 921600.0, samples[284].as_secs_f64() * 1e9 / 921600.0);
                outputs.push(surface.pixels().to_vec());
            }
            assert_eq!(outputs[0], outputs[1], "{label}");
        }
    }

    #[test]
    fn span_compositor_matches_scalar_for_fractional_fragments_and_layers() {
        // Filtering precedes gamma and GL source-alpha composition
        // (src/StdGL.cpp:908,1081-1087,1246-1255).
        let ramps = [
            clonk_graphics::GammaRamp::identity(),
            clonk_graphics::GammaRamp::standard(),
            clonk_graphics::GammaRamp::from_control_points([0x102030, 0x507080, 0xd0e0f0]),
        ];
        let mut seed = 0x12_34_56_78_u64;
        for gamma in [None, Some(&ramps[0]), Some(&ramps[1]), Some(&ramps[2])] {
            let compositor = SpanCompositor::new(gamma).unwrap();
            for mode in [0, C4GFXBLIT_ADDITIVE] {
                let blit = SpriteBlitState {
                    mode,
                    ..SpriteBlitState::normal()
                };
                for _ in 0..10_000 {
                    let mut next = || {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        seed as u32
                    };
                    let bytes = next().to_le_bytes();
                    let legacy = PreparedSpriteFragment::Legacy(Color::new(
                        bytes[0], bytes[1], bytes[2], bytes[3],
                    ));
                    let shader = PreparedSpriteFragment::Shader {
                        rgb: std::array::from_fn(|_| (next() & 0xffff) as f32 / 257.0),
                        alpha: (next() & 0xffff) as f32 / 257.0,
                    };
                    let bytes = next().to_le_bytes();
                    let destination = Color::new(bytes[0], bytes[1], bytes[2], bytes[3]);
                    for fragment in [
                        legacy,
                        shader,
                        PreparedSpriteFragment::Layers {
                            base: legacy.into_layer(),
                            overlay: shader.into_layer(),
                        },
                    ] {
                        assert_eq!(
                            compositor.composite(fragment, destination, blit),
                            composite_sprite_fragment(fragment, destination, blit, gamma)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn integer_modulation_index_matches_float_gamma_sampling_for_every_texel_and_modulation() {
        // Shader modulation followed by GL_NEAREST R16 gamma texture lookup
        // (src/StdGL.cpp:1068-1087, 1246-1255).
        let gamma = clonk_graphics::GammaRamp::standard();
        let channel = clonk_graphics::gamma::GammaChannel::Red;
        for modulation in 0..=255_u8 {
            for source in 0..=255_u8 {
                let fragment = f32::from(source) * f32::from(modulation) / 255.0;
                let index = nearest_gamma_modulation_index(source, modulation);
                let reference_index = ((fragment * 256.0 / 255.0) as usize).min(255);
                assert_eq!(usize::from(index), reference_index);
                assert_eq!(
                    gamma.sample_channel_float(channel, f32::from(index)),
                    gamma.sample_channel_float(channel, fragment),
                    "source={source} modulation={modulation}"
                );
            }
        }
    }

    #[test]
    fn integer_span_pair_matches_scalar_for_every_alpha_and_channel_pair() {
        for alpha in 0..=255_u8 {
            for source in 0..=255_u8 {
                for destination in 0..=255_u8 {
                    let foreground = Color::new(source, 255 - source, source, alpha);
                    let background =
                        Color::new(destination, destination, 255 - destination, destination);
                    let expected = blend_colors(foreground, background);
                    let actual = blend_legacy_pair(
                        [[foreground.r, foreground.g, foreground.b, foreground.a]; 2],
                        [[background.r, background.g, background.b, background.a]; 2],
                    );
                    assert_eq!(
                        actual,
                        [[expected.r, expected.g, expected.b, expected.a]; 2]
                    );
                }
            }
        }
    }

    #[test]
    fn constant_fog_span_preserves_modulation_and_mod2_black_quad_rule() {
        // Native combines modulation at vertices before interpolation and
        // disables MOD2 for a wholly black quad (src/StdGL.cpp:471-472).
        for fog_color in [0, 0x00ff_ffff, 0x376b_a1ef] {
            let sampler = FogSpriteSampler {
                source_width: 4.0,
                source_height: 4.0,
                columns: 1,
                x_ranges: vec![(0.0, 4.0)],
                y_ranges: vec![(0.0, 4.0)],
                quads: vec![FogColorQuad {
                    x: (0.0, 4.0),
                    y: (0.0, 4.0),
                    modulation: [fog_color; 4],
                }],
            };
            for mode in [0, C4GFXBLIT_MOD2, C4GFXBLIT_MOD2 | C4GFXBLIT_ADDITIVE] {
                for modulation in [None, Some(0), Some(0x2345_6789)] {
                    let blit = SpriteBlitState {
                        mode,
                        modulation,
                        ..SpriteBlitState::normal()
                    };
                    let constant = constant_blit_for_span(blit, &sampler).unwrap();
                    for (x, y) in [(0.0, 0.0), (0.31, 0.29), (0.93, 0.77), (1.0, 1.0)] {
                        let sampled = sampler.blit_at(blit, x, y);
                        let source = Color::new(17, 129, 243, 151);
                        let background = Color::new(33, 65, 97, 131);
                        assert_eq!(
                            composite_sprite_fragment(
                                prepare_sprite_fragment(source, None, None, constant),
                                background,
                                constant,
                                None
                            ),
                            composite_sprite_fragment(
                                prepare_sprite_fragment(source, None, None, sampled),
                                background,
                                sampled,
                                None
                            ),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fog_spans_match_scalar_for_gradient_gamma_mod2_and_flat_shading() {
        let image = ImageData::new(
            16,
            16,
            (0_u32..256)
                .flat_map(|i| {
                    [
                        i.wrapping_mul(37) as u8,
                        i.wrapping_mul(71) as u8,
                        i.wrapping_mul(113) as u8,
                        i as u8,
                    ]
                })
                .collect(),
        );
        let source = FloatSourceRect::scaled(SourceRect::new(0, 0, 16, 16), 1.0);
        let rect = GuiRect::new(-0.25, 1.25, 24.4, 22.7);
        let destination = SurfaceRect::new(0, 1, 24, 23);
        let ramps = [
            clonk_graphics::GammaRamp::identity(),
            clonk_graphics::GammaRamp::standard(),
            clonk_graphics::GammaRamp::from_control_points([0x102030, 0x507080, 0xd0e0f0]),
        ];
        for colors in [
            [0; 4],
            [0x00ff_ffff; 4],
            [0x0012_3456, 0x4578_9abc, 0x89de_f012, 0xcf34_5678],
        ] {
            let sampler = FogSpriteSampler {
                source_width: 16.0,
                source_height: 16.0,
                columns: 1,
                x_ranges: vec![(0.0, 16.0)],
                y_ranges: vec![(0.0, 16.0)],
                quads: vec![FogColorQuad {
                    x: (0.0, 16.0),
                    y: (0.0, 16.0),
                    modulation: colors,
                }],
            };
            for gamma in [None, Some(&ramps[0]), Some(&ramps[1]), Some(&ramps[2])] {
                for mode in 0..4 {
                    for modulation in [None, Some(0), Some(0x3567_89ab)] {
                        for flags in 0..8 {
                            let mut blit = SpriteBlitState {
                                mode,
                                modulation,
                                ..SpriteBlitState::normal()
                            };
                            blit.renderer_config.shader = flags & 1 != 0;
                            blit.renderer_config.no_alpha_add = flags & 2 != 0;
                            blit.renderer_config.no_box_fades = flags & 4 != 0;
                            let flip = flags & 1 != 0;
                            let mut actual = Surface::new(24, 25, PixelFormat::Rgba8888);
                            actual.fill(Color::new(99, 103, 107, 109));
                            actual.set_clip(SurfaceRect::new(2, 3, 19, 18));
                            let mut expected = actual.clone();
                            assert!(draw_nearest_sprite_span(
                                &mut actual,
                                &image,
                                &source,
                                destination,
                                blit,
                                gamma,
                                flip,
                                Some((&sampler, &rect))
                            ));
                            rasterize_sprite_region(&mut expected, destination, gamma, |x, y| {
                                let normalized_x =
                                    ((x - destination.x) as f32 + 0.5) / destination.width as f32;
                                let normalized_y =
                                    ((y - destination.y) as f32 + 0.5) / destination.height as f32;
                                let (source_x, source_y) =
                                    source.source_edge(normalized_x, normalized_y, flip);
                                let pixel_blit = sampler.blit_at(
                                    blit,
                                    (x as f32 + 0.5 - rect.origin.x) / rect.size.width,
                                    (y as f32 + 0.5 - rect.origin.y) / rect.size.height,
                                );
                                prepare_runtime_sprite_sample(
                                    &image,
                                    None,
                                    &source,
                                    true,
                                    source_x,
                                    source_y,
                                    BlitSampling::Nearest,
                                    None,
                                    pixel_blit,
                                )
                                .map(|fragment| (fragment, pixel_blit))
                            });
                            assert_eq!(actual.pixels(), expected.pixels(), "mode={mode} modulation={modulation:?} flags={flags} colors={colors:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nearest_span_leaves_incomplete_images_to_the_scalar_path() {
        let image = ImageData::new(2, 2, vec![11, 23, 37, 255]);
        let source = FloatSourceRect::scaled(SourceRect::new(0, 0, 2, 2), 1.0);
        let mut surface = Surface::new(2, 2, PixelFormat::Rgba8888);
        assert!(!draw_nearest_sprite_span(
            &mut surface,
            &image,
            &source,
            SurfaceRect::new(0, 0, 2, 2),
            SpriteBlitState::normal(),
            None,
            false,
            None,
        ));
        assert!(surface.pixels().iter().all(|byte| *byte == 0));
    }

    #[test]
    fn nearest_span_copies_opaque_texels() {
        // CStdGL::PerformBlt: nearest texture sampling and source-alpha blend
        // (pinned oracle src/StdGL.cpp:908, 1304-1324).
        let image = ImageData::new(2, 1, vec![11, 23, 37, 255, 41, 53, 67, 255]);
        let source = FloatSourceRect::scaled(SourceRect::new(0, 0, 2, 1), 1.0);
        let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);
        assert!(draw_nearest_sprite_span(
            &mut surface,
            &image,
            &source,
            SurfaceRect::new(0, 0, 2, 1),
            SpriteBlitState::normal(),
            None,
            false,
            None,
        ));
        assert_eq!(surface.pixels(), image.pixels());
    }

    #[test]
    fn nearest_spans_match_scalar_fragment_oracle() {
        // Preserve the established CPU oracle for texture modulation, R16
        // gamma lookup, and blending (src/StdGL.cpp:1068-1087, 1304-1324).
        let image = ImageData::new(
            4,
            4,
            (0_u8..16)
                .flat_map(|i| {
                    [
                        i.wrapping_mul(37),
                        i.wrapping_mul(71),
                        i.wrapping_mul(113),
                        i.wrapping_mul(17),
                    ]
                })
                .collect(),
        );
        let source = FloatSourceRect::scaled(SourceRect::new(0, 0, 4, 4), 1.0);
        let ramps = [
            clonk_graphics::GammaRamp::identity(),
            clonk_graphics::GammaRamp::standard(),
            clonk_graphics::GammaRamp::from_control_points([0x102030, 0x507080, 0xd0e0f0]),
        ];
        for gamma in [None, Some(&ramps[0]), Some(&ramps[1]), Some(&ramps[2])] {
            for mode in [
                0,
                C4GFXBLIT_ADDITIVE,
                C4GFXBLIT_MOD2,
                C4GFXBLIT_MOD2 | C4GFXBLIT_ADDITIVE,
            ] {
                for modulation in [None, Some(0x00ff_ffff), Some(0), Some(0x4357_8bc1)] {
                    let blit = SpriteBlitState {
                        mode,
                        modulation,
                        ..SpriteBlitState::normal()
                    };
                    let background = Color::new(99, 103, 107, 109);
                    let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
                    surface.fill(background);
                    assert!(draw_nearest_sprite_span(
                        &mut surface,
                        &image,
                        &source,
                        SurfaceRect::new(0, 0, 4, 4),
                        blit,
                        gamma,
                        false,
                        None,
                    ));
                    for y in 0..4 {
                        for x in 0..4 {
                            let fragment = prepare_runtime_sprite_sample(
                                &image,
                                None,
                                &source,
                                false,
                                x as f32 + 0.5,
                                y as f32 + 0.5,
                                BlitSampling::Nearest,
                                None,
                                blit,
                            )
                            .unwrap();
                            let expected =
                                composite_sprite_fragment(fragment, background, blit, gamma);
                            assert_eq!(
                                surface.get_pixel(x, y),
                                Some(expected),
                                "x={x} y={y} mode={mode} modulation={modulation:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn nearest_spans_match_scalar_for_fractional_crops_scaling_flips_and_clips() {
        let mut random = 0x0061_8601_u64;
        let mut byte = || {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random as u8
        };
        let image = ImageData::new(64, 64, (0..64 * 64 * 4).map(|_| byte()).collect());
        let source = FloatSourceRect {
            x: 3.25,
            y: 7.5,
            width: 57.5,
            height: 52.25,
        };
        let gamma = clonk_graphics::GammaRamp::from_control_points([0x010305, 0x6789ab, 0xdfe7fb]);
        for case in 0..128 {
            let mut blit = SpriteBlitState::normal();
            blit.mode = u32::from(byte()) & 3;
            blit.modulation =
                (case % 4 != 0).then(|| u32::from_le_bytes([byte(), byte(), byte(), byte()]));
            blit.renderer_config.shader = case % 2 == 0;
            blit.renderer_config.no_alpha_add = case % 3 == 0;
            let gamma = (case % 5 != 0).then_some(&gamma);
            let flip = case % 2 != 0;
            let destination = SurfaceRect::new(-3, 2, 71, 59);
            let background = Color::new(byte(), byte(), byte(), byte());
            let mut actual = Surface::new(70, 64, PixelFormat::Rgba8888);
            actual.fill(background);
            actual.set_clip(SurfaceRect::new(1, 4, 61, 51));
            let mut expected = actual.clone();
            assert!(draw_nearest_sprite_span(
                &mut actual,
                &image,
                &source,
                destination,
                blit,
                gamma,
                flip,
                None,
            ));
            rasterize_sprite_region(&mut expected, destination, gamma, |x, y| {
                let normalized_x = ((x - destination.x) as f32 + 0.5) / destination.width as f32;
                let normalized_y = ((y - destination.y) as f32 + 0.5) / destination.height as f32;
                let (x, y) = source.source_edge(normalized_x, normalized_y, flip);
                prepare_runtime_sprite_sample(
                    &image,
                    None,
                    &source,
                    false,
                    x,
                    y,
                    BlitSampling::Nearest,
                    None,
                    blit,
                )
                .map(|fragment| (fragment, blit))
            });
            assert_eq!(actual.pixels(), expected.pixels(), "randomized draw {case}");
        }
    }
}
