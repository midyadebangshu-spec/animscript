//! 3D meshes: a list of vertices and polygon faces, plus the builders (primitives, `extrude`, `lathe`).
//!
//! Axes are the screen's: x to the right, y **down**, z away from the viewer (a right-handed system), so a mesh
//! placed at `(400, 300, 0)` shows up at `(400, 300)` on the canvas. A face's normal is `(v1 - v0) x (v2 - v0)` and
//! points out of the solid. Faces are convex (the builders triangulate anything concave).
use crate::geometry::P;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub verts: Vec<[f64; 3]>,
    pub faces: Vec<Vec<u32>>,
    /// Drawn from both sides (planes); closed solids are culled from behind.
    pub double_sided: bool,
}

/// Scale about the pivot, rotate (in turns: about y first, the spin, then x, the tilt, then z, the roll), then
/// translate by `position`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform3 {
    pub position: [f64; 3],
    pub rot: [f64; 3],
    pub scale: [f64; 3],
    pub pivot: [f64; 3],
    /// A rotation as a quaternion `[w, x, y, z]` that replaces `rot` (physics bodies keep one).
    pub orient: Option<[f64; 4]>,
}

impl Transform3 {
    pub fn identity() -> Transform3 {
        Transform3 {
            position: [0.0; 3],
            rot: [0.0; 3],
            scale: [1.0; 3],
            pivot: [0.0; 3],
            orient: None,
        }
    }
    pub fn apply(&self, p: [f64; 3]) -> [f64; 3] {
        let mut v = [
            (p[0] - self.pivot[0]) * self.scale[0],
            (p[1] - self.pivot[1]) * self.scale[1],
            (p[2] - self.pivot[2]) * self.scale[2],
        ];
        if let Some([w, x, y, z]) = self.orient {
            let t = [
                2.0 * (y * v[2] - z * v[1]),
                2.0 * (z * v[0] - x * v[2]),
                2.0 * (x * v[1] - y * v[0]),
            ];
            v = [
                v[0] + w * t[0] + (y * t[2] - z * t[1]),
                v[1] + w * t[1] + (z * t[0] - x * t[2]),
                v[2] + w * t[2] + (x * t[1] - y * t[0]),
            ];
        } else {
            let tau = std::f64::consts::TAU;
            let (s, c) = (self.rot[1] * tau).sin_cos();
            v = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
            let (s, c) = (self.rot[0] * tau).sin_cos();
            v = [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
            let (s, c) = (self.rot[2] * tau).sin_cos();
            v = [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
        }
        [
            v[0] + self.pivot[0] + self.position[0],
            v[1] + self.pivot[1] + self.position[1],
            v[2] + self.pivot[2] + self.position[2],
        ]
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Mesh {
    pub fn bbox(&self) -> ([f64; 3], [f64; 3]) {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in &self.verts {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        (lo, hi)
    }

    /// The unit normal of a face (Newell's method; zero for a degenerate face).
    pub fn face_normal(&self, f: usize) -> [f64; 3] {
        let fa = &self.faces[f];
        let mut n = [0.0; 3];
        for k in 0..fa.len() {
            let (p, q) = (
                self.verts[fa[k] as usize],
                self.verts[fa[(k + 1) % fa.len()] as usize],
            );
            n[0] += (p[1] - q[1]) * (p[2] + q[2]);
            n[1] += (p[2] - q[2]) * (p[0] + q[0]);
            n[2] += (p[0] - q[0]) * (p[1] + q[1]);
        }
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l == 0.0 {
            [0.0; 3]
        } else {
            [n[0] / l, n[1] / l, n[2] / l]
        }
    }

    pub fn transformed(&self, t: &Transform3) -> Mesh {
        Mesh {
            verts: self.verts.iter().map(|v| t.apply(*v)).collect(),
            faces: self.faces.clone(),
            double_sided: self.double_sided,
        }
    }

    fn flip(&mut self) {
        for f in &mut self.faces {
            f.reverse();
        }
    }

    /// Make every face of a convex solid centred on the origin point outward.
    fn orient_out(&mut self) {
        for f in 0..self.faces.len() {
            let n = self.face_normal(f);
            let fa = &self.faces[f];
            let mut c = [0.0; 3];
            for &i in fa {
                for (k, ck) in c.iter_mut().enumerate() {
                    *ck += self.verts[i as usize][k] / fa.len() as f64;
                }
            }
            if n[0] * c[0] + n[1] * c[1] + n[2] * c[2] < 0.0 {
                self.faces[f].reverse();
            }
        }
    }

    fn signed_volume(&self) -> f64 {
        let mut vol = 0.0;
        for fa in &self.faces {
            let v = |i: usize| self.verts[fa[i] as usize];
            for k in 1..fa.len() - 1 {
                let c = cross(v(k), v(k + 1));
                let p = v(0);
                vol += (p[0] * c[0] + p[1] * c[1] + p[2] * c[2]) / 6.0;
            }
        }
        vol
    }

    pub fn cube(sx: f64, sy: f64, sz: f64) -> Mesh {
        let (x, y, z) = (sx / 2.0, sy / 2.0, sz / 2.0);
        let verts = vec![
            [-x, -y, -z],
            [x, -y, -z],
            [x, y, -z],
            [-x, y, -z],
            [-x, -y, z],
            [x, -y, z],
            [x, y, z],
            [-x, y, z],
        ];
        let faces = vec![
            vec![0, 1, 2, 3],
            vec![5, 4, 7, 6],
            vec![4, 0, 3, 7],
            vec![1, 5, 6, 2],
            vec![4, 5, 1, 0],
            vec![3, 2, 6, 7],
        ];
        let mut m = Mesh {
            verts,
            faces,
            double_sided: false,
        };
        m.orient_out();
        m
    }

    /// Every quad face cut into `n` x `n` quads (other faces are kept). Each face gets its own
    /// vertices, so the creases of a cube stay sharp; soft bodies weld them again.
    pub fn subdivided(&self, n: usize) -> Mesh {
        let n = n.max(1);
        if n == 1 {
            return self.clone();
        }
        let mut verts: Vec<[f64; 3]> = vec![];
        let mut faces: Vec<Vec<u32>> = vec![];
        for f in &self.faces {
            if f.len() != 4 || n == 1 {
                let base = verts.len() as u32;
                for i in f {
                    verts.push(self.verts[*i as usize]);
                }
                faces.push((0..f.len() as u32).map(|k| base + k).collect());
                continue;
            }
            let p: Vec<[f64; 3]> = f.iter().map(|i| self.verts[*i as usize]).collect();
            let base = verts.len() as u32;
            for j in 0..=n {
                for i in 0..=n {
                    let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                    let mut q = [0.0; 3];
                    for (k, qk) in q.iter_mut().enumerate() {
                        let a = p[0][k] + (p[1][k] - p[0][k]) * u;
                        let b = p[3][k] + (p[2][k] - p[3][k]) * u;
                        *qk = a + (b - a) * v;
                    }
                    verts.push(q);
                }
            }
            let at = |i: usize, j: usize| base + (j * (n + 1) + i) as u32;
            for j in 0..n {
                for i in 0..n {
                    faces.push(vec![at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
                }
            }
        }
        Mesh {
            verts,
            faces,
            double_sided: self.double_sided,
        }
    }

    /// A surface of revolution about the y axis from a profile of `(radius, height)` points (height up the screen).
    pub fn lathe(profile: &[P], segments: usize) -> Result<Mesh, String> {
        if profile.len() < 2 {
            return Err("a lathe needs a profile of at least two points".into());
        }
        if segments < 3 {
            return Err("a lathe needs at least 3 segments".into());
        }
        if profile
            .iter()
            .any(|p| p[0] < 0.0 || !p[0].is_finite() || !p[1].is_finite())
        {
            return Err(
                "a lathe profile has radius >= 0 and finite numbers: points are (radius, height)"
                    .into(),
            );
        }
        let mut verts: Vec<[f64; 3]> = vec![];
        // index of (profile point, segment); points on the axis share one vertex
        let mut idx: Vec<Vec<u32>> = vec![];
        for p in profile {
            let mut row = vec![];
            if p[0] == 0.0 {
                verts.push([0.0, -p[1], 0.0]);
                row = vec![verts.len() as u32 - 1; segments];
            } else {
                for k in 0..segments {
                    let a = std::f64::consts::TAU * k as f64 / segments as f64;
                    verts.push([p[0] * a.cos(), -p[1], p[0] * a.sin()]);
                    row.push(verts.len() as u32 - 1);
                }
            }
            idx.push(row);
        }
        let mut faces = vec![];
        for i in 0..profile.len() - 1 {
            for k in 0..segments {
                let k2 = (k + 1) % segments;
                let mut q = vec![idx[i][k], idx[i][k2], idx[i + 1][k2], idx[i + 1][k]];
                q.dedup();
                while q.len() > 1 && q.first() == q.last() {
                    q.pop();
                }
                if q.len() >= 3 {
                    faces.push(q);
                }
            }
        }
        let mut m = Mesh {
            verts,
            faces,
            double_sided: false,
        };
        if m.signed_volume() < 0.0 {
            m.flip();
        }
        Ok(m)
    }

    pub fn sphere(radius: f64, segments: usize, rings: usize) -> Mesh {
        let rings = rings.max(2);
        let profile: Vec<P> = (0..=rings)
            .map(|i| {
                let a = std::f64::consts::PI * i as f64 / rings as f64;
                [
                    if i == 0 || i == rings {
                        0.0
                    } else {
                        radius * a.sin()
                    },
                    radius * a.cos(),
                ]
            })
            .collect();
        Mesh::lathe(&profile, segments.max(3)).expect("a valid profile")
    }

    pub fn cylinder(radius: f64, height: f64, segments: usize) -> Mesh {
        let h = height / 2.0;
        Mesh::lathe(
            &[[0.0, -h], [radius, -h], [radius, h], [0.0, h]],
            segments.max(3),
        )
        .expect("a valid profile")
    }

    pub fn cone(radius: f64, height: f64, segments: usize) -> Mesh {
        let h = height / 2.0;
        Mesh::lathe(&[[0.0, -h], [radius, -h], [0.0, h]], segments.max(3)).expect("a valid profile")
    }

    /// A flat grid in the x-z plane (the floor), facing up the screen, drawn from both sides.
    pub fn plane(width: f64, depth: f64, nx: usize, nz: usize) -> Mesh {
        let (nx, nz) = (nx.max(1), nz.max(1));
        let mut verts = vec![];
        for j in 0..=nz {
            for i in 0..=nx {
                verts.push([
                    (i as f64 / nx as f64 - 0.5) * width,
                    0.0,
                    (j as f64 / nz as f64 - 0.5) * depth,
                ]);
            }
        }
        let at = |i: usize, j: usize| (j * (nx + 1) + i) as u32;
        let mut faces = vec![];
        for j in 0..nz {
            for i in 0..nx {
                faces.push(vec![at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
            }
        }
        Mesh {
            verts,
            faces,
            double_sided: true,
        }
    }

    /// A polygon (in the x-y plane) pushed `depth` along z, centred on z = 0. The polygon may be concave and either
    /// way round; it must not cross itself.
    pub fn extrude(poly: &[P], depth: f64) -> Result<Mesh, String> {
        if poly.len() < 3 {
            return Err("an extrusion needs a polygon of at least three points".into());
        }
        if depth <= 0.0 || !depth.is_finite() {
            return Err("`depth` must be positive".into());
        }
        let tris = triangulate(poly)?;
        let n = poly.len();
        let (front, back) = (-depth / 2.0, depth / 2.0);
        let mut verts: Vec<[f64; 3]> = poly.iter().map(|p| [p[0], p[1], front]).collect();
        verts.extend(poly.iter().map(|p| [p[0], p[1], back]));
        let mut faces = vec![];
        let area: f64 = (0..n)
            .map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1])
            .sum::<f64>();
        for t in &tris {
            let (p, q, r) = (poly[t[0]], poly[t[1]], poly[t[2]]);
            let z = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
            // the front cap faces -z, the back cap +z
            let (a, b, c) = (t[0] as u32, t[1] as u32, t[2] as u32);
            faces.push(if z > 0.0 {
                vec![a, c, b]
            } else {
                vec![a, b, c]
            });
            let (a, b, c) = (a + n as u32, b + n as u32, c + n as u32);
            faces.push(if z > 0.0 {
                vec![a, b, c]
            } else {
                vec![a, c, b]
            });
        }
        for i in 0..n {
            let j = (i + 1) % n;
            let q = vec![i as u32, j as u32, (j + n) as u32, (i + n) as u32];
            faces.push(if area > 0.0 {
                q
            } else {
                q.into_iter().rev().collect()
            });
        }
        Ok(Mesh {
            verts,
            faces,
            double_sided: false,
        })
    }
}

fn segments_cross(a: P, b: P, c: P, d: P) -> bool {
    let o = |p: P, q: P, r: P| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (o(a, b, c), o(a, b, d), o(c, d, a), o(c, d, b));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Triangles (indices into `poly`) that exactly cover a simple polygon, by ear clipping. Fails if the polygon crosses itself.
pub fn triangulate(poly: &[P]) -> Result<Vec<[usize; 3]>, String> {
    let n = poly.len();
    if n < 3 {
        return Err("a polygon needs at least three points".into());
    }
    for i in 0..n {
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            if segments_cross(poly[i], poly[(i + 1) % n], poly[j], poly[(j + 1) % n]) {
                return Err("the polygon crosses itself".into());
            }
        }
    }
    let area: f64 = (0..n)
        .map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1])
        .sum();
    let mut idx: Vec<usize> = if area > 0.0 {
        (0..n).collect()
    } else {
        (0..n).rev().collect()
    };
    let cross2 = |a: P, b: P, c: P| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let inside = |p: P, a: P, b: P, c: P| {
        cross2(a, b, p) >= 0.0 && cross2(b, c, p) >= 0.0 && cross2(c, a, p) >= 0.0
    };
    let mut out = vec![];
    let mut guard = 0;
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (poly[ia], poly[ib], poly[ic]);
            let turn = cross2(a, b, c);
            if turn < 0.0 {
                continue; // a reflex corner is not an ear
            }
            let blocked = idx.iter().any(|&j| {
                j != ia
                    && j != ib
                    && j != ic
                    && inside(poly[j], a, b, c)
                    && poly[j] != a
                    && poly[j] != b
                    && poly[j] != c
            });
            if blocked {
                continue;
            }
            if turn > 0.0 {
                out.push([ia, ib, ic]);
            }
            idx.remove(k);
            clipped = true;
            break;
        }
        guard += 1;
        if !clipped || guard > 4 * n * n {
            return Err("the polygon could not be triangulated (is it simple?)".into());
        }
    }
    if cross2(poly[idx[0]], poly[idx[1]], poly[idx[2]]) > 0.0 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    Ok(out)
}
