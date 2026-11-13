//! The renderer applies the snapshot's camera (centre, zoom, rotation) and blends transitions.
use animscript_render::render_with;
use serde_json::{json, Value};

/// A 100 x 100 red square at (100..200, 100..200) on a black 400 x 300 canvas.
fn snap(camera: Value, extra: Value) -> Value {
    let mut s = json!({
        "canvas": [400, 300], "background": "#000000",
        "camera": camera,
        "shapes": [{"visible": true, "faces": [{"fill": "#ff0000", "segments": [
            [[100.0, 100.0], [200.0, 100.0]], [[200.0, 100.0], [200.0, 200.0]],
            [[200.0, 200.0], [100.0, 200.0]], [[100.0, 200.0], [100.0, 100.0]]]}],
            "edges": [], "vertices": []}],
    });
    for (k, v) in extra.as_object().into_iter().flatten() {
        s[k] = v.clone();
    }
    s
}
fn red(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> u8 {
    pm.pixel(x, y).unwrap().red()
}
fn cam(x: f64, y: f64, zoom: f64, rot: f64) -> Value {
    json!({"position": [x, y], "zoom": zoom, "rotation": rot})
}

#[test]
fn the_default_camera_changes_nothing() {
    let pm = render_with(
        &snap(cam(200.0, 150.0, 1.0, 0.0), json!({})),
        1.0,
        false,
        false,
    )
    .unwrap();
    assert!(red(&pm, 150, 150) > 200);
    assert_eq!(red(&pm, 50, 50), 0);
}

#[test]
fn zoom_scales_about_the_view_centre() {
    // centred on the square, zoom 2: it fills (100..300) x (50..250) of the picture, around the middle
    let pm = render_with(
        &snap(cam(150.0, 150.0, 2.0, 0.0), json!({})),
        1.0,
        false,
        false,
    )
    .unwrap();
    assert!(red(&pm, 200, 150) > 200, "the middle is the square");
    assert!(
        red(&pm, 110, 150) > 200,
        "and it reaches 100 px left of the middle, twice as far as before"
    );
    assert_eq!(red(&pm, 90, 150), 0, "but not further");
    assert_eq!(red(&pm, 310, 150), 0);
}

#[test]
fn the_view_centre_follows_the_camera_position() {
    // looking at (250, 150) with zoom 1: the square (100..200) moves 50 px to the left of where it was
    let pm = render_with(
        &snap(cam(250.0, 150.0, 1.0, 0.0), json!({})),
        1.0,
        false,
        false,
    )
    .unwrap();
    assert!(red(&pm, 60, 150) > 200);
    assert_eq!(red(&pm, 160, 150), 0);
}

#[test]
fn a_rotated_camera_turns_the_world_the_other_way() {
    // a small red square 100 px to the right of the view centre (150, 150); the camera turns a quarter clockwise,
    // so the world turns a quarter anticlockwise about the centre of the picture (200, 150): the square goes
    // from right of centre to above it
    let mut s = snap(cam(150.0, 150.0, 1.0, 0.25), json!({}));
    s["shapes"][0]["faces"][0]["segments"] = json!([
        [[240.0, 140.0], [260.0, 140.0]],
        [[260.0, 140.0], [260.0, 160.0]],
        [[260.0, 160.0], [240.0, 160.0]],
        [[240.0, 160.0], [240.0, 140.0]]
    ]);
    let pm = render_with(&s, 1.0, false, false).unwrap();
    assert!(red(&pm, 200, 50) > 200, "the square is above the centre");
    assert_eq!(red(&pm, 300, 150), 0, "and no longer to the right of it");
}

#[test]
fn output_scale_and_camera_compose() {
    let pm = render_with(
        &snap(cam(150.0, 150.0, 2.0, 0.0), json!({})),
        2.0,
        false,
        false,
    )
    .unwrap();
    assert_eq!((pm.width(), pm.height()), (800, 600));
    assert!(red(&pm, 400, 300) > 200);
    assert_eq!(
        red(&pm, 180, 300),
        0,
        "100 px left of the middle at zoom 2 and scale 2 is 400 px: not 180"
    );
}

#[test]
fn a_dissolve_mixes_the_two_views() {
    let tr = json!({"kind": "dissolve", "t": 0.5, "from": cam(200.0, 150.0, 1.0, 0.0)});
    // the new view moved the square 100 px right of the old one: at (150, 150) the old view shows red, the new none
    let s = snap(cam(100.0, 150.0, 1.0, 0.0), json!({"transition": tr}));
    let pm = render_with(&s, 1.0, false, false).unwrap();
    let mid = red(&pm, 150, 150) as i32;
    assert!((mid - 127).abs() < 3, "half red: {mid}");
    let tr = json!({"kind": "dissolve", "t": 0.0, "from": cam(200.0, 150.0, 1.0, 0.0)});
    let pm = render_with(
        &snap(cam(100.0, 150.0, 1.0, 0.0), json!({"transition": tr})),
        1.0,
        false,
        false,
    )
    .unwrap();
    assert!(red(&pm, 150, 150) > 250, "t = 0 is the old view");
}

#[test]
fn fade_through_goes_to_the_colour_and_comes_out_of_it() {
    let base = |t: f64| {
        let tr = json!({"kind": "fade_through", "t": t, "color": "#ffffff", "from": cam(200.0, 150.0, 1.0, 0.0)});
        render_with(
            &snap(cam(100.0, 150.0, 1.0, 0.0), json!({"transition": tr})),
            1.0,
            false,
            false,
        )
        .unwrap()
    };
    let half = base(0.5);
    // at the midpoint the whole picture is the colour
    assert!(half.pixel(5, 5).unwrap().green() > 250 && half.pixel(150, 150).unwrap().blue() > 250);
    let quarter = base(0.25);
    let g = quarter.pixel(5, 5).unwrap().green() as i32;
    assert!(
        (g - 127).abs() < 3,
        "a quarter of the way the black background is half white: {g}"
    );
    let end = base(1.0);
    assert!(
        end.pixel(5, 5).unwrap().green() < 3,
        "the end is the new view"
    );
}
