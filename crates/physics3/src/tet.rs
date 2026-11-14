//! A tetrahedralizer for closed surface meshes: a regular lattice of cells (six tetrahedra each,
//! with the same diagonals everywhere so neighbouring cells fit) over the cells the mesh fills or
//! touches, and every surface vertex embedded in the tetrahedron that holds it. Deterministic.
use crate::math::*;
use crate::soft::Point;
use std::collections::BTreeMap;

pub struct Tets {
    pub nodes: Vec<V3>,
    pub tets: Vec<[usize; 4]>,
    /// One per surface vertex, in the order given.
    pub points: Vec<Point>,
    /// Which lattice nodes lie inside the mesh (the others are skin the lattice needs to cover it).
    pub inside: Vec<bool>,
}

/// Is `p` inside the closed triangle mesh (ray parity along a fixed oblique direction)?
pub fn point_in_mesh(p: V3, verts: &[V3], tris: &[[usize; 3]]) -> bool {
    let d = v3(0.5772, 0.2137, 0.7891).normalized();
    let mut hits = 0;
    for t in tris {
        let (a, b, c) = (verts[t[0]], verts[t[1]], verts[t[2]]);
        let (e1, e2) = (b - a, c - a);
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-14 {
            continue;
        }
        let f = 1.0 / det;
        let s = p - a;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = f * d.dot(q);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        if f * e2.dot(q) > 0.0 {
            hits += 1;
        }
    }
    hits % 2 == 1
}

fn bary(a: V3, b: V3, c: V3, d: V3, p: V3) -> Option<[f64; 4]> {
    let m = Mat3 {
        m: [
            [b.x - a.x, c.x - a.x, d.x - a.x],
            [b.y - a.y, c.y - a.y, d.y - a.y],
            [b.z - a.z, c.z - a.z, d.z - a.z],
        ],
    };
    if m.det().abs() < 1e-300 {
        return None;
    }
    let l = m.inverse().mul_v(p - a);
    Some([1.0 - l.x - l.y - l.z, l.x, l.y, l.z])
}

/// Fill a closed mesh with tetrahedra. `cells` is the number of lattice cells along its longest side.
pub fn tetrahedralize(verts: &[V3], tris: &[[usize; 3]], cells: usize) -> Option<Tets> {
    if verts.len() < 4 || tris.is_empty() || cells == 0 {
        return None;
    }
    let (mut lo, mut hi) = (verts[0], verts[0]);
    for v in verts {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let ext = hi - lo;
    let longest = ext.x.max(ext.y).max(ext.z);
    if longest <= 0.0 {
        return None;
    }
    let h = longest / cells as f64;
    let origin = lo - v3(h, h, h) * 0.25;
    let dim = |e: f64| ((e + 0.5 * h) / h).ceil() as usize;
    let (nx, ny, nz) = (dim(ext.x), dim(ext.y), dim(ext.z));
    let node_at = |i: usize, j: usize, k: usize| origin + v3(i as f64, j as f64, k as f64) * h;
    let nid = |i: usize, j: usize, k: usize| (k * (ny + 1) + j) * (nx + 1) + i;
    // lattice nodes inside the mesh
    let mut inside_node = vec![false; (nx + 1) * (ny + 1) * (nz + 1)];
    for k in 0..=nz {
        for j in 0..=ny {
            for i in 0..=nx {
                inside_node[nid(i, j, k)] = point_in_mesh(node_at(i, j, k), verts, tris);
            }
        }
    }
    let cell_of = |p: V3| -> (usize, usize, usize) {
        let f = |x: f64, n: usize| ((x / h).floor().max(0.0) as usize).min(n - 1);
        let q = p - origin;
        (f(q.x, nx), f(q.y, ny), f(q.z, nz))
    };
    let mut touched = vec![false; nx * ny * nz];
    for v in verts {
        let (i, j, k) = cell_of(*v);
        touched[(k * ny + j) * nx + i] = true;
    }
    let mut node_map: BTreeMap<usize, usize> = BTreeMap::new();
    let mut node_in: Vec<bool> = vec![];
    let mut nodes: Vec<V3> = vec![];
    let mut tets: Vec<[usize; 4]> = vec![];
    let mut cell_tets: BTreeMap<(usize, usize, usize), Vec<usize>> = BTreeMap::new();
    let perms = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let corners_in = (0..8)
                    .any(|c| inside_node[nid(i + (c & 1), j + ((c >> 1) & 1), k + ((c >> 2) & 1))]);
                if !corners_in && !touched[(k * ny + j) * nx + i] {
                    continue;
                }
                for p in perms {
                    let mut at = [i, j, k];
                    let mut path = [(i, j, k); 4];
                    for (s, ax) in p.iter().enumerate() {
                        at[*ax] += 1;
                        path[s + 1] = (at[0], at[1], at[2]);
                    }
                    let mut ids = [0usize; 4];
                    for (s, (a, b, c)) in path.iter().enumerate() {
                        let key = nid(*a, *b, *c);
                        let next = nodes.len();
                        let id = *node_map.entry(key).or_insert_with(|| {
                            nodes.push(node_at(*a, *b, *c));
                            node_in.push(inside_node[key]);
                            next
                        });
                        ids[s] = id;
                    }
                    let vol = (nodes[ids[1]] - nodes[ids[0]])
                        .cross(nodes[ids[2]] - nodes[ids[0]])
                        .dot(nodes[ids[3]] - nodes[ids[0]]);
                    if vol < 0.0 {
                        ids.swap(1, 2);
                    }
                    cell_tets.entry((i, j, k)).or_default().push(tets.len());
                    tets.push(ids);
                }
            }
        }
    }
    if tets.is_empty() {
        return None;
    }
    // embed every surface vertex in the tetrahedron that holds it (or the nearest one)
    let mut points = vec![];
    for v in verts {
        let (ci, cj, ck) = cell_of(*v);
        let mut best: Option<(f64, [usize; 4], [f64; 4])> = None;
        for dk in 0..3usize {
            for dj in 0..3usize {
                for di in 0..3usize {
                    let (i, j, k) = (
                        (ci + di).wrapping_sub(1),
                        (cj + dj).wrapping_sub(1),
                        (ck + dk).wrapping_sub(1),
                    );
                    let Some(list) = cell_tets.get(&(i, j, k)) else {
                        continue;
                    };
                    for &t in list {
                        let n = tets[t];
                        let Some(w) = bary(nodes[n[0]], nodes[n[1]], nodes[n[2]], nodes[n[3]], *v)
                        else {
                            continue;
                        };
                        let worst = w.iter().cloned().fold(f64::MAX, f64::min);
                        if best.as_ref().is_none_or(|b| worst > b.0) {
                            best = Some((worst, n, w));
                        }
                    }
                }
            }
        }
        let (_, n, w) = best?;
        points.push(Point { nodes: n, w, n: 4 });
    }
    Some(Tets {
        nodes,
        tets,
        points,
        inside: node_in,
    })
}
