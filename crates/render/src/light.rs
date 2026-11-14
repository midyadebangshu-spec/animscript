//! Photographic lighting: the lights of the snapshot (`lights`), a physically based shading model
//! (Lambert diffuse, GGX specular, Schlick Fresnel, hemisphere ambient), and the filmic tone-map.
//! Everything is a pure function of its inputs, evaluated in a fixed order.
use serde_json::Value;

pub type V3 = [f64; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Sun,
    Point,
    Spot,
    Ambient,
}

#[derive(Clone, Debug)]
pub struct Lt {
    pub kind: Kind,
    pub pos: V3,
    /// The direction the light travels.
    pub dir: V3,
    pub color: [f64; 3],
    pub intensity: f64,
    pub reach: f64,
    pub size: f64,
    pub angle: f64,
    pub softness: f64,
    pub ground: Option<[f64; 3]>,
    pub shadows: bool,
}

/// A surface point to shade. `albedo` is in linear light.
#[derive(Clone, Copy, Debug)]
pub struct Surf {
    pub pos: V3,
    pub n: V3,
    /// Unit vector from the point to the eye.
    pub view: V3,
    pub albedo: [f64; 3],
    pub rough: f64,
    pub metal: f64,
    pub emissive: f64,
    /// How much of the ambient light reaches the point (1 = all).
    pub ao: f64,
}

fn v3(v: &Value) -> V3 {
    [
        v[0].as_f64().unwrap_or(0.0),
        v[1].as_f64().unwrap_or(0.0),
        v[2].as_f64().unwrap_or(0.0),
    ]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l < 1e-300 {
        [0.0, 0.0, 0.0]
    } else {
        [a[0] / l, a[1] / l, a[2] / l]
    }
}

/// The lights of a snapshot (empty when it has none: the renderer then keeps its toon shading).
pub fn parse_lights(snapshot: &Value) -> Vec<Lt> {
    snapshot["lights"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| Lt {
            kind: match l["kind"].as_str() {
                Some("sun") => Kind::Sun,
                Some("spot") => Kind::Spot,
                Some("ambient") => Kind::Ambient,
                _ => Kind::Point,
            },
            pos: v3(&l["pos"]),
            dir: norm(v3(&l["dir"])),
            color: v3(&l["color"]),
            intensity: l["intensity"].as_f64().unwrap_or(1.0),
            reach: l["reach"].as_f64().unwrap_or(200.0).max(1e-6),
            size: l["size"].as_f64().unwrap_or(0.0),
            angle: l["angle"].as_f64().unwrap_or(0.7),
            softness: l["softness"].as_f64().unwrap_or(0.25),
            ground: l["ground"].is_array().then(|| v3(&l["ground"])),
            shadows: l["shadows"].as_bool().unwrap_or(true),
        })
        .collect()
}

pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The ACES filmic curve (Narkowicz's fit) then the sRGB encoding: linear light in, 0..1 out.
pub fn tonemap(c: [f64; 3], exposure: f64) -> [f64; 3] {
    let f = |x: f64| {
        let x = (x * exposure * 0.6).max(0.0);
        ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
    };
    [
        linear_to_srgb(f(c[0])),
        linear_to_srgb(f(c[1])),
        linear_to_srgb(f(c[2])),
    ]
}

/// The direction to a light from `p`, its distance (infinite for a sun) and the irradiance factor
/// (falloff and spot cone) there. `None` for an ambient light.
pub fn arrival(l: &Lt, p: V3) -> Option<(V3, f64, f64)> {
    match l.kind {
        Kind::Ambient => None,
        Kind::Sun => Some((
            [-l.dir[0], -l.dir[1], -l.dir[2]],
            f64::INFINITY,
            l.intensity,
        )),
        Kind::Point | Kind::Spot => {
            let d = sub(l.pos, p);
            let dist = dot(d, d).sqrt().max(1e-9);
            let to = [d[0] / dist, d[1] / dist, d[2] / dist];
            // intensity is the irradiance at `reach` px; it falls with the square of the distance
            let r = l.reach / dist.max(l.reach * 0.1);
            let mut e = l.intensity * r * r;
            if l.kind == Kind::Spot {
                let c = -dot(to, l.dir);
                let outer = l.angle.cos();
                let inner = (l.angle * (1.0 - l.softness)).cos();
                let t = if inner - outer < 1e-9 {
                    (c >= outer) as u8 as f64
                } else {
                    ((c - outer) / (inner - outer)).clamp(0.0, 1.0)
                };
                e *= t * t * (3.0 - 2.0 * t);
            }
            Some((to, dist, e))
        }
    }
}

/// Linear radiance leaving the surface towards the eye. `visible(i)` is how much of light `i` (its
/// index in `lights`) reaches the point (1 = all, 0 = shadowed).
pub fn radiance(lights: &[Lt], s: &Surf, visible: &dyn Fn(usize) -> f64) -> [f64; 3] {
    let rough = s.rough.clamp(0.04, 1.0);
    let a = rough * rough;
    let a2 = a * a;
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let ndv = dot(s.n, s.view).max(1e-4);
    let f0: [f64; 3] = std::array::from_fn(|i| 0.04 * (1.0 - s.metal) + s.albedo[i] * s.metal);
    let diffuse_k = 1.0 - s.metal;
    let mut out = [0.0; 3];
    for (i, l) in lights.iter().enumerate() {
        match arrival(l, s.pos) {
            None => {
                // hemisphere ambient: from above (world -y) to below
                let up = [0.0, -1.0, 0.0];
                let t = 0.5 + 0.5 * dot(s.n, up);
                let from_above = l.color;
                let from_below = l.ground.unwrap_or(l.color);
                for c in 0..3 {
                    let irr =
                        (from_below[c] + (from_above[c] - from_below[c]) * t) * l.intensity * s.ao;
                    out[c] += s.albedo[c] * diffuse_k * irr + f0[c] * irr * 0.6;
                }
            }
            Some((to, _, e)) => {
                let ndl = dot(s.n, to);
                if ndl <= 0.0 || e <= 0.0 {
                    continue;
                }
                let vis = visible(i);
                if vis <= 0.0 {
                    continue;
                }
                let h = norm([to[0] + s.view[0], to[1] + s.view[1], to[2] + s.view[2]]);
                let ndh = dot(s.n, h).max(0.0);
                let vdh = dot(s.view, h).max(0.0);
                let d = a2 / (std::f64::consts::PI * (ndh * ndh * (a2 - 1.0) + 1.0).powi(2));
                let g = (ndv / (ndv * (1.0 - k) + k)) * (ndl / (ndl * (1.0 - k) + k));
                let fw = (1.0 - vdh).powi(5);
                for c in 0..3 {
                    let f = f0[c] + (1.0 - f0[c]) * fw;
                    let spec = d * g * f / (4.0 * ndv * ndl + 1e-4) * std::f64::consts::PI;
                    out[c] += (s.albedo[c] * diffuse_k + spec) * ndl * e * l.color[c] * vis;
                }
            }
        }
    }
    for c in 0..3 {
        out[c] += s.albedo[c] * s.emissive;
    }
    out
}

fn hash01(x: usize, y: usize) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// How much of light `l` reaches `p` (surface normal `n`): 1 = all, 0 = none. A light with `size` zero
/// casts a hard shadow (one ray); otherwise `samples` rays go to points of its disc, laid out on a
/// spiral that is turned by a hash of the pixel `(x, y)`, so the same picture always comes out.
pub fn visibility(
    l: &Lt,
    bvh: &crate::bvh::Bvh,
    samples: usize,
    p: V3,
    n: V3,
    x: usize,
    y: usize,
) -> f64 {
    if !l.shadows || bvh.is_empty() {
        return 1.0;
    }
    let Some((to, dist, _)) = arrival(l, p) else {
        return 1.0;
    };
    let ndl = dot(n, to).clamp(0.0, 1.0);
    let bias = 0.8 + 1.5 * (1.0 - ndl);
    let o = [p[0] + n[0] * bias, p[1] + n[1] * bias, p[2] + n[2] * bias];
    let soft = l.size > 0.0 && samples > 1;
    if !soft {
        let tmax = if dist.is_finite() {
            dist - 1.0
        } else {
            f64::INFINITY
        };
        return if bvh.any_hit(o, to, tmax) { 0.0 } else { 1.0 };
    }
    let up = if to[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t1 = norm(cross(to, up));
    let t2 = cross(to, t1);
    let rot = hash01(x, y) * std::f64::consts::TAU;
    let mut open = 0usize;
    for k in 0..samples {
        let r = ((k as f64 + 0.5) / samples as f64).sqrt();
        let th = k as f64 * 2.399_963_229_728_653 + rot;
        let (dx, dy) = (r * th.cos(), r * th.sin());
        let (dir, tmax) = if dist.is_finite() {
            let target = [
                l.pos[0] + (t1[0] * dx + t2[0] * dy) * l.size,
                l.pos[1] + (t1[1] * dx + t2[1] * dy) * l.size,
                l.pos[2] + (t1[2] * dx + t2[2] * dy) * l.size,
            ];
            let d = sub(target, o);
            let len = dot(d, d).sqrt().max(1e-9);
            ([d[0] / len, d[1] / len, d[2] / len], len - 1.0)
        } else {
            let tan = (l.size.to_radians() * 0.5).tan();
            (
                norm([
                    to[0] + (t1[0] * dx + t2[0] * dy) * tan,
                    to[1] + (t1[1] * dx + t2[1] * dy) * tan,
                    to[2] + (t1[2] * dx + t2[2] * dy) * tan,
                ]),
                f64::INFINITY,
            )
        };
        if !bvh.any_hit(o, dir, tmax) {
            open += 1;
        }
    }
    open as f64 / samples as f64
}

/// The share of the hemisphere above `p` that is open within `radius` px: `samples` cosine-weighted rays
/// on a spiral turned by a hash of the pixel.
pub fn occlusion(
    bvh: &crate::bvh::Bvh,
    samples: usize,
    radius: f64,
    p: V3,
    n: V3,
    x: usize,
    y: usize,
) -> f64 {
    if bvh.is_empty() || radius <= 0.0 || samples == 0 {
        return 1.0;
    }
    let up = if n[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t1 = norm(cross(n, up));
    let t2 = cross(n, t1);
    let o = [p[0] + n[0] * 1.5, p[1] + n[1] * 1.5, p[2] + n[2] * 1.5];
    let rot = hash01(x ^ 0x55, y ^ 0xAA) * std::f64::consts::TAU;
    let mut open = 0usize;
    for k in 0..samples {
        let r = ((k as f64 + 0.5) / samples as f64).sqrt();
        let th = k as f64 * 2.399_963_229_728_653 + rot;
        let (dx, dy, dz) = (r * th.cos(), r * th.sin(), (1.0 - r * r).max(0.0).sqrt());
        let d = [
            t1[0] * dx + t2[0] * dy + n[0] * dz,
            t1[1] * dx + t2[1] * dy + n[1] * dz,
            t1[2] * dx + t2[2] * dy + n[2] * dz,
        ];
        if !bvh.any_hit(o, d, radius) {
            open += 1;
        }
    }
    open as f64 / samples as f64
}

/// Bloom: the part of `rgba` (premultiplied, supersampled, `w` x `h`) brighter than a threshold is
/// blurred (three box passes) and added back with `strength`. `radius` is in buffer pixels.
pub fn bloom(rgba: &mut [[f32; 4]], w: usize, h: usize, strength: f64, radius: usize) {
    if strength <= 0.0 || radius == 0 {
        return;
    }
    let mut glow: Vec<[f32; 3]> = rgba
        .iter()
        .map(|p| {
            let l = (p[0] + p[1] + p[2]) / 3.0;
            let k = ((l - 0.7) / 0.3).clamp(0.0, 1.0);
            [p[0] * k, p[1] * k, p[2] * k]
        })
        .collect();
    let mut tmp = glow.clone();
    for _ in 0..3 {
        // horizontal then vertical box blur with a running sum
        for y in 0..h {
            blur_line(
                &glow[y * w..(y + 1) * w],
                &mut tmp[y * w..(y + 1) * w],
                radius,
            );
        }
        let mut col = vec![[0.0f32; 3]; h];
        let mut out = vec![[0.0f32; 3]; h];
        for x in 0..w {
            for y in 0..h {
                col[y] = tmp[y * w + x];
            }
            blur_line(&col, &mut out, radius);
            for y in 0..h {
                glow[y * w + x] = out[y];
            }
        }
    }
    let s = strength as f32;
    for (p, g) in rgba.iter_mut().zip(&glow) {
        for c in 0..3 {
            p[c] = (p[c] + g[c] * s).min(1.0);
        }
        p[3] = p[3].max(((g[0] + g[1] + g[2]) / 3.0 * s).min(1.0));
    }
}

fn blur_line(src: &[[f32; 3]], dst: &mut [[f32; 3]], r: usize) {
    let n = src.len();
    let mut acc = [0.0f32; 3];
    let norm = 1.0 / (2 * r + 1) as f32;
    for (i, p) in src.iter().enumerate().take(r.min(n)) {
        let _ = i;
        for c in 0..3 {
            acc[c] += p[c];
        }
    }
    for i in 0..n {
        if i + r < n {
            for c in 0..3 {
                acc[c] += src[i + r][c];
            }
        }
        if i > r {
            for c in 0..3 {
                acc[c] -= src[i - r - 1][c];
            }
        }
        for c in 0..3 {
            dst[i][c] = acc[c] * norm;
        }
    }
}
