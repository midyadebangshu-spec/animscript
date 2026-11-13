//! Joints between two bodies: ball, hinge (with limits and a motor), fixed and distance (rigid,
//! spring or rope). Velocity-level sequential impulses with a Baumgarte push for position error.
use crate::math::*;
use crate::world::Body;

#[derive(Clone, Debug)]
pub enum JointKind {
    Ball,
    Hinge {
        limits: Option<(f64, f64)>,
        /// Target speed (rad/s) and the most torque it may use.
        motor: Option<(f64, f64)>,
    },
    Fixed,
    Distance {
        length: f64,
        /// Frequency (Hz) and damping ratio of a soft spring.
        spring: Option<(f64, f64)>,
        /// A rope only pulls when stretched beyond `length`.
        rope: bool,
    },
}

#[derive(Clone, Debug)]
pub struct JointDef {
    pub a: usize,
    pub b: usize,
    pub kind: JointKind,
    /// World anchor point(s); `anchor_b` defaults to `anchor_a`.
    pub anchor_a: V3,
    pub anchor_b: Option<V3>,
    /// World hinge axis.
    pub axis: V3,
    pub collide_connected: bool,
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub a: usize,
    pub b: usize,
    pub kind: JointKind,
    pub collide_connected: bool,
    /// The step the joint is solved with (seconds); set by the world.
    pub h: f64,
    la: V3,
    lb: V3,
    axis_a: V3,
    axis_b: V3,
    ref_a: V3,
    ref_b: V3,
    q_rel: Quat,
    // prepared each step
    ra: V3,
    rb: V3,
    rows: Vec<Row>,
    k_inv: Vec<f64>,
    /// Hinge: the impulses on every row (the axis row last) that change the axis speed by one
    /// while the other rows stay satisfied.
    axis_col: Vec<f64>,
    axis_dir: V3,
    bias: Vec<f64>,
    // accumulated impulses
    acc: Vec<f64>,
    acc_lo: f64,
    acc_hi: f64,
    acc_motor: f64,
    acc_dist: f64,
}

/// One scalar constraint: its velocity is `lin . (relative anchor velocity) + ang . (relative
/// angular velocity)`. Hinge and fixed joints solve all their rows together as one block, so the
/// point and the angular parts cannot fight each other.
#[derive(Clone, Copy, Debug)]
struct Row {
    lin: V3,
    ang: V3,
}

fn solve_dense(n: usize, a: &[f64]) -> Vec<f64> {
    // Gauss-Jordan inverse with partial pivoting; a singular matrix gives zeros
    let mut m = vec![0.0; n * 2 * n];
    for i in 0..n {
        for j in 0..n {
            m[i * 2 * n + j] = a[i * n + j];
        }
        m[i * 2 * n + n + i] = 1.0;
    }
    for c in 0..n {
        let mut piv = c;
        for r in c + 1..n {
            if m[r * 2 * n + c].abs() > m[piv * 2 * n + c].abs() {
                piv = r;
            }
        }
        if m[piv * 2 * n + c].abs() < 1e-300 {
            return vec![0.0; n * n];
        }
        if piv != c {
            for j in 0..2 * n {
                m.swap(c * 2 * n + j, piv * 2 * n + j);
            }
        }
        let d = m[c * 2 * n + c];
        for j in 0..2 * n {
            m[c * 2 * n + j] /= d;
        }
        for r in 0..n {
            if r != c {
                let f = m[r * 2 * n + c];
                if f != 0.0 {
                    for j in 0..2 * n {
                        m[r * 2 * n + j] -= f * m[c * 2 * n + j];
                    }
                }
            }
        }
    }
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            inv[i * n + j] = m[i * 2 * n + n + j];
        }
    }
    inv
}

fn vel_at(b: &Body, r: V3) -> V3 {
    b.v + b.w.cross(r)
}

fn apply_lin(bodies: &mut [Body], i: usize, r: V3, p: V3) {
    let b = &mut bodies[i];
    b.v += p * b.inv_mass;
    b.w += b.inv_i.mul_v(r.cross(p));
}

fn apply_ang(bodies: &mut [Body], i: usize, t: V3) {
    let b = &mut bodies[i];
    b.w += b.inv_i.mul_v(t);
}

impl Joint {
    pub fn new(def: &JointDef, bodies: &[Body]) -> Joint {
        let (ba, bb) = (&bodies[def.a], &bodies[def.b]);
        let pa = def.anchor_a;
        let pb = def.anchor_b.unwrap_or(pa);
        let la = ba.q.conj().rotate(pa - ba.p);
        let lb = bb.q.conj().rotate(pb - bb.p);
        let axis = def.axis.normalized();
        let axis = if axis == V3::ZERO {
            v3(0.0, 1.0, 0.0)
        } else {
            axis
        };
        let rf = axis.any_perp();
        let mut kind = def.kind.clone();
        if let JointKind::Distance { length, .. } = &mut kind {
            if *length < 0.0 {
                *length = (pb - pa).len();
            }
        }
        Joint {
            a: def.a,
            b: def.b,
            kind,
            collide_connected: def.collide_connected,
            h: 1.0 / 240.0,
            la,
            lb,
            axis_a: ba.q.conj().rotate(axis),
            axis_b: bb.q.conj().rotate(axis),
            ref_a: ba.q.conj().rotate(rf),
            ref_b: bb.q.conj().rotate(rf),
            q_rel: ba.q.conj().mul(bb.q),
            ra: V3::ZERO,
            rb: V3::ZERO,
            rows: vec![],
            k_inv: vec![],
            axis_col: vec![],
            axis_dir: V3::ZERO,
            bias: vec![],
            acc: vec![],
            acc_lo: 0.0,
            acc_hi: 0.0,
            acc_motor: 0.0,
            acc_dist: 0.0,
        }
    }

    pub fn anchors(&self, bodies: &[Body]) -> (V3, V3) {
        let (a, b) = (&bodies[self.a], &bodies[self.b]);
        (a.p + a.q.rotate(self.la), b.p + b.q.rotate(self.lb))
    }

    pub fn angle(&self, bodies: &[Body]) -> f64 {
        let (a, b) = (&bodies[self.a], &bodies[self.b]);
        let ua = a.q.rotate(self.axis_a);
        let (xa, xb) = (a.q.rotate(self.ref_a), b.q.rotate(self.ref_b));
        xa.cross(xb).dot(ua).atan2(xa.dot(xb))
    }

    pub fn prepare(&mut self, bodies: &mut [Body]) {
        let (ia, ib) = (self.a, self.b);
        self.ra = bodies[ia].q.rotate(self.la);
        self.rb = bodies[ib].q.rotate(self.lb);
        let (ra, rb) = (self.ra, self.rb);
        if matches!(self.kind, JointKind::Distance { .. }) {
            self.acc_dist = 0.0;
            return;
        }
        let ex = [v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0)];
        let mut rows: Vec<Row> = ex
            .iter()
            .map(|e| Row {
                lin: *e,
                ang: V3::ZERO,
            })
            .collect();
        let beta = 0.2 / self.h;
        let (pa, pb) = (bodies[ia].p + ra, bodies[ib].p + rb);
        let c = pb - pa;
        let mut bias: Vec<f64> = vec![beta * c.x, beta * c.y, beta * c.z];
        match self.kind {
            JointKind::Hinge { .. } => {
                let ua = bodies[ia].q.rotate(self.axis_a);
                let ub = bodies[ib].q.rotate(self.axis_b);
                let e = ub.cross(ua);
                let t1 = ua.any_perp();
                let t2 = ua.cross(t1);
                for t in [t1, t2] {
                    rows.push(Row {
                        lin: V3::ZERO,
                        ang: t,
                    });
                    bias.push(-beta * e.dot(t));
                }
            }
            JointKind::Fixed => {
                let target = bodies[ia].q.mul(self.q_rel);
                let mut qd = target.mul(bodies[ib].q.conj());
                if qd.w < 0.0 {
                    qd = Quat {
                        w: -qd.w,
                        x: -qd.x,
                        y: -qd.y,
                        z: -qd.z,
                    };
                }
                let phi = v3(qd.x, qd.y, qd.z) * 2.0;
                for e in ex {
                    rows.push(Row {
                        lin: V3::ZERO,
                        ang: e,
                    });
                    bias.push(-beta * phi.dot(e));
                }
            }
            _ => {}
        }
        let n = rows.len();
        let mut k = vec![0.0; n * n];
        for (j, rj) in rows.iter().enumerate() {
            let (dv, dw) = Self::response(bodies, ia, ib, ra, rb, rj.lin, rj.ang);
            for (i, ri) in rows.iter().enumerate() {
                k[i * n + j] = ri.lin.dot(dv) + ri.ang.dot(dw);
            }
        }
        self.k_inv = solve_dense(n, &k);
        if let JointKind::Hinge { .. } = self.kind {
            let ua = bodies[ia].q.rotate(self.axis_a);
            let mut rows6 = rows.clone();
            rows6.push(Row {
                lin: V3::ZERO,
                ang: ua,
            });
            let n6 = n + 1;
            let mut k6 = vec![0.0; n6 * n6];
            for (j, rj) in rows6.iter().enumerate() {
                let (dv, dw) = Self::response(bodies, ia, ib, ra, rb, rj.lin, rj.ang);
                for (i, ri) in rows6.iter().enumerate() {
                    k6[i * n6 + j] = ri.lin.dot(dv) + ri.ang.dot(dw);
                }
            }
            let inv6 = solve_dense(n6, &k6);
            self.axis_col = (0..n6).map(|i| inv6[i * n6 + n]).collect();
            self.axis_dir = ua;
        }
        self.bias = bias;
        if self.acc.len() != n {
            self.acc = vec![0.0; n];
        }
        // warm start
        let (mut p, mut l) = (V3::ZERO, V3::ZERO);
        for (r, a) in rows.iter().zip(&self.acc) {
            p += r.lin * *a;
            l += r.ang * *a;
        }
        if let JointKind::Hinge { .. } = self.kind {
            let ua = bodies[ia].q.rotate(self.axis_a);
            l += ua * (self.acc_lo + self.acc_hi + self.acc_motor);
        }
        self.rows = rows;
        apply_lin(bodies, ia, ra, -p);
        apply_lin(bodies, ib, rb, p);
        apply_ang(bodies, ia, -l);
        apply_ang(bodies, ib, l);
    }

    /// The change in (relative anchor velocity, relative angular velocity) caused by the impulse
    /// `p` at the anchors and the angular impulse `l`, applied +B / -A.
    fn response(bodies: &[Body], ia: usize, ib: usize, ra: V3, rb: V3, p: V3, l: V3) -> (V3, V3) {
        let (a, b) = (&bodies[ia], &bodies[ib]);
        let dwa = -a.inv_i.mul_v(ra.cross(p) + l);
        let dwb = b.inv_i.mul_v(rb.cross(p) + l);
        let dva = -p * a.inv_mass;
        let dvb = p * b.inv_mass;
        (dvb + dwb.cross(rb) - dva - dwa.cross(ra), dwb - dwa)
    }

    pub fn solve(&mut self, bodies: &mut [Body], h: f64) {
        match self.kind.clone() {
            JointKind::Distance {
                length,
                spring,
                rope,
            } => self.solve_distance(bodies, h, length, spring, rope),
            kind => {
                self.solve_block(bodies);
                if let JointKind::Hinge { limits, motor } = kind {
                    self.solve_hinge(bodies, h, 0.2 / h, limits, motor);
                }
            }
        }
    }

    fn solve_block(&mut self, bodies: &mut [Body]) {
        let (ia, ib, ra, rb) = (self.a, self.b, self.ra, self.rb);
        let n = self.rows.len();
        let (a, b) = (&bodies[ia], &bodies[ib]);
        let dv = vel_at(b, rb) - vel_at(a, ra);
        let wr = b.w - a.w;
        let res: Vec<f64> = self
            .rows
            .iter()
            .zip(&self.bias)
            .map(|(r, bs)| r.lin.dot(dv) + r.ang.dot(wr) + bs)
            .collect();
        let (mut p, mut l) = (V3::ZERO, V3::ZERO);
        for i in 0..n {
            let mut lam = 0.0;
            for (j, rj) in res.iter().enumerate() {
                lam -= self.k_inv[i * n + j] * rj;
            }
            self.acc[i] += lam;
            p += self.rows[i].lin * lam;
            l += self.rows[i].ang * lam;
        }
        apply_lin(bodies, ia, ra, -p);
        apply_lin(bodies, ib, rb, p);
        apply_ang(bodies, ia, -l);
        apply_ang(bodies, ib, l);
    }

    /// Change the hinge's axis speed by `delta` (rad/s) with the other rows still satisfied; the
    /// accumulated axis impulse is kept inside `[lo, hi]`.
    fn axis_impulse(&mut self, bodies: &mut [Body], delta: f64, which: u8, lo: f64, hi: f64) {
        let n = self.rows.len();
        let lam: Vec<f64> = self.axis_col.iter().map(|c| c * delta).collect();
        let acc = match which {
            0 => self.acc_motor,
            1 => self.acc_lo,
            _ => self.acc_hi,
        };
        let want = lam[n];
        let new = (acc + want).clamp(lo, hi);
        let f = if want.abs() > 1e-300 {
            (new - acc) / want
        } else {
            0.0
        };
        match which {
            0 => self.acc_motor = new,
            1 => self.acc_lo = new,
            _ => self.acc_hi = new,
        }
        let (ia, ib, ra, rb) = (self.a, self.b, self.ra, self.rb);
        let (mut p, mut l) = (V3::ZERO, self.axis_dir * (lam[n] * f));
        for i in 0..n {
            self.acc[i] += lam[i] * f;
            p += self.rows[i].lin * (lam[i] * f);
            l += self.rows[i].ang * (lam[i] * f);
        }
        apply_lin(bodies, ia, ra, -p);
        apply_lin(bodies, ib, rb, p);
        apply_ang(bodies, ia, -l);
        apply_ang(bodies, ib, l);
    }

    fn solve_hinge(
        &mut self,
        bodies: &mut [Body],
        h: f64,
        beta: f64,
        limits: Option<(f64, f64)>,
        motor: Option<(f64, f64)>,
    ) {
        let (ia, ib) = (self.a, self.b);
        let ua = self.axis_dir;
        if let Some((speed, max_t)) = motor {
            let wr = (bodies[ib].w - bodies[ia].w).dot(ua);
            self.axis_impulse(bodies, speed - wr, 0, -max_t * h, max_t * h);
        }
        if let Some((lo, hi)) = limits {
            let th = self.angle(bodies);
            if th < lo {
                let wr = (bodies[ib].w - bodies[ia].w).dot(ua);
                self.axis_impulse(bodies, -beta * (th - lo) - wr, 1, 0.0, f64::MAX);
            } else {
                self.acc_lo = 0.0;
            }
            if th > hi {
                let wr = (bodies[ib].w - bodies[ia].w).dot(ua);
                self.axis_impulse(bodies, -beta * (th - hi) - wr, 2, f64::MIN, 0.0);
            } else {
                self.acc_hi = 0.0;
            }
        }
    }

    fn solve_distance(
        &mut self,
        bodies: &mut [Body],
        h: f64,
        length: f64,
        spring: Option<(f64, f64)>,
        rope: bool,
    ) {
        let (ia, ib, ra, rb) = (self.a, self.b, self.ra, self.rb);
        let (a, b) = (&bodies[ia], &bodies[ib]);
        let d = (b.p + rb) - (a.p + ra);
        let l = d.len();
        if l < 1e-12 {
            return;
        }
        let n = d / l;
        let c = l - length;
        if rope && c < 0.0 {
            self.acc_dist = 0.0;
            return;
        }
        let k = a.inv_mass
            + b.inv_mass
            + ra.cross(n).dot(a.inv_i.mul_v(ra.cross(n)))
            + rb.cross(n).dot(b.inv_i.mul_v(rb.cross(n)));
        if k < 1e-300 {
            return;
        }
        let dv = (vel_at(b, rb) - vel_at(a, ra)).dot(n);
        let (m, bias, gamma) = if let Some((hz, zeta)) = spring {
            let mass = 1.0 / k;
            let omega = std::f64::consts::TAU * hz;
            let dd = 2.0 * mass * zeta * omega;
            let kk = mass * omega * omega;
            let g = 1.0 / (h * (dd + h * kk));
            let bs = c * h * kk * g;
            (1.0 / (k + g), bs, g)
        } else {
            (1.0 / k, c * 0.2 / h, 0.0)
        };
        let lam = -m * (dv + bias + gamma * self.acc_dist);
        let old = self.acc_dist;
        self.acc_dist = if rope {
            (old + lam).min(0.0)
        } else {
            old + lam
        };
        let dl = self.acc_dist - old;
        apply_lin(bodies, ia, ra, -n * dl);
        apply_lin(bodies, ib, rb, n * dl);
    }
}
