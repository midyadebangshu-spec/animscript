//! Soft bodies in 3D: extended position-based dynamics (XPBD). A body is a set of simulated nodes;
//! the points that collide and are drawn are embedded in them (a surface vertex is its own node; a
//! vertex of a solid body sits inside a tetrahedron). Edges keep their length, a closed surface
//! keeps its volume (pressure), tetrahedra keep theirs (solid jelly), pins hold nodes at a target,
//! and points collide with rigid bodies (pushing them back) and with the triangles of soft bodies.
//! Units: metres, kilograms, seconds. Every iteration order is fixed, so results are reproducible.
use crate::math::*;
use crate::world::{BodyKind, World};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct SoftParams {
    /// Collision radius of a point, metres.
    pub radius: f64,
    pub friction: f64,
    pub restitution: f64,
    /// Velocity damping per second.
    pub damping: f64,
    /// Let the body's points collide with its own triangles too.
    pub self_collide: bool,
}

impl Default for SoftParams {
    fn default() -> SoftParams {
        SoftParams {
            radius: 0.01,
            friction: 0.5,
            restitution: 0.1,
            damping: 0.1,
            self_collide: false,
        }
    }
}

/// A point of the body: a weighted blend of up to four nodes.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub nodes: [usize; 4],
    pub w: [f64; 4],
    pub n: usize,
}

impl Point {
    pub fn node(i: usize) -> Point {
        Point {
            nodes: [i, 0, 0, 0],
            w: [1.0, 0.0, 0.0, 0.0],
            n: 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pin {
    pub target: V3,
    last: V3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Edge,
    /// A bending link across two triangles.
    Bend(usize, usize),
    /// An internal brace of a solid body.
    Brace,
}

#[derive(Clone, Debug)]
struct Dist {
    a: usize,
    b: usize,
    rest: f64,
    compliance: f64,
    kind: Kind,
}

#[derive(Clone, Debug)]
struct TetC {
    n: [usize; 4],
    rest: f64,
    compliance: f64,
}

#[derive(Clone, Debug)]
pub struct SoftBody3 {
    pub pos: Vec<V3>,
    pub vel: Vec<V3>,
    pub mass: Vec<f64>,
    pub pins: Vec<Option<Pin>>,
    pub points: Vec<Point>,
    /// Surface triangles over the points, with the mesh face each came from.
    pub tris: Vec<[usize; 3]>,
    pub tri_group: Vec<usize>,
    pub tri_alive: Vec<bool>,
    pub params: SoftParams,
    dist: Vec<Dist>,
    tets: Vec<TetC>,
    volume: Option<(f64, f64)>,
    rest_points: Vec<V3>,
    old: Vec<V3>,
    exclude: f64,
}

struct RContact {
    point: usize,
    body: usize,
    normal: V3,
    at: V3,
    depth: f64,
}

fn tri_closest(p: V3, a: V3, b: V3, c: V3) -> (V3, [f64; 3]) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let den = 1.0 / (va + vb + vc);
    let v = vb * den;
    let w = vc * den;
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

fn seg_seg_dist(p1: V3, q1: V3, p2: V3, q2: V3) -> f64 {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t);
    if a < 1e-300 && e < 1e-300 {
        s = 0.0;
        t = 0.0;
    } else if a < 1e-300 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e < 1e-300 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let den = a * e - b * b;
            let mut ss = if den > 1e-12 * a * e {
                ((b * f - c * e) / den).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut tt = (b * ss + f) / e;
            if tt < 0.0 {
                tt = 0.0;
                ss = (-c / a).clamp(0.0, 1.0);
            } else if tt > 1.0 {
                tt = 1.0;
                ss = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = ss;
            t = tt;
        }
    }
    ((p1 + d1 * s) - (p2 + d2 * t)).len()
}

impl SoftBody3 {
    pub fn new(pos: Vec<V3>, mass: Vec<f64>, points: Vec<Point>, params: SoftParams) -> SoftBody3 {
        let n = pos.len();
        let mut b = SoftBody3 {
            pos,
            vel: vec![V3::ZERO; n],
            mass,
            pins: vec![None; n],
            points,
            tris: vec![],
            tri_group: vec![],
            tri_alive: vec![],
            params,
            dist: vec![],
            tets: vec![],
            volume: None,
            rest_points: vec![],
            old: vec![V3::ZERO; n],
            exclude: 0.0,
        };
        b.rest_points = (0..b.points.len()).map(|i| b.point_pos(i)).collect();
        b
    }

    /// The surface triangles (over the points) and the mesh face each belongs to.
    pub fn set_surface(&mut self, tris: Vec<[usize; 3]>, groups: Vec<usize>) {
        self.tri_alive = vec![true; tris.len()];
        self.tris = tris;
        self.tri_group = groups;
    }

    /// Keep `a` and `b` at their current distance.
    pub fn add_edge(&mut self, a: usize, b: usize, compliance: f64) {
        let rest = (self.pos[a] - self.pos[b]).len();
        self.dist.push(Dist {
            a,
            b,
            rest,
            compliance,
            kind: Kind::Edge,
        });
    }
    pub fn add_brace(&mut self, a: usize, b: usize, compliance: f64) {
        let rest = (self.pos[a] - self.pos[b]).len();
        self.dist.push(Dist {
            a,
            b,
            rest,
            compliance,
            kind: Kind::Brace,
        });
    }
    /// A bending link between the vertices opposite a shared edge of triangles `t1` and `t2`.
    pub fn add_bend(&mut self, a: usize, b: usize, t1: usize, t2: usize, compliance: f64) {
        let rest = (self.pos[a] - self.pos[b]).len();
        self.dist.push(Dist {
            a,
            b,
            rest,
            compliance,
            kind: Kind::Bend(t1, t2),
        });
    }
    pub fn add_tet(&mut self, n: [usize; 4], compliance: f64) {
        let rest = self.tet_volume(n);
        self.tets.push(TetC {
            n,
            rest,
            compliance,
        });
    }
    /// Keep the closed surface at `scale` times its current volume (`scale > 1` inflates it).
    pub fn set_volume(&mut self, scale: f64, compliance: f64) {
        let v = self.surface_volume();
        self.volume = Some((v * scale, compliance));
    }
    pub fn set_exclusion(&mut self, d: f64) {
        self.exclude = d;
    }

    fn tet_volume(&self, n: [usize; 4]) -> f64 {
        let (a, b, c, d) = (
            self.pos[n[0]],
            self.pos[n[1]],
            self.pos[n[2]],
            self.pos[n[3]],
        );
        (b - a).cross(c - a).dot(d - a) / 6.0
    }

    pub fn point_pos(&self, i: usize) -> V3 {
        let p = &self.points[i];
        let mut s = V3::ZERO;
        for k in 0..p.n {
            s += self.pos[p.nodes[k]] * p.w[k];
        }
        s
    }
    pub fn point_vel(&self, i: usize) -> V3 {
        let p = &self.points[i];
        let mut s = V3::ZERO;
        for k in 0..p.n {
            s += self.vel[p.nodes[k]] * p.w[k];
        }
        s
    }
    fn winv(&self, n: usize) -> f64 {
        if self.pins[n].is_some() || self.mass[n] <= 0.0 {
            0.0
        } else {
            1.0 / self.mass[n]
        }
    }
    /// How freely the point moves under a unit force: the sum of weight squared over mass.
    fn point_w(&self, i: usize) -> f64 {
        let p = &self.points[i];
        (0..p.n)
            .map(|k| p.w[k] * p.w[k] * self.winv(p.nodes[k]))
            .sum()
    }
    /// Move a point by `d` by moving its nodes (freest nodes move most).
    fn push_point(&mut self, i: usize, d: V3) {
        let wp = self.point_w(i);
        if wp <= 0.0 {
            return;
        }
        let p = self.points[i];
        for k in 0..p.n {
            let n = p.nodes[k];
            let f = p.w[k] * self.winv(n) / wp;
            self.pos[n] += d * f;
        }
    }
    fn push_point_vel(&mut self, i: usize, dv: V3) {
        let wp = self.point_w(i);
        if wp <= 0.0 {
            return;
        }
        let p = self.points[i];
        for k in 0..p.n {
            let n = p.nodes[k];
            let f = p.w[k] * self.winv(n) / wp;
            self.vel[n] += dv * f;
        }
    }

    pub fn pin(&mut self, node: usize, target: V3) {
        self.pins[node] = Some(Pin {
            target,
            last: target,
        });
        self.vel[node] = V3::ZERO;
    }
    pub fn set_pin_target(&mut self, node: usize, target: V3) {
        if let Some(p) = &mut self.pins[node] {
            p.last = p.target;
            p.target = target;
        }
    }
    pub fn unpin(&mut self, node: usize) {
        self.pins[node] = None;
    }

    pub fn total_mass(&self) -> f64 {
        self.mass.iter().sum()
    }
    /// Add a velocity change that an impulse (kg m/s) gives the whole body.
    pub fn apply_impulse(&mut self, impulse: V3) {
        let m: f64 = (0..self.pos.len())
            .filter(|i| self.winv(*i) > 0.0)
            .map(|i| self.mass[i])
            .sum();
        if m <= 0.0 {
            return;
        }
        for i in 0..self.pos.len() {
            if self.winv(i) > 0.0 {
                self.vel[i] += impulse / m;
            }
        }
    }

    pub fn surface_volume(&self) -> f64 {
        let mut v = 0.0;
        for (i, t) in self.tris.iter().enumerate() {
            if !self.tri_alive[i] {
                continue;
            }
            let (a, b, c) = (
                self.point_pos(t[0]),
                self.point_pos(t[1]),
                self.point_pos(t[2]),
            );
            v += a.dot(b.cross(c)) / 6.0;
        }
        v
    }
    /// Largest edge strain, `|length - rest| / rest`.
    pub fn max_strain(&self) -> f64 {
        self.dist
            .iter()
            .filter(|d| d.rest > 1e-12 && d.kind == Kind::Edge)
            .map(|d| (((self.pos[d.a] - self.pos[d.b]).len() - d.rest) / d.rest).abs())
            .fold(0.0, f64::max)
    }
    /// Spheres that stand for the skin of this body inside the box `lo`-`hi`: the points of every
    /// live triangle and samples across it about `spacing` apart (fluids treat them as obstacles).
    pub fn obstacle_points(&self, spacing: f64, lo: V3, hi: V3) -> Vec<(V3, f64)> {
        let r = self.params.radius.max(0.005);
        let mut out = vec![];
        for (i, t) in self.tris.iter().enumerate() {
            if !self.tri_alive[i] {
                continue;
            }
            let p = [
                self.point_pos(t[0]),
                self.point_pos(t[1]),
                self.point_pos(t[2]),
            ];
            let (tl, th) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            if th.x < lo.x
                || tl.x > hi.x
                || th.y < lo.y
                || tl.y > hi.y
                || th.z < lo.z
                || tl.z > hi.z
            {
                continue;
            }
            let longest = (p[1] - p[0])
                .len()
                .max((p[2] - p[1]).len())
                .max((p[0] - p[2]).len());
            let k = ((longest / spacing).ceil() as usize).clamp(1, 12);
            for a in 0..=k {
                for b in 0..=(k - a) {
                    let (u, v) = (a as f64 / k as f64, b as f64 / k as f64);
                    out.push((p[0] + (p[1] - p[0]) * u + (p[2] - p[0]) * v, r));
                }
            }
        }
        out
    }

    pub fn node_count(&self) -> usize {
        self.pos.len()
    }

    /// Cut the surface where the segment `a`-`b` passes within `width` of a triangle's edge: those
    /// triangles (and every triangle of the same mesh face) are removed and the edge and bend
    /// links no triangle holds any more are dropped. Returns the removed groups (mesh faces).
    pub fn cut(&mut self, a: V3, b: V3, width: f64) -> Vec<usize> {
        let mut groups: Vec<usize> = vec![];
        for (i, t) in self.tris.iter().enumerate() {
            if !self.tri_alive[i] {
                continue;
            }
            let p = [
                self.point_pos(t[0]),
                self.point_pos(t[1]),
                self.point_pos(t[2]),
            ];
            let hit = (0..3).any(|k| seg_seg_dist(a, b, p[k], p[(k + 1) % 3]) <= width);
            if hit && !groups.contains(&self.tri_group[i]) {
                groups.push(self.tri_group[i]);
            }
        }
        if groups.is_empty() {
            return groups;
        }
        for i in 0..self.tris.len() {
            if groups.contains(&self.tri_group[i]) {
                self.tri_alive[i] = false;
            }
        }
        let alive = self.tri_alive.clone();
        let tris = self.tris.clone();
        let held = |x: usize, y: usize| {
            tris.iter()
                .enumerate()
                .any(|(i, t)| alive[i] && t.contains(&x) && t.contains(&y))
        };
        self.dist.retain(|d| match d.kind {
            Kind::Edge => held(d.a, d.b),
            Kind::Bend(t1, t2) => alive[t1] && alive[t2],
            Kind::Brace => true,
        });
        self.volume = None;
        groups
    }

    // ---- the step -------------------------------------------------------------------------

    fn predict(&mut self, h: f64, gravity: V3, s: usize, substeps: usize) {
        let damp = (-self.params.damping * h).exp();
        let frac = (s + 1) as f64 / substeps as f64;
        for i in 0..self.pos.len() {
            self.old[i] = self.pos[i];
            if let Some(p) = self.pins[i] {
                self.pos[i] = p.last + (p.target - p.last) * frac;
                continue;
            }
            self.vel[i] += gravity * h;
            self.vel[i] *= damp;
            self.pos[i] += self.vel[i] * h;
        }
    }

    fn solve(&mut self, h: f64, lam: &mut Lambdas) {
        let inv_h2 = 1.0 / (h * h);
        for (k, d) in self.dist.iter().enumerate() {
            let (wa, wb) = (self.winv(d.a), self.winv(d.b));
            if wa + wb <= 0.0 {
                continue;
            }
            let dv = self.pos[d.a] - self.pos[d.b];
            let l = dv.len();
            if l < 1e-12 {
                continue;
            }
            let n = dv / l;
            let alpha = d.compliance * inv_h2;
            let c = l - d.rest;
            let dl = (-c - alpha * lam.dist[k]) / (wa + wb + alpha);
            lam.dist[k] += dl;
            self.pos[d.a] += n * (dl * wa);
            self.pos[d.b] -= n * (dl * wb);
        }
        for (k, t) in self.tets.iter().enumerate() {
            let [a, b, c, d] = t.n;
            let (pa, pb, pc, pd) = (self.pos[a], self.pos[b], self.pos[c], self.pos[d]);
            let ga = (pd - pb).cross(pc - pb) / 6.0;
            let gb = (pc - pa).cross(pd - pa) / 6.0;
            let gc = (pd - pa).cross(pb - pa) / 6.0;
            let gd = (pb - pa).cross(pc - pa) / 6.0;
            let w = [self.winv(a), self.winv(b), self.winv(c), self.winv(d)];
            let g = [ga, gb, gc, gd];
            let denom: f64 = (0..4).map(|i| w[i] * g[i].len2()).sum();
            let alpha = t.compliance * inv_h2;
            if denom + alpha <= 1e-300 {
                continue;
            }
            let vol = (pb - pa).cross(pc - pa).dot(pd - pa) / 6.0;
            let c = vol - t.rest;
            let dl = (-c - alpha * lam.tet[k]) / (denom + alpha);
            lam.tet[k] += dl;
            for i in 0..4 {
                self.pos[t.n[i]] += g[i] * (dl * w[i]);
            }
        }
        if let Some((target, compliance)) = self.volume {
            let mut grad: BTreeMap<usize, V3> = BTreeMap::new();
            let mut vol = 0.0;
            for (i, t) in self.tris.iter().enumerate() {
                if !self.tri_alive[i] {
                    continue;
                }
                let p = [
                    self.point_pos(t[0]),
                    self.point_pos(t[1]),
                    self.point_pos(t[2]),
                ];
                vol += p[0].dot(p[1].cross(p[2])) / 6.0;
                let g = [
                    p[1].cross(p[2]) / 6.0,
                    p[2].cross(p[0]) / 6.0,
                    p[0].cross(p[1]) / 6.0,
                ];
                for k in 0..3 {
                    *grad.entry(t[k]).or_insert(V3::ZERO) += g[k];
                }
            }
            // the gradient is per point; spread it over the nodes by weight
            let mut ng: BTreeMap<usize, V3> = BTreeMap::new();
            for (pi, g) in &grad {
                let p = self.points[*pi];
                for k in 0..p.n {
                    *ng.entry(p.nodes[k]).or_insert(V3::ZERO) += *g * p.w[k];
                }
            }
            let denom: f64 = ng.iter().map(|(n, g)| self.winv(*n) * g.len2()).sum();
            let alpha = compliance * inv_h2;
            if denom + alpha > 1e-300 {
                let c = vol - target;
                let dl = (-c - alpha * lam.vol) / (denom + alpha);
                lam.vol += dl;
                for (n, g) in &ng {
                    let w = self.winv(*n);
                    self.pos[*n] += *g * (dl * w);
                }
            }
        }
    }

    fn collide_rigid(&mut self, world: &World) -> Vec<RContact> {
        let mut out = vec![];
        for i in 0..self.points.len() {
            let p = self.point_pos(i);
            for hit in world.query_sphere(p, self.params.radius) {
                // static and kinematic bodies are pushed out of by position; a dynamic body is
                // pushed apart by velocity in `contact_velocity`, shared by inverse mass
                if world.body(hit.body).kind != BodyKind::Dynamic {
                    // at most a quarter radius per substep, so a deep overlap (a body that
                    // starts resting on something) is eased out instead of being popped
                    let d = hit.depth.min(self.params.radius * 0.25);
                    self.push_point(i, hit.normal * d);
                }
                out.push(RContact {
                    point: i,
                    body: hit.body,
                    normal: hit.normal,
                    at: hit.point,
                    depth: hit.depth,
                });
            }
        }
        out
    }

    fn finish(&mut self, h: f64) {
        for i in 0..self.pos.len() {
            if self.pins[i].is_some() {
                self.vel[i] = V3::ZERO;
                continue;
            }
            self.vel[i] = (self.pos[i] - self.old[i]) / h;
        }
    }

    /// Bounce and friction against the bodies touched this substep; a dynamic body and the point
    /// share the contact impulse by inverse mass, with a push that separates them by `depth`.
    fn contact_velocity(&mut self, world: &mut World, contacts: &[RContact], h: f64) {
        for c in contacts {
            let wp = self.point_w(c.point);
            if wp <= 0.0 {
                continue;
            }
            let rb = world.body(c.body);
            let dynamic = rb.kind == BodyKind::Dynamic;
            let r = c.at - rb.p;
            let kb = if dynamic {
                let rn = r.cross(c.normal);
                rb.inv_mass + rn.dot(rb.inv_i.mul_v(rn))
            } else {
                0.0
            };
            let mu = (self.params.friction * rb.friction).sqrt();
            let e = self.params.restitution.max(rb.restitution);
            let n = c.normal;
            let denom = wp + kb;
            let vr = rb.velocity_at(c.at);
            let vn = (self.point_vel(c.point) - vr).dot(n);
            let mut jn = 0.0;
            if dynamic {
                let mut target = (0.2 * c.depth / h).min(1.5);
                if vn < -0.5 {
                    target = target.max(-e * vn);
                }
                jn = ((target - vn) / denom).max(0.0);
            } else if vn < 0.0 {
                let target = if vn < -0.5 { -e * vn } else { 0.0 };
                jn = (target - vn) / denom;
            }
            if jn > 0.0 {
                self.push_point_vel(c.point, n * (jn * wp));
                if dynamic {
                    world.apply_impulse_at(c.body, -n * jn, c.at);
                }
            }
            // a resting contact on a static body holds with the push that kept it out
            let jn_f = if dynamic {
                jn
            } else {
                jn.max(c.depth / h / wp)
            };
            let rb = world.body(c.body);
            let rel = self.point_vel(c.point) - rb.velocity_at(c.at);
            let vt = rel - n * rel.dot(n);
            let vtl = vt.len();
            if vtl > 1e-9 && jn_f > 0.0 {
                let jt = (vtl / denom).min(mu * jn_f);
                let dir = vt / vtl;
                self.push_point_vel(c.point, -dir * (jt * wp));
                if dynamic {
                    world.apply_impulse_at(c.body, dir * jt, c.at);
                }
            }
        }
    }
}

struct Lambdas {
    dist: Vec<f64>,
    tet: Vec<f64>,
    vol: f64,
}

/// Soft-body collision: every point against the triangles of every soft body (its own too when
/// `self_collide`), solved by position.
/// Build a surface body (cloth, balloon): every vertex is a node, every edge keeps its length,
/// every shared edge gets a bending link and a closed mesh keeps its volume (`pressure`).
pub struct SurfaceSpec {
    pub density: f64,
    pub edge_compliance: f64,
    pub bend_compliance: f64,
    /// `Some((pressure, compliance))` on a closed mesh keeps `pressure` times its volume.
    pub volume: Option<(f64, f64)>,
}

/// Does every edge of the triangles belong to exactly two of them?
pub fn is_closed(tris: &[[usize; 3]]) -> bool {
    let mut count: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for t in tris {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    !count.is_empty() && count.values().all(|c| *c == 2)
}

impl SoftBody3 {
    pub fn from_surface(
        verts: &[V3],
        tris: &[[usize; 3]],
        groups: &[usize],
        spec: &SurfaceSpec,
        params: SoftParams,
    ) -> SoftBody3 {
        let mut mass = vec![0.0; verts.len()];
        for t in tris {
            let a = (verts[t[1]] - verts[t[0]])
                .cross(verts[t[2]] - verts[t[0]])
                .len()
                / 2.0;
            for k in t {
                mass[*k] += spec.density * a / 3.0;
            }
        }
        let points = (0..verts.len()).map(Point::node).collect();
        let mut b = SoftBody3::new(verts.to_vec(), mass, points, params);
        b.set_surface(tris.to_vec(), groups.to_vec());
        let mut edges: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
        for (ti, t) in tris.iter().enumerate() {
            for k in 0..3 {
                let (x, y) = (t[k], t[(k + 1) % 3]);
                edges.entry((x.min(y), x.max(y))).or_default().push(ti);
            }
        }
        for ((x, y), ts) in &edges {
            b.add_edge(*x, *y, spec.edge_compliance);
            if ts.len() == 2 {
                let opp = |ti: usize| {
                    *tris[ti]
                        .iter()
                        .find(|v| *v != x && *v != y)
                        .expect("opposite vertex")
                };
                let (c, d) = (opp(ts[0]), opp(ts[1]));
                if c != d {
                    b.add_bend(c, d, ts[0], ts[1], spec.bend_compliance);
                }
            }
        }
        if let Some((pressure, compliance)) = spec.volume {
            if is_closed(tris) {
                b.set_volume(pressure, compliance);
            }
        }
        b
    }

    /// Build a solid body: tetrahedra fill the closed mesh (`cells` lattice cells along its longest
    /// side); the mesh vertices are embedded in them.
    #[allow(clippy::too_many_arguments)]
    pub fn from_solid(
        verts: &[V3],
        tris: &[[usize; 3]],
        groups: &[usize],
        cells: usize,
        density: f64,
        compliance: f64,
        pressure: f64,
        params: SoftParams,
    ) -> Option<SoftBody3> {
        let t = crate::tet::tetrahedralize(verts, tris, cells)?;
        let mut mass = vec![0.0; t.nodes.len()];
        for n in &t.tets {
            let (a, b, c, d) = (t.nodes[n[0]], t.nodes[n[1]], t.nodes[n[2]], t.nodes[n[3]]);
            let v = (b - a).cross(c - a).dot(d - a).abs() / 6.0;
            for k in n {
                mass[*k] += density * v / 4.0;
            }
        }
        // the mesh's own points first (they are what is drawn), then every lattice node as a
        // collision-only point, so a coarse mesh cannot slip through an obstacle between its vertices
        let mut points = t.points;
        for i in 0..t.nodes.len() {
            if t.inside[i] {
                points.push(Point::node(i));
            }
        }
        let mut b = SoftBody3::new(t.nodes.clone(), mass, points, params);
        b.set_surface(tris.to_vec(), groups.to_vec());
        let mut edges: BTreeMap<(usize, usize), ()> = BTreeMap::new();
        for n in &t.tets {
            for i in 0..4 {
                for j in i + 1..4 {
                    edges.insert((n[i].min(n[j]), n[i].max(n[j])), ());
                }
            }
        }
        for (x, y) in edges.keys() {
            b.add_brace(*x, *y, compliance);
        }
        for n in &t.tets {
            let rest = b.tet_volume(*n);
            b.tets.push(TetC {
                n: *n,
                rest: rest * pressure,
                compliance,
            });
        }
        let mean_edge = {
            let l: f64 = b.dist.iter().map(|d| d.rest).sum();
            l / b.dist.len().max(1) as f64
        };
        b.exclude = mean_edge * 2.0;
        Some(b)
    }
}

fn collide_soft(bodies: &mut [SoftBody3]) {
    // a grid over all live triangles, in a fixed order
    let mut cell = 0.0f64;
    let mut count = 0usize;
    for b in bodies.iter() {
        cell = cell.max(2.0 * b.params.radius);
        for (i, t) in b.tris.iter().enumerate() {
            if b.tri_alive[i] {
                let (p, q) = (b.point_pos(t[0]), b.point_pos(t[1]));
                cell = cell.max((p - q).len());
                count += 1;
            }
        }
    }
    if count == 0 || bodies.len() == 1 && !bodies[0].params.self_collide {
        return;
    }
    let key = |p: V3| {
        (
            (p.x / cell).floor() as i64,
            (p.y / cell).floor() as i64,
            (p.z / cell).floor() as i64,
        )
    };
    let mut grid: BTreeMap<(i64, i64, i64), Vec<(usize, usize)>> = BTreeMap::new();
    for (bi, b) in bodies.iter().enumerate() {
        for (ti, t) in b.tris.iter().enumerate() {
            if !b.tri_alive[ti] {
                continue;
            }
            let p = [b.point_pos(t[0]), b.point_pos(t[1]), b.point_pos(t[2])];
            let lo =
                p[0].min(p[1]).min(p[2]) - v3(b.params.radius, b.params.radius, b.params.radius);
            let hi =
                p[0].max(p[1]).max(p[2]) + v3(b.params.radius, b.params.radius, b.params.radius);
            let (kl, kh) = (key(lo), key(hi));
            for x in kl.0..=kh.0 {
                for y in kl.1..=kh.1 {
                    for z in kl.2..=kh.2 {
                        grid.entry((x, y, z)).or_default().push((bi, ti));
                    }
                }
            }
        }
    }
    for ai in 0..bodies.len() {
        for pi in 0..bodies[ai].points.len() {
            let p = bodies[ai].point_pos(pi);
            let Some(cands) = grid.get(&key(p)) else {
                continue;
            };
            for &(bi, ti) in cands {
                if bi == ai && !bodies[ai].params.self_collide {
                    continue;
                }
                let tri = bodies[bi].tris[ti];
                if bi == ai {
                    if tri.contains(&pi) {
                        continue;
                    }
                    let rp = bodies[ai].rest_points[pi];
                    let ex = bodies[ai].exclude;
                    if tri
                        .iter()
                        .any(|v| (bodies[ai].rest_points[*v] - rp).len() < ex)
                    {
                        continue;
                    }
                }
                let tp = [
                    bodies[bi].point_pos(tri[0]),
                    bodies[bi].point_pos(tri[1]),
                    bodies[bi].point_pos(tri[2]),
                ];
                let p = bodies[ai].point_pos(pi);
                let (q, bary) = tri_closest(p, tp[0], tp[1], tp[2]);
                let min_d = bodies[ai].params.radius + bodies[bi].params.radius;
                let dv = p - q;
                let dist = dv.len();
                if dist >= min_d {
                    continue;
                }
                let n = if dist > 1e-12 {
                    dv / dist
                } else {
                    (tp[1] - tp[0]).cross(tp[2] - tp[0]).normalized()
                };
                let c = dist - min_d;
                let wp = bodies[ai].point_w(pi);
                let wt: [f64; 3] = [
                    bodies[bi].point_w(tri[0]),
                    bodies[bi].point_w(tri[1]),
                    bodies[bi].point_w(tri[2]),
                ];
                let denom = wp + (0..3).map(|k| bary[k] * bary[k] * wt[k]).sum::<f64>();
                if denom <= 1e-300 {
                    continue;
                }
                let lam = -c / denom;
                bodies[ai].push_point(pi, n * (lam * wp));
                for k in 0..3 {
                    bodies[bi].push_point(tri[k], -n * (lam * bary[k] * wt[k]));
                }
            }
        }
    }
}

/// Step the rigid world and the soft bodies together: `substeps` small steps, each stepping the
/// rigid bodies, then every soft body (constraints, collisions, velocities).
pub fn soft_step(
    world: &mut World,
    bodies: &mut [SoftBody3],
    dt: f64,
    substeps: usize,
    iterations: usize,
) {
    coupled_step3(world, bodies, &mut [], dt, substeps, iterations, 0, 1);
}

/// Step the rigid world, the soft bodies and the fluids together. The soft bodies step every
/// substep; a fluid steps `fluid_substeps` times per frame (every `substeps / fluid_substeps`-th
/// substep, with a step that long).
#[allow(clippy::too_many_arguments)]
pub fn coupled_step3(
    world: &mut World,
    bodies: &mut [SoftBody3],
    fluids: &mut [crate::fluid::Fluid3],
    dt: f64,
    substeps: usize,
    iterations: usize,
    fluid_iterations: usize,
    fluid_substeps: usize,
) {
    let substeps = if bodies.is_empty() {
        fluid_substeps.max(1)
    } else {
        substeps.max(1)
    };
    let ratio = (substeps / fluid_substeps.max(1)).max(1);
    let h = dt / substeps as f64;
    let gravity = world.gravity;
    for s in 0..substeps {
        world.step(h);
        world.refresh_shapes();
        if !bodies.is_empty() {
            let mut lams: Vec<Lambdas> = bodies
                .iter()
                .map(|b| Lambdas {
                    dist: vec![0.0; b.dist.len()],
                    tet: vec![0.0; b.tets.len()],
                    vol: 0.0,
                })
                .collect();
            for b in bodies.iter_mut() {
                b.predict(h, gravity, s, substeps);
            }
            for _ in 0..iterations {
                for (b, l) in bodies.iter_mut().zip(lams.iter_mut()) {
                    b.solve(h, l);
                }
            }
            let mut contacts: Vec<Vec<RContact>> = vec![];
            for b in bodies.iter_mut() {
                contacts.push(b.collide_rigid(world));
            }
            collide_soft(bodies);
            for b in bodies.iter_mut() {
                b.finish(h);
            }
            for (b, c) in bodies.iter_mut().zip(contacts.iter()) {
                b.contact_velocity(world, c, h);
            }
        }
        if !fluids.is_empty() && (s + 1) % ratio == 0 {
            world.refresh_shapes();
            for f in fluids.iter_mut() {
                f.substep(
                    h * ratio as f64,
                    gravity,
                    fluid_iterations,
                    Some(world),
                    bodies,
                );
            }
        }
    }
}
