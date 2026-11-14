use animscript_physics3::*;
use std::f64::consts::{FRAC_PI_2, PI};

/// shoulder -> elbow -> wrist, 1 m each, hanging along +x.
fn arm() -> Skeleton3 {
    let x = v3(1.0, 0.0, 0.0);
    let mut upper = BoneDef3::new(None, V3::ZERO, x, 1.0);
    upper.swing = Some(2.0);
    let mut lower = BoneDef3::new(Some(0), v3(1.0, 0.0, 0.0), x, 1.0);
    lower.hinge = Some((v3(0.0, 0.0, 1.0), 0.0, 2.5));
    Skeleton3::new(vec![upper, lower])
}

fn near(a: V3, b: V3, e: f64) -> bool {
    (a - b).len() < e
}

#[test]
fn forward_kinematics_matches_a_hand_computation() {
    let sk = arm();
    let z = v3(0.0, 0.0, 1.0);
    // the elbow bends 90 degrees about z: the forearm points along +y
    let p = sk.pose(
        V3::ZERO,
        &[Quat::IDENTITY, Quat::from_axis_angle(z, FRAC_PI_2)],
    );
    assert!(near(p[1].head, v3(1.0, 0.0, 0.0), 1e-12));
    assert!(near(p[1].tip, v3(1.0, 1.0, 0.0), 1e-12));
    // rotating the shoulder 90 degrees about z carries the elbow to (0,1,0) and the forearm on to -x
    let p = sk.pose(
        v3(0.0, 0.0, 0.0),
        &[
            Quat::from_axis_angle(z, FRAC_PI_2),
            Quat::from_axis_angle(z, FRAC_PI_2),
        ],
    );
    assert!(near(p[1].head, v3(0.0, 1.0, 0.0), 1e-12));
    assert!(near(p[1].tip, v3(-1.0, 1.0, 0.0), 1e-12));
    // the root offset moves everything
    let p = sk.pose(v3(5.0, 0.0, 0.0), &[]);
    assert!(near(p[1].tip, v3(7.0, 0.0, 0.0), 1e-12));
}

#[test]
fn hinges_and_cones_clamp_and_say_so() {
    let sk = arm();
    let z = v3(0.0, 0.0, 1.0);
    let (q, hit) = sk.clamp(1, Quat::from_axis_angle(z, -0.5));
    assert!(hit);
    assert!(
        q.rotate(v3(1.0, 0.0, 0.0)).dot(v3(1.0, 0.0, 0.0)) > 0.999,
        "hyperextension clamped to straight"
    );
    let (q, hit) = sk.clamp(1, Quat::from_axis_angle(z, 1.0));
    assert!(!hit);
    assert!((q.rotate(v3(1.0, 0.0, 0.0)).y - 1.0f64.sin()).abs() < 1e-9);
    // sideways rotation of a hinge is dropped
    let (q, _) = sk.clamp(1, Quat::from_axis_angle(v3(0.0, 1.0, 0.0), 1.0));
    assert!((q.rotate(v3(1.0, 0.0, 0.0)) - v3(1.0, 0.0, 0.0)).len() < 1e-9);
    // a cone of 2 rad lets 1 through and holds 3 to 2
    let (_, hit) = sk.clamp(0, Quat::from_axis_angle(z, 1.0));
    assert!(!hit);
    let (q, hit) = sk.clamp(0, Quat::from_axis_angle(z, 3.0));
    assert!(hit);
    let ang = q.rotate(v3(1.0, 0.0, 0.0)).dot(v3(1.0, 0.0, 0.0)).acos();
    assert!((ang - 2.0).abs() < 1e-6, "{ang}");
}

#[test]
fn two_bone_ik_reaches_and_the_pole_picks_the_bend() {
    let sk = arm();
    let target = v3(1.2, 0.8, 0.0);
    for pole in [v3(0.0, -5.0, 0.0), v3(0.0, 5.0, 0.0)] {
        let (d, reach) = sk.ik_chain(&[0, 1], V3::ZERO, Quat::IDENTITY, target, pole);
        assert!(!reach);
        let p = sk.pose(V3::ZERO, &d);
        assert!(near(p[1].tip, target, 1e-6), "{:?}", p[1].tip);
        let side = (p[1].head.y - 0.0).signum();
        assert_eq!(side, pole.y.signum(), "elbow on the pole's side");
    }
}

#[test]
fn ik_past_reach_straightens_toward_the_target() {
    let sk = arm();
    let (d, reach) = sk.ik_chain(
        &[0, 1],
        V3::ZERO,
        Quat::IDENTITY,
        v3(0.0, 5.0, 0.0),
        v3(1.0, 0.0, 0.0),
    );
    assert!(reach);
    let p = sk.pose(V3::ZERO, &d);
    assert!(near(p[1].tip, v3(0.0, 2.0, 0.0), 1e-6));
}

#[test]
fn fabrik_handles_a_three_bone_chain() {
    let x = v3(1.0, 0.0, 0.0);
    let bones = vec![
        BoneDef3::new(None, V3::ZERO, x, 1.0),
        BoneDef3::new(Some(0), v3(1.0, 0.0, 0.0), x, 1.0),
        BoneDef3::new(Some(1), v3(2.0, 0.0, 0.0), x, 1.0),
    ];
    let sk = Skeleton3::new(bones);
    let t = v3(1.5, 1.5, 0.7);
    let (d, _) = sk.ik_chain(&[0, 1, 2], V3::ZERO, Quat::IDENTITY, t, V3::ZERO);
    let p = sk.pose(V3::ZERO, &d);
    assert!(near(p[2].tip, t, 1e-4), "{:?}", p[2].tip);
}

#[test]
fn skin_weights_sum_to_one_and_follow_the_nearest_bone() {
    let sk = arm();
    let w = auto_weights3(&sk, v3(0.5, 0.1, 0.0), &[], 0.2);
    let s: f64 = w.iter().map(|x| x.1).sum();
    assert!((s - 1.0).abs() < 1e-9);
    assert_eq!(
        w.iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap()
            .0,
        0
    );
    let w = auto_weights3(&sk, v3(1.9, 0.0, 0.0), &[1], 0.2);
    assert_eq!(w, vec![(1, 1.0)]);
}

#[test]
fn a_bent_elbow_skins_the_forearm_and_blends_at_the_joint() {
    let sk = arm();
    let z = v3(0.0, 0.0, 1.0);
    let p = sk.pose(
        V3::ZERO,
        &[Quat::IDENTITY, Quat::from_axis_angle(z, FRAC_PI_2)],
    );
    let far = v3(1.8, 0.0, 0.0);
    let wf = auto_weights3(&sk, far, &[], 0.1);
    assert!(near(
        skin_point3(&sk, &p, &wf, far),
        v3(1.0, 0.8, 0.0),
        0.02
    ));
    // a vertex at the elbow, half and half, sits between the two rigid positions
    let at = v3(1.0, 0.05, 0.0);
    let wj = [(0, 0.5), (1, 0.5)];
    let s = skin_point3(&sk, &p, &wj, at);
    assert!(
        s.len().is_finite() && (s - v3(1.0, 0.025, 0.0)).len() < 0.1,
        "{s:?}"
    );
    let _ = PI;
}

#[test]
fn slerp_and_arc_are_exact_at_their_ends() {
    let a = arc(v3(1.0, 0.0, 0.0), v3(0.0, 1.0, 0.0));
    assert!(near(a.rotate(v3(1.0, 0.0, 0.0)), v3(0.0, 1.0, 0.0), 1e-12));
    let b = arc(v3(1.0, 0.0, 0.0), v3(-1.0, 0.0, 0.0));
    assert!(near(b.rotate(v3(1.0, 0.0, 0.0)), v3(-1.0, 0.0, 0.0), 1e-12));
    let h = slerp(Quat::IDENTITY, a, 0.5);
    assert!(near(
        h.rotate(v3(1.0, 0.0, 0.0)),
        v3(0.5f64.sqrt(), 0.5f64.sqrt(), 0.0),
        1e-9
    ));
}
