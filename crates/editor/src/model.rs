//! What the panels show, worked out from a run (no drawing here, so all of it is tested without a window).
use crate::engine::RunOutput;
use animscript_lang::journal::{ItemKind, Journal};
use serde_json::Value as J;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    None,
    Shape(String),
    Entity(String),
    Scene(usize),
    /// A declaration that is not a shape or an entity: an emitter, a fluid, a field, an armature.
    Decl(String),
}

// ---------------------------------------------------------------------------------- hierarchy

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub label: String,
    /// A short remark shown dimmed after the label (`z 2`, `hidden`, `24 ticks`).
    pub detail: String,
    pub sel: Selection,
    /// The source line of its declaration.
    pub line: Option<u32>,
    pub children: Vec<Node>,
}

fn group(label: &str, children: Vec<Node>) -> Node {
    Node {
        label: label.to_string(),
        detail: children.len().to_string(),
        sel: Selection::None,
        line: None,
        children,
    }
}

fn decl_line(j: &Journal, name: &str) -> Option<u32> {
    j.decls.iter().find(|d| d.name == name).map(|d| d.span.line)
}

/// The tree of the hierarchy panel at `tick`: scenes, shapes (top-most first), entities, everything else.
/// Empty groups are left out.
pub fn hierarchy(out: &RunOutput, tick: usize) -> Vec<Node> {
    let j = &out.journal;
    let snap = out.snapshots.get(tick);
    let mut groups = vec![];
    let scenes: Vec<Node> = j
        .scenes
        .iter()
        .enumerate()
        .map(|(i, s)| Node {
            label: s.name.clone(),
            detail: format!("{} ticks", s.end - s.start),
            sel: Selection::Scene(i),
            line: Some(s.span.line),
            children: vec![],
        })
        .collect();
    let mut shapes: Vec<Node> = snap
        .and_then(|s| s["shapes"].as_array())
        .map(|list| {
            list.iter()
                .filter_map(|sh| {
                    let name = sh["id"].as_str()?;
                    let mut detail = format!("z {}", sh["z"]);
                    if sh["visible"] == J::Bool(false) {
                        detail += ", hidden";
                    }
                    Some(Node {
                        label: name.to_string(),
                        detail,
                        sel: Selection::Shape(name.to_string()),
                        line: decl_line(j, name),
                        children: vec![],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    shapes.reverse(); // the snapshot is bottom-up; a layer list reads top-down
    let entities: Vec<Node> = snap
        .and_then(|s| s["entities"].as_array())
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let name = e["id"].as_str()?;
                    Some(Node {
                        label: name.to_string(),
                        detail: on_flags(&e["flags"]).join(", "),
                        sel: Selection::Entity(name.to_string()),
                        line: decl_line(j, name),
                        children: vec![],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let other: Vec<Node> = j
        .decls
        .iter()
        .filter(|d| !matches!(d.kind.as_str(), "shape" | "prop" | "character"))
        .map(|d| Node {
            label: d.name.clone(),
            detail: d.kind.clone(),
            sel: Selection::Decl(d.name.clone()),
            line: Some(d.span.line),
            children: vec![],
        })
        .collect();
    for (label, list) in [
        ("Scenes", scenes),
        ("Shapes", shapes),
        ("Entities", entities),
        ("Other", other),
    ] {
        if !list.is_empty() {
            groups.push(group(label, list));
        }
    }
    groups
}

/// The names of the flags that are set, whatever shape the snapshot gives them (a map or a list).
fn on_flags(flags: &J) -> Vec<String> {
    match flags {
        J::Object(m) => m
            .iter()
            .filter(|(_, v)| **v == J::Bool(true))
            .map(|(k, _)| k.clone())
            .collect(),
        J::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------------- inspector

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub key: String,
    pub value: String,
    /// A swatch for colour values (straight RGBA).
    pub color: Option<[u8; 4]>,
    /// A heading of a group of rows.
    pub heading: bool,
    /// A row that leads to a source line when clicked.
    pub line: Option<u32>,
}

fn row(depth: usize, key: &str, value: impl Into<String>) -> Row {
    let value = value.into();
    Row {
        depth,
        key: key.to_string(),
        color: parse_color(&value),
        value,
        heading: false,
        line: None,
    }
}
fn heading(depth: usize, key: &str) -> Row {
    Row {
        heading: true,
        ..row(depth, key, "")
    }
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let h = s.strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v: Vec<u8> = h.chars().map(|c| c.to_digit(16).unwrap() as u8).collect();
    match v.len() {
        3 | 4 => Some([
            v[0] * 17,
            v[1] * 17,
            v[2] * 17,
            v.get(3).map_or(255, |a| a * 17),
        ]),
        6 | 8 => Some([
            v[0] * 16 + v[1],
            v[2] * 16 + v[3],
            v[4] * 16 + v[5],
            if v.len() == 8 { v[6] * 16 + v[7] } else { 255 },
        ]),
        _ => None,
    }
}

/// A number the way a person reads it: at most three decimals, no trailing zeros.
pub fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

fn scalar(v: &J) -> Option<String> {
    match v {
        J::Null => Some("none".to_string()),
        J::Bool(b) => Some(b.to_string()),
        J::Number(n) => Some(n.as_f64().map_or(n.to_string(), num)),
        J::String(s) => Some(s.clone()),
        J::Array(a) if a.len() <= 4 && a.iter().all(|x| !x.is_array() && !x.is_object()) => {
            Some(format!(
                "({})",
                a.iter().filter_map(scalar).collect::<Vec<_>>().join(", ")
            ))
        }
        _ => None,
    }
}

const MAX_ROWS: usize = 400;

/// Any JSON value as rows: scalars and short lists on one line, the rest nested.
fn json_rows(depth: usize, key: &str, v: &J, out: &mut Vec<Row>) {
    if out.len() >= MAX_ROWS {
        return;
    }
    if let Some(s) = scalar(v) {
        out.push(row(depth, key, s));
        return;
    }
    match v {
        J::Object(m) => {
            out.push(heading(depth, key));
            for (k, x) in m {
                json_rows(depth + 1, k, x, out);
            }
        }
        J::Array(a) => {
            out.push(Row {
                value: format!("{} items", a.len()),
                ..heading(depth, key)
            });
            for (i, x) in a.iter().enumerate() {
                json_rows(depth + 1, &format!("[{i}]"), x, out);
            }
        }
        _ => {}
    }
}

fn pair(v: &J) -> String {
    format!(
        "({}, {})",
        num(v[0].as_f64().unwrap_or(0.0)),
        num(v[1].as_f64().unwrap_or(0.0))
    )
}

fn names(v: &J, name: &str) -> bool {
    ["shape", "id", "name", "body"]
        .iter()
        .any(|k| v[*k].as_str() == Some(name))
}

/// Entries of the other snapshot sections (physics bodies, particles, rigs...) that belong to `name`.
fn related(snap: &J, name: &str, out: &mut Vec<Row>) {
    let Some(top) = snap.as_object() else { return };
    for (section, v) in top {
        if matches!(section.as_str(), "shapes" | "entities" | "camera") {
            continue;
        }
        let lists: Vec<(String, &Vec<J>)> = match v {
            J::Array(a) => vec![(section.clone(), a)],
            J::Object(m) => m
                .iter()
                .filter_map(|(k, x)| x.as_array().map(|a| (format!("{section}.{k}"), a)))
                .collect(),
            _ => vec![],
        };
        for (label, list) in lists {
            for item in list.iter().filter(|x| names(x, name)) {
                json_rows(0, &label, item, out);
            }
        }
    }
}

/// The rows of the inspector for `sel` at `tick`. Empty when nothing is selected or it does not exist there.
pub fn inspect(out: &RunOutput, tick: usize, sel: &Selection) -> Vec<Row> {
    let j = &out.journal;
    let snap = out.snapshots.get(tick);
    let mut rows = vec![];
    let source = |rows: &mut Vec<Row>, line: Option<u32>| {
        if let Some(l) = line {
            rows.push(Row {
                line: Some(l),
                ..row(0, "declared", format!("line {l}"))
            });
        }
    };
    match sel {
        Selection::None => {}
        Selection::Scene(i) => {
            let Some(s) = j.scenes.get(*i) else {
                return rows;
            };
            rows.push(row(0, "scene", s.name.clone()));
            source(&mut rows, Some(s.span.line));
            rows.push(row(0, "ticks", format!("{} to {}", s.start, s.end - 1)));
            rows.push(row(0, "length", fmt_time(s.end - s.start, out.fps)));
            let n = j.items.iter().filter(|x| x.scene == s.name).count();
            rows.push(row(0, "timed items", n.to_string()));
        }
        Selection::Entity(name) => {
            let Some(e) = snap
                .and_then(|s| s["entities"].as_array())
                .and_then(|l| l.iter().find(|e| e["id"].as_str() == Some(name)))
            else {
                return rows;
            };
            rows.push(row(0, "entity", name.clone()));
            source(&mut rows, decl_line(j, name));
            json_rows(0, "flags", &e["flags"], &mut rows);
        }
        Selection::Decl(name) => {
            let Some(d) = j.decls.iter().find(|d| d.name == *name) else {
                return rows;
            };
            rows.push(row(0, &d.kind, name.clone()));
            source(&mut rows, Some(d.span.line));
            if let Some(s) = snap {
                related(s, name, &mut rows);
            }
        }
        Selection::Shape(name) => {
            let Some(sh) = snap
                .and_then(|s| s["shapes"].as_array())
                .and_then(|l| l.iter().find(|s| s["id"].as_str() == Some(name)))
            else {
                return rows;
            };
            rows.push(row(0, "shape", name.clone()));
            source(&mut rows, decl_line(j, name));
            rows.push(row(0, "z", scalar(&sh["z"]).unwrap_or_default()));
            rows.push(row(
                0,
                "visible",
                scalar(&sh["visible"]).unwrap_or_default(),
            ));
            let tf = &sh["transform"];
            rows.push(heading(0, "Transform"));
            rows.push(row(1, "position", pair(&tf["position"])));
            rows.push(row(1, "scale", pair(&tf["scale"])));
            let turns = tf["rotation"].as_f64().unwrap_or(0.0);
            rows.push(row(1, "rotation", format!("{} deg", num(turns * 360.0))));
            rows.push(row(1, "pivot", pair(&tf["pivot"])));
            rows.push(heading(0, "Style"));
            for k in ["fill", "stroke", "stroke_width", "opacity", "tint"] {
                rows.push(row(1, k, scalar(&sh[k]).unwrap_or_default()));
            }
            let active: Vec<&animscript_lang::journal::Item> = j
                .items
                .iter()
                .filter(|x| x.target == *name && item_active(x, tick as i64))
                .collect();
            if !active.is_empty() {
                rows.push(heading(0, "Animating now"));
                for a in active {
                    rows.push(Row {
                        line: a.span.map(|s| s.line),
                        ..row(1, kind_name(a.kind), item_range(a, out.fps))
                    });
                    rows.last_mut().unwrap().key = a.label.clone();
                }
            }
            let count = |k: &str| sh[k].as_array().map_or(0, |a| a.len());
            rows.push(heading(0, "Geometry"));
            if let Some(b) = snap.and_then(|s| shape_bounds(s, name)) {
                rows.push(row(
                    1,
                    "bounds",
                    format!(
                        "({}, {}) to ({}, {})",
                        num(b[0]),
                        num(b[1]),
                        num(b[2]),
                        num(b[3])
                    ),
                ));
            }
            rows.push(row(1, "edges", count("edges").to_string()));
            rows.push(row(1, "faces", count("faces").to_string()));
            rows.push(Row {
                value: format!("{} items", count("vertices")),
                ..heading(1, "vertices")
            });
            for v in sh["vertices"].as_array().into_iter().flatten().take(200) {
                rows.push(row(
                    2,
                    &format!("v{}", v[0]),
                    format!(
                        "({}, {})",
                        num(v[1].as_f64().unwrap_or(0.0)),
                        num(v[2].as_f64().unwrap_or(0.0))
                    ),
                ));
            }
            // whatever else the snapshot says about this shape (a picture, a mesh, a paint layer...)
            if let Some(m) = sh.as_object() {
                const SHOWN: &[&str] = &[
                    "id",
                    "z",
                    "visible",
                    "transform",
                    "fill",
                    "stroke",
                    "stroke_width",
                    "opacity",
                    "tint",
                    "vertices",
                    "edges",
                    "faces",
                ];
                for (k, v) in m.iter().filter(|(k, _)| !SHOWN.contains(&k.as_str())) {
                    json_rows(0, k, v, &mut rows);
                }
            }
            if let Some(s) = snap {
                related(s, name, &mut rows);
            }
        }
    }
    rows.truncate(MAX_ROWS);
    rows
}

// ---------------------------------------------------------------------------------- timeline

pub fn kind_name(k: ItemKind) -> &'static str {
    match k {
        ItemKind::Action => "action",
        ItemKind::Modifier => "modifier",
        ItemKind::Keys => "keyframes",
    }
}

/// Whether an item covers `tick`. A key set covers its last key; actions and modifiers end before `end`.
pub fn item_active(it: &animscript_lang::journal::Item, tick: i64) -> bool {
    let end = it.end.unwrap_or(i64::MAX);
    tick >= it.start
        && if it.kind == ItemKind::Keys {
            tick <= end
        } else {
            tick < end
        }
}

fn item_range(it: &animscript_lang::journal::Item, fps: f64) -> String {
    match it.end {
        Some(e) => format!(
            "tick {} to {} ({})",
            it.start,
            e,
            fmt_time(e - it.start, fps)
        ),
        None => format!("from tick {}", it.start),
    }
}

/// What a bar of the timeline says when the pointer rests on it.
pub fn item_tooltip(it: &animscript_lang::journal::Item, fps: f64) -> String {
    let mut s = format!(
        "{} ({})\n{}",
        it.label,
        kind_name(it.kind),
        item_range(it, fps)
    );
    if !it.keys.is_empty() {
        let keys: Vec<String> = it.keys.iter().map(|k| k.to_string()).collect();
        s += &format!("\nkeys at {}", keys.join(", "));
    }
    if let Some(sp) = it.span {
        s += &format!("\nline {}", sp.line);
    }
    s
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRow {
    pub target: String,
    /// Indices into `journal.items`, in start order.
    pub items: Vec<usize>,
    /// How many lanes the row needs so that overlapping items do not cover each other, and each item's lane.
    pub lanes: usize,
    pub lane_of: Vec<usize>,
}

/// One row per animated shape, in the order the shapes are declared (then anything else, by first use).
pub fn timeline_rows(j: &Journal) -> Vec<TimelineRow> {
    let mut order: Vec<&str> = j.decls.iter().map(|d| d.name.as_str()).collect();
    for it in &j.items {
        if !order.contains(&it.target.as_str()) {
            order.push(&it.target);
        }
    }
    order
        .into_iter()
        .filter_map(|target| {
            let mut items: Vec<usize> = (0..j.items.len())
                .filter(|i| j.items[*i].target == target)
                .collect();
            if items.is_empty() {
                return None;
            }
            items.sort_by_key(|i| (j.items[*i].start, *i));
            let mut lane_end: Vec<i64> = vec![];
            let lane_of: Vec<usize> = items
                .iter()
                .map(|i| {
                    let it = &j.items[*i];
                    // a key set owns its last tick too, so the next item starts a lane later than that
                    let end = it.end.unwrap_or(i64::MAX).max(it.start + 1)
                        + i64::from(it.kind == ItemKind::Keys);
                    let lane = lane_end
                        .iter()
                        .position(|e| *e <= it.start)
                        .unwrap_or(lane_end.len());
                    if lane == lane_end.len() {
                        lane_end.push(end);
                    } else {
                        lane_end[lane] = end;
                    }
                    lane
                })
                .collect();
            Some(TimelineRow {
                target: target.to_string(),
                items,
                lanes: lane_end.len().max(1),
                lane_of,
            })
        })
        .collect()
}

/// `tick` as seconds at `fps`, e.g. `1.5s`.
pub fn fmt_time(ticks: i64, fps: f64) -> String {
    format!("{}s", num(ticks as f64 / fps.max(1e-9)))
}

/// The index of the scene that contains project tick `tick`.
pub fn scene_at(j: &Journal, tick: i64) -> Option<usize> {
    j.scenes
        .iter()
        .position(|s| tick >= s.start && tick < s.end)
}

// ---------------------------------------------------------------------------------- console

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
    Trace,
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub level: Level,
    pub text: String,
    /// Clicking the line goes to this source line...
    pub line: Option<u32>,
    /// ...and to this tick.
    pub tick: Option<i64>,
}

/// The lines of the console for a run: a summary, diagnostics (with their notes and help), `note` output
/// and, when asked for, the runtime trace.
pub fn console(out: &RunOutput, show_trace: bool) -> Vec<Line> {
    let mut lines = vec![];
    let ticks = out.snapshots.len();
    lines.push(Line {
        level: Level::Info,
        text: format!(
            "ran {ticks} tick{} ({}) in {} ms: {} error{}, {} warning{}",
            if ticks == 1 { "" } else { "s" },
            fmt_time(ticks as i64, out.fps),
            out.millis,
            out.errors(),
            if out.errors() == 1 { "" } else { "s" },
            out.warnings(),
            if out.warnings() == 1 { "" } else { "s" },
        ),
        line: None,
        tick: None,
    });
    for d in &out.diagnostics {
        let level = if d.is_error() {
            Level::Error
        } else {
            Level::Warning
        };
        let line = d.span.map(|s| s.line);
        let at = line.map_or(String::new(), |l| format!("line {l}: "));
        let label = if d.label.is_empty() {
            String::new()
        } else {
            format!(" ({})", d.label)
        };
        lines.push(Line {
            level,
            text: format!("{at}[{}] {}{label}", d.code, d.message),
            line,
            tick: None,
        });
        for (kind, list) in [("note", &d.notes), ("help", &d.helps)] {
            for t in list {
                lines.push(Line {
                    level,
                    text: format!("    {kind}: {t}"),
                    line,
                    tick: None,
                });
            }
        }
    }
    for n in &out.journal.notes {
        let at = n.tick.map_or("build".to_string(), |t| format!("tick {t}"));
        lines.push(Line {
            level: Level::Note,
            text: format!("{at}: {}", n.text),
            line: n.span.map(|s| s.line),
            tick: n.tick,
        });
    }
    if show_trace {
        lines.extend(out.trace.iter().map(|t| Line {
            level: Level::Trace,
            text: t.clone(),
            line: None,
            tick: None,
        }));
    }
    lines
}

// ---------------------------------------------------------------------------------- preview geometry

/// How canvas coordinates land on the rendered picture (in canvas pixels): the snapshot's camera, exactly as
/// the renderer applies it.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pos: (f64, f64),
    zoom: f64,
    cos: f64,
    sin: f64,
    half: (f64, f64),
}

impl View {
    pub fn of(snap: &J) -> View {
        let cw = snap["canvas"][0].as_f64().unwrap_or(800.0);
        let ch = snap["canvas"][1].as_f64().unwrap_or(600.0);
        let cam = &snap["camera"];
        let a = -cam["rotation"].as_f64().unwrap_or(0.0) * std::f64::consts::TAU;
        View {
            pos: (
                cam["position"][0].as_f64().unwrap_or(cw / 2.0),
                cam["position"][1].as_f64().unwrap_or(ch / 2.0),
            ),
            zoom: cam["zoom"].as_f64().unwrap_or(1.0).clamp(1.0, 8.0),
            cos: a.cos(),
            sin: a.sin(),
            half: (cw / 2.0, ch / 2.0),
        }
    }
    /// Scene coordinates to picture pixels.
    pub fn to_picture(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = ((x - self.pos.0) * self.zoom, (y - self.pos.1) * self.zoom);
        (
            self.cos * x - self.sin * y + self.half.0,
            self.sin * x + self.cos * y + self.half.1,
        )
    }
    /// Picture pixels back to scene coordinates.
    pub fn to_scene(&self, px: f64, py: f64) -> (f64, f64) {
        let (x, y) = (px - self.half.0, py - self.half.1);
        let (x, y) = (self.cos * x + self.sin * y, -self.sin * x + self.cos * y);
        (x / self.zoom + self.pos.0, y / self.zoom + self.pos.1)
    }
}

fn find_shape<'a>(snap: &'a J, name: &str) -> Option<&'a J> {
    snap["shapes"]
        .as_array()?
        .iter()
        .find(|s| s["id"].as_str() == Some(name))
}

/// The bounding box `[x0, y0, x1, y1]` of a shape's vertices in scene coordinates.
pub fn shape_bounds(snap: &J, name: &str) -> Option<[f64; 4]> {
    bounds_of(find_shape(snap, name)?)
}

fn bounds_of(sh: &J) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for v in sh["vertices"].as_array()? {
        let (x, y) = (v[1].as_f64()?, v[2].as_f64()?);
        b = Some(match b {
            None => [x, y, x, y],
            Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
        });
    }
    b
}

/// The corners of a shape's bounding box as picture pixels (four points: the box may be turned by the camera).
pub fn shape_outline(snap: &J, name: &str) -> Option<[(f64, f64); 4]> {
    let b = shape_bounds(snap, name)?;
    let v = View::of(snap);
    Some([
        v.to_picture(b[0], b[1]),
        v.to_picture(b[2], b[1]),
        v.to_picture(b[2], b[3]),
        v.to_picture(b[0], b[3]),
    ])
}

/// The top-most visible shape whose bounding box contains the picture pixel `(px, py)`.
pub fn pick(snap: &J, px: f64, py: f64) -> Option<String> {
    let (x, y) = View::of(snap).to_scene(px, py);
    snap["shapes"].as_array()?.iter().rev().find_map(|sh| {
        if sh["visible"] == J::Bool(false) {
            return None;
        }
        let b = bounds_of(sh)?;
        // a line has no area: give every box a few pixels to be clicked on
        let pad = 3.0;
        (x >= b[0] - pad && x <= b[2] + pad && y >= b[1] - pad && y <= b[3] + pad)
            .then(|| sh["id"].as_str().map(String::from))?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::run_source;
    use std::path::Path;

    const SRC: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 10\n\nshape back = square(size: 200, at: (200, 150))\nback.fill = #336699\nshape dot = circle(8, radius: 20, at: (200, 150))\ndot.z = 1\nprop door:\n    states: open\nscene \"one\" for 1.second:\n    main:\n        note \"go\"\n        start dot.fade_to(0.5) over 3.frames\n        dot.move_by(50, 0) over 5.frames\n        dot.rotate_by(90.deg) over 2.frames\nscene \"two\" for 5.frames:\n    main:\n        door becomes open\n        wait 1.frame\n";

    fn out() -> RunOutput {
        let o = run_source(SRC, Path::new("."), true, 1);
        assert_eq!(o.errors(), 0, "{:?}", o.diagnostics);
        o
    }
    fn line_of(needle: &str) -> u32 {
        SRC.lines().position(|l| l.contains(needle)).unwrap() as u32 + 1
    }
    fn value<'a>(rows: &'a [Row], key: &str) -> &'a str {
        &rows
            .iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("no row {key}: {rows:?}"))
            .value
    }

    #[test]
    fn hierarchy_lists_scenes_shapes_top_down_and_entities() {
        let o = out();
        let h = hierarchy(&o, 0);
        let labels: Vec<&str> = h.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Scenes", "Shapes", "Entities"]);
        assert_eq!(h[0].children.len(), 2);
        assert_eq!(h[0].children[1].sel, Selection::Scene(1));
        assert_eq!(h[0].children[1].line, Some(line_of("scene \"two\"")));
        let shapes: Vec<&str> = h[1].children.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(shapes, ["dot", "back"], "the higher z comes first");
        assert_eq!(h[1].children[0].line, Some(line_of("shape dot")));
        assert_eq!(h[2].children[0].sel, Selection::Entity("door".into()));
        // the flag set in scene two shows up there
        assert_eq!(h[2].children[0].detail, "");
        let later = hierarchy(&o, 12);
        assert_eq!(later[2].children[0].detail, "open");
    }

    #[test]
    fn the_inspector_follows_the_playhead() {
        let o = out();
        let sel = Selection::Shape("dot".into());
        let at0 = inspect(&o, 0, &sel);
        assert_eq!(value(&at0, "shape"), "dot");
        assert_eq!(value(&at0, "position"), "(0, 0)");
        assert_eq!(
            value(&at0, "declared"),
            format!("line {}", line_of("shape dot"))
        );
        let at9 = inspect(&o, 9, &sel);
        assert_eq!(value(&at9, "position"), "(50, 0)");
        assert_eq!(value(&at9, "rotation"), "90 deg");
        assert_eq!(value(&at9, "opacity"), "0.5");
        // tick 1: the move and the fade are running, the rotation has not started
        let at1 = inspect(&o, 1, &sel);
        assert!(at1
            .iter()
            .any(|r| r.key == "dot.move_by" && r.line == Some(line_of("move_by"))));
        assert!(at1.iter().any(|r| r.key == "dot.fade_to"));
        assert!(!at1.iter().any(|r| r.key == "dot.rotate_by"));
    }

    #[test]
    fn colours_get_a_swatch_and_other_selections_have_rows() {
        let o = out();
        let rows = inspect(&o, 0, &Selection::Shape("back".into()));
        let fill = rows.iter().find(|r| r.key == "fill").unwrap();
        assert_eq!(fill.color, Some([0x33, 0x66, 0x99, 255]));
        assert_eq!(value(&rows, "vertices"), "4 items");
        assert_eq!(value(&rows, "bounds"), "(100, 50) to (300, 250)");
        let scene = inspect(&o, 0, &Selection::Scene(1));
        assert_eq!(value(&scene, "ticks"), "10 to 14");
        assert_eq!(value(&scene, "length"), "0.5s");
        assert!(!inspect(&o, 12, &Selection::Entity("door".into())).is_empty());
        assert!(inspect(&o, 0, &Selection::Shape("nobody".into())).is_empty());
        assert!(inspect(&o, 0, &Selection::None).is_empty());
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#00000080"), Some([0, 0, 0, 128]));
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn timeline_rows_group_by_shape_and_stack_overlaps() {
        let o = out();
        let rows = timeline_rows(&o.journal);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target, "dot");
        assert_eq!(rows[0].items.len(), 3);
        // fade [0,3) and move [0,5) overlap: two lanes; the rotation [5,7) fits back into the first
        assert_eq!(rows[0].lanes, 2);
        assert_eq!(rows[0].lane_of, vec![0, 1, 0]);
        assert_eq!(scene_at(&o.journal, 9), Some(0));
        assert_eq!(scene_at(&o.journal, 10), Some(1));
        assert_eq!(scene_at(&o.journal, 15), None);
        let mv = o
            .journal
            .items
            .iter()
            .find(|i| i.label == "dot.move_by")
            .unwrap();
        let tip = item_tooltip(mv, o.fps);
        assert!(
            tip.contains("dot.move_by") && tip.contains("0.5s") && tip.contains("line"),
            "{tip}"
        );
    }

    #[test]
    fn the_console_has_a_summary_notes_diagnostics_and_trace() {
        let o = out();
        let c = console(&o, false);
        assert_eq!(c[0].level, Level::Info);
        assert!(
            c[0].text.starts_with("ran 15 ticks (1.5s)"),
            "{}",
            c[0].text
        );
        let note = c.iter().find(|l| l.level == Level::Note).unwrap();
        assert_eq!(note.text, "tick 0: go");
        assert_eq!(
            (note.line, note.tick),
            (Some(line_of("note \"go\"")), Some(0))
        );
        assert!(!c.iter().any(|l| l.level == Level::Trace));
        assert!(console(&o, true)
            .iter()
            .any(|l| l.level == Level::Trace && l.text.contains("ACT+")));

        let bad = run_source(
            "shape s = square(size: 100)\ns.v9.move_to(0, 0)\n",
            Path::new("."),
            false,
            1,
        );
        let c = console(&bad, false);
        let err = c.iter().find(|l| l.level == Level::Error).unwrap();
        assert!(err.text.starts_with("line 2: [E-SHP-002]"), "{}", err.text);
        assert_eq!(err.line, Some(2));
        assert!(c.iter().any(|l| l.text.trim_start().starts_with("help:")));
    }

    #[test]
    fn picking_and_outlines_use_the_camera() {
        let o = out();
        let snap = &o.snapshots[0];
        // the dot is drawn over the square
        assert_eq!(pick(snap, 200.0, 150.0).as_deref(), Some("dot"));
        assert_eq!(pick(snap, 110.0, 60.0).as_deref(), Some("back"));
        assert_eq!(pick(snap, 5.0, 5.0), None);
        let outline = shape_outline(snap, "back").unwrap();
        assert_eq!(outline[0], (100.0, 50.0));
        assert_eq!(outline[2], (300.0, 250.0));
        // a camera looking at (100, 50) with zoom 2 puts that point in the middle of the picture
        let mut zoomed = snap.clone();
        zoomed["camera"] =
            serde_json::json!({"position": [100.0, 50.0], "zoom": 2.0, "rotation": 0.25});
        let v = View::of(&zoomed);
        let p = v.to_picture(100.0, 50.0);
        assert!((p.0 - 200.0).abs() < 1e-9 && (p.1 - 150.0).abs() < 1e-9);
        let back = v.to_scene(v.to_picture(37.0, 91.0).0, v.to_picture(37.0, 91.0).1);
        assert!((back.0 - 37.0).abs() < 1e-9 && (back.1 - 91.0).abs() < 1e-9);
    }

    #[test]
    fn numbers_read_cleanly() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.12345), "0.123");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(fmt_time(15, 10.0), "1.5s");
    }
}
