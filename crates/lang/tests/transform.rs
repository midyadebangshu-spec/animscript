//! Per-face colour, shape transforms, keyframes, the layer fold and radial_array in the language.
//! Written before the implementation.
use animscript_core::Kind;
use animscript_lang::layers::{fold, BlendMode, Contribution};
use animscript_lang::*;
use serde_json::Value;
use std::collections::BTreeMap;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";

fn body(s: &str) -> String {
    format!("{PROJECT}{s}")
}
fn load_err(src: &str) -> Diagnostic {
    match load(src) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn run_err(src: &str) -> Diagnostic {
    match run(src, &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn ok(src: &str) -> Interp {
    match load(src) {
        Ok(i) => i,
        Err(d) => panic!("{}", d.render(Some(src), "test")),
    }
}
fn go(src: &str) -> RunResult {
    match run(src, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(src), "test")),
    }
}
fn vtx(snap: &Value, shape: usize) -> BTreeMap<u64, (f64, f64)> {
    snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v[0].as_u64().unwrap(),
                (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()),
            )
        })
        .collect()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn near2(a: (f64, f64), b: (f64, f64)) {
    near(a.0, b.0);
    near(a.1, b.1);
}
fn rotation(snap: &Value) -> f64 {
    snap["shapes"][0]["transform"]["rotation"].as_f64().unwrap()
}
fn pair(v: &Value) -> (f64, f64) {
    (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}
/// y of vertex `id` at every tick.
fn ys(r: &RunResult, id: u64) -> Vec<f64> {
    r.snapshots.iter().map(|s| vtx(s, 0)[&id].1).collect()
}
const SQ: &str = "shape s = square(size: 100)\n";

// ---- per-face and per-edge colour -----------------------------------------------------------

#[test]
fn colour_one_face_or_edge() {
    let it = ok("shape s = square(size: 100)\n\
                 s.f1.fill = #f00\n\
                 s.e2.stroke = #00f\n\
                 s.e2.stroke_width = 6\n\
                 assert s.f1.fill == #f00\n\
                 assert s.e2.stroke == #0000ff\n\
                 assert s.e1.stroke == none\n\
                 assert s.e2.stroke_width == 6\n");
    let s = &it.shapes["s"];
    assert_eq!(s.faces[&1].fill, Some([255, 0, 0, 255]));
    assert_eq!(
        (s.edges[&2].stroke, s.edges[&2].width),
        (Some([0, 0, 255, 255]), Some(6.0))
    );
    ok("shape s = square(size: 100)\ns.f1.fill = #f00\ns.f1.fill = none\nassert s.f1.fill == none\n");
}

#[test]
fn colour_a_selection() {
    let it = ok("shape s = square(size: 100)\ns.e3.extrude(100)\n\
                 s.faces.where(f -> f.x > 0).fill = #0f0\n\
                 s.edges.every(2).stroke = #123456\n");
    let s = &it.shapes["s"];
    assert_eq!(
        (s.faces[&1].fill, s.faces[&2].fill),
        (None, Some([0, 255, 0, 255]))
    );
    assert_eq!(s.edges[&1].stroke, Some([0x12, 0x34, 0x56, 255]));
    assert_eq!(s.edges[&2].stroke, None);
}

#[test]
fn colour_errors() {
    assert_eq!(
        load_err(&format!("{SQ}s.e1.fill = #f00\n")).code,
        "E-SHP-005"
    );
    assert_eq!(
        load_err(&format!("{SQ}s.f1.stroke = #f00\n")).code,
        "E-SHP-005"
    );
    assert_eq!(load_err(&format!("{SQ}s.f1.fill = 5\n")).code, "E-TYP-001");
    assert_eq!(
        load_err(&format!("{SQ}s.e1.stroke_width = #f00\n")).code,
        "E-TYP-001"
    );
    assert_eq!(
        load_err(&format!("{SQ}s.v1.fill = #f00\n")).code,
        "E-NAM-003"
    );
    assert_eq!(
        load_err(&format!("{SQ}s.f9.fill = #f00\n")).code,
        "E-SHP-002"
    );
}

#[test]
fn snapshot_carries_the_effective_colour_of_every_face_and_edge() {
    let src = body("shape s = square(size: 100)\ns.fill = #111111\ns.stroke = #222222\ns.stroke_width = 2\n\
                    s.e3.extrude(100)\ns.f2.fill = #ff0000\ns.e1.stroke = none\ns.e2.stroke_width = 9\n\
                    scene \"a\" for 1.second:\n    main:\n        wait 1.second\n");
    let r = go(&src);
    let sh = &r.snapshots[0]["shapes"][0];
    let face = |i: u64| {
        sh["faces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == i)
            .unwrap()
            .clone()
    };
    assert_eq!(face(1)["fill"], "#111111"); // falls back to the shape
    assert_eq!(face(2)["fill"], "#ff0000"); // its own
    let edge = |i: u64| {
        sh["edges"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == i)
            .unwrap()
            .clone()
    };
    assert_eq!(edge(2)["stroke"], "#222222");
    assert_eq!(edge(2)["stroke_width"], 9.0);
    assert_eq!(edge(3)["stroke_width"], 2.0);
}

#[test]
fn colour_can_change_at_runtime() {
    let src = body("shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        wait 0.5.seconds\n        s.f1.fill = #f00\n");
    let r = go(&src);
    assert_eq!(
        r.snapshots[11]["shapes"][0]["faces"][0]["fill"],
        Value::Null
    );
    assert_eq!(r.snapshots[12]["shapes"][0]["faces"][0]["fill"], "#ff0000");
}

// ---- shape position, scale, rotation --------------------------------------------------------

#[test]
fn rotation_scale_and_position_move_the_resolved_points() {
    let r = go(&body("shape s = square(size: 100)\ns.rotation = 90.degrees\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"));
    near2(vtx(&r.snapshots[0], 0)[&1], (50.0, -50.0)); // clockwise on screen: (-50, -50) -> (50, -50)
    near(rotation(&r.snapshots[0]), 0.25);
    let r = go(&body("shape s = square(size: 100)\ns.scale = 2\ns.position = (10, 0)\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"));
    near2(vtx(&r.snapshots[0], 0)[&1], (-90.0, -100.0));
    let r = go(&body("shape s = square(size: 100)\ns.scale = (3, 1)\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"));
    near2(vtx(&r.snapshots[0], 0)[&3], (150.0, 50.0));
}

#[test]
fn the_pivot_defaults_to_the_bounding_box_centre_and_can_be_set() {
    let r = go(&body("shape s = square(size: 100, at: (400, 300))\ns.rotation = 90.degrees\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"));
    near2(vtx(&r.snapshots[0], 0)[&1], (450.0, 250.0)); // turned about its own centre (400, 300)
    near2(
        pair(&r.snapshots[0]["shapes"][0]["transform"]["pivot"]),
        (400.0, 300.0),
    );
    let r = go(&body("shape s = square(size: 100, at: (400, 300))\ns.pivot = (0, 0)\ns.rotation = 90.degrees\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"));
    near2(vtx(&r.snapshots[0], 0)[&1], (-250.0, 350.0)); // turned about the origin
    ok("shape s = square(size: 100)\nassert s.pivot == (0, 0)\nassert s.rotation == 0.deg\nassert s.scale == (1, 1)\nassert s.position == (0, 0)\n");
}

#[test]
fn handles_follow_the_transform_exactly_even_under_non_uniform_scale() {
    let r = go(&body("shape s = square(size: 100)\ns.e1.bend(45.degrees)\ns.scale = (2, 1)\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"));
    let e1 = r.snapshots[0]["shapes"][0]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == 1)
        .unwrap()
        .clone();
    near2(pair(&e1["pts"][1]), (-200.0, 0.0)); // the local control point (-100, 0) scaled in x
    near2(pair(&e1["pts"][0]), (-100.0, -50.0));
    let f = &r.snapshots[0]["shapes"][0]["faces"][0]["segments"][0];
    near2(pair(&f[1]), (-200.0, 0.0));
}

#[test]
fn the_transform_is_part_of_the_hash() {
    let make = |rot: &str| {
        body(&format!("shape s = square(size: 100)\ns.rotation = {rot}\nscene \"a\" for 1.second:\n    main:\n        wait 1.second\n"))
    };
    let (a, b, c) = (
        go(&make("10.deg")),
        go(&make("10.deg")),
        go(&make("11.deg")),
    );
    assert_eq!(a.hashes, b.hashes);
    assert_ne!(a.hashes[0], c.hashes[0]);
}

#[test]
fn transform_errors() {
    assert_eq!(
        load_err(&format!("{SQ}s.rotation = 90\n")).code,
        "E-TYP-002"
    );
    assert_eq!(
        load_err(&format!("{SQ}s.scale = \"a\"\n")).code,
        "E-TYP-001"
    );
    assert_eq!(load_err(&format!("{SQ}s.position = 5\n")).code, "E-TYP-001");
    assert_eq!(
        load_err(&format!("{SQ}s.rotate_by(90)\n")).code,
        "E-TYP-002"
    );
    assert_eq!(
        load_err(&format!("{SQ}s.scale_to(1.second)\n")).code,
        "E-TYP-002"
    );
}

#[test]
fn build_time_transform_calls_apply_instantly_and_accumulate() {
    let it = ok(&format!("{SQ}s.rotate_by(90.deg)\ns.rotate_by(90.deg)\ns.move_by(5, 0)\ns.move_to((7, 8))\ns.scale_to(2)\n"));
    let s = &it.shapes["s"];
    assert!((s.rotation - 0.5).abs() < 1e-12);
    assert_eq!((s.position, s.scale), ([7.0, 8.0], [2.0, 2.0]));
}

// ---- transform actions ----------------------------------------------------------------------

#[test]
fn rotate_by_over_time_and_commit() {
    let r = go(&body(&format!("{SQ}scene \"a\" for 1.second:\n    main:\n        s.rotate_by(90.deg) over 0.5.seconds ease linear\n")));
    near(rotation(&r.snapshots[0]), 0.0);
    near(rotation(&r.snapshots[6]), 0.125);
    near2(vtx(&r.snapshots[6], 0)[&1], (0.0, -50.0 * 2f64.sqrt())); // (-50, -50) turned 45 degrees clockwise
    near(rotation(&r.snapshots[12]), 0.25); // committed at the end tick
    near(rotation(&r.snapshots[23]), 0.25);
}

#[test]
fn rotation_interpolates_linearly_so_full_spins_work() {
    let r = go(&body(&format!("{SQ}scene \"a\" for 2.seconds:\n    main:\n        s.rotate_by(360.deg) over 1.second ease linear\n")));
    near(rotation(&r.snapshots[12]), 0.5);
    near2(vtx(&r.snapshots[12], 0)[&1], (50.0, 50.0)); // half way round
    near(rotation(&r.snapshots[24]), 1.0);
    near2(vtx(&r.snapshots[24], 0)[&1], (-50.0, -50.0)); // back where it started
}

#[test]
fn scale_move_and_rotate_to_actions() {
    // five blocking actions of 12 ticks each: scale 0-12, move_to 12-24, move_by 24-36, rotate_to 36-48, scale 48-60
    let r = go(&body(&format!(
        "{SQ}scene \"a\" for 4.seconds:\n    main:\n        s.scale_to(2) over 0.5.seconds ease linear\n        s.move_to((100, 50)) over 0.5.seconds ease linear\n        s.move_by(10, 0) over 0.5.seconds ease linear\n        s.rotate_to(180.deg) over 0.5.seconds ease linear\n        s.scale_to((1, 3)) over 0.5.seconds ease linear\n"
    )));
    let t = |k: usize| &r.snapshots[k]["shapes"][0]["transform"];
    near2(pair(&t(6)["scale"]), (1.5, 1.5));
    near2(pair(&t(12)["scale"]), (2.0, 2.0));
    near2(pair(&t(18)["position"]), (50.0, 25.0));
    near2(pair(&t(24)["position"]), (100.0, 50.0));
    near2(pair(&t(30)["position"]), (105.0, 50.0));
    near2(pair(&t(48)["position"]), (110.0, 50.0));
    near(t(42)["rotation"].as_f64().unwrap(), 0.25);
    near(t(48)["rotation"].as_f64().unwrap(), 0.5);
    near2(pair(&t(54)["scale"]), (1.5, 2.5));
    near2(pair(&t(60)["scale"]), (1.0, 3.0));
}

#[test]
fn zero_time_transform_calls_at_runtime() {
    let r = go(&body(&format!("{SQ}scene \"a\" for 1.second:\n    main:\n        wait 0.5.seconds\n        s.rotate_by(90.deg)\n")));
    near(rotation(&r.snapshots[11]), 0.0);
    near(rotation(&r.snapshots[12]), 0.25);
}

#[test]
fn a_transform_action_runs_beside_vertex_edits_but_two_rotations_queue() {
    let r = go(&body(&format!(
        "{SQ}scene \"a\" for 2.seconds:\n    main:\n        together:\n            s.rotate_by(90.deg) over 0.5.seconds ease linear\n            s.v[2].move_by(0, 10) over 0.5.seconds ease linear\n"
    )));
    near(rotation(&r.snapshots[6]), 0.125);
    near(
        r.snapshots[12]["shapes"][0]["transform"]["rotation"]
            .as_f64()
            .unwrap(),
        0.25,
    );
    let src = body(&format!("{SQ}scene \"a\" for 2.seconds:\n    main:\n        together:\n            s.rotate_by(90.deg) over 0.5.seconds ease linear\n            s.rotate_by(90.deg) over 0.5.seconds ease linear\n"));
    let r = go(&src);
    near(rotation(&r.snapshots[12]), 0.25);
    near(rotation(&r.snapshots[18]), 0.375); // the second one started at tick 12
    near(rotation(&r.snapshots[24]), 0.5);
}

// ---- the layer fold -------------------------------------------------------------------------

#[test]
fn the_fold_reproduces_the_spec_worked_example() {
    // spec 7.10: rest 520, a semantic override to 410, a wobble adding 4.2, then keyframes at weight 0.5 -> 397.1
    let mut c = vec![
        Contribution {
            layer: 3,
            mode: BlendMode::Override,
            weight: 0.5,
            priority: 0,
            start: 84,
            id: 3,
            value: 380.0,
        },
        Contribution {
            layer: 2,
            mode: BlendMode::Add,
            weight: 1.0,
            priority: 0,
            start: 0,
            id: 2,
            value: 4.2,
        },
        Contribution {
            layer: 1,
            mode: BlendMode::Override,
            weight: 1.0,
            priority: 0,
            start: 0,
            id: 1,
            value: 410.0,
        },
    ];
    assert!((fold(520.0, &mut c) - 397.1).abs() < 1e-9);
    // without the keyframe set the result is 414.2
    let mut c2 = vec![c[0].clone(), c[1].clone()]; // `fold` sorted `c` by layer: [semantic, wobble, keyframes]
    assert!((fold(520.0, &mut c2) - 414.2).abs() < 1e-9);
    // ties inside a layer: higher priority applies later (wins), then later start, then higher id
    let mk = |p: i64, s: i64, id: u32, v: f64| Contribution {
        layer: 1,
        mode: BlendMode::Override,
        weight: 1.0,
        priority: p,
        start: s,
        id,
        value: v,
    };
    assert_eq!(fold(0.0, &mut [mk(5, 0, 1, 1.0), mk(0, 9, 2, 2.0)]), 1.0);
    assert_eq!(fold(0.0, &mut [mk(0, 1, 1, 1.0), mk(0, 2, 2, 2.0)]), 2.0);
    assert_eq!(fold(0.0, &mut [mk(0, 1, 9, 1.0), mk(0, 1, 3, 2.0)]), 1.0);
    // multiply scales the running value
    let mut m = vec![Contribution {
        layer: 2,
        mode: BlendMode::Multiply,
        weight: 0.5,
        priority: 0,
        start: 0,
        id: 1,
        value: 3.0,
    }];
    assert_eq!(fold(10.0, &mut m), 20.0); // v * lerp(1, 3, 0.5)
}

// ---- keyframes ------------------------------------------------------------------------------

#[test]
fn keyframes_on_a_vertex_component_interpolate_and_then_let_go() {
    let src = body(&format!(
        "{SQ}scene \"k\" for 2.seconds:\n    main:\n        s.v[2].keyframes over 1.second:\n            at 0.s: position.y = 0\n            at 0.5.s: position.y = 100 ease linear\n            at 1.s: position.y = 60\n"
    ));
    let y = ys(&go(&src), 2);
    near(y[0], 0.0);
    near(y[6], 50.0);
    near(y[12], 100.0);
    near(y[18], 80.0);
    near(y[24], 60.0); // the last key tick is still inside the range
    near(y[25], 50.0); // afterwards the layer below shows through again: the rest value
    near(vtx(&go(&src).snapshots[6], 0)[&2].0, -50.0); // the x component is untouched
}

#[test]
fn keyframes_only_contribute_inside_their_range_and_can_hold() {
    let src = |extra: &str| {
        body(&format!(
            "{SQ}scene \"k\" for 3.seconds:\n    main:\n        wait 0.5.seconds\n        s.v[2].keyframes over 1.second:\n            at 0.5.s: position.y = 100\n            at 1.s: position.y = 60\n{extra}"
        ))
    };
    let y = ys(&go(&src("")), 2);
    near(y[11], 50.0);
    near(y[12], 50.0); // starts at tick 12, but the first key is half a second later
    near(y[24], 100.0);
    near(y[36], 60.0);
    near(y[37], 50.0);
    let y = ys(
        &go(&src("            hold_before\n            hold_after\n")),
        2,
    );
    near(y[12], 100.0); // hold_before: the first key's value from the start of the statement
    near(y[60], 60.0); // hold_after: the last key's value for good
}

#[test]
fn blend_in_and_blend_out_soften_the_edges() {
    let src = body(&format!(
        "{SQ}scene \"k\" for 2.seconds:\n    main:\n        s.v[2].keyframes over 1.second:\n            blend_in: 12f\n            blend_out: 12f\n            at 0.s: position.y = 150\n            at 1.s: position.y = 150\n"
    ));
    let y = ys(&go(&src), 2);
    near(y[0], 50.0); // weight 0
    near(y[6], 100.0); // weight 0.5: halfway between the rest value 50 and 150
    near(y[12], 150.0);
    near(y[18], 100.0);
    near(y[24], 50.0);
}

#[test]
fn additive_keys_work_on_a_whole_selection() {
    let src = body(&format!(
        "{SQ}scene \"k\" for 2.seconds:\n    main:\n        s.vertices.keyframes over 1.second:\n            at 0.s: position.y += 0\n            at 0.5.s: position.y += 10 ease linear\n            at 1.s: position.y += 0\n"
    ));
    let r = go(&src);
    for (id, rest) in [(1u64, -50.0), (2, 50.0), (3, 50.0), (4, -50.0)] {
        let y = ys(&r, id);
        near(y[6], rest + 5.0);
        near(y[12], rest + 10.0);
        near(y[30], rest);
    }
}

#[test]
fn keyframes_on_the_shape_transform() {
    let src = body(&format!(
        "{SQ}scene \"k\" for 2.seconds:\n    main:\n        s.keyframes over 1.second:\n            at 0.s: rotation = 0.deg\n            at 1.s: rotation = 90.deg\n            at 0.s: scale = 1\n            at 0.5.s: scale = 2 ease linear\n            at 1.s: position = (100, 0)\n"
    ));
    let r = go(&src);
    let t = |k: usize| &r.snapshots[k]["shapes"][0]["transform"];
    near(t(12)["rotation"].as_f64().unwrap(), 0.125);
    near2(pair(&t(6)["scale"]), (1.5, 1.5));
    near2(pair(&t(12)["scale"]), (2.0, 2.0));
    near2(pair(&t(24)["position"]), (100.0, 0.0));
    near(t(30)["rotation"].as_f64().unwrap(), 0.0); // let go
}

#[test]
fn keyframes_sit_above_actions_and_revert_to_the_action_value() {
    let src = body(&format!(
        "{SQ}scene \"k\" for 2.seconds:\n    main:\n        together:\n            s.v[1].move_by(0, 100) over 1.second ease linear\n            s.v[1].keyframes over 0.5.seconds:\n                at 0.s: position.y = 0\n                at 0.5.s: position.y = 0\n"
    ));
    let r = go(&src);
    // the second statement needs the vertex channel? no: keyframes are a layer, not an action, so it starts at once
    let y = ys(&r, 1);
    near(y[3], 0.0); // keyframes (layer 3) win while active
    near(y[12], 0.0); // last key tick
    near(y[13], -50.0 + 100.0 * 13.0 / 24.0); // then the action shows again
    near(y[24], 50.0); // committed
}

#[test]
fn timeline_has_absolute_scene_times() {
    let src = body(&format!(
        "{SQ}scene \"t\" for 2.seconds:\n    timeline:\n        at 12f:\n            s.v[1].position.x = 0\n        at 24f:\n            s.v[1].position.x = 100 ease linear\n"
    ));
    let r = go(&src);
    let x = |k: usize| vtx(&r.snapshots[k], 0)[&1].0;
    near(x(11), -50.0);
    near(x(12), 0.0);
    near(x(18), 50.0);
    near(x(24), 100.0);
    near(x(25), -50.0);
}

#[test]
fn timeline_can_key_the_shape_transform() {
    let src = body(&format!(
        "{SQ}scene \"t\" for 2.seconds:\n    timeline:\n        at 12f:\n            s.rotation = 0.deg\n        at 24f:\n            s.rotation = 90.deg\n"
    ));
    let r = go(&src);
    near(rotation(&r.snapshots[11]), 0.0);
    near(rotation(&r.snapshots[18]), 0.125);
    near(rotation(&r.snapshots[24]), 0.25);
    near(rotation(&r.snapshots[25]), 0.0);
}

#[test]
fn timeline_supports_additive_keys_and_inline_entries() {
    let src = body(&format!(
        "{SQ}scene \"t\" for 2.seconds:\n    timeline:\n        at 0f: s.vertices.position.y += 0\n        at 1.second: s.vertices.position.y += 20\n"
    ));
    let r = go(&src);
    near(ys(&r, 1)[12], -50.0 + 10.0);
    near(ys(&r, 3)[24], 50.0 + 20.0);
}

#[test]
fn keyframe_errors() {
    let main = |lines: &str| {
        body(&format!("{SQ}scene \"k\" for 2.seconds:\n    main:\n        s.v[2].keyframes over 1.second:\n{lines}"))
    };
    // two keys for one component at the same time
    let d = run_err(&main(
        "            at 0.s: position.y = 0\n            at 0.s: position.y = 10\n",
    ));
    assert_eq!(d.code, "E-PRP-003");
    assert!(!d.helps.is_empty());
    // `=` and `+=` cannot be mixed for one property
    assert_eq!(
        run_err(&main(
            "            at 0.s: position.y = 0\n            at 1.s: position.y += 10\n"
        ))
        .code,
        "E-TYP-001"
    );
    // a key beyond the `over` duration
    assert_eq!(
        run_err(&main("            at 2.s: position.y = 0\n")).code,
        "E-TYP-001"
    );
    // unknown property, wrong unit
    assert_eq!(
        run_err(&main("            at 0.s: position.q = 0\n")).code,
        "E-NAM-003"
    );
    assert_eq!(
        run_err(&main("            at 0.s: position.y = 1.second\n")).code,
        "E-TYP-002"
    );
    assert_eq!(
        run_err(&main("            at 0.s: rotation = 90\n")).code,
        "E-NAM-003"
    ); // a vertex has no rotation
    assert_eq!(
        run_err(&main("            at 0.s: position = 5\n")).code,
        "E-TYP-001"
    );
    // build-time keyframes make no sense
    assert_eq!(
        load_err(&format!(
            "{SQ}s.v[2].keyframes over 1.second:\n    at 0.s: position.y = 0\n"
        ))
        .code,
        "E-SHP-005"
    );
    // a timeline needs a target
    let d = run_err(&body(&format!("{SQ}scene \"t\" for 1.second:\n    timeline:\n        at 0f:\n            position.y = 0\n")));
    assert_eq!(d.code, "E-TYP-001");
    // and the same time twice is also an error there
    let d = run_err(&body(&format!("{SQ}scene \"t\" for 1.second:\n    timeline:\n        at 0f: s.v[1].position.y = 0\n        at 0f: s.v[1].position.y = 5\n")));
    assert_eq!(d.code, "E-PRP-003");
}

#[test]
fn keyframed_runs_are_deterministic() {
    let src = body(&format!(
        "{SQ}scene \"k\" for 2.seconds:\n    main:\n        together:\n            s.rotate_by(90.deg) over 1.second\n            s.vertices.keyframes over 1.second:\n                blend_in: 4f\n                at 0.s: position.y += 0\n                at 0.5.s: position.y += 7 ease ease_out\n                at 1.s: position.y += 0\n"
    ));
    assert_eq!(go(&src).hashes, go(&src).hashes);
}

// ---- radial_array ---------------------------------------------------------------------------

const WEDGE: &str = "shape w = polygon([(400, 300), (500, 300), (400 + 100 * cos(45.deg), 300 + 100 * sin(45.deg))])\n";

#[test]
fn radial_array_in_the_language() {
    let it = ok(&format!(
        "{WEDGE}w.radial_array(count: 8, around: (400, 300), angle: 360.deg)\n"
    ));
    let s = &it.shapes["w"];
    assert_eq!(
        (
            s.live_ids(Kind::V).len(),
            s.live_ids(Kind::E).len(),
            s.live_ids(Kind::F).len()
        ),
        (9, 16, 8)
    );
    // the angle defaults to a full turn
    let it = ok(&format!("{WEDGE}w.radial_array(8, (400, 300))\n"));
    assert_eq!(it.shapes["w"].live_ids(Kind::F).len(), 8);
    // an arc, and a selection of faces
    let it = ok(&format!(
        "{WEDGE}w.radial_array(count: 3, around: (400, 300), angle: 90.deg)\n"
    ));
    assert_eq!(it.shapes["w"].live_ids(Kind::F).len(), 3);
    let it = ok("shape s = square(size: 200)\ns.e3.extrude(100)\ns.radial_array(2, (0, 0), 180.deg, faces: s.faces.every(2, offset: 1))\n");
    assert_eq!(it.shapes["s"].live_ids(Kind::F).len(), 3);
}

#[test]
fn radial_array_errors_and_the_runtime_split() {
    assert_eq!(
        load_err(&format!("{WEDGE}w.radial_array(8)\n")).code,
        "E-TYP-003"
    );
    assert_eq!(
        load_err(&format!("{WEDGE}w.radial_array(1, (0, 0))\n")).code,
        "E-SHP-005"
    );
    assert_eq!(
        load_err(&format!("{WEDGE}w.radial_array(4, (0, 0), faces: 5)\n")).code,
        "E-TYP-001"
    );
    let src = body(&format!(
        "{WEDGE}scene \"x\" for 1.second:\n    main:\n        w.radial_array(4, (400, 300))\n"
    ));
    let d: Vec<_> = check(&src)
        .into_iter()
        .filter(|d| d.code == "E-SHP-003")
        .collect();
    assert_eq!(d.len(), 1);
    assert_eq!(run_err(&src).code, "E-SHP-003");
}

// ---- the pinwheel example -------------------------------------------------------------------

fn example(name: &str) -> String {
    std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

#[test]
fn the_pinwheel_example_uses_radial_array_colour_transform_and_keyframes() {
    let src = example("pinwheel.anim");
    assert!(check(&src).is_empty(), "{:?}", check(&src));
    let r = go(&src);
    assert_eq!(r.snapshots.len(), 96);
    let sh = &r.snapshots[0]["shapes"][0];
    assert_eq!(sh["faces"].as_array().unwrap().len(), 8);
    let fills: std::collections::BTreeSet<String> = sh["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["fill"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(fills.len(), 2); // alternating colours
    near(rotation(&r.snapshots[48]), 0.5);
    let scale = |k: usize| pair(&r.snapshots[k]["shapes"][0]["transform"]["scale"]).0;
    near(scale(0), 1.0);
    assert!(scale(48) > 1.2);
    assert_eq!(r.hashes, go(&src).hashes);
}

#[test]
fn the_leaf_example_colours_the_inner_face_differently() {
    let src = example("leaf.anim");
    assert!(check(&src).is_empty());
    let r = go(&src);
    let leaf = r.snapshots[0]["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "leaf")
        .unwrap()
        .clone();
    let inner = leaf["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == 1)
        .unwrap()
        .clone();
    let ring = leaf["faces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] != 1)
        .unwrap()
        .clone();
    assert_ne!(inner["fill"], ring["fill"]);
}
