//! User-defined `action`s and `fn`s, `while`, `return`, assignment to locals and `import`.
//! Written first.
use animscript_lang::*;
use serde_json::Value;
use std::path::Path;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";
const BASE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");

fn go(body: &str) -> RunResult {
    let text = format!("{PROJECT}{body}");
    match run_in(&text, &RunOptions::default(), Path::new(BASE)) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(body: &str) -> Diagnostic {
    match run_in(
        &format!("{PROJECT}{body}"),
        &RunOptions::default(),
        Path::new(BASE),
    ) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn verts(snap: &Value, shape: usize) -> Vec<(f64, f64)> {
    snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect()
}
fn centre(snap: &Value, shape: usize) -> (f64, f64) {
    let v = verts(snap, shape);
    (
        v.iter().map(|p| p.0).sum::<f64>() / v.len() as f64,
        v.iter().map(|p| p.1).sum::<f64>() / v.len() as f64,
    )
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
const SQ: &str = "shape s = square(size: 10, at: (0, 0))\n";
const SC: &str = "scene \"x\" for 2.frames:\n    main:\n        wait 2.frames\n";

#[test]
fn a_fn_returns_a_value_and_may_branch_and_recurse() {
    let r = go(&format!("fn at_least(x: Number, lo: Number) -> Number:\n    if x < lo:\n        return lo\n    return x\n\nfn fact(n: Int) -> Int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\n\n{SQ}s.move_to((at_least(3, 10), fact(5)))\n{SC}"));
    let c = centre(&r.snapshots[0], 0);
    near(c.0, 10.0, 1e-9);
    near(c.1, 120.0, 1e-9);
}

#[test]
fn while_and_assignment_to_a_local() {
    let r = go(&format!("{SQ}let n = 0\nlet total = 0\nwhile n < 4:\n    s.move_by(10, 0)\n    n += 1\n    total = total + n * 2\ns.move_by(0, total)\n{SC}"));
    let c = centre(&r.snapshots[0], 0);
    near(c.0, 40.0, 1e-9);
    near(c.1, 20.0, 1e-9); // 2 + 4 + 6 + 8
}

#[test]
fn a_while_loop_that_never_waits_is_stopped() {
    let e = err(&format!("{SQ}let n = 0\nwhile true:\n    n += 1\n{SC}"));
    assert_eq!(e.code, "E-STM-003");
}

#[test]
fn an_action_runs_like_a_builtin_and_blocks_the_caller() {
    let r = go(&format!("action slide(self: Shape, dx: Number, time: Duration = 1.second):\n    self.move_by(dx, 0) over time\n\n{SQ}scene \"x\" for 3.seconds:\n    main:\n        s.slide(100)\n        s.slide(50, time: 0.5.seconds)\n        wait 1.second\n"));
    let x = |t: usize| centre(&r.snapshots[t], 0).0;
    assert!(
        x(12) > 20.0 && x(12) < 80.0,
        "half way through the first: {}",
        x(12)
    );
    near(x(24), 100.0, 0.5);
    near(x(40), 150.0, 0.5); // the second one only started when the first had finished
    near(x(70), 150.0, 1e-6);
}

#[test]
fn start_await_and_cancel_work_on_user_actions() {
    let slide = "action slide(self: Shape, dx: Number, time: Duration = 1.second):\n    self.move_by(dx, 0) over time\n\n";
    let r = go(&format!("{slide}{SQ}scene \"x\" for 3.seconds:\n    main:\n        let h = start s.slide(100, time: 2.seconds)\n        wait 1.second\n        cancel h\n        wait 2.seconds\n"));
    let (a, b) = (centre(&r.snapshots[30], 0).0, centre(&r.snapshots[70], 0).0);
    assert!(a > 20.0 && a < 80.0, "it was stopped half way: {a}");
    near(a, b, 1e-6); // and stays there
    let r = go(&format!("{slide}{SQ}scene \"x\" for 3.seconds:\n    main:\n        let h = start s.slide(100, time: 1.second)\n        wait 0.25.seconds\n        await h\n        s.move_by(10, 0)\n        wait 1.second\n"));
    near(centre(r.snapshots.last().unwrap(), 0).0, 110.0, 0.5);
}

#[test]
fn user_actions_work_inside_together() {
    let r = go("action slide(self: Shape, dx: Number, time: Duration = 1.second):\n    self.move_by(dx, 0) over time\n\nshape a = square(size: 10, at: (0, 0))\nshape b = square(size: 10, at: (0, 50))\nscene \"x\" for 3.seconds:\n    main:\n        together:\n            a.slide(100, time: 1.second)\n            b.slide(40, time: 0.5.seconds)\n        a.move_by(0, 7)\n        wait 1.second\n");
    let last = r.snapshots.last().unwrap();
    near(centre(last, 0).0, 100.0, 0.5);
    near(centre(last, 0).1, 7.0, 0.5);
    near(centre(last, 1).0, 40.0, 0.5);
}

#[test]
fn defaults_may_use_earlier_parameters_and_calls_take_named_arguments() {
    let r = go(&format!("fn plus(a: Number, k: Number = a * 2) -> Number:\n    return a + k\n\n{SQ}s.move_to((plus(3), plus(3, 1)))\ns.move_by(plus(a: 2, k: 10), plus(k: 1, a: 1))\n{SC}"));
    let c = centre(&r.snapshots[0], 0);
    near(c.0, 9.0 + 12.0, 1e-9);
    near(c.1, 4.0 + 2.0, 1e-9);
}

#[test]
fn return_ends_an_action_early_and_actions_can_call_actions_and_fns() {
    let r = go(&format!("fn twice(x: Number) -> Number:\n    return x * 2\n\naction maybe(self: Shape, go: Bool):\n    if not go:\n        return\n    self.move_by(twice(25), 0)\n\naction both(self: Shape):\n    self.maybe(false)\n    self.maybe(true)\n    self.maybe(true)\n\n{SQ}s.both()\n{SC}"));
    near(centre(&r.snapshots[0], 0).0, 100.0, 1e-9);
}

#[test]
fn meta_lines_are_accepted_and_requires_is_checked() {
    let ok = go(&format!("action tap(self: Shape, n: Int = 1):\n    uses body\n    requires n > 0\n    priority 2\n    self.move_by(n, 0)\n\n{SQ}s.tap()\n{SC}"));
    near(centre(&ok.snapshots[0], 0).0, 1.0, 1e-9);
    assert_eq!(err(&format!("action tap(self: Shape, n: Int = 1):\n    requires n > 0\n    self.move_by(n, 0)\n\n{SQ}s.tap(0)\n{SC}")).code, "E-CAP-002");
}

#[test]
fn bad_calls_and_definitions_are_rejected() {
    let f = "fn plus(a: Number, k: Number = 1) -> Number:\n    return a + k\n\n";
    assert_eq!(
        err(&format!("{f}{SQ}s.move_to((plus(1, z: 2), 0))\n{SC}")).code,
        "E-TYP-003"
    ); // no such argument
    assert_eq!(
        err(&format!("{f}{SQ}s.move_to((plus(), 0))\n{SC}")).code,
        "E-TYP-003"
    ); // missing argument
    assert_eq!(
        err(&format!("{f}{SQ}s.move_to((plus(\"x\"), 0))\n{SC}")).code,
        "E-TYP-001"
    ); // wrong type
    assert_eq!(err(&format!("{f}{f}{SQ}{SC}")).code, "E-DEF-001"); // defined twice
    assert_eq!(
        err(&format!(
            "fn bad() -> Number:\n    return \"x\"\n\n{SQ}s.move_to((bad(), 0))\n{SC}"
        ))
        .code,
        "E-TYP-001"
    ); // wrong return type
    assert_eq!(err(&format!("{SQ}return\n{SC}")).code, "E-SYN-002"); // return outside a definition
    assert_eq!(err(&format!("fn slow() -> Number:\n    wait 1.second\n    return 1\n\n{SQ}s.move_to((slow(), 0))\n{SC}")).code, "E-DEF-002"); // a fn cannot take time
    assert_eq!(err(&format!("fn forever(n: Int) -> Int:\n    return forever(n + 1)\n\n{SQ}s.move_to((forever(0), 0))\n{SC}")).code, "E-RUN-007"); // too deep
    assert_eq!(err(&format!("action slide(self: Shape, dx: Number):\n    self.move_by(dx, 0)\n\n{SQ}slide(3, 4)\n{SC}")).code, "E-TYP-001"); // self must be a shape
    assert_eq!(err(&format!("{SQ}n = 3\n{SC}")).code, "E-NAM-001"); // assigning to a name that was never declared
}

#[test]
fn import_pulls_in_definitions_and_cycles_are_harmless() {
    let r = go(&format!(
        "import \"data/lib_test.anim\"\n{SQ}s.move_to((triple(4), 0))\ns.nudge()\n{SC}"
    ));
    near(centre(&r.snapshots[0], 0).0, 17.0, 1e-9);
    let r = go(&format!(
        "import \"data/lib_cycle_a.anim\"\n{SQ}s.move_to((from_a() + from_b(), 0))\n{SC}"
    ));
    near(centre(&r.snapshots[0], 0).0, 3.0, 1e-9);
    assert_eq!(
        err(&format!("import \"data/nope.anim\"\n{SQ}{SC}")).code,
        "E-AST-001"
    );
    assert_eq!(
        err(&format!("import \"std/nothing\"\n{SQ}{SC}")).code,
        "E-AST-001"
    );
}

#[test]
fn errors_in_your_own_file_keep_their_line_numbers_when_something_is_imported() {
    let e = err("import \"data/lib_test.anim\"\nshape s = square(size: 10)\nshape s = square(size: 10)\nscene \"x\" for 1.frame:\n    main:\n        wait 1.frame\n");
    assert_eq!(e.span.unwrap().line, 8); // the project block is 4 lines, a blank, the import, then the shapes
}

#[test]
fn the_standard_motion_library_ships_as_plain_animscript() {
    let r = go("import \"std/motion\"\nshape bar = rectangle(width: 100, height: 20, at: (200, 200))\nshape ball = square(size: 20, at: (400, 400))\nscene \"x\" for 3.seconds:\n    main:\n        bar.look_to((200, 300))\n        ball.shake(amount: 8, times: 4, over: 0.4.seconds)\n        ball.pulse(1.5, over: 0.4.seconds)\n        wait 1.second\n");
    let last = r.snapshots.last().unwrap();
    let ys: Vec<f64> = verts(last, 0).iter().map(|p| p.1).collect();
    let tall =
        ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
    near(tall, 100.0, 1.0); // the bar now points down at (200, 300): a quarter turn
    let c = centre(last, 1);
    near(c.0, 400.0, 1.5); // the shake returns to where it started
    near(c.1, 400.0, 1.5);
}

#[test]
fn programs_with_actions_check_cleanly_and_replay_identically() {
    let s = format!("action slide(self: Shape, dx: Number, time: Duration = 1.second):\n    self.move_by(dx, 0) over time\n\n{SQ}scene \"x\" for 2.seconds:\n    main:\n        s.slide(30)\n        s.slide(30)\n");
    assert_eq!(go(&s).hashes, go(&s).hashes);
    let diags = check_in(&format!("{PROJECT}{s}"), Path::new(BASE));
    assert!(diags.iter().all(|d| !d.is_error()), "{diags:?}");
}
