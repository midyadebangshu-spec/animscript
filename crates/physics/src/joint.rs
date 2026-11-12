//! Joints and springs, solved as soft constraints (the same scheme as contacts): every joint
//! keeps accumulated impulses that are warm started on each substep, and corrects positional
//! drift through a bias velocity during the main pass but not the relax pass.
use crate::math::*;
use crate::world::{two_mut, Body, BodyId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    Revolute,
    Distance,
    Rope,
    Prismatic,
    Weld,
    Target,
}

/// Anchors are given in world space at creation; the joint remembers them in each body's frame.
#[derive(Clone, Debug)]
pub struct JointDef {
    pub kind: JointKind,
    pub a: BodyId,
    pub b: BodyId,
    pub anchor_a: Vec2,
    pub anchor_b: Vec2,
    pub axis: Vec2,
    pub target: Vec2,
    pub length: Option<f64>,
    pub spring: Option<(f64, f64)>,
    pub lower: f64,
    pub upper: f64,
    pub enable_limit: bool,
    pub motor: Option<(f64, f64)>,
    pub max_force: f64,
    pub collide_connected: bool,
}

impl JointDef {
    fn base(kind: JointKind, a: BodyId, b: BodyId, anchor_a: Vec2, anchor_b: Vec2) -> JointDef {
        JointDef {
            kind,
            a,
            b,
            anchor_a,
            anchor_b,
            axis: v2(1.0, 0.0),
            target: Vec2::ZERO,
            length: None,
            spring: None,
            lower: 0.0,
            upper: 0.0,
            enable_limit: false,
            motor: None,
            max_force: 1e9,
            collide_connected: false,
        }
    }
    pub fn revolute(a: BodyId, b: BodyId, anchor: Vec2) -> JointDef {
        Self::base(JointKind::Revolute, a, b, anchor, anchor)
    }
    pub fn weld(a: BodyId, b: BodyId, anchor: Vec2) -> JointDef {
        Self::base(JointKind::Weld, a, b, anchor, anchor)
    }
    pub fn prismatic(a: BodyId, b: BodyId, anchor: Vec2, axis: Vec2) -> JointDef {
        JointDef {
            axis,
            ..Self::base(JointKind::Prismatic, a, b, anchor, anchor)
        }
    }
    pub fn distance(a: BodyId, b: BodyId, anchor_a: Vec2, anchor_b: Vec2) -> JointDef {
        Self::base(JointKind::Distance, a, b, anchor_a, anchor_b)
    }
    pub fn rope(a: BodyId, b: BodyId, anchor_a: Vec2, anchor_b: Vec2, max: f64) -> JointDef {
        JointDef {
            upper: max,
            ..Self::base(JointKind::Rope, a, b, anchor_a, anchor_b)
        }
    }
    /// Pull `anchor` (a world point on `body`) towards `target` like a spring.
    pub fn target(body: BodyId, anchor: Vec2, target: Vec2) -> JointDef {
        JointDef {
            target,
            spring: Some((5.0, 0.7)),
            max_force: 1e3,
            ..Self::base(JointKind::Target, body, body, anchor, anchor)
        }
    }
    pub fn length(mut self, l: f64) -> JointDef {
        self.length = Some(l);
        self
    }
    /// Make a distance joint (or target) soft: `frequency` in Hz, `damping` as a ratio.
    pub fn spring(mut self, frequency: f64, damping: f64) -> JointDef {
        self.spring = Some((frequency, damping));
        self
    }
    /// Angle limits (revolute, radians) or translation limits (prismatic, metres).
    pub fn limits(mut self, lower: f64, upper: f64) -> JointDef {
        self.lower = lower;
        self.upper = upper;
        self.enable_limit = true;
        self
    }
    pub fn motor(mut self, speed: f64, max_torque: f64) -> JointDef {
        self.motor = Some((speed, max_torque));
        self
    }
    pub fn force(mut self, max_force: f64) -> JointDef {
        self.max_force = max_force;
        self
    }
    pub fn collide_connected(mut self, yes: bool) -> JointDef {
        self.collide_connected = yes;
        self
    }
}

#[derive(Clone, Copy)]
pub struct Softness {
    pub bias_rate: f64,
    pub mass_scale: f64,
    pub impulse_scale: f64,
}

pub fn make_soft(hertz: f64, zeta: f64, h: f64) -> Softness {
    if hertz <= 0.0 {
        return Softness {
            bias_rate: 0.0,
            mass_scale: 1.0,
            impulse_scale: 0.0,
        };
    }
    let omega = 2.0 * std::f64::consts::PI * hertz;
    let a1 = 2.0 * zeta + h * omega;
    let a2 = h * omega * a1;
    let a3 = 1.0 / (1.0 + a2);
    Softness {
        bias_rate: omega / a1,
        mass_scale: a2 * a3,
        impulse_scale: a3,
    }
}

pub struct Ctx {
    pub h: f64,
    pub inv_h: f64,
    pub soft: Softness,
    pub use_bias: bool,
}

impl Ctx {
    /// (bias, mass scale, impulse scale) for a position error `c` on a bilateral constraint.
    fn bilateral(&self, c: f64) -> (f64, f64, f64) {
        if self.use_bias {
            (
                self.soft.bias_rate * c,
                self.soft.mass_scale,
                self.soft.impulse_scale,
            )
        } else {
            (0.0, 1.0, 0.0)
        }
    }
    /// The same for a one-sided limit: speculative while there is slack.
    fn limit(&self, c: f64) -> (f64, f64, f64) {
        if c > 0.0 {
            (c * self.inv_h, 1.0, 0.0)
        } else {
            self.bilateral(c)
        }
    }
}

pub struct Joint {
    pub kind: JointKind,
    pub a: BodyId,
    pub b: BodyId,
    pub collide_connected: bool,
    la: Vec2,
    lb: Vec2,
    laxis: Vec2,
    ref_q: Rot,
    rest: f64,
    lower: f64,
    upper: f64,
    enable_limit: bool,
    motor: Option<(f64, f64)>,
    spring: Option<(f64, f64)>,
    max_force: f64,
    pub target: Vec2,
    // accumulated impulses
    lin: Vec2,
    ang: f64,
    imp: f64,
    lower_imp: f64,
    upper_imp: f64,
    motor_imp: f64,
    perp: Vec2,
}

fn solve22(k11: f64, k12: f64, k22: f64, b: Vec2) -> Vec2 {
    let det = k11 * k22 - k12 * k12;
    if det.abs() < 1e-300 {
        return Vec2::ZERO;
    }
    let inv = 1.0 / det;
    v2(inv * (k22 * b.x - k12 * b.y), inv * (k11 * b.y - k12 * b.x))
}

fn rel_angle(ba: &Body, bb: &Body, ref_q: Rot) -> f64 {
    let d = ref_q.inv_mul(ba.q.inv_mul(bb.q));
    d.s.atan2(d.c)
}

/// Apply impulse `p` to B and `-p` to A at the anchors `ra`, `rb` (relative to the centres).
fn apply(ba: &mut Body, bb: &mut Body, ra: Vec2, rb: Vec2, p: Vec2) {
    ba.v -= p * ba.inv_mass;
    ba.w -= ba.inv_i * ra.cross(p);
    bb.v += p * bb.inv_mass;
    bb.w += bb.inv_i * rb.cross(p);
}

fn solve_point(lin: &mut Vec2, ba: &mut Body, bb: &mut Body, la: Vec2, lb: Vec2, ctx: &Ctx) {
    let ra = ba.q.rotate(la - ba.local_center);
    let rb = bb.q.rotate(lb - bb.local_center);
    let cdot = (bb.v + cross_sv(bb.w, rb)) - (ba.v + cross_sv(ba.w, ra));
    let (bias, ms, is) = if ctx.use_bias {
        let c = (bb.c + rb) - (ba.c + ra);
        (
            c * ctx.soft.bias_rate,
            ctx.soft.mass_scale,
            ctx.soft.impulse_scale,
        )
    } else {
        (Vec2::ZERO, 1.0, 0.0)
    };
    let (ma, ia, mb, ib) = (ba.inv_mass, ba.inv_i, bb.inv_mass, bb.inv_i);
    let k11 = ma + mb + ra.y * ra.y * ia + rb.y * rb.y * ib;
    let k12 = -ra.y * ra.x * ia - rb.y * rb.x * ib;
    let k22 = ma + mb + ra.x * ra.x * ia + rb.x * rb.x * ib;
    let b = solve22(k11, k12, k22, cdot + bias);
    let impulse = -(b * ms) - *lin * is;
    *lin += impulse;
    apply(ba, bb, ra, rb, impulse);
}

impl Joint {
    pub fn new(def: &JointDef, bodies: &[Body]) -> Joint {
        let (ba, bb) = (&bodies[def.a], &bodies[def.b]);
        let la = ba.q.inv_rotate(def.anchor_a - ba.p);
        let lb = bb.q.inv_rotate(def.anchor_b - bb.p);
        let rest = def.length.unwrap_or((def.anchor_b - def.anchor_a).length());
        Joint {
            kind: def.kind,
            a: def.a,
            b: def.b,
            collide_connected: def.collide_connected,
            la,
            lb,
            laxis: ba.q.inv_rotate(def.axis).normalized(),
            ref_q: ba.q.inv_mul(bb.q),
            rest,
            lower: def.lower,
            upper: def.upper,
            enable_limit: def.enable_limit,
            motor: def.motor,
            spring: def.spring,
            max_force: def.max_force,
            target: def.target,
            lin: Vec2::ZERO,
            ang: 0.0,
            imp: 0.0,
            lower_imp: 0.0,
            upper_imp: 0.0,
            motor_imp: 0.0,
            perp: Vec2::ZERO,
        }
    }

    /// The two anchor points in world space (for drawing).
    pub fn anchors(&self, bodies: &[Body]) -> (Vec2, Vec2) {
        let (ba, bb) = (&bodies[self.a], &bodies[self.b]);
        (ba.p + ba.q.rotate(self.la), bb.p + bb.q.rotate(self.lb))
    }

    pub fn warm_start(&mut self, bodies: &mut [Body]) {
        if self.kind == JointKind::Target {
            let bb = &mut bodies[self.b];
            let rb = bb.q.rotate(self.lb - bb.local_center);
            bb.v += self.lin * bb.inv_mass;
            bb.w += bb.inv_i * rb.cross(self.lin);
            return;
        }
        let (ba, bb) = two_mut(bodies, self.a, self.b);
        let ra = ba.q.rotate(self.la - ba.local_center);
        let rb = bb.q.rotate(self.lb - bb.local_center);
        match self.kind {
            JointKind::Revolute => {
                let axial = self.motor_imp + self.lower_imp - self.upper_imp;
                apply(ba, bb, ra, rb, self.lin);
                ba.w -= ba.inv_i * axial;
                bb.w += bb.inv_i * axial;
            }
            JointKind::Weld => {
                apply(ba, bb, ra, rb, self.lin);
                ba.w -= ba.inv_i * self.ang;
                bb.w += bb.inv_i * self.ang;
            }
            JointKind::Distance | JointKind::Rope => {
                let d = (bb.c + rb) - (ba.c + ra);
                let axis = d.normalized();
                apply(
                    ba,
                    bb,
                    ra,
                    rb,
                    axis * (self.imp + self.lower_imp - self.upper_imp),
                );
            }
            JointKind::Prismatic => {
                let d = (bb.c - ba.c) + rb - ra;
                let axis = ba.q.rotate(self.laxis);
                let perp = axis.perp();
                let (a1, a2) = ((d + ra).cross(axis), rb.cross(axis));
                let (s1, s2) = ((d + ra).cross(perp), rb.cross(perp));
                let axial = self.lower_imp - self.upper_imp;
                let p = perp * self.perp.x + axis * axial;
                let la = self.perp.x * s1 + self.perp.y + axial * a1;
                let lb = self.perp.x * s2 + self.perp.y + axial * a2;
                ba.v -= p * ba.inv_mass;
                ba.w -= ba.inv_i * la;
                bb.v += p * bb.inv_mass;
                bb.w += bb.inv_i * lb;
            }
            JointKind::Target => {}
        }
    }

    pub fn solve(&mut self, bodies: &mut [Body], ctx: &Ctx) {
        if self.kind == JointKind::Target {
            if ctx.use_bias {
                self.solve_target(&mut bodies[self.b], ctx);
            }
            return;
        }
        let (ba, bb) = two_mut(bodies, self.a, self.b);
        match self.kind {
            JointKind::Revolute => self.solve_revolute(ba, bb, ctx),
            JointKind::Weld => self.solve_weld(ba, bb, ctx),
            JointKind::Distance | JointKind::Rope => self.solve_distance(ba, bb, ctx),
            JointKind::Prismatic => self.solve_prismatic(ba, bb, ctx),
            JointKind::Target => {}
        }
    }

    fn solve_revolute(&mut self, ba: &mut Body, bb: &mut Body, ctx: &Ctx) {
        let (ia, ib) = (ba.inv_i, bb.inv_i);
        let axial_mass = if ia + ib > 0.0 { 1.0 / (ia + ib) } else { 0.0 };
        // the point constraint goes first so the angular limit has the last word on the rotation
        solve_point(&mut self.lin, ba, bb, self.la, self.lb, ctx);
        if let Some((speed, torque)) = self.motor {
            if axial_mass > 0.0 {
                let cdot = bb.w - ba.w - speed;
                let max = ctx.h * torque;
                let new = (self.motor_imp - axial_mass * cdot).clamp(-max, max);
                let impulse = new - self.motor_imp;
                self.motor_imp = new;
                ba.w -= ia * impulse;
                bb.w += ib * impulse;
            }
        }
        if self.enable_limit && axial_mass > 0.0 {
            let angle = rel_angle(ba, bb, self.ref_q);
            let (bias, ms, is) = ctx.limit(angle - self.lower);
            let cdot = bb.w - ba.w;
            let new =
                (self.lower_imp - axial_mass * ms * (cdot + bias) - is * self.lower_imp).max(0.0);
            let impulse = new - self.lower_imp;
            self.lower_imp = new;
            ba.w -= ia * impulse;
            bb.w += ib * impulse;

            let (bias, ms, is) = ctx.limit(self.upper - angle);
            let cdot = ba.w - bb.w;
            let new =
                (self.upper_imp - axial_mass * ms * (cdot + bias) - is * self.upper_imp).max(0.0);
            let impulse = new - self.upper_imp;
            self.upper_imp = new;
            ba.w += ia * impulse;
            bb.w -= ib * impulse;
        }
    }

    fn solve_weld(&mut self, ba: &mut Body, bb: &mut Body, ctx: &Ctx) {
        let (ia, ib) = (ba.inv_i, bb.inv_i);
        if ia + ib > 0.0 {
            let (bias, ms, is) = ctx.bilateral(rel_angle(ba, bb, self.ref_q));
            let cdot = bb.w - ba.w;
            let impulse = -ms * (cdot + bias) / (ia + ib) - is * self.ang;
            self.ang += impulse;
            ba.w -= ia * impulse;
            bb.w += ib * impulse;
        }
        solve_point(&mut self.lin, ba, bb, self.la, self.lb, ctx);
    }

    fn solve_distance(&mut self, ba: &mut Body, bb: &mut Body, ctx: &Ctx) {
        let ra = ba.q.rotate(self.la - ba.local_center);
        let rb = bb.q.rotate(self.lb - bb.local_center);
        let d = (bb.c + rb) - (ba.c + ra);
        let len = d.length();
        if len < 1e-9 {
            return;
        }
        let axis = d / len;
        let vr = (bb.v + cross_sv(bb.w, rb)) - (ba.v + cross_sv(ba.w, ra));
        let (cra, crb) = (ra.cross(axis), rb.cross(axis));
        let k = ba.inv_mass + bb.inv_mass + ba.inv_i * cra * cra + bb.inv_i * crb * crb;
        let am = if k > 0.0 { 1.0 / k } else { 0.0 };
        let cdot = axis.dot(vr);
        if self.kind == JointKind::Rope {
            // only pulls: the slack is a speculative allowance
            let (bias, ms, is) = ctx.limit(self.upper - len);
            let new = (self.upper_imp + am * ms * (cdot - bias) - is * self.upper_imp).max(0.0);
            let impulse = new - self.upper_imp;
            self.upper_imp = new;
            apply(ba, bb, ra, rb, axis * -impulse);
        } else if let Some((hz, zeta)) = self.spring.filter(|s| s.0 > 0.0) {
            if ctx.use_bias {
                let soft = make_soft(hz, zeta, ctx.h);
                let bias = soft.bias_rate * (len - self.rest);
                let impulse = -soft.mass_scale * am * (cdot + bias) - soft.impulse_scale * self.imp;
                self.imp += impulse;
                apply(ba, bb, ra, rb, axis * impulse);
            }
        } else {
            let (bias, ms, is) = ctx.bilateral(len - self.rest);
            let impulse = -ms * am * (cdot + bias) - is * self.imp;
            self.imp += impulse;
            apply(ba, bb, ra, rb, axis * impulse);
        }
    }

    fn solve_prismatic(&mut self, ba: &mut Body, bb: &mut Body, ctx: &Ctx) {
        let (ma, ia, mb, ib) = (ba.inv_mass, ba.inv_i, bb.inv_mass, bb.inv_i);
        let ra = ba.q.rotate(self.la - ba.local_center);
        let rb = bb.q.rotate(self.lb - bb.local_center);
        let d = (bb.c - ba.c) + rb - ra;
        let axis = ba.q.rotate(self.laxis);
        let perp = axis.perp();
        let translation = axis.dot(d);
        let (a1, a2) = ((d + ra).cross(axis), rb.cross(axis));
        let k = ma + mb + ia * a1 * a1 + ib * a2 * a2;
        let axial_mass = if k > 0.0 { 1.0 / k } else { 0.0 };
        if self.enable_limit {
            let (bias, ms, is) = ctx.limit(translation - self.lower);
            let cdot = axis.dot(bb.v - ba.v) + a2 * bb.w - a1 * ba.w;
            let new =
                (self.lower_imp - axial_mass * ms * (cdot + bias) - is * self.lower_imp).max(0.0);
            let impulse = new - self.lower_imp;
            self.lower_imp = new;
            let p = axis * impulse;
            ba.v -= p * ma;
            ba.w -= ia * impulse * a1;
            bb.v += p * mb;
            bb.w += ib * impulse * a2;

            let (bias, ms, is) = ctx.limit(self.upper - translation);
            let cdot = axis.dot(ba.v - bb.v) + a1 * ba.w - a2 * bb.w;
            let new =
                (self.upper_imp - axial_mass * ms * (cdot + bias) - is * self.upper_imp).max(0.0);
            let impulse = new - self.upper_imp;
            self.upper_imp = new;
            let p = axis * impulse;
            ba.v += p * ma;
            ba.w += ia * impulse * a1;
            bb.v -= p * mb;
            bb.w -= ib * impulse * a2;
        }
        // keep the bodies on the axis and aligned
        let (s1, s2) = ((d + ra).cross(perp), rb.cross(perp));
        let cdot = v2(perp.dot(bb.v - ba.v) + s2 * bb.w - s1 * ba.w, bb.w - ba.w);
        let c = v2(perp.dot(d), rel_angle(ba, bb, self.ref_q));
        let (bias, ms, is) = if ctx.use_bias {
            (
                c * ctx.soft.bias_rate,
                ctx.soft.mass_scale,
                ctx.soft.impulse_scale,
            )
        } else {
            (Vec2::ZERO, 1.0, 0.0)
        };
        let k11 = ma + mb + ia * s1 * s1 + ib * s2 * s2;
        let k12 = ia * s1 + ib * s2;
        let k22 = if ia + ib == 0.0 { 1.0 } else { ia + ib };
        let b = solve22(k11, k12, k22, cdot + bias);
        let impulse = -(b * ms) - self.perp * is;
        self.perp += impulse;
        let p = perp * impulse.x;
        let la = impulse.x * s1 + impulse.y;
        let lb = impulse.x * s2 + impulse.y;
        ba.v -= p * ma;
        ba.w -= ia * la;
        bb.v += p * mb;
        bb.w += ib * lb;
    }

    fn solve_target(&mut self, bb: &mut Body, ctx: &Ctx) {
        let (hz, zeta) = self.spring.unwrap_or((5.0, 0.7));
        let soft = make_soft(hz, zeta, ctx.h);
        let rb = bb.q.rotate(self.lb - bb.local_center);
        let cdot = bb.v + cross_sv(bb.w, rb);
        let c = (bb.c + rb) - self.target;
        let (mb, ib) = (bb.inv_mass, bb.inv_i);
        let k11 = mb + ib * rb.y * rb.y;
        let k12 = -ib * rb.x * rb.y;
        let k22 = mb + ib * rb.x * rb.x;
        let b = solve22(k11, k12, k22, cdot + c * soft.bias_rate);
        let mut new = self.lin + (-(b * soft.mass_scale) - self.lin * soft.impulse_scale);
        let max = self.max_force * ctx.h;
        let len = new.length();
        if len > max {
            new = new * (max / len);
        }
        let impulse = new - self.lin;
        self.lin = new;
        bb.v += impulse * mb;
        bb.w += ib * rb.cross(impulse);
    }
}
