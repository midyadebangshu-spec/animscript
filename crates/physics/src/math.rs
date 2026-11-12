//! Small 2D maths. The simulation hot path uses only exactly rounded operations (`+ - * /` and
//! `sqrt`), so results never depend on the platform's libm; `sin`/`cos`/`atan2` appear only when
//! converting angles at the edges.
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

pub const fn v2(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

impl Vec2 {
    pub const ZERO: Vec2 = v2(0.0, 0.0);
    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }
    pub fn length_sq(self) -> f64 {
        self.dot(self)
    }
    pub fn length(self) -> f64 {
        self.length_sq().sqrt()
    }
    pub fn normalized(self) -> Vec2 {
        let l = self.length();
        if l > 1e-300 {
            self / l
        } else {
            Vec2::ZERO
        }
    }
    /// Rotated a quarter turn the way `Rot::from_angle(PI/2)` rotates.
    pub fn perp(self) -> Vec2 {
        v2(-self.y, self.x)
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        v2(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        v2(self.x - o.x, self.y - o.y)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        v2(-self.x, -self.y)
    }
}
impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, s: f64) -> Vec2 {
        v2(self.x * s, self.y * s)
    }
}
impl Mul<Vec2> for f64 {
    type Output = Vec2;
    fn mul(self, v: Vec2) -> Vec2 {
        v2(self * v.x, self * v.y)
    }
}
impl Div<f64> for Vec2 {
    type Output = Vec2;
    fn div(self, s: f64) -> Vec2 {
        v2(self.x / s, self.y / s)
    }
}
impl AddAssign for Vec2 {
    fn add_assign(&mut self, o: Vec2) {
        *self = *self + o;
    }
}
impl SubAssign for Vec2 {
    fn sub_assign(&mut self, o: Vec2) {
        *self = *self - o;
    }
}

/// `s x v` for a scalar angular velocity.
pub fn cross_sv(s: f64, v: Vec2) -> Vec2 {
    v2(-s * v.y, s * v.x)
}

/// A rotation stored as sine and cosine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rot {
    pub s: f64,
    pub c: f64,
}

impl Rot {
    pub const IDENTITY: Rot = Rot { s: 0.0, c: 1.0 };
    pub fn from_angle(a: f64) -> Rot {
        Rot {
            s: a.sin(),
            c: a.cos(),
        }
    }
    pub fn angle(self) -> f64 {
        self.s.atan2(self.c)
    }
    pub fn rotate(self, v: Vec2) -> Vec2 {
        v2(self.c * v.x - self.s * v.y, self.s * v.x + self.c * v.y)
    }
    pub fn inv_rotate(self, v: Vec2) -> Vec2 {
        v2(self.c * v.x + self.s * v.y, -self.s * v.x + self.c * v.y)
    }
    /// `conj(self) * o`: `o` expressed relative to `self`.
    pub fn inv_mul(self, o: Rot) -> Rot {
        Rot {
            c: self.c * o.c + self.s * o.s,
            s: self.c * o.s - self.s * o.c,
        }
    }
    /// Rotate by a small angle `da` (first order, renormalised).
    pub fn integrate(self, da: f64) -> Rot {
        let c = self.c - da * self.s;
        let s = self.s + da * self.c;
        let inv = 1.0 / (s * s + c * c).sqrt();
        Rot {
            s: s * inv,
            c: c * inv,
        }
    }
}

/// A rigid transform: rotate, then translate.
#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub p: Vec2,
    pub q: Rot,
}

impl Xf {
    pub fn apply(&self, v: Vec2) -> Vec2 {
        self.p + self.q.rotate(v)
    }
}
