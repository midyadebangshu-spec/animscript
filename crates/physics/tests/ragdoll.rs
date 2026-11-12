//! Engine additions for sub-round B: ray casts, switching a body's kind, per-pair collision
//! filtering and the revolute angle spring (an active ragdoll's muscle). Written first.
use animscript_physics::*;

const DT: f64 = 1.0 / 96.0;

fn world() -> World {
    World::new(v2(0.0, 10.0))
}
fn body(w: &mut World, kind: BodyKind, p: Vec2, shape: Shape) -> BodyId {
    let b = w.create_body(&BodyDef {
        kind,
        position: p,
        ..BodyDef::default()
    });
    w.add_fixture(b, &FixtureDef::new(shape));
    b
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}

// ---- raycast ---------------------------------------------------------------------------------

#[test]
fn a_ray_hits_a_box_on_its_near_face() {
    let mut w = world();
    let b = body(
        &mut w,
        BodyKind::Static,
        v2(5.0, 0.0),
        Shape::boxed(1.0, 1.0),
    );
    let h = w
        .raycast(v2(0.0, 0.0), v2(1.0, 0.0), 20.0, None)
        .expect("hit");
    assert_eq!(h.body, b);
    near(h.distance, 4.0, 1e-9);
    near(h.point.x, 4.0, 1e-9);
    near(h.normal.x, -1.0, 1e-9);
}
#[test]
fn a_ray_hits_a_circle() {
    let mut w = world();
    body(
        &mut w,
        BodyKind::Static,
        v2(0.0, 5.0),
        Shape::Circle {
            center: Vec2::ZERO,
            radius: 1.0,
        },
    );
    let h = w
        .raycast(v2(0.0, 0.0), v2(0.0, 1.0), 20.0, None)
        .expect("hit");
    near(h.distance, 4.0, 1e-9);
    near(h.normal.y, -1.0, 1e-9);
}
#[test]
fn a_ray_respects_rotation_and_edges() {
    let mut w = world();
    let b = w.create_body(&BodyDef {
        position: v2(0.0, 5.0),
        angle: std::f64::consts::FRAC_PI_4,
        ..BodyDef::default()
    });
    w.add_fixture(b, &FixtureDef::new(Shape::boxed(1.0, 1.0)));
    let h = w
        .raycast(v2(0.0, 0.0), v2(0.0, 1.0), 20.0, None)
        .expect("hit");
    near(h.distance, 5.0 - 2f64.sqrt(), 1e-9); // the corner of the diamond
    let mut w = world();
    body(
        &mut w,
        BodyKind::Static,
        Vec2::ZERO,
        Shape::edge(v2(-5.0, 3.0), v2(5.0, 3.0)),
    );
    let h = w
        .raycast(v2(0.0, 0.0), v2(0.0, 1.0), 20.0, None)
        .expect("hit");
    near(h.distance, 3.0, 1e-9);
}
#[test]
fn a_ray_returns_the_nearest_hit_and_can_miss_or_stop_short() {
    let mut w = world();
    body(
        &mut w,
        BodyKind::Static,
        v2(10.0, 0.0),
        Shape::boxed(1.0, 1.0),
    );
    let near_box = body(
        &mut w,
        BodyKind::Static,
        v2(5.0, 0.0),
        Shape::boxed(1.0, 1.0),
    );
    assert_eq!(
        w.raycast(v2(0.0, 0.0), v2(1.0, 0.0), 20.0, None)
            .unwrap()
            .body,
        near_box
    );
    assert!(w.raycast(v2(0.0, 0.0), v2(0.0, 1.0), 20.0, None).is_none());
    assert!(
        w.raycast(v2(0.0, 0.0), v2(1.0, 0.0), 3.0, None).is_none(),
        "too short"
    );
    assert!(
        w.raycast(v2(0.0, 0.0), v2(1.0, 0.0), 20.0, Some(near_box))
            .is_some(),
        "the far box"
    );
    let far = w
        .raycast(v2(0.0, 0.0), v2(1.0, 0.0), 20.0, Some(near_box))
        .unwrap();
    near(far.distance, 9.0, 1e-9);
}
#[test]
fn a_ray_ignores_sensors() {
    let mut w = world();
    let b = w.create_body(&BodyDef {
        position: v2(5.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(
        b,
        &FixtureDef {
            sensor: true,
            ..FixtureDef::new(Shape::boxed(1.0, 1.0))
        },
    );
    assert!(w.raycast(v2(0.0, 0.0), v2(1.0, 0.0), 20.0, None).is_none());
}

// ---- body kind -------------------------------------------------------------------------------

#[test]
fn a_kinematic_body_can_become_dynamic_and_fall() {
    let mut w = world();
    let b = body(
        &mut w,
        BodyKind::Kinematic,
        v2(0.0, 0.0),
        Shape::boxed(0.5, 0.5),
    );
    for _ in 0..48 {
        w.step(DT);
    }
    near(w.body(b).p.y, 0.0, 1e-9);
    w.set_body_kind(b, BodyKind::Dynamic);
    assert!(w.body(b).mass > 0.0);
    for _ in 0..48 {
        w.step(DT);
    }
    assert!(w.body(b).p.y > 1.0, "now falls: {}", w.body(b).p.y);
    w.set_body_kind(b, BodyKind::Kinematic);
    assert_eq!(w.body(b).inv_mass, 0.0);
}
#[test]
fn switching_kind_keeps_pose_and_velocity() {
    let mut w = world();
    let b = body(
        &mut w,
        BodyKind::Kinematic,
        v2(2.0, 3.0),
        Shape::boxed(0.5, 0.5),
    );
    w.set_linear_velocity(b, v2(4.0, 0.0));
    w.set_body_kind(b, BodyKind::Dynamic);
    assert_eq!(w.body(b).p, v2(2.0, 3.0));
    near(w.body(b).v.x, 4.0, 1e-12);
}

// ---- collision filtering ---------------------------------------------------------------------

#[test]
fn disabled_pairs_do_not_collide() {
    let build = |disable: bool| {
        let mut w = world();
        let floor = body(
            &mut w,
            BodyKind::Static,
            v2(0.0, 5.0),
            Shape::boxed(10.0, 0.5),
        );
        let a = body(
            &mut w,
            BodyKind::Dynamic,
            v2(0.0, 3.0),
            Shape::boxed(0.5, 0.5),
        );
        if disable {
            w.disable_collision(a, floor);
        }
        for _ in 0..192 {
            w.step(DT);
        }
        w.body(a).p.y
    };
    assert!(build(false) < 4.1, "rests on the floor");
    assert!(build(true) > 8.0, "falls through the floor");
}

// ---- the angle spring ------------------------------------------------------------------------

/// A pendulum bar whose pivot (on a fixed anchor) has an angle spring aimed at `target` radians.
fn arm(target: f64, hz: f64, torque: f64, gravity: f64) -> (World, BodyId, JointId) {
    let mut w = World::new(v2(0.0, gravity));
    let anchor = body(
        &mut w,
        BodyKind::Static,
        v2(0.0, 0.0),
        Shape::boxed(0.1, 0.1),
    );
    let bar = body(
        &mut w,
        BodyKind::Dynamic,
        v2(1.0, 0.0),
        Shape::boxed(1.0, 0.1),
    );
    let j = w
        .create_joint(&JointDef::revolute(anchor, bar, v2(0.0, 0.0)).angle_spring(hz, 0.7, torque));
    w.set_joint_angle_target(j, target);
    (w, bar, j)
}
fn angle(w: &World, b: BodyId) -> f64 {
    w.body(b).q.angle()
}

#[test]
fn the_spring_drives_the_joint_to_its_target() {
    let (mut w, bar, _) = arm(0.8, 6.0, 1e6, 0.0);
    for _ in 0..384 {
        w.step(DT);
    }
    near(angle(&w, bar), 0.8, 0.02);
}
#[test]
fn a_weak_spring_sags_under_gravity_and_a_strong_one_holds() {
    let hold = |torque: f64| {
        let (mut w, bar, _) = arm(0.0, 8.0, torque, 10.0);
        for _ in 0..384 {
            w.step(DT);
        }
        angle(&w, bar)
    };
    let weak = hold(0.5);
    let strong = hold(1e6);
    assert!(weak > 1.0, "a weak muscle lets the arm fall: {weak}");
    assert!(strong.abs() < 0.1, "a strong muscle holds it up: {strong}");
}
#[test]
fn the_target_can_change_and_the_arm_follows() {
    let (mut w, bar, j) = arm(0.0, 8.0, 1e6, 0.0);
    for _ in 0..192 {
        w.step(DT);
    }
    near(angle(&w, bar), 0.0, 0.02);
    w.set_joint_angle_target(j, -0.6);
    for _ in 0..288 {
        w.step(DT);
    }
    near(angle(&w, bar), -0.6, 0.02);
}
#[test]
fn without_a_spring_the_joint_is_free() {
    let mut w = World::new(v2(0.0, 10.0));
    let anchor = body(
        &mut w,
        BodyKind::Static,
        v2(0.0, 0.0),
        Shape::boxed(0.1, 0.1),
    );
    let bar = body(
        &mut w,
        BodyKind::Dynamic,
        v2(1.0, 0.0),
        Shape::boxed(1.0, 0.1),
    );
    w.create_joint(&JointDef::revolute(anchor, bar, v2(0.0, 0.0)));
    for _ in 0..192 {
        w.step(DT);
    }
    assert!(angle(&w, bar).abs() > 0.5, "swings down freely");
}

#[test]
fn the_angle_spring_can_be_switched_on_and_off_at_runtime() {
    let mut w = World::new(v2(0.0, 10.0));
    let anchor = body(
        &mut w,
        BodyKind::Static,
        v2(0.0, 0.0),
        Shape::boxed(0.1, 0.1),
    );
    let bar = body(
        &mut w,
        BodyKind::Dynamic,
        v2(1.0, 0.0),
        Shape::boxed(1.0, 0.1),
    );
    let j = w.create_joint(&JointDef::revolute(anchor, bar, v2(0.0, 0.0)));
    w.set_joint_angle_spring(j, Some((8.0, 0.7, 1e6)));
    w.set_joint_angle_target(j, 0.0);
    for _ in 0..192 {
        w.step(DT);
    }
    assert!(angle(&w, bar).abs() < 0.1, "held while on");
    w.set_joint_angle_spring(j, None);
    for _ in 0..192 {
        w.step(DT);
    }
    assert!(angle(&w, bar) > 0.5, "falls once off: {}", angle(&w, bar));
}
