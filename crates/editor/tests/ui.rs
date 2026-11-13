//! The editor's panels, driven without a window (egui_kittest).
use anim_editor::app::{EditorApp, Tab};
use anim_editor::model::Selection;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

const SRC: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 10\n\nshape back = square(size: 200, at: (200, 150))\nback.fill = #336699\nshape dot = circle(8, radius: 20, at: (200, 150))\ndot.fill = #ffcc00\ndot.z = 1\nscene \"one\" for 1.second:\n    main:\n        note \"hello console\"\n        dot.move_by(50, 0) over 5.frames\nscene \"two\" for 5.frames:\n    main:\n        wait 1.frame\n";

fn app_with(src: &str) -> EditorApp {
    let mut app = EditorApp::new(|| {});
    app.headless = true;
    app.source = src.to_string();
    app.saved = src.to_string();
    app.run_now();
    app.settle();
    app
}

fn harness(app: EditorApp) -> Harness<'static, EditorApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, app: &mut EditorApp| app.show(ui), app);
    h.run_steps(3);
    h
}

#[test]
fn every_panel_is_on_screen() {
    let h = harness(app_with(SRC));
    assert_eq!(Tab::ALL.len(), 6);
    // code (the text is one node), transport, preview toolbar
    assert!(h.query_by_label("Fit").is_some());
    assert!(h.query_by_label("Loop").is_some());
    // hierarchy
    assert!(h.query_by_label("dot").is_some());
    assert!(h.query_by_label("back").is_some());
    // console
    assert!(h.query_by_label_contains("hello console").is_some());
    assert!(h.query_by_label_contains("ran 15 ticks").is_some());
    // nothing selected yet
    assert!(h.query_by_label_contains("Select something").is_some());
}

#[test]
fn clicking_a_shape_in_the_hierarchy_fills_the_inspector() {
    let mut h = harness(app_with(SRC));
    h.get_by_label("dot").click();
    h.run_steps(3);
    assert_eq!(h.state().selection, Selection::Shape("dot".into()));
    assert!(h.query_by_label("Transform").is_some());
    assert!(h.query_by_label("#ffcc00").is_some());
    assert!(h.query_by_label("(0, 0)").is_some(), "position at tick 0");
    // move the playhead: the inspector follows
    h.state_mut().set_tick(9);
    h.run_steps(3);
    assert!(
        h.query_by_label("(50, 0)").is_some(),
        "position after the move"
    );
}

#[test]
fn clicking_a_scene_moves_the_playhead_to_it() {
    let mut h = harness(app_with(SRC));
    h.get_by_label("two").click();
    h.run_steps(3);
    assert_eq!(h.state().selection, Selection::Scene(1));
    assert_eq!(h.state().tick, 10);
}

#[test]
fn clicking_a_console_note_goes_to_its_line_and_tick() {
    let mut app = app_with(SRC);
    app.set_tick(7);
    let mut h = harness(app);
    h.get_by_label_contains("hello console").click();
    h.run_steps(3);
    assert_eq!(h.state().tick, 0);
}

#[test]
fn transport_buttons_step_and_play() {
    let mut h = harness(app_with(SRC));
    h.get_by_label(">").click();
    h.run_steps(2);
    assert_eq!(h.state().tick, 1);
    h.get_by_label(">|").click();
    h.run_steps(2);
    assert_eq!(h.state().tick, 14);
    h.get_by_label("|<").click();
    h.run_steps(2);
    assert_eq!(h.state().tick, 0);
    h.get_by_label("Play").click();
    h.run_steps(2);
    assert!(h.state().playing);
    assert!(h.query_by_label("Pause").is_some());
}

#[test]
fn errors_show_in_the_console_and_keep_the_editor_alive() {
    let h = harness(app_with(
        "shape s = square(size: 100)\ns.v9.move_to(0, 0)\n",
    ));
    assert!(h.query_by_label_contains("[E-SHP-002]").is_some());
    assert!(h.query_all_by_label_contains("No frames").next().is_some());
    assert!(
        !h.state().stale(),
        "there never was a good run to fall back to"
    );
    assert!(h.state().out.as_ref().unwrap().errors() >= 1);
}

#[test]
fn playback_wraps_or_stops_at_the_end() {
    let mut app = app_with(SRC);
    app.toggle_play();
    app.advance(1.0); // 10 ticks at 10 fps
    assert_eq!(app.tick, 10);
    app.advance(1.0); // 20 -> wraps over 15 ticks
    assert_eq!(app.tick, 5);
    app.looping = false;
    app.advance(5.0);
    assert_eq!(app.tick, 14);
    assert!(!app.playing);
}

#[test]
fn saving_writes_the_file_and_clears_the_modified_mark() {
    let dir = std::env::temp_dir().join(format!("anim-editor-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.anim");
    std::fs::write(&path, SRC).unwrap();
    let mut app = EditorApp::new(|| {});
    app.headless = true;
    app.open_path(&path);
    app.settle();
    assert!(!app.modified());
    assert_eq!(app.file_name(), "a.anim");
    app.source.push_str("# edited\n");
    assert!(app.modified());
    assert!(app.save());
    assert!(!app.modified());
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .ends_with("# edited\n"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn going_to_a_line_selects_it_in_the_code_panel() {
    let mut app = app_with(SRC);
    app.goto_line(8);
    let mut h = harness(app);
    assert!(h.state().code.jump.is_none(), "the jump was carried out");
    let state = egui::TextEdit::load_state(&h.ctx, egui::Id::new("code_editor")).unwrap();
    let range = state.cursor.char_range().unwrap();
    let (a, b): (usize, usize) = (range.secondary.index.into(), range.primary.index.into());
    let text: String = SRC.chars().skip(a.min(b)).take(a.abs_diff(b)).collect();
    assert_eq!(text, "shape dot = circle(8, radius: 20, at: (200, 150))");
    h.run_steps(2);
}

#[test]
fn timeline_items_of_the_example_point_into_the_file() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/actions.anim"
    ))
    .unwrap();
    let app = app_with(&src);
    let out = app.out.as_ref().unwrap();
    let lines = src.lines().count() as u32;
    assert!(!out.journal.items.is_empty());
    for it in &out.journal.items {
        assert!(it.span.is_some_and(|s| s.line <= lines), "{it:?}");
    }
}

#[test]
fn a_broken_edit_keeps_the_last_good_frames_on_screen() {
    let mut app = app_with(SRC);
    app.set_tick(9);
    app.source.push_str("dot.move_to(\n");
    app.run_now();
    app.settle();
    assert!(app.out.as_ref().unwrap().errors() >= 1);
    assert!(app.out.as_ref().unwrap().snapshots.is_empty());
    assert!(app.stale());
    assert_eq!(app.shown.as_ref().unwrap().snapshots.len(), 15);
    assert_eq!(app.tick, 9);
    let h = harness(app);
    assert!(
        h.query_by_label("dot").is_some(),
        "the hierarchy still lists the shapes"
    );
    assert!(h.query_by_label("last good run").is_some());
    assert!(
        h.query_by_label_contains("[E-").is_some(),
        "and the console shows the error"
    );
}
