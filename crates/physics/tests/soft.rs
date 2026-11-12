//! Soft bodies (XPBD): distance and area constraints, pins, collision with rigid bodies and with
//! each other, determinism. Written first. Metres and seconds; y points down.
use animscript_physics::*;

const DT: f64 = 1.0 / 24.0;
const G: Vec2 = v2(0.0, 9.8);

fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn run(bodies: &mut [SoftBody], world: Option<&mut World>, secs: f64) {
    let mut world = world;
    for _ in 0..(secs / DT).round() as usize {
        soft_step(bodies, world.as_deref_mut(), G, DT, 16, 4); // steps the world too
    }
}
fn chain(n: usize, len: f64, end_mass: f64, compliance: f64) -> SoftBody {
    let pos: Vec<Vec2> = (0..n)
        .map(|i| v2(i as f64 * len / (n - 1) as f64, 0.0))
        .collect();
    let mut mass = vec![0.05; n];
    mass[n - 1] = end_mass;
    let mut b = SoftBody::new(pos, mass, SoftParams::default());
    for i in 0..n - 1 {
        b.add_distance(i, i + 1, compliance);
    }
    b.pin(0, v2(0.0, 0.0));
    b
}
fn polygon_area(b: &SoftBody, ring: &[usize]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| b.pos[ring[i]].cross(b.pos[ring[(i + 1) % n]]))
        .sum::<f64>()
        / 2.0
}

#[test]
fn a_rigid_chain_keeps_its_link_lengths_under_load() {
    let mut b = [chain(21, 1.0, 1.0, 0.0)];
    run(&mut b, None, 2.0);
    for i in 0..20 {
        near((b[0].pos[i + 1] - b[0].pos[i]).length(), 0.05, 0.05 * 0.06); // within 6% with a 20:1 mass ratio
    }
    near(b[0].pos[0].length(), 0.0, 1e-9); // the pin holds
    assert!(b[0].pos[20].y > 0.5, "it hangs down: {:?}", b[0].pos[20]);
}

#[test]
fn a_rope_with_a_weight_swings_like_a_pendulum() {
    let mut b = [chain(11, 1.0, 5.0, 0.0)];
    // start the rope horizontal, pull the end up a little to swing from a small angle
    let a0 = 6f64.to_radians();
    for i in 0..11 {
        let r = i as f64 * 0.1;
        b[0].pos[i] = v2(r * a0.sin(), r * a0.cos());
    }
    let mut crossings = vec![];
    let mut prev = b[0].pos[10].x;
    for k in 0..(8.0 / DT) as usize {
        soft_step(&mut b, None, G, DT, 16, 4);
        let x = b[0].pos[10].x;
        if prev > 0.0 && x <= 0.0 {
            crossings.push(k as f64 * DT);
        }
        prev = x;
    }
    assert!(crossings.len() >= 2, "{crossings:?}");
    near(
        crossings[1] - crossings[0],
        2.0 * std::f64::consts::PI * (1.0f64 / 9.8).sqrt(),
        0.2,
    );
}

#[test]
fn lower_stiffness_stretches_more() {
    let stretch = |c: f64| {
        let mut b = [chain(2, 0.5, 20.0, c)];
        run(&mut b, None, 3.0);
        (b[0].pos[1] - b[0].pos[0]).length() - 0.5
    };
    let (rigid, soft, softer) = (stretch(0.0), stretch(0.002), stretch(0.02));
    assert!(rigid.abs() < 0.005, "{rigid}");
    assert!(
        soft > rigid + 0.01 && softer > soft + 0.02,
        "{rigid} {soft} {softer}"
    );
}

fn ring(
    n: usize,
    r: f64,
    centre: Vec2,
    compliance: f64,
    pressure: f64,
    mass: f64,
) -> (SoftBody, Vec<usize>) {
    let pos: Vec<Vec2> = (0..n)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / n as f64;
            centre + v2(a.cos() * r, a.sin() * r)
        })
        .collect();
    let mut b = SoftBody::new(pos, vec![mass; n], SoftParams::default());
    for i in 0..n {
        b.add_distance(i, (i + 1) % n, compliance);
        b.add_distance(i, (i + 2) % n, compliance * 4.0);
    }
    let r: Vec<usize> = (0..n).collect();
    b.add_area(&r, pressure, compliance * 0.1);
    (b, r)
}

#[test]
fn pressure_inflates_and_unit_pressure_keeps_the_volume() {
    let mut w = World::new(Vec2::ZERO);
    let _ = &mut w;
    let (b, r) = ring(24, 0.5, v2(0.0, 0.0), 0.01, 1.5, 0.02); // soft edges: a balloon has to stretch
    let a0 = polygon_area(&b, &r); // the rest area (the target is 1.5 times this)
    let mut bodies = [b];
    for _ in 0..(3.0 / DT) as usize {
        soft_step(&mut bodies, None, Vec2::ZERO, DT, 16, 4);
    }
    near(polygon_area(&bodies[0], &r) / a0, 1.5, 0.08);
    // unit pressure under gravity, resting on the ground
    let mut world = World::new(G);
    let g = world.create_body(&BodyDef {
        position: v2(0.0, 2.5),
        ..BodyDef::default()
    });
    world.add_fixture(g, &FixtureDef::new(Shape::boxed(10.0, 0.5)));
    let (b, r) = ring(24, 0.5, v2(0.0, 1.0), 0.0005, 1.0, 0.02);
    let a1 = polygon_area(&b, &r);
    let mut bodies = [b];
    run(&mut bodies, Some(&mut world), 4.0);
    near(polygon_area(&bodies[0], &r) / a1, 1.0, 0.06);
    let lowest = bodies[0].pos.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    assert!(
        lowest < 2.0 + 0.02,
        "nothing sank into the ground (top at y = 2): {lowest}"
    );
}

fn jelly(n: usize, size: f64, top_left: Vec2, compliance: f64) -> (SoftBody, Vec<Vec<usize>>) {
    let idx = |i: usize, j: usize| j * (n + 1) + i;
    let mut pos = vec![];
    for j in 0..=n {
        for i in 0..=n {
            pos.push(top_left + v2(i as f64 * size / n as f64, j as f64 * size / n as f64));
        }
    }
    let m = pos.len();
    let mut b = SoftBody::new(
        pos,
        vec![0.02; m],
        SoftParams {
            restitution: 0.2,
            friction: 0.4,
            ..SoftParams::default()
        },
    );
    let mut faces = vec![];
    for j in 0..n {
        for i in 0..n {
            let ring = vec![idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1)];
            for k in 0..4 {
                b.add_distance(ring[k], ring[(k + 1) % 4], compliance);
            }
            b.add_distance(ring[0], ring[2], compliance);
            b.add_distance(ring[1], ring[3], compliance);
            b.add_area(&ring, 1.0, compliance);
            faces.push(ring);
        }
    }
    (b, faces)
}

#[test]
fn a_jelly_squashes_on_the_ground_and_stays_above_it() {
    let mut world = World::new(G);
    let g = world.create_body(&BodyDef {
        position: v2(0.0, 3.5),
        ..BodyDef::default()
    });
    world.add_fixture(g, &FixtureDef::new(Shape::boxed(10.0, 0.5))); // top at y = 3
    let (b, faces) = jelly(4, 1.0, v2(-0.5, 0.0), 0.004);
    let area0: f64 = faces.iter().map(|f| polygon_area(&b, f)).sum();
    let mut bodies = [b];
    let mut squashed = f64::MAX;
    for _ in 0..(3.0 / DT) as usize {
        soft_step(&mut bodies, Some(&mut world), G, DT, 16, 4);
        let h = bodies[0].pos.iter().map(|p| p.y).fold(f64::MIN, f64::max)
            - bodies[0].pos.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        squashed = squashed.min(h);
        for p in &bodies[0].pos {
            assert!(p.y < 3.0 + 0.02, "a vertex went through the ground: {p:?}");
        }
    }
    assert!(squashed < 0.98, "it squashed on impact: {squashed}");
    let area: f64 = faces.iter().map(|f| polygon_area(&bodies[0], f)).sum();
    near(area / area0, 1.0, 0.12);
    let speed: f64 = bodies[0].vel.iter().map(|v| v.length()).fold(0.0, f64::max);
    assert!(speed < 0.5, "it settled: {speed}");
}

#[test]
fn a_heavy_rigid_box_rests_on_a_soft_pad_and_does_not_sink_through() {
    let mut world = World::new(G);
    let g = world.create_body(&BodyDef {
        position: v2(0.0, 3.5),
        ..BodyDef::default()
    });
    world.add_fixture(g, &FixtureDef::new(Shape::boxed(10.0, 0.5)));
    let (pad, _) = jelly(4, 1.0, v2(-0.5, 2.0), 0.0005); // a 1 m pad, 1 m tall, resting on the ground
    let b = world.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 1.5), // dropped from 0.3 m: a harder drop tumbles it off a 1 m pad
        ..BodyDef::default()
    });
    world.add_fixture(
        b,
        &FixtureDef {
            density: 2.0,
            ..FixtureDef::new(Shape::boxed(0.3, 0.2))
        },
    );
    let mut bodies = [pad];
    run(&mut bodies, Some(&mut world), 4.0);
    let y = world.body(b).p.y;
    assert!(
        y > 1.0 && y < 2.0,
        "the box sits on the pad, above the ground and not inside it: {y}"
    );
    let top = bodies[0].pos.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    assert!(top > 1.5, "the pad was compressed, not crossed: {top}");
}

#[test]
fn pins_follow_their_target() {
    let mut b = [chain(6, 0.5, 0.05, 0.0)];
    b[0].pin(0, v2(0.0, 0.0));
    for k in 1..=48 {
        b[0].set_pin_target(0, v2(k as f64 * 0.02, 0.0));
        soft_step(&mut b, None, G, DT, 16, 4);
    }
    near(b[0].pos[0].x, 0.96, 1e-6);
    assert!(
        b[0].pos[5].x > 0.3,
        "the rope was dragged along: {}",
        b[0].pos[5].x
    );
    b[0].unpin(0);
    assert!(b[0].pins[0].is_none());
}

#[test]
fn a_falling_vertex_does_not_pass_through_another_soft_edge() {
    // a pinned horizontal bar, and a loose point mass falling onto it
    let mut bar = SoftBody::new(
        vec![v2(-1.0, 0.0), v2(0.0, 0.0), v2(1.0, 0.0)],
        vec![1.0; 3],
        SoftParams::default(),
    );
    bar.add_distance(0, 1, 0.0);
    bar.add_distance(1, 2, 0.0);
    bar.pin(0, v2(-1.0, 0.0));
    bar.pin(2, v2(1.0, 0.0));
    bar.pin(1, v2(0.0, 0.0));
    let drop = SoftBody::new(vec![v2(0.13, -1.0)], vec![0.05], SoftParams::default());
    let mut bodies = [bar, drop];
    for _ in 0..(2.0 / DT) as usize {
        soft_step(&mut bodies, None, G, DT, 16, 4);
        assert!(
            bodies[1].pos[0].y < 0.0 + 1e-3,
            "it crossed the bar: {}",
            bodies[1].pos[0].y
        );
    }
}

#[test]
fn the_same_setup_gives_bit_identical_results() {
    let build = || {
        let (b, _) = jelly(3, 1.0, v2(0.0, 0.0), 0.001);
        [b]
    };
    let (mut a, mut b) = (build(), build());
    for body in [&mut a, &mut b] {
        for _ in 0..40 {
            soft_step(body, None, G, DT, 16, 4);
        }
    }
    for (p, q) in a[0].pos.iter().zip(&b[0].pos) {
        assert_eq!(
            (p.x.to_bits(), p.y.to_bits()),
            (q.x.to_bits(), q.y.to_bits())
        );
    }
}

#[test]
fn fifteen_hundred_vertices_simulate_within_budget() {
    let (b, _) = jelly(38, 3.0, v2(-1.5, 0.0), 0.001);
    assert!(b.pos.len() >= 1500);
    let mut bodies = [b];
    let start = std::time::Instant::now();
    for _ in 0..48 {
        soft_step(&mut bodies, None, G, DT, 16, 4);
    }
    let took = start.elapsed().as_secs_f64();
    assert!(took < 30.0, "took {took:.1} s");
}

#[test]
fn a_rope_cannot_slip_past_a_post_thinner_than_its_vertex_spacing() {
    // five vertices 0.5 m apart, both ends pinned; a 2 cm post stands between the second and third
    let pos: Vec<Vec2> = (0..5).map(|i| v2(i as f64 * 0.5, 0.0)).collect();
    let mut rope = SoftBody::new(pos, vec![0.05; 5], SoftParams::default());
    for i in 0..4 {
        rope.add_distance(i, i + 1, 0.01); // a stretchy rope that sags onto the post
    }
    rope.pin(0, v2(0.0, 0.0));
    rope.pin(4, v2(2.0, 0.0));
    let mut world = World::new(Vec2::ZERO);
    let post = world.create_body(&BodyDef {
        position: v2(0.6, 0.54),
        ..BodyDef::default()
    });
    world.add_fixture(post, &FixtureDef::new(Shape::boxed(0.01, 0.5))); // its top is at y = 0.04
    let mut bodies = [rope];
    run(&mut bodies, Some(&mut world), 3.0);
    let (a, b) = (bodies[0].pos[1], bodies[0].pos[2]);
    let y_at_post = a.y + (b.y - a.y) * (0.6 - a.x) / (b.x - a.x);
    assert!(
        y_at_post < 0.04 + 0.025,
        "the rope hangs through the post: y at the post = {y_at_post}"
    );
    assert!(b.y > 0.04, "and it did sag down to it: {}", b.y);
}

#[test]
fn a_fully_pinned_soft_body_is_an_obstacle_for_rigid_bodies() {
    let pos: Vec<Vec2> = (0..6).map(|i| v2(i as f64 * 0.5, 1.0)).collect();
    let mut line = SoftBody::new(pos, vec![0.05; 6], SoftParams::default());
    for i in 0..5 {
        line.add_distance(i, i + 1, 0.0);
    }
    for i in 0..6 {
        line.pin(i, v2(i as f64 * 0.5, 1.0));
    }
    let mut world = World::new(G);
    let ball = world.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(1.25, 0.0),
        ..BodyDef::default()
    });
    world.add_fixture(
        ball,
        &FixtureDef::new(Shape::Circle {
            center: v2(0.0, 0.0),
            radius: 0.2,
        }),
    );
    let mut bodies = [line];
    run(&mut bodies, Some(&mut world), 2.0);
    let y = world.body(ball).p.y;
    assert!(
        y < 0.9,
        "the ball rests on the pinned line, not below it: {y}"
    );
}
