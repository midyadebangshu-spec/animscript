//! Particle emitters: seeded emission, lifetime, forces, collision with bodies. Written first.
//! Particle units are pixels and seconds; collision against a `World` (metres) uses `ppm` px per metre.
use animscript_physics::*;

const DT: f64 = 1.0 / 96.0;
const PPM: f64 = 100.0;

fn def() -> EmitterDef {
    EmitterDef {
        lifetime: (1.0, 1.0),
        speed: (0.0, 0.0),
        ..EmitterDef::default()
    }
}
fn at_origin(_: u64) -> Vec2 {
    Vec2::ZERO
}
fn run(e: &mut Emitter, secs: f64, world: Option<&World>) {
    for _ in 0..(secs / DT).round() as usize {
        e.step(DT, world.map(|w| (w, PPM)), &mut at_origin);
    }
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}

#[test]
fn the_same_seed_gives_the_same_particles_and_another_seed_differs() {
    let d = EmitterDef {
        rate: 100.0,
        speed: (50.0, 150.0),
        angle: (0.0, std::f64::consts::TAU),
        lifetime: (0.5, 1.5),
        ..def()
    };
    let make = |seed| {
        let mut e = Emitter::new(d.clone(), seed);
        e.start();
        run(&mut e, 0.7, None);
        e.particles()
            .iter()
            .map(|p| (p.p.x.to_bits(), p.p.y.to_bits(), p.life.to_bits()))
            .collect::<Vec<_>>()
    };
    assert_eq!(make(7), make(7));
    assert_ne!(make(7), make(8));
    assert!(!make(7).is_empty());
}

#[test]
fn lifetimes_stay_in_range_and_the_pool_is_capped() {
    let d = EmitterDef {
        rate: 500.0,
        lifetime: (0.2, 0.4),
        max: 50,
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.start();
    for _ in 0..(1.0 / DT) as usize {
        e.step(DT, None, &mut at_origin);
        assert!(e.particles().len() <= 50);
        assert!(e
            .particles()
            .iter()
            .all(|p| p.life >= 0.2 && p.life <= 0.4 && p.age < p.life));
    }
    assert!(
        e.particles().len() >= 45,
        "the pool stays nearly full: {}",
        e.particles().len()
    );
}

#[test]
fn the_rate_is_exact_over_time_and_stopping_lets_particles_finish() {
    let d = EmitterDef {
        rate: 60.0,
        lifetime: (10.0, 10.0),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.start();
    run(&mut e, 1.0, None);
    assert!((59..=61).contains(&e.emitted()), "{}", e.emitted());
    e.stop();
    let n = e.particles().len();
    run(&mut e, 1.0, None);
    assert_eq!(
        e.particles().len(),
        n,
        "nothing new after stop, nothing has died yet"
    );
    let d = EmitterDef {
        rate: 60.0,
        lifetime: (0.3, 0.3),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.start();
    run(&mut e, 0.5, None);
    e.stop();
    run(&mut e, 0.5, None);
    assert!(e.particles().is_empty());
}

#[test]
fn a_burst_spawns_exactly_n() {
    let mut e = Emitter::new(
        EmitterDef {
            lifetime: (5.0, 5.0),
            ..def()
        },
        3,
    );
    e.burst(40);
    e.step(DT, None, &mut at_origin);
    assert_eq!(e.particles().len(), 40);
    e.step(DT, None, &mut at_origin);
    assert_eq!(e.particles().len(), 40);
}

#[test]
fn gravity_and_drag_follow_the_formulas() {
    let d = EmitterDef {
        gravity: v2(0.0, 980.0),
        lifetime: (5.0, 5.0),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.burst(1);
    run(&mut e, 1.0, None);
    near(e.particles()[0].p.y, 490.0, 8.0);
    let d = EmitterDef {
        drag: 1.0,
        speed: (100.0, 100.0),
        angle: (0.0, 0.0),
        lifetime: (5.0, 5.0),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.burst(1);
    run(&mut e, 1.0, None);
    near(e.particles()[0].v.x, 100.0 * (-1.0f64).exp(), 1.5);
}

fn ground(top_m: f64, thick_m: f64) -> World {
    let mut w = World::new(Vec2::ZERO);
    let g = w.create_body(&BodyDef {
        position: v2(0.0, top_m + thick_m / 2.0),
        ..BodyDef::default()
    });
    w.add_fixture(g, &FixtureDef::new(Shape::boxed(50.0, thick_m / 2.0)));
    w
}

#[test]
fn particles_bounce_with_their_restitution() {
    let w = ground(3.0, 0.5); // the floor's top is at y = 300 px
    let d = EmitterDef {
        gravity: v2(0.0, 980.0),
        restitution: 0.5,
        collide: true,
        size: (2.0, 2.0),
        lifetime: (10.0, 10.0),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.burst(1);
    let mut hit = false;
    let (mut lowest, mut apex) = (0.0f64, f64::MAX);
    for _ in 0..(3.0 / DT) as usize {
        e.step(DT, Some((&w, PPM)), &mut |_| v2(0.0, 100.0)); // spawns 100 px above the origin, i.e. at y = 100
        let p = e.particles()[0];
        if !hit {
            lowest = lowest.max(p.p.y);
            hit = p.v.y < 0.0;
        } else {
            apex = apex.min(p.p.y);
        }
    }
    assert!(hit);
    near(lowest, 298.0, 3.0); // rests against the surface minus its radius
                              // dropped 198 px: the bounce rises e^2 * 198 = 49.5 px
    near(lowest - apex, 49.5, 8.0);
}

#[test]
fn fast_particles_do_not_tunnel_through_a_thin_floor() {
    let w = ground(3.0, 0.1);
    let d = EmitterDef {
        speed: (700.0, 700.0),
        angle: (std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2),
        collide: true,
        restitution: 0.3,
        size: (2.0, 2.0),
        lifetime: (3.0, 3.0),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.burst(1);
    for _ in 0..(1.5 / DT) as usize {
        e.step(DT, Some((&w, PPM)), &mut |_| v2(0.0, 0.0));
        assert!(e.particles()[0].p.y < 305.0, "{}", e.particles()[0].p.y);
    }
}

#[test]
fn a_moving_body_throws_a_particle() {
    let mut w = World::new(Vec2::ZERO);
    let plat = w.create_body(&BodyDef {
        kind: BodyKind::Kinematic,
        position: v2(0.0, 0.5),
        linear_velocity: v2(0.0, -1.0),
        ..BodyDef::default()
    });
    w.add_fixture(plat, &FixtureDef::new(Shape::boxed(5.0, 0.1)));
    let d = EmitterDef {
        collide: true,
        restitution: 0.0,
        size: (2.0, 2.0),
        lifetime: (5.0, 5.0),
        ..def()
    };
    let mut e = Emitter::new(d, 1);
    e.burst(1);
    for _ in 0..(0.5 / DT) as usize {
        e.step(DT, Some((&w, PPM)), &mut |_| v2(0.0, 0.0));
        w.step(DT);
    }
    assert!(
        e.particles()[0].v.y < -50.0,
        "the platform carries it up: {}",
        e.particles()[0].v.y
    );
}
