//! Tests for dissolve, inset, mirror / flip and group moves. Written before the implementation.
use animscript_core::*;

fn pos(sh: &Shape, id: u32) -> (f64, f64) {
    let v = &sh.verts[&id];
    (v.x, v.y)
}
fn near(a: (f64, f64), b: (f64, f64)) {
    assert!(
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9,
        "{a:?} != {b:?}"
    );
}
fn pt(p: [f64; 2]) -> (f64, f64) {
    (p[0], p[1])
}
fn err<T: std::fmt::Debug>(r: Result<T, Diagnostic>) -> Diagnostic {
    r.expect_err("expected an error")
}
fn loop_ids(sh: &Shape, f: u32) -> Vec<u32> {
    sh.faces[&f].loop_.iter().map(|x| x.0).collect()
}
fn sq() -> Shape {
    square(200.0, [0.0, 0.0])
}
/// A square with a second square glued to its right side: f1 and f2 share e3.
fn two_squares() -> Shape {
    let mut s = sq();
    s.extrude(e(3), 100.0).unwrap();
    s
}

// ---- dissolve -------------------------------------------------------------------------------

#[test]
fn dissolving_a_shared_edge_merges_two_faces() {
    let mut s = two_squares();
    s.dissolve(e(3)).unwrap();
    assert_eq!(s.live_ids(Kind::F), [1]);
    assert_eq!(s.live_ids(Kind::E), [1, 2, 4, 5, 6, 7]);
    assert_eq!(loop_ids(&s, 1), [4, 1, 2, 5, 6, 7]);
    assert_eq!(s.face_vertex_ids(1), [4, 1, 2, 3, 5, 6]);
    assert!((s.face_area(1) + 60000.0).abs() < 1e-6);
    // the lower id survives, the other is retired and says where it went
    assert_eq!(s.faces[&2].meta.state, State::Replaced);
    assert_eq!(s.faces[&2].meta.children, [1]);
    assert_eq!(s.edges[&3].meta.state, State::Deleted);
    assert_eq!(s.live_ids(Kind::V), [1, 2, 3, 4, 5, 6]); // vertices are untouched
    let d = err(s.move_by(f(2), 1.0, 1.0));
    assert_eq!(d.code, "E-SHP-002");
    assert!(d.helps.iter().any(|h| h.contains("f1")), "{:?}", d.helps);
    assert!(s.warnings.is_empty());
}

#[test]
fn extrude_dissolve_smooth_now_works() {
    let mut s = sq();
    s.extrude(e(1), 40.0).unwrap();
    assert_eq!(err(s.smooth(1)).code, "E-SHP-005"); // the base edge branches the outline
    s.dissolve(e(1)).unwrap();
    s.smooth(1).unwrap();
    assert_eq!(s.live_ids(Kind::F), [1]);
    assert_eq!(s.faces[&1].loop_.len(), 12);
}

#[test]
fn dissolve_edge_errors() {
    let mut s = sq();
    let d = err(s.dissolve(e(1))); // only one face uses it
    assert_eq!(d.code, "E-SHP-005");
    assert!(!d.helps.is_empty());
    assert_eq!(
        err(line([0.0, 0.0], [1.0, 1.0]).dissolve(e(1))).code,
        "E-SHP-005"
    );
    assert_eq!(err(s.dissolve(f(1))).code, "E-SHP-005");
    s.delete(e(2)).unwrap();
    assert_eq!(err(s.dissolve(e(2))).code, "E-SHP-002");
}

#[test]
fn dissolving_a_vertex_merges_its_two_edges() {
    let mut s = sq();
    s.subdivide(e(1), 2, &[]).unwrap(); // v5 splits e1 into e5 (v1-v5) and e6 (v5-v2)
    s.dissolve(v(5)).unwrap();
    assert_eq!(s.verts[&5].meta.state, State::Deleted);
    assert_eq!(s.edges[&6].meta.state, State::Deleted);
    assert_eq!((s.edges[&5].a, s.edges[&5].b), (1, 2)); // the lower edge id survives and is rewired
    assert_eq!(loop_ids(&s, 1), [5, 2, 3, 4]);
    assert_eq!(s.face_vertex_ids(1), [1, 2, 3, 4]);
    assert!((s.face_area(1) + 40000.0).abs() < 1e-9);
    assert!(s.warnings.is_empty());
}

#[test]
fn dissolve_vertex_errors() {
    let mut s = sq();
    s.extrude(e(1), 10.0).unwrap();
    assert_eq!(err(s.dissolve(v(1))).code, "E-SHP-005"); // three edges
    let mut t = sq();
    t.subdivide(e(1), 2, &[]).unwrap();
    t.bend(e(5), 0.05).unwrap();
    let d = err(t.dissolve(v(5))); // would lose the curve
    assert_eq!(d.code, "E-SHP-005");
    assert!(!d.helps.is_empty());
}

// ---- inset ----------------------------------------------------------------------------------

#[test]
fn inset_makes_an_inner_face_and_a_ring() {
    let mut s = sq();
    s.inset(f(1), 20.0).unwrap();
    near(pos(&s, 5), (-80.0, -80.0));
    near(pos(&s, 6), (-80.0, 80.0));
    near(pos(&s, 7), (80.0, 80.0));
    near(pos(&s, 8), (80.0, -80.0));
    for i in 1..=4 {
        assert_eq!(s.verts[&i].meta.state, State::Live); // the outer corners are untouched
    }
    assert_eq!(s.live_ids(Kind::F), [1, 2, 3, 4, 5]);
    assert_eq!(s.live_ids(Kind::E).len(), 12);
    // the original face keeps its ID and becomes the inner face
    assert_eq!(loop_ids(&s, 1), [9, 10, 11, 12]);
    assert_eq!(s.face_vertex_ids(1), [5, 6, 7, 8]);
    assert!((s.face_area(1) + 160.0 * 160.0).abs() < 1e-9);
    // one ring face per original edge, same winding as the original
    assert_eq!(s.face_vertex_ids(2), [1, 2, 6, 5]);
    let ring: f64 = (2..=5).map(|i| s.face_area(i)).sum();
    assert!((ring + (40000.0 - 25600.0)).abs() < 1e-6);
    assert!((2..=5).all(|i| s.face_area(i) < 0.0));
    assert_eq!(s.verts[&5].meta.parent, Some(f(1)));
    assert_eq!(s.faces[&2].meta.parent, Some(f(1)));
    assert!(s.warnings.is_empty());
}

#[test]
fn inset_works_on_a_triangle_and_a_face_next_to_another() {
    let mut t = triangle(100.0, [0.0, 0.0]);
    t.inset(f(1), 10.0).unwrap();
    assert_eq!(t.live_ids(Kind::F).len(), 4);
    assert!(t.face_area(1).abs() < triangle(100.0, [0.0, 0.0]).face_area(1).abs());
    let mut s = two_squares();
    s.inset(f(2), 10.0).unwrap(); // shares e3 with f1, which must stay valid
    assert_eq!(loop_ids(&s, 1), [1, 2, 3, 4]);
    assert!(s.warnings.is_empty());
}

#[test]
fn inset_errors() {
    let mut s = sq();
    assert_eq!(err(s.inset(f(1), 100.0)).code, "E-SHP-005"); // would collapse the face
    assert_eq!(err(s.inset(f(1), 0.0)).code, "E-SHP-005");
    assert_eq!(err(s.inset(f(1), -5.0)).code, "E-SHP-005");
    assert_eq!(err(s.inset(e(1), 5.0)).code, "E-SHP-005");
    s.bend(e(1), 0.05).unwrap();
    let d = err(s.inset(f(1), 5.0));
    assert_eq!(d.code, "E-SHP-005");
    assert!(!d.helps.is_empty());
    assert_eq!(s.live_ids(Kind::F), [1]); // nothing changed
}

// ---- flip -----------------------------------------------------------------------------------

#[test]
fn flip_reflects_in_place_and_keeps_the_winding() {
    let mut s = square(100.0, [500.0, 0.0]);
    s.flip(Axis::X(400.0)).unwrap();
    near(pos(&s, 1), (350.0, -50.0));
    near(pos(&s, 2), (350.0, 50.0));
    near(pos(&s, 3), (250.0, 50.0));
    near(pos(&s, 4), (250.0, -50.0));
    assert_eq!(s.live_ids(Kind::V), [1, 2, 3, 4]); // nothing renumbered
    assert_eq!(s.face_vertex_ids(1), [1, 4, 3, 2]); // loop reversed so it is still counter-clockwise
    assert!((s.face_area(1) + 10000.0).abs() < 1e-9);
    assert!(s.warnings.is_empty());
}

#[test]
fn flip_reflects_curves_and_other_axes() {
    let mut s = square(100.0, [500.0, 0.0]);
    s.bend(e(1), 45.0 / 360.0).unwrap(); // control point at (400, 0): on the axis
    s.flip(Axis::X(400.0)).unwrap();
    near(pt(s.edge_points(1)[1]), (400.0, 0.0));
    let mut t = square(100.0, [0.0, 0.0]);
    t.flip(Axis::Y(100.0)).unwrap(); // y -> 200 - y
    near(pos(&t, 1), (-50.0, 250.0));
    let mut u = square(100.0, [0.0, 0.0]);
    u.flip(Axis::Line([0.0, 0.0], [1.0, 1.0])).unwrap(); // the diagonal swaps x and y
    near(pos(&u, 1), (-50.0, -50.0));
    near(pos(&u, 2), (50.0, -50.0));
}

// ---- mirror ---------------------------------------------------------------------------------

fn half_leaf() -> Shape {
    polygon(&[
        [400.0, 100.0],
        [520.0, 160.0],
        [580.0, 300.0],
        [520.0, 440.0],
        [400.0, 520.0],
    ])
    .unwrap()
}

#[test]
fn mirror_adds_a_welded_copy_across_the_axis() {
    let mut s = half_leaf();
    s.mirror(Axis::X(400.0), None, 1e-6).unwrap();
    assert_eq!(s.live_ids(Kind::V), [1, 2, 3, 4, 5, 6, 7, 8]); // v1 and v5 sit on the axis: welded, not copied
    near(pos(&s, 6), (280.0, 160.0));
    near(pos(&s, 7), (220.0, 300.0));
    near(pos(&s, 8), (280.0, 440.0));
    assert_eq!(s.live_ids(Kind::E), (1..=9).collect::<Vec<_>>()); // e5 lies on the axis and is shared
    assert_eq!((s.edges[&6].a, s.edges[&6].b), (1, 6));
    assert_eq!((s.edges[&9].a, s.edges[&9].b), (8, 5));
    assert_eq!(s.live_ids(Kind::F), [1, 2]);
    // same winding as the original, and the shared seam is walked in opposite directions
    assert!((s.face_area(2) - s.face_area(1)).abs() < 1e-9);
    let dir = |f: u32| s.faces[&f].loop_.iter().find(|x| x.0 == 5).unwrap().1;
    assert_eq!(dir(1), -dir(2));
    // parents: the mirror image records the original (same kind, so it is a child)
    assert_eq!(s.verts[&6].meta.parent, Some(v(2)));
    assert_eq!(s.verts[&2].meta.children, [6]);
    assert_eq!(s.edges[&7].meta.parent, Some(e(2)));
    assert_eq!(s.faces[&2].meta.parent, Some(f(1)));
    assert_eq!(s.faces[&1].meta.children, [2]);
    assert!(s.warnings.is_empty());
}

#[test]
fn mirror_then_dissolve_the_seam_then_smooth() {
    let mut s = half_leaf();
    s.mirror(Axis::X(400.0), None, 1e-6).unwrap();
    s.dissolve(e(5)).unwrap();
    assert_eq!(s.live_ids(Kind::F), [1]);
    assert_eq!(s.faces[&1].loop_.len(), 8);
    s.smooth(2).unwrap();
    assert_eq!(s.live_ids(Kind::F), [1]);
    assert_eq!(s.faces[&1].loop_.len(), 32);
    // the result is left-right symmetric: the bounding box centre sits on the axis
    assert!((s.bbox_center()[0] - 400.0).abs() < 1e-9);
}

#[test]
fn mirror_reflects_curves() {
    let mut s = half_leaf();
    s.bend(e(2), 20.0 / 360.0).unwrap();
    s.mirror(Axis::X(400.0), None, 1e-6).unwrap();
    let orig = s.edge_points(2);
    let copy = s.edge_points(7);
    assert_eq!(copy.len(), 3);
    near(pt(copy[1]), (800.0 - orig[1][0], orig[1][1]));
    assert!(matches!(s.edges[&7].curve, Curve::Quad(_)));
}

#[test]
fn mirror_a_selection_of_faces_only() {
    let mut s = sq();
    s.extrude(e(3), 100.0).unwrap(); // f2 is the extruded face to the right of f1
    s.mirror(Axis::X(0.0), Some(&[2]), 1e-6).unwrap(); // mirror only f2 across x = 0
    assert_eq!(s.live_ids(Kind::F), [1, 2, 3]);
    assert_eq!(s.faces[&3].meta.parent, Some(f(2)));
    assert_eq!(loop_ids(&s, 1), [1, 2, 3, 4]); // f1 untouched
    let xs: Vec<f64> = s.face_vertex_ids(3).iter().map(|i| pos(&s, *i).0).collect();
    assert!(xs.iter().any(|x| (*x + 200.0).abs() < 1e-9), "{xs:?}");
}

#[test]
fn mirror_and_flip_errors() {
    let mut s = half_leaf();
    assert_eq!(
        err(s.mirror(Axis::X(400.0), Some(&[9]), 1e-6)).code,
        "E-SHP-002"
    );
    s.delete(f(1)).unwrap();
    assert_eq!(
        err(s.mirror(Axis::X(400.0), Some(&[1]), 1e-6)).code,
        "E-SHP-002"
    );
    let mut on_axis = line([400.0, 0.0], [400.0, 100.0]);
    let d = err(on_axis.mirror(Axis::X(400.0), None, 1e-6));
    assert_eq!(d.code, "E-SHP-005"); // everything is on the axis: nothing to mirror
    assert!(!d.helps.is_empty());
    assert_eq!(
        err(sq().flip(Axis::Line([1.0, 1.0], [1.0, 1.0]))).code,
        "E-SHP-005"
    ); // a line needs two points
}

// ---- group moves ----------------------------------------------------------------------------

#[test]
fn group_move_moves_shared_vertices_once() {
    let mut s = sq();
    s.move_group_by(&[e(1), e(2)], 10.0, 0.0).unwrap(); // e1 and e2 share v2
    near(pos(&s, 1), (-90.0, -100.0));
    near(pos(&s, 2), (-90.0, 100.0));
    near(pos(&s, 3), (110.0, 100.0));
    near(pos(&s, 4), (100.0, -100.0));
    near(pt(s.group_anchor(&[v(1), v(3)])), (10.0, 0.0)); // measured after the move
    assert_eq!(
        err(s.move_group_by(&[v(1), v(9)], 1.0, 1.0)).code,
        "E-SHP-002"
    );
}
