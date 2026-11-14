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
    Box3([f64; 6]),
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
    /// `space: "3d"`: a fluid of a scene with mesh bodies even if nothing else says so.
    pub space3: bool,
    /// Whether `spacing` was written (a 3D fluid defaults to a coarser 12 px).
    pub spacing_given: bool,
    /// `field:` a wave field the water rests on and makes ripples in (3D fluids).
    pub link: Option<FieldLink>,
    /// How strongly a 3D fluid's surface reflects (Fresnel scale) and bends what is behind it.
    pub reflect: f64,
    pub refract: f64,
    /// A fluid on a field: water that reaches the surface joins the field (a sea); `false`: it rests
    /// on it as a pool.
    pub absorb: bool,
}

/// The coupling of a 3D fluid to a wave field.
#[derive(Clone, Debug)]
pub struct FieldLink {
    pub field: String,
    pub channel: usize,
    pub velocity: Option<usize>,
    /// The field's rest surface, y in px: the surface is at `level - h`.
    pub level: f64,
    /// How strongly the water's impacts make ripples (0 = none).
    pub ripple: f64,
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
                space3: false,
                spacing_given: false,
                link: None,
                reflect: 1.0,
                refract: 1.0,
                absorb: true,
            };
            let (
                mut link_field,
                mut link_level,
                mut link_channel,
                mut link_velocity,
                mut link_ripple,
            ) = (
                None::<String>,
                None::<f64>,
                None::<String>,
                None::<String>,
                1.0f64,
            );
            let (mut color_end, mut spacing) = (None, 8.0);
            for (key, expr, span) in &d.attrs {
                let v = self.eval(expr, &vars, Mode::Build)?;
                let num =
                    |s: &Interp, what: &str| s.num(&v, what).map_err(|e| e1(e.message).at(*span));
                match key.as_str() {
                    "field" => match &v {
                        Value::Field(f) => link_field = Some(f.clone()),
                        _ => return Err(e1("`field` must be a field, such as `field: lagoon`").at(*span)),
                    },
                    "level" => link_level = Some(num(self, "`level`")?),
                    "channel" | "velocity" => match &v {
                        Value::Str(c) => {
                            if key == "channel" {
                                link_channel = Some(c.clone())
                            } else {
                                link_velocity = Some(c.clone())
                            }
                        }
                        _ => return Err(e1(format!("`{key}` must be a channel name in quotes")).at(*span)),
                    },
                    "absorb" => match v {
                        Value::Bool(b) => cfg.absorb = b,
                        _ => return Err(e1("`absorb` must be true or false").at(*span)),
                    },
                    "reflect" | "refract" => {
                        let x = num(self, &format!("`{key}`"))?;
                        if !(0.0..=1.0).contains(&x) {
                            return Err(e1(format!("`{key}` is between 0 and 1")).at(*span));
                        }
                        if key == "reflect" {
                            cfg.reflect = x
                        } else {
                            cfg.refract = x
                        }
                    }
                    "ripple" => {
                        link_ripple = num(self, "`ripple`")?;
                        if link_ripple < 0.0 {
                            return Err(e1("`ripple` cannot be negative").at(*span));
                        }
                    }
                    "space" => match &v {
                        Value::Str(s) if s == "3d" => cfg.space3 = true,
                        Value::Str(s) if s == "2d" => cfg.space3 = false,
                        _ => return Err(e1("`space` must be \"2d\" or \"3d\"").at(*span)),
                    },
                    "fill" | "in" => {
                        cfg.fill = Some(match &v {
                            Value::Rect(r) => Region::Rect(*r),
                            Value::Box3(b) => Region::Box3(*b),
                            Value::Shape(s) => Region::Shape(s.clone()),
                            other => return Err(e2(format!("`{key}` must be a `rect(x, y, w, h)` or a shape, found {}", other.type_name())).at(*span).help("write `fill: rect(60, 300, 240, 200)` or `fill: some_shape` (its faces are filled)")),
                        })
                    }
                    "spacing" => {
                        cfg.spacing_given = true;
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
                    other => return Err(e1(format!("a fluid has no setting `{other}`")).at(*span).help("settings: fill (or in), space, field, level, channel, velocity, ripple, absorb, reflect, refract, spacing, density, viscosity, surface_tension, foam, smoothness, color, color_end, render, behind, max")),
                }
            }
            if let Some(f) = link_field {
                let fc = self
                    .fields
                    .get(&f)
                    .ok_or_else(|| e1(format!("unknown field `{f}`")).at(d.span))?;
                if fc.chain.is_some() {
                    return Err(e1("a chain field cannot carry water")
                        .at(d.span)
                        .help("use a grid field"));
                }
                let level = link_level.ok_or_else(|| {
                    e1("a fluid on a field needs `level:`, the y (px) of the field's rest surface")
                        .at(d.span)
                        .help("for the lagoon's water plane at y = 520 write `level: 520`")
                })?;
                let find = |c: &str| {
                    fc.channels
                        .iter()
                        .position(|x| x == c)
                        .ok_or_else(|| e1(format!("field `{f}` has no channel `{c}`")).at(d.span))
                };
                let channel = match &link_channel {
                    Some(c) => find(c)?,
                    None => 0,
                };
                let velocity = match &link_velocity {
                    Some(c) => Some(find(c)?),
                    None => fc.channels.iter().position(|x| x == "v"),
                };
                cfg.link = Some(FieldLink {
                    field: f,
                    channel,
                    velocity,
                    level,
                    ripple: link_ripple,
                });
                cfg.space3 = true;
            } else if link_level.is_some() || link_channel.is_some() || link_velocity.is_some() {
                return Err(e1("`level`, `channel` and `velocity` need `field:`").at(d.span));
            }
            cfg.color_end = color_end.unwrap_or(cfg.color);
            if !cfg.spacing_given && matches!(cfg.fill, Some(Region::Box3(_))) {
                spacing = 12.0;
            }
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
        self.fluid3 = None;
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
        self.reset_fluids3();
        for (name, cfg) in &self.fluid_cfg {
            if self.fluid_is_3d(cfg) {
                continue;
            }
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
            Region::Box3(_) => vec![],
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
        let names: Vec<String> = self.fluid_cfg.keys().cloned().collect();
        if self.fluid.is_none() {
            return self.fill_fluids3();
        }
        self.fill_fluids3()?;
        let mut i2d = 0usize;
        for name in names.iter() {
            let cfg = self.fluid_cfg[name].clone();
            if self.fluid_is_3d(&cfg) {
                continue;
            }
            let i = i2d;
            i2d += 1;
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
        if self
            .fluid3
            .as_ref()
            .is_some_and(|s| s.names.iter().any(|n| n == name))
        {
            return self.fluid3_method(name, method, args);
        }
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
        if let Some(n) = self.fluid3_count(name) {
            return n;
        }
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
