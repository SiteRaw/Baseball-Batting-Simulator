// Utility: lightweight Vec3, RNG helpers, math.

pub const MPH_TO_FPS: f32 = 1.466_667;
pub const RPM_TO_RADS: f32 = 0.104_720;

// ---------------------------------------------------------------------------
// Minimal 3-D vector (no external dependency required in physics / outcomes)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    #[inline] pub fn new(x: f32, y: f32, z: f32) -> Self { Self { x, y, z } }

    #[inline]
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    #[inline]
    pub fn length_sq(self) -> f32 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    #[inline]
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len < 1e-10 { Self::ZERO } else { self * (1.0 / len) }
    }

    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Right-hand rule cross product: self × rhs
    #[inline]
    pub fn cross(self, r: Self) -> Self {
        Self::new(
            self.y * r.z - self.z * r.y,
            self.z * r.x - self.x * r.z,
            self.x * r.y - self.y * r.x,
        )
    }

    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

use std::ops::*;

impl Add for Vec3  { type Output = Self; fn add(self, r: Self) -> Self { Self::new(self.x+r.x, self.y+r.y, self.z+r.z) } }
impl Sub for Vec3  { type Output = Self; fn sub(self, r: Self) -> Self { Self::new(self.x-r.x, self.y-r.y, self.z-r.z) } }
impl Mul<f32> for Vec3  { type Output = Self; fn mul(self, s: f32) -> Self { Self::new(self.x*s, self.y*s, self.z*s) } }
impl Div<f32> for Vec3  { type Output = Self; fn div(self, s: f32) -> Self { Self::new(self.x/s, self.y/s, self.z/s) } }
impl Neg for Vec3  { type Output = Self; fn neg(self) -> Self { Self::new(-self.x, -self.y, -self.z) } }
impl AddAssign for Vec3 { fn add_assign(&mut self, r: Self) { *self = *self + r; } }
impl SubAssign for Vec3 { fn sub_assign(&mut self, r: Self) { *self = *self - r; } }
impl MulAssign<f32> for Vec3 { fn mul_assign(&mut self, s: f32) { *self = *self * s; } }

// ---------------------------------------------------------------------------
// RNG helpers (macroquad's gen_range / Box-Muller gaussian)
// ---------------------------------------------------------------------------

/// Standard-normal sample scaled by sigma (Box-Muller).
pub fn gauss(sigma: f32) -> f32 {
    use std::f32::consts::PI;
    // gen_range returns [lo, hi); avoid log(0) by flooring at tiny positive
    let u1 = macroquad::rand::gen_range(1e-9_f32, 1.0_f32);
    let u2 = macroquad::rand::gen_range(0.0_f32, 1.0_f32);
    sigma * (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
}

pub fn frand(lo: f32, hi: f32) -> f32 {
    macroquad::rand::gen_range(lo, hi)
}

pub fn chance(p: f32) -> bool {
    macroquad::rand::gen_range(0.0_f32, 1.0_f32) < p
}

// ---------------------------------------------------------------------------
// Scalar math helpers
// ---------------------------------------------------------------------------

#[inline] pub fn clampf(v: f32, lo: f32, hi: f32) -> f32 { v.max(lo).min(hi) }
#[inline] pub fn lerpf(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
