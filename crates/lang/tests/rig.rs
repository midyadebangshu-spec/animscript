//! Armatures in the language: bones as nodes, FK poses through ordinary actions and keyframes, IK
//! handles, `bind` and `skin`, limits, layering and errors. Written first.
use animscript_lang::*;
use serde_json::Value;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";

fn go(body: &str) -> RunResult {
    let text = format!("{PROJECT}{body}");
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(body: &str) -> Diagnostic {
    match run(&format!("{PROJECT}{body}"), &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn bone(snap: &Value, arm: usize, name: &str) -> (f64, f64, f64, f64, f64) {
    let b = snap["rigs"][arm]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no bone {name}"));
    (
        b["head"][0].as_f64().unwrap(),
        b["head"][1].as_f64().unwrap(),
        b["tip"][0].as_f64().unwrap(),
        b["tip"][1].as_f64().unwrap(),
        b["angle"].as_f64().unwrap(),
    )
}
fn verts(snap: &Value, shape: usize) -> Vec<(f64, f64)> {
    snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect()
}
fn centre(snap: &Value, shape: usize) -> (f64, f64) {
    let v = verts(snap, shape);
    (
        v.iter().map(|p| p.0).sum::<f64>() / v.len() as f64,
        v.iter().map(|p| p.1).sum::<f64>() / v.len() as f64,
    )
}
const ARM: &str = "armature a:\n    bone upper: at: (200, 300), length: 100, angle: 0.deg\n    bone lower: parent: upper, length: 100, limits: (0.deg, 150.deg)\n    ik hand: chain: [upper, lower]\n\n";
fn scene(main: &str, frames: u32) -> String {
    format!("scene \"s\" for {frames}.frames:\n    main:\n{main}        wait {frames}.frames\n")
}

#[test]
fn the_snapshot_reports_the_bones_in_pixels_and_turns() {
    let r = go(&format!("{ARM}{}", scene("", 2)));
    let up = bone(&r.snapshots[0], 0, "upper");
    assert_eq!(
        ((up.0, up.1), (up.2, up.3)),
        ((200.0, 300.0), (300.0, 300.0))
    );
    near(up.4, 0.0, 1e-12);
    let lo = bone(&r.snapshots[0], 0, "lower");
    assert_eq!(
        ((lo.0, lo.1), (lo.2, lo.3)),
        ((300.0, 300.0), (400.0, 300.0))
    );
    assert_eq!(r.snapshots[0]["rigs"][0]["name"], "a");
    assert!(go(
        "shape s = square(size: 10)\nscene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n"
    )
    .snapshots[0]
        .get("rigs")
        .is_none());
}

#[test]
fn rotating_a_bone_is_forward_kinematics() {
    let r = go(&format!(
        "{ARM}{}",
        scene("        a.upper.rotate_to(90.deg) over 1.frame\n", 3)
    ));
    let up = bone(&r.snapshots[2], 0, "upper");
    near(up.2, 200.0, 1e-6);
    near(up.3, 400.0, 1e-6); // the upper arm now points down
    let lo = bone(&r.snapshots[2], 0, "lower");
    near((lo.2, lo.3).0, 200.0, 1e-6);
    near(lo.3, 500.0, 1e-6); // and carries the forearm with it
    near(lo.4, 0.25, 1e-9);
}

#[test]
fn limits_clamp_the_pose() {
    let r = go(&format!(
        "{ARM}{}",
        scene("        a.lower.rotate_to(200.deg) over 1.frame\n", 3)
    ));
    near(bone(&r.snapshots[2], 0, "lower").4, 150.0 / 360.0, 1e-9);
    assert!(
        r.diagnostics.iter().any(|d| d.code == "W-RIG-001"),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn keyframes_on_a_bone_are_a_clip() {
    let r = go(&format!("{ARM}scene \"s\" for 1.second:\n    main:\n        a.upper.keyframes over 1.second:\n            at 0.s: rotation = 0.deg\n            at 1.s: rotation = 90.deg\n        wait 1.second\n"));
    near(bone(&r.snapshots[12], 0, "upper").4, 0.125, 1e-9); // half-way through
}

#[test]
fn a_bound_shape_follows_its_bone() {
    let r = go(&format!(
        "{ARM}shape bar = rectangle(width: 100, height: 20, at: (250, 300))\nbar.bind(a.upper)\n{}",
        scene("        a.upper.rotate_to(90.deg) over 1.frame\n", 3)
    ));
    let (a, b) = (centre(&r.snapshots[0], 0), centre(&r.snapshots[2], 0));
    near(a.0, 250.0, 1e-6);
    near(b.0, 200.0, 1e-6);
    near(b.1, 350.0, 1e-6); // swung a quarter turn about the shoulder at (200, 300)
}

#[test]
fn skinning_bends_a_mesh_between_two_bones() {
    let r = go(&format!("{ARM}shape strip = rectangle(width: 200, height: 20, at: (300, 300))\nstrip.f1.subdivide(4)\nstrip.skin(a, blend: 6)\n{}", scene("        a.lower.rotate_to(90.deg) over 1.frame\n", 3)));
    let (rest, bent) = (verts(&r.snapshots[0], 0), verts(&r.snapshots[2], 0));
    let (far_rest, far) = (
        rest.iter()
            .cloned()
            .fold((f64::MIN, 0.0), |m, p| if p.0 > m.0 { p } else { m }),
        bent.iter()
            .zip(&rest)
            .filter(|(_, r)| r.0 > 395.0)
            .map(|(b, _)| *b)
            .fold((0.0, f64::MIN), |m, p| if p.1 > m.1 { p } else { m }),
    );
    near(far_rest.0, 400.0, 1e-6);
    assert!(far.1 > 380.0, "the far end swung down: {far:?}");
    for (b, r) in bent.iter().zip(&rest) {
        if r.0 < 215.0 {
            near(b.0, r.0, 1.0);
            near(b.1, r.1, 1.0); // the shoulder end stays where it was
        }
    }
}

#[test]
fn ik_reaches_a_point_and_release_returns_to_the_pose() {
    let r = go(&format!("{ARM}scene \"s\" for 6.frames:\n    main:\n        a.hand.reach((250, 380)) over 1.frame\n        wait 3.frames\n        a.hand.release() over 1.frame\n        wait 2.frames\n"));
    let lo = bone(&r.snapshots[2], 0, "lower");
    near(lo.2, 250.0, 1e-3);
    near(lo.3, 380.0, 1e-3);
    let end = bone(&r.snapshots[5], 0, "lower");
    near(end.2, 400.0, 1e-3); // back to the plain pose
    near(end.3, 300.0, 1e-3);
    // an unreachable point stretches the arm straight towards it
    let r = go(&format!(
        "{ARM}{}",
        scene("        a.hand.reach((600, 300)) over 1.frame\n", 3)
    ));
    let lo = bone(&r.snapshots[2], 0, "lower");
    near(lo.2, 400.0, 1e-3);
    near(lo.3, 300.0, 1e-3);
}

#[test]
fn keyframes_on_the_art_still_win_over_the_rig() {
    // bound to the forearm, whose head swings from (300, 300) to (200, 400) when the upper arm turns
    let body = |keyed: bool| {
        let timeline = if keyed {
            "    timeline:\n        at 0.s:\n            bar.position.x = 0\n        at 1.s:\n            bar.position.x = 0\n"
        } else {
            ""
        };
        format!("{ARM}shape bar = rectangle(width: 100, height: 20, at: (350, 300))\nbar.bind(a.lower)\nscene \"s\" for 4.frames:\n{timeline}    main:\n        a.upper.rotate_to(90.deg) over 1.frame\n        wait 3.frames\n")
    };
    let free = centre(&go(&body(false)).snapshots[3], 0);
    near(free.0, 200.0, 1e-6); // the rig carries the bar to (200, 450)
    near(free.1, 450.0, 1e-6);
    let keyed = centre(&go(&body(true)).snapshots[3], 0);
    near(keyed.0, 300.0, 1e-6); // the key holds position.x at 0: the rig no longer decides it
    near(keyed.1, 450.0, 1e-6); // but still decides the rest
}

#[test]
fn the_next_scene_starts_from_the_final_pose_and_replays_are_identical() {
    let s = format!("{ARM}scene \"a\" for 3.frames:\n    main:\n        a.upper.rotate_to(90.deg) over 1.frame\n        wait 3.frames\nscene \"b\" for 2.frames:\n    main:\n        wait 2.frames\n");
    let r = go(&s);
    near(bone(&r.snapshots[3], 0, "upper").4, 0.25, 1e-9);
    assert_eq!(go(&s).hashes, r.hashes);
}

#[test]
fn bad_armatures_are_rejected() {
    let sc = scene("", 1);
    assert_eq!(err(&format!("armature a:\n    bone x: at: (0, 0), length: 10\n    bone x: parent: x, length: 5\n{sc}")).code, "E-RIG-001"); // duplicate
    assert_eq!(
        err(&format!(
            "armature a:\n    bone x: at: (0, 0), length: 0\n{sc}"
        ))
        .code,
        "E-RIG-001"
    );
    assert_eq!(err(&format!("armature a:\n    bone y: parent: x, length: 5\n    bone x: at: (0, 0), length: 10\n{sc}")).code, "E-RIG-001"); // parent after child
    assert_eq!(
        err(&format!(
            "armature a:\n    bone x: at: (0, 0), length: 10, wings: 2\n{sc}"
        ))
        .code,
        "E-RIG-001"
    );
    assert_eq!(
        err(&format!("armature a:\n    ik z: chain: [x, y]\n{sc}")).code,
        "E-RIG-001"
    ); // no bones
    assert_eq!(err(&format!("armature a:\n    bone x: at: (0, 0), length: 10\n    bone y: parent: x, length: 5, limits: (90.deg, 10.deg)\n{sc}")).code, "E-RIG-002");
    assert_eq!(err(&format!("armature a:\n    bone x: at: (0, 0), length: 10\n    bone y: parent: x, length: 5\n    ik h: chain: [x]\n{sc}")).code, "E-RIG-002"); // a chain needs two bones
    assert_eq!(err(&format!("armature a:\n    bone x: at: (0, 0), length: 10\n    bone y: at: (50, 0), length: 5\n    ik h: chain: [x, y]\n{sc}")).code, "E-RIG-002"); // not connected
    assert_eq!(
        err(&format!("{ARM}{sc}")
            .replace(
                "armature a:",
                "armature a:\n    bone w: at: (0, 0), length: 10\n"
            )
            .replace("[upper, lower]", "[upper, nope]"))
        .code,
        "E-RIG-002"
    );
    assert_eq!(
        err(&format!(
            "{ARM}shape s = square(size: 10)\ns.body()\ns.bind(a.upper)\n{sc}"
        ))
        .code,
        "E-RIG-003"
    );
    assert_eq!(
        err(&format!(
            "{ARM}shape s = square(size: 10)\ns.bind(a.upper)\ns.skin(a)\n{sc}"
        ))
        .code,
        "E-RIG-003"
    );
    assert_eq!(
        err(&format!(
            "{ARM}shape s = square(size: 10)\ns.bind(a.upper)\ns.bind(a.lower)\n{sc}"
        ))
        .code,
        "E-RIG-003"
    );
    assert_eq!(
        err(&format!("{ARM}shape s = square(size: 10)\ns.bind(s)\n{sc}")).code,
        "E-RIG-003"
    ); // not a bone
    assert_eq!(err(&format!("{ARM}shape s = square(size: 10)\nscene \"s\" for 1.frame:\n    main:\n        s.bind(a.upper)\n        wait 1.frame\n")).code, "E-SHP-003");
    assert_eq!(
        err(&format!(
            "{ARM}shape s = square(size: 10)\nshape t = square(size: 10)\nt.bind(a.nope)\n{sc}"
        ))
        .code,
        "E-NAM-003"
    );
}

#[test]
fn bone_head_tip_and_angle_can_be_read() {
    let r = go(&format!("{ARM}shape probe = square(size: 10, at: (0, 0))\naction watch():\n    wait 2.frames\n    probe.move_to(a.lower.tip)\n\nscene \"s\" for 6.frames:\n    main:\n        a.upper.rotate_to(90.deg) over 1.frame\n        watch()\n        wait 3.frames\n"));
    let c = centre(r.snapshots.last().unwrap(), 0);
    near(c.0, 200.0, 1e-6); // the forearm's tip after the upper arm turned down
    near(c.1, 500.0, 1e-6);
    let at_rest = go(&format!(
        "{ARM}shape probe = square(size: 10, at: (0, 0))\nprobe.move_to(a.upper.head)\n{}",
        scene("", 2)
    ));
    let c = centre(&at_rest.snapshots[0], 0);
    near(c.0, 200.0, 1e-6);
    near(c.1, 300.0, 1e-6);
}
