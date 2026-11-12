//! The character controller: `body.controller(...)`, `.move(dir)`, `.jump()`, `.grounded`.
//! Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\nworld:\n    gravity: (0, 9.8)\n    scale: 100\n\n";
// ground top at y = 560; the hero stands on it (a 40x80 box, centre y = 520)
const SET: &str = "shape ground = rectangle(width: 2000, height: 40, at: (400, 580))\nground.body(kind: static)\nshape hero = rectangle(width: 40, height: 80, at: (100, 519))\nhero.body()\nhero.controller(speed: 200, accel: 2000, jump: 400)\n";

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

#[test]
fn a_controlled_body_stands_still_without_input() {
    let r = go(&format!(
        "{SET}scene \"s\" for 2.seconds:\n    main:\n        wait 2.seconds\n"
    ));
    let (x, y) = centre(&r.snapshots[47], 1);
    near(x, 100.0, 0.5);
    near(y, 520.0, 2.0);
}

#[test]
fn move_walks_at_the_speed_and_zero_stops() {
    let r = go(&format!("{SET}scene \"s\" for 3.seconds:\n    main:\n        hero.move(1)\n        wait 1.second\n        hero.move(0)\n        wait 2.seconds\n"));
    // 1 s at 200 px/s, give or take the acceleration ramp (200 / 2000 = 0.1 s) and the braking
    let at_1s = centre(&r.snapshots[24], 1).0;
    near(at_1s, 100.0 + 200.0 - 10.0, 8.0);
    let end = centre(&r.snapshots[71], 1).0;
    assert!(
        end >= at_1s && end < at_1s + 12.0,
        "braked within the ramp: {at_1s} -> {end}"
    );
    assert!(
        centre(&r.snapshots[71], 1).0 - centre(&r.snapshots[60], 1).0 < 0.5,
        "stopped"
    );
}

#[test]
fn a_negative_direction_walks_left() {
    let r = go(&format!(
        "{SET}scene \"s\" for 1.second:\n    main:\n        hero.move(-1)\n        wait 1.second\n"
    ));
    assert!(centre(&r.snapshots[23], 1).0 < 100.0 - 150.0);
}

#[test]
fn a_wall_stops_the_walker() {
    let r = go(&format!("shape wall = rectangle(width: 40, height: 400, at: (260, 360))\nwall.body(kind: static)\n{SET}scene \"s\" for 3.seconds:\n    main:\n        hero.move(1)\n        wait 3.seconds\n"));
    // the wall's left face is at x = 240, the hero is 40 wide: its centre stops at 220
    let x = centre(&r.snapshots[71], 2).0;
    near(x, 220.0, 3.0);
}

#[test]
fn jump_rises_to_about_v_squared_over_two_g() {
    let r = go(&format!("{SET}scene \"s\" for 2.seconds:\n    main:\n        wait 0.5.seconds\n        hero.jump()\n        wait 1.5.seconds\n"));
    let top = (0..48)
        .map(|i| centre(&r.snapshots[i], 1).1)
        .fold(f64::MAX, f64::min);
    // v = 400 px/s, g = 980 px/s^2: 81.6 px above the standing height of 520
    near(520.0 - top, 81.6, 8.0);
    near(centre(&r.snapshots[47], 1).1, 520.0, 2.0); // and lands again
}

#[test]
fn grounded_is_true_on_the_ground_and_false_in_the_air() {
    let r = go(&format!("{SET}shape probe = square(size: 4, at: (0, 0))\naction watch():\n    while true:\n        if hero.grounded:\n            probe.move_to((10, 0))\n        else:\n            probe.move_to((20, 0))\n        wait 1.frame\n\nscene \"s\" for 2.seconds:\n    main:\n        start watch()\n        wait 0.5.seconds\n        hero.jump()\n        wait 1.5.seconds\n"));
    let probe = |i: usize| centre(&r.snapshots[i], 2).0;
    near(probe(6), 10.0, 1.0); // standing
    let mid_air = (12..30).any(|i| (probe(i) - 20.0).abs() < 1.0);
    assert!(mid_air, "false while airborne");
    near(probe(46), 10.0, 1.0); // landed
}

#[test]
fn jumping_in_the_air_does_nothing() {
    let r = go(&format!("{SET}scene \"s\" for 2.seconds:\n    main:\n        wait 0.5.seconds\n        hero.jump()\n        wait 0.3.seconds\n        hero.jump()\n        wait 1.2.seconds\n"));
    let top = (0..48)
        .map(|i| centre(&r.snapshots[i], 1).1)
        .fold(f64::MAX, f64::min);
    assert!(520.0 - top < 95.0, "no double jump: {}", 520.0 - top);
}

#[test]
fn a_controller_does_not_tip_over() {
    let r = go(&format!("{SET}scene \"s\" for 2.seconds:\n    main:\n        hero.move(1)\n        wait 2.seconds\n"));
    let v: Vec<(f64, f64)> = r.snapshots[47]["shapes"][1]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect();
    let spread = |f: fn(&(f64, f64)) -> f64| {
        v.iter().map(f).fold(f64::MIN, f64::max) - v.iter().map(f).fold(f64::MAX, f64::min)
    };
    near(spread(|p| p.0), 40.0, 0.01); // still upright: a 40 x 80 box
    near(spread(|p| p.1), 80.0, 0.01);
}

#[test]
fn controller_errors() {
    let e = run_src("shape a = square(size: 10, at: (0, 0))\na.controller()\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").err().expect("no body");
    assert_eq!(e.code, "E-PHY-002");
    let e = run_src("shape a = square(size: 10, at: (0, 0))\na.body(kind: static)\na.controller()\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").err().expect("static");
    assert_eq!(e.code, "E-PHY-005");
    let e = run_src("shape a = square(size: 10, at: (0, 0))\na.body()\nscene \"s\" for 1.frame:\n    main:\n        a.move(1)\n        wait 1.frame\n").err().expect("no controller");
    assert_eq!(e.code, "E-PHY-005");
    let e = run_src("shape a = square(size: 10, at: (0, 0))\na.body()\na.controller(speed: -1)\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").err().expect("bad speed");
    assert_eq!(e.code, "E-PHY-005");
    let e = run_src("shape a = square(size: 10, at: (0, 0))\na.body()\nscene \"s\" for 1.frame:\n    main:\n        a.controller()\n        wait 1.frame\n").err().expect("in a scene");
    assert!(
        e.code == "E-PHY-005" || e.code.starts_with("E-"),
        "{}",
        e.code
    );
}
