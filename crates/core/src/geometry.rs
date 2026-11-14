//! Editable 2D shapes made of vertices, edges and faces.
//!
//! Hard rule: IDs are stable. Elements are never renumbered or reused; retired elements stay in
//! the tables (state `Deleted` / `Replaced`) so that history queries (`parent`, `children`) keep
//! working and so that a stale reference is reported as `E-SHP-002` instead of silently pointing
//! at something else.
use crate::diag::{Diagnostic, Res};
use std::collections::{BTreeMap, BTreeSet};

pub type P = [f64; 2];
const FLATTEN_STEPS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    V,
    E,
    F,
}

impl Kind {
    pub fn letter(self) -> char {
        match self {
            Kind::V => 'v',
            Kind::E => 'e',
            Kind::F => 'f',
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::V => "vertex",
            Kind::E => "edge",
            Kind::F => "face",
        }
    }
    pub fn plural(self) -> &'static str {
        match self {
            Kind::V => "vertices",
            Kind::E => "edges",
            Kind::F => "faces",
        }
    }
    pub fn from_letter(c: char) -> Option<Kind> {
        match c {
            'v' => Some(Kind::V),
            'e' => Some(Kind::E),
            'f' => Some(Kind::F),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElemRef {
    pub kind: Kind,
    pub id: u32,
}
pub fn v(id: u32) -> ElemRef {
    ElemRef { kind: Kind::V, id }
}
pub fn e(id: u32) -> ElemRef {
    ElemRef { kind: Kind::E, id }
}
pub fn f(id: u32) -> ElemRef {
    ElemRef { kind: Kind::F, id }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Live,
    Deleted,
    Replaced,
}

#[derive(Clone, Debug)]
pub struct Meta {
    pub state: State,
    /// The element the operation that created this one was applied to (any kind).
    pub parent: Option<ElemRef>,
    /// Same-kind elements created from this one (e.g. the edges an edge was subdivided into).
    pub children: Vec<u32>,
    /// Every element created from this one, of any kind, in creation order.
    pub created: Vec<ElemRef>,
    pub alias: Option<String>,
    /// Why the element was retired, e.g. "subdivided into e5, e6".
    pub note: String,
}

impl Meta {
    fn new(parent: Option<ElemRef>) -> Meta {
        Meta {
            state: State::Live,
            parent,
            children: vec![],
            created: vec![],
            alias: None,
            note: String::new(),
        }
    }
}

/// Edge curvature. Handles are stored in the edge's own frame so they follow their vertices:
/// control = p0 + u * d + w * perp(d) with d = p1 - p0 and perp(d) = (-d.y, d.x) (right of travel on screen).
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Straight,
    Quad([f64; 2]),
    Cubic([f64; 2], [f64; 2]),
}

#[derive(Clone, Debug)]
pub struct Vertex {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub meta: Meta,
}
#[derive(Clone, Debug)]
pub struct Edge {
    pub id: u32,
    pub a: u32,
    pub b: u32,
    pub curve: Curve,
    /// Own stroke colour and width; `None` falls back to the shape's.
    pub stroke: Option<Color>,
    pub width: Option<f64>,
    pub meta: Meta,
}
#[derive(Clone, Debug)]
pub struct Face {
    pub id: u32,
    /// Ordered loop of `(edge id, direction)`; direction `1` traverses a -> b, `-1` traverses b -> a.
    pub loop_: Vec<(u32, i8)>,
    /// Own fill colour; `None` falls back to the shape's.
    pub fill: Option<Color>,
    pub meta: Meta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeAt {
    Center,
    First,
    Last,
}

pub type Color = [u8; 4];

/// A picture mapped onto a shape's faces. Every vertex keeps a texture coordinate (its position in
/// `rect`, the shape's bounding box when the picture was assigned), so the picture follows any
/// later deformation; vertices made afterwards are mapped linearly from their position.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageFill {
    /// The path as written in the program, relative to the program's folder.
    pub src: String,
    pub w: u32,
    pub h: u32,
    /// FNV-1a of the file's bytes, so a changed picture changes the frame hash.
    pub hash: u64,
    /// `[x, y, width, height]` of the shape (local coordinates) that the picture covers.
    pub rect: [f64; 4],
}

/// A mesh's special surface (`glass(...)`, `mirror(...)`).
#[derive(Clone, Debug, PartialEq)]
pub struct MeshMaterial {
    /// `"glass"` or `"mirror"`.
    pub kind: String,
    /// How much of the surroundings it reflects (a mirror: how mirror-like; glass: the Fresnel strength).
    pub reflect: f64,
    /// How much it bends what is behind it (glass only).
    pub refract: f64,
    /// How opaque the glass body is, 0 to 1.
    pub opacity: f64,
}

#[derive(Clone, Debug)]
pub struct Shape {
    pub name: String,
    pub verts: BTreeMap<u32, Vertex>,
    pub edges: BTreeMap<u32, Edge>,
    pub faces: BTreeMap<u32, Face>,
    next: [u32; 3],
    aliases: BTreeMap<String, ElemRef>,
    /// Warnings produced by edits since the caller last drained them.
    pub warnings: Vec<Diagnostic>,
    pub warn_on_edit: bool,
    warned: BTreeSet<(u32, u8)>,
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub stroke_width: f64,
    pub z: i64,
    pub visible: bool,
    /// Transform on top of the local vertex coordinates (applied by the runtime when it resolves a frame).
    pub position: P,
    pub scale: P,
    /// In turns, clockwise on screen.
    pub rotation: f64,
    /// `None` until frozen: then the bounding-box centre of the vertices.
    pub pivot: Option<P>,
    /// Multiplies the alpha of every fill and stroke (0 to 1).
    pub opacity: f64,
    /// Multiplies the colour of every fill and stroke (white = no change).
    pub tint: Color,
    pub image: Option<ImageFill>,
    /// Set on a paint layer (`paint_layer()`): its size in canvas pixels and resolution; the strokes live in the runtime.
    pub paint: Option<PaintInfo>,
    /// A 3D shape (`cube`, `sphere`, `lathe`, ...): the mesh in local coordinates, centred about `pivot3`. A mesh has no
    /// 2D vertices, edges or faces. Its position (x, y: `position`; z: `pos_z`), turns about x, y and z (`rot_x`,
    /// `rot_y`, `rotation`) and scale (`scale` and `scale_z`) are animated like any shape's.
    pub mesh: Option<crate::mesh::Mesh>,
    pub pos_z: f64,
    pub rot_x: f64,
    pub rot_y: f64,
    pub scale_z: f64,
    pub pivot3: [f64; 3],
    /// What the mesh's surface does with light beyond shading: reflect its surroundings (mirror) or
    /// let them through bent (glass).
    pub material: Option<MeshMaterial>,
    /// A physics body's orientation as a quaternion `[w, x, y, z]`; replaces the three turns when set.
    pub orient: Option<[f64; 4]>,
    /// Texture coordinates `[u, v]` (0 to 1) fixed when the image was assigned.
    pub uv: BTreeMap<u32, [f64; 2]>,
}

/// A paint layer's raster size (canvas pixels) and how many picture pixels there are per canvas pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintInfo {
    pub size: [u32; 2],
    pub resolution: f64,
}

/// Scale about the pivot, then rotate about the pivot, then translate by `position`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: P,
    pub scale: P,
    pub rotation: f64,
    pub pivot: P,
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        self.position == [0.0, 0.0] && self.scale == [1.0, 1.0] && self.rotation == 0.0
    }
    pub fn apply(&self, p: P) -> P {
        if self.is_identity() {
            return p; // exact, so shapes that never use a transform are bit-for-bit unchanged
        }
        let (dx, dy) = (
            (p[0] - self.pivot[0]) * self.scale[0],
            (p[1] - self.pivot[1]) * self.scale[1],
        );
        let (s, c) = (self.rotation * std::f64::consts::TAU).sin_cos();
        [
            self.pivot[0] + dx * c - dy * s + self.position[0],
            self.pivot[1] + dx * s + dy * c + self.position[1],
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// Curve maths
// ---------------------------------------------------------------------------------------------

fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Absolute control points of an edge from its endpoints and curve.
pub fn curve_points(p0: P, p1: P, c: &Curve) -> Vec<P> {
    let d = [p1[0] - p0[0], p1[1] - p0[1]];
    let perp = [-d[1], d[0]];
    let ctrl = |uw: &[f64; 2]| {
        [
            p0[0] + uw[0] * d[0] + uw[1] * perp[0],
            p0[1] + uw[0] * d[1] + uw[1] * perp[1],
        ]
    };
    match c {
        Curve::Straight => vec![p0, p1],
        Curve::Quad(a) => vec![p0, ctrl(a), p1],
        Curve::Cubic(a, b) => vec![p0, ctrl(a), ctrl(b), p1],
    }
}

/// Express an absolute control point in the chord frame of `p0 -> p1`.
pub fn to_local(p0: P, p1: P, c: P) -> [f64; 2] {
    let d = [p1[0] - p0[0], p1[1] - p0[1]];
    let d2 = d[0] * d[0] + d[1] * d[1];
    if d2 < 1e-18 {
        return [0.0, 0.0];
    }
    let r = [c[0] - p0[0], c[1] - p0[1]];
    [
        (r[0] * d[0] + r[1] * d[1]) / d2,
        (r[0] * -d[1] + r[1] * d[0]) / d2,
    ]
}

fn split_bezier(pts: &[P], t: f64) -> (Vec<P>, Vec<P>) {
    let mut levels: Vec<Vec<P>> = vec![pts.to_vec()];
    while levels.last().unwrap().len() > 1 {
        let cur = levels.last().unwrap();
        let next: Vec<P> = (0..cur.len() - 1)
            .map(|i| lerp(cur[i], cur[i + 1], t))
            .collect();
        levels.push(next);
    }
    let left = levels.iter().map(|l| l[0]).collect();
    let right = levels.iter().rev().map(|l| *l.last().unwrap()).collect();
    (left, right)
}

fn bezier_at(pts: &[P], t: f64) -> P {
    let (l, _) = split_bezier(pts, t);
    *l.last().unwrap()
}

fn piece_curve(pts: &[P]) -> Curve {
    match pts.len() {
        3 => Curve::Quad(to_local(pts[0], pts[2], pts[1])),
        4 => Curve::Cubic(
            to_local(pts[0], pts[3], pts[1]),
            to_local(pts[0], pts[3], pts[2]),
        ),
        _ => Curve::Straight,
    }
}

/// Polyline approximation of a Bézier given by its control points.
pub fn flatten(pts: &[P]) -> Vec<P> {
    if pts.len() == 2 {
        return pts.to_vec();
    }
    (0..=FLATTEN_STEPS)
        .map(|i| bezier_at(pts, i as f64 / FLATTEN_STEPS as f64))
        .collect()
}

/// Signed area by the shoelace formula. Negative means counter-clockwise *on screen* (y points down).
pub fn shoelace(poly: &[P]) -> f64 {
    let n = poly.len();
    let mut s = 0.0;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s / 2.0
}

fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn proper_cross(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2, d3, d4) = (
        orient(c, d, a),
        orient(c, d, b),
        orient(a, b, c),
        orient(a, b, d),
    );
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

fn self_intersects(poly: &[P]) -> bool {
    let n = poly.len();
    for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if proper_cross(poly[i], poly[(i + 1) % n], poly[j], poly[(j + 1) % n]) {
                return true;
            }
        }
    }
    false
}

fn fmt_ids(kind: Kind, ids: &[u32]) -> String {
    let mut s: Vec<String> = ids
        .iter()
        .take(12)
        .map(|i| format!("{}{}", kind.letter(), i))
        .collect();
    if ids.len() > 12 {
        s.push(format!("… ({} more)", ids.len() - 12));
    }
    if s.is_empty() {
        "none".into()
    } else {
        s.join(", ")
    }
}

const RESERVED_NAMES: &[&str] = &[
    "v",
    "e",
    "f",
    "name",
    "vertices",
    "edges",
    "faces",
    "fill",
    "stroke",
    "stroke_width",
    "z",
    "visible",
    "copy",
    "count",
    "smooth",
    "merge",
    "delete",
    "extrude",
    "morph_to",
];

fn invalid(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-SHP-005", msg)
}

// ---------------------------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------------------------

impl Shape {
    pub fn new(name: &str) -> Shape {
        Shape {
            name: name.to_string(),
            verts: BTreeMap::new(),
            edges: BTreeMap::new(),
            faces: BTreeMap::new(),
            next: [1, 1, 1],
            aliases: BTreeMap::new(),
            warnings: vec![],
            warn_on_edit: true,
            warned: BTreeSet::new(),
            fill: None,
            stroke: Some([0x22, 0x22, 0x22, 0xff]),
            stroke_width: 2.0,
            z: 0,
            visible: true,
            position: [0.0, 0.0],
            scale: [1.0, 1.0],
            rotation: 0.0,
            pivot: None,
            opacity: 1.0,
            tint: [255, 255, 255, 255],
            image: None,
            paint: None,
            mesh: None,
            pos_z: 0.0,
            rot_x: 0.0,
            rot_y: 0.0,
            scale_z: 1.0,
            pivot3: [0.0; 3],
            material: None,
            orient: None,
            uv: BTreeMap::new(),
        }
    }

    /// Map `img` onto the shape: the picture covers the bounding box of the current vertices.
    pub fn set_image(&mut self, mut img: ImageFill) {
        let ids = self.live_ids(Kind::V);
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for i in &ids {
            let p = self.pos(*i);
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if ids.is_empty() {
            lo = [0.0; 2];
            hi = [1.0; 2];
        }
        img.rect = [
            lo[0],
            lo[1],
            (hi[0] - lo[0]).max(1e-9),
            (hi[1] - lo[1]).max(1e-9),
        ];
        self.uv = ids
            .iter()
            .map(|i| {
                let p = self.pos(*i);
                (
                    *i,
                    [
                        (p[0] - img.rect[0]) / img.rect[2],
                        (p[1] - img.rect[1]) / img.rect[3],
                    ],
                )
            })
            .collect();
        self.image = Some(img);
    }

    /// Apply the shape's own position, scale and rotation to its vertices and reset them, so the
    /// local coordinates are the world coordinates (needed before a rig takes over the transform).
    pub fn bake_transform(&mut self) {
        let t = self.transform();
        if t.is_identity() {
            return;
        }
        for v in self.verts.values_mut() {
            let p = t.apply([v.x, v.y]);
            v.x = p[0];
            v.y = p[1];
        }
        self.position = [0.0, 0.0];
        self.scale = [1.0, 1.0];
        self.rotation = 0.0;
        self.pivot = None;
    }

    /// The texture coordinate of a vertex: its stored one, else linear in its rest position.
    pub fn uv_of(&self, vid: u32) -> [f64; 2] {
        if let Some(uv) = self.uv.get(&vid) {
            return *uv;
        }
        match &self.image {
            Some(img) => {
                let p = self.pos(vid);
                [
                    (p[0] - img.rect[0]) / img.rect[2],
                    (p[1] - img.rect[1]) / img.rect[3],
                ]
            }
            None => [0.0, 0.0],
        }
    }

    // ---- lookup ----------------------------------------------------------------------------

    pub fn label(&self, r: ElemRef) -> String {
        format!("{}.{}{}", self.name, r.kind.letter(), r.id)
    }

    pub fn meta(&self, r: ElemRef) -> Option<&Meta> {
        match r.kind {
            Kind::V => self.verts.get(&r.id).map(|x| &x.meta),
            Kind::E => self.edges.get(&r.id).map(|x| &x.meta),
            Kind::F => self.faces.get(&r.id).map(|x| &x.meta),
        }
    }
    fn meta_mut(&mut self, r: ElemRef) -> Option<&mut Meta> {
        match r.kind {
            Kind::V => self.verts.get_mut(&r.id).map(|x| &mut x.meta),
            Kind::E => self.edges.get_mut(&r.id).map(|x| &mut x.meta),
            Kind::F => self.faces.get_mut(&r.id).map(|x| &mut x.meta),
        }
    }

    pub fn live_ids(&self, kind: Kind) -> Vec<u32> {
        match kind {
            Kind::V => self
                .verts
                .values()
                .filter(|x| x.meta.state == State::Live)
                .map(|x| x.id)
                .collect(),
            Kind::E => self
                .edges
                .values()
                .filter(|x| x.meta.state == State::Live)
                .map(|x| x.id)
                .collect(),
            Kind::F => self
                .faces
                .values()
                .filter(|x| x.meta.state == State::Live)
                .map(|x| x.id)
                .collect(),
        }
    }

    fn unknown(&self, kind: Kind, id: u32) -> Diagnostic {
        let allocated = self.next[kind as usize] - 1;
        Diagnostic::new(
            "E-SHP-002",
            format!(
                "unknown {} `{}.{}{}`",
                kind.name(),
                self.name,
                kind.letter(),
                id
            ),
        )
        .help(format!(
            "`{}` has {} {}{}",
            self.name,
            kind.plural(),
            fmt_ids(kind, &self.live_ids(kind)),
            if allocated > 0 {
                format!(" (highest id ever used: {}{})", kind.letter(), allocated)
            } else {
                String::new()
            }
        ))
    }

    /// A reference to an element that has been allocated (live or retired). Errors with `E-SHP-002` otherwise.
    pub fn get_ref(&self, kind: Kind, id: u32) -> Res<ElemRef> {
        let r = ElemRef { kind, id };
        if self.meta(r).is_some() {
            Ok(r)
        } else {
            Err(self.unknown(kind, id))
        }
    }
    pub fn vref(&self, id: u32) -> Res<ElemRef> {
        self.get_ref(Kind::V, id)
    }
    pub fn eref(&self, id: u32) -> Res<ElemRef> {
        self.get_ref(Kind::E, id)
    }
    pub fn fref(&self, id: u32) -> Res<ElemRef> {
        self.get_ref(Kind::F, id)
    }
    pub fn resolve_name(&self, name: &str) -> Option<ElemRef> {
        self.aliases.get(name).copied()
    }

    fn retired(&self, r: ElemRef, m: &Meta) -> Diagnostic {
        let d = Diagnostic::new("E-SHP-002", format!("`{}` was {}", self.label(r), m.note));
        if m.state == State::Replaced && !m.children.is_empty() {
            let c: Vec<String> = m
                .children
                .iter()
                .map(|i| format!("{}.{}{}", self.name, r.kind.letter(), i))
                .collect();
            d.help(format!(
                "it no longer exists; use {} instead",
                c.join(" or ")
            ))
        } else {
            d.help(format!(
                "retired ids are never reused; live {} are {}",
                r.kind.plural(),
                fmt_ids(r.kind, &self.live_ids(r.kind))
            ))
        }
    }

    /// Check that `r` exists, is live, and (optionally) has the expected kind.
    pub fn require_live(&self, r: ElemRef, kind: Option<Kind>, op: &str) -> Res<()> {
        let m = self.meta(r).ok_or_else(|| self.unknown(r.kind, r.id))?;
        if let Some(k) = kind {
            if k != r.kind {
                return Err(invalid(format!(
                    "`{op}` needs a {}, but `{}` is a {}",
                    k.name(),
                    self.label(r),
                    r.kind.name()
                )));
            }
        }
        if m.state != State::Live {
            return Err(self.retired(r, m));
        }
        Ok(())
    }

    // ---- allocation ------------------------------------------------------------------------

    fn link(&mut self, parent: Option<ElemRef>, child: ElemRef) {
        if let Some(p) = parent {
            if let Some(m) = self.meta_mut(p) {
                m.created.push(child);
                if p.kind == child.kind {
                    m.children.push(child.id);
                }
            }
        }
    }
    fn new_vertex(&mut self, x: f64, y: f64, parent: Option<ElemRef>) -> u32 {
        let id = self.next[0];
        self.next[0] += 1;
        self.verts.insert(
            id,
            Vertex {
                id,
                x,
                y,
                meta: Meta::new(parent),
            },
        );
        self.link(parent, v(id));
        id
    }
    fn new_edge(&mut self, a: u32, b: u32, parent: Option<ElemRef>) -> u32 {
        let id = self.next[1];
        self.next[1] += 1;
        self.edges.insert(
            id,
            Edge {
                id,
                a,
                b,
                curve: Curve::Straight,
                stroke: None,
                width: None,
                meta: Meta::new(parent),
            },
        );
        self.link(parent, e(id));
        self.inherit_style(parent, e(id));
        id
    }
    fn new_face(&mut self, loop_: Vec<(u32, i8)>, parent: Option<ElemRef>) -> u32 {
        let id = self.next[2];
        self.next[2] += 1;
        self.faces.insert(
            id,
            Face {
                id,
                loop_,
                fill: None,
                meta: Meta::new(parent),
            },
        );
        self.link(parent, f(id));
        self.inherit_style(parent, f(id));
        id
    }
    /// A new element copies the style of its parent when the parent is of the same kind.
    fn inherit_style(&mut self, parent: Option<ElemRef>, child: ElemRef) {
        let Some(p) = parent else { return };
        if p.kind != child.kind {
            return;
        }
        match p.kind {
            Kind::E => {
                let (s, w) = (self.edges[&p.id].stroke, self.edges[&p.id].width);
                let c = self.edges.get_mut(&child.id).unwrap();
                c.stroke = s;
                c.width = w;
            }
            Kind::F => {
                let fl = self.faces[&p.id].fill;
                self.faces.get_mut(&child.id).unwrap().fill = fl;
            }
            Kind::V => {}
        }
    }
    fn copy_edge_style(&mut self, from: u32, to: u32) {
        let (s, w) = (self.edges[&from].stroke, self.edges[&from].width);
        let c = self.edges.get_mut(&to).unwrap();
        c.stroke = s;
        c.width = w;
    }
    fn retire(&mut self, r: ElemRef, state: State, note: String) {
        if let Some(m) = self.meta_mut(r) {
            m.state = state;
            m.note = note;
        }
    }

    // ---- geometry queries -----------------------------------------------------------------

    pub fn pos(&self, vid: u32) -> P {
        let v = &self.verts[&vid];
        [v.x, v.y]
    }
    fn pos_with(&self, vid: u32, ov: Option<&BTreeMap<u32, P>>) -> P {
        ov.and_then(|m| m.get(&vid).copied())
            .unwrap_or_else(|| self.pos(vid))
    }

    /// Absolute control points of an edge (2 = straight, 3 = quadratic, 4 = cubic).
    pub fn edge_points(&self, eid: u32) -> Vec<P> {
        self.edge_points_with(eid, None)
    }
    /// Same, with some vertex positions overridden (used by the runtime for resolved positions).
    pub fn edge_points_with(&self, eid: u32, ov: Option<&BTreeMap<u32, P>>) -> Vec<P> {
        let ed = &self.edges[&eid];
        curve_points(self.pos_with(ed.a, ov), self.pos_with(ed.b, ov), &ed.curve)
    }
    /// Directed segments of a face (control points in traversal order).
    pub fn face_segments_with(&self, fid: u32, ov: Option<&BTreeMap<u32, P>>) -> Vec<Vec<P>> {
        self.faces[&fid]
            .loop_
            .iter()
            .map(|&(eid, s)| {
                let mut p = self.edge_points_with(eid, ov);
                if s < 0 {
                    p.reverse();
                }
                p
            })
            .collect()
    }
    pub fn face_vertex_ids(&self, fid: u32) -> Vec<u32> {
        self.faces[&fid]
            .loop_
            .iter()
            .map(|&(eid, s)| self.start_vertex(eid, s))
            .collect()
    }
    fn start_vertex(&self, eid: u32, s: i8) -> u32 {
        let ed = &self.edges[&eid];
        if s > 0 {
            ed.a
        } else {
            ed.b
        }
    }
    fn end_vertex(&self, eid: u32, s: i8) -> u32 {
        let ed = &self.edges[&eid];
        if s > 0 {
            ed.b
        } else {
            ed.a
        }
    }
    fn sign_from(&self, eid: u32, from: u32) -> i8 {
        if self.edges[&eid].a == from {
            1
        } else {
            -1
        }
    }
    pub fn face_polygon(&self, fid: u32) -> Vec<P> {
        let mut poly = vec![];
        for seg in self.face_segments_with(fid, None) {
            let fl = flatten(&seg);
            poly.extend_from_slice(&fl[..fl.len() - 1]);
        }
        poly
    }
    /// Signed area of a face (negative = counter-clockwise on screen).
    pub fn face_area(&self, fid: u32) -> f64 {
        shoelace(&self.face_polygon(fid))
    }
    /// Centre of the bounding box of all live vertices.
    pub fn bbox_center(&self) -> P {
        let ids = self.live_ids(Kind::V);
        if ids.is_empty() {
            return [0.0, 0.0];
        }
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for i in ids {
            let p = self.pos(i);
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]
    }

    pub fn check_morph(&self, other: &Shape) -> Res<()> {
        let (a, b) = (self.live_ids(Kind::V).len(), other.live_ids(Kind::V).len());
        if a == b {
            return Ok(());
        }
        let fewer = if a < b { &self.name } else { &other.name };
        Err(Diagnostic::new("E-SHP-001", format!("cannot morph: {a} vs {b} vertices; subdivide `{fewer}` first"))
            .help("morphing pairs vertices one-to-one, so both shapes need the same number of live vertices")
            .help("or build both shapes from the same primitive and the same sequence of operations"))
    }

    // ---- names (aliases) ------------------------------------------------------------------

    fn validate_names(&self, names: &[String]) -> Res<()> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for n in names {
            let ok = !n.is_empty()
                && n.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                && n.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            let id_like = n.len() > 1
                && n.chars().next().and_then(Kind::from_letter).is_some()
                && n[1..].chars().all(|c| c.is_ascii_digit());
            if !ok || id_like || RESERVED_NAMES.contains(&n.as_str()) {
                return Err(invalid(format!("`{n}` is not a valid semantic name"))
                    .help("names are lower snake case and must not look like an id (`v1`, `e2`) or clash with a member name"));
            }
            if let Some(existing) = self.aliases.get(n) {
                return Err(Diagnostic::new(
                    "E-SHP-004",
                    format!("duplicate semantic name `{n}` in `{}`", self.name),
                )
                .help(format!("`{n}` already names {}", self.label(*existing)))
                .help("semantic names must be unique within a shape; pick another"));
            }
            if !seen.insert(n.as_str()) {
                return Err(Diagnostic::new(
                    "E-SHP-004",
                    format!("duplicate semantic name `{n}` in `{}`", self.name),
                )
                .help("the same name appears twice in this `names:` list"));
            }
        }
        Ok(())
    }
    fn apply_names(&mut self, kind: Kind, ids: &[u32], names: &[String]) {
        for (id, n) in ids.iter().zip(names) {
            let r = ElemRef { kind, id: *id };
            if let Some(m) = self.meta_mut(r) {
                m.alias = Some(n.clone());
            }
            self.aliases.insert(n.clone(), r);
        }
    }
    /// Give an existing live element a semantic name.
    pub fn set_alias(&mut self, r: ElemRef, name: &str) -> Res<()> {
        self.require_live(r, None, "name")?;
        self.validate_names(&[name.to_string()])?;
        self.apply_names(r.kind, &[r.id], &[name.to_string()]);
        Ok(())
    }
    fn expect_names(&self, names: &[String], n: usize, what: &str) -> Res<()> {
        if !names.is_empty() && names.len() != n {
            return Err(invalid(format!(
                "`names:` has {} entries but {what} produces {n}",
                names.len()
            )));
        }
        Ok(())
    }

    // ---- warnings -------------------------------------------------------------------------

    fn post_edit(&mut self, op: &str) {
        if !self.warn_on_edit {
            return;
        }
        let mut now: BTreeSet<(u32, u8)> = BTreeSet::new();
        let mut fresh: Vec<Diagnostic> = vec![];
        for fid in self.live_ids(Kind::F) {
            let poly = self.face_polygon(fid);
            let label = self.label(f(fid));
            let problem = if self.faces[&fid].loop_.len() < 3 || shoelace(&poly).abs() < 1e-9 {
                Some((
                    0u8,
                    format!("degenerate face `{label}` (zero area) after `{op}`"),
                ))
            } else if self_intersects(&poly) {
                Some((
                    1u8,
                    format!("face `{label}` intersects itself after `{op}`"),
                ))
            } else {
                None
            };
            if let Some((k, msg)) = problem {
                now.insert((fid, k));
                if !self.warned.contains(&(fid, k)) {
                    fresh.push(
                        Diagnostic::new("W-SHP-010", msg).help(
                            "move the vertices apart, or `delete` the face if it is not wanted",
                        ),
                    );
                }
            }
        }
        self.warned = now;
        self.warnings.extend(fresh);
    }

    // ---- moving ---------------------------------------------------------------------------

    fn face_unique_vertices(&self, fid: u32) -> Vec<u32> {
        let mut seen = BTreeSet::new();
        self.face_vertex_ids(fid)
            .into_iter()
            .filter(|x| seen.insert(*x))
            .collect()
    }
    fn shift(&mut self, ids: &[u32], dx: f64, dy: f64) {
        for i in ids {
            if let Some(vx) = self.verts.get_mut(i) {
                vx.x += dx;
                vx.y += dy;
            }
        }
    }
    /// The distinct vertices an element is made of (a vertex, an edge's two ends, a face's loop).
    pub fn element_vertices(&self, r: ElemRef) -> Vec<u32> {
        match r.kind {
            Kind::V => vec![r.id],
            Kind::E => {
                let ed = &self.edges[&r.id];
                if ed.a == ed.b {
                    vec![ed.a]
                } else {
                    vec![ed.a, ed.b]
                }
            }
            Kind::F => self.face_unique_vertices(r.id),
        }
    }

    /// Absolute move. A vertex goes to `(x, y)`; an edge's midpoint and a face's centroid go there,
    /// carrying all their vertices with them.
    pub fn move_to(&mut self, r: ElemRef, x: f64, y: f64) -> Res<()> {
        self.require_live(r, None, "move_to")?;
        let ids = self.element_vertices(r);
        let [cx, cy] = self.move_anchor(r);
        self.shift(&ids, x - cx, y - cy);
        self.post_edit("move_to");
        Ok(())
    }
    /// The point `move_to` places at the target: a vertex itself, an edge's midpoint, a face's centroid.
    pub fn move_anchor(&self, r: ElemRef) -> P {
        let ids = self.element_vertices(r);
        let n = ids.len().max(1) as f64;
        [
            ids.iter().map(|i| self.pos(*i)[0]).sum::<f64>() / n,
            ids.iter().map(|i| self.pos(*i)[1]).sum::<f64>() / n,
        ]
    }
    pub fn move_by(&mut self, r: ElemRef, dx: f64, dy: f64) -> Res<()> {
        self.require_live(r, None, "move_by")?;
        let ids = self.element_vertices(r);
        self.shift(&ids, dx, dy);
        self.post_edit("move_by");
        Ok(())
    }

    // ---- subdivide ------------------------------------------------------------------------

    /// Split an edge into `n` equal parts (in curve parameter) or a quad face into an `n` x `n` grid.
    pub fn subdivide(&mut self, r: ElemRef, n: usize, names: &[String]) -> Res<()> {
        self.require_live(r, None, "subdivide")?;
        if n < 2 {
            return Err(invalid("`subdivide` needs a count of at least 2")
                .help("write `subdivide(2)` to split in half"));
        }
        match r.kind {
            Kind::E => {
                self.expect_names(names, n, "this subdivision")?;
                self.validate_names(names)?;
                let (_, es) = self.split_edge(r.id, n);
                self.apply_names(Kind::E, &es, names);
            }
            Kind::F => self.subdivide_face(r.id, n, names)?,
            Kind::V => {
                return Err(
                    invalid("a vertex cannot be subdivided").help("subdivide an edge or a face")
                )
            }
        }
        self.post_edit("subdivide");
        Ok(())
    }

    /// Returns (vertices a..b incl. both ends, new edges), both in the edge's stored direction.
    fn split_edge(&mut self, eid: u32, n: usize) -> (Vec<u32>, Vec<u32>) {
        let ed = self.edges[&eid].clone();
        let (p0, p1) = (self.pos(ed.a), self.pos(ed.b));
        let mut pieces: Vec<Vec<P>> = vec![];
        if ed.curve == Curve::Straight {
            for k in 0..n {
                pieces.push(vec![
                    lerp(p0, p1, k as f64 / n as f64),
                    lerp(p0, p1, (k + 1) as f64 / n as f64),
                ]);
            }
            pieces[0][0] = p0;
            pieces[n - 1][1] = p1;
        } else {
            let mut rest = curve_points(p0, p1, &ed.curve);
            for k in 1..n {
                let (l, r) = split_bezier(&rest, 1.0 / (n - k + 1) as f64);
                pieces.push(l);
                rest = r;
            }
            pieces.push(rest);
        }
        let parent = Some(e(eid));
        let mut vs = vec![ed.a];
        for piece in pieces.iter().take(n - 1) {
            let p = *piece.last().unwrap();
            vs.push(self.new_vertex(p[0], p[1], parent));
        }
        vs.push(ed.b);
        let mut es = vec![];
        for i in 0..n {
            let id = self.new_edge(vs[i], vs[i + 1], parent);
            self.edges.get_mut(&id).unwrap().curve = piece_curve(&pieces[i]);
            es.push(id);
        }
        let note = format!("subdivided into {}", fmt_ids(Kind::E, &es));
        self.retire(e(eid), State::Replaced, note);
        self.replace_edge_in_faces(eid, &es);
        (vs, es)
    }

    fn replace_edge_in_faces(&mut self, old: u32, new_ids: &[u32]) {
        for fc in self
            .faces
            .values_mut()
            .filter(|x| x.meta.state == State::Live)
        {
            let mut out = Vec::with_capacity(fc.loop_.len() + new_ids.len());
            for &(eid, s) in &fc.loop_ {
                if eid == old {
                    if s > 0 {
                        out.extend(new_ids.iter().map(|&i| (i, 1i8)));
                    } else {
                        out.extend(new_ids.iter().rev().map(|&i| (i, -1i8)));
                    }
                } else {
                    out.push((eid, s));
                }
            }
            fc.loop_ = out;
        }
    }

    #[allow(clippy::needless_range_loop)]
    fn subdivide_face(&mut self, fid: u32, n: usize, names: &[String]) -> Res<()> {
        let fc = self.faces[&fid].clone();
        let lbl = self.label(f(fid));
        if fc.loop_.len() != 4 {
            return Err(invalid(format!(
                "`{lbl}` has {} sides; a face grid needs exactly 4",
                fc.loop_.len()
            ))
            .help(
                "subdivide the edges individually, or delete the face and rebuild it from a quad",
            ));
        }
        if fc
            .loop_
            .iter()
            .any(|&(eid, _)| self.edges[&eid].curve != Curve::Straight)
        {
            return Err(invalid(format!("`{lbl}` has curved edges"))
                .help("call `straighten()` on its edges first, then subdivide the face"));
        }
        self.expect_names(names, n * n, "this grid")?;
        self.validate_names(names)?;
        let corners: Vec<u32> = fc
            .loop_
            .iter()
            .map(|&(eid, s)| self.start_vertex(eid, s))
            .collect();
        let cp: Vec<P> = corners.iter().map(|&c| self.pos(c)).collect();
        // Subdivide the four boundary edges (shared with neighbouring faces, which stay consistent).
        let mut chains: Vec<(Vec<u32>, Vec<u32>)> = vec![];
        for &(eid, s) in &fc.loop_ {
            let (mut vs, mut es) = self.split_edge(eid, n);
            if s < 0 {
                vs.reverse();
                es.reverse();
            }
            chains.push((vs, es));
        }
        let parent = Some(f(fid));
        let mut vg = vec![vec![0u32; n + 1]; n + 1]; // vg[i][j]
        let mut he = vec![vec![0u32; n + 1]; n]; // he[i][j]: (i,j) -> (i+1,j)
        let mut ve = vec![vec![0u32; n]; n + 1]; // ve[i][j]: (i,j) -> (i,j+1)
        for i in 0..=n {
            vg[i][0] = chains[0].0[i]; // c0 -> c1
            vg[i][n] = chains[2].0[n - i]; // c3 -> c2 is the reverse of c2 -> c3
        }
        for j in 0..=n {
            vg[n][j] = chains[1].0[j]; // c1 -> c2
            vg[0][j] = chains[3].0[n - j]; // c0 -> c3 is the reverse of c3 -> c0
        }
        for i in 0..n {
            he[i][0] = chains[0].1[i];
            he[i][n] = chains[2].1[n - 1 - i];
        }
        for j in 0..n {
            ve[n][j] = chains[1].1[j];
            ve[0][j] = chains[3].1[n - 1 - j];
        }
        for j in 1..n {
            for i in 1..n {
                let (s, t) = (i as f64 / n as f64, j as f64 / n as f64);
                let bottom = lerp(cp[0], cp[1], s);
                let top = lerp(cp[3], cp[2], s);
                let p = lerp(bottom, top, t);
                vg[i][j] = self.new_vertex(p[0], p[1], parent);
            }
        }
        for j in 1..n {
            for i in 0..n {
                he[i][j] = self.new_edge(vg[i][j], vg[i + 1][j], parent);
            }
        }
        for i in 1..n {
            for j in 0..n {
                ve[i][j] = self.new_edge(vg[i][j], vg[i][j + 1], parent);
            }
        }
        let mut new_faces = vec![];
        for j in 0..n {
            for i in 0..n {
                let steps = [
                    (he[i][j], vg[i][j]),
                    (ve[i + 1][j], vg[i + 1][j]),
                    (he[i][j + 1], vg[i + 1][j + 1]),
                    (ve[i][j], vg[i][j + 1]),
                ];
                let loop_: Vec<(u32, i8)> = steps
                    .iter()
                    .map(|&(eid, from)| (eid, self.sign_from(eid, from)))
                    .collect();
                new_faces.push(self.new_face(loop_, parent));
            }
        }
        self.apply_names(Kind::F, &new_faces, names);
        let note = format!("subdivided into {}", fmt_ids(Kind::F, &new_faces));
        self.retire(f(fid), State::Replaced, note);
        Ok(())
    }

    // ---- extrude --------------------------------------------------------------------------

    /// Push a boundary (or free) edge outward by `by`, creating two vertices, three edges and a face.
    pub fn extrude(&mut self, r: ElemRef, by: f64) -> Res<()> {
        self.require_live(r, Some(Kind::E), "extrude")?;
        let lbl = self.label(r);
        if by == 0.0 || !by.is_finite() {
            return Err(invalid("`extrude` needs a non-zero distance")
                .help("write `extrude(by: 40)`; negative values push inward"));
        }
        let ed = self.edges[&r.id].clone();
        let (pa, pb) = (self.pos(ed.a), self.pos(ed.b));
        let users: Vec<u32> = self
            .live_ids(Kind::F)
            .into_iter()
            .filter(|fid| self.faces[fid].loop_.iter().any(|x| x.0 == r.id))
            .collect();
        if users.len() > 1 {
            return Err(invalid(format!(
                "`{lbl}` is shared by {} faces, so there is no single outward side",
                users.len()
            ))
            .help("extrude edges on the boundary of the shape"));
        }
        let d = [pb[0] - pa[0], pb[1] - pa[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len < 1e-12 {
            return Err(invalid(format!("`{lbl}` has zero length")));
        }
        // "Right of travel" on screen is (-dy, dx). For a counter-clockwise face the outside is on the right.
        let mut dir = if let Some(&fid) = users.first() {
            let s = self.faces[&fid]
                .loop_
                .iter()
                .find(|x| x.0 == r.id)
                .unwrap()
                .1;
            let travel = if s > 0 { d } else { [-d[0], -d[1]] };
            let sign = if self.face_area(fid) < 0.0 { 1.0 } else { -1.0 };
            [-travel[1] * sign, travel[0] * sign]
        } else {
            [-d[1], d[0]]
        };
        dir = [dir[0] / len, dir[1] / len];
        let parent = Some(r);
        let a2 = self.new_vertex(pa[0] + dir[0] * by, pa[1] + dir[1] * by, parent);
        let b2 = self.new_vertex(pb[0] + dir[0] * by, pb[1] + dir[1] * by, parent);
        let e1 = self.new_edge(ed.a, a2, parent);
        let e2 = self.new_edge(a2, b2, parent);
        let e3 = self.new_edge(b2, ed.b, parent);
        // ring a -> a2 -> b2 -> b -> a (back along the original edge), wound counter-clockwise on screen
        let mut loop_ = vec![
            (e1, 1i8),
            (e2, 1),
            (e3, 1),
            (r.id, self.sign_from(r.id, ed.b)),
        ];
        let ring = [
            pa,
            [pa[0] + dir[0] * by, pa[1] + dir[1] * by],
            [pb[0] + dir[0] * by, pb[1] + dir[1] * by],
            pb,
        ];
        if shoelace(&ring) > 0.0 {
            loop_ = loop_.into_iter().rev().map(|(i, s)| (i, -s)).collect();
        }
        self.new_face(loop_, parent);
        self.post_edit("extrude");
        Ok(())
    }

    // ---- bevel / round --------------------------------------------------------------------

    /// Cut a corner with a straight edge, `radius` away from the corner along both neighbours.
    pub fn bevel(&mut self, r: ElemRef, radius: f64) -> Res<()> {
        self.soften(r, radius, false, "bevel")
    }
    /// Like `bevel` but joins the cut with a quadratic curve through the corner (a rounded corner).
    pub fn round(&mut self, r: ElemRef, radius: f64) -> Res<()> {
        self.soften(r, radius, true, "round")
    }

    fn soften(&mut self, r: ElemRef, radius: f64, rounded: bool, op: &str) -> Res<()> {
        self.require_live(r, Some(Kind::V), op)?;
        let lbl = self.label(r);
        let inc: Vec<u32> = self
            .live_ids(Kind::E)
            .into_iter()
            .filter(|i| self.edges[i].a == r.id || self.edges[i].b == r.id)
            .collect();
        if inc.len() != 2 {
            return Err(invalid(format!(
                "`{op}` needs a corner with exactly two edges, `{lbl}` has {}",
                inc.len()
            )));
        }
        if inc.iter().any(|i| self.edges[i].curve != Curve::Straight) {
            return Err(invalid(format!("a corner of `{lbl}` has a curved edge"))
                .help("call `straighten()` on the neighbouring edges, or bevel before bending"));
        }
        let (ea, eb) = (inc[0], inc[1]); // lower id first
        let other = |s: &Shape, id: u32| {
            let ed = &s.edges[&id];
            if ed.a == r.id {
                ed.b
            } else {
                ed.a
            }
        };
        let (pa, pb, pv) = (
            self.pos(other(self, ea)),
            self.pos(other(self, eb)),
            self.pos(r.id),
        );
        let len = |p: P| ((p[0] - pv[0]).powi(2) + (p[1] - pv[1]).powi(2)).sqrt();
        let (la, lb) = (len(pa), len(pb));
        let max = la.min(lb);
        if !(radius > 0.0 && radius < max) {
            return Err(invalid(format!(
                "`{op}` radius must be between 0 and {max:.6} (exclusive) at `{lbl}`"
            ))
            .help(format!("the shortest edge at this corner is {max:.3} long")));
        }
        let along = |p: P, l: f64| {
            [
                pv[0] + (p[0] - pv[0]) / l * radius,
                pv[1] + (p[1] - pv[1]) / l * radius,
            ]
        };
        let (qa, qb) = (along(pa, la), along(pb, lb));
        let va = self.new_vertex(qa[0], qa[1], Some(r));
        let vb = self.new_vertex(qb[0], qb[1], Some(r));
        for (eid, nv) in [(ea, va), (eb, vb)] {
            let ed = self.edges.get_mut(&eid).unwrap();
            if ed.a == r.id {
                ed.a = nv;
            } else {
                ed.b = nv;
            }
        }
        let ne = self.new_edge(va, vb, Some(r));
        self.copy_edge_style(ea, ne);
        if rounded {
            self.edges.get_mut(&ne).unwrap().curve = Curve::Quad(to_local(qa, qb, pv));
        }
        for fid in self.live_ids(Kind::F) {
            let lp = self.faces[&fid].loop_.clone();
            let n = lp.len();
            for i in 0..n {
                let (e1, e2) = (lp[i].0, lp[(i + 1) % n].0);
                if (e1 == ea && e2 == eb) || (e1 == eb && e2 == ea) {
                    let from = self.end_vertex(lp[i].0, lp[i].1);
                    let s = self.sign_from(ne, from);
                    self.faces
                        .get_mut(&fid)
                        .unwrap()
                        .loop_
                        .insert(i + 1, (ne, s));
                    break;
                }
            }
        }
        let note = format!(
            "{} into {}",
            if rounded { "rounded" } else { "beveled" },
            fmt_ids(Kind::V, &[va, vb])
        );
        self.retire(r, State::Replaced, note);
        self.post_edit(op);
        Ok(())
    }

    // ---- smooth (Chaikin) -----------------------------------------------------------------

    /// Chaikin corner cutting, `levels` times. Needs closed outlines where every vertex has exactly two edges.
    pub fn smooth(&mut self, levels: u32) -> Res<()> {
        if levels < 1 {
            return Err(invalid("`smooth` needs at least 1 level")
                .help("write `smooth(2)` for two rounds of Chaikin subdivision"));
        }
        for _ in 0..levels {
            self.chaikin()?;
        }
        self.post_edit("smooth");
        Ok(())
    }

    fn chaikin(&mut self) -> Res<()> {
        let live_e = self.live_ids(Kind::E);
        if live_e.is_empty() {
            return Err(invalid(format!("`{}` has no edges to smooth", self.name)));
        }
        if let Some(&c) = live_e
            .iter()
            .find(|i| self.edges[i].curve != Curve::Straight)
        {
            return Err(invalid(format!(
                "`{}` is curved; smooth needs straight edges",
                self.label(e(c))
            ))
            .help(format!("call `{}.straighten()` first", self.label(e(c)))));
        }
        let mut deg: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for &eid in &live_e {
            let ed = &self.edges[&eid];
            deg.entry(ed.a).or_default().push(eid);
            deg.entry(ed.b).or_default().push(eid);
        }
        if let Some((vid, es)) = deg.iter().find(|(_, es)| es.len() != 2) {
            return Err(invalid(format!("`{}` has {} edges; smooth needs closed outlines where every vertex has exactly two", self.label(v(*vid)), es.len()))
                .help("use `bevel`/`round` to soften single corners")
                .help("or `delete` the extra edges first (for example the base edge after an `extrude`)"));
        }
        struct Ring {
            verts: Vec<u32>,
            edges: Vec<(u32, i8)>,
            faces: Vec<u32>,
        }
        let mut rings: Vec<Ring> = vec![];
        let mut covered: BTreeSet<u32> = BTreeSet::new();
        for fid in self.live_ids(Kind::F) {
            let verts = self.face_vertex_ids(fid);
            let uniq: BTreeSet<u32> = verts.iter().copied().collect();
            if uniq.len() != verts.len() {
                return Err(invalid(format!(
                    "`{}` touches itself; smooth needs simple outlines",
                    self.label(f(fid))
                )));
            }
            let mut key: Vec<u32> = self.faces[&fid].loop_.iter().map(|x| x.0).collect();
            key.sort();
            if let Some(r) = rings.iter_mut().find(|r| {
                let mut k: Vec<u32> = r.edges.iter().map(|x| x.0).collect();
                k.sort();
                k == key
            }) {
                r.faces.push(fid);
                continue;
            }
            covered.extend(uniq);
            rings.push(Ring {
                verts,
                edges: self.faces[&fid].loop_.clone(),
                faces: vec![fid],
            });
        }
        for &start in deg.keys() {
            if covered.contains(&start) {
                continue;
            }
            let (mut cur, mut verts, mut edges) = (start, vec![], vec![]);
            let mut eid = deg[&start][0];
            loop {
                let s = self.sign_from(eid, cur);
                verts.push(cur);
                edges.push((eid, s));
                covered.insert(cur);
                cur = self.end_vertex(eid, s);
                if cur == start {
                    break;
                }
                eid = *deg[&cur]
                    .iter()
                    .find(|&&x| x != eid)
                    .unwrap_or(&deg[&cur][0]);
            }
            rings.push(Ring {
                verts,
                edges,
                faces: vec![],
            });
        }
        for ring in rings {
            let n = ring.edges.len();
            let (mut qs, mut rs) = (vec![], vec![]);
            for i in 0..n {
                let (p, pn) = (self.pos(ring.verts[i]), self.pos(ring.verts[(i + 1) % n]));
                let parent = Some(e(ring.edges[i].0));
                let q = lerp(p, pn, 0.25);
                let r = lerp(p, pn, 0.75);
                qs.push(self.new_vertex(q[0], q[1], parent));
                rs.push(self.new_vertex(r[0], r[1], parent));
            }
            let mut new_loop = vec![];
            for i in 0..n {
                let inner = self.new_edge(qs[i], rs[i], Some(e(ring.edges[i].0)));
                let corner =
                    self.new_edge(rs[i], qs[(i + 1) % n], Some(v(ring.verts[(i + 1) % n])));
                self.copy_edge_style(ring.edges[i].0, corner);
                new_loop.push((inner, 1i8));
                new_loop.push((corner, 1i8));
            }
            for fid in &ring.faces {
                self.faces.get_mut(fid).unwrap().loop_ = new_loop.clone();
            }
            for &(eid, _) in &ring.edges {
                self.retire(e(eid), State::Replaced, "smoothed away".into());
            }
            for &vid in &ring.verts {
                self.retire(v(vid), State::Replaced, "smoothed away".into());
            }
        }
        Ok(())
    }

    // ---- merge / delete -------------------------------------------------------------------

    /// Weld vertex `b` into vertex `a` (`a` survives; `b` is retired).
    pub fn merge(&mut self, a: ElemRef, b: ElemRef, at: MergeAt) -> Res<()> {
        self.require_live(a, Some(Kind::V), "merge")?;
        self.require_live(b, Some(Kind::V), "merge")?;
        if a.id == b.id {
            return Err(invalid("cannot merge a vertex with itself"));
        }
        let (pa, pb) = (self.pos(a.id), self.pos(b.id));
        let np = match at {
            MergeAt::Center => lerp(pa, pb, 0.5),
            MergeAt::First => pa,
            MergeAt::Last => pb,
        };
        {
            let va = self.verts.get_mut(&a.id).unwrap();
            va.x = np[0];
            va.y = np[1];
        }
        for ed in self
            .edges
            .values_mut()
            .filter(|x| x.meta.state == State::Live)
        {
            if ed.a == b.id {
                ed.a = a.id;
            }
            if ed.b == b.id {
                ed.b = a.id;
            }
        }
        let collapsed: Vec<u32> = self
            .live_ids(Kind::E)
            .into_iter()
            .filter(|i| self.edges[i].a == self.edges[i].b)
            .collect();
        for fc in self
            .faces
            .values_mut()
            .filter(|x| x.meta.state == State::Live)
        {
            fc.loop_.retain(|x| !collapsed.contains(&x.0));
        }
        for c in &collapsed {
            self.retire(
                e(*c),
                State::Deleted,
                format!(
                    "collapsed when {} was merged into {}",
                    self.label(b),
                    self.label(a)
                ),
            );
        }
        for fid in self.live_ids(Kind::F) {
            if self.faces[&fid].loop_.len() < 3 {
                self.retire(
                    f(fid),
                    State::Deleted,
                    "removed because a merge left it with fewer than 3 edges".into(),
                );
                if self.warn_on_edit {
                    self.warnings.push(
                        Diagnostic::new(
                            "W-SHP-010",
                            format!(
                                "degenerate face `{}` collapsed and was removed by `merge`",
                                self.label(f(fid))
                            ),
                        )
                        .help("merge fewer vertices, or rebuild the face"),
                    );
                }
            }
        }
        let note = format!("merged into {}", self.label(a));
        self.retire(b, State::Replaced, note);
        self.post_edit("merge");
        Ok(())
    }

    /// Delete an element; the IDs of everything else are untouched. Deleting a vertex deletes its
    /// edges and the faces using them; deleting an edge deletes the faces using it; a face goes alone.
    pub fn delete(&mut self, r: ElemRef) -> Res<()> {
        self.require_live(r, None, "delete")?;
        match r.kind {
            Kind::V => {
                let inc: Vec<u32> = self
                    .live_ids(Kind::E)
                    .into_iter()
                    .filter(|i| self.edges[i].a == r.id || self.edges[i].b == r.id)
                    .collect();
                for eid in inc {
                    self.delete_edge(eid);
                }
                self.retire(r, State::Deleted, "deleted".into());
            }
            Kind::E => self.delete_edge(r.id),
            Kind::F => self.retire(r, State::Deleted, "deleted".into()),
        }
        self.post_edit("delete");
        Ok(())
    }
    fn delete_edge(&mut self, eid: u32) {
        let users: Vec<u32> = self
            .live_ids(Kind::F)
            .into_iter()
            .filter(|fid| self.faces[fid].loop_.iter().any(|x| x.0 == eid))
            .collect();
        for fid in users {
            self.retire(
                f(fid),
                State::Deleted,
                format!("deleted because {} was deleted", self.label(e(eid))),
            );
        }
        self.retire(e(eid), State::Deleted, "deleted".into());
    }

    // ---- curves ---------------------------------------------------------------------------

    /// Bend an edge into a quadratic Bézier whose end tangents deviate from the chord by `turns`
    /// (full turns, like every `Angle`). Positive bulges to the right of travel on screen.
    pub fn bend(&mut self, r: ElemRef, turns: f64) -> Res<()> {
        self.require_live(r, Some(Kind::E), "bend")?;
        if turns.abs() >= 0.25 || !turns.is_finite() {
            return Err(invalid("`bend` angle must be between -90.degrees and 90.degrees (exclusive)")
                .help("a quarter turn would need an infinitely long handle; use `set_handle` or subdivide first"));
        }
        self.edges.get_mut(&r.id).unwrap().curve =
            Curve::Quad([0.5, (turns * std::f64::consts::TAU).tan() / 2.0]);
        self.post_edit("bend");
        Ok(())
    }
    /// Make an edge quadratic with an explicit absolute control point.
    pub fn set_handle(&mut self, r: ElemRef, x: f64, y: f64) -> Res<()> {
        self.require_live(r, Some(Kind::E), "set_handle")?;
        let ed = self.edges[&r.id].clone();
        let local = to_local(self.pos(ed.a), self.pos(ed.b), [x, y]);
        self.edges.get_mut(&r.id).unwrap().curve = Curve::Quad(local);
        self.post_edit("set_handle");
        Ok(())
    }
    /// Make an edge cubic with two explicit absolute control points.
    pub fn set_handles(&mut self, r: ElemRef, h1: P, h2: P) -> Res<()> {
        self.require_live(r, Some(Kind::E), "set_handles")?;
        let ed = self.edges[&r.id].clone();
        let (p0, p1) = (self.pos(ed.a), self.pos(ed.b));
        self.edges.get_mut(&r.id).unwrap().curve =
            Curve::Cubic(to_local(p0, p1, h1), to_local(p0, p1, h2));
        self.post_edit("set_handles");
        Ok(())
    }
    pub fn straighten(&mut self, r: ElemRef) -> Res<()> {
        self.require_live(r, Some(Kind::E), "straighten")?;
        self.edges.get_mut(&r.id).unwrap().curve = Curve::Straight;
        self.post_edit("straighten");
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Dissolve, inset, flip, mirror, group moves
// ---------------------------------------------------------------------------------------------

/// A mirror line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Axis {
    /// The vertical line `x = value`.
    X(f64),
    /// The horizontal line `y = value`.
    Y(f64),
    /// The line through two distinct points.
    Line(P, P),
}

impl Axis {
    fn check(self) -> Res<()> {
        if let Axis::Line(a, b) = self {
            if (b[0] - a[0]).hypot(b[1] - a[1]) < 1e-12 {
                return Err(invalid("a mirror line needs two different points")
                    .help("write `line: ((0, 0), (100, 100))`"));
            }
        }
        Ok(())
    }
    /// Signed distance from the line.
    fn signed(self, p: P) -> f64 {
        match self {
            Axis::X(a) => p[0] - a,
            Axis::Y(a) => p[1] - a,
            Axis::Line(a, b) => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l = dx.hypot(dy);
                ((p[0] - a[0]) * -dy + (p[1] - a[1]) * dx) / l
            }
        }
    }
    pub fn distance(self, p: P) -> f64 {
        self.signed(p).abs()
    }
    pub fn reflect(self, p: P) -> P {
        match self {
            Axis::X(a) => [2.0 * a - p[0], p[1]],
            Axis::Y(a) => [p[0], 2.0 * a - p[1]],
            Axis::Line(a, b) => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l = dx.hypot(dy);
                let (nx, ny) = (-dy / l, dx / l);
                let d = self.signed(p);
                [p[0] - 2.0 * d * nx, p[1] - 2.0 * d * ny]
            }
        }
    }
}

fn negate_curve(c: &Curve) -> Curve {
    // a reflection reverses handedness, so the sideways component of every handle changes sign
    match c {
        Curve::Straight => Curve::Straight,
        Curve::Quad([u, w]) => Curve::Quad([*u, -w]),
        Curve::Cubic([u1, w1], [u2, w2]) => Curve::Cubic([*u1, -w1], [*u2, -w2]),
    }
}

impl Shape {
    /// The mean of the distinct vertices of a group of elements.
    pub fn group_anchor(&self, refs: &[ElemRef]) -> P {
        let ids: BTreeSet<u32> = refs
            .iter()
            .flat_map(|r| self.element_vertices(*r))
            .collect();
        let n = ids.len().max(1) as f64;
        [
            ids.iter().map(|i| self.pos(*i)[0]).sum::<f64>() / n,
            ids.iter().map(|i| self.pos(*i)[1]).sum::<f64>() / n,
        ]
    }

    /// Move a group of elements together; a vertex shared by several of them moves exactly once.
    pub fn move_group_by(&mut self, refs: &[ElemRef], dx: f64, dy: f64) -> Res<()> {
        for r in refs {
            self.require_live(*r, None, "move_by")?;
        }
        let ids: BTreeSet<u32> = refs
            .iter()
            .flat_map(|r| self.element_vertices(*r))
            .collect();
        let ids: Vec<u32> = ids.into_iter().collect();
        self.shift(&ids, dx, dy);
        self.post_edit("move_by");
        Ok(())
    }

    /// Remove faces left with fewer than 3 edges by an edit, warning about each (`W-SHP-010`).
    fn prune_short_faces(&mut self, by: &str) {
        for fid in self.live_ids(Kind::F) {
            if self.faces[&fid].loop_.len() < 3 {
                self.retire(
                    f(fid),
                    State::Deleted,
                    format!("removed because {by} left it with fewer than 3 edges"),
                );
                if self.warn_on_edit {
                    self.warnings.push(
                        Diagnostic::new(
                            "W-SHP-010",
                            format!(
                                "degenerate face `{}` collapsed and was removed by {by}",
                                self.label(f(fid))
                            ),
                        )
                        .help("edit fewer vertices, or rebuild the face"),
                    );
                }
            }
        }
    }

    // ---- dissolve -------------------------------------------------------------------------

    /// Remove an edge that two faces share, merging them into one (the lower face ID survives), or
    /// remove a vertex that has exactly two edges, merging those edges into one.
    pub fn dissolve(&mut self, r: ElemRef) -> Res<()> {
        self.require_live(r, None, "dissolve")?;
        match r.kind {
            Kind::E => self.dissolve_edge(r.id)?,
            Kind::V => self.dissolve_vertex(r.id)?,
            Kind::F => {
                return Err(invalid(format!(
                    "`{}` is a face; `dissolve` works on an edge or a vertex",
                    self.label(r)
                ))
                .help(
                    "dissolve an edge to merge the two faces it separates, or `delete` the face",
                ))
            }
        }
        self.post_edit("dissolve");
        Ok(())
    }

    fn dissolve_edge(&mut self, eid: u32) -> Res<()> {
        let lbl = self.label(e(eid));
        let uses: Vec<(u32, usize)> = self
            .live_ids(Kind::F)
            .into_iter()
            .flat_map(|fid| {
                self.faces[&fid]
                    .loop_
                    .iter()
                    .enumerate()
                    .filter(|(_, x)| x.0 == eid)
                    .map(move |(i, _)| (fid, i))
                    .collect::<Vec<_>>()
            })
            .collect();
        if uses.len() != 2 || uses[0].0 == uses[1].0 {
            return Err(invalid(format!("`{lbl}` is used by {} face side(s); `dissolve` needs an edge between exactly two faces", uses.len()))
                .help("use `delete` to remove an edge together with the faces using it"));
        }
        let ((fa, i), (fb, j)) = (uses[0], uses[1]);
        let (la, lb) = (self.faces[&fa].loop_.clone(), self.faces[&fb].loop_.clone());
        if la[i].1 == lb[j].1 {
            return Err(invalid(format!(
                "the two faces around `{lbl}` are wound in opposite senses and cannot be merged"
            ))
            .help("`flip` one of them, or rebuild the faces with the same winding"));
        }
        let mut merged = Vec::with_capacity(la.len() + lb.len() - 2);
        merged.extend_from_slice(&la[i + 1..]);
        merged.extend_from_slice(&la[..i]);
        merged.extend_from_slice(&lb[j + 1..]);
        merged.extend_from_slice(&lb[..j]);
        self.faces.get_mut(&fa).unwrap().loop_ = merged;
        self.retire(
            f(fb),
            State::Replaced,
            format!("dissolved into {}", self.label(f(fa))),
        );
        self.meta_mut(f(fb)).unwrap().children.push(fa);
        self.retire(e(eid), State::Deleted, "dissolved".into());
        Ok(())
    }

    fn dissolve_vertex(&mut self, vid: u32) -> Res<()> {
        let lbl = self.label(v(vid));
        let inc: Vec<u32> = self
            .live_ids(Kind::E)
            .into_iter()
            .filter(|i| self.edges[i].a == vid || self.edges[i].b == vid)
            .collect();
        if inc.len() != 2 {
            return Err(invalid(format!(
                "`{lbl}` has {} edges; `dissolve` needs a vertex with exactly two",
                inc.len()
            ))
            .help("use `delete` to remove a vertex together with its edges"));
        }
        if inc.iter().any(|i| self.edges[i].curve != Curve::Straight) {
            return Err(invalid(format!(
                "an edge at `{lbl}` is curved, so merging the two edges would lose the curve"
            ))
            .help("`straighten()` the edges first if that is what you want"));
        }
        let (ea, eb) = (inc[0], inc[1]);
        let far = |s: &Shape, id: u32| {
            let ed = &s.edges[&id];
            if ed.a == vid {
                ed.b
            } else {
                ed.a
            }
        };
        let (fa, fb) = (far(self, ea), far(self, eb));
        if fa == fb {
            return Err(invalid(format!(
                "both edges at `{lbl}` join the same two vertices"
            )));
        }
        {
            let ed = self.edges.get_mut(&ea).unwrap();
            if ed.a == vid {
                ed.a = fb;
            } else {
                ed.b = fb;
            }
        }
        for fc in self
            .faces
            .values_mut()
            .filter(|x| x.meta.state == State::Live)
        {
            fc.loop_.retain(|x| x.0 != eb);
        }
        self.retire(
            e(eb),
            State::Deleted,
            format!("dissolved together with {lbl}"),
        );
        self.retire(v(vid), State::Deleted, "dissolved".into());
        self.prune_short_faces("a dissolve");
        Ok(())
    }

    // ---- inset ----------------------------------------------------------------------------

    /// Shrink a face inward by `amount` (the distance from every edge), leaving a ring of new faces.
    /// The face keeps its ID and becomes the inner face.
    pub fn inset(&mut self, r: ElemRef, amount: f64) -> Res<()> {
        self.require_live(r, Some(Kind::F), "inset")?;
        let lbl = self.label(r);
        if !(amount > 0.0 && amount.is_finite()) {
            return Err(invalid("`inset` needs a positive amount").help("write `inset(20)`"));
        }
        let fc = self.faces[&r.id].clone();
        if let Some(&(eid, _)) = fc
            .loop_
            .iter()
            .find(|x| self.edges[&x.0].curve != Curve::Straight)
        {
            return Err(invalid(format!(
                "`{lbl}` has a curved edge (`{}`)",
                self.label(e(eid))
            ))
            .help("call `straighten()` on its edges first; inset works on straight-sided faces"));
        }
        let n = fc.loop_.len();
        let corners: Vec<u32> = fc
            .loop_
            .iter()
            .map(|&(eid, s)| self.start_vertex(eid, s))
            .collect();
        let p: Vec<P> = corners.iter().map(|c| self.pos(*c)).collect();
        let area = shoelace(&p);
        if area.abs() < 1e-12 {
            return Err(invalid(format!("`{lbl}` has no area")));
        }
        // inward unit normal of every edge i (from p[i] to p[i + 1]): the left side for a counter-clockwise face
        let mut normal = Vec::with_capacity(n);
        for i in 0..n {
            let (a, b) = (p[i], p[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy);
            if l < 1e-12 {
                return Err(invalid(format!("`{lbl}` has an edge of zero length")));
            }
            let left = [dy / l, -dx / l];
            normal.push(if area < 0.0 {
                left
            } else {
                [-left[0], -left[1]]
            });
        }
        let mut q: Vec<P> = Vec::with_capacity(n);
        for i in 0..n {
            let (n0, n1) = (normal[(i + n - 1) % n], normal[i]);
            let denom = 1.0 + n0[0] * n1[0] + n0[1] * n1[1];
            if denom < 1e-9 {
                return Err(invalid(format!(
                    "`{lbl}` has a corner that folds back on itself"
                )));
            }
            let k = amount / denom;
            q.push([p[i][0] + (n0[0] + n1[0]) * k, p[i][1] + (n0[1] + n1[1]) * k]);
        }
        let new_area = shoelace(&q);
        if new_area * area <= 0.0 || new_area.abs() >= area.abs() {
            return Err(
                invalid(format!("an inset of {amount} is too large for `{lbl}`"))
                    .help("the inner face would collapse or turn inside out; use a smaller amount"),
            );
        }
        let parent = Some(r);
        let qv: Vec<u32> = q
            .iter()
            .map(|pt| self.new_vertex(pt[0], pt[1], parent))
            .collect();
        let spokes: Vec<u32> = (0..n)
            .map(|i| self.new_edge(corners[i], qv[i], parent))
            .collect();
        let inner: Vec<u32> = (0..n)
            .map(|i| self.new_edge(qv[i], qv[(i + 1) % n], parent))
            .collect();
        for i in 0..n {
            let next = (i + 1) % n;
            let loop_ = vec![
                fc.loop_[i],
                (spokes[next], 1),
                (inner[i], -1),
                (spokes[i], -1),
            ];
            self.new_face(loop_, parent);
        }
        self.faces.get_mut(&r.id).unwrap().loop_ = inner.iter().map(|&i| (i, 1i8)).collect();
        self.post_edit("inset");
        Ok(())
    }

    // ---- flip / mirror ----------------------------------------------------------------------

    /// Reflect the whole shape in place across `axis`. IDs are unchanged; face loops are reversed
    /// so the winding convention is kept, and curve handles are reflected.
    pub fn flip(&mut self, axis: Axis) -> Res<()> {
        axis.check()?;
        for id in self.live_ids(Kind::V) {
            let q = axis.reflect(self.pos(id));
            let vx = self.verts.get_mut(&id).unwrap();
            vx.x = q[0];
            vx.y = q[1];
        }
        for ed in self
            .edges
            .values_mut()
            .filter(|x| x.meta.state == State::Live)
        {
            ed.curve = negate_curve(&ed.curve);
        }
        for fc in self
            .faces
            .values_mut()
            .filter(|x| x.meta.state == State::Live)
        {
            fc.loop_ = fc.loop_.iter().rev().map(|&(i, s)| (i, -s)).collect();
        }
        self.post_edit("flip");
        Ok(())
    }

    /// Add a reflected copy of the whole shape (or of the given faces) across `axis`, joined to the
    /// original: vertices within `weld` of the axis are shared instead of copied, and so are straight
    /// edges lying on it. New elements record their original as `parent`.
    pub fn mirror(&mut self, axis: Axis, faces: Option<&[u32]>, weld: f64) -> Res<()> {
        axis.check()?;
        let src_faces: Vec<u32> = match faces {
            Some(list) => {
                for &id in list {
                    self.get_ref(Kind::F, id)?;
                    self.require_live(f(id), Some(Kind::F), "mirror")?;
                }
                list.iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            }
            None => self.live_ids(Kind::F),
        };
        let src_edges: Vec<u32> = match faces {
            Some(_) => src_faces
                .iter()
                .flat_map(|fid| self.faces[fid].loop_.iter().map(|x| x.0))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            None => self.live_ids(Kind::E),
        };
        let src_verts: Vec<u32> = match faces {
            Some(_) => src_edges
                .iter()
                .flat_map(|eid| [self.edges[eid].a, self.edges[eid].b])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            None => self.live_ids(Kind::V),
        };
        if !src_verts
            .iter()
            .any(|&vid| axis.distance(self.pos(vid)) > weld)
        {
            return Err(
                invalid("nothing to mirror: every vertex lies on the mirror line")
                    .help("move the shape off the line, or pass `faces:` that are not on it"),
            );
        }
        let mut vmap: BTreeMap<u32, u32> = BTreeMap::new();
        for &vid in &src_verts {
            let pt = self.pos(vid);
            if axis.distance(pt) <= weld {
                vmap.insert(vid, vid);
            } else {
                let qpt = axis.reflect(pt);
                vmap.insert(vid, self.new_vertex(qpt[0], qpt[1], Some(v(vid))));
            }
        }
        let mut emap: BTreeMap<u32, u32> = BTreeMap::new();
        for &eid in &src_edges {
            let ed = self.edges[&eid].clone();
            let (a2, b2) = (vmap[&ed.a], vmap[&ed.b]);
            if a2 == ed.a && b2 == ed.b && ed.curve == Curve::Straight {
                emap.insert(eid, eid); // lies on the axis: shared
            } else {
                let ne = self.new_edge(a2, b2, Some(e(eid)));
                self.edges.get_mut(&ne).unwrap().curve = negate_curve(&ed.curve);
                emap.insert(eid, ne);
            }
        }
        for &fid in &src_faces {
            let loop_: Vec<(u32, i8)> = self.faces[&fid]
                .loop_
                .iter()
                .rev()
                .map(|&(eid, s)| (emap[&eid], -s))
                .collect();
            self.new_face(loop_, Some(f(fid)));
        }
        self.post_edit("mirror");
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------------------------

/// A closed polygon: v1..vN in the given order, eN from vN to vN+1 (the last closes to v1), and f1.
fn closed(points: &[P]) -> Shape {
    let mut s = Shape::new("shape");
    let n = points.len() as u32;
    for p in points {
        s.new_vertex(p[0], p[1], None);
    }
    let mut lp = vec![];
    for i in 1..=n {
        let eid = s.new_edge(i, i % n + 1, None);
        lp.push((eid, 1i8));
    }
    s.new_face(lp, None);
    s
}

/// A path of cubic Bezier pieces (from `fit_curve`) as a shape: one vertex at each join and one cubic edge per piece,
/// and a face if the path is `closed`. The shape's vertices are v1, v2, ... in path order.
pub fn bezier_shape(segs: &[crate::fit::FitSeg], closed_path: bool) -> Shape {
    let mut s = Shape::new("shape");
    let n = segs.len() as u32;
    for seg in segs {
        s.new_vertex(seg.p0[0], seg.p0[1], None);
    }
    if !closed_path {
        if let Some(last) = segs.last() {
            s.new_vertex(last.p3[0], last.p3[1], None);
        }
    }
    let mut lp = vec![];
    for (i, seg) in segs.iter().enumerate() {
        let a = i as u32 + 1;
        let b = if closed_path { (a % n) + 1 } else { a + 1 };
        let eid = s.new_edge(a, b, None);
        let (c1, c2) = (
            to_local(seg.p0, seg.p3, seg.c1),
            to_local(seg.p0, seg.p3, seg.c2),
        );
        if let Some(e) = s.edges.get_mut(&eid) {
            e.curve = Curve::Cubic(c1, c2);
        }
        lp.push((eid, 1i8));
    }
    if closed_path && !lp.is_empty() {
        s.new_face(lp, None);
    }
    s
}

/// A square centred on `at`: v1..v4 counter-clockwise (on screen) from the top-left.
pub fn square(size: f64, at: P) -> Shape {
    rectangle(size, size, at)
}
pub fn rectangle(width: f64, height: f64, at: P) -> Shape {
    let (hw, hh) = (width / 2.0, height / 2.0);
    closed(&[
        [at[0] - hw, at[1] - hh],
        [at[0] - hw, at[1] + hh],
        [at[0] + hw, at[1] + hh],
        [at[0] + hw, at[1] - hh],
    ])
}
/// Equilateral triangle pointing up with side `size`: v1 top, v2 bottom-left, v3 bottom-right.
pub fn triangle(size: f64, at: P) -> Shape {
    let r = size / 3f64.sqrt();
    closed(&[
        [at[0], at[1] - r],
        [at[0] - size / 2.0, at[1] + r / 2.0],
        [at[0] + size / 2.0, at[1] + r / 2.0],
    ])
}
/// A regular n-gon: v1 at the top, then counter-clockwise on screen.
pub fn circle(n: usize, radius: f64, at: P) -> Res<Shape> {
    if n < 3 {
        return Err(invalid("`circle` needs at least 3 sides")
            .help("write `circle(16, radius: 50)` for a smooth-looking circle"));
    }
    let pts: Vec<P> = (0..n)
        .map(|k| {
            let a = -std::f64::consts::FRAC_PI_2 - std::f64::consts::TAU * k as f64 / n as f64;
            [at[0] + radius * a.cos(), at[1] + radius * a.sin()]
        })
        .collect();
    Ok(closed(&pts))
}
pub fn polygon(points: &[P]) -> Res<Shape> {
    if points.len() < 3 {
        return Err(invalid(format!(
            "`polygon` needs at least 3 points, got {}",
            points.len()
        ))
        .help("use `line(a, b)` for two points"));
    }
    Ok(closed(points))
}
/// A polyline through `points` (v1..vN in order, one straight edge between neighbours); with `closed`, the last
/// joins the first and there is a face. Needs two points (three if closed).
pub fn polyline(points: &[P], closed_path: bool) -> Res<Shape> {
    let need = if closed_path { 3 } else { 2 };
    if points.len() < need {
        return Err(invalid(format!(
            "a {} curve needs at least {need} points, got {}",
            if closed_path { "closed" } else { "open" },
            points.len()
        )));
    }
    if closed_path {
        return Ok(closed(points));
    }
    let mut s = Shape::new("shape");
    for p in points {
        s.new_vertex(p[0], p[1], None);
    }
    for i in 1..points.len() as u32 {
        s.new_edge(i, i + 1, None);
    }
    Ok(s)
}
/// A shape of loose segments: vertex k at `points[k]` and one straight edge per `(a, b)` pair of point indices
/// (used for contours, which have no face).
pub fn segments(points: &[P], pairs: &[(usize, usize)]) -> Shape {
    let mut s = Shape::new("shape");
    for p in points {
        s.new_vertex(p[0], p[1], None);
    }
    for (a, b) in pairs {
        s.new_edge(*a as u32 + 1, *b as u32 + 1, None);
    }
    s
}
/// Two vertices and one edge, no face.
pub fn line(a: P, b: P) -> Shape {
    let mut s = Shape::new("shape");
    s.new_vertex(a[0], a[1], None);
    s.new_vertex(b[0], b[1], None);
    s.new_edge(1, 2, None);
    s
}

// ---------------------------------------------------------------------------------------------
// Style setters, the pivot, and radial_array
// ---------------------------------------------------------------------------------------------

impl Shape {
    /// Set (or, with `None`, reset to the shape's) fill colour of one face.
    pub fn set_face_fill(&mut self, r: ElemRef, c: Option<Color>) -> Res<()> {
        self.require_live(r, Some(Kind::F), "fill")?;
        self.faces.get_mut(&r.id).unwrap().fill = c;
        Ok(())
    }
    pub fn set_edge_stroke(&mut self, r: ElemRef, c: Option<Color>) -> Res<()> {
        self.require_live(r, Some(Kind::E), "stroke")?;
        self.edges.get_mut(&r.id).unwrap().stroke = c;
        Ok(())
    }
    pub fn set_edge_width(&mut self, r: ElemRef, w: Option<f64>) -> Res<()> {
        self.require_live(r, Some(Kind::E), "stroke_width")?;
        if w.is_some_and(|x| !(x >= 0.0 && x.is_finite())) {
            return Err(
                invalid("a stroke width cannot be negative").help("write `stroke_width = 3`")
            );
        }
        self.edges.get_mut(&r.id).unwrap().width = w;
        Ok(())
    }

    /// The current transform; an unfrozen pivot follows the bounding-box centre.
    pub fn transform(&self) -> Transform {
        Transform {
            position: self.position,
            scale: self.scale,
            rotation: self.rotation,
            pivot: self.pivot.unwrap_or_else(|| self.bbox_center()),
        }
    }
    /// Fix the pivot at the bounding-box centre unless one was set explicitly.
    pub fn freeze_pivot(&mut self) {
        if self.pivot.is_none() {
            self.pivot = Some(self.bbox_center());
        }
    }

    /// Add rotated copies of the whole shape (or of the given faces) about `around`, in the same shape.
    /// A full turn (`|angle| >= 1`) uses `count` equal steps and closes the ring; a smaller angle spreads the
    /// `count` copies (the original included) from 0 to `angle`. Vertices that land within `weld` of an existing
    /// vertex are shared, and so are straight edges between shared vertices, so neighbouring copies join up.
    pub fn radial_array(
        &mut self,
        count: usize,
        around: P,
        angle_turns: f64,
        faces: Option<&[u32]>,
        weld: f64,
    ) -> Res<()> {
        if count < 2 {
            return Err(invalid("`radial_array` needs a count of at least 2")
                .help("the count includes the original: `count: 8` makes 7 copies"));
        }
        if !(angle_turns.abs() > 1e-12 && angle_turns.is_finite()) {
            return Err(invalid("`radial_array` needs a non-zero angle")
                .help("write `angle: 360.deg` for a full ring"));
        }
        let src_faces: Vec<u32> = match faces {
            Some(list) => {
                for &id in list {
                    self.get_ref(Kind::F, id)?;
                    self.require_live(f(id), Some(Kind::F), "radial_array")?;
                }
                list.iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            }
            None => self.live_ids(Kind::F),
        };
        let src_edges: Vec<u32> = match faces {
            Some(_) => src_faces
                .iter()
                .flat_map(|fid| self.faces[fid].loop_.iter().map(|x| x.0))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            None => self.live_ids(Kind::E),
        };
        let src_verts: Vec<u32> = match faces {
            Some(_) => src_edges
                .iter()
                .flat_map(|eid| [self.edges[eid].a, self.edges[eid].b])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            None => self.live_ids(Kind::V),
        };
        let step = if angle_turns.abs() >= 1.0 - 1e-12 {
            angle_turns / count as f64
        } else {
            angle_turns / (count - 1) as f64
        };
        // vertices that already exist, for welding (a simple scan: shapes are small)
        let mut known: Vec<(u32, P)> = self
            .live_ids(Kind::V)
            .into_iter()
            .map(|i| (i, self.pos(i)))
            .collect();
        // straight edges that already exist, by their (sorted) end vertices
        let mut straight: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for eid in self.live_ids(Kind::E) {
            let ed = &self.edges[&eid];
            if ed.curve == Curve::Straight {
                straight
                    .entry((ed.a.min(ed.b), ed.a.max(ed.b)))
                    .or_insert(eid);
            }
        }
        for k in 1..count {
            let (s, c) = (step * k as f64 * std::f64::consts::TAU).sin_cos();
            let rot = |p: P| -> P {
                let (dx, dy) = (p[0] - around[0], p[1] - around[1]);
                [around[0] + dx * c - dy * s, around[1] + dx * s + dy * c]
            };
            let mut vmap: BTreeMap<u32, u32> = BTreeMap::new();
            for &vid in &src_verts {
                let q = rot(self.pos(vid));
                let hit = known
                    .iter()
                    .find(|(_, p)| (p[0] - q[0]).hypot(p[1] - q[1]) <= weld)
                    .map(|x| x.0);
                let id = match hit {
                    Some(id) => id,
                    None => {
                        let id = self.new_vertex(q[0], q[1], Some(v(vid)));
                        known.push((id, q));
                        id
                    }
                };
                vmap.insert(vid, id);
            }
            let mut emap: BTreeMap<u32, u32> = BTreeMap::new();
            for &eid in &src_edges {
                let ed = self.edges[&eid].clone();
                let (a2, b2) = (vmap[&ed.a], vmap[&ed.b]);
                if a2 == b2 {
                    continue; // the edge collapsed onto a single vertex
                }
                let key = (a2.min(b2), a2.max(b2));
                let reuse = if ed.curve == Curve::Straight {
                    straight.get(&key).copied()
                } else {
                    None
                };
                let id = match reuse {
                    Some(id) => id,
                    None => {
                        let ne = self.new_edge(a2, b2, Some(e(eid)));
                        self.edges.get_mut(&ne).unwrap().curve = ed.curve.clone();
                        if ed.curve == Curve::Straight {
                            straight.insert(key, ne);
                        }
                        ne
                    }
                };
                emap.insert(eid, id);
            }
            for &fid in &src_faces {
                let lp = self.faces[&fid].loop_.clone();
                if lp.iter().any(|(eid, _)| !emap.contains_key(eid)) {
                    continue;
                }
                let loop_: Vec<(u32, i8)> = lp
                    .iter()
                    .map(|&(eid, sgn)| {
                        let old = &self.edges[&eid];
                        let from = vmap[&if sgn > 0 { old.a } else { old.b }];
                        (emap[&eid], self.sign_from(emap[&eid], from))
                    })
                    .collect();
                self.new_face(loop_, Some(f(fid)));
            }
        }
        self.post_edit("radial_array");
        Ok(())
    }
}
