//! Sub-round C: the primitives gait needs (`raycast`, IK handle reads, `self: Armature`) and the
//! bundled `std/gait` library. Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\nworld:\n    gravity: (0, 9.8)\n    scale: 100\n\n";
const GROUND: &str = "shape ground = rectangle(width: 2000, height: 40, at: (400, 580))\nground.body(kind: static)\n";

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
fn scene(main: &str, frames: u32) -> String {
    format!("scene \"s\" for {frames}.frames:\n    main:\n{main}        wait {frames}.frames\n")
}

// ---- raycast ----------------------------------------------------------------------------------

#[test]
fn raycast_returns_the_hit_point_and_normal() {
    let r = go(&format!("{GROUND}shape probe = square(size: 4, at: (0, 0))\nshape n = square(size: 4, at: (0, 0))\n{}", scene("        wait 1.frame\n        probe.move_to(raycast((100, 300), (0, 1), 1000))\n        n.move_to((50 + raycast_normal((100, 300), (0, 1), 1000).x * 10, 50 + raycast_normal((100, 300), (0, 1), 1000).y * 10))\n        wait 1.frame\n", 4)));
    let (p, n) = (centre(&r.snapshots[3], 1), centre(&r.snapshots[3], 2));
    near(p.0, 100.0, 1e-6);
    near(p.1, 560.0, 1e-6);
    near(n.0, 50.0, 1e-6);
    near(n.1, 40.0, 1e-6); // the normal points up: (0, -1)
}

#[test]
fn raycast_misses_give_none_and_respect_the_length() {
    let r = go(&format!("{GROUND}shape probe = square(size: 4, at: (0, 0))\n{}", scene("        wait 1.frame\n        let up = raycast((100, 300), (0, -1), 1000)\n        let short = raycast((100, 300), (0, 1), 100)\n        if up == none and short == none:\n            probe.move_to((50, 50))\n        wait 1.frame\n", 4)));
    near(centre(&r.snapshots[3], 1).0, 50.0, 1e-6);
}

#[test]
fn raycast_can_ignore_a_shape_or_an_armature() {
    let r = go(&format!("{GROUND}shape box = rectangle(width: 40, height: 40, at: (100, 400))\nbox.body(kind: static)\nshape probe = square(size: 4, at: (0, 0))\n{}", scene("        wait 1.frame\n        probe.move_to(raycast((100, 300), (0, 1), 1000, ignore: box))\n        wait 1.frame\n", 4)));
    near(centre(&r.snapshots[3], 2).1, 560.0, 1e-6);
    let r = go(&format!("{GROUND}shape box = rectangle(width: 40, height: 40, at: (100, 400))\nbox.body(kind: static)\nshape probe = square(size: 4, at: (0, 0))\n{}", scene("        wait 1.frame\n        probe.move_to(raycast((100, 300), (0, 1), 1000))\n        wait 1.frame\n", 4)));
    near(centre(&r.snapshots[3], 2).1, 380.0, 1e-6); // the top of the box
}

#[test]
fn raycast_ignores_the_bones_of_an_armature_when_asked() {
    // (also "bones": every armature)
    let arm = "armature a:\n    bone upper: at: (100, 350), length: 100, angle: 90.deg\n    ragdoll: radius: 6\n\n";
    let r = go(&format!("{GROUND}{arm}shape probe = square(size: 4, at: (0, 0))\nshape p2 = square(size: 4, at: (0, 0))\n{}", scene("        wait 1.frame\n        probe.move_to(raycast((100, 300), (0, 1), 1000))\n        p2.move_to(raycast((100, 300), (0, 1), 1000, ignore: a))\n        wait 1.frame\n", 4)));
    assert!(centre(&r.snapshots[3], 1).1 < 400.0, "hits the bone");
    near(centre(&r.snapshots[3], 2).1, 560.0, 1e-6);
    let r = go(&format!("{GROUND}{arm}shape probe = square(size: 4, at: (0, 0))\n{}", scene("        wait 1.frame\n        probe.move_to(raycast((100, 300), (0, 1), 1000, ignore: \"bones\"))\n        wait 1.frame\n", 4)));
    near(centre(&r.snapshots[3], 1).1, 560.0, 1e-6);
}

#[test]
fn raycast_errors() {
    let e = err(&format!(
        "{GROUND}{}",
        scene("        let h = raycast((0, 0), (0, 0), 100)\n", 2)
    ));
    assert_eq!(e.code, "E-PHY-006"); // a zero direction
    let e = err(&format!(
        "{GROUND}{}",
        scene("        let h = raycast((0, 0), (0, 1), -5)\n", 2)
    ));
    assert_eq!(e.code, "E-PHY-006");
}

// ---- IK reads and armature actions ------------------------------------------------------------

const LEG: &str = "armature a:\n    bone torso: at: (200, 300), length: 60, angle: -90.deg\n    bone up: parent: torso, at: 0.0, length: 50, angle: 170.deg\n    bone lo: parent: up, length: 50\n    ik foot: chain: [up, lo]\n\n";

#[test]
fn an_ik_handle_reports_its_target_and_weight() {
    let r = go(&format!("{LEG}shape probe = square(size: 4, at: (0, 0))\nshape w = square(size: 4, at: (0, 0))\n{}", scene("        a.foot.reach((260, 400))\n        wait 2.frames\n        probe.move_to(a.foot.target)\n        w.move_to((a.foot.weight * 100, 0))\n", 6)));
    let p = centre(&r.snapshots[5], 0);
    near(p.0, 260.0, 1e-6);
    near(p.1, 400.0, 1e-6);
    near(centre(&r.snapshots[5], 1).0, 100.0, 1e-6);
}

#[test]
fn an_action_can_take_an_armature_as_self() {
    let r = go(&format!("{LEG}shape probe = square(size: 4, at: (0, 0))\naction mark(self: Armature):\n    probe.move_to(self.torso.head)\n\n{}", scene("        a.mark()\n", 3)));
    near(centre(&r.snapshots[2], 0).0, 200.0, 1e-6);
    let e = err(&format!("{LEG}action mark(self: Armature):\n    wait 1.frame\n\nshape s = square(size: 4, at: (0, 0))\n{}", scene("        s.mark()\n", 3)));
    assert!(e.code.starts_with("E-"), "{}", e.code);
}

// ---- std/gait ---------------------------------------------------------------------------------

// a biped: the hips are the root bone `torso`, two straight legs, feet on the ground (top y = 560)
const BIPED: &str = "import \"std/gait\"\n\narmature hero:\n    bone torso: at: (200, 440), length: 100, angle: -90.deg\n    bone thigh_l: parent: torso, at: 0.0, length: 60, angle: -175.deg\n    bone shin_l: parent: thigh_l, length: 60\n    bone thigh_r: parent: torso, at: 0.0, length: 60, angle: 175.deg\n    bone shin_r: parent: thigh_r, length: 60\n    ik foot_l: chain: [thigh_l, shin_l]\n    ik foot_r: chain: [thigh_r, shin_r]\n    ragdoll: radius: 5\n\n";

fn feet(snap: &Value) -> Vec<(String, f64, f64)> {
    snap["rigs"][0]["ik"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| {
            (
                k["name"].as_str().unwrap().to_string(),
                k["target"][0].as_f64().unwrap(),
                k["target"][1].as_f64().unwrap(),
            )
        })
        .collect()
}
fn bone_tip(snap: &Value, name: &str) -> (f64, f64) {
    let b = snap["rigs"][0]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap();
    (b["tip"][0].as_f64().unwrap(), b["tip"][1].as_f64().unwrap())
}
fn head(snap: &Value, name: &str) -> (f64, f64) {
    let b = snap["rigs"][0]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap();
    (
        b["head"][0].as_f64().unwrap(),
        b["head"][1].as_f64().unwrap(),
    )
}
fn walk(extra: &str, call: &str, secs: u32) -> RunResult {
    go(&format!(
        "{GROUND}{extra}{BIPED}{}",
        scene(&format!("        {call}\n"), secs * 24)
    ))
}

#[test]
fn walk_to_moves_the_hips_to_the_target_at_the_speed() {
    let r = walk("", "hero.walk_to(500, speed: 120)", 6);
    let x0 = head(&r.snapshots[0], "torso").0;
    near(x0, 200.0, 6.0); // the first frame already moved one step
                          // 300 px at 120 px/s is 2.5 s; halfway (frame 30) about 150 px along
    let mid = head(&r.snapshots[30], "torso").0;
    near(mid, 200.0 + 120.0 * 30.0 / 24.0, 12.0);
    near(head(&r.snapshots[143], "torso").0, 500.0, 6.0);
}

#[test]
fn one_foot_swings_at_a_time_and_a_planted_foot_does_not_move() {
    let r = walk("", "hero.walk_to(500, speed: 120)", 6);
    let mut swings = [0usize; 2];
    for i in 4..60 {
        let (a, b) = (feet(&r.snapshots[i]), feet(&r.snapshots[i + 1]));
        let moved: Vec<bool> = (0..2)
            .map(|k| (a[k].1 - b[k].1).abs() + (a[k].2 - b[k].2).abs() > 1e-9)
            .collect();
        assert!(
            moved.iter().filter(|m| **m).count() <= 1,
            "both feet moved at frame {i}"
        );
        for k in 0..2 {
            swings[k] += moved[k] as usize;
        }
    }
    assert!(swings[0] > 6 && swings[1] > 6, "both feet step: {swings:?}");
}

#[test]
fn the_legs_reach_the_planted_feet() {
    let r = walk("", "hero.walk_to(500, speed: 120)", 6);
    for i in 10..60 {
        let f = feet(&r.snapshots[i]);
        for (k, shin) in ["shin_l", "shin_r"].iter().enumerate() {
            let tip = bone_tip(&r.snapshots[i], shin);
            let d = (tip.0 - f[k].1).hypot(tip.1 - f[k].2);
            assert!(d < 4.0, "frame {i}: the {shin} is {d} px from its target");
        }
    }
}

#[test]
fn planted_feet_are_on_the_ground_and_swinging_feet_are_lifted() {
    let r = walk("", "hero.walk_to(500, speed: 120)", 6);
    let (mut lifted, mut planted) = (0, 0);
    for i in 10..60 {
        let (a, b) = (feet(&r.snapshots[i]), feet(&r.snapshots[i + 1]));
        for k in 0..2 {
            let still = (a[k].1 - b[k].1).abs() < 1e-9 && (a[k].2 - b[k].2).abs() < 1e-9;
            if still {
                planted += 1;
                near(a[k].2, 560.0, 0.5);
            } else if a[k].2 < 552.0 {
                lifted += 1;
            }
        }
    }
    assert!(
        planted > 40 && lifted > 4,
        "planted {planted}, lifted {lifted}"
    );
}

#[test]
fn it_ends_standing_with_both_feet_under_the_hips() {
    let r = walk("", "hero.walk_to(500, speed: 120)", 8);
    let end = &r.snapshots[191];
    let hx = head(end, "torso").0;
    let f = feet(end);
    near(f[0].1, hx - 10.4, 6.0);
    near(f[1].1, hx + 10.4, 6.0);
    near(f[0].2, 560.0, 0.5);
    near(f[1].2, 560.0, 0.5);
    near(head(end, "torso").1, 440.0, 2.0); // and the hips are back at standing height
}

#[test]
fn it_walks_left_too() {
    let r = go(&format!(
        "{GROUND}{BIPED}{}",
        scene("        hero.walk_to(60, speed: 120)\n", 168)
    ));
    near(head(&r.snapshots[167], "torso").0, 60.0, 6.0);
}

#[test]
fn feet_land_on_stairs_and_the_hips_climb() {
    // three steps of 20 px: tops at y = 540, 520, 500 for x from 300, 380, 460
    let stairs = "shape s1 = rectangle(width: 80, height: 40, at: (340, 560))\ns1.body(kind: static)\nshape s2 = rectangle(width: 80, height: 80, at: (420, 560))\ns2.body(kind: static)\nshape s3 = rectangle(width: 400, height: 120, at: (700, 560))\ns3.body(kind: static)\n";
    let r = walk(stairs, "hero.walk_to(620, speed: 100)", 8);
    let end = &r.snapshots[191];
    let f = feet(end);
    near(f[0].2, 500.0, 0.5);
    near(f[1].2, 500.0, 0.5);
    near(head(end, "torso").1, 440.0 - 60.0, 3.0);
    // while crossing, every planted foot stands on a surface (one of the tops)
    for i in 10..150 {
        let (a, b) = (feet(&r.snapshots[i]), feet(&r.snapshots[i + 1]));
        for k in 0..2 {
            if (a[k].2 - b[k].2).abs() < 1e-9 && (a[k].1 - b[k].1).abs() < 1e-9 {
                let y = a[k].2;
                assert!(
                    [560.0, 540.0, 520.0, 500.0]
                        .iter()
                        .any(|t| (y - t).abs() < 0.5),
                    "frame {i}: foot at y {y}"
                );
            }
        }
    }
}

#[test]
fn a_walk_is_deterministic() {
    let a = walk("", "hero.walk_to(400, speed: 120)", 4);
    let b = walk("", "hero.walk_to(400, speed: 120)", 4);
    assert_eq!(a.snapshots[60], b.snapshots[60]);
}

// a quadruped: diagonal pairs step together
const DOG: &str = "import \"std/gait\"\n\narmature dog:\n    bone torso: at: (200, 450), length: 120, angle: 0.deg\n    bone fl_up: parent: torso, at: 1.0, length: 55, angle: 90.deg\n    bone fl_lo: parent: fl_up, length: 55\n    bone fr_up: parent: torso, at: 1.0, length: 55, angle: 90.deg\n    bone fr_lo: parent: fr_up, length: 55\n    bone bl_up: parent: torso, at: 0.0, length: 55, angle: 90.deg\n    bone bl_lo: parent: bl_up, length: 55\n    bone br_up: parent: torso, at: 0.0, length: 55, angle: 90.deg\n    bone br_lo: parent: br_up, length: 55\n    ik foot_fl: chain: [fl_up, fl_lo]\n    ik foot_fr: chain: [fr_up, fr_lo]\n    ik foot_bl: chain: [bl_up, bl_lo]\n    ik foot_br: chain: [br_up, br_lo]\n\n";

#[test]
fn a_quadruped_trots_with_diagonal_pairs() {
    let r = go(&format!(
        "{GROUND}{DOG}{}",
        scene("        dog.trot_to(500, speed: 140)\n", 144)
    ));
    near(head(&r.snapshots[143], "torso").0, 500.0, 8.0);
    let names: Vec<String> = feet(&r.snapshots[0]).iter().map(|f| f.0.clone()).collect();
    let idx = |n: &str| names.iter().position(|x| x == n).unwrap();
    let (fl, fr, bl, br) = (
        idx("foot_fl"),
        idx("foot_fr"),
        idx("foot_bl"),
        idx("foot_br"),
    );
    let (mut together, mut apart) = (0, 0);
    for i in 6..60 {
        let (a, b) = (feet(&r.snapshots[i]), feet(&r.snapshots[i + 1]));
        let mv = |k: usize| (a[k].1 - b[k].1).abs() + (a[k].2 - b[k].2).abs() > 1e-9;
        if mv(fl) {
            assert_eq!(
                mv(fl),
                mv(br),
                "frame {i}: the diagonal pair fl/br moves together"
            );
            assert!(!mv(fr) && !mv(bl), "frame {i}: the other pair is planted");
            together += 1;
        } else if mv(fr) {
            apart += 1;
        }
    }
    assert!(
        together > 4 && apart > 4,
        "both pairs step: {together} {apart}"
    );
}
