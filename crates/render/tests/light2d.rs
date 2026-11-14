//! 2D shapes marked `lit`: shaded by the same lights, shadowed by meshes and by other 2D shapes.
use animscript_render::render_with;
use serde_json::{json, Value};

/// A closed outline as one straight segment per side, the way snapshots carry them.
fn segs(pts: &[Value]) -> Value {
    let n = pts.len();
    Value::Array((0..n).map(|i| json!([pts[i], pts[(i + 1) % n]])).collect())
}

fn circle(id: &str, cx: f64, cy: f64, r: f64, fill: &str, lit: Value) -> Value {
    let n = 48;
    let pts: Vec<Value> = (0..n)
        .map(|k| {
            let a = k as f64 / n as f64 * std::f64::consts::TAU;
            json!([cx + r * a.cos(), cy + r * a.sin()])
        })
        .collect();
    let mut sh = json!({"id": id, "z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [],
        "faces": [{"id": 0, "fill": fill, "segments": segs(&pts)}]});
    if !lit.is_null() {
        sh["lit"] = lit;
    }
    sh
}
fn rect(id: &str, x0: f64, y0: f64, x1: f64, y1: f64, fill: &str, lit: Value) -> Value {
    let mut sh = json!({"id": id, "z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [],
        "faces": [{"id": 0, "fill": fill, "segments": segs(&[json!([x0, y0]), json!([x1, y0]), json!([x1, y1]), json!([x0, y1])])}]});
    if !lit.is_null() {
        sh["lit"] = lit;
    }
    sh
}
fn lit(bevel: f64, depth: f64, occluder: bool, emissive: f64) -> Value {
    json!({"bevel": bevel, "surface": [0.7, 0.0, emissive], "occluder": occluder, "depth": depth})
}
fn light(kind: &str, pos: [f64; 3], dir: [f64; 3], intensity: f64) -> Value {
    json!({"id": "l", "kind": kind, "pos": pos, "dir": dir, "color": [1.0, 1.0, 1.0], "intensity": intensity,
        "reach": 200.0, "size": 0.0, "angle": 0.6, "softness": 0.2, "shadows": true})
}
fn snap(shapes: Vec<Value>, lights: Vec<Value>, bloom: f64) -> Value {
    let mut l3 = json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 3, "crease": 40.0});
    if bloom > 0.0 {
        l3["bloom"] = json!(bloom);
    }
    let mut s = json!({"canvas": [200, 200], "background": "#101010", "camera": {"position": [100.0, 100.0], "zoom": 1.0, "rotation": 0.0},
        "light3": l3, "shapes": shapes});
    if !lights.is_empty() {
        s["lights"] = json!(lights);
    }
    s
}
fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (i32, i32, i32) {
    let p = pm.pixel(x, y).unwrap();
    (p.red() as i32, p.green() as i32, p.blue() as i32)
}
fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}

#[test]
fn an_unlit_shape_is_untouched_by_lights() {
    let flat = render(&snap(
        vec![circle("c", 100.0, 100.0, 50.0, "#c08040", Value::Null)],
        vec![],
        0.0,
    ));
    let with = render(&snap(
        vec![circle("c", 100.0, 100.0, 50.0, "#c08040", Value::Null)],
        vec![light("sun", [0.0; 3], [0.0, 0.0, 1.0], 2.0)],
        0.0,
    ));
    assert_eq!(flat.data(), with.data());
}

#[test]
fn a_bevelled_disc_is_brighter_on_the_side_facing_the_light() {
    // a point light to the left and in front: the left rim faces it
    let s = snap(
        vec![circle(
            "c",
            100.0,
            100.0,
            60.0,
            "#c0c0c0",
            lit(40.0, 0.0, false, 0.0),
        )],
        vec![light(
            "point",
            [-100.0, 100.0, -100.0],
            [1.0, 0.0, 1.0],
            3.0,
        )],
        0.0,
    );
    let pm = render(&s);
    let (left, right) = (px(&pm, 55, 100), px(&pm, 145, 100));
    assert!(left.0 > right.0 + 30, "left {left:?} right {right:?}");
    // the middle of the disc is flat and lit
    assert!(px(&pm, 100, 100).0 > 20);
}

#[test]
fn a_flat_lit_shape_takes_the_colour_of_the_light() {
    let mut l = light("sun", [0.0; 3], [0.0, 0.0, 1.0], 2.0);
    l["color"] = json!([1.0, 0.1, 0.0]);
    l["intensity"] = json!(0.5);
    let pm = render(&snap(
        vec![rect(
            "r",
            20.0,
            20.0,
            180.0,
            180.0,
            "#ffffff",
            lit(0.0, 0.0, false, 0.0),
        )],
        vec![l],
        0.0,
    ));
    let c = px(&pm, 100, 100);
    assert!(c.0 > 3 * c.1.max(1) && c.2 <= c.1, "{c:?}");
}

#[test]
fn a_mesh_in_front_shadows_a_lit_2d_shape() {
    let cube_front = json!({"id": "m", "z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": [[80.0, 80.0, -100.0], [120.0, 80.0, -100.0], [120.0, 120.0, -100.0], [80.0, 120.0, -100.0]],
        "faces": [[3, 2, 1, 0]], "double_sided": true, "fill": "#303030", "outline": null, "width": 0.0}});
    let mut s = snap(
        vec![
            rect(
                "r",
                0.0,
                0.0,
                200.0,
                200.0,
                "#c0c0c0",
                lit(0.0, 0.0, false, 0.0),
            ),
            cube_front,
        ],
        vec![light("point", [100.0, 100.0, -300.0], [0.0, 0.0, 1.0], 3.0)],
        0.0,
    );
    s["camera3"] = json!({"eye": [100.0, 100.0, -1000.0], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": 1000.0, "cx": 100.0, "cy": 100.0});
    let pm = render(&s);
    // the card (z = -100) between the light (z = -300) and the sheet (z = 0) throws a shadow 1.5 times its size: 60 wide
    // around the centre, so (100 + 25, 100) is in shadow beyond the card's own screen rectangle (80..120 at this zoom)
    let shadowed = px(&pm, 126, 100).0;
    let outside = px(&pm, 20, 20).0;
    assert!(
        shadowed < outside / 2 || shadowed < 30,
        "{shadowed} vs {outside}"
    );
}

#[test]
fn an_occluder_shadows_a_shape_behind_it() {
    // an occluder disc at depth -50, the sheet behind at 0, a light in front on the axis
    let s = snap(
        vec![
            rect(
                "sheet",
                0.0,
                0.0,
                200.0,
                200.0,
                "#c0c0c0",
                lit(0.0, 0.0, false, 0.0),
            ),
            circle(
                "blocker",
                100.0,
                100.0,
                20.0,
                "#202020",
                lit(0.0, -50.0, true, 0.0),
            ),
        ],
        vec![light("point", [100.0, 100.0, -150.0], [0.0, 0.0, 1.0], 3.0)],
        0.0,
    );
    let pm = render(&s);
    // the disc's shadow on the sheet is 1.33 times as big (radius ~27): look between the disc rim and the shadow rim
    let in_shadow_ring = px(&pm, 100 + 24, 100).0;
    let lit_far = px(&pm, 20, 20).0;
    assert!(
        in_shadow_ring < lit_far / 2,
        "{in_shadow_ring} vs {lit_far}"
    );
}

#[test]
fn bloom_spreads_light_from_an_emissive_2d_shape() {
    let shapes = |l: Value| vec![rect("glow", 90.0, 90.0, 110.0, 110.0, "#ffffff", l)];
    let lights = vec![
        json!({"id": "a", "kind": "ambient", "pos": [0.0, 0.0, 0.0], "dir": [0.0, 1.0, 0.0], "color": [1.0, 1.0, 1.0],
        "intensity": 0.05, "reach": 200.0, "size": 0.0, "angle": 0.7, "softness": 0.2, "shadows": true}),
    ];
    let plain = render(&snap(
        shapes(lit(0.0, 0.0, false, 6.0)),
        lights.clone(),
        0.0,
    ));
    let glow = render(&snap(shapes(lit(0.0, 0.0, false, 6.0)), lights, 1.0));
    assert!(px(&glow, 100, 84).0 > px(&plain, 100, 84).0 + 10);
}

#[test]
fn lit_2d_rendering_is_deterministic() {
    let s = snap(
        vec![circle(
            "c",
            100.0,
            100.0,
            60.0,
            "#c0c0c0",
            lit(30.0, 0.0, true, 0.0),
        )],
        vec![light("point", [20.0, 20.0, -100.0], [1.0, 1.0, 1.0], 3.0)],
        0.3,
    );
    assert_eq!(render(&s).data(), render(&s).data());
}

#[test]
fn a_2d_occluder_shadows_a_mesh_behind_it() {
    let wall = json!({"id": "w", "z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": [[-300.0, -300.0, 100.0], [500.0, -300.0, 100.0], [500.0, 500.0, 100.0], [-300.0, 500.0, 100.0]],
        "faces": [[3, 2, 1, 0]], "double_sided": true, "fill": "#c0c0c0", "outline": null, "width": 0.0, "surface": [1.0, 0.0, 0.0]}});
    let mut s = snap(
        vec![
            circle(
                "blocker",
                100.0,
                100.0,
                20.0,
                "#202020",
                lit(0.0, -50.0, true, 0.0),
            ),
            wall,
        ],
        vec![light("point", [100.0, 100.0, -150.0], [0.0, 0.0, 1.0], 3.0)],
        0.0,
    );
    s["camera3"] = json!({"eye": [100.0, 100.0, -1000.0], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": 1000.0, "cx": 100.0, "cy": 100.0});
    let pm = render(&s);
    // the disc (radius 20 at 100 from the light) throws a shadow of radius 50 on the wall 250 away; on screen the wall
    // is scaled by 1000 / 1100, so the shadow reaches about 45 px: look at 35 px from the centre
    let ring = px(&pm, 135, 100).0;
    let far = px(&pm, 10, 10).0;
    assert!(ring < far / 2, "{ring} vs {far}");
}
