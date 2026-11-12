//! Test 9: turn a square into a flower in five steps, and snapshot-test the result.
use animscript_core::*;
use animscript_lang::*;
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn source() -> String {
    std::fs::read_to_string(root().join("examples/flower.anim")).unwrap()
}
fn q(x: f64) -> i64 {
    (x / 1e-6).round_ties_even() as i64
}

/// The same five steps written against the Rust API; returns the shape and the live vertex count after each step.
fn five_steps() -> (Shape, Vec<usize>) {
    let mut sh = square(200.0, [400.0, 300.0]);
    let mut counts = vec![sh.live_ids(Kind::V).len()]; // 1. a square
    for i in 1..=4 {
        sh.subdivide(e(i), 4, &[]).unwrap(); // 2. every side into four
    }
    counts.push(sh.live_ids(Kind::V).len());
    let ring: Vec<u32> = sh.face_vertex_ids(1);
    for (i, id) in ring.iter().enumerate() {
        let a = (225.0 / 360.0 - 22.5 / 360.0 * i as f64) * std::f64::consts::TAU; // 3. out and in alternately
        let r = if i % 2 == 0 { 230.0 } else { 110.0 };
        sh.move_to(v(*id), 400.0 + a.cos() * r, 300.0 + a.sin() * r)
            .unwrap();
    }
    counts.push(sh.live_ids(Kind::V).len());
    for (i, id) in ring.iter().enumerate() {
        if i % 2 == 0 {
            sh.bevel(v(*id), 45.0).unwrap(); // 4. bevel the tips
        }
    }
    counts.push(sh.live_ids(Kind::V).len());
    sh.smooth(2).unwrap(); // 5. Chaikin, twice
    counts.push(sh.live_ids(Kind::V).len());
    (sh, counts)
}

#[test]
fn the_five_steps_have_the_expected_shape_at_every_step() {
    let (sh, counts) = five_steps();
    assert_eq!(counts, [4, 16, 16, 24, 96]);
    assert!(sh.warnings.is_empty(), "{:?}", sh.warnings);
    assert_eq!(sh.live_ids(Kind::F), [1]);
    assert_eq!(sh.faces[&1].loop_.len(), 96);
    // IDs are never reused: 4 corners + 12 (subdivide) + 16 (bevel) + 48 (smooth level 1) + 96 (level 2)
    assert_eq!(sh.verts.len(), 4 + 12 + 16 + 48 + 96);
}

#[test]
fn the_program_produces_the_same_geometry_as_the_api() {
    let (sh, _) = five_steps();
    let r = run(&source(), &RunOptions::default()).unwrap();
    let flower = &r.snapshots.last().unwrap()["shapes"][0];
    assert_eq!(flower["id"], "flower");
    let got: Vec<(u64, f64, f64)> = flower["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x[0].as_u64().unwrap(),
                x[1].as_f64().unwrap(),
                x[2].as_f64().unwrap(),
            )
        })
        .collect();
    let want = sh.live_ids(Kind::V);
    assert_eq!(got.iter().map(|g| g.0 as u32).collect::<Vec<_>>(), want);
    for (g, id) in got.iter().zip(want) {
        let p = sh.pos(id);
        assert!(
            (g.1 - p[0]).abs() < 1e-6 && (g.2 - p[1]).abs() < 1e-6,
            "v{id}: {g:?} vs {p:?}"
        );
    }
}

#[test]
fn the_example_checks_clean_and_is_deterministic() {
    assert!(check(&source()).is_empty());
    let a = run(&source(), &RunOptions::default()).unwrap();
    let b = run(&source(), &RunOptions::default()).unwrap();
    assert_eq!(a.hashes, b.hashes);
    assert_eq!(a.snapshots.len(), 24);
}

fn digest(snap: &Value) -> Value {
    let shapes: Vec<Value> = snap["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sh| {
            json!({
                "id": sh["id"], "fill": sh["fill"], "stroke": sh["stroke"],
                "vertices": sh["vertices"].as_array().unwrap().iter().map(|v| json!([v[0], q(v[1].as_f64().unwrap()), q(v[2].as_f64().unwrap())])).collect::<Vec<_>>(),
                "edges": sh["edges"].as_array().unwrap().iter().map(|e| json!([e["id"], e["a"], e["b"], e["kind"]])).collect::<Vec<_>>(),
                "faces": sh["faces"].as_array().unwrap().iter().map(|f| json!([f["id"], f["segments"].as_array().unwrap().iter().map(|s| s.as_array().unwrap().iter().flat_map(|p| [q(p[0].as_f64().unwrap()), q(p[1].as_f64().unwrap())]).collect::<Vec<_>>()).collect::<Vec<_>>()])).collect::<Vec<_>>(),
            })
        })
        .collect();
    Value::Array(shapes)
}

#[test]
fn golden_snapshot_of_the_finished_flower() {
    let r = run(&source(), &RunOptions::default()).unwrap();
    let got = json!({"hash": format!("{:016x}", r.hashes.last().unwrap()), "frame": digest(r.snapshots.last().unwrap())});
    let path = root().join("crates/lang/tests/golden/flower.json");
    if std::env::var("UPDATE_GOLDEN").is_ok() || !path.exists() {
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        if std::env::var("UPDATE_GOLDEN").is_err() {
            panic!("golden file was missing and has been written; review and commit it");
        }
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        got["frame"], want["frame"],
        "the finished flower changed; run with UPDATE_GOLDEN=1 if that is intended"
    );
    assert_eq!(got["hash"], want["hash"]);
}
