//! Exact byte-copy and grey interpolation kernels for retained CPU spans.

/// A clamped integer transform proved equivalent to every encoded LUT entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AffineGammaCopy {
    scale: [u16; 4],
    offset: [u16; 4],
    minimum: [u8; 4],
    maximum: [u8; 4],
}

impl AffineGammaCopy {
    pub(super) fn from_encoded(encoded: &[[u8; 256]; 3]) -> Option<Self> {
        fn channel(encoded: &[u8; 256]) -> Option<(u16, u16, u8, u8)> {
            let minimum = *encoded.iter().min()?;
            let maximum = *encoded.iter().max()?;
            for scale in 0..=256i32 {
                // The SIMD multiply and sum are unsigned 16-bit operations.
                let mut lower = 0;
                let mut upper = 65535 - scale * 255;
                for (input, &value) in encoded.iter().enumerate() {
                    let product = scale * input as i32;
                    if value > minimum {
                        lower = lower.max(i32::from(value) * 256 - product);
                    }
                    if value < maximum {
                        upper = upper.min((i32::from(value) + 1) * 256 - 1 - product);
                    }
                    if lower > upper {
                        break;
                    }
                }
                if lower <= upper
                    && encoded.iter().enumerate().all(|(input, &value)| {
                        ((scale * input as i32 + lower) >> 8)
                            .clamp(i32::from(minimum), i32::from(maximum))
                            == i32::from(value)
                    })
                {
                    return Some((scale as u16, lower as u16, minimum, maximum));
                }
            }
            None
        }
        let mut result = Self {
            scale: [256; 4],
            offset: [0; 4],
            minimum: [0; 4],
            maximum: [255; 4],
        };
        for (index, encoded) in encoded.iter().enumerate() {
            let (scale, offset, minimum, maximum) = channel(encoded)?;
            result.scale[index] = scale;
            result.offset[index] = offset;
            result.minimum[index] = minimum;
            result.maximum[index] = maximum;
        }
        Some(result)
    }

    pub(super) fn copy(&self, source: &[u8], destination: &mut [u8]) {
        assert_eq!(source.len(), destination.len());
        assert_eq!(source.len() % 4, 0);
        #[cfg(not(target_arch = "x86_64"))]
        let copied = 0;
        #[cfg(target_arch = "x86_64")]
        let copied = {
            use std::arch::x86_64::*;
            let mut copied = 0;
            let end = source.len() / 16 * 16;
            // SAFETY: SSE2 is mandatory on x86_64. Equal checked RGBA lengths
            // bound each unaligned 16-byte load/store. Recognition proves
            // scale * 255 + offset <= 65535 for every lane, so unsigned
            // multiplication/addition cannot wrap. The shifted lanes fit u8.
            unsafe {
                let lanes = |values: [u16; 4]| {
                    _mm_set_epi16(
                        values[3] as i16,
                        values[2] as i16,
                        values[1] as i16,
                        values[0] as i16,
                        values[3] as i16,
                        values[2] as i16,
                        values[1] as i16,
                        values[0] as i16,
                    )
                };
                let scale = lanes(self.scale);
                let offset = lanes(self.offset);
                let minimum = _mm_set1_epi32(i32::from_le_bytes(self.minimum));
                let maximum = _mm_set1_epi32(i32::from_le_bytes(self.maximum));
                let zero = _mm_setzero_si128();
                let transform = |values| {
                    _mm_srli_epi16::<8>(_mm_add_epi16(_mm_mullo_epi16(values, scale), offset))
                };
                while copied < end {
                    let source = _mm_loadu_si128(source.as_ptr().add(copied).cast());
                    let low = transform(_mm_unpacklo_epi8(source, zero));
                    let high = transform(_mm_unpackhi_epi8(source, zero));
                    let packed = _mm_packus_epi16(low, high);
                    let clamped = _mm_min_epu8(_mm_max_epu8(packed, minimum), maximum);
                    _mm_storeu_si128(destination.as_mut_ptr().add(copied).cast(), clamped);
                    copied += 16;
                }
            }
            copied
        };
        for (source, destination) in source[copied..]
            .chunks_exact(4)
            .zip(destination[copied..].chunks_exact_mut(4))
        {
            for channel in 0..4 {
                destination[channel] = ((u32::from(source[channel])
                    * u32::from(self.scale[channel])
                    + u32::from(self.offset[channel]))
                    >> 8)
                    .clamp(
                        u32::from(self.minimum[channel]),
                        u32::from(self.maximum[channel]),
                    ) as u8;
            }
        }
    }
}

/// Blend already-prepared shader RGBA, preserving the scalar operation order.
#[inline(always)]
pub(super) fn blend_shader_source_over(source: [f32; 4], destination: [u8; 4]) -> [u8; 4] {
    let coverage = (source[3] / 255.0).clamp(0.0, 1.0);
    let inverse = 1.0 - coverage;
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        // SAFETY: SSE2 is mandatory on x86_64. The source array contains
        // exactly four f32 lanes for the unaligned load. Destination bytes
        // enter and leave through a checked-size integer, without pointers.
        // Separate multiplies and addition preserve source-over ordering.
        unsafe {
            let zero = _mm_setzero_si128();
            let source = _mm_loadu_ps(source.as_ptr());
            let destination = _mm_cvtsi32_si128(i32::from_le_bytes(destination));
            let destination = _mm_cvtepi32_ps(_mm_unpacklo_epi16(
                _mm_unpacklo_epi8(destination, zero),
                zero,
            ));
            let rgb = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
            let contribution = _mm_mul_ps(source, _mm_set1_ps(coverage));
            let contribution = _mm_or_ps(_mm_and_ps(contribution, rgb), _mm_andnot_ps(rgb, source));
            let retained = _mm_mul_ps(destination, _mm_set1_ps(inverse));
            let value = _mm_add_ps(contribution, retained);
            // A NaN first operand selects zero in MAXPS, matching store_channel.
            let clamped = _mm_min_ps(_mm_max_ps(value, _mm_setzero_ps()), _mm_set1_ps(255.0));
            let integral = _mm_cvttps_epi32(clamped);
            let fraction = _mm_sub_ps(clamped, _mm_cvtepi32_ps(integral));
            let up = _mm_and_si128(
                _mm_castps_si128(_mm_cmpge_ps(fraction, _mm_set1_ps(0.5))),
                _mm_set1_epi32(1),
            );
            let rounded = _mm_add_epi32(integral, up);
            let bytes = _mm_packus_epi16(_mm_packs_epi32(rounded, zero), zero);
            _mm_cvtsi128_si32(bytes).to_le_bytes()
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    std::array::from_fn(|channel| {
        let value = if channel == 3 {
            source[channel] + f32::from(destination[channel]) * inverse
        } else {
            source[channel] * coverage + f32::from(destination[channel]) * inverse
        };
        value.round().clamp(0.0, 255.0) as u8
    })
}

/// Blend RGB-zero fragments after gamma, preserving fractional sample alpha.
/// Alpha must be finite and nonnegative; black channels finite in 0..=255.
pub(super) fn blend_black(alpha: &[f32], black: [f32; 3], destination: &mut [u8]) {
    assert_eq!(destination.len() / 4, alpha.len());
    assert_eq!(destination.len() % 4, 0);
    debug_assert!(alpha.iter().all(|alpha| alpha.is_finite() && *alpha >= 0.0));
    debug_assert!(black
        .iter()
        .all(|value| value.is_finite() && (0.0..=255.0).contains(value)));
    #[cfg(not(target_arch = "x86_64"))]
    let blended = 0;
    #[cfg(target_arch = "x86_64")]
    let blended = {
        use std::arch::x86_64::*;
        let mut blended = 0;
        let end = alpha.len() / 4 * 4;
        // SAFETY: SSE2 is mandatory on x86_64. The checked RGBA length gives
        // sixteen destination bytes for each four-alpha unaligned load.
        // All stores remain in those bounds. Separate division, multiply and
        // add intrinsics preserve the scalar source-over operation order.
        unsafe {
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            let maximum = _mm_set1_ps(255.0);
            let mask = _mm_set1_epi32(255);
            let black = black.map(|value| _mm_set1_ps(value));
            let round = |value| {
                let clamped = _mm_min_ps(_mm_max_ps(value, zero), maximum);
                let integral = _mm_cvttps_epi32(clamped);
                let fraction = _mm_sub_ps(clamped, _mm_cvtepi32_ps(integral));
                let up = _mm_and_si128(
                    _mm_castps_si128(_mm_cmpge_ps(fraction, _mm_set1_ps(0.5))),
                    _mm_set1_epi32(1),
                );
                _mm_add_epi32(integral, up)
            };
            while blended < end {
                let source_alpha = _mm_loadu_ps(alpha.as_ptr().add(blended));
                let coverage = _mm_min_ps(_mm_max_ps(_mm_div_ps(source_alpha, maximum), zero), one);
                let inverse = _mm_sub_ps(one, coverage);
                let pixels = _mm_loadu_si128(destination.as_ptr().add(blended * 4).cast());
                let channels = [
                    _mm_and_si128(pixels, mask),
                    _mm_and_si128(_mm_srli_epi32::<8>(pixels), mask),
                    _mm_and_si128(_mm_srli_epi32::<16>(pixels), mask),
                    _mm_srli_epi32::<24>(pixels),
                ];
                let mut output = [pixels; 4];
                for channel in 0..4 {
                    let retained = _mm_mul_ps(_mm_cvtepi32_ps(channels[channel]), inverse);
                    let source = if channel == 3 {
                        source_alpha
                    } else {
                        _mm_mul_ps(black[channel], coverage)
                    };
                    output[channel] = round(_mm_add_ps(source, retained));
                }
                let packed = _mm_or_si128(
                    _mm_or_si128(output[0], _mm_slli_epi32::<8>(output[1])),
                    _mm_or_si128(
                        _mm_slli_epi32::<16>(output[2]),
                        _mm_slli_epi32::<24>(output[3]),
                    ),
                );
                _mm_storeu_si128(destination.as_mut_ptr().add(blended * 4).cast(), packed);
                blended += 4;
            }
        }
        blended
    };
    for (alpha, pixel) in alpha[blended..]
        .iter()
        .zip(destination[blended * 4..].chunks_exact_mut(4))
    {
        let coverage = (*alpha / 255.0).clamp(0.0, 1.0);
        let inverse = 1.0 - coverage;
        for channel in 0..4 {
            let value = if channel == 3 {
                *alpha + f32::from(pixel[channel]) * inverse
            } else {
                black[channel] * coverage + f32::from(pixel[channel]) * inverse
            };
            pixel[channel] = value.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Fill a row after the caller has proved every covered fragment is opaque.
pub(super) fn fill_opaque(destination: &mut [u8], color: [u8; 4]) {
    assert_eq!(destination.len() % 4, 0);
    debug_assert_eq!(color[3], 255);
    #[cfg(not(target_arch = "x86_64"))]
    let filled = 0;
    #[cfg(target_arch = "x86_64")]
    let filled = {
        use std::arch::x86_64::{_mm_set1_epi32, _mm_storeu_si128};
        let mut filled = 0;
        let end = destination.len() / 16 * 16;
        // SAFETY: SSE2 is mandatory on x86_64. Each unaligned 16-byte store
        // stays within the checked RGBA destination, including its tail.
        unsafe {
            let pixels = _mm_set1_epi32(i32::from_le_bytes(color));
            while filled < end {
                _mm_storeu_si128(destination.as_mut_ptr().add(filled).cast(), pixels);
                filled += 16;
            }
        }
        filled
    };
    for pixel in destination[filled..].chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
}

pub(super) fn copy_standard_gamma(source: &[u8], destination: &mut [u8]) {
    assert_eq!(source.len(), destination.len());
    assert_eq!(source.len() % 4, 0);
    #[cfg(not(target_arch = "x86_64"))]
    let copied = 0;
    #[cfg(target_arch = "x86_64")]
    let copied = {
        let mut copied = 0;
        use std::arch::x86_64::{_mm_loadu_si128, _mm_max_epu8, _mm_set1_epi32, _mm_storeu_si128};
        let end = source.len() / 16 * 16;
        // SAFETY: SSE2 is mandatory on x86_64. Equal checked lengths and
        // copied < end ensure both unaligned 16-byte accesses stay in bounds.
        unsafe {
            let minimum = _mm_set1_epi32(0x0001_0101);
            while copied < end {
                let pixels = _mm_loadu_si128(source.as_ptr().add(copied).cast());
                _mm_storeu_si128(
                    destination.as_mut_ptr().add(copied).cast(),
                    _mm_max_epu8(pixels, minimum),
                );
                copied += 16;
            }
        }
        copied
    };
    for (input, output) in source[copied..]
        .chunks_exact(4)
        .zip(destination[copied..].chunks_exact_mut(4))
    {
        output.copy_from_slice(&[input[0].max(1), input[1].max(1), input[2].max(1), input[3]]);
    }
}

/// Interpolate one fog row using the retained rasterizer's ordered f32 sum.
/// Corners must be finite in 0..=255 and coordinates finite in 0..=1.
pub(super) fn interpolate_grey(
    corners: [f32; 4],
    horizontal: &[f32],
    vertical: f32,
    output: &mut [u8],
) {
    assert_eq!(horizontal.len(), output.len());
    debug_assert!(corners
        .iter()
        .all(|value| value.is_finite() && (0.0..=255.0).contains(value)));
    debug_assert!(vertical.is_finite() && (0.0..=1.0).contains(&vertical));
    debug_assert!(horizontal
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
    #[cfg(not(target_arch = "x86_64"))]
    let interpolated = 0;
    #[cfg(target_arch = "x86_64")]
    let interpolated = {
        let mut interpolated = 0;
        use std::arch::x86_64::*;
        let end = horizontal.len() / 4 * 4;
        // SAFETY: SSE2 is mandatory on x86_64. Every unaligned load reads
        // four in-bounds horizontal coordinates; the checked output length
        // permits the corresponding four-byte store. Clamped conversions
        // are finite and in 0..=255. Separate mul/add intrinsics preserve
        // the scalar sum's order without fused multiply-add operations.
        unsafe {
            let zero = _mm_setzero_ps();
            let one = _mm_set1_ps(1.0);
            let v = _mm_set1_ps(vertical);
            let values = corners.map(|corner| _mm_set1_ps(corner));
            while interpolated < end {
                let u = _mm_loadu_ps(horizontal.as_ptr().add(interpolated));
                let uv = _mm_add_ps(u, v);
                let first = _mm_cmple_ps(uv, one);
                let select = |yes, no| _mm_or_ps(_mm_and_ps(first, yes), _mm_andnot_ps(first, no));
                let weights = [
                    select(_mm_sub_ps(_mm_sub_ps(one, u), v), zero),
                    select(u, _mm_sub_ps(one, v)),
                    select(v, _mm_sub_ps(one, u)),
                    select(zero, _mm_sub_ps(uv, one)),
                ];
                let mut sum = zero;
                for index in 0..4 {
                    sum = _mm_add_ps(sum, _mm_mul_ps(values[index], weights[index]));
                }
                let clamped = _mm_min_ps(_mm_max_ps(sum, zero), _mm_set1_ps(255.0));
                let integral = _mm_cvttps_epi32(clamped);
                let fraction = _mm_sub_ps(clamped, _mm_cvtepi32_ps(integral));
                let round_up = _mm_and_si128(
                    _mm_castps_si128(_mm_cmpge_ps(fraction, _mm_set1_ps(0.5))),
                    _mm_set1_epi32(1),
                );
                let rounded = _mm_add_epi32(integral, round_up);
                let packed = _mm_packus_epi16(
                    _mm_packs_epi32(rounded, _mm_setzero_si128()),
                    _mm_setzero_si128(),
                );
                output[interpolated..interpolated + 4]
                    .copy_from_slice(&_mm_cvtsi128_si32(packed).to_le_bytes());
                interpolated += 4;
            }
        }
        interpolated
    };
    for (u, output) in horizontal[interpolated..]
        .iter()
        .zip(&mut output[interpolated..])
    {
        let weights = if *u + vertical <= 1.0 {
            [1.0 - *u - vertical, *u, vertical, 0.0]
        } else {
            [0.0, 1.0 - vertical, 1.0 - *u, *u + vertical - 1.0]
        };
        let mut sum = 0.0;
        for index in 0..4 {
            sum += corners[index] * weights[index];
        }
        *output = sum.round().clamp(0.0, 255.0) as u8;
    }
}

/// Shade opaque RGBA pixels with one grey modulation per pixel.
/// `None` selects standard gamma; `Some` supplies the encoded RGB lookup.
/// Every source alpha must be 255.
pub(super) fn shade_opaque_grey(
    source: &[u8],
    modulations: &[u8],
    destination: &mut [u8],
    encoded: Option<&[[u8; 256]; 3]>,
) {
    assert_eq!(source.len(), destination.len());
    assert_eq!(source.len() % 4, 0);
    assert_eq!(source.len() / 4, modulations.len());
    debug_assert!(source.chunks_exact(4).all(|pixel| pixel[3] == 255));
    #[cfg(not(target_arch = "x86_64"))]
    let shaded = 0;
    #[cfg(target_arch = "x86_64")]
    let shaded = {
        use std::arch::x86_64::*;
        let end = modulations.len() / 4 * 4;
        let mut shaded = 0;
        // SAFETY: SSE2 is mandatory on x86_64. Each iteration has four
        // checked modulation bytes and sixteen source/destination bytes.
        // Unaligned loads/stores remain inside the equal-length RGBA slices.
        unsafe {
            let zero = _mm_setzero_si128();
            let one = _mm_set1_epi16(1);
            let divisor = _mm_set1_epi16(254);
            let maximum = _mm_set1_epi16(255);
            let indices = |source, modulation| {
                let product = _mm_mullo_epi16(source, modulation);
                // floor(p * 256 / 65025) has divisor 254 + 1/256.
                // mulhi(p, 258) underestimates by at most one. For q <= 255,
                // the next quotient is reached exactly when p - q*254 >=255.
                let quotient = _mm_mulhi_epu16(product, _mm_set1_epi16(258));
                let remainder = _mm_sub_epi16(product, _mm_mullo_epi16(quotient, divisor));
                let increment = _mm_and_si128(_mm_cmpgt_epi16(remainder, divisor), one);
                _mm_min_epi16(_mm_add_epi16(quotient, increment), maximum)
            };
            while shaded < end {
                let pixels = _mm_loadu_si128(source.as_ptr().add(shaded * 4).cast());
                let modulation = |index| {
                    ((u32::from(modulations[shaded + index]) * 0x0001_0101) | 0xff00_0000) as i32
                };
                let modulations =
                    _mm_set_epi32(modulation(3), modulation(2), modulation(1), modulation(0));
                let low = indices(
                    _mm_unpacklo_epi8(pixels, zero),
                    _mm_unpacklo_epi8(modulations, zero),
                );
                let high = indices(
                    _mm_unpackhi_epi8(pixels, zero),
                    _mm_unpackhi_epi8(modulations, zero),
                );
                let packed = _mm_packus_epi16(low, high);
                let packed = if encoded.is_none() {
                    _mm_max_epu8(packed, _mm_set1_epi32(0x0001_0101))
                } else {
                    packed
                };
                _mm_storeu_si128(destination.as_mut_ptr().add(shaded * 4).cast(), packed);
                shaded += 4;
            }
        }
        if let Some(encoded) = encoded {
            for pixel in destination[..shaded * 4].chunks_exact_mut(4) {
                for channel in 0..3 {
                    pixel[channel] = encoded[channel][usize::from(pixel[channel])];
                }
            }
        }
        shaded
    };
    for (index, (input, output)) in source[shaded * 4..]
        .chunks_exact(4)
        .zip(destination[shaded * 4..].chunks_exact_mut(4))
        .enumerate()
    {
        for channel in 0..3 {
            let product = u16::from(input[channel]) * u16::from(modulations[shaded + index]);
            let lookup = ((f32::from(product) / 255.0 * 256.0 / 255.0) as usize).min(255);
            output[channel] =
                encoded.map_or((lookup as u8).max(1), |encoded| encoded[channel][lookup]);
        }
        output[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_copy_recognizes_lightning_floor_and_clamp_exactly() {
        let encoded = std::array::from_fn(|_| {
            std::array::from_fn(|input| ((160 * input + 24704) >> 8).clamp(97, 255) as u8)
        });
        assert!(AffineGammaCopy::from_encoded(&encoded).is_some());
    }

    fn asymmetric_affine_encoding() -> [[u8; 256]; 3] {
        [
            std::array::from_fn(|input| ((160 * input + 24704) >> 8).clamp(97, 255) as u8),
            std::array::from_fn(|input| ((256 * input + 127) >> 8).clamp(3, 239) as u8),
            std::array::from_fn(|input| ((128 * input + 32768) >> 8) as u8),
        ]
    }

    #[test]
    fn affine_copy_rejects_a_changed_interior_entry() {
        let mut encoded = asymmetric_affine_encoding();
        assert!(AffineGammaCopy::from_encoded(&encoded).is_some());
        encoded[0][128] -= 1;
        // Endpoints and monotonicity are still valid; the full interior proof
        // must reject this one-entry departure from the fitted transform.
        assert!(encoded[0].windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(AffineGammaCopy::from_encoded(&encoded).is_none());
        assert!(AffineGammaCopy::from_encoded(&custom_encoding()).is_none());
    }

    #[test]
    fn affine_copy_matches_every_input_and_alpha_with_asymmetric_channels() {
        let source = (0..=255u8)
            .flat_map(|alpha| {
                (0..=255u8)
                    .flat_map(move |input| [input, input.wrapping_mul(17), input ^ 91, alpha])
            })
            .collect::<Vec<_>>();
        let mut output = vec![0; source.len()];
        for encoded in [
            asymmetric_affine_encoding(),
            [[31; 256], [209; 256], [0; 256]],
        ] {
            let affine = AffineGammaCopy::from_encoded(&encoded).unwrap();
            for channel in 0..4 {
                assert!(
                    u32::from(affine.scale[channel]) * 255 + u32::from(affine.offset[channel])
                        <= 65535
                );
            }
            affine.copy(&source, &mut output);
            for (source, output) in source.chunks_exact(4).zip(output.chunks_exact(4)) {
                for channel in 0..3 {
                    assert_eq!(
                        output[channel],
                        encoded[channel][usize::from(source[channel])]
                    );
                }
                assert_eq!(output[3], source[3]);
            }
        }
    }

    #[test]
    fn affine_copy_matches_unaligned_rows_and_preserves_tail_guards() {
        let encoded = asymmetric_affine_encoding();
        let affine = AffineGammaCopy::from_encoded(&encoded).unwrap();
        for pixels in 0..=260 {
            let bytes = pixels * 4;
            for source_offset in 0..16 {
                let destination_offset = (source_offset * 7 + 3) % 16;
                let mut source = vec![0x5a; source_offset + bytes + 16];
                for (index, pixel) in source[source_offset..source_offset + bytes]
                    .chunks_exact_mut(4)
                    .enumerate()
                {
                    pixel.copy_from_slice(&[
                        index as u8,
                        (index as u8).wrapping_mul(37),
                        (index as u8).wrapping_add(128),
                        (index as u8).wrapping_mul(53),
                    ]);
                }
                let mut output = vec![0xa5; destination_offset + bytes + 16];
                let input = &source[source_offset..source_offset + bytes];
                affine.copy(
                    input,
                    &mut output[destination_offset..destination_offset + bytes],
                );
                for (input, actual) in input
                    .chunks_exact(4)
                    .zip(output[destination_offset..destination_offset + bytes].chunks_exact(4))
                {
                    for channel in 0..3 {
                        assert_eq!(
                            actual[channel],
                            encoded[channel][usize::from(input[channel])]
                        );
                    }
                    assert_eq!(actual[3], input[3]);
                }
                assert!(output[..destination_offset]
                    .iter()
                    .all(|value| *value == 0xa5));
                assert!(output[destination_offset + bytes..]
                    .iter()
                    .all(|value| *value == 0xa5));
                assert!(source[..source_offset].iter().all(|value| *value == 0x5a));
                assert!(source[source_offset + bytes..]
                    .iter()
                    .all(|value| *value == 0x5a));
            }
        }
    }

    #[test]
    fn shader_source_over_preserves_fractional_alpha_and_nonopaque_destination() {
        assert_eq!(
            blend_shader_source_over([201.0, 73.0, 129.0, 127.5], [13, 27, 39, 97]),
            [107, 50, 84, 176]
        );
    }

    fn reference_shader_source_over(source: [f32; 4], destination: [u8; 4]) -> [u8; 4] {
        // Literal pre-kernel put_fragment Shader/Normal/SourceOver equations,
        // including its nonnegative half-up store and NaN-to-zero policy.
        let alpha = (source[3] / 255.0).clamp(0.0, 1.0);
        std::array::from_fn(|channel| {
            let value = if channel == 3 {
                source[channel] + f32::from(destination[channel]) * (1.0 - alpha)
            } else {
                source[channel] * alpha + f32::from(destination[channel]) * (1.0 - alpha)
            };
            if value <= 0.0 || value.is_nan() {
                0
            } else if value >= 255.0 {
                255
            } else {
                let integral = value as u8;
                integral + u8::from(value - f32::from(integral) >= 0.5)
            }
        })
    }

    #[test]
    fn shader_source_over_matches_every_integer_source_destination_and_alpha() {
        for alpha in 0..=255u8 {
            for input in 0..=255u8 {
                let source = [
                    f32::from(input),
                    f32::from(input.wrapping_add(91)),
                    f32::from(input.wrapping_mul(17)),
                    f32::from(alpha),
                ];
                for dest in 0..=255u8 {
                    let destination = [dest, dest.wrapping_add(37), dest ^ 91, dest];
                    assert_eq!(
                        blend_shader_source_over(source, destination),
                        reference_shader_source_over(source, destination),
                        "input={input} alpha={alpha} destination={dest}"
                    );
                }
            }
        }
    }

    #[test]
    fn shader_source_over_matches_raw_gamma_fractions_and_float_boundaries() {
        let alpha = [
            0.0, 0.25, 0.5, 1.0, 63.499996, 127.49999, 127.5, 127.50001, 128.5, 254.49998, 254.5,
            255.0,
        ];
        for raw in 0..=65535u16 {
            for &alpha in &alpha {
                let source = [
                    f32::from(raw) / 257.0,
                    f32::from(65535 - raw) / 257.0,
                    f32::from(raw ^ 0x5a5a) / 257.0,
                    alpha,
                ];
                for destination in [[0; 4], [255; 4], [13, 27, 39, 97], [254, 1, 127, 17]] {
                    assert_eq!(
                        blend_shader_source_over(source, destination),
                        reference_shader_source_over(source, destination),
                        "raw={raw} alpha={alpha} destination={destination:?}"
                    );
                }
            }
        }
        let values = [
            f32::NEG_INFINITY,
            -256.0,
            -1.0,
            -0.0,
            0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            0.49999997,
            0.5,
            0.50000006,
            1.0,
            126.5,
            127.49999,
            127.5,
            127.50001,
            254.49998,
            254.5,
            254.50002,
            255.0,
            255.25,
            300.0,
            f32::INFINITY,
            f32::NAN,
        ];
        for (index, &alpha) in values.iter().enumerate() {
            for (sample, &red) in values.iter().enumerate() {
                let source = [
                    red,
                    values[(sample + 7) % values.len()],
                    values[(sample + 13) % values.len()],
                    alpha,
                ];
                for dest in [0u8, 1, 17, 63, 127, 128, 254, 255] {
                    let destination = [dest, dest.wrapping_add(37), dest ^ 91, index as u8 * 11];
                    assert_eq!(
                        blend_shader_source_over(source, destination),
                        reference_shader_source_over(source, destination),
                        "source={source:?} destination={destination:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn shader_source_over_handles_pixel_rows_at_every_alignment_without_touching_guards() {
        for pixels in 0..=19 {
            for offset in 0..16 {
                let mut output = vec![0xa5; offset + pixels * 4 + 16];
                for (index, pixel) in output[offset..offset + pixels * 4]
                    .chunks_exact_mut(4)
                    .enumerate()
                {
                    let source = [
                        index as f32 + 0.49999997,
                        index as f32 * 7.0,
                        index as f32 * 11.0,
                        index as f32 * 13.25,
                    ];
                    let destination = [index as u8 * 7, 19, 177, index as u8 * 11];
                    pixel.copy_from_slice(&blend_shader_source_over(source, destination));
                    assert_eq!(pixel, reference_shader_source_over(source, destination));
                }
                assert!(output[..offset].iter().all(|value| *value == 0xa5));
                assert!(output[offset + pixels * 4..]
                    .iter()
                    .all(|value| *value == 0xa5));
            }
        }
    }

    fn reference_black(alpha: f32, black: [f32; 3], destination: [u8; 4]) -> [u8; 4] {
        // Pins put_fragment's shader source-over equations: keep the division,
        // two products and their sum separate, including fractional sample alpha.
        let coverage = (alpha / 255.0).clamp(0.0, 1.0);
        let inverse = 1.0 - coverage;
        std::array::from_fn(|channel| {
            let value = if channel == 3 {
                alpha + f32::from(destination[channel]) * inverse
            } else {
                black[channel] * coverage + f32::from(destination[channel]) * inverse
            };
            value.round().clamp(0.0, 255.0) as u8
        })
    }

    #[test]
    fn black_blending_preserves_fractional_alpha_and_transparent_fragments() {
        let alpha = [0.0, 127.5, 255.0, 0.25, 128.0];
        let black = [256.0 / 257.0, 32768.0 / 257.0, 65535.0 / 257.0];
        let original = [37, 89, 211, 97].repeat(alpha.len());
        let mut actual = original.clone();
        blend_black(&alpha, black, &mut actual);
        let expected = alpha
            .iter()
            .zip(original.chunks_exact(4))
            .flat_map(|(alpha, pixel)| reference_black(*alpha, black, pixel.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    fn check_black_blend(alpha: &[f32], black: [f32; 3], original: &[u8]) {
        let mut actual = original.to_vec();
        blend_black(alpha, black, &mut actual);
        for ((alpha, input), output) in alpha
            .iter()
            .zip(original.chunks_exact(4))
            .zip(actual.chunks_exact(4))
        {
            assert_eq!(
                output,
                reference_black(*alpha, black, input.try_into().unwrap()),
                "alpha={alpha} black={black:?} input={input:?}"
            );
        }
    }

    #[test]
    fn black_blending_matches_all_integer_alpha_destination_pairs() {
        let destination = (0..=255u8)
            .flat_map(|value| [value, value ^ 0x5a, value.wrapping_mul(37), value])
            .collect::<Vec<_>>();
        for gamma in [0, 1, 128, 255, 256, 257, 258, 32767, 32768, 65534, 65535u16] {
            let black = [
                f32::from(gamma) / 257.0,
                f32::from(gamma ^ 0x5a5a) / 257.0,
                f32::from(gamma.wrapping_mul(37)) / 257.0,
            ];
            for alpha in 0..=255u8 {
                check_black_blend(&[f32::from(alpha); 256], black, &destination);
            }
        }
    }

    #[test]
    fn black_blending_matches_every_gamma_zero_with_fractional_alpha() {
        let destination = [0, 1, 2, 63, 127, 128, 254, 255u8]
            .into_iter()
            .flat_map(|value| [value; 4])
            .collect::<Vec<_>>();
        for gamma in 0..=65535u16 {
            let black = [f32::from(gamma) / 257.0; 3];
            for alpha in [0.0, 0.25, 0.5, 1.0, 63.5, 127.5, 128.0, 254.5, 255.0] {
                check_black_blend(&[alpha; 8], black, &destination);
            }
        }
    }

    #[test]
    fn black_blending_matches_unaligned_fractional_rows_and_preserves_guards() {
        let black = [256.0 / 257.0, 32767.0 / 257.0, 65534.0 / 257.0];
        for count in 0..=260 {
            for alpha_offset in 0..4 {
                let alpha = (0..count + alpha_offset)
                    .map(|index| match index % 8 {
                        0 => 0.0,
                        1 => 255.0,
                        2 => f32::from_bits(127.5f32.to_bits() - 1),
                        3 => f32::from_bits(127.5f32.to_bits() + 1),
                        _ => (index as f32 * 177.37) % 255.0,
                    })
                    .collect::<Vec<_>>();
                let alpha = &alpha[alpha_offset..];
                for destination_offset in 0..16 {
                    let mut destination = vec![0xa5; destination_offset + count * 4 + 16];
                    for (index, pixel) in destination
                        [destination_offset..destination_offset + count * 4]
                        .chunks_exact_mut(4)
                        .enumerate()
                    {
                        pixel.copy_from_slice(&[
                            index as u8,
                            (index as u8).wrapping_mul(37),
                            (index as u8) ^ 0x5a,
                            (index as u8).wrapping_mul(17),
                        ]);
                    }
                    let mut expected = destination.clone();
                    for (alpha, pixel) in alpha.iter().zip(
                        expected[destination_offset..destination_offset + count * 4]
                            .chunks_exact_mut(4),
                    ) {
                        let input = pixel[..].try_into().unwrap();
                        pixel.copy_from_slice(&reference_black(*alpha, black, input));
                    }
                    blend_black(
                        alpha,
                        black,
                        &mut destination[destination_offset..destination_offset + count * 4],
                    );
                    assert_eq!(destination, expected, "count={count} alpha_offset={alpha_offset} destination_offset={destination_offset}");
                }
            }
        }
    }

    #[test]
    fn opaque_fill_writes_encoded_color_without_touching_guards() {
        // StdGL.cpp:1076-1084 applies modulation before gamma. Opaque
        // RGB-zero modulation therefore has one encoded color for any texel.
        for count in 0..=260 {
            for offset in 0..16 {
                for color in [[1, 1, 1, 255], [3, 127, 241, 255]] {
                    let mut actual = vec![37; offset + count * 4 + 16];
                    fill_opaque(&mut actual[offset..offset + count * 4], color);
                    let mut expected = vec![37; actual.len()];
                    expected[offset..offset + count * 4].copy_from_slice(&color.repeat(count));
                    assert_eq!(
                        actual, expected,
                        "count={count} offset={offset} color={color:?}"
                    );
                }
            }
        }
    }

    fn reference_product(source: u8, modulation: u8) -> u8 {
        // StdGL.cpp:1076-1084 applies gamma after modulation; 1254-1255
        // selects nearest filtering. StdDDraw2.cpp:240,259,267 clamps
        // MinGamma to 0x100, giving the standard ramp its RGB zero lift.
        let prepared = f32::from(source) * f32::from(modulation) / 255.0;
        ((prepared * 256.0 / 255.0) as usize).min(255) as u8
    }

    #[test]
    fn opaque_grey_shading_keeps_alpha_and_lifts_zero_products() {
        let source = [0, 31, 63, 255].repeat(5);
        let mut output = [0; 20];
        shade_opaque_grey(&source, &[0; 5], &mut output, None);
        assert_eq!(output, [1, 1, 1, 255].repeat(5).as_slice());
    }

    fn custom_encoding() -> [[u8; 256]; 3] {
        std::array::from_fn(|channel| {
            std::array::from_fn(|index| {
                (index as u8)
                    .wrapping_mul(37 + channel as u8 * 6)
                    .wrapping_add(11 + channel as u8 * 86)
            })
        })
    }

    #[test]
    fn opaque_grey_shading_matches_all_source_modulation_pairs_and_custom_gamma() {
        let encoded = custom_encoding();
        let source = (0..=255u8)
            .flat_map(|value| [value, value.wrapping_mul(17), value ^ 0x5a, 255])
            .collect::<Vec<_>>();
        let mut output = vec![0; source.len()];
        for modulation in 0..=255u8 {
            for encoding in [None, Some(&encoded)] {
                shade_opaque_grey(&source, &[modulation; 256], &mut output, encoding);
                for (source, destination) in source.chunks_exact(4).zip(output.chunks_exact(4)) {
                    for channel in 0..3 {
                        let index = reference_product(source[channel], modulation);
                        let expected = encoding
                            .map_or(index.max(1), |encoded| encoded[channel][usize::from(index)]);
                        assert_eq!(
                            destination[channel], expected,
                            "source={} modulation={modulation} channel={channel}",
                            source[channel]
                        );
                    }
                    assert_eq!(destination[3], 255);
                }
            }
        }
    }

    #[test]
    fn opaque_grey_shading_matches_mixed_blocks_with_unaligned_rgba_tails() {
        let encoded = custom_encoding();
        for pixels in 0..=260 {
            let bytes = pixels * 4;
            let modulations = (0..pixels)
                .map(|index| (index as u8).wrapping_mul(53))
                .collect::<Vec<_>>();
            for source_offset in 0..16 {
                let mut source = vec![0x5a; source_offset + bytes + 16];
                for (index, pixel) in source[source_offset..source_offset + bytes]
                    .chunks_exact_mut(4)
                    .enumerate()
                {
                    pixel.copy_from_slice(&[
                        index as u8,
                        (index as u8).wrapping_mul(37),
                        (index as u8).wrapping_add(128),
                        255,
                    ]);
                }
                let source = &source[source_offset..source_offset + bytes];
                for destination_offset in 0..16 {
                    for encoding in [None, Some(&encoded)] {
                        let mut destination = vec![0xa5; destination_offset + bytes + 16];
                        shade_opaque_grey(
                            source,
                            &modulations,
                            &mut destination[destination_offset..destination_offset + bytes],
                            encoding,
                        );
                        for (index, (input, output)) in source
                            .chunks_exact(4)
                            .zip(
                                destination[destination_offset..destination_offset + bytes]
                                    .chunks_exact(4),
                            )
                            .enumerate()
                        {
                            for channel in 0..3 {
                                let lookup = reference_product(input[channel], modulations[index]);
                                let expected = encoding.map_or(lookup.max(1), |encoded| {
                                    encoded[channel][usize::from(lookup)]
                                });
                                assert_eq!(output[channel], expected);
                            }
                            assert_eq!(output[3], 255);
                        }
                        assert!(destination[..destination_offset]
                            .iter()
                            .all(|byte| *byte == 0xa5));
                        assert!(destination[destination_offset + bytes..]
                            .iter()
                            .all(|byte| *byte == 0xa5));
                    }
                }
            }
        }
    }

    fn reference_grey(corners: [f32; 4], horizontal: f32, vertical: f32) -> u8 {
        let weights = if horizontal + vertical <= 1.0 {
            [1.0 - horizontal - vertical, horizontal, vertical, 0.0]
        } else {
            [
                0.0,
                1.0 - vertical,
                1.0 - horizontal,
                horizontal + vertical - 1.0,
            ]
        };
        let mut value = 0.0;
        for index in 0..4 {
            value += corners[index] * weights[index];
        }
        value.round().clamp(0.0, 255.0) as u8
    }

    #[test]
    fn grey_interpolation_preserves_flat_white_with_an_uneven_tail() {
        interpolate_grey([0.0; 4], &[], 0.0, &mut []);
        let horizontal = [0.0, 0.25, 0.5, 0.75, 1.0];
        let mut output = [0; 5];
        interpolate_grey([255.0; 4], &horizontal, 0.0, &mut output);
        assert_eq!(output, [255; 5]);
    }

    #[test]
    fn grey_interpolation_matches_ordered_scalar_at_branch_and_rounding_boundaries() {
        let neighbours = |value: f32| {
            [
                f32::from_bits(value.to_bits().saturating_sub(1)).clamp(0.0, 1.0),
                value,
                f32::from_bits(value.to_bits() + 1).clamp(0.0, 1.0),
            ]
        };
        let mut horizontal = vec![0.0, 1.0];
        for value in [0.25, 0.5, 0.75] {
            horizontal.extend(neighbours(value));
        }
        for integer in 0..255 {
            horizontal.extend(neighbours((integer as f32 + 0.5) / 255.0));
        }
        let mut corner_sets = vec![
            [0.0; 4],
            [255.0; 4],
            [0.0, 255.0, 255.0, 0.0],
            [255.0, 0.0, 0.0, 255.0],
            [17.0, 64.0, 193.0, 251.0],
            [254.0, 3.0, 128.0, 77.0],
        ];
        for value in [0.5f32, 1.5, 127.5, 254.5] {
            for value in [
                f32::from_bits(value.to_bits() - 1),
                value,
                f32::from_bits(value.to_bits() + 1),
            ] {
                corner_sets.push([value; 4]);
            }
        }
        let mut vertical = vec![0.0, 1.0];
        for value in [0.25, 0.5, 0.75] {
            vertical.extend(neighbours(value));
        }
        for corners in corner_sets {
            for &v in &vertical {
                let mut output = vec![0; horizontal.len()];
                interpolate_grey(corners, &horizontal, v, &mut output);
                for (&u, &actual) in horizontal.iter().zip(&output) {
                    assert_eq!(
                        actual,
                        reference_grey(corners, u, v),
                        "{corners:?} u={u} v={v}"
                    );
                }
            }
        }
    }

    #[test]
    fn grey_interpolation_matches_millions_of_deterministic_samples_with_unaligned_tails() {
        let mut state = 0x24b5_68c1u32;
        let mut random = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as f32 / 16777215.0
        };
        let mut samples = 0;
        for row in 0..65536 {
            let corners = if row % 2 == 0 {
                match row % 8 {
                    0 => [0.0; 4],
                    2 => [255.0; 4],
                    4 => [0.0, 255.0, 255.0, 0.0],
                    _ => [255.0, 0.0, 0.0, 255.0],
                }
            } else {
                std::array::from_fn(|_| random() * 255.0)
            };
            let length = row % 67 + 1;
            let offset = row % 4;
            let vertical = random();
            let mut horizontal = [0.0; 70];
            for value in &mut horizontal[offset..offset + length] {
                *value = random();
            }
            let mut output = [0xa5; 70];
            interpolate_grey(
                corners,
                &horizontal[offset..offset + length],
                vertical,
                &mut output[offset..offset + length],
            );
            for index in offset..offset + length {
                assert_eq!(
                    output[index],
                    reference_grey(corners, horizontal[index], vertical),
                    "row={row} index={index} corners={corners:?} u={} v={vertical}",
                    horizontal[index]
                );
            }
            assert!(output[..offset].iter().all(|byte| *byte == 0xa5));
            assert!(output[offset + length..].iter().all(|byte| *byte == 0xa5));
            samples += length;
        }
        assert!(samples > 2_000_000);
    }

    #[test]
    fn standard_gamma_copy_preserves_alpha_and_unaligned_rgba_tails() {
        // StdDDraw2.cpp:240,259,267 clamps the standard ramp to MinGamma = 0x100,
        // lifting RGB zero to one while preserving the other byte values.
        for pixels in 0..=260 {
            let bytes = pixels * 4;
            for source_offset in 0..16 {
                for destination_offset in 0..16 {
                    let mut source = vec![0x5a; source_offset + bytes + 16];
                    for (index, pixel) in source[source_offset..source_offset + bytes]
                        .chunks_exact_mut(4)
                        .enumerate()
                    {
                        pixel.copy_from_slice(&[
                            index as u8,
                            (index as u8).wrapping_mul(37),
                            (index as u8).wrapping_add(128),
                            (index as u8).wrapping_mul(53),
                        ]);
                    }
                    let source = &source[source_offset..source_offset + bytes];
                    let mut destination = vec![0xa5; destination_offset + bytes + 16];
                    copy_standard_gamma(
                        source,
                        &mut destination[destination_offset..destination_offset + bytes],
                    );
                    for (input, output) in source.chunks_exact(4).zip(
                        destination[destination_offset..destination_offset + bytes].chunks_exact(4),
                    ) {
                        assert_eq!(
                            output,
                            [input[0].max(1), input[1].max(1), input[2].max(1), input[3]]
                        );
                    }
                    assert!(destination[..destination_offset]
                        .iter()
                        .all(|byte| *byte == 0xa5));
                    assert!(destination[destination_offset + bytes..]
                        .iter()
                        .all(|byte| *byte == 0xa5));
                }
            }
        }
    }

    #[test]
    #[should_panic]
    fn standard_gamma_copy_rejects_different_lengths() {
        copy_standard_gamma(&[0; 4], &mut [0; 8]);
    }

    #[test]
    #[should_panic]
    fn standard_gamma_copy_rejects_incomplete_rgba_pixels() {
        copy_standard_gamma(&[0; 3], &mut [0; 3]);
    }
}
