//! Brushes, paint layers and strokes in the language. Written alongside the implementation.
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
fn paint(snap: &Value, shape: usize) -> &Value {
    &snap["shapes"][shape]["paint"]
}
fn scene(body: &str, secs: u32) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{body}")
}
const SETUP: &str =
    "let ink = brush(\"pen\", radius: 3, color: #223355)\nshape art = paint_layer()\n";

#[test]
fn a_paint_layer_is_a_shape_with_strokes() {
    let r = go(&format!(
        "{SETUP}art.stroke(ink, [(10, 10), (100, 10, 0.5), (200, 60)])\n{}",
        scene("        wait 1.frame\n", 1)
    ));
    let p = paint(&r.snapshots[0], 0);
    assert_eq!(p["size"][0], 400);
    assert_eq!(p["strokes"].as_array().unwrap().len(), 1);
    let s = &p["strokes"][0];
    near(s["progress"].as_f64().unwrap(), 1.0, 1e-12); // made at build time: already painted
    assert_eq!(s["points"].as_array().unwrap().len(), 3);
    near(s["points"][0][2].as_f64().unwrap(), 0.7, 1e-12); // default pressure
    near(s["points"][1][2].as_f64().unwrap(), 0.5, 1e-12);
    // the time between points follows the speed (300 px/s): 90 px take 0.3 s
    near(s["points"][1][3].as_f64().unwrap(), 0.3, 1e-9);
    let b = &p["brushes"][0];
    assert_eq!(b["source"], "pen");
    assert_eq!(b["color"].as_array().unwrap().len(), 3);
    near(b["set"][0][1].as_f64().unwrap(), 3f64.ln(), 1e-12); // radius is stored as its logarithm
}

#[test]
fn a_timed_stroke_is_drawn_in_over_its_duration() {
    let r = go(&format!("{SETUP}shape mark = square(size: 4, at: (0, 0))\n{}", scene("        art.stroke(ink, [(10, 10), (300, 10)]) over 1.second\n        mark.position = (5, 0)\n        wait 2.seconds\n", 3)));
    let prog = |i: usize| {
        paint(&r.snapshots[i], 0)["strokes"][0]["progress"]
            .as_f64()
            .unwrap()
    };
    near(prog(0), 0.0, 1e-9);
    near(prog(12), 0.5, 1e-6); // linear by default
    near(prog(24), 1.0, 1e-9);
    near(prog(60), 1.0, 1e-9);
    // blocking: the next statement starts when the stroke is done
    let mark = |i: usize| {
        r.snapshots[i]["shapes"][1]["vertices"][0][1]
            .as_f64()
            .unwrap()
    };
    assert!(mark(23) < mark(25) - 4.0, "{} {}", mark(23), mark(25));
}

#[test]
fn an_easing_changes_how_the_stroke_is_drawn() {
    let r = go(&format!(
        "{SETUP}{}",
        scene(
            "        art.stroke(ink, [(10, 10), (300, 10)]) over 1.second ease ease_in\n",
            2
        )
    ));
    let at12 = paint(&r.snapshots[12], 0)["strokes"][0]["progress"]
        .as_f64()
        .unwrap();
    assert!(at12 < 0.4, "ease_in starts slowly: {at12}");
}

#[test]
fn strokes_can_be_drawn_together_and_in_order() {
    let r = go(&format!("{SETUP}{}", scene("        together:\n            art.stroke(ink, [(10, 10), (300, 10)]) over 1.second\n            art.stroke(ink, [(10, 50), (300, 50)]) over 2.seconds\n        art.stroke(ink, [(10, 90), (300, 90)]) over 1.second\n", 4)));
    let prog = |i: usize, k: usize| {
        paint(&r.snapshots[i], 0)["strokes"][k]["progress"]
            .as_f64()
            .unwrap()
    };
    near(prog(12, 0), 0.5, 1e-6);
    near(prog(12, 1), 0.25, 1e-6); // both at once, each at its own pace
    near(prog(30, 1), 0.625, 1e-6);
    assert_eq!(
        paint(&r.snapshots[47], 0)["strokes"]
            .as_array()
            .unwrap()
            .len(),
        2
    ); // the third is made after the second ends (tick 48)
    assert!(prog(60, 2) > 0.4);
}

#[test]
fn the_layer_moves_fades_and_orders_like_a_shape() {
    let r = go(&format!("{SETUP}shape back = square(size: 10, at: (5, 5))\nart.z = 5\n{}", scene("        art.stroke(ink, [(10, 10), (30, 10)])\n        art.move_by(40, 0) over 1.second ease linear\n        art.fade_to(0.5) over 1.second\n", 3)));
    near(
        r.snapshots[12]["shapes"][1]["transform"]["position"][0]
            .as_f64()
            .unwrap(),
        20.0,
        1e-6,
    );
    near(
        r.snapshots[60]["shapes"][1]["opacity"].as_f64().unwrap(),
        0.5,
        1e-6,
    );
    assert!(
        r.snapshots[0]["shapes"][1]["paint"].is_object(),
        "z 5 puts the layer after the square"
    );
    near(
        r.snapshots[0]["shapes"][1]["transform"]["pivot"][0]
            .as_f64()
            .unwrap(),
        200.0,
        1e-9,
    ); // turns about the canvas centre
}

#[test]
fn clear_strokes_empties_the_layer() {
    let r = go(&format!(
        "{SETUP}art.stroke(ink, [(10, 10), (30, 10)])\n{}",
        scene(
            "        wait 1.frame\n        art.clear_strokes()\n        wait 1.frame\n",
            1
        )
    ));
    assert_eq!(
        paint(&r.snapshots[0], 0)["strokes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        paint(&r.snapshots[1], 0)["strokes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn the_same_brush_is_stored_once() {
    let r = go(&format!("{SETUP}art.stroke(ink, [(10, 10), (30, 10)])\nart.stroke(ink, [(10, 20), (30, 20)])\nlet other = brush(\"pencil\")\nart.stroke(other, [(10, 30), (30, 30)])\n{}", scene("        wait 1.frame\n", 1)));
    assert_eq!(
        paint(&r.snapshots[0], 0)["brushes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(paint(&r.snapshots[0], 0)["strokes"][2]["brush"], 1);
}

#[test]
fn brush_settings_by_name_and_a_file_brush() {
    let r = go(&format!("let b = brush(\"charcoal\", opacity: 0.4, hardness: 0.5, eraser: true, slow_tracking: 2)\nshape art = paint_layer()\nart.stroke(b, [(1, 1), (9, 9)])\n{}", scene("        wait 1.frame\n", 1)));
    let set = paint(&r.snapshots[0], 0)["brushes"][0]["set"]
        .as_array()
        .unwrap()
        .clone();
    let get = |k: &str| {
        set.iter()
            .find(|p| p[0] == k)
            .map(|p| p[1].as_f64().unwrap())
    };
    assert_eq!(get("opaque"), Some(0.4));
    assert_eq!(get("eraser"), Some(1.0));
    assert_eq!(get("slow_tracking"), Some(2.0));
    let dir = std::env::temp_dir().join(format!("animscript_brush_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mine.myb");
    std::fs::write(
        &path,
        r#"{"version": 3, "settings": {"opaque": {"base_value": 0.5, "inputs": {}}}}"#,
    )
    .unwrap();
    let r = go(&format!(
        "let b = brush(\"{}\")\nshape art = paint_layer()\nart.stroke(b, [(1, 1), (9, 9)])\n{}",
        path.display(),
        scene("        wait 1.frame\n", 1)
    ));
    assert!(paint(&r.snapshots[0], 0)["brushes"][0]["myb"]
        .as_str()
        .unwrap()
        .contains("\"opaque\""));
}

#[test]
fn brush_errors() {
    assert_eq!(err("let b = brush(\"no_such\")\n").code, "E-BRU-001");
    assert_eq!(
        err("let b = brush(\"pen\", no_such_setting: 1)\n").code,
        "E-BRU-001"
    );
    assert_eq!(
        err("let b = brush(\"/nonexistent/x.myb\")\n").code,
        "E-BRU-001"
    );
    assert_eq!(
        err("let b = brush(\"pen\", radius: -2)\n").code,
        "E-BRU-001"
    );
}

#[test]
fn stroke_errors() {
    assert_eq!(
        err(&format!("{SETUP}art.stroke(ink, [])\n")).code,
        "E-BRU-002"
    );
    assert_eq!(
        err(&format!("{SETUP}art.stroke(ink, [(1,)])\n")).code,
        "E-BRU-002"
    );
    assert_eq!(
        err(&format!("{SETUP}art.stroke(ink, [(1, 2, 3)])\n")).code,
        "E-BRU-002"
    ); // pressure above 1
    assert_eq!(
        err(&format!("{SETUP}art.stroke(5, [(1, 2)])\n")).code,
        "E-BRU-002"
    );
    assert_eq!(
        err(&format!("{SETUP}art.stroke(ink, [(1, 2)], speed: 0)\n")).code,
        "E-BRU-002"
    );
}

#[test]
fn painting_is_deterministic() {
    let src = format!(
        "{SETUP}{}",
        scene(
            "        art.stroke(ink, [(10, 10), (100, 50, 0.4), (300, 20)]) over 1.second\n",
            2
        )
    );
    assert_eq!(go(&src).snapshots[14], go(&src).snapshots[14]);
}

// ---- tilt, symmetry and blend modes --------------------------------------------------------

fn stroke_json(snap: &Value, k: usize) -> &Value {
    &paint(snap, 0)["strokes"][k]
}

#[test]
fn points_may_carry_a_tilt_and_a_stroke_can_default_it() {
    let r = go(&format!("{SETUP}art.stroke(ink, [(10, 10, 0.5, 0.01, 0.3, -0.4), (60, 10, 0.5, 0.05, -0.2, 0.1)])\nart.stroke(ink, [(10, 30), (60, 30, 0.8, 0.05, 0.9, 0.9)], tilt: (0.5, 0))\n{}", scene("        wait 1.frame\n", 1)));
    let p = &stroke_json(&r.snapshots[0], 0)["points"];
    near(p[0][4].as_f64().unwrap(), 0.3, 1e-12);
    near(p[1][5].as_f64().unwrap(), 0.1, 1e-12);
    let q = &stroke_json(&r.snapshots[0], 1)["points"];
    near(q[0][4].as_f64().unwrap(), 0.5, 1e-12); // the stroke default
    near(q[1][4].as_f64().unwrap(), 0.9, 1e-12); // a point's own tilt wins
}

#[test]
fn tilt_follow_leans_the_pen_the_way_the_stroke_goes() {
    let r = go(&format!("{SETUP}art.stroke(ink, [(10, 10), (60, 10), (110, 10)], tilt: \"follow\")\nart.stroke(ink, [(10, 40), (10, 90), (10, 140)], tilt: \"follow\")\n{}", scene("        wait 1.frame\n", 1)));
    let right = &stroke_json(&r.snapshots[0], 0)["points"][1];
    near(right[4].as_f64().unwrap(), 0.6, 1e-9);
    near(right[5].as_f64().unwrap(), 0.0, 1e-9);
    let down = &stroke_json(&r.snapshots[0], 1)["points"][1];
    near(down[4].as_f64().unwrap(), 0.0, 1e-9);
    near(down[5].as_f64().unwrap(), 0.6, 1e-9);
}

#[test]
fn barrel_is_stored_in_turns() {
    let r = go(&format!(
        "{SETUP}art.stroke(ink, [(10, 10), (60, 10)], barrel: 90.deg)\n{}",
        scene("        wait 1.frame\n", 1)
    ));
    near(
        stroke_json(&r.snapshots[0], 0)["barrel"].as_f64().unwrap(),
        0.25,
        1e-12,
    );
}

#[test]
fn symmetry_is_stored_per_stroke_with_the_canvas_centre_by_default() {
    let r = go(&format!("{SETUP}art.stroke(ink, [(10, 10), (60, 10)], symmetry: \"radial\", lines: 5)\nart.stroke(ink, [(10, 10), (60, 10)], symmetry: \"snowflake\", lines: 8, center: (100, 50), angle: 90.deg)\nart.stroke(ink, [(10, 10), (60, 10)])\n{}", scene("        wait 1.frame\n", 1)));
    let a = &stroke_json(&r.snapshots[0], 0)["symmetry"];
    assert_eq!(
        (a["kind"].as_str().unwrap(), a["lines"].as_i64().unwrap()),
        ("radial", 5)
    );
    near(a["center"][0].as_f64().unwrap(), 200.0, 1e-9); // 400 x 300 canvas
    near(a["center"][1].as_f64().unwrap(), 150.0, 1e-9);
    let b = &stroke_json(&r.snapshots[0], 1)["symmetry"];
    near(b["center"][0].as_f64().unwrap(), 100.0, 1e-9);
    near(b["angle"].as_f64().unwrap(), 90.0, 1e-9); // stored in degrees
    assert!(stroke_json(&r.snapshots[0], 2)["symmetry"].is_null());
}

#[test]
fn a_layer_default_symmetry_applies_to_later_strokes_only() {
    let r = go(&format!("{SETUP}art.stroke(ink, [(10, 10), (60, 10)])\nart.symmetry(\"both\", center: (50, 50))\nart.stroke(ink, [(10, 20), (60, 20)])\nart.stroke(ink, [(10, 30), (60, 30)], symmetry: none)\nart.stroke(ink, [(10, 40), (60, 40)], symmetry: \"vertical\")\nart.symmetry(none)\nart.stroke(ink, [(10, 50), (60, 50)])\n{}", scene("        wait 1.frame\n", 1)));
    let kind = |k: usize| {
        stroke_json(&r.snapshots[0], k)["symmetry"]["kind"]
            .as_str()
            .map(|s| s.to_string())
    };
    assert_eq!(kind(0), None);
    assert_eq!(kind(1).as_deref(), Some("both"));
    assert_eq!(kind(2), None, "`symmetry: none` overrides the default");
    assert_eq!(kind(3).as_deref(), Some("vertical"));
    assert_eq!(kind(4), None, "cleared");
}

#[test]
fn blend_and_strength_are_stored_and_the_layer_has_a_blend() {
    let r = go(&format!("{SETUP}art.stroke(ink, [(10, 10), (60, 10)], blend: \"multiply\", strength: 0.5)\nart.stroke(ink, [(10, 30), (60, 30)])\nart.blend = \"screen\"\n{}", scene("        wait 1.frame\n", 1)));
    let s = stroke_json(&r.snapshots[0], 0);
    assert_eq!(s["blend"], "multiply");
    near(s["strength"].as_f64().unwrap(), 0.5, 1e-12);
    assert_eq!(stroke_json(&r.snapshots[0], 1)["blend"], "normal");
    assert_eq!(paint(&r.snapshots[0], 0)["blend"], "screen");
}

#[test]
fn tilt_symmetry_and_blend_errors() {
    assert_eq!(
        err(&format!(
            "{SETUP}art.stroke(ink, [(1, 2, 0.5, 0.01, 0.3)])\n"
        ))
        .code,
        "E-BRU-002"
    ); // five numbers
    assert_eq!(
        err(&format!(
            "{SETUP}art.stroke(ink, [(1, 2, 0.5, 0.01, 2, 0)])\n"
        ))
        .code,
        "E-BRU-002"
    ); // tilt above 1
    assert_eq!(
        err(&format!("{SETUP}art.stroke(ink, [(1, 2)], tilt: (3, 0))\n")).code,
        "E-BRU-002"
    );
    assert_eq!(
        err(&format!(
            "{SETUP}art.stroke(ink, [(1, 2)], tilt: \"sideways\")\n"
        ))
        .code,
        "E-BRU-003"
    );
    assert_eq!(
        err(&format!(
            "{SETUP}art.stroke(ink, [(1, 2)], symmetry: \"spiral\")\n"
        ))
        .code,
        "E-BRU-003"
    );
    assert_eq!(
        err(&format!(
            "{SETUP}art.stroke(ink, [(1, 2)], symmetry: \"radial\", lines: 1)\n"
        ))
        .code,
        "E-BRU-003"
    );
    assert_eq!(
        err(&format!(
            "{SETUP}art.stroke(ink, [(1, 2)], blend: \"glow\")\n"
        ))
        .code,
        "E-BRU-003"
    );
    assert_eq!(
        err(&format!("{SETUP}art.stroke(ink, [(1, 2)], strength: 2)\n")).code,
        "E-BRU-003"
    );
    assert_eq!(
        err(&format!("{SETUP}art.blend = \"glow\"\n")).code,
        "E-BRU-003"
    );
    assert_eq!(
        err(&format!("{SETUP}art.symmetry(\"nope\")\n")).code,
        "E-BRU-003"
    );
}
