//! Narrow phase: contact manifolds between world-space shapes. The normal always points from the
//! first shape towards the second. Convex polyhedra use the separating axis test (face axes and
//! the edge pairs that form faces of the Minkowski difference) and face clipping for the points.
use crate::hull::*;
use crate::math::*;
use crate::shape::WShape;

#[derive(Clone, Debug)]
pub struct Manifold {
    pub normal: V3,
    /// World point and penetration depth.
    pub points: Vec<(V3, f64)>,
}

/// The contact manifolds of two shapes (several when one is a triangle mesh).
pub fn collide(a: &WShape, b: &WShape) -> Vec<Manifold> {
    use WShape::*;
    match (a, b) {
        (Tris(ts), _) => tris_vs(ts, b, false),
        (_, Tris(ts)) => tris_vs(ts, a, true),
        (Plane { .. }, Plane { .. }) => vec![],
        (Plane { normal, d }, other) => one(plane_vs(*normal, *d, other)),
        (other, Plane { normal, d }) => one(plane_vs(*normal, *d, other).map(flip)),
        (Sphere { c: ca, r: ra }, Sphere { c: cb, r: rb }) => {
            one(sphere_sphere(*ca, *ra, *cb, *rb))
        }
        (Sphere { c, r }, Capsule { a: p, b: q, r: rc }) => {
            one(sphere_capsule(*c, *r, *p, *q, *rc))
        }
        (Capsule { a: p, b: q, r: rc }, Sphere { c, r }) => {
            one(sphere_capsule(*c, *r, *p, *q, *rc).map(flip))
        }
        (
            Capsule {
                a: p0,
                b: q0,
                r: r0,
            },
            Capsule {
                a: p1,
                b: q1,
                r: r1,
            },
        ) => one(capsule_capsule(*p0, *q0, *r0, *p1, *q1, *r1)),
        (Sphere { c, r }, Hull(h)) => one(capsule_hull(*c, *c, *r, h).map(flip)),
        (Hull(h), Sphere { c, r }) => one(capsule_hull(*c, *c, *r, h)),
        (Capsule { a: p, b: q, r }, Hull(h)) => one(capsule_hull(*p, *q, *r, h).map(flip)),
        (Hull(h), Capsule { a: p, b: q, r }) => one(capsule_hull(*p, *q, *r, h)),
        (Hull(x), Hull(y)) => one(hull_hull(x, y)),
    }
}

fn one(m: Option<Manifold>) -> Vec<Manifold> {
    m.into_iter().collect()
}

fn flip(m: Manifold) -> Manifold {
    Manifold {
        normal: -m.normal,
        points: m.points,
    }
}

fn tris_vs(ts: &[Hull], other: &WShape, other_first: bool) -> Vec<Manifold> {
    let Some((lo, hi)) = other.aabb() else {
        return vec![];
    };
    let mut out = vec![];
    for t in ts {
        let (l, u) = t.aabb();
        if l.x > hi.x || u.x < lo.x || l.y > hi.y || u.y < lo.y || l.z > hi.z || u.z < lo.z {
            continue;
        }
        let ws = WShape::Hull(t.clone());
        let ms = if other_first {
            collide(other, &ws)
        } else {
            collide(&ws, other)
        };
        out.extend(ms);
    }
    out
}

fn plane_vs(n: V3, d: f64, other: &WShape) -> Option<Manifold> {
    // normal points from the plane towards the other shape
    let mut pts: Vec<(V3, f64)> = vec![];
    match other {
        WShape::Sphere { c, r } => {
            let h = n.dot(*c) - d;
            if h < *r {
                pts.push((*c - n * h, r - h));
            }
        }
        WShape::Capsule { a, b, r } => {
            for c in [a, b] {
                let h = n.dot(*c) - d;
                if h < *r {
                    pts.push((*c - n * h, r - h));
                }
            }
        }
        WShape::Hull(hh) => {
            for v in &hh.verts {
                let h = n.dot(*v) - d;
                if h < 0.0 {
                    pts.push((*v - n * h, -h));
                }
            }
        }
        _ => {}
    }
    if pts.is_empty() {
        return None;
    }
    Some(Manifold {
        normal: n,
        points: reduce(pts, n),
    })
}

fn sphere_sphere(ca: V3, ra: f64, cb: V3, rb: f64) -> Option<Manifold> {
    let d = cb - ca;
    let l = d.len();
    if l >= ra + rb {
        return None;
    }
    let n = if l > 1e-12 { d / l } else { v3(0.0, -1.0, 0.0) };
    Some(Manifold {
        normal: n,
        points: vec![(ca + n * (ra - (ra + rb - l) * 0.5), ra + rb - l)],
    })
}

fn closest_on_segment(a: V3, b: V3, p: V3) -> (V3, f64) {
    let ab = b - a;
    let l2 = ab.len2();
    let t = if l2 < 1e-300 {
        0.0
    } else {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    };
    (a + ab * t, t)
}

fn sphere_capsule(c: V3, r: f64, p: V3, q: V3, rc: f64) -> Option<Manifold> {
    let (cp, _) = closest_on_segment(p, q, c);
    sphere_sphere(c, r, cp, rc)
}

/// Closest points of two segments, as parameters.
fn segment_segment(p1: V3, q1: V3, p2: V3, q2: V3) -> (f64, f64) {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    if a < 1e-300 && e < 1e-300 {
        return (0.0, 0.0);
    }
    if a < 1e-300 {
        return (0.0, (f / e).clamp(0.0, 1.0));
    }
    let c = d1.dot(r);
    if e < 1e-300 {
        return ((-c / a).clamp(0.0, 1.0), 0.0);
    }
    let b = d1.dot(d2);
    let den = a * e - b * b;
    let mut s = if den > 1e-12 * a * e {
        ((b * f - c * e) / den).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    (s, t)
}

fn capsule_capsule(p0: V3, q0: V3, r0: f64, p1: V3, q1: V3, r1: f64) -> Option<Manifold> {
    let (s, t) = segment_segment(p0, q0, p1, q1);
    sphere_sphere(p0 + (q0 - p0) * s, r0, p1 + (q1 - p1) * t, r1)
}

/// Closest point of a polygon face to `p`.
fn closest_on_face(h: &Hull, f: &Face, p: V3) -> V3 {
    let proj = p - f.normal * (f.normal.dot(p) - f.d);
    let k = f.verts.len();
    let mut inside = true;
    for e in 0..k {
        let (a, b) = (h.verts[f.verts[e]], h.verts[f.verts[(e + 1) % k]]);
        if (b - a).cross(proj - a).dot(f.normal) < 0.0 {
            inside = false;
            break;
        }
    }
    if inside {
        return proj;
    }
    let mut best = h.verts[f.verts[0]];
    let mut bd = f64::MAX;
    for e in 0..k {
        let (a, b) = (h.verts[f.verts[e]], h.verts[f.verts[(e + 1) % k]]);
        let (c, _) = closest_on_segment(a, b, p);
        let d = (c - p).len2();
        if d < bd {
            bd = d;
            best = c;
        }
    }
    best
}

/// Signed distance from `c` to the hull: the surface point nearest to it and the outward normal there.
fn hull_probe(h: &Hull, c: V3) -> (f64, V3, V3) {
    let mut smax = f64::MIN;
    let mut fi = 0;
    for (i, f) in h.faces.iter().enumerate() {
        let s = f.normal.dot(c) - f.d;
        if s > smax {
            smax = s;
            fi = i;
        }
    }
    if smax <= 0.0 {
        let n = h.faces[fi].normal;
        return (smax, c - n * smax, n);
    }
    let mut best = (f64::MAX, V3::ZERO);
    for f in &h.faces {
        if f.normal.dot(c) - f.d <= 0.0 {
            continue;
        }
        let q = closest_on_face(h, f, c);
        let d = (c - q).len2();
        if d < best.0 {
            best = (d, q);
        }
    }
    let n = (c - best.1).normalized();
    (best.0.sqrt(), best.1, n)
}

/// A capsule (a sphere when the ends coincide) against a hull; the normal points from the hull
/// to the capsule (the hull is the first shape of the pair, as `collide` calls it).
fn capsule_hull(a: V3, b: V3, r: f64, h: &Hull) -> Option<Manifold> {
    let at = |t: f64| a + (b - a) * t;
    let sd = |t: f64| hull_probe(h, at(t)).0;
    let mut best_t = 0.0;
    if (b - a).len2() > 1e-18 {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            if sd(m1) < sd(m2) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        best_t = (lo + hi) * 0.5;
    }
    let s_best = sd(best_t);
    if s_best >= r {
        return None;
    }
    let (_, q0, n0) = hull_probe(h, at(best_t));
    let mut points = vec![(q0, r - s_best)];
    if (b - a).len2() > 1e-18 {
        for t in [0.0, 1.0] {
            if (t - best_t).abs() < 1e-9 {
                continue;
            }
            let (s, q, _) = hull_probe(h, at(t));
            if s < r && s < s_best + 0.05 * r.max(1e-9) + 1e-6 {
                points.push((q, r - s));
            }
        }
    }
    let n = if n0 == V3::ZERO {
        v3(0.0, -1.0, 0.0)
    } else {
        n0
    };
    // `n` points out of the hull, i.e. from the first shape (hull) to the capsule
    Some(Manifold { normal: n, points })
}

fn face_sep(a: &Hull, b: &Hull) -> (f64, usize) {
    let mut best = (f64::MIN, 0);
    for (i, f) in a.faces.iter().enumerate() {
        let m = b
            .verts
            .iter()
            .map(|v| f.normal.dot(*v) - f.d)
            .fold(f64::MAX, f64::min);
        if m > best.0 {
            best = (m, i);
        }
    }
    best
}

fn minkowski_face(a: V3, b: V3, c: V3, d: V3) -> bool {
    let bxa = b.cross(a);
    let dxc = d.cross(c);
    let cba = c.dot(bxa);
    let dba = d.dot(bxa);
    let adc = a.dot(dxc);
    let bdc = b.dot(dxc);
    cba * dba < 0.0 && adc * bdc < 0.0 && cba * bdc > 0.0
}

fn hull_hull(a: &Hull, b: &Hull) -> Option<Manifold> {
    let (sa, fa) = face_sep(a, b);
    if sa > 0.0 {
        return None;
    }
    let (sb, fb) = face_sep(b, a);
    if sb > 0.0 {
        return None;
    }
    // edge axes
    let ca = a.centre();
    let cb = b.centre();
    let mut best_edge: Option<(f64, usize, usize, V3)> = None;
    for (ia, ea) in a.edges.iter().enumerate() {
        let (a1, a2) = (a.faces[ea.f1].normal, a.faces[ea.f2].normal);
        let da = a.verts[ea.b] - a.verts[ea.a];
        for (ib, eb) in b.edges.iter().enumerate() {
            let (b1, b2) = (b.faces[eb.f1].normal, b.faces[eb.f2].normal);
            if !minkowski_face(a1, a2, -b1, -b2) {
                continue;
            }
            let db = b.verts[eb.b] - b.verts[eb.a];
            let mut axis = da.cross(db);
            let l = axis.len();
            if l < 1e-9 * da.len() * db.len() {
                continue;
            }
            axis = axis / l;
            if axis.dot(cb - ca) < 0.0 {
                axis = -axis;
            }
            let sep = b
                .verts
                .iter()
                .map(|v| axis.dot(*v))
                .fold(f64::MAX, f64::min)
                - a.verts
                    .iter()
                    .map(|v| axis.dot(*v))
                    .fold(f64::MIN, f64::max);
            if sep > 0.0 {
                return None;
            }
            if best_edge.is_none_or(|e| sep > e.0) {
                best_edge = Some((sep, ia, ib, axis));
            }
        }
    }
    let face_best = sa.max(sb);
    if let Some((se, ia, ib, axis)) = best_edge {
        if se > 0.95 * face_best + 1e-5 {
            let (ea, eb) = (a.edges[ia], b.edges[ib]);
            let (p1, q1) = (a.verts[ea.a], a.verts[ea.b]);
            let (p2, q2) = (b.verts[eb.a], b.verts[eb.b]);
            let (s, t) = segment_segment(p1, q1, p2, q2);
            let (x, y) = (p1 + (q1 - p1) * s, p2 + (q2 - p2) * t);
            return Some(Manifold {
                normal: axis,
                points: vec![((x + y) * 0.5, -se)],
            });
        }
    }
    // face contact: the reference face is on the hull with the shallower penetration
    let (r, i, ref_is_a) = if sa >= sb * 0.98 - 1e-6 {
        (a, a.faces[fa].clone(), true)
    } else {
        (b, b.faces[fb].clone(), false)
    };
    let inc_hull = if ref_is_a { b } else { a };
    let n_ref = i.normal;
    // incident face: the most anti-parallel
    let inc = inc_hull
        .faces
        .iter()
        .min_by(|x, y| {
            x.normal
                .dot(n_ref)
                .partial_cmp(&y.normal.dot(n_ref))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("hull has faces");
    let mut poly: Vec<V3> = inc.verts.iter().map(|k| inc_hull.verts[*k]).collect();
    let k = i.verts.len();
    for e in 0..k {
        let (p, q) = (r.verts[i.verts[e]], r.verts[i.verts[(e + 1) % k]]);
        let side = (q - p).cross(n_ref).normalized();
        let dd = side.dot(p);
        poly = clip(&poly, side, dd);
        if poly.is_empty() {
            break;
        }
    }
    let mut pts: Vec<(V3, f64)> = vec![];
    for p in poly {
        let dist = n_ref.dot(p) - i.d;
        if dist <= 1e-9 {
            pts.push((p - n_ref * (dist * 0.5), -dist));
        }
    }
    if pts.is_empty() {
        return None;
    }
    let normal = if ref_is_a { n_ref } else { -n_ref };
    Some(Manifold {
        normal,
        points: reduce(pts, normal),
    })
}

/// Keep the part of a polygon with `n . p <= d`.
fn clip(poly: &[V3], n: V3, d: f64) -> Vec<V3> {
    let mut out = vec![];
    let k = poly.len();
    for e in 0..k {
        let (p, q) = (poly[e], poly[(e + 1) % k]);
        let (dp, dq) = (n.dot(p) - d, n.dot(q) - d);
        if dp <= 0.0 {
            out.push(p);
        }
        if (dp < 0.0 && dq > 0.0) || (dp > 0.0 && dq < 0.0) {
            let t = dp / (dp - dq);
            out.push(p + (q - p) * t);
        }
    }
    out
}

/// At most four contact points that cover the manifold well: the deepest, the farthest from it,
/// then the two that make the largest area.
fn reduce(pts: Vec<(V3, f64)>, normal: V3) -> Vec<(V3, f64)> {
    if pts.len() <= 4 {
        return pts;
    }
    let mut chosen: Vec<usize> = vec![];
    let deepest = (0..pts.len()).fold(0, |b, i| if pts[i].1 > pts[b].1 { i } else { b });
    chosen.push(deepest);
    let far = (0..pts.len()).fold(deepest, |b, i| {
        if (pts[i].0 - pts[deepest].0).len2() > (pts[b].0 - pts[deepest].0).len2() {
            i
        } else {
            b
        }
    });
    chosen.push(far);
    let (p0, p1) = (pts[deepest].0, pts[far].0);
    let area = |i: usize| (pts[i].0 - p0).cross(p1 - p0).dot(normal);
    let pos = (0..pts.len()).fold(deepest, |b, i| if area(i) > area(b) { i } else { b });
    let neg = (0..pts.len()).fold(deepest, |b, i| if area(i) < area(b) { i } else { b });
    for c in [pos, neg] {
        if !chosen.contains(&c) {
            chosen.push(c);
        }
    }
    for i in 0..pts.len() {
        if chosen.len() >= 4 {
            break;
        }
        if !chosen.contains(&i) {
            chosen.push(i);
        }
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| pts[i]).collect()
}
