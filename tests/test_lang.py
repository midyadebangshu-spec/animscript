"""Language-level tests: parser, build-time ops, runtime animation, errors, determinism."""
import math
import unittest

import animscript
from animscript.errors import AnimError

PROJECT = '''project "t":
    anim_version: "0.1"
    canvas: 800x600
    fps: 24
'''


def vtx(snap, shape=0):
    return {v[0]: (v[1], v[2]) for v in snap["shapes"][shape]["vertices"]}


class SyntaxTests(unittest.TestCase):
    def test_task_example_syntax(self):
        it = animscript.load('''shape sq1 = square(size: 200)
sq1.v1.move_to(0, 0)        # vertex 1 of shape sq1
sq1.e2.bend(20.degrees)     # edge 2 of shape sq1
sq1.v[3].move_by(40, -10)   # indexed form
sq1.f1
''')
        sq = it.shapes["sq1"]
        self.assertEqual((sq.verts[1].x, sq.verts[1].y), (0, 0))
        self.assertEqual((sq.verts[3].x, sq.verts[3].y), (140, 90))
        self.assertEqual(sq.edges[2].curve[0], "q")
        self.assertAlmostEqual(sq.edges[2].curve[1][1], math.tan(math.radians(20)) / 2)

    def test_indexed_form_in_loops_and_computed_access(self):
        it = animscript.load('''shape s = square(size: 100)
for i in 1..4:
    s.v[i].move_by(i, 0)
let k = 2 + 1
s.v[k].move_by(0, 5)
''')
        s = it.shapes["s"]
        self.assertEqual([s.verts[i].x for i in (1, 2, 3, 4)], [-49, -48, 53, 54])
        self.assertEqual(s.verts[3].y, 55)

    def test_v1_dot_1_is_rejected_with_a_hint(self):
        with self.assertRaises(AnimError) as cm:
            animscript.load("shape s = square(size: 10)\ns.v1.1.move_to(0, 0)\n")
        self.assertEqual(cm.exception.code, "E-SYN-001")
        self.assertEqual(cm.exception.span.line, 2)
        self.assertTrue(any("s.v[" in h or "s.v1" in h for h in cm.exception.helps))

    def test_tabs_and_bad_dedent(self):
        with self.assertRaises(AnimError) as cm:
            animscript.load("for i in 1..2:\n\tpass\n")
        self.assertEqual(cm.exception.code, "E-LEX-001")
        with self.assertRaises(AnimError) as cm:
            animscript.load("for i in 1..2:\n    pass\n  pass\n")
        self.assertEqual(cm.exception.code, "E-LEX-002")

    def test_units_and_colors(self):
        it = animscript.load('''shape s = square(size: 10)
s.fill = #f0a
s.stroke = #112233
s.stroke_width = 3
let a = 0.25.turns + 90.deg
''')
        s = it.shapes["s"]
        self.assertEqual(s.fill.hex(), "#ff00aa")
        self.assertEqual(s.stroke.hex(), "#112233")
        self.assertEqual(s.stroke_width, 3)

    def test_unit_mismatch_is_E_TYP_002(self):
        for src in ("let a = 1.second + 2\n", "let a = 10.deg + 1.second\n"):
            with self.assertRaises(AnimError) as cm:
                animscript.load(src)
            self.assertEqual(cm.exception.code, "E-TYP-002")
        with self.assertRaises(AnimError) as cm:
            animscript.load("shape s = square(size: 10)\ns.e1.bend(20)\n")
        self.assertEqual(cm.exception.code, "E-TYP-002")


class BuildErrors(unittest.TestCase):
    def test_E_SHP_002_has_span_and_help(self):
        src = "shape s = square(size: 10)\ns.v2.delete()\ns.v2.move_to(1, 1)\n"
        with self.assertRaises(AnimError) as cm:
            animscript.load(src)
        e = cm.exception
        self.assertEqual((e.code, e.span.line), ("E-SHP-002", 3))
        text = e.render(src, "x.anim")
        self.assertIn("error[E-SHP-002]", text)
        self.assertIn("s.v2.move_to(1, 1)", text)
        self.assertIn("help:", text)

    def test_unknown_id_and_unknown_alias(self):
        for src, code in (("shape s = square(size: 10)\ns.v9.move_to(0, 0)\n", "E-SHP-002"),
                          ("shape s = square(size: 10)\ns.nope.move_to(0, 0)\n", "E-NAM-003")):
            with self.assertRaises(AnimError) as cm:
                animscript.load(src)
            self.assertEqual(cm.exception.code, code)

    def test_E_SHP_004_duplicate_name(self):
        with self.assertRaises(AnimError) as cm:
            animscript.load('shape s = square(size: 10)\ns.e1.subdivide(2, names: ["a", "a"])\n')
        self.assertEqual(cm.exception.code, "E-SHP-004")

    def test_names_resolve_in_the_language(self):
        it = animscript.load('''shape s = square(size: 200)
s.e1.subdivide(into: 3, names: ["left", "mid", "right"])
s.mid.bend(10.degrees)
''')
        self.assertEqual(it.shapes["s"].edges[6].curve[0], "q")

    def test_W_SHP_010_is_reported_with_span(self):
        diags = animscript.check('''shape s = square(size: 200)
s.v3.move_to(100, -100)
s.v4.move_to(100, 100)
''')
        warns = [d for d in diags if d.code == "W-SHP-010"]
        self.assertTrue(warns)
        self.assertEqual(warns[0].span.line, 3)

    def test_children_and_parent_are_visible_to_programs(self):
        it = animscript.load('''shape s = square(size: 200)
s.e1.subdivide(2)
let kids = s.e1.children
let p = s.v5.parent
assert len(kids) == 2
assert p == s.e1
assert s.e1.alive == false
assert s.v5.x == -100
''')
        self.assertEqual(it.shapes["s"].edges[1].children, [5, 6])


class StaticChecks(unittest.TestCase):
    def codes(self, src):
        return [d.code for d in animscript.check(src)]

    def test_E_SHP_003_topology_change_at_runtime(self):
        src = PROJECT + '''
shape s = square(size: 100)
scene "x" for 1.second:
    when scene.starts:
        s.e1.subdivide(2)
'''
        diags = animscript.check(src)
        d = [d for d in diags if d.code == "E-SHP-003"]
        self.assertEqual(len(d), 1)
        self.assertEqual(d[0].span.line, 7)
        self.assertTrue(d[0].helps)

    def test_E_SHP_003_also_guards_dynamic_execution(self):
        src = PROJECT + '''
shape s = square(size: 100)
scene "x" for 1.second:
    main:
        s.v1.bevel(5)
'''
        with self.assertRaises(AnimError) as cm:
            animscript.run(src)
        self.assertEqual(cm.exception.code, "E-SHP-003")

    def test_topology_is_allowed_in_setup_and_in_the_build_block(self):
        src = PROJECT + '''
shape s = square(size: 100):
    s.e1.subdivide(2)
scene "x" for 1.second:
    setup:
        s.e2.subdivide(2)
    main:
        wait 1.second
'''
        self.assertEqual([c for c in self.codes(src) if c.startswith("E")], [])
        r = animscript.run(src)
        self.assertEqual(len(r.snapshots[0]["shapes"][0]["vertices"]), 6)

    def test_E_SHP_001_morph_mismatch_static_and_dynamic(self):
        src = PROJECT + '''
shape a = square(size: 100)
shape b = circle(12, radius: 50)
scene "m" for 1.second:
    main:
        a.morph_to(b) over 0.5.seconds
'''
        diags = [d for d in animscript.check(src) if d.code == "E-SHP-001"]
        self.assertEqual(len(diags), 1)
        self.assertEqual(diags[0].message, "cannot morph: 4 vs 12 vertices; subdivide `a` first")
        self.assertEqual(diags[0].span.line, 8)
        with self.assertRaises(AnimError) as cm:
            animscript.run(src)
        self.assertEqual(cm.exception.code, "E-SHP-001")

    def test_clean_program_has_no_diagnostics(self):
        src = PROJECT + '''
shape a = square(size: 100)
shape b = square(size: 50, at: (10, 10))
scene "m" for 1.second:
    main:
        a.morph_to(b) over 0.5.seconds
'''
        self.assertEqual(animscript.check(src), [])


class Runtime(unittest.TestCase):
    def test_vertex_animation_over_time_with_easing_and_commit(self):
        src = PROJECT + '''
shape s = square(size: 100)
scene "a" for 1.second:
    main:
        s.v[2].move_by(0, 10) over 0.5.seconds ease linear
'''
        r = animscript.run(src)
        self.assertEqual(len(r.snapshots), 24)
        y0 = 50
        self.assertAlmostEqual(vtx(r.snapshots[0])[2][1], y0)
        self.assertAlmostEqual(vtx(r.snapshots[6])[2][1], y0 + 5)
        self.assertAlmostEqual(vtx(r.snapshots[11])[2][1], y0 + 10 * 11 / 12)
        self.assertAlmostEqual(vtx(r.snapshots[12])[2][1], y0 + 10)    # committed to rest at its end tick
        self.assertAlmostEqual(vtx(r.snapshots[23])[2][1], y0 + 10)
        self.assertAlmostEqual(vtx(r.snapshots[6])[1][1], -50)         # other vertices untouched

    def test_curve_handles_are_resolved_in_the_snapshot(self):
        src = PROJECT + '''
shape s = square(size: 100)
s.e1.bend(45.degrees)
scene "a" for 1.second:
    main:
        s.v[1].move_by(0, -50) over 0.5.seconds ease linear
'''
        r = animscript.run(src)
        e1 = [e for e in r.snapshots[12]["shapes"][0]["edges"] if e["id"] == 1][0]
        self.assertEqual(e1["kind"], "quadratic")
        self.assertEqual(len(e1["pts"]), 3)
        self.assertAlmostEqual(e1["pts"][0][1], -100)
        f1 = r.snapshots[12]["shapes"][0]["faces"][0]
        self.assertEqual(f1["id"], 1)
        self.assertEqual(len(f1["segments"]), 4)

    def test_together_and_sequencing(self):
        src = PROJECT + '''
shape s = square(size: 100)
scene "a" for 2.seconds:
    main:
        together:
            s.v[1].move_by(10, 0) over 0.5.seconds ease linear
            s.v[3].move_by(0, 10) over 1.second ease linear
        s.v[2].move_by(5, 0) over 0.5.seconds ease linear
'''
        r = animscript.run(src)
        self.assertAlmostEqual(vtx(r.snapshots[12])[1][0], -40)          # first finished
        self.assertAlmostEqual(vtx(r.snapshots[12])[3][1], 50 + 5)        # second halfway
        self.assertAlmostEqual(vtx(r.snapshots[24])[2][0], -50)           # sequel starts when both finished
        self.assertAlmostEqual(vtx(r.snapshots[36])[2][0], -45)

    def test_reactive_rule_moves_vertices(self):
        src = PROJECT + '''
prop bird:
    states: is_scared

shape cloud = square(size: 100)

scene "r" for 2.seconds:
    main:
        wait 1.second
        bird becomes is_scared
    when bird.is_scared:
        cloud.v[2].move_by(0, 10) over 0.5.seconds ease linear
'''
        r = animscript.run(src, trace=True)
        self.assertAlmostEqual(vtx(r.snapshots[23])[2][1], 50)
        self.assertAlmostEqual(vtx(r.snapshots[24])[2][1], 50)            # starts in the very tick the flag is set
        self.assertAlmostEqual(vtx(r.snapshots[30])[2][1], 55)
        self.assertAlmostEqual(vtx(r.snapshots[36])[2][1], 60)
        fire = [t for t in r.trace if " FIRE " in t]
        self.assertEqual(len(fire), 1)
        self.assertIn("tick 024", fire[0])

    def test_zero_time_move_without_over(self):
        src = PROJECT + '''
shape s = square(size: 100)
scene "a" for 1.second:
    main:
        wait 0.5.seconds
        s.v[1].move_to(0, 0)
'''
        r = animscript.run(src)
        self.assertEqual(vtx(r.snapshots[11])[1], (-50, -50))
        self.assertEqual(vtx(r.snapshots[12])[1], (0, 0))

    def test_morph_animates_vertices_to_the_other_shape(self):
        src = PROJECT + '''
shape a = square(size: 100)
shape b = square(size: 200)
scene "m" for 1.second:
    main:
        a.morph_to(b) over 0.5.seconds ease linear
'''
        r = animscript.run(src)
        self.assertEqual(vtx(r.snapshots[12], 0)[1], (-100, -100))
        self.assertAlmostEqual(vtx(r.snapshots[6], 0)[1][0], -75)

    def test_channel_queue_serialises_actions_on_one_vertex(self):
        src = PROJECT + '''
shape s = square(size: 100)
scene "a" for 2.seconds:
    main:
        together:
            s.v[1].move_by(10, 0) over 0.5.seconds ease linear
            s.v[1].move_by(0, 10) over 0.5.seconds ease linear
'''
        r = animscript.run(src)
        v = lambda k: vtx(r.snapshots[k])[1]
        self.assertEqual(v(12), (-40, -50))        # first action done, second just starting
        self.assertEqual(v(24), (-40, -40))

    def test_whenever_and_when_fire_counts(self):
        src = PROJECT + '''
prop p:
    states: on

shape s = square(size: 100)
scene "a" for 3.seconds:
    main:
        repeat 3 times:
            p becomes on
            wait 0.5.seconds
            p clears on
            wait 0.5.seconds
    when p.on:
        s.v[1].move_by(1, 0)
    whenever p.on:
        s.v[2].move_by(1, 0)
'''
        r = animscript.run(src)
        last = vtx(r.snapshots[-1])
        self.assertEqual(last[1][0], -49)      # `when` fired once
        self.assertEqual(last[2][0], -47)      # `whenever` fired three times

    def test_delta_limit_E_RUN_001(self):
        src = PROJECT + '''
prop p:
    states: a, b
scene "loop" for 1.second:
    setup:
        p becomes a
    whenever p.a:
        p clears a
        p becomes b
    whenever p.b:
        p clears b
        p becomes a
'''
        with self.assertRaises(AnimError) as cm:
            animscript.run(src)
        self.assertEqual(cm.exception.code, "E-RUN-001")


class Determinism(unittest.TestCase):
    SRC = PROJECT + '''
shape s = circle(8, radius: 40, at: (400, 300))
s.smooth(1)
scene "a" for 1.second:
    main:
        s.v[2].move_by(0, 10) over 0.5.seconds
        s.e[3].move_by(5, 5) over 0.25.seconds ease ease_out
'''

    def test_same_program_twice_gives_identical_snapshot_hashes(self):
        a = animscript.run(self.SRC)
        b = animscript.run(self.SRC)
        self.assertEqual(a.hashes, b.hashes)
        self.assertEqual(len(a.hashes), 24)
        self.assertNotEqual(a.hashes[0], a.hashes[6])      # the animation really changes the hash

    def test_vertex_positions_are_part_of_the_hash(self):
        a = animscript.run(self.SRC)
        b = animscript.run(self.SRC.replace("move_by(0, 10)", "move_by(0, 10.001)"))
        self.assertNotEqual(a.hashes[-1], b.hashes[-1])
        self.assertEqual(a.hashes[0], b.hashes[0])

    def test_quantisation_makes_tiny_float_noise_irrelevant(self):
        s1 = {"x": 1.0000000001}
        s2 = {"x": 1.0}
        self.assertEqual(animscript.snapshot_hash(s1), animscript.snapshot_hash(s2))


if __name__ == "__main__":
    unittest.main()
