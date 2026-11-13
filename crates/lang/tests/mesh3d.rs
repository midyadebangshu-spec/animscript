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

// ---- vertex drives: equations move the vertices of a mesh --------------------------------------------------------------

#[test]
fn a_drive_can_move_the_vertices_of_a_mesh_by_an_equation() {
    // a 4 x 4 floor: the height of every vertex follows a wave of x and the time
    let src = format!(
        "shape floor = plane(width: 300, depth: 300, nx: 3, nz: 3, at: (400, 500, 0))\n{}",
        scene(
            11,
            "        start floor.drive(y: \"-20 * sin(x / 50 + t * 3)\")\n"
        )
    );
    let r = go(&src);
    let v0 = verts(mesh(&r.snapshots[0], 0));
    let v10 = verts(mesh(&r.snapshots[10], 0));
    assert_eq!(v0.len(), 16);
    // the rest height is 500; x runs from -150 to 150 around 400
    for v in &v0 {
        let local_x = v[0] - 400.0;
        near(v[1], 500.0 - 20.0 * (local_x / 50.0).sin(), 1e-9);
    }
    for v in &v10 {
        let local_x = v[0] - 400.0;
        near(v[1], 500.0 - 20.0 * (local_x / 50.0 + 3.0).sin(), 1e-9);
    }
    // x and z are untouched
    assert_eq!(
        v0.iter().map(|v| v[2].to_bits()).collect::<Vec<_>>(),
        v10.iter().map(|v| v[2].to_bits()).collect::<Vec<_>>()
    );
}

#[test]
fn a_mesh_drive_has_z_and_the_vertex_index_and_replace_mode() {
    let src = format!(
        "shape box = cube(size: 100, at: (400, 300, 0))\n{}",
        scene(
            3,
            "        start box.drive(z: \"z * 2\", mode: \"replace\")\n"
        )
    );
    let r = go(&src);
    let (lo, hi) = bbox(mesh(&r.snapshots[1], 0));
    near(hi[2] - lo[2], 200.0, 1e-9); // the depth doubled: z was replaced by 2 z
    near(hi[0] - lo[0], 100.0, 1e-9);
    // `z` is for meshes only
    let d = err(&format!(
        "shape s = square(size: 50)\n{}",
        scene(2, "        start s.drive(z: \"1\")\n")
    ));
    assert_eq!(d.code, "E-MTH-004");
}

#[test]
fn a_mesh_drive_reads_a_field_at_the_vertex_x_z() {
    // the field covers the floor's world rectangle (x 300..500, z -100..100); its value is x - 400: the height
    let src = format!(
        "shape floor = plane(width: 200, depth: 200, nx: 2, nz: 2, at: (400, 500, 0))\nfield pond:\n    grid: (2, 2)\n    rect: (300, -100, 200, 200)\n    state: [\"h\"]\n    init: [(\"h\", \"x - 400\")]\n    step: [(\"h\", \"h\")]\n{}",
        scene(3, "        start floor.drive(y: \"-pond * 10\", fields: [pond])\n")
    );
    let r = go(&src);
    let v = verts(mesh(&r.snapshots[1], 0));
    // cell centres are at x = -50 and 50: the field is x there, clamped beyond (so -50 at x = -100, 0 at x = 0, 50 at x = 100)
    for p in &v {
        let lx = p[0] - 400.0;
        let want = lx.clamp(-50.0, 50.0);
        near(p[1], 500.0 - want * 10.0, 1e-9);
    }
}

// ---- std/3d ----------------------------------------------------------------------------------------------------------

#[test]
fn std_3d_spins_bobs_sways_and_orbits() {
    let body = "        start box.spin(rate: 0.125, over: 2.seconds)\n        start box.bob(amount: 10, period: 2.seconds, over: 2.seconds)\n        start camera3.orbit(turns: 0.25, over: 2.seconds)\n";
    let src = format!(
        "import \"std/3d\"\nshape box = cube(size: 100, at: (640, 360, 0))\n{}",
        scene(21, body)
    );
    let r = go(&src);
    // after one second: an eighth of a turn (spin), 10 sin(pi) = 0 (bob), an eighth of the orbit
    let (lo, hi) = bbox(mesh(&r.snapshots[10], 0));
    near(hi[0] - lo[0], 100.0 * 2f64.sqrt(), 1e-6);
    let c = &r.snapshots[10]["camera3"];
    let f0 = 360.0 / 20.0f64.to_radians().tan();
    let a = std::f64::consts::TAU * 0.125 * 1.0 / 1.0;
    let a = a / 2.0 * 2.0; // rate = 0.25 turns / 2 s = 0.125 turns per second, so 45 degrees at t = 1
    near(c["eye"][0].as_f64().unwrap(), 640.0 + f0 * a.sin(), 1e-6);
    near(
        c["eye"][2].as_f64().unwrap(),
        -f0 + f0 * (1.0 - a.cos()),
        1e-6,
    );
    near(c["forward"][0].as_f64().unwrap(), -a.sin(), 1e-9);
    near(c["forward"][2].as_f64().unwrap(), a.cos(), 1e-9);
    // the camera keeps looking at the middle of the stage: the point (640, 360, 0) is straight ahead
    let eye = [
        c["eye"][0].as_f64().unwrap(),
        c["eye"][1].as_f64().unwrap(),
        c["eye"][2].as_f64().unwrap(),
    ];
    let d = [640.0 - eye[0], 360.0 - eye[1], 0.0 - eye[2]];
    let fwd = [
        c["forward"][0].as_f64().unwrap(),
        0.0,
        c["forward"][2].as_f64().unwrap(),
    ];
    let len = (d[0] * d[0] + d[2] * d[2]).sqrt();
    near(d[0] / len, fwd[0], 1e-9);
    near(d[2] / len, fwd[2], 1e-9);
}

#[test]
fn std_3d_sway_and_crane_run() {
    let body = "        start box.sway(over: 1.second)\n        start camera3.crane(height: 100, pitch: 0.05, over: 2.seconds)\n";
    let src = format!(
        "import \"std/3d\"\nshape box = cube(size: 100, at: (640, 360, 0))\n{}",
        scene(21, body)
    );
    let r = go(&src);
    let c = &r.snapshots[10]["camera3"];
    assert!(c["eye"][1].as_f64().unwrap() < 360.0, "the camera rose");
    assert!(
        c["forward"][1].as_f64().unwrap() > -1.0 && c["forward"][1].as_f64().unwrap() != 0.0,
        "and looks down or up a little"
    );
}

#[test]
fn pivot_is_the_world_point_a_mesh_turns_about() {
    // a cube at x = 500 turned a quarter about a pivot at x = 400 (in the xz plane): its centre moves to z = -100 / 100
    let src = format!(
        "shape box = cube(size: 40, at: (500, 300, 0), pivot: (400, 300, 0))\n{}",
        scene(11, "        start box.drive(rot_y: \"t * 0.25\")\n")
    );
    let r = go(&src);
    let c0 = centre(mesh(&r.snapshots[0], 0));
    near(c0[0], 500.0, 1e-9);
    let c = centre(mesh(&r.snapshots[10], 0));
    near(c[0], 400.0, 1e-9);
    near(c[2].abs(), 100.0, 1e-9);
    assert_eq!(
        err(&format!(
            "shape box = cube(size: 40, pivot: \"middle\")\n{}",
            scene(2, "")
        ))
        .code,
        "E-MSH-001"
    );
}

#[test]
fn a_cube_can_be_subdivided() {
    let r = go(&format!(
        "shape c = cube(size: 100, subdivide: 4, at: (400, 300, 0))\n{}",
        scene(1, "")
    ));
    let m = mesh(&r.snapshots[0], 0);
    assert_eq!(m["faces"].as_array().unwrap().len(), 6 * 16);
    assert_eq!(err("shape c = cube(subdivide: 100)\n").code, "E-MSH-001");
}
