//! Derived flags (`derive name when condition`, spec 4.8) and the flag predicates `held_for`, `since`
//! and `rose`. A derived flag is recomputed at the start of every delta, in dependency order, from a
//! condition in which `self` is its entity and bare names are that entity's own flags. Every flag keeps
//! a little history (when it last changed) so the predicates can look back.
use crate::ast::*;
use crate::interp::{Interp, Mode, Vars};
use crate::value::*;
use animscript_core::*;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
pub struct FlagHist {
    pub value: bool,
    /// The tick the flag took its current value.
    pub since: i64,
    /// `(tick, delta)` of the last rising edge.
    pub rose: (i64, u32),
}

impl Default for FlagHist {
    fn default() -> FlagHist {
        FlagHist {
            value: false,
            since: 0,
            rose: (-1, 0),
        }
    }
}

pub struct DerivedFlag {
    pub entity: String,
    pub name: String,
    pub cond: Rc<Expr>,
}

/// The flags a condition mentions, as `(entity, flag)` (the owner of `self` and bare names is `own`).
fn collect_refs(e: &Expr, own: &str, out: &mut Vec<(String, String)>) {
    match &e.kind {
        ExprKind::Name(n) => out.push((own.to_string(), n.clone())),
        ExprKind::Member(o, f) => match &o.kind {
            ExprKind::Name(x) => {
                let ent = if x == "self" { own } else { x };
                out.push((ent.to_string(), f.clone()));
            }
            _ => collect_refs(o, own, out),
        },
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::Range(a, b) => {
            collect_refs(a, own, out);
            collect_refs(b, own, out);
        }
        ExprKind::Call(f, args) => {
            match &f.kind {
                ExprKind::Member(o, _) => collect_refs(o, own, out),
                _ => collect_refs(f, own, out),
            }
            for a in args {
                collect_refs(&a.value, own, out);
            }
        }
        ExprKind::Unary(_, x) => collect_refs(x, own, out),
        ExprKind::Tuple(items) | ExprKind::List(items) => {
            for i in items {
                collect_refs(i, own, out);
            }
        }
        ExprKind::Lambda { body, .. } => collect_refs(body, own, out),
        _ => {}
    }
}

impl Interp {
    /// Validate the `derive` lines of every entity and put them in dependency order.
    pub fn register_derives(&mut self, entities: &[EntityDecl]) -> Res<()> {
        let mut all: Vec<(String, String, Rc<Expr>, Span)> = vec![];
        for e in entities {
            for (name, cond, span) in &e.derives {
                let ent = self.entities.get_mut(&e.name).expect("registered");
                if ent.flags.contains_key(name) {
                    return Err(Diagnostic::new(
                        "E-NAM-002",
                        format!("`{}` already has a flag named `{name}`", e.name),
                    )
                    .at(*span));
                }
                ent.flags.insert(name.clone(), false);
                ent.derived.push(name.clone());
                all.push((e.name.clone(), name.clone(), Rc::new(cond.clone()), *span));
            }
        }
        let names: BTreeSet<(String, String)> =
            all.iter().map(|d| (d.0.clone(), d.1.clone())).collect();
        let mut deps: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
        for (ent, name, cond, _) in &all {
            let mut refs = vec![];
            collect_refs(cond, ent, &mut refs);
            refs.retain(|r| names.contains(r));
            deps.insert((ent.clone(), name.clone()), refs);
        }
        // depth-first topological order; meeting a node that is being visited is a cycle
        let mut state: BTreeMap<(String, String), u8> = BTreeMap::new();
        let mut order: Vec<(String, String)> = vec![];
        fn visit(
            n: &(String, String),
            deps: &BTreeMap<(String, String), Vec<(String, String)>>,
            state: &mut BTreeMap<(String, String), u8>,
            order: &mut Vec<(String, String)>,
            path: &mut Vec<String>,
        ) -> Result<(), String> {
            match state.get(n) {
                Some(2) => return Ok(()),
                Some(1) => {
                    path.push(format!("{}.{}", n.0, n.1));
                    return Err(path.join(" -> "));
                }
                _ => {}
            }
            state.insert(n.clone(), 1);
            path.push(format!("{}.{}", n.0, n.1));
            for d in &deps[n] {
                visit(d, deps, state, order, path)?;
            }
            path.pop();
            state.insert(n.clone(), 2);
            order.push(n.clone());
            Ok(())
        }
        for d in &all {
            let key = (d.0.clone(), d.1.clone());
            if let Err(chain) = visit(&key, &deps, &mut state, &mut order, &mut vec![]) {
                return Err(Diagnostic::new(
                    "E-STA-002",
                    format!("derived flags depend on each other in a cycle: {chain}"),
                )
                .at(d.3)
                .help("a derived flag can only use flags that do not depend on it"));
            }
        }
        for key in order {
            let d = all
                .iter()
                .find(|d| d.0 == key.0 && d.1 == key.1)
                .expect("known");
            self.derived.push(DerivedFlag {
                entity: d.0.clone(),
                name: d.1.clone(),
                cond: Rc::clone(&d.2),
            });
        }
        Ok(())
    }

    fn sync_flag(&mut self, ent: &str, flag: &str, value: bool) {
        let (tick, delta) = (self.tick, self.delta);
        let h = self
            .flag_hist
            .entry((ent.to_string(), flag.to_string()))
            .or_default();
        if h.value != value {
            h.value = value;
            h.since = tick;
            if value {
                h.rose = (tick, delta);
            }
        }
    }

    /// Start of a delta: note what the authored flags are now, then recompute every derived flag in order.
    pub fn update_flags(&mut self) -> Res<()> {
        let authored: Vec<(String, String, bool)> = self
            .entities
            .iter()
            .flat_map(|(en, e)| {
                e.states
                    .iter()
                    .map(move |s| (en.clone(), s.clone(), e.flags[s]))
            })
            .collect();
        for (en, f, v) in authored {
            self.sync_flag(&en, &f, v);
        }
        let n = self.derived.len();
        for i in 0..n {
            let (ent, name, cond) = {
                let d = &self.derived[i];
                (d.entity.clone(), d.name.clone(), Rc::clone(&d.cond))
            };
            let mut vars = Vars::new();
            vars.scopes[0].insert("self".into(), Value::Entity(ent.clone()));
            for (k, v) in &self.entities[&ent].flags {
                vars.scopes[0].insert(k.clone(), Value::Bool(*v));
            }
            let v = self
                .eval(&cond, &vars, Mode::Run)
                .map_err(|d| d.or_at(cond.span))?;
            let Value::Bool(b) = v else {
                return Err(Diagnostic::new(
                    "E-TYP-001",
                    format!(
                        "a `derive` condition must be a Bool, found {}",
                        v.type_name()
                    ),
                )
                .at(cond.span));
            };
            self.entities
                .get_mut(&ent)
                .expect("entity")
                .flags
                .insert(name.clone(), b);
            self.sync_flag(&ent, &name, b);
        }
        Ok(())
    }

    fn entity_named(&self, n: &str, vars: &Vars) -> Option<String> {
        if n == "self" {
            return match vars.get("self") {
                Some(Value::Entity(e)) => Some(e.clone()),
                _ => None,
            };
        }
        if vars.get(n).is_some() {
            return None;
        }
        self.entities.contains_key(n).then(|| n.to_string())
    }

    /// The `(entity, flag)` an expression such as `bird.afraid`, `self.afraid` or a bare `afraid` names.
    pub fn flag_ref(&self, e: &Expr, vars: &Vars) -> Option<(String, String)> {
        match &e.kind {
            ExprKind::Member(o, f) => {
                let ExprKind::Name(n) = &o.kind else {
                    return None;
                };
                let ent = self.entity_named(n, vars)?;
                self.entities[&ent]
                    .flags
                    .contains_key(f)
                    .then(|| (ent, f.clone()))
            }
            ExprKind::Name(f) => {
                let Some(Value::Entity(ent)) = vars.get("self") else {
                    return None;
                };
                self.entities[ent]
                    .flags
                    .contains_key(f)
                    .then(|| (ent.clone(), f.clone()))
            }
            _ => None,
        }
    }

    /// `x.held_for(d)` and `x.since(d)`, when `x` names a flag.
    pub fn flag_call(
        &mut self,
        recv: &Expr,
        name: &str,
        args: &[Arg],
        vars: &Vars,
        mode: Mode,
    ) -> Option<Res<Value>> {
        if name != "held_for" && name != "since" {
            return None;
        }
        let key = self.flag_ref(recv, vars)?;
        Some((|| {
            let [a] = args else {
                return Err(Diagnostic::new(
                    "E-TYP-002",
                    format!("`{name}` takes one duration"),
                ));
            };
            let d = self.eval(&a.value, vars, mode)?;
            let n = self.frames_of(&d, a.value.span)?;
            let h = self.flag_hist.get(&key).copied().unwrap_or_default();
            let age = self.tick - h.since;
            Ok(Value::Bool(
                h.value
                    && if name == "held_for" {
                        age >= n
                    } else {
                        age <= n
                    },
            ))
        })())
    }

    /// `x.rose`: the flag became true in this delta.
    pub fn flag_rose(&self, recv: &Expr, vars: &Vars) -> Option<Value> {
        let key = self.flag_ref(recv, vars)?;
        let h = self.flag_hist.get(&key).copied().unwrap_or_default();
        Some(Value::Bool(h.value && h.rose == (self.tick, self.delta)))
    }
}
