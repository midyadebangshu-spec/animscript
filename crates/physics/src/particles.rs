//! Seeded particle emitters. Particles are points with a radius in *pixels*; they collide with a
//! `World` (metres, `ppm` pixels per metre) but never push it. Every random attribute is a pure
//! function of (seed, spawn index, attribute), so emission is reproducible and order independent.
use crate::math::*;
use crate::world::World;

#[derive(Clone, Debug)]
pub struct EmitterDef {
    /// Particles per second while the emitter is started.
    pub rate: f64,
    pub lifetime: (f64, f64),
    pub speed: (f64, f64),
    /// Direction range in radians; 0 is +x and angles turn towards +y (clockwise on screen).
    pub angle: (f64, f64),
    /// Starting radius range in pixels.
    pub size: (f64, f64),
    pub gravity: Vec2,
    pub drag: f64,
    pub restitution: f64,
    pub collide: bool,
    pub max: usize,
}

impl Default for EmitterDef {
    fn default() -> EmitterDef {
        EmitterDef {
            rate: 0.0,
            lifetime: (1.0, 1.0),
            speed: (0.0, 0.0),
            angle: (0.0, std::f64::consts::TAU),
            size: (3.0, 3.0),
            gravity: Vec2::ZERO,
            drag: 0.0,
            restitution: 0.4,
            collide: false,
            max: 1000,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub p: Vec2,
    pub v: Vec2,
    pub age: f64,
    pub life: f64,
    pub size: f64,
    pub index: u64,
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A value in [0, 1) for one attribute of one particle.
fn unit(seed: u64, index: u64, attr: u64) -> f64 {
    let h = splitmix(
        splitmix(seed ^ index.wrapping_mul(0xd6e8_feb8_6659_fd93))
            ^ attr.wrapping_mul(0xa076_1d64_78bd_642f),
    );
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn lerp(r: (f64, f64), t: f64) -> f64 {
    r.0 + (r.1 - r.0) * t
}

pub struct Emitter {
    def: EmitterDef,
    seed: u64,
    parts: Vec<Particle>,
    active: bool,
    active_time: f64,
    by_rate: u64,
    burst: usize,
    next_index: u64,
}

impl Emitter {
    pub fn new(def: EmitterDef, seed: u64) -> Emitter {
        Emitter {
            def,
            seed,
            parts: vec![],
            active: false,
            active_time: 0.0,
            by_rate: 0,
            burst: 0,
            next_index: 0,
        }
    }
    pub fn start(&mut self) {
        self.active = true;
    }
    pub fn stop(&mut self) {
        self.active = false;
    }
    pub fn burst(&mut self, n: usize) {
        self.burst += n;
    }
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn particles(&self) -> &[Particle] {
        &self.parts
    }
    /// For forces applied from outside (the language's `force_x` / `force_y` equations).
    pub fn particles_mut(&mut self) -> &mut [Particle] {
        &mut self.parts
    }
    /// How many particles have been spawned in total (including ones that died or were over the cap).
    pub fn emitted(&self) -> u64 {
        self.next_index
    }
    pub fn def(&self) -> &EmitterDef {
        &self.def
    }

    /// Advance by `dt` seconds (`dt = 0` only spawns). `spawn_at(i)` gives the start position of the
    /// particle with spawn index `i`; `world` (with pixels per metre) enables collision.
    pub fn step(
        &mut self,
        dt: f64,
        world: Option<(&World, f64)>,
        spawn_at: &mut dyn FnMut(u64) -> Vec2,
    ) {
        let mut count = std::mem::take(&mut self.burst);
        if self.active {
            self.active_time += dt;
            let target = (self.def.rate * self.active_time + 1e-9).floor() as u64;
            count += (target - self.by_rate) as usize;
            self.by_rate = target;
        }
        for _ in 0..count {
            let i = self.next_index;
            self.next_index += 1;
            if self.parts.len() >= self.def.max {
                continue;
            }
            let angle = lerp(self.def.angle, unit(self.seed, i, 2));
            let speed = lerp(self.def.speed, unit(self.seed, i, 1));
            self.parts.push(Particle {
                p: spawn_at(i),
                v: v2(angle.cos() * speed, angle.sin() * speed),
                age: 0.0,
                life: lerp(self.def.lifetime, unit(self.seed, i, 0)),
                size: lerp(self.def.size, unit(self.seed, i, 3)),
                index: i,
            });
        }
        if dt <= 0.0 {
            return;
        }
        let d = &self.def;
        for p in &mut self.parts {
            p.v += d.gravity * dt;
            p.v = p.v * (1.0 / (1.0 + d.drag * dt));
            p.p += p.v * dt;
            p.age += dt;
            if let (true, Some((w, ppm))) = (d.collide, world) {
                let margin = p.v.length() * dt / ppm;
                if let Some((n, sep, vb)) = w.collide_circle(p.p / ppm, p.size / ppm, margin) {
                    let mut rel = p.v / ppm - vb;
                    let vn = rel.dot(n);
                    if sep <= 0.0 {
                        p.p += n * (sep * ppm);
                    }
                    if vn > 0.0 && (sep <= 0.0 || vn * dt >= sep) {
                        if sep > 0.0 {
                            p.p += n * (sep * ppm); // arriving this step: put it on the surface
                        }
                        rel -= n * ((1.0 + d.restitution) * vn);
                        p.v = (rel + vb) * ppm;
                    }
                }
            }
        }
        self.parts.retain(|p| p.age < p.life);
    }
}
