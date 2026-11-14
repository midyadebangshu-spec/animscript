//! Skeleton maths for 3D rigs: forward kinematics from per-bone delta rotations, joint limits,
//! inverse kinematics (exact two-bone with a pole, FABRIK for longer chains) and linear blend
//! skinning weights. Pure functions of their inputs; no state, no randomness.
use crate::math::{v3, Quat, V3};

/// One bone of the rest pose. Bones are listed parents first.
#[derive(Clone, Debug)]
pub struct BoneDef3 {
    pub parent: Option<usize>,
    /// Rest head in the world.
    pub head: V3,
    /// Unit rest direction (head to tip).
    pub dir: V3,
    pub length: f64,
    /// Hinge: only rotation about this world axis (turns of the delta), clamped to `(lo, hi)` radians.
    pub hinge: Option<(V3, f64, f64)>,
    /// Ball: swing cone half-angle (radians) about the rest direction.
    pub swing: Option<f64>,
    /// Ball: twist range (radians) about the rest direction.
    pub twist: Option<(f64, f64)>,
}

impl BoneDef3 {
    pub fn new(parent: Option<usize>, head: V3, dir: V3, length: f64) -> BoneDef3 {
        BoneDef3 {
            parent,
            head,
            dir: dir.normalized(),
            length,
            hinge: None,
            swing: None,
            twist: None,
        }
    }
    pub fn tip(&self) -> V3 {
        self.head + self.dir * self.length
    }
}

/// The posed bone: its head, and the cumulative rotation relative to rest.
#[derive(Clone, Copy, Debug)]
pub struct BonePose {
    pub head: V3,
    pub c: Quat,
    pub tip: V3,
}

#[derive(Clone, Debug, Default)]
pub struct Skeleton3 {
    pub bones: Vec<BoneDef3>,
}

fn qneg(q: Quat) -> Quat {
    Quat {
        w: -q.w,
        x: -q.x,
        y: -q.y,
        z: -q.z,
    }
}

/// Spherical interpolation, the short way round.
pub fn slerp(a: Quat, b: Quat, t: f64) -> Quat {
    let mut d = a.w * b.w + a.x * b.x + a.y * b.y + a.z * b.z;
    let b = if d < 0.0 {
        d = -d;
        qneg(b)
    } else {
        b
    };
    if d > 0.9995 {
        return Quat {
            w: a.w + (b.w - a.w) * t,
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
            z: a.z + (b.z - a.z) * t,
        }
        .normalized();
    }
    let th = d.clamp(-1.0, 1.0).acos();
    let s = th.sin();
    let (wa, wb) = (((1.0 - t) * th).sin() / s, (t * th).sin() / s);
    Quat {
        w: a.w * wa + b.w * wb,
        x: a.x * wa + b.x * wb,
        y: a.y * wa + b.y * wb,
        z: a.z * wa + b.z * wb,
    }
    .normalized()
}

/// The shortest rotation taking unit vector `a` to unit vector `b`.
pub fn arc(a: V3, b: V3) -> Quat {
    let d = a.dot(b).clamp(-1.0, 1.0);
    if d > 1.0 - 1e-12 {
        return Quat::IDENTITY;
    }
    if d < -1.0 + 1e-12 {
        return Quat::from_axis_angle(a.any_perp(), std::f64::consts::PI);
    }
    let c = a.cross(b);
    Quat {
        w: 1.0 + d,
        x: c.x,
        y: c.y,
        z: c.z,
    }
    .normalized()
}

/// The part of `q` that is a rotation about `axis` (swing-twist decomposition).
fn twist_about(q: Quat, axis: V3) -> Quat {
    let q = if q.w < 0.0 { qneg(q) } else { q };
    let p = axis * (v3(q.x, q.y, q.z).dot(axis));
    if q.w * q.w + p.len2() < 1e-18 {
        return Quat::IDENTITY;
    }
    Quat {
        w: q.w,
        x: p.x,
        y: p.y,
        z: p.z,
    }
    .normalized()
}

impl Skeleton3 {
    pub fn new(bones: Vec<BoneDef3>) -> Skeleton3 {
        Skeleton3 { bones }
    }

    /// Forward kinematics. `deltas[i]` is bone i's rotation about its own head, in the rest-world
    /// frame its parent carries; `root` moves the whole skeleton.
    pub fn pose(&self, root: V3, deltas: &[Quat]) -> Vec<BonePose> {
        let mut out: Vec<BonePose> = Vec::with_capacity(self.bones.len());
        for (i, b) in self.bones.iter().enumerate() {
            let d = deltas.get(i).copied().unwrap_or(Quat::IDENTITY);
            let (head, c) = match b.parent {
                None => (b.head + root, d),
                Some(p) => {
                    let pp = &out[p];
                    let pb = &self.bones[p];
                    (pp.head + pp.c.rotate(b.head - pb.head), pp.c.mul(d))
                }
            };
            out.push(BonePose {
                head,
                c,
                tip: head + c.rotate(b.dir * b.length),
            });
        }
        out
    }

    /// Clamp a delta to the bone's limits. Returns the clamped rotation and whether it changed.
    pub fn clamp(&self, i: usize, d: Quat) -> (Quat, bool) {
        let b = &self.bones[i];
        let d = d.normalized();
        if let Some((axis, lo, hi)) = b.hinge {
            let axis = axis.normalized();
            let q = if d.w < 0.0 { qneg(d) } else { d };
            let ang = 2.0 * v3(q.x, q.y, q.z).dot(axis).atan2(q.w);
            let c = ang.clamp(lo, hi);
            let off = (q.x * q.x + q.y * q.y + q.z * q.z).sqrt() > 1e-9
                && (v3(q.x, q.y, q.z) - axis * v3(q.x, q.y, q.z).dot(axis)).len() > 1e-6;
            let r = Quat::from_axis_angle(axis, c);
            return (r, off || (c - ang).abs() > 1e-12);
        }
        if b.swing.is_none() && b.twist.is_none() {
            return (d, false);
        }
        let tw = twist_about(d, b.dir);
        // d = swing * twist
        let mut swing = d.mul(tw.conj()).normalized();
        let mut hit = false;
        let mut tw = tw;
        if let Some((lo, hi)) = b.twist {
            let t = if tw.w < 0.0 { qneg(tw) } else { tw };
            let ang = 2.0 * v3(t.x, t.y, t.z).dot(b.dir).atan2(t.w);
            let c = ang.clamp(lo, hi);
            if (c - ang).abs() > 1e-12 {
                hit = true;
                tw = Quat::from_axis_angle(b.dir, c);
            }
        }
        if let Some(cone) = b.swing {
            let s = if swing.w < 0.0 { qneg(swing) } else { swing };
            let v = v3(s.x, s.y, s.z);
            let sn = v.len();
            let ang = 2.0 * sn.atan2(s.w);
            if ang > cone && sn > 1e-12 {
                hit = true;
                swing = Quat::from_axis_angle(v, cone);
            }
        }
        (swing.mul(tw).normalized(), hit)
    }

    /// Solve IK for the chain `chain` (bone indices, each the parent of the next, so a limb root
    /// to its end). The end of the last bone is taken to `target`; `pole` picks the bend side.
    /// `head`/`c_parent` are the current head of the chain's first bone and its parent's cumulative
    /// rotation. Returns the delta of each bone (unclamped) and whether the target was out of reach.
    pub fn ik_chain(
        &self,
        chain: &[usize],
        head: V3,
        c_parent: Quat,
        target: V3,
        pole: V3,
    ) -> (Vec<Quat>, bool) {
        let n = chain.len();
        if n == 0 {
            return (vec![], false);
        }
        let lens: Vec<f64> = chain.iter().map(|&i| self.bones[i].length).collect();
        let total: f64 = lens.iter().sum();
        // joint positions, starting from the rest pose carried by the parent
        let mut p = vec![head];
        for (k, &i) in chain.iter().enumerate() {
            let b = &self.bones[i];
            let d = c_parent.rotate(b.dir);
            // a following bone starts where this one ends
            let last = p[k];
            p.push(last + d * lens[k]);
        }
        let to = target - head;
        let dist = to.len();
        let reach = dist > total - 1e-9;
        let tgt = if dist > total {
            head + to.normalized() * total
        } else {
            target
        };
        if n == 2 {
            let (l1, l2) = (lens[0], lens[1]);
            let dd = (tgt - head)
                .len()
                .clamp((l1 - l2).abs() + 1e-6, l1 + l2 - 1e-9);
            let dn = (tgt - head).normalized();
            let a = (l1 * l1 - l2 * l2 + dd * dd) / (2.0 * dd);
            let h = (l1 * l1 - a * a).max(0.0).sqrt();
            let pv = pole - head;
            let mut perp = pv - dn * pv.dot(dn);
            if perp.len() < 1e-9 {
                perp = dn.any_perp();
            }
            let perp = perp.normalized();
            p[1] = head + dn * a + perp * h;
            p[2] = head + dn * dd;
        } else {
            for _ in 0..24 {
                p[n] = tgt;
                for k in (0..n).rev() {
                    let d = (p[k] - p[k + 1]).normalized();
                    p[k] = p[k + 1] + d * lens[k];
                }
                p[0] = head;
                for k in 0..n {
                    let d = (p[k + 1] - p[k]).normalized();
                    p[k + 1] = p[k] + d * lens[k];
                }
                if (p[n] - tgt).len() < 1e-6 {
                    break;
                }
            }
        }
        // a hinged lower bone: twist the upper bone so the hinge axis stands across the bend plane
        if n == 2 {
            if let Some((axis, lo, hi)) = self.bones[chain[1]].hinge {
                let (ia, ib) = (chain[0], chain[1]);
                let u = (p[1] - p[0]).normalized();
                let l = (p[2] - p[1]).normalized();
                let mut nrm = u.cross(l);
                if nrm.len() > 1e-9 {
                    nrm = nrm.normalized();
                    if lo + hi < 0.0 {
                        nrm = -nrm;
                    }
                    let ca = arc(self.bones[ia].dir, u);
                    let aw = ca.rotate(axis);
                    let proj = |v: V3| {
                        let q = v - u * v.dot(u);
                        if q.len() < 1e-12 {
                            v.any_perp()
                        } else {
                            q.normalized()
                        }
                    };
                    let (x, y) = (proj(aw), proj(nrm));
                    let th = x.cross(y).dot(u).atan2(x.dot(y));
                    let ca = Quat::from_axis_angle(u, th).mul(ca).normalized();
                    let l_loc = ca.conj().rotate(l);
                    let lb0 = self.bones[ib].dir;
                    let (a0, a1) = (lb0 - axis * lb0.dot(axis), l_loc - axis * l_loc.dot(axis));
                    let phi = if a0.len() < 1e-12 || a1.len() < 1e-12 {
                        0.0
                    } else {
                        a0.cross(a1).dot(axis).atan2(a0.dot(a1))
                    };
                    let da = c_parent.conj().mul(ca).normalized();
                    return (vec![da, Quat::from_axis_angle(axis, phi)], reach);
                }
            }
        }
        let mut out = vec![];
        let mut cprev = c_parent;
        for (k, &i) in chain.iter().enumerate() {
            let nd = (p[k + 1] - p[k]).normalized();
            let ck = arc(self.bones[i].dir, nd);
            out.push(cprev.conj().mul(ck).normalized());
            cprev = ck;
        }
        (out, reach)
    }
}

/// Skin weights of rest point `p` over `bones` (all when empty): `exp(-distance / blend)` to each
/// bone's segment, normalised, weights under 0.005 dropped. Sorted by bone index.
pub fn auto_weights3(sk: &Skeleton3, p: V3, bones: &[usize], blend: f64) -> Vec<(usize, f64)> {
    let blend = blend.max(1e-9);
    let all: Vec<usize> = (0..sk.bones.len()).collect();
    let set: &[usize] = if bones.is_empty() { &all } else { bones };
    let mut w: Vec<(usize, f64)> = set
        .iter()
        .map(|&i| {
            let b = &sk.bones[i];
            let ab = b.tip() - b.head;
            let t = if ab.len2() < 1e-18 {
                0.0
            } else {
                ((p - b.head).dot(ab) / ab.len2()).clamp(0.0, 1.0)
            };
            let d = (p - (b.head + ab * t)).len();
            (i, (-d / blend).exp())
        })
        .collect();
    let sum: f64 = w.iter().map(|x| x.1).sum();
    if sum <= 0.0 {
        return vec![];
    }
    w.iter_mut().for_each(|x| x.1 /= sum);
    w.retain(|x| x.1 >= 0.005);
    let s2: f64 = w.iter().map(|x| x.1).sum();
    w.iter_mut().for_each(|x| x.1 /= s2);
    w.sort_by_key(|x| x.0);
    w
}

/// Linear blend skinning of rest point `p`.
pub fn skin_point3(sk: &Skeleton3, poses: &[BonePose], weights: &[(usize, f64)], p: V3) -> V3 {
    let mut o = V3::ZERO;
    for &(i, w) in weights {
        o += (poses[i].head + poses[i].c.rotate(p - sk.bones[i].head)) * w;
    }
    o
}
