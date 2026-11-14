//! Fields: user equations stepped over a grid of cells (or along a chain of a shape's vertices), deterministically.
//!
//! ```anim
//! field pond:
//!     grid: (160, 120)
//!     rect: (100, 100, 640, 480)
//!     state: ["h", "v"]
//!     init: [("h", "exp(-((x - 400)^2 + (y - 300)^2) / 400)"), ("v", "0")]
//!     step: [[("v", "v + dt * c2 * (h[-1, 0] + h[1, 0] + h[0, -1] + h[0, 1] - 4 * h)")], [("h", "h + dt * v")]]
//!     c2: 900
//!     edges: "mirror"
//!     substeps: 4
//! ```
//!
//! A *phase* is a set of equations that all read the state as it was before the phase; the next phase reads the new
//! state. The equations are compiled by the math engine (one scalar program per equation, run over all cells at once,
//! as machine code for big grids when the `jit` feature is on); neighbour reads `h[dx, dy]` become shifted copies of
//! the channel with the edge rule applied. Everything is double buffered and single threaded, so the result depends
//! on nothing but the script.
use crate::interp::{Interp, Mode, Vars};
use crate::value::*;
use animscript_core::*;
use animscript_math as m;
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, HashMap};

type Args = Vec<(Option<String>, Value)>;

const MAX_SIDE: i64 = 512;
const MAX_CELLS: i64 = 512 * 512;
const MAX_SUBSTEPS: i64 = 64;
const IMAGE_SIDE: usize = 256;

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-FLD-001", msg)
}
fn e2(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-FLD-002", msg)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Clamp,
    Wrap,
    Mirror,
    Zero,
}

/// What a field's declaration says (checked while building).
#[derive(Clone, Debug)]
pub struct FieldCfg {
    pub nx: usize,
    pub ny: usize,
    pub rect: [f64; 4],
    /// A chain field has one cell per live vertex of this shape (in id order) and no rows.
    pub chain: Option<String>,
    pub channels: Vec<String>,
    pub init: Vec<(usize, Value)>,
    pub phases: Vec<Vec<(usize, Value)>>,
    pub consts: BTreeMap<String, f64>,
    pub edges: Edge,
    pub substeps: u32,
    pub ramp: Vec<[u8; 4]>,
    pub show: usize,
    pub range: (f64, f64),
    pub z: i64,
    pub opacity: f64,
    pub blend: String,
    pub raw: bool,
}

#[derive(Clone, Copy, Debug)]
enum Src {
    X,
    Y,
    I,
    J,
    Chan(usize, i32, i32),
}

struct Compiled {
    channel: usize,
    kernel: Kernel,
    srcs: Vec<Src>,
}

/// The running state of one field (rebuilt at the start of every scene).
pub struct FieldRt {
    pub nx: usize,
    pub ny: usize,
    pub data: Vec<Vec<f64>>,
    xs: Vec<f64>,
    ys: Vec<f64>,
    is: Vec<f64>,
    js: Vec<f64>,
    phases: Vec<Vec<Compiled>>,
    time: f64,
}

impl std::fmt::Debug for FieldRt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FieldRt({}x{})", self.nx, self.ny)
    }
}

fn resolve(i: i64, n: usize, edge: Edge) -> Option<usize> {
    let n = n as i64;
    match edge {
        Edge::Clamp => Some(i.clamp(0, n - 1) as usize),
        Edge::Wrap => Some(i.rem_euclid(n) as usize),
        Edge::Zero => (0..n).contains(&i).then_some(i as usize),
        Edge::Mirror => {
            let mut i = i;
            while i < 0 || i >= n {
                i = if i < 0 { -i - 1 } else { 2 * n - i - 1 };
            }
            Some(i as usize)
        }
    }
}

/// `data` read at the cell `(i + dx, j + dy)` for every cell, with the edge rule for what lies outside.
fn shifted(data: &[f64], nx: usize, ny: usize, dx: i32, dy: i32, edge: Edge) -> Vec<f64> {
    let mut out = Vec::with_capacity(data.len());
    for j in 0..ny {
        let sj = resolve(j as i64 + dy as i64, ny, edge);
        for i in 0..nx {
            let si = resolve(i as i64 + dx as i64, nx, edge);
            out.push(match (si, sj) {
                (Some(si), Some(sj)) => data[sj * nx + si],
                _ => 0.0,
            });
        }
    }
    out
}

impl Interp {
    // ---- the declaration -----------------------------------------------------------------------------------------

    pub fn eval_fields(&mut self) -> Res<()> {
        let decls = self.program.fields.clone();
        let vars = Vars {
            scopes: vec![self.globals.clone()],
        };
        for d in &decls {
            if self.fields.contains_key(&d.name) {
                return Err(e1(format!("there is already a field named `{}`", d.name)).at(d.span));
            }
            let mut cfg = FieldCfg {
                nx: 0,
                ny: 0,
                rect: [0.0, 0.0, 0.0, 0.0],
                chain: None,
                channels: vec![],
                init: vec![],
                phases: vec![],
                consts: BTreeMap::new(),
                edges: Edge::Clamp,
                substeps: 1,
                ramp: vec![],
                show: 0,
                range: (0.0, 1.0),
                z: 0,
                opacity: 1.0,
                blend: "normal".into(),
                raw: false,
            };
            let mut raw_state: Option<Vec<String>> = None;
            let (mut init_v, mut step_v, mut show_ch) = (None, None, None);
            let mut grid = None;
            for (key, expr, span) in &d.attrs {
                let v = self.eval(expr, &vars, Mode::Build)?;
                let at = |e: Diagnostic| e.at(*span);
                match key.as_str() {
                    "grid" => {
                        let (Value::Tuple(t) | Value::List(t)) = &v else {
                            return Err(
                                e2("`grid` must be two whole numbers, like `(160, 120)`").at(*span)
                            );
                        };
                        if t.len() != 2 {
                            return Err(
                                e2("`grid` must be two whole numbers, like `(160, 120)`").at(*span)
                            );
                        }
                        let (nx, ny) = (
                            self.int(&t[0], "`grid`").map_err(at)?,
                            self.int(&t[1], "`grid`").map_err(at)?,
                        );
                        if !(1..=MAX_SIDE).contains(&nx)
                            || !(1..=MAX_SIDE).contains(&ny)
                            || nx * ny > MAX_CELLS
                        {
                            return Err(e2(format!(
                                "a grid has 1 to {MAX_SIDE} cells on each side, found {nx} x {ny}"
                            ))
                            .at(*span)
                            .help("512 x 512 is the most"));
                        }
                        grid = Some((nx as usize, ny as usize));
                    }
                    "rect" => {
                        let (Value::Tuple(t) | Value::List(t)) = &v else {
                            return Err(e1("`rect` must be (x, y, width, height)").at(*span));
                        };
                        if t.len() != 4 {
                            return Err(e1("`rect` must be (x, y, width, height)").at(*span));
                        }
                        for (k, x) in t.iter().enumerate() {
                            cfg.rect[k] = self.num(x, "`rect`").map_err(at)?;
                        }
                        if cfg.rect[2] <= 0.0 || cfg.rect[3] <= 0.0 {
                            return Err(
                                e1("`rect` must have a positive width and height").at(*span)
                            );
                        }
                    }
                    "chain" => match &v {
                        Value::Shape(s) => cfg.chain = Some(s.clone()),
                        o => {
                            return Err(e1(format!(
                                "`chain` must be a shape, found {}",
                                o.type_name()
                            ))
                            .at(*span))
                        }
                    },
                    "state" => {
                        let Value::List(items) = &v else {
                            return Err(e1(
                                "`state` must be a list of channel names, like `[\"h\", \"v\"]`",
                            )
                            .at(*span));
                        };
                        let mut names = vec![];
                        for it in items {
                            let Value::Str(s) = it else {
                                return Err(e1("a channel name must be a string").at(*span));
                            };
                            if names.contains(s) {
                                return Err(
                                    e1(format!("the channel `{s}` is listed twice")).at(*span)
                                );
                            }
                            if matches!(s.as_str(), "x" | "y" | "i" | "j" | "t" | "dt")
                                || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                                || s.is_empty()
                            {
                                return Err(e1(format!("`{s}` cannot be a channel name")).at(*span).help("channels are plain names; x y i j t dt are inputs of the equations"));
                            }
                            names.push(s.clone());
                        }
                        if names.is_empty() {
                            return Err(e1("`state` needs at least one channel").at(*span));
                        }
                        raw_state = Some(names);
                    }
                    "init" => init_v = Some((v, *span)),
                    "step" => step_v = Some((v, *span)),
                    "edges" => {
                        cfg.edges =
                            match &v {
                                Value::Str(s) if s == "clamp" => Edge::Clamp,
                                Value::Str(s) if s == "wrap" => Edge::Wrap,
                                Value::Str(s) if s == "mirror" => Edge::Mirror,
                                Value::Str(s) if s == "zero" => Edge::Zero,
                                _ => return Err(e1(
                                    "`edges` must be \"clamp\", \"wrap\", \"mirror\" or \"zero\"",
                                )
                                .at(*span)),
                            }
                    }
                    "substeps" => {
                        let n = self.int(&v, "`substeps`").map_err(at)?;
                        if !(1..=MAX_SUBSTEPS).contains(&n) {
                            return Err(
                                e2(format!("`substeps` must be 1 to {MAX_SUBSTEPS}")).at(*span)
                            );
                        }
                        cfg.substeps = n as u32;
                    }
                    "ramp" => {
                        let Value::List(items) = &v else {
                            return Err(
                                e1("`ramp` must be a list of at least two colours").at(*span)
                            );
                        };
                        let mut cols = vec![];
                        for c in items {
                            let Value::Color(c) = c else {
                                return Err(e1(
                                    "`ramp` must be a list of colours like `[#103a6b, #ffffff]`",
                                )
                                .at(*span));
                            };
                            cols.push(*c);
                        }
                        if cols.len() < 2 {
                            return Err(e1("`ramp` needs at least two colours").at(*span));
                        }
                        cfg.ramp = cols;
                    }
                    "channel" => match &v {
                        Value::Str(s) => show_ch = Some((s.clone(), *span)),
                        _ => return Err(e1("`channel` must be a channel name").at(*span)),
                    },
                    "range" => {
                        let (Value::Tuple(t) | Value::List(t)) = &v else {
                            return Err(e1("`range` must be (low, high)").at(*span));
                        };
                        if t.len() != 2 {
                            return Err(e1("`range` must be (low, high)").at(*span));
                        }
                        cfg.range = (
                            self.num(&t[0], "`range`").map_err(at)?,
                            self.num(&t[1], "`range`").map_err(at)?,
                        );
                        if cfg.range.1 <= cfg.range.0 {
                            return Err(
                                e1("`range` must be (low, high) with high above low").at(*span)
                            );
                        }
                    }
                    "z" => cfg.z = self.int(&v, "`z`").map_err(at)?,
                    "opacity" => {
                        cfg.opacity = self.num(&v, "`opacity`").map_err(at)?.clamp(0.0, 1.0)
                    }
                    "blend" => match &v {
                        Value::Str(s) => cfg.blend = s.clone(),
                        _ => return Err(e1("`blend` must be a blend mode name").at(*span)),
                    },
                    "raw" => match &v {
                        Value::Bool(b) => cfg.raw = *b,
                        _ => return Err(e1("`raw` must be true or false").at(*span)),
                    },
                    other => {
                        // any other name with a number is a constant of the equations
                        match &v {
                            Value::Int(_) | Value::Num(_) | Value::Dur(_) | Value::Angle(_) | Value::Bool(_) => {
                                if matches!(other, "x" | "y" | "i" | "j" | "t" | "dt") {
                                    return Err(e1(format!("`{other}` is an input of the equations and cannot be a constant")).at(*span));
                                }
                                let c = match &v {
                                    Value::Dur(r) => r.to_f64(),
                                    Value::Angle(a) => a * std::f64::consts::TAU,
                                    Value::Bool(b) => f64::from(*b),
                                    o => self.num(o, other).map_err(at)?,
                                };
                                cfg.consts.insert(other.to_string(), c);
                            }
                            _ => {
                                return Err(e1(format!("a field has no setting `{other}`"))
                                    .at(*span)
                                    .help("settings: grid, rect, chain, state, init, step, edges, substeps, ramp, channel, range, z, opacity, blend, raw; any other name with a number is a constant"))
                            }
                        }
                    }
                }
            }
            // the shape of the field
            if let Some(chain) = cfg.chain.clone() {
                let Some(sh) = self.shapes.get(&chain) else {
                    return Err(e1(format!("`chain` names no shape `{chain}`")).at(d.span));
                };
                let n = sh.live_ids(Kind::V).len();
                if n < 2 {
                    return Err(e1("a chain needs a shape with at least two vertices").at(d.span));
                }
                (cfg.nx, cfg.ny) = (n, 1);
                if grid.is_some() {
                    return Err(e1("a field has a `grid` or a `chain`, not both").at(d.span));
                }
            } else {
                let Some((nx, ny)) = grid else {
                    return Err(e1("a field needs a `grid` (or a `chain`)")
                        .at(d.span)
                        .help("for example `grid: (160, 120)` and `rect: (0, 0, 800, 600)`"));
                };
                if cfg.rect[2] <= 0.0 {
                    return Err(e1("a grid field needs a `rect: (x, y, width, height)`").at(d.span));
                }
                (cfg.nx, cfg.ny) = (nx, ny);
            }
            let Some(channels) = raw_state else {
                return Err(e1("a field needs `state: [\"name\", ...]`").at(d.span));
            };
            cfg.channels = channels;
            let chan = |name: &str, span: Span| -> Res<usize> {
                cfg.channels.iter().position(|c| c == name).ok_or_else(|| {
                    e1(format!("`{name}` is not a channel of this field"))
                        .at(span)
                        .help(format!("its channels are: {}", cfg.channels.join(", ")))
                })
            };
            let pair = |v: &Value, span: Span| -> Res<(usize, Value)> {
                match v {
                    Value::Tuple(t) if t.len() == 2 => match &t[0] {
                        Value::Str(c) => Ok((chan(c, span)?, t[1].clone())),
                        _ => Err(
                            e1("an equation is written as (\"channel\", \"expression\")").at(span),
                        ),
                    },
                    _ => {
                        Err(e1("an equation is written as (\"channel\", \"expression\")").at(span))
                    }
                }
            };
            if let Some((v, span)) = init_v {
                let Value::List(items) = &v else {
                    return Err(
                        e1("`init` must be a list of (\"channel\", \"expression\")").at(span)
                    );
                };
                for it in items {
                    let (c, eq) = pair(it, span)?;
                    cfg.init.push((c, eq));
                }
            }
            if let Some((v, span)) = step_v {
                let Value::List(items) = &v else {
                    return Err(e1("`step` must be a list of equations and phases").at(span));
                };
                let mut pending: Vec<(usize, Value)> = vec![];
                for it in items {
                    match it {
                        Value::List(phase) => {
                            if !pending.is_empty() {
                                cfg.phases.push(std::mem::take(&mut pending));
                            }
                            cfg.phases
                                .push(phase.iter().map(|p| pair(p, span)).collect::<Res<_>>()?);
                        }
                        other => pending.push(pair(other, span)?),
                    }
                }
                if !pending.is_empty() {
                    cfg.phases.push(pending);
                }
                for ph in &cfg.phases {
                    let mut seen = vec![];
                    for (c, _) in ph {
                        if seen.contains(c) {
                            return Err(e1(format!(
                                "the channel `{}` is set twice in one phase",
                                cfg.channels[*c]
                            ))
                            .at(span));
                        }
                        seen.push(*c);
                    }
                }
            }
            if let Some((name, span)) = show_ch {
                cfg.show = chan(&name, span)?;
            }
            self.fields.insert(d.name.clone(), cfg);
        }
        Ok(())
    }

    // ---- the start of a scene: initial state ------------------------------------------------------------------------

    fn compile_field_eq(
        &mut self,
        fname: &str,
        cfg: &FieldCfg,
        channel: usize,
        eq: &Value,
        what: &str,
    ) -> Res<Compiled> {
        let mut point: Vec<&str> = vec!["x", "y", "i", "j"];
        for c in &cfg.channels {
            point.push(c);
        }
        self.eq_stencils = cfg.channels.iter().cloned().collect();
        let seed = self.seed32(&format!("{}/field/{fname}/{what}", self.scene_name));
        let label = format!("{fname}.{}", cfg.channels[channel]);
        let prog = self.compile_equations(
            &[(label.as_str(), eq)],
            &point,
            &["t", "dt"],
            &cfg.consts,
            seed,
        );
        self.eq_stencils.clear();
        let prog = prog?;
        let mut srcs = vec![];
        for (name, rate) in prog.input_decls() {
            if rate == m::Rate::Frame {
                continue;
            }
            srcs.push(match name {
                "x" => Src::X,
                "y" => Src::Y,
                "i" => Src::I,
                "j" => Src::J,
                n => {
                    let (base, off) = match n.split_once('@') {
                        Some((b, o)) => (b, o),
                        None => (n, "0,0"),
                    };
                    let c = cfg.channels.iter().position(|c| c == base).expect("only channels are indexed");
                    let (dx, dy) = off.split_once(',').map(|(a, b)| (a.parse::<i32>().unwrap_or(0), b.parse::<i32>().unwrap_or(0))).unwrap_or((0, 0));
                    if dy != 0 && cfg.ny == 1 {
                        return Err(e1(format!("`{base}[{dx}, {dy}]` reads another row, but this field has only one"))
                            .help("a one-dimensional field (a chain, or a grid with one row) can only read neighbours along x"));
                    }
                    Src::Chan(c, dx, dy)
                }
            });
        }
        let cells = cfg.nx * cfg.ny;
        let kernel = self.make_kernel(prog, cells);
        Ok(Compiled {
            channel,
            kernel,
            srcs,
        })
    }

    /// A fresh state for every field: coordinates, the initial equations evaluated, the step equations compiled.
    pub fn reset_fields(&mut self) -> Res<()> {
        self.field_rt.clear();
        let names: Vec<String> = self.fields.keys().cloned().collect();
        for name in names {
            let mut cfg = self.fields[&name].clone();
            let (xs, ys): (Vec<f64>, Vec<f64>) = if let Some(chain) = &cfg.chain {
                let sh = &self.shapes[chain];
                let ids = sh.live_ids(Kind::V);
                if ids.len() != cfg.nx {
                    return Err(e1(format!("the chain `{chain}` has {} vertices now, but had {} when `{name}` was declared", ids.len(), cfg.nx)));
                }
                ids.iter().map(|v| (sh.verts[v].x, sh.verts[v].y)).unzip()
            } else {
                let (cw, ch) = (cfg.rect[2] / cfg.nx as f64, cfg.rect[3] / cfg.ny as f64);
                let mut xs = Vec::with_capacity(cfg.nx * cfg.ny);
                let mut ys = Vec::with_capacity(cfg.nx * cfg.ny);
                for j in 0..cfg.ny {
                    for i in 0..cfg.nx {
                        xs.push(cfg.rect[0] + (i as f64 + 0.5) * cw);
                        ys.push(cfg.rect[1] + (j as f64 + 0.5) * ch);
                    }
                }
                (xs, ys)
            };
            let n = cfg.nx * cfg.ny;
            let is: Vec<f64> = (0..n).map(|k| (k % cfg.nx) as f64).collect();
            let js: Vec<f64> = (0..n).map(|k| (k / cfg.nx) as f64).collect();
            let mut rt = FieldRt {
                nx: cfg.nx,
                ny: cfg.ny,
                data: vec![vec![0.0; n]; cfg.channels.len()],
                xs,
                ys,
                is,
                js,
                phases: vec![],
                time: 0.0,
            };
            let dt0 = 1.0 / (self.fps.to_f64() * cfg.substeps as f64);
            // the initial state: each equation sees the zero state of the others
            let inits: Vec<Compiled> = cfg
                .init
                .clone()
                .iter()
                .map(|(c, eq)| self.compile_field_eq(&name, &cfg, *c, eq, "init"))
                .collect::<Res<_>>()?;
            for comp in &inits {
                let out = Self::run_compiled(&rt, &cfg, comp, &mut HashMap::new(), 0.0, dt0);
                self.check_finite(&name, &cfg, comp.channel, &out, 0)?;
                rt.data[comp.channel] = out;
            }
            for ph in cfg.phases.clone() {
                let comps: Vec<Compiled> = ph
                    .iter()
                    .map(|(c, eq)| self.compile_field_eq(&name, &cfg, *c, eq, "step"))
                    .collect::<Res<_>>()?;
                rt.phases.push(comps);
            }
            cfg.nx = rt.nx;
            self.field_rt.insert(name, rt);
        }
        Ok(())
    }

    fn run_compiled(
        rt: &FieldRt,
        cfg: &FieldCfg,
        comp: &Compiled,
        cache: &mut HashMap<(usize, i32, i32), Vec<f64>>,
        t: f64,
        dt: f64,
    ) -> Vec<f64> {
        for s in &comp.srcs {
            if let Src::Chan(c, dx, dy) = *s {
                if dx != 0 || dy != 0 {
                    cache
                        .entry((c, dx, dy))
                        .or_insert_with(|| shifted(&rt.data[c], rt.nx, rt.ny, dx, dy, cfg.edges));
                }
            }
        }
        let cols: Vec<&[f64]> = comp
            .srcs
            .iter()
            .map(|s| match *s {
                Src::X => rt.xs.as_slice(),
                Src::Y => rt.ys.as_slice(),
                Src::I => rt.is.as_slice(),
                Src::J => rt.js.as_slice(),
                Src::Chan(c, 0, 0) => rt.data[c].as_slice(),
                Src::Chan(c, dx, dy) => cache[&(c, dx, dy)].as_slice(),
            })
            .collect();
        comp.kernel
            .eval_n(rt.nx * rt.ny, &cols, &[t, dt], &[])
            .expect("the compiled field inputs match")
            .remove(0)
    }

    fn check_finite(
        &self,
        name: &str,
        cfg: &FieldCfg,
        channel: usize,
        vals: &[f64],
        tick: i64,
    ) -> Res<()> {
        if let Some(k) = vals.iter().position(|v| !v.is_finite()) {
            let (i, j) = (k % cfg.nx, k / cfg.nx);
            return Err(Diagnostic::new(
                "E-FLD-003",
                format!("the field `{name}` channel `{}` became {} at cell ({i}, {j}) in tick {tick}", cfg.channels[channel], vals[k]),
            )
            .help("the time step is probably too large for the equation: lower its coefficients or raise `substeps`"));
        }
        Ok(())
    }

    // ---- stepping ---------------------------------------------------------------------------------------------------

    /// Advance every field by one tick (not at tick 0, which shows the initial state).
    pub fn step_fields(&mut self, tick: i64) -> Res<()> {
        if self.field_rt.is_empty() || tick == 0 {
            return Ok(());
        }
        let names: Vec<String> = self.field_rt.keys().cloned().collect();
        for name in names {
            let cfg = self.fields[&name].clone();
            let mut rt = self
                .field_rt
                .remove(&name)
                .expect("a runtime for every field");
            let dt = 1.0 / (self.fps.to_f64() * cfg.substeps as f64);
            for _ in 0..cfg.substeps {
                rt.time += dt;
                for p in 0..rt.phases.len() {
                    let mut cache = HashMap::new();
                    let results: Vec<(usize, Vec<f64>)> = rt.phases[p]
                        .iter()
                        .map(|comp| {
                            (
                                comp.channel,
                                Self::run_compiled(&rt, &cfg, comp, &mut cache, rt.time, dt),
                            )
                        })
                        .collect();
                    for (c, vals) in results {
                        if let Err(e) = self.check_finite(&name, &cfg, c, &vals, tick) {
                            self.field_rt.insert(name, rt);
                            return Err(e);
                        }
                        rt.data[c] = vals;
                    }
                }
            }
            self.field_rt.insert(name, rt);
        }
        Ok(())
    }

    // ---- reading and writing from scripts -----------------------------------------------------------------------------

    fn channel_arg(&self, name: &str, m: &HashMap<&'static str, Value>) -> Res<usize> {
        let cfg = &self.fields[name];
        match m.get("channel") {
            None => Ok(0),
            Some(Value::Str(c)) => cfg.channels.iter().position(|x| x == c).ok_or_else(|| {
                e1(format!("`{c}` is not a channel of `{name}`"))
                    .help(format!("its channels are: {}", cfg.channels.join(", ")))
            }),
            Some(o) => Err(e1(format!(
                "`channel` must be a channel name, found {}",
                o.type_name()
            ))),
        }
    }

    pub fn field_method(&mut self, name: &str, method: &str, args: Args) -> Res<Value> {
        let Some(rt) = self.field_rt.get(name) else {
            return Err(e1(format!("the field `{name}` has no state yet"))
                .help("fields exist while a scene runs: use them in `setup:` or `main:`"));
        };
        let (nx, ny) = (rt.nx, rt.ny);
        match method {
            "poke" => {
                let m = self.bind(
                    method,
                    args,
                    &["x", "y", "amount", "radius", "channel"],
                    &["x", "y"],
                )?;
                let (x, y) = (self.num(&m["x"], "`x`")?, self.num(&m["y"], "`y`")?);
                let amount = m
                    .get("amount")
                    .map(|v| self.num(v, "`amount`"))
                    .unwrap_or(Ok(1.0))?;
                let cfg = &self.fields[name];
                let cell = (cfg.rect[2] / nx as f64).max(cfg.rect[3] / ny as f64);
                let radius = m
                    .get("radius")
                    .map(|v| self.num(v, "`radius`"))
                    .unwrap_or(Ok(cell * 2.0))?;
                if radius <= 0.0 {
                    return Err(e1("`radius` must be positive"));
                }
                let c = self.channel_arg(name, &m)?;
                let rt = self.field_rt.get_mut(name).expect("checked");
                for k in 0..nx * ny {
                    let d2 = (rt.xs[k] - x).powi(2) + (rt.ys[k] - y).powi(2);
                    if d2 < radius * radius {
                        let f = 1.0 - d2 / (radius * radius);
                        rt.data[c][k] += amount * f * f;
                    }
                }
                Ok(Value::None)
            }
            "fill" => {
                let m = self.bind(method, args, &["value", "channel"], &["value"])?;
                let v = self.num(&m["value"], "`value`")?;
                let c = self.channel_arg(name, &m)?;
                for cell in self.field_rt.get_mut(name).expect("checked").data[c].iter_mut() {
                    *cell = v;
                }
                Ok(Value::None)
            }
            "at" => {
                let m = self.bind(method, args, &["x", "y", "channel"], &["x", "y"])?;
                let (x, y) = (self.num(&m["x"], "`x`")?, self.num(&m["y"], "`y`")?);
                let c = self.channel_arg(name, &m)?;
                Ok(Value::Num(self.field_sample(name, c, x, y)))
            }
            "sum" | "max" | "min" => {
                let m = self.bind(method, args, &["channel"], &[])?;
                let c = self.channel_arg(name, &m)?;
                let d = &self.field_rt[name].data[c];
                Ok(Value::Num(match method {
                    "sum" => d.iter().sum(),
                    "max" => d.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    _ => d.iter().cloned().fold(f64::INFINITY, f64::min),
                }))
            }
            other => Err(Diagnostic::new(
                "E-NAM-003",
                format!("a field has no operation `{other}`"),
            )
            .help("operations: poke, fill, at, sum, max, min")),
        }
    }

    /// Add `amount` to a channel around `(x, y)`: the falloff `poke` uses.
    pub(crate) fn field_add(
        &mut self,
        name: &str,
        channel: usize,
        x: f64,
        y: f64,
        amount: f64,
        radius: f64,
    ) {
        let Some(rt) = self.field_rt.get_mut(name) else {
            return;
        };
        let (nx, ny) = (rt.nx, rt.ny);
        for k in 0..nx * ny {
            let d2 = (rt.xs[k] - x).powi(2) + (rt.ys[k] - y).powi(2);
            if d2 < radius * radius {
                let f = 1.0 - d2 / (radius * radius);
                // a ripple is never taller than the water that made it can be
                rt.data[channel][k] = (rt.data[channel][k] + amount * f * f).clamp(-150.0, 150.0);
            }
        }
    }

    /// The value of a channel at a point: bilinear between cell centres (a chain: linear along its x).
    pub fn field_sample(&self, name: &str, channel: usize, x: f64, y: f64) -> f64 {
        let (cfg, rt) = (&self.fields[name], &self.field_rt[name]);
        let d = &rt.data[channel];
        if cfg.chain.is_some() {
            let n = rt.nx;
            if x <= rt.xs[0] {
                return d[0];
            }
            for k in 1..n {
                if x <= rt.xs[k] {
                    let t = (x - rt.xs[k - 1]) / (rt.xs[k] - rt.xs[k - 1]).max(1e-12);
                    return d[k - 1] + (d[k] - d[k - 1]) * t;
                }
            }
            return d[n - 1];
        }
        let (cw, ch) = (cfg.rect[2] / rt.nx as f64, cfg.rect[3] / rt.ny as f64);
        let fx = ((x - cfg.rect[0]) / cw - 0.5).clamp(0.0, (rt.nx - 1) as f64);
        let fy = ((y - cfg.rect[1]) / ch - 0.5).clamp(0.0, (rt.ny - 1) as f64);
        let (i0, j0) = (fx.floor() as usize, fy.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(rt.nx - 1), (j0 + 1).min(rt.ny - 1));
        let (tx, ty) = (fx - i0 as f64, fy - j0 as f64);
        let at = |i: usize, j: usize| d[j * rt.nx + i];
        let top = at(i0, j0) + (at(i1, j0) - at(i0, j0)) * tx;
        let bottom = at(i0, j1) + (at(i1, j1) - at(i0, j1)) * tx;
        top + (bottom - top) * ty
    }

    /// The equation inputs that read fields: `<field>_<channel>` for every channel and `<field>` for the first one.
    /// Returns (input name, field, channel) for the fields listed in a `fields: [...]` value.
    pub fn field_inputs(&self, v: &Value, what: &str) -> Res<Vec<(String, String, usize)>> {
        let Value::List(items) = v else {
            return Err(Diagnostic::new(
                "E-MTH-003",
                format!(
                    "`fields` must be a list of fields, like `fields: [pond]`, found {}",
                    v.type_name()
                ),
            ));
        };
        let mut out: Vec<(String, String, usize)> = vec![];
        for it in items {
            let Value::Field(f) = it else {
                return Err(Diagnostic::new(
                    "E-MTH-003",
                    format!(
                        "`fields` in {what} must list fields, found a {}",
                        it.type_name()
                    ),
                ));
            };
            if out.iter().any(|o| &o.1 == f) {
                continue;
            }
            let cfg = &self.fields[f];
            for (c, ch) in cfg.channels.iter().enumerate() {
                out.push((format!("{f}_{ch}"), f.clone(), c));
            }
            out.push((f.clone(), f.clone(), 0));
        }
        Ok(out)
    }

    pub fn has_fields(&self) -> bool {
        !self.field_rt.is_empty()
    }

    // ---- the snapshot --------------------------------------------------------------------------------------------------

    pub fn field_snapshot(&self) -> Option<J> {
        if self.field_rt.is_empty() {
            return None;
        }
        let mut out = vec![];
        for (name, rt) in &self.field_rt {
            let cfg = &self.fields[name];
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            let mut stats = serde_json::Map::new();
            for (c, ch) in cfg.channels.iter().enumerate() {
                let (mut sum, mut lo, mut hi) = (0.0f64, f64::INFINITY, f64::NEG_INFINITY);
                for v in &rt.data[c] {
                    sum += v;
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                    for b in v.to_bits().to_le_bytes() {
                        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
                stats.insert(ch.clone(), json!({"sum": sum, "min": lo, "max": hi}));
            }
            let mut f = json!({
                "name": name, "nx": rt.nx, "ny": rt.ny, "rect": cfg.rect, "z": cfg.z, "opacity": cfg.opacity,
                "blend": cfg.blend, "checksum": format!("{h:016x}"), "channels": cfg.channels, "stats": stats,
            });
            if !cfg.ramp.is_empty() && cfg.chain.is_none() {
                let (iw, ih) = (rt.nx.min(IMAGE_SIDE), rt.ny.min(IMAGE_SIDE));
                let d = &rt.data[cfg.show];
                let mut px = Vec::with_capacity(iw * ih * 4);
                for j in 0..ih {
                    for i in 0..iw {
                        let (si, sj) = (i * rt.nx / iw, j * rt.ny / ih);
                        px.extend_from_slice(&ramp_colour(
                            &cfg.ramp,
                            (d[sj * rt.nx + si] - cfg.range.0) / (cfg.range.1 - cfg.range.0),
                        ));
                    }
                }
                f["iw"] = json!(iw);
                f["ih"] = json!(ih);
                f["rgba"] = json!(base64(&px));
            }
            if cfg.raw {
                let mut data = serde_json::Map::new();
                for (c, ch) in cfg.channels.iter().enumerate() {
                    data.insert(ch.clone(), json!(rt.data[c]));
                }
                f["data"] = J::Object(data);
            }
            out.push(f);
        }
        Some(J::Array(out))
    }
}

fn ramp_colour(ramp: &[[u8; 4]], t: f64) -> [u8; 4] {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let x = t * (ramp.len() - 1) as f64;
    let i = (x.floor() as usize).min(ramp.len() - 2);
    let f = x - i as f64;
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    [
        mix(ramp[i][0], ramp[i + 1][0]),
        mix(ramp[i][1], ramp[i + 1][1]),
        mix(ramp[i][2], ramp[i + 1][2]),
        mix(ramp[i][3], ramp[i + 1][3]),
    ]
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
