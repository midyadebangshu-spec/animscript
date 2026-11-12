//! The world: bodies, fixtures, contacts, the soft-step solver and sleeping islands.
//!
//! One `step(dt)` runs: collide (broadphase, manifolds, persistent contacts, events), then for
//! each substep integrate velocities, warm start, solve joints and contacts with bias, integrate
//! positions and relax; finally restitution, impulse storage and island sleeping.
use crate::collide::*;
use crate::joint::*;
use crate::math::*;
use crate::shape::*;
use std::collections::{BTreeMap, BTreeSet};

pub type BodyId = usize;
pub type JointId = usize;

const SPEC_MARGIN: f64 = 0.02;
const TOUCH_BEGIN: f64 = 0.01;
const LINEAR_SLOP: f64 = 0.005;
const MAX_PUSH: f64 = 3.0;
const RESTITUTION_THRESHOLD: f64 = 1.0;
const SLEEP_LINEAR: f64 = 0.05;
const SLEEP_ANGULAR: f64 = 0.0349;
const TIME_TO_SLEEP: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Static,
    Dynamic,
    Kinematic,
}

#[derive(Clone, Debug)]
pub struct BodyDef {
    pub kind: BodyKind,
    pub position: Vec2,
    pub angle: f64,
    pub linear_velocity: Vec2,
    pub angular_velocity: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub gravity_scale: f64,
    pub fixed_rotation: bool,
    pub allow_sleep: bool,
}

impl Default for BodyDef {
    fn default() -> BodyDef {
        BodyDef {
            kind: BodyKind::Static,
            position: Vec2::ZERO,
            angle: 0.0,
            linear_velocity: Vec2::ZERO,
            angular_velocity: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            gravity_scale: 1.0,
            fixed_rotation: false,
            allow_sleep: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FixtureDef {
    pub shape: Shape,
    pub density: f64,
    pub friction: f64,
    pub restitution: f64,
    pub sensor: bool,
}

impl FixtureDef {
    pub fn new(shape: Shape) -> FixtureDef {
        FixtureDef {
            shape,
            density: 1.0,
            friction: 0.6,
            restitution: 0.0,
            sensor: false,
        }
    }
}

struct Fixture {
    body: BodyId,
    shape: Shape,
    friction: f64,
    restitution: f64,
    sensor: bool,
    density: f64,
}

#[derive(Clone, Debug)]
pub struct Body {
    pub kind: BodyKind,
    /// Origin of the body frame (shapes are defined relative to it).
    pub p: Vec2,
    pub q: Rot,
    /// World position of the centre of mass (what the solver moves).
    pub c: Vec2,
    pub local_center: Vec2,
    pub v: Vec2,
    pub w: f64,
    pub mass: f64,
    pub inertia: f64,
    pub inv_mass: f64,
    pub inv_i: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub gravity_scale: f64,
    pub fixed_rotation: bool,
    pub allow_sleep: bool,
    pub awake: bool,
    pub sleep_time: f64,
    sleep_group: u32,
    fixtures: Vec<usize>,
    dp: Vec2,
    dq: Rot,
}

impl Body {
    /// Static bodies never move; sleeping bodies are skipped until woken.
    pub fn is_active(&self) -> bool {
        self.kind != BodyKind::Static && self.awake
    }
}

#[derive(Clone, Debug)]
pub struct ContactEvent {
    pub a: BodyId,
    pub b: BodyId,
    pub begin: bool,
    pub sensor: bool,
}

/// Result of `World::circle_hit`.
#[derive(Clone, Debug)]
pub struct CircleHit {
    pub normal: Vec2,
    pub sep: f64,
    pub vel: Vec2,
    pub body: BodyId,
    pub point: Vec2,
}

/// A contact point for debug drawing and the snapshot.
#[derive(Clone, Debug)]
pub struct ContactInfo {
    pub a: BodyId,
    pub b: BodyId,
    pub point: Vec2,
    pub normal: Vec2,
    pub sep: f64,
    pub normal_impulse: f64,
}

struct Contact {
    ba: BodyId,
    bb: BodyId,
    manifold: Manifold,
    touching: bool,
    sensor: bool,
    friction: f64,
    restitution: f64,
}

struct Cp {
    ra: Vec2,
    rb: Vec2,
    base_sep: f64,
    nmass: f64,
    tmass: f64,
    relvel: f64,
    ni: f64,
    ti: f64,
    max_ni: f64,
}

struct Cc {
    key: (usize, usize),
    a: usize,
    b: usize,
    normal: Vec2,
    friction: f64,
    restitution: f64,
    soft: Softness,
    pts: Vec<Cp>,
}

pub(crate) fn two_mut<T>(s: &mut [T], a: usize, b: usize) -> (&mut T, &mut T) {
    assert!(a != b, "a constraint needs two different bodies");
    if a < b {
        let (l, r) = s.split_at_mut(b);
        (&mut l[a], &mut r[0])
    } else {
        let (l, r) = s.split_at_mut(a);
        (&mut r[0], &mut l[b])
    }
}

fn wake_body(bodies: &mut [Body], id: usize) {
    if bodies[id].awake || bodies[id].kind == BodyKind::Static {
        return;
    }
    let g = bodies[id].sleep_group;
    if g == 0 {
        bodies[id].awake = true;
        bodies[id].sleep_time = 0.0;
        return;
    }
    for b in bodies.iter_mut() {
        if b.sleep_group == g {
            b.awake = true;
            b.sleep_group = 0;
            b.sleep_time = 0.0;
        }
    }
}

/// Where a ray hit: the body, the world point, the outward surface normal and the distance.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub body: BodyId,
    pub point: Vec2,
    pub normal: Vec2,
    pub distance: f64,
}

/// Distance and local normal of the first hit of a ray (local space) with a shape, within `max`.
fn ray_shape(shape: &Shape, o: Vec2, d: Vec2, max: f64) -> Option<(f64, Vec2)> {
    match shape {
        Shape::Circle { center, radius } => {
            let m = o - *center;
            let b = m.dot(d);
            let c = m.length_sq() - radius * radius;
            if c <= 0.0 || b > 0.0 {
                return None; // starts inside, or points away
            }
            let disc = b * b - c;
            if disc < 0.0 {
                return None;
            }
            let t = -b - disc.sqrt();
            (0.0..=max)
                .contains(&t)
                .then(|| (t, (m + d * t).normalized()))
        }
        Shape::Polygon(p) => {
            let (mut enter, mut exit, mut normal) = (0.0f64, max, None);
            for i in 0..p.verts.len() {
                let (n, v) = (p.normals[i], p.verts[i]);
                let (denom, num) = (n.dot(d), n.dot(v - o));
                if denom == 0.0 {
                    if num < 0.0 {
                        return None;
                    }
                } else if denom < 0.0 && num < enter * denom {
                    enter = num / denom;
                    normal = Some(n);
                } else if denom > 0.0 && num < exit * denom {
                    exit = num / denom;
                }
                if exit < enter {
                    return None;
                }
            }
            normal.map(|n| (enter, n))
        }
        Shape::Edge { a, b } => {
            let e = *b - *a;
            let denom = d.cross(e);
            if denom.abs() < 1e-12 {
                return None;
            }
            let w = *a - o;
            let t = w.cross(e) / denom;
            let u = w.cross(d) / denom;
            if !(0.0..=max).contains(&t) || !(0.0..=1.0).contains(&u) {
                return None;
            }
            let n = v2(e.y, -e.x).normalized();
            Some((t, if n.dot(d) > 0.0 { -n } else { n }))
        }
    }
}

pub struct World {
    pub gravity: Vec2,
    pub substeps: usize,
    bodies: Vec<Body>,
    fixtures: Vec<Fixture>,
    joints: Vec<Joint>,
    contacts: BTreeMap<(usize, usize), Contact>,
    no_collide: BTreeSet<(usize, usize)>,
    touch_counts: BTreeMap<(usize, usize, bool), u32>,
    events: Vec<ContactEvent>,
    next_sleep_group: u32,
}

impl World {
    pub fn new(gravity: Vec2) -> World {
        World {
            gravity,
            substeps: 4,
            bodies: vec![],
            fixtures: vec![],
            joints: vec![],
            contacts: BTreeMap::new(),
            no_collide: BTreeSet::new(),
            touch_counts: BTreeMap::new(),
            events: vec![],
            next_sleep_group: 0,
        }
    }

    // ---- construction -----------------------------------------------------------------------

    pub fn create_body(&mut self, def: &BodyDef) -> BodyId {
        let q = Rot::from_angle(def.angle);
        let movable = def.kind != BodyKind::Static;
        self.bodies.push(Body {
            kind: def.kind,
            p: def.position,
            q,
            c: def.position,
            local_center: Vec2::ZERO,
            v: if movable {
                def.linear_velocity
            } else {
                Vec2::ZERO
            },
            w: if movable && !def.fixed_rotation {
                def.angular_velocity
            } else {
                0.0
            },
            mass: 0.0,
            inertia: 0.0,
            inv_mass: 0.0,
            inv_i: 0.0,
            linear_damping: def.linear_damping,
            angular_damping: def.angular_damping,
            gravity_scale: def.gravity_scale,
            fixed_rotation: def.fixed_rotation,
            allow_sleep: def.allow_sleep,
            awake: true,
            sleep_time: 0.0,
            sleep_group: 0,
            fixtures: vec![],
            dp: Vec2::ZERO,
            dq: Rot::IDENTITY,
        });
        self.bodies.len() - 1
    }

    pub fn add_fixture(&mut self, body: BodyId, def: &FixtureDef) -> usize {
        self.fixtures.push(Fixture {
            body,
            shape: def.shape.clone(),
            friction: def.friction,
            restitution: def.restitution,
            sensor: def.sensor,
            density: def.density,
        });
        let id = self.fixtures.len() - 1;
        self.bodies[body].fixtures.push(id);
        self.recompute_mass(body);
        id
    }

    fn recompute_mass(&mut self, id: BodyId) {
        let fixtures = &self.fixtures;
        let b = &mut self.bodies[id];
        b.mass = 0.0;
        b.inertia = 0.0;
        b.inv_mass = 0.0;
        b.inv_i = 0.0;
        b.local_center = Vec2::ZERO;
        if b.kind == BodyKind::Dynamic {
            let parts: Vec<MassData> = b
                .fixtures
                .iter()
                .filter(|&&f| !fixtures[f].sensor)
                .map(|&f| fixtures[f].shape.mass_data(fixtures[f].density))
                .collect();
            let mass: f64 = parts.iter().map(|m| m.mass).sum();
            if mass > 0.0 {
                let mut center = Vec2::ZERO;
                for m in &parts {
                    center += m.center * m.mass;
                }
                center = center / mass;
                let inertia: f64 = parts
                    .iter()
                    .map(|m| m.inertia + m.mass * (m.center - center).length_sq())
                    .sum();
                b.mass = mass;
                b.inertia = inertia;
                b.local_center = center;
                b.inv_mass = 1.0 / mass;
                b.inv_i = if inertia > 0.0 && !b.fixed_rotation {
                    1.0 / inertia
                } else {
                    0.0
                };
            } else {
                b.mass = 1.0;
                b.inv_mass = 1.0;
            }
        }
        b.c = b.p + b.q.rotate(b.local_center);
    }

    pub fn create_joint(&mut self, def: &JointDef) -> JointId {
        let joint = Joint::new(def, &self.bodies);
        if def.kind != JointKind::Target && !def.collide_connected {
            self.no_collide.insert((def.a.min(def.b), def.a.max(def.b)));
        }
        self.joints.push(joint);
        wake_body(&mut self.bodies, def.a);
        wake_body(&mut self.bodies, def.b);
        self.joints.len() - 1
    }

    // ---- access and control -----------------------------------------------------------------

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
    pub fn take_events(&mut self) -> Vec<ContactEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn wake(&mut self, id: BodyId) {
        wake_body(&mut self.bodies, id);
    }
    pub fn set_linear_velocity(&mut self, id: BodyId, v: Vec2) {
        wake_body(&mut self.bodies, id);
        if self.bodies[id].kind != BodyKind::Static {
            self.bodies[id].v = v;
        }
    }
    pub fn set_angular_velocity(&mut self, id: BodyId, w: f64) {
        wake_body(&mut self.bodies, id);
        let b = &mut self.bodies[id];
        if b.kind != BodyKind::Static && !b.fixed_rotation {
            b.w = w;
        }
    }
    pub fn apply_linear_impulse(&mut self, id: BodyId, impulse: Vec2) {
        wake_body(&mut self.bodies, id);
        let b = &mut self.bodies[id];
        b.v += impulse * b.inv_mass;
    }
    pub fn apply_angular_impulse(&mut self, id: BodyId, impulse: f64) {
        wake_body(&mut self.bodies, id);
        let b = &mut self.bodies[id];
        b.w += impulse * b.inv_i;
    }
    /// An impulse applied at a world point (also spins the body).
    pub fn apply_impulse_at(&mut self, id: BodyId, impulse: Vec2, point: Vec2) {
        wake_body(&mut self.bodies, id);
        let b = &mut self.bodies[id];
        b.v += impulse * b.inv_mass;
        b.w += b.inv_i * (point - b.c).cross(impulse);
    }
    /// Teleport a body (keeps its velocity).
    pub fn set_transform(&mut self, id: BodyId, p: Vec2, angle: f64) {
        wake_body(&mut self.bodies, id);
        let b = &mut self.bodies[id];
        b.p = p;
        b.q = Rot::from_angle(angle);
        b.c = b.p + b.q.rotate(b.local_center);
    }
    /// Give a kinematic body the velocity that carries it to `p`/`angle` in `dt` seconds.
    pub fn move_kinematic(&mut self, id: BodyId, p: Vec2, angle: f64, dt: f64) {
        wake_body(&mut self.bodies, id);
        let b = &mut self.bodies[id];
        b.v = (p - b.p) / dt;
        let mut da = angle - b.q.angle();
        let tau = std::f64::consts::TAU;
        da -= tau * (da / tau).round();
        b.w = da / dt;
    }
    pub fn set_joint_target(&mut self, id: JointId, target: Vec2) {
        let b = self.joints[id].b;
        self.joints[id].target = target;
        wake_body(&mut self.bodies, b);
    }
    /// Aim a revolute joint's angle spring (see `JointDef::angle_spring`) at `angle` radians.
    pub fn set_joint_angle_target(&mut self, id: JointId, angle: f64) {
        let b = self.joints[id].b;
        self.joints[id].ang_target = angle;
        wake_body(&mut self.bodies, b);
    }
    /// Switch a revolute joint's angle spring on (`Some((hertz, damping ratio, max torque))`) or off.
    pub fn set_joint_angle_spring(&mut self, id: JointId, spring: Option<(f64, f64, f64)>) {
        let b = self.joints[id].b;
        self.joints[id].set_angle_spring(spring);
        wake_body(&mut self.bodies, b);
    }
    /// Change what kind a body is, keeping its pose and velocity (mass is recomputed).
    pub fn set_body_kind(&mut self, id: BodyId, kind: BodyKind) {
        self.bodies[id].kind = kind;
        if kind == BodyKind::Static {
            self.bodies[id].v = Vec2::ZERO;
            self.bodies[id].w = 0.0;
        }
        self.recompute_mass(id);
        wake_body(&mut self.bodies, id);
    }
    /// Stop two bodies from colliding with each other.
    pub fn disable_collision(&mut self, a: BodyId, b: BodyId) {
        self.no_collide.insert((a.min(b), a.max(b)));
    }
    /// The nearest solid surface along a ray from `origin` in unit direction `dir`, within `max`
    /// metres. Sensors are ignored, a ray that starts inside a shape does not hit it, and
    /// `exclude` skips one body.
    pub fn raycast(
        &self,
        origin: Vec2,
        dir: Vec2,
        max: f64,
        exclude: Option<BodyId>,
    ) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        for f in &self.fixtures {
            if f.sensor || exclude == Some(f.body) {
                continue;
            }
            let b = &self.bodies[f.body];
            let xf = Xf { p: b.p, q: b.q };
            let (o, d) = (b.q.inv_rotate(origin - b.p), b.q.inv_rotate(dir));
            let limit = best.map_or(max, |h| h.distance);
            if let Some((t, n)) = ray_shape(&f.shape, o, d, limit) {
                if best.is_none_or(|h| t < h.distance) {
                    best = Some(RayHit {
                        body: f.body,
                        point: origin + dir * t,
                        normal: xf.q.rotate(n),
                        distance: t,
                    });
                }
            }
        }
        best
    }
    pub fn joint_anchors(&self, id: JointId) -> (Vec2, Vec2) {
        self.joints[id].anchors(&self.bodies)
    }
    /// Pairs of bodies that already interpenetrate by more than `depth` (metres) and are not
    /// joined; used to warn about a scene that starts with overlapping bodies.
    pub fn overlapping_pairs(&self, depth: f64) -> Vec<(BodyId, BodyId)> {
        let boxes: Vec<Aabb> = self
            .fixtures
            .iter()
            .map(|f| {
                f.shape.aabb(&Xf {
                    p: self.bodies[f.body].p,
                    q: self.bodies[f.body].q,
                })
            })
            .collect();
        let mut out = BTreeSet::new();
        for (fa, fb) in find_pairs(&boxes) {
            let (ia, ib) = (self.fixtures[fa].body, self.fixtures[fb].body);
            if ia == ib
                || self.fixtures[fa].sensor
                || self.fixtures[fb].sensor
                || self.no_collide.contains(&(ia.min(ib), ia.max(ib)))
            {
                continue;
            }
            if self.bodies[ia].kind != BodyKind::Dynamic
                && self.bodies[ib].kind != BodyKind::Dynamic
            {
                continue;
            }
            let xa = Xf {
                p: self.bodies[ia].p,
                q: self.bodies[ia].q,
            };
            let xb = Xf {
                p: self.bodies[ib].p,
                q: self.bodies[ib].q,
            };
            if collide(
                &self.fixtures[fa].shape,
                &xa,
                &self.fixtures[fb].shape,
                &xb,
                0.0,
            )
            .is_some_and(|m| m.points.iter().any(|p| p.sep < -depth))
            {
                out.insert((ia.min(ib), ia.max(ib)));
            }
        }
        out.into_iter().collect()
    }
    /// The nearest non-sensor fixture a circle (centre and radius in metres) touches or will touch
    /// within `margin`: `(normal pointing from the circle into the fixture, separation, velocity of
    /// the fixture at the contact point)`. Used by particles, which never push anything back.
    pub fn collide_circle(
        &self,
        center: Vec2,
        radius: f64,
        margin: f64,
    ) -> Option<(Vec2, f64, Vec2)> {
        self.circle_hit(center, radius, margin)
            .map(|h| (h.normal, h.sep, h.vel))
    }

    /// True when any non-sensor fixture's bounding box overlaps the given box (a cheap pre-test).
    pub fn near_fixture(&self, lo: Vec2, hi: Vec2) -> bool {
        let q = Aabb { lo, hi };
        self.fixtures.iter().filter(|f| !f.sensor).any(|f| {
            let b = &self.bodies[f.body];
            f.shape.aabb(&Xf { p: b.p, q: b.q }).overlaps(&q)
        })
    }

    /// Resolve the approach of dynamic body `id` towards an immovable obstacle point: remove the
    /// velocity of the body's contact point along `n` (pointing from the obstacle into the body)
    /// relative to the obstacle's own velocity. Used for pinned soft vertices, which cannot move away.
    pub fn bounce_at(&mut self, id: BodyId, point: Vec2, n: Vec2, obstacle_vel: Vec2) {
        let b = &self.bodies[id];
        if b.kind != BodyKind::Dynamic {
            return;
        }
        let r = point - b.c;
        let vn = (b.v + cross_sv(b.w, r) - obstacle_vel).dot(n);
        if vn >= 0.0 {
            return;
        }
        let rn = r.cross(n);
        let denom = b.inv_mass + b.inv_i * rn * rn;
        if denom <= 0.0 {
            return;
        }
        self.apply_impulse_at(id, n * (-vn / denom), point);
    }

    /// Like `collide_circle`, but also says which body was hit and where (soft bodies push back).
    pub fn circle_hit(&self, center: Vec2, radius: f64, margin: f64) -> Option<CircleHit> {
        self.circle_hits(center, radius, margin)
            .into_iter()
            .min_by(|a, b| a.sep.total_cmp(&b.sep))
    }

    /// Every non-sensor fixture the circle touches or will touch within `margin`, one hit per
    /// fixture (its deepest point), in fixture order.
    pub fn circle_hits(&self, center: Vec2, radius: f64, margin: f64) -> Vec<CircleHit> {
        let circle = Shape::Circle {
            center: Vec2::ZERO,
            radius,
        };
        let xc = Xf {
            p: center,
            q: Rot::IDENTITY,
        };
        let mut out: Vec<CircleHit> = vec![];
        for f in self.fixtures.iter().filter(|f| !f.sensor) {
            let b = &self.bodies[f.body];
            let xf = Xf { p: b.p, q: b.q };
            let r = radius + margin;
            let bb = f.shape.aabb(&xf);
            if center.x < bb.lo.x - r
                || center.x > bb.hi.x + r
                || center.y < bb.lo.y - r
                || center.y > bb.hi.y + r
            {
                continue;
            }
            let Some(m) = collide(&circle, &xc, &f.shape, &xf, margin) else {
                continue;
            };
            if let Some(pt) = m.points.iter().min_by(|a, b| a.sep.total_cmp(&b.sep)) {
                out.push(CircleHit {
                    normal: m.normal,
                    sep: pt.sep,
                    vel: b.v + cross_sv(b.w, pt.point - b.c),
                    body: f.body,
                    point: pt.point,
                });
            }
        }
        out
    }

    /// All contact points of touching, non-sensor contacts (for the snapshot and debug drawing).
    pub fn contact_infos(&self) -> Vec<ContactInfo> {
        let mut out = vec![];
        for c in self.contacts.values().filter(|c| c.touching && !c.sensor) {
            for p in &c.manifold.points {
                out.push(ContactInfo {
                    a: c.ba,
                    b: c.bb,
                    point: p.point,
                    normal: c.manifold.normal,
                    sep: p.sep,
                    normal_impulse: p.normal_impulse,
                });
            }
        }
        out
    }

    // ---- stepping ---------------------------------------------------------------------------

    pub fn step(&mut self, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let n = self.substeps.max(1);
        let h = dt / n as f64;
        self.update_contacts(dt);
        for b in &mut self.bodies {
            b.dp = Vec2::ZERO;
            b.dq = Rot::IDENTITY;
        }
        let mut cons = self.prepare_contacts(h);
        let hz = 30f64.min(0.25 / h);
        let main = Ctx {
            h,
            inv_h: 1.0 / h,
            soft: make_soft(2.0 * hz, 2.0, h),
            use_bias: true,
        };
        let relax = Ctx {
            use_bias: false,
            ..main_copy(&main)
        };
        let active: Vec<usize> = (0..self.joints.len())
            .filter(|&i| {
                self.bodies[self.joints[i].a].is_active()
                    || self.bodies[self.joints[i].b].is_active()
            })
            .collect();
        for _ in 0..n {
            self.integrate_velocities(h);
            for &i in &active {
                self.joints[i].warm_start(&mut self.bodies);
            }
            warm_start_contacts(&cons, &mut self.bodies);
            for &i in &active {
                self.joints[i].solve(&mut self.bodies, &main);
            }
            solve_contacts(&mut cons, &mut self.bodies, h, true);
            self.integrate_positions(h);
            for &i in &active {
                self.joints[i].solve(&mut self.bodies, &relax);
            }
            solve_contacts(&mut cons, &mut self.bodies, h, false);
        }
        apply_restitution(&mut cons, &mut self.bodies);
        for cc in &cons {
            let c = self
                .contacts
                .get_mut(&cc.key)
                .expect("a solved contact exists");
            for (mp, cp) in c.manifold.points.iter_mut().zip(&cc.pts) {
                mp.normal_impulse = cp.ni;
                mp.tangent_impulse = cp.ti;
            }
        }
        // a contact the solver pushed on is touching, even when the impact happened inside this step
        let hit: Vec<(usize, usize)> = self
            .contacts
            .iter()
            .filter(|(_, c)| {
                !c.sensor && !c.touching && c.manifold.points.iter().any(|p| p.normal_impulse > 0.0)
            })
            .map(|(k, _)| *k)
            .collect();
        for k in hit {
            let c = self.contacts.get_mut(&k).expect("listed");
            c.touching = true;
            let (a, b) = (c.ba, c.bb);
            self.touch_change(a, b, false, true);
        }
        for b in &mut self.bodies {
            if b.is_active() {
                b.p = b.c - b.q.rotate(b.local_center);
            }
        }
        self.update_sleep(dt);
    }

    fn integrate_velocities(&mut self, h: f64) {
        for b in &mut self.bodies {
            if !b.awake || b.kind != BodyKind::Dynamic {
                continue;
            }
            b.v += self.gravity * (h * b.gravity_scale);
            b.v = b.v * (1.0 / (1.0 + h * b.linear_damping));
            b.w *= 1.0 / (1.0 + h * b.angular_damping);
            if b.fixed_rotation {
                b.w = 0.0;
            }
        }
    }

    fn integrate_positions(&mut self, h: f64) {
        for b in &mut self.bodies {
            if !b.is_active() {
                continue;
            }
            b.c += b.v * h;
            b.dp += b.v * h;
            let da = h * b.w;
            b.q = b.q.integrate(da);
            b.dq = b.dq.integrate(da);
        }
    }

    // ---- collision --------------------------------------------------------------------------

    fn update_contacts(&mut self, dt: f64) {
        let boxes: Vec<Aabb> = self
            .fixtures
            .iter()
            .map(|f| {
                let b = &self.bodies[f.body];
                let speed = if b.is_active() { b.v.length() } else { 0.0 };
                f.shape
                    .aabb(&Xf { p: b.p, q: b.q })
                    .expanded(SPEC_MARGIN + speed * dt)
            })
            .collect();
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
        for (fa, fb) in find_pairs(&boxes) {
            let (ia, ib) = (self.fixtures[fa].body, self.fixtures[fb].body);
            if ia == ib {
                continue;
            }
            if self.bodies[ia].kind != BodyKind::Dynamic
                && self.bodies[ib].kind != BodyKind::Dynamic
            {
                continue;
            }
            if self.no_collide.contains(&(ia.min(ib), ia.max(ib))) {
                continue;
            }
            let sensor = self.fixtures[fa].sensor || self.fixtures[fb].sensor;
            if self.fixtures[fa].sensor && self.fixtures[fb].sensor {
                continue;
            }
            let key = (fa, fb);
            seen.insert(key);
            if !self.bodies[ia].is_active() && !self.bodies[ib].is_active() {
                continue; // both asleep: the contact stays as it was
            }
            let speed = |b: &Body| if b.is_active() { b.v.length() } else { 0.0 };
            let margin = if sensor {
                0.0
            } else {
                SPEC_MARGIN + dt * (speed(&self.bodies[ia]) + speed(&self.bodies[ib]))
            };
            let xa = Xf {
                p: self.bodies[ia].p,
                q: self.bodies[ia].q,
            };
            let xb = Xf {
                p: self.bodies[ib].p,
                q: self.bodies[ib].q,
            };
            let found = collide(
                &self.fixtures[fa].shape,
                &xa,
                &self.fixtures[fb].shape,
                &xb,
                margin,
            );
            let existing = self.contacts.remove(&key);
            match found {
                None => {
                    if existing.is_some_and(|c| c.touching) {
                        self.touch_change(ia, ib, sensor, false);
                    }
                }
                Some(mut m) => {
                    let mut c = existing.unwrap_or(Contact {
                        ba: ia,
                        bb: ib,
                        manifold: Manifold {
                            normal: m.normal,
                            points: vec![],
                        },
                        touching: false,
                        sensor,
                        friction: 0.0,
                        restitution: 0.0,
                    });
                    for p in &mut m.points {
                        if let Some(o) = c.manifold.points.iter().find(|o| o.id == p.id) {
                            p.normal_impulse = o.normal_impulse;
                            p.tangent_impulse = o.tangent_impulse;
                        }
                    }
                    let min_sep = m.points.iter().map(|p| p.sep).fold(f64::MAX, f64::min);
                    c.manifold = m;
                    c.friction = (self.fixtures[fa].friction * self.fixtures[fb].friction).sqrt();
                    c.restitution = self.fixtures[fa]
                        .restitution
                        .max(self.fixtures[fb].restitution);
                    let was = c.touching;
                    let now = if sensor {
                        min_sep <= 0.0
                    } else if was {
                        min_sep <= SPEC_MARGIN
                    } else {
                        min_sep <= TOUCH_BEGIN
                    };
                    c.touching = now;
                    if was != now {
                        self.touch_change(ia, ib, sensor, now);
                    }
                    if now && !sensor {
                        // touching a sleeper wakes its island
                        let (a_act, b_act) =
                            (self.bodies[ia].is_active(), self.bodies[ib].is_active());
                        if a_act && !b_act {
                            wake_body(&mut self.bodies, ib);
                        } else if b_act && !a_act {
                            wake_body(&mut self.bodies, ia);
                        }
                    }
                    self.contacts.insert(key, c);
                }
            }
        }
        let gone: Vec<(usize, usize)> = self
            .contacts
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect();
        for k in gone {
            let c = self.contacts.remove(&k).expect("listed");
            if c.touching {
                self.touch_change(c.ba, c.bb, c.sensor, false);
            }
        }
    }

    /// Body-pair events: one `begin` when the first contact between two bodies starts touching and
    /// one `end` when the last one stops.
    fn touch_change(&mut self, a: BodyId, b: BodyId, sensor: bool, begin: bool) {
        let key = (a.min(b), a.max(b), sensor);
        let count = self.touch_counts.entry(key).or_insert(0);
        if begin {
            *count += 1;
            if *count == 1 {
                self.events.push(ContactEvent {
                    a: key.0,
                    b: key.1,
                    begin: true,
                    sensor,
                });
            }
        } else {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.touch_counts.remove(&key);
                self.events.push(ContactEvent {
                    a: key.0,
                    b: key.1,
                    begin: false,
                    sensor,
                });
            }
        }
    }

    fn prepare_contacts(&self, h: f64) -> Vec<Cc> {
        let hz = 30f64.min(0.25 / h);
        let soft_dyn = make_soft(hz, 10.0, h);
        let soft_static = make_soft(2.0 * hz, 10.0, h);
        let mut out = vec![];
        for (key, c) in &self.contacts {
            if c.sensor || c.manifold.points.is_empty() {
                continue;
            }
            let (ba, bb) = (&self.bodies[c.ba], &self.bodies[c.bb]);
            if !ba.is_active() && !bb.is_active() {
                continue;
            }
            let n = c.manifold.normal;
            let t = v2(n.y, -n.x);
            let mut pts = vec![];
            for mp in &c.manifold.points {
                let ra = mp.point - ba.c;
                let rb = mp.point - bb.c;
                let kn = ba.inv_mass
                    + bb.inv_mass
                    + ba.inv_i * ra.cross(n).powi(2)
                    + bb.inv_i * rb.cross(n).powi(2);
                let kt = ba.inv_mass
                    + bb.inv_mass
                    + ba.inv_i * ra.cross(t).powi(2)
                    + bb.inv_i * rb.cross(t).powi(2);
                let vrel = (bb.v + cross_sv(bb.w, rb)) - (ba.v + cross_sv(ba.w, ra));
                pts.push(Cp {
                    ra,
                    rb,
                    base_sep: mp.sep - n.dot(rb - ra),
                    nmass: if kn > 0.0 { 1.0 / kn } else { 0.0 },
                    tmass: if kt > 0.0 { 1.0 / kt } else { 0.0 },
                    relvel: n.dot(vrel),
                    ni: mp.normal_impulse,
                    ti: mp.tangent_impulse,
                    max_ni: 0.0,
                });
            }
            let soft = if ba.inv_mass == 0.0 || bb.inv_mass == 0.0 {
                soft_static
            } else {
                soft_dyn
            };
            out.push(Cc {
                key: *key,
                a: c.ba,
                b: c.bb,
                normal: n,
                friction: c.friction,
                restitution: c.restitution,
                soft,
                pts,
            });
        }
        out
    }

    // ---- sleeping ---------------------------------------------------------------------------

    fn update_sleep(&mut self, dt: f64) {
        let n = self.bodies.len();
        for b in &mut self.bodies {
            if !b.is_active() {
                continue;
            }
            if !b.allow_sleep
                || b.v.length_sq() > SLEEP_LINEAR * SLEEP_LINEAR
                || b.w.abs() > SLEEP_ANGULAR
            {
                b.sleep_time = 0.0;
            } else {
                b.sleep_time += dt;
            }
        }
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        let mut links: Vec<(usize, usize)> = vec![];
        for c in self.contacts.values().filter(|c| c.touching && !c.sensor) {
            links.push((c.ba, c.bb));
        }
        for j in &self.joints {
            if j.kind != JointKind::Target {
                links.push((j.a, j.b));
            }
        }
        for (a, b) in links {
            if self.bodies[a].is_active() && self.bodies[b].is_active() {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        let mut min_t = vec![f64::MAX; n];
        for i in 0..n {
            if self.bodies[i].is_active() {
                let r = find(&mut parent, i);
                min_t[r] = min_t[r].min(self.bodies[i].sleep_time);
            }
        }
        let mut groups: BTreeMap<usize, u32> = BTreeMap::new();
        for i in 0..n {
            if !self.bodies[i].is_active() {
                continue;
            }
            let r = find(&mut parent, i);
            if min_t[r] >= TIME_TO_SLEEP {
                let next = &mut self.next_sleep_group;
                let g = *groups.entry(r).or_insert_with(|| {
                    *next += 1;
                    *next
                });
                let b = &mut self.bodies[i];
                b.awake = false;
                b.sleep_group = g;
                b.v = Vec2::ZERO;
                b.w = 0.0;
            }
        }
    }
}

fn main_copy(c: &Ctx) -> Ctx {
    Ctx {
        h: c.h,
        inv_h: c.inv_h,
        soft: c.soft,
        use_bias: c.use_bias,
    }
}

// ---- contact solver -------------------------------------------------------------------------

fn warm_start_contacts(cons: &[Cc], bodies: &mut [Body]) {
    for cc in cons {
        let (ba, bb) = two_mut(bodies, cc.a, cc.b);
        let n = cc.normal;
        let t = v2(n.y, -n.x);
        for p in &cc.pts {
            let imp = n * p.ni + t * p.ti;
            ba.v -= imp * ba.inv_mass;
            ba.w -= ba.inv_i * p.ra.cross(imp);
            bb.v += imp * bb.inv_mass;
            bb.w += bb.inv_i * p.rb.cross(imp);
        }
    }
}

fn solve_contacts(cons: &mut [Cc], bodies: &mut [Body], h: f64, use_bias: bool) {
    let inv_h = 1.0 / h;
    for cc in cons.iter_mut() {
        let (ba, bb) = two_mut(bodies, cc.a, cc.b);
        let n = cc.normal;
        let t = v2(n.y, -n.x);
        let dp = bb.dp - ba.dp;
        // friction first, so that the normal constraint has the last word
        for p in &mut cc.pts {
            let dv = (bb.v + cross_sv(bb.w, p.rb)) - (ba.v + cross_sv(ba.w, p.ra));
            let vt = dv.dot(t);
            let max = cc.friction * p.ni;
            let new = (p.ti - p.tmass * vt).clamp(-max, max);
            let impulse = new - p.ti;
            p.ti = new;
            let imp = t * impulse;
            ba.v -= imp * ba.inv_mass;
            ba.w -= ba.inv_i * p.ra.cross(imp);
            bb.v += imp * bb.inv_mass;
            bb.w += bb.inv_i * p.rb.cross(imp);
        }
        for p in &mut cc.pts {
            let prb = bb.dq.rotate(p.rb);
            let pra = ba.dq.rotate(p.ra);
            let s = (dp + (prb - pra)).dot(n) + p.base_sep;
            let (bias, ms, is) = if s > 0.0 {
                (s * inv_h, 1.0, 0.0)
            } else if use_bias {
                (
                    (cc.soft.bias_rate * (s + LINEAR_SLOP).min(0.0)).max(-MAX_PUSH),
                    cc.soft.mass_scale,
                    cc.soft.impulse_scale,
                )
            } else {
                (0.0, 1.0, 0.0)
            };
            let dv = (bb.v + cross_sv(bb.w, p.rb)) - (ba.v + cross_sv(ba.w, p.ra));
            let vn = dv.dot(n);
            let new = (p.ni - p.nmass * ms * (vn + bias) - is * p.ni).max(0.0);
            let impulse = new - p.ni;
            p.ni = new;
            p.max_ni = p.max_ni.max(new);
            let imp = n * impulse;
            ba.v -= imp * ba.inv_mass;
            ba.w -= ba.inv_i * p.ra.cross(imp);
            bb.v += imp * bb.inv_mass;
            bb.w += bb.inv_i * p.rb.cross(imp);
        }
    }
}

fn apply_restitution(cons: &mut [Cc], bodies: &mut [Body]) {
    for cc in cons.iter_mut() {
        if cc.restitution == 0.0 {
            continue;
        }
        let (ba, bb) = two_mut(bodies, cc.a, cc.b);
        let n = cc.normal;
        for p in &mut cc.pts {
            if p.relvel > -RESTITUTION_THRESHOLD || p.max_ni == 0.0 {
                continue;
            }
            let dv = (bb.v + cross_sv(bb.w, p.rb)) - (ba.v + cross_sv(ba.w, p.ra));
            let vn = dv.dot(n);
            let new = (p.ni - p.nmass * (vn + cc.restitution * p.relvel)).max(0.0);
            let impulse = new - p.ni;
            p.ni = new;
            p.max_ni = p.max_ni.max(new);
            let imp = n * impulse;
            ba.v -= imp * ba.inv_mass;
            ba.w -= ba.inv_i * p.ra.cross(imp);
            bb.v += imp * bb.inv_mass;
            bb.w += bb.inv_i * p.rb.cross(imp);
        }
    }
}
