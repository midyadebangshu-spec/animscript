//! Reflections, refraction and materials: mirrors show their surroundings, water and glass bend and
//! reflect what is behind them.
use animscript_render::render_with;
use serde_json::{json, Value};

const F: f64 = 400.0;

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

fn mesh(verts: Value, faces: Value, fill: &str, material: Value, double: bool) -> Value {
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": verts, "faces": faces, "double_sided": double, "fill": fill, "outline": null, "width": 0.0, "material": material}})
}

fn cube(cx: f64, cy: f64, cz: f64, half: f64, fill: &str, material: Value) -> Value {
    let (x0, x1, y0, y1, z0, z1) = (
        cx - half,
        cx + half,
        cy - half,
        cy + half,
        cz - half,
        cz + half,
    );
    mesh(
        json!([
            [x0, y0, z0],
            [x1, y0, z0],
            [x1, y1, z0],
            [x0, y1, z0],
            [x0, y0, z1],
            [x1, y0, z1],
            [x1, y1, z1],
            [x0, y1, z1]
        ]),
        json!([
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [4, 7, 3, 0],
            [1, 2, 6, 5],
            [0, 1, 5, 4],
            [7, 6, 2, 3]
        ]),
        fill,
        material,
        false,
    )
}

/// A big horizontal quad at height `y` (the floor).
fn floor(y: f64, fill: &str, material: Value) -> Value {
    mesh(
        json!([
            [-200.0, y, -300.0],
            [400.0, y, -300.0],
            [400.0, y, 800.0],
            [-200.0, y, 800.0]
        ]),
        json!([[0, 1, 2, 3]]),
        fill,
        material,
        true,
    )
}

fn water(r: f64, spacing: f64, cx: f64, cy: f64, z: f64, refract: f64) -> Value {
    let n = (r / spacing).ceil() as i32;
    let mut bytes = vec![];
    let mut count = 0;
    for k in -n..=n {
        for j in -n..=n {
            for i in -n..=n {
                let p = [i as f64 * spacing, j as f64 * spacing, k as f64 * spacing];
                if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= r * r {
                    for v in [cx + p[0], cy + p[1], z + p[2]] {
                        bytes.extend_from_slice(&(v as f32).to_le_bytes());
                    }
                    count += 1;
                }
            }
        }
    }
    json!({"name": "w", "spacing": spacing, "smoothness": 1.4, "color": "#4f9be820", "color_end": "#4f9be8ff",
        "render": "surface", "reflect": 0.0, "refract": refract, "count": count, "particles": b64(&bytes), "foam_count": 0, "foam": ""})
}

fn snap(shapes: Vec<Value>, fluids: Vec<Value>, light: Value) -> Value {
    json!({"canvas": [200, 200], "background": "#ffffff", "camera": {"position": [100.0, 100.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 100.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 100.0},
        "light3": light, "shapes": shapes, "fluids3": fluids})
}

fn light() -> Value {
    json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.6, "bands": 0, "crease": 40.0})
}

fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (i32, i32, i32) {
    let p = pm.pixel(x, y).unwrap();
    (p.red() as i32, p.green() as i32, p.blue() as i32)
}

fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}

fn is_red(c: (i32, i32, i32)) -> bool {
    c.0 > 150 && c.1 < 110 && c.2 < 110
}

fn mirror(r: f64) -> Value {
    json!({"kind": "mirror", "reflect": r, "refract": 0.0, "opacity": 1.0})
}

#[test]
fn a_mirror_floor_shows_the_cube_standing_on_it_upside_down() {
    // the cube hovers over a floor at y = 140; its reflection hangs below it on screen
    let red = cube(100.0, 100.0, 100.0, 15.0, "#ff0000", Value::Null);
    let plain = render(&snap(
        vec![red.clone(), floor(140.0, "#808080", Value::Null)],
        vec![],
        light(),
    ));
    let shiny = render(&snap(
        vec![red, floor(140.0, "#808080", mirror(1.0))],
        vec![],
        light(),
    ));
    // the reflected cube centre: y = 180 seen at 100 + 400 * 80 / 500 = 164
    assert!(
        !is_red(px(&plain, 100, 164)),
        "the plain floor shows red: {:?}",
        px(&plain, 100, 164)
    );
    assert!(
        is_red(px(&shiny, 100, 164)),
        "no reflection of the cube: {:?}",
        px(&shiny, 100, 164)
    );
    // the cube itself is where it was
    assert!(is_red(px(&shiny, 100, 100)));
}

#[test]
fn a_half_mirror_blends_the_reflection_with_its_own_colour() {
    let red = cube(100.0, 100.0, 100.0, 15.0, "#ff0000", Value::Null);
    let full = px(
        &render(&snap(
            vec![red.clone(), floor(140.0, "#808080", mirror(1.0))],
            vec![],
            light(),
        )),
        100,
        164,
    );
    let half = px(
        &render(&snap(
            vec![red, floor(140.0, "#808080", mirror(0.5))],
            vec![],
            light(),
        )),
        100,
        164,
    );
    assert!(
        half.0 < full.0 && half.1 > full.1,
        "half {half:?} is not between grey and red {full:?}"
    );
}

#[test]
fn a_mirror_reflects_the_sky_where_nothing_is_in_the_way() {
    let l = json!({"dir": [0.0, 0.0, 1.0], "ambient": 0.6, "bands": 0, "crease": 40.0, "sky": ["#0000ff", "#00ff00"]});
    let pm = render(&snap(vec![floor(140.0, "#808080", mirror(1.0))], vec![], l));
    // looking at a floor shows the sky near the horizon (green) shading into blue higher up
    let c = px(&pm, 100, 150);
    assert!(c.1 > 100 || c.2 > 100, "the floor reflects no sky: {c:?}");
    assert!(c.0 < 100, "the sky has no red: {c:?}");
}

#[test]
fn water_bends_what_is_behind_it() {
    // a red cube far behind the ball of water, off to one side: its picture through the water moves
    let behind = cube(130.0, 100.0, 300.0, 12.0, "#ff0000", Value::Null);
    let straight = render(&snap(
        vec![behind.clone()],
        vec![water(36.0, 6.0, 100.0, 100.0, 40.0, 0.0)],
        light(),
    ));
    let bent = render(&snap(
        vec![behind],
        vec![water(36.0, 6.0, 100.0, 100.0, 40.0, 1.0)],
        light(),
    ));
    let mut different = 0;
    for y in 0..200 {
        for x in 0..200 {
            let (a, b) = (px(&straight, x, y), px(&bent, x, y));
            if (a.0 - b.0).abs() + (a.1 - b.1).abs() + (a.2 - b.2).abs() > 60 {
                different += 1;
            }
        }
    }
    assert!(different > 25, "refraction changed only {different} pixels");
    // with no refraction the cube sits where the straight line puts it: pinkish at x = 117
    let c = px(&straight, 117, 100);
    assert!(c.0 > c.1 + 20, "the cube is not where it should be: {c:?}");
}

#[test]
fn glass_lets_a_farther_mesh_show_through_and_does_not_hide_it() {
    let red = cube(100.0, 100.0, 300.0, 20.0, "#ff0000", Value::Null);
    let pane = cube(
        100.0,
        100.0,
        100.0,
        40.0,
        "#c0e8ff",
        json!({"kind": "glass", "reflect": 0.2, "refract": 1.0, "opacity": 0.2}),
    );
    let bare = render(&snap(vec![red.clone()], vec![], light()));
    let through = render(&snap(vec![red, pane], vec![], light()));
    let (a, b) = (px(&bare, 100, 100), px(&through, 100, 100));
    assert!(is_red(a));
    assert_ne!(a, b, "the glass changed nothing");
    // still clearly the red cube, tinted a little blue
    assert!(
        b.0 > 140 && b.2 > a.2,
        "no tint through the glass: {a:?} -> {b:?}"
    );
}
