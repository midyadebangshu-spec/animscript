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
