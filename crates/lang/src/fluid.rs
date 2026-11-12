//! Fluids in the language: the `fluid name:` block, `pour` / `stop` / `add`, the reads `count` and
//! `level`, and the coupled stepping with soft bodies and the rigid world. Particles live in metres
//! in the engine; everything the user sees is in pixels.
use crate::color::hex;
use crate::interp::{Interp, Mode, Vars};
use crate::value::*;
use animscript_core::*;
use animscript_physics as ph;
use ph::{v2, Vec2};
use serde_json::{json, Value as J};

type Args = Vec<(Option<String>, Value)>;

const HARD_LIMIT: usize = 20_000;

#[derive(Clone, Debug)]
pub enum Region {
    Rect([f64; 4]),
    Shape(String),
}

#[derive(Clone, Debug)]
pub struct FluidCfg {
    pub params: ph::FluidParams,
    pub spacing_px: f64,
    pub fill: Option<Region>,
    pub color: [u8; 4],
    pub color_end: [u8; 4],
    pub blob: bool,
    pub behind: bool,
    /// Radius of the blob surface in spacings (smaller = crisper).
    pub smoothness: f64,
}

struct Pour {
    at: [f64; 2],
    speed: [f64; 2],
    width: f64,
    rate: f64,
    active: f64,
    emitted: u64,
}

pub struct FluidSim {
    names: Vec<String>,
    fluids: Vec<ph::Fluid>,
    pours: Vec<Vec<Pour>>,
    ppm: f64,
}

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-FLD-001", msg)
}
fn e2(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-FLD-002", msg)
}

fn inside(poly: &[P], p: P) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
    }
    c
}

impl Interp {
    pub fn eval_fluids(&mut self) -> Res<()> {
        let decls = self.program.fluids.clone();
        let vars = Vars {
            scopes: vec![self.globals.clone()],
        };
        let ppm = self.world_cfg.scale;
        let mut total = 0usize;
        for d in &decls {
            if self.fluid_cfg.contains_key(&d.name) {
                return Err(e1(format!("there is already a fluid named `{}`", d.name)).at(d.span));
            }
            let mut cfg = FluidCfg {
                params: ph::FluidParams::default(),
                spacing_px: 8.0,
                fill: None,
                color: [0x4f, 0x9b, 0xe8, 255],
                color_end: [0x4f, 0x9b, 0xe8, 255],
                blob: true,
                behind: false,
                smoothness: 1.4,
            };
            let (mut color_end, mut spacing) = (None, 8.0);
            for (key, expr, span) in &d.attrs {
                let v = self.eval(expr, &vars, Mode::Build)?;
                let num =
                    |s: &Interp, what: &str| s.num(&v, what).map_err(|e| e1(e.message).at(*span));
                match key.as_str() {
                    "fill" | "in" => {
                        cfg.fill = Some(match &v {
                            Value::Rect(r) => Region::Rect(*r),
                            Value::Shape(s) => Region::Shape(s.clone()),
                            other => return Err(e2(format!("`{key}` must be a `rect(x, y, w, h)` or a shape, found {}", other.type_name())).at(*span).help("write `fill: rect(60, 300, 240, 200)` or `fill: some_shape` (its faces are filled)")),
                        })
                    }
                    "spacing" => {
                        spacing = num(self, "`spacing`")?;
                        if spacing <= 0.5 {
                            return Err(e1("`spacing` (particle distance in px) must be above 0.5").at(*span));
                        }
                    }
                    "density" => {
                        cfg.params.density = num(self, "`density`")?;
                        if cfg.params.density <= 0.0 {
                            return Err(e1("`density` must be positive").at(*span));
                        }
                    }
                    "viscosity" => {
                        cfg.params.viscosity = num(self, "`viscosity`")?;
                        if !(0.0..=1.0).contains(&cfg.params.viscosity) {
                            return Err(e1("`viscosity` must be between 0 and 1").at(*span));
                        }
                    }
                    "surface_tension" => {
                        cfg.params.cohesion = num(self, "`surface_tension`")?;
                        if cfg.params.cohesion < 0.0 {
                            return Err(e1("`surface_tension` cannot be negative").at(*span));
                        }
                    }
                    "color" => match v {
                        Value::Color(c) => cfg.color = c,
                        _ => return Err(e1("`color` must be a colour such as #4f9be8").at(*span)),
                    },
                    "color_end" => match v {
                        Value::Color(c) => color_end = Some(c),
                        _ => return Err(e1("`color_end` must be a colour such as #2b5fa6").at(*span)),
                    },
                    "render" => match &v {
                        Value::Str(s) if s == "blob" => cfg.blob = true,
                        Value::Str(s) if s == "dots" => cfg.blob = false,
                        _ => return Err(e1("`render` must be \"blob\" or \"dots\"").at(*span)),
                    },
                    "behind" => match v {
                        Value::Bool(b) => cfg.behind = b,
                        _ => return Err(e1("`behind` must be true or false").at(*span)),
                    },
                    "foam" => {
                        cfg.params.foam = num(self, "`foam`")?;
                        if !(0.0..=1.0).contains(&cfg.params.foam) {
                            return Err(e1("`foam` must be between 0 and 1").at(*span));
                        }
                    }
                    "smoothness" => {
                        cfg.smoothness = num(self, "`smoothness`")?;
                        if !(0.8..=4.0).contains(&cfg.smoothness) {
                            return Err(e1("`smoothness` must be between 0.8 and 4 (the surface radius in particle spacings)").at(*span));
                        }
                    }
                    "max" => {
                        let n = self.int(&v, "`max`").map_err(|e| e1(e.message).at(*span))?;
                        if !(1..=HARD_LIMIT as i64).contains(&n) {
                            return Err(e1(format!("`max` must be between 1 and {HARD_LIMIT}")).at(*span));
                        }
                        cfg.params.max = n as usize;
                    }
                    other => return Err(e1(format!("a fluid has no setting `{other}`")).at(*span).help("settings: fill (or in), spacing, density, viscosity, surface_tension, foam, smoothness, color, color_end, render, behind, max")),
                }
            }
            cfg.color_end = color_end.unwrap_or(cfg.color);
            cfg.spacing_px = spacing;
            cfg.params.spacing = spacing / ppm;
            total += cfg.params.max;
            if total > HARD_LIMIT {
                return Err(e1(format!(
                    "all fluids together may hold at most {HARD_LIMIT} particles"
                ))
                .at(d.span)
                .help("lower `max` on some fluids"));
            }
            self.fluid_cfg.insert(d.name.clone(), cfg);
        }
        Ok(())
    }

    /// An empty fluid for every declaration (each scene starts dry until `fill_fluids`).
    pub fn reset_fluids(&mut self) {
        self.fluid = None;
        if self.fluid_cfg.is_empty() {
            return;
        }
        let (cw, ch) = (self.canvas.0 as f64, self.canvas.1 as f64);
        let ppm = self.world_cfg.scale;
        let mut sim = FluidSim {
            names: vec![],
            fluids: vec![],
            pours: vec![],
            ppm,
        };
        for (name, cfg) in &self.fluid_cfg {
            let mut f = ph::Fluid::new(cfg.params.clone());
            f.set_seed(crate::procedural::stream_seed(
                self.seed,
                &format!("fluid:{}:{name}", self.scene_name),
            ));
            f.set_bounds(
                v2(-2.0 * cw / ppm, -2.0 * ch / ppm),
                v2(3.0 * cw / ppm, 3.0 * ch / ppm),
            );
            sim.names.push(name.clone());
            sim.fluids.push(f);
            sim.pours.push(vec![]);
        }
        self.fluid = Some(sim);
    }

    fn region_points(&self, cfg: &FluidCfg, region: &Region) -> Vec<[f64; 2]> {
        let s = cfg.spacing_px;
        match region {
            Region::Rect([x, y, w, h]) => {
                let (nx, ny) = (
                    ((w / s) + 1e-9).floor() as usize,
                    ((h / s) + 1e-9).floor() as usize,
                );
                let mut out = Vec::with_capacity(nx * ny);
                for j in 0..ny {
                    for i in 0..nx {
                        out.push([x + (i as f64 + 0.5) * s, y + (j as f64 + 0.5) * s]);
                    }
                }
                out
            }
            Region::Shape(name) => {
                let sh = &self.shapes[name];
                let t = sh.transform();
                let polys: Vec<Vec<P>> = sh
                    .live_ids(Kind::F)
                    .into_iter()
                    .map(|f| sh.face_polygon(f).into_iter().map(|p| t.apply(p)).collect())
                    .collect();
                let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                for p in polys.iter().flatten() {
                    for k in 0..2 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                let mut out = vec![];
                let mut y = lo[1] + s / 2.0;
                while y < hi[1] {
                    let mut x = lo[0] + s / 2.0;
                    while x < hi[0] {
                        if polys.iter().any(|p| inside(p, [x, y])) {
                            out.push([x, y]);
                        }
                        x += s;
                    }
                    y += s;
                }
                out
            }
        }
    }

    /// Put each fluid's `fill` region on the lattice (after `setup:`, so shapes are in place).
    pub fn fill_fluids(&mut self) -> Res<()> {
        if self.fluid.is_none() {
            return Ok(());
        }
        let names: Vec<String> = self.fluid_cfg.keys().cloned().collect();
        for (i, name) in names.iter().enumerate() {
            let cfg = self.fluid_cfg[name].clone();
            let Some(region) = &cfg.fill else { continue };
            let pts = self.region_points(&cfg, region);
            let sim = self.fluid.as_mut().expect("checked");
            if sim.fluids[i].len() + pts.len() > cfg.params.max {
                return Err(e1(format!(
                    "`{name}` has room for {} particles but its fill region needs {}",
                    cfg.params.max,
                    pts.len()
                ))
                .help("raise `max`, make the region smaller or raise `spacing`"));
            }
            let ppm = sim.ppm;
            let m: Vec<Vec2> = pts.iter().map(|p| v2(p[0] / ppm, p[1] / ppm)).collect();
            sim.fluids[i].add(&m);
        }
        Ok(())
    }

    pub fn fluid_method(&mut self, name: &str, method: &str, args: Args) -> Res<Value> {
        let cfg = self.fluid_cfg[name].clone();
        let ppm = self.world_cfg.scale;
        let Some(i) = self
            .fluid
            .as_ref()
            .and_then(|s| s.names.iter().position(|n| n == name))
        else {
            return Ok(Value::None);
        };
        match method {
            "pour" => {
                let m = self.bind(method, args, &["at", "speed", "width", "rate"], &["at", "speed"])?;
                let at = self.point(&m["at"], "`at`").map_err(|e| e1(e.message))?;
                let speed = self.point(&m["speed"], "`speed`").map_err(|e| e1(e.message))?;
                let width = m.get("width").map(|v| self.num(v, "`width`").map_err(|e| e1(e.message))).transpose()?.unwrap_or(0.0);
                let sp = speed[0].hypot(speed[1]);
                if sp <= 0.0 {
                    return Err(e1("a pour needs a speed that is not (0, 0)").help("for example `speed: (0, 200)` pours straight down"));
                }
                let rate = match m.get("rate") {
                    Some(v) => self.num(v, "`rate`").map_err(|e| e1(e.message))?,
                    None => sp / cfg.spacing_px * (width / cfg.spacing_px).max(1.0),
                };
                if rate <= 0.0 || width < 0.0 {
                    return Err(e1("`rate` must be positive and `width` cannot be negative"));
                }
                self.fluid.as_mut().expect("found").pours[i].push(Pour { at, speed, width, rate, active: 0.0, emitted: 0 });
                Ok(Value::None)
            }
            "stop" => {
                self.bind(method, args, &[], &[])?;
                self.fluid.as_mut().expect("found").pours[i].clear();
                Ok(Value::None)
            }
            "add" => {
                let m = self.bind(method, args, &["region"], &["region"])?;
                let region = match &m["region"] {
                    Value::Rect(r) => Region::Rect(*r),
                    Value::Shape(s) => Region::Shape(s.clone()),
                    other => return Err(e2(format!("`add` needs a `rect(x, y, w, h)` or a shape, found {}", other.type_name()))),
                };
                let pts: Vec<Vec2> = self.region_points(&cfg, &region).iter().map(|p| v2(p[0] / ppm, p[1] / ppm)).collect();
                self.fluid.as_mut().expect("found").fluids[i].add(&pts);
                Ok(Value::None)
            }
            "level" => {
                let m = self.bind(method, args, &["x"], &["x"])?;
                let x = self.num(&m["x"], "`x`")?;
                let sim = self.fluid.as_ref().expect("found");
                let top = sim.fluids[i].pos.iter().filter(|p| (p.x * ppm - x).abs() <= cfg.spacing_px).map(|p| p.y * ppm).fold(f64::MAX, f64::min);
                Ok(if top == f64::MAX { Value::None } else { Value::Num(top) })
            }
            _ => Err(Diagnostic::new("E-NAM-003", format!("a fluid has no operation `{method}`")).help("fluids have `pour(at:, speed:, width:)`, `stop()`, `add(region)`, `level(x)` and the member `count`")),
        }
    }

    pub fn fluid_count(&self, name: &str) -> usize {
        self.fluid
            .as_ref()
            .and_then(|s| {
                s.names
                    .iter()
                    .position(|n| n == name)
                    .map(|i| s.fluids[i].len())
            })
            .unwrap_or(0)
    }

    pub fn has_fluid(&self) -> bool {
        self.fluid.is_some()
    }

    /// Advance soft bodies, fluids and the rigid world together for this frame (tick 0 only spawns).
    pub fn step_coupled(&mut self, tick: i64) {
        let (cfg, fps) = (self.world_cfg.clone(), self.fps.to_f64());
        let dt = 1.0 / fps;
        if tick == 0 {
            return;
        }
        let has_soft = self.soft.is_some();
        if let Some(sim) = self.fluid.as_mut() {
            for (i, pours) in sim.pours.iter_mut().enumerate() {
                let s_px = self.fluid_cfg[&sim.names[i]].spacing_px;
                for p in pours.iter_mut() {
                    p.active += dt;
                    let target = (p.rate * p.active + 1e-9).floor() as u64;
                    let due = target - p.emitted;
                    let sp = p.speed[0].hypot(p.speed[1]);
                    let (dir, perp) = (
                        [p.speed[0] / sp, p.speed[1] / sp],
                        [-p.speed[1] / sp, p.speed[0] / sp],
                    );
                    let m = ((p.width / s_px).round() as u64).max(1);
                    let mut pts = vec![];
                    for j in 0..due {
                        let t = (j as f64 + 0.5) / due as f64 * dt;
                        let k = p.emitted + j;
                        let lat = ((k % m) as f64 - (m as f64 - 1.0) / 2.0) * s_px;
                        let x = [
                            p.at[0] + p.speed[0] * (dt - t) + perp[0] * lat,
                            p.at[1] + p.speed[1] * (dt - t) + perp[1] * lat,
                        ];
                        let _ = dir;
                        pts.push((
                            v2(x[0] / sim.ppm, x[1] / sim.ppm),
                            v2(p.speed[0] / sim.ppm, p.speed[1] / sim.ppm),
                        ));
                    }
                    p.emitted = target;
                    sim.fluids[i].add_with_velocity(&pts);
                }
            }
        }
        let substeps = match (has_soft, self.fluid.is_some()) {
            (true, true) => cfg.soft_substeps.max(cfg.fluid_substeps),
            (true, false) => cfg.soft_substeps,
            _ => cfg.fluid_substeps,
        };
        let soft_bodies: &mut [ph::SoftBody] = match self.soft.as_mut() {
            Some(s) => s.bodies_mut(),
            None => &mut [],
        };
        let fluids: &mut [ph::Fluid] = match self.fluid.as_mut() {
            Some(f) => &mut f.fluids,
            None => &mut [],
        };
        let world = self.phys.as_mut().map(|p| &mut p.world);
        ph::coupled_step(
            world,
            soft_bodies,
            fluids,
            v2(cfg.gravity[0], cfg.gravity[1]),
            dt,
            substeps,
            cfg.soft_iterations,
            cfg.fluid_iterations,
        );
    }

    pub fn fluid_snapshot(&self) -> Option<J> {
        let sim = self.fluid.as_ref()?;
        let out: Vec<J> = sim
            .names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let c = &self.fluid_cfg[n];
                let pts: Vec<J> = sim.fluids[i].pos.iter().map(|p| json!([p.x * sim.ppm, p.y * sim.ppm])).collect();
                let foam: Vec<J> = sim.fluids[i].foam.iter().map(|f| json!([f.pos.x * sim.ppm, f.pos.y * sim.ppm, f.age / f.life])).collect();
                json!({"name": n, "spacing": c.spacing_px, "smoothness": c.smoothness, "color": hex(c.color), "color_end": hex(c.color_end), "render": if c.blob { "blob" } else { "dots" }, "behind": c.behind, "particles": pts, "foam": foam})
            })
            .collect();
        Some(J::Array(out))
    }

    pub fn end_fluids(&mut self) {
        self.fluid = None;
    }
}
