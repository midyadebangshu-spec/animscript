//! Language-level tests: parser, build-time ops, runtime animation, errors, determinism.
//! Written before the implementation.
use animscript_core::Kind;
use animscript_lang::*;
use serde_json::Value;
use std::collections::BTreeMap;

const PROJECT: &str =
    "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n";

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
fn load_err(src: &str) -> Diagnostic {
    match load(src) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn run_err(src: &str) -> Diagnostic {
    match run(src, &RunOptions::default()) {
        Ok(_) => panic!("expected an error"),
        Err(d) => d,
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
fn errors(src: &str) -> Vec<String> {
    check(src)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.code)
        .collect()
}
fn with_project(body: &str) -> String {
    format!("{PROJECT}\n{body}")
}

// ---- syntax --------------------------------------------------------------------------------

#[test]
fn task_example_syntax() {
    let it = load(
        "shape sq1 = square(size: 200)\n\
         sq1.v1.move_to(0, 0)        # vertex 1 of shape sq1\n\
         sq1.e2.bend(20.degrees)     # edge 2 of shape sq1\n\
         sq1.v[3].move_by(40, -10)   # indexed form\n\
         sq1.f1\n",
    )
    .unwrap();
    let sq = &it.shapes["sq1"];
    assert_eq!((sq.verts[&1].x, sq.verts[&1].y), (0.0, 0.0));
    assert_eq!((sq.verts[&3].x, sq.verts[&3].y), (140.0, 90.0));
    match &sq.edges[&2].curve {
        animscript_core::Curve::Quad(uw) => near(uw[1], (20f64).to_radians().tan() / 2.0),
        c => panic!("{c:?}"),
    }
}

#[test]
fn indexed_form_in_loops_and_computed_access() {
    let it = load(
        "shape s = square(size: 100)\n\
         for i in 1..4:\n    s.v[i].move_by(i, 0)\n\
         let k = 2 + 1\n\
         s.v[k].move_by(0, 5)\n",
    )
    .unwrap();
    let s = &it.shapes["s"];
    let xs: Vec<f64> = (1..=4).map(|i| s.verts[&i].x).collect();
    assert_eq!(xs, [-49.0, -48.0, 53.0, 54.0]);
    assert_eq!(s.verts[&3].y, 55.0);
}

#[test]
fn v1_dot_1_is_rejected_with_a_hint() {
    let d = load_err("shape s = square(size: 10)\ns.v1.1.move_to(0, 0)\n");
    assert_eq!(d.code, "E-SYN-001");
    assert_eq!(d.span.unwrap().line, 2);
    assert!(d.helps.iter().any(|h| h.contains("s.v[")), "{:?}", d.helps);
}

#[test]
fn tabs_and_bad_dedent() {
    assert_eq!(load_err("for i in 1..2:\n\tpass\n").code, "E-LEX-001");
    assert_eq!(
        load_err("for i in 1..2:\n    pass\n  pass\n").code,
        "E-LEX-002"
    );
    assert_eq!(load_err("let a = \"abc\n").code, "E-LEX-003");
}

#[test]
fn units_and_colors() {
    let it = load(
        "shape s = square(size: 10)\n\
         s.fill = #f0a\n\
         s.stroke = #112233\n\
         s.stroke_width = 3\n\
         let a = 0.25.turns + 90.deg\n\
         assert a == 0.5.turns\n\
         assert 1.5.s == 36.frames\n\
         assert 250.ms * 4 == 1.second\n",
    )
    .unwrap();
    let s = &it.shapes["s"];
    assert_eq!(s.fill, Some([0xff, 0x00, 0xaa, 0xff]));
    assert_eq!(s.stroke, Some([0x11, 0x22, 0x33, 0xff]));
    assert_eq!(s.stroke_width, 3.0);
}

#[test]
fn unit_mismatch_is_e_typ_002() {
    assert_eq!(load_err("let a = 1.second + 2\n").code, "E-TYP-002");
    assert_eq!(load_err("let a = 10.deg + 1.second\n").code, "E-TYP-002");
    let d = load_err("shape s = square(size: 10)\ns.e1.bend(20)\n");
    assert_eq!(d.code, "E-TYP-002");
    assert!(
        d.helps.iter().any(|h| h.contains("degrees")),
        "{:?}",
        d.helps
    );
}

#[test]
fn comments_blank_lines_and_inline_blocks() {
    let it = load(
        "# a comment\n\n## a doc comment\nshape s = square(size: 10)   # trailing\n\
         for i in 1..2: s.v[i].move_by(1, 0)\n\
         if true: s.v3.move_by(1, 0)\n    \n\
         else: s.v3.move_by(100, 0)\n",
    );
    // `else` after an inline `if` on its own line must also parse
    let it = it.unwrap();
    assert_eq!(it.shapes["s"].verts[&1].x, -4.0);
    assert_eq!(it.shapes["s"].verts[&3].x, 6.0);
}

// ---- build-time errors ---------------------------------------------------------------------

#[test]
fn e_shp_002_has_span_help_and_renders() {
    let src = "shape s = square(size: 10)\ns.v2.delete()\ns.v2.move_to(1, 1)\n";
    let d = load_err(src);
    assert_eq!((d.code.as_str(), d.span.unwrap().line), ("E-SHP-002", 3));
    let text = d.render(Some(src), "x.anim");
    assert!(text.contains("error[E-SHP-002]"), "{text}");
    assert!(text.contains("s.v2.move_to(1, 1)"), "{text}");
    assert!(text.contains("help:"), "{text}");
}

#[test]
fn unknown_id_and_unknown_member() {
    assert_eq!(
        load_err("shape s = square(size: 10)\ns.v9.move_to(0, 0)\n").code,
        "E-SHP-002"
    );
    assert_eq!(
        load_err("shape s = square(size: 10)\ns.nope.move_to(0, 0)\n").code,
        "E-NAM-003"
    );
    assert_eq!(load_err("let a = nothing\n").code, "E-NAM-001");
}

#[test]
fn e_shp_004_duplicate_semantic_name() {
    let d = load_err("shape s = square(size: 10)\ns.e1.subdivide(2, names: [\"a\", \"a\"])\n");
    assert_eq!(d.code, "E-SHP-004");
    assert_eq!(d.span.unwrap().line, 2);
}

#[test]
fn semantic_names_resolve_in_the_language() {
    let it = load(
        "shape s = square(size: 200)\n\
         s.e1.subdivide(into: 3, names: [\"left\", \"mid\", \"right\"])\n\
         s.mid.bend(10.degrees)\n",
    )
    .unwrap();
    assert!(matches!(
        it.shapes["s"].edges[&6].curve,
        animscript_core::Curve::Quad(_)
    ));
}

#[test]
fn w_shp_010_is_reported_with_a_span() {
    let diags =
        check("shape s = square(size: 200)\ns.v3.move_to(100, -100)\ns.v4.move_to(100, 100)\n");
    let w: Vec<_> = diags.iter().filter(|d| d.code == "W-SHP-010").collect();
    assert!(!w.is_empty());
    assert_eq!(w[0].span.unwrap().line, 3);
    assert!(!w[0].is_error());
}

#[test]
fn children_parent_and_alive_are_visible_to_programs() {
    let it = load(
        "shape s = square(size: 200)\n\
         s.e1.subdivide(2)\n\
         let kids = s.e1.children\n\
         let p = s.v5.parent\n\
         assert len(kids) == 2\n\
         assert kids[0] == s.e5\n\
         assert p == s.e1\n\
         assert s.e1.alive == false\n\
         assert s.v5.x == -100\n\
         assert s.f1.vertices[1] == s.v5\n",
    )
    .unwrap();
    assert_eq!(it.shapes["s"].edges[&1].meta.children, [5, 6]);
    assert_eq!(it.shapes["s"].live_ids(Kind::V).len(), 5);
}

#[test]
fn failed_assert_is_e_run_004() {
    assert_eq!(
        load_err("shape s = square(size: 10)\nassert s.v1.x == 0\n").code,
        "E-RUN-004"
    );
}

#[test]
fn primitives_accept_named_and_positional_arguments() {
    let it = load(
        "shape a = square(100)\nshape b = rectangle(width: 40, height: 20, at: (10, 10))\n\
         shape c = triangle(size: 30)\nshape d = circle(12, radius: 5)\n\
         shape e = polygon([(0, 0), (10, 0), (5, 8)])\nshape f = line((0, 0), (9, 9))\n",
    )
    .unwrap();
    assert_eq!(it.shapes["d"].live_ids(Kind::V).len(), 12);
    assert_eq!(it.shapes["f"].live_ids(Kind::F).len(), 0);
    assert_eq!(it.shapes["b"].verts[&3].x, 30.0);
    assert_eq!(
        load_err("shape a = square(size: 1)\nshape a = square(size: 2)\n").code,
        "E-NAM-002"
    );
    assert_eq!(load_err("shape a = square(bogus: 1)\n").code, "E-TYP-003");
}

// ---- static checks -------------------------------------------------------------------------

#[test]
fn e_shp_003_topology_change_at_runtime_static() {
    let src = with_project(
        "shape s = square(size: 100)\nscene \"x\" for 1.second:\n    when scene.starts:\n        s.e1.subdivide(2)\n",
    );
    let d: Vec<_> = check(&src)
        .into_iter()
        .filter(|d| d.code == "E-SHP-003")
        .collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].span.unwrap().line, 9);
    assert!(!d[0].helps.is_empty());
}

#[test]
fn e_shp_003_also_guards_dynamic_execution() {
    let src = with_project("shape s = square(size: 100)\nscene \"x\" for 1.second:\n    main:\n        s.v1.bevel(5)\n");
    assert_eq!(run_err(&src).code, "E-SHP-003");
}

#[test]
fn topology_is_allowed_in_setup_and_in_the_build_block() {
    let src = with_project(
        "shape s = square(size: 100):\n    s.e1.subdivide(2)\n\
         scene \"x\" for 1.second:\n    setup:\n        s.e2.subdivide(2)\n    main:\n        wait 1.second\n",
    );
    assert!(errors(&src).is_empty(), "{:?}", check(&src));
    let r = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(vtx(&r.snapshots[0], 0).len(), 6);
}

#[test]
fn e_shp_001_morph_mismatch_static_and_dynamic() {
    let src = with_project(
        "shape a = square(size: 100)\nshape b = circle(12, radius: 50)\n\
         scene \"m\" for 1.second:\n    main:\n        a.morph_to(b) over 0.5.seconds\n",
    );
    let d: Vec<_> = check(&src)
        .into_iter()
        .filter(|d| d.code == "E-SHP-001")
        .collect();
    assert_eq!(d.len(), 1);
    assert_eq!(
        d[0].message,
        "cannot morph: 4 vs 12 vertices; subdivide `a` first"
    );
    assert_eq!(d[0].span.unwrap().line, 10);
    assert_eq!(run_err(&src).code, "E-SHP-001");
}

#[test]
fn clean_program_has_no_diagnostics() {
    let src = with_project(
        "shape a = square(size: 100)\nshape b = square(size: 50, at: (10, 10))\n\
         scene \"m\" for 1.second:\n    main:\n        a.morph_to(b) over 0.5.seconds\n",
    );
    assert!(check(&src).is_empty(), "{:?}", check(&src));
}

#[test]
fn missing_anim_version_is_e_prj_001() {
    let d = load_err("project \"p\":\n    canvas: 10x10\n    fps: 24\n");
    assert_eq!(d.code, "E-PRJ-001");
}

// ---- runtime --------------------------------------------------------------------------------

#[test]
fn vertex_animation_over_time_with_easing_and_commit() {
    let src = with_project(
        "shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        s.v[2].move_by(0, 10) over 0.5.seconds ease linear\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(r.snapshots.len(), 24);
    let y = |k: usize| vtx(&r.snapshots[k], 0)[&2].1;
    near(y(0), 50.0);
    near(y(6), 55.0);
    near(y(11), 50.0 + 10.0 * 11.0 / 12.0);
    near(y(12), 60.0); // committed to rest at its end tick
    near(y(23), 60.0);
    near(vtx(&r.snapshots[6], 0)[&1].1, -50.0); // other vertices untouched
}

#[test]
fn curve_handles_are_resolved_in_the_snapshot() {
    let src = with_project(
        "shape s = square(size: 100)\ns.e1.bend(45.degrees)\nscene \"a\" for 1.second:\n    main:\n        s.v[1].move_by(0, -50) over 0.5.seconds ease linear\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    let sh = &r.snapshots[12]["shapes"][0];
    let e1 = sh["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == 1)
        .unwrap();
    assert_eq!(e1["kind"], "quadratic");
    assert_eq!(e1["pts"].as_array().unwrap().len(), 3);
    near(e1["pts"][0][1].as_f64().unwrap(), -100.0);
    let f1 = &sh["faces"][0];
    assert_eq!(f1["id"], 1);
    assert_eq!(f1["segments"].as_array().unwrap().len(), 4);
}

#[test]
fn together_and_sequencing() {
    let src = with_project(
        "shape s = square(size: 100)\nscene \"a\" for 2.seconds:\n    main:\n        together:\n            s.v[1].move_by(10, 0) over 0.5.seconds ease linear\n            s.v[3].move_by(0, 10) over 1.second ease linear\n        s.v[2].move_by(5, 0) over 0.5.seconds ease linear\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    let at = |k: usize, id: u64| vtx(&r.snapshots[k], 0)[&id];
    near(at(12, 1).0, -40.0); // first finished
    near(at(12, 3).1, 55.0); // second halfway
    near(at(24, 2).0, -50.0); // the sequel starts when both finished
    near(at(36, 2).0, -45.0);
}

#[test]
fn reactive_rule_moves_vertices_in_the_same_tick_the_flag_is_set() {
    let src = with_project(
        "prop bird:\n    states: is_scared\n\nshape cloud = square(size: 100)\n\n\
         scene \"r\" for 2.seconds:\n    main:\n        wait 1.second\n        bird becomes is_scared\n    when bird.is_scared:\n        cloud.v[2].move_by(0, 10) over 0.5.seconds ease linear\n",
    );
    let r = run(&src, &RunOptions { trace: true }).unwrap();
    let y = |k: usize| vtx(&r.snapshots[k], 0)[&2].1;
    near(y(23), 50.0);
    near(y(24), 50.0); // starts in the very tick the flag is set
    near(y(30), 55.0);
    near(y(36), 60.0);
    let fire: Vec<_> = r.trace.iter().filter(|t| t.contains(" FIRE ")).collect();
    assert_eq!(fire.len(), 1, "{:?}", r.trace);
    assert!(fire[0].contains("tick 024"), "{}", fire[0]);
}

#[test]
fn zero_time_move_without_over() {
    let src = with_project(
        "shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        wait 0.5.seconds\n        s.v[1].move_to(0, 0)\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(vtx(&r.snapshots[11], 0)[&1], (-50.0, -50.0));
    assert_eq!(vtx(&r.snapshots[12], 0)[&1], (0.0, 0.0));
}

#[test]
fn morph_animates_vertices_to_the_other_shape() {
    let src = with_project(
        "shape a = square(size: 100)\nshape b = square(size: 200)\nscene \"m\" for 1.second:\n    main:\n        a.morph_to(b) over 0.5.seconds ease linear\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(vtx(&r.snapshots[12], 0)[&1], (-100.0, -100.0));
    near(vtx(&r.snapshots[6], 0)[&1].0, -75.0);
}

#[test]
fn channel_queue_serialises_actions_on_one_vertex() {
    let src = with_project(
        "shape s = square(size: 100)\nscene \"a\" for 2.seconds:\n    main:\n        together:\n            s.v[1].move_by(10, 0) over 0.5.seconds ease linear\n            s.v[1].move_by(0, 10) over 0.5.seconds ease linear\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    assert_eq!(vtx(&r.snapshots[12], 0)[&1], (-40.0, -50.0));
    assert_eq!(vtx(&r.snapshots[24], 0)[&1], (-40.0, -40.0));
}

#[test]
fn when_fires_once_and_whenever_fires_every_rising_edge() {
    let src = with_project(
        "prop p:\n    states: on\n\nshape s = square(size: 100)\nscene \"a\" for 3.seconds:\n    main:\n        repeat 3 times:\n            p becomes on\n            wait 0.5.seconds\n            p clears on\n            wait 0.5.seconds\n    when p.on:\n        s.v[1].move_by(1, 0)\n    whenever p.on:\n        s.v[2].move_by(1, 0)\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    let last = vtx(r.snapshots.last().unwrap(), 0);
    assert_eq!(last[&1].0, -49.0);
    assert_eq!(last[&2].0, -47.0);
}

#[test]
fn delta_limit_is_e_run_001() {
    let src = with_project(
        "prop p:\n    states: a, b\nscene \"loop\" for 1.second:\n    setup:\n        p becomes a\n    whenever p.a:\n        p clears a\n        p becomes b\n    whenever p.b:\n        p clears b\n        p becomes a\n",
    );
    assert_eq!(run_err(&src).code, "E-RUN-001");
}

#[test]
fn sub_frame_duration_warns_w_tim_001() {
    let src = with_project(
        "shape s = square(size: 100)\nscene \"a\" for 1.second:\n    main:\n        s.v[1].move_by(1, 0) over 0.0208.seconds\n",
    );
    let r = run(&src, &RunOptions::default()).unwrap();
    assert!(r.diagnostics.iter().any(|d| d.code == "W-TIM-001"));
}

// ---- determinism ----------------------------------------------------------------------------

const DET: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 800x600\n    fps: 24\n\n\
shape s = circle(8, radius: 40, at: (400, 300))\ns.smooth(1)\n\
scene \"a\" for 1.second:\n    main:\n        s.v[9].move_by(0, 10) over 0.5.seconds\n        s.e[9].move_by(5, 5) over 0.25.seconds ease ease_out\n";

#[test]
fn same_program_twice_gives_identical_snapshot_hashes() {
    let a = run(DET, &RunOptions::default()).unwrap();
    let b = run(DET, &RunOptions::default()).unwrap();
    assert_eq!(a.hashes, b.hashes);
    assert_eq!(a.hashes.len(), 24);
    assert_ne!(
        a.hashes[0], a.hashes[6],
        "the animation really changes the hash"
    );
}

#[test]
fn vertex_positions_are_part_of_the_hash() {
    let a = run(DET, &RunOptions::default()).unwrap();
    let b = run(
        &DET.replace("move_by(0, 10)", "move_by(0, 10.001)"),
        &RunOptions::default(),
    )
    .unwrap();
    assert_ne!(a.hashes.last(), b.hashes.last());
    assert_eq!(a.hashes[0], b.hashes[0]);
}

#[test]
fn quantisation_makes_tiny_float_noise_irrelevant() {
    let a = serde_json::json!({"x": 1.0000000001, "n": 3});
    let b = serde_json::json!({"x": 1.0, "n": 3});
    assert_eq!(snapshot_hash(&a), snapshot_hash(&b));
    assert_ne!(
        snapshot_hash(&a),
        snapshot_hash(&serde_json::json!({"x": 1.001, "n": 3}))
    );
}
