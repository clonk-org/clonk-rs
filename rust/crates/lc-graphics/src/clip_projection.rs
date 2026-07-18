use crate::Rect;

/// The physical GL viewport and logical projection installed for one C++
/// primary clipper (`StdGL.cpp:393-407`).
///
/// C++ rounds the viewport origin and extent before installing an orthographic
/// projection over the unrounded logical clip. Consequently, fractional
/// application scales must map draw coordinates relative to this clip rather
/// than multiplying absolute coordinates by the application scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipperProjection {
    logical_clip: Rect,
    physical_clip: Rect,
}

impl ClipperProjection {
    /// Build the top-down framebuffer counterpart of CStdGL::UpdateClipper.
    /// The logical size is the C++ render target; physical height belongs to
    /// the actual (possibly shorter) framebuffer receiving the viewport.
    pub fn new(
        scale: f32,
        logical_target_size: (u32, u32),
        physical_target_height: u32,
        logical_clip: Rect,
    ) -> Self {
        debug_assert!(scale.is_finite() && scale > 0.0);
        let logical_clip = logical_clip
            .intersection(Rect::new(
                0,
                0,
                logical_target_size.0,
                logical_target_size.1,
            ))
            .unwrap_or_else(|| Rect::new(0, 0, 0, 0));
        let left = scaled_floor(i64::from(logical_clip.x), scale);
        let viewport_bottom = scaled_floor(
            i64::from(logical_target_size.1)
                .saturating_sub(i64::from(logical_clip.y))
                .saturating_sub(i64::from(logical_clip.height)),
            scale,
        );
        let width = scaled_ceil_extent(logical_clip.width, scale);
        let height = scaled_ceil_extent(logical_clip.height, scale);
        let top = i64::from(physical_target_height)
            .saturating_sub(i64::from(viewport_bottom))
            .saturating_sub(i64::from(height))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;

        Self {
            logical_clip,
            physical_clip: Rect::new(left, top, width, height),
        }
    }

    pub fn logical_clip(self) -> Rect {
        self.logical_clip
    }

    pub fn physical_clip(self) -> Rect {
        self.physical_clip
    }

    /// Independent X/Y scales produced by the rounded viewport extents.
    pub fn scale(self) -> (f64, f64) {
        (
            extent_ratio(self.physical_clip.width, self.logical_clip.width),
            extent_ratio(self.physical_clip.height, self.logical_clip.height),
        )
    }

    /// Map one logical coordinate through the clip-relative orthographic
    /// projection into top-down physical framebuffer coordinates.
    pub fn logical_to_physical(self, x: f64, y: f64) -> (f64, f64) {
        let (scale_x, scale_y) = self.scale();
        (
            f64::from(self.physical_clip.x) + (x - f64::from(self.logical_clip.x)) * scale_x,
            f64::from(self.physical_clip.y) + (y - f64::from(self.logical_clip.y)) * scale_y,
        )
    }
}

fn scaled_floor(value: i64, scale: f32) -> i32 {
    ((value as f32) * scale)
        .floor()
        .clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn scaled_ceil_extent(value: u32, scale: f32) -> u32 {
    ((value as f32) * scale).ceil().clamp(0.0, u32::MAX as f32) as u32
}

fn extent_ratio(physical: u32, logical: u32) -> f64 {
    if logical == 0 {
        0.0
    } else {
        f64::from(physical) / f64::from(logical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_clipper_rounds_viewport_before_projecting_relative_points() {
        let projection = ClipperProjection::new(1.5, (4, 4), 6, Rect::new(1, 1, 2, 2));

        assert_eq!(projection.physical_clip(), Rect::new(1, 2, 3, 3));
        assert_eq!(projection.logical_to_physical(1.0, 1.0), (1.0, 2.0));
        assert_eq!(projection.logical_to_physical(2.0, 1.0), (2.5, 2.0));
    }
}
