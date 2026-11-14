//! Patterns on meshes: a mask equation of the surface coordinates (u, v) blends in a second colour and a hole
//! equation cuts see-through openings. Written first.
use animscript_render::render_with;
use serde_json::{json, Value};

const F: f64 = 400.0;

/// A square facing the camera, 100 px wide, centred in a 200 x 100 canvas; u runs left to right, v top to bottom.
fn quad(pattern: Value) -> Value {
    let (x0, x1, y0, y1, z) = (50.0, 150.0, 0.0, 100.0, 0.0);
    let mut mesh = json!({
        "verts": [[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]],
        "faces": [[3, 2, 1, 0]],
        "uv": [[[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]],
        "double_sided": true, "fill": "#0000ff", "outline": null, "width": 0.0
    });
    mesh["pattern"] = pattern;
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [], "mesh": mesh})
}
fn snap(shape: Value) -> Value {
    json!({"canvas": [200, 100], "background": "#ffffff", "camera": {"position": [100.0, 50.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 50.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 50.0},
        "light3": {"dir": [0.0, 0.0, 1.0], "ambient": 1.0, "bands": 0, "crease": 40.0}, "shapes": [shape]})
}
fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}
fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}

#[test]
fn a_mask_blends_in_the_pattern_colour() {
    let pm = render(&snap(quad(json!({"mask": "u > 0.5", "color": "#ff0000"}))));
    assert_eq!(px(&pm, 70, 50), (0, 0, 255)); // u = 0.2: the fill
    assert_eq!(px(&pm, 130, 50), (255, 0, 0)); // u = 0.8: the pattern colour
}

#[test]
fn a_soft_mask_gives_an_in_between_colour() {
    let pm = render(&snap(quad(json!({"mask": "0.5", "color": "#ff0000"}))));
    let c = px(&pm, 100, 50);
    assert!(c.0 > 100 && c.0 < 160 && c.2 > 100 && c.2 < 160, "{c:?}");
}

#[test]
fn v_runs_down_the_surface() {
    let pm = render(&snap(quad(json!({"mask": "v > 0.5", "color": "#ff0000"}))));
    assert_eq!(px(&pm, 100, 20), (0, 0, 255));
    assert_eq!(px(&pm, 100, 80), (255, 0, 0));
}

#[test]
fn a_hole_is_see_through_and_does_not_hide_what_is_behind() {
    // a second, plain square behind the first shows through the hole
    let mut back = quad(Value::Null);
    back["mesh"]["verts"] = json!([
        [50.0, 0.0, 50.0],
        [150.0, 0.0, 50.0],
        [150.0, 100.0, 50.0],
        [50.0, 100.0, 50.0]
    ]);
    back["mesh"]["fill"] = json!("#00ff00");
    back["mesh"].as_object_mut().unwrap().remove("pattern");
    let front = quad(json!({"hole": "u > 0.5", "color": "#0000ff"}));
    let mut s = snap(front);
    s["shapes"].as_array_mut().unwrap().push(back);
    let pm = render(&s);
    assert_eq!(
        px(&pm, 70, 50),
        (0, 0, 255),
        "the left half of the front square"
    );
    let behind = px(&pm, 130, 50);
    assert!(
        behind.1 > 200 && behind.0 < 60,
        "the back square shows through: {behind:?}"
    );
}

#[test]
fn a_hole_over_nothing_shows_the_background() {
    let pm = render(&snap(quad(json!({"hole": "u > 0.5"}))));
    assert_eq!(px(&pm, 130, 50), (255, 255, 255));
}

#[test]
fn a_bad_equation_draws_the_plain_mesh() {
    let pm = render(&snap(quad(json!({"mask": "u +", "color": "#ff0000"}))));
    assert_eq!(px(&pm, 130, 50), (0, 0, 255));
}

#[test]
fn named_numbers_can_be_used_in_the_equations() {
    let pm = render(&snap(quad(
        json!({"mask": "u > split", "color": "#ff0000", "consts": {"split": 0.25}}),
    )));
    assert_eq!(px(&pm, 100, 50), (255, 0, 0)); // u = 0.5 is past 0.25
    let pm = render(&snap(quad(
        json!({"mask": "u > split", "color": "#ff0000", "consts": {"split": 0.75}}),
    )));
    assert_eq!(px(&pm, 100, 50), (0, 0, 255)); // u = 0.5 is not past 0.75
}

// ---- translucency: meshes with opacity below 1 are blended over what is behind them -------------------------------

fn flat_quad(z: f64, fill: &str, opacity: f64) -> Value {
    let mut q = quad(Value::Null);
    q["mesh"]["verts"] = json!([
        [50.0, 0.0, z],
        [150.0, 0.0, z],
        [150.0, 100.0, z],
        [50.0, 100.0, z]
    ]);
    q["mesh"]["fill"] = json!(fill);
    q["mesh"].as_object_mut().unwrap().remove("pattern");
    q["opacity"] = json!(opacity);
    q
}

#[test]
fn a_translucent_mesh_blends_with_the_mesh_behind_it() {
    // green behind (z = 50), half-opaque red in front (z = 0)
    let mut s = snap(flat_quad(0.0, "#ff0000", 0.5));
    s["shapes"]
        .as_array_mut()
        .unwrap()
        .push(flat_quad(50.0, "#00ff00", 1.0));
    let c = px(&render(&s), 100, 50);
    assert!(
        c.0 > 110 && c.0 < 145 && c.1 > 110 && c.1 < 145 && c.2 < 20,
        "{c:?}"
    );
}

#[test]
fn translucent_meshes_show_through_each_other_whatever_the_order_they_are_listed_in() {
    // a red sheet and a blue sheet, both half-opaque, over white: the nearer one is on top
    let mk = |first_red: bool| {
        let (r, b) = (
            flat_quad(0.0, "#ff0000", 0.5),
            flat_quad(30.0, "#0000ff", 0.5),
        );
        let mut s = snap(if first_red { r.clone() } else { b.clone() });
        s["shapes"]
            .as_array_mut()
            .unwrap()
            .push(if first_red { b } else { r });
        px(&render(&s), 100, 50)
    };
    let (a, b) = (mk(true), mk(false));
    assert_eq!(a, b, "the order of the list does not matter");
    // red in front of blue in front of white: more red than blue, and some of each
    assert!(a.0 > a.2 && a.2 > 20, "{a:?}");
}

#[test]
fn an_opaque_mesh_in_front_hides_a_translucent_one_behind_it() {
    let mut s = snap(flat_quad(0.0, "#00ff00", 1.0));
    s["shapes"]
        .as_array_mut()
        .unwrap()
        .push(flat_quad(50.0, "#ff0000", 0.5));
    assert_eq!(px(&render(&s), 100, 50), (0, 255, 0));
}

#[test]
fn the_mask_can_have_its_own_opacity() {
    // a see-through sheet whose pattern lines are solid
    let mut q = quad(json!({"mask": "u > 0.5", "color": "#000000", "alpha": 1.0}));
    q["opacity"] = json!(0.3);
    let pm = render(&snap(q));
    let (plain, line) = (px(&pm, 70, 50), px(&pm, 130, 50));
    assert!(
        plain.0 > 150,
        "the left half is faint over white: {plain:?}"
    );
    assert!(
        line.0 < 20 && line.1 < 20 && line.2 < 20,
        "the right half is solid black: {line:?}"
    );
}
