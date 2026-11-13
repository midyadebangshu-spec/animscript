#![allow(clippy::needless_range_loop)]
use animscript_physics3::*;

const DT: f64 = 1.0 / 24.0;

fn uv_sphere(r: f64, seg: usize, rings: usize, c: V3) -> (Vec<V3>, Vec<[usize; 3]>) {
    let mut v = vec![c + v3(0.0, -r, 0.0)];
    for i in 1..rings {
        let phi = std::f64::consts::PI * i as f64 / rings as f64;
        for j in 0..seg {
            let th = std::f64::consts::TAU * j as f64 / seg as f64;
            v.push(
                c + v3(
                    r * phi.sin() * th.cos(),
                    -r * phi.cos(),
                    r * phi.sin() * th.sin(),
                ),
            );
        }
    }
    v.push(c + v3(0.0, r, 0.0));
    let last = v.len() - 1;
    let ring = |i: usize, j: usize| 1 + (i - 1) * seg + j % seg;
    let mut t = vec![];
    for j in 0..seg {
        t.push([0, ring(1, j + 1), ring(1, j)]);
    }
    for i in 1..rings - 1 {
        for j in 0..seg {
            t.push([ring(i, j), ring(i, j + 1), ring(i + 1, j)]);
            t.push([ring(i, j + 1), ring(i + 1, j + 1), ring(i + 1, j)]);
        }
    }
    for j in 0..seg {
        t.push([ring(rings - 1, j), ring(rings - 1, j + 1), last]);
    }
    (v, t)
}

/// A flat sheet in the xz plane, `n` x `n` vertices, `size` metres, at height `y`, centred on (cx, cz).
fn sheet(n: usize, size: f64, y: f64, cx: f64, cz: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let mut v = vec![];
    for j in 0..n {
        for i in 0..n {
            v.push(v3(
                cx + size * (i as f64 / (n - 1) as f64 - 0.5),
                y,
                cz + size * (j as f64 / (n - 1) as f64 - 0.5),
            ));
        }
    }
    let mut t = vec![];
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = j * n + i;
            t.push([a, a + 1, a + n]);
            t.push([a + 1, a + n + 1, a + n]);
        }
    }
    (v, t)
}

fn groups(t: &[[usize; 3]]) -> Vec<usize> {
    (0..t.len()).collect()
}

fn floor(w: &mut World, y: f64) -> BodyId {
    let mut d = BodyDef::new(Shape::Plane {
        normal: v3(0.0, -1.0, 0.0),
        d: -y,
    });
    d.kind = BodyKind::Static;
    d.friction = 0.8;
    w.create_body(&d)
}

fn balloon(c: V3, r: f64, pressure: f64) -> SoftBody3 {
    let (v, t) = uv_sphere(r, 16, 10, c);
    SoftBody3::from_surface(
        &v,
        &t,
        &groups(&t),
        &SurfaceSpec {
            density: 1.0,
            edge_compliance: 1e-5,
            bend_compliance: 1e-3,
            volume: Some((pressure, 1e-6)),
        },
        SoftParams {
            radius: 0.015,
            ..SoftParams::default()
        },
    )
}

fn run(w: &mut World, b: &mut [SoftBody3], seconds: f64) {
    for _ in 0..(seconds / DT) as usize {
        soft_step(w, b, DT, 16, 4);
    }
}

fn centroid(b: &SoftBody3) -> V3 {
    let mut c = V3::ZERO;
    for i in 0..b.points.len() {
        c += b.point_pos(i);
    }
    c / b.points.len() as f64
}

fn hash(b: &SoftBody3) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in &b.pos {
        for x in [p.x, p.y, p.z] {
            for byte in x.to_bits().to_le_bytes() {
                h ^= byte as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
    }
    h
}

#[test]
fn a_closed_surface_is_detected() {
    let (_, t) = uv_sphere(0.5, 12, 8, V3::ZERO);
    assert!(is_closed(&t));
    let (_, s) = sheet(4, 1.0, 0.0, 0.0, 0.0);
    assert!(!is_closed(&s));
}

#[test]
fn pressure_inflates_a_balloon() {
    let mut w = World::new(V3::ZERO);
    let mut b = [balloon(V3::ZERO, 0.5, 1.4)];
    let v0 = {
        let flat = balloon(V3::ZERO, 0.5, 1.0);
        flat.surface_volume()
    };
    run(&mut w, &mut b, 1.5);
    let ratio = b[0].surface_volume() / v0;
    assert!(ratio > 1.25 && ratio < 1.5, "volume ratio {ratio}");
}

#[test]
fn a_balloon_dropped_on_the_floor_keeps_its_volume_and_rests() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let mut b = [balloon(v3(0.0, 0.2, 0.0), 0.3, 1.0)];
    let v0 = b[0].surface_volume();
    run(&mut w, &mut b, 3.0);
    let ratio = b[0].surface_volume() / v0;
    assert!(ratio > 0.85 && ratio < 1.15, "volume ratio {ratio}");
    let low = (0..b[0].points.len())
        .map(|i| b[0].point_pos(i).y)
        .fold(f64::MIN, f64::max);
    assert!(low < 1.0 + 0.03, "sank into the floor: {low}");
    assert!(
        centroid(&b[0]).y > 0.6 && centroid(&b[0]).y < 0.9,
        "centroid {}",
        centroid(&b[0]).y
    );
    assert!(b[0].max_strain() < 0.25, "strain {}", b[0].max_strain());
}

#[test]
fn a_soft_ball_bounces() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let (v, t) = uv_sphere(0.15, 12, 8, v3(0.0, -0.5, 0.0));
    let mut b = [SoftBody3::from_surface(
        &v,
        &t,
        &groups(&t),
        &SurfaceSpec {
            density: 1.0,
            edge_compliance: 1e-6,
            bend_compliance: 1e-4,
            volume: Some((1.0, 1e-7)),
        },
        SoftParams {
            radius: 0.015,
            restitution: 0.85,
            ..SoftParams::default()
        },
    )];
    let mut hit = false;
    let mut top_after = f64::MAX;
    for _ in 0..(3.0 / DT) as usize {
        soft_step(&mut w, &mut b, DT, 16, 4);
        let y = centroid(&b[0]).y;
        if y > 0.8 {
            hit = true;
        }
        if hit {
            top_after = top_after.min(y);
        }
    }
    assert!(hit, "never reached the floor");
    assert!(
        top_after < 0.72,
        "never bounced back up (min y after impact {top_after})"
    );
}

#[test]
fn a_hanging_cloth_stays_pinned_and_hangs_down() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    // a vertical sheet in the xy plane: rows run down
    let n = 10;
    let mut verts = vec![];
    for j in 0..n {
        for i in 0..n {
            verts.push(v3(i as f64 * 0.1, j as f64 * 0.1, 0.0));
        }
    }
    let mut tris = vec![];
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = j * n + i;
            tris.push([a, a + 1, a + n]);
            tris.push([a + 1, a + n + 1, a + n]);
        }
    }
    let mut b = [SoftBody3::from_surface(
        &verts,
        &tris,
        &groups(&tris),
        &SurfaceSpec {
            density: 0.3,
            edge_compliance: 1e-6,
            bend_compliance: 1e-2,
            volume: None,
        },
        SoftParams::default(),
    )];
    for i in 0..n {
        b[0].pin(i, verts[i]);
    }
    run(&mut w, &mut b, 3.0);
    for i in 0..n {
        assert!((b[0].pos[i] - verts[i]).len() < 1e-9, "pin moved");
    }
    let bottom = (0..n).map(|i| b[0].pos[(n - 1) * n + i].y).sum::<f64>() / n as f64;
    assert!(bottom > 0.85 && bottom < 1.15, "bottom row at {bottom}");
    assert!(b[0].max_strain() < 0.12, "strain {}", b[0].max_strain());
}

#[test]
fn a_cloth_drapes_over_a_sphere_without_going_through() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let mut s = BodyDef::new(Shape::Sphere(0.3));
    s.kind = BodyKind::Static;
    s.position = v3(0.0, 0.0, 0.0);
    w.create_body(&s);
    floor(&mut w, 0.9);
    let (v, t) = sheet(18, 1.2, -0.45, 0.0, 0.0);
    let mut b = [SoftBody3::from_surface(
        &v,
        &t,
        &groups(&t),
        &SurfaceSpec {
            density: 0.3,
            edge_compliance: 1e-6,
            bend_compliance: 1e-2,
            volume: None,
        },
        SoftParams {
            radius: 0.012,
            ..SoftParams::default()
        },
    )];
    run(&mut w, &mut b, 3.0);
    let nearest = (0..b[0].points.len())
        .map(|i| b[0].point_pos(i).len())
        .fold(f64::MAX, f64::min);
    assert!(
        nearest > 0.3 - 0.02,
        "a vertex went inside the sphere: {nearest}"
    );
    // it has draped: the middle is on top of the sphere, the corners have fallen
    let top = (0..b[0].points.len())
        .map(|i| b[0].point_pos(i).y)
        .fold(f64::MAX, f64::min);
    let low = (0..b[0].points.len())
        .map(|i| b[0].point_pos(i).y)
        .fold(f64::MIN, f64::max);
    assert!(top < -0.25 && low > 0.3, "top {top} low {low}");
}

#[test]
fn solid_jelly_is_tetrahedralized_and_keeps_its_volume() {
    let (v, t) = uv_sphere(0.3, 14, 9, v3(0.0, 0.0, 0.0));
    let tets = tetrahedralize(&v, &t, 8).unwrap();
    assert!(tets.tets.len() > 100);
    let vol: f64 = tets
        .tets
        .iter()
        .map(|n| {
            let (a, b, c, d) = (
                tets.nodes[n[0]],
                tets.nodes[n[1]],
                tets.nodes[n[2]],
                tets.nodes[n[3]],
            );
            (b - a).cross(c - a).dot(d - a) / 6.0
        })
        .sum();
    let sphere = 4.0 / 3.0 * std::f64::consts::PI * 0.3f64.powi(3);
    assert!(
        vol > sphere && vol < 2.2 * sphere,
        "tet volume {vol} vs sphere {sphere}"
    );
    // every surface vertex is held (weights sum to one, none far outside)
    for p in &tets.points {
        let s: f64 = p.w.iter().sum();
        assert!((s - 1.0).abs() < 1e-9);
        assert!(p.w.iter().all(|w| *w > -0.6), "far outside: {:?}", p.w);
    }
}

#[test]
fn a_solid_jelly_block_squashes_and_springs_back() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    let (v, t) = uv_sphere(0.25, 14, 9, v3(0.0, 0.3, 0.0));
    let mut b = [SoftBody3::from_solid(
        &v,
        &t,
        &groups(&t),
        8,
        1.0,
        1e-5,
        1.0,
        SoftParams {
            radius: 0.012,
            restitution: 0.3,
            ..SoftParams::default()
        },
    )
    .unwrap()];
    run(&mut w, &mut b, 3.0);
    let low = (0..b[0].points.len())
        .map(|i| b[0].point_pos(i).y)
        .fold(f64::MIN, f64::max);
    assert!(low < 1.03, "sank into the floor: {low}");
    let top = (0..b[0].points.len())
        .map(|i| b[0].point_pos(i).y)
        .fold(f64::MAX, f64::min);
    // a 0.5 m jelly settles somewhat lower than its undeformed height but is clearly still solid
    assert!(low - top > 0.38, "collapsed to {}", low - top);
}

#[test]
fn a_heavy_box_is_held_up_by_a_soft_pad() {
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    floor(&mut w, 1.0);
    // a soft pad: a flat slab 0.8 x 0.3 x 0.8 sitting on the floor
    let (x, y0, z) = (0.4, 0.7, 0.4);
    let mut v = vec![];
    for yy in [y0, 1.0] {
        for (xx, zz) in [(-x, -z), (x, -z), (x, z), (-x, z)] {
            v.push(v3(xx, yy, zz));
        }
    }
    let t: Vec<[usize; 3]> = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let mut b = [SoftBody3::from_solid(
        &v,
        &t,
        &groups(&t),
        8,
        1.0,
        3e-6,
        1.0,
        SoftParams {
            radius: 0.012,
            ..SoftParams::default()
        },
    )
    .unwrap()];
    let h = 0.1;
    let mut pts = vec![];
    for x in [-h, h] {
        for y in [-h, h] {
            for z in [-h, h] {
                pts.push(v3(x, y, z));
            }
        }
    }
    let mut def = BodyDef::new(Shape::Hull(Hull::from_points(&pts).unwrap()));
    def.position = v3(0.0, 0.5, 0.0);
    def.density = 1.0;
    let cube = w.create_body(&def);
    run(&mut w, &mut b, 4.0);
    let y = w.body(cube).p.y;
    // the pad's top is near 0.85 - 0.15 = 0.7 (pad spans 0.7..1.0); the cube's centre stays above it
    assert!(y < 0.8 && y > 0.45, "box centre at {y}");
    assert!(
        w.body(cube).v.len() < 0.5,
        "box still moving at {}",
        w.body(cube).v.len()
    );
}

#[test]
fn cutting_a_pinned_sheet_lets_the_lower_part_fall() {
    let n = 12;
    let mut verts = vec![];
    for j in 0..n {
        for i in 0..n {
            verts.push(v3(i as f64 * 0.1, j as f64 * 0.1, 0.0));
        }
    }
    let mut tris = vec![];
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = j * n + i;
            tris.push([a, a + 1, a + n]);
            tris.push([a + 1, a + n + 1, a + n]);
        }
    }
    let build = || {
        let mut b = SoftBody3::from_surface(
            &verts,
            &tris,
            &groups(&tris),
            &SurfaceSpec {
                density: 0.3,
                edge_compliance: 1e-6,
                bend_compliance: 1e-2,
                volume: None,
            },
            SoftParams::default(),
        );
        for i in 0..n {
            b.pin(i, verts[i]);
        }
        b
    };
    let mut w = World::new(v3(0.0, 9.8, 0.0));
    let mut whole = [build()];
    run(&mut w, &mut whole, 1.0);
    let mut cutb = [build()];
    let mut w2 = World::new(v3(0.0, 9.8, 0.0));
    run(&mut w2, &mut cutb, 0.5);
    // slice straight across, a third of the way down
    let removed = cutb[0].cut(v3(-0.1, 0.45, 0.0), v3(1.3, 0.45, 0.0), 0.02);
    assert!(!removed.is_empty());
    run(&mut w2, &mut cutb, 1.5);
    let bottom = |b: &SoftBody3| (0..n).map(|i| b.pos[(n - 1) * n + i].y).sum::<f64>() / n as f64;
    assert!(
        bottom(&cutb[0]) > bottom(&whole[0]) + 1.0,
        "cut {} vs whole {}",
        bottom(&cutb[0]),
        bottom(&whole[0])
    );
}

#[test]
fn identical_runs_give_identical_bits() {
    let build = || {
        let mut w = World::new(v3(0.0, 9.8, 0.0));
        floor(&mut w, 1.0);
        let b = [balloon(v3(0.0, 0.2, 0.0), 0.25, 1.0)];
        (w, b)
    };
    let (mut w1, mut b1) = build();
    let (mut w2, mut b2) = build();
    run(&mut w1, &mut b1, 1.5);
    run(&mut w2, &mut b2, 1.5);
    assert_eq!(hash(&b1[0]), hash(&b2[0]));
}
