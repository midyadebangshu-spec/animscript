"""A tiny 'hand' for AnimScript: turns anchors into brush strokes with wobble, pressure and ragged fills."""
import math, random

class Hand:
    def __init__(self, seed=7):
        self.r = random.Random(seed)
        self.brushes = {}      # name -> definition line
        self.layers = {}       # layer -> z
        self.lines = {}        # layer -> list of stroke statements (in order)

    # ---- brushes -------------------------------------------------------------
    def brush(self, kind, radius, color, **kw):
        extra = "".join(f", {k}: {v}" for k, v in kw.items())
        name = f"{kind}{str(radius).replace('.','_')}_{color.lstrip('#')}" + "".join(f"_{str(v).replace('.','_')}" for v in kw.values())
        self.radius = getattr(self, "radius", {})
        self.radius[name] = (kind, radius)
        if name not in self.brushes:
            self.brushes[name] = f'let {name} = brush("{kind}", radius: {radius}, color: {color}{extra})'
        return name

    def layer(self, name, z):
        self.layers[name] = z
        self.lines.setdefault(name, [])

    # ---- geometry ------------------------------------------------------------
    @staticmethod
    def catmull(pts, closed=False, per=8):
        n = len(pts)
        out = []
        rng = range(n) if closed else range(n - 1)
        for i in rng:
            p0 = pts[(i - 1) % n] if (closed or i > 0) else pts[0]
            p1 = pts[i]
            p2 = pts[(i + 1) % n]
            p3 = pts[(i + 2) % n] if (closed or i + 2 < n) else pts[-1]
            for k in range(per):
                t = k / per
                t2, t3 = t * t, t * t * t
                x = 0.5 * ((2 * p1[0]) + (-p0[0] + p2[0]) * t + (2 * p0[0] - 5 * p1[0] + 4 * p2[0] - p3[0]) * t2 + (-p0[0] + 3 * p1[0] - 3 * p2[0] + p3[0]) * t3)
                y = 0.5 * ((2 * p1[1]) + (-p0[1] + p2[1]) * t + (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * t2 + (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * t3)
                out.append((x, y))
        if not closed:
            out.append(pts[-1])
        else:
            out.append(out[0])
        return out

    @staticmethod
    def resample(pts, step):
        out = [pts[0]]
        acc = 0.0
        for a, b in zip(pts, pts[1:]):
            d = math.hypot(b[0] - a[0], b[1] - a[1])
            if d == 0:
                continue
            pos = step - acc
            while pos <= d:
                t = pos / d
                out.append((a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t))
                pos += step
            acc = d - (pos - step)
        if out[-1] != pts[-1]:
            out.append(pts[-1])
        return out

    def noise(self, n, scale=6, amp=1.0):
        """Smooth noise of n samples (sum of sines with random phases)."""
        ph = [self.r.uniform(0, 6.28) for _ in range(3)]
        fr = [1.0, 2.3, 4.1]
        return [amp * sum(math.sin(ph[k] + fr[k] * i / scale) / (k + 1) for k in range(3)) / 1.6 for i in range(n)]

    # ---- strokes -------------------------------------------------------------
    def emit(self, layer, brush, pts, press):
        body = ", ".join("(%.1f, %.1f, %.2f)" % (x, y, p) for (x, y), p in zip(pts, press))
        self.lines[layer].append(f"art_{layer}.stroke({brush}, [{body}])")

    def line(self, layer, anchors, brush, closed=False, wobble=1.2, pmin=0.25, pmax=0.95, taper=0.18, over=(0, 0), step=5, per=8, pvary=0.25):
        """A loose ink line through the anchors: wobble, thick-thin pressure, tapered ends, optional overshoot."""
        path = self.catmull(anchors, closed, per)
        path = self.resample(path, step)
        if over[0] or over[1]:
            def ext(p, q, d):
                L = math.hypot(p[0] - q[0], p[1] - q[1]) or 1
                return (p[0] + (p[0] - q[0]) / L * d, p[1] + (p[1] - q[1]) / L * d)
            if over[0]:
                path.insert(0, ext(path[0], path[min(3, len(path) - 1)], over[0]))
            if over[1]:
                path.append(ext(path[-1], path[max(-4, -len(path))], over[1]))
        n = len(path)
        wx, wy = self.noise(n, 7, wobble), self.noise(n, 7, wobble)
        path = [(x + wx[i], y + wy[i]) for i, (x, y) in enumerate(path)]
        pn = self.noise(n, 9, pvary)
        mid = (pmin + pmax) / 2
        press = []
        for i in range(n):
            t = i / max(1, n - 1)
            ramp = min(1.0, t / taper, (1 - t) / taper) if taper > 0 else 1.0
            ramp = 0.35 + 0.65 * ramp
            p = (mid + pn[i]) * ramp
            press.append(max(0.08, min(1.0, p)))
        self.emit(layer, brush, path, press)

    def point_in(self, poly, x, y):
        inside = False
        j = len(poly) - 1
        for i in range(len(poly)):
            xi, yi = poly[i]
            xj, yj = poly[j]
            if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi + 1e-12) + xi:
                inside = not inside
            j = i
        return inside

    def fill(self, layer, anchors, brushes, spacing=9, angle=-8, rag=4, bow=1.5, pressure=0.7, closed=True, grow=3, step=6, density=1.0):
        """Paint the inside of a closed outline with parallel brush strokes: ragged ends, a slight bow, mixed brushes."""
        kinds = [self.radius[b] for b in brushes]
        if all(k == "marker" for k, _ in kinds):
            spacing = 0.5 * 0.62 * sum(r for _, r in kinds) / len(kinds) * density
        poly = self.catmull(anchors, True, 8)
        poly = self.resample(poly, 4)
        th = math.radians(angle)
        ct, st = math.cos(-th), math.sin(-th)
        rot = [(x * ct - y * st, x * st + y * ct) for x, y in poly]
        ys = [p[1] for p in rot]
        y0, y1 = min(ys), max(ys)
        y = y0 + spacing * 0.5
        back = lambda x, y: (x * math.cos(th) - y * math.sin(th), x * math.sin(th) + y * math.cos(th))
        flip = False
        while y < y1:
            xs = []
            for (ax, ay), (bx, by) in zip(rot, rot[1:] + rot[:1]):
                if (ay <= y < by) or (by <= y < ay):
                    xs.append(ax + (bx - ax) * (y - ay) / (by - ay))
            xs.sort()
            for a, b in zip(xs[0::2], xs[1::2]):
                a -= grow + self.r.uniform(-rag, rag)
                b += grow + self.r.uniform(-rag, rag)
                if b - a < 3:
                    continue
                n = max(2, int((b - a) / step))
                yy = y + self.r.uniform(-spacing * 0.15, spacing * 0.15)
                ph = self.r.uniform(0, 6.28)
                pts = []
                for k in range(n + 1):
                    t = k / n
                    pts.append(back(a + (b - a) * t, yy + bow * math.sin(ph + t * 3.2) + self.r.uniform(-0.4, 0.4)))
                if flip:
                    pts.reverse()
                flip = not flip
                p = [max(0.3, min(1.0, pressure + self.r.uniform(-0.08, 0.08))) for _ in pts]
                self.emit(layer, self.r.choice(brushes), pts, p)
            y += spacing * self.r.uniform(0.9, 1.1)

    def scribble(self, layer, center, rx, ry, brush, passes=3, rot=0.0, press=0.5):
        """A few loose curved strokes: soft shading, blush."""
        for k in range(passes):
            pts = []
            for i in range(14):
                t = i / 13
                x = (t - 0.5) * 2 * rx
                y = math.sin(t * 3.1 + k) * ry * 0.4 + (k - (passes - 1) / 2) * ry * 0.7
                pts.append((center[0] + x * math.cos(rot) - y * math.sin(rot) + self.r.uniform(-1, 1), center[1] + x * math.sin(rot) + y * math.cos(rot) + self.r.uniform(-1, 1)))
            self.emit(layer, brush, pts, [press] * len(pts))

    # ---- output ----------------------------------------------------------------
    def program(self, project, scene_name, seconds, extra_after=""):
        out = [project, ""]
        out += list(self.brushes.values())
        out.append("")
        for name, z in sorted(self.layers.items(), key=lambda kv: kv[1]):
            out.append(f"shape art_{name} = paint_layer(z: {z})")
        out += ["", f'scene "{scene_name}" for {seconds}.seconds:', "    main:"]
        for name, z in sorted(self.layers.items(), key=lambda kv: kv[1]):
            out.append(f"        # ---- {name}")
            out += ["        " + l for l in self.lines[name]]
        out.append(extra_after or f"        wait {seconds}.seconds")
        return "\n".join(out) + "\n"
