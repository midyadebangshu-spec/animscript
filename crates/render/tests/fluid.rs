//! The fluid surface is drawn from the snapshot alone (a pure function).
use animscript_render::render;
use serde_json::json;

fn snapshot() -> serde_json::Value {
    let mut pts = vec![];
    for j in 0..6 {
        for i in 0..10 {
            pts.push(json!([100.0 + i as f64 * 10.0, 200.0 + j as f64 * 10.0]));
        }
    }
    json!({"canvas": [300, 300], "background": "#ffffff", "shapes": [],
           "fluids": [{"name": "w", "spacing": 10.0, "color": "#2060d0", "color_end": "#103070", "render": "blob", "behind": false, "particles": pts}]})
}

#[test]
fn the_blob_covers_its_particles_and_nothing_far_from_them() {
    let pm = render(&snapshot(), 1.0, false).unwrap();
    let px = |x: u32, y: u32| pm.pixel(x, y).unwrap();
    let inside = px(145, 225);
    assert!(
        inside.alpha() == 255 && inside.blue() > inside.red(),
        "inside the water: {inside:?}"
    );
    let outside = px(20, 20);
    assert_eq!(
        (outside.red(), outside.green(), outside.blue()),
        (255, 255, 255)
    );
}

#[test]
fn the_surface_is_darker_with_depth() {
    let pm = render(&snapshot(), 1.0, false).unwrap();
    let top = pm.pixel(145, 203).unwrap();
    let deep = pm.pixel(145, 250).unwrap();
    assert!(deep.blue() < top.blue(), "{top:?} {deep:?}");
}

#[test]
fn dots_mode_draws_circles_and_rendering_is_repeatable() {
    let mut s = snapshot();
    s["fluids"][0]["render"] = json!("dots");
    let a = render(&s, 1.0, false).unwrap();
    let b = render(&s, 1.0, false).unwrap();
    assert_eq!(a.data(), b.data());
    assert!(a.pixel(105, 205).unwrap().alpha() == 255);
    let blob = render(&snapshot(), 1.0, false).unwrap();
    assert_eq!(blob.data(), render(&snapshot(), 1.0, false).unwrap().data());
}
