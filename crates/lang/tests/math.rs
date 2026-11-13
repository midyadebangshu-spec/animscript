//! The mathematical animation engine in the language: `drive`, `plot`, `curve`, `polar`, `implicit`.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 10\n\n";

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
/// A scene of `frames` frames whose main block is `body` (already indented 8 spaces).
fn scene(frames: u32, body: &str) -> String {
    format!("scene \"s\" for {frames}.frames:\n    main:\n{body}        wait {frames}.frames\n")
}
fn verts(snap: &Value, shape: usize) -> Vec<(f64, f64)> {
    snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect()
}

// ---- shapes from equations ----------------------------------------------------------------------

#[test]
fn plot_samples_y_of_x() {
    let r = go(&format!(
        "shape s = plot(y: \"x * 2 + 1\", from: 0, to: 10, n: 6)\n{}",
        scene(2, "")
    ));
    let v = verts(&r.snapshots[0], 0);
    assert_eq!(v.len(), 6);
    for (i, (x, y)) in v.iter().enumerate() {
        near(*x, 2.0 * i as f64, 1e-9);
        near(*y, 4.0 * i as f64 + 1.0, 1e-9);
    }
    // an open curve: five edges, no face
    assert_eq!(
        r.snapshots[0]["shapes"][0]["edges"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        r.snapshots[0]["shapes"][0]["faces"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn plot_accepts_a_lambda_and_captured_numbers() {
    let r = go(&format!(
        "let k = 3\nshape s = plot(y: (x) -> x * k, from: 0, to: 4, n: 5)\n{}",
        scene(2, "")
    ));
    let v = verts(&r.snapshots[0], 0);
    near(v[4].1, 12.0, 1e-9);
}

#[test]
fn curve_is_parametric_and_can_close() {
    let r = go(&format!(
        "shape s = curve(x: \"100 * cos(u)\", y: \"100 * sin(u)\", from: 0, to: 6.283185307179586, n: 8, closed: true)\n{}",
        scene(2, "")
    ));
    let v = verts(&r.snapshots[0], 0);
    assert_eq!(v.len(), 8);
    near(v[0].0, 100.0, 1e-9);
    near(v[2].1, 100.0, 1e-9);
    assert_eq!(
        r.snapshots[0]["shapes"][0]["faces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn polar_uses_theta() {
    let r = go(&format!(
        "shape s = polar(r: \"50\", n: 4)\n{}",
        scene(2, "")
    ));
    let v = verts(&r.snapshots[0], 0);
    assert_eq!(v.len(), 4);
    near(v[0].0, 50.0, 1e-9);
    near(v[1].0, 50.0 * (std::f64::consts::TAU / 4.0).cos(), 1e-9);
    // a polar curve closes by default
    assert_eq!(
        r.snapshots[0]["shapes"][0]["faces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn implicit_circle_is_a_closed_loop_of_the_right_radius() {
    let r = go(&format!(
        "shape s = implicit(f: \"x*x + y*y - 100*100\", x0: -150, y0: -150, x1: 150, y1: 150, res: 60)\n{}",
        scene(2, "")
    ));
    let v = verts(&r.snapshots[0], 0);
    assert!(v.len() > 40);
    for (x, y) in &v {
        near((x * x + y * y).sqrt(), 100.0, 0.5);
    }
    let e = r.snapshots[0]["shapes"][0]["edges"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        e,
        v.len(),
        "a single closed contour has as many edges as vertices"
    );
}

#[test]
fn equation_errors() {
    assert_eq!(
        err("shape s = plot(y: \"sin(\", from: 0, to: 1, n: 5)\n").code,
        "E-MTH-001"
    );
    assert_eq!(
        err("shape s = plot(y: \"foo\", from: 0, to: 1, n: 5)\n").code,
        "E-MTH-001"
    );
    assert_eq!(
        err("shape s = plot(y: \"x\", from: 0, to: 1, n: 1)\n").code,
        "E-MTH-003"
    );
    assert_eq!(
        err("shape s = plot(y: \"1 / (x - 1)\", from: 0, to: 2, n: 3)\n").code,
        "E-MTH-005"
    );
    assert_eq!(
        err("shape s = plot(y: (x) -> x.foo, from: 0, to: 1, n: 3)\n").code,
        "E-MTH-002"
    );
    assert_eq!(
        err("shape s = implicit(f: \"x\", x0: 0, y0: 0, x1: 1, y1: 1, res: 0)\n").code,
        "E-MTH-003"
    );
}

// ---- drive ---------------------------------------------------------------------------------------

const LINE: &str = "shape w = plot(y: \"0\", from: 0, to: 100, n: 11)\n";

#[test]
fn drive_adds_an_equation_to_the_vertices() {
    let src = format!(
        "{LINE}{}",
        scene(10, "        start w.drive(y: \"x / 10 + t\")\n")
    );
    let r = go(&src);
    // tick 0: t = 0; tick 5 (fps 10): t = 0.5
    let v0 = verts(&r.snapshots[0], 0);
    let v5 = verts(&r.snapshots[5], 0);
    for i in 0..11 {
        near(v0[i].1, i as f64, 1e-9);
        near(v5[i].1, i as f64 + 0.5, 1e-9);
        near(v5[i].0, 10.0 * i as f64, 1e-9);
    }
}

#[test]
fn drive_can_move_x_too_and_replace_instead_of_add() {
    let src = format!(
        "{LINE}{}",
        scene(
            4,
            "        start w.drive(x: \"x + 1\", y: \"7\", mode: \"replace\")\n"
        )
    );
    let r = go(&src);
    let v = verts(&r.snapshots[1], 0);
    for (i, p) in v.iter().enumerate() {
        near(p.0, 10.0 * i as f64 + 1.0, 1e-9);
        near(p.1, 7.0, 1e-9);
    }
}

#[test]
fn drive_knows_the_index_the_count_and_u() {
    let src = format!(
        "{LINE}{}",
        scene(2, "        start w.drive(y: \"i * 100 + n + u\")\n")
    );
    let r = go(&src);
    let v = verts(&r.snapshots[0], 0);
    near(v[3].1, 300.0 + 11.0 + 0.3, 1e-9);
    near(v[10].1, 1000.0 + 11.0 + 1.0, 1e-9);
}

#[test]
fn drive_takes_named_numbers_and_lambdas() {
    let src = format!(
        "{LINE}let amp = 4\n{}",
        scene(
            10,
            "        start w.drive(y: (x, t) -> amp * (x / 100) + t, speed: 2)\n"
        )
    );
    let r = go(&src);
    let v = verts(&r.snapshots[5], 0);
    // `speed` scales t: tick 5 is 0.5 s, so t = 1.0
    near(v[10].1, 4.0 + 1.0, 1e-9);
}

#[test]
fn keyframes_still_win_over_a_drive() {
    let src = format!(
        "{LINE}{}",
        scene(
            10,
            "        start w.drive(y: \"50\")\n        w.v2.move_to(10, 9) over 1.frames\n"
        )
    );
    let r = go(&src);
    // the plain action works on the rest value, the drive still adds on top: 9 + 50
    let v = verts(&r.snapshots[3], 0);
    near(v[1].1, 59.0, 1e-9);
    near(v[2].1, 50.0, 1e-9);
}

#[test]
fn drive_ends_after_over_and_blends() {
    let src = format!(
        "{LINE}{}",
        scene(
            10,
            "        w.drive(y: \"20\", blend_in: 0.2.seconds, blend_out: 0.2.seconds) over 0.5.seconds\n"
        )
    );
    let r = go(&src);
    near(verts(&r.snapshots[0], 0)[0].1, 0.0, 1e-9);
    near(verts(&r.snapshots[1], 0)[0].1, 10.0, 1e-9);
    near(verts(&r.snapshots[3], 0)[0].1, 20.0, 1e-9);
    near(verts(&r.snapshots[7], 0)[0].1, 0.0, 1e-9);
}

#[test]
fn drive_shape_properties() {
    let src = "shape c = circle(n: 8, radius: 20)\n".to_string()
        + &scene(10, "        start c.drive(opacity: \"-t\")\n");
    let r = go(&src);
    near(
        r.snapshots[5]["shapes"][0]["opacity"].as_f64().unwrap(),
        0.5,
        1e-9,
    );
}

#[test]
fn drive_errors() {
    let bad = |body: &str, code: &str| {
        let src = format!("{LINE}{}", scene(2, body));
        assert_eq!(err(&src).code, code, "{body}");
    };
    bad("        start w.drive()\n", "E-MTH-003");
    bad("        start w.drive(y: \"1 +\")\n", "E-MTH-001");
    bad(
        "        start w.drive(y: \"1\", x: \"2\", opacity: \"1\")\n",
        "E-MTH-004",
    );
    bad(
        "        start w.drive(y: \"1\", mode: \"nope\")\n",
        "E-MTH-003",
    );
    bad("        start w.drive(colour: \"1\")\n", "E-MTH-004");
}

#[test]
fn a_non_finite_result_drives_nothing() {
    let src = format!(
        "{LINE}{}",
        scene(3, "        start w.drive(y: \"1 / (i - 3)\")\n")
    );
    let text = format!("{HEAD}{src}");
    let r = run(
        &text,
        &RunOptions {
            trace: true,
            ..Default::default()
        },
    )
    .unwrap();
    let v = verts(&r.snapshots[0], 0);
    assert_eq!(v[3].1, 0.0);
    near(v[4].1, 1.0, 1e-9);
    assert!(r.trace.iter().any(|l| l.contains("MATH")), "{:?}", r.trace);
}

#[test]
fn runs_are_deterministic() {
    let src = format!(
        "{LINE}{}",
        scene(
            20,
            "        start w.drive(y: \"sin(x / 7 + t * 5) * 6 + noise(x / 20, t) * 3\")\n"
        )
    );
    let a = go(&src);
    let b = go(&src);
    assert_eq!(a.hashes, b.hashes);
    assert!(a.hashes.windows(2).any(|w| w[0] != w[1]), "it moves");
}

// ---- std/waves -----------------------------------------------------------------------------------

#[test]
fn swell_is_a_travelling_sine() {
    let src = format!(
        "import \"std/waves\"\n{LINE}{}",
        scene(10, "        w.swell(amp: 10, length: 100, period: 1.second, over: 1.second, blend: 0.seconds)\n")
    );
    let r = go(&src);
    for tick in [0usize, 3, 5] {
        let t = tick as f64 / 10.0;
        let v = verts(&r.snapshots[tick], 0);
        near(
            v[2].1,
            10.0 * (std::f64::consts::TAU * (20.0 / 100.0 - t)).sin(),
            1e-9,
        );
    }
}

#[test]
fn choppy_moves_vertices_sideways_and_ripples_runs() {
    let src = format!(
        "import \"std/waves\"\n{LINE}{}",
        scene(10, "        w.choppy(amp: 10, length: 100, period: 1.second, sharp: 0.5, over: 1.second, blend: 0.seconds)\n")
    );
    let r = go(&src);
    let v = verts(&r.snapshots[0], 0);
    // t = 0, x = 0: cos(0) = 1 -> dx = -sharp * amp
    near(v[0].0, -5.0, 1e-9);
    let src = format!(
        "import \"std/waves\"\n{LINE}{}",
        scene(10, "        w.ripples(over: 1.second, blend: 0.seconds)\n")
    );
    let a = go(&src);
    assert!(a.hashes.windows(2).any(|w| w[0] != w[1]));
}

#[test]
fn float_on_bobs_and_tilts_with_the_surface() {
    let src = format!(
        "import \"std/waves\"\nshape boat = rectangle(width: 20, height: 10, at: (30, 0))\n{}",
        scene(10, "        boat.float_on(amp: 10, length: 100, period: 1.second, tilt: 1, over: 1.second, blend: 0.seconds)\n")
    );
    let r = go(&src);
    let tf = |tick: usize| &r.snapshots[tick]["shapes"][0]["transform"];
    // the boat's rest centre is at x = 30: height 10 * sin(tau * 0.3) at t = 0
    near(
        tf(0)["position"][1].as_f64().unwrap(),
        10.0 * (std::f64::consts::TAU * 0.3).sin(),
        1e-9,
    );
    let slope = 10.0 * std::f64::consts::TAU / 100.0 * (std::f64::consts::TAU * 0.3).cos();
    near(
        tf(0)["rotation"].as_f64().unwrap(),
        slope.atan() / std::f64::consts::TAU,
        1e-9,
    );
}
