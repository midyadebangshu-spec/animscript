//! Freehand: fitting hand-drawn points to Beziers and the point-list helpers, in the language.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 24\n\n";

fn go(s: &str) -> RunResult {
    let text = format!("{HEAD}{s}");
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(s: &str) -> Box<Diagnostic> {
    match run(&format!("{HEAD}{s}"), &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => Box::new(d),
    }
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn scene(body: &str) -> String {
    format!("scene \"s\" for 2.frames:\n    main:\n{body}        wait 2.frames\n")
}
/// A hand-drawn-looking circle of 120 points (a deterministic wobble) as AnimScript source.
fn circle_src(noise: f64) -> String {
    let pts: Vec<String> = (0..=120)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 120.0;
            let w = noise * ((i * 7919 % 13) as f64 / 13.0 - 0.5) * 2.0;
            format!(
                "({:.2}, {:.2})",
                200.0 + (80.0 + w) * a.cos(),
                150.0 + (80.0 + w) * a.sin()
            )
        })
        .collect();
    format!("[{}]", pts.join(", "))
}
fn edges(snap: &Value) -> &Vec<Value> {
    snap["shapes"][0]["edges"].as_array().unwrap()
}

#[test]
fn freehand_makes_a_shape_of_a_few_cubic_edges() {
    let r = go(&format!(
        "let pts = {}\nshape s = freehand(pts, tolerance: 3, closed: true)\n{}",
        circle_src(1.0),
        scene("")
    ));
    let e = edges(&r.snapshots[0]);
    assert!(e.len() >= 2 && e.len() <= 8, "{} edges", e.len());
    assert!(e.iter().all(|e| e["kind"] == "cubic"));
    assert_eq!(
        r.snapshots[0]["shapes"][0]["faces"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "closed: a face"
    );
}

#[test]
fn an_open_freehand_path_has_no_face_and_keeps_its_ends() {
    let r = go(&format!(
        "shape s = freehand([(10, 10), (60, 40), (110, 20), (160, 60)])\n{}",
        scene("")
    ));
    assert_eq!(
        r.snapshots[0]["shapes"][0]["faces"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let v = r.snapshots[0]["shapes"][0]["vertices"].as_array().unwrap();
    assert_eq!(
        (v[0][1].as_f64().unwrap(), v[0][2].as_f64().unwrap()),
        (10.0, 10.0)
    );
    let last = v.last().unwrap();
    assert_eq!(
        (last[1].as_f64().unwrap(), last[2].as_f64().unwrap()),
        (160.0, 60.0)
    );
}

#[test]
fn the_tolerance_and_the_corner_angle_change_the_result() {
    let sq = "[(100, 100), (150, 100), (200, 100), (200, 150), (200, 200), (150, 200), (100, 200), (100, 150), (100, 100)]";
    let sharp = go(&format!(
        "shape s = freehand({sq}, closed: true, corner: 40.deg)\n{}",
        scene("")
    ));
    assert_eq!(edges(&sharp.snapshots[0]).len(), 4, "corners kept");
    let tight = go(&format!(
        "let p = {}\nshape s = freehand(p, tolerance: 0.3, closed: true)\n{}",
        circle_src(1.5),
        scene("")
    ));
    let loose = go(&format!(
        "let p = {}\nshape s = freehand(p, tolerance: 4, closed: true)\n{}",
        circle_src(1.5),
        scene("")
    ));
    assert!(edges(&tight.snapshots[0]).len() >= edges(&loose.snapshots[0]).len());
}

#[test]
fn a_freehand_shape_is_a_shape_it_can_be_filled_and_moved() {
    let r = go(&format!(
        "let pts = {}\nshape s = freehand(pts, closed: true)\ns.fill = #cc3355\n{}",
        circle_src(0.5),
        scene("        s.move_by(20, 0)\n")
    ));
    assert_eq!(r.snapshots[1]["shapes"][0]["faces"][0]["fill"], "#cc3355");
}

#[test]
fn smooth_resample_simplify_and_path_length() {
    let r = go(&format!("let pts = {}\nlet sm = smooth(pts, tolerance: 2, closed: true, spacing: 10)\nlet rs = resample([(0, 0), (100, 0)], 10)\nlet sp = simplify([(0, 0), (10, 0.2), (20, -0.1), (30, 0)], 1)\nshape probe = square(size: 2, at: (0, 0))\nprobe.position = (len(sm), len(rs))\nshape probe2 = square(size: 2, at: (0, 0))\nprobe2.position = (len(sp), path_length([(0, 0), (30, 40)]))\n{}", circle_src(1.0), scene("")));
    let c = |i: usize| {
        (
            r.snapshots[0]["shapes"][i]["vertices"][0][1]
                .as_f64()
                .unwrap()
                + 1.0,
            r.snapshots[0]["shapes"][i]["vertices"][0][2]
                .as_f64()
                .unwrap()
                + 1.0,
        )
    };
    let (smooth_n, resample_n) = c(0);
    assert!(
        (smooth_n - 52.0).abs() <= 4.0,
        "a circle of circumference 503 at spacing 10: {smooth_n}"
    );
    near(resample_n, 11.0, 1e-9);
    let (simplified, length) = c(1);
    near(simplified, 2.0, 1e-9);
    near(length, 50.0, 1e-9);
}

#[test]
fn points_along_an_edge_and_outline_of_a_shape_feed_strokes() {
    let src = format!("let pts = {}\nshape s = freehand(pts, closed: true)\nlet ink = brush(\"pen\", radius: 2)\nshape art = paint_layer()\nart.stroke(ink, outline(s, spacing: 5))\nart.stroke(ink, points_along(s.e1, spacing: 3))\n{}", circle_src(1.0), scene(""));
    let r = go(&src);
    let strokes = r.snapshots[0]["shapes"][1]["paint"]["strokes"]
        .as_array()
        .unwrap();
    assert_eq!(strokes.len(), 2);
    let n = strokes[0]["points"].as_array().unwrap().len();
    assert!(
        (n as i64 - 100).abs() <= 6,
        "circumference 503 at spacing 5: {n}"
    ); // about 100 points
    assert!(strokes[1]["points"].as_array().unwrap().len() > 5);
}

#[test]
fn jitter_is_seeded_and_bounded() {
    let r = go(&format!("let line = resample([(0, 0), (300, 0)], 5)\nlet a = jitter(line, amount: 2, seed: \"a\")\nlet b = jitter(line, amount: 2, seed: \"a\")\nlet c = jitter(line, amount: 2, seed: \"c\")\nshape p = square(size: 2, at: (0, 0))\nshape q = square(size: 2, at: (0, 0))\nshape w = square(size: 2, at: (0, 0))\np.position = (a[10].x, a[10].y)\nq.position = (b[10].x, b[10].y)\nw.position = (c[10].x, c[10].y)\n{}", scene("")));
    let pos = |i: usize| {
        (
            r.snapshots[0]["shapes"][i]["vertices"][0][1]
                .as_f64()
                .unwrap()
                + 1.0,
            r.snapshots[0]["shapes"][i]["vertices"][0][2]
                .as_f64()
                .unwrap()
                + 1.0,
        )
    };
    assert_eq!(pos(0), pos(1), "the same seed gives the same wobble");
    assert_ne!(pos(0), pos(2), "another seed another wobble");
    assert!(
        (pos(0).0 - 50.0).abs() <= 2.0 + 1e-9 && pos(0).1.abs() <= 2.0 + 1e-9,
        "within the amount: {:?}",
        pos(0)
    );
}

#[test]
fn freehand_errors() {
    assert_eq!(err("shape s = freehand([(1, 1)])\n").code, "E-FRE-001");
    assert_eq!(
        err("shape s = freehand([(1, 1), (1, 1)])\n").code,
        "E-FRE-001"
    );
    assert_eq!(
        err("shape s = freehand([(0, 0), (9, 9)], tolerance: 0)\n").code,
        "E-FRE-001"
    );
    assert_eq!(
        err("let a = resample([(0, 0), (9, 9)], -1)\n").code,
        "E-FRE-001"
    );
    assert_eq!(err("let a = smooth(5)\n").code, "E-FRE-001");
    assert_eq!(
        err("shape q = square(size: 4, at: (1, 1))\nlet a = points_along(q, spacing: 2)\n").code,
        "E-FRE-001"
    );
}

#[test]
fn freehand_is_deterministic() {
    let src = format!(
        "let pts = {}\nshape s = freehand(pts, closed: true)\n{}",
        circle_src(1.5),
        scene("")
    );
    assert_eq!(go(&src).snapshots[0], go(&src).snapshots[0]);
}
