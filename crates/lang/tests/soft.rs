//! Soft bodies in the language: `grid`, `.soft()`, pins, reads, layering, errors. Written first.
use animscript_lang::*;
use serde_json::Value;

const PROJECT: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\nworld:\n    gravity: (0, 9.8)\n    scale: 100\n\n";

fn go(body: &str) -> RunResult {
    let text = format!("{PROJECT}{body}");
    match run(&text, &RunOptions::default()) {
        Ok(r) => r,
        Err(d) => panic!("{}", d.render(Some(&text), "test")),
    }
}
fn err(body: &str) -> Diagnostic {
    match run(&format!("{PROJECT}{body}"), &RunOptions::default()) {
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
fn near(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tolerance {tol})");
}
const END: &str = "    main:\n        wait 3.seconds\n";

#[test]
fn grid_makes_n_by_n_cells() {
    let r = go("shape g = grid(4, width: 200, height: 120, at: (300, 200))\nscene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n");
    assert_eq!(verts(&r.snapshots[0], 0).len(), 25);
    assert_eq!(
        r.snapshots[0]["shapes"][0]["faces"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
    let v = verts(&r.snapshots[0], 0);
    let (lx, hx) = (
        v.iter().map(|p| p.0).fold(f64::MAX, f64::min),
        v.iter().map(|p| p.0).fold(f64::MIN, f64::max),
    );
    near(hx - lx, 200.0, 1e-9);
}

#[test]
fn a_pinned_rope_hangs_and_keeps_its_length() {
    let r = go(&format!("shape rope = line((100, 50), (300, 50))\nrope.e1.subdivide(10)\nrope.soft(stiffness: 1.0)\nrope.pin(rope.v[1])\nscene \"s\" for 3.seconds:\n{END}"));
    let first = verts(&r.snapshots[0], 0);
    let last = verts(r.snapshots.last().unwrap(), 0);
    near(last[0].0, 100.0, 1e-6); // vertex 1 is pinned in place
    near(last[0].1, 50.0, 1e-6);
    let end = last[1]; // vertex 2 is the free end
    assert!(end.1 > 100.0, "it fell below its pin: {end:?}");
    near(
        ((end.0 - 100.0).powi(2) + (end.1 - 50.0).powi(2)).sqrt() / 200.0,
        1.0,
        0.12,
    );
    assert_eq!(first.len(), last.len());
}

#[test]
fn a_jelly_drops_squashes_and_rests_on_the_ground() {
    let r = go("shape ground = rectangle(width: 760, height: 40, at: (400, 580))\nshape jelly = rectangle(width: 120, height: 100, at: (400, 150))\njelly.f1.subdivide(4)\nground.body(kind: static)\njelly.soft(stiffness: 0.9, pressure: 1.0, restitution: 0.2)\nscene \"s\" for 4.seconds:\n    main:\n        wait 4.seconds\n");
    for s in &r.snapshots {
        for p in verts(s, 1) {
            assert!(p.1 < 560.0 + 3.0, "a vertex sank into the ground: {p:?}");
        }
    }
    let last = r.snapshots.last().unwrap();
    let v = verts(last, 1);
    let bottom = v.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    near(bottom, 560.0, 4.0);
    let soft = &last["soft"]["bodies"][0];
    assert_eq!(soft["shape"], "jelly");
    near(
        soft["volume"].as_f64().unwrap() / (120.0 * 100.0),
        1.0,
        0.12,
    );
}

#[test]
fn volume_and_stretch_can_be_read() {
    let r = go("shape jelly = square(size: 100, at: (300, 300))\nshape probe = square(size: 10, at: (0, 0))\njelly.soft()\nscene \"s\" for 2.frames:\n    main:\n        probe.move_to((jelly.volume, jelly.stretch))\n        wait 1.frame\n");
    let p = &verts(&r.snapshots[1], 1);
    let c = (
        p.iter().map(|q| q.0).sum::<f64>() / 4.0,
        p.iter().map(|q| q.1).sum::<f64>() / 4.0,
    );
    near(c.0, 10000.0, 100.0);
    near(c.1, 0.0, 0.05);
}

#[test]
fn a_vertex_pinned_to_a_kinematic_shape_follows_it() {
    let r = go("shape plat = square(size: 20, at: (100, 100))\nshape rope = line((100, 100), (300, 100))\nrope.e1.subdivide(8)\nplat.body(kind: kinematic)\nrope.soft(stiffness: 1.0)\nrope.pin(rope.v[1], to: plat)\nscene \"s\" for 2.seconds:\n    main:\n        plat.move_by(200, 0) over 2.seconds ease linear\n");
    let pin = verts(r.snapshots.last().unwrap(), 1)[0];
    near(pin.0, 100.0 + 200.0 * 47.0 / 48.0, 3.0);
    near(pin.1, 100.0, 3.0);
}

#[test]
fn keyframes_win_over_the_simulation() {
    let r = go("shape rope = line((100, 50), (300, 50))\nrope.soft(stiffness: 1.0)\nrope.pin(rope.v[1])\nscene \"s\" for 1.second:\n    timeline:\n        at 0.s:\n            rope.v[2].position.y = 20\n        at 1.s:\n            rope.v[2].position.y = 20\n");
    for s in &r.snapshots {
        near(verts(s, 0)[1].1, 20.0, 1e-6); // the keyed vertex stays where the keys say, not where gravity pulls it
    }
}

#[test]
fn the_same_program_gives_the_same_hashes() {
    let s = format!("shape jelly = rectangle(width: 100, height: 80, at: (300, 100))\njelly.f1.subdivide(3)\njelly.soft(stiffness: 0.7)\nscene \"s\" for 2.seconds:\n{END}");
    assert_eq!(go(&s).hashes, go(&s).hashes);
}

#[test]
fn the_next_scene_starts_from_the_final_shape() {
    let r = go("shape rope = line((100, 50), (300, 50))\nrope.e1.subdivide(4)\nrope.soft(stiffness: 1.0)\nrope.pin(rope.v[1])\nscene \"a\" for 12.frames:\n    main:\n        wait 12.frames\nscene \"b\" for 12.frames:\n    main:\n        wait 12.frames\n");
    let (a, b) = (verts(&r.snapshots[11], 0), verts(&r.snapshots[12], 0));
    for (p, q) in a.iter().zip(&b) {
        near(p.0, q.0, 1e-6);
        near(p.1, q.1, 1e-6);
    }
}

#[test]
fn bad_soft_bodies_are_rejected() {
    let sc = "scene \"s\" for 1.frame:\n    main:\n        wait 1.frame\n";
    assert_eq!(
        err(&format!(
            "shape s = square(size: 50)\ns.body()\ns.soft()\n{sc}"
        ))
        .code,
        "E-PHY-003"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 50)\ns.soft()\ns.body()\n{sc}"
        ))
        .code,
        "E-PHY-003"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 50)\ns.soft(stiffness: 2)\n{sc}"
        ))
        .code,
        "E-PHY-004"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 50)\ns.soft(pressure: -1)\n{sc}"
        ))
        .code,
        "E-PHY-004"
    );
    assert_eq!(
        err(&format!(
            "shape s = square(size: 50)\ns.soft()\ns.pin(s.e1)\n{sc}"
        ))
        .code,
        "E-PHY-004"
    );
    assert_eq!(
        err(&format!("shape s = square(size: 50)\ns.pin(s.v[1])\n{sc}")).code,
        "E-PHY-004"
    );
    assert_eq!(err("shape s = square(size: 50)\nscene \"s\" for 1.frame:\n    main:\n        s.soft()\n        wait 1.frame\n").code, "E-SHP-003");
}
