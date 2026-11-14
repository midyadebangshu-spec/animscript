//! Soft bodies on meshes: `.soft()`, `pin_where`, `cut`, the simulation built at the start of a
//! scene and its per-tick output (every mesh vertex's position). The engine is the soft part of
//! `animscript-physics3`; it steps together with the 3D rigid world.
//!
//! A soft mesh's own transform is baked into its vertices when the scene starts (its position,
//! turns and scale are then identity), and the final vertices are written back when it ends.
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::mesh::{Mesh, Transform3};
use animscript_core::*;
use animscript_physics3 as p3;
use p3::{v3, Quat, SoftBody3, SoftParams, SurfaceSpec, V3};
use serde_json::{json, Value as J};
use std::collections::BTreeMap;

type Args = Vec<(Option<String>, Value)>;

fn err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-PHY-009", msg)
}

#[derive(Clone, Debug)]
pub struct Soft3Decl {
    pub shape: String,
    pub stiffness: f64,
    pub bend: f64,
    pub pressure: f64,
    pub density: f64,
    pub friction: f64,
    pub restitution: f64,
    pub damping: f64,
    pub thickness: f64,
    pub self_collide: bool,
    pub solid: bool,
    pub resolution: usize,
}

#[derive(Clone, Debug)]
pub struct Pin3Decl {
    pub shape: String,
    pub ids: Vec<usize>,
    pub to: Option<String>,
}

/// The live soft bodies of the running scene.
pub struct Soft3Sim {
    pub names: Vec<String>,
    pub bodies: Vec<SoftBody3>,
    /// Per body: the point each mesh vertex is (duplicates at one place share a point).
    vmap: Vec<Vec<usize>>,
    /// Per body: the faces of the mesh as first built, and which are still there.
    faces: Vec<Vec<Vec<u32>>>,
    alive: Vec<Vec<bool>>,
    /// Per body: node -> (the shape it follows, the pin's place in that shape's mesh coordinates).
    pin_to: Vec<BTreeMap<usize, (String, [f64; 3])>>,
    ppm: f64,
}

fn compliance(stiffness: f64) -> f64 {
    if stiffness >= 1.0 {
        0.0
    } else {
        (1.0 - stiffness) / stiffness.max(1e-3) * 1e-4
    }
}

fn bend_compliance(bend: f64) -> f64 {
    (1.0 - bend.min(1.0)) / bend.max(1e-3) * 5e-3
}

/// The inverse of `Transform3::apply` (for a transform without a quaternion).
fn unapply(t: &Transform3, p: [f64; 3]) -> [f64; 3] {
    let tau = std::f64::consts::TAU;
    let mut v = [
        p[0] - t.pivot[0] - t.position[0],
        p[1] - t.pivot[1] - t.position[1],
        p[2] - t.pivot[2] - t.position[2],
    ];
    if let Some([w, x, y, z]) = t.orient {
        let q = Quat { w, x, y, z }.conj();
        let r = q.rotate(v3(v[0], v[1], v[2]));
        v = [r.x, r.y, r.z];
    } else {
        let (s, c) = (-t.rot[2] * tau).sin_cos();
        v = [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
        let (s, c) = (-t.rot[0] * tau).sin_cos();
        v = [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
        let (s, c) = (-t.rot[1] * tau).sin_cos();
        v = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
    }
    [
        v[0] / t.scale[0] + t.pivot[0],
        v[1] / t.scale[1] + t.pivot[1],
        v[2] / t.scale[2] + t.pivot[2],
    ]
}

struct Welded {
    verts: Vec<V3>,
    tris: Vec<[usize; 3]>,
    groups: Vec<usize>,
    vmap: Vec<usize>,
}

/// World-space vertices in metres with duplicates at one place merged, and the faces as triangles.
fn weld(mesh: &Mesh, t: &Transform3, ppm: f64) -> Welded {
    let mut seen: BTreeMap<(i64, i64, i64), usize> = BTreeMap::new();
    let mut verts = vec![];
    let mut vmap = vec![];
    for v in &mesh.verts {
        let w = t.apply(*v);
        let key = (
            (w[0] * 1000.0).round() as i64,
            (w[1] * 1000.0).round() as i64,
            (w[2] * 1000.0).round() as i64,
        );
        let id = *seen.entry(key).or_insert_with(|| {
            verts.push(v3(w[0] / ppm, w[1] / ppm, w[2] / ppm));
            verts.len() - 1
        });
        vmap.push(id);
    }
    let mut tris = vec![];
    let mut groups = vec![];
    for (fi, f) in mesh.faces.iter().enumerate() {
        for k in 1..f.len().saturating_sub(1) {
            let t = [
                vmap[f[0] as usize],
                vmap[f[k] as usize],
                vmap[f[k + 1] as usize],
            ];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                tris.push(t);
                groups.push(fi);
            }
        }
    }
    Welded {
        verts,
        tris,
        groups,
        vmap,
    }
}

impl Interp {
    fn triple3(&self, v: &Value, what: &str) -> Res<[f64; 3]> {
        match v {
            Value::Tuple(t) if t.len() == 3 => Ok([
                self.num(&t[0], what)?,
                self.num(&t[1], what)?,
                self.num(&t[2], what)?,
            ]),
            other => Err(err(format!(
                "{what} must be a 3D point like `(10, 20, 30)`, found {}",
                other.type_name()
            ))),
        }
    }

    /// The vertices of a mesh shape (world pixels now) that satisfy a test.
    pub(crate) fn select_vertices(
        &mut self,
        sn: &str,
        test: &Value,
        mode: Mode,
    ) -> Res<Vec<usize>> {
        let Value::Lambda(l) = test else {
            return Err(err("the test must be a function like `p -> p.y < 120`"));
        };
        let l = l.clone();
        let positions: Vec<[f64; 3]> = match self
            .soft3
            .as_ref()
            .and_then(|s| s.names.iter().position(|n| n == sn).map(|i| (s, i)))
        {
            Some((s, i)) => s.vmap[i]
                .iter()
                .map(|pi| {
                    let p = s.bodies[i].point_pos(*pi);
                    [p.x * s.ppm, p.y * s.ppm, p.z * s.ppm]
                })
                .collect(),
            None => {
                let t = self.mesh_transform(sn);
                self.shapes[sn]
                    .mesh
                    .as_ref()
                    .expect("mesh")
                    .verts
                    .iter()
                    .map(|v| t.apply(*v))
                    .collect()
            }
        };
        let mut out = vec![];
        for (i, p) in positions.iter().enumerate() {
            let arg = Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1]), Value::Num(p[2])]);
            match self.call_lambda(&l, vec![arg], mode)? {
                Value::Bool(true) => out.push(i),
                Value::Bool(false) => {}
                other => {
                    return Err(err(format!(
                        "the test must give true or false, found {}",
                        other.type_name()
                    )))
                }
            }
        }
        Ok(out)
    }

    pub fn soft3_method(&mut self, sn: &str, name: &str, args: Args, mode: Mode) -> Res<Value> {
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
                        "solid",
                        "resolution",
                    ],
                    &[],
                )?;
                let f = |k: &str, d: f64| -> Res<f64> {
                    m.get(k)
                        .map(|v| self.num(v, k).map_err(|e| err(e.message)))
                        .unwrap_or(Ok(d))
                };
                let b = |k: &str| -> Res<bool> {
                    match m.get(k) {
                        None => Ok(false),
                        Some(Value::Bool(x)) => Ok(*x),
                        Some(o) => Err(Diagnostic::new(
                            "E-TYP-001",
                            format!("`{k}` must be true or false, found {}", o.type_name()),
                        )),
                    }
                };
                let solid = b("solid")?;
                let decl = Soft3Decl {
                    shape: sn.to_string(),
                    stiffness: f("stiffness", 0.8)?,
                    bend: f("bend", 0.3)?,
                    pressure: f("pressure", 1.0)?,
                    density: f("density", 1.0)?,
                    friction: f("friction", 0.5)?,
                    restitution: f("restitution", 0.1)?,
                    damping: f("damping", 0.1)?,
                    thickness: f("thickness", 3.0)?,
                    self_collide: b("self_collide")?,
                    solid,
                    resolution: match m.get("resolution") {
                        None => 8,
                        Some(v) => self.int(v, "`resolution`")? as usize,
                    },
                };
                if !(decl.stiffness > 0.0 && decl.stiffness <= 1.0) {
                    return Err(err("`stiffness` must be above 0 and at most 1"));
                }
                if !(0.0..=1.0).contains(&decl.bend) {
                    return Err(err("`bend` must be between 0 and 1"));
                }
                if decl.pressure < 0.0
                    || decl.density <= 0.0
                    || decl.damping < 0.0
                    || decl.thickness <= 0.0
                    || decl.friction < 0.0
                    || decl.restitution < 0.0
                {
                    return Err(err("`pressure`, `damping`, `friction` and `restitution` cannot be negative, `density` and `thickness` must be positive"));
                }
                if !(3..=24).contains(&decl.resolution) {
                    return Err(err("`resolution` (lattice cells along the longest side) must be between 3 and 24"));
                }
                if self.decls3.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(err(format!("`{sn}` is already soft")));
                }
                if self.decls3.body(sn).is_some() {
                    return Err(Diagnostic::new(
                        "E-PHY-003",
                        format!("`{sn}` is rigid; a shape is either rigid or soft"),
                    ));
                }
                let sh = &self.shapes[sn];
                let w = weld(
                    sh.mesh.as_ref().expect("mesh"),
                    &self.mesh_transform(sn),
                    self.world_cfg.scale,
                );
                let closed = p3::is_closed(&w.tris);
                if solid && !closed {
                    return Err(err(format!("`{sn}` is not a closed mesh, so it cannot be solid"))
                        .help("a solid body needs a closed surface (a cube, sphere, cylinder, cone, lathe or extrusion), a sheet can be soft without `solid`"));
                }
                if m.contains_key("pressure") && decl.pressure != 1.0 && !closed {
                    return Err(err(format!(
                        "`{sn}` is an open mesh, which has no volume to pressurise"
                    ))
                    .help("`pressure` is for closed meshes; leave it out for sheets"));
                }
                if w.tris.is_empty() {
                    return Err(err(format!("`{sn}` has no faces, so it cannot be soft")));
                }
                if self.decls.bodies.iter().any(|d| d.1.shape == sn) {
                    return Err(err("a 2D body cannot be soft here"));
                }
                if self.decls.skins3.iter().any(|d| d.1.shape == sn) {
                    return Err(Diagnostic::new("E-RIG-007", format!("`{sn}` is driven by a rig; a mesh is rigid, soft or rig-driven, not two of them")));
                }
                self.decls3.soft.push((mode == Mode::Setup, decl));
                Ok(Value::None)
            }
            "pin_where" | "unpin_where" => {
                let m = self.bind(name, args, &["test", "to"], &["test"])?;
                if !self.decls3.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(err(format!("`{sn}` is not soft; call `{sn}.soft()` first")));
                }
                let to = match m.get("to") {
                    None => None,
                    Some(Value::Shape(t)) if self.is_mesh_shape(t) && t != sn => Some(t.clone()),
                    Some(_) => {
                        return Err(err("`to` must be another mesh shape (a rigid body, a kinematic or an animated mesh)"))
                    }
                };
                let ids = self.select_vertices(sn, &m["test"], mode)?;
                if mode == Mode::Run {
                    self.pin_live3(sn, name == "pin_where", &ids, to);
                } else if name == "pin_where" {
                    self.decls3.pins.push((
                        mode == Mode::Setup,
                        Pin3Decl {
                            shape: sn.to_string(),
                            ids,
                            to,
                        },
                    ));
                } else {
                    let set: Vec<usize> = ids;
                    for p in self.decls3.pins.iter_mut() {
                        if p.1.shape == sn {
                            p.1.ids.retain(|i| !set.contains(i));
                        }
                    }
                }
                Ok(Value::None)
            }
            "soft_impulse" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let by = self.triple3(&m["by"], "`by`")?;
                if !self.decls3.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(err(format!("`{sn}` is not soft; call `{sn}.soft()` first")));
                }
                if mode == Mode::Run {
                    if let Some(sim) = self.soft3.as_mut() {
                        if let Some(i) = sim.names.iter().position(|n| n == sn) {
                            let p = sim.ppm;
                            sim.bodies[i].apply_impulse(v3(by[0] / p, by[1] / p, by[2] / p));
                        }
                    }
                } else {
                    self.decls3
                        .soft_ops
                        .push((mode == Mode::Setup, (sn.to_string(), by)));
                }
                Ok(Value::None)
            }
            "cut" => {
                let m = self.bind(name, args, &["from", "to", "width"], &["from", "to"])?;
                let a = self.triple3(&m["from"], "`from`")?;
                let b = self.triple3(&m["to"], "`to`")?;
                let width = m
                    .get("width")
                    .map(|v| self.num(v, "`width`"))
                    .unwrap_or(Ok(8.0))?;
                if width < 0.0 {
                    return Err(err("`width` cannot be negative"));
                }
                if !self.decls3.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(err(format!("`{sn}` is not soft; call `{sn}.soft()` first")));
                }
                if self
                    .decls3
                    .soft
                    .iter()
                    .any(|d| d.1.shape == sn && d.1.solid)
                {
                    return Err(err(format!(
                        "`{sn}` is solid; `cut` works on sheets and balloons (surface bodies)"
                    )));
                }
                if mode != Mode::Run {
                    return Err(err("`cut` slices a soft body while a scene runs"));
                }
                let Some(sim) = self.soft3.as_mut() else {
                    return Ok(Value::Int(0));
                };
                let Some(i) = sim.names.iter().position(|n| n == sn) else {
                    return Ok(Value::Int(0));
                };
                let p = sim.ppm;
                let removed = sim.bodies[i].cut(
                    v3(a[0] / p, a[1] / p, a[2] / p),
                    v3(b[0] / p, b[1] / p, b[2] / p),
                    width / p,
                );
                for g in &removed {
                    sim.alive[i][*g] = false;
                }
                let faces: Vec<Vec<u32>> = sim.faces[i]
                    .iter()
                    .zip(&sim.alive[i])
                    .filter(|(_, a)| **a)
                    .map(|(f, _)| f.clone())
                    .collect();
                if let Some(mesh) = self.shapes.get_mut(sn).and_then(|s| s.mesh.as_mut()) {
                    mesh.faces = faces;
                }
                Ok(Value::Int(removed.len() as i64))
            }
            _ => Err(err(format!("`{name}` is not available on a soft mesh"))),
        }
    }

    pub fn is_soft3(&self, sn: &str) -> bool {
        self.decls3.soft.iter().any(|d| d.1.shape == sn)
    }

    /// `volume` (px^3) and `stretch` (largest edge strain) of a soft mesh.
    pub fn soft3_attr(&self, sn: &str, name: &str) -> Option<Res<Value>> {
        if !matches!(name, "volume" | "stretch") || !self.is_soft3(sn) {
            return None;
        }
        let live = self
            .soft3
            .as_ref()
            .and_then(|s| s.names.iter().position(|n| n == sn).map(|i| (s, i)));
        Some(Ok(Value::Num(match (name, live) {
            ("stretch", Some((s, i))) => s.bodies[i].max_strain(),
            ("stretch", None) => 0.0,
            ("volume", Some((s, i))) => s.bodies[i].surface_volume().abs() * s.ppm.powi(3),
            _ => {
                let sh = &self.shapes[sn];
                let m = sh.mesh.as_ref().expect("mesh");
                let t = self.mesh_transform(sn);
                let w: Vec<[f64; 3]> = m.verts.iter().map(|v| t.apply(*v)).collect();
                let faces: Vec<Vec<u32>> = m.faces.clone();
                crate::phys3::mesh_volume(&w, &faces)
            }
        })))
    }

    fn pin_live3(&mut self, sn: &str, pin: bool, ids: &[usize], to: Option<String>) {
        let tf_to = to.as_ref().map(|t| self.target_transform3(t, 0.0));
        let Some(sim) = self.soft3.as_mut() else {
            return;
        };
        let Some(i) = sim.names.iter().position(|n| n == sn) else {
            return;
        };
        for id in ids {
            let pi = sim.vmap[i][*id];
            let p = sim.bodies[i].points[pi];
            for k in 0..p.n {
                let node = p.nodes[k];
                if pin {
                    let at = sim.bodies[i].pos[node];
                    sim.bodies[i].pin(node, at);
                    match (&to, &tf_to) {
                        (Some(t), Some(tf)) => {
                            sim.pin_to[i].insert(
                                node,
                                (
                                    t.clone(),
                                    unapply(tf, [at.x * sim.ppm, at.y * sim.ppm, at.z * sim.ppm]),
                                ),
                            );
                        }
                        _ => {
                            sim.pin_to[i].remove(&node);
                        }
                    }
                } else {
                    sim.bodies[i].unpin(node);
                    sim.pin_to[i].remove(&node);
                }
            }
        }
    }

    /// Where a mesh shape is now (a dynamic body from the world, anything else from its own
    /// values), `ahead` seconds from now at its current velocity.
    pub(crate) fn target_transform3(&self, name: &str, ahead: f64) -> Transform3 {
        if let Some(ph) = self.phys3.as_ref() {
            if let (Some(&id), Some(com)) = (ph.ids.get(name), ph.com.get(name)) {
                let b = ph.world.body(id);
                if b.kind == p3::BodyKind::Dynamic {
                    let p = b.p + b.v * ahead;
                    let q = b.q.integrate(b.w, ahead);
                    return Transform3 {
                        position: [
                            p.x * ph.ppm - com[0],
                            p.y * ph.ppm - com[1],
                            p.z * ph.ppm - com[2],
                        ],
                        rot: [0.0; 3],
                        scale: [1.0; 3],
                        pivot: *com,
                        orient: Some(q.arr()),
                    };
                }
            }
        }
        self.mesh_transform(name)
    }

    // ---- scene start and end ------------------------------------------------------------

    pub fn build_soft3(&mut self) -> Res<()> {
        self.soft3 = None;
        if self.decls3.soft.is_empty() {
            return Ok(());
        }
        let ppm = self.world_cfg.scale;
        let decls: Vec<Soft3Decl> = self.decls3.soft.iter().map(|d| d.1.clone()).collect();
        let mut sim = Soft3Sim {
            names: vec![],
            bodies: vec![],
            vmap: vec![],
            faces: vec![],
            alive: vec![],
            pin_to: vec![],
            ppm,
        };
        for d in &decls {
            let t = self.mesh_transform(&d.shape);
            let (mesh, faces) = {
                let m = self.shapes[&d.shape].mesh.clone().expect("mesh");
                let f = m.faces.clone();
                (m, f)
            };
            let w = weld(&mesh, &t, ppm);
            let params = SoftParams {
                radius: d.thickness / ppm,
                friction: d.friction,
                restitution: d.restitution,
                damping: d.damping,
                self_collide: d.self_collide,
            };
            let closed = p3::is_closed(&w.tris);
            let mut body = if d.solid {
                let b = SoftBody3::from_solid(
                    &w.verts,
                    &w.tris,
                    &w.groups,
                    d.resolution,
                    d.density,
                    compliance(d.stiffness),
                    d.pressure.max(0.01),
                    params,
                )
                .ok_or_else(|| err(format!("`{}` could not be filled with tetrahedra", d.shape)))?;
                if b.node_count() > 9000 {
                    return Err(err(format!(
                        "`{}` needs {} lattice nodes; lower `resolution`",
                        d.shape,
                        b.node_count()
                    )));
                }
                b
            } else {
                SoftBody3::from_surface(
                    &w.verts,
                    &w.tris,
                    &w.groups,
                    &SurfaceSpec {
                        density: d.density,
                        edge_compliance: compliance(d.stiffness),
                        bend_compliance: bend_compliance(d.bend),
                        volume: (closed && d.pressure > 0.0).then_some((d.pressure, 1e-6)),
                    },
                    params,
                )
            };
            if d.self_collide && !d.solid {
                let mean = {
                    let mut s = 0.0;
                    for t in &w.tris {
                        s += (w.verts[t[0]] - w.verts[t[1]]).len();
                    }
                    s / w.tris.len().max(1) as f64
                };
                body.set_exclusion(mean * 2.5);
            }
            // the shape now lives in world pixels
            let shape = self.shapes.get_mut(&d.shape).expect("declared shape");
            let sm = shape.mesh.as_mut().expect("mesh");
            for v in sm.verts.iter_mut() {
                *v = t.apply(*v);
            }
            shape.position = [0.0, 0.0];
            shape.pos_z = 0.0;
            shape.rot_x = 0.0;
            shape.rot_y = 0.0;
            shape.rotation = 0.0;
            shape.scale = [1.0, 1.0];
            shape.scale_z = 1.0;
            shape.pivot3 = [0.0; 3];
            shape.pivot = None;
            shape.orient = None;
            sim.names.push(d.shape.clone());
            sim.alive.push(vec![true; faces.len()]);
            sim.faces.push(faces);
            sim.vmap.push(w.vmap);
            sim.pin_to.push(BTreeMap::new());
            sim.bodies.push(body);
        }
        // pins
        let pins: Vec<Pin3Decl> = self.decls3.pins.iter().map(|p| p.1.clone()).collect();
        for pd in &pins {
            let Some(i) = sim.names.iter().position(|n| *n == pd.shape) else {
                continue;
            };
            let tf_to = pd.to.as_ref().map(|t| self.target_transform3(t, 0.0));
            for id in &pd.ids {
                let pi = sim.vmap[i][*id];
                let p = sim.bodies[i].points[pi];
                for k in 0..p.n {
                    let node = p.nodes[k];
                    let at = sim.bodies[i].pos[node];
                    sim.bodies[i].pin(node, at);
                    if let (Some(t), Some(tf)) = (&pd.to, &tf_to) {
                        sim.pin_to[i].insert(
                            node,
                            (t.clone(), unapply(tf, [at.x * ppm, at.y * ppm, at.z * ppm])),
                        );
                    }
                }
            }
        }
        let ops: Vec<(String, [f64; 3])> =
            self.decls3.soft_ops.iter().map(|o| o.1.clone()).collect();
        for (n, by) in ops {
            if let Some(i) = sim.names.iter().position(|x| *x == n) {
                sim.bodies[i].apply_impulse(v3(by[0] / ppm, by[1] / ppm, by[2] / ppm));
            }
        }
        // the world the bodies step with exists even when no rigid body was declared
        if self.phys3.is_none() {
            let g = self.world_cfg.gravity3;
            self.phys3 = Some(crate::phys3::Physics3::empty(
                p3::World::new(v3(g[0], g[1], g[2])),
                ppm,
                self.world_cfg.substeps,
            ));
        }
        self.soft3 = Some(sim);
        Ok(())
    }

    /// Names of the shapes soft pins follow.
    pub fn soft3_pin_targets(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        if let Some(s) = &self.soft3 {
            for m in &s.pin_to {
                for (t, _) in m.values() {
                    if !out.contains(t) {
                        out.push(t.clone());
                    }
                }
            }
        }
        out
    }

    /// Move the pins that follow other shapes to where those shapes are going (`poses`: the
    /// resolved pose of every target that physics does not drive).
    pub fn soft3_prepare(&mut self, tick: i64, poses: &BTreeMap<String, Transform3>) {
        if tick == 0 {
            return;
        }
        let ahead = 1.0 / self.fps.to_f64();
        let Some(sim_ref) = self.soft3.as_ref() else {
            return;
        };
        let mut moves: Vec<(usize, usize, V3)> = vec![];
        let mut cache: BTreeMap<String, Transform3> = BTreeMap::new();
        for (i, m) in sim_ref.pin_to.iter().enumerate() {
            for (node, (target, local)) in m {
                let tf = *cache.entry(target.clone()).or_insert_with(|| {
                    let dynamic = self
                        .phys3
                        .as_ref()
                        .and_then(|p| p.ids.get(target).map(|id| p.world.body(*id).kind))
                        == Some(p3::BodyKind::Dynamic);
                    if dynamic {
                        self.target_transform3(target, ahead)
                    } else {
                        poses
                            .get(target)
                            .copied()
                            .unwrap_or_else(|| self.mesh_transform(target))
                    }
                });
                let w = tf.apply(*local);
                let p = sim_ref.ppm;
                moves.push((i, *node, v3(w[0] / p, w[1] / p, w[2] / p)));
            }
        }
        if let Some(sim) = self.soft3.as_mut() {
            for (i, node, target) in moves {
                sim.bodies[i].set_pin_target(node, target);
            }
        }
    }

    /// Every mesh vertex of every soft body, in the pixels of the scene.
    pub fn soft3_collect(&self) -> Vec<(String, BTreeMap<u32, [f64; 3]>)> {
        let Some(sim) = self.soft3.as_ref() else {
            return vec![];
        };
        sim.names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut m = BTreeMap::new();
                for (vi, pi) in sim.vmap[i].iter().enumerate() {
                    let p = sim.bodies[i].point_pos(*pi);
                    m.insert(vi as u32, [p.x * sim.ppm, p.y * sim.ppm, p.z * sim.ppm]);
                }
                (n.clone(), m)
            })
            .collect()
    }

    pub fn soft3_snapshot(&self) -> Option<J> {
        let sim = self.soft3.as_ref()?;
        let bodies: Vec<J> = sim
            .names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let b = &sim.bodies[i];
                let pins: Vec<J> = (0..b.node_count())
                    .filter_map(|k| {
                        b.pins[k].map(|_| {
                            let p = b.pos[k];
                            json!([p.x * sim.ppm, p.y * sim.ppm, p.z * sim.ppm])
                        })
                    })
                    .collect();
                json!({"shape": n, "nodes": b.node_count(), "pins": pins})
            })
            .collect();
        Some(json!({"bodies": bodies}))
    }

    /// Write the final vertices back (the next scene starts from them) and drop the simulation.
    pub fn end_soft3(&mut self) {
        let out = self.soft3_collect();
        self.soft3 = None;
        for (n, m) in out {
            if let Some(mesh) = self.shapes.get_mut(&n).and_then(|s| s.mesh.as_mut()) {
                for (i, p) in m {
                    mesh.verts[i as usize] = p;
                }
            }
        }
    }
}
