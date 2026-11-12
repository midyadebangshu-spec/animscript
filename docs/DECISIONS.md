# Decisions

Every ambiguity resolved while implementing the shape system, and the reason. Section references
are to the AnimScript Language Specification v0.1 ("the spec"). Items marked **needs review** are
judgement calls I expect you to want to look at.

## Scope and architecture

1. **Rust workspace.** `animscript-core` (shape model + diagnostics, no parsing), `animscript-lang`
   (lexer, parser, interpreter, tick loop, snapshots, check), `animscript-render` (snapshot → PNG,
   `tiny-skia`), `anim` (CLI: `check`, `run`, `render`). The Python prototype was discarded.
2. **Reduced runtime.** The repo had no runtime, so I built the slice the shapes need: scenes
   (`for N.seconds` / `for auto`), `setup:`, `main:`, `when` / `whenever` on flags and `scene.starts`,
   `together:`, `wait`, `repeat N times`, `if`/`else`, `for`, `let`, `assert`, `note`, `prop` with
   `states:`, `becomes`/`clears`. Not built: `race`, `while`, `finally`, `emit`, `start`/`await`,
   derived flags, physical events, cast checks, shots, timeline, audio, Script mode, assets, imports.
   Anything unsupported inside a scene is reported as `E-SYN-001` with a hint.
3. **Layer stack reduced to L0 + L1.** Vertex positions have a rest value (L0) and timed
   `move_to` / `move_by` / `morph_to` contributions that override it (L1). Procedural modifiers,
   keyframes, locks and constraints (L2–L4) are not implemented, so "ordinary animatable properties
   (rest, procedural, keyframes)" is only met for rest + actions. **Needs review.**
4. **Channels.** Each vertex of each shape is its own channel (key `(shape, vertex)`), policy
   `queue` only, atomic acquisition (an edge or face move needs all its vertices at once). So two
   moves of the same vertex run one after the other; moves of different vertices run together.
5. **Easing.** `linear`, `ease_in/out/in_out`, the three cubic variants, `ease_out_back` (default k),
   `step`. `cubic_bezier(...)` is not implemented. Default easing for vertex moves is `ease_in_out`.

## Syntax added on top of the spec (approved)

6. `for i in a..b:` inclusive range (matches 1-based IDs); `for x in list:`. List indexing is **0-based**
   (IDs are 1-based; lists are not IDs).
7. Bare top-level statements are the **build phase**, so the task's examples (`sq1.v1.move_to(0, 0)`)
   work at the top level. `shape name = expr` registers a shape; `shape name = expr:` followed by a
   block runs that block in the build phase.
8. `over D [ease E]` is a **statement modifier**: `cloud.v[2].move_by(0, 10) over 0.3.seconds ease ease_out`
   (the form in the task). `over:` / `ease:` as named arguments also work.
9. `v1.1` is rejected with `E-SYN-001`; the hint points at `s.v1` and `s.v[n]`. Element names are
   matched as `[vef][1-9][0-9]*`, so `v0` and `v01` are not element names.
10. **`20.degrees` and `20.deg`** are both accepted (task and spec spell it differently); likewise
    `second(s)`, `s`, `ms`, `frame(s)`, `f`, `turn(s)`, `rad`, `px`. A bare number in a shape operation
    is in canvas pixels. `Int / Int` is a (true) division giving a Number; `%` is Euclidean remainder.
11. **`#` is both comment and colour.** `#` followed by exactly 3, 4, 6 or 8 hex digits and then a
    non-identifier character is a colour; anything else starts a comment. So `#fff` with no space is a
    colour, and a comment that starts with a hex-looking word (`#bad`) must have a space after the `#`.

## Shape model

12. **Orientation.** The screen origin is top-left and +y is down. "Counter-clockwise" means
    *visually* counter-clockwise on screen, so a square is v1 top-left, v2 bottom-left, v3 bottom-right,
    v4 top-right. Signed area (shoelace, numeric y-down) is therefore **negative** for the standard
    winding. A circle starts at the top and then goes counter-clockwise (v2 is to the left of v1).
13. **Positions are absolute canvas coordinates.** There is no per-shape transform. `at:` is the
    *centre* of a primitive (default `(0, 0)`). `triangle(size)` is equilateral, point up, side = `size`.
14. **A face is an ordered loop of edge IDs** plus a direction per entry (`+1` = a→b, `-1` = b→a).
    The direction is needed because shared edges are walked in opposite directions by neighbouring faces.
15. **Retired elements stay in the tables.** `delete`, `subdivide`, `bevel`, `round`, `merge` and
    `smooth` retire elements (`deleted` / `replaced`). A retired ID can never be reused, `children` /
    `parent` / `created` / `alive` still work on it, and every other use is `E-SHP-002` whose message says
    why ("was subdivided into e5, e6") and whose help suggests the replacement or the live IDs.
    `E-SHP-002` is also raised for an ID that was never allocated (`s.v9`).
16. **`children` vs `parent`.** `children` lists the *same-kind* elements created from this one (so
    `sq1.e1.children` is the new edges, as the task asks). `parent` is the element the operation was
    applied to, *of any kind* (`sq1.v5.parent` is `e1`). `created` lists everything of any kind.
17. **A face keeps its ID when its edges change** (subdividing an edge, bevelling a corner, smoothing),
    but a face that is subdivided *into a grid* is replaced by new faces.
18. **Semantic names** are aliases that must be unique across *all* elements of a shape (so `s.left`
    is unambiguous), lower snake case, must not look like an ID, and must not clash with a member name.
    Duplicates are `E-SHP-004`; invalid names are `E-SHP-005`. `names:` must have exactly as many entries
    as elements produced. A name is checked before anything is changed (a failed call changes nothing),
    and names are never reused after their element is retired. A name can also be set with `s.e1.name = "x"`.

## Operations

19. **`subdivide(n)` on an edge** splits at equal steps of the curve parameter (for a bent edge the new
    vertices lie on the curve and the pieces stay curved; this is *not* equal arc length). It replaces the
    edge in every face that uses it. `n >= 2`.
20. **`subdivide(n)` on a face** needs exactly four sides and straight edges (else `E-SHP-005` with help).
    The four boundary edges are subdivided (retired and replaced, so neighbouring faces stay consistent);
    interior vertices, then horizontal then vertical interior edges, then `n*n` faces (row by row) get new
    IDs. The original face is retired with those faces as `children`.
21. **`move_to` on an edge or face.** A vertex goes to `(x, y)`; an edge's *midpoint* goes there; a face's
    *centroid* (mean of its distinct vertices) goes there; all its vertices move with it.
22. **`extrude(edge, by)`.** Creates 2 vertices (offset along the outward normal), 3 edges, and 1 face
    that shares the original edge. "Outward" is the side away from the single face that uses the edge,
    or the right of travel for a free edge. An edge shared by two faces has no outward side (`E-SHP-005`).
    `by == 0` is an error; a negative `by` pushes inward. The new face is wound like the primitives.
    The extruded edge's three new edges are its same-kind children.
23. **`bevel(r)` / `round(r)`** act on a vertex with exactly two live, straight edges. The cut points are
    `r` away along each edge (so `0 < r < ` the shorter edge). The corner vertex is retired (children: the
    two new vertices). The two existing edges **keep their IDs** (only one endpoint changes), and one new
    edge joins the cut points. `round` joins them with a quadratic Bézier whose control point is the old
    corner: a good approximation of a fillet, not a true circular arc. **Needs review.**
24. **`smooth(levels)`** is Chaikin corner cutting (¼ / ¾ points). It requires every live vertex to have
    exactly two live straight edges (closed outlines). Anything else is `E-SHP-005` with help
    (e.g. an `extrude` leaves branching vertices; `delete` the base edge first, or use `bevel`/`round`).
    All old vertices and edges are retired; faces keep their IDs and get the new loop. Each new vertex's
    `parent` is the edge it was cut from; each corner edge's parent is the vertex it replaces.
25. **`merge(a, b)`** welds `b` into `a` (`a` survives, `b` is retired). The weld position is the midpoint
    by default; `at: "first"` / `"last"` picks a vertex. Edges that become self-loops are deleted, faces left
    with fewer than 3 edges are removed (with `W-SHP-010`).
26. **`delete`.** A vertex deletes its edges and the faces using them; an edge deletes the faces using it;
    a face deletes only itself. Other vertices are never removed automatically (isolated vertices stay).
    There is no "dissolve" (merge two faces) operation yet.
27. **Curves.** An edge is straight, quadratic or cubic. Handles are stored in the edge's own frame
    (`control = p0 + u·d + w·perp(d)`), so a bent edge keeps its shape when its vertices move or animate.
    `bend(a)` makes a quadratic whose end tangents deviate from the chord by `a`; **positive bulges to the
    right of travel on screen, which is outward for a counter-clockwise face**. `|a| < 90°`.
    Extra operations: `set_handle(x, y)`, `set_handles(p1, p2)`, `straighten()`.

## Build time versus runtime

28. **Topology operations** are `subdivide`, `extrude`, `bevel`, `round`, `smooth`, `merge`, `delete`,
    `bend`, `set_handle(s)`, `straighten`, `copy` and creating shapes. They are allowed at the top level,
    in a `shape` block and in `setup:`. In `main:`, rules and everything they start they raise `E-SHP-003`,
    **both statically** (`anim check`, with the call's span) and dynamically. `bend` and the handle operations
    count as topology edits (a curve change is a structural change and morphing needs matching structure).
    **Needs review** if you want curve handles to be animatable.
29. `over` is only valid at runtime (`E-SHP-005` in build/setup). A runtime `move_*` without `over` is a
    zero-time staged write applied at the end of the delta, like `becomes`.
30. **Morph.** `a.morph_to(b)` pairs the i-th live vertex of `a` with the i-th live vertex of `b` in
    ascending ID order, and needs equal live vertex counts (`E-SHP-001`, message
    `cannot morph: 4 vs 12 vertices; subdivide `sq1` first`, naming the *smaller* shape). Targets are `b`'s
    rest positions when the action starts. Only vertices move; curve handles are not morphed. The check
    runs statically when both operands are plain shape names, and always at runtime.
31. **`W-SHP-010`** is checked after every authoring-time edit (not at runtime) on every live face: fewer
    than 3 edges, |area| < 1e-9, or a self-intersection (proper crossing of non-adjacent segments of the
    face outline, curves flattened to 8 segments). A face is warned about once per problem, and again if it
    is fixed and then breaks again.
32. **New diagnostic `E-SHP-005`**: invalid argument, or operation not applicable to that element
    (e.g. `bevel` radius too large, `subdivide(1)`, `extrude` an interior edge, `smooth` on branching
    geometry). Other codes used outside the shape list: `E-LEX-001..004`, `E-SYN-001/002`, `E-PRJ-001`,
    `E-NAM-001/002/003`, `E-TYP-001/002/003`, `E-STM-001/003`, `E-SCN-001`, `E-TRG-001`, `E-RUN-001/004`,
    `W-TIM-001`, and one new code `E-RUN-005` (division by zero).

## Spec errata I had to decide (from my first review of the spec)

33. **Delta numbering.** Every tick starts with a *delta 0* that contains only phase ⑤ for tasks made ready
    in phase ② (at tick 0: `setup:` then `main:`). Delta cycles of ③–⑤ are numbered from 1 and always run at
    least once per tick. The trace uses the same numbering everywhere.
34. **Scene length.** `for 10.seconds` at 24 fps is 240 frames: ticks 0..239 each produce a snapshot; the
    scene ends in phase ② of tick 240, before that tick is simulated. `for auto` ends after the tick in which
    `main:` finished (that tick's snapshot is emitted).
35. **Number → Duration** conversion is exact: a number's shortest decimal representation is read as a
    rational (`0.6` is 3/5, not a binary fraction). Durations round half to even; a positive duration that
    rounds to 0 becomes 1 frame with `W-TIM-001`.
36. **`when` with a false guard** is *not* spent (the occurrence is dropped, a later one can still fire).
    `whenever` always uses `retrigger ignore`.

## Snapshot, hash and rendering

37. **Snapshot** (JSON): `tick`, `project_tick`, `scene`, `canvas`, `background`, `camera`, `entities`
    (flags), and `shapes` sorted by `z` then declaration order. Each shape has `vertices` `[id, x, y]`,
    `edges` with resolved absolute control points (`pts`: 2, 3 or 4 points) and `kind`, and `faces` as lists of
    directed `segments` (control points in traversal order). Nothing in the renderer knows about topology.
38. **Hash.** 64-bit FNV-1a over a canonical encoding (sorted keys) where every floating-point number is
    quantised to 1e-6 (round half to even). Vertex positions, curve handles, flags and ticks are included.
    Note: values within 1e-6 of a quantisation boundary can still differ across machines (the spec's claim
    of cross-machine stability in standard mode is not strictly true).
39. **Renderer.** `tiny-skia`; faces are filled with the non-zero winding rule, then every edge is stroked
    (round caps and joins). `--debug` adds a dot on every vertex. Fill defaults to none and stroke to
    dark grey. Rendering is a pure function of the snapshot.
40. **Golden test.** The finished flower is stored as a quantised digest (`tests/golden/flower.json`);
    regenerate with `UPDATE_GOLDEN=1 cargo test -p animscript-lang --test flower`.

## Round 2: dissolve, inset, selection sets, lambdas, mirror and flip

41. **Lambda syntax (approved).** `v -> expr` and `(v, i) -> expr` (element, index). The body extends as far
    right as an expression can, so `where(v -> v.x > 0 and v.y < 5)` works. Lambdas capture the variables in
    scope *by value* when they are written. They take one or two parameters (`E-SYN-001` otherwise). In this
    build they are only meaningful as the argument of `where`; a lambda is a first-class value but nothing
    else calls it yet.
42. **Selections are plain lists.** `s.vertices`, `s.edges`, `s.faces` and `x.children` are lists of element
    references; no new value type. Selection methods: `every(n, offset: 0)`, `first(n)`, `last(n)`, `skip(n)`,
    `reverse()`, `where(lambda)`, `nearest(point, n = 1)` (stable on ties, measured from an element's anchor),
    `union(other)` and `except(other)` (order of first appearance, no duplicates), plus the list attribute
    `count`. Lists stay 0-based. Selection methods work at runtime too (they only read).
43. **Element attributes added for lambdas.** `id` (also works on retired elements), `x` / `y` / `position`
    for every kind (vertex position, edge midpoint, face centroid: the same anchor `move_to` uses),
    `length` for edges (the chord between the end vertices) and `area` for faces (absolute value).
44. **Broadcast.** These element operations also work on a list: `subdivide`, `extrude`, `bevel`, `round`,
    `dissolve`, `inset`, `delete`, `bend`, `set_handle(s)`, `straighten`. They run one element after the other
    with the same arguments and the first error aborts (a build-time error aborts the whole program, so a
    partial result is never observable). An element that an *earlier step of the same call* removed (for example
    an edge deleted together with its vertex) is **skipped**; an element that was already dead before the call
    is still `E-SHP-002`.
45. **Group moves.** `list.move_by(dx, dy)` moves every distinct vertex of the selection once (a vertex shared
    by two selected edges is not moved twice). `list.move_to(x, y)` is a *group* move: the centroid of the
    distinct vertices lands on `(x, y)` and the shape of the selection is kept. With `over` it is one action
    (one `ACT+` trace line), whose channels are all the vertices at once.
46. **New operations are topology operations**, so they are `E-SHP-003` at runtime, statically and
    dynamically: `dissolve`, `inset`, `mirror`, `flip` (and the existing ones). `flip` is classified that way
    because it reverses face loops.
47. **`dissolve` on an edge** needs exactly two faces on it, wound consistently (opposite directions on the
    shared edge); otherwise `E-SHP-005`. The lower face ID survives with the merged loop; the other is retired
    as *replaced* and lists the survivor as its `children`, so a stale reference gets the hint "use f1". The
    edge is retired as deleted. Vertices are never removed, even if they end up with two edges (dissolve them
    separately).
48. **`dissolve` on a vertex** needs exactly two straight edges (a curved edge would lose its shape:
    `E-SHP-005` with a hint). The lower edge ID survives, rewired to the far end of the other; the vertex and
    the other edge are retired. A face left with fewer than 3 edges is removed with `W-SHP-010`. Dissolving a
    face is an error (use `delete`).
49. **`inset(amount)`** shrinks a face so that every edge moves inward by `amount` (corners follow the mitre).
    It creates one inner vertex per corner, a spoke edge per corner, an inner edge per side and one ring face
    per original side (so a selection of faces is inset face by face, each with its own ring: "do what is
    best", because overlapping rings are never wanted). The original face **keeps its ID and becomes the inner
    face**. New elements have the face as `parent`. Needs straight edges. `E-SHP-005` if the amount is not
    positive, a corner folds back, or the inner face would collapse or invert; a local self-intersection of the
    result (very sharp or very short corners relative to the amount) is `W-SHP-010`.
50. **`flip(x:|y:|line:)`** reflects the whole shape in place. No ID changes; every live face loop is reversed
    so the winding convention (and the sign of its area) is kept, and curve handles are reflected (the
    sideways component of each handle changes sign). Whole shape only; flipping part of a connected shape would
    tear it.
51. **`mirror(x:|y:|line:, faces:, weld:)`** adds a reflected copy joined to the original. Vertices within
    `weld` (default 1e-6) of the axis are *shared*, and so are straight edges whose ends are both shared, so a
    half shape closes into a symmetric one. New elements are created in source-ID order (vertices, then edges,
    then faces) with the original as `parent` (so `v2.children` is `[v6]`). Each mirrored face is the
    reversed loop, so it has the *same winding as its original* and the seam is walked in opposite
    directions by the two faces (which is what `dissolve` needs). Curved edges are duplicated with reflected
    handles even when both ends are on the axis. `faces:` (a list of faces of this shape) mirrors only those
    faces together with their edges and vertices; default is the whole shape. `E-SHP-005` if every vertex is on
    the line.
52. **Mirror lines.** `x: 400` is the vertical line x = 400, `y: 300` the horizontal line y = 300,
    `line: ((x1, y1), (x2, y2))` any line through two different points. Exactly one of the three must be given
    (`E-TYP-003`).
53. **Known limitation.** `smooth` after `mirror` needs the seam dissolved first (`leaf.e5.dissolve()`), which
    the leaf example does. There is still no per-face colour, so an inset ring and the inner face share one fill.

## Round 3: per-face colour, shape transforms, keyframes and the layer fold, radial_array

54. **Per-face and per-edge style.** A face has its own `fill`; an edge its own `stroke` and `stroke_width`.
    `none` (a new literal) clears it, and an element without its own style uses the shape's. Assignment works
    on one element or on a whole list (`s.faces.where(...).fill = #f00`) and is a zero-time staged write at
    runtime. Colour is **not** animatable yet (no tints or fades).
55. **Style inheritance.** A new element copies the style of its parent when the parent is of the same kind,
    so these keep their colour: the ring faces made by `inset`, mirrored and `radial_array` copies, the faces of
    a subdivided face grid, and the two halves of a subdivided edge. Two special cases: `smooth` gives the
    corner edges the style of the edge they were cut from, and `bevel` / `round` give the new edge the style of
    the lower-numbered edge at the corner. Faces and edges made from an element of another kind (an extruded
    face, the spokes of an inset) start unstyled. `dissolve` keeps the surviving face's own fill.
56. **Snapshot style.** Faces carry their *effective* fill (own, else the shape's) and edges their effective
    stroke and width, so the renderer reads colour straight from the snapshot and has no fallback logic. The
    shape-level `fill` / `stroke` are still reported.
57. **Shape transform.** A shape has `position` (a translation, default (0, 0)), `scale` (default (1, 1)),
    `rotation` (an Angle, default 0) and `pivot`. Vertex coordinates stay local; the runtime resolves
    `scale` about the pivot, then `rotation` about the pivot (clockwise on screen, like every Angle), then
    `position`, and the snapshot is in world space (vertices, edge points and face segments).
    Handles are transformed point by point (an affine map keeps a Bézier a Bézier exactly, even under
    non-uniform scale). A negative scale reverses the winding; faces are filled with the non-zero rule, so that
    is harmless. An identity transform is applied exactly (no round-off), so programs that never use a
    transform are bit-for-bit unchanged.
58. **Pivot (approved: bounding-box centre).** An unset pivot is the centre of the bounding box of the shape's
    live vertices, **frozen when the build phase ends** (and again after each scene's `setup:`, for shapes made
    there). Later edits or animation of vertices do not move it. `s.pivot = (x, y)` sets it explicitly and is
    never replaced; `s.pivot = none` goes back to automatic. Curve handles do not count towards the box.
59. **Rotation interpolates linearly (approved deviation from spec 7.3).** The spec lerps angles by the
    shortest arc, which would make `rotate_by(360.deg)` do nothing. Rotation is stored as an unbounded
    number of turns and actions and keys interpolate it linearly, so a full spin, or `rotate_by(720.deg)`, works.
60. **Shape actions.** `s.move_to((x, y))` / `s.move_by(dx, dy)` change `position`, `s.scale_to(f)` (a number or
    `(sx, sy)`), `s.rotate_to(a)` and `s.rotate_by(a)` change the rest transform. Without `over` they are
    instant (build time) or staged zero-time writes (runtime); with `over` they are timed actions that commit
    their final value to rest when they finish. Note that a *shape's* `move_to` sets its `position`, while
    an *element's* `move_to` moves vertices in local space. A shape's transform has its own channel
    (`body`), so a transform action runs alongside vertex actions; two transform actions queue.
61. **The layer fold is real now (spec 7.2 / 7.3).** Every scalar (a vertex x or y, a transform component) is
    resolved each tick from its rest value and its contributions, sorted by `(layer, priority, start tick, id)`;
    `override` is `lerp(v, value, weight)`, `add` is `v + weight * value`, `multiply` is
    `v * lerp(1, value, weight)`. Layer 1 holds actions (always weight 1), layer 3 keyframes; layers 2
    (procedural) and 4 (locks) are reserved. A unit test reproduces the worked example of 7.10 (397.1, and 414.2
    without the keyframe set). `multiply` exists in the fold but nothing produces it yet.
62. **Keyframes (spec 7.7).** `target.keyframes over D:` followed by `at TIME: property = value [ease E]`
    lines (an `at` block may also hold several property lines). Targets: one vertex, a selection (every vertex
    of its elements, a shared vertex keyed once) or a shape. Properties: `position`, `position.x`,
    `position.y` (vertices and shapes), `scale`, `scale.x`, `scale.y` and `rotation` (shapes only: `E-NAM-003`
    for a vertex). Keys are grouped per scalar. The easing of a key shapes the segment that ends at it, and the
    default is **`linear`** (spec 7.4/7.7), unlike moves, which default to `ease_in_out`.
63. **Keyframe range.** A key set contributes from its first to its **last key tick inclusive** and then lets go:
    the layer below shows through again (nothing is committed to rest, unlike an action). `hold_before` extends
    the first value back to the start of the statement, `hold_after` extends the last value until the scene
    ends, `blend_in: D` ramps the weight from 0 to 1 over D after the start and `blend_out: D` from 1 to 0 over D
    before the end. The statement blocks for `over` D (and `E-TYP-001` if a key is later than D), so the next
    statement starts when the last key is reached. A key set is a layer, not an action, so it never waits for a
    channel and stacks on top of a running action (keyframes win while they are active).
64. **`+=` keys (approved extension).** `at 1.s: position.y += 10` adds an offset on top of the layers below
    (layer-3 `add`), which is what a selection needs, because an absolute `=` would put every selected vertex at
    the same place. One property cannot mix `=` and `+=` within a statement (`E-TYP-001`). Two keys for one
    property at the same time are `E-PRP-003`.
65. **`timeline:` (spec 4.9 / 7.7).** A scene item with `at TIME:` blocks in **absolute scene time** and full
    targets (`s.v[1].position.x = 0`, `s.rotation = 90.deg`, `s.vertices.position.y += 10`); a timeline line
    without a target is `E-TYP-001`. Its key sets are registered at tick 0, after `setup:`. Values are
    evaluated once, when the key sets are registered, not every tick. Markers are not implemented.
66. **`radial_array(count, around, angle = 360.deg, faces:, weld:)` (approved).** Adds copies of the whole shape
    or of the given faces, rotated about `around`, *inside the same shape*; `count` includes the original. A full
    turn (|angle| of 360° or more) uses `count` equal steps and closes the ring; a smaller angle spreads the
    copies over the arc, so `count: 4, angle: 90.deg` has steps of 30° and ends exactly at 90°. Vertices that
    land within `weld` (default 1e-6) of an existing vertex are shared, and so are straight edges between shared
    vertices, so neighbouring copies join. Curved edges are always duplicated. New elements are created in
    source-ID order, copy by copy, with the source as `parent`, and inherit its style. It is a topology
    operation (`E-SHP-003` at runtime). The weld scan is quadratic in the vertex count.
67. **Not in this round:** procedural modifiers (`wobble`, `noise`) and locks, colour animation, a linear
    `array`, and markers in `timeline:`.
68. **Golden file.** The snapshot gained fields (transform, per-face and per-edge style), so the flower's hash
    changed. Its geometry digest did not: only the `hash` line of the golden file differs.
