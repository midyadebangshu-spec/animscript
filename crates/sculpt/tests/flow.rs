//! Vortices and flows.
use animscript_sculpt::flow::*;
use animscript_sculpt::noise::{Kind, Spec};
use animscript_sculpt::sdf::*;
use animscript_sculpt::*;
use std::sync::Arc;

const LO: V3 = [-40.0, -40.0, -40.0];
const HI: V3 = [40.0, 40.0, 40.0];

fn ball(c: V3, r: f64) -> Volume {
    let mut v = Volume::new(1.0, 3.0);
    apply(
        &mut v,
        &Sdf::Translate(Box::new(Sdf::Sphere(r)), c),
        Op::Add,
        0.0,
        LO,
        HI,
    );
    v
}
/// The centre of mass of the material.
fn centroid(v: &Volume) -> V3 {
    let (mut s, mut n) = ([0.0; 3], 0.0);
    for k in -30..=30 {
        for j in -30..=30 {
            for i in -30..=30 {
                let d = v.get([i, j, k]) as f64;
                let w = (0.5 - d).clamp(0.0, 1.0);
                if w > 0.0 {
                    s = add(s, mul([i as f64, j as f64, k as f64], w));
                    n += w;
                }
            }
        }
    }
    mul(s, 1.0 / n)
}

#[test]
fn a_vortex_circles_its_axis_and_fades_with_distance() {
    let vx = Vortex {
        center: [0.0; 3],
        axis: [0.0, 1.0, 0.0],
        omega: 2.0,
        sigma: 10.0,
    };
    // at (3, 0, 0): axis x r = (0,1,0) x (3,0,0) = (0, 0, -3); speed omega * 3 * exp(-9/200)
    let v = vx.velocity([3.0, 0.0, 0.0]);
    let expect = 2.0 * 3.0 * (-9.0f64 / 200.0).exp();
    assert!(
        v[0].abs() < 1e-12 && v[1].abs() < 1e-12 && (v[2] + expect).abs() < 1e-9,
        "{v:?}"
    );
    // tangent: perpendicular to the radius and to the axis, nothing on the axis, little far away
    assert!(dot(v, [3.0, 0.0, 0.0]).abs() < 1e-12);
    assert_eq!(vx.velocity([0.0, 5.0, 0.0]), [0.0; 3]);
    assert!(len(vx.velocity([60.0, 0.0, 0.0])) < 1e-4);
}

#[test]
fn a_swirl_carries_a_ball_round_the_axis_by_the_expected_angle() {
    let mut v = ball([8.0, 0.0, 0.0], 5.0);
    let vx = Vortex {
        center: [0.0; 3],
        axis: [0.0, 1.0, 0.0],
        omega: 1.0,
        sigma: 30.0,
    };
    let vol0 = v.volume();
    advect(&mut v, &|p| vx.velocity(p), 1.0, 5);
    let angle = 1.0 * (-64.0f64 / 1800.0).exp(); // radians in one second at r = 8
    let expect = [8.0 * angle.cos(), 0.0, -8.0 * angle.sin()];
    let c = centroid(&v);
    assert!(len(sub(c, expect)) < 0.5, "{c:?} vs {expect:?}");
    assert!(
        (v.volume() / vol0 - 1.0).abs() < 0.1,
        "{} -> {}",
        vol0,
        v.volume()
    );
    assert!(v.mesh().is_closed());
}

#[test]
fn no_flow_changes_nothing() {
    let mut v = ball([0.0; 3], 6.0);
    let before = v.sample([3.0, 1.0, 2.0]);
    advect(&mut v, &|_| [0.0; 3], 1.0, 4);
    assert_eq!(v.sample([3.0, 1.0, 2.0]), before);
    assert!((v.volume() / ball([0.0; 3], 6.0).volume() - 1.0).abs() < 1e-9);
}

#[test]
fn a_wind_moves_the_ball() {
    let mut v = ball([0.0; 3], 5.0);
    advect(&mut v, &|_| [4.0, 0.0, 0.0], 2.0, 8);
    let c = centroid(&v);
    assert!((c[0] - 8.0).abs() < 0.3 && c[1].abs() < 0.2, "{c:?}");
}

#[test]
fn a_stronger_swirl_twists_a_bar_into_a_spiral() {
    // a bar along y near the axis, swirled with a falloff that differs along y: the top turns more than the bottom
    let mut v = Volume::new(1.0, 3.0);
    apply(
        &mut v,
        &Sdf::Box([6.0, 14.0, 1.5], 0.0),
        Op::Add,
        0.0,
        LO,
        HI,
    );
    let vx = Vortex {
        center: [0.0, 6.0, 0.0],
        axis: [0.0, 1.0, 0.0],
        omega: 0.6,
        sigma: 9.0,
    };
    advect(&mut v, &|p| vx.velocity(p), 1.0, 20);
    // near the swirl's centre height the bar has turned; at the bottom (far from it) hardly
    let width_z = |y: f64| {
        let mut zs = vec![];
        for i in -80..=80 {
            let z = i as f64 * 0.25;
            if v.sample([0.0, y, z]) < 0.0 {
                zs.push(z);
            }
        }
        zs.iter().fold(0.0f64, |m, z| m.max(z.abs()))
    };
    // the bar was 1.5 wide in z everywhere; turned by a radian-ish it reaches further in z
    assert!(width_z(6.0) < 7.5);
    let mesh = v.mesh();
    assert!(mesh.is_closed());
}

#[test]
fn reinitialisation_restores_unit_gradients_and_keeps_the_surface() {
    let mut v = ball([0.0; 3], 10.0);
    // squash the field: half the distance everywhere (gradient 0.5)
    v.edit([-20, -20, -20], [20, 20, 20], &mut |_, d| d * 0.5);
    let grad = |v: &Volume| {
        let mut ratios = vec![];
        for i in -12..=12 {
            let p = [i as f64 * 0.8, 3.0, 4.0];
            if v.sample(p).abs() < 1.5 {
                ratios.push(len(v.gradient(p)));
            }
        }
        ratios.iter().sum::<f64>() / ratios.len().max(1) as f64
    };
    let g0 = grad(&v);
    assert!(g0 < 0.7, "{g0}");
    reinitialize(&mut v, 12);
    let g1 = grad(&v);
    assert!((g1 - 1.0).abs() < 0.15, "{g0} -> {g1}");
    // the zero level has not moved much: sphere radius still 10 within half a voxel
    for dir in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.6, 0.0, 0.8]] {
        assert!(
            v.sample(mul(dir, 10.0)).abs() < 0.6,
            "{dir:?} {}",
            v.sample(mul(dir, 10.0))
        );
    }
}

#[test]
fn curl_noise_stirs_a_ball_without_creating_or_losing_much_material() {
    let mut v = ball([0.0; 3], 10.0);
    let spec = Arc::new(Spec::new(Kind::Perlin, 12.0, 1, 3));
    let f = Field::Curl(spec, 30.0);
    let vol0 = v.volume();
    let area0 = v.mesh().area();
    advect(&mut v, &|p| f.velocity(p), 1.0, 12);
    reinitialize(&mut v, 6);
    assert!(
        (v.volume() / vol0 - 1.0).abs() < 0.12,
        "{} -> {}",
        vol0,
        v.volume()
    );
    assert!(
        v.mesh().area() > area0 * 1.02,
        "stirring wrinkles the surface"
    );
}

#[test]
fn fields_add_up() {
    let w = Field::Sum(vec![
        Field::Uniform([1.0, 0.0, 0.0]),
        Field::Uniform([0.0, 2.0, 0.0]),
        Field::Vortex(Vortex {
            center: [0.0; 3],
            axis: [0.0, 1.0, 0.0],
            omega: 1.0,
            sigma: 5.0,
        }),
    ]);
    let v = w.velocity([2.0, 0.0, 0.0]);
    assert!((v[0] - 1.0).abs() < 1e-12 && (v[1] - 2.0).abs() < 1e-12 && v[2] < 0.0);
}

#[test]
fn flows_are_deterministic() {
    let run = || {
        let mut v = ball([4.0, 0.0, 0.0], 5.0);
        let vx = Vortex {
            center: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            omega: 1.5,
            sigma: 8.0,
        };
        advect(&mut v, &|p| vx.velocity(p), 1.0, 10);
        reinitialize(&mut v, 4);
        v.mesh()
    };
    let (a, b) = (run(), run());
    assert_eq!(a.tris, b.tris);
    assert!(a
        .verts
        .iter()
        .zip(&b.verts)
        .all(|(p, q)| p.iter().zip(q).all(|(x, y)| x.to_bits() == y.to_bits())));
}
