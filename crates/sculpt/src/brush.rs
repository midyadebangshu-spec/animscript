//! Sculpting brushes. A brush works on a volume in dabs: each dab finds the surface near its centre, estimates the
//! surface's normal and centre there, and edits the distance field in a sphere of the brush's radius with a falloff.
//! A stroke is a path; dabs are placed along it at a spacing that is a fraction of the radius.
use crate::sdf::{smax, smin};
use crate::volume::Volume;
use crate::{add, dot, len, mul, norm, sub, V3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Add a rounded blob of material at the surface.
    Draw,
    /// Build up a flat layer (a plane just above the surface).
    Clay,
    /// Relax the surface (average each distance with its neighbours).
    Smooth,
    /// Push the surface out along its normal.
    Inflate,
    /// Pull the surface onto the plane through the dab.
    Flatten,
    /// Draw the surface sideways in towards the stroke.
    Pinch,
    /// Cut away what stands above the plane through the dab.
    Scrape,
    /// Smear: drag the material along the stroke.
    Grease,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "draw" => Kind::Draw,
            "clay" => Kind::Clay,
            "smooth" => Kind::Smooth,
            "inflate" => Kind::Inflate,
            "flatten" => Kind::Flatten,
            "pinch" => Kind::Pinch,
            "scrape" => Kind::Scrape,
            "grease" | "smear" => Kind::Grease,
            _ => return None,
        })
    }
    pub const NAMES: &'static [&'static str] = &[
        "draw", "clay", "smooth", "inflate", "flatten", "pinch", "scrape", "grease",
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    Smooth,
    Linear,
    Sharp,
    Constant,
}

impl Falloff {
    pub fn parse(s: &str) -> Option<Falloff> {
        Some(match s {
            "smooth" => Falloff::Smooth,
            "linear" => Falloff::Linear,
            "sharp" => Falloff::Sharp,
            "constant" => Falloff::Constant,
            _ => return None,
        })
    }
    /// The weight at `t = distance / radius`: 1 at the centre, 0 at the edge.
    pub fn w(self, t: f64) -> f64 {
        if t >= 1.0 {
            return 0.0;
        }
        let u = 1.0 - t.max(0.0);
        match self {
            Falloff::Smooth => u * u * (3.0 - 2.0 * u),
            Falloff::Linear => u,
            Falloff::Sharp => u * u,
            Falloff::Constant => 1.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Brush {
    pub kind: Kind,
    pub radius: f64,
    /// About 0 to 1 (more is stronger).
    pub strength: f64,
    pub falloff: Falloff,
    /// Do the opposite (remove instead of add, and so on).
    pub invert: bool,
    /// The distance between dabs as a share of the radius.
    pub spacing: f64,
    /// Mirror the stroke across the planes through `mirror_at` (x, y, z).
    pub mirror: [bool; 3],
    pub mirror_at: V3,
    /// Passes per dab (for smoothing).
    pub passes: u32,
}

impl Brush {
    pub fn new(kind: Kind, radius: f64, strength: f64) -> Brush {
        Brush {
            kind,
            radius,
            strength,
            falloff: Falloff::Smooth,
            invert: false,
            spacing: 0.25,
            mirror: [false; 3],
            mirror_at: [0.0; 3],
            passes: 1,
        }
    }

    /// The dab centres and the stroke direction at each, for a path (each segment of the path at the spacing).
    fn dabs(&self, vol: &Volume, path: &[V3]) -> Vec<(V3, V3)> {
        let step = (self.spacing * self.radius).max(vol.h * 0.5);
        let mut out = vec![];
        if path.is_empty() {
            return out;
        }
        if path.len() == 1 {
            return vec![(path[0], [0.0; 3])];
        }
        let mut carry = 0.0;
        out.push((path[0], norm(sub(path[1], path[0]))));
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let seg = len(sub(b, a));
            if seg < 1e-12 {
                continue;
            }
            let dir = norm(sub(b, a));
            let mut d = step - carry;
            while d <= seg {
                out.push((add(a, mul(dir, d)), dir));
                d += step;
            }
            carry = seg - (d - step);
        }
        let last = path[path.len() - 1];
        if len(sub(out[out.len() - 1].0, last)) > step * 0.25 {
            let dir = norm(sub(last, path[path.len() - 2]));
            out.push((last, dir));
        }
        out
    }

    /// Apply the brush along a path (world positions); returns the number of dabs.
    pub fn stroke(&self, vol: &mut Volume, path: &[V3]) -> usize {
        let dabs = self.dabs(vol, path);
        let step = (self.spacing * self.radius).max(vol.h * 0.5);
        let mut count = 0;
        for (c, dir) in &dabs {
            for mask in 0..8u32 {
                if (0..3).any(|a| mask & (1 << a) != 0 && !self.mirror[a]) {
                    continue;
                }
                let (mut cc, mut dd) = (*c, *dir);
                for a in 0..3 {
                    if mask & (1 << a) != 0 {
                        cc[a] = 2.0 * self.mirror_at[a] - cc[a];
                        dd[a] = -dd[a];
                    }
                }
                for _ in 0..self.passes.max(1) {
                    self.dab(vol, cc, dd, step);
                }
                count += 1;
            }
        }
        count
    }

    /// The point on the surface near `p`, found by walking down the gradient.
    fn project(&self, vol: &Volume, mut p: V3) -> V3 {
        for _ in 0..10 {
            let d = vol.sample(p);
            if d.abs() < 0.05 * vol.h {
                break;
            }
            let g = norm(vol.gradient(p));
            if len(g) == 0.0 {
                break;
            }
            let s = d.clamp(-self.radius, self.radius);
            p = sub(p, mul(g, s));
        }
        p
    }

    /// The surface's centre and normal under the dab: the weighted mean of the surface voxels in the brush and of
    /// their gradients.
    fn frame(&self, vol: &Volume, p: V3) -> Option<(V3, V3)> {
        let (h, r) = (vol.h, self.radius);
        let (lo, hi) = vol.index_box(
            [p[0] - r, p[1] - r, p[2] - r],
            [p[0] + r, p[1] + r, p[2] + r],
        );
        let (mut c, mut n, mut wsum) = ([0.0; 3], [0.0; 3], 0.0);
        for k in lo[2]..=hi[2] {
            for j in lo[1]..=hi[1] {
                for i in lo[0]..=hi[0] {
                    let d = vol.get([i, j, k]) as f64;
                    if d.abs() > 0.75 * h {
                        continue;
                    }
                    let q = [i as f64 * h, j as f64 * h, k as f64 * h];
                    let w = self.falloff_w(len(sub(q, p)) / r).max(
                        if self.falloff == Falloff::Constant {
                            1.0
                        } else {
                            0.0
                        },
                    );
                    if w <= 0.0 {
                        continue;
                    }
                    let g = norm(vol.gradient(q));
                    // move the voxel onto the surface along its gradient
                    let qs = sub(q, mul(g, d));
                    c = add(c, mul(qs, w));
                    n = add(n, mul(g, w));
                    wsum += w;
                }
            }
        }
        if wsum <= 0.0 {
            return None;
        }
        Some((mul(c, 1.0 / wsum), norm(n)))
    }

    fn falloff_w(&self, t: f64) -> f64 {
        Falloff::Smooth.w(t)
    }

    fn dab(&self, vol: &mut Volume, centre: V3, dir: V3, step: f64) {
        let (r, h, s) = (self.radius, vol.h, self.strength);
        let sign = if self.invert { -1.0 } else { 1.0 };
        let p = self.project(vol, centre);
        // no surface within reach: nothing to work on (a dab in empty space would leave a floating blob)
        let Some((c, n)) = self.frame(vol, p) else {
            return;
        };
        let c = if len(sub(c, p)) < r { c } else { p };
        let (lo, hi) = vol.index_box(
            [p[0] - r - h, p[1] - r - h, p[2] - r - h],
            [p[0] + r + h, p[1] + r + h, p[2] + r + h],
        );
        let src = vol.snapshot(lo, hi);
        let fo = self.falloff;
        let kind = self.kind;
        let ncen = n;
        vol.edit(lo, hi, &mut |x, d| {
            let dist = len(sub(x, p));
            let w = fo.w(dist / r);
            if w <= 0.0 {
                return d;
            }
            match kind {
                Kind::Draw => {
                    // a blob of radius r/2 whose top stands `a` above the surface (sunk into it when carving)
                    let a = s * 0.3 * r;
                    let rb = 0.5 * r;
                    let cc = add(c, mul(ncen, sign * (a - rb)));
                    let blob = len(sub(x, cc)) - rb;
                    if sign > 0.0 {
                        smin(d, blob, 0.4 * rb)
                    } else {
                        smax(d, -blob, 0.4 * rb)
                    }
                }
                Kind::Clay => {
                    let a = s * 0.12 * r;
                    if sign > 0.0 {
                        let plane = dot(sub(x, add(c, mul(ncen, a))), ncen);
                        d + w * (d.min(plane) - d)
                    } else {
                        let plane = dot(sub(x, sub(c, mul(ncen, a))), ncen);
                        d + w * (d.max(plane) - d)
                    }
                }
                Kind::Inflate => d - sign * s * 0.1 * r * w,
                Kind::Flatten => {
                    let plane = dot(sub(x, c), ncen);
                    d + w * s.min(1.0) * (plane - d)
                }
                Kind::Scrape => {
                    let plane = dot(sub(x, sub(c, mul(ncen, s * 0.08 * r))), ncen);
                    d + w * (d.max(plane) - d)
                }
                Kind::Smooth => {
                    let mut sum = 0.0;
                    for a in 0..3 {
                        let (mut u, mut v) = (x, x);
                        u[a] += h;
                        v[a] -= h;
                        sum += src.sample(u) + src.sample(v);
                    }
                    d + w * (s * 0.8).min(1.0) * (sum / 6.0 - d)
                }
                Kind::Pinch => {
                    // sample from farther out, so the shape draws in towards the dab
                    let v = sub(p, x);
                    let vt = sub(v, mul(ncen, dot(v, ncen)));
                    let x2 = sub(x, mul(vt, w * s * 0.35 * sign));
                    src.sample(x2)
                }
                Kind::Grease => {
                    if len(dir) == 0.0 {
                        return d;
                    }
                    let x2 = sub(x, mul(dir, w * s * step * sign));
                    src.sample(x2)
                }
            }
        });
    }
}
