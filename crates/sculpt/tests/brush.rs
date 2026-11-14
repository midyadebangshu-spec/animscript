//! The brushes: each does what its name says, measurably.
use animscript_sculpt::brush::{Brush, Falloff, Kind};
use animscript_sculpt::noise::{Kind as NoiseKind, Spec};
use animscript_sculpt::sdf::*;
use animscript_sculpt::*;
use std::sync::Arc;

const LO: V3 = [-40.0, -40.0, -40.0];
const HI: V3 = [40.0, 40.0, 40.0];

fn sphere(r: f64) -> Volume {
    let mut v = Volume::new(1.0, 3.0);
    apply(&mut v, &Sdf::Sphere(r), Op::Add, 0.0, LO, HI);
    v
}
/// A slab: solid below y = 0 (y grows down the screen, but here "up" is +y: the field is `y`).
fn slab() -> Volume {
    let mut v = Volume::new(1.0, 3.0);
    apply(
        &mut v,
        &Sdf::Plane([0.0, 1.0, 0.0], 0.0),
        Op::Replace,
        0.0,
        LO,
        HI,
    );
    v
}
/// The height of the surface above the origin plane at (x, z): the zero crossing of the field along y.
fn height(v: &Volume, x: f64, z: f64) -> f64 {
    let (mut lo, mut hi) = (-8.0, 12.0);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if v.sample([x, mid, z]) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}
fn radius_spread(m: &Mesh) -> f64 {
    let r: Vec<f64> = m.verts.iter().map(|p| len(*p)).collect();
    let mean = r.iter().sum::<f64>() / r.len() as f64;
    (r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / r.len() as f64).sqrt()
}

#[test]
fn falloffs_go_from_one_at_the_centre_to_zero_at_the_edge() {
    for f in [Falloff::Smooth, Falloff::Linear, Falloff::Sharp] {
        assert_eq!(f.w(0.0), 1.0);
        assert_eq!(f.w(1.0), 0.0);
        let mut prev = 1.1;
        for i in 0..=20 {
            let v = f.w(i as f64 / 20.0);
            assert!(v <= prev + 1e-12, "{f:?} is monotonic");
            prev = v;
        }
    }
    assert_eq!(Falloff::Constant.w(0.9), 1.0);
    assert_eq!(Falloff::Constant.w(1.0), 0.0);
    assert!(Falloff::Sharp.w(0.5) < Falloff::Smooth.w(0.5));
}

#[test]
fn strokes_place_dabs_along_the_path_at_the_spacing() {
    let mut v = sphere(14.0);
    let b = Brush::new(Kind::Inflate, 8.0, 0.1);
    // 20 long at a spacing of 2: the start, then every 2, then the end
    let n = b.stroke(&mut v, &[[0.0, 14.0, -10.0], [0.0, 14.0, 10.0]]);
    assert!((10..=12).contains(&n), "{n}");
    assert_eq!(b.stroke(&mut v, &[[0.0, 14.0, 0.0]]), 1);
    assert_eq!(b.stroke(&mut v, &[]), 0);
}

#[test]
fn draw_adds_a_blob_and_inverted_it_digs() {
    let mut v = sphere(16.0);
    let before = v.volume();
    Brush::new(Kind::Draw, 8.0, 1.0).stroke(&mut v, &[[0.0, 16.0, 0.0]]);
    assert!(v.volume() > before + 10.0, "{} {}", v.volume(), before);
    assert!(
        v.sample([0.0, 17.5, 0.0]) < 0.0,
        "the surface stands above where it was"
    );
    assert!(
        (v.sample([16.0, 0.0, 0.0])).abs() < 0.2,
        "far from the brush nothing moved"
    );
    let mut d = sphere(16.0);
    let mut b = Brush::new(Kind::Draw, 8.0, 1.0);
    b.invert = true;
    b.stroke(&mut d, &[[0.0, 16.0, 0.0]]);
    assert!(d.volume() < before - 10.0);
    assert!(d.sample([0.0, 14.5, 0.0]) > 0.0, "a dent");
}

#[test]
fn clay_builds_a_flat_layer() {
    let mut v = slab();
    let path: Vec<V3> = (-20..=20).map(|i| [i as f64, 0.0, 0.0]).collect();
    let before = v.volume();
    Brush::new(Kind::Clay, 8.0, 1.0).stroke(&mut v, &path);
    assert!(v.volume() > before);
    let hs: Vec<f64> = [-12.0, 0.0, 12.0]
        .iter()
        .map(|x| height(&v, *x, 0.0))
        .collect();
    // a raised strip of one height all along it (overlapping dabs build the layer up, each over the last)
    assert!(hs.iter().all(|h| *h > 0.9), "{hs:?}");
    assert!(
        hs.iter().cloned().fold(0.0, f64::max) - hs.iter().cloned().fold(9.0, f64::min) < 0.15,
        "flat: {hs:?}"
    );
    assert!(
        height(&v, 0.0, 20.0).abs() < 0.1,
        "away from the stroke the slab is untouched"
    );
}

#[test]
fn smooth_relaxes_a_rough_surface_without_changing_its_size() {
    let mut v = Volume::new(1.0, 3.0);
    let n = Sdf::Noise(
        Box::new(Sdf::Sphere(18.0)),
        Arc::new(Spec::new(NoiseKind::Perlin, 5.0, 1, 4)),
        2.0,
    );
    apply(&mut v, &n, Op::Add, 0.0, LO, HI);
    let (spread0, vol0) = (radius_spread(&v.mesh()), v.volume());
    let mut b = Brush::new(Kind::Smooth, 12.0, 1.0);
    b.passes = 8;
    // dabs all round the sphere: every direction of the 26 neighbours of a cube
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if (x, y, z) != (0, 0, 0) {
                    let d = norm([x as f64, y as f64, z as f64]);
                    b.stroke(&mut v, &[mul(d, 18.0)]);
                }
            }
        }
    }
    let (spread1, vol1) = (radius_spread(&v.mesh()), v.volume());
    assert!(spread1 < spread0 * 0.8, "{spread0} -> {spread1}");
    assert!((vol1 / vol0 - 1.0).abs() < 0.05, "{vol0} {vol1}");
}

#[test]
fn inflate_pushes_the_surface_out() {
    let mut v = sphere(16.0);
    let before = v.volume();
    Brush::new(Kind::Inflate, 10.0, 1.0).stroke(&mut v, &[[0.0, 16.0, 0.0]]);
    assert!(v.volume() > before);
    let top = height_along_y(&v);
    assert!(top > 16.5, "{top}");
    assert!((v.sample([-16.0, 0.0, 0.0])).abs() < 0.2);
}
fn height_along_y(v: &Volume) -> f64 {
    let (mut lo, mut hi) = (0.0, 30.0);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if v.sample([0.0, mid, 0.0]) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

#[test]
fn flatten_pulls_a_bump_onto_a_plane() {
    let mut v = slab();
    apply(
        &mut v,
        &Sdf::Translate(Box::new(Sdf::Sphere(5.0)), [0.0, 0.0, 0.0]),
        Op::Add,
        0.0,
        LO,
        HI,
    );
    let bump = height(&v, 0.0, 0.0);
    assert!(bump > 4.5);
    let mut b = Brush::new(Kind::Flatten, 10.0, 1.0);
    b.passes = 3;
    b.stroke(&mut v, &[[0.0, 0.0, 0.0]]);
    assert!(
        height(&v, 0.0, 0.0) < bump * 0.5,
        "{} -> {}",
        bump,
        height(&v, 0.0, 0.0)
    );
}

#[test]
fn scrape_cuts_away_what_stands_above_the_plane() {
    let mut v = sphere(16.0);
    let before = v.volume();
    let mut b = Brush::new(Kind::Scrape, 10.0, 1.0);
    b.passes = 3;
    b.stroke(&mut v, &[[0.0, 16.0, 0.0]]);
    assert!(v.volume() < before);
    assert!(height_along_y(&v) < 15.5);
    assert!((v.sample([-16.0, 0.0, 0.0])).abs() < 0.2);
}

#[test]
fn pinch_draws_a_bump_in_towards_the_stroke() {
    let mut v = slab();
    apply(&mut v, &Sdf::Sphere(6.0), Op::Add, 0.0, LO, HI);
    // the footprint of the bump at 0.5 above the slab
    let footprint = |v: &Volume| {
        let mut n = 0;
        for i in -12..=12 {
            for k in -12..=12 {
                if v.sample([i as f64, 0.5, k as f64]) < 0.0 {
                    n += 1;
                }
            }
        }
        n
    };
    let before = footprint(&v);
    let mut b = Brush::new(Kind::Pinch, 10.0, 1.0);
    b.passes = 4;
    b.stroke(&mut v, &[[0.0, 0.0, 0.0]]);
    let after = footprint(&v);
    assert!(after < before - 10, "{before} -> {after}");
}

#[test]
fn grease_drags_material_along_the_stroke() {
    let mut v = slab();
    apply(&mut v, &Sdf::Sphere(5.0), Op::Add, 0.0, LO, HI);
    // the mean x of the material standing above the slab
    let centroid = |v: &Volume| {
        let (mut sx, mut n) = (0.0, 0.0);
        for i in -20..=24 {
            for j in 1..=8 {
                for k in -10..=10 {
                    if v.sample([i as f64, j as f64 * 0.8, k as f64]) < 0.0 {
                        sx += i as f64;
                        n += 1.0;
                    }
                }
            }
        }
        sx / n
    };
    let before = centroid(&v);
    assert!(before.abs() < 0.5);
    let b = Brush::new(Kind::Grease, 8.0, 1.0);
    b.stroke(&mut v, &[[-2.0, 2.0, 0.0], [10.0, 2.0, 0.0]]);
    assert!(
        centroid(&v) > before + 0.8,
        "{} -> {}",
        before,
        centroid(&v)
    );
}

#[test]
fn symmetry_repeats_a_stroke_across_a_plane() {
    let mut v = sphere(16.0);
    let mut b = Brush::new(Kind::Draw, 6.0, 1.0);
    b.mirror = [true, false, false];
    let n = b.stroke(&mut v, &[[8.0, 14.0, 0.0]]);
    assert_eq!(n, 2);
    for p in [
        [8.0, 16.5, 0.0],
        [-8.0, 16.5, 0.0],
        [10.0, 12.0, 3.0],
        [-10.0, 12.0, 3.0],
    ] {
        let q = [-p[0], p[1], p[2]];
        assert!((v.sample(p) - v.sample(q)).abs() < 1e-5, "{p:?}");
    }
    assert!(v.sample([-8.0, 17.0, 0.0]) < v.sample([-2.0, 20.0, 0.0]));
}

#[test]
fn strokes_are_deterministic() {
    let make = || {
        let mut v = sphere(14.0);
        let mut b = Brush::new(Kind::Clay, 7.0, 0.8);
        b.passes = 2;
        b.stroke(
            &mut v,
            &[[-8.0, 12.0, 0.0], [8.0, 12.0, 4.0], [0.0, 14.0, 9.0]],
        );
        v.mesh()
    };
    let (a, b) = (make(), make());
    assert_eq!(a.tris, b.tris);
    assert!(a
        .verts
        .iter()
        .zip(&b.verts)
        .all(|(p, q)| p.iter().zip(q).all(|(x, y)| x.to_bits() == y.to_bits())));
}

#[test]
fn every_brush_keeps_the_surface_closed_and_valid() {
    for kind in [
        Kind::Draw,
        Kind::Clay,
        Kind::Smooth,
        Kind::Inflate,
        Kind::Flatten,
        Kind::Pinch,
        Kind::Scrape,
        Kind::Grease,
    ] {
        let mut v = sphere(14.0);
        Brush::new(kind, 7.0, 0.7).stroke(&mut v, &[[0.0, 14.0, -5.0], [0.0, 14.0, 5.0]]);
        let m = v.mesh();
        assert!(m.is_closed() && !m.tris.is_empty(), "{kind:?}");
        assert!(m.enclosed_volume() > 0.0, "{kind:?}");
    }
    assert!(
        Kind::parse("grease").is_some()
            && Kind::parse("smear").is_some()
            && Kind::parse("zzz").is_none()
    );
}

#[test]
fn a_dab_with_no_surface_in_reach_does_nothing() {
    let mut v = sphere(10.0);
    let before = v.mesh().tris.len();
    for kind in [
        Kind::Draw,
        Kind::Clay,
        Kind::Inflate,
        Kind::Flatten,
        Kind::Scrape,
    ] {
        Brush::new(kind, 5.0, 1.0).stroke(&mut v, &[[0.0, 40.0, 0.0]]);
    }
    assert_eq!(v.mesh().tris.len(), before);
    assert!(v.sample([0.0, 40.0, 0.0]) > 0.0);
}
