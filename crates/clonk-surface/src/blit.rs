//! Geometry for blitting the CPU frame buffer onto the drawable.

/// Where the frame buffer lands on the drawable, and how much of it is covered.
///
/// `transform` is the column-major 4x4 the vertex shader applies to a
/// full-screen triangle; `clip_rect` is the scissor that keeps the blit inside
/// the letterboxed area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlitTransform {
    transform: [f32; 16],
    clip_rect: (u32, u32, u32, u32),
}

impl BlitTransform {
    /// The whole-pixel-multiple fit that presentation has always used.
    ///
    /// The buffer is magnified by the largest integer factor that still fits
    /// the drawable, and is never minified: a buffer larger than the drawable
    /// keeps scale 1 and is clipped. Mirrors `pixels` 0.17.2
    /// `ScalingMatrix::new` under `ScalingMode::PixelPerfect`, which is the
    /// only mode the application ever selected.
    pub fn pixel_perfect(buffer: (u32, u32), drawable: (u32, u32)) -> Self {
        let (buffer_width, buffer_height) = (buffer.0.max(1) as f32, buffer.1.max(1) as f32);
        let (drawable_width, drawable_height) =
            (drawable.0.max(1) as f32, drawable.1.max(1) as f32);

        let scale = (drawable_width / buffer_width)
            .max(1.0)
            .min((drawable_height / buffer_height).max(1.0))
            .floor()
            .max(1.0);
        let (scaled_width, scaled_height) = (buffer_width * scale, buffer_height * scale);

        // A drawable with an odd extent has no whole-pixel centre, so the
        // half-pixel remainder is carried in the translation. Dropping it
        // shifts every presented frame by half a pixel.
        let sw = scaled_width / drawable_width;
        let sh = scaled_height / drawable_height;
        let tx = (drawable_width / 2.0).fract() / drawable_width;
        let ty = (drawable_height / 2.0).fract() / drawable_height;
        #[rustfmt::skip]
        let transform = [
            sw,  0.0, 0.0, 0.0,
            0.0, sh,  0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            tx,  ty,  0.0, 1.0,
        ];

        let clipped_width = scaled_width.min(drawable_width);
        let clipped_height = scaled_height.min(drawable_height);
        let clip_rect = (
            ((drawable_width - clipped_width) / 2.0) as u32,
            ((drawable_height - clipped_height) / 2.0) as u32,
            clipped_width as u32,
            clipped_height as u32,
        );

        Self {
            transform,
            clip_rect,
        }
    }

    /// The scissor rectangle: the drawable's inner bounds, without the border.
    pub const fn clip_rect(&self) -> (u32, u32, u32, u32) {
        self.clip_rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every consumer sizes its frame buffer to the drawable's own physical
    // extent, so this is the case that actually ships: scale 1, no letterbox,
    // and a transform that must not nudge the image off the pixel grid.
    #[test]
    fn a_buffer_matching_an_even_drawable_blits_one_to_one() {
        let transform = BlitTransform::pixel_perfect((960, 640), (960, 640));

        assert_eq!(
            transform.transform,
            [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ]
        );
        assert_eq!(transform.clip_rect(), (0, 0, 960, 640));
    }

    // An odd extent has no whole-pixel centre. The remainder rides in the
    // translation column, and a port that drops it moves every presented frame
    // half a pixel — which no snapshot at an even extent would ever show.
    #[test]
    fn an_odd_drawable_carries_its_half_pixel_remainder_in_the_translation() {
        let transform = BlitTransform::pixel_perfect((961, 641), (961, 641));

        assert_eq!(transform.transform[12], 0.5 / 961.0);
        assert_eq!(transform.transform[13], 0.5 / 641.0);
        assert_eq!(transform.clip_rect(), (0, 0, 961, 641));
    }

    // Magnification is whole-pixel and takes the smaller axis, so a buffer that
    // would fit three times across but only twice down is magnified twice and
    // letterboxed on both axes.
    #[test]
    fn a_smaller_buffer_is_magnified_by_whole_pixels_and_letterboxed() {
        let transform = BlitTransform::pixel_perfect((320, 240), (960, 640));

        assert_eq!(transform.transform[0], 640.0 / 960.0);
        assert_eq!(transform.transform[5], 480.0 / 640.0);
        assert_eq!(transform.clip_rect(), (160, 80, 640, 480));
    }

    // A buffer larger than the drawable is never minified: it stays at scale 1
    // and the scissor clamps to the drawable rather than growing past it.
    #[test]
    fn a_buffer_larger_than_the_drawable_is_clipped_rather_than_shrunk() {
        let transform = BlitTransform::pixel_perfect((1920, 1080), (960, 640));

        assert_eq!(transform.transform[0], 2.0);
        assert_eq!(transform.transform[5], 1080.0 / 640.0);
        assert_eq!(transform.clip_rect(), (0, 0, 960, 640));
    }
}
