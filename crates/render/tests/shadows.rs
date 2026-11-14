//! Shadows: the BVH agrees with brute force, occluders darken what is behind them, soft shadows have a
//! penumbra, and everything is deterministic.
use animscript_render::bvh::Bvh;
use animscript_render::render_with;
use serde_json::{json, Value};

type V3 = [f64; 3];

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 33) as f64) / (1u64 << 31) as f64
}

#[test]
fn the_bvh_agrees_with_brute_force() {
    let mut seed = 7;
    let mut tris: Vec<[V3; 3]> = vec![];
    for _ in 0..400 {
        let c = [
            lcg(&mut seed) * 100.0,
            lcg(&mut seed) * 100.0,
            lcg(&mut seed) * 100.0,
        ];
        let mut t = [c; 3];
        for v in t.iter_mut() {
            for k in 0..3 {
                v[k] += lcg(&mut seed) * 10.0 - 5.0;
            }
        }
        tris.push(t);
    }
    let bvh = Bvh::new(tris);
    for _ in 0..2000 {
        let o = [
            lcg(&mut seed) * 140.0 - 20.0,
            lcg(&mut seed) * 140.0 - 20.0,
            lcg(&mut seed) * 140.0 - 20.0,
        ];
        let d = [
            lcg(&mut seed) - 0.5,
            lcg(&mut seed) - 0.5,
            lcg(&mut seed) - 0.5,
        ];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let d = [d[0] / l, d[1] / l, d[2] / l];
        let tmax = if lcg(&mut seed) < 0.3 {
            f64::INFINITY
        } else {
            lcg(&mut seed) * 150.0
        };
        assert_eq!(bvh.any_hit(o, d, tmax), bvh.any_hit_brute(o, d, tmax));
    }
}

#[test]
fn an_empty_bvh_hits_nothing() {
    assert!(!Bvh::new(vec![]).any_hit([0.0; 3], [0.0, 0.0, 1.0], f64::INFINITY));
}

const F: f64 = 400.0;

fn quad(verts: Value, fill: &str) -> Value {
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [],
        "mesh": {"verts": verts, "faces": [[3, 2, 1, 0]], "double_sided": true, "fill": fill, "outline": null, "width": 0.0, "surface": [1.0, 0.0, 0.0]}})
}
/// the wall at z = 300
fn wall() -> Value {
    quad(
        json!([
            [-300.0, -300.0, 300.0],
            [500.0, -300.0, 300.0],
            [500.0, 500.0, 300.0],
            [-300.0, 500.0, 300.0]
        ]),
        "#c0c0c0",
    )
}
/// a square card at z = 150, 60 wide, centred on the line of sight
fn card() -> Value {
    quad(
        json!([
            [70.0, 70.0, 150.0],
            [130.0, 70.0, 150.0],
            [130.0, 130.0, 150.0],
            [70.0, 130.0, 150.0]
        ]),
        "#303030",
    )
}
fn light(size: f64, shadows: bool) -> Value {
    json!({"id": "l", "kind": "point", "pos": [100.0, 100.0, 0.0], "dir": [0.0, 0.0, 1.0], "color": [1.0, 1.0, 1.0], "intensity": 3.0,
        "reach": 300.0, "size": size, "angle": 0.4, "softness": 0.2, "shadows": shadows})
}
fn snap(shapes: Vec<Value>, lights: Vec<Value>) -> Value {
    json!({"canvas": [200, 200], "background": "#000000", "camera": {"position": [100.0, 100.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 100.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 100.0},
        "light3": {"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 3, "crease": 40.0}, "shapes": shapes, "lights": lights})
}
fn red(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> i32 {
    pm.pixel(x, y).unwrap().red() as i32
}
fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}

#[test]
fn a_card_casts_a_shadow_on_the_wall_behind_it() {
    // the light is at the camera's height and 300 px from the wall; the card is half way: its shadow
    // on the wall is twice its size (120 px) around the centre. The card itself hides the middle of the
    // picture, so look just outside it, inside the shadow: x = 100 +- 55 is on the card's edge, so use the corner area.
    let with = render(&snap(vec![wall(), card()], vec![light(0.0, true)]));
    let without = render(&snap(vec![wall()], vec![light(0.0, true)]));
    // the screen point (100, 40) is wall seen above the card (the card spans 70..130 on screen too at z=150? no: at
    // z=150 the card projects larger): sample a point in the shadow but beside the card on screen
    let probe = (100u32, 72u32);
    let (a, b) = (
        red(&with, probe.0, probe.1),
        red(&without, probe.0, probe.1),
    );
    assert!(a < b / 2, "in the shadow {a}, unshadowed {b}");
    // far from the card the wall is lit as before
    assert_eq!(red(&with, 5, 5), red(&without, 5, 5));
}

#[test]
fn shadows_can_be_switched_off_per_light() {
    let on = render(&snap(vec![wall(), card()], vec![light(0.0, true)]));
    let off = render(&snap(vec![wall(), card()], vec![light(0.0, false)]));
    assert!(red(&off, 100, 72) > red(&on, 100, 72) + 40);
}

#[test]
fn a_big_light_makes_a_penumbra() {
    // along a row across the shadow's edge a hard shadow jumps; a soft one has in-between values
    let edge_values = |size: f64| -> Vec<i32> {
        let pm = render(&snap(vec![wall(), card()], vec![light(size, true)]));
        (0..200u32).map(|x| red(&pm, x, 72)).collect()
    };
    let steps = |v: &[i32]| v.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap();
    let hard = edge_values(0.0);
    let soft = edge_values(25.0);
    assert!(
        steps(&hard) > 2 * steps(&soft),
        "hard {} soft {}",
        steps(&hard),
        steps(&soft)
    );
}

#[test]
fn shadowed_rendering_is_deterministic() {
    let s = snap(vec![wall(), card()], vec![light(20.0, true)]);
    assert_eq!(render(&s).data(), render(&s).data());
}
