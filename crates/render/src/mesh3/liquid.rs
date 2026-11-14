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

fn over(px: &mut [u8], rgb: [f64; 3], a: f64) {
    let a = a.clamp(0.0, 1.0);
    for k in 0..3 {
        px[k] =
            (rgb[k].clamp(0.0, 1.0) * a * 255.0 + px[k] as f64 * (1.0 - a) + 0.5).min(255.0) as u8;
    }
    px[3] = (a * 255.0 + px[3] as f64 * (1.0 - a) + 0.5).min(255.0) as u8;
}

/// Draw every liquid of the snapshot over the (supersampled, premultiplied) picture `big`, hidden
/// by the opaque surfaces whose depth is `opaque`.
#[allow(clippy::too_many_arguments)]
pub(super) fn composite(
    big: &mut Pixmap,
    opaque: &[f32],
    w: usize,
    h: usize,
    screen: &Screen,
    light: &Light,
    fluids: &Value,
    threads: usize,
) {
    for fl in fluids.as_array().into_iter().flatten() {
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
        let rgb = |c: tiny_skia::Color| [c.red() as f64, c.green() as f64, c.blue() as f64];
        let (c0, c1) = (rgb(surf), rgb(deep));
        let base_alpha = (surf.alpha() as f64).clamp(0.15, 1.0);
        let cam = &screen.cam;
        let view = [-cam.forward[0], -cam.forward[1], -cam.forward[2]];
        // the half vector for a highlight
        let ldir = [-light.dir[0], -light.dir[1], -light.dir[2]];
        let hv = {
            let v = [ldir[0] + view[0], ldir[1] + view[1], ldir[2] + view[2]];
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
            [v[0] / l, v[1] / l, v[2] / l]
        };
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
                            let px = &mut big.data_mut()[(y * w + x) * 4..(y * w + x) * 4 + 4];
                            over(px, [c0[0], c0[1], c0[2]], base_alpha);
                        }
                    }
                }
            }
            continue;
        }
        let mesh = surface(&particles, spacing, smoothness, threads, 1.0);
        let n_px = w * h;
        let mut front = vec![f64::INFINITY; n_px];
        let mut fnorm = vec![[0.0f64; 3]; n_px];
        let mut back = vec![f64::INFINITY; n_px];
        let mut facing = vec![];
        for (ti, t) in mesh.tris.iter().enumerate() {
            let (u, v) = (sub(t[1], t[0]), sub(t[2], t[0]));
            let gn = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let to_eye = sub(cam.eye, t[0]);
            let is_front = gn[0] * to_eye[0] + gn[1] * to_eye[1] + gn[2] * to_eye[2] > 0.0;
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
                mesh.normals[ti],
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
        for (ti, t) in mesh.tris.iter().enumerate() {
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
                mesh.normals[ti],
                w,
                h,
                &mut |p, d, _| {
                    if front[p].is_finite() && d > front[p] - 1e-6 && d < back[p] {
                        back[p] = d;
                    }
                },
            );
        }
        let data = big.data_mut();
        for p in 0..n_px {
            if !front[p].is_finite() {
                continue;
            }
            let end = back[p].min(opaque[p] as f64);
            let thick = if end.is_finite() {
                (end - front[p]).max(0.0)
            } else {
                4.0 * spacing
            };
            let t = 1.0 - (-thick / (8.0 * spacing)).exp();
            let n = fnorm[p];
            let lit = shade(light, n);
            let mut col = [0.0; 3];
            for k in 0..3 {
                col[k] = (c0[k] + (c1[k] - c0[k]) * t) * (0.6 + 0.4 * lit);
            }
            let nv = (n[0] * view[0] + n[1] * view[1] + n[2] * view[2]).abs();
            let fres = (1.0 - nv).powi(3);
            let spec = n[0] * hv[0] + n[1] * hv[1] + n[2] * hv[2];
            if spec > 0.94 {
                for c in col.iter_mut() {
                    *c = (*c + 0.35).min(1.0);
                }
            }
            let alpha = base_alpha + (1.0 - base_alpha) * (t * 0.75 + fres * 0.5);
            over(&mut data[p * 4..p * 4 + 4], col, alpha.min(0.96));
        }
        // an inked silhouette where the water meets the air or a farther surface
        let ink = [0.06, 0.16, 0.30];
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
                        !front[q].is_finite() && opaque[q] as f64 >= front[p]
                    });
                if edge {
                    over(&mut data[p * 4..p * 4 + 4], ink, 0.85);
                }
            }
        }
        // thin jets and loose droplets are too sparse to hold a surface: draw them as small spheres
        {
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
            for p in &particles {
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
                let r =
                    (0.6 * spacing * cam.focal / cc[2] * screen.tf.sx as f64 * SS as f64).max(1.0);
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
                        if d2 <= r * r && cc[2] < opaque[q] as f64 && cc[2] < front[q] {
                            // a lit ball: brighter towards the light's side
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
                            over(
                                &mut data[q * 4..q * 4 + 4],
                                col,
                                (base_alpha + 0.25).min(0.95),
                            );
                        }
                    }
                }
            }
        }
        // foam and spray: small white sprites, hidden by anything nearer
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
                    if d2 <= r * r && (cc[2] < opaque[q] as f64) && cc[2] < front[q] {
                        over(&mut data[q * 4..q * 4 + 4], [1.0, 1.0, 1.0], 0.7 * fade);
                    }
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
