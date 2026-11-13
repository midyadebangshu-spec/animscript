"""The head, traced from the reference drawing: its ink lines become brush strokes (pressure from line thickness),
its regions are painted with marker strokes snapped to those lines."""
import json, math
import numpy as np
from paint import Hand

K = 0.6
CX, CY = 470, 290
def M(x, y):                       # mask pixels -> canvas
    return (CX + (x - 300) * K, CY + (y - 300) * K)
ES, EC = 1.3, (398, 306)           # the eye is drawn a little larger than in the reference so it reads at this size
def ME(x, y):
    return M(EC[0] + (x - EC[0]) * ES, EC[1] + (y - EC[1]) * ES)
def Rm(x, y):                      # reference-view pixels (8x crop) -> mask pixels
    return (0.75 * x - 24, 0.75 * y - 12)

import os
HERE = os.path.dirname(os.path.abspath(__file__))
paths = json.load(open(os.path.join(HERE, 'trace', 'paths.json')))
ink = np.load(os.path.join(HERE, 'trace', 'ink.npy'))
ys, xs = np.nonzero(ink)
INKPTS = np.stack([xs, ys], 1)

def snap(p, r=16):
    d = np.hypot(INKPTS[:, 0] - p[0], INKPTS[:, 1] - p[1])
    i = int(d.argmin())
    return tuple(INKPTS[i]) if d[i] <= r else p

def poly_from_ref(pts, snap_r=14):
    out = []
    for x, y in pts:
        m = Rm(x, y)
        out.append(M(*snap(m, snap_r)))
    return out

def smooth(v, n=2):
    v = np.array(v, float)
    for _ in range(n):
        v = np.convolve(np.pad(v, 2, mode='edge'), np.ones(5) / 5, mode='valid')
    return v

def region(path):
    xs = [p[0] for p in path['pts']]; ys = [p[1] for p in path['pts']]
    cx, cy = sum(xs) / len(xs), sum(ys) / len(ys)
    if 190 <= cx <= 290 and 262 <= cy <= 412: return "ear"
    if 352 <= cx <= 445 and 270 <= cy <= 340 and len(path['pts']) < 160: return "face"      # the eye
    if cx > 290 and cy > 335: return "head_ink"                                               # profile, jaw, neck
    if cx < 200 and cy > 520: return "head_ink"                                                   # back of the neck
    return "hair"

BROW = {}

def stroke_points(pts, thick, mapper, press_scale=1.0, taper=0.15):
    """A traced path as canvas points and pressures (pressure from the drawn thickness)."""
    idx = list(range(0, len(pts), 2)) + ([len(pts) - 1] if (len(pts) - 1) % 2 else [])
    px = smooth([pts[i][0] for i in idx], 2); py = smooth([pts[i][1] for i in idx], 2)
    th = smooth([thick[i] for i in idx], 3)
    canvas = [mapper(x, y) for x, y in zip(px, py)]
    n = len(canvas)
    press = []
    for i in range(n):
        t = i / max(1, n - 1)
        ramp = 0.7 + 0.3 * min(1.0, t / taper, (1 - t) / taper)
        press.append(max(0.45, min(1.0, (0.52 + 0.1 * th[i]) * ramp * press_scale)))
    return canvas, press

def build(H, mk, INK="#24151a", min_len=9):
    ink_b = {"ink": H.brush("ink", 3.0, INK)}
    eye_b = H.brush("ink", 1.9, INK)
    count = {"hair": 0, "head_ink": 0, "face": 0, "ear": 0}
    for path in list(paths):
        pts = path['pts']
        xs_ = [p[0] for p in pts]; ys_ = [p[1] for p in pts]
        if min(xs_) == 354 and max(xs_) == 496 and min(ys_) == 253:       # the forehead line and the brow are one chain: split them
            cut = next(i for i in range(100, len(pts)) if pts[i][1] <= 262 and pts[i][0] < 462)
            BROW.update(pts=pts[cut - 2:], thick=path['thick'][cut - 2:])
            path = {'pts': pts[:cut + 3], 'thick': path['thick'][:cut + 3]}
            pts = path['pts']
        if len(pts) < min_len:
            continue
        # simplify by decimation + smoothing, then map
        idx = list(range(0, len(pts), 2)) + ([len(pts) - 1] if (len(pts) - 1) % 2 else [])
        px = smooth([pts[i][0] for i in idx], 2); py = smooth([pts[i][1] for i in idx], 2)
        th = smooth([path['thick'][i] for i in idx], 3)
        canvas = [(ME if region(path) == 'face' else M)(x, y) for x, y in zip(px, py)]
        # pressure from the drawn thickness (1 to ~5 mask px), tapered at the ends
        n = len(canvas)
        press = []
        for i in range(n):
            t = i / max(1, n - 1)
            ramp = 0.7 + 0.3 * min(1.0, t / 0.15, (1 - t) / 0.15)
            press.append(max(0.45, min(1.0, (0.52 + 0.1 * th[i]) * ramp)))
        lay = region(path)
        count[lay] += 1
        if lay == "face":                       # the eye: an even, light line (the reference's thick right edge would read as black)
            press = [0.8] * len(press)
        elif len(pts) < 24:
            continue                              # tiny stray fragments of the strands
        brush = eye_b if lay == "face" else ink_b["ink"]
        lay = "eye_open" if lay == "face" else lay
        pts_c = H.resample(canvas, 3.5) if len(canvas) > 2 else canvas
        # resampling changes the count: interpolate pressure
        pr = np.interp(np.linspace(0, 1, len(pts_c)), np.linspace(0, 1, n), press)
        H.emit(lay, brush, pts_c, [float(p) for p in pr])
    return count
