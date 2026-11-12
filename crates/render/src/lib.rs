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
    for sh in snapshot["shapes"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[])
    {
        if sh["visible"] == Value::Bool(false) {
            continue;
        }
        if let Some(color) = sh["fill"].as_str().and_then(parse_hex) {
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            for face in sh["faces"].as_array().into_iter().flatten() {
                if let Some(path) = face_path(face) {
                    pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
                }
            }
        }
        if let Some(color) = sh["stroke"].as_str().and_then(parse_hex) {
            let mut paint = Paint::default();
            paint.set_color(color);
            paint.anti_alias = true;
            let stroke = Stroke {
                width: sh["stroke_width"].as_f64().unwrap_or(2.0) as f32,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            for edge in sh["edges"].as_array().into_iter().flatten() {
                if let Some(path) = edge_path(edge) {
                    pm.stroke_path(&path, &paint, &stroke, tf, None);
                }
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
    Ok(pm)
}

pub fn render_png(
    snapshot: &Value,
    path: &std::path::Path,
    scale: f32,
    debug: bool,
) -> Result<(), String> {
    render(snapshot, scale, debug)?
        .save_png(path)
        .map_err(|e| e.to_string())
}
