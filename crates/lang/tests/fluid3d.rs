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

/// Four walls without a floor: the wave field is the floor.
const WALLS: &str = "shape wall_l = cube(width: 40, height: 440, depth: 400, at: (320, 400, 200))\nwall_l.body(kind: static, friction: 0.1)\nshape wall_r = cube(width: 40, height: 440, depth: 400, at: (960, 400, 200))\nwall_r.body(kind: static, friction: 0.1)\nshape wall_b = cube(width: 700, height: 440, depth: 40, at: (640, 400, 50))\nwall_b.body(kind: static, friction: 0.1)\nshape wall_f = cube(width: 700, height: 440, depth: 40, at: (640, 400, 350))\nwall_f.body(kind: static, friction: 0.1)\n";

const SEA: &str = "field sea:\n    grid: (30, 14)\n    rect: (340, 70, 600, 260)\n    state: [\"h\", \"v\"]\n    edges: \"mirror\"\n    c2: 150\n    damp: 0.8\n    init: [(\"h\", \"0\"), (\"v\", \"0\")]\n    step: [[(\"v\", \"v + dt * (c2 * (h[-1, 0] + h[1, 0] + h[0, -1] + h[0, 1] - 4 * h) - damp * v)\")], [(\"h\", \"h + dt * v\")]]\n";

#[test]
fn water_rests_on_a_wave_field_and_makes_ripples_in_it() {
    let src = format!(
        "{WALLS}{SEA}fluid water:\n    fill: box(350, 380, 80, 580, 100, 240)\n    spacing: 24\n    field: sea\n    level: 520\n    absorb: false\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 4.seconds:\n    main:\n        wait 0.5.seconds\n        let low = sea.min()\n        wait 0.5.seconds\n        probe.move_to((sea.min() * 10, sea.max() * 10))\n        wait 3.seconds\n"
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    let n = last["shapes"].as_array().unwrap().len();
    // the water fell 40 px onto the surface and stayed on it
    assert!(fl(last)["count"].as_u64().unwrap() >= 960);
    let pos = &last["shapes"][n - 1]["transform"]["position"];
    // the impact pressed the field down somewhere (min below zero)
    assert!(pos[0].as_f64().unwrap() < -5.0, "no ripple: {pos}");
}

fn mean_y(snap: &Value) -> f64 {
    let a = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let v: Vec<u32> = fl(snap)["particles"]
        .as_str()
        .unwrap()
        .bytes()
        .filter(|c| *c != b'=')
        .map(|c| a.iter().position(|x| *x == c).unwrap() as u32)
        .collect();
    let mut bytes = vec![];
    for c in v.chunks(4) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, x)| n | x << (18 - 6 * i));
        bytes.push((n >> 16) as u8);
        if c.len() > 2 {
            bytes.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            bytes.push(n as u8);
        }
    }
    let f: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let n = f.len() / 3;
    (0..n).map(|k| f[k * 3 + 1] as f64).sum::<f64>() / n as f64
}

#[test]
fn a_hump_in_the_field_lifts_the_water_above_it() {
    let src = |poke: &str| {
        format!(
            "{WALLS}{SEA}fluid water:\n    fill: box(350, 400, 80, 580, 100, 240)\n    spacing: 24\n    field: sea\n    level: 520\n    ripple: 0\n    absorb: false\n{}",
            scene(3, &format!("        wait 2.seconds\n{poke}"))
        )
    };
    // a quarter of a second after the poke (tick 54)
    let flat = mean_y(&go(&src("")).snapshots[54]);
    let hump = mean_y(
        &go(&src(
            "        sea.poke(640, 200, amount: 70, radius: 150)\n        wait 0.5.seconds\n",
        ))
        .snapshots[54],
    );
    assert!(
        hump < flat - 8.0,
        "the water did not rise over the hump: flat {flat}, hump {hump}"
    );
}

#[test]
fn a_fluid_on_a_field_needs_a_level() {
    let d = err(&format!(
        "{SEA}fluid water:\n    field: sea\n{}",
        scene(1, "")
    ));
    assert_eq!(d.code, "E-FLD-001");
}

#[test]
fn a_jet_of_water_pushes_a_free_cloth() {
    // no gravity, so only the water can move the cloth
    let src = |jet: &str| {
        format!(
            "world:\n    gravity: (0, 0, 0)\n\nshape cloth = plane(width: 300, depth: 200, nx: 12, nz: 8, at: (640, 300, 200))\ncloth.soft(stiffness: 0.8, density: 0.2)\nfluid water:\n    spacing: 20\n    space: \"3d\"\n{}",
            scene(2, jet)
        )
    };
    let mean_y = |r: &RunResult| {
        let v = r.snapshots.last().unwrap()["shapes"][0]["mesh"]["verts"]
            .as_array()
            .unwrap()
            .clone();
        v.iter().map(|p| p[1].as_f64().unwrap()).sum::<f64>() / v.len() as f64
    };
    let calm = mean_y(&go(&src("")));
    let hit = mean_y(&go(&src(
        "        water.pour(at: (640, 40, 200), speed: (0, 300, 0), width: 100)\n",
    )));
    assert!(
        (calm - 300.0).abs() < 0.5,
        "the cloth moved by itself: {calm}"
    );
    assert!(
        hit > calm + 20.0,
        "the jet did not push the cloth: calm {calm}, hit {hit}"
    );
}

#[test]
fn water_that_comes_to_rest_on_a_field_joins_it() {
    let src = |absorb: &str| {
        format!(
            "{WALLS}{SEA}fluid water:\n    fill: box(350, 440, 80, 580, 60, 240)\n    spacing: 24\n    field: sea\n    level: 520\n{absorb}shape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 4.seconds:\n    main:\n        wait 3.5.seconds\n        probe.move_to((sea.sum(), 0))\n        wait 0.5.seconds\n"
        )
    };
    let kept = go(&src("    absorb: false\n"));
    let joined = go(&src(""));
    let count = |r: &RunResult| {
        r.snapshots.last().unwrap()["fluids3"][0]["count"]
            .as_u64()
            .unwrap()
    };
    let total = |r: &RunResult| {
        let s = r.snapshots.last().unwrap();
        let n = s["shapes"].as_array().unwrap().len();
        s["shapes"][n - 1]["transform"]["position"][0]
            .as_f64()
            .unwrap()
    };
    assert_eq!(count(&kept), 480);
    assert!(
        count(&joined) < 200,
        "the water did not settle into the field: {} left",
        count(&joined)
    );
    // the field has gained (a little of) the water's volume
    assert!(
        total(&joined) > total(&kept) + 20.0,
        "{} vs {}",
        total(&joined),
        total(&kept)
    );
}
