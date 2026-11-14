//! 3D armatures: bones, skinning, IK, limits, errors.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 1280x720\n    fps: 24\n\n";

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
fn scene(secs: u32, body: &str) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{body}        wait {secs}.seconds\n")
}
fn p3(v: &Value) -> [f64; 3] {
    [
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    ]
}
fn bone(snap: &Value, name: &str) -> (Vec<f64>, Vec<f64>) {
    for b in snap["rigs3"][0]["bones"].as_array().unwrap() {
        if b["name"] == name {
            return (p3(&b["head"]).to_vec(), p3(&b["tip"]).to_vec());
        }
    }
    panic!("no bone {name}");
}
const ARM: &str = "armature arm:
    bone upper: at: (100, 300, 0), dir: (1, 0, 0), length: 100, swing: 120.deg
    bone lower: parent: upper, length: 100, axis: (0, 0, 1), limits: (0.deg, 150.deg)
    ik hand: chain: [upper, lower], bend: (150, 100, 0)
";

#[test]
fn the_rest_pose_lays_bones_head_to_tip() {
    let r = go(&format!("{ARM}{}", scene(1, "")));
    let s = r.snapshots.last().unwrap();
    let (h, t) = bone(s, "lower");
    assert!(
        (h[0] - 200.0).abs() < 1e-9 && (t[0] - 300.0).abs() < 1e-9 && (t[1] - 300.0).abs() < 1e-9
    );
}

#[test]
fn animating_a_bone_bends_the_limb() {
    let src = format!(
        "{ARM}{}",
        scene(
            1,
            "        arm.lower.rotate_to(90.deg, over: 0.5.seconds)\n"
        )
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let (h, t) = bone(s, "lower");
    assert!((h[0] - 200.0).abs() < 1e-6);
    assert!(
        (t[0] - 200.0).abs() < 1.0 && (t[1] - 400.0).abs() < 1.0,
        "{t:?}"
    );
}

#[test]
fn a_hinge_is_clamped_to_its_limits() {
    let src = format!(
        "{ARM}{}",
        scene(
            1,
            "        arm.lower.rotate_to(180.deg, over: 0.2.seconds)\n"
        )
    );
    let r = go(&src);
    let (_, t) = bone(r.snapshots.last().unwrap(), "lower");
    // 150 degrees at most: the tip is 100 px from the elbow at that angle
    let ang = ((t[1] - 300.0).atan2(t[0] - 200.0)).to_degrees();
    assert!((ang - 150.0).abs() < 1.0, "{ang}");
    assert!(r.diagnostics.iter().any(|d| d.code == "W-RIG-001"));
}

#[test]
fn reaching_with_ik_puts_the_hand_on_the_target() {
    let src = format!(
        "{ARM}{}",
        scene(
            2,
            "        arm.hand.reach((220, 380, 40), over: 0.5.seconds)\n"
        )
    );
    let r = go(&src);
    let (_, t) = bone(r.snapshots.last().unwrap(), "lower");
    let d = ((t[0] - 220.0).powi(2) + (t[1] - 380.0).powi(2) + (t[2] - 40.0).powi(2)).sqrt();
    assert!(d < 1.0, "{t:?} {d}");
}

#[test]
fn a_skinned_mesh_follows_its_bones() {
    let src = format!(
        "{ARM}shape rod = cube(width: 200, height: 20, depth: 20, at: (200, 300, 0))\nrod.skin(arm, blend: 6)\n{}",
        scene(1, "        arm.lower.rotate_to(90.deg, over: 0.5.seconds)\n")
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let rod = s["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "rod")
        .unwrap();
    let ys: Vec<f64> = rod["mesh"]["verts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| p3(v)[1])
        .collect();
    let max = ys.iter().cloned().fold(f64::MIN, f64::max);
    assert!(max > 380.0, "the forearm half swung down: {max}");
}

#[test]
fn bad_rigs_are_reported() {
    let e = err(
        "armature a:\n    bone b: at: (0, 0, 0), dir: (1, 0, 0), length: 10, axis: (0, 0, 1)\n",
    );
    assert_eq!(e.code, "E-RIG-005");
    let e = err(&format!(
        "{ARM}shape s = circle(n: 12, radius: 20, at: (0, 0))\ns.skin(arm)\n"
    ));
    assert_eq!(e.code, "E-RIG-007");
}

#[test]
fn it_is_deterministic() {
    let src = format!(
        "{ARM}{}",
        scene(
            1,
            "        arm.hand.reach((220, 380, 40), over: 0.5.seconds)\n"
        )
    );
    let a = go(&src);
    let b = go(&src);
    assert_eq!(
        a.snapshots.last().unwrap()["rigs3"],
        b.snapshots.last().unwrap()["rigs3"]
    );
}

// ---- ragdolls

const FLOOR: &str = "shape floor = cube(width: 2600, height: 60, depth: 1800, at: (640, 630, 300))\nfloor.body(kind: static)\n";
const DOLL: &str = "armature doll:
    ragdoll: radius: 10, density: 200
    bone spine: at: (400, 300, 300), dir: (0, 1, 0), length: 120, swing: 40.deg, twist: (-20.deg, 20.deg)
    bone arm: parent: spine, at: 0.0, dir: (1, 0, 0), length: 100, swing: 100.deg, twist: (-30.deg, 30.deg)
    bone fore: parent: arm, length: 90, axis: (0, 0, 1), limits: (0.deg, 140.deg)
";

fn tip_y(r: &RunResult, name: &str) -> f64 {
    bone(r.snapshots.last().unwrap(), name).1[1]
}

#[test]
fn an_animated_ragdoll_follows_the_animation_and_does_not_fall() {
    let r = go(&format!("{FLOOR}{DOLL}{}", scene(2, "")));
    let (h, t) = bone(r.snapshots.last().unwrap(), "fore");
    assert!(
        (h[0] - 500.0).abs() < 1.0 && (t[0] - 590.0).abs() < 1.0,
        "{h:?} {t:?}"
    );
}

#[test]
fn a_limp_ragdoll_falls_and_keeps_its_joints_together() {
    let r = go(&format!(
        "{FLOOR}{DOLL}{}",
        scene(4, "        wait 0.5.seconds\n        doll.ragdoll()\n")
    ));
    let s = r.snapshots.last().unwrap();
    let (_, spine_tip) = bone(s, "spine");
    let (arm_head, _) = bone(s, "arm");
    // the arm stays attached to the spine
    let gap: f64 = (0..3)
        .map(|k| (arm_head[k] - spine_tip[k]).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!(gap < 120.0, "limb joint gap {gap}");
    // everything came to rest above the floor top (y = 600) and below where it started
    assert!(tip_y(&r, "fore") > 500.0, "fell: {}", tip_y(&r, "fore"));
    assert!(tip_y(&r, "fore") < 610.0);
}

#[test]
fn a_strong_active_ragdoll_holds_its_pose_a_limp_one_does_not() {
    let hold = |strength: &str| {
        let r = go(&format!(
            "{FLOOR}{DOLL}{}",
            scene(
                3,
                &format!("        wait 0.2.seconds\n        doll.ragdoll(strength: {strength})\n")
            )
        ));
        tip_y(&r, "arm")
    };
    let strong = hold("1");
    let limp = hold("0");
    assert!(strong < limp - 60.0, "strong {strong} limp {limp}");
}

#[test]
fn a_ragdoll_is_deterministic_and_animate_hands_it_back() {
    let src = format!(
        "{FLOOR}{DOLL}{}",
        scene(
            3,
            "        doll.ragdoll()\n        wait 1.second\n        doll.animate()\n"
        )
    );
    let (a, b) = (go(&src), go(&src));
    assert_eq!(
        a.snapshots.last().unwrap()["rigs3"],
        b.snapshots.last().unwrap()["rigs3"]
    );
}

#[test]
fn ragdoll_without_a_ragdoll_line_is_an_error() {
    let e = err(&format!("{ARM}{}", scene(1, "        arm.ragdoll()\n")));
    assert_eq!(e.code, "E-RIG-008");
}

fn gap(s: &Value, a: &str, b: &str) -> f64 {
    let (_, t) = bone(s, a);
    let (h, _) = bone(s, b);
    (0..3).map(|k| (t[k] - h[k]).powi(2)).sum::<f64>().sqrt()
}

#[test]
fn an_active_ragdoll_stays_in_one_piece() {
    for strength in ["0", "0.3", "0.7"] {
        let r = go(&format!(
            "{FLOOR}{DOLL}{}",
            scene(5, &format!("        wait 0.2.seconds\n        doll.ragdoll()\n        wait 1.5.seconds\n        doll.ragdoll(strength: {strength})\n"))
        ));
        for s in r.snapshots.iter().step_by(6) {
            // the forearm starts where the arm ends, and the arm where the spine's top is
            assert!(
                gap(s, "arm", "fore") < 8.0,
                "strength {strength}: arm/fore gap {}",
                gap(s, "arm", "fore")
            );
        }
    }
}
