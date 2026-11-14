use animscript_physics3::*;

const DT: f64 = 1.0 / 24.0;
const S: f64 = 0.1;

fn plane(w: &mut World, n: V3, d: f64) {
    let mut def = BodyDef::new(Shape::Plane { normal: n, d });
    def.kind = BodyKind::Static;
    def.friction = 0.0;
    w.create_body(&def);
}

/// A tank open at the top: x in 0..wd, z in 0..dp, the floor at y = h (y points down).
fn tank(wd: f64, dp: f64, h: f64) -> World {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    plane(&mut w, v3(0.0, -1.0, 0.0), -h);
    plane(&mut w, v3(1.0, 0.0, 0.0), 0.0);
    plane(&mut w, v3(-1.0, 0.0, 0.0), -wd);
    plane(&mut w, v3(0.0, 0.0, 1.0), 0.0);
    plane(&mut w, v3(0.0, 0.0, -1.0), -dp);
    w
}

fn block(x0: f64, y0: f64, z0: f64, nx: usize, ny: usize, nz: usize) -> Vec<V3> {
    let mut v = vec![];
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                v.push(v3(
                    x0 + (i as f64 + 0.5) * S,
                    y0 + (j as f64 + 0.5) * S,
                    z0 + (k as f64 + 0.5) * S,
                ));
            }
        }
    }
    v
}

fn fluid(threads: usize) -> Fluid3 {
    Fluid3::new(FluidParams3 {
        spacing: S,
        threads,
        ..FluidParams3::default()
    })
}

fn run(w: &mut World, f: &mut Fluid3, seconds: f64) {
    for _ in 0..(seconds / DT) as usize {
        coupled_step3(w, &mut [], std::slice::from_mut(f), DT, 4, 3, 3, 4);
    }
}

#[test]
fn water_at_rest_keeps_its_rest_density() {
    let mut w = tank(0.8, 0.8, 1.0);
    let mut f = fluid(0);
    f.add(&block(0.0, 0.2, 0.0, 8, 8, 8));
    run(&mut w, &mut f, 1.5);
    let r = f.median_density_ratio();
    assert!(r > 0.85 && r < 1.2, "density ratio {r}");
    assert_eq!(f.len(), 512);
}

#[test]
fn a_dam_break_settles_to_a_level_surface_without_leaking() {
    let (wd, dp, h) = (2.0, 1.0, 1.0);
    let mut w = tank(wd, dp, h);
    let mut f = fluid(0);
    f.add(&block(0.0, 0.0, 0.0, 6, 10, 10));
    let n = f.len();
    run(&mut w, &mut f, 4.0);
    assert_eq!(f.len(), n, "particles were lost");
    for p in &f.pos {
        assert!(
            p.x > -0.05 && p.x < wd + 0.05 && p.z > -0.05 && p.z < dp + 0.05 && p.y < h + 0.05,
            "leaked: {p:?}"
        );
    }
    // the surface (highest particle of each x bin) is level, at about volume / floor area = 0.3 m
    let mut tops = vec![];
    for b in 0..10 {
        let (lo, hi) = (b as f64 * 0.2, (b as f64 + 1.0) * 0.2);
        let top = f
            .pos
            .iter()
            .filter(|p| p.x >= lo && p.x < hi)
            .map(|p| p.y)
            .fold(f64::MAX, f64::min);
        if top < f64::MAX {
            tops.push(top);
        }
    }
    let (min, max) = (
        tops.iter().cloned().fold(f64::MAX, f64::min),
        tops.iter().cloned().fold(f64::MIN, f64::max),
    );
    assert!(max - min < 0.3, "the surface is not level: {min}..{max}");
    let mean = tops.iter().sum::<f64>() / tops.len() as f64;
    assert!((mean - (h - 0.3)).abs() < 0.12, "mean surface at {mean}");
}

fn crate_body(w: &mut World, density: f64, at: V3) -> BodyId {
    let hf = 0.2;
    let mut pts = vec![];
    for x in [-hf, hf] {
        for y in [-hf, hf] {
            for z in [-hf, hf] {
                pts.push(v3(x, y, z));
            }
        }
    }
    let mut def = BodyDef::new(Shape::Hull(Hull::from_points(&pts).unwrap()));
    def.density = density;
    def.position = at;
    w.create_body(&def)
}

#[test]
fn a_light_crate_floats_and_a_heavy_one_sinks() {
    let run_with = |density: f64| {
        let mut w = tank(1.2, 1.2, 1.0);
        let mut f = fluid(0);
        f.add(&block(0.0, 0.2, 0.0, 12, 8, 12));
        let id = crate_body(&mut w, density, v3(0.6, 0.0, 0.6));
        run(&mut w, &mut f, 3.0);
        (w.body(id).p, f.len())
    };
    let (light, n) = run_with(0.4);
    let (heavy, _) = run_with(3.0);
    assert!(n > 1000);
    // water surface is at about y = 0.2; the floor at 1.0 (a crate on the floor has its centre at 0.8)
    assert!(light.y < 0.45, "the light crate sank to {}", light.y);
    assert!(heavy.y > 0.65, "the heavy crate floated at {}", heavy.y);
}

#[test]
fn a_pour_adds_particles_up_to_the_cap() {
    let mut f = Fluid3::new(FluidParams3 {
        spacing: S,
        max: 10,
        ..FluidParams3::default()
    });
    let pts: Vec<(V3, V3)> = (0..25)
        .map(|i| (v3(i as f64 * S, 0.0, 0.0), v3(0.0, 1.0, 0.0)))
        .collect();
    assert_eq!(f.add_with_velocity(&pts), 10);
    assert_eq!(f.len(), 10);
}

#[test]
fn the_thread_count_does_not_change_the_result() {
    let go = |threads: usize| {
        let mut w = tank(1.0, 1.0, 1.0);
        let mut f = fluid(threads);
        f.add(&block(0.0, 0.0, 0.0, 5, 10, 10));
        run(&mut w, &mut f, 1.0);
        f.state_hash()
    };
    let (a, b, c) = (go(1), go(4), go(3));
    assert_eq!(a, b);
    assert_eq!(a, c);
    assert_eq!(go(1), a, "two runs differ");
}

#[test]
fn spray_is_seeded_and_never_pushes_the_water() {
    let go = |foam: f64| {
        let mut w = tank(1.0, 1.0, 1.0);
        let mut f = Fluid3::new(FluidParams3 {
            spacing: S,
            foam,
            ..FluidParams3::default()
        });
        f.set_seed(7);
        f.add(&block(0.0, 0.0, 0.0, 5, 10, 10));
        run(&mut w, &mut f, 1.0);
        f
    };
    let (a, b, none) = (go(1.0), go(1.0), go(0.0));
    assert!(!a.foam.is_empty(), "no foam was thrown");
    assert_eq!(a.foam.len(), b.foam.len());
    assert_eq!(a.state_hash(), none.state_hash(), "foam changed the water");
}
