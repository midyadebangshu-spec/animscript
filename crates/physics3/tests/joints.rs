use animscript_physics3::*;

fn ball(w: &mut World, p: V3, kind: BodyKind) -> BodyId {
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.kind = kind;
    d.position = p;
    w.create_body(&d)
}

fn run(w: &mut World, seconds: f64) {
    for _ in 0..(seconds * 240.0) as usize {
        w.step(1.0 / 240.0);
    }
}

fn joint(a: BodyId, b: BodyId, kind: JointKind, anchor: V3, axis: V3) -> JointDef {
    JointDef {
        a,
        b,
        kind,
        anchor_a: anchor,
        anchor_b: None,
        axis,
        collide_connected: false,
    }
}

#[test]
fn ball_joint_swings_like_a_pendulum() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let pivot = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let bob = ball(&mut w, v3(1.0, 0.0, 0.0), BodyKind::Dynamic);
    w.create_joint(&joint(
        pivot,
        bob,
        JointKind::Ball,
        v3(0.0, 0.0, 0.0),
        V3::ZERO,
    ));
    let mut max_err: f64 = 0.0;
    for _ in 0..(240 * 3) {
        w.step(1.0 / 240.0);
        max_err = max_err.max((w.body(bob).p.len() - 1.0).abs());
    }
    assert!(max_err < 0.02, "the rod stretched by {max_err}");
    // it has swung down from the horizontal start
    assert!(w.body(bob).p.y > 0.2 || w.body(bob).v.len() > 1.0);
}

#[test]
fn ball_joint_keeps_two_dynamic_bodies_together() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let top = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let a = ball(&mut w, v3(0.0, 0.3, 0.0), BodyKind::Dynamic);
    let b = ball(&mut w, v3(0.0, 0.6, 0.0), BodyKind::Dynamic);
    w.create_joint(&joint(top, a, JointKind::Ball, v3(0.0, 0.0, 0.0), V3::ZERO));
    w.create_joint(&joint(a, b, JointKind::Ball, v3(0.0, 0.3, 0.0), V3::ZERO));
    w.set_linear_velocity(b, v3(1.0, 0.0, 0.5));
    run(&mut w, 2.0);
    assert!(((w.body(a).p).len() - 0.3).abs() < 0.02);
    assert!(((w.body(b).p - w.body(a).p).len() - 0.3).abs() < 0.03);
}

#[test]
fn hinge_only_turns_about_its_axis() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let wall = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.position = v3(0.5, 0.0, 0.0);
    let door = w.create_body(&d);
    // axis along z: the door swings in the xy plane only
    w.create_joint(&joint(
        wall,
        door,
        JointKind::Hinge {
            limits: None,
            motor: None,
        },
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.0, 1.0),
    ));
    w.set_linear_velocity(door, v3(0.0, 0.0, 2.0));
    w.set_angular_velocity(door, v3(1.0, 1.0, 0.0));
    run(&mut w, 1.5);
    let b = w.body(door);
    assert!(b.p.z.abs() < 0.03, "left the hinge plane: z = {}", b.p.z);
    assert!((b.p.len() - 0.5).abs() < 0.03);
    assert!(b.p.y > 0.1, "it should have fallen under gravity");
}

#[test]
fn hinge_limits_stop_the_swing() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let wall = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.position = v3(0.5, 0.0, 0.0);
    let door = w.create_body(&d);
    let j = w.create_joint(&joint(
        wall,
        door,
        JointKind::Hinge {
            limits: Some((-0.5, 0.5)),
            motor: None,
        },
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.0, 1.0),
    ));
    run(&mut w, 2.0);
    let a = w.joint_angle(j);
    assert!(a.abs() <= 0.5 + 0.06, "angle {a}");
    assert!(a.abs() > 0.4, "it should rest against the limit, angle {a}");
}

#[test]
fn hinge_motor_reaches_its_speed() {
    let mut w = World::new(v3(0.0, 0.0, 0.0));
    let wall = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.position = v3(0.5, 0.0, 0.0);
    let door = w.create_body(&d);
    w.create_joint(&joint(
        wall,
        door,
        JointKind::Hinge {
            limits: None,
            motor: Some((2.0, 100.0)),
        },
        v3(0.0, 0.0, 0.0),
        v3(0.0, 0.0, 1.0),
    ));
    run(&mut w, 1.0);
    let wz = w.body(door).w.z;
    assert!((wz - 2.0).abs() < 0.1, "w.z = {wz}");
}

#[test]
fn fixed_joint_holds_a_relative_pose() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let top = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let a = ball(&mut w, v3(0.4, 0.0, 0.0), BodyKind::Dynamic);
    w.create_joint(&joint(
        top,
        a,
        JointKind::Fixed,
        v3(0.2, 0.0, 0.0),
        V3::ZERO,
    ));
    run(&mut w, 2.0);
    let b = w.body(a);
    assert!(
        (b.p - v3(0.4, 0.0, 0.0)).len() < 0.02,
        "sagged to {:?}",
        b.p
    );
    let up = b.q.rotate(v3(1.0, 0.0, 0.0));
    assert!(up.x > 0.995, "rotated: {up:?}");
}

#[test]
fn distance_joint_holds_length_and_a_spring_stretches() {
    let build = |spring: Option<(f64, f64)>| {
        let mut w = World::new(v3(0.0, 9.8, 0.0));
        let top = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
        let b = ball(&mut w, v3(0.0, 1.0, 0.0), BodyKind::Dynamic);
        let mut def = joint(
            top,
            b,
            JointKind::Distance {
                length: 1.0,
                spring,
                rope: false,
            },
            v3(0.0, 0.0, 0.0),
            V3::ZERO,
        );
        def.anchor_b = Some(v3(0.0, 1.0, 0.0));
        w.create_joint(&def);
        run(&mut w, 3.0);
        w.body(b).p.y
    };
    let rigid = build(None);
    let soft = build(Some((1.5, 0.5)));
    assert!((rigid - 1.0).abs() < 0.02, "rigid length {rigid}");
    assert!(soft > 1.05, "spring did not stretch: {soft}");
}

#[test]
fn rope_goes_slack_but_never_stretches() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let top = ball(&mut w, v3(0.0, 0.0, 0.0), BodyKind::Static);
    let b = ball(&mut w, v3(0.0, 0.2, 0.0), BodyKind::Dynamic);
    let mut def = joint(
        top,
        b,
        JointKind::Distance {
            length: 1.0,
            spring: None,
            rope: true,
        },
        v3(0.0, 0.0, 0.0),
        V3::ZERO,
    );
    def.anchor_b = Some(v3(0.0, 0.2, 0.0));
    w.create_joint(&def);
    run(&mut w, 0.25);
    // free fall through the slack part
    assert!(w.body(b).p.y > 0.25);
    run(&mut w, 3.0);
    assert!(w.body(b).p.y < 1.05, "stretched to {}", w.body(b).p.y);
}
