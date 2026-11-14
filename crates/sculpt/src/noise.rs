//! Seeded 3D noise: gradient (Perlin), value, cellular (Worley) and the fractal combinations. A seed fixes a
//! permutation table, so a noise is a pure function of its seed and position.
use crate::V3;

#[derive(Clone)]
pub struct Noise {
    perm: [u8; 512],
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

const GRADS: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

impl Noise {
    pub fn new(seed: u32) -> Noise {
        let mut p: Vec<u8> = (0..=255u8).collect();
        let mut s = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        for i in (1..256).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let j = (s % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Noise { perm }
    }

    fn h(&self, x: i32, y: i32, z: i32) -> usize {
        let p = &self.perm;
        p[p[p[(x & 255) as usize] as usize + (y & 255) as usize] as usize + (z & 255) as usize]
            as usize
    }

    /// Improved Perlin noise, in about `-1..1`.
    pub fn perlin(&self, p: V3) -> f64 {
        let (xi, yi, zi) = (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        );
        let (x, y, z) = (p[0] - xi as f64, p[1] - yi as f64, p[2] - zi as f64);
        let g = |dx: i32, dy: i32, dz: i32, fx: f64, fy: f64, fz: f64| {
            let gr = GRADS[self.h(xi + dx, yi + dy, zi + dz) % 12];
            gr[0] * fx + gr[1] * fy + gr[2] * fz
        };
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let x1 = lerp(g(0, 0, 0, x, y, z), g(1, 0, 0, x - 1.0, y, z), u);
        let x2 = lerp(
            g(0, 1, 0, x, y - 1.0, z),
            g(1, 1, 0, x - 1.0, y - 1.0, z),
            u,
        );
        let y1 = lerp(x1, x2, v);
        let x3 = lerp(
            g(0, 0, 1, x, y, z - 1.0),
            g(1, 0, 1, x - 1.0, y, z - 1.0),
            u,
        );
        let x4 = lerp(
            g(0, 1, 1, x, y - 1.0, z - 1.0),
            g(1, 1, 1, x - 1.0, y - 1.0, z - 1.0),
            u,
        );
        let y2 = lerp(x3, x4, v);
        lerp(y1, y2, w)
    }

    /// Value noise (random values at lattice points, smoothly interpolated), `-1..1`.
    pub fn value(&self, p: V3) -> f64 {
        let (xi, yi, zi) = (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        );
        let (u, v, w) = (
            fade(p[0] - xi as f64),
            fade(p[1] - yi as f64),
            fade(p[2] - zi as f64),
        );
        let val =
            |dx: i32, dy: i32, dz: i32| self.h(xi + dx, yi + dy, zi + dz) as f64 / 127.5 - 1.0;
        let x1 = lerp(
            lerp(val(0, 0, 0), val(1, 0, 0), u),
            lerp(val(0, 1, 0), val(1, 1, 0), u),
            v,
        );
        let x2 = lerp(
            lerp(val(0, 0, 1), val(1, 0, 1), u),
            lerp(val(0, 1, 1), val(1, 1, 1), u),
            v,
        );
        lerp(x1, x2, w)
    }

    /// Cellular noise: the distance to the nearest of the feature points (one per unit cell), `0..~1`.
    pub fn worley(&self, p: V3) -> f64 {
        let (xi, yi, zi) = (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        );
        let mut best = f64::MAX;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (cx, cy, cz) = (xi + dx, yi + dy, zi + dz);
                    let f = |k: i32| self.h(cx + k * 17, cy + k * 31, cz + k * 47) as f64 / 255.0;
                    let q = [cx as f64 + f(1), cy as f64 + f(2), cz as f64 + f(3)];
                    let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
                    best = best.min(d);
                }
            }
        }
        best.sqrt()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Perlin,
    Value,
    /// Bumps at the cells of a Worley pattern (high at the feature points).
    Worley,
    /// Fractal Perlin: octaves added up.
    Fbm,
    /// Sharp ridges: `1 - |noise|` octaves.
    Ridged,
    /// Rounded creases: `|noise|` octaves.
    Billow,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "perlin" => Kind::Perlin,
            "value" => Kind::Value,
            "worley" | "cellular" => Kind::Worley,
            "fbm" | "fractal" => Kind::Fbm,
            "ridged" => Kind::Ridged,
            "billow" => Kind::Billow,
            _ => return None,
        })
    }
}

/// A noise as a user describes it: the kind, the size of its features, octaves and the seed.
#[derive(Clone)]
pub struct Spec {
    pub kind: Kind,
    /// The size of the largest features (world units).
    pub scale: f64,
    pub octaves: u32,
    pub lacunarity: f64,
    pub gain: f64,
    noise: Noise,
}

impl Spec {
    pub fn new(kind: Kind, scale: f64, octaves: u32, seed: u32) -> Spec {
        Spec {
            kind,
            scale: scale.max(1e-9),
            octaves: octaves.clamp(1, 12),
            lacunarity: 2.0,
            gain: 0.5,
            noise: Noise::new(seed),
        }
    }

    /// The noise at a world position, in about `-1..1`.
    pub fn eval(&self, p: V3) -> f64 {
        let q = [p[0] / self.scale, p[1] / self.scale, p[2] / self.scale];
        match self.kind {
            Kind::Perlin => self.noise.perlin(q),
            Kind::Value => self.noise.value(q),
            Kind::Worley => 1.0 - 2.0 * self.noise.worley(q).min(1.0),
            Kind::Fbm | Kind::Ridged | Kind::Billow => {
                let (mut amp, mut freq, mut sum, mut norm) = (1.0, 1.0, 0.0, 0.0);
                for o in 0..self.octaves {
                    // each octave looks at a shifted copy of the field so they do not line up
                    let shift = 17.3 * o as f64;
                    let n = self.noise.perlin([
                        q[0] * freq + shift,
                        q[1] * freq - shift,
                        q[2] * freq + shift * 0.5,
                    ]);
                    let v = match self.kind {
                        Kind::Ridged => 1.0 - 2.0 * n.abs(),
                        Kind::Billow => 2.0 * n.abs() - 1.0,
                        _ => n,
                    };
                    sum += v * amp;
                    norm += amp;
                    amp *= self.gain;
                    freq *= self.lacunarity;
                }
                sum / norm
            }
        }
    }

    /// A divergence-free vector field made from the noise (the curl of three potentials): swirly, never compressing.
    pub fn curl(&self, p: V3) -> V3 {
        let e = self.scale * 0.01;
        let pot = |q: V3, k: usize| {
            let s = [
                q[0] + 31.7 * k as f64,
                q[1] - 12.3 * k as f64,
                q[2] + 7.9 * k as f64,
            ];
            self.eval(s)
        };
        let d = |k: usize, axis: usize| {
            let (mut a, mut b) = (p, p);
            a[axis] += e;
            b[axis] -= e;
            (pot(a, k) - pot(b, k)) / (2.0 * e)
        };
        // curl(psi) = (dz psi_y - dy psi_z, dx psi_z - dz psi_x, dy psi_x - dx psi_y)
        [d(1, 2) - d(2, 1), d(2, 0) - d(0, 2), d(0, 1) - d(1, 0)]
    }
}
