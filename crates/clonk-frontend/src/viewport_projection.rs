//! Local window coordinates to world coordinates, per viewport.
//!
//! `C4Viewport`'s Windows message handlers convert a window-local pointer
//! position through *that viewport's* scale and view origin
//! (`C4Viewport.cpp:112,181,192`):
//!
//! ```cpp
//! Console.EditCursor.Move(cvp->ViewX + static_cast<int32_t>(LOWORD(lParam) / scale),
//!                         cvp->ViewY + static_cast<int32_t>(HIWORD(lParam) / scale), wParam);
//! ```
//!
//! The division is floating point and the cast truncates **toward zero**, which
//! differs from flooring once a pointer leaves the window on the top or left.
//! The origin is added after truncation, not before.

/// One viewport's projection inputs. Keyed per viewport because a detached
/// console viewport has its own scale and view origin — the last globally
/// rendered layout is not a substitute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportProjection {
    /// `C4Viewport::ViewX` — the world x at the viewport's left edge.
    pub view_x: i32,
    /// `C4Viewport::ViewY`.
    pub view_y: i32,
    /// The presenter scale this viewport's window is drawn at.
    pub scale: f32,
}

impl ViewportProjection {
    /// `ViewX + static_cast<int32_t>(local / scale)` (`C4Viewport.cpp:181`).
    ///
    /// A non-positive scale cannot divide, so the local offset is dropped and
    /// the view origin is returned — the same place a zero-sized viewport puts
    /// every pointer.
    pub fn world_position(self, local_x: i32, local_y: i32) -> (i32, i32) {
        // NaN is caught by `is_finite` rather than a negated comparison, which
        // would silently read as "scale <= 0" for a NaN.
        if !self.scale.is_finite() || self.scale <= 0.0 {
            return (self.view_x, self.view_y);
        }
        (
            self.view_x
                .saturating_add(truncate_toward_zero(local_x, self.scale)),
            self.view_y
                .saturating_add(truncate_toward_zero(local_y, self.scale)),
        )
    }
}

/// `static_cast<int32_t>(value / scale)` — C++ integer conversion truncates
/// toward zero, so -3 / 2.0 is -1, not -2.
fn truncate_toward_zero(value: i32, scale: f32) -> i32 {
    (value as f32 / scale) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4Viewport.cpp:112,181,192 — the origin is added after a
    // truncating-toward-zero division by that viewport's own scale.
    #[test]
    fn detached_viewport_pointer_projection_uses_window_identity_and_scale() {
        let unscaled = ViewportProjection {
            view_x: 100,
            view_y: 40,
            scale: 1.0,
        };
        assert_eq!(unscaled.world_position(0, 0), (100, 40));
        assert_eq!(unscaled.world_position(25, 10), (125, 50));

        // A scaled window divides the local offset before adding the origin.
        let doubled = ViewportProjection {
            scale: 2.0,
            ..unscaled
        };
        assert_eq!(doubled.world_position(0, 0), (100, 40));
        assert_eq!(doubled.world_position(50, 20), (125, 50));
        // Truncation, not rounding: 51/2 = 25.5 -> 25.
        assert_eq!(doubled.world_position(51, 21), (125, 50));

        // Toward zero, not floor — this is where a `floor` implementation
        // diverges, once the pointer leaves the window top-left.
        assert_eq!(doubled.world_position(-3, -3), (99, 39));
        assert_eq!(
            doubled.world_position(-1, -1),
            (100, 40),
            "-1/2 truncates to 0, so the origin is unchanged"
        );

        // A fractional scale is honoured as written.
        let fractional = ViewportProjection {
            scale: 1.5,
            ..unscaled
        };
        assert_eq!(fractional.world_position(3, 3), (102, 42));

        // Two viewports at the same local point project differently — the
        // whole reason projection state is keyed by physical identity.
        let other = ViewportProjection {
            view_x: 0,
            view_y: 0,
            scale: 1.0,
        };
        assert_ne!(
            unscaled.world_position(10, 10),
            other.world_position(10, 10)
        );

        // A degenerate scale yields the view origin rather than a wild value.
        for scale in [0.0, -1.0, f32::NAN] {
            let degenerate = ViewportProjection { scale, ..unscaled };
            assert_eq!(degenerate.world_position(500, 500), (100, 40));
        }
    }
}
