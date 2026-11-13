//! User-defined `action`s and `fn`s: registration, argument binding with types and defaults, calling.
//! An `fn` runs to completion in a throwaway task and returns a value; an `action` call evaluates to a
//! deferred `Value::Call` that the statement running it pushes onto its task (so it blocks like a
//! built-in action) or that `start` runs as a task of its own.
use crate::ast::*;
use crate::interp::{Frame, Interp, Mode, Step, Task, Vars};
use crate::value::*;
use animscript_core::*;
use std::collections::HashMap;
use std::rc::Rc;

type Args = Vec<(Option<String>, Value)>;

const TYPES: &[&str] = &[
    "Any", "Number", "Int", "Bool", "String", "Point", "Vec2", "Angle", "Duration", "Color",
    "Shape", "Armature", "Entity", "List", "Easing", "Region", "Vertex", "Edge", "Face", "Handle",
];
/// Calls nested inside one another: an `fn` call recurses on the Rust stack, so it is limited more
/// tightly than an inline `action`, which only adds frames to its task.
const MAX_DEPTH: usize = 64;
const MAX_FN_DEPTH: usize = 64;

impl Interp {
    pub fn register_defs(&mut self) -> Res<()> {
        let defs = self.program.defs.clone();
        for d in defs {
            if self.defs.contains_key(&d.name) {
                return Err(
                    Diagnostic::new("E-DEF-001", format!("`{}` is defined twice", d.name))
                        .at(d.span)
                        .help("rename one of them"),
                );
            }
            for p in &d.params {
                if let Some(t) = &p.ty {
                    if !TYPES.contains(&t.as_str()) {
                        return Err(Diagnostic::new(
                            "E-DEF-003",
                            format!(
                                "unknown type `{t}` for parameter `{}` of `{}`",
                                p.name, d.name
                            ),
                        )
                        .at(p.span)
                        .help(format!("types: {}", TYPES.join(", "))));
                    }
                }
            }
            if let Some(t) = &d.ret {
                if !TYPES.contains(&t.as_str()) {
                    return Err(Diagnostic::new(
                        "E-DEF-003",
                        format!("unknown return type `{t}` of `{}`", d.name),
                    )
                    .at(d.span));
                }
            }
            self.defs.insert(d.name.clone(), d);
        }
        Ok(())
    }

    fn type_ok(ty: &str, v: &Value) -> bool {
        match (ty, v) {
            ("Any", _) => true,
            ("Number", Value::Int(_) | Value::Num(_)) | ("Int", Value::Int(_)) => true,
            ("Bool", Value::Bool(_))
            | ("String", Value::Str(_))
            | ("Angle", Value::Angle(_))
            | ("Color", Value::Color(_)) => true,
            ("Duration", Value::Dur(_) | Value::Frames(_)) => true,
            ("Point" | "Vec2", Value::Tuple(t)) => {
                t.len() == 2 && t.iter().all(|x| x.as_f64().is_some())
            }
            ("Shape", Value::Shape(_))
            | ("Armature", Value::Armature(_))
            | ("Entity", Value::Entity(_))
            | ("List", Value::List(_))
            | ("Easing", Value::Ease(_))
            | ("Region", Value::Rect(_))
            | ("Handle", Value::Handle(_)) => true,
            ("Vertex", Value::Elem(_, r)) => r.kind == Kind::V,
            ("Edge", Value::Elem(_, r)) => r.kind == Kind::E,
            ("Face", Value::Elem(_, r)) => r.kind == Kind::F,
            _ => false,
        }
    }

    /// Bind call arguments to the parameters: `self` first when called as a method, then positional
    /// arguments in order, then named ones; defaults are evaluated in order with the earlier parameters visible.
    fn bind_user(
        &mut self,
        def: &Def,
        args: Args,
        self_val: Option<Value>,
        mode: Mode,
    ) -> Res<HashMap<String, Value>> {
        let mut bound: HashMap<String, Value> = HashMap::new();
        let mut next = 0usize;
        if let Some(s) = self_val {
            if def.params.first().map(|p| p.name.as_str()) != Some("self") {
                return Err(Diagnostic::new(
                    "E-TYP-003",
                    format!("`{}` has no `self` parameter", def.name),
                ));
            }
            bound.insert("self".into(), s);
            next = 1;
        }
        for (name, v) in args {
            match name {
                None => {
                    while next < def.params.len() && bound.contains_key(&def.params[next].name) {
                        next += 1;
                    }
                    let Some(p) = def.params.get(next) else {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("too many arguments to `{}`", def.name),
                        )
                        .help(format!("it takes: {}", Self::signature(def))));
                    };
                    bound.insert(p.name.clone(), v);
                    next += 1;
                }
                Some(n) => {
                    if !def.params.iter().any(|p| p.name == n) {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("`{}` has no argument named `{n}`", def.name),
                        )
                        .help(format!("it takes: {}", Self::signature(def))));
                    }
                    if bound.insert(n.clone(), v).is_some() {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("argument `{n}` of `{}` is given twice", def.name),
                        ));
                    }
                }
            }
        }
        for p in &def.params {
            if !bound.contains_key(&p.name) {
                match &p.default {
                    Some(e) => {
                        let vars = Vars {
                            scopes: vec![self.globals.clone(), bound.clone()],
                        };
                        let v = self.eval(e, &vars, mode)?;
                        bound.insert(p.name.clone(), v);
                    }
                    None => {
                        return Err(Diagnostic::new(
                            "E-TYP-003",
                            format!("`{}` needs the argument `{}`", def.name, p.name),
                        )
                        .help(format!("it takes: {}", Self::signature(def))))
                    }
                }
            }
            if let Some(t) = &p.ty {
                let v = &bound[&p.name];
                if !Self::type_ok(t, v) {
                    return Err(Diagnostic::new(
                        "E-TYP-001",
                        format!(
                            "argument `{}` of `{}` must be a {t}, found {}",
                            p.name,
                            def.name,
                            v.type_name()
                        ),
                    )
                    .help(format!("it takes: {}", Self::signature(def))));
                }
            }
        }
        Ok(bound)
    }

    fn cannot_take_time(def: &Def) -> Diagnostic {
        Diagnostic::new("E-DEF-002", format!("`fn {}` cannot take time", def.name)).help("a `fn` computes a value instantly; use an `action` for anything that waits or animates")
    }

    fn signature(def: &Def) -> String {
        def.params
            .iter()
            .map(|p| {
                format!(
                    "{}{}{}",
                    p.name,
                    p.ty.as_ref().map(|t| format!(": {t}")).unwrap_or_default(),
                    if p.default.is_some() { " = ..." } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Call a user definition: an `fn` returns its value, an `action` returns a deferred call.
    pub(crate) fn call_def(
        &mut self,
        def: &Rc<Def>,
        args: Args,
        self_val: Option<Value>,
        mode: Mode,
    ) -> Res<Value> {
        let scope = self.bind_user(def, args, self_val, mode)?;
        for r in &def.requires {
            let vars = Vars {
                scopes: vec![self.globals.clone(), scope.clone()],
            };
            match self.eval(r, &vars, mode)? {
                Value::Bool(true) => {}
                Value::Bool(false) => {
                    return Err(Diagnostic::new(
                        "E-CAP-002",
                        format!(
                            "`{}` cannot be called here: `{}` is not satisfied",
                            def.name,
                            self.span_text(r.span)
                        ),
                    )
                    .at(r.span))
                }
                other => {
                    return Err(Diagnostic::new(
                        "E-TYP-001",
                        format!("`requires` must be a Bool, found {}", other.type_name()),
                    )
                    .at(r.span))
                }
            }
        }
        if def.is_action {
            return Ok(Value::Call(Rc::new(UserCall {
                def: Rc::clone(def),
                scope,
            })));
        }
        self.run_fn(def, scope, mode)
    }

    fn run_fn(&mut self, def: &Rc<Def>, scope: HashMap<String, Value>, mode: Mode) -> Res<Value> {
        if self.call_depth >= MAX_FN_DEPTH {
            return Err(Diagnostic::new(
                "E-RUN-007",
                format!(
                    "`{}` calls itself too deeply (more than {MAX_FN_DEPTH} levels)",
                    def.name
                ),
            )
            .help("a recursive `fn` needs a case that stops"));
        }
        let vars = Vars {
            scopes: vec![self.globals.clone(), scope],
        };
        let mut t = Task::new(0, Rc::clone(&def.body), vars, mode, false);
        t.frames.insert(
            0,
            Frame::Call {
                saved: None,
                id: 0,
                uses: Rc::new(vec![]),
            },
        );
        self.ret = None;
        self.call_depth += 1;
        let result = match self.step(&mut t) {
            Ok(Step::Done) => Ok(()),
            Ok(Step::Blocked(_)) => Err(Self::cannot_take_time(def)),
            Err(d) if d.code == "E-STM-001" => Err(Self::cannot_take_time(def)),
            Err(d) => Err(d),
        };
        self.call_depth -= 1;
        result?;
        let v = self.ret.take();
        match (&def.ret, v) {
            (Some(ty), Some(v)) => {
                if Self::type_ok(ty, &v) {
                    Ok(v)
                } else {
                    Err(Diagnostic::new(
                        "E-TYP-001",
                        format!(
                            "`fn {}` must return a {ty}, but returned {}",
                            def.name,
                            v.type_name()
                        ),
                    ))
                }
            }
            (Some(ty), None) => Err(Diagnostic::new(
                "E-TYP-001",
                format!("`fn {}` must return a {ty}, but returned nothing", def.name),
            )),
            (None, v) => Ok(v.unwrap_or(Value::None)),
        }
    }

    /// Run a deferred action call inline on the calling task: its body becomes the task's next frames.
    pub(crate) fn enter_call(&mut self, t: &mut Task, c: &Rc<UserCall>) -> Res<()> {
        let depth = t
            .frames
            .iter()
            .filter(|f| matches!(f, Frame::Call { .. }))
            .count();
        if depth >= MAX_DEPTH {
            return Err(Diagnostic::new(
                "E-RUN-007",
                format!(
                    "`{}` calls itself too deeply (more than {MAX_DEPTH} levels)",
                    c.def.name
                ),
            )
            .help("a recursive `action` needs a case that stops"));
        }
        let vars = Vars {
            scopes: vec![self.globals.clone(), c.scope.clone()],
        };
        let saved = std::mem::replace(&mut t.vars, vars);
        self.next_call += 1;
        t.frames.push(Frame::Call {
            saved: Some(saved),
            id: self.next_call,
            uses: Rc::new(c.def.uses.iter().map(|u| u.0.clone()).collect()),
        });
        t.frames.push(Frame::Block {
            stmts: Rc::clone(&c.def.body),
            idx: 0,
            scope: false,
        });
        Ok(())
    }

    /// `return`: stop the innermost call, remembering the value for an `fn`.
    pub(crate) fn do_return(&mut self, t: &mut Task, value: Option<Value>) {
        self.ret = value;
        while let Some(f) = t.frames.pop() {
            if let Frame::Block { stmts, idx, .. } = &f {
                if let Some(Stmt {
                    kind: StmtKind::Finally(b),
                    ..
                }) = stmts.last()
                {
                    if *idx < stmts.len() {
                        let _ = self.run_final(b, &t.vars, t.mode);
                    }
                }
            }
            if let Frame::Call { saved, .. } = f {
                if let Some(v) = saved {
                    t.vars = v;
                }
                break;
            }
        }
    }
}
