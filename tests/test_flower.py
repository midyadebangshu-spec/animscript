"""T9: turn a square into a flower in five steps, and snapshot-test the result."""
import json
import math
import os
import unittest
from pathlib import Path

import animscript
from animscript.geometry import circle, square

ROOT = Path(__file__).resolve().parent.parent
GOLDEN = ROOT / "tests" / "golden" / "flower.json"
UNIT = 1e-6


def q(x):
    return int(round(x / UNIT))


def digest(snap):
    """A compact, quantised description of the final frame used as the golden file."""
    out = []
    for sh in snap["shapes"]:
        out.append({
            "id": sh["id"], "fill": sh["fill"], "stroke": sh["stroke"],
            "vertices": [[v[0], q(v[1]), q(v[2])] for v in sh["vertices"]],
            "edges": [[e["id"], e["a"], e["b"], e["kind"]] for e in sh["edges"]],
            "faces": [[f["id"], [[q(c) for p in s for c in p] for s in f["segments"]]] for f in sh["faces"]],
        })
    return out


def flower_steps():
    """The same five steps written against the Python API; returns the live vertex count after each step."""
    sh = square(size=200, at=(400, 300))
    counts = [len(sh.live_ids("v"))]                       # 1. start from a square
    for i in (1, 2, 3, 4):                                 # 2. subdivide every side into four
        sh.e(i).subdivide(4)
    counts.append(len(sh.live_ids("v")))
    ring = [sh.v(i) for i in sh.face_vertex_ids(1)]        # 3. push alternate ring vertices out / in
    for i, v in enumerate(ring):
        a = (225 / 360 - i * 22.5 / 360) * 2 * math.pi
        r = 150 if i % 2 == 0 else 80
        v.move_to(400 + math.cos(a) * r, 300 + math.sin(a) * r)
    counts.append(len(sh.live_ids("v")))
    for i, v in enumerate(ring):                           # 4. bevel the tips
        if i % 2 == 0:
            v.bevel(30)
    counts.append(len(sh.live_ids("v")))
    sh.smooth(2)                                           # 5. Chaikin smoothing
    counts.append(len(sh.live_ids("v")))
    return sh, counts


class Flower(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src = (ROOT / "examples" / "flower.anim").read_text()
        cls.result = animscript.run(cls.src, filename="flower.anim")

    def test_five_steps_through_the_api(self):
        sh, counts = flower_steps()
        self.assertEqual(counts, [4, 16, 16, 24, 96])
        self.assertEqual(sh.warnings, [])
        self.assertEqual(sorted(sh.live_ids("f")), [1])
        self.assertEqual(len(sh.faces[1].loop), 96)

    def test_program_matches_the_api_version(self):
        sh, _ = flower_steps()
        got = {v[0]: (v[1], v[2]) for v in self.result.snapshots[-1]["shapes"][0]["vertices"]}
        want = {i: (v.x, v.y) for i, v in sh.verts.items() if v.state == "live"}
        self.assertEqual(sorted(got), sorted(want))
        for i in want:
            self.assertAlmostEqual(got[i][0], want[i][0], places=6)
            self.assertAlmostEqual(got[i][1], want[i][1], places=6)

    def test_no_diagnostics(self):
        self.assertEqual(animscript.check(self.src, "flower.anim"), [])

    def test_identical_hash_on_repeat(self):
        again = animscript.run(self.src, filename="flower.anim")
        self.assertEqual(self.result.hashes, again.hashes)

    def test_golden_snapshot(self):
        got = {"hash": self.result.hashes[-1], "frame": digest(self.result.snapshots[-1])}
        if os.environ.get("UPDATE_GOLDEN") == "1" or not GOLDEN.exists():
            GOLDEN.write_text(json.dumps(got, indent=1) + "\n")
            if not os.environ.get("UPDATE_GOLDEN"):
                self.fail("golden file was missing and has been written; review and commit it")
        want = json.loads(GOLDEN.read_text())
        self.assertEqual(got["frame"], want["frame"])
        self.assertEqual(got["hash"], want["hash"])


if __name__ == "__main__":
    unittest.main()
