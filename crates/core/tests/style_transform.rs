//! Per-face / per-edge style, the shape transform, and radial_array. Written before the implementation.
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
fn err<T: std::fmt::Debug>(r: Result<T, Diagnostic>) -> Diagnostic {
    r.expect_err("expected an error")
}
fn sq() -> Shape {
    square(200.0, [0.0, 0.0])
}
const RED: Color = [255, 0, 0, 255];
const BLUE: Color = [0, 0, 255, 255];

// ---- per-face and per-edge style ------------------------------------------------------------

#[test]
fn faces_and_edges_start_without_their_own_style() {
    let s = sq();
    assert_eq!(s.faces[&1].fill, None);
    assert_eq!((s.edges[&1].stroke, s.edges[&1].width), (None, None));
}

#[test]
fn set_and_reset_face_fill_and_edge_stroke() {
    let mut s = sq();
    s.set_face_fill(f(1), Some(RED)).unwrap();
    s.set_edge_stroke(e(2), Some(BLUE)).unwrap();
    s.set_edge_width(e(2), Some(5.0)).unwrap();
    assert_eq!(s.faces[&1].fill, Some(RED));
    assert_eq!(
        (s.edges[&2].stroke, s.edges[&2].width),
        (Some(BLUE), Some(5.0))
    );
    s.set_face_fill(f(1), None).unwrap();
    assert_eq!(s.faces[&1].fill, None);
    assert_eq!(s.edges[&1].stroke, None);
}

#[test]
fn style_errors() {
    let mut s = sq();
    assert_eq!(err(s.set_face_fill(e(1), Some(RED))).code, "E-SHP-005"); // a fill belongs to a face
    assert_eq!(err(s.set_edge_stroke(f(1), Some(RED))).code, "E-SHP-005");
    assert_eq!(err(s.set_face_fill(f(9), Some(RED))).code, "E-SHP-002");
    s.delete(f(1)).unwrap();
    assert_eq!(err(s.set_face_fill(f(1), Some(RED))).code, "E-SHP-002");
    assert_eq!(err(sq().set_edge_width(e(1), Some(-1.0))).code, "E-SHP-005");
}

#[test]
fn new_elements_inherit_the_style_of_the_element_they_came_from() {
    // inset: the ring faces and the inner face keep the colour
    let mut s = sq();
    s.set_face_fill(f(1), Some(RED)).unwrap();
    s.inset(f(1), 20.0).unwrap();
    assert!((1..=5).all(|i| s.faces[&i].fill == Some(RED)));
    // mirror copies
    let mut m = polygon(&[
        [400.0, 100.0],
        [520.0, 160.0],
        [580.0, 300.0],
        [400.0, 520.0],
    ])
    .unwrap();
    m.set_face_fill(f(1), Some(BLUE)).unwrap();
    m.mirror(Axis::X(400.0), None, 1e-6).unwrap();
    assert_eq!(m.faces[&2].fill, Some(BLUE));
    // a face grid
    let mut g = sq();
    g.set_face_fill(f(1), Some(RED)).unwrap();
    g.subdivide(f(1), 2, &[]).unwrap();
    assert!(g
        .live_ids(Kind::F)
        .iter()
        .all(|i| g.faces[i].fill == Some(RED)));
    // an edge split in two
    let mut t = sq();
    t.set_edge_stroke(e(1), Some(BLUE)).unwrap();
    t.set_edge_width(e(1), Some(4.0)).unwrap();
    t.subdivide(e(1), 2, &[]).unwrap();
    for i in [5, 6] {
        assert_eq!(
            (t.edges[&i].stroke, t.edges[&i].width),
            (Some(BLUE), Some(4.0))
        );
    }
    assert_eq!(t.edges[&2].stroke, None);
}

#[test]
fn smooth_and_bevel_carry_edge_style_over() {
    let mut s = sq();
    s.set_edge_stroke(e(1), Some(BLUE)).unwrap();
    s.smooth(1).unwrap();
    // inner edge e5 and corner edge e6 were cut from e1
    assert_eq!(s.edges[&5].stroke, Some(BLUE));
    assert_eq!(s.edges[&6].stroke, Some(BLUE));
    assert_eq!(s.edges[&7].stroke, None);
    let mut b = sq();
    b.set_edge_stroke(e(1), Some(BLUE)).unwrap();
    b.bevel(v(1), 20.0).unwrap(); // the new edge copies the lower-numbered edge at the corner (e1)
    assert_eq!(b.edges[&5].stroke, Some(BLUE));
}

// ---- transform ------------------------------------------------------------------------------

#[test]
fn a_new_shape_has_the_identity_transform() {
    let s = sq();
    assert_eq!(
        (s.position, s.scale, s.rotation, s.pivot),
        ([0.0, 0.0], [1.0, 1.0], 0.0, None)
    );
    let t = s.transform();
    assert!(t.is_identity());
    assert_eq!(t.apply([12.3, 4.5]), [12.3, 4.5]); // exactly unchanged
}

#[test]
fn transform_is_scale_then_rotate_about_the_pivot_then_translate() {
    let t = Transform {
        position: [10.0, 0.0],
        scale: [2.0, 2.0],
        rotation: 0.25,
        pivot: [0.0, 0.0],
    };
    near(
        (t.apply([2.0, 0.0])[0], t.apply([2.0, 0.0])[1]),
        (10.0, 4.0),
    ); // (2,0) -> (4,0) -> turned a quarter turn clockwise: (0,4) -> + (10,0)
    let about = Transform {
        position: [0.0, 0.0],
        scale: [1.0, 1.0],
        rotation: 0.25,
        pivot: [100.0, 100.0],
    };
    near(
        (
            about.apply([100.0, 100.0])[0],
            about.apply([100.0, 100.0])[1],
        ),
        (100.0, 100.0),
    ); // the pivot stays put
    near(
        (
            about.apply([110.0, 100.0])[0],
            about.apply([110.0, 100.0])[1],
        ),
        (100.0, 110.0),
    ); // clockwise on screen
    let nonuniform = Transform {
        position: [0.0, 0.0],
        scale: [3.0, 0.5],
        rotation: 0.0,
        pivot: [10.0, 10.0],
    };
    near(
        (
            nonuniform.apply([12.0, 14.0])[0],
            nonuniform.apply([12.0, 14.0])[1],
        ),
        (16.0, 12.0),
    );
}

#[test]
fn the_default_pivot_is_the_bounding_box_centre_once_frozen() {
    let mut s = square(100.0, [400.0, 300.0]);
    assert_eq!(s.transform().pivot, [400.0, 300.0]); // not frozen yet: follows the shape
    s.freeze_pivot();
    assert_eq!(s.pivot, Some([400.0, 300.0]));
    s.move_by(v(1), -100.0, 0.0).unwrap(); // editing afterwards does not move a frozen pivot
    assert_eq!(s.transform().pivot, [400.0, 300.0]);
    s.pivot = Some([0.0, 0.0]);
    s.freeze_pivot(); // an explicit pivot is never replaced
    assert_eq!(s.pivot, Some([0.0, 0.0]));
}

#[test]
fn rest_transform_is_part_of_the_shape_and_survives_copies() {
    let mut s = sq();
    s.position = [5.0, 6.0];
    s.rotation = 0.125;
    let c = s.clone();
    assert_eq!((c.position, c.rotation), ([5.0, 6.0], 0.125));
}

// ---- radial_array ---------------------------------------------------------------------------

fn wedge() -> Shape {
    // the centre, a point on the ray at 0 degrees, and a point on the ray at 45 degrees (clockwise on screen)
    let a = std::f64::consts::FRAC_PI_4;
    polygon(&[
        [400.0, 300.0],
        [500.0, 300.0],
        [400.0 + 100.0 * a.cos(), 300.0 + 100.0 * a.sin()],
    ])
    .unwrap()
}

#[test]
fn a_full_turn_closes_into_a_welded_ring() {
    let mut s = wedge();
    let a0 = s.face_area(1);
    s.radial_array(8, [400.0, 300.0], 1.0, None, 1e-6).unwrap();
    assert_eq!(s.live_ids(Kind::V).len(), 9); // the centre and eight rim points, all shared
    assert_eq!(s.live_ids(Kind::E).len(), 16); // eight spokes and eight rim edges
    assert_eq!(s.live_ids(Kind::F), (1..=8).collect::<Vec<_>>());
    for i in 1..=8 {
        assert!((s.face_area(i) - a0).abs() < 1e-6, "face {i}"); // the same winding and size
    }
    // a spoke (an edge at the centre, v1) has a face on each side; a rim edge is the outer boundary
    for eid in s.live_ids(Kind::E) {
        let uses = s
            .live_ids(Kind::F)
            .iter()
            .filter(|f| s.faces[f].loop_.iter().any(|x| x.0 == eid))
            .count();
        let spoke = s.edges[&eid].a == 1 || s.edges[&eid].b == 1;
        assert_eq!(uses, if spoke { 2 } else { 1 }, "e{eid}");
    }
    assert_eq!(s.faces[&2].meta.parent, Some(f(1)));
    assert_eq!(s.faces[&1].meta.children, [2, 3, 4, 5, 6, 7, 8]);
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
}

#[test]
fn a_partial_angle_spreads_the_copies_over_an_arc() {
    let mut s = wedge();
    s.radial_array(3, [400.0, 300.0], 0.25, None, 1e-6).unwrap(); // copies at 45 and 90 degrees
    assert_eq!(s.live_ids(Kind::F).len(), 3);
    assert_eq!(s.live_ids(Kind::V).len(), 5); // the centre and rim points at 0, 45, 90, 135 degrees
                                              // the last rim point is at 135 degrees: (400 - 70.7, 300 + 70.7)
    let last = s
        .live_ids(Kind::V)
        .into_iter()
        .map(|i| pos(&s, i))
        .find(|p| p.0 < 399.0)
        .unwrap();
    near(
        last,
        (
            400.0 - 100.0 * std::f64::consts::FRAC_PI_4.cos(),
            300.0 + 100.0 * std::f64::consts::FRAC_PI_4.sin(),
        ),
    );
}

#[test]
fn radial_array_keeps_style_and_works_on_a_selection_of_faces() {
    let mut s = wedge();
    s.set_face_fill(f(1), Some(RED)).unwrap();
    s.radial_array(4, [400.0, 300.0], 1.0, None, 1e-6).unwrap();
    assert!(s
        .live_ids(Kind::F)
        .iter()
        .all(|i| s.faces[i].fill == Some(RED)));
    // only f2 of a two-face shape
    let mut t = sq();
    t.extrude(e(3), 100.0).unwrap();
    t.radial_array(2, [0.0, 0.0], 0.5, Some(&[2]), 1e-6)
        .unwrap(); // half a turn: one copy of f2
    assert_eq!(t.live_ids(Kind::F), [1, 2, 3]);
    assert_eq!(t.faces[&3].meta.parent, Some(f(2)));
    near(pos(&t, 7), (-200.0, -100.0)); // v5 (200, 100) turned half a turn about the origin; v3 and v4 weld onto v1 and v2
}

#[test]
fn radial_array_errors() {
    assert_eq!(
        err(wedge().radial_array(1, [0.0, 0.0], 1.0, None, 1e-6)).code,
        "E-SHP-005"
    );
    assert_eq!(
        err(wedge().radial_array(4, [0.0, 0.0], 0.0, None, 1e-6)).code,
        "E-SHP-005"
    );
    assert_eq!(
        err(wedge().radial_array(4, [0.0, 0.0], 1.0, Some(&[9]), 1e-6)).code,
        "E-SHP-002"
    );
    let mut d = wedge();
    d.delete(f(1)).unwrap();
    assert_eq!(
        err(d.radial_array(4, [0.0, 0.0], 1.0, Some(&[1]), 1e-6)).code,
        "E-SHP-002"
    );
}
