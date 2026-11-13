//! The tick loop (spec §6.2, reduced to what the shape system needs): ingest, delta cycles with
//! rule dispatch and task stepping, action sampling, property resolution and snapshots.
use crate::ast::*;
use crate::color::{to_linear, to_srgb8};
use crate::interp::*;
use crate::layers::{fold, BlendMode, Contribution};
use crate::parser::parse;
use crate::procedural::value_noise;
use crate::snapshot::{build_snapshot, snapshot_hash, Resolved};
use crate::value::*;
use animscript_core::*;
use serde_json::Value as J;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Default, Clone, Debug)]
pub struct RunOptions {
    pub trace: bool,
}

pub struct RunResult {
    pub snapshots: Vec<J>,
    pub hashes: Vec<u64>,
    pub diagnostics: Vec<Diagnostic>,
    pub trace: Vec<String>,
}

struct ActComp {
    prop: PropId,
    from: f64,
    to: f64,
    motion: Motion,
}
struct Act {
    id: u32,
    task: u32,
    shape: String,
    channels: Vec<u32>,
    comps: Vec<ActComp>,
    start: i64,
    end: i64,
    ease: Ease,
    prio: i64,
    last: BTreeMap<PropId, f64>,
    /// Tasks suspended in `await` on this action.
    waiters: Vec<u32>,
}
/// A layer-3 contribution: keys for one scalar between `from` and `to` (inclusive; `None` = until the scene ends).
struct KeySet {
    id: u32,
    shape: String,
    prop: PropId,
    add: bool,
    keys: Vec<(i64, f64, Ease)>,
    from: i64,
    to: Option<i64>,
    blend_in: i64,
    blend_out: i64,
}
/// A layer-2 contribution: a procedural modifier running from `from` until `to` (exclusive; `None` = until cancelled).
struct ModSet {
    id: u32,
    shape: String,
    kind: ModKind,
    comps: Vec<ModComp>,
    from: i64,
    to: Option<i64>,
    blend_in: i64,
    blend_out: i64,
}
/// A lock: a constant layer-4 value for one scalar from `from` until `to` (exclusive; `None` = released by hand).
struct LockRt {
    handle: u32,
    shape: String,
    prop: PropId,
    value: f64,
    from: i64,
    to: Option<i64>,
}
enum LinkKind {
    /// The child follows the rigid motion of the parent's frame (anchor point and direction) since `q0`/`rot0`.
    Attach {
        child: String,
        parent: String,
        q0: [f64; 2],
        rot0: f64,
    },
    Inside {
        shape: String,
        rect: [f64; 4],
        weight: f64,
    },
    Follow {
        shape: String,
        target: String,
        lag: usize,
        weight: f64,
        offset: [f64; 2],
        hist: Vec<[f64; 2]>,
    },
}
struct LinkRt {
    handle: u32,
    kind: LinkKind,
}
/// A named channel held by one call of an action.
struct Occ {
    shape: String,
    ch: String,
    task: u32,
    call: u64,
    prio: i64,
}
struct NamedWait {
    task: u32,
    req: AcquireReq,
    seq: u64,
}

#[derive(Clone, Copy, Debug)]
enum HandleState {
    Lock,
    Link,
    /// Waiting for its channels.
    Queued,
    Act(u32),
    Mod(u32),
    /// A user-defined action started with `start`, running as this task.
    Task(u32),
}
struct PendingAct {
    req: ActionReq,
    task: u32,
    prio: i64,
    seq: u64,
    /// The `start` handle of this action (0 = none) and tasks waiting for it.
    handle: u32,
    waiters: Vec<u32>,
}
#[derive(Default, Clone)]
struct RuleSt {
    prev: bool,
    spent: bool,
    running: Option<u32>,
}

pub struct Runtime {
    pub it: Interp,
    tasks: BTreeMap<u32, Task>,
    next_task: u32,
    acts: BTreeMap<u32, Act>,
    next_act: u32,
    pending: Vec<PendingAct>,
    seq: u64,
    channels: BTreeMap<(String, u32), u32>,
    rules: Vec<RuleSt>,
    resolved: Resolved,
    keysets: Vec<KeySet>,
    modsets: Vec<ModSet>,
    handles: BTreeMap<u32, HandleState>,
    main_task: Option<u32>,
    main_done: bool,
    /// An error raised by a `finally:` block while a task was being cancelled.
    deferred_err: Option<Diagnostic>,
    /// The running scene's timeline markers: name and tick.
    markers: Vec<(String, i64)>,
    /// The scene's rules followed by the rules written inside shots, and the shot each belongs to.
    rules_all: Rc<Vec<Rule>>,
    rule_shot: Vec<Option<usize>>,
    shots: Rc<Vec<ShotItem>>,
    cur_shot: Option<usize>,
    shots_started: bool,
    shot_start: i64,
    shot_len: Option<i64>,
    shot_task: Option<u32>,
    shot_until_prev: bool,
    locks: Vec<LockRt>,
    links: Vec<LinkRt>,
    named: Vec<Occ>,
    named_wait: Vec<NamedWait>,
    project_tick: i64,
    scene_name: String,
    snapshots: Vec<J>,
    hashes: Vec<u64>,
}

/// Parse, build and run a program, returning one snapshot (and hash) per tick.
pub fn run(src: &str, opts: &RunOptions) -> Res<RunResult> {
    run_in(src, opts, std::path::Path::new("."))
}

/// Like `run`, with the folder that relative asset paths (images) are resolved from.
pub fn run_in(src: &str, opts: &RunOptions, base: &std::path::Path) -> Res<RunResult> {
    let (src, opts, base) = (src.to_string(), opts.clone(), base.to_path_buf());
    crate::on_big_stack(move || run_in_here(&src, &opts, &base))
}

fn run_in_here(src: &str, opts: &RunOptions, base: &std::path::Path) -> Res<RunResult> {
    let src = &crate::imports::expand_imports(src, base)?;
    let program = parse(src)?;
    if program.project.is_none() {
        return Err(
            Diagnostic::new("E-PRJ-001", "a runnable program needs a `project` block").help(
                "start the file with `project \"name\":` giving `anim_version`, `canvas` and `fps`",
            ),
        );
    }
    let mut it = Interp::new(src, program)?;
    it.base_dir = base.to_path_buf();
    it.trace_on = opts.trace;
    it.build()?;
    let mut rt = Runtime::new(it);
    let scenes: Vec<Rc<Scene>> = rt.it.program.scenes.clone();
    for sc in &scenes {
        rt.run_scene(sc)?;
    }
    Ok(RunResult {
        snapshots: rt.snapshots,
        hashes: rt.hashes,
        diagnostics: rt.it.diagnostics,
        trace: rt.it.trace,
    })
}

impl Runtime {
    fn new(it: Interp) -> Runtime {
        Runtime {
            it,
            tasks: BTreeMap::new(),
            next_task: 1,
            acts: BTreeMap::new(),
            next_act: 1,
            pending: vec![],
            seq: 0,
            channels: BTreeMap::new(),
            rules: vec![],
            resolved: Resolved::default(),
            keysets: vec![],
            modsets: vec![],
            handles: BTreeMap::new(),
            main_task: None,
            main_done: false,
            deferred_err: None,
            markers: vec![],
            rules_all: Rc::new(vec![]),
            rule_shot: vec![],
            shots: Rc::new(vec![]),
            cur_shot: None,
            shots_started: false,
            shot_start: 0,
            shot_len: None,
            shot_task: None,
            shot_until_prev: false,
            locks: vec![],
            links: vec![],
            named: vec![],
            named_wait: vec![],
            project_tick: 0,
            scene_name: String::new(),
            snapshots: vec![],
            hashes: vec![],
        }
    }

    fn base_vars(&self) -> Vars {
        Vars {
            scopes: vec![self.it.globals.clone()],
        }
    }

    fn run_scene(&mut self, sc: &Rc<Scene>) -> Res<()> {
        let frames: Option<i64> = match &sc.length {
            SceneLen::Auto => {
                if sc.main.is_none() {
                    return Err(Diagnostic::new(
                        "E-SCN-001",
                        "`for auto` needs a `main:` block to know when the scene ends",
                    )
                    .at(sc.span));
                }
                None
            }
            SceneLen::Expr(e) => {
                let v = self.it.eval(e, &Vars::new(), Mode::Build)?;
                Some(self.it.frames_of(&v, e.span)?)
            }
        };
        self.tasks.clear();
        self.acts.clear();
        self.keysets.clear();
        self.modsets.clear();
        self.handles.clear();
        self.locks.clear();
        self.links.clear();
        self.named.clear();
        self.named_wait.clear();
        self.it.scene_name = sc.name.clone();
        self.it.decls.start_scene();
        self.it.reset_particles();
        self.it.reset_fluids();
        self.it.mod_counts.clear();
        self.pending.clear();
        self.channels.clear();
        let mut all: Vec<Rule> = sc.rules.clone();
        let mut owner: Vec<Option<usize>> = vec![None; all.len()];
        for (i, item) in sc.shots.iter().enumerate() {
            if let ShotItem::Shot(sh) = item {
                for r in &sh.rules {
                    all.push(r.clone());
                    owner.push(Some(i));
                }
            }
        }
        self.rules = vec![RuleSt::default(); all.len()];
        self.rules_all = Rc::new(all);
        self.rule_shot = owner;
        self.shots = Rc::new(sc.shots.clone());
        self.cur_shot = None;
        self.shots_started = false;
        self.shot_task = None;
        self.it.shot_name = None;
        self.it.transition = None;
        self.it.scene_frames = frames;
        // every scene starts from the establishing shot: the whole canvas, no rotation
        if let Some(cam) = self.it.shapes.get_mut("camera") {
            cam.position = [0.0, 0.0];
            cam.scale = [1.0, 1.0];
            cam.rotation = 0.0;
        }
        self.it.flag_hist.clear();
        self.main_task = None;
        self.main_done = false;
        self.scene_name = sc.name.clone();
        let mut n = 0i64;
        loop {
            self.it.tick = n;
            self.it.delta = 0;
            self.ingest();
            if frames.is_some_and(|f| n >= f) {
                break; // phase 2(e): the scene ends before this tick is simulated
            }
            if n == 0 {
                if let Some(setup) = &sc.setup {
                    let mut t = Task::new(0, Rc::clone(setup), self.base_vars(), Mode::Setup, true);
                    self.it.run_sync(&mut t)?;
                }
                self.it.freeze_pivots(); // pivots default to the bounding-box centre once building is over
                self.it.build_physics()?; // the world starts from the shapes' poses after setup
                self.it.build_soft()?;
                self.it.fill_fluids()?;
                self.it.build_rigs()?;
                self.register_timeline(sc)?;
                if let Some(main) = &sc.main {
                    let id = self.new_task_id();
                    let t = Task::new(id, Rc::clone(main), self.base_vars(), Mode::Run, true);
                    self.main_task = Some(id);
                    self.tasks.insert(id, t);
                }
            }
            // delta 0: tasks made ready in phase 2 (and, at tick 0, setup + main) run before any rule is looked at
            self.step_tasks()?;
            self.it.apply_staged()?;
            if n == 0 {
                self.it.events.insert("scene.starts".into());
            }
            for (name, at) in &self.markers {
                if *at == n {
                    self.it.events.insert(format!("marker:{name}"));
                }
            }
            let mut d = 1u32;
            loop {
                self.it.delta = d;
                self.it.update_flags()?;
                self.update_shots()?;
                self.dispatch_rules(sc)?;
                self.step_tasks()?;
                if self.it.apply_staged()? == 0 {
                    break;
                }
                d += 1;
                if d > 64 {
                    return Err(Diagnostic::new("E-RUN-001", format!("tick {n} did not settle after 64 delta cycles"))
                        .help("a chain of zero-time reactions must end; add `wait 1.frame` to one of the rules involved"));
                }
            }
            self.advance_actions();
            let snap = build_snapshot(
                &self.it,
                &self.scene_name,
                n,
                self.project_tick,
                &self.resolved,
            );
            self.hashes.push(snapshot_hash(&snap));
            self.snapshots.push(snap);
            self.project_tick += 1;
            n += 1;
            if frames.is_none() && self.main_done {
                break;
            }
        }
        self.end_scene()?;
        Ok(())
    }

    fn new_task_id(&mut self) -> u32 {
        let id = self.next_task;
        self.next_task += 1;
        id
    }

    fn key_free(&self, req: &ActionReq) -> bool {
        req.channels
            .iter()
            .all(|c| !self.channels.contains_key(&(req.shape.clone(), *c)))
    }

    /// The current colour (linear light) a colour property starts from: its own, else the shape's.
    fn colour_rest(&self, shape: &str, prop: PropId) -> Option<[f64; 4]> {
        let sh = &self.it.shapes[shape];
        match prop {
            PropId::FillC(f, _) => sh.faces[&f].fill.or(sh.fill).map(to_linear),
            PropId::StrokeC(e, _) => sh.edges[&e].stroke.or(sh.stroke).map(to_linear),
            PropId::ShapeFillC(_) => sh.fill.map(to_linear),
            PropId::ShapeStrokeC(_) => sh.stroke.map(to_linear),
            PropId::TintC(_) => Some(to_linear(sh.tint)),
            _ => None,
        }
    }

    fn colour_comp(prop: PropId) -> usize {
        match prop {
            PropId::FillC(_, c)
            | PropId::StrokeC(_, c)
            | PropId::ShapeFillC(c)
            | PropId::ShapeStrokeC(c)
            | PropId::TintC(c) => c as usize,
            _ => 0,
        }
    }

    /// The rest (layer 0) value of one scalar property.
    fn rest_value(&self, shape: &str, prop: PropId) -> f64 {
        let sh = &self.it.shapes[shape];
        match prop {
            PropId::Vx(i) => sh.verts[&i].x,
            PropId::Vy(i) => sh.verts[&i].y,
            PropId::PosX => sh.position[0],
            PropId::PosY => sh.position[1],
            PropId::ScaleX => sh.scale[0],
            PropId::ScaleY => sh.scale[1],
            PropId::Rot => sh.rotation,
            PropId::EdgeWidth(e) => sh.edges[&e].width.unwrap_or(sh.stroke_width),
            PropId::ShapeWidth => sh.stroke_width,
            PropId::Opacity => sh.opacity,
            c => self
                .colour_rest(shape, c)
                .map_or(0.0, |col| col[Self::colour_comp(c)]),
        }
    }

    fn set_rest(&mut self, shape: &str, prop: PropId, v: f64) {
        if prop.is_colour() {
            let mut col = self.colour_rest(shape, prop).unwrap_or([0.0; 4]);
            col[Self::colour_comp(prop)] = v;
            let c8 = to_srgb8(col);
            let Some(sh) = self.it.shapes.get_mut(shape) else {
                return;
            };
            match prop {
                PropId::FillC(f, _) => {
                    if let Some(x) = sh.faces.get_mut(&f) {
                        x.fill = Some(c8);
                    }
                }
                PropId::StrokeC(e, _) => {
                    if let Some(x) = sh.edges.get_mut(&e) {
                        x.stroke = Some(c8);
                    }
                }
                PropId::ShapeFillC(_) => sh.fill = Some(c8),
                PropId::ShapeStrokeC(_) => sh.stroke = Some(c8),
                _ => sh.tint = c8,
            }
            return;
        }
        let Some(sh) = self.it.shapes.get_mut(shape) else {
            return;
        };
        match prop {
            PropId::Vx(i) => {
                if let Some(x) = sh.verts.get_mut(&i) {
                    x.x = v;
                }
            }
            PropId::Vy(i) => {
                if let Some(x) = sh.verts.get_mut(&i) {
                    x.y = v;
                }
            }
            PropId::EdgeWidth(e) => {
                if let Some(x) = sh.edges.get_mut(&e) {
                    x.width = Some(v.max(0.0));
                }
            }
            PropId::ShapeWidth => sh.stroke_width = v.max(0.0),
            PropId::Opacity => sh.opacity = v.clamp(0.0, 1.0),
            other => set_transform_prop(sh, other, v, false),
        }
    }

    // ---- phase 2 -----------------------------------------------------------------------

    fn ingest(&mut self) {
        let tick = self.it.tick;
        // key sets past their last tick simply stop: the layer below shows through again
        self.keysets.retain(|k| k.to.is_none_or(|t| tick <= t));
        self.modsets.retain(|m| m.to.is_none_or(|t| tick < t));
        let done: Vec<u32> = self
            .acts
            .values()
            .filter(|a| a.end <= tick)
            .map(|a| a.id)
            .collect();
        for id in done {
            let a = self.acts.remove(&id).unwrap();
            self.commit(&a, false);
            self.it
                .trace_line("ACT-", &format!("{} {}", a.shape, a.comps.len()));
            for w in a.waiters.iter().chain(std::iter::once(&a.task)) {
                if let Some(t) = self.tasks.get_mut(w) {
                    t.state = TaskState::Ready;
                }
            }
        }
        for t in self.tasks.values_mut() {
            if t.state == TaskState::Waiting && t.wake <= tick {
                t.state = TaskState::Ready;
            }
        }
        // grant pass: queued actions start in priority order, then request order
        let mut pend = std::mem::take(&mut self.pending);
        pend.sort_by(|a, b| b.prio.cmp(&a.prio).then(a.seq.cmp(&b.seq)));
        for p in pend {
            if self.key_free(&p.req) {
                self.start_action(p);
            } else {
                self.pending.push(p);
            }
        }
    }

    /// Commit an action's values to the rest layer (spec 7.6): the final value when it finished,
    /// the last sampled value when it was cut off.
    fn commit(&mut self, a: &Act, interrupted: bool) {
        for c in &a.comps {
            let v = if interrupted {
                a.last.get(&c.prop).copied().unwrap_or(c.from)
            } else {
                c.to
            };
            self.set_rest(&a.shape, c.prop, v);
        }
        for ch in &a.channels {
            self.channels.remove(&(a.shape.clone(), *ch));
        }
    }

    fn start_action(&mut self, p: PendingAct) -> u32 {
        let tick = self.it.tick;
        let fps = self.it.fps.to_f64();
        let dur_secs = p.req.frames as f64 / fps;
        let mut comps: Vec<ActComp> = vec![];
        for (i, (prop, t)) in p.req.comps.iter().enumerate() {
            let motion = p.req.motions.get(i).copied().unwrap_or(Motion::Eased);
            let from = self.rest_value(&p.req.shape, *prop);
            let to = if motion != Motion::Eased {
                motion_value(&motion, from, dur_secs)
            } else {
                match t {
                    Tgt::To(v) => *v,
                    Tgt::By(d) => from + d,
                }
            };
            comps.push(ActComp {
                prop: *prop,
                from,
                to,
                motion,
            });
        }
        // animating a colour that is not set yet fades in from transparent, without a colour shift
        for c in comps.iter_mut() {
            if c.prop.is_colour() && self.colour_rest(&p.req.shape, c.prop).is_none() {
                c.from = if Self::colour_comp(c.prop) == 3 {
                    0.0
                } else {
                    c.to
                };
            }
        }
        let id = self.next_act;
        self.next_act += 1;
        for ch in &p.req.channels {
            self.channels.insert((p.req.shape.clone(), *ch), id);
        }
        let end = tick + p.req.frames;
        self.it.trace_line(
            "ACT+",
            &format!("T{} {} [{} -> {}]", p.task, p.req.label, tick, end),
        );
        if p.handle != 0 {
            self.handles.insert(p.handle, HandleState::Act(id));
        }
        self.acts.insert(
            id,
            Act {
                id,
                task: p.task,
                shape: p.req.shape,
                channels: p.req.channels,
                comps,
                start: tick,
                end,
                ease: p.req.ease,
                prio: p.prio,
                last: BTreeMap::new(),
                waiters: p.waiters,
            },
        );
        id
    }

    /// Pick up what the last step produced: key sets, started actions and modifiers, cancellations.
    fn take_keysets(&mut self) {
        let specs = std::mem::take(&mut self.it.pending_keysets);
        let base = self.it.tick;
        for s in specs {
            self.add_keyset(s, base);
        }
        for (h, req) in std::mem::take(&mut self.it.pending_starts) {
            match req {
                StartReq::Act(a) => {
                    let p = PendingAct {
                        req: a,
                        task: 0,
                        prio: 0,
                        seq: self.seq,
                        handle: h,
                        waiters: vec![],
                    };
                    self.seq += 1;
                    if self.key_free(&p.req) {
                        self.start_action(p);
                    } else {
                        if h != 0 {
                            self.handles.insert(h, HandleState::Queued);
                        }
                        self.pending.push(p);
                    }
                }
                StartReq::Mod(m) => self.start_mod(m, h),
                StartReq::Task(c) => {
                    let id = self.new_task_id();
                    let vars = Vars {
                        scopes: vec![self.it.globals.clone(), c.scope.clone()],
                    };
                    let mut t = Task::new(id, Rc::clone(&c.def.body), vars, Mode::Run, false);
                    self.it.next_call += 1;
                    t.frames.insert(
                        0,
                        Frame::Call {
                            saved: None,
                            id: self.it.next_call,
                            uses: Rc::new(c.def.uses.iter().map(|u| u.0.clone()).collect()),
                        },
                    );
                    self.it
                        .trace_line("TASK+", &format!("T{id} {}", c.def.name));
                    self.tasks.insert(id, t);
                    if h != 0 {
                        self.handles.insert(h, HandleState::Task(id));
                    }
                }
            }
        }
        for h in std::mem::take(&mut self.it.pending_cancels) {
            self.cancel_handle(h);
        }
        for (h, spec) in std::mem::take(&mut self.it.pending_locks) {
            self.add_lock(h, spec);
        }
        for (h, spec) in std::mem::take(&mut self.it.pending_links) {
            self.add_link(h, spec);
        }
        for h in std::mem::take(&mut self.it.pending_releases) {
            self.cancel_handle(h);
        }
    }

    fn add_lock(&mut self, handle: u32, spec: crate::interp::LockSpec) {
        let from = self.it.tick;
        let to = spec.frames.map(|f| from + f);
        for (shape, prop, value) in spec.comps {
            let clash = self.locks.iter().any(|l| {
                l.shape == shape
                    && l.prop == prop
                    && l.to.is_none_or(|t| t > from)
                    && to.is_none_or(|t| l.from < t)
            });
            if clash {
                self.deferred_err.get_or_insert(
                    Diagnostic::new(
                        "E-PRP-002",
                        format!("two locks hold `{shape}` {prop:?} at the same time"),
                    )
                    .at(spec.span)
                    .help("`release` the first lock, or give it `for D`"),
                );
                return;
            }
            self.locks.push(LockRt {
                handle,
                shape,
                prop,
                value,
                from,
                to,
            });
        }
        self.handles.insert(handle, HandleState::Lock);
        self.it.trace_line("LOCK+", &format!("h{handle}"));
    }

    /// Where a shape's rigid frame is: the pivot's place in the world and its direction in turns; a bone's
    /// head and direction. `resolved` is the transform set of the tick being resolved (else the last tick's).
    fn frame_of(&self, name: &str, resolved: Option<&Resolved>) -> ([f64; 2], f64) {
        if let Some(f) = self.it.bone_frame(name) {
            return f;
        }
        let sh = &self.it.shapes[name];
        let tf = match resolved {
            // while resolving: what this tick's fold produced, else the rest transform
            Some(r) => r.tf.get(name).copied(),
            None => self.it.live_tf.get(name).copied(),
        }
        .unwrap_or_else(|| sh.transform());
        (
            [tf.pivot[0] + tf.position[0], tf.pivot[1] + tf.position[1]],
            tf.rotation,
        )
    }

    fn add_link(&mut self, handle: u32, spec: crate::interp::LinkSpec) {
        use crate::interp::LinkSpec;
        let kind = match spec {
            LinkSpec::Attach {
                child,
                parent,
                offset,
                span,
            } => {
                // a parent chain that leads back to the child is a cycle
                let mut cur = parent.clone();
                let mut hops = 0;
                loop {
                    if cur == child {
                        self.deferred_err.get_or_insert(
                            Diagnostic::new(
                                "E-CON-001",
                                format!("attaching `{child}` to `{parent}` would make a cycle"),
                            )
                            .at(span)
                            .help("a shape cannot be its own ancestor"),
                        );
                        return;
                    }
                    let next = self.links.iter().find_map(|l| match &l.kind {
                        LinkKind::Attach {
                            child: c,
                            parent: p,
                            ..
                        } if *c == cur => Some(p.clone()),
                        _ => None,
                    });
                    match next {
                        Some(n) if hops < 1000 => {
                            cur = n;
                            hops += 1;
                        }
                        _ => break,
                    }
                }
                let (q0, rot0) = self.frame_of(&parent, None);
                if let Some(o) = offset {
                    // put the child's centre at the parent's anchor plus the offset (in the parent's frame)
                    let (s, c) = (rot0 * std::f64::consts::TAU).sin_cos();
                    let want = [q0[0] + o[0] * c - o[1] * s, q0[1] + o[0] * s + o[1] * c];
                    let sh = &self.it.shapes[&child];
                    let tf = self
                        .it
                        .live_tf
                        .get(&child)
                        .copied()
                        .unwrap_or_else(|| sh.transform());
                    let have = tf.apply(sh.bbox_center());
                    let sh = self.it.shapes.get_mut(&child).expect("shape");
                    sh.position[0] += want[0] - have[0];
                    sh.position[1] += want[1] - have[1];
                }
                LinkKind::Attach {
                    child,
                    parent,
                    q0,
                    rot0,
                }
            }
            LinkSpec::Inside {
                shape,
                rect,
                weight,
            } => LinkKind::Inside {
                shape,
                rect,
                weight,
            },
            LinkSpec::Follow {
                shape,
                target,
                lag,
                weight,
            } => {
                let (qt, _) = self.frame_of(&target, None);
                let (qs, _) = self.frame_of(&shape, None);
                LinkKind::Follow {
                    shape,
                    target,
                    lag: lag.max(0) as usize,
                    weight,
                    offset: [qs[0] - qt[0], qs[1] - qt[1]],
                    hist: vec![],
                }
            }
        };
        self.links.push(LinkRt { handle, kind });
        self.handles.insert(handle, HandleState::Link);
    }

    /// After the layer stack: children follow their parents (parents first), then the constraints, in the
    /// order they were made.
    fn apply_links(&mut self, resolved: &mut Resolved) {
        if self.links.is_empty() {
            return;
        }
        // attachments, shallowest parent first
        let depth = |links: &Vec<LinkRt>, name: &str| {
            let (mut cur, mut d) = (name.to_string(), 0);
            while let Some(p) = links.iter().find_map(|l| match &l.kind {
                LinkKind::Attach { child, parent, .. } if *child == cur => Some(parent.clone()),
                _ => None,
            }) {
                cur = p;
                d += 1;
                if d > 1000 {
                    break;
                }
            }
            d
        };
        let mut order: Vec<(usize, usize)> = self
            .links
            .iter()
            .enumerate()
            .filter_map(|(i, l)| match &l.kind {
                LinkKind::Attach { child, .. } => Some((depth(&self.links, child), i)),
                _ => None,
            })
            .collect();
        order.sort();
        let tau = std::f64::consts::TAU;
        for (_, i) in order {
            let LinkKind::Attach {
                child,
                parent,
                q0,
                rot0,
            } = &self.links[i].kind
            else {
                continue;
            };
            let (q, rot) = self.frame_of(parent, Some(resolved));
            let theta = (rot - rot0) * tau;
            let d = [q[0] - q0[0], q[1] - q0[1]];
            let sh = &self.it.shapes[child];
            let mut tf = resolved
                .tf
                .get(child)
                .copied()
                .unwrap_or_else(|| sh.transform());
            let w = [
                tf.pivot[0] + tf.position[0] - q0[0],
                tf.pivot[1] + tf.position[1] - q0[1],
            ];
            let (s, c) = theta.sin_cos();
            let w2 = [
                q0[0] + w[0] * c - w[1] * s + d[0],
                q0[1] + w[0] * s + w[1] * c + d[1],
            ];
            tf.position = [w2[0] - tf.pivot[0], w2[1] - tf.pivot[1]];
            tf.rotation += theta / tau;
            resolved.tf.insert(child.clone(), tf);
        }
        // constraints
        for i in 0..self.links.len() {
            enum Act {
                Shift(String, [f64; 2]),
                None,
            }
            let act = match &self.links[i].kind {
                LinkKind::Attach { .. } => Act::None,
                LinkKind::Inside {
                    shape,
                    rect,
                    weight,
                } => {
                    let sh = &self.it.shapes[shape];
                    let tf = resolved
                        .tf
                        .get(shape)
                        .copied()
                        .unwrap_or_else(|| sh.transform());
                    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                    for id in sh.live_ids(Kind::V) {
                        let p = resolved
                            .verts
                            .get(shape)
                            .and_then(|m| m.get(&id))
                            .copied()
                            .unwrap_or_else(|| sh.pos(id));
                        let p = tf.apply(p);
                        for k in 0..2 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                    let mut shift = [0.0; 2];
                    for k in 0..2 {
                        let (min, size) = (rect[k], rect[k + 2]);
                        shift[k] = if hi[k] - lo[k] > size {
                            min + size / 2.0 - (lo[k] + hi[k]) / 2.0
                        } else if lo[k] < min {
                            min - lo[k]
                        } else if hi[k] > min + size {
                            min + size - hi[k]
                        } else {
                            0.0
                        } * weight;
                    }
                    Act::Shift(shape.clone(), shift)
                }
                LinkKind::Follow {
                    shape,
                    target,
                    lag,
                    weight,
                    offset,
                    ..
                } => {
                    let (shape, target, lag, weight, offset) =
                        (shape.clone(), target.clone(), *lag, *weight, *offset);
                    let (qt, _) = self.frame_of(&target, Some(resolved));
                    let LinkKind::Follow { hist, .. } = &mut self.links[i].kind else {
                        unreachable!()
                    };
                    hist.push(qt);
                    let q = hist[hist.len().saturating_sub(1 + lag)];
                    let want = [q[0] + offset[0], q[1] + offset[1]];
                    let (have, _) = self.frame_of(&shape, Some(resolved));
                    Act::Shift(
                        shape,
                        [(want[0] - have[0]) * weight, (want[1] - have[1]) * weight],
                    )
                }
            };
            if let Act::Shift(shape, d) = act {
                let sh = &self.it.shapes[&shape];
                let mut tf = resolved
                    .tf
                    .get(&shape)
                    .copied()
                    .unwrap_or_else(|| sh.transform());
                tf.position[0] += d[0];
                tf.position[1] += d[1];
                resolved.tf.insert(shape, tf);
            }
        }
    }

    /// Stop a started user action: its own task, the tasks it spawned, and the actions it was running
    /// (which keep the last value they sampled, like `cancel` on any action).
    fn cancel_task(&mut self, id: u32) {
        let mut doomed = vec![id];
        let mut i = 0;
        while i < doomed.len() {
            let parent = doomed[i];
            doomed.extend(
                self.tasks
                    .iter()
                    .filter(|(_, t)| t.parent == Some(parent))
                    .map(|(k, _)| *k),
            );
            i += 1;
        }
        for d in doomed {
            let acts: Vec<u32> = self
                .acts
                .iter()
                .filter(|(_, a)| a.task == d)
                .map(|(k, _)| *k)
                .collect();
            for a in acts {
                if let Some(act) = self.acts.remove(&a) {
                    self.commit(&act, true);
                    for w in &act.waiters {
                        if let Some(t) = self.tasks.get_mut(w) {
                            t.state = TaskState::Ready;
                        }
                    }
                }
            }
            self.pending.retain(|p| p.task != d);
            if let Some(t) = self.tasks.remove(&d) {
                // a cancelled task still gets to run its `finally:` blocks
                for b in Interp::pending_finals(&t) {
                    if let Err(e) = self.it.run_final(&b, &t.vars, Mode::Run) {
                        self.deferred_err.get_or_insert(e);
                    }
                }
                for w in t.awaiters {
                    if let Some(x) = self.tasks.get_mut(&w) {
                        x.state = TaskState::Ready;
                    }
                }
            }
        }
    }

    fn start_mod(&mut self, m: ModReq, handle: u32) {
        let tick = self.it.tick;
        let id = self.next_act;
        self.next_act += 1;
        let to = m.frames.map(|f| tick + f);
        self.it.trace_line(
            "MOD+",
            &format!(
                "{} [{} -> {}]",
                m.label,
                tick,
                to.map_or("open".to_string(), |t| t.to_string())
            ),
        );
        if handle != 0 {
            self.handles.insert(handle, HandleState::Mod(id));
        }
        self.modsets.push(ModSet {
            id,
            shape: m.shape,
            kind: m.kind,
            comps: m.comps,
            from: tick,
            to,
            blend_in: m.blend_in,
            blend_out: m.blend_out,
        });
    }

    /// `cancel h`: an action keeps the last value it sampled (spec 7.6), a modifier simply stops.
    fn cancel_handle(&mut self, h: u32) {
        match self.handles.remove(&h) {
            Some(HandleState::Lock) => self.locks.retain(|l| l.handle != h),
            Some(HandleState::Link) => self.links.retain(|l| l.handle != h),
            Some(HandleState::Act(id)) => {
                if let Some(a) = self.acts.remove(&id) {
                    self.commit(&a, true);
                    for w in &a.waiters {
                        if let Some(t) = self.tasks.get_mut(w) {
                            t.state = TaskState::Ready;
                        }
                    }
                }
            }
            Some(HandleState::Mod(id)) => self.modsets.retain(|m| m.id != id),
            Some(HandleState::Task(id)) => self.cancel_task(id),
            Some(HandleState::Queued) => {
                let mut woken = vec![];
                self.pending.retain(|p| {
                    if p.handle == h {
                        woken.extend(p.waiters.iter().copied());
                        false
                    } else {
                        true
                    }
                });
                for w in woken {
                    if let Some(t) = self.tasks.get_mut(&w) {
                        t.state = TaskState::Ready;
                    }
                }
            }
            None => {}
        }
    }

    fn add_keyset(&mut self, s: KeySetSpec, base: i64) {
        let shift = if s.absolute { 0 } else { base };
        let keys: Vec<(i64, f64, Ease)> =
            s.keys.iter().map(|(t, v, e)| (t + shift, *v, *e)).collect();
        let (first, last) = (keys[0].0, keys[keys.len() - 1].0);
        let from = if s.hold_before { shift } else { first };
        let to = if s.hold_after { None } else { Some(last) };
        let id = self.next_act;
        self.next_act += 1;
        self.it.trace_line(
            "KEYS",
            &format!(
                "{} {:?} [{} -> {}]",
                s.shape,
                s.prop,
                from,
                to.map_or("end".to_string(), |t| t.to_string())
            ),
        );
        self.keysets.push(KeySet {
            id,
            shape: s.shape,
            prop: s.prop,
            add: s.add,
            keys,
            from,
            to,
            blend_in: s.blend_in,
            blend_out: s.blend_out,
        });
    }

    /// A scene's `timeline:` becomes key sets with absolute scene times.
    fn register_timeline(&mut self, sc: &Rc<Scene>) -> Res<()> {
        self.markers.clear();
        for (name, at, span) in &sc.markers {
            if self.markers.iter().any(|m| m.0 == *name) {
                return Err(
                    Diagnostic::new("E-NAM-002", format!("duplicate marker `{name}`")).at(*span),
                );
            }
            let v = self.it.eval(at, &self.base_vars(), Mode::Run)?;
            let tick = self.it.frames_of(&v, at.span)?;
            self.markers.push((name.clone(), tick));
        }
        if sc.timeline.is_empty() {
            return Ok(());
        }
        let vars = self.base_vars();
        let specs = self.it.build_keysets(
            None,
            &sc.timeline,
            None,
            &vars,
            Mode::Run,
            true,
            [0, 0],
            (false, false),
        )?;
        for s in specs {
            self.add_keyset(s, 0);
        }
        Ok(())
    }

    // ---- phases 3-5 --------------------------------------------------------------------

    fn shot_frames(&mut self, e: &Expr) -> Res<i64> {
        let v = self.it.eval(e, &self.base_vars(), Mode::Run)?;
        self.it.frames_of(&v, e.span)
    }

    fn begin_shot(&mut self, i: usize) -> Res<()> {
        let shots = Rc::clone(&self.shots);
        let ShotItem::Shot(sh) = &shots[i] else {
            return Ok(());
        };
        self.cur_shot = Some(i);
        self.shot_start = self.it.tick;
        self.shot_until_prev = false;
        self.shot_len = match &sh.end {
            ShotEnd::For(d) => Some(self.shot_frames(d)?),
            _ => None,
        };
        self.it.shot_name = Some(sh.name.clone());
        self.it.trace_line("SHOT+", &sh.name);
        let id = self.new_task_id();
        let t = Task::new(id, Rc::clone(&sh.body), self.base_vars(), Mode::Run, true);
        self.tasks.insert(id, t);
        self.shot_task = Some(id);
        Ok(())
    }

    fn end_shot(&mut self, i: usize) {
        if let Some(t) = self.shot_task.take() {
            if self.tasks.contains_key(&t) {
                self.cancel_task(t);
            }
        }
        // the shot's rules stop listening, and what they were running stops with them
        let owned: Vec<usize> = (0..self.rule_shot.len())
            .filter(|r| self.rule_shot[*r] == Some(i))
            .collect();
        for r in owned {
            if let Some(t) = self.rules[r].running {
                self.cancel_task(t);
            }
            self.rules[r] = RuleSt::default();
        }
        if let ShotItem::Shot(sh) = &self.shots[i] {
            self.it.trace_line("SHOT-", &sh.name);
        }
    }

    fn shot_over(&mut self, i: usize) -> Res<bool> {
        let shots = Rc::clone(&self.shots);
        let ShotItem::Shot(sh) = &shots[i] else {
            return Ok(true);
        };
        Ok(match &sh.end {
            ShotEnd::For(_) => self.it.tick - self.shot_start >= self.shot_len.unwrap_or(0),
            ShotEnd::Auto => self.shot_task.is_none_or(|t| !self.tasks.contains_key(&t)),
            ShotEnd::Until(c) => {
                let now = match self.it.eval(c, &self.base_vars(), Mode::Run)? {
                    Value::Bool(b) => b,
                    other => {
                        return Err(Diagnostic::new(
                            "E-TRG-001",
                            format!(
                                "a shot's `until` must be a flag, predicate or event, found {}",
                                other.type_name()
                            ),
                        )
                        .at(c.span))
                    }
                };
                let rose = now && !self.shot_until_prev;
                self.shot_until_prev = now;
                rose
            }
        })
    }

    /// Run the shot list: end the current shot when its time is up (or its condition rises), then follow the
    /// `cut`, `dissolve` or `fade_through` after it into the next shot.
    fn update_shots(&mut self) -> Res<()> {
        if let Some(tr) = &self.it.transition {
            if self.it.tick - tr.start >= tr.dur {
                self.it.transition = None;
            }
        }
        let shots = Rc::clone(&self.shots);
        if shots.is_empty() {
            return Ok(());
        }
        for _ in 0..shots.len() + 1 {
            let cur = self.cur_shot;
            match cur {
                None if !self.shots_started => {
                    self.shots_started = true;
                    match shots.iter().position(|s| matches!(s, ShotItem::Shot(_))) {
                        Some(i) => self.begin_shot(i)?,
                        None => return Ok(()),
                    }
                }
                Some(i) if self.shot_over(i)? => {
                    self.end_shot(i);
                    let mut j = i + 1;
                    let mut transition: Option<(String, Option<String>, i64)> = None;
                    while j < shots.len() && !matches!(shots[j], ShotItem::Shot(_)) {
                        match &shots[j] {
                            ShotItem::Cut => transition = None,
                            ShotItem::Dissolve(d) => {
                                transition = Some(("dissolve".into(), None, self.shot_frames(d)?))
                            }
                            ShotItem::FadeThrough(c, d) => {
                                let col = match self.it.eval(c, &self.base_vars(), Mode::Run)? {
                                    Value::Color(c) => {
                                        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
                                    }
                                    other => {
                                        return Err(Diagnostic::new(
                                            "E-TYP-001",
                                            format!(
                                                "`fade_through` needs a colour, found {}",
                                                other.type_name()
                                            ),
                                        )
                                        .at(c.span))
                                    }
                                };
                                transition =
                                    Some(("fade_through".into(), Some(col), self.shot_frames(d)?));
                            }
                            ShotItem::Shot(_) => {}
                        }
                        j += 1;
                    }
                    if j < shots.len() {
                        if let Some((kind, color, dur)) = transition {
                            let from = crate::snapshot::camera_view(&self.it, &self.resolved);
                            self.it.transition = Some(crate::snapshot::Transition {
                                kind,
                                color,
                                start: self.it.tick,
                                dur,
                                from,
                            });
                        }
                        self.begin_shot(j)?;
                    } else {
                        self.cur_shot = None; // the last shot is over: the camera stays as it is
                        self.it.shot_name = None;
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn dispatch_rules(&mut self, sc: &Rc<Scene>) -> Res<()> {
        let vars = self.base_vars();
        let mut fired: Vec<(i64, usize)> = vec![];
        let rules = Rc::clone(&self.rules_all);
        let _ = sc;
        for (i, rule) in rules.iter().enumerate() {
            if let Some(owner) = self.rule_shot[i] {
                if self.cur_shot != Some(owner) {
                    self.rules[i] = RuleSt {
                        running: self.rules[i].running,
                        ..RuleSt::default()
                    };
                    continue; // a shot's rules only listen while it is current
                }
            }
            let now = match self.it.eval(&rule.trigger, &vars, Mode::Run)? {
                Value::Bool(b) => b,
                other => {
                    return Err(Diagnostic::new(
                        "E-TRG-001",
                        format!(
                            "a trigger must be a flag, predicate or event, found {}",
                            other.type_name()
                        ),
                    )
                    .at(rule.trigger.span))
                }
            };
            let (prev, spent, running) = (
                self.rules[i].prev,
                self.rules[i].spent,
                self.rules[i].running,
            );
            self.rules[i].prev = now;
            if !now || prev {
                continue; // only rising edges fire
            }
            if (rule.kind == RuleKind::When && spent)
                || (rule.kind == RuleKind::Whenever && running.is_some())
            {
                continue;
            }
            if let Some(g) = &rule.guard {
                match self.it.eval(g, &vars, Mode::Run)? {
                    Value::Bool(true) => {}
                    Value::Bool(false) => continue, // the occurrence is dropped, not remembered (spec §6.4)
                    other => {
                        return Err(Diagnostic::new(
                            "E-TYP-001",
                            format!("a guard must be a Bool, found {}", other.type_name()),
                        )
                        .at(g.span))
                    }
                }
            }
            if rule.kind == RuleKind::When {
                self.rules[i].spent = true;
            }
            fired.push((rule.priority, i));
        }
        self.it.events.clear();
        fired.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (prio, i) in fired {
            let id = self.new_task_id();
            let mut t = Task::new(
                id,
                Rc::clone(&rules[i].body),
                self.base_vars(),
                Mode::Run,
                true,
            );
            t.priority = prio;
            t.rule = Some(i);
            self.rules[i].running = Some(id);
            let label = self.it.span_text(rules[i].trigger.span);
            let kind = if rules[i].kind == RuleKind::When {
                "when"
            } else {
                "whenever"
            };
            self.it
                .trace_line("FIRE", &format!("R{} {kind} {label} -> T{id}", i + 1));
            self.tasks.insert(id, t);
        }
        Ok(())
    }

    fn step_tasks(&mut self) -> Res<()> {
        let tick = self.it.tick;
        loop {
            self.pump_named(None)?;
            let Some(id) = self
                .tasks
                .iter()
                .find(|(_, t)| t.state == TaskState::Ready)
                .map(|(i, _)| *i)
            else {
                break;
            };
            let mut t = self.tasks.remove(&id).unwrap();
            let stepped = self.it.step(&mut t)?;
            self.take_keysets();
            match stepped {
                Step::Blocked(Req::Acquire(req)) => {
                    match self.try_acquire(id, &req, Some(&t))? {
                        true => t.state = TaskState::Ready,
                        false => {
                            t.state = TaskState::BlockedAct;
                            self.named_wait.push(NamedWait {
                                task: id,
                                req,
                                seq: self.seq,
                            });
                            self.seq += 1;
                        }
                    }
                    self.tasks.insert(id, t);
                }
                Step::Blocked(Req::Act(req)) if Interp::undeclared_channel(&t, &req.shape) => {
                    return Err(Diagnostic::new(
                        "E-CAP-003",
                        "an action touched the `body` channel but its `uses` line does not declare it",
                    )
                    .help("add `body` to the action's `uses` line (every shape motion and colour change uses `body`)"));
                }
                Step::Done => self.finish_task(t),
                Step::Blocked(Req::Wait(n)) => {
                    if n == 0 {
                        t.state = TaskState::Ready;
                    } else {
                        t.state = TaskState::Waiting;
                        t.wake = tick + n;
                    }
                    self.tasks.insert(id, t);
                }
                Step::Blocked(Req::Act(req)) => {
                    t.state = TaskState::BlockedAct;
                    let p = PendingAct {
                        req,
                        task: id,
                        prio: t.priority,
                        seq: self.seq,
                        handle: 0,
                        waiters: vec![],
                    };
                    self.seq += 1;
                    self.tasks.insert(id, t);
                    if self.key_free(&p.req) {
                        self.start_action(p);
                    } else {
                        self.pending.push(p); // queue policy: waits for every channel at once
                    }
                }
                Step::Blocked(Req::Await(h)) => {
                    let tick = self.it.tick;
                    match self.handles.get(&h).copied() {
                        Some(HandleState::Act(aid)) if self.acts.contains_key(&aid) => {
                            self.acts.get_mut(&aid).unwrap().waiters.push(id);
                            t.state = TaskState::BlockedAct;
                        }
                        Some(HandleState::Queued) => {
                            if let Some(p) = self.pending.iter_mut().find(|p| p.handle == h) {
                                p.waiters.push(id);
                            }
                            t.state = TaskState::BlockedAct;
                        }
                        Some(HandleState::Mod(mid)) if self.modsets.iter().any(|m| m.id == mid) => {
                            let to = self.modsets.iter().find(|m| m.id == mid).and_then(|m| m.to);
                            let Some(to) = to else {
                                return Err(Diagnostic::new("E-RUN-006", "`await` on a modifier that never ends")
                                    .help("give the modifier `over` a duration, or `cancel` it instead of awaiting it"));
                            };
                            t.state = TaskState::Waiting;
                            t.wake = to;
                            let _ = tick;
                        }
                        Some(HandleState::Task(tid)) if self.tasks.contains_key(&tid) => {
                            self.tasks.get_mut(&tid).expect("checked").awaiters.push(id);
                            t.state = TaskState::BlockedAct;
                        }
                        _ => t.state = TaskState::Ready, // already finished or cancelled
                    }
                    self.tasks.insert(id, t);
                }
                Step::Blocked(req @ (Req::Together(_) | Req::Race(_))) => {
                    let (blocks, race) = match req {
                        Req::Together(b) => (b, false),
                        Req::Race(b) => (b, true),
                        _ => unreachable!(),
                    };
                    t.race = race;
                    t.pending = blocks.len() as u32;
                    t.state = if blocks.is_empty() {
                        TaskState::Ready
                    } else {
                        TaskState::BlockedKids
                    };
                    let (vars, prio) = (t.vars.flatten(), t.priority);
                    self.tasks.insert(id, t);
                    for b in blocks {
                        let cid = self.new_task_id();
                        let mut c = Task::new(cid, b, vars.clone(), Mode::Run, true);
                        c.parent = Some(id);
                        c.priority = prio;
                        self.tasks.insert(cid, c);
                    }
                }
            }
        }
        match self.deferred_err.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Is the channel holder's call still running (its task alive, with that call on its frame stack)?
    fn occ_alive(&self, o: &Occ, cur: Option<&Task>) -> bool {
        let has = |t: &Task| {
            t.frames
                .iter()
                .any(|f| matches!(f, Frame::Call { id, .. } if *id == o.call))
        };
        match cur {
            Some(c) if c.id == o.task => has(c),
            _ => self.tasks.get(&o.task).is_some_and(has),
        }
    }

    /// Try to take every channel of `req` at once for `task`. `Ok(false)`: it has to wait.
    fn try_acquire(&mut self, task: u32, req: &AcquireReq, cur: Option<&Task>) -> Res<bool> {
        let alive: Vec<bool> = self.named.iter().map(|o| self.occ_alive(o, cur)).collect();
        let mut i = 0;
        self.named.retain(|_| {
            i += 1;
            alive[i - 1]
        });
        let mut victims: Vec<u32> = vec![];
        let mut wait = false;
        for (ch, policy) in &req.channels {
            let occ = self
                .named
                .iter()
                .find(|o| o.shape == req.shape && o.ch == *ch && o.task != task);
            let Some(occ) = occ else { continue };
            match policy {
                Policy::Share => {}
                Policy::Queue => wait = true,
                Policy::Reject => {
                    return Err(Diagnostic::new(
                        "E-RUN-002",
                        format!(
                        "channel `{ch}` of `{}` is busy and the action rejects instead of waiting",
                        req.shape
                    ),
                    )
                    .help("use `on_conflict queue` to wait, or `interrupt` to take over"))
                }
                Policy::Interrupt => {
                    if req.prio >= occ.prio {
                        victims.push(occ.task);
                    } else {
                        wait = true;
                    }
                }
            }
        }
        if wait {
            return Ok(false);
        }
        victims.sort();
        victims.dedup();
        for v in victims {
            self.it
                .trace_line("ACT!", &format!("T{v} interrupted by T{task}"));
            self.cancel_task(v);
        }
        for (ch, policy) in &req.channels {
            if *policy != Policy::Share {
                self.named.push(Occ {
                    shape: req.shape.clone(),
                    ch: ch.clone(),
                    task,
                    call: req.call,
                    prio: req.prio,
                });
            }
        }
        Ok(true)
    }

    /// Let waiting actions have their channels when they are free: highest priority first, then in order.
    fn pump_named(&mut self, cur: Option<&Task>) -> Res<()> {
        if self.named_wait.is_empty() {
            return Ok(());
        }
        loop {
            self.named_wait.retain(|w| self.tasks.contains_key(&w.task));
            let mut order: Vec<usize> = (0..self.named_wait.len()).collect();
            order.sort_by(|a, b| {
                let (x, y) = (&self.named_wait[*a], &self.named_wait[*b]);
                y.req.prio.cmp(&x.req.prio).then(x.seq.cmp(&y.seq))
            });
            let mut progressed = false;
            for i in order {
                let (task, req) = (self.named_wait[i].task, self.named_wait[i].req.clone());
                if self.try_acquire(task, &req, cur)? {
                    self.named_wait.remove(i);
                    if let Some(t) = self.tasks.get_mut(&task) {
                        t.state = TaskState::Ready;
                    }
                    progressed = true;
                    break;
                }
            }
            if !progressed {
                return Ok(());
            }
        }
    }

    fn finish_task(&mut self, t: Task) {
        self.it.trace_line("TASK-", &format!("T{}", t.id));
        if let Some(r) = t.rule {
            self.rules[r].running = None;
        }
        if Some(t.id) == self.main_task {
            self.main_done = true;
        }
        for w in &t.awaiters {
            if let Some(x) = self.tasks.get_mut(w) {
                x.state = TaskState::Ready;
            }
        }
        if let Some(p) = t.parent {
            let mut lost = vec![];
            if let Some(parent) = self.tasks.get_mut(&p) {
                parent.pending = parent.pending.saturating_sub(1);
                if parent.race && parent.state == TaskState::BlockedKids {
                    // the first child home wins: the others are cancelled
                    parent.pending = 0;
                    parent.race = false;
                    parent.state = TaskState::Ready;
                    lost = self
                        .tasks
                        .iter()
                        .filter(|(_, k)| k.parent == Some(p))
                        .map(|(k, _)| *k)
                        .collect();
                } else if parent.pending == 0 {
                    parent.state = TaskState::Ready;
                }
            }
            for l in lost {
                self.cancel_task(l);
            }
        }
    }

    // ---- phases 6-7 --------------------------------------------------------------------

    /// Sample every running action, modifier and key set at this tick and resolve every touched scalar
    /// through the layer fold (spec 7.3): the rest value, then the contributions in
    /// `(layer, priority, start, id)` order.
    fn advance_actions(&mut self) {
        let tick = self.it.tick;
        let fps = self.it.fps.to_f64();
        let mut by_prop: BTreeMap<(String, PropId), Vec<Contribution>> = BTreeMap::new();
        for a in self.acts.values_mut() {
            let p = (tick - a.start) as f64 / (a.end - a.start) as f64;
            let e = a.ease.apply(p);
            let secs = (tick - a.start) as f64 / fps;
            for c in &a.comps {
                let value = if c.motion == Motion::Eased {
                    c.from + (c.to - c.from) * e
                } else {
                    motion_value(&c.motion, c.from, secs)
                };
                a.last.insert(c.prop, value);
                by_prop
                    .entry((a.shape.clone(), c.prop))
                    .or_default()
                    .push(Contribution {
                        layer: 1,
                        mode: BlendMode::Override,
                        weight: 1.0,
                        priority: a.prio,
                        start: a.start,
                        id: a.id,
                        value,
                    });
            }
        }
        for (k, l) in self.locks.iter().enumerate() {
            if tick < l.from || l.to.is_some_and(|t| tick >= t) {
                continue;
            }
            by_prop
                .entry((l.shape.clone(), l.prop))
                .or_default()
                .push(Contribution {
                    layer: 4,
                    mode: BlendMode::Override,
                    weight: 1.0,
                    priority: 0,
                    start: l.from,
                    id: 0x7800_0000 + k as u32,
                    value: l.value,
                });
        }
        for m in &self.modsets {
            if tick < m.from || m.to.is_some_and(|t| tick >= t) {
                continue;
            }
            let secs = (tick - m.from) as f64 / fps;
            let mut w = 1.0f64;
            if m.blend_in > 0 {
                w = w.min((tick - m.from) as f64 / m.blend_in as f64);
            }
            if let (true, Some(to)) = (m.blend_out > 0, m.to) {
                w = w.min((to - tick) as f64 / m.blend_out as f64);
            }
            for c in &m.comps {
                let value = match m.kind {
                    ModKind::Wobble => {
                        c.amount * (std::f64::consts::TAU * (c.speed * secs + c.phase)).sin()
                    }
                    ModKind::Noise => c.amount * value_noise(c.seed, c.speed * secs),
                };
                by_prop
                    .entry((m.shape.clone(), c.prop))
                    .or_default()
                    .push(Contribution {
                        layer: 2,
                        mode: BlendMode::Add,
                        weight: w.clamp(0.0, 1.0),
                        priority: 0,
                        start: m.from,
                        id: m.id,
                        value,
                    });
            }
        }
        for k in &self.keysets {
            if tick < k.from || k.to.is_some_and(|t| tick > t) {
                continue;
            }
            let mut w = 1.0f64;
            if k.blend_in > 0 {
                w = w.min((tick - k.from) as f64 / k.blend_in as f64);
            }
            if let (true, Some(to)) = (k.blend_out > 0, k.to) {
                w = w.min((to - tick) as f64 / k.blend_out as f64);
            }
            by_prop
                .entry((k.shape.clone(), k.prop))
                .or_default()
                .push(Contribution {
                    layer: 3,
                    mode: if k.add {
                        BlendMode::Add
                    } else {
                        BlendMode::Override
                    },
                    weight: w.clamp(0.0, 1.0),
                    priority: 0,
                    start: k.from,
                    id: k.id,
                    value: key_value(&k.keys, tick),
                });
        }
        // the animated pose of every armature (FK, limits, IK) comes first: ragdoll bodies follow it
        // through the physics step, and the final pose (physics may replace it) is written afterwards
        let mut anim_poses = BTreeMap::new();
        if self.it.has_rigs() {
            let (bone_keys, ik_keys) = self.it.rig_keys();
            let mut bones = BTreeMap::new();
            for key in bone_keys {
                let mut get = |p: PropId| {
                    let rest = self.rest_value(&key, p);
                    by_prop
                        .get_mut(&(key.clone(), p))
                        .map_or(rest, |c| fold(rest, c))
                };
                let v = [get(PropId::Rot), get(PropId::PosX), get(PropId::PosY)];
                bones.insert(key, v);
            }
            let mut iks = BTreeMap::new();
            for key in ik_keys {
                let mut get = |p: PropId| {
                    let rest = self.rest_value(&key, p);
                    by_prop
                        .get_mut(&(key.clone(), p))
                        .map_or(rest, |c| fold(rest, c))
                };
                let v = [get(PropId::PosX), get(PropId::PosY), get(PropId::Opacity)];
                iks.insert(key, v);
            }
            anim_poses = self.it.rig_solve(&bones, &iks);
        }
        let mut physics = None;
        let mut soft_snap = None;
        let mut fluid_snap = None;
        let (has_phys, has_soft, has_fluid) = (
            self.it.phys.is_some(),
            self.it.soft.is_some(),
            self.it.has_fluid(),
        );
        if has_phys || has_soft || has_fluid {
            // kinematic bodies follow the pose the other layers resolved; dynamic bodies write theirs
            let mut kin = BTreeMap::new();
            for n in self.it.kinematic_bodies() {
                let mut get = |p: PropId| {
                    let rest = self.rest_value(&n, p);
                    by_prop
                        .get_mut(&(n.clone(), p))
                        .map_or(rest, |c| fold(rest, c))
                };
                let pose = [get(PropId::PosX), get(PropId::PosY), get(PropId::Rot)];
                kin.insert(n.clone(), pose);
            }
            self.it.physics_prepare(tick, &kin);
            self.it.ragdoll_prepare(tick, 1.0 / fps, &anim_poses);
            // soft pins follow the pose of the shape they are pinned to
            let mut targets = BTreeMap::new();
            for n in self.it.soft_pin_targets() {
                if let Some(t) = self.it.dynamic_pose(&n, 1.0 / fps) {
                    targets.insert(n, t);
                    continue;
                }
                let mut get = |p: PropId| {
                    let rest = self.rest_value(&n, p);
                    by_prop
                        .get_mut(&(n.clone(), p))
                        .map_or(rest, |c| fold(rest, c))
                };
                let t = Transform {
                    position: [get(PropId::PosX), get(PropId::PosY)],
                    scale: [get(PropId::ScaleX), get(PropId::ScaleY)],
                    rotation: get(PropId::Rot),
                    pivot: self.it.shapes[&n].transform().pivot,
                };
                targets.insert(n, t);
            }
            let mut own = BTreeMap::new();
            for n in self.it.soft_shapes() {
                let mut get = |p: PropId| {
                    let rest = self.rest_value(&n, p);
                    by_prop
                        .get_mut(&(n.clone(), p))
                        .map_or(rest, |c| fold(rest, c))
                };
                let t = Transform {
                    position: [get(PropId::PosX), get(PropId::PosY)],
                    scale: [get(PropId::ScaleX), get(PropId::ScaleY)],
                    rotation: get(PropId::Rot),
                    pivot: self.it.shapes[&n].transform().pivot,
                };
                own.insert(n, t);
            }
            if has_soft {
                self.it.soft_prepare(tick, &targets, &own);
            }
            if has_soft || has_fluid {
                self.it.step_coupled(tick);
            } else {
                self.it.physics_advance(tick);
            }
            let soft_out = self.it.soft_collect(tick);
            let poses = self.it.physics_collect();
            for (i, (n, x, y, r)) in poses.dynamic.into_iter().enumerate() {
                for (prop, value) in [(PropId::PosX, x), (PropId::PosY, y), (PropId::Rot, r)] {
                    by_prop
                        .entry((n.clone(), prop))
                        .or_default()
                        .push(Contribution {
                            layer: 2,
                            mode: BlendMode::Override,
                            weight: 1.0,
                            priority: 0,
                            start: 0,
                            id: 0x4000_0000 + i as u32,
                            value,
                        });
                }
            }
            for (k, (n, vid, x, y)) in soft_out.into_iter().enumerate() {
                for (prop, value) in [(PropId::Vx(vid), x), (PropId::Vy(vid), y)] {
                    by_prop
                        .entry((n.clone(), prop))
                        .or_default()
                        .push(Contribution {
                            layer: 2,
                            mode: BlendMode::Override,
                            weight: 1.0,
                            priority: 0,
                            start: 0,
                            id: 0x6000_0000 + k as u32,
                            value,
                        });
                }
            }
            physics = self.it.physics_snapshot();
            soft_snap = self.it.soft_snapshot();
            fluid_snap = self.it.fluid_snapshot();
        }
        let mut rigs_snap = None;
        if self.it.has_rigs() {
            self.it.ragdoll_poses(&mut anim_poses);
            let out = self.it.rig_finish(&anim_poses);
            for (k, (shape, prop, value)) in out.overrides.into_iter().enumerate() {
                by_prop
                    .entry((shape, prop))
                    .or_default()
                    .push(Contribution {
                        layer: 2,
                        mode: BlendMode::Override,
                        weight: 1.0,
                        priority: 0,
                        start: 0,
                        id: 0x7000_0000 + k as u32,
                        value,
                    });
            }
            rigs_snap = Some(out.snapshot);
        }
        let mut resolved = Resolved {
            physics,
            rigs: rigs_snap,
            soft: soft_snap,
            fluids: fluid_snap,
            ..Resolved::default()
        };
        for ((shape, prop), mut contribs) in by_prop {
            let v = fold(self.rest_value(&shape, prop), &mut contribs);
            let base = if prop.is_colour() {
                self.colour_rest(&shape, prop).unwrap_or([0.0; 4])
            } else {
                [0.0; 4]
            };
            let comp = Self::colour_comp(prop);
            let sh = &self.it.shapes[&shape];
            match prop {
                PropId::Vx(i) | PropId::Vy(i) => {
                    let p = resolved
                        .verts
                        .entry(shape.clone())
                        .or_default()
                        .entry(i)
                        .or_insert_with(|| sh.pos(i));
                    if matches!(prop, PropId::Vx(_)) {
                        p[0] = v;
                    } else {
                        p[1] = v;
                    }
                }
                PropId::PosX | PropId::PosY | PropId::ScaleX | PropId::ScaleY | PropId::Rot => {
                    let t = resolved
                        .tf
                        .entry(shape.clone())
                        .or_insert_with(|| sh.transform());
                    match prop {
                        PropId::PosX => t.position[0] = v,
                        PropId::PosY => t.position[1] = v,
                        PropId::ScaleX => t.scale[0] = v,
                        PropId::ScaleY => t.scale[1] = v,
                        _ => t.rotation = v,
                    }
                }
                PropId::FillC(f, _) => {
                    resolved.fills.entry((shape.clone(), f)).or_insert(base)[comp] = v
                }
                PropId::StrokeC(e, _) => {
                    resolved.strokes.entry((shape.clone(), e)).or_insert(base)[comp] = v
                }
                PropId::ShapeFillC(_) => {
                    resolved.shape_fill.entry(shape.clone()).or_insert(base)[comp] = v
                }
                PropId::ShapeStrokeC(_) => {
                    resolved.shape_stroke.entry(shape.clone()).or_insert(base)[comp] = v
                }
                PropId::TintC(_) => resolved.tint.entry(shape.clone()).or_insert(base)[comp] = v,
                PropId::EdgeWidth(e) => {
                    resolved.widths.insert((shape.clone(), e), v.max(0.0));
                }
                PropId::ShapeWidth => {
                    resolved.shape_width.insert(shape.clone(), v.max(0.0));
                }
                PropId::Opacity => {
                    resolved.opacity.insert(shape.clone(), v.clamp(0.0, 1.0));
                }
            }
        }
        self.apply_links(&mut resolved);
        resolved.particles = self.it.particles_tick(tick, &resolved.tf);
        self.it.live_tf = resolved.tf.clone();
        self.resolved = resolved;
    }

    fn end_scene(&mut self) -> Res<()> {
        self.it.shot_name = None;
        self.it.transition = None;
        // tasks still running when the scene ends are cancelled: they run their `finally:` blocks
        let left: Vec<u32> = self.tasks.keys().copied().collect();
        for id in left {
            if self.tasks.contains_key(&id) {
                self.cancel_task(id);
            }
        }
        if let Some(e) = self.deferred_err.take() {
            return Err(e);
        }
        self.it.apply_staged()?; // what the `finally:` blocks changed
        self.it.live_tf.clear();
        self.it.rig_live.clear();
        self.it.ik_live.clear();
        self.it.end_soft();
        self.it.end_fluids();
        self.it.end_physics();
        self.it.parts.clear();
        let acts = std::mem::take(&mut self.acts);
        for a in acts.values() {
            self.commit(a, true); // cancelled actions keep their last sampled value
        }
        self.tasks.clear();
        self.pending.clear();
        self.channels.clear();
        self.it.staged.clear();
        self.it.events.clear();
        self.keysets.clear();
        self.modsets.clear();
        self.handles.clear();
        self.resolved = Resolved::default();
        Ok(())
    }
}

/// The value of a key set at tick `t`: linear between keys with the *later* key's easing (spec 7.7).
fn key_value(keys: &[(i64, f64, Ease)], t: i64) -> f64 {
    if t <= keys[0].0 {
        return keys[0].1;
    }
    for w in keys.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t <= b.0 {
            let p = (t - a.0) as f64 / (b.0 - a.0) as f64;
            return a.1 + (b.1 - a.1) * b.2.apply(p);
        }
    }
    keys[keys.len() - 1].1
}
