//! Ragdolls: an armature with a `ragdoll:` line gets a capsule body per bone, joined by limited hinges.
//! `armature.ragdoll(strength:, push:)` hands the bones to physics, `armature.animate()` takes them back.
//! Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\nworld:\n    gravity: (0, 9.8)\n    scale: 100\n\n";
const GROUND: &str = "shape ground = rectangle(width: 2000, height: 40, at: (400, 580))\nground.body(kind: static)\n";
// a two-bone arm lying to the right; the elbow bends one way only
const ARM: &str = "armature a:\n    bone upper: at: (300, 200), length: 100, angle: 0.deg\n    bone lower: parent: upper, length: 100, limits: (0.deg, 150.deg)\n    ragdoll: radius: 6\n\n";

fn run_src(s: &str) -> Result<RunResult, Box<Diagnostic>> {
    run(&format!("{HEAD}{s}"), &RunOptions::default()).map_err(Box::new)
}
fn go(s: &str) -> RunResult {
    let text = format!("{HEAD}{s}");
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
/// `(head x, head y, tip x, tip y, angle in turns)` of a bone in a snapshot.
fn bone(snap: &Value, name: &str) -> (f64, f64, f64, f64, f64) {
    let b = snap["rigs"][0]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .expect("bone");
    let f = |k: &str, i: usize| b[k][i].as_f64().unwrap();
    (
        f("head", 0),
        f("head", 1),
        f("tip", 0),
        f("tip", 1),
        b["angle"].as_f64().unwrap(),
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
fn scene(main: &str, secs: f64) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{main}        wait {secs}.seconds\n")
}

#[test]
fn a_ragdoll_armature_still_follows_its_animation_until_it_goes_limp() {
    let r = go(&format!(
        "{GROUND}{ARM}{}",
        scene(
            "        a.upper.rotate_to(90.deg) over 1.second ease linear\n",
            2.0
        )
    ));
    let (hx, hy, tx, ty, ang) = bone(&r.snapshots[47], "upper");
    near(ang, 0.25, 0.01); // 90 degrees: pointing down
    near((hx, hy).0, 300.0, 0.5);
    near(hy, 200.0, 0.5);
    near(tx, 300.0, 1.0);
    near(ty, 300.0, 1.0);
}

#[test]
fn a_limp_ragdoll_falls_and_lies_on_the_floor() {
    let r = go(&format!(
        "{GROUND}{ARM}{}",
        scene("        wait 0.5.seconds\n        a.ragdoll()\n", 3.0)
    ));
    let before = bone(&r.snapshots[10], "upper");
    near(before.1, 200.0, 0.5); // still held
    let end = &r.snapshots[71];
    for name in ["upper", "lower"] {
        let b = bone(end, name);
        // the floor's top is y = 560: nothing sinks into it (radius 6)
        assert!(
            b.1 <= 560.0 && b.3 <= 560.0,
            "{name} above the floor: {b:?}"
        );
        assert!(b.1 > 400.0 && b.3 > 400.0, "{name} came down: {b:?}");
    }
}

#[test]
fn limits_hold_while_the_ragdoll_tumbles() {
    let r = go(&format!(
        "{GROUND}{ARM}{}",
        scene("        a.ragdoll(push: (300, -200))\n", 3.0)
    ));
    for i in (0..72).step_by(3) {
        let (u, l) = (
            bone(&r.snapshots[i], "upper").4,
            bone(&r.snapshots[i], "lower").4,
        );
        let mut d = l - u;
        d -= d.round();
        assert!(
            (-0.01..=150.0 / 360.0 + 0.01).contains(&d),
            "frame {i}: elbow {d} turns is outside its limits"
        );
    }
}

#[test]
fn push_gives_the_bones_a_velocity() {
    let r = go(&format!(
        "{GROUND}{ARM}{}",
        scene("        a.ragdoll(push: (400, 0))\n", 1.0)
    ));
    let (h0, h1) = (
        bone(&r.snapshots[1], "upper").0,
        bone(&r.snapshots[12], "upper").0,
    );
    assert!(h1 - h0 > 100.0, "flew to the right: {}", h1 - h0);
}

#[test]
fn art_bound_to_a_bone_follows_the_ragdoll() {
    let r = go(&format!("{GROUND}{ARM}shape pad = rectangle(width: 100, height: 20, at: (350, 200))\npad.bind(a.upper)\n{}", scene("        wait 0.5.seconds\n        a.ragdoll()\n", 3.0)));
    // shape order: ground (0), pad (1)
    let y0 = centre(&r.snapshots[5], 1).1;
    let y1 = centre(&r.snapshots[71], 1).1;
    near(y0, 200.0, 1.0);
    assert!(y1 > 400.0, "the art fell with its bone: {y1}");
}

#[test]
fn animate_takes_the_bones_back() {
    let r = go(&format!(
        "{GROUND}{ARM}{}",
        scene(
            "        a.ragdoll()\n        wait 1.second\n        a.animate()\n",
            3.0
        )
    ));
    assert!(
        bone(&r.snapshots[20], "upper").1 > 300.0,
        "limp and falling"
    );
    let b = bone(&r.snapshots[71], "upper");
    near(b.0, 300.0, 1.0); // back in the animated pose
    near(b.1, 200.0, 1.0);
}

#[test]
fn ragdolled_reports_the_state() {
    let r = go(&format!("{GROUND}{ARM}shape probe = square(size: 4, at: (0, 0))\naction watch():\n    while true:\n        if a.ragdolled:\n            probe.move_to((20, 0))\n        else:\n            probe.move_to((10, 0))\n        wait 1.frame\n\n{}", scene("        start watch()\n        wait 0.5.seconds\n        a.ragdoll()\n", 1.0)));
    near(centre(&r.snapshots[3], 1).0, 10.0, 1.0);
    near(centre(&r.snapshots[20], 1).0, 20.0, 1.0);
}

#[test]
fn an_active_ragdoll_follows_the_animated_pose_and_a_weak_one_does_not() {
    let run_with = |strength: &str| {
        go(&format!("{GROUND}{ARM}{}", scene(&format!("        a.ragdoll(strength: {strength})\n        a.lower.rotate_to(60.deg) over 0.5.seconds ease linear\n"), 3.0)))
    };
    let strong = run_with("1.0");
    let mut d = bone(&strong.snapshots[71], "lower").4 - bone(&strong.snapshots[71], "upper").4;
    d -= d.round();
    near(d, 60.0 / 360.0, 0.03); // the elbow holds the animated angle
    near(bone(&strong.snapshots[30], "upper").1, 200.0, 15.0); // and the arm stays up
    let weak = run_with("0.01");
    assert!(
        bone(&weak.snapshots[30], "upper").1 > 300.0,
        "a weak muscle lets the arm fall: {}",
        bone(&weak.snapshots[30], "upper").1
    );
}

#[test]
fn a_ragdoll_run_is_deterministic() {
    let src = format!(
        "{GROUND}{ARM}{}",
        scene("        a.ragdoll(push: (200, -100))\n", 2.0)
    );
    let (a, b) = (go(&src), go(&src));
    assert_eq!(a.snapshots[47], b.snapshots[47]);
}

#[test]
fn ragdoll_errors() {
    let e = run_src(&format!(
        "{GROUND}armature a:\n    bone upper: at: (300, 200), length: 100\n\n{}",
        scene("        a.ragdoll()\n", 1.0)
    ))
    .err()
    .expect("no ragdoll line");
    assert_eq!(e.code, "E-RIG-004");
    let e = run_src(&format!("{GROUND}{ARM}a.ragdoll()\n{}", scene("", 1.0)))
        .err()
        .expect("not running");
    assert_eq!(e.code, "E-RIG-004");
    let e = run_src(&format!(
        "{GROUND}{ARM}{}",
        scene("        a.ragdoll(strength: 2)\n", 1.0)
    ))
    .err()
    .expect("strength");
    assert_eq!(e.code, "E-RIG-004");
    let e = run_src(&format!("{GROUND}armature a:\n    bone upper: at: (300, 200), length: 100\n    ragdoll: floppiness: 3\n\n{}", scene("", 1.0))).err().expect("bad key");
    assert_eq!(e.code, "E-RIG-001");
    let e = run_src(&format!("{GROUND}armature a:\n    bone upper: at: (300, 200), length: 100\n    ragdoll: radius: -1\n\n{}", scene("", 1.0))).err().expect("bad radius");
    assert_eq!(e.code, "E-RIG-004");
}

#[test]
fn a_bone_can_have_its_own_radius() {
    let r = go(&format!("{GROUND}armature a:\n    bone upper: at: (300, 200), length: 100, angle: 90.deg, radius: 30\n    ragdoll: radius: 6\n\n{}", scene("        a.ragdoll()\n", 2.0)));
    // a vertical bone with a fat capsule comes to rest with its tip a bone-radius above the floor
    let b = bone(&r.snapshots[47], "upper");
    assert!(b.1 < 560.0 && b.3 < 560.0);
}
