//! Lights: declarations, animation, errors, the snapshot.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 1280x720\n    fps: 24\n\nimport \"std/3d\"\n";

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
fn light<'a>(snap: &'a Value, id: &str) -> &'a Value {
    snap["lights"]
        .as_array()
        .expect("lights")
        .iter()
        .find(|l| l["id"] == id)
        .unwrap_or_else(|| panic!("no light {id}"))
}
fn v3(v: &Value) -> [f64; 3] {
    [
        v[0].as_f64().unwrap(),
        v[1].as_f64().unwrap(),
        v[2].as_f64().unwrap(),
    ]
}

#[test]
fn a_scene_without_lights_has_no_lights_in_its_snapshot() {
    let r = go(&scene(1, ""));
    assert!(r.snapshots.last().unwrap().get("lights").is_none());
}

#[test]
fn each_kind_of_light_reaches_the_snapshot() {
    let src = format!(
        "shape sun = sun_light(dir: (0, 2, 0), intensity: 3, color: #ffffff)
shape lamp = point_light(at: (400, 200, 150), color: #ff8000, intensity: 2, reach: 300, size: 10)
shape spot = spot_light(at: (900, 80, 300), dir: (0, 1, 0), angle: 30.deg, softness: 0.4, intensity: 5)
shape sky = ambient_light(color: #8fb0d0, intensity: 0.3, ground: #3a2a20)
{}",
        scene(1, "")
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    assert_eq!(s["lights"].as_array().unwrap().len(), 4);
    let sun = light(s, "sun");
    assert_eq!(sun["kind"], "sun");
    assert_eq!(v3(&sun["dir"]), [0.0, 1.0, 0.0]);
    assert_eq!(sun["intensity"], 3.0);
    let lamp = light(s, "lamp");
    assert_eq!(lamp["kind"], "point");
    assert_eq!(v3(&lamp["pos"]), [400.0, 200.0, 150.0]);
    assert_eq!(lamp["reach"], 300.0);
    assert_eq!(lamp["size"], 10.0);
    // colour is in linear light: #ff8000 is 1.0, ~0.216, 0.0
    let c = v3(&lamp["color"]);
    assert!(
        (c[0] - 1.0).abs() < 1e-6 && (c[1] - 0.2158).abs() < 1e-3 && c[2] == 0.0,
        "{c:?}"
    );
    let spot = light(s, "spot");
    assert_eq!(spot["kind"], "spot");
    assert!((spot["angle"].as_f64().unwrap() - 30.0f64.to_radians()).abs() < 1e-9);
    assert_eq!(spot["softness"], 0.4);
    let sky = light(s, "sky");
    assert_eq!(sky["kind"], "ambient");
    assert!(sky["ground"].is_array());
}

#[test]
fn lights_are_not_drawn_shapes() {
    let src = format!("shape lamp = point_light(at: (1, 2, 3))\n{}", scene(1, ""));
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let shape = s["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "lamp")
        .unwrap();
    assert_eq!(shape["visible"], false);
}

#[test]
fn a_light_moves_fades_turns_and_changes_colour_like_any_shape() {
    let src = format!(
        "shape lamp = point_light(at: (100, 100, 100), intensity: 4, color: #ffffff)
shape sun = sun_light(dir: (0, 1, 0))
{}",
        scene(
            6,
            "        lamp.move_to((500, 300), over: 1.second)
        lamp.fade_to(0.25, over: 1.second)
        lamp.fill_to(#0000ff, over: 1.second)
        sun.rotate_to(90.deg, over: 1.second)
"
        )
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let lamp = light(s, "lamp");
    let p = v3(&lamp["pos"]);
    assert!(
        (p[0] - 500.0).abs() < 1e-6 && (p[1] - 300.0).abs() < 1e-6 && p[2] == 100.0,
        "{p:?}"
    );
    assert!(
        (lamp["intensity"].as_f64().unwrap() - 1.0).abs() < 1e-6,
        "fade multiplies intensity: {}",
        lamp["intensity"]
    );
    let c = v3(&lamp["color"]);
    assert!(c[0] < 1e-6 && c[2] > 0.99, "{c:?}");
    // a quarter turn about z takes (0, 1, 0) to (-1, 0, 0) or (1, 0, 0)
    let d = v3(&light(s, "sun")["dir"]);
    assert!(d[0].abs() > 0.99 && d[1].abs() < 1e-6, "{d:?}");
}

#[test]
fn a_light_between_keys_is_halfway() {
    let src = format!(
        "shape lamp = point_light(at: (0, 0, 0), intensity: 2)\n{}",
        scene(2, "        lamp.fade_to(0.0, over: 2.seconds)\n")
    );
    let r = go(&src);
    let mid = &r.snapshots[r.snapshots.len() / 2];
    let i = light(mid, "lamp")["intensity"].as_f64().unwrap();
    assert!(i > 0.5 && i < 1.5, "{i}");
}

#[test]
fn bad_lights_are_reported() {
    assert_eq!(
        err("shape l = point_light(at: (0, 0, 0), intensity: -1)\n").code,
        "E-LIT-002"
    );
    assert_eq!(
        err("shape l = spot_light(at: (0, 0, 0), dir: (0, 0, 0))\n").code,
        "E-LIT-002"
    );
    assert_eq!(
        err("shape l = spot_light(at: (0, 0, 0), angle: 120.deg)\n").code,
        "E-LIT-002"
    );
    assert_eq!(
        err("shape l = point_light(at: (0, 0, 0), bogus: 1)\n").code,
        "E-LIT-001"
    );
    assert_eq!(err("shape l = point_light()\n").code, "E-LIT-001");
}

#[test]
fn it_is_deterministic() {
    let src = format!(
        "shape lamp = point_light(at: (1, 2, 3))\n{}",
        scene(1, "        lamp.fade_to(0.3, over: 1.second)\n")
    );
    let (a, b) = (go(&src), go(&src));
    assert_eq!(
        a.snapshots.last().unwrap()["lights"],
        b.snapshots.last().unwrap()["lights"]
    );
}

#[test]
fn a_mesh_has_a_surface() {
    let src = format!(
        "shape ball = sphere(radius: 50, at: (300, 300, 100))\nball.surface(roughness: 0.2, metallic: 1, emissive: 2)\nshape plain = cube(size: 40, at: (600, 300, 100))\n{}",
        scene(1, "")
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let shapes = s["shapes"].as_array().unwrap();
    let ball = shapes.iter().find(|x| x["id"] == "ball").unwrap();
    assert_eq!(ball["mesh"]["surface"], serde_json::json!([0.2, 1.0, 2.0]));
    let plain = shapes.iter().find(|x| x["id"] == "plain").unwrap();
    assert!(plain["mesh"].get("surface").is_none());
    assert_eq!(
        err("shape b = cube(size: 10)\nb.surface(roughness: 2)\n").code,
        "E-LIT-002"
    );
}

#[test]
fn a_flat_shape_can_be_lit() {
    let src = format!(
        "shape moon = circle(n: 24, radius: 60, at: (300, 300))\nmoon.lit(bevel: 20, roughness: 0.3, emissive: 0.5, occluder: true, depth: 120)\nshape plain = circle(n: 24, radius: 20, at: (600, 300))\n{}",
        scene(1, "")
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    let shapes = s["shapes"].as_array().unwrap();
    let moon = shapes.iter().find(|x| x["id"] == "moon").unwrap();
    assert_eq!(moon["lit"]["bevel"], 20.0);
    assert_eq!(moon["lit"]["surface"], serde_json::json!([0.3, 0.0, 0.5]));
    assert_eq!(moon["lit"]["occluder"], true);
    assert_eq!(moon["lit"]["depth"], 120.0);
    assert!(shapes
        .iter()
        .find(|x| x["id"] == "plain")
        .unwrap()
        .get("lit")
        .is_none());
}

#[test]
fn lit_is_rejected_on_meshes_and_bad_values() {
    assert_eq!(err("shape b = cube(size: 10)\nb.lit()\n").code, "E-LIT-001");
    assert_eq!(
        err("shape c = circle(n: 12, radius: 10)\nc.lit(roughness: 3)\n").code,
        "E-LIT-002"
    );
    assert_eq!(
        err("shape c = circle(n: 12, radius: 10)\nc.lit(occluder: 1)\n").code,
        "E-LIT-002"
    );
}
