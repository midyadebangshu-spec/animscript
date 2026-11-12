//! Physics in the language: `world:`, `.body()`, joints, events and how bodies share the layer
//! stack with actions and keyframes. Written before the implementation.
use animscript_lang::*;
use serde_json::Value;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";
const WORLD: &str = "world:\n    gravity: (0, 9.8)\n    scale: 100\n\n";

fn src(s: &str) -> String {
    format!("{PROJECT}{WORLD}{s}")
}
fn go(s: &str) -> RunResult {
    let text = src(s);
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(s: &str) -> Diagnostic {
    match run(&src(s), &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
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
fn max_y(snap: &Value, shape: usize) -> f64 {
    verts(snap, shape)
        .iter()
        .map(|p| p.1)
        .fold(f64::MIN, f64::max)
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}

const BALL_AND_GROUND: &str = "shape ball = circle(16, radius: 25, at: (400, 100))\nshape ground = rectangle(width: 800, height: 40, at: (400, 580))\nground.body(kind: static)\nball.body()\n";

#[test]
fn a_dynamic_body_falls_with_the_world_gravity() {
    // 9.8 m/s^2 at 100 px/m is 980 px/s^2: after one second a body has fallen 490 px
    let r = go("shape b = square(size: 20, at: (400, 0))\nb.body()\nscene \"s\" for 1.second:\n    main:\n        wait 1.second\n");
    let y0 = centre(&r.snapshots[0], 0).1;
    let y1 = centre(&r.snapshots[23], 0).1; // 23 steps = 23/24 s
    near(y0, 0.0, 1e-9);
    let t = 23.0 / 24.0;
    near(y1, 0.5 * 980.0 * t * t, 0.5 * 980.0 * t * t * 0.04);
    near(centre(&r.snapshots[23], 0).0, 400.0, 1e-6);
}

#[test]
fn a_ball_comes_to_rest_on_the_ground() {
    let r = go(&format!(
        "{BALL_AND_GROUND}scene \"s\" for 4.seconds:\n    main:\n        wait 4.seconds\n"
    ));
    let last = r.snapshots.last().unwrap();
    near(max_y(last, 0), 560.0, 2.0); // the ground's top edge is at y = 560
    near(centre(last, 0).0, 400.0, 2.0);
    let (a, b) = (centre(&r.snapshots[90], 0).1, centre(last, 0).1);
    near(a, b, 0.5); // and it stays there
}

#[test]
fn restitution_makes_the_ball_bounce() {
    let r = go("shape ball = circle(16, radius: 25, at: (400, 100))\nshape ground = rectangle(width: 800, height: 40, at: (400, 580))\nground.body(kind: static, restitution: 0.8)\nball.body(restitution: 0.8)\nscene \"s\" for 3.seconds:\n    main:\n        wait 3.seconds\n");
    let ys: Vec<f64> = r.snapshots.iter().map(|s| centre(s, 0).1).collect();
    let first_hit = ys.iter().position(|y| *y > 520.0).unwrap();
    let apex = ys[first_hit + 2..].iter().cloned().fold(f64::MAX, f64::min);
    assert!(apex < 400.0, "it rose again after the first impact: {apex}");
    assert!(apex > 150.0);
}

#[test]
fn touching_is_a_trigger() {
    let text = src(&format!("{BALL_AND_GROUND}scene \"s\" for 2.seconds:\n    when ball.touches(ground):\n        ball.fill_to(#ff0000) over 1.frame\n"));
    let r = run(&text, &RunOptions { trace: true })
        .unwrap_or_else(|d| panic!("{}", d.render(Some(&text), "t")));
    let fire: Vec<&String> = r.trace.iter().filter(|l| l.contains("FIRE")).collect();
    assert_eq!(fire.len(), 1, "{fire:?}");
    // the ball is about 435 px above its landing: sqrt(2 * 4.35 / 9.8) = 0.94 s, so tick ~22
    let tick: i64 = fire[0].split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!((20..=26).contains(&tick), "{tick}");
}

#[test]
fn impulses_velocity_speed_and_awake_can_be_used() {
    let r = go("shape ball = circle(16, radius: 25, at: (400, 300))\nball.body(gravity_scale: 0)\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 1.second:\n    setup:\n        ball.set_velocity((120, 0))\n        ball.apply_impulse((0, -60))\n    main:\n        probe.move_to((ball.velocity.x, ball.speed))\n        wait 1.frame\n");
    // the ball has mass pi * 0.25^2 = 0.196 kg at density 1, so the impulse adds -60/0.196 px/s
    let p = centre(&r.snapshots[1], 1);
    near(p.0, 120.0, 1e-6);
    let vy = -60.0 / polygon_mass();
    near(p.1, (120.0f64.powi(2) + vy.powi(2)).sqrt(), 0.5);
}

fn polygon_mass() -> f64 {
    // area of a regular 16-gon of circumradius 0.25 m
    0.5 * 16.0 * 0.25f64 * 0.25 * (std::f64::consts::TAU / 16.0).sin()
}

#[test]
fn the_same_program_gives_the_same_hashes() {
    let s =
        format!("{BALL_AND_GROUND}scene \"s\" for 2.seconds:\n    main:\n        wait 2.seconds\n");
    let (a, b) = (go(&s), go(&s));
    assert_eq!(a.hashes, b.hashes);
    assert_ne!(a.hashes[3], a.hashes[10], "the world is moving");
}

#[test]
fn a_pendulum_keeps_its_arm_length() {
    let r = go("shape pivot = square(size: 10, at: (400, 100))\nshape bob = square(size: 20, at: (600, 100))\npivot.body(kind: static)\nbob.body()\nlet hinge = revolute(pivot, bob, anchor: (400, 100))\nscene \"s\" for 3.seconds:\n    main:\n        wait 3.seconds\n");
    for s in r.snapshots.iter().step_by(6) {
        let (p, b) = (centre(s, 0), centre(s, 1));
        near(
            ((b.0 - p.0).powi(2) + (b.1 - p.1).powi(2)).sqrt(),
            200.0,
            3.0,
        );
    }
    assert!(centre(&r.snapshots[24], 1).1 > 200.0, "the bob swung down");
}

#[test]
fn a_spring_pulls_to_its_rest_length() {
    let r = go("shape a = square(size: 10, at: (400, 100))\nshape b = square(size: 20, at: (400, 400))\na.body(kind: static)\nb.body(gravity_scale: 0)\nlet s = distance(a, b, length: 150, spring: (2, 1))\nscene \"s\" for 4.seconds:\n    main:\n        wait 4.seconds\n");
    let (p, b) = (
        centre(r.snapshots.last().unwrap(), 0),
        centre(r.snapshots.last().unwrap(), 1),
    );
    near(b.1 - p.1, 150.0, 3.0);
}

#[test]
fn a_moving_kinematic_platform_carries_a_box() {
    let r = go("shape plat = rectangle(width: 400, height: 20, at: (300, 400))\nshape box = square(size: 40, at: (200, 370))\nplat.body(kind: kinematic, friction: 1.0)\nbox.body(friction: 1.0)\nscene \"s\" for 2.seconds:\n    main:\n        plat.move_by(200, 0) over 2.seconds ease linear\n");
    let last = r.snapshots.last().unwrap();
    near(centre(last, 0).0, 300.0 + 200.0 * 47.0 / 48.0, 4.0);
    assert!(
        centre(last, 1).0 > 340.0,
        "the box rode along: {}",
        centre(last, 1).0
    );
    near(max_y(last, 1), 390.0, 3.0); // and stayed on top
}

#[test]
fn keyframes_win_over_physics() {
    let r = go("shape b = square(size: 20, at: (400, 100))\nb.body()\nscene \"s\" for 1.second:\n    timeline:\n        at 0.s:\n            b.position.y = 0\n        at 1.s:\n            b.position.y = 0\n");
    for s in &r.snapshots {
        near(centre(s, 0).1, 100.0, 1e-6);
    }
}

#[test]
fn the_next_scene_starts_where_the_last_one_ended() {
    let r = go("shape b = square(size: 20, at: (400, 0))\nb.body()\nscene \"a\" for 12.frames:\n    main:\n        wait 12.frames\nscene \"b\" for 12.frames:\n    main:\n        wait 12.frames\n");
    near(
        centre(&r.snapshots[12], 0).1,
        centre(&r.snapshots[11], 0).1,
        1e-9,
    );
    assert!(centre(&r.snapshots[23], 0).1 > centre(&r.snapshots[12], 0).1 + 100.0);
}

#[test]
fn the_snapshot_reports_contacts_and_sleeping() {
    let r = go(&format!(
        "{BALL_AND_GROUND}scene \"s\" for 4.seconds:\n    main:\n        wait 4.seconds\n"
    ));
    let first = &r.snapshots[0];
    assert!(first["physics"]["contacts"].as_array().unwrap().is_empty());
    let last = r.snapshots.last().unwrap();
    assert!(!last["physics"]["contacts"].as_array().unwrap().is_empty());
    let bodies = last["physics"]["bodies"].as_array().unwrap();
    let ball = bodies.iter().find(|b| b["shape"] == "ball").unwrap();
    assert_eq!(ball["awake"], false);
    assert_eq!(ball["kind"], "dynamic");
    // programs without physics have no such section
    let plain = go(
        "shape s = square(size: 10)\nscene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n",
    );
    assert!(plain.snapshots[0].get("physics").is_none());
}

#[test]
fn overlapping_bodies_at_the_start_warn() {
    let r = go("shape a = square(size: 100, at: (300, 300))\nshape b = square(size: 100, at: (350, 300))\na.body(gravity_scale: 0)\nb.body(gravity_scale: 0)\nscene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n");
    assert!(
        r.diagnostics.iter().any(|d| d.code == "W-PHY-001"),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn bad_bodies_and_joints_are_rejected() {
    assert_eq!(err("shape l = line((0, 0), (100, 0))\nl.body()\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PHY-001"); // no area
    assert_eq!(err("shape s = square(size: 10)\ns.scale = (2, 1)\ns.body()\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PHY-001"); // non-uniform scale
    assert_eq!(err("shape s = square(size: 10)\ns.body(kind: \"floaty\")\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PHY-001");
    assert_eq!(err("shape s = square(size: 10)\nshape t = square(size: 10)\ns.body()\nlet j = revolute(s, t, anchor: (0, 0))\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PHY-002"); // t is not a body
    assert_eq!(err("shape s = square(size: 10)\nshape t = square(size: 10, at: (50, 0))\ns.body()\nt.body()\nlet j = revolute(s, t, anchor: (0, 0), limits: (1.turn, 0.turn))\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PHY-002");
    assert_eq!(err("shape s = square(size: 10)\nscene \"s\" for 1.frame:\n    main:\n        s.body()\n        wait 1.frame\n").code, "E-SHP-003");
    // bodies are made while building
}

#[test]
fn a_fast_bounce_still_counts_as_touching_for_that_tick() {
    // restitution 0.8: each contact begins and ends within one frame, yet every landing must fire the rule
    let text = src("shape ball = circle(16, radius: 25, at: (400, 100))\nshape ground = rectangle(width: 800, height: 40, at: (400, 580))\nground.body(kind: static, restitution: 0.8)\nball.body(restitution: 0.8)\nscene \"s\" for 6.seconds:\n    whenever ball.touches(ground):\n        ball.fill_to(#ff0000) over 1.frame\n");
    let r = run(&text, &RunOptions { trace: true })
        .unwrap_or_else(|d| panic!("{}", d.render(Some(&text), "t")));
    let fires: Vec<i64> = r
        .trace
        .iter()
        .filter(|l| l.contains("FIRE"))
        .map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap())
        .collect();
    assert!(fires.len() >= 3, "{fires:?}");
    assert!((20..=27).contains(&fires[0]), "{fires:?}");
}
