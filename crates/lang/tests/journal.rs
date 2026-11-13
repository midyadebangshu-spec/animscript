//! The run journal: what an editor needs besides the snapshots (scenes, timed items with their source
//! lines, notes, declarations). Written before the implementation.
use animscript_lang::journal::ItemKind;
use animscript_lang::*;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";

fn opts() -> RunOptions {
    RunOptions::default()
}
fn go(src: &str) -> RunResult {
    match run(src, &opts()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(src), "test")),
    }
}
fn line_of(src: &str, needle: &str) -> u32 {
    src.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line with `{needle}`")) as u32
        + 1
}

#[test]
fn scenes_are_listed_in_project_ticks() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        wait 1.frame\nscene \"b\" for 12.frames:\n    main:\n        wait 1.frame\n"
    );
    let r = go(&src);
    let sc = &r.journal.scenes;
    assert_eq!(sc.len(), 2);
    assert_eq!((sc[0].name.as_str(), sc[0].start, sc[0].end), ("a", 0, 24));
    assert_eq!((sc[1].name.as_str(), sc[1].start, sc[1].end), ("b", 24, 36));
    assert_eq!(sc[1].span.line, line_of(&src, "scene \"b\""));
    assert_eq!(r.snapshots.len(), 36);
}

#[test]
fn an_action_is_recorded_with_its_ticks_and_source_line() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        wait 3.frames\n        s.move_by(10, 0) over 12.frames\n"
    );
    let r = go(&src);
    let it = &r.journal.items;
    assert_eq!(it.len(), 1, "{it:?}");
    assert_eq!(it[0].kind, ItemKind::Action);
    assert_eq!(it[0].target, "s");
    assert_eq!(it[0].label, "s.move_by");
    assert_eq!((it[0].start, it[0].end), (3, Some(15)));
    assert_eq!(it[0].span.unwrap().line, line_of(&src, "move_by"));
}

#[test]
fn actions_in_a_later_scene_use_project_ticks() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 10.frames:\n    main:\n        wait 1.frame\nscene \"b\" for 20.frames:\n    main:\n        s.rotate_by(90.deg) over 5.frames\n"
    );
    let r = go(&src);
    let it = &r.journal.items;
    assert_eq!(it.len(), 1);
    assert_eq!(it[0].scene, "b");
    assert_eq!((it[0].start, it[0].end), (10, Some(15)));
}

#[test]
fn started_actions_and_modifiers_keep_their_own_lines() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        start s.fade_to(0.5) over 6.frames\n        let h = start s.vertices.wobble(amount: 2, speed: 1)\n        wait 4.frames\n        cancel h\n"
    );
    let r = go(&src);
    let it = &r.journal.items;
    let fade = it.iter().find(|i| i.label.contains("fade_to")).unwrap();
    assert_eq!(fade.kind, ItemKind::Action);
    assert_eq!((fade.start, fade.end), (0, Some(6)));
    assert_eq!(fade.span.unwrap().line, line_of(&src, "fade_to"));
    let wob = it.iter().find(|i| i.kind == ItemKind::Modifier).unwrap();
    assert_eq!(wob.target, "s");
    assert_eq!(wob.span.unwrap().line, line_of(&src, "wobble"));
    // open-ended, then cancelled at tick 4
    assert_eq!((wob.start, wob.end), (0, Some(4)));
}

#[test]
fn an_open_ended_modifier_ends_with_its_scene() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 10.frames:\n    main:\n        start s.vertices.wobble(amount: 2, speed: 1)\n        wait 1.frame\n"
    );
    let r = go(&src);
    let wob = &r.journal.items[0];
    assert_eq!((wob.start, wob.end), (0, Some(10)));
}

#[test]
fn keyframes_are_one_item_with_their_key_ticks() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 2.seconds:\n    main:\n        wait 2.frames\n        s.keyframes over 1.second:\n            at 0.seconds: position = (0, 0)\n            at 12.frames: position = (50, 20)\n            at 1.second: position = (100, 0)\n"
    );
    let r = go(&src);
    let it = &r.journal.items;
    // position.x and position.y are two key sets in the runtime but one statement in the source
    assert_eq!(it.len(), 1, "{it:?}");
    assert_eq!(it[0].kind, ItemKind::Keys);
    assert_eq!(it[0].target, "s");
    assert_eq!(it[0].keys, vec![2, 14, 26]);
    assert_eq!((it[0].start, it[0].end), (2, Some(26)));
    assert_eq!(it[0].span.unwrap().line, line_of(&src, "s.keyframes"));
}

#[test]
fn timeline_blocks_are_recorded_as_keys() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 1.second:\n    timeline:\n        at 0.seconds:\n            s.rotation = 0.deg\n        at 12.frames:\n            s.rotation = 90.deg\n    main:\n        wait 1.frame\n"
    );
    let r = go(&src);
    let it = &r.journal.items;
    assert_eq!(it.len(), 1, "{it:?}");
    assert_eq!(it[0].kind, ItemKind::Keys);
    assert_eq!(it[0].keys, vec![0, 12]);
    assert!(it[0].span.is_some());
}

#[test]
fn notes_are_kept_with_their_tick_and_line() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nnote \"building\"\nscene \"a\" for 10.frames:\n    main:\n        wait 4.frames\n        note \"hello\"\n"
    );
    let r = go(&src);
    let n = &r.journal.notes;
    assert_eq!(n.len(), 2, "{n:?}");
    assert_eq!((n[0].tick, n[0].text.as_str()), (None, "building"));
    assert_eq!((n[1].tick, n[1].text.as_str()), (Some(4), "hello"));
    assert_eq!(n[1].span.unwrap().line, line_of(&src, "note \"hello\""));
}

#[test]
fn declarations_point_at_their_lines() {
    let src = format!(
        "{PROJECT}shape a = square(size: 100)\nshape b = circle(8, radius: 20)\nprop door:\n    states: open\nscene \"s\" for 2.frames:\n    main:\n        wait 1.frame\n"
    );
    let r = go(&src);
    let d = &r.journal.decls;
    let find = |n: &str| {
        d.iter()
            .find(|x| x.name == n)
            .unwrap_or_else(|| panic!("{n}: {d:?}"))
    };
    assert_eq!(find("a").kind, "shape");
    assert_eq!(find("a").span.line, line_of(&src, "shape a"));
    assert_eq!(find("b").span.line, line_of(&src, "shape b"));
    assert_eq!(find("door").kind, "prop");
    assert_eq!(find("door").span.line, line_of(&src, "prop door"));
}

#[test]
fn the_journal_is_not_in_the_snapshots() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/actions.anim"
    ))
    .unwrap();
    let with = run(&src, &opts()).unwrap();
    assert!(!with.journal.items.is_empty());
    // the journal is a side table: no snapshot mentions it
    assert!(with.snapshots.iter().all(|s| s.get("journal").is_none()));
}

#[test]
fn a_runtime_error_keeps_the_frames_made_so_far() {
    let src = format!(
        "{PROJECT}shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        s.move_by(10, 0) over 6.frames\n        assert 1 == 2\n"
    );
    let (r, err) = run_partial(&src, &opts()).unwrap();
    let err = err.expect("the assert fails");
    assert!(err.is_error());
    assert!(!r.snapshots.is_empty() && r.snapshots.len() < 24);
    assert_eq!(r.journal.items.len(), 1);
    // a program that does not even build has no frames to keep
    assert!(run_partial("shape s = nothing(1)\n", &opts()).is_err());
}

#[test]
fn an_action_inside_an_imported_library_points_at_the_line_that_called_it() {
    let src = format!(
        "{PROJECT}import \"std/motion\"\nshape s = square(size: 100)\nscene \"a\" for 2.seconds:\n    main:\n        s.hop(height: 50, over: 12.frames)\n        s.look_to((400, 100), over: 6.frames)\n"
    );
    let r = go(&src);
    assert_eq!(r.journal.items.len(), 3);
    for it in &r.journal.items {
        // `look_to` calls library functions while working out its angle: still the caller's line
        let want = if it.label.contains("rotate") {
            "s.look_to"
        } else {
            "s.hop"
        };
        assert_eq!(
            it.span.expect("a span in the user's file").line,
            line_of(&src, want),
            "{it:?}"
        );
    }
}

#[test]
fn an_action_inside_your_own_action_points_at_the_statement_in_its_body() {
    let src = format!(
        "{PROJECT}action nudge(self: Shape):\n    self.move_by(10, 0) over 4.frames\n\nshape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        start s.nudge()\n        wait 6.frames\n        together:\n            s.rotate_by(90.deg) over 3.frames\n"
    );
    let r = go(&src);
    let mv = r
        .journal
        .items
        .iter()
        .find(|i| i.label.contains("move_by"))
        .unwrap();
    assert_eq!(mv.span.unwrap().line, line_of(&src, "self.move_by"));
    let rot = r
        .journal
        .items
        .iter()
        .find(|i| i.label.contains("rotate_by"))
        .unwrap();
    assert_eq!(rot.span.unwrap().line, line_of(&src, "s.rotate_by"));
}
