//! 2D homogeneous blit transform, a faithful port of C++ `CBltTransform`
//! (`src/StdDDraw2.h:53`, `src/StdDDraw2.cpp`). Used for rotated/scaled/mirrored
//! object sprites (`SetObjDrawTransform`, action facets), projective
//! `SetObjDrawTransform2` matrices and HUD elements.
//!
//! The matrix layout matches C++ exactly: a row-major 3x3 homogeneous matrix
//! stored as `mat[0..9]`, applied to a point with a homogeneous divide.

/// Angle scale: `iAngle` is in 1/100-degrees; C++ uses `iAngle * -pi/18000`
/// (note the reversed sign — see `CBltTransform::SetRotate`).
const ANGLE_TO_RAD: f32 = -1.745_329_2e-4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub mat: [f32; 9],
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform {
    pub const fn identity() -> Self {
        Self {
            mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub const fn set(
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
        g: f32,
        h: f32,
        i: f32,
    ) -> Self {
        Self {
            mat: [a, b, c, d, e, f, g, h, i],
        }
    }

    /// Rotation by `angle` (1/100-degrees) about pivot `(off_x, off_y)`, mirroring
    /// C++ `CBltTransform::SetRotate` (`src/StdDDraw2.cpp:41`).
    pub fn set_rotate(angle: i32, off_x: f32, off_y: f32) -> Self {
        let a = angle as f32 * ANGLE_TO_RAD;
        let (fsin, fcos) = (a.sin(), a.cos());
        Self {
            mat: [
                fcos,
                fsin,
                (1.0 - fcos) * off_x - fsin * off_y,
                -fsin,
                fcos,
                (1.0 - fcos) * off_y + fsin * off_x,
                0.0,
                0.0,
                1.0,
            ],
        }
    }

    /// Move-and-scale, mirroring C++ `CBltTransform::SetMoveScale`.
    pub fn set_move_scale(dx: f32, dy: f32, sx: f32, sy: f32) -> Self {
        Self {
            mat: [sx, 0.0, dx, 0.0, sy, dy, 0.0, 0.0, 1.0],
        }
    }

    /// Apply the matrix to a point with homogeneous divide, mirroring C++
    /// `CBltTransform::TransformPoint`.
    pub fn transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        let m = &self.mat;
        let w = m[6] * x + m[7] * y + m[8];
        let tx = (m[0] * x + m[1] * y + m[2]) / w;
        let ty = (m[3] * x + m[4] * y + m[5]) / w;
        (tx, ty)
    }

    /// Matrix product `self * rhs`, mirroring C++ `operator*=`.
    pub fn multiply(&self, r: &Transform) -> Transform {
        let m = &self.mat;
        let n = &r.mat;
        Transform::set(
            m[0] * n[0] + m[3] * n[1] + m[6] * n[2],
            m[1] * n[0] + m[4] * n[1] + m[7] * n[2],
            m[2] * n[0] + m[5] * n[1] + m[8] * n[2],
            m[0] * n[3] + m[3] * n[4] + m[6] * n[5],
            m[1] * n[3] + m[4] * n[4] + m[7] * n[5],
            m[2] * n[3] + m[5] * n[4] + m[8] * n[5],
            m[0] * n[6] + m[3] * n[7] + m[6] * n[8],
            m[1] * n[6] + m[4] * n[7] + m[7] * n[8],
            m[2] * n[6] + m[5] * n[7] + m[8] * n[8],
        )
    }

    /// Inverse of the complete homogeneous 3x3 matrix, matching
    /// `CBltTransform::SetAsInv` (`src/StdDDraw2.cpp`). This includes the
    /// projective `g/h/i` row used by `SetObjDrawTransform2`.
    ///
    /// C++ rejects an exactly-zero determinant. Rust additionally rejects
    /// non-finite input/results so a malformed script transform cannot feed
    /// infinities or NaNs into the software rasterizer.
    pub fn inverse(&self) -> Option<Transform> {
        let m = &self.mat;
        if !m.iter().all(|component| component.is_finite()) {
            return None;
        }
        let det = m[0] * m[4] * m[8] + m[1] * m[5] * m[6] + m[2] * m[3] * m[7]
            - m[2] * m[4] * m[6]
            - m[0] * m[5] * m[7]
            - m[1] * m[3] * m[8];
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inverse = Transform::set(
            (m[4] * m[8] - m[5] * m[7]) / det,
            (m[2] * m[7] - m[1] * m[8]) / det,
            (m[1] * m[5] - m[2] * m[4]) / det,
            (m[5] * m[6] - m[3] * m[8]) / det,
            (m[0] * m[8] - m[2] * m[6]) / det,
            (m[2] * m[3] - m[0] * m[5]) / det,
            (m[3] * m[7] - m[4] * m[6]) / det,
            (m[1] * m[6] - m[0] * m[7]) / det,
            (m[0] * m[4] - m[1] * m[3]) / det,
        );
        inverse
            .mat
            .iter()
            .all(|component| component.is_finite())
            .then_some(inverse)
    }

    /// Inverse of an affine transform (`mat[6]=mat[7]=0, mat[8]=1`). Returns
    /// `None` for a degenerate (non-invertible) matrix. Retained for callers
    /// that explicitly operate on the affine subset; projective blits use
    /// [`Self::inverse`].
    pub fn inverse_affine(&self) -> Option<Transform> {
        let m = &self.mat;
        let (a, b, c) = (m[0], m[1], m[2]);
        let (d, e, f) = (m[3], m[4], m[5]);
        let det = a * e - b * d;
        if det.abs() < f32::EPSILON {
            return None;
        }
        Some(Transform::set(
            e / det,
            -b / det,
            (b * f - c * e) / det,
            -d / det,
            a / det,
            (c * d - a * f) / det,
            0.0,
            0.0,
            1.0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_leaves_point_unchanged() {
        let t = Transform::identity();
        assert_eq!(t.transform_point(12.0, -7.0), (12.0, -7.0));
    }

    #[test]
    fn rotate_180_about_origin_negates() {
        // 18000 centi-degrees = 180°. cos=-1, sin~0; point (1,0) -> (-1, 0).
        let t = Transform::set_rotate(18000, 0.0, 0.0);
        let (x, y) = t.transform_point(1.0, 0.0);
        assert!((x - -1.0).abs() < 1e-3, "x={x}");
        assert!(y.abs() < 1e-3, "y={y}");
    }

    #[test]
    fn rotate_90_about_pivot_keeps_pivot_fixed() {
        let t = Transform::set_rotate(9000, 5.0, 5.0);
        let (px, py) = t.transform_point(5.0, 5.0);
        assert!((px - 5.0).abs() < 1e-3 && (py - 5.0).abs() < 1e-3);
    }

    #[test]
    fn move_scale_applies_scale_then_translate() {
        let t = Transform::set_move_scale(10.0, 20.0, 2.0, 3.0);
        assert_eq!(
            t.transform_point(4.0, 5.0),
            (4.0 * 2.0 + 10.0, 5.0 * 3.0 + 20.0)
        );
    }

    #[test]
    fn inverse_round_trips() {
        let t = Transform::set_rotate(3500, 3.0, -2.0)
            .multiply(&Transform::set_move_scale(7.0, 1.0, 1.5, 0.5));
        let inv = t.inverse_affine().expect("invertible");
        let (x, y) = t.transform_point(9.0, -4.0);
        let (rx, ry) = inv.transform_point(x, y);
        assert!(
            (rx - 9.0).abs() < 1e-2 && (ry - -4.0).abs() < 1e-2,
            "({rx},{ry})"
        );
    }

    #[test]
    fn projective_inverse_round_trips() {
        let transform = Transform::set(1.2, 0.1, 3.0, -0.2, 0.9, -2.0, 0.01, -0.02, 1.0);
        let inverse = transform
            .inverse()
            .expect("projective matrix is invertible");

        for (expected_x, expected_y) in [(9.0, -4.0), (0.5, 2.25), (-3.0, 7.0)] {
            let (x, y) = transform.transform_point(expected_x, expected_y);
            let (actual_x, actual_y) = inverse.transform_point(x, y);
            assert!(
                (actual_x - expected_x).abs() < 1e-3 && (actual_y - expected_y).abs() < 1e-3,
                "({expected_x},{expected_y}) round-tripped as ({actual_x},{actual_y})"
            );
        }
    }

    #[test]
    fn general_inverse_matches_cpp_zero_and_finite_guards() {
        assert!(Transform::set(1.0, 2.0, 3.0, 2.0, 4.0, 6.0, 0.0, 0.0, 1.0)
            .inverse()
            .is_none());
        assert!(
            Transform::set(f32::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0,)
                .inverse()
                .is_none()
        );

        // SetAsInv tests `!det`, not an epsilon. Preserve invertible, very
        // small finite determinants rather than treating them as singular.
        let tiny = Transform::set(1.0e-10, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
        let inverse = tiny.inverse().expect("nonzero determinant is invertible");
        assert!((inverse.mat[0] - 1.0e10).abs() / 1.0e10 < 1e-6);
    }
}
