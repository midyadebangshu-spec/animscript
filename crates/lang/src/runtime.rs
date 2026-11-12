//! The tick loop (spec §6.2, reduced to what the shape system needs): ingest, delta cycles with
//! rule dispatch and task stepping, action sampling, property resolution and snapshots.
use crate::ast::*;
use crate::interp::*;
use crate::layers::{fold, BlendMode, Contribution};
use crate::parser::parse;
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
struct PendingAct {
    req: ActionReq,
    task: u32,
    prio: i64,
    seq: u64,
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
    main_task: Option<u32>,
    main_done: bool,
    project_tick: i64,
    scene_name: String,
    snapshots: Vec<J>,
    hashes: Vec<u64>,
}

/// Parse, build and run a program, returning one snapshot (and hash) per tick.
pub fn run(src: &str, opts: &RunOptions) -> Res<RunResult> {
    let program = parse(src)?;
    if program.project.is_none() {
        return Err(
            Diagnostic::new("E-PRJ-001", "a runnable program needs a `project` block").help(
                "start the file with `project \"name\":` giving `anim_version`, `canvas` and `fps`",
            ),
        );
    }
    let mut it = Interp::new(src, program)?;
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
            main_task: None,
            main_done: false,
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
        self.pending.clear();
        self.channels.clear();
        self.rules = vec![RuleSt::default(); sc.rules.len()];
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
            let mut d = 1u32;
            loop {
                self.it.delta = d;
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
        self.end_scene();
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
        }
    }

    fn set_rest(&mut self, shape: &str, prop: PropId, v: f64) {
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
            other => set_transform_prop(sh, other, v, false),
        }
    }

    // ---- phase 2 -----------------------------------------------------------------------

    fn ingest(&mut self) {
        let tick = self.it.tick;
        // key sets past their last tick simply stop: the layer below shows through again
        self.keysets.retain(|k| k.to.is_none_or(|t| tick <= t));
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
            if let Some(t) = self.tasks.get_mut(&a.task) {
                t.state = TaskState::Ready;
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

    fn start_action(&mut self, p: PendingAct) {
        let tick = self.it.tick;
        let comps: Vec<ActComp> = p
            .req
            .comps
            .iter()
            .map(|(prop, t)| {
                let from = self.rest_value(&p.req.shape, *prop);
                let to = match t {
                    Tgt::To(v) => *v,
                    Tgt::By(d) => from + d,
                };
                ActComp {
                    prop: *prop,
                    from,
                    to,
                }
            })
            .collect();
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
            },
        );
    }

    /// Pick up key sets registered by `keyframes` statements during the last step.
    fn take_keysets(&mut self) {
        let specs = std::mem::take(&mut self.it.pending_keysets);
        let base = self.it.tick;
        for s in specs {
            self.add_keyset(s, base);
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
                    };
                    self.seq += 1;
                    self.tasks.insert(id, t);
                    if self.key_free(&p.req) {
                        self.start_action(p);
                    } else {
                        self.pending.push(p); // queue policy: waits for every channel at once
                    }
                }
                Step::Blocked(Req::Together(blocks)) => {
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
        Ok(())
    }

    fn finish_task(&mut self, t: Task) {
        self.it.trace_line("TASK-", &format!("T{}", t.id));
        if let Some(r) = t.rule {
            self.rules[r].running = None;
        }
        if Some(t.id) == self.main_task {
            self.main_done = true;
        }
        if let Some(p) = t.parent {
            if let Some(parent) = self.tasks.get_mut(&p) {
                parent.pending = parent.pending.saturating_sub(1);
                if parent.pending == 0 {
                    parent.state = TaskState::Ready;
                }
            }
        }
    }

    // ---- phases 6-7 --------------------------------------------------------------------

    /// Sample every running action and key set at this tick and resolve every touched scalar through the
    /// layer fold (spec 7.3): the rest value, then the contributions in `(layer, priority, start, id)` order.
    fn advance_actions(&mut self) {
        let tick = self.it.tick;
        let mut by_prop: BTreeMap<(String, PropId), Vec<Contribution>> = BTreeMap::new();
        for a in self.acts.values_mut() {
            let p = (tick - a.start) as f64 / (a.end - a.start) as f64;
            let e = a.ease.apply(p);
            for c in &a.comps {
                let value = c.from + (c.to - c.from) * e;
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
        let mut resolved = Resolved::default();
        for ((shape, prop), mut contribs) in by_prop {
            let v = fold(self.rest_value(&shape, prop), &mut contribs);
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
                other => {
                    let t = resolved
                        .tf
                        .entry(shape.clone())
                        .or_insert_with(|| sh.transform());
                    match other {
                        PropId::PosX => t.position[0] = v,
                        PropId::PosY => t.position[1] = v,
                        PropId::ScaleX => t.scale[0] = v,
                        PropId::ScaleY => t.scale[1] = v,
                        _ => t.rotation = v,
                    }
                }
            }
        }
        self.resolved = resolved;
    }

    fn end_scene(&mut self) {
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
        self.resolved = Resolved::default();
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
