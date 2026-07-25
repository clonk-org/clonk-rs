//! `command` — moved verbatim from the parent module.
//!
//! Structural only: same crate, same items, same bodies.

use super::*;

// C4Command.cpp:31-36 movement-control constants.
pub(in crate::command) const LET_GO_RANGE1: i32 = 7;
pub(in crate::command) const LET_GO_RANGE2: i32 = 30;
pub(in crate::command) const LET_GO_HANGLE_ANGLE: i32 = 110;
pub(in crate::command) const JUMP_ANGLE: i32 = 35;
pub(in crate::command) const JUMP_LOW_ANGLE: i32 = 80;
pub(in crate::command) const JUMP_ANGLE_RANGE: i32 = 10;
pub(in crate::command) const JUMP_HIGH_ANGLE: i32 = 0;
pub(in crate::command) const FLIGHT_ANGLE_RANGE: i32 = 60;
pub(in crate::command) const PATH_RANGE: i32 = 20;
pub(in crate::command) const MAX_PATH_RANGE: i32 = 1_000;

pub(crate) fn inside(value: i32, lo: i32, hi: i32) -> bool {
    value >= lo && value <= hi
}

pub(in crate::command) fn bound_by(value: i32, lo: i32, hi: i32) -> i32 {
    // C++ BoundBy preserves the supplied bound order, including the
    // inverted bounds produced by negative transfer-zone extents.
    if value < lo {
        lo
    } else if value > hi {
        hi
    } else {
        value
    }
}

/// `Angle` (C4Math.cpp:33-45): 0 = up, 90 = right, clockwise; float
/// atan2 truncated like the C++ `static_cast<int>`.
pub(in crate::command) fn c4_angle(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dy = (y1 - y2).abs() as f32;
    let dx = (x1 - x2).abs() as f32;
    let angle =
        (180.0_f64 * f64::from(dy.atan2(dx)) * f64::from(std::f32::consts::FRAC_1_PI)) as i32;
    if x2 > x1 {
        if y2 < y1 {
            90 - angle
        } else {
            90 + angle
        }
    } else if y2 < y1 {
        270 + angle
    } else {
        270 - angle
    }
}

/// `Distance` (C4Math.cpp:22-31): integer sqrt with the double-step
/// correction.
pub(crate) fn c4_distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = i64::from(x1) - i64::from(x2);
    let dy = i64::from(y1) - i64::from(y2);
    let d2 = dx * dx + dy * dy;
    let mut dist = (d2 as f64).sqrt() as i64;
    if dist * dist < d2 {
        dist += 1;
    }
    if dist * dist > d2 {
        dist -= 1;
    }
    dist as i32
}

/// The global `PathFree` probe used before C4PathFinder
/// (C4Command.cpp:235; C4Landscape.cpp:1683-1738,2052-2055).
pub(in crate::command) fn command_path_free(
    landscape: &crate::Landscape,
    mut x1: i32,
    mut y1: i32,
    mut x2: i32,
    mut y2: i32,
) -> bool {
    if (x2 - x1).abs() < (y2 - y1).abs() {
        if y1 > y2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y1, &mut y2);
        }
        let xincr = if x2 > x1 { 1 } else { -1 };
        let dy = y2 - y1;
        let dx = (x2 - x1).abs();
        let mut d = 2 * dx - dy;
        let aincr = 2 * (dx - dy);
        let bincr = 2 * dx;
        let mut x = x1;
        if landscape.is_solid_at(x, y1) {
            return false;
        }
        for y in (y1 + 1)..=y2 {
            if d >= 0 {
                x += xincr;
                d += aincr;
            } else {
                d += bincr;
            }
            if landscape.is_solid_at(x, y) {
                return false;
            }
        }
    } else {
        if x1 > x2 {
            std::mem::swap(&mut x1, &mut x2);
            std::mem::swap(&mut y1, &mut y2);
        }
        let yincr = if y2 > y1 { 1 } else { -1 };
        let dx = x2 - x1;
        let dy = (y2 - y1).abs();
        let mut d = 2 * dy - dx;
        let aincr = 2 * (dy - dx);
        let bincr = 2 * dy;
        let mut y = y1;
        if landscape.is_solid_at(x1, y) {
            return false;
        }
        for x in (x1 + 1)..=x2 {
            if d >= 0 {
                y += yincr;
                d += aincr;
            } else {
                d += bincr;
            }
            if landscape.is_solid_at(x, y) {
                return false;
            }
        }
    }
    true
}

/// `AdjustSolidOffset` (C4Command.cpp:126-143): move a pathfinder waypoint
/// away from nearby solid pixels by half the moving object's shape.
pub(in crate::command) fn adjust_solid_offset(
    landscape: &crate::Landscape,
    x: &mut i32,
    y: &mut i32,
    x_offset: i32,
    y_offset: i32,
) -> bool {
    if landscape.is_solid_at(*x, *y) {
        return false;
    }
    for offset in 1..y_offset {
        if landscape.is_solid_at(*x, *y + offset) && !landscape.is_solid_at(*x, *y - offset) {
            *y -= 1;
        }
        if landscape.is_solid_at(*x, *y - offset) && !landscape.is_solid_at(*x, *y + offset) {
            *y += 1;
        }
    }
    for offset in 1..x_offset {
        if landscape.is_solid_at(*x + offset, *y) && !landscape.is_solid_at(*x - offset, *y) {
            *x -= 1;
        }
        if landscape.is_solid_at(*x - offset, *y) && !landscape.is_solid_at(*x + offset, *y) {
            *x += 1;
        }
    }
    true
}

/// `ObjectComLetGo` (C4ObjectCom.cpp:310-314) as an object update:
/// ObjectActionJump(itofix(xdirf), Fix0) — the hardcoded Jump action plus
/// the launch velocity (the fixed-velocity delta apply arms mobility,
/// matching Mobile=1 in ObjectActionJump). Any pending ComDir steer from
/// the same Execute rides along.
pub(in crate::command) fn let_go_update(
    steer: Option<CommandDirection>,
    xdirf: i32,
) -> ObjectUpdate {
    let mut update = ObjectUpdate::new();
    if let Some(direction) = steer {
        update = update.with_command_direction(direction);
    }
    let mut update = update.with_action_update(
        ActionUpdate::default()
            .with_name("Jump")
            .with_phase(0)
            .with_ticks(0)
            .with_force(true),
    );
    update.fixed_velocity = Some(FixedVec2::new(
        math::itofix(xdirf),
        crate::C4Fixed::from_raw(0),
    ));
    update
}

/// `SolidOnWhichSide` (C4Command.cpp:147-156).
pub(in crate::command) fn solid_on_which_side(landscape: &crate::Landscape, x: i32, y: i32) -> i32 {
    for cx in 1..10 {
        for cy in 0..10 {
            if landscape.is_solid_at(x - cx, y - cy) || landscape.is_solid_at(x - cx, y + cy) {
                return -1;
            }
            if landscape.is_solid_at(x + cx, y - cy) || landscape.is_solid_at(x + cx, y + cy) {
                return 1;
            }
        }
    }
    0
}

/// `AdjustMoveToTarget` (C4Command.cpp:94-114): raise above solid, then
/// (walking) drop to the bottom of free space and lift half a shape
/// above near-ground solid.
pub(crate) fn adjust_move_to_target(
    landscape: &crate::Landscape,
    x: &mut i32,
    y: &mut i32,
    free_move: bool,
    shape_height: i32,
) {
    let mut iy = *y;
    while iy >= 0 && landscape.is_solid_at(*x, iy) {
        iy -= 1;
    }
    if iy >= 0 {
        *y = iy;
    }
    if !free_move {
        if !landscape.is_semi_solid_at(*x, *y) {
            let back_hgt = landscape.estimated_height();
            let mut iy = *y;
            while iy < back_hgt && !landscape.is_semi_solid_at(*x, iy + 1) {
                iy += 1;
            }
            if iy < back_hgt {
                *y = iy;
            }
        }
        if (landscape.is_solid_at(*x, *y + 1) || landscape.is_solid_at(*x, *y + 5))
            && !landscape.is_semi_solid_at(*x, *y - shape_height / 2)
        {
            *y -= shape_height / 2;
        }
    }
}
