//! The sparse signed-distance volume and its surface extraction.
use crate::{dot, norm, V3};
use std::collections::BTreeMap;

pub const B: i32 = 8;
const N: usize = (B * B * B) as usize;

#[derive(Clone, Debug)]
enum Block {
    /// Every voxel of the block has this value (far outside, or deep inside).
    Uniform(f32),
    Dense(Box<[f32; N]>),
}

/// A triangle mesh: vertices in world units and triangles (counter-clockwise seen from outside, so that
/// `(b - a) x (c - a)` points out of the solid).
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub verts: Vec<V3>,
    pub tris: Vec<[u32; 3]>,
}

#[derive(Clone, Debug)]
pub struct Volume {
    /// Voxel size in world units. Voxel `(i, j, k)` sits at `(i, j, k) * h`.
    pub h: f64,
    /// The band: stored distances are clamped to `-band..band` (world units).
    pub band: f64,
    blocks: BTreeMap<[i32; 3], Block>,
}

fn split(i: [i32; 3]) -> ([i32; 3], usize) {
    let key = [i[0].div_euclid(B), i[1].div_euclid(B), i[2].div_euclid(B)];
    let l = [i[0].rem_euclid(B), i[1].rem_euclid(B), i[2].rem_euclid(B)];
    (key, ((l[2] * B + l[1]) * B + l[0]) as usize)
}

impl Volume {
    /// An empty volume (everything outside). The band is `band_voxels` voxels wide on each side of the surface.
    pub fn new(h: f64, band_voxels: f64) -> Volume {
        Volume {
            h,
            band: h * band_voxels.max(1.5),
            blocks: BTreeMap::new(),
        }
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }
    pub fn dense_block_count(&self) -> usize {
        self.blocks
            .values()
            .filter(|b| matches!(b, Block::Dense(_)))
            .count()
    }

    pub fn get(&self, i: [i32; 3]) -> f32 {
        let (key, o) = split(i);
        match self.blocks.get(&key) {
            None => self.band as f32,
            Some(Block::Uniform(v)) => *v,
            Some(Block::Dense(d)) => d[o],
        }
    }

    pub fn set(&mut self, i: [i32; 3], v: f64) {
        let (key, o) = split(i);
        let v = v.clamp(-self.band, self.band) as f32;
        let band = self.band as f32;
        let b = self.blocks.entry(key).or_insert(Block::Uniform(band));
        match b {
            Block::Uniform(u) if *u == v => {}
            Block::Uniform(u) => {
                let mut d = Box::new([*u; N]);
                d[o] = v;
                *b = Block::Dense(d);
            }
            Block::Dense(d) => d[o] = v,
        }
    }

    /// Voxel indices covering the world box `lo..hi`.
    pub fn index_box(&self, lo: V3, hi: V3) -> ([i32; 3], [i32; 3]) {
        let f = |v: f64| (v / self.h).floor() as i32;
        let c = |v: f64| (v / self.h).ceil() as i32;
        (
            [f(lo[0]), f(lo[1]), f(lo[2])],
            [c(hi[0]), c(hi[1]), c(hi[2])],
        )
    }

    /// Rewrite every voxel in the index box `lo..=hi` as `f(position, old)`; blocks that end up uniform collapse.
    pub fn edit(&mut self, lo: [i32; 3], hi: [i32; 3], f: &mut dyn FnMut(V3, f64) -> f64) {
        let band = self.band;
        let h = self.h;
        let (bl, bh) = (
            [
                lo[0].div_euclid(B),
                lo[1].div_euclid(B),
                lo[2].div_euclid(B),
            ],
            [
                hi[0].div_euclid(B),
                hi[1].div_euclid(B),
                hi[2].div_euclid(B),
            ],
        );
        for bz in bl[2]..=bh[2] {
            for by in bl[1]..=bh[1] {
                for bx in bl[0]..=bh[0] {
                    let key = [bx, by, bz];
                    let mut vals: Vec<f32> = match self.blocks.get(&key) {
                        None => vec![band as f32; N],
                        Some(Block::Uniform(v)) => vec![*v; N],
                        Some(Block::Dense(d)) => d.to_vec(),
                    };
                    for lz in 0..B {
                        for ly in 0..B {
                            for lx in 0..B {
                                let i = [bx * B + lx, by * B + ly, bz * B + lz];
                                if (0..3).any(|a| i[a] < lo[a] || i[a] > hi[a]) {
                                    continue;
                                }
                                let o = ((lz * B + ly) * B + lx) as usize;
                                let p = [i[0] as f64 * h, i[1] as f64 * h, i[2] as f64 * h];
                                vals[o] = f(p, vals[o] as f64).clamp(-band, band) as f32;
                            }
                        }
                    }
                    let first = vals[0];
                    if vals.iter().all(|v| *v == first) {
                        if first == band as f32 {
                            self.blocks.remove(&key);
                        } else {
                            self.blocks.insert(key, Block::Uniform(first));
                        }
                    } else {
                        let mut d = Box::new([0.0f32; N]);
                        d.copy_from_slice(&vals);
                        self.blocks.insert(key, Block::Dense(d));
                    }
                }
            }
        }
    }

    /// Trilinear sample of the field at a world position.
    pub fn sample(&self, p: V3) -> f64 {
        let q = [p[0] / self.h, p[1] / self.h, p[2] / self.h];
        let i0 = [
            q[0].floor() as i32,
            q[1].floor() as i32,
            q[2].floor() as i32,
        ];
        let f = [
            q[0] - i0[0] as f64,
            q[1] - i0[1] as f64,
            q[2] - i0[2] as f64,
        ];
        let mut acc = 0.0;
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let w = (if dx == 1 { f[0] } else { 1.0 - f[0] })
                        * (if dy == 1 { f[1] } else { 1.0 - f[1] })
                        * (if dz == 1 { f[2] } else { 1.0 - f[2] });
                    if w != 0.0 {
                        acc += w * self.get([i0[0] + dx, i0[1] + dy, i0[2] + dz]) as f64;
                    }
                }
            }
        }
        acc
    }

    /// The gradient of the field (central differences over one voxel): a vector that points out of the solid.
    pub fn gradient(&self, p: V3) -> V3 {
        let e = self.h;
        [
            (self.sample([p[0] + e, p[1], p[2]]) - self.sample([p[0] - e, p[1], p[2]])) / (2.0 * e),
            (self.sample([p[0], p[1] + e, p[2]]) - self.sample([p[0], p[1] - e, p[2]])) / (2.0 * e),
            (self.sample([p[0], p[1], p[2] + e]) - self.sample([p[0], p[1], p[2] - e])) / (2.0 * e),
        ]
    }

    /// The index box of the dense blocks (where the surface is), or `None` if there are none.
    pub fn active_box(&self) -> Option<([i32; 3], [i32; 3])> {
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        let mut any = false;
        for (k, b) in &self.blocks {
            if matches!(b, Block::Dense(_)) {
                any = true;
                for a in 0..3 {
                    lo[a] = lo[a].min(k[a] * B);
                    hi[a] = hi[a].max(k[a] * B + B - 1);
                }
            }
        }
        any.then_some((lo, hi))
    }

    /// The solid's volume in world units cubed (each voxel counts as the share of it that is inside, from its distance).
    pub fn volume(&self) -> f64 {
        let mut total = 0.0;
        for (k, b) in &self.blocks {
            match b {
                Block::Uniform(v) => {
                    if *v < 0.0 {
                        total += N as f64;
                    }
                }
                Block::Dense(d) => {
                    for v in d.iter() {
                        total += (0.5 - *v as f64 / self.h).clamp(0.0, 1.0);
                    }
                }
            }
            let _ = k;
        }
        total * self.h * self.h * self.h
    }

    /// Copy of the blocks that touch the index box (for operations that read old values while writing new ones).
    pub fn snapshot(&self, lo: [i32; 3], hi: [i32; 3]) -> Volume {
        let mut out = Volume {
            h: self.h,
            band: self.band,
            blocks: BTreeMap::new(),
        };
        let (bl, bh) = (
            [
                lo[0].div_euclid(B) - 1,
                lo[1].div_euclid(B) - 1,
                lo[2].div_euclid(B) - 1,
            ],
            [
                hi[0].div_euclid(B) + 1,
                hi[1].div_euclid(B) + 1,
                hi[2].div_euclid(B) + 1,
            ],
        );
        for (k, b) in &self.blocks {
            if (0..3).all(|a| k[a] >= bl[a] && k[a] <= bh[a]) {
                out.blocks.insert(*k, b.clone());
            }
        }
        out
    }

    /// Mesh of the zero level set (surface nets): one vertex per cell the surface crosses, at the mean of its edge
    /// crossings, and a quad for every grid edge the surface crosses.
    pub fn mesh(&self) -> Mesh {
        let Some((lo, hi)) = self.active_box() else {
            return Mesh::default();
        };
        let lo = [lo[0] - 1, lo[1] - 1, lo[2] - 1];
        let hi = [hi[0] + 2, hi[1] + 2, hi[2] + 2];
        let dims = [
            (hi[0] - lo[0] + 1) as usize,
            (hi[1] - lo[1] + 1) as usize,
            (hi[2] - lo[2] + 1) as usize,
        ];
        // the samples of the box in one array
        let mut g = vec![self.band as f32; dims[0] * dims[1] * dims[2]];
        let idx = |x: usize, y: usize, z: usize| (z * dims[1] + y) * dims[0] + x;
        for (k, b) in &self.blocks {
            let base = [k[0] * B, k[1] * B, k[2] * B];
            if (0..3).any(|a| base[a] + B - 1 < lo[a] || base[a] > hi[a]) {
                continue;
            }
            for lz in 0..B {
                for ly in 0..B {
                    for lx in 0..B {
                        let i = [base[0] + lx, base[1] + ly, base[2] + lz];
                        if (0..3).any(|a| i[a] < lo[a] || i[a] > hi[a]) {
                            continue;
                        }
                        let v = match b {
                            Block::Uniform(v) => *v,
                            Block::Dense(d) => d[((lz * B + ly) * B + lx) as usize],
                        };
                        g[idx(
                            (i[0] - lo[0]) as usize,
                            (i[1] - lo[1]) as usize,
                            (i[2] - lo[2]) as usize,
                        )] = v;
                    }
                }
            }
        }
        let cell_dims = [dims[0] - 1, dims[1] - 1, dims[2] - 1];
        let cidx = |x: usize, y: usize, z: usize| (z * cell_dims[1] + y) * cell_dims[0] + x;
        let mut cell_vert = vec![u32::MAX; cell_dims[0] * cell_dims[1] * cell_dims[2]];
        let mut verts: Vec<V3> = vec![];
        const EDGES: [[usize; 2]; 12] = [
            [0, 1],
            [2, 3],
            [4, 5],
            [6, 7],
            [0, 2],
            [1, 3],
            [4, 6],
            [5, 7],
            [0, 4],
            [1, 5],
            [2, 6],
            [3, 7],
        ];
        for z in 0..cell_dims[2] {
            for y in 0..cell_dims[1] {
                for x in 0..cell_dims[0] {
                    let mut c = [0.0f32; 8];
                    let mut neg = 0;
                    for k in 0..8 {
                        c[k] = g[idx(x + (k & 1), y + ((k >> 1) & 1), z + ((k >> 2) & 1))];
                        if c[k] < 0.0 {
                            neg += 1;
                        }
                    }
                    if neg == 0 || neg == 8 {
                        continue;
                    }
                    let mut acc = [0.0f64; 3];
                    let mut cnt = 0.0;
                    for e in EDGES {
                        let (a, b) = (c[e[0]] as f64, c[e[1]] as f64);
                        if (a < 0.0) != (b < 0.0) {
                            let t = a / (a - b);
                            let pa = [
                                (e[0] & 1) as f64,
                                ((e[0] >> 1) & 1) as f64,
                                ((e[0] >> 2) & 1) as f64,
                            ];
                            let pb = [
                                (e[1] & 1) as f64,
                                ((e[1] >> 1) & 1) as f64,
                                ((e[1] >> 2) & 1) as f64,
                            ];
                            for d in 0..3 {
                                acc[d] += pa[d] + (pb[d] - pa[d]) * t;
                            }
                            cnt += 1.0;
                        }
                    }
                    let p = [
                        (lo[0] as f64 + x as f64 + acc[0] / cnt) * self.h,
                        (lo[1] as f64 + y as f64 + acc[1] / cnt) * self.h,
                        (lo[2] as f64 + z as f64 + acc[2] / cnt) * self.h,
                    ];
                    cell_vert[cidx(x, y, z)] = verts.len() as u32;
                    verts.push(p);
                }
            }
        }
        // a quad around every grid edge with a sign change
        let mut tris: Vec<[u32; 3]> = vec![];
        let quad = |a: u32, b: u32, c: u32, d: u32, flip: bool, tris: &mut Vec<[u32; 3]>| {
            if a == u32::MAX || b == u32::MAX || c == u32::MAX || d == u32::MAX {
                return;
            }
            if flip {
                tris.push([a, c, b]);
                tris.push([a, d, c]);
            } else {
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        };
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let here = g[idx(x, y, z)] < 0.0;
                    // edges along +x, +y, +z
                    if x < cell_dims[0]
                        && y >= 1
                        && z >= 1
                        && y < cell_dims[1]
                        && z < cell_dims[2]
                        && (g[idx(x + 1, y, z)] < 0.0) != here
                    {
                        let (a, b, c, d) = (
                            cell_vert[cidx(x, y - 1, z - 1)],
                            cell_vert[cidx(x, y, z - 1)],
                            cell_vert[cidx(x, y, z)],
                            cell_vert[cidx(x, y - 1, z)],
                        );
                        quad(a, b, c, d, !here, &mut tris);
                    }
                    if y < cell_dims[1]
                        && x >= 1
                        && z >= 1
                        && x < cell_dims[0]
                        && z < cell_dims[2]
                        && (g[idx(x, y + 1, z)] < 0.0) != here
                    {
                        let (a, b, c, d) = (
                            cell_vert[cidx(x - 1, y, z - 1)],
                            cell_vert[cidx(x - 1, y, z)],
                            cell_vert[cidx(x, y, z)],
                            cell_vert[cidx(x, y, z - 1)],
                        );
                        quad(a, b, c, d, !here, &mut tris);
                    }
                    if z < cell_dims[2]
                        && x >= 1
                        && y >= 1
                        && x < cell_dims[0]
                        && y < cell_dims[1]
                        && (g[idx(x, y, z + 1)] < 0.0) != here
                    {
                        let (a, b, c, d) = (
                            cell_vert[cidx(x - 1, y - 1, z)],
                            cell_vert[cidx(x, y - 1, z)],
                            cell_vert[cidx(x, y, z)],
                            cell_vert[cidx(x - 1, y, z)],
                        );
                        quad(a, b, c, d, !here, &mut tris);
                    }
                }
            }
        }
        // orient every triangle to the gradient (the field points out of the solid)
        for t in tris.iter_mut() {
            let (a, b, c) = (
                verts[t[0] as usize],
                verts[t[1] as usize],
                verts[t[2] as usize],
            );
            let n = crate::cross(crate::sub(b, a), crate::sub(c, a));
            let centre = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            if dot(n, norm(self.gradient(centre))) < 0.0 {
                t.swap(1, 2);
            }
        }
        Mesh { verts, tris }
    }
}

impl Mesh {
    /// The enclosed volume by the divergence theorem (for closed meshes).
    pub fn enclosed_volume(&self) -> f64 {
        let mut v = 0.0;
        for t in &self.tris {
            let (a, b, c) = (
                self.verts[t[0] as usize],
                self.verts[t[1] as usize],
                self.verts[t[2] as usize],
            );
            v += dot(a, crate::cross(b, c)) / 6.0;
        }
        v
    }
    pub fn area(&self) -> f64 {
        let mut s = 0.0;
        for t in &self.tris {
            let (a, b, c) = (
                self.verts[t[0] as usize],
                self.verts[t[1] as usize],
                self.verts[t[2] as usize],
            );
            s += crate::len(crate::cross(crate::sub(b, a), crate::sub(c, a))) * 0.5;
        }
        s
    }
    /// Every edge shared by exactly two triangles (a closed surface has no others).
    pub fn is_closed(&self) -> bool {
        let mut e: BTreeMap<(u32, u32), i32> = BTreeMap::new();
        for t in &self.tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *e.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        e.values().all(|c| *c == 2)
    }
}
