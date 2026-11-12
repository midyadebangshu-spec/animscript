//! Particles in the language: `emitter` blocks, start/stop/burst, gradients, shape sources, collision.
use animscript_lang::*;
use serde_json::Value;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n    seed: 5\n\n";

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
fn items(snap: &Value, i: usize) -> Vec<Value> {
    snap["particles"][i]["items"].as_array().unwrap().clone()
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
const MAIN: &str =
    "scene \"s\" for 2.seconds:\n    main:\n        sparks.start()\n        wait 2.seconds\n";

#[test]
fn an_emitter_fills_the_snapshot_and_programs_without_one_have_no_section() {
    let r = go(&format!("emitter sparks:\n    at: (400, 300)\n    rate: 24\n    lifetime: (10.0, 10.0)\n    speed: (50, 100)\n\n{MAIN}"));
    assert!(items(&r.snapshots[0], 0).is_empty());
    let n = items(&r.snapshots[24], 0).len();
    assert!((23..=25).contains(&n), "{n}"); // 24 per second for one second
    assert_eq!(r.snapshots[24]["particles"][0]["name"], "sparks");
    let plain = go(
        "shape s = square(size: 10)\nscene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n",
    );
    assert!(plain.snapshots[0].get("particles").is_none());
}

#[test]
fn the_same_program_and_seed_repeat_and_another_seed_differs() {
    let prog = "emitter sparks:\n    at: (400, 300)\n    rate: 40\n    lifetime: (0.5, 1.5)\n    speed: (50, 200)\n    angle: (0.turns, 1.turn)\n\n";
    let a = go(&format!("{prog}{MAIN}"));
    let b = go(&format!("{prog}{MAIN}"));
    assert_eq!(a.hashes, b.hashes);
    let other = format!("{PROJECT}{prog}{MAIN}").replace("seed: 5", "seed: 6");
    let c = run(&other, &RunOptions::default()).unwrap();
    assert_ne!(a.hashes[30], c.hashes[30]);
}

#[test]
fn a_burst_and_stop() {
    let r = go("emitter pop:\n    at: (100, 100)\n    lifetime: (0.25, 0.25)\n    speed: (10, 20)\n\nscene \"s\" for 1.second:\n    main:\n        pop.burst(30)\n        wait 1.second\n");
    assert_eq!(items(&r.snapshots[0], 0).len(), 30);
    assert_eq!(items(&r.snapshots[3], 0).len(), 30);
    assert!(
        items(&r.snapshots[8], 0).is_empty(),
        "all gone after 0.25 s"
    );
}

#[test]
fn colour_size_and_opacity_run_over_the_life() {
    let r = go("emitter e:\n    at: (50, 50)\n    lifetime: (1.0, 1.0)\n    size: (10, 10)\n    size_end: 0\n    color: #ff0000\n    color_end: #0000ff\n    opacity: 1\n    opacity_end: 0\n\nscene \"s\" for 2.seconds:\n    main:\n        e.burst(1)\n        wait 2.seconds\n");
    let first = &items(&r.snapshots[0], 0)[0];
    near(first[2].as_f64().unwrap(), 10.0, 1e-9);
    assert_eq!(first[3], "#ff0000"); // opaque colours have no alpha digits
    let mid = &items(&r.snapshots[12], 0)[0];
    near(mid[2].as_f64().unwrap(), 5.0, 0.1);
    assert_ne!(mid[3], first[3]);
    assert_eq!(
        mid[3].as_str().unwrap().len(),
        9,
        "half-way it is translucent: {}",
        mid[3]
    );
}

#[test]
fn an_emitter_can_follow_a_shape() {
    let r = go("shape ball = circle(8, radius: 10, at: (100, 100))\nemitter trail:\n    at: ball\n    rate: 24\n    lifetime: (5.0, 5.0)\n\nscene \"s\" for 2.seconds:\n    main:\n        trail.start()\n        ball.move_by(300, 0) over 2.seconds ease linear\n");
    let late = items(&r.snapshots[46], 0);
    let newest = late.last().unwrap()[0].as_f64().unwrap();
    assert!(newest > 350.0, "{newest}"); // emitted near the ball, which has moved far to the right
    let oldest = late[0][0].as_f64().unwrap();
    assert!(oldest < 150.0, "{oldest}");
}

#[test]
fn particles_collide_with_bodies_when_asked() {
    let r = go("shape ground = rectangle(width: 800, height: 40, at: (400, 580))\nground.body(kind: static)\nemitter rain:\n    at: (400, 100)\n    rate: 30\n    lifetime: (3.0, 3.0)\n    gravity: (0, 400)\n    speed: (0, 100)\n    size: (3, 3)\n    collide: true\n    restitution: 0.3\n\nscene \"s\" for 3.seconds:\n    main:\n        rain.start()\n        wait 3.seconds\n");
    for s in &r.snapshots {
        for p in items(s, 0) {
            assert!(
                p[1].as_f64().unwrap() < 560.5,
                "a particle sank into the ground: {p}"
            );
        }
    }
}

#[test]
fn bad_emitters_are_rejected() {
    assert_eq!(
        err(
            "emitter e:\n    rate: -3\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n"
        )
        .code,
        "E-PTC-001"
    );
    assert_eq!(err("emitter e:\n    lifetime: (0, 0)\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PTC-001");
    assert_eq!(err("emitter e:\n    max: 99999\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PTC-001");
    assert_eq!(err("emitter e:\n    colour: #ff0000\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n").code, "E-PTC-001");
    assert_eq!(
        err("emitter e:\n    at: 5\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n")
            .code,
        "E-PTC-002"
    );
}

#[test]
fn a_new_scene_starts_with_no_particles() {
    let r = go("emitter e:\n    at: (10, 10)\n    rate: 24\n    lifetime: (5.0, 5.0)\n\nscene \"a\" for 1.second:\n    main:\n        e.start()\n        wait 1.second\nscene \"b\" for 1.second:\n    main:\n        wait 1.second\n");
    assert!(!items(&r.snapshots[23], 0).is_empty());
    assert!(items(&r.snapshots[24], 0).is_empty());
}
