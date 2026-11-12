//! The physics layer of the language: declarations made while building (`world:`, `.body()`,
//! joints), the world constructed at the start of each scene, and the per-tick step.
//!
//! Units: the language works in pixels, seconds and kilograms; the engine in metres. `scale` (px per
//! metre, default 100) converts. A body's pose is the shape's own `position` / `rotation` with the
//! pivot at the centre of mass; dynamic bodies write layer-2 override contributions, kinematic
//! bodies read the pose the other layers resolved, and keyframes (layer 3) still win.
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::*;
use animscript_physics as ph;
use ph::{v2, BodyKind, JointDef, Vec2};
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, BTreeSet};

type Args = Vec<(Option<String>, Value)>;

#[derive(Clone, Debug)]
pub struct WorldCfg {
    pub gravity: [f64; 2],
    pub scale: f64,
    pub substeps: usize,
    pub soft_substeps: usize,
    pub soft_iterations: usize,
    pub fluid_substeps: usize,
    pub fluid_iterations: usize,
}

impl Default for WorldCfg {
    fn default() -> WorldCfg {
        WorldCfg {
            gravity: [0.0, 9.8],
            scale: 100.0,
            substeps: 4,
            soft_substeps: 16,
            soft_iterations: 4,
            fluid_substeps: 4,
            fluid_iterations: 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BodyDecl {
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
}

/// A character controller: walk and jump a dynamic body that never tips over.
#[derive(Clone, Debug)]
pub struct ControllerDecl {
    pub shape: String,
    /// Top walking speed, px/s.
    pub speed: f64,
    /// How fast the horizontal velocity changes on the ground, px/s^2.
    pub accel: f64,
    /// Take-off speed of a jump, px/s.
    pub jump: f64,
    /// Fraction of `accel` available in the air.
    pub air_control: f64,
}

/// Live controller state of the running scene.
#[derive(Clone, Debug)]
struct Ctrl {
    decl: ControllerDecl,
    input: f64,
    jump_pending: bool,
    grounded: bool,
    /// Half width and the distance from the centre of mass to the feet, metres.
    half_w: f64,
    feet: f64,
}

#[derive(Clone, Debug)]
pub enum JointSpec {
    Revolute {
        anchor: [f64; 2],
        limits: Option<(f64, f64)>,
        motor: Option<(f64, f64)>,
    },
    Weld {
        anchor: [f64; 2],
    },
    Prismatic {
        anchor: Option<[f64; 2]>,
        axis: [f64; 2],
        limits: Option<(f64, f64)>,
    },
    Distance {
        length: Option<f64>,
        spring: Option<(f64, f64)>,
        anchor_a: Option<[f64; 2]>,
        anchor_b: Option<[f64; 2]>,
    },
    Rope {
        max: f64,
        anchor_a: Option<[f64; 2]>,
        anchor_b: Option<[f64; 2]>,
    },
    Follow {
        target: [f64; 2],
        force: f64,
        spring: (f64, f64),
    },
}

#[derive(Clone, Debug)]
pub struct JointDecl {
    pub a: String,
    pub b: String,
    pub collide: bool,
    pub spec: JointSpec,
}

#[derive(Clone, Debug)]
pub enum InitOp {
    Velocity(String, [f64; 2]),
    Impulse(String, [f64; 2]),
}

/// Declarations; `scene` ones were made in a scene's `setup:` and are dropped when it ends.
#[derive(Default)]
pub struct Decls {
    pub bodies: Vec<(bool, BodyDecl)>,
    pub joints: Vec<(bool, JointDecl)>,
    pub ops: Vec<(bool, InitOp)>,
    pub soft: Vec<(bool, crate::soft::SoftDecl)>,
    pub binds: Vec<(bool, crate::rig::BindDecl)>,
    pub skins: Vec<(bool, crate::rig::SkinDecl)>,
    pub controllers: Vec<(bool, ControllerDecl)>,
}

impl Decls {
    pub fn start_scene(&mut self) {
        self.bodies.retain(|b| !b.0);
        self.joints.retain(|b| !b.0);
        self.ops.retain(|b| !b.0);
        self.soft.retain(|b| !b.0);
        self.binds.retain(|b| !b.0);
        self.skins.retain(|b| !b.0);
        self.controllers.retain(|b| !b.0);
    }
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }
    pub fn body(&self, shape: &str) -> Option<&BodyDecl> {
        self.bodies.iter().map(|b| &b.1).find(|b| b.shape == shape)
    }
}

/// The live world of the running scene.
pub struct Physics {
    pub world: ph::World,
    pub(crate) ppm: f64,
    steps: usize,
    pub(crate) names: Vec<String>,
    pub(crate) ids: BTreeMap<String, usize>,
    com: BTreeMap<String, [f64; 2]>,
    kinds: BTreeMap<String, BodyKind>,
    angle: BTreeMap<String, f64>,
    last: BTreeMap<String, f64>,
    touching: BTreeSet<(usize, usize)>,
    /// Pairs whose contact began during the last frame, even if it already ended again.
    began: BTreeSet<(usize, usize)>,
    ctrl: BTreeMap<String, Ctrl>,
    pub(crate) rag: BTreeMap<String, crate::ragdoll::Rag>,
}

impl Physics {
    /// Before each step: find out whether each controlled body stands on something, then move it
    /// towards the velocity its input asks for and start a pending jump.
    fn steer(&mut self, dt: f64) {
        let names: Vec<String> = self.ctrl.keys().cloned().collect();
        for n in names {
            let id = self.ids[&n];
            let (c, v) = {
                let b = self.world.body(id);
                (b.c, b.v)
            };
            let st = &self.ctrl[&n];
            let reach = st.feet + 0.04;
            let grounded = [-0.9, 0.0, 0.9].iter().any(|k| {
                self.world
                    .raycast(v2(c.x + k * st.half_w, c.y), v2(0.0, 1.0), reach, Some(id))
                    .is_some_and(|h| h.normal.y < -0.5)
            });
            let st = self.ctrl.get_mut(&n).expect("controller");
            st.grounded = grounded;
            let d = st.decl.clone();
            let target = st.input * d.speed / self.ppm;
            let rate = d.accel / self.ppm * if grounded { 1.0 } else { d.air_control } * dt;
            let mut nv = v;
            nv.x += (target - v.x).clamp(-rate, rate);
            if std::mem::take(&mut st.jump_pending) && grounded {
                nv.y = -d.jump / self.ppm;
            }
            if nv != v {
                self.world.set_linear_velocity(id, nv);
            }
        }
    }
}

pub struct TickPoses {
    pub dynamic: Vec<(String, f64, f64, f64)>,
}

fn phys_err(code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, msg)
}

fn shoelace(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n)
        .map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1])
        .sum::<f64>()
        / 2.0
}

fn centroid(p: &[[f64; 2]]) -> ([f64; 2], f64) {
    let a = shoelace(p);
    if a.abs() < 1e-12 {
        return ([0.0, 0.0], 0.0);
    }
    let n = p.len();
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let (q, r) = (p[i], p[(i + 1) % n]);
        let w = q[0] * r[1] - r[0] * q[1];
        cx += (q[0] + r[0]) * w;
        cy += (q[1] + r[1]) * w;
    }
    ([cx / (6.0 * a), cy / (6.0 * a)], a.abs())
}

impl Interp {
    pub fn eval_world(&mut self) -> Res<()> {
        let Some(w) = self.program.world.clone() else {
            return Ok(());
        };
        let vars = crate::interp::Vars::new();
        let mut cfg = WorldCfg::default();
        if let Some(e) = &w.gravity {
            let v = self.eval(e, &vars, Mode::Build)?;
            cfg.gravity = self.point(&v, "`gravity`")?;
        }
        if let Some(e) = &w.scale {
            let v = self.eval(e, &vars, Mode::Build)?;
            cfg.scale = self.num(&v, "`scale`")?;
            if cfg.scale <= 0.0 {
                return Err(phys_err(
                    "E-PHY-001",
                    "the world `scale` (pixels per metre) must be positive",
                )
                .at(e.span));
            }
        }
        if let Some(e) = &w.substeps {
            let v = self.eval(e, &vars, Mode::Build)?;
            let n = self.int(&v, "`substeps`")?;
            if !(1..=64).contains(&n) {
                return Err(phys_err("E-PHY-001", "`substeps` must be between 1 and 64").at(e.span));
            }
            cfg.substeps = n as usize;
        }
        for (e, name, lo, hi) in [
            (&w.soft_substeps, "soft_substeps", 1, 128),
            (&w.soft_iterations, "soft_iterations", 1, 64),
            (&w.fluid_substeps, "fluid_substeps", 1, 64),
            (&w.fluid_iterations, "fluid_iterations", 1, 16),
        ] {
            if let Some(e) = e {
                let v = self.eval(e, &vars, Mode::Build)?;
                let n = self.int(&v, name)?;
                if !(lo..=hi).contains(&n) {
                    return Err(phys_err(
                        "E-PHY-001",
                        format!("`{name}` must be between {lo} and {hi}"),
                    )
                    .at(e.span));
                }
                match name {
                    "soft_substeps" => cfg.soft_substeps = n as usize,
                    "soft_iterations" => cfg.soft_iterations = n as usize,
                    "fluid_substeps" => cfg.fluid_substeps = n as usize,
                    _ => cfg.fluid_iterations = n as usize,
                }
            }
        }
        self.world_cfg = cfg;
        Ok(())
    }

    fn guard_build(&self, mode: Mode, what: &str) -> Res<()> {
        if mode == Mode::Run {
            return Err(phys_err("E-SHP-003", format!("`{what}` creates physics objects and is not allowed while the scene is running"))
                .help("bodies and joints are made at the top level or in `setup:`"));
        }
        Ok(())
    }

    fn is_scene_local(&self, mode: Mode) -> bool {
        mode == Mode::Setup
    }

    fn bool_arg(&self, v: &Value, what: &str) -> Res<bool> {
        match v {
            Value::Bool(b) => Ok(*b),
            other => Err(Diagnostic::new(
                "E-TYP-001",
                format!("{what} must be true or false, found {}", other.type_name()),
            )),
        }
    }

    // ---- shape methods ------------------------------------------------------------------

    pub fn body_method(&mut self, sn: &str, name: &str, args: Args, mode: Mode) -> Res<Value> {
        match name {
            "body" => {
                self.guard_build(mode, "body")?;
                if sn.contains('.') {
                    return Err(Diagnostic::new(
                        "E-RIG-003",
                        "a bone cannot be a physics body (yet)",
                    ));
                }
                if self.decls.binds.iter().any(|d| d.1.shape == sn)
                    || self.decls.skins.iter().any(|d| d.1.shape == sn)
                {
                    return Err(Diagnostic::new("E-RIG-003", format!("`{sn}` is driven by a rig; a shape is driven by a rig or by physics, not both (yet)")));
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
                            return Err(phys_err(
                                "E-PHY-001",
                                format!("unknown body kind `{other}`"),
                            )
                            .help("kinds: static, dynamic, kinematic"))
                        }
                    },
                    Some(other) => {
                        return Err(phys_err(
                            "E-PHY-001",
                            format!(
                                "`kind` must be static, dynamic or kinematic, found {}",
                                other.type_name()
                            ),
                        ))
                    }
                };
                let f = |k: &str, d: f64| -> Res<f64> {
                    m.get(k).map(|v| self.num(v, k)).unwrap_or(Ok(d))
                };
                let decl = BodyDecl {
                    shape: sn.to_string(),
                    kind,
                    density: f("density", 1.0)?,
                    friction: f("friction", 0.6)?,
                    restitution: f("restitution", 0.0)?,
                    linear_damping: f("linear_damping", 0.0)?,
                    angular_damping: f("angular_damping", 0.0)?,
                    gravity_scale: f("gravity_scale", 1.0)?,
                    fixed_rotation: m
                        .get("fixed_rotation")
                        .map(|v| self.bool_arg(v, "`fixed_rotation`"))
                        .unwrap_or(Ok(false))?,
                    sensor: m
                        .get("sensor")
                        .map(|v| self.bool_arg(v, "`sensor`"))
                        .unwrap_or(Ok(false))?,
                };
                if decl.density <= 0.0 || decl.friction < 0.0 || decl.restitution < 0.0 {
                    return Err(phys_err("E-PHY-001", "`density` must be positive, `friction` and `restitution` cannot be negative"));
                }
                if self.decls.body(sn).is_some() {
                    return Err(phys_err("E-PHY-001", format!("`{sn}` already has a body")));
                }
                if self.decls.soft.iter().any(|d| d.1.shape == sn) {
                    return Err(Diagnostic::new(
                        "E-PHY-003",
                        format!("`{sn}` is soft; a shape is either rigid or soft"),
                    ));
                }
                self.check_body_shape(&decl)?;
                let scene = self.is_scene_local(mode);
                self.decls.bodies.push((scene, decl));
                Ok(Value::None)
            }
            "apply_impulse" | "set_velocity" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let by = self.point(&m["by"], "a vector")?;
                if self.decls.body(sn).is_none() {
                    return Err(phys_err(
                        "E-PHY-002",
                        format!("`{sn}` has no body; call `{sn}.body()` first"),
                    ));
                }
                if mode == Mode::Run {
                    let Some(ph) = self.phys.as_mut() else {
                        return Ok(Value::None);
                    };
                    let (Some(&id), ppm) = (ph.ids.get(sn), ph.ppm) else {
                        return Ok(Value::None);
                    };
                    let v = v2(by[0] / ppm, by[1] / ppm);
                    if name == "set_velocity" {
                        ph.world.set_linear_velocity(id, v);
                    } else {
                        ph.world.apply_linear_impulse(id, v);
                    }
                } else {
                    let scene = self.is_scene_local(mode);
                    let op = if name == "set_velocity" {
                        InitOp::Velocity(sn.to_string(), by)
                    } else {
                        InitOp::Impulse(sn.to_string(), by)
                    };
                    self.decls.ops.push((scene, op));
                }
                Ok(Value::None)
            }
            "controller" => {
                self.guard_build(mode, "controller")?;
                let m = self.bind(name, args, &["speed", "accel", "jump", "air_control"], &[])?;
                let f = |k: &str, d: f64| -> Res<f64> {
                    m.get(k).map(|v| self.num(v, k)).unwrap_or(Ok(d))
                };
                let decl = ControllerDecl {
                    shape: sn.to_string(),
                    speed: f("speed", 200.0)?,
                    accel: f("accel", 1200.0)?,
                    jump: f("jump", 400.0)?,
                    air_control: f("air_control", 0.5)?,
                };
                let Some(body) = self.decls.body(sn) else {
                    return Err(phys_err(
                        "E-PHY-002",
                        format!("`{sn}` has no body; call `{sn}.body()` first"),
                    ));
                };
                if body.kind != BodyKind::Dynamic {
                    return Err(phys_err(
                        "E-PHY-005",
                        format!("`{sn}` is not a dynamic body, so it cannot have a controller"),
                    ));
                }
                if decl.speed < 0.0
                    || decl.accel <= 0.0
                    || decl.jump < 0.0
                    || !(0.0..=1.0).contains(&decl.air_control)
                {
                    return Err(phys_err("E-PHY-005", "`speed` and `jump` cannot be negative, `accel` must be positive and `air_control` between 0 and 1"));
                }
                if self.decls.controllers.iter().any(|c| c.1.shape == sn) {
                    return Err(phys_err(
                        "E-PHY-005",
                        format!("`{sn}` already has a controller"),
                    ));
                }
                let scene = self.is_scene_local(mode);
                self.decls.controllers.push((scene, decl));
                Ok(Value::None)
            }
            "move" | "jump" => {
                if mode != Mode::Run {
                    return Err(phys_err(
                        "E-PHY-005",
                        format!("`{name}` steers a character while a scene runs"),
                    ));
                }
                let input = if name == "move" {
                    let m = self.bind(name, args, &["direction"], &["direction"])?;
                    Some(self.num(&m["direction"], "`direction`")?.clamp(-1.0, 1.0))
                } else {
                    self.bind(name, args, &[], &[])?;
                    None
                };
                let Some(c) = self.phys.as_mut().and_then(|p| p.ctrl.get_mut(sn)) else {
                    return Err(phys_err(
                        "E-PHY-005",
                        format!("`{sn}` has no controller; call `{sn}.controller()` first"),
                    ));
                };
                match input {
                    Some(d) => c.input = d,
                    None => c.jump_pending = true,
                }
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
                    let t = self.phys.as_ref().is_some_and(|ph| {
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
                    Ok(Value::Bool(self.events.contains(&leave_key(sn, other))))
                }
            }
            _ => unreachable!("body_method is called for body methods only"),
        }
    }

    pub fn is_body_method(name: &str) -> bool {
        matches!(
            name,
            "body"
                | "apply_impulse"
                | "set_velocity"
                | "touches"
                | "leaves"
                | "controller"
                | "move"
                | "jump"
        )
    }

    fn check_body_shape(&self, d: &BodyDecl) -> Res<()> {
        let sh = &self.shapes[&d.shape];
        if sh.scale[0] != sh.scale[1] {
            return Err(phys_err(
                "E-PHY-001",
                format!(
                    "`{}` has a non-uniform scale, which a body cannot have",
                    d.shape
                ),
            )
            .help("use a uniform scale, or reshape the geometry"));
        }
        let faces = sh.live_ids(Kind::F);
        let has_area = faces.iter().any(|f| sh.face_area(*f).abs() > 1e-9);
        let has_edges = !sh.live_ids(Kind::E).is_empty();
        if !has_area && !(d.kind == BodyKind::Static && has_edges) {
            return Err(phys_err("E-PHY-001", format!("`{}` has no area, so it cannot be a {} body", d.shape, kind_name(d.kind))).help("only a static body can be made of bare edges; give a moving body at least one face"));
        }
        Ok(())
    }

    // ---- joints -------------------------------------------------------------------------

    pub fn is_joint_function(name: &str, args: &Args) -> bool {
        matches!(name, "revolute" | "weld" | "prismatic" | "rope" | "follow")
            || (name == "distance" && matches!(args.first(), Some((None, Value::Shape(_)))))
    }

    fn body_arg(&self, v: &Value, what: &str) -> Res<String> {
        match v {
            Value::Shape(s) if self.decls.body(s).is_some() => Ok(s.clone()),
            Value::Shape(s) => Err(phys_err("E-PHY-002", format!("{what} `{s}` has no body"))
                .help(format!("call `{s}.body()` before using it in a joint"))),
            other => Err(phys_err(
                "E-PHY-002",
                format!(
                    "{what} must be a shape with a body, found {}",
                    other.type_name()
                ),
            )),
        }
    }

    fn pair_arg(&self, v: &Value, what: &str) -> Res<(f64, f64)> {
        match v {
            Value::Tuple(t) if t.len() == 2 => Ok((
                self.num(&t[0], what)
                    .map_err(|e| e.with_code("E-PHY-002"))?,
                self.num(&t[1], what)
                    .map_err(|e| e.with_code("E-PHY-002"))?,
            )),
            other => Err(phys_err(
                "E-PHY-002",
                format!(
                    "{what} must be a pair such as `(2, 0.5)`, found {}",
                    other.type_name()
                ),
            )),
        }
    }

    fn angle_pair(&self, v: &Value, what: &str) -> Res<(f64, f64)> {
        match v {
            Value::Tuple(t) if t.len() == 2 => {
                let mut out = [0.0; 2];
                for (i, x) in t.iter().enumerate() {
                    out[i] = match x {
                        Value::Angle(a) => a * std::f64::consts::TAU,
                        _ => {
                            return Err(phys_err(
                                "E-PHY-002",
                                format!("{what} must be angles such as `(-60.deg, 60.deg)`"),
                            ))
                        }
                    };
                }
                Ok((out[0], out[1]))
            }
            _ => Err(phys_err(
                "E-PHY-002",
                format!("{what} must be a pair of angles such as `(-60.deg, 60.deg)`"),
            )),
        }
    }

    pub fn joint_function(&mut self, name: &str, args: Args, mode: Mode) -> Res<Value> {
        self.guard_build(mode, name)?;
        let pt = |s: &Interp, v: &Value, what: &str| {
            s.point(v, what).map_err(|e| e.with_code("E-PHY-002"))
        };
        let (a, b, collide, spec) = match name {
            "revolute" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "anchor", "limits", "motor", "collide"],
                    &["a", "b", "anchor"],
                )?;
                let anchor = pt(self, &m["anchor"], "`anchor`")?;
                let limits = m
                    .get("limits")
                    .map(|v| self.angle_pair(v, "`limits`"))
                    .transpose()?;
                if limits.is_some_and(|l| l.0 > l.1) {
                    return Err(phys_err(
                        "E-PHY-002",
                        "the lower limit is above the upper limit",
                    ));
                }
                let motor = match m.get("motor") {
                    None => None,
                    Some(Value::Tuple(t)) if t.len() == 2 => match &t[0] {
                        Value::Angle(a) => Some((a * std::f64::consts::TAU, self.num(&t[1], "the motor torque").map_err(|e| e.with_code("E-PHY-002"))?)),
                        _ => return Err(phys_err("E-PHY-002", "`motor` is (speed, torque) with the speed as an angle per second, such as `(90.deg, 50)`")),
                    },
                    Some(_) => return Err(phys_err("E-PHY-002", "`motor` is (speed, torque), such as `(90.deg, 50)`")),
                };
                (
                    m["a"].clone(),
                    m["b"].clone(),
                    self.collide_arg(&m)?,
                    JointSpec::Revolute {
                        anchor,
                        limits,
                        motor,
                    },
                )
            }
            "weld" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "anchor", "collide"],
                    &["a", "b", "anchor"],
                )?;
                let anchor = pt(self, &m["anchor"], "`anchor`")?;
                (
                    m["a"].clone(),
                    m["b"].clone(),
                    self.collide_arg(&m)?,
                    JointSpec::Weld { anchor },
                )
            }
            "prismatic" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "axis", "anchor", "limits", "collide"],
                    &["a", "b", "axis"],
                )?;
                let axis = pt(self, &m["axis"], "`axis`")?;
                if axis == [0.0, 0.0] {
                    return Err(phys_err("E-PHY-002", "`axis` cannot be (0, 0)"));
                }
                let anchor = m
                    .get("anchor")
                    .map(|v| pt(self, v, "`anchor`"))
                    .transpose()?;
                let limits = m
                    .get("limits")
                    .map(|v| self.pair_arg(v, "`limits`"))
                    .transpose()?;
                if limits.is_some_and(|l| l.0 > l.1) {
                    return Err(phys_err(
                        "E-PHY-002",
                        "the lower limit is above the upper limit",
                    ));
                }
                (
                    m["a"].clone(),
                    m["b"].clone(),
                    self.collide_arg(&m)?,
                    JointSpec::Prismatic {
                        anchor,
                        axis,
                        limits,
                    },
                )
            }
            "distance" => {
                let m = self.bind(
                    name,
                    args,
                    &[
                        "a", "b", "length", "spring", "anchor_a", "anchor_b", "collide",
                    ],
                    &["a", "b"],
                )?;
                let length = m
                    .get("length")
                    .map(|v| {
                        self.num(v, "`length`")
                            .map_err(|e| e.with_code("E-PHY-002"))
                    })
                    .transpose()?;
                if length.is_some_and(|l| l <= 0.0) {
                    return Err(phys_err("E-PHY-002", "`length` must be positive"));
                }
                let spring = m
                    .get("spring")
                    .map(|v| self.pair_arg(v, "`spring`"))
                    .transpose()?;
                let (aa, ab) = (
                    m.get("anchor_a")
                        .map(|v| pt(self, v, "`anchor_a`"))
                        .transpose()?,
                    m.get("anchor_b")
                        .map(|v| pt(self, v, "`anchor_b`"))
                        .transpose()?,
                );
                (
                    m["a"].clone(),
                    m["b"].clone(),
                    self.collide_arg(&m)?,
                    JointSpec::Distance {
                        length,
                        spring,
                        anchor_a: aa,
                        anchor_b: ab,
                    },
                )
            }
            "rope" => {
                let m = self.bind(
                    name,
                    args,
                    &["a", "b", "max", "anchor_a", "anchor_b", "collide"],
                    &["a", "b", "max"],
                )?;
                let max = self
                    .num(&m["max"], "`max`")
                    .map_err(|e| e.with_code("E-PHY-002"))?;
                if max <= 0.0 {
                    return Err(phys_err("E-PHY-002", "`max` must be positive"));
                }
                let (aa, ab) = (
                    m.get("anchor_a")
                        .map(|v| pt(self, v, "`anchor_a`"))
                        .transpose()?,
                    m.get("anchor_b")
                        .map(|v| pt(self, v, "`anchor_b`"))
                        .transpose()?,
                );
                (
                    m["a"].clone(),
                    m["b"].clone(),
                    self.collide_arg(&m)?,
                    JointSpec::Rope {
                        max,
                        anchor_a: aa,
                        anchor_b: ab,
                    },
                )
            }
            "follow" => {
                let m = self.bind(
                    name,
                    args,
                    &["body", "target", "force", "spring"],
                    &["body", "target"],
                )?;
                let target = pt(self, &m["target"], "`target`")?;
                let force = m
                    .get("force")
                    .map(|v| self.num(v, "`force`").map_err(|e| e.with_code("E-PHY-002")))
                    .transpose()?
                    .unwrap_or(1000.0);
                let spring = m
                    .get("spring")
                    .map(|v| self.pair_arg(v, "`spring`"))
                    .transpose()?
                    .unwrap_or((5.0, 0.7));
                (
                    m["body"].clone(),
                    m["body"].clone(),
                    false,
                    JointSpec::Follow {
                        target,
                        force,
                        spring,
                    },
                )
            }
            _ => unreachable!(),
        };
        let (a, b) = (
            self.body_arg(&a, "the first body")?,
            self.body_arg(&b, "the second body")?,
        );
        if a == b && !matches!(spec, JointSpec::Follow { .. }) {
            return Err(phys_err("E-PHY-002", "a joint needs two different bodies"));
        }
        let scene = self.is_scene_local(mode);
        self.decls.joints.push((
            scene,
            JointDecl {
                a,
                b,
                collide,
                spec,
            },
        ));
        Ok(Value::Joint(self.decls.joints.len() - 1))
    }

    fn collide_arg(&self, m: &std::collections::HashMap<&'static str, Value>) -> Res<bool> {
        m.get("collide")
            .map(|v| self.bool_arg(v, "`collide`"))
            .unwrap_or(Ok(false))
    }

    pub fn joint_method(&mut self, j: usize, name: &str, args: Args, mode: Mode) -> Res<Value> {
        match name {
            "set_target" => {
                let m = self.bind(name, args, &["to"], &["to"])?;
                let to = self.point(&m["to"], "the target")?;
                let Some((_, d)) = self.decls.joints.get_mut(j) else {
                    return Ok(Value::None);
                };
                let JointSpec::Follow { target, .. } = &mut d.spec else {
                    return Err(phys_err("E-PHY-002", "only a `follow` joint has a target"));
                };
                *target = to;
                if mode == Mode::Run {
                    if let Some(ph) = self.phys.as_mut() {
                        ph.world
                            .set_joint_target(j, v2(to[0] / ph.ppm, to[1] / ph.ppm));
                    }
                }
                Ok(Value::None)
            }
            _ => Err(
                Diagnostic::new("E-NAM-003", format!("a joint has no operation `{name}`"))
                    .help("joints have `set_target` (follow joints)"),
            ),
        }
    }

    // ---- member reads -------------------------------------------------------------------

    pub fn phys_attr(&self, sn: &str, name: &str) -> Option<Res<Value>> {
        if let Some(r) = self.soft_attr(sn, name) {
            return Some(r);
        }
        if name == "grounded" && self.decls.controllers.iter().any(|c| c.1.shape == sn) {
            return Some(Ok(Value::Bool(
                self.phys
                    .as_ref()
                    .and_then(|p| p.ctrl.get(sn))
                    .is_some_and(|c| c.grounded),
            )));
        }
        if !matches!(
            name,
            "velocity" | "speed" | "awake" | "mass" | "angular_velocity"
        ) {
            return None;
        }
        if self.decls.body(sn).is_none() {
            return Some(Err(phys_err(
                "E-PHY-002",
                format!("`{sn}` has no body, so it has no `{name}`"),
            )
            .help(format!("call `{sn}.body()` first"))));
        }
        let live = self
            .phys
            .as_ref()
            .and_then(|ph| ph.ids.get(sn).map(|id| (ph, *id)));
        Some(Ok(match (name, live) {
            ("velocity", Some((ph, id))) => {
                let v = ph.world.body(id).v;
                Value::Tuple(vec![Value::Num(v.x * ph.ppm), Value::Num(v.y * ph.ppm)])
            }
            ("velocity", None) => Value::Tuple(vec![Value::Num(0.0), Value::Num(0.0)]),
            ("speed", Some((ph, id))) => Value::Num(ph.world.body(id).v.length() * ph.ppm),
            ("speed", None) => Value::Num(0.0),
            ("awake", Some((ph, id))) => Value::Bool(ph.world.body(id).awake),
            ("awake", None) => Value::Bool(true),
            ("mass", Some((ph, id))) => Value::Num(ph.world.body(id).mass),
            ("mass", None) => Value::Num(0.0),
            ("angular_velocity", Some((ph, id))) => {
                Value::Angle(ph.world.body(id).w / std::f64::consts::TAU)
            }
            _ => Value::Angle(0.0),
        }))
    }

    // ---- scene start and end ------------------------------------------------------------

    /// Build the world for the scene that is starting, from the shapes' current poses.
    pub fn build_physics(&mut self) -> Res<()> {
        if self.decls.is_empty() && !self.has_ragdolls() {
            self.phys = None;
            return Ok(());
        }
        let cfg = self.world_cfg.clone();
        let ppm = cfg.scale;
        let mut world = ph::World::new(v2(cfg.gravity[0], cfg.gravity[1]));
        let mut ph = Physics {
            world: ph::World::new(Vec2::ZERO),
            ppm,
            steps: cfg.substeps,
            names: vec![],
            ids: BTreeMap::new(),
            com: BTreeMap::new(),
            kinds: BTreeMap::new(),
            angle: BTreeMap::new(),
            last: BTreeMap::new(),
            touching: BTreeSet::new(),
            began: BTreeSet::new(),
            ctrl: BTreeMap::new(),
            rag: BTreeMap::new(),
        };
        let decls: Vec<BodyDecl> = self.decls.bodies.iter().map(|b| b.1.clone()).collect();
        for d in &decls {
            self.check_body_shape(d)?;
            let ctrl = self
                .decls
                .controllers
                .iter()
                .find(|c| c.1.shape == d.shape)
                .map(|c| c.1.clone());
            let mut d = d.clone();
            if ctrl.is_some() {
                d.fixed_rotation = true;
                d.friction = 0.0;
            }
            let d = &d;
            let sh = self.shapes[&d.shape].clone();
            let s = sh.scale[0];
            let faces: Vec<Vec<[f64; 2]>> = sh
                .live_ids(Kind::F)
                .into_iter()
                .map(|f| sh.face_polygon(f))
                .filter(|p| p.len() >= 3 && shoelace(p).abs() > 1e-9)
                .collect();
            let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
            for f in &faces {
                let (c, a) = centroid(f);
                area += a;
                cx += c[0] * a;
                cy += c[1] * a;
            }
            let com = if area > 0.0 {
                [cx / area, cy / area]
            } else {
                sh.bbox_center()
            };
            let t = sh.transform();
            let origin = t.apply(com);
            let angle = t.rotation * std::f64::consts::TAU;
            let id = world.create_body(&ph::BodyDef {
                kind: d.kind,
                position: v2(origin[0] / ppm, origin[1] / ppm),
                angle,
                linear_damping: d.linear_damping,
                angular_damping: d.angular_damping,
                gravity_scale: d.gravity_scale,
                fixed_rotation: d.fixed_rotation,
                ..ph::BodyDef::default()
            });
            let local = |p: [f64; 2]| v2((p[0] - com[0]) * s / ppm, (p[1] - com[1]) * s / ppm);
            let fixture = |shape: ph::Shape| ph::FixtureDef {
                shape,
                density: d.density,
                friction: d.friction,
                restitution: d.restitution,
                sensor: d.sensor,
            };
            if faces.is_empty() {
                for e in sh.live_ids(Kind::E) {
                    let ed = &sh.edges[&e];
                    world.add_fixture(
                        id,
                        &fixture(ph::Shape::edge(local(sh.pos(ed.a)), local(sh.pos(ed.b)))),
                    );
                }
            }
            for f in &faces {
                let pts: Vec<Vec2> = f.iter().map(|p| local(*p)).collect();
                let pieces = ph::decompose(&pts).map_err(|e| {
                    phys_err("E-PHY-001", format!("`{}` cannot be a body: {e}", d.shape))
                })?;
                for piece in pieces {
                    let poly = ph::Polygon::new(piece).ok_or_else(|| {
                        phys_err(
                            "E-PHY-001",
                            format!("`{}` has a face that could not be made convex", d.shape),
                        )
                    })?;
                    world.add_fixture(id, &fixture(ph::Shape::Polygon(poly)));
                }
            }
            if let Some(decl) = ctrl {
                let ys: Vec<f64> = faces.iter().flatten().map(|p| p[1]).collect();
                let xs: Vec<f64> = faces.iter().flatten().map(|p| p[0]).collect();
                let hi = ys.iter().cloned().fold(f64::MIN, f64::max);
                let (lo_x, hi_x) = (
                    xs.iter().cloned().fold(f64::MAX, f64::min),
                    xs.iter().cloned().fold(f64::MIN, f64::max),
                );
                ph.ctrl.insert(
                    d.shape.clone(),
                    Ctrl {
                        decl,
                        input: 0.0,
                        jump_pending: false,
                        grounded: false,
                        half_w: (hi_x - lo_x) * s / ppm / 2.0,
                        feet: (hi - com[1]) * s / ppm,
                    },
                );
            }
            ph.names.push(d.shape.clone());
            ph.ids.insert(d.shape.clone(), id);
            ph.com.insert(d.shape.clone(), com);
            ph.kinds.insert(d.shape.clone(), d.kind);
            ph.angle.insert(d.shape.clone(), angle);
            ph.last.insert(d.shape.clone(), world.body(id).q.angle());
            // the shape now rotates about its centre of mass; the pose is unchanged
            let shape = self.shapes.get_mut(&d.shape).expect("declared shape");
            shape.pivot = Some(com);
            shape.position = [origin[0] - com[0], origin[1] - com[1]];
        }
        let joints: Vec<JointDecl> = self.decls.joints.iter().map(|j| j.1.clone()).collect();
        let at = |ph: &Physics, world: &ph::World, name: &str| {
            let b = world.body(ph.ids[name]);
            b.p
        };
        for j in &joints {
            let (ia, ib) = (ph.ids[&j.a], ph.ids[&j.b]);
            let w = |p: [f64; 2]| v2(p[0] / ppm, p[1] / ppm);
            let def = match &j.spec {
                JointSpec::Revolute {
                    anchor,
                    limits,
                    motor,
                } => {
                    let mut d = JointDef::revolute(ia, ib, w(*anchor));
                    if let Some((lo, hi)) = limits {
                        d = d.limits(*lo, *hi);
                    }
                    if let Some((speed, torque)) = motor {
                        d = d.motor(*speed, torque / ppm / ppm);
                    }
                    d
                }
                JointSpec::Weld { anchor } => JointDef::weld(ia, ib, w(*anchor)),
                JointSpec::Prismatic {
                    anchor,
                    axis,
                    limits,
                } => {
                    let a = anchor.map(w).unwrap_or_else(|| at(&ph, &world, &j.a));
                    let mut d = JointDef::prismatic(ia, ib, a, v2(axis[0], axis[1]).normalized());
                    if let Some((lo, hi)) = limits {
                        d = d.limits(lo / ppm, hi / ppm);
                    }
                    d
                }
                JointSpec::Distance {
                    length,
                    spring,
                    anchor_a,
                    anchor_b,
                } => {
                    let (pa, pb) = (
                        anchor_a.map(w).unwrap_or_else(|| at(&ph, &world, &j.a)),
                        anchor_b.map(w).unwrap_or_else(|| at(&ph, &world, &j.b)),
                    );
                    let mut d = JointDef::distance(ia, ib, pa, pb);
                    if let Some(l) = length {
                        d = d.length(l / ppm);
                    }
                    if let Some((f, z)) = spring {
                        d = d.spring(*f, *z);
                    }
                    d
                }
                JointSpec::Rope {
                    max,
                    anchor_a,
                    anchor_b,
                } => {
                    let (pa, pb) = (
                        anchor_a.map(w).unwrap_or_else(|| at(&ph, &world, &j.a)),
                        anchor_b.map(w).unwrap_or_else(|| at(&ph, &world, &j.b)),
                    );
                    JointDef::rope(ia, ib, pa, pb, max / ppm)
                }
                JointSpec::Follow {
                    target,
                    force,
                    spring,
                } => JointDef::target(ia, at(&ph, &world, &j.a), w(*target))
                    .force(force / ppm)
                    .spring(spring.0, spring.1),
            };
            world.create_joint(&def.collide_connected(j.collide));
        }
        self.build_ragdolls(&mut world, &mut ph);
        let ops: Vec<InitOp> = self.decls.ops.iter().map(|o| o.1.clone()).collect();
        for op in &ops {
            match op {
                InitOp::Velocity(n, v) => {
                    world.set_linear_velocity(ph.ids[n], v2(v[0] / ppm, v[1] / ppm))
                }
                InitOp::Impulse(n, v) => {
                    world.apply_linear_impulse(ph.ids[n], v2(v[0] / ppm, v[1] / ppm))
                }
            }
        }
        for (a, b) in world.overlapping_pairs(0.01) {
            self.diagnostics.push(
                phys_err(
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
        self.phys = Some(ph);
        Ok(())
    }

    /// Names of kinematic bodies (their pose is read from the other layers before each step).
    pub fn kinematic_bodies(&self) -> Vec<String> {
        self.phys
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

    /// First part of a physics tick: kinematic bodies are given the velocity that reaches the pose
    /// the other layers resolved (`kin`: x offset, y offset, turns). Nothing moves yet.
    pub fn physics_prepare(&mut self, tick: i64, kin: &BTreeMap<String, [f64; 3]>) {
        let fps = self.fps.to_f64();
        let Some(ph) = self.phys.as_mut() else { return };
        ph.began.clear();
        if tick > 0 {
            ph.steer(1.0 / fps);
        }
        if tick > 0 {
            for (name, pose) in kin {
                let (id, com) = (ph.ids[name], ph.com[name]);
                let p = v2((pose[0] + com[0]) / ph.ppm, (pose[1] + com[1]) / ph.ppm);
                ph.world
                    .move_kinematic(id, p, pose[2] * std::f64::consts::TAU, 1.0 / fps);
            }
        }
    }

    /// Step the world for this frame (used when no soft body drives the stepping).
    pub fn physics_advance(&mut self, tick: i64) {
        let fps = self.fps.to_f64();
        let Some(ph) = self.phys.as_mut() else { return };
        if tick > 0 {
            for _ in 0..ph.steps {
                ph.world.step(1.0 / fps / ph.steps as f64);
            }
        }
    }

    /// Last part of a physics tick: turn the frame's contact events into `touches`/`leaves` state and
    /// read the dynamic bodies' poses.
    pub fn physics_collect(&mut self) -> TickPoses {
        let mut leaves: Vec<String> = vec![];
        let Some(ph) = self.phys.as_mut() else {
            return TickPoses { dynamic: vec![] };
        };
        for e in ph.world.take_events() {
            if e.begin {
                ph.touching.insert((e.a, e.b));
                ph.began.insert((e.a, e.b));
            } else {
                ph.touching.remove(&(e.a, e.b));
                leaves.push(leave_key(&ph.names[e.a], &ph.names[e.b]));
            }
        }
        let tau = std::f64::consts::TAU;
        let mut dynamic = vec![];
        let names = ph.names.clone();
        for n in names {
            if ph.kinds.get(&n) != Some(&BodyKind::Dynamic) {
                continue;
            }
            let b = ph.world.body(ph.ids[&n]);
            let (px, py, a) = (b.p.x * ph.ppm, b.p.y * ph.ppm, b.q.angle());
            let last = ph.last.insert(n.clone(), a).unwrap_or(a);
            let mut d = a - last;
            d -= tau * (d / tau).round();
            let acc = ph.angle.get_mut(&n).expect("angle tracked");
            *acc += d;
            let com = ph.com[&n];
            dynamic.push((n, px - com[0], py - com[1], *acc / tau));
        }
        for l in leaves {
            self.events.insert(l);
        }
        TickPoses { dynamic }
    }

    /// The pose a dynamic rigid body had when the frame began (what a pin to it follows).
    pub fn dynamic_pose(&self, name: &str, ahead: f64) -> Option<Transform> {
        let ph = self.phys.as_ref()?;
        if ph.kinds.get(name) != Some(&BodyKind::Dynamic) {
            return None;
        }
        let b = ph.world.body(ph.ids[name]);
        let com = ph.com[name];
        let sh = &self.shapes[name];
        // extrapolated by its velocity over `ahead` seconds, so a pin lands where the body will be
        Some(Transform {
            position: [
                (b.p.x + b.v.x * ahead) * ph.ppm - com[0],
                (b.p.y + b.v.y * ahead) * ph.ppm - com[1],
            ],
            scale: sh.scale,
            rotation: (ph.angle[name] + b.w * ahead) / std::f64::consts::TAU,
            pivot: com,
        })
    }

    pub fn physics_snapshot(&self) -> Option<J> {
        let ph = self.phys.as_ref()?;
        let px = |v: Vec2| json!([v.x * ph.ppm, v.y * ph.ppm]);
        let bodies: Vec<J> = ph.names.iter().map(|n| json!({"shape": n, "kind": kind_name(ph.world.body(ph.ids[n]).kind), "awake": ph.world.body(ph.ids[n]).awake})).collect();
        let contacts: Vec<J> = ph.world.contact_infos().iter().map(|c| json!({"a": ph.names[c.a], "b": ph.names[c.b], "point": px(c.point), "normal": [c.normal.x, c.normal.y], "impulse": c.normal_impulse})).collect();
        let joints: Vec<J> = (0..ph.world.joint_count())
            .map(|i| {
                let (a, b) = ph.world.joint_anchors(i);
                json!({"kind": format!("{:?}", ph.world.joint(i).kind).to_lowercase(), "a": px(a), "b": px(b)})
            })
            .collect();
        Some(json!({"bodies": bodies, "contacts": contacts, "joints": joints}))
    }

    /// Write the final poses back as rest values and drop the world (the next scene starts from them).
    pub fn end_physics(&mut self) {
        let Some(ph) = self.phys.take() else { return };
        for n in &ph.names {
            if ph.kinds.get(n) != Some(&BodyKind::Dynamic) {
                continue;
            }
            let b = ph.world.body(ph.ids[n]);
            let com = ph.com[n];
            if let Some(sh) = self.shapes.get_mut(n) {
                sh.position = [b.p.x * ph.ppm - com[0], b.p.y * ph.ppm - com[1]];
                sh.rotation = ph.angle[n] / std::f64::consts::TAU;
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

fn leave_key(a: &str, b: &str) -> String {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    format!("leaves:{x}:{y}")
}
