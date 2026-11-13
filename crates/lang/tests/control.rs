//! Round L1: `race:`, `finally:`, `derive`, `cubic_bezier`, timeline markers. Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";

fn go(s: &str) -> RunResult {
    let text = format!("{HEAD}{s}");
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(s: &str) -> Box<Diagnostic> {
    match run(&format!("{HEAD}{s}"), &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => Box::new(d),
    }
}
fn centre(snap: &Value, shape: usize) -> (f64, f64) {
    let v: Vec<(f64, f64)> = snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect();
    (
        v.iter().map(|p| p.0).sum::<f64>() / v.len() as f64,
        v.iter().map(|p| p.1).sum::<f64>() / v.len() as f64,
    )
}
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
const SH: &str = "shape a = square(size: 10, at: (0, 0))\nshape b = square(size: 10, at: (0, 50))\nshape c = square(size: 10, at: (0, 100))\n";

// ---- race -------------------------------------------------------------------------------------

#[test]
fn a_race_resumes_when_the_first_child_finishes_and_cancels_the_rest() {
    let r = go(&format!("{SH}scene \"s\" for 4.seconds:\n    main:\n        race:\n            a.move_to((100, 0)) over 1.second ease linear\n            b.move_to((100, 50)) over 2.seconds ease linear\n        c.move_to((100, 100)) over 1.second ease linear\n"));
    near(centre(&r.snapshots[24], 0).0, 100.0, 1e-6); // the winner arrived
    let b_at_cancel = centre(&r.snapshots[24], 1).0;
    near(b_at_cancel, 50.0, 3.0); // half way when cancelled
    near(centre(&r.snapshots[60], 1).0, b_at_cancel, 1e-6); // and it stays there
                                                            // the next statement starts when the first child is home (tick 24)
    assert!(centre(&r.snapshots[30], 2).0 > 10.0 && centre(&r.snapshots[23], 2).0 < 1e-6);
    near(centre(&r.snapshots[48], 2).0, 100.0, 1e-6);
}

#[test]
fn a_wait_can_win_a_race() {
    let r = go(&format!("{SH}scene \"s\" for 3.seconds:\n    main:\n        race:\n            wait 0.5.seconds\n            a.move_to((100, 0)) over 2.seconds ease linear\n        c.move_to((100, 100)) over 0.5.seconds ease linear\n"));
    near(centre(&r.snapshots[12], 0).0, 25.0, 3.0); // a was stopped a quarter of the way
    near(
        centre(&r.snapshots[30], 0).0,
        centre(&r.snapshots[12], 0).0,
        1e-6,
    );
    near(centre(&r.snapshots[24], 2).0, 100.0, 1e-6);
}

#[test]
fn a_tie_goes_to_the_first_child_and_nothing_breaks() {
    let r = go(&format!("{SH}scene \"s\" for 3.seconds:\n    main:\n        race:\n            a.move_to((100, 0)) over 1.second ease linear\n            b.move_to((100, 50)) over 1.second ease linear\n        c.move_to((100, 100)) over 1.second ease linear\n"));
    near(centre(&r.snapshots[24], 0).0, 100.0, 1e-6);
    near(centre(&r.snapshots[48], 2).0, 100.0, 1e-6);
}

// ---- finally ----------------------------------------------------------------------------------

#[test]
fn finally_runs_when_a_block_ends_normally() {
    let r = go(&format!("{SH}scene \"s\" for 2.seconds:\n    main:\n        wait 0.5.seconds\n        finally:\n            a.position = (40, 0)\n"));
    // `finally` is the last statement of main: it runs when main ends, after the wait
    near(centre(&r.snapshots[20], 0).0, 40.0, 1e-6);
}

#[test]
fn finally_runs_when_a_started_action_is_cancelled() {
    let r = go(&format!("{SH}action work(self: Shape):\n    self.move_by(100, 0) over 2.seconds ease linear\n    finally:\n        b.position = (33, 0)\n\nscene \"s\" for 2.seconds:\n    main:\n        let h = start a.work()\n        wait 0.5.seconds\n        cancel h\n        wait 1.second\n"));
    near(centre(&r.snapshots[11], 1).0, 0.0, 1e-6);
    near(centre(&r.snapshots[20], 1).0, 33.0, 1e-6);
}

#[test]
fn finally_runs_for_a_race_loser() {
    let r = go(&format!("{SH}action work(self: Shape):\n    self.move_by(100, 0) over 2.seconds ease linear\n    finally:\n        b.position = (21, 0)\n\nscene \"s\" for 2.seconds:\n    main:\n        race:\n            a.work()\n            wait 0.5.seconds\n        wait 1.second\n"));
    near(centre(&r.snapshots[20], 1).0, 21.0, 1e-6);
}

#[test]
fn finally_runs_when_the_scene_ends() {
    let r = go(&format!("{SH}action work(self: Shape):\n    wait 10.seconds\n    finally:\n        b.position = (17, 0)\n\nscene \"one\" for 1.second:\n    main:\n        start a.work()\n        wait 1.second\n\nscene \"two\" for 3.frames:\n    main:\n        wait 3.frames\n"));
    near(centre(&r.snapshots[23], 1).0, 0.0, 1e-6);
    near(centre(&r.snapshots[24], 1).0, 17.0, 1e-6); // scene two starts with the value the finally set
}

#[test]
fn return_runs_the_finally_of_the_action() {
    let r = go(&format!("{SH}action work(self: Shape):\n    if true:\n        return\n    wait 5.seconds\n    finally:\n        b.position = (9, 0)\n\nscene \"s\" for 3.frames:\n    main:\n        a.work()\n        wait 2.frames\n"));
    near(centre(&r.snapshots[2], 1).0, 9.0, 1e-6);
}

#[test]
fn finally_must_not_wait_and_must_be_last() {
    let e = err(&format!("{SH}action work(self: Shape):\n    wait 5.seconds\n    finally:\n        wait 1.second\n\nscene \"s\" for 1.second:\n    main:\n        let h = start a.work()\n        wait 0.5.seconds\n        cancel h\n        wait 0.5.seconds\n"));
    assert_eq!(e.code, "E-STM-003");
    let e = err(&format!("{SH}scene \"s\" for 1.second:\n    main:\n        finally:\n            note \"x\"\n        wait 1.second\n"));
    assert_eq!(e.code, "E-SYN-001");
    let e = err(&format!("{SH}scene \"s\" for 1.second:\n    main:\n        together:\n            wait 1.second\n            finally:\n                note \"x\"\n"));
    assert_eq!(e.code, "E-SYN-001");
}

// ---- cubic_bezier -----------------------------------------------------------------------------

#[test]
fn cubic_bezier_matches_the_css_curves() {
    // CSS `ease` = (0.25, 0.1, 0.25, 1): 0.8024 at half time
    let r = go(&format!("{SH}scene \"s\" for 2.seconds:\n    main:\n        a.move_to((100, 0)) over 1.second ease cubic_bezier(0.25, 0.1, 0.25, 1)\n        b.move_to((100, 50)) over 1.second ease cubic_bezier(0, 0, 1, 1)\n"));
    let _ = &r;
    let r = go(&format!("{SH}scene \"s\" for 2.seconds:\n    main:\n        together:\n            a.move_to((100, 0)) over 1.second ease cubic_bezier(0.25, 0.1, 0.25, 1)\n            b.move_to((100, 50)) over 1.second ease cubic_bezier(0, 0, 1, 1)\n            c.move_to((100, 100)) over 1.second ease cubic_bezier(0.5, 0, 0.5, 1)\n"));
    near(centre(&r.snapshots[12], 0).0, 80.24, 0.2);
    near(centre(&r.snapshots[12], 1).0, 50.0, 1e-6); // (0, 0, 1, 1) is linear
    near(centre(&r.snapshots[12], 2).0, 50.0, 1e-6); // symmetric
    near(centre(&r.snapshots[0], 0).0, 0.0, 1e-9);
    near(centre(&r.snapshots[24], 0).0, 100.0, 1e-9);
    assert!(
        centre(&r.snapshots[6], 2).0 < 25.0,
        "ease in-out starts slowly"
    );
}

#[test]
fn cubic_bezier_can_overshoot_and_checks_its_arguments() {
    let r = go(&format!("{SH}scene \"s\" for 2.seconds:\n    main:\n        a.move_to((100, 0)) over 1.second ease cubic_bezier(0.3, 1.6, 0.6, 1)\n"));
    let peak = (0..24)
        .map(|i| centre(&r.snapshots[i], 0).0)
        .fold(f64::MIN, f64::max);
    assert!(peak > 100.0, "overshoots: {peak}");
    let e = err(&format!("{SH}scene \"s\" for 1.second:\n    main:\n        a.move_to((100, 0)) over 1.second ease cubic_bezier(2, 0, 0.5, 1)\n"));
    assert_eq!(e.code, "E-TYP-001");
}

// ---- markers ----------------------------------------------------------------------------------

#[test]
fn a_timeline_marker_raises_an_event_a_rule_can_use() {
    let r = go(&format!("{SH}scene \"s\" for 4.seconds:\n    timeline:\n        marker \"dawn\" at 2.seconds\n        at 0.s:\n            c.position.x = 0\n    when marker(\"dawn\"):\n        a.move_to((100, 0)) over 1.second ease linear\n    main:\n        wait 4.seconds\n"));
    near(centre(&r.snapshots[47], 0).0, 0.0, 1e-9); // nothing before 2 s
    assert!(
        centre(&r.snapshots[49], 0).0 > 1.0,
        "the rule started at the marker"
    );
    near(centre(&r.snapshots[72], 0).0, 100.0, 1e-6);
}

#[test]
fn a_marker_fires_once_and_duplicates_are_errors() {
    let r = go(&format!("{SH}scene \"s\" for 4.seconds:\n    timeline:\n        marker \"m\" at 1.second\n    whenever marker(\"m\"):\n        a.move_by(10, 0)\n    main:\n        wait 4.seconds\n"));
    near(centre(&r.snapshots[90], 0).0, 10.0, 1e-6);
    let e = err(&format!("{SH}scene \"s\" for 1.second:\n    timeline:\n        marker \"m\" at 0.s\n        marker \"m\" at 1.s\n    main:\n        wait 1.second\n"));
    assert_eq!(e.code, "E-NAM-002");
}

// ---- derive -----------------------------------------------------------------------------------

fn flag(snap: &Value, entity: &str, name: &str) -> bool {
    let e = snap["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == entity)
        .expect("entity");
    e["flags"][name]
        .as_bool()
        .unwrap_or_else(|| panic!("no flag {name}"))
}
const BIRD: &str = "prop bird:\n    states: afraid, ready\n    derive alert when afraid or ready\n    derive calm when not alert\n    derive scared_long when afraid.held_for(1.second)\n    derive fresh when afraid.since(0.25.seconds)\n\n";

#[test]
fn a_derived_flag_follows_its_condition_every_tick() {
    let r = go(&format!("{BIRD}scene \"s\" for 4.seconds:\n    main:\n        wait 1.second\n        bird becomes ready\n        wait 1.second\n        bird clears ready\n        wait 2.seconds\n"));
    assert!(flag(&r.snapshots[0], "bird", "calm") && !flag(&r.snapshots[0], "bird", "alert"));
    assert!(flag(&r.snapshots[30], "bird", "alert") && !flag(&r.snapshots[30], "bird", "calm"));
    assert!(flag(&r.snapshots[60], "bird", "calm"));
}

#[test]
fn held_for_since_and_rose_look_back() {
    let r = go(&format!("{BIRD}scene \"s\" for 4.seconds:\n    main:\n        wait 0.5.seconds\n        bird becomes afraid\n        wait 3.seconds\n"));
    // afraid is set at tick 12
    assert!(!flag(&r.snapshots[11], "bird", "afraid"));
    assert!(flag(&r.snapshots[12], "bird", "afraid"));
    assert!(
        flag(&r.snapshots[12], "bird", "fresh"),
        "since: just became true"
    );
    assert!(flag(&r.snapshots[17], "bird", "fresh"));
    assert!(
        !flag(&r.snapshots[19], "bird", "fresh"),
        "since(0.25 s) ends after 6 ticks"
    );
    assert!(!flag(&r.snapshots[35], "bird", "scared_long"));
    assert!(
        flag(&r.snapshots[36], "bird", "scared_long"),
        "held for 1 s at tick 36"
    );
}

#[test]
fn rules_can_use_derived_flags_and_rose() {
    let r = go(&format!("{SH}{BIRD}scene \"s\" for 4.seconds:\n    when bird.scared_long:\n        a.move_to((100, 0)) over 1.second ease linear\n    when bird.afraid.rose:\n        b.move_to((100, 50)) over 1.second ease linear\n    main:\n        wait 0.5.seconds\n        bird becomes afraid\n        wait 3.seconds\n"));
    assert!(
        centre(&r.snapshots[36], 1).0 > 1.0 && centre(&r.snapshots[13], 1).0 > 1.0,
        "b starts when afraid rises"
    );
    near(centre(&r.snapshots[35], 0).0, 0.0, 1e-9);
    assert!(
        centre(&r.snapshots[38], 0).0 > 1.0,
        "a starts when scared_long becomes true"
    );
}

#[test]
fn derived_flags_cannot_be_set_and_cannot_form_cycles() {
    let e = err(&format!("{BIRD}scene \"s\" for 1.second:\n    main:\n        bird becomes alert\n        wait 1.second\n"));
    assert_eq!(e.code, "E-STA-001");
    let e = err("prop p:\n    states: x\n    derive a when b\n    derive b when a\n\nscene \"s\" for 1.second:\n    main:\n        wait 1.second\n");
    assert_eq!(e.code, "E-STA-002");
    let e = err("prop p:\n    states: x\n    derive x when true\n\nscene \"s\" for 1.second:\n    main:\n        wait 1.second\n");
    assert_eq!(e.code, "E-NAM-002");
}

#[test]
fn a_derived_flag_can_read_another_entitys_flags() {
    let r = go("prop a:\n    states: up\n\nprop b:\n    states: down\n    derive both when a.up and down\n\nscene \"s\" for 2.seconds:\n    main:\n        a becomes up\n        wait 0.5.seconds\n        b becomes down\n        wait 1.second\n");
    assert!(!flag(&r.snapshots[6], "b", "both"));
    assert!(flag(&r.snapshots[13], "b", "both"));
}
