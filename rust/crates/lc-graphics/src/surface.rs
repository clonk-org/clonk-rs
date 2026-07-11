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
    /// `C4GFXBLIT_MOD2` — additive color modulation around 0x7f, alpha-weighted.
    Mod2,
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
        })
    }

    /// Set the clipping rectangle (C++ `SetPrimaryClipper`); subsequent draws are
    /// restricted to `clip ∩ bounds`. The rect is stored as given and intersected
    /// with the surface at draw time.
    pub fn set_clip(&mut self, clip: Rect) {
        self.clip = Some(clip);
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

    /// Full blit: modulate each source pixel by `modulation` (white = GL identity),
    /// then composite onto the destination per `mode` (`StdDDraw2::Blit` +
    /// `dwBlitMode`). The combination of `dwModClr` modulation and the blit mode is
    /// the path every engine draw flows through.
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
        let modulate = modulation != Color::opaque(255, 255, 255);

        for row in 0..src_rect.height {
            let src_y = (src_rect.y + row as i32) as u32;
            let dest_y = (dest.y + row as i32) as u32;
            let src_row_offset = src.pixel_offset(src_rect.x as u32, src_y);
            let dest_row_offset = self.pixel_offset(dest.x as u32, dest_y);

            for col in 0..src_rect.width {
                let src_offset = src_row_offset + col as usize * bpp;
                let dest_offset = dest_row_offset + col as usize * bpp;

                let source = {
                    let slice = &src.data[src_offset..src_offset + bpp];
                    let raw = Self::read_color(src.format, slice);
                    if modulate {
                        raw.modulate_clr(modulation)
                    } else {
                        raw
                    }
                };
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

    /// Affine-transformed blit (rotation/scale/mirror), the C++ `CBltTransform`
    /// path used for rotated object sprites. The `src_rect` is conceptually placed
    /// at `dest_origin` and then `transform` is applied in destination space; each
    /// covered destination pixel is inverse-mapped back to source space, sampled
    /// nearest-neighbour, modulated (white = identity) and composited per `mode`.
    /// A non-invertible transform draws nothing.
    pub fn blit_transformed(
        &mut self,
        src: &Surface,
        src_rect: Rect,
        dest_origin: Point,
        transform: &crate::transform::Transform,
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
        if src_rect.width == 0 || src_rect.height == 0 {
            return Ok(());
        }
        let inv = match transform.inverse_affine() {
            Some(t) => t,
            None => return Ok(()),
        };
        // Forward-transform the four corners of the dest-placed rect to find the
        // destination bounding box to rasterise.
        let (ox, oy) = (dest_origin.x as f32, dest_origin.y as f32);
        let (w, h) = (src_rect.width as f32, src_rect.height as f32);
        let corners = [(ox, oy), (ox + w, oy), (ox, oy + h), (ox + w, oy + h)];
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for &(cx, cy) in &corners {
            let (tx, ty) = transform.transform_point(cx, cy);
            min_x = min_x.min(tx);
            min_y = min_y.min(ty);
            max_x = max_x.max(tx);
            max_y = max_y.max(ty);
        }
        let bbox = Rect::new(
            min_x.floor() as i32,
            min_y.floor() as i32,
            (max_x.ceil() - min_x.floor()).max(0.0) as u32,
            (max_y.ceil() - min_y.floor()).max(0.0) as u32,
        );
        let clipped = match bbox.intersection(self.clip_bounds()) {
            Some(r) => r,
            None => return Ok(()),
        };
        let modulate = modulation != Color::opaque(255, 255, 255);
        let bpp = self.format.bytes_per_pixel();
        for row in 0..clipped.height {
            let dest_y = clipped.y + row as i32;
            for col in 0..clipped.width {
                let dest_x = clipped.x + col as i32;
                // Inverse-map the pixel centre back to source-local coordinates.
                let (bx, by) = inv.transform_point(dest_x as f32 + 0.5, dest_y as f32 + 0.5);
                let lx = (bx - ox).floor();
                let ly = (by - oy).floor();
                if lx < 0.0 || ly < 0.0 || lx >= w || ly >= h {
                    continue;
                }
                let src_x = src_rect.x as u32 + lx as u32;
                let src_y = src_rect.y as u32 + ly as u32;
                let source = {
                    let off = src.pixel_offset(src_x, src_y);
                    let raw = Self::read_color(src.format, &src.data[off..off + bpp]);
                    if modulate {
                        raw.modulate_clr(modulation)
                    } else {
                        raw
                    }
                };
                let dest_off = self.pixel_offset(dest_x as u32, dest_y as u32);
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

    fn composite(source: Color, destination: Color, mode: BlitMode) -> Color {
        match mode {
            BlitMode::Normal => source.blend_over(destination),
            BlitMode::Additive => source.blend_additive(destination),
            BlitMode::Mod2 => source.blend_mod2(destination),
        }
    }

    /// Stretched blit: sample `src_rect` of `src` into the (possibly
    /// differently-sized) `dest_rect` with nearest-neighbour sampling — the C++
    /// facet-scaling path (`StdGL::PerformBlt` texcoord stepping; pixel gfx use
    /// point sampling). Each sampled pixel is modulated by `modulation`
    /// (white = identity) and composited per `mode`. Clipped to the destination.
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
        let modulate = modulation != Color::opaque(255, 255, 255);
        let bpp = self.format.bytes_per_pixel();
        for row in 0..clipped.height {
            let dest_y = (clipped.y + row as i32) as u32;
            let local_y = (clipped.y - dest_rect.y) as u32 + row;
            let src_y = src_rect.y as u32 + (local_y * src_rect.height) / dest_rect.height;
            for col in 0..clipped.width {
                let dest_x = (clipped.x + col as i32) as u32;
                let local_x = (clipped.x - dest_rect.x) as u32 + col;
                let src_x = src_rect.x as u32 + (local_x * src_rect.width) / dest_rect.width;
                let source = {
                    let off = src.pixel_offset(src_x, src_y);
                    let raw = Self::read_color(src.format, &src.data[off..off + bpp]);
                    if modulate {
                        raw.modulate_clr(modulation)
                    } else {
                        raw
                    }
                };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use rand::{rngs::SmallRng, Rng, SeedableRng};

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
            Color::new(128, 0, 0, 255),
        )
        .unwrap();
        assert_eq!(dest.get_pixel(0, 0), Some(Color::new(100, 0, 0, 255)));
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
    fn blit_mod2_mode_combines_around_midgrey() {
        let mut dest = Surface::new(1, 1, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(0x7f, 0x7f, 0x7f));
        let mut src = Surface::new(1, 1, PixelFormat::Rgba8888);
        src.fill(Color::new(0x7f, 0x7f, 0x7f, 255));
        dest.blit_region_ex(
            &src,
            Rect::new(0, 0, 1, 1),
            Point::new(0, 0),
            Color::opaque(255, 255, 255),
            BlitMode::Mod2,
        )
        .unwrap();
        // (0x7f+0x7f-0x7f)*2 = 0xfe per channel; dest alpha preserved.
        assert_eq!(
            dest.get_pixel(0, 0),
            Some(Color::new(0xfe, 0xfe, 0xfe, 255))
        );
    }

    #[test]
    fn blit_stretched_2x_nearest_neighbour() {
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
        // 2x upscale: each src pixel fills a 2x2 block.
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
