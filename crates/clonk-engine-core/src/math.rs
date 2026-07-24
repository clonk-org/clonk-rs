use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

// Pre-computed sine values for degrees 0.00–90.00 (steps of 0.01°).
// Copied verbatim from `src/Fixed.cpp` for bit-exact parity. Fixed.h:39.
include!("sine_table.rs");

// ── C4Fixed ───────────────────────────────────────────────────────────────────

/// 16.16 fixed-point number matching C++ `C4Fixed`. `Fixed.h:46-219`.
///
/// Internal representation: `val` stores the number scaled by 2^16 (65536).
/// `itofix(1).val == 65536`, `itofix(1) + itofix(1) == itofix(2)`.
///
/// This type is determinism-critical: only use arithmetic on this type for
/// physics state; never silently convert to/from floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct C4Fixed(i32);

const FPF: i32 = 1 << 16; // = 65536

impl C4Fixed {
    pub const ZERO: Self = Self(0);

    /// Raw internal value (for serialization / direct bit comparison).
    #[inline]
    pub const fn val(self) -> i32 {
        self.0
    }

    /// Construct from a raw `val` field (use sparingly – prefer `itofix`).
    #[inline]
    pub const fn from_raw(v: i32) -> Self {
        Self(v)
    }

    /// Round to nearest integer. Fixed.h:84-95 (`to_int()`).
    #[inline]
    pub fn to_int(self) -> i32 {
        let v = self.0;
        let mut r = v;
        // Add 0.5 unless that would overflow
        if v <= i32::MAX - FPF / 2 {
            r += FPF / 2;
        }
        // Ensure -x.5 rounds toward -∞ rather than toward zero
        if r < 0 {
            r -= 1;
        }
        r >>= 16; // arithmetic right-shift
                  // Edge-case: 32767.5 must round to 32768
        if v > i32::MAX - FPF / 2 {
            r += 1;
        }
        r
    }

    /// Absolute value (the C++ `Abs` template over `C4Fixed`).
    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.wrapping_abs())
    }

    /// Round to nearest with precision multiplier. Fixed.h:97-105.
    #[inline]
    pub fn to_int_prec(self, prec: i32) -> i32 {
        let mut r = i64::from(self.0) * i64::from(prec);
        r += i64::from(FPF) / 2;
        if r < 0 {
            r -= 1;
        }
        (r >> 16) as i32
    }

    /// Convert to `f32`. Fixed.h:108-110.
    #[inline]
    pub fn to_float(self) -> f32 {
        self.0 as f32 / FPF as f32
    }

    /// True if the internal value is non-zero.
    #[inline]
    pub fn is_nonzero(self) -> bool {
        self.0 != 0
    }

    /// Sine of angle in degrees (input is `self` in degrees). Fixed.h:188-202.
    pub fn sin_deg(self) -> Self {
        let v = ((i64::from(self.0) * 100) / i64::from(FPF)) as i32;
        let v = if v < 0 { 18000 - v } else { v };
        let v = v.rem_euclid(36000) as usize;
        let raw = match v / 9000 {
            0 => SINE_TABLE[v],
            1 => SINE_TABLE[18000 - v],
            2 => -SINE_TABLE[v - 18000],
            _ => -SINE_TABLE[36000 - v],
        };
        Self(raw)
    }

    /// Cosine of angle in degrees (input is `self` in degrees). Fixed.h:204-218.
    pub fn cos_deg(self) -> Self {
        let v = ((i64::from(self.0) * 100) / i64::from(FPF)).unsigned_abs() as usize;
        let v = v % 36000;
        let raw = match v / 9000 {
            0 => SINE_TABLE[9000 - v],
            1 => -SINE_TABLE[v - 9000],
            2 => -SINE_TABLE[27000 - v],
            _ => SINE_TABLE[v - 27000],
        };
        Self(raw)
    }
}

// ── Arithmetic operators ──────────────────────────────────────────────────────
// Mirrors C++ Fixed.h:122-183.

impl AddAssign for C4Fixed {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0.wrapping_add(rhs.0);
    }
}

impl SubAssign for C4Fixed {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0.wrapping_sub(rhs.0);
    }
}

impl MulAssign for C4Fixed {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        self.0 = (i64::from(self.0) * i64::from(rhs.0) / i64::from(FPF)) as i32;
    }
}

impl DivAssign for C4Fixed {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        self.0 = (i64::from(self.0) * i64::from(FPF) / i64::from(rhs.0)) as i32;
    }
}

impl MulAssign<i32> for C4Fixed {
    #[inline]
    fn mul_assign(&mut self, rhs: i32) {
        self.0 = self.0.wrapping_mul(rhs);
    }
}

impl DivAssign<i32> for C4Fixed {
    #[inline]
    fn div_assign(&mut self, rhs: i32) {
        self.0 /= rhs;
    }
}

impl Neg for C4Fixed {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(self.0.wrapping_neg())
    }
}

impl Add for C4Fixed {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        let mut v = self;
        v += rhs;
        v
    }
}

impl Sub for C4Fixed {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        let mut v = self;
        v -= rhs;
        v
    }
}

impl Mul for C4Fixed {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let mut v = self;
        v *= rhs;
        v
    }
}

impl Div for C4Fixed {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        let mut v = self;
        v /= rhs;
        v
    }
}

impl Add<i32> for C4Fixed {
    type Output = Self;
    #[inline]
    fn add(self, rhs: i32) -> Self {
        self + itofix(rhs)
    }
}

impl Sub<i32> for C4Fixed {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: i32) -> Self {
        self - itofix(rhs)
    }
}

impl Mul<i32> for C4Fixed {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: i32) -> Self {
        let mut v = self;
        v *= rhs;
        v
    }
}

impl Div<i32> for C4Fixed {
    type Output = Self;
    #[inline]
    fn div(self, rhs: i32) -> Self {
        let mut v = self;
        v /= rhs;
        v
    }
}

impl AddAssign<i32> for C4Fixed {
    #[inline]
    fn add_assign(&mut self, rhs: i32) {
        *self += itofix(rhs);
    }
}

impl SubAssign<i32> for C4Fixed {
    #[inline]
    fn sub_assign(&mut self, rhs: i32) {
        *self -= itofix(rhs);
    }
}

impl PartialEq<i32> for C4Fixed {
    fn eq(&self, &rhs: &i32) -> bool {
        *self == itofix(rhs)
    }
}

impl PartialOrd<i32> for C4Fixed {
    fn partial_cmp(&self, rhs: &i32) -> Option<std::cmp::Ordering> {
        self.partial_cmp(&itofix(*rhs))
    }
}

// ── Conversion functions ──────────────────────────────────────────────────────

/// Integer → fixed-point. `itofix(n).val == n * 65536`. Fixed.h:226.
/// Wrapping like the C++ int32 multiply: real content feeds values beyond
/// ±32767 (e.g. `Size=100000` in Objects.txt) where C++ silently wraps.
#[inline]
pub fn itofix(x: i32) -> C4Fixed {
    C4Fixed(x.wrapping_mul(FPF))
}

/// Integer → fixed-point with precision denominator. Fixed.h:227.
/// `itofix_prec(x, p)` = `x/p` as a `C4Fixed` — wrapping i32 products like
/// the C++ expression.
#[inline]
pub fn itofix_prec(x: i32, prec: i32) -> C4Fixed {
    let val = if prec < FPF {
        x.wrapping_mul(FPF / prec)
            .wrapping_add(x.wrapping_mul(FPF % prec) / prec)
    } else {
        (i64::from(x) * i64::from(FPF) / i64::from(prec)) as i32
    };
    C4Fixed(val)
}

/// The constant procedure accelerations (C4Movement.cpp:31-34), `FIXED100`
/// values with the C++ constructor's truncating division.
pub const WALK_ACCEL: C4Fixed = C4Fixed(50 * FPF / 100);
pub const SWIM_ACCEL: C4Fixed = C4Fixed(20 * FPF / 100);
pub const FLOAT_ACCEL: C4Fixed = C4Fixed(10 * FPF / 100);
/// `FloatFriction = FIXED100(2)` (C4Movement.cpp:31): x/r decay while
/// floating in liquid.
pub const FLOAT_FRICTION: C4Fixed = C4Fixed(2 * FPF / 100);
pub const ROTATE_ACCEL: C4Fixed = C4Fixed(20 * FPF / 100);

/// `StableRange` (C4Physics.h:23): degrees within which an object counts
/// as upright.
pub const STABLE_RANGE: i32 = 10;

/// `ValByPhysical` (C4InfoCore.h:224-227): the percentage of a physical's
/// maximum as a fixed value — `itofix(physical * (percent/5),
/// C4MaxPhysical * 20)`. The `percent / 5` is integer division.
#[inline]
pub fn val_by_physical(percent: i32, physical: i32) -> C4Fixed {
    itofix_prec(physical.wrapping_mul(percent / 5), 100_000 * 20)
}

/// `Towards` (C4Object.cpp:4561-4566): step `val` toward `target` by `step`,
/// snapping when within range.
pub fn towards(val: &mut C4Fixed, target: C4Fixed, step: C4Fixed) {
    if *val == target {
        return;
    }
    let diff = if *val < target {
        target - *val
    } else {
        *val - target
    };
    if diff <= step {
        *val = target;
        return;
    }
    if *val < target {
        *val += step;
    } else {
        *val -= step;
    }
}

/// Fixed-point → integer (round to nearest). Fixed.h:224.
#[inline]
pub fn fixtoi(x: C4Fixed) -> i32 {
    x.to_int()
}

/// Fixed-point → integer with precision multiplier. Fixed.h:225.
#[inline]
pub fn fixtoi_prec(x: C4Fixed, prec: i32) -> i32 {
    x.to_int_prec(prec)
}

/// Float → fixed-point (truncates). Fixed.h:223.
#[inline]
pub fn ftofix(x: f32) -> C4Fixed {
    C4Fixed((x * FPF as f32) as i32)
}

/// Fixed-point → float. Fixed.h:222.
#[inline]
pub fn fixtof(x: C4Fixed) -> f32 {
    x.to_float()
}

/// `x/100` as fixed-point. Fixed.h:232.
#[inline]
pub fn fixed100(x: i32) -> C4Fixed {
    itofix_prec(x, 100)
}

/// `x/256` as fixed-point. Fixed.h:233.
#[inline]
pub fn fixed256(x: i32) -> C4Fixed {
    C4Fixed(x.wrapping_mul(FPF) / 256)
}

/// `x/10` as fixed-point. Fixed.h:234.
#[inline]
pub fn fixed10(x: i32) -> C4Fixed {
    itofix_prec(x, 10)
}

// ── FixedVec2 ─────────────────────────────────────────────────────────────────

/// Two-component vector of `C4Fixed` values (sub-pixel position / velocity).
/// Mirrors `fix_x`/`fix_y` and `xdir`/`ydir` in C++ `C4Object`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedVec2 {
    pub x: C4Fixed,
    pub y: C4Fixed,
}

impl FixedVec2 {
    pub const ZERO: Self = Self {
        x: C4Fixed::ZERO,
        y: C4Fixed::ZERO,
    };

    /// Construct from C4Fixed components.
    #[inline]
    pub fn new(x: C4Fixed, y: C4Fixed) -> Self {
        Self { x, y }
    }

    /// Construct from integer pixel coordinates (uses `itofix`).
    #[inline]
    pub fn from_ints(x: i32, y: i32) -> Self {
        Self {
            x: itofix(x),
            y: itofix(y),
        }
    }

    /// Integer pixel x (fixtoi of x component).
    #[inline]
    pub fn int_x(self) -> i32 {
        fixtoi(self.x)
    }

    /// Integer pixel y (fixtoi of y component).
    #[inline]
    pub fn int_y(self) -> i32 {
        fixtoi(self.y)
    }
}

impl AddAssign for FixedVec2 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl Add for FixedVec2 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        let mut v = self;
        v += rhs;
        v
    }
}

// ── Legacy integer distance (unchanged) ───────────────────────────────────────

pub fn integer_distance(x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = i64::from(x1) - i64::from(x2);
    let dy = i64::from(y1) - i64::from(y2);
    let d2 = dx * dx + dy * dy;
    if d2 < 0 {
        return -1;
    }
    let mut dist = (d2 as f64).sqrt() as i32;
    if i64::from(dist) * i64::from(dist) < d2 {
        dist += 1;
    }
    if i64::from(dist) * i64::from(dist) > d2 {
        dist -= 1;
    }
    dist
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── itofix / fixtoi ───────────────────────────────────────────────────────

    #[test]
    fn itofix_wraps_beyond_int16_range_like_cpp() {
        // C++ `itofix` is a plain int32 multiply (Fixed.h:226) that wraps
        // for |x| > 32767 in practice; real Objects.txt carries such values
        // (Size=100000). The Rust port must not panic and must produce the
        // same wrapped raw value.
        assert_eq!(itofix(100_000).val(), 100_000i32.wrapping_mul(FPF));
        assert_eq!(itofix(-100_000).val(), (-100_000i32).wrapping_mul(FPF));
    }

    #[test]
    fn itofix_fixtoi_roundtrip() {
        // fixtoi(itofix(n)) == n for all integers. Fixed.h:224,226.
        for n in -1000..=1000 {
            assert_eq!(fixtoi(itofix(n)), n, "n={n}");
        }
    }

    #[test]
    fn itofix_internal_value() {
        // itofix(1).val == 65536. Fixed.h:226.
        assert_eq!(itofix(1).val(), 65536);
        assert_eq!(itofix(-1).val(), -65536);
        assert_eq!(itofix(0).val(), 0);
    }

    #[test]
    fn fixtoi_rounds_half_up_positive() {
        // 0.5 → 1. Fixed.h:88-89.
        assert_eq!(fixtoi(C4Fixed::from_raw(FPF / 2)), 1);
    }

    #[test]
    fn fixtoi_rounds_just_below_half_down() {
        assert_eq!(fixtoi(C4Fixed::from_raw(FPF / 2 - 1)), 0);
    }

    #[test]
    fn fixtoi_negative_half_rounds_to_zero() {
        // -0.5 rounds to 0 (not -1). Fixed.h:90-92.
        assert_eq!(fixtoi(C4Fixed::from_raw(-(FPF / 2))), 0);
    }

    #[test]
    fn fixtoi_negative_one_and_half_rounds_down() {
        // -1.5 rounds to -2. Fixed.h:90-92.
        assert_eq!(fixtoi(C4Fixed::from_raw(-3 * FPF / 2)), -2);
    }

    // ── Arithmetic ────────────────────────────────────────────────────────────

    #[test]
    fn addition_of_integer_equivalents() {
        assert_eq!(itofix(3) + itofix(4), itofix(7));
    }

    #[test]
    fn subtraction_of_integer_equivalents() {
        assert_eq!(itofix(7) - itofix(3), itofix(4));
    }

    #[test]
    fn multiplication_exact() {
        // itofix(2) * itofix(3) == itofix(6). Fixed.h:134-137.
        assert_eq!(itofix(2) * itofix(3), itofix(6));
    }

    #[test]
    fn division_exact() {
        // itofix(6) / itofix(2) == itofix(3). Fixed.h:146-149.
        assert_eq!(itofix(6) / itofix(2), itofix(3));
    }

    #[test]
    fn multiplication_by_int() {
        assert_eq!(itofix(5) * 3, itofix(15));
    }

    #[test]
    fn division_by_int() {
        assert_eq!(itofix(12) / 4, itofix(3));
    }

    #[test]
    fn negation() {
        assert_eq!(-itofix(5), itofix(-5));
    }

    #[test]
    fn add_int_rhs() {
        assert_eq!(itofix(3) + 4, itofix(7));
    }

    #[test]
    fn sub_int_rhs() {
        assert_eq!(itofix(7) - 3, itofix(4));
    }

    #[test]
    fn raw_arithmetic_wraps_like_cpp_int32() {
        // Fixed.h:121-183 and FIXED256 at :232 use the engine's practical
        // two's-complement int32 semantics in every build profile.
        let max = C4Fixed::from_raw(i32::MAX);
        let one = C4Fixed::from_raw(1);
        assert_eq!((max + one).val(), i32::MIN);
        assert_eq!((C4Fixed::from_raw(i32::MIN) - one).val(), i32::MAX);
        assert_eq!((-C4Fixed::from_raw(i32::MIN)).val(), i32::MIN);
        assert_eq!(C4Fixed::from_raw(i32::MIN).abs().val(), i32::MIN);
        assert_eq!((max * 2).val(), -2);
        assert_eq!(fixed256(i32::MAX).val(), i32::MAX.wrapping_mul(FPF) / 256);
        assert_eq!(
            (itofix(100_000) + itofix(100_000)).val(),
            100_000i32
                .wrapping_mul(FPF)
                .wrapping_add(100_000i32.wrapping_mul(FPF))
        );
    }

    // ── Sub-pixel accumulation (the key item-1 regression test) ──────────────

    #[test]
    fn sub_pixel_velocity_accumulates() {
        // In C++: fix_x=0, xdir.val=300 (≈0.00458 px/frame).
        // After 220 frames: fix_x.val = 66000 → fixtoi = 1 pixel.
        // Plain i32 addition (old Rust): 0+0+0+… = 0 forever.
        let mut pos = C4Fixed::ZERO;
        let vel = C4Fixed::from_raw(300); // sub-pixel velocity
        for _ in 0..220 {
            pos += vel;
        }
        // 220 * 300 = 66000 > 65536, so fixtoi = 1 pixel
        assert_eq!(
            fixtoi(pos),
            1,
            "sub-pixel velocity must accumulate: got {}",
            fixtoi(pos)
        );
    }

    #[test]
    fn integer_velocity_accumulates_correctly() {
        // itofix(5) per frame for 10 frames = itofix(50) total position
        let mut pos = C4Fixed::ZERO;
        let vel = itofix(5);
        for _ in 0..10 {
            pos += vel;
        }
        assert_eq!(fixtoi(pos), 50);
    }

    // ── fixed100 / fixed256 / fixed10 ─────────────────────────────────────────

    #[test]
    fn fixed100_is_one_hundredth() {
        // fixed100(100) == itofix(1). Fixed.h:232.
        assert_eq!(fixed100(100), itofix(1));
    }

    #[test]
    fn fixed256_gravity_constant() {
        // FIXED256(10) ≈ 10/256 ≈ 0.039 — typical gravity value. Fixed.h:233.
        assert_eq!(fixed256(10).val(), 10 * FPF / 256);
    }

    #[test]
    fn fixed10_is_one_tenth() {
        // fixed10(10) == itofix(1). Fixed.h:234.
        assert_eq!(fixed10(10), itofix(1));
    }

    // ── itofix_prec ───────────────────────────────────────────────────────────

    #[test]
    fn itofix_prec_100() {
        // itofix_prec(50, 100) == fixed100(50) == itofix(0.5)
        assert_eq!(itofix_prec(50, 100), fixed100(50));
    }

    // ── Sin / Cos ─────────────────────────────────────────────────────────────

    #[test]
    fn sin_zero_is_zero() {
        assert_eq!(itofix(0).sin_deg(), C4Fixed::ZERO);
    }

    #[test]
    fn sin_ninety_is_one() {
        // sin(90°) = 1 exactly (SineTable[9000] = 65536). Fixed.h:196.
        assert_eq!(itofix(90).sin_deg(), itofix(1));
    }

    #[test]
    fn cos_zero_is_one() {
        assert_eq!(itofix(0).cos_deg(), itofix(1));
    }

    #[test]
    fn cos_ninety_is_zero() {
        // cos(90°) = 0 (SineTable[0] = 0). Fixed.h:213.
        assert_eq!(itofix(90).cos_deg(), C4Fixed::ZERO);
    }

    #[test]
    fn sin_180_is_zero() {
        assert_eq!(itofix(180).sin_deg(), C4Fixed::ZERO);
    }

    #[test]
    fn sin_270_is_minus_one() {
        assert_eq!(itofix(270).sin_deg(), itofix(-1));
    }

    // ── FixedVec2 ─────────────────────────────────────────────────────────────

    #[test]
    fn fixed_vec2_from_ints_roundtrips() {
        let v = FixedVec2::from_ints(3, -7);
        assert_eq!(v.int_x(), 3);
        assert_eq!(v.int_y(), -7);
    }

    #[test]
    fn fixed_vec2_add_assign() {
        let mut a = FixedVec2::from_ints(1, 2);
        let b = FixedVec2::from_ints(3, 4);
        a += b;
        assert_eq!(a.int_x(), 4);
        assert_eq!(a.int_y(), 6);
    }

    #[test]
    fn fixed_vec2_sub_pixel_add() {
        // 300 sub-pixel/frame for 220 frames on x → 1 pixel
        let mut pos = FixedVec2::ZERO;
        let vel = FixedVec2::new(C4Fixed::from_raw(300), C4Fixed::ZERO);
        for _ in 0..220 {
            pos += vel;
        }
        assert_eq!(pos.int_x(), 1);
        assert_eq!(pos.int_y(), 0);
    }
}
