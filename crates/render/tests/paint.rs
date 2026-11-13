//! Paint layers in the snapshot are replayed with libmypaint: strokes, progress, the layer transform and the camera.
use animscript_render::render_with;
use serde_json::{json, Value};

fn layer(strokes: Value, extra: Value) -> Value {
    let mut sh = json!({
        "visible": true, "opacity": 1.0, "z": 0,
        "transform": {"position": [0.0, 0.0], "scale": [1.0, 1.0], "rotation": 0.0, "pivot": [100.0, 50.0]},
        "vertices": [], "edges": [], "faces": [],
        "paint": {"size": [200, 100], "resolution": 1.0,
            "brushes": [{"source": "rounded", "myb": null, "set": [["radius_logarithmic", 2.3], ["opaque", 1.0], ["hardness", 0.9]], "color": [1.0, 0.0, 0.0]}],
            "strokes": strokes},
    });
    for (k, v) in extra.as_object().into_iter().flatten() {
        sh[k] = v.clone();
    }
    sh
}
fn snap(shape: Value) -> Value {
    json!({"canvas": [200, 100], "background": "#ffffff", "camera": {"position": [100.0, 50.0], "zoom": 1.0, "rotation": 0.0}, "shapes": [shape]})
}
fn line(progress: f64) -> Value {
    json!([{"brush": 0, "progress": progress,
        "points": [[20.0, 50.0, 0.9, 0.01], [60.0, 50.0, 0.9, 0.05], [100.0, 50.0, 0.9, 0.05], [140.0, 50.0, 0.9, 0.05], [180.0, 50.0, 0.9, 0.05]]}])
}
fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}
fn painted(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> bool {
    let (r, g, b) = px(pm, x, y);
    r > 200 && g < 150 && b < 150
}

#[test]
fn a_stroke_is_painted_in_its_colour() {
    let pm = render_with(&snap(layer(line(1.0), json!({}))), 1.0, false, false).unwrap();
    assert!(painted(&pm, 100, 50), "the middle of the stroke is red");
    assert!(painted(&pm, 60, 50));
    assert_eq!(
        px(&pm, 100, 10),
        (255, 255, 255),
        "elsewhere stays the background"
    );
}

#[test]
fn progress_paints_only_the_first_part_of_the_path() {
    let pm = render_with(&snap(layer(line(0.5), json!({}))), 1.0, false, false).unwrap();
    assert!(painted(&pm, 50, 50), "the start is there");
    assert_eq!(
        px(&pm, 150, 50),
        (255, 255, 255),
        "the end is not painted yet"
    );
    let none = render_with(&snap(layer(line(0.0), json!({}))), 1.0, false, false).unwrap();
    assert_eq!(px(&none, 100, 50), (255, 255, 255));
}

#[test]
fn the_same_snapshot_renders_the_same_picture() {
    let s = snap(layer(line(0.8), json!({})));
    let (a, b) = (
        render_with(&s, 1.0, false, false).unwrap(),
        render_with(&s, 1.0, false, false).unwrap(),
    );
    assert_eq!(a.data(), b.data());
}

#[test]
fn the_layers_transform_and_opacity_apply() {
    let moved = layer(
        line(1.0),
        json!({"transform": {"position": [0.0, 30.0], "scale": [1.0, 1.0], "rotation": 0.0, "pivot": [100.0, 50.0]}}),
    );
    let pm = render_with(&snap(moved), 1.0, false, false).unwrap();
    assert!(painted(&pm, 100, 80), "moved 30 px down");
    assert_eq!(px(&pm, 100, 50), (255, 255, 255));
    let faint = layer(line(1.0), json!({"opacity": 0.5}));
    let pm = render_with(&snap(faint), 1.0, false, false).unwrap();
    let (r, g, _) = px(&pm, 100, 50);
    assert!(
        r > 200 && g > 90 && g < 200,
        "half faded to the background: {g}"
    );
}

#[test]
fn the_camera_and_output_scale_apply_to_paint() {
    let base = render_with(&snap(layer(line(1.0), json!({}))), 1.0, false, false).unwrap();
    assert_eq!(
        px(&base, 100, 66),
        (255, 255, 255),
        "a 10 px brush does not reach 16 px away"
    );
    let mut s = snap(layer(line(1.0), json!({})));
    s["camera"] = json!({"position": [100.0, 50.0], "zoom": 2.0, "rotation": 0.0});
    let pm = render_with(&s, 1.0, false, false).unwrap();
    assert!(
        painted(&pm, 100, 50 + 16),
        "at zoom 2 the same stroke is twice as thick"
    );
    let big = render_with(&snap(layer(line(1.0), json!({}))), 2.0, false, false).unwrap();
    assert_eq!(big.width(), 400);
    assert!(painted(&big, 200, 100));
}

#[test]
fn an_unknown_brush_is_skipped_not_a_crash() {
    let mut l = layer(line(1.0), json!({}));
    l["paint"]["brushes"][0]["source"] = json!("no_such_brush");
    let pm = render_with(&snap(l), 1.0, false, false).unwrap();
    assert_eq!(px(&pm, 100, 50), (255, 255, 255));
}

// ---- symmetry, blend modes and tilt in the picture -----------------------------------------

fn solid_brush(color: [f64; 3]) -> Value {
    json!({"source": "rounded", "myb": null, "set": [["radius_logarithmic", 2.3], ["opaque", 1.0], ["hardness", 0.95]], "color": color})
}
fn pts(a: [f64; 2], b: [f64; 2]) -> Value {
    json!([
        [a[0], a[1], 1.0, 0.01],
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, 1.0, 0.03],
        [b[0], b[1], 1.0, 0.03]
    ])
}
fn two_brush_layer(strokes: Value) -> Value {
    let mut l = layer(strokes, json!({}));
    l["paint"]["brushes"] = json!([solid_brush([1.0, 0.0, 0.0]), solid_brush([0.0, 0.0, 1.0])]);
    l
}

#[test]
fn a_stroke_with_symmetry_is_repeated_in_the_picture() {
    let strokes = json!([{"brush": 0, "progress": 1.0, "points": pts([30.0, 25.0], [50.0, 25.0]),
        "symmetry": {"kind": "both", "lines": 2, "center": [100.0, 50.0], "angle": 0.0}}]);
    let pm = render_with(&snap(two_brush_layer(strokes)), 1.0, false, false).unwrap();
    for (x, y) in [(40, 25), (160, 25), (40, 75), (160, 75)] {
        assert!(painted(&pm, x, y), "copy at ({x}, {y})");
    }
    assert_eq!(px(&pm, 100, 50), (255, 255, 255));
}

#[test]
fn symmetry_follows_the_layers_resolution() {
    let mut l = two_brush_layer(
        json!([{"brush": 0, "progress": 1.0, "points": pts([30.0, 25.0], [50.0, 25.0]),
        "symmetry": {"kind": "vertical", "lines": 2, "center": [100.0, 50.0], "angle": 0.0}}]),
    );
    l["paint"]["resolution"] = json!(2.0);
    let pm = render_with(&snap(l), 1.0, false, false).unwrap();
    assert!(
        painted(&pm, 40, 25) && painted(&pm, 160, 25),
        "mirrored about the canvas centre line at resolution 2"
    );
}

#[test]
fn a_multiply_stroke_darkens_what_is_under_it_and_a_layer_blend_applies_to_the_scene() {
    let strokes = json!([
        {"brush": 0, "progress": 1.0, "points": pts([40.0, 50.0], [120.0, 50.0])},
        {"brush": 1, "progress": 1.0, "points": pts([80.0, 30.0], [80.0, 70.0]), "blend": "multiply", "strength": 1.0},
    ]);
    let pm = render_with(&snap(two_brush_layer(strokes)), 1.0, false, false).unwrap();
    // red x blue = black where they cross; blue alone elsewhere
    let (r, g, b) = px(&pm, 80, 50);
    assert!(r < 40 && g < 40 && b < 40, "multiplied: {r} {g} {b}");
    let (r, _, b) = px(&pm, 80, 70);
    assert!(
        b > 150 && r < 80,
        "the blue part over the background: {r} {b}"
    );
    // a layer blend of multiply with a dark background keeps it dark; with `add` it lights the picture
    let mut l = two_brush_layer(
        json!([{"brush": 0, "progress": 1.0, "points": pts([40.0, 50.0], [120.0, 50.0])}]),
    );
    l["paint"]["blend"] = json!("add");
    let mut s = snap(l);
    s["background"] = json!("#003300");
    let pm = render_with(&s, 1.0, false, false).unwrap();
    let (r, g, _) = px(&pm, 80, 50);
    assert!(
        r > 200 && g > 40,
        "red added to the green background: {r} {g}"
    );
}

#[test]
fn tilt_reaches_the_brush() {
    let myb = r#"{"version": 3, "settings": {"opaque": {"base_value": 1.0, "inputs": {}}, "hardness": {"base_value": 0.95, "inputs": {}},
        "radius_logarithmic": {"base_value": 1.8, "inputs": {}}, "elliptical_dab_ratio": {"base_value": 4.0, "inputs": {}},
        "elliptical_dab_angle": {"base_value": 0.0, "inputs": {"tilt_ascension": [[-180.0, 0.0], [180.0, 180.0]]}}}}"#;
    let one = |xt: f64, yt: f64| {
        let mut l = layer(
            json!([{"brush": 0, "progress": 1.0, "points": [[100.0, 50.0, 0.9, 0.01, xt, yt], [102.0, 50.0, 0.9, 0.01, xt, yt], [104.0, 50.0, 0.9, 0.01, xt, yt]]}]),
            json!({}),
        );
        l["paint"]["brushes"] =
            json!([{"source": "mine", "myb": myb, "set": [], "color": [1.0, 0.0, 0.0]}]);
        let pm = render_with(&snap(l), 1.0, false, false).unwrap();
        let (mut w, mut h) = ((200u32, 0u32), (100u32, 0u32));
        for y in 0..100 {
            for x in 0..200 {
                if painted(&pm, x, y) {
                    w = (w.0.min(x), w.1.max(x));
                    h = (h.0.min(y), h.1.max(y));
                }
            }
        }
        (w.1 - w.0, h.1 - h.0)
    };
    let (a, b) = (one(0.9, 0.0), one(0.0, 0.9));
    assert_ne!(a, b, "{a:?} vs {b:?}");
}

// ---- the paint cache: same pictures, only faster -----------------------------------------------------------------

fn bytes(pm: &tiny_skia::Pixmap) -> Vec<u8> {
    pm.data().to_vec()
}

#[test]
fn a_cached_layer_renders_the_same_pixels() {
    let s = snap(layer(line(1.0), json!({})));
    let first = render_with(&s, 1.0, false, false).unwrap();
    let second = render_with(&s, 1.0, false, false).unwrap(); // served from the cache
    assert_eq!(bytes(&first), bytes(&second));
}

#[test]
fn a_moved_layer_reuses_the_picture_and_lands_in_the_new_place() {
    let base = render_with(&snap(layer(line(1.0), json!({}))), 1.0, false, false).unwrap();
    let moved = {
        let mut sh = layer(line(1.0), json!({}));
        sh["transform"]["position"] = json!([0.0, 30.0]);
        render_with(&snap(sh), 1.0, false, false).unwrap()
    };
    assert!(painted(&base, 100, 50) && !painted(&base, 100, 80));
    assert!(
        painted(&moved, 100, 80) && !painted(&moved, 100, 50),
        "the same stroke, 30 px lower"
    );
}

#[test]
fn changed_strokes_are_not_served_from_a_stale_cache() {
    let full = render_with(&snap(layer(line(1.0), json!({}))), 1.0, false, false).unwrap();
    let half = render_with(&snap(layer(line(0.5), json!({}))), 1.0, false, false).unwrap();
    assert!(painted(&full, 170, 50));
    assert!(
        !painted(&half, 170, 50),
        "progress 0.5 has not reached x = 170"
    );
    let again = render_with(&snap(layer(line(1.0), json!({}))), 1.0, false, false).unwrap();
    assert_eq!(bytes(&full), bytes(&again));
}
