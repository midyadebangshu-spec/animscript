//! FrameSnapshots (spec §14.2) and the snapshot hash (spec §13.3).
use crate::color::{to_linear, to_srgb8};
use crate::interp::Interp;
use animscript_core::*;
use serde_json::{json, Map, Value as J};
use std::collections::BTreeMap;

pub const QUANTUM: f64 = 1e-6;

use crate::color::hex;
fn pt(p: &P) -> J {
    json!([p[0], p[1]])
}

/// Values the runtime resolved this tick: local vertex positions and shape transforms.
#[derive(Default, Clone, Debug)]
pub struct Resolved {
    pub verts: BTreeMap<String, BTreeMap<u32, P>>,
    pub tf: BTreeMap<String, Transform>,
    /// Animated 3D components per shape: position z, turns about x and y, scale z.
    pub t3: BTreeMap<String, [f64; 4]>,
    /// Physics orientation quaternions `[w, x, y, z]` of mesh bodies.
    pub q3: BTreeMap<String, [f64; 4]>,
    /// Animated mesh vertices (local coordinates): shape, vertex index, position.
    pub mverts: BTreeMap<String, BTreeMap<u32, [f64; 3]>>,
    /// Linear-light colours of faces and edges whose own colour is animated.
    pub fills: BTreeMap<(String, u32), [f64; 4]>,
    pub strokes: BTreeMap<(String, u32), [f64; 4]>,
    pub widths: BTreeMap<(String, u32), f64>,
    pub shape_fill: BTreeMap<String, [f64; 4]>,
    pub shape_stroke: BTreeMap<String, [f64; 4]>,
    pub tint: BTreeMap<String, [f64; 4]>,
    pub shape_width: BTreeMap<String, f64>,
    pub opacity: BTreeMap<String, f64>,
    /// Painted fraction of paint-layer strokes.
    pub stroke_progress: BTreeMap<(String, u32), f64>,
    /// The physics section of the snapshot (contacts, joints, sleep flags), when a world exists.
    pub physics: Option<J>,
    /// The 3D physics section (contacts, joints, sleep flags), when a mesh world exists.
    pub physics3: Option<J>,
    /// Soft meshes (pins, sizes), when any exist.
    pub soft3: Option<J>,
    /// 3D fluids (particles as packed floats), when any exist.
    pub fluids3: Option<J>,
    /// Live particles per emitter, when the program declares emitters.
    pub particles: Option<J>,
    /// Soft bodies: volume, stretch and pinned points, when any exist.
    pub soft: Option<J>,
    /// Fluid particles per fluid, when any are declared.
    pub fluids: Option<J>,
    /// Armatures: every bone's head, tip and angle, and the ik targets, when any are declared.
    pub rigs: Option<J>,
    /// 3D armatures: bone heads, tips and orientations.
    pub rigs3: Option<J>,
}

/// The complete description of one tick. Vertex positions, curve handles and styles are fully resolved
/// here (in world space, after the shape transform), so a renderer only fills faces and strokes edges:
/// it contains no shape logic.
pub fn build_snapshot(
    it: &Interp,
    scene: &str,
    tick: i64,
    project_tick: i64,
    resolved: &Resolved,
) -> J {
    let mut order: Vec<&String> = it.shape_order.iter().collect();
    order.sort_by_key(|n| it.shapes[*n].z); // stable: declaration order breaks ties
    let shapes: Vec<J> = order
        .iter()
        .map(|n| {
            let sh = &it.shapes[*n];
            let ov = resolved.verts.get(*n);
            let tf = resolved.tf.get(*n).copied().unwrap_or_else(|| sh.transform());
            let opacity = resolved.opacity.get(*n).copied().unwrap_or(sh.opacity).clamp(0.0, 1.0);
            let tint = resolved.tint.get(*n).copied().unwrap_or_else(|| to_linear(sh.tint));
            // an element's colour: its own (animated or not), else the shape's (animated or not); then tint and opacity
            let finish = |c: Option<[f64; 4]>| -> J {
                match c {
                    None => J::Null,
                    Some(c) => J::String(hex(to_srgb8([c[0] * tint[0], c[1] * tint[1], c[2] * tint[2], c[3] * tint[3] * opacity]))),
                }
            };
            let shape_fill = resolved.shape_fill.get(*n).copied().or_else(|| sh.fill.map(to_linear));
            let shape_stroke = resolved.shape_stroke.get(*n).copied().or_else(|| sh.stroke.map(to_linear));
            let shape_width = resolved.shape_width.get(*n).copied().unwrap_or(sh.stroke_width);
            let w = |p: &P| pt(&tf.apply(*p));
            let vertices: Vec<J> = sh
                .live_ids(Kind::V)
                .into_iter()
                .map(|i| {
                    let p = tf.apply(ov.and_then(|m| m.get(&i).copied()).unwrap_or_else(|| sh.pos(i)));
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
                    json!({
                        "id": i, "a": ed.a, "b": ed.b, "kind": kind,
                        "stroke": finish(resolved.strokes.get(&((*n).clone(), i)).copied().or_else(|| ed.stroke.map(to_linear)).or(shape_stroke)),
                        "stroke_width": resolved.widths.get(&((*n).clone(), i)).copied().or(ed.width).unwrap_or(shape_width),
                        "pts": pts.iter().map(&w).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let faces: Vec<J> = sh
                .live_ids(Kind::F)
                .into_iter()
                .map(|i| {
                    let segs: Vec<J> = sh.face_segments_with(i, ov).iter().map(|s| J::Array(s.iter().map(&w).collect())).collect();
                    json!({"id": i, "fill": finish(resolved.fills.get(&((*n).clone(), i)).copied().or_else(|| sh.faces[&i].fill.map(to_linear)).or(shape_fill)), "segments": segs})
                })
                .collect();
            let mut shape_json = json!({
                "id": n, "z": sh.z, "visible": sh.visible,
                "fill": sh.fill.map(hex), "stroke": sh.stroke.map(hex), "stroke_width": sh.stroke_width,
                "opacity": opacity, "tint": hex(to_srgb8(tint)),
                "transform": {"position": pt(&tf.position), "scale": pt(&tf.scale), "rotation": tf.rotation, "pivot": pt(&tf.pivot)},
                "vertices": vertices, "edges": edges, "faces": faces,
            });
            if let Some(p) = it.paint_snapshot(n, &resolved.stroke_progress) {
                shape_json["paint"] = p;
            }
            if sh.mesh.is_some() {
                let fill = finish(shape_fill.or(Some(to_linear([200, 200, 208, 255]))));
                shape_json["mesh"] = crate::mesh3d::mesh_json(sh, &tf, resolved.t3.get(*n).copied(), resolved.q3.get(*n).copied(), resolved.mverts.get(*n), fill, finish(shape_stroke), shape_width);
            }
            if let Some(img) = &sh.image {
                let tris = textured_triangles(sh, ov, &tf);
                shape_json["image"] = json!({"src": img.src, "w": img.w, "h": img.h, "hash": format!("{:016x}", img.hash), "tris": tris});
            }
            shape_json
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
    let mut snap = json!({
        "tick": tick, "project_tick": project_tick, "scene": scene,
        "canvas": [it.canvas.0, it.canvas.1],
        "background": it.background.map(hex),
        "camera": camera_view(it, resolved).to_json(),
        "entities": entities, "shapes": shapes,
    });
    if let Some(name) = &it.shot_name {
        snap["shot"] = json!(name);
    }
    if let Some(tr) = &it.transition {
        let t = ((tick - tr.start) as f64 / tr.dur.max(1) as f64).clamp(0.0, 1.0);
        snap["transition"] =
            json!({"kind": tr.kind, "t": t, "color": tr.color, "from": tr.from.to_json()});
    }
    if let Some(p) = &resolved.physics {
        snap["physics"] = p.clone();
    }
    if let Some(p) = &resolved.physics3 {
        snap["physics3"] = p.clone();
    }
    if let Some(p) = &resolved.soft3 {
        snap["soft3"] = p.clone();
    }
    if let Some(p) = &resolved.fluids3 {
        snap["fluids3"] = p.clone();
    }
    if let Some(p) = &resolved.soft {
        snap["soft"] = p.clone();
    }
    if let Some(l) = it.lights_json(resolved) {
        snap["lights"] = l;
    }
    if let Some(p) = &resolved.rigs3 {
        snap["rigs3"] = p.clone();
    }
    if let Some(p) = &resolved.rigs {
        snap["rigs"] = p.clone();
    }
    if let Some(p) = &resolved.fluids {
        snap["fluids"] = p.clone();
    }
    if let Some(p) = &resolved.particles {
        snap["particles"] = p.clone();
    }
    if let Some(p) = it.field_snapshot() {
        snap["fields"] = p;
    }
    if it.any_visible_mesh() {
        snap["camera3"] = it.camera3_json(resolved);
        snap["light3"] = it.light3_json();
    }
    snap
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

/// The triangles of every face of a textured shape, each point `[x, y, u, v]` with `x, y` in world
/// space. Triangles and quads are split directly; other faces are cut into convex pieces first. The
/// texture coordinate belongs to the vertex, so the picture follows any deformation.
fn textured_triangles(sh: &Shape, ov: Option<&BTreeMap<u32, P>>, tf: &Transform) -> Vec<J> {
    use animscript_physics::{decompose, v2};
    let mut out = vec![];
    for f in sh.live_ids(Kind::F) {
        let ids = sh.face_vertex_ids(f);
        let pos: Vec<P> = ids
            .iter()
            .map(|i| {
                ov.and_then(|m| m.get(i).copied())
                    .unwrap_or_else(|| sh.pos(*i))
            })
            .collect();
        let point = |k: usize| {
            let w = tf.apply(pos[k]);
            let uv = sh.uv_of(ids[k]);
            json!([w[0], w[1], uv[0], uv[1]])
        };
        let convex = |p: &[P]| {
            let n = p.len();
            let signs: Vec<f64> = (0..n)
                .map(|i| {
                    let (a, b, c) = (p[i], p[(i + 1) % n], p[(i + 2) % n]);
                    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
                })
                .collect();
            signs.iter().all(|s| *s >= 0.0) || signs.iter().all(|s| *s <= 0.0)
        };
        match pos.len() {
            0..=2 => {}
            3 => out.push(json!([point(0), point(1), point(2)])),
            4 if convex(&pos) => {
                out.push(json!([point(0), point(1), point(2)]));
                out.push(json!([point(0), point(2), point(3)]));
            }
            _ => {
                let ring: Vec<_> = pos.iter().map(|p| v2(p[0], p[1])).collect();
                let Ok(pieces) = decompose(&ring) else {
                    continue;
                };
                for piece in pieces {
                    let idx: Vec<usize> = piece
                        .iter()
                        .filter_map(|q| ring.iter().position(|r| r.x == q.x && r.y == q.y))
                        .collect();
                    if idx.len() != piece.len() {
                        continue;
                    }
                    for k in 1..idx.len() - 1 {
                        out.push(json!([point(idx[0]), point(idx[k]), point(idx[k + 1])]));
                    }
                }
            }
        }
    }
    out
}

/// Where the camera looks: the centre of the view in canvas coordinates, the zoom (1 to 8) and the rotation (turns).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamView {
    pub position: [f64; 2],
    pub zoom: f64,
    pub rotation: f64,
}

impl CamView {
    pub fn to_json(&self) -> J {
        json!({"position": self.position, "zoom": self.zoom, "rotation": self.rotation})
    }
}

/// A `dissolve` or `fade_through` between shots: the camera view it started from is kept while it runs.
#[derive(Clone, Debug)]
pub struct Transition {
    pub kind: String,
    pub color: Option<String>,
    pub start: i64,
    pub dur: i64,
    pub from: CamView,
}

/// The camera as the layers resolved it this tick.
pub fn camera_view(it: &Interp, resolved: &Resolved) -> CamView {
    let sh = &it.shapes["camera"];
    let t = resolved
        .tf
        .get("camera")
        .copied()
        .unwrap_or_else(|| sh.transform());
    CamView {
        position: [
            it.canvas.0 as f64 / 2.0 + t.position[0],
            it.canvas.1 as f64 / 2.0 + t.position[1],
        ],
        zoom: t.scale[0].clamp(1.0, 8.0),
        rotation: t.rotation,
    }
}
