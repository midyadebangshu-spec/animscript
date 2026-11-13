use animscript_physics3::*;

fn cube_hull(h: f64) -> Hull {
    let mut pts = vec![];
    for x in [-h, h] {
        for y in [-h, h] {
            for z in [-h, h] {
                pts.push(v3(x, y, z));
            }
        }
    }
    Hull::from_points(&pts).unwrap()
}

fn floor(w: &mut World, y: f64) -> BodyId {
    // y points down: the solid is below `y`, so the outward normal points up (-y)
    let mut d = BodyDef::new(Shape::Plane {
        normal: v3(0.0, -1.0, 0.0),
        d: -y,
    });
    d.kind = BodyKind::Static;
    w.create_body(&d)
}

fn run(w: &mut World, seconds: f64) {
    let n = (seconds * 240.0) as usize;
    for _ in 0..n {
        w.step(1.0 / 240.0);
    }
}

#[test]
fn hull_of_a_cube_has_six_quad_faces_and_twelve_edges() {
    let h = cube_hull(0.5);
    assert_eq!(h.verts.len(), 8);
    assert_eq!(h.faces.len(), 6);
    assert_eq!(h.edges.len(), 12);
    assert!(h.faces.iter().all(|f| f.verts.len() == 4));
    let mp = h.mass_props();
    assert!((mp.volume - 1.0).abs() < 1e-9);
    assert!((mp.inertia.m[0][0] - 1.0 / 6.0).abs() < 1e-9);
    assert!(mp.centroid.len() < 1e-9);
}

#[test]
fn hull_of_flat_points_is_none() {
    let pts = [
        v3(0.0, 0.0, 0.0),
        v3(1.0, 0.0, 0.0),
        v3(0.0, 1.0, 0.0),
        v3(1.0, 1.0, 0.0),
    ];
    assert!(Hull::from_points(&pts).is_none());
}

#[test]
fn free_fall_matches_the_closed_form() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let b = w.create_body(&BodyDef::new(Shape::Sphere(0.1)));
    run(&mut w, 1.0);
    let p = w.body(b).p;
    // semi-implicit Euler is within a fraction of a percent of 0.5 g t^2
    assert!((p.y - 4.9).abs() < 0.03, "y = {}", p.y);
    assert!(p.x.abs() < 1e-12 && p.z.abs() < 1e-12);
}

#[test]
fn sphere_rests_on_a_plane() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.position = v3(0.0, 0.2, 0.0);
    let b = w.create_body(&d);
    run(&mut w, 2.0);
    let p = w.body(b).p;
    assert!((p.y - 0.9).abs() < 0.01, "y = {}", p.y);
    assert!(!w.body(b).awake, "it should have gone to sleep");
}

#[test]
fn box_rests_flat_and_does_not_jitter() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut d = BodyDef::new(Shape::Hull(cube_hull(0.25)));
    d.position = v3(0.0, 0.5, 0.0);
    let b = w.create_body(&d);
    run(&mut w, 3.0);
    let body = w.body(b);
    assert!((body.p.y - 0.75).abs() < 0.01, "y = {}", body.p.y);
    assert!(body.v.len() < 0.01);
    // still upright
    let up = body.q.rotate(v3(0.0, 1.0, 0.0));
    assert!(up.y > 0.999);
}

#[test]
fn box_stack_stays_stable_and_sleeps() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut ids = vec![];
    for k in 0..4 {
        let mut d = BodyDef::new(Shape::Hull(cube_hull(0.25)));
        d.position = v3(0.0, 0.75 - k as f64 * 0.5 - 0.001, 0.0);
        ids.push(w.create_body(&d));
    }
    run(&mut w, 5.0);
    for (k, id) in ids.iter().enumerate() {
        let b = w.body(*id);
        let want = 0.75 - k as f64 * 0.5;
        assert!(
            (b.p.y - want).abs() < 0.02,
            "box {k}: y = {} want {want}",
            b.p.y
        );
        assert!(b.p.x.abs() < 0.02 && b.p.z.abs() < 0.02, "box {k} drifted");
    }
    assert!(ids.iter().all(|i| !w.body(*i).awake));
}

#[test]
fn restitution_bounces_back() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.restitution = 0.8;
    d.position = v3(0.0, -1.0, 0.0);
    let b = w.create_body(&d);
    let mut top = f64::MAX;
    let mut hit = false;
    for _ in 0..(240 * 2) {
        w.step(1.0 / 240.0);
        let y = w.body(b).p.y;
        if y > 0.89 {
            hit = true;
        }
        if hit {
            top = top.min(y);
        }
    }
    // dropped from 1.9 above the plane contact height 0.9: rebound height ~ 0.64 * 1.9 = 1.2
    let rise = 0.9 - top;
    assert!(rise > 0.9 && rise < 1.4, "rose {rise}");
}

#[test]
fn friction_holds_a_box_on_a_slope_and_ice_lets_it_slide() {
    let slope = |mu: f64| {
        let mut w = World::new(v3(0.0, 9.8, 0.0));
        // a plane tilted 20 degrees about z
        let a = 20f64.to_radians();
        let n = v3(a.sin(), -a.cos(), 0.0);
        let mut d = BodyDef::new(Shape::Plane {
            normal: n,
            d: n.dot(v3(0.0, 1.0, 0.0)),
        });
        d.kind = BodyKind::Static;
        d.friction = mu;
        w.create_body(&d);
        let mut b = BodyDef::new(Shape::Hull(cube_hull(0.1)));
        b.friction = mu;
        b.orientation = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), a);
        b.position = v3(0.0, 1.0, 0.0) + n * 0.1;
        let id = w.create_body(&b);
        run(&mut w, 1.5);
        w.body(id).p
    };
    let held = slope(0.8);
    let slid = slope(0.02);
    assert!(held.x.abs() < 0.05, "stuck box moved {}", held.x);
    assert!(slid.x.abs() > 0.5, "slippery box only moved {}", slid.x);
}

#[test]
fn off_centre_impact_gives_spin() {
    let mut w = World::new(v3(0.0, 0.0, 0.0));
    let mut d = BodyDef::new(Shape::Hull(cube_hull(0.25)));
    let b = w.create_body(&d);
    d = BodyDef::new(Shape::Sphere(0.05));
    d.position = v3(-1.0, 0.2, 0.0);
    let s = w.create_body(&d);
    w.set_linear_velocity(s, v3(4.0, 0.0, 0.0));
    run(&mut w, 0.5);
    assert!(w.body(b).w.z < -0.02, "no spin: {:?}", w.body(b).w);
    assert!(w.body(b).v.x > 0.0);
}

#[test]
fn tetrahedron_comes_to_rest_on_a_face() {
    let pts = [
        v3(0.0, -0.2, 0.0),
        v3(0.2, 0.1, 0.0),
        v3(-0.1, 0.1, 0.17),
        v3(-0.1, 0.1, -0.17),
    ];
    let (hull, _) = hull_centered(&pts).unwrap();
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut d = BodyDef::new(Shape::Hull(hull));
    d.position = v3(0.0, 0.2, 0.0);
    d.orientation = Quat::from_axis_angle(v3(1.0, 0.3, 0.2), 1.1);
    let b = w.create_body(&d);
    run(&mut w, 4.0);
    assert!(!w.body(b).awake);
    // the lowest vertex height equals the plane: the hull rests on a face, not tilting through it
    let wh = match w.body(b).shape.world(w.body(b).p, w.body(b).q) {
        WShape::Hull(h) => h,
        _ => unreachable!(),
    };
    let low = wh.verts.iter().map(|v| v.y).fold(f64::MIN, f64::max);
    assert!((low - 1.0).abs() < 0.01, "lowest point at {low}");
    // three vertices touch the floor
    let touching = wh.verts.iter().filter(|v| (v.y - 1.0).abs() < 0.01).count();
    assert_eq!(touching, 3);
}

#[test]
fn vase_shaped_hull_tips_over_and_settles() {
    // a tall, slightly tapered hexagonal vase balanced on its rim: it falls over
    let mut pts = vec![];
    for k in 0..12 {
        let a = k as f64 / 12.0 * std::f64::consts::TAU;
        pts.push(v3(0.05 * a.cos(), -0.3, 0.05 * a.sin()));
        pts.push(v3(0.12 * a.cos(), 0.0, 0.12 * a.sin()));
        pts.push(v3(0.08 * a.cos(), 0.3, 0.08 * a.sin()));
    }
    let (hull, _) = hull_centered(&pts).unwrap();
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut d = BodyDef::new(Shape::Hull(hull));
    d.position = v3(0.0, 0.69, 0.0);
    d.orientation = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), 1.2);
    let b = w.create_body(&d);
    run(&mut w, 4.0);
    assert!(!w.body(b).awake, "still moving");
    let wh = match w.body(b).shape.world(w.body(b).p, w.body(b).q) {
        WShape::Hull(h) => h,
        _ => unreachable!(),
    };
    let low = wh.verts.iter().map(|v| v.y).fold(f64::MIN, f64::max);
    assert!((low - 1.0).abs() < 0.015, "lowest point at {low}");
}

#[test]
fn hull_rests_on_a_triangle_mesh_ground() {
    let tris = vec![
        Hull::triangle(v3(-5.0, 1.0, -5.0), v3(5.0, 1.0, -5.0), v3(5.0, 1.0, 5.0)).unwrap(),
        Hull::triangle(v3(-5.0, 1.0, -5.0), v3(5.0, 1.0, 5.0), v3(-5.0, 1.0, 5.0)).unwrap(),
    ];
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let mut g = BodyDef::new(Shape::Tris(tris));
    g.kind = BodyKind::Static;
    w.create_body(&g);
    let mut d = BodyDef::new(Shape::Hull(cube_hull(0.25)));
    d.position = v3(0.5, 0.4, 0.5);
    let b = w.create_body(&d);
    let mut s = BodyDef::new(Shape::Sphere(0.2));
    s.position = v3(-1.0, 0.0, 0.0);
    let sb = w.create_body(&s);
    run(&mut w, 3.0);
    assert!(
        (w.body(b).p.y - 0.75).abs() < 0.02,
        "box y {}",
        w.body(b).p.y
    );
    assert!(
        (w.body(sb).p.y - 0.8).abs() < 0.02,
        "ball y {}",
        w.body(sb).p.y
    );
}

#[test]
fn capsule_lies_flat_on_a_box_top() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let mut f = BodyDef::new(Shape::Hull(cube_hull(0.5)));
    f.kind = BodyKind::Static;
    f.position = v3(0.0, 0.5, 0.0);
    w.create_body(&f);
    let mut c = BodyDef::new(Shape::Capsule { half: 0.2, r: 0.05 });
    c.position = v3(0.0, -0.3, 0.0);
    c.orientation = Quat::from_axis_angle(v3(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2 + 0.2);
    let id = w.create_body(&c);
    run(&mut w, 3.0);
    let b = w.body(id);
    assert!((b.p.y - (0.0 - 0.05)).abs() < 0.01, "y {}", b.p.y);
    // lying horizontal: its axis ends up level
    let axis = b.q.rotate(v3(0.0, 1.0, 0.0));
    assert!(axis.y.abs() < 0.05, "axis {axis:?}");
}

#[test]
fn identical_runs_give_identical_hashes() {
    let build = || {
        let mut w = World::new(v3(0.0, 9.8, 0.0));
        floor(&mut w, 1.0);
        for k in 0..5 {
            let mut d = BodyDef::new(Shape::Hull(cube_hull(0.15)));
            d.position = v3(0.02 * k as f64, 0.5 - 0.4 * k as f64, 0.01 * k as f64);
            d.orientation = Quat::from_axis_angle(v3(1.0, 1.0, 0.0), 0.3 * k as f64);
            w.create_body(&d);
        }
        let mut s = BodyDef::new(Shape::Sphere(0.2));
        s.position = v3(0.1, -2.0, 0.0);
        w.create_body(&s);
        w
    };
    let (mut a, mut b) = (build(), build());
    run(&mut a, 3.0);
    run(&mut b, 3.0);
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn touch_events_begin_and_end() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let f = floor(&mut w, 1.0);
    let mut d = BodyDef::new(Shape::Sphere(0.1));
    d.restitution = 0.9;
    d.position = v3(0.0, 0.0, 0.0);
    let b = w.create_body(&d);
    let mut begins = 0;
    let mut ends = 0;
    for _ in 0..(240 * 2) {
        w.step(1.0 / 240.0);
        for e in w.take_events() {
            assert_eq!((e.a, e.b), (f, b));
            if e.begin {
                begins += 1
            } else {
                ends += 1
            }
        }
    }
    assert!(begins >= 2 && ends >= 1, "begins {begins} ends {ends}");
}
