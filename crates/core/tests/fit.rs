//! Fitting hand-drawn point lists with cubic Beziers (Schneider's algorithm) and the helpers around it. Written first.
use animscript_core::*;

fn circle_pts(n: usize, r: f64, noise: f64) -> Vec<P> {
    (0..=n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            // a deterministic wobble, not random
            let w = noise * ((i * 7919 % 13) as f64 / 13.0 - 0.5) * 2.0;
            [200.0 + (r + w) * a.cos(), 200.0 + (r + w) * a.sin()]
        })
        .collect()
}
/// The distance from `p` to the fitted curve, by dense sampling.
fn dist_to_curve(p: P, segs: &[FitSeg]) -> f64 {
    let mut best = f64::MAX;
    for s in segs {
        for k in 0..=3000 {
            let q = s.at(k as f64 / 3000.0);
            best = best.min(((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt());
        }
    }
    best
}
fn max_error(pts: &[P], segs: &[FitSeg]) -> f64 {
    pts.iter()
        .map(|p| dist_to_curve(*p, segs))
        .fold(0.0, f64::max)
}
fn dir(a: P, b: P) -> P {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = d[0].hypot(d[1]);
    [d[0] / l, d[1] / l]
}

#[test]
fn a_circle_needs_few_curves_and_stays_within_the_tolerance() {
    let pts = circle_pts(160, 100.0, 0.0);
    let segs = fit_curve(&pts, 1.0, 40.0, true);
    assert!(segs.len() <= 6, "a circle in {} pieces", segs.len());
    assert!(
        max_error(&pts, &segs) <= 1.0 + 1e-6,
        "error {}",
        max_error(&pts, &segs)
    );
}

#[test]
fn a_noisy_circle_is_smoothed_not_followed() {
    let pts = circle_pts(200, 100.0, 1.5);
    let segs = fit_curve(&pts, 3.0, 40.0, true);
    assert!(segs.len() <= 8, "{} pieces for a wobbly circle", segs.len());
    assert!(max_error(&pts, &segs) <= 3.0 + 1e-6);
}

#[test]
fn a_straight_line_is_one_piece_and_noise_below_the_tolerance_is_ignored() {
    let line: Vec<P> = (0..=50)
        .map(|i| [10.0 + i as f64 * 4.0, 50.0 + (i % 3) as f64 * 0.4 - 0.4])
        .collect();
    let segs = fit_curve(&line, 2.0, 40.0, false);
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].p0, line[0]);
    assert_eq!(segs[0].p3, *line.last().unwrap());
}

#[test]
fn corners_are_kept() {
    // a square drawn with 25 points per side
    let mut pts: Vec<P> = vec![];
    for (a, b) in [
        ([100.0, 100.0], [300.0, 100.0]),
        ([300.0, 100.0], [300.0, 300.0]),
        ([300.0, 300.0], [100.0, 300.0]),
        ([100.0, 300.0], [100.0, 100.0]),
    ] {
        for i in 0..25 {
            let t = i as f64 / 25.0;
            pts.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    pts.push([100.0, 100.0]);
    let segs = fit_curve(&pts, 2.0, 40.0, true);
    assert_eq!(segs.len(), 4, "one piece per side");
    for corner in [
        [300.0, 100.0],
        [300.0, 300.0],
        [100.0, 300.0],
        [100.0, 100.0],
    ] {
        assert!(
            segs.iter().any(|s| s.p3 == corner || s.p0 == corner),
            "{corner:?} stays a vertex"
        );
    }
    // with the corner angle above 90 degrees the corners are rounded off: every join is smooth
    let round = fit_curve(&pts, 8.0, 120.0, true);
    for i in 0..round.len() {
        let (a, b) = (&round[i], &round[(i + 1) % round.len()]);
        let (d1, d2) = (dir(a.c2, a.p3), dir(b.p0, b.c1));
        assert!(d1[0] * d2[0] + d1[1] * d2[1] > 0.999, "kink at join {i}");
    }
}

#[test]
fn a_smooth_closed_curve_joins_smoothly() {
    let segs = fit_curve(&circle_pts(160, 100.0, 0.0), 1.0, 40.0, true);
    for i in 0..segs.len() {
        let (a, b) = (&segs[i], &segs[(i + 1) % segs.len()]);
        assert!(
            ((a.p3[0] - b.p0[0]).powi(2) + (a.p3[1] - b.p0[1]).powi(2)).sqrt() < 1e-9,
            "the pieces connect"
        );
        let (d1, d2) = (dir(a.c2, a.p3), dir(b.p0, b.c1));
        assert!(
            d1[0] * d2[0] + d1[1] * d2[1] > 0.999,
            "the join {i} has no kink"
        );
    }
}

#[test]
fn an_open_curve_runs_from_the_first_point_to_the_last() {
    let wave: Vec<P> = (0..=120)
        .map(|i| [i as f64 * 4.0, 100.0 + 40.0 * (i as f64 * 0.1).sin()])
        .collect();
    let segs = fit_curve(&wave, 1.5, 40.0, false);
    assert_eq!(segs[0].p0, wave[0]);
    assert_eq!(segs.last().unwrap().p3, *wave.last().unwrap());
    assert!(segs.len() < 12);
    assert!(max_error(&wave, &segs) <= 1.5 + 1e-6);
}

#[test]
fn a_tighter_tolerance_uses_more_pieces() {
    let pts = circle_pts(200, 100.0, 1.5);
    assert!(fit_curve(&pts, 0.5, 40.0, true).len() >= fit_curve(&pts, 4.0, 40.0, true).len());
}

#[test]
fn degenerate_input_is_handled() {
    assert!(fit_curve(&[], 2.0, 40.0, false).is_empty());
    assert!(fit_curve(&[[5.0, 5.0]], 2.0, 40.0, false).is_empty());
    let two = fit_curve(&[[0.0, 0.0], [10.0, 0.0]], 2.0, 40.0, false);
    assert_eq!(two.len(), 1);
    let same = fit_curve(&[[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]], 2.0, 40.0, false);
    assert!(same.is_empty(), "no extent, nothing to draw");
    // repeated points are dropped
    let dup = fit_curve(
        &[[0.0, 0.0], [0.0, 0.0], [10.0, 0.0], [10.0, 0.0]],
        2.0,
        40.0,
        false,
    );
    assert_eq!(dup.len(), 1);
}

#[test]
fn fitting_is_deterministic() {
    let pts = circle_pts(200, 100.0, 1.5);
    assert_eq!(
        fit_curve(&pts, 2.0, 40.0, true),
        fit_curve(&pts, 2.0, 40.0, true)
    );
}

#[test]
fn simplify_keeps_the_shape_with_fewer_points() {
    let wave: Vec<P> = (0..=100)
        .map(|i| [i as f64 * 3.0, 50.0 + 20.0 * (i as f64 * 0.2).sin()])
        .collect();
    let s = simplify(&wave, 1.0);
    assert!(s.len() < wave.len() / 2 && s.len() > 4);
    assert_eq!(s[0], wave[0]);
    assert_eq!(*s.last().unwrap(), *wave.last().unwrap());
    let line: Vec<P> = (0..=10).map(|i| [i as f64, 0.0]).collect();
    assert_eq!(simplify(&line, 0.1).len(), 2);
}

#[test]
fn resample_spaces_points_evenly_along_the_path() {
    let path = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0]];
    assert!((path_length(&path) - 150.0).abs() < 1e-9);
    let r = resample(&path, 10.0);
    assert_eq!(r.len(), 16);
    assert_eq!(r[0], [0.0, 0.0]);
    assert_eq!(*r.last().unwrap(), [100.0, 50.0]);
    for w in r.windows(2) {
        let d = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
        assert!((d - 10.0).abs() < 1e-6, "{d}");
    }
    assert_eq!(resample(&[[1.0, 1.0]], 5.0), vec![[1.0, 1.0]]);
}

#[test]
fn a_fitted_path_becomes_a_shape_of_cubic_edges() {
    let segs = fit_curve(&circle_pts(160, 100.0, 0.0), 1.0, 40.0, true);
    let s = bezier_shape(&segs, true);
    assert_eq!(s.live_ids(Kind::E).len(), segs.len());
    assert_eq!(s.live_ids(Kind::F).len(), 1, "a closed path has a face");
    assert!(s
        .live_ids(Kind::E)
        .iter()
        .all(|e| matches!(s.edges[e].curve, Curve::Cubic(..))));
    let open = bezier_shape(
        &fit_curve(&[[0.0, 0.0], [50.0, 20.0], [100.0, 0.0]], 1.0, 40.0, false),
        false,
    );
    assert_eq!(open.live_ids(Kind::F).len(), 0);
}
