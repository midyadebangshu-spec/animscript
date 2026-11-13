//! Colour animation, procedural modifiers (wobble, noise), the spring easing, physics-style actions
//! (fall, bounce_in) and start / cancel / await handles. Written before the implementation.
use animscript_lang::color::{to_linear, to_srgb8};
use animscript_lang::procedural::{hash01, stream_seed, value_noise};
use animscript_lang::*;
use serde_json::Value;
use std::collections::BTreeMap;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";
const SQ: &str = "shape s = square(size: 100)\n";

fn body(s: &str) -> String {
    format!("{PROJECT}{s}")
}
fn go(src: &str) -> RunResult {
    match run(src, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(src), "test")),
    }
}
fn run_err(src: &str) -> Diagnostic {
    match run(src, &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn load_err(src: &str) -> Diagnostic {
    match load(src) {
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
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
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
fn ys(r: &RunResult, id: u64) -> Vec<f64> {
    r.snapshots.iter().map(|s| vtx(s, 0)[&id].1).collect()
}
fn xs(r: &RunResult, id: u64) -> Vec<f64> {
    r.snapshots.iter().map(|s| vtx(s, 0)[&id].0).collect()
}
fn fill(r: &RunResult, k: usize) -> Value {
    r.snapshots[k]["shapes"][0]["faces"][0]["fill"].clone()
}
fn scene(main: &str, secs: u32) -> String {
    format!("scene \"a\" for {secs}.seconds:\n    main:\n{main}")
}

// ---- colour space and noise primitives ------------------------------------------------------

#[test]
fn srgb_and_linear_light_round_trip_exactly() {
    for v in 0..=255u8 {
        assert_eq!(to_srgb8(to_linear([v, v, v, 255]))[0], v);
    }
    let l = to_linear([0x80, 0x80, 0x80, 0x80]);
    assert!((l[0] - 0.2158605).abs() < 1e-6);
    assert!((l[3] - 128.0 / 255.0).abs() < 1e-12); // alpha is already linear
                                                   // the linear-light midpoint of black and white is 0.5, which is sRGB 188 (#bc)
    assert_eq!(to_srgb8([0.5, 0.5, 0.5, 1.0]), [188, 188, 188, 255]);
}

#[test]
fn noise_is_a_pure_deterministic_function_of_stream_and_time() {
    let s = stream_seed(7, "scene/shape/v1/wobble#0/y");
    assert_eq!(s, stream_seed(7, "scene/shape/v1/wobble#0/y"));
    assert_ne!(s, stream_seed(8, "scene/shape/v1/wobble#0/y")); // the project seed matters
    assert_ne!(s, stream_seed(7, "scene/shape/v2/wobble#0/y")); // so does the stream name
    let h = hash01(s, 3);
    assert!((0.0..1.0).contains(&h) && h == hash01(s, 3));
    for i in 0..200 {
        let t = i as f64 * 0.137;
        let n = value_noise(s, t);
        assert!((-1.0..=1.0).contains(&n), "{n}");
        assert_eq!(n, value_noise(s, t)); // random access: no hidden state
    }
    // continuous, and equal to the lattice value at whole numbers
    assert!((value_noise(s, 2.5) - value_noise(s, 2.5 + 1e-9)).abs() < 1e-6);
    assert_eq!(value_noise(s, 4.0), hash01(s, 4) * 2.0 - 1.0);
}

#[test]
fn noise_and_random_builtins() {
    ok("assert noise(\"a\", 3) == noise(\"a\", 3)\n\
        assert abs(noise(\"a\", 2.5)) <= 1\n\
        assert noise(\"a\", 2.5) != noise(\"b\", 2.5)\n\
        assert random(\"a\", 1) != random(\"a\", 2)\n\
        assert random(\"a\") >= 0 and random(\"a\") < 1\n\
        assert random(\"a\") == random(\"a\", 0)\n");
    let a = ok("project \"p\":\n    anim_version: \"0.1\"\n    canvas: 10x10\n    fps: 24\n    seed: 1\nlet n = noise(\"a\", 2.5)\nlet r = random(\"a\")\n");
    let b = ok("project \"p\":\n    anim_version: \"0.1\"\n    canvas: 10x10\n    fps: 24\n    seed: 2\nlet n = noise(\"a\", 2.5)\nlet r = random(\"a\")\n");
    let (ga, gb) = (&a.globals, &b.globals);
    let num = |v: &animscript_lang::value::Value| v.as_f64().unwrap();
    assert_ne!(num(&ga["n"]), num(&gb["n"]));
    assert_ne!(num(&ga["r"]), num(&gb["r"]));
}

// ---- wobble ---------------------------------------------------------------------------------

#[test]
fn wobble_adds_a_sine_offset_on_top_of_the_rest_value() {
    let src = body(&format!(
        "{SQ}{}",
        scene("        s.v[2].wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0) over 2.seconds\n", 3)
    ));
    let r = go(&src);
    let y = ys(&r, 2);
    near(y[0], 50.0);
    near(y[6], 60.0); // sin(pi / 2)
    near(y[12], 50.0);
    near(y[18], 40.0); // sin(3 pi / 2)
    near(y[48], 50.0); // over: the modifier lets go, the rest value shows again
    near(y[60], 50.0);
    near(xs(&r, 2)[6], -50.0); // axis: y leaves x alone
}

#[test]
fn wobble_on_both_axes_and_blend_in_out() {
    let both = go(&body(&format!(
        "{SQ}{}",
        scene(
            "        s.v[2].wobble(amount: 10, speed: 1, sync: true, phase: 0) over 2.seconds\n",
            3
        )
    )));
    near(vtx(&both.snapshots[6], 0)[&2].0, -40.0);
    near(vtx(&both.snapshots[6], 0)[&2].1, 60.0);
    let blended = go(&body(&format!(
        "{SQ}{}",
        scene("        s.v[2].wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0, blend_in: 0.5.seconds, blend_out: 0.5.seconds) over 2.seconds\n", 3)
    )));
    let y = ys(&blended, 2);
    near(y[6], 55.0); // weight 0.5 on the way in
    near(y[12], 50.0);
    near(y[42], 45.0); // weight 0.5 on the way out: -10 * 0.5
}

#[test]
fn each_vertex_gets_its_own_phase_unless_sync_is_set() {
    let indep = go(&body(&format!(
        "{SQ}{}",
        scene(
            "        s.vertices.wobble(amount: 10, speed: 1, axis: y) over 2.seconds\n",
            3
        )
    )));
    assert_ne!(
        ys(&indep, 1)[6] + 50.0,
        ys(&indep, 3)[6] - 50.0,
        "independent phases"
    );
    let synced = go(&body(&format!("{SQ}{}", scene("        s.vertices.wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0) over 2.seconds\n", 3))));
    near(ys(&synced, 1)[6] + 50.0, ys(&synced, 3)[6] - 50.0);
}

#[test]
fn wobble_on_shape_properties() {
    let rot = go(&body(&format!(
        "{SQ}{}",
        scene("        s.wobble(property: rotation, amount: 36.deg, speed: 1, sync: true, phase: 0) over 2.seconds\n", 3)
    )));
    near(
        rot.snapshots[6]["shapes"][0]["transform"]["rotation"]
            .as_f64()
            .unwrap(),
        0.1,
    );
    let scale = go(&body(&format!(
        "{SQ}{}",
        scene("        s.wobble(property: scale, amount: 0.5, speed: 1, sync: true, phase: 0) over 2.seconds\n", 3)
    )));
    near(
        scale.snapshots[6]["shapes"][0]["transform"]["scale"][0]
            .as_f64()
            .unwrap(),
        1.5,
    );
    near(
        scale.snapshots[6]["shapes"][0]["transform"]["scale"][1]
            .as_f64()
            .unwrap(),
        1.5,
    );
    let op = go(&body(&format!(
        "{SQ}{}",
        scene("        s.wobble(property: opacity, amount: 0.25, speed: 1, sync: true, phase: 0) over 2.seconds\n", 3)
    )));
    near(
        op.snapshots[18]["shapes"][0]["opacity"].as_f64().unwrap(),
        0.75,
    );
}

#[test]
fn modifiers_sit_between_actions_and_keyframes() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        together:\n            s.v[2].move_by(0, 20) over 1.second ease linear\n            s.v[2].wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0) over 1.second\n",
            2
        )
    ));
    near(ys(&go(&src), 2)[6], 50.0 + 5.0 + 10.0); // rest + action + wobble
    let keyed = body(&format!(
        "{SQ}{}",
        scene(
            "        together:\n            s.v[2].move_by(0, 20) over 1.second ease linear\n            s.v[2].wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0) over 1.second\n            s.v[2].keyframes over 1.second:\n                at 0.s: position.y = 0\n                at 1.s: position.y = 0\n",
            2
        )
    ));
    near(ys(&go(&keyed), 2)[6], 0.0); // keyframes override everything below them
}

#[test]
fn unrelated_modifiers_never_change_another_modifiers_numbers() {
    // spec test T14: adding something else must not change the random numbers of an existing modifier
    let one = body(&format!(
        "{SQ}{}",
        scene(
            "        s.v[1].wobble(amount: 8, speed: 2, axis: y) over 2.seconds\n",
            3
        )
    ));
    let two = body(&format!(
        "{SQ}shape other = square(size: 10)\n{}",
        scene(
            "        together:\n            s.v[3].wobble(amount: 5, speed: 3, axis: x) over 1.second\n            other.vertices.noise(amount: 2, speed: 1) over 1.second\n            s.v[1].wobble(amount: 8, speed: 2, axis: y) over 2.seconds\n",
            3
        )
    ));
    assert_eq!(ys(&go(&one), 1), ys(&go(&two), 1));
}

// ---- noise ----------------------------------------------------------------------------------

#[test]
fn noise_is_smooth_bounded_deterministic_and_not_constant() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        s.v[2].noise(amount: 10, speed: 3, axis: y, sync: true) over 2.seconds\n",
            3
        )
    ));
    let a = go(&src);
    let b = go(&src);
    assert_eq!(a.hashes, b.hashes);
    let off: Vec<f64> = ys(&a, 2).iter().take(48).map(|y| y - 50.0).collect();
    assert!(off.iter().all(|o| o.abs() <= 10.0 + 1e-9));
    let (lo, hi) = (
        off.iter().cloned().fold(f64::MAX, f64::min),
        off.iter().cloned().fold(f64::MIN, f64::max),
    );
    assert!(hi - lo > 3.0, "{lo} {hi}");
    assert!(
        off.windows(2).all(|w| (w[1] - w[0]).abs() < 4.0),
        "continuous"
    );
    near(ys(&a, 2)[49], 50.0);
}

// ---- modifier errors ------------------------------------------------------------------------

#[test]
fn modifier_errors() {
    let m = |call: &str| body(&format!("{SQ}{}", scene(&format!("        {call}\n"), 2)));
    assert_eq!(
        run_err(&m("s.v[2].wobble(speed: 1) over 1.second")).code,
        "E-TYP-003"
    ); // amount is required
    let d = run_err(&m("s.v[2].wobble(amount: 5)")); // open-ended without start
    assert_eq!(d.code, "E-TYP-003");
    assert!(!d.helps.is_empty());
    assert_eq!(
        run_err(&m("s.v[2].wobble(amount: 5, axis: \"z\") over 1.second")).code,
        "E-TYP-001"
    );
    assert_eq!(
        run_err(&m("s.wobble(property: rotation, amount: 5) over 1.second")).code,
        "E-TYP-002"
    ); // a bare number is not an Angle
    assert_eq!(
        run_err(&m(
            "s.wobble(property: \"colour\", amount: 5) over 1.second"
        ))
        .code,
        "E-TYP-001"
    );
    assert_eq!(
        run_err(&m("s.wobble(amount: 5) over 1.second")).code,
        "E-TYP-003"
    ); // a shape needs `property:`
    assert_eq!(
        run_err(&m(
            "s.v[2].wobble(amount: 5, property: scale) over 1.second"
        ))
        .code,
        "E-TYP-001"
    );
    assert_eq!(
        run_err(&m("s.v[2].wobble(amount: 1.second) over 1.second")).code,
        "E-TYP-002"
    );
}

// ---- spring easing --------------------------------------------------------------------------

#[test]
fn spring_easing_overshoots_and_lands_exactly() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        s.move_by(100, 0) over 1.second ease spring(2, 4)\n",
            2
        )
    ));
    let r = go(&src);
    let px = |k: usize| {
        r.snapshots[k]["shapes"][0]["transform"]["position"][0]
            .as_f64()
            .unwrap()
    };
    near(px(0), 0.0);
    // f(p) = (1 - exp(-d p) cos(2 pi n p)) / (1 - exp(-d)) with n = 2 bounces and damping d = 4; p = 6 / 24
    let f = |p: f64| {
        (1.0 - (-4.0 * p).exp() * (std::f64::consts::TAU * 2.0 * p).cos()) / (1.0 - (-4.0f64).exp())
    };
    near(px(6), 100.0 * f(0.25));
    assert!(px(6) > 100.0, "it overshoots");
    assert!((0..24).map(px).fold(f64::MIN, f64::max) > 100.0);
    near(px(24), 100.0); // committed exactly
    ok("let e = spring(3, 5)\nlet d = spring()\n");
    assert_eq!(load_err("let e = spring(2, 0)\n").code, "E-TYP-001");
    assert_eq!(load_err("let e = spring(2, -1)\n").code, "E-TYP-001");
}

// ---- physics-style actions ------------------------------------------------------------------

#[test]
fn fall_follows_gravity_and_stops_at_the_floor() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        s.v[1].fall(floor: 100, gravity: 100) over 3.seconds\n",
            4
        )
    ));
    let r = go(&src);
    let y = ys(&r, 1);
    near(y[0], -50.0);
    near(y[12], -50.0 + 0.5 * 100.0 * 0.25); // t = 0.5 s
    near(y[24], 0.0); // t = 1 s
    near(y[47], 100.0); // landed at t = sqrt(3) s, a bounce of 0 stays on the floor
    near(y[72], 100.0); // committed
    near(xs(&r, 1)[40], -50.0);
}

#[test]
fn fall_bounces_with_restitution_and_moves_sideways() {
    let src = body(&format!("{SQ}{}", scene("        s.v[1].fall(floor: 100, gravity: 100, bounce: 0.5, velocity: (30, 0)) over 4.seconds\n", 5)));
    let r = go(&src);
    let tau = 3f64.sqrt(); // time of the first landing
    let vb = 100.0 * tau * 0.5; // speed after the bounce
    let t = 62.0 / 24.0;
    let tp = t - tau;
    near(ys(&r, 1)[62], 100.0 - vb * tp + 0.5 * 100.0 * tp * tp);
    assert!(ys(&r, 1)[62] < 100.0, "airborne after the first bounce");
    near(xs(&r, 1)[24], -50.0 + 30.0);
}

#[test]
fn bounce_in_reflects_off_the_walls() {
    let src = body(&format!("{SQ}{}", scene("        s.v[1].bounce_in(rect(-60, -100, 120, 200), velocity: (60, 0)) over 4.seconds\n", 5)));
    let r = go(&src);
    near(xs(&r, 1)[24], 10.0); // -50 + 60 * 1
    near(xs(&r, 1)[48], 50.0); // hit x = 60 at 1.83 s and came back
    near(ys(&r, 1)[48], -50.0); // no vertical speed
    assert!(xs(&r, 1)
        .iter()
        .all(|x| *x >= -60.0 - 1e-9 && *x <= 60.0 + 1e-9));
}

#[test]
fn physics_errors() {
    let m = |call: &str| body(&format!("{SQ}{}", scene(&format!("        {call}\n"), 2)));
    assert_eq!(
        run_err(&m("s.v[1].fall(floor: 100, bounce: 1.5) over 1.second")).code,
        "E-TYP-001"
    );
    assert_eq!(run_err(&m("s.v[1].fall(floor: 100)")).code, "E-TYP-003"); // needs `over`
    assert_eq!(
        run_err(&m("s.v[1].bounce_in(5) over 1.second")).code,
        "E-TYP-001"
    );
    assert_eq!(load_err("let r = rect(1, 2, 3)\n").code, "E-TYP-003");
}

// ---- handles: start, cancel, await ----------------------------------------------------------

#[test]
fn start_runs_a_modifier_open_ended_until_it_is_cancelled() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        let h = start s.v[2].wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0)\n        wait 1.second\n        cancel h\n        wait 1.second\n",
            3
        )
    ));
    let y = ys(&go(&src), 2);
    near(y[6], 60.0);
    near(y[18], 40.0);
    near(y[30], 50.0); // cancelled at tick 24: it would have been 60 again
    let forever = body(&format!(
        "{SQ}{}",
        scene("        let h = start s.v[2].wobble(amount: 10, speed: 1, axis: y, sync: true, phase: 0)\n        wait 2.seconds\n", 3)
    ));
    near(
        ys(&go(&forever), 2)[30],
        50.0 + 10.0 * (std::f64::consts::TAU * 30.0 / 24.0).sin(),
    ); // still running
}

#[test]
fn start_does_not_block_and_await_waits_for_the_end() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        let h = start s.rotate_by(90.deg) over 1.second ease linear\n        s.v[2].move_by(0, 10) over 0.5.seconds ease linear\n        await h\n        s.v[3].move_by(0, 10) over 0.5.seconds ease linear\n",
            3
        )
    ));
    let r = run(
        &src,
        &RunOptions {
            trace: true,
            ..Default::default()
        },
    )
    .unwrap();
    let start_of = |what: &str| -> String {
        r.trace
            .iter()
            .find(|t| t.contains(" ACT+ ") && t.contains(what))
            .cloned()
            .unwrap_or_default()
    };
    assert!(start_of("rotate_by").contains("[0 -> 24]"), "{:?}", r.trace);
    assert!(
        start_of("s.v2.move_by").contains("[0 -> 12]"),
        "the move ran while the rotation ran: {:?}",
        r.trace
    );
    assert!(
        start_of("s.v3.move_by").contains("[24 -> 36]"),
        "await blocked until the rotation ended: {:?}",
        r.trace
    );
    let rot = |k: usize| {
        r.snapshots[k]["shapes"][0]["transform"]["rotation"]
            .as_f64()
            .unwrap()
    };
    near(rot(12), 0.125);
    near(rot(24), 0.25);
}

#[test]
fn start_without_a_binding_and_cancelling_an_action_keeps_its_last_value() {
    let src = body(&format!(
        "{SQ}{}",
        scene("        let h = start s.rotate_by(90.deg) over 1.second ease linear\n        wait 0.5.seconds\n        cancel h\n", 2)
    ));
    let r = go(&src);
    let rot = |k: usize| {
        r.snapshots[k]["shapes"][0]["transform"]["rotation"]
            .as_f64()
            .unwrap()
    };
    near(rot(12), 0.25 * 11.0 / 24.0); // the last sampled value was written to rest (spec 7.6)
    near(rot(40), 0.25 * 11.0 / 24.0);
    let bare = body(&format!("{SQ}{}", scene("        start s.rotate_by(90.deg) over 0.5.seconds ease linear\n        wait 1.second\n", 2)));
    near(
        go(&bare).snapshots[20]["shapes"][0]["transform"]["rotation"]
            .as_f64()
            .unwrap(),
        0.25,
    );
}

#[test]
fn handle_errors() {
    let m = |main: &str| body(&format!("{SQ}{}", scene(main, 2)));
    assert_eq!(run_err(&m("        cancel 5\n")).code, "E-TYP-001");
    assert_eq!(run_err(&m("        await 5\n")).code, "E-TYP-001");
    let d = run_err(&m(
        "        let h = start s.v[2].wobble(amount: 5)\n        await h\n",
    ));
    assert_eq!(d.code, "E-RUN-006");
    assert!(!d.helps.is_empty());
    // awaiting or cancelling something that already finished is fine
    go(&m("        let h = start s.rotate_by(90.deg) over 0.25.seconds\n        wait 1.second\n        await h\n        cancel h\n"));
}

// ---- colour animation -----------------------------------------------------------------------

#[test]
fn fill_to_blends_in_linear_light_and_commits() {
    let src = body(&format!(
        "{SQ}s.f1.fill = #000000\n{}",
        scene(
            "        s.f1.fill_to(#ffffff) over 1.second ease linear\n",
            2
        )
    ));
    let r = go(&src);
    assert_eq!(fill(&r, 0), "#000000");
    assert_eq!(fill(&r, 12), "#bcbcbc"); // not #808080: the blend happens in linear light
    assert_eq!(fill(&r, 24), "#ffffff");
    assert_eq!(fill(&r, 40), "#ffffff");
}

#[test]
fn animating_from_no_colour_fades_in_without_a_colour_shift() {
    let src = body(&format!(
        "{SQ}{}",
        scene(
            "        s.f1.fill_to(#ff0000) over 1.second ease linear\n",
            2
        )
    ));
    let r = go(&src);
    assert_eq!(fill(&r, 0), "#ff000000");
    assert_eq!(fill(&r, 12), "#ff000080");
    assert_eq!(fill(&r, 24), "#ff0000");
}

#[test]
fn shape_level_and_selection_colour_actions() {
    let shape = body(&format!(
        "{SQ}s.fill = #000000\ns.e3.extrude(100)\ns.f2.fill = #ff0000\n{}",
        scene("        s.fill_to(#ffffff) over 1.second ease linear\n", 2)
    ));
    let r = go(&shape);
    let face = |k: usize, i: usize| r.snapshots[k]["shapes"][0]["faces"][i]["fill"].clone();
    assert_eq!(face(12, 0), "#bcbcbc"); // f1 inherits the shape's colour
    assert_eq!(face(12, 1), "#ff0000"); // f2 has its own and is untouched
    let sel = body(&format!(
        "{SQ}s.fill = #000000\ns.e3.extrude(100)\n{}",
        scene(
            "        s.faces.fill_to(#ffffff) over 1.second ease linear\n",
            2
        )
    ));
    let r = go(&sel);
    let r2 = run(
        &sel,
        &RunOptions {
            trace: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r2.trace.iter().filter(|t| t.contains(" ACT+ ")).count(), 1); // one action for the whole selection
    assert_eq!(r.snapshots[12]["shapes"][0]["faces"][1]["fill"], "#bcbcbc");
}

#[test]
fn stroke_and_width_actions() {
    let src = body(&format!(
        "{SQ}s.stroke = #000000\ns.stroke_width = 2\n{}",
        scene("        together:\n            s.e1.stroke_to(#ffffff) over 1.second ease linear\n            s.e1.width_to(6) over 1.second ease linear\n            s.edges.every(2, offset: 1).width_to(10) over 1.second ease linear\n", 2)
    ));
    let r = go(&src);
    let edge = |k: usize, i: usize| r.snapshots[k]["shapes"][0]["edges"][i].clone();
    assert_eq!(edge(12, 0)["stroke"], "#bcbcbc");
    near(edge(12, 0)["stroke_width"].as_f64().unwrap(), 4.0);
    near(edge(12, 1)["stroke_width"].as_f64().unwrap(), 6.0); // e2 is in the selection: 2 -> 10
    near(edge(12, 2)["stroke_width"].as_f64().unwrap(), 2.0);
    near(edge(24, 0)["stroke_width"].as_f64().unwrap(), 6.0);
}

#[test]
fn opacity_and_tint() {
    let fade = body(&format!(
        "{SQ}s.fill = #ff0000\n{}",
        scene("        s.fade_to(0) over 1.second ease linear\n", 2)
    ));
    let r = go(&fade);
    assert_eq!(r.snapshots[12]["shapes"][0]["opacity"], 0.5);
    assert_eq!(fill(&r, 12), "#ff000080");
    assert_eq!(
        r.snapshots[12]["shapes"][0]["edges"][0]["stroke"],
        "#22222280"
    ); // opacity covers strokes too
    near(
        r.snapshots[24]["shapes"][0]["opacity"].as_f64().unwrap(),
        0.0,
    );
    let tint = go(&body(&format!(
        "{SQ}s.fill = #ffffff\ns.tint = #ff8000\n{}",
        scene("        wait 1.second\n", 2)
    )));
    assert_eq!(fill(&tint, 0), "#ff8000"); // white times the tint
    let tinted = go(&body(&format!(
        "{SQ}s.fill = #ffffff\n{}",
        scene("        s.tint_to(#0000ff) over 1.second ease linear\n", 2)
    )));
    assert_eq!(fill(&tinted, 12), "#bcbcff");
    assert_eq!(fill(&tinted, 0), "#ffffff");
    ok("shape s = square(size: 10)\ns.opacity = 0.5\ns.tint = #ff0000\nassert s.opacity == 0.5\nassert s.tint == #ff0000\n");
}

#[test]
fn zero_time_colour_calls() {
    let it = ok(&format!("{SQ}s.f1.fill_to(#ff0000)\ns.e1.stroke_to(#00ff00)\ns.e1.width_to(5)\ns.fade_to(0.25)\ns.tint_to(#0000ff)\ns.fill_to(#101010)\n"));
    let s = &it.shapes["s"];
    assert_eq!(s.faces[&1].fill, Some([255, 0, 0, 255]));
    assert_eq!(
        (s.edges[&1].stroke, s.edges[&1].width),
        (Some([0, 255, 0, 255]), Some(5.0))
    );
    assert_eq!(
        (s.opacity, s.tint, s.fill),
        (0.25, [0, 0, 255, 255], Some([16, 16, 16, 255]))
    );
    let r = go(&body(&format!(
        "{SQ}{}",
        scene(
            "        wait 0.5.seconds\n        s.f1.fill_to(#ff0000)\n",
            2
        )
    )));
    assert_eq!(fill(&r, 11), Value::Null);
    assert_eq!(fill(&r, 12), "#ff0000");
}

#[test]
fn colour_keyframes() {
    let src = body(&format!(
        "{SQ}{}",
        scene("        s.f1.keyframes over 1.second:\n            at 0.s: fill = #000000\n            at 1.s: fill = #ffffff\n", 2)
    ));
    let r = go(&src);
    assert_eq!(fill(&r, 0), "#000000");
    assert_eq!(fill(&r, 12), "#bcbcbc");
    assert_eq!(fill(&r, 30), Value::Null); // let go: back to no colour
    let shape = body(&format!(
        "{SQ}s.fill = #ff0000\n{}",
        scene("        s.keyframes over 1.second:\n            at 0.s: opacity = 1\n            at 1.s: opacity = 0\n            at 0.s: tint = #ffffff\n            at 1.s: tint = #00ff00\n            at 0.s: stroke_width = 2\n            at 1.s: stroke_width = 8\n", 2)
    ));
    let r = go(&shape);
    near(
        r.snapshots[12]["shapes"][0]["opacity"].as_f64().unwrap(),
        0.5,
    );
    near(
        r.snapshots[12]["shapes"][0]["edges"][0]["stroke_width"]
            .as_f64()
            .unwrap(),
        5.0,
    );
    let fill12 = r.snapshots[12]["shapes"][0]["faces"][0]["fill"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(fill12.ends_with("80"), "{fill12}"); // alpha 0.5
}

#[test]
fn colour_errors() {
    let m = |call: &str| body(&format!("{SQ}{}", scene(&format!("        {call}\n"), 2)));
    assert_eq!(
        load_err(&format!("{SQ}s.f1.fill_to(5)\n")).code,
        "E-TYP-001"
    );
    let d = load_err(&format!("{SQ}s.f1.fill_to(none)\n"));
    assert_eq!(d.code, "E-TYP-001");
    assert!(!d.helps.is_empty());
    assert_eq!(
        load_err(&format!("{SQ}s.e1.fill_to(#fff)\n")).code,
        "E-SHP-005"
    );
    assert_eq!(
        load_err(&format!("{SQ}s.f1.stroke_to(#fff)\n")).code,
        "E-SHP-005"
    );
    assert_eq!(load_err(&format!("{SQ}s.fade_to(2)\n")).code, "E-TYP-001");
    assert_eq!(
        load_err(&format!("{SQ}s.fade_to(1.second)\n")).code,
        "E-TYP-002"
    );
    assert_eq!(load_err(&format!("{SQ}s.opacity = 2\n")).code, "E-TYP-001");
    assert_eq!(
        run_err(&m(
            "s.f1.keyframes over 1.second:\n            at 0.s: fill += #ffffff"
        ))
        .code,
        "E-TYP-001"
    );
    assert_eq!(
        run_err(&m(
            "s.v[1].keyframes over 1.second:\n            at 0.s: fill = #ffffff"
        ))
        .code,
        "E-NAM-003"
    );
    assert_eq!(
        run_err(&m(
            "s.e1.keyframes over 1.second:\n            at 0.s: fill = #ffffff"
        ))
        .code,
        "E-SHP-005"
    );
}

#[test]
fn colour_actions_queue_on_one_face_but_run_beside_vertex_moves() {
    let src = body(&format!(
        "{SQ}s.f1.fill = #000000\n{}",
        scene("        together:\n            s.f1.fill_to(#ffffff) over 0.5.seconds ease linear\n            s.f1.fill_to(#ff0000) over 0.5.seconds ease linear\n            s.v[1].move_by(0, 10) over 0.5.seconds ease linear\n", 2)
    ));
    let r = go(&src);
    assert_eq!(fill(&r, 6), "#bcbcbc"); // the first half way
    assert_eq!(fill(&r, 12), "#ffffff"); // second starts here (p = 0): white
    assert_eq!(fill(&r, 24), "#ff0000");
    near(ys(&r, 1)[12], -40.0); // the vertex move was not held up
}

#[test]
fn animated_colours_and_modifiers_are_part_of_the_hash() {
    let mk = |c: &str| {
        body(&format!(
            "{SQ}s.f1.fill = #000000\n{}",
            scene(&format!("        s.f1.fill_to({c}) over 1.second\n"), 2)
        ))
    };
    assert_eq!(go(&mk("#ffffff")).hashes, go(&mk("#ffffff")).hashes);
    assert_ne!(go(&mk("#ffffff")).hashes[12], go(&mk("#fffffe")).hashes[12]);
}

// ---- the breathing example ------------------------------------------------------------------

#[test]
fn the_breathing_example_uses_modifiers_colour_and_springs() {
    let src = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/breathing.anim"),
    )
    .unwrap();
    assert!(check(&src).is_empty(), "{:?}", check(&src));
    let r = go(&src);
    assert_eq!(r.hashes, go(&src).hashes);
    assert_ne!(fill(&r, 0), fill(&r, 60));
    let y0 = vtx(&r.snapshots[0], 0);
    let y1 = vtx(&r.snapshots[40], 0);
    assert_ne!(y0, y1, "the petals move");
}
