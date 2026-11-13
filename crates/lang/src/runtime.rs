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
#[derive(Clone, Copy, Debug)]
enum HandleState {
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
        self.it.scene_name = sc.name.clone();
        self.it.decls.start_scene();
        self.it.reset_particles();
        self.it.reset_fluids();
        self.it.mod_counts.clear();
        self.pending.clear();
        self.channels.clear();
        self.rules = vec![RuleSt::default(); sc.rules.len()];
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
                    t.frames.insert(0, Frame::Call { saved: None });
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

    fn dispatch_rules(&mut self, sc: &Rc<Scene>) -> Res<()> {
        let vars = self.base_vars();
        let mut fired: Vec<(i64, usize)> = vec![];
        for (i, rule) in sc.rules.iter().enumerate() {
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
                Rc::clone(&sc.rules[i].body),
                self.base_vars(),
                Mode::Run,
                true,
            );
            t.priority = prio;
            t.rule = Some(i);
            self.rules[i].running = Some(id);
            let label = self.it.span_text(sc.rules[i].trigger.span);
            let kind = if sc.rules[i].kind == RuleKind::When {
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
        while let Some(id) = self
            .tasks
            .iter()
            .find(|(_, t)| t.state == TaskState::Ready)
            .map(|(i, _)| *i)
        {
            let mut t = self.tasks.remove(&id).unwrap();
            let stepped = self.it.step(&mut t)?;
            self.take_keysets();
            match stepped {
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
        resolved.particles = self.it.particles_tick(tick, &resolved.tf);
        self.it.live_tf = resolved.tf.clone();
        self.resolved = resolved;
    }

    fn end_scene(&mut self) -> Res<()> {
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
