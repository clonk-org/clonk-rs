//! Graphics.Scale support: the C++ engine lays the GUI out at
//! `ResolutionX x ResolutionY` and scales every draw by `Scale/100`
//! (C4Application.cpp:183, C4Gui.cpp:461). The Rust app renders the same
//! logical layout into a CPU surface, linearly magnifies it through the same
//! nominal viewport, and clips any top/right overflow to the framebuffer.

use lc_graphics::{ClipperProjection, Rect};

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

/// Shared logical-to-physical geometry for one presented frame.
///
/// C++ anchors an oversized scaled viewport at the lower-left of the
/// framebuffer. In top-down pixel coordinates that means any fractional
/// overflow is clipped from the top and right. Native-resolution passes must
/// use this geometry rather than independently rounding the application
/// scale, otherwise their anchors drift from the bilinearly scaled imagery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationGeometry {
    scale: f32,
    logical: (u32, u32),
    physical: (u32, u32),
    viewport: (u32, u32),
}

impl PresentationGeometry {
    fn new(scale: f32, logical: (u32, u32), physical: (u32, u32)) -> Self {
        Self {
            scale,
            logical,
            physical,
            viewport: viewport_size_for(logical.0, logical.1, scale),
        }
    }

    pub fn scale(self) -> f32 {
        self.scale
    }

    pub fn logical_size(self) -> (u32, u32) {
        self.logical
    }

    pub fn physical_size(self) -> (u32, u32) {
        self.physical
    }

    pub fn viewport_size(self) -> (u32, u32) {
        self.viewport
    }

    /// Rows clipped from the top of the nominal scaled viewport.
    pub fn crop_top(self) -> u32 {
        self.viewport.1.saturating_sub(self.physical.1)
    }

    /// Maps a GUI-space point to top-down physical framebuffer coordinates.
    pub fn logical_to_physical(self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale, y * self.scale - self.crop_top() as f32)
    }
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

/// Upscales into C++'s nominal GL viewport and writes only the part covered
/// by the physical framebuffer. The viewport is anchored at OpenGL's
/// lower-left, so top-down pixels lose overflow at the top and right.
#[allow(clippy::too_many_arguments)]
fn upscale_frame_in_viewport(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    viewport_width: u32,
    viewport_height: u32,
) {
    let (sw, sh) = (src_width as usize, src_height as usize);
    let (dw, dh) = (dst_width as usize, dst_height as usize);
    let (vw, vh) = (viewport_width as usize, viewport_height as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || vw == 0 || vh == 0 {
        return;
    }
    debug_assert!(dw <= vw && dh <= vh);
    debug_assert!(src.len() >= sw * sh * 4);
    debug_assert!(dst.len() >= dw * dh * 4);

    let x_taps: Vec<(usize, usize, u32)> = (0..dw).map(|x| axis_tap(x, vw, sw)).collect();
    let crop_top = vh.saturating_sub(dh);
    let mut source_words = vec![0u32; sw];
    let mut top = ScaledRow::new(dw);
    let mut bottom = ScaledRow::new(dw);

    for dst_y in 0..dh {
        let viewport_y = dst_y + crop_top;
        let (y0, y1, fy) = axis_tap(viewport_y, vh, sh);
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

/// Scales one premultiplied-alpha logical layer into the nominal viewport and
/// composites it over an already-presented physical frame.
///
/// Keeping the layer premultiplied is important: bilinear interpolation must
/// not pull arbitrary RGB out of transparent texels, and source RGB must not
/// be multiplied by alpha a second time during composition. A transparent
/// [`lc_graphics::Surface`](https://docs.rs/lc-graphics) rendered with normal
/// source-over operations naturally has this representation; callers that
/// write translucent pixels directly must premultiply them first.
#[allow(clippy::too_many_arguments)]
fn composite_premultiplied_layer_in_viewport(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    viewport_width: u32,
    viewport_height: u32,
) {
    let (sw, sh) = (src_width as usize, src_height as usize);
    let (dw, dh) = (dst_width as usize, dst_height as usize);
    let (vw, vh) = (viewport_width as usize, viewport_height as usize);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || vw == 0 || vh == 0 {
        return;
    }
    debug_assert!(dw <= vw && dh <= vh);
    debug_assert!(src.len() >= sw * sh * 4);
    debug_assert!(dst.len() >= dw * dh * 4);

    let x_taps: Vec<(usize, usize, u32)> = (0..dw).map(|x| axis_tap(x, vw, sw)).collect();
    let crop_top = vh.saturating_sub(dh);
    let mut source_words = vec![0u32; sw];
    let mut top = ScaledRow::new(dw);
    let mut bottom = ScaledRow::new(dw);

    for dst_y in 0..dh {
        let viewport_y = dst_y + crop_top;
        let (y0, y1, fy) = axis_tap(viewport_y, vh, sh);
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
            let source = lerp_word(above, below, fy).to_le_bytes();
            composite_premultiplied_pixel(out_px, source);
        }
    }
}

/// Composites an isolated logical layer through the rounded viewport installed
/// for one C++ primary clipper. Source sampling is relative to the logical
/// clip, matching the projection used by native text in that same clipper.
#[allow(clippy::too_many_arguments)]
fn composite_premultiplied_layer_with_clipper(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    projection: ClipperProjection,
) {
    let (sw, sh) = (src_width as usize, src_height as usize);
    let (dw, dh) = (dst_width as usize, dst_height as usize);
    let logical = projection.logical_clip();
    let physical = projection.physical_clip();
    if sw == 0
        || sh == 0
        || dw == 0
        || dh == 0
        || logical.width == 0
        || logical.height == 0
        || physical.width == 0
        || physical.height == 0
    {
        return;
    }
    debug_assert!(src.len() >= sw * sh * 4);
    debug_assert!(dst.len() >= dw * dh * 4);

    let Some(visible) = physical.intersection(Rect::new(0, 0, dst_width, dst_height)) else {
        return;
    };
    let logical_x = logical.x as usize;
    let logical_y = logical.y as usize;
    debug_assert!(logical_x + logical.width as usize <= sw);
    debug_assert!(logical_y + logical.height as usize <= sh);

    let physical_x_offset = i64::from(visible.x) - i64::from(physical.x);
    let x_taps = (0..visible.width as usize)
        .map(|x| {
            let local_x = (physical_x_offset + x as i64) as usize;
            let (x0, x1, fraction) =
                axis_tap(local_x, physical.width as usize, logical.width as usize);
            (logical_x + x0, logical_x + x1, fraction)
        })
        .collect::<Vec<_>>();
    let mut source_words = vec![0u32; sw];
    let mut top = ScaledRow::new(visible.width as usize);
    let mut bottom = ScaledRow::new(visible.width as usize);
    let physical_y_offset = i64::from(visible.y) - i64::from(physical.y);

    for row in 0..visible.height as usize {
        let local_y = (physical_y_offset + row as i64) as usize;
        let (y0, y1, fy) = axis_tap(local_y, physical.height as usize, logical.height as usize);
        let y0 = logical_y + y0;
        let y1 = logical_y + y1;
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

        let dst_y = visible.y as usize + row;
        let dst_start = (dst_y * dw + visible.x as usize) * 4;
        let out = &mut dst[dst_start..dst_start + visible.width as usize * 4];
        for ((out_px, &above), &below) in out
            .chunks_exact_mut(4)
            .zip(top.words.iter())
            .zip(bottom.words.iter())
        {
            let source = lerp_word(above, below, fy).to_le_bytes();
            composite_premultiplied_pixel(out_px, source);
        }
    }
}

#[inline]
fn composite_premultiplied_pixel(destination: &mut [u8], source: [u8; 4]) {
    let alpha = u16::from(source[3]);
    if source == [0, 0, 0, 0] {
        return;
    }
    if alpha == 255 {
        destination.copy_from_slice(&source);
        return;
    }

    let inverse = 255 - alpha;
    for channel in 0..3 {
        let retained = (u16::from(destination[channel]) * inverse + 127) / 255;
        destination[channel] = (u16::from(source[channel]) + retained).min(255) as u8;
    }
    destination[3] = (alpha + (u16::from(destination[3]) * inverse + 127) / 255).min(255) as u8;
}

fn viewport_size_for(logical_width: u32, logical_height: u32, scale: f32) -> (u32, u32) {
    let scale = scale.max(f32::EPSILON);
    let scaled = |extent: u32| ((extent as f32) * scale).ceil().clamp(1.0, u32::MAX as f32) as u32;
    (scaled(logical_width), scaled(logical_height))
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
/// through the fixed-size viewport used by C++. The GUI lives at
/// `ResX x ResY`; any fractional final scaled row/column outside the physical
/// framebuffer is clipped rather than fit-resampled.
pub struct FramePresenter {
    scale: f32,
    physical: (u32, u32),
    logical: Option<LogicalFrame>,
    ordered_layer: Vec<u8>,
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
            ordered_layer: Vec::new(),
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

    /// Rebuilds the logical frame for a new application scale while keeping
    /// the physical output size fixed. This is the presentation half of the
    /// startup Options scale test: the host may call it temporarily and only
    /// persist the value after the confirmation dialog returns Yes.
    pub fn set_scale(&mut self, scale: f32) {
        let scale = scale.max(f32::EPSILON);
        if (self.scale - scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = scale;
        self.resize(self.physical.0, self.physical.1);
    }

    /// Geometry shared by the filtered base and every ordered physical pass.
    pub fn presentation_geometry(&self) -> PresentationGeometry {
        PresentationGeometry::new(self.scale, self.logical_size(), self.physical)
    }

    pub fn resize(&mut self, physical_width: u32, physical_height: u32) {
        self.physical = (physical_width, physical_height);
        self.stale = true;
        self.ordered_layer.clear();
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
                    let viewport = viewport_size_for(logical.width, logical.height, self.scale);
                    upscale_frame_in_viewport(
                        &logical.frame,
                        logical.width,
                        logical.height,
                        output,
                        self.physical.0,
                        self.physical.1,
                        viewport.0,
                        viewport.1,
                    );
                    self.stale = false;
                }
                Ok(refreshed)
            }
        }
    }

    /// Starts an ordered sequence of logical raster layers and physical
    /// native-resolution passes over an already-presented base frame.
    ///
    /// Call this only when [`Self::present`] reports a refresh. The returned
    /// composer reuses presenter-owned storage, so it is cheap to create for
    /// every refreshed frame. Each later layer is composited after prior
    /// native drawing and therefore occludes it in the same order as C++.
    pub fn ordered_composer<'a>(&'a mut self, output: &'a mut [u8]) -> OrderedFrameComposer<'a> {
        let geometry = self.presentation_geometry();
        let (logical_width, logical_height) = geometry.logical_size();
        let logical_len = logical_width as usize * logical_height as usize * 4;
        let (physical_width, physical_height) = geometry.physical_size();
        let physical_len = physical_width as usize * physical_height as usize * 4;
        assert!(
            output.len() >= physical_len,
            "physical output has {} bytes, expected at least {physical_len}",
            output.len()
        );
        self.ordered_layer.resize(logical_len, 0);
        self.ordered_layer.fill(0);
        OrderedFrameComposer {
            geometry,
            output: &mut output[..physical_len],
            layer: self.ordered_layer.as_mut_slice(),
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
        assert_eq!(logical_size_for(3456, 1930, 3.0), (1152, 644));
        assert_eq!(viewport_size_for(1152, 644, 3.0), (3456, 1932));
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
    fn presenter_crops_a_fixed_scale_viewport_from_the_top_and_right() {
        // C4Application::SetResolution rounds the 5x5 framebuffer up to a
        // 2x2 GUI, then CStdGL::UpdateClipper installs a 6x6 viewport. OpenGL
        // clips its overflow on the visual top and right instead of fitting
        // the GUI into 5x5 (C4Application.cpp:536-538; StdGL.cpp:398-407).
        let source = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let mut full_viewport = vec![0_u8; 6 * 6 * 4];
        upscale_frame(&source, 2, 2, &mut full_viewport, 6, 6);
        let mut expected = vec![0_u8; 5 * 5 * 4];
        for y in 0..5_usize {
            let source_start = ((y + 1) * 6) * 4;
            let target_start = y * 5 * 4;
            expected[target_start..target_start + 5 * 4]
                .copy_from_slice(&full_viewport[source_start..source_start + 5 * 4]);
        }

        let mut presenter = FramePresenter::new(3.0, 5, 5);
        let mut output = vec![0_u8; 5 * 5 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                frame.copy_from_slice(&source);
                Ok(true)
            })
            .unwrap();

        assert_eq!(output, expected);
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
    fn presenter_scale_test_keeps_physical_size_and_rebuilds_logical_frame() {
        let mut presenter = FramePresenter::new(1.0, 1_280, 720);
        presenter.set_scale(2.0);
        assert_eq!(presenter.physical_size(), (1_280, 720));
        assert_eq!(presenter.logical_size(), (640, 360));
        assert_eq!(presenter.scale(), 2.0);
        presenter.set_scale(1.0);
        assert_eq!(presenter.logical_size(), (1_280, 720));
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
    fn presentation_geometry_shares_fractional_viewport_crop_with_native_passes() {
        let presenter = FramePresenter::new(1.5, 5, 4);
        let geometry = presenter.presentation_geometry();

        assert_eq!(geometry.logical_size(), (4, 3));
        assert_eq!(geometry.physical_size(), (5, 4));
        assert_eq!(geometry.viewport_size(), (6, 5));
        assert_eq!(geometry.crop_top(), 1);
        assert_eq!(geometry.logical_to_physical(2.0, 2.0), (3.0, 2.0));
    }

    #[test]
    fn ordered_raster_layer_occludes_an_earlier_native_pass() {
        let mut presenter = FramePresenter::new(2.0, 2, 2);
        let mut output = vec![0_u8; 2 * 2 * 4];
        assert!(presenter
            .present::<()>(&mut output, |frame| {
                frame.copy_from_slice(&[0, 0, 255, 255]);
                Ok(true)
            })
            .unwrap());

        let mut composer = presenter.ordered_composer(&mut output);
        composer.draw_native(|physical, geometry| {
            assert_eq!(geometry.logical_size(), (1, 1));
            for pixel in physical.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[255, 0, 0, 255]);
            }
        });
        // Premultiplied 50%-opaque green. This later chrome must blend over
        // the prior native red, rather than native text being replayed last.
        composer.begin_layer().copy_from_slice(&[0, 128, 0, 128]);
        composer.composite_layer();

        assert!(output
            .chunks_exact(4)
            .all(|pixel| pixel == [127, 128, 0, 255]));
    }

    #[test]
    fn ordered_layer_preserves_additive_rgb_with_zero_alpha() {
        let mut presenter = FramePresenter::new(2.0, 2, 2);
        let mut output = vec![0_u8; 2 * 2 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                frame.copy_from_slice(&[100, 50, 25, 255]);
                Ok(true)
            })
            .unwrap();

        let mut composer = presenter.ordered_composer(&mut output);
        // Additive framebuffer operations preserve destination alpha. On a
        // transparent ordered layer their contribution is therefore RGB with
        // A=0; generalized premultiplied composition must retain that RGB.
        composer.begin_layer().copy_from_slice(&[32, 10, 5, 0]);
        composer.composite_layer();

        assert!(output
            .chunks_exact(4)
            .all(|pixel| pixel == [132, 60, 30, 255]));
    }

    #[test]
    fn fractional_ordered_layer_uses_rounded_clipper_projection() {
        let mut presenter = FramePresenter::new(1.5, 6, 6);
        let mut output = solid(6, 6, [20, 40, 80, 255]);
        let mut composer = presenter.ordered_composer(&mut output);
        let layer = composer.begin_layer();
        for y in 1..3_usize {
            for x in 1..3_usize {
                let offset = (y * 4 + x) * 4;
                layer[offset..offset + 4].copy_from_slice(&[0, 200, 0, 255]);
            }
        }

        // At 1.5x CStdGL rounds this 2x2 clip to a 3x3 viewport rooted at
        // (1, 2). Absolute full-frame scaling would instead straddle both
        // horizontal clip edges with half-covered pixels.
        composer.composite_layer_with_clip(Rect::new(1, 1, 2, 2));

        for y in 0..6_usize {
            for x in 0..6_usize {
                let offset = (y * 6 + x) * 4;
                let expected = if (1..4).contains(&x) && (2..5).contains(&y) {
                    [0, 200, 0, 255]
                } else {
                    [20, 40, 80, 255]
                };
                assert_eq!(&output[offset..offset + 4], &expected, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn native_pass_after_a_raster_layer_remains_topmost() {
        let mut presenter = FramePresenter::new(1.5, 3, 3);
        let mut output = vec![0_u8; 3 * 3 * 4];
        presenter
            .present::<()>(&mut output, |frame| {
                for pixel in frame.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[12, 24, 48, 255]);
                }
                Ok(true)
            })
            .unwrap();

        let mut composer = presenter.ordered_composer(&mut output);
        for pixel in composer.begin_layer().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0, 200, 0, 255]);
        }
        composer.composite_layer();
        composer.draw_native(|physical, _| {
            physical[..4].copy_from_slice(&[255, 255, 255, 255]);
        });

        assert_eq!(&output[..4], &[255, 255, 255, 255]);
        assert!(output[4..]
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 200, 0, 255]));
    }

    #[test]
    fn upscale_handles_non_multiple_dimensions() {
        // The standalone fit-scaler remains safe for arbitrary dimensions;
        // FramePresenter applies C++'s fixed viewport and clipping policy.
        let src = solid(3, 3, [9, 9, 9, 255]);
        let mut dst = vec![0u8; 8 * 8 * 4];
        upscale_frame(&src, 3, 3, &mut dst, 8, 8);
        assert!(dst.chunks(4).all(|px| px == [9, 9, 9, 255]));
    }
}

/// Ordered compositor used after [`FramePresenter::present`].
///
/// A renderer alternates [`Self::begin_layer`] / [`Self::composite_layer`]
/// with [`Self::draw_native`]. Raster layers contain premultiplied RGBA in
/// logical coordinates; native callbacks draw directly into the physical
/// output. Committing another raster layer afterward correctly places its
/// chrome above earlier glyphs instead of replaying every text surface last.
pub struct OrderedFrameComposer<'a> {
    geometry: PresentationGeometry,
    output: &'a mut [u8],
    layer: &'a mut [u8],
}

impl OrderedFrameComposer<'_> {
    pub fn geometry(&self) -> PresentationGeometry {
        self.geometry
    }

    /// Clears and returns the next premultiplied-alpha logical raster layer.
    pub fn begin_layer(&mut self) -> &mut [u8] {
        self.layer.fill(0);
        &mut *self.layer
    }

    /// Scales and alpha-composites the current logical layer over all earlier
    /// raster and native passes.
    pub fn composite_layer(&mut self) {
        let (logical_width, logical_height) = self.geometry.logical_size();
        let (physical_width, physical_height) = self.geometry.physical_size();
        let (viewport_width, viewport_height) = self.geometry.viewport_size();
        composite_premultiplied_layer_in_viewport(
            self.layer,
            logical_width,
            logical_height,
            self.output,
            physical_width,
            physical_height,
            viewport_width,
            viewport_height,
        );
        self.layer.fill(0);
    }

    /// Scales and alpha-composites an isolated layer through one logical
    /// clipper. Use this only when every raster command in the layer is known
    /// to belong to that clip; mixed layers require [`Self::composite_layer`].
    pub fn composite_layer_with_clip(&mut self, logical_clip: Rect) {
        let (logical_width, logical_height) = self.geometry.logical_size();
        let (physical_width, physical_height) = self.geometry.physical_size();
        let projection = ClipperProjection::new(
            self.geometry.scale(),
            (logical_width, logical_height),
            physical_height,
            logical_clip,
        );
        composite_premultiplied_layer_with_clipper(
            self.layer,
            logical_width,
            logical_height,
            self.output,
            physical_width,
            physical_height,
            projection,
        );
        self.layer.fill(0);
    }

    /// Runs one native-resolution pass at this exact point in the layer
    /// sequence. The callback receives the shared viewport geometry needed to
    /// transform GUI anchors and clips.
    pub fn draw_native<R>(&mut self, draw: impl FnOnce(&mut [u8], PresentationGeometry) -> R) -> R {
        draw(self.output, self.geometry)
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
        eprintln!(
            "upscale {}x{} -> {}x{}: {:?}/frame",
            sw,
            sh,
            dw,
            dh,
            start.elapsed() / iterations
        );
    }
}
