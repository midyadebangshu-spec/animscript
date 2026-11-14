#![allow(clippy::cloned_ref_to_slice_refs)]
//! Photographic lighting: the shading model and what the renderer does with `lights`.
use animscript_render::light::*;
use animscript_render::render_with;
use serde_json::{json, Value};

fn lt(kind: Kind) -> Lt {
    Lt {
        kind,
        pos: [0.0; 3],
        dir: [0.0, 0.0, 1.0],
        color: [1.0; 3],
        intensity: 1.0,
        reach: 100.0,
        size: 0.0,
        angle: 0.6,
        softness: 0.25,
        ground: None,
        shadows: true,
    }
}
fn surf(pos: V3) -> Surf {
    Surf {
        pos,
        n: [0.0, 0.0, -1.0],
        view: [0.0, 0.0, -1.0],
        albedo: [0.8; 3],
        rough: 0.6,
        metal: 0.0,
        emissive: 0.0,
        ao: 1.0,
    }
}
fn lum(c: [f64; 3]) -> f64 {
    (c[0] + c[1] + c[2]) / 3.0
}

#[test]
fn a_point_light_falls_off_with_the_square_of_the_distance() {
    let mut l = lt(Kind::Point);
    l.intensity = 1.0;
    l.reach = 100.0;
    let mut at = |d: f64| {
        l.pos = [0.0, 0.0, -d];
        // the diffuse part only: a rough surface, light along the normal
        let mut s = surf([0.0; 3]);
        s.rough = 1.0;
        lum(radiance(&[l.clone()], &s, &|_| 1.0))
    };
    let (a, b) = (at(200.0), at(400.0));
    assert!((a / b - 4.0).abs() < 0.1, "{a} {b}");
    // the intensity is the irradiance at `reach`
    assert!(at(100.0) > a);
}

#[test]
fn a_surface_turned_from_the_light_is_dark() {
    let mut l = lt(Kind::Sun);
    l.dir = [0.0, 0.0, 1.0];
    let lit = lum(radiance(&[l.clone()], &surf([0.0; 3]), &|_| 1.0));
    let mut away = surf([0.0; 3]);
    away.n = [0.0, 0.0, 1.0];
    let dark = lum(radiance(&[l], &away, &|_| 1.0));
    assert!(lit > 0.1 && dark == 0.0, "{lit} {dark}");
}

#[test]
fn light_colour_tints_the_surface() {
    let mut l = lt(Kind::Sun);
    l.color = [1.0, 0.2, 0.0];
    let c = radiance(&[l], &surf([0.0; 3]), &|_| 1.0);
    assert!(c[0] > 3.0 * c[1] && c[2] == 0.0, "{c:?}");
}

#[test]
fn a_shadowed_light_adds_nothing() {
    let l = lt(Kind::Sun);
    let s = surf([0.0; 3]);
    let full = lum(radiance(&[l.clone()], &s, &|_| 1.0));
    let half = lum(radiance(&[l.clone()], &s, &|_| 0.5));
    let none = lum(radiance(&[l], &s, &|_| 0.0));
    assert!((half - full / 2.0).abs() < 1e-9 && none == 0.0);
}

#[test]
fn a_spot_lights_only_its_cone() {
    let mut l = lt(Kind::Spot);
    l.pos = [0.0, 0.0, -100.0];
    l.dir = [0.0, 0.0, 1.0];
    l.angle = 0.3;
    l.softness = 0.2;
    let inside = lum(radiance(&[l.clone()], &surf([0.0, 0.0, 0.0]), &|_| 1.0));
    let outside = lum(radiance(&[l], &surf([100.0, 0.0, 0.0]), &|_| 1.0));
    assert!(inside > 0.1 && outside == 0.0, "{inside} {outside}");
}

#[test]
fn smooth_surfaces_have_a_sharper_brighter_highlight() {
    let mut l = lt(Kind::Sun);
    l.dir = [0.0, 0.0, 1.0];
    let peak = |rough: f64| {
        let mut s = surf([0.0; 3]);
        s.rough = rough;
        s.albedo = [0.2; 3];
        lum(radiance(&[l.clone()], &s, &|_| 1.0))
    };
    assert!(peak(0.1) > peak(0.9) * 1.5, "{} {}", peak(0.1), peak(0.9));
}

#[test]
fn metals_take_their_colour_from_the_albedo_in_the_highlight() {
    let l = lt(Kind::Sun);
    let mut s = surf([0.0; 3]);
    s.metal = 1.0;
    s.rough = 0.3;
    s.albedo = [1.0, 0.5, 0.1];
    let c = radiance(&[l], &s, &|_| 1.0);
    assert!(c[0] > c[1] && c[1] > c[2], "{c:?}");
}

#[test]
fn ambient_is_a_hemisphere_and_emission_adds_to_it() {
    let mut a = lt(Kind::Ambient);
    a.color = [1.0, 1.0, 1.0];
    a.ground = Some([0.0, 0.0, 0.0]);
    a.intensity = 1.0;
    let mut up = surf([0.0; 3]);
    up.n = [0.0, -1.0, 0.0];
    let mut down = up;
    down.n = [0.0, 1.0, 0.0];
    assert!(lum(radiance(&[a.clone()], &up, &|_| 1.0)) > 0.5);
    assert!(lum(radiance(&[a.clone()], &down, &|_| 1.0)) < 1e-9);
    let mut glow = down;
    glow.emissive = 2.0;
    assert!(lum(radiance(&[a], &glow, &|_| 1.0)) > 1.0);
}

#[test]
fn the_tone_map_is_monotonic_bounded_and_black_stays_black() {
    let t = |x: f64| tonemap([x; 3], 1.0)[0];
    assert_eq!(t(0.0), 0.0);
    let mut prev = -1.0;
    for k in 0..200 {
        let v = t(k as f64 * 0.1);
        assert!(v >= prev && v <= 1.0);
        prev = v;
    }
    assert!(t(1000.0) <= 1.0);
    assert!(tonemap([0.5; 3], 4.0)[0] > tonemap([0.5; 3], 1.0)[0]);
}

// ---- through the renderer

const F: f64 = 400.0;

fn quad(z: f64, fill: &str, surface: Value) -> Value {
    let mut mesh = json!({"verts": [[-300.0, -300.0, z], [500.0, -300.0, z], [500.0, 500.0, z], [-300.0, 500.0, z]],
        "faces": [[3, 2, 1, 0]], "double_sided": true, "fill": fill, "outline": null, "width": 0.0});
    if !surface.is_null() {
        mesh["surface"] = surface;
    }
    json!({"z": 0, "visible": true, "opacity": 1.0, "vertices": [], "edges": [], "faces": [], "mesh": mesh})
}

fn snap(shapes: Vec<Value>, lights: Vec<Value>) -> Value {
    let mut s = json!({"canvas": [200, 200], "background": "#000000", "camera": {"position": [100.0, 100.0], "zoom": 1.0, "rotation": 0.0},
        "camera3": {"eye": [100.0, 100.0, -F], "right": [1.0, 0.0, 0.0], "down": [0.0, 1.0, 0.0], "forward": [0.0, 0.0, 1.0], "focal": F, "cx": 100.0, "cy": 100.0},
        "light3": {"dir": [0.0, 0.0, 1.0], "ambient": 0.35, "bands": 3, "crease": 40.0}, "shapes": shapes});
    if !lights.is_empty() {
        s["lights"] = json!(lights);
    }
    s
}

fn light_json(kind: &str, pos: [f64; 3], dir: [f64; 3], intensity: f64) -> Value {
    json!({"id": "l", "kind": kind, "pos": pos, "dir": dir, "color": [1.0, 1.0, 1.0], "intensity": intensity,
        "reach": 300.0, "size": 0.0, "angle": 0.4, "softness": 0.2})
}

fn px(pm: &tiny_skia::Pixmap, x: u32, y: u32) -> (i32, i32, i32) {
    let p = pm.pixel(x, y).unwrap();
    (p.red() as i32, p.green() as i32, p.blue() as i32)
}
fn render(s: &Value) -> tiny_skia::Pixmap {
    render_with(s, 1.0, false, false).unwrap()
}

#[test]
fn a_point_light_makes_a_pool_of_light_on_a_wall() {
    let s = snap(
        vec![quad(300.0, "#c0c0c0", Value::Null)],
        vec![light_json(
            "point",
            [100.0, 100.0, 100.0],
            [0.0, 0.0, 1.0],
            2.0,
        )],
    );
    let pm = render(&s);
    let (centre, edge) = (px(&pm, 100, 100), px(&pm, 5, 5));
    assert!(centre.0 > edge.0 + 40, "{centre:?} {edge:?}");
}

#[test]
fn a_scene_without_lights_keeps_its_toon_shading() {
    let s = snap(vec![quad(300.0, "#c0c0c0", Value::Null)], vec![]);
    let pm = render(&s);
    assert_eq!(px(&pm, 100, 100), px(&pm, 5, 5));
}

#[test]
fn an_emissive_surface_glows_without_light_on_it() {
    let amb = json!({"id": "a", "kind": "ambient", "pos": [0.0, 0.0, 0.0], "dir": [0.0, 1.0, 0.0], "color": [1.0, 1.0, 1.0],
        "intensity": 0.05, "reach": 200.0, "size": 0.0, "angle": 0.7, "softness": 0.2});
    let dark = render(&snap(
        vec![quad(300.0, "#c08040", Value::Null)],
        vec![amb.clone()],
    ));
    let glow = render(&snap(
        vec![quad(300.0, "#c08040", json!([0.6, 0.0, 3.0]))],
        vec![amb],
    ));
    assert!(px(&glow, 100, 100).0 > px(&dark, 100, 100).0 + 80);
}

#[test]
fn lit_rendering_is_deterministic() {
    let s = snap(
        vec![quad(300.0, "#c0c0c0", json!([0.3, 0.5, 0.0]))],
        vec![light_json(
            "spot",
            [100.0, 100.0, 0.0],
            [0.0, 0.0, 1.0],
            3.0,
        )],
    );
    let (a, b) = (render(&s), render(&s));
    assert_eq!(a.data(), b.data());
}
