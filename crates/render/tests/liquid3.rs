//! 3D liquids in the snapshot: particles become a lit, transparent surface that opaque meshes hide.
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

/// A ball of particles `r` px across at the centre of the picture, `z` px from the screen plane.
fn water(r: f64, spacing: f64, z: f64, color: &str) -> Value {
    let n = (r / spacing).ceil() as i32;
    let mut bytes = vec![];
    let mut count = 0;
    for k in -n..=n {
        for j in -n..=n {
            for i in -n..=n {
                let p = [i as f64 * spacing, j as f64 * spacing, k as f64 * spacing];
                if p[0] * p[0] + p[1] * p[1] + p[2] * p[2] <= r * r {
                    for v in [100.0 + p[0], 50.0 + p[1], z + p[2]] {
                        bytes.extend_from_slice(&(v as f32).to_le_bytes());
                    }
                    count += 1;
                }
            }
        }
    }
    json!({"name": "w", "spacing": spacing, "smoothness": 1.4, "color": color, "color_end": "#1030a0ff",
        "render": "surface", "count": count, "particles": b64(&bytes), "foam_count": 0, "foam": ""})
}

fn cube(cx: f64, cz: f64, half: f64, fill: &str) -> Value {
    let (x0, x1, y0, y1, z0, z1) = (
        cx - half,
        cx + half,
        50.0 - half,
        50.0 + half,
        cz - half,
        cz + half,
    );
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": [[x0,y0,z0],[x1,y0,z0],[x1,y1,z0],[x0,y1,z0],[x0,y0,z1],[x1,y0,z1],[x1,y1,z1],[x0,y1,z1]],
            "faces": [[3,2,1,0],[4,5,6,7],[4,7,3,0],[1,2,6,5],[0,1,5,4],[7,6,2,3]],
            "double_sided": false, "fill": fill, "outline": null, "width": 0.0}})
}

fn snap(shapes: Vec<Value>, fluids: Vec<Value>) -> Value {
    json!({"canvas": [200, 100], "background": "#ffffff", "camera": {"position": [100.0, 50.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 50.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 50.0},
        "light3": {"dir": [0.0, 0.0, 1.0], "ambient": 0.4, "bands": 3, "crease": 40.0},
        "shapes": shapes, "fluids3": fluids})
}

fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let p = pm.pixel(x, y).unwrap();
    (p.red(), p.green(), p.blue())
}

#[test]
fn a_ball_of_water_is_drawn_blue_over_the_background() {
    let pm = render_with(
        &snap(vec![], vec![water(24.0, 5.0, 0.0, "#4f9be8cc")]),
        1.0,
        false,
        false,
    )
    .unwrap();
    let c = px(&pm, 100, 50);
    assert!(
        c.2 as i32 > c.0 as i32 + 30,
        "the middle is not blue: {c:?}"
    );
    assert_eq!(
        px(&pm, 5, 5),
        (255, 255, 255),
        "the corner should stay clear"
    );
    assert_eq!(px(&pm, 195, 95), (255, 255, 255));
}

#[test]
fn a_nearer_opaque_mesh_hides_the_water() {
    // the cube is in front of the water (smaller z) and covers the middle
    let pm = render_with(
        &snap(
            vec![cube(100.0, -60.0, 14.0, "#ff0000")],
            vec![water(24.0, 5.0, 0.0, "#4f9be8cc")],
        ),
        1.0,
        false,
        false,
    )
    .unwrap();
    let c = px(&pm, 100, 50);
    assert!(
        c.0 > 200 && c.2 < 80,
        "the cube should show unchanged: {c:?}"
    );
}

#[test]
fn the_water_is_transparent_over_a_farther_mesh() {
    let behind = render_with(
        &snap(vec![cube(100.0, 60.0, 30.0, "#ff0000")], vec![]),
        1.0,
        false,
        false,
    )
    .unwrap();
    let through = render_with(
        &snap(
            vec![cube(100.0, 60.0, 30.0, "#ff0000")],
            vec![water(24.0, 5.0, 0.0, "#4f9be8aa")],
        ),
        1.0,
        false,
        false,
    )
    .unwrap();
    let (a, b) = (px(&behind, 100, 50), px(&through, 100, 50));
    assert!(a.0 > 200, "the red cube should be there: {a:?}");
    assert_ne!(a, b, "the water changed nothing");
    // still reddish through the water, but bluer than before
    assert!(b.2 as i32 > a.2 as i32 + 20, "no tint: {a:?} -> {b:?}");
    assert!(
        b.0 as i32 > b.2 as i32 - 40 || b.0 > 90,
        "the cube vanished behind opaque water: {b:?}"
    );
}

#[test]
fn dots_mode_draws_spheres_at_the_particles() {
    let mut w = water(15.0, 6.0, 0.0, "#4f9be8ff");
    w["render"] = json!("dots");
    let pm = render_with(&snap(vec![], vec![w]), 1.0, false, false).unwrap();
    assert_ne!(px(&pm, 100, 50), (255, 255, 255));
}
