//! Collision shapes, mass properties, convex decomposition and the broadphase.
use crate::math::*;

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub lo: Vec2,
    pub hi: Vec2,
}

impl Aabb {
    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.lo.x <= o.hi.x && o.lo.x <= self.hi.x && self.lo.y <= o.hi.y && o.lo.y <= self.hi.y
    }
    pub fn expanded(&self, m: f64) -> Aabb {
        Aabb {
            lo: v2(self.lo.x - m, self.lo.y - m),
            hi: v2(self.hi.x + m, self.hi.y + m),
        }
    }
}

/// A convex polygon with positive signed area (`sum cross(v_i, v_i+1) > 0`) and outward normals.
#[derive(Clone, Debug)]
pub struct Polygon {
    pub verts: Vec<Vec2>,
    pub normals: Vec<Vec2>,
}

impl Polygon {
    /// `None` unless the points form a strictly convex polygon with positive area.
    pub fn new(verts: Vec<Vec2>) -> Option<Polygon> {
        let n = verts.len();
        if n < 3 {
            return None;
        }
        let area2: f64 = (0..n).map(|i| verts[i].cross(verts[(i + 1) % n])).sum();
        if area2.is_nan() || area2 <= 1e-12 {
            return None;
        }
        let mut normals = Vec::with_capacity(n);
        for i in 0..n {
            let e = verts[(i + 1) % n] - verts[i];
            let e2 = verts[(i + 2) % n] - verts[(i + 1) % n];
            let turn = e.cross(e2);
            if e.length_sq() < 1e-24 || turn.is_nan() || turn <= 1e-14 {
                return None;
            }
            normals.push(v2(e.y, -e.x).normalized());
        }
        Some(Polygon { verts, normals })
    }

    /// A segment as a two-vertex polygon with opposite normals (used by `Shape::Edge`).
    pub(crate) fn two_gon(a: Vec2, b: Vec2) -> Polygon {
        let e = b - a;
        let n = v2(e.y, -e.x).normalized();
        Polygon {
            verts: vec![a, b],
            normals: vec![n, -n],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MassData {
    pub mass: f64,
    /// Centre of mass in the shape's local frame.
    pub center: Vec2,
    /// Rotational inertia about the centre of mass.
    pub inertia: f64,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Circle {
        center: Vec2,
        radius: f64,
    },
    Polygon(Polygon),
    /// A thin two-sided segment (mass zero; for static boundaries).
    Edge {
        a: Vec2,
        b: Vec2,
    },
}

impl Shape {
    /// A box of half extents `hx`, `hy` centred on the origin.
    pub fn boxed(hx: f64, hy: f64) -> Shape {
        Shape::Polygon(
            Polygon::new(vec![v2(-hx, -hy), v2(hx, -hy), v2(hx, hy), v2(-hx, hy)])
                .expect("a box has positive extents"),
        )
    }
    pub fn edge(a: Vec2, b: Vec2) -> Shape {
        Shape::Edge { a, b }
    }
    pub fn mass_data(&self, density: f64) -> MassData {
        match self {
            Shape::Circle { center, radius } => {
                let mass = density * std::f64::consts::PI * radius * radius;
                MassData {
                    mass,
                    center: *center,
                    inertia: 0.5 * mass * radius * radius,
                }
            }
            Shape::Edge { a, b } => MassData {
                mass: 0.0,
                center: (*a + *b) * 0.5,
                inertia: 0.0,
            },
            Shape::Polygon(p) => {
                let s = p.verts[0];
                let (mut area, mut center, mut inertia) = (0.0, Vec2::ZERO, 0.0);
                for i in 1..p.verts.len() - 1 {
                    let e1 = p.verts[i] - s;
                    let e2 = p.verts[i + 1] - s;
                    let d = e1.cross(e2);
                    let tri = 0.5 * d;
                    area += tri;
                    center += (e1 + e2) * (tri / 3.0);
                    let intx2 = e1.x * e1.x + e2.x * e1.x + e2.x * e2.x;
                    let inty2 = e1.y * e1.y + e2.y * e1.y + e2.y * e2.y;
                    inertia += (0.25 / 3.0) * d * (intx2 + inty2);
                }
                let mass = density * area;
                let c_local = center / area;
                let i_origin = density * inertia;
                MassData {
                    mass,
                    center: c_local + s,
                    inertia: i_origin - mass * c_local.length_sq(),
                }
            }
        }
    }
    pub fn aabb(&self, xf: &Xf) -> Aabb {
        match self {
            Shape::Circle { center, radius } => {
                let c = xf.apply(*center);
                Aabb {
                    lo: v2(c.x - radius, c.y - radius),
                    hi: v2(c.x + radius, c.y + radius),
                }
            }
            Shape::Edge { a, b } => bounds(&[xf.apply(*a), xf.apply(*b)]),
            Shape::Polygon(p) => bounds(&p.verts.iter().map(|v| xf.apply(*v)).collect::<Vec<_>>()),
        }
    }
}

fn bounds(pts: &[Vec2]) -> Aabb {
    let mut lo = pts[0];
    let mut hi = pts[0];
    for p in &pts[1..] {
        lo = v2(lo.x.min(p.x), lo.y.min(p.y));
        hi = v2(hi.x.max(p.x), hi.y.max(p.y));
    }
    Aabb { lo, hi }
}

// ---- broadphase ---------------------------------------------------------------------------

/// All overlapping pairs `(i, j)` with `i < j`, sorted. Sweep along x, then test y.
pub fn find_pairs(boxes: &[Aabb]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| boxes[a].lo.x.total_cmp(&boxes[b].lo.x).then(a.cmp(&b)));
    let mut pairs = vec![];
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            if boxes[j].lo.x > boxes[i].hi.x {
                break;
            }
            if boxes[i].overlaps(&boxes[j]) {
                pairs.push((i.min(j), i.max(j)));
            }
        }
    }
    pairs.sort();
    pairs
}

// ---- convex decomposition -----------------------------------------------------------------

fn signed_area(p: &[Vec2]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].cross(p[(i + 1) % n])).sum::<f64>() / 2.0
}

fn segments_intersect(p1: Vec2, p2: Vec2, p3: Vec2, p4: Vec2) -> bool {
    let eps = 1e-12;
    let d1 = (p4 - p3).cross(p1 - p3);
    let d2 = (p4 - p3).cross(p2 - p3);
    let d3 = (p2 - p1).cross(p3 - p1);
    let d4 = (p2 - p1).cross(p4 - p1);
    if ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
        && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    {
        return true;
    }
    let on = |a: Vec2, b: Vec2, p: Vec2| {
        p.x >= a.x.min(b.x) - eps
            && p.x <= a.x.max(b.x) + eps
            && p.y >= a.y.min(b.y) - eps
            && p.y <= a.y.max(b.y) + eps
    };
    (d1.abs() <= eps && on(p3, p4, p1))
        || (d2.abs() <= eps && on(p3, p4, p2))
        || (d3.abs() <= eps && on(p1, p2, p3))
        || (d4.abs() <= eps && on(p1, p2, p4))
}

fn is_simple(p: &[Vec2]) -> bool {
    let n = p.len();
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if segments_intersect(p[i], p[(i + 1) % n], p[j], p[(j + 1) % n]) {
                return false;
            }
        }
    }
    true
}

fn remove_collinear(p: &mut Vec<Vec2>) {
    loop {
        let n = p.len();
        if n < 3 {
            return;
        }
        let found = (0..n).find(|&i| {
            let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
            let (e1, e2) = (b - a, c - b);
            e1.cross(e2).abs() <= 1e-9 * e1.length() * e2.length() || e1.length_sq() < 1e-24
        });
        match found {
            Some(i) => {
                p.remove(i);
            }
            None => return,
        }
    }
}

fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let eps = -1e-12;
    (b - a).cross(p - a) >= eps && (c - b).cross(p - b) >= eps && (a - c).cross(p - c) >= eps
}

fn is_convex_idx(poly: &[usize], pts: &[Vec2]) -> bool {
    let n = poly.len();
    (0..n).all(|i| {
        let (a, b, c) = (pts[poly[i]], pts[poly[(i + 1) % n]], pts[poly[(i + 2) % n]]);
        (b - a).cross(c - b) >= -1e-12
    })
}

fn try_merge(pi: &[usize], pj: &[usize], pts: &[Vec2]) -> Option<Vec<usize>> {
    let (li, lj) = (pi.len(), pj.len());
    for a in 0..li {
        let (u, v) = (pi[a], pi[(a + 1) % li]);
        for b in 0..lj {
            if pj[b] == v && pj[(b + 1) % lj] == u {
                let mut merged: Vec<usize> = (0..li).map(|k| pi[(a + 1 + k) % li]).collect(); // v ... u
                for k in 2..lj {
                    merged.push(pj[(b + k) % lj]);
                }
                return is_convex_idx(&merged, pts).then_some(merged);
            }
        }
    }
    None
}

/// Split a simple polygon (either winding, possibly concave) into convex, counter-clockwise
/// pieces whose total area equals the original: ear clipping, then Hertel–Mehlhorn merging.
pub fn decompose(poly: &[Vec2]) -> Result<Vec<Vec<Vec2>>, String> {
    if poly.len() < 3 {
        return Err("a polygon needs at least 3 points".into());
    }
    let mut pts = poly.to_vec();
    let area = signed_area(&pts);
    if area.abs() < 1e-12 {
        return Err("the polygon has no area".into());
    }
    if area < 0.0 {
        pts.reverse();
    }
    if !is_simple(&pts) {
        return Err("the polygon crosses itself".into());
    }
    remove_collinear(&mut pts);
    if pts.len() < 3 {
        return Err("the polygon has no area".into());
    }
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut tris: Vec<Vec<usize>> = vec![];
    while idx.len() > 3 {
        let len = idx.len();
        let ear = (0..len).find(|&k| {
            let (ia, ib, ic) = (idx[(k + len - 1) % len], idx[k], idx[(k + 1) % len]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if (b - a).cross(c - b) <= 1e-12 {
                return false;
            }
            !idx.iter()
                .any(|&j| j != ia && j != ib && j != ic && in_triangle(pts[j], a, b, c))
        });
        match ear {
            Some(k) => {
                tris.push(vec![idx[(k + len - 1) % len], idx[k], idx[(k + 1) % len]]);
                idx.remove(k);
            }
            None => return Err("could not triangulate the polygon".into()),
        }
    }
    tris.push(idx);
    let mut polys = tris;
    'again: loop {
        for i in 0..polys.len() {
            for j in i + 1..polys.len() {
                if let Some(m) = try_merge(&polys[i], &polys[j], &pts) {
                    polys[i] = m;
                    polys.remove(j);
                    continue 'again;
                }
            }
        }
        break;
    }
    Ok(polys
        .into_iter()
        .map(|p| {
            let mut v: Vec<Vec2> = p.into_iter().map(|i| pts[i]).collect();
            remove_collinear(&mut v);
            v
        })
        .collect())
}
