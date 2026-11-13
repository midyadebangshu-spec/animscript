//! A field in the snapshot is drawn as its colour-mapped image inside its rectangle, in order with the shapes by z.
use animscript_render::render_with;
use serde_json::{json, Value};

/// A base64 of RGBA pixels (the same encoding the language writes).
fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn snap(fields: Value, shapes: Value) -> Value {
    json!({"canvas": [200, 100], "background": "#ffffff", "camera": {"position": [100.0, 50.0], "zoom": 1.0, "rotation": 0.0}, "shapes": shapes, "fields": fields})
}
/// A 4 x 1 field: red, red, blue, blue.
fn field(z: i64, opacity: f64) -> Value {
    let px: Vec<u8> = [
        [255, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 0, 255, 255],
        [0, 0, 255, 255],
    ]
    .concat();
    json!({"name": "f", "nx": 4, "ny": 1, "iw": 4, "ih": 1, "rect": [20.0, 10.0, 160.0, 80.0], "z": z, "opacity": opacity, "blend": "normal", "rgba": b64(&px), "checksum": "0"})
}
fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}
fn square(z: i64) -> Value {
    json!({"z": z, "visible": true, "edges": [], "vertices": [],
        "faces": [{"fill": "#00ff00", "segments": [
            [[70.0, 30.0], [130.0, 30.0]], [[130.0, 30.0], [130.0, 70.0]],
            [[130.0, 70.0], [70.0, 70.0]], [[70.0, 70.0], [70.0, 30.0]]]}]})
}

#[test]
fn a_field_fills_its_rectangle_with_its_image() {
    let pm = render_with(&snap(json!([field(0, 1.0)]), json!([])), 1.0, false, false).unwrap();
    let (r, _, b) = px(&pm, 30, 50);
    assert!(r > 200 && b < 60, "left part is red: {:?}", px(&pm, 30, 50));
    let (r, _, b) = px(&pm, 170, 50);
    assert!(b > 200 && r < 60, "right part is blue");
    assert_eq!(
        px(&pm, 5, 50),
        (255, 255, 255),
        "outside the rectangle is the background"
    );
    assert_eq!(px(&pm, 100, 95), (255, 255, 255));
    // between the two halves the colours blend smoothly
    let (r, _, b) = px(&pm, 100, 50);
    assert!(
        r > 60 && r < 200 && b > 60 && b < 200,
        "{:?}",
        px(&pm, 100, 50)
    );
}

#[test]
fn opacity_and_z_order_apply() {
    let faint = render_with(&snap(json!([field(0, 0.5)]), json!([])), 1.0, false, false).unwrap();
    let (r, g, _) = px(&faint, 30, 50);
    assert!(
        r > 200 && g > 100 && g < 160,
        "half red over white: {:?}",
        px(&faint, 30, 50)
    );
    // a shape with a higher z is drawn over the field; with a lower z, under it
    let over = render_with(
        &snap(json!([field(0, 1.0)]), json!([square(5)])),
        1.0,
        false,
        false,
    )
    .unwrap();
    assert_eq!(px(&over, 100, 50), (0, 255, 0));
    let under = render_with(
        &snap(json!([field(5, 1.0)]), json!([square(0)])),
        1.0,
        false,
        false,
    )
    .unwrap();
    assert_ne!(px(&under, 100, 50), (0, 255, 0));
}

#[test]
fn the_camera_applies_and_a_field_without_an_image_draws_nothing() {
    let mut s = snap(json!([field(0, 1.0)]), json!([]));
    s["camera"]["zoom"] = json!(2.0);
    let pm = render_with(&s, 1.0, false, false).unwrap();
    // zoom 2 about the centre: the red/blue boundary stays in the middle, the field now overflows the canvas
    assert!(px(&pm, 10, 50).0 > 200 && px(&pm, 190, 50).2 > 200);
    let mut bare = field(0, 1.0);
    bare.as_object_mut().unwrap().remove("rgba");
    let pm = render_with(&snap(json!([bare]), json!([])), 1.0, false, false).unwrap();
    assert_eq!(px(&pm, 30, 50), (255, 255, 255));
}
