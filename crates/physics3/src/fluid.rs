//! Position based fluids (Macklin and Mueller) in 3D. Every particle stands for `spacing^3` of
//! fluid; a density constraint pushes neighbours apart until the local density matches the rest
//! density, then XSPH smoothing gives viscosity. Particles collide with rigid bodies (pushing
//! dynamic ones back, which is what makes a crate float and a ball splash) and treat soft bodies as
//! obstacles. Neighbours come from a sorted cell list; the per-particle loops run on several
//! threads in contiguous chunks and are joined in order, so every sum runs in a fixed order and the
//! result does not depend on the thread count. Metres, kilograms, seconds.
use crate::math::*;
use crate::par::{par_map, par_ranges};
use crate::soft::SoftBody3;
use crate::world::{BodyKind, World};

#[derive(Clone, Debug)]
pub struct FluidParams3 {
    /// Distance between particles at rest, metres.
    pub spacing: f64,
    /// Kilograms per cubic metre (1 = water in this unit system, where a body of density 1 floats
    /// just submerged).
    pub density: f64,
    /// XSPH smoothing strength, 0 (water) to about 0.5 (syrup).
    pub viscosity: f64,
    /// Attraction between neighbours (droplets, 0 = none).
    pub cohesion: f64,
    pub max: usize,
    /// Amount of foam and spray thrown by fast, breaking water: 0 (none) to 1 (a lot).
    pub foam: f64,
    pub foam_life: f64,
    pub foam_max: usize,
    /// Worker threads; 0 picks the machine's parallelism. The result does not depend on it.
    pub threads: usize,
}

impl Default for FluidParams3 {
    fn default() -> FluidParams3 {
        FluidParams3 {
            spacing: 0.12,
            density: 1.0,
            viscosity: 0.05,
            cohesion: 0.0,
            max: 6000,
            foam: 0.0,
            foam_life: 1.5,
            foam_max: 3000,
            threads: 0,
        }
    }
}

/// A secondary particle: spray in the air, foam riding the surface. It never acts on the water.
#[derive(Clone, Copy, Debug)]
pub struct Foam3 {
    pub pos: V3,
    pub vel: V3,
    pub age: f64,
    pub life: f64,
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn unit(seed: u64, n: u64, attr: u64) -> f64 {
    let h = splitmix(seed ^ splitmix(n.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ attr));
    (h >> 11) as f64 / (1u64 << 53) as f64
}

#[derive(Clone, Debug)]
pub struct Fluid3 {
    params: FluidParams3,
    pub pos: Vec<V3>,
    pub vel: Vec<V3>,
    pub foam: Vec<Foam3>,
    seed: u64,
    steps: u64,
    h: f64,
    c_poly6: f64,
    c_spiky: f64,
    rho0: f64,
    eps: f64,
    wq: f64,
    bounds: Option<(V3, V3)>,
}

struct Contact {
    i: usize,
    /// Out of the body, towards the particle.
    n: V3,
    /// Signed distance from the particle's start to the body surface.
    sd: f64,
    xref: V3,
    body: usize,
    point: V3,
    dynamic: bool,
    acc: f64,
}

impl Fluid3 {
    pub fn new(params: FluidParams3) -> Fluid3 {
        let h = 2.0 * params.spacing;
        let c_poly6 = 315.0 / (64.0 * std::f64::consts::PI * h.powi(9));
        let c_spiky = 45.0 / (std::f64::consts::PI * h.powi(6));
        let mut f = Fluid3 {
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
        let (mut gsum, mut gsq) = (V3::ZERO, 0.0);
        let mut grads = vec![];
        for k in -3i32..=3 {
            for j in -3i32..=3 {
                for i in -3i32..=3 {
                    if i == 0 && j == 0 && k == 0 {
                        continue;
                    }
                    let d = v3(i as f64 * s, j as f64 * s, k as f64 * s);
                    let r2 = d.len2();
                    if r2 < h * h {
                        rho += f.w(r2);
                        grads.push(f.grad(d, r2.sqrt()));
                    }
                }
            }
        }
        f.rho0 = rho;
        for g in grads {
            let g = g / rho;
            gsum += g;
            gsq += g.len2();
        }
        f.eps = 0.2 * (gsq + gsum.len2());
        f.wq = f.w((0.2 * h) * (0.2 * h));
        f
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }
    pub fn params(&self) -> &FluidParams3 {
        &self.params
    }
    pub fn set_threads(&mut self, threads: usize) {
        self.params.threads = threads;
    }
    pub fn len(&self) -> usize {
        self.pos.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }
    /// Remove particles that leave the box (so lost water does not fill the pool).
    pub fn set_bounds(&mut self, lo: V3, hi: V3) {
        self.bounds = Some((lo, hi));
    }
    pub fn add(&mut self, pts: &[V3]) -> usize {
        let room = self.params.max.saturating_sub(self.pos.len());
        let k = room.min(pts.len());
        self.pos.extend_from_slice(&pts[..k]);
        self.vel.extend(std::iter::repeat_n(V3::ZERO, k));
        k
    }
    pub fn add_with_velocity(&mut self, pts: &[(V3, V3)]) -> usize {
        let room = self.params.max.saturating_sub(self.pos.len());
        let k = room.min(pts.len());
        for (p, v) in &pts[..k] {
            self.pos.push(*p);
            self.vel.push(*v);
        }
        k
    }

    /// poly6 kernel (3D) of the squared distance.
    fn w(&self, r2: f64) -> f64 {
        let h2 = self.h * self.h;
        if r2 >= h2 {
            0.0
        } else {
            self.c_poly6 * (h2 - r2).powi(3)
        }
    }
    /// spiky kernel gradient for the offset `d` from a neighbour to this particle.
    fn grad(&self, d: V3, r: f64) -> V3 {
        if r >= self.h || r < 1e-9 {
            V3::ZERO
        } else {
            d * (-self.c_spiky * (self.h - r) * (self.h - r) / r)
        }
    }

    /// Candidate neighbours within 1.3 h of each particle, in a fixed order: `(start, list)`.
    fn neighbours(&self, xs: &[V3]) -> (Vec<usize>, Vec<u32>) {
        let cell = 1.3 * self.h;
        let key = |p: V3| {
            (
                (p.z / cell).floor() as i64,
                (p.y / cell).floor() as i64,
                (p.x / cell).floor() as i64,
            )
        };
        let mut cells: Vec<(i64, i64, i64, u32)> = xs
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (cz, cy, cx) = key(*p);
                (cz, cy, cx, i as u32)
            })
            .collect();
        cells.sort_unstable();
        let reach2 = cell * cell;
        let chunks = par_ranges(xs.len(), self.params.threads, |r| {
            let mut counts = Vec::with_capacity(r.len());
            let mut list = vec![];
            for i in r {
                let p = xs[i];
                let before = list.len();
                let (cz, cy, cx) = key(p);
                for dz in -1..=1 {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let t = (cz + dz, cy + dy, cx + dx);
                            let mut k = cells.partition_point(|c| (c.0, c.1, c.2) < t);
                            while k < cells.len() && (cells[k].0, cells[k].1, cells[k].2) == t {
                                let j = cells[k].3 as usize;
                                if j != i && (xs[j] - p).len2() < reach2 {
                                    list.push(j as u32);
                                }
                                k += 1;
                            }
                        }
                    }
                }
                counts.push((list.len() - before) as u32);
            }
            (counts, list)
        });
        let mut start = Vec::with_capacity(xs.len() + 1);
        let mut list: Vec<u32> = vec![];
        let mut at = 0usize;
        for (counts, l) in chunks {
            for c in counts {
                start.push(at);
                at += c as usize;
            }
            list.extend(l);
        }
        start.push(at);
        (start, list)
    }

    /// The median over particles of density / rest density (the bulk, away from walls and surface).
    pub fn median_density_ratio(&self) -> f64 {
        let (start, list) = self.neighbours(&self.pos);
        let mut r: Vec<f64> = par_map(self.pos.len(), self.params.threads, |i| {
            let mut rho = self.w(0.0);
            for &j in &list[start[i]..start[i + 1]] {
                rho += self.w((self.pos[i] - self.pos[j as usize]).len2());
            }
            rho / self.rho0
        });
        r.sort_by(|a, b| a.total_cmp(b));
        r.get(r.len() / 2).copied().unwrap_or(1.0)
    }

    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for (p, v) in self.pos.iter().zip(&self.vel) {
            for x in [p.x, p.y, p.z, v.x, v.y, v.z] {
                for b in x.to_bits().to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }

    /// One substep of `dt`. Soft bodies are one-way obstacles.
    pub fn substep(
        &mut self,
        dt: f64,
        gravity: V3,
        iterations: usize,
        mut world: Option<&mut World>,
        soft: &[SoftBody3],
    ) {
        let n = self.pos.len();
        if n == 0 {
            self.update_foam(dt, gravity, world.as_deref());
            return;
        }
        let threads = self.params.threads;
        let s = self.params.spacing;
        let h2 = self.h * self.h;
        let mut xs: Vec<V3> = Vec::with_capacity(n);
        for i in 0..n {
            self.vel[i] += gravity * dt;
            xs.push(self.pos[i] + self.vel[i] * dt);
        }
        let (start, list) = self.neighbours(&xs);
        // contacts with rigid bodies, found once from where the particle was, with a margin for how
        // far it is about to move (so a fast particle cannot jump over a thin wall)
        let r_wall = 0.5 * s;
        let mut contacts: Vec<Contact> = vec![];
        if let Some(w) = world.as_deref() {
            let pos = &self.pos;
            let vel = &self.vel;
            let chunks = par_ranges(n, threads, |r| {
                let mut out = vec![];
                for i in r {
                    let x = pos[i];
                    let margin = 0.02 + vel[i].len() * dt + iterations as f64 * 0.4 * s;
                    let reach = r_wall + margin;
                    for hit in w.query_sphere(x, reach) {
                        let dynamic = w.body(hit.body).kind == BodyKind::Dynamic;
                        out.push(Contact {
                            i,
                            n: hit.normal,
                            sd: reach - hit.depth,
                            xref: x,
                            body: hit.body,
                            point: hit.point,
                            dynamic,
                            acc: 0.0,
                        });
                    }
                }
                out
            });
            contacts = chunks.into_iter().flatten().collect();
        }
        // soft bodies as fixed obstacle spheres for this substep
        let mut near_obst: Vec<(usize, V3, f64)> = vec![];
        if !soft.is_empty() {
            let (mut lo, mut hi) = (
                v3(f64::MAX, f64::MAX, f64::MAX),
                v3(f64::MIN, f64::MIN, f64::MIN),
            );
            for x in &xs {
                lo = lo.min(*x);
                hi = hi.max(*x);
            }
            let pad = 2.0 * s;
            let mut obst: Vec<(V3, f64)> = vec![];
            for b in soft {
                obst.extend(b.obstacle_points(
                    0.5 * s,
                    lo - v3(pad, pad, pad),
                    hi + v3(pad, pad, pad),
                ));
            }
            if !obst.is_empty() {
                let cell = 2.0 * s;
                let key = |p: V3| {
                    (
                        (p.z / cell).floor() as i64,
                        (p.y / cell).floor() as i64,
                        (p.x / cell).floor() as i64,
                    )
                };
                let mut cells: Vec<(i64, i64, i64, u32)> = obst
                    .iter()
                    .enumerate()
                    .map(|(k, o)| {
                        let (cz, cy, cx) = key(o.0);
                        (cz, cy, cx, k as u32)
                    })
                    .collect();
                cells.sort_unstable();
                for (i, x) in xs.iter().enumerate() {
                    let (cz, cy, cx) = key(*x);
                    for dz in -1..=1 {
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let t = (cz + dz, cy + dy, cx + dx);
                                let mut k = cells.partition_point(|c| (c.0, c.1, c.2) < t);
                                while k < cells.len() && (cells[k].0, cells[k].1, cells[k].2) == t {
                                    let (p, r) = obst[cells[k].3 as usize];
                                    if (*x - p).len() < r + r_wall + 0.5 * s {
                                        near_obst.push((i, p, r + r_wall));
                                    }
                                    k += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        let max_move = 0.4 * s;
        for _ in 0..iterations {
            let lambda: Vec<f64> = par_map(n, threads, |i| {
                let mut rho = self.w(0.0);
                let (mut gsum, mut gsq) = (V3::ZERO, 0.0);
                for &j in &list[start[i]..start[i + 1]] {
                    let d = xs[i] - xs[j as usize];
                    let r2 = d.len2();
                    if r2 < h2 {
                        rho += self.w(r2);
                        let g = self.grad(d, r2.sqrt()) / self.rho0;
                        gsum += g;
                        gsq += g.len2();
                    }
                }
                let c = (rho / self.rho0 - 1.0).max(0.0);
                -c / (gsq + gsum.len2() + self.eps)
            });
            let dp: Vec<V3> = par_map(n, threads, |i| {
                let mut d_i = V3::ZERO;
                for &j in &list[start[i]..start[i + 1]] {
                    let j = j as usize;
                    let d = xs[i] - xs[j];
                    let r2 = d.len2();
                    if r2 < h2 {
                        let tensile = -0.1 * h2 * (self.w(r2) / self.wq).powi(4);
                        d_i += self.grad(d, r2.sqrt())
                            * ((lambda[i] + lambda[j] + tensile) / self.rho0);
                    }
                }
                let m = d_i.len();
                if m > max_move {
                    d_i * (max_move / m)
                } else {
                    d_i
                }
            });
            for i in 0..n {
                xs[i] += dp[i];
            }
            for c in contacts.iter_mut() {
                let clearance = c.sd + (xs[c.i] - c.xref).dot(c.n) - r_wall;
                if clearance < 0.0 {
                    xs[c.i] += c.n * (-clearance);
                    c.acc += -clearance;
                }
            }
            for (i, p, rad) in &near_obst {
                let d = xs[*i] - *p;
                let dist = d.len();
                if dist < *rad {
                    let dir = if dist > 1e-9 {
                        d / dist
                    } else {
                        v3(0.0, -1.0, 0.0)
                    };
                    xs[*i] += dir * (rad - dist);
                }
            }
        }
        // velocities from the position change, then XSPH viscosity and cohesion
        let old: Vec<V3> = (0..n).map(|i| (xs[i] - self.pos[i]) / dt).collect();
        let max_speed = 20.0;
        let (visc, cohesion, h) = (self.params.viscosity, self.params.cohesion, self.h);
        let new_vel: Vec<V3> = par_map(n, threads, |i| {
            let mut rho = self.w(0.0);
            let (mut acc, mut pull) = (V3::ZERO, V3::ZERO);
            for &j in &list[start[i]..start[i + 1]] {
                let j = j as usize;
                let d = xs[j] - xs[i];
                let r2 = d.len2();
                if r2 < h2 {
                    let w = self.w(r2);
                    rho += w;
                    acc += (old[j] - old[i]) * w;
                    let r = r2.sqrt();
                    if r > 1e-9 {
                        pull += d * ((1.0 - r / h).powi(2) / r);
                    }
                }
            }
            let mut v = old[i] + acc * (visc / rho);
            v += pull * (cohesion * 10.0 * dt);
            let sp = v.len();
            if sp > max_speed {
                v * (max_speed / sp)
            } else {
                v
            }
        });
        self.pos = xs;
        self.vel = new_vel;
        // water pushes bodies back with the momentum of its push-out
        if let Some(w) = world.as_mut() {
            let m = self.params.density * s * s * s;
            for c in &contacts {
                if c.dynamic && c.acc > 0.0 {
                    w.apply_impulse_at(c.body, -c.n * (m * c.acc / dt), c.point);
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
                .map(|p| {
                    p.x >= lo.x
                        && p.x <= hi.x
                        && p.y >= lo.y
                        && p.y <= hi.y
                        && p.z >= lo.z
                        && p.z <= hi.z
                })
                .collect();
            if keep.iter().any(|k| !k) {
                let mut it = keep.iter();
                self.pos.retain(|_| *it.next().expect("same length"));
                let mut it = keep.iter();
                self.vel.retain(|_| *it.next().expect("same length"));
            }
        }
    }

    /// Move the existing foam: it rides the water when it is inside it (and floats up), flies like a
    /// projectile in the air, fades, and stops at rigid bodies.
    fn update_foam(&mut self, dt: f64, gravity: V3, world: Option<&World>) {
        if self.foam.is_empty() {
            return;
        }
        let h = self.h;
        let cell = h;
        let key = |p: V3| {
            (
                (p.z / cell).floor() as i64,
                (p.y / cell).floor() as i64,
                (p.x / cell).floor() as i64,
            )
        };
        let mut cells: Vec<(i64, i64, i64, u32)> = self
            .pos
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let (cz, cy, cx) = key(*p);
                (cz, cy, cx, i as u32)
            })
            .collect();
        cells.sort_unstable();
        let s = self.params.spacing;
        for f in self.foam.iter_mut() {
            let (cz, cy, cx) = key(f.pos);
            let (mut k, mut vsum) = (0usize, V3::ZERO);
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let t = (cz + dz, cy + dy, cx + dx);
                        let mut c = cells.partition_point(|e| (e.0, e.1, e.2) < t);
                        while c < cells.len() && (cells[c].0, cells[c].1, cells[c].2) == t {
                            let j = cells[c].3 as usize;
                            if (self.pos[j] - f.pos).len2() < h * h {
                                k += 1;
                                vsum += self.vel[j];
                            }
                            c += 1;
                        }
                    }
                }
            }
            if k >= 8 {
                let vavg = vsum / k as f64;
                f.vel += (vavg - f.vel) * 0.5;
                f.vel += gravity * (-0.4 * dt);
                f.age += if k >= 14 { dt * 4.0 } else { dt };
            } else {
                f.vel += gravity * dt;
                f.age += dt * 1.5;
            }
            f.pos += f.vel * dt;
            if let Some(w) = world {
                for hit in w.query_sphere(f.pos, 0.25 * s) {
                    f.pos += hit.normal * hit.depth;
                    let vn = f.vel.dot(hit.normal);
                    if vn < 0.0 {
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
            let sp = self.vel[i].len();
            if sp <= 1.0 {
                continue;
            }
            let near = list[start[i]..start[i + 1]]
                .iter()
                .filter(|&&j| (self.pos[i] - self.pos[j as usize]).len2() < h2)
                .count();
            if near > 12 {
                continue; // only the surface throws foam
            }
            let lambda = self.params.foam * 6.0 * (sp - 1.0) * dt;
            let n = self.steps.wrapping_mul(1_000_003).wrapping_add(i as u64);
            if unit(self.seed, n, 0) < lambda {
                let jitter = v3(
                    unit(self.seed, n, 1) - 0.5,
                    unit(self.seed, n, 2) - 0.5,
                    unit(self.seed, n, 7) - 0.5,
                );
                let pos = self.pos[i] + jitter * (0.8 * s);
                let vel = self.vel[i] * (0.7 + 0.5 * unit(self.seed, n, 3))
                    + v3(
                        unit(self.seed, n, 4) - 0.5,
                        unit(self.seed, n, 5) - 0.5,
                        unit(self.seed, n, 8) - 0.5,
                    ) * 0.8;
                self.foam.push(Foam3 {
                    pos,
                    vel,
                    age: 0.0,
                    life: self.params.foam_life * (0.6 + 0.8 * unit(self.seed, n, 6)),
                });
            }
        }
    }
}
