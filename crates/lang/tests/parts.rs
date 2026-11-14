//! Parts: imports from folders, namespaces (`as`), instanced components (`as name(param: value)`), run order
//! and diagnostics that name the part file. Written first.
use animscript_lang::*;
use std::path::{Path, PathBuf};

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";
const SCENE: &str = "scene \"s\" for 2.frames:\n    main:\n        wait 1.frame\n";

fn folder(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("anim_parts_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir
}

#[allow(clippy::result_large_err)]
fn run(main: &str, dir: &Path) -> Result<RunResult, Diagnostic> {
    run_in(main, &RunOptions::default(), dir)
}

fn fails(main: &str, dir: &Path) -> Diagnostic {
    match run(main, dir) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

/// Centre x of the shape with this name in the first snapshot.
fn x_of(r: &RunResult, name: &str) -> f64 {
    let shapes = r.snapshots[0]["shapes"].as_array().unwrap();
    let s = shapes
        .iter()
        .find(|s| s["id"] == name)
        .unwrap_or_else(|| panic!("no shape {name}"));
    let v = s["vertices"].as_array().unwrap();
    v.iter().map(|p| p[1].as_f64().unwrap()).sum::<f64>() / v.len() as f64
}

#[test]
fn a_part_runs_before_the_main_file_so_main_can_use_its_shapes() {
    let d = folder(
        "order",
        &[(
            "head.anim",
            "shape head = square(size: 20, at: (100, 100))\n",
        )],
    );
    let main = format!("{PROJECT}import \"head.anim\"\nhead.move_by(200, 0)\n\n{SCENE}");
    let r = run(&main, &d).unwrap_or_else(|e| panic!("{}", e.render(Some(&main), "main")));
    assert!((x_of(&r, "head") - 300.0).abs() < 1e-9);
}

#[test]
fn imports_reach_into_folders_and_the_extension_is_optional() {
    let d = folder(
        "dirs",
        &[
            (
                "parts/head.anim",
                "shape head = square(size: 20, at: (50, 50))\n",
            ),
            (
                "parts/deep/hand.anim",
                "import \"../head\"\nshape hand = square(size: 10, at: (70, 50))\n",
            ),
        ],
    );
    let main = format!("{PROJECT}import \"parts/deep/hand\"\n{SCENE}");
    let r = run(&main, &d).unwrap();
    assert!((x_of(&r, "head") - 50.0).abs() < 1e-9);
    assert!((x_of(&r, "hand") - 70.0).abs() < 1e-9);
}

#[test]
fn a_namespace_keeps_part_names_apart() {
    let d = folder(
        "ns",
        &[(
            "a.anim",
            "let gap = 30\nshape box = square(size: 10, at: (gap, 100))\n",
        )],
    );
    // `box` and `gap` exist in the main file too; the part's are reached through the alias
    let main = format!(
        "{PROJECT}import \"a\" as p\nlet gap = 5\nshape box = square(size: 10, at: (gap, 200))\np.box.move_by(p.gap, 0)\n{SCENE}"
    );
    let r = run(&main, &d).unwrap_or_else(|e| panic!("{}", e.render(Some(&main), "main")));
    assert!((x_of(&r, "p__box") - 60.0).abs() < 1e-9);
    assert!((x_of(&r, "box") - 5.0).abs() < 1e-9);
}

#[test]
fn one_file_can_be_instanced_twice_with_different_params() {
    let d = folder(
        "inst",
        &[(
            "arm.anim",
            "param cx = 100\nparam side = \"left\"\nshape arm = rectangle(width: 10, height: 40, at: (cx, 300))\nfn arm_side() -> String:\n    return side\n",
        )],
    );
    let main = format!(
        "{PROJECT}import \"arm\" as left(cx: 200)\nimport \"arm\" as right(cx: 400, side: \"right\")\n{SCENE}"
    );
    // `fn`s are global, so a repeated one is a duplicate: params and shapes are what repeat
    let e = fails(&main, &d);
    assert_eq!(e.code, "E-DEF-001", "{}", e.message);
    let d = folder(
        "inst2",
        &[(
            "arm.anim",
            "param cx = 100\nshape arm = rectangle(width: 10, height: 40, at: (cx, 300))\n",
        )],
    );
    let r = run(&main_without_fn(), &d).unwrap();
    assert!((x_of(&r, "left__arm") - 200.0).abs() < 1e-9);
    assert!((x_of(&r, "right__arm") - 400.0).abs() < 1e-9);
    // opened on its own, the default applies
    let own =
        format!("{PROJECT}param cx = 100\nshape arm = square(size: 4, at: (cx, 300))\n{SCENE}");
    let r = run(&own, &d).unwrap();
    assert!((x_of(&r, "arm") - 100.0).abs() < 1e-9);
}

fn main_without_fn() -> String {
    format!("{PROJECT}import \"arm\" as left(cx: 200)\nimport \"arm\" as right(cx: 400)\n{SCENE}")
}

#[test]
fn unknown_params_and_missing_files_are_reported() {
    let d = folder(
        "bad",
        &[("arm.anim", "param cx = 1\nshape a = square(size: 1)\n")],
    );
    let e = fails(&format!("{PROJECT}import \"arm\" as l(cy: 2)\n{SCENE}"), &d);
    assert_eq!(e.code, "E-AST-001");
    assert!(e.helps.iter().any(|h| h.contains("cx")));
    let e = fails(&format!("{PROJECT}import \"nothing\"\n{SCENE}"), &d);
    assert_eq!(e.code, "E-AST-001");
}

#[test]
fn a_part_s_project_and_scene_are_ignored_when_imported() {
    let d = folder(
        "own",
        &[(
            "head.anim",
            &format!("{PROJECT}shape head = square(size: 20, at: (10, 10))\n\nscene \"preview\" for 100.frames:\n    main:\n        wait 100.frames\n"),
        )],
    );
    let main = format!("{PROJECT}import \"head\"\n{SCENE}");
    let r = run(&main, &d).unwrap_or_else(|e| panic!("{}", e.render(Some(&main), "main")));
    assert_eq!(r.snapshots.len(), 2); // only main's two-frame scene ran
}

#[test]
fn errors_inside_a_part_name_the_file_and_its_own_line() {
    let d = folder(
        "diag",
        &[(
            "legs/leg.anim",
            "shape leg = square(size: 10)\nleg.nope_such_thing = 3\n",
        )],
    );
    let main = format!("{PROJECT}import \"legs/leg\"\n{SCENE}");
    let e = fails(&main, &d);
    let (file, _) = e.file.clone().expect("the part file is named");
    assert!(file.ends_with("leg.anim"), "{file}");
    assert_eq!(e.span.unwrap().line, 2);
    assert!(e.render(Some(&main), "main.anim").contains("leg.anim:2"));
    let found = check_in(&main, &d);
    assert!(found.iter().any(|d| d.file.is_some()));
}

#[test]
fn part_constants_can_drive_an_armature() {
    let d = folder("rig", &[("sizes.anim", "let thigh = 60\n")]);
    let main = format!(
        "{PROJECT}import \"sizes\"\narmature hero:\n    bone torso: at: (200, 300), length: 100, angle: -90.deg\n    bone thigh: parent: torso, at: 0.0, length: thigh, angle: 90.deg\n\n{SCENE}"
    );
    let r = run(&main, &d).unwrap_or_else(|e| panic!("{}", e.render(Some(&main), "main")));
    assert_eq!(r.snapshots.len(), 2);
}
