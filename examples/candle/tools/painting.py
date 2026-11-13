import sys, math
from paint import Hand
H = Hand(seed=11)
OX, OY = 447, 292
T = lambda pts: [(OX + x, OY + y) for x, y in pts]

INK = "#24151a"
ink = H.brush("ink", 5, INK)
ink_thin = H.brush("ink", 3.5, INK)
ink_fine = H.brush("pen", 2.5, INK)
pencil = H.brush("pencil", 4, "#6a5a58", opacity=0.7)

# layers, back to front
for name, z in [("bg", 1), ("tail", 14), ("torso", 16), ("neck", 18), ("scarf", 20), ("head", 30), ("mouth_smile", 31), ("head_ink", 32), ("eye_open", 34), ("hair", 40), ("ear", 44), ("table", 60), ("candle", 64)]:
    H.layer(name, z)

def mk(kind, r, cols, **kw):
    return [H.brush(kind, r, c, **kw) for c in cols]

# ------------------------------------------------------------------ background: painted wall, window, shelf
wall = mk("marker", 60, ["#1f4450", "#1b3f4b", "#235060", "#1d424d"])
H.fill("bg", [(-40, -40), (1320, -40), (1320, 760), (-40, 760)], wall, angle=-5, rag=14, bow=5, pressure=0.7, step=40)
# window
win = [(960, 70), (1190, 70), (1190, 330), (960, 330)]
H.fill("bg", win, mk("marker", 30, ["#3b7388", "#44829a", "#376b80"]), angle=-10, rag=2, grow=-4, pressure=0.7)
H.fill("bg", [(1104, 110), (1152, 110), (1152, 160), (1104, 160)], mk("marker", 16, ["#f4f3d6", "#eef0c8"]), angle=20, rag=2)
frame = H.brush("ink", 7, "#12262d")
H.line("bg", [(958, 70), (1192, 70), (1192, 332), (958, 332), (958, 70)], frame, over=(6, 6), wobble=1.5, per=1)
H.line("bg", [(1075, 70), (1076, 332)], frame, over=(4, 4), wobble=1.2, per=1)
H.line("bg", [(958, 205), (1192, 204)], frame, over=(4, 4), wobble=1.2, per=1)
H.line("bg", [(944, 346), (1208, 346)], frame, over=(6, 6), wobble=1.2, per=1)
# shelf and books
H.fill("bg", [(30, 300), (330, 300), (330, 318), (30, 318)], mk("marker", 12, ["#2b2220", "#33292a"]), angle=-2, rag=2)
H.line("bg", [(28, 319), (332, 318)], ink, over=(6, 4), wobble=1.2)
books = [(44, 226, 22, "#7a3b2e"), (68, 242, 18, "#2e6b6b"), (88, 214, 26, "#c2873a"), (116, 236, 20, "#3a4f7a"),
         (138, 222, 24, "#7a3b2e"), (164, 246, 18, "#5b7d4a"), (186, 232, 22, "#a8524a"), (210, 218, 26, "#2e6b6b")]
for x, top, w, c in books:
    H.fill("bg", [(x, top), (x + w, top), (x + w, 300), (x, 300)], mk("marker", 11, [c]), angle=70, rag=1.5)
    H.line("bg", [(x, 300), (x - 1, top)], ink_fine, wobble=0.7, over=(2, 2), per=1, step=3)
    H.line("bg", [(x + w, top - 1), (x + w + 1, 300)], ink_fine, wobble=0.7, over=(2, 2), per=1, step=3)
    H.line("bg", [(x - 2, top), (x + w + 2, top - 1)], ink_fine, wobble=0.7, per=1, step=3)
H.fill("bg", [(256, 262), (292, 262), (290, 300), (258, 300)], mk("marker", 11, ["#e8d9b0"]), angle=70, rag=1.5)
H.line("bg", [(256, 262), (258, 300), (290, 300), (292, 262)], ink_fine, wobble=0.8, over=(3, 3), per=1, step=3)
H.line("bg", [(292, 270), (306, 276), (304, 290), (291, 292)], ink_thin, wobble=0.8)

# ------------------------------------------------------------------ character (right profile, faces right)
SKIN = mk("marker", 17, ["#f0b98d", "#eeb286", "#f3c19a", "#efb48a"])
SKIN_SH = H.brush("rounded", 24, "#c9784f", opacity=0.4)
SKIN_SH2 = H.brush("rounded", 20, "#d9895f", opacity=0.55)

# torso: sweater, turned three quarters; ragged, loosely painted
torso = T([(-52, 198), (-150, 218), (-250, 268), (-300, 355), (-318, 470), (-318, 520), (300, 520), (292, 440), (280, 360), (236, 280), (160, 226), (62, 200)])
H.fill("torso", torso, mk("marker", 24, ["#c98a2b", "#d29531", "#bf8024", "#cc8e2e"]), angle=-12, rag=5, pressure=0.7, step=14)
H.fill("torso", T([(-300, 355), (-318, 470), (-318, 520), (-230, 520), (-240, 420), (-262, 330)]), [H.brush("rounded", 26, "#8f5a14", opacity=0.45)], spacing=14, angle=70, rag=4)
H.line("torso", T([(-52, 198), (-150, 218), (-250, 268), (-300, 355), (-318, 470), (-320, 520)]), ink, over=(5, 0), wobble=1.6, pmin=0.45, pmax=1.0)
H.line("torso", T([(62, 200), (160, 226), (236, 280), (280, 360), (292, 440), (300, 520)]), ink, over=(5, 0), wobble=1.6, pmin=0.45, pmax=1.0)
H.line("torso", T([(150, 300), (196, 360), (210, 430)]), H.brush("ink", 5, "#6a4510"), wobble=1.2, pmin=0.3, pmax=0.8)
H.line("torso", T([(-160, 300), (-186, 380), (-176, 450)]), H.brush("ink", 5, "#6a4510"), wobble=1.2, pmin=0.3, pmax=0.8)

# scarf tail (flies back a little)
tail = T([(-80, 222), (-150, 262), (-196, 340), (-206, 420), (-160, 430), (-128, 350), (-84, 282)])
H.fill("tail", tail, mk("marker", 16, ["#b8452e", "#c04f36", "#ae3d28"]), angle=60, rag=3)
H.line("tail", T([(-80, 222), (-150, 262), (-196, 340), (-206, 420), (-160, 430), (-128, 350), (-84, 282)]), ink, wobble=1.4, over=(4, 4), closed=False, pmin=0.4, pmax=1.0)
H.line("tail", T([(-176, 332), (-130, 346)]), H.brush("marker", 12, "#f3e6c4"), wobble=1, pmin=0.6, pmax=0.8)
H.line("tail", T([(-190, 380), (-142, 394)]), H.brush("marker", 12, "#f3e6c4"), wobble=1, pmin=0.6, pmax=0.8)

# neck
neck = T([(-48, 120), (-56, 170), (-56, 214), (50, 214), (52, 170), (40, 140)])
H.fill("neck", neck, SKIN, angle=80, rag=2.5)
H.fill("neck", T([(-48, 130), (-56, 214), (0, 214), (10, 170)]), [SKIN_SH], spacing=10, angle=70, rag=3)
H.line("neck", T([(-48, 124), (-56, 170), (-56, 214)]), ink_thin, wobble=1.0, over=(0, 3), pmin=0.4, pmax=0.9)
H.line("neck", T([(40, 142), (52, 176), (50, 214)]), ink_thin, wobble=1.0, over=(0, 3), pmin=0.4, pmax=0.9)

# scarf band
scarf = T([(-96, 176), (-40, 184), (50, 190), (98, 180), (122, 204), (100, 240), (40, 258), (-30, 256), (-92, 238), (-120, 206)])
H.fill("scarf", scarf, mk("marker", 17, ["#b8452e", "#c04f36", "#ae3d28", "#b94a31"]), angle=-25, rag=3)
H.line("scarf", T([(-96, 176), (-40, 184), (50, 190), (98, 180), (122, 204), (100, 240), (40, 258), (-30, 256), (-92, 238), (-120, 206), (-96, 176)]), ink, wobble=1.4, over=(6, 6), pmin=0.4, pmax=1.0)
H.line("scarf", T([(-84, 214), (-30, 232), (40, 234), (92, 214)]), H.brush("ink", 5, "#7a2a1b"), wobble=1.2, pmin=0.3, pmax=0.8)


# ---- head: traced from the reference drawing (see trace_head.py)
import os
import trace_head as TH
SKINB = mk("marker", 15, ["#f0b98d", "#eeb286", "#f3c19a", "#efb48a"])
sk = TH.poly_from_ref([(656, 380), (642, 425), (655, 478), (692, 520), (700, 542), (682, 566), (668, 584), (676, 602), (655, 624), (670, 642), (662, 668), (656, 702), (640, 730), (598, 749),
                       (540, 746), (505, 742), (505, 880), (252, 880), (254, 740), (255, 665)], snap_r=14)
sk += TH.poly_from_ref([(205, 690), (120, 520), (130, 300), (300, 150), (560, 150), (700, 300)], snap_r=0)
pencil3 = H.brush("pencil", 3.2, "#7a6a68", opacity=0.55)
H.line("head", TH.poly_from_ref([(150, 150), (60, 360), (100, 500), (210, 690)], 0), pencil3, wobble=2, taper=0, pmin=0.4, pmax=0.6, step=6)
H.line("head", TH.poly_from_ref([(150, 130), (440, 40), (760, 190), (800, 450), (560, 780), (250, 700), (80, 450), (150, 130)], 0), pencil3, wobble=3, taper=0, pmin=0.35, pmax=0.55, step=6)
H.line("head", TH.poly_from_ref([(430, 90), (440, 780)], 0), pencil3, wobble=2, taper=0.2, pmin=0.3, pmax=0.5, step=6)
H.line("head", TH.poly_from_ref([(300, 440), (700, 410)], 0), pencil3, wobble=2, taper=0.2, pmin=0.3, pmax=0.5, step=6)
H.fill("head", sk, SKINB, angle=-8, rag=2, grow=-1)
H.fill("head", TH.poly_from_ref([(255, 700), (254, 880), (380, 880), (330, 780), (300, 720)], 0), [SKIN_SH], spacing=10, angle=70, rag=3)
H.fill("head", TH.poly_from_ref([(440, 690), (540, 746), (505, 742), (505, 800), (470, 760)], 0), [SKIN_SH], spacing=10, angle=20, rag=3)
MPh = lambda pts: [TH.M(*p) for p in pts]
mouth_b = H.brush("ink", 3.0, "#3a1618")
H.fill("mouth_smile", MPh([(470, 428), (482, 440), (481, 454), (474, 461), (428, 465), (434, 455), (450, 445)]), mk("marker", 4, ["#bb4a4a", "#c4524f"]), angle=-10, rag=0.5, grow=0)          # upper lip
H.fill("mouth_smile", MPh([(428, 466), (474, 464), (482, 472), (476, 487), (458, 493), (444, 481)]), mk("marker", 4, ["#d86a62", "#dd7068"]), angle=-10, rag=0.5, grow=0)            # lower lip
H.fill("mouth_smile", MPh([(452, 474), (468, 473), (468, 479), (452, 480)]), mk("marker", 2.5, ["#f3b3a6"]), angle=0, rag=0.2, grow=-1)                                                # a touch of light on it
H.scribble("head", TH.poly_from_ref([(590, 560)], 0)[0], 22, 10, H.brush("rounded", 16, "#ee8d7c", opacity=0.45), passes=2)
# ear, over the hair
H.fill("ear", TH.poly_from_ref([(388, 388), (342, 380), (308, 424), (304, 492), (332, 542), (378, 562), (392, 520)], 10), SKINB, angle=30, rag=1.5, grow=-1)
# the eye: white almond, big dark iris at the front, highlight
MP = lambda pts: [TH.M(*p) for p in pts]
MPe = lambda pts: [TH.ME(*p) for p in pts]
# the eye: white almond, a hazel iris with a small dark pupil and a highlight (the traced outline goes over it)
H.fill("eye_open", MPe([(374, 306), (386, 294), (404, 290), (416, 294), (418, 316), (402, 324), (384, 320)]), mk("marker", 6, ["#fffaf0"]), angle=-15, rag=0.5, grow=-1)
H.fill("eye_open", MPe([(398, 293), (414, 292), (417, 314), (404, 321), (396, 308)]), mk("marker", 5, ["#a8723c", "#9a6a3a"]), angle=80, rag=0.4, grow=-1)
H.fill("eye_open", MPe([(404, 299), (413, 298), (414, 312), (408, 316), (403, 307)]), mk("marker", 3.5, ["#2a1a14"]), angle=80, rag=0.3, grow=-1)
H.fill("eye_open", MPe([(406, 296), (411, 296), (411, 301), (406, 301)]), mk("marker", 2.5, ["#ffffff"]), angle=0, rag=0.2, grow=-1)
# hair: a lighter brown over the swept mass and the lock of the bang
hair_poly = [(205, 690), (150, 590), (95, 480), (68, 370), (80, 270), (125, 185), (170, 140), (225, 100), (300, 62), (400, 48), (500, 70), (590, 110), (680, 140), (735, 142), (790, 185), (830, 270), (852, 335),
             (820, 372), (775, 418), (715, 398), (665, 372), (610, 326), (550, 300), (495, 318), (470, 372), (442, 430), (445, 456), (405, 470), (360, 585), (300, 640), (250, 672)]
H.fill("hair", TH.poly_from_ref(hair_poly, 7), mk("marker", 9, ["#6b4332", "#74493a", "#5f3b2e", "#6d4534"]), angle=-30, rag=2, grow=1)
H.fill("hair", TH.poly_from_ref([(70, 380), (95, 480), (150, 590), (205, 690), (250, 672), (160, 520), (120, 400)], 12), [H.brush("rounded", 14, "#3a211c", opacity=0.45)], spacing=8, angle=70, rag=2)
for strand in ([(560, 120), (660, 160), (720, 170)], [(300, 110), (380, 80), (460, 90)], [(150, 300), (140, 420), (170, 540)]):
    H.line("hair", TH.poly_from_ref(strand, 0), H.brush("marker", 6, "#9a6a52"), wobble=0.8, pmin=0.45, pmax=0.8, step=4)
lid_b = H.brush("ink", 2.9, INK)
H.line("eye_open", MPe([(368, 308), (380, 294), (396, 287), (414, 284), (430, 286)]), lid_b, over=(2, 2), wobble=0.4, pmin=0.7, pmax=1.0, taper=0.12, step=3)       # the heavy upper lid
H.line("eye_open", MPe([(380, 324), (398, 328), (418, 326)]), H.brush("ink", 1.8, INK), wobble=0.3, pmin=0.6, pmax=0.9, step=3)                                     # lower lid
H.line("mouth_smile", MPh([(424, 460), (432, 465), (452, 466), (470, 463), (482, 460)]), mouth_b, wobble=0.3, pmin=0.7, pmax=1.0, taper=0.08, step=2)                                        # the mouth
print("traced strokes:", TH.build(H, mk))

# ------------------------------------------------------------------ table, matchbox, candle
H.fill("table", [(-20, 612), (1300, 612), (1300, 740), (-20, 740)], mk("marker", 34, ["#4a2f24", "#523426", "#42291f", "#4e3025"]), angle=-1, rag=8, bow=3, pressure=0.7, step=30)
H.line("table", [(-10, 616), (1290, 614)], H.brush("marker", 14, "#7b523a"), wobble=1.5, over=(0, 0), pmin=0.6, pmax=0.9, step=12)
H.line("table", [(-10, 612), (1290, 611)], ink, wobble=1.5, pmin=0.5, pmax=0.9, step=12)
for x in (200, 560, 900, 1180):
    H.line("table", [(x, 632), (x - 5, 700), (x - 2, 722)], H.brush("pen", 4, "#2c1a14"), wobble=1.2, pmin=0.3, pmax=0.7)
H.fill("table", [(720, 640), (800, 640), (804, 674), (716, 674)], mk("marker", 11, ["#8a2b2b", "#962f2f"]), angle=-5, rag=1.5)
H.line("table", [(718, 640), (800, 640), (806, 674), (714, 674), (718, 640)], ink_thin, wobble=0.9, over=(3, 3))
H.fill("table", [(724, 646), (796, 646), (798, 656), (722, 656)], mk("marker", 7, ["#3a2a22"]), angle=0, rag=1)
H.fill("candle", [(800, 650), (940, 650), (940, 672), (800, 672)], mk("marker", 14, ["#b88a3a", "#c09442"]), angle=0, rag=4)
H.line("candle", [(800, 658), (810, 670), (870, 676), (934, 668), (942, 656)], ink_thin, wobble=1.2, pmin=0.45, pmax=0.95)
H.fill("candle", [(846, 520), (896, 520), (898, 654), (844, 654)], mk("marker", 13, ["#efe0b8", "#f3e6c4", "#e9d9ae"]), angle=82, rag=2.5)
H.fill("candle", [(846, 524), (844, 654), (858, 654), (860, 534)], [H.brush("rounded", 12, "#b5a070", opacity=0.5)], spacing=7, angle=85, rag=2)
H.fill("candle", [(846, 520), (896, 520), (894, 528), (848, 528)], mk("marker", 9, ["#fbf1d4"]), angle=0, rag=1.5, grow=-1)                      # the melted top
H.line("candle", [(848, 524), (846, 650)], ink_thin, wobble=0.9, over=(3, 4), pmin=0.4, pmax=0.9)
H.line("candle", [(895, 524), (898, 650)], ink_thin, wobble=0.9, over=(3, 4), pmin=0.4, pmax=0.9)
H.line("candle", [(846, 520), (860, 528), (884, 528), (896, 520)], ink_fine, wobble=0.5, pmin=0.35, pmax=0.7)                                     # only the front rim of the top
H.fill("candle", [(884, 524), (894, 524), (894, 568), (889, 578), (884, 568)], mk("marker", 7, ["#fbf1d4"]), angle=85, rag=1, grow=0)             # a drip
H.line("candle", [(871, 522), (873, 496)], H.brush("ink", 3.0, "#2a1a1a"), wobble=0.4, pmin=0.6, pmax=0.9, step=3)                                # the wick
if __name__ == "__main__":
  src = H.program('project "The Candle":\n    anim_version: "0.1"\n    canvas: 1280x720\n    fps: 24\n    background: #12262d', "Still", 1)
  open(sys.argv[1], "w").write(src)
  print(sum(len(v) for v in H.lines.values()), "strokes,", len(src) // 1024, "KB")
