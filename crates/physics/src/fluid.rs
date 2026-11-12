//! Position based fluids (Macklin and Mueller) in 2D. Every particle stands for `spacing^2` of fluid;
//! a density constraint pushes neighbours apart until the local density matches the rest density,
//! then XSPH smoothing gives viscosity. Particles collide with rigid fixtures (pushing dynamic bodies
//! back, which is what makes a box float and a ball splash) and treat soft bodies as obstacles.
//! Neighbours come from a sorted cell list, so every sum runs in a fixed order and results are
//! reproducible. Metres, kilograms, seconds.
use crate::math::*;
use crate::soft::SoftBody;
use crate::world::{BodyKind, World};

#[derive(Clone, Debug)]
pub struct FluidParams {
    /// Distance between particles at rest, metres.
    pub spacing: f64,
    /// Kilograms per square metre (1 = water in this unit system).
    pub density: f64,
    /// XSPH smoothing strength, 0 (water) to about 0.5 (syrup).
    pub viscosity: f64,
    /// Attraction between neighbours (droplets, 0 = none).
    pub cohesion: f64,
    /// Most particles this fluid may hold.
    pub max: usize,
    /// Amount of foam and spray thrown by fast, breaking water: 0 (none) to 1 (a lot).
    pub foam: f64,
    /// Seconds a foam particle lives in the water (spray in the air fades faster).
    pub foam_life: f64,
    /// Most foam particles at once.
    pub foam_max: usize,
}

impl Default for FluidParams {
    fn default() -> FluidParams {
        FluidParams {
            spacing: 0.08,
            density: 1.0,
            viscosity: 0.05,
            cohesion: 0.0,
            max: 4000,
            foam: 0.0,
            foam_life: 1.5,
            foam_max: 2000,
        }
    }
}

/// A secondary particle: spray in the air, foam riding the surface. It never acts on the water.
#[derive(Clone, Copy, Debug)]
pub struct Foam {
    pub pos: Vec2,
    pub vel: Vec2,
    pub age: f64,
    pub life: f64,
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A value in [0, 1) for one random draw of the foam of one substep.
fn unit(seed: u64, n: u64, attr: u64) -> f64 {
    let h = splitmix(
        splitmix(seed ^ n.wrapping_mul(0xd6e8_feb8_6659_fd93))
            ^ attr.wrapping_mul(0xa076_1d64_78bd_642f),
    );
    (h >> 11) as f64 / (1u64 << 53) as f64
}

pub struct Fluid {
    params: FluidParams,
    pub pos: Vec<Vec2>,
    pub vel: Vec<Vec2>,
    /// Foam and spray (empty unless `params.foam > 0`).
    pub foam: Vec<Foam>,
    seed: u64,
    steps: u64,
    h: f64,
    c_poly6: f64,
    c_spiky: f64,
    rho0: f64,
    eps: f64,
    wq: f64,
    bounds: Option<(Vec2, Vec2)>,
}

struct Contact {
    i: usize,
    n: Vec2,
    sep: f64,
    xref: Vec2,
    body: usize,
    point: Vec2,
    dynamic: bool,
    acc: f64,
}

impl Fluid {
    pub fn new(params: FluidParams) -> Fluid {
        let h = 2.0 * params.spacing;
        let c_poly6 = 4.0 / (std::f64::consts::PI * h.powi(8));
        let c_spiky = 30.0 / (std::f64::consts::PI * h.powi(5));
        let mut f = Fluid {
            params,
            pos: vec![],
            vel: vec![],
            foam: vec![],
            seed: 0,
            steps: 0,
            h,
            c_poly6,
            c_spiky,
            rho0: 0.0,
            eps: 0.0,
            wq: 0.0,
            bounds: None,
        };
        // rest density and the gradient normalisation, from a perfect lattice
        let s = f.params.spacing;
        let mut rho = f.w(0.0);
        let (mut gsum, mut gsq) = (Vec2::ZERO, 0.0);
        let mut grads = vec![];
        for j in -4i32..=4 {
            for i in -4i32..=4 {
                if i == 0 && j == 0 {
                    continue;
                }
                let d = v2(i as f64 * s, j as f64 * s);
                let r2 = d.length_sq();
                if r2 < h * h {
                    rho += f.w(r2);
                    grads.push(f.grad(d, r2.sqrt()));
                }
            }
        }
        f.rho0 = rho;
        for g in grads {
            let g = g / rho;
            gsum += g;
            gsq += g.length_sq();
        }
        f.eps = 0.2 * (gsq + gsum.length_sq());
        f.wq = f.w((0.2 * h) * (0.2 * h));
        f
    }

    /// The seed of the random draws that decide where foam is born.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }
    pub fn add_foam(&mut self, pos: Vec2, vel: Vec2) {
        if self.foam.len() < self.params.foam_max {
            self.foam.push(Foam {
                pos,
                vel,
                age: 0.0,
                life: self.params.foam_life,
            });
        }
    }
    pub fn params(&self) -> &FluidParams {
        &self.params
    }
    pub fn len(&self) -> usize {
        self.pos.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }
    /// Remove particles that leave the box (so lost water does not fill the pool).
    pub fn set_bounds(&mut self, lo: Vec2, hi: Vec2) {
        self.bounds = Some((lo, hi));
    }
    /// Add particles at rest; returns how many fit under `max`.
    pub fn add(&mut self, pts: &[Vec2]) -> usize {
        let room = self.params.max.saturating_sub(self.pos.len());
        let k = room.min(pts.len());
        self.pos.extend_from_slice(&pts[..k]);
        self.vel.extend(std::iter::repeat_n(Vec2::ZERO, k));
        k
    }
    pub fn add_with_velocity(&mut self, pts: &[(Vec2, Vec2)]) -> usize {
        let room = self.params.max.saturating_sub(self.pos.len());
        let k = room.min(pts.len());
        for (p, v) in &pts[..k] {
            self.pos.push(*p);
            self.vel.push(*v);
        }
        k
    }

    /// poly6 kernel (2D) of the squared distance.
    fn w(&self, r2: f64) -> f64 {
        let h2 = self.h * self.h;
        if r2 >= h2 {
            0.0
        } else {
            self.c_poly6 * (h2 - r2).powi(3)
        }
    }
    /// spiky kernel gradient (2D) for the offset `d` from a neighbour to this particle.
    fn grad(&self, d: Vec2, r: f64) -> Vec2 {
        if r >= self.h || r < 1e-9 {
            Vec2::ZERO
        } else {
            d * (-self.c_spiky * (self.h - r) * (self.h - r) / r)
        }
    }

    /// Candidate neighbours within 1.3 h of each particle, in a fixed order: `(start, list)`.
    fn neighbours(&self, xs: &[Vec2]) -> (Vec<usize>, Vec<u32>) {
        let cell = 1.3 * self.h;
        let key = |p: Vec2| ((p.y / cell).floor() as i64, (p.x / cell).floor() as i64);
        let mut cells: Vec<(i64, i64, u32)> = xs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (cy, cx) = key(*p);
                (cy, cx, i as u32)
            })
            .collect();
        cells.sort_unstable();
        let reach2 = cell * cell;
        let mut start = Vec::with_capacity(xs.len() + 1);
        let mut list = vec![];
        for (i, p) in xs.iter().enumerate() {
            start.push(list.len());
            let (cy, cx) = key(*p);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (ty, tx) = (cy + dy, cx + dx);
                    let mut k = cells.partition_point(|c| (c.0, c.1) < (ty, tx));
                    while k < cells.len() && (cells[k].0, cells[k].1) == (ty, tx) {
                        let j = cells[k].2 as usize;
                        if j != i && (xs[j] - *p).length_sq() < reach2 {
                            list.push(j as u32);
                        }
                        k += 1;
                    }
                }
            }
        }
        start.push(list.len());
        (start, list)
    }

    /// The median over particles of density / rest density (the bulk, away from walls and surface).
    pub fn median_density_ratio(&self) -> f64 {
        let (start, list) = self.neighbours(&self.pos);
        let mut r: Vec<f64> = (0..self.pos.len())
            .map(|i| {
                let mut rho = self.w(0.0);
                for &j in &list[start[i]..start[i + 1]] {
                    rho += self.w((self.pos[i] - self.pos[j as usize]).length_sq());
                }
                rho / self.rho0
            })
            .collect();
        r.sort_by(|a, b| a.total_cmp(b));
        r.get(r.len() / 2).copied().unwrap_or(1.0)
    }

    /// One substep of `dt`. `soft` bodies are one-way obstacles.
    pub fn substep(
        &mut self,
        dt: f64,
        gravity: Vec2,
        iterations: usize,
        mut world: Option<&mut World>,
        soft: &[SoftBody],
    ) {
        let n = self.pos.len();
        if n == 0 {
            self.update_foam(dt, gravity, world.as_deref());
            return;
        }
        let s = self.params.spacing;
        let h2 = self.h * self.h;
        let mut xs: Vec<Vec2> = Vec::with_capacity(n);
        for i in 0..n {
            self.vel[i] += gravity * dt;
            xs.push(self.pos[i] + self.vel[i] * dt);
        }
        let (start, list) = self.neighbours(&xs);
        // contacts with rigid fixtures, found once at the predicted positions
        let r_wall = 0.5 * s;
        let mut contacts: Vec<Contact> = vec![];
        if let Some(w) = world.as_deref() {
            for i in 0..n {
                // found from where the particle was, with a margin for how far it is about to move,
                // so a fast particle cannot jump over a thin wall
                let x = self.pos[i];
                // the density solver can also move it, by up to `max_move` per iteration
                let margin = 0.02 + self.vel[i].length() * dt + iterations as f64 * 0.4 * s;
                for hit in w.circle_hits(x, r_wall, margin) {
                    let dynamic = w.body(hit.body).kind == BodyKind::Dynamic;
                    contacts.push(Contact {
                        i,
                        n: hit.normal,
                        sep: hit.sep,
                        xref: x,
                        body: hit.body,
                        point: hit.point,
                        dynamic,
                        acc: 0.0,
                    });
                }
            }
        }
        // soft bodies as fixed obstacle points for this substep (vertices and samples along edges)
        let mut obst: Vec<(Vec2, f64)> = vec![];
        if !soft.is_empty() {
            let (mut lo, mut hi) = (v2(f64::MAX, f64::MAX), v2(f64::MIN, f64::MIN));
            for x in &xs {
                lo = v2(lo.x.min(x.x), lo.y.min(x.y));
                hi = v2(hi.x.max(x.x), hi.y.max(x.y));
            }
            let pad = 2.0 * s;
            let near = |p: Vec2| {
                p.x > lo.x - pad && p.x < hi.x + pad && p.y > lo.y - pad && p.y < hi.y + pad
            };
            for b in soft {
                let r = b.params.radius.max(0.005);
                for p in &b.pos {
                    if near(*p) {
                        obst.push((*p, r));
                    }
                }
                for (a, c) in b.collider_segments() {
                    let box_near = !(a.x.max(c.x) < lo.x - pad
                        || a.x.min(c.x) > hi.x + pad
                        || a.y.max(c.y) < lo.y - pad
                        || a.y.min(c.y) > hi.y + pad);
                    if !box_near {
                        continue;
                    }
                    let k = (((c - a).length() / (0.5 * s)).ceil() as usize).clamp(1, 64);
                    for q in 1..k {
                        obst.push((a + (c - a) * (q as f64 / k as f64), r));
                    }
                }
            }
        }
        let mut near_obst: Vec<(usize, Vec2, f64)> = vec![];
        if !obst.is_empty() {
            let cell = 2.0 * s;
            let key = |p: Vec2| ((p.y / cell).floor() as i64, (p.x / cell).floor() as i64);
            let mut cells: Vec<(i64, i64, u32)> = obst
                .iter()
                .enumerate()
                .map(|(k, o)| {
                    let (cy, cx) = key(o.0);
                    (cy, cx, k as u32)
                })
                .collect();
            cells.sort_unstable();
            for (i, x) in xs.iter().enumerate() {
                let (cy, cx) = key(*x);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (ty, tx) = (cy + dy, cx + dx);
                        let mut k = cells.partition_point(|c| (c.0, c.1) < (ty, tx));
                        while k < cells.len() && (cells[k].0, cells[k].1) == (ty, tx) {
                            let (p, r) = obst[cells[k].2 as usize];
                            if (*x - p).length() < r + r_wall + 0.5 * s {
                                near_obst.push((i, p, r + r_wall));
                            }
                            k += 1;
                        }
                    }
                }
            }
        }
        let mut lambda = vec![0.0; n];
        let mut dp = vec![Vec2::ZERO; n];
        let max_move = 0.4 * s;
        for _ in 0..iterations {
            for i in 0..n {
                let mut rho = self.w(0.0);
                let (mut gsum, mut gsq) = (Vec2::ZERO, 0.0);
                for &j in &list[start[i]..start[i + 1]] {
                    let d = xs[i] - xs[j as usize];
                    let r2 = d.length_sq();
                    if r2 < h2 {
                        rho += self.w(r2);
                        let g = self.grad(d, r2.sqrt()) / self.rho0;
                        gsum += g;
                        gsq += g.length_sq();
                    }
                }
                let c = (rho / self.rho0 - 1.0).max(0.0);
                lambda[i] = -c / (gsq + gsum.length_sq() + self.eps);
            }
            for i in 0..n {
                let mut d_i = Vec2::ZERO;
                for &j in &list[start[i]..start[i + 1]] {
                    let j = j as usize;
                    let d = xs[i] - xs[j];
                    let r2 = d.length_sq();
                    if r2 < h2 {
                        let tensile = -0.1 * h2 * (self.w(r2) / self.wq).powi(4); // keeps particles from clumping
                        d_i += self.grad(d, r2.sqrt())
                            * ((lambda[i] + lambda[j] + tensile) / self.rho0);
                    }
                }
                let m = d_i.length();
                dp[i] = if m > max_move {
                    d_i * (max_move / m)
                } else {
                    d_i
                };
            }
            for i in 0..n {
                xs[i] += dp[i];
            }
            for c in contacts.iter_mut() {
                let clearance = c.sep + (xs[c.i] - c.xref).dot(-c.n);
                if clearance < 0.0 {
                    xs[c.i] += -c.n * (-clearance);
                    c.acc += -clearance;
                }
            }
            for (i, p, rad) in &near_obst {
                let d = xs[*i] - *p;
                let dist = d.length();
                if dist < *rad {
                    let dir = if dist > 1e-9 { d / dist } else { v2(0.0, -1.0) };
                    xs[*i] += dir * (rad - dist);
                }
            }
        }
        // velocities from the position change, then XSPH viscosity and cohesion
        let old: Vec<Vec2> = (0..n).map(|i| (xs[i] - self.pos[i]) / dt).collect();
        let max_speed = 20.0;
        let mut new_vel = old.clone();
        for i in 0..n {
            let mut rho = self.w(0.0);
            let (mut acc, mut pull) = (Vec2::ZERO, Vec2::ZERO);
            for &j in &list[start[i]..start[i + 1]] {
                let j = j as usize;
                let d = xs[j] - xs[i];
                let r2 = d.length_sq();
                if r2 < h2 {
                    let w = self.w(r2);
                    rho += w;
                    acc += (old[j] - old[i]) * w;
                    let r = r2.sqrt();
                    if r > 1e-9 {
                        pull += d * ((1.0 - r / self.h).powi(2) / r);
                    }
                }
            }
            let mut v = old[i] + acc * (self.params.viscosity / rho);
            v += pull * (self.params.cohesion * 10.0 * dt);
            let sp = v.length();
            new_vel[i] = if sp > max_speed {
                v * (max_speed / sp)
            } else {
                v
            };
        }
        self.pos = xs;
        self.vel = new_vel;
        // water pushes bodies back with the momentum of its push-out
        if let Some(w) = world.as_mut() {
            let m = self.params.density * s * s;
            for c in &contacts {
                if c.dynamic && c.acc > 0.0 {
                    w.apply_impulse_at(c.body, c.n * (m * c.acc / dt), c.point);
                }
            }
        }
        if self.params.foam > 0.0 {
            self.update_foam(dt, gravity, world.as_deref());
            self.spawn_foam(dt, &start, &list);
        }
        self.steps += 1;
        if let Some((lo, hi)) = self.bounds {
            let keep: Vec<bool> = self
                .pos
                .iter()
                .map(|p| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y)
                .collect();
            if keep.iter().any(|k| !k) {
                let mut it = keep.iter();
                self.pos.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.vel.retain(|_| *it.next().unwrap());
            }
        }
    }

    /// Move the existing foam: it rides the water when it is inside it (and floats up), flies like a
    /// projectile in the air, fades, and stops at rigid bodies.
    fn update_foam(&mut self, dt: f64, gravity: Vec2, world: Option<&World>) {
        if self.foam.is_empty() {
            return;
        }
        let h = self.h;
        let cell = h;
        let key = |p: Vec2| ((p.y / cell).floor() as i64, (p.x / cell).floor() as i64);
        let mut cells: Vec<(i64, i64, u32)> = self
            .pos
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (cy, cx) = key(*p);
                (cy, cx, i as u32)
            })
            .collect();
        cells.sort_unstable();
        let s = self.params.spacing;
        for f in self.foam.iter_mut() {
            let (cy, cx) = key(f.pos);
            let (mut k, mut vsum) = (0usize, Vec2::ZERO);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (ty, tx) = (cy + dy, cx + dx);
                    let mut c = cells.partition_point(|e| (e.0, e.1) < (ty, tx));
                    while c < cells.len() && (cells[c].0, cells[c].1) == (ty, tx) {
                        let j = cells[c].2 as usize;
                        if (self.pos[j] - f.pos).length_sq() < h * h {
                            k += 1;
                            vsum += self.vel[j];
                        }
                        c += 1;
                    }
                }
            }
            if k >= 5 {
                // in the water: carried along, and lighter than water so it floats up; deep down it
                // dissolves quickly, at the surface it lingers as foam
                let vavg = vsum / k as f64;
                f.vel += (vavg - f.vel) * 0.5;
                f.vel += gravity * (-0.4 * dt);
                f.age += if k >= 8 { dt * 4.0 } else { dt };
            } else {
                f.vel += gravity * dt;
                f.age += dt * 1.5;
            }
            f.pos += f.vel * dt;
            if let Some(w) = world {
                for hit in w.circle_hits(f.pos, 0.25 * s, 0.0) {
                    if hit.sep < 0.0 {
                        f.pos += hit.normal * hit.sep;
                    }
                    let vn = (f.vel - hit.vel).dot(hit.normal);
                    if vn > 0.0 {
                        f.vel -= hit.normal * vn;
                    }
                }
            }
        }
        self.foam.retain(|f| f.age < f.life);
    }

    /// Fast water at the surface throws spray and foam: a seeded draw per particle and substep.
    fn spawn_foam(&mut self, dt: f64, start: &[usize], list: &[u32]) {
        let (h2, s) = (self.h * self.h, self.params.spacing);
        for i in 0..self.pos.len() {
            if self.foam.len() >= self.params.foam_max {
                break;
            }
            let sp = self.vel[i].length();
            if sp <= 1.0 {
                continue;
            }
            let near = list[start[i]..start[i + 1]]
                .iter()
                .filter(|&&j| (self.pos[i] - self.pos[j as usize]).length_sq() < h2)
                .count();
            if near > 5 {
                continue; // only the surface throws foam; the bulk of the water does not
            }
            let weight = 1.0;
            let lambda = self.params.foam * 6.0 * (sp - 1.0) * dt * weight;
            let n = self.steps.wrapping_mul(1_000_003).wrapping_add(i as u64);
            if unit(self.seed, n, 0) < lambda {
                let jitter = v2(unit(self.seed, n, 1) - 0.5, unit(self.seed, n, 2) - 0.5);
                let pos = self.pos[i] + jitter * (0.8 * s);
                let vel = self.vel[i] * (0.7 + 0.5 * unit(self.seed, n, 3))
                    + v2(unit(self.seed, n, 4) - 0.5, unit(self.seed, n, 5) - 0.5) * 0.8;
                self.foam.push(Foam {
                    pos,
                    vel,
                    age: 0.0,
                    life: self.params.foam_life * (0.6 + 0.8 * unit(self.seed, n, 6)),
                });
            }
        }
    }
}
