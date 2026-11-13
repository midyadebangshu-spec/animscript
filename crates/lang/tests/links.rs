//! Round L2: locks, attach (parenting), constrain, and named channels with policies. Written first.
use animscript_lang::*;
use serde_json::Value;

const HEAD: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\nworld:\n    gravity: (0, 9.8)\n    scale: 100\n\n";

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
// a, b, c, d: squares of 10 at (0,0), (0,50), (0,100), (0,150)
const SH: &str = "shape a = square(size: 10, at: (0, 0))\nshape b = square(size: 10, at: (0, 50))\nshape c = square(size: 10, at: (0, 100))\nshape d = square(size: 10, at: (0, 150))\n";
fn scene(body: &str, secs: u32) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{body}")
}

// ---- locks ------------------------------------------------------------------------------------

#[test]
fn a_lock_beats_actions_and_keyframes() {
    let r = go(&format!("{SH}{}", scene("        lock a.position.x = 30\n        a.move_to((100, 0)) over 1.second ease linear\n", 2)));
    near(centre(&r.snapshots[12], 0).0, 30.0, 1e-9);
    let r = go(&format!("{SH}{}", scene("        lock a.position.x = 30\n        a.keyframes over 1.second:\n            at 0.s: position.x = 0\n            at 1.s: position.x = 100\n", 2)));
    near(centre(&r.snapshots[12], 0).0, 30.0, 1e-9);
}

#[test]
fn a_lock_holds_one_component_only() {
    let r = go(&format!("{SH}{}", scene("        lock a.position.x = 30\n        a.move_to((100, 80)) over 1.second ease linear\n", 2)));
    near(centre(&r.snapshots[12], 0).0, 30.0, 1e-9);
    near(centre(&r.snapshots[12], 0).1, 40.0, 1e-6); // the y part of the action is not locked
}

#[test]
fn a_lock_with_for_releases_itself_and_release_ends_it() {
    let r = go(&format!("{SH}{}", scene("        lock a.position.x = 30 for 0.5.seconds\n        a.move_to((100, 0)) over 2.seconds ease linear\n", 3)));
    near(centre(&r.snapshots[6], 0).0, 30.0, 1e-9);
    near(centre(&r.snapshots[36], 0).0, 75.0, 1.5); // back to the action's own value
    let r = go(&format!("{SH}{}", scene("        let h = lock b.position.x = 30\n        start b.move_to((100, 50)) over 2.seconds ease linear\n        wait 0.5.seconds\n        release h\n", 3)));
    near(centre(&r.snapshots[6], 1).0, 30.0, 1e-9);
    near(centre(&r.snapshots[18], 1).0, 37.5, 1.5);
}

#[test]
fn a_lock_can_hold_a_whole_position_or_a_rotation() {
    let r = go(&format!("{SH}{}", scene("        lock a.position = (10, 20)\n        lock b.rotation = 90.deg\n        wait 1.second\n", 2)));
    let c = centre(&r.snapshots[5], 0);
    near(c.0, 10.0, 1e-9);
    near(c.1, 20.0, 1e-9);
    let v = r.snapshots[5]["shapes"][1]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()))
        .collect::<Vec<_>>();
    near(v[0].0, 55.0 - 50.0 + 0.0 - 5.0 + 5.0 + 0.0, 20.0); // rotated about its centre: still centred on x = 0
    let cx = v.iter().map(|p| p.0).sum::<f64>() / 4.0;
    near(cx, 0.0, 1e-9);
}

#[test]
fn a_lock_wins_over_a_physics_body() {
    let r = go(&format!(
        "{SH}a.body()\n{}",
        scene("        lock a.position.y = 0\n        wait 1.second\n", 2)
    ));
    near(centre(&r.snapshots[20], 0).1, 0.0, 1e-9);
}

#[test]
fn two_locks_on_one_component_at_once_are_e_prp_002() {
    let e = err(&format!(
        "{SH}{}",
        scene(
            "        lock a.position.x = 1\n        lock a.position.x = 2\n        wait 1.second\n",
            1
        )
    ));
    assert_eq!(e.code, "E-PRP-002");
    // one after the other is fine
    go(&format!("{SH}{}", scene("        lock a.position.x = 1 for 0.5.seconds\n        wait 0.5.seconds\n        lock a.position.x = 2\n        wait 0.5.seconds\n", 2)));
}

// ---- attach -----------------------------------------------------------------------------------

#[test]
fn a_child_follows_its_parents_translation() {
    let r = go(&format!(
        "{SH}{}",
        scene(
            "        attach b to a\n        a.move_by(100, 0) over 1.second ease linear\n",
            2
        )
    ));
    near(centre(&r.snapshots[24], 1).0, 100.0, 1e-6);
    near(centre(&r.snapshots[24], 1).1, 50.0, 1e-6);
    near(centre(&r.snapshots[12], 1).0, 50.0, 1e-6);
}

#[test]
fn a_child_follows_its_parents_rotation_about_the_parents_pivot() {
    let r = go(&format!(
        "shape p = square(size: 10, at: (0, 0))\nshape q = square(size: 10, at: (50, 0))\n{}",
        scene(
            "        attach q to p\n        p.rotate_by(90.deg) over 1.second ease linear\n",
            2
        )
    ));
    let c = centre(&r.snapshots[24], 1);
    near(c.0, 0.0, 1e-6);
    near(c.1, 50.0, 1e-6); // clockwise on screen: (50, 0) -> (0, 50)
}

#[test]
fn attach_offset_places_the_child_at_the_parents_anchor() {
    let r = go(&format!("shape p = square(size: 10, at: (100, 100))\nshape q = square(size: 10, at: (0, 0))\n{}", scene("        attach q to p offset (20, 0)\n        wait 1.frame\n        p.move_by(10, 0) over 0.5.seconds ease linear\n", 2)));
    let c = centre(&r.snapshots[1], 1);
    near(c.0, 120.0, 1e-6);
    near(c.1, 100.0, 1e-6);
    near(centre(&r.snapshots[20], 1).0, 130.0, 1e-6);
}

#[test]
fn parents_are_solved_before_children() {
    let r = go(&format!("{SH}{}", scene("        attach c to b\n        attach b to a\n        a.move_by(100, 0) over 1.second ease linear\n", 2)));
    near(centre(&r.snapshots[24], 1).0, 100.0, 1e-6);
    near(centre(&r.snapshots[24], 2).0, 100.0, 1e-6); // the grandchild too, whatever the order written
}

#[test]
fn release_detaches_and_a_cycle_is_e_con_001() {
    let r = go(&format!("{SH}{}", scene("        let h = attach b to a\n        a.move_by(100, 0) over 1.second ease linear\n        release h\n        wait 0.5.seconds\n", 2)));
    near(centre(&r.snapshots[30], 1).0, 0.0, 1e-6);
    let e = err(&format!(
        "{SH}{}",
        scene(
            "        attach b to a\n        attach a to b\n        wait 1.second\n",
            1
        )
    ));
    assert_eq!(e.code, "E-CON-001");
}

#[test]
fn a_bone_can_be_the_parent() {
    let arm = "armature arm:\n    bone upper: at: (100, 100), length: 80, angle: 0.deg\n\nshape pad = square(size: 10, at: (150, 100))\n";
    let r = go(&format!("{arm}{}", scene("        attach pad to arm.upper\n        arm.upper.rotate_to(90.deg) over 1.second ease linear\n", 2)));
    let c = centre(&r.snapshots[24], 0);
    near(c.0, 100.0, 1e-6);
    near(c.1, 150.0, 1e-6);
}

#[test]
fn attach_is_for_a_running_scene() {
    let e = err(&format!(
        "{SH}attach b to a\n{}",
        scene("        wait 1.second\n", 1)
    ));
    assert_eq!(e.code, "E-STM-001");
}

// ---- constrain --------------------------------------------------------------------------------

#[test]
fn inside_keeps_the_whole_shape_in_the_rectangle() {
    let r = go(&format!("{SH}{}", scene("        constrain a.position inside rect(0, 0, 100, 100)\n        a.move_to((500, 0)) over 1.second ease linear\n", 2)));
    near(centre(&r.snapshots[24], 0).0, 95.0, 1e-6);
    near(centre(&r.snapshots[24], 0).1, 5.0, 1e-6); // and the other side
    let r = go(&format!("{SH}{}", scene("        constrain a.position inside canvas\n        a.move_to((900, 700)) over 1.second ease linear\n", 2)));
    let c = centre(&r.snapshots[24], 0);
    near(c.0, 795.0, 1e-6);
    near(c.1, 595.0, 1e-6);
}

#[test]
fn follow_copies_the_targets_motion_a_few_frames_late() {
    let r = go(&format!("{SH}{}", scene("        constrain b.position to follow(a, lag: 6.frames)\n        a.move_to((100, 0)) over 1.second ease linear\n", 2)));
    near(
        centre(&r.snapshots[18], 1).0,
        centre(&r.snapshots[12], 0).0,
        1e-6,
    );
    near(centre(&r.snapshots[18], 1).1, 50.0, 1e-6); // keeps its offset from the target
    near(centre(&r.snapshots[30], 1).0, 100.0, 1e-6);
}

#[test]
fn weight_blends_a_constraint_in() {
    let r = go(&format!("{SH}{}", scene("        constrain b.position to follow(a, lag: 0.frames) weight 0.5\n        a.move_to((100, 0)) over 1.second ease linear\n", 2)));
    near(centre(&r.snapshots[24], 1).0, 50.0, 1e-6);
}

#[test]
fn constraints_run_after_locks_and_can_be_released() {
    let r = go(&format!("{SH}{}", scene("        constrain a.position inside rect(0, 0, 100, 100)\n        lock a.position.x = 300\n        wait 1.second\n", 2)));
    near(centre(&r.snapshots[6], 0).0, 95.0, 1e-6);
    let r = go(&format!("{SH}{}", scene("        let h = constrain a.position inside rect(0, 0, 100, 100)\n        a.move_to((500, 0)) over 1.second ease linear\n        release h\n        wait 0.5.seconds\n", 2)));
    near(centre(&r.snapshots[30], 0).0, 500.0, 1e-6);
}

// ---- channels ---------------------------------------------------------------------------------

const MARK: &str = "action mark(self: Shape, who: Shape, x: Number):\n    uses body\n    wait 1.second\n    who.position = (x, 0)\n\naction urgent(self: Shape, who: Shape, x: Number):\n    uses body on_conflict interrupt\n    wait 1.second\n    who.position = (x, 0)\n\naction mild(self: Shape, who: Shape, x: Number):\n    uses body on_conflict reject\n    wait 1.second\n    who.position = (x, 0)\n\naction friendly(self: Shape, who: Shape, x: Number):\n    uses body on_conflict share\n    wait 1.second\n    who.position = (x, 0)\n\n";

#[test]
fn a_busy_channel_queues_the_next_action_by_default() {
    let r = go(&format!(
        "{SH}{MARK}{}",
        scene(
            "        start a.mark(b, 10)\n        start a.mark(c, 20)\n        wait 3.seconds\n",
            3
        )
    ));
    near(centre(&r.snapshots[25], 1).0, 10.0, 1e-6);
    near(centre(&r.snapshots[25], 2).0, 0.0, 1e-6); // the second has only just started
    near(centre(&r.snapshots[50], 2).0, 20.0, 1e-6);
}

#[test]
fn channels_belong_to_one_shape() {
    let r = go(&format!(
        "{SH}{MARK}{}",
        scene(
            "        start a.mark(c, 10)\n        start b.mark(d, 20)\n        wait 2.seconds\n",
            2
        )
    ));
    near(centre(&r.snapshots[25], 2).0, 10.0, 1e-6);
    near(centre(&r.snapshots[25], 3).0, 20.0, 1e-6); // different shapes, so at the same time
}

#[test]
fn interrupt_takes_over_and_cancels_the_occupant() {
    let r = go(&format!("{SH}{MARK}{}", scene("        start a.mark(b, 10)\n        wait 0.5.seconds\n        a.urgent(c, 20)\n        wait 1.second\n", 3)));
    near(centre(&r.snapshots[60], 1).0, 0.0, 1e-6); // the first never finished
    near(centre(&r.snapshots[40], 2).0, 20.0, 1e-6);
}

#[test]
fn interrupt_queues_behind_a_higher_priority_occupant() {
    let high = "action boss(self: Shape, who: Shape, x: Number):\n    uses body\n    priority 5\n    wait 1.second\n    who.position = (x, 0)\n\n";
    let r = go(&format!("{SH}{MARK}{high}{}", scene("        start a.boss(b, 10)\n        wait 0.25.seconds\n        start a.urgent(c, 20)\n        wait 3.seconds\n", 4)));
    near(centre(&r.snapshots[25], 1).0, 10.0, 1e-6); // the boss finished
    near(centre(&r.snapshots[40], 2).0, 0.0, 1e-6);
    near(centre(&r.snapshots[60], 2).0, 20.0, 1e-6); // the urgent one ran afterwards
}

#[test]
fn reject_fails_and_share_runs_together() {
    let e = err(&format!(
        "{SH}{MARK}{}",
        scene(
            "        start a.mark(b, 10)\n        start a.mild(c, 20)\n        wait 2.seconds\n",
            2
        )
    ));
    assert_eq!(e.code, "E-RUN-002");
    let r = go(&format!("{SH}{MARK}{}", scene("        start a.mark(b, 10)\n        start a.friendly(c, 20)\n        wait 2.seconds\n", 2)));
    near(centre(&r.snapshots[25], 1).0, 10.0, 1e-6);
    near(centre(&r.snapshots[25], 2).0, 20.0, 1e-6);
}

#[test]
fn an_interrupted_action_runs_its_finally() {
    let fin = "action tidy(self: Shape):\n    uses body\n    wait 5.seconds\n    finally:\n        d.position = (7, 0)\n\n";
    let r = go(&format!("{SH}{MARK}{fin}{}", scene("        start a.tidy()\n        wait 0.5.seconds\n        a.urgent(c, 20)\n        wait 1.second\n", 3)));
    near(centre(&r.snapshots[40], 3).0, 7.0, 1e-6);
}

#[test]
fn touching_an_undeclared_channel_is_e_cap_003() {
    let e = err(&format!(
        "{SH}action bad(self: Shape):\n    uses gaze\n    self.move_by(10, 0) over 1.second\n\n{}",
        scene("        a.bad()\n", 1)
    ));
    assert_eq!(e.code, "E-CAP-003");
}
