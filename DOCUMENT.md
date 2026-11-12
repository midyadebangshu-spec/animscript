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
- Optional first lines of a definition: `uses body` and `priority 2` (recorded for documentation) and
  `requires n > 0` (checked on every call; `E-CAP-002` when false).
- `import "file.anim"` (relative to the importing file) or `import "std/motion"` (bundled) includes
  definitions. Library files are plain AnimScript: see `crates/lang/lib/motion.anim`, and copy and change them freely.
- `atan2(y, x)` gives an Angle; `angle_to(a, b)` (from `std/motion`) is the direction from one point to another.
- A definition cannot yet read a property's current *animated* value, so an action cannot follow a moving target
  frame by frame. Worked example: `examples/actions.anim`.

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
