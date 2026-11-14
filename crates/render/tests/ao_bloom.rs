//! Ambient occlusion darkens creases; bloom spreads bright light into its surroundings.
use animscript_render::render_with;
use serde_json::{json, Value};

const F: f64 = 400.0;

fn quad(verts: Value, fill: &str, surface: Value) -> Value {
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": verts, "faces": [[3, 2, 1, 0]], "double_sided": true, "fill": fill, "outline": null, "width": 0.0, "surface": surface}})
}
fn ambient() -> Value {
    json!({"id": "a", "kind": "ambient", "pos": [0.0, 0.0, 0.0], "dir": [0.0, 1.0, 0.0], "color": [1.0, 1.0, 1.0], "intensity": 0.5,
        "reach": 200.0, "size": 0.0, "angle": 0.7, "softness": 0.2, "shadows": true})
}
fn snap(shapes: Vec<Value>, light3: Value) -> Value {
    json!({"canvas": [200, 200], "background": "#000000", "camera": {"position": [100.0, 100.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 100.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 100.0},
        "light3": light3, "shapes": shapes, "lights": [ambient()]})
}
fn red(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> i32 {
    pm.pixel(x, y).unwrap().red() as i32
}
fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}

/// a floor (y = 140) meeting a wall (z = 400) in a crease, seen from the front and slightly above
fn corner() -> Vec<Value> {
    vec![
        quad(
            json!([
                [-300.0, 140.0, 0.0],
                [500.0, 140.0, 0.0],
                [500.0, 140.0, 400.0],
                [-300.0, 140.0, 400.0]
            ]),
            "#c0c0c0",
            json!([1.0, 0.0, 0.0]),
        ),
        quad(
            json!([
                [-300.0, -300.0, 400.0],
                [500.0, -300.0, 400.0],
                [500.0, 140.0, 400.0],
                [-300.0, 140.0, 400.0]
            ]),
            "#c0c0c0",
            json!([1.0, 0.0, 0.0]),
        ),
    ]
}

#[test]
fn ambient_occlusion_darkens_the_crease() {
    let l3 =
        json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 0, "crease": 40.0, "ao": 80.0});
    let pm = render(&snap(corner(), l3));
    // the crease is where the floor meets the wall: screen y = 100 + 400 * 40 / 800 = 120
    let near_crease = red(&pm, 100, 121);
    let on_floor = red(&pm, 100, 139);
    let on_wall = red(&pm, 100, 60);
    assert!(
        near_crease < on_floor - 3 && near_crease < on_wall - 3,
        "crease {near_crease} floor {on_floor} wall {on_wall}"
    );
    // with AO off the corner is flat
    let off = render(&snap(
        corner(),
        json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 0, "crease": 40.0, "ao": 0.0}),
    ));
    assert!((red(&off, 100, 121) - red(&off, 100, 139)).abs() <= red(&off, 100, 121) / 3 + 10);
    assert!(red(&off, 100, 121) > near_crease);
}

#[test]
fn bloom_spreads_a_bright_patch_into_the_dark_around_it() {
    let patch = quad(
        json!([
            [80.0, 80.0, 300.0],
            [120.0, 80.0, 300.0],
            [120.0, 120.0, 300.0],
            [80.0, 120.0, 300.0]
        ]),
        "#ffffff",
        json!([1.0, 0.0, 6.0]),
    );
    let l3 = |b: f64| json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 0, "crease": 40.0, "bloom": b});
    let plain = render(&snap(vec![patch.clone()], l3(0.0)));
    let glow = render(&snap(vec![patch], l3(1.0)));
    // a pixel just outside the patch
    let (a, b) = (red(&plain, 100, 84), red(&glow, 100, 84));
    assert!(b > a + 10, "{a} -> {b}");
    // far away is still dark
    assert!(red(&glow, 3, 3) < 15);
}

#[test]
fn ao_and_bloom_are_deterministic() {
    let l3 = json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 0, "crease": 40.0, "ao": 80.0, "bloom": 0.5});
    let s = snap(corner(), l3);
    assert_eq!(render(&s).data(), render(&s).data());
}
