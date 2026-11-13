//! Language-level tests for lambdas, selection sets, broadcast operations, dissolve, inset, flip and
//! mirror. Written before the implementation.
use animscript_core::Kind;
use animscript_lang::*;
use serde_json::Value;
use std::collections::BTreeMap;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n";

fn load_err(src: &str) -> Diagnostic {
    match load(src) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn ok(src: &str) -> Interp {
    match load(src) {
        Ok(i) => i,
        Err(d) => panic!("{}", d.render(Some(src), "test")),
    }
}
fn vtx(snap: &Value, shape: usize) -> BTreeMap<u64, (f64, f64)> {
    snap["shapes"][shape]["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v[0].as_u64().unwrap(),
                (v[1].as_f64().unwrap(), v[2].as_f64().unwrap()),
            )
        })
        .collect()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn body(s: &str) -> String {
    format!("{PROJECT}{s}")
}

// ---- lambdas and where ----------------------------------------------------------------------

#[test]
fn where_filters_with_a_lambda() {
    ok("shape s = square(size: 200, at: (400, 300))\n\
        let right = s.vertices.where(v -> v.x > 400)\n\
        assert len(right) == 2\n\
        assert right[0] == s.v3\n\
        assert right[1] == s.v4\n\
        let low = s.vertices.where(v -> v.y > 300 and v.x < 400)\n\
        assert low.count == 1\n\
        assert low[0] == s.v2\n");
}

#[test]
fn a_lambda_may_take_the_index_and_capture_variables() {
    ok("shape s = square(size: 200)\n\
        let limit = 2\n\
        let early = s.vertices.where((v, i) -> i < limit)\n\
        assert early.count == 2\n\
        assert early[1] == s.v2\n\
        let odd = s.vertices.where((v, i) -> i % 2 == 1)\n\
        assert odd[0] == s.v2 and odd[1] == s.v4\n");
}

#[test]
fn elements_expose_position_length_and_area_to_lambdas() {
    ok("shape s = square(size: 200, at: (400, 300))\n\
        assert s.e1.length == 200\n\
        assert s.e1.x == 300 and s.e1.y == 300\n\
        assert s.f1.x == 400 and s.f1.y == 300\n\
        assert s.f1.area == 40000\n\
        let long = s.edges.where(e -> e.length > 100)\n\
        assert long.count == 4\n\
        let faces = s.faces.where(f -> f.area > 1)\n\
        assert faces.count == 1\n");
}

#[test]
fn lambda_errors_are_reported() {
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.where(5)\n").code,
        "E-TYP-001"
    );
    let d = load_err("shape s = square(size: 1)\nlet a = s.vertices.where(v -> v.x)\n");
    assert_eq!(d.code, "E-TYP-001");
    assert!(!d.helps.is_empty() || d.message.contains("Bool"));
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.where((a, b, c) -> true)\n").code,
        "E-SYN-001"
    );
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.where(v -> w.x > 1)\n").code,
        "E-NAM-001"
    );
    assert_eq!(load_err("let f = v -> v\nlet a = f.x\n").code, "E-TYP-001");
}

// ---- selection methods ----------------------------------------------------------------------

#[test]
fn selection_methods_pick_elements() {
    ok("shape s = square(size: 200, at: (400, 300))\n\
        let all = s.vertices\n\
        assert all.every(2)[0] == s.v1 and all.every(2)[1] == s.v3 and all.every(2).count == 2\n\
        assert all.every(2, offset: 1)[0] == s.v2 and all.every(2, offset: 1)[1] == s.v4\n\
        assert all.first(2).count == 2 and all.first(2)[1] == s.v2\n\
        assert all.last(1)[0] == s.v4\n\
        assert all.skip(3)[0] == s.v4 and all.skip(3).count == 1\n\
        assert all.reverse()[0] == s.v4\n\
        assert all.nearest((300, 200), 2)[0] == s.v1\n\
        assert all.nearest((300, 200), 2)[1] == s.v2\n\
        assert all.nearest(s.v3)[0] == s.v3\n\
        let u = all.first(1).union(all.last(2))\n\
        assert u.count == 3 and u[2] == s.v4\n\
        assert all.union(all).count == 4\n\
        let rest = all.except(all.every(2))\n\
        assert rest.count == 2 and rest[0] == s.v2\n\
        assert all.every(5).count == 1\n");
}

#[test]
fn selection_arguments_are_checked() {
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.every(0)\n").code,
        "E-TYP-001"
    );
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.every()\n").code,
        "E-TYP-003"
    );
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.nonsense()\n").code,
        "E-NAM-003"
    );
    assert_eq!(
        load_err("shape s = square(size: 1)\nlet a = s.vertices.union(3)\n").code,
        "E-TYP-001"
    );
}

// ---- broadcast operations -------------------------------------------------------------------

#[test]
fn operations_broadcast_over_a_selection() {
    let it = ok("shape s = square(size: 100)\n\
                 s.vertices.every(2).move_by(0, 10)\n");
    let s = &it.shapes["s"];
    assert_eq!(
        (s.verts[&1].y, s.verts[&2].y, s.verts[&3].y, s.verts[&4].y),
        (-40.0, 50.0, 60.0, -50.0)
    );
}

#[test]
fn group_moves_move_shared_vertices_once() {
    let it = ok("shape s = square(size: 100)\ns.edges.move_by(5, 0)\n");
    let s = &it.shapes["s"];
    assert_eq!(
        (1..=4).map(|i| s.verts[&i].x).collect::<Vec<_>>(),
        [-45.0, -45.0, 55.0, 55.0]
    );
    // move_to moves the selection as a group: its centroid lands on the target
    let it = ok("shape s = square(size: 100)\ns.vertices.move_to(10, 20)\n");
    let s = &it.shapes["s"];
    assert_eq!((s.verts[&1].x, s.verts[&1].y), (-40.0, -30.0));
    assert_eq!((s.verts[&3].x, s.verts[&3].y), (60.0, 70.0));
}

#[test]
fn bulk_topology_operations() {
    let it = ok("shape s = square(size: 200)\ns.vertices.bevel(20)\n");
    assert_eq!(it.shapes["s"].live_ids(Kind::V).len(), 8);
    assert!(it.diagnostics.is_empty());
    let it = ok("shape s = square(size: 200)\ns.edges.subdivide(2)\n");
    assert_eq!(it.shapes["s"].live_ids(Kind::V).len(), 8);
    let it = ok("shape s = square(size: 200)\ns.edges.subdivide(2)\ns.vertices.where(v -> v.id > 4).round(10)\n");
    assert_eq!(it.shapes["s"].live_ids(Kind::V).len(), 8 + 4);
}

#[test]
fn bulk_operations_skip_elements_an_earlier_step_removed() {
    // deleting v1 also removes e1, e4 and f1, which are later in the same selection
    let it = ok("shape s = square(size: 200)\ns.vertices.first(1).union(s.edges).delete()\n");
    let s = &it.shapes["s"];
    assert!(s.live_ids(Kind::E).is_empty());
    assert_eq!(s.live_ids(Kind::V), [2, 3, 4]);
}

#[test]
fn an_element_that_was_already_dead_before_the_operation_is_still_an_error() {
    let d = load_err("shape s = square(size: 200)\nlet e = s.edges\ns.e1.delete()\ne.delete()\n");
    assert_eq!(d.code, "E-SHP-002");
}

#[test]
fn inset_is_applied_to_each_face_separately() {
    let it = ok("shape s = square(size: 200)\ns.e3.extrude(100)\ns.faces.inset(10)\n");
    let s = &it.shapes["s"];
    assert_eq!(s.live_ids(Kind::F).len(), 2 + 8);
    assert!(it.diagnostics.is_empty(), "{:?}", it.diagnostics);
}

// ---- the new operations in the language -----------------------------------------------------

#[test]
fn dissolve_in_the_language() {
    let it = ok(
        "shape a = square(size: 200)\na.e3.extrude(100)\na.e3.dissolve()\n\
                 shape b = square(size: 200)\nb.e1.subdivide(2)\nb.v5.dissolve()\n",
    );
    assert_eq!(it.shapes["a"].live_ids(Kind::F), [1]);
    assert_eq!(it.shapes["a"].faces[&1].loop_.len(), 6);
    assert_eq!(it.shapes["b"].live_ids(Kind::V), [1, 2, 3, 4]);
    assert_eq!(
        load_err("shape a = square(size: 200)\na.e1.dissolve()\n").code,
        "E-SHP-005"
    );
    assert_eq!(
        load_err("shape a = square(size: 200)\na.f1.dissolve()\n").code,
        "E-SHP-005"
    );
}

#[test]
fn inset_in_the_language() {
    let it = ok("shape s = square(size: 200)\ns.f1.inset(20)\n");
    let s = &it.shapes["s"];
    assert_eq!(s.live_ids(Kind::F).len(), 5);
    assert_eq!((s.verts[&5].x, s.verts[&5].y), (-80.0, -80.0));
    assert_eq!(
        load_err("shape s = square(size: 200)\ns.f1.inset(100)\n").code,
        "E-SHP-005"
    );
    assert_eq!(
        load_err("shape s = square(size: 200)\ns.f1.inset()\n").code,
        "E-TYP-003"
    );
}

#[test]
fn flip_and_mirror_in_the_language() {
    let it = ok("shape s = square(size: 100, at: (500, 0))\ns.flip(x: 400)\n");
    assert_eq!(it.shapes["s"].verts[&1].x, 350.0);
    let it = ok("shape s = square(size: 100)\ns.flip(y: 100)\n");
    assert_eq!(it.shapes["s"].verts[&1].y, 250.0);
    let it = ok("shape s = square(size: 100)\ns.flip(line: ((0, 0), (1, 1)))\n");
    near(it.shapes["s"].verts[&2].x, 50.0);
    near(it.shapes["s"].verts[&2].y, -50.0);

    let it = ok("shape leaf = polygon([(400, 100), (520, 160), (580, 300), (520, 440), (400, 520)])\nleaf.mirror(x: 400)\n");
    let leaf = &it.shapes["leaf"];
    assert_eq!(leaf.live_ids(Kind::V).len(), 8);
    assert_eq!(leaf.live_ids(Kind::F), [1, 2]);
    assert_eq!(leaf.verts[&2].meta.children, [6]);
}

#[test]
fn mirror_of_a_selection_of_faces_and_the_weld_distance() {
    let it = ok("shape s = square(size: 200)\ns.e3.extrude(100)\n\
                 s.mirror(x: 0, faces: s.faces.where(f -> f.x > 0))\n");
    let s = &it.shapes["s"];
    assert_eq!(s.live_ids(Kind::F), [1, 2, 3]);
    assert_eq!(s.faces[&3].meta.parent, Some(animscript_core::f(2)));
    // a vertex within `weld` of the axis is shared
    let it = ok("shape s = polygon([(400.3, 100), (520, 160), (580, 300), (400, 520)])\ns.mirror(x: 400, weld: 0.5)\n");
    assert_eq!(it.shapes["s"].live_ids(Kind::V).len(), 4 + 2);
}

#[test]
fn mirror_and_flip_argument_errors() {
    let pre = "shape s = square(size: 100)\n";
    assert_eq!(load_err(&format!("{pre}s.flip()\n")).code, "E-TYP-003");
    assert_eq!(
        load_err(&format!("{pre}s.flip(x: 1, y: 2)\n")).code,
        "E-TYP-003"
    );
    assert_eq!(
        load_err(&format!("{pre}s.mirror(x: 1, faces: 5)\n")).code,
        "E-TYP-001"
    );
    assert_eq!(
        load_err(&format!("{pre}s.flip(line: (1, 2))\n")).code,
        "E-TYP-001"
    );
    assert_eq!(
        load_err(&format!("{pre}s.mirror(x: 0, faces: s.edges)\n")).code,
        "E-TYP-001"
    );
    assert_eq!(
        load_err(&format!("{pre}s.mirror(x: 0, faces: [])\n")).code,
        "E-SHP-005"
    );
}

// ---- build / runtime split ------------------------------------------------------------------

#[test]
fn new_topology_operations_are_rejected_at_runtime_statically_and_dynamically() {
    for call in [
        "s.f1.inset(5)",
        "s.e1.dissolve()",
        "s.mirror(x: 0)",
        "s.flip(x: 0)",
        "s.faces.inset(5)",
    ] {
        let src = body(&format!(
            "shape s = square(size: 100)\nscene \"x\" for 1.second:\n    main:\n        {call}\n"
        ));
        let d: Vec<_> = check(&src)
            .into_iter()
            .filter(|d| d.code == "E-SHP-003")
            .collect();
        assert_eq!(d.len(), 1, "{call}: {:?}", check(&src));
        let e = run(&src, &RunOptions::default())
            .err()
            .expect("dynamic error");
        assert_eq!(e.code, "E-SHP-003", "{call}");
    }
}

#[test]
fn selection_methods_and_lambdas_are_fine_at_runtime() {
    let src = body(
        "shape s = square(size: 100)\nscene \"x\" for 1.second:\n    main:\n        s.vertices.where(v -> v.x > 0).move_by(0, 10) over 0.5.seconds ease linear\n",
    );
    assert!(check(&src).is_empty(), "{:?}", check(&src));
    let r = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(vtx(&r.snapshots[12], 0)[&3], (50.0, 60.0));
    assert_eq!(vtx(&r.snapshots[12], 0)[&1], (-50.0, -50.0));
}

#[test]
fn a_selection_animates_as_one_action() {
    let src = body("shape s = square(size: 100)\nscene \"x\" for 1.second:\n    main:\n        s.vertices.every(2).move_by(0, 10) over 0.5.seconds ease linear\n");
    let r = run(
        &src,
        &RunOptions {
            trace: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        r.trace.iter().filter(|t| t.contains(" ACT+ ")).count(),
        1,
        "{:?}",
        r.trace
    );
    let at = |k: usize, id: u64| vtx(&r.snapshots[k], 0)[&id];
    near(at(6, 1).1, -45.0);
    near(at(6, 3).1, 55.0);
    near(at(6, 2).1, 50.0);
    near(at(12, 1).1, -40.0);
}

#[test]
fn shared_vertices_of_an_animated_selection_move_once() {
    let src = body("shape s = square(size: 100)\nscene \"x\" for 1.second:\n    main:\n        s.edges.move_by(10, 0) over 0.5.seconds ease linear\n");
    let r = run(&src, &RunOptions::default()).unwrap();
    let last = vtx(r.snapshots.last().unwrap(), 0);
    assert_eq!(
        (1..=4).map(|i| last[&i].0).collect::<Vec<_>>(),
        [-40.0, -40.0, 60.0, 60.0]
    );
}

#[test]
fn group_move_to_at_runtime_lands_the_centroid() {
    let src = body("shape s = square(size: 100)\nscene \"x\" for 1.second:\n    main:\n        s.vertices.move_to(0, 0) over 0.5.seconds ease linear\n        s.vertices.first(2).move_by(1, 1)\n");
    let r = run(&src, &RunOptions::default()).unwrap();
    let last = vtx(r.snapshots.last().unwrap(), 0);
    assert_eq!(last[&3], (50.0, 50.0)); // the centroid was already at (0, 0)
    assert_eq!(last[&1], (-49.0, -49.0));
}

// ---- the leaf example -----------------------------------------------------------------------

fn leaf_source() -> String {
    std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/leaf.anim"),
    )
    .unwrap()
}

#[test]
fn the_leaf_example_uses_mirror_dissolve_smooth_and_inset() {
    let src = leaf_source();
    assert!(check(&src).is_empty(), "{:?}", check(&src));
    let r = run(&src, &RunOptions::default()).unwrap();
    let leaf = r.snapshots[0]["shapes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "leaf")
        .unwrap()
        .clone();
    assert_eq!(leaf["vertices"].as_array().unwrap().len(), 64); // 32 outline + 32 inset
    assert_eq!(leaf["faces"].as_array().unwrap().len(), 33); // inner face + 32 ring faces
                                                             // symmetric about x = 400: the x coordinates mirror each other
    let xs: Vec<f64> = leaf["vertices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v[1].as_f64().unwrap())
        .collect();
    let (lo, hi) = (
        xs.iter().cloned().fold(f64::MAX, f64::min),
        xs.iter().cloned().fold(f64::MIN, f64::max),
    );
    near((lo + hi) / 2.0, 400.0);
    let again = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(r.hashes, again.hashes);
}
