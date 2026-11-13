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
