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

## Round 9: character rigs, sub-round 0 (images and textured meshes)

139. **Plan for the rig rounds** (agreed): bones are nodes of an armature (not shapes), art is attached to
    bones (`bind`, `skin`), user code defines actions (`action`, `fn`, `while`, `import`, a `lib/` of
    AnimScript), in the order 0 (images), A (armature, FK, IK, skinning, clips), the action language step,
    B (ragdoll, active ragdoll, controller), C (gait and example characters). This entry covers 0 only.
140. **Syntax.** `shape.fill = image("file.png")` (or `shape.image = image(...)`) maps a PNG onto the shape's
    faces; `shape.image = none` removes it. A colour `fill` can coexist with it (the colour is drawn first).
    The path is relative to the program's folder (`run_in`, `check_in`, `load_in` and the CLI pass it; plain
    `run` uses the current folder). A missing file is `E-AST-001`, a file that is not a PNG (signature and
    `IHDR` are checked) or has a size outside 1 to 16384 is `E-AST-002`. Per-face or per-edge image fills are
    `E-TYP-001` (images are per shape for now).
141. **Texture coordinates belong to vertices.** When an image is assigned every live vertex stores
    `[u, v]` = its position in the shape's bounding box, so a later move of a vertex (soft body, skinning,
    a vertex action, keyframes) drags the picture with it; a vertex created afterwards (`subdivide`, `extrude`)
    is mapped linearly from its rest position. `shape.copy()` copies the mapping. No tiling, wrapping or
    separate UV editing yet.
142. **Snapshot.** A textured shape has `image: {src, w, h, hash, tris}`: `tris` lists triangles of
    `[x, y, u, v]` points in world space. A triangle face is one triangle, a convex quad two, and any other
    face is cut into convex pieces (the same ear clipping used for physics) and fanned. `hash` is the FNV-1a
    of the file's bytes, so changing a picture changes every frame hash while the path alone would not.
143. **Rendering.** The renderer (still a pure function of the snapshot plus the files) decodes the PNG with
    the `png` crate (a new dependency, pure Rust, justified by the need to read sprites; 16-bit and palette
    images are expanded, alpha is premultiplied), then fills each triangle with a bilinear pattern through
    the affine map of its three corners (`render_assets`, `render_png_assets`; the older `render*` use the
    current folder). Triangles are filled without anti-aliasing so shared edges leave no seams. Opacity is
    the shape's. A picture that cannot be read is drawn magenta; it never stops the render.
144. **Limits.** The picture is stretched over the bounding box (no aspect-preserving fit option yet); a
    bend inside one quad is shown as two flat triangles (subdivide for smoother curves); curved edges of a
    face are ignored by the texture (the straight vertex polygon is used); the image `tint` is not applied.

## Round 9, sub-round A: armatures, FK, IK, binding and skinning

145. **Bones are nodes.** `armature name:` is a top-level block of `bone` and `ik` lines (`bone arm: parent:
    torso, at: 0.9, length: 60, angle: 90.deg, limits: (-30.deg, 120.deg)`). A root bone has `at: (x, y)`
    (its head, px) and an absolute `angle` (clockwise from +x, as everywhere); a child has `parent`, `at` (a
    fraction along its parent, default 1 = the tip) and an `angle` *relative to the parent's direction* (default
    0, so it continues straight). `length` is positive px. Parents must be listed before children.
    `limits` are the allowed pose rotation relative to the rest direction. Armatures are built at the very
    start of the build phase (before top-level code, so shapes can `bind` to them); their expressions may
    not use program variables. Not drawn; the debug overlay shows them.
146. **Pseudo-shapes.** Each bone and each `ik` line is a hidden shape named `armature.bone` (a line from head
    to tip; invisible, kept out of the snapshot's `shapes`), reached as `armature.bone`. So `rotate_to`,
    `rotate_by`, `keyframes`, `timeline:`, `wobble`/`noise`, easing, `start`/`cancel`/`await`, the layer fold
    and scene commits work on bones unchanged. Meaning of the properties: a bone's `rotation` is its pose
    rotation relative to rest and to its parent (turns), the root bone's `position` offsets the whole
    skeleton, and an IK handle's `position` is its target relative to the rest tip and its `opacity` is the IK
    weight (0 = off). A clip is therefore just a `keyframes` block on bones; reusable named clips wait for
    user-defined actions.
147. **FK, limits.** The pose is the usual chain: a child's head is on its parent at `at`, its direction is the
    parent's plus its rest angle plus its pose rotation. A pose rotation beyond `limits` is clamped, with
    `W-RIG-001` once per bone and scene.
148. **IK.** `hand.reach((x, y)) over d ease e` moves the target and sets the weight to 1 (its starting target
    is wherever the chain's tip is), `hand.release() over d` fades the weight back to 0; blending is per bone,
    along the shortest angle, so IK and animation mix smoothly. The solver is exact for a two-bone chain (law
    of cosines; `bend: positive|negative` picks the elbow side, `positive` = clockwise of the line to the
    target, and the other side is used automatically when limits make the requested one miss) and FABRIK for
    longer chains (at most 24 passes, limits applied in each pass). An unreachable target stretches the chain
    straight at it. A chain must be connected tip to head (`E-RIG-002`).
149. **Binding.** `art.bind(bone)` attaches a whole shape to one bone: the shape's own transform is baked into
    its vertices at the start of each scene, its pivot is the bone's rest head, and each tick the rig writes
    `position` and `rotation` as layer-2 overrides (so keyframes on the art still win). `art.skin(armature,
    blend: 20, bones: [...])` makes every vertex follow several bones by linear blend skinning with automatic
    weights (inverse-exponential falloff of the distance to each bone segment relative to the nearest, `blend` px;
    weights sum to 1) and writes `v.x`/`v.y` overrides; `art.set_weight(vertices, bone, w)` paints weights by
    selection (the others are rescaled). The art must be drawn in the bind pose, i.e. matching the bones at
    rest. A shape is bound or skinned once, and cannot also be a rigid or soft body (`E-RIG-003`); `bind`/`skin`
    are build or `setup:` only (`E-SHP-003`).
150. **Output.** The snapshot has `rigs`: per armature the bones (`head`, `tip` in px, world `angle` in turns)
    and the IK targets with weights, included in the hash; absent when no armature exists.
    `--debug-physics` draws bones as green lines with a circle at the head and active IK targets as orange rings.
151. **Determinism note.** The rig maths uses `sin`, `cos`, `acos` and `atan2` (unlike the physics hot path, which
    is limited to exactly rounded operations), so results can differ in the last bits between platforms; the
    hash quantises to 1e-6, which hides it.
152. **Not yet.** Named reusable clips (they come with user actions), mirroring/flipping a rig, bone length
    animation, per-bone translation of non-root bones, and physical bones (sub-round B).

## Round 9, language step: actions, functions, `while`, `return`, `import`

153. **What is built in and what is not.** Only primitives are built in (`move_by`, `rotate_to`, `reach`, `bind`,
    `center`, `atan2`, ...). Everything higher-level, `look_to`, `hesitate`, `shake`, `pulse`, `hop` (and later
    `walk_to`), is AnimScript in `crates/lang/lib/motion.anim`, bundled into the compiler so `import "std/motion"`
    works anywhere, and also a plain file anyone can read, copy and change. The spec's table of "standard
    library actions" is therefore library code, not language.
154. **Definitions.** `action NAME(params):` takes time and runs like a built-in action; `fn NAME(params) ->
    Type:` is a pure function that returns a value. Parameters are `name: Type = default` (type and default
    optional); defaults are evaluated at each call with the earlier parameters visible. Types: `Any, Number,
    Int, Bool, String, Point (or Vec2), Angle, Duration, Color, Shape, Entity, List, Easing, Region, Vertex,
    Edge, Face, Handle`; an unknown type is `E-DEF-003`, a name defined twice `E-DEF-001`. A bone is a `Shape`.
155. **Calling.** `thing.action(args)` when the first parameter is `self`, or `action(thing, args)`; positional
    arguments fill the parameters in order and named ones by name (`E-TYP-003` for a missing, unknown or
    duplicated argument, `E-TYP-001` for a value of the wrong type). `over`/`ease` suffixes after a call are
    passed as the named arguments `over` and `ease` (so an action declares `over: Duration = ...` to accept
    them). A user definition takes precedence over a built-in of the same name.
156. **Running an action.** A call evaluates to a deferred call; as a statement it pushes the action's body onto the
    calling task (blocking it exactly like a built-in action), inside `together:` each statement is its own
    child task as before, and `start thing.action(...)` runs it as a task of its own with a handle: `await h`
    waits for the task to finish, `cancel h` stops it and the actions it was running (they keep the last value they
    sampled). An action call is not a value: storing it with `let`/assignment is `E-TYP-001`.
157. **Scoping.** A body sees the program's top-level `let` values and its own parameters and locals, not the
    caller's locals. `x = value` and `x += value` (also `-=`, `*=`, `/=`) change the innermost existing local;
    assigning to a name that was never declared is `E-NAM-001`. Assignments to top-level names from inside a
    definition change only the definition's copy.
158. **`fn`.** Runs to completion immediately in a throwaway task and returns the value of its `return`; the
    declared return type is checked. It cannot take time (`wait`, timed actions): `E-DEF-002`. Calls nest at most 64
    deep (`E-RUN-007`; the interpreter runs on a 256 MB stack so that this is safe), actions inline at most 64
    deep too.
159. **`return`.** Allowed only inside an `action` or `fn` (`E-SYN-002` at parse time); ends the innermost call.
160. **`while cond:`** with a `Bool` condition, evaluated before each pass. A loop that never blocks is stopped by
    the existing limit of 5,000,000 steps per run (`E-STM-003`); a loop with a `wait` or a timed action is fine.
161. **`uses`, `requires`, `priority`.** Accepted as the first lines of a definition. `requires EXPR` is evaluated
    at every call with the arguments bound and must be true (`E-CAP-002`). `uses` and `priority` are recorded but
    not enforced: this implementation has no named channels such as `gaze`, so `E-CAP-003` does not exist yet.
162. **`import "path.anim"` / `import "std/name"`.** Textual inclusion before parsing: the import line is
    blanked, so line numbers in the importing file never change, and the imported text is appended at the end;
    files are included once (cycles are harmless), paths are relative to the importing file's folder, and a
    missing file or library is `E-AST-001`. An imported file should contain only definitions (it is parsed as
    part of the program, so its errors report line numbers after the end of your file).
163. **New builtin and the limits.** `atan2(y, x)` returns an Angle. Named reusable clips, generics and closures are
    not implemented. (The earlier limit that expressions only see rest values is lifted by 164.)
164. **Live reads.** Expressions see the state the layer stack resolved on the *previous* tick: `shape.position`,
    `.scale`, `.rotation` and `center(shape)` return the live transform (so an action can track a moving target
    with `while`/`wait 1.frame`), and a read is at most one frame behind. `center(shape)` is the world centre,
    i.e. the shape's own transform applied to its bounding-box centre. Bone pseudo-shapes expose `head`, `tip`
    (world px) and `angle` (Angle) from the last solved pose, or the rest pose before the first tick. Live state is
    cleared at the end of every scene, so a new scene reads rest values again. Only transforms are live; colours,
    opacity and vertex positions still read rest values.
165. **Engine additions for sub-round B.** `World::raycast(origin, dir, max, exclude)` returns the nearest solid
    hit (body, point, outward normal, distance); sensors are ignored and a ray that starts inside a shape does not
    hit it. `set_body_kind` keeps pose and velocity and recomputes mass; `disable_collision(a, b)` filters a pair.
    A revolute joint can carry an *angle spring* (`JointDef::angle_spring`, `set_joint_angle_target`,
    `set_joint_angle_spring`): a soft constraint on the angle relative to the rest angle, solved in the biased pass
    like the distance spring, with accumulated impulse clamped to `max_torque * h`, and switchable at runtime.
166. **Character controller.** `body.controller(speed: 200, accel: 1200, jump: 400, air_control: 0.5)` on a dynamic
    body makes it `fixed_rotation` with friction 0 (so it neither tips nor sticks to walls). Every tick, before the
    step, three rays from the centre of mass (centre and +-90% of the half width) go down for the distance to the
    feet plus 4 px; grounded means any hit with an upward-facing normal (`normal.y < -0.5`). The horizontal
    velocity moves towards `input * speed` by at most `accel * dt` (times `air_control` in the air); a pending
    jump sets the vertical velocity to `-jump` only if grounded and is dropped otherwise (no buffering, no
    double jump). There is no step-up: slopes and ledges are handled by the contact solver. `move`/`jump` are Run
    only; the declaration is build/setup only. A controller on a non-dynamic body or with a bad value is the new
    `E-PHY-005`, as is `move`/`jump` on a shape without one.
167. **Ragdoll declaration.** `ragdoll: radius: 8, density: 1, friction: 0.6, restitution: 0` is a nameless
    armature line (one per armature); a bone's own `radius:` overrides it. Each bone becomes a kinematic body at
    the rest pose with a box (half length x radius) and a circle at each end; each non-root bone gets a revolute
    joint to its parent at its own head, carrying the bone's `limits` (relative to the rest pose, the same
    reference as the pose rotation). All bones of an armature have collisions with each other disabled (not only
    neighbours). Bone bodies take the names `armature.bone` in the physics name table, so `touches` and
    `leaves` work for bones; they are not in `kinds`/`com`, so a bone is never written back as a shape pose.
168. **Ragdoll states.** Every scene starts animated: kinematic bodies moved to the animated pose each tick with
    `move_kinematic` (teleported at tick 0). `ragdoll(strength, push)` switches the bodies to dynamic, keeping their
    velocity (so a moving animation throws the body), adds `push` px/s to every bone and, at strength 0, switches
    all muscles off. `animate()` makes them kinematic again with zero velocity and the next tick snaps them to
    the animated pose (no blending back). The state is per scene and is not carried over. Both are Run-only; with
    no `ragdoll:` line, outside a running scene or with a `strength` outside 0..1 they are `E-RIG-004`.
169. **Active ragdoll.** With strength s > 0 each non-root hinge's angle spring is aimed every tick at the animated
    pose rotation of that bone (after limit clamping and IK) with frequency `3 + 9 s` Hz, damping ratio 0.7 and
    max torque `s * 8 * g * m * r`, where m is the mass of the bone and its descendants and r the farthest
    distance from the hinge to a descendant's tip. The root has no hinge, so its linear and angular velocity are
    blended by `s` towards the velocity that would reach the animated placement this tick: s = 1 follows the
    animation closely, small s sags and falls. This is a heuristic, not a balance controller: a standing pose
    that needs foot placement is gait work (sub-round C).
170. **Order of the tick.** The animated pose of every armature (FK, limits, IK) is solved first
    (`rig_solve`), then the physics step runs with ragdoll bodies targeting it (`ragdoll_prepare`), then physical
    armatures' poses are read back from the bodies (`ragdoll_poses`) and `rig_finish` writes the snapshot, the
    live bone values and the art overrides. A physical bone's angle is kept within half a turn of the animated
    angle (so it is continuous when the animation is); only its placement is the physics'.
171. **Limits recorded.** A shape cannot be both rig-driven art and a rigid body (still `E-RIG-003`); fluids and
    soft bodies do not see bone bodies as anything special (they collide with them as with any rigid body).
    Torque and frequency in 169 are tuned by eye; no per-bone strength yet.
172. **`raycast`.** A builtin over `World::raycast_filtered` (the unfiltered `raycast` now calls it). Arguments are
    pixels in the language and metres in the engine; `direction` need not be unit length. It returns a point or
    `none`; the normal is a separate builtin `raycast_normal` because there are no record values yet. No world
    (not running a scene) gives `none`, not an error. `ignore:` accepts a shape, an armature (all its bone
    bodies), the string `"bones"` (all armatures) or a list; anything else, a zero direction or a negative length
    is the new `E-PHY-006`. Static bodies are hit like any other solid.
173. **Armature type and IK reads.** `Armature` is a parameter type (so `self: Armature`). IK handles are shapes, so
    `self: Shape` already covered them. `handle.target` and `handle.weight` are the last solved values (the rest
    tip and 0 before the first tick); like other live reads they are one frame behind and cleared per scene.
174. **Gait is a library, not an engine feature.** `std/gait` is cadence driven: every foot loops "swing for n
    frames, stay planted n frames", offset by half a cycle between groups, so a biped has exactly one foot in
    the air and a quadruped swings diagonal pairs. Landing x is where the hips will be when the foot lands plus
    half of the travel during the stance, clamped so it never overshoots the destination. The ground under that
    point is found with a downward ray from 100 px above the hips, so stairs and ramps work; there is no
    adaptation if the ground is missing (the foot keeps its height). Hips follow the average foot height with a
    25% per frame smoothing, so they bob with the swing. Stopping is by distance: each foot loop ends when the hips
    are within one frame's travel of the target, then each foot steps home, then the hips straighten. Phase
    conventions and bone names are in DOCUMENT 13i. There is no gap-crossing or jumping yet.
175. **Language limits met while writing it.** Lists are immutable (no `xs[i] = v`), so per-foot state lives in
    separate tasks (`together:`), not in a list; durations are used as frame counts (`frames(d)`); there is no
    conditional expression, so direction is set with `let d = 1` and `if`.
176. **`race:`** is `together:` with a different wake rule: the parent task wakes when the first child task finishes,
    the other children (and everything they started) are cancelled through the same path as `cancel`, so their
    actions keep the value they last sampled. Children are the statements of the block, like `together:`; a tie goes
    to the lowest task id, i.e. the one written first.
177. **`finally:`.** A statement `Finally(block)` that must be the last of its block (`E-SYN-001` otherwise, and it is
    refused directly inside `together:`/`race:`). Running off the end of the block executes it; cancelling a task
    (cancel, a lost race, scene end) runs the `finally:` of every block of the task's frame stack that has not
    reached it yet, innermost first; `return` runs those of the blocks it leaves. All of them run in a throwaway
    task with a flattened copy of the task's variables and must not block (`E-STM-003`). Scene end cancels the tasks
    that are still alive and applies what their `finally:` blocks staged. Errors raised while cancelling are held
    and raised at the end of the step. `finally:` is allowed anywhere a block is, including user actions (my lean).
178. **`derive`.** Parsed inside an entity; its condition is evaluated with `self` and the entity's flag names bound as
    variables. Derived values live in the entity's flag map (so snapshots and rules see them) and are recomputed at the
    start of every delta, after the authored flags were noted, in dependency order (depth-first over the references
    found in the conditions), so a rule sees this tick's value. `E-STA-002` reports the cycle path. History per flag
    (`since`: tick it took its value; `rose`: tick and delta of the last rising edge) is kept for every flag and reset
    at each scene start. `held_for(d)` is `now - since >= frames(d)`; `since(d)` is `now - since <= frames(d)`; `rose`
    compares with the current tick and delta. These three are recognised syntactically (the receiver must name a flag).
179. **`cubic_bezier`.** `Ease::Bezier([x1, y1, x2, y2])`: Newton iterations on x(t) = p with a bisection fallback, then
    y(t); the ends are exact. x1 and x2 outside 0..1 is `E-TYP-001`.
180. **Markers.** `marker "name" at TIME` lines inside `timeline:` (time in absolute scene time). The runtime inserts the
    event `marker:name` on that tick before the first delta; `marker("name")` reads it, so it works as a trigger like
    `scene.starts`. Only in `timeline:` for now (shots may declare them later).
181. **Locks.** `lock TARGET = VALUE [for D]` uses the same target and value machinery as timeline keys (so `position`,
    `position.x`, `rotation`, `scale`, `opacity`, colours, vertex coordinates and bones work) and registers one
    layer-4 Override of weight 1 per scalar, constant, from the tick of the statement until `D` later or `release`.
    Overlap of two locks on one scalar is `E-PRP-002`, raised when the second is made. Locks are folded with
    everything else, so they also beat physics, rigs and procedural results; `lock`, `attach`, `constrain` and
    `release` outside a running scene are `E-STM-001`.
182. **Handles.** `lock`, `attach` and `constrain` return a handle like `start` does (`let h = ...`); `release h` and
    `cancel h` remove it. Releasing an attachment does not move the child: it simply stops following, so it is drawn
    at its own placement again.
183. **`attach`.** Applied after the fold on the resolved transforms. Parent frame = (pivot world position + position,
    rotation) for a shape, (head, direction) for a bone; the child receives the rigid motion since the frame the parent
    had when attached: rotation about that earlier anchor point plus the anchor's displacement, which is exact for the
    transform model (scale about the pivot, rotate, translate). Parent scale is ignored. Depth decides the order
    (parents before children, then creation order). Cycles, including through several attachments, are `E-CON-001`.
    `offset (x, y)` moves the child's *rest* position so its centre lands on the anchor plus the offset rotated by
    the parent's rotation; it is the only part that changes rest state.
184. **Constraints.** `inside RECT` clamps the transformed bounding box (the resolved vertices, if any) into the
    rectangle, centring it when it is larger; `canvas` is a built-in rectangle (0, 0, width, height). `follow(target,
    lag: N)` keeps a history of the target's anchor, one entry per tick starting when the constraint is made (an
    unknown past is the oldest entry), and puts the shape at that point plus the offset the two had when the
    constraint was made. `weight` blends toward the unconstrained placement. `constrain X.position ...` and
    `constrain X ...` are the same. Other forms in the spec (`gaze`) have no meaning for shapes and are not parsed.
185. **Named channels.** The first statement of an action with a `uses` line is a synthesized `Acquire`. It takes
    `(self shape, channel)` for each name at once, tagged with the call's id; the holder is alive while its task has
    that call on its frame stack, so channels are freed when the call ends, is cancelled or returns, without any
    explicit release. A task never conflicts with itself, so an action may call another that uses the same
    channel. Conflict policies as in spec 6.4; waiting requests are served highest priority first, then in order,
    after every step. Interrupt cancels the whole holder task (the spec's default) through the usual cancel path
    (values kept, `finally:` run), and a victim that is also an ancestor is not special-cased. `share` neither
    waits nor holds the channel. Native actions keep their per-vertex channels. Priority of an action: its
    `priority` line if non-zero, else the starting task's.
186. **`E-CAP-003`.** Checked when a native shape action is issued: if the innermost action call has a non-empty
    `uses` that lacks `body`, it is `E-CAP-003`. Actions without `uses` are unchecked, so existing code is unaffected.
    The std libraries only declare `body`.
187. **Note on `move_to`.** On a shape, `move_to` and `move_by` change the `position` offset, not the centre
    (DOCUMENT section 8.1's `follow` example is therefore exact only for shapes whose rest centre is the origin;
    use `p - center(self)` to aim a centre). Found while writing the links example.
188. **`requires` is `E-CAP-001`.** A `requires` that is false (and any action whose receiver does not fit) is a missing
    capability, `E-CAP-001`, as spec 8.3 says; it used to be `E-CAP-002` (decision 154), which the spec keeps for
    ambiguity. The existing test and DOCUMENT were updated.
189. **Overloading by `self` type.** `method_defs` keeps every definition with a `self` parameter by name. One candidate:
    called as before (a type mismatch is `E-TYP-001`). Several: those whose `self` type accepts the receiver are
    candidates; a named type has rank 2, `Any` or untyped rank 1; the highest rank wins and a tie would be `E-CAP-002`
    (it cannot happen yet because two definitions with the same, or an untyped, `self` are refused at registration as
    `E-DEF-001`, as is a method and a plain function of one name). No fit is `E-CAP-001` listing the types. Overloading
    on other parameters is not supported.
190. **`clip` is sugar for an action.** `clip name(params) over D [uses ...]:` plus `at T:` blocks is lowered by the
    parser into `action name(params, speed: Number = 1, loop: Bool = false)` with `requires speed > 0`, the `Acquire` of
    its channels (default `body`), one `keyframes over D / speed` with every key time divided by `speed`, and a
    `while loop:` that plays the keys again. For `self: Armature` each key's first path component names a bone: the keys
    are grouped per bone and one `self.bone.keyframes` per bone runs in a `together:`. `self` must be a `Shape` or
    an `Armature`; a clip needs `over` and at least one key. Clips are not loaded from assets (this project has no
    asset system for animation data). `play_clip("name", ...)` finds the clips of that name, picks by `self` like any
    call, and passes `speed`/`loop`/parameters through.
191. **`std/2d`.** Shape versions of the spec's character actions. Because a shape has no gaze or rig, `look_at` turns
    the shape (with `facing`), `step_back` and `walk_to` work on the centre, and targets are points. `fly_away` and
    `fly_to` move frame by frame with a seeded sway from `noise(self.name, ...)`, so they are deterministic and differ
    between shapes. Directions are built-in names (like `canvas`), resolved after user names. The camera actions
    (`hold`, `pan_to`, `zoom_to`, `follow`, `shake`) are for the camera round, and `std/motion` is unchanged.
192. **The camera is a pseudo-shape.** A hidden shape named `camera` (made with the other shapes, so the name is taken)
    whose transform is the camera: `position` is stored as an offset from the canvas centre, `scale` is the zoom and
    `rotation` the roll, so actions, keyframes, modifiers, locks and the layer fold all work. At the edges the
    absolute convention is kept: `move_to`, assignments and keys convert a position to the offset (`cam_off`),
    `camera.position` reads the centre, `camera.zoom` and `camera.rotation` read the others. Zoom set by a timed or
    instant action outside [1, 8] is `E-CAM-001`; the value written to the snapshot is always clamped, so keys and
    modifiers cannot break the picture. Position, zoom and rotation actions use three separate native channels, so a
    single user action can run them together; the user-visible `frame` channel makes separate actions take turns.
193. **Camera actions are library code.** `std/2d` defines `pan_to`, `zoom_to`, `hold`, `follow`, `shake`, `frame` for
    `self: Camera` (a new type that only the camera satisfies). `follow` smooths: every frame it closes the gap to the
    target by `1 / lag` (lag 8.frames is a time constant, not a delayed copy), so it never overshoots. `shake` is the
    seeded `noise` modifier on position and shares the channel. `frame(size, on:, thirds:, over:)`: zoom is `size *
    canvas height / bounds height` clamped to 1..8 (unchanged for a point), the centre is the target's anchor moved
    by a sixth of the view per third, `establishing` (size 0) is the whole canvas, and `over` zero is instant.
    New built-ins: `bounds` (a rectangle with members x y w h), `anchor`, the five size names, and the camera
    properties above. A zero `over` is instant in `pan_to` and `zoom_to` too.
194. **Shots.** `shots:` holds `shot "name" for D | until TRIGGER | auto:` items with `cut`, `dissolve D` and
    `fade_through COLOR D` between them (no item between two shots is a cut). The runtime starts the first shot in the
    first delta of the scene, and the shot's body as its own task; the shot's end is checked at the start of every
    delta (`for`: the frames since it began; `until`: the trigger's rising edge; `auto`: the body task is gone).
    Ending cancels the body task and the rule tasks of that shot, then the next shot starts in the same delta (so
    several zero-length shots chain), with any transition recorded first. After the last shot the camera stays as it
    was and the shot name is cleared. `when`/`whenever` inside a shot belong to it: their memory is reset each time
    the shot is current, they do not fire otherwise, and a trigger that is already true when the shot begins fires.
195. **Transitions.** The runtime keeps the camera view at the moment the shots changed (`from`), the kind, colour and
    length in frames; the snapshot reports `t` as progress in 0..1 and drops the object when it is over. The new
    shot runs from the first frame of the transition (spec: the dissolve is blocking and blends both views). The
    renderer draws the same world twice, once per camera, and mixes the pictures pixel by pixel: a dissolve linearly,
    a `fade_through` as old view to colour in the first half and colour to new view in the second.
196. **Rendering with a camera.** All drawing goes through one `View` (output scale after zoom, rotation and
    centring): shapes, pictures, particles, the physics overlay and the fluid and foam drawing (which works in
    pixels and maps centres and radii with it), so vector art stays sharp when zoomed; stroke widths scale with the
    zoom. A positive rotation turns the world anticlockwise on the picture (the camera rolls clockwise).
197. **`scene.ends_in(d)`** is true when the tick is at or past `length - frames(d)` of a scene with a set length; it is
    always false for `for auto`.
198. **Brush engine = real libmypaint (B1).** libmypaint (commit in `third_party/libmypaint/COMMIT`, ISC licence) is
    vendored with only the brush engine and the fixed-size tiled surface, compiled by the `cc` crate in the new
    `animscript-brush` crate (the one new build dependency, plus a C compiler). No glib, json-c, GEGL or gettext:
    `config.h` is new, the two settings headers are generated by upstream's `generate.py`, the json-c brush loader is
    compiled out, and two functions are added to the fixed surface (`clear`, `read`). Every change is listed in
    `third_party/libmypaint/README.animscript`. `.myb` files are read in Rust (serde_json) and applied through
    `mypaint_brush_set_base_value` / `_set_mapping_*`; settings this libmypaint does not know are skipped.
199. **Determinism of the brush.** The build passes `-ffp-contract=off -fno-fast-math` so the same C arithmetic runs
    everywhere, but libmypaint uses `float` and the C maths library, so the same stroke is exactly reproducible on one
    machine and may differ in the last bit (an 8-bit value at worst) across platforms or libm versions. libmypaint's
    random generator carries on from stroke to stroke inside a `Brush`, so a stroke is repeated exactly by using a fresh
    `Brush` (the renderer will do this for every stroke each frame). Tests compare pictures from the same process only.
200. **Brush API.** `Brush` (libmypaint settings; `from_myb`, `bundled`, `set`/`get` by libmypaint name, `set_radius`,
    `set_opacity`, `set_color`), `Point {x, y, pressure, xtilt, ytilt, dt}`, `Canvas` (a transparent premultiplied
    RGBA picture; `stroke` begins with a zero-pressure sample and ends with one, as libmypaint expects; `rgba8`
    converts 15-bit pixels). Nine MyPaint classic brushes (CC0, `Licenses.dep5` kept beside them) are bundled:
    pencil, pen, marker, charcoal, calligraphy, ink, smudge, eraser, rounded.
201. **Paint layers are shapes (B2).** `paint_layer(z:, resolution:)` returns a geometry-less shape (core `Shape.paint`
    holds the canvas size and resolution); registering it as `shape name = ...` creates its stroke list and puts its
    pivot at the canvas centre. Position, scale, rotation, opacity, z and visibility are the shape's, folded by the
    usual layers; the snapshot entry has no vertices and a `paint` object (`size`, `resolution`, `brushes`, `strokes`).
202. **Strokes are data, progress is a property.** A stroke is a brush index, points `[x, y, pressure, dt]` and a
    progress. `PropId::StrokeProgress(i)` makes a timed stroke an ordinary action (`over`, `ease`, `cancel`, keyframes,
    locks all work); the default easing of a stroke is linear. A stroke made with `over` while the scene runs starts at
    0, any other at 1. Every stroke has its own channel (`0x4000_0000 + index`) so strokes can be drawn together;
    statements in sequence draw one after another. Strokes are made when the statement runs, not when it starts waiting.
203. **Brush values.** `brush(source, name: value, ...)` validates at once: a bundled name is built, a `.myb` path is
    read relative to the program's folder (its text is kept in the value so the snapshot does not depend on the file),
    and every setting is tried on a probe brush (`E-BRU-001` for unknown names, bad types, `radius <= 0`).
    `radius` is converted to libmypaint's logarithm, `opacity` is the name of `opaque`, booleans are 0 and 1. A layer
    stores each distinct brush once.
204. **Points and time.** Without a fourth number the time since the previous point is the distance over `speed` (min 1 ms)
    and the first point's is 10 ms; this is what libmypaint's speed inputs and spacing see, so the same path drawn with
    another `speed` looks different. Pressure outside 0..1 or a negative time is `E-BRU-002`.
205. **Replay rendering.** The renderer builds a `Canvas` of `size * resolution` pixels per layer and frame, replays
    each stroke with a fresh `Brush` (so the random numbers restart and a frame is a pure function of the snapshot),
    truncates a stroke to its progress along the path length (the last point interpolated), scales points and
    `radius_logarithmic` by the resolution, and draws the picture with bilinear filtering through scale/rotate/translate
    about the pivot, then the camera. Cost is proportional to the number of dabs per frame, with no caching yet.
206. **Not yet.** Per-stroke blend of the symmetric copies separately, and brush-level modes other than libmypaint's own.
207. **A stroke starts cleanly.** libmypaint filters the cursor towards its previous position (`slow_tracking`) before it
    notices that a stroke has started, so a brush with tracking drew a tail sliding in from the last position (or the
    origin). `Canvas::stroke` now gives the first, zero-pressure sample a time step above libmypaint's 5 s limit, which
    makes it start exactly at the first point. A regression test covers it.
208. **Curve fitting (B3).** `animscript_core::fit_curve(points, tolerance, corner_deg, closed)` is Schneider's algorithm:
    chord-length parameters, least-squares control points for the end tangents, Newton-Raphson reparametrisation, and a
    split at the worst point with a shared tangent. Changes from the textbook: repeated points are dropped; a closed
    path starts at its first corner (or, if smooth, joins itself with one shared tangent); acceptance also checks the
    *true* distance from every point to the curve (nearest sample refined by ternary search), not only the distance at
    the sample parameters, so the tolerance holds; tangents are the sum of the vectors to the points within `2 *
    tolerance` of path (not just the next point) and the split tangent likewise; control arms longer than the chord
    are replaced by thirds. Everything is plain f64 arithmetic, so results are deterministic.
209. **Corners and smoothing.** A corner is a point where the path turns by more than `corner` between points
    `max(4 * tolerance, 4)` px away on each side; the sharpest of a group closer than that along the path is kept.
    Before fitting, the path is smoothed with a triangular moving average (window 0.7, 0.4 or 0.2 times the
    tolerance, the first that moves no point more than half the tolerance), and the distance moved is subtracted from
    the tolerance, so the curve stays within `tolerance` of the points that were given. Noise much larger than the
    tolerance still makes many small pieces: the tolerance should be about the size of the tremor.
210. **Language functions.** `freehand` builds a shape with `bezier_shape` (v1.. in path order, cubic edges stored in the
    chord frame as the shape model wants, a face when closed); `smooth` samples the fitted pieces and resamples them;
    `simplify`, `resample`, `path_length` wrap the core helpers; `points_along` and `outline` sample edges (straight,
    quadratic and cubic) densely and resample by arc length with the shape's current transform applied; `jitter` adds
    seeded value noise (one stream per axis, advancing with path length / `scale`) so strokes of the same seed repeat.
    Argument problems are `E-FRE-001`.
211. **Tilt.** A point may carry `xtilt, ytilt` (-1 to 1) as its 5th and 6th numbers (a point has 2, 3, 4 or 6 numbers;
    five is an error); a stroke's `tilt: (x, y)` fills in the points that have none, and `tilt: "follow"` sets
    `0.6 * (cos, sin)` of the local direction of travel (the line between the neighbouring points) for them. `barrel:` is
    in turns, as libmypaint's argument. The snapshot's point arrays have six numbers; the renderer reads missing ones as 0.
212. **Symmetry.** Uses libmypaint's own symmetry on the tiled surface (`set_symmetry_state`, applied at `begin_atomic`, cleared
    after each stroke). Kinds `vertical`, `horizontal`, `both`, `radial`, `snowflake`; `lines` 2 to 64; `angle` in
    degrees (the language takes an Angle); `center` defaults to the canvas centre and is multiplied by the layer's
    resolution when rendering. It is stored on each stroke: `layer.symmetry(...)` sets the default for strokes made
    after it, `none` clears it, and `symmetry: none` on one stroke overrides the default.
213. **Blend modes.** `Canvas::stroke_with` paints a non-`normal` or partly strong stroke on a transparent canvas of
    the same size, then composites it onto the picture on premultiplied 15-bit pixels with `co = cs (1 - ab) + cb (1 - as)
    + as ab B(cb, cs)`, `ao = as + ab - as ab` (`strength` scales the source alpha and colour first), and writes the result
    back (`mypaint_fixed_tiled_surface_write`, a new vendored accessor). Separable W3C modes: multiply, screen, overlay,
    darken, lighten, color dodge and burn, hard and soft light, difference, exclusion, plus `add`. A smudge brush in a blended
    stroke sees only its own canvas (documented). `art.blend` maps the same names to tiny-skia blend modes when the
    layer is drawn onto the scene (`add` is `Plus`).
214. **Errors.** `E-BRU-002` for bad points and tilt numbers, new `E-BRU-003` for bad tilt style, symmetry or blend arguments,
    unknown symmetry kinds and blend modes, `lines` and `strength` out of range.


## Mathematical Animation Engine, M1 (IR, interpreter, derivatives, parser)

215. **ExprTk is a test oracle only.** It is vendored under `third_party/exprtk` and compiled only with the `exprtk` feature of `animscript-math`; it never runs in the product. The differential test (`tests/oracle.rs`) compares the interpreter with it to 1e-9.
216. **The IR is scalar `f64` SSA**, hash-consed, in `animscript-math`. Vectors are expanded by the frontend. Inputs have a rate: `Point` (varies per sample) or `Frame` (one value per evaluation); nodes depending only on frame inputs, params and constants are computed once per batch.
217. **Determinism:** every transcendental goes through the `libm` crate (never the platform libm), so results are bit-identical across machines; `noise`/`hash` use an integer lattice hash on float bits and smoothstep interpolation, with no global state.
218. **Simplification is conservative:** constant folding plus only the exact identities (`x*1`, `x/1`, `x-0`, `pow(x,1)`); `x*0` is NOT folded (x may be infinite or NaN). Commutative operands are ordered by id so `a+b` and `b+a` share a node. Dead code is removed in `finish`.
219. **Derivatives are symbolic** (`Builder::diff`). Step-like ops (floor, ceil, round, trunc, sign, comparisons) have derivative 0; `min`/`max`/`select` differentiate the branch taken; `noise` is an error. A term whose factor derivative is a zero constant is dropped.
220. **String syntax:** `^` (and `**`) is right associative and binds tighter than unary minus (`-2^2 = -4`); comparisons and `&&`/`||` yield 0 or 1; `c ? a : b` and `if(c,a,b)` are the same node; constants `pi tau e true false`; `ln` = `log`; `frac` = `fract`; `%` and `mod` are fmod.

## Mathematical Animation Engine, M2 (language integration)

221. **Surface.** `plot`, `curve`, `polar` and `implicit` build shapes once; `drive` is a layer-2 modifier (`ModKind::Drive`), so keyframes (layer 3) and locks (layer 4) still win. Equations are a string or a lambda; arguments use the language's `name: value` form, and `over` / `start` work as for `wobble` (an open-ended drive without either is `E-TYP-003`).
222. **Lambda lowering.** A lambda's parameters are the equation's inputs. Only arithmetic, comparisons, `and`/`or`/`not` and the shared function table (`animscript_math::apply_fn`) are lowered; a sub-expression that mentions no input is evaluated by the interpreter first (so captured variables, units and member reads are constants). Anything else is `E-MTH-002`. There is no `^` in the language, so lambdas use `pow`. There is no inline `if`; `select(c, a, b)` is the conditional in both forms (the string form also has `c ? a : b` and `if(c, a, b)`).
223. **Units in equations.** Plain radians and seconds: `sin` takes radians (unlike the language's angle `sin`), a duration passed as a named number is seconds, an angle is radians. `rotation` as a driven shape property is in turns (it adds to the shape's rotation, which is in turns).
224. **Named numbers** are any extra named argument with a number, duration, angle or bool value; any other extra name is `E-MTH-004` (this catches typos in property names for strings). Names equal to an input (`x y i u t n`) are `E-MTH-003`.
225. **Drive inputs.** `x`, `y` are the vertex's rest position at each tick (so an action that moves the rest value moves the wave's sampling too), not its position at the drive start. For a shape-level drive they are the shape's rest centre in the scene. `i` is the index in vertex-id order among live vertices, `u = i / (n - 1)` (0 for one vertex), `t` is seconds since the drive began times `speed`, `n` the vertex count (1 for shape-level drives).
226. **Add or replace.** Default is additive. `mode: "replace"` still contributes additively: the offset is `value - rest`, evaluated each tick.
227. **Non-finite results** at run time contribute 0 (or the rest value, in replace mode) and add one `MATH` trace line per drive; at build time (`plot`, `curve`, `polar`, `implicit`) they are `E-MTH-005`. A drive cannot mix vertex (`x`, `y`) and shape properties (`E-MTH-004`), and shape properties cannot be driven on a vertex selection.
228. **Sampling.** With `closed: true` the samples are `from + (to - from) * k / n` (the end value is not repeated), otherwise `k / (n - 1)`. `polar` defaults to 0..tau and closed; `plot` and `curve` default to 0..1 and open. `implicit` uses `f < 0` as inside, the saddle cells are resolved by the sign of the average of the four corners, and crossings are shared between cells, so contours are connected; cells are scanned row by row, so the vertex order is fixed.
229. **`std/waves`** is AnimScript on top of `drive` (`swell`, `ripples`, `choppy`, `float_on`). `float_on` is visual only (a shape-level drive of `pos_y` and `rotation` using the surface equation at the shape's own x); buoyancy in the physics engine is not part of M2. `choppy` and `float_on` do not match exactly, since `choppy` also moves vertices sideways; pair `float_on` with `swell`.
230. **Determinism.** The program is compiled once per call from the text; noise is seeded from the script seed and the drive's stream name (`scene/shape/drive#k`), so the same script gives the same hashes on every run.

## Mathematical Animation Engine, M3 (Cranelift JIT)

231. **Gate.** Before the JIT existed I measured the interpreter against hand-fused native Rust (an upper bound), then against the real JIT. At 10k points the JIT was 3.0x (swell), 3.2x (ripple with noise), 34x (a polynomial) and 1.4x (trig-heavy) faster than the interpreter, a geometric mean of about 4.7x against the agreed bar of 2x. Compiling an equation costs about 0.5 ms. `crates/math/examples/bench.rs` reproduces this (`cargo run --release -p animscript-math --features jit --example bench`).
232. **Bit-identical by construction.** Native instructions only where IEEE 754 fixes the result (`+ - * /`, negation, `abs`, `sqrt`, `floor`, `ceil`, `trunc`, comparisons, selects, the integer hash). Everything else (`libm` functions, `round`, `noise`) is a call to a thunk that runs the same `un_apply` / `bin_apply` / `noise_apply` as the interpreter. No FMA, no reassociation. `round` is a thunk because Cranelift's `nearest` is ties-to-even while Rust's `round` is half away from zero.
233. **NaN contract.** Results are identical bit for bit except that a NaN may differ in sign and payload (it is a NaN in both). `drive` treats every non-finite value as 0, so this never shows in a snapshot.
234. **Opt-in.** `jit` is a feature of `animscript-math`, `animscript-lang` and `anim`, off by default; the default build has no Cranelift. Cranelift is pinned to an exact version (`=0.137.1`; its API changes between releases). `anim --jit auto|always|off` and `RunOptions::jit` choose; `auto` compiles drives over at least 128 vertices (below that, a compile never pays back). Shape-level drives and fewer points use the interpreter in `auto`. If compiling fails (no executable memory, unsupported CPU) the interpreter is used and a `JIT` trace line says so.
235. **Where it fuses.** The generated function computes the nodes that depend on no per-point input once before its loop, as the interpreter does, then one scalar loop over the points keeps every other node in registers. There is no SIMD: the transcendental functions are scalar calls.
236. **Tests.** `crates/math/tests/jit.rs` compares every unary and binary operation, `select` and `noise` (1 to 3 dimensions) on 31 adversarial values (signed zeros, infinities, NaN, subnormals, 2^53+1, rounding ties, huge exponents), 600 seeded random programs (including derivatives), sizes 0 to 1001, uniform-only programs, argument errors, and parsed equations. `crates/lang/tests/math_jit.rs` runs scripts and `examples/water.anim` with the JIT off, auto and always and requires identical snapshots and hashes.
237. **A bug the fuzz test found.** `noise` at huge coordinates (`1e300`) overflowed `i64` while adding the lattice offset: a panic in a debug build, a silent wrap in release. Fixed with wrapping arithmetic (the release behaviour), with a test.

## The Candle example and the paint cache

238. **Paint cache.** `crates/render` keeps, per thread, the picture of each paint layer keyed by a hash of its size, resolution, brushes and strokes (with their progress), at most 48 pictures. A layer that only changes transform, opacity, blend or z is drawn from the cached picture; changed strokes or progress miss the cache and are replayed. Tests: identical pixels on a repeat render, a moved layer lands where it should, changed progress is never served stale.
239. **A puppet without `attach`.** The example computes every layer's pose itself (forward kinematics, a two-bone IK solve, springs for follow-through) and keys the result in the scene timeline. Layers of one group share a pivot, so one rotation and offset series drives the whole group; the forearm's absolute direction is keyed (not relative to the upper arm) so no hierarchy is needed. Reason: `attach` applies the parent's motion from the moment of attaching, which would have to be offset by the entrance.
240. **Equation flames.** The flame outline is a closed `curve` (two sides of one parametric teardrop selected with `select`) and `drive` runs in `mode: "replace"` on its vertices with `x`, `y` and `t`. The ignition is an envelope inside the equation, not an animated scale, so the base stays on the wick.

## Mathematical Animation Engine, M4 (fields)

241. **Declaration.** `field name:` is a block of `key: expression` lines like `emitter`. Equations are `("channel", "equation")` pairs (strings or lambdas); `step:` is a list where consecutive pairs form one phase and a nested list is a phase by itself. Any other name with a number (or duration, angle, bool) is a constant of the equations; another kind of value is `E-FLD-001`. Fields are built before emitters, so an emitter's `fields:` can name them.
242. **Stepping.** Not at tick 0 (the first snapshot shows the initial state). Each tick runs `substeps` steps of `dt = 1 / (fps * substeps)`; `t` is the time at the end of each substep. Inside a phase every equation reads the old state and the results are committed together; phases run in order. Double buffered, one thread, row-major: the result depends on nothing but the script (and the machine-code path is bit-identical, tested).
243. **Edges.** `clamp`, `wrap`, `mirror` (a cell beyond the edge reads the border cell itself, and reflection continues for larger offsets) and `zero`. Reading another row in a one-row field is `E-FLD-001`.
244. **Limits and errors.** At most 512 cells on a side and 512 x 512 in all, 1 to 64 substeps (`E-FLD-002`). A non-finite value after any phase is `E-FLD-003` naming the channel, cell and tick; the run stops there.
245. **Stencils in equations.** `name[dx, dy]` in strings and `name(dx, dy)` in lambdas (offsets at most 16, whole numbers) become the per-cell inputs `name@dx,dy`; the engine builds shifted copies of the channel with the edge rule applied. Only channels can be indexed (`Env::stencils` in `animscript-math`).
246. **Snapshot.** Per field: name, `nx`, `ny`, `rect`, `z`, `opacity`, `blend`, `channels`, per-channel `stats` (sum, min, max), a 16 hex digit FNV-1a `checksum` over the bits of every number, and, with a `ramp` on a grid, `iw`, `ih` and `rgba` (base64, at most 256 cells on a side, sampled by nearest cell). `raw: true` adds `data`. Fields are drawn by the renderer in `z` order with the shapes (a field before a shape of the same z), stretched over `rect` with bilinear filtering.
247. **Coupling.** `drive(..., fields: [f])` adds the inputs `<field>_<channel>` for every channel and `<field>` for the first. The field is sampled bilinearly at each vertex's rest position (a chain field: linearly along x between its cells; a shape-level drive: at the shape's rest centre). Emitters take `force_x`, `force_y` (px / s^2) over `x, y, vx, vy, age, i, t` plus the named fields; the force is added to each particle's velocity before every substep of the particle integrator, and a non-finite force is ignored for that particle.
248. **Examples.** `pond.anim` (wave equation with rain, a boat and leaves coupled to the surface, reeds by equation) and `turing.anim` (Gray-Scott). Both render in about a minute or two at 1280x720.

## M4 part A: the LLVM spike

249. **Result of the gate.** `animscript-math` has an optional `llvm` feature (inkwell 0.11, LLVM 18, the O3 pipeline for the host CPU, no fast-math flags) that builds the same kernel as the Cranelift one. It is bit-identical to the interpreter: the whole parity battery of `tests/jit.rs` runs against it (`tests/llvm.rs`). Speed against Cranelift at 10k and 100k points: the libm-free polynomial is 1.8x faster (LLVM vectorizes the loop: 7.3 us against 13.2 us at 10k, 81 us against 147 us at 100k), the wave (swell), the ripple with noise and the trig-heavy equation are 0.98x to 1.04x (the time is in the same `libm` thunks, which no backend can vectorize without giving up the bit-identical results). Compiling takes 6 to 10 ms against 0.5 ms.
250. **What was kept.** The gate (at least 1.5x on arithmetic-heavy equations while bit-identical) is met, so the backend stays as an experimental opt-in feature of `animscript-math` with its tests and its benchmark rows. It is NOT wired into the language, the CLI or `--jit`: the gain exists only for equations without transcendental functions, the compile is twenty times slower, and it needs the LLVM development package on the machine. The Cranelift JIT stays the one machine-code path. A candidate for later: big field kernels that are pure arithmetic (diffusion, reaction-diffusion).
251. **Building it.** `apt-get install llvm-18-dev`, then `LLVM_SYS_181_PREFIX=/usr/lib/llvm-18 cargo test -p animscript-math --features llvm` (`cargo run --release -p animscript-math --features jit,llvm --example bench` for the numbers). It links the shared `libLLVM` (`llvm18-1-prefer-dynamic`), because the Ubuntu package has no static Polly. Default builds, `jit` builds and CI are unaffected: nothing of it is compiled unless the feature is on.
252. **The thunks moved** to `crates/math/src/thunks.rs`, shared by both machine-code backends (each one calls the same Rust functions the interpreter runs).
253. A mesh is a Shape with `mesh: Option<Mesh>`, so every 2D feature (keys, drive, opacity, z, fill, stroke) applies unchanged. `position.z` is valid on any shape.
254. Axes: x right, y down, z away from the viewer (right-handed); face normal is (v1-v0)x(v2-v0), outward; faces are convex (builders triangulate by ear clipping).
255. Rotation order is spin about y, then tilt about x, then roll about z, then translate, so a tilt stays a tilt after a spin. `pivot:` is a world point to turn about; `at:` places the bounding-box centre; default `at` is the origin.
256. `camera3` is a hidden shape created lazily at the first mesh and kept out of `shape_order`. Pitch: positive looks up (use negative to look down). Position is an offset from the default eye (w/2, h/2, -f0), f0 = (h/2)/tan(fov/2).
257. `stage3` defaults: light (-0.4, 0.7, 0.6) as the direction the light travels, ambient 0.35, bands 3, crease 40 deg, smooth 50 deg, fov 40.
258. Per-corner smoothed normals (within `smooth`) are interpolated per pixel, perspective-correct; outline visibility uses the depth buffer with margins (silhouette 3%+5, others 1.2%+2) to avoid dashed edges on curved silhouettes.
259. Vertex drives gain input `z` (DRIVE_POINT is x,y,z,i,u); `z` and the 3D property drives are meshes-only (`E-MTH-004` on 2D shapes). Fields are sampled at a mesh vertex's world (x, z).
260. `E-MSH-001` covers bad mesh builtin arguments; `extrude` with neither a shape nor points is `E-TYP-003`.
261. All meshes are drawn together at the first mesh's z-order position; 2D shapes at other z values are in front of or behind that group. The 2D view transform also applies to 3D.
262. **Own 3D engine, no Jolt (approved).** `crates/physics3` is a dependency-free sequential-impulse engine in the same style as the 2D one: determinism is bit-exact (a `state_hash` test), no C++ toolchain, and the `PhysicsWorld` trait stays as the place a Jolt backend could still plug in.
263. **Shapes.** Sphere, capsule (along the body's y), convex hull (boxes are hulls), static infinite plane and static triangle soup. Hulls are built by an incremental hull with coplanar triangles merged into polygon faces; mass and inertia come from the hull, and the body frame is centred on the centre of mass.
264. **Collision.** Hull-hull uses the separating axis test (face axes and only the edge pairs that form faces of the Minkowski difference) with face clipping for up to four contact points (reduced to the deepest, the farthest, and the two giving the largest area). Sphere and capsule against a hull use the signed distance to the hull (the capsule by a ternary search along its axis, adding the end contacts when it lies flat). A triangle of a static mesh is a two-faced hull. The broad phase is a plain O(n^2) bounding-box test with fixed pair order (scenes are small); this replaces the sweep and prune mentioned in the plan.
265. **Solver.** Per substep: forces, contacts rebuilt, warm starting matched by the contact's position in body A's frame (0.05 m), 10 velocity iterations with a Baumgarte push (0.2, slop 4 mm, at most 3 m/s), friction as two tangent rows clamped by the normal impulse, restitution above 0.6 m/s. Islands sleep after 0.5 s below 0.06 m/s and 0.12 rad/s and wake when touched or jointed to something awake.
266. **Joints.** Ball, hinge, fixed, distance (rigid, spring or rope). Ball, hinge and fixed solve all their rows as one dense block (5x5 or 6x6 inverse), because solving the point and angular rows one after the other did not converge for a light body on a long lever; hinge limits and motors act on the axis row with the other rows kept satisfied. No gyroscopic term.
267. **Language names.** `world: gravity: (x, y, z)` (three numbers) sets a 3D gravity; a 2D pair keeps meaning 2D (and gives the 3D world (x, y, 0)). Joint functions are `ball`, `hinge`, `fixed`, `strut` and `tether` so they cannot be mistaken for the 2D `revolute`/`weld`/`distance`/`rope`. The body argument for the shape is `collider`. Codes `E-PHY-007` (body) and `E-PHY-008` (joint) because `E-PHY-003` to `-006` were taken. A scene has 2D or mesh bodies, not both.
268. **Pose write-back.** A body's centre of mass is the mesh's pivot (`pivot3`), exactly as in 2D; the shape's `position` is moved so the pose is unchanged. A dynamic body's orientation is a quaternion held beside the shape (`orient`) and in the resolved frame (`q3`); the mesh transform uses it instead of the three turns. At the end of a scene it is converted back to the (y, x, z) turns, so later scenes and actions see ordinary rotation values. Kinematic bodies read the resolved position and turns each tick. Non-convex static meshes are made into triangles in world space at the start of the scene.
269. **Sphere detection.** `collider: auto` treats a mesh as a sphere only when every vertex is within 2% of the mean distance from the centroid and its hull has at least 20 faces (a cube's corners are all equidistant too).
270. **Floating.** `float` samples a 3 x 3 x 3 grid of points in the body's bounding box; each point under the surface `level - field(x, z)` adds a share of the buoyant force (fluid density x gravity x volume) at that point, so a tilted body is righted; the water's drag is linear in how much of the body is wet.
271. **Not now:** continuous collision detection, ragdolls and skeletons in 3D, 3D soft bodies and particle fluids (next in the plan), concave dynamic bodies (build them from several convex meshes joined with `fixed`).
272. **Method (3D soft bodies).** XPBD, like the 2D engine (`crates/physics3/src/soft.rs`), stepped together with the rigid world: `soft_substeps` small steps per frame, each stepping the rigid world and then every soft body (predict, constraint iterations, collisions, velocity update, bounce and friction).
273. **Nodes and points.** A body has simulated nodes and collision/render points embedded in them (up to four nodes with weights). A surface body's vertex is its own node; a solid body's vertices are embedded in the tetrahedron that holds them. A correction to a point is shared among its nodes by weight over mass, so every collision and pin works the same for both.
274. **Constraints.** Edges keep their length; every edge shared by two triangles gets a bending link between the opposite vertices; a closed surface keeps a global volume (pressure) by gradient of the signed volume; a solid body has a volume constraint per tetrahedron plus its edges as braces. Stiffness maps to compliance `(1 - s) / s * 1e-4` (bend: `(1 - b) / b * 5e-3`, volume `1e-6`).
275. **Tetrahedralizer.** A regular lattice (`resolution` cells along the longest side) over the cells the closed mesh fills or touches, six tetrahedra per cell with the same diagonals everywhere, and every mesh vertex embedded in the tetrahedron that holds it (the nearest one if outside). Chosen over a Delaunay tetrahedralization because it is deterministic and needs no robust predicates; the cost is a stair-stepped skin under the mesh, which is why only lattice nodes inside the mesh also collide (as extra points) and why a solid cube needs `subdivide` to show a dent.
276. **Contacts.** Points are spheres of radius `thickness` against rigid shapes (`World::query_sphere`). Static and kinematic bodies are pushed out of by position (at most a quarter radius per substep, so a body that starts resting on something is eased out instead of popped); a dynamic body and the point share a velocity impulse by inverse mass with a separation bias, so a heavy box sinks into a pad only as fast as the pad gives way. Friction is limited by the normal impulse, or for a resting contact on a static body by the push that held it out. Soft against soft is point against triangle by position, through a grid in a fixed order; self collision skips triangles near the point at rest.
277. **Pins and cut.** Pins hold nodes at a target that moves linearly through the substeps; `pin_where(test, to: mesh)` stores the pin's place in the other mesh's own coordinates and follows it (a dynamic body from the world, extrapolated by its velocity for a frame; anything else from its resolved pose). `cut` removes faces by edge distance to a segment and every link no remaining face holds; the mesh's faces are edited and later cuts keep their original numbers. Cutting solid bodies is not supported (the skin is embedded in the lattice, so a cut would have to split tetrahedra).
278. **Language.** A soft mesh's transform is baked into its vertices when the scene starts and the final vertices are written back at the end; vertex positions go to the renderer through the resolved vertex overrides (soft wins over vertex drives). Duplicate vertices at one place (a lathe seam, a cube's faces) are welded for the simulation. `soft` arguments are the 2D ones plus `solid` and `resolution`; `E-PHY-009` is the 3D soft error, `E-PHY-003` still means rigid and soft on one shape. `cube` gained `subdivide`. A scene may mix soft meshes and rigid mesh bodies; soft bodies are not 2D.

## The editor (round E1)

279. **A Rust desktop app that works with the code (approved).** `crates/editor` (binary `anim-editor`) is an `eframe`/`egui` 0.36 application with six docked panels: code, preview, hierarchy, inspector, timeline and console. The code is the only document: every panel shows what the last run of the code produced, and the panels are read-only in this round (editing through the inspector or the timeline is a later round). It uses the language crates directly; a separate facade crate was dropped when independence from the language stopped being a goal.
280. **Dependencies.** `eframe`/`egui` (the app and widgets), `egui_dock` 0.21 (dockable, resizable, closable panels), `rfd` (native open/save dialogs) and, for tests only, `egui_kittest` (drives the UI without a window). `egui_code_editor` was planned but its current release needs egui 0.37 while `egui_dock` needs 0.36, so the code panel is egui's own `TextEdit` with our colouring. No ready-made timeline widget exists for egui: the timeline is drawn by hand (`panels/timeline.rs`).
281. **OpenGL backend.** eframe is built with `glow` instead of its default `wgpu`: it starts on software rendering and remote displays (it is how the app is run under Xvfb here), and it builds much faster.
282. **Not in the default build.** The workspace's `default-members` leave the editor out, so `cargo build` and `cargo test` never compile the GUI stack. Use `cargo run -p anim-editor -- file.anim` and `cargo test -p anim-editor` (or `--workspace`).
283. **The run journal (`crates/lang/src/journal.rs`).** A run now also returns a journal: the scenes with their project-tick ranges, every timed item (action, modifier, key set) with its target shape, label, start and end tick and source span, every `note` with its tick, and the declarations (shapes, props, emitters, fluids, fields, armatures) with their lines. It is always recorded (a few pushes per action) and is a side table: nothing of it is in a snapshot, so no hash or golden file changed.
284. **Journal ticks are project ticks** (indices into `snapshots`), not scene ticks. An action's `end` is exclusive; a key set's `end` is its last key tick (inclusive, as in decision 63). Whatever is still running when its scene ends (an open-ended modifier, `hold_after` keys) ends there, and a cancelled or interrupted action or modifier ends at the tick it was stopped.
285. **One key item per statement.** The runtime makes one key set per scalar (`position.x`, `position.y`...). The journal merges the key sets that one statement makes for one shape in one tick into a single item with the union of their key ticks; the keys of a scene's `timeline:` are merged per shape and point at its first `at` block.
286. **Which line an item points at.** The innermost statement *in the user's own file* that led to it. Inside an imported library (whose text is appended after the program) that is the statement that called into it, so `turret.look_to(...)` is the line of a `rotate_to` made by `std/motion`; inside your own `action` it is the statement in the action's body. A `fn` called while evaluating a statement never changes the current statement. A task started by `start`, `together:` or `race:` inherits the line that started it until it runs a statement of its own.
287. **`run_partial`.** `run_partial` / `run_partial_in` return the frames made before a runtime error together with the error; `run` / `run_in` are now thin wrappers that turn that error back into `Err`. A program that does not parse or build is still an `Err` with no frames.
288. **Running while typing.** The editor checks and runs the whole program on a worker thread 0.35 s after the last keystroke (`Run > Run while typing` turns that off; F5 or Ctrl+Enter runs at once). Only the newest text waiting is run. There is no incremental run: a heavy simulation lags behind typing by its run time.
289. **The last good run stays on screen.** The preview, hierarchy, inspector and timeline show the last run that produced frames (marked "last good run" when the text has moved on), so a half-typed line does not blank them. The console and the underlines in the code always belong to the newest run.
290. **Errors past the end of the file.** Some diagnostics ("unclosed bracket at end of file") point past the user's last line when something is imported, because imports are appended to the source. The editor moves such a span to the last line and adds a note with the reported line. The underlying line number is unchanged in the CLI. **Needs review** if you want it fixed in the lexer instead.
291. **Syntax colours.** A forgiving scanner in the editor (`highlight.rs`), not the language's lexer, because the lexer stops at the first error and drops comments. It follows decision 11 for `#` (colour or comment) and treats a word after a dot as a member, never a keyword.
292. **Preview.** The frame is drawn by `animscript-render` (so it is what `anim render` writes) at the size shown, up to 2x, and re-rendered only when the tick, the run, an overlay or the zoom step changes. Selecting is by bounding box (the top-most visible shape whose box contains the pointer); the selection is outlined with that box through the snapshot's camera.
293. **Inspector.** For a shape: transform, style, what is animating it at the playhead (each entry links to its line), geometry, then every other key of its snapshot entry and any entry of the other snapshot sections (physics bodies, particles, rigs...) that names it, shown generically, so new engine features appear without editor changes.
294. **Not in this round:** editing values or keys from the panels, autocomplete, find and replace, multiple files or tabs, exporting video from the editor, saved window layouts, and audio.
295. **Prebuilt binaries.** `.github/workflows/release.yml` runs on a `v*` tag (or by hand, without publishing): one Ubuntu 22.04 job builds `anim-editor` and `anim` for Linux and, with MinGW, for Windows (`x86_64-pc-windows-gnu`), packs each with the examples (without the videos) and `DOCUMENT.md`, and creates the GitHub Release. Windows is cross-compiled because the vendored C brush engine is built with GCC flags (`-std=gnu11`) that MSVC does not take; the resulting programs need only libraries Windows ships. Ubuntu 22.04 keeps the Linux programs usable on older systems. 64-bit x86 only, nothing is code-signed, and there is no installer. The editor is a GUI-subsystem program in Windows release builds (no console window).
280. **Method (3D fluids).** The same position based fluids as 2D (`crates/physics3/src/fluid.rs`): poly6 and spiky kernels in 3D (support h = 2 spacings, rest density from a cubic lattice), a density constraint solved Jacobi style for `fluid_iterations` iterations, XSPH viscosity, optional cohesion, speed capped at 20 m/s. A fluid steps `fluid_substeps` times per frame, every `soft_substeps / fluid_substeps`-th substep of the coupled step when soft bodies are present.
281. **Deterministic threads.** The solver's per-particle loops (neighbour lists, density, displacement, velocity update, contact queries) run through `par_ranges`: the index range is cut into contiguous chunks, each computed on its own thread (`std::thread::scope`, no dependency) and joined in chunk order, so every element is computed by the same code from the same inputs whatever the thread count and the output is identical for 1 or N threads (tests compare state hashes for 1, 3 and 4). `fluid_threads` in `world:` sets the count (0 = the machine's). The marching-cubes density field is filled in z slabs, each summing the particles that reach it in index order, with the same guarantee.
282. **Contacts.** Particles are spheres of half a spacing against rigid bodies (`World::query_sphere` from a margin around where the particle was, so a fast particle cannot jump a wall), pushed out inside the density iterations; a dynamic body gets the momentum of the push-out as an impulse (`m = density x spacing^3`), which makes crates float and balls splash. Soft bodies are one-way obstacles (spheres sampled over their live triangles about half a spacing apart). A body at rest keeps one layer of particles under it, so it floats about a spacing above the floor.
283. **Units.** The density of a fluid is kg per cubic metre with 1 meaning water, in the unit system where a body of density 1 floats just submerged; bodies below 1 float. Spacing defaults to 12 px in 3D (8 px would need 3 to 4 times the particles for a tank), `max` to 6000 per fluid and 20000 over all fluids (2D and 3D).
284. **Syntax.** The existing `fluid` block and operations, with `fill: box(x, y, z, w, h, d)` (a new `Region` value) or a mesh shape, and `space: "3d"` for a fluid that is only poured. The dimension is decided when a scene starts (a mesh shape is only known to be a mesh then). `pour` takes 3D points and a jet diameter (a disc of particles every spacing travelled); `level(x, z)`; `add(box(...))`. `world: fluid_threads`. Codes: `E-FLD-004` for 3D fluid errors (`E-FLD-001..003` already exist for 2D fluids and fields).
285. **Surface (marching cubes, in the renderer).** The snapshot carries each fluid's particles packed as little-endian f32 triples in base64 (a frame is ~12 bytes a particle instead of a JSON array per particle: `anim frames` keeps every snapshot). The renderer decodes them, sums a kernel (radius `smoothness` x spacing) onto a grid of half-spacing cells (coarser if the grid would pass 6 million cells), takes the iso value from the field half a spacing outside a flat lattice face, and extracts triangles with the classic 256-case table (Bourke's numbering; wound to agree with the field gradient, normals from the gradient). It is a pure function of the snapshot.
286. **Transparency.** The water is drawn after the opaque meshes and their outlines, with its own depth passes: the nearest front face (hidden if an opaque surface is nearer), the nearest back face behind it, and the distance between them or to the opaque surface as the thickness. Colour goes from `color` to `color_end` by `1 - exp(-thickness / (8 spacings))`, alpha from the colour's alpha, the thickness and a grazing-angle term; the surface is toon-lit by `stage3` with a highlight band and an inked silhouette where it meets air or something farther. Loose droplets and thin jets (three or fewer neighbours) are drawn as small lit balls and foam as white sprites, both hidden by nearer surfaces. A scene whose only 3D object is water still gets the 3D pass (and a camera3).
287. **Not now:** refraction, reflection and caustics, fluid-to-field coupling, two-way coupling with soft bodies, continuous collision for fast bodies, fast collision against large static triangle meshes.
288. **Water pushes soft bodies.** The obstacle spheres a fluid sees on a soft skin now carry their triangle and barycentric weights (`Sample`); a particle pushed out of one gives the triangle's points the reaction momentum `m * push / dt`, shared by weight and inverse mass (`SoftBody3::apply_sample_impulse`), with the sample's velocity change capped at 3 m/s per fluid step so a light cloth cannot be flung. Same convention as the rigid reaction.
289. **Wave fields as a surface.** A fluid with `field:` rests on a `Surface` handed to the engine each frame (a copy of the field's height and velocity channels, sampled bilinearly like `field_sample`): the surface is at `level - h` px, its normal from the slope (central differences over half a cell), its velocity `-v`. Particles are pushed out of it by position (at most 0.8 spacing per iteration), keep its tangential velocity and lose the component into it. Outside the field's rectangle there is no surface.
290. **Ripples and absorption.** Water pressing into the surface reports impacts: only a particle arriving (not touching it in the last 6 substeps) and faster than 0.5 m/s relative to the surface counts, with momentum `m * (into - 0.5)`, `into` capped at 4 m/s; resting water pressing with its weight would otherwise sink the field without end, and an impact that fed back through the surface velocity made it unstable. Impacts poke the height channel by `-10 px per kg m/s * ripple` (radius two cells or 1.5 spacings, clamped to +-150 px). With `absorb` (default for a fluid on a field) a particle that touches the surface for three substeps in a row joins it and is removed; the field rises by its volume (`3 V / (pi r^2)` at the poke, so the volume is conserved). `absorb: false` keeps a pool resting on the field.
291. **Stable fields.** The wave equation of a field is solved with `c2` in grid-cell units, so a step is stable only while `c2 * dt^2 < 0.5` (`dt` is the frame divided by the field's `substeps`); the examples use `c2: 2400` with `substeps: 4`. Examples and tests that coupled water to a field blew the field up until this was found; it is in the docs.
292. **Materials.** A mesh shape may have a material: `mirror(reflect)` keeps it opaque in the z-buffer pass and records, per pixel, its reflectance and normal, which the water/glass pass turns into a reflection; `glass(opacity, reflect, refract)` takes it out of the opaque pass and composites it like water (front and back faces give a thickness, `fill` is the tint). `matte()` clears it. They are properties of the shape (`MeshMaterial`), so any mesh can have them, including the water plane driven by a field.
293. **Screen-space reflection and sky.** The renderer builds the picture under the surfaces (the 2D backdrop sampled at supersample size, the opaque meshes over it), then for each reflective pixel marches a reflected ray (up to 56 steps growing by 14% + 4 px) and compares its camera depth with the opaque depth buffer (thickness 30 px + 6% of the depth); a hit gives the colour of that pixel, a miss the sky. The sky is a vertical gradient between two colours (`stage3(sky: (top, horizon))`, default: the project background at the horizon and a bluer one above), darker below the horizon, plus a light glint `s^160 * 1.4 + s^16 * 0.12`. Everything is a pure function of the snapshot.
294. **Refraction and Fresnel.** For water and glass the refracted ray (eta 1/1.33) is followed for `min(thickness, 1.5 absorption lengths) * refract` and the picture where it comes out (if that pixel is behind the surface) replaces the straight-through one; it is tinted by `1 - exp(-thickness / (8 spacings))` toward the body colour. The reflected share is `(0.05 + 0.95 (1 - cos)^5) * reflect`, capped at 0.95. Surfaces are composited in order: mirrors, then fluids, then glass, each updating the picture the next sees, so glass in front of water refracts the water. Order among surfaces of one kind is not sorted.
295. **Not now:** caustics, total internal reflection, reflections of glass and water in each other, ray-traced (off-screen) reflections, two-way coupling of water with a field's gradient channels beyond the slope force.
