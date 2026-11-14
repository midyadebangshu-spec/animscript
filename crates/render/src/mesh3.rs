//! A software rasterizer for the snapshot's 3D meshes: perspective projection, near-plane clipping, a z-buffer, flat
//! or toon shading per face, and inked outlines (silhouettes, boundaries and creases) that are hidden by nearer
//! surfaces. Everything is drawn at twice the resolution and averaged down for smooth edges. It is a pure function of
//! the snapshot: no threads, no randomness.
use crate::{parse_hex, View};
mod liquid;
mod mc_table;

use serde_json::Value;
use std::collections::BTreeMap;
use tiny_skia::{
    Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform,
};

const SS: usize = 2;
const NEAR: f64 = 1.0;

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l == 0.0 {
        [0.0; 3]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}
fn v3(v: &Value) -> V3 {
    [
        v[0].as_f64().unwrap_or(0.0),
        v[1].as_f64().unwrap_or(0.0),
        v[2].as_f64().unwrap_or(0.0),
    ]
}

struct Camera {
    eye: V3,
    right: V3,
    down: V3,
    forward: V3,
    focal: f64,
    cx: f64,
    cy: f64,
}

struct Mesh {
    verts: Vec<V3>,
    faces: Vec<Vec<usize>>,
    double_sided: bool,
    fill: Color,
    outline: Option<Color>,
    width: f64,
    opacity: f64,
}

struct Light {
    dir: V3,
    ambient: f64,
    bands: u32,
    crease: f64,
    smooth: f64,
}

/// Where a camera-space point lands on the supersampled buffer.
struct Screen {
    cam: Camera,
    tf: Transform,
}

impl Screen {
    fn to_camera(&self, p: V3) -> V3 {
        let d = sub(p, self.cam.eye);
        [
            dot(d, self.cam.right),
            dot(d, self.cam.down),
            dot(d, self.cam.forward),
        ]
    }
    /// Pixel (in the supersampled buffer) of a camera-space point with z > 0.
    fn project(&self, c: V3) -> [f64; 2] {
        let (x, y) = (
            self.cam.cx + self.cam.focal * c[0] / c[2],
            self.cam.cy + self.cam.focal * c[1] / c[2],
        );
        let t = &self.tf;
        let (px, py) = (
            t.sx as f64 * x + t.kx as f64 * y + t.tx as f64,
            t.ky as f64 * x + t.sy as f64 * y + t.ty as f64,
        );
        [px * SS as f64, py * SS as f64]
    }
}

/// A convex polygon in camera space clipped to `z >= NEAR` (Sutherland-Hodgman).
fn clip_near(poly: &[V3]) -> Vec<V3> {
    let mut out = vec![];
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ina, inb) = (a[2] >= NEAR, b[2] >= NEAR);
        if ina {
            out.push(a);
        }
        if ina != inb {
            let t = (NEAR - a[2]) / (b[2] - a[2]);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, NEAR]);
        }
    }
    out
}

struct Buffers {
    w: usize,
    h: usize,
    depth: Vec<f32>,
    rgba: Vec<[f32; 4]>, // premultiplied
}

impl Buffers {
    fn triangle(
        &mut self,
        p: [[f64; 2]; 3],
        z: [f64; 3],
        normals: [V3; 3],
        base: [f32; 4],
        light: &Light,
    ) {
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let min_x = p
            .iter()
            .map(|q| q[0])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as usize;
        let max_x = (p
            .iter()
            .map(|q| q[0])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .max(0.0) as usize)
            .min(self.w.saturating_sub(1));
        let min_y = p
            .iter()
            .map(|q| q[1])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as usize;
        let max_y = (p
            .iter()
            .map(|q| q[1])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .max(0.0) as usize)
            .min(self.h.saturating_sub(1));
        let inv_z = [1.0 / z[0], 1.0 / z[1], 1.0 / z[2]];
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let w0 = ((p[1][0] - cx) * (p[2][1] - cy) - (p[1][1] - cy) * (p[2][0] - cx)) / area;
                let w1 = ((p[2][0] - cx) * (p[0][1] - cy) - (p[2][1] - cy) * (p[0][0] - cx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let (a0, a1, a2) = (w0 * inv_z[0], w1 * inv_z[1], w2 * inv_z[2]);
                let zz = (1.0 / (a0 + a1 + a2)) as f32;
                let i = y * self.w + x;
                if zz < self.depth[i] {
                    self.depth[i] = zz;
                    // the normal, interpolated with perspective correction, then lit per pixel
                    let k = 1.0 / (a0 + a1 + a2);
                    let n = unit([
                        (a0 * normals[0][0] + a1 * normals[1][0] + a2 * normals[2][0]) * k,
                        (a0 * normals[0][1] + a1 * normals[1][1] + a2 * normals[2][1]) * k,
                        (a0 * normals[0][2] + a1 * normals[1][2] + a2 * normals[2][2]) * k,
                    ]);
                    let s = shade(light, n) as f32;
                    self.rgba[i] = [
                        base[0] * s * base[3],
                        base[1] * s * base[3],
                        base[2] * s * base[3],
                        base[3],
                    ];
                }
            }
        }
    }
}

fn shade(light: &Light, n: V3) -> f64 {
    let lambert = dot(n, [-light.dir[0], -light.dir[1], -light.dir[2]]).max(0.0);
    let q = if light.bands >= 2 {
        let b = light.bands as f64;
        ((lambert * b).floor().min(b - 1.0)) / (b - 1.0)
    } else {
        lambert
    };
    light.ambient + (1.0 - light.ambient) * q
}

/// Draw every visible mesh of the snapshot into `pm` (one z-buffer for all of them).
pub fn draw_meshes(pm: &mut Pixmap, snapshot: &Value, view: &View) {
    let c = &snapshot["camera3"];
    if !c.is_object() {
        return;
    }
    let cam = Camera {
        eye: v3(&c["eye"]),
        right: v3(&c["right"]),
        down: v3(&c["down"]),
        forward: v3(&c["forward"]),
        focal: c["focal"].as_f64().unwrap_or(1000.0),
        cx: c["cx"].as_f64().unwrap_or(0.0),
        cy: c["cy"].as_f64().unwrap_or(0.0),
    };
    let l = &snapshot["light3"];
    let light = Light {
        dir: unit(v3(&l["dir"])),
        ambient: l["ambient"].as_f64().unwrap_or(0.35),
        bands: l["bands"].as_u64().unwrap_or(3) as u32,
        crease: l["crease"].as_f64().unwrap_or(40.0),
        smooth: l["smooth"].as_f64().unwrap_or(0.0),
    };
    let mut meshes = vec![];
    for sh in snapshot["shapes"].as_array().into_iter().flatten() {
        if sh["visible"] == Value::Bool(false) || !sh["mesh"].is_object() {
            continue;
        }
        let m = &sh["mesh"];
        meshes.push(Mesh {
            verts: m["verts"]
                .as_array()
                .into_iter()
                .flatten()
                .map(v3)
                .collect(),
            faces: m["faces"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| {
                    f.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|i| i.as_u64())
                        .map(|i| i as usize)
                        .collect()
                })
                .collect(),
            double_sided: m["double_sided"] == Value::Bool(true),
            fill: m["fill"]
                .as_str()
                .and_then(parse_hex)
                .unwrap_or(Color::WHITE),
            outline: m["outline"].as_str().and_then(parse_hex),
            width: m["width"].as_f64().unwrap_or(0.0),
            opacity: sh["opacity"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0),
        });
    }
    let has_liquid = snapshot["fluids3"]
        .as_array()
        .is_some_and(|f| !f.is_empty());
    if meshes.is_empty() && !has_liquid {
        return;
    }
    let (w, h) = (pm.width() as usize * SS, pm.height() as usize * SS);
    let screen = Screen { cam, tf: view.tf };
    let mut buf = Buffers {
        w,
        h,
        depth: vec![f32::INFINITY; w * h],
        rgba: vec![[0.0; 4]; w * h],
    };

    // faces: cull, shade, clip, rasterize
    let mut facing: Vec<Vec<bool>> = vec![];
    let mut normals: Vec<Vec<V3>> = vec![];
    for m in &meshes {
        let mut fc = vec![];
        let mut ns = vec![];
        for f in &m.faces {
            if f.len() < 3 || f.iter().any(|i| *i >= m.verts.len()) {
                fc.push(false);
                ns.push([0.0; 3]);
                continue;
            }
            let (a, b, cc) = (m.verts[f[0]], m.verts[f[1]], m.verts[f[2]]);
            let n = unit(cross(sub(b, a), sub(cc, a)));
            let towards = dot(n, sub(screen.cam.eye, a));
            fc.push(towards > 0.0);
            ns.push(n);
        }
        facing.push(fc);
        normals.push(ns);
    }
    let smooth_cos = light.smooth.to_radians().cos();
    for (mi, m) in meshes.iter().enumerate() {
        // the faces around each vertex, for normals that are smooth across gentle edges
        let mut around: Vec<Vec<usize>> = vec![vec![]; m.verts.len()];
        for (fi, f) in m.faces.iter().enumerate() {
            if f.len() >= 3 && f.iter().all(|i| *i < m.verts.len()) {
                for &v in f {
                    around[v].push(fi);
                }
            }
        }
        for (fi, f) in m.faces.iter().enumerate() {
            if f.len() < 3 || f.iter().any(|i| *i >= m.verts.len()) {
                continue;
            }
            let flip = !facing[mi][fi];
            if flip && !m.double_sided {
                continue;
            }
            let sign = if flip { -1.0 } else { 1.0 };
            let fnorm = normals[mi][fi];
            // a normal per corner: the average of the faces around it that are within the smoothing angle
            let corner: Vec<V3> = f
                .iter()
                .map(|&v| {
                    let mut acc = fnorm;
                    if light.smooth > 0.0 {
                        acc = [0.0; 3];
                        for &g in &around[v] {
                            if dot(fnorm, normals[mi][g]) >= smooth_cos {
                                let ng = normals[mi][g];
                                acc = [acc[0] + ng[0], acc[1] + ng[1], acc[2] + ng[2]];
                            }
                        }
                        acc = unit(acc);
                    }
                    [acc[0] * sign, acc[1] * sign, acc[2] * sign]
                })
                .collect();
            let a = (m.fill.alpha() as f64 * m.opacity) as f32;
            let base = [m.fill.red(), m.fill.green(), m.fill.blue(), a];
            // clip the polygon to the near plane, carrying the corner normals along
            let cam: Vec<V3> = f.iter().map(|i| screen.to_camera(m.verts[*i])).collect();
            let mut poly: Vec<(V3, V3)> = vec![];
            for i in 0..cam.len() {
                let (p, q) = (i, (i + 1) % cam.len());
                let (ina, inb) = (cam[p][2] >= NEAR, cam[q][2] >= NEAR);
                if ina {
                    poly.push((cam[p], corner[p]));
                }
                if ina != inb {
                    let t = (NEAR - cam[p][2]) / (cam[q][2] - cam[p][2]);
                    let pos = [
                        cam[p][0] + (cam[q][0] - cam[p][0]) * t,
                        cam[p][1] + (cam[q][1] - cam[p][1]) * t,
                        NEAR,
                    ];
                    let nrm = unit([
                        corner[p][0] + (corner[q][0] - corner[p][0]) * t,
                        corner[p][1] + (corner[q][1] - corner[p][1]) * t,
                        corner[p][2] + (corner[q][2] - corner[p][2]) * t,
                    ]);
                    poly.push((pos, nrm));
                }
            }
            if poly.len() < 3 {
                continue;
            }
            let pts: Vec<[f64; 2]> = poly.iter().map(|(c, _)| screen.project(*c)).collect();
            for k in 1..poly.len() - 1 {
                buf.triangle(
                    [pts[0], pts[k], pts[k + 1]],
                    [poly[0].0[2], poly[k].0[2], poly[k + 1].0[2]],
                    [poly[0].1, poly[k].1, poly[k + 1].1],
                    base,
                    &light,
                );
            }
        }
    }

    // the picture, averaged down from the supersampled buffer
    let (ow, oh) = (pm.width() as usize, pm.height() as usize);
    let mut big = Pixmap::new(w as u32, h as u32).expect("a pixmap");
    for (i, p) in buf.rgba.iter().enumerate() {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        let a = q(p[3]);
        let px = &mut big.data_mut()[i * 4..i * 4 + 4];
        px.copy_from_slice(&[q(p[0]).min(a), q(p[1]).min(a), q(p[2]).min(a), a]);
    }

    // outlines, drawn into the big picture where the edge is not hidden by a nearer surface
    for (mi, m) in meshes.iter().enumerate() {
        let (Some(oc), true) = (m.outline, m.width > 0.0) else {
            continue;
        };
        let mut edges: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        for (fi, f) in m.faces.iter().enumerate() {
            if f.len() < 3 || f.iter().any(|i| *i >= m.verts.len()) {
                continue;
            }
            for k in 0..f.len() {
                let (a, b) = (f[k], f[(k + 1) % f.len()]);
                edges.entry((a.min(b), a.max(b))).or_default().push(fi);
            }
        }
        let crease_cos = light.crease.to_radians().cos();
        let mut paint = Paint::default();
        paint.set_color(oc);
        paint.anti_alias = true;
        let scale = (view.tf.sx.hypot(view.tf.ky)) as f64;
        let stroke = Stroke {
            width: (m.width * scale * SS as f64) as f32,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        for ((a, b), fs) in &edges {
            let silhouette = matches!(fs.as_slice(), [f, g] if !m.double_sided && facing[mi][*f] != facing[mi][*g]);
            let draw = match fs.as_slice() {
                [f] => facing[mi][*f] || m.double_sided,
                [f, g] => {
                    let (ff, fg) = (facing[mi][*f], facing[mi][*g]);
                    if m.double_sided {
                        dot(normals[mi][*f], normals[mi][*g]) < crease_cos
                    } else if ff != fg {
                        true
                    } else {
                        ff && dot(normals[mi][*f], normals[mi][*g]) < crease_cos
                    }
                }
                _ => false,
            };
            if !draw {
                continue;
            }
            let seg = clip_near(&[screen.to_camera(m.verts[*a]), screen.to_camera(m.verts[*b])]);
            // a two point "polygon" clipped to the near plane is the clipped segment (or a point / nothing)
            let (p0, p1) = match seg.len() {
                2 | 3 => (seg[0], seg[1]),
                _ => continue,
            };
            let (s0, s1) = (screen.project(p0), screen.project(p1));
            let len = ((s1[0] - s0[0]).powi(2) + (s1[1] - s0[1]).powi(2)).sqrt();
            let steps = (len.ceil() as usize).max(1);
            let mut run: Vec<[f64; 2]> = vec![];
            let flush = |run: &mut Vec<[f64; 2]>, big: &mut Pixmap| {
                if run.len() >= 2 {
                    let mut pb = PathBuilder::new();
                    pb.move_to(run[0][0] as f32, run[0][1] as f32);
                    for q in &run[1..] {
                        pb.line_to(q[0] as f32, q[1] as f32);
                    }
                    if let Some(path) = pb.finish() {
                        big.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                    }
                }
                run.clear();
            };
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                let sp = [s0[0] + (s1[0] - s0[0]) * t, s0[1] + (s1[1] - s0[1]) * t];
                // perspective-correct depth along the segment: 1/z is linear on the screen
                let z = 1.0 / ((1.0 - t) / p0[2] + t / p1[2]);
                let (ix, iy) = (sp[0].floor() as i64, sp[1].floor() as i64);
                let visible = ix >= 0 && iy >= 0 && (ix as usize) < w && (iy as usize) < h && {
                    // look at the neighbourhood: the edge sits on the surface it bounds, so allow a margin
                    let mut near = f64::INFINITY;
                    for dy in -1i64..=1 {
                        for dx in -1i64..=1 {
                            let (x, y) = (ix + dx, iy + dy);
                            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                                near = near.min(buf.depth[y as usize * w + x as usize] as f64);
                            }
                        }
                    }
                    z <= near
                        + if silhouette {
                            0.03 * z + 5.0
                        } else {
                            0.012 * z + 2.0
                        }
                };
                if visible {
                    run.push(sp);
                } else {
                    flush(&mut run, &mut big);
                }
            }
            flush(&mut run, &mut big);
        }
    }

    // liquids go over everything opaque (and its outlines), tinted by how deep the water is
    if has_liquid {
        liquid::composite(
            &mut big,
            &buf.depth,
            w,
            h,
            &screen,
            &light,
            &snapshot["fluids3"],
            0,
        );
    }

    // average SS x SS blocks (premultiplied) and draw the result over the scene
    let mut out = Pixmap::new(ow as u32, oh as u32).expect("a pixmap");
    {
        let src = big.data();
        let dst = out.data_mut();
        for y in 0..oh {
            for x in 0..ow {
                let mut acc = [0u32; 4];
                for sy in 0..SS {
                    for sx in 0..SS {
                        let i = ((y * SS + sy) * w + x * SS + sx) * 4;
                        for k in 0..4 {
                            acc[k] += src[i + k] as u32;
                        }
                    }
                }
                let o = (y * ow + x) * 4;
                for k in 0..4 {
                    dst[o + k] = ((acc[k] + (SS * SS / 2) as u32) / (SS * SS) as u32) as u8;
                }
            }
        }
    }
    pm.draw_pixmap(
        0,
        0,
        out.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// `--debug-physics` for 3D worlds: contacts are red dots with a normal line, joints blue lines
/// between their anchors. Drawn over the finished picture, without hiding.
pub fn draw_physics3(pm: &mut Pixmap, snapshot: &Value, tf: Transform) {
    let (c, phys) = (&snapshot["camera3"], &snapshot["physics3"]);
    if !c.is_object() || !phys.is_object() {
        return;
    }
    let s = Screen {
        cam: Camera {
            eye: v3(&c["eye"]),
            right: v3(&c["right"]),
            down: v3(&c["down"]),
            forward: v3(&c["forward"]),
            focal: c["focal"].as_f64().unwrap_or(1000.0),
            cx: c["cx"].as_f64().unwrap_or(0.0),
            cy: c["cy"].as_f64().unwrap_or(0.0),
        },
        tf,
    };
    // the picture itself is not supersampled: undo the factor `project` applies
    let at = |p: V3| -> Option<(f32, f32)> {
        let cp = s.to_camera(p);
        if cp[2] < NEAR {
            return None;
        }
        let q = s.project(cp);
        Some((q[0] as f32 / SS as f32, q[1] as f32 / SS as f32))
    };
    let mut red = Paint::default();
    red.set_color(Color::from_rgba8(0xe0, 0x30, 0x30, 255));
    red.anti_alias = true;
    let mut blue = Paint::default();
    blue.set_color(Color::from_rgba8(0x20, 0x60, 0xd0, 255));
    blue.anti_alias = true;
    let line = Stroke {
        width: 2.0,
        ..Stroke::default()
    };
    let ident = Transform::identity();
    for ct in phys["contacts"].as_array().into_iter().flatten() {
        let (p, n) = (v3(&ct["point"]), v3(&ct["normal"]));
        let tip = [p[0] + n[0] * 30.0, p[1] + n[1] * 30.0, p[2] + n[2] * 30.0];
        if let (Some((x0, y0)), Some((x1, y1))) = (at(p), at(tip)) {
            let mut pb = PathBuilder::new();
            pb.move_to(x0, y0);
            pb.line_to(x1, y1);
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &red, &line, ident, None);
            }
            if let Some(dot) = PathBuilder::from_circle(x0, y0, 3.0) {
                pm.fill_path(&dot, &red, tiny_skia::FillRule::Winding, ident, None);
            }
        }
    }
    // soft body pins
    for b in snapshot["soft3"]["bodies"].as_array().into_iter().flatten() {
        for p in b["pins"].as_array().into_iter().flatten() {
            if let Some((x, y)) = at(v3(p)) {
                let mut pb = PathBuilder::new();
                pb.push_rect(
                    tiny_skia::Rect::from_xywh(x - 3.0, y - 3.0, 6.0, 6.0)
                        .unwrap_or(tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
                );
                if let Some(path) = pb.finish() {
                    pm.stroke_path(&path, &blue, &line, ident, None);
                }
            }
        }
    }
    for j in phys["joints"].as_array().into_iter().flatten() {
        if let (Some((ax, ay)), Some((bx, by))) = (at(v3(&j["a"])), at(v3(&j["b"]))) {
            let mut pb = PathBuilder::new();
            pb.move_to(ax, ay);
            pb.line_to(bx, by);
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &blue, &line, ident, None);
            }
            if let Some(dot) = PathBuilder::from_circle(ax, ay, 4.0) {
                pm.stroke_path(&dot, &blue, &line, ident, None);
            }
        }
    }
}
