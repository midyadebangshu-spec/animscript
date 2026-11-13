//! Expression evaluation, statement execution (as resumable tasks) and the shape methods.
use crate::ast::*;
use crate::color::to_linear;
use crate::procedural::{hash01, stream_seed, value_noise};
use crate::value::*;
use animscript_core::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Top level and `shape` build blocks.
    Build,
    /// A scene's `setup:` block.
    Setup,
    /// `main:`, rules, and everything started from them.
    Run,
}

#[derive(Clone, Default, Debug)]
pub struct Vars {
    pub scopes: Vec<HashMap<String, Value>>,
}
impl Vars {
    pub fn new() -> Vars {
        Vars {
            scopes: vec![HashMap::new()],
        }
    }
    pub fn get(&self, n: &str) -> Option<&Value> {
        self.scopes.iter().rev().find_map(|s| s.get(n))
    }
    pub fn define(&mut self, n: &str, v: Value) {
        self.scopes.last_mut().unwrap().insert(n.to_string(), v);
    }
    pub fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }
    /// Change the innermost existing binding of `n`; false when there is none.
    pub fn assign(&mut self, n: &str, v: Value) -> bool {
        for s in self.scopes.iter_mut().rev() {
            if let Some(slot) = s.get_mut(n) {
                *slot = v;
                return true;
            }
        }
        false
    }
    pub fn pop(&mut self) {
        self.scopes.pop();
    }
    pub fn flatten(&self) -> Vars {
        let mut m = HashMap::new();
        for s in &self.scopes {
            for (k, v) in s {
                m.insert(k.clone(), v.clone());
            }
        }
        Vars { scopes: vec![m] }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Entity {
    pub states: Vec<String>,
    pub flags: BTreeMap<String, bool>,
    /// Names of derived flags (their values live in `flags` too).
    pub derived: Vec<String>,
}

/// A zero-time effect that is applied at the end of the delta (spec §6.3).
#[derive(Clone, Debug)]
pub enum Staged {
    Flag {
        entity: String,
        flag: String,
        value: bool,
    },
    Move {
        shape: String,
        r: ElemRef,
        x: f64,
        y: f64,
        by: bool,
    },
    /// A zero-time change of one component of a shape's transform.
    Transform {
        shape: String,
        prop: PropId,
        value: f64,
        by: bool,
    },
    /// A fill / stroke / stroke width of one face or edge.
    ElemStyle {
        shape: String,
        r: ElemRef,
        prop: String,
        value: Value,
    },
    Style {
        shape: String,
        prop: String,
        value: Value,
    },
}

#[derive(Clone, Debug)]
/// A lock to register: layer-4 values for some scalars, for a number of frames (or until released).
pub struct LockSpec {
    pub comps: Vec<(String, PropId, f64)>,
    pub frames: Option<i64>,
    pub span: Span,
}
/// An attachment or constraint to register.
pub enum LinkSpec {
    Attach {
        child: String,
        parent: String,
        offset: Option<[f64; 2]>,
        span: Span,
    },
    Inside {
        shape: String,
        rect: [f64; 4],
        weight: f64,
    },
    Follow {
        shape: String,
        target: String,
        lag: i64,
        weight: f64,
    },
}
/// An action asks for the channels it `uses` on one shape.
#[derive(Clone)]
pub struct AcquireReq {
    pub shape: String,
    pub channels: Vec<(String, Policy)>,
    pub prio: i64,
    pub call: u64,
}

pub enum Req {
    Wait(i64),
    Acquire(AcquireReq),
    /// Suspend until a started action or modifier ends.
    Await(u32),
    Act(ActionReq),
    Together(Vec<Block>),
    /// Like `Together`, but the first child to finish cancels the others and wakes the parent.
    Race(Vec<Block>),
}
pub enum Step {
    Done,
    Blocked(Req),
}

pub enum Frame {
    Block {
        stmts: Block,
        idx: usize,
        scope: bool,
    },
    Repeat {
        body: Block,
        left: i64,
    },
    While {
        cond: Rc<Expr>,
        body: Block,
    },
    /// Marks the start of a user action's body; popping it restores the caller's variables.
    Call {
        saved: Option<Vars>,
        /// Identifies this call for the channels it holds (0: none).
        id: u64,
        /// The channel names its `uses` line declares (empty: not declared).
        uses: Rc<Vec<String>>,
    },
    For {
        var: String,
        items: Vec<Value>,
        idx: usize,
        body: Block,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    Ready,
    Waiting,
    BlockedAct,
    BlockedKids,
}

pub struct Task {
    pub id: u32,
    pub frames: Vec<Frame>,
    pub vars: Vars,
    pub mode: Mode,
    pub priority: i64,
    pub parent: Option<u32>,
    pub pending: u32,
    pub state: TaskState,
    pub wake: i64,
    pub rule: Option<usize>,
    /// Tasks waiting for this one to finish (`await` on a started user action).
    pub awaiters: Vec<u32>,
    /// Waiting in a `race:`: the first child to finish ends the wait.
    pub race: bool,
}
impl Task {
    pub fn new(id: u32, block: Block, mut vars: Vars, mode: Mode, scope: bool) -> Task {
        if scope {
            vars.push();
        }
        Task {
            id,
            frames: vec![Frame::Block {
                stmts: block,
                idx: 0,
                scope,
            }],
            vars,
            mode,
            priority: 0,
            parent: None,
            pending: 0,
            state: TaskState::Ready,
            wake: 0,
            rule: None,
            awaiters: vec![],
            race: false,
        }
    }
    pub(crate) fn push_block(&mut self, b: Block, bind: Vec<(String, Value)>) {
        self.vars.push();
        for (k, v) in bind {
            self.vars.define(&k, v);
        }
        self.frames.push(Frame::Block {
            stmts: b,
            idx: 0,
            scope: true,
        });
    }
}

pub struct Interp {
    pub src: String,
    pub program: Program,
    pub shapes: BTreeMap<String, Shape>,
    pub shape_order: Vec<String>,
    pub entities: BTreeMap<String, Entity>,
    pub entity_order: Vec<String>,
    pub globals: HashMap<String, Value>,
    /// Warnings collected while evaluating.
    pub diagnostics: Vec<Diagnostic>,
    pub fps: Rat,
    pub canvas: (u32, u32),
    pub background: Option<[u8; 4]>,
    pub tick: i64,
    pub delta: u32,
    pub events: BTreeSet<String>,
    pub staged: Vec<Staged>,
    pub trace_on: bool,
    pub trace: Vec<String>,
    /// Key sets registered by `keyframes` statements, waiting for the runtime to pick them up.
    pub pending_keysets: Vec<KeySetSpec>,
    /// `(handle, request)` pairs waiting for the runtime; handle 0 means "no handle" (a blocking modifier).
    pub pending_starts: Vec<(u32, StartReq)>,
    pub pending_cancels: Vec<u32>,
    pub pending_locks: Vec<(u32, LockSpec)>,
    pub pending_links: Vec<(u32, LinkSpec)>,
    pub pending_releases: Vec<u32>,
    pub next_call: u64,
    pub next_handle: u32,
    /// The project seed and the running scene's name feed the names of random streams.
    pub seed: i64,
    pub scene_name: String,
    pub mod_counts: BTreeMap<String, u32>,
    pub world_cfg: crate::phys::WorldCfg,
    pub decls: crate::phys::Decls,
    pub phys: Option<crate::phys::Physics>,
    pub emitters: BTreeMap<String, crate::particles::EmitterCfg>,
    pub parts: BTreeMap<String, animscript_physics::Emitter>,
    pub soft: Option<crate::soft::SoftSim>,
    pub fluid_cfg: BTreeMap<String, crate::fluid::FluidCfg>,
    pub fluid: Option<crate::fluid::FluidSim>,
    /// Where the program lives: relative asset paths (images) are resolved from here.
    pub base_dir: std::path::PathBuf,
    pub rigs: BTreeMap<String, crate::rig::RigDef>,
    pub rig_sim: Option<crate::rig::RigSim>,
    /// The transforms the layer stack resolved on the last tick; what expressions read as "now".
    pub live_tf: BTreeMap<String, Transform>,
    /// Last solved bone poses `(head, tip, angle)` keyed `armature.bone`.
    pub rig_live: BTreeMap<String, (P, P, f64)>,
    pub derived: Vec<crate::derive::DerivedFlag>,
    pub flag_hist: BTreeMap<(String, String), crate::derive::FlagHist>,
    /// Last solved IK handles `(target, weight)` keyed `armature.handle`.
    pub ik_live: BTreeMap<String, (P, f64)>,
    /// User-defined `action`s and `fn`s by name.
    pub defs: BTreeMap<String, Rc<Def>>,
    /// The value of the last executed `return`.
    pub ret: Option<Value>,
    pub call_depth: usize,
}

pub const TOPOLOGY_METHODS: &[&str] = &[
    "subdivide",
    "extrude",
    "bevel",
    "round",
    "smooth",
    "merge",
    "delete",
    "bend",
    "set_handle",
    "set_handles",
    "straighten",
    "copy",
    "dissolve",
    "inset",
    "mirror",
    "flip",
    "radial_array",
];

fn parse_elem_name(n: &str) -> Option<(Kind, u32)> {
    let mut ch = n.chars();
    let kind = Kind::from_letter(ch.next()?)?;
    let rest: String = ch.collect();
    if rest.is_empty() || rest.starts_with('0') || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((kind, rest.parse().ok()?))
}

fn type_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-TYP-001", msg)
}

impl Interp {
    pub fn new(src: &str, program: Program) -> Res<Interp> {
        let mut it = Interp {
            src: src.to_string(),
            fps: Rat::int(24),
            canvas: (1280, 720),
            background: None,
            shapes: BTreeMap::new(),
            shape_order: vec![],
            entities: BTreeMap::new(),
            entity_order: vec![],
            globals: HashMap::new(),
            diagnostics: vec![],
            tick: 0,
            delta: 0,
            events: BTreeSet::new(),
            staged: vec![],
            trace_on: false,
            trace: vec![],
            pending_keysets: vec![],
            pending_starts: vec![],
            pending_cancels: vec![],
            pending_locks: vec![],
            pending_links: vec![],
            pending_releases: vec![],
            next_call: 0,
            next_handle: 0,
            seed: 0,
            scene_name: String::new(),
            mod_counts: BTreeMap::new(),
            world_cfg: Default::default(),
            decls: Default::default(),
            phys: None,
            emitters: BTreeMap::new(),
            parts: BTreeMap::new(),
            soft: None,
            fluid_cfg: BTreeMap::new(),
            fluid: None,
            base_dir: std::path::PathBuf::from("."),
            rigs: BTreeMap::new(),
            rig_sim: None,
            live_tf: BTreeMap::new(),
            rig_live: BTreeMap::new(),
            derived: vec![],
            flag_hist: BTreeMap::new(),
            ik_live: BTreeMap::new(),
            defs: BTreeMap::new(),
            ret: None,
            call_depth: 0,
            program: Program::default(),
        };
        if let Some(p) = &program.project {
            if p.anim_version.is_none() {
                return Err(Diagnostic::new("E-PRJ-001", "project is missing `anim_version`")
                    .at(p.span)
                    .help("add `anim_version: \"0.1\"` so a future compiler can refuse files it does not understand"));
            }
            it.fps = Rat::new(p.fps.0 as i128, p.fps.1 as i128);
            it.canvas = p.canvas;
            it.seed = p.seed;
            it.background = p.background;
        }
        for e in &program.entities {
            if it.entities.contains_key(&e.name) {
                return Err(Diagnostic::new(
                    "E-NAM-002",
                    format!("duplicate declaration `{}`", e.name),
                )
                .at(e.span));
            }
            let flags = e.states.iter().map(|s| (s.clone(), false)).collect();
            it.entities.insert(
                e.name.clone(),
                Entity {
                    states: e.states.clone(),
                    flags,
                    derived: vec![],
                },
            );
            it.entity_order.push(e.name.clone());
        }
        it.register_derives(&program.entities)?;
        it.program = program;
        it.register_defs()?;
        Ok(it)
    }

    /// Run the top-level statements (the build phase) and publish their `let` bindings as globals.
    pub fn build(&mut self) -> Res<()> {
        self.eval_world()?;
        self.eval_armatures()?; // before the top-level code, which binds shapes to bones
        let block: Block = Rc::new(self.program.top.clone());
        let mut t = Task::new(0, block, Vars::new(), Mode::Build, false);
        self.run_sync(&mut t)?;
        self.globals = t.vars.scopes[0].clone();
        self.freeze_pivots();
        self.eval_emitters()?;
        self.eval_fluids()?;
        Ok(())
    }

    fn running_only(&self, mode: Mode, what: &str, span: Span) -> Res<()> {
        if mode != Mode::Run {
            return Err(
                Diagnostic::new("E-STM-001", format!("`{what}` works while a scene runs"))
                    .at(span)
                    .help("use it in `main:`, a rule or an action"),
            );
        }
        Ok(())
    }

    /// True when the innermost action call declared `uses` without `body` (native shape actions use `body`).
    pub fn undeclared_channel(t: &Task) -> bool {
        t.frames
            .iter()
            .rev()
            .find_map(|f| match f {
                Frame::Call { uses, .. } => Some(uses),
                _ => None,
            })
            .is_some_and(|u| !u.is_empty() && !u.iter().any(|c| c == "body"))
    }

    /// Run a `finally:` body: zero-time statements only, with a copy of the task's variables.
    pub fn run_final(&mut self, body: &Block, vars: &Vars, mode: Mode) -> Res<()> {
        let mut t = Task::new(0, Rc::clone(body), vars.flatten(), mode, true);
        match self.step(&mut t) {
            Ok(Step::Done) => Ok(()),
            Ok(Step::Blocked(_)) => Err(Self::final_blocks()),
            Err(d) if d.code == "E-STM-001" => Err(Self::final_blocks()),
            Err(d) => Err(d),
        }
    }

    fn final_blocks() -> Diagnostic {
        Diagnostic::new("E-STM-003", "a `finally:` block cannot wait or run a timed action")
            .help("`finally:` runs while a task is being stopped: use only zero-time statements (`note`, `let`, `becomes`, `emit`, ...)")
    }

    /// The `finally:` bodies of a task's blocks that have not reached them yet, innermost first.
    pub fn pending_finals(t: &Task) -> Vec<Block> {
        let mut out = vec![];
        for f in t.frames.iter().rev() {
            if let Frame::Block { stmts, idx, .. } = f {
                if let Some(Stmt {
                    kind: StmtKind::Finally(b),
                    ..
                }) = stmts.last()
                {
                    if *idx < stmts.len() {
                        out.push(Rc::clone(b));
                    }
                }
            }
        }
        out
    }

    /// Run a task that must not block (build and setup code) to completion.
    pub fn run_sync(&mut self, t: &mut Task) -> Res<()> {
        while let Step::Blocked(_) = self.step(t)? {}
        Ok(())
    }

    pub fn trace_line(&mut self, kind: &str, detail: &str) {
        if self.trace_on {
            let l = format!(
                "tick {:03} d{} {:<7} {}",
                self.tick, self.delta, kind, detail
            );
            self.trace.push(l);
        }
    }

    pub fn span_text(&self, sp: Span) -> String {
        self.src
            .lines()
            .nth(sp.line as usize - 1)
            .map(|l| {
                l.chars()
                    .skip(sp.col as usize - 1)
                    .take((sp.end_col - sp.col) as usize)
                    .collect()
            })
            .unwrap_or_default()
    }

    // ---- task stepping -----------------------------------------------------------------

    pub fn step(&mut self, t: &mut Task) -> Res<Step> {
        let mut budget = 5_000_000u32;
        loop {
            budget -= 1;
            if budget == 0 {
                return Err(Diagnostic::new(
                    "E-STM-003",
                    "a loop that never blocks ran for too long",
                )
                .help("a loop body in `main:` or a rule must contain a `wait` or a timed action"));
            }
            let Some(top) = t.frames.last_mut() else {
                return Ok(Step::Done);
            };
            match top {
                Frame::Block { stmts, idx, scope } => {
                    if *idx >= stmts.len() {
                        let sc = *scope;
                        t.frames.pop();
                        if sc {
                            t.vars.pop();
                        }
                        continue;
                    }
                    let stmts = Rc::clone(stmts);
                    let i = *idx;
                    *idx += 1;
                    let st = &stmts[i];
                    if let Some(req) = self.exec(st, t).map_err(|d| d.or_at(st.span))? {
                        if t.mode != Mode::Run {
                            return Err(Diagnostic::new("E-STM-001", "blocking statement outside a running scene")
                                .at(st.span)
                                .help("build blocks and `setup:` may contain only zero-time statements; move this to `main:` or a rule"));
                        }
                        return Ok(Step::Blocked(req));
                    }
                }
                Frame::Call { .. } => {
                    if let Some(Frame::Call { saved: Some(v), .. }) = t.frames.pop() {
                        t.vars = v;
                    }
                }
                Frame::While { cond, body } => {
                    let (c, b) = (Rc::clone(cond), Rc::clone(body));
                    match self
                        .eval(&c, &t.vars, t.mode)
                        .map_err(|d| d.or_at(c.span))?
                    {
                        Value::Bool(true) => t.push_block(b, vec![]),
                        Value::Bool(false) => {
                            t.frames.pop();
                        }
                        other => {
                            return Err(type_err(format!(
                                "a `while` condition must be a Bool, found {}",
                                other.type_name()
                            ))
                            .at(c.span))
                        }
                    }
                }
                Frame::Repeat { body, left } => {
                    if *left <= 0 {
                        t.frames.pop();
                        continue;
                    }
                    *left -= 1;
                    let b = Rc::clone(body);
                    t.push_block(b, vec![]);
                }
                Frame::For {
                    var,
                    items,
                    idx,
                    body,
                } => {
                    if *idx >= items.len() {
                        t.frames.pop();
                        continue;
                    }
                    let v = items[*idx].clone();
                    let var = var.clone();
                    *idx += 1;
                    let b = Rc::clone(body);
                    t.push_block(b, vec![(var, v)]);
                }
            }
        }
    }

    fn exec(&mut self, st: &Stmt, t: &mut Task) -> Res<Option<Req>> {
        let mode = t.mode;
        match &st.kind {
            StmtKind::Pass => {}
            StmtKind::Let(name, e) => {
                let v = self.eval(e, &t.vars, mode)?;
                if matches!(v, Value::Call(_)) {
                    return Err(type_err("an action call is not a value; run it as a statement or with `start`").help("`let h = start thing.action(...)` runs it in the background and gives a handle"));
                }
                if matches!(v, Value::Fresh(_)) {
                    return Err(
                        type_err("a new shape must be declared with `shape name = ...`")
                            .help("`let` cannot bind a freshly created shape"),
                    );
                }
                t.vars.define(name, v);
            }
            StmtKind::ShapeDecl { name, expr, body } => {
                if mode == Mode::Run {
                    return Err(Diagnostic::new(
                        "E-SHP-003",
                        "shapes cannot be created while the scene is running",
                    )
                    .help("declare shapes at the top level or in `setup:`"));
                }
                let v = self.eval(expr, &t.vars, mode)?;
                let Value::Fresh(mut sh) = v else {
                    return Err(type_err(format!("`shape {name} = ...` needs a new shape, found {}", v.type_name()))
                        .help("create one with `square`, `rectangle`, `triangle`, `circle`, `polygon`, `line` or `.copy()`"));
                };
                if self.shapes.contains_key(name)
                    || self.entities.contains_key(name)
                    || t.vars.get(name).is_some()
                {
                    return Err(Diagnostic::new(
                        "E-NAM-002",
                        format!("duplicate declaration `{name}`"),
                    ));
                }
                sh.name = name.clone();
                self.shapes.insert(name.clone(), *sh);
                self.shape_order.push(name.clone());
                if let Some(b) = body {
                    t.push_block(Rc::clone(b), vec![]);
                }
            }
            StmtKind::Assign(target, e) => {
                let v = self.eval(e, &t.vars, mode)?;
                if matches!(v, Value::Call(_)) {
                    return Err(type_err(
                        "an action call cannot be stored; run it as a statement or with `start`",
                    ));
                }
                if let ExprKind::Name(n) = &target.kind {
                    if !t.vars.assign(n, v) {
                        return Err(Diagnostic::new(
                            "E-NAM-001",
                            format!("`{n}` was not declared, so it cannot be assigned"),
                        )
                        .at(target.span)
                        .help(format!("declare it first with `let {n} = ...`")));
                    }
                } else {
                    self.assign(target, v, &t.vars, mode)?;
                }
            }
            StmtKind::While { cond, body } => {
                t.frames.push(Frame::While {
                    cond: Rc::new(cond.clone()),
                    body: Rc::clone(body),
                });
            }
            StmtKind::Return(e) => {
                let v = match e {
                    Some(e) => Some(self.eval(e, &t.vars, mode)?),
                    None => None,
                };
                self.do_return(t, v);
            }
            StmtKind::Expr { expr, over, ease } => {
                let extra = {
                    let mut x = vec![];
                    if let Some(o) = over {
                        x.push((Some("over".to_string()), self.eval(o, &t.vars, mode)?));
                    }
                    if let Some(o) = ease {
                        x.push((Some("ease".to_string()), self.eval(o, &t.vars, mode)?));
                    }
                    x
                };
                let v = if extra.is_empty() {
                    self.eval(expr, &t.vars, mode)?
                } else if let ExprKind::Call(f, args) = &expr.kind {
                    self.eval_call(expr, f, args, extra, &t.vars, mode)?
                } else {
                    return Err(Diagnostic::new(
                        "E-SYN-001",
                        "`over` / `ease` can only follow a call",
                    )
                    .at(expr.span));
                };
                match v {
                    Value::Call(c) => self.enter_call(t, &c)?,
                    Value::Pending(req) => return Ok(Some(Req::Act(*req))),
                    Value::Modifier(m) => {
                        let Some(frames) = m.frames else {
                            return Err(Diagnostic::new("E-TYP-003", "an open-ended modifier needs `over` or `start`")
                                .help("write `... over 2.seconds`, or `let h = start ...` and later `cancel h`"));
                        };
                        self.pending_starts.push((0, StartReq::Mod(*m)));
                        return Ok(Some(Req::Wait(frames)));
                    }
                    Value::Fresh(_) => {
                        return Err(type_err(
                            "a new shape must be declared with `shape name = ...`",
                        ))
                    }
                    _ => {}
                }
            }
            StmtKind::For { var, iter, body } => {
                let items = match self.eval(iter, &t.vars, mode)? {
                    Value::List(l) => l,
                    other => {
                        return Err(
                            type_err(format!("cannot loop over a {}", other.type_name()))
                                .help("loop over a range `1..4` or a list"),
                        )
                    }
                };
                t.frames.push(Frame::For {
                    var: var.clone(),
                    items,
                    idx: 0,
                    body: Rc::clone(body),
                });
            }
            StmtKind::If { cond, then, else_ } => {
                let c = match self.eval(cond, &t.vars, mode)? {
                    Value::Bool(b) => b,
                    other => {
                        return Err(type_err(format!(
                            "`if` needs a Bool, found {}",
                            other.type_name()
                        )))
                    }
                };
                if c {
                    t.push_block(Rc::clone(then), vec![]);
                } else if let Some(e) = else_ {
                    t.push_block(Rc::clone(e), vec![]);
                }
            }
            StmtKind::Repeat { count, body } => match self.eval(count, &t.vars, mode)? {
                Value::Int(n) if n >= 0 => t.frames.push(Frame::Repeat {
                    body: Rc::clone(body),
                    left: n,
                }),
                other => {
                    return Err(type_err(format!(
                        "`repeat` needs a non-negative Int, found {}",
                        other.type_name()
                    )))
                }
            },
            StmtKind::Wait(e) => {
                let v = self.eval(e, &t.vars, mode)?;
                let n = self.frames_of(&v, e.span)?;
                return Ok(Some(Req::Wait(n)));
            }
            StmtKind::Start {
                name,
                call,
                over,
                ease,
            } => {
                let mut extra = vec![];
                if let Some(o) = over {
                    extra.push((Some("over".to_string()), self.eval(o, &t.vars, mode)?));
                }
                if let Some(o) = ease {
                    extra.push((Some("ease".to_string()), self.eval(o, &t.vars, mode)?));
                }
                let ExprKind::Call(f, args) = &call.kind else {
                    return Err(Diagnostic::new("E-SYN-001", "`start` needs a call")
                        .at(call.span)
                        .help("for example `start s.rotate_by(90.deg) over 1.second`"));
                };
                let req =
                    match self.eval_call(call, f, args, extra, &t.vars, mode)? {
                        Value::Pending(a) => StartReq::Act(*a),
                        Value::Modifier(m) => StartReq::Mod(*m),
                        Value::Call(c) => StartReq::Task(c),
                        other => return Err(type_err(format!(
                            "`start` needs a timed action or a modifier, found {}",
                            other.type_name()
                        ))
                        .at(call.span)
                        .help(
                            "give an action `over` a duration, or use a modifier such as `wobble`",
                        )),
                    };
                self.next_handle += 1;
                let h = self.next_handle;
                self.pending_starts.push((h, req));
                if let Some(n) = name {
                    t.vars.define(n, Value::Handle(h));
                }
            }
            StmtKind::Cancel(e) => match self.eval(e, &t.vars, mode)? {
                Value::Handle(h) => self.pending_cancels.push(h),
                other => {
                    return Err(type_err(format!(
                        "`cancel` needs a handle from `start`, found {}",
                        other.type_name()
                    )))
                }
            },
            StmtKind::Await(e) => match self.eval(e, &t.vars, mode)? {
                Value::Handle(h) => return Ok(Some(Req::Await(h))),
                other => {
                    return Err(type_err(format!(
                        "`await` needs a handle from `start`, found {}",
                        other.type_name()
                    )))
                }
            },
            StmtKind::Keyframes {
                target,
                over,
                opts,
                keys,
            } => {
                let tv = self.eval(target, &t.vars, mode)?;
                if mode != Mode::Run {
                    return Err(Diagnostic::new("E-SHP-005", "`keyframes` needs a running scene")
                        .help("use it in `main:` or a rule; at build time just assign the value, e.g. `s.rotation = 45.degrees`"));
                }
                let ov = self.eval(over, &t.vars, mode)?;
                let d = self.frames_of(&ov, over.span)?;
                let mut blend = [0i64; 2];
                for (i, e) in [&opts.blend_in, &opts.blend_out].into_iter().enumerate() {
                    if let Some(e) = e {
                        let v = self.eval(e, &t.vars, mode)?;
                        blend[i] = self.frames_of(&v, e.span)?;
                    }
                }
                let specs = self.build_keysets(
                    Some(&tv),
                    keys,
                    Some(d),
                    &t.vars,
                    mode,
                    false,
                    blend,
                    (opts.hold_before, opts.hold_after),
                )?;
                self.pending_keysets.extend(specs);
                return Ok(Some(Req::Wait(d)));
            }
            StmtKind::Together(body) => {
                let kids: Vec<Block> = body.iter().map(|s| Rc::new(vec![s.clone()])).collect();
                return Ok(Some(Req::Together(kids)));
            }
            StmtKind::Lock {
                name,
                target,
                value,
                duration,
            } => {
                self.running_only(mode, "lock", st.span)?;
                let (texpr, path) = self.split_prop_path(target)?;
                let Some(te) = texpr else {
                    return Err(
                        type_err("`lock` needs a target such as `bird.position.y`").at(target.span)
                    );
                };
                let tv = self.eval(te, &t.vars, mode)?;
                let val = self.eval(value, &t.vars, mode)?;
                let comps = self.key_scalars(&tv, &path, &val, st.span)?;
                let frames = match duration {
                    Some(d) => {
                        let dv = self.eval(d, &t.vars, mode)?;
                        Some(self.frames_of(&dv, d.span)?)
                    }
                    None => None,
                };
                self.next_handle += 1;
                let h = self.next_handle;
                self.pending_locks.push((
                    h,
                    LockSpec {
                        comps,
                        frames,
                        span: st.span,
                    },
                ));
                if let Some(n) = name {
                    t.vars.define(n, Value::Handle(h));
                }
            }
            StmtKind::Release(e) => {
                self.running_only(mode, "release", st.span)?;
                match self.eval(e, &t.vars, mode)? {
                    Value::Handle(h) => self.pending_releases.push(h),
                    other => {
                        return Err(type_err(format!(
                        "`release` needs a handle from `lock`, `attach` or `constrain`, found {}",
                        other.type_name()
                    )))
                    }
                }
            }
            StmtKind::Attach {
                name,
                child,
                parent,
                offset,
            } => {
                self.running_only(mode, "attach", st.span)?;
                let shape_of = |me: &mut Interp, e: &Expr, what: &str| -> Res<String> {
                    match me.eval(e, &t.vars, mode)? {
                        Value::Shape(n) => Ok(n),
                        other => Err(type_err(format!(
                            "{what} must be a shape or a bone, found {}",
                            other.type_name()
                        ))
                        .at(e.span)),
                    }
                };
                let c = shape_of(self, child, "the child of `attach`")?;
                let p = shape_of(self, parent, "the parent of `attach`")?;
                let offset = match offset {
                    Some(o) => {
                        let v = self.eval(o, &t.vars, mode)?;
                        Some(self.point(&v, "`offset`")?)
                    }
                    None => None,
                };
                self.next_handle += 1;
                let h = self.next_handle;
                self.pending_links.push((
                    h,
                    LinkSpec::Attach {
                        child: c,
                        parent: p,
                        offset,
                        span: st.span,
                    },
                ));
                if let Some(n) = name {
                    t.vars.define(n, Value::Handle(h));
                }
            }
            StmtKind::Constrain {
                name,
                target,
                kind,
                weight,
            } => {
                self.running_only(mode, "constrain", st.span)?;
                // `constrain s.position ...` and `constrain s ...` both name the shape s
                let te = match &target.kind {
                    ExprKind::Member(o, n) if n == "position" => o.as_ref(),
                    _ => target,
                };
                let Value::Shape(shape) = self.eval(te, &t.vars, mode)? else {
                    return Err(type_err(
                        "`constrain` needs a shape, such as `constrain s.position inside canvas`",
                    )
                    .at(target.span));
                };
                let weight = match weight {
                    Some(w) => {
                        let v = self.eval(w, &t.vars, mode)?;
                        self.num(&v, "`weight`")?.clamp(0.0, 1.0)
                    }
                    None => 1.0,
                };
                let spec = match kind {
                    ConKind::Inside(r) => match self.eval(r, &t.vars, mode)? {
                        Value::Rect(rect) => LinkSpec::Inside { shape, rect, weight },
                        other => return Err(type_err(format!("`inside` needs a region such as `canvas` or `rect(x, y, w, h)`, found {}", other.type_name())).at(r.span)),
                    },
                    ConKind::Follow { target: tg, lag } => {
                        let Value::Shape(other) = self.eval(tg, &t.vars, mode)? else {
                            return Err(type_err("`follow` needs a shape").at(tg.span));
                        };
                        let lag = match lag {
                            Some(l) => {
                                let v = self.eval(l, &t.vars, mode)?;
                                self.frames_of(&v, l.span)?
                            }
                            None => 0,
                        };
                        LinkSpec::Follow { shape, target: other, lag, weight }
                    }
                };
                self.next_handle += 1;
                let h = self.next_handle;
                self.pending_links.push((h, spec));
                if let Some(n) = name {
                    t.vars.define(n, Value::Handle(h));
                }
            }
            StmtKind::Acquire(chans, prio) => {
                let shape = match t.vars.get("self") {
                    Some(Value::Shape(n)) | Some(Value::Armature(n)) => n.clone(),
                    _ => "_".to_string(),
                };
                let call = t
                    .frames
                    .iter()
                    .rev()
                    .find_map(|f| match f {
                        Frame::Call { id, .. } => Some(*id),
                        _ => None,
                    })
                    .unwrap_or(0);
                let prio = if *prio != 0 { *prio } else { t.priority };
                if mode == Mode::Run {
                    return Ok(Some(Req::Acquire(AcquireReq {
                        shape,
                        channels: chans.clone(),
                        prio,
                        call,
                    })));
                }
            }
            StmtKind::Race(body) => {
                let kids: Vec<Block> = body.iter().map(|s| Rc::new(vec![s.clone()])).collect();
                return Ok(Some(Req::Race(kids)));
            }
            StmtKind::Finally(body) => {
                // reached by running off the end of its block: the normal way a block finishes
                self.run_final(body, &t.vars, mode)?;
            }
            StmtKind::Become { target, flag, set } => {
                let ent = self.entities.get(target).ok_or_else(|| {
                    Diagnostic::new("E-NAM-001", format!("unknown entity `{target}`"))
                        .help("declare it with `prop name:` and `states: ...`")
                })?;
                if ent.derived.contains(flag) {
                    return Err(Diagnostic::new(
                        "E-STA-001",
                        format!("`{flag}` of `{target}` is derived, so it cannot be set or cleared"),
                    )
                    .help("a derived flag follows its `derive ... when` condition; change what the condition looks at"));
                }
                if !ent.states.contains(flag) {
                    return Err(Diagnostic::new(
                        "E-NAM-003",
                        format!("`{target}` has no authored flag `{flag}`"),
                    )
                    .help(format!("declared flags: {}", ent.states.join(", "))));
                }
                let (entity, flag) = (target.clone(), flag.clone());
                self.apply_or_stage(
                    Staged::Flag {
                        entity,
                        flag,
                        value: *set,
                    },
                    mode,
                )?;
            }
            StmtKind::Note(s) => {
                self.trace_line("NOTE", s);
            }
            StmtKind::Assert(e) => match self.eval(e, &t.vars, mode)? {
                Value::Bool(true) => {}
                Value::Bool(false) => {
                    return Err(Diagnostic::new("E-RUN-004", "assertion failed")
                        .label("this condition is false"));
                }
                other => {
                    return Err(type_err(format!(
                        "`assert` needs a Bool, found {}",
                        other.type_name()
                    )))
                }
            },
        }
        Ok(None)
    }

    pub fn apply_or_stage(&mut self, s: Staged, mode: Mode) -> Res<()> {
        if mode == Mode::Run {
            self.staged.push(s);
            Ok(())
        } else {
            self.apply_staged_one(s)
        }
    }

    /// Apply every staged effect together at the end of a delta. Returns how many there were.
    pub fn apply_staged(&mut self) -> Res<usize> {
        let all = std::mem::take(&mut self.staged);
        let n = all.len();
        for s in all {
            self.apply_staged_one(s)?;
        }
        Ok(n)
    }

    fn apply_staged_one(&mut self, s: Staged) -> Res<()> {
        match s {
            Staged::Flag {
                entity,
                flag,
                value,
            } => {
                if let Some(e) = self.entities.get_mut(&entity) {
                    e.flags.insert(flag, value);
                }
            }
            Staged::Move { shape, r, x, y, by } => {
                let sh = self.shapes.get_mut(&shape).unwrap();
                sh.warn_on_edit = false;
                if by {
                    sh.move_by(r, x, y)?;
                } else {
                    sh.move_to(r, x, y)?;
                }
            }
            Staged::Style { shape, prop, value } => self.set_style(&shape, &prop, value)?,
            Staged::Transform {
                shape,
                prop,
                value,
                by,
            } => set_transform_prop(self.shapes.get_mut(&shape).unwrap(), prop, value, by),
            Staged::ElemStyle {
                shape,
                r,
                prop,
                value,
            } => self.set_elem_style(&shape, r, &prop, &value)?,
        }
        Ok(())
    }

    fn set_elem_style(&mut self, shape: &str, r: ElemRef, prop: &str, value: &Value) -> Res<()> {
        let sh = self.shapes.get_mut(shape).unwrap();
        match (prop, value) {
            ("fill", Value::Color(c)) => sh.set_face_fill(r, Some(*c)),
            ("fill", Value::None) => sh.set_face_fill(r, None),
            ("stroke", Value::Color(c)) => sh.set_edge_stroke(r, Some(*c)),
            ("stroke", Value::None) => sh.set_edge_stroke(r, None),
            ("stroke_width", Value::None) => sh.set_edge_width(r, None),
            ("stroke_width", v) if v.as_f64().is_some() => sh.set_edge_width(r, v.as_f64()),
            _ => Err(type_err(format!("cannot set `{prop}` to a {}", value.type_name()))
                .help("fill and stroke take a colour like #f0a (or `none` to inherit), stroke_width a number")),
        }
    }

    fn set_style(&mut self, shape: &str, prop: &str, value: Value) -> Res<()> {
        let sh = self.shapes.get_mut(shape).unwrap();
        match (prop, &value) {
            ("fill", Value::Color(c)) => sh.fill = Some(*c),
            ("fill", Value::None) => sh.fill = None,
            ("fill" | "image", Value::Image(img)) => sh.set_image(img.clone()),
            ("image", Value::None) => {
                sh.image = None;
                sh.uv.clear();
            }
            ("stroke", Value::Color(c)) => sh.stroke = Some(*c),
            ("stroke", Value::None) => sh.stroke = None,
            ("stroke_width", v) if v.as_f64().is_some() => sh.stroke_width = v.as_f64().unwrap(),
            ("z", Value::Int(i)) => sh.z = *i,
            ("visible", Value::Bool(b)) => sh.visible = *b,
            ("opacity", v) if v.as_f64().is_some() => sh.opacity = v.as_f64().unwrap().clamp(0.0, 1.0),
            ("tint", Value::Color(c)) => sh.tint = *c,
            ("position", Value::Tuple(t)) if t.len() == 2 && t.iter().all(|x| x.as_f64().is_some()) => {
                sh.position = [t[0].as_f64().unwrap(), t[1].as_f64().unwrap()]
            }
            ("scale", v) if v.as_f64().is_some() => sh.scale = [v.as_f64().unwrap(); 2],
            ("scale", Value::Tuple(t)) if t.len() == 2 && t.iter().all(|x| x.as_f64().is_some()) => {
                sh.scale = [t[0].as_f64().unwrap(), t[1].as_f64().unwrap()]
            }
            ("rotation", Value::Angle(a)) => sh.rotation = *a,
            ("pivot", Value::Tuple(t)) if t.len() == 2 && t.iter().all(|x| x.as_f64().is_some()) => {
                sh.pivot = Some([t[0].as_f64().unwrap(), t[1].as_f64().unwrap()])
            }
            ("pivot", Value::None) => sh.pivot = None,
            _ => {
                return Err(type_err(format!("cannot set `{prop}` to a {}", value.type_name()))
                    .help("fill/stroke take a colour like #f0a, stroke_width a number, z an Int, visible a Bool"))
            }
        }
        Ok(())
    }

    fn assign(&mut self, target: &Expr, v: Value, vars: &Vars, mode: Mode) -> Res<()> {
        let ExprKind::Member(obj, attr) = &target.kind else {
            return Err(Diagnostic::new("E-SYN-001", "only members can be assigned")
                .at(target.span)
                .help("use `let x = ...` to bind a name"));
        };
        match self.eval(obj, vars, mode)? {
            Value::Shape(sn) => {
                if !matches!(
                    attr.as_str(),
                    "fill"
                        | "stroke"
                        | "stroke_width"
                        | "z"
                        | "visible"
                        | "position"
                        | "scale"
                        | "rotation"
                        | "pivot"
                        | "opacity"
                        | "tint"
                        | "image"
                ) {
                    return Err(Diagnostic::new(
                        "E-NAM-003",
                        format!("cannot assign to `{sn}.{attr}`"),
                    )
                    .help("assignable members: fill, stroke, stroke_width, z, visible, position, scale, rotation, pivot, opacity, tint, image"));
                }
                self.check_style_value(attr, &v)?;
                self.apply_or_stage(
                    Staged::Style {
                        shape: sn,
                        prop: attr.clone(),
                        value: v,
                    },
                    mode,
                )
            }
            Value::Elem(sn, r) if matches!(attr.as_str(), "fill" | "stroke" | "stroke_width") => {
                self.assign_elem_style(&sn, r, attr, v, mode)
            }
            Value::List(items) if matches!(attr.as_str(), "fill" | "stroke" | "stroke_width") => {
                for item in items {
                    let Value::Elem(sn, r) = item else {
                        return Err(type_err(format!(
                            "`{attr}` can only be set on faces and edges, found a {} in the list",
                            item.type_name()
                        )));
                    };
                    self.assign_elem_style(&sn, r, attr, v.clone(), mode)?;
                }
                Ok(())
            }
            Value::Elem(sn, r) if attr == "name" => {
                let Value::Str(name) = v else {
                    return Err(type_err("a semantic name must be a string"));
                };
                if mode == Mode::Run {
                    return self.guard_topology(mode, "name");
                }
                self.with_shape(&sn, mode, target.span, |sh| sh.set_alias(r, &name))
            }
            other => Err(type_err(format!(
                "cannot assign to a member of {}",
                other.type_name()
            ))),
        }
    }

    /// Check the value of a shape property assignment now, so a bad value is reported at the assignment.
    pub(crate) fn load_image(&self, path: &str) -> Res<Value> {
        let full = self.base_dir.join(path);
        let bytes = std::fs::read(&full).map_err(|_| {
            Diagnostic::new("E-AST-001", format!("image file `{path}` was not found")).help(
                format!(
                    "paths are relative to the program's folder ({})",
                    self.base_dir.display()
                ),
            )
        })?;
        const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        if bytes.len() < 33 || bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
            return Err(
                Diagnostic::new("E-AST-002", format!("`{path}` is not a PNG image"))
                    .help("only PNG files are supported"),
            );
        }
        let be =
            |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        let (w, h) = (be(16), be(20));
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return Err(Diagnostic::new(
                "E-AST-002",
                format!("`{path}` has an unusable size ({w} x {h})"),
            ));
        }
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in &bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Ok(Value::Image(ImageFill {
            src: path.to_string(),
            w,
            h,
            hash,
            rect: [0.0; 4],
        }))
    }

    fn check_style_value(&self, prop: &str, v: &Value) -> Res<()> {
        let pair = |v: &Value| matches!(v, Value::Tuple(t) if t.len() == 2 && t.iter().all(|x| x.as_f64().is_some()));
        match (prop, v) {
            ("fill" | "stroke", Value::Color(_) | Value::None) | ("visible", Value::Bool(_)) | ("z", Value::Int(_)) => Ok(()),
            ("fill" | "image", Value::Image(_)) | ("image", Value::None) => Ok(()),
            ("stroke_width", v) if v.as_f64().is_some() => Ok(()),
            ("position" | "pivot", v) if pair(v) => Ok(()),
            ("pivot", Value::None) => Ok(()),
            ("scale", v) if v.as_f64().is_some() || pair(v) => Ok(()),
            ("rotation", Value::Angle(_)) => Ok(()),
            ("opacity", v) if v.as_f64().is_some_and(|o| (0.0..=1.0).contains(&o)) => Ok(()),
            ("opacity", Value::Int(_) | Value::Num(_)) => Err(type_err("`opacity` must be between 0 and 1").help("0 is invisible, 1 is solid")),
            ("tint", Value::Color(_)) => Ok(()),
            ("rotation", Value::Int(_) | Value::Num(_)) => Err(Diagnostic::new("E-TYP-002", "`rotation` needs an Angle, found a bare number")
                .help("a bare number has no unit; write `90.degrees`")),
            ("scale" | "position" | "pivot", Value::Tuple(t)) if t.iter().any(|x| matches!(x, Value::Dur(_) | Value::Frames(_) | Value::Angle(_))) => {
                Err(Diagnostic::new("E-TYP-002", format!("`{prop}` takes plain numbers, found a value with a unit")))
            }
            (_, other) => Err(type_err(format!("cannot set `{prop}` to a {}", other.type_name()))
                .help("fill/stroke take a colour like #f0a or `none`, stroke_width a number, z an Int, visible a Bool, position/pivot a point `(x, y)`, scale a number or `(sx, sy)`, rotation an Angle")),
        }
    }

    fn assign_elem_style(
        &mut self,
        sn: &str,
        r: ElemRef,
        attr: &str,
        v: Value,
        mode: Mode,
    ) -> Res<()> {
        if r.kind == Kind::V {
            return Err(
                Diagnostic::new("E-NAM-003", format!("a vertex has no `{attr}`"))
                    .help("colour faces with `fill` and edges with `stroke` / `stroke_width`"),
            );
        }
        let needed = if attr == "fill" { Kind::F } else { Kind::E };
        self.shapes[sn].require_live(r, Some(needed), attr)?;
        match (attr, &v) {
            ("fill" | "stroke", Value::Color(_) | Value::None) | ("stroke_width", Value::None) => {}
            ("stroke_width", x) if x.as_f64().is_some() => {}
            (_, other) => {
                return Err(type_err(format!("cannot set `{attr}` to a {}", other.type_name()))
                    .help("fill and stroke take a colour like #f0a (or `none` to inherit), stroke_width a number"))
            }
        }
        self.apply_or_stage(
            Staged::ElemStyle {
                shape: sn.to_string(),
                r,
                prop: attr.to_string(),
                value: v,
            },
            mode,
        )
    }

    // ---- helpers -----------------------------------------------------------------------

    fn guard_topology(&self, mode: Mode, name: &str) -> Res<()> {
        if mode == Mode::Run {
            return Err(Diagnostic::new("E-SHP-003", format!("`{name}` changes topology and is not allowed while the scene is running"))
                .help("topology edits (subdivide, extrude, bevel, round, smooth, merge, delete, bend) run in `setup:` or a `shape` build block")
                .help("at runtime only vertex positions animate: `shape.v[2].move_by(0, 10) over 0.3.seconds`"));
        }
        Ok(())
    }

    fn with_shape<R>(
        &mut self,
        sn: &str,
        mode: Mode,
        span: Span,
        f: impl FnOnce(&mut Shape) -> Res<R>,
    ) -> Res<R> {
        let sh = self.shapes.get_mut(sn).expect("shape registered");
        sh.warn_on_edit = mode != Mode::Run;
        let r = f(sh);
        for w in std::mem::take(&mut sh.warnings) {
            self.diagnostics.push(w.or_at(span));
        }
        r
    }

    fn to_dur(&self, v: &Value) -> Option<Rat> {
        match v {
            Value::Dur(r) => Some(*r),
            Value::Frames(n) => Rat::int(*n).div(self.fps),
            _ => None,
        }
    }

    /// Duration -> whole ticks, rounding half to even; a positive duration never becomes 0 (W-TIM-001).
    pub fn frames_of(&mut self, v: &Value, span: Span) -> Res<i64> {
        match v {
            Value::Frames(n) if *n >= 0 => Ok(*n),
            Value::Dur(r) if r.n >= 0 => {
                let mut n = r.mul(self.fps).round_half_even() as i64;
                if r.n > 0 && n == 0 {
                    n = 1;
                    self.diagnostics.push(
                        Diagnostic::new("W-TIM-001", "sub-frame duration raised to one frame")
                            .at(span)
                            .help(
                                "durations are whole ticks; use a duration of at least one frame",
                            ),
                    );
                }
                Ok(n)
            }
            Value::Dur(_) | Value::Frames(_) => Err(type_err("a duration cannot be negative")),
            Value::Int(_) | Value::Num(_) => Err(Diagnostic::new(
                "E-TYP-002",
                "expected a Duration, found a bare number",
            )
            .help("a bare number has no unit; write `2.seconds` or `12.frames`")),
            other => Err(type_err(format!(
                "expected a Duration, found {}",
                other.type_name()
            ))),
        }
    }

    pub(crate) fn num(&self, v: &Value, what: &str) -> Res<f64> {
        match v {
            Value::Int(i) => Ok(*i as f64),
            Value::Num(n) => Ok(*n),
            Value::Dur(_) | Value::Frames(_) | Value::Angle(_) => Err(Diagnostic::new(
                "E-TYP-002",
                format!("{what} must be a plain number, found {}", v.type_name()),
            )),
            other => Err(type_err(format!(
                "{what} must be a number, found {}",
                other.type_name()
            ))),
        }
    }
    pub(crate) fn int(&self, v: &Value, what: &str) -> Res<i64> {
        match v {
            Value::Int(i) => Ok(*i),
            other => Err(type_err(format!(
                "{what} must be an Int, found {}",
                other.type_name()
            ))),
        }
    }
    pub(crate) fn angle(&self, v: &Value, what: &str) -> Res<f64> {
        match v {
            Value::Angle(t) => Ok(*t),
            Value::Int(_) | Value::Num(_) => Err(Diagnostic::new(
                "E-TYP-002",
                format!("{what} needs an Angle, found a bare number"),
            )
            .help("a bare number has no unit; write `20.degrees`")),
            other => Err(type_err(format!(
                "{what} must be an Angle, found {}",
                other.type_name()
            ))),
        }
    }
    pub(crate) fn point(&self, v: &Value, what: &str) -> Res<[f64; 2]> {
        match v {
            Value::Tuple(t) if t.len() == 2 => Ok([self.num(&t[0], what)?, self.num(&t[1], what)?]),
            Value::Elem(sn, r) if r.kind == Kind::V => {
                let sh = &self.shapes[sn];
                sh.require_live(*r, Some(Kind::V), what)?;
                Ok(sh.pos(r.id))
            }
            other => Err(type_err(format!(
                "{what} must be a point like `(10, 20)`, found {}",
                other.type_name()
            ))),
        }
    }

    pub(crate) fn bind(
        &self,
        fname: &str,
        args: Vec<(Option<String>, Value)>,
        params: &[&'static str],
        required: &[&str],
    ) -> Res<HashMap<&'static str, Value>> {
        let mut out: HashMap<&'static str, Value> = HashMap::new();
        let mut pos = 0usize;
        for (name, v) in args {
            match name {
                None => {
                    if pos >= params.len() {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("too many arguments to `{fname}`"),
                        )
                        .help(format!("`{fname}` takes: {}", params.join(", "))));
                    }
                    out.insert(params[pos], v);
                    pos += 1;
                }
                Some(n) => {
                    let Some(p) = params.iter().find(|p| **p == n) else {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("`{fname}` has no argument named `{n}`"),
                        )
                        .help(format!("`{fname}` takes: {}", params.join(", "))));
                    };
                    if out.insert(p, v).is_some() {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("argument `{n}` given twice to `{fname}`"),
                        ));
                    }
                }
            }
        }
        for r in required {
            if !out.contains_key(r) {
                return Err(Diagnostic::new(
                    "E-TYP-003",
                    format!("`{fname}` is missing the argument `{r}`"),
                )
                .help(format!("`{fname}` takes: {}", params.join(", "))));
            }
        }
        Ok(out)
    }

    // ---- expressions -------------------------------------------------------------------

    pub fn eval(&mut self, e: &Expr, vars: &Vars, mode: Mode) -> Res<Value> {
        self.eval_inner(e, vars, mode).map_err(|d| d.or_at(e.span))
    }

    fn eval_inner(&mut self, e: &Expr, vars: &Vars, mode: Mode) -> Res<Value> {
        match &e.kind {
            ExprKind::Num(t) => {
                if t.chars().all(|c| c.is_ascii_digit()) {
                    t.parse::<i64>()
                        .map(Value::Int)
                        .map_err(|_| type_err("integer literal too large"))
                } else {
                    t.parse::<f64>()
                        .map(Value::Num)
                        .map_err(|_| type_err(format!("bad number `{t}`")))
                }
            }
            ExprKind::Unit { num, unit } => self.unit_value(num, unit),
            ExprKind::Str(s) => Ok(Value::Str(s.clone())),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Color(c) => Ok(Value::Color(*c)),
            ExprKind::Name(n) => self.lookup(n, vars),
            ExprKind::Member(o, n) => {
                if n == "rose" {
                    if let Some(v) = self.flag_rose(o, vars) {
                        return Ok(v);
                    }
                }
                let ov = self.eval(o, vars, mode)?;
                self.get_attr(ov, n)
            }
            ExprKind::Index(o, i) => {
                let ov = self.eval(o, vars, mode)?;
                let iv = self.eval(i, vars, mode)?;
                self.index(ov, iv)
            }
            ExprKind::Call(f, args) => self.eval_call(e, f, args, vec![], vars, mode),
            ExprKind::Unary(op, x) => {
                let v = self.eval(x, vars, mode)?;
                match (*op, v) {
                    ("-", Value::Int(i)) => Ok(Value::Int(-i)),
                    ("-", Value::Num(n)) => Ok(Value::Num(-n)),
                    ("-", Value::Angle(a)) => Ok(Value::Angle(-a)),
                    ("-", Value::Dur(r)) => Ok(Value::Dur(Rat::new(-r.n, r.d))),
                    ("-", Value::Tuple(t)) => Ok(Value::Tuple(
                        t.into_iter()
                            .map(|x| self.binop("*", x, Value::Int(-1)))
                            .collect::<Res<_>>()?,
                    )),
                    ("not", Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (o, v) => Err(type_err(format!("cannot apply `{o}` to {}", v.type_name()))),
                }
            }
            ExprKind::Binary(op, a, b) => {
                if *op == "and" || *op == "or" {
                    let av = self.eval(a, vars, mode)?;
                    let Value::Bool(ab) = av else {
                        return Err(type_err(format!(
                            "`{op}` needs Bool operands, found {}",
                            av.type_name()
                        )));
                    };
                    if (*op == "and" && !ab) || (*op == "or" && ab) {
                        return Ok(Value::Bool(ab));
                    }
                    let bv = self.eval(b, vars, mode)?;
                    return match bv {
                        Value::Bool(x) => Ok(Value::Bool(x)),
                        other => Err(type_err(format!(
                            "`{op}` needs Bool operands, found {}",
                            other.type_name()
                        ))),
                    };
                }
                let av = self.eval(a, vars, mode)?;
                if *op == "??" {
                    return if matches!(av, Value::None) {
                        self.eval(b, vars, mode)
                    } else {
                        Ok(av)
                    };
                }
                let bv = self.eval(b, vars, mode)?;
                self.binop(op, av, bv)
            }
            ExprKind::Tuple(items) => Ok(Value::Tuple(
                items
                    .iter()
                    .map(|x| self.eval(x, vars, mode))
                    .collect::<Res<_>>()?,
            )),
            ExprKind::List(items) => Ok(Value::List(
                items
                    .iter()
                    .map(|x| self.eval(x, vars, mode))
                    .collect::<Res<_>>()?,
            )),
            ExprKind::Range(a, b) => {
                let (av, bv) = (self.eval(a, vars, mode)?, self.eval(b, vars, mode)?);
                let (lo, hi) = (
                    self.int(&av, "a range bound")?,
                    self.int(&bv, "a range bound")?,
                );
                Ok(Value::List((lo..=hi).map(Value::Int).collect()))
            }
            ExprKind::Lambda { params, body } => Ok(Value::Lambda(Rc::new(LambdaData {
                params: params.clone(),
                body: Rc::clone(body),
                vars: vars.flatten(),
            }))),
        }
    }

    fn unit_value(&self, num: &str, unit: &str) -> Res<Value> {
        let rat = || {
            Rat::parse_decimal(num)
                .ok_or_else(|| type_err(format!("`{num}` cannot be used with a unit")))
        };
        let f = || {
            num.parse::<f64>()
                .map_err(|_| type_err(format!("bad number `{num}`")))
        };
        Ok(match unit {
            "seconds" | "second" | "s" => Value::Dur(rat()?),
            "ms" => Value::Dur(rat()?.mul(Rat::new(1, 1000))),
            "frames" | "frame" | "f" => Value::Frames(num.parse::<i64>().map_err(|_| {
                type_err("frame counts are whole numbers")
                    .help("`36f` is exact; fractions of a frame do not exist")
            })?),
            "degrees" | "degree" | "deg" => Value::Angle(f()? / 360.0),
            "turns" | "turn" => Value::Angle(f()?),
            "rad" | "radians" => Value::Angle(f()? / std::f64::consts::TAU),
            "px" => Value::Num(f()?),
            _ => {
                return Err(Diagnostic::new(
                    "E-LEX-004",
                    format!("unknown unit suffix `{unit}`"),
                ))
            }
        })
    }

    fn lookup(&mut self, n: &str, vars: &Vars) -> Res<Value> {
        if let Some(v) = vars.get(n) {
            return Ok(v.clone());
        }
        if let Some(v) = self.globals.get(n) {
            return Ok(v.clone());
        }
        if self.shapes.contains_key(n) {
            return Ok(Value::Shape(n.to_string()));
        }
        if self.entities.contains_key(n) {
            return Ok(Value::Entity(n.to_string()));
        }
        if n == "canvas" {
            return Ok(Value::Rect([
                0.0,
                0.0,
                self.canvas.0 as f64,
                self.canvas.1 as f64,
            ]));
        }
        if self.emitters.contains_key(n) {
            return Ok(Value::Emitter(n.to_string()));
        }
        if self.fluid_cfg.contains_key(n) {
            return Ok(Value::Fluid(n.to_string()));
        }
        if self.rigs.contains_key(n) {
            return Ok(Value::Armature(n.to_string()));
        }
        if let Some(e) = Ease::from_name(n) {
            return Ok(Value::Ease(e));
        }
        if n == "scene" {
            return Ok(Value::SceneObj);
        }
        if n == "none" {
            return Ok(Value::None);
        }
        // names that stand for themselves in arguments such as `axis: y` or `property: rotation`
        if matches!(
            n,
            "x" | "y"
                | "both"
                | "position"
                | "scale"
                | "rotation"
                | "opacity"
                | "static"
                | "dynamic"
                | "kinematic"
        ) {
            return Ok(Value::Str(n.to_string()));
        }
        let mut d = Diagnostic::new("E-NAM-001", format!("unknown name `{n}`"));
        if !self.shape_order.is_empty() {
            d = d.help(format!("shapes in scope: {}", self.shape_order.join(", ")));
        }
        Err(d)
    }

    fn elem_list(&self, sn: &str, kind: Kind, ids: &[u32]) -> Value {
        Value::List(
            ids.iter()
                .map(|i| Value::Elem(sn.to_string(), ElemRef { kind, id: *i }))
                .collect(),
        )
    }

    fn get_attr(&mut self, obj: Value, name: &str) -> Res<Value> {
        match obj {
            Value::Shape(sn) => {
                if let Some(v) = self.phys_attr(&sn, name) {
                    return v;
                }
                if let Some(v) = self.bone_attr(&sn, name) {
                    return Ok(v);
                }
                let live = self.live_tf.get(&sn).copied();
                let sh = &self.shapes[&sn];
                if let Some((k, id)) = parse_elem_name(name) {
                    return Ok(Value::Elem(sn.clone(), sh.get_ref(k, id)?));
                }
                if let Some(r) = sh.resolve_name(name) {
                    return Ok(Value::Elem(sn.clone(), r));
                }
                Ok(match name {
                    "v" => Value::Index(sn, Kind::V),
                    "e" => Value::Index(sn, Kind::E),
                    "f" => Value::Index(sn, Kind::F),
                    "vertices" => self.elem_list(&sn, Kind::V, &sh.live_ids(Kind::V)),
                    "edges" => self.elem_list(&sn, Kind::E, &sh.live_ids(Kind::E)),
                    "faces" => self.elem_list(&sn, Kind::F, &sh.live_ids(Kind::F)),
                    "name" => Value::Str(sn.clone()),
                    "z" => Value::Int(sh.z),
                    "visible" => Value::Bool(sh.visible),
                    "stroke_width" => Value::Num(sh.stroke_width),
                    "position" => {
                        let p = live.map_or(sh.position, |t| t.position);
                        Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1])])
                    }
                    "scale" => {
                        let p = live.map_or(sh.scale, |t| t.scale);
                        Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1])])
                    }
                    "rotation" => Value::Angle(live.map_or(sh.rotation, |t| t.rotation)),
                    "opacity" => Value::Num(sh.opacity),
                    "tint" => Value::Color(sh.tint),
                    "pivot" => {
                        let p = sh.transform().pivot;
                        Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1])])
                    }
                    "fill" => sh.fill.map(Value::Color).unwrap_or(Value::None),
                    "stroke" => sh.stroke.map(Value::Color).unwrap_or(Value::None),
                    _ => {
                        return Err(Diagnostic::new("E-NAM-003", format!("`{sn}` has no member `{name}`"))
                            .help("elements are written `v1`, `e2`, `f1` or indexed `v[n]`; semantic names are set with `names:` in `subdivide`"))
                    }
                })
            }
            Value::Elem(sn, r) => {
                let sh = &self.shapes[&sn];
                let meta = sh.meta(r).expect("allocated");
                let same = |ids: &[u32]| self.elem_list(&sn, r.kind, ids);
                match name {
                    "children" => return Ok(same(&meta.children)),
                    "parent" => {
                        return Ok(meta
                            .parent
                            .map(|p| Value::Elem(sn.clone(), p))
                            .unwrap_or(Value::None))
                    }
                    "created" => {
                        return Ok(Value::List(
                            meta.created
                                .iter()
                                .map(|c| Value::Elem(sn.clone(), *c))
                                .collect(),
                        ))
                    }
                    "alive" => return Ok(Value::Bool(meta.state == State::Live)),
                    "id" => return Ok(Value::Int(r.id as i64)),
                    "name" => return Ok(meta.alias.clone().map(Value::Str).unwrap_or(Value::None)),
                    _ => {}
                }
                sh.require_live(r, None, name)?;
                match (r.kind, name) {
                    (_, "x") => Ok(Value::Num(sh.move_anchor(r)[0])),
                    (_, "y") => Ok(Value::Num(sh.move_anchor(r)[1])),
                    (Kind::F, "fill") => Ok(sh.faces[&r.id].fill.map(Value::Color).unwrap_or(Value::None)),
                    (Kind::E, "stroke") => Ok(sh.edges[&r.id].stroke.map(Value::Color).unwrap_or(Value::None)),
                    (Kind::E, "stroke_width") => Ok(sh.edges[&r.id].width.map(Value::Num).unwrap_or(Value::None)),
                    (Kind::E, "length") => {
                        let ed = &sh.edges[&r.id];
                        let (a, b) = (sh.pos(ed.a), sh.pos(ed.b));
                        Ok(Value::Num((a[0] - b[0]).hypot(a[1] - b[1])))
                    }
                    (_, "position") => {
                        let p = sh.move_anchor(r);
                        Ok(Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1])]))
                    }
                    (Kind::E, "vertices") => {
                        let ed = &sh.edges[&r.id];
                        Ok(self.elem_list(&sn, Kind::V, &[ed.a, ed.b]))
                    }
                    (Kind::E, "curve") => Ok(Value::Str(
                        match sh.edges[&r.id].curve {
                            Curve::Straight => "straight",
                            Curve::Quad(_) => "quadratic",
                            Curve::Cubic(..) => "cubic",
                        }
                        .into(),
                    )),
                    (Kind::F, "vertices") => Ok(self.elem_list(&sn, Kind::V, &sh.face_vertex_ids(r.id))),
                    (Kind::F, "edges") => {
                        let ids: Vec<u32> = sh.faces[&r.id].loop_.iter().map(|x| x.0).collect();
                        Ok(self.elem_list(&sn, Kind::E, &ids))
                    }
                    (Kind::F, "area") => Ok(Value::Num(sh.face_area(r.id).abs())),
                    _ => Err(Diagnostic::new("E-NAM-003", format!("{} `{}` has no member `{name}`", r.kind.name(), sh.label(r)))
                        .help("members: children, parent, created, alive, name, plus x/y/position (vertex), vertices/curve (edge), vertices/edges/area (face)")),
                }
            }
            Value::Entity(en) => {
                let ent = &self.entities[&en];
                match ent.flags.get(name) {
                    Some(b) => Ok(Value::Bool(*b)),
                    None => Err(Diagnostic::new(
                        "E-NAM-003",
                        format!("`{en}` has no flag `{name}`"),
                    )
                    .help(format!("declared flags: {}", ent.states.join(", ")))),
                }
            }
            Value::SceneObj => match name {
                "starts" => Ok(Value::Bool(self.events.contains("scene.starts"))),
                _ => Err(
                    Diagnostic::new("E-NAM-003", format!("`scene` has no event `{name}`"))
                        .help("this build supports `scene.starts`"),
                ),
            },
            Value::Armature(arm) => self.armature_member(&arm, name),
            Value::Fluid(fname) if name == "count" => {
                Ok(Value::Int(self.fluid_count(&fname) as i64))
            }
            Value::Emitter(en) if name == "count" => Ok(Value::Int(self.emitter_count(&en) as i64)),
            Value::Tuple(t) if t.len() == 2 && (name == "x" || name == "y") => {
                Ok(t[usize::from(name == "y")].clone())
            }
            Value::List(items) => match name {
                "count" => Ok(Value::Int(items.len() as i64)),
                _ => Err(
                    Diagnostic::new("E-NAM-003", format!("a list has no member `{name}`"))
                        .help("lists have `count`; selection methods are called with parentheses"),
                ),
            },
            other => Err(type_err(format!("a {} has no members", other.type_name()))),
        }
    }

    fn index(&self, obj: Value, idx: Value) -> Res<Value> {
        match obj {
            Value::Index(sn, kind) => {
                let i = self.int(&idx, "an element index")?;
                let id = if i < 1 { 0 } else { i as u32 };
                Ok(Value::Elem(sn.clone(), self.shapes[&sn].get_ref(kind, id)?))
            }
            Value::List(items) | Value::Tuple(items) => {
                let i = self.int(&idx, "a list index")?;
                items
                    .get(usize::try_from(i).unwrap_or(usize::MAX))
                    .cloned()
                    .ok_or_else(|| {
                        Diagnostic::new(
                            "E-TYP-001",
                            format!("index {i} is out of range for a list of {}", items.len()),
                        )
                        .help("lists are indexed from 0")
                    })
            }
            other => Err(type_err(format!(
                "a {} cannot be indexed",
                other.type_name()
            ))),
        }
    }

    // ---- operators ---------------------------------------------------------------------

    fn mismatch(&self, verb: &str, a: &Value, b: &Value) -> Diagnostic {
        let mut d = Diagnostic::new(
            "E-TYP-002",
            format!("cannot {verb} `{}` and `{}`", a.type_name(), b.type_name()),
        );
        let bare = |v: &Value| matches!(v, Value::Int(_) | Value::Num(_));
        let unit = |v: &Value| matches!(v, Value::Dur(_) | Value::Frames(_) | Value::Angle(_));
        if (bare(a) && unit(b)) || (unit(a) && bare(b)) {
            d = d.help("a bare number has no unit; write `2.seconds` or `20.degrees`");
        }
        d
    }

    pub fn values_eq(&self, a: &Value, b: &Value) -> bool {
        use Value::*;
        match (a, b) {
            (None, None) => true,
            (Bool(x), Bool(y)) => x == y,
            (Int(x), Int(y)) => x == y,
            (Str(x), Str(y)) => x == y,
            (Angle(x), Angle(y)) => (x - y).abs() < 1e-12,
            (Color(x), Color(y)) => x == y,
            (Shape(x), Shape(y)) => x == y,
            (Elem(s1, r1), Elem(s2, r2)) => s1 == s2 && r1 == r2,
            (Entity(x), Entity(y)) => x == y,
            (List(x), List(y)) | (Tuple(x), Tuple(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(p, q)| self.values_eq(p, q))
            }
            _ => {
                if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
                    return x == y;
                }
                if let (Some(x), Some(y)) = (self.to_dur(a), self.to_dur(b)) {
                    return x == y;
                }
                false
            }
        }
    }

    pub fn binop(&self, op: &str, a: Value, b: Value) -> Res<Value> {
        use Value::*;
        if op == "==" || op == "!=" {
            return Ok(Bool(self.values_eq(&a, &b) == (op == "==")));
        }
        if let (Some(x), Some(y)) = (self.to_dur(&a), self.to_dur(&b)) {
            match op {
                "+" => return Ok(Dur(x.add(y))),
                "-" => return Ok(Dur(x.sub(y))),
                "/" => {
                    return x
                        .div(y)
                        .map(|r| Num(r.to_f64()))
                        .ok_or_else(|| Diagnostic::new("E-RUN-005", "division by zero"))
                }
                "<" | "<=" | ">" | ">=" => {
                    let o = x.cmp(y);
                    return Ok(Bool(match op {
                        "<" => o.is_lt(),
                        "<=" => o.is_le(),
                        ">" => o.is_gt(),
                        _ => o.is_ge(),
                    }));
                }
                _ => {}
            }
        }
        let verb = match op {
            "+" => "add",
            "-" => "subtract",
            "*" => "multiply",
            "/" => "divide",
            "%" => "take the remainder of",
            _ => "compare",
        };
        match (&a, &b) {
            (Int(x), Int(y)) => {
                return Ok(match op {
                    "+" => Int(x.wrapping_add(*y)),
                    "-" => Int(x.wrapping_sub(*y)),
                    "*" => Int(x.wrapping_mul(*y)),
                    "/" | "%" if *y == 0 => {
                        return Err(Diagnostic::new("E-RUN-005", "division by zero"))
                    }
                    "/" => Num(*x as f64 / *y as f64),
                    "%" => Int(x.rem_euclid(*y)),
                    "<" => Bool(x < y),
                    "<=" => Bool(x <= y),
                    ">" => Bool(x > y),
                    ">=" => Bool(x >= y),
                    _ => return Err(self.mismatch(verb, &a, &b)),
                })
            }
            (Str(x), Str(y)) if op == "+" => return Ok(Str(format!("{x}{y}"))),
            _ => {}
        }
        if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
            return Ok(match op {
                "+" => Num(x + y),
                "-" => Num(x - y),
                "*" => Num(x * y),
                "/" if y == 0.0 => return Err(Diagnostic::new("E-RUN-005", "division by zero")),
                "/" => Num(x / y),
                "%" if y == 0.0 => return Err(Diagnostic::new("E-RUN-005", "division by zero")),
                "%" => Num(x.rem_euclid(y)),
                "<" => Bool(x < y),
                "<=" => Bool(x <= y),
                ">" => Bool(x > y),
                ">=" => Bool(x >= y),
                _ => return Err(self.mismatch(verb, &a, &b)),
            });
        }
        match (&a, &b) {
            (Angle(x), Angle(y)) => match op {
                "+" => Ok(Angle(x + y)),
                "-" => Ok(Angle(x - y)),
                "<" => Ok(Bool(x < y)),
                "<=" => Ok(Bool(x <= y)),
                ">" => Ok(Bool(x > y)),
                ">=" => Ok(Bool(x >= y)),
                _ => Err(self.mismatch(verb, &a, &b)),
            },
            (Angle(x), n) | (n, Angle(x))
                if n.as_f64().is_some() && (op == "*" || (op == "/" && matches!(a, Angle(_)))) =>
            {
                let k = n.as_f64().unwrap();
                Ok(Angle(if op == "*" { x * k } else { x / k }))
            }
            (d, n) | (n, d)
                if self.to_dur(d).is_some()
                    && n.as_f64().is_some()
                    && (op == "*" || (op == "/" && self.to_dur(&a).is_some())) =>
            {
                let k = Rat::from_f64(n.as_f64().unwrap())
                    .ok_or_else(|| type_err("cannot scale a duration by that number"))?;
                let x = self.to_dur(d).unwrap();
                if op == "*" {
                    Ok(Dur(x.mul(k)))
                } else {
                    x.div(k)
                        .map(Dur)
                        .ok_or_else(|| Diagnostic::new("E-RUN-005", "division by zero"))
                }
            }
            (Tuple(x), Tuple(y)) if (op == "+" || op == "-") && x.len() == y.len() => Ok(Tuple(
                x.iter()
                    .zip(y)
                    .map(|(p, q)| self.binop(op, p.clone(), q.clone()))
                    .collect::<Res<_>>()?,
            )),
            (Tuple(x), n) if n.as_f64().is_some() && (op == "*" || op == "/") => Ok(Tuple(
                x.iter()
                    .map(|p| self.binop(op, p.clone(), n.clone()))
                    .collect::<Res<_>>()?,
            )),
            (n, Tuple(x)) if n.as_f64().is_some() && op == "*" => Ok(Tuple(
                x.iter()
                    .map(|p| self.binop(op, n.clone(), p.clone()))
                    .collect::<Res<_>>()?,
            )),
            _ => Err(self.mismatch(verb, &a, &b)),
        }
    }

    // ---- calls -------------------------------------------------------------------------

    pub fn eval_call(
        &mut self,
        call: &Expr,
        f: &Expr,
        args: &[Arg],
        extra: Vec<(Option<String>, Value)>,
        vars: &Vars,
        mode: Mode,
    ) -> Res<Value> {
        if let ExprKind::Member(o, name) = &f.kind {
            if let Some(r) = self.flag_call(o, name, args, vars, mode) {
                return r;
            }
        }
        let mut vals: Vec<(Option<String>, Value)> = vec![];
        let callee = match &f.kind {
            ExprKind::Member(o, name) => Some((self.eval(o, vars, mode)?, name.clone())),
            _ => None,
        };
        for a in args {
            vals.push((a.name.clone(), self.eval(&a.value, vars, mode)?));
        }
        vals.extend(extra);
        match (&f.kind, callee) {
            (_, Some((obj, name))) => self.call_method(obj, &name, vals, call.span, mode),
            (ExprKind::Name(n), None) => self.call_function(n, vals, mode),
            _ => Err(type_err("this expression cannot be called")),
        }
    }

    fn call_function(
        &mut self,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        _mode: Mode,
    ) -> Res<Value> {
        if let Some(def) = self.defs.get(name).cloned() {
            return self.call_def(&def, args, None, _mode);
        }
        let at_default = |m: &HashMap<&'static str, Value>, s: &Interp| -> Res<[f64; 2]> {
            m.get("at")
                .map(|v| s.point(v, "`at`"))
                .unwrap_or(Ok([0.0, 0.0]))
        };
        if Interp::is_joint_function(name, &args) {
            return self.joint_function(name, args, _mode);
        }
        match name {
            "image" => {
                let m = self.bind(name, args, &["path"], &["path"])?;
                let Value::Str(path) = &m["path"] else { return Err(type_err("`image` needs the file name as a string, such as `image(\"hero.png\")`")) };
                self.load_image(path)
            }
            "grid" => {
                let m = self.bind(name, args, &["cells", "width", "height", "at"], &["cells"])?;
                let n = self.int(&m["cells"], "`cells`")?;
                let w = m.get("width").map(|v| self.num(v, "`width`")).unwrap_or(Ok(100.0))?;
                let h = m.get("height").map(|v| self.num(v, "`height`")).unwrap_or(Ok(w))?;
                if !(1..=100).contains(&n) || w <= 0.0 || h <= 0.0 {
                    return Err(type_err("`grid` needs 1 to 100 cells and a positive width and height"));
                }
                let mut sh = rectangle(w, h, at_default(&m, self)?);
                if n > 1 {
                    let f = sh.get_ref(Kind::F, 1)?;
                    sh.subdivide(f, n as usize, &[])?;
                }
                Ok(Value::Fresh(Box::new(sh)))
            }
            "square" => {
                let m = self.bind(name, args, &["size", "at"], &[])?;
                let size = m.get("size").map(|v| self.num(v, "`size`")).unwrap_or(Ok(100.0))?;
                Ok(Value::Fresh(Box::new(square(size, at_default(&m, self)?))))
            }
            "rectangle" => {
                let m = self.bind(name, args, &["width", "height", "at"], &["width", "height"])?;
                Ok(Value::Fresh(Box::new(rectangle(self.num(&m["width"], "`width`")?, self.num(&m["height"], "`height`")?, at_default(&m, self)?))))
            }
            "triangle" => {
                let m = self.bind(name, args, &["size", "at"], &[])?;
                let size = m.get("size").map(|v| self.num(v, "`size`")).unwrap_or(Ok(100.0))?;
                Ok(Value::Fresh(Box::new(triangle(size, at_default(&m, self)?))))
            }
            "circle" => {
                let m = self.bind(name, args, &["n", "radius", "at"], &["n"])?;
                let n = self.int(&m["n"], "`n`")?.max(0) as usize;
                let r = m.get("radius").map(|v| self.num(v, "`radius`")).unwrap_or(Ok(50.0))?;
                Ok(Value::Fresh(Box::new(circle(n, r, at_default(&m, self)?)?)))
            }
            "polygon" => {
                let m = self.bind(name, args, &["points"], &["points"])?;
                let Value::List(items) = &m["points"] else { return Err(type_err("`points` must be a list of points")) };
                let pts: Vec<[f64; 2]> = items.iter().map(|p| self.point(p, "a polygon point")).collect::<Res<_>>()?;
                Ok(Value::Fresh(Box::new(polygon(&pts)?)))
            }
            "line" => {
                let m = self.bind(name, args, &["a", "b"], &["a", "b"])?;
                Ok(Value::Fresh(Box::new(line(self.point(&m["a"], "`a`")?, self.point(&m["b"], "`b`")?))))
            }
            "distance" => {
                let m = self.bind(name, args, &["a", "b"], &["a", "b"])?;
                let (a, b) = (self.point(&m["a"], "`a`")?, self.point(&m["b"], "`b`")?);
                Ok(Value::Num(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()))
            }
            "raycast" | "raycast_normal" => self.raycast_builtin(name, args),
            "atan2" => {
                let m = self.bind(name, args, &["y", "x"], &["y", "x"])?;
                let (y, x) = (self.num(&m["y"], "`y`")?, self.num(&m["x"], "`x`")?);
                Ok(Value::Angle(y.atan2(x) / std::f64::consts::TAU))
            }
            "direction_to" => {
                let m = self.bind(name, args, &["a", "b"], &["a", "b"])?;
                let (a, b) = (self.point(&m["a"], "`a`")?, self.point(&m["b"], "`b`")?);
                Ok(Value::Angle((b[1] - a[1]).atan2(b[0] - a[0]) / std::f64::consts::TAU))
            }
            "unit_vector" => {
                let m = self.bind(name, args, &["angle"], &["angle"])?;
                let t = self.angle(&m["angle"], "`angle`")? * std::f64::consts::TAU;
                Ok(Value::Tuple(vec![Value::Num(t.cos()), Value::Num(t.sin())]))
            }
            "center" => {
                let m = self.bind(name, args, &["of"], &["of"])?;
                match &m["of"] {
                    Value::Shape(sn) => {
                        let sh = &self.shapes[sn];
                        let t = self.live_tf.get(sn).copied().unwrap_or_else(|| sh.transform());
                        let c = t.apply(sh.bbox_center());
                        Ok(Value::Tuple(vec![Value::Num(c[0]), Value::Num(c[1])]))
                    }
                    Value::Elem(sn, r) => {
                        let c = self.shapes[sn].move_anchor(*r);
                        Ok(Value::Tuple(vec![Value::Num(c[0]), Value::Num(c[1])]))
                    }
                    other => Err(type_err(format!("`center` needs a shape or element, found {}", other.type_name()))),
                }
            }
            "min" | "max" => {
                let m = self.bind(name, args, &["a", "b"], &["a", "b"])?;
                let less = self.binop("<", m["a"].clone(), m["b"].clone())?;
                let Value::Bool(l) = less else { unreachable!() };
                Ok(if (name == "min") == l { m["a"].clone() } else { m["b"].clone() })
            }
            "abs" | "floor" | "ceil" | "round" | "sqrt" | "sin" | "cos" => {
                let m = self.bind(name, args, &["x"], &["x"])?;
                if let (Value::Int(i), "abs") = (&m["x"], name) {
                    return Ok(Value::Int(i.abs()));
                }
                let x = match (&m["x"], name) {
                    (Value::Angle(t), "sin" | "cos") => t * std::f64::consts::TAU,
                    (v, _) => self.num(v, "the argument")?,
                };
                Ok(match name {
                    "abs" => Value::Num(x.abs()),
                    "floor" => Value::Int(x.floor() as i64),
                    "ceil" => Value::Int(x.ceil() as i64),
                    "round" => Value::Int(x.round_ties_even() as i64),
                    "sqrt" => Value::Num(x.sqrt()),
                    "sin" => Value::Num(x.sin()),
                    _ => Value::Num(x.cos()),
                })
            }
            "clamp" => {
                let m = self.bind(name, args, &["x", "lo", "hi"], &["x", "lo", "hi"])?;
                let (x, lo, hi) = (self.num(&m["x"], "`x`")?, self.num(&m["lo"], "`lo`")?, self.num(&m["hi"], "`hi`")?);
                Ok(Value::Num(x.max(lo).min(hi)))
            }
            "lerp" => {
                let m = self.bind(name, args, &["a", "b", "t"], &["a", "b", "t"])?;
                let (a, b, t) = (self.num(&m["a"], "`a`")?, self.num(&m["b"], "`b`")?, self.num(&m["t"], "`t`")?);
                Ok(Value::Num(a + (b - a) * t))
            }
            "frames" => {
                let m = self.bind(name, args, &["d"], &["d"])?;
                let n = self.frames_of(&m["d"].clone(), Span::new(1, 1, 1))?;
                Ok(Value::Int(n))
            }
            "spring" => {
                let m = self.bind(name, args, &["bounces", "damping"], &[])?;
                let bounces = m.get("bounces").map(|v| self.num(v, "`bounces`")).unwrap_or(Ok(3.0))?;
                let damping = m.get("damping").map(|v| self.num(v, "`damping`")).unwrap_or(Ok(5.0))?;
                let end = 1.0 - (-damping).exp() * (std::f64::consts::TAU * bounces).cos();
                if !(damping > 0.0 && bounces >= 0.0 && end > 0.2) {
                    return Err(type_err("a spring needs damping above 0, bounces of 0 or more, and a settling value that is not too small")
                        .help("try `spring(3, 5)`: three bounces, damping 5"));
                }
                Ok(Value::Ease(Ease::Spring { bounces, damping }))
            }
            "marker" => {
                let m = self.bind(name, args, &["name"], &["name"])?;
                let Value::Str(n) = &m["name"] else {
                    return Err(type_err("`marker` needs the marker's name as a string"));
                };
                Ok(Value::Bool(self.events.contains(&format!("marker:{n}"))))
            }
            "cubic_bezier" => {
                let m = self.bind(name, args, &["x1", "y1", "x2", "y2"], &["x1", "y1", "x2", "y2"])?;
                let n = |k: &str| self.num(&m[k], "a `cubic_bezier` value");
                let v = [n("x1")?, n("y1")?, n("x2")?, n("y2")?];
                if !(0.0..=1.0).contains(&v[0]) || !(0.0..=1.0).contains(&v[2]) {
                    return Err(type_err("the x values of `cubic_bezier` must be between 0 and 1")
                        .help("y values may go outside 0..1 to overshoot; try `cubic_bezier(0.25, 0.1, 0.25, 1)`"));
                }
                Ok(Value::Ease(Ease::Bezier(v)))
            }
            "rect" => {
                let m = self.bind(name, args, &["x", "y", "w", "h"], &["x", "y", "w", "h"])?;
                let n = |k: &str| self.num(&m[k], "a rectangle value");
                let (x, y, w, h) = (n("x")?, n("y")?, n("w")?, n("h")?);
                if !(w > 0.0 && h > 0.0) {
                    return Err(type_err("a rectangle needs a positive width and height"));
                }
                Ok(Value::Rect([x, y, w, h]))
            }
            "noise" => {
                let m = self.bind(name, args, &["seed_name", "t"], &["seed_name", "t"])?;
                let Value::Str(s) = &m["seed_name"] else { return Err(type_err("`noise` needs the stream name as a string")) };
                let t = self.num(&m["t"], "`t`")?;
                Ok(Value::Num(value_noise(stream_seed(self.seed, s), t)))
            }
            "random" => {
                let m = self.bind(name, args, &["seed_name", "index"], &["seed_name"])?;
                let Value::Str(s) = &m["seed_name"] else { return Err(type_err("`random` needs the stream name as a string")) };
                let i = m.get("index").map(|v| self.int(v, "`index`")).unwrap_or(Ok(0))?;
                Ok(Value::Num(hash01(stream_seed(self.seed, s), i)))
            }
            "len" => {
                let m = self.bind(name, args, &["of"], &["of"])?;
                match &m["of"] {
                    Value::List(l) | Value::Tuple(l) => Ok(Value::Int(l.len() as i64)),
                    Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
                    other => Err(type_err(format!("`len` needs a list, found {}", other.type_name()))),
                }
            }
            _ => Err(Diagnostic::new("E-NAM-001", format!("unknown function `{name}`"))
                .help("shapes: square, rectangle, triangle, circle, polygon, line; maths: distance, direction_to, unit_vector, center, min, max, abs, clamp, lerp, floor, ceil, round, sqrt, sin, cos, frames, len")),
        }
    }

    pub(crate) fn over_frames(
        &mut self,
        m: &HashMap<&'static str, Value>,
        mode: Mode,
        span: Span,
    ) -> Res<Option<(i64, Ease)>> {
        let Some(o) = m.get("over") else {
            if m.contains_key("ease") {
                return Err(Diagnostic::new("E-TYP-003", "`ease` needs `over`")
                    .help("add a duration: `... over 0.3.seconds ease ease_out`"));
            }
            return Ok(None);
        };
        if mode != Mode::Run {
            return Err(Diagnostic::new("E-SHP-005", "`over` needs a running scene")
                .help("build blocks and `setup:` apply edits instantly; remove `over`, or move this into `main:` or a rule"));
        }
        let n = self.frames_of(&o.clone(), span)?;
        let ease = match m.get("ease") {
            Some(Value::Ease(e)) => *e,
            Some(other) => {
                return Err(type_err(format!(
                    "`ease` must be an easing such as `ease_out`, found {}",
                    other.type_name()
                )))
            }
            None => Ease::InOut,
        };
        Ok(if n == 0 { None } else { Some((n, ease)) })
    }

    fn call_method(
        &mut self,
        obj: Value,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        if let Some(def) = self.defs.get(name).cloned() {
            if def.params.first().map(|p| p.name.as_str()) == Some("self") {
                return self.call_def(&def, args, Some(obj), mode);
            }
        }
        if TOPOLOGY_METHODS.contains(&name) {
            self.guard_topology(mode, name)?;
        }
        if let (Value::Shape(sn), true) = (&obj, Interp::is_body_method(name)) {
            let sn = sn.clone();
            return self.body_method(&sn, name, args, mode);
        }
        if let (Value::Shape(sn), true) = (&obj, Interp::is_soft_method(name)) {
            let sn = sn.clone();
            return self.soft_method(&sn, name, args, mode);
        }
        if let (Value::Shape(sn), true) = (&obj, Interp::is_rig_method(name)) {
            let sn = sn.clone();
            return self.rig_method(&sn, name, args, mode, span);
        }
        if let (Value::Armature(arm), "ragdoll" | "animate") = (&obj, name) {
            let arm = arm.clone();
            return self.armature_method(&arm, name, args, mode);
        }
        if let Value::Fluid(fname) = &obj {
            let fname = fname.clone();
            return self.fluid_method(&fname, name, args);
        }
        if let Value::Emitter(en) = &obj {
            let en = en.clone();
            return self.emitter_method(&en, name, args);
        }
        if let Value::Joint(j) = obj {
            return self.joint_method(j, name, args, mode);
        }
        match name {
            "wobble" | "noise" => return self.modifier_method(obj, name, args, span, mode),
            "fall" | "bounce_in" => return self.physics_method(obj, name, args, span, mode),
            "fill_to" | "stroke_to" | "width_to" | "fade_to" | "tint_to" => {
                return self.colour_method(obj, name, args, span, mode)
            }
            _ => {}
        }
        match obj {
            Value::Shape(sn) => self.shape_method(&sn, name, args, span, mode),
            Value::Elem(sn, r) => self.elem_method(&sn, r, name, args, span, mode),
            Value::List(items) => self.list_method(items, name, args, span, mode),
            other => Err(Diagnostic::new(
                "E-NAM-003",
                format!("a {} has no operation `{name}`", other.type_name()),
            )),
        }
    }

    fn elem_ref(&self, v: &Value, sn: &str, kind: Kind, what: &str) -> Res<ElemRef> {
        match v {
            Value::Elem(s, r) if s == sn && r.kind == kind => Ok(*r),
            Value::Elem(s, r) if s != sn => Err(type_err(format!(
                "`{}` belongs to another shape, not `{sn}`",
                self.shapes[s].label(*r)
            ))),
            other => Err(type_err(format!(
                "{what} must be a {}, found {}",
                kind.name(),
                other.type_name()
            ))),
        }
    }

    fn shape_method(
        &mut self,
        sn: &str,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        match name {
            "smooth" => {
                let m = self.bind(name, args, &["levels"], &[])?;
                let n = m.get("levels").map(|v| self.int(v, "`levels`")).unwrap_or(Ok(1))?;
                self.with_shape(sn, mode, span, |sh| sh.smooth(n.max(0) as u32))?;
                Ok(Value::None)
            }
            "merge" => {
                let m = self.bind(name, args, &["a", "b", "at"], &["a", "b"])?;
                let (a, b) = (self.elem_ref(&m["a"], sn, Kind::V, "`a`")?, self.elem_ref(&m["b"], sn, Kind::V, "`b`")?);
                let at = self.merge_at(m.get("at"))?;
                self.with_shape(sn, mode, span, |sh| sh.merge(a, b, at))?;
                Ok(Value::None)
            }
            "delete" => {
                let m = self.bind(name, args, &["element"], &["element"])?;
                let Value::Elem(s, r) = &m["element"] else { return Err(type_err("`delete` needs a vertex, edge or face")) };
                if s != sn {
                    return Err(type_err(format!("`{}` belongs to another shape", self.shapes[s].label(*r))));
                }
                let r = *r;
                self.with_shape(sn, mode, span, |sh| sh.delete(r))?;
                Ok(Value::None)
            }
            "extrude" => {
                let m = self.bind(name, args, &["edge", "by"], &["edge", "by"])?;
                let r = self.elem_ref(&m["edge"], sn, Kind::E, "`edge`")?;
                let by = self.num(&m["by"], "`by`")?;
                self.with_shape(sn, mode, span, |sh| sh.extrude(r, by))?;
                Ok(Value::None)
            }
            "copy" => {
                self.bind(name, args, &[], &[])?;
                Ok(Value::Fresh(Box::new(self.shapes[sn].clone())))
            }
            "morph_to" => {
                let m = self.bind(name, args, &["other", "over", "ease"], &["other"])?;
                let Value::Shape(other) = &m["other"] else { return Err(type_err("`morph_to` needs a shape")) };
                self.shapes[sn].check_morph(&self.shapes[other])?;
                let mine = self.shapes[sn].live_ids(Kind::V);
                let theirs = self.shapes[other].live_ids(Kind::V);
                let targets: Vec<(u32, [f64; 2])> = mine.iter().zip(&theirs).map(|(a, b)| (*a, self.shapes[other].pos(*b))).collect();
                match self.over_frames(&m, mode, span)? {
                    Some((frames, ease)) => Ok(Value::Pending(Box::new(ActionReq::vertices(
                        sn.to_string(),
                        format!("{sn}.morph_to({other})"),
                        frames,
                        ease,
                        targets.into_iter().map(|(id, p)| (id, Target::To(p))).collect(),
                    )))),
                    None => {
                        for (id, p) in targets {
                            self.apply_or_stage(Staged::Move { shape: sn.to_string(), r: v(id), x: p[0], y: p[1], by: false }, mode)?;
                        }
                        Ok(Value::None)
                    }
                }
            }
            "move_to" | "move_by" => {
                let m = self.bind(name, args, &["x", "y", "over", "ease"], &["x"])?;
                let p = self.xy(&m)?;
                let t = |v: f64| if name == "move_by" { Tgt::By(v) } else { Tgt::To(v) };
                self.shape_transform(sn, name, vec![(PropId::PosX, t(p[0])), (PropId::PosY, t(p[1]))], &m, mode, span)
            }
            "scale_to" => {
                let m = self.bind(name, args, &["factor", "over", "ease"], &["factor"])?;
                let f = match &m["factor"] {
                    Value::Tuple(_) => self.point(&m["factor"], "`factor`")?,
                    other => {
                        let n = self.num(other, "`factor`")?;
                        [n, n]
                    }
                };
                self.shape_transform(sn, name, vec![(PropId::ScaleX, Tgt::To(f[0])), (PropId::ScaleY, Tgt::To(f[1]))], &m, mode, span)
            }
            "rotate_to" | "rotate_by" => {
                let m = self.bind(name, args, &["angle", "over", "ease"], &["angle"])?;
                let a = self.angle(&m["angle"], "`angle`")?;
                let t = if name == "rotate_by" { Tgt::By(a) } else { Tgt::To(a) };
                self.shape_transform(sn, name, vec![(PropId::Rot, t)], &m, mode, span)
            }
            "radial_array" => {
                let m = self.bind(name, args, &["count", "around", "angle", "faces", "weld"], &["count", "around"])?;
                let count = self.count_arg(&m["count"], "`count`")?;
                let around = self.point(&m["around"], "`around`")?;
                let angle = m.get("angle").map(|v| self.angle(v, "`angle`")).unwrap_or(Ok(1.0))?;
                let faces: Option<Vec<u32>> = match m.get("faces") {
                    None => None,
                    Some(Value::List(items)) => Some(items.iter().map(|x| self.elem_ref(x, sn, Kind::F, "`faces`").map(|r| r.id)).collect::<Res<_>>()?),
                    Some(other) => return Err(type_err(format!("`faces` must be a list of faces, found {}", other.type_name()))),
                };
                let weld = m.get("weld").map(|v| self.num(v, "`weld`")).unwrap_or(Ok(1e-6))?;
                self.with_shape(sn, mode, span, |sh| sh.radial_array(count, around, angle, faces.as_deref(), weld))?;
                Ok(Value::None)
            }
            "flip" => {
                let m = self.bind(name, args, &["x", "y", "line"], &[])?;
                let axis = self.axis_of(&m)?;
                self.with_shape(sn, mode, span, |sh| sh.flip(axis))?;
                Ok(Value::None)
            }
            "mirror" => {
                let m = self.bind(name, args, &["x", "y", "line", "faces", "weld"], &[])?;
                let axis = self.axis_of(&m)?;
                let faces: Option<Vec<u32>> = match m.get("faces") {
                    None => None,
                    Some(Value::List(items)) => Some(items.iter().map(|x| self.elem_ref(x, sn, Kind::F, "`faces`").map(|r| r.id)).collect::<Res<_>>()?),
                    Some(other) => return Err(type_err(format!("`faces` must be a list of faces, found {}", other.type_name())).help("for example `faces: s.faces.where(f -> f.x > 0)`")),
                };
                let weld = m.get("weld").map(|v| self.num(v, "`weld`")).unwrap_or(Ok(1e-6))?;
                self.with_shape(sn, mode, span, |sh| sh.mirror(axis, faces.as_deref(), weld))?;
                Ok(Value::None)
            }
            _ => Err(Diagnostic::new("E-NAM-003", format!("shape `{sn}` has no operation `{name}`"))
                .help("shape operations: smooth, merge, delete, extrude, morph_to, copy, mirror, flip; element operations are called on `v1`, `e1`, `f1`; selections on `s.vertices` etc.")),
        }
    }

    fn merge_at(&self, v: Option<&Value>) -> Res<MergeAt> {
        match v {
            None => Ok(MergeAt::Center),
            Some(Value::Str(s)) => match s.as_str() {
                "center" => Ok(MergeAt::Center),
                "first" => Ok(MergeAt::First),
                "last" => Ok(MergeAt::Last),
                _ => Err(type_err("`at` must be \"center\", \"first\" or \"last\"")),
            },
            Some(o) => Err(type_err(format!(
                "`at` must be a string, found {}",
                o.type_name()
            ))),
        }
    }

    fn xy(&self, m: &HashMap<&'static str, Value>) -> Res<[f64; 2]> {
        match (m.get("x"), m.get("y")) {
            (Some(p @ Value::Tuple(_)), None) => self.point(p, "the target"),
            (Some(x), Some(y)) => Ok([self.num(x, "`x`")?, self.num(y, "`y`")?]),
            _ => Err(
                Diagnostic::new("E-TYP-003", "this operation needs two numbers or one point")
                    .help("write `move_to(10, 20)` or `move_to((10, 20))`"),
            ),
        }
    }

    fn elem_method(
        &mut self,
        sn: &str,
        r: ElemRef,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        match name {
            "move_to" | "move_by" => {
                let m = self.bind(name, args, &["x", "y", "over", "ease"], &["x"])?;
                let p = self.xy(&m)?;
                let by = name == "move_by";
                let sh = &self.shapes[sn];
                sh.require_live(r, None, name)?;
                match self.over_frames(&m, mode, span)? {
                    Some((frames, ease)) => {
                        let sh = &self.shapes[sn];
                        let delta = if by { p } else { let a = sh.move_anchor(r); [p[0] - a[0], p[1] - a[1]] };
                        let items = sh
                            .element_vertices(r)
                            .into_iter()
                            .map(|id| (id, if by { Target::By(p) } else { let q = sh.pos(id); Target::To([q[0] + delta[0], q[1] + delta[1]]) }))
                            .collect();
                        Ok(Value::Pending(Box::new(ActionReq::vertices(sn.to_string(), format!("{}.{name}", sh.label(r)), frames, ease, items))))
                    }
                    None => {
                        if mode == Mode::Run {
                            self.staged.push(Staged::Move { shape: sn.to_string(), r, x: p[0], y: p[1], by });
                        } else {
                            self.with_shape(sn, mode, span, |sh| if by { sh.move_by(r, p[0], p[1]) } else { sh.move_to(r, p[0], p[1]) })?;
                        }
                        Ok(Value::None)
                    }
                }
            }
            "subdivide" => {
                let m = self.bind(name, args, &["into", "names"], &["into"])?;
                let n = self.int(&m["into"], "`into`")?;
                let names: Vec<String> = match m.get("names") {
                    None => vec![],
                    Some(Value::List(l)) => l
                        .iter()
                        .map(|x| match x {
                            Value::Str(s) => Ok(s.clone()),
                            o => Err(type_err(format!("names must be strings, found {}", o.type_name()))),
                        })
                        .collect::<Res<_>>()?,
                    Some(o) => return Err(type_err(format!("`names` must be a list of strings, found {}", o.type_name()))),
                };
                self.with_shape(sn, mode, span, |sh| sh.subdivide(r, n.max(0) as usize, &names))?;
                Ok(Value::None)
            }
            "extrude" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let by = self.num(&m["by"], "`by`")?;
                self.with_shape(sn, mode, span, |sh| sh.extrude(r, by))?;
                Ok(Value::None)
            }
            "bevel" | "round" => {
                let m = self.bind(name, args, &["radius"], &["radius"])?;
                let rad = self.num(&m["radius"], "`radius`")?;
                self.with_shape(sn, mode, span, |sh| if name == "bevel" { sh.bevel(r, rad) } else { sh.round(r, rad) })?;
                Ok(Value::None)
            }
            "merge" => {
                let m = self.bind(name, args, &["other", "at"], &["other"])?;
                let other = self.elem_ref(&m["other"], sn, Kind::V, "`other`")?;
                let at = self.merge_at(m.get("at"))?;
                self.with_shape(sn, mode, span, |sh| sh.merge(r, other, at))?;
                Ok(Value::None)
            }
            "dissolve" => {
                self.bind(name, args, &[], &[])?;
                self.with_shape(sn, mode, span, |sh| sh.dissolve(r))?;
                Ok(Value::None)
            }
            "inset" => {
                let m = self.bind(name, args, &["amount"], &["amount"])?;
                let amount = self.num(&m["amount"], "`amount`")?;
                self.with_shape(sn, mode, span, |sh| sh.inset(r, amount))?;
                Ok(Value::None)
            }
            "delete" => {
                self.bind(name, args, &[], &[])?;
                self.with_shape(sn, mode, span, |sh| sh.delete(r))?;
                Ok(Value::None)
            }
            "bend" => {
                let m = self.bind(name, args, &["angle"], &["angle"])?;
                let a = self.angle(&m["angle"], "`bend`")?;
                self.with_shape(sn, mode, span, |sh| sh.bend(r, a))?;
                Ok(Value::None)
            }
            "set_handle" => {
                let m = self.bind(name, args, &["x", "y"], &["x"])?;
                let p = self.xy(&m)?;
                self.with_shape(sn, mode, span, |sh| sh.set_handle(r, p[0], p[1]))?;
                Ok(Value::None)
            }
            "set_handles" => {
                let m = self.bind(name, args, &["h1", "h2"], &["h1", "h2"])?;
                let (a, b) = (self.point(&m["h1"], "`h1`")?, self.point(&m["h2"], "`h2`")?);
                self.with_shape(sn, mode, span, |sh| sh.set_handles(r, a, b))?;
                Ok(Value::None)
            }
            "straighten" => {
                self.bind(name, args, &[], &[])?;
                self.with_shape(sn, mode, span, |sh| sh.straighten(r))?;
                Ok(Value::None)
            }
            _ => Err(Diagnostic::new("E-NAM-003", format!("{} `{}` has no operation `{name}`", r.kind.name(), self.shapes[sn].label(r)))
                .help("element operations: move_to, move_by, subdivide, extrude, bevel, round, merge, delete, dissolve, inset, bend, set_handle, set_handles, straighten")),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Selections (lists of elements), lambdas and mirror axes
// ---------------------------------------------------------------------------------------------

/// Element operations that apply to every element of a selection, one after the other.
const BROADCAST: &[&str] = &[
    "subdivide",
    "extrude",
    "bevel",
    "round",
    "dissolve",
    "inset",
    "delete",
    "bend",
    "set_handle",
    "set_handles",
    "straighten",
];

impl Interp {
    fn call_lambda(&mut self, l: &LambdaData, args: Vec<Value>, mode: Mode) -> Res<Value> {
        let mut vars = l.vars.clone();
        vars.push();
        for (p, a) in l.params.iter().zip(args) {
            vars.define(p, a);
        }
        self.eval(&l.body, &vars, mode)
    }

    fn count_arg(&self, v: &Value, what: &str) -> Res<usize> {
        let n = self.int(v, what)?;
        usize::try_from(n).map_err(|_| type_err(format!("{what} cannot be negative")))
    }

    fn axis_of(&self, m: &HashMap<&'static str, Value>) -> Res<Axis> {
        let given: Vec<&str> = ["x", "y", "line"]
            .into_iter()
            .filter(|k| m.contains_key(k))
            .collect();
        if given.len() != 1 {
            return Err(Diagnostic::new("E-TYP-003", "give exactly one mirror line: `x:`, `y:` or `line:`")
                .help("`x: 400` is the vertical line x = 400, `y: 300` the horizontal line y = 300, `line: ((0, 0), (1, 1))` any line through two points"));
        }
        Ok(match given[0] {
            "x" => Axis::X(self.num(&m["x"], "`x`")?),
            "y" => Axis::Y(self.num(&m["y"], "`y`")?),
            _ => match &m["line"] {
                Value::Tuple(t) if t.len() == 2 => Axis::Line(
                    self.point(&t[0], "the first point of `line`")?,
                    self.point(&t[1], "the second point of `line`")?,
                ),
                other => {
                    return Err(type_err(format!(
                        "`line` must be two points like `((0, 0), (1, 1))`, found {}",
                        other.type_name()
                    )))
                }
            },
        })
    }

    fn list_method(
        &mut self,
        items: Vec<Value>,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        match name {
            "every" => {
                let m = self.bind(name, args, &["n", "offset"], &["n"])?;
                let n = self.count_arg(&m["n"], "`n`")?;
                if n == 0 {
                    return Err(type_err("`every` needs a step of at least 1").help("`every(2)` picks the 1st, 3rd, 5th ... element; `every(2, offset: 1)` the 2nd, 4th ..."));
                }
                let offset = m.get("offset").map(|v| self.count_arg(v, "`offset`")).unwrap_or(Ok(0))?;
                Ok(Value::List(items.into_iter().enumerate().filter(|(i, _)| *i >= offset && (*i - offset) % n == 0).map(|(_, v)| v).collect()))
            }
            "first" | "last" | "skip" => {
                let m = self.bind(name, args, &["n"], &["n"])?;
                let n = self.count_arg(&m["n"], "`n`")?.min(items.len());
                let len = items.len();
                Ok(Value::List(match name {
                    "first" => items.into_iter().take(n).collect(),
                    "last" => items.into_iter().skip(len - n).collect(),
                    _ => items.into_iter().skip(n).collect(),
                }))
            }
            "reverse" => {
                self.bind(name, args, &[], &[])?;
                Ok(Value::List(items.into_iter().rev().collect()))
            }
            "where" => {
                let m = self.bind(name, args, &["test"], &["test"])?;
                let Value::Lambda(l) = &m["test"] else {
                    return Err(type_err(format!("`where` needs a lambda, found {}", m["test"].type_name()))
                        .help("write it like `where(v -> v.x > 400)` or `where((v, i) -> i % 2 == 0)`"));
                };
                let l = Rc::clone(l);
                let mut out = vec![];
                for (i, item) in items.into_iter().enumerate() {
                    let a = if l.params.len() == 1 { vec![item.clone()] } else { vec![item.clone(), Value::Int(i as i64)] };
                    match self.call_lambda(&l, a, mode)? {
                        Value::Bool(true) => out.push(item),
                        Value::Bool(false) => {}
                        other => {
                            return Err(type_err(format!("the lambda given to `where` must return a Bool, found {}", other.type_name()))
                                .help("compare something, e.g. `v -> v.x > 400`"))
                        }
                    }
                }
                Ok(Value::List(out))
            }
            "nearest" => {
                let m = self.bind(name, args, &["point", "n"], &["point"])?;
                let target = self.point(&m["point"], "`point`")?;
                let n = m.get("n").map(|v| self.count_arg(v, "`n`")).unwrap_or(Ok(1))?;
                let mut scored: Vec<(f64, usize, Value)> = vec![];
                for (i, item) in items.into_iter().enumerate() {
                    let Value::Elem(sn, r) = &item else { return Err(type_err("`nearest` needs a list of elements")) };
                    self.shapes[sn].require_live(*r, None, "nearest")?;
                    let a = self.shapes[sn].move_anchor(*r);
                    scored.push(((a[0] - target[0]).hypot(a[1] - target[1]), i, item));
                }
                scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                Ok(Value::List(scored.into_iter().take(n).map(|x| x.2).collect()))
            }
            "union" | "except" => {
                let m = self.bind(name, args, &["other"], &["other"])?;
                let Value::List(other) = &m["other"] else {
                    return Err(type_err(format!("`{name}` needs another list, found {}", m["other"].type_name())));
                };
                let mut out: Vec<Value> = vec![];
                if name == "union" {
                    for v in items.into_iter().chain(other.iter().cloned()) {
                        if !out.iter().any(|x| self.values_eq(x, &v)) {
                            out.push(v);
                        }
                    }
                } else {
                    out = items.into_iter().filter(|v| !other.iter().any(|x| self.values_eq(x, v))).collect();
                }
                Ok(Value::List(out))
            }
            "move_to" | "move_by" => self.group_move(&items, name, args, span, mode),
            n if BROADCAST.contains(&n) => {
                let mut was_live = vec![];
                for item in &items {
                    let Value::Elem(sn, r) = item else {
                        return Err(type_err(format!("`{name}` applies to a list of vertices, edges or faces, found a {} in the list", item.type_name())));
                    };
                    was_live.push(self.shapes[sn].meta(*r).is_some_and(|m| m.state == State::Live));
                }
                for (item, was) in items.iter().zip(was_live) {
                    let Value::Elem(sn, r) = item else { unreachable!() };
                    // elements removed by an earlier step of this same operation are skipped, not errors
                    if was && !self.shapes[sn].meta(*r).is_some_and(|m| m.state == State::Live) {
                        continue;
                    }
                    self.elem_method(sn, *r, name, args.clone(), span, mode)?;
                }
                Ok(Value::None)
            }
            _ => Err(Diagnostic::new("E-NAM-003", format!("a list has no operation `{name}`")).help(
                "selection methods: every, first, last, skip, reverse, where, nearest, union, except; operations: move_to, move_by, subdivide, extrude, bevel, round, dissolve, inset, delete, bend, set_handle, set_handles, straighten",
            )),
        }
    }

    /// Move a whole selection together. A vertex shared by several selected elements moves once, and
    /// `move_to` puts the centroid of the selection on the target.
    fn group_move(
        &mut self,
        items: &[Value],
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        let m = self.bind(name, args, &["x", "y", "over", "ease"], &["x"])?;
        let p = self.xy(&m)?;
        let mut sn: Option<String> = None;
        let mut refs: Vec<ElemRef> = vec![];
        for item in items {
            let Value::Elem(s, r) = item else {
                return Err(type_err(format!(
                    "`{name}` applies to a list of elements, found a {} in the list",
                    item.type_name()
                )));
            };
            if sn.as_ref().is_some_and(|x| x != s) {
                return Err(type_err(
                    "a selection moved together must come from one shape",
                ));
            }
            self.shapes[s].require_live(*r, None, name)?;
            sn = Some(s.clone());
            refs.push(*r);
        }
        let Some(sn) = sn else { return Ok(Value::None) };
        let by = name == "move_by";
        let sh = &self.shapes[&sn];
        let delta = if by {
            p
        } else {
            let a = sh.group_anchor(&refs);
            [p[0] - a[0], p[1] - a[1]]
        };
        let vids: BTreeSet<u32> = refs.iter().flat_map(|r| sh.element_vertices(*r)).collect();
        match self.over_frames(&m, mode, span)? {
            Some((frames, ease)) => {
                let sh = &self.shapes[&sn];
                let targets = vids
                    .into_iter()
                    .map(|id| {
                        let q = sh.pos(id);
                        (
                            id,
                            if by {
                                Target::By(delta)
                            } else {
                                Target::To([q[0] + delta[0], q[1] + delta[1]])
                            },
                        )
                    })
                    .collect();
                Ok(Value::Pending(Box::new(ActionReq::vertices(
                    sn.clone(),
                    format!("{sn}.{name} ({} elements)", refs.len()),
                    frames,
                    ease,
                    targets,
                ))))
            }
            None => {
                if mode == Mode::Run {
                    for id in vids {
                        self.staged.push(Staged::Move {
                            shape: sn.clone(),
                            r: v(id),
                            x: delta[0],
                            y: delta[1],
                            by: true,
                        });
                    }
                } else {
                    self.with_shape(&sn, mode, span, |sh| {
                        sh.move_group_by(&refs, delta[0], delta[1])
                    })?;
                }
                Ok(Value::None)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shape transform actions and keyframes
// ---------------------------------------------------------------------------------------------

/// Write (or add to) one component of a shape's rest transform.
pub fn set_transform_prop(sh: &mut Shape, prop: PropId, v: f64, by: bool) {
    let slot: Option<&mut f64> = match prop {
        PropId::PosX => Some(&mut sh.position[0]),
        PropId::PosY => Some(&mut sh.position[1]),
        PropId::ScaleX => Some(&mut sh.scale[0]),
        PropId::ScaleY => Some(&mut sh.scale[1]),
        PropId::Rot => Some(&mut sh.rotation),
        _ => None,
    };
    if let Some(s) = slot {
        *s = if by { *s + v } else { v };
    }
}

const PROP_ROOTS: &[&str] = &[
    "position",
    "scale",
    "rotation",
    "fill",
    "stroke",
    "stroke_width",
    "opacity",
    "tint",
];

fn root_name_of(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Name(n) => Some(n),
        ExprKind::Member(o, _) | ExprKind::Index(o, _) => root_name_of(o),
        _ => None,
    }
}

impl Interp {
    /// Pivots default to the bounding-box centre when the build phase ends.
    pub fn freeze_pivots(&mut self) {
        for sh in self.shapes.values_mut() {
            sh.freeze_pivot();
        }
    }

    pub(crate) fn shape_transform(
        &mut self,
        sn: &str,
        label: &str,
        comps: Vec<(PropId, Tgt)>,
        m: &HashMap<&'static str, Value>,
        mode: Mode,
        span: Span,
    ) -> Res<Value> {
        match self.over_frames(m, mode, span)? {
            Some((frames, ease)) => Ok(Value::Pending(Box::new(ActionReq {
                shape: sn.to_string(),
                label: format!("{sn}.{label}"),
                frames,
                ease,
                channels: vec![BODY],
                comps,
                motions: vec![],
            }))),
            None => {
                for (prop, t) in comps {
                    let (value, by) = match t {
                        Tgt::To(v) => (v, false),
                        Tgt::By(v) => (v, true),
                    };
                    self.apply_or_stage(
                        Staged::Transform {
                            shape: sn.to_string(),
                            prop,
                            value,
                            by,
                        },
                        mode,
                    )?;
                }
                Ok(Value::None)
            }
        }
    }

    /// Split `target.position.y` into the target expression and the property path `["position", "y"]`.
    /// A bare `position.y` (inside a `keyframes` block) has no target.
    fn split_prop_path<'a>(&self, e: &'a Expr) -> Res<(Option<&'a Expr>, Vec<String>)> {
        let is_prop = |n: &str| {
            matches!(
                n,
                "position"
                    | "scale"
                    | "rotation"
                    | "fill"
                    | "stroke"
                    | "stroke_width"
                    | "opacity"
                    | "tint"
                    | "x"
                    | "y"
            )
        };
        let mut names: Vec<String> = vec![];
        let mut cur = e;
        while let ExprKind::Member(o, n) = &cur.kind {
            if names.len() < 2 && is_prop(n) {
                names.push(n.clone());
                cur = o;
            } else {
                break;
            }
        }
        names.reverse();
        let mut target = Some(cur);
        if let ExprKind::Name(n) = &cur.kind {
            if PROP_ROOTS.contains(&n.as_str())
                && !names
                    .first()
                    .is_some_and(|f| PROP_ROOTS.contains(&f.as_str()))
            {
                names.insert(0, n.clone());
                target = None;
            }
        }
        let valid = matches!(
            names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .as_slice(),
            ["rotation"]
                | ["position"]
                | ["scale"]
                | ["position", "x" | "y"]
                | ["scale", "x" | "y"]
                | ["fill"]
                | ["stroke"]
                | ["stroke_width"]
                | ["opacity"]
                | ["tint"]
        );
        if !valid {
            let shown = if names.is_empty() {
                root_name_of(e).unwrap_or("?").to_string()
            } else {
                names.join(".")
            };
            return Err(Diagnostic::new("E-NAM-003", format!("`{shown}` is not an animatable property"))
                .at(e.span)
                .help("properties: position, position.x, position.y, scale, scale.x, scale.y, rotation (vertices have position only); colours: fill, stroke, tint; also stroke_width and opacity"));
        }
        Ok((target, names))
    }

    /// The scalars a key assignment sets: `(shape, property, value)`.
    fn key_scalars(
        &self,
        target: &Value,
        path: &[String],
        val: &Value,
        span: Span,
    ) -> Res<Vec<(String, PropId, f64)>> {
        let (p0, comp) = (path[0].as_str(), path.get(1).map(|s| s.as_str()));
        let pair = |v: &Value| -> Res<[f64; 2]> {
            match v {
                Value::Tuple(t) if t.len() == 2 => Ok([
                    self.num(&t[0], "a coordinate")?,
                    self.num(&t[1], "a coordinate")?,
                ]),
                other => Err(type_err(format!(
                    "`{}` needs a point like `(x, y)`, found {}",
                    path.join("."),
                    other.type_name()
                ))),
            }
        };
        match target {
            Value::Shape(sn) => {
                let s = |p: PropId, x: f64| (sn.clone(), p, x);
                Ok(match (p0, comp) {
                    ("position", None) => {
                        let p = pair(val)?;
                        vec![s(PropId::PosX, p[0]), s(PropId::PosY, p[1])]
                    }
                    ("position", Some(c)) => vec![s(
                        if c == "x" { PropId::PosX } else { PropId::PosY },
                        self.num(val, "a coordinate")?,
                    )],
                    ("scale", None) => match val {
                        Value::Tuple(_) => {
                            let p = pair(val)?;
                            vec![s(PropId::ScaleX, p[0]), s(PropId::ScaleY, p[1])]
                        }
                        other => {
                            let n = self.num(other, "`scale`")?;
                            vec![s(PropId::ScaleX, n), s(PropId::ScaleY, n)]
                        }
                    },
                    ("scale", Some(c)) => vec![s(
                        if c == "x" {
                            PropId::ScaleX
                        } else {
                            PropId::ScaleY
                        },
                        self.num(val, "a scale")?,
                    )],
                    ("fill" | "stroke" | "tint", None) => {
                        let c = self.colour_arg(val, &format!("`{p0}`"))?;
                        (0..4)
                            .map(|i| {
                                let id = match p0 {
                                    "fill" => PropId::ShapeFillC(i as u8),
                                    "stroke" => PropId::ShapeStrokeC(i as u8),
                                    _ => PropId::TintC(i as u8),
                                };
                                s(id, c[i])
                            })
                            .collect()
                    }
                    ("stroke_width", None) => {
                        vec![s(PropId::ShapeWidth, self.num(val, "a width")?)]
                    }
                    ("opacity", None) => vec![s(PropId::Opacity, self.num(val, "`opacity`")?)],
                    _ => vec![s(PropId::Rot, self.angle(val, "`rotation`")?)],
                })
            }
            Value::Elem(..) | Value::List(_) => {
                let refs: Vec<(String, ElemRef)> = match target {
                    Value::Elem(sn, r) => vec![(sn.clone(), *r)],
                    Value::List(items) => items
                        .iter()
                        .map(|i| match i {
                            Value::Elem(sn, r) => Ok((sn.clone(), *r)),
                            o => Err(type_err(format!("keyframes apply to vertices, edges, faces or a shape, found a {} in the list", o.type_name()))),
                        })
                        .collect::<Res<_>>()?,
                    _ => unreachable!(),
                };
                if matches!(p0, "fill" | "stroke" | "stroke_width") {
                    let needed = if p0 == "fill" { Kind::F } else { Kind::E };
                    let mut out = vec![];
                    let mut seen: BTreeSet<(String, ElemRef)> = BTreeSet::new();
                    for (sn, r) in &refs {
                        if r.kind == Kind::V {
                            return Err(Diagnostic::new(
                                "E-NAM-003",
                                format!("a vertex has no `{p0}`"),
                            )
                            .at(span)
                            .help(
                                "colour faces with `fill` and edges with `stroke` / `stroke_width`",
                            ));
                        }
                        self.shapes[sn].require_live(*r, Some(needed), "keyframes")?;
                        if !seen.insert((sn.clone(), *r)) {
                            continue;
                        }
                        match p0 {
                            "fill" => {
                                for (i, c) in self.colour_arg(val, "`fill`")?.iter().enumerate() {
                                    out.push((sn.clone(), PropId::FillC(r.id, i as u8), *c));
                                }
                            }
                            "stroke" => {
                                for (i, c) in self.colour_arg(val, "`stroke`")?.iter().enumerate() {
                                    out.push((sn.clone(), PropId::StrokeC(r.id, i as u8), *c));
                                }
                            }
                            _ => out.push((
                                sn.clone(),
                                PropId::EdgeWidth(r.id),
                                self.num(val, "a width")?,
                            )),
                        }
                    }
                    return Ok(out);
                }
                if p0 != "position" {
                    return Err(Diagnostic::new("E-NAM-003", format!("a vertex has no `{p0}`"))
                        .at(span)
                        .help("vertices animate `position`; `rotation` and `scale` belong to the whole shape: `shape.keyframes over ...`"));
                }
                let xy: Option<[f64; 2]> = if comp.is_none() {
                    Some(pair(val)?)
                } else {
                    None
                };
                let one = if comp.is_some() {
                    Some(self.num(val, "a coordinate")?)
                } else {
                    None
                };
                let mut seen: BTreeSet<(String, u32)> = BTreeSet::new();
                let mut out = vec![];
                for (sn, r) in refs {
                    self.shapes[&sn].require_live(r, None, "keyframes")?;
                    for vid in self.shapes[&sn].element_vertices(r) {
                        if !seen.insert((sn.clone(), vid)) {
                            continue; // a vertex shared by several selected elements is keyed once
                        }
                        match (comp, xy, one) {
                            (None, Some(p), _) => {
                                out.push((sn.clone(), PropId::Vx(vid), p[0]));
                                out.push((sn.clone(), PropId::Vy(vid), p[1]));
                            }
                            (Some("x"), _, Some(x)) => out.push((sn.clone(), PropId::Vx(vid), x)),
                            (_, _, Some(y)) => out.push((sn.clone(), PropId::Vy(vid), y)),
                            _ => {}
                        }
                    }
                }
                Ok(out)
            }
            other => Err(type_err(format!(
                "keyframes apply to a vertex, a selection or a shape, found {}",
                other.type_name()
            ))
            .help("for example `s.v[2].keyframes over 1.second:` or `s.keyframes over 1.second:`")),
        }
    }

    /// Turn key blocks into key sets (one per shape property). `limit` is the `over` duration of a
    /// relative `keyframes` statement; `absolute` marks a scene-level `timeline:`.
    #[allow(clippy::too_many_arguments)]
    pub fn build_keysets(
        &mut self,
        implicit: Option<&Value>,
        keys: &[KeyBlock],
        limit: Option<i64>,
        vars: &Vars,
        mode: Mode,
        absolute: bool,
        blend: [i64; 2],
        hold: (bool, bool),
    ) -> Res<Vec<KeySetSpec>> {
        type Group = (bool, Vec<(i64, f64, Ease)>);
        let mut groups: BTreeMap<(String, PropId), Group> = BTreeMap::new();
        for kb in keys {
            let atv = self.eval(&kb.at, vars, mode)?;
            let at = self.frames_of(&atv, kb.at.span)?;
            if let Some(lim) = limit {
                if at > lim {
                    return Err(type_err(format!("a key at {at} frames lies beyond the `over` duration of {lim} frames"))
                        .at(kb.span)
                        .help("make `over` as long as the last key, e.g. `keyframes over 2.seconds:` with a key `at 2.s:`"));
                }
            }
            for a in &kb.assigns {
                let (texpr, path) = self.split_prop_path(&a.lhs)?;
                let tv = match (texpr, implicit) {
                    (Some(e), None) => self.eval(e, vars, mode)?,
                    (None, Some(v)) => v.clone(),
                    (None, None) => {
                        return Err(type_err("a timeline key needs a target").at(a.span).help("write the target too: `s.v[1].position.y = 100`, `s.rotation = 90.deg`"))
                    }
                    (Some(_), Some(_)) => {
                        return Err(type_err("inside `keyframes` write just the property, without a target").at(a.span).help("for example `at 1.s: position.y = 100`"))
                    }
                };
                let val = self.eval(&a.value, vars, mode)?;
                let ease = match &a.ease {
                    None => Ease::Linear,
                    Some(e) => match self.eval(e, vars, mode)? {
                        Value::Ease(x) => x,
                        o => {
                            return Err(type_err(format!(
                                "`ease` must be an easing such as `ease_out`, found {}",
                                o.type_name()
                            ))
                            .at(e.span))
                        }
                    },
                };
                let comps = self
                    .key_scalars(&tv, &path, &val, a.span)
                    .map_err(|d| d.or_at(a.span))?;
                if a.add && comps.iter().any(|c| c.1.is_colour()) {
                    return Err(type_err("colours cannot be offset with `+=`")
                        .at(a.span)
                        .help("use `=` with a colour, e.g. `at 1.s: fill = #ff0000`"));
                }
                for (shape, prop, x) in comps {
                    match groups.get_mut(&(shape.clone(), prop)) {
                        Some((add, list)) => {
                            if *add != a.add {
                                return Err(type_err(format!(
                                    "`{}` mixes `=` and `+=` keys",
                                    path.join(".")
                                ))
                                .at(a.span)
                                .help(
                                    "use `=` for absolute values or `+=` for offsets, not both",
                                ));
                            }
                            if list.iter().any(|k| k.0 == at) {
                                return Err(Diagnostic::new(
                                    "E-PRP-003",
                                    format!("two keys for `{}` at the same time", path.join(".")),
                                )
                                .at(a.span)
                                .help("keys of one property must be at different times"));
                            }
                            list.push((at, x, ease));
                        }
                        None => {
                            groups.insert((shape, prop), (a.add, vec![(at, x, ease)]));
                        }
                    }
                }
            }
        }
        Ok(groups
            .into_iter()
            .map(|((shape, prop), (add, mut list))| {
                list.sort_by_key(|k| k.0);
                KeySetSpec {
                    shape,
                    prop,
                    add,
                    keys: list,
                    absolute,
                    hold_before: hold.0,
                    hold_after: hold.1,
                    blend_in: blend[0],
                    blend_out: blend[1],
                }
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------------------------
// Colour actions, procedural modifiers, physics-style actions
// ---------------------------------------------------------------------------------------------

type Bound = HashMap<&'static str, Value>;

impl Interp {
    /// A colour argument as linear-light RGBA.
    fn colour_arg(&self, v: &Value, what: &str) -> Res<[f64; 4]> {
        match v {
            Value::Color(c) => Ok(to_linear(*c)),
            Value::None => Err(type_err(format!("{what} needs a colour, not `none`"))
                .help("to fade something out animate `opacity` with `fade_to(0)`")),
            o => Err(type_err(format!(
                "{what} must be a colour like #f0a, found {}",
                o.type_name()
            ))),
        }
    }

    /// A timed action when `over` is given, otherwise the instant effect (applied now or staged).
    #[allow(clippy::too_many_arguments)]
    fn style_action(
        &mut self,
        sn: &str,
        label: &str,
        comps: Vec<(PropId, Tgt)>,
        channels: Vec<u32>,
        m: &Bound,
        mode: Mode,
        span: Span,
        instant: Vec<Staged>,
    ) -> Res<Value> {
        match self.over_frames(m, mode, span)? {
            Some((frames, ease)) => Ok(Value::Pending(Box::new(ActionReq {
                shape: sn.to_string(),
                label: format!("{sn}.{label}"),
                frames,
                ease,
                channels,
                comps,
                motions: vec![],
            }))),
            None => {
                for s in instant {
                    self.apply_or_stage(s, mode)?;
                }
                Ok(Value::None)
            }
        }
    }

    fn colour_method(
        &mut self,
        obj: Value,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        match obj {
            Value::Shape(sn) => self.shape_colour(&sn, name, args, span, mode),
            Value::Elem(sn, r) => self.group_colour(&[Value::Elem(sn, r)], name, args, span, mode),
            Value::List(items) => self.group_colour(&items, name, args, span, mode),
            other => Err(Diagnostic::new(
                "E-NAM-003",
                format!("a {} has no operation `{name}`", other.type_name()),
            )),
        }
    }

    fn shape_colour(
        &mut self,
        sn: &str,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        let shape = sn.to_string();
        match name {
            "fill_to" | "stroke_to" | "tint_to" => {
                let m = self.bind(name, args, &["color", "over", "ease"], &["color"])?;
                let c = self.colour_arg(&m["color"], "the colour")?;
                let (ctor, chan, prop): (fn(u8) -> PropId, u32, &str) = match name {
                    "fill_to" => (PropId::ShapeFillC, CH_SHAPE_FILL, "fill"),
                    "stroke_to" => (PropId::ShapeStrokeC, CH_SHAPE_STROKE, "stroke"),
                    _ => (PropId::TintC, CH_TINT, "tint"),
                };
                let comps = (0..4).map(|i| (ctor(i as u8), Tgt::To(c[i]))).collect();
                let instant = vec![Staged::Style {
                    shape: shape.clone(),
                    prop: prop.to_string(),
                    value: m["color"].clone(),
                }];
                self.style_action(sn, name, comps, vec![chan], &m, mode, span, instant)
            }
            "width_to" => {
                let m = self.bind(name, args, &["width", "over", "ease"], &["width"])?;
                let w = self.num(&m["width"], "`width`")?;
                let instant = vec![Staged::Style {
                    shape: shape.clone(),
                    prop: "stroke_width".into(),
                    value: Value::Num(w),
                }];
                self.style_action(
                    sn,
                    name,
                    vec![(PropId::ShapeWidth, Tgt::To(w))],
                    vec![CH_SHAPE_WIDTH],
                    &m,
                    mode,
                    span,
                    instant,
                )
            }
            "fade_to" => {
                let m = self.bind(name, args, &["opacity", "over", "ease"], &["opacity"])?;
                let o = self.num(&m["opacity"], "`opacity`")?;
                if !(0.0..=1.0).contains(&o) {
                    return Err(type_err("`opacity` must be between 0 and 1")
                        .help("0 is invisible, 1 is solid"));
                }
                let instant = vec![Staged::Style {
                    shape: shape.clone(),
                    prop: "opacity".into(),
                    value: Value::Num(o),
                }];
                self.style_action(
                    sn,
                    name,
                    vec![(PropId::Opacity, Tgt::To(o))],
                    vec![CH_OPACITY],
                    &m,
                    mode,
                    span,
                    instant,
                )
            }
            _ => Err(Diagnostic::new(
                "E-NAM-003",
                format!("shape `{sn}` has no operation `{name}`"),
            )),
        }
    }

    /// `fill_to` on faces, `stroke_to` / `width_to` on edges: one element or a whole selection as one action.
    fn group_colour(
        &mut self,
        items: &[Value],
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        let needed = match name {
            "fill_to" => Kind::F,
            "stroke_to" | "width_to" => Kind::E,
            _ => {
                return Err(Diagnostic::new(
                    "E-NAM-003",
                    format!("`{name}` works on a shape, not on its elements"),
                )
                .help("elements take `fill_to` (faces), `stroke_to` and `width_to` (edges)"))
            }
        };
        let m = if name == "width_to" {
            self.bind(name, args, &["width", "over", "ease"], &["width"])?
        } else {
            self.bind(name, args, &["color", "over", "ease"], &["color"])?
        };
        let col = if name == "width_to" {
            None
        } else {
            Some(self.colour_arg(&m["color"], "the colour")?)
        };
        let w = if name == "width_to" {
            Some(self.num(&m["width"], "`width`")?)
        } else {
            None
        };
        let mut sn: Option<String> = None;
        let (mut comps, mut channels, mut instant) = (vec![], vec![], vec![]);
        for item in items {
            let Value::Elem(s, r) = item else {
                return Err(type_err(format!(
                    "`{name}` applies to a list of faces or edges, found a {} in the list",
                    item.type_name()
                )));
            };
            if sn.as_ref().is_some_and(|x| x != s) {
                return Err(type_err(
                    "a selection coloured together must come from one shape",
                ));
            }
            self.shapes[s].require_live(*r, Some(needed), name)?;
            sn = Some(s.clone());
            match (name, col, w) {
                ("fill_to", Some(c), _) => {
                    comps.extend((0..4).map(|i| (PropId::FillC(r.id, i as u8), Tgt::To(c[i]))));
                    channels.push(face_fill_channel(r.id));
                    instant.push(Staged::ElemStyle {
                        shape: s.clone(),
                        r: *r,
                        prop: "fill".into(),
                        value: m["color"].clone(),
                    });
                }
                ("stroke_to", Some(c), _) => {
                    comps.extend((0..4).map(|i| (PropId::StrokeC(r.id, i as u8), Tgt::To(c[i]))));
                    channels.push(edge_stroke_channel(r.id));
                    instant.push(Staged::ElemStyle {
                        shape: s.clone(),
                        r: *r,
                        prop: "stroke".into(),
                        value: m["color"].clone(),
                    });
                }
                (_, _, Some(w)) => {
                    comps.push((PropId::EdgeWidth(r.id), Tgt::To(w)));
                    channels.push(edge_width_channel(r.id));
                    instant.push(Staged::ElemStyle {
                        shape: s.clone(),
                        r: *r,
                        prop: "stroke_width".into(),
                        value: Value::Num(w),
                    });
                }
                _ => {}
            }
        }
        let Some(sn) = sn else { return Ok(Value::None) };
        let label = if items.len() == 1 {
            self.shapes[&sn].label(match &items[0] {
                Value::Elem(_, r) => *r,
                _ => unreachable!(),
            }) + "."
                + name
        } else {
            format!("{sn}.{name} ({} elements)", items.len())
        };
        match self.over_frames(&m, mode, span)? {
            Some((frames, ease)) => Ok(Value::Pending(Box::new(ActionReq {
                shape: sn,
                label,
                frames,
                ease,
                channels,
                comps,
                motions: vec![],
            }))),
            None => {
                for s in instant {
                    self.apply_or_stage(s, mode)?;
                }
                Ok(Value::None)
            }
        }
    }

    /// The vertices a modifier or physics action drives: every vertex of the element(s), once each.
    fn target_vertices(&self, obj: &Value, what: &str) -> Res<Option<(String, Vec<u32>)>> {
        let refs: Vec<(String, ElemRef)> = match obj {
            Value::Elem(s, r) => vec![(s.clone(), *r)],
            Value::List(items) => items
                .iter()
                .map(|i| match i {
                    Value::Elem(s, r) => Ok((s.clone(), *r)),
                    o => Err(type_err(format!(
                        "`{what}` applies to elements, found a {} in the list",
                        o.type_name()
                    ))),
                })
                .collect::<Res<_>>()?,
            _ => return Ok(None),
        };
        let mut shape: Option<String> = None;
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        for (s, r) in &refs {
            if shape.as_ref().is_some_and(|x| x != s) {
                return Err(type_err(format!(
                    "a selection given to `{what}` must come from one shape"
                )));
            }
            self.shapes[s].require_live(*r, None, what)?;
            ids.extend(self.shapes[s].element_vertices(*r));
            shape = Some(s.clone());
        }
        Ok(shape.map(|s| (s, ids.into_iter().collect())))
    }

    /// The next stream seed for a base name: calls on the same target and kind are numbered, so
    /// unrelated modifiers elsewhere never change this one's numbers.
    fn mod_seed(&mut self, base: &str) -> u64 {
        let n = self.mod_counts.entry(base.to_string()).or_insert(0);
        let k = *n;
        *n += 1;
        stream_seed(self.seed, &format!("{base}#{k}"))
    }

    fn axis_arg(&self, m: &Bound) -> Res<&'static str> {
        match m.get("axis") {
            None => Ok("both"),
            Some(Value::Str(s)) => match s.as_str() {
                "x" => Ok("x"),
                "y" => Ok("y"),
                "both" => Ok("both"),
                o => Err(type_err(format!(
                    "`axis` must be x, y or both, found \"{o}\""
                ))),
            },
            Some(o) => Err(type_err(format!(
                "`axis` must be x, y or both, found {}",
                o.type_name()
            ))),
        }
    }

    fn modifier_method(
        &mut self,
        obj: Value,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        let kind = if name == "wobble" {
            ModKind::Wobble
        } else {
            ModKind::Noise
        };
        let m = self.bind(
            name,
            args,
            &[
                "amount",
                "speed",
                "axis",
                "sync",
                "phase",
                "property",
                "blend_in",
                "blend_out",
                "over",
            ],
            &["amount"],
        )?;
        if mode != Mode::Run {
            return Err(
                Diagnostic::new("E-SHP-005", format!("`{name}` needs a running scene"))
                    .help("use it in `main:` or a rule; modifiers add motion over time"),
            );
        }
        let speed = m
            .get("speed")
            .map(|v| self.num(v, "`speed`"))
            .unwrap_or(Ok(1.0))?;
        let sync = match m.get("sync") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(o) => {
                return Err(type_err(format!(
                    "`sync` must be true or false, found {}",
                    o.type_name()
                )))
            }
        };
        let explicit_phase = m.get("phase").map(|v| self.num(v, "`phase`")).transpose()?;
        let frames = match m.get("over") {
            Some(v) => Some(self.frames_of(&v.clone(), span)?),
            None => None,
        };
        let blend = |me: &mut Interp, k: &str| -> Res<i64> {
            match m.get(k) {
                Some(v) => me.frames_of(&v.clone(), span),
                None => Ok(0),
            }
        };
        let (blend_in, blend_out) = (blend(self, "blend_in")?, blend(self, "blend_out")?);
        let mut comps: Vec<ModComp> = vec![];
        let scene = self.scene_name.clone();
        let shape_name;
        if let Some((sn, vids)) = self.target_vertices(&obj, name)? {
            if m.contains_key("property") {
                return Err(type_err("`property:` is for shapes")
                    .help("a vertex always animates its position; pick `axis: x`, `y` or `both`"));
            }
            let amount = self.num(&m["amount"], "`amount`")?;
            let axis = self.axis_arg(&m)?;
            let axes: Vec<&str> = if axis == "both" {
                vec!["x", "y"]
            } else {
                vec![axis]
            };
            shape_name = sn.clone();
            let sync_seeds: BTreeMap<&str, u64> = if sync {
                axes.iter()
                    .map(|a| (*a, self.mod_seed(&format!("{scene}/{sn}/sync/{a}/{name}"))))
                    .collect()
            } else {
                BTreeMap::new()
            };
            for vid in vids {
                for a in &axes {
                    let seed = if sync {
                        sync_seeds[a]
                    } else {
                        self.mod_seed(&format!("{scene}/{sn}/v{vid}/{a}/{name}"))
                    };
                    let prop = if *a == "x" {
                        PropId::Vx(vid)
                    } else {
                        PropId::Vy(vid)
                    };
                    comps.push(ModComp {
                        prop,
                        amount,
                        speed,
                        seed,
                        phase: explicit_phase.unwrap_or_else(|| hash01(seed, 0)),
                    });
                }
            }
        } else if let Value::Shape(sn) = &obj {
            shape_name = sn.clone();
            let prop = match m.get("property") {
                Some(Value::Str(p))
                    if matches!(p.as_str(), "position" | "scale" | "rotation" | "opacity") =>
                {
                    p.clone()
                }
                Some(o) => {
                    return Err(type_err(format!(
                        "`property` must be position, scale, rotation or opacity, found {}",
                        match o {
                            Value::Str(s) => format!("\"{s}\""),
                            o => o.type_name().to_string(),
                        }
                    )))
                }
                None => {
                    return Err(
                        Diagnostic::new("E-TYP-003", "a shape modifier needs `property:`").help(
                            "for example `property: rotation`, `scale`, `position` or `opacity`",
                        ),
                    )
                }
            };
            let amount = if prop == "rotation" {
                self.angle(&m["amount"], "`amount`")?
            } else {
                self.num(&m["amount"], "`amount`")?
            };
            let mk = |me: &mut Interp, id: PropId, label: &str| {
                let seed = me.mod_seed(&format!("{scene}/{sn}/{label}/{name}"));
                ModComp {
                    prop: id,
                    amount,
                    speed,
                    seed,
                    phase: explicit_phase.unwrap_or_else(|| hash01(seed, 0)),
                }
            };
            match prop.as_str() {
                "position" => {
                    let axis = self.axis_arg(&m)?;
                    for a in if axis == "both" {
                        vec!["x", "y"]
                    } else {
                        vec![axis]
                    } {
                        let c = mk(
                            self,
                            if a == "x" { PropId::PosX } else { PropId::PosY },
                            &format!("position/{a}"),
                        );
                        comps.push(c);
                    }
                }
                "scale" => {
                    // a uniform scale: both components share one stream
                    let c = mk(self, PropId::ScaleX, "scale");
                    comps.push(ModComp {
                        prop: PropId::ScaleY,
                        ..c.clone()
                    });
                    comps.push(c);
                }
                "rotation" => {
                    let c = mk(self, PropId::Rot, "rotation");
                    comps.push(c);
                }
                _ => {
                    let c = mk(self, PropId::Opacity, "opacity");
                    comps.push(c);
                }
            }
        } else {
            return Err(Diagnostic::new(
                "E-NAM-003",
                format!("a {} has no operation `{name}`", obj.type_name()),
            ));
        }
        Ok(Value::Modifier(Box::new(ModReq {
            label: format!("{shape_name}.{name}"),
            shape: shape_name,
            kind,
            comps,
            frames,
            blend_in,
            blend_out,
        })))
    }

    fn physics_method(
        &mut self,
        obj: Value,
        name: &str,
        args: Vec<(Option<String>, Value)>,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        let m = if name == "fall" {
            self.bind(
                name,
                args,
                &["floor", "gravity", "bounce", "velocity", "over", "ease"],
                &[],
            )?
        } else {
            self.bind(
                name,
                args,
                &["region", "velocity", "over", "ease"],
                &["region"],
            )?
        };
        let Some((frames, ease)) = self.over_frames(&m, mode, span)? else {
            return Err(
                Diagnostic::new("E-TYP-003", format!("`{name}` needs `over`"))
                    .help("it runs for a while: `... over 2.seconds`"),
            );
        };
        let vel = match m.get("velocity") {
            None => [0.0, 0.0],
            Some(v) => self.point(v, "`velocity`")?,
        };
        let (floor, gravity, bounce) = if name == "fall" {
            let floor = m.get("floor").map(|v| self.num(v, "`floor`")).transpose()?;
            let g = m
                .get("gravity")
                .map(|v| self.num(v, "`gravity`"))
                .unwrap_or(Ok(980.0))?;
            let b = m
                .get("bounce")
                .map(|v| self.num(v, "`bounce`"))
                .unwrap_or(Ok(0.0))?;
            if !(0.0..=1.0).contains(&b) {
                return Err(type_err("`bounce` must be between 0 (no bounce) and 1")
                    .help("0.5 loses half the speed at every bounce"));
            }
            (floor, g, b)
        } else {
            (None, 0.0, 0.0)
        };
        let region = if name == "bounce_in" {
            match &m["region"] {
                Value::Rect(r) => Some(*r),
                o => {
                    return Err(type_err(format!(
                        "`region` must be a rectangle such as `rect(x, y, w, h)`, found {}",
                        o.type_name()
                    )))
                }
            }
        } else {
            None
        };
        let motion = |axis: usize, v: f64| -> Motion {
            match region {
                Some(r) => Motion::Reflect {
                    lo: r[axis],
                    hi: r[axis] + r[axis + 2],
                    v,
                },
                None if axis == 1 => Motion::Fall {
                    v0: v,
                    g: gravity,
                    floor,
                    restitution: bounce,
                },
                None => Motion::Fall {
                    v0: v,
                    g: 0.0,
                    floor: None,
                    restitution: 0.0,
                },
            }
        };
        let (shape, props, channels): (String, Vec<[PropId; 2]>, Vec<u32>) =
            if let Some((sn, vids)) = self.target_vertices(&obj, name)? {
                (
                    sn,
                    vids.iter()
                        .map(|v| [PropId::Vx(*v), PropId::Vy(*v)])
                        .collect(),
                    vids,
                )
            } else if let Value::Shape(sn) = &obj {
                (sn.clone(), vec![[PropId::PosX, PropId::PosY]], vec![BODY])
            } else {
                return Err(Diagnostic::new(
                    "E-NAM-003",
                    format!("a {} has no operation `{name}`", obj.type_name()),
                ));
            };
        let (mut comps, mut motions) = (vec![], vec![]);
        for [px, py] in props {
            comps.push((px, Tgt::To(0.0)));
            motions.push(motion(0, vel[0]));
            comps.push((py, Tgt::To(0.0)));
            motions.push(motion(1, vel[1]));
        }
        Ok(Value::Pending(Box::new(ActionReq {
            label: format!("{shape}.{name}"),
            shape,
            frames,
            ease,
            channels,
            comps,
            motions,
        })))
    }
}
