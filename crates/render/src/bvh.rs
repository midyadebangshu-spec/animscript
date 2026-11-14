//! A bounding volume hierarchy over triangles for shadow and occlusion rays. The build sorts by
//! centroid with the triangle index as tie-break and splits at the median, so the same triangles
//! always give the same tree, and traversal order is fixed.
pub type V3 = [f64; 3];

#[derive(Clone, Copy)]
struct Node {
    lo: V3,
    hi: V3,
    /// Inner node: index of the right child (the left one follows the node). Leaf: first triangle.
    a: u32,
    /// 0 for an inner node, else the number of triangles of the leaf.
    n: u32,
}

pub struct Bvh {
    tris: Vec<[V3; 3]>,
    nodes: Vec<Node>,
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

const LEAF: usize = 4;

impl Bvh {
    pub fn new(tris: Vec<[V3; 3]>) -> Bvh {
        let mut order: Vec<usize> = (0..tris.len()).collect();
        let mut nodes = vec![];
        if !tris.is_empty() {
            let cent: Vec<V3> = tris
                .iter()
                .map(|t| {
                    [
                        (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                        (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                        (t[0][2] + t[1][2] + t[2][2]) / 3.0,
                    ]
                })
                .collect();
            Self::build(&tris, &cent, &mut order, 0, tris.len(), &mut nodes);
        }
        let tris = order.iter().map(|&i| tris[i]).collect();
        Bvh { tris, nodes }
    }

    fn build(
        tris: &[[V3; 3]],
        cent: &[V3],
        order: &mut [usize],
        from: usize,
        to: usize,
        nodes: &mut Vec<Node>,
    ) -> usize {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for &i in &order[from..to] {
            for v in &tris[i] {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
        }
        let me = nodes.len();
        nodes.push(Node {
            lo,
            hi,
            a: from as u32,
            n: (to - from) as u32,
        });
        if to - from <= LEAF {
            return me;
        }
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] {
            0
        } else if ext[1] >= ext[2] {
            1
        } else {
            2
        };
        order[from..to].sort_by(|&p, &q| cent[p][axis].total_cmp(&cent[q][axis]).then(p.cmp(&q)));
        let mid = (from + to) / 2;
        let left = Self::build(tris, cent, order, from, mid, nodes);
        let right = Self::build(tris, cent, order, mid, to, nodes);
        debug_assert_eq!(left, me + 1);
        nodes[me].a = right as u32;
        nodes[me].n = 0;
        me
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    fn hit_box(n: &Node, o: V3, inv: V3, tmax: f64) -> bool {
        let (mut t0, mut t1) = (0.0f64, tmax);
        for k in 0..3 {
            let a = (n.lo[k] - o[k]) * inv[k];
            let b = (n.hi[k] - o[k]) * inv[k];
            let (a, b) = if a < b { (a, b) } else { (b, a) };
            // NaN (0 * inf) compares false and leaves the interval unchanged
            if a > t0 {
                t0 = a;
            }
            if b < t1 {
                t1 = b;
            }
            if t0 > t1 {
                return false;
            }
        }
        true
    }

    fn hit_tri(t: &[V3; 3], o: V3, d: V3, tmax: f64) -> bool {
        let e1 = sub(t[1], t[0]);
        let e2 = sub(t[2], t[0]);
        let p = cross(d, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-12 {
            return false;
        }
        let inv = 1.0 / det;
        let s = sub(o, t[0]);
        let u = dot(s, p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = cross(s, e1);
        let v = dot(d, q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let dist = dot(e2, q) * inv;
        dist > 1e-6 && dist < tmax
    }

    /// Is anything between `o` and `o + d * tmax` (`d` a unit vector; `tmax` may be infinite)?
    pub fn any_hit(&self, o: V3, d: V3, tmax: f64) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let inv = [1.0 / d[0], 1.0 / d[1], 1.0 / d[2]];
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let idx = stack[sp] as usize;
            let n = &self.nodes[idx];
            if !Self::hit_box(n, o, inv, tmax) {
                continue;
            }
            if n.n > 0 {
                for t in &self.tris[n.a as usize..(n.a + n.n) as usize] {
                    if Self::hit_tri(t, o, d, tmax) {
                        return true;
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = n.a; // the right child is pushed first, so the left is looked at first
                stack[sp + 1] = idx as u32 + 1;
                sp += 2;
            }
        }
        false
    }

    /// Brute force, for tests.
    pub fn any_hit_brute(&self, o: V3, d: V3, tmax: f64) -> bool {
        self.tris.iter().any(|t| Self::hit_tri(t, o, d, tmax))
    }
}
