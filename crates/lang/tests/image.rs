//! Image fills: `shape.fill = image("file.png")` maps a PNG onto the shape's faces by per-vertex UV
//! coordinates, so the picture follows every deformation. Written first.
use animscript_lang::*;
use serde_json::Value;
use std::path::Path;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";
const BASE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");

fn go(body: &str) -> RunResult {
    let text = format!("{PROJECT}{body}");
    match run_in(&text, &RunOptions::default(), Path::new(BASE)) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(body: &str) -> Diagnostic {
    match run_in(
        &format!("{PROJECT}{body}"),
        &RunOptions::default(),
        Path::new(BASE),
    ) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn tris(snap: &Value, shape: usize) -> Vec<Vec<[f64; 4]>> {
    snap["shapes"][shape]["image"]["tris"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            t.as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    [
                        p[0].as_f64().unwrap(),
                        p[1].as_f64().unwrap(),
                        p[2].as_f64().unwrap(),
                        p[3].as_f64().unwrap(),
                    ]
                })
                .collect()
        })
        .collect()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} vs {b}");
}
const SC: &str = "scene \"s\" for 2.frames:\n    main:\n        wait 2.frames\n";

#[test]
fn an_image_fill_maps_the_picture_onto_a_face() {
    let r = go(&format!(
        "shape s = square(size: 100, at: (200, 200))\ns.fill = image(\"data/checker.png\")\n{SC}"
    ));
    let img = &r.snapshots[0]["shapes"][0]["image"];
    assert_eq!(img["src"], "data/checker.png");
    assert_eq!((img["w"].as_u64(), img["h"].as_u64()), (Some(4), Some(4)));
    let t = tris(&r.snapshots[0], 0);
    assert_eq!(t.len(), 2); // a quad is two triangles
                            // each corner of the square carries the matching corner of the picture
    for (x, y, u, v) in [
        (150.0, 150.0, 0.0, 0.0),
        (250.0, 150.0, 1.0, 0.0),
        (250.0, 250.0, 1.0, 1.0),
        (150.0, 250.0, 0.0, 1.0),
    ] {
        let p = t
            .iter()
            .flatten()
            .find(|p| (p[0] - x).abs() < 1e-9 && (p[1] - y).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no corner at {x},{y}: {t:?}"));
        near(p[2], u);
        near(p[3], v);
    }
}

#[test]
fn subdividing_keeps_the_mapping_linear() {
    let r = go(&format!("shape s = square(size: 100, at: (200, 200))\ns.fill = image(\"data/checker.png\")\ns.f1.subdivide(2)\n{SC}"));
    let t = tris(&r.snapshots[0], 0);
    assert_eq!(t.len(), 8); // 2 x 2 faces, two triangles each
    let centre = t
        .iter()
        .flatten()
        .find(|p| (p[0] - 200.0).abs() < 1e-9 && (p[1] - 200.0).abs() < 1e-9)
        .unwrap();
    near(centre[2], 0.5);
    near(centre[3], 0.5);
}

#[test]
fn the_picture_follows_transforms_and_deformation() {
    let r = go("shape s = square(size: 100, at: (200, 200))\ns.fill = image(\"data/checker.png\")\nscene \"s\" for 2.frames:\n    main:\n        s.v[1].move_by(30, 0) over 1.frame\n        wait 1.frame\n");
    let (a, b) = (tris(&r.snapshots[0], 0), tris(&r.snapshots[1], 0));
    let moved = |t: &Vec<Vec<[f64; 4]>>| {
        t.iter()
            .flatten()
            .filter(|p| p[2] == 0.0 && p[3] == 0.0)
            .map(|p| p[0])
            .fold(f64::NAN, f64::min)
    };
    near(moved(&b) - moved(&a), 30.0); // the corner moved, and it still carries uv (0, 0)
    let r = go(&format!("shape s = square(size: 100, at: (200, 200))\ns.fill = image(\"data/checker.png\")\ns.rotation = 0.25.turns\n{SC}"));
    let t = tris(&r.snapshots[0], 0);
    // a quarter turn: the picture's top-left corner is now at the top-right of the square
    let tl = t
        .iter()
        .flatten()
        .find(|p| p[2] == 0.0 && p[3] == 0.0)
        .unwrap();
    near(tl[0], 250.0);
    near(tl[1], 150.0);
}

#[test]
fn a_soft_shape_drags_its_texture_with_it() {
    let r = go("shape cloth = grid(4, width: 120, height: 120, at: (300, 100))\ncloth.fill = image(\"data/checker.png\")\ncloth.soft(stiffness: 0.5)\ncloth.pin(cloth.v[1])\nscene \"s\" for 1.second:\n    main:\n        wait 1.second\n");
    let (a, b) = (
        tris(&r.snapshots[0], 0),
        tris(r.snapshots.last().unwrap(), 0),
    );
    assert_eq!(a.len(), 32);
    assert_ne!(
        a[10][1][0], b[10][1][0],
        "the triangles moved with the simulation"
    );
    near(a[10][1][2], b[10][1][2]); // but the uv of a vertex never changes
    near(a[10][1][3], b[10][1][3]);
}

#[test]
fn fill_colour_and_image_coexist_and_none_removes_the_image() {
    let r = go(&format!(
        "shape s = square(size: 100)\ns.fill = image(\"data/checker.png\")\ns.fill = #336699\n{SC}"
    ));
    assert!(
        r.snapshots[0]["shapes"][0].get("image").is_some(),
        "a colour fill does not remove the image"
    );
    assert_eq!(r.snapshots[0]["shapes"][0]["fill"], "#336699");
    let r = go(&format!(
        "shape s = square(size: 100)\ns.fill = image(\"data/checker.png\")\ns.image = none\n{SC}"
    ));
    assert!(r.snapshots[0]["shapes"][0].get("image").is_none());
    let none = go(&format!("shape s = square(size: 100)\n{SC}"));
    assert!(none.snapshots[0]["shapes"][0].get("image").is_none());
}

#[test]
fn bad_images_are_rejected() {
    assert_eq!(
        err(&format!(
            "shape s = square(size: 100)\ns.fill = image(\"data/missing.png\")\n{SC}"
        ))
        .code,
        "E-AST-001"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 100)\ns.fill = image(\"data/notapng.png\")\n{SC}"
        ))
        .code,
        "E-AST-002"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 100)\ns.f1.fill = image(\"data/checker.png\")\n{SC}"
        ))
        .code,
        "E-TYP-001"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 100)\ns.image = #ff0000\n{SC}"
        ))
        .code,
        "E-TYP-001"
    );
}

#[test]
fn the_same_program_gives_the_same_hashes() {
    let s = "shape s = square(size: 100, at: (200, 200))\ns.fill = image(\"data/checker.png\")\nscene \"s\" for 3.frames:\n    main:\n        s.rotate_by(90.deg) over 2.frames\n        wait 2.frames\n";
    assert_eq!(go(s).hashes, go(s).hashes);
}
