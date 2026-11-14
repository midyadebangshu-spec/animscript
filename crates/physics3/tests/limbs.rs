use animscript_physics3::*;

fn capsule(w: &mut World, at: V3, q: Quat, kind: BodyKind) -> BodyId {
    let mut d = BodyDef::new(Shape::Capsule { half: 0.2, r: 0.06 });
    d.kind = kind;
    d.position = at;
    d.orientation = q;
    d.density = 400.0;
    w.create_body(&d)
}

fn run(w: &mut World, seconds: f64) {
    for _ in 0..(seconds * 240.0) as usize {
        w.step(1.0 / 240.0);
    }
}

fn ball(a: BodyId, b: BodyId, anchor: V3, axis: V3) -> JointDef {
    JointDef {
        a,
        b,
        kind: JointKind::Ball,
        anchor_a: anchor,
        anchor_b: None,
        axis,
        collide_connected: false,
    }
}

/// The angle between the capsule's axis (its local y) and `dir`.
fn angle_to(w: &World, id: BodyId, dir: V3) -> f64 {
    w.body(id)
        .q
        .rotate(v3(0.0, 1.0, 0.0))
        .dot(dir.normalized())
        .clamp(-1.0, 1.0)
        .acos()
}

#[test]
fn a_cone_limit_stops_a_limb_swinging_past_its_angle() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let root = capsule(&mut w, v3(0.0, 0.0, 0.0), Quat::IDENTITY, BodyKind::Static);
    // a limb starting pointing along +x (horizontal), hinged at the origin: gravity swings it down
    let q = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), -std::f64::consts::FRAC_PI_2);
    let limb = capsule(&mut w, v3(0.2, 0.0, 0.0), q, BodyKind::Dynamic);
    let j = w.create_joint(&ball(root, limb, V3::ZERO, v3(1.0, 0.0, 0.0)));
    w.set_joint_cone(
        j,
        Some(ConeDef {
            swing: 0.5,
            twist: None,
        }),
    );
    run(&mut w, 2.0);
    // it started 0 off the rest direction (+x); the cone is 0.5 rad around it: the limb hangs 0.5 below
    let a = angle_to(&w, limb, v3(1.0, 0.0, 0.0));
    assert!(a < 0.6 && a > 0.4, "swing angle {a}");
}

#[test]
fn a_twist_limit_stops_a_limb_spinning() {
    let mut w = World::new(V3::ZERO);
    let root = capsule(&mut w, V3::ZERO, Quat::IDENTITY, BodyKind::Static);
    let limb = capsule(&mut w, v3(0.0, 0.2, 0.0), Quat::IDENTITY, BodyKind::Dynamic);
    let j = w.create_joint(&ball(root, limb, V3::ZERO, v3(0.0, 1.0, 0.0)));
    w.set_joint_cone(
        j,
        Some(ConeDef {
            swing: 1.0,
            twist: Some((-0.4, 0.4)),
        }),
    );
    w.set_angular_velocity(limb, v3(0.0, 3.0, 0.0));
    run(&mut w, 1.5);
    // the capsule is a body of revolution about y: read the twist from the orientation about y
    let q = w.body(limb).q;
    let twist = 2.0 * q.y.atan2(q.w);
    assert!(twist.abs() < 0.55, "twist {twist}");
}

#[test]
fn a_muscle_holds_a_limb_up_against_gravity_and_a_weak_one_sags() {
    let held = |torque: f64| {
        let mut w = World::new(v3(0.0, 9.8, 0.0));
        let root = capsule(&mut w, V3::ZERO, Quat::IDENTITY, BodyKind::Static);
        let q = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), -std::f64::consts::FRAC_PI_2);
        let limb = capsule(&mut w, v3(0.2, 0.0, 0.0), q, BodyKind::Dynamic);
        let j = w.create_joint(&ball(root, limb, V3::ZERO, v3(1.0, 0.0, 0.0)));
        w.set_joint_muscle(
            j,
            Some(MuscleDef {
                hertz: 4.0,
                zeta: 0.7,
                max_torque: torque,
            }),
        );
        // aim at the starting orientation (horizontal)
        let rel = w.joint_rest_relative(j);
        w.set_joint_muscle_target(j, rel);
        run(&mut w, 2.0);
        angle_to(&w, limb, v3(1.0, 0.0, 0.0))
    };
    let strong = held(200.0);
    let weak = held(0.05);
    assert!(strong < 0.15, "the strong muscle let it sag {strong}");
    assert!(weak > 0.8, "the weak muscle held {weak}");
}

#[test]
fn a_chain_of_limbs_falls_and_settles_without_the_joints_coming_apart() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let mut floor = BodyDef::new(Shape::Plane {
        normal: v3(0.0, -1.0, 0.0),
        d: -2.0,
    });
    floor.kind = BodyKind::Static;
    w.create_body(&floor);
    let mut prev = None;
    let mut ids = vec![];
    for k in 0..4 {
        // a horizontal chain along x
        let q = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), -std::f64::consts::FRAC_PI_2);
        let id = capsule(
            &mut w,
            v3(0.2 + 0.4 * k as f64, 0.5, 0.0),
            q,
            BodyKind::Dynamic,
        );
        if let Some(p) = prev {
            let j = w.create_joint(&ball(
                p,
                id,
                v3(0.4 * k as f64, 0.5, 0.0),
                v3(1.0, 0.0, 0.0),
            ));
            w.set_joint_cone(
                j,
                Some(ConeDef {
                    swing: 1.2,
                    twist: Some((-0.5, 0.5)),
                }),
            );
        }
        prev = Some(id);
        ids.push(id);
    }
    run(&mut w, 5.0);
    for k in 1..ids.len() {
        let (a, b) = (w.body(ids[k - 1]), w.body(ids[k]));
        let anchor_gap =
            (a.p + a.q.rotate(v3(0.0, 0.2, 0.0)) - (b.p + b.q.rotate(v3(0.0, -0.2, 0.0)))).len();
        assert!(anchor_gap < 0.03, "joint {k} came apart by {anchor_gap}");
    }
    for id in &ids {
        let p = w.body(*id).p;
        assert!(
            p.x.is_finite() && p.y.is_finite() && p.y < 2.0 + 0.01,
            "{p:?}"
        );
    }
}
