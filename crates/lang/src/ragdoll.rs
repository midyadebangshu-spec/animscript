//! Ragdolls and active ragdolls. An armature with a `ragdoll:` line gets one capsule body per bone,
//! joined by hinges that carry the bones' `limits`. Bones start *animated*: kinematic bodies that follow
//! the pose the layer stack and IK produce, so they can push other bodies. `armature.ragdoll()` makes them
//! dynamic (they keep their motion); with `strength` above zero each hinge gets a muscle (an angle spring)
//! that pulls it towards the animated pose and the root bone is driven towards its animated placement.
//! `armature.animate()` hands the bones back. While physical, the bone poses (and so any bound or skinned
//! art, `bone.head` and the snapshot) come from the bodies.
use crate::interp::{Interp, Mode};
use crate::phys::Physics;
use crate::rig::RigPose;
use crate::value::*;
use animscript_core::*;
use animscript_physics as ph;
use ph::{v2, Vec2};
use std::collections::BTreeMap;

type Args = Vec<(Option<String>, Value)>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RagState {
    Animated,
    /// Dynamic bodies; the number is the muscle strength (0 = limp).
    Physical(f64),
}

pub struct Rag {
    pub(crate) bodies: Vec<usize>,
    /// The hinge to each bone's parent (none for the root).
    pub(crate) joints: Vec<Option<usize>>,
    /// Torque that holds the bone and everything below it against gravity, times a safety factor.
    pub(crate) full_torque: Vec<f64>,
    pub(crate) state: RagState,
}

fn dir(a: f64) -> Vec2 {
    v2(a.cos(), a.sin())
}

fn rag_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-004", msg)
}

impl Interp {
    pub fn has_ragdolls(&self) -> bool {
        self.rigs.values().any(|r| r.ragdoll.is_some())
    }

    /// Create the bodies and hinges of every ragdoll armature, in the rest pose, as animated bodies.
    #[allow(clippy::needless_range_loop)]
    pub(crate) fn build_ragdolls(&self, world: &mut ph::World, p: &mut Physics) {
        let ppm = p.ppm;
        let g = world.gravity.length().max(9.8);
        for (name, r) in &self.rigs {
            let Some(cfg) = r.ragdoll else { continue };
            let n = r.sk.bones.len();
            let rest = r.sk.pose(&vec![0.0; n], Vec2::ZERO);
            let mut bodies = vec![];
            let mut masses = vec![];
            for i in 0..n {
                let len = r.sk.bones[i].length;
                let rad = r.radius[i].unwrap_or(cfg.radius);
                let (hx, hy, cr) = (len / 2.0 / ppm, rad / ppm, rad / ppm);
                let centre = rest[i].head + dir(rest[i].angle) * (len / 2.0);
                let id = world.create_body(&ph::BodyDef {
                    kind: ph::BodyKind::Kinematic,
                    position: v2(centre.x / ppm, centre.y / ppm),
                    angle: rest[i].angle,
                    ..ph::BodyDef::default()
                });
                let fix = |shape: ph::Shape| ph::FixtureDef {
                    shape,
                    density: cfg.density,
                    friction: cfg.friction,
                    restitution: cfg.restitution,
                    sensor: false,
                };
                world.add_fixture(id, &fix(ph::Shape::boxed(hx, hy)));
                for sx in [-hx, hx] {
                    world.add_fixture(
                        id,
                        &fix(ph::Shape::Circle {
                            center: v2(sx, 0.0),
                            radius: cr,
                        }),
                    );
                }
                masses.push(cfg.density * (4.0 * hx * hy + 2.0 * std::f64::consts::PI * cr * cr));
                let key = format!("{name}.{}", r.bones[i]);
                debug_assert_eq!(p.names.len(), id);
                p.names.push(key.clone());
                p.ids.insert(key, id);
                bodies.push(id);
            }
            let mut joints = vec![None; n];
            for i in 0..n {
                if let Some(par) = r.sk.bones[i].parent {
                    let anchor = v2(rest[i].head.x / ppm, rest[i].head.y / ppm);
                    let mut def = ph::JointDef::revolute(bodies[par], bodies[i], anchor);
                    if let Some((lo, hi)) = r.sk.bones[i].limits {
                        def = def.limits(lo, hi);
                    }
                    joints[i] = Some(world.create_joint(&def));
                }
            }
            for a in 0..n {
                for b in a + 1..n {
                    world.disable_collision(bodies[a], bodies[b]);
                }
            }
            // what a muscle must hold: the bone and its descendants, about the hinge
            let mut full_torque = vec![0.0; n];
            for i in 0..n {
                let mut mass = 0.0;
                let mut reach = 0.0f64;
                for j in i..n {
                    let mut k = j;
                    let below = loop {
                        if k == i {
                            break true;
                        }
                        match r.sk.bones[k].parent {
                            Some(pk) => k = pk,
                            None => break false,
                        }
                    };
                    if below {
                        mass += masses[j];
                        let tip = r.sk.tip(&rest, j);
                        let d = tip - rest[i].head;
                        reach = reach.max(d.length() / ppm);
                    }
                }
                full_torque[i] = 8.0 * g * mass * reach.max(0.01);
            }
            p.rag.insert(
                name.clone(),
                Rag {
                    bodies,
                    joints,
                    full_torque,
                    state: RagState::Animated,
                },
            );
        }
    }

    /// Before the step: animated bodies move to the pose of this tick; muscles aim at it.
    pub fn ragdoll_prepare(&mut self, tick: i64, dt: f64, poses: &BTreeMap<String, RigPose>) {
        let Some(p) = self.phys.as_mut() else { return };
        let ppm = p.ppm;
        let names: Vec<String> = p.rag.keys().cloned().collect();
        for name in names {
            let Some(rp) = poses.get(&name) else { continue };
            let r = &self.rigs[&name];
            let rag = &p.rag[&name];
            let target = |i: usize| {
                let len = r.sk.bones[i].length;
                let a = rp.pose[i].angle;
                let c = rp.pose[i].head + dir(a) * (len / 2.0);
                (v2(c.x / ppm, c.y / ppm), a)
            };
            match rag.state {
                RagState::Animated => {
                    for i in 0..rag.bodies.len() {
                        let (c, a) = target(i);
                        if tick == 0 {
                            p.world.set_transform(rag.bodies[i], c, a);
                        } else {
                            p.world.move_kinematic(rag.bodies[i], c, a, dt);
                        }
                    }
                }
                RagState::Physical(s) if s > 0.0 && tick > 0 => {
                    for i in 0..rag.bodies.len() {
                        match rag.joints[i] {
                            Some(j) => {
                                p.world.set_joint_angle_target(j, rp.rots[i]);
                                p.world.set_joint_angle_spring(
                                    j,
                                    Some((3.0 + 9.0 * s, 0.7, s * rag.full_torque[i])),
                                );
                            }
                            None => {
                                // the root has no hinge: its velocity is blended towards the animated one
                                let (c, a) = target(i);
                                let b = p.world.body(rag.bodies[i]);
                                let v = b.v + ((c - b.p) / dt - b.v) * s;
                                let w = b.w + (ph::wrap_angle(a - b.q.angle()) / dt - b.w) * s;
                                p.world.set_linear_velocity(rag.bodies[i], v);
                                p.world.set_angular_velocity(rag.bodies[i], w);
                            }
                        }
                    }
                }
                RagState::Physical(_) => {}
            }
        }
    }

    /// After the step: a physical armature's pose is read back from its bodies. The angle stays close to
    /// the animated one (it is continuous when the animation is), only the placement is the physics'.
    pub fn ragdoll_poses(&self, poses: &mut BTreeMap<String, RigPose>) {
        let Some(p) = self.phys.as_ref() else { return };
        for (name, rag) in &p.rag {
            if rag.state == RagState::Animated {
                continue;
            }
            let (Some(rp), Some(r)) = (poses.get_mut(name), self.rigs.get(name)) else {
                continue;
            };
            for i in 0..rag.bodies.len() {
                let b = p.world.body(rag.bodies[i]);
                let anim = rp.pose[i].angle;
                let angle = anim + ph::wrap_angle(b.q.angle() - anim);
                let head = b.p - dir(angle) * (r.sk.bones[i].length / 2.0 / p.ppm);
                rp.pose[i] = ph::BonePose {
                    head: v2(head.x * p.ppm, head.y * p.ppm),
                    angle,
                };
            }
        }
    }

    pub fn is_ragdolled(&self, arm: &str) -> bool {
        self.phys
            .as_ref()
            .and_then(|p| p.rag.get(arm))
            .is_some_and(|r| r.state != RagState::Animated)
    }

    /// `armature.ragdoll(strength: 0, push: (0, 0))` and `armature.animate()`.
    pub fn armature_method(&mut self, arm: &str, name: &str, args: Args, mode: Mode) -> Res<Value> {
        if self.rigs3.contains_key(arm) {
            return self.armature_method3(arm, name, args, mode);
        }
        if self.rigs[arm].ragdoll.is_none() {
            return Err(rag_err(format!("armature `{arm}` has no `ragdoll:` line"))
                .help("add `ragdoll: radius: 8` to the armature to give its bones bodies"));
        }
        if mode != Mode::Run {
            return Err(rag_err(format!(
                "`{name}` hands the bones to physics (or takes them back) while a scene runs"
            )));
        }
        let (strength, push) = if name == "ragdoll" {
            let m = self.bind(name, args, &["strength", "push"], &[])?;
            let s = m
                .get("strength")
                .map(|v| self.num(v, "`strength`"))
                .transpose()?
                .unwrap_or(0.0);
            if !(0.0..=1.0).contains(&s) {
                return Err(rag_err(
                    "`strength` is between 0 (limp) and 1 (as stiff as the animation)",
                ));
            }
            let push = m
                .get("push")
                .map(|v| self.point(v, "`push`"))
                .transpose()?
                .unwrap_or([0.0, 0.0]);
            (s, push)
        } else {
            self.bind(name, args, &[], &[])?;
            (0.0, [0.0, 0.0])
        };
        let Some(p) = self.phys.as_mut() else {
            return Err(rag_err("there is no physics world to run the ragdoll in"));
        };
        let ppm = p.ppm;
        let Some(rag) = p.rag.get_mut(arm) else {
            return Err(rag_err(format!(
                "armature `{arm}` is not in the running scene"
            )));
        };
        if name == "animate" {
            rag.state = RagState::Animated;
            for (i, id) in rag.bodies.iter().enumerate() {
                p.world.set_body_kind(*id, ph::BodyKind::Kinematic);
                p.world.set_linear_velocity(*id, Vec2::ZERO);
                p.world.set_angular_velocity(*id, 0.0);
                if let Some(j) = rag.joints[i] {
                    p.world.set_joint_angle_spring(j, None);
                }
            }
        } else {
            rag.state = RagState::Physical(strength);
            for (i, id) in rag.bodies.iter().enumerate() {
                p.world.set_body_kind(*id, ph::BodyKind::Dynamic);
                let v = p.world.body(*id).v;
                p.world
                    .set_linear_velocity(*id, v + v2(push[0] / ppm, push[1] / ppm));
                if strength == 0.0 {
                    if let Some(j) = rag.joints[i] {
                        p.world.set_joint_angle_spring(j, None);
                    }
                }
            }
        }
        Ok(Value::None)
    }
}

impl Interp {
    /// `raycast(origin, direction, max, ignore: shape-armature-or-"bones")` is the first solid point along the ray
    /// (or `none`); `raycast_normal` is the surface normal there. Bodies exist only while a scene runs.
    pub(crate) fn raycast_builtin(&mut self, name: &str, args: Args) -> Res<Value> {
        let m = self.bind(
            name,
            args,
            &["origin", "direction", "max", "ignore"],
            &["origin", "direction", "max"],
        )?;
        let o = self.point(&m["origin"], "`origin`")?;
        let d = self.point(&m["direction"], "`direction`")?;
        let max = self.num(&m["max"], "`max`")?;
        let len = d[0].hypot(d[1]);
        if len < 1e-12 || max < 0.0 {
            return Err(Diagnostic::new(
                "E-PHY-006",
                "`raycast` needs a direction that is not zero and a length that is not negative",
            ));
        }
        let Some(p) = self.phys.as_ref() else {
            return Ok(Value::None);
        };
        let mut skip: Vec<usize> = vec![];
        let mut ignore = |v: &Value| -> Res<()> {
            match v {
                Value::Shape(sn) => skip.extend(p.ids.get(sn)),
                Value::Armature(arm) => {
                    let prefix = format!("{arm}.");
                    skip.extend(
                        p.ids
                            .iter()
                            .filter(|(k, _)| k.starts_with(&prefix))
                            .map(|(_, i)| *i),
                    );
                }
                Value::Str(w) if w == "bones" => {
                    // every armature's bodies: the ground, not the characters standing on it
                    skip.extend(p.rag.keys().flat_map(|a| {
                        let prefix = format!("{a}.");
                        p.ids
                            .iter()
                            .filter(move |(k, _)| k.starts_with(&prefix))
                            .map(|(_, i)| *i)
                    }));
                }
                Value::None => {}
                other => {
                    return Err(Diagnostic::new(
                        "E-PHY-006",
                        format!(
                            "`ignore` is a shape, an armature, `bones` or a list of them, found {}",
                            other.type_name()
                        ),
                    ))
                }
            }
            Ok(())
        };
        match m.get("ignore") {
            Some(Value::List(items)) => {
                for i in items {
                    ignore(i)?;
                }
            }
            Some(v) => ignore(v)?,
            None => {}
        }
        let ppm = p.ppm;
        let dir = v2(d[0] / len, d[1] / len);
        let hit = p
            .world
            .raycast_filtered(v2(o[0] / ppm, o[1] / ppm), dir, max / ppm, &|b| {
                skip.contains(&b)
            });
        let pt = |x: f64, y: f64| Value::Tuple(vec![Value::Num(x), Value::Num(y)]);
        Ok(match hit {
            None => Value::None,
            Some(h) if name == "raycast" => pt(h.point.x * ppm, h.point.y * ppm),
            Some(h) => pt(h.normal.x, h.normal.y),
        })
    }
}
