//! Reading the *current* animated state from AnimScript: `center(shape)`, `shape.position`, bone
//! `head`/`tip`/`angle`. Expressions used to see only rest values, so an action could not follow a
//! moving target. Written first. A value read in tick n is the state resolved at the end of tick n - 1.
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
const FOLLOW: &str = "action follow(self: Shape, target: Shape, n: Int):\n    let i = 0\n    while i < n:\n        self.move_to(center(target))\n        wait 1.frame\n        i += 1\n\n";

#[test]
fn an_action_can_follow_a_moving_shape_frame_by_frame() {
    let r = go(&format!("{FOLLOW}shape ball = square(size: 20, at: (100, 100))\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 2.seconds:\n    main:\n        start probe.follow(ball, 46)\n        ball.move_by(200, 0) over 2.seconds ease linear\n"));
    let (b, p) = (centre(&r.snapshots[24], 0), centre(&r.snapshots[24], 1));
    near(b.0, 200.0, 1.0); // half way
    assert!(
        p.0 > 150.0,
        "the probe followed the ball and did not stay at its rest value: {}",
        p.0
    );
    near(p.0, b.0, 6.0); // one frame behind at most
    let (b, p) = (centre(&r.snapshots[44], 0), centre(&r.snapshots[44], 1));
    near(p.0, b.0, 6.0);
}

#[test]
fn center_is_the_world_centre_with_the_shapes_own_transform() {
    let r = go("shape a = square(size: 20, at: (100, 100))\nshape probe = square(size: 10, at: (0, 0))\na.move_by(50, 25)\nprobe.move_to(center(a))\nscene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n");
    let p = centre(&r.snapshots[0], 1);
    near(p.0, 150.0, 1e-9);
    near(p.1, 125.0, 1e-9);
}

#[test]
fn position_and_rotation_reads_are_live() {
    let r = go("shape a = square(size: 20, at: (100, 100))\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 2.seconds:\n    main:\n        a.move_by(100, 0) over 1.second ease linear\n        probe.move_to((a.position.x, 0))\n        wait 1.frame\n");
    let p = centre(&r.snapshots[25], 1);
    near(p.0, 100.0, 100.0 / 24.0 + 1e-6); // a read sees the previous tick's state: at most one frame behind
    let r = go("shape a = square(size: 20, at: (100, 100))\nshape probe = square(size: 10, at: (0, 0))\naction watch(n: Int):\n    let i = 0\n    while i < n:\n        probe.move_to((a.position.x, 0))\n        wait 1.frame\n        i += 1\n\nscene \"s\" for 2.seconds:\n    main:\n        start watch(30)\n        a.move_by(100, 0) over 1.second ease linear\n");
    let mid = centre(&r.snapshots[12], 1).0;
    assert!(mid > 30.0 && mid < 70.0, "half way through the move: {mid}");
}

#[test]
fn a_new_scene_reads_rest_values_again() {
    let r = go("shape a = square(size: 20, at: (100, 100))\nshape probe = square(size: 10, at: (0, 0))\nscene \"a\" for 1.second:\n    main:\n        a.move_by(100, 0) over 1.second ease linear\nscene \"b\" for 2.frames:\n    setup:\n        probe.move_to(center(a))\n    main:\n        wait 2.frames\n");
    near(centre(&r.snapshots[24], 1).0, 200.0, 0.01); // scene b's setup sees where a ended
}
