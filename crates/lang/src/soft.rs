//! Soft bodies in the language: `shape.soft(...)`, pins, and the per-tick step. The solver (XPBD,
//! in `animscript-physics`) works on world-space vertex positions in metres; the results are written
//! back as layer-2 overrides of the shape's local vertex positions, so keyframes still win.
//! Animating a soft shape's own `position`/`rotation` moves the picture but not the simulation.
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::*;
use animscript_physics as ph;
use ph::{v2, Vec2};
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, BTreeSet};

type Args = Vec<(Option<String>, Value)>;

#[derive(Clone, Debug)]
pub struct SoftDecl {
    pub shape: String,
    pub stiffness: f64,
    pub bend: f64,
    pub pressure: f64,
    pub density: f64,
    pub friction: f64,
    pub restitution: f64,
    pub damping: f64,
    pub self_collide: bool,
    pub thickness: f64,
    pub pins: Vec<(u32, Option<String>)>,
    pub impulse: [f64; 2],
}

pub struct SoftSim {
    names: Vec<String>,
    bodies: Vec<ph::SoftBody>,
    ids: Vec<Vec<u32>>,
    index: Vec<BTreeMap<u32, usize>>,
    faces: Vec<Vec<Vec<usize>>>,
    tf: Vec<Transform>,
    pin_to: Vec<BTreeMap<usize, (String, [f64; 2])>>,
    ppm: f64,
}

fn e4(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-PHY-004", msg)
}

/// The inverse of `Transform::apply`.
pub fn unapply(t: &Transform, w: P) -> P {
    if t.is_identity() {
        return w;
    }
    let (s, c) = (t.rotation * std::f64::consts::TAU).sin_cos();
    let (dx, dy) = (
        w[0] - t.pivot[0] - t.position[0],
        w[1] - t.pivot[1] - t.position[1],
    );
    [
        t.pivot[0] + (c * dx + s * dy) / t.scale[0],
        t.pivot[1] + (-s * dx + c * dy) / t.scale[1],
    ]
}

/// Compliance (m/N) for a stiffness between 0 (very soft) and 1 (rigid).
fn compliance(stiffness: f64) -> f64 {
    if stiffness >= 1.0 {
        0.0
    } else {
        (1.0 - stiffness) / stiffness.max(1e-3) * 0.02
    }
}

fn shoelace(p: &[P]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
        .sum::<f64>()
        / 2.0
}

impl Interp {
    pub fn is_soft_method(name: &str) -> bool {
        matches!(
            name,
            "soft" | "pin" | "unpin" | "soft_impulse" | "pin_where" | "unpin_where" | "cut"
        )
    }

    fn vertex_ids(&self, sn: &str, v: &Value) -> Res<Vec<u32>> {
        let one = |x: &Value| match x {
            Value::Elem(s, r) if s == sn && r.kind == Kind::V => Ok(r.id),
            other => Err(e4(format!(
                "`pin` and `unpin` take vertices of `{sn}`, found {}",
                other.type_name()
            ))),
        };
        match v {
            Value::List(items) => items.iter().map(one).collect(),
            x => Ok(vec![one(x)?]),
        }
    }

    pub fn soft_method(&mut self, sn: &str, name: &str, args: Args, mode: Mode) -> Res<Value> {
        if self.is_mesh_shape(sn) {
            return self.soft3_method(sn, name, args, mode);
        }
        match name {
            "soft" => {
                if mode == Mode::Run {
                    return Err(Diagnostic::new(
                        "E-SHP-003",
                        "`soft` creates a soft body and is not allowed while the scene is running",
                    )
                    .help("make soft bodies at the top level or in `setup:`"));
                }
                let m = self.bind(
                    name,
                    args,
                    &[
                        "stiffness",
                        "bend",
                        "pressure",
                        "density",
                        "friction",
                        "restitution",
                        "damping",
                        "self_collide",
                        "thickness",
                    ],
                    &[],
                )?;
                let f = |k: &str, d: f64| -> Res<f64> {
                    m.get(k)
                        .map(|v| self.num(v, k).map_err(|e| e4(e.message)))
                        .unwrap_or(Ok(d))
                };
                let decl = SoftDecl {
                    shape: sn.to_string(),
                    stiffness: f("stiffness", 0.8)?,
                    bend: f("bend", 0.3)?,
                    pressure: f("pressure", 1.0)?,
                    density: f("density", 1.0)?,
                    friction: f("friction", 0.5)?,
                    restitution: f("restitution", 0.1)?,
                    damping: f("damping", 0.1)?,
                    self_collide: match m.get("self_collide") {
                        None => false,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => return Err(e4("`self_collide` must be true or false")),
                    },
                    thickness: f("thickness", 2.0)?,
                    pins: vec![],
                    impulse: [0.0, 0.0],
                };
                if !(decl.stiffness > 0.0 && decl.stiffness <= 1.0) {
                    return Err(e4("`stiffness` must be above 0 and at most 1"));
                }
                if !(0.0..=1.0).contains(&decl.bend) {
                    return Err(e4("`bend` must be between 0 and 1"));
                }
                if decl.pressure < 0.0
                    || decl.density <= 0.0
                    || decl.friction < 0.0
                    || decl.restitution < 0.0
                    || decl.damping < 0.0
                    || decl.thickness <= 0.0
                {
                    return Err(e4("`pressure`, `friction`, `restitution` and `damping` cannot be negative; `density` and `thickness` must be positive"));
                }
                if self.decls.body(sn).is_some() {
                    return Err(Diagnostic::new(
                        "E-PHY-003",
                        format!("`{sn}` already has a rigid body; a shape is either rigid or soft"),
                    ));
                }
                if self.decls.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(Diagnostic::new(
                        "E-PHY-003",
                        format!("`{sn}` is already soft"),
                    ));
                }
                let sh = &self.shapes[sn];
                if sh.live_ids(Kind::E).is_empty() {
                    return Err(Diagnostic::new(
                        "E-PHY-003",
                        format!("`{sn}` has no edges, so it cannot be soft"),
                    ));
                }
                if sh.scale[0] == 0.0 || sh.scale[1] == 0.0 {
                    return Err(e4(format!("`{sn}` has a zero scale")));
                }
                let scene = mode == Mode::Setup;
                self.decls.soft.push((scene, decl));
                Ok(Value::None)
            }
            "pin" | "unpin" => {
                let params: &[&str] = if name == "pin" {
                    &["vertex", "to"]
                } else {
                    &["vertex"]
                };
                let m = self.bind(name, args, params, &["vertex"])?;
                let ids = self.vertex_ids(sn, &m["vertex"])?;
                let to = match m.get("to") {
                    None => None,
                    Some(Value::Shape(t)) if t != sn => Some(t.clone()),
                    Some(Value::Shape(_)) => {
                        return Err(e4("a soft shape cannot be pinned to itself"))
                    }
                    Some(other) => {
                        return Err(e4(format!(
                            "`to` must be a shape, found {}",
                            other.type_name()
                        )))
                    }
                };
                if !self.decls.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(e4(format!(
                        "`{sn}` is not soft, so it has nothing to pin; call `{sn}.soft()` first"
                    )));
                }
                if mode == Mode::Run {
                    self.pin_live(sn, name == "pin", &ids, to)?;
                } else if let Some(d) = self.decls.soft.iter_mut().find(|d| d.1.shape == sn) {
                    for id in ids {
                        d.1.pins.retain(|p| p.0 != id);
                        if name == "pin" {
                            d.1.pins.push((id, to.clone()));
                        }
                    }
                }
                Ok(Value::None)
            }
            "soft_impulse" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let by = self.point(&m["by"], "a vector")?;
                if !self.decls.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(e4(format!("`{sn}` is not soft; call `{sn}.soft()` first")));
                }
                if mode == Mode::Run {
                    if let Some(sim) = self.soft.as_mut() {
                        if let Some(i) = sim.names.iter().position(|n| n == sn) {
                            let b = &mut sim.bodies[i];
                            let dv = v2(by[0] / sim.ppm, by[1] / sim.ppm) / b.total_mass();
                            for v in 0..b.pos.len() {
                                if b.pins[v].is_none() {
                                    b.vel[v] += dv;
                                }
                            }
                        }
                    }
                } else if let Some(d) = self.decls.soft.iter_mut().find(|d| d.1.shape == sn) {
                    d.1.impulse[0] += by[0];
                    d.1.impulse[1] += by[1];
                }
                Ok(Value::None)
            }
            _ => unreachable!(),
        }
    }

    fn pin_live(&mut self, sn: &str, pin: bool, ids: &[u32], to: Option<String>) -> Res<()> {
        let tf_to = to.as_ref().map(|t| self.shapes[t].transform());
        let Some(sim) = self.soft.as_mut() else {
            return Ok(());
        };
        let Some(i) = sim.names.iter().position(|n| n == sn) else {
            return Ok(());
        };
        for id in ids {
            let Some(&v) = sim.index[i].get(id) else {
                continue;
            };
            if pin {
                let at = sim.bodies[i].pos[v];
                sim.bodies[i].pin(v, at);
                match (&to, &tf_to) {
                    (Some(t), Some(tf)) => {
                        sim.pin_to[i].insert(
                            v,
                            (t.clone(), unapply(tf, [at.x * sim.ppm, at.y * sim.ppm])),
                        );
                    }
                    _ => {
                        sim.pin_to[i].remove(&v);
                    }
                }
            } else {
                sim.bodies[i].unpin(v);
                sim.pin_to[i].remove(&v);
            }
        }
        Ok(())
    }

    /// Reads: `volume` (px^2, from the faces) and `stretch` (largest edge strain, 0 = none).
    pub fn soft_attr(&self, sn: &str, name: &str) -> Option<Res<Value>> {
        if !matches!(name, "volume" | "stretch") {
            return None;
        }
        if !self.decls.soft.iter().any(|d| d.1.shape == sn) {
            return Some(Err(e4(format!(
                "`{sn}` is not soft, so it has no `{name}`"
            ))
            .help(format!("call `{sn}.soft()` first"))));
        }
        let live = self
            .soft
            .as_ref()
            .and_then(|s| s.names.iter().position(|n| n == sn).map(|i| (s, i)));
        Some(Ok(Value::Num(match (name, live) {
            ("stretch", Some((s, i))) => s.bodies[i].max_strain(),
            ("stretch", None) => 0.0,
            ("volume", Some((s, i))) => s.volume(i),
            _ => {
                let sh = &self.shapes[sn];
                sh.live_ids(Kind::F)
                    .into_iter()
                    .map(|f| sh.face_area(f).abs())
                    .sum()
            }
        })))
    }

    /// Build the soft bodies for the scene that is starting, from the shapes' current vertices.
    pub fn build_soft(&mut self) -> Res<()> {
        self.soft = None;
        if self.decls.soft.is_empty() {
            return Ok(());
        }
        let ppm = self.world_cfg.scale;
        let decls: Vec<SoftDecl> = self.decls.soft.iter().map(|d| d.1.clone()).collect();
        let mut sim = SoftSim {
            names: vec![],
            bodies: vec![],
            ids: vec![],
            index: vec![],
            faces: vec![],
            tf: vec![],
            pin_to: vec![],
            ppm,
        };
        for d in &decls {
            let sh = &self.shapes[&d.shape];
            let t = sh.transform();
            let ids = sh.live_ids(Kind::V);
            let index: BTreeMap<u32, usize> =
                ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            let world: Vec<P> = ids.iter().map(|id| t.apply(sh.pos(*id))).collect();
            let pos: Vec<Vec2> = world.iter().map(|p| v2(p[0] / ppm, p[1] / ppm)).collect();
            let mut mass = vec![0.0; ids.len()];
            let mut in_face: BTreeSet<u32> = BTreeSet::new();
            let mut rings: Vec<Vec<usize>> = vec![];
            for f in sh.live_ids(Kind::F) {
                let ring_ids = sh.face_vertex_ids(f);
                if ring_ids.len() < 3 {
                    continue;
                }
                for (e, _) in &sh.faces[&f].loop_ {
                    in_face.insert(*e);
                }
                let ring: Vec<usize> = ring_ids.iter().map(|v| index[v]).collect();
                let area = shoelace(&ring.iter().map(|i| world[*i]).collect::<Vec<_>>()).abs()
                    / (ppm * ppm);
                for i in &ring {
                    mass[*i] += d.density * area / ring.len() as f64;
                }
                rings.push(ring);
            }
            let edges = sh.live_ids(Kind::E);
            for e in &edges {
                if in_face.contains(e) {
                    continue;
                }
                let ed = &sh.edges[e];
                let len = (pos[index[&ed.a]] - pos[index[&ed.b]]).length();
                for v in [ed.a, ed.b] {
                    mass[index[&v]] += d.density * 0.05 * len / 2.0;
                }
            }
            for m in &mut mass {
                if *m <= 0.0 {
                    *m = 1e-3;
                }
            }
            let params = ph::SoftParams {
                radius: d.thickness / 2.0 / ppm,
                friction: d.friction,
                restitution: d.restitution,
                damping: d.damping,
                self_collide: d.self_collide,
            };
            let mut body = ph::SoftBody::new(pos.clone(), mass, params);
            let c = compliance(d.stiffness);
            let mut adj: Vec<Vec<usize>> = vec![vec![]; ids.len()];
            for e in &edges {
                let ed = &sh.edges[e];
                let (a, b) = (index[&ed.a], index[&ed.b]);
                body.add_distance(a, b, c);
                adj[a].push(b);
                adj[b].push(a);
            }
            for ring in &rings {
                if ring.len() == 4 {
                    body.add_internal(ring[0], ring[2], c);
                    body.add_internal(ring[1], ring[3], c);
                }
                if d.pressure > 0.0 {
                    body.add_area(ring, d.pressure, c * 0.25);
                }
            }
            // bend: link the far ends of two edges that run straight through a vertex
            let cb = compliance(d.bend);
            let mut linked: BTreeSet<(usize, usize)> = BTreeSet::new();
            for v in 0..ids.len() {
                for (i, &a) in adj[v].iter().enumerate() {
                    for &b in &adj[v][i + 1..] {
                        let (d1, d2) = (
                            (pos[a] - pos[v]).normalized(),
                            (pos[b] - pos[v]).normalized(),
                        );
                        if d1.dot(d2) < -0.9 && linked.insert((a.min(b), a.max(b))) {
                            body.add_internal(a, b, cb);
                        }
                    }
                }
            }
            let mut pin_to = BTreeMap::new();
            for (vid, to) in &d.pins {
                let Some(&i) = index.get(vid) else { continue };
                body.pin(i, pos[i]);
                if let Some(t) = to {
                    let tt = self.shapes[t].transform();
                    pin_to.insert(i, (t.clone(), unapply(&tt, world[i])));
                }
            }
            if d.impulse != [0.0, 0.0] {
                let dv = v2(d.impulse[0] / ppm, d.impulse[1] / ppm) / body.total_mass();
                for v in 0..body.pos.len() {
                    if body.pins[v].is_none() {
                        body.vel[v] += dv;
                    }
                }
            }
            sim.names.push(d.shape.clone());
            sim.bodies.push(body);
            sim.ids.push(ids);
            sim.index.push(index);
            sim.faces.push(rings);
            sim.tf.push(t);
            sim.pin_to.push(pin_to);
        }
        self.soft = Some(sim);
        Ok(())
    }

    /// Shapes whose pose the pins of soft bodies follow.
    pub fn soft_pin_targets(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        if let Some(s) = &self.soft {
            for m in &s.pin_to {
                out.extend(m.values().map(|(n, _)| n.clone()));
            }
        }
        out.into_iter().collect()
    }

    /// Step the soft bodies (and, through them, the rigid world) to this tick. Returns the new local
    /// vertex positions `(shape, vertex id, x, y)` to write as layer-2 overrides.
    /// Names of the soft shapes (whose own transform carries the simulation).
    pub fn soft_shapes(&self) -> Vec<String> {
        self.soft
            .as_ref()
            .map(|s| s.names.clone())
            .unwrap_or_default()
    }

    /// `own` holds each soft shape's own resolved transform this tick: when it changes, the whole
    /// simulation (points and pin targets) is carried along with it.
    pub fn soft_prepare(
        &mut self,
        tick: i64,
        targets: &BTreeMap<String, Transform>,
        own: &BTreeMap<String, Transform>,
    ) {
        let Some(sim) = self.soft.as_mut() else {
            return;
        };
        if tick == 0 {
            return;
        }
        let ppm = sim.ppm;
        for i in 0..sim.bodies.len() {
            if let Some(new) = own.get(&sim.names[i]) {
                if *new != sim.tf[i] {
                    let old = sim.tf[i];
                    let carry = |p: Vec2| {
                        let w = new.apply(unapply(&old, [p.x * ppm, p.y * ppm]));
                        v2(w[0] / ppm, w[1] / ppm)
                    };
                    sim.bodies[i].map_points(&carry);
                    sim.tf[i] = *new;
                }
            }
        }
        for i in 0..sim.bodies.len() {
            for (v, (t, off)) in &sim.pin_to[i] {
                if let Some(tf) = targets.get(t) {
                    let w = tf.apply(*off);
                    sim.bodies[i].set_pin_target(*v, v2(w[0] / ppm, w[1] / ppm));
                }
            }
        }
    }

    /// The new local vertex positions `(shape, vertex id, x, y)` to write as layer-2 overrides.
    pub fn soft_collect(&self, tick: i64) -> Vec<(String, u32, f64, f64)> {
        let Some(sim) = self.soft.as_ref() else {
            return vec![];
        };
        if tick == 0 {
            return vec![];
        }
        let ppm = sim.ppm;
        let mut out = vec![];
        for i in 0..sim.bodies.len() {
            for (v, id) in sim.ids[i].iter().enumerate() {
                let p = sim.bodies[i].pos[v];
                let l = unapply(&sim.tf[i], [p.x * ppm, p.y * ppm]);
                out.push((sim.names[i].clone(), *id, l[0], l[1]));
            }
        }
        out
    }

    pub fn soft_snapshot(&self) -> Option<J> {
        let sim = self.soft.as_ref()?;
        let bodies: Vec<J> = (0..sim.bodies.len())
            .map(|i| {
                let pins: Vec<J> = sim.bodies[i].pins.iter().enumerate().filter_map(|(v, p)| p.map(|_| json!([sim.bodies[i].pos[v].x * sim.ppm, sim.bodies[i].pos[v].y * sim.ppm]))).collect();
                json!({"shape": sim.names[i], "volume": sim.volume(i), "stretch": sim.bodies[i].max_strain(), "pins": pins})
            })
            .collect();
        Some(json!({"bodies": bodies}))
    }

    /// Write the final vertex positions back as rest values and drop the simulation.
    pub fn end_soft(&mut self) {
        let Some(sim) = self.soft.take() else { return };
        for i in 0..sim.bodies.len() {
            let Some(sh) = self.shapes.get_mut(&sim.names[i]) else {
                continue;
            };
            for (v, id) in sim.ids[i].iter().enumerate() {
                let p = sim.bodies[i].pos[v];
                let l = unapply(&sim.tf[i], [p.x * sim.ppm, p.y * sim.ppm]);
                if let Some(vert) = sh.verts.get_mut(id) {
                    vert.x = l[0];
                    vert.y = l[1];
                }
            }
        }
    }
}

impl SoftSim {
    pub(crate) fn bodies_mut(&mut self) -> &mut [ph::SoftBody] {
        &mut self.bodies
    }
    fn volume(&self, i: usize) -> f64 {
        self.faces[i]
            .iter()
            .map(|r| {
                shoelace(
                    &r.iter()
                        .map(|v| {
                            [
                                self.bodies[i].pos[*v].x * self.ppm,
                                self.bodies[i].pos[*v].y * self.ppm,
                            ]
                        })
                        .collect::<Vec<_>>(),
                )
                .abs()
            })
            .sum()
    }
}
