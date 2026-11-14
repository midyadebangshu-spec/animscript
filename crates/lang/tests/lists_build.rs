//! Building lists: `+` joins lists and `map` turns each element into something else (so a profile or a ring of
//! points can be computed). Written first.
use animscript_lang::*;
use std::path::Path;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 24\n\n";

fn run_ok(body: &str) -> RunResult {
    let text =
        format!("{PROJECT}{body}scene \"s\" for 2.frames:\n    main:\n        wait 1.frame\n");
    match run_in(&text, &RunOptions::default(), Path::new(".")) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}

#[test]
fn lists_join_with_plus_and_map_computes_elements() {
    let r = run_ok(
        "let pts = [(0, 0)] + (1..4).map(i -> (i * 10, i * 5)) + [(99, 99)]\nassert pts.count == 6\nassert pts[3] == (30, 15)\nassert pts[5] == (99, 99)\nlet idx = (0..2).map((v, i) -> v * 2 + i)\nassert idx[2] == 6\n",
    );
    assert_eq!(r.snapshots.len(), 2);
}

#[test]
fn map_and_plus_have_clear_errors() {
    let text = format!(
        "{PROJECT}let a = [1] + 2\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n"
    );
    let e = run_in(&text, &RunOptions::default(), Path::new("."))
        .err()
        .unwrap();
    assert_eq!(e.code, "E-TYP-002");
    let text = format!(
        "{PROJECT}let a = [1].map(3)\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n"
    );
    let e = run_in(&text, &RunOptions::default(), Path::new("."))
        .err()
        .unwrap();
    assert_eq!(e.code, "E-TYP-001");
}

#[test]
fn a_computed_profile_makes_a_lathe() {
    let r = run_ok(
        "import \"std/3d\"\nshape bowl = lathe(profile: [(0, 0)] + (1..8).map(i -> (50 * sin(i / 8 * 0.25.turns), i * 10)), segments: 16, at: (200, 150, 0))\n",
    );
    assert!(!r.snapshots[0]["shapes"].as_array().unwrap().is_empty());
}

#[test]
fn a_fn_called_while_building_reads_earlier_top_level_lets() {
    let r = run_ok("let k = 7\nfn times_k(x: Number) -> Number:\n    return x * k\nlet y = times_k(3)\nassert y == 21\n");
    assert_eq!(r.snapshots.len(), 2);
}

#[test]
fn a_lathe_strip_can_be_double_sided() {
    let r = run_ok("import \"std/3d\"\nshape ring = lathe(profile: [(40, 0), (44, 20), (40, 40)], segments: 16, double_sided: true)\n");
    let sh = &r.snapshots[0]["shapes"][0];
    assert_eq!(sh["mesh"]["double_sided"], true);
}
