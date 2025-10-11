use crate::surface::Surface;
use std::fmt;

const FNV_OFFSET_BASIS: u32 = 0x811C9DC5;
const FNV_PRIME: u32 = 16777619;

pub(crate) const FNV_OFFSET: u32 = FNV_OFFSET_BASIS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceSnapshot {
    width: u32,
    height: u32,
    checksum: u32,
}

impl SurfaceSnapshot {
    pub fn from_surface(surface: &Surface) -> Self {
        let checksum = checksum(surface.pixels());
        Self::from_parts(surface.width(), surface.height(), checksum)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn checksum(&self) -> u32 {
        self.checksum
    }
}

impl fmt::Display for SurfaceSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}#{:08x}", self.width, self.height, self.checksum)
    }
}

impl SurfaceSnapshot {
    pub(crate) const fn from_parts(width: u32, height: u32, checksum: u32) -> Self {
        Self {
            width,
            height,
            checksum,
        }
    }
}

#[derive(Default)]
pub struct SnapshotHasher {
    state: u32,
    surfaces: u32,
}

impl SnapshotHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_surface(&mut self, surface: &Surface) {
        let snapshot = SurfaceSnapshot::from_surface(surface);
        self.update_snapshot(snapshot);
    }

    pub fn update_snapshot(&mut self, snapshot: SurfaceSnapshot) {
        self.state = self
            .state
            .wrapping_mul(FNV_PRIME)
            .wrapping_add(snapshot.width());
        self.state = self
            .state
            .wrapping_mul(FNV_PRIME)
            .wrapping_add(snapshot.height());
        self.state = self
            .state
            .wrapping_mul(FNV_PRIME)
            .wrapping_add(snapshot.checksum());
        self.surfaces = self.surfaces.wrapping_add(1);
    }

    pub fn finish(&self) -> u64 {
        ((self.surfaces as u64) << 32) | self.state as u64
    }
}

fn checksum(bytes: &[u8]) -> u32 {
    checksum_update(FNV_OFFSET_BASIS, bytes)
}

pub(crate) fn checksum_update(mut hash: u32, bytes: &[u8]) -> u32 {
    for &byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::surface::{PixelFormat, Surface};

    #[test]
    fn snapshot_is_deterministic() {
        let mut surface = Surface::new(2, 2, PixelFormat::Rgba8888);
        surface.set_pixel(0, 0, Color::opaque(1, 2, 3)).unwrap();
        surface.set_pixel(1, 0, Color::opaque(4, 5, 6)).unwrap();
        surface.set_pixel(0, 1, Color::new(7, 8, 9, 128)).unwrap();
        surface.set_pixel(1, 1, Color::new(10, 11, 12, 64)).unwrap();

        let snapshot_a = SurfaceSnapshot::from_surface(&surface);
        let snapshot_b = surface.snapshot();

        assert_eq!(snapshot_a, snapshot_b);
        assert_eq!(format!("{}", snapshot_a), "2x2#a5ea02c5");
    }

    #[test]
    fn snapshot_hasher_accumulates_surfaces() {
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface.set_pixel(0, 0, Color::opaque(255, 0, 0)).unwrap();
        let snapshot = surface.snapshot();

        let mut hasher = SnapshotHasher::new();
        hasher.update_snapshot(snapshot);
        hasher.update_snapshot(snapshot);
        assert_ne!(hasher.finish(), 0);
    }
}
