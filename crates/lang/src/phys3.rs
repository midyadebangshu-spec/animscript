//! 3D physics in the language: `.body()` on a mesh shape, the 3D joints, the world built at the
//! start of a scene and the per-tick step. The engine is `animscript-physics3`; poses are written
//! back as the mesh's `position` (x, y, z) and an orientation quaternion.
//!
//! Units: the language works in pixels, the engine in metres (`scale` pixels per metre). A body's
//! frame is centred on its centre of mass, and the mesh turns about that point (`pivot3`).
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::mesh::Transform3;
use animscript_core::*;
use animscript_physics3 as p3;
use p3::{v3, BodyKind, Quat, V3};
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, BTreeSet};

type Args = Vec<(Option<String>, Value)>;

fn err(code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, msg)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    Auto,
    Sphere,
    Capsule,
    Hull,
}

#[derive(Clone, Debug)]
pub struct Body3Decl {
    pub shape: String,
    pub kind: BodyKind,
    pub density: f64,
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub gravity_scale: f64,
    pub fixed_rotation: bool,
    pub sensor: bool,
    pub collider: Collider,
}

#[derive(Clone, Debug)]
pub struct FloatDecl {
    pub shape: String,
    pub field: String,
    pub channel: usize,
    pub level: f64,
    pub density: f64,
    pub drag: f64,
}

#[derive(Clone, Debug)]
pub enum Joint3Spec {
    Ball {
        anchor: [f64; 3],
    },
    Hinge {
        anchor: [f64; 3],
        axis: [f64; 3],
        limits: Option<(f64, f64)>,
        motor: Option<(f64, f64)>,
    },
    Fixed {
        anchor: [f64; 3],
    },
    Strut {
        length: Option<f64>,
        spring: Option<(f64, f64)>,
        anchor_a: Option<[f64; 3]>,
        anchor_b: Option<[f64; 3]>,
    },
    Tether {
        max: f64,
        anchor_a: Option<[f64; 3]>,
        anchor_b: Option<[f64; 3]>,
    },
}

#[derive(Clone, Debug)]
pub struct Joint3Decl {
    pub a: String,
    pub b: String,
    pub collide: bool,
    pub spec: Joint3Spec,
}

#[derive(Clone, Debug)]
pub enum InitOp3 {
    Velocity(String, [f64; 3]),
    Impulse(String, [f64; 3]),
    Spin(String, [f64; 3]),
}

/// Declarations; the `bool` marks the ones made in a scene's `setup:` (dropped when it ends).
#[derive(Default)]
pub struct Decls3 {
    pub bodies: Vec<(bool, Body3Decl)>,
    pub joints: Vec<(bool, Joint3Decl)>,
    pub ops: Vec<(bool, InitOp3)>,
    pub floats: Vec<(bool, FloatDecl)>,
    pub soft: Vec<(bool, crate::soft3::Soft3Decl)>,
    pub pins: Vec<(bool, crate::soft3::Pin3Decl)>,
    pub soft_ops: Vec<(bool, (String, [f64; 3]))>,
}

impl Decls3 {
    pub fn start_scene(&mut self) {
        self.bodies.retain(|b| !b.0);
        self.joints.retain(|b| !b.0);
        self.ops.retain(|b| !b.0);
        self.floats.retain(|b| !b.0);
        self.soft.retain(|b| !b.0);
        self.pins.retain(|b| !b.0);
        self.soft_ops.retain(|b| !b.0);
    }
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty() && self.soft.is_empty()
    }
    pub fn body(&self, shape: &str) -> Option<&Body3Decl> {
        self.bodies.iter().map(|b| &b.1).find(|b| b.shape == shape)
    }
}

struct FloatRt {
    id: usize,
    decl: FloatDecl,
    /// Sample points in the body frame (metres) and the spacing between them.
    pts: Vec<V3>,
    spacing: f64,
    volume: f64,
}

/// The live 3D world of the running scene.
pub struct Physics3 {
    pub world: p3::World,
    pub(crate) ppm: f64,
    steps: usize,
    pub(crate) names: Vec<String>,
    pub(crate) ids: BTreeMap<String, usize>,
    /// The centre of mass in the mesh's own vertex coordinates (pixels).
    pub(crate) com: BTreeMap<String, [f64; 3]>,
    kinds: BTreeMap<String, BodyKind>,
    touching: BTreeSet<(usize, usize)>,
    began: BTreeSet<(usize, usize)>,
    floats: Vec<FloatRt>,
}

/// What a tick produced: for every dynamic body the shape position (x, y, z in pixels) and the
/// orientation quaternion `[w, x, y, z]`.
pub struct Poses3 {
    pub dynamic: Vec<(String, [f64; 3], [f64; 4])>,
}

pub(crate) fn mesh_volume(pts: &[[f64; 3]], faces: &[Vec<u32>]) -> f64 {
    let mut vol = 0.0;
    for f in faces {
        for k in 1..f.len().saturating_sub(1) {
            let (a, b, c) = (
                pts[f[0] as usize],
                pts[f[k] as usize],
                pts[f[k + 1] as usize],
            );
            let cr = [
                b[1] * c[2] - b[2] * c[1],
                b[2] * c[0] - b[0] * c[2],
                b[0] * c[1] - b[1] * c[0],
            ];
            vol += (a[0] * cr[0] + a[1] * cr[1] + a[2] * cr[2]) / 6.0;
        }
    }
    vol.abs()
}

/// Turn a rotation (quaternion) into the turns about x, y and z the mesh transform uses.
fn quat_to_turns(q: Quat) -> [f64; 3] {
    let m = q.to_mat3().m;
    let tau = std::f64::consts::TAU;
    let alpha = m[2][1].clamp(-1.0, 1.0).asin();
    let beta = (-m[2][0]).atan2(m[2][2]);
    let gamma = (-m[0][1]).atan2(m[1][1]);
    [alpha / tau, beta / tau, gamma / tau]
}

impl Physics3 {
    /// A world with no rigid bodies (soft bodies still need one to step with).
    pub fn empty(world: p3::World, ppm: f64, steps: usize) -> Physics3 {
        Physics3 {
            world,
            ppm,
            steps,
            names: vec![],
            ids: BTreeMap::new(),
            com: BTreeMap::new(),
            kinds: BTreeMap::new(),
            touching: BTreeSet::new(),
            began: BTreeSet::new(),
            floats: vec![],
        }
    }
}

impl Interp {
    pub fn is_mesh_shape_value(&self, v: &Value) -> bool {
        matches!(v, Value::Shape(sn) if self.is_mesh_shape(sn))
    }

    pub fn is_mesh_shape(&self, sn: &str) -> bool {
        self.shapes.get(sn).is_some_and(|s| s.mesh.is_some())
    }

    fn triple(&self, v: &Value, what: &str) -> Res<[f64; 3]> {
        match v {
            Value::Tuple(t) if t.len() == 3 => Ok([
                self.num(&t[0], what)?,
                self.num(&t[1], what)?,
                self.num(&t[2], what)?,
            ]),
            other => Err(err(
                "E-PHY-008",
                format!(
                    "{what} must be a 3D point like `(10, 20, 30)`, found {}",
                    other.type_name()
                ),
            )),
        }
    }

    // ---- shape methods ------------------------------------------------------------------

    pub fn body3_method(&mut self, sn: &str, name: &str, args: Args, mode: Mode) -> Res<Value> {
        match name {
            "body" => {
                if mode == Mode::Run {
                    return Err(Diagnostic::new("E-SHP-003", "`body` creates physics objects and is not allowed while the scene is running").help("bodies and joints are made at the top level or in `setup:`"));
                }
                let m = self.bind(
                    name,
                    args,
                    &[
                        "kind",
                        "density",
                        "friction",
                        "restitution",
                        "linear_damping",
                        "angular_damping",
                        "gravity_scale",
                        "fixed_rotation",
                        "sensor",
                        "collider",
                    ],
                    &[],
                )?;
                let kind = match m.get("kind") {
                    None => BodyKind::Dynamic,
                    Some(Value::Str(s)) => match s.as_str() {
                        "static" => BodyKind::Static,
                        "dynamic" => BodyKind::Dynamic,
                        "kinematic" => BodyKind::Kinematic,
                        other => {
                            return Err(err("E-PHY-007", format!("unknown body kind `{other}`"))
                                .help("kinds: static, dynamic, kinematic"))
                        }
                    },
                    Some(o) => {
                        return Err(err(
                            "E-PHY-007",
                            format!(
                                "`kind` must be static, dynamic or kinematic, found {}",
                                o.type_name()
                            ),
                        ))
                    }
                };
                let collider = match m.get("collider") {
                    None => Collider::Auto,
                    Some(Value::Str(s)) => match s.as_str() {
                        "auto" => Collider::Auto,
                        "sphere" => Collider::Sphere,
                        "capsule" => Collider::Capsule,
                        "hull" => Collider::Hull,
                        other => {
                            return Err(err("E-PHY-007", format!("unknown collider `{other}`"))
                                .help("colliders: auto, sphere, capsule, hull"))
                        }
                    },
                    Some(o) => {
                        return Err(err(
                            "E-PHY-007",
                            format!("`collider` must be a string, found {}", o.type_name()),
                        ))
                    }
                };
                let f = |k: &str, d: f64| -> Res<f64> {
                    m.get(k).map(|v| self.num(v, k)).unwrap_or(Ok(d))
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
                let decl = Body3Decl {
                    shape: sn.to_string(),
                    kind,
                    density: f("density", 1.0)?,
                    friction: f("friction", 0.6)?,
                    restitution: f("restitution", 0.0)?,
                    linear_damping: f("linear_damping", 0.0)?,
                    angular_damping: f("angular_damping", 0.0)?,
                    gravity_scale: f("gravity_scale", 1.0)?,
                    fixed_rotation: b("fixed_rotation")?,
                    sensor: b("sensor")?,
                    collider,
                };
                if decl.density <= 0.0 || decl.friction < 0.0 || decl.restitution < 0.0 {
                    return Err(err("E-PHY-007", "`density` must be positive, `friction` and `restitution` cannot be negative"));
                }
                if self.decls3.body(sn).is_some() {
                    return Err(err("E-PHY-007", format!("`{sn}` already has a body")));
                }
                let sh = &self.shapes[sn];
                if sh.scale[0] != sh.scale[1] || sh.scale[0] != sh.scale_z {
                    return Err(err(
                        "E-PHY-007",
                        format!("`{sn}` has a non-uniform scale, which a body cannot have"),
                    )
                    .help("use a uniform scale, or reshape the geometry"));
                }
                if !self.decls.bodies.is_empty() {
                    return Err(err(
                        "E-PHY-007",
                        "a scene has either 2D bodies or mesh bodies, not both (yet)",
                    ));
                }
                let scene = mode == Mode::Setup;
                self.decls3.bodies.push((scene, decl));
                Ok(Value::None)
            }
            "apply_impulse" | "set_velocity" | "set_spin" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let by = self.triple(&m["by"], "a 3D vector")?;
                if self.decls3.body(sn).is_none() {
                    return Err(err(
                        "E-PHY-007",
                        format!("`{sn}` has no body; call `{sn}.body()` first"),
                    ));
                }
                if mode == Mode::Run {
                    let Some(ph) = self.phys3.as_mut() else {
                        return Ok(Value::None);
                    };
                    let Some(&id) = ph.ids.get(sn) else {
                        return Ok(Value::None);
                    };
                    let v = v3(by[0] / ph.ppm, by[1] / ph.ppm, by[2] / ph.ppm);
                    match name {
                        "set_velocity" => ph.world.set_linear_velocity(id, v),
                        "apply_impulse" => ph.world.apply_linear_impulse(id, v),
                        _ => {
                            let w = v3(by[0], by[1], by[2]) * std::f64::consts::TAU;
                            ph.world.set_angular_velocity(id, w)
                        }
                    }
                } else {
                    let scene = mode == Mode::Setup;
                    let op = match name {
                        "set_velocity" => InitOp3::Velocity(sn.to_string(), by),
                        "apply_impulse" => InitOp3::Impulse(sn.to_string(), by),
                        _ => InitOp3::Spin(sn.to_string(), by),
                    };
                    self.decls3.ops.push((scene, op));
                }
                Ok(Value::None)
            }
            "float" => {
                if mode == Mode::Run {
                    return Err(Diagnostic::new(
                        "E-SHP-003",
                        "`float` is set up before the scene runs",
                    ));
                }
                let m = self.bind(
                    name,
                    args,
                    &["field", "channel", "level", "density", "drag"],
                    &["field", "level"],
                )?;
                let Value::Field(fname) = &m["field"] else {
                    return Err(err(
                        "E-PHY-007",
                        "`field` must be a field, such as `field: lagoon`",
                    ));
                };
                let Some(cfg) = self.fields.get(fname) else {
                    return Err(err("E-PHY-007", format!("unknown field `{fname}`")));
                };
                let channel = match m.get("channel") {
                    None => 0,
                    Some(Value::Str(c)) => {
                        cfg.channels.iter().position(|x| x == c).ok_or_else(|| {
                            err("E-PHY-007", format!("field `{fname}` has no channel `{c}`"))
                        })?
                    }
                    Some(_) => {
                        return Err(err("E-PHY-007", "`channel` is a channel name in quotes"))
                    }
                };
                let level = self.num(&m["level"], "`level`")?;
                let density = m
                    .get("density")
                    .map(|v| self.num(v, "`density`"))
                    .unwrap_or(Ok(2.0))?;
                let drag = m
                    .get("drag")
                    .map(|v| self.num(v, "`drag`"))
                    .unwrap_or(Ok(1.0))?;
                if density <= 0.0 || drag < 0.0 {
                    return Err(err(
                        "E-PHY-007",
                        "`density` must be positive and `drag` cannot be negative",
                    ));
                }
                if self.decls3.body(sn).is_none() {
                    return Err(err(
                        "E-PHY-007",
                        format!("`{sn}` has no body; call `{sn}.body()` first"),
                    ));
                }
                let decl = FloatDecl {
                    shape: sn.to_string(),
                    field: fname.clone(),
                    channel,
                    level,
                    density,
                    drag,
                };
                let scene = mode == Mode::Setup;
                self.decls3.floats.push((scene, decl));
                Ok(Value::None)
            }
            "touches" | "leaves" => {
                let m = self.bind(name, args, &["other"], &["other"])?;
                let Value::Shape(other) = &m["other"] else {
                    return Err(Diagnostic::new(
                        "E-TYP-001",
                        format!("`{name}` needs a shape"),
                    ));
                };
                if name == "touches" {
                    let t = self.phys3.as_ref().is_some_and(|ph| {
                        match (ph.ids.get(sn), ph.ids.get(other)) {
                            (Some(a), Some(b)) => {
                                let k = (*a.min(b), *a.max(b));
                                ph.touching.contains(&k) || ph.began.contains(&k)
                            }
                            _ => false,
                        }
                    });
                    Ok(Value::Bool(t))
                } else {
                    Ok(Value::Bool(
                        self.events.contains(&crate::phys::leave_key(sn, other)),
                    ))
                }
            }
            _ => Err(err(
                "E-PHY-007",
                format!("`{name}` is not available on a mesh body"),
            )),
        }
    }

    // ---- joints -------------------------------------------------------------------------

    pub fn is_joint3_function(name: &str) -> bool {
        matches!(name, "ball" | "hinge" | "fixed" | "strut" | "tether")
    }

    fn body3_arg(&self, v: &Value, what: &str) -> Res<String> {
        match v {
            Value::Shape(n) if self.decls3.body(n).is_some() => Ok(n.clone()),
            Value::Shape(n) => Err(err(
                "E-PHY-008",
                format!("{what} `{n}` is not a mesh body; call `{n}.body()` first"),
            )),
            other => Err(err(
                "E-PHY-008",
                format!("{what} must be a mesh shape, found {}", other.type_name()),
            )),
        }
    }

    pub fn joint3_function(&mut self, name: &str, args: Args, mode: Mode) -> Res<Value> {
        if mode == Mode::Run {
            return Err(Diagnostic::new(
                "E-SHP-003",
                format!("`{name}` creates physics objects and is not allowed while the scene is running"),
            ));
        }
        let opt3 = |s: &Interp,
                    m: &std::collections::HashMap<&'static str, Value>,
                    k: &str|
         -> Res<Option<[f64; 3]>> {
            m.get(k).map(|v| s.triple(v, &format!("`{k}`"))).transpose()
        };
        let collide_of =
            |s: &Interp, m: &std::collections::HashMap<&'static str, Value>| -> Res<bool> {
                let _ = s;
                match m.get("collide") {
                    None => Ok(false),
                    Some(Value::Bool(b)) => Ok(*b),
                    Some(o) => Err(Diagnostic::new(
                        "E-TYP-001",
                        format!("`collide` must be true or false, found {}", o.type_name()),
                    )),
                }
            };
        let (a, b, collide, spec) = match name {
            "ball" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "anchor", "collide"],
                    &["a", "b", "anchor"],
                )?;
                (
                    self.body3_arg(&m["a"], "`a`")?,
                    self.body3_arg(&m["b"], "`b`")?,
                    collide_of(self, &m)?,
                    Joint3Spec::Ball {
                        anchor: self.triple(&m["anchor"], "`anchor`")?,
                    },
                )
            }
            "fixed" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "anchor", "collide"],
                    &["a", "b", "anchor"],
                )?;
                (
                    self.body3_arg(&m["a"], "`a`")?,
                    self.body3_arg(&m["b"], "`b`")?,
                    collide_of(self, &m)?,
                    Joint3Spec::Fixed {
                        anchor: self.triple(&m["anchor"], "`anchor`")?,
                    },
                )
            }
            "hinge" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "anchor", "axis", "limits", "motor", "collide"],
                    &["a", "b", "anchor", "axis"],
                )?;
                let axis = self.triple(&m["axis"], "`axis`")?;
                if axis == [0.0; 3] {
                    return Err(err("E-PHY-008", "the hinge `axis` cannot be (0, 0, 0)"));
                }
                let limits = m
                    .get("limits")
                    .map(|v| self.angle_pair(v, "`limits`"))
                    .transpose()?
                    .map(|(lo, hi)| (lo * std::f64::consts::TAU, hi * std::f64::consts::TAU));
                if limits.is_some_and(|l| l.0 > l.1) {
                    return Err(err("E-PHY-008", "the lower limit is above the upper limit"));
                }
                let motor = match m.get("motor") {
                    None => None,
                    Some(Value::Tuple(t)) if t.len() == 2 => match &t[0] {
                        Value::Angle(a) => Some((a * std::f64::consts::TAU, self.num(&t[1], "the motor torque")?)),
                        _ => return Err(err("E-PHY-008", "`motor` is (speed, torque) with the speed as an angle per second, such as `(90.deg, 50)`")),
                    },
                    Some(_) => return Err(err("E-PHY-008", "`motor` is (speed, torque), such as `(90.deg, 50)`")),
                };
                (
                    self.body3_arg(&m["a"], "`a`")?,
                    self.body3_arg(&m["b"], "`b`")?,
                    collide_of(self, &m)?,
                    Joint3Spec::Hinge {
                        anchor: self.triple(&m["anchor"], "`anchor`")?,
                        axis,
                        limits,
                        motor,
                    },
                )
            }
            "strut" => {
                let m = self.bind(
                    name,
                    args,
                    &[
                        "a", "b", "length", "spring", "anchor_a", "anchor_b", "collide",
                    ],
                    &["a", "b"],
                )?;
                let spring = match m.get("spring") {
                    None => None,
                    Some(Value::Tuple(t)) if t.len() == 2 => Some((
                        self.num(&t[0], "the spring frequency")?,
                        self.num(&t[1], "the spring damping")?,
                    )),
                    Some(_) => {
                        return Err(err(
                            "E-PHY-008",
                            "`spring` is (frequency, damping), such as `(2, 0.5)`",
                        ))
                    }
                };
                let length = m
                    .get("length")
                    .map(|v| self.num(v, "`length`"))
                    .transpose()?;
                if length.is_some_and(|l| l < 0.0) {
                    return Err(err("E-PHY-008", "`length` cannot be negative"));
                }
                (
                    self.body3_arg(&m["a"], "`a`")?,
                    self.body3_arg(&m["b"], "`b`")?,
                    collide_of(self, &m)?,
                    Joint3Spec::Strut {
                        length,
                        spring,
                        anchor_a: opt3(self, &m, "anchor_a")?,
                        anchor_b: opt3(self, &m, "anchor_b")?,
                    },
                )
            }
            _ => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "max", "anchor_a", "anchor_b", "collide"],
                    &["a", "b", "max"],
                )?;
                let max = self.num(&m["max"], "`max`")?;
                if max <= 0.0 {
                    return Err(err("E-PHY-008", "`max` must be positive"));
                }
                (
                    self.body3_arg(&m["a"], "`a`")?,
                    self.body3_arg(&m["b"], "`b`")?,
                    collide_of(self, &m)?,
                    Joint3Spec::Tether {
                        max,
                        anchor_a: opt3(self, &m, "anchor_a")?,
                        anchor_b: opt3(self, &m, "anchor_b")?,
                    },
                )
            }
        };
        if a == b {
            return Err(err("E-PHY-008", "a joint needs two different bodies"));
        }
        let scene = mode == Mode::Setup;
        self.decls3.joints.push((
            scene,
            Joint3Decl {
                a,
                b,
                collide,
                spec,
            },
        ));
        Ok(Value::None)
    }

    // ---- scene start and end ------------------------------------------------------------

    /// The mesh's vertices in pixels with the shape's current transform applied.
    pub(crate) fn mesh_transform(&self, sn: &str) -> Transform3 {
        let sh = &self.shapes[sn];
        let tf = sh.transform();
        Transform3 {
            position: [tf.position[0], tf.position[1], sh.pos_z],
            rot: [sh.rot_x, sh.rot_y, tf.rotation],
            scale: [tf.scale[0], tf.scale[1], sh.scale_z],
            pivot: sh.pivot3,
            orient: sh.orient,
        }
    }

    pub fn build_physics3(&mut self) -> Res<()> {
        if self.decls3.is_empty() {
            self.phys3 = None;
            return Ok(());
        }
        let cfg = self.world_cfg.clone();
        let ppm = cfg.scale;
        let g = cfg.gravity3;
        let mut world = p3::World::new(v3(g[0], g[1], g[2]));
        let mut ph = Physics3 {
            world: p3::World::new(V3::ZERO),
            ppm,
            steps: cfg.substeps,
            names: vec![],
            ids: BTreeMap::new(),
            com: BTreeMap::new(),
            kinds: BTreeMap::new(),
            touching: BTreeSet::new(),
            began: BTreeSet::new(),
            floats: vec![],
        };
        let decls: Vec<Body3Decl> = self.decls3.bodies.iter().map(|b| b.1.clone()).collect();
        for d in &decls {
            let sh = self.shapes[&d.shape].clone();
            let mesh = sh.mesh.clone().expect("mesh body");
            let s = sh.scale[0];
            let t = self.mesh_transform(&d.shape);
            let faces: Vec<Vec<u32>> = mesh.faces.iter().map(|f| f.to_vec()).collect();
            let vol_px = mesh_volume(&mesh.verts, &faces) * s * s * s;
            let q0 = match sh.orient {
                Some(o) => Quat::from_arr(o),
                None => Quat::from_euler_turns(sh.rot_x, sh.rot_y, sh.transform().rotation),
            };
            let verts: Vec<V3> = mesh.verts.iter().map(|v| V3::from_arr(*v)).collect();
            let scaled: Vec<V3> = verts.iter().map(|v| *v * (s / ppm)).collect();
            let name = d.shape.clone();
            let bad = |what: &str| {
                err(
                    "E-PHY-007",
                    format!("`{name}` cannot be a {} body: {what}", kind_name(d.kind)),
                )
            };
            let hull = p3::Hull::from_points(&scaled);
            let hull_vol = hull.as_ref().map(|h| h.mass_props().volume).unwrap_or(0.0);
            let convex = hull_vol > 0.0 && vol_px / (ppm * ppm * ppm) >= 0.85 * hull_vol;
            // static bodies that are not convex become a triangle mesh in world space
            let as_tris = d.kind == BodyKind::Static && d.collider == Collider::Auto && !convex;
            if as_tris {
                let mut tris = vec![];
                for f in &faces {
                    for k in 1..f.len() - 1 {
                        let w = |i: u32| {
                            let p = t.apply(mesh.verts[i as usize]);
                            v3(p[0] / ppm, p[1] / ppm, p[2] / ppm)
                        };
                        if let Some(h) = p3::Hull::triangle(w(f[0]), w(f[k]), w(f[k + 1])) {
                            tris.push(h);
                        }
                    }
                }
                if tris.is_empty() {
                    return Err(bad("it has no faces"));
                }
                let mut def = p3::BodyDef::new(p3::Shape::Tris(tris));
                def.kind = BodyKind::Static;
                def.friction = d.friction;
                def.restitution = d.restitution;
                def.sensor = d.sensor;
                let id = world.create_body(&def);
                ph.names.push(d.shape.clone());
                ph.ids.insert(d.shape.clone(), id);
                ph.com.insert(d.shape.clone(), sh.pivot3);
                ph.kinds.insert(d.shape.clone(), d.kind);
                continue;
            }
            let (shape, com_m) = match d.collider {
                Collider::Sphere => {
                    let c = scaled.iter().fold(V3::ZERO, |a, v| a + *v) / scaled.len() as f64;
                    let r =
                        scaled.iter().map(|v| (*v - c).len()).sum::<f64>() / scaled.len() as f64;
                    (p3::Shape::Sphere(r), c)
                }
                Collider::Capsule => {
                    let (mut lo, mut hi) = (scaled[0], scaled[0]);
                    for v in &scaled {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                    let c = (lo + hi) * 0.5;
                    let r = ((hi.x - lo.x).max(hi.z - lo.z)) * 0.5;
                    let half = ((hi.y - lo.y) * 0.5 - r).max(0.0);
                    (p3::Shape::Capsule { half, r }, c)
                }
                Collider::Auto | Collider::Hull => {
                    // a sphere-like mesh rolls as a true sphere
                    let c = scaled.iter().fold(V3::ZERO, |a, v| a + *v) / scaled.len() as f64;
                    let rs: Vec<f64> = scaled.iter().map(|v| (*v - c).len()).collect();
                    let mean = rs.iter().sum::<f64>() / rs.len() as f64;
                    let round = d.collider == Collider::Auto
                        && mean > 0.0
                        && hull.as_ref().is_some_and(|h| h.faces.len() >= 20)
                        && rs.iter().all(|r| (r - mean).abs() < 0.02 * mean);
                    if round {
                        (p3::Shape::Sphere(mean), c)
                    } else {
                        if d.collider == Collider::Auto && !convex {
                            return Err(bad("its shape is not convex").help("use `collider: \"hull\"` to collide as the convex hull of the mesh (it fills any dents), or build the body from several convex shapes joined with `fixed`"));
                        }
                        let Some(h) = hull else {
                            return Err(bad("it has no volume"));
                        };
                        let c = h.mass_props().centroid;
                        (p3::Shape::Hull(h.translated(-c)), c)
                    }
                }
            };
            let com_v = com_m * (ppm / s); // the centre of mass in vertex coordinates (pixels)
                                           // the mesh now turns about its centre of mass; the pose is unchanged
            let origin = t.apply(com_v.arr());
            let mut def = p3::BodyDef::new(shape);
            def.kind = d.kind;
            def.position = v3(origin[0] / ppm, origin[1] / ppm, origin[2] / ppm);
            def.orientation = q0;
            def.density = d.density;
            def.friction = d.friction;
            def.restitution = d.restitution;
            def.linear_damping = d.linear_damping;
            def.angular_damping = d.angular_damping;
            def.gravity_scale = d.gravity_scale;
            def.fixed_rotation = d.fixed_rotation;
            def.sensor = d.sensor;
            let id = world.create_body(&def);
            ph.names.push(d.shape.clone());
            ph.ids.insert(d.shape.clone(), id);
            ph.com.insert(d.shape.clone(), com_v.arr());
            ph.kinds.insert(d.shape.clone(), d.kind);
            let shape_mut = self.shapes.get_mut(&d.shape).expect("declared shape");
            shape_mut.pivot3 = com_v.arr();
            shape_mut.position = [origin[0] - com_v.x, origin[1] - com_v.y];
            shape_mut.pos_z = origin[2] - com_v.z;
            if d.kind == BodyKind::Dynamic {
                shape_mut.orient = Some(q0.arr());
            }
            if let Some(f) = self.decls3.floats.iter().find(|f| f.1.shape == d.shape) {
                let b = world.body(id);
                let (lo, hi) = match &b.shape {
                    p3::Shape::Sphere(r) => (v3(-r, -r, -r), v3(*r, *r, *r)),
                    p3::Shape::Capsule { half, r } => (v3(-r, -half - r, -r), v3(*r, half + r, *r)),
                    p3::Shape::Hull(h) => {
                        let mut lo = h.verts[0];
                        let mut hi = lo;
                        for v in &h.verts {
                            lo = lo.min(*v);
                            hi = hi.max(*v);
                        }
                        (lo, hi)
                    }
                    _ => (V3::ZERO, V3::ZERO),
                };
                let mut pts = vec![];
                for i in 0..3 {
                    for j in 0..3 {
                        for k in 0..3 {
                            let u = |t: usize, a: f64, b: f64| a + (b - a) * (t as f64 + 0.5) / 3.0;
                            pts.push(v3(u(i, lo.x, hi.x), u(j, lo.y, hi.y), u(k, lo.z, hi.z)));
                        }
                    }
                }
                let spacing = ((hi - lo).len() / 3f64.sqrt() / 3.0).max(1e-6);
                let volume = if b.mass > 0.0 {
                    b.mass / d.density
                } else {
                    0.0
                };
                ph.floats.push(FloatRt {
                    id,
                    decl: f.1.clone(),
                    pts,
                    spacing,
                    volume,
                });
            }
        }
        let joints: Vec<Joint3Decl> = self.decls3.joints.iter().map(|j| j.1.clone()).collect();
        for j in &joints {
            let (ia, ib) = (ph.ids[&j.a], ph.ids[&j.b]);
            let w = |p: [f64; 3]| v3(p[0] / ppm, p[1] / ppm, p[2] / ppm);
            let body_p = |id: usize| world.body(id).p;
            let mut def = p3::JointDef {
                a: ia,
                b: ib,
                kind: p3::JointKind::Ball,
                anchor_a: V3::ZERO,
                anchor_b: None,
                axis: V3::ZERO,
                collide_connected: j.collide,
            };
            match &j.spec {
                Joint3Spec::Ball { anchor } => def.anchor_a = w(*anchor),
                Joint3Spec::Fixed { anchor } => {
                    def.kind = p3::JointKind::Fixed;
                    def.anchor_a = w(*anchor);
                }
                Joint3Spec::Hinge {
                    anchor,
                    axis,
                    limits,
                    motor,
                } => {
                    def.kind = p3::JointKind::Hinge {
                        limits: *limits,
                        motor: motor.map(|(s, t)| (s, t / ppm / ppm)),
                    };
                    def.anchor_a = w(*anchor);
                    def.axis = v3(axis[0], axis[1], axis[2]);
                }
                Joint3Spec::Strut {
                    length,
                    spring,
                    anchor_a,
                    anchor_b,
                } => {
                    def.kind = p3::JointKind::Distance {
                        length: length.map(|l| l / ppm).unwrap_or(-1.0),
                        spring: *spring,
                        rope: false,
                    };
                    def.anchor_a = anchor_a.map(w).unwrap_or_else(|| body_p(ia));
                    def.anchor_b = Some(anchor_b.map(w).unwrap_or_else(|| body_p(ib)));
                }
                Joint3Spec::Tether {
                    max,
                    anchor_a,
                    anchor_b,
                } => {
                    def.kind = p3::JointKind::Distance {
                        length: max / ppm,
                        spring: None,
                        rope: true,
                    };
                    def.anchor_a = anchor_a.map(w).unwrap_or_else(|| body_p(ia));
                    def.anchor_b = Some(anchor_b.map(w).unwrap_or_else(|| body_p(ib)));
                }
            }
            world.create_joint(&def);
        }
        let ops: Vec<InitOp3> = self.decls3.ops.iter().map(|o| o.1.clone()).collect();
        for op in &ops {
            match op {
                InitOp3::Velocity(n, v) => {
                    world.set_linear_velocity(ph.ids[n], v3(v[0] / ppm, v[1] / ppm, v[2] / ppm))
                }
                InitOp3::Impulse(n, v) => {
                    world.apply_linear_impulse(ph.ids[n], v3(v[0] / ppm, v[1] / ppm, v[2] / ppm))
                }
                InitOp3::Spin(n, v) => world
                    .set_angular_velocity(ph.ids[n], v3(v[0], v[1], v[2]) * std::f64::consts::TAU),
            }
        }
        for (a, b) in world.overlapping_pairs(0.01) {
            self.diagnostics.push(
                err(
                    "W-PHY-001",
                    format!(
                        "`{}` and `{}` overlap at the start of the scene",
                        ph.names[a], ph.names[b]
                    ),
                )
                .help("move them apart; overlapping bodies are pushed out of each other"),
            );
        }
        ph.world = world;
        self.phys3 = Some(ph);
        Ok(())
    }

    pub fn kinematic_bodies3(&self) -> Vec<String> {
        self.phys3
            .as_ref()
            .map(|p| {
                p.kinds
                    .iter()
                    .filter(|k| *k.1 == BodyKind::Kinematic)
                    .map(|k| k.0.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Kinematic bodies are given the velocity that reaches the pose the other layers resolved
    /// (`kin`: x, y, z in pixels and the turns about x, y, z).
    pub fn physics3_prepare(&mut self, tick: i64, kin: &BTreeMap<String, [f64; 6]>) {
        let fps = self.fps.to_f64();
        let Some(ph) = self.phys3.as_mut() else {
            return;
        };
        ph.began.clear();
        if tick > 0 {
            for (name, pose) in kin {
                let (id, com) = (ph.ids[name], ph.com[name]);
                let p = v3(
                    (pose[0] + com[0]) / ph.ppm,
                    (pose[1] + com[1]) / ph.ppm,
                    (pose[2] + com[2]) / ph.ppm,
                );
                let q = Quat::from_euler_turns(pose[3], pose[4], pose[5]);
                ph.world.move_kinematic(id, p, q, 1.0 / fps);
            }
        }
    }

    /// Floating bodies: buoyancy and drag from a field's height, sampled at the body's points.
    fn apply_floats(&mut self, ph: &mut Physics3) {
        let g = ph.world.gravity;
        let gl = g.len();
        if gl < 1e-12 {
            return;
        }
        let up = -g / gl;
        for f in &ph.floats {
            let (p, q) = {
                let b = ph.world.body(f.id);
                (b.p, b.q)
            };
            let mut sub = 0.0;
            let mut pts = vec![];
            for lp in &f.pts {
                let wp = p + q.rotate(*lp);
                let (px, py, pz) = (wp.x * ph.ppm, wp.y * ph.ppm, wp.z * ph.ppm);
                let h = self.field_sample(&f.decl.field, f.decl.channel, px, pz);
                let surface = f.decl.level - h;
                let depth = (py - surface) / ph.ppm;
                let frac = (depth / f.spacing + 0.5).clamp(0.0, 1.0);
                sub += frac;
                pts.push((wp, frac));
            }
            if sub <= 0.0 {
                continue;
            }
            let each = f.decl.density * gl * f.volume / f.pts.len() as f64;
            for (wp, frac) in pts {
                if frac > 0.0 {
                    ph.world.apply_force_at(f.id, up * (each * frac), wp);
                }
            }
            // water resists motion in proportion to how much of the body is under it
            let wet = sub / f.pts.len() as f64;
            let b = ph.world.body(f.id);
            let (v, w, m) = (b.v, b.w, b.mass);
            ph.world
                .apply_force_at(f.id, -v * (f.decl.drag * m * wet), p);
            let drag_t = -w * (f.decl.drag * m * wet * f.spacing * f.spacing);
            // an angular drag as a couple about the centre
            let arm = v3(1.0, 0.0, 0.0);
            let dir = drag_t.cross(arm);
            if dir.len2() > 0.0 {
                let force = dir * (1.0 / arm.len2());
                ph.world.apply_force_at(f.id, force, p + arm);
                ph.world.apply_force_at(f.id, -force, p);
            }
        }
    }

    pub fn physics3_advance(&mut self, tick: i64) {
        let fps = self.fps.to_f64();
        let Some(mut ph) = self.phys3.take() else {
            return;
        };
        if tick > 0 {
            let dt = 1.0 / fps;
            self.fluid3_emit_for(dt);
            let cfg = self.world_cfg.clone();
            let soft = self.soft3.take();
            let fluid = self.fluid3.take();
            if soft.is_some() || fluid.is_some() {
                let mut soft = soft;
                let mut fluid = fluid;
                if let Some(f) = fluid.as_mut() {
                    self.couple_fluids_to_fields(f);
                }
                let has_soft = soft.is_some();
                let (mut empty_s, mut empty_f) = (vec![], vec![]);
                let bodies = soft.as_mut().map(|s| &mut s.bodies).unwrap_or(&mut empty_s);
                let fluids = fluid
                    .as_mut()
                    .map(|f| &mut f.fluids)
                    .unwrap_or(&mut empty_f);
                let substeps = if has_soft {
                    cfg.soft_substeps
                } else {
                    cfg.fluid_substeps
                };
                p3::coupled_step3(
                    &mut ph.world,
                    bodies,
                    fluids,
                    dt,
                    substeps,
                    cfg.soft_iterations,
                    cfg.fluid_iterations,
                    cfg.fluid_substeps,
                );
                if let Some(f) = fluid.as_mut() {
                    self.ripples_from_fluids(f);
                }
                self.soft3 = soft;
                self.fluid3 = fluid;
            } else {
                for _ in 0..ph.steps {
                    self.apply_floats(&mut ph);
                    ph.world.step(dt / ph.steps as f64);
                }
            }
        }
        self.phys3 = Some(ph);
    }

    pub fn physics3_collect(&mut self) -> Poses3 {
        let mut leaves: Vec<String> = vec![];
        let Some(ph) = self.phys3.as_mut() else {
            return Poses3 { dynamic: vec![] };
        };
        for e in ph.world.take_events() {
            if e.begin {
                ph.touching.insert((e.a, e.b));
                ph.began.insert((e.a, e.b));
            } else {
                ph.touching.remove(&(e.a, e.b));
                leaves.push(crate::phys::leave_key(&ph.names[e.a], &ph.names[e.b]));
            }
        }
        let mut dynamic = vec![];
        for n in ph.names.clone() {
            if ph.kinds.get(&n) != Some(&BodyKind::Dynamic) {
                continue;
            }
            let b = ph.world.body(ph.ids[&n]);
            let com = ph.com[&n];
            dynamic.push((
                n,
                [
                    b.p.x * ph.ppm - com[0],
                    b.p.y * ph.ppm - com[1],
                    b.p.z * ph.ppm - com[2],
                ],
                b.q.arr(),
            ));
        }
        for l in leaves {
            self.events.insert(l);
        }
        Poses3 { dynamic }
    }

    pub fn physics3_snapshot(&self) -> Option<J> {
        let ph = self.phys3.as_ref()?;
        let px = |v: V3| json!([v.x * ph.ppm, v.y * ph.ppm, v.z * ph.ppm]);
        let bodies: Vec<J> = ph
            .names
            .iter()
            .map(|n| {
                let b = ph.world.body(ph.ids[n]);
                json!({"shape": n, "kind": kind_name(b.kind), "awake": b.awake})
            })
            .collect();
        let contacts: Vec<J> = ph
            .world
            .contact_infos()
            .iter()
            .map(|c| json!({"a": ph.names[c.a], "b": ph.names[c.b], "point": px(c.point), "normal": [c.normal.x, c.normal.y, c.normal.z], "impulse": c.normal_impulse}))
            .collect();
        let joints: Vec<J> = (0..ph.world.joint_count())
            .map(|i| {
                let (a, b) = ph.world.joint_anchors(i);
                json!({"a": px(a), "b": px(b)})
            })
            .collect();
        Some(json!({"bodies": bodies, "contacts": contacts, "joints": joints}))
    }

    /// Write the final poses back as rest values and drop the world.
    pub fn end_physics3(&mut self) {
        let Some(ph) = self.phys3.take() else { return };
        for n in &ph.names {
            if ph.kinds.get(n) != Some(&BodyKind::Dynamic) {
                continue;
            }
            let b = ph.world.body(ph.ids[n]);
            let com = ph.com[n];
            if let Some(sh) = self.shapes.get_mut(n) {
                sh.position = [b.p.x * ph.ppm - com[0], b.p.y * ph.ppm - com[1]];
                sh.pos_z = b.p.z * ph.ppm - com[2];
                let t = quat_to_turns(b.q);
                sh.rot_x = t[0];
                sh.rot_y = t[1];
                sh.rotation = t[2];
                sh.orient = None;
            }
        }
    }
}

fn kind_name(k: BodyKind) -> &'static str {
    match k {
        BodyKind::Static => "static",
        BodyKind::Dynamic => "dynamic",
        BodyKind::Kinematic => "kinematic",
    }
}
