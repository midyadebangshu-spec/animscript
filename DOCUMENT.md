# AnimScript Syntax Guide

A learn-by-example guide to writing AnimScript (`.anim`) files. It covers every piece of syntax in the
language specification v0.1, plus the planned shape system.

> **Status.** The compiler is being written in Rust. Today only the **shape system** (section 13) and the
> small part of the language it needs (statements, loops, `when`/`whenever` rules on flags, vertex animation)
> actually run. Everything else in this guide describes the specification and is **not implemented yet**.
> If this guide and the specification disagree, the specification wins, except for section 13, which the next
> spec revision will absorb (see `docs/DECISIONS.md` for every choice made).

---

## 1. The big idea

An AnimScript program describes a **story that reacts to itself**. Characters have *flags* (facts such as
`is_scared`). *Rules* watch flags and events and start *actions* (things that take time, like flying away).
The whole thing is simulated one frame ("tick") at a time and rendered to video.

Three ideas to keep in mind:

1. **Time is whole ticks.** One tick is one frame. There is no wall-clock time.
2. **Rules react; actions move.** Rules start actions. Actions never start rules directly. They only change the
   world and raise events.
3. **Same input, same output.** Every run with the same source, assets and seed is bit-for-bit identical.

---

## 2. Files and layout

- Source files are UTF-8 with the extension `.anim`. A file that contains only a screenplay is `.animplay`.
- Line endings may be LF or CRLF. A leading byte-order mark is ignored.
- A project may span several files joined with `import`. Exactly one file contains `project`.

### Indentation and blocks

A block starts with a colon at the end of a line, and its lines are indented by **four spaces**.

```anim
when bird.is_ready:
    bird.look_at(rose)
    wait 1.second
```

- Tabs in indentation are an error (`E-LEX-001`).
- A dedent that doesn't return to an enclosing level is an error (`E-LEX-002`).
- Inside `()`, `[]`, `{}` newlines are ignored, so long calls can span lines.
- A block with one simple statement may sit on the same line: `when bird.is_ready: bird.look_at(rose)`.

### Comments

```anim
# a normal comment, runs to the end of the line
## a doc comment: attached to the next declaration and visible to tools
```

There are no block comments.

### Names

- Identifiers are lower snake case: `[a-z_][a-z0-9_]*` (`bird`, `is_ready`, `look_at`).
- Type names (used in action signatures) are `CapitalCase`: `Character`, `Duration`.
- Project, scene and shot names are string literals and may contain spaces: `"The Departure"`.
- A dotted name selects a member of an entity: `bird.is_ready` (a flag, event, property, channel or action).

---

## 3. Literals, types and units

| Literal | Examples | Type |
|---|---|---|
| Integer | `24`, `1_000` | `Int` |
| Number | `0.5`, `.25`, `3e2` | `Number` |
| Boolean | `true`, `false` | `Bool` |
| String | `"wind"`, `"The Departure"` | `String` |
| Length | `40.px`, `10%w`, `5%h` | `Length` |
| Angle | `90.deg`, `0.5.turn`, `1.2.rad` | `Angle` |
| Duration | `2.seconds`, `1.second`, `1.5.s`, `250.ms`, `36.frames`, `36f` | `Duration` |
| Gain | `-6.dB` | `Gain` |
| Colour | `#1d2b53`, `#fff`, `rgb(29, 43, 83)`, `hsl(220, 50%, 20%)` | `Color` |
| Vector / point | `(900.px, 520.px)` | `Vec2` / `Point` |

Unit suffixes follow a dot (`1.5.s`). The shorthand `1.5s` is also accepted. An unknown suffix is `E-LEX-004`.

**Coordinate system.** The origin is the top-left of the canvas. `+x` is right and `+y` is down (like SVG).
Angles are measured **clockwise from `+x`**, so `0.deg` points right and `90.deg` points down.
`%w` and `%h` are percentages of the canvas width and height.

### All types

`Bool`, `Int`, `Number`, `String`, `Duration`, `Length`, `Angle`, `Gain`, `Color`, `Vec2`, `Point`,
`Region` (`rect(x, y, w, h)`, `circle(centre, r)`, `canvas`, `view`), `Entity` (a character, prop or the camera),
`Asset`, `Easing`, `Path`, `Direction`, `Handle` and `Flag`.

`Direction` is one of `up`, `down`, `left`, `right`, `up_left`, `up_right`, `down_left`, `down_right`,
or an `Angle`.

`Int` converts to `Number` implicitly, never the reverse.

### Arithmetic is unit-checked

| Expression | Result |
|---|---|
| `Duration + Duration`, `Duration - Duration` | `Duration` |
| `Duration * Number`, `Duration / Number` | `Duration` |
| `Duration / Duration` | `Number` |
| `Length + Length`, `Length * Number`, `Length / Length` (gives `Number`) | `Length` / `Number` |
| `Length / Duration` | speed |
| `Angle + Angle`, `Angle * Number` | `Angle` |
| `Point + Vec2`, `Point - Point` | `Point`, `Vec2` |
| `Length + Duration`, `Duration + Number`, … | **error** `E-TYP-002` |

A bare number has no unit. Write `2.seconds`, not `2`.

### Operators (high to low precedence)

1. member access `a.b`, call `f(x)`, index `a[i]`
2. unary `-`, `not`
3. `*` `/` `%`
4. `+` `-`
5. `<` `<=` `>` `>=`
6. `==` `!=`
7. `and`
8. `or`
9. `??` (null-coalescing: `a ?? b` is `b` when `a` is absent)

There is **no assignment expression**. Assignment is a statement.

### Built-in pure functions

`distance(a, b)`, `direction_to(a, b)`, `center(e)`, `bounds(e)`, `inside(e, region)`, `touching(a, b)`,
`on_screen(e)`, `min`, `max`, `clamp`, `lerp`, `abs`, `round`, `floor`, `ceil`, `frames(d)`,
`unit_vector(angle)`, `left_of(e)`, `noise(seed_name, t)`, `random(seed_name)`.

**Reading the current state.** `shape.position`, `.scale`, `.rotation` and `center(shape)` return the *current*
animated transform (as resolved on the previous frame), so an action can follow a moving target:

    action follow(self: Shape, target: Shape, n: Int):
        let i = 0
        while i < n:
            self.move_to(center(target))
            wait 1.frame
            i += 1

A bone also has `head`, `tip` (points) and `angle`: `probe.move_to(hero.forearm.tip)`.

Functions that read the world see the **last committed frame**, never a half-updated world.

---

## 4. Declarations

A program has exactly one `project`, then imports, assets, a cast, sets, action definitions and scenes.

### 4.1 `project`

```anim
project "The Little Bird":
    anim_version: "0.1"
    canvas: 1280x720
    fps: 24                 # an integer, or a rational such as 30000/1001
    seed: 7                 # master seed for all randomness (default 0)
    sample_rate: 48000      # audio, Hz (default 48000)
    output: "mp4"
```

`anim_version`, `canvas` and `fps` are required. A missing `anim_version` is `E-PRJ-001`.
`experimental: [name]` opts in to unstable features (`E-PRJ-002` if you use one without it).

### 4.2 `import`

```anim
import std.2d                      # automatic unless you write `import none`
import "shared/forest.anim" as forest
```

Name clashes between imports are `E-NAM-002`. Qualify a name to choose: `forest.fly_away`.

### 4.3 Assets

```anim
sprite  bird_art:   "assets/bird.svg"
sound   wind:       "assets/wind.ogg"
music   theme:      "assets/theme.ogg"
clip    bird_flap:  "assets/bird_flap.clip"
font    title_font: "assets/title.ttf"

sound wind: "assets/wind.ogg"
    cue "gust" at 1.2.seconds        # a named time point inside the asset
```

A missing file is `E-AST-001`.

### 4.4 `character`

```anim
character bird:
    rig: sprite_rig(bird_art, pivot: (0.5, 0.9), features: [gaze, wings],
                    clips: { flap: bird_flap })
    at: (900.px, 520.px)
    z: 5                                      # drawing order; higher is in front
    states: is_ready, feels_afraid            # authored flags (all start false)
    event chirped                             # a custom event
    derive decides_to_leave when feels_afraid.held_for(1.5.seconds)
    can: look_at, step_back, look_around, fly_away
```

- `rig:` gives the character a body. `sprite_rig(asset, pivot, features, clips)` is the only rig in v0.1.
  *Features* (`gaze`, `wings`, `legs`, …) are what actions *require*.
- `states:` declares authored flags. `derive` declares a derived flag. `event` declares a custom event.
- `can:` is an allow-list. A call to any other action is `E-CAP-001`. If omitted, every action whose
  requirements the rig satisfies is allowed.

### 4.5 `prop`

An entity with no behaviour of its own. It accepts the same `states`, `event`, `derive` and `can` entries.

```anim
prop rose:
    sprite: rose_art
    at: (600.px, 540.px)
    z: 3
```

### 4.6 `set`

A stack of background layers with a ground line.

```anim
set garden:
    layers:
        - sprite: "assets/garden_sky.svg"    parallax: 0.1
        - sprite: "assets/garden_ground.svg" parallax: 1.0
    ground_y: 560.px
```

`parallax` is the fraction of camera movement a layer follows (`1.0` moves with the camera).

### 4.7 Flags and variables

A **flag** is a named boolean fact about an entity.

| Kind | Declared with | Changed by | Example |
|---|---|---|---|
| Authored | `states:` | `becomes` / `clears` statements only | `bird.is_ready` |
| Derived | `derive … when …` | never assigned; recomputed from a predicate every delta | `bird.decides_to_leave` |
| Physical event | built in | the engine | `bird.leaves_screen` |

A **variable** holds a typed value:

```anim
var courage: Number = 1.0
bird.courage = 0.4                  # a statement
when bird.courage < 0.5:            # a predicate trigger
    ...
```

The compiler warns (`W-RUL-001`) if a rule's trigger is an authored flag that nothing can ever set.

### 4.8 `derive`

```anim
derive decides_to_leave when feels_afraid.held_for(1.5.seconds)
derive too_close         when distance(self, rose) < 120.px
derive alone             when not touching(self, rose) and not on_screen(dog)
```

Inside an entity, `self` is that entity and bare flag names refer to its own flags. `becomes` and `clears` on a
derived flag are errors (`E-STA-001`). A cycle between derived flags is `E-STA-002`.

Predicate helpers on flags:

| Function | True when |
|---|---|
| `x.held_for(d)` | `x` has been continuously true for at least `frames(d)` ticks |
| `x.since(d)` | `x` became true no more than `frames(d)` ticks ago |
| `x.rose` | `x` became true in this delta (a rising edge) |

### 4.9 `scene`

```anim
scene "The Departure" in garden for 10.seconds:
    cast: bird, rose
    setup:
        bird becomes is_ready
    main:
        ...                         # optional linear narration
    when bird.is_ready:
        ...                         # reactive rules
    derive ...                      # scene-level derived flags
    shots:
        ...                         # camera
    timeline:
        ...                         # absolute-time keyframes
```

- `for 10.seconds` fixes the length. `for auto` ends the scene when `main:` finishes (`E-SCN-001` without `main:`).
- **Cast.** Only cast entities exist. An entity from the previous scene starts at its end-of-scene state. Mark
  `(fresh)` to start from its declaration instead: `cast: bird (fresh), rose`.
- `setup:` runs once at tick 0 and may contain only zero-time statements (`E-STM-001` otherwise).
- `main:` is task 0, starting at tick 0 after `setup:`.
- Rules last for the scene (or for the shot they're written in). **Flags carry over between scenes, rules do not.**
- Unknown set is `E-SCN-002`.

---

## 5. Time

- One tick = one frame. At `fps: 24`, tick `n` is scene time `n / 24` seconds. Scene time starts at tick 0
  in every scene.
- Durations convert to frames with **exact rational arithmetic, rounding half to even**:
  `0.5.seconds` → 12 frames, `0.6.seconds` → 14. A positive duration that rounds to 0 is raised to 1
  frame with warning `W-TIM-001`.
- `36f` and `36.frames` are exact tick counts and never rounded.
- There is no sub-frame timing, except audio, which is sample-accurate.
- `fps` must not change between scenes.

An action of `D` frames starting at tick `s` is sampled at ticks `s … s+D-1`. It *finishes* at tick `s+D`, and the
next statement of its task starts **in the same tick**, so there are no gaps and no doubled frames.

---

## 6. Rules, tasks and statements

### 6.1 Rules

```anim
when      <trigger> [if <guard>] [priority <int>]:                      body
whenever  <trigger> [if <guard>] [priority <int>] [retrigger ignore|restart|queue]:   body
while     <condition> [priority <int>]:
    body
    finally:
        cleanup
```

| Form | Meaning |
|---|---|
| `when T:` | Fires on the **first** occurrence of `T` in the scene, then is *spent*. Use for story beats that happen once. |
| `whenever T:` | Fires on **every** occurrence. While its body runs, a new occurrence is handled by `retrigger`: `ignore` (default, dropped), `restart`, or `queue` (at most 8 waiting). |
| `while C:` | Runs while `C` is true. It is **cancelled** when `C` becomes false and its `finally:` block runs. |

`priority` ranges from −100 to 100 (default 0). Higher priority runs first. Ties are broken by source order.

**Triggers:**

| Kind | Example | Fires |
|---|---|---|
| Flag | `bird.feels_afraid` | on a rising edge (false → true) |
| Predicate | `distance(bird, rose) < 120.px` | on a rising edge |
| Event | `bird.leaves_screen`, `marker("dawn")` | on each occurrence |
| Compound | `a.flag and b.flag` | on a rising edge of the whole expression |

Rules of triggers:
- A condition already true at tick 0 fires once, at tick 0.
- Events are instants, not values: they cannot appear under `and`/`or`/`not` (`E-TRG-002`). Add a guard instead:
  `when bird.leaves_screen if rose.in_bloom:`.
- A guard is checked at the moment of firing. If false, that occurrence is dropped (not retried). To wait for a
  condition, use a predicate trigger.
- Physical events lag by one tick: if the bird's first fully off-screen frame is 125, `leaves_screen` is raised at
  tick 126.
- An unusable trigger is `E-TRG-001`.

### 6.2 Built-in events and predicates

| Name | Kind | Raised / true when |
|---|---|---|
| `e.enters_screen` / `e.leaves_screen` | event | `on_screen(e)` changed |
| `a.touches(b)` | event | `touching(a, b)` became true |
| `e.finishes(action)` | event | an action named `action` on `e` finished this tick |
| `e.interrupted(action)` | event | such an action was interrupted this tick |
| `scene.starts` | event | tick 0, after `setup:` |
| `scene.ends_in(d)` | event | `frames(d)` ticks before the scene ends |
| `marker("name")` | event | a marker declared in `timeline:` or `shots:` |
| `cue("name")` | event | an audio cue is reached |
| `heard(asset)` | event | `sound.play(asset)` started in the previous tick |
| `x.chirped` | event | `emit x.chirped` ran in the previous delta |
| `on_screen(e)`, `inside(e, r)`, `touching(a, b)`, `distance(a, b)` | predicate | computed from the last committed frame |

An entity that starts the scene on-screen raises no `enters_screen`, and one that starts off-screen raises no
`leaves_screen`.

### 6.3 Statements

Every statement is **zero-time** (runs instantly), **blocking** (suspends the task) or **non-blocking** (starts
something and continues).

| Statement | Kind | Effect |
|---|---|---|
| `bird.look_at(rose)` (any action call) | blocking | Starts the action; resumes when it finishes |
| `wait 1.second` | blocking | Suspends for `frames(1.second)` ticks |
| `together:` *block* | blocking | Starts every child at once; resumes when **all** finish |
| `race:` *block* | blocking | Starts every child; resumes when the **first** finishes (others are cancelled) |
| `repeat N times:` / `repeat until C:` | blocking | Loops; the body must block at least once per iteration (`E-STM-003`) |
| `sound.play(wind)`, `music.play(theme)`, `sound.stop(h)` | non-blocking | Starts or stops audio and continues |
| `let h = start bird.flap()` | non-blocking | Starts an action in the background; `h` is a `Handle` |
| `await h` | blocking | Suspends until the handle's action ends |
| `cancel h` | zero-time | Cancels a running action |
| `bird becomes feels_afraid` / `bird clears feels_afraid` | zero-time | Sets / clears an authored flag |
| `emit bird.chirped` | zero-time | Raises a custom event in the next delta |
| `bird.courage = 0.4`, `place bird at (10.px, 10.px)` | zero-time | Writes a variable or rest value |
| `lock … = …`, `release h`, `attach`, `constrain` | zero-time | See §8 |
| `let x = expr` | zero-time | Binds a local name |
| `if cond:` / `else:` | zero-time | Chooses a block |
| `assert expr` | zero-time | Fails with `E-RUN-004` if false |
| `note "text"` | zero-time | Writes into the trace; no other effect |
| `finally:` | | Runs when the task ends for *any* reason; zero-time statements only |

**Sequencing is real time.** Two blocking statements in a row are separated by exactly the duration of the first:

```anim
when bird.feels_afraid:
    bird.step_back()        # body: tick 36 → 48
    bird.look_around()      # gaze: tick 48 → 84, starts the same tick step_back ends
```

**The most common mistake.** `camera.hold` blocks, so this starts the wind *after* the hold:

```anim
when bird.leaves_screen:
    camera.hold(2.seconds)
    sound.play(wind)        # starts 2 s later  ← usually not intended
```

Write one of these instead:

```anim
when bird.leaves_screen:
    sound.play(wind)        # non-blocking, starts now
    camera.hold(2.seconds)

when bird.leaves_screen:    # or make simultaneity explicit
    together:
        camera.hold(2.seconds)
        sound.play(wind)
```

The compiler warns (`W-SEQ-001`) when a non-blocking audio statement follows a blocking one longer than a second.

**Cancellation.** Cancelling a task cancels every action it owns (their current values are committed), cancels its
child tasks, then runs its `finally:` block. A task is cancelled when its scene ends, a `while` condition becomes
false, a `race` is lost, `restart` is chosen, or one of its actions is interrupted.

**Failure.** An action that fails at runtime puts its task in the *failed* state and writes `E-RUN-002` to the
trace. With `--strict`, the first failure aborts the render.

### 6.4 Channels

Every action *uses* channels, and at most one action may occupy a channel at a time.

| Entity | Standard channels |
|---|---|
| Character | `body`, `gaze`, `face`, `voice` |
| Prop | `body` |
| Camera | `frame`, `lens` (reserved) |

When a channel is busy, the action's policy decides (default `queue`):

| Policy | Behaviour |
|---|---|
| `queue` | Waits in a FIFO queue ordered by priority, then request order |
| `interrupt` | Takes over, but only if its priority is **greater than or equal to** the current occupant's. Otherwise it queues. |
| `reject` | Fails with `E-RUN-002` |
| `share` | Allowed only if every property uses `mode add` (e.g. a wobble); both run |

An action's priority is that of the rule that started it (or `0` for `main:`). Acquisition is atomic: an action
gets *all* its channels at once, so channel deadlock cannot happen.

An interrupted action's current values are committed, and its owning task is cancelled by default.

---

## 7. Properties, layers and blending

Every entity has **properties**, and each property **component** is resolved separately (a keyframe on
`position.y` doesn't disturb `position.x`).

| Property | Type | Components | Kind |
|---|---|---|---|
| `position` | `Point` | `x`, `y` | continuous |
| `rotation` | `Angle` | | continuous |
| `scale` | `Vec2` | `x`, `y` | continuous |
| `opacity` | `Number` in [0, 1] | | continuous |
| `tint` | `Color` | `r g b a` (linear light) | continuous |
| `gaze_angle` | `Angle` | | continuous |
| `gaze_target` | entity, point or none | | read-only; set by `look_at` / `gaze_to` |
| `z`, `pose`, `facing` (`left`/`right`), `visible` | | | **discrete** |
| camera `position`, `zoom`, `rotation` | | | continuous |

### Layers

Later layers win. Each tick the resolver folds the contributions of five layers:

| Layer | Source | Default mode |
|---|---|---|
| L0 Rest | entity declaration, plus values committed by finished actions | |
| L1 Semantic | library actions and clips (`fly_away`, `look_at`) | `override` |
| L2 Procedural | `wobble`, `spring`, `noise`, physics-like modifiers | `add` |
| L3 Keyframes | `timeline:` and `keyframes` | `override` |
| L4 Locks | `lock` statements | `override`, weight 1 |

Modes: `override` replaces the running value (blended by weight), `add` adds a scaled offset, `multiply` scales it.

```
v = rest
for each contribution sorted by (layer, priority, start tick, id):
    override: v = lerp(v, c.value, weight)
    add:      v = v + weight * c.value
    multiply: v = v * lerp(1, c.value, weight)
for each lock on the component: v = lock.value
```

- `lerp` on angles takes the shortest signed difference; on colours it works in linear-light RGBA.
- Weight is 1 unless the contribution declares `blend_in` / `blend_out` (frames).
- An action's value at tick `k` is its easing of progress, mapped between `from` and `to`. `from` is the
  property's resolved value at the action's start tick, so an action never snaps.
- Discrete properties can't blend. The last contribution (with weight above 0) wins.

### Commit on end

| How it stopped | `override` actions | `add` / `multiply` actions |
|---|---|---|
| Finished | final value written to rest | contribution removed, nothing committed |
| Interrupted / cancelled / failed | value of the last sampled tick written to rest | contribution removed |

So a bird that flew to the right edge *stays* there; a wobble that stops simply stops wobbling.

### Easing

| Name | `f(p)` |
|---|---|
| `linear` | `p` |
| `ease_in` | `p * p` |
| `ease_out` | `1 - (1 - p) * (1 - p)` |
| `ease_in_out` | `p < 0.5 ? 2 * p * p : 1 - 2 * (1 - p) * (1 - p)` |
| `ease_in_cubic`, `ease_out_cubic`, `ease_in_out_cubic` | cubic versions |
| `ease_out_back(k)` | overshoot; default `k = 1.70158` |
| `step` | `0` for `p < 1`, `1` at `p = 1` |
| `cubic_bezier(x1, y1, x2, y2)` | CSS-compatible |

### Keyframes and timelines

Two spellings, both layer L3.

```anim
timeline:                                  # scene level, ABSOLUTE scene time
    at 24f:
        bird.position = (400.px, 300.px)
    at 48f:
        bird.position = (650.px, 500.px)    ease ease_out
    at 2.seconds:
        bird.position.y = 450.px            # a single component
    marker "dawn" at 3.seconds

bird.keyframes over 2.seconds:             # inside a task, RELATIVE to the statement's start
    at 0.s:  position.x = 650.px
    at 1.s:  position.x = 700.px   ease ease_in_out
    at 2.s:  position.x = 900.px
```

- Keys are grouped per component. Between two keys the value uses the **later** key's easing (default `linear`).
- A keyframe set only contributes **inside its first-to-last key range**. Outside, its weight is 0. Add
  `hold_before` / `hold_after` to extend the first or last value.
- `blend_in` / `blend_out` soften the entry and exit.
- `bird.keyframes over …` is blocking and resumes at the last key.
- Overlapping keys for one component are `E-PRP-003`.

### Locks

```anim
let h = lock bird.position.y = 300.px          # non-blocking, returns a Handle
lock bird.position.y = 300.px for 2.seconds    # releases itself
release h
```

Two locks on the same component at the same tick are `E-PRP-002`.

### Constraints and parenting

Constraints run after the layer stack.

```anim
attach rose to bird.hand offset (4.px, 0.px)           # rose becomes a child of bird
constrain bird.position inside canvas                  # clamp after resolution
constrain bird.gaze to look_at(rose) weight 0.6        # bias gaze toward a target
constrain dog.position to follow(bird, lag: 6.frames)
```

Parents are solved before children; ties follow declaration order. A cycle is `E-CON-001`.

---

## 8. Actions and the standard library

### 8.0 Defining your own actions and functions (implemented)

Only the primitive operations are built into the language. Everything bigger, like turning to look at
something, walking somewhere or shaking, is an **action** you (or a library) write in AnimScript:

```anim
import "std/motion"                    # bundled: look_to, hesitate, shake, pulse, hop, angle_to

fn slot(i: Int) -> Number:             # a function computes a value instantly
    return 140 + i * 130

action pop_in(self: Shape, over: Duration = 0.4.seconds):    # an action takes time
    self.scale_to(0.01)
    self.scale_to(1) over over ease ease_out

shape b = square(size: 70, at: (slot(1), 470))
scene "s" for 3.seconds:
    main:
        b.pop_in()                         # runs like a built-in action, then the next line starts
        start b.hop(height: 50)            # or run it in the background: `let h = start ...`, `await h`, `cancel h`
        turret.look_to((400, 100), over: 0.6.seconds)         # look_to is written in AnimScript, in lib/motion.anim
```

- `action name(self: Shape, x: Number = 1, over: Duration = 1.second):` and
  `fn name(a: Number) -> Number:`. The first parameter named `self` makes the call work as `thing.name(...)`;
  without it call `name(thing, ...)`. Arguments are positional or named; defaults may use earlier parameters;
  a wrong type is `E-TYP-001`, a missing, unknown or doubled argument `E-TYP-003`.
- Types: `Any Number Int Bool String Point Angle Duration Color Shape Entity List Easing Region Vertex Edge
  Face Handle` (a bone is a `Shape`). Leave a type out to accept anything.
- `over D ease E` after a call becomes the named arguments `over` and `ease`, so declare them as parameters to
  accept them.
- **Inside a definition:** `return` (ends the call; gives an `fn`'s value), `while cond:`, `if`/`else`,
  `for`, `repeat`, `wait`, calls to other actions and functions, and `x = ...`, `x += ...` on variables declared
  with `let`. A body sees the top-level `let` values and its own parameters, not the caller's variables.
- A `fn` cannot wait or animate (`E-DEF-002`); calls nest at most 64 deep (`E-RUN-007`); a `while` that never
  waits is stopped (`E-STM-003`).
- Optional first lines of a definition: `uses body [on_conflict POLICY]` (channels, see 13k), `priority 2` and
  `requires n > 0` (checked on every call; `E-CAP-001` when false).
- `import "file.anim"` (relative to the importing file) or `import "std/motion"` (bundled) includes
  definitions. Library files are plain AnimScript: see `crates/lang/lib/motion.anim`, and copy and change them freely.
- `atan2(y, x)` gives an Angle; `angle_to(a, b)` (from `std/motion`) is the direction from one point to another.
- An action reads current values with `center(s)`, `s.position` and so on (13a), so it can follow a moving target.
  Worked example: `examples/actions.anim`.

### 8.1 Defining an action

```anim
action hesitate(self: Character, count: Int = 2, over: Duration = 1.second):
    ## The character leans back and forth before committing to something.
    uses body
    requires self.rig.has("legs") or self.rig.has("wings")
    repeat count times:
        move_by(self, (-6.px, 0.px), over * 0.25, ease_out)
        move_by(self, ( 6.px, 0.px), over * 0.25, ease_in)
    finally:
        note "hesitate ended"
```

General form:

```anim
action NAME ( self: Type [, param: Type [= default]]* ):
    uses CHANNEL [on_conflict POLICY] [, CHANNEL …]     # at least one
    [requires PREDICATE]*
    [priority INT]
    body
    [finally: zero-time statements]
```

- `self` is the entity the action is called on. Its type (`Character`, `Prop`, `Camera`, `Entity`) restricts who can call it.
- `uses` declares the channels occupied for the whole duration. Touching an undeclared channel is `E-CAP-003`.
- `requires` states conditions on the rig and entity, checked at compile time.
- Calls take positional or named arguments: `bird.fly_away(direction: up_left, over: 2.seconds)`.

### 8.2 Clips

A clip is data (named keyframe tracks of known duration) loaded from an asset. Call it like an action:

```anim
character bird:
    rig: sprite_rig(bird_art, clips: { flap: bird_flap, preen: bird_preen })

bird.preen()            # plays the clip "preen" on the body channel
```

### 8.3 Which implementation does a call use?

`entity.name(args)` is resolved **at compile time**; the first match wins:

1. a **clip** named `name` on the entity's rig (most specific);
2. a **rig-bound** action (`self: SpriteRig`);
3. a **library** action whose `self` type accepts the entity and whose `requires` the entity satisfies.

Two equally specific library actions give `E-CAP-002`. Qualify the name (`forest.fly_away`) to choose. If nothing
matches you get `E-CAP-001` listing the missing capability and suggested fixes. The compiler never substitutes a
different action and never invents motion.

### 8.4 Native primitives

Implemented by the runtime. Everything else is built from them.

| Primitive | Uses | Effect |
|---|---|---|
| `move_to(self, target, over, ease)` | body | Moves `position` to a point or entity |
| `move_by(self, delta, over, ease)` | body | Moves by an offset |
| `move_along(path, over, ease)` | body | Moves along a `Path` by arc length |
| `rotate_to`, `rotate_by`, `rotate_along_tangent(path, bank)` | body | Rotation (optionally following a path with banking) |
| `scale_to(factor, over, ease)` | body | Scales |
| `fade_to(opacity, over, ease)`, `tint_to(color, over)` | body | Opacity and colour |
| `set_pose(name)`, `face(direction)` | body | Instant discrete changes |
| `play_clip(clip, speed, loop)` | body | Plays a clip |
| `gaze_to(target, over, ease)`, `gaze_angle_to(angle, over, ease)` | gaze | Turns the gaze |
| `shake(self, amount, over)` | frame (camera) | Additive seeded shake |
| `wait` | none | A statement, no channel |

Paths are built with `line`, `arc`, `curve` and `arc_to_exit(self, direction, margin, wobble)`.

### 8.5 The standard library `std.2d`

| Action | `self` | Channels | Parameters (defaults) |
|---|---|---|---|
| `look_at` | Character | gaze | `target`, `over = 0.5.seconds`, `ease = ease_in_out` |
| `look_around` | Character | gaze | `sweep = 60.deg`, `over = 1.5.seconds` |
| `step_back` | Character | body | `from = gaze target`, `distance = 40.px`, `over = 0.5.seconds` |
| `walk_to` | Character | body | `target`, `speed = 120.px/second` or `over` |
| `turn_around` | Character | body | `over = 0.3.seconds` |
| `fly_away` | Character | body, gaze | `direction = up_right`, `over = 3.seconds`, `irregularity = 0.2` |
| `fly_to` | Character | body, gaze | `target`, `over`, `irregularity = 0.1` |
| `fade_in`, `fade_out` | Entity | body | `over = 1.second` |
| `hold` | Camera | frame | `over` |
| `pan_to` | Camera | frame | `target`, `over`, `ease = ease_in_out` |
| `zoom_to` | Camera | frame | `factor` or `size`, `over`, `ease` |
| `follow` | Camera | frame | `target`, `lag = 8.frames` (until cancelled) |
| `shake` | Camera | frame | `amount`, `over` |

Example of a real library definition:

```anim
action fly_away(self: Character, direction: Direction = up_right,
                over: Duration = 3.seconds, irregularity: Number = 0.2):
    uses body
    uses gaze on_conflict interrupt           # flying displaces whatever the bird was looking at
    requires self.rig.has("wings")
    let path = arc_to_exit(self, direction, margin: 40.px, wobble: irregularity)
    together:
        move_along(self, path, over, ease_in_out)
        rotate_along_tangent(self, path, bank: 15.deg)
        play_clip(self.rig.clip("flap"), loop: true)
```

---

## 9. Camera and shots

There is one camera, named `camera`, with the channel `frame` and the properties `position`, `zoom` and
`rotation`. `zoom` stays in **[1, 8]** (`E-CAM-001`). `1.0` shows the whole canvas. The visible region is
`canvas / zoom`, centred on `position`.

### Camera actions

```anim
camera.frame(medium, on: rose, thirds: left)      # at over: 0.s it is an instant set-up
camera.hold(2.seconds)                            # occupies the frame channel without moving
camera.pan_to(rose, over: 1.second, ease: ease_in_out)
camera.zoom_to(2.0, over: 1.second)
camera.follow(bird, lag: 8.frames)                # until cancelled
camera.shake(2.px, 0.5.seconds)
```

**Shot sizes** (the fraction of frame height the target's bounds should fill):

| Size | Fraction | Meaning |
|---|---|---|
| `extreme_close` | 0.90 | A detail |
| `close` | 0.60 | Head and shoulders, or a whole small prop |
| `medium` | 0.35 | The character from the waist up |
| `wide` | 0.15 | The character small in the environment |
| `establishing` | n/a | The whole set (`zoom = 1`) |

### Shots

A shot is a camera set-up that lasts a while. It is **not a cut in time**: the world keeps running.

```anim
shots:
    shot "Rose and bird" for 4.seconds:
        camera.frame(medium, on: rose, thirds: left)
    cut
    shot "Follow the bird" until bird.leaves_screen:
        camera.follow(bird, lag: 6.frames)
    dissolve 0.5.seconds
    shot "Empty garden" for 2.seconds:
        camera.frame(establishing)
        when scene.ends_in(1.second):             # rules are allowed inside a shot
            camera.shake(2.px, 0.5.seconds)
```

- A shot ends when its `for` duration elapses, its `until` condition rises, or (`auto`) its body finishes.
- A scene without `shots:` has one implicit `establishing` shot for the whole scene.
- `cut` is zero-time. `dissolve d` and `fade_through #000 d` are blocking, and the renderer blends both camera views.
- Rules inside a shot are active only while that shot is current.

---

## 10. Audio

Four mixer lanes, each with its own gain: `dialogue`, `music`, `ambient`, `sfx`.

```anim
let h = sound.play(wind, gain: -6.dB, pan: 0.0, loop: false, fade_in: 0.s, lane: sfx)   # non-blocking
sound.play(wind, wait: true)                       # blocks until the clip has played to its end
music.play(theme, gain: -12.dB, loop: true, fade_in: 2.s)
sound.stop(h, fade_out: 0.5.s)
sound.fade(h, to: -20.dB, over: 2.s)
bird.say("I should go.", tone: "soft")             # dialogue, BLOCKING until the line ends
bird.say(line_asset)
```

- `say` with text only lasts `max(1 second, 0.4 s × number of words)` and appears as a caption.
- Audio starts at sample `round_half_even(tick × sample_rate / fps)`. At 48 kHz and 24 fps that is `2000 × tick`.
- Lanes mix in the fixed order dialogue, music, ambient, sfx. A clipping mix warns (`W-AUD-001`); there is no limiter.
- Cues named in an asset raise `cue("name")` in the tick that contains that sample.

---

## 11. Script mode: writing a scene as a screenplay

Script mode is an alternate surface syntax for `main:`, following the Fountain screenplay format. It lowers to
exactly the same program, so everything after parsing treats the two identically.

```anim
scene "The Departure" in garden for auto:
    cast: bird, rose
    screenplay:
        EXT. GARDEN - DAWN

        The BIRD looks at the ROSE.
        (1 second)
        The BIRD steps back.
        The BIRD looks around.
        The BIRD flies away.

        SOUND: wind
        HOLD FOR TWO SECONDS.
```

A scene may contain both a `screenplay:` and rules. A file that is *entirely* script mode uses the extension `.animplay`.

| Element | Recognised as | Lowers to |
|---|---|---|
| Scene heading | `INT.` / `EXT.` then `PLACE - TIME` | binds the scene's set (`E-SCR-001` if unknown) |
| Action line | paragraph whose first word is a cast name | one action call, via the verb table |
| Character cue | capitals naming a cast member, then dialogue | `character.say(…)` |
| Parenthetical | `(softly)` under a cue | the `tone:` argument |
| Beat | `(beat)`, `(pause)`, `(2 seconds)` on its own line | `wait 0.5.seconds`, `wait 1.second`, `wait 2.seconds` |
| Camera direction | `CLOSE ON`, `WIDE ON`, `MEDIUM ON`, `PAN TO`, `HOLD`, `FOLLOW` | camera action |
| Sound cue | `SOUND:` or `MUSIC:` + asset | `sound.play(…)` / `music.play(…)` |
| Transition | `CUT TO:`, `DISSOLVE TO:`, `FADE OUT.` | `cut`, `dissolve`, `fade_through` |
| Parallel block | `MEANWHILE:` + indented lines | `together:` |
| Note | `[[text]]` | `note "text"` |
| Section / synopsis | `#`, `=` lines | ignored |

Blank lines separate elements. **Everything is sequential**: each action line finishes before the next starts.

### Verb table

```anim
phrase "looks at {target}"  -> look_at(target)
phrase "steps back"         -> step_back()
phrase "flies away"         -> fly_away()
phrase "is ready"           -> become is_ready          # a phrase may set a flag
```

Resolution of an action line:

1. The first word group matching a cast name is the **subject** (articles `the`, `a` are ignored).
2. The longest phrase in the table matching the rest is the **verb phrase**. `{target}` matches a cast name.
3. **Adverbs** adjust a parameter: `slowly` (over × 1.5), `quickly` (over × 0.6), `suddenly` (over = 0.2.seconds).
4. The result is type- and capability-checked like any call ("The STATUE flies away" is `E-CAP-001` if it has no wings).

A line that matches nothing is **not** an error. It becomes a `note` and the checker emits `W-SCR-001` so you can
see which prose has no animation behind it.

Standard phrases:

| Phrase | Call |
|---|---|
| `looks at {target}` | `look_at(target)` |
| `looks around` | `look_around()` |
| `steps back` / `steps back from {target}` | `step_back()` / `step_back(from: target)` |
| `walks to {target}` | `walk_to(target)` |
| `turns around` | `turn_around()` |
| `flies away` / `flies to {target}` | `fly_away()` / `fly_to(target)` |
| `fades in` / `fades out` | `fade_in()` / `fade_out()` |
| `hesitates` | `hesitate()` (when defined) |

Camera phrases: `CLOSE ON THE ROSE.` → `camera.frame(close, on: rose)`, `WIDE ON THE BIRD.`,
`PAN TO THE ROSE.` → `camera.pan_to(rose, over: 1.second)`, `HOLD FOR TWO SECONDS.` / `HOLD 2 SECONDS.` →
`camera.hold(2.seconds)` (number words `one` … `ten`, or digits), `FOLLOW THE BIRD.` → `camera.follow(bird)`.

**What script mode cannot do:** triggers, flags beyond the phrase table, variables and custom actions. When a story
needs reactions, put `when` rules in Core mode beside the screenplay. They interoperate because the screenplay can
set flags with phrases such as `The BIRD feels afraid.`

---

## 12. Continuity

- At the end of every scene the runtime stores each cast entity's state: rest values, authored flags, variables and
  attachments. In the next scene an entity that was also in the previous scene **starts from that state**. Use
  `(fresh)` to reset it. Derived flags are recomputed, rule memories are reset and tasks never carry over.
- `assert expr` checks a condition when it executes (`E-RUN-004` on failure) and is allowed in `setup:`:

```anim
setup:
    assert rose.opacity == 1.0
    assert not bird.feels_afraid
```

- Static checks: a scene entity must be in its `cast:` (`E-NAM-003`), and `place` may not put an entity outside the
  canvas unless `off_canvas: true` (`W-CNT-001`).

---

## 13. Shapes (implemented)

A **shape** is editable 2D geometry made of **vertices**, **edges** and **faces**, like Blender's edit mode
but in 2D. Try it:

```bash
cargo run -p anim -- check examples/flower.anim
cargo run -p anim -- render examples/flower.anim -o flower.png
```

### Creating shapes

```anim
shape sq1  = square(size: 200)                      # centred on (0, 0); add `at: (x, y)` to move it
shape box  = rectangle(width: 300, height: 120, at: (400, 300))
shape tri  = triangle(size: 150)                    # equilateral, point up
shape ring = circle(16, radius: 80)                 # an n-gon with n sides
shape poly = polygon([(0, 0), (100, 0), (50, 80)])
shape ruler = line((0, 0), (200, 0))                # two vertices, one edge, no face
shape copy1 = sq1.copy()
```

Positions are absolute canvas coordinates (origin top-left, +y down); `at:` is the centre.
Bare numbers in shape operations are pixels. Angles accept `20.degrees` or `20.deg`.

Style (applies to the whole shape; all optional):

```anim
sq1.fill = #f4a6c0
sq1.stroke = #b83a6b
sq1.stroke_width = 3
sq1.z = 1                       # drawing order, higher is in front
```

### Naming elements

```anim
sq1.v1                       # vertex 1 of shape sq1
sq1.e2                       # edge 2
sq1.f1                       # face 1
sq1.v[3]                     # indexed form, for loops and computed access
sq1.v1.move_to(0, 0)
sq1.e2.bend(20.degrees)
sq1.v[3].move_by(40, -10)
```

- `vN`, `eN`, `fN` are numbered from 1. **`v1.1` is not valid syntax.** Use `v1` or `v[1]`.
- A square starts with `v1..v4` going counter-clockwise **on screen** from the top-left (top-left, bottom-left,
  bottom-right, top-right), `e1..e4` (`eN` runs from `vN` to `vN+1`, `e4` closes back to `v1`) and `f1`.
- An **edge** is an ordered pair of vertex IDs. A **face** is an ordered loop of edge IDs.
- Reading things: `sq1.v1.x`, `sq1.v1.y`, `sq1.v1.position`, `sq1.f1.vertices` (in loop order),
  `sq1.f1.edges`, `sq1.e1.vertices`, `sq1.e1.curve`, `sq1.vertices` / `.edges` / `.faces` (all live ones).

### Stable IDs

Existing elements are **never renumbered or reused**; deleted IDs stay retired. New elements get the next unused
number. Subdividing `e1` of a square creates `v5` and edges `e5`, `e6`; `v1..v4` keep their IDs and positions.

```anim
sq1.e1.subdivide(2)
sq1.e1.children       # [e5, e6]  the same-kind elements created from e1
sq1.v5.parent         # e1        the element v5 came from
sq1.e1.created        # [v5, e5, e6]  everything created from e1
sq1.e1.alive          # false     e1 was replaced; using it in an operation is E-SHP-002
```

Optional semantic names are aliases for IDs and must be unique within the shape:

```anim
sq1.e2.subdivide(into: 3, names: ["left", "mid", "right"])
sq1.mid.bend(10.degrees)
```

### Operations

| Operation | Effect |
|---|---|
| `e.subdivide(n)` / `f.subdivide(n)` | Split an edge into `n` equal parts, or a four-sided face into an `n`×`n` grid |
| `v.move_to(x, y)`, `v.move_by(dx, dy)` | Reposition a vertex. Also accept a point: `move_to((x, y))`. On an edge, the midpoint goes to `(x, y)` and both vertices move. On a face, the centroid does |
| `e.extrude(by: d)` | Push an edge outward by `d`, creating 2 vertices, 3 edges and a face |
| `v.bevel(r)` / `v.round(r)` | Cut a corner `r` away along both edges (`round` joins the cut with a curve) |
| `s.smooth(levels)` | Chaikin subdivision: rounds a closed outline (every vertex must have exactly two edges) |
| `v1.merge(v2)` or `s.merge(v1, v2)` | Weld two vertices (`at: "first"` / `"last"` / default centre) |
| `x.delete()` or `s.delete(x)` | Remove an element; the IDs of the rest stay |
| `e.bend(angle)` | Curve an edge (|angle| < 90°; positive bulges to the right of travel, outward for a normal shape) |
| `e.set_handle(x, y)`, `e.set_handles(p1, p2)`, `e.straighten()` | Explicit quadratic / cubic Bézier handles, or back to straight |
| `e.dissolve()` | Remove an edge shared by two faces, merging them (lower face ID survives) |
| `v.dissolve()` | Remove a vertex that has exactly two straight edges, merging the edges |
| `f.inset(amount)` | Shrink a face inward, leaving a ring of new faces (the face keeps its ID as the inner face) |
| `s.mirror(x: 400)` | Add a reflected copy joined to the shape (also `y:`, `line: ((x1, y1), (x2, y2))`, `faces:`, `weld:`) |
| `s.flip(x: 400)` | Reflect the whole shape in place (same axes) |
| `s.radial_array(count: 8, around: (400, 300))` | Add rotated copies around a point, welded where they touch (also `angle:`, `faces:`, `weld:`) |
| `a.morph_to(b)` | Move every vertex of `a` to the matching vertex of `b` |

Edges are straight by default. Deleting a vertex deletes its edges and the faces using them; deleting an edge
deletes the faces using it; deleting a face deletes only the face.

### Selections and lambdas

`s.vertices`, `s.edges`, `s.faces` and `x.children` are lists of elements. Lists have selection methods, and
operations on a list apply to every element in it:

```anim
let ring = flower.f1.vertices
ring.every(2).bevel(45)                          # 1st, 3rd, 5th ...
ring.every(2, offset: 1).move_by(0, -5)          # 2nd, 4th ...
flower.vertices.where(v -> v.x > 400)            # a lambda picks by a test
flower.vertices.where((v, i) -> i % 2 == 0)      # the lambda may also take the index
flower.edges.where(e -> e.length > 100).subdivide(2)
flower.vertices.nearest((400, 300), 3).move_by(0, 10)
```

| Method | Result |
|---|---|
| `every(n, offset: 0)` | every n-th element starting at `offset` |
| `first(n)`, `last(n)`, `skip(n)`, `reverse()` | the obvious sub-lists |
| `where(v -> test)` or `where((v, i) -> test)` | elements for which the test is true |
| `nearest(point, n)` | the `n` closest elements (default 1) |
| `union(other)`, `except(other)` | set operations, order kept, no duplicates |
| `count` | the number of elements |

Elements expose `id`, `x`, `y`, `position` (a vertex, an edge's midpoint, a face's centroid), `length` (edges) and
`area` (faces) so tests can use them. Lambdas see the variables around them.

Operations that work on a whole list: `move_by`, `move_to` (moves the group so its centroid lands on the target;
a vertex shared by several selected elements moves once), `subdivide`, `extrude`, `bevel`, `round`, `dissolve`,
`inset` (face by face), `delete`, `bend`, `set_handle(s)`, `straighten`. If an earlier step of the same call
removes an element that is later in the list (deleting a vertex also removes its edges), that element is skipped.

At runtime, selections and lambdas are fine (they only read) and `list.move_by(...) over 0.3.seconds` is one
action on all of its vertices.

### Colour per face and per edge

```anim
flower.fill = #f4a6c0               # the shape's default fill
flower.f1.fill = #b9e08f            # one face
flower.faces.every(2, offset: 1).fill = #3b8ea5
flower.e2.stroke = #ff0000          # one edge (also stroke_width)
flower.e2.stroke_width = 6
flower.f1.fill = none               # back to the shape's colour
```

A face or edge without its own colour uses the shape's. New faces and edges copy the colour of the element they
came from, so an inset ring, a mirrored copy or a `radial_array` copy keeps its colour. Colours can also be set at
runtime (an instant change); fading between colours is not available yet.

### Position, scale and rotation

Every shape has a transform on top of its vertices: `position` (a shift), `scale` and `rotation`, applied around
the shape's `pivot`. The pivot is the centre of the shape's bounding box, fixed when the build phase ends; set your
own with `s.pivot = (x, y)`.

```anim
s.position = (40, 0)
s.scale = 1.5                       # or (sx, sy)
s.rotation = 45.degrees

s.move_to((100, 50))                # these four are timed with `over`
s.move_by(10, 0) over 0.5.seconds
s.scale_to(2) over 1.second ease ease_out
s.rotate_by(360.deg) over 4.seconds ease linear     # rotation is linear, so full spins work
```

Without `over` they are instant. A shape's `move_to` moves its `position`; an element's (`s.v1.move_to`) edits the
vertex itself. The transform runs beside vertex animation, and everything in a frame is resolved in a fixed order.

### Keyframes and timelines

Keys give exact values at exact times. A key set only works from its first to its last key, then lets go: the value
underneath (the rest value or a running action) shows through again.

```anim
s.v[2].keyframes over 1.second:             # a statement in main: or a rule; blocks for 1 second
    at 0.s: position.y = 0                  # relative to the start of the statement
    at 0.5.s: position.y = 100 ease ease_out
    at 1.s: position.y = 60                 # the default easing is linear

pinwheel.keyframes over 4.seconds:          # a shape: position, scale and rotation
    at 0.s: scale = 1
    at 2.s: scale = 1.25 ease ease_in_out
    at 4.s: scale = 1

s.vertices.keyframes over 1.second:         # `+=` adds an offset: good for groups
    at 0.s: position.y += 0
    at 0.5.s: position.y += 10
    at 1.s: position.y += 0
```

Options (one per line inside the block): `hold_before`, `hold_after` (keep the first or last value outside the
range), `blend_in: 12f` and `blend_out: 12f` (fade the whole set in or out). Two keys for one property at the
same time are `E-PRP-003`; a key later than the `over` time, or mixing `=` with `+=`, is `E-TYP-001`.

A scene can also list keys in absolute scene time:

```anim
scene "Sway" for 4.seconds:
    timeline:
        at 12f:
            s.v[1].position.x = 0
            s.rotation = 0.deg
        at 2.seconds:
            s.v[1].position.x = 100 ease linear
            s.rotation = 90.deg
```

### Colour animation

Colours blend in linear light, which is how light really mixes (a black to white fade is `#bcbcbc` at the
middle, brighter than a plain `#808080`). The result is rounded to 8 bits.

```anim
s.f1.fill_to(#ff0000) over 1.second ease ease_out      # one face
s.faces.fill_to(#ffffff) over 1.second                 # a selection: one action
s.e1.stroke_to(#00ff00) over 1.second
s.e1.width_to(6) over 1.second
s.fill_to(#101010) over 1.second                       # the shape's colour (faces without their own follow)
s.fade_to(0.5) over 1.second                           # opacity: 0 invisible, 1 solid (covers strokes too)
s.tint_to(#ff8800) over 1.second                       # multiplies every colour; white = no change
s.opacity = 0.5                                        # or set them directly
s.tint = #ff8800
```

A colour that is not set yet fades in from transparent without a colour shift. Colours can be keyed too:
`at 1.s: fill = #ff0000`, `stroke`, `stroke_width`, `opacity`, `tint` (but not with `+=`, except
`stroke_width` and `opacity`).

### Procedural modifiers

A modifier adds motion on top of whatever else is happening (layer 2: above actions, below keyframes).

```anim
s.vertices.wobble(amount: 4, speed: 1.5) over 3.seconds    # a sine sway; every vertex has its own phase
s.v[2].wobble(amount: 10, axis: y, sync: true, phase: 0) over 2.seconds
s.vertices.noise(amount: 3, speed: 2) over 3.seconds       # smooth random drift
flower.wobble(property: scale, amount: 0.05, speed: 2) over 4.seconds
flower.noise(property: rotation, amount: 6.deg) over 4.seconds   # also position and opacity
```

`amount` is in pixels for positions, an Angle for `rotation`, a plain number for `scale` and `opacity`.
`speed` is cycles per second (wobble) or noise steps per second. `axis: x | y | both` picks the direction,
`sync: true` makes a whole selection move together, and `blend_in: 0.5.seconds` / `blend_out:` fade a modifier in
and out. Randomness is seeded by the project `seed`, and adding another modifier elsewhere never changes an
existing one. `noise("name", t)` and `random("name", i)` give the same kind of numbers directly.

### Springs and physics

```anim
s.move_by(100, 0) over 1.second ease spring(2, 4)        # overshoots, then lands exactly (bounces, damping)
s.v[1].fall(floor: 400, gravity: 900, bounce: 0.5) over 3.seconds
ball.fall(floor: 300, bounce: 0.6, velocity: (40, 0)) over 4.seconds     # a shape: floor is its position.y
s.v[1].bounce_in(rect(-60, -100, 120, 200), velocity: (60, 40)) over 4.seconds
```

`fall` and `bounce_in` are actions: the thing ends up where it stopped. They know fixed floors and walls only;
bodies do not collide with each other.

### start, cancel and await

`over` blocks until the thing finishes. To keep going, or to run something open-ended, use `start`:

```anim
let h = start s.vertices.wobble(amount: 3)          # no `over`: runs until cancelled or the scene ends
let r = start s.rotate_by(360.deg) over 4.seconds   # an action that runs while the script continues
wait 2.seconds
cancel h                                            # a modifier stops; an action keeps its last value
await r                                             # wait for the rotation to finish
```

### Two modes

1. **Authoring-time** operations change topology (`subdivide`, `extrude`, `bevel`, `round`, `smooth`, `merge`,
   `delete`, `bend` and the handle operations). They run at the top level, in a `shape` build block or in a
   scene's `setup:`, are deterministic, and finish before animation starts.

   ```anim
   shape flower = square(size: 200):      # a build block
       flower.e1.subdivide(4)
   ```

2. **Runtime animation** only moves vertices. Vertex positions are ordinary animatable values, so easing,
   `together:` and rules all work:

   ```anim
   when bird.is_scared:
       cloud.v[2].move_by(0, 10) over 0.3.seconds ease ease_out
   ```

   Two moves of the same vertex run one after the other; moves of different vertices run together.
   Without `over`, a move is an instant, zero-time write.

Morphing needs equal vertex counts, otherwise `E-SHP-001`
("cannot morph: 4 vs 12 vertices; subdivide `sq1` first").

### Language used by the examples

```anim
for i in 1..4:                  # inclusive range: 1, 2, 3, 4
    flower.e[i].subdivide(4)
let ring = flower.f1.vertices   # lists are indexed from 0: ring[0]
if i % 2 == 0:
    ...
else:
    ...
let c = (400, 300)
let p = c + unit_vector(225.degrees) * 150     # points support + - and * by a number
```

Helpers: `unit_vector(angle)`, `direction_to(a, b)`, `distance(a, b)`, `center(shape)`, `min`, `max`, `abs`, `clamp`,
`lerp`, `floor`, `ceil`, `round`, `sqrt`, `sin`, `cos`, `len`, `frames(d)`.

### Shape errors

| Code | Meaning |
|---|---|
| `E-SHP-001` | Morph vertex-count mismatch |
| `E-SHP-002` | Unknown vertex/edge/face ID, or one that was deleted or replaced |
| `E-SHP-003` | Operation not allowed at runtime (topology change after setup); also caught by `anim check` |
| `E-SHP-004` | Duplicate semantic name |
| `E-RUN-006` | `await` on a modifier that never ends |
| `E-SHP-005` | Invalid argument or operation not applicable (e.g. bevel radius longer than the edge, dissolving an edge that is not between two faces, inset too large) |
| `W-SHP-010` | Degenerate face (zero area) or self-intersection after an edit |

### Worked example: a square becomes a flower

This is `examples/flower.anim`; the picture is `examples/flower.png`.

```anim
# 1. Start from a square (v1..v4, e1..e4, f1).
shape flower = square(size: 200, at: (400, 300))

# 2. Cut every side into four. IDs never change, so a loop over e1..e4 is safe.
for i in 1..4:
    flower.e[i].subdivide(4)

# 3. Push the ring of 16 vertices out and in alternately.
let c = (400, 300)
let ring = flower.f1.vertices          # in loop order, starting at v1
for i in 0..15:
    let a = 225.degrees - i * 22.5.degrees
    if i % 2 == 0:
        ring[i].move_to(c + unit_vector(a) * 230)
    else:
        ring[i].move_to(c + unit_vector(a) * 110)

# 4. Bevel the eight tips: every second vertex of the ring.
ring.every(2).bevel(45)

# 5. Round the whole outline (Chaikin subdivision, twice).
flower.smooth(2)
```

### Worked example: a leaf from half an outline

`examples/leaf.anim` (picture: `examples/leaf.png`) uses `mirror`, `dissolve`, `smooth` and `inset`:

```anim
# 1. Draw only the right half. v1 and v5 sit on the axis x = 400.
shape leaf = polygon([(400, 60), (445, 150), (545, 300), (445, 450), (400, 540)])

# 2. Add the mirror image. Vertices on the axis are shared, so the halves are glued together.
leaf.mirror(x: 400)

# 3. The seam (e5) is now inside the shape; dissolving it merges the two faces into one outline.
leaf.e5.dissolve()

# 4. Round the outline (Chaikin, twice).
leaf.smooth(2)

# 5. Inset the face: an inner leaf plus a ring of border faces.
leaf.f1.inset(10)
```

### Worked example: a pinwheel

`examples/pinwheel.anim` uses `radial_array`, per-face colour, a shape transform and keyframes:

```anim
shape pinwheel = polygon([(400, 300), (600, 300), (400 + 200 * cos(45.deg), 300 + 200 * sin(45.deg))])
pinwheel.e2.bend(-25.degrees)
pinwheel.f1.fill = #e4572e
pinwheel.radial_array(count: 8, around: (400, 300), angle: 360.deg)
pinwheel.faces.every(2, offset: 1).fill = #3b8ea5

scene "Spin" for 4.seconds:
    main:
        together:
            pinwheel.rotate_by(360.deg) over 4.seconds ease linear
            pinwheel.keyframes over 4.seconds:
                at 0.s: scale = 1
                at 2.s: scale = 1.25 ease ease_in_out
                at 4.s: scale = 1 ease ease_in_out
```

### Worked example: a breathing flower

`examples/breathing.anim` (frames: `examples/breathing_t0.png`, `examples/breathing_t72.png`) combines them: every
petal vertex wobbles, the whole flower drifts with noise, its colour shifts, its centre fades, it pops with a
spring, and a seed falls and bounces.

```anim
scene "Breathe" for 5.seconds:
    main:
        start flower.vertices.wobble(amount: 2, speed: 0.6)
        start flower.noise(property: rotation, amount: 5.deg, speed: 1)
        start flower.fill_to(#ffd1dc) over 5.seconds ease ease_in_out
        start heart.fade_to(0.55) over 2.5.seconds ease ease_in_out
        start seed.fall(floor: 420, gravity: 900, bounce: 0.5) over 4.seconds
        flower.scale_to(1.15) over 1.second ease spring(2, 5)
        flower.scale_to(1.0) over 1.second ease ease_in_out
        wait 3.seconds
```

---

## 13b. Physics (rigid bodies, joints and springs)

A shape becomes a physical body with `.body()`. Gravity, collisions, friction, bounce, sleeping and joints
are simulated deterministically, so the same program always gives the same frames.

```anim
world:
    gravity: (0, 9.8)     # metres per second squared, y points down (the default)
    scale: 100            # pixels per metre (the default)
    substeps: 4           # engine steps per frame (the default)

shape floor = rectangle(width: 760, height: 30, at: (400, 570))
shape ball  = circle(20, radius: 26, at: (170, 250))
floor.body(kind: static)
ball.body(restitution: 0.55, friction: 0.3)
```

`world:` is optional (the values above are the defaults). Write it once, at the top level, after `project`.

### Bodies

`shape.body(...)` takes these named arguments (all optional):

| Argument | Meaning | Default |
|---|---|---|
| `kind` | `static` (never moves), `dynamic` (simulated), `kinematic` (follows your animation and pushes dynamic bodies) | `dynamic` |
| `density` | kilograms per square metre; a 100 px square of density 1 weighs 1 kg | `1` |
| `friction` | 0 is ice; two surfaces use the geometric mean | `0.6` |
| `restitution` | bounciness, 0 to 1; two surfaces use the larger | `0` |
| `linear_damping`, `angular_damping` | slows motion down, like air drag | `0` |
| `gravity_scale` | `0` floats, `-1` rises | `1` |
| `fixed_rotation` | `true` stops it from turning | `false` |
| `sensor` | `true` reports overlap but never pushes | `false` |

Every face of the shape collides, so a concave face (an L, a star) works: it is split into convex pieces for
you. A moving body needs area; a static body may be a bare line of edges (a wall). A body's `scale` must be
uniform and is fixed when the scene starts.

Make bodies at the top level or in `setup:`, never in `main:` (`E-SHP-003`).

### The pose is the shape's own

A dynamic body drives the shape's `position` and `rotation`. Actions on those properties lose to physics;
keyframes and `timeline:` win (the body keeps simulating underneath). A **kinematic** body is moved by your
actions and keyframes, and carries or pushes dynamic bodies:

```anim
platform.body(kind: kinematic, friction: 1.0)
scene "ride" for 2.seconds:
    main:
        platform.move_by(200, 0) over 2.seconds ease linear
```

Each scene starts the world again from the shapes' current poses with zero velocity, and leaves them where
they ended.

### Reading and nudging a body

```anim
ball.set_velocity((120, 0))     # px per second
ball.apply_impulse((0, -300))   # kg * px / s: a 1 kg body gains 300 px/s
let v = ball.velocity           # (x, y) in px/s;  v.x, v.y
ball.speed                      # px/s
ball.awake                      # false once it has come to rest
ball.mass                       # kilograms
```

Before the scene runs `velocity` and `speed` read 0. Calls in `setup:` set the starting motion.

### Reacting to contact

```anim
scene "s" for 6.seconds:
    when ball.touches(ground):          # fires once, when they meet
        ball.fill_to(#ffb347) over 6.frames
    when ball.leaves(ground):           # an event, true for one tick
        ...
```

`a.touches(b)` is true while the two are in contact; like all runtime state it is seen one tick after it
happens.

### Joints and springs

```anim
let hinge = revolute(door, frame, anchor: (200, 100), limits: (-60.deg, 60.deg), motor: (90.deg, 50))
let s = distance(a, b, length: 120, spring: (4, 0.5))      # rest length, (frequency Hz, damping ratio)
let r = rope(a, b, max: 150)                               # slack until it is taut
let rail = prismatic(slider, track, axis: (1, 0), limits: (0, 200))   # slides, limits in px
let w = weld(a, b, anchor: (x, y))                         # glued together
let f = follow(ball, target: (300, 200), force: 500, spring: (5, 0.7))
f.set_target((100, 100))
```

- `anchor` is where the joint sits in the world at the start. `distance` and `rope` default to the bodies'
  centres (`anchor_a:`, `anchor_b:` choose other points); a `distance` without `spring` is a rigid rod.
- Joined bodies do not collide with each other unless you add `collide: true`.
- An angle `limits` needs angles (`60.deg`); `motor` is `(speed, torque)` with the speed an angle per second.
- `distance(a, b)` with two *shapes* is the joint; with two points it is still the distance between points.

### Looking at what the engine does

```bash
anim render scene.anim -o frame.png --tick 36 --debug-physics
anim frames scene.anim -o frames/ --debug-physics     # every tick, simulated once: f0000.png ...
```

draws contact points with their normals (red) and joints (blue). Snapshots carry a `physics` section (bodies
and whether they are awake, touching contacts, joint anchors); it is part of the frame hash.

New errors: `E-PHY-001` (a body that cannot exist), `E-PHY-002` (bad joint arguments), warning `W-PHY-001`
(bodies overlap at the start of a scene). Worked example: `examples/physics.anim`.

## 13c. Particles

An **emitter** throws many small seeded circles: sparks, dust, water, confetti. Declare it once at the top
level; control it with `start()`, `stop()` and `burst(n)`.

```anim
emitter fountain:
    at: (150, 555)                  # a point, or a shape (born inside it, following it)
    rate: 90                        # particles per second while started
    lifetime: (1.2, 1.8)            # seconds, picked per particle: (shortest, longest)
    speed: (320, 420)               # px/s
    angle: (-105.deg, -75.deg)      # 0 points right; -90.deg is up
    gravity: (0, 600)               # px/s^2
    size: (3, 4.5)                  # radius in px at birth ...
    size_end: 1                     # ... and at the end of its life
    color: #8fd0ff
    color_end: #3d7bd9
    opacity: 1
    opacity_end: 0.2
    collide: true                   # bounce off bodies
    restitution: 0.35

scene "s" for 6.seconds:
    whenever ball.touches(ground):
        sparks.burst(36)            # exactly 36, at once
    main:
        fountain.start()
        wait 3.seconds
        fountain.stop()             # the ones in the air finish their life
```

| Setting | Meaning | Default |
|---|---|---|
| `at` | a point `(x, y)` or a shape | `(0, 0)` |
| `rate` | particles per second while started (0 = only bursts) | `0` |
| `lifetime` | seconds, number or `(min, max)` | `1` |
| `speed`, `angle` | number or `(min, max)` pair | `0`, a full circle |
| `size`, `size_end` | radius in px; end value over the life | `3`, same as `size` |
| `color`, `color_end` | colour over the life (in linear light) | white |
| `opacity`, `opacity_end` | 0 to 1 over the life | `1` |
| `gravity`, `drag` | px/s^2 vector; slows down like air | none |
| `collide`, `restitution` | bounce off physics bodies | `false`, `0.4` |
| `max` | most live particles (all emitters together: 20000) | `1000` |

- Everything random is seeded: the same program (and project `seed:`) always draws the same particles.
- Particles bounce off bodies but never push them, and they do not collide with each other.
- `name.count` is the number of live particles.
- Emitters start stopped at the start of every scene.
- Errors: `E-PTC-001` (bad setting), `E-PTC-002` (`at` is not a point or shape). Example: `examples/particles.anim`.

## 13d. Soft bodies

`shape.soft(...)` makes a shape squash, stretch, swing and drape. The shape's vertices become simulated
points, its edges springs and its faces optional "air" that keeps the volume.

```anim
shape jelly = grid(4, width: 110, height: 110, at: (130, 90))
jelly.soft(stiffness: 0.55, pressure: 1.0, restitution: 0.3)      # a jelly cube

shape rope = line((470, 50), (650, 50))
rope.e1.subdivide(14)                  # more vertices = a smoother, softer rope
rope.soft(stiffness: 1.0, bend: 0.4)
rope.pin(rope.v[1])                    # hold one end where it is

shape flag = grid(9, width: 150, height: 100, at: (700, 230))
flag.soft(stiffness: 0.45, bend: 0.04)
flag.pin(flag.vertices.where(v -> v.x < 626))    # pin a whole column with a selection
flag.soft_impulse((600, -150))                     # a puff of wind (kg*px/s)
```

| Argument | Meaning | Default |
|---|---|---|
| `stiffness` | above 0, at most 1: how hard it resists stretching (1 = rigid) | `0.8` |
| `bend` | 0 to 1: resistance to folding sharply (ropes, cloth) | `0.3` |
| `pressure` | 1 keeps the volume, above 1 inflates, 0 ignores volume | `1` |
| `density` | kg per square metre (edges with no face: per 5 cm of width) | `1` |
| `friction`, `restitution` | against bodies, like rigid bodies | `0.5`, `0.1` |
| `damping` | slows motion down | `0.1` |
| `thickness` | collision size of a vertex, px | `2` |
| `self_collide` | let the shape's edges stop its own vertices | `false` |

- **Pins:** `pin(vertex)` or `pin(list)` hold vertices in place; `pin(v, to: other_shape)` makes them follow
  that shape (a kinematic platform, an animated or falling shape), so it can carry a rope or a cloth. `unpin(v)` lets go (also while running).
- **Collision:** soft shapes collide with rigid bodies (`.body()`) and with each other. They rest on them,
  bounce, and push dynamic bodies (a soft pad supports a box). Obstacles thinner than the vertex spacing are
  still felt (edges are sampled finely near them), and a fully pinned soft body is an obstacle too.
- **A shape is either rigid or soft:** `.body()` and `.soft()` on the same shape is `E-PHY-003`.
- **Moving it:** moving the shape's own `position`/`rotation` (an action or keyframes) carries the whole
  simulation with it; it is a teleport, so it will push things out of the way. To drag a soft body gently, pin
  vertices to a moving shape. Keyframes on a single vertex still win.
- **Reading it:** `shape.volume` (px^2) and `shape.stretch` (largest edge strain, 0 means none).
- `grid(cells, width:, height:, at:)` makes a `cells` x `cells` grid of square faces.
- In `world:` the settings `soft_substeps` (16) and `soft_iterations` (4) trade speed for accuracy.
- Errors: `E-PHY-003`, `E-PHY-004`. Worked example: `examples/soft.anim`.

## 13e. Fluids

A **fluid** is thousands of small particles that pour, splash, pool and slosh like water. It collides with
rigid bodies and pushes them (a light box floats, a heavy one sinks, a ball splashes).

```anim
shape floor = rectangle(width: 540, height: 30, at: (400, 575))
shape wall_l = rectangle(width: 20, height: 340, at: (150, 400))
shape wall_r = rectangle(width: 20, height: 340, at: (650, 400))
floor.body(kind: static)
wall_l.body(kind: static)
wall_r.body(kind: static)

fluid water:
    fill: rect(170, 450, 460, 110)     # a block of water, in px (or a shape: its faces are filled)
    spacing: 5                         # distance between particles in px: smaller = finer and slower
    smoothness: 1.3                    # a crisper surface
    foam: 0.8                          # spray and foam where it splashes
    viscosity: 0.05                    # 0 = water, higher = syrup (0 to 1)
    color: #62b0f0dd                   # at the surface (alpha allowed) ...
    color_end: #1f4f9add               # ... and deep down

scene "s" for 7.seconds:
    main:
        water.pour(at: (230, 40), speed: (60, 260), width: 30)    # a jet of water
        wait 2.seconds
        water.stop()
        water.add(rect(300, 200, 60, 60))                           # drop in a block at once
```

| Setting | Meaning | Default |
|---|---|---|
| `fill` / `in` | starting region: `rect(x, y, w, h)` or a shape | none (dry) |
| `spacing` | particle distance, px | `8` |
| `density` | kg per square metre | `1` |
| `viscosity` | 0 to 1 | `0.05` |
| `surface_tension` | pull between neighbours (droplets) | `0` |
| `foam` | 0 to 1: foam and spray thrown where fast water breaks the surface | `0` |
| `smoothness` | 0.8 to 4: radius of the blob surface in spacings (lower = crisper) | `1.4` |
| `color`, `color_end` | surface and deep colour, alpha allowed | light blue |
| `render` | `"blob"` (smooth surface) or `"dots"` | `"blob"` |
| `behind` | draw under the shapes instead of over them | `false` |
| `max` | most particles (all fluids together: 20000) | `4000` |

- **Containers** are ordinary static bodies; `fill` only chooses where the water starts. Water that leaves the
  area around the canvas is removed.
- `pour(at:, speed:, width:)` makes a continuous stream (`rate:` overrides the automatic particle rate);
  `stop()` ends the streams; `add(region)` adds a block; `water.level(x)` is the surface height at `x` (px);
  `water.count` is the number of particles.
- **Bodies:** dynamic bodies are pushed and buoyed by water; give a body `density` below 1 to float it. Soft
  shapes are obstacles for water but are not pushed by it.
- **Foam and spray** are decorative: they never push the water or anything else, they are seeded (the same
  program always throws the same foam) and only the surface produces them.
- **Cost:** a few thousand particles is normal; halving `spacing` is four times the particles and cost. Render
  many frames at once with `anim frames` (it simulates once).
- Each scene starts again from the `fill` region. In `world:` the settings `fluid_substeps` (4) and
  `fluid_iterations` (3) trade speed for accuracy.
- Errors: `E-FLD-001` (bad setting, over the particle cap), `E-FLD-002` (`fill` is not a region). Example:
  `examples/fluid.anim`.

## 13f. Pictures on shapes (sprites)

A shape can be filled with a PNG picture. The picture is mapped onto the shape's faces by one texture
coordinate per vertex, so it bends, stretches and moves with the shape: a sprite on a soft cloth waves, a
sprite on a rigid body bounces.

```anim
shape flag = grid(12, width: 240, height: 160, at: (560, 150))
flag.fill = image("assets/flag.png")       # a path relative to this file's folder
flag.stroke = none
flag.soft(stiffness: 0.5)                   # the picture now follows the simulation

shape face = circle(24, radius: 60, at: (200, 120))
face.fill = image("assets/face.png")        # transparent pixels stay transparent
face.body(restitution: 0.75)
```

- `image("file.png")` reads the file when the program is built. A missing file is `E-AST-001`; a file that is
  not a PNG is `E-AST-002`. Only PNG is supported (all bit depths, with or without transparency).
- The picture covers the shape's bounding box when you assign it; each vertex remembers where it was in
  that box. More vertices (`subdivide`, `grid`) give smoother bends: a face is drawn as triangles.
- `shape.fill = #336699` after an image keeps the image (the colour is drawn behind it);
  `shape.image = none` removes the picture. `opacity` applies to the picture.
- Changing the picture file changes the frame hashes (the file's bytes are part of them).
- `anim render` and `anim frames` find the pictures next to the program file.
- Not yet: fitting without stretching, tiling, a separate UV editor, and per-face pictures.
- Worked example: `examples/sprite.anim`.

## 13g. Armatures: bones, poses, IK and skinning

An **armature** is a skeleton of bones. Bones are invisible nodes; you attach art (shapes or pictures) to
them, and animating the bones moves the art.

```anim
armature hero:
    bone torso:    at: (400, 400), length: 120, angle: -90.deg              # the root: a point and a direction
    bone head:     parent: torso, length: 36, limits: (-40.deg, 40.deg)     # hangs off the torso's tip
    bone arm_up:   parent: torso, at: 0.92, length: 70, angle: 90.deg       # 92% up the torso, sticking out right
    bone arm_lo:   parent: arm_up, length: 62, limits: (-150.deg, 0.deg)
    ik reach: chain: [arm_up, arm_lo]                                       # an IK chain (two or more bones)

body.bind(hero.torso)                          # the whole shape follows a bone
arm.f1.subdivide(4)
arm.skin(hero, bones: [hero.arm_up, hero.arm_lo], blend: 10)    # every vertex follows several bones

scene "s" for 4.seconds:
    main:
        hero.head.rotate_to(25.deg) over 0.5.seconds       # a bone angle, like any rotation
        hero.reach.reach((505, 215)) over 1.second          # IK: the hand goes to a point
        hero.reach.release() over 0.5.seconds               # IK off: back to the bone angles
        hero.arm_up.keyframes over 1.second:                # a clip is keyframes on bones
            at 0.s: rotation = 0.deg
            at 0.5.s: rotation = -60.deg
        hero.torso.move_by(120, 0) over 1.5.seconds          # moving the root moves the whole skeleton
```

- **Bones:** the root has `at: (x, y)` and an absolute `angle` (clockwise from +x); every other bone has a
  `parent`, `at` (0 to 1 along the parent, default the tip) and an `angle` relative to the parent (default 0,
  continuing straight). `length` is in px. List parents before their children. `limits` bound the pose
  rotation; going past them is clamped (warning `W-RIG-001`).
- **`hero.bone` is a shape-like handle:** `rotate_to`, `rotate_by`, `keyframes`, `timeline:`, `wobble`, easing
  and `start`/`await` all work on it. Its `rotation` is relative to its rest pose and its parent.
- **IK:** `chain.reach(point)` aims the end of the chain at a point (`over`, `ease` work); `release()` hands the
  bones back to their angles. Two-bone chains are solved exactly (`bend: positive|negative` picks the elbow
  side, and the other side is used when a limit blocks it); longer chains (tails, spines) use FABRIK. A point
  out of reach stretches the chain toward it.
- **`bind` and `skin`:** draw the art matching the bones at rest. `bind` makes a shape follow one bone;
  `skin` blends each vertex between bones by distance (`blend` px of falloff); `art.set_weight(vertices, bone,
  w)` paints. More vertices (`subdivide`, `grid`) bend more smoothly. A rig-driven shape cannot also be a
  physics body (`E-RIG-003`). Pictures work too: skin a `fill = image(...)` mesh to bend a sprite.
- Keyframes on the art itself still win over the rig. Scene end keeps the final bone pose as the next scene's
  start.
- Errors: `E-RIG-001` (bad bone or armature), `E-RIG-002` (bad limits or IK chain), `E-RIG-003` (bad bind).
  `--debug-physics` draws the bones. Worked example: `examples/rig.anim`.

## 13h. Ragdolls, active ragdolls and the character controller

**Ragdoll.** Add a `ragdoll:` line to an armature and every bone also gets a physics body (a capsule: a box
with a round end at each end) joined to its parent by a hinge that carries the bone's `limits`. Bones of one
armature never collide with each other; they do collide with the world's bodies.

```anim
armature hero:
    bone torso: at: (200, 465), length: 120, angle: -90.deg
    bone head:  parent: torso, length: 36, limits: (-40.deg, 40.deg), radius: 14   # own capsule radius
    ragdoll: radius: 9, density: 1, friction: 0.8, restitution: 0                  # all optional

hero.ragdoll()                          # limp: the bones are dynamic and keep their motion
hero.ragdoll(push: (-120, -90))         # ... with an extra velocity (px/s) on every bone
hero.ragdoll(strength: 0.9)             # active ragdoll: muscles pull the joints towards the animated pose
hero.animate()                          # back to the animation (the bones snap to the animated pose)
hero.ragdolled                          # true while the bones belong to physics
```

- **Animated** (the start of every scene): the bodies are kinematic and follow the pose that keyframes, actions
  and IK produce, so an animated bone can push things.
- **Limp** (`strength: 0`): dynamic bodies, free hinges within their limits, the root bone falls like any body.
- **Active** (`strength` 0 to 1): every hinge gets a muscle (an angle spring) aimed at the animated pose, with a
  strength of up to eight times what holds the limb up against gravity, and the root bone's velocity is blended
  towards its animated one. At 1 the pose is followed firmly, a small value sags. Animate the bones as usual
  (`hero.arm.rotate_to(...)`) and the muscles chase the result.
- While physical, bound and skinned art, `hero.bone.head`, `tip`, `angle` and the snapshot's `rigs` come from the
  bodies. The ragdoll state starts over (animated) with every scene. `ragdoll()`/`animate()` run during a scene.
- `body()` on a bone is still `E-RIG-003`; the `ragdoll:` line is how a bone gets a body.
- Errors: `E-RIG-004` (no `ragdoll:` line, used outside a running scene, bad `strength`, bad ragdoll value).

**Character controller.** Steer a dynamic body like a character: it does not tip over and ignores friction
with walls.

```anim
runner.body()
runner.controller(speed: 220, accel: 1500, jump: 480, air_control: 0.5)

runner.move(1)         # walk right at `speed` (px/s); -1 left; 0 stop; stays until changed
runner.jump()          # leaves the ground at `jump` px/s, only when grounded (no double jump)
runner.grounded        # true while something solid is under its feet
```

`speed` is the top walking speed, `accel` (px/s^2) how fast the velocity changes on the ground and `air_control`
the fraction of it in the air. `move` and `jump` are used while a scene runs. Errors: `E-PHY-005` (a controller on
a body that is not dynamic, a bad value, `move`/`jump` without a controller or outside a running scene).

Because expressions read the current animated state (section 8), a chasing behaviour is a plain action:

```anim
action run_to(self: Shape, x: Number):
    while abs(center(self).x - x) > 8:
        if center(self).x < x:
            self.move(1)
        else:
            self.move(-1)
        wait 1.frame
    self.move(0)
```

Worked example: `examples/ragdoll.anim`.

## 13i. Walking: raycasts, IK reads and `std/gait`

**`raycast(origin, direction, max, ignore:)`** returns the first solid point along a ray (a point, or `none`),
`raycast_normal(...)` the surface normal there. Sensors are skipped and a ray that starts inside a shape does
not hit it. `ignore:` takes a shape, an armature, the word `bones` (every armature: the ground, not the
characters) or a list of them. It sees bodies, so it works while a scene runs (otherwise `none`). A zero
direction or a negative `max` is `E-PHY-006`.

```anim
let hit = raycast((x, 0), (0, 1), 1000, ignore: "bones")     # the ground below x
if hit != none:
    foot.reach(hit)
```

**IK handles can be read:** `hero.foot.target` (where it is reaching, in the world) and `hero.foot.weight`.
Together with bone `head`/`tip` and the live reads of section 8 this is everything a gait needs.
**Actions can take an armature:** `action wave(self: Armature, ...)`, called `hero.wave()`.

**`import "std/gait"`** is a gait written in AnimScript (read `crates/lang/lib/gait.anim`; copy and change it):

```anim
hero.walk_to(500, speed: 120, step_time: 0.3.seconds, lift: 16, crouch: 6)   # a biped
dog.trot_to(470, speed: 120)                                                  # a quadruped
```

- `walk_to` expects the root (hip) bone `torso` and IK handles `foot_l`, `foot_r`; `trot_to` expects `torso` and
  `foot_fl`, `foot_fr`, `foot_bl`, `foot_br`. Legs are two-bone IK chains; use `bend: negative` so knees point
  forward when walking right.
- The hips move at `speed` and ride at a fixed height above the average height of the feet, less `crouch`
  while moving. Each foot in turn swings (over `step_time`, lifted by `lift` px) to a point ahead of the hips where
  `raycast` finds the ground, then stays planted, since an IK target is a point in the world, while the hips pass.
  The biped alternates feet; the quadruped steps diagonal pairs together. The walk ends standing.
- The lower-level actions (`gait_swing`, `gait_foot`, `gait_root`, `gait_rise`, `gait_pause`) are public: build a
  different gait (a hop, a crawl, more legs) from them.
- A walk is an ordinary action: `start` it, `cancel` it, put it in `together:`. To knock a walker over, `cancel`
  the walk, `ragdoll()` it, and later `animate()` it, which returns to where its animated pose was.

Worked example: `examples/gait.anim` (a biped and a dog over stairs and a ramp, with a knock-over).

## 13j. Control flow added in round L1: `race`, `finally`, `derive`, `cubic_bezier`, markers

```anim
race:                                  # starts every child; resumes when the FIRST finishes, cancels the rest
    runner.chase()
    wait 3.seconds                     # a timeout

action chase(self: Shape):
    self.move_to((700, 150)) over 4.seconds ease cubic_bezier(0.25, 0.1, 0.25, 1)
    finally:                           # runs when this block ends: normally, `return`, cancel, a lost race, scene end
        light.fill = #aaaaaa

prop bird:
    states: afraid
    derive scared_long when afraid.held_for(1.second)     # a flag recomputed every delta
    derive startled    when afraid.since(0.25.seconds)
    derive fresh       when afraid.rose                   # became true in this delta

scene "s" for 8.seconds:
    timeline:
        marker "alarm" at 2.seconds                       # an event at scene time
    when marker("alarm"):
        bird becomes afraid
```

- **`race:`** cancels the losers like `cancel` does: an action keeps its last sampled value. A tie goes to the
  child written first.
- **`finally:`** is the last statement of a block (`main:`, a rule body, an action body, any nested block) and holds
  zero-time statements only (`E-STM-003` if it waits or runs a timed action; `E-SYN-001` if it is not last or is
  inside `together:`/`race:`). It sees the variables of the task at the moment it runs.
- **`derive name when condition`** goes in an entity (`prop`, `character`). `self` is the entity and bare names are
  its own flags; other entities are written `other.flag`. `becomes`/`clears` on it is `E-STA-001`, a cycle is
  `E-STA-002`, a name that is already a flag is `E-NAM-002`. Predicates on a flag: `x.held_for(d)` (continuously true
  for at least `d`), `x.since(d)` (became true at most `d` ago), `x.rose` (became true in this delta).
- **`cubic_bezier(x1, y1, x2, y2)`** is a CSS-compatible easing; the x values must be in 0..1, the y values may
  overshoot.
- **`marker "name" at TIME`** (in `timeline:`) raises an event on that tick; rules use `when marker("name"):`.
  Marker names are unique in a scene (`E-NAM-002`).

Worked example: `examples/control.anim`.

## 13k. Locks, parenting, constraints and channels (round L2)

All of these are statements for a **running scene** (`main:`, a rule or an action; elsewhere they are `E-STM-001`).
`lock`, `attach` and `constrain` can be given a name (`let h = lock ...`) and ended with `release h`.

```anim
lock lamp.position.y = 0                  # beats actions, keyframes, modifiers and physics (layer 4)
lock lamp.position = (10, 20) for 2.seconds     # releases itself
let h = lock shape.rotation = 90.deg
release h

attach planet to sun                      # the child follows the parent's translation and rotation
attach moon to planet                     # chains work, parents are solved first
attach hat to hero.head offset (0, -30)   # a bone can be the parent; the offset is in the parent's frame
constrain ball.position inside rect(100, 410, 200, 120)    # or `canvas`; the whole shape stays inside
constrain tail.position to follow(head, lag: 6.frames) weight 0.8
```

- **Locks** hold a component to a constant value from the moment of the statement. Two locks on one component at
  the same time are `E-PRP-002`. A locked physics body keeps simulating underneath; only what you see is held.
- **`attach`** applies the parent's rigid motion since the moment of attaching (its pivot's translation and its
  rotation about that pivot); the parent's scale is ignored. Without `offset` the child stays where it is. With it,
  the child's centre moves to the parent's anchor (a shape's pivot, a bone's head) plus the offset. A cycle is
  `E-CON-001`. `release` lets the child go back to its own placement.
- **Constraints** run after the layers and after parenting, in the order they were made. `inside` keeps the whole
  bounding box in a rectangle (centred if it is bigger). `follow` places the shape where the target was `lag`
  frames ago plus the offset the two had when the constraint was made. `weight` (0 to 1) blends it in.
- **Channels.** An action that starts with `uses body, gaze on_conflict interrupt` takes those named channels of
  its `self` shape for as long as the call runs, all at once. If one is busy: `queue` (the default) waits, in
  order of priority, `interrupt` cancels the occupant (its values are kept and its `finally:` runs) when its
  priority is at least the occupant's and otherwise waits, `reject` is `E-RUN-002`, `share` runs alongside.
  An action's priority is its `priority N` line, else the priority of the rule that started it. If a `uses`
  line is present, shape motion and colour inside it must be on `body`: otherwise `E-CAP-003`.

Worked example: `examples/links.anim`.

## 13l. Clips, overloading and `std/2d` (round L3)

**Clips** are named keyframes of a known duration, called like an action:

```anim
clip wave(self: Shape, amount: Number = 20) over 1.second:        # self: Shape or Armature
    at 0.s:   rotation = 0.deg
    at 0.5.s: rotation = amount.deg ease ease_in_out
    at 1.s:   rotation = 0.deg

clip bend(self: Armature) over 1.second uses face:                # armature clips key bones by name
    at 0.s: upper.rotation = 0.deg
    at 1.s: upper.rotation = 90.deg

a.wave()                         # blocks for 1 second; start/together/cancel work as for any action
a.wave(amount: 40, speed: 2)     # `speed` divides every key time; every clip also takes `loop: true`
let h = start a.wave(loop: true) # plays until cancelled
a.play_clip("wave", speed: 2)    # by name
```

A clip is an ordinary action underneath: it takes the `body` channel of its shape (`uses face` or
`uses body, gaze on_conflict share` after `over D` picks others), so two clips on one shape take turns unless they
use different channels. Keys follow the rules of `keyframes` (a key past the duration is `E-TYP-001`). `speed` must be
above zero (`E-CAP-001`).

**One name, several `self` types.** Actions of one name may coexist when their first parameter is typed differently,
and a call picks the one that accepts the receiver (a specific type beats `Any`). If none fits, `E-CAP-001` lists
what the name is defined for. Two with the same `self` type, or a plain `fn` of the same name, are `E-DEF-001`.
`std/gait`'s `walk_to` (an armature) and `std/2d`'s (a shape) are both usable in one file.

**`import "std/2d"`** (written in AnimScript, see `crates/lang/lib/2d.anim`): `look_at(target, over, ease, facing)`,
`look_around(sweep, over)`, `step_back(from, distance, over)`, `walk_to(target, speed)`, `turn_around(over)`,
`fly_away(direction, over, irregularity)`, `fly_to(target, over, irregularity)`, `fade_in`, `fade_out`. Targets are
points (`center(thing)` to aim at a shape). The directions `up`, `down`, `left`, `right`, `up_left`, `up_right`,
`down_left`, `down_right` are built-in unit vectors. The camera actions of the spec arrive with the camera.

Worked example: `examples/clips.anim`.

## 13m. Camera and shots (round L4)

There is one camera, `camera` (built in). Its `position` is the centre of the view in canvas coordinates (an 800x600
canvas starts at `(400, 300)`), `zoom` is 1 to 8 (`1` shows the whole canvas; outside the range is `E-CAM-001`,
except through keyframes and modifiers, which are clamped) and `rotation` is an angle (a positive turn is clockwise,
so the world turns the other way). It is animated like a shape (`camera.move_to`, `scale_to`, keyframes, `noise`),
and every scene starts with the camera reset. A shape cannot be named `camera`.

With `import "std/2d"` the camera gets its actions, all on the `frame` channel (so they take turns):

```anim
camera.frame(medium, on: rose, thirds: left)      # instant set-up; add `over: 1.second` to animate it
camera.pan_to(rose, over: 1.second, ease: ease_in_out)      # a shape (its centre) or a point
camera.zoom_to(2.0, over: 1.second)
camera.hold(2.seconds)                           # occupies the channel without moving
camera.follow(bird, lag: 8.frames)               # trails the target until cancelled
camera.shake(2, over: 0.5.seconds)               # seeded and additive; shares the channel
```

Shot sizes are numbers: `extreme_close` 0.90, `close` 0.60, `medium` 0.35, `wide` 0.15 (the fraction of the frame
height the target's bounds fill) and `establishing` (the whole canvas). `thirds` takes `left`, `right`, `up` or `down`.
`bounds(shape)` is the current rectangle of a shape (members `x y w h`), `anchor(shape or point)` its centre.

```anim
scene "Garden" for 15.seconds:
    shots:
        shot "Establishing" for 2.seconds:
            camera.frame(establishing)
        cut
        shot "Rose" for 3.seconds:
            camera.frame(medium, on: rose, thirds: right, over: 1.second)
        dissolve 0.8.seconds
        shot "Follow" until marker("landed"):          # any trigger: a flag, a derived flag, a marker, ...
            camera.follow(bird, lag: 6.frames)
        fade_through #000000 1.second
        shot "End" auto:                               # ends when its body finishes
            camera.hold(2.seconds)
            when scene.ends_in(1.second):               # rules are active only while their shot is current
                camera.shake(3, over: 0.5.seconds)
```

- A shot is a camera set-up that lasts a while; **it is not a cut in time**, the world keeps running. Its body runs
  as a task; when the shot ends the task is stopped (its camera actions keep their last value) and so are the
  rules written inside it. `cut` (also the default between shots) is instant; `dissolve D` and `fade_through
  COLOR D` blend the old and the new view for `D` while the new shot is already running.
- `scene.ends_in(D)` is true for the last `D` of a scene that has a set length.
- The snapshot has `camera`, the name of the current `shot` and, during a transition, `transition {kind, t, color,
  from}`; the renderer draws the world with both cameras and cross-fades (or fades through the colour).
- A scene without `shots:` stays on the establishing view. Errors: `E-CAM-001`; a shot needs `for`, `until` or `auto`
  (`E-SYN-001`).

Worked example: `examples/shots.anim`.

## 13n. Painting with brushes (libmypaint)

Strokes are painted by **libmypaint**, the brush engine of MyPaint, so brushes behave and look like MyPaint's. A
`brush(...)` value says which brush and what to change; a `paint_layer()` shape holds the strokes.

```anim
let ink    = brush("pen", radius: 3, color: #223355)             # a bundled brush
let chalk  = brush("charcoal", radius: 7, opacity: 0.6, hardness: 0.5)
let mine   = brush("brushes/mine.myb", slow_tracking: 2)         # a .myb file (MyPaint brush format)
shape art = paint_layer(z: 5, resolution: 2)                     # declare it like a shape

art.stroke(ink, [(10, 10), (100, 40, 0.5), (200, 60)])                       # painted at once
art.stroke(ink, [(10, 10), (300, 10)]) over 2.seconds ease linear            # drawn in over time (a blocking action)
art.stroke(ink, points, speed: 400)                                          # px/s: sets the time between points
art.clear_strokes()
art.fade_to(0.5) over 1.second                                               # a layer is a shape: move, fade, rotate, z
```

- **Brushes.** Bundled: `pencil`, `pen`, `marker`, `charcoal`, `calligraphy`, `ink`, `smudge`, `eraser`, `rounded`
  (MyPaint's classic brushes, CC0). A name with `/` or ending in `.myb` is a file. Named arguments set libmypaint
  settings by name (`radius` in pixels, `opacity` for `opaque`, `color`, `hardness`, `eraser: true`, `slow_tracking`,
  and any other setting of `brushsettings.json`). Unknown brushes, settings or files are `E-BRU-001`.
- **Points** are `(x, y)`, `(x, y, pressure)` or `(x, y, pressure, seconds)`: pressure is 0 to 1 (default 0.7) and the
  seconds since the previous point default to the distance divided by `speed` (300 px/s). Speed-dependent brushes
  react to it. Bad points are `E-BRU-002`.
- **Drawing in.** A stroke with `over` starts empty and its *progress* (0 to 1, along its length) animates like any
  property; one without `over` is complete at once. Strokes have channels of their own, so `together:` draws several
  strokes at the same time.
- **The layer** is a shape with no geometry: position, scale, rotation (about the canvas centre), opacity, `z` and
  visibility work as for shapes, and the camera applies. Order with other shapes is by `z`. `resolution` (0.25 to 4)
  paints on more pixels per canvas pixel, so a zoomed picture stays crisp.
- **How it renders.** The snapshot carries the brushes and the strokes with their progress, never pixels; the
  renderer replays every stroke with a fresh libmypaint brush onto a transparent picture each frame and draws it
  through the layer's transform and the camera. Smudge and eraser brushes therefore see the strokes painted before
  them, in order.

Worked example: `examples/painting.anim` (about 35 seconds to render 16 seconds of video, because every frame replays
every stroke).

### Tilt, symmetry and blend modes

```anim
# tilt: a pen leaning on the surface (-1 to 1 each way, as a tablet reports it); brushes such as calligraphy react
art.stroke(pen, [(10, 10, 0.5, 0.02, 0.3, -0.4), (60, 10)], tilt: (0.5, 0))     # per point (x, y, pressure, seconds, xtilt, ytilt) or a default
art.stroke(pen, points, tilt: "follow", barrel: 45.deg)                         # lean the way the stroke goes; twist the pen

# symmetry: every dab is repeated around a centre (default: the canvas centre)
art.stroke(ink, points, symmetry: "radial", lines: 6, center: (400, 300), angle: 15.deg)
art.symmetry("snowflake", lines: 8)        # the default for the strokes that follow ...
art.symmetry(none)                         # ... until cleared; `symmetry: none` on a stroke overrides it once

# blend modes: composite a stroke onto what is already painted
art.stroke(glow, points, blend: "screen", strength: 0.8)
art.blend = "add"                          # how the whole layer is composited onto the scene
```

- **Tilt.** A point has 2, 3, 4 or 6 numbers: `(x, y)`, `(x, y, pressure)`, `(x, y, pressure, seconds)` or all six with
  `xtilt, ytilt`. `tilt: (x, y)` is the default for points without their own, `tilt: "follow"` leans the pen 0.6 of the
  way towards the direction of travel, and `barrel:` twists it (an angle). Out of range is `E-BRU-002`, a bad `tilt:` value
  `E-BRU-003`. Only brushes that use tilt inputs (calligraphy, some chalks) change.
- **Symmetry.** `vertical` (mirrors across the vertical line through the centre), `horizontal`, `both` (four copies),
  `radial` (`lines` copies turned around the centre) and `snowflake` (the same, each also reflected); `angle` turns the
  lines. A stroke keeps the symmetry it was made with, so the snapshot is self-contained. Bad kinds or `lines` outside
  2 to 64 are `E-BRU-003`.
- **Blend modes.** `normal`, `multiply`, `screen`, `overlay`, `darken`, `lighten`, `color_dodge`, `color_burn`,
  `hard_light`, `soft_light`, `difference`, `exclusion`, `add` (the W3C compositing formulas). `strength` (0 to 1) is how
  much of the stroke is composited. A blended stroke is painted on a transparent copy first, so a **smudge** brush in
  a blended stroke sees nothing under it; later strokes see the blended result. `art.blend = "..."` blends the whole
  layer onto the scene. Unknown modes and a `strength` outside 0 to 1 are `E-BRU-003`.
- **Modes that live in the brush.** `eraser: true` (erase), `lock_alpha: 1` (paint only where there is paint) and
  `colorize: 1` (take the hue and saturation of the colour, keep the lightness underneath) are libmypaint brush
  settings, set like any other: `brush("rounded", lock_alpha: 1, color: #ff0000)`.

Worked example: `examples/mandala.anim`.


## 13o. Freehand drawing: Bezier fitting and point lists

Hand-drawn points (a tremor and all) become smooth cubic Bezier curves with Philip Schneider's curve-fitting
algorithm, with corner detection so sharp turns stay sharp.

```anim
let rough = [(100, 100), (104, 96), (110, 99), ...]                 # a hand-drawn path, any number of points
shape heart = freehand(rough, tolerance: 6, corner: 40.deg, closed: true)    # a vector shape of cubic edges
heart.fill = #ff8fab

let clean = smooth(rough, tolerance: 6, closed: true, spacing: 3)   # the fitted curve as points 3 px apart
art.stroke(ink, clean) over 2.seconds                               # ... painted with a brush (13n)
art.stroke(chalk, outline(heart, spacing: 3))                       # a stroke along any shape's outline
art.stroke(pencil, points_along(heart.e2, spacing: 2))              # ... or along one edge
art.stroke(pencil, jitter(resample(corners, 4), amount: 3, scale: 25, seed: "box"))   # a ruler-straight path, drawn by hand
```

- `freehand(points, tolerance: 2, corner: 40.deg, closed: false)` returns a new shape: one vertex at each join and
  one cubic edge per fitted piece (and a face when `closed`). `tolerance` is the largest distance, in pixels, a given
  point may be from the curve; pick about the size of the tremor (a larger value gives fewer, smoother pieces; a very
  small one follows every wobble). Turns sharper than `corner` stay corners; gentler ones are smooth. Two or more
  different points are needed (`E-FRE-001`). The result is an ordinary shape: fill, move, rotate, morph, animate.
- `smooth(points, tolerance:, corner:, closed:, spacing: 4)` returns the fitted curve as a list of points, ready
  for a brush stroke. `simplify(points, tolerance)` (Douglas-Peucker) drops points that do not matter,
  `resample(points, spacing)` spaces points evenly along the path (the ends are kept), `path_length(points)`.
- `points_along(edge, spacing:)` and `outline(shape, spacing:)` sample a curved edge or a shape's outline (its first
  face's boundary, else its edges in order) in world coordinates, so any Bezier edge can be painted along.
- `jitter(points, amount: 1.5, scale: 30, seed: "name")` moves each point by a smooth seeded wobble (the wobble changes
  over `scale` px of path), for a hand-drawn look; the same seed gives the same wobble.

Worked example: `examples/freehand.anim`.

## 13p. Equations: `plot`, `curve`, `polar`, `implicit`, `drive` and `std/waves`

Any shape can be given an equation. An equation is a **string** (`"sin(x / 40 + t * 3) * 8"`) or a **lambda**
(`(x, t) -> sin(x / 40 + t * 3) * 8`). Both compile to the same small program (the math engine, crate
`animscript-math`), which is run over all the points at once and gives the same numbers on every machine.

```anim
shape wave  = plot(y: "sin(x / 30) * 20", from: 0, to: 800, n: 120)            # y as a function of x
shape heart = curve(x: "16 * sin(u)^3", y: "-13 * cos(u) + 5 * cos(2*u) + 2 * cos(3*u) + cos(4*u)", from: 0, to: 6.2832, n: 120, closed: true)
shape rose  = polar(r: "40 * cos(3 * theta)", n: 200)                          # closed by default (full turn)
shape ring  = implicit(f: "x*x + y*y - 100*100", x0: -150, y0: -150, x1: 150, y1: 150, res: 64)   # where f = 0

water.drive(y: "amp * sin(tau * (x / 240 - t / 3))", amp: 10) over 8.seconds     # moves every vertex of `water`
start boat.drive(pos_y: "6 * sin(t * 2)", rotation: "0.02 * sin(t)")             # shape properties
```

- **Shapes from equations** are ordinary shapes (fill, move, animate, drive them). `plot`, `curve` and `polar` sample `n`
  points (default 100, 2 to 100000) from `from` to `to`; with `closed: true` the last point joins the first (and the
  samples exclude the end value, so a full turn does not repeat its first point). `plot` gives `x`, `curve` gives `u`,
  `polar` gives `theta` (radians, default 0 to tau); a non-finite value is `E-MTH-005`. `implicit` finds where `f`
  changes sign on a grid of `res` x `res` cells (marching squares, negative is inside) and joins the crossings into
  edges; it has no face, and no crossing at all is `E-MTH-003`.
- **Equation syntax (strings):** numbers, `+ - * / %`, `^` (right-associative, tighter than unary minus), comparisons,
  `&&` `||` `!` (giving 0 or 1), `c ? a : b` and `if(c, a, b)`; constants `pi tau e`; functions `sin cos tan asin acos
  atan atan2 sinh cosh tanh exp log ln log2 log10 sqrt abs floor ceil round trunc frac sign min max pow mod clamp
  mix smoothstep step noise hash select`. Angles are **radians**. `noise(x)`, `noise(x, y)`, `noise(x, y, z)` is smooth
  value noise in [-1, 1], seeded by the script's seed, so it is the same on every run. A syntax error is `E-MTH-001`.
- **Lambdas** name their inputs as parameters (`(x, t) -> ...`). They may use `+ - * / %`, comparisons, `and or not`
  and the functions above (there is no `^`: write `pow(x, 2)`). Anything that does not mention an input is worked out
  first by the interpreter, so `amp * 2`, `3.seconds` or `s.v2.x` are fine as constants; anything else is `E-MTH-002`.
- **Named numbers:** any extra named argument that is a number (or a duration, angle or bool) is a constant in the
  equation (`amp: 10`); durations are seconds and angles radians there.
- **`drive`** is a layer-2 modifier, like `wobble`: it *adds* to what is below it and keyframes and locks still win.
  It needs `over` or `start`. On a shape it moves all its vertices; on a vertex selection (`w.vertices.first(5)`) just
  those. Properties: `x`, `y` (vertex offsets), or `opacity`, `width`, `rotation` (turns), `scale`, `pos_x`, `pos_y`
  for the shape (not both kinds in one drive, `E-MTH-004`). Options: `mode: "replace"` (the equation gives the value
  itself instead of an offset), `speed` (multiplies `t`), `blend_in`, `blend_out`, `over`.
- **Inputs of a drive:** `x`, `y` (the vertex's own rest position; for shape properties the shape's rest centre in the
  scene), `i` (vertex index, 0-based, in id order), `n` (vertex count), `u` (i / (n - 1)), and `t` (seconds since the
  drive started, times `speed`). A result that is not a finite number drives nothing (0 is added, and a `MATH` line
  appears in the trace once).
- **`import "std/waves"`** (read `crates/lang/lib/waves.anim`): `swell(amp, length, period, over, blend)` a travelling
  sine, `ripples(...)` two crossing waves plus noise, `choppy(..., sharp)` sharp crests (vertices also move sideways),
  `float_on(amp, length, period, tilt, over, blend)` bobs and tilts a shape with a `swell` surface at its own x (give
  it the surface's numbers). They use the equations above: copy and change them.

**Machine code (optional).** A build with `--features jit` (`cargo build -p anim --release --features jit`) compiles the
equations of a `drive` over 128 or more vertices to machine code (Cranelift). The results are identical to the
interpreter's bit for bit (a NaN stays a NaN), so scripts, snapshots and hashes never change; only the speed does (about
3x on wave-like equations, up to 30x on arithmetic-only ones, 1.4x when almost all the time is in `sin`/`exp`). Choose
with `anim --jit auto|always|off` (default `auto`); if the machine does not allow it the interpreter is used and a
`JIT` line appears in the trace.

(`animscript-math` also has an experimental `llvm` feature, an LLVM 18 backend that is faster only on equations without
`sin`, `exp`, `noise` and the like; it is not used by the language. See `docs/DECISIONS.md`, items 249-252.)

Diagnostics: `E-MTH-001` equation syntax, `-002` construct not allowed in an equation, `-003` bad arguments (missing
equation, `n`, `res`, bounds, `mode`), `-004` unknown or mixed drive property, `-005` non-finite value while building a shape.

Worked example: `examples/water.anim`.

## 13q. A worked example: "The Candle" (brushes, a rigged puppet, equation flames)

`examples/candle/candle.anim` (video: `examples/candle/candle.mp4`; how it is made: `examples/candle/tools/README.md`)
puts the painting, rigging and equation features together:

- **Drawing.** Every part of the character (torso, scarf, head, hair, ear, five eyes, three brows, four mouths, the arm
  with a match) is a `paint_layer` with its own `pivot`, painted with libmypaint strokes: ragged marker fills,
  pressure-varying ink lines, a pencil under-drawing, charcoal shading. The head is a trace of a reference drawing.
- **Performance.** One scene `timeline:` keys `position`, `rotation` and `opacity` of the layers (and `camera.position`,
  `camera.scale`): the entrance with a settling bounce, the head with spring overshoot, hair and scarf follow-through, a
  two-bone arm solved with inverse kinematics, expressions swapped by cross-fading layer opacity, blinks, a push-in.
- **Equations.** The candle flame, the match flame and the smoke are `curve(...)` shapes. A `drive` with
  `mode: "replace"` moves every vertex each frame from an equation: a travelling sine sway that grows with height,
  `noise` flicker, an ignition envelope (`smoothstep(0, 1.6, t)`) and a height flicker that shares its sine terms with
  the glow, which is a paint layer whose `opacity` is driven by `smoothstep(...) * (0.62 + 0.07 * sin(...) + ...)`.
- **Light.** A multiplied "dusk" wash lifts while an additive glow and a screened light on the face fade in.

The renderer **caches each painted layer** by its content (brushes, strokes with progress, size, resolution): a layer
that only moves, turns or fades is not painted again, only drawn through its new transform. That brings a 16 second,
about 1200-stroke scene from roughly fifteen minutes to a couple.

## 13r. Fields: equations stepped over a grid (round M4)

A **field** is a grid of cells holding one or more numbers, updated every frame by equations you write: a wave, heat,
a chemical reaction. It is deterministic (fixed time step, double buffered, one thread) and it can be shown, read from
scripts, and used to move shapes and particles.

```anim
field pond:
    grid: (256, 104)                        # cells (at most 512 on a side, 512 x 512 in all): E-FLD-002
    rect: (0, 200, 1280, 520)               # where it lies: x, y, width, height
    state: ["h", "v"]                       # the channels (scalars)
    init: [("h", "exp(-((x - 400)^2 + (y - 300)^2) / 400)"), ("v", "0")]
    step: [[("v", "v + dt * (c2 * (h[-1, 0] + h[1, 0] + h[0, -1] + h[0, 1] - 4 * h) - damp * v)")],
           [("h", "h + dt * v")]]           # two phases: the second reads the new v
    c2: 1100                                # any other name with a number is a constant of the equations
    damp: 0.9
    edges: "mirror"                         # clamp (default) | wrap | mirror | zero: what lies beyond the grid
    substeps: 4                             # 1 to 64 steps per frame; dt = 1 / (fps * substeps)
    ramp: [#061827, #1b6fa3, #ffffff]       # show it: the colours, the channel and the range they cover
    channel: "h"
    range: (-0.5, 0.9)
    z: 2                                    # drawing order among the shapes (opacity: and blend: too)
```

- **Equations** are strings or lambdas (the math engine, section 13p). Inputs: `x`, `y` (the cell's centre), `i`, `j`
  (its column and row), `t` (seconds since the scene began, at the end of the substep), `dt`, the channels by name, and
  your constants. **Neighbours:** `h[-1, 0]` is the cell to the left, `h[0, 1]` the one below (offsets of at most 16;
  lambdas write `h(-1, 0)`). A one-row field can only read along x.
- **Phases.** `step:` is a list; pairs written one after another form one phase, a nested list is a phase of its own.
  Inside a phase every equation reads the state as it was before the phase (so `a`/`b` swap correctly); the next phase
  sees the result. A channel appears once per phase. `init:` gives the starting state (channels you leave out start at
  0); `noise(...)` and `hash(...)` are seeded by the script, so every run starts the same.
- **Edges.** `clamp` repeats the border cell, `wrap` joins opposite sides, `mirror` reflects (the cell beyond the edge
  is the border cell itself), `zero` reads 0.
- **From the script** (while a scene runs): `pond.poke(x, y, amount: 1, radius: 20, channel: "h")` adds a smooth bump,
  `pond.fill(0, channel: "h")`, `pond.at(x, y)` reads the value there (bilinear between cell centres), and
  `pond.sum()`, `pond.max()`, `pond.min()`. A field starts over with every scene.
- **A chain.** `chain: wave` (a shape) makes a one-dimensional field with one cell per vertex, in id order: a string or
  a water line that is *simulated*.
- **Reading a field from a drive.** `w.drive(y: "-pond_h * 40", fields: [pond])` gives the equation the inputs
  `<field>_<channel>` (and `<field>` for the first channel), sampled at each vertex's rest position (a shape-level
  drive: at the shape's rest centre). A boat that bobs and tilts: `drive(pos_y: "-pond * 110", rotation: "atan(pond_gx * 26) / tau", fields: [pond])`.
- **Particles with forces.** An emitter takes `force_x: "..."`, `force_y: "..."` (px per second squared) over `x`, `y`,
  `vx`, `vy`, `age`, `i`, `t`, and the fields named in `fields: [pond]`: leaves that slide down a wave's slope.
- **Snapshots** carry, per field, the name, grid and rectangle, a checksum of all the numbers, the sum, minimum and
  maximum of each channel, and (with a `ramp`) the colour-mapped picture (at most 256 cells on a side). `raw: true`
  adds every number. The renderer draws the picture smoothly inside the rectangle.
- **Errors:** `E-FLD-001` bad declaration (missing grid or channels, unknown channel, bad edge rule, a second row read in
  a one-row field, a duplicate name), `-002` size limits, `-003` a value became infinite or not a number (the message
  says where and when; usually the time step is too large: lower the coefficients or raise `substeps`).
  Equation syntax is `E-MTH-001` as usual.
- **Machine code.** With the `jit` feature, equations of fields with 128 or more cells run as machine code, with
  identical results.

Worked examples: `examples/pond.anim` (the wave equation, rain, a boat and leaves driven by the surface) and
`examples/turing.anim` (reaction-diffusion growing mazes).

## 13s. 3D: meshes in the same scenes (round 3D-1/2)

A mesh is an ordinary shape that happens to have volume, so keyframes, `drive`, `opacity`, `z`, `fill`, `stroke` all apply. Axes: x right, y down, z away from the viewer. The default camera shows the z=0 plane at canvas scale, so a mesh placed at `(400, 300, 0)` appears at `(400, 300)`.

```
import "std/3d"
stage3(fov: 40, light: (-0.6, 0.65, 0.5), ambient: 0.4, bands: 4)
shape vase = lathe([(0, 0), (60, 20), (40, 120), (70, 200)], at: (400, 300, 0), fill: #c8553d)
shape blob = extrude(my_freehand_shape, depth: 60, at: (200, 300, 0))
vase.spin(0.5)
camera3.orbit(turns: 0.1, over: 10.seconds)
```

- **Builtins:** `cube` (with `subdivide: n` to cut each face into n x n quads), `sphere`, `cylinder`, `cone`, `plane(width, depth, nx, nz)`, `lathe(profile)`, `extrude(shape | points, depth)`; each takes `at:` (centre of the bounding box, default the origin), `pivot:` (world point it turns about), `fill`, `outline`, `double_sided`.
- **Properties:** `position.z`, `rotation.x`, `rotation.y`, `scale.z` (keys use angle units such as `.deg`). Turn order: spin about y, tilt about x, roll about z, then translate. Drive names: `pos_z, rot_x, rot_y, scale_z` (turns); vertex drives `x, y, z` (z only on meshes: `E-MTH-004` on 2D shapes); drive inputs are `x, y, z, i, u` plus frame `t, n`.
- **Fields:** a mesh vertex reads a field at its world `(x, z)`, so a `plane` can follow a wave field and boats ride its height and slope (see `examples/lagoon.anim`).
- **`stage3(fov, light, ambient, bands, crease, smooth)`:** `light` is the direction the light travels. Defaults: (-0.4, 0.7, 0.6), 0.35, 3 bands, 40 deg crease, 50 deg smooth, fov 40.
- **`camera3`:** created with the first mesh. `position.x/y/z` offset from the default eye, `rotation.y` yaw (positive looks toward +x), `rotation.x` pitch (positive looks up), `rotation` roll, `scale.x` zoom. `focal3()` gives the eye distance to the z=0 plane.
- **std/3d actions:** `spin(rate)`, `bob(amount, period)`, `sway(roll, nod, period)`, `orbit(turns, distance, over)`, `crane(height, pitch, over)`.
- **Rendering:** a deterministic software rasterizer (z-buffer, 2x supersampling, toon bands, smoothed normals, inked outlines on silhouette, boundary and crease edges). All meshes draw together at the first mesh's z-order position; 2D shapes at other `z` values sit in front of or behind them.
- **Errors:** `E-MSH-001` bad mesh arguments.
- **Not yet:** 3D physics, glTF/OBJ import, mesh editing.

## 13t. 3D physics (round P3D)

A mesh shape becomes a rigid body with the same `.body()` call 2D uses. Gravity, collisions, friction, bounce, sleeping and joints are simulated deterministically by `crates/physics3`; the body drives the mesh's `position` (x, y, z) and its orientation.

```anim
world:
    gravity: (0, 9.8, 0)     # a 3-vector makes a 3D world (the default); scale: and substeps: work as in 2D
shape ground = cube(width: 2600, height: 60, depth: 1800, at: (640, 630, 300))
ground.body(kind: static)
shape vase = lathe(profile: [(0, 0), (34, 0), (50, 62), (20, 120), (28, 152), (0, 152)], segments: 28, at: (480, 454, 120))
vase.body(collider: "hull", density: 0.6)
let gate_joint = hinge(post, gate, anchor: (1140, 350, 120), axis: (0, 0, 1), limits: (-80.deg, 80.deg))
```

- **`.body(...)`** takes `kind` (`static`, `dynamic`, `kinematic`), `density` (kg per cubic metre; a 100 px cube of density 1 weighs 1 kg), `friction`, `restitution`, `linear_damping`, `angular_damping`, `gravity_scale`, `fixed_rotation`, `sensor` and `collider`.
- **Colliders:** `auto` (the default) uses a sphere for a round mesh, the convex hull for a convex one (boxes, cones, extruded convex outlines) and, for a static body that is not convex, the exact triangles. A moving body that is not convex is `E-PHY-007`; `collider: "hull"` collides as the convex hull of the mesh (it fills dents), `"sphere"` and `"capsule"` (along the mesh's y axis) are explicit. A body's `scale` must be uniform and is fixed when the scene starts.
- **Pose:** a dynamic body writes `position.x/y/z` and a quaternion orientation; actions, keys and `rotation` on it lose to physics, and at the end of the scene the final pose is written back as ordinary rest values. A kinematic body follows `position`, `position.z` and the turns you animate and pushes dynamic bodies. Make bodies at the top level or in `setup:`.
- **Methods:** `set_velocity((x, y, z))`, `apply_impulse((x, y, z))` (pixels per second), `set_spin((x, y, z))` (turns per second), `touches(other)`, `leaves(other)`; `whenever a.touches(b):` works as in 2D.
- **Joints** (functions, between two mesh bodies, anchors are world points `(x, y, z)`): `ball(a, b, anchor)`, `hinge(a, b, anchor, axis, limits, motor)` (limits are angles, motor is `(speed, torque)` such as `(90.deg, 50)`), `fixed(a, b, anchor)`, `strut(a, b, length, spring: (hz, damping), anchor_a, anchor_b)` (rigid or a spring) and `tether(a, b, max, anchor_a, anchor_b)` (a rope). All take `collide: true` to let the pair collide.
- **Floating:** `body.float(field: sea, channel: "h", level: 500, density: 2, drag: 1)` (in `setup:`) lifts the body with buoyancy from a field: the water surface at a point is `level - field value` at the point's world (x, z) (the same convention as `water.drive(y: "-sea_h", fields: [sea])`). A body lighter than `density` floats; `drag` damps what is under the water.
- **Debug:** `anim render scene.anim -o f.png --debug-physics` draws contact points with their normals (red) and joints (blue) over the 3D view. Snapshots carry a `physics3` section (bodies, contacts, joints) only when a mesh world exists.
- **Errors:** `E-PHY-007` bad mesh body (not convex, non-uniform scale, unknown kind or collider, a second body, `float` arguments), `E-PHY-008` bad 3D joint (a shape that is not a mesh body, a zero axis, limits the wrong way round); `W-PHY-001` bodies overlap at the start. A scene has 2D bodies or mesh bodies, not both.
- **Limits:** no continuous collision (a very fast small body can pass through a thin one), no 3D soft bodies or particle fluids yet, no ragdolls. Worked example: `examples/crates.anim`.

## 13u. 3D soft bodies (round S3D)

`mesh.soft(...)` makes a mesh squash, stretch, bounce, swing and drape. It steps together with the 3D rigid bodies of §13t: soft meshes collide with rigid bodies (and push dynamic ones back), with the ground and with each other.

```anim
shape ball = sphere(radius: 52, at: (90, 330, 150))
ball.soft(pressure: 1.0, stiffness: 0.85, restitution: 0.5)       # a balloon: a closed surface that keeps its volume

shape jelly = cube(size: 92, subdivide: 6, at: (500, 100, 60))
jelly.soft(solid: true, stiffness: 0.55, resolution: 7)            # solid jelly, filled with tetrahedra

shape flag = plane(width: 210, depth: 130, nx: 14, nz: 9, at: (950, 294, 250))
flag.soft(stiffness: 0.9, bend: 0.12, density: 0.3)                # cloth
flag.pin_where(p -> p.z < 190)                                      # hold the vertices that pass the test
```

| Argument | Meaning | Default |
|---|---|---|
| `stiffness` | above 0, at most 1: resistance to stretching (1 = rigid) | `0.8` |
| `bend` | 0 to 1: resistance to folding sharply (cloth) | `0.3` |
| `pressure` | closed surfaces: 1 keeps the volume, above 1 inflates, 0 ignores volume; solid bodies scale their rest volume by it | `1` |
| `density` | kg per square metre of surface (a solid body: per cubic metre) | `1` |
| `friction`, `restitution` | against bodies, like rigid bodies | `0.5`, `0.1` |
| `damping` | slows motion down | `0.1` |
| `thickness` | collision size of a vertex, px | `3` |
| `self_collide` | let the mesh's own triangles stop its own vertices (cloth on cloth) | `false` |
| `solid` | fill a closed mesh with tetrahedra (jelly) instead of keeping only its skin | `false` |
| `resolution` | solid bodies: lattice cells along the longest side, 3 to 24 | `8` |

- **Surface or solid:** a surface body is its skin: edges keep their length, shared edges resist folding, and a closed one keeps its volume (balloons, rubber balls, cloth, flags). A solid body is a lattice of tetrahedra that keeps its volume everywhere (jelly, a soft pad). The mesh's vertices ride on the lattice, so give a solid cube enough vertices to show a dent: `cube(size: 100, subdivide: 6)` (every face cut into 6 x 6 quads).
- **Pins:** `pin_where(test)` holds the vertices for which the test is true (the test gets each vertex's position in pixels, `p -> p.y < 120`); `pin_where(test, to: other)` makes them follow another mesh (a rigid body, a kinematic or an animated one) so it can carry a cloth. `unpin_where(test)` lets go, also while running.
- **`cut(from: (x, y, z), to: (x, y, z), width: 8)`** (while a scene runs, surface bodies only): every face with an edge within `width` pixels of the line segment is removed and the links it held are dropped. A cut balloon loses its pressure; a cut cloth parts. It returns the number of faces removed.
- **`soft_impulse(by: (x, y, z))`** gives the whole body a push (kg px/s): a puff of wind. **Reading:** `shape.volume` (px^3, from the faces) and `shape.stretch` (largest edge strain).
- **Moving it:** the mesh's own position, turns and scale are baked into its vertices when the scene starts (a soft mesh is then in world coordinates); move it by pinning vertices to something that moves. The final vertices are kept when the scene ends.
- **Rules:** a shape is rigid or soft (`E-PHY-003` otherwise). `E-PHY-009`: bad `soft` arguments, `solid` or `pressure` on an open mesh, a solid mesh that cannot be filled or needs too many nodes, `pin_where`/`cut` on a shape that is not soft, `cut` on a solid body or outside a scene. `--debug-physics` draws pins (blue squares).
- **Limits:** soft meshes that are very fast and thin can pass through each other's triangles; a large static triangle mesh is slow to collide with (use cubes and planes for the set); no cutting of solid bodies; no soft bodies in a 2D scene. Worked example: `examples/jelly.anim`. `world:` takes `soft_substeps` (16) and `soft_iterations` (4), as for 2D.

## 13v. 3D fluids (round F3D)

A `fluid` in a 3D scene is thousands of particles that pour, splash, pool and slosh like water. It collides with the rigid mesh bodies (a tank is a few static cubes), pushes them and buoys them (a light crate floats, a heavy ball sinks), and is drawn as a smooth, transparent, lit surface built from the particles.

```anim
world:
    fluid_threads: 0            # worker threads for the solver (0 = the machine's); the result is the same for any number

fluid water:
    fill: box(342, 420, 66, 270, 180, 288)    # x, y, z of the near top-left corner, then width, height, depth (px)
    spacing: 16                                # particle distance in px: smaller = finer and much slower
    viscosity: 0.04
    foam: 0.35
    smoothness: 1.5
    color: #72c6f890                           # at the surface (alpha allowed) ...
    color_end: #2a6cc0ff                       # ... and deep down
    max: 7000

scene "s" for 8.seconds:
    main:
        water.pour(at: (780, 120, 210), speed: (0, 260, 0), width: 64)   # a jet 64 px across, falling
        wait 2.seconds
        water.stop()
        water.add(box(400, 300, 100, 80, 80, 80))                         # drop in a block at once
```

- **3D or 2D:** a fluid is 3D when its `fill` is a `box(x, y, z, w, h, d)` or a mesh shape (its inside is filled), or when it says `space: "3d"` (a fluid that is only poured). A 3D fluid goes with mesh bodies (§13t) and the 3D camera; a scene's fluids may be 2D or 3D but not both kinds in one scene.
- **Settings** are those of §13e (`spacing`, `density`, `viscosity`, `surface_tension`, `foam`, `smoothness`, `color`, `color_end`, `max`) plus `space`. `spacing` defaults to 12 px in 3D. `render: "dots"` draws every particle as a small ball instead of the surface. All fluids together hold at most 20000 particles.
- **Operations:** `pour(at: (x, y, z), speed: (x, y, z), width:)` makes a jet whose cross-section is a disc `width` px across; `stop()`; `add(region)` drops in a block (`box(...)` or a mesh); `level(x, z)` is the surface height above that point (px, or none when there is no water there); `water.count` is the number of particles.
- **Bodies:** containers are ordinary static bodies (cubes, planes; hide a wall you want to see through with `visible = false`, it still holds the water). Dynamic bodies are pushed and buoyed: give a body `density` below 1 to float it. Soft meshes are obstacles for water and are pushed by it (§13w). A resting body keeps a thin layer of particles under it.
- **The picture:** the renderer fills the particles' density on a grid and extracts a triangle mesh from it (marching cubes), draws it over the opaque meshes with transparency (bodies show through, the colour deepens with the thickness of water, the surface is toon-lit and has an inked silhouette), and draws thin jets, droplets and foam as small spheres. Foam and spray are decorative: they never push anything and are seeded, so the same program always throws the same foam.
- **Cost and threads:** a few thousand particles is normal (the example is 4 to 7 thousand: under a second a frame); halving `spacing` is eight times the particles. The solver's per-particle loops run on several threads in fixed contiguous chunks that are joined in order, and the surface field is summed in a fixed order, so the same program gives the same frames on one thread or many. In `world:` the settings `fluid_substeps` (4), `fluid_iterations` (3) and `fluid_threads` (0) tune it. Render many frames at once with `anim frames` (it simulates once).
- **Errors:** `E-FLD-004` (a 3D fluid fills a `box(...)` or a mesh, a pour needs a speed, the fill region does not fit under `max`), `E-FLD-001` (a bad setting). Worked example: `examples/splash.anim`.
- **Limits:** reflection and refraction are screen-space (§13w); a very fast body can pass through a thin wall (no continuous collision); a large static triangle mesh is slow to collide with, use cubes and planes for the tank; a fluid can be coupled to a wave field (§13w).

## 13w. Water that reflects, bends and pushes back (round W)

Four additions to the 3D water of §13v and the wave fields of §13r.

```anim
stage3(sky: (#7fb9df, #dcedf0))         # the sky that mirrors and water reflect where nothing else is

ocean.mirror(reflect: 0.5)               # a mesh that shows its surroundings
pane.glass(opacity: 0.16, reflect: 0.3, refract: 0.9)   # a mesh you look through, bent

fluid splash:
    space: "3d"
    field: sea                           # rest on, and make ripples in, this wave field
    level: 520                           # the field's rest surface: y in px, the surface is level - h
    channel: "h"                         # height channel (default: the first); "velocity:" the speed channel (default: "v")
    ripple: 1                            # how strongly impacts ripple the field (0 = none)
    absorb: true                         # water that reaches the field joins it (false: it rests on it as a pool)
    reflect: 1                           # Fresnel strength of the water's surface, 0 to 1
    refract: 1                           # how much it bends what is behind it, 0 to 1
```

- **Water pushes soft bodies.** Water that is stopped by a soft mesh (cloth, a balloon, jelly) gives it the momentum of the push, shared among the triangle's vertices by inverse mass, exactly as it already did for rigid bodies. A jet pushes a flag, water weighs on a hammock. Soft meshes still are not dragged along by water that merely flows past.
- **Wave fields.** A fluid with `field:` treats the field as a floor with a velocity: water cannot sink below the surface `level - h(x, z)` (the same convention as `water.drive(y: "-sea_h", fields: [sea])` and `float`), slides along its slope and is carried by its velocity channel, so a wavemaker lifts the water above it. In the other direction, water that lands on the surface makes ripples: a real impact (faster than 0.5 m/s) presses the height channel down by 10 px per kg m/s around the point, and with `absorb: true` (the default) water that reaches the surface joins the field and raises it by its volume. A field-coupled fluid is a sea; set `absorb: false` for a pool that rests on the surface. Field values are clamped to +-150 px at an impact, and the field must be a stable wave field (see `substeps` in §13r: with `c2` in cell units, `c2 * dt^2` must stay below 0.5). Chain fields cannot carry water.
- **Materials.** `mesh.mirror(reflect: 0.5)` makes an opaque surface that blends in what it reflects (0 to 1); `mesh.glass(opacity: 0.2, reflect: 0.25, refract: 1)` makes a transparent one that bends the picture behind it, tints it with the shape's `fill` and reflects by Fresnel; `mesh.matte()` removes either. (`mirror` is also a 2D editing operation of §13c; on a mesh it is the material.) They work on any mesh, including the water plane of a wave field. Glass is drawn after the water and does not hide what is behind it.
- **Reflection and refraction.** Screen-space: a reflection ray marches against the depth of the opaque meshes and shows what it hits, or the sky (a gradient, `stage3(sky: (top, horizon))`, by default a bluer version of the project background above its horizon colour) with a glint of the light source; so only what is on screen and opaque is reflected (not glass, not other water, nothing behind the camera). Water refracts by Snell's law (index 1.33): it looks where the bent ray would come out and tints by how deep the water is (Beer's law style, the `color` to `color_end` blend); the reflected share is Schlick's Fresnel, small head-on and large at a grazing angle.
- **Errors:** `E-MSH-001` for a bad material argument, `sky` that is not two colours, or `glass`/`mirror` on a shape that is not a mesh; `E-FLD-001` for `field:` without `level:`, an unknown channel, or `ripple`, `reflect`, `refract` out of range. Worked example: `examples/harbour.anim`.
- **Limits:** no caustics (light patterns on the floor), no total internal reflection, reflections do not see glass or water, and a field must be a grid field.

## 13x. 3D skeletons, skinning and ragdolls (round R)

An `armature` whose bones have a `dir:` is a 3D skeleton (everything of §11's 2D armatures that makes sense carries over: bones and IK handles are hidden pseudo-shapes `armature.bone`, so every action, keyframe, easing and `drive` works on them).

```
armature puppet:
    ragdoll: radius: 12, density: 100, friction: 0.7
    bone spine: at: (640, 384, 200), dir: (0, -1, 0), length: 120, swing: 35.deg, twist: (-25.deg, 25.deg)
    bone upper_r: parent: spine, at: 1.0, dir: (1, 0, 0), length: 85, swing: 150.deg, twist: (-60.deg, 60.deg)
    bone fore_r: parent: upper_r, length: 85, axis: (1, 0, 0), limits: (-150.deg, 0.deg)
    ik hand_r: chain: [upper_r, fore_r], bend: (760, 384, 60)
```

- **Bones.** `at:` is a 3D point for the root and a fraction (0 to 1, default 1) along the parent for the others; `dir:` is the rest direction (a child inherits its parent's); `length:` is required; `radius:` sizes the ragdoll capsule. A bone is a **hinge** (`axis:` and `limits:`, a pair of angles about the world rest axis) or a **ball** (`swing:` is a cone half-angle about the rest direction, `twist:` a pair of angles about it); both at once is an error.
- **Posing.** A bone's `rotation.x`, `.y`, `.z` (turns, applied y, then x, then z like a mesh's) rotate it about its own head relative to its rest pose, in the frame its parent carries. The root bone's `position` (x, y, z) offsets the whole skeleton. Rotation beyond the limits is clamped with `W-RIG-001`.
- **IK.** `hand.reach((x, y, z), over:, ease:)` and `hand.release()`; the handle's position is the target relative to the rest tip and its opacity the weight. A two-bone chain is solved exactly (the elbow lies in the plane through the pole point `bend:`; with a hinged lower bone the upper bone is twisted so the hinge stands across the bend plane), longer chains with FABRIK. A target out of reach straightens the chain towards it.
- **Meshes.** `mesh.bind(armature.bone)` fixes a mesh to a bone, `mesh.skin(armature, blend: 20, bones: [...])` blends it over bones by distance (`exp(-d / blend)`, normalised, linear blend skinning), `mesh.set_weight(where: p -> p.y < 300, bone: armature.bone, weight: 1)` paints. Vertices are skinned from the rest pose; the mesh's own transform is baked into its vertices when the scene starts. A mesh is rigid, soft or rig-driven, never two (`E-RIG-007`). `bone.head`, `bone.tip`, `hand.target`, `hand.weight` read the solved pose.
- **Ragdolls.** With a `ragdoll:` line every bone gets a capsule body in the 3D world (ball joints with the cone and twist limits, hinges with theirs). Bones start *animated*: kinematic bodies that follow the pose and can push other bodies. `armature.ragdoll(strength: 0, push: (vx, vy, vz))` makes them dynamic; above zero every joint has a muscle (a spring on the joint's orientation, torque limited to `strength` times what holds the limb against gravity) and every bone is pulled towards its animated placement (speed capped at 3 m/s). `armature.animate()` hands them back. While physical, skinned meshes, `bone.head` and the snapshot follow the bodies; `armature.ragdolled` reads the state. Muscles at `strength` 1 can stretch joints by a few centimetres; use 0.7 or less for stiffness without strain.
- **Debug.** `--debug-physics` draws bones as green lines with a ring at the head. The snapshot has `rigs3` (per armature the bones' head, tip and orientation quaternion, and the ik targets).
- **Errors.** `E-RIG-005` bad bone, `E-RIG-006` bad ik line, `E-RIG-007` bind/skin/weight on something that is not a free mesh, `E-RIG-008` ragdoll misuse (no `ragdoll:` line, not in a running scene, bad `strength` or `push`).
- **Limits.** No IK-planted walking gait for 3D (the 2D `gait` has no 3D counterpart yet), no blend shapes, IK ignores a ball joint's swing cone (the limit clamps afterwards), mesh import is not supported.

Example: `examples/puppet3d.anim` (waving, knocked over by a ball, limp, standing up again as an active ragdoll).

## 13y. Photographic lighting (round L)

A scene that declares at least one light is lit photographically, in 3D and 2D alike. A scene without lights renders exactly as before (toon bands from `stage3(light:, ambient:, bands:)`); when lights exist those three settings are ignored.

```
stage3(fov: 40, ao: 90, bloom: 0.3, exposure: 1.2, quality: "high")
shape lamp = point_light(at: (640, 150, 40), color: #ffc78a, intensity: 7, reach: 300, size: 16)
shape sun = sun_light(dir: (0.35, 0.45, 1), color: #6f8cff, intensity: 0.16, size: 4)
shape spot = spot_light(at: (900, 80, 300), dir: (0, 1, 0), angle: 30.deg, softness: 0.4, intensity: 5)
shape room = ambient_light(color: #5a6a9a, ground: #3a2a20, intensity: 0.32)
```

- **Lights are hidden shapes.** `at` is the position (x, y, z in px, as for meshes: z grows away from the viewer), the shape's turns rotate `dir`, its fill is the colour and its opacity multiplies `intensity`, so `lamp.move_to(...)`, `lamp.fade_to(...)`, `fill_to`, `rotate_to`, keyframes and easing all work on lights.
- **Kinds.** `sun_light(dir, color, intensity, size, shadows)`; `point_light(at, color, intensity, reach, size, shadows)`; `spot_light(at, dir, angle, softness, color, intensity, reach, size, shadows)`; `ambient_light(color, ground, intensity)` (a hemisphere: `color` from above, `ground` from below, flat if omitted). `intensity` of a point or spot is the irradiance at `reach` px and falls with the square of the distance (not below a tenth of `reach`); `size` is the emitter's radius in px (the sun's angular size in degrees) and makes shadows soft; a spot's `angle` is its half-angle (up to 90 degrees) and `softness` the share of it that fades. `shadows: false` turns a light's shadows off.
- **Surfaces.** `mesh.surface(roughness: 0.6, metallic: 0, emissive: 0)`: GGX specular, metals take their highlight colour from the fill, emissive meshes glow (and bloom). `mirror(...)` still gives screen-space reflections, now of the lit picture.
- **Stage settings.** `exposure` (1), `ao` (ambient occlusion distance in px, default 60 and only with an ambient light, 0 off), `bloom` (strength 0 to 4, default 0), `quality` ("low", "medium", "high": 4, 8 or 16 rays per soft shadow and AO).
- **Shadows and occlusion.** Every opaque mesh casts shadows (rays against a deterministic BVH); soft shadows and AO use fixed spiral patterns turned by a hash of the pixel, so a frame is the same on every run and every machine, whatever the thread count. Glass and meshes with opacity under 0.5 do not cast.
- **2D shapes.** `shape.lit(bevel: 0, roughness: 0.6, metallic: 0, emissive: 0, occluder: false, depth: 0)` makes a flat shape take the light as a sheet facing the viewer at `depth` (z, px); `bevel` (px) rounds its edges for shading; it is shaded, shadowed by meshes and by other occluders, and with `occluder: true` it casts shadows on 3D meshes and on other lit sheets (anywhere its alpha is solid). An unlit 2D shape is never touched. A sheet is lit from the viewer's side only: put 2D lights and depths so the light is nearer than the sheet.
- **Bloom** is applied to the whole finished picture, so emissive 2D and 3D things glow alike.
- **Debug.** `--debug-physics` draws lights: a yellow ring for point and spot lights (with a line along a spot's axis), a line for a sun's direction.
- **Errors.** `E-LIT-001` unknown setting or wrong use (`lit` on a mesh), `E-LIT-002` bad value (negative intensity, zero `dir`, `angle` over 90 degrees, ...).
- **Limits.** No global illumination, caustics or environment maps (metals reflect through `mirror` or look dark away from highlights); water and glass are composited after lighting, so they carry the light only through the picture beneath them; a lit frame costs about 3 to 5 s at 1280x720; 2D lit shapes ignore the 2D camera's rotation for light positions (lights live in canvas coordinates).

Example: `examples/lamplight.anim` (a swinging soft-shadow lamp, a 2D figure lit and casting a shadow on a 3D wall, emissive bulb with bloom, ambient occlusion).

## 13z. Music as code (round A)

A `score` describes music: a tempo, a key and tracks that play notes, chords or drum patterns. `play(score)` puts it on the project's timeline and the picture can listen to it. Sound is rendered by `anim audio`, entirely in Rust and deterministically (the same program gives the same WAV bytes).

```
score night:
    tempo: 96, meter: "4/4", key: "A minor", reverb: 0.3, delay: 0.12, swing: 0.15
    track strings: instrument: "pad", chords: "i | VI | III | VII", repeat: 3, volume: 0.55, pan: -0.15
    track bass: instrument: "synth_bass", notes: ["A2:1/8 . A2:1/8 A2:1/8 . E3:1/8 . A2:1/8", "F2:1/8 ..."], bars: 1
    track drums: instrument: "drumkit", beat: "k...h...s...h..h", steps: 16, repeat: 12, bars: 2
    track bells: instrument: "bell", chords: "i | VI | III | VII", style: "arp_up", rate: "1/8", octave: 5

scene "Night" for 14.seconds:
    main:
        play(night, fade_in: 1.second)
        wait 12.seconds
        stop(night, fade_out: 2.seconds)
```

- **Score settings:** `tempo` (bpm, 20 to 400), `meter` (`"4/4"`, `"3/4"`, `"6/8"`), `key` (`"A minor"`, `"F# major"`), `swing` (0 to 1: delays the off-beat eighths), `reverb`, `room`, `delay`, `delay_time`, `delay_feedback` (0 to 1 unless noted), `master` (gain), `rate` (sample rate, default 44100), `library` (folder of sample libraries, below).
- **Tracks:** `instrument` plus exactly one of `notes`, `chords` or `beat` (a text, or a list of texts that are joined as separate bars); `volume`, `pan` (-1 to 1), `repeat` (how many times to play it), `bars` (bars of silence before it starts), `transpose` (semitones), `vel` (0 to 1), `humanize` (0 to 1: each note's start moves by up to 40 ticks and its velocity by up to 0.15 times that, from a seed made of the score and track names, so every render is the same), and the curves below.
- **Notes:** `C4:1/4 E4 G4:1/2 .:1/4 [C4 E4 G4]:1/2 | D4:1`. A pitch is a letter, `#` or `b`, and an octave (C4 is middle C); `:` gives the length as a fraction of a whole note (`1/4`; a trailing `.` dots it and `t` makes a triplet); a missing length repeats the last one; `.` or `r` is a rest; `[...]` plays together; `|` is a bar line, checked against the meter. Per-note control follows.
- **Articulation:** `'` staccato (held half the written length), `_` tenuto (the full length), `@0.7` any gate from above 0 to 1; the next note still comes at the written time. `^` accents, `!90` sets a velocity, `+20c` / `-35c` detunes in cents (up to 100). A velocity is not only loudness: harder notes are brighter (filter opening, FM index, harmonic balance).
- **Slurs and glides:** `(C4:1/4 D4 E4)` slurs: the notes connect, and on a built-in subtractive instrument (`sine`, `saw_lead`, `square_lead`, `pad`, `synth_bass`, `brass`, `flute`, `violin`, `cello`) they are one voice that is not retriggered and glides over 50 ms from note to note; plucked, FM, additive, drum and sampled instruments play each note separately. `C4~E4:1/2` is a glissando: one note that slides from the first pitch to the second over its length (every melodic built-in instrument and samples slide). `{D5}C5:1/4` is a grace note: a thirty-second note taken from the start of the note it leads into.
- **Dynamics:** the marks `ppp pp p mp mf f ff` (velocity 0.15, 0.25, 0.4, 0.55, 0.7, 0.85, 0.98; the default is mf) set the level of the notes after them. `<` (crescendo) and `>` (decrescendo) ramp the velocity of the notes between them and the next dynamic mark, which they must reach (a `<` that would end softer is an error); a note with its own `!vel` or `^` keeps it.
- **Curves (automation):** `volume_curve`, `pan_curve`, `cutoff_curve`, `brightness_curve` and `reverb_curve` change a track over time. A curve is text of points `bar:value` separated by spaces: `"0:0.2 4:1 8:0.5"` (bars count from the start of the score and may be fractional). A point may end in `s` (ease smoothly to the next) or `h` (hold until the next); the default is a straight line; before the first and after the last point the value is held. Ranges: volume 0 to 4 (multiplies `volume`), pan -1 to 1 (replaces `pan`), cutoff 0.05 to 8 (multiplies the filter cutoff; continuous on subtractive voices, so it sweeps inside a long note; FM and additive voices take it once per note), brightness 0 to 1 (0.5 leaves the tone; each step of 0.25 is an octave of filter), reverb send 0 to 2 (how much of the track feeds the reverb; the dry sound is unchanged; default 1).
- **Chords:** bars separated by `|`, the chords of a bar sharing it equally. Symbols are letters (`Am`, `G7`, `Cmaj7`, `Dm7b5`, `Csus4`, `F#dim`, `C/E` with a bass note) or roman numerals in the score's key (`i`, `VI`, `V7`, `bVII`: capital is major, small is minor). `%` repeats the last chord and `.` is a rest. `style` is `block` (default), `arp_up`, `arp_down` (one note per `rate`, such as `"1/8"`) or `strum` (`strum:` ticks apart); `hold` is how much of its time a chord sounds; `octave` is where the root sits (default 3).
- **Drums:** a grid of `steps` per bar (default 16): `k` kick, `s` snare, `h` closed hat, `o` open hat, `c` crash, `t` tom, `r` rim, `p` clap; a capital is accented, `.` a rest, spaces are ignored, `|` ends a bar.
- **Instruments:** built in: `sine`, `saw_lead`, `square_lead`, `pad` (`strings`, `synth_strings`), `synth_bass` (`bass`), `brass`, `flute`, `violin`, `cello`, `pluck` (`guitar`, `harp`), `bell`, `epiano`, `organ`, `piano`, `drumkit` (`drums`). They are synthesised (oscillators, envelopes, filters, plucked strings, FM, additive and noise drums); they sound like synths, not recordings.
- **Sample libraries:** `library: "samples/vsco2"` makes an instrument name that is not built in load `<library>/<name>.sfz`; `sfz: "path/Violin.sfz"` on a track names a file directly. SFZ files map WAV recordings over keys and velocities (`sample`, `key`, `lokey`, `hikey`, `pitch_keycenter`, `lovel`, `hivel`, `loop_mode`, `loop_start`, `loop_end`, `volume`, `<control> default_path`); loops stored in a WAV are used when the SFZ gives none. Samples must be WAV (PCM 8/16/24/32 bit or float): convert FLAC first. Libraries such as VSCO 2 and the Sonatina orchestra are not shipped here: download them, check their licences, and point `library:` at them (paths are relative to the program file).
- **Playing:** `play(score, volume: 1, fade_in: 0.s, fade_out: 0.s, loop: false, offset: 0.s)` and `stop(score, fade_out: 0.s)`, or `score.play(...)` and `score.stop(...)`, while a scene runs. A looped score repeats every whole number of bars. Scenes continue one project clock.
- **Listening:** a score has members: `playing`, `time` (seconds into it), `beat` and `bar` (phase 0 to 1), `pulse` (1 at each beat, fading), `beat_now` and `bar_now` (true for the one tick a beat or bar starts, so `whenever night.beat_now:` fires on every beat), and a level from 0 to 1 for each track by its name (`night.drums` jumps when that track plays a note). They are computed from the score, not from the rendered sound.
- **Rendering:** `anim audio program.anim -o sound.wav [--rate 48000]` writes the soundtrack, exactly as long as the project. To join it to frames: `anim frames program.anim -o frames` then `ffmpeg -framerate 24 -i frames/f%04d.png -i sound.wav -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest out.mp4`.
- **Errors:** `E-AUD-001` bad score or track setting, `E-AUD-002` notation error (names the track and the bad token), `E-AUD-003` unknown instrument, `E-AUD-004` bad `play` or `stop`, `E-AUD-005` bad curve.
- **Limits:** no MIDI import, no realtime playback (render, then listen), tempo is constant, no trills or ornaments beyond a single grace note, slurs glide only on built-in subtractive voices, curves cover volume, pan, filter, brightness and reverb send only (not delay send or per-note vibrato), sample libraries must be WAV, and the synthesised instruments are approximations.

Examples: `examples/nocturne.anim` (strings, bass, drums and bells; lit orbs and a glow that follow the music) and `examples/etude.anim` (a violin line with dynamics, slurs, a glissando and a grace note, a pad whose filter sweeps, bells that brighten and pan, a reverb send that opens); both videos have their soundtrack.

## 14. Diagnostics, tracing and the command line

Every diagnostic has a **severity** (`error`, `warning`, `info`), a **stable code**, a **source span**, a message,
and optional `note:` / `help:` lines. Codes never change meaning between versions.

```
error[E-TYP-002]: cannot add `Length` to `Duration`
  --> departure.anim:34:25
   |
34 |         bird.step_back(distance: 40.px + 1.second)
   |                                  ^^^^^^^^^^^^^^^^ `40.px` is a Length, `1.second` is a Duration
   |
help: did you mean `over: 1.second`?
```

### Diagnostic codes

| Phase | Codes |
|---|---|
| Lexing | `E-LEX-001` tab in indentation, `-002` bad dedent, `-003` unterminated string, `-004` unknown unit suffix |
| Parsing | `E-SYN-001` unexpected token, `-002` missing `:` before a block |
| Project | `E-PRJ-001` missing `anim_version`, `-002` experimental feature without opt-in |
| Names | `E-NAM-001` unknown name, `-002` duplicate/clashing declaration, `-003` entity not in `cast:` / unknown flag or event |
| Types | `E-TYP-001` type mismatch, `-002` unit mismatch, `-003` wrong number or name of arguments |
| Capability | `E-CAP-001` action cannot be resolved, `-002` ambiguous action, `-003` channel not declared |
| Triggers | `E-TRG-001` trigger is not a flag/predicate/event, `-002` event under `and`/`or`/`not` |
| State | `E-STA-001` `become`/`clear` on a derived flag, `-002` derived-flag cycle |
| Statements | `E-STM-001` blocking statement in `setup:`, `-003` zero-time loop body |
| Scenes | `E-SCN-001` `for auto` without `main:`, `-002` unknown set, `E-SCR-001` script heading names no known set |
| Properties | `E-CON-001` constraint cycle, `E-PRP-002` two locks on one component, `-003` overlapping keyframes |
| Fields | `E-FLD-001` bad field declaration, `-002` size limit, `-003` value became infinite or NaN |
| 3D meshes | `E-MSH-001` bad mesh or stage3 arguments |
| 3D fluids | `E-FLD-004` bad 3D fluid region, pour or fill that does not fit; `E-FLD-001` also for a bad `field:` link |
| 3D physics | `E-PHY-007` bad mesh body or `float`, `-008` bad 3D joint, `-009` bad soft mesh, pin or cut |
| Equations | `E-MTH-001` equation syntax, `-002` construct not allowed in an equation, `-003` bad arguments, `-004` unknown or mixed drive property, `-005` non-finite value |
| Camera / assets | `E-CAM-001` zoom outside [1, 8], `E-AST-001` asset file not found |
| Runtime | `E-RUN-001` tick did not settle in 64 deltas, `-002` action failed or rejected, `-004` `assert` failed |
| Warnings | `W-TIM-001` sub-frame duration, `W-SEQ-001` non-blocking after long blocking, `W-RUL-001` rule can never fire, `W-RUL-002` flag set but never read, `W-SCR-001` script line matched no phrase, `W-AUD-001` mix clipped, `W-CNT-001` entity outside canvas |

### The trace

`anim run --trace` prints one line per event: `tick`, `delta`, a kind, and details.

| Kind | Meaning |
|---|---|
| `SETUP` | a `setup:` statement ran |
| `DERIVE` | a derived flag changed |
| `EVENT` | an event was raised |
| `FIRE` | a rule fired and created a task |
| `STEP` | a task ran a zero-time statement |
| `ACT+` / `ACT-` / `ACT!` | action started / finished / interrupted, cancelled or failed |
| `WAIT-` | a `wait` ended |
| `TASK-` | a task ended |
| `AUDIO` | an audio statement ran (with start sample) |
| `NOTE` | a `note` or an unresolved script line |

### Commands

| Command | Purpose |
|---|---|
| `anim check file.anim` | Parse, type-check and capability-check; print diagnostics. Exit code 1 on errors |
| `anim run file.anim [--trace] [--hash] [--strict]` | Run the simulation without rendering |
| `anim explain frame N` | Print every contribution that produced each property in frame `N` |
| `anim explain why ENTITY.NAME [--at TICK]` | Print the causal chain for a flag, event or action |
| `anim render file.anim -o out.mp4 [--range A:B] [--draft]` | Render frames, mix audio, encode |
| `anim fmt file.anim [--to script]` | Format; optionally print a linear scene as a screenplay |
| `anim test dir/` | Run golden-trace tests |

### The editor

Ready-made programs for Linux and Windows (64-bit) are on the repository's **Releases** page: unpack the
archive and start `anim-editor` (`anim-editor.exe`) from its folder; the `anim` command line and the
examples are in the same archive. Windows shows an "unknown publisher" warning the first time, because
the programs are not signed. Or build from source as below.

`anim-editor [file.anim]` opens a desktop editor with six panels that can be dragged, resized, tabbed and
closed (`View` brings them back, `View > Reset layout` restores the default):

```
cargo run -p anim-editor --release -- examples/actions.anim
```

| Panel | What it shows | What clicking does |
|---|---|---|
| **Code** | The program, with syntax colours, line numbers and underlined errors and warnings | The program runs again 0.35 s after you stop typing |
| **Preview** | The frame at the playhead, drawn by the same renderer as `anim render` | Click a shape to select it; double-click to see its declaration; wheel zooms, drag pans, `Fit` resets |
| **Hierarchy** | Scenes, shapes (top-most first), entities and other declarations | Click selects (a scene also moves the playhead to its start); double-click shows the declaration |
| **Inspector** | The selected thing at the playhead: transform, style, what is animating it now, geometry, physics | Links lead to the source line |
| **Timeline** | Scenes, a ruler, and one row per animated shape: actions (blue), modifiers (green), keyframes (amber diamonds) | Drag on the ruler to scrub; click a bar to select its shape and see the statement that made it |
| **Console** | A run summary, errors and warnings with their help, `note` output, and the runtime trace (`Trace`) | Click a line to go to its source line (and tick, for a note) |

Keys: `Space` play or pause, `Left` / `Right` step one tick, `Home` / `End` first and last tick (when the
code is not being typed in), `F5` or `Ctrl+Enter` run now, `Ctrl+S` save, `Ctrl+Shift+S` save as, `Ctrl+O`
open, `Ctrl+N` new.

While the code has an error the console shows it and the other panels keep showing the last version that
ran, marked "last good run". An error raised while a scene is running keeps the frames made before it.
The panels do not edit the program in this version: the code is the only place to change things.

---

## 15. A complete example: "The Departure"

```anim
project "The Little Bird":
    anim_version: "0.1"
    canvas: 1280x720
    fps: 24
    seed: 7
    sample_rate: 48000

sprite bird_art: "assets/bird.svg"
sprite rose_art: "assets/rose.svg"
clip   bird_flap: "assets/bird_flap.clip"
sound  wind:      "assets/wind.ogg"

set garden:
    layers:
        - sprite: "assets/garden_sky.svg"    parallax: 0.1
        - sprite: "assets/garden_ground.svg" parallax: 1.0
    ground_y: 560.px

character bird:
    rig: sprite_rig(bird_art, pivot: (0.5, 0.9), features: [gaze, wings],
                    clips: { flap: bird_flap })
    at: (900.px, 520.px)
    z: 5
    states: is_ready, feels_afraid
    derive decides_to_leave when feels_afraid.held_for(1.5.seconds)

prop rose:
    sprite: rose_art
    at: (600.px, 540.px)
    z: 3

scene "The Departure" in garden for 10.seconds:
    cast: bird, rose

    setup:
        bird becomes is_ready

    when bird.is_ready:                      # R1
        bird.look_at(rose)
        wait 1.second
        bird becomes feels_afraid

    when bird.feels_afraid:                  # R2
        bird.step_back()
        bird.look_around()

    when bird.decides_to_leave:              # R3
        bird.fly_away()

    when bird.leaves_screen:                 # R4
        together:
            camera.hold(2.seconds)
            sound.play(wind)
```

What happens, and why:

1. `setup:` sets `is_ready` at tick 0, so **R1** fires in the next delta of tick 0.
2. R1 looks at the rose (12 frames), waits a second (24 frames), then sets `feels_afraid` at tick 36.
3. **R2** fires in the *same tick* (a zero-time chain). The bird steps back (36 → 48) and looks around from tick 48.
4. `decides_to_leave` is derived: `feels_afraid` has been true for exactly 36 ticks, so it becomes true at tick 72.
5. **R3** starts `fly_away`. It uses the `gaze` channel with `interrupt`, so it cuts off `look_around` after 24 of its
   36 frames and cancels R2's task.
6. When the bird's bounds leave the canvas, `leaves_screen` is raised one tick later and **R4** holds the camera
   and plays the wind at the same time.

The same scene as a plain screenplay, when it doesn't need to react:

```anim
scene "The Departure" in garden for 10.seconds:
    cast: bird, rose
    screenplay:
        EXT. GARDEN - DAWN

        The BIRD looks at the ROSE.
        (1 second)
        The BIRD steps back.
        The BIRD looks around.
        The BIRD flies away.

        SOUND: wind
        HOLD FOR TWO SECONDS.
```

---

## 16. Quick reference

**Reserved words:** `project import sprite sound music clip font character prop set camera scene shot shots cast
setup main timeline screenplay when whenever while if else derive states event var can rig features action uses
requires on_conflict conflict together race repeat times wait start await cancel finally become clear emit let place
lock release constrain attach cut dissolve assert fresh note and or not true false in for auto priority retrigger once`

Also reserved for future use: `after`, `then`, `continue`. `shape` is also a keyword.

**Statement cheat sheet**

```anim
bird becomes feels_afraid          # set an authored flag
bird clears feels_afraid           # clear it
emit bird.chirped                  # raise a custom event
bird.look_at(rose)                 # blocking action
wait 1.second                      # blocking wait
together:                          # all at once, resume when all finish
race:                              # resume when the first finishes
repeat 3 times:                    # counted loop (body must block)
repeat until bird.on_screen:       # conditional loop
let h = start bird.flap()          # background action
await h  /  cancel h
sound.play(wind)                   # non-blocking audio
let x = 5.seconds                  # local binding
if cond:  /  else:                 # branching
assert expr                        # check
note "text"                        # trace note
```

**Rule cheat sheet**

```anim
when T:                      # once per scene
whenever T:                  # every occurrence
whenever T retrigger restart:
while C:                     # while the condition holds
    ...
    finally:
        ...
when T if guard priority 10:
```
