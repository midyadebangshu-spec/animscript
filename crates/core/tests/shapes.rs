//! Shape data-model tests (no parser involved). Written before the implementation.
use animscript_core::*;

fn pos(sh: &Shape, id: u32) -> (f64, f64) {
    let v = &sh.verts[&id];
    (v.x, v.y)
}
fn near(a: (f64, f64), b: (f64, f64)) {
    assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9, "{a:?} != {b:?}");
}
fn pt(p: [f64; 2]) -> (f64, f64) {
    (p[0], p[1])
}
fn code<T: std::fmt::Debug>(r: Result<T, Diagnostic>) -> Diagnostic {
    r.expect_err("expected an error")
}
fn loop_ids(sh: &Shape, f: u32) -> Vec<u32> {
    sh.faces[&f].loop_.iter().map(|x| x.0).collect()
}
fn sq() -> Shape {
    square(200.0, [0.0, 0.0])
}

// ---- Test 1 ---------------------------------------------------------------------------
#[test]
fn t1_square_has_v1_v4_e1_e4_f1_with_correct_connectivity() {
    let s = sq();
    assert_eq!(s.live_ids(Kind::V), [1, 2, 3, 4]);
    assert_eq!(s.live_ids(Kind::E), [1, 2, 3, 4]);
    assert_eq!(s.live_ids(Kind::F), [1]);
    // counter-clockwise on screen, from the top-left
    near(pos(&s, 1), (-100.0, -100.0));
    near(pos(&s, 2), (-100.0, 100.0));
    near(pos(&s, 3), (100.0, 100.0));
    near(pos(&s, 4), (100.0, -100.0));
    // eN runs from vN to vN+1, e4 closes to v1
    let ends: Vec<_> = (1..=4).map(|i| (s.edges[&i].a, s.edges[&i].b)).collect();
    assert_eq!(ends, [(1, 2), (2, 3), (3, 4), (4, 1)]);
    assert_eq!(loop_ids(&s, 1), [1, 2, 3, 4]);
    assert_eq!(s.face_vertex_ids(1), [1, 2, 3, 4]);
    assert!(s.face_area(1) < 0.0, "visually counter-clockwise (y points down)");
    assert!(s.warnings.is_empty());
}

#[test]
fn t1_other_primitives() {
    let r = rectangle(40.0, 20.0, [0.0, 0.0]);
    near(pos(&r, 3), (20.0, 10.0));
    let t = triangle(100.0, [0.0, 0.0]);
    assert_eq!(t.live_ids(Kind::V), [1, 2, 3]);
    assert_eq!(t.faces[&1].loop_.len(), 3);
    let c = circle(8, 10.0, [0.0, 0.0]).unwrap();
    assert_eq!(c.live_ids(Kind::V).len(), 8);
    near(pos(&c, 1), (0.0, -10.0));
    assert!(pos(&c, 2).0 < 0.0, "v2 follows v1 counter-clockwise on screen");
    let p = polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]).unwrap();
    assert_eq!(p.live_ids(Kind::E), [1, 2, 3]);
    let l = line([0.0, 0.0], [5.0, 5.0]);
    assert_eq!((l.live_ids(Kind::V), l.live_ids(Kind::E), l.live_ids(Kind::F)), (vec![1, 2], vec![1], vec![]));
    assert_eq!(code(polygon(&[[0.0, 0.0], [1.0, 1.0]])).code, "E-SHP-005");
    assert_eq!(code(circle(2, 1.0, [0.0, 0.0])).code, "E-SHP-005");
}

// ---- Test 2 ---------------------------------------------------------------------------
#[test]
fn t2_subdividing_e1_into_2_yields_v5_midpoint_and_children() {
    let mut s = sq();
    s.subdivide(e(1), 2, &[]).unwrap();
    near(pos(&s, 5), (-100.0, 0.0));
    for (i, p) in [(1, (-100.0, -100.0)), (2, (-100.0, 100.0)), (3, (100.0, 100.0)), (4, (100.0, -100.0))] {
        near(pos(&s, i), p); // v1..v4 keep IDs and positions
    }
    assert_eq!(s.edges[&1].meta.children, [5, 6]);
    assert_eq!((s.edges[&5].a, s.edges[&5].b), (1, 5));
    assert_eq!((s.edges[&6].a, s.edges[&6].b), (5, 2));
    assert_eq!(s.verts[&5].meta.parent, Some(e(1)));
    assert_eq!(s.edges[&5].meta.parent, Some(e(1)));
    assert_eq!(s.edges[&1].meta.state, State::Replaced);
    assert_eq!(loop_ids(&s, 1), [5, 6, 2, 3, 4]);
    assert_eq!(s.face_vertex_ids(1), [1, 5, 2, 3, 4]);
}

#[test]
fn t2_edge_subdivide_into_n_equal_parts() {
    let mut s = square(300.0, [0.0, 0.0]);
    s.subdivide(e(2), 3, &[]).unwrap();
    near(pos(&s, 5), (-50.0, 150.0));
    near(pos(&s, 6), (50.0, 150.0));
    assert_eq!(s.edges[&2].meta.children, [5, 6, 7]);
}

#[test]
fn t2_face_subdivide_makes_a_grid() {
    let mut s = sq();
    s.subdivide(f(1), 2, &[]).unwrap();
    assert_eq!(s.live_ids(Kind::V).len(), 9);
    assert_eq!(s.live_ids(Kind::F).len(), 4);
    assert_eq!(s.faces[&1].meta.state, State::Replaced);
    assert_eq!(s.faces[&1].meta.children, [2, 3, 4, 5]);
    let centre = s.live_ids(Kind::V).into_iter().filter(|i| pos(&s, *i) == (0.0, 0.0)).count();
    assert_eq!(centre, 1);
    for fid in s.live_ids(Kind::F) {
        assert_eq!(s.faces[&fid].loop_.len(), 4);
        assert!((s.face_area(fid) + 10000.0).abs() < 1e-6);
    }
    assert!(s.warnings.is_empty());
}

#[test]
fn t2_semantic_names_are_aliases() {
    let mut s = sq();
    let names: Vec<String> = ["left", "mid", "right"].iter().map(|x| x.to_string()).collect();
    s.subdivide(e(1), 3, &names).unwrap();
    assert_eq!(s.resolve_name("mid"), Some(e(6)));
    assert_eq!(s.edges[&7].meta.alias.as_deref(), Some("right"));
}

#[test]
fn t2_duplicate_semantic_name_is_e_shp_004_and_changes_nothing() {
    let mut s = sq();
    let n = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    s.subdivide(e(1), 2, &n(&["a", "b"])).unwrap();
    let err = code(s.subdivide(e(2), 2, &n(&["b", "c"])));
    assert_eq!(err.code, "E-SHP-004");
    assert!(!err.helps.is_empty());
    assert_eq!(code(s.subdivide(e(2), 2, &n(&["x", "x"]))).code, "E-SHP-004");
    assert_eq!(s.live_ids(Kind::E), [2, 3, 4, 5, 6]);
    assert_eq!(code(s.subdivide(e(2), 2, &n(&["v9", "y"]))).code, "E-SHP-005");
}

// ---- Test 3 ---------------------------------------------------------------------------
#[test]
fn t3_subdividing_twice_never_reuses_or_renumbers_an_id() {
    let mut s = sq();
    s.subdivide(e(1), 2, &[]).unwrap(); // v5, e5, e6
    s.subdivide(e(5), 2, &[]).unwrap(); // v6, e7, e8  (e5 retired)
    s.subdivide(e(2), 3, &[]).unwrap(); // v7 v8, e9 e10 e11
    assert_eq!(s.verts.keys().copied().collect::<Vec<_>>(), (1..=8).collect::<Vec<_>>());
    assert_eq!(s.edges.keys().copied().collect::<Vec<_>>(), (1..=11).collect::<Vec<_>>());
    near(pos(&s, 1), (-100.0, -100.0));
    near(pos(&s, 6), (-100.0, -50.0));
    assert_eq!((s.edges[&1].meta.state, s.edges[&5].meta.state), (State::Replaced, State::Replaced));
    assert_eq!(s.edges[&5].meta.children, [7, 8]);
    assert_eq!(s.edges[&7].meta.parent, Some(e(5)));
    assert_eq!(loop_ids(&s, 1), [7, 8, 6, 9, 10, 11, 3, 4]);
    assert_eq!(s.face_vertex_ids(1), [1, 6, 5, 2, 7, 8, 3, 4]);
    assert_eq!(code(s.subdivide(e(5), 2, &[])).code, "E-SHP-002");
}

#[test]
fn t3_deleted_ids_are_not_reused() {
    let mut s = sq();
    s.delete(v(2)).unwrap();
    s.subdivide(e(3), 2, &[]).unwrap();
    assert_eq!(*s.verts.keys().max().unwrap(), 5);
    assert_eq!(s.verts[&2].meta.state, State::Deleted);
}

// ---- Test 4 ---------------------------------------------------------------------------
#[test]
fn t4_move_to_on_a_vertex_updates_both_adjacent_edges() {
    let mut s = sq();
    s.move_to(v(1), 0.0, 0.0).unwrap();
    near(pt(s.edge_points(1)[0]), (0.0, 0.0)); // e1 starts at v1
    near(pt(*s.edge_points(4).last().unwrap()), (0.0, 0.0)); // e4 ends at v1
    near(pt(s.edge_points(2)[0]), (-100.0, 100.0));
    s.move_by(v(3), 40.0, -10.0).unwrap();
    near(pos(&s, 3), (140.0, 90.0));
}

#[test]
fn t4_moving_an_edge_or_face_moves_its_vertices() {
    let mut s = sq();
    s.move_by(e(2), 10.0, 0.0).unwrap();
    near(pos(&s, 2), (-90.0, 100.0));
    near(pos(&s, 3), (110.0, 100.0));
    s.move_to(e(2), 0.0, 0.0).unwrap(); // the edge midpoint goes to (0, 0)
    near(pos(&s, 2), (-100.0, 0.0));
    near(pos(&s, 3), (100.0, 0.0));
    let mut s = sq();
    s.move_by(f(1), 5.0, 5.0).unwrap();
    near(pos(&s, 1), (-95.0, -95.0));
    s.move_to(f(1), 0.0, 0.0).unwrap();
    near(pos(&s, 3), (100.0, 100.0));
}

// ---- Test 5 ---------------------------------------------------------------------------
#[test]
fn t5_extrude_creates_the_expected_new_face() {
    let mut s = sq();
    s.extrude(e(1), 40.0).unwrap(); // e1 is the left edge: outward is -x
    near(pos(&s, 5), (-140.0, -100.0));
    near(pos(&s, 6), (-140.0, 100.0));
    let ends: Vec<_> = (5..=7).map(|i| (s.edges[&i].a, s.edges[&i].b)).collect();
    assert_eq!(ends, [(1, 5), (5, 6), (6, 2)]);
    let mut ids = loop_ids(&s, 2);
    ids.sort();
    assert_eq!(ids, [1, 5, 6, 7]);
    assert_eq!(s.face_vertex_ids(2), [1, 5, 6, 2]);
    assert!((s.face_area(2) + 40.0 * 200.0).abs() < 1e-9, "same winding as the original face");
    assert_eq!(s.faces[&2].meta.parent, Some(e(1)));
    assert_eq!(s.verts[&5].meta.parent, Some(e(1)));
    assert_eq!(s.edges[&1].meta.children, [5, 6, 7]);
    assert_eq!(loop_ids(&s, 1), [1, 2, 3, 4]); // the original face is untouched
}

#[test]
fn t5_extrude_other_sides_free_edges_and_errors() {
    let mut s = sq();
    s.extrude(e(3), 10.0).unwrap();
    near(pos(&s, 5), (110.0, 100.0));
    assert_eq!(code(s.extrude(e(1), 0.0)).code, "E-SHP-005");
    s.extrude(e(1), 10.0).unwrap();
    assert_eq!(code(s.extrude(e(1), 10.0)).code, "E-SHP-005"); // now shared by two faces
    let mut l = line([0.0, 0.0], [100.0, 0.0]);
    l.extrude(e(1), 20.0).unwrap();
    near(pos(&l, 3), (0.0, 20.0));
    assert!(l.face_area(1) < 0.0);
}

// ---- Test 6 ---------------------------------------------------------------------------
#[test]
fn t6_delete_leaves_other_ids_intact_and_deleted_id_raises_e_shp_002() {
    let mut s = sq();
    s.delete(v(2)).unwrap();
    assert_eq!(s.live_ids(Kind::V), [1, 3, 4]);
    assert_eq!(s.live_ids(Kind::E), [3, 4]);
    assert_eq!(s.live_ids(Kind::F), Vec::<u32>::new());
    near(pos(&s, 3), (100.0, 100.0));
    let err = code(s.move_to(v(2), 0.0, 0.0));
    assert_eq!(err.code, "E-SHP-002");
    assert!(err.message.contains("deleted"), "{}", err.message);
    assert!(!err.helps.is_empty());
    assert_eq!(code(s.bend(e(1), 0.05)).code, "E-SHP-002");
    assert_eq!(code(s.move_by(f(1), 1.0, 1.0)).code, "E-SHP-002");
    assert_eq!(code(s.vref(9)).code, "E-SHP-002"); // never existed
}

#[test]
fn t6_delete_edge_and_face() {
    let mut s = sq();
    s.delete(e(3)).unwrap();
    assert_eq!(s.live_ids(Kind::E), [1, 2, 4]);
    assert_eq!(s.live_ids(Kind::F), Vec::<u32>::new());
    assert_eq!(s.live_ids(Kind::V), [1, 2, 3, 4]);
    let mut s = sq();
    s.delete(f(1)).unwrap();
    assert_eq!((s.live_ids(Kind::E), s.live_ids(Kind::V)), (vec![1, 2, 3, 4], vec![1, 2, 3, 4]));
}

#[test]
fn t6_history_survives_deletion() {
    let mut s = sq();
    s.subdivide(e(1), 2, &[]).unwrap();
    s.delete(v(5)).unwrap();
    assert_eq!(s.verts[&5].meta.parent, Some(e(1)));
    assert_eq!(s.edges[&1].meta.children[0], 5);
}

// ---- Test 7 ---------------------------------------------------------------------------
#[test]
fn t7_morph_vertex_count_mismatch_is_e_shp_001() {
    let mut a = square(100.0, [0.0, 0.0]);
    a.name = "sq1".into();
    let b = circle(12, 50.0, [0.0, 0.0]).unwrap();
    let err = code(a.check_morph(&b));
    assert_eq!(err.code, "E-SHP-001");
    assert_eq!(err.message, "cannot morph: 4 vs 12 vertices; subdivide `sq1` first");
    assert!(!err.helps.is_empty());
    assert!(a.check_morph(&square(50.0, [0.0, 0.0])).is_ok());
}

// ---- Other operations -----------------------------------------------------------------
#[test]
fn bevel_and_round_replace_a_corner() {
    let mut s = sq();
    s.bevel(v(1), 20.0).unwrap();
    assert_eq!(s.verts[&1].meta.state, State::Replaced);
    assert_eq!(s.verts[&1].meta.children, [5, 6]);
    near(pos(&s, 5), (-100.0, -80.0)); // along e1 (v1 -> v2)
    near(pos(&s, 6), (-80.0, -100.0)); // along e4
    assert_eq!(s.faces[&1].loop_.len(), 5);
    assert_eq!(s.edges[&5].curve, Curve::Straight);
    assert_eq!(s.face_vertex_ids(1), [5, 2, 3, 4, 6]); // same cycle; loop order is kept stable
    let mut r = sq();
    r.round(v(3), 30.0).unwrap();
    assert!(matches!(r.edges[&5].curve, Curve::Quad(_)));
    assert_eq!(code(r.bevel(v(1), 250.0)).code, "E-SHP-005"); // longer than the edge
    assert_eq!(code(r.bevel(v(1), 0.0)).code, "E-SHP-005");
}

#[test]
fn bend_makes_quadratic_and_handles_make_cubic() {
    let mut s = sq();
    s.bend(e(1), 45.0 / 360.0).unwrap();
    let pts = s.edge_points(1);
    assert_eq!(pts.len(), 3);
    near(pt(pts[1]), (-200.0, 0.0)); // positive bend bulges to the right of travel (outward here)
    s.bend(e(1), -45.0 / 360.0).unwrap();
    near(pt(s.edge_points(1)[1]), (0.0, 0.0));
    s.straighten(e(1)).unwrap();
    assert_eq!(s.edge_points(1).len(), 2);
    s.set_handle(e(2), 0.0, 150.0).unwrap();
    near(pt(s.edge_points(2)[1]), (0.0, 150.0));
    s.set_handles(e(3), [100.0, 50.0], [100.0, -50.0]).unwrap();
    assert_eq!(s.edge_points(3).len(), 4);
    assert_eq!(code(s.bend(e(1), 0.25)).code, "E-SHP-005");
}

#[test]
fn curved_edges_follow_their_vertices() {
    let mut s = sq();
    s.bend(e(1), 45.0 / 360.0).unwrap();
    s.move_by(v(1), 0.0, -100.0).unwrap();
    let pts = s.edge_points(1);
    near(pt(pts[0]), (-100.0, -200.0));
    assert!(pts[1][0].abs() > 100.0);
}

#[test]
fn subdividing_a_curved_edge_preserves_the_curve() {
    let mut s = sq();
    s.bend(e(1), 45.0 / 360.0).unwrap();
    s.subdivide(e(1), 2, &[]).unwrap();
    near(pos(&s, 5), (-150.0, 0.0)); // the curve's own point at t = 0.5
    assert!(matches!(s.edges[&5].curve, Curve::Quad(_)));
}

#[test]
fn merge_welds_two_vertices() {
    let mut s = sq();
    s.merge(v(1), v(2), MergeAt::Center).unwrap();
    assert_eq!(s.verts[&2].meta.state, State::Replaced);
    assert_eq!(s.live_ids(Kind::V), [1, 3, 4]);
    near(pos(&s, 1), (-100.0, 0.0));
    assert_eq!(s.live_ids(Kind::E), [2, 3, 4]); // e1 collapsed
    assert_eq!(s.faces[&1].loop_.len(), 3);
    assert_eq!(s.edges[&2].a, 1);
    let mut t = sq();
    t.merge(v(1), v(3), MergeAt::First).unwrap();
    near(pos(&t, 1), (-100.0, -100.0));
    assert_eq!(code(t.merge(v(1), v(1), MergeAt::Center)).code, "E-SHP-005");
}

#[test]
fn smooth_is_chaikin() {
    let mut s = sq();
    s.smooth(1).unwrap();
    assert_eq!(s.live_ids(Kind::V), (5..=12).collect::<Vec<_>>()); // old ids retired, never reused
    assert_eq!(s.live_ids(Kind::E).len(), 8);
    assert_eq!(s.verts[&1].meta.state, State::Replaced);
    assert_eq!(s.live_ids(Kind::F), [1]);
    assert_eq!(s.faces[&1].loop_.len(), 8);
    assert!((s.face_area(1) + (200.0 * 200.0 - 4.0 * 0.5 * 50.0 * 50.0)).abs() < 1e-6);
    let first = pos(&s, 5);
    near(first, (-100.0, -50.0));
    s.smooth(2).unwrap();
    assert_eq!(s.live_ids(Kind::V).len(), 32);
    assert_eq!(code(line([0.0, 0.0], [1.0, 1.0]).smooth(1)).code, "E-SHP-005");
    assert_eq!(code(sq().smooth(0)).code, "E-SHP-005");
}

#[test]
fn smooth_rejects_branching_geometry_with_help() {
    let mut s = sq();
    s.extrude(e(1), 10.0).unwrap();
    let err = code(s.smooth(1));
    assert_eq!(err.code, "E-SHP-005");
    assert!(!err.helps.is_empty());
}

// ---- Warnings -------------------------------------------------------------------------
fn has_w010(s: &Shape) -> bool {
    s.warnings.iter().any(|w| w.code == "W-SHP-010")
}

#[test]
fn self_intersection_after_an_edit_warns() {
    let mut s = sq();
    s.move_to(v(3), 100.0, -100.0).unwrap();
    s.move_to(v(4), 100.0, 100.0).unwrap(); // bow-tie
    assert!(has_w010(&s));
}

#[test]
fn zero_area_face_warns() {
    let mut s = sq();
    for i in 1..=4 {
        s.move_to(v(i), 0.0, 0.0).unwrap();
    }
    assert!(has_w010(&s));
}

#[test]
fn normal_edits_do_not_warn_and_warnings_can_be_disabled() {
    let mut s = sq();
    s.subdivide(e(1), 2, &[]).unwrap();
    s.move_by(v(5), -30.0, 0.0).unwrap();
    assert!(s.warnings.is_empty());
    s.warn_on_edit = false;
    s.move_to(v(3), 100.0, -100.0).unwrap();
    s.move_to(v(4), 100.0, 100.0).unwrap();
    assert!(s.warnings.is_empty());
}
