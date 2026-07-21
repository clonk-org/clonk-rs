use crate::clonk_font::CapturedClonkText;
use crate::color::Color;
use crate::snapshot::{checksum_update, SurfaceSnapshot, FNV_OFFSET};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8888,
}

impl PixelFormat {
    fn bytes_per_pixel(self) -> usize {
        4
    }
}

/// How a (modulated) source pixel composites with the destination, mirroring the
/// C++ `C4GFXBLIT_*` blit modes (`src/C4Surface.h:39`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlitMode {
    /// `C4GFXBLIT_NORMAL` — standard source-alpha over (GL `GL_SRC_ALPHA,
    /// GL_ONE_MINUS_SRC_ALPHA`).
    #[default]
    Normal,
    /// `C4GFXBLIT_ADDITIVE` — `dst + src·srcAlpha` (GL `GL_SRC_ALPHA, GL_ONE`).
    Additive,
    /// `C4GFXBLIT_MOD2` — ADD_SIGNED*2 source modulation, then alpha-over.
    Mod2,
    /// `C4GFXBLIT_MOD2 | C4GFXBLIT_ADDITIVE` — MOD2 source preparation,
    /// followed by additive framebuffer composition.
    Mod2Additive,
}

impl BlitMode {
    const fn uses_mod2(self) -> bool {
        matches!(self, Self::Mod2 | Self::Mod2Additive)
    }

    /// Prepare one source texel as the live StdGL blit shader does before
    /// framebuffer composition. Keeping this operation separate lets callers
    /// that cache straight-alpha pictures avoid blending the texel into a
    /// transparent scratch framebuffer first.
    pub fn prepare_source(self, source: Color, modulation: Color) -> Color {
        if self.uses_mod2() && modulation != Color::transparent() {
            // PerformBlt resets MOD2 only for the exact packed value zero.
            // Otherwise the live shader applies ADD_SIGNED*2 to RGB and
            // leaves texture opacity untouched.
            source.modulate_rgb_mod2(modulation)
        } else if modulation != Color::opaque(255, 255, 255) {
            // Exact zero reaches this path after the native MOD2 reset and
            // therefore produces an ordinarily modulated black silhouette.
            source.modulate_clr(modulation)
        } else {
            source
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn intersection(self, other: Rect) -> Option<Rect> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width as i32).min(other.x + other.width as i32);
        let bottom = (self.y + self.height as i32).min(other.y + other.height as i32);

        if right <= left || bottom <= top {
            None
        } else {
            Some(Rect {
                x: left,
                y: top,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            })
        }
    }
}

#[derive(Debug, Error)]
pub enum SurfaceError {
    #[error("pixel buffer has invalid length: expected {expected} bytes, got {actual}")]
    InvalidBufferLength { expected: usize, actual: usize },
    #[error("pixel coordinate out of bounds: ({x}, {y}) for {width}x{height} surface")]
    OutOfBounds {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    #[error("pixel formats do not match: source {src:?}, destination {dst:?}")]
    FormatMismatch { src: PixelFormat, dst: PixelFormat },
}

/// Minimal pixel target used by draw routines that can operate directly on
/// either an owned [`Surface`] or a borrowed RGBA framebuffer.
pub trait SurfaceDrawTarget {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn clip(&self) -> Option<Rect>;
    fn set_clip(&mut self, clip: Rect);
    fn clear_clip(&mut self);
    fn get_pixel(&self, x: u32, y: u32) -> Option<Color>;
    fn set_pixel(&mut self, x: u32, y: u32, color: Color) -> Result<(), SurfaceError>;

    /// Semantic text capture is an owned-surface facility. Borrowed native
    /// presentation targets always rasterize immediately.
    #[doc(hidden)]
    fn capture_clonk_text(&mut self, _command: CapturedClonkText) -> bool {
        false
    }
}

/// Scoped, zero-copy drawing view over a tightly packed RGBA8 framebuffer.
///
/// Unlike [`Surface::from_bytes`], this target borrows the caller's bytes, so
/// native-resolution overlays can blend in place without cloning and copying
/// an entire physical frame for every draw batch.
#[derive(Debug)]
pub struct RgbaSurfaceViewMut<'a> {
    width: u32,
    height: u32,
    stride: usize,
    data: &'a mut [u8],
    clip: Option<Rect>,
}

impl<'a> RgbaSurfaceViewMut<'a> {
    pub fn new(width: u32, height: u32, data: &'a mut [u8]) -> Result<Self, SurfaceError> {
        let stride = width as usize * PixelFormat::Rgba8888.bytes_per_pixel();
        let expected = stride * height as usize;
        if data.len() != expected {
            return Err(SurfaceError::InvalidBufferLength {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            stride,
            data,
            clip: None,
        })
    }

    fn pixel_in_clip(&self, x: u32, y: u32) -> bool {
        self.clip.is_none_or(|clip| {
            let x = i64::from(x);
            let y = i64::from(y);
            let left = i64::from(clip.x);
            let top = i64::from(clip.y);
            x >= left
                && y >= top
                && x < left + i64::from(clip.width)
                && y < top + i64::from(clip.height)
        })
    }

    fn pixel_offset(&self, x: u32, y: u32) -> usize {
        y as usize * self.stride + x as usize * PixelFormat::Rgba8888.bytes_per_pixel()
    }
}

impl SurfaceDrawTarget for RgbaSurfaceViewMut<'_> {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn clip(&self) -> Option<Rect> {
        self.clip
    }

    fn set_clip(&mut self, clip: Rect) {
        self.clip = Some(clip);
    }

    fn clear_clip(&mut self) {
        self.clip = None;
    }

    fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = self.pixel_offset(x, y);
        Some(Color::new(
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
        ))
    }

    fn set_pixel(&mut self, x: u32, y: u32, color: Color) -> Result<(), SurfaceError> {
        if x >= self.width || y >= self.height {
            return Err(SurfaceError::OutOfBounds {
                x,
                y,
                width: self.width,
                height: self.height,
            });
        }
        if !self.pixel_in_clip(x, y) {
            return Ok(());
        }
        let offset = self.pixel_offset(x, y);
        self.data[offset..offset + 4].copy_from_slice(&[color.r, color.g, color.b, color.a]);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Surface {
    width: u32,
    height: u32,
    format: PixelFormat,
    stride: usize,
    data: Vec<u8>,
    /// Active clipping rectangle (C++ `SetPrimaryClipper`); `None` = full surface.
    /// All draws are restricted to `clip ∩ bounds`.
    clip: Option<Rect>,
    /// Active semantic font capture. Ordinary Surface operations remain
    /// unchanged; role-tagged ClonkFont draws append here and suppress their
    /// logical glyph pixels until the command list is taken.
    clonk_text_capture: Option<Vec<CapturedClonkText>>,
}

impl Surface {
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        let stride = width as usize * format.bytes_per_pixel();
        let data = vec![0; stride * height as usize];
        Self {
            width,
            height,
            format,
            stride,
            data,
            clip: None,
            clonk_text_capture: None,
        }
    }

    pub fn from_bytes(
        width: u32,
        height: u32,
        format: PixelFormat,
        data: Vec<u8>,
    ) -> Result<Self, SurfaceError> {
        let stride = width as usize * format.bytes_per_pixel();
        let expected = stride * height as usize;
        if data.len() != expected {
            return Err(SurfaceError::InvalidBufferLength {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            format,
            stride,
            data,
            clip: None,
            clonk_text_capture: None,
        })
    }

    /// Begin a fresh semantic ClonkFont capture, discarding any untaken
    /// commands from an earlier capture on this surface.
    pub fn begin_clonk_text_capture(&mut self) {
        self.clonk_text_capture = Some(Vec::new());
    }

    /// End semantic ClonkFont capture and return commands in draw order.
    /// Returns an empty vector when capture was not active.
    pub fn take_clonk_text_capture(&mut self) -> Vec<CapturedClonkText> {
        self.clonk_text_capture.take().unwrap_or_default()
    }

    /// Take commands captured on a temporary child surface, restrict them to
    /// that child's bounds, and translate their anchors and clippers into this
    /// surface's coordinate system.
    ///
    /// A child with no explicit clip still clips drawing to its own bounds.
    /// Making that implicit clip explicit here prevents scale-native replay
    /// from escaping a viewport or scratch surface after translation.
    /// Returns `false` when semantic capture is not active on the destination.
    pub fn extend_clonk_text_capture_from(&mut self, child: &mut Surface, offset: Point) -> bool {
        if self.clonk_text_capture.is_none() {
            return false;
        }
        let child_bounds = child.bounds();
        let mut commands = child.take_clonk_text_capture();
        for command in &mut commands {
            command.x = command.x.saturating_add(offset.x);
            command.y = command.y.saturating_add(offset.y);
            let mut clip = match command.clip {
                Some(clip) => clip
                    .intersection(child_bounds)
                    .unwrap_or(Rect::new(child_bounds.x, child_bounds.y, 0, 0)),
                None => child_bounds,
            };
            clip.x = clip.x.saturating_add(offset.x);
            clip.y = clip.y.saturating_add(offset.y);
            command.clip = Some(clip);
        }
        let destination = self
            .clonk_text_capture
            .as_mut()
            .expect("capture presence checked above");
        destination.extend(commands);
        true
    }

    /// Extend semantic text from a temporary layer while applying the same
    /// packed-C4 modulation that will be used to composite that layer's
    /// raster pixels. This keeps scale-native text and inline images visually
    /// attached to a modulated GUI surface.
    pub fn extend_clonk_text_capture_from_modulated(
        &mut self,
        child: &mut Surface,
        offset: Point,
        modulation: Color,
    ) -> bool {
        if self.clonk_text_capture.is_none() {
            return false;
        }
        let child_bounds = child.bounds();
        let mut commands = child.take_clonk_text_capture();
        for command in &mut commands {
            command.x = command.x.saturating_add(offset.x);
            command.y = command.y.saturating_add(offset.y);
            let mut clip = match command.clip {
                Some(clip) => clip
                    .intersection(child_bounds)
                    .unwrap_or(Rect::new(child_bounds.x, child_bounds.y, 0, 0)),
                None => child_bounds,
            };
            clip.x = clip.x.saturating_add(offset.x);
            clip.y = clip.y.saturating_add(offset.y);
            command.clip = Some(clip);

            let color = Color::new(
                command.color[0],
                command.color[1],
                command.color[2],
                command.color[3],
            )
            .modulate_clr(modulation);
            command.color = [color.r, color.g, color.b, color.a];
            for image in &mut command.images {
                for pixel in image.rgba.chunks_exact_mut(4) {
                    let color = Color::new(pixel[0], pixel[1], pixel[2], pixel[3])
                        .modulate_clr(modulation);
                    pixel.copy_from_slice(&[color.r, color.g, color.b, color.a]);
                }
            }
        }
        let destination = self
            .clonk_text_capture
            .as_mut()
            .expect("capture presence checked above");
        destination.extend(commands);
        true
    }

    /// Whether role-tagged ClonkFont draws are currently being captured.
    pub fn is_clonk_text_capture_active(&self) -> bool {
        self.clonk_text_capture.is_some()
    }

    /// Append a command when capture is active. The boolean tells ClonkFont
    /// whether logical rasterization must be suppressed.
    pub(crate) fn capture_clonk_text(&mut self, command: CapturedClonkText) -> bool {
        let Some(commands) = self.clonk_text_capture.as_mut() else {
            return false;
        };
        commands.push(command);
        true
    }

    /// Set the clipping rectangle (C++ `SetPrimaryClipper`); subsequent draws are
    /// restricted to `clip ∩ bounds`. The rect is stored as given and intersected
    /// with the surface at draw time.
    pub fn set_clip(&mut self, clip: Rect) {
        self.clip = Some(clip);
    }

    /// Return the active clipping rectangle so nested renderers can restore
    /// their caller's primary clipper after drawing a bounded child.
    pub fn clip(&self) -> Option<Rect> {
        self.clip
    }

    /// Remove the clipping rectangle; draws cover the full surface again
    /// (C++ `NoPrimaryClipper`).
    pub fn clear_clip(&mut self) {
        self.clip = None;
    }

    /// The effective draw region: the active clip intersected with the surface
    /// bounds, or the full bounds when no clip is set. An empty intersection
    /// yields a zero-size rect, so nothing draws.
    fn clip_bounds(&self) -> Rect {
        match self.clip {
            Some(c) => c
                .intersection(self.bounds())
                .unwrap_or(Rect::new(0, 0, 0, 0)),
            None => self.bounds(),
        }
    }

    fn pixel_in_clip(&self, x: u32, y: u32) -> bool {
        self.clip.is_none_or(|clip| {
            let x = i64::from(x);
            let y = i64::from(y);
            let left = i64::from(clip.x);
            let top = i64::from(clip.y);
            x >= left
                && y >= top
                && x < left + i64::from(clip.width)
                && y < top + i64::from(clip.height)
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn pixels(&self) -> &[u8] {
        &self.data
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn snapshot(&self) -> SurfaceSnapshot {
        SurfaceSnapshot::from_surface(self)
    }

    pub fn snapshot_region(&self, rect: Rect) -> Option<SurfaceSnapshot> {
        let region = rect.intersection(self.bounds())?;
        if region.width == 0 || region.height == 0 {
            return None;
        }
        let bpp = self.format.bytes_per_pixel();
        let mut hash = FNV_OFFSET;
        for row in 0..region.height {
            let y = (region.y + row as i32) as u32;
            let offset = self.pixel_offset(region.x as u32, y);
            let span = &self.data[offset..offset + region.width as usize * bpp];
            hash = checksum_update(hash, span);
        }
        Some(SurfaceSnapshot::from_parts(
            region.width,
            region.height,
            hash,
        ))
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub fn fill(&mut self, color: Color) {
        let bpp = self.format.bytes_per_pixel();
        for chunk in self.data.chunks_exact_mut(bpp) {
            Self::write_color(self.format, chunk, color);
        }
    }

    /// Alpha-blend `color` over every pixel of `rect` (intersected with the
    /// active clip), the C++ `DrawBoxDw`/`DrawBoxFade` filled-box primitive used
    /// for menu/dialog backgrounds and bars. A fully opaque colour overwrites; a
    /// translucent one composites.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let region = match rect.intersection(self.clip_bounds()) {
            Some(r) => r,
            None => return,
        };
        let bpp = self.format.bytes_per_pixel();
        for row in 0..region.height {
            let y = (region.y + row as i32) as u32;
            let row_off = self.pixel_offset(region.x as u32, y);
            for col in 0..region.width {
                let off = row_off + col as usize * bpp;
                let dst = Self::read_color(self.format, &self.data[off..off + bpp]);
                let blended = color.blend_over(dst);
                Self::write_color(self.format, &mut self.data[off..off + bpp], blended);
            }
        }
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) -> Result<(), SurfaceError> {
        if x >= self.width || y >= self.height {
            return Err(SurfaceError::OutOfBounds {
                x,
                y,
                width: self.width,
                height: self.height,
            });
        }
        if !self.pixel_in_clip(x, y) {
            return Ok(());
        }
        let bpp = self.format.bytes_per_pixel();
        let offset = self.pixel_offset(x, y);
        let slice = &mut self.data[offset..offset + bpp];
        Self::write_color(self.format, slice, color);
        Ok(())
    }

    pub fn blend_pixel(&mut self, x: u32, y: u32, color: Color) -> Result<(), SurfaceError> {
        if x >= self.width || y >= self.height {
            return Err(SurfaceError::OutOfBounds {
                x,
                y,
                width: self.width,
                height: self.height,
            });
        }
        if !self.pixel_in_clip(x, y) {
            return Ok(());
        }
        let bpp = self.format.bytes_per_pixel();
        let offset = self.pixel_offset(x, y);
        let slice = &mut self.data[offset..offset + bpp];
        let existing = Self::read_color(self.format, slice);
        let blended = color.blend_over(existing);
        Self::write_color(self.format, slice, blended);
        Ok(())
    }

    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let bpp = self.format.bytes_per_pixel();
        let offset = self.pixel_offset(x, y);
        let slice = &self.data[offset..offset + bpp];
        Some(Self::read_color(self.format, slice))
    }

    pub fn blit(&mut self, src: &Surface, dest: Point) -> Result<(), SurfaceError> {
        self.blit_region(src, Rect::new(0, 0, src.width, src.height), dest)
    }

    pub fn blit_region(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest: Point,
    ) -> Result<(), SurfaceError> {
        // No modulation: white is the identity in the C++ GL renderer
        // (glColor4ub(255,255,255,255) is a normalized 1.0 multiply).
        self.blit_region_modulated(src, src_rect, dest, Color::opaque(255, 255, 255))
    }

    /// Blit `src_rect` of `src` to `dest`, modulating every source pixel by
    /// `modulation` before an alpha-over composite (Normal mode). An opaque white
    /// modulation is treated as the identity (matching the GL renderer, which
    /// multiplies by normalized 1.0), so unmodulated draws are byte-exact and only
    /// genuine tints pay the `(a*b)>>8` modulation (see `Color::modulate_clr`).
    pub fn blit_region_modulated(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest: Point,
        modulation: Color,
    ) -> Result<(), SurfaceError> {
        self.blit_region_ex(src, src_rect, dest, modulation, BlitMode::Normal)
    }

    /// Full blit: prepare each source pixel with `modulation`, then composite
    /// onto the destination per `mode` (`StdDDraw2::Blit` + `dwBlitMode`).
    /// White is the Normal-mode GL identity; MOD2 instead applies its
    /// ADD_SIGNED*2 equation even for white.
    pub fn blit_region_ex(
        &mut self,
        src: &Surface,
        mut src_rect: Rect,
        mut dest: Point,
        modulation: Color,
        mode: BlitMode,
    ) -> Result<(), SurfaceError> {
        if self.format != src.format {
            return Err(SurfaceError::FormatMismatch {
                src: src.format,
                dst: self.format,
            });
        }

        let src_bounds = src.bounds();
        src_rect = match src_rect.intersection(src_bounds) {
            Some(r) => r,
            None => return Ok(()),
        };

        if src_rect.width == 0 || src_rect.height == 0 {
            return Ok(());
        }

        if dest.x < 0 {
            let shift = dest.x.saturating_neg() as u32;
            if shift >= src_rect.width {
                return Ok(());
            }
            src_rect.x += shift as i32;
            src_rect.width -= shift;
            dest.x = 0;
        }

        if dest.y < 0 {
            let shift = dest.y.saturating_neg() as u32;
            if shift >= src_rect.height {
                return Ok(());
            }
            src_rect.y += shift as i32;
            src_rect.height -= shift;
            dest.y = 0;
        }

        // Clip the destination to the active clip rect (∩ surface bounds). When a
        // clip rect has a non-zero origin we must also advance the source so the
        // sampled pixels stay aligned with the clipped destination.
        let dest_bounds = self.clip_bounds();
        if dest.x < dest_bounds.x {
            let shift = (dest_bounds.x - dest.x) as u32;
            if shift >= src_rect.width {
                return Ok(());
            }
            src_rect.x += shift as i32;
            src_rect.width -= shift;
            dest.x = dest_bounds.x;
        }
        if dest.y < dest_bounds.y {
            let shift = (dest_bounds.y - dest.y) as u32;
            if shift >= src_rect.height {
                return Ok(());
            }
            src_rect.y += shift as i32;
            src_rect.height -= shift;
            dest.y = dest_bounds.y;
        }
        let dest_right = dest_bounds.x + dest_bounds.width as i32;
        let dest_bottom = dest_bounds.y + dest_bounds.height as i32;
        if dest.x >= dest_right || dest.y >= dest_bottom {
            return Ok(());
        }

        let max_width = (dest_right - dest.x) as u32;
        if src_rect.width > max_width {
            src_rect.width = max_width;
        }

        let max_height = (dest_bottom - dest.y) as u32;
        if src_rect.height > max_height {
            src_rect.height = max_height;
        }

        if src_rect.width == 0 || src_rect.height == 0 {
            return Ok(());
        }

        let bpp = self.format.bytes_per_pixel();
        for row in 0..src_rect.height {
            let src_y = (src_rect.y + row as i32) as u32;
            let dest_y = (dest.y + row as i32) as u32;
            let src_row_offset = src.pixel_offset(src_rect.x as u32, src_y);
            let dest_row_offset = self.pixel_offset(dest.x as u32, dest_y);

            for col in 0..src_rect.width {
                let src_offset = src_row_offset + col as usize * bpp;
                let dest_offset = dest_row_offset + col as usize * bpp;

                let slice = &src.data[src_offset..src_offset + bpp];
                let raw = Self::read_color(src.format, slice);
                let source = mode.prepare_source(raw, modulation);
                let destination = {
                    let slice = &self.data[dest_offset..dest_offset + bpp];
                    Self::read_color(self.format, slice)
                };
                let blended = Self::composite(source, destination, mode);
                {
                    let slice = &mut self.data[dest_offset..dest_offset + bpp];
                    Self::write_color(self.format, slice, blended);
                }
            }
        }

        Ok(())
    }

    /// Homogeneous 3x3 transformed blit (rotation/scale/mirror/projective),
    /// the C++ `CBltTransform` path used for object sprites. The `src_rect` is
    /// conceptually placed at `dest_origin` and then `transform` is applied in
    /// destination space; each covered destination pixel is inverse-mapped
    /// back to source space, sampled nearest-neighbour, prepared according to
    /// `mode`, and composited. A non-invertible transform, or a projective quad
    /// crossing the horizon, draws nothing.
    pub fn blit_transformed(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest_origin: Point,
        transform: &crate::transform::Transform,
        modulation: Color,
        mode: BlitMode,
    ) -> Result<(), SurfaceError> {
        self.blit_transformed_impl(
            src,
            src_rect,
            dest_origin,
            transform,
            Some((modulation, mode)),
        )
    }

    /// Transformed nearest-neighbour copy with no modulation or framebuffer
    /// blend. Picture-cache code uses this to preserve straight-alpha texels
    /// until it can composite them with the cache's explicit alpha model.
    pub fn copy_transformed(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest_origin: Point,
        transform: &crate::transform::Transform,
    ) -> Result<(), SurfaceError> {
        self.blit_transformed_impl(src, src_rect, dest_origin, transform, None)
    }

    fn blit_transformed_impl(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest_origin: Point,
        transform: &crate::transform::Transform,
        composite: Option<(Color, BlitMode)>,
    ) -> Result<(), SurfaceError> {
        if self.format != src.format {
            return Err(SurfaceError::FormatMismatch {
                src: src.format,
                dst: self.format,
            });
        }
        let src_rect = match src_rect.intersection(src.bounds()) {
            Some(r) => r,
            None => return Ok(()),
        };
        if src_rect.width == 0 || src_rect.height == 0 {
            return Ok(());
        }
        let inv = match transform.inverse() {
            Some(t) => t,
            None => return Ok(()),
        };
        // Forward-transform the four corners of the dest-placed rect to find the
        // destination bounding box to rasterise.
        let (ox, oy) = (dest_origin.x as f32, dest_origin.y as f32);
        let (w, h) = (src_rect.width as f32, src_rect.height as f32);
        let corners = [(ox, oy), (ox + w, oy), (ox, oy + h), (ox + w, oy + h)];
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut positive_w = false;
        let mut negative_w = false;
        for &(cx, cy) in &corners {
            let w = transform.mat[6] * cx + transform.mat[7] * cy + transform.mat[8];
            if !w.is_finite() || w == 0.0 {
                return Ok(());
            }
            positive_w |= w.is_sign_positive();
            negative_w |= w.is_sign_negative();
            // A linear homogeneous denominator with both signs at the quad's
            // corners crosses zero somewhere inside. Its image is unbounded;
            // skip it instead of constructing an overflowing raster box.
            if positive_w && negative_w {
                return Ok(());
            }
            let (tx, ty) = transform.transform_point(cx, cy);
            if !tx.is_finite() || !ty.is_finite() {
                return Ok(());
            }
            min_x = min_x.min(tx);
            min_y = min_y.min(ty);
            max_x = max_x.max(tx);
            max_y = max_y.max(ty);
        }
        // Clip in floating-point space before integer conversion. Besides
        // avoiding a giant temporary rectangle, this keeps very large but
        // finite projective coordinates away from i32/u32 overflow paths.
        let clip = self.clip_bounds();
        if clip.width == 0 || clip.height == 0 {
            return Ok(());
        }
        let clip_left = clip.x as f32;
        let clip_top = clip.y as f32;
        let clip_right = clip_left + clip.width as f32;
        let clip_bottom = clip_top + clip.height as f32;
        let left = min_x.floor().max(clip_left);
        let top = min_y.floor().max(clip_top);
        let right = max_x.ceil().min(clip_right);
        let bottom = max_y.ceil().min(clip_bottom);
        if left >= right || top >= bottom {
            return Ok(());
        }
        let clipped = Rect::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        );
        let bpp = self.format.bytes_per_pixel();
        for row in 0..clipped.height {
            let dest_y = clipped.y + row as i32;
            for col in 0..clipped.width {
                let dest_x = clipped.x + col as i32;
                // Inverse-map the pixel centre back to source-local coordinates.
                let (bx, by) = inv.transform_point(dest_x as f32 + 0.5, dest_y as f32 + 0.5);
                let local_x = bx - ox;
                let local_y = by - oy;
                if !local_x.is_finite() || !local_y.is_finite() {
                    continue;
                }
                let lx = local_x.floor();
                let ly = local_y.floor();
                if lx < 0.0 || ly < 0.0 || lx >= w || ly >= h {
                    continue;
                }
                let src_x = src_rect.x as u32 + lx as u32;
                let src_y = src_rect.y as u32 + ly as u32;
                let off = src.pixel_offset(src_x, src_y);
                let raw = Self::read_color(src.format, &src.data[off..off + bpp]);
                let dest_off = self.pixel_offset(dest_x as u32, dest_y as u32);
                let output = if let Some((modulation, mode)) = composite {
                    let source = mode.prepare_source(raw, modulation);
                    let destination =
                        Self::read_color(self.format, &self.data[dest_off..dest_off + bpp]);
                    Self::composite(source, destination, mode)
                } else {
                    raw
                };
                Self::write_color(
                    self.format,
                    &mut self.data[dest_off..dest_off + bpp],
                    output,
                );
            }
        }
        Ok(())
    }

    fn composite(source: Color, destination: Color, mode: BlitMode) -> Color {
        match mode {
            BlitMode::Normal => source.blend_over(destination),
            BlitMode::Additive => source.blend_additive(destination),
            BlitMode::Mod2 => source.blend_shader_over(destination),
            BlitMode::Mod2Additive => source.blend_shader_additive(destination),
        }
    }

    /// Stretched point blit: sample `src_rect` of `src` into the (possibly
    /// differently-sized) `dest_rect` with nearest-neighbour sampling. This CPU
    /// primitive does not select C++'s runtime GPU filtering policy; callers
    /// that need linear filtering must use a filtering-aware renderer. Each
    /// sampled pixel is prepared with `modulation` according to `mode` and then
    /// composited. Clipped to the destination.
    pub fn blit_stretched(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest_rect: Rect,
        modulation: Color,
        mode: BlitMode,
    ) -> Result<(), SurfaceError> {
        if self.format != src.format {
            return Err(SurfaceError::FormatMismatch {
                src: src.format,
                dst: self.format,
            });
        }
        let src_rect = match src_rect.intersection(src.bounds()) {
            Some(r) => r,
            None => return Ok(()),
        };
        if dest_rect.width == 0
            || dest_rect.height == 0
            || src_rect.width == 0
            || src_rect.height == 0
        {
            return Ok(());
        }
        // Clip the destination rectangle to the surface, tracking the offset into
        // it so source sampling stays aligned.
        let clipped = match dest_rect.intersection(self.clip_bounds()) {
            Some(r) => r,
            None => return Ok(()),
        };
        let bpp = self.format.bytes_per_pixel();
        for row in 0..clipped.height {
            let dest_y = (clipped.y + row as i32) as u32;
            let local_y = (clipped.y - dest_rect.y) as u32 + row;
            let src_y = src_rect.y as u32 + (local_y * src_rect.height) / dest_rect.height;
            for col in 0..clipped.width {
                let dest_x = (clipped.x + col as i32) as u32;
                let local_x = (clipped.x - dest_rect.x) as u32 + col;
                let src_x = src_rect.x as u32 + (local_x * src_rect.width) / dest_rect.width;
                let off = src.pixel_offset(src_x, src_y);
                let raw = Self::read_color(src.format, &src.data[off..off + bpp]);
                let source = mode.prepare_source(raw, modulation);
                let dest_off = self.pixel_offset(dest_x, dest_y);
                let destination =
                    Self::read_color(self.format, &self.data[dest_off..dest_off + bpp]);
                let blended = Self::composite(source, destination, mode);
                Self::write_color(
                    self.format,
                    &mut self.data[dest_off..dest_off + bpp],
                    blended,
                );
            }
        }
        Ok(())
    }

    fn pixel_offset(&self, x: u32, y: u32) -> usize {
        y as usize * self.stride + x as usize * self.format.bytes_per_pixel()
    }

    fn read_color(format: PixelFormat, bytes: &[u8]) -> Color {
        match format {
            PixelFormat::Rgba8888 => Color::new(bytes[0], bytes[1], bytes[2], bytes[3]),
        }
    }

    fn write_color(format: PixelFormat, bytes: &mut [u8], color: Color) {
        match format {
            PixelFormat::Rgba8888 => {
                bytes[0] = color.r;
                bytes[1] = color.g;
                bytes[2] = color.b;
                bytes[3] = color.a;
            }
        }
    }
}

impl SurfaceDrawTarget for Surface {
    fn width(&self) -> u32 {
        Surface::width(self)
    }

    fn height(&self) -> u32 {
        Surface::height(self)
    }

    fn clip(&self) -> Option<Rect> {
        Surface::clip(self)
    }

    fn set_clip(&mut self, clip: Rect) {
        Surface::set_clip(self, clip);
    }

    fn clear_clip(&mut self) {
        Surface::clear_clip(self);
    }

    fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        Surface::get_pixel(self, x, y)
    }

    fn set_pixel(&mut self, x: u32, y: u32, color: Color) -> Result<(), SurfaceError> {
        Surface::set_pixel(self, x, y, color)
    }

    fn capture_clonk_text(&mut self, command: CapturedClonkText) -> bool {
        Surface::capture_clonk_text(self, command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clonk_font::{CapturedFontImage, ClonkFontRole, TextAlign};
    use crate::color::Color;
    use rand::{rngs::SmallRng, Rng, SeedableRng};

    fn captured_text(clip: Option<Rect>) -> CapturedClonkText {
        CapturedClonkText {
            role: ClonkFontRole::GuiText,
            x: 2,
            y: 3,
            text: "child".to_owned(),
            color: [255, 255, 255, 255],
            align: TextAlign::Left,
            markup: false,
            clip,
            gamma: None,
            images: Vec::new(),
        }
    }

    #[test]
    fn child_text_capture_makes_implicit_bounds_explicit_before_translation() {
        let mut child = Surface::new(6, 4, PixelFormat::Rgba8888);
        child.begin_clonk_text_capture();
        assert!(child.capture_clonk_text(captured_text(None)));

        let mut destination = Surface::new(30, 30, PixelFormat::Rgba8888);
        destination.begin_clonk_text_capture();
        assert!(destination.extend_clonk_text_capture_from(
            &mut child,
            Point::new(10, 7),
        ));

        let commands = destination.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!((commands[0].x, commands[0].y), (12, 10));
        assert_eq!(commands[0].clip, Some(Rect::new(10, 7, 6, 4)));
        assert!(!child.is_clonk_text_capture_active());
    }

    #[test]
    fn child_text_capture_intersects_existing_clip_before_translation() {
        let mut child = Surface::new(6, 4, PixelFormat::Rgba8888);
        child.begin_clonk_text_capture();
        assert!(child.capture_clonk_text(captured_text(Some(Rect::new(-2, 1, 5, 8)))));

        let mut destination = Surface::new(30, 30, PixelFormat::Rgba8888);
        destination.begin_clonk_text_capture();
        assert!(destination.extend_clonk_text_capture_from(
            &mut child,
            Point::new(10, 7),
        ));

        let commands = destination.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].clip,
            Some(Rect::new(10, 8, 3, 3)),
            "the local clip is intersected with 6x4 child bounds, then translated"
        );
    }

    #[test]
    fn modulated_child_text_capture_matches_c4_layer_modulation() {
        let mut command = captured_text(None);
        command.images.push(CapturedFontImage {
            tag: "ICON".to_owned(),
            width: 1,
            height: 1,
            rgba: vec![255, 128, 0, 255],
        });
        let mut child = Surface::new(6, 4, PixelFormat::Rgba8888);
        child.begin_clonk_text_capture();
        assert!(child.capture_clonk_text(command));

        let mut destination = Surface::new(30, 30, PixelFormat::Rgba8888);
        destination.begin_clonk_text_capture();
        assert!(destination.extend_clonk_text_capture_from_modulated(
            &mut child,
            Point::new(10, 7),
            Color::new(255, 255, 255, 0xaf),
        ));

        let commands = destination.take_clonk_text_capture();
        assert_eq!(commands[0].color, [254, 254, 254, 80]);
        assert_eq!(commands[0].images[0].rgba, [254, 127, 0, 80]);
        assert_eq!(commands[0].clip, Some(Rect::new(10, 7, 6, 4)));
    }

    #[test]
    fn fill_sets_all_pixels() {
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        let color = Color::opaque(10, 20, 30);
        surface.fill(color);

        for y in 0..4u32 {
            for x in 0..4u32 {
                assert_eq!(surface.get_pixel(x, y), Some(color));
            }
        }
    }

    #[test]
    fn set_and_get_pixel() {
        let mut surface = Surface::new(2, 2, PixelFormat::Rgba8888);
        let color = Color::new(1, 2, 3, 4);
        surface.set_pixel(1, 1, color).unwrap();
        assert_eq!(surface.get_pixel(1, 1), Some(color));
    }

    #[test]
    fn blit_with_alpha() {
        let mut dest = Surface::new(2, 2, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(0, 0, 0));

        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.fill(Color::new(255, 0, 0, 128));

        dest.blit(&src, Point::new(0, 0)).unwrap();

        let expected = Color::new(128, 0, 0, 255);
        assert_eq!(dest.get_pixel(0, 0), Some(expected));
    }

    #[test]
    fn blit_modulated_tints_source() {
        let mut dest = Surface::new(1, 1, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(0, 0, 0));

        let mut src = Surface::new(1, 1, PixelFormat::Rgba8888);
        src.fill(Color::new(200, 200, 200, 255));

        // Modulate by a red tint: r=(200*128)>>8=100, g=b=0, opaque → over black.
        dest.blit_region_modulated(
            &src,
            Rect::new(0, 0, 1, 1),
            Point::new(0, 0),
            Color::new(128, 0, 0, 0),
        )
        .unwrap();
        assert_eq!(dest.get_pixel(0, 0), Some(Color::new(100, 0, 0, 255)));
    }

    #[test]
    fn blit_modulation_alpha_fades_without_revealing_transparent_pixels() {
        let mut dest = Surface::new(2, 1, PixelFormat::Rgba8888);
        let mut src = Surface::new(2, 1, PixelFormat::Rgba8888);
        src.set_pixel(0, 0, Color::opaque(255, 255, 255)).unwrap();
        src.set_pixel(1, 0, Color::new(10, 20, 30, 0)).unwrap();

        dest.blit_region_modulated(
            &src,
            Rect::new(0, 0, 2, 1),
            Point::new(0, 0),
            Color::new(255, 255, 255, 128),
        )
        .unwrap();

        assert_eq!(dest.get_pixel(0, 0).map(|pixel| pixel.a), Some(127));
        assert_eq!(dest.get_pixel(1, 0), Some(Color::transparent()));
    }

    #[test]
    fn blit_modulated_by_white_is_identity() {
        // White modulation must be byte-identical to a plain blit (GL identity),
        // not the (255*255)>>8=254 software darkening.
        let mut a = Surface::new(2, 2, PixelFormat::Rgba8888);
        let mut b = Surface::new(2, 2, PixelFormat::Rgba8888);
        a.fill(Color::opaque(10, 20, 30));
        b.fill(Color::opaque(10, 20, 30));

        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.fill(Color::new(240, 250, 255, 200));

        a.blit(&src, Point::new(0, 0)).unwrap();
        b.blit_region_modulated(
            &src,
            Rect::new(0, 0, 2, 2),
            Point::new(0, 0),
            Color::opaque(255, 255, 255),
        )
        .unwrap();
        assert_eq!(a.get_pixel(0, 0), b.get_pixel(0, 0));
        assert_eq!(a.get_pixel(1, 1), b.get_pixel(1, 1));
    }

    #[test]
    fn blit_additive_mode_adds_to_destination() {
        let mut dest = Surface::new(1, 1, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(100, 50, 0));
        let mut src = Surface::new(1, 1, PixelFormat::Rgba8888);
        src.fill(Color::new(200, 100, 50, 128));
        dest.blit_region_ex(
            &src,
            Rect::new(0, 0, 1, 1),
            Point::new(0, 0),
            Color::opaque(255, 255, 255),
            BlitMode::Additive,
        )
        .unwrap();
        // dst + src*srcAlpha: (100+100, 50+50, 0+25, dst.a) = (200,100,25,255).
        assert_eq!(dest.get_pixel(0, 0), Some(Color::new(200, 100, 25, 255)));
    }

    #[test]
    fn blit_mod2_prepares_source_independently_of_destination() {
        let mut src = Surface::new(2, 1, PixelFormat::Rgba8888);
        src.fill(Color::opaque(64, 128, 192));
        let mut dest = Surface::new(2, 1, PixelFormat::Rgba8888);
        dest.set_pixel(0, 0, Color::opaque(10, 20, 30)).unwrap();
        dest.set_pixel(1, 0, Color::opaque(210, 120, 40)).unwrap();

        dest.blit_region_ex(
            &src,
            Rect::new(0, 0, 2, 1),
            Point::new(0, 0),
            Color::new(32, 64, 128, 0),
            BlitMode::Mod2,
        )
        .unwrap();

        // Live StdGL: clamp(2*S + 2*M - 255), never source-vs-destination.
        let prepared = Some(Color::opaque(0, 129, 255));
        assert_eq!(dest.get_pixel(0, 0), prepared);
        assert_eq!(dest.get_pixel(1, 0), prepared);

        let mut pivot = Surface::new(2, 1, PixelFormat::Rgba8888);
        pivot
            .blit_region_ex(
                &src,
                Rect::new(0, 0, 2, 1),
                Point::new(0, 0),
                Color::new(0x7f, 0x7f, 0x7f, 0),
                BlitMode::Mod2,
            )
            .unwrap();
        assert_eq!(
            pivot.get_pixel(0, 0),
            Some(Color::opaque(127, 255, 255)),
            "0x7f is not an identity: live GL yields clamp(2*S-1)",
        );
    }

    #[test]
    fn blit_mod2_keeps_source_alpha_and_resets_only_for_exact_zero() {
        let mut src = Surface::new(1, 1, PixelFormat::Rgba8888);
        src.fill(Color::new(64, 128, 192, 128));
        let background = Color::opaque(10, 20, 30);

        let render = |modulation| {
            let mut dest = Surface::new(1, 1, PixelFormat::Rgba8888);
            dest.fill(background);
            dest.blit_region_ex(
                &src,
                Rect::new(0, 0, 1, 1),
                Point::new(0, 0),
                modulation,
                BlitMode::Mod2,
            )
            .unwrap();
            dest.get_pixel(0, 0)
        };

        assert_eq!(
            render(Color::new(32, 64, 128, 255)),
            Some(Color::opaque(5, 75, 143)),
            "MOD2 ignores the modulation high byte and blends with source alpha 128",
        );
        assert_eq!(
            render(Color::new(0x7f, 0x7f, 0x7f, 0)),
            Some(Color::opaque(69, 138, 143)),
            "the live 0x7f pivot produces clamp(2*S-1) and preserves partial alpha",
        );
        assert_eq!(
            render(Color::transparent()),
            Some(Color::opaque(5, 10, 15)),
            "packed zero disables MOD2, then ordinary black modulation applies",
        );
        assert_eq!(
            render(Color::new(0, 0, 0, 1)),
            Some(Color::opaque(5, 10, 80)),
            "a nonzero high byte keeps MOD2 enabled even with black RGB",
        );
    }

    #[test]
    fn blit_mod2_additive_modulates_before_framebuffer_addition() {
        let mut src = Surface::new(1, 1, PixelFormat::Rgba8888);
        src.fill(Color::new(64, 128, 192, 128));
        let mut dest = Surface::new(1, 1, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(10, 20, 30));

        dest.blit_region_ex(
            &src,
            Rect::new(0, 0, 1, 1),
            Point::new(0, 0),
            Color::new(32, 64, 128, 0),
            BlitMode::Mod2Additive,
        )
        .unwrap();

        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(10, 85, 158)));
    }

    #[test]
    fn stretched_and_transformed_blits_share_live_mod2_source_preparation() {
        use crate::transform::Transform;

        let mut src = Surface::new(1, 1, PixelFormat::Rgba8888);
        src.fill(Color::opaque(64, 128, 192));
        let modulation = Color::new(32, 64, 128, 0);
        let expected = Some(Color::opaque(0, 129, 255));

        let mut stretched = Surface::new(2, 2, PixelFormat::Rgba8888);
        stretched
            .blit_stretched(
                &src,
                Rect::new(0, 0, 1, 1),
                Rect::new(0, 0, 2, 2),
                modulation,
                BlitMode::Mod2,
            )
            .unwrap();
        assert_eq!(stretched.get_pixel(1, 1), expected);

        let mut transformed = Surface::new(1, 1, PixelFormat::Rgba8888);
        transformed
            .blit_transformed(
                &src,
                Rect::new(0, 0, 1, 1),
                Point::new(0, 0),
                &Transform::identity(),
                modulation,
                BlitMode::Mod2,
            )
            .unwrap();
        assert_eq!(transformed.get_pixel(0, 0), expected);
    }

    #[test]
    fn blit_stretched_is_explicit_point_sampling() {
        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.set_pixel(0, 0, Color::opaque(255, 0, 0)).unwrap();
        src.set_pixel(1, 0, Color::opaque(0, 255, 0)).unwrap();
        src.set_pixel(0, 1, Color::opaque(0, 0, 255)).unwrap();
        src.set_pixel(1, 1, Color::opaque(255, 255, 255)).unwrap();

        let mut dest = Surface::new(4, 4, PixelFormat::Rgba8888);
        dest.blit_stretched(
            &src,
            Rect::new(0, 0, 2, 2),
            Rect::new(0, 0, 4, 4),
            Color::opaque(255, 255, 255),
            BlitMode::Normal,
        )
        .unwrap();
        // This point-only primitive maps each source pixel to a 2x2 block.
        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(255, 0, 0)));
        assert_eq!(dest.get_pixel(1, 1), Some(Color::opaque(255, 0, 0)));
        assert_eq!(dest.get_pixel(2, 0), Some(Color::opaque(0, 255, 0)));
        assert_eq!(dest.get_pixel(0, 3), Some(Color::opaque(0, 0, 255)));
        assert_eq!(dest.get_pixel(3, 3), Some(Color::opaque(255, 255, 255)));
    }

    #[test]
    fn blit_stretched_clips_to_destination() {
        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.fill(Color::opaque(10, 20, 30));
        let mut dest = Surface::new(4, 4, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(0, 0, 0));
        // dest_rect extends past the surface; must clip without panicking.
        dest.blit_stretched(
            &src,
            Rect::new(0, 0, 2, 2),
            Rect::new(2, 2, 4, 4),
            Color::opaque(255, 255, 255),
            BlitMode::Normal,
        )
        .unwrap();
        assert_eq!(dest.get_pixel(3, 3), Some(Color::opaque(10, 20, 30)));
        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn blit_transformed_identity_matches_plain_blit() {
        use crate::transform::Transform;
        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.set_pixel(0, 0, Color::opaque(255, 0, 0)).unwrap();
        src.set_pixel(1, 1, Color::opaque(0, 255, 0)).unwrap();

        let mut a = Surface::new(2, 2, PixelFormat::Rgba8888);
        let mut b = Surface::new(2, 2, PixelFormat::Rgba8888);
        a.blit(&src, Point::new(0, 0)).unwrap();
        b.blit_transformed(
            &src,
            Rect::new(0, 0, 2, 2),
            Point::new(0, 0),
            &Transform::identity(),
            Color::opaque(255, 255, 255),
            BlitMode::Normal,
        )
        .unwrap();
        assert_eq!(a.get_pixel(0, 0), b.get_pixel(0, 0));
        assert_eq!(a.get_pixel(1, 1), b.get_pixel(1, 1));
    }

    #[test]
    fn blit_transformed_180_degrees_flips_corners() {
        use crate::transform::Transform;
        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.set_pixel(0, 0, Color::opaque(255, 0, 0)).unwrap(); // red
        src.set_pixel(1, 0, Color::opaque(0, 255, 0)).unwrap(); // green
        src.set_pixel(0, 1, Color::opaque(0, 0, 255)).unwrap(); // blue
        src.set_pixel(1, 1, Color::opaque(255, 255, 255)).unwrap(); // white

        let mut dest = Surface::new(2, 2, PixelFormat::Rgba8888);
        // 180° about the rect centre (1,1): a point-symmetry that swaps corners.
        let t = Transform::set_rotate(18000, 1.0, 1.0);
        dest.blit_transformed(
            &src,
            Rect::new(0, 0, 2, 2),
            Point::new(0, 0),
            &t,
            Color::opaque(255, 255, 255),
            BlitMode::Normal,
        )
        .unwrap();
        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(255, 255, 255))); // was (1,1)
        assert_eq!(dest.get_pixel(1, 1), Some(Color::opaque(255, 0, 0))); // was (0,0)
        assert_eq!(dest.get_pixel(0, 1), Some(Color::opaque(0, 255, 0))); // was (1,0)
        assert_eq!(dest.get_pixel(1, 0), Some(Color::opaque(0, 0, 255))); // was (0,1)
    }

    #[test]
    fn blit_transformed_uses_projective_inverse_for_sampling() {
        use crate::transform::Transform;
        let mut src = Surface::new(4, 2, PixelFormat::Rgba8888);
        let colors = [
            Color::opaque(255, 0, 0),
            Color::opaque(0, 255, 0),
            Color::opaque(0, 0, 255),
            Color::opaque(255, 255, 255),
        ];
        for y in 0..2 {
            for (x, color) in colors.into_iter().enumerate() {
                src.set_pixel(x as u32, y, color).unwrap();
            }
        }
        let background = Color::opaque(7, 9, 11);
        let mut dest = Surface::new(4, 2, PixelFormat::Rgba8888);
        dest.fill(background);

        // x' = x / (0.1*x + 1): the right edge contracts toward the left.
        let transform = Transform::set(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.1, 0.0, 1.0);
        dest.blit_transformed(
            &src,
            Rect::new(0, 0, 4, 2),
            Point::new(0, 0),
            &transform,
            Color::opaque(255, 255, 255),
            BlitMode::Normal,
        )
        .unwrap();

        assert_eq!(dest.get_pixel(0, 0), Some(colors[0]));
        assert_eq!(dest.get_pixel(1, 0), Some(colors[1]));
        assert_eq!(
            dest.get_pixel(2, 0),
            Some(colors[3]),
            "general inverse samples source column 3, not affine column 2"
        );
        assert_eq!(dest.get_pixel(2, 1), Some(background));
    }

    #[test]
    fn blit_transformed_rejects_non_finite_or_horizon_crossing_quads() {
        use crate::transform::Transform;
        let mut src = Surface::new(2, 2, PixelFormat::Rgba8888);
        src.fill(Color::opaque(255, 0, 0));
        let background = Color::opaque(3, 5, 7);

        for transform in [
            // w=x: the left corners transform through division by zero.
            Transform::set(0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0),
            // w=x-1 crosses the projective horizon through the source quad.
            Transform::set(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0),
            Transform::set(f32::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0),
        ] {
            let mut dest = Surface::new(3, 3, PixelFormat::Rgba8888);
            dest.fill(background);
            dest.blit_transformed(
                &src,
                Rect::new(0, 0, 2, 2),
                Point::new(0, 0),
                &transform,
                Color::opaque(255, 255, 255),
                BlitMode::Normal,
            )
            .unwrap();
            assert!(dest
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [background.r, background.g, background.b, background.a]));
        }
    }

    #[test]
    fn fill_rect_blends_and_respects_clip() {
        let mut s = Surface::new(4, 4, PixelFormat::Rgba8888);
        s.fill(Color::opaque(0, 0, 0));
        // Opaque fill of a sub-rect overwrites only that rect.
        s.fill_rect(Rect::new(1, 1, 2, 2), Color::opaque(255, 0, 0));
        assert_eq!(s.get_pixel(1, 1), Some(Color::opaque(255, 0, 0)));
        assert_eq!(s.get_pixel(0, 0), Some(Color::opaque(0, 0, 0)));
        // Translucent fill composites (half-white over black = grey-ish).
        s.fill_rect(Rect::new(0, 0, 4, 4), Color::new(255, 255, 255, 128));
        let p = s.get_pixel(0, 0).unwrap();
        assert!(p.r > 100 && p.r < 160, "blended r={}", p.r);
        // Clip confines the fill.
        s.fill(Color::opaque(0, 0, 0));
        s.set_clip(Rect::new(2, 2, 1, 1));
        s.fill_rect(Rect::new(0, 0, 4, 4), Color::opaque(9, 9, 9));
        assert_eq!(s.get_pixel(2, 2), Some(Color::opaque(9, 9, 9)));
        assert_eq!(s.get_pixel(0, 0), Some(Color::opaque(0, 0, 0)));
    }

    #[test]
    fn clip_rect_restricts_blit() {
        let mut dest = Surface::new(4, 4, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(0, 0, 0));
        dest.set_clip(Rect::new(1, 1, 2, 2)); // only the 2x2 block at (1,1)
        assert_eq!(dest.clip(), Some(Rect::new(1, 1, 2, 2)));
        dest.set_pixel(0, 0, Color::opaque(255, 0, 0))
            .expect("clipped pixel write is discarded");
        dest.blend_pixel(3, 3, Color::opaque(0, 255, 0))
            .expect("clipped blend is discarded");
        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(0, 0, 0)));
        assert_eq!(dest.get_pixel(3, 3), Some(Color::opaque(0, 0, 0)));

        let mut src = Surface::new(4, 4, PixelFormat::Rgba8888);
        src.fill(Color::opaque(255, 255, 255));
        dest.blit(&src, Point::new(0, 0)).unwrap();

        // Inside the clip: written. Outside: untouched.
        assert_eq!(dest.get_pixel(1, 1), Some(Color::opaque(255, 255, 255)));
        assert_eq!(dest.get_pixel(2, 2), Some(Color::opaque(255, 255, 255)));
        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(0, 0, 0)));
        assert_eq!(dest.get_pixel(3, 3), Some(Color::opaque(0, 0, 0)));
        assert_eq!(dest.get_pixel(0, 1), Some(Color::opaque(0, 0, 0)));

        // Clearing the clip restores full-surface drawing.
        dest.clear_clip();
        assert_eq!(dest.clip(), None);
        dest.blit(&src, Point::new(0, 0)).unwrap();
        assert_eq!(dest.get_pixel(0, 0), Some(Color::opaque(255, 255, 255)));
    }

    #[test]
    fn blit_clipped() {
        let mut dest = Surface::new(4, 4, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(10, 20, 30));

        let mut src = Surface::new(4, 4, PixelFormat::Rgba8888);
        // Seeded so the dev-dep needs no thread_rng (std_rng feature), keeping
        // rand's feature set identical between the build and test graphs.
        let mut rng = SmallRng::seed_from_u64(0x5EED);
        for y in 0..4u32 {
            for x in 0..4u32 {
                let color = Color::new(rng.gen(), rng.gen(), rng.gen(), 255);
                src.set_pixel(x, y, color).unwrap();
            }
        }

        dest.blit_region(&src, Rect::new(1, 1, 3, 3), Point::new(-1, -1))
            .unwrap();

        assert_eq!(dest.get_pixel(0, 0), src.get_pixel(2, 2));
    }

    #[test]
    fn snapshot_region_matches_surface_blit() {
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        for y in 0..4 {
            for x in 0..4 {
                let value = (y * 4 + x) as u8;
                surface
                    .set_pixel(x, y, Color::new(value, value.saturating_mul(2), value, 255))
                    .unwrap();
            }
        }

        let region = Rect::new(1, 1, 2, 2);
        let snapshot = surface.snapshot_region(region).expect("region snapshot");

        let mut expected = Surface::new(2, 2, PixelFormat::Rgba8888);
        expected
            .blit_region(&surface, region, Point::new(0, 0))
            .expect("copy succeeds");
        let expected_snapshot = expected.snapshot();

        assert_eq!(snapshot, expected_snapshot);
    }

    #[test]
    fn snapshot_region_outside_returns_none() {
        let surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        assert!(surface.snapshot_region(Rect::new(8, 8, 2, 2)).is_none());
    }

    #[test]
    fn snapshot_region_partially_outside_clamps() {
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        surface.set_pixel(3, 3, Color::opaque(200, 10, 10)).unwrap();
        let snapshot = surface
            .snapshot_region(Rect::new(3, 3, 5, 5))
            .expect("clamped snapshot");
        assert_eq!(snapshot.width(), 1);
        assert_eq!(snapshot.height(), 1);
        assert_eq!(
            snapshot,
            surface.snapshot_region(Rect::new(3, 3, 1, 1)).unwrap()
        );
    }
}
