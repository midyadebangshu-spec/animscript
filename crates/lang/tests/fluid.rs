//! Fluids in the language: `fluid` blocks, fill, pour/stop/add, count and level, bodies, errors.
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
fn particles(snap: &Value, i: usize) -> Vec<(f64, f64)> {
    snap["fluids"][i]["particles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap()))
        .collect()
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
const FRAME: &str = "scene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n";
// a container: floor and two walls, 300 px wide inside, floor top at y = 560
const TANK: &str = "shape floor = rectangle(width: 360, height: 20, at: (400, 570))\nshape wl = rectangle(width: 20, height: 300, at: (240, 420))\nshape wr = rectangle(width: 20, height: 300, at: (560, 420))\nfloor.body(kind: static)\nwl.body(kind: static)\nwr.body(kind: static)\n";

#[test]
fn a_fluid_fills_its_region_with_a_lattice_and_programs_without_one_have_no_section() {
    let r = go(&format!(
        "fluid water:\n    fill: rect(100, 300, 100, 50)\n    spacing: 10\n\n{FRAME}"
    ));
    assert_eq!(particles(&r.snapshots[0], 0).len(), 50);
    assert_eq!(r.snapshots[0]["fluids"][0]["name"], "water");
    let plain = go(
        "shape s = square(size: 10)\nscene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n",
    );
    assert!(plain.snapshots[0].get("fluids").is_none());
}

#[test]
fn water_settles_in_a_tank_and_stays_inside_it() {
    let r = go(&format!("{TANK}fluid water:\n    fill: rect(290, 400, 220, 150)\n    spacing: 10\n\nscene \"s\" for 4.seconds:\n    main:\n        wait 4.seconds\n"));
    let last = particles(r.snapshots.last().unwrap(), 0);
    assert_eq!(last.len(), 22 * 15);
    for p in &last {
        assert!(
            p.0 > 250.0 - 1.0 && p.0 < 550.0 + 1.0 && p.1 < 560.0 + 1.0,
            "escaped: {p:?}"
        );
    }
    let top = last.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    assert!(
        // 22 x 15 particles spread over the 300 px wide tank: a pool about 110 px deep
        top > 435.0 && top < 465.0,
        "the surface: {top}"
    );
}

#[test]
fn the_same_program_gives_the_same_hashes_and_a_new_scene_refills() {
    let s = "fluid water:\n    fill: rect(100, 300, 60, 40)\n    spacing: 10\n\nscene \"a\" for 12.frames:\n    main:\n        wait 12.frames\nscene \"b\" for 12.frames:\n    main:\n        wait 12.frames\n";
    let (a, b) = (go(s), go(s));
    assert_eq!(a.hashes, b.hashes);
    assert_eq!(particles(&a.snapshots[12], 0).len(), 24); // scene b starts from the region again
    assert_eq!(
        particles(&a.snapshots[12], 0)[0],
        particles(&a.snapshots[0], 0)[0]
    );
    assert_ne!(
        particles(&a.snapshots[11], 0)[0],
        particles(&a.snapshots[0], 0)[0]
    );
}

#[test]
fn pouring_adds_particles_at_the_stream_rate_until_stopped() {
    let r = go("fluid water:\n    spacing: 10\n    max: 500\n\nscene \"s\" for 3.seconds:\n    main:\n        water.pour(at: (300, 50), speed: (0, 200), width: 20)\n        wait 1.second\n        water.stop()\n        wait 2.seconds\n");
    // 200 px/s at 10 px spacing and 20 px wide is 40 particles per second
    let (n1, n2, n3) = (
        particles(&r.snapshots[23], 0).len(),
        particles(&r.snapshots[24], 0).len(),
        particles(&r.snapshots[70], 0).len(),
    );
    assert!((36..=44).contains(&n1), "{n1}");
    assert!(n2 >= n1 && n3 <= n2 + 2, "{n1} {n2} {n3}"); // nothing new after stop() (a particle may fall out of the world)
}

#[test]
fn count_and_level_can_be_read() {
    let r = go(&format!("{TANK}shape probe = square(size: 10, at: (0, 0))\nfluid water:\n    fill: rect(250, 460, 300, 100)\n    spacing: 10\n\nscene \"s\" for 3.seconds:\n    main:\n        wait 2.seconds\n        probe.move_to((water.count, water.level(400)))\n        wait 1.second\n"));
    let c = {
        let v = r.snapshots.last().unwrap()["shapes"][3]["vertices"]
            .as_array()
            .unwrap()
            .clone();
        (
            v.iter().map(|p| p[1].as_f64().unwrap()).sum::<f64>() / 4.0,
            v.iter().map(|p| p[2].as_f64().unwrap()).sum::<f64>() / 4.0,
        )
    };
    near(c.0, 300.0, 0.01); // 30 x 10 particles
    near(c.1, 465.0, 20.0); // the surface of a 100 px deep pool
}

#[test]
fn a_light_ball_floats_on_water_and_a_heavy_one_sinks() {
    let run_with = |density: f64| {
        let r = go(&format!("{TANK}fluid water:\n    fill: rect(290, 440, 220, 120)\n    spacing: 10\n\nshape ball = circle(16, radius: 25, at: (400, 300))\nball.body(density: {density})\n\nscene \"s\" for 5.seconds:\n    main:\n        wait 5.seconds\n"));
        let v: Vec<f64> = r.snapshots.last().unwrap()["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "ball" || true)
            .map(|_| vec![])
            .unwrap();
        let _ = v;
        let shapes = r.snapshots.last().unwrap()["shapes"].as_array().unwrap();
        let ball = &shapes[3]["vertices"].as_array().unwrap().clone();
        ball.iter().map(|p| p[2].as_f64().unwrap()).sum::<f64>() / ball.len() as f64
    };
    let (light, heavy) = (run_with(0.3), run_with(4.0));
    assert!(
        light < 500.0,
        "the light ball floats near the surface: {light}"
    );
    assert!(heavy > 520.0, "the heavy ball sinks to the floor: {heavy}");
}

#[test]
fn bad_fluids_are_rejected() {
    let sc = FRAME;
    assert_eq!(
        err(&format!("fluid w:\n    spacing: -4\n{sc}")).code,
        "E-FLD-001"
    );
    assert_eq!(
        err(&format!("fluid w:\n    viscosity: -1\n{sc}")).code,
        "E-FLD-001"
    );
    assert_eq!(
        err(&format!("fluid w:\n    max: 99999\n{sc}")).code,
        "E-FLD-001"
    );
    assert_eq!(
        err(&format!("fluid w:\n    wetness: 3\n{sc}")).code,
        "E-FLD-001"
    );
    assert_eq!(
        err(&format!("fluid w:\n    fill: 5\n{sc}")).code,
        "E-FLD-002"
    );
    assert_eq!(
        err(&format!(
            "fluid w:\n    fill: rect(0, 0, 10, 10)\n    spacing: 1\n    max: 10\n{sc}"
        ))
        .code,
        "E-FLD-001"
    ); // 100 particles into a 10 cap
    assert_eq!(err("fluid w:\n    spacing: 10\nscene \"s\" for 1.frame:\n    main:\n        w.pour(at: (1, 1), speed: (0, 0), width: 10)\n        wait 1.frame\n").code, "E-FLD-001");
}
