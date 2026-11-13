import sys, math
OUT = sys.argv[1] if len(sys.argv) > 1 else 'candle.anim'
import parts
from parts import H
import rig
from rig import *

HEAD_GROUP = ["head", "mouth_smile", "mouth_flat", "mouth_o", "mouth_grin", "head_ink", "brow_neutral", "brow_up", "brow_worry",
              "eye_open", "eye_half", "eye_wide", "eye_closed", "eye_happy", "face_light", "hair", "ear"]
TORSO_GROUP = ["torso", "neck", "scarf"]
TAIL_P = (367.0, 514.0)

tx, ty = torso_series()
head_rot, hdx, hdy = head_series(tx)
hair_extra = hair_series(head_rot)
tail_rot = tail_series(tx)
arm = arm_series(tx, ty)
ops = opacity_series()

# ---------------------------------------------------------------- vector shapes: the flames, the smoke (equations!)
BX, BY = 873.0, 497.0            # the top of the wick
def V(): return "select(u < 1, u, 2 - u)"
def flame_curve(name, bx, by, W, Hh, n=64):
    v = V()
    x = f"{bx} + select(u < 1, 1, -1) * {W} * pow(sin(pi * pow({v}, 0.72)), 1.15) * (1 - 0.3 * {v})"
    y = f"{by} - {Hh} * {v}"
    return f'shape {name} = curve(x: "{x}", y: "{y}", from: 0, to: 2, n: {n}, closed: true)'

def flame_drive(bx, by, Hh, A, B, ph, s1, s2, env):
    h = f"clamp(({by} - y) / {Hh}, 0, 1)"
    E = f"smoothstep(0, {env}, t)"
    x = (f"{bx} + (x - {bx}) * (0.35 + 0.65 * {E}) * (1 + 0.05 * noise(t * 2.7, {s1})) "
         f"+ {E} * ({A} * pow({h}, 2) * sin(tau * (1.15 * t + 0.7 * {h} + {ph})) + {B} * {h} * noise({h} * 1.8 + 5, t * 2.3 + {ph}))")
    y = f"{by} + (y - {by}) * {E} * (1 + 0.07 * noise(t * 3.1, {s2}) + 0.035 * sin(tau * t * 5.3 + {ph}))"
    return f'start {{n}}.drive(x: "{x}", y: "{y}", mode: "replace")'

shapes = []
def vec(name, defn, fill=None, stroke=None, width=0, z=68, opacity=0):
    shapes.append(defn)
    if fill: shapes.append(f"{name}.fill = {fill}")
    if stroke: shapes.append(f"{name}.stroke = {stroke}")
    shapes.append(f"{name}.stroke_width = {width}")
    shapes.append(f"{name}.opacity = {opacity}")
    shapes.append(f"{name}.z = {z}")

vec("cflame_out", flame_curve("cflame_out", BX, BY, 27, 100), "#ff8a2a")
vec("cflame", flame_curve("cflame", BX, BY, 20, 86), "#ffc43a", "#ff9a2e", 2)
vec("cflame_core", flame_curve("cflame_core", BX, BY + 1, 10, 48), "#fff2b8")
shapes.append('shape cflame_base = curve(x: "%s + 6 * cos(u)", y: "%s + 3 + 8 * sin(u)", from: 0, to: 6.283185307179586, n: 16, closed: true)' % (BX, BY))
shapes += ["cflame_base.fill = #6fa6ff", "cflame_base.stroke_width = 0", "cflame_base.opacity = 0", "cflame_base.z = 69"]
vec("mflame_out", flame_curve("mflame_out", 0, 0, 17, 68, 48), "#ff8a2a", z=68)
vec("mflame", flame_curve("mflame", 0, 0, 13, 58, 48), "#ffc43a", "#ff9a2e", 1.5, z=68)
vec("mflame_core", flame_curve("mflame_core", 0, 1, 6, 32, 48), "#fff2b8", z=69)
SX, SY = 846.0 + 6, 534.0 - 22      # the match head when it is shaken out; the smoke rises from there
shapes.append('shape smoke = curve(x: "%s", y: "%s - 130 * u", from: 0, to: 1, n: 48)' % (SX, SY))
shapes += ["smoke.stroke = #cfd6d9", "smoke.stroke_width = 6", "smoke.opacity = 0", "smoke.z = 69"]
shapes.append("shape cover = polygon([(-10, -10), (1290, -10), (1290, 730), (-10, 730)])")
shapes += ["cover.fill = #000000", "cover.stroke_width = 0", "cover.opacity = 1", "cover.z = 100"]

# ---------------------------------------------------------------- timeline keys
keys = {}      # frame -> list of "target = value"
def put(f, target, value):
    keys.setdefault(f, []).append(f"{target} = {value}")

def series(name, prop, vals, tol, fmt):
    for i in rdp_series(vals, tol):
        put(i, f"{name}.{prop}", fmt(vals[i]))

deg = lambda v: "%.3f.deg" % v
px = lambda v: "%.2f" % v
op = lambda v: "%.3f" % v

for n in TORSO_GROUP:
    series(f"art_{n}", "position.x", tx, 0.2, px); series(f"art_{n}", "position.y", ty, 0.2, px)
series("art_tail", "position.x", tx, 0.2, px); series("art_tail", "position.y", ty, 0.2, px); series("art_tail", "rotation", tail_rot, 0.1, deg)
hx = [tx[f] + hdx[f] for f in FRAMES]; hy = [ty[f] + hdy[f] for f in FRAMES]
for n in HEAD_GROUP:
    series(f"art_{n}", "position.x", hx, 0.2, px); series(f"art_{n}", "position.y", hy, 0.2, px)
    series(f"art_{n}", "rotation", [head_rot[f] + (hair_extra[f] if n == "hair" else 0.0) for f in FRAMES], 0.05, deg)
for n, w in ops.items():
    series(f"art_{n}", "opacity", w, 0.02, op)
# the arm
S_up = [(a["S"][0] - SD[0], a["S"][1] - SD[1]) for a in arm]
series("art_arm_up", "position.x", [p[0] for p in S_up], 0.3, px); series("art_arm_up", "position.y", [p[1] for p in S_up], 0.3, px)
series("art_arm_up", "rotation", [a["a"] for a in arm], 0.15, deg)
series("art_arm_fore", "position.x", [a["E"][0] - ED[0] for a in arm], 0.3, px); series("art_arm_fore", "position.y", [a["E"][1] - ED[1] for a in arm], 0.3, px)
series("art_arm_fore", "rotation", [a["b"] for a in arm], 0.15, deg)
# the match flame sits on the match head and follows the tip
tipx = [a["tip"][0] for a in arm]; tipy = [a["tip"][1] - 4 for a in arm]
for n in ("mflame_out", "mflame", "mflame_core"):
    series(n, "position.x", tipx, 0.3, px); series(n, "position.y", tipy, 0.3, px)
f_m, f_o, f_c = round(T_MATCH_LIT * FPS), round(T_MATCH_OUT * FPS), round(T_CANDLE_LIT * FPS)
def on_off(name, on, off, full, ramp=2):
    """0 before `on`, full from on+ramp, 0 from `off` (None: stays)."""
    vals = [0.0] * (N + 1)
    for f in FRAMES:
        v = clamp((f - on) / ramp) * full
        if off is not None and f >= off: v = max(0.0, full * (1 - (f - off) / 3.0))
        vals[f] = v
    series(name, "opacity", vals, 0.02, op)
on_off("mflame_out", f_m, f_o, 0.5); on_off("mflame", f_m, f_o, 1.0, 1); on_off("mflame_core", f_m, f_o, 1.0, 1)
on_off("cflame_out", f_c, None, 0.5, 3); on_off("cflame", f_c, None, 1.0, 2); on_off("cflame_core", f_c, None, 1.0, 3); on_off("cflame_base", f_c, None, 0.8, 3)
f_s = round(T_SMOKE * FPS)
smoke = [0.0] * (N + 1)
for f in FRAMES:
    t = tt(f) - T_SMOKE
    smoke[f] = 0.0 if t < 0 else 0.75 * clamp(t / 0.5) * (1 - clamp((t - 1.8) / 1.6))
series("smoke", "opacity", smoke, 0.02, op)
cover = [1.0 - clamp(tt(f) / T_FADE_IN) if tt(f) < T_FADE_OUT[0] else clamp((tt(f) - T_FADE_OUT[0]) / (T_FADE_OUT[1] - T_FADE_OUT[0])) for f in FRAMES]
series("cover", "opacity", cover, 0.01, op)
dusk = [0.62 - 0.40 * ease_in_out(clamp((tt(f) - T_CANDLE_LIT) / 2.2)) - 0.08 * ease_in_out(clamp((tt(f) - T_MATCH_LIT) / 0.6)) * (1 if tt(f) < T_CANDLE_LIT else 0) for f in FRAMES]
series("art_dusk", "opacity", dusk, 0.01, op)
# camera: a slow push-in on the face and the candle
cam_x = [640 + (710 - 640) * ease_in_out(clamp((tt(f) - T_ZOOM[0]) / (T_ZOOM[1] - T_ZOOM[0]))) for f in FRAMES]
cam_y = [360 + (400 - 360) * ease_in_out(clamp((tt(f) - T_ZOOM[0]) / (T_ZOOM[1] - T_ZOOM[0]))) for f in FRAMES]
cam_s = [1.0 + 0.17 * ease_in_out(clamp((tt(f) - T_ZOOM[0]) / (T_ZOOM[1] - T_ZOOM[0]))) for f in FRAMES]
series("camera", "position.x", cam_x, 0.2, px); series("camera", "position.y", cam_y, 0.2, px); series("camera", "scale", cam_s, 0.002, lambda v: "%.4f" % v)

# ---------------------------------------------------------------- the program
glow_eq = "smoothstep(0, 1.8, t) * (0.62 + 0.07 * sin(tau * t * 3.7 + 1) + 0.05 * sin(tau * t * 7.3) + 0.03 * sin(tau * t * 13.1 + 2))"
face_eq = "0.8 * " + glow_eq
events = [
    (round(T_ARM_UP * FPS), ["art_arm_fore.z = 67"]),
    (f_m, [flame_drive(0, 0, 58, 6, 5, 0.0, 1.3, 4.4, 0.35).replace("{n}", "mflame"),
           flame_drive(0, 0, 68, 7, 6, 0.3, 2.1, 5.2, 0.4).replace("{n}", "mflame_out"),
           flame_drive(0, 1, 32, 3, 2.5, 0.6, 3.3, 6.1, 0.3).replace("{n}", "mflame_core")]),
    (f_c, [flame_drive(BX, BY, 86, 7, 6, 0.0, 1.3, 4.4, 1.6).replace("{n}", "cflame"),
           flame_drive(BX, BY, 100, 8, 7, 0.25, 2.1, 5.2, 1.7).replace("{n}", "cflame_out"),
           flame_drive(BX, BY + 1, 48, 3, 2.5, 0.5, 3.3, 6.1, 1.5).replace("{n}", "cflame_core"),
           f'start glow.drive(opacity: "{glow_eq}")', f'start face_light_fx.drive(opacity: "{face_eq}")']),
    (f_s, [f'start smoke.drive(x: "(5 + 18 * clamp(({SY} - y) / 130, 0, 1)) * sin(tau * (1.05 * clamp(({SY} - y) / 130, 0, 1) - 0.7 * t)) + {SX} + 7 * clamp(({SY} - y) / 130, 0, 1) * noise(clamp(({SY} - y) / 130, 0, 1) * 2.4, t * 1.3)", '
                 f'y: "{SY} + (y - {SY}) * (0.25 + 0.75 * smoothstep(0, 1.0, t))", mode: "replace")']),
    (round(T_ARM_HIDE * FPS), ["art_arm_fore.z = 23"]),
]
events.sort(key=lambda e: e[0])

out = ['project "The Candle":', '    anim_version: "0.1"', '    canvas: 1280x720', '    fps: 24', '    background: #12262d', '']
out += list(H.brushes.values()) + ['']
extra_layers = {"face_light_fx": 36}
for name, z in sorted(H.layers.items(), key=lambda kv: kv[1]):
    out.append(f"shape art_{name} = paint_layer(z: {z})")
out.append("")
# pivots and static settings
for n in HEAD_GROUP: out.append(f"art_{n}.pivot = ({P0[0]}, {P0[1]})")
out.append(f"art_tail.pivot = ({TAIL_P[0]}, {TAIL_P[1]})")
out.append(f"art_arm_up.pivot = ({SD[0]}, {SD[1]})")
out.append(f"art_arm_fore.pivot = ({ED[0]}, {ED[1]})")
out.append(f"art_arm_up.scale = {ARM_S}")
out.append(f"art_arm_fore.scale = {ARM_S}")
out += ["art_dusk.blend = \"multiply\"", "art_glow.blend = \"add\"", "art_face_light.blend = \"screen\"", "art_glow.opacity = 0", "art_face_light.opacity = 0", ""]
for n in EYES[1:] + BROWS[1:] + MOUTHS[1:] + ["mouth_smile"]:
    pass
out += shapes + [""]
out += ["", f'scene "The Candle" for {int(DUR)}.seconds:', "    timeline:"]
for f in sorted(keys):
    out.append(f"        at {f}f:")
    out += [f"            {k}" for k in keys[f]]
out.append("    main:")
for name, z in sorted(H.layers.items(), key=lambda kv: kv[1]):
    out.append(f"        # ---- {name}")
    out += ["        " + l for l in H.lines[name]]
prev = 0
for f, lines in events:
    if f > prev:
        out.append(f"        wait {f - prev}.frames")
    out += ["        " + l for l in lines]
    prev = f
out.append(f"        wait {N - prev}.frames")
text = "\n".join(out) + "\n"
text = text.replace("face_light_fx.drive", "art_face_light.drive").replace("start glow.drive", "start art_glow.drive")
open(OUT, "w").write(text)
print(len(text) // 1024, "KB;", sum(len(v) for v in keys.values()), "keys at", len(keys), "frames")
