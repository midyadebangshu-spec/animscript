import numpy as np, json, warnings, math
warnings.filterwarnings('ignore')
from PIL import Image, ImageFilter
from scipy import ndimage as ndi
from skimage.morphology import skeletonize, remove_small_objects, disk, dilation
from skimage.measure import label

import sys, os
HERE = os.path.dirname(os.path.abspath(__file__))
im = Image.open(sys.argv[1]).convert('L')
S = 6
crop = im.crop((44, 2, 152, 110))
big = crop.resize((crop.width * S, crop.height * S), Image.BICUBIC)
a = np.asarray(big).astype(float)
paper = np.maximum(np.asarray(big.filter(ImageFilter.GaussianBlur(40))).astype(float), np.percentile(a, 80))
d = paper - a
ink = remove_small_objects(d > 38, 40)
ink = ndi.binary_closing(ink, structure=np.ones((3, 3)))
thick = ndi.distance_transform_edt(ink)
skel = skeletonize(ink)

def trace(skel):
    pts = set(zip(*np.nonzero(skel)))
    def nbrs(p):
        y, x = p
        return [(y + dy, x + dx) for dy in (-1, 0, 1) for dx in (-1, 0, 1) if (dy or dx) and (y + dy, x + dx) in pts]
    deg = {p: len(nbrs(p)) for p in pts}
    nodes = {p for p, k in deg.items() if k != 2}
    seen_edges = set(); paths = []
    def walk(s, n):
        path = [s, n]; prev, cur = s, n
        while cur not in nodes:
            nx = [q for q in nbrs(cur) if q != prev and (cur, q) not in seen_edges]
            if not nx: break
            prev, cur = cur, nx[0]; path.append(cur)
        return path
    for s in sorted(nodes):
        for n in nbrs(s):
            if (s, n) in seen_edges: continue
            p = walk(s, n)
            for u, v in zip(p, p[1:]): seen_edges.add((u, v)); seen_edges.add((v, u))
            paths.append(p)
    covered = {q for p in paths for q in p}
    for s in sorted(pts - covered):
        if s in covered or not nbrs(s): continue
        p = walk(s, nbrs(s)[0]); covered.update(p); paths.append(p)
    return paths, deg

# 1. prune spurs (short branches that end freely), twice
for _ in range(3):
    paths, deg = trace(skel)
    for p in paths:
        free_a, free_b = deg[p[0]] == 1, deg[p[-1]] == 1
        if (free_a != free_b) and len(p) < 14:
            for (y, x) in (p[:-1] if free_a else p[1:]):
                skel[y, x] = False
paths, deg = trace(skel)

# 2. merge at junctions by direction continuity
def direction(p, end, k=8):
    q = p[:k + 1] if end == 0 else p[-k - 1:][::-1]
    (y0, x0), (y1, x1) = q[0], q[-1]
    v = np.array([x1 - x0, y1 - y0], float); n = np.hypot(*v) or 1
    return v / n
junction = {pt for pt, k in deg.items() if k >= 3}
labels = label(np.isin(np.indices(skel.shape)[0] * 0, [1]) | np.zeros(skel.shape, bool))  # placeholder
jmask = np.zeros(skel.shape, bool)
for (y, x) in junction: jmask[y, x] = True
jlab = label(dilation(jmask, disk(2)))
def jid(pt):
    return jlab[pt] if jlab[pt] else 0
ends = {}
for i, p in enumerate(paths):
    for e, pt in ((0, p[0]), (1, p[-1])):
        j = jid(pt)
        if j: ends.setdefault(j, []).append((i, e))
parent = list(range(len(paths)))
links = {}   # (path, end) -> (path, end)
for j, lst in ends.items():
    cand = []
    for a1 in range(len(lst)):
        for b1 in range(a1 + 1, len(lst)):
            (i1, e1), (i2, e2) = lst[a1], lst[b1]
            if i1 == i2: continue
            cos = float(np.dot(direction(paths[i1], e1), direction(paths[i2], e2)))
            cand.append((cos, lst[a1], lst[b1]))
    cand.sort()
    used = set()
    for cos, A, B in cand:
        if cos > -0.5: break
        if A in used or B in used: continue
        used.add(A); used.add(B); links[A] = B; links[B] = A
chains = []; done = set()
def follow(i, start_end):
    chain = []
    cur, e = i, start_end   # we enter path cur at its end `e` and leave from the other end
    while True:
        p = paths[cur] if e == 0 else paths[cur][::-1]
        chain += p
        done.add(cur)
        out_end = 1 - e
        nxt = links.get((cur, out_end))
        if not nxt or nxt[0] in done: break
        cur, e = nxt
    return chain
for i in range(len(paths)):
    if i in done: continue
    # walk back to the chain start
    cur, e = i, 0
    seen = {i}
    while True:
        prev = links.get((cur, e))
        if not prev or prev[0] in seen: break
        seen.add(prev[0]); cur, e = prev[0], 1 - prev[1]
    chains.append(follow(cur, e))
chains = [c for c in chains if len(c) >= 16]
print(len(paths), "paths ->", len(chains), "strokes,", sum(len(c) for c in chains), "points")
out = [{"pts": [(int(x), int(y)) for y, x in c], "thick": [float(thick[y, x]) for y, x in c]} for c in chains]
json.dump(out, open(os.path.join(HERE, 'paths.json'), 'w')); np.save(os.path.join(HERE, 'ink.npy'), ink)

vis = Image.new('RGB', skel.shape[::-1], 'white'); px = vis.load(); rng = np.random.RandomState(3)
for c in chains:
    col = tuple(int(v) for v in rng.randint(0, 190, 3))
    for y, x in c: px[x, y] = col
vis.save(os.path.join(HERE, 'paths.png'))
