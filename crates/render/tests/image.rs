//! Textured faces are drawn triangle by triangle from the snapshot (a pure function of it plus the
//! PNG file). Written first.
use animscript_render::render_assets;
use serde_json::{json, Value};
use std::path::Path;

const BASE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");

fn quad(p: [[f64; 2]; 4], src: &str, opacity: f64) -> Value {
    // p = top-left, top-right, bottom-right, bottom-left corners (uv 0,0 / 1,0 / 1,1 / 0,1)
    let v = |i: usize, u: f64, w: f64| json!([p[i][0], p[i][1], u, w]);
    json!({"canvas": [300, 300], "background": "#ffffff",
           "shapes": [{"id": "s", "z": 0, "visible": true, "fill": null, "stroke": null, "stroke_width": 0.0, "opacity": opacity,
                       "vertices": [], "edges": [], "faces": [],
                       "image": {"src": src, "w": 4, "h": 4, "hash": "0", "tris": [[v(0, 0.0, 0.0), v(1, 1.0, 0.0), v(2, 1.0, 1.0)], [v(0, 0.0, 0.0), v(2, 1.0, 1.0), v(3, 0.0, 1.0)]]}}]})
}
fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}

#[test]
fn the_picture_lands_on_the_quad() {
    let s = quad(
        [
            [100.0, 100.0],
            [200.0, 100.0],
            [200.0, 200.0],
            [100.0, 200.0],
        ],
        "data/checker.png",
        1.0,
    );
    let pm = render_assets(&s, 1.0, false, false, Path::new(BASE)).unwrap();
    assert_eq!(px(&pm, 125, 125), (255, 0, 0)); // the 4 x 4 picture: red, green / blue, white quarters
    assert_eq!(px(&pm, 175, 125), (0, 255, 0));
    assert_eq!(px(&pm, 125, 175), (0, 0, 255));
    assert_eq!(px(&pm, 175, 175), (255, 255, 255));
    assert_eq!(px(&pm, 50, 50), (255, 255, 255)); // outside: the background
}

#[test]
fn a_skewed_quad_samples_the_same_picture_positions() {
    let s = quad(
        [
            [100.0, 100.0],
            [200.0, 120.0],
            [180.0, 220.0],
            [80.0, 200.0],
        ],
        "data/checker.png",
        1.0,
    );
    let pm = render_assets(&s, 1.0, false, false, Path::new(BASE)).unwrap();
    assert_eq!(px(&pm, 120, 130), (255, 0, 0)); // uv (0.25, 0.25)
    assert_eq!(px(&pm, 170, 140), (0, 255, 0)); // uv (0.75, 0.25)
    assert_eq!(px(&pm, 110, 180), (0, 0, 255)); // uv (0.25, 0.75)
}

#[test]
fn opacity_blends_with_what_is_behind() {
    let s = quad(
        [
            [100.0, 100.0],
            [200.0, 100.0],
            [200.0, 200.0],
            [100.0, 200.0],
        ],
        "data/checker.png",
        0.5,
    );
    let pm = render_assets(&s, 1.0, false, false, Path::new(BASE)).unwrap();
    let (r, g, b) = px(&pm, 125, 125);
    assert!(
        r > 240 && g > 100 && g < 160 && b > 100 && b < 160,
        "half-transparent red on white: {r} {g} {b}"
    );
}

#[test]
fn scaling_the_canvas_scales_the_picture() {
    let s = quad(
        [
            [100.0, 100.0],
            [200.0, 100.0],
            [200.0, 200.0],
            [100.0, 200.0],
        ],
        "data/checker.png",
        1.0,
    );
    let pm = render_assets(&s, 2.0, false, false, Path::new(BASE)).unwrap();
    assert_eq!(px(&pm, 250, 250), (255, 0, 0));
    assert_eq!(px(&pm, 350, 350), (255, 255, 255));
}

#[test]
fn a_missing_picture_is_drawn_magenta_not_a_crash_and_rendering_is_repeatable() {
    let s = quad(
        [
            [100.0, 100.0],
            [200.0, 100.0],
            [200.0, 200.0],
            [100.0, 200.0],
        ],
        "data/nope.png",
        1.0,
    );
    let a = render_assets(&s, 1.0, false, false, Path::new(BASE)).unwrap();
    let (r, g, b) = px(&a, 150, 150);
    assert!(
        r > 200 && g < 80 && b > 200,
        "a magenta placeholder: {r} {g} {b}"
    );
    let good = quad(
        [
            [100.0, 100.0],
            [200.0, 100.0],
            [200.0, 200.0],
            [100.0, 200.0],
        ],
        "data/checker.png",
        1.0,
    );
    assert_eq!(
        render_assets(&good, 1.0, false, false, Path::new(BASE))
            .unwrap()
            .data(),
        render_assets(&good, 1.0, false, false, Path::new(BASE))
            .unwrap()
            .data()
    );
}
