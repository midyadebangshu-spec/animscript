//! Round L3: clips, overloading of actions by `self` type, and `std/2d`. Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";

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
fn bone_angle(snap: &Value, name: &str) -> f64 {
    snap["rigs"][0]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap()["angle"]
        .as_f64()
        .unwrap()
}
const SH: &str =
    "shape a = square(size: 10, at: (0, 0))\nshape b = square(size: 10, at: (0, 50))\n";
const WAVE: &str = "clip wave(self: Shape) over 1.second:\n    at 0.s: position.x = 0\n    at 0.5.s: position.x = 100\n    at 1.s: position.x = 0\n\n";
fn scene(body: &str, secs: u32) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{body}")
}

// ---- clips ------------------------------------------------------------------------------------

#[test]
fn a_clip_plays_its_keys_relative_to_the_call() {
    let r = go(&format!(
        "{SH}{WAVE}{}",
        scene("        wait 0.5.seconds\n        a.wave()\n", 3)
    ));
    near(centre(&r.snapshots[12], 0).0, 0.0, 1e-9); // not started
    near(centre(&r.snapshots[18], 0).0, 50.0, 1e-6);
    near(centre(&r.snapshots[24], 0).0, 100.0, 1e-6);
    near(centre(&r.snapshots[36], 0).0, 0.0, 1e-6);
}

#[test]
fn a_clip_blocks_for_its_duration_and_works_with_start_and_together() {
    let r = go(&format!(
        "{SH}{WAVE}{}",
        scene("        a.wave()\n        b.position = (5, 0)\n", 3)
    ));
    near(centre(&r.snapshots[23], 1).0, 0.0, 1e-9);
    near(centre(&r.snapshots[25], 1).0, 5.0, 1e-9); // the next statement came after the clip
    let r = go(&format!(
        "{SH}{WAVE}{}",
        scene(
            "        together:\n            a.wave()\n            b.wave()\n",
            2
        )
    ));
    near(centre(&r.snapshots[12], 0).0, 100.0, 1e-6);
    near(centre(&r.snapshots[12], 1).0, 100.0, 1e-6);
}

#[test]
fn speed_scales_time_and_parameters_reach_the_keys() {
    let r = go(&format!(
        "{SH}{WAVE}{}",
        scene("        a.wave(speed: 2)\n", 2)
    ));
    near(centre(&r.snapshots[6], 0).0, 100.0, 1e-6); // twice as fast
    near(centre(&r.snapshots[12], 0).0, 0.0, 1e-6);
    let r = go(&format!("{SH}clip slide(self: Shape, dx: Number = 40) over 1.second:\n    at 1.s: position.x = dx\n\n{}", scene("        a.slide(dx: 80)\n        b.slide()\n", 3)));
    near(centre(&r.snapshots[24], 0).0, 80.0, 1e-6);
    near(centre(&r.snapshots[48], 1).0, 40.0, 1e-6);
}

#[test]
fn a_clip_can_loop_until_cancelled() {
    let r = go(&format!("{SH}{WAVE}{}", scene("        let h = start a.wave(loop: true)\n        wait 2.5.seconds\n        cancel h\n", 4)));
    near(centre(&r.snapshots[12], 0).0, 100.0, 1e-6);
    near(centre(&r.snapshots[24], 0).0, 0.0, 1e-6);
    near(centre(&r.snapshots[36], 0).0, 100.0, 1e-6); // the second round
    near(centre(&r.snapshots[60], 0).0, 100.0, 1e-6); // cancelled at the third peak: keeps its value
}

#[test]
fn play_clip_runs_a_clip_by_name() {
    let r = go(&format!(
        "{SH}{WAVE}{}",
        scene("        a.play_clip(\"wave\", speed: 2)\n", 2)
    ));
    near(centre(&r.snapshots[6], 0).0, 100.0, 1e-6);
    let e = err(&format!(
        "{SH}{}",
        scene("        a.play_clip(\"nope\")\n", 1)
    ));
    assert_eq!(e.code, "E-NAM-001");
}

#[test]
fn two_clips_on_one_shape_take_turns_on_the_body_channel() {
    let r = go(&format!(
        "{SH}{WAVE}{}",
        scene(
            "        start a.wave()\n        start a.wave()\n        wait 3.seconds\n",
            3
        )
    ));
    near(centre(&r.snapshots[12], 0).0, 100.0, 1e-6);
    near(centre(&r.snapshots[24], 0).0, 0.0, 1e-6);
    near(centre(&r.snapshots[36], 0).0, 100.0, 1e-6); // the second one, after the first
}

#[test]
fn an_armature_clip_keys_bones() {
    let arm = "armature a:\n    bone upper: at: (100, 100), length: 50, angle: 0.deg\n    bone lower: parent: upper, length: 50\n\nclip bend(self: Armature) over 1.second:\n    at 0.s:\n        upper.rotation = 0.deg\n        lower.rotation = 0.deg\n    at 1.s:\n        upper.rotation = 90.deg\n        lower.rotation = 45.deg\n\n";
    let r = go(&format!("{arm}{}", scene("        a.bend()\n", 2)));
    near(bone_angle(&r.snapshots[24], "upper"), 0.25, 1e-6);
    near(bone_angle(&r.snapshots[24], "lower"), 0.375, 1e-6); // 90 + 45 degrees
    near(bone_angle(&r.snapshots[12], "upper"), 0.125, 1e-6);
}

#[test]
fn clip_errors() {
    let e = err(&format!(
        "{SH}clip bad(self: Shape) over 1.second:\n    at 2.s: position.x = 1\n\n{}",
        scene("        a.bad()\n", 1)
    ));
    assert_eq!(e.code, "E-TYP-001"); // a key beyond the duration
    let e = err(&format!(
        "{SH}clip bad(self: Shape):\n    at 1.s: position.x = 1\n\n{}",
        scene("        wait 1.second\n", 1)
    ));
    assert_eq!(e.code, "E-SYN-001");
    let e = err(&format!(
        "{SH}clip bad(n: Number) over 1.second:\n    at 1.s: position.x = 1\n\n{}",
        scene("        wait 1.second\n", 1)
    ));
    assert_eq!(e.code, "E-SYN-001");
    let e = err(&format!(
        "{SH}{WAVE}{}",
        scene("        a.wave(speed: 0)\n", 1)
    ));
    assert_eq!(e.code, "E-CAP-001"); // `requires speed > 0`
}

// ---- overloading ------------------------------------------------------------------------------

const ARM: &str = "armature r:\n    bone root: at: (100, 100), length: 50\n\n";

#[test]
fn actions_of_one_name_with_different_self_types_coexist() {
    let defs = "action tag(self: Shape):\n    self.position = (5, 0)\n\naction tag(self: Armature):\n    b.position = (7, 0)\n\n";
    let r = go(&format!(
        "{SH}{ARM}{defs}{}",
        scene(
            "        a.tag()\n        r.tag()\n        wait 1.frame\n",
            1
        )
    ));
    near(centre(&r.snapshots[1], 0).0, 5.0, 1e-9);
    near(centre(&r.snapshots[1], 1).0, 7.0, 1e-9);
}

#[test]
fn a_specific_type_beats_any() {
    let defs = "action tag(self: Any):\n    b.position = (3, 0)\n\naction tag(self: Shape):\n    a.position = (5, 0)\n\n";
    let r = go(&format!(
        "{SH}{ARM}{defs}{}",
        scene(
            "        a.tag()\n        r.tag()\n        wait 1.frame\n",
            1
        )
    ));
    near(centre(&r.snapshots[1], 0).0, 5.0, 1e-9); // the Shape one for a shape
    near(centre(&r.snapshots[1], 1).0, 3.0, 1e-9); // the Any one for an armature
}

#[test]
fn no_matching_overload_is_e_cap_001_and_clashes_are_e_def_001() {
    let defs = "action tag(self: Armature):\n    wait 1.frame\n\naction tag(self: Number):\n    wait 1.frame\n\n";
    let e = err(&format!("{SH}{ARM}{defs}{}", scene("        a.tag()\n", 1)));
    assert_eq!(e.code, "E-CAP-001");
    let e = err(&format!("{SH}action tag(self: Shape):\n    wait 1.frame\n\naction tag(self: Shape):\n    wait 1.frame\n\n{}", scene("        wait 1.frame\n", 1)));
    assert_eq!(e.code, "E-DEF-001");
    let e = err(&format!("{SH}fn tag(x: Number) -> Number:\n    return x\n\naction tag(self: Shape):\n    wait 1.frame\n\n{}", scene("        wait 1.frame\n", 1)));
    assert_eq!(e.code, "E-DEF-001");
}

// ---- std/2d -----------------------------------------------------------------------------------

const S2: &str = "import \"std/2d\"\n\nshape a = square(size: 20, at: (400, 300))\nshape t = square(size: 4, at: (0, 0))\n";

#[test]
fn look_at_turns_towards_a_point_and_look_around_comes_back() {
    let r = go(&format!("{S2}{}", scene("        a.look_at((400, 500), over: 0.5.seconds)\n        a.look_around(sweep: 60.deg, over: 1.second)\n        wait 0.5.seconds\n", 3)));
    // facing straight down is a quarter turn: the first vertex of a square moves with the rotation
    let v0 = |i: usize| {
        (
            r.snapshots[i]["shapes"][0]["vertices"][0][1]
                .as_f64()
                .unwrap(),
            r.snapshots[i]["shapes"][0]["vertices"][0][2]
                .as_f64()
                .unwrap(),
        )
    };
    let before = v0(0);
    let turned = v0(12);
    assert!(
        (before.0 - turned.0).abs() > 1.0 || (before.1 - turned.1).abs() > 1.0,
        "rotated"
    );
    near(centre(&r.snapshots[12], 0).0, 400.0, 1e-6); // about its own centre
    let a = v0(12);
    let b = v0(36); // after the sweep it is back where it was
    near(a.0, b.0, 1e-6);
    near(a.1, b.1, 1e-6);
}

#[test]
fn walk_to_moves_the_centre_to_the_target_at_the_speed() {
    let r = go(&format!(
        "{S2}{}",
        scene("        a.walk_to((640, 300), speed: 120)\n", 4)
    ));
    near(centre(&r.snapshots[48], 0).0, 640.0, 1e-6); // 240 px at 120 px/s: 2 s
    near(centre(&r.snapshots[24], 0).0, 520.0, 1e-6);
}

#[test]
fn step_back_moves_away_from_a_point() {
    let r = go(&format!(
        "{S2}{}",
        scene(
            "        a.step_back((300, 300), distance: 50, over: 0.5.seconds)\n",
            2
        )
    ));
    near(centre(&r.snapshots[12], 0).0, 450.0, 1e-6);
    near(centre(&r.snapshots[12], 0).1, 300.0, 1e-6);
}

#[test]
fn fade_in_and_out_and_turn_around() {
    let r = go(&format!("{S2}{}", scene("        a.fade_out(over: 0.5.seconds)\n        a.fade_in(over: 0.5.seconds)\n        a.turn_around(over: 0.25.seconds)\n", 3)));
    let op = |i: usize| r.snapshots[i]["shapes"][0]["opacity"].as_f64().unwrap();
    near(op(12), 0.0, 1e-6);
    near(op(24), 1.0, 1e-6);
    let x = |i: usize| {
        r.snapshots[i]["shapes"][0]["vertices"][0][1]
            .as_f64()
            .unwrap()
    };
    near(x(36), 410.0, 1e-6); // mirrored about its centre: the first vertex is now on the right
    assert!((x(0) - x(36)).abs() > 1.0, "mirrored");
}

#[test]
fn fly_away_leaves_the_canvas_and_fly_to_arrives() {
    let r = go(&format!(
        "{S2}{}",
        scene(
            "        a.fly_away(direction: up_right, over: 2.seconds)\n",
            3
        )
    ));
    let c = centre(&r.snapshots[48], 0);
    assert!(c.0 > 900.0 && c.1 < -300.0, "far outside: {c:?}");
    let r = go(&format!(
        "{S2}{}",
        scene("        a.fly_to((100, 100), over: 1.5.seconds)\n", 2)
    ));
    let c = centre(&r.snapshots[36], 0);
    near(c.0, 100.0, 1e-6);
    near(c.1, 100.0, 1e-6);
}

#[test]
fn std_2d_and_std_gait_can_be_imported_together() {
    let r = go(&format!(
        "import \"std/gait\"\n{S2}{}",
        scene("        a.walk_to((500, 300), speed: 100)\n", 3)
    ));
    near(centre(&r.snapshots[48], 0).0, 500.0, 1e-6);
}

#[test]
fn std_2d_is_deterministic() {
    let src = format!("{S2}{}", scene("        a.fly_away(over: 2.seconds)\n", 3));
    assert_eq!(go(&src).snapshots[30], go(&src).snapshots[30]);
}

#[test]
fn a_clip_can_use_another_channel_so_two_run_together() {
    let two = "clip nod(self: Shape) over 1.second uses face:\n    at 0.5.s: position.y = 40\n    at 1.s: position.y = 0\n\n";
    let r = go(&format!(
        "{SH}{WAVE}{two}{}",
        scene(
            "        start a.wave()\n        start a.nod()\n        wait 2.seconds\n",
            2
        )
    ));
    near(centre(&r.snapshots[12], 0).0, 100.0, 1e-6);
    near(centre(&r.snapshots[12], 0).1, 40.0, 1e-6); // both at once: different channels
}
