//! Mathematical sculpting: shapes as distance functions, combined with smooth booleans, warped by domain
//! operations, displaced by noise or written as an equation of `x`, `y`, `z`; and the operations that apply them to
//! a volume.
use crate::noise::Spec;
use crate::volume::Volume;
use crate::{dot, len, mul, norm, sub, V3};
use std::sync::Arc;

use animscript_math as m;

/// Polynomial smooth minimum: `min(a, b)` rounded over a width `k` (the same as `min` for `k = 0`).
pub fn smin(a: f64, b: f64, k: f64) -> f64 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}
pub fn smax(a: f64, b: f64, k: f64) -> f64 {
    -smin(-a, -b, k)
}

/// A compiled equation `f(x, y, z)` whose value is the distance (negative inside).
pub struct Expr {
    prog: m::Program,
    pub source: String,
}

impl Expr {
    pub fn compile(src: &str) -> Result<Expr, String> {
        let mut b = m::Builder::new();
        let env = m::Env::new()
            .with("x", b.input("x", m::Rate::Point))
            .with("y", b.input("y", m::Rate::Point))
            .with("z", b.input("z", m::Rate::Point));
        let id = m::parse(src, &mut b, &env).map_err(|e| e.to_string())?;
        Ok(Expr {
            prog: b.finish(&[id]),
            source: src.to_string(),
        })
    }
    pub fn eval(&self, p: V3) -> f64 {
        self.prog
            .eval1(&p, &[], &[])
            .map(|v| v[0])
            .unwrap_or(f64::NAN)
    }
}

#[derive(Clone)]
pub enum Sdf {
    Sphere(f64),
    /// Half sizes and a corner rounding.
    Box(V3, f64),
    /// Major and minor radius, around the y axis.
    Torus(f64, f64),
    Capsule(V3, V3, f64),
    /// Radius and half height, along y.
    Cylinder(f64, f64),
    /// Bottom radius, top radius and half height, along y (a cone has a top radius of 0).
    Cone(f64, f64, f64),
    Ellipsoid(V3),
    /// Points with `dot(p, n) < d` are inside; `n` is a unit vector.
    Plane(V3, f64),
    Union(Vec<Sdf>, f64),
    Subtract(Box<Sdf>, Box<Sdf>, f64),
    Intersect(Vec<Sdf>, f64),
    Translate(Box<Sdf>, V3),
    /// Turns about y, then x, then z (the same order as a mesh's rotation).
    Rotate(Box<Sdf>, V3),
    Scale(Box<Sdf>, f64),
    /// Turns per unit length along an axis (0 x, 1 y, 2 z).
    Twist(Box<Sdf>, f64, usize),
    /// Curvature (1/units) bending the shape about the z axis along x.
    Bend(Box<Sdf>, f64),
    /// Repeat with a period per axis (0 leaves an axis alone).
    Repeat(Box<Sdf>, V3),
    Mirror(Box<Sdf>, usize),
    /// A hollow skin of this thickness.
    Shell(Box<Sdf>, f64),
    /// Grow (positive) or shrink (negative) by this much.
    Offset(Box<Sdf>, f64),
    /// Displace the distance by `amount * noise`.
    Noise(Box<Sdf>, Arc<Spec>, f64),
    Expr(Arc<Expr>),
}

fn rot(p: V3, turns: V3, inverse: bool) -> V3 {
    let tau = std::f64::consts::TAU;
    let ry = |p: V3, a: f64| {
        let (s, c) = (a * tau).sin_cos();
        [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c]
    };
    let rx = |p: V3, a: f64| {
        let (s, c) = (a * tau).sin_cos();
        [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c]
    };
    let rz = |p: V3, a: f64| {
        let (s, c) = (a * tau).sin_cos();
        [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
    };
    if inverse {
        ry(rx(rz(p, -turns[2]), -turns[0]), -turns[1])
    } else {
        rz(rx(ry(p, turns[1]), turns[0]), turns[2])
    }
}

fn vmax(a: V3, k: f64) -> V3 {
    [a[0].max(k), a[1].max(k), a[2].max(k)]
}

impl Sdf {
    pub fn eval(&self, p: V3) -> f64 {
        match self {
            Sdf::Sphere(r) => len(p) - r,
            Sdf::Box(h, round) => {
                let q = [p[0].abs() - h[0], p[1].abs() - h[1], p[2].abs() - h[2]];
                len(vmax(q, 0.0)) + q[0].max(q[1]).max(q[2]).min(0.0) - round
            }
            Sdf::Torus(r1, r2) => {
                let q = [(p[0] * p[0] + p[2] * p[2]).sqrt() - r1, p[1]];
                (q[0] * q[0] + q[1] * q[1]).sqrt() - r2
            }
            Sdf::Capsule(a, b, r) => {
                let (pa, ba) = (sub(p, *a), sub(*b, *a));
                let t = (dot(pa, ba) / dot(ba, ba).max(1e-300)).clamp(0.0, 1.0);
                len(sub(pa, mul(ba, t))) - r
            }
            Sdf::Cylinder(r, hh) => {
                let d = [(p[0] * p[0] + p[2] * p[2]).sqrt() - r, p[1].abs() - hh];
                d[0].max(d[1]).min(0.0) + (d[0].max(0.0).powi(2) + d[1].max(0.0).powi(2)).sqrt()
            }
            Sdf::Cone(r1, r2, hh) => {
                // capped cone along y (Inigo Quilez)
                let q = [(p[0] * p[0] + p[2] * p[2]).sqrt(), p[1]];
                let k1 = [*r2, *hh];
                let k2 = [r2 - r1, 2.0 * hh];
                let ca = [
                    q[0] - q[0].min(if q[1] < 0.0 { *r1 } else { *r2 }),
                    q[1].abs() - hh,
                ];
                let t = (((k1[0] - q[0]) * k2[0] + (k1[1] - q[1]) * k2[1])
                    / (k2[0] * k2[0] + k2[1] * k2[1]).max(1e-300))
                .clamp(0.0, 1.0);
                let cb = [q[0] - k1[0] + k2[0] * t, q[1] - k1[1] + k2[1] * t];
                let s = if cb[0] < 0.0 && ca[1] < 0.0 {
                    -1.0
                } else {
                    1.0
                };
                s * (ca[0] * ca[0] + ca[1] * ca[1])
                    .min(cb[0] * cb[0] + cb[1] * cb[1])
                    .sqrt()
            }
            Sdf::Ellipsoid(r) => {
                let k0 = len([p[0] / r[0], p[1] / r[1], p[2] / r[2]]);
                let k1 = len([
                    p[0] / (r[0] * r[0]),
                    p[1] / (r[1] * r[1]),
                    p[2] / (r[2] * r[2]),
                ]);
                if k1 < 1e-300 {
                    -r[0].min(r[1]).min(r[2])
                } else {
                    k0 * (k0 - 1.0) / k1
                }
            }
            Sdf::Plane(n, d) => dot(p, *n) - d,
            Sdf::Union(c, k) => c
                .iter()
                .map(|s| s.eval(p))
                .reduce(|a, b| smin(a, b, *k))
                .unwrap_or(1e9),
            Sdf::Intersect(c, k) => c
                .iter()
                .map(|s| s.eval(p))
                .reduce(|a, b| smax(a, b, *k))
                .unwrap_or(-1e9),
            Sdf::Subtract(a, b, k) => smax(a.eval(p), -b.eval(p), *k),
            Sdf::Translate(s, t) => s.eval(sub(p, *t)),
            Sdf::Rotate(s, r) => s.eval(rot(p, *r, true)),
            Sdf::Scale(s, k) => s.eval(mul(p, 1.0 / k)) * k,
            Sdf::Twist(s, k, axis) => {
                let ang = k * p[*axis];
                let mut turns = [0.0; 3];
                turns[*axis] = -ang;
                s.eval(rot(p, turns, false))
            }
            Sdf::Bend(s, k) => {
                let (c, sn) = ((k * p[0]).cos(), (k * p[0]).sin());
                s.eval([c * p[0] - sn * p[1], sn * p[0] + c * p[1], p[2]])
            }
            Sdf::Repeat(s, per) => {
                let mut q = p;
                for a in 0..3 {
                    if per[a] > 0.0 {
                        q[a] = p[a] - per[a] * (p[a] / per[a]).round();
                    }
                }
                s.eval(q)
            }
            Sdf::Mirror(s, axis) => {
                let mut q = p;
                q[*axis] = q[*axis].abs();
                s.eval(q)
            }
            Sdf::Shell(s, t) => s.eval(p).abs() - t * 0.5,
            Sdf::Offset(s, d) => s.eval(p) - d,
            Sdf::Noise(s, spec, amount) => s.eval(p) + amount * spec.eval(p),
            Sdf::Expr(e) => e.eval(p),
        }
    }
}

/// How a shape combines with what is already in the volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Add the shape (smooth union).
    Add,
    /// Cut the shape out (smooth subtraction).
    Subtract,
    /// Keep only what is inside the shape.
    Intersect,
    /// Write the shape's field over the region, ignoring what was there.
    Replace,
}

/// Apply a shape to a volume within the world box `lo..hi` (plus the band around it) with a smoothing width `k`.
pub fn apply(vol: &mut Volume, sdf: &Sdf, op: Op, k: f64, lo: V3, hi: V3) {
    let pad = vol.band + vol.h;
    let (il, ih) = vol.index_box(
        [lo[0] - pad, lo[1] - pad, lo[2] - pad],
        [hi[0] + pad, hi[1] + pad, hi[2] + pad],
    );
    vol.edit(il, ih, &mut |p, old| {
        let f = sdf.eval(p);
        match op {
            Op::Add => smin(old, f, k),
            Op::Subtract => smax(old, -f, k),
            Op::Intersect => smax(old, f, k),
            Op::Replace => f,
        }
    });
}

/// The unit normal of a field at a point from central differences (helper for tests and brushes).
pub fn field_normal(f: &dyn Fn(V3) -> f64, p: V3, e: f64) -> V3 {
    let g = |a: usize| {
        let (mut x, mut y) = (p, p);
        x[a] += e;
        y[a] -= e;
        f(x) - f(y)
    };
    norm([g(0), g(1), g(2)])
}
