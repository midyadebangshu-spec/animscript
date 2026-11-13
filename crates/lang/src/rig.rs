//! Armatures in the language: `armature name:` blocks of bones and IK chains, shapes `bind`-ed to a
//! bone or `skin`-ned over several, and the per-tick solve. A bone and an IK handle are hidden
//! pseudo-shapes named `armature.bone` (a line from head to tip; not in the snapshot's shape list),
//! so every action, keyframe, modifier, easing and scene commit that works on a shape works on a
//! bone: its `rotation` is the pose rotation relative to rest (turns), the root bone's `position` is
//! an offset of the whole skeleton, and an IK handle's `position` is its target relative to the rest tip
//! and its `opacity` is the IK weight. The solve writes layer-2 overrides for bound shapes.
use crate::ast::ArmItem;
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::*;
use animscript_physics as ph;
use ph::{v2, Vec2};
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, BTreeSet};

type Args = Vec<(Option<String>, Value)>;
const TAU: f64 = std::f64::consts::TAU;

pub struct IkDef {
    pub name: String,
    pub chain: Vec<usize>,
    pub bend: ph::Bend,
    pub rest_tip: [f64; 2],
}

/// The `ragdoll:` line of an armature: how its bones look to physics.
#[derive(Clone, Copy, Debug)]
pub struct RagCfg {
    /// Capsule radius in px (a bone's own `radius:` overrides it).
    pub radius: f64,
    pub density: f64,
    pub friction: f64,
    pub restitution: f64,
}

pub struct RigDef {
    pub name: String,
    pub sk: ph::Skeleton,
    pub bones: Vec<String>,
    pub ik: Vec<IkDef>,
    pub ragdoll: Option<RagCfg>,
    /// Per-bone `radius:` overrides.
    pub radius: Vec<Option<f64>>,
}

#[derive(Clone, Debug)]
pub struct BindDecl {
    pub shape: String,
    pub arm: String,
    pub bone: usize,
}

#[derive(Clone, Debug)]
pub struct SkinDecl {
    pub shape: String,
    pub arm: String,
    pub blend: f64,
    pub bones: Option<Vec<usize>>,
    pub paints: Vec<(Vec<u32>, usize, f64)>,
}

struct SkinSim {
    shape: String,
    arm: String,
    ids: Vec<u32>,
    rest: Vec<Vec2>,
    weights: Vec<Vec<(usize, f64)>>,
}

#[derive(Default)]
pub struct RigSim {
    binds: Vec<(BindDecl, Vec2, f64)>, // with the bone's rest head and direction
    skins: Vec<SkinSim>,
    warned: BTreeSet<(String, usize)>,
}

/// One armature's pose for a tick: pose rotations (radians, relative to rest), world poses, and the
/// IK part of the snapshot.
pub struct RigPose {
    pub rots: Vec<f64>,
    pub pose: Vec<ph::BonePose>,
    ik: Vec<J>,
}

pub struct RigOut {
    pub overrides: Vec<(String, PropId, f64)>,
    pub snapshot: J,
}

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-001", msg)
}
fn e2(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-002", msg)
}
fn e3(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-003", msg)
}

/// The name an expression stands for, read without evaluating it (`parent: torso`).
fn name_of(e: &crate::ast::Expr) -> Option<String> {
    match &e.kind {
        crate::ast::ExprKind::Name(n) => Some(n.clone()),
        crate::ast::ExprKind::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn e4(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-004", msg)
}

fn lerp_angle(a: f64, b: f64, t: f64) -> f64 {
    a + ph::wrap_angle(b - a) * t
}

impl Interp {
    /// Build the armatures declared in the program (at the start of the build phase).
    pub fn eval_armatures(&mut self) -> Res<()> {
        let decls = self.program.armatures.clone();
        let vars = crate::interp::Vars::new(); // armatures come first, so no names are defined yet
        for d in &decls {
            if self.rigs.contains_key(&d.name) {
                return Err(
                    e1(format!("there is already an armature named `{}`", d.name)).at(d.span),
                );
            }
            let mut bones: Vec<String> = vec![];
            let mut defs: Vec<ph::BoneDef> = vec![];
            let mut radius: Vec<Option<f64>> = vec![];
            for it in d.items.iter().filter(|i| i.kind == "bone") {
                if bones.contains(&it.name)
                    || d.items.iter().any(|o| o.kind == "ik" && o.name == it.name)
                {
                    return Err(e1(format!(
                        "`{}` is defined twice in armature `{}`",
                        it.name, d.name
                    ))
                    .at(it.span));
                }
                let (def, r) = self.eval_bone(d.name.as_str(), it, &bones, &vars)?;
                bones.push(it.name.clone());
                defs.push(def);
                radius.push(r);
            }
            if bones.is_empty() {
                return Err(e1(format!("armature `{}` has no bones", d.name))
                    .at(d.span)
                    .help("add `bone name: at: (x, y), length: 80, angle: -90.deg`"));
            }
            let sk = ph::Skeleton::new(defs);
            let mut iks: Vec<IkDef> = vec![];
            let rest = sk.pose(&vec![0.0; bones.len()], Vec2::ZERO);
            for it in d.items.iter().filter(|i| i.kind == "ik") {
                if iks.iter().any(|k| k.name == it.name) {
                    return Err(e1(format!(
                        "`{}` is defined twice in armature `{}`",
                        it.name, d.name
                    ))
                    .at(it.span));
                }
                iks.push(self.eval_ik(it, &bones, &sk, &rest)?);
            }
            let mut ragdoll = None;
            for it in d.items.iter().filter(|i| i.kind == "ragdoll") {
                if ragdoll.is_some() {
                    return Err(
                        e1(format!("armature `{}` has two `ragdoll:` lines", d.name)).at(it.span),
                    );
                }
                ragdoll = Some(self.eval_ragdoll(it, &vars)?);
            }
            // the hidden pseudo-shapes
            for (i, b) in bones.iter().enumerate() {
                let (h, t) = (rest[i].head, sk.tip(&rest, i));
                let mut sh = line([h.x, h.y], [t.x, t.y]);
                sh.name = format!("{}.{b}", d.name);
                sh.visible = false;
                self.shapes.insert(sh.name.clone(), sh);
            }
            for k in &iks {
                let mut sh = line(k.rest_tip, [k.rest_tip[0] + 1.0, k.rest_tip[1]]);
                sh.name = format!("{}.{}", d.name, k.name);
                sh.visible = false;
                sh.opacity = 0.0; // the IK weight: off until a `reach`
                self.shapes.insert(sh.name.clone(), sh);
            }
            self.rigs.insert(
                d.name.clone(),
                RigDef {
                    name: d.name.clone(),
                    sk,
                    bones,
                    ik: iks,
                    ragdoll,
                    radius,
                },
            );
        }
        Ok(())
    }

    fn eval_ragdoll(&mut self, it: &ArmItem, vars: &crate::interp::Vars) -> Res<RagCfg> {
        let mut cfg = RagCfg {
            radius: 8.0,
            density: 1.0,
            friction: 0.6,
            restitution: 0.0,
        };
        for (key, expr, span) in &it.attrs {
            let v = self.eval(expr, vars, Mode::Build)?;
            let n = self.num(&v, key).map_err(|e| e4(e.message).at(*span))?;
            match key.as_str() {
                "radius" if n > 0.0 => cfg.radius = n,
                "density" if n > 0.0 => cfg.density = n,
                "friction" if n >= 0.0 => cfg.friction = n,
                "restitution" if n >= 0.0 => cfg.restitution = n,
                "radius" | "density" | "friction" | "restitution" => {
                    return Err(e4(format!("`{key}` of a ragdoll is out of range"))
                        .at(*span)
                        .help("`radius` and `density` must be positive, `friction` and `restitution` cannot be negative"))
                }
                other => {
                    return Err(e1(format!("a ragdoll line has no setting `{other}`"))
                        .at(*span)
                        .help("settings: radius, density, friction, restitution"))
                }
            }
        }
        Ok(cfg)
    }

    fn eval_bone(
        &mut self,
        arm: &str,
        it: &ArmItem,
        bones: &[String],
        vars: &crate::interp::Vars,
    ) -> Res<(ph::BoneDef, Option<f64>)> {
        let mut radius = None;
        let mut def = ph::BoneDef {
            parent: None,
            at: 1.0,
            length: 0.0,
            rest_angle: 0.0,
            head: Vec2::ZERO,
            limits: None,
        };
        let (mut at_point, mut at_frac, mut length) = (None, None, None);
        for (key, expr, span) in &it.attrs {
            if key == "parent" {
                let p = name_of(expr)
                    .ok_or_else(|| e1("`parent` must be the name of a bone").at(*span))?;
                let i = bones.iter().position(|b| *b == p).ok_or_else(|| {
                    e1(format!(
                        "`{p}` is not a bone of `{arm}` declared before `{}`",
                        it.name
                    ))
                    .at(*span)
                    .help("list a parent before its children")
                })?;
                def.parent = Some(i);
                continue;
            }
            let v = self.eval(expr, vars, Mode::Build)?;
            match key.as_str() {
                "at" => match &v {
                    Value::Tuple(_) => {
                        at_point = Some(
                            self.point(&v, "`at`")
                                .map_err(|e| e1(e.message).at(*span))?,
                        )
                    }
                    other => {
                        at_frac = Some(
                            self.num(other, "`at`")
                                .map_err(|e| e1(e.message).at(*span))?,
                        )
                    }
                },
                "length" => {
                    length = Some(
                        self.num(&v, "`length`")
                            .map_err(|e| e1(e.message).at(*span))?,
                    )
                }
                "angle" => {
                    def.rest_angle = self
                        .angle(&v, "`angle`")
                        .map_err(|e| e1(e.message).at(*span))?
                        * TAU
                }
                "limits" => match &v {
                    Value::Tuple(t)
                        if t.len() == 2 && t.iter().all(|x| matches!(x, Value::Angle(_))) =>
                    {
                        let (lo, hi) = (
                            self.angle(&t[0], "limit")? * TAU,
                            self.angle(&t[1], "limit")? * TAU,
                        );
                        if lo > hi {
                            return Err(e2(format!(
                                "the lower limit of `{}` is above the upper limit",
                                it.name
                            ))
                            .at(*span));
                        }
                        def.limits = Some((lo, hi));
                    }
                    _ => {
                        return Err(e2(
                            "`limits` must be a pair of angles such as `(-30.deg, 90.deg)`",
                        )
                        .at(*span))
                    }
                },
                "radius" => {
                    let r = self
                        .num(&v, "`radius`")
                        .map_err(|e| e1(e.message).at(*span))?;
                    if r <= 0.0 {
                        return Err(e4("a bone's `radius` must be positive").at(*span));
                    }
                    radius = Some(r);
                }
                other => {
                    return Err(e1(format!("a bone has no setting `{other}`"))
                        .at(*span)
                        .help("settings: parent, at, length, angle, limits, radius"))
                }
            }
        }
        let len =
            length.ok_or_else(|| e1(format!("bone `{}` needs a `length`", it.name)).at(it.span))?;
        if len <= 0.0 {
            return Err(e1(format!("bone `{}` needs a positive `length`", it.name)).at(it.span));
        }
        def.length = len;
        match def.parent {
            None => {
                let p = at_point.ok_or_else(|| {
                    e1(format!("the root bone `{}` needs `at: (x, y)`", it.name)).at(it.span)
                })?;
                if at_frac.is_some() {
                    return Err(
                        e1(format!("`at` of the root bone `{}` is a point", it.name)).at(it.span),
                    );
                }
                def.head = v2(p[0], p[1]);
            }
            Some(_) => {
                if at_point.is_some() {
                    return Err(e1(format!(
                        "`at` of `{}` is a fraction along its parent (0 to 1), not a point",
                        it.name
                    ))
                    .at(it.span));
                }
                def.at = at_frac.unwrap_or(1.0);
                if !(0.0..=1.0).contains(&def.at) {
                    return Err(
                        e1(format!("`at` of `{}` must be between 0 and 1", it.name)).at(it.span)
                    );
                }
            }
        }
        Ok((def, radius))
    }

    fn eval_ik(
        &mut self,
        it: &ArmItem,
        bones: &[String],
        sk: &ph::Skeleton,
        rest: &[ph::BonePose],
    ) -> Res<IkDef> {
        let mut chain: Vec<usize> = vec![];
        let mut bend = ph::Bend::Positive;
        for (key, expr, span) in &it.attrs {
            match key.as_str() {
                "chain" => {
                    let crate::ast::ExprKind::List(items) = &expr.kind else {
                        return Err(e2(
                            "`chain` must be a list of bone names, such as `[upper, lower]`",
                        )
                        .at(*span));
                    };
                    for x in items {
                        let n =
                            name_of(x).ok_or_else(|| e2("`chain` lists bone names").at(*span))?;
                        chain.push(bones.iter().position(|b| *b == n).ok_or_else(|| {
                            e2(format!("`{n}` is not a bone of this armature")).at(*span)
                        })?);
                    }
                }
                "bend" => {
                    bend =
                        match name_of(expr).as_deref() {
                            Some("positive") => ph::Bend::Positive,
                            Some("negative") => ph::Bend::Negative,
                            _ => return Err(e2(
                                "`bend` is `positive` (elbow on the clockwise side) or `negative`",
                            )
                            .at(*span)),
                        }
                }
                other => {
                    return Err(e1(format!("an ik line has no setting `{other}`"))
                        .at(*span)
                        .help("settings: chain, bend"))
                }
            }
        }
        if chain.len() < 2 {
            return Err(e2(format!(
                "ik `{}` needs a chain of at least two bones",
                it.name
            ))
            .at(it.span));
        }
        for w in chain.windows(2) {
            let b = &sk.bones[w[1]];
            if b.parent != Some(w[0]) || (b.at - 1.0).abs() > 1e-12 {
                return Err(e2(format!("the chain of `{}` is not connected: `{}` must be a child of `{}` hanging from its tip", it.name, bones[w[1]], bones[w[0]])).at(it.span));
            }
        }
        let tip = sk.tip(rest, *chain.last().expect("non-empty"));
        Ok(IkDef {
            name: it.name.clone(),
            chain,
            bend,
            rest_tip: [tip.x, tip.y],
        })
    }

    pub fn has_rigs(&self) -> bool {
        !self.rigs.is_empty()
    }

    /// `(bone keys, ik keys)` of every armature: the pseudo-shape names whose properties the solve reads.
    pub fn rig_keys(&self) -> (Vec<String>, Vec<String>) {
        let (mut bones, mut iks) = (vec![], vec![]);
        for r in self.rigs.values() {
            bones.extend(r.bones.iter().map(|b| format!("{}.{b}", r.name)));
            iks.extend(r.ik.iter().map(|k| format!("{}.{}", r.name, k.name)));
        }
        (bones, iks)
    }

    pub fn is_rig_method(name: &str) -> bool {
        matches!(name, "bind" | "skin" | "set_weight" | "reach" | "release")
    }

    pub fn armature_member(&self, arm: &str, name: &str) -> Res<Value> {
        let r = &self.rigs[arm];
        if r.bones.iter().any(|b| b == name) || r.ik.iter().any(|k| k.name == name) {
            return Ok(Value::Shape(format!("{arm}.{name}")));
        }
        if name == "ragdolled" && r.ragdoll.is_some() {
            return Ok(Value::Bool(self.is_ragdolled(arm)));
        }
        let mut all: Vec<&str> = r.bones.iter().map(String::as_str).collect();
        all.extend(r.ik.iter().map(|k| k.name.as_str()));
        Err(Diagnostic::new(
            "E-NAM-003",
            format!("armature `{arm}` has no bone or ik `{name}`"),
        )
        .help(format!("it has: {}", all.join(", "))))
    }

    /// `head`, `tip` (world px) and `angle` of a bone pseudo-shape: the last solved pose, or the rest pose.
    pub fn bone_attr(&self, key: &str, name: &str) -> Option<Value> {
        if matches!(name, "target" | "weight") {
            let (arm, k) = self.ik_of(key)?;
            let (t, w) = self
                .ik_live
                .get(key)
                .copied()
                .unwrap_or((self.rigs[&arm].ik[k].rest_tip, 0.0));
            return Some(if name == "target" {
                Value::Tuple(vec![Value::Num(t[0]), Value::Num(t[1])])
            } else {
                Value::Num(w)
            });
        }
        if !matches!(name, "head" | "tip" | "angle") {
            return None;
        }
        let (arm, bi) = self.bone_of(key)?;
        let (head, tip, angle) = match self.rig_live.get(key) {
            Some(v) => *v,
            None => {
                let sk = &self.rigs[&arm].sk;
                let rest = sk.pose(&vec![0.0; self.rigs[&arm].bones.len()], Vec2::ZERO);
                let (h, t) = (rest[bi].head, sk.tip(&rest, bi));
                ([h.x, h.y], [t.x, t.y], rest[bi].angle / TAU)
            }
        };
        let pt = |p: P| Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1])]);
        Some(match name {
            "head" => pt(head),
            "tip" => pt(tip),
            _ => Value::Angle(angle),
        })
    }

    /// The head (world px) and direction (turns) of a bone: the last solved pose, or the rest pose.
    pub fn bone_frame(&self, key: &str) -> Option<([f64; 2], f64)> {
        let (arm, bi) = self.bone_of(key)?;
        Some(match self.rig_live.get(key) {
            Some(v) => (v.0, v.2),
            None => {
                let sk = &self.rigs[&arm].sk;
                let rest = sk.pose(&vec![0.0; self.rigs[&arm].bones.len()], Vec2::ZERO);
                ([rest[bi].head.x, rest[bi].head.y], rest[bi].angle / TAU)
            }
        })
    }

    /// `(armature, bone index)` when `key` names a bone pseudo-shape.
    fn bone_of(&self, key: &str) -> Option<(String, usize)> {
        let (arm, bone) = key.split_once('.')?;
        let r = self.rigs.get(arm)?;
        r.bones
            .iter()
            .position(|b| b == bone)
            .map(|i| (arm.to_string(), i))
    }

    fn ik_of(&self, key: &str) -> Option<(String, usize)> {
        let (arm, ik) = key.split_once('.')?;
        let r = self.rigs.get(arm)?;
        r.ik.iter()
            .position(|k| k.name == ik)
            .map(|i| (arm.to_string(), i))
    }

    pub fn rig_method(
        &mut self,
        sn: &str,
        name: &str,
        args: Args,
        mode: Mode,
        span: Span,
    ) -> Res<Value> {
        match name {
            "reach" | "release" => {
                let Some((arm, k)) = self.ik_of(sn) else {
                    return Err(e2(format!(
                        "`{name}` works on an ik handle such as `armature.hand`, not on `{sn}`"
                    )));
                };
                let params: &[&str] = if name == "reach" {
                    &["to", "over", "ease"]
                } else {
                    &["over", "ease"]
                };
                let m = self.bind(
                    name,
                    args,
                    params,
                    if name == "reach" { &["to"] } else { &[] },
                )?;
                let comps = if name == "reach" {
                    let to = self
                        .point(&m["to"], "the target")
                        .map_err(|e| e2(e.message))?;
                    let rest = self.rigs[&arm].ik[k].rest_tip;
                    vec![
                        (PropId::PosX, Tgt::To(to[0] - rest[0])),
                        (PropId::PosY, Tgt::To(to[1] - rest[1])),
                        (PropId::Opacity, Tgt::To(1.0)),
                    ]
                } else {
                    vec![(PropId::Opacity, Tgt::To(0.0))]
                };
                if self.over_frames(&m, mode, span)?.is_none() {
                    // instantly: set the position offset and the weight now
                    for (prop, t) in comps {
                        let Tgt::To(v) = t else { continue };
                        match prop {
                            PropId::Opacity => self.apply_or_stage(
                                crate::interp::Staged::Style {
                                    shape: sn.to_string(),
                                    prop: "opacity".into(),
                                    value: Value::Num(v),
                                },
                                mode,
                            )?,
                            p => self.apply_or_stage(
                                crate::interp::Staged::Transform {
                                    shape: sn.to_string(),
                                    prop: p,
                                    value: v,
                                    by: false,
                                },
                                mode,
                            )?,
                        }
                    }
                    return Ok(Value::None);
                }
                self.shape_transform(sn, name, comps, &m, mode, span)
            }
            "bind" | "skin" | "set_weight" => {
                if mode == Mode::Run {
                    return Err(Diagnostic::new("E-SHP-003", format!("`{name}` attaches art to a rig and is not allowed while the scene is running")).help("bind and skin at the top level or in `setup:`"));
                }
                if sn.contains('.') {
                    return Err(e3("a bone or ik handle cannot be bound or skinned"));
                }
                let scene = mode == Mode::Setup;
                match name {
                    "bind" => {
                        let m = self.bind(name, args, &["bone"], &["bone"])?;
                        let Value::Shape(key) = &m["bone"] else {
                            return Err(e3(format!(
                                "`bind` needs a bone such as `armature.torso`, found {}",
                                m["bone"].type_name()
                            )));
                        };
                        let Some((arm, bone)) = self.bone_of(key) else {
                            return Err(e3(format!("`{key}` is not a bone")));
                        };
                        self.check_rig_free(sn)?;
                        self.decls.binds.push((
                            scene,
                            BindDecl {
                                shape: sn.to_string(),
                                arm,
                                bone,
                            },
                        ));
                    }
                    "skin" => {
                        let m =
                            self.bind(name, args, &["armature", "blend", "bones"], &["armature"])?;
                        let Value::Armature(arm) = &m["armature"] else {
                            return Err(e3(format!(
                                "`skin` needs an armature, found {}",
                                m["armature"].type_name()
                            )));
                        };
                        let blend = m
                            .get("blend")
                            .map(|v| self.num(v, "`blend`").map_err(|e| e2(e.message)))
                            .transpose()?
                            .unwrap_or(20.0);
                        if blend <= 0.0 {
                            return Err(e2("`blend` must be positive (the falloff length in px)"));
                        }
                        let mut only = None;
                        if let Some(Value::List(items)) = m.get("bones") {
                            let mut v = vec![];
                            for x in items {
                                let Value::Shape(k) = x else {
                                    return Err(e2("`bones` lists bones of the armature"));
                                };
                                match self.bone_of(k) {
                                    Some((a, i)) if a == *arm => v.push(i),
                                    _ => return Err(e2(format!("`{k}` is not a bone of `{arm}`"))),
                                }
                            }
                            only = Some(v);
                        }
                        self.check_rig_free(sn)?;
                        self.decls.skins.push((
                            scene,
                            SkinDecl {
                                shape: sn.to_string(),
                                arm: arm.clone(),
                                blend,
                                bones: only,
                                paints: vec![],
                            },
                        ));
                    }
                    _ => {
                        let m = self.bind(
                            name,
                            args,
                            &["vertices", "bone", "weight"],
                            &["vertices", "bone", "weight"],
                        )?;
                        let ids: Vec<u32> = match &m["vertices"] {
                            Value::Elem(s, r) if s == sn && r.kind == Kind::V => vec![r.id],
                            Value::List(items) => items
                                .iter()
                                .filter_map(|x| {
                                    if let Value::Elem(s, r) = x {
                                        (s == sn && r.kind == Kind::V).then_some(r.id)
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                            _ => return Err(e2(format!("`set_weight` takes vertices of `{sn}`"))),
                        };
                        let Value::Shape(key) = &m["bone"] else {
                            return Err(e2("`set_weight` needs a bone"));
                        };
                        let w = self
                            .num(&m["weight"], "`weight`")
                            .map_err(|e| e2(e.message))?;
                        if !(0.0..=1.0).contains(&w) {
                            return Err(e2("a weight is between 0 and 1"));
                        }
                        let bone = self.bone_of(key);
                        let Some(d) = self.decls.skins.iter_mut().find(|d| d.1.shape == sn) else {
                            return Err(e3(format!(
                                "`{sn}` is not skinned; call `{sn}.skin(armature)` first"
                            )));
                        };
                        match bone {
                            Some((a, i)) if a == d.1.arm => d.1.paints.push((ids, i, w)),
                            _ => {
                                return Err(e2(format!(
                                    "`{key}` is not a bone of the armature `{}`",
                                    d.1.arm
                                )))
                            }
                        }
                    }
                }
                Ok(Value::None)
            }
            _ => unreachable!(),
        }
    }

    fn check_rig_free(&self, sn: &str) -> Res<()> {
        if self.decls.body(sn).is_some() || self.decls.soft.iter().any(|d| d.1.shape == sn) {
            return Err(e3(format!("`{sn}` is a simulated body; a shape is driven by a rig or by physics, not both (yet)")));
        }
        if self.decls.binds.iter().any(|d| d.1.shape == sn)
            || self.decls.skins.iter().any(|d| d.1.shape == sn)
        {
            return Err(e3(format!("`{sn}` is already bound or skinned")));
        }
        Ok(())
    }

    /// Fix each bound or skinned shape to the armature's rest pose (after `setup:`).
    pub fn build_rigs(&mut self) -> Res<()> {
        self.rig_sim = None;
        if self.decls.binds.is_empty() && self.decls.skins.is_empty() {
            return Ok(());
        }
        let mut sim = RigSim::default();
        let binds: Vec<BindDecl> = self.decls.binds.iter().map(|b| b.1.clone()).collect();
        for b in binds {
            let r = &self.rigs[&b.arm];
            let rest = r.sk.pose(&vec![0.0; r.bones.len()], Vec2::ZERO);
            let (head, angle) = (rest[b.bone].head, rest[b.bone].angle);
            let sh = self.shapes.get_mut(&b.shape).expect("declared shape");
            sh.bake_transform();
            sh.pivot = Some([head.x, head.y]);
            sim.binds.push((b, head, angle));
        }
        let skins: Vec<SkinDecl> = self.decls.skins.iter().map(|b| b.1.clone()).collect();
        for d in skins {
            let r = &self.rigs[&d.arm];
            let rest = r.sk.pose(&vec![0.0; r.bones.len()], Vec2::ZERO);
            let sh = self.shapes.get_mut(&d.shape).expect("declared shape");
            sh.bake_transform();
            let ids = sh.live_ids(Kind::V);
            let pts: Vec<Vec2> = ids
                .iter()
                .map(|i| v2(sh.pos(*i)[0], sh.pos(*i)[1]))
                .collect();
            let bones: Vec<usize> = d
                .bones
                .clone()
                .unwrap_or_else(|| (0..r.bones.len()).collect());
            let mut weights = ph::auto_weights(&pts, &r.sk, &rest, &bones, d.blend);
            for (vids, bone, w) in &d.paints {
                for vid in vids {
                    let Some(k) = ids.iter().position(|i| i == vid) else {
                        continue;
                    };
                    let others: f64 = weights[k]
                        .iter()
                        .filter(|x| x.0 != *bone)
                        .map(|x| x.1)
                        .sum();
                    let mut nw: Vec<(usize, f64)> = weights[k]
                        .iter()
                        .filter(|x| x.0 != *bone)
                        .map(|x| {
                            (
                                x.0,
                                if others > 0.0 {
                                    x.1 / others * (1.0 - w)
                                } else {
                                    0.0
                                },
                            )
                        })
                        .collect();
                    nw.push((*bone, *w));
                    nw.retain(|x| x.1 > 0.0);
                    nw.sort_by_key(|x| x.0);
                    weights[k] = nw;
                }
            }
            sim.skins.push(SkinSim {
                shape: d.shape.clone(),
                arm: d.arm.clone(),
                ids,
                rest: pts,
                weights,
            });
        }
        self.rig_sim = Some(sim);
        Ok(())
    }

    /// Solve the animated pose of every armature for this tick (FK, limits, IK). `bones`:
    /// `[rotation (turns), x offset, y offset]` of each bone pseudo-shape as the layer stack resolved it;
    /// `iks`: `[x offset, y offset, weight]`. Physics may replace a pose before `rig_finish`.
    pub fn rig_solve(
        &mut self,
        bones: &BTreeMap<String, [f64; 3]>,
        iks: &BTreeMap<String, [f64; 3]>,
    ) -> BTreeMap<String, RigPose> {
        let mut out = BTreeMap::new();
        let names: Vec<String> = self.rigs.keys().cloned().collect();
        for name in names {
            let (sk, bone_names, ik_defs) = {
                let r = &self.rigs[&name];
                (
                    r.sk.clone(),
                    r.bones.clone(),
                    r.ik.iter()
                        .map(|k| (k.name.clone(), k.chain.clone(), k.bend, k.rest_tip))
                        .collect::<Vec<_>>(),
                )
            };
            let n = sk.bones.len();
            let mut rots: Vec<f64> = bone_names
                .iter()
                .map(|b| {
                    bones
                        .get(&format!("{name}.{b}"))
                        .map_or(0.0, |v| v[0] * TAU)
                })
                .collect();
            let off = bone_names
                .first()
                .and_then(|b| bones.get(&format!("{name}.{b}")))
                .map_or(Vec2::ZERO, |v| v2(v[1], v[2]));
            let before = rots.clone();
            if sk.clamp_limits(&mut rots) {
                for i in 0..n {
                    if rots[i] != before[i]
                        && self
                            .rig_sim
                            .as_mut()
                            .is_none_or(|s| s.warned.insert((name.clone(), i)))
                    {
                        self.diagnostics.push(Diagnostic::new("W-RIG-001", format!("bone `{}.{}` was asked to rotate beyond its limits and was clamped", name, bone_names[i])).help("widen `limits` or change the animation"));
                    }
                }
            }
            let mut ik_snap = vec![];
            for (iname, chain, bend, rest_tip) in &ik_defs {
                let v = iks
                    .get(&format!("{name}.{iname}"))
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0]);
                let w = v[2].clamp(0.0, 1.0);
                let target = v2(rest_tip[0] + v[0], rest_tip[1] + v[1]);
                if w > 0.0 {
                    let mut solved = rots.clone();
                    ph::solve_ik(&sk, &mut solved, off, chain, target, *bend, 24);
                    for &b in chain {
                        rots[b] = lerp_angle(rots[b], solved[b], w);
                    }
                }
                self.ik_live
                    .insert(format!("{name}.{iname}"), ([target.x, target.y], w));
                ik_snap.push(json!({"name": iname, "target": [target.x, target.y], "weight": w}));
            }
            let pose = sk.pose(&rots, off);
            out.insert(
                name,
                RigPose {
                    rots,
                    pose,
                    ik: ik_snap,
                },
            );
        }
        out
    }

    /// Turn the final poses into the snapshot and the overrides for bound and skinned art.
    pub fn rig_finish(&mut self, poses: &BTreeMap<String, RigPose>) -> RigOut {
        let mut overrides = vec![];
        let mut snap = vec![];
        for (name, rp) in poses {
            let (sk, bone_names) = {
                let r = &self.rigs[name];
                (r.sk.clone(), r.bones.clone())
            };
            let n = sk.bones.len();
            let pose = &rp.pose;
            let rest = sk.pose(&vec![0.0; n], Vec2::ZERO);
            let bone_snap: Vec<J> = (0..n)
                .map(|i| {
                    let tip = sk.tip(pose, i);
                    self.rig_live.insert(
                        format!("{name}.{}", bone_names[i]),
                        ([pose[i].head.x, pose[i].head.y], [tip.x, tip.y], pose[i].angle / TAU),
                    );
                    json!({"name": bone_names[i], "head": [pose[i].head.x, pose[i].head.y], "tip": [tip.x, tip.y], "angle": pose[i].angle / TAU})
                })
                .collect();
            snap.push(json!({"name": name, "bones": bone_snap, "ik": rp.ik}));
            if let Some(sim) = &self.rig_sim {
                for (b, head, angle) in sim.binds.iter().filter(|b| b.0.arm == *name) {
                    overrides.push((b.shape.clone(), PropId::PosX, pose[b.bone].head.x - head.x));
                    overrides.push((b.shape.clone(), PropId::PosY, pose[b.bone].head.y - head.y));
                    overrides.push((
                        b.shape.clone(),
                        PropId::Rot,
                        (pose[b.bone].angle - angle) / TAU,
                    ));
                }
                for s in sim.skins.iter().filter(|s| s.arm == *name) {
                    for (k, id) in s.ids.iter().enumerate() {
                        let p = ph::skin_point(&sk, &rest, pose, &s.weights[k], s.rest[k]);
                        overrides.push((s.shape.clone(), PropId::Vx(*id), p.x));
                        overrides.push((s.shape.clone(), PropId::Vy(*id), p.y));
                    }
                }
            }
        }
        RigOut {
            overrides,
            snapshot: J::Array(snap),
        }
    }
}
