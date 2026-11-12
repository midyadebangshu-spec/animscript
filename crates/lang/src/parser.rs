//! Recursive-descent parser for the Core-mode subset used by the shape system.
use crate::ast::*;
use crate::lexer::{lex, Token, T};
use animscript_core::{Diagnostic, Res, Span};
use std::rc::Rc;

pub const UNITS: &[&str] = &[
    "seconds", "second", "s", "ms", "frames", "frame", "f", "degrees", "degree", "deg", "turns",
    "turn", "rad", "radians", "px",
];
const SHORT_UNITS: &[&str] = &["s", "ms", "f", "px", "deg"];

pub fn parse(src: &str) -> Res<Program> {
    let toks = lex(src)?;
    let mut p = Parser { toks, i: 0 };
    p.program()
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
}

fn parse_color(hex: &str) -> [u8; 4] {
    let d: Vec<u8> = hex.chars().map(|c| c.to_digit(16).unwrap() as u8).collect();
    match d.len() {
        3 => [d[0] * 17, d[1] * 17, d[2] * 17, 255],
        4 => [d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17],
        6 => [d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], 255],
        _ => [
            d[0] * 16 + d[1],
            d[2] * 16 + d[3],
            d[4] * 16 + d[5],
            d[6] * 16 + d[7],
        ],
    }
}

fn root_name(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Name(n) => Some(n),
        ExprKind::Member(o, _) | ExprKind::Index(o, _) => root_name(o),
        _ => None,
    }
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.toks[self.i.min(self.toks.len() - 1)]
    }
    fn peek_at(&self, k: usize) -> &Token {
        &self.toks[(self.i + k).min(self.toks.len() - 1)]
    }
    fn next(&mut self) -> Token {
        let t = self.peek().clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn prev_span(&self) -> Span {
        self.toks[self.i.saturating_sub(1)].span
    }
    fn at_op(&self, o: &str) -> bool {
        matches!(&self.peek().t, T::Op(x) if *x == o)
    }
    fn at_ident(&self, s: &str) -> bool {
        matches!(&self.peek().t, T::Ident(x) if x == s)
    }
    fn at_ident_k(&self, k: usize, s: &str) -> bool {
        matches!(&self.peek_at(k).t, T::Ident(x) if x == s)
    }
    fn eat_op(&mut self, o: &str) -> bool {
        if self.at_op(o) {
            self.next();
            true
        } else {
            false
        }
    }
    fn err<X>(&self, msg: impl Into<String>) -> Res<X> {
        Err(Diagnostic::new("E-SYN-001", msg).at(self.peek().span))
    }
    fn describe(&self) -> String {
        match &self.peek().t {
            T::Num(s) => format!("number `{s}`"),
            T::Str(_) => "a string".into(),
            T::Ident(s) => format!("`{s}`"),
            T::Color(_) => "a colour".into(),
            T::Canvas(..) => "a canvas size".into(),
            T::Op(o) => format!("`{o}`"),
            T::Newline => "end of line".into(),
            T::Indent => "an unexpected indent".into(),
            T::Dedent => "a dedent".into(),
            T::Eof => "end of file".into(),
        }
    }
    fn expect_op(&mut self, o: &str) -> Res<Token> {
        if self.at_op(o) {
            Ok(self.next())
        } else {
            let code = if o == ":" { "E-SYN-002" } else { "E-SYN-001" };
            let msg = if o == ":" {
                "missing `:` before a block".to_string()
            } else {
                format!("expected `{o}`, found {}", self.describe())
            };
            Err(Diagnostic::new(code, msg).at(self.peek().span))
        }
    }
    fn ident(&mut self, what: &str) -> Res<String> {
        if let T::Ident(s) = &self.peek().t {
            let s = s.clone();
            self.next();
            Ok(s)
        } else {
            self.err(format!("expected {what}, found {}", self.describe()))
        }
    }
    fn end_stmt(&mut self) -> Res<()> {
        match self.peek().t {
            T::Newline => {
                self.next();
                Ok(())
            }
            T::Dedent | T::Eof => Ok(()),
            _ => self.err(format!("expected end of line, found {}", self.describe())),
        }
    }
    fn skip_newlines(&mut self) {
        while matches!(self.peek().t, T::Newline) {
            self.next();
        }
    }

    // ---- top level ---------------------------------------------------------------------

    fn program(&mut self) -> Res<Program> {
        let mut prog = Program::default();
        loop {
            self.skip_newlines();
            match &self.peek().t {
                T::Eof => break,
                T::Ident(s) if s == "project" => {
                    let p = self.project()?;
                    prog.project = Some(p);
                }
                T::Ident(s)
                    if s == "world" && matches!(&self.peek_at(1).t, T::Op(o) if *o == ":") =>
                {
                    let w = self.world()?;
                    prog.world = Some(w);
                }
                T::Ident(s) if s == "emitter" && matches!(&self.peek_at(1).t, T::Ident(_)) => {
                    let start = self.next().span;
                    let name = self.ident("an emitter name")?;
                    let mut attrs = vec![];
                    self.attrs(|p, key, span| {
                        attrs.push((key.to_string(), p.expr()?, span));
                        Ok(())
                    })?;
                    prog.emitters.push(EmitterDecl {
                        name,
                        attrs,
                        span: start,
                    });
                }
                T::Ident(s) if s == "prop" || s == "character" => {
                    let e = self.entity()?;
                    prog.entities.push(e);
                }
                T::Ident(s) if s == "scene" => {
                    let s = self.scene()?;
                    prog.scenes.push(Rc::new(s));
                }
                _ => {
                    let st = self.stmt()?;
                    prog.top.push(st);
                }
            }
        }
        Ok(prog)
    }

    fn block(&mut self) -> Res<Block> {
        self.expect_op(":")?;
        let mut out = vec![];
        if matches!(self.peek().t, T::Newline) {
            self.next();
            if !matches!(self.peek().t, T::Indent) {
                return self.err("expected an indented block");
            }
            self.next();
            loop {
                self.skip_newlines();
                if matches!(self.peek().t, T::Dedent | T::Eof) {
                    break;
                }
                out.push(self.stmt()?);
            }
            if matches!(self.peek().t, T::Dedent) {
                self.next();
            }
        } else {
            out.push(self.stmt()?);
        }
        Ok(Rc::new(out))
    }

    /// `key: value` lines of an indented attribute block; `f` handles one attribute.
    fn attrs(&mut self, mut f: impl FnMut(&mut Self, &str, Span) -> Res<()>) -> Res<()> {
        self.expect_op(":")?;
        if !matches!(self.peek().t, T::Newline) {
            return self.err("expected an indented block of `name: value` lines");
        }
        self.next();
        if !matches!(self.peek().t, T::Indent) {
            return self.err("expected an indented block");
        }
        self.next();
        loop {
            self.skip_newlines();
            if matches!(self.peek().t, T::Dedent | T::Eof) {
                break;
            }
            let span = self.peek().span;
            let key = self.ident("an attribute name")?;
            self.expect_op(":")?;
            f(self, &key, span)?;
            self.end_stmt()?;
        }
        if matches!(self.peek().t, T::Dedent) {
            self.next();
        }
        Ok(())
    }

    fn project(&mut self) -> Res<Project> {
        let start = self.next().span;
        let name = match self.next().t {
            T::Str(s) => s,
            _ => {
                return Err(
                    Diagnostic::new("E-SYN-001", "expected the project name as a string").at(start),
                )
            }
        };
        let mut pr = Project {
            name,
            anim_version: None,
            canvas: (1280, 720),
            fps: (24, 1),
            seed: 0,
            background: None,
            span: start,
        };
        self.attrs(|p, key, span| {
            match key {
                "anim_version" => match p.next().t {
                    T::Str(s) => pr.anim_version = Some(s),
                    _ => {
                        return Err(
                            Diagnostic::new("E-SYN-001", "anim_version must be a string").at(span),
                        )
                    }
                },
                "canvas" => match p.next().t {
                    T::Canvas(w, h) => pr.canvas = (w, h),
                    _ => {
                        return Err(Diagnostic::new(
                            "E-SYN-001",
                            "canvas must look like `1280x720`",
                        )
                        .at(span))
                    }
                },
                "fps" => {
                    let n = match p.next().t {
                        T::Num(s) => s.parse::<i64>().unwrap_or(24),
                        _ => {
                            return Err(Diagnostic::new(
                                "E-SYN-001",
                                "fps must be an integer or a ratio such as 30000/1001",
                            )
                            .at(span))
                        }
                    };
                    let mut d = 1;
                    if p.eat_op("/") {
                        if let T::Num(s) = p.next().t {
                            d = s.parse::<i64>().unwrap_or(1);
                        }
                    }
                    if n <= 0 || d <= 0 {
                        return Err(Diagnostic::new("E-SYN-001", "fps must be positive").at(span));
                    }
                    pr.fps = (n, d);
                }
                "seed" => {
                    if let T::Num(s) = p.next().t {
                        pr.seed = s.parse().unwrap_or(0);
                    }
                }
                "background" => {
                    if let T::Color(h) = p.next().t {
                        pr.background = Some(parse_color(&h));
                    }
                }
                _ => {
                    p.expr()?; // other attributes are accepted and ignored in this slice
                }
            }
            Ok(())
        })?;
        Ok(pr)
    }

    fn world(&mut self) -> Res<WorldDecl> {
        let start = self.next().span;
        let mut w = WorldDecl {
            gravity: None,
            scale: None,
            substeps: None,
            span: start,
        };
        self.attrs(|p, key, span| {
            match key {
                "gravity" => w.gravity = Some(p.expr()?),
                "scale" => w.scale = Some(p.expr()?),
                "substeps" => w.substeps = Some(p.expr()?),
                _ => {
                    return Err(Diagnostic::new(
                        "E-SYN-001",
                        format!("`world` has no setting `{key}`"),
                    )
                    .at(span)
                    .help("settings: gravity, scale, substeps"))
                }
            }
            Ok(())
        })?;
        Ok(w)
    }

    fn entity(&mut self) -> Res<EntityDecl> {
        let start = self.peek().span;
        let kind = self.ident("entity kind")?;
        let name = self.ident("an entity name")?;
        let mut states = vec![];
        self.attrs(|p, key, _| {
            if key == "states" {
                loop {
                    states.push(p.ident("a flag name")?);
                    if !p.eat_op(",") {
                        break;
                    }
                }
            } else {
                p.expr()?;
            }
            Ok(())
        })?;
        Ok(EntityDecl {
            kind,
            name,
            states,
            span: start,
        })
    }

    fn scene(&mut self) -> Res<Scene> {
        let start = self.next().span;
        let name = match self.next().t {
            T::Str(s) => s,
            _ => {
                return Err(
                    Diagnostic::new("E-SYN-001", "expected the scene name as a string").at(start),
                )
            }
        };
        if self.at_ident("in") {
            self.next();
            self.ident("a set name")?;
        }
        if !self.at_ident("for") {
            return self.err("expected `for <duration>` or `for auto`");
        }
        self.next();
        let length = if self.at_ident("auto") {
            self.next();
            SceneLen::Auto
        } else {
            SceneLen::Expr(self.expr()?)
        };
        self.expect_op(":")?;
        if !matches!(self.peek().t, T::Newline) {
            return self.err("expected an indented scene body");
        }
        self.next();
        if !matches!(self.peek().t, T::Indent) {
            return self.err("expected an indented scene body");
        }
        self.next();
        let mut sc = Scene {
            name,
            length,
            setup: None,
            main: None,
            timeline: vec![],
            rules: vec![],
            span: start,
        };
        loop {
            self.skip_newlines();
            if matches!(self.peek().t, T::Dedent | T::Eof) {
                break;
            }
            let sp = self.peek().span;
            let kw = self.ident("a scene item (`setup:`, `main:`, `when`, `whenever`)")?;
            match kw.as_str() {
                "cast" => {
                    self.expect_op(":")?;
                    loop {
                        self.ident("a cast name")?;
                        if !self.eat_op(",") {
                            break;
                        }
                    }
                    self.end_stmt()?;
                }
                "setup" => sc.setup = Some(self.block()?),
                "main" => sc.main = Some(self.block()?),
                "timeline" => sc.timeline = self.key_blocks(None)?,
                "when" | "whenever" => {
                    let kind = if kw == "when" {
                        RuleKind::When
                    } else {
                        RuleKind::Whenever
                    };
                    let trigger = self.expr()?;
                    let mut guard = None;
                    let mut priority = 0;
                    loop {
                        if self.at_ident("if") {
                            self.next();
                            guard = Some(self.expr()?);
                        } else if self.at_ident("priority") {
                            self.next();
                            let neg = self.eat_op("-");
                            if let T::Num(s) = self.next().t {
                                priority = s.parse::<i64>().unwrap_or(0) * if neg { -1 } else { 1 };
                            }
                        } else if self.at_ident("retrigger") {
                            self.next();
                            let m = self.ident("`ignore`")?;
                            if m != "ignore" {
                                return Err(Diagnostic::new(
                                    "E-SYN-001",
                                    "only `retrigger ignore` is supported in this slice",
                                )
                                .at(self.prev_span()));
                            }
                        } else {
                            break;
                        }
                    }
                    let body = self.block()?;
                    sc.rules.push(Rule {
                        kind,
                        trigger,
                        guard,
                        priority,
                        body,
                        span: sp,
                    });
                }
                other => {
                    return Err(Diagnostic::new(
                        "E-SYN-001",
                        format!("`{other}` is not supported in a scene yet"),
                    )
                    .at(sp)
                    .help("this build supports `cast:`, `setup:`, `main:`, `when` and `whenever`"))
                }
            }
        }
        if matches!(self.peek().t, T::Dedent) {
            self.next();
        }
        Ok(sc)
    }

    // ---- statements --------------------------------------------------------------------

    fn stmt(&mut self) -> Res<Stmt> {
        let start = self.peek().span;
        let kw = if let T::Ident(s) = &self.peek().t {
            s.clone()
        } else {
            String::new()
        };
        let mk = |kind: StmtKind, end: Span| Stmt {
            kind,
            span: start.to(end),
        };
        match kw.as_str() {
            "let" => {
                self.next();
                let name = self.ident("a name")?;
                self.expect_op("=")?;
                if self.at_ident("start") {
                    self.next();
                    return self.start_stmt(Some(name), start);
                }
                let e = self.expr()?;
                self.end_stmt()?;
                return Ok(mk(StmtKind::Let(name, e), self.prev_span()));
            }
            "shape"
                if matches!(self.peek_at(1).t, T::Ident(_))
                    && matches!(self.peek_at(2).t, T::Op("=")) =>
            {
                self.next();
                let name = self.ident("a shape name")?;
                self.next();
                let expr = self.expr()?;
                let end = self.prev_span();
                let body = if self.at_op(":") {
                    Some(self.block()?)
                } else {
                    self.end_stmt()?;
                    None
                };
                return Ok(mk(StmtKind::ShapeDecl { name, expr, body }, end));
            }
            "for" => {
                self.next();
                let var = self.ident("a loop variable")?;
                if !self.at_ident("in") {
                    return self.err("expected `in`");
                }
                self.next();
                let lo = self.expr()?;
                let iter = if self.eat_op("..") {
                    let hi = self.expr()?;
                    let sp = lo.span.to(hi.span);
                    Expr {
                        kind: ExprKind::Range(Box::new(lo), Box::new(hi)),
                        span: sp,
                    }
                } else {
                    lo
                };
                let end = self.prev_span();
                let body = self.block()?;
                return Ok(mk(StmtKind::For { var, iter, body }, end));
            }
            "if" => {
                self.next();
                let cond = self.expr()?;
                let end = self.prev_span();
                let then = self.block()?;
                let else_ = if self.at_ident("else") {
                    self.next();
                    Some(self.block()?)
                } else {
                    None
                };
                return Ok(mk(StmtKind::If { cond, then, else_ }, end));
            }
            "else" => return self.err("`else` without a matching `if`"),
            "repeat" => {
                self.next();
                let count = self.expr()?;
                if !self.at_ident("times") {
                    return self.err("expected `times` (only `repeat N times:` is supported)");
                }
                self.next();
                let end = self.prev_span();
                let body = self.block()?;
                return Ok(mk(StmtKind::Repeat { count, body }, end));
            }
            "together" if matches!(self.peek_at(1).t, T::Op(":")) => {
                self.next();
                let body = self.block()?;
                return Ok(mk(StmtKind::Together(body), start));
            }
            "start" => {
                self.next();
                return self.start_stmt(None, start);
            }
            "cancel" | "await" => {
                self.next();
                let e = self.expr()?;
                self.end_stmt()?;
                let kind = if kw == "cancel" {
                    StmtKind::Cancel(e)
                } else {
                    StmtKind::Await(e)
                };
                return Ok(mk(kind, self.prev_span()));
            }
            "wait" => {
                self.next();
                let e = self.expr()?;
                self.end_stmt()?;
                return Ok(mk(StmtKind::Wait(e), self.prev_span()));
            }
            "note" => {
                self.next();
                let s = match self.next().t {
                    T::Str(s) => s,
                    _ => {
                        return Err(Diagnostic::new("E-SYN-001", "`note` needs a string").at(start))
                    }
                };
                self.end_stmt()?;
                return Ok(mk(StmtKind::Note(s), self.prev_span()));
            }
            "assert" => {
                self.next();
                let e = self.expr()?;
                self.end_stmt()?;
                return Ok(mk(StmtKind::Assert(e), self.prev_span()));
            }
            "pass" => {
                self.next();
                self.end_stmt()?;
                return Ok(mk(StmtKind::Pass, start));
            }
            _ => {}
        }
        // `bird becomes flag` / `bird clears flag`
        if matches!(self.peek().t, T::Ident(_))
            && (self.at_ident_k(1, "becomes") || self.at_ident_k(1, "clears"))
        {
            let target = self.ident("an entity")?;
            let set = self.ident("")? == "becomes";
            let flag = self.ident("a flag name")?;
            self.end_stmt()?;
            return Ok(mk(StmtKind::Become { target, flag, set }, self.prev_span()));
        }
        let e = self.expr()?;
        if let ExprKind::Member(obj, name) = &e.kind {
            if name == "keyframes" && self.at_ident("over") {
                self.next();
                let over = self.expr()?;
                let target = (**obj).clone();
                let mut opts = KeyOpts::default();
                let keys = self.key_blocks(Some(&mut opts))?;
                return Ok(mk(
                    StmtKind::Keyframes {
                        target,
                        over,
                        opts,
                        keys,
                    },
                    start,
                ));
            }
        }
        if self.eat_op("=") {
            let v = self.expr()?;
            self.end_stmt()?;
            return Ok(mk(StmtKind::Assign(e, v), self.prev_span()));
        }
        let (mut over, mut ease) = (None, None);
        if self.at_ident("over") {
            self.next();
            over = Some(self.expr()?);
        }
        if self.at_ident("ease") {
            self.next();
            ease = Some(self.expr()?);
        }
        self.end_stmt()?;
        let end = self.prev_span();
        Ok(mk(
            StmtKind::Expr {
                expr: e,
                over,
                ease,
            },
            end,
        ))
    }

    // ---- expressions -------------------------------------------------------------------

    pub fn expr(&mut self) -> Res<Expr> {
        if let Some(l) = self.lambda()? {
            return Ok(l);
        }
        self.binary(0)
    }

    /// After `start`: the call, then the optional `over` / `ease` modifiers.
    fn start_stmt(&mut self, name: Option<String>, start: Span) -> Res<Stmt> {
        let call = self.expr()?;
        let (mut over, mut ease) = (None, None);
        if self.at_ident("over") {
            self.next();
            over = Some(self.expr()?);
        }
        if self.at_ident("ease") {
            self.next();
            ease = Some(self.expr()?);
        }
        self.end_stmt()?;
        Ok(Stmt {
            kind: StmtKind::Start {
                name,
                call,
                over,
                ease,
            },
            span: start.to(self.prev_span()),
        })
    }

    /// The indented body of `keyframes over D:` or `timeline:`: `at TIME:` blocks and, for `keyframes`,
    /// the option lines `hold_before`, `hold_after`, `blend_in: D`, `blend_out: D`.
    fn key_blocks(&mut self, mut opts: Option<&mut KeyOpts>) -> Res<Vec<KeyBlock>> {
        self.expect_op(":")?;
        if !matches!(self.peek().t, T::Newline) {
            return self.err("expected an indented block of `at TIME:` keys");
        }
        self.next();
        if !matches!(self.peek().t, T::Indent) {
            return self.err("expected an indented block of `at TIME:` keys");
        }
        self.next();
        let mut out = vec![];
        loop {
            self.skip_newlines();
            if matches!(self.peek().t, T::Dedent | T::Eof) {
                break;
            }
            let span = self.peek().span;
            if self.at_ident("at") {
                self.next();
                let at = self.expr()?;
                self.expect_op(":")?;
                let mut assigns = vec![];
                if matches!(self.peek().t, T::Newline) {
                    self.next();
                    if !matches!(self.peek().t, T::Indent) {
                        return self.err("expected the keys under `at ...:`");
                    }
                    self.next();
                    loop {
                        self.skip_newlines();
                        if matches!(self.peek().t, T::Dedent | T::Eof) {
                            break;
                        }
                        assigns.push(self.key_assign()?);
                    }
                    if matches!(self.peek().t, T::Dedent) {
                        self.next();
                    }
                } else {
                    assigns.push(self.key_assign()?);
                }
                out.push(KeyBlock { at, assigns, span });
                continue;
            }
            let Some(o) = opts.as_mut() else {
                return self.err(format!("expected `at TIME:`, found {}", self.describe()));
            };
            let word =
                self.ident("`at`, `hold_before`, `hold_after`, `blend_in` or `blend_out`")?;
            match word.as_str() {
                "hold_before" => o.hold_before = true,
                "hold_after" => o.hold_after = true,
                "blend_in" | "blend_out" => {
                    self.expect_op(":")?;
                    let e = self.expr()?;
                    if word == "blend_in" {
                        o.blend_in = Some(e);
                    } else {
                        o.blend_out = Some(e);
                    }
                }
                other => {
                    return Err(Diagnostic::new(
                        "E-SYN-001",
                        format!("`{other}` is not a keyframe option"),
                    )
                    .at(span)
                    .help("options: hold_before, hold_after, blend_in: D, blend_out: D"))
                }
            }
            self.end_stmt()?;
        }
        if matches!(self.peek().t, T::Dedent) {
            self.next();
        }
        Ok(out)
    }

    /// `property = value [ease E]` or `property += value [ease E]`.
    fn key_assign(&mut self) -> Res<KeyAssign> {
        let start = self.peek().span;
        let lhs = self.expr()?;
        let add = if self.eat_op("+=") {
            true
        } else if self.eat_op("=") {
            false
        } else {
            return self.err(format!("expected `=` or `+=`, found {}", self.describe()));
        };
        let value = self.expr()?;
        let ease = if self.at_ident("ease") {
            self.next();
            Some(self.expr()?)
        } else {
            None
        };
        self.end_stmt()?;
        Ok(KeyAssign {
            lhs,
            add,
            value,
            ease,
            span: start.to(self.prev_span()),
        })
    }

    /// `name -> expr` or `(a, b) -> expr`; the body extends as far right as an expression can.
    fn lambda(&mut self) -> Res<Option<Expr>> {
        let start = self.peek().span;
        let mut params = vec![];
        let mut skip;
        if matches!(self.peek().t, T::Ident(_)) && matches!(self.peek_at(1).t, T::Op("->")) {
            if let T::Ident(n) = &self.peek().t {
                params.push(n.clone());
            }
            skip = 2;
        } else if self.at_op("(") {
            let mut k = 1;
            loop {
                match &self.peek_at(k).t {
                    T::Ident(s) => params.push(s.clone()),
                    _ => return Ok(None),
                }
                k += 1;
                match self.peek_at(k).t {
                    T::Op(",") => k += 1,
                    T::Op(")") => {
                        k += 1;
                        break;
                    }
                    _ => return Ok(None),
                }
            }
            if !matches!(self.peek_at(k).t, T::Op("->")) {
                return Ok(None);
            }
            skip = k + 1;
        } else {
            return Ok(None);
        }
        if params.len() > 2 {
            return Err(Diagnostic::new(
                "E-SYN-001",
                "a lambda takes one or two parameters: the element and, optionally, its index",
            )
            .at(start)
            .help("write `v -> v.x > 0` or `(v, i) -> i % 2 == 0`"));
        }
        while skip > 0 {
            self.next();
            skip -= 1;
        }
        let body = self.expr()?;
        let span = start.to(body.span);
        Ok(Some(Expr {
            kind: ExprKind::Lambda {
                params,
                body: Rc::new(body),
            },
            span,
        }))
    }

    fn binary(&mut self, level: usize) -> Res<Expr> {
        // lowest to highest precedence
        const LEVELS: &[&[&str]] = &[
            &["??"],
            &["or"],
            &["and"],
            &["==", "!="],
            &["<", "<=", ">", ">="],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        loop {
            let op = match &self.peek().t {
                T::Op(o) if LEVELS[level].contains(o) => *o,
                T::Ident(s) if LEVELS[level].contains(&s.as_str()) => {
                    if s == "or" {
                        "or"
                    } else {
                        "and"
                    }
                }
                _ => break,
            };
            self.next();
            let rhs = self.binary(level + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
                span,
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Res<Expr> {
        let start = self.peek().span;
        if self.at_op("-") {
            self.next();
            let e = self.unary()?;
            return Ok(Expr {
                span: start.to(e.span),
                kind: ExprKind::Unary("-", Box::new(e)),
            });
        }
        if self.at_ident("not") {
            self.next();
            let e = self.unary()?;
            return Ok(Expr {
                span: start.to(e.span),
                kind: ExprKind::Unary("not", Box::new(e)),
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Res<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.at_op(".") {
                self.next();
                match self.peek().t.clone() {
                    T::Ident(name) => {
                        let sp = self.next().span;
                        e = Expr {
                            span: e.span.to(sp),
                            kind: ExprKind::Member(Box::new(e), name),
                        };
                    }
                    T::Num(n) => {
                        let root = root_name(&e).unwrap_or("shape").to_string();
                        return Err(Diagnostic::new(
                            "E-SYN-001",
                            format!("unexpected number `{n}` after `.`"),
                        )
                        .at(self.peek().span)
                        .help(format!(
                            "`v1.1` is not valid syntax; write `{root}.v1` for vertex 1"
                        ))
                        .help(format!(
                            "for loops and computed access use the indexed form `{root}.v[n]`"
                        )));
                    }
                    _ => {
                        return self.err(format!(
                            "expected a member name after `.`, found {}",
                            self.describe()
                        ))
                    }
                }
            } else if self.at_op("[") {
                self.next();
                let idx = self.expr()?;
                let end = self.expect_op("]")?.span;
                e = Expr {
                    span: e.span.to(end),
                    kind: ExprKind::Index(Box::new(e), Box::new(idx)),
                };
            } else if self.at_op("(") {
                self.next();
                let mut args = vec![];
                while !self.at_op(")") {
                    let name = if matches!(self.peek().t, T::Ident(_))
                        && matches!(self.peek_at(1).t, T::Op(":"))
                    {
                        let n = self.ident("")?;
                        self.next();
                        Some(n)
                    } else {
                        None
                    };
                    let value = self.expr()?;
                    args.push(Arg { name, value });
                    if !self.eat_op(",") {
                        break;
                    }
                }
                let end = self.expect_op(")")?.span;
                e = Expr {
                    span: e.span.to(end),
                    kind: ExprKind::Call(Box::new(e), args),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> Res<Expr> {
        let tok = self.next();
        let span = tok.span;
        let kind = match tok.t {
            T::Num(n) => {
                // unit suffix: `2.seconds`, `1.5s`
                if self.at_op(".") && self.peek().adj {
                    if let T::Ident(u) = self.peek_at(1).t.clone() {
                        if self.peek_at(1).adj {
                            if !UNITS.contains(&u.as_str()) {
                                return Err(Diagnostic::new(
                                    "E-LEX-004",
                                    format!("unknown unit suffix `{u}`"),
                                )
                                .at(self.peek_at(1).span)
                                .help("units: seconds, ms, frames, degrees, turns, rad, px"));
                            }
                            self.next();
                            let us = self.next().span;
                            return Ok(Expr {
                                span: span.to(us),
                                kind: ExprKind::Unit { num: n, unit: u },
                            });
                        }
                    }
                }
                if let T::Ident(u) = self.peek().t.clone() {
                    if self.peek().adj {
                        if !SHORT_UNITS.contains(&u.as_str()) {
                            return Err(Diagnostic::new(
                                "E-LEX-004",
                                format!("unknown unit suffix `{u}`"),
                            )
                            .at(self.peek().span));
                        }
                        let us = self.next().span;
                        return Ok(Expr {
                            span: span.to(us),
                            kind: ExprKind::Unit { num: n, unit: u },
                        });
                    }
                }
                ExprKind::Num(n)
            }
            T::Str(s) => ExprKind::Str(s),
            T::Color(h) => ExprKind::Color(parse_color(&h)),
            T::Ident(s) => match s.as_str() {
                "true" => ExprKind::Bool(true),
                "false" => ExprKind::Bool(false),
                _ => ExprKind::Name(s),
            },
            T::Op("(") => {
                let first = self.expr()?;
                if self.at_op(",") {
                    let mut items = vec![first];
                    while self.eat_op(",") {
                        if self.at_op(")") {
                            break;
                        }
                        items.push(self.expr()?);
                    }
                    let end = self.expect_op(")")?.span;
                    return Ok(Expr {
                        span: span.to(end),
                        kind: ExprKind::Tuple(items),
                    });
                }
                let end = self.expect_op(")")?.span;
                return Ok(Expr {
                    span: span.to(end),
                    kind: first.kind,
                });
            }
            T::Op("[") => {
                let mut items = vec![];
                while !self.at_op("]") {
                    items.push(self.expr()?);
                    if !self.eat_op(",") {
                        break;
                    }
                }
                let end = self.expect_op("]")?.span;
                return Ok(Expr {
                    span: span.to(end),
                    kind: ExprKind::List(items),
                });
            }
            _ => {
                self.i = self.i.saturating_sub(1);
                return self.err(format!("expected an expression, found {}", self.describe()));
            }
        };
        Ok(Expr { kind, span })
    }
}
