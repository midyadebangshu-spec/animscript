//! Soft bodies: extended position-based dynamics (XPBD). Vertices are particles; distance
//! constraints keep edges at their rest length, area constraints give volume and pressure, pins
//! hold vertices at (or drag them with) a target. Vertices collide with rigid fixtures (pushing
//! dynamic bodies back) and with the edges of soft bodies. Units: metres, kilograms, seconds.
//! Every iteration order is fixed, so results are reproducible.
use crate::math::*;
use crate::shape::{find_pairs, Aabb};
use crate::world::{BodyKind, World};

#[derive(Clone, Debug)]
pub struct SoftParams {
    /// Collision radius of a vertex (and half the thickness of an edge), metres.
    pub radius: f64,
    pub friction: f64,
    pub restitution: f64,
    /// Velocity damping per second.
    pub damping: f64,
    /// Let this body's vertices collide with its own edges too.
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

#[derive(Clone, Debug)]
struct Dist {
    a: usize,
    b: usize,
    rest: f64,
    compliance: f64,
    collider: bool,
}

#[derive(Clone, Debug)]
struct Area {
    ring: Vec<usize>,
    target: f64,
    compliance: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Pin {
    pub target: Vec2,
    last: Vec2,
}

#[derive(Clone, Debug)]
pub struct SoftBody {
    pub pos: Vec<Vec2>,
    pub vel: Vec<Vec2>,
    pub mass: Vec<f64>,
    pub pins: Vec<Option<Pin>>,
    pub params: SoftParams,
    dist: Vec<Dist>,
    area: Vec<Area>,
}

fn signed_area(pos: &[Vec2], ring: &[usize]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| pos[ring[i]].cross(pos[ring[(i + 1) % n]]))
        .sum::<f64>()
        / 2.0
}

impl SoftBody {
    pub fn new(pos: Vec<Vec2>, mass: Vec<f64>, params: SoftParams) -> SoftBody {
        let n = pos.len();
        SoftBody {
            pos,
            vel: vec![Vec2::ZERO; n],
            mass,
            pins: vec![None; n],
            params,
            dist: vec![],
            area: vec![],
        }
    }
    /// Keep `a` and `b` at their current distance; the edge also collides.
    pub fn add_distance(&mut self, a: usize, b: usize, compliance: f64) {
        let rest = (self.pos[a] - self.pos[b]).length();
        self.dist.push(Dist {
            a,
            b,
            rest,
            compliance,
            collider: true,
        });
    }
    /// The same, for an internal brace (a diagonal or a bend link) that does not collide.
    pub fn add_internal(&mut self, a: usize, b: usize, compliance: f64) {
        let rest = (self.pos[a] - self.pos[b]).length();
        self.dist.push(Dist {
            a,
            b,
            rest,
            compliance,
            collider: false,
        });
    }
    /// Keep the polygon `ring` at `scale` times its current area (`scale > 1` inflates it).
    pub fn add_area(&mut self, ring: &[usize], scale: f64, compliance: f64) {
        let target = signed_area(&self.pos, ring) * scale;
        self.area.push(Area {
            ring: ring.to_vec(),
            target,
            compliance,
        });
    }
    pub fn pin(&mut self, i: usize, target: Vec2) {
        self.pins[i] = Some(Pin {
            target,
            last: target,
        });
        self.vel[i] = Vec2::ZERO;
    }
    pub fn set_pin_target(&mut self, i: usize, target: Vec2) {
        if let Some(p) = &mut self.pins[i] {
            p.last = p.target;
            p.target = target;
        }
    }
    pub fn unpin(&mut self, i: usize) {
        self.pins[i] = None;
    }
    /// Largest edge strain, `|length - rest| / rest`.
    pub fn max_strain(&self) -> f64 {
        self.dist
            .iter()
            .filter(|d| d.rest > 1e-12)
            .map(|d| (((self.pos[d.a] - self.pos[d.b]).length() - d.rest) / d.rest).abs())
            .fold(0.0, f64::max)
    }
    pub fn total_mass(&self) -> f64 {
        self.mass.iter().sum()
    }
    fn w(&self, i: usize) -> f64 {
        if self.pins[i].is_some() || self.mass[i] <= 0.0 {
            0.0
        } else {
            1.0 / self.mass[i]
        }
    }
}

struct Contact {
    body: usize,
    v0: usize,
    v1: Option<usize>,
    n: Vec2,
    sep: f64,
    xref: Vec2,
    hit: usize,
    vb: Vec2,
    point: Vec2,
    dynamic: bool,
    acc: f64,
    react: f64,
}

struct Pair {
    vb: usize,
    v: usize,
    eb: usize,
    a: usize,
    b: usize,
    side: f64,
}

fn perp_unit(d: Vec2) -> Vec2 {
    v2(-d.y, d.x).normalized()
}

/// Advance every soft body by `dt` with `substeps` substeps of `iterations` constraint passes each.
/// Contacts with `world` bodies push dynamic ones back (apply impulses). The world is stepped here,
/// one substep at a time in lockstep with the soft bodies, so neither sees stale poses; the caller
/// must not step it for this frame as well.
#[allow(clippy::needless_range_loop)] // the loops index several parallel per-vertex arrays
pub fn soft_step(
    bodies: &mut [SoftBody],
    mut world: Option<&mut World>,
    gravity: Vec2,
    dt: f64,
    substeps: usize,
    iterations: usize,
) {
    if dt <= 0.0 || bodies.is_empty() {
        return;
    }
    let n = substeps.max(1);
    let h = dt / n as f64;
    for s in 0..n {
        let frac = (s + 1) as f64 / n as f64;
        let mut prev: Vec<Vec<Vec2>> = vec![];
        let mut vpred: Vec<Vec<Vec2>> = vec![];
        for b in bodies.iter_mut() {
            prev.push(b.pos.clone());
            for i in 0..b.pos.len() {
                if let Some(p) = b.pins[i] {
                    let target = p.last + (p.target - p.last) * frac;
                    b.vel[i] = (target - b.pos[i]) / h;
                    b.pos[i] = target;
                } else {
                    b.vel[i] += gravity * h;
                    b.vel[i] = b.vel[i] * (1.0 / (1.0 + b.params.damping * h));
                    b.pos[i] += b.vel[i] * h;
                }
            }
            vpred.push(b.vel.clone());
        }
        // rigid contacts, detected once per substep at the predicted positions
        let mut contacts: Vec<Contact> = vec![];
        if let Some(w) = world.as_deref() {
            for (bi, b) in bodies.iter().enumerate() {
                let r = b.params.radius;
                for v in 0..b.pos.len() {
                    if b.w(v) == 0.0 {
                        continue;
                    }
                    let margin = 0.02 + b.vel[v].length() * h;
                    for hit in w.circle_hits(b.pos[v], r, margin) {
                        contacts.push(Contact {
                            body: bi,
                            v0: v,
                            v1: None,
                            n: hit.normal,
                            sep: hit.sep,
                            xref: b.pos[v],
                            hit: hit.body,
                            vb: hit.vel,
                            point: hit.point,
                            dynamic: w.body(hit.body).kind == BodyKind::Dynamic,
                            acc: 0.0,
                            react: 0.0,
                        });
                    }
                }
                for d in b.dist.iter().filter(|d| d.collider) {
                    if b.w(d.a) == 0.0 && b.w(d.b) == 0.0 {
                        continue;
                    }
                    let mid = (b.pos[d.a] + b.pos[d.b]) * 0.5;
                    let margin = 0.02 + ((b.vel[d.a] + b.vel[d.b]) * 0.5).length() * h;
                    for hit in w.circle_hits(mid, r, margin) {
                        contacts.push(Contact {
                            body: bi,
                            v0: d.a,
                            v1: Some(d.b),
                            n: hit.normal,
                            sep: hit.sep,
                            xref: mid,
                            hit: hit.body,
                            vb: hit.vel,
                            point: hit.point,
                            dynamic: w.body(hit.body).kind == BodyKind::Dynamic,
                            acc: 0.0,
                            react: 0.0,
                        });
                    }
                }
            }
        }
        // soft-soft: vertices against edges, remembering which side of the edge each vertex was on
        let mut boxes: Vec<Aabb> = vec![];
        let need_boxes = bodies.len() > 1 || bodies.iter().any(|b| b.params.self_collide);
        let mut tags: Vec<(bool, usize, usize, usize)> = vec![]; // (is_edge, body, a, b)
        for (bi, b) in bodies.iter().enumerate().filter(|_| need_boxes) {
            let r = b.params.radius * 2.0 + 0.01;
            for v in 0..b.pos.len() {
                let m = r + (b.pos[v] - prev[bi][v]).length();
                boxes.push(
                    Aabb {
                        lo: v2(b.pos[v].x.min(prev[bi][v].x), b.pos[v].y.min(prev[bi][v].y)),
                        hi: v2(b.pos[v].x.max(prev[bi][v].x), b.pos[v].y.max(prev[bi][v].y)),
                    }
                    .expanded(m),
                );
                tags.push((false, bi, v, v));
            }
        }
        for (bi, b) in bodies.iter().enumerate().filter(|_| need_boxes) {
            let r = b.params.radius * 2.0 + 0.01;
            for d in b.dist.iter().filter(|d| d.collider) {
                let pts = [b.pos[d.a], b.pos[d.b], prev[bi][d.a], prev[bi][d.b]];
                let lo = v2(
                    pts.iter().map(|p| p.x).fold(f64::MAX, f64::min),
                    pts.iter().map(|p| p.y).fold(f64::MAX, f64::min),
                );
                let hi = v2(
                    pts.iter().map(|p| p.x).fold(f64::MIN, f64::max),
                    pts.iter().map(|p| p.y).fold(f64::MIN, f64::max),
                );
                boxes.push(Aabb { lo, hi }.expanded(r));
                tags.push((true, bi, d.a, d.b));
            }
        }
        let mut pairs: Vec<Pair> = vec![];
        let need_pairs = bodies.len() > 1 || bodies.iter().any(|b| b.params.self_collide);
        for (i, j) in if need_pairs {
            find_pairs(&boxes)
        } else {
            vec![]
        } {
            let (ti, tj) = (tags[i], tags[j]);
            let (vt, et) = match (ti.0, tj.0) {
                (false, true) => (ti, tj),
                (true, false) => (tj, ti),
                _ => continue,
            };
            let (vb, v, eb, a, b) = (vt.1, vt.2, et.1, et.2, et.3);
            if vb == eb && (!bodies[vb].params.self_collide || v == a || v == b) {
                continue;
            }
            if bodies[vb].w(v) == 0.0 && bodies[eb].w(a) == 0.0 && bodies[eb].w(b) == 0.0 {
                continue;
            }
            let ab = prev[eb][b] - prev[eb][a];
            let side = if perp_unit(ab).dot(prev[vb][v] - prev[eb][a]) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            pairs.push(Pair {
                vb,
                v,
                eb,
                a,
                b,
                side,
            });
        }
        let mut lam_d: Vec<Vec<f64>> = bodies.iter().map(|b| vec![0.0; b.dist.len()]).collect();
        let mut lam_a: Vec<Vec<f64>> = bodies.iter().map(|b| vec![0.0; b.area.len()]).collect();
        let inv_h2 = 1.0 / (h * h);
        for it in 0..iterations {
            let reverse = it % 2 == 1; // sweep both ways so a heavy end mass is felt along the whole chain
            for (bi, b) in bodies.iter_mut().enumerate() {
                let dists = std::mem::take(&mut b.dist);
                for q in 0..dists.len() {
                    let k = if reverse { dists.len() - 1 - q } else { q };
                    let d = &dists[k];
                    let (wa, wb) = (b.w(d.a), b.w(d.b));
                    if wa + wb == 0.0 {
                        continue;
                    }
                    let delta = b.pos[d.a] - b.pos[d.b];
                    let len = delta.length();
                    if len < 1e-12 {
                        continue;
                    }
                    let nrm = delta / len;
                    let alpha = d.compliance * inv_h2;
                    let dl = (-(len - d.rest) - alpha * lam_d[bi][k]) / (wa + wb + alpha);
                    lam_d[bi][k] += dl;
                    b.pos[d.a] += nrm * (dl * wa);
                    b.pos[d.b] -= nrm * (dl * wb);
                }
                b.dist = dists;
                let areas = std::mem::take(&mut b.area);
                for q in 0..areas.len() {
                    let k = if reverse { areas.len() - 1 - q } else { q };
                    let ar = &areas[k];
                    let m = ar.ring.len();
                    let c = signed_area(&b.pos, &ar.ring) - ar.target;
                    let grad = |pos: &[Vec2], i: usize| {
                        let (p, q) = (pos[ar.ring[(i + m - 1) % m]], pos[ar.ring[(i + 1) % m]]);
                        v2(q.y - p.y, p.x - q.x) * 0.5
                    };
                    let denom: f64 = (0..m)
                        .map(|i| b.w(ar.ring[i]) * grad(&b.pos, i).length_sq())
                        .sum();
                    let alpha = ar.compliance * inv_h2;
                    if denom + alpha < 1e-300 {
                        continue;
                    }
                    let dl = (-c - alpha * lam_a[bi][k]) / (denom + alpha);
                    lam_a[bi][k] += dl;
                    // gradients are taken at the positions before any vertex of this ring moved
                    let moves: Vec<(usize, Vec2)> = (0..m)
                        .map(|i| (ar.ring[i], grad(&b.pos, i) * (dl * b.w(ar.ring[i]))))
                        .collect();
                    for (v, d) in moves {
                        b.pos[v] += d;
                    }
                }
                b.area = areas;
            }
            for c in contacts.iter_mut() {
                let b = &mut bodies[c.body];
                let x = match c.v1 {
                    Some(v1) => (b.pos[c.v0] + b.pos[v1]) * 0.5,
                    None => b.pos[c.v0],
                };
                let clearance = c.sep + (x - c.xref).dot(-c.n);
                if clearance < 0.0 {
                    let push = -clearance;
                    for v in std::iter::once(c.v0).chain(c.v1) {
                        if b.w(v) > 0.0 {
                            b.pos[v] += -c.n * push;
                            c.react += b.mass[v] * push;
                        }
                    }
                    c.acc += push;
                }
            }
            for p in &pairs {
                let (pa, pb, pv) = (
                    bodies[p.eb].pos[p.a],
                    bodies[p.eb].pos[p.b],
                    bodies[p.vb].pos[p.v],
                );
                let ab = pb - pa;
                let l2 = ab.length_sq();
                if l2 < 1e-18 {
                    continue;
                }
                let t = ((pv - pa).dot(ab) / l2).clamp(0.0, 1.0);
                let perp = perp_unit(ab) * p.side;
                let signed = perp.dot(pv - (pa + ab * t));
                let need = bodies[p.vb].params.radius + bodies[p.eb].params.radius;
                if signed >= need {
                    continue;
                }
                let depth = need - signed;
                let (wv, wa, wb) = (
                    bodies[p.vb].w(p.v),
                    bodies[p.eb].w(p.a),
                    bodies[p.eb].w(p.b),
                );
                let denom = wv + wa * (1.0 - t) * (1.0 - t) + wb * t * t;
                if denom <= 0.0 {
                    continue;
                }
                let lam = depth / denom;
                bodies[p.vb].pos[p.v] += perp * (lam * wv);
                bodies[p.eb].pos[p.a] -= perp * (lam * wa * (1.0 - t));
                bodies[p.eb].pos[p.b] -= perp * (lam * wb * t);
            }
        }
        // velocities from the position change, then contact response
        for (bi, b) in bodies.iter_mut().enumerate() {
            for i in 0..b.pos.len() {
                if b.pins[i].is_none() {
                    b.vel[i] = (b.pos[i] - prev[bi][i]) / h;
                }
            }
        }
        for c in &contacts {
            if c.acc <= 0.0 {
                continue;
            }
            let params = bodies[c.body].params.clone();
            for v in std::iter::once(c.v0).chain(c.v1) {
                if bodies[c.body].w(v) == 0.0 {
                    continue;
                }
                let pre = vpred[c.body][v] - c.vb;
                let vn_pre = pre.dot(c.n);
                let mut rel = bodies[c.body].vel[v] - c.vb;
                if vn_pre > 1.0 && params.restitution > 0.0 {
                    rel += c.n * (-params.restitution * vn_pre - rel.dot(c.n));
                }
                let t = rel - c.n * rel.dot(c.n);
                let tl = t.length();
                if tl > 1e-12 {
                    let keep = (1.0 - params.friction * (c.acc / h) / tl).max(0.0);
                    rel = c.n * rel.dot(c.n) + t * keep;
                }
                bodies[c.body].vel[v] = rel + c.vb;
            }
            if c.dynamic && c.react > 0.0 {
                if let Some(w) = world.as_deref_mut() {
                    w.apply_impulse_at(c.hit, c.n * (c.react / h), c.point);
                }
            }
        }
        if let Some(w) = world.as_deref_mut() {
            w.step(h);
        }
    }
    for b in bodies.iter_mut() {
        for p in b.pins.iter_mut().flatten() {
            p.last = p.target;
        }
    }
}
