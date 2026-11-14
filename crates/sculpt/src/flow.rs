//! Flows: velocity fields (vortices, curl noise, a uniform wind) that stir a volume. The field is moved with a
//! backward semi-Lagrangian step (each new distance is read from where the material came from), and
//! `reinitialize` restores the distance property afterwards.
use crate::noise::Spec;
use crate::volume::Volume;
use crate::{add, cross, len, mul, sub, V3};
use std::sync::Arc;

/// A swirl: material circles an axis through `center` with angular speed `omega` (radians per second) at the
/// centre, fading away with a Gaussian of width `sigma`. Points keep their distance from the centre, so they
/// orbit it at a constant angular speed `omega * exp(-r^2 / 2 sigma^2)`.
#[derive(Clone, Copy, Debug)]
pub struct Vortex {
    pub center: V3,
    pub axis: V3,
    pub omega: f64,
    pub sigma: f64,
}

impl Vortex {
    pub fn velocity(&self, p: V3) -> V3 {
        let r = sub(p, self.center);
        let fall =
            (-(r[0] * r[0] + r[1] * r[1] + r[2] * r[2]) / (2.0 * self.sigma * self.sigma)).exp();
        mul(cross(crate::norm(self.axis), r), self.omega * fall)
    }
}

#[derive(Clone)]
pub enum Field {
    Vortex(Vortex),
    /// Divergence-free swirls from a noise: the curl of three potentials, times `amount` (units per second).
    Curl(Arc<Spec>, f64),
    /// A steady wind.
    Uniform(V3),
    Sum(Vec<Field>),
}

impl Field {
    pub fn velocity(&self, p: V3) -> V3 {
        match self {
            Field::Vortex(v) => v.velocity(p),
            Field::Curl(s, amount) => mul(s.curl(p), *amount),
            Field::Uniform(v) => *v,
            Field::Sum(f) => f.iter().fold([0.0; 3], |acc, g| add(acc, g.velocity(p))),
        }
    }
}

/// Move the volume with a velocity field for `dt` seconds in `steps` sub-steps.
pub fn advect(vol: &mut Volume, field: &dyn Fn(V3) -> V3, dt: f64, steps: u32) {
    let Some((lo, hi)) = vol.active_box() else {
        return;
    };
    // how far material can move: the largest speed on a coarse lattice over the box
    let h = vol.h;
    let mut vmax: f64 = 0.0;
    let stride = 4;
    let mut k = lo[2];
    while k <= hi[2] {
        let mut j = lo[1];
        while j <= hi[1] {
            let mut i = lo[0];
            while i <= hi[0] {
                vmax = vmax.max(len(field([i as f64 * h, j as f64 * h, k as f64 * h])));
                i += stride;
            }
            j += stride;
        }
        k += stride;
    }
    let pad = ((vmax * dt / h).ceil() as i32 + 3).min(64);
    let (lo, hi) = (
        [lo[0] - pad, lo[1] - pad, lo[2] - pad],
        [hi[0] + pad, hi[1] + pad, hi[2] + pad],
    );
    let sub_dt = dt / steps.max(1) as f64;
    for _ in 0..steps.max(1) {
        let src = vol.snapshot(lo, hi);
        vol.edit(lo, hi, &mut |x, _| {
            // a midpoint trace back to where this material was
            let v1 = field(x);
            let mid = sub(x, mul(v1, 0.5 * sub_dt));
            let v2 = field(mid);
            src.sample(sub(x, mul(v2, sub_dt)))
        });
    }
}

/// Restore `|grad d| = 1` near the surface (the Eikonal equation, a few Godunov steps), so brushes and meshing keep
/// behaving after a flow has stretched or squeezed the field.
pub fn reinitialize(vol: &mut Volume, iterations: u32) {
    let Some((lo, hi)) = vol.active_box() else {
        return;
    };
    let h = vol.h;
    let (lo, hi) = (
        [lo[0] - 1, lo[1] - 1, lo[2] - 1],
        [hi[0] + 1, hi[1] + 1, hi[2] + 1],
    );
    let phi0 = vol.snapshot(lo, hi);
    let dtau = 0.3 * h;
    let band = vol.band;
    for _ in 0..iterations {
        let src = vol.snapshot(lo, hi);
        vol.edit(lo, hi, &mut |x, d| {
            if d.abs() >= band * 0.999 && phi0.sample(x).abs() >= band * 0.999 {
                return d;
            }
            let s0 = phi0.sample(x);
            let sign = s0 / (s0 * s0 + h * h).sqrt();
            let mut g2 = 0.0;
            for a in 0..3 {
                let (mut u, mut w) = (x, x);
                u[a] += h;
                w[a] -= h;
                let (dm, dp) = ((d - src.sample(w)) / h, (src.sample(u) - d) / h);
                g2 += if sign > 0.0 {
                    dm.max(0.0).powi(2).max(dp.min(0.0).powi(2))
                } else {
                    dm.min(0.0).powi(2).max(dp.max(0.0).powi(2))
                };
            }
            d - dtau * sign * (g2.sqrt() - 1.0)
        });
    }
}
