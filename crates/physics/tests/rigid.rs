//! Rigid bodies: shapes, mass, decomposition, collision, friction, restitution, stacking, sleeping, events.
//! Written before the implementation. Units are metres, kilograms and seconds; y points down.
use animscript_physics::*;

const DT: f64 = 1.0 / 96.0;

fn world() -> World {
    World::new(v2(0.0, 10.0))
}
fn ground(w: &mut World, y: f64) -> BodyId {
    let g = w.create_body(&BodyDef {
        position: v2(0.0, y + 0.5),
        ..BodyDef::default()
    });
    w.add_fixture(g, &FixtureDef::new(Shape::boxed(50.0, 0.5)));
    g
}
fn dynamic(w: &mut World, pos: Vec2, shape: Shape) -> BodyId {
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: pos,
        ..BodyDef::default()
    });
    w.add_fixture(b, &FixtureDef::new(shape));
    b
}
fn run(w: &mut World, seconds: f64) {
    for _ in 0..(seconds / DT).round() as usize {
        w.step(DT);
    }
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}

// ---- shapes and mass --------------------------------------------------------------------------

#[test]
fn box_mass_and_inertia() {
    let m = Shape::boxed(1.0, 0.5).mass_data(2.0); // a 2 x 1 box, density 2
    near(m.mass, 4.0, 1e-12);
    near(m.center.x, 0.0, 1e-12);
    near(m.inertia, 4.0 * (4.0 + 1.0) / 12.0, 1e-12);
    let c = Shape::Circle {
        center: v2(1.0, 0.0),
        radius: 0.5,
    }
    .mass_data(1.0);
    near(c.mass, std::f64::consts::PI * 0.25, 1e-12);
    near(c.center.x, 1.0, 1e-12);
    near(c.inertia, 0.5 * c.mass * 0.25, 1e-12);
    near(
        Shape::edge(v2(0.0, 0.0), v2(1.0, 0.0)).mass_data(1.0).mass,
        0.0,
        0.0,
    );
}

#[test]
fn polygon_validation() {
    assert!(Polygon::new(vec![v2(0.0, 0.0), v2(1.0, 0.0), v2(1.0, 1.0), v2(0.0, 1.0)]).is_some());
    // clockwise or concave polygons are rejected (decompose them first)
    assert!(Polygon::new(vec![v2(0.0, 0.0), v2(0.0, 1.0), v2(1.0, 1.0), v2(1.0, 0.0)]).is_none());
    let l = vec![
        v2(0.0, 0.0),
        v2(2.0, 0.0),
        v2(2.0, 1.0),
        v2(1.0, 1.0),
        v2(1.0, 2.0),
        v2(0.0, 2.0),
    ];
    assert!(Polygon::new(l).is_none());
    assert!(Polygon::new(vec![v2(0.0, 0.0)]).is_none());
}

fn area(p: &[Vec2]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].cross(p[(i + 1) % n])).sum::<f64>() / 2.0
}

#[test]
fn decomposition_splits_a_concave_polygon_into_convex_pieces_keeping_its_area() {
    let l = vec![
        v2(0.0, 0.0),
        v2(2.0, 0.0),
        v2(2.0, 1.0),
        v2(1.0, 1.0),
        v2(1.0, 2.0),
        v2(0.0, 2.0),
    ];
    let pieces = decompose(&l).unwrap();
    assert!(pieces.len() >= 2 && pieces.len() <= 4, "{}", pieces.len());
    near(pieces.iter().map(|p| area(p)).sum::<f64>(), 3.0, 1e-9);
    for p in &pieces {
        assert!(Polygon::new(p.clone()).is_some(), "{p:?}"); // each piece is convex and counter-clockwise
    }
    // clockwise input works the same
    let mut cw = l.clone();
    cw.reverse();
    near(
        decompose(&cw).unwrap().iter().map(|p| area(p)).sum::<f64>(),
        3.0,
        1e-9,
    );
    // a convex polygon stays in one piece
    assert_eq!(
        decompose(&[v2(0.0, 0.0), v2(1.0, 0.0), v2(1.0, 1.0), v2(0.0, 1.0)])
            .unwrap()
            .len(),
        1
    );
    // a star
    let star: Vec<Vec2> = (0..10)
        .map(|i| {
            let a = i as f64 * std::f64::consts::TAU / 10.0;
            let r = if i % 2 == 0 { 2.0 } else { 0.8 };
            v2(a.cos() * r, a.sin() * r)
        })
        .collect();
    let pieces = decompose(&star).unwrap();
    near(
        pieces.iter().map(|p| area(p)).sum::<f64>(),
        area(&star).abs(),
        1e-9,
    );
    assert!(pieces.iter().all(|p| Polygon::new(p.clone()).is_some()));
}

#[test]
fn decomposition_rejects_bad_input() {
    assert!(decompose(&[v2(0.0, 0.0), v2(1.0, 1.0)]).is_err());
    assert!(decompose(&[v2(0.0, 0.0), v2(1.0, 0.0), v2(2.0, 0.0)]).is_err()); // no area
    let bow = vec![v2(0.0, 0.0), v2(2.0, 2.0), v2(2.0, 0.0), v2(0.0, 2.0)]; // self-intersecting
    assert!(decompose(&bow).is_err());
}

// ---- broadphase -------------------------------------------------------------------------------

#[test]
fn broadphase_matches_brute_force() {
    let mut seed = 12345u64;
    let mut rnd = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / (1u64 << 31) as f64
    };
    for _ in 0..20 {
        let boxes: Vec<Aabb> = (0..60)
            .map(|_| {
                let (x, y, w, h) = (rnd() * 20.0, rnd() * 20.0, rnd() * 3.0, rnd() * 3.0);
                Aabb {
                    lo: v2(x, y),
                    hi: v2(x + w, y + h),
                }
            })
            .collect();
        let mut brute = vec![];
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                if boxes[i].overlaps(&boxes[j]) {
                    brute.push((i, j));
                }
            }
        }
        assert_eq!(find_pairs(&boxes), brute); // same pairs, same (sorted) order
    }
}

// ---- free fall, resting, restitution ----------------------------------------------------------

#[test]
fn free_fall_follows_gravity() {
    let mut w = world();
    let b = dynamic(&mut w, v2(0.0, 0.0), Shape::boxed(0.5, 0.5));
    run(&mut w, 1.0);
    near(w.body(b).p.y, 5.0, 0.06); // 1/2 g t^2 (a symplectic step is 1% high)
    near(w.body(b).v.y, 10.0, 0.01);
    near(w.body(b).p.x, 0.0, 1e-12);
}

#[test]
fn a_box_comes_to_rest_on_the_ground_and_falls_asleep() {
    let mut w = world();
    ground(&mut w, 5.0);
    let b = dynamic(&mut w, v2(0.0, 3.0), Shape::boxed(0.5, 0.5));
    run(&mut w, 3.0);
    let body = w.body(b);
    near(body.p.y, 4.5, 0.02); // its centre rests half a metre above the ground surface at y = 5
    near(body.v.length(), 0.0, 0.01);
    near(body.q.angle(), 0.0, 1e-3);
    assert!(!body.awake, "a resting body sleeps");
}

#[test]
fn restitution_bounces_to_e_squared_times_the_drop_height() {
    let mut w = world();
    let g = w.create_body(&BodyDef {
        position: v2(0.0, 5.5),
        ..BodyDef::default()
    });
    w.add_fixture(
        g,
        &FixtureDef {
            restitution: 0.5,
            ..FixtureDef::new(Shape::boxed(50.0, 0.5))
        },
    );
    let ball = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 2.0),
        ..BodyDef::default()
    });
    w.add_fixture(
        ball,
        &FixtureDef {
            restitution: 0.5,
            ..FixtureDef::new(Shape::Circle {
                center: v2(0.0, 0.0),
                radius: 0.25,
            })
        },
    );
    // drop height: from y = 2 to the contact at y = 4.75 is 2.75 m; apex after the first bounce is e^2 * 2.75
    let mut lowest = f64::MIN;
    let mut apex = f64::MAX;
    let mut bounced = false;
    for _ in 0..(3.0 / DT) as usize {
        w.step(DT);
        let y = w.body(ball).p.y;
        if !bounced {
            lowest = lowest.max(y);
            if w.body(ball).v.y < 0.0 {
                bounced = true;
            }
        } else {
            apex = apex.min(y);
        }
    }
    assert!(bounced);
    let rise = lowest - apex;
    near(rise, 0.25 * 2.75, 0.12);
}

// ---- friction ---------------------------------------------------------------------------------

fn incline(angle: f64, mu: f64) -> (World, BodyId, Vec2) {
    let mut w = world();
    let g = w.create_body(&BodyDef {
        position: v2(0.0, 0.0),
        angle,
        ..BodyDef::default()
    });
    w.add_fixture(
        g,
        &FixtureDef {
            friction: mu,
            ..FixtureDef::new(Shape::boxed(20.0, 0.5))
        },
    );
    // a box resting on the sloped top surface, at the middle of the ramp
    let q = Rot::from_angle(angle);
    let normal = q.rotate(v2(0.0, -1.0)); // out of the surface
    let pos = normal * 1.0; // half the ramp thickness (0.5) + half the box (0.5)
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: pos,
        angle,
        ..BodyDef::default()
    });
    w.add_fixture(
        b,
        &FixtureDef {
            friction: mu,
            ..FixtureDef::new(Shape::boxed(0.5, 0.5))
        },
    );
    (w, b, pos)
}

#[test]
fn friction_holds_a_box_below_the_critical_angle() {
    let angle = 10f64.to_radians(); // tan = 0.176 < mu = 0.3
    let (mut w, b, start) = incline(angle, 0.3);
    run(&mut w, 2.0);
    assert!(
        (w.body(b).p - start).length() < 0.05,
        "{:?}",
        w.body(b).p - start
    );
}

#[test]
fn a_box_slides_with_the_analytic_acceleration_above_the_critical_angle() {
    let angle = 25f64.to_radians(); // tan = 0.466 > mu = 0.3
    let (mut w, b, start) = incline(angle, 0.3);
    run(&mut w, 1.0);
    let travelled = (w.body(b).p - start).length();
    let a = 10.0 * (angle.sin() - 0.3 * angle.cos());
    near(travelled, 0.5 * a * 1.0, 0.5 * a * 0.2);
}

// ---- stacking, sleeping, events ---------------------------------------------------------------

#[test]
fn a_stack_of_boxes_stays_put_and_sleeps() {
    let mut w = world();
    ground(&mut w, 10.0);
    let ids: Vec<BodyId> = (0..5)
        .map(|i| {
            dynamic(
                &mut w,
                v2(0.0, 9.5 - i as f64 * 1.0),
                Shape::boxed(0.5, 0.5),
            )
        })
        .collect();
    run(&mut w, 6.0);
    for (i, id) in ids.iter().enumerate() {
        let b = w.body(*id);
        near(b.p.x, 0.0, 0.03);
        near(b.p.y, 9.5 - i as f64, 0.06);
        assert!(!b.awake, "box {i} sleeps");
    }
}

#[test]
fn contact_events_report_begin_and_end_once_per_body_pair() {
    let mut w = world();
    let g = ground(&mut w, 5.0);
    let b = dynamic(&mut w, v2(0.0, 3.5), Shape::boxed(0.5, 0.5));
    let mut events = vec![];
    for _ in 0..(1.0 / DT) as usize {
        w.step(DT);
        events.extend(w.take_events());
    }
    let begins: Vec<_> = events.iter().filter(|e| e.begin).collect();
    assert_eq!(begins.len(), 1, "{events:?}");
    assert_eq!(
        (begins[0].a.min(begins[0].b), begins[0].a.max(begins[0].b)),
        (g.min(b), g.max(b))
    );
    // launch it away: the contact ends
    w.set_linear_velocity(b, v2(0.0, -8.0));
    let mut ends = 0;
    for _ in 0..(0.3 / DT) as usize {
        w.step(DT);
        ends += w.take_events().iter().filter(|e| !e.begin).count();
    }
    assert_eq!(ends, 1);
}

#[test]
fn an_impulse_wakes_a_sleeping_body_and_a_falling_body_wakes_a_sleeper() {
    let mut w = world();
    ground(&mut w, 5.0);
    let sleeper = dynamic(&mut w, v2(0.0, 4.5), Shape::boxed(0.5, 0.5));
    run(&mut w, 2.0);
    assert!(!w.body(sleeper).awake);
    w.apply_linear_impulse(sleeper, v2(2.0, 0.0));
    assert!(w.body(sleeper).awake);
    run(&mut w, 0.3);
    assert!(w.body(sleeper).p.x > 0.05);
    // let it settle and sleep again, then drop something on it
    run(&mut w, 3.0);
    assert!(!w.body(sleeper).awake);
    let x = w.body(sleeper).p.x;
    let _drop = dynamic(&mut w, v2(x, 1.0), Shape::boxed(0.4, 0.4));
    run(&mut w, 1.2);
    assert!(w.body(sleeper).awake || w.body(sleeper).p.y > 4.4);
}

// ---- body options, kinematic, sensor, determinism ---------------------------------------------

#[test]
fn fixed_rotation_damping_and_gravity_scale() {
    let mut w = world();
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 0.0),
        fixed_rotation: true,
        gravity_scale: 0.5,
        linear_damping: 1.0,
        angular_velocity: 5.0,
        ..BodyDef::default()
    });
    w.add_fixture(b, &FixtureDef::new(Shape::boxed(0.5, 0.5)));
    run(&mut w, 1.0);
    near(w.body(b).q.angle(), 0.0, 1e-12); // no rotation allowed
    assert!(
        w.body(b).v.y > 0.0 && w.body(b).v.y < 5.0,
        "half gravity and damped: {}",
        w.body(b).v.y
    );
}

#[test]
fn a_kinematic_body_moves_and_carries_a_box_by_friction() {
    let mut w = world();
    let plat = w.create_body(&BodyDef {
        kind: BodyKind::Kinematic,
        position: v2(0.0, 5.5),
        linear_velocity: v2(1.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(
        plat,
        &FixtureDef {
            friction: 0.8,
            ..FixtureDef::new(Shape::boxed(10.0, 0.5))
        },
    );
    let b = dynamic(&mut w, v2(0.0, 4.5), Shape::boxed(0.5, 0.5));
    run(&mut w, 2.0);
    near(w.body(plat).p.x, 2.0, 0.01);
    near(w.body(b).p.x, 1.9, 0.35); // carried along (after a short slip)
    assert!(w.body(b).p.y > 4.4 && w.body(b).p.y < 4.6);
}

#[test]
fn a_sensor_reports_overlap_without_pushing() {
    let mut w = world();
    let s = w.create_body(&BodyDef {
        position: v2(0.0, 5.0),
        ..BodyDef::default()
    });
    w.add_fixture(
        s,
        &FixtureDef {
            sensor: true,
            ..FixtureDef::new(Shape::boxed(5.0, 0.5))
        },
    );
    let b = dynamic(&mut w, v2(0.0, 3.0), Shape::boxed(0.5, 0.5));
    let mut began = false;
    for _ in 0..(1.5 / DT) as usize {
        w.step(DT);
        began |= w.take_events().iter().any(|e| e.begin);
    }
    assert!(began);
    assert!(
        w.body(b).p.y > 6.0,
        "it fell straight through: {}",
        w.body(b).p.y
    ); // 3 + 1/2 g (1.5)^2 minus nothing
}

#[test]
fn the_same_world_gives_bit_identical_results() {
    let build = || {
        let mut w = world();
        ground(&mut w, 8.0);
        for i in 0..12 {
            let x = (i % 3) as f64 * 0.9 - 0.9;
            dynamic(
                &mut w,
                v2(x, 6.0 - i as f64 * 0.6),
                if i % 2 == 0 {
                    Shape::boxed(0.4, 0.3)
                } else {
                    Shape::Circle {
                        center: v2(0.0, 0.0),
                        radius: 0.3,
                    }
                },
            );
        }
        w
    };
    let (mut a, mut b) = (build(), build());
    run(&mut a, 4.0);
    run(&mut b, 4.0);
    for id in 0..a.body_count() {
        let (x, y) = (a.body(id), b.body(id));
        assert_eq!(
            (x.p.x.to_bits(), x.p.y.to_bits(), x.q.s.to_bits()),
            (y.p.x.to_bits(), y.p.y.to_bits(), y.q.s.to_bits())
        );
    }
}

#[test]
fn a_concave_decomposed_body_rests_on_the_ground() {
    let mut w = world();
    ground(&mut w, 8.0);
    let l = vec![
        v2(0.0, 0.0),
        v2(2.0, 0.0),
        v2(2.0, 1.0),
        v2(1.0, 1.0),
        v2(1.0, 2.0),
        v2(0.0, 2.0),
    ];
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 5.0),
        ..BodyDef::default()
    });
    for piece in decompose(&l).unwrap() {
        w.add_fixture(
            b,
            &FixtureDef::new(Shape::Polygon(Polygon::new(piece).unwrap())),
        );
    }
    run(&mut w, 4.0);
    assert!(!w.body(b).awake);
    // it lies on a flat side: its lowest vertex touches the ground surface (y = 8)
    near(w.body(b).mass, 3.0, 1e-9);
}

#[test]
fn a_pile_of_two_hundred_bodies_simulates_within_budget() {
    // generous (it runs in a debug build): 200 bodies for 2 simulated seconds must take well under 20 s
    let mut w = world();
    ground(&mut w, 20.0);
    let wall = |w: &mut World, x: f64| {
        let b = w.create_body(&BodyDef {
            position: v2(x, 10.0),
            ..BodyDef::default()
        });
        w.add_fixture(b, &FixtureDef::new(Shape::boxed(0.5, 20.0)));
    };
    wall(&mut w, -5.5);
    wall(&mut w, 5.5);
    for i in 0..200 {
        let (col, row) = (i % 10, i / 10);
        let shape = if i % 3 == 0 {
            Shape::Circle {
                center: v2(0.0, 0.0),
                radius: 0.22,
            }
        } else {
            Shape::boxed(0.22, 0.18)
        };
        dynamic(
            &mut w,
            v2(-4.0 + col as f64 * 0.85, 18.0 - row as f64 * 0.7),
            shape,
        );
    }
    let start = std::time::Instant::now();
    run(&mut w, 2.0);
    let took = start.elapsed().as_secs_f64();
    assert!(took < 20.0, "took {took:.1} s");
    for id in 0..w.body_count() {
        let p = w.body(id).p;
        assert!(
            p.x.abs() < 6.0 && p.y < 21.0 && p.y > -1.0,
            "body {id} escaped to {p:?}"
        );
    }
}
