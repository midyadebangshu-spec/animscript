//! Renders a FrameSnapshot to an image. This crate contains no shape logic: it only fills the
//! faces and strokes the edges that the runtime already resolved into the snapshot.
use serde_json::Value;
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

/// Foam and spray: small white specks that fade with age.
fn draw_foam(pm: &mut Pixmap, fl: &Value, scale: f32) {
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
        let radius = ((0.3 * spacing) as f32 * (1.0 - 0.5 * t)).max(1.0) * scale;
        if let Some(c) = PathBuilder::from_circle(x * scale, y * scale, radius) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// The foam of the fluids on one side (`behind` or in front), drawn after their surfaces.
fn draw_fluid_foam(pm: &mut Pixmap, fluids: &Value, scale: f32, behind: bool) {
    for fl in fluids.as_array().into_iter().flatten() {
        if fl["behind"].as_bool().unwrap_or(false) == behind {
            draw_foam(pm, fl, scale);
        }
    }
}

/// Draw the fluids of a snapshot: `blob` rasterises a smooth density field of the particles
/// (shaded from the surface colour to the deep colour by depth under the surface), `dots` draws
/// overlapping circles. Only `behind` or only front fluids are drawn, as asked.
fn draw_fluids(pm: &mut Pixmap, fluids: &Value, scale: f32, behind: bool) {
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
                if let Some(circle) = PathBuilder::from_circle(
                    p.0 as f32 * scale,
                    p.1 as f32 * scale,
                    (0.75 * spacing) as f32 * scale,
                ) {
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
        let r = (smooth * spacing) as f32 * scale;
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
            let (x, y) = (p.0 as f32 * scale, p.1 as f32 * scale);
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
            let (cx, cy) = (p.0 as f32 * scale, p.1 as f32 * scale);
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
fn draw_physics(pm: &mut Pixmap, phys: &Value, soft: &Value, tf: Transform) {
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
    let tf = Transform::from_scale(scale, scale);
    draw_fluids(&mut pm, &snapshot["fluids"], scale, true);
    draw_fluid_foam(&mut pm, &snapshot["fluids"], scale, true);
    for sh in snapshot["shapes"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[])
    {
        if sh["visible"] == Value::Bool(false) {
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
    draw_fluids(&mut pm, &snapshot["fluids"], scale, false);
    draw_fluid_foam(&mut pm, &snapshot["fluids"], scale, false);
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
    if debug_physics {
        draw_physics(&mut pm, &snapshot["physics"], &snapshot["soft"], tf);
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
