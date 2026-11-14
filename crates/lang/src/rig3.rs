//! 3D armatures: `armature name:` blocks whose bones have a 3D `dir:`. Bones are hidden pseudo-shapes
//! `armature.bone` just like the 2D ones: a bone's `rotation.x`/`.y`/`.z` (turns, applied y, then x,
//! then z, like a mesh's) are its pose relative to rest, the root bone's `position` offsets the whole
//! skeleton, and an IK handle's `position` is its target relative to the rest tip (opacity is the
//! weight). Meshes are `bind`-ed to a bone or `skin`-ned over several (linear blend skinning); the
//! solve writes the skinned vertices. Skinned meshes have their own transform baked at scene start.
use crate::ast::ArmItem;
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::*;
use animscript_physics3 as p3;
use p3::{v3, BoneDef3, Quat, Skeleton3, V3};
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, BTreeSet};

type Args = Vec<(Option<String>, Value)>;
type P3 = [f64; 3];
const TAU: f64 = std::f64::consts::TAU;

pub struct Ik3Def {
    pub name: String,
    pub chain: Vec<usize>,
    /// The point the middle joint bends toward.
    pub pole: Option<P3>,
    pub rest_tip: P3,
}

pub struct Rig3Def {
    pub name: String,
    pub sk: Skeleton3,
    pub bones: Vec<String>,
    pub ik: Vec<Ik3Def>,
    pub radius: Vec<Option<f64>>,
    pub ragdoll: Option<crate::rig::RagCfg>,
}

#[derive(Clone, Debug)]
pub struct Skin3Decl {
    pub shape: String,
    pub arm: String,
    pub blend: f64,
    pub bones: Option<Vec<usize>>,
    pub paints: Vec<(Vec<usize>, usize, f64)>,
}

struct Skin3Sim {
    shape: String,
    arm: String,
    rest: Vec<V3>,
    weights: Vec<Vec<(usize, f64)>>,
}

#[derive(Default)]
pub struct Rig3Sim {
    skins: Vec<Skin3Sim>,
    warned: BTreeSet<(String, usize)>,
}

pub struct Rig3Pose {
    pub deltas: Vec<Quat>,
    pub pose: Vec<p3::BonePose>,
    ik: Vec<J>,
}

pub struct Rig3Out {
    pub verts: Vec<(String, BTreeMap<u32, [f64; 3]>)>,
    pub snapshot: J,
}

fn e5(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-005", msg)
}
fn e6(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-006", msg)
}
fn e7(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-007", msg)
}

fn name_of(e: &crate::ast::Expr) -> Option<String> {
    match &e.kind {
        crate::ast::ExprKind::Name(n) => Some(n.clone()),
        crate::ast::ExprKind::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn vp(a: P3) -> V3 {
    v3(a[0], a[1], a[2])
}
fn pv(v: V3) -> P3 {
    [v.x, v.y, v.z]
}

impl Interp {
    pub fn is_armature3(&self, n: &str) -> bool {
        self.rigs3.contains_key(n)
    }

    /// An armature is 3D when any of its bones has a `dir:`.
    pub fn armature_is_3d(it: &[ArmItem]) -> bool {
        it.iter()
            .any(|i| i.kind == "bone" && i.attrs.iter().any(|a| a.0 == "dir"))
    }

    fn triple_of(&self, v: &Value, what: &str) -> Res<P3> {
        match v {
            Value::Tuple(t) if t.len() == 3 => Ok([
                self.num(&t[0], what)?,
                self.num(&t[1], what)?,
                self.num(&t[2], what)?,
            ]),
            other => Err(e5(format!(
                "{what} must be a 3D point like `(10, 20, 30)`, found {}",
                other.type_name()
            ))),
        }
    }

    fn rig3_angle_pair(&self, v: &Value, what: &str) -> Res<(f64, f64)> {
        match v {
            Value::Tuple(t) if t.len() == 2 && t.iter().all(|x| matches!(x, Value::Angle(_))) => {
                let (lo, hi) = (
                    self.angle(&t[0], what)? * TAU,
                    self.angle(&t[1], what)? * TAU,
                );
                if lo > hi {
                    return Err(e5(format!("the lower {what} is above the upper one")));
                }
                Ok((lo, hi))
            }
            _ => Err(e5(format!(
                "{what} must be a pair of angles such as `(-30.deg, 90.deg)`"
            ))),
        }
    }

    pub fn eval_armature3(&mut self, d: &crate::ast::ArmatureDecl) -> Res<()> {
        let vars = self.armature_constants();
        if self.rigs.contains_key(&d.name) || self.rigs3.contains_key(&d.name) {
            return Err(e5(format!("there is already an armature named `{}`", d.name)).at(d.span));
        }
        let mut names: Vec<String> = vec![];
        let mut defs: Vec<BoneDef3> = vec![];
        let mut radius: Vec<Option<f64>> = vec![];
        for it in d.items.iter().filter(|i| i.kind == "bone") {
            if names.contains(&it.name)
                || d.items.iter().any(|o| o.kind == "ik" && o.name == it.name)
            {
                return Err(e5(format!(
                    "`{}` is defined twice in armature `{}`",
                    it.name, d.name
                ))
                .at(it.span));
            }
            let (def, r) = self.eval_bone3(&d.name, it, &names, &defs, &vars)?;
            names.push(it.name.clone());
            defs.push(def);
            radius.push(r);
        }
        let sk = Skeleton3::new(defs);
        let rest = sk.pose(V3::ZERO, &[]);
        let mut iks: Vec<Ik3Def> = vec![];
        for it in d.items.iter().filter(|i| i.kind == "ik") {
            if iks.iter().any(|k| k.name == it.name) {
                return Err(e5(format!(
                    "`{}` is defined twice in armature `{}`",
                    it.name, d.name
                ))
                .at(it.span));
            }
            iks.push(self.eval_ik3(it, &names, &sk, &rest, &vars)?);
        }
        let mut ragdoll = None;
        for it in d.items.iter().filter(|i| i.kind == "ragdoll") {
            if ragdoll.is_some() {
                return Err(
                    e5(format!("armature `{}` has two `ragdoll:` lines", d.name)).at(it.span),
                );
            }
            ragdoll = Some(self.eval_ragdoll3(it, &vars)?);
        }
        for (i, b) in names.iter().enumerate() {
            let (h, t) = (rest[i].head, rest[i].tip);
            let mut sh = line([h.x, h.y], [t.x, t.y]);
            sh.name = format!("{}.{b}", d.name);
            sh.visible = false;
            self.shapes.insert(sh.name.clone(), sh);
        }
        for k in &iks {
            let mut sh = line(
                [k.rest_tip[0], k.rest_tip[1]],
                [k.rest_tip[0] + 1.0, k.rest_tip[1]],
            );
            sh.name = format!("{}.{}", d.name, k.name);
            sh.visible = false;
            sh.opacity = 0.0;
            self.shapes.insert(sh.name.clone(), sh);
        }
        self.rigs3.insert(
            d.name.clone(),
            Rig3Def {
                name: d.name.clone(),
                sk,
                bones: names,
                ik: iks,
                radius,
                ragdoll,
            },
        );
        Ok(())
    }

    fn eval_ragdoll3(
        &mut self,
        it: &ArmItem,
        vars: &crate::interp::Vars,
    ) -> Res<crate::rig::RagCfg> {
        let mut cfg = crate::rig::RagCfg {
            radius: 8.0,
            density: 1.0,
            friction: 0.6,
            restitution: 0.0,
        };
        for (key, expr, span) in &it.attrs {
            let v = self.eval(expr, vars, Mode::Build)?;
            let n = self.num(&v, key).map_err(|e| e5(e.message).at(*span))?;
            match key.as_str() {
                "radius" if n > 0.0 => cfg.radius = n,
                "density" if n > 0.0 => cfg.density = n,
                "friction" if n >= 0.0 => cfg.friction = n,
                "restitution" if n >= 0.0 => cfg.restitution = n,
                "radius" | "density" | "friction" | "restitution" => {
                    return Err(e5(format!("`{key}` of a ragdoll is out of range")).at(*span))
                }
                other => {
                    return Err(e5(format!("a ragdoll line has no setting `{other}`"))
                        .at(*span)
                        .help("settings: radius, density, friction, restitution"))
                }
            }
        }
        Ok(cfg)
    }

    fn eval_bone3(
        &mut self,
        arm: &str,
        it: &ArmItem,
        names: &[String],
        defs: &[BoneDef3],
        vars: &crate::interp::Vars,
    ) -> Res<(BoneDef3, Option<f64>)> {
        let (mut parent, mut at_point, mut at_frac, mut dir, mut length) =
            (None, None, None, None, None);
        let (mut axis, mut limits, mut swing, mut twist, mut radius) =
            (None, None, None, None, None);
        for (key, expr, span) in &it.attrs {
            if key == "parent" {
                let p = name_of(expr)
                    .ok_or_else(|| e5("`parent` must be the name of a bone").at(*span))?;
                parent = Some(names.iter().position(|b| *b == p).ok_or_else(|| {
                    e5(format!(
                        "`{p}` is not a bone of `{arm}` declared before `{}`",
                        it.name
                    ))
                    .at(*span)
                    .help("list a parent before its children")
                })?);
                continue;
            }
            let v = self.eval(expr, vars, Mode::Build)?;
            let at = |e: Diagnostic| e.at(*span);
            match key.as_str() {
                "at" => match &v {
                    Value::Tuple(_) => at_point = Some(self.triple_of(&v, "`at`").map_err(at)?),
                    o => at_frac = Some(self.num(o, "`at`").map_err(|e| e5(e.message).at(*span))?),
                },
                "dir" => dir = Some(self.triple_of(&v, "`dir`").map_err(at)?),
                "length" => {
                    length = Some(
                        self.num(&v, "`length`")
                            .map_err(|e| e5(e.message).at(*span))?,
                    )
                }
                "axis" => axis = Some(self.triple_of(&v, "`axis`").map_err(at)?),
                "limits" => limits = Some(self.rig3_angle_pair(&v, "`limits`").map_err(at)?),
                "twist" => twist = Some(self.rig3_angle_pair(&v, "`twist`").map_err(at)?),
                "swing" => {
                    let a = self
                        .angle(&v, "`swing`")
                        .map_err(|e| e5(e.message).at(*span))?
                        * TAU;
                    if !(0.0..=std::f64::consts::PI).contains(&a) {
                        return Err(
                            e5("`swing` is a cone half-angle between 0 and 180 degrees").at(*span)
                        );
                    }
                    swing = Some(a);
                }
                "radius" => {
                    let r = self
                        .num(&v, "`radius`")
                        .map_err(|e| e5(e.message).at(*span))?;
                    if r <= 0.0 {
                        return Err(e5("a bone's `radius` must be positive").at(*span));
                    }
                    radius = Some(r);
                }
                other => {
                    return Err(e5(format!("a 3D bone has no setting `{other}`"))
                        .at(*span)
                        .help(
                            "settings: parent, at, dir, length, axis, limits, swing, twist, radius",
                        ))
                }
            }
        }
        let len =
            length.ok_or_else(|| e5(format!("bone `{}` needs a `length`", it.name)).at(it.span))?;
        if len <= 0.0 {
            return Err(e5(format!("bone `{}` needs a positive `length`", it.name)).at(it.span));
        }
        let (head, dirv) = match parent {
            None => {
                let p = at_point.ok_or_else(|| {
                    e5(format!("the root bone `{}` needs `at: (x, y, z)`", it.name)).at(it.span)
                })?;
                let d = dir.ok_or_else(|| {
                    e5(format!(
                        "the root bone `{}` needs `dir: (x, y, z)`",
                        it.name
                    ))
                    .at(it.span)
                })?;
                (vp(p), vp(d))
            }
            Some(pi) => {
                if at_point.is_some() {
                    return Err(e5(format!(
                        "`at` of `{}` is a fraction along its parent (0 to 1), not a point",
                        it.name
                    ))
                    .at(it.span));
                }
                let f = at_frac.unwrap_or(1.0);
                if !(0.0..=1.0).contains(&f) {
                    return Err(
                        e5(format!("`at` of `{}` must be between 0 and 1", it.name)).at(it.span)
                    );
                }
                let pb = &defs[pi];
                (pb.head + pb.dir * (pb.length * f), dir.map_or(pb.dir, vp))
            }
        };
        if dirv.len() < 1e-12 {
            return Err(
                e5(format!("`dir` of `{}` cannot be the zero vector", it.name)).at(it.span),
            );
        }
        let mut b = BoneDef3::new(parent, head, dirv, len);
        match (axis, limits) {
            (Some(a), Some((lo, hi))) => {
                if vp(a).len() < 1e-12 {
                    return Err(e5("a hinge `axis` cannot be the zero vector").at(it.span));
                }
                b.hinge = Some((vp(a).normalized(), lo, hi));
            }
            (None, None) => {}
            _ => {
                return Err(e5(format!("`{}`: a hinge needs both `axis` and `limits`", it.name))
                    .at(it.span)
                    .help("`axis: (0, 0, 1), limits: (0.deg, 140.deg)`; for a ball joint use `swing` and `twist`"))
            }
        }
        if b.hinge.is_some() && (swing.is_some() || twist.is_some()) {
            return Err(e5(format!(
                "`{}` is a hinge (`axis`, `limits`) or a ball (`swing`, `twist`), not both",
                it.name
            ))
            .at(it.span));
        }
        b.swing = swing;
        b.twist = twist;
        Ok((b, radius))
    }

    fn eval_ik3(
        &mut self,
        it: &ArmItem,
        names: &[String],
        sk: &Skeleton3,
        rest: &[p3::BonePose],
        vars: &crate::interp::Vars,
    ) -> Res<Ik3Def> {
        let mut chain: Vec<usize> = vec![];
        let mut pole = None;
        for (key, expr, span) in &it.attrs {
            match key.as_str() {
                "chain" => {
                    let crate::ast::ExprKind::List(items) = &expr.kind else {
                        return Err(e6(
                            "`chain` must be a list of bone names, such as `[upper, lower]`",
                        )
                        .at(*span));
                    };
                    for x in items {
                        let n =
                            name_of(x).ok_or_else(|| e6("`chain` lists bone names").at(*span))?;
                        chain.push(names.iter().position(|b| *b == n).ok_or_else(|| {
                            e6(format!("`{n}` is not a bone of this armature")).at(*span)
                        })?);
                    }
                }
                "bend" => {
                    let v = self.eval(expr, vars, Mode::Build)?;
                    pole = Some(
                        self.triple_of(&v, "`bend`")
                            .map_err(|e| e6(e.message).at(*span))?,
                    );
                }
                other => {
                    return Err(e6(format!("an ik line has no setting `{other}`"))
                        .at(*span)
                        .help("settings: chain, bend (a 3D point the middle joint bends toward)"))
                }
            }
        }
        if chain.len() < 2 {
            return Err(e6(format!(
                "ik `{}` needs a chain of at least two bones",
                it.name
            ))
            .at(it.span));
        }
        for w in chain.windows(2) {
            let b = &sk.bones[w[1]];
            let pb = &sk.bones[w[0]];
            if b.parent != Some(w[0]) || (b.head - pb.tip()).len() > 1e-9 {
                return Err(e6(format!("the chain of `{}` is not connected: `{}` must be a child of `{}` hanging from its tip", it.name, names[w[1]], names[w[0]])).at(it.span));
            }
        }
        let tip = rest[*chain.last().expect("non-empty")].tip;
        Ok(Ik3Def {
            name: it.name.clone(),
            chain,
            pole,
            rest_tip: pv(tip),
        })
    }

    pub fn has_rigs3(&self) -> bool {
        !self.rigs3.is_empty()
    }

    pub fn rig3_keys(&self) -> (Vec<String>, Vec<String>) {
        let (mut bones, mut iks) = (vec![], vec![]);
        for r in self.rigs3.values() {
            bones.extend(r.bones.iter().map(|b| format!("{}.{b}", r.name)));
            iks.extend(r.ik.iter().map(|k| format!("{}.{}", r.name, k.name)));
        }
        (bones, iks)
    }

    pub fn armature_member3(&self, arm: &str, name: &str) -> Res<Value> {
        let r = &self.rigs3[arm];
        if r.bones.iter().any(|b| b == name) || r.ik.iter().any(|k| k.name == name) {
            return Ok(Value::Shape(format!("{arm}.{name}")));
        }
        if name == "ragdolled" && r.ragdoll.is_some() {
            return Ok(Value::Bool(self.is_ragdolled3(arm)));
        }
        let mut all: Vec<&str> = r.bones.iter().map(String::as_str).collect();
        all.extend(r.ik.iter().map(|k| k.name.as_str()));
        Err(Diagnostic::new(
            "E-NAM-003",
            format!("armature `{arm}` has no bone or ik `{name}`"),
        )
        .help(format!("it has: {}", all.join(", "))))
    }

    pub(crate) fn bone3_of(&self, key: &str) -> Option<(String, usize)> {
        let (arm, bone) = key.split_once('.')?;
        let r = self.rigs3.get(arm)?;
        r.bones
            .iter()
            .position(|b| b == bone)
            .map(|i| (arm.to_string(), i))
    }

    fn ik3_of(&self, key: &str) -> Option<(String, usize)> {
        let (arm, ik) = key.split_once('.')?;
        let r = self.rigs3.get(arm)?;
        r.ik.iter()
            .position(|k| k.name == ik)
            .map(|i| (arm.to_string(), i))
    }

    /// `head`, `tip` of a 3D bone and `target`, `weight` of an ik handle (world px).
    pub fn bone3_attr(&self, key: &str, name: &str) -> Option<Value> {
        let tup = |p: P3| Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1]), Value::Num(p[2])]);
        if matches!(name, "target" | "weight") {
            let (arm, k) = self.ik3_of(key)?;
            let (t, w) = self
                .ik3_live
                .get(key)
                .copied()
                .unwrap_or((self.rigs3[&arm].ik[k].rest_tip, 0.0));
            return Some(if name == "target" {
                tup(t)
            } else {
                Value::Num(w)
            });
        }
        if !matches!(name, "head" | "tip") {
            return None;
        }
        let (arm, bi) = self.bone3_of(key)?;
        let (h, t) = match self.rig3_live.get(key) {
            Some(v) => *v,
            None => {
                let sk = &self.rigs3[&arm].sk;
                (pv(sk.bones[bi].head), pv(sk.bones[bi].tip()))
            }
        };
        Some(tup(if name == "head" { h } else { t }))
    }

    /// Does this rig-method call belong to a 3D armature?
    pub fn rig3_wants(&self, sn: &str, name: &str, args: &Args) -> bool {
        match name {
            "reach" | "release" => self.ik3_of(sn).is_some(),
            "bind" => args
                .iter()
                .any(|(_, v)| matches!(v, Value::Shape(k) if self.bone3_of(k).is_some())),
            "skin" => args
                .iter()
                .any(|(_, v)| matches!(v, Value::Armature(a) if self.rigs3.contains_key(a))),
            "set_weight" => self.decls.skins3.iter().any(|d| d.1.shape == sn),
            _ => false,
        }
    }

    fn check_rig3_free(&self, sn: &str) -> Res<()> {
        if self.decls.body(sn).is_some()
            || self.decls.soft.iter().any(|d| d.1.shape == sn)
            || self.decls3.bodies.iter().any(|d| d.1.shape == sn)
            || self.decls3.soft.iter().any(|d| d.1.shape == sn)
        {
            return Err(e7(format!(
                "`{sn}` is a simulated body; a mesh is rigid, soft or rig-driven, not two of them"
            )));
        }
        if self.decls.binds.iter().any(|d| d.1.shape == sn)
            || self.decls.skins.iter().any(|d| d.1.shape == sn)
            || self.decls.skins3.iter().any(|d| d.1.shape == sn)
        {
            return Err(e7(format!("`{sn}` is already bound or skinned")));
        }
        Ok(())
    }

    pub fn rig3_method(
        &mut self,
        sn: &str,
        name: &str,
        args: Args,
        mode: Mode,
        span: Span,
    ) -> Res<Value> {
        match name {
            "reach" | "release" => {
                let Some((arm, k)) = self.ik3_of(sn) else {
                    unreachable!()
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
                        .triple_of(&m["to"], "the target")
                        .map_err(|e| e6(e.message))?;
                    let rest = self.rigs3[&arm].ik[k].rest_tip;
                    vec![
                        (PropId::PosX, Tgt::To(to[0] - rest[0])),
                        (PropId::PosY, Tgt::To(to[1] - rest[1])),
                        (PropId::PosZ, Tgt::To(to[2] - rest[2])),
                        (PropId::Opacity, Tgt::To(1.0)),
                    ]
                } else {
                    vec![(PropId::Opacity, Tgt::To(0.0))]
                };
                if self.over_frames(&m, mode, span)?.is_none() {
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
            _ => {
                if mode == Mode::Run {
                    return Err(Diagnostic::new("E-SHP-003", format!("`{name}` attaches art to a rig and is not allowed while the scene is running")).help("bind and skin at the top level or in `setup:`"));
                }
                if !self.is_mesh_shape(sn) {
                    return Err(e7(format!("`{sn}` is not a mesh; a 3D armature skins meshes (cube, sphere, lathe, extrude, ...)")));
                }
                let scene = mode == Mode::Setup;
                match name {
                    "bind" => {
                        let m = self.bind(name, args, &["bone"], &["bone"])?;
                        let Value::Shape(key) = &m["bone"] else {
                            return Err(e7("`bind` needs a bone such as `armature.torso`"));
                        };
                        let Some((arm, bone)) = self.bone3_of(key) else {
                            return Err(e7(format!("`{key}` is not a bone")));
                        };
                        self.check_rig3_free(sn)?;
                        self.decls.skins3.push((
                            scene,
                            Skin3Decl {
                                shape: sn.to_string(),
                                arm,
                                blend: 1.0,
                                bones: Some(vec![bone]),
                                paints: vec![],
                            },
                        ));
                    }
                    "skin" => {
                        let m =
                            self.bind(name, args, &["armature", "blend", "bones"], &["armature"])?;
                        let Value::Armature(arm) = &m["armature"] else {
                            return Err(e7("`skin` needs an armature"));
                        };
                        let blend = m
                            .get("blend")
                            .map(|v| self.num(v, "`blend`").map_err(|e| e7(e.message)))
                            .transpose()?
                            .unwrap_or(20.0);
                        if blend <= 0.0 {
                            return Err(e7("`blend` must be positive (the falloff length in px)"));
                        }
                        let mut only = None;
                        if let Some(Value::List(items)) = m.get("bones") {
                            let mut v = vec![];
                            for x in items {
                                let Value::Shape(k) = x else {
                                    return Err(e7("`bones` lists bones of the armature"));
                                };
                                match self.bone3_of(k) {
                                    Some((a, i)) if a == *arm => v.push(i),
                                    _ => return Err(e7(format!("`{k}` is not a bone of `{arm}`"))),
                                }
                            }
                            only = Some(v);
                        }
                        self.check_rig3_free(sn)?;
                        self.decls.skins3.push((
                            scene,
                            Skin3Decl {
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
                            &["where", "bone", "weight"],
                            &["where", "bone", "weight"],
                        )?;
                        let ids = self.select_vertices(sn, &m["where"], mode)?;
                        let Value::Shape(key) = &m["bone"] else {
                            return Err(e7("`set_weight` needs a bone"));
                        };
                        let w = self
                            .num(&m["weight"], "`weight`")
                            .map_err(|e| e7(e.message))?;
                        if !(0.0..=1.0).contains(&w) {
                            return Err(e7("a weight is between 0 and 1"));
                        }
                        let bone = self.bone3_of(key);
                        let Some(d) = self.decls.skins3.iter_mut().find(|d| d.1.shape == sn) else {
                            return Err(e7(format!(
                                "`{sn}` is not skinned; call `{sn}.skin(armature)` first"
                            )));
                        };
                        match bone {
                            Some((a, i)) if a == d.1.arm => d.1.paints.push((ids, i, w)),
                            _ => {
                                return Err(e7(format!(
                                    "`{key}` is not a bone of the armature `{}`",
                                    d.1.arm
                                )))
                            }
                        }
                    }
                }
                Ok(Value::None)
            }
        }
    }

    /// Fix each skinned mesh to the rest pose (after `setup:`): bake its transform and compute weights.
    pub fn build_rigs3(&mut self) -> Res<()> {
        self.rig3_sim = None;
        if self.decls.skins3.is_empty() {
            return Ok(());
        }
        let mut sim = Rig3Sim::default();
        let skins: Vec<Skin3Decl> = self.decls.skins3.iter().map(|b| b.1.clone()).collect();
        for d in skins {
            let t = self.mesh_transform(&d.shape);
            let r = &self.rigs3[&d.arm];
            let sh = self.shapes.get_mut(&d.shape).expect("declared shape");
            let m = sh.mesh.as_mut().expect("mesh");
            for v in m.verts.iter_mut() {
                *v = t.apply(*v);
            }
            sh.position = [0.0, 0.0];
            sh.pos_z = 0.0;
            sh.rot_x = 0.0;
            sh.rot_y = 0.0;
            sh.rotation = 0.0;
            sh.scale = [1.0, 1.0];
            sh.scale_z = 1.0;
            sh.pivot3 = [0.0; 3];
            sh.pivot = None;
            sh.orient = None;
            let rest: Vec<V3> = m.verts.iter().map(|v| vp(*v)).collect();
            let bones: Vec<usize> = d.bones.clone().unwrap_or_default();
            let mut weights: Vec<Vec<(usize, f64)>> = rest
                .iter()
                .map(|p| p3::auto_weights3(&r.sk, *p, &bones, d.blend))
                .collect();
            for (vids, bone, w) in &d.paints {
                for &vid in vids {
                    let Some(cur) = weights.get(vid) else {
                        continue;
                    };
                    let others: f64 = cur.iter().filter(|x| x.0 != *bone).map(|x| x.1).sum();
                    let mut nw: Vec<(usize, f64)> = cur
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
                    weights[vid] = nw;
                }
            }
            sim.skins.push(Skin3Sim {
                shape: d.shape.clone(),
                arm: d.arm.clone(),
                rest,
                weights,
            });
        }
        self.rig3_sim = Some(sim);
        Ok(())
    }

    fn pose_of(sk: &Skeleton3, off: V3, deltas: &[Quat]) -> Vec<p3::BonePose> {
        sk.pose(off, deltas)
    }

    /// FK, limits and IK for every 3D armature. `bones`: `[rot x, rot y, rot z (turns), x, y, z offset]`
    /// of each bone pseudo-shape; `iks`: `[x, y, z offset, weight]`.
    pub fn rig3_solve(
        &mut self,
        bones: &BTreeMap<String, [f64; 6]>,
        iks: &BTreeMap<String, [f64; 4]>,
    ) -> BTreeMap<String, Rig3Pose> {
        let mut out = BTreeMap::new();
        let names: Vec<String> = self.rigs3.keys().cloned().collect();
        for name in names {
            let (sk, bone_names, ik_defs) = {
                let r = &self.rigs3[&name];
                (
                    r.sk.clone(),
                    r.bones.clone(),
                    r.ik.iter()
                        .map(|k| (k.name.clone(), k.chain.clone(), k.pole, k.rest_tip))
                        .collect::<Vec<_>>(),
                )
            };
            let n = sk.bones.len();
            let mut deltas: Vec<Quat> = Vec::with_capacity(n);
            for b in &bone_names {
                let v = bones
                    .get(&format!("{name}.{b}"))
                    .copied()
                    .unwrap_or([0.0; 6]);
                deltas.push(Quat::from_euler_turns(v[0], v[1], v[2]));
            }
            let off = bone_names
                .first()
                .and_then(|b| bones.get(&format!("{name}.{b}")))
                .map_or(V3::ZERO, |v| v3(v[3], v[4], v[5]));
            for i in 0..n {
                let (q, hit) = sk.clamp(i, deltas[i]);
                if hit
                    && self
                        .rig3_sim
                        .as_mut()
                        .is_none_or(|s| s.warned.insert((name.clone(), i)))
                {
                    self.diagnostics.push(Diagnostic::new("W-RIG-001", format!("bone `{}.{}` was asked to rotate beyond its limits and was clamped", name, bone_names[i])).help("widen the limits or change the animation"));
                }
                deltas[i] = q;
            }
            let mut ik_snap = vec![];
            for (iname, chain, pole, rest_tip) in &ik_defs {
                let v = iks
                    .get(&format!("{name}.{iname}"))
                    .copied()
                    .unwrap_or([0.0; 4]);
                let w = v[3].clamp(0.0, 1.0);
                let target = v3(rest_tip[0] + v[0], rest_tip[1] + v[1], rest_tip[2] + v[2]);
                if w > 0.0 {
                    let pose = Self::pose_of(&sk, off, &deltas);
                    let first = chain[0];
                    let c_parent = sk.bones[first].parent.map_or(Quat::IDENTITY, |p| pose[p].c);
                    let head = pose[first].head;
                    let pole_pt = match pole {
                        Some(p) => vp(*p),
                        None => head + v3(0.0, 0.0, -1000.0),
                    };
                    let (solved, _) = sk.ik_chain(chain, head, c_parent, target, pole_pt);
                    for (k, &b) in chain.iter().enumerate() {
                        let blended = p3::slerp(deltas[b], solved[k], w);
                        deltas[b] = sk.clamp(b, blended).0;
                    }
                }
                self.ik3_live
                    .insert(format!("{name}.{iname}"), (pv(target), w));
                ik_snap.push(json!({"name": iname, "target": pv(target), "weight": w}));
            }
            let pose = Self::pose_of(&sk, off, &deltas);
            out.insert(
                name,
                Rig3Pose {
                    deltas,
                    pose,
                    ik: ik_snap,
                },
            );
        }
        out
    }

    /// The snapshot and the skinned vertices of every skinned mesh.
    pub fn rig3_finish(&mut self, poses: &BTreeMap<String, Rig3Pose>) -> Rig3Out {
        let mut verts = vec![];
        let mut snap = vec![];
        for (name, rp) in poses {
            let (sk, bone_names) = {
                let r = &self.rigs3[name];
                (r.sk.clone(), r.bones.clone())
            };
            let bone_snap: Vec<J> = (0..sk.bones.len())
                .map(|i| {
                    let p = &rp.pose[i];
                    self.rig3_live.insert(format!("{name}.{}", bone_names[i]), (pv(p.head), pv(p.tip)));
                    json!({"name": bone_names[i], "head": pv(p.head), "tip": pv(p.tip), "q": p.c.arr()})
                })
                .collect();
            snap.push(json!({"name": name, "bones": bone_snap, "ik": rp.ik}));
            if let Some(sim) = &self.rig3_sim {
                for s in sim.skins.iter().filter(|s| s.arm == *name) {
                    let mut m = BTreeMap::new();
                    for (k, p) in s.rest.iter().enumerate() {
                        let o = if s.weights[k].is_empty() {
                            *p
                        } else {
                            p3::skin_point3(&sk, &rp.pose, &s.weights[k], *p)
                        };
                        m.insert(k as u32, pv(o));
                    }
                    verts.push((s.shape.clone(), m));
                }
            }
        }
        Rig3Out {
            verts,
            snapshot: J::Array(snap),
        }
    }
}
