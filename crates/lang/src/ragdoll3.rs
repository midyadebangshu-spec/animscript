//! 3D ragdolls and active ragdolls: an armature with a `ragdoll:` line gets one capsule body per bone
//! in the 3D physics world, joined by ball joints (swing and twist cones) or hinges. Bones start
//! *animated*: kinematic bodies that follow the pose the animation and IK produce, so they can push
//! other bodies. `armature.ragdoll(strength: 0, push: (0, 0, 0))` makes them dynamic; with `strength`
//! above zero each joint gets a muscle that pulls it towards the animated pose. `armature.animate()`
//! hands the bones back. While physical, the pose (and so skinned meshes, `bone.head`, the snapshot)
//! comes from the bodies.
use crate::interp::{Interp, Mode};
use crate::phys3::Physics3;
use crate::ragdoll::RagState;
use crate::rig3::Rig3Pose;
use crate::value::*;
use animscript_core::*;
use animscript_physics3 as p3;
use p3::{arc, v3, BodyKind, Quat, V3};
use std::collections::BTreeMap;

type Args = Vec<(Option<String>, Value)>;

pub struct Rag3 {
    pub(crate) bodies: Vec<usize>,
    pub(crate) joints: Vec<Option<usize>>,
    pub(crate) full_torque: Vec<f64>,
    pub(crate) state: RagState,
}

fn rag_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-RIG-008", msg)
}

fn up() -> V3 {
    v3(0.0, 1.0, 0.0)
}

impl Interp {
    pub fn has_ragdolls3(&self) -> bool {
        self.rigs3.values().any(|r| r.ragdoll.is_some())
    }

    #[allow(clippy::needless_range_loop)]
    pub(crate) fn build_ragdolls3(&self, world: &mut p3::World, p: &mut Physics3) {
        let ppm = p.ppm;
        let g = world.gravity.len().max(9.8);
        for (name, r) in &self.rigs3 {
            let Some(cfg) = r.ragdoll else { continue };
            let n = r.sk.bones.len();
            let rest = r.sk.pose(V3::ZERO, &[]);
            let mut bodies = vec![];
            let mut masses = vec![];
            for i in 0..n {
                let b = &r.sk.bones[i];
                let rad = r.radius[i].unwrap_or(cfg.radius) / ppm;
                let len = b.length / ppm;
                let half = (len / 2.0 - rad).max(0.0);
                let centre = (rest[i].head + b.dir * (b.length / 2.0)) / ppm;
                let mut def = p3::BodyDef::new(p3::Shape::Capsule { half, r: rad });
                def.kind = BodyKind::Kinematic;
                def.position = centre;
                def.orientation = arc(up(), b.dir);
                def.density = cfg.density;
                def.friction = cfg.friction;
                def.restitution = cfg.restitution;
                def.angular_damping = 0.05;
                let id = world.create_body(&def);
                masses.push(world.body(id).shape.mass_data(cfg.density).mass);
                let key = format!("{name}.{}", r.bones[i]);
                debug_assert_eq!(p.names.len(), id);
                p.names.push(key.clone());
                p.ids.insert(key, id);
                bodies.push(id);
            }
            let mut joints = vec![None; n];
            for i in 0..n {
                let Some(par) = r.sk.bones[i].parent else {
                    continue;
                };
                let b = &r.sk.bones[i];
                let anchor = rest[i].head / ppm;
                let mut def = p3::JointDef {
                    a: bodies[par],
                    b: bodies[i],
                    kind: p3::JointKind::Ball,
                    anchor_a: anchor,
                    anchor_b: None,
                    axis: b.dir,
                    collide_connected: false,
                };
                if let Some((axis, lo, hi)) = b.hinge {
                    def.kind = p3::JointKind::Hinge {
                        limits: Some((lo, hi)),
                        motor: None,
                    };
                    def.axis = axis;
                }
                let j = world.create_joint(&def);
                if b.hinge.is_none() && (b.swing.is_some() || b.twist.is_some()) {
                    world.set_joint_cone(
                        j,
                        Some(p3::ConeDef {
                            swing: b.swing.unwrap_or(std::f64::consts::PI),
                            twist: b.twist,
                        }),
                    );
                }
                joints[i] = Some(j);
            }
            for a in 0..n {
                for b in a + 1..n {
                    world.disable_collision(bodies[a], bodies[b]);
                }
            }
            let mut full_torque = vec![0.0; n];
            for i in 0..n {
                let (mut mass, mut reach) = (0.0, 0.0f64);
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
                        reach = reach.max((r.sk.bones[j].tip() - r.sk.bones[i].head).len() / ppm);
                    }
                }
                full_torque[i] = 8.0 * g * mass * reach.max(0.01);
            }
            p.rag.insert(
                name.clone(),
                Rag3 {
                    bodies,
                    joints,
                    full_torque,
                    state: RagState::Animated,
                },
            );
        }
    }

    /// Body centre and orientation for a posed bone.
    fn bone_target(&self, arm: &str, i: usize, rp: &Rig3Pose, ppm: f64) -> (V3, Quat) {
        let b = &self.rigs3[arm].sk.bones[i];
        let pose = &rp.pose[i];
        let centre = pose.head + pose.c.rotate(b.dir * (b.length / 2.0));
        (centre / ppm, pose.c.mul(arc(up(), b.dir)).normalized())
    }

    /// Before the step: animated bodies move to this tick's pose; muscles aim at it.
    #[allow(clippy::needless_range_loop)]
    pub fn ragdoll3_prepare(&mut self, tick: i64, dt: f64, poses: &BTreeMap<String, Rig3Pose>) {
        let Some(mut p) = self.phys3.take() else {
            return;
        };
        let ppm = p.ppm;
        let names: Vec<String> = p.rag.keys().cloned().collect();
        for name in names {
            let Some(rp) = poses.get(&name) else { continue };
            let n = self.rigs3[&name].sk.bones.len();
            let targets: Vec<(V3, Quat)> = (0..n)
                .map(|i| self.bone_target(&name, i, rp, ppm))
                .collect();
            let rag = &p.rag[&name];
            match rag.state {
                RagState::Animated => {
                    for i in 0..n {
                        let (c, q) = targets[i];
                        if tick == 0 {
                            p.world.set_transform(rag.bodies[i], c, q);
                        } else {
                            p.world.move_kinematic(rag.bodies[i], c, q, dt);
                        }
                    }
                }
                RagState::Physical(s) if s > 0.0 && tick > 0 => {
                    for i in 0..n {
                        if let (Some(j), Some(par)) =
                            (rag.joints[i], self.rigs3[&name].sk.bones[i].parent)
                        {
                            let rel = targets[par].1.conj().mul(targets[i].1).normalized();
                            p.world.set_joint_muscle_target(j, rel);
                            p.world.set_joint_muscle(
                                j,
                                Some(p3::MuscleDef {
                                    hertz: 1.5 + 2.5 * s,
                                    zeta: 0.7,
                                    max_torque: s * rag.full_torque[i] * 0.25,
                                }),
                            );
                        }
                        // every bone is also pulled towards its animated placement, so the whole body is drawn
                        // up together (the speeds are capped: a body lying far away stands up, it is not thrown)
                        let cap = |v: V3, m: f64| if v.len() > m { v * (m / v.len()) } else { v };
                        let (c, q) = targets[i];
                        let b = p.world.body(rag.bodies[i]);
                        let k = 0.9 * s;
                        let v = b.v + (cap((c - b.p) / dt, 3.0) - b.v) * k;
                        let mut dq = q.mul(b.q.conj());
                        if dq.w < 0.0 {
                            dq = Quat {
                                w: -dq.w,
                                x: -dq.x,
                                y: -dq.y,
                                z: -dq.z,
                            };
                        }
                        let sn = (dq.x * dq.x + dq.y * dq.y + dq.z * dq.z).sqrt();
                        let target_w = if sn < 1e-12 {
                            V3::ZERO
                        } else {
                            v3(dq.x, dq.y, dq.z) / sn * (2.0 * sn.atan2(dq.w) / dt)
                        };
                        let w = b.w + (cap(target_w, 3.0) - b.w) * k;
                        p.world.set_linear_velocity(rag.bodies[i], v);
                        p.world.set_angular_velocity(rag.bodies[i], w);
                    }
                }
                RagState::Physical(_) => {}
            }
        }
        self.phys3 = Some(p);
    }

    /// After the step: a physical armature's pose is read back from its bodies.
    pub fn ragdoll3_poses(&self, poses: &mut BTreeMap<String, Rig3Pose>) {
        let Some(p) = self.phys3.as_ref() else { return };
        for (name, rag) in &p.rag {
            if rag.state == RagState::Animated {
                continue;
            }
            let (Some(rp), Some(r)) = (poses.get_mut(name), self.rigs3.get(name)) else {
                continue;
            };
            for i in 0..rag.bodies.len() {
                let b = p.world.body(rag.bodies[i]);
                let d = &r.sk.bones[i];
                let c = b.q.mul(arc(up(), d.dir).conj()).normalized();
                let head = b.p * p.ppm - c.rotate(d.dir * (d.length / 2.0));
                rp.pose[i] = p3::BonePose {
                    head,
                    c,
                    tip: head + c.rotate(d.dir * d.length),
                };
            }
        }
    }

    pub fn is_ragdolled3(&self, arm: &str) -> bool {
        self.phys3
            .as_ref()
            .and_then(|p| p.rag.get(arm))
            .is_some_and(|r| r.state != RagState::Animated)
    }

    pub fn armature_method3(
        &mut self,
        arm: &str,
        name: &str,
        args: Args,
        mode: Mode,
    ) -> Res<Value> {
        if self.rigs3[arm].ragdoll.is_none() {
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
            let push = match m.get("push") {
                None => [0.0; 3],
                Some(Value::Tuple(t)) if t.len() == 3 => [
                    self.num(&t[0], "`push`")?,
                    self.num(&t[1], "`push`")?,
                    self.num(&t[2], "`push`")?,
                ],
                Some(_) => return Err(rag_err("`push` is a 3D velocity such as `(0, 0, 300)`")),
            };
            (s, push)
        } else {
            self.bind(name, args, &[], &[])?;
            (0.0, [0.0; 3])
        };
        let Some(p) = self.phys3.as_mut() else {
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
                p.world.set_body_kind(*id, BodyKind::Kinematic);
                if let Some(j) = rag.joints[i] {
                    p.world.set_joint_muscle(j, None);
                }
            }
        } else {
            rag.state = RagState::Physical(strength);
            for (i, id) in rag.bodies.iter().enumerate() {
                let v = p.world.body(*id).v;
                p.world.set_body_kind(*id, BodyKind::Dynamic);
                p.world
                    .set_linear_velocity(*id, v + v3(push[0], push[1], push[2]) / ppm);
                if strength == 0.0 {
                    if let Some(j) = rag.joints[i] {
                        p.world.set_joint_muscle(j, None);
                    }
                }
            }
        }
        Ok(Value::None)
    }
}
