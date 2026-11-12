//! The debug overlay draws bones and IK targets from the snapshot's `rigs` section.
use animscript_render::render_with;
use serde_json::json;

#[test]
fn bones_are_drawn_as_lines_with_joint_circles() {
    let snap = json!({"canvas": [200, 200], "background": "#000000", "shapes": [],
        "rigs": [{"name": "a", "bones": [{"name": "b", "head": [50.0, 100.0], "tip": [150.0, 100.0], "angle": 0.0}], "ik": [{"name": "h", "target": [100.0, 150.0], "weight": 1.0}]}]});
    let plain = render_with(&snap, 1.0, false, false).unwrap();
    assert_eq!(
        plain.pixel(100, 100).unwrap().green(),
        0,
        "nothing is drawn without the overlay"
    );
    let dbg = render_with(&snap, 1.0, false, true).unwrap();
    let p = dbg.pixel(100, 100).unwrap();
    assert!(
        p.green() > 100 && p.red() < 100,
        "the bone line is green: {p:?}"
    );
    let t = dbg.pixel(100, 150).unwrap();
    assert!(
        t.red() > 100 || t.green() > 100 || t.blue() > 100,
        "the IK target is marked: {t:?}"
    );
}
