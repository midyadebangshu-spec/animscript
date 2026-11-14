//! `mesh.pattern(mask:, hole:, color:, glow:)`: equations of the surface coordinates `u` and `v`. Written first.
use animscript_lang::*;
use std::path::Path;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 400x300\n    fps: 24\n\nimport \"std/3d\"\n";
const SCENE: &str = "scene \"s\" for 2.frames:\n    main:\n        wait 1.frame\n";

#[allow(clippy::result_large_err)]
fn run(body: &str) -> Result<RunResult, Diagnostic> {
    run_in(
        &format!("{PROJECT}{body}{SCENE}"),
        &RunOptions::default(),
        Path::new("."),
    )
}

#[test]
fn a_pattern_reaches_the_snapshot_with_surface_coordinates() {
    let r = run("shape vase = lathe(profile: [(10, 0), (30, 40), (10, 80)], segments: 12)\nvase.pattern(mask: \"fract(u * 6) < 0.5\", color: #ff0000, hole: \"v > 0.9\", glow: 2)\n").ok().unwrap();
    let m = &r.snapshots[0]["shapes"][0]["mesh"];
    assert_eq!(m["pattern"]["mask"], "fract(u * 6) < 0.5");
    assert_eq!(m["pattern"]["hole"], "v > 0.9");
    assert_eq!(m["pattern"]["color"], "#ff0000");
    assert_eq!(m["pattern"]["glow"], 2.0);
    assert_eq!(
        m["uv"].as_array().unwrap().len(),
        m["faces"].as_array().unwrap().len()
    );
}

#[test]
fn a_mesh_without_a_pattern_carries_no_surface_coordinates() {
    let r = run("shape vase = lathe(profile: [(10, 0), (30, 40), (10, 80)], segments: 12)\n")
        .ok()
        .unwrap();
    assert!(r.snapshots[0]["shapes"][0]["mesh"].get("uv").is_none());
}

#[test]
fn a_pattern_may_use_the_math_functions_and_noise() {
    assert!(run("shape p = plane(width: 100, depth: 100, nx: 2, nz: 2)\np.pattern(mask: \"smoothstep(0.4, 0.6, noise(u * 8, v * 8))\")\n").is_ok());
}

#[test]
fn pattern_mistakes_are_reported() {
    let lathe = "shape vase = lathe(profile: [(10, 0), (30, 40), (10, 80)], segments: 12)\n";
    for (call, what) in [
        ("vase.pattern(mask: \"u +\")", "a broken equation"),
        ("vase.pattern(mask: \"w > 1\")", "an unknown name"),
        ("vase.pattern(color: #ff0000)", "no mask or hole"),
        ("vase.pattern(glow: -1, mask: \"u\")", "negative glow"),
        ("vase.pattern(mask: 3)", "a number instead of an equation"),
    ] {
        let e = run(&format!("{lathe}{call}\n"))
            .err()
            .unwrap_or_else(|| panic!("{what} should fail"));
        assert_eq!(e.code, "E-MSH-001", "{what}: {}", e.message);
    }
    // an extrusion has no surface coordinates
    let e = run("shape e = extrude(points: [(0, 0), (40, 0), (20, 30)], depth: 10)\ne.pattern(mask: \"u\")\n").err().unwrap();
    assert_eq!(e.code, "E-MSH-001");
    assert!(e.helps.iter().any(|h| h.contains("lathe")), "{:?}", e.helps);
    // not a mesh
    let e = run("shape s = square(size: 10)\ns.pattern(mask: \"u\")\n")
        .err()
        .unwrap();
    assert_eq!(e.code, "E-MSH-001");
}

#[test]
fn extra_named_numbers_are_constants_in_the_equations() {
    let r = run("shape p = plane(width: 100, depth: 100, nx: 1, nz: 1)\np.pattern(mask: \"fract(u * repeats) < width\", repeats: 12, width: 0.3)\n").ok().unwrap();
    let c = &r.snapshots[0]["shapes"][0]["mesh"]["pattern"]["consts"];
    assert_eq!(c["repeats"], 12.0);
    assert_eq!(c["width"], 0.3);
}
