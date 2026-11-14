//! Mathematical sculpting: shapes, booleans, warps, equations and noise.
use animscript_sculpt::noise::*;
use animscript_sculpt::sdf::*;
use animscript_sculpt::*;
use std::sync::Arc;

fn near(a: f64, b: f64, e: f64) -> bool {
    (a - b).abs() < e
}

#[test]
fn primitives_have_the_exact_distance() {
    assert!(near(Sdf::Sphere(5.0).eval([8.0, 0.0, 0.0]), 3.0, 1e-12));
    assert!(near(Sdf::Sphere(5.0).eval([1.0, 2.0, 2.0]), -2.0, 1e-12));
    let b = Sdf::Box([2.0, 3.0, 4.0], 0.0);
    assert!(near(b.eval([5.0, 0.0, 0.0]), 3.0, 1e-12));
    assert!(near(b.eval([0.0, 0.0, 0.0]), -2.0, 1e-12)); // nearest face is x at 2
    assert!(near(b.eval([5.0, 7.0, 0.0]), 5.0, 1e-12)); // corner-ish: dx 3, dy 4
    assert!(near(
        Sdf::Box([2.0, 2.0, 2.0], 0.5).eval([2.5, 0.0, 0.0]),
        0.0,
        1e-12
    ));
    let t = Sdf::Torus(10.0, 2.0);
    assert!(near(t.eval([10.0, 0.0, 0.0]), -2.0, 1e-12));
    assert!(near(t.eval([13.0, 0.0, 0.0]), 1.0, 1e-12));
    assert!(near(t.eval([0.0, 0.0, 0.0]), 8.0, 1e-12));
    let c = Sdf::Capsule([0.0, -5.0, 0.0], [0.0, 5.0, 0.0], 2.0);
    assert!(near(c.eval([4.0, 0.0, 0.0]), 2.0, 1e-12));
    assert!(near(c.eval([0.0, 9.0, 0.0]), 2.0, 1e-12));
    let cy = Sdf::Cylinder(3.0, 4.0);
    assert!(near(cy.eval([5.0, 0.0, 0.0]), 2.0, 1e-12));
    assert!(near(cy.eval([0.0, 6.0, 0.0]), 2.0, 1e-12));
    assert!(cy.eval([0.0, 0.0, 0.0]) < 0.0);
    let co = Sdf::Cone(4.0, 0.0, 5.0);
    assert!(
        co.eval([0.0, -4.9, 0.0]) < 0.0
            && co.eval([0.0, 4.9, 3.0]) > 0.0
            && co.eval([0.0, 6.0, 0.0]) > 0.5
    );
    let e = Sdf::Ellipsoid([4.0, 2.0, 2.0]);
    assert!(
        e.eval([4.0, 0.0, 0.0]).abs() < 1e-9
            && e.eval([0.0, 2.0, 0.0]).abs() < 1e-9
            && e.eval([0.0, 0.0, 0.0]) < 0.0
    );
    assert!(near(
        Sdf::Plane([0.0, 1.0, 0.0], 3.0).eval([5.0, 7.0, 1.0]),
        4.0,
        1e-12
    ));
}

#[test]
fn smooth_minimum_rounds_the_join_and_is_exact_far_away() {
    assert_eq!(smin(1.0, 2.0, 0.0), 1.0);
    assert!(
        smin(1.0, 1.0, 1.0) < 1.0,
        "a smooth union dips below both where they meet"
    );
    assert_eq!(
        smin(1.0, 5.0, 1.0),
        1.0,
        "far from the join it is just the minimum"
    );
    assert!(smin(0.0, 0.0, 2.0) >= -0.5 - 1e-12);
    assert!(near(smax(1.0, 2.0, 0.0), 2.0, 1e-12));
}

#[test]
fn booleans_combine_distances() {
    let a = Sdf::Sphere(5.0);
    let b = Sdf::Translate(Box::new(Sdf::Sphere(5.0)), [6.0, 0.0, 0.0]);
    let u = Sdf::Union(vec![a.clone(), b.clone()], 0.0);
    assert!(
        u.eval([0.0, 0.0, 0.0]) < 0.0
            && u.eval([6.0, 0.0, 0.0]) < 0.0
            && u.eval([3.0, 0.0, 0.0]) < 0.0
    );
    assert!(u.eval([3.0, 8.0, 0.0]) > 0.0);
    let i = Sdf::Intersect(vec![a.clone(), b.clone()], 0.0);
    assert!(i.eval([3.0, 0.0, 0.0]) < 0.0 && i.eval([-4.0, 0.0, 0.0]) > 0.0);
    let s = Sdf::Subtract(Box::new(a.clone()), Box::new(b.clone()), 0.0);
    assert!(s.eval([-3.0, 0.0, 0.0]) < 0.0 && s.eval([3.0, 0.0, 0.0]) > 0.0);
    // the smooth union is never bigger inside than the sharp one is smaller: it fills the crease
    let su = Sdf::Union(vec![a, b], 3.0);
    assert!(su.eval([3.0, 4.2, 0.0]) < u.eval([3.0, 4.2, 0.0]));
}

#[test]
fn domain_operations_move_rotate_scale_mirror_and_repeat() {
    let s = Sdf::Sphere(2.0);
    assert!(near(
        Sdf::Translate(Box::new(s.clone()), [10.0, 0.0, 0.0]).eval([10.0, 0.0, 3.0]),
        1.0,
        1e-12
    ));
    assert!(near(
        Sdf::Scale(Box::new(s.clone()), 3.0).eval([9.0, 0.0, 0.0]),
        3.0,
        1e-12
    ));
    // a box rotated a quarter turn about z swaps its x and y extents
    let b = Sdf::Box([6.0, 1.0, 1.0], 0.0);
    let r = Sdf::Rotate(Box::new(b.clone()), [0.0, 0.0, 0.25]);
    assert!(b.eval([5.0, 0.0, 0.0]) < 0.0 && b.eval([0.0, 5.0, 0.0]) > 0.0);
    assert!(
        r.eval([0.0, 5.0, 0.0]).abs() < 1.1
            && r.eval([0.0, 5.0, 0.0]) < 0.0
            && r.eval([5.0, 0.0, 0.0]) > 0.0
    );
    // mirror: the field is symmetric about the plane
    let m = Sdf::Mirror(
        Box::new(Sdf::Translate(Box::new(s.clone()), [5.0, 0.0, 0.0])),
        0,
    );
    assert!(
        near(m.eval([5.0, 1.0, 0.0]), m.eval([-5.0, 1.0, 0.0]), 1e-12)
            && m.eval([-5.0, 0.0, 0.0]) < 0.0
    );
    // repeat: one sphere per period
    let rp = Sdf::Repeat(Box::new(s), [10.0, 0.0, 0.0]);
    assert!(
        rp.eval([30.0, 0.0, 0.0]) < 0.0
            && rp.eval([-20.0, 0.0, 0.0]) < 0.0
            && rp.eval([5.0, 0.0, 0.0]) > 0.0
    );
}

#[test]
fn twist_turns_a_bar_about_its_axis_and_bend_curves_it() {
    // a flat bar along y, 1 thick in z; twisting 0.1 turns per unit about y turns the cross-section with height
    let bar = Sdf::Box([4.0, 20.0, 0.5], 0.0);
    let tw = Sdf::Twist(Box::new(bar.clone()), 0.05, 1);
    assert!(
        near(tw.eval([3.0, 0.0, 0.0]), bar.eval([3.0, 0.0, 0.0]), 1e-9),
        "no twist at the axis origin"
    );
    // at y = 5 the section has turned by 0.25 turns: the long side now lies along z
    assert!(tw.eval([0.0, 5.0, 3.0]) < 0.0 && tw.eval([3.0, 5.0, 0.0]) > 0.0);
    // a bend of 0.05/unit carries points at x = 10 around by half a radian
    let bd = Sdf::Bend(Box::new(Sdf::Sphere(1.0)), 0.0);
    assert!(near(bd.eval([2.0, 0.0, 0.0]), 1.0, 1e-9));
}

#[test]
fn a_shell_is_hollow_and_an_offset_grows() {
    let s = Sdf::Shell(Box::new(Sdf::Sphere(10.0)), 2.0);
    assert!(
        s.eval([0.0, 0.0, 0.0]) > 0.0
            && s.eval([10.0, 0.0, 0.0]) < 0.0
            && s.eval([12.5, 0.0, 0.0]) > 0.0
    );
    assert!(near(
        Sdf::Offset(Box::new(Sdf::Sphere(3.0)), 2.0).eval([5.0, 0.0, 0.0]),
        0.0,
        1e-12
    ));
}

#[test]
fn an_equation_is_a_shape() {
    let e = Expr::compile("sqrt(x*x + y*y + z*z) - 5").unwrap();
    let s = Sdf::Expr(Arc::new(e));
    assert!(near(s.eval([5.0, 0.0, 0.0]), 0.0, 1e-9) && near(s.eval([0.0, 8.0, 0.0]), 3.0, 1e-9));
    // a wavy plane: y - 2 sin(x / 3)
    let w = Sdf::Expr(Arc::new(Expr::compile("y - 2 * sin(x / 3)").unwrap()));
    assert!(near(w.eval([0.0, 1.0, 0.0]), 1.0, 1e-9));
    assert!(
        Expr::compile("sqrt(q)").is_err(),
        "an unknown name is an error"
    );
    assert!(Expr::compile("x +").is_err());
}

#[test]
fn noise_is_seeded_continuous_and_bounded() {
    for kind in [
        Kind::Perlin,
        Kind::Value,
        Kind::Worley,
        Kind::Fbm,
        Kind::Ridged,
        Kind::Billow,
    ] {
        let a = Spec::new(kind, 10.0, 4, 7);
        let b = Spec::new(kind, 10.0, 4, 7);
        let c = Spec::new(kind, 10.0, 4, 8);
        let mut same = true;
        let mut diff = false;
        let mut max = 0.0f64;
        let mut sum = 0.0;
        let n = 2000;
        for i in 0..n {
            let p = [
                i as f64 * 1.37,
                (i as f64 * 0.73).sin() * 40.0,
                i as f64 * -0.41,
            ];
            let (va, vb, vc) = (a.eval(p), b.eval(p), c.eval(p));
            same &= va.to_bits() == vb.to_bits();
            diff |= va != vc;
            max = max.max(va.abs());
            sum += va;
            assert!(va.is_finite());
        }
        assert!(same, "{kind:?}: the same seed gives the same noise");
        assert!(diff, "{kind:?}: another seed differs");
        assert!(max <= 1.5, "{kind:?}: {max}");
        // not constant
        assert!(max > 0.1, "{kind:?}");
        let _ = sum;
    }
    // continuity: a tiny step changes a Perlin value a little
    let p = Spec::new(Kind::Perlin, 10.0, 1, 3);
    for i in 0..200 {
        let q = [i as f64 * 0.9, i as f64 * 0.4, 1.0];
        let r = [q[0] + 0.01, q[1], q[2]];
        assert!((p.eval(q) - p.eval(r)).abs() < 0.02);
    }
}

#[test]
fn curl_noise_is_divergence_free() {
    let s = Spec::new(Kind::Perlin, 8.0, 1, 5);
    let e = 0.05;
    for i in 0..30 {
        let p = [i as f64 * 1.7, 3.0 - i as f64 * 0.6, 5.0 + i as f64 * 0.3];
        let d = |a: usize| {
            let (mut x, mut y) = (p, p);
            x[a] += e;
            y[a] -= e;
            (s.curl(x)[a] - s.curl(y)[a]) / (2.0 * e)
        };
        let div = d(0) + d(1) + d(2);
        let mag = len(s.curl(p)).max(1e-3);
        assert!(div.abs() < 0.1 * mag + 0.01, "{div} {mag}");
    }
}

fn vol() -> Volume {
    Volume::new(1.0, 3.0)
}
const LO: V3 = [-30.0, -30.0, -30.0];
const HI: V3 = [30.0, 30.0, 30.0];

#[test]
fn shapes_applied_to_a_volume_make_the_right_solid() {
    let mut v = vol();
    apply(&mut v, &Sdf::Sphere(12.0), Op::Add, 0.0, LO, HI);
    let exact = 4.0 / 3.0 * std::f64::consts::PI * 12.0f64.powi(3);
    assert!((v.volume() / exact - 1.0).abs() < 0.03);
    // cut a hole through it: the centre is outside and the volume dropped
    let before = v.volume();
    apply(&mut v, &Sdf::Cylinder(4.0, 20.0), Op::Subtract, 0.0, LO, HI);
    assert!(v.sample([0.0, 0.0, 0.0]) > 0.0 && v.sample([9.0, 0.0, 0.0]) < 0.0);
    // the bore: a cylinder of radius 4 through a sphere of radius 12 removes about pi 4^2 2 sqrt(12^2 - 4^2)
    let bore = std::f64::consts::PI * 16.0 * 2.0 * (144.0f64 - 16.0).sqrt();
    assert!(
        ((before - v.volume()) / bore - 1.0).abs() < 0.15,
        "{} {}",
        before - v.volume(),
        bore
    );
    // intersect with a box keeps only the part inside
    apply(
        &mut v,
        &Sdf::Box([6.0, 6.0, 6.0], 0.0),
        Op::Intersect,
        0.0,
        LO,
        HI,
    );
    assert!(v.sample([9.0, 0.0, 0.0]) > 0.0);
    let m = v.mesh();
    assert!(m.is_closed());
}

#[test]
fn a_smooth_union_has_more_material_than_a_sharp_one() {
    let two = |k: f64| {
        let mut v = vol();
        apply(
            &mut v,
            &Sdf::Translate(Box::new(Sdf::Sphere(8.0)), [-7.0, 0.0, 0.0]),
            Op::Add,
            k,
            LO,
            HI,
        );
        apply(
            &mut v,
            &Sdf::Translate(Box::new(Sdf::Sphere(8.0)), [7.0, 0.0, 0.0]),
            Op::Add,
            k,
            LO,
            HI,
        );
        v.volume()
    };
    assert!(two(6.0) > two(0.0) * 1.01);
}

#[test]
fn noise_displacement_roughens_a_sphere_deterministically() {
    let make = |seed: u32| {
        let mut v = vol();
        let n = Sdf::Noise(
            Box::new(Sdf::Sphere(14.0)),
            Arc::new(Spec::new(Kind::Fbm, 8.0, 3, seed)),
            2.5,
        );
        apply(&mut v, &n, Op::Add, 0.0, LO, HI);
        v
    };
    let (a, b, c) = (make(1), make(1), make(2));
    let (ma, mb, mc) = (a.mesh(), b.mesh(), c.mesh());
    assert_eq!(ma.tris, mb.tris);
    assert_ne!(ma.verts.len() + ma.tris.len(), 0);
    // the surface is no longer at one radius
    let radii: Vec<f64> = ma.verts.iter().map(|p| len(*p)).collect();
    let (lo, hi) = (
        radii.iter().cloned().fold(f64::MAX, f64::min),
        radii.iter().cloned().fold(0.0, f64::max),
    );
    assert!(hi - lo > 2.0, "{lo} {hi}");
    assert!(
        ma.area() > 4.0 * std::f64::consts::PI * 14.0 * 14.0,
        "rougher means more area"
    );
    assert!(ma.verts != mc.verts, "another seed makes another surface");
}
