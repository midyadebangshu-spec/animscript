//! Small 3D maths: vectors, quaternions and 3x3 matrices. `f64` throughout; no hidden inputs.
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub const ZERO: V3 = V3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len2(self) -> f64 {
        self.dot(self)
    }
    pub fn len(self) -> f64 {
        self.len2().sqrt()
    }
    /// Unit vector, or zero when the length is negligible.
    pub fn normalized(self) -> V3 {
        let l = self.len();
        if l < 1e-300 {
            V3::ZERO
        } else {
            self * (1.0 / l)
        }
    }
    pub fn get(self, i: usize) -> f64 {
        [self.x, self.y, self.z][i]
    }
    pub fn arr(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
    pub fn from_arr(a: [f64; 3]) -> V3 {
        v3(a[0], a[1], a[2])
    }
    /// Any unit vector perpendicular to this one (deterministic).
    pub fn any_perp(self) -> V3 {
        let a = if self.x.abs() < 0.6 {
            v3(1.0, 0.0, 0.0)
        } else {
            v3(0.0, 1.0, 0.0)
        };
        self.cross(a).normalized()
    }
    pub fn min(self, o: V3) -> V3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max(self, o: V3) -> V3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}
impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f64> for V3 {
    type Output = V3;
    fn div(self, s: f64) -> V3 {
        v3(self.x / s, self.y / s, self.z / s)
    }
}
impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}
impl SubAssign for V3 {
    fn sub_assign(&mut self, o: V3) {
        *self = *self - o;
    }
}
impl MulAssign<f64> for V3 {
    fn mul_assign(&mut self, s: f64) {
        *self = *self * s;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Quat {
    pub const IDENTITY: Quat = Quat {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub fn from_axis_angle(axis: V3, angle: f64) -> Quat {
        let a = axis.normalized();
        let (s, c) = (angle * 0.5).sin_cos();
        Quat {
            w: c,
            x: a.x * s,
            y: a.y * s,
            z: a.z * s,
        }
    }
    /// Rotation that spins about y first, then tilts about x, then rolls about z (angles in turns):
    /// the same order the mesh `Transform3` uses.
    pub fn from_euler_turns(rx: f64, ry: f64, rz: f64) -> Quat {
        let tau = std::f64::consts::TAU;
        let qy = Quat::from_axis_angle(v3(0.0, 1.0, 0.0), ry * tau);
        let qx = Quat::from_axis_angle(v3(1.0, 0.0, 0.0), rx * tau);
        let qz = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), rz * tau);
        qz.mul(qx).mul(qy)
    }
    pub fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }
    pub fn conj(self) -> Quat {
        Quat {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
    pub fn normalized(self) -> Quat {
        let l = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if l < 1e-300 {
            return Quat::IDENTITY;
        }
        Quat {
            w: self.w / l,
            x: self.x / l,
            y: self.y / l,
            z: self.z / l,
        }
    }
    pub fn rotate(self, v: V3) -> V3 {
        let u = v3(self.x, self.y, self.z);
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }
    /// Advance by angular velocity `w` (rad/s, world frame) over `dt`.
    pub fn integrate(self, w: V3, dt: f64) -> Quat {
        let dq = Quat {
            w: 0.0,
            x: w.x,
            y: w.y,
            z: w.z,
        }
        .mul(self);
        Quat {
            w: self.w + 0.5 * dt * dq.w,
            x: self.x + 0.5 * dt * dq.x,
            y: self.y + 0.5 * dt * dq.y,
            z: self.z + 0.5 * dt * dq.z,
        }
        .normalized()
    }
    pub fn to_mat3(self) -> Mat3 {
        let (w, x, y, z) = (self.w, self.x, self.y, self.z);
        Mat3 {
            m: [
                [
                    1.0 - 2.0 * (y * y + z * z),
                    2.0 * (x * y - z * w),
                    2.0 * (x * z + y * w),
                ],
                [
                    2.0 * (x * y + z * w),
                    1.0 - 2.0 * (x * x + z * z),
                    2.0 * (y * z - x * w),
                ],
                [
                    2.0 * (x * z - y * w),
                    2.0 * (y * z + x * w),
                    1.0 - 2.0 * (x * x + y * y),
                ],
            ],
        }
    }
    pub fn arr(self) -> [f64; 4] {
        [self.w, self.x, self.y, self.z]
    }
    pub fn from_arr(a: [f64; 4]) -> Quat {
        Quat {
            w: a[0],
            x: a[1],
            y: a[2],
            z: a[3],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub m: [[f64; 3]; 3],
}

impl Mat3 {
    pub const ZERO: Mat3 = Mat3 { m: [[0.0; 3]; 3] };
    pub const IDENTITY: Mat3 = Mat3 {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    pub fn diag(a: f64, b: f64, c: f64) -> Mat3 {
        Mat3 {
            m: [[a, 0.0, 0.0], [0.0, b, 0.0], [0.0, 0.0, c]],
        }
    }
    pub fn mul_v(&self, v: V3) -> V3 {
        v3(
            self.m[0][0] * v.x + self.m[0][1] * v.y + self.m[0][2] * v.z,
            self.m[1][0] * v.x + self.m[1][1] * v.y + self.m[1][2] * v.z,
            self.m[2][0] * v.x + self.m[2][1] * v.y + self.m[2][2] * v.z,
        )
    }
    pub fn transpose(&self) -> Mat3 {
        let mut r = Mat3::ZERO;
        for i in 0..3 {
            for j in 0..3 {
                r.m[i][j] = self.m[j][i];
            }
        }
        r
    }
    pub fn mul(&self, o: &Mat3) -> Mat3 {
        let mut r = Mat3::ZERO;
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    r.m[i][j] += self.m[i][k] * o.m[k][j];
                }
            }
        }
        r
    }
    pub fn add(&self, o: &Mat3) -> Mat3 {
        let mut r = *self;
        for i in 0..3 {
            for j in 0..3 {
                r.m[i][j] += o.m[i][j];
            }
        }
        r
    }
    pub fn sub(&self, o: &Mat3) -> Mat3 {
        let mut r = *self;
        for i in 0..3 {
            for j in 0..3 {
                r.m[i][j] -= o.m[i][j];
            }
        }
        r
    }
    pub fn scale(&self, s: f64) -> Mat3 {
        let mut r = *self;
        for row in &mut r.m {
            for e in row {
                *e *= s;
            }
        }
        r
    }
    pub fn outer(a: V3, b: V3) -> Mat3 {
        let (a, b) = (a.arr(), b.arr());
        let mut r = Mat3::ZERO;
        for i in 0..3 {
            for j in 0..3 {
                r.m[i][j] = a[i] * b[j];
            }
        }
        r
    }
    pub fn trace(&self) -> f64 {
        self.m[0][0] + self.m[1][1] + self.m[2][2]
    }
    /// The matrix of `v x .`
    pub fn skew(v: V3) -> Mat3 {
        Mat3 {
            m: [[0.0, -v.z, v.y], [v.z, 0.0, -v.x], [-v.y, v.x, 0.0]],
        }
    }
    pub fn det(&self) -> f64 {
        let m = &self.m;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
    /// The inverse, or zero for a singular matrix.
    pub fn inverse(&self) -> Mat3 {
        let d = self.det();
        if d.abs() < 1e-300 {
            return Mat3::ZERO;
        }
        let m = &self.m;
        let c = |a: usize, b: usize, c: usize, d2: usize| m[a][b] * m[c][d2] - m[a][d2] * m[c][b];
        Mat3 {
            m: [
                [c(1, 1, 2, 2) / d, -c(0, 1, 2, 2) / d, c(0, 1, 1, 2) / d],
                [-c(1, 0, 2, 2) / d, c(0, 0, 2, 2) / d, -c(0, 0, 1, 2) / d],
                [c(1, 0, 2, 1) / d, -c(0, 0, 2, 1) / d, c(0, 0, 1, 1) / d],
            ],
        }
    }
}
