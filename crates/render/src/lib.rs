//! Renders a FrameSnapshot to an image. This crate contains no shape logic: it only fills the
//! faces and strokes the edges that the runtime already resolved into the snapshot.
use serde_json::Value;

pub mod bvh;
pub mod light;
mod mesh3;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform,
};

fn parse_hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let b: Vec<u8> = (0..h.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect();
    match b.len() {
        3 => Some(Color::from_rgba8(b[0], b[1], b[2], 255)),
        4 => Some(Color::from_rgba8(b[0], b[1], b[2], b[3])),
        _ => None,
    }
}

fn pt(v: &Value) -> (f32, f32) {
    (
        v[0].as_f64().unwrap_or(0.0) as f32,
        v[1].as_f64().unwrap_or(0.0) as f32,
    )
}

/// Append the curve given by 2 (line), 3 (quadratic) or 4 (cubic) control points; assumes the pen is at the first one.
fn segment_to(pb: &mut PathBuilder, pts: &[Value]) {
    let p: Vec<(f32, f32)> = pts.iter().map(pt).collect();
    match p.len() {
        2 => pb.line_to(p[1].0, p[1].1),
        3 => pb.quad_to(p[1].0, p[1].1, p[2].0, p[2].1),
        4 => pb.cubic_to(p[1].0, p[1].1, p[2].0, p[2].1, p[3].0, p[3].1),
        _ => {}
    }
}

fn face_path(face: &Value) -> Option<Path> {
    let segs = face["segments"].as_array()?;
    let first = segs.first()?.as_array()?.first()?;
    let mut pb = PathBuilder::new();
    let (x, y) = pt(first);
    pb.move_to(x, y);
    for s in segs {
        segment_to(&mut pb, s.as_array()?);
    }
    pb.close();
    pb.finish()
}

fn edge_path(edge: &Value) -> Option<Path> {
    let pts = edge["pts"].as_array()?;
    let mut pb = PathBuilder::new();
    let (x, y) = pt(pts.first()?);
    pb.move_to(x, y);
    segment_to(&mut pb, pts);
    pb.finish()
}

/// Draw one snapshot. `scale` multiplies the canvas size; `debug` adds a dot on every vertex.
pub fn render(snapshot: &Value, scale: f32, debug: bool) -> Result<Pixmap, String> {
    render_with(snapshot, scale, debug, false)
}

/// How canvas points land on the picture: the camera (centre, zoom, rotation) and the output scale. `tf` maps
/// points; `k` multiplies lengths (radii) for the drawing that works in pixels.
#[derive(Clone, Copy)]
struct View {
    tf: Transform,
    k: f32,
}

impl View {
    fn pt(&self, x: f32, y: f32) -> (f32, f32) {
        let t = &self.tf;
        (t.sx * x + t.kx * y + t.tx, t.ky * x + t.sy * y + t.ty)
    }
}

/// The view of a camera object `{position, zoom, rotation}` (turns) on a canvas of `cw` x `ch`, at output `scale`.
fn camera_view(cam: &Value, cw: f32, ch: f32, scale: f32) -> View {
    let pos = (
        cam["position"][0].as_f64().unwrap_or(cw as f64 / 2.0) as f32,
        cam["position"][1].as_f64().unwrap_or(ch as f64 / 2.0) as f32,
    );
    let zoom = cam["zoom"].as_f64().unwrap_or(1.0).clamp(1.0, 8.0) as f32;
    let rot = cam["rotation"].as_f64().unwrap_or(0.0) as f32 * 360.0;
    let tf = Transform::from_translate(-pos.0, -pos.1)
        .post_scale(zoom, zoom)
        .post_rotate(-rot)
        .post_translate(cw / 2.0, ch / 2.0)
        .post_scale(scale, scale);
    View {
        tf,
        k: scale * zoom,
    }
}

/// The points of a stroke up to `progress` (0 to 1) of its length, the last one interpolated.
fn partial_stroke(points: &[Value], progress: f64) -> Vec<animscript_brush::Point> {
    let pts: Vec<[f64; 6]> = points
        .iter()
        .map(|p| {
            let g = |i: usize, d: f64| p[i].as_f64().unwrap_or(d);
            [
                g(0, 0.0),
                g(1, 0.0),
                g(2, 0.7),
                g(3, 0.01),
                g(4, 0.0),
                g(5, 0.0),
            ]
        })
        .collect();
    let total: f64 = pts
        .windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum();
    let target = total * progress.clamp(0.0, 1.0);
    let mut out: Vec<[f64; 6]> = vec![];
    let mut run = 0.0;
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            out.push(*p);
            continue;
        }
        let q = pts[i - 1];
        let seg = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        if run + seg <= target + 1e-9 {
            out.push(*p);
            run += seg;
        } else {
            let f = if seg > 0.0 { (target - run) / seg } else { 0.0 };
            if f > 0.0 {
                let mut m = [0.0; 6];
                for k in 0..6 {
                    m[k] = q[k] + (p[k] - q[k]) * f;
                }
                m[3] = p[3] * f;
                out.push(m);
            }
            break;
        }
    }
    if progress <= 0.0 {
        return vec![];
    }
    out.iter()
        .map(|p| animscript_brush::Point {
            x: p[0] as f32,
            y: p[1] as f32,
            pressure: p[2] as f32,
            xtilt: p[4] as f32,
            ytilt: p[5] as f32,
            dt: p[3],
        })
        .collect()
}

fn blend_mode(name: &str) -> tiny_skia::BlendMode {
    use tiny_skia::BlendMode as B;
    match name {
        "multiply" => B::Multiply,
        "screen" => B::Screen,
        "overlay" => B::Overlay,
        "darken" => B::Darken,
        "lighten" => B::Lighten,
        "color_dodge" => B::ColorDodge,
        "color_burn" => B::ColorBurn,
        "hard_light" => B::HardLight,
        "soft_light" => B::SoftLight,
        "difference" => B::Difference,
        "exclusion" => B::Exclusion,
        "add" => B::Plus,
        _ => B::SourceOver,
    }
}

thread_local! {
    /// Painted layers by content: replaying every stroke of a layer is slow, but a layer that only moves, fades or
    /// is shown again has the same strokes, so its picture is reused and only blitted through the new transform.
    static PAINT_CACHE: std::cell::RefCell<std::collections::HashMap<u64, std::rc::Rc<Pixmap>>> = Default::default();
}

/// Replay every stroke of a paint layer with a fresh libmypaint brush onto a transparent picture.
fn replay_layer(p: &Value, pw: u32, ph: u32, res: f64) -> Option<Pixmap> {
    let brushes = p["brushes"].as_array().cloned().unwrap_or_default();
    let mut canvas = animscript_brush::Canvas::new(pw, ph);
    for stroke in p["strokes"].as_array().into_iter().flatten() {
        let Some(spec) = brushes.get(stroke["brush"].as_u64().unwrap_or(0) as usize) else {
            continue;
        };
        let source = spec["source"].as_str().unwrap_or("pencil");
        let brush = match spec["myb"].as_str() {
            Some(text) => animscript_brush::Brush::from_myb(text).ok(),
            None => animscript_brush::Brush::bundled(source),
        };
        let Some(mut brush) = brush else { continue };
        for kv in spec["set"].as_array().into_iter().flatten() {
            if let (Some(k), Some(v)) = (kv[0].as_str(), kv[1].as_f64()) {
                let _ = brush.set(k, v as f32);
            }
        }
        // a higher resolution paints the same stroke on more pixels: the dab radius grows with it
        if let Some(r) = brush.get("radius_logarithmic") {
            let _ = brush.set("radius_logarithmic", r + res.ln() as f32);
        }
        if let Some(c) = spec["color"].as_array() {
            brush.set_color([
                c[0].as_f64().unwrap_or(0.0) as f32,
                c[1].as_f64().unwrap_or(0.0) as f32,
                c[2].as_f64().unwrap_or(0.0) as f32,
            ]);
        }
        let mut pts = partial_stroke(
            stroke["points"]
                .as_array()
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
            stroke["progress"].as_f64().unwrap_or(1.0),
        );
        for q in pts.iter_mut() {
            q.x *= res as f32;
            q.y *= res as f32;
        }
        let symmetry = stroke["symmetry"].as_object().and_then(|y| {
            use animscript_brush::SymmetryKind as K;
            let kind = match y["kind"].as_str()? {
                "vertical" => K::Vertical,
                "horizontal" => K::Horizontal,
                "both" => K::Both,
                "radial" => K::Radial,
                "snowflake" => K::Snowflake,
                _ => return None,
            };
            Some(animscript_brush::Symmetry {
                kind,
                center: [
                    y["center"][0].as_f64()? as f32 * res as f32,
                    y["center"][1].as_f64()? as f32 * res as f32,
                ],
                angle_deg: y["angle"].as_f64().unwrap_or(0.0) as f32,
                lines: y["lines"].as_i64().unwrap_or(2) as i32,
            })
        });
        let opts = animscript_brush::StrokeOpts {
            barrel: stroke["barrel"].as_f64().unwrap_or(0.0) as f32,
            symmetry,
            blend: animscript_brush::Blend::from_name(stroke["blend"].as_str().unwrap_or("normal"))
                .unwrap_or(animscript_brush::Blend::Normal),
            strength: stroke["strength"].as_f64().unwrap_or(1.0) as f32,
        };
        canvas.stroke_with(&mut brush, &pts, &opts);
    }
    let size = tiny_skia::IntSize::from_wh(pw, ph)?;
    Pixmap::from_vec(canvas.rgba8(), size)
}

/// Paint a layer: its strokes are replayed onto a transparent picture, which is then drawn through the shape's
/// transform and the camera. The picture is cached by the content of the layer (brushes, strokes with their
/// progress, size, resolution), so a layer that only moves, fades or turns is not painted again.
fn draw_paint(pm: &mut Pixmap, sh: &Value, view: &View) {
    let p = &sh["paint"];
    let size = (
        p["size"][0].as_u64().unwrap_or(0) as f64,
        p["size"][1].as_u64().unwrap_or(0) as f64,
    );
    let res = p["resolution"].as_f64().unwrap_or(1.0);
    let (pw, ph) = (
        (size.0 * res).round().max(1.0) as u32,
        (size.1 * res).round().max(1.0) as u32,
    );
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (pw, ph, res.to_bits()).hash(&mut h);
        p["brushes"].to_string().hash(&mut h);
        p["strokes"].to_string().hash(&mut h);
        h.finish()
    };
    let cached = PAINT_CACHE.with(|c| c.borrow().get(&key).cloned());
    let layer = match cached {
        Some(l) => l,
        None => {
            let Some(l) = replay_layer(p, pw, ph, res) else {
                return;
            };
            let l = std::rc::Rc::new(l);
            PAINT_CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.len() >= 48 {
                    c.clear();
                }
                c.insert(key, l.clone());
            });
            l
        }
    };
    let t = &sh["transform"];
    let g = |k: &str, i: usize, d: f32| t[k][i].as_f64().map_or(d, |v| v as f32);
    let (pos, scale, piv) = (
        (g("position", 0, 0.0), g("position", 1, 0.0)),
        (g("scale", 0, 1.0), g("scale", 1, 1.0)),
        (g("pivot", 0, 0.0), g("pivot", 1, 0.0)),
    );
    let rot = t["rotation"].as_f64().unwrap_or(0.0) as f32 * 360.0;
    let shape_tf = Transform::from_translate(-piv.0, -piv.1)
        .post_scale(scale.0, scale.1)
        .post_rotate(rot)
        .post_translate(piv.0 + pos.0, piv.1 + pos.1);
    let tf = Transform::from_scale(1.0 / res as f32, 1.0 / res as f32)
        .post_concat(shape_tf)
        .post_concat(view.tf);
    let paint = tiny_skia::PixmapPaint {
        opacity: sh["opacity"].as_f64().unwrap_or(1.0) as f32,
        blend_mode: blend_mode(p["blend"].as_str().unwrap_or("normal")),
        quality: tiny_skia::FilterQuality::Bilinear,
    };
    pm.draw_pixmap(0, 0, (*layer).as_ref(), &paint, tf, None);
}

fn base64_decode(s: &str) -> Vec<u8> {
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
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in s.as_bytes().chunks(4) {
        let vals: Vec<u32> = chunk.iter().filter_map(|c| val(*c)).collect();
        if vals.len() < 2 {
            break;
        }
        let n = vals
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, v)| a | v << (18 - 6 * i));
        out.push((n >> 16) as u8);
        if vals.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if vals.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}

/// A field: its colour-mapped image (RGBA, `iw` x `ih`) stretched over its rectangle, through the camera.
fn draw_field(pm: &mut Pixmap, f: &Value, view: &View) {
    let (Some(iw), Some(ih), Some(b64)) = (f["iw"].as_u64(), f["ih"].as_u64(), f["rgba"].as_str())
    else {
        return;
    };
    let mut bytes = base64_decode(b64);
    if iw == 0 || ih == 0 || bytes.len() != (iw * ih * 4) as usize {
        return;
    }
    for px in bytes.chunks_mut(4) {
        // tiny-skia pictures are premultiplied
        let a = px[3] as u32;
        for c in px.iter_mut().take(3) {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
    let Some(size) = tiny_skia::IntSize::from_wh(iw as u32, ih as u32) else {
        return;
    };
    let Some(img) = Pixmap::from_vec(bytes, size) else {
        return;
    };
    let r = &f["rect"];
    let g = |i: usize| r[i].as_f64().unwrap_or(0.0) as f32;
    let tf = Transform::from_scale(g(2) / iw as f32, g(3) / ih as f32)
        .post_translate(g(0), g(1))
        .post_concat(view.tf);
    let paint = tiny_skia::PixmapPaint {
        opacity: f["opacity"].as_f64().unwrap_or(1.0) as f32,
        blend_mode: blend_mode(f["blend"].as_str().unwrap_or("normal")),
        quality: tiny_skia::FilterQuality::Bilinear,
    };
    pm.draw_pixmap(0, 0, img.as_ref(), &paint, tf, None);
}

/// Foam and spray: small white specks that fade with age.
fn draw_foam(pm: &mut Pixmap, fl: &Value, view: &View) {
    let spacing = fl["spacing"].as_f64().unwrap_or(8.0);
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    for f in fl["foam"].as_array().into_iter().flatten() {
        let (x, y, t) = (
            f[0].as_f64().unwrap_or(0.0) as f32,
            f[1].as_f64().unwrap_or(0.0) as f32,
            f[2].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32,
        );
        paint.set_color(Color::from_rgba(1.0, 1.0, 1.0, (1.0 - t) * 0.95).unwrap_or(Color::WHITE));
        let radius = ((0.3 * spacing) as f32 * (1.0 - 0.5 * t)).max(1.0) * view.k;
        let (x, y) = view.pt(x, y);
        if let Some(c) = PathBuilder::from_circle(x, y, radius) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// A PNG decoded to premultiplied RGBA, or `None` if it cannot be read.
fn load_png(path: &std::path::Path) -> Option<Pixmap> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let px = &buf[..info.buffer_size()];
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for c in match info.color_type {
        png::ColorType::Rgba => px
            .chunks(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect::<Vec<_>>(),
        png::ColorType::Rgb => px.chunks(3).map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => px.iter().map(|g| [*g, *g, *g, 255]).collect(),
    } {
        let a = c[3] as u32;
        let pre = |v: u8| ((v as u32 * a + 127) / 255) as u8;
        rgba.extend_from_slice(&[pre(c[0]), pre(c[1]), pre(c[2]), c[3]]);
    }
    Pixmap::from_vec(rgba, tiny_skia::IntSize::from_wh(w, h)?)
}

/// Draw a textured shape: every triangle `[x, y, u, v] x 3` is filled with the picture through the
/// affine map of its three corners (a missing picture is a magenta placeholder).
fn draw_image(
    pm: &mut Pixmap,
    img: &Value,
    opacity: f32,
    tf: Transform,
    base: &std::path::Path,
    cache: &mut std::collections::HashMap<String, Option<Pixmap>>,
) {
    let src = img["src"].as_str().unwrap_or("");
    let tex: &Option<Pixmap> = cache
        .entry(src.to_string())
        .or_insert_with(|| load_png(&base.join(src)));
    for t in img["tris"].as_array().into_iter().flatten() {
        let p: Vec<[f64; 4]> = t
            .as_array()
            .into_iter()
            .flatten()
            .map(|q| {
                [
                    q[0].as_f64().unwrap_or(0.0),
                    q[1].as_f64().unwrap_or(0.0),
                    q[2].as_f64().unwrap_or(0.0),
                    q[3].as_f64().unwrap_or(0.0),
                ]
            })
            .collect();
        if p.len() != 3 {
            continue;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(p[0][0] as f32, p[0][1] as f32);
        pb.line_to(p[1][0] as f32, p[1][1] as f32);
        pb.line_to(p[2][0] as f32, p[2][1] as f32);
        pb.close();
        let Some(path) = pb.finish() else { continue };
        let mut paint = Paint {
            anti_alias: false,
            ..Paint::default()
        };
        match tex {
            None => paint.set_color(Color::from_rgba8(255, 0, 255, 255)),
            Some(pix) => {
                // solve texture pixels (u * w, v * h) -> canvas (x, y) from the three corners
                let (w, h) = (pix.width() as f64, pix.height() as f64);
                let (a, b, c) = (
                    (p[0][2] * w, p[0][3] * h),
                    (p[1][2] * w, p[1][3] * h),
                    (p[2][2] * w, p[2][3] * h),
                );
                let det = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
                if det.abs() < 1e-12 {
                    continue;
                }
                let solve = |va: f64, vb: f64, vc: f64| {
                    let (dx1, dx2) = (vb - va, vc - va);
                    let m = (dx1 * (c.1 - a.1) - dx2 * (b.1 - a.1)) / det; // d value / d tex x
                    let n = (dx2 * (b.0 - a.0) - dx1 * (c.0 - a.0)) / det; // d value / d tex y
                    (m, n, va - m * a.0 - n * a.1)
                };
                let (sx, kx, tx) = solve(p[0][0], p[1][0], p[2][0]);
                let (ky, sy, ty) = solve(p[0][1], p[1][1], p[2][1]);
                paint.shader = tiny_skia::Pattern::new(
                    pix.as_ref(),
                    tiny_skia::SpreadMode::Pad,
                    tiny_skia::FilterQuality::Bilinear,
                    opacity,
                    Transform::from_row(
                        sx as f32, ky as f32, kx as f32, sy as f32, tx as f32, ty as f32,
                    ),
                );
            }
        }
        if tex.is_none() {
            // placeholder: fully opaque magenta, only as transparent as the shape is
            paint.set_color(Color::from_rgba(1.0, 0.0, 1.0, opacity).unwrap_or(Color::WHITE));
        }
        pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
    }
}

/// The foam of the fluids on one side (`behind` or in front), drawn after their surfaces.
fn draw_fluid_foam(pm: &mut Pixmap, fluids: &Value, view: &View, behind: bool) {
    for fl in fluids.as_array().into_iter().flatten() {
        if fl["behind"].as_bool().unwrap_or(false) == behind {
            draw_foam(pm, fl, view);
        }
    }
}

/// Draw the fluids of a snapshot: `blob` rasterises a smooth density field of the particles
/// (shaded from the surface colour to the deep colour by depth under the surface), `dots` draws
/// overlapping circles. Only `behind` or only front fluids are drawn, as asked.
fn draw_fluids(pm: &mut Pixmap, fluids: &Value, view: &View, behind: bool) {
    for fl in fluids.as_array().into_iter().flatten() {
        if fl["behind"].as_bool().unwrap_or(false) != behind {
            continue;
        }
        let spacing = fl["spacing"].as_f64().unwrap_or(8.0);
        let (c0, c1) = (
            fl["color"]
                .as_str()
                .and_then(parse_hex)
                .unwrap_or(Color::from_rgba8(80, 150, 230, 255)),
            fl["color_end"]
                .as_str()
                .and_then(parse_hex)
                .unwrap_or(Color::from_rgba8(80, 150, 230, 255)),
        );
        let pts: Vec<(f64, f64)> = fl["particles"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| (p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0)))
            .collect();
        if pts.is_empty() {
            continue;
        }
        // depth below the surface: per column (bins one spacing wide) the particles are walked from the
        // top; a gap of more than 2.5 spacings starts a new run (a droplet in the air is its own surface)
        let mut cols: std::collections::BTreeMap<i64, Vec<(f64, usize)>> = Default::default();
        for (i, p) in pts.iter().enumerate() {
            cols.entry((p.0 / spacing).floor() as i64)
                .or_default()
                .push((p.1, i));
        }
        let mut depth = vec![0.0f64; pts.len()];
        for col in cols.values_mut() {
            col.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let (mut top, mut last) = (col[0].0, col[0].0);
            for (y, i) in col.iter() {
                if y - last > 2.5 * spacing {
                    top = *y;
                }
                depth[*i] = ((y - top) / 90.0).clamp(0.0, 1.0); // fully deep after 90 px
                last = *y;
            }
        }
        let mix = |t: f64| -> [f32; 3] {
            let t = t as f32;
            [
                c0.red() + (c1.red() - c0.red()) * t,
                c0.green() + (c1.green() - c0.green()) * t,
                c0.blue() + (c1.blue() - c0.blue()) * t,
            ]
        };
        if fl["render"].as_str() == Some("dots") {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            for (p, d) in pts.iter().zip(&depth) {
                let c = mix(*d);
                paint.set_color(Color::from_rgba(c[0], c[1], c[2], c0.alpha()).unwrap_or(c0));
                let (cx, cy) = view.pt(p.0 as f32, p.1 as f32);
                if let Some(circle) =
                    PathBuilder::from_circle(cx, cy, (0.75 * spacing) as f32 * view.k)
                {
                    pm.fill_path(
                        &circle,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
            continue;
        }
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let smooth = fl["smoothness"].as_f64().unwrap_or(1.4).clamp(0.8, 4.0);
        let r = (smooth * spacing) as f32 * view.k;
        // the field value of a particle in the middle of a perfect lattice: thresholds are relative to
        // it, so the surface sits at the same fraction of a full density whatever the radius
        let lattice: f32 = (-4i32..=4)
            .flat_map(|j| (-4i32..=4).map(move |i| (i, j)))
            .map(|(i, j)| {
                let q = ((i * i + j * j) as f64 / (smooth * smooth)) as f32;
                if q < 1.0 {
                    (1.0 - q) * (1.0 - q)
                } else {
                    0.0
                }
            })
            .sum();
        let r2 = r * r;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for p in &pts {
            let (x, y) = view.pt(p.0 as f32, p.1 as f32);
            x0 = x0.min((x - r).floor() as i32);
            y0 = y0.min((y - r).floor() as i32);
            x1 = x1.max((x + r).ceil() as i32);
            y1 = y1.max((y + r).ceil() as i32);
        }
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(w - 1), y1.min(h - 1));
        if x1 < x0 || y1 < y0 {
            continue;
        }
        let bw = (x1 - x0 + 1) as usize;
        let mut field = vec![0f32; bw * (y1 - y0 + 1) as usize];
        let mut dsum = vec![0f32; field.len()];
        for (p, d) in pts.iter().zip(&depth) {
            let (cx, cy) = view.pt(p.0 as f32, p.1 as f32);
            let (ax, bx) = (
                ((cx - r).floor() as i32).max(x0),
                ((cx + r).ceil() as i32).min(x1),
            );
            let (ay, by) = (
                ((cy - r).floor() as i32).max(y0),
                ((cy + r).ceil() as i32).min(y1),
            );
            for py in ay..=by {
                for px in ax..=bx {
                    let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                    let q = (dx * dx + dy * dy) / r2;
                    if q < 1.0 {
                        let k = (1.0 - q) * (1.0 - q);
                        let i = (py - y0) as usize * bw + (px - x0) as usize;
                        field[i] += k;
                        dsum[i] += k * *d as f32;
                    }
                }
            }
        }
        let stride = pm.width() as usize * 4;
        let data = pm.data_mut();
        for py in y0..=y1 {
            for px in x0..=x1 {
                let i = (py - y0) as usize * bw + (px - x0) as usize;
                let f = field[i] / lattice;
                // a soft edge between 18% and 30% of a full lattice density, low enough that a lone
                // droplet or a thin stream stays about one spacing wide
                let t = ((f - 0.18) / 0.12).clamp(0.0, 1.0);
                let a = t * t * (3.0 - 2.0 * t);
                if a <= 0.0 {
                    continue;
                }
                let mut c = mix((dsum[i] / field[i]) as f64);
                // a light rim just inside the edge, like the meniscus of a real surface
                let rim = 1.0 - ((f - 0.30) / 0.22).clamp(0.0, 1.0);
                for ch in c.iter_mut() {
                    *ch += (1.0 - *ch) * 0.35 * rim;
                }
                let a = a * c0.alpha();
                let o = py as usize * stride + px as usize * 4;
                for (k, ch) in c.iter().enumerate() {
                    let src = ch * a;
                    let dst = data[o + k] as f32 / 255.0;
                    data[o + k] = ((src + dst * (1.0 - a)) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                }
                let dst_a = data[o + 3] as f32 / 255.0;
                data[o + 3] = ((a + dst_a * (1.0 - a)) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Draw the physics section of a snapshot: contact points with their normals (red) and joints as
/// lines between their anchors (blue). Only what the snapshot says; no simulation happens here.
fn draw_physics(pm: &mut Pixmap, phys: &Value, soft: &Value, rigs: &Value, tf: Transform) {
    let xy = |v: &Value| {
        (
            v[0].as_f64().unwrap_or(0.0) as f32,
            v[1].as_f64().unwrap_or(0.0) as f32,
        )
    };
    let mut blue = Paint::default();
    blue.set_color(Color::from_rgba8(0x20, 0x60, 0xd0, 255));
    blue.anti_alias = true;
    let line = Stroke {
        width: 2.0,
        ..Stroke::default()
    };
    for j in phys["joints"].as_array().into_iter().flatten() {
        let ((ax, ay), (bx, by)) = (xy(&j["a"]), xy(&j["b"]));
        let mut pb = PathBuilder::new();
        pb.move_to(ax, ay);
        pb.line_to(bx, by);
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &blue, &line, tf, None);
        }
        if let Some(c) = PathBuilder::from_circle(ax, ay, 4.0) {
            pm.stroke_path(&c, &blue, &line, tf, None);
        }
    }
    for b in soft["bodies"].as_array().into_iter().flatten() {
        for p in b["pins"].as_array().into_iter().flatten() {
            let (x, y) = xy(p);
            let mut pb = PathBuilder::new();
            pb.push_rect(
                tiny_skia::Rect::from_xywh(x - 4.0, y - 4.0, 8.0, 8.0)
                    .unwrap_or(tiny_skia::Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
            );
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &blue, &line, tf, None);
            }
        }
    }
    // armatures: bones are green lines with a circle at the head; ik targets are orange rings
    let mut green = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    green.set_color(Color::from_rgba8(0x20, 0xb0, 0x40, 255));
    let thick = Stroke {
        width: 3.0,
        ..Stroke::default()
    };
    for arm in rigs.as_array().into_iter().flatten() {
        for b in arm["bones"].as_array().into_iter().flatten() {
            let ((hx, hy), (tx, ty)) = (xy(&b["head"]), xy(&b["tip"]));
            let mut pb = PathBuilder::new();
            pb.move_to(hx, hy);
            pb.line_to(tx, ty);
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &green, &thick, tf, None);
            }
            if let Some(c) = PathBuilder::from_circle(hx, hy, 4.0) {
                pm.stroke_path(&c, &green, &line, tf, None);
            }
        }
        let mut orange = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        orange.set_color(Color::from_rgba8(0xf0, 0x90, 0x20, 255));
        for k in arm["ik"].as_array().into_iter().flatten() {
            if k["weight"].as_f64().unwrap_or(0.0) <= 0.0 {
                continue;
            }
            let (x, y) = xy(&k["target"]);
            if let Some(c) = PathBuilder::from_circle(x, y, 6.0) {
                pm.stroke_path(&c, &orange, &line, tf, None);
            }
            if let Some(c) = PathBuilder::from_circle(x, y, 1.5) {
                pm.fill_path(&c, &orange, FillRule::Winding, tf, None);
            }
        }
    }
    let mut red = Paint::default();
    red.set_color(Color::from_rgba8(0xd0, 0x30, 0x30, 255));
    red.anti_alias = true;
    for c in phys["contacts"].as_array().into_iter().flatten() {
        let (x, y) = xy(&c["point"]);
        let (nx, ny) = xy(&c["normal"]);
        if let Some(d) = PathBuilder::from_circle(x, y, 3.0) {
            pm.fill_path(&d, &red, FillRule::Winding, tf, None);
        }
        let mut pb = PathBuilder::new();
        pb.move_to(x, y);
        pb.line_to(x + nx * 14.0, y + ny * 14.0);
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &red, &line, tf, None);
        }
    }
}

/// `render` plus optional overlays: `debug_physics` draws contacts and joints.
pub fn render_with(
    snapshot: &Value,
    scale: f32,
    debug: bool,
    debug_physics: bool,
) -> Result<Pixmap, String> {
    render_assets(
        snapshot,
        scale,
        debug,
        debug_physics,
        std::path::Path::new("."),
    )
}

/// Like `render_with`, loading pictures (PNG files named by the snapshot) relative to `base`. The camera of the
/// snapshot is applied; during a `dissolve` or `fade_through` both camera views are drawn and blended.
pub fn render_assets(
    snapshot: &Value,
    scale: f32,
    debug: bool,
    debug_physics: bool,
    base: &std::path::Path,
) -> Result<Pixmap, String> {
    let tr = &snapshot["transition"];
    if !tr.is_object() {
        return render_view(
            snapshot,
            &snapshot["camera"],
            scale,
            debug,
            debug_physics,
            base,
        );
    }
    let t = tr["t"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    let mut from = render_view(snapshot, &tr["from"], scale, debug, debug_physics, base)?;
    let mut to = render_view(
        snapshot,
        &snapshot["camera"],
        scale,
        debug,
        debug_physics,
        base,
    )?;
    if tr["kind"].as_str() == Some("fade_through") {
        let c = tr["color"]
            .as_str()
            .and_then(parse_hex)
            .unwrap_or(Color::BLACK);
        let col = [c.red(), c.green(), c.blue()];
        // the old view fades to the colour in the first half, the new one comes out of it in the second
        if t < 0.5 {
            blend_towards(&mut from, col, t * 2.0);
            Ok(from)
        } else {
            blend_towards(&mut to, col, 2.0 - t * 2.0);
            Ok(to)
        }
    } else {
        for (a, b) in from.data_mut().iter_mut().zip(to.data()) {
            *a = (*a as f32 * (1.0 - t) + *b as f32 * t + 0.5) as u8;
        }
        Ok(from)
    }
}

/// Mix every pixel towards an opaque colour by `u` (0 leaves the picture, 1 is the colour).
fn blend_towards(pm: &mut Pixmap, col: [f32; 3], u: f32) {
    for px in pm.data_mut().chunks_mut(4) {
        for k in 0..3 {
            px[k] = (px[k] as f32 * (1.0 - u) + col[k] * 255.0 * u + 0.5) as u8;
        }
        px[3] = (px[3] as f32 * (1.0 - u) + 255.0 * u + 0.5) as u8;
    }
}

fn render_view(
    snapshot: &Value,
    cam: &Value,
    scale: f32,
    debug: bool,
    debug_physics: bool,
    base: &std::path::Path,
) -> Result<Pixmap, String> {
    let cw = snapshot["canvas"][0]
        .as_u64()
        .ok_or("snapshot has no canvas")? as f32;
    let ch = snapshot["canvas"][1]
        .as_u64()
        .ok_or("snapshot has no canvas")? as f32;
    let mut pm = Pixmap::new((cw * scale).round() as u32, (ch * scale).round() as u32)
        .ok_or("canvas size is not drawable")?;
    pm.fill(
        snapshot["background"]
            .as_str()
            .and_then(parse_hex)
            .unwrap_or(Color::WHITE),
    );
    let view = camera_view(cam, cw, ch, scale);
    let tf = view.tf;
    let mut textures: std::collections::HashMap<String, Option<Pixmap>> = Default::default();
    draw_fluids(&mut pm, &snapshot["fluids"], &view, true);
    draw_fluid_foam(&mut pm, &snapshot["fluids"], &view, true);
    // fields are drawn in order with the shapes, by z (a field goes before a shape of the same z)
    let mut fields: Vec<&Value> = snapshot["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    fields.sort_by_key(|f| f["z"].as_i64().unwrap_or(0));
    let mut next_field = 0usize;
    let mut meshes_drawn = false;
    for sh in snapshot["shapes"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[])
    {
        while next_field < fields.len()
            && fields[next_field]["z"].as_i64().unwrap_or(0) <= sh["z"].as_i64().unwrap_or(0)
        {
            draw_field(&mut pm, fields[next_field], &view);
            next_field += 1;
        }
        if sh["visible"] == Value::Bool(false) {
            continue;
        }
        if sh["paint"].is_object() {
            draw_paint(&mut pm, sh, &view);
            continue;
        }
        if sh["mesh"].is_object() {
            // all the meshes are drawn together, with one z-buffer, where the first of them comes in the z order
            if !meshes_drawn {
                mesh3::draw_meshes(&mut pm, snapshot, &view);
                meshes_drawn = true;
            }
            continue;
        }
        // every face is filled with its own resolved colour, every edge stroked with its own
        for face in sh["faces"].as_array().into_iter().flatten() {
            let Some(color) = face["fill"].as_str().and_then(parse_hex) else {
                continue;
            };
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            if let Some(path) = face_path(face) {
                pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
            }
        }
        if sh["image"].is_object() {
            let opacity = sh["opacity"].as_f64().unwrap_or(1.0) as f32;
            draw_image(&mut pm, &sh["image"], opacity, tf, base, &mut textures);
        }
        for edge in sh["edges"].as_array().into_iter().flatten() {
            let Some(color) = edge["stroke"].as_str().and_then(parse_hex) else {
                continue;
            };
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            let stroke = Stroke {
                width: edge["stroke_width"].as_f64().unwrap_or(2.0) as f32,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            if let Some(path) = edge_path(edge) {
                pm.stroke_path(&path, &paint, &stroke, tf, None);
            }
        }
        if debug {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(0xd0, 0x30, 0x30, 255));
            paint.anti_alias = true;
            for v in sh["vertices"].as_array().into_iter().flatten() {
                let (x, y) = (
                    v[1].as_f64().unwrap_or(0.0) as f32,
                    v[2].as_f64().unwrap_or(0.0) as f32,
                );
                if let Some(c) = PathBuilder::from_circle(x, y, 3.0) {
                    pm.fill_path(&c, &paint, FillRule::Winding, tf, None);
                }
            }
        }
    }
    draw_fluids(&mut pm, &snapshot["fluids"], &view, false);
    draw_fluid_foam(&mut pm, &snapshot["fluids"], &view, false);
    for em in snapshot["particles"].as_array().into_iter().flatten() {
        for it in em["items"].as_array().into_iter().flatten() {
            let Some(color) = it[3].as_str().and_then(parse_hex) else {
                continue;
            };
            let (x, y, r) = (
                it[0].as_f64().unwrap_or(0.0) as f32,
                it[1].as_f64().unwrap_or(0.0) as f32,
                it[2].as_f64().unwrap_or(0.0) as f32,
            );
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            if let Some(c) = PathBuilder::from_circle(x, y, r.max(0.01)) {
                pm.fill_path(&c, &paint, FillRule::Winding, tf, None);
            }
        }
    }
    // a scene whose only 3D thing is water has no mesh to trigger the 3D pass
    if !meshes_drawn && snapshot["fluids3"].is_array() {
        mesh3::draw_meshes(&mut pm, snapshot, &view);
    }
    while next_field < fields.len() {
        draw_field(&mut pm, fields[next_field], &view);
        next_field += 1;
    }
    if debug_physics {
        draw_physics(
            &mut pm,
            &snapshot["physics"],
            &snapshot["soft"],
            &snapshot["rigs"],
            tf,
        );
        mesh3::draw_physics3(&mut pm, snapshot, tf);
    }
    Ok(pm)
}

pub fn render_png(
    snapshot: &Value,
    path: &std::path::Path,
    scale: f32,
    debug: bool,
) -> Result<(), String> {
    render_png_with(snapshot, path, scale, debug, false)
}

/// Render to a PNG file, loading pictures relative to `base`.
pub fn render_png_assets(
    snapshot: &Value,
    path: &std::path::Path,
    scale: f32,
    debug: bool,
    debug_physics: bool,
    base: &std::path::Path,
) -> Result<(), String> {
    render_assets(snapshot, scale, debug, debug_physics, base)?
        .save_png(path)
        .map_err(|e| e.to_string())
}

pub fn render_png_with(
    snapshot: &Value,
    path: &std::path::Path,
    scale: f32,
    debug: bool,
    debug_physics: bool,
) -> Result<(), String> {
    render_with(snapshot, scale, debug, debug_physics)?
        .save_png(path)
        .map_err(|e| e.to_string())
}
