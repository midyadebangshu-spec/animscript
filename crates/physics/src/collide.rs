//! Contact manifolds in world space. A manifold's normal points from shape A to shape B and each
//! point carries its separation (negative = penetration). Points up to `margin` apart are kept so
//! the solver can approach them speculatively.
use crate::math::*;
use crate::shape::*;

#[derive(Clone, Debug)]
pub struct ManifoldPoint {
    pub point: Vec2,
    pub sep: f64,
    /// Feature id, stable between steps so impulses can be warm started.
    pub id: u32,
    pub normal_impulse: f64,
    pub tangent_impulse: f64,
}

#[derive(Clone, Debug)]
pub struct Manifold {
    pub normal: Vec2,
    pub points: Vec<ManifoldPoint>,
}

struct WPoly {
    v: Vec<Vec2>,
    n: Vec<Vec2>,
}

fn world_poly(s: &Shape, xf: &Xf) -> Option<WPoly> {
    match s {
        Shape::Circle { .. } => None,
        Shape::Polygon(p) => Some(WPoly {
            v: p.verts.iter().map(|v| xf.apply(*v)).collect(),
            n: p.normals.iter().map(|n| xf.q.rotate(*n)).collect(),
        }),
        Shape::Edge { a, b } => {
            let p = Polygon::two_gon(*a, *b);
            Some(WPoly {
                v: p.verts.iter().map(|v| xf.apply(*v)).collect(),
                n: p.normals.iter().map(|n| xf.q.rotate(*n)).collect(),
            })
        }
    }
}

fn point(point: Vec2, sep: f64, id: u32) -> ManifoldPoint {
    ManifoldPoint {
        point,
        sep,
        id,
        normal_impulse: 0.0,
        tangent_impulse: 0.0,
    }
}

pub fn collide(a: &Shape, xa: &Xf, b: &Shape, xb: &Xf, margin: f64) -> Option<Manifold> {
    match (a, b) {
        (
            Shape::Circle {
                center: ca,
                radius: ra,
            },
            Shape::Circle {
                center: cb,
                radius: rb,
            },
        ) => {
            let (ca, cb) = (xa.apply(*ca), xb.apply(*cb));
            let d = cb - ca;
            let dist = d.length();
            let sep = dist - ra - rb;
            if sep > margin {
                return None;
            }
            let n = if dist > 1e-12 { d / dist } else { v2(0.0, 1.0) };
            let p = (ca + n * *ra + cb - n * *rb) * 0.5;
            Some(Manifold {
                normal: n,
                points: vec![point(p, sep, 0)],
            })
        }
        (Shape::Circle { center, radius }, _) => {
            let poly = world_poly(b, xb)?;
            let mut m = poly_circle(&poly, xa.apply(*center), *radius, margin)?;
            m.normal = -m.normal;
            Some(m)
        }
        (_, Shape::Circle { center, radius }) => {
            poly_circle(&world_poly(a, xa)?, xb.apply(*center), *radius, margin)
        }
        _ => polys(&world_poly(a, xa)?, &world_poly(b, xb)?, margin),
    }
}

/// Polygon A against circle B; the normal points from the polygon to the circle.
fn poly_circle(p: &WPoly, c: Vec2, r: f64, margin: f64) -> Option<Manifold> {
    let n = p.v.len();
    let (mut bi, mut bs) = (0, f64::MIN);
    for i in 0..n {
        let s = p.n[i].dot(c - p.v[i]);
        if s > bs {
            bi = i;
            bs = s;
        }
    }
    if bs - r > margin {
        return None;
    }
    let (v1, v2_) = (p.v[bi], p.v[(bi + 1) % n]);
    if bs > 0.0 {
        let u1 = (c - v1).dot(v2_ - v1);
        let u2 = (c - v2_).dot(v1 - v2_);
        for (u, v) in [(u1, v1), (u2, v2_)] {
            if u <= 0.0 {
                let d = c - v;
                let dist = d.length();
                if dist - r > margin {
                    return None;
                }
                let nrm = if dist > 1e-12 { d / dist } else { p.n[bi] };
                let mid = (v + (c - nrm * r)) * 0.5;
                return Some(Manifold {
                    normal: nrm,
                    points: vec![point(mid, dist - r, 0)],
                });
            }
        }
    }
    let nrm = p.n[bi];
    let mid = c - nrm * ((bs + r) * 0.5);
    Some(Manifold {
        normal: nrm,
        points: vec![point(mid, bs - r, 0)],
    })
}

fn max_separation(a: &WPoly, b: &WPoly) -> (usize, f64) {
    let (mut best, mut sep) = (0, f64::MIN);
    for i in 0..a.v.len() {
        let (n, v) = (a.n[i], a.v[i]);
        let s = b.v.iter().map(|p| n.dot(*p - v)).fold(f64::MAX, f64::min);
        if s > sep {
            best = i;
            sep = s;
        }
    }
    (best, sep)
}

fn polys(a: &WPoly, b: &WPoly, margin: f64) -> Option<Manifold> {
    let (ea, sa) = max_separation(a, b);
    let (eb, sb) = max_separation(b, a);
    if sa > margin || sb > margin {
        return None;
    }
    // prefer A as the reference face unless B is clearly better (avoids flip-flopping)
    let flip = sb > sa + 0.0005;
    let (r, i, inc) = if flip { (b, eb, a) } else { (a, ea, b) };
    let nref = r.n[i];
    let (v1, v2_) = (r.v[i], r.v[(i + 1) % r.v.len()]);
    // incident edge: most anti-parallel to the reference normal
    let mut j = 0;
    let mut least = f64::MAX;
    for (k, n) in inc.n.iter().enumerate() {
        let d = nref.dot(*n);
        if d < least {
            least = d;
            j = k;
        }
    }
    let flag = if flip { 1u32 << 31 } else { 0 };
    let k1 = j;
    let k2 = (j + 1) % inc.v.len();
    let mut seg = [(inc.v[k1], k1 as u32), (inc.v[k2], k2 as u32)];
    let t = (v2_ - v1).normalized();
    // clip to the reference face's side planes
    for (nrm, off) in [(t, t.dot(v1)), (-t, -t.dot(v2_))] {
        let d0 = nrm.dot(seg[0].0) - off;
        let d1 = nrm.dot(seg[1].0) - off;
        if d0 < 0.0 && d1 < 0.0 {
            return None;
        }
        if d0 < 0.0 {
            seg[0].0 = seg[0].0 + (seg[1].0 - seg[0].0) * (d0 / (d0 - d1));
        } else if d1 < 0.0 {
            seg[1].0 = seg[1].0 + (seg[0].0 - seg[1].0) * (d1 / (d1 - d0));
        }
    }
    let normal = if flip { -nref } else { nref };
    let mut points = vec![];
    for (p, k) in seg {
        let s = nref.dot(p - v1);
        if s <= margin {
            points.push(point(
                p - nref * (s * 0.5),
                s,
                flag | ((i as u32) << 16) | k,
            ));
        }
    }
    if points.is_empty() {
        None
    } else {
        Some(Manifold { normal, points })
    }
}
