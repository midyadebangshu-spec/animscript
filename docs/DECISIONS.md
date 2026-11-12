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

## Round 4: colour animation, procedural modifiers, spring and physics-style motion, handles

69. **Colour animation (approved: linear light).** Colours are written and stored as 8-bit sRGB. Animation
    treats each colour as four scalars (r, g, b, a) in **linear light** (alpha is already linear), as spec 3.1 /
    7.3 require, blends them through the layer fold, and converts back to 8-bit sRGB rounding half to even
    (`color.rs`). `to_linear` / `to_srgb8` round-trip every 8-bit value exactly, so an unanimated colour comes
    out bit-for-bit as written. The linear midpoint of black and white is `#bcbcbc`, not `#808080`. Rest colours
    stay 8-bit (approved): a finished colour action commits a rounded 8-bit colour.
70. **What is animatable.** A face's own `fill` (`fill_to`), an edge's own `stroke` (`stroke_to`) and
    `stroke_width` (`width_to`), the shape's `fill`, `stroke`, `stroke_width`, and two new shape properties
    (approved): `opacity` (0 to 1, `fade_to`) and `tint` (`tint_to`, default white). Each has its own channel
    (so two `fill_to` on one face queue, but colour actions run beside vertex moves). A selection
    (`s.faces.fill_to(...)`) is one action. Without `over`, each is an instant (build time) or staged
    (runtime) assignment.
71. **From `none`.** A fill that is not set at all fades in from transparent without a colour shift: the
    rgb start at the target and only alpha animates from 0. `fill_to(none)` is `E-TYP-001` (use `fade_to`).
72. **Colour pipeline.** An element's colour is its own (animated or not), else the shape's (animated or not);
    then `tint` multiplies rgb and alpha and `opacity` multiplies alpha, and the result is converted to hex
    (`#rrggbb`, or `#rrggbbaa` when not opaque). The snapshot carries the effective colour of every face and
    edge plus the shape's `opacity` and `tint`. Opacity covers strokes too. Tint and opacity at their
    defaults multiply by exactly 1.0, so existing output is unchanged.
73. **Colour keyframes.** `fill`, `stroke`, `stroke_width`, `opacity` and `tint` can be keyed like position.
    `fill` needs faces and `stroke` / `stroke_width` edges (`E-SHP-005` for the wrong kind, `E-NAM-003` for a
    vertex); the shape accepts all five. `+=` is not allowed for colours (`E-TYP-001`) but is for
    `stroke_width` and `opacity`.
74. **Procedural modifiers are layer 2 (approved: wobble and noise).** `target.wobble(amount, speed = 1,
    axis = both, sync = false, phase, property, blend_in, blend_out)` and `.noise(...)` add an offset on top of
    everything below and under keyframes. Wobble is `amount * sin(2 pi (speed * t + phase))` (speed in cycles per
    second); noise is `amount * value_noise(speed * t)` (speed in lattice steps per second). `t` is seconds since
    the modifier started. Targets: a vertex or selection (position, in local space, before the shape transform;
    `axis: x | y | both`), or a shape with `property: position | scale | rotation | opacity` (a uniform scale shares
    one stream; `rotation` takes an Angle amount). A modifier is active for ticks `[start, start + D)` and then
    lets go; `blend_in` / `blend_out` fade it. Modifiers on colours are not supported, except `opacity`.
75. **Independent phases (approved).** Each vertex (and each axis) gets its own seeded phase or noise stream
    unless `sync: true`, which shares one stream per axis across the selection. `phase:` (in turns) overrides the
    seeded phase of a wobble.
76. **Randomness deviates from the spec's PCG32.** Streams are named
    `"<scene>/<shape>/<target>/<kind>#<n>/<axis>"`, hashed (FNV-1a) with the project seed, and read through
    *pure, counter-based* functions (SplitMix64 for `hash01`, smoothstep value noise on top), not a sequential
    generator. So any frame can be computed on its own, and an unrelated modifier, rule or shape never changes the
    numbers of an existing one (spec test T14; `n` counts calls on the same target and kind, not calls overall).
    The builtins `noise("name", t)` (in [-1, 1]) and `random("name", index = 0)` (in [0, 1)) read the same
    streams. **Needs review** if you want literal PCG32.
77. **Handles (approved).** `let h = start CALL [over D] [ease E]` (or a bare `start CALL`) begins a timed
    action or a modifier without blocking, `cancel h` stops it and `await h` blocks until it ends. `over`-less
    modifiers are open-ended and need `start`; as a plain statement an open-ended modifier is `E-TYP-003`.
    Cancelling an action keeps the last value it sampled (spec 7.6); cancelling a modifier just removes it.
    Awaiting something already finished or cancelled returns at once; awaiting an open-ended modifier is
    `E-RUN-006` (new). Started actions use the same channels and queue as any action. `start` works on actions
    and modifiers, not on `keyframes`. Handles last until the scene ends.
78. **Spring easing (approved).** `spring(bounces = 3, damping = 5)` is an easing value:
    `f(p) = (1 - e^(-d p) cos(2 pi n p)) / (1 - e^(-d) cos(2 pi n))`, so it overshoots `n`-ish times,
    starts at 0 and lands **exactly** on 1. It is stateless (no integration) and needs `damping > 0` and a
    settling value above 0.2 (`E-TYP-001` otherwise). `ease_out_back(k)` and `cubic_bezier` are still not
    available as calls.
79. **Physics-style motion (approved: included).** `fall(floor, gravity = 980, bounce = 0, velocity = (0, 0))`
    and `bounce_in(rect(x, y, w, h), velocity)` are **layer-1 override actions** (they commit where they end,
    which a falling object should), not layer-2 modifiers as the spec's wording suggests. Both need `over`. The
    paths are analytic and stateless: free fall with constant acceleration, bouncing on a floor with a
    restitution of 0 to 1 (a bounce slower than 0.5 px/s stops it; at most 200 bounces), and constant velocity
    reflected between four walls (a triangle wave). The floor is in the property's own units: the local y of a
    vertex, or the `position.y` of a shape (so `shape.fall(floor: 400)` lets the shape drop 400 px). Only fixed
    floors and walls are supported: **no collisions between bodies**, no rotation, and the speed at the end is not
    kept. **Needs review** if you expect a real physics engine (the spec defers that to 0.4).
80. **Constants.** In argument position the names `x`, `y`, `both`, `position`, `scale`, `rotation` and
    `opacity` stand for themselves (`axis: y`, `property: rotation`) when no variable of that name exists.
81. **Golden file.** The snapshot gained `opacity` and `tint` (and per-element colours are now computed in
    linear light), so the flower's hash changed again; its geometry digest did not.

## Round 5: the physics engine (rigid bodies, joints and springs)

82. **Own engine, no dependencies.** `crates/physics` is a Box2D-style 2D engine written for this project
    (the plan was approved over wrapping Box2D): `f64`, single threaded, fixed iteration orders, only exactly
    rounded arithmetic in the hot path (`sin`/`cos`/`atan2` only convert angles at the edges), so a program
    always produces the same hashes. A `PhysicsWorld` trait (step, body count, events) marks where a 3D backend
    (a Jolt wrapper) will plug in later; nothing 3D exists yet.
83. **Solver.** Soft-step sequential impulses: 4 solver substeps per engine step, warm starting, relax pass,
    speculative contacts (a contact within `0.02 m + speed * dt` is kept so a fast body is stopped at the
    surface), 0.005 m linear slop, push-out speed capped at 3 m/s, Coulomb friction solved before the normal
    constraint, restitution with a 1 m/s threshold, friction mixed as `sqrt(a * b)` and restitution as `max`.
    Joints are soft constraints too (60 Hz, damping ratio 2).
84. **Units.** The language is in pixels, seconds and kilograms; the engine in metres. `world: scale:` (default
    100) is pixels per metre, `gravity` is in m/s^2 with y down (default `(0, 9.8)`), so 9.8 m/s^2 is 980 px/s^2.
    Velocities read from a body are px/s; `apply_impulse((x, y))` is in kg*px/s (a 1 kg body gains x px/s).
    Density is kg per square metre, so a 100 px square of density 1 weighs 1 kg.
85. **`substeps`** (default 4) is the number of engine steps per frame, each of `1/(fps * substeps)` seconds. At
    24 fps that is 1/96 s, which the engine tests use.
86. **Shapes become bodies.** Each live face of the shape is flattened (curves at 0.5 px), and concave faces are
    split into convex pieces (ear clipping, then Hertel-Mehlhorn merging) whose total area equals the face's.
    A face that crosses itself or has no area is `E-PHY-001`. A static body with no faces uses its edges as thin
    walls; a moving body needs area. Circles made with `circle(n, ...)` are n-gons. Face holes are not supported.
87. **Pose.** At the start of every scene (after `setup:` and pivot freezing) each body's pivot is fixed at its
    centre of mass and its `position` is set so the pose is unchanged; `position`/`rotation` of the shape then
    *are* the body's pose. A uniform `scale` is baked into the collision shape at that moment; changing scale
    later does not change the body, and a non-uniform scale is `E-PHY-001`.
88. **Layers.** A dynamic body writes layer-2 *override* contributions on `position.x`, `position.y` and
    `rotation`. Actions (layer 1) lose to physics, keyframes (layer 3) win and are not fed back into the world
    (the body keeps simulating underneath). A kinematic body writes nothing: before each step the runtime folds
    the other layers, and the body is given the velocity that reaches that pose by the end of the frame, so it
    carries things by friction and pushes them. A static body is never moved; to move a platform make it
    kinematic.
89. **Scene boundaries.** The world is rebuilt at every scene start from the shapes' rest poses with zero
    velocity (plus any `set_velocity`/`apply_impulse` made in `setup:` or at the top level, which are applied in
    order); at scene end each dynamic body's final pose is written to `position`/`rotation`. The step for tick 0
    is skipped so the first snapshot shows the starting pose. `rotation` accumulates across wraps (it can pass
    1 turn).
90. **Creation.** `.body()`, joint constructors and the `world:` block belong to the build phase or `setup:`;
    in `main:`/rules they are `E-SHP-003`. Bodies made in `setup:` last for that scene only.
91. **Events.** `a.touches(b)` is true while any of the two bodies' fixtures is touching (a rising edge is the
    begin event, so `when a.touches(b):` fires once per contact). It becomes visible one tick after the contact
    starts, like every state the runtime computes after the delta cycles. `a.leaves(b)` is an event true for
    one tick. Touching starts when the closest point is within 1 cm and ends beyond 2 cm. A sensor
    (`sensor: true`) never pushes: it only reports overlap through the same two predicates.
92. **Joints.** `revolute`, `weld`, `prismatic`, `distance` (rigid, or a spring with `spring: (frequency,
    damping)`), `rope` (a maximum length) and `follow` (pull a body to a target, `force` capped) return a joint;
    `follow` joints have `set_target((x, y))`. Joined bodies do not collide unless `collide: true`. `distance`
    with two shapes is the joint; with two points it is still the maths function. Angles in `limits` must be
    angles (`60.deg`); a motor is `(speed, torque)` with the speed as an angle per second.
93. **Arguments.** Tuples with named fields are not in the language yet, so `spring` and `motor` are positional
    pairs. The names `static`, `dynamic` and `kinematic` stand for themselves in `kind:` like `x` and `y` do.
    Tuples gained `.x` and `.y`.
94. **Angle limits.** An engaged revolute limit is solved together with the joint's point constraint as a 3x3
    block (as Box2D v2.4 does), so it holds even a point mass on a long arm, whose own inertia is tiny. (The
    first version solved them separately and leaked by about 0.18 rad on such a body.) Engaged means the limit
    would be violated within the substep; the block impulse is not warm started.
95. **Sleeping.** A body sleeps when its island (bodies linked by touching contacts and joints, never through
    static bodies) has stayed under 0.05 m/s and 2 deg/s for 0.5 s; touching a moving body, an impulse or a
    velocity change wakes the whole island. A moving kinematic body keeps what touches it awake.
96. **Snapshot.** When a world exists the snapshot has a `physics` section (bodies with `kind` and `awake`,
    touching contact points with normals and impulses, joint anchors in pixels); it is part of the hash.
    Programs without physics have no such key, so earlier hashes (the flower golden file) are unchanged.
    `anim render --debug-physics` draws contacts (red, with normals) and joints (blue).
97. **Diagnostics.** `E-PHY-001` (a body that cannot exist: no area, non-uniform scale, unknown kind, bad
    value, second body on a shape, bad `world:` value), `E-PHY-002` (bad joint arguments, a joint or physics
    member on a shape without a body) and `W-PHY-001` (bodies overlap by more than 1 cm at the start of a scene).
98. **Not yet.** Continuous collision for very fast thin bodies, collision filtering by category, body
    reading while building (`velocity` before the scene runs reads zero), face holes, and soft bodies, fluids,
    particles and character rigs (the next rounds).

## Round 6: particles

99. **Declaration.** `emitter name:` is a top-level block of `key: value` lines (like `world:`), evaluated once
    while building, so `at: ball` may name a shape and values may use earlier `let`s. An emitter is a value:
    `name.start()`, `name.stop()`, `name.burst(n)` and the member `name.count` work anywhere, including
    `main:` and rules. Settings: `at`, `rate`, `lifetime`, `speed`, `angle`, `size`, `size_end`, `color`,
    `color_end`, `opacity`, `opacity_end`, `gravity`, `drag`, `restitution`, `collide`, `max`. Unknown ones are
    `E-PTC-001`; `at` that is neither a point nor a shape is `E-PTC-002`.
100. **Ranges and ends.** A range is a `(min, max)` pair (or one number); there is no `+-` operator. The value a
    property has at the end of a particle's life is a separate setting (`size_end`, `color_end`,
    `opacity_end`) instead of an arrow. Lifetimes accept durations (`0.5.seconds`, `12.frames`) or seconds as
    numbers; angles must be angles (`-90.deg`); an angle of 0 points right and positive angles turn clockwise
    on screen. Sizes are radii in pixels. Speeds are px/s and gravity px/s^2.
101. **Randomness.** Each attribute of each particle is a pure function of (project seed, scene name, emitter
    name, spawn index, attribute) through SplitMix64, so a change in one emitter never disturbs another and
    the same program always gives the same particles. The `seed:` in `project` changes them.
102. **Emission.** While started, particle number `k` is born at the first step where
    `rate * active_time >= k`, so the rate is exact (no drift, no carried fraction); `burst(n)` adds `n` at the
    next step. Emitters start stopped at each scene start. Stopping lets live particles finish. Over the `max`
    cap (default 1000; at most 20000 over all emitters) spawns are skipped but still use up an index.
103. **Where they are born.** A point, or for `at: shape` a random point inside the shape's faces (rejection
    sampling in its bounding box, 16 tries, then the centre) using the shape's resolved pose of that tick, so
    particles follow a moving or simulated shape. A shape with no faces emits from its centre.
104. **Stepping.** After the rigid bodies have been stepped for the frame, each emitter takes the same
    `substeps` per frame: gravity, then drag (`v / (1 + drag * dt)`), then position. At tick 0 emitters only
    spawn (a burst at the start shows at once). Particles are stepped against the bodies' poses *after* the
    frame's physics, not interleaved with it.
105. **Collision.** `collide: true` makes particles circles that bounce off every non-sensor fixture with their
    own `restitution` (default 0.4) and no friction, are pushed out of penetration, and take the velocity of a
    moving body at the contact point; an approaching particle is placed on the surface instead of bouncing
    early. They never push bodies and never collide with each other. Very fast particles can still pass through
    something thinner than `speed * dt`. Without a physics world nothing happens.
106. **Output.** The snapshot has a `particles` list (absent when the program declares no emitters) of
    `{name, items: [[x, y, radius, "#rrggbb[aa]"], ...]}` in spawn order, included in the hash. Colours are
    interpolated in linear light like colour animation; opacity multiplies alpha. The renderer draws filled
    circles over the shapes (under the debug overlay).
107. **Engine fix found on the way.** A fast bounce begins and ends its contact inside one frame, and the
    touching test (closest point within 1 cm at the start of a step) never saw it, so `touches` rules missed
    every bounce. A contact the solver pushed on now counts as touching, and `touches` is also true for a tick
    in which a contact began, even if it already ended.

## Round 7: soft bodies

108. **A shape is rigid or soft.** `shape.soft(...)` and `shape.body(...)` on the same shape is `E-PHY-003` in
    either order, as is `.soft()` on a shape without edges. A soft shape's vertices are the particles, its
    edges stretch constraints and its faces optional volume constraints; nothing new is needed in the shape
    model. A mixed shape (soft skin, rigid core) is not supported.
109. **Solver.** XPBD (extended position-based dynamics with compliance) in `crates/physics/src/soft.rs`: per
    substep predict positions, run `iterations` passes over distance constraints then area constraints (the
    sweep direction alternates each pass, so a heavy end mass is felt along a chain), then rigid contacts, then
    soft-vs-soft; velocities come from the position change. Defaults are `soft_substeps: 16` and
    `soft_iterations: 4` in `world:` (small steps converge far better than many iterations; 16 x 4 keeps a
    rope with a 20:1 end mass within about 6% strain). Everything is in fixed order, so results are
    reproducible.
110. **Stiffness.** `stiffness` (above 0, at most 1) maps to a compliance `(1 - s) / s * 0.02` m/N; 1 is rigid.
    `bend` (0 to 1) uses the same mapping for the links that stop a rope or cloth folding sharply: the far ends
    of two edges that run straight through a vertex are joined. Cells with four corners get both diagonals
    (the shear bracing that keeps a jelly square from collapsing). `pressure` (default 1) is an area
    constraint per face at `pressure` times its starting area, with a compliance a quarter of the stiffness
    one, so 1 keeps the volume, above 1 inflates and 0 turns volume off.
111. **Mass.** Each face shares `density` (kg per square metre) times its area among its vertices; edges that
    belong to no face (a rope) count `density * 0.05 m` of width. Pinned vertices behave as infinitely heavy.
112. **Pins.** `pin(vertex or list of vertices)` holds vertices where they are; `pin(v, to: shape)` makes them
    follow that shape's pose (the offset in its frame is remembered), so a kinematic platform or an animated
    shape can carry a soft body. Pins to a *dynamic* rigid body use the pose it had when the frame began (one
    frame of lag). `unpin` and `pin` work while running too; `soft_impulse((x, y))` is in kg*px/s spread over
    the whole body (calls before the scene runs set its starting motion).
113. **Pose.** The simulation runs in world space using the shape's transform at the start of the scene; the
    result is written back as layer-2 overrides of the local vertex positions (so keyframes on a vertex still
    win), and as the rest positions at the end of the scene. Animating the soft shape's own `position` or
    `rotation` therefore moves the drawn shape without affecting the simulation; to move a soft body, pin
    some vertices to a shape that moves.
114. **Collision with rigid bodies.** Every vertex, and the middle of every edge, is a small circle
    (`thickness`, default 2 px) tested against all non-sensor fixtures, each fixture separately (a pad
    squeezed between a box and the floor sees both). The vertex is pushed out and takes the surface velocity
    of a moving body; friction removes tangential speed in proportion to the support (normal) impulse and
    `restitution` bounces fast approaches. A dynamic body is pushed back with the momentum the push-out
    represents, so a soft body supports a rigid one. Soft and rigid bodies advance in lockstep: the soft step
    drives the world's substeps (`soft_substeps` of them per frame) so neither sees stale poses; rigid-only
    scenes still use `substeps`.
115. **Soft against soft.** A vertex is kept on the side of another soft body's edge it started the substep
    on, so it cannot tunnel through; between two different soft shapes this is always on, within one shape only
    with `self_collide: true`. Edges of the same shape sharing a vertex never collide with it.
116. **Limits.** An obstacle thinner than the vertex spacing can pass between vertices (the edge midpoints
    help; subdivide more); a rigid body is not pushed by a soft body whose vertices are all pinned; the grid
    primitive is `grid(cells, width:, height:, at:)` with `cells` x `cells` square-ish cells (use `subdivide`
    on a rectangle for other splits).
117. **Output.** The snapshot has a `soft` section `{bodies: [{shape, volume, stretch, pins}]}` (volume in px^2
    from the faces, stretch the largest edge strain, pins in world pixels), included in the hash; it is absent
    without soft bodies. `anim render --debug-physics` draws pins as blue squares. `shape.volume` and
    `shape.stretch` read the same numbers while running. New errors: `E-PHY-003` (soft and rigid at once, or
    nothing to simulate) and `E-PHY-004` (a bad soft value, or `pin`/`unpin` misuse).

### Round 7 follow-up: the limits fixed

118. **Carrying.** Item 113 is replaced: when the soft shape's own resolved `position`/`rotation`/`scale`
    changes between ticks (an action or keyframes on the shape), the whole simulation, including pin targets,
    is carried through that change with velocities kept, and the output is expressed in the shape's current
    transform. Moving a soft body with `move_by` therefore makes it collide with things on the way (it is a
    teleport, so a fast move into a wall is a violent push-out); pins are still the gentler way.
119. **Thin obstacles.** Item 116's first limit is fixed: an edge near any fixture is sampled every
    `max(2 * radius, 1 cm)` along its length (up to 64 samples), so a 2 cm post between two vertices 50 cm
    apart is felt. Each sample moves the whole edge, and its friction/restitution response is weighted by
    `1 / samples` so a finely sampled edge does not compound it. Edges far from every fixture are skipped.
120. **Pinned vertices are obstacles.** A pinned vertex (or edge between two pinned vertices) cannot move
    out of the way, so a dynamic rigid body that reaches it has its approach velocity removed (relative to the
    pinned point's own velocity) with an impulse at the contact point. A fully pinned soft body now holds a ball.
121. **Pins to dynamic bodies** no longer lag: the target is the body's pose extrapolated by its current
    velocity over one frame (item 112 updated); a pin follows a body in free fall to within a few pixels.
122. **`anim frames`.** `anim frames file.anim -o DIR [--scale s] [--from n --to m] [--debug-physics]` simulates
    once and writes `f0000.png`...; rendering 144 physics frames one at a time re-simulated every time.
123. **Still limits.** Mass ratios far above about 20:1 along one chain stretch noticeably; a soft shape has no
    skin-and-core mode; a very fast rigid body can still tunnel through a soft edge in one substep.


## Round 8: fluids

124. **Solver.** Position based fluids in 2D (`crates/physics/src/fluid.rs`): each particle stands for
    `spacing^2` of fluid; a density constraint (poly6 density, spiky gradient, smoothing radius `2 * spacing`,
    one-sided: only compression is corrected) moves neighbours until the local density matches the rest
    density computed from a perfect lattice, with a tensile correction against clumping, XSPH smoothing for
    `viscosity` and a small pull for `surface_tension`. Neighbours come from a sorted cell list (cells of
    `1.3 h`, visited in cell then index order), so every sum has a fixed order and the result is
    reproducible. Defaults: 4 substeps per frame and 3 density iterations (`fluid_substeps`,
    `fluid_iterations` in `world:`). Particle speed is capped at 20 m/s.
125. **Declaration.** `fluid name:` is a top-level block like `emitter`. Settings: `fill` (or `in`, an alias),
    `spacing` (px, default 8), `density` (default 1), `viscosity` (0 to 1, default 0.05), `surface_tension`,
    `color`, `color_end`, `render` (`"blob"` or `"dots"`), `behind`, `max` (default 4000, 20000 over all
    fluids). `fill` takes `rect(x, y, w, h)` or a shape (its faces are filled; hide the shape if it is only a
    region). Unknown settings or bad values are `E-FLD-001`; a bad `fill` is `E-FLD-002`. A region that needs
    more than `max` particles is an error, not a silent truncation. There is no automatic container: `in:`
    does not create walls; build the container from static bodies.
126. **Lattice and scenes.** The region is filled with one particle per `spacing x spacing` cell, row by row,
    after `setup:` (so shapes are in place). Every scene starts from its own fill: the water of the previous
    scene is not kept. Particles that leave a box of three canvas sizes are removed so lost water does not
    fill the pool.
127. **Methods.** `pour(at:, speed:, width:, rate:)` starts a stream (the rate defaults to the one that makes a
    continuous stream, `|speed| / spacing * width / spacing` particles per second; particles of a frame are
    placed along the stream path so the jet is smooth), `stop()` ends all streams of that fluid, `add(region)`
    adds a lattice block at once, `level(x)` is the surface height (px) within one spacing of `x` (none when
    there is no water), and `count` is the number of particles. A pour needs a speed other than (0, 0).
128. **Rigid bodies, two way.** Particles collide with every non-sensor fixture (a particle is a circle of half
    the spacing). The contact is searched from where the particle was, with a margin for its speed and for the
    solver's own movement, so a fast particle cannot jump a thin wall. Dynamic bodies receive the momentum of
    the push-out as an impulse, which is how a box floats (about Archimedes' depth) and a ball splashes.
129. **Soft bodies, one way.** Soft shapes are obstacles for the fluid (vertices plus samples along edges every
    half spacing): water is pushed by them, but they are not pushed by it. Making this two way is left for
    later. A cloth in a stream therefore does not move.
130. **Lockstep.** `step_coupled` advances the rigid world, soft bodies and fluids together, substep by
    substep (soft solve, fluid solve, world step). The number of substeps per frame is `fluid_substeps` for
    fluids alone, `soft_substeps` for soft bodies alone and the larger of the two when both exist.
131. **Output.** The snapshot has a `fluids` list (absent when none are declared) of `{name, spacing, color,
    color_end, render, behind, particles: [[x, y]...]}` in pixels, included in the hash. The renderer, a pure
    function of the snapshot, draws `"blob"` by splatting a smooth kernel (radius `1.6 * spacing`) into a
    density field and thresholding it with a soft edge (the colour runs from `color` at the surface to
    `color_end` eight spacings down; colours may carry an alpha), or `"dots"` as overlapping circles. Fluids
    are drawn over the shapes, or under them with `behind: true`.
132. **Limits.** The surface is only as fine as the spacing (8 px is a few thousand particles; halving it is
    four times the particles); water at walls has a small density deficit, so it settles about a spacing
    lower than the exact level; there is no air (no bubbles) and no two-way coupling with soft bodies.

### Round 8 follow-up: finer water, a crisper surface, foam and spray

133. **Why the water looked coarse.** It was the example's `spacing: 10` (a particle stands for a 10 px patch)
    and a fixed blob radius of 1.6 spacings, not an engine limit. The example now uses `spacing: 5` (four
    times the particles: about 2000 at the start, up to 4500 with the pour; 5 s for the 7 s scene in a release
    build, so use `anim frames` and the release binary).
134. **Smoothness.** `smoothness` (0.8 to 4, default 1.4) is the radius of the blob surface in particle
    spacings. The renderer now normalises the density field by the value of a particle in the middle of a
    perfect lattice, so the surface always sits at 18% to 30% of full density whatever the radius; lower
    values are crisper, higher values rounder. The threshold is low enough that a lone droplet stays about
    one spacing wide. A light rim just inside the edge gives the surface a meniscus highlight.
135. **Depth shading** is measured from the surface of each connected run of water in its column (a gap of
    more than 2.5 spacings starts a new run, so a droplet in the air is its own surface), over a fixed 90 px
    from `color` to `color_end`, instead of from the highest particle in the column (which made the water
    far too dark once there was spray) and independent of the spacing.
136. **Foam and spray.** `foam` (0 to 1, default 0 = none) turns on secondary particles that never act on the
    water. Each substep, a water particle moving faster than 1 m/s and near the surface (at most 5 neighbours
    within the smoothing radius, against 8 at rest) throws a foam particle with probability
    `foam * 6 * (speed - 1) * dt`, drawn from a seeded stream (project seed, scene, fluid, substep, particle),
    so replays are identical. A foam particle in water (5 or more neighbours) is carried with the local water
    velocity and floats up; deep inside (8 or more) it dissolves four times faster, so only the surface keeps
    foam; in the air it flies as a projectile and ages 1.5 times faster. It stops at rigid bodies. Life is
    1.5 s (0.6 to 1.4 times that), at most 2000 per fluid.
137. **Output.** The snapshot's fluid entries gained `smoothness` and `foam` (`[x, y, age fraction]` per
    particle); the renderer draws foam as small white specks that shrink and fade with age, after the water.
    Both are part of the hash. `E-FLD-001` also covers `foam` outside 0 to 1 and `smoothness` outside 0.8 to 4.
138. **Not done.** True air (two-phase flow with bubbles and trapped pockets) stays out: a 1000:1 density ratio
    is unstable in this kind of solver and doubles the particles.
