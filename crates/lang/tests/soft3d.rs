//! 3D soft bodies: balloons, cloth, solid jelly, pins, cuts, errors.
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
fn lowest(snap: &Value, shape: usize) -> f64 {
    verts(snap, shape)
        .iter()
        .map(|v| v[1])
        .fold(f64::MIN, f64::max)
}
fn highest(snap: &Value, shape: usize) -> f64 {
    verts(snap, shape)
        .iter()
        .map(|v| v[1])
        .fold(f64::MAX, f64::min)
}

const FLOOR: &str = "shape floor = cube(width: 2600, height: 60, depth: 1800, at: (640, 630, 300))\nfloor.body(kind: static)\n";

#[test]
fn a_balloon_dropped_on_the_floor_keeps_its_shape_and_rests() {
    let src = format!(
        "{FLOOR}shape ball = sphere(radius: 80, segments: 16, rings: 10, at: (640, 250, 120))\nball.soft(pressure: 1, restitution: 0.3)\n{}",
        scene(4, "")
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    // the floor top is y = 600
    assert!(
        lowest(last, 1) < 606.0,
        "sank into the floor: {}",
        lowest(last, 1)
    );
    assert!(
        lowest(last, 1) > 585.0,
        "still falling: {}",
        lowest(last, 1)
    );
    let height = lowest(last, 1) - highest(last, 1);
    assert!(height > 110.0 && height < 170.0, "height {height}");
    assert!(last["soft3"]["bodies"].as_array().unwrap().len() == 1);
}

#[test]
fn a_pinned_sheet_hangs_from_its_pins() {
    let src = "shape cloth = plane(width: 300, depth: 300, nx: 12, nz: 12, at: (640, 200, 0))\ncloth.soft(stiffness: 0.9, bend: 0.1)\ncloth.pin_where(p -> p.z < -140)\n".to_string()
        + &scene(3, "");
    let r = go(&src);
    let first = &r.snapshots[0];
    let last = r.snapshots.last().unwrap();
    // pinned edge (z = -150) is where it was; the free edge has swung down below it
    let pinned: Vec<[f64; 3]> = verts(last, 0)
        .into_iter()
        .filter(|v| v[2] < -140.0)
        .collect();
    assert!(!pinned.is_empty());
    assert!(
        pinned.iter().all(|v| (v[1] - 200.0).abs() < 1.0),
        "pins moved"
    );
    let free_y = verts(last, 0).iter().map(|v| v[1]).fold(f64::MIN, f64::max);
    assert!(free_y > 300.0, "the sheet did not hang: {free_y}");
    // at tick 0 it is flat
    assert!((highest(first, 0) - lowest(first, 0)).abs() < 1.0);
}

#[test]
fn solid_jelly_rests_on_the_floor() {
    let src = format!(
        "{FLOOR}shape jelly = cube(size: 120, at: (640, 300, 120))\njelly.soft(solid: true, stiffness: 0.8, restitution: 0.2)\n{}",
        scene(4, "")
    );
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    assert!(lowest(last, 1) < 608.0, "sank: {}", lowest(last, 1));
    let height = lowest(last, 1) - highest(last, 1);
    assert!(height > 70.0, "collapsed to {height}");
}

#[test]
fn a_cut_lets_the_far_half_of_a_sheet_fall_away() {
    let build = |cut: bool| {
        let body = if cut {
            "        let n = cloth.cut(from: (400, 200, 0), to: (880, 200, 0), width: 12)\n"
        } else {
            ""
        };
        let src = "shape cloth = plane(width: 300, depth: 300, nx: 12, nz: 12, at: (640, 200, 0))\ncloth.soft(stiffness: 0.9, bend: 0.1)\ncloth.pin_where(p -> p.z < -140)\n".to_string()
            + &scene(3, body);
        go(&src)
    };
    let whole = build(false);
    let cut = build(true);
    let far = |r: &RunResult| {
        let s = r.snapshots.last().unwrap();
        verts(s, 0)
            .iter()
            .filter(|v| v[2] > 100.0)
            .map(|v| v[1])
            .fold(f64::MIN, f64::max)
    };
    // the cut flap falls with nothing to hold it; uncut it only hangs at the end of the sheet
    assert!(
        far(&cut) > far(&whole) + 150.0,
        "cut {} whole {}",
        far(&cut),
        far(&whole)
    );
    // the removed faces are gone from the picture
    let faces_whole = whole.snapshots.last().unwrap()["shapes"][0]["mesh"]["faces"]
        .as_array()
        .unwrap()
        .len();
    let faces_cut = cut.snapshots.last().unwrap()["shapes"][0]["mesh"]["faces"]
        .as_array()
        .unwrap()
        .len();
    assert!(faces_cut < faces_whole);
}

#[test]
fn pins_can_follow_a_moving_mesh() {
    let src = "shape bar = cube(width: 60, height: 20, depth: 300, at: (500, 100, 0))\nbar.body(kind: kinematic)\nshape cloth = plane(width: 300, depth: 300, nx: 10, nz: 10, at: (500, 205, 0))\ncloth.soft(stiffness: 0.9)\ncloth.pin_where(p -> p.x < 360, to: bar)\n".to_string()
        + &scene(2, "        bar.move_to((200, 100), over: 2.seconds)\n");
    let r = go(&src);
    let last = r.snapshots.last().unwrap();
    // the pinned edge started at x = 350 and the bar moved 300 px: the pins went with it
    // the cloth started between x = 350 and 650; its pinned edge was pulled 300 px left and dragged the rest
    let far_x = verts(last, 1).iter().map(|v| v[0]).fold(f64::MIN, f64::max);
    assert!(far_x < 560.0, "the cloth was left behind at x = {far_x}");
}

#[test]
fn volume_and_stretch_can_be_read() {
    let src = format!(
        "{FLOOR}shape ball = sphere(radius: 60, at: (640, 400, 0))\nball.soft()\nshape probe = square(size: 10, at: (0, 0))\nscene \"s\" for 1.second:\n    main:\n        probe.move_to((ball.volume / 10000, ball.stretch * 100))\n        wait 1.second\n"
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let n = s["shapes"].as_array().unwrap().len();
    let pos = &s["shapes"][n - 1]["transform"]["position"];
    // 4/3 pi 60^3 = 904778 px^3 -> 90
    assert!(
        pos[0].as_f64().unwrap() > 70.0 && pos[0].as_f64().unwrap() < 110.0,
        "{pos}"
    );
}

#[test]
fn identical_programs_give_identical_snapshots() {
    let src = format!(
        "{FLOOR}shape ball = sphere(radius: 60, at: (600, 250, 0))\nball.soft(pressure: 1.1)\nshape cloth = plane(width: 300, depth: 300, nx: 8, nz: 8, at: (800, 150, 0))\ncloth.soft()\ncloth.pin_where(p -> p.z < -140)\n{}",
        scene(2, "")
    );
    let (a, b) = (go(&src), go(&src));
    assert_eq!(a.snapshots.last(), b.snapshots.last());
}

#[test]
fn errors() {
    let tail = "scene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n";
    // rigid and soft
    let d = err("shape a = cube(size: 50, at: (300, 300, 0))\na.body()\na.soft()\n");
    assert_eq!(d.code, "E-PHY-003");
    // an open mesh cannot be solid or pressurised
    let d = err("shape p = plane(width: 100, depth: 100, nx: 3, nz: 3, at: (300, 300, 0))\np.soft(solid: true)\n");
    assert_eq!(d.code, "E-PHY-009");
    let d = err("shape p = plane(width: 100, depth: 100, nx: 3, nz: 3, at: (300, 300, 0))\np.soft(pressure: 2)\n");
    assert_eq!(d.code, "E-PHY-009");
    // stiffness range
    let d = err("shape a = cube(size: 50, at: (300, 300, 0))\na.soft(stiffness: 0)\n");
    assert_eq!(d.code, "E-PHY-009");
    // pinning something that is not soft
    let d = err(&format!(
        "shape a = cube(size: 50, at: (300, 300, 0))\na.pin_where(p -> p.x < 0)\n{tail}"
    ));
    assert_eq!(d.code, "E-PHY-009");
    // cutting a solid body
    let d = err("shape a = cube(size: 50, at: (300, 300, 0))\na.soft(solid: true)\nscene \"s\" for 2.frames:\n    main:\n        a.cut(from: (0, 0, 0), to: (1, 1, 1))\n        wait 2.frames\n");
    assert_eq!(d.code, "E-PHY-009");
    // soft bodies are made before the scene runs
    let d = err("shape a = cube(size: 50, at: (300, 300, 0))\nscene \"s\" for 2.frames:\n    main:\n        a.soft()\n        wait 2.frames\n");
    assert_eq!(d.code, "E-SHP-003");
}
