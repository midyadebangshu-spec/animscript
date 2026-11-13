//! The 3D world: bodies, joints, the step (forces, contacts, sequential impulses, sleeping) and
//! contact events. Metres, kilograms, seconds. Every loop runs in index order, so a world always
//! produces the same bits for the same inputs.
use crate::collide::{collide, Manifold};
use crate::hull::Hull;
use crate::joint::{Joint, JointDef};
use crate::math::*;
use crate::shape::{Shape, WShape};
use std::collections::{BTreeMap, BTreeSet};

pub type BodyId = usize;
pub type JointId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Static,
    Dynamic,
    Kinematic,
}

#[derive(Clone, Debug)]
pub struct BodyDef {
    pub kind: BodyKind,
    pub shape: Shape,
    pub position: V3,
    pub orientation: Quat,
    pub density: f64,
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub gravity_scale: f64,
    pub fixed_rotation: bool,
    pub sensor: bool,
}

impl BodyDef {
    pub fn new(shape: Shape) -> BodyDef {
        BodyDef {
            kind: BodyKind::Dynamic,
            shape,
            position: V3::ZERO,
            orientation: Quat::IDENTITY,
            density: 1.0,
            friction: 0.6,
            restitution: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            gravity_scale: 1.0,
            fixed_rotation: false,
            sensor: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Body {
    pub kind: BodyKind,
    pub shape: Shape,
    pub p: V3,
    pub q: Quat,
    pub v: V3,
    pub w: V3,
    pub inv_mass: f64,
    pub inv_i_local: Mat3,
    pub inv_i: Mat3,
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub gravity_scale: f64,
    pub sensor: bool,
    pub awake: bool,
    pub sleep_t: f64,
    pub force: V3,
    pub torque: V3,
    pub mass: f64,
}

impl Body {
    pub fn is_dynamic(&self) -> bool {
        self.kind == BodyKind::Dynamic
    }
    fn refresh_inertia(&mut self) {
        let r = self.q.to_mat3();
        self.inv_i = r.mul(&self.inv_i_local).mul(&r.transpose());
    }
    /// Velocity of the material point at `world` position.
    pub fn velocity_at(&self, world: V3) -> V3 {
        self.v + self.w.cross(world - self.p)
    }
}

/// A touch starting or ending (`a < b`).
#[derive(Clone, Copy, Debug)]
pub struct ContactEvent {
    pub a: BodyId,
    pub b: BodyId,
    pub begin: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ContactInfo {
    pub a: BodyId,
    pub b: BodyId,
    pub point: V3,
    pub normal: V3,
    pub normal_impulse: f64,
}

#[derive(Clone, Debug)]
struct Contact {
    a: usize,
    b: usize,
    n: V3,
    point: V3,
    ra: V3,
    rb: V3,
    t1: V3,
    t2: V3,
    m_n: f64,
    m_t1: f64,
    m_t2: f64,
    vtarget: f64,
    jn: f64,
    jt1: f64,
    jt2: f64,
    friction: f64,
    local_a: V3,
}

#[derive(Clone, Debug)]
struct Cached {
    local_a: V3,
    impulse: V3,
}

pub struct World {
    pub gravity: V3,
    bodies: Vec<Body>,
    joints: Vec<Joint>,
    wshapes: Vec<Option<WShape>>,
    prev: BTreeMap<(usize, usize), Vec<Cached>>,
    touching: BTreeSet<(usize, usize)>,
    contacts: Vec<Contact>,
    events: Vec<ContactEvent>,
    disabled: BTreeSet<(usize, usize)>,
    pub iterations: usize,
    aabbs: Vec<Option<(V3, V3)>>,
}

/// A soft particle touching a rigid body.
#[derive(Clone, Copy, Debug)]
pub struct SoftHit {
    pub body: BodyId,
    /// A point on the rigid surface.
    pub point: V3,
    /// Out of the rigid body, towards the particle.
    pub normal: V3,
    pub depth: f64,
}

const SLOP: f64 = 0.004;
const PUSH: f64 = 0.2;
const MAX_PUSH: f64 = 3.0;
const BOUNCE_THRESHOLD: f64 = 0.6;
const SLEEP_LIN: f64 = 0.06;
const SLEEP_ANG: f64 = 0.12;
const SLEEP_TIME: f64 = 0.5;

impl World {
    pub fn new(gravity: V3) -> World {
        World {
            gravity,
            bodies: vec![],
            joints: vec![],
            wshapes: vec![],
            prev: BTreeMap::new(),
            touching: BTreeSet::new(),
            contacts: vec![],
            events: vec![],
            disabled: BTreeSet::new(),
            iterations: 10,
            aabbs: vec![],
        }
    }

    pub fn create_body(&mut self, def: &BodyDef) -> BodyId {
        let (mut inv_mass, mut inv_i) = (0.0, Mat3::ZERO);
        let mut mass = 0.0;
        if def.kind == BodyKind::Dynamic {
            let md = def.shape.mass_data(def.density);
            if md.mass > 0.0 {
                mass = md.mass;
                inv_mass = 1.0 / md.mass;
                if !def.fixed_rotation {
                    inv_i = md.inertia.inverse();
                }
            }
        }
        let mut b = Body {
            kind: def.kind,
            shape: def.shape.clone(),
            p: def.position,
            q: def.orientation.normalized(),
            v: V3::ZERO,
            w: V3::ZERO,
            inv_mass,
            inv_i_local: inv_i,
            inv_i,
            friction: def.friction,
            restitution: def.restitution,
            linear_damping: def.linear_damping,
            angular_damping: def.angular_damping,
            gravity_scale: def.gravity_scale,
            sensor: def.sensor,
            awake: true,
            sleep_t: 0.0,
            force: V3::ZERO,
            torque: V3::ZERO,
            mass,
        };
        b.refresh_inertia();
        self.bodies.push(b);
        self.wshapes.push(None);
        self.bodies.len() - 1
    }

    pub fn create_joint(&mut self, def: &JointDef) -> JointId {
        let j = Joint::new(def, &self.bodies);
        if !def.collide_connected {
            self.disabled.insert((def.a.min(def.b), def.a.max(def.b)));
        }
        self.joints.push(j);
        self.joints.len() - 1
    }

    pub fn body(&self, id: BodyId) -> &Body {
        &self.bodies[id]
    }
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }
    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }
    pub fn joint(&self, id: JointId) -> &Joint {
        &self.joints[id]
    }
    pub fn joint_anchors(&self, id: JointId) -> (V3, V3) {
        self.joints[id].anchors(&self.bodies)
    }
    pub fn joint_angle(&self, id: JointId) -> f64 {
        self.joints[id].angle(&self.bodies)
    }
    pub fn take_events(&mut self) -> Vec<ContactEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn disable_collision(&mut self, a: BodyId, b: BodyId) {
        self.disabled.insert((a.min(b), a.max(b)));
    }

    pub fn wake(&mut self, id: BodyId) {
        if self.bodies[id].is_dynamic() {
            self.bodies[id].awake = true;
            self.bodies[id].sleep_t = 0.0;
        }
    }
    pub fn set_linear_velocity(&mut self, id: BodyId, v: V3) {
        self.bodies[id].v = v;
        self.wake(id);
    }
    pub fn set_angular_velocity(&mut self, id: BodyId, w: V3) {
        self.bodies[id].w = w;
        self.wake(id);
    }
    pub fn apply_linear_impulse(&mut self, id: BodyId, impulse: V3) {
        let b = &mut self.bodies[id];
        b.v += impulse * b.inv_mass;
        self.wake(id);
    }
    pub fn apply_impulse_at(&mut self, id: BodyId, impulse: V3, point: V3) {
        let b = &mut self.bodies[id];
        b.v += impulse * b.inv_mass;
        let r = point - b.p;
        b.w += b.inv_i.mul_v(r.cross(impulse));
        self.wake(id);
    }
    /// A force held until the next step (cleared after it).
    pub fn apply_force_at(&mut self, id: BodyId, force: V3, point: V3) {
        let b = &mut self.bodies[id];
        b.force += force;
        b.torque += (point - b.p).cross(force);
        self.wake(id);
    }
    pub fn set_transform(&mut self, id: BodyId, p: V3, q: Quat) {
        let b = &mut self.bodies[id];
        b.p = p;
        b.q = q.normalized();
        b.refresh_inertia();
        self.wake(id);
    }
    /// Move a kinematic body to a pose; it gets the velocity that reaches it over `dt`.
    pub fn move_kinematic(&mut self, id: BodyId, p: V3, q: Quat, dt: f64) {
        let b = &mut self.bodies[id];
        b.v = (p - b.p) / dt;
        let mut dq = q.mul(b.q.conj());
        if dq.w < 0.0 {
            dq = Quat {
                w: -dq.w,
                x: -dq.x,
                y: -dq.y,
                z: -dq.z,
            };
        }
        let s = (dq.x * dq.x + dq.y * dq.y + dq.z * dq.z).sqrt();
        b.w = if s < 1e-12 {
            V3::ZERO
        } else {
            v3(dq.x, dq.y, dq.z) / s * (2.0 * s.atan2(dq.w) / dt)
        };
    }

    pub fn contact_infos(&self) -> Vec<ContactInfo> {
        self.contacts
            .iter()
            .map(|c| ContactInfo {
                a: c.a,
                b: c.b,
                point: c.point,
                normal: c.n,
                normal_impulse: c.jn,
            })
            .collect()
    }

    /// Pairs of bodies that overlap by more than `depth` right now.
    pub fn overlapping_pairs(&self, depth: f64) -> Vec<(BodyId, BodyId)> {
        let ws: Vec<WShape> = self
            .bodies
            .iter()
            .map(|b| b.shape.world(b.p, b.q))
            .collect();
        let mut out = vec![];
        for i in 0..ws.len() {
            for j in i + 1..ws.len() {
                if !self.bodies[i].is_dynamic() && !self.bodies[j].is_dynamic() {
                    continue;
                }
                if collide(&ws[i], &ws[j])
                    .iter()
                    .any(|m| m.points.iter().any(|p| p.1 > depth))
                {
                    out.push((i, j));
                }
            }
        }
        out
    }

    fn aabb_overlap(a: &Option<(V3, V3)>, b: &Option<(V3, V3)>, margin: f64) -> bool {
        match (a, b) {
            (Some((al, ah)), Some((bl, bh))) => {
                al.x - margin <= bh.x
                    && bl.x - margin <= ah.x
                    && al.y - margin <= bh.y
                    && bl.y - margin <= ah.y
                    && al.z - margin <= bh.z
                    && bl.z - margin <= ah.z
            }
            _ => true,
        }
    }

    /// Rebuild the world-space shapes (soft bodies query them between rigid steps).
    pub fn refresh_shapes(&mut self) {
        for i in 0..self.bodies.len() {
            let b = &self.bodies[i];
            if b.kind == BodyKind::Static && self.wshapes[i].is_some() {
                continue;
            }
            self.wshapes[i] = Some(b.shape.world(b.p, b.q));
        }
        self.aabbs = self
            .wshapes
            .iter()
            .map(|w| w.as_ref().and_then(|w| w.aabb()))
            .collect();
    }

    /// Rigid bodies a sphere of radius `r` at `c` overlaps (call `refresh_shapes` first).
    pub fn query_sphere(&self, c: V3, r: f64) -> Vec<SoftHit> {
        let probe = Some((c - v3(r, r, r), c + v3(r, r, r)));
        let sphere = WShape::Sphere { c, r };
        let mut out = vec![];
        for (i, w) in self.wshapes.iter().enumerate() {
            let (Some(w), Some(bb)) = (w, self.aabbs.get(i)) else {
                continue;
            };
            if self.bodies[i].sensor || !Self::aabb_overlap(bb, &probe, 0.0) {
                continue;
            }
            for m in collide(&sphere, w) {
                if let Some(&(pt, depth)) = m
                    .points
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                {
                    out.push(SoftHit {
                        body: i,
                        point: pt,
                        normal: -m.normal,
                        depth,
                    });
                }
            }
        }
        out
    }

    pub fn step(&mut self, dt: f64) {
        let n = self.bodies.len();
        // forces -> velocities
        for b in &mut self.bodies {
            if b.is_dynamic() && b.awake {
                b.v += (self.gravity * b.gravity_scale + b.force * b.inv_mass) * dt;
                b.w += b.inv_i.mul_v(b.torque) * dt;
                b.v *= 1.0 / (1.0 + dt * b.linear_damping);
                b.w *= 1.0 / (1.0 + dt * b.angular_damping);
            }
            b.force = V3::ZERO;
            b.torque = V3::ZERO;
        }
        // world shapes
        for i in 0..n {
            let b = &self.bodies[i];
            if b.kind == BodyKind::Static && self.wshapes[i].is_some() {
                continue;
            }
            self.wshapes[i] = Some(b.shape.world(b.p, b.q));
        }
        let aabbs: Vec<Option<(V3, V3)>> = self
            .wshapes
            .iter()
            .map(|w| w.as_ref().and_then(|w| w.aabb()))
            .collect();
        // contacts
        let mut contacts: Vec<Contact> = vec![];
        let mut now: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut manifolds: Vec<(usize, usize, Manifold)> = vec![];
        for i in 0..n {
            for j in i + 1..n {
                let (bi, bj) = (&self.bodies[i], &self.bodies[j]);
                if !bi.is_dynamic() && !bj.is_dynamic() {
                    continue;
                }
                let active = (bi.is_dynamic() && bi.awake) || (bj.is_dynamic() && bj.awake);
                if !active || self.disabled.contains(&(i, j)) {
                    continue;
                }
                if !Self::aabb_overlap(&aabbs[i], &aabbs[j], 0.01) {
                    continue;
                }
                let (wa, wb) = (
                    self.wshapes[i].as_ref().expect("built"),
                    self.wshapes[j].as_ref().expect("built"),
                );
                for m in collide(wa, wb) {
                    if m.points.is_empty() {
                        continue;
                    }
                    manifolds.push((i, j, m));
                }
            }
        }
        for (i, j, m) in manifolds {
            now.insert((i, j));
            let (bi, bj) = (&self.bodies[i], &self.bodies[j]);
            if bi.sensor || bj.sensor {
                continue;
            }
            // a touching sleeper wakes up
            if bi.is_dynamic() && !bi.awake || bj.is_dynamic() && !bj.awake {
                for k in [i, j] {
                    if self.bodies[k].is_dynamic() {
                        self.bodies[k].awake = true;
                        self.bodies[k].sleep_t = 0.0;
                    }
                }
            }
            let (bi, bj) = (&self.bodies[i], &self.bodies[j]);
            let friction = (bi.friction * bj.friction).sqrt();
            let restitution = bi.restitution.max(bj.restitution);
            let t1 = m.normal.any_perp();
            let t2 = m.normal.cross(t1);
            let cached = self.prev.get(&(i, j));
            for (pt, depth) in &m.points {
                let ra = *pt - bi.p;
                let rb = *pt - bj.p;
                let local_a = bi.q.conj().rotate(ra);
                let eff = |d: V3| -> f64 {
                    let k = bi.inv_mass
                        + bj.inv_mass
                        + ra.cross(d).dot(bi.inv_i.mul_v(ra.cross(d)))
                        + rb.cross(d).dot(bj.inv_i.mul_v(rb.cross(d)));
                    if k > 1e-300 {
                        1.0 / k
                    } else {
                        0.0
                    }
                };
                let dv = bj.velocity_at(*pt) - bi.velocity_at(*pt);
                let vn = dv.dot(m.normal);
                let mut vtarget = ((depth - SLOP).max(0.0) * PUSH / dt).min(MAX_PUSH);
                if vn < -BOUNCE_THRESHOLD {
                    vtarget = vtarget.max(-restitution * vn);
                }
                let (mut jn, mut jt1, mut jt2) = (0.0, 0.0, 0.0);
                if let Some(c) = cached {
                    let mut best: Option<(f64, &Cached)> = None;
                    for cc in c {
                        let d = (cc.local_a - local_a).len();
                        if d < 0.05 && best.is_none_or(|b| d < b.0) {
                            best = Some((d, cc));
                        }
                    }
                    if let Some((_, cc)) = best {
                        jn = cc.impulse.dot(m.normal).max(0.0);
                        jt1 = cc.impulse.dot(t1);
                        jt2 = cc.impulse.dot(t2);
                    }
                }
                contacts.push(Contact {
                    a: i,
                    b: j,
                    n: m.normal,
                    point: *pt,
                    ra,
                    rb,
                    t1,
                    t2,
                    m_n: eff(m.normal),
                    m_t1: eff(t1),
                    m_t2: eff(t2),
                    vtarget,
                    jn,
                    jt1,
                    jt2,
                    friction,
                    local_a,
                });
            }
        }
        // events
        for k in &now {
            if !self.touching.contains(k) {
                self.events.push(ContactEvent {
                    a: k.0,
                    b: k.1,
                    begin: true,
                });
            }
        }
        for k in &self.touching {
            if !now.contains(k) {
                self.events.push(ContactEvent {
                    a: k.0,
                    b: k.1,
                    begin: false,
                });
            }
        }
        self.touching = now;
        // joints and warm starting
        let live = |bodies: &[Body], a: usize, b: usize| {
            (bodies[a].is_dynamic() && bodies[a].awake)
                || (bodies[b].is_dynamic() && bodies[b].awake)
        };
        let mut joints = std::mem::take(&mut self.joints);
        for j in &mut joints {
            if live(&self.bodies, j.a, j.b) {
                for k in [j.a, j.b] {
                    if self.bodies[k].is_dynamic() {
                        self.bodies[k].awake = true;
                    }
                }
                j.h = dt;
                j.prepare(&mut self.bodies);
            }
        }
        for c in &contacts {
            let p = c.n * c.jn + c.t1 * c.jt1 + c.t2 * c.jt2;
            Self::apply(&mut self.bodies, c.a, c.b, c.ra, c.rb, p);
        }
        for _ in 0..self.iterations {
            for j in &mut joints {
                if live(&self.bodies, j.a, j.b) {
                    j.solve(&mut self.bodies, dt);
                }
            }
            for c in &mut contacts {
                let (ba, bb) = (&self.bodies[c.a], &self.bodies[c.b]);
                let dv = (bb.v + bb.w.cross(c.rb)) - (ba.v + ba.w.cross(c.ra));
                // friction
                let maxf = c.friction * c.jn;
                let l1 = -c.m_t1 * dv.dot(c.t1);
                let old = c.jt1;
                c.jt1 = (old + l1).clamp(-maxf, maxf);
                let d1 = c.jt1 - old;
                Self::apply(&mut self.bodies, c.a, c.b, c.ra, c.rb, c.t1 * d1);
                let (ba, bb) = (&self.bodies[c.a], &self.bodies[c.b]);
                let dv = (bb.v + bb.w.cross(c.rb)) - (ba.v + ba.w.cross(c.ra));
                let l2 = -c.m_t2 * dv.dot(c.t2);
                let old = c.jt2;
                c.jt2 = (old + l2).clamp(-maxf, maxf);
                let d2 = c.jt2 - old;
                Self::apply(&mut self.bodies, c.a, c.b, c.ra, c.rb, c.t2 * d2);
                // normal
                let (ba, bb) = (&self.bodies[c.a], &self.bodies[c.b]);
                let dv = (bb.v + bb.w.cross(c.rb)) - (ba.v + ba.w.cross(c.ra));
                let vn = dv.dot(c.n);
                let ln = c.m_n * (c.vtarget - vn);
                let old = c.jn;
                c.jn = (old + ln).max(0.0);
                let dn = c.jn - old;
                Self::apply(&mut self.bodies, c.a, c.b, c.ra, c.rb, c.n * dn);
            }
        }
        self.joints = joints;
        // remember impulses for the next step
        self.prev.clear();
        for c in &contacts {
            self.prev.entry((c.a, c.b)).or_default().push(Cached {
                local_a: c.local_a,
                impulse: c.n * c.jn + c.t1 * c.jt1 + c.t2 * c.jt2,
            });
        }
        self.contacts = contacts;
        // positions
        for b in &mut self.bodies {
            if b.kind == BodyKind::Static || (b.is_dynamic() && !b.awake) {
                continue;
            }
            let wl = b.w.len();
            if wl > 60.0 {
                b.w *= 60.0 / wl;
            }
            b.p += b.v * dt;
            b.q = b.q.integrate(b.w, dt);
            b.refresh_inertia();
        }
        self.sleep(dt);
    }

    fn apply(bodies: &mut [Body], a: usize, b: usize, ra: V3, rb: V3, p: V3) {
        let ba = &mut bodies[a];
        ba.v -= p * ba.inv_mass;
        ba.w -= ba.inv_i.mul_v(ra.cross(p));
        let bb = &mut bodies[b];
        bb.v += p * bb.inv_mass;
        bb.w += bb.inv_i.mul_v(rb.cross(p));
    }

    /// Islands of touching or joined dynamic bodies fall asleep together once all are still.
    fn sleep(&mut self, dt: f64) {
        let n = self.bodies.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while p[r] != r {
                r = p[r];
            }
            let mut c = i;
            while p[c] != r {
                let nx = p[c];
                p[c] = r;
                c = nx;
            }
            r
        }
        let mut links: Vec<(usize, usize)> = self.touching.iter().cloned().collect();
        links.extend(self.joints.iter().map(|j| (j.a, j.b)));
        for (a, b) in links {
            if self.bodies[a].is_dynamic() && self.bodies[b].is_dynamic() {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        for b in &mut self.bodies {
            if b.is_dynamic() && b.awake {
                if b.v.len() < SLEEP_LIN && b.w.len() < SLEEP_ANG {
                    b.sleep_t += dt;
                } else {
                    b.sleep_t = 0.0;
                }
            }
        }
        let mut min_t: BTreeMap<usize, f64> = BTreeMap::new();
        for i in 0..n {
            if self.bodies[i].is_dynamic() {
                let r = find(&mut parent, i);
                let t = if self.bodies[i].awake {
                    self.bodies[i].sleep_t
                } else {
                    f64::MAX
                };
                let e = min_t.entry(r).or_insert(f64::MAX);
                *e = e.min(t);
            }
        }
        for i in 0..n {
            if self.bodies[i].is_dynamic() && self.bodies[i].awake {
                let r = find(&mut parent, i);
                if min_t[&r] >= SLEEP_TIME && min_t[&r] != f64::MAX {
                    let b = &mut self.bodies[i];
                    b.awake = false;
                    b.v = V3::ZERO;
                    b.w = V3::ZERO;
                }
            }
        }
    }

    /// A hash of every body's pose and velocity (for determinism checks).
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |x: f64| {
            for byte in x.to_bits().to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for b in &self.bodies {
            for x in [
                b.p.x, b.p.y, b.p.z, b.q.w, b.q.x, b.q.y, b.q.z, b.v.x, b.v.y, b.v.z,
            ] {
                mix(x);
            }
        }
        h
    }
}

/// Make a convex hull body shape from points (body frame must be centred on the centre of mass:
/// see [`hull_centered`]).
pub fn hull_centered(pts: &[V3]) -> Option<(Hull, V3)> {
    let h = Hull::from_points(pts)?;
    let mp = h.mass_props();
    Some((h.translated(-mp.centroid), mp.centroid))
}
