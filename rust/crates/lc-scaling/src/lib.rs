//! Graphics.Scale support: the C++ engine lays the GUI out at
//! `ResolutionX x ResolutionY` and scales every draw by `Scale/100`
//! (C4Application.cpp:183, C4Gui.cpp:461). The Rust app renders the same
//! logical layout into a CPU surface and upscales the finished frame to the
//! window's pixel size, mirroring the GL pipeline's linear magnification.

/// GUI layout size for a window pixel size: ceil(pixels / scale), at least
/// one pixel — C4Application::SetResolution (C4Application.cpp:536-538).
pub fn logical_size_for(physical_width: u32, physical_height: u32, scale: f32) -> (u32, u32) {
    let scale = scale.max(f32::EPSILON);
    let width = ((physical_width as f32) / scale).ceil().max(1.0) as u32;
    let height = ((physical_height as f32) / scale).ceil().max(1.0) as u32;
    (width, height)
}

/// True when the scale needs no resampling pass.
pub fn is_identity_scale(scale: f32) -> bool {
    (scale - 1.0).abs() < f32::EPSILON
}

/// Upscales an RGBA8 frame to the window's pixel size with bilinear
/// sampling, the CPU counterpart of the GL_LINEAR magnification the C++
/// engine uses for scaled output (StdGL.cpp texture filtering). Source
/// coordinates follow GL texel-center sampling; edges clamp.
pub fn upscale_frame(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
) {
    let (sw, sh) = (src_width as usize, src_height as usize);
    let (dw, dh) = (dst_width as usize, dst_height as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return;
    }
    debug_assert!(src.len() >= sw * sh * 4);
    debug_assert!(dst.len() >= dw * dh * 4);

    if sw == dw && sh == dh {
        dst[..dw * dh * 4].copy_from_slice(&src[..sw * sh * 4]);
        return;
    }

    // Separable two-pass bilinear on packed RGBA words: each needed source
    // row is scaled horizontally once and cached; every output row is then
    // one vertical lerp of two cached rows.
    let x_taps: Vec<(usize, usize, u32)> = (0..dw).map(|x| axis_tap(x, dw, sw)).collect();

    let mut source_words = vec![0u32; sw];
    let mut top = ScaledRow::new(dw);
    let mut bottom = ScaledRow::new(dw);

    for dst_y in 0..dh {
        let (y0, y1, fy) = axis_tap(dst_y, dh, sh);
        if top.source != y0 {
            if bottom.source == y0 {
                std::mem::swap(&mut top, &mut bottom);
            } else {
                top.build(src, y0, sw, &x_taps, &mut source_words);
            }
        }
        if bottom.source != y1 {
            bottom.build(src, y1, sw, &x_taps, &mut source_words);
        }
        let out = &mut dst[dst_y * dw * 4..(dst_y * dw + dw) * 4];
        for ((out_px, &above), &below) in out
            .chunks_exact_mut(4)
            .zip(top.words.iter())
            .zip(bottom.words.iter())
        {
            out_px.copy_from_slice(&lerp_word(above, below, fy).to_le_bytes());
        }
    }
}

/// A source row scaled to the destination width, tagged with its source
/// row index so consecutive output rows reuse it.
struct ScaledRow {
    source: usize,
    words: Vec<u32>,
}

impl ScaledRow {
    fn new(dst_width: usize) -> Self {
        Self {
            source: usize::MAX,
            words: vec![0; dst_width],
        }
    }

    fn build(
        &mut self,
        src: &[u8],
        src_y: usize,
        src_width: usize,
        x_taps: &[(usize, usize, u32)],
        source_words: &mut [u32],
    ) {
        let row = &src[src_y * src_width * 4..(src_y * src_width + src_width) * 4];
        for (word, px) in source_words.iter_mut().zip(row.chunks_exact(4)) {
            *word = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        }
        for (out, &(x0, x1, fx)) in self.words.iter_mut().zip(x_taps) {
            *out = lerp_word(source_words[x0], source_words[x1], fx);
        }
        self.source = src_y;
    }
}

/// Blends two packed RGBA8 words with an 8-bit weight (0..=256), two
/// channels per multiply; component products stay within 16 bits.
#[inline]
fn lerp_word(a: u32, b: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = (((a & 0x00FF_00FF) * g + (b & 0x00FF_00FF) * f) >> 8) & 0x00FF_00FF;
    let ag = ((((a >> 8) & 0x00FF_00FF) * g + ((b >> 8) & 0x00FF_00FF) * f) >> 8) & 0x00FF_00FF;
    rb | (ag << 8)
}

/// Owns the logical-resolution frame the app renders into and upscales it
/// to the window's pixel buffer — the C++ engine's window/GUI split, where
/// the GUI lives at `ResX x ResY` and output pixels at `ResX*Scale`.
pub struct FramePresenter {
    scale: f32,
    physical: (u32, u32),
    logical: Option<LogicalFrame>,
    stale: bool,
}

struct LogicalFrame {
    width: u32,
    height: u32,
    frame: Vec<u8>,
}

impl FramePresenter {
    pub fn new(scale: f32, physical_width: u32, physical_height: u32) -> Self {
        let mut presenter = Self {
            scale,
            physical: (physical_width, physical_height),
            logical: None,
            stale: true,
        };
        presenter.resize(physical_width, physical_height);
        presenter
    }

    /// The size the app lays out and renders at.
    pub fn logical_size(&self) -> (u32, u32) {
        self.logical
            .as_ref()
            .map(|logical| (logical.width, logical.height))
            .unwrap_or(self.physical)
    }

    pub fn physical_size(&self) -> (u32, u32) {
        self.physical
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn resize(&mut self, physical_width: u32, physical_height: u32) {
        self.physical = (physical_width, physical_height);
        self.stale = true;
        self.logical = (!is_identity_scale(self.scale)).then(|| {
            let (width, height) = logical_size_for(physical_width, physical_height, self.scale);
            LogicalFrame {
                width,
                height,
                frame: vec![0; width as usize * height as usize * 4],
            }
        });
    }

    /// Window pixels to GUI coordinates, like the C++ mouse path divides by
    /// the application scale (C4MouseControl.cpp:185).
    pub fn position_to_gui(&self, x: f64, y: f64) -> (f64, f64) {
        let scale = f64::from(self.scale.max(f32::EPSILON));
        (x / scale, y / scale)
    }

    /// Runs `render` against the logical frame and upscales into `output`
    /// (the window-sized pixel buffer). `render` returns whether it composed
    /// new content; unchanged frames skip the upscale, relying on `output`
    /// persisting between calls. At identity scale `render` draws straight
    /// into `output`. Returns whether the physical output was refreshed; a
    /// caller may use that one-shot commit point for native-resolution text.
    pub fn present<E>(
        &mut self,
        output: &mut [u8],
        render: impl FnOnce(&mut [u8]) -> Result<bool, E>,
    ) -> Result<bool, E> {
        match self.logical.as_mut() {
            None => {
                let changed = render(output)?;
                let refreshed = changed || self.stale;
                self.stale = false;
                Ok(refreshed)
            }
            Some(logical) => {
                let changed = render(&mut logical.frame)?;
                let refreshed = changed || self.stale;
                if refreshed {
                    upscale_frame(
                        &logical.frame,
                        logical.width,
                        logical.height,
                        output,
                        self.physical.0,
                        self.physical.1,
                    );
                    self.stale = false;
                }
                Ok(refreshed)
            }
        }
    }
}

/// Source taps and 8-bit blend weight for one destination coordinate:
/// GL texel-center mapping src = (dst + 0.5) * (src_len / dst_len) - 0.5,
/// clamped to the source range.
fn axis_tap(dst: usize, dst_len: usize, src_len: usize) -> (usize, usize, u32) {
    let position = (dst as f32 + 0.5) * (src_len as f32 / dst_len as f32) - 0.5;
    let clamped = position.max(0.0);
    let base = clamped.floor();
    let frac = ((clamped - base) * 256.0).round() as u32;
    let i0 = (base as usize).min(src_len - 1);
    let i1 = (i0 + 1).min(src_len - 1);
    (i0, i1, frac.min(256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_size_uses_ceil_like_set_resolution() {
        // C4Application.cpp:536-538.
        assert_eq!(logical_size_for(2742, 1716, 3.0), (914, 572));
        assert_eq!(logical_size_for(2743, 1717, 3.0), (915, 573));
        assert_eq!(logical_size_for(1280, 720, 1.0), (1280, 720));
        assert_eq!(logical_size_for(1, 1, 3.0), (1, 1));
    }

    #[test]
    fn identity_scale_detection() {
        assert!(is_identity_scale(1.0));
        assert!(!is_identity_scale(3.0));
        assert!(!is_identity_scale(1.5));
    }

    fn solid(width: usize, height: usize, rgba: [u8; 4]) -> Vec<u8> {
        rgba.iter()
            .copied()
            .cycle()
            .take(width * height * 4)
            .collect()
    }

    #[test]
    fn upscale_keeps_solid_color_solid() {
        let src = solid(3, 2, [10, 200, 30, 255]);
        let mut dst = vec![0u8; 9 * 6 * 4];
        upscale_frame(&src, 3, 2, &mut dst, 9, 6);
        assert!(dst.chunks(4).all(|px| px == [10, 200, 30, 255]));
    }

    #[test]
    fn upscale_replicates_single_pixel() {
        let src = solid(1, 1, [1, 2, 3, 4]);
        let mut dst = vec![0u8; 3 * 3 * 4];
        upscale_frame(&src, 1, 1, &mut dst, 3, 3);
        assert!(dst.chunks(4).all(|px| px == [1, 2, 3, 4]));
    }

    #[test]
    fn upscale_identity_size_copies() {
        let src: Vec<u8> = (0..2 * 2 * 4).map(|v| v as u8).collect();
        let mut dst = vec![0u8; 2 * 2 * 4];
        upscale_frame(&src, 2, 2, &mut dst, 2, 2);
        assert_eq!(dst, src);
    }

    #[test]
    fn upscale_corners_match_source_corners() {
        // 2x2 checker: corners of the scaled image sample the matching
        // source corner texels (clamped GL_LINEAR edges).
        let mut src = solid(2, 2, [0, 0, 0, 255]);
        src[..4].copy_from_slice(&[255, 0, 0, 255]); // top-left red
        src[12..16].copy_from_slice(&[0, 255, 0, 255]); // bottom-right green
        let mut dst = vec![0u8; 8 * 8 * 4];
        upscale_frame(&src, 2, 2, &mut dst, 8, 8);
        assert_eq!(&dst[..4], &[255, 0, 0, 255]);
        let last = dst.len() - 4;
        assert_eq!(&dst[last..], &[0, 255, 0, 255]);
    }

    #[test]
    fn presenter_identity_scale_renders_into_output_directly() {
        let mut presenter = FramePresenter::new(1.0, 4, 4);
        assert_eq!(presenter.logical_size(), (4, 4));
        let mut output = vec![0u8; 4 * 4 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                frame.fill(7);
                Ok(true)
            })
            .unwrap();
        assert!(output.iter().all(|&value| value == 7));
    }

    #[test]
    fn presenter_scaled_renders_logical_and_upscales() {
        // 3x scale: the app draws at ceil(6/3)=2 logical pixels per axis,
        // the presenter fills the 6x6 window buffer.
        let mut presenter = FramePresenter::new(3.0, 6, 6);
        assert_eq!(presenter.logical_size(), (2, 2));
        let mut output = vec![0u8; 6 * 6 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                assert_eq!(frame.len(), 2 * 2 * 4);
                frame.fill(50);
                Ok(true)
            })
            .unwrap();
        assert!(output.iter().all(|&value| value == 50));
    }

    #[test]
    fn presenter_skips_upscale_for_unchanged_frames() {
        let mut presenter = FramePresenter::new(2.0, 4, 4);
        let mut output = vec![0u8; 4 * 4 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                frame.fill(9);
                Ok(true)
            })
            .unwrap();
        // An unchanged frame (menu cache replay) must not touch the output,
        // even if the logical buffer were rewritten.
        presenter
            .present::<()>(&mut output, |frame| {
                frame.fill(200);
                Ok(false)
            })
            .unwrap();
        assert!(output.iter().all(|&value| value == 9));
    }

    #[test]
    fn presenter_resize_forces_upscale() {
        let mut presenter = FramePresenter::new(2.0, 4, 4);
        let mut output = vec![0u8; 4 * 4 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                frame.fill(9);
                Ok(true)
            })
            .unwrap();
        presenter.resize(4, 4);
        presenter
            .present::<()>(&mut output, |frame| {
                frame.fill(33);
                Ok(false)
            })
            .unwrap();
        assert!(output.iter().all(|&value| value == 33));
    }

    #[test]
    fn presenter_maps_positions_to_gui_space() {
        let presenter = FramePresenter::new(3.0, 6, 6);
        assert_eq!(presenter.position_to_gui(300.0, 150.0), (100.0, 50.0));
    }

    #[test]
    fn presenter_reports_native_overlay_commit_point_after_bilinear_base() {
        // C++ filters image textures at Graphics.Scale != 100 (StdGL.cpp:
        // 527-532), but CStdFont's scale-native atlas lands at one atlas texel
        // per output pixel (C4Fonts.cpp:158-173; StdFont.cpp:319-352,841-842).
        // The presenter must expose exactly one post-filter commit point so a
        // physical caption is neither filtered nor alpha-blended repeatedly.
        let mut presenter = FramePresenter::new(3.0, 6, 3);
        let mut output = vec![0_u8; 6 * 3 * 4];
        let updated = presenter
            .present::<()>(&mut output, |frame| {
                for pixel in frame[..4].chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[255, 0, 0, 255]);
                }
                for pixel in frame[4..8].chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[0, 0, 255, 255]);
                }
                Ok(true)
            })
            .unwrap();
        assert!(updated, "new bilinear base opens the physical overlay pass");
        let middle = &output[(2 * 4)..(3 * 4)];
        assert!(middle[0] > 0 && middle[2] > 0, "imagery stays bilinear");

        output[2 * 4..3 * 4].copy_from_slice(&[255, 255, 0, 255]);
        assert_eq!(&output[2 * 4..3 * 4], &[255, 255, 0, 255]);
        let updated = presenter
            .present::<()>(&mut output, |_frame| Ok(false))
            .unwrap();
        assert!(!updated, "a cached frame must not blend native text twice");
        assert_eq!(&output[2 * 4..3 * 4], &[255, 255, 0, 255]);
    }

    #[test]
    fn upscale_handles_cropped_destination() {
        // ceil() layouts can overdraw by a partial pixel row/column; the
        // destination just clamps (no panic, edges from the last texels).
        let src = solid(3, 3, [9, 9, 9, 255]);
        let mut dst = vec![0u8; 8 * 8 * 4];
        upscale_frame(&src, 3, 3, &mut dst, 8, 8);
        assert!(dst.chunks(4).all(|px| px == [9, 9, 9, 255]));
    }
}

#[cfg(test)]
mod perf_probe {
    use super::*;

    #[test]
    #[ignore = "manual timing probe"]
    fn upscale_timing_probe() {
        let (sw, sh) = (1371u32, 858u32);
        let (dw, dh) = (4113u32, 2574u32);
        let src = vec![128u8; sw as usize * sh as usize * 4];
        let mut dst = vec![0u8; dw as usize * dh as usize * 4];
        let start = std::time::Instant::now();
        let iterations = 5;
        for _ in 0..iterations {
            upscale_frame(&src, sw, sh, &mut dst, dw, dh);
        }
        eprintln!("upscale {}x{} -> {}x{}: {:?}/frame", sw, sh, dw, dh, start.elapsed() / iterations);
    }
}
