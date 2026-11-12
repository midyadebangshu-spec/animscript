//! Skeleton maths: forward kinematics, limits, two-bone and FABRIK inverse kinematics, skinning.
//! Written first. Angles are radians, clockwise on screen (y points down): angle 0 is +x.
use animscript_physics::*;
use std::f64::consts::{FRAC_PI_2, PI};

fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn near_v(a: Vec2, b: Vec2, tol: f64) {
    assert!((a - b).length() <= tol, "{a:?} vs {b:?} (tolerance {tol})");
}
fn bone(parent: Option<usize>, length: f64, rest_angle: f64) -> BoneDef {
    BoneDef {
        parent,
        at: 1.0,
        length,
        rest_angle,
        head: Vec2::ZERO,
        limits: None,
    }
}
fn arm() -> Skeleton {
    // an upper arm 1 m long and a forearm 1 m long, along +x at rest, rooted at (2, 3)
    let mut a = bone(None, 1.0, 0.0);
    a.head = v2(2.0, 3.0);
    Skeleton::new(vec![a, bone(Some(0), 1.0, 0.0)])
}

#[test]
fn forward_kinematics_walks_the_chain() {
    let sk = arm();
    let rest = sk.pose(&[0.0, 0.0], Vec2::ZERO);
    near_v(rest[0].head, v2(2.0, 3.0), 1e-12);
    near_v(sk.tip(&rest, 0), v2(3.0, 3.0), 1e-12);
    near_v(rest[1].head, v2(3.0, 3.0), 1e-12);
    near_v(sk.tip(&rest, 1), v2(4.0, 3.0), 1e-12);
    // the elbow bends a quarter turn clockwise (towards +y), the shoulder stays
    let bent = sk.pose(&[0.0, FRAC_PI_2], Vec2::ZERO);
    near_v(sk.tip(&bent, 1), v2(3.0, 4.0), 1e-12);
    // rotating the shoulder carries the forearm
    let up = sk.pose(&[-FRAC_PI_2, 0.0], Vec2::ZERO);
    near_v(sk.tip(&up, 0), v2(2.0, 2.0), 1e-12);
    near_v(sk.tip(&up, 1), v2(2.0, 1.0), 1e-12);
    near(up[1].angle, -FRAC_PI_2, 1e-12);
    // a root offset moves everything
    let moved = sk.pose(&[0.0, 0.0], v2(10.0, 0.0));
    near_v(sk.tip(&moved, 1), v2(14.0, 3.0), 1e-12);
}

#[test]
fn a_child_can_hang_off_the_middle_of_its_parent_with_a_rest_angle() {
    let mut a = bone(None, 2.0, 0.0);
    a.head = Vec2::ZERO;
    let mut b = bone(Some(0), 1.0, FRAC_PI_2); // sticks out sideways (clockwise) from the middle
    b.at = 0.5;
    let sk = Skeleton::new(vec![a, b]);
    let rest = sk.pose(&[0.0, 0.0], Vec2::ZERO);
    near_v(rest[1].head, v2(1.0, 0.0), 1e-12);
    near_v(sk.tip(&rest, 1), v2(1.0, 1.0), 1e-12);
}

#[test]
fn limits_clamp_the_pose_rotation() {
    let mut b = bone(Some(0), 1.0, 0.0);
    b.limits = Some((-0.5, 1.0));
    let mut sk = arm();
    sk.bones[1] = b;
    let mut rots = [0.0, 2.0];
    assert!(sk.clamp_limits(&mut rots)); // true when something was clamped
    near(rots[1], 1.0, 1e-12);
    let mut rots = [0.0, -3.0];
    sk.clamp_limits(&mut rots);
    near(rots[1], -0.5, 1e-12);
    let mut rots = [0.0, 0.3];
    assert!(!sk.clamp_limits(&mut rots));
}

#[test]
fn two_bone_ik_reaches_any_reachable_point_exactly() {
    let sk = arm();
    for target in [
        v2(3.5, 2.0),
        v2(2.5, 4.0),
        v2(1.0, 3.0),
        v2(2.0, 1.2),
        v2(3.9, 3.0),
    ] {
        let mut rots = [0.0, 0.0];
        solve_ik(
            &sk,
            &mut rots,
            Vec2::ZERO,
            &[0, 1],
            target,
            Bend::Positive,
            20,
        );
        let pose = sk.pose(&rots, Vec2::ZERO);
        near_v(sk.tip(&pose, 1), target, 1e-9);
        // bone lengths are untouched by construction; the elbow is where the first bone ends
        near_v(pose[1].head, sk.tip(&pose, 0), 1e-9);
    }
}

#[test]
fn two_bone_ik_stretches_towards_an_unreachable_target_and_picks_the_bend_side() {
    let sk = arm();
    let mut rots = [0.0, 0.0];
    solve_ik(
        &sk,
        &mut rots,
        Vec2::ZERO,
        &[0, 1],
        v2(12.0, 3.0),
        Bend::Positive,
        20,
    );
    let pose = sk.pose(&rots, Vec2::ZERO);
    near_v(sk.tip(&pose, 1), v2(4.0, 3.0), 1e-9); // fully stretched along the direction to the target
                                                  // reaching a nearby point: the two bend sides put the elbow on opposite sides of the line
    let (mut a, mut b) = ([0.0, 0.0], [0.0, 0.0]);
    solve_ik(
        &sk,
        &mut a,
        Vec2::ZERO,
        &[0, 1],
        v2(3.0, 3.0),
        Bend::Positive,
        20,
    );
    solve_ik(
        &sk,
        &mut b,
        Vec2::ZERO,
        &[0, 1],
        v2(3.0, 3.0),
        Bend::Negative,
        20,
    );
    let (pa, pb) = (sk.pose(&a, Vec2::ZERO), sk.pose(&b, Vec2::ZERO));
    assert!(
        pa[1].head.y > 3.0 && pb[1].head.y < 3.0,
        "{:?} {:?}",
        pa[1].head,
        pb[1].head
    );
    near_v(sk.tip(&pa, 1), v2(3.0, 3.0), 1e-9);
    near_v(sk.tip(&pb, 1), v2(3.0, 3.0), 1e-9);
}

#[test]
fn fabrik_reaches_with_a_long_chain_and_keeps_the_lengths() {
    let mut bones = vec![{
        let mut b = bone(None, 0.5, 0.0);
        b.head = v2(0.0, 0.0);
        b
    }];
    for i in 1..5 {
        bones.push(bone(Some(i - 1), 0.5, 0.0));
    }
    let sk = Skeleton::new(bones);
    let target = v2(0.8, 1.1);
    let mut rots = [0.0; 5];
    let ok = solve_ik(
        &sk,
        &mut rots,
        Vec2::ZERO,
        &[0, 1, 2, 3, 4],
        target,
        Bend::Positive,
        40,
    );
    assert!(ok);
    let pose = sk.pose(&rots, Vec2::ZERO);
    near_v(sk.tip(&pose, 4), target, 0.01);
    for i in 1..5 {
        near_v(pose[i].head, sk.tip(&pose, i - 1), 1e-9);
    }
    // an unreachable target leaves the chain straight and pointing at it
    let mut far = [0.0; 5];
    let ok = solve_ik(
        &sk,
        &mut far,
        Vec2::ZERO,
        &[0, 1, 2, 3, 4],
        v2(0.0, 9.0),
        Bend::Positive,
        40,
    );
    assert!(!ok);
    let pose = sk.pose(&far, Vec2::ZERO);
    near_v(sk.tip(&pose, 4), v2(0.0, 2.5), 0.01);
}

#[test]
fn ik_respects_joint_limits() {
    let mut bones = vec![{
        let mut b = bone(None, 1.0, 0.0);
        b.head = Vec2::ZERO;
        b
    }];
    let mut elbow = bone(Some(0), 1.0, 0.0);
    elbow.limits = Some((0.0, PI)); // the forearm can only fold clockwise
    bones.push(elbow);
    let sk = Skeleton::new(bones);
    let mut rots = [0.0, 0.0];
    // reaching it on this bend side needs the forearm to fold counter-clockwise (rotation -90 deg): the limit clamps it
    solve_ik(
        &sk,
        &mut rots,
        Vec2::ZERO,
        &[0, 1],
        v2(1.0, -1.0),
        Bend::Positive,
        20,
    );
    assert!(
        rots[1] >= -1e-12 && rots[1] <= PI + 1e-12,
        "the limit held: {}",
        rots[1]
    );
}

#[test]
fn skinning_follows_a_bone_rigidly_and_blends_between_two() {
    let sk = arm();
    let rest = sk.pose(&[0.0, 0.0], Vec2::ZERO);
    let bent = sk.pose(&[0.0, FRAC_PI_2], Vec2::ZERO);
    let p = v2(3.5, 3.0); // on the forearm
    near_v(
        skin_point(&sk, &rest, &bent, &[(1, 1.0)], p),
        v2(3.0, 3.5),
        1e-12,
    );
    near_v(skin_point(&sk, &rest, &bent, &[(0, 1.0)], p), p, 1e-12); // the upper arm did not move
    let mid = skin_point(&sk, &rest, &bent, &[(0, 0.5), (1, 0.5)], p);
    near_v(mid, (p + v2(3.0, 3.5)) * 0.5, 1e-12);
}

#[test]
fn automatic_weights_favour_the_nearest_bone_and_sum_to_one() {
    let sk = arm();
    let rest = sk.pose(&[0.0, 0.0], Vec2::ZERO);
    let pts = [v2(2.3, 3.2), v2(3.0, 3.1), v2(3.8, 2.9)];
    let w = auto_weights(&pts, &sk, &rest, &[0, 1], 0.2);
    for ws in &w {
        near(ws.iter().map(|x| x.1).sum::<f64>(), 1.0, 1e-9);
    }
    let weight = |i: usize, b: usize| w[i].iter().find(|x| x.0 == b).map_or(0.0, |x| x.1);
    assert!(weight(0, 0) > 0.9, "near the shoulder side: {:?}", w[0]);
    assert!(weight(2, 1) > 0.9, "near the hand: {:?}", w[2]);
    near(weight(1, 0), 0.5, 0.15); // at the elbow they share
}

#[test]
fn the_solvers_are_deterministic() {
    let sk = arm();
    let run = || {
        let mut r = [0.0, 0.0];
        solve_ik(
            &sk,
            &mut r,
            Vec2::ZERO,
            &[0, 1],
            v2(3.1, 2.4),
            Bend::Positive,
            20,
        );
        (r[0].to_bits(), r[1].to_bits())
    };
    assert_eq!(run(), run());
}
