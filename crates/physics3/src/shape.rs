//! Collision shapes. A body's shape lives in the body frame (origin at the centre of mass); the
//! world form (`WShape`) is rebuilt from the pose once per step.
use crate::hull::*;
use crate::math::*;

#[derive(Clone, Debug)]
pub enum Shape {
    Sphere(f64),
    /// A segment along the body's y axis, `half` either side of the centre, swollen by `r`.
    Capsule {
        half: f64,
        r: f64,
    },
    Hull(Hull),
    /// A static half space: solid where `normal . p < d`. Placed in world coordinates.
    Plane {
        normal: V3,
        d: f64,
    },
    /// A static triangle soup in world coordinates.
    Tris(Vec<Hull>),
}

#[derive(Clone, Debug)]
pub enum WShape {
    Sphere { c: V3, r: f64 },
    Capsule { a: V3, b: V3, r: f64 },
    Hull(Hull),
    Plane { normal: V3, d: f64 },
    Tris(Vec<Hull>),
}

pub struct MassData {
    pub mass: f64,
    pub inertia: Mat3,
}

impl Shape {
    pub fn world(&self, p: V3, q: Quat) -> WShape {
        match self {
            Shape::Sphere(r) => WShape::Sphere { c: p, r: *r },
            Shape::Capsule { half, r } => {
                let axis = q.rotate(v3(0.0, *half, 0.0));
                WShape::Capsule {
                    a: p - axis,
                    b: p + axis,
                    r: *r,
                }
            }
            Shape::Hull(h) => WShape::Hull(h.transformed(p, q)),
            Shape::Plane { normal, d } => WShape::Plane {
                normal: *normal,
                d: *d,
            },
            Shape::Tris(t) => WShape::Tris(t.clone()),
        }
    }

    /// Mass and inertia about the centre for `density` (kilograms per cubic metre).
    pub fn mass_data(&self, density: f64) -> MassData {
        match self {
            Shape::Sphere(r) => {
                let m = density * 4.0 / 3.0 * std::f64::consts::PI * r * r * r;
                let i = 0.4 * m * r * r;
                MassData {
                    mass: m,
                    inertia: Mat3::diag(i, i, i),
                }
            }
            Shape::Capsule { half, r } => {
                let (h, pi) = (2.0 * half, std::f64::consts::PI);
                let mc = density * pi * r * r * h;
                let ms = density * 4.0 / 3.0 * pi * r * r * r;
                let iy = 0.5 * mc * r * r + 0.4 * ms * r * r;
                let ix = mc * (r * r / 4.0 + h * h / 12.0)
                    + ms * (0.4 * r * r + 0.25 * h * h + 3.0 / 8.0 * h * r);
                MassData {
                    mass: mc + ms,
                    inertia: Mat3::diag(ix, iy, ix),
                }
            }
            Shape::Hull(h) => {
                let mp = h.mass_props();
                MassData {
                    mass: density * mp.volume,
                    inertia: mp.inertia.scale(density),
                }
            }
            _ => MassData {
                mass: 0.0,
                inertia: Mat3::ZERO,
            },
        }
    }
}

impl WShape {
    /// Bounding box, or `None` for an infinite plane.
    pub fn aabb(&self) -> Option<(V3, V3)> {
        match self {
            WShape::Sphere { c, r } => Some((*c - v3(*r, *r, *r), *c + v3(*r, *r, *r))),
            WShape::Capsule { a, b, r } => {
                let rr = v3(*r, *r, *r);
                Some((a.min(*b) - rr, a.max(*b) + rr))
            }
            WShape::Hull(h) => Some(h.aabb()),
            WShape::Plane { .. } => None,
            WShape::Tris(t) => {
                let mut lo = v3(f64::MAX, f64::MAX, f64::MAX);
                let mut hi = v3(f64::MIN, f64::MIN, f64::MIN);
                for h in t {
                    let (l, u) = h.aabb();
                    lo = lo.min(l);
                    hi = hi.max(u);
                }
                Some((lo, hi))
            }
        }
    }
}
