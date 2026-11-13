//! The mathematical animation engine in the language: equations (a string like `"sin(x / 40 + t * 3) * 8"` or a
//! lambda like `(x, t) -> sin(x / 40 + t * 3) * 8`) compile to an `animscript_math::Program` that
//! - builds shapes (`plot`, `curve`, `polar`, `implicit`), once, and
//! - drives a shape (`drive`) as a layer-2 modifier, evaluated for all its vertices at every tick.
//!
//! Equations are in radians and plain numbers; lambdas may only use arithmetic, comparisons, `and`/`or`/`not` and
//! the math functions of `animscript_math::apply_fn`; any sub-expression that mentions none of the equation's
//! inputs is evaluated by the interpreter first, so `amp * 2`, `3.seconds` or `s.v2.x` all work as constants.
use crate::ast::{Expr, ExprKind};
use crate::interp::{Interp, Mode, Vars};
use crate::value::*;
use animscript_core::*;
use animscript_math as m;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

type Args = Vec<(Option<String>, Value)>;
type Split = (HashMap<&'static str, Value>, BTreeMap<String, f64>);

fn mth(code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, msg)
}

/// With `JitMode::Auto`, drives over at least this many vertices are compiled (a compile costs about half a
/// millisecond, which fewer points never win back).
#[cfg(feature = "jit")]
const JIT_MIN_POINTS: usize = 128;

/// The equation inputs of a `drive`.
const DRIVE_POINT: [&str; 4] = ["x", "y", "i", "u"];
const DRIVE_FRAME: [&str; 2] = ["t", "n"];

fn mentions(e: &Expr, scope: &HashMap<String, m::Id>) -> bool {
    match &e.kind {
        ExprKind::Name(n) => scope.contains_key(n),
        ExprKind::Num(_)
        | ExprKind::Unit { .. }
        | ExprKind::Str(_)
        | ExprKind::Bool(_)
        | ExprKind::Color(_) => false,
        ExprKind::Member(a, _) => mentions(a, scope),
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::Range(a, b) => {
            mentions(a, scope) || mentions(b, scope)
        }
        ExprKind::Call(f, args) => {
            mentions(f, scope) || args.iter().any(|a| mentions(&a.value, scope))
        }
        ExprKind::Unary(_, a) => mentions(a, scope),
        ExprKind::Tuple(v) | ExprKind::List(v) => v.iter().any(|x| mentions(x, scope)),
        ExprKind::Lambda { .. } => false,
    }
}

impl Interp {
    /// A value that must be a plain number in an equation context (durations are seconds, angles radians).
    fn eq_const(&self, v: &Value) -> Res<f64> {
        match v {
            Value::Int(i) => Ok(*i as f64),
            Value::Num(n) => Ok(*n),
            Value::Dur(r) => Ok(r.to_f64()),
            Value::Frames(n) => Ok(*n as f64 / self.fps.to_f64()),
            Value::Angle(t) => Ok(t * std::f64::consts::TAU),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            o => Err(mth(
                "E-MTH-002",
                format!(
                    "a {} cannot be used as a number inside an equation",
                    o.type_name()
                ),
            )),
        }
    }

    /// Lower a lambda body to IR. Names in `scope` are the equation's inputs.
    fn lower(
        &mut self,
        e: &Expr,
        b: &mut m::Builder,
        scope: &HashMap<String, m::Id>,
        vars: &Vars,
        seed: u32,
    ) -> Res<m::Id> {
        if !mentions(e, scope) {
            let v = self.eval(e, vars, Mode::Build).map_err(|d| d.at(e.span))?;
            let c = self.eq_const(&v).map_err(|d| d.at(e.span))?;
            return Ok(b.constant(c));
        }
        let unsupported = |what: &str| {
            mth("E-MTH-002", format!("{what} cannot be used in an equation that depends on its inputs"))
                .at(e.span)
                .help("equations may use + - * / %, comparisons, and/or/not and the math functions (sin, cos, pow, select, ...)")
        };
        match &e.kind {
            ExprKind::Name(n) => Ok(scope[n]),
            ExprKind::Unary(op, a) => {
                let a = self.lower(a, b, scope, vars, seed)?;
                match *op {
                    "-" => Ok(b.neg(a)),
                    "not" => Ok(b.un(m::Un::Not, a)),
                    _ => Err(unsupported(&format!("`{op}`"))),
                }
            }
            ExprKind::Binary(op, x, y) => {
                let op = match *op {
                    "+" => m::Bin::Add,
                    "-" => m::Bin::Sub,
                    "*" => m::Bin::Mul,
                    "/" => m::Bin::Div,
                    "%" => m::Bin::Rem,
                    "<" => m::Bin::Lt,
                    "<=" => m::Bin::Le,
                    ">" => m::Bin::Gt,
                    ">=" => m::Bin::Ge,
                    "==" => m::Bin::Eq,
                    "!=" => m::Bin::Ne,
                    "and" => m::Bin::And,
                    "or" => m::Bin::Or,
                    o => return Err(unsupported(&format!("`{o}`"))),
                };
                let x = self.lower(x, b, scope, vars, seed)?;
                let y = self.lower(y, b, scope, vars, seed)?;
                Ok(b.bin(op, x, y))
            }
            ExprKind::Call(f, args) => {
                let ExprKind::Name(fname) = &f.kind else {
                    return Err(unsupported("this call"));
                };
                if args.iter().any(|a| a.name.is_some()) {
                    return Err(unsupported("a named argument"));
                }
                let ids: Vec<m::Id> = args
                    .iter()
                    .map(|a| self.lower(&a.value, b, scope, vars, seed))
                    .collect::<Res<_>>()?;
                m::apply_fn(b, fname, &ids, seed).map_err(|msg| mth("E-MTH-002", msg).at(e.span))
            }
            _ => Err(unsupported("this expression")),
        }
    }

    /// Compile equations over named inputs into one program (one output per equation).
    fn compile_equations(
        &mut self,
        eqs: &[(&str, &Value)],
        point: &[&str],
        frame: &[&str],
        consts: &BTreeMap<String, f64>,
        seed: u32,
    ) -> Res<m::Program> {
        let mut b = m::Builder::new();
        let mut scope: HashMap<String, m::Id> = HashMap::new();
        for n in point {
            scope.insert(n.to_string(), b.input(n, m::Rate::Point));
        }
        for n in frame {
            scope.insert(n.to_string(), b.input(n, m::Rate::Frame));
        }
        let mut env = m::Env::new();
        env.noise_seed = seed;
        env.names = scope.clone();
        for (k, v) in consts {
            env.names.insert(k.clone(), b.constant(*v));
        }
        let mut outs = vec![];
        for (label, v) in eqs {
            let id = match v {
                Value::Str(src) => m::parse(src, &mut b, &env).map_err(|e| {
                    mth(
                        "E-MTH-001",
                        format!("in the equation for `{label}`: {}", e.msg),
                    )
                    .help(format!("at position {} of \"{src}\"", e.at))
                })?,
                Value::Lambda(l) => {
                    let mut sc = HashMap::new();
                    for p in &l.params {
                        let Some(id) = scope.get(p) else {
                            return Err(mth(
                                "E-MTH-002",
                                format!("`{p}` is not an input of the equation for `{label}`"),
                            )
                            .help(format!(
                                "its inputs are: {}",
                                point
                                    .iter()
                                    .chain(frame)
                                    .copied()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )));
                        };
                        sc.insert(p.clone(), *id);
                    }
                    for (k, v) in consts {
                        sc.entry(k.clone()).or_insert_with(|| b.constant(*v));
                    }
                    // named numbers are constants, not inputs: they must not count as "mentioned"
                    let inputs: HashMap<String, m::Id> =
                        l.params.iter().map(|p| (p.clone(), sc[p])).collect();
                    let mut vars = l.vars.clone();
                    vars.push();
                    for (k, v) in consts {
                        vars.define(k, Value::Num(*v));
                    }
                    self.lower(&l.body, &mut b, &inputs, &vars, seed)?
                }
                Value::Int(_) | Value::Num(_) => b.constant(self.eq_const(v)?),
                o => {
                    return Err(mth(
                        "E-MTH-003",
                        format!(
                            "the equation for `{label}` must be a string or a lambda, found {}",
                            o.type_name()
                        ),
                    ))
                }
            };
            outs.push(id);
        }
        Ok(b.finish(&outs))
    }

    /// Split arguments into the named ones in `known` and the rest; the rest must be numbers (named constants).
    fn split_math_args(
        &self,
        what: &str,
        args: Args,
        known: &[&'static str],
        reserved: &[&str],
    ) -> Res<Split> {
        let mut named: HashMap<&'static str, Value> = HashMap::new();
        let mut consts = BTreeMap::new();
        for (name, v) in args {
            let Some(n) = name else {
                return Err(
                    mth("E-MTH-003", format!("`{what}` takes named arguments only"))
                        .help(format!("known: {}", known.join(", "))),
                );
            };
            if let Some(k) = known.iter().find(|k| **k == n) {
                if named.insert(k, v).is_some() {
                    return Err(mth(
                        "E-MTH-003",
                        format!("argument `{n}` given twice to `{what}`"),
                    ));
                }
            } else if reserved.contains(&n.as_str()) {
                return Err(mth(
                    "E-MTH-003",
                    format!("`{n}` is an equation input and cannot be a named number"),
                ));
            } else if matches!(
                v,
                Value::Int(_)
                    | Value::Num(_)
                    | Value::Dur(_)
                    | Value::Frames(_)
                    | Value::Angle(_)
                    | Value::Bool(_)
            ) {
                consts.insert(n, self.eq_const(&v)?);
            } else {
                return Err(
                    mth("E-MTH-004", format!("`{what}` has no property `{n}`")).help(format!(
                        "known: {}; any other name must be a number",
                        known.join(", ")
                    )),
                );
            }
        }
        Ok((named, consts))
    }

    fn seed32(&mut self, base: &str) -> u32 {
        (self.mod_seed(base) & 0xffff_ffff) as u32
    }

    // ---- shapes from equations ------------------------------------------------------------------

    pub(crate) fn math_shape(&mut self, name: &str, args: Args) -> Res<Value> {
        if name == "implicit" {
            return self.implicit_shape(args);
        }
        let (known, var, eq_names): (&[&'static str], &str, &[&str]) = match name {
            "plot" => (&["y", "from", "to", "n", "closed"], "x", &["y"]),
            "curve" => (&["x", "y", "from", "to", "n", "closed"], "u", &["x", "y"]),
            _ => (&["r", "from", "to", "n", "closed"], "theta", &["r"]),
        };
        let (mut m, consts) = self.split_math_args(name, args, known, &[var])?;
        for k in eq_names {
            if !m.contains_key(k) {
                return Err(mth(
                    "E-MTH-003",
                    format!("`{name}` is missing the equation `{k}`"),
                ));
            }
        }
        let n = match m.get("n") {
            Some(v) => self.int(v, "`n`")?,
            None => 100,
        };
        if !(2..=100_000).contains(&n) {
            return Err(mth(
                "E-MTH-003",
                "`n` (the number of samples) must be between 2 and 100000",
            ));
        }
        let n = n as usize;
        let full_turn = name == "polar" && !m.contains_key("from") && !m.contains_key("to");
        let get = |me: &Interp, k: &str, d: f64, m: &HashMap<&'static str, Value>| match m.get(k) {
            Some(v) => me.num(v, &format!("`{k}`")),
            None => Ok(d),
        };
        let (from, to) = match name {
            "polar" => (
                get(self, "from", 0.0, &m)?,
                get(self, "to", std::f64::consts::TAU, &m)?,
            ),
            _ => (get(self, "from", 0.0, &m)?, get(self, "to", 1.0, &m)?),
        };
        let closed = match m.remove("closed") {
            Some(Value::Bool(b)) => b,
            Some(o) => {
                return Err(mth(
                    "E-MTH-003",
                    format!("`closed` must be true or false, found {}", o.type_name()),
                ))
            }
            None => full_turn,
        };
        let seed = self.seed32(&format!("{}/{name}", self.scene_name));
        let eqs: Vec<(&str, &Value)> = eq_names.iter().map(|k| (*k, &m[k])).collect();
        let prog = self.compile_equations(&eqs, &[var], &[], &consts, seed)?;
        let denom = if closed { n as f64 } else { (n - 1) as f64 };
        let ts: Vec<f64> = (0..n)
            .map(|k| from + (to - from) * k as f64 / denom)
            .collect();
        let out = prog.eval(&[&ts], &[], &[]).expect("inputs match");
        let mut pts = Vec::with_capacity(n);
        for k in 0..n {
            let p = match name {
                "plot" => [ts[k], out[0][k]],
                "curve" => [out[0][k], out[1][k]],
                _ => [out[0][k] * ts[k].cos(), out[0][k] * ts[k].sin()],
            };
            if !p[0].is_finite() || !p[1].is_finite() {
                return Err(mth(
                    "E-MTH-005",
                    format!("`{name}` gave a non-finite value at {var} = {}", ts[k]),
                )
                .help("avoid dividing by zero or taking the log or root of a negative number"));
            }
            pts.push(p);
        }
        Ok(Value::Fresh(Box::new(polyline(&pts, closed)?)))
    }

    fn implicit_shape(&mut self, args: Args) -> Res<Value> {
        let known = ["f", "x0", "y0", "x1", "y1", "res"];
        let (m, consts) = self.split_math_args("implicit", args, &known, &["x", "y"])?;
        for k in ["f", "x0", "y0", "x1", "y1"] {
            if !m.contains_key(k) {
                return Err(mth("E-MTH-003", format!("`implicit` is missing `{k}`"))
                    .help("implicit(f: \"x*x + y*y - 100\", x0: -50, y0: -50, x1: 50, y1: 50, res: 64)"));
            }
        }
        let (x0, y0, x1, y1) = (
            self.num(&m["x0"], "`x0`")?,
            self.num(&m["y0"], "`y0`")?,
            self.num(&m["x1"], "`x1`")?,
            self.num(&m["y1"], "`y1`")?,
        );
        let res = match m.get("res") {
            Some(v) => self.int(v, "`res`")?,
            None => 64,
        };
        if !(1..=2000).contains(&res) || x1 <= x0 || y1 <= y0 {
            return Err(mth(
                "E-MTH-003",
                "`res` must be 1 to 2000 and the bounds must have x1 > x0 and y1 > y0",
            ));
        }
        let res = res as usize;
        let seed = self.seed32(&format!("{}/implicit", self.scene_name));
        let prog = self.compile_equations(&[("f", &m["f"])], &["x", "y"], &[], &consts, seed)?;
        let w = res + 1;
        let (mut xs, mut ys) = (Vec::with_capacity(w * w), Vec::with_capacity(w * w));
        for j in 0..w {
            for i in 0..w {
                xs.push(x0 + (x1 - x0) * i as f64 / res as f64);
                ys.push(y0 + (y1 - y0) * j as f64 / res as f64);
            }
        }
        let f = prog
            .eval(&[&xs, &ys], &[], &[])
            .expect("inputs match")
            .remove(0);
        if f.iter().any(|v| !v.is_finite()) {
            return Err(mth(
                "E-MTH-005",
                "`implicit` gave a non-finite value inside its bounds",
            ));
        }
        let (pts, pairs) = marching_squares(&xs, &ys, &f, res);
        if pairs.is_empty() {
            return Err(mth(
                "E-MTH-003",
                "the equation has no zero crossing inside the bounds",
            )
            .help(
                "the contour is where `f` changes sign; widen the bounds or check the equation",
            ));
        }
        Ok(Value::Fresh(Box::new(segments(&pts, &pairs))))
    }

    /// The interpreter or, in a `jit` build, machine code for the same program (see `JitMode`).
    fn make_kernel(&mut self, prog: m::Program, points: usize) -> Kernel {
        #[cfg(feature = "jit")]
        {
            let want = match self.jit {
                JitMode::Off => false,
                JitMode::Always => true,
                JitMode::Auto => points >= JIT_MIN_POINTS,
            };
            if want {
                match prog.compile() {
                    Ok(c) => {
                        self.trace_line("JIT", &format!("compiled a drive over {points} points"));
                        return Kernel::Jit(Box::new(c));
                    }
                    Err(e) => self.trace_line("JIT", &format!("{e}; using the interpreter")),
                }
            }
        }
        let _ = points;
        Kernel::Interp(prog)
    }

    // ---- drive ---------------------------------------------------------------------------------

    pub(crate) fn drive_method(
        &mut self,
        obj: Value,
        args: Args,
        span: Span,
        mode: Mode,
    ) -> Res<Value> {
        if mode != Mode::Run {
            return Err(mth("E-SHP-005", "`drive` needs a running scene")
                .help("use it in `main:` or a rule; a drive adds motion over time"));
        }
        const VERT: [&str; 2] = ["x", "y"];
        const SHAPE: [(&str, &str); 6] = [
            ("opacity", "opacity"),
            ("width", "width"),
            ("rotation", "rotation"),
            ("scale", "scale"),
            ("pos_x", "pos_x"),
            ("pos_y", "pos_y"),
        ];
        let known: [&'static str; 12] = [
            "x",
            "y",
            "opacity",
            "width",
            "rotation",
            "scale",
            "pos_x",
            "pos_y",
            "speed",
            "mode",
            "blend_in",
            "blend_out",
        ];
        let mut args = args;
        // `over` may be given as an argument like for `wobble`
        let over = args
            .iter()
            .position(|(n, _)| n.as_deref() == Some("over"))
            .map(|i| args.remove(i).1);
        let reserved: Vec<&str> = DRIVE_POINT
            .iter()
            .chain(DRIVE_FRAME.iter())
            .copied()
            .collect();
        let (m, consts) = self.split_math_args("drive", args, &known, &reserved)?;
        let vert_eqs: Vec<&str> = VERT.iter().copied().filter(|k| m.contains_key(k)).collect();
        let shape_eqs: Vec<&str> = SHAPE
            .iter()
            .map(|s| s.0)
            .filter(|k| m.contains_key(k))
            .collect();
        if vert_eqs.is_empty() && shape_eqs.is_empty() {
            return Err(mth("E-MTH-003", "`drive` needs at least one equation")
                .help("for example `w.drive(y: \"sin(x / 40 + t * 3) * 8\")`; properties: x, y, opacity, width, rotation, scale, pos_x, pos_y"));
        }
        if !vert_eqs.is_empty() && !shape_eqs.is_empty() {
            return Err(mth(
                "E-MTH-004",
                "a `drive` moves vertices (x, y) or sets shape properties, not both",
            )
            .help("start two drives"));
        }
        let replace = match m.get("mode") {
            None => false,
            Some(Value::Str(s)) if s == "add" => false,
            Some(Value::Str(s)) if s == "replace" => true,
            Some(_) => return Err(mth("E-MTH-003", "`mode` must be \"add\" or \"replace\"")),
        };
        let speed = match m.get("speed") {
            Some(v) => self.num(v, "`speed`")?,
            None => 1.0,
        };
        let frames = match &over {
            Some(v) => Some(self.frames_of(v, span)?),
            None => None,
        };
        let blend = |me: &mut Interp, k: &str| -> Res<i64> {
            match m.get(k) {
                Some(v) => me.frames_of(&v.clone(), span),
                None => Ok(0),
            }
        };
        let (blend_in, blend_out) = (blend(self, "blend_in")?, blend(self, "blend_out")?);

        let (shape, verts): (String, Vec<u32>) =
            if let Some((sn, vids)) = self.target_vertices(&obj, "drive")? {
                if !shape_eqs.is_empty() {
                    return Err(mth(
                        "E-MTH-004",
                        "shape properties cannot be driven on a selection of vertices",
                    )
                    .help("drive them on the shape itself"));
                }
                (sn, vids)
            } else if let Value::Shape(sn) = &obj {
                let vids = if vert_eqs.is_empty() {
                    vec![]
                } else {
                    self.shapes[sn].live_ids(Kind::V)
                };
                (sn.clone(), vids)
            } else {
                return Err(mth(
                    "E-NAM-003",
                    format!("a {} has no operation `drive`", obj.type_name()),
                ));
            };

        let scene = self.scene_name.clone();
        let seed = self.seed32(&format!("{scene}/{shape}/drive"));
        let (names, props): (Vec<&str>, Vec<Vec<PropId>>) = if shape_eqs.is_empty() {
            (
                vert_eqs.clone(),
                vert_eqs
                    .iter()
                    .map(|a| {
                        verts
                            .iter()
                            .map(|v| {
                                if *a == "x" {
                                    PropId::Vx(*v)
                                } else {
                                    PropId::Vy(*v)
                                }
                            })
                            .collect()
                    })
                    .collect(),
            )
        } else {
            (
                shape_eqs.clone(),
                shape_eqs
                    .iter()
                    .map(|k| match *k {
                        "opacity" => vec![PropId::Opacity],
                        "width" => vec![PropId::ShapeWidth],
                        "rotation" => vec![PropId::Rot],
                        "scale" => vec![PropId::ScaleX, PropId::ScaleY],
                        "pos_x" => vec![PropId::PosX],
                        _ => vec![PropId::PosY],
                    })
                    .collect(),
            )
        };
        let eqs: Vec<(&str, &Value)> = names.iter().map(|k| (*k, &m[k])).collect();
        let prog = self.compile_equations(&eqs, &DRIVE_POINT, &DRIVE_FRAME, &consts, seed)?;
        let prog = self.make_kernel(prog, verts.len());
        let drive = Drive {
            prog,
            verts,
            outs: props.into_iter().map(|props| DriveOut { props }).collect(),
            replace,
            speed,
        };
        Ok(Value::Modifier(Box::new(ModReq {
            label: format!("{shape}.drive"),
            shape,
            kind: ModKind::Drive,
            comps: vec![],
            frames,
            blend_in,
            blend_out,
            drive: Some(Rc::new(drive)),
        })))
    }
}

/// Marching squares over a lattice of `(res + 1)^2` samples (row-major, x fastest): the contour of `f = 0` (negative
/// inside) as points and index pairs. Crossings are shared between neighbouring cells, so contours are connected;
/// cells are scanned in a fixed order, so the output is deterministic.
fn marching_squares(
    xs: &[f64],
    ys: &[f64],
    f: &[f64],
    res: usize,
) -> (Vec<[f64; 2]>, Vec<(usize, usize)>) {
    let w = res + 1;
    let at = |i: usize, j: usize| j * w + i;
    let mut points: Vec<[f64; 2]> = vec![];
    // (i, j, 0) = the horizontal lattice edge from (i, j) to (i + 1, j); (i, j, 1) the vertical one to (i, j + 1)
    let mut ids: BTreeMap<(usize, usize, u8), usize> = BTreeMap::new();
    let mut cross = |i: usize, j: usize, v: u8, points: &mut Vec<[f64; 2]>| -> usize {
        *ids.entry((i, j, v)).or_insert_with(|| {
            let (a, b) = if v == 0 {
                (at(i, j), at(i + 1, j))
            } else {
                (at(i, j), at(i, j + 1))
            };
            let t = f[a] / (f[a] - f[b]);
            points.push([xs[a] + (xs[b] - xs[a]) * t, ys[a] + (ys[b] - ys[a]) * t]);
            points.len() - 1
        })
    };
    let mut pairs = vec![];
    for j in 0..res {
        for i in 0..res {
            let c = [
                f[at(i, j)],
                f[at(i + 1, j)],
                f[at(i + 1, j + 1)],
                f[at(i, j + 1)],
            ];
            let inside = |k: usize| c[k] < 0.0;
            let case = inside(0) as usize
                | (inside(1) as usize) << 1
                | (inside(2) as usize) << 2
                | (inside(3) as usize) << 3;
            // edges: 0 bottom, 1 right, 2 top, 3 left
            let centre_in = (c[0] + c[1] + c[2] + c[3]) / 4.0 < 0.0;
            let segs: &[(u8, u8)] = match case {
                1 | 14 => &[(3, 0)],
                2 | 13 => &[(0, 1)],
                3 | 12 => &[(3, 1)],
                4 | 11 => &[(1, 2)],
                6 | 9 => &[(0, 2)],
                7 | 8 => &[(2, 3)],
                5 => {
                    if centre_in {
                        &[(0, 1), (2, 3)]
                    } else {
                        &[(3, 0), (1, 2)]
                    }
                }
                10 => {
                    if centre_in {
                        &[(3, 0), (1, 2)]
                    } else {
                        &[(0, 1), (2, 3)]
                    }
                }
                _ => &[],
            };
            for (a, b) in segs {
                let mut id = |e: u8| match e {
                    0 => cross(i, j, 0, &mut points),
                    1 => cross(i + 1, j, 1, &mut points),
                    2 => cross(i, j + 1, 0, &mut points),
                    _ => cross(i, j, 1, &mut points),
                };
                let (pa, pb) = (id(*a), id(*b));
                pairs.push((pa, pb));
            }
        }
    }
    (points, pairs)
}
