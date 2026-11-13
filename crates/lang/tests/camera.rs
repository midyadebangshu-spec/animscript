//! Round L4: the camera, camera actions, shots and transitions (snapshot side). Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\nimport \"std/2d\"\n\n";

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
fn cam(snap: &Value) -> (f64, f64, f64, f64) {
    let c = &snap["camera"];
    (
        c["position"][0].as_f64().unwrap(),
        c["position"][1].as_f64().unwrap(),
        c["zoom"].as_f64().unwrap(),
        c["rotation"].as_f64().unwrap(),
    )
}
fn centre(snap: &Value, shape: usize) -> (f64, f64) {
    let v: Vec<(f64, f64)> = snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect();
    (
        v.iter().map(|p| p.0).sum::<f64>() / v.len() as f64,
        v.iter().map(|p| p.1).sum::<f64>() / v.len() as f64,
    )
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn scene(body: &str, secs: u32) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{body}")
}
const A: &str =
    "shape a = square(size: 100, at: (400, 300))\nshape b = square(size: 50, at: (200, 200))\n";

// ---- the camera -------------------------------------------------------------------------------

#[test]
fn the_default_camera_shows_the_whole_canvas() {
    let r = go(&format!("{A}{}", scene("        wait 1.frame\n", 1)));
    assert_eq!(cam(&r.snapshots[0]), (400.0, 300.0, 1.0, 0.0));
}

#[test]
fn camera_position_is_the_centre_of_the_view_in_canvas_coordinates() {
    let r = go(&format!(
        "{A}{}",
        scene(
            "        camera.move_to((200, 100))\n        wait 1.frame\n",
            1
        )
    ));
    let c = cam(&r.snapshots[1]);
    near(c.0, 200.0, 1e-9);
    near(c.1, 100.0, 1e-9);
    let r = go(&format!("{A}{}", scene("        camera.position = (250, 150)\n        wait 2.frames\n        b.position = camera.position\n        wait 1.frame\n", 1)));
    near(cam(&r.snapshots[1]).0, 250.0, 1e-9);
    near(centre(&r.snapshots[2], 1).0, 450.0, 1e-9); // b's centre moved by the camera's position (250, 150)
}

#[test]
fn pan_and_zoom_are_timed_actions_that_take_turns_on_the_frame_channel() {
    let r = go(&format!("{A}{}", scene("        start camera.pan_to((200, 100), over: 1.second, ease: linear)\n        start camera.zoom_to(3, over: 1.second, ease: linear)\n        wait 3.seconds\n", 3)));
    let c = cam(&r.snapshots[12]);
    near(c.0, 300.0, 1e-6);
    near(c.1, 200.0, 1e-6);
    near(c.2, 1.0, 1e-9); // the zoom waits for the pan: one action at a time on `frame`
    near(cam(&r.snapshots[36]).2, 2.0, 1e-6);
    near(cam(&r.snapshots[60]).2, 3.0, 1e-6);
}

#[test]
fn frame_moves_and_zooms_in_one_action() {
    let r = go(&format!(
        "{A}{}",
        scene("        camera.frame(medium, on: b, over: 1.second)\n", 2)
    ));
    let c = cam(&r.snapshots[12]);
    assert!(
        c.0 < 400.0 && c.0 > 200.0 && c.2 > 1.0,
        "both moving at once: {c:?}"
    );
}

#[test]
fn a_pan_can_aim_at_a_shape() {
    let r = go(&format!(
        "{A}{}",
        scene(
            "        camera.pan_to(b, over: 0.5.seconds, ease: linear)\n",
            1
        )
    ));
    near(cam(&r.snapshots[12]).0, 200.0, 1e-6);
    near(cam(&r.snapshots[12]).1, 200.0, 1e-6);
}

#[test]
fn zoom_stays_between_1_and_8() {
    assert_eq!(
        err(&format!(
            "{A}{}",
            scene("        camera.zoom_to(9, over: 1.second)\n", 1)
        ))
        .code,
        "E-CAM-001"
    );
    assert_eq!(
        err(&format!(
            "{A}{}",
            scene("        camera.zoom_to(0.5, over: 0.seconds)\n", 1)
        ))
        .code,
        "E-CAM-001"
    );
    // keyframes beyond the range are clamped, not errors
    let r = go(&format!("{A}{}", scene("        camera.keyframes over 1.second:\n            at 0.s: scale = 1\n            at 1.s: scale = 20\n", 2)));
    near(cam(&r.snapshots[24]).2, 8.0, 1e-9);
}

#[test]
fn hold_occupies_the_frame_channel() {
    let r = go(&format!("{A}{}", scene("        start camera.hold(1.second)\n        start camera.pan_to((100, 100), over: 1.second, ease: linear)\n        wait 3.seconds\n", 3)));
    near(cam(&r.snapshots[12]).0, 400.0, 1e-9); // the pan is queued behind the hold
    near(cam(&r.snapshots[60]).0, 100.0, 1e-6);
}

#[test]
fn follow_trails_the_target_until_cancelled() {
    let r = go(&format!("{A}{}", scene("        let h = start camera.follow(a, lag: 8.frames)\n        a.move_by(200, 0) over 2.seconds ease linear\n        cancel h\n        wait 1.second\n", 4)));
    // the camera lags behind a: it is between the start and the target
    let (x, ax) = (cam(&r.snapshots[24]).0, centre(&r.snapshots[24], 0).0);
    assert!(x > 400.0 && x < ax, "trails: camera {x}, target {ax}");
    let stop = cam(&r.snapshots[60]).0;
    near(cam(&r.snapshots[90]).0, stop, 1e-9); // cancelled: it stays put
}

#[test]
fn shake_is_seeded_additive_and_ends_where_it_began() {
    let src = format!("{A}{}", scene("        camera.pan_to((300, 300), over: 1.second, ease: linear)\n        camera.shake(6, over: 1.second)\n        wait 1.second\n", 4));
    let (r1, r2) = (go(&src), go(&src));
    assert_eq!(r1.snapshots[30], r2.snapshots[30]);
    let during = (24..44)
        .map(|i| (cam(&r1.snapshots[i]).0 - 300.0).abs())
        .fold(0.0f64, f64::max);
    assert!(
        during > 0.1 && during <= 6.5,
        "shakes within the amount: {during}"
    );
    near(cam(&r1.snapshots[72]).0, 300.0, 1e-9);
}

#[test]
fn frame_sets_zoom_from_the_shot_size_and_the_targets_bounds() {
    // a 100 px square, medium = 0.35 of the frame height (600): zoom = 0.35 * 600 / 100 = 2.1
    let r = go(&format!(
        "{A}{}",
        scene(
            "        camera.frame(medium, on: a)\n        wait 1.frame\n",
            1
        )
    ));
    let c = cam(&r.snapshots[1]);
    near(c.2, 2.1, 1e-9);
    near(c.0, 400.0, 1e-9);
    // `thirds: left` puts the target on the left third: the view centre moves right by a sixth of the view width
    let r = go(&format!(
        "{A}{}",
        scene(
            "        camera.frame(medium, on: a, thirds: left)\n        wait 1.frame\n",
            1
        )
    ));
    near(cam(&r.snapshots[1]).0, 400.0 + 800.0 / 2.1 / 6.0, 1e-6);
    let r = go(&format!(
        "{A}{}",
        scene(
            "        camera.zoom_to(4, over: 0.seconds)\n        camera.frame(establishing)\n        wait 1.frame\n",
            1
        )
    ));
    assert_eq!(cam(&r.snapshots[1]), (400.0, 300.0, 1.0, 0.0));
    // timed
    let r = go(&format!(
        "{A}{}",
        scene("        camera.frame(close, on: b, over: 1.second)\n", 2)
    ));
    near(cam(&r.snapshots[24]).2, 7.2, 1e-9); // 0.6 * 600 / 50
}

#[test]
fn bounds_and_the_camera_can_be_read() {
    let r = go(&format!("{A}shape p = square(size: 2, at: (0, 0))\n{}", scene("        camera.zoom_to(2, over: 0.seconds)\n        wait 1.frame\n        p.position = (bounds(a).w, camera.zoom)\n        wait 1.frame\n", 1)));
    let c = centre(&r.snapshots[2], 2);
    near(c.0, 100.0, 1e-6);
    near(c.1, 2.0, 1e-6);
}

#[test]
fn the_camera_resets_with_every_scene_and_its_name_is_taken() {
    let r = go(&format!("{A}scene \"one\" for 1.frame:\n    main:\n        camera.zoom_to(3, over: 0.seconds)\n        wait 1.frame\nscene \"two\" for 1.frame:\n    main:\n        wait 1.frame\n"));
    near(cam(&r.snapshots[0]).2, 3.0, 1e-9);
    assert_eq!(cam(&r.snapshots[1]), (400.0, 300.0, 1.0, 0.0));
    let e = err(&format!(
        "shape camera = square(size: 5, at: (0, 0))\n{}",
        scene("        wait 1.frame\n", 1)
    ));
    assert!(e.code.starts_with("E-"));
}

// ---- shots ------------------------------------------------------------------------------------

const SHOTS: &str = "shots:\n    shot \"One\" for 1.second:\n        camera.frame(medium, on: a)\n    cut\n    shot \"Two\" for 1.second:\n        camera.frame(close, on: b)\n";

fn shot(snap: &Value) -> Option<String> {
    snap["shot"].as_str().map(|s| s.to_string())
}

#[test]
fn shots_run_one_after_another_and_set_the_camera() {
    let r = go(&format!(
        "{A}scene \"s\" for 3.seconds:\n    {SHOTS}\n    main:\n        wait 3.seconds\n"
    )
    .replace("    shots:\n    shot", "    shots:\n        shot")
    .replace("\n    shot \"Two\"", "\n        shot \"Two\"")
    .replace("\n    cut", "\n        cut")
    .replace(
        "        camera.frame(medium",
        "            camera.frame(medium",
    )
    .replace(
        "        camera.frame(close",
        "            camera.frame(close",
    ));
    assert_eq!(shot(&r.snapshots[0]).as_deref(), Some("One"));
    assert_eq!(shot(&r.snapshots[23]).as_deref(), Some("One"));
    assert_eq!(shot(&r.snapshots[24]).as_deref(), Some("Two"));
    near(cam(&r.snapshots[10]).2, 2.1, 1e-9);
    near(cam(&r.snapshots[30]).0, 200.0, 1e-9); // framed on b
                                                // after the last shot the camera stays and the shot name goes
    assert_eq!(shot(&r.snapshots[60]), None);
}

fn with_shots(body: &str, secs: u32) -> String {
    format!("{A}scene \"s\" for {secs}.seconds:\n    shots:\n{body}    main:\n        wait {secs}.seconds\n")
}

#[test]
fn a_shot_can_end_on_a_condition_or_when_its_body_is_done() {
    let r = go(&with_shots("        shot \"A\" until marker(\"m\"):\n            wait 10.seconds\n        shot \"B\" auto:\n            wait 0.5.seconds\n        shot \"C\" for 10.seconds:\n            wait 1.frame\n", 3).replace("    shots:", "    timeline:\n        marker \"m\" at 1.second\n    shots:"));
    assert_eq!(shot(&r.snapshots[23]).as_deref(), Some("A"));
    assert_eq!(shot(&r.snapshots[24]).as_deref(), Some("B"));
    assert_eq!(shot(&r.snapshots[35]).as_deref(), Some("B"));
    assert_eq!(shot(&r.snapshots[36]).as_deref(), Some("C"));
}

#[test]
fn a_shots_camera_actions_stop_when_it_ends() {
    let r = go(&with_shots("        shot \"A\" for 1.second:\n            camera.pan_to((100, 300), over: 4.seconds, ease: linear)\n        shot \"B\" for 2.seconds:\n            wait 1.second\n", 3));
    let at_end = cam(&r.snapshots[24]).0;
    assert!(at_end < 400.0 && at_end > 300.0, "partway: {at_end}");
    near(cam(&r.snapshots[60]).0, cam(&r.snapshots[25]).0, 1e-9); // and it stayed there
}

#[test]
fn rules_inside_a_shot_only_listen_during_that_shot() {
    let src = format!("{A}scene \"s\" for 4.seconds:\n    timeline:\n        marker \"m1\" at 0.5.seconds\n        marker \"m2\" at 1.5.seconds\n    shots:\n        shot \"A\" for 1.second:\n            wait 1.frame\n        shot \"B\" for 2.seconds:\n            when marker(\"m1\"):\n                a.position = (10, 0)\n            when marker(\"m2\"):\n                b.position = (20, 0)\n    main:\n        wait 4.seconds\n");
    let r = go(&src);
    near(centre(&r.snapshots[60], 0).0, 400.0, 1e-9); // m1 came during shot A: the rule did not hear it
    near(centre(&r.snapshots[60], 1).0, 220.0, 1e-9); // m2 came during B
}

#[test]
fn scene_ends_in_is_true_for_the_last_stretch() {
    let r = go(&format!("{A}scene \"s\" for 3.seconds:\n    shots:\n        shot \"A\" auto:\n            when scene.ends_in(1.second):\n                a.position = (30, 0)\n            wait 3.seconds\n    main:\n        wait 3.seconds\n"));
    near(centre(&r.snapshots[40], 0).0, 400.0, 1e-9);
    near(centre(&r.snapshots[60], 0).0, 430.0, 1e-9);
}

#[test]
fn dissolve_and_fade_through_are_in_the_snapshot_while_they_run() {
    let r = go(&with_shots("        shot \"A\" for 1.second:\n            camera.zoom_to(2, over: 0.seconds)\n        dissolve 0.5.seconds\n        shot \"B\" for 1.second:\n            camera.zoom_to(4, over: 0.seconds)\n        fade_through #000 1.second\n        shot \"C\" for 1.second:\n            wait 1.frame\n", 4));
    assert!(r.snapshots[23]["transition"].is_null());
    let t = &r.snapshots[30]["transition"];
    assert_eq!(t["kind"], "dissolve");
    near(t["t"].as_f64().unwrap(), 6.0 / 12.0, 1e-9);
    near(t["from"]["zoom"].as_f64().unwrap(), 2.0, 1e-9); // the old view is kept while it fades
    assert!(r.snapshots[37]["transition"].is_null());
    let f = &r.snapshots[60]["transition"];
    assert_eq!(f["kind"], "fade_through");
    assert_eq!(f["color"], "#000000");
    near(f["from"]["zoom"].as_f64().unwrap(), 4.0, 1e-9);
}

#[test]
fn shot_errors() {
    let e = err(&format!("{A}scene \"s\" for 1.second:\n    shots:\n        shot \"A\":\n            wait 1.frame\n    main:\n        wait 1.second\n"));
    assert_eq!(e.code, "E-SYN-001");
    let e = err(&format!("{A}scene \"s\" for 1.second:\n    shots:\n        zoom 2\n    main:\n        wait 1.second\n"));
    assert_eq!(e.code, "E-SYN-001");
}

#[test]
fn shots_are_deterministic() {
    let src = with_shots("        shot \"A\" for 1.second:\n            camera.shake(4, over: 1.second)\n        dissolve 0.5.seconds\n        shot \"B\" auto:\n            camera.zoom_to(2, over: 1.second)\n", 3);
    assert_eq!(go(&src).snapshots[40], go(&src).snapshots[40]);
}
