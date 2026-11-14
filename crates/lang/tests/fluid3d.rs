//! 3D fluids: fills, pours, levels, floating bodies, errors and determinism.
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
fn fl(snap: &Value) -> &Value {
    &snap["fluids3"][0]
}

/// A tank open at the top: a floor and four walls (static cubes), the inside 600 x 400 x 400 px.
const TANK: &str = "shape floor = cube(width: 700, height: 40, depth: 500, at: (640, 620, 200))\nfloor.body(kind: static, friction: 0.1)\nshape wall_l = cube(width: 40, height: 440, depth: 500, at: (320, 400, 200))\nwall_l.body(kind: static, friction: 0.1)\nshape wall_r = cube(width: 40, height: 440, depth: 500, at: (960, 400, 200))\nwall_r.body(kind: static, friction: 0.1)\nshape wall_b = cube(width: 700, height: 440, depth: 40, at: (640, 400, 0))\nwall_b.body(kind: static, friction: 0.1)\nshape wall_f = cube(width: 700, height: 440, depth: 40, at: (640, 400, 400))\nwall_f.body(kind: static, friction: 0.1)\n";

#[test]
fn a_box_of_water_is_filled_and_counted() {
    let src = format!(
        "{TANK}fluid water:\n    fill: box(350, 400, 20, 280, 180, 360)\n    spacing: 20\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 1.second:\n    main:\n        probe.move_to((water.count, 0))\n        wait 1.second\n"
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    // 14 x 9 x 18 particles
    assert_eq!(fl(last)["count"].as_u64().unwrap(), 14 * 9 * 18);
    assert!(fl(last)["particles"].as_str().unwrap().len() > 1000);
    let n = last["shapes"].as_array().unwrap().len();
    let x = last["shapes"][n - 1]["transform"]["position"][0]
        .as_f64()
        .unwrap();
    assert!((x - 2268.0).abs() < 1.0, "count read as {x}");
}

#[test]
fn the_water_stays_in_the_tank_and_settles() {
    let src = format!(
        "{TANK}fluid water:\n    fill: box(350, 340, 20, 280, 240, 360)\n    spacing: 24\n{}",
        scene(2, "")
    );
    let r = go(&src);
    let (first, last) = (&r.snapshots[0], r.snapshots.last().unwrap());
    assert_eq!(fl(first)["count"], fl(last)["count"], "particles were lost");
    assert!(fl(last)["render"] == "surface");
}

#[test]
fn a_pour_adds_water() {
    let src = format!(
        "{TANK}fluid water:\n    spacing: 20\n    space: \"3d\"\n{}",
        scene(
            3,
            "        water.pour(at: (640, 100, 200), speed: (0, 240, 0), width: 60)\n"
        )
    );
    let r = go(&src);
    let counts: Vec<u64> = r
        .snapshots
        .iter()
        .map(|s| s["fluids3"][0]["count"].as_u64().unwrap())
        .collect();
    assert_eq!(counts[0], 0);
    assert!(
        counts.last().unwrap() > &150,
        "count {}",
        counts.last().unwrap()
    );
    assert!(
        counts.windows(2).all(|w| w[1] >= w[0]),
        "particles disappeared"
    );
}

#[test]
fn level_reads_the_surface_of_a_column() {
    let src = format!(
        "{TANK}fluid water:\n    fill: box(350, 440, 20, 280, 140, 360)\n    spacing: 20\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 1.second:\n    main:\n        probe.move_to((0, water.level(400, 200)))\n        wait 1.second\n"
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    let n = last["shapes"].as_array().unwrap().len();
    let y = last["shapes"][n - 1]["transform"]["position"][1]
        .as_f64()
        .unwrap();
    // the top row of the fill is at y = 450, a little lower after a second of settling
    assert!(y > 440.0 && y < 540.0, "level {y}");
}

#[test]
fn a_light_crate_floats_and_a_heavy_one_sinks() {
    let src = format!(
        "{TANK}fluid water:\n    fill: box(350, 440, 20, 280, 140, 360)\n    spacing: 20\nshape light = cube(size: 80, at: (500, 300, 200))\nlight.body(density: 0.3)\nshape heavy = cube(size: 80, at: (780, 300, 200))\nheavy.body(density: 3)\n{}",
        scene(5, "")
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    let ys = |idx: usize| -> f64 {
        let v = last["shapes"][idx]["mesh"]["verts"].as_array().unwrap();
        v.iter().map(|p| p[1].as_f64().unwrap()).sum::<f64>() / v.len() as f64
    };
    // the water spreads over the 600 px floor to a depth of about 75 px (surface near y = 520); a crate
    // resting on the floor has its centre near y = 540 (a layer of particles stays under it), a floating
    // one is held at the surface
    // shapes: 5 tank parts, then light, heavy
    let (light, heavy) = (ys(5), ys(6));
    assert!(light < 530.0, "light crate at {light}");
    assert!(
        heavy > 532.0 && heavy > light + 12.0,
        "heavy crate at {heavy}"
    );
}

#[test]
fn the_thread_setting_does_not_change_the_picture() {
    let src = |t: u32| {
        format!(
            "world:\n    fluid_threads: {t}\n\n{TANK}fluid water:\n    fill: box(350, 400, 20, 200, 180, 360)\n    spacing: 24\n{}",
            scene(1, "")
        )
    };
    let (a, b) = (go(&src(1)), go(&src(4)));
    assert_eq!(
        fl(a.snapshots.last().unwrap()),
        fl(b.snapshots.last().unwrap())
    );
}

#[test]
fn errors() {
    let tail = scene(1, "");
    // a 3D fluid fills boxes and meshes
    let d = err(&format!(
        "{TANK}fluid water:\n    fill: rect(0, 0, 10, 10)\n    space: \"3d\"\n{tail}"
    ));
    assert_eq!(d.code, "E-FLD-004");
    // a zero speed
    let d = err(&format!(
        "{TANK}fluid water:\n    space: \"3d\"\n{}",
        scene(1, "        water.pour(at: (1, 1, 1), speed: (0, 0, 0))\n")
    ));
    assert_eq!(d.code, "E-FLD-004");
    // a bad space
    let d = err(&format!("fluid water:\n    space: \"4d\"\n{tail}"));
    assert_eq!(d.code, "E-FLD-001");
    // too many particles for the fill
    let d = err(&format!("{TANK}fluid water:\n    fill: box(0, 0, 0, 1000, 1000, 1000)\n    spacing: 20\n    max: 100\n{tail}"));
    assert_eq!(d.code, "E-FLD-004");
}
