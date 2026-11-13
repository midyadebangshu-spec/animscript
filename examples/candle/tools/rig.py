"""The puppet: per-frame poses of every layer (torso, head, hair, scarf tail, a two-bone arm with IK), the expression
schedule, and the keys that go into the scene's timeline."""
import math

FPS = 24
DUR = 16.0
N = int(DUR * FPS)           # frames 0 .. N

# ---------------------------------------------------------------- easing and interpolation
def ease_in_out(p): return 2 * p * p if p < 0.5 else 1 - 2 * (1 - p) ** 2
def ease_out(p): return 1 - (1 - p) ** 2
def ease_in(p): return p * p
def ease_out_back(p, k=1.3): return 1 + (k + 1) * (p - 1) ** 3 + k * (p - 1) ** 2
def clamp(x, a=0.0, b=1.0): return max(a, min(b, x))

def keyed(keys, t, ease=ease_in_out):
    """keys: [(time, value)] or [(time, value, easing)]; the easing of a key shapes the segment that STARTS at it."""
    if t <= keys[0][0]: return keys[0][1]
    for k0, k1 in zip(keys, keys[1:]):
        if t <= k1[0]:
            e = k0[2] if len(k0) > 2 else ease
            p = (t - k0[0]) / (k1[0] - k0[0])
            return k0[1] + (k1[1] - k0[1]) * e(p)
    return keys[-1][1]

def spring(target, k=90.0, c=9.0, sub=8):
    """A damped spring following `target` (one value per frame): gives overshoot and follow-through."""
    out = []
    x, v = target[0], 0.0
    dt = 1.0 / (FPS * sub)
    for tg in target:
        for _ in range(sub):
            a = -k * (x - tg) - c * v
            v += a * dt
            x += v * dt
        out.append(x)
    return out

# ---------------------------------------------------------------- geometry
S0 = (440.0, 590.0)         # the shoulder in the final standing pose
P0 = (450.0, 468.0)         # the neck: the head turns about it
ARM_S = 0.9                 # the arm art is scaled by this
L1 = 230 * ARM_S
L2 = 354.1 * ARM_S
DELTA = math.degrees(math.atan2(-10, 354))     # the match sits a little above the forearm's axis
SD, ED = (120.0, 170.0), (350.0, 170.0)         # shoulder and elbow in the arm layers

def ik(S, T):
    dx, dy = T[0] - S[0], T[1] - S[1]
    d = clamp(math.hypot(dx, dy), abs(L1 - L2) + 1, L1 + L2 - 1, ) if False else max(abs(L1 - L2) + 1, min(L1 + L2 - 1, math.hypot(dx, dy)))
    phi = math.atan2(dy, dx)
    alpha = math.acos((L1 * L1 + d * d - L2 * L2) / (2 * L1 * d))
    a = phi + alpha                                    # the elbow below the line shoulder-tip
    E = (S[0] + L1 * math.cos(a), S[1] + L1 * math.sin(a))
    b_axis = math.atan2(T[1] - E[1], T[0] - E[0]) if d < L1 + L2 - 1.5 else a
    return math.degrees(a), math.degrees(b_axis), E

def tip_of(S, a_deg, b_deg):
    a = math.radians(a_deg)
    E = (S[0] + L1 * math.cos(a), S[1] + L1 * math.sin(a))
    b = math.radians(b_deg)
    return (E[0] + L2 * math.cos(b), E[1] + L2 * math.sin(b)), E

# ---------------------------------------------------------------- the schedule (seconds)
T_FADE_IN = 0.8
T_ENTER = (0.6, 3.4)
T_ARM_UP = 4.1             # the arm swings in over the table
T_STRIKE = 5.0             # the match is struck here
T_MATCH_LIT = 5.2
T_TOUCH = 8.9              # the match touches the wick
T_CANDLE_LIT = 9.1
T_MATCH_OUT = 10.3
T_ARM_DOWN = 12.0
T_ARM_HIDE = 12.4
T_SMOKE = 10.35
T_FADE_OUT = (15.1, 16.0)
T_ZOOM = (12.3, 15.6)

FRAMES = range(N + 1)
def tt(f): return f / FPS

# ---- torso
def torso_series():
    xs, ys = [], []
    for f in FRAMES:
        t = tt(f)
        p = clamp((t - T_ENTER[0]) / (T_ENTER[1] - T_ENTER[0]))
        x = -620 * (1 - ease_in_out(p)) + (16 * math.exp(-(t - T_ENTER[1]) * 4.5) * math.sin((t - T_ENTER[1]) * 13) if t > T_ENTER[1] else 0.0)
        bob = -5 * abs(math.sin(math.pi * (t - T_ENTER[0]) * 1.7)) * (1 - p) ** 0.5 if 0 < p < 1 else 0.0
        breathe = 1.6 * math.sin(math.tau * t / 3.6) * (1 if t > 3.0 else 0)
        lean = keyed([(0, 0), (6.2, 0), (8.6, 46, ease_in_out), (9.1, 40, ease_out), (10.6, 38), (12.6, 0, ease_in_out), (13.0, 0)], t)
        recoil = -10 * math.sin(math.pi * clamp((t - 9.05) / 0.7)) if 9.05 < t < 9.75 else 0.0
        xs.append(x + lean + recoil)
        ys.append(bob + breathe + keyed([(0, 0), (8.6, 4), (9.1, 0)], t))
    return xs, ys

# ---- head (rotation in degrees, clockwise = leaning forward; offsets in px, relative to the torso)
def head_series(tx):
    rot_t, dx_t, dy_t = [], [], []
    for f in FRAMES:
        t = tt(f)
        rot_t.append(keyed([(0, 0), (2.7, 0), (3.1, 3.5, ease_out), (3.7, -3.5), (4.5, 0), (4.9, 2.0), (5.3, 3.0), (5.9, -2.5, ease_out), (6.5, 0),
                            (8.6, 5.5), (9.0, 4.5), (9.12, -6.0, ease_out), (9.7, -3.0), (10.8, -1.0), (12.0, 1.0), (12.6, 3.5), (14.0, 3.0), (16, 3.0)], t))
        dx_t.append(keyed([(0, 0), (6.2, 0), (8.6, 14), (9.0, 14), (9.12, -6, ease_out), (9.8, -2), (11, 0), (12.6, 8), (16, 8)], t))
        dy_t.append(keyed([(0, 0), (8.6, 4), (9.1, 0), (12.6, 2), (16, 2)], t))
    # follow-through: the head lags the torso's sliding; the spring overshoots the targets a little
    ax = [0.0] + [tx[i + 1] - 2 * tx[i] + tx[i - 1] if 0 < i < N else 0.0 for i in range(N)]
    ax = ax[:N + 1]
    kick = [-0.05 * a for a in ax]
    rot = spring([r + k for r, k in zip(rot_t, kick)], k=120, c=12)
    return rot, dx_t, dy_t

def hair_series(head_rot):
    lag = spring(head_rot, k=34, c=3.2)
    return [h - r for h, r in zip(lag, head_rot)]          # an extra swing of the hair relative to the head

def tail_series(tx):
    v = [0.0] + [tx[i] - tx[i - 1] for i in range(1, N + 1)]
    target = [-0.35 * x for x in v]
    return spring(target, k=40, c=3.5)

# ---- arm: tip waypoints (time, x, y), eased; "rest" is straight down
REST_TIP = (S0[0], S0[1] + L1 + L2)
def tip_target(t, S):
    kp = [
        (0.0, REST_TIP, None),
        (T_ARM_UP - 0.05, REST_TIP, None),
        (T_ARM_UP + 0.9, (700.0, 604.0), ease_out),            # swings up over the table, above the matchbox
        (T_STRIKE - 0.35, (698.0, 596.0), ease_in_out),         # a breath of anticipation: back and a bit up
        (T_STRIKE - 0.12, (684.0, 588.0), ease_in),
        (T_STRIKE + 0.12, (800.0, 650.0), ease_in),             # the strike along the strip
        (T_STRIKE + 0.32, (812.0, 640.0), ease_out),            # follow-through
        (T_MATCH_LIT + 0.9, (790.0, 600.0), ease_in_out),       # holds the lit match up
        (T_TOUCH - 1.3, (822.0, 566.0), ease_in_out),
        (T_TOUCH, (866.0, 512.0), ease_in_out),                  # to the wick
        (T_CANDLE_LIT + 0.7, (868.0, 512.0), ease_out),
        (T_MATCH_OUT - 0.1, (846.0, 534.0), ease_in_out),        # draws back a little
        (T_MATCH_OUT + 0.55, (846.0, 534.0), ease_in_out),
        (T_ARM_DOWN, REST_TIP, ease_in_out),
        (16.0, REST_TIP, None),
    ]
    for (t0, p0, e0), (t1, p1, _) in zip(kp, kp[1:]):
        if t <= t1:
            if t <= t0: return p0
            e = e0 or ease_in_out
            q = e((t - t0) / (t1 - t0))
            return (p0[0] + (p1[0] - p0[0]) * q, p0[1] + (p1[1] - p0[1]) * q)
    return kp[-1][1]

def shake_off(t):
    """Shaking the match out: a quick flick of the tip."""
    if T_MATCH_OUT - 0.15 < t < T_MATCH_OUT + 0.55:
        p = (t - (T_MATCH_OUT - 0.15)) / 0.7
        return 16 * math.sin(math.tau * 3.2 * p) * (1 - p), -10 * math.sin(math.tau * 3.2 * p) * (1 - p)
    return 0.0, 0.0

def arm_series(tx, ty):
    out = []
    for f in FRAMES:
        t = tt(f)
        S = (S0[0] + tx[f], S0[1] + ty[f])
        T = tip_target(t, S)
        sx, sy = shake_off(t)
        T = (T[0] + sx, T[1] + sy)
        # the rest tip follows the shoulder so the arm just hangs
        if T == REST_TIP or T[1] > 1000: T = (S[0], S[1] + L1 + L2)
        a, b_axis, E = ik(S, T)
        b = b_axis - DELTA
        tip, _ = tip_of(S, a, b_axis)
        out.append(dict(S=S, a=a, b=b, E=E, tip=tip))
    return out

# ---- expressions: (start time, eye, brow, mouth)
EXPR = [
    (0.0, "eye_open", "brow_neutral", "mouth_flat"),
    (3.6, "eye_open", "brow_up", "mouth_flat"),          # curious
    (4.7, "eye_open", "brow_neutral", "mouth_flat"),
    (T_MATCH_LIT - 0.05, "eye_wide", "brow_up", "mouth_flat"),   # the match flares
    (T_MATCH_LIT + 0.7, "eye_open", "brow_neutral", "mouth_flat"),
    (6.6, "eye_half", "brow_worry", "mouth_flat"),       # concentrating, carrying the flame
    (T_CANDLE_LIT - 0.05, "eye_wide", "brow_up", "mouth_o"),     # surprise: it caught
    (10.6, "eye_open", "brow_up", "mouth_smile"),        # wonder
    (12.4, "eye_happy", "brow_neutral", "mouth_grin"),   # a soft, happy smile
]
BLINKS = [2.9, 7.6, 11.3]
EYES = ["eye_open", "eye_half", "eye_wide", "eye_closed", "eye_happy"]
BROWS = ["brow_neutral", "brow_up", "brow_worry"]
MOUTHS = ["mouth_smile", "mouth_flat", "mouth_o", "mouth_grin"]

def expr_state(f):
    t = tt(f)
    cur = EXPR[0]
    for e in EXPR:
        if t >= e[0]: cur = e
    return cur

def opacity_series():
    """weights per layer per frame, with 2-frame crossfades between states."""
    weights = {n: [0.0] * (N + 1) for n in EYES + BROWS + MOUTHS}
    prev_state = None
    for f in FRAMES:
        eye, brow, mouth = expr_state(f)[1:]
        for b in BLINKS:
            if b * FPS <= f < b * FPS + 3 and eye in ("eye_open", "eye_wide", "eye_half"):
                eye = "eye_closed"
        cur = {"eye": eye, "brow": brow, "mouth": mouth}
        for kind, names in (("eye", EYES), ("brow", BROWS), ("mouth", MOUTHS)):
            for n in names:
                weights[n][f] = 1.0 if cur[kind] == n else 0.0
    # crossfade: blend each series with a 3-tap window so the swap takes ~2 frames
    for n in weights:
        w = weights[n]
        weights[n] = [max(0.0, min(1.0, (w[max(0, i - 1)] + w[i] * 2 + w[min(N, i + 1)]) / 4 * 1.0)) for i in range(N + 1)]
        # sharpen: keep a full 1.0 where the neighbours agree
        weights[n] = [1.0 if w[max(0, i - 1)] == w[i] == w[min(N, i + 1)] == 1.0 else weights[n][i] for i in range(N + 1)]
    return weights

# ---------------------------------------------------------------- compressing a series into keys
def rdp_series(vals, tol):
    """indices of the points to keep so that linear interpolation stays within `tol`."""
    keep = {0, len(vals) - 1}
    def rec(i, j):
        if j <= i + 1: return
        worst, wi = 0.0, None
        for k in range(i + 1, j):
            v = vals[i] + (vals[j] - vals[i]) * (k - i) / (j - i)
            d = abs(vals[k] - v)
            if d > worst: worst, wi = d, k
        if wi is not None and worst > tol:
            keep.add(wi); rec(i, wi); rec(wi, j)
    rec(0, len(vals) - 1)
    return sorted(keep)
