//! Convex hulls: an incremental hull builder, polygon faces (coplanar triangles merged), the
//! undirected edges with their two faces, and mass properties. Everything runs in index order, so
//! the same points always give the same hull.
use crate::math::*;

#[derive(Clone, Debug)]
pub struct Face {
    /// Vertex indices, counter-clockwise seen from outside.
    pub verts: Vec<usize>,
    pub normal: V3,
    /// Plane offset: `normal . p = d` for points of the face.
    pub d: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub a: usize,
    pub b: usize,
    /// The face that runs a -> b, and the face that runs b -> a.
    pub f1: usize,
    pub f2: usize,
}

#[derive(Clone, Debug)]
pub struct Hull {
    pub verts: Vec<V3>,
    pub faces: Vec<Face>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Copy, Debug)]
pub struct MassProps {
    pub volume: f64,
    pub centroid: V3,
    /// Inertia tensor about the centroid for unit density.
    pub inertia: Mat3,
}

impl Hull {
    /// The convex hull of `pts`, or `None` when they do not span a volume.
    pub fn from_points(pts: &[V3]) -> Option<Hull> {
        if pts.len() < 4 {
            return None;
        }
        let (mut lo, mut hi) = (pts[0], pts[0]);
        for p in pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let extent = (hi - lo).len();
        if extent <= 0.0 {
            return None;
        }
        let eps = 1e-9 * extent.max(1e-12);
        // initial tetrahedron
        let i0 = (0..pts.len()).fold(0, |b, i| if pts[i].x < pts[b].x { i } else { b });
        let i1 = (0..pts.len()).fold(i0, |b, i| {
            if (pts[i] - pts[i0]).len2() > (pts[b] - pts[i0]).len2() {
                i
            } else {
                b
            }
        });
        if (pts[i1] - pts[i0]).len() < eps {
            return None;
        }
        let dir = (pts[i1] - pts[i0]).normalized();
        let line_dist = |p: V3| ((p - pts[i0]) - dir * (p - pts[i0]).dot(dir)).len();
        let i2 = (0..pts.len()).fold(i0, |b, i| {
            if line_dist(pts[i]) > line_dist(pts[b]) {
                i
            } else {
                b
            }
        });
        if line_dist(pts[i2]) < eps {
            return None;
        }
        let n = (pts[i1] - pts[i0]).cross(pts[i2] - pts[i0]).normalized();
        let plane_dist = |p: V3| (p - pts[i0]).dot(n);
        let i3 = (0..pts.len()).fold(i0, |b, i| {
            if plane_dist(pts[i]).abs() > plane_dist(pts[b]).abs() {
                i
            } else {
                b
            }
        });
        if plane_dist(pts[i3]).abs() < eps {
            return None;
        }
        let mut tris: Vec<[usize; 3]> = vec![];
        let add = |tris: &mut Vec<[usize; 3]>, a: usize, b: usize, c: usize, inside: V3| {
            let nn = (pts[b] - pts[a]).cross(pts[c] - pts[a]);
            if nn.dot(inside - pts[a]) > 0.0 {
                tris.push([a, c, b]);
            } else {
                tris.push([a, b, c]);
            }
        };
        let inside = (pts[i0] + pts[i1] + pts[i2] + pts[i3]) * 0.25;
        add(&mut tris, i0, i1, i2, inside);
        add(&mut tris, i0, i1, i3, inside);
        add(&mut tris, i0, i2, i3, inside);
        add(&mut tris, i1, i2, i3, inside);
        let plane = |t: &[usize; 3]| {
            let nn = (pts[t[1]] - pts[t[0]])
                .cross(pts[t[2]] - pts[t[0]])
                .normalized();
            (nn, nn.dot(pts[t[0]]))
        };
        for (pi, p) in pts.iter().enumerate() {
            if [i0, i1, i2, i3].contains(&pi) {
                continue;
            }
            let visible: Vec<bool> = tris
                .iter()
                .map(|t| {
                    let (nn, d) = plane(t);
                    nn.dot(*p) - d > eps
                })
                .collect();
            if !visible.iter().any(|v| *v) {
                continue;
            }
            let mut directed: Vec<(usize, usize)> = vec![];
            for (t, vis) in tris.iter().zip(&visible) {
                if *vis {
                    directed.push((t[0], t[1]));
                    directed.push((t[1], t[2]));
                    directed.push((t[2], t[0]));
                }
            }
            let horizon: Vec<(usize, usize)> = directed
                .iter()
                .filter(|(a, b)| !directed.contains(&(*b, *a)))
                .cloned()
                .collect();
            let mut kept: Vec<[usize; 3]> = tris
                .iter()
                .zip(&visible)
                .filter(|(_, v)| !**v)
                .map(|(t, _)| *t)
                .collect();
            for (a, b) in horizon {
                kept.push([a, b, pi]);
            }
            tris = kept;
        }
        // merge coplanar triangles into polygon faces
        let planes: Vec<(V3, f64)> = tris.iter().map(&plane).collect();
        let mut group: Vec<usize> = (0..tris.len()).collect();
        fn find(g: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while g[r] != r {
                r = g[r];
            }
            let mut c = i;
            while g[c] != r {
                let nx = g[c];
                g[c] = r;
                c = nx;
            }
            r
        }
        let edge_set = |t: &[usize; 3]| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])];
        for i in 0..tris.len() {
            for j in i + 1..tris.len() {
                let coplanar = planes[i].0.dot(planes[j].0) > 1.0 - 1e-9
                    && (planes[i].1 - planes[j].1).abs() < 1e-7 * extent.max(1.0);
                if !coplanar {
                    continue;
                }
                let shares = edge_set(&tris[i])
                    .iter()
                    .any(|(a, b)| edge_set(&tris[j]).contains(&(*b, *a)));
                if shares {
                    let (ri, rj) = (find(&mut group, i), find(&mut group, j));
                    if ri != rj {
                        group[rj.max(ri)] = rj.min(ri);
                    }
                }
            }
        }
        let mut roots: Vec<usize> = vec![];
        for i in 0..tris.len() {
            let r = find(&mut group, i);
            if !roots.contains(&r) {
                roots.push(r);
            }
        }
        let mut faces: Vec<Face> = vec![];
        for r in roots {
            let members: Vec<usize> = (0..tris.len())
                .filter(|i| find(&mut group, *i) == r)
                .collect();
            let all: Vec<(usize, usize)> =
                members.iter().flat_map(|i| edge_set(&tris[*i])).collect();
            let boundary: Vec<(usize, usize)> = all
                .iter()
                .filter(|(a, b)| !all.contains(&(*b, *a)))
                .cloned()
                .collect();
            // chain the boundary edges into one loop
            let mut ring = vec![boundary[0].0];
            let mut cur = boundary[0].1;
            let mut guard = 0;
            while cur != ring[0] && guard < boundary.len() + 2 {
                ring.push(cur);
                cur = boundary
                    .iter()
                    .find(|(a, _)| *a == cur)
                    .map(|e| e.1)
                    .unwrap_or(ring[0]);
                guard += 1;
            }
            let mut nn = V3::ZERO;
            for i in &members {
                let t = &tris[*i];
                nn += (pts[t[1]] - pts[t[0]]).cross(pts[t[2]] - pts[t[0]]);
            }
            let nn = nn.normalized();
            let d = ring.iter().map(|i| nn.dot(pts[*i])).sum::<f64>() / ring.len() as f64;
            faces.push(Face {
                verts: ring,
                normal: nn,
                d,
            });
        }
        // keep only the vertices that are used
        let mut remap = vec![usize::MAX; pts.len()];
        let mut verts = vec![];
        for f in &faces {
            for &i in &f.verts {
                if remap[i] == usize::MAX {
                    remap[i] = verts.len();
                    verts.push(pts[i]);
                }
            }
        }
        for f in &mut faces {
            for i in &mut f.verts {
                *i = remap[*i];
            }
        }
        // undirected edges with both faces
        let mut edges: Vec<Edge> = vec![];
        for (fi, f) in faces.iter().enumerate() {
            let k = f.verts.len();
            for e in 0..k {
                let (a, b) = (f.verts[e], f.verts[(e + 1) % k]);
                if a < b {
                    edges.push(Edge {
                        a,
                        b,
                        f1: fi,
                        f2: usize::MAX,
                    });
                }
            }
        }
        for (fi, f) in faces.iter().enumerate() {
            let k = f.verts.len();
            for e in 0..k {
                let (a, b) = (f.verts[e], f.verts[(e + 1) % k]);
                if a > b {
                    if let Some(ed) = edges.iter_mut().find(|x| x.a == b && x.b == a) {
                        ed.f2 = fi;
                    }
                }
            }
        }
        if edges.iter().any(|e| e.f2 == usize::MAX) || verts.len() < 4 {
            return None;
        }
        Some(Hull {
            verts,
            faces,
            edges,
        })
    }

    pub fn mass_props(&self) -> MassProps {
        let r0 = self.verts.iter().fold(V3::ZERO, |a, v| a + *v) / self.verts.len() as f64;
        let mut vol = 0.0;
        let mut first = V3::ZERO;
        let mut cov = Mat3::ZERO;
        for f in &self.faces {
            for k in 1..f.verts.len() - 1 {
                let a = self.verts[f.verts[0]] - r0;
                let b = self.verts[f.verts[k]] - r0;
                let c = self.verts[f.verts[k + 1]] - r0;
                let det = a.dot(b.cross(c));
                vol += det / 6.0;
                first += (a + b + c) * (det / 24.0);
                let s = a + b + c;
                let m = Mat3::outer(a, a)
                    .add(&Mat3::outer(b, b))
                    .add(&Mat3::outer(c, c))
                    .add(&Mat3::outer(s, s));
                cov = cov.add(&m.scale(det / 120.0));
            }
        }
        let com = first / vol;
        let cov = cov.sub(&Mat3::outer(com, com).scale(vol));
        let inertia = Mat3::diag(cov.trace(), cov.trace(), cov.trace()).sub(&cov);
        MassProps {
            volume: vol,
            centroid: r0 + com,
            inertia,
        }
    }

    pub fn translated(&self, by: V3) -> Hull {
        let mut h = self.clone();
        for v in &mut h.verts {
            *v += by;
        }
        for f in &mut h.faces {
            f.d = f.normal.dot(h.verts[f.verts[0]]);
        }
        h
    }

    /// Furthest vertex along `dir`.
    pub fn support(&self, dir: V3) -> V3 {
        let mut best = self.verts[0];
        let mut bd = best.dot(dir);
        for v in &self.verts[1..] {
            let d = v.dot(dir);
            if d > bd {
                bd = d;
                best = *v;
            }
        }
        best
    }

    /// A hull made of one triangle (two opposite faces): the building block of a triangle mesh.
    pub fn triangle(a: V3, b: V3, c: V3) -> Option<Hull> {
        let n = (b - a).cross(c - a).normalized();
        if n == V3::ZERO {
            return None;
        }
        let d = n.dot(a);
        Some(Hull {
            verts: vec![a, b, c],
            faces: vec![
                Face {
                    verts: vec![0, 1, 2],
                    normal: n,
                    d,
                },
                Face {
                    verts: vec![0, 2, 1],
                    normal: -n,
                    d: -d,
                },
            ],
            edges: vec![
                Edge {
                    a: 0,
                    b: 1,
                    f1: 0,
                    f2: 1,
                },
                Edge {
                    a: 1,
                    b: 2,
                    f1: 0,
                    f2: 1,
                },
                Edge {
                    a: 0,
                    b: 2,
                    f1: 1,
                    f2: 0,
                },
            ],
        })
    }
}

impl Hull {
    /// The hull rotated by `q` and moved by `p`.
    pub fn transformed(&self, p: V3, q: Quat) -> Hull {
        Hull {
            verts: self.verts.iter().map(|v| q.rotate(*v) + p).collect(),
            faces: self
                .faces
                .iter()
                .map(|f| {
                    let n = q.rotate(f.normal);
                    Face {
                        verts: f.verts.clone(),
                        normal: n,
                        d: f.d + n.dot(p),
                    }
                })
                .collect(),
            edges: self.edges.clone(),
        }
    }
    pub fn aabb(&self) -> (V3, V3) {
        let mut lo = self.verts[0];
        let mut hi = lo;
        for v in &self.verts {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        (lo, hi)
    }
    pub fn centre(&self) -> V3 {
        self.verts.iter().fold(V3::ZERO, |a, v| a + *v) / self.verts.len() as f64
    }
}
