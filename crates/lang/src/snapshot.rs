//! FrameSnapshots (spec §14.2) and the snapshot hash (spec §13.3).
use crate::interp::Interp;
use animscript_core::*;
use serde_json::{json, Map, Value as J};
use std::collections::BTreeMap;

pub const QUANTUM: f64 = 1e-6;

fn hex(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}
fn pt(p: &P) -> J {
    json!([p[0], p[1]])
}

/// The complete description of one tick. Vertex positions and curve handles are fully resolved here,
/// so a renderer only fills faces and strokes edges: it contains no shape logic.
pub fn build_snapshot(
    it: &Interp,
    scene: &str,
    tick: i64,
    project_tick: i64,
    resolved: &BTreeMap<String, BTreeMap<u32, P>>,
) -> J {
    let mut order: Vec<&String> = it.shape_order.iter().collect();
    order.sort_by_key(|n| it.shapes[*n].z); // stable: declaration order breaks ties
    let shapes: Vec<J> = order
        .iter()
        .map(|n| {
            let sh = &it.shapes[*n];
            let ov = resolved.get(*n);
            let vertices: Vec<J> = sh
                .live_ids(Kind::V)
                .into_iter()
                .map(|i| {
                    let p = ov.and_then(|m| m.get(&i).copied()).unwrap_or_else(|| sh.pos(i));
                    json!([i, p[0], p[1]])
                })
                .collect();
            let edges: Vec<J> = sh
                .live_ids(Kind::E)
                .into_iter()
                .map(|i| {
                    let ed = &sh.edges[&i];
                    let pts = sh.edge_points_with(i, ov);
                    let kind = match ed.curve {
                        Curve::Straight => "straight",
                        Curve::Quad(_) => "quadratic",
                        Curve::Cubic(..) => "cubic",
                    };
                    json!({"id": i, "a": ed.a, "b": ed.b, "kind": kind, "pts": pts.iter().map(pt).collect::<Vec<_>>()})
                })
                .collect();
            let faces: Vec<J> = sh
                .live_ids(Kind::F)
                .into_iter()
                .map(|i| {
                    let segs: Vec<J> = sh.face_segments_with(i, ov).iter().map(|s| J::Array(s.iter().map(pt).collect())).collect();
                    json!({"id": i, "segments": segs})
                })
                .collect();
            json!({
                "id": n, "z": sh.z, "visible": sh.visible,
                "fill": sh.fill.map(hex), "stroke": sh.stroke.map(hex), "stroke_width": sh.stroke_width,
                "vertices": vertices, "edges": edges, "faces": faces,
            })
        })
        .collect();
    let entities: Vec<J> = it
        .entity_order
        .iter()
        .map(|n| {
            let flags: Map<String, J> = it.entities[n]
                .flags
                .iter()
                .map(|(k, v)| (k.clone(), J::Bool(*v)))
                .collect();
            json!({"id": n, "flags": flags})
        })
        .collect();
    json!({
        "tick": tick, "project_tick": project_tick, "scene": scene,
        "canvas": [it.canvas.0, it.canvas.1],
        "background": it.background.map(hex),
        "camera": {"position": [it.canvas.0 as f64 / 2.0, it.canvas.1 as f64 / 2.0], "zoom": 1.0, "rotation": 0.0},
        "entities": entities, "shapes": shapes,
    })
}

fn canon(v: &J, out: &mut String) {
    match v {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Number(n) => {
            if n.is_f64() {
                // quantise spatial values so hashes agree across machines (spec §6.11)
                let q = (n.as_f64().unwrap() / QUANTUM).round_ties_even() as i64;
                out.push_str(&format!("q{q}"));
            } else {
                out.push_str(&n.to_string());
            }
        }
        J::String(s) => out.push_str(&serde_json::to_string(s).unwrap()),
        J::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canon(x, out);
            }
            out.push(']');
        }
        J::Object(m) => {
            out.push('{');
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                canon(&m[*k], out);
            }
            out.push('}');
        }
    }
}

/// 64-bit FNV-1a over the canonical, quantised encoding of a snapshot.
pub fn snapshot_hash(v: &J) -> u64 {
    let mut s = String::new();
    canon(v, &mut s);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
