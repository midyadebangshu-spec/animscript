//! 3D meshes in the snapshot: a z-buffer rasterizer with flat or toon shading and inked outlines.
use animscript_render::render_with;
use serde_json::{json, Value};

const F: f64 = 400.0; // the focal length in these tests (canvas 200 x 100, camera at (100, 50, -400))

fn cube(cx: f64, cy: f64, cz: f64, half: f64, fill: &str, outline: Value, width: f64) -> Value {
    let (x0, x1, y0, y1, z0, z1) = (
        cx - half,
        cx + half,
        cy - half,
        cy + half,
        cz - half,
        cz + half,
    );
    let verts = json!([
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1]
    ]);
    // outward normals (cross of the first two edges): the front face looks at -z
    let faces = json!([
        [3, 2, 1, 0],
        [4, 5, 6, 7],
        [4, 7, 3, 0],
        [1, 2, 6, 5],
        [0, 1, 5, 4],
        [7, 6, 2, 3]
    ]);
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": verts, "faces": faces, "double_sided": false, "fill": fill, "outline": outline, "width": width}})
}
fn snap(shapes: Vec<Value>, light: Value) -> Value {
    json!({"canvas": [200, 100], "background": "#ffffff", "camera": {"position": [100.0, 50.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 50.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 50.0},
        "light3": light, "shapes": shapes})
}
/// Light travelling along +z (it hits the faces that look at the viewer head on), no shading bands.
fn flat_light(ambient: f64) -> Value {
    json!({"dir": [0.0, 0.0, 1.0], "ambient": ambient, "bands": 0, "crease": 40.0})
}
fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}
fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}
fn is_bg(c: (u8, u8, u8)) -> bool {
    c == (255, 255, 255)
}

#[test]
fn a_cube_in_front_of_the_camera_is_a_filled_square_in_its_colour() {
    let pm = render(&snap(
        vec![cube(100.0, 50.0, 0.0, 20.0, "#ff0000", Value::Null, 0.0)],
        flat_light(0.3),
    ));
    assert_eq!(
        px(&pm, 100, 50),
        (255, 0, 0),
        "the front face is fully lit: its own colour"
    );
    assert!(is_bg(px(&pm, 100, 5)) && is_bg(px(&pm, 20, 50)) && is_bg(px(&pm, 180, 90)));
    // the front face is at z = -20: its half size is 20 * F / (F - 20) = 21.05 pixels
    assert!(
        !is_bg(px(&pm, 100 + 20, 50)) && is_bg(px(&pm, 100 + 23, 50)),
        "{:?} {:?}",
        px(&pm, 120, 50),
        px(&pm, 123, 50)
    );
}

#[test]
fn nearer_things_are_larger_and_hide_what_is_behind() {
    let width = |z: f64| {
        let pm = render(&snap(
            vec![cube(100.0, 50.0, z, 15.0, "#00aa00", Value::Null, 0.0)],
            flat_light(0.3),
        ));
        (0..200).filter(|x| !is_bg(px(&pm, *x, 50))).count()
    };
    assert!(
        width(-100.0) > width(0.0) && width(0.0) > width(200.0),
        "{} {} {}",
        width(-100.0),
        width(0.0),
        width(200.0)
    );
    // a near red cube covers part of a far blue one, whichever is listed first
    let (near, far) = (
        cube(90.0, 50.0, -60.0, 15.0, "#ff0000", Value::Null, 0.0),
        cube(110.0, 50.0, 60.0, 15.0, "#0000ff", Value::Null, 0.0),
    );
    for order in [vec![near.clone(), far.clone()], vec![far, near]] {
        let pm = render(&snap(order, flat_light(0.3)));
        assert_eq!(
            px(&pm, 100, 50),
            (255, 0, 0),
            "red is in front where they overlap"
        );
        assert_eq!(px(&pm, 116, 50), (0, 0, 255), "blue shows beside it");
    }
}

#[test]
fn faces_that_look_away_are_culled_unless_the_mesh_is_double_sided() {
    let mut plane = cube(100.0, 50.0, 0.0, 20.0, "#ff0000", Value::Null, 0.0);
    // a single square facing +z (away from the camera)
    plane["mesh"]["verts"] = json!([
        [80.0, 30.0, 0.0],
        [120.0, 30.0, 0.0],
        [120.0, 70.0, 0.0],
        [80.0, 70.0, 0.0]
    ]);
    plane["mesh"]["faces"] = json!([[0, 1, 2, 3]]);
    let pm = render(&snap(vec![plane.clone()], flat_light(0.3)));
    assert!(is_bg(px(&pm, 100, 50)), "its back is not drawn");
    plane["mesh"]["double_sided"] = json!(true);
    let pm = render(&snap(vec![plane], flat_light(0.3)));
    assert!(
        !is_bg(px(&pm, 100, 50)),
        "double sided: drawn from either side"
    );
}

#[test]
fn lighting_ambient_flat_and_toon_bands() {
    // the light travels towards the viewer: the front face is not lit, only ambient
    let dark = json!({"dir": [0.0, 0.0, -1.0], "ambient": 0.25, "bands": 0, "crease": 40.0});
    let pm = render(&snap(
        vec![cube(100.0, 50.0, 0.0, 20.0, "#ff8000", Value::Null, 0.0)],
        dark,
    ));
    let (r, g, _) = px(&pm, 100, 50);
    assert!(
        (r as f64 - 255.0 * 0.25).abs() <= 1.0 && (g as f64 - 128.0 * 0.25).abs() <= 1.5,
        "{:?}",
        px(&pm, 100, 50)
    );
    // a sphere-like fan of normals under 3 bands shows only a few distinct shades
    let mut tri = cube(100.0, 50.0, 0.0, 20.0, "#ffffff", Value::Null, 0.0);
    let n = 24;
    let mut verts = vec![json!([100.0, 50.0, -40.0])];
    let mut faces = vec![];
    for i in 0..n {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        verts.push(json!([
            100.0 + 30.0 * a.cos(),
            50.0 + 30.0 * a.sin(),
            -10.0 + 25.0 * (1.0 - (a.cos() * 0.5 + 0.5))
        ]));
    }
    for i in 0..n {
        faces.push(json!([0, 1 + (i + 1) % n, 1 + i]));
    }
    tri["mesh"]["verts"] = json!(verts);
    tri["mesh"]["faces"] = json!(faces);
    tri["mesh"]["double_sided"] = json!(true);
    let banded = json!({"dir": [-0.4, 0.5, 0.8], "ambient": 0.2, "bands": 3, "crease": 40.0});
    let pm = render(&snap(vec![tri.clone()], banded));
    let mut shades = std::collections::BTreeSet::new();
    for i in 0..n {
        let a = std::f64::consts::TAU * (i as f64 + 0.5) / n as f64;
        let (x, y) = (
            (100.0 + 12.0 * a.cos()) as u32,
            (50.0 + 12.0 * a.sin()) as u32,
        );
        shades.insert(px(&pm, x, y).0);
    }
    assert!(
        shades.len() <= 3 && shades.len() >= 2,
        "bands give a few flat shades: {shades:?}"
    );
    let smooth = json!({"dir": [-0.4, 0.5, 0.8], "ambient": 0.2, "bands": 0, "crease": 40.0});
    let pm = render(&snap(vec![tri], smooth));
    let mut many = std::collections::BTreeSet::new();
    for i in 0..n {
        let a = std::f64::consts::TAU * (i as f64 + 0.5) / n as f64;
        many.insert(
            px(
                &pm,
                (100.0 + 12.0 * a.cos()) as u32,
                (50.0 + 12.0 * a.sin()) as u32,
            )
            .0,
        );
    }
    assert!(
        many.len() > shades.len(),
        "smooth lighting has more shades than bands"
    );
}

#[test]
fn outlines_are_inked_on_the_silhouette_and_hidden_behind_nearer_meshes() {
    let pm = render(&snap(
        vec![cube(
            100.0,
            50.0,
            0.0,
            20.0,
            "#ffcc00",
            json!("#000000"),
            3.0,
        )],
        flat_light(0.3),
    ));
    // the silhouette: the square's edge (21 pixels from the centre) is dark
    let edge = px(&pm, 100 + 21, 50);
    assert!(edge.0 < 60 && edge.1 < 60, "{edge:?}");
    assert_eq!(px(&pm, 100, 50), (255, 204, 0), "the inside is untouched");
    // a far cube's outline is not drawn where a nearer cube covers it
    let near = cube(100.0, 50.0, -100.0, 30.0, "#ffffff", Value::Null, 0.0);
    let far = cube(100.0, 50.0, 100.0, 20.0, "#ffcc00", json!("#000000"), 3.0);
    let pm = render(&snap(vec![near, far], flat_light(0.3)));
    assert_eq!(
        px(&pm, 100 + 17, 50),
        (255, 255, 255),
        "{:?}",
        px(&pm, 117, 50)
    );
    // a crease between two visible faces is inked: a cube turned 40 degrees about y shows two faces
    let a = 40.0f64.to_radians();
    let mut c = cube(0.0, 0.0, 0.0, 20.0, "#ffcc00", json!("#000000"), 2.0);
    let verts: Vec<Value> = c["mesh"]["verts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let (x, y, z) = (
                v[0].as_f64().unwrap(),
                v[1].as_f64().unwrap(),
                v[2].as_f64().unwrap(),
            );
            json!([
                100.0 + x * a.cos() + z * a.sin(),
                50.0 + y,
                -x * a.sin() + z * a.cos()
            ])
        })
        .collect();
    c["mesh"]["verts"] = json!(verts);
    let pm = render(&snap(vec![c], flat_light(0.3)));
    // the vertical corner nearest the camera is at the front: a dark column of pixels near x = 100 - 20 * sin 40 + ...
    let dark_in_row: Vec<u32> = (60..140)
        .filter(|x| {
            let p = px(&pm, *x, 50);
            p.0 < 90 && p.1 < 90
        })
        .collect();
    assert!(
        dark_in_row.len() >= 4,
        "silhouette sides and the crease: {dark_in_row:?}"
    );
}

#[test]
fn the_camera_zoom_and_the_2d_camera_scale_the_picture() {
    let width = |s: &Value| {
        let pm = render(s);
        (0..200).filter(|x| !is_bg(px(&pm, *x, 50))).count()
    };
    let base = snap(
        vec![cube(100.0, 50.0, 0.0, 10.0, "#00aa00", Value::Null, 0.0)],
        flat_light(0.3),
    );
    let mut zoomed = base.clone();
    zoomed["camera3"]["focal"] = json!(2.0 * F);
    let mut cam2d = base.clone();
    cam2d["camera"]["zoom"] = json!(2.0);
    let w = width(&base);
    assert!(
        (width(&zoomed) as f64 / w as f64 - 2.0).abs() < 0.2,
        "{} vs {w}",
        width(&zoomed)
    );
    assert!(
        (width(&cam2d) as f64 / w as f64 - 2.0).abs() < 0.2,
        "{} vs {w}",
        width(&cam2d)
    );
}

#[test]
fn things_behind_the_camera_and_straddling_its_plane_do_not_break_the_picture() {
    let behind = render(&snap(
        vec![cube(100.0, 50.0, -600.0, 20.0, "#ff0000", Value::Null, 0.0)],
        flat_light(0.3),
    ));
    assert!(is_bg(px(&behind, 100, 50)), "behind the camera: nothing");
    // a big cube that contains the camera: clipped by the near plane, no panic, no stray colour outside
    let straddle = render(&snap(
        vec![cube(
            100.0,
            50.0,
            -F,
            300.0,
            "#ff0000",
            json!("#000000"),
            2.0,
        )],
        flat_light(0.3),
    ));
    let _ = px(&straddle, 100, 50);
}

#[test]
fn z_order_with_2d_shapes_and_opacity() {
    let flat = |z: i64| {
        json!({"z": z, "visible": true, "edges": [], "vertices": [], "opacity": 1.0,
        "faces": [{"fill": "#00ff00", "segments": [[[90.0, 40.0], [110.0, 40.0]], [[110.0, 40.0], [110.0, 60.0]], [[110.0, 60.0], [90.0, 60.0]], [[90.0, 60.0], [90.0, 40.0]]]}]})
    };
    let mesh = cube(100.0, 50.0, 0.0, 30.0, "#ff0000", Value::Null, 0.0);
    let over = render(&snap(vec![mesh.clone(), flat(5)], flat_light(0.3)));
    assert_eq!(
        px(&over, 100, 50),
        (0, 255, 0),
        "a 2D shape with a higher z is drawn over the mesh"
    );
    let under = render(&snap(vec![flat(-5), mesh.clone()], flat_light(0.3)));
    assert_eq!(
        px(&under, 100, 50),
        (255, 0, 0),
        "and under it with a lower z"
    );
    let mut half = mesh;
    half["opacity"] = json!(0.5);
    let pm = render(&snap(vec![half], flat_light(0.3)));
    let (r, g, _) = px(&pm, 100, 50);
    assert!(
        r > 240 && g > 100 && g < 160,
        "half red over white: {:?}",
        px(&pm, 100, 50)
    );
}

#[test]
fn the_same_snapshot_renders_the_same_pixels_and_a_scene_without_meshes_is_unchanged() {
    let s = snap(
        vec![cube(
            100.0,
            50.0,
            0.0,
            20.0,
            "#ff8000",
            json!("#000000"),
            2.0,
        )],
        json!({"dir": [-0.4, 0.7, 0.6], "ambient": 0.3, "bands": 3, "crease": 40.0}),
    );
    assert_eq!(render(&s).data(), render(&s).data());
    let mut none = s;
    none["shapes"] = json!([]);
    assert!(render(&none)
        .data()
        .chunks(4)
        .all(|p| p == [255, 255, 255, 255]));
}

#[test]
fn smooth_shading_varies_inside_a_facet_and_flat_shading_does_not() {
    // two quads bent by 20 degrees along the middle, both facing the camera
    let mut m = cube(100.0, 50.0, 0.0, 20.0, "#ffffff", Value::Null, 0.0);
    m["mesh"]["verts"] = json!([
        [80.0, 30.0, -10.0],
        [100.0, 30.0, 0.0],
        [120.0, 30.0, -10.0],
        [80.0, 70.0, -10.0],
        [100.0, 70.0, 0.0],
        [120.0, 70.0, -10.0]
    ]);
    m["mesh"]["faces"] = json!([[0, 1, 4, 3], [1, 2, 5, 4]]);
    m["mesh"]["double_sided"] = json!(true);
    let lit = |smooth: f64| {
        let mut s = snap(
            vec![m.clone()],
            json!({"dir": [-1.0, 0.0, 0.5], "ambient": 0.1, "bands": 0, "crease": 40.0, "smooth": smooth}),
        );
        s["light3"]["smooth"] = json!(smooth);
        let pm = render(&s);
        (px(&pm, 84, 50).0, px(&pm, 96, 50).0)
    };
    let (a, b) = lit(0.0);
    assert!(a.abs_diff(b) <= 1, "flat: one shade per face {a} {b}");
    let (a, b) = lit(60.0);
    assert!(
        a.abs_diff(b) > 8,
        "smooth: the shade changes across the facet {a} {b}"
    );
}
