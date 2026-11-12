//! Skeleton maths: forward kinematics, joint limits, inverse kinematics (an exact two-bone solver
//! and FABRIK for longer chains) and linear blend skinning. Pure functions of their inputs, so the
//! results are reproducible. Angles are radians, clockwise on screen (y points down): 0 is +x.
use crate::math::*;
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub struct BoneDef {
    /// Parents are listed before their children.
    pub parent: Option<usize>,
    /// Where on the parent the head sits: 0 at its head, 1 at its tip.
    pub at: f64,
    pub length: f64,
    /// Rest direction relative to the parent's direction (absolute for a root).
    pub rest_angle: f64,
    /// A root's rest head position (ignored for children).
    pub head: Vec2,
    /// Allowed pose rotation relative to the rest direction.
    pub limits: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug)]
pub struct BonePose {
    pub head: Vec2,
    /// World direction.
    pub angle: f64,
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub bones: Vec<BoneDef>,
}

/// Which side the middle joint of a two-bone chain bends to: `Positive` puts it on the clockwise side
/// of the line to the target (below it when reaching right on screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bend {
    Positive,
    Negative,
}

fn dir(a: f64) -> Vec2 {
    v2(a.cos(), a.sin())
}

/// An angle wrapped into (-pi, pi].
pub fn wrap_angle(a: f64) -> f64 {
    let t = a - 2.0 * PI * (a / (2.0 * PI)).round();
    if t <= -PI {
        t + 2.0 * PI
    } else {
        t
    }
}

fn atan2v(v: Vec2) -> f64 {
    v.y.atan2(v.x)
}

impl Skeleton {
    pub fn new(bones: Vec<BoneDef>) -> Skeleton {
        Skeleton { bones }
    }

    /// World pose of every bone for pose rotations `rots` (relative to rest); roots are moved by `root_offset`.
    pub fn pose(&self, rots: &[f64], root_offset: Vec2) -> Vec<BonePose> {
        let mut out: Vec<BonePose> = Vec::with_capacity(self.bones.len());
        for (i, b) in self.bones.iter().enumerate() {
            let r = rots.get(i).copied().unwrap_or(0.0);
            out.push(match b.parent {
                None => BonePose {
                    head: b.head + root_offset,
                    angle: b.rest_angle + r,
                },
                Some(p) => {
                    let pp = out[p];
                    BonePose {
                        head: pp.head + dir(pp.angle) * (self.bones[p].length * b.at),
                        angle: pp.angle + b.rest_angle + r,
                    }
                }
            });
        }
        out
    }

    pub fn tip(&self, pose: &[BonePose], i: usize) -> Vec2 {
        pose[i].head + dir(pose[i].angle) * self.bones[i].length
    }

    /// Clamp each rotation to its bone's limits; true when anything was changed.
    pub fn clamp_limits(&self, rots: &mut [f64]) -> bool {
        let mut changed = false;
        for (b, r) in self.bones.iter().zip(rots.iter_mut()) {
            if let Some((lo, hi)) = b.limits {
                let c = r.clamp(lo, hi);
                if c != *r {
                    *r = c;
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Pose the chain (bone indices, each the child of the previous at its tip) so its last tip reaches
/// `target`, writing the pose rotations of the chain's bones into `rots`. Two bones are solved exactly
/// (law of cosines, `bend` picks the side); longer chains use FABRIK for up to `iterations` passes.
/// An unreachable target stretches the chain straight at it. Limits are honoured. Returns whether
/// the tip reached the target.
pub fn solve_ik(
    sk: &Skeleton,
    rots: &mut [f64],
    root_offset: Vec2,
    chain: &[usize],
    target: Vec2,
    bend: Bend,
    iterations: usize,
) -> bool {
    if chain.len() < 2 {
        return false;
    }
    let pose = sk.pose(rots, root_offset);
    let first = chain[0];
    let (head, parent_angle) = match sk.bones[first].parent {
        None => (pose[first].head, 0.0),
        Some(p) => (pose[first].head, pose[p].angle),
    };
    let lens: Vec<f64> = chain.iter().map(|&b| sk.bones[b].length).collect();
    let total: f64 = lens.iter().sum();
    let to = target - head;
    let dist = to.length();
    // world angles of the chain bones
    let mut angles = vec![0.0; chain.len()];
    if chain.len() == 2 {
        let (l1, l2) = (lens[0], lens[1]);
        let dc = dist.clamp((l1 - l2).abs() + 1e-9, l1 + l2);
        let base = if dist > 1e-12 { atan2v(to) } else { 0.0 };
        let cos_a = ((l1 * l1 + dc * dc - l2 * l2) / (2.0 * l1 * dc)).clamp(-1.0, 1.0);
        let alpha = cos_a.acos();
        let reach = head + dir(base) * dc;
        // both elbow sides; limits may rule one out, so the side that ends closer to the target wins
        // (the requested side on a tie)
        let solution = |sign: f64| -> ([f64; 2], f64) {
            let t1 = base + sign * alpha;
            let elbow = head + dir(t1) * l1;
            let t2 = atan2v(reach - elbow);
            let mut parent_world = parent_angle;
            let mut a = [t1, t2];
            let mut r = [0.0; 2];
            for k in 0..2 {
                let bd = &sk.bones[chain[k]];
                let mut rel = wrap_angle(a[k] - parent_world - bd.rest_angle);
                if let Some((lo, hi)) = bd.limits {
                    rel = rel.clamp(lo, hi);
                }
                r[k] = rel;
                a[k] = parent_world + bd.rest_angle + rel;
                parent_world = a[k];
            }
            let tip = head + dir(a[0]) * l1 + dir(a[1]) * l2;
            (a, (tip - target).length())
        };
        let first = if bend == Bend::Positive { 1.0 } else { -1.0 };
        let (a1, e1) = solution(first);
        let (a2, e2) = solution(-first);
        let chosen = if e2 + 1e-9 < e1 { a2 } else { a1 };
        angles[0] = chosen[0];
        angles[1] = chosen[1];
    } else if dist >= total {
        let a = atan2v(to);
        angles.iter_mut().for_each(|x| *x = a);
    } else {
        // FABRIK on the joint positions
        let mut p: Vec<Vec2> = Vec::with_capacity(chain.len() + 1);
        p.push(head);
        for (k, &b) in chain.iter().enumerate() {
            p.push(p[k] + dir(pose[b].angle) * lens[k]);
        }
        let tol = 1e-4 * total;
        let mut ran = false;
        for _ in 0..iterations.max(1) {
            if (p[chain.len()] - target).length() < tol {
                break;
            }
            ran = true;
            let n = chain.len();
            p[n] = target;
            for i in (0..n).rev() {
                let d = (p[i] - p[i + 1]).normalized();
                p[i] = p[i + 1] + d * lens[i];
            }
            p[0] = head;
            let mut parent_world = parent_angle;
            for i in 0..n {
                let b = &sk.bones[chain[i]];
                let mut a = atan2v(p[i + 1] - p[i]);
                if let Some((lo, hi)) = b.limits {
                    let rel = wrap_angle(a - parent_world - b.rest_angle).clamp(lo, hi);
                    a = parent_world + b.rest_angle + rel;
                }
                angles[i] = a;
                p[i + 1] = p[i] + dir(a) * lens[i];
                parent_world = a;
            }
        }
        if !ran {
            // already on target: keep the current pose angles
            for (i, &b) in chain.iter().enumerate() {
                angles[i] = pose[b].angle;
            }
        }
    }
    // back to pose rotations relative to rest, clamped
    let mut parent_world = parent_angle;
    for (k, &b) in chain.iter().enumerate() {
        let bd = &sk.bones[b];
        let mut r = wrap_angle(angles[k] - parent_world - bd.rest_angle);
        if let Some((lo, hi)) = bd.limits {
            r = r.clamp(lo, hi);
        }
        rots[b] = r;
        parent_world += bd.rest_angle + r;
    }
    let after = sk.pose(rots, root_offset);
    let tip = sk.tip(&after, *chain.last().expect("non-empty"));
    (tip - target).length() < 1e-3 * total.max(1e-9)
}

fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let l2 = ab.length_sq();
    let t = if l2 > 1e-18 {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p - (a + ab * t)).length()
}

/// Automatic skin weights: each point follows the listed bones with weights that fall off with its
/// distance to the bone segment relative to the nearest one (`blend` is the falloff length). The
/// weights of a point sum to 1; negligible ones are dropped.
pub fn auto_weights(
    points: &[Vec2],
    sk: &Skeleton,
    rest: &[BonePose],
    bones: &[usize],
    blend: f64,
) -> Vec<Vec<(usize, f64)>> {
    let blend = blend.max(1e-9);
    points
        .iter()
        .map(|p| {
            let d: Vec<(usize, f64)> = bones
                .iter()
                .map(|&b| (b, dist_to_segment(*p, rest[b].head, sk.tip(rest, b))))
                .collect();
            let dmin = d.iter().map(|x| x.1).fold(f64::MAX, f64::min);
            let mut w: Vec<(usize, f64)> = d
                .iter()
                .map(|&(b, di)| (b, (-(di - dmin) / blend).exp()))
                .collect();
            let sum: f64 = w.iter().map(|x| x.1).sum();
            w.iter_mut().for_each(|x| x.1 /= sum);
            w.retain(|x| x.1 >= 0.005);
            let sum: f64 = w.iter().map(|x| x.1).sum();
            w.iter_mut().for_each(|x| x.1 /= sum);
            w.sort_by_key(|x| x.0);
            w
        })
        .collect()
}

/// Linear blend skinning of one rest-pose point: the weighted sum of where each bone's motion takes it.
pub fn skin_point(
    _sk: &Skeleton,
    rest: &[BonePose],
    pose: &[BonePose],
    weights: &[(usize, f64)],
    p: Vec2,
) -> Vec2 {
    let mut out = Vec2::ZERO;
    for &(b, w) in weights {
        let d = pose[b].angle - rest[b].angle;
        let rot = Rot::from_angle(d);
        out += (pose[b].head + rot.rotate(p - rest[b].head)) * w;
    }
    out
}
