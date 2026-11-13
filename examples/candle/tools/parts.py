"""Everything the animation needs on top of the base painting (art3): expression variants, the arm, the light layers."""
import math, sys
sys.argv = sys.argv[:1]
import painting
from painting import H, mk, INK, TH, MP, MPe, MPh, SKINB, SKIN_SH

for name, z in [("mouth_flat", 31), ("mouth_o", 31), ("mouth_grin", 31),
                ("brow_neutral", 33), ("brow_up", 33), ("brow_worry", 33),
                ("eye_half", 34), ("eye_wide", 34), ("eye_closed", 34), ("eye_happy", 34),
                ("face_light", 36), ("arm_up", 22), ("arm_fore", 23), ("dusk", 69), ("glow", 70)]:
    H.layer(name, z)

# ------------------------------------------------------------------ eyes (mask coordinates, drawn a little larger: MPe)
EYE_X = [368, 380, 394, 408, 422, 430]
UP0 = [308, 296, 289, 286, 287, 293]
LO0 = [310, 320, 326, 328, 326, 320]
lid_b = H.brush("ink", 2.9, INK)
low_b = H.brush("ink", 1.8, INK)

def arcs(ou, ol, c=308):
    return ([(x, c + (y - c) * ou) for x, y in zip(EYE_X, UP0)], [(x, c + (y - c) * ol) for x, y in zip(EYE_X, LO0)])

def eye(layer, ou, ol, iris_k=1.0, pupil_k=1.0, looking=0):
    up, lo = arcs(ou, ol)
    H.fill(layer, MPe(up + lo[::-1]), mk("marker", 6, ["#fffaf0"]), angle=-15, rag=0.5, grow=-1)
    cy = (up[3][1] + lo[3][1]) / 2
    half = (lo[3][1] - up[3][1]) / 2
    iry = max(2.5, min(13, 0.92 * half) * iris_k)
    cx = 410 + looking
    iris = [(cx + 10 * math.cos(a), cy + iry * math.sin(a)) for a in [i * math.pi / 4 for i in range(8)]]
    H.fill(layer, MPe(iris), mk("marker", 5, ["#a8723c", "#9a6a3a"]), angle=80, rag=0.4, grow=-1)
    prx, pry = 5.5 * pupil_k, max(1.8, iry * 0.55 * pupil_k)
    H.fill(layer, MPe([(cx + prx * math.cos(a), cy + pry * math.sin(a)) for a in [i * math.pi / 4 for i in range(8)]]), mk("marker", 3.5, ["#2a1a14"]), angle=80, rag=0.3, grow=-1)
    H.fill(layer, MPe([(cx - 3, cy - pry * 0.9), (cx + 2, cy - pry * 0.9), (cx + 2, cy - pry * 0.9 + 4), (cx - 3, cy - pry * 0.9 + 4)]), mk("marker", 2.5, ["#ffffff"]), angle=0, rag=0.2, grow=-1)
    H.line(layer, MPe(up + [(436, up[-1][1] - 8)]), lid_b, over=(2, 0), wobble=0.4, pmin=0.7, pmax=1.0, taper=0.12, step=3)
    H.line(layer, MPe(lo), low_b, wobble=0.3, pmin=0.6, pmax=0.9, step=3)

eye("eye_half", 0.42, 0.8, iris_k=1.0)
eye("eye_wide", 1.35, 1.15, iris_k=0.78, pupil_k=0.8)
H.line("eye_closed", MPe([(366, 308), (380, 314), (398, 319), (416, 316), (430, 308)]), lid_b, wobble=0.4, pmin=0.7, pmax=1.0, taper=0.1, step=3)
H.line("eye_closed", MPe([(430, 308), (438, 316)]), low_b, wobble=0.2, pmin=0.6, pmax=0.9, step=2)
H.line("eye_closed", MPe([(404, 318), (410, 326)]), low_b, wobble=0.2, pmin=0.5, pmax=0.8, step=2)
H.line("eye_happy", MPe([(366, 314), (382, 302), (402, 297), (420, 300), (432, 310)]), H.brush("ink", 3.2, INK), wobble=0.4, pmin=0.7, pmax=1.0, taper=0.1, step=3)
H.line("eye_happy", MPe([(432, 310), (440, 304)]), low_b, wobble=0.2, pmin=0.6, pmax=0.9, step=2)
H.fill("eye_happy", MPe([(372, 318), (392, 312), (414, 312), (430, 318), (414, 326), (392, 326)]), mk("marker", 5, ["#f3b79a"]), angle=0, rag=0.5, grow=-1)    # the cheek pushes up under the eye

# ------------------------------------------------------------------ brows: the traced brow, moved
brow_b = H.brush("ink", 3.0, INK)
def brow(layer, dy_fn):
    b = TH.BROW
    pts = [(x, y + dy_fn((x - 354) / 106.0)) for x, y in b["pts"]]
    c, p = TH.stroke_points(pts, b["thick"], lambda x, y: TH.M(x, y))
    c = H.resample(c, 3.5)
    import numpy as np
    pr = np.interp(np.linspace(0, 1, len(c)), np.linspace(0, 1, len(p)), p)
    H.emit(layer, brow_b, c, [float(v) for v in pr])
brow("brow_neutral", lambda s: 0)
brow("brow_up", lambda s: -15 - 3 * math.sin(math.pi * s))
brow("brow_worry", lambda s: -16 * s + 7 * (1 - s))

# ------------------------------------------------------------------ mouths (the smile already exists as mouth_smile)
mline = H.brush("ink", 3.0, "#3a1618")
def lips(layer, upper, lower, line, teeth=None, hole=None):
    if hole:
        H.fill(layer, MPh(hole[0]), mk("marker", 4, ["#d86a62"]), angle=-10, rag=0.5, grow=0)
        H.fill(layer, MPh(hole[1]), mk("marker", 3.5, ["#4a1518"]), angle=80, rag=0.3, grow=-1)
    else:
        H.fill(layer, MPh(upper), mk("marker", 4, ["#bb4a4a", "#c4524f"]), angle=-10, rag=0.5, grow=0)
        H.fill(layer, MPh(lower), mk("marker", 4, ["#d86a62", "#dd7068"]), angle=-10, rag=0.5, grow=0)
    if teeth:
        H.fill(layer, MPh(teeth), mk("marker", 3, ["#fffaf0"]), angle=-5, rag=0.3, grow=-1)
    if line:
        H.line(layer, MPh(line), mline, wobble=0.3, pmin=0.7, pmax=1.0, taper=0.08, step=2)
lips("mouth_flat", [(470, 430), (482, 441), (481, 454), (474, 461), (436, 464), (440, 455), (454, 445)],
     [(436, 465), (474, 464), (482, 471), (476, 484), (460, 489), (446, 480)], [(434, 463), (454, 465), (482, 461)])
lips("mouth_o", None, None, None, hole=([(452, 440), (480, 445), (484, 466), (476, 488), (458, 490), (446, 472)], [(458, 450), (474, 452), (477, 466), (470, 480), (460, 479), (454, 466)]))
H.line("mouth_o", MPh([(452, 440), (480, 445), (484, 466), (476, 488), (458, 490), (446, 472), (452, 440)]), H.brush("ink", 2.2, "#7a2a2a"), wobble=0.3, pmin=0.6, pmax=0.9, step=2)
lips("mouth_grin", [(470, 426), (483, 438), (482, 452), (474, 458), (426, 458), (432, 448), (450, 440)],
     [(428, 468), (474, 466), (483, 474), (476, 488), (458, 494), (440, 484)], [(420, 454), (428, 466), (452, 472), (472, 466), (484, 458)],
     teeth=[(434, 460), (476, 458), (474, 468), (440, 470)])

# ------------------------------------------------------------------ the arm: drawn horizontally (pointing +x) in an empty corner, placed by the rig
SLEEVE = mk("marker", 13, ["#b97d24", "#c18428", "#b27620", "#bd8026"])
SLEEVE_L = mk("marker", 13, ["#cf9330", "#d89a35"])
ink_a = H.brush("ink", 3.4, INK)
ink_t = H.brush("ink", 2.4, INK)
fold = H.brush("ink", 2.4, "#6a4510")
# upper arm (shoulder S_d = (120, 170), elbow E_d = (350, 170))
H.fill("arm_up", [(96, 170), (120, 132), (230, 138), (350, 146), (362, 176), (350, 204), (230, 210), (120, 208)], SLEEVE, angle=-4, rag=3)
H.line("arm_up", [(100, 156), (120, 134), (230, 139), (350, 147), (362, 176)], ink_a, wobble=1.0, pmin=0.5, pmax=1.0, step=4)
H.line("arm_up", [(362, 176), (350, 204), (230, 210), (120, 208), (100, 190)], ink_a, wobble=1.0, pmin=0.5, pmax=1.0, step=4)
H.line("arm_up", [(300, 160), (318, 176), (300, 194)], fold, wobble=0.6, pmin=0.4, pmax=0.8, step=3)
H.line("arm_up", [(170, 150), (210, 158)], fold, wobble=0.6, pmin=0.4, pmax=0.7, step=3)
# forearm, cuff, hand, match (elbow E_d = (350, 170); the match tip is at x = 704)
H.fill("arm_fore", [(336, 170), (344, 146), (450, 152), (560, 160), (568, 176), (560, 192), (450, 198), (344, 200)], SLEEVE, angle=-3, rag=3)
H.line("arm_fore", [(340, 156), (344, 146), (450, 152), (560, 160)], ink_a, wobble=1.0, pmin=0.5, pmax=1.0, step=4)
H.line("arm_fore", [(560, 192), (450, 198), (344, 200), (334, 184)], ink_a, wobble=1.0, pmin=0.5, pmax=1.0, step=4)
H.line("arm_fore", [(420, 160), (440, 176), (420, 192)], fold, wobble=0.6, pmin=0.4, pmax=0.8, step=3)
H.fill("arm_fore", [(548, 154), (588, 156), (590, 194), (548, 192)], SLEEVE_L, angle=80, rag=1.5)
H.line("arm_fore", [(548, 154), (588, 156), (590, 194), (548, 192), (548, 154)], ink_t, wobble=0.6, pmin=0.5, pmax=0.9, step=3, per=1)
# the match: a thin stick, a coloured head; the hand is painted over its back end
H.fill("arm_fore", [(600, 164), (704, 158), (704, 166), (600, 172)], mk("marker", 5, ["#d9b36b", "#cfa75e"]), angle=-3, rag=0.5, grow=0)
H.line("arm_fore", [(600, 163), (700, 157)], H.brush("ink", 1.6, "#6a4a20"), wobble=0.3, pmin=0.6, pmax=0.8, step=3)
H.line("arm_fore", [(600, 173), (700, 167)], H.brush("ink", 1.6, "#6a4a20"), wobble=0.3, pmin=0.6, pmax=0.8, step=3)
H.fill("arm_fore", [(698, 154), (712, 156), (712, 168), (698, 170)], mk("marker", 5, ["#b8452e", "#a83a26"]), angle=0, rag=0.4, grow=0)
H.line("arm_fore", [(698, 153), (712, 156), (712, 168), (698, 171)], H.brush("ink", 1.8, INK), wobble=0.3, pmin=0.6, pmax=0.9, step=2, per=1)
HAND = mk("marker", 9, ["#f0b98d", "#eeb286", "#f3c19a"])
H.fill("arm_fore", [(588, 150), (616, 142), (644, 150), (656, 172), (644, 196), (616, 204), (588, 194)], HAND, angle=-5, rag=1.2)
H.line("arm_fore", [(588, 150), (616, 142), (644, 150), (656, 172), (644, 196), (616, 204), (588, 194)], ink_a, wobble=0.8, pmin=0.5, pmax=1.0, step=3, over=(2, 2))
for pts in ([(630, 156), (642, 166)], [(634, 172), (646, 178)], [(630, 188), (640, 190)]):
    H.line("arm_fore", pts, ink_t, wobble=0.3, pmin=0.5, pmax=0.8, step=2)               # the fingers around the match
H.fill("arm_fore", [(598, 148), (626, 140), (640, 146), (622, 156), (602, 156)], mk("marker", 7, ["#f6c8a2"]), angle=0, rag=0.8, grow=-1)           # the thumb
H.line("arm_fore", [(598, 150), (626, 141), (640, 147)], ink_t, wobble=0.4, pmin=0.5, pmax=0.9, step=3)

# ------------------------------------------------------------------ light: a warm glow around the wick, and light on the face
glow_b = [H.brush("rounded", 230, "#ff7a1e", opacity=0.45), H.brush("rounded", 110, "#ffa43a", opacity=0.55), H.brush("rounded", 44, "#ffe3a0", opacity=0.6)]
for b, (x, y) in zip(glow_b, [(873, 520), (873, 510), (873, 500)]):
    for k in range(3):
        H.emit("glow", b, [(x, y), (x + 1, y + 1), (x + 2, y)], [0.95, 0.95, 0.95])
for k in range(3):
    H.emit("glow", H.brush("rounded", 140, "#ff9a3c", opacity=0.4), [(760, 655), (880, 645), (990, 655)], [0.9] * 3)      # the light on the table
# the dusk: the room before the candle is lit, a dark blue wash multiplied over everything (it lifts when the flame catches)
H.fill("dusk", [(-40, -40), (1320, -40), (1320, 760), (-40, 760)], mk("marker", 90, ["#5d7f94", "#557689", "#648699"]), angle=-6, rag=10, bow=4, pressure=0.7, step=40)
fl = H.brush("rounded", 22, "#ffb066", opacity=0.55)
for (mx, my, r) in [(500, 388, 1), (486, 330, 1), (452, 372, 1), (470, 440, 1), (440, 412, 1), (478, 300, 1)]:
    for k in range(2):
        H.emit("face_light", fl, [TH.M(mx, my), TH.M(mx + 1, my + 1), TH.M(mx + 2, my)], [0.9] * 3)
H.emit("face_light", H.brush("rounded", 40, "#ff9a3c", opacity=0.3), [TH.M(470, 360), TH.M(472, 380), TH.M(470, 400)], [0.9] * 3)

if __name__ == "__main__":
    print({k: len(v) for k, v in H.lines.items()})
