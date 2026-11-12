//! `anim check`: parse, build, dry-run every `setup:`, then static rules (E-SHP-001, E-SHP-003).
use crate::ast::*;
use crate::interp::*;
use crate::parser::parse;
use animscript_core::*;

pub fn check(src: &str) -> Vec<Diagnostic> {
    let program = match parse(src) {
        Ok(p) => p,
        Err(d) => return vec![d],
    };
    let mut it = match Interp::new(src, program) {
        Ok(i) => i,
        Err(d) => return vec![d],
    };
    let mut out = vec![];
    let built = it.build();
    out.append(&mut it.diagnostics);
    if let Err(d) = built {
        out.push(d);
        return out;
    }
    let scenes = it.program.scenes.clone();
    for sc in &scenes {
        if let Some(setup) = &sc.setup {
            let vars = Vars {
                scopes: vec![it.globals.clone()],
            };
            let mut t = Task::new(0, std::rc::Rc::clone(setup), vars, Mode::Setup, true);
            let r = it.run_sync(&mut t);
            out.append(&mut it.diagnostics);
            if let Err(d) = r {
                out.push(d);
                return out;
            }
        }
    }
    for sc in &scenes {
        if let Some(m) = &sc.main {
            walk_block(&it, m, &mut out);
        }
        for r in &sc.rules {
            walk_expr(&it, &r.trigger, &mut out);
            walk_block(&it, &r.body, &mut out);
        }
    }
    out
}

fn walk_block(it: &Interp, b: &Block, out: &mut Vec<Diagnostic>) {
    for s in b.iter() {
        walk_stmt(it, s, out);
    }
}

fn walk_stmt(it: &Interp, s: &Stmt, out: &mut Vec<Diagnostic>) {
    match &s.kind {
        StmtKind::Let(_, e) | StmtKind::Wait(e) | StmtKind::Assert(e) => walk_expr(it, e, out),
        StmtKind::Assign(a, b) => {
            walk_expr(it, a, out);
            walk_expr(it, b, out);
        }
        StmtKind::Expr { expr, over, ease } => {
            walk_expr(it, expr, out);
            for e in [over, ease].into_iter().flatten() {
                walk_expr(it, e, out);
            }
        }
        StmtKind::For { iter, body, .. } => {
            walk_expr(it, iter, out);
            walk_block(it, body, out);
        }
        StmtKind::If { cond, then, else_ } => {
            walk_expr(it, cond, out);
            walk_block(it, then, out);
            if let Some(e) = else_ {
                walk_block(it, e, out);
            }
        }
        StmtKind::Repeat { count, body } => {
            walk_expr(it, count, out);
            walk_block(it, body, out);
        }
        StmtKind::Together(b) => walk_block(it, b, out),
        StmtKind::Keyframes {
            target,
            over,
            opts,
            keys,
        } => {
            walk_expr(it, target, out);
            walk_expr(it, over, out);
            for e in [&opts.blend_in, &opts.blend_out].into_iter().flatten() {
                walk_expr(it, e, out);
            }
            for k in keys {
                walk_expr(it, &k.at, out);
                for a in &k.assigns {
                    walk_expr(it, &a.lhs, out);
                    walk_expr(it, &a.value, out);
                }
            }
        }
        StmtKind::ShapeDecl { name, .. } => out.push(
            Diagnostic::new(
                "E-SHP-003",
                format!("shape `{name}` cannot be created while the scene is running"),
            )
            .at(s.span)
            .help("declare shapes at the top level or in `setup:`"),
        ),
        StmtKind::Become { .. } | StmtKind::Note(_) | StmtKind::Pass => {}
    }
}

fn walk_expr(it: &Interp, e: &Expr, out: &mut Vec<Diagnostic>) {
    match &e.kind {
        ExprKind::Call(f, args) => {
            if let ExprKind::Member(obj, name) = &f.kind {
                if TOPOLOGY_METHODS.contains(&name.as_str()) {
                    out.push(
                        Diagnostic::new("E-SHP-003", format!("`{name}` changes topology and is not allowed while the scene is running"))
                            .at(e.span)
                            .help("topology edits (subdivide, extrude, bevel, round, smooth, merge, delete, bend) run in `setup:` or a `shape` build block")
                            .help("at runtime only vertex positions animate: `shape.v[2].move_by(0, 10) over 0.3.seconds`"),
                    );
                }
                if name == "morph_to" {
                    if let (ExprKind::Name(a), Some(ExprKind::Name(b))) =
                        (&obj.kind, args.first().map(|x| &x.value.kind))
                    {
                        if let (Some(sa), Some(sb)) = (it.shapes.get(a), it.shapes.get(b)) {
                            if let Err(d) = sa.check_morph(sb) {
                                out.push(d.at(e.span));
                            }
                        }
                    }
                }
                walk_expr(it, obj, out);
            } else {
                walk_expr(it, f, out);
            }
            for a in args {
                walk_expr(it, &a.value, out);
            }
        }
        ExprKind::Member(o, _) | ExprKind::Unary(_, o) => walk_expr(it, o, out),
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::Range(a, b) => {
            walk_expr(it, a, out);
            walk_expr(it, b, out);
        }
        ExprKind::Tuple(v) | ExprKind::List(v) => v.iter().for_each(|x| walk_expr(it, x, out)),
        ExprKind::Lambda { body, .. } => walk_expr(it, body, out),
        _ => {}
    }
}
