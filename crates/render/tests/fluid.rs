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

fn dark(mut s: serde_json::Value) -> serde_json::Value {
    s["background"] = json!("#000000");
    s
}

fn covered(pm: &tiny_skia::Pixmap) -> usize {
    (0..pm.height())
        .flat_map(|y| (0..pm.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = pm.pixel(x, y).unwrap();
            p.red() as u32 + p.green() as u32 + p.blue() as u32 > 60
        })
        .count()
}

#[test]
fn a_lower_smoothness_gives_a_crisper_smaller_surface() {
    let mut loose = dark(snapshot());
    loose["fluids"][0]["smoothness"] = json!(2.0);
    let mut crisp = dark(snapshot());
    crisp["fluids"][0]["smoothness"] = json!(1.2);
    let (a, b) = (
        covered(&render(&loose, 1.0, false).unwrap()),
        covered(&render(&crisp, 1.0, false).unwrap()),
    );
    assert!(b < a, "crisp {b} vs loose {a}");
    assert!(b > 3000, "but it still covers the block of particles: {b}"); // 10 x 6 particles at 10 px is 6000 px^2
}

#[test]
fn foam_is_drawn_as_white_specks_that_fade() {
    let mut s = dark(snapshot());
    s["fluids"][0]["foam"] = json!([[50.0, 50.0, 0.0], [80.0, 50.0, 0.9]]);
    let pm = render(&s, 1.0, false).unwrap();
    let fresh = pm.pixel(50, 50).unwrap();
    assert!(
        fresh.red() > 230 && fresh.blue() > 230,
        "a white speck: {fresh:?}"
    );
    let old = pm.pixel(80, 50).unwrap();
    assert!(
        old.red() < fresh.red() && old.red() > 0,
        "older foam is fainter: {old:?} {fresh:?}"
    );
}
