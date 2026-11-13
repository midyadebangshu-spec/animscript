//! 3D meshes as shapes: builtins, 3D transforms, camera3, stage3, errors and the snapshot.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 1280x720\n    fps: 10\n\n";

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
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
fn scene(ticks: u32, body: &str) -> String {
    format!("scene \"s\" for {ticks}.frames:\n    main:\n{body}        wait {ticks}.frames\n")
}
fn mesh(snap: &Value, shape: usize) -> &Value {
    &snap["shapes"][shape]["mesh"]
}
fn verts(m: &Value) -> Vec<[f64; 3]> {
    m["verts"]
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
fn bbox(m: &Value) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for v in verts(m) {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    (lo, hi)
}
fn centre(m: &Value) -> [f64; 3] {
    let (lo, hi) = bbox(m);
    [
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    ]
}

// ---- the builtins --------------------------------------------------------------------------------------------

#[test]
fn a_cube_is_a_shape_with_a_mesh_in_the_snapshot() {
    let r = go(&format!("shape box = cube(size: 100)\nbox.fill = #c98a2b\nbox.stroke = #24151a\nbox.stroke_width = 3\n{}", scene(2, "")));
    let m = mesh(&r.snapshots[0], 0);
    assert_eq!(m["verts"].as_array().unwrap().len(), 8);
    assert_eq!(m["faces"].as_array().unwrap().len(), 6);
    assert_eq!(m["fill"], "#c98a2b");
    assert_eq!(m["outline"], "#24151a");
    assert_eq!(m["width"].as_f64(), Some(3.0));
    assert_eq!(m["double_sided"], false);
    assert_eq!(bbox(m), ([-50.0, -50.0, -50.0], [50.0, 50.0, 50.0]));
    // it has no flat geometry of its own
    assert!(r.snapshots[0]["shapes"][0]["faces"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn builtins_make_spheres_cylinders_cones_planes_lathes_and_extrusions() {
    let src = format!(
        "shape a = sphere(radius: 40, segments: 16, rings: 8)\nshape b = cylinder(radius: 20, height: 100, segments: 12)\nshape c = cone(radius: 30, height: 60, segments: 12)\nshape d = plane(width: 300, depth: 200, nx: 3, nz: 2)\nshape e = lathe(profile: [(0, 0), (30, 0), (20, 60), (0, 60)], segments: 16)\nshape tri = polygon([(0, 0), (80, 0), (40, 60)])\nshape f = extrude(tri, depth: 30)\nshape g = extrude(points: [(0, 0), (60, 0), (60, 20), (20, 20), (20, 60), (0, 60)], depth: 10)\n{}",
        scene(2, "")
    );
    let r = go(&src);
    let s = &r.snapshots[0];
    let count = |k: usize| {
        (
            mesh(s, k)["verts"].as_array().unwrap().len(),
            mesh(s, k)["faces"].as_array().unwrap().len(),
        )
    };
    assert_eq!(count(0), (2 + 7 * 16, 16 * 8)); // poles are single vertices; the polar rows are triangles
    assert_eq!(mesh(s, 3)["double_sided"], true);
    assert_eq!(count(3), (4 * 3, 6));
    let (lo, hi) = bbox(mesh(s, 4));
    near(lo[1], -60.0, 1e-9);
    near(hi[1], 0.0, 1e-9);
    assert!(hi[0] <= 30.0 + 1e-9);
    // extrusion: the triangle (80 x 60) pushed 30: its centre ends up where the polygon's centre was, with depth 30
    let (lo, hi) = bbox(mesh(s, 6));
    near(hi[2] - lo[2], 30.0, 1e-9);
    near(hi[0] - lo[0], 80.0, 1e-9);
    // the concave L is triangulated into a closed solid
    assert!(mesh(s, 7)["faces"].as_array().unwrap().len() > 8);
    // the flat triangle used for `extrude` is still an ordinary shape
    assert_eq!(s["shapes"][5]["mesh"], Value::Null);
}

#[test]
fn at_places_the_pivot_in_the_scene() {
    let r = go(&format!("shape box = cube(size: 100, at: (400, 300, 50))\nshape vase = lathe(profile: [(0, 0), (30, 0), (20, 60), (0, 60)], segments: 8, at: (200, 500))\n{}", scene(2, "")));
    let c = centre(mesh(&r.snapshots[0], 0));
    for (a, b) in c.iter().zip([400.0, 300.0, 50.0]) {
        near(*a, b, 1e-9);
    }
    // for a lathe the bounding-box centre goes to `at`
    let c = centre(mesh(&r.snapshots[0], 1));
    for (a, b) in c.iter().zip([200.0, 500.0, 0.0]) {
        near(*a, b, 1e-9);
    }
}

// ---- 3D transforms: keyed, driven ---------------------------------------------------------------------------------------

#[test]
fn position_z_rotation_x_y_and_scale_z_are_animatable() {
    let src = "shape box = cube(size: 100, at: (400, 300))\nscene \"s\" for 4.frames:\n    timeline:\n        at 0f:\n            box.position.z = 0\n            box.rotation.y = 0.deg\n            box.scale.z = 1\n        at 3f:\n            box.position.z = 90\n            box.rotation.y = 90.deg\n            box.scale.z = 2\n    main:\n        wait 4.frames\n";
    let r = go(src);
    let c = centre(mesh(&r.snapshots[3], 0));
    near(c[2], 90.0, 1e-9);
    // a quarter turn about y of a cube stretched 2x in z: it is now 200 wide (x) and 100 deep... wait: scale first, then turn
    let (lo, hi) = bbox(mesh(&r.snapshots[3], 0));
    near(hi[0] - lo[0], 200.0, 1e-9);
    near(hi[2] - lo[2], 100.0, 1e-9);
    let (lo, hi) = bbox(mesh(&r.snapshots[0], 0));
    near(hi[2] - lo[2], 100.0, 1e-9);
}

#[test]
fn a_drive_spins_a_mesh_by_an_equation() {
    let src = format!(
        "shape box = cube(size: 100, at: (400, 300))\n{}",
        scene(
            11,
            "        start box.drive(rot_y: \"t * 0.25\", pos_z: \"t * 100\")\n"
        )
    );
    let r = go(&src);
    // after 1 s (tick 10) the box has turned a quarter and moved 100 deeper
    near(centre(mesh(&r.snapshots[10], 0))[2], 100.0, 1e-9);
    let a = bbox(mesh(&r.snapshots[0], 0));
    let b = bbox(mesh(&r.snapshots[10], 0));
    // a cube turned by a quarter looks the same; by an eighth (tick 5) its x extent is 100 * sqrt(2)
    let e = bbox(mesh(&r.snapshots[5], 0));
    near(e.1[0] - e.0[0], 100.0 * 2f64.sqrt(), 1e-9);
    near(a.1[0] - a.0[0], b.1[0] - b.0[0], 1e-9);
}

// ---- camera3 and the stage ------------------------------------------------------------------------------------------------

#[test]
fn the_default_camera_shows_the_z_equals_0_plane_at_canvas_scale() {
    let r = go(&format!("shape box = cube(size: 100)\n{}", scene(2, "")));
    let c = &r.snapshots[0]["camera3"];
    let f0 = 360.0 / (20.0f64.to_radians()).tan();
    near(c["focal"].as_f64().unwrap(), f0, 1e-9);
    let eye = c["eye"].as_array().unwrap();
    near(eye[0].as_f64().unwrap(), 640.0, 1e-9);
    near(eye[1].as_f64().unwrap(), 360.0, 1e-9);
    near(eye[2].as_f64().unwrap(), -f0, 1e-9);
    assert_eq!(c["forward"], serde_json::json!([0.0, 0.0, 1.0]));
    assert_eq!(c["cx"].as_f64(), Some(640.0));
    // a scene without meshes has no 3D section at all
    let flat = go(&format!("shape sq = square(size: 50)\n{}", scene(2, "")));
    assert!(
        flat.snapshots[0].get("camera3").is_none() && flat.snapshots[0].get("light3").is_none()
    );
}

#[test]
fn stage3_sets_the_lens_and_the_light_and_camera3_can_move_and_turn() {
    let src = "stage3(fov: 60, light: (0, -1, 0), ambient: 0.2, bands: 4, crease: 30.deg, smooth: 60.deg)\nshape box = cube(size: 100)\nscene \"s\" for 4.frames:\n    timeline:\n        at 0f:\n            camera3.position.x = 0\n            camera3.position.z = 0\n            camera3.rotation.y = 0.deg\n            camera3.scale.x = 1\n        at 3f:\n            camera3.position.x = 100\n            camera3.position.z = 50\n            camera3.rotation.y = 90.deg\n            camera3.scale.x = 2\n    main:\n        wait 4.frames\n";
    let r = go(src);
    let l = &r.snapshots[0]["light3"];
    assert_eq!(l["ambient"].as_f64(), Some(0.2));
    assert_eq!(l["bands"].as_u64(), Some(4));
    near(l["crease"].as_f64().unwrap(), 30.0, 1e-9);
    near(l["smooth"].as_f64().unwrap(), 60.0, 1e-9);
    let d = l["dir"].as_array().unwrap();
    near(d[1].as_f64().unwrap(), -1.0, 1e-9);
    let f0 = 360.0 / (30.0f64.to_radians()).tan();
    near(
        r.snapshots[0]["camera3"]["focal"].as_f64().unwrap(),
        f0,
        1e-9,
    );
    let c = &r.snapshots[3]["camera3"];
    near(c["focal"].as_f64().unwrap(), 2.0 * f0, 1e-9);
    near(c["eye"][0].as_f64().unwrap(), 740.0, 1e-9);
    near(c["eye"][2].as_f64().unwrap(), -f0 + 50.0, 1e-9);
    // turned a quarter about y: it looks along +x
    near(c["forward"][0].as_f64().unwrap(), 1.0, 1e-9);
    near(c["forward"][2].as_f64().unwrap(), 0.0, 1e-9);
}

// ---- errors -----------------------------------------------------------------------------------------------------------------

#[test]
fn mesh_errors() {
    let e = |s: &str| err(&format!("{s}\n{}", scene(2, ""))).code;
    assert_eq!(e("shape a = cube(size: -5)"), "E-MSH-001");
    assert_eq!(e("shape a = sphere(radius: 0)"), "E-MSH-001");
    assert_eq!(e("shape a = sphere(radius: 10, segments: 2)"), "E-MSH-001");
    assert_eq!(
        e("shape a = lathe(profile: [(1, 0)], segments: 8)"),
        "E-MSH-001"
    );
    assert_eq!(
        e("shape a = lathe(profile: [(-1, 0), (2, 1)], segments: 8)"),
        "E-MSH-001"
    );
    assert_eq!(
        e("shape a = extrude(points: [(0, 0), (10, 10), (10, 0), (0, 10)], depth: 5)"),
        "E-MSH-001"
    ); // crosses itself
    assert_eq!(
        e("shape a = extrude(points: [(0, 0), (10, 0), (5, 9)], depth: 0)"),
        "E-MSH-001"
    );
    assert_eq!(e("shape a = extrude(depth: 5)"), "E-TYP-003");
    assert_eq!(
        e("shape a = plane(width: 10, depth: 10, nx: 0, nz: 2)"),
        "E-MSH-001"
    );
    assert_eq!(e("stage3(fov: 0)"), "E-MSH-001");
    assert_eq!(e("stage3(ambient: 2)"), "E-MSH-001");
    assert_eq!(e("stage3(bands: -1)"), "E-MSH-001");
    assert_eq!(e("stage3(smooth: 3)"), "E-MSH-001");
    assert_eq!(e("stage3(light: (0, 0, 0))"), "E-MSH-001");
    assert_eq!(
        e("shape a = cube(size: 10)\nshape b = cube(size: 10)\nb.rotation.q = 1"),
        "E-TYP-001"
    );
}
