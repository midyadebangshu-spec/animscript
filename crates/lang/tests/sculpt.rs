//! Sculpting from the language.
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
const ROCK: &str = "shape rock = sculpt(size: (240, 240, 240), voxel: 6, at: (400, 300, 100))\n";
fn scene(secs: u32, body: &str) -> String {
    format!("scene \"s\" for {secs}.seconds:\n    main:\n{body}        wait {secs}.seconds\n")
}
fn mesh<'a>(snap: &'a Value, id: &str) -> &'a Value {
    &snap["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap()["mesh"]
}
fn verts(snap: &Value, id: &str) -> Vec<[f64; 3]> {
    mesh(snap, id)["verts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            [
                v[0].as_f64().unwrap(),
                v[1].as_f64().unwrap(),
                v[2].as_f64().unwrap(),
            ]
        })
        .collect()
}
fn faces(snap: &Value, id: &str) -> usize {
    mesh(snap, id)["faces"].as_array().unwrap().len()
}
fn bbox(v: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in v {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    (lo, hi)
}

#[test]
fn an_empty_sculpt_has_no_surface_and_a_sphere_makes_one() {
    let r = go(&format!("{ROCK}{}", scene(1, "")));
    assert_eq!(faces(r.snapshots.last().unwrap(), "rock"), 0);
    let r = go(&format!(
        "{ROCK}rock.add(sdf_sphere(radius: 80).at((400, 300, 100)))\n{}",
        scene(1, "")
    ));
    let s = r.snapshots.last().unwrap();
    assert!(faces(s, "rock") > 200);
    let (lo, hi) = bbox(&verts(s, "rock"));
    for a in 0..3 {
        let c = [400.0, 300.0, 100.0][a];
        assert!(
            (lo[a] - (c - 80.0)).abs() < 8.0 && (hi[a] - (c + 80.0)).abs() < 8.0,
            "axis {a}: {lo:?} {hi:?}"
        );
    }
}

#[test]
fn volume_reports_the_material() {
    let src = format!("{ROCK}rock.add(sdf_sphere(radius: 60).at((400, 300, 100)))\nlet v = rock.volume()\nshape probe = circle(n: 12, radius: 5, at: (v / 1000, 50))\n{}", scene(1, ""));
    let r = go(&src);
    let x = r.snapshots.last().unwrap()["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "probe")
        .unwrap()["vertices"][0][1]
        .as_f64()
        .unwrap();
    // 4/3 pi 60^3 = 904778 -> 904.8 (+ the circle's own offset of one radius)
    assert!((x - 5.0 - 904.8).abs() < 40.0, "{x}");
}

#[test]
fn booleans_equations_and_warps() {
    let src = format!(
        "{ROCK}rock.add(sdf_box(size: (140, 140, 140)).at((400, 300, 100)))\nrock.subtract(sdf_cylinder(radius: 30, height: 300).at((400, 300, 100)))\nrock.add(sdf_expr(\"sqrt((x-400)*(x-400) + (y-300)*(y-300) + (z-100)*(z-100)) - 40\"), smooth: 10)\n{}",
        scene(1, "")
    );
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    assert!(faces(s, "rock") > 300);
    // a cube with a hole through it and a ball in the hole: more faces than a plain ball
    let ball = go(&format!(
        "{ROCK}rock.add(sdf_sphere(radius: 40).at((400, 300, 100)))\n{}",
        scene(1, "")
    ));
    assert!(faces(s, "rock") > faces(ball.snapshots.last().unwrap(), "rock"));
    let tw = go(&format!(
        "{ROCK}rock.add(sdf_box(size: (60, 180, 60)).twist(turns: 0.004).at((400, 300, 100)))\n{}",
        scene(1, "")
    ));
    assert!(faces(tw.snapshots.last().unwrap(), "rock") > 100);
}

#[test]
fn brushes_change_the_volume_in_the_expected_direction() {
    let vol = |extra: &str| -> f64 {
        let src = format!("{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\n{extra}let v = rock.volume()\nshape probe = circle(n: 12, radius: 5, at: (v / 1000, 50))\n{}", scene(1, ""));
        let r = go(&src);
        r.snapshots.last().unwrap()["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "probe")
            .unwrap()["vertices"][0][1]
            .as_f64()
            .unwrap()
            - 5.0
    };
    let base = vol("");
    let top = "(400, 230, 100)";
    assert!(
        vol(&format!(
            "rock.brush(\"draw\", at: {top}, radius: 40, strength: 1)\n"
        )) > base + 5.0
    );
    assert!(
        vol(&format!(
            "rock.brush(\"draw\", at: {top}, radius: 40, strength: 1, invert: true)\n"
        )) < base - 5.0
    );
    assert!(vol("rock.brush(\"clay\", along: [(360, 230, 100), (440, 230, 100)], radius: 30, strength: 1)\n") > base);
    assert!(
        vol(&format!(
            "rock.brush(\"scrape\", at: {top}, radius: 40, strength: 1, passes: 3)\n"
        )) < base
    );
    assert!(
        vol(&format!(
            "rock.brush(\"inflate\", at: {top}, radius: 40, strength: 1)\n"
        )) > base
    );
}

#[test]
fn every_brush_runs() {
    for k in [
        "draw", "clay", "smooth", "inflate", "flatten", "pinch", "scrape", "grease",
    ] {
        let src = format!("{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\nrock.brush(\"{k}\", along: [(370, 230, 100), (430, 230, 100)], radius: 30, strength: 0.5, falloff: \"linear\", mirror: \"x\")\n{}", scene(1, ""));
        let r = go(&src);
        assert!(faces(r.snapshots.last().unwrap(), "rock") > 100, "{k}");
    }
}

#[test]
fn a_stroke_can_be_applied_over_time() {
    let body = "        for i in 0..3:\n            rock.brush(\"clay\", along: [(340, 230, 100), (460, 230, 100)], radius: 30, strength: 1, part: (i * 0.25, (i + 1) * 0.25))\n            wait 2.frames\n";
    let src = format!(
        "{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\n{}",
        scene(1, body)
    );
    let r = go(&src);
    let a = faces(&r.snapshots[0], "rock");
    let last = faces(r.snapshots.last().unwrap(), "rock");
    assert!(a > 0 && last > 0);
    assert_ne!(
        verts(&r.snapshots[0], "rock"),
        verts(r.snapshots.last().unwrap(), "rock")
    );
}

#[test]
fn noise_roughens_deterministically_by_seed() {
    let make = |seed: u32| {
        let r = go(&format!("{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\nrock.noise(kind: \"fbm\", scale: 40, amount: 8, seed: {seed})\n{}", scene(1, "")));
        verts(r.snapshots.last().unwrap(), "rock")
    };
    let (a, b, c) = (make(1), make(1), make(2));
    assert_eq!(a, b);
    assert_ne!(a, c);
    let plain = go(&format!(
        "{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\n{}",
        scene(1, "")
    ));
    assert_ne!(a, verts(plain.snapshots.last().unwrap(), "rock"));
}

#[test]
fn a_vortex_carries_a_ball_round() {
    let centre = |extra: &str| {
        let r = go(&format!(
            "{ROCK}rock.add(sdf_sphere(radius: 30).at((460, 300, 100)))\n{extra}{}",
            scene(1, "")
        ));
        let v = verts(r.snapshots.last().unwrap(), "rock");
        let (lo, hi) = bbox(&v);
        [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ]
    };
    let before = centre("");
    let after = centre("rock.vortex(at: (400, 300, 100), axis: (0, 1, 0), speed: 1.0, radius: 500, time: 1, steps: 8)\n");
    // 60 px from the axis turning about one radian: the ball swings well off its starting side
    assert!((before[0] - 460.0).abs() < 8.0);
    assert!(
        after[0] < before[0] - 15.0 && (after[2] - before[2]).abs() > 30.0,
        "{before:?} {after:?}"
    );
    // a wind
    let wind = centre("rock.wind(by: (30, 0, 0), time: 1)\n");
    assert!(wind[0] > before[0] + 15.0);
    // a curl flow keeps something there
    let flow = centre("rock.flow(kind: \"perlin\", scale: 60, amount: 20, time: 1, seed: 2)\n");
    assert!(flow[0].is_finite());
}

#[test]
fn clear_empties_the_sculpt() {
    let r = go(&format!(
        "{ROCK}rock.add(sdf_sphere(radius: 50).at((400, 300, 100)))\nrock.clear()\n{}",
        scene(1, "")
    ));
    assert_eq!(faces(r.snapshots.last().unwrap(), "rock"), 0);
}

#[test]
fn sculpts_are_lit_and_drawn_like_any_mesh() {
    let src = format!("{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\nrock.fill = #c8a070\nrock.stroke_width = 0\n{}", scene(1, ""));
    let r = go(&src);
    let s = r.snapshots.last().unwrap();
    assert_eq!(mesh(s, "rock")["fill"], "#c8a070");
}

#[test]
fn mistakes_are_reported() {
    assert_eq!(
        err(&format!("{ROCK}rock.brush(\"zzz\", at: (400, 300, 100))\n")).code,
        "E-SCL-002"
    );
    assert_eq!(
        err(&format!("{ROCK}rock.brush(\"clay\")\n")).code,
        "E-SCL-001"
    );
    assert_eq!(
        err(&format!(
            "{ROCK}rock.brush(\"clay\", at: (1, 2, 3), radius: -4)\n"
        ))
        .code,
        "E-SCL-002"
    );
    assert_eq!(
        err(&format!(
            "{ROCK}rock.brush(\"clay\", along: [(0, 0, 0), (1, 1, 1)], part: (0.8, 0.2))\n"
        ))
        .code,
        "E-SCL-002"
    );
    assert_eq!(err(&format!("{ROCK}rock.add(5)\n")).code, "E-SCL-001");
    assert_eq!(
        err(&format!("{ROCK}rock.add(sdf_expr(\"x +\"))\n")).code,
        "E-SCL-002"
    );
    assert_eq!(
        err(&format!("{ROCK}rock.add(sdf_expr(\"q * 2\"))\n")).code,
        "E-SCL-002"
    );
    assert_eq!(
        err("shape big = sculpt(size: (4000, 4000, 4000), voxel: 1)\n").code,
        "E-SCL-004"
    );
    assert_eq!(
        err("shape bad = sculpt(size: (0, 10, 10))\n").code,
        "E-SCL-001"
    );
    assert_eq!(
        err(&format!("{ROCK}rock.noise(kind: \"bogus\")\n")).code,
        "E-SCL-002"
    );
    assert_eq!(
        err(&format!(
            "{ROCK}rock.vortex(at: (0, 0, 0), axis: (0, 0, 0))\n"
        ))
        .code,
        "E-SCL-001"
    );
    assert_eq!(
        err("let s = sdf_sphere(radius: 5)\nlet t = s.bogus()\n").code,
        "E-NAM-003"
    );
}

#[test]
fn sculpting_is_deterministic() {
    let src = format!(
        "{ROCK}rock.add(sdf_sphere(radius: 70).at((400, 300, 100)))\nrock.brush(\"clay\", along: [(340, 230, 100), (460, 230, 100)], radius: 30, strength: 1)\nrock.noise(scale: 30, amount: 5, seed: 3)\nrock.vortex(at: (400, 300, 100), speed: 0.5, radius: 200)\n{}",
        scene(1, "")
    );
    let (a, b) = (go(&src), go(&src));
    assert_eq!(
        a.snapshots.last().unwrap()["shapes"],
        b.snapshots.last().unwrap()["shapes"]
    );
}
