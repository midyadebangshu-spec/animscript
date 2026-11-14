#![allow(clippy::needless_range_loop)]
//! Liquids in the 3D picture. The snapshot carries a fluid's particles; here their density field is
//! filled on a grid and a triangle mesh is extracted from it (marching cubes), then drawn on top of
//! the opaque meshes with transparency: the colour deepens with the thickness of water behind each
//! pixel, the surface is toon-lit like the meshes and inked at its silhouette. A pure function of
//! the snapshot; the field is summed in a fixed order, so the picture does not depend on how many
//! threads fill it.
use super::mc_table::{EDGES, TRI};
use super::{shade, sub, Light, Screen, NEAR, SS};
use crate::parse_hex;
use serde_json::Value;
use tiny_skia::Pixmap;

type V3 = [f64; 3];

/// The triangles of a liquid surface: three vertices (pixels of the scene) and their outward unit
/// normals per triangle.
pub struct LiquidMesh {
    pub tris: Vec<[V3; 3]>,
    pub normals: Vec<[V3; 3]>,
}

fn unb64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let v: Vec<u32> = s.bytes().filter_map(val).collect();
    let mut out = Vec::with_capacity(v.len() * 3 / 4);
    for c in v.chunks(4) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, x)| n | x << (18 - 6 * i));
        out.push((n >> 16) as u8);
        if c.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

fn floats(s: &str) -> Vec<f32> {
    unb64(s)
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn kernel(r2: f64, big_r2: f64) -> f64 {
    if r2 >= big_r2 {
        0.0
    } else {
        let q = 1.0 - r2 / big_r2;
        q * q * q
    }
}

/// The iso value of the density field: the field half a spacing outside a flat face of a lattice of
/// particles, so the surface of resting water lies half a spacing beyond its outer particles.
fn iso_value(spacing: f64, radius: f64) -> f64 {
    let r2 = radius * radius;
    let n = (radius / spacing).ceil() as i32 + 1;
    let mut sum = 0.0;
    for i in 0..=n {
        for j in -n..=n {
            for k in -n..=n {
                let d2 = ((0.5 + i as f64) * spacing).powi(2)
                    + (j as f64 * spacing).powi(2)
                    + (k as f64 * spacing).powi(2);
                sum += kernel(d2, r2);
            }
        }
    }
    sum
}

/// Extract the surface of a particle cloud: `smoothness` is the radius of each particle's blob in
/// spacings; `threads` fills the field in z slabs (0 = the machine's parallelism).
pub fn surface(
    particles: &[V3],
    spacing: f64,
    smoothness: f64,
    threads: usize,
    iso_scale: f64,
) -> LiquidMesh {
    let mut mesh = LiquidMesh {
        tris: vec![],
        normals: vec![],
    };
    if particles.is_empty() {
        return mesh;
    }
    let radius = smoothness * spacing;
    let big_r2 = radius * radius;
    let iso = iso_value(spacing, radius) * iso_scale;
    let (mut lo, mut hi) = (particles[0], particles[0]);
    for p in particles {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mut cell = spacing * 0.5;
    let pad = radius + 2.0 * cell;
    let dims = |cell: f64| -> [usize; 3] {
        [
            ((hi[0] - lo[0] + 2.0 * pad) / cell).ceil() as usize + 1,
            ((hi[1] - lo[1] + 2.0 * pad) / cell).ceil() as usize + 1,
            ((hi[2] - lo[2] + 2.0 * pad) / cell).ceil() as usize + 1,
        ]
    };
    let mut n = dims(cell);
    while n[0] * n[1] * n[2] > 6_000_000 {
        cell *= 1.25;
        n = dims(cell);
    }
    let origin = [lo[0] - pad, lo[1] - pad, lo[2] - pad];
    let (nx, ny, nz) = (n[0], n[1], n[2]);
    let idx = |i: usize, j: usize, k: usize| (k * ny + j) * nx + i;
    let t = if threads > 0 {
        threads
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    }
    .min(nz)
    .max(1);
    let per = nz.div_ceil(t);
    let reach = (radius / cell).ceil() as i64 + 1;
    // fill the density field: each slab of z layers sums the particles that reach it, in order
    let slabs: Vec<(usize, Vec<f32>)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..t)
            .map(|si| {
                let (k0, k1) = ((si * per).min(nz), ((si + 1) * per).min(nz));
                s.spawn(move || {
                    let mut f = vec![0f32; nx * ny * (k1 - k0)];
                    for p in particles {
                        let c = [
                            ((p[0] - origin[0]) / cell).round() as i64,
                            ((p[1] - origin[1]) / cell).round() as i64,
                            ((p[2] - origin[2]) / cell).round() as i64,
                        ];
                        let (kz0, kz1) = (
                            (c[2] - reach).max(k0 as i64),
                            (c[2] + reach).min(k1 as i64 - 1),
                        );
                        for k in kz0..=kz1 {
                            let dz = origin[2] + k as f64 * cell - p[2];
                            for j in (c[1] - reach).max(0)..=(c[1] + reach).min(ny as i64 - 1) {
                                let dy = origin[1] + j as f64 * cell - p[1];
                                for i in (c[0] - reach).max(0)..=(c[0] + reach).min(nx as i64 - 1) {
                                    let dx = origin[0] + i as f64 * cell - p[0];
                                    let w = kernel(dx * dx + dy * dy + dz * dz, big_r2);
                                    if w > 0.0 {
                                        f[((k as usize - k0) * ny + j as usize) * nx
                                            + i as usize] += w as f32;
                                    }
                                }
                            }
                        }
                    }
                    (k0, f)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a field worker panicked"))
            .collect()
    });
    let mut field = vec![0f32; nx * ny * nz];
    for (k0, f) in slabs {
        field[k0 * nx * ny..k0 * nx * ny + f.len()].copy_from_slice(&f);
    }
    let at = |i: i64, j: i64, k: i64| -> f64 {
        let (i, j, k) = (
            i.clamp(0, nx as i64 - 1) as usize,
            j.clamp(0, ny as i64 - 1) as usize,
            k.clamp(0, nz as i64 - 1) as usize,
        );
        field[idx(i, j, k)] as f64
    };
    // the gradient at a node (central differences), pointing into the water
    let grad = |i: usize, j: usize, k: usize| -> V3 {
        let (i, j, k) = (i as i64, j as i64, k as i64);
        [
            at(i + 1, j, k) - at(i - 1, j, k),
            at(i, j + 1, k) - at(i, j - 1, k),
            at(i, j, k + 1) - at(i, j, k - 1),
        ]
    };
    let corner = [
        [0usize, 0, 0],
        [1, 0, 0],
        [1, 1, 0],
        [0, 1, 0],
        [0, 0, 1],
        [1, 0, 1],
        [1, 1, 1],
        [0, 1, 1],
    ];
    for k in 0..nz - 1 {
        for j in 0..ny - 1 {
            for i in 0..nx - 1 {
                let v: Vec<f64> = corner
                    .iter()
                    .map(|c| field[idx(i + c[0], j + c[1], k + c[2])] as f64)
                    .collect();
                let mut cube = 0usize;
                for (b, val) in v.iter().enumerate() {
                    if *val > iso {
                        cube |= 1 << b;
                    }
                }
                if cube == 0 || cube == 255 {
                    continue;
                }
                let row = &TRI[cube];
                let mut e = 0;
                while e < 16 && row[e] >= 0 {
                    let mut pts = [[0.0; 3]; 3];
                    let mut nrm = [[0.0; 3]; 3];
                    for q in 0..3 {
                        let [a, b] = EDGES[row[e + q] as usize];
                        let (fa, fb) = (v[a], v[b]);
                        let tt = if (fb - fa).abs() < 1e-12 {
                            0.5
                        } else {
                            (iso - fa) / (fb - fa)
                        };
                        let (ca, cb) = (corner[a], corner[b]);
                        let pa = [i + ca[0], j + ca[1], k + ca[2]];
                        let pb = [i + cb[0], j + cb[1], k + cb[2]];
                        let (ga, gb) = (grad(pa[0], pa[1], pa[2]), grad(pb[0], pb[1], pb[2]));
                        let mut g = [0.0; 3];
                        for d in 0..3 {
                            pts[q][d] = origin[d]
                                + ((pa[d] as f64) + ((pb[d] as f64) - (pa[d] as f64)) * tt) * cell;
                            g[d] = -(ga[d] + (gb[d] - ga[d]) * tt);
                        }
                        let l = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-12);
                        nrm[q] = [g[0] / l, g[1] / l, g[2] / l];
                    }
                    // wind the triangle so its geometric normal agrees with the field's
                    let (u, w) = (sub(pts[1], pts[0]), sub(pts[2], pts[0]));
                    let gn = [
                        u[1] * w[2] - u[2] * w[1],
                        u[2] * w[0] - u[0] * w[2],
                        u[0] * w[1] - u[1] * w[0],
                    ];
                    let avg = [
                        nrm[0][0] + nrm[1][0] + nrm[2][0],
                        nrm[0][1] + nrm[1][1] + nrm[2][1],
                        nrm[0][2] + nrm[1][2] + nrm[2][2],
                    ];
                    if gn[0] * avg[0] + gn[1] * avg[1] + gn[2] * avg[2] < 0.0 {
                        pts.swap(1, 2);
                        nrm.swap(1, 2);
                    }
                    mesh.tris.push(pts);
                    mesh.normals.push(nrm);
                    e += 3;
                }
            }
        }
    }
    mesh
}

/// Rasterize triangles (screen space, camera depth, per-corner normals) into a depth buffer:
/// `f(pixel, depth, normal)` is called for every covered pixel.
fn raster(
    s: [[f64; 2]; 3],
    z: [f64; 3],
    n: [V3; 3],
    w: usize,
    h: usize,
    f: &mut impl FnMut(usize, f64, V3),
) {
    let (minx, maxx) = (
        s.iter()
            .map(|p| p[0])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.0) as usize,
        (s.iter().map(|p| p[0]).fold(f64::MIN, f64::max).ceil() as i64).min(w as i64 - 1),
    );
    let (miny, maxy) = (
        s.iter()
            .map(|p| p[1])
            .fold(f64::MAX, f64::min)
            .floor()
            .max(0.0) as usize,
        (s.iter().map(|p| p[1]).fold(f64::MIN, f64::max).ceil() as i64).min(h as i64 - 1),
    );
    if maxx < 0 || maxy < 0 {
        return;
    }
    let area =
        (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
    if area.abs() < 1e-12 {
        return;
    }
    let inv_z = [1.0 / z[0], 1.0 / z[1], 1.0 / z[2]];
    for y in miny..=maxy as usize {
        for x in minx..=maxx as usize {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
            let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let iz = w0 * inv_z[0] + w1 * inv_z[1] + w2 * inv_z[2];
            let depth = 1.0 / iz;
            let mut nn = [0.0; 3];
            for d in 0..3 {
                nn[d] =
                    (w0 * n[0][d] * inv_z[0] + w1 * n[1][d] * inv_z[1] + w2 * n[2][d] * inv_z[2])
                        / iz;
            }
            let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2])
                .sqrt()
                .max(1e-12);
            f(y * w + x, depth, [nn[0] / l, nn[1] / l, nn[2] / l]);
        }
    }
}

/// A transparent skin to composite: a liquid's surface or a glass mesh.
struct Skin {
    tris: Vec<[V3; 3]>,
    normals: Vec<[V3; 3]>,
    c0: [f64; 3],
    c1: [f64; 3],
    base_alpha: f64,
    /// How far through it the colour deepens (px).
    absorb: f64,
    reflect: f64,
    refract: f64,
    ink: Option<[f64; 3]>,
}

#[derive(Clone, Copy)]
struct Sky {
    top: [f64; 3],
    horizon: [f64; 3],
}

fn rgb_of(c: tiny_skia::Color) -> [f64; 3] {
    [c.red() as f64, c.green() as f64, c.blue() as f64]
}

fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn norm(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn dotv(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Everything the surfaces need to see: the picture so far and the depth of the opaque meshes.
struct Ctx<'a> {
    w: usize,
    h: usize,
    opaque: &'a [f32],
    screen: &'a Screen,
    light: &'a Light,
    sky: Sky,
    /// The scene as it is now (backdrop, opaque meshes, surfaces already composited), opaque colours.
    under: Vec<[f32; 3]>,
}

impl Ctx<'_> {
    /// The scene point under pixel `(x, y)` (supersampled) at camera depth `z`.
    fn unproject(&self, x: f64, y: f64, z: f64) -> V3 {
        let t = &self.screen.tf;
        let (px, py) = (x / SS as f64 - t.tx as f64, y / SS as f64 - t.ty as f64);
        let det = t.sx as f64 * t.sy as f64 - t.kx as f64 * t.ky as f64;
        let sx = (t.sy as f64 * px - t.kx as f64 * py) / det;
        let sy = (-t.ky as f64 * px + t.sx as f64 * py) / det;
        let cam = &self.screen.cam;
        let (c0, c1) = ((sx - cam.cx) * z / cam.focal, (sy - cam.cy) * z / cam.focal);
        [
            cam.eye[0] + cam.right[0] * c0 + cam.down[0] * c1 + cam.forward[0] * z,
            cam.eye[1] + cam.right[1] * c0 + cam.down[1] * c1 + cam.forward[1] * z,
            cam.eye[2] + cam.right[2] * c0 + cam.down[2] * c1 + cam.forward[2] * z,
        ]
    }

    /// Pixel index of a scene point and its camera depth, if it is on screen.
    fn locate(&self, p: V3) -> Option<(usize, f64)> {
        let c = self.screen.to_camera(p);
        if c[2] < NEAR {
            return None;
        }
        let s = self.screen.project(c);
        if s[0] < 0.0 || s[1] < 0.0 || s[0] >= self.w as f64 || s[1] >= self.h as f64 {
            return None;
        }
        Some((s[1] as usize * self.w + s[0] as usize, c[2]))
    }

    fn sky_color(&self, dir: V3) -> [f64; 3] {
        let up = (-dir[1]).clamp(-1.0, 1.0);
        let mut col = if up >= 0.0 {
            mix(self.sky.horizon, self.sky.top, up.powf(0.6))
        } else {
            // below the horizon: the ground, a darker horizon colour
            mix(
                self.sky.horizon,
                [
                    0.25 * self.sky.horizon[0],
                    0.25 * self.sky.horizon[1],
                    0.25 * self.sky.horizon[2],
                ],
                (-up).min(1.0),
            )
        };
        // a glint of the light source
        let sun = dotv(
            dir,
            [-self.light.dir[0], -self.light.dir[1], -self.light.dir[2]],
        )
        .max(0.0);
        let glint = sun.powi(160) * 1.4 + sun.powi(16) * 0.12;
        for c in col.iter_mut() {
            *c = (*c + glint).min(1.0);
        }
        col
    }

    /// What a mirror at `p` with normal `n` shows: screen-space ray march against the opaque
    /// meshes, the sky where it finds nothing.
    fn reflect_color(&self, p: V3, n: V3) -> [f64; 3] {
        let cam = &self.screen.cam;
        let v = norm(sub(p, cam.eye));
        let n = if dotv(n, v) > 0.0 {
            [-n[0], -n[1], -n[2]]
        } else {
            n
        };
        let d = dotv(v, n);
        let r = norm([
            v[0] - 2.0 * d * n[0],
            v[1] - 2.0 * d * n[1],
            v[2] - 2.0 * d * n[2],
        ]);
        let start = [p[0] + n[0] * 3.0, p[1] + n[1] * 3.0, p[2] + n[2] * 3.0];
        let mut t = 8.0;
        for _ in 0..56 {
            let q = [
                start[0] + r[0] * t,
                start[1] + r[1] * t,
                start[2] + r[2] * t,
            ];
            let Some((i, z)) = self.locate(q) else {
                break;
            };
            let depth = self.opaque[i] as f64;
            if depth.is_finite() && z > depth && z - depth < 30.0 + 0.06 * z {
                let c = self.under[i];
                return [c[0] as f64, c[1] as f64, c[2] as f64];
            }
            t = t * 1.14 + 4.0;
            if t > 6000.0 {
                break;
            }
        }
        self.sky_color(r)
    }
}

fn put(data: &mut [u8], p: usize, rgb: [f64; 3]) {
    for k in 0..3 {
        data[p * 4 + k] = (rgb[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    data[p * 4 + 3] = 255;
}

/// Composite one transparent skin over the picture: refract what is behind it, tint it by its
/// thickness, light it, reflect the scene by Fresnel, ink its silhouette.
fn skin_pass(ctx: &mut Ctx, big: &mut Pixmap, skin: &Skin) -> Vec<f64> {
    let (w, h) = (ctx.w, ctx.h);
    let cam = &ctx.screen.cam;
    let eye = cam.eye;
    let view = [-cam.forward[0], -cam.forward[1], -cam.forward[2]];
    let n_px = w * h;
    let mut front = vec![f64::INFINITY; n_px];
    let mut fnorm = vec![[0.0f64; 3]; n_px];
    let mut back = vec![f64::INFINITY; n_px];
    let mut facing = vec![];
    let screen = ctx.screen;
    let opaque = ctx.opaque;
    for (ti, t) in skin.tris.iter().enumerate() {
        let (u, v) = (sub(t[1], t[0]), sub(t[2], t[0]));
        let gn = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let to_eye = sub(eye, t[0]);
        let is_front = dotv(gn, to_eye) > 0.0;
        facing.push(is_front);
        if !is_front {
            continue;
        }
        let cs: Vec<V3> = t.iter().map(|p| screen.to_camera(*p)).collect();
        if cs.iter().any(|c| c[2] < NEAR) {
            continue;
        }
        let sp = [
            screen.project(cs[0]),
            screen.project(cs[1]),
            screen.project(cs[2]),
        ];
        raster(
            sp,
            [cs[0][2], cs[1][2], cs[2][2]],
            skin.normals[ti],
            w,
            h,
            &mut |p, d, n| {
                if d < front[p] && (d as f32) < opaque[p] {
                    front[p] = d;
                    fnorm[p] = n;
                }
            },
        );
    }
    for (ti, t) in skin.tris.iter().enumerate() {
        if facing[ti] {
            continue;
        }
        let cs: Vec<V3> = t.iter().map(|p| screen.to_camera(*p)).collect();
        if cs.iter().any(|c| c[2] < NEAR) {
            continue;
        }
        let sp = [
            screen.project(cs[0]),
            screen.project(cs[1]),
            screen.project(cs[2]),
        ];
        raster(
            sp,
            [cs[0][2], cs[1][2], cs[2][2]],
            skin.normals[ti],
            w,
            h,
            &mut |p, d, _| {
                if front[p].is_finite() && d > front[p] - 1e-6 && d < back[p] {
                    back[p] = d;
                }
            },
        );
    }
    let ldir = [-ctx.light.dir[0], -ctx.light.dir[1], -ctx.light.dir[2]];
    let hv = norm([ldir[0] + view[0], ldir[1] + view[1], ldir[2] + view[2]]);
    let mut result: Vec<Option<[f64; 3]>> = vec![None; n_px];
    for p in 0..n_px {
        if !front[p].is_finite() {
            continue;
        }
        let (x, y) = ((p % w) as f64 + 0.5, (p / w) as f64 + 0.5);
        let end = back[p].min(ctx.opaque[p] as f64);
        let thick = if end.is_finite() {
            (end - front[p]).max(0.0)
        } else {
            4.0 * skin.absorb / 3.0
        };
        let t = 1.0 - (-thick / skin.absorb).exp();
        let n = fnorm[p];
        let pos = ctx.unproject(x, y, front[p]);
        let vdir = norm(sub(pos, eye));
        // refraction: bend the ray into the water by Snell's law and look where it would come out
        let under_here = ctx.under[p];
        let mut trans = [
            under_here[0] as f64,
            under_here[1] as f64,
            under_here[2] as f64,
        ];
        if skin.refract > 0.0 {
            let cos_i = -dotv(n, vdir);
            let eta = 1.0 / 1.33;
            let k = 1.0 - eta * eta * (1.0 - cos_i * cos_i);
            if k > 0.0 {
                let f = eta * cos_i - k.sqrt();
                let rdir = [
                    eta * vdir[0] + f * n[0],
                    eta * vdir[1] + f * n[1],
                    eta * vdir[2] + f * n[2],
                ];
                let dist = thick.min(skin.absorb * 1.5) * skin.refract;
                let q = [
                    pos[0] + rdir[0] * dist,
                    pos[1] + rdir[1] * dist,
                    pos[2] + rdir[2] * dist,
                ];
                if let Some((qi, _)) = ctx.locate(q) {
                    // only what lies behind the water can show through it
                    if (ctx.opaque[qi] as f64) > front[p] {
                        let c = ctx.under[qi];
                        trans = [c[0] as f64, c[1] as f64, c[2] as f64];
                    }
                }
            }
        }
        let lit = shade(ctx.light, n);
        let body = mix(skin.c0, skin.c1, t);
        let col = [
            body[0] * (0.6 + 0.4 * lit),
            body[1] * (0.6 + 0.4 * lit),
            body[2] * (0.6 + 0.4 * lit),
        ];
        let nv = dotv(n, view).abs();
        let fres = (1.0 - nv).powi(5);
        let alpha = (skin.base_alpha + (1.0 - skin.base_alpha) * (t * 0.75 + fres * 0.4)).min(0.96);
        let seen = mix(trans, col, alpha);
        // Fresnel: more of the surroundings at a grazing angle
        let f = ((0.05 + 0.95 * fres) * skin.reflect).clamp(0.0, 0.95);
        let mut out = if f > 0.002 {
            mix(seen, ctx.reflect_color(pos, n), f)
        } else {
            seen
        };
        if dotv(n, hv) > 0.94 {
            for c in out.iter_mut() {
                *c = (*c + 0.3).min(1.0);
            }
        }
        result[p] = Some(out);
    }
    let data = big.data_mut();
    for p in 0..n_px {
        if let Some(c) = result[p] {
            put(data, p, c);
            ctx.under[p] = [c[0] as f32, c[1] as f32, c[2] as f32];
        }
    }
    // an inked silhouette where the skin meets the air or a farther surface
    if let Some(ink) = skin.ink {
        let data = big.data_mut();
        for y in 0..h {
            for x in 0..w {
                let p = y * w + x;
                if !front[p].is_finite() {
                    continue;
                }
                let edge = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| {
                        let (qx, qy) = (x as i64 + dx, y as i64 + dy);
                        if qx < 0 || qy < 0 || qx >= w as i64 || qy >= h as i64 {
                            return false;
                        }
                        let q = qy as usize * w + qx as usize;
                        !front[q].is_finite() && ctx.opaque[q] as f64 >= front[p]
                    });
                if edge {
                    let cur = ctx.under[p];
                    let c = mix([cur[0] as f64, cur[1] as f64, cur[2] as f64], ink, 0.85);
                    put(data, p, c);
                    ctx.under[p] = [c[0] as f32, c[1] as f32, c[2] as f32];
                }
            }
        }
    }
    front
}

/// What the renderer hands to the liquids and materials.
pub(super) struct Surfaces<'a> {
    /// The picture the meshes are drawn over (the backdrop).
    pub backdrop: &'a Pixmap,
    /// Per pixel: how mirror-like the opaque surface there is, and its normal.
    pub refl: &'a [f32],
    pub nrm: &'a [[f32; 3]],
    /// Glass meshes: (vertices, faces, fill colour, material reflect, refract, outline, smooth angle).
    pub glass: &'a [GlassMesh],
}

pub(super) struct GlassMesh {
    pub verts: Vec<V3>,
    pub faces: Vec<Vec<usize>>,
    pub fill: tiny_skia::Color,
    pub outline: Option<tiny_skia::Color>,
    pub reflect: f64,
    pub refract: f64,
}

fn sky_of(snapshot_light: &Value, background: &Value) -> Sky {
    let parse = |v: &Value| v.as_str().and_then(parse_hex).map(rgb_of);
    if let Some(a) = snapshot_light["sky"].as_array() {
        if let (Some(t), Some(h)) = (a.first().and_then(parse), a.get(1).and_then(parse)) {
            return Sky { top: t, horizon: h };
        }
    }
    // no sky given: the background colour at the horizon, a bluer and deeper version above
    let bg = parse(background).unwrap_or([0.8, 0.9, 0.95]);
    Sky {
        top: mix(bg, [0.35, 0.62, 0.9], 0.55),
        horizon: bg,
    }
}

/// Draw the mirrors, then the liquids, then the glass of the snapshot over the (supersampled,
/// premultiplied) picture `big`, hidden by the opaque surfaces whose depth is `opaque`.
#[allow(clippy::too_many_arguments)]
pub(super) fn composite(
    big: &mut Pixmap,
    opaque: &[f32],
    w: usize,
    h: usize,
    screen: &Screen,
    light: &Light,
    snapshot: &Value,
    surfaces: &Surfaces,
    threads: usize,
) {
    // the scene as it stands: the backdrop under the opaque meshes
    let mut under = vec![[0f32; 3]; w * h];
    {
        let bd = surfaces.backdrop;
        let (bw, bh) = (bd.width() as usize, bd.height() as usize);
        let src = bd.data();
        let b = big.data();
        for y in 0..h {
            for x in 0..w {
                let (bx, by) = ((x / SS).min(bw - 1), (y / SS).min(bh - 1));
                let si = (by * bw + bx) * 4;
                let pi = (y * w + x) * 4;
                let a = b[pi + 3] as f32 / 255.0;
                for k in 0..3 {
                    under[y * w + x][k] =
                        b[pi + k] as f32 / 255.0 + (src[si + k] as f32 / 255.0) * (1.0 - a);
                }
            }
        }
    }
    let sky = sky_of(&snapshot["light3"], &snapshot["background"]);
    let mut ctx = Ctx {
        w,
        h,
        opaque,
        screen,
        light,
        sky,
        under,
    };
    // mirrors: opaque surfaces that show their surroundings
    for p in 0..w * h {
        let r = surfaces.refl[p] as f64;
        if r <= 0.0 || !opaque[p].is_finite() {
            continue;
        }
        let (x, y) = ((p % w) as f64 + 0.5, (p / w) as f64 + 0.5);
        let pos = ctx.unproject(x, y, opaque[p] as f64);
        let n = surfaces.nrm[p];
        let refl = ctx.reflect_color(pos, [n[0] as f64, n[1] as f64, n[2] as f64]);
        let cur = ctx.under[p];
        let c = mix([cur[0] as f64, cur[1] as f64, cur[2] as f64], refl, r);
        put(big.data_mut(), p, c);
        ctx.under[p] = [c[0] as f32, c[1] as f32, c[2] as f32];
    }
    for fl in snapshot["fluids3"].as_array().into_iter().flatten() {
        let pts_flat = floats(fl["particles"].as_str().unwrap_or(""));
        let particles: Vec<V3> = pts_flat
            .chunks_exact(3)
            .map(|c| [c[0] as f64, c[1] as f64, c[2] as f64])
            .collect();
        let spacing = fl["spacing"].as_f64().unwrap_or(12.0);
        let smoothness = fl["smoothness"].as_f64().unwrap_or(1.4);
        let surf = fl["color"]
            .as_str()
            .and_then(parse_hex)
            .unwrap_or(tiny_skia::Color::from_rgba8(0x4f, 0x9b, 0xe8, 0xcc));
        let deep = fl["color_end"].as_str().and_then(parse_hex).unwrap_or(surf);
        let (c0, c1) = (rgb_of(surf), rgb_of(deep));
        let base_alpha = (surf.alpha() as f64).clamp(0.15, 1.0);
        let cam = &screen.cam;
        if fl["render"] == "dots" {
            for p in &particles {
                let c = screen.to_camera(*p);
                if c[2] < NEAR {
                    continue;
                }
                let s = screen.project(c);
                let r =
                    (0.5 * spacing * cam.focal / c[2] * screen.tf.sx as f64 * SS as f64).max(1.0);
                let (x0, x1) = (
                    (s[0] - r).floor().max(0.0) as usize,
                    ((s[0] + r).ceil() as i64).min(w as i64 - 1),
                );
                let (y0, y1) = (
                    (s[1] - r).floor().max(0.0) as usize,
                    ((s[1] + r).ceil() as i64).min(h as i64 - 1),
                );
                if x1 < 0 || y1 < 0 {
                    continue;
                }
                for y in y0..=y1 as usize {
                    for x in x0..=x1 as usize {
                        let d2 = (x as f64 + 0.5 - s[0]).powi(2) + (y as f64 + 0.5 - s[1]).powi(2);
                        if d2 <= r * r && (c[2] as f32) < opaque[y * w + x] {
                            let cur = ctx.under[y * w + x];
                            let col = mix(
                                [cur[0] as f64, cur[1] as f64, cur[2] as f64],
                                c0,
                                base_alpha,
                            );
                            put(big.data_mut(), y * w + x, col);
                            ctx.under[y * w + x] = [col[0] as f32, col[1] as f32, col[2] as f32];
                        }
                    }
                }
            }
            continue;
        }
        let mesh = surface(&particles, spacing, smoothness, threads, 1.0);
        let skin = Skin {
            tris: mesh.tris,
            normals: mesh.normals,
            c0,
            c1,
            base_alpha,
            absorb: 8.0 * spacing,
            reflect: fl["reflect"].as_f64().unwrap_or(1.0),
            refract: fl["refract"].as_f64().unwrap_or(1.0),
            ink: Some([0.06, 0.16, 0.30]),
        };
        // water in front of a surface that is not visible has no front faces to show
        let front = skin_pass(&mut ctx, big, &skin);
        // droplets and foam over it, hidden where the water is nearer
        drop_sprites(
            &mut ctx, big, &particles, fl, spacing, c0, base_alpha, &front,
        );
    }
    for g in surfaces.glass {
        let (tris, normals) = glass_triangles(g, light);
        let c = rgb_of(g.fill);
        let skin = Skin {
            tris,
            normals,
            c0: c,
            c1: c,
            base_alpha: (g.fill.alpha() as f64).clamp(0.04, 1.0),
            absorb: 60.0,
            reflect: g.reflect,
            refract: g.refract,
            ink: g.outline.map(rgb_of),
        };
        skin_pass(&mut ctx, big, &skin);
    }
}

/// Triangles of a glass mesh with smooth corner normals.
fn glass_triangles(g: &GlassMesh, light: &Light) -> (Vec<[V3; 3]>, Vec<[V3; 3]>) {
    let mut fnormals = vec![];
    for f in &g.faces {
        if f.len() < 3 || f.iter().any(|i| *i >= g.verts.len()) {
            fnormals.push([0.0; 3]);
            continue;
        }
        let (a, b, c) = (g.verts[f[0]], g.verts[f[1]], g.verts[f[2]]);
        let (u, v) = (sub(b, a), sub(c, a));
        fnormals.push(norm([
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]));
    }
    let mut around: Vec<Vec<usize>> = vec![vec![]; g.verts.len()];
    for (fi, f) in g.faces.iter().enumerate() {
        if f.len() >= 3 && f.iter().all(|i| *i < g.verts.len()) {
            for &v in f {
                around[v].push(fi);
            }
        }
    }
    let smooth_cos = light.smooth.to_radians().cos();
    let (mut tris, mut normals) = (vec![], vec![]);
    for (fi, f) in g.faces.iter().enumerate() {
        if f.len() < 3 || f.iter().any(|i| *i >= g.verts.len()) {
            continue;
        }
        let corner: Vec<V3> = f
            .iter()
            .map(|&v| {
                if light.smooth <= 0.0 {
                    return fnormals[fi];
                }
                let mut acc = [0.0; 3];
                for &o in &around[v] {
                    if dotv(fnormals[fi], fnormals[o]) >= smooth_cos {
                        for k in 0..3 {
                            acc[k] += fnormals[o][k];
                        }
                    }
                }
                norm(acc)
            })
            .collect();
        for k in 1..f.len() - 1 {
            tris.push([g.verts[f[0]], g.verts[f[k]], g.verts[f[k + 1]]]);
            normals.push([corner[0], corner[k], corner[k + 1]]);
        }
    }
    (tris, normals)
}

/// Loose droplets and foam as small spheres over the finished water.
#[allow(clippy::too_many_arguments)]
fn drop_sprites(
    ctx: &mut Ctx,
    big: &mut Pixmap,
    particles: &[V3],
    fl: &Value,
    spacing: f64,
    c0: [f64; 3],
    base_alpha: f64,
    front: &[f64],
) {
    let (w, h) = (ctx.w, ctx.h);
    let screen = ctx.screen;
    let cam = &screen.cam;
    let light = ctx.light;
    let cellw = 1.3 * spacing;
    let key = |p: &V3| {
        (
            (p[0] / cellw).floor() as i64,
            (p[1] / cellw).floor() as i64,
            (p[2] / cellw).floor() as i64,
        )
    };
    let mut grid: std::collections::HashMap<(i64, i64, i64), Vec<u32>> =
        std::collections::HashMap::new();
    for (i, p) in particles.iter().enumerate() {
        grid.entry(key(p)).or_default().push(i as u32);
    }
    for p in particles {
        let k = key(p);
        let mut near = 0;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(list) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                        near += list
                            .iter()
                            .filter(|j| {
                                let q = particles[**j as usize];
                                let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < cellw * cellw
                            })
                            .count();
                    }
                }
            }
        }
        if near > 3 {
            continue;
        }
        let cc = screen.to_camera(*p);
        if cc[2] < NEAR {
            continue;
        }
        let s = screen.project(cc);
        let r = (0.6 * spacing * cam.focal / cc[2] * screen.tf.sx as f64 * SS as f64).max(1.0);
        let (x0, x1) = (
            (s[0] - r).floor().max(0.0) as usize,
            ((s[0] + r).ceil() as i64).min(w as i64 - 1),
        );
        let (y0, y1) = (
            (s[1] - r).floor().max(0.0) as usize,
            ((s[1] + r).ceil() as i64).min(h as i64 - 1),
        );
        if x1 < 0 || y1 < 0 {
            continue;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let q = y * w + x;
                let (dx, dy) = (x as f64 + 0.5 - s[0], y as f64 + 0.5 - s[1]);
                let d2 = dx * dx + dy * dy;
                if d2 <= r * r && cc[2] < ctx.opaque[q] as f64 && cc[2] < front[q] {
                    let nz = (1.0 - d2 / (r * r)).sqrt();
                    let n = [dx / r, dy / r, -nz];
                    let n = [
                        n[0] * cam.right[0] + n[1] * cam.down[0] + n[2] * cam.forward[0],
                        n[0] * cam.right[1] + n[1] * cam.down[1] + n[2] * cam.forward[1],
                        n[0] * cam.right[2] + n[1] * cam.down[2] + n[2] * cam.forward[2],
                    ];
                    let lit = shade(light, n);
                    let col = [
                        c0[0] * (0.6 + 0.4 * lit),
                        c0[1] * (0.6 + 0.4 * lit),
                        c0[2] * (0.6 + 0.4 * lit),
                    ];
                    let cur = ctx.under[q];
                    let out = mix(
                        [cur[0] as f64, cur[1] as f64, cur[2] as f64],
                        col,
                        (base_alpha + 0.25).min(0.95),
                    );
                    put(big.data_mut(), q, out);
                    ctx.under[q] = [out[0] as f32, out[1] as f32, out[2] as f32];
                }
            }
        }
    }
    let foam = floats(fl["foam"].as_str().unwrap_or(""));
    for c in foam.chunks_exact(4) {
        let p = [c[0] as f64, c[1] as f64, c[2] as f64];
        let cc = screen.to_camera(p);
        if cc[2] < NEAR {
            continue;
        }
        let s = screen.project(cc);
        let r = (0.3 * spacing * cam.focal / cc[2] * screen.tf.sx as f64 * SS as f64).max(1.0);
        let fade = (1.0 - c[3] as f64).clamp(0.0, 1.0);
        let (x0, x1) = (
            (s[0] - r).floor().max(0.0) as usize,
            ((s[0] + r).ceil() as i64).min(w as i64 - 1),
        );
        let (y0, y1) = (
            (s[1] - r).floor().max(0.0) as usize,
            ((s[1] + r).ceil() as i64).min(h as i64 - 1),
        );
        if x1 < 0 || y1 < 0 {
            continue;
        }
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let q = y * w + x;
                let d2 = (x as f64 + 0.5 - s[0]).powi(2) + (y as f64 + 0.5 - s[1]).powi(2);
                if d2 <= r * r && cc[2] < ctx.opaque[q] as f64 && cc[2] < front[q] {
                    let cur = ctx.under[q];
                    let out = mix(
                        [cur[0] as f64, cur[1] as f64, cur[2] as f64],
                        [1.0, 1.0, 1.0],
                        0.7 * fade,
                    );
                    put(big.data_mut(), q, out);
                    ctx.under[q] = [out[0] as f32, out[1] as f32, out[2] as f32];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball(radius: f64, spacing: f64) -> Vec<V3> {
        let n = (radius / spacing).ceil() as i32;
        let mut v = vec![];
        for k in -n..=n {
            for j in -n..=n {
                for i in -n..=n {
                    let p = [i as f64 * spacing, j as f64 * spacing, k as f64 * spacing];
                    if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= radius * radius {
                        v.push(p);
                    }
                }
            }
        }
        v
    }

    fn volume(m: &LiquidMesh) -> f64 {
        m.tris
            .iter()
            .map(|t| {
                let c = [
                    t[1][1] * t[2][2] - t[1][2] * t[2][1],
                    t[1][2] * t[2][0] - t[1][0] * t[2][2],
                    t[1][0] * t[2][1] - t[1][1] * t[2][0],
                ];
                (t[0][0] * c[0] + t[0][1] * c[1] + t[0][2] * c[2]) / 6.0
            })
            .sum()
    }

    #[test]
    fn a_ball_of_particles_gives_an_outward_closed_surface_of_about_its_volume() {
        let pts = ball(60.0, 12.0);
        let m = surface(&pts, 12.0, 1.4, 1, 1.0);
        assert!(m.tris.len() > 200);
        let want = pts.len() as f64 * 12.0f64.powi(3);
        let v = volume(&m);
        assert!(v > 0.0, "the surface faces inward ({v})");
        assert!(v > 0.7 * want && v < 1.6 * want, "volume {v} vs {want}");
        // every triangle's winding agrees with its normals, which point away from the centre
        let outward = m
            .tris
            .iter()
            .zip(&m.normals)
            .filter(|(t, n)| {
                let c = [
                    (t[0][0] + t[1][0] + t[2][0]) / 3.0,
                    (t[0][1] + t[1][1] + t[2][1]) / 3.0,
                    (t[0][2] + t[1][2] + t[2][2]) / 3.0,
                ];
                n[0][0] * c[0] + n[0][1] * c[1] + n[0][2] * c[2] > 0.0
            })
            .count();
        assert!(outward as f64 > 0.97 * m.tris.len() as f64);
    }

    #[test]
    fn the_thread_count_does_not_change_the_mesh() {
        let pts = ball(48.0, 12.0);
        let a = surface(&pts, 12.0, 1.4, 1, 1.0);
        let b = surface(&pts, 12.0, 1.4, 5, 1.0);
        assert_eq!(a.tris, b.tris);
        assert_eq!(a.normals, b.normals);
    }

    #[test]
    fn no_particles_no_surface() {
        assert!(surface(&[], 12.0, 1.4, 0, 1.0).tris.is_empty());
    }

    #[test]
    fn the_packed_floats_round_trip() {
        // 1.5f32, -2.0f32 little endian, base64
        let bytes = [1.5f32.to_le_bytes(), (-2.0f32).to_le_bytes()].concat();
        let enc = {
            const A: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for c in bytes.chunks(3) {
                let n = (c[0] as u32) << 16
                    | (*c.get(1).unwrap_or(&0) as u32) << 8
                    | *c.get(2).unwrap_or(&0) as u32;
                out.push(A[(n >> 18) as usize & 63] as char);
                out.push(A[(n >> 12) as usize & 63] as char);
                out.push(if c.len() > 1 {
                    A[(n >> 6) as usize & 63] as char
                } else {
                    '='
                });
                out.push(if c.len() > 2 {
                    A[n as usize & 63] as char
                } else {
                    '='
                });
            }
            out
        };
        assert_eq!(floats(&enc), vec![1.5, -2.0]);
    }
}
