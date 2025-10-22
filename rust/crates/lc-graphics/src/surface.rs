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
        mut src_rect: Rect,
        mut dest: Point,
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

        let dest_bounds = self.bounds();
        if dest.x >= dest_bounds.width as i32 || dest.y >= dest_bounds.height as i32 {
            return Ok(());
        }

        let max_width = dest_bounds.width.saturating_sub(dest.x as u32);
        if src_rect.width > max_width {
            src_rect.width = max_width;
        }

        let max_height = dest_bounds.height.saturating_sub(dest.y as u32);
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

                let source = {
                    let slice = &src.data[src_offset..src_offset + bpp];
                    Self::read_color(src.format, slice)
                };
                let destination = {
                    let slice = &self.data[dest_offset..dest_offset + bpp];
                    Self::read_color(self.format, slice)
                };
                let blended = source.blend_over(destination);
                {
                    let slice = &mut self.data[dest_offset..dest_offset + bpp];
                    Self::write_color(self.format, slice, blended);
                }
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
    use rand::Rng;

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
    fn blit_clipped() {
        let mut dest = Surface::new(4, 4, PixelFormat::Rgba8888);
        dest.fill(Color::opaque(10, 20, 30));

        let mut src = Surface::new(4, 4, PixelFormat::Rgba8888);
        let mut rng = rand::thread_rng();
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
