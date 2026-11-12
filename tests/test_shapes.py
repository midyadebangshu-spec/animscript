"""Shape data-model tests (geometry API, no parser involved)."""
import math
import unittest

from animscript.errors import AnimError
from animscript.geometry import circle, line, polygon, rectangle, square, triangle


def live_pos(sh):
    return {i: (v.x, v.y) for i, v in sh.verts.items() if v.state == "live"}


class Base(unittest.TestCase):
    def assertPt(self, p, q, places=9):
        self.assertAlmostEqual(p[0], q[0], places=places)
        self.assertAlmostEqual(p[1], q[1], places=places)

    def assertCode(self, code, fn, *a, **k):
        with self.assertRaises(AnimError) as cm:
            fn(*a, **k)
        self.assertEqual(cm.exception.code, code)
        return cm.exception


class T1_Square(Base):
    def test_square_has_v1_to_v4_e1_to_e4_f1_with_connectivity(self):
        sq = square(size=200)
        self.assertEqual(sorted(sq.verts), [1, 2, 3, 4])
        self.assertEqual(sorted(sq.edges), [1, 2, 3, 4])
        self.assertEqual(sorted(sq.faces), [1])
        # counter-clockwise on screen from top-left: TL, BL, BR, TR
        self.assertEqual(live_pos(sq), {1: (-100, -100), 2: (-100, 100), 3: (100, 100), 4: (100, -100)})
        # eN runs from vN to vN+1, e4 closes to v1
        self.assertEqual([(sq.edges[i].a, sq.edges[i].b) for i in (1, 2, 3, 4)],
                         [(1, 2), (2, 3), (3, 4), (4, 1)])
        # a face is an ordered loop of edge IDs
        self.assertEqual([eid for eid, _ in sq.faces[1].loop], [1, 2, 3, 4])
        self.assertEqual(sq.face_vertex_ids(1), [1, 2, 3, 4])
        self.assertLess(sq.face_area(1), 0)  # visually counter-clockwise (y points down)
        self.assertEqual(sq.warnings, [])

    def test_square_at_and_other_primitives(self):
        sq = square(size=10, at=(50, 60))
        self.assertPt((sq.verts[1].x, sq.verts[1].y), (45, 55))
        r = rectangle(width=40, height=20)
        self.assertPt((r.verts[3].x, r.verts[3].y), (20, 10))
        t = triangle(size=100)
        self.assertEqual(sorted(t.verts), [1, 2, 3])
        self.assertEqual(len(t.faces[1].loop), 3)
        c = circle(n=8, radius=10)
        self.assertEqual(len(c.live_ids("v")), 8)
        self.assertPt((c.verts[1].x, c.verts[1].y), (0, -10))      # v1 at the top
        self.assertLess(c.verts[2].x, 0)                           # then counter-clockwise on screen
        p = polygon([(0, 0), (10, 0), (10, 10)])
        self.assertEqual(sorted(p.edges), [1, 2, 3])
        ln = line((0, 0), (5, 5))
        self.assertEqual((sorted(ln.verts), sorted(ln.edges), sorted(ln.faces)), ([1, 2], [1], []))
        self.assertCode("E-SHP-005", polygon, [(0, 0), (1, 1)])


class T2_Subdivide(Base):
    def test_subdividing_e1_into_2_yields_v5_midpoint_and_children(self):
        sq = square(size=200)
        before = live_pos(sq)
        sq.e(1).subdivide(2)
        self.assertPt((sq.verts[5].x, sq.verts[5].y), (-100, 0))
        for i in (1, 2, 3, 4):                       # v1..v4 keep IDs and positions
            self.assertPt((sq.verts[i].x, sq.verts[i].y), before[i])
        self.assertEqual(sq.edges[1].children, [5, 6])
        self.assertEqual((sq.edges[5].a, sq.edges[5].b), (1, 5))
        self.assertEqual((sq.edges[6].a, sq.edges[6].b), (5, 2))
        self.assertEqual(sq.verts[5].parent, ("e", 1))
        self.assertEqual(sq.edges[5].parent, ("e", 1))
        self.assertFalse(sq.e(1).alive)
        self.assertEqual([eid for eid, _ in sq.faces[1].loop], [5, 6, 2, 3, 4])
        self.assertEqual(sq.face_vertex_ids(1), [1, 5, 2, 3, 4])
        # children / parent are also reachable through the Ref API
        self.assertEqual([r.id for r in sq.e(1).children], [5, 6])
        self.assertEqual(sq.v(5).parent.id, 1)

    def test_edge_subdivide_into_n_equal_parts(self):
        sq = square(size=300)
        sq.e(2).subdivide(3)                          # v2 (-150,150) -> v3 (150,150)
        self.assertPt((sq.verts[5].x, sq.verts[5].y), (-50, 150))
        self.assertPt((sq.verts[6].x, sq.verts[6].y), (50, 150))
        self.assertEqual(sq.edges[2].children, [5, 6, 7])

    def test_subdividing_a_reversed_edge_keeps_the_face_loop_consistent(self):
        sq = square(size=200)
        sq.e(1).subdivide(2)
        sq.e(1 + 4).subdivide(2)                      # e5 (v1->v5) inside a loop
        self.assertEqual(sq.face_vertex_ids(1), [1, 6, 5, 2, 3, 4])

    def test_semantic_names_are_aliases_for_ids(self):
        sq = square(size=200)
        sq.e(1).subdivide(into=3, names=["left", "mid", "right"])
        self.assertEqual(sq.aliases["left"], ("e", 5))
        self.assertEqual(sq.resolve_name("mid").id, 6)
        self.assertEqual(sq.edges[7].alias, "right")

    def test_duplicate_semantic_name_is_E_SHP_004(self):
        sq = square(size=200)
        sq.e(1).subdivide(2, names=["a", "b"])
        err = self.assertCode("E-SHP-004", sq.e(2).subdivide, 2, names=["b", "c"])
        self.assertTrue(err.helps)
        self.assertCode("E-SHP-004", sq.e(2).subdivide, 2, names=["x", "x"])
        # nothing was changed by the failed calls
        self.assertEqual(sorted(sq.edges), [1, 2, 3, 4, 5, 6])

    def test_face_subdivide_grid(self):
        sq = square(size=200)
        sq.f(1).subdivide(2)
        self.assertEqual(len(sq.live_ids("v")), 9)            # 4 corners + 4 edge midpoints + centre
        self.assertEqual(len(sq.live_ids("f")), 4)
        self.assertEqual(sq.faces[1].state, "replaced")
        self.assertEqual(sq.faces[1].children, [2, 3, 4, 5])
        centre = [v for v in sq.verts.values() if v.state == "live" and (v.x, v.y) == (0, 0)]
        self.assertEqual(len(centre), 1)
        for fid in sq.live_ids("f"):
            self.assertEqual(len(sq.faces[fid].loop), 4)
            self.assertAlmostEqual(sq.face_area(fid), -100 * 100 / 1.0 * 1.0, places=6)
        self.assertEqual(sq.warnings, [])


class T3_StableIds(Base):
    def test_subdividing_twice_never_reuses_or_renumbers_an_id(self):
        sq = square(size=200)
        sq.e(1).subdivide(2)          # v5, e5, e6
        sq.e(5).subdivide(2)          # v6, e7, e8   (e5 is now retired)
        sq.e(2).subdivide(3)          # v7, v8, e9, e10, e11
        self.assertEqual(sorted(sq.verts), list(range(1, 9)))
        self.assertEqual(sorted(sq.edges), list(range(1, 12)))
        for i, p in {1: (-100, -100), 2: (-100, 100), 3: (100, 100), 4: (100, -100)}.items():
            self.assertPt((sq.verts[i].x, sq.verts[i].y), p)
        self.assertPt((sq.verts[6].x, sq.verts[6].y), (-100, -50))
        self.assertEqual((sq.edges[1].state, sq.edges[5].state), ("replaced", "replaced"))
        self.assertEqual(sq.edges[1].children, [5, 6])
        self.assertEqual(sq.edges[5].children, [7, 8])
        self.assertEqual(sq.edges[7].parent, ("e", 5))
        self.assertEqual([e for e, _ in sq.faces[1].loop], [7, 8, 6, 9, 10, 11, 3, 4])
        self.assertEqual(sq.face_vertex_ids(1), [1, 6, 5, 2, 7, 8, 3, 4])
        self.assertCode("E-SHP-002", sq.e(5).subdivide, 2)      # retired ids stay retired

    def test_deleted_ids_are_not_reused_by_later_elements(self):
        sq = square(size=200)
        sq.v(2).delete()
        sq.e(3).subdivide(2)
        self.assertEqual(max(sq.verts), 5)
        self.assertEqual(sq.verts[2].state, "deleted")


class T4_Move(Base):
    def test_vertex_move_updates_both_adjacent_edges(self):
        sq = square(size=200)
        sq.v(1).move_to(0, 0)
        self.assertPt(sq.edge_points(1)[0], (0, 0))      # e1 starts at v1
        self.assertPt(sq.edge_points(4)[-1], (0, 0))     # e4 ends at v1
        self.assertPt(sq.edge_points(2)[0], (-100, 100))  # untouched edges unchanged
        sq.v(3).move_by(40, -10)
        self.assertPt((sq.verts[3].x, sq.verts[3].y), (140, 90))

    def test_edge_move_moves_both_vertices(self):
        sq = square(size=200)
        sq.e(2).move_by(10, 0)
        self.assertPt((sq.verts[2].x, sq.verts[2].y), (-90, 100))
        self.assertPt((sq.verts[3].x, sq.verts[3].y), (110, 100))
        sq.e(2).move_to(0, 0)                            # edge midpoint goes to (0, 0)
        self.assertPt((sq.verts[2].x, sq.verts[2].y), (-100, 0))
        self.assertPt((sq.verts[3].x, sq.verts[3].y), (100, 0))

    def test_face_move_moves_all_its_vertices(self):
        sq = square(size=200)
        sq.f(1).move_by(5, 5)
        self.assertPt((sq.verts[1].x, sq.verts[1].y), (-95, -95))
        sq.f(1).move_to(0, 0)
        self.assertPt((sq.verts[3].x, sq.verts[3].y), (100, 100))


class T5_Extrude(Base):
    def test_extrude_edge_creates_vertices_edges_and_a_face(self):
        sq = square(size=200)
        sq.e(1).extrude(by=40)                           # e1 is the left edge: outward is -x
        self.assertPt((sq.verts[5].x, sq.verts[5].y), (-140, -100))
        self.assertPt((sq.verts[6].x, sq.verts[6].y), (-140, 100))
        self.assertEqual([(sq.edges[i].a, sq.edges[i].b) for i in (5, 6, 7)], [(1, 5), (5, 6), (6, 2)])
        self.assertEqual(sorted(e for e, _ in sq.faces[2].loop), [1, 5, 6, 7])
        self.assertEqual(sq.face_vertex_ids(2), [1, 5, 6, 2])
        self.assertLess(sq.face_area(2), 0)               # same winding as the original face
        self.assertAlmostEqual(sq.face_area(2), -40 * 200)
        self.assertEqual((sq.faces[2].parent, sq.verts[5].parent), (("e", 1), ("e", 1)))
        self.assertEqual(sq.edges[1].children, [5, 6, 7])
        # the original face is untouched
        self.assertEqual([e for e, _ in sq.faces[1].loop], [1, 2, 3, 4])
        self.assertAlmostEqual(sq.face_area(1), -200 * 200)

    def test_extrude_other_sides_and_errors(self):
        sq = square(size=200)
        sq.e(3).extrude(by=10)                            # right edge: outward is +x
        self.assertPt((sq.verts[5].x, sq.verts[5].y), (110, 100))
        self.assertCode("E-SHP-005", sq.e(1).extrude, by=0)
        sq.e(1).extrude(by=10)
        self.assertCode("E-SHP-005", sq.e(1).extrude, by=10)   # now shared by two faces

    def test_extrude_a_free_edge_pushes_to_the_right_of_travel(self):
        ln = line((0, 0), (100, 0))
        ln.e(1).extrude(by=20)
        self.assertPt((ln.verts[3].x, ln.verts[3].y), (0, 20))
        self.assertLess(ln.face_area(1), 0)


class T6_Delete(Base):
    def test_delete_vertex_keeps_other_ids_and_raises_E_SHP_002_afterwards(self):
        sq = square(size=200)
        sq.v(2).delete()
        self.assertEqual(sq.live_ids("v"), [1, 3, 4])
        self.assertEqual(sq.live_ids("e"), [3, 4])        # e1 and e2 touched v2
        self.assertEqual(sq.live_ids("f"), [])            # f1 used them
        self.assertEqual(live_pos(sq)[3], (100, 100))
        err = self.assertCode("E-SHP-002", sq.v(2).move_to, 0, 0)
        self.assertIn("deleted", err.message)
        self.assertTrue(err.helps)
        self.assertCode("E-SHP-002", sq.e(1).bend, 0.05)
        self.assertCode("E-SHP-002", sq.f(1).move_by, 1, 1)
        self.assertCode("E-SHP-002", sq.v, 9)              # never existed

    def test_delete_edge_and_face(self):
        sq = square(size=200)
        sq.e(3).delete()
        self.assertEqual(sq.live_ids("e"), [1, 2, 4])
        self.assertEqual(sq.live_ids("f"), [])
        self.assertEqual(sq.live_ids("v"), [1, 2, 3, 4])
        sq2 = square(size=200)
        sq2.f(1).delete()
        self.assertEqual((sq2.live_ids("e"), sq2.live_ids("v")), ([1, 2, 3, 4], [1, 2, 3, 4]))

    def test_history_queries_survive_deletion(self):
        sq = square(size=200)
        sq.e(1).subdivide(2)
        sq.v(5).delete()
        self.assertEqual(sq.v(5).parent.id, 1)
        self.assertEqual(sq.e(1).children[0].id, 5)


class T7_Morph(Base):
    def test_morph_vertex_count_mismatch_is_E_SHP_001(self):
        a = square(size=100)
        a.name = "sq1"
        b = circle(n=12, radius=50)
        b.name = "ring"
        err = self.assertCode("E-SHP-001", a.check_morph, b)
        self.assertEqual(err.message, "cannot morph: 4 vs 12 vertices; subdivide `sq1` first")
        self.assertTrue(err.helps)
        b2 = square(size=50)
        a.check_morph(b2)                                  # equal counts are fine


class T8_Elements(Base):
    def test_bevel_and_round_replace_a_corner(self):
        sq = square(size=200)
        sq.v(1).bevel(20)
        self.assertEqual(sq.verts[1].state, "replaced")
        self.assertEqual(sq.verts[1].children, [5, 6])
        self.assertPt((sq.verts[5].x, sq.verts[5].y), (-100, -80))   # along e1 (v1->v2)
        self.assertPt((sq.verts[6].x, sq.verts[6].y), (-80, -100))   # along e4
        self.assertEqual(len(sq.faces[1].loop), 5)
        self.assertEqual(sq.edges[5].curve, None)
        self.assertEqual(sq.face_vertex_ids(1), [6, 5, 2, 3, 4])
        sq2 = square(size=200)
        sq2.v(3).round(30)
        new_edge = sq2.edges[5]
        self.assertEqual(new_edge.curve[0], "q")
        self.assertCode("E-SHP-005", sq2.v(1).bevel, 100)           # radius must stay inside the edge
        self.assertCode("E-SHP-005", sq2.v(1).bevel, 0)

    def test_bend_makes_a_quadratic_edge(self):
        sq = square(size=200)
        sq.e(1).bend(45 / 360)
        pts = sq.edge_points(1)
        self.assertEqual(len(pts), 3)
        self.assertPt(pts[1], (-200, 0))          # bulges outward (positive bend = right of travel)
        sq.e(1).bend(-45 / 360)
        self.assertPt(sq.edge_points(1)[1], (0, 0))
        sq.e(1).straighten()
        self.assertEqual(len(sq.edge_points(1)), 2)
        sq.e(2).set_handle(0, 150)
        self.assertPt(sq.edge_points(2)[1], (0, 150))
        sq.e(3).set_handles((100, 50), (100, -50))
        self.assertEqual(len(sq.edge_points(3)), 4)
        self.assertCode("E-SHP-005", sq.e(1).bend, 0.25)

    def test_curved_edges_follow_their_vertices(self):
        sq = square(size=200)
        sq.e(1).bend(45 / 360)
        sq.v(1).move_by(0, -100)       # the chord becomes longer; the handle scales with it
        pts = sq.edge_points(1)
        self.assertPt(pts[0], (-100, -200))
        self.assertGreater(abs(pts[1][0]), 100)

    def test_subdividing_a_curved_edge_preserves_the_curve(self):
        sq = square(size=200)
        sq.e(1).bend(45 / 360)
        mid = sq.edge_points(1)
        sq.e(1).subdivide(2)
        self.assertPt((sq.verts[5].x, sq.verts[5].y), (-150, 0))   # point of the curve at t = 0.5
        self.assertEqual(sq.edges[5].curve[0], "q")

    def test_merge_welds_two_vertices(self):
        sq = square(size=200)
        sq.merge(sq.v(1), sq.v(2))
        self.assertEqual(sq.verts[2].state, "replaced")
        self.assertEqual(sq.live_ids("v"), [1, 3, 4])
        self.assertPt((sq.verts[1].x, sq.verts[1].y), (-100, 0))     # centre by default
        self.assertEqual(sq.live_ids("e"), [2, 3, 4])                # e1 collapsed
        self.assertEqual(len(sq.faces[1].loop), 3)
        self.assertEqual(sq.edges[2].a, 1)
        sq2 = square(size=200)
        sq2.v(1).merge(sq2.v(3), at="first")
        self.assertPt((sq2.verts[1].x, sq2.verts[1].y), (-100, -100))
        self.assertCode("E-SHP-005", sq2.v(1).merge, sq2.v(1))

    def test_smooth_is_chaikin(self):
        sq = square(size=200)
        sq.smooth(1)
        self.assertEqual(len(sq.live_ids("v")), 8)
        self.assertEqual(len(sq.live_ids("e")), 8)
        self.assertEqual(sorted(sq.live_ids("v")), list(range(5, 13)))  # the old ids are retired, not reused
        pts = sorted(live_pos(sq).values())
        self.assertPt(pts[0], (-100, -50))
        self.assertEqual(sq.verts[1].state, "replaced")
        self.assertEqual(sq.live_ids("f"), [1])
        self.assertEqual(len(sq.faces[1].loop), 8)
        self.assertAlmostEqual(sq.face_area(1), -(200 * 200 - 4 * 0.5 * 50 * 50), places=6)
        sq.smooth(2)
        self.assertEqual(len(sq.live_ids("v")), 32)
        self.assertCode("E-SHP-005", line((0, 0), (1, 1)).smooth, 1)
        self.assertCode("E-SHP-005", square(size=1).smooth, 0)

    def test_smooth_rejects_branching_geometry_with_help(self):
        sq = square(size=200)
        sq.e(1).extrude(by=10)
        err = self.assertCode("E-SHP-005", sq.smooth, 1)
        self.assertTrue(err.helps)


class T9_Warnings(Base):
    def test_self_intersection_after_an_edit_warns(self):
        sq = square(size=200)
        sq.v(3).move_to(100, -100)
        sq.v(4).move_to(100, 100)                      # bow-tie
        self.assertIn("W-SHP-010", [w.code for w in sq.warnings])

    def test_zero_area_face_warns(self):
        sq = square(size=200)
        for i in (1, 2, 3, 4):
            sq.v(i).move_to(0, i * 0)
        self.assertIn("W-SHP-010", [w.code for w in sq.warnings])

    def test_no_warning_for_normal_edits_and_warn_can_be_disabled(self):
        sq = square(size=200)
        sq.e(1).subdivide(2)
        sq.v(5).move_by(-30, 0)
        self.assertEqual(sq.warnings, [])
        sq.warn_on_edit = False
        sq.v(3).move_to(100, -100)
        sq.v(4).move_to(100, 100)
        self.assertEqual(sq.warnings, [])


if __name__ == "__main__":
    unittest.main()
