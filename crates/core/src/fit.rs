//! Fitting digitised (hand-drawn) points with cubic Beziers: Philip J. Schneider's algorithm ("An Algorithm for
//! Automatically Fitting Digitized Curves", Graphics Gems, 1990), with corner detection so sharp turns stay sharp,
//! plus the helpers that go with it (`simplify`, `resample`, `path_length`). Pure functions of their input.
use crate::geometry::P;

/// One cubic Bezier piece: end points `p0`, `p3` and control points `c1`, `c2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitSeg {
    pub p0: P,
    pub c1: P,
    pub c2: P,
    pub p3: P,
}

impl FitSeg {
    pub fn at(&self, t: f64) -> P {
        let u = 1.0 - t;
        let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        [
            self.p0[0] * b0 + self.c1[0] * b1 + self.c2[0] * b2 + self.p3[0] * b3,
            self.p0[1] * b0 + self.c1[1] * b1 + self.c2[1] * b2 + self.p3[1] * b3,
        ]
    }
    fn d1(&self, t: f64) -> P {
        let u = 1.0 - t;
        let k = [3.0 * u * u, 6.0 * u * t, 3.0 * t * t];
        let (a, b, c) = (
            sub(self.c1, self.p0),
            sub(self.c2, self.c1),
            sub(self.p3, self.c2),
        );
        [
            a[0] * k[0] + b[0] * k[1] + c[0] * k[2],
            a[1] * k[0] + b[1] * k[1] + c[1] * k[2],
        ]
    }
    fn d2(&self, t: f64) -> P {
        let u = 1.0 - t;
        let a = [
            self.c2[0] - 2.0 * self.c1[0] + self.p0[0],
            self.c2[1] - 2.0 * self.c1[1] + self.p0[1],
        ];
        let b = [
            self.p3[0] - 2.0 * self.c2[0] + self.c1[0],
            self.p3[1] - 2.0 * self.c2[1] + self.c1[1],
        ];
        [6.0 * (u * a[0] + t * b[0]), 6.0 * (u * a[1] + t * b[1])]
    }
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}
fn norm(a: P) -> P {
    let l = len(a);
    if l < 1e-12 {
        [0.0, 0.0]
    } else {
        mul(a, 1.0 / l)
    }
}

/// The length of the polyline through the points.
pub fn path_length(pts: &[P]) -> f64 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// Points `spacing` apart along the path (the first and last points are kept exactly; the last gap may be shorter).
pub fn resample(pts: &[P], spacing: f64) -> Vec<P> {
    if pts.len() < 2 || spacing <= 0.0 {
        return pts.to_vec();
    }
    let total = path_length(pts);
    let mut out = vec![pts[0]];
    let (mut next, mut run, mut seg) = (spacing, 0.0, 0usize);
    while next < total - 1e-9 && seg + 1 < pts.len() {
        let l = dist(pts[seg], pts[seg + 1]);
        if run + l >= next - 1e-12 {
            let t = if l > 0.0 { (next - run) / l } else { 0.0 };
            out.push(add(pts[seg], mul(sub(pts[seg + 1], pts[seg]), t)));
            next += spacing;
        } else {
            run += l;
            seg += 1;
        }
    }
    out.push(*pts.last().expect("not empty"));
    out
}

/// Douglas-Peucker: the fewest of the points that keep the path within `tolerance` of the original.
pub fn simplify(pts: &[P], tolerance: f64) -> Vec<P> {
    let pts = dedupe(pts);
    if pts.len() < 3 {
        return pts;
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    *keep.last_mut().expect("not empty") = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut worst, mut at) = (0.0, a);
        for i in a + 1..b {
            let d = point_segment(pts[i], pts[a], pts[b]);
            if d > worst {
                worst = d;
                at = i;
            }
        }
        if worst > tolerance {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    pts.iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect()
}

fn point_segment(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-18 {
        return dist(p, a);
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    dist(p, add(a, mul(ab, t)))
}

fn dedupe(pts: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_none_or(|q| dist(*q, *p) > 1e-9) {
            out.push(*p);
        }
    }
    out
}

/// Fit cubic Beziers through `points`. `tolerance` is the largest distance (in the units of the points) a point may
/// be from the curve; `corner_deg` is the turn above which the path is cut and both sides keep a sharp corner;
/// `closed` joins the last point to the first (smoothly unless there is a corner there).
pub fn fit_curve(points: &[P], tolerance: f64, corner_deg: f64, closed: bool) -> Vec<FitSeg> {
    let mut pts = dedupe(points);
    if pts.len() < 2 {
        return vec![];
    }
    let tol = tolerance.max(1e-6);
    let diag = {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &pts {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        dist(lo, hi)
    };
    let mut closed = closed;
    if closed
        && pts.len() > 2
        && dist(pts[0], *pts.last().expect("not empty")) < 1e-6 * (1.0 + diag)
    {
        pts.pop();
    }
    if closed && pts.len() < 3 {
        closed = false;
    }
    // Noise smaller than the tolerance only makes the fit wander: smooth the path a little first, and spend the
    // distance the smoothing moved the points out of the tolerance, so the curve stays within `tolerance` of the
    // points that were given.
    let mut tol = tol;
    for strength in [0.7, 0.4, 0.2] {
        let sm = smooth_points(&pts, strength * tol, closed);
        let dev = pts
            .iter()
            .zip(&sm)
            .map(|(a, b)| dist(*a, *b))
            .fold(0.0, f64::max);
        if dev <= 0.5 * tol {
            tol -= dev;
            pts = sm;
            break;
        }
    }
    let corners = find_corners(&pts, tol, corner_deg, closed);
    let mut out = vec![];
    let err2 = tol * tol;
    if !closed {
        let mut cuts = vec![0usize];
        cuts.extend(corners);
        cuts.push(pts.len() - 1);
        for w in cuts.windows(2) {
            fit_piece(&pts[w[0]..=w[1]], None, tol, err2, &mut out);
        }
    } else if corners.is_empty() {
        // a smooth closed curve: one piece that starts and ends at the first point with the same tangent
        let n = pts.len();
        let t = norm(sub(pts[1], pts[n - 1]));
        let mut piece = pts.clone();
        piece.push(pts[0]);
        fit_piece(&piece, Some(t), tol, err2, &mut out);
    } else {
        // start at the first corner and cut at the others; the last piece ends back at the first corner
        let first = corners[0];
        let mut rot: Vec<P> = pts[first..].to_vec();
        rot.extend_from_slice(&pts[..first]);
        rot.push(pts[first]);
        let mut cuts: Vec<usize> = vec![0];
        cuts.extend(corners[1..].iter().map(|c| c - first));
        cuts.push(rot.len() - 1);
        for w in cuts.windows(2) {
            fit_piece(&rot[w[0]..=w[1]], None, tol, err2, &mut out);
        }
    }
    out
}

/// A moving average over `w` of path length on each side (triangular weights); the ends of an open path stay put.
fn smooth_points(pts: &[P], w: f64, closed: bool) -> Vec<P> {
    let n = pts.len();
    if n < 3 || w <= 0.0 {
        return pts.to_vec();
    }
    let mut cum = vec![0.0; n];
    for i in 1..n {
        cum[i] = cum[i - 1] + dist(pts[i], pts[i - 1]);
    }
    let total = cum[n - 1]
        + if closed {
            dist(pts[n - 1], pts[0])
        } else {
            0.0
        };
    (0..n)
        .map(|i| {
            if !closed && (i == 0 || i == n - 1) {
                return pts[i];
            }
            let (mut sum, mut wsum) = ([0.0, 0.0], 0.0);
            for j in 0..n {
                let mut d = (cum[j] - cum[i]).abs();
                if closed {
                    d = d.min(total - d);
                }
                if d <= w {
                    let k = 1.0 - d / w;
                    sum = add(sum, mul(pts[j], k));
                    wsum += k;
                }
            }
            if wsum > 0.0 {
                mul(sum, 1.0 / wsum)
            } else {
                pts[i]
            }
        })
        .collect()
}

/// Where the path turns by more than `corner_deg` over a window of about `4 * tol`: one index per sharp turn.
fn find_corners(pts: &[P], tol: f64, corner_deg: f64, closed: bool) -> Vec<usize> {
    let n = pts.len();
    if n < 3 || corner_deg >= 180.0 {
        return vec![];
    }
    let window = (4.0 * tol).max(4.0);
    let at = |i: isize| -> P { pts[i.rem_euclid(n as isize) as usize] };
    let mut cand: Vec<(usize, f64)> = vec![];
    let range: Vec<usize> = if closed {
        (0..n).collect()
    } else {
        (1..n - 1).collect()
    };
    for &i in &range {
        // walk away from i on both sides until the path is `window` long
        let walk = |dir: isize| -> Option<P> {
            let (mut d, mut k) = (0.0, 0isize);
            loop {
                k += dir;
                let steps_left = if closed {
                    n as isize
                } else if dir < 0 {
                    i as isize
                } else {
                    (n - 1 - i) as isize
                };
                if k.abs() > steps_left || k.abs() >= n as isize {
                    // ran out of path: an open path's end counts if it is far enough away
                    return if d >= window * 0.5 {
                        Some(at(i as isize + k - dir))
                    } else {
                        None
                    };
                }
                d += dist(at(i as isize + k), at(i as isize + k - dir));
                if d >= window {
                    return Some(at(i as isize + k));
                }
            }
        };
        let (Some(a), Some(b)) = (walk(-1), walk(1)) else {
            continue;
        };
        let (v1, v2) = (sub(pts[i], a), sub(b, pts[i]));
        let (l1, l2) = (len(v1), len(v2));
        if l1 < 1e-9 || l2 < 1e-9 {
            continue;
        }
        let angle = (dot(v1, v2) / (l1 * l2))
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        if angle > corner_deg {
            cand.push((i, angle));
        }
    }
    // among candidates close together along the path (a turn spread over several points) keep the sharpest
    let mut cum = vec![0.0; n];
    for i in 1..n {
        cum[i] = cum[i - 1] + dist(pts[i], pts[i - 1]);
    }
    let mut out: Vec<usize> = vec![];
    let mut best: Option<(usize, f64)> = None;
    let mut last_seen: Option<usize> = None;
    for (i, a) in cand {
        let near = last_seen.is_some_and(|l| cum[i] - cum[l] <= window);
        match best {
            Some((_, ba)) if near && a <= ba => {}
            Some(_) if near => best = Some((i, a)),
            Some((bi, _)) => {
                out.push(bi);
                best = Some((i, a));
            }
            None => best = Some((i, a)),
        }
        last_seen = Some(i);
    }
    if let Some((bi, _)) = best {
        out.push(bi);
    }
    out
}

/// Fit one piece between corners. `joint` is the shared tangent of a smooth closed curve's start and end.
fn fit_piece(piece: &[P], joint: Option<P>, tol: f64, err2: f64, out: &mut Vec<FitSeg>) {
    let n = piece.len();
    if n < 2 {
        return;
    }
    // the direction at an end: the sum of the vectors to the points within a short stretch of path (not just the
    // next point, which noise tilts), at least `2 * tol` long and at most a third of the piece
    let total = path_length(piece);
    let reach = (2.0 * tol).max(1.0).min(total / 3.0);
    let tangent = |from_end: bool| -> P {
        let base = if from_end { piece[n - 1] } else { piece[0] };
        let (mut sum, mut j) = ([0.0, 0.0], 1usize);
        while j < n {
            let p = if from_end { piece[n - 1 - j] } else { piece[j] };
            sum = add(sum, sub(p, base));
            if dist(p, base) >= reach {
                break;
            }
            j += 1;
        }
        norm(sum)
    };
    let (t1, t2) = match joint {
        Some(t) => (t, mul(t, -1.0)),
        None => (tangent(false), tangent(true)),
    };
    fit_cubic(piece, t1, t2, err2, out);
}

fn fit_cubic(d: &[P], t1: P, t2: P, err2: f64, out: &mut Vec<FitSeg>) {
    let n = d.len();
    if n == 2 {
        let k = dist(d[0], d[1]) / 3.0;
        out.push(FitSeg {
            p0: d[0],
            c1: add(d[0], mul(t1, k)),
            c2: add(d[1], mul(t2, k)),
            p3: d[1],
        });
        return;
    }
    let mut u = chord_params(d);
    let mut bez = generate(d, &u, t1, t2);
    let (mut max, mut split) = max_error(d, &bez, &u);
    // the error at the sample parameters can be a little under the true distance to the curve: check that too
    if max < err2 && true_error(d, &bez) < err2 {
        out.push(bez);
        return;
    }
    if max < err2 {
        max = err2 * 2.0; // not good enough after all: take the Newton step below
    }
    if max < err2 * 16.0 {
        // close: improve the parameter values with Newton-Raphson and try again
        for _ in 0..4 {
            let up = reparam(d, &u, &bez);
            bez = generate(d, &up, t1, t2);
            let (m, s) = max_error(d, &bez, &up);
            max = m;
            split = s;
            u = up;
            if max < err2 && true_error(d, &bez) < err2 {
                out.push(bez);
                return;
            }
        }
    }
    let _ = max;
    // the tangent where the curve is cut: across a few points, so noise in single points does not tilt it
    let reach = (2.0 * err2.sqrt()).max(1.0);
    let (mut a, mut b) = (split - 1, split + 1);
    while a > 0 && dist(d[a], d[split]) < reach && split - a < 6 {
        a -= 1;
    }
    while b < n - 1 && dist(d[b], d[split]) < reach && b - split < 6 {
        b += 1;
    }
    let center = norm(sub(d[a], d[b]));
    fit_cubic(&d[..=split], t1, center, err2, out);
    fit_cubic(&d[split..], mul(center, -1.0), t2, err2, out);
}

/// The largest squared distance from a point to the curve: the nearest of 32 samples, refined by ternary search.
fn true_error(d: &[P], bez: &FitSeg) -> f64 {
    let samples: Vec<P> = (0..=32).map(|k| bez.at(k as f64 / 32.0)).collect();
    let d2 = |p: P, t: f64| {
        let q = bez.at(t);
        dot(sub(q, p), sub(q, p))
    };
    d.iter()
        .map(|p| {
            let (mut best, mut bt) = (f64::MAX, 0.0);
            for (k, q) in samples.iter().enumerate() {
                let e = dot(sub(*q, *p), sub(*q, *p));
                if e < best {
                    best = e;
                    bt = k as f64 / 32.0;
                }
            }
            let (mut lo, mut hi) = ((bt - 1.0 / 32.0).max(0.0), (bt + 1.0 / 32.0).min(1.0));
            for _ in 0..14 {
                let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
                if d2(*p, m1) < d2(*p, m2) {
                    hi = m2;
                } else {
                    lo = m1;
                }
            }
            best.min(d2(*p, 0.5 * (lo + hi)))
        })
        .fold(0.0, f64::max)
}

fn chord_params(d: &[P]) -> Vec<f64> {
    let mut u = vec![0.0; d.len()];
    for i in 1..d.len() {
        u[i] = u[i - 1] + dist(d[i], d[i - 1]);
    }
    let total = u[d.len() - 1];
    for v in u.iter_mut() {
        *v /= total;
    }
    u
}

/// Least-squares control points for the given tangents and parameter values.
fn generate(d: &[P], u: &[f64], t1: P, t2: P) -> FitSeg {
    let (first, last) = (d[0], d[d.len() - 1]);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..d.len() {
        let t = u[i];
        let s = 1.0 - t;
        let (b0, b1, b2, b3) = (s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t);
        let (a1, a2) = (mul(t1, b1), mul(t2, b2));
        c00 += dot(a1, a1);
        c01 += dot(a1, a2);
        c11 += dot(a2, a2);
        let tmp = sub(d[i], add(mul(first, b0 + b1), mul(last, b2 + b3)));
        x0 += dot(a1, tmp);
        x1 += dot(a2, tmp);
    }
    let det = c00 * c11 - c01 * c01;
    let (mut al, mut ar) = if det.abs() < 1e-30 {
        (0.0, 0.0)
    } else {
        ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det)
    };
    let seg = dist(first, last);
    // a control arm much longer than the piece is a numerical accident of noisy points: fall back to thirds
    if !(al.is_finite() && ar.is_finite())
        || al < 1e-6 * seg
        || ar < 1e-6 * seg
        || al > 1.5 * seg
        || ar > 1.5 * seg
    {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    FitSeg {
        p0: first,
        c1: add(first, mul(t1, al)),
        c2: add(last, mul(t2, ar)),
        p3: last,
    }
}

fn max_error(d: &[P], bez: &FitSeg, u: &[f64]) -> (f64, usize) {
    let (mut max, mut at) = (0.0, d.len() / 2);
    for i in 1..d.len() - 1 {
        let q = bez.at(u[i]);
        let e = dot(sub(q, d[i]), sub(q, d[i]));
        if e >= max {
            max = e;
            at = i;
        }
    }
    (max, at)
}

fn reparam(d: &[P], u: &[f64], bez: &FitSeg) -> Vec<f64> {
    d.iter()
        .zip(u)
        .map(|(p, &t)| {
            let (q, q1, q2) = (bez.at(t), bez.d1(t), bez.d2(t));
            let num = dot(sub(q, *p), q1);
            let den = dot(q1, q1) + dot(sub(q, *p), q2);
            if den.abs() < 1e-30 {
                t
            } else {
                (t - num / den).clamp(0.0, 1.0)
            }
        })
        .collect()
}
