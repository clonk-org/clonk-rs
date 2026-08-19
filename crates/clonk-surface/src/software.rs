//! Presenting the CPU frame buffer without a GPU adapter.
//!
//! An interactive window still needs a wgpu device today, because even the
//! CPU-composed frame reaches the drawable through the blit pipeline. On a
//! GLES-2-only Raspberry Pi, or anywhere else no adapter is usable, that
//! aborts before a window exists (clonk-org/clonk-rs#299).
//!
//! This is the pixel half of the software path: given the CPU frame and the
//! drawable extent, it produces exactly the pixels the GPU blit would have
//! presented. Geometry is not reimplemented — it comes from
//! [`BlitTransform::pixel_perfect`], the same fit the shader uses, so the two
//! paths cannot drift into disagreeing about scale, centring or letterboxing.
//!
//! Deliberately free of any windowing or presentation dependency: this
//! function is what a `softbuffer`-backed presenter will fill its buffer with,
//! and keeping it separate is what lets it be tested on a machine with no
//! display at all.

use crate::blit::BlitTransform;

/// Why a software present could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SoftwarePresentError {
    /// The frame is not exactly `width * height * 4` bytes.
    #[error("software frame has {actual} bytes, expected {expected} for {width}x{height}")]
    FrameSize {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
    /// The destination is not exactly `width * height` words.
    #[error("software drawable has {actual} words, expected {expected} for {width}x{height}")]
    DrawableSize {
        width: u32,
        height: u32,
        expected: usize,
        actual: usize,
    },
    /// An extent overflows `usize` on this target.
    #[error("software presentation extent overflows usize")]
    ExtentOverflow,
}

/// The colour the letterbox around the scaled frame is cleared to.
///
/// Black, matching the clear the blit pass applies outside its scissor. A
/// different value here would show as a coloured border on any window whose
/// aspect does not divide evenly.
const LETTERBOX: u32 = 0x0000_0000;

/// Composite `frame` onto `drawable`, magnified by the pixel-perfect fit.
///
/// `frame` is tightly packed `Rgba8888`, the layout the CPU renderer already
/// produces. `drawable` receives one `0RGB` word per pixel, which is what
/// every `softbuffer` backend expects regardless of platform byte order.
///
/// Magnification is nearest-neighbour by a whole number, never interpolated
/// and never minified: the frame is a rasterized image whose pixels are the
/// output, so smoothing them would blur what the renderer deliberately placed.
/// A frame larger than the drawable keeps scale 1 and is centre-cropped, which
/// is what the shader's scissor does with the same inputs.
pub fn present_pixel_perfect(
    frame: &[u8],
    frame_extent: (u32, u32),
    drawable_extent: (u32, u32),
    drawable: &mut [u32],
) -> Result<(), SoftwarePresentError> {
    let (frame_width, frame_height) = frame_extent;
    let (drawable_width, drawable_height) = drawable_extent;

    let expected_frame = checked_area(frame_width, frame_height)?
        .checked_mul(4)
        .ok_or(SoftwarePresentError::ExtentOverflow)?;
    if frame.len() != expected_frame {
        return Err(SoftwarePresentError::FrameSize {
            width: frame_width,
            height: frame_height,
            expected: expected_frame,
            actual: frame.len(),
        });
    }
    let expected_drawable = checked_area(drawable_width, drawable_height)?;
    if drawable.len() != expected_drawable {
        return Err(SoftwarePresentError::DrawableSize {
            width: drawable_width,
            height: drawable_height,
            expected: expected_drawable,
            actual: drawable.len(),
        });
    }

    drawable.fill(LETTERBOX);
    if expected_drawable == 0 || expected_frame == 0 {
        return Ok(());
    }

    let transform = BlitTransform::pixel_perfect(frame_extent, drawable_extent);
    let scale = transform.scale().max(1);
    let (clip_x, clip_y, clip_width, clip_height) = transform.clip_rect();

    // The clip rectangle is where the scaled frame lands. Walking destination
    // pixels rather than source ones keeps a frame wider than the drawable
    // cropped instead of writing out of bounds.
    for row in 0..clip_height {
        let destination_y = clip_y + row;
        if destination_y >= drawable_height {
            break;
        }
        let source_y = row / scale;
        if source_y >= frame_height {
            break;
        }
        let source_row = (source_y as usize) * (frame_width as usize) * 4;
        let destination_row = (destination_y as usize) * (drawable_width as usize);
        for column in 0..clip_width {
            let destination_x = clip_x + column;
            if destination_x >= drawable_width {
                break;
            }
            let source_x = column / scale;
            if source_x >= frame_width {
                break;
            }
            let source = source_row + (source_x as usize) * 4;
            drawable[destination_row + destination_x as usize] =
                pack_0rgb(frame[source], frame[source + 1], frame[source + 2]);
        }
    }
    Ok(())
}

/// `softbuffer`'s pixel word: zero, then red, green, blue, most significant
/// byte first, whatever the platform's own byte order is.
///
/// Alpha is dropped rather than blended. The frame reaching a window is
/// already composited over the engine's background; treating its alpha as
/// window transparency would make the desktop show through wherever the
/// renderer left a translucent pixel.
const fn pack_0rgb(red: u8, green: u8, blue: u8) -> u32 {
    ((red as u32) << 16) | ((green as u32) << 8) | (blue as u32)
}

fn checked_area(width: u32, height: u32) -> Result<usize, SoftwarePresentError> {
    usize::try_from(width)
        .ok()
        .zip(usize::try_from(height).ok())
        .and_then(|(width, height)| width.checked_mul(height))
        .ok_or(SoftwarePresentError::ExtentOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One opaque pixel per cell, distinguishable by position and never pure
    /// black — a black source pixel is indistinguishable from the letterbox,
    /// which would make the centring test pass or fail on the fixture rather
    /// than on the geometry.
    fn frame(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|index| {
                [
                    (index % 251 + 1) as u8,
                    ((index / 251) % 256) as u8,
                    (index % 7 * 36 + 1) as u8,
                    255,
                ]
            })
            .collect()
    }

    #[test]
    fn a_frame_that_fits_exactly_is_copied_pixel_for_pixel() {
        let source = frame(4, 3);
        let mut drawable = vec![0xdead_beef; 12];
        present_pixel_perfect(&source, (4, 3), (4, 3), &mut drawable).expect("exact fit presents");

        let expected: Vec<u32> = source
            .chunks_exact(4)
            .map(|pixel| pack_0rgb(pixel[0], pixel[1], pixel[2]))
            .collect();
        assert_eq!(drawable, expected);
    }

    #[test]
    fn magnification_repeats_each_pixel_by_the_whole_number_scale() {
        // 2x3 into 6x9 is a clean 3x, so every source pixel becomes a 3x3
        // block and nothing is left over to letterbox.
        let source = frame(2, 3);
        let mut drawable = vec![0; 54];
        present_pixel_perfect(&source, (2, 3), (6, 9), &mut drawable).expect("3x magnifies");

        for y in 0..9_u32 {
            for x in 0..6_u32 {
                let source_index = ((y / 3) * 2 + x / 3) as usize * 4;
                assert_eq!(
                    drawable[(y * 6 + x) as usize],
                    pack_0rgb(
                        source[source_index],
                        source[source_index + 1],
                        source[source_index + 2]
                    ),
                    "pixel ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn the_frame_is_centred_and_the_remainder_is_letterboxed() {
        // 2x2 into 7x5 scales by 2 and leaves an odd border, so the frame
        // cannot sit on a whole-pixel centre. The blit's clip rectangle owns
        // that decision; this must land on the same pixels.
        let source = frame(2, 2);
        let mut drawable = vec![0xffff_ffff; 35];
        present_pixel_perfect(&source, (2, 2), (7, 5), &mut drawable).expect("odd drawable");

        let transform = BlitTransform::pixel_perfect((2, 2), (7, 5));
        let (clip_x, clip_y, clip_width, clip_height) = transform.clip_rect();
        assert_eq!(transform.scale(), 2);
        for y in 0..5_u32 {
            for x in 0..7_u32 {
                let inside = x >= clip_x
                    && x < clip_x + clip_width
                    && y >= clip_y
                    && y < clip_y + clip_height;
                let word = drawable[(y * 7 + x) as usize];
                if inside {
                    assert_ne!(word, LETTERBOX, "({x}, {y}) is inside the frame");
                } else {
                    assert_eq!(word, LETTERBOX, "({x}, {y}) is letterbox");
                }
            }
        }
    }

    #[test]
    fn a_frame_larger_than_the_drawable_is_cropped_rather_than_minified() {
        // Never minifying is the contract `pixel_perfect` documents; the
        // software path has to crop the same way instead of scaling down.
        let source = frame(8, 8);
        let mut drawable = vec![0; 9];
        present_pixel_perfect(&source, (8, 8), (3, 3), &mut drawable).expect("oversized frame");

        assert_eq!(BlitTransform::pixel_perfect((8, 8), (3, 3)).scale(), 1);
        for y in 0..3_u32 {
            for x in 0..3_u32 {
                let source_index = ((y * 8) + x) as usize * 4;
                assert_eq!(
                    drawable[(y * 3 + x) as usize],
                    pack_0rgb(
                        source[source_index],
                        source[source_index + 1],
                        source[source_index + 2]
                    ),
                );
            }
        }
    }

    #[test]
    fn alpha_is_dropped_rather_than_shown_as_window_transparency() {
        // A translucent pixel must present as its colour, not let the desktop
        // through: the frame is already composited by the time it arrives.
        let source = [10_u8, 20, 30, 0, 40, 50, 60, 128];
        let mut drawable = vec![0; 2];
        present_pixel_perfect(&source, (2, 1), (2, 1), &mut drawable).expect("translucent frame");
        assert_eq!(drawable, vec![pack_0rgb(10, 20, 30), pack_0rgb(40, 50, 60)]);
    }

    #[test]
    fn a_mismatched_frame_or_drawable_is_refused_rather_than_partly_drawn() {
        let mut drawable = vec![0; 4];
        assert!(matches!(
            present_pixel_perfect(&[0; 12], (2, 2), (2, 2), &mut drawable),
            Err(SoftwarePresentError::FrameSize { .. })
        ));
        assert!(matches!(
            present_pixel_perfect(&[0; 16], (2, 2), (3, 3), &mut drawable),
            Err(SoftwarePresentError::DrawableSize { .. })
        ));
    }
}
