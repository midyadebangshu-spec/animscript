//! The tick loop (spec §6.2, reduced to what the shape system needs): ingest, delta cycles with
//! rule dispatch and task stepping, action sampling, property resolution and snapshots.
use crate::ast::*;
use crate::interp::*;
use crate::parser::parse;
use crate::snapshot::{build_snapshot, snapshot_hash};
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

struct ActItem {
    vid: u32,
    from: P,
    to: P,
}
struct Act {
    id: u32,
    task: u32,
    shape: String,
    items: Vec<ActItem>,
    start: i64,
    end: i64,
    ease: Ease,
    last: BTreeMap<u32, P>,
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
    resolved: BTreeMap<String, BTreeMap<u32, P>>,
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
            resolved: BTreeMap::new(),
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
        req.items
            .iter()
            .all(|(vid, _)| !self.channels.contains_key(&(req.shape.clone(), *vid)))
    }

    // ---- phase 2 -----------------------------------------------------------------------

    fn ingest(&mut self) {
        let tick = self.it.tick;
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
                .trace_line("ACT-", &format!("{} {}", a.shape, a.items.len()));
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

    /// Commit an action's values to the rest layer (spec §7.6): the final value when it finished,
    /// the last sampled value when it was cut off.
    fn commit(&mut self, a: &Act, interrupted: bool) {
        for item in &a.items {
            let p = if interrupted {
                a.last.get(&item.vid).copied().unwrap_or(item.from)
            } else {
                item.to
            };
            if let Some(v) = self
                .it
                .shapes
                .get_mut(&a.shape)
                .and_then(|s| s.verts.get_mut(&item.vid))
            {
                v.x = p[0];
                v.y = p[1];
            }
            self.channels.remove(&(a.shape.clone(), item.vid));
        }
    }

    fn start_action(&mut self, p: PendingAct) {
        let tick = self.it.tick;
        let sh = &self.it.shapes[&p.req.shape];
        let items: Vec<ActItem> = p
            .req
            .items
            .iter()
            .map(|(vid, t)| {
                let from = sh.pos(*vid);
                let to = match t {
                    Target::To(q) => *q,
                    Target::By(d) => [from[0] + d[0], from[1] + d[1]],
                };
                ActItem {
                    vid: *vid,
                    from,
                    to,
                }
            })
            .collect();
        let id = self.next_act;
        self.next_act += 1;
        for it in &items {
            self.channels.insert((p.req.shape.clone(), it.vid), id);
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
                items,
                start: tick,
                end,
                ease: p.req.ease,
                last: BTreeMap::new(),
            },
        );
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
            match self.it.step(&mut t)? {
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

    /// Sample every running action at this tick and resolve each vertex component: the rest value,
    /// overridden by the action contributions in a fixed order (layer, start tick, id).
    fn advance_actions(&mut self) {
        let tick = self.it.tick;
        self.resolved.clear();
        for a in self.acts.values_mut() {
            let p = (tick - a.start) as f64 / (a.end - a.start) as f64;
            let e = a.ease.apply(p);
            for item in &a.items {
                let q = [
                    item.from[0] + (item.to[0] - item.from[0]) * e,
                    item.from[1] + (item.to[1] - item.from[1]) * e,
                ];
                a.last.insert(item.vid, q);
                self.resolved
                    .entry(a.shape.clone())
                    .or_default()
                    .insert(item.vid, q);
            }
        }
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
        self.resolved.clear();
    }
}
