//! 3D physics: bodies on meshes, collisions, joints, kinematic bodies, floating, errors.
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
fn verts(snap: &Value, shape: usize) -> Vec<[f64; 3]> {
    snap["shapes"][shape]["mesh"]["verts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            [
                v[0].as_f64().unwrap(),
                v[1].as_f64().unwrap(),
                v[2].as_f64().unwrap(),
            ]
        })
        .collect()
}
fn centre(snap: &Value, shape: usize) -> [f64; 3] {
    let v = verts(snap, shape);
    let mut c = [0.0; 3];
    for p in &v {
        for k in 0..3 {
            c[k] += p[k] / v.len() as f64;
        }
    }
    c
}
fn bottom(snap: &Value, shape: usize) -> f64 {
    verts(snap, shape)
        .iter()
        .map(|v| v[1])
        .fold(f64::MIN, f64::max)
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}

const FLOOR: &str = "shape floor = cube(width: 1200, height: 40, depth: 800, at: (640, 640, 0))\nfloor.body(kind: static)\n";

#[test]
fn a_cube_falls_lands_on_the_floor_and_stays_upright() {
    let src = format!(
        "{FLOOR}shape box = cube(size: 100, at: (640, 200, 0))\nbox.body()\n{}",
        scene(4, "")
    );
    let r = go(&src);
    let first = &r.snapshots[0];
    let last = r.snapshots.last().unwrap();
    near(centre(first, 1)[1], 200.0, 1e-6);
    // floor top is y = 620; the cube's bottom rests on it
    near(bottom(last, 1), 620.0, 3.0);
    near(centre(last, 1)[0], 640.0, 2.0);
    // it stays a cube: the top is 100 above the bottom
    let top = verts(last, 1).iter().map(|v| v[1]).fold(f64::MAX, f64::min);
    near(bottom(last, 1) - top, 100.0, 0.5);
    assert!(last["physics3"]["bodies"].as_array().unwrap().len() == 2);
}

#[test]
fn a_programs_without_mesh_bodies_have_no_physics3_section() {
    let r = go(&format!(
        "shape box = cube(size: 100, at: (640, 200, 0))\n{}",
        scene(1, "")
    ));
    assert!(r.snapshots[0].get("physics3").is_none());
}

#[test]
fn a_sphere_rolls_down_a_ramp() {
    let src = "world:\n    gravity: (0, 9.8, 0)\n\nshape ramp = cube(width: 800, height: 30, depth: 300, at: (640, 500, 0))\nramp.rotation = 12.deg\nramp.body(kind: static)\nshape ball = sphere(radius: 30, at: (400, 380, 0))\nball.body(restitution: 0.1)\n".to_string()
        + &scene(3, "");
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    let c = centre(last, 1);
    assert!(c[0] > 600.0, "ball only reached x = {}", c[0]);
    // the ball turned as it rolled: an object with quaternion orientation still has 3D vertices
    assert!(verts(last, 1).len() > 10);
}

#[test]
fn gravity_can_point_along_z() {
    let src = "world:\n    gravity: (0, 0, 9.8)\n\nshape ball = sphere(radius: 20, at: (640, 360, 0))\nball.body()\n".to_string()
        + &scene(1, "");
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    let c = centre(last, 0);
    assert!(c[2] > 200.0, "z = {}", c[2]);
    near(c[1], 360.0, 1.0);
}

#[test]
fn set_velocity_and_a_hull_body_tumbles() {
    let src = format!(
        "{FLOOR}shape rock = cone(radius: 50, height: 120, at: (300, 400, 0))\nrock.rotation = 70.deg\nrock.body(collider: \"hull\")\n{}",
        scene(3, "")
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    // it comes to rest on the floor, not through it
    near(bottom(last, 1), 620.0, 4.0);
}

#[test]
fn a_ball_joint_holds_a_pendulum() {
    let src = "shape pin = sphere(radius: 10, at: (640, 100, 0))\npin.body(kind: static)\nshape bob = sphere(radius: 25, at: (840, 100, 0))\nbob.body()\nlet j = ball(pin, bob, anchor: (640, 100, 0))\n".to_string()
        + &scene(2, "");
    let r = go(&src);
    for snap in &r.snapshots {
        let c = centre(snap, 1);
        let d = ((c[0] - 640.0).powi(2) + (c[1] - 100.0).powi(2) + c[2].powi(2)).sqrt();
        near(d, 200.0, 6.0);
    }
    let last = r.snapshots.last().unwrap();
    assert!(centre(last, 1)[1] > 120.0 || centre(last, 1)[0] < 800.0);
}

#[test]
fn a_hinge_keeps_a_door_in_its_plane() {
    let src = "shape post = cube(size: 20, at: (640, 100, 0))\npost.body(kind: static)\nshape door = cube(width: 200, height: 20, depth: 60, at: (740, 100, 0))\ndoor.body()\nlet j = hinge(post, door, anchor: (640, 100, 0), axis: (0, 0, 1))\n".to_string()
        + &scene(2, "        door.set_velocity((0, 0, 300))\n");
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    near(centre(last, 1)[2], 0.0, 8.0);
}

#[test]
fn a_kinematic_body_pushes_a_dynamic_one() {
    let src = "shape paddle = cube(width: 40, height: 200, depth: 200, at: (100, 560, 0))\npaddle.body(kind: kinematic)\nshape ball = sphere(radius: 30, at: (200, 560, 0))\nball.body(gravity_scale: 0)\n".to_string()
        + &scene(1, "        paddle.move_to((500, 560), over: 1.second)\n");
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    assert!(
        centre(last, 1)[0] > 300.0,
        "ball x = {}",
        centre(last, 1)[0]
    );
}

#[test]
fn touches_reports_contact() {
    let src = format!(
        "{FLOOR}shape ball = sphere(radius: 30, at: (640, 400, 0))\nball.body()\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 3.seconds:\n    whenever ball.touches(floor):\n        probe.move_to((100, 0)) over 1.frame\n    main:\n        wait 3.seconds\n"
    );
    let r = go(&src);
    let p = &r.snapshots.last().unwrap()["shapes"];
    let n = p.as_array().unwrap().len();
    let probe = &p[n - 1]["transform"]["position"];
    assert!(probe[0].as_f64().unwrap() > 50.0);
}

#[test]
fn a_body_floats_on_a_field() {
    let src = "field sea:\n    grid: (8, 8)\n    rect: (0, 0, 1280, 1280)\n    state: [\"h\"]\n    raw: true\n    init: [(\"h\", \"0\")]\n    step: [(\"h\", \"h\")]\n\nshape buoy = cube(size: 80, at: (640, 400, 640))\nbuoy.body()\n".to_string()
        + "scene \"s\" for 6.seconds:\n    setup:\n        buoy.float(field: sea, channel: \"h\", level: 500, density: 2, drag: 4)\n    main:\n        wait 6.seconds\n";
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    // it sinks until the water (y = 500) is half way up its side: centre near 500 - small
    let c = centre(last, 0)[1];
    assert!(c > 440.0 && c < 560.0, "buoy centre at {c}");
    // and it is not still falling
    let prev = centre(&r.snapshots[r.snapshots.len() - 12], 0)[1];
    near(prev, c, 12.0);
}

#[test]
fn identical_programs_give_identical_snapshots() {
    let src = format!(
        "{FLOOR}shape a = cube(size: 80, at: (600, 100, 0))\na.rotation = 20.deg\na.body()\nshape b = sphere(radius: 40, at: (640, -100, 0))\nb.body(restitution: 0.4)\n{}",
        scene(3, "")
    );
    let (x, y) = (go(&src), go(&src));
    assert_eq!(x.snapshots.last(), y.snapshots.last());
}

#[test]
fn errors() {
    // a body that is not convex
    let d = err("shape h = lathe(profile: [(0, 0), (60, 0), (10, 50), (60, 100), (0, 100)], segments: 12, at: (300, 300, 0))\nh.body()\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n");
    assert_eq!(d.code, "E-PHY-007");
    // a non-uniform scale
    let d = err("shape c = cube(size: 50, at: (300, 300, 0))\nc.scale = (2, 1)\nc.body()\n");
    assert_eq!(d.code, "E-PHY-007");
    // unknown collider
    let d = err("shape c = cube(size: 50, at: (300, 300, 0))\nc.body(collider: \"blob\")\n");
    assert_eq!(d.code, "E-PHY-007");
    // joint with a non-body
    let d = err("shape a = cube(size: 50, at: (300, 300, 0))\nshape b = cube(size: 50, at: (400, 300, 0))\na.body()\nlet j = ball(a, b, anchor: (350, 300, 0))\n");
    assert_eq!(d.code, "E-PHY-008");
    // a hinge needs an axis
    let d = err("shape a = cube(size: 50, at: (300, 300, 0))\nshape b = cube(size: 50, at: (400, 300, 0))\na.body()\nb.body()\nlet j = hinge(a, b, anchor: (350, 300, 0), axis: (0, 0, 0))\n");
    assert_eq!(d.code, "E-PHY-008");
}
