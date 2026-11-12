//! Fluids (position based, 2D): hydrostatics, volume, dam break, leaks, buoyancy, viscosity,
//! determinism. Written first. Metres, kilograms, seconds; y points down.
use animscript_physics::*;

const DT: f64 = 1.0 / 24.0;
const G: Vec2 = v2(0.0, 9.8);
const S: f64 = 0.08; // particle spacing

fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn tank(world: &mut World, width: f64, floor_y: f64) {
    // floor top at floor_y, walls reaching 5 m above it, inner width `width` centred on x = 0
    let t = 0.2;
    let f = world.create_body(&BodyDef {
        position: v2(0.0, floor_y + t / 2.0),
        ..BodyDef::default()
    });
    world.add_fixture(f, &FixtureDef::new(Shape::boxed(width / 2.0 + t, t / 2.0)));
    for side in [-1.0, 1.0] {
        let w = world.create_body(&BodyDef {
            position: v2(side * (width / 2.0 + t / 2.0), floor_y - 2.5),
            ..BodyDef::default()
        });
        world.add_fixture(w, &FixtureDef::new(Shape::boxed(t / 2.0, 3.0)));
    }
}
fn block(fluid: &mut Fluid, x0: f64, y0: f64, w: f64, h: f64) {
    let mut pts = vec![];
    let (nx, ny) = ((w / S).round() as usize, (h / S).round() as usize);
    for j in 0..ny {
        for i in 0..nx {
            pts.push(v2(x0 + (i as f64 + 0.5) * S, y0 + (j as f64 + 0.5) * S));
        }
    }
    fluid.add(&pts);
}
fn run(world: &mut World, fluids: &mut [Fluid], secs: f64) {
    for _ in 0..(secs / DT).round() as usize {
        coupled_step(Some(&mut *world), &mut [], fluids, G, DT, 4, 4, 3);
    }
}
fn level(f: &Fluid) -> f64 {
    // surface height: the mean of the topmost particle in each column
    let mut tops: std::collections::BTreeMap<i64, f64> = Default::default();
    for p in &f.pos {
        let k = (p.x / S).floor() as i64;
        let e = tops.entry(k).or_insert(f64::MAX);
        *e = e.min(p.y);
    }
    tops.values().sum::<f64>() / tops.len() as f64
}

#[test]
fn a_column_of_water_settles_flat_at_rest_density() {
    let mut w = World::new(G);
    tank(&mut w, 1.04, 3.0);
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    block(&mut f[0], -0.52, 2.0, 1.04, 0.96); // 13 x 12 particles standing on the floor
    let n0 = f[0].len();
    run(&mut w, &mut f, 3.0);
    assert_eq!(f[0].len(), n0, "no particle was lost");
    let r = f[0].median_density_ratio();
    near(r, 1.0, 0.08);
    for p in &f[0].pos {
        assert!(
            p.x.abs() < 0.52 + 0.01 && p.y < 3.0 + 0.01,
            "a particle escaped: {p:?}"
        );
    }
    // the free surface is flat to within about one spacing
    let top = f[0].pos.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let flat = level(&f[0]);
    assert!(
        flat - top < 1.5 * S,
        "surface not flat: mean {flat}, highest {top}"
    );
}

#[test]
fn volume_is_conserved_and_the_level_matches() {
    let mut w = World::new(G);
    tank(&mut w, 1.04, 3.0);
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    block(&mut f[0], -0.36, 0.5, 0.72, 0.8); // dropped from height
    let area = f[0].len() as f64 * S * S;
    run(&mut w, &mut f, 4.0);
    let expected_level = 3.0 - area / 1.04; // surface height for that area in a 1.04 m wide tank
    near(level(&f[0]), expected_level + S / 2.0, 0.12);
}

#[test]
fn a_dam_break_front_runs_at_about_the_shallow_water_speed() {
    let mut w = World::new(G);
    tank(&mut w, 5.0, 3.0);
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    let h0 = 0.8;
    block(&mut f[0], -2.5, 3.0 - h0, 0.8, h0);
    let front = |f: &Fluid| f.pos.iter().map(|p| p.x).fold(f64::MIN, f64::max);
    let x0 = front(&f[0]);
    run(&mut w, &mut f, 0.5);
    let speed = (front(&f[0]) - x0) / 0.5;
    let c = (9.8 * h0).sqrt();
    assert!(
        speed > 0.8 * c && speed < 2.2 * c,
        "front speed {speed:.2} m/s vs sqrt(g h) = {c:.2}"
    );
}

#[test]
fn fast_water_does_not_leak_through_a_thin_floor() {
    let mut w = World::new(G);
    let floor = w.create_body(&BodyDef {
        position: v2(0.0, 3.02),
        ..BodyDef::default()
    });
    w.add_fixture(floor, &FixtureDef::new(Shape::boxed(2.0, 0.02))); // 4 cm thick
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    let mut pts = vec![];
    for j in 0..6 {
        for i in 0..10 {
            pts.push((v2(-0.4 + i as f64 * S, 0.5 + j as f64 * S), v2(0.0, 8.0)));
        }
    }
    f[0].add_with_velocity(&pts);
    run(&mut w, &mut f, 2.0);
    assert!(
        // (a splash particle that skidded off the end of the 4 m floor may fall; only those above it count)
        f[0].pos
            .iter()
            .filter(|p| p.x.abs() < 2.0)
            .all(|p| p.y < 3.0 + 1e-3),
        "water went through the floor"
    );
}

fn float_test(density: f64) -> (f64, f64) {
    let mut w = World::new(G);
    tank(&mut w, 1.6, 3.0);
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    block(&mut f[0], -0.8, 2.2, 1.6, 0.8);
    let b = w.create_body(&BodyDef {
        kind: BodyKind::Dynamic,
        position: v2(0.0, 1.6),
        ..BodyDef::default()
    });
    w.add_fixture(
        b,
        &FixtureDef {
            density,
            ..FixtureDef::new(Shape::boxed(0.3, 0.3))
        },
    );
    run(&mut w, &mut f, 5.0);
    (w.body(b).p.y, level(&f[0]))
}

#[test]
fn a_light_box_floats_and_a_heavy_one_sinks() {
    let (y, surface) = float_test(0.4);
    // Archimedes: it sinks to 40% of its height (0.24 m below the surface)
    near(y + 0.3 - surface, 0.24, 0.12);
    assert!(y < 2.9, "it floats, it does not rest on the floor: {y}");
    let (y2, _) = float_test(3.0);
    assert!(
        y2 > 3.0 - 0.3 - 0.1,
        "the heavy box rests on the floor: {y2}"
    );
}

#[test]
fn a_moving_paddle_pushes_the_water() {
    let mut w = World::new(G);
    tank(&mut w, 3.0, 3.0);
    let paddle = w.create_body(&BodyDef {
        kind: BodyKind::Kinematic,
        position: v2(-1.2, 2.6),
        linear_velocity: v2(1.0, 0.0),
        ..BodyDef::default()
    });
    w.add_fixture(paddle, &FixtureDef::new(Shape::boxed(0.05, 0.5)));
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    block(&mut f[0], -1.0, 2.5, 1.0, 0.5);
    let mean_x = |f: &Fluid| f.pos.iter().map(|p| p.x).sum::<f64>() / f.len() as f64;
    let before = mean_x(&f[0]);
    run(&mut w, &mut f, 1.5);
    assert!(
        mean_x(&f[0]) > before + 0.3,
        "the water was shoved along: {} -> {}",
        before,
        mean_x(&f[0])
    );
}

fn slosh_energy(viscosity: f64) -> f64 {
    let mut w = World::new(G);
    tank(&mut w, 2.0, 3.0);
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        viscosity,
        ..FluidParams::default()
    })];
    block(&mut f[0], -1.0, 2.4, 1.0, 0.6); // half the tank: it sloshes
    run(&mut w, &mut f, 1.0);
    f[0].vel.iter().map(|v| v.length_sq()).sum::<f64>() / f[0].len() as f64
}

#[test]
fn higher_viscosity_damps_a_slosh_faster() {
    let (thin, thick) = (slosh_energy(0.0), slosh_energy(0.4));
    assert!(thick < 0.8 * thin, "{thick} vs {thin}");
}

#[test]
fn the_particle_cap_and_the_kill_box_hold() {
    let mut f = Fluid::new(FluidParams {
        spacing: S,
        max: 50,
        ..FluidParams::default()
    });
    let pts: Vec<Vec2> = (0..80).map(|i| v2(i as f64 * 0.01, 0.0)).collect();
    assert_eq!(f.add(&pts), 50);
    assert_eq!(f.len(), 50);
    f.set_bounds(v2(-1.0, -1.0), v2(1.0, 1.0));
    let mut fl = [f];
    for _ in 0..48 {
        coupled_step(None, &mut [], &mut fl, G, DT, 4, 4, 3);
    }
    assert!(
        fl[0].len() < 50,
        "particles that fell out of the bounds were removed: {}",
        fl[0].len()
    );
}

#[test]
fn a_pinned_rope_holds_water_back_one_way() {
    let mut rope = SoftBody::new(
        (0..9).map(|i| v2(-0.5 + i as f64 * 0.125, 3.0)).collect(),
        vec![0.05; 9],
        SoftParams::default(),
    );
    for i in 0..8 {
        rope.add_distance(i, i + 1, 0.0);
    }
    for i in 0..9 {
        rope.pin(i, v2(-0.5 + i as f64 * 0.125, 3.0));
    }
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    block(&mut f[0], -0.3, 1.8, 0.6, 0.6);
    let mut bodies = [rope];
    for _ in 0..72 {
        coupled_step(None, &mut bodies, &mut f, G, DT, 8, 4, 3);
    }
    let below = f[0]
        .pos
        .iter()
        .filter(|p| p.y > 3.0 + 0.02 && p.x.abs() < 0.45)
        .count();
    assert_eq!(below, 0, "water got through the pinned rope");
}

#[test]
fn the_same_setup_gives_bit_identical_results() {
    let build = || {
        let mut w = World::new(G);
        tank(&mut w, 1.04, 3.0);
        let mut f = [Fluid::new(FluidParams {
            spacing: S,
            ..FluidParams::default()
        })];
        block(&mut f[0], -0.4, 1.5, 0.8, 0.8);
        (w, f)
    };
    let ((mut wa, mut fa), (mut wb, mut fb)) = (build(), build());
    run(&mut wa, &mut fa, 1.0);
    run(&mut wb, &mut fb, 1.0);
    for (p, q) in fa[0].pos.iter().zip(&fb[0].pos) {
        assert_eq!(
            (p.x.to_bits(), p.y.to_bits()),
            (q.x.to_bits(), q.y.to_bits())
        );
    }
}

#[test]
fn fifteen_hundred_particles_simulate_within_budget() {
    let mut w = World::new(G);
    tank(&mut w, 2.0, 3.0);
    let mut f = [Fluid::new(FluidParams {
        spacing: S,
        ..FluidParams::default()
    })];
    block(&mut f[0], -0.96, 1.2, 1.92, 0.9);
    assert!(f[0].len() >= 250);
    block(&mut f[0], -0.96, 0.2, 1.92, 0.9);
    let start = std::time::Instant::now();
    run(&mut w, &mut f, 1.0);
    let took = start.elapsed().as_secs_f64();
    assert!(took < 40.0, "{} particles took {took:.1} s", f[0].len());
}
