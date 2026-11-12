//! Joints and springs. Written before the implementation.
use animscript_physics::*;

const DT: f64 = 1.0 / 96.0;

fn world() -> World {
    World::new(v2(0.0, 10.0))
}
fn anchor_body(w: &mut World, p: Vec2) -> BodyId {
    let b = w.create_body(&BodyDef {
        position: p,
        ..BodyDef::default()
    });
    w.add_fixture(b, &FixtureDef::new(Shape::boxed(0.1, 0.1)));
    b
}
fn ball(w: &mut World, p: Vec2) -> BodyId {
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: p,
        ..BodyDef::default()
    });
    w.add_fixture(
        b,
        &FixtureDef {
            density: 1.0,
            ..FixtureDef::new(Shape::Circle {
                center: v2(0.0, 0.0),
                radius: 0.05,
            })
        },
    );
    b
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn run(w: &mut World, seconds: f64) {
    for _ in 0..(seconds / DT).round() as usize {
        w.step(DT);
    }
}

#[test]
fn a_pendulum_has_the_right_period() {
    let mut w = world();
    let pivot = anchor_body(&mut w, v2(0.0, 0.0));
    let l = 1.0;
    let a0 = 5f64.to_radians();
    let bob = ball(&mut w, v2(l * a0.sin(), l * a0.cos()));
    w.create_joint(&JointDef::revolute(pivot, bob, v2(0.0, 0.0)));
    // measure the time between two upward zero crossings of the horizontal position
    let mut crossings = vec![];
    let mut prev = w.body(bob).p.x;
    for i in 0..(6.0 / DT) as usize {
        w.step(DT);
        let x = w.body(bob).p.x;
        if prev > 0.0 && x <= 0.0 {
            crossings.push(i as f64 * DT);
        }
        prev = x;
    }
    assert!(crossings.len() >= 2, "{crossings:?}");
    let period = crossings[1] - crossings[0];
    near(period, 2.0 * std::f64::consts::PI * (l / 10.0).sqrt(), 0.07);
    // the arm length stays 1
    let d = (w.body(bob).p - w.body(pivot).p).length();
    near(d, l, 0.01);
}

#[test]
fn a_distance_spring_oscillates_at_its_frequency() {
    let mut w = World::new(v2(0.0, 0.0));
    let anchor = anchor_body(&mut w, v2(0.0, 0.0));
    let m = ball(&mut w, v2(1.5, 0.0)); // stretched by 0.5 m
    w.create_joint(
        &JointDef::distance(anchor, m, v2(0.0, 0.0), v2(1.5, 0.0))
            .length(1.0)
            .spring(2.0, 0.0),
    );
    let mut crossings = vec![];
    let mut prev = w.body(m).p.x - 1.0;
    for i in 0..(3.0 / DT) as usize {
        w.step(DT);
        let x = w.body(m).p.x - 1.0;
        if prev > 0.0 && x <= 0.0 {
            crossings.push(i as f64 * DT);
        }
        prev = x;
    }
    assert!(crossings.len() >= 3, "{crossings:?}");
    near(crossings[2] - crossings[1], 0.5, 0.025); // 2 Hz
}

#[test]
fn a_damped_spring_settles_at_its_rest_length() {
    let mut w = World::new(v2(0.0, 0.0));
    let anchor = anchor_body(&mut w, v2(0.0, 0.0));
    let m = ball(&mut w, v2(2.0, 0.0));
    w.create_joint(
        &JointDef::distance(anchor, m, v2(0.0, 0.0), v2(2.0, 0.0))
            .length(1.0)
            .spring(3.0, 0.7),
    );
    run(&mut w, 4.0);
    near(w.body(m).p.x, 1.0, 0.02);
}

#[test]
fn a_rigid_distance_keeps_its_length() {
    let mut w = world();
    let a = anchor_body(&mut w, v2(0.0, 0.0));
    let b = ball(&mut w, v2(1.0, 0.0));
    w.create_joint(&JointDef::distance(a, b, v2(0.0, 0.0), v2(1.0, 0.0)));
    run(&mut w, 3.0);
    near((w.body(b).p - w.body(a).p).length(), 1.0, 0.01);
}

#[test]
fn a_rope_goes_slack_but_never_stretches() {
    let mut w = world();
    let a = anchor_body(&mut w, v2(0.0, 0.0));
    let b = ball(&mut w, v2(0.0, 0.2)); // starts with plenty of slack on a 1.0 m rope
    w.create_joint(&JointDef::rope(a, b, v2(0.0, 0.0), v2(0.0, 0.2), 1.0));
    let mut max_len: f64 = 0.0;
    let mut fell_freely = false;
    for _ in 0..(3.0 / DT) as usize {
        w.step(DT);
        let len = (w.body(b).p - w.body(a).p).length();
        max_len = max_len.max(len);
        if len > 0.5 && len < 0.9 && w.body(b).v.y > 3.0 {
            fell_freely = true;
        }
    }
    assert!(fell_freely, "the rope is slack at first");
    assert!(max_len < 1.06, "{max_len}");
}

#[test]
fn a_revolute_limit_stops_the_swing() {
    let mut w = world();
    let pivot = anchor_body(&mut w, v2(0.0, 0.0));
    let bob = ball(&mut w, v2(1.0, 0.0)); // a point mass on a 1 m arm: tiny inertia, the hard case
    w.create_joint(&JointDef::revolute(pivot, bob, v2(0.0, 0.0)).limits(-0.5, 0.5));
    let mut max_angle: f64 = 0.0;
    for _ in 0..(3.0 / DT) as usize {
        w.step(DT);
        let rel = w.body(bob).q.angle() - w.body(pivot).q.angle();
        max_angle = max_angle.max(rel.abs());
    }
    assert!(max_angle < 0.5 + 0.06, "{max_angle}");
    assert!(max_angle > 0.4, "it reached the limit: {max_angle}");
}

#[test]
fn a_revolute_motor_spins_at_its_speed() {
    let mut w = World::new(v2(0.0, 0.0));
    let hub = anchor_body(&mut w, v2(0.0, 0.0));
    let wheel = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(wheel, &FixtureDef::new(Shape::boxed(0.5, 0.1)));
    w.create_joint(&JointDef::revolute(hub, wheel, v2(0.0, 0.0)).motor(2.0, 100.0));
    run(&mut w, 1.0);
    near(w.body(wheel).w, 2.0, 0.05);
}

#[test]
fn a_prismatic_joint_slides_along_its_axis_within_limits() {
    let mut w = World::new(v2(0.0, 0.0));
    let rail = anchor_body(&mut w, v2(0.0, 0.0));
    let slider = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 0.0),
        linear_velocity: v2(3.0, 1.0),
        ..BodyDef::default()
    });
    w.add_fixture(slider, &FixtureDef::new(Shape::boxed(0.2, 0.2)));
    w.create_joint(&JointDef::prismatic(rail, slider, v2(0.0, 0.0), v2(1.0, 0.0)).limits(0.0, 2.0));
    run(&mut w, 2.0);
    let p = w.body(slider).p;
    near(p.x, 2.0, 0.05); // stopped at the upper limit
    near(p.y, 0.0, 0.03); // the sideways speed was removed
    near(w.body(slider).q.angle(), 0.0, 0.02);
}

#[test]
fn a_weld_keeps_two_bodies_rigid() {
    let mut w = world();
    let a = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(a, &FixtureDef::new(Shape::boxed(0.3, 0.3)));
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(1.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(b, &FixtureDef::new(Shape::boxed(0.3, 0.3)));
    let pin = anchor_body(&mut w, v2(-1.0, 0.0));
    w.create_joint(&JointDef::weld(a, b, v2(0.5, 0.0)));
    w.create_joint(&JointDef::revolute(pin, a, v2(-1.0, 0.0)));
    run(&mut w, 2.0);
    let (pa, pb) = (w.body(a), w.body(b));
    near((pb.p - pa.p).length(), 1.0, 0.03);
    near(pb.q.angle() - pa.q.angle(), 0.0, 0.03);
    assert!(pa.q.angle().abs() > 0.3, "the pair swung");
}

#[test]
fn a_target_joint_pulls_a_body_to_a_point() {
    let mut w = World::new(v2(0.0, 0.0));
    let b = ball(&mut w, v2(0.0, 0.0));
    let j = w.create_joint(
        &JointDef::target(b, v2(0.0, 0.0), v2(3.0, 2.0))
            .force(50.0)
            .spring(4.0, 1.0),
    );
    run(&mut w, 3.0);
    near(w.body(b).p.x, 3.0, 0.1);
    near(w.body(b).p.y, 2.0, 0.1);
    w.set_joint_target(j, v2(-1.0, 0.0));
    run(&mut w, 3.0);
    near(w.body(b).p.x, -1.0, 0.1);
}

#[test]
fn joints_between_bodies_can_disable_their_collision() {
    let mut w = World::new(v2(0.0, 0.0));
    let a = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(a, &FixtureDef::new(Shape::boxed(0.5, 0.5)));
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.5, 0.0),
        ..BodyDef::default()
    }); // overlapping
    w.add_fixture(b, &FixtureDef::new(Shape::boxed(0.5, 0.5)));
    w.create_joint(&JointDef::rope(a, b, v2(0.0, 0.0), v2(0.5, 0.0), 2.0));
    run(&mut w, 1.0);
    near((w.body(b).p - w.body(a).p).length(), 0.5, 0.05); // not pushed apart
    let mut w2 = World::new(v2(0.0, 0.0));
    let a = w2.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 0.0),
        ..BodyDef::default()
    });
    w2.add_fixture(a, &FixtureDef::new(Shape::boxed(0.5, 0.5)));
    let b = w2.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.5, 0.0),
        ..BodyDef::default()
    });
    w2.add_fixture(b, &FixtureDef::new(Shape::boxed(0.5, 0.5)));
    w2.create_joint(&JointDef::rope(a, b, v2(0.0, 0.0), v2(0.5, 0.0), 2.0).collide_connected(true));
    run(&mut w2, 1.0);
    assert!(
        (w2.body(b).p - w2.body(a).p).length() > 0.9,
        "pushed apart when they may collide"
    );
}

#[test]
fn joint_worlds_are_deterministic() {
    let build = || {
        let mut w = world();
        let mut prev = anchor_body(&mut w, v2(0.0, 0.0));
        for i in 1..=6 {
            let b = ball(&mut w, v2(i as f64 * 0.3, 0.0));
            w.create_joint(&JointDef::revolute(
                prev,
                b,
                v2((i as f64 - 0.5) * 0.3, 0.0),
            ));
            prev = b;
        }
        w
    };
    let (mut a, mut b) = (build(), build());
    run(&mut a, 3.0);
    run(&mut b, 3.0);
    for id in 0..a.body_count() {
        assert_eq!(a.body(id).p.x.to_bits(), b.body(id).p.x.to_bits());
        assert_eq!(a.body(id).p.y.to_bits(), b.body(id).p.y.to_bits());
    }
}
