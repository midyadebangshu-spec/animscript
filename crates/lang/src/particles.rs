//! Particle emitters in the language: the `emitter name:` block, `start`/`stop`/`burst`, and the
//! per-tick step. Particles live in pixels; they collide with the physics world when `collide: true`
//! but never push bodies. Everything random comes from seeded streams keyed by the scene, the
//! emitter and the particle's spawn index.
use crate::color::{hex, to_linear, to_srgb8};
use crate::interp::{Interp, Vars};
use crate::procedural::{hash01, stream_seed};
use crate::value::*;
use animscript_core::*;
use animscript_physics as ph;
use ph::{v2, Vec2};
use serde_json::{json, Value as J};
use std::collections::BTreeMap;

type Args = Vec<(Option<String>, Value)>;

#[derive(Clone, Debug)]
pub enum Source {
    Point([f64; 2]),
    Shape(String),
}

#[derive(Clone, Debug)]
pub struct EmitterCfg {
    pub def: ph::EmitterDef,
    pub source: Source,
    pub size_end: Option<f64>,
    pub color: [u8; 4],
    pub color_end: [u8; 4],
    pub opacity: (f64, f64),
    /// `force_x` / `force_y` (px / s^2) and the fields they read: equations over x, y, vx, vy, age, i, t.
    pub force: [Option<Value>; 2],
    pub fields: Option<Value>,
}

/// A compiled `force_x` / `force_y` pair for one emitter.
pub struct PartForce {
    pub kernel: Kernel,
    pub fields: Vec<(String, usize)>,
}

const HARD_LIMIT: usize = 20_000;

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-PTC-001", msg)
}

impl Interp {
    fn secs(&self, v: &Value) -> Option<f64> {
        match v {
            Value::Dur(r) => Some(r.to_f64()),
            Value::Frames(n) => Some(*n as f64 / self.fps.to_f64()),
            other => other.as_f64(),
        }
    }

    /// One number, or a `(min, max)` pair, converted by `conv`.
    fn range(
        &self,
        v: &Value,
        what: &str,
        conv: &dyn Fn(&Value) -> Option<f64>,
    ) -> Res<(f64, f64)> {
        let bad = || e1(format!("`{what}` must be a number or a (min, max) pair"));
        match v {
            Value::Tuple(t) if t.len() == 2 => {
                Ok((conv(&t[0]).ok_or_else(bad)?, conv(&t[1]).ok_or_else(bad)?))
            }
            one => {
                let x = conv(one).ok_or_else(bad)?;
                Ok((x, x))
            }
        }
    }

    pub fn eval_emitters(&mut self) -> Res<()> {
        let decls = self.program.emitters.clone();
        let vars = Vars {
            scopes: vec![self.globals.clone()],
        };
        let mut total = 0usize;
        for d in &decls {
            if self.emitters.contains_key(&d.name) {
                return Err(
                    e1(format!("there is already an emitter named `{}`", d.name)).at(d.span),
                );
            }
            let mut cfg = EmitterCfg {
                def: ph::EmitterDef::default(),
                source: Source::Point([0.0, 0.0]),
                size_end: None,
                color: [255; 4],
                color_end: [255; 4],
                opacity: (1.0, 1.0),
                force: [None, None],
                fields: None,
            };
            let (mut color_end, mut opacity_end) = (None, None);
            for (key, expr, span) in &d.attrs {
                let v = self.eval(expr, &vars, crate::interp::Mode::Build)?;
                let at = |e: Diagnostic| e.at(*span);
                let num =
                    |s: &Interp, what: &str| s.num(&v, what).map_err(|e| e1(e.message).at(*span));
                match key.as_str() {
                    "at" => {
                        cfg.source = match &v {
                            Value::Shape(s) => Source::Shape(s.clone()),
                            Value::Tuple(t) if t.len() == 2 && t.iter().all(|x| x.as_f64().is_some()) => Source::Point([t[0].as_f64().unwrap(), t[1].as_f64().unwrap()]),
                            other => return Err(Diagnostic::new("E-PTC-002", format!("`at` must be a point or a shape, found {}", other.type_name())).at(*span).help("write `at: (400, 300)` or `at: ball`")),
                        }
                    }
                    "rate" => {
                        cfg.def.rate = num(self, "`rate`")?;
                        if cfg.def.rate < 0.0 {
                            return Err(e1("`rate` cannot be negative").at(*span));
                        }
                    }
                    "lifetime" => {
                        cfg.def.lifetime = self.range(&v, "lifetime", &|x| self.secs(x)).map_err(at)?;
                        if cfg.def.lifetime.0 <= 0.0 || cfg.def.lifetime.1 < cfg.def.lifetime.0 {
                            return Err(e1("`lifetime` must be positive, as (shortest, longest)").at(*span));
                        }
                    }
                    "speed" => cfg.def.speed = self.range(&v, "speed", &|x| x.as_f64()).map_err(at)?,
                    "angle" => cfg.def.angle = self.range(&v, "angle", &|x| if let Value::Angle(a) = x { Some(a * std::f64::consts::TAU) } else { None }).map_err(|_| e1("`angle` must be an angle or a pair of angles, such as `(-120.deg, -60.deg)`").at(*span))?,
                    "size" => {
                        cfg.def.size = self.range(&v, "size", &|x| x.as_f64()).map_err(at)?;
                        if cfg.def.size.0 < 0.0 || cfg.def.size.1 < 0.0 {
                            return Err(e1("`size` cannot be negative").at(*span));
                        }
                    }
                    "size_end" => cfg.size_end = Some(num(self, "`size_end`")?),
                    "color" | "color_end" => match v {
                        Value::Color(c) if key == "color" => cfg.color = c,
                        Value::Color(c) => color_end = Some(c),
                        _ => return Err(e1(format!("`{key}` must be a colour such as #ffcc33")).at(*span)),
                    },
                    "opacity" => cfg.opacity.0 = num(self, "`opacity`")?,
                    "opacity_end" => opacity_end = Some(num(self, "`opacity_end`")?),
                    "gravity" => {
                        let p = self.point(&v, "`gravity`").map_err(|e| e1(e.message).at(*span))?;
                        cfg.def.gravity = v2(p[0], p[1]);
                    }
                    "drag" => cfg.def.drag = num(self, "`drag`")?.max(0.0),
                    "restitution" => cfg.def.restitution = num(self, "`restitution`")?.max(0.0),
                    "collide" => match v {
                        Value::Bool(b) => cfg.def.collide = b,
                        _ => return Err(e1("`collide` must be true or false").at(*span)),
                    },
                    "force_x" | "force_y" => {
                        if !matches!(v, Value::Str(_) | Value::Lambda(_) | Value::Int(_) | Value::Num(_)) {
                            return Err(Diagnostic::new("E-PTC-002", format!("`{key}` must be an equation (a string or a lambda) or a number, found {}", v.type_name())).at(*span));
                        }
                        cfg.force[usize::from(key == "force_y")] = Some(v.clone());
                    }
                    "fields" => cfg.fields = Some(v.clone()),
                    "max" => {
                        let n = self.int(&v, "`max`").map_err(|e| e1(e.message).at(*span))?;
                        if !(1..=HARD_LIMIT as i64).contains(&n) {
                            return Err(e1(format!("`max` must be between 1 and {HARD_LIMIT}")).at(*span));
                        }
                        cfg.def.max = n as usize;
                    }
                    other => return Err(e1(format!("an emitter has no setting `{other}`")).at(*span).help("settings: force_x, force_y, fields, at, rate, lifetime, speed, angle, size, size_end, color, color_end, opacity, opacity_end, gravity, drag, restitution, collide, max")),
                }
            }
            cfg.color_end = color_end.unwrap_or(cfg.color);
            cfg.opacity.1 = opacity_end.unwrap_or(cfg.opacity.0);
            total += cfg.def.max;
            if total > HARD_LIMIT {
                return Err(e1(format!(
                    "all emitters together may hold at most {HARD_LIMIT} particles"
                ))
                .at(d.span)
                .help("lower `max` on some emitters"));
            }
            self.emitters.insert(d.name.clone(), cfg);
        }
        Ok(())
    }

    /// A fresh, stopped emitter for every declaration (called at the start of each scene).
    pub fn reset_particles(&mut self) {
        self.parts.clear();
        for (name, cfg) in &self.emitters {
            let seed = stream_seed(self.seed, &format!("emitter:{}:{name}", self.scene_name));
            self.parts
                .insert(name.clone(), ph::Emitter::new(cfg.def.clone(), seed));
        }
    }

    /// Compile the force equations of every emitter that has them (at the start of each scene).
    pub fn reset_particle_forces(&mut self) -> Res<()> {
        self.part_forces.clear();
        let emitters: Vec<(String, EmitterCfg)> = self
            .emitters
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (name, cfg) in emitters {
            if cfg.force[0].is_none() && cfg.force[1].is_none() {
                continue;
            }
            let field_in = match &cfg.fields {
                Some(v) => self.field_inputs(v, "an emitter")?,
                None => vec![],
            };
            let mut point: Vec<&str> = vec!["x", "y", "vx", "vy", "age", "i"];
            point.extend(field_in.iter().map(|f| f.0.as_str()));
            let (fx, fy) = (
                cfg.force[0].clone().unwrap_or(Value::Int(0)),
                cfg.force[1].clone().unwrap_or(Value::Int(0)),
            );
            let seed = self.seed32(&format!("{}/force/{name}", self.scene_name));
            let prog = self.compile_equations(
                &[("force_x", &fx), ("force_y", &fy)],
                &point,
                &["t"],
                &BTreeMap::new(),
                seed,
            )?;
            let kernel = self.make_kernel(prog, cfg.def.max);
            self.part_forces.insert(
                name,
                PartForce {
                    kernel,
                    fields: field_in.iter().map(|f| (f.1.clone(), f.2)).collect(),
                },
            );
        }
        Ok(())
    }

    /// The velocity change `force * dt` of every live particle of one emitter (none if it has no force equations).
    fn force_deltas(&self, name: &str, dt: f64, t: f64) -> Option<Vec<(f64, f64)>> {
        let pf = self.part_forces.get(name)?;
        let ps = self.parts[name].particles();
        if ps.is_empty() {
            return None;
        }
        let n = ps.len();
        let (xs, ys): (Vec<f64>, Vec<f64>) = ps.iter().map(|p| (p.p.x, p.p.y)).unzip();
        let vx: Vec<f64> = ps.iter().map(|p| p.v.x).collect();
        let vy: Vec<f64> = ps.iter().map(|p| p.v.y).collect();
        let age: Vec<f64> = ps.iter().map(|p| p.age).collect();
        let idx: Vec<f64> = ps.iter().map(|p| p.index as f64).collect();
        let extra: Vec<Vec<f64>> = pf
            .fields
            .iter()
            .map(|(f, c)| {
                xs.iter()
                    .zip(&ys)
                    .map(|(x, y)| self.field_sample(f, *c, *x, *y))
                    .collect()
            })
            .collect();
        let mut cols: Vec<&[f64]> = vec![&xs, &ys, &vx, &vy, &age, &idx];
        cols.extend(extra.iter().map(|c| c.as_slice()));
        let out = pf
            .kernel
            .eval_n(n, &cols, &[t], &[])
            .expect("force inputs match");
        // a force that is not a number leaves the particle alone
        Some(
            (0..n)
                .map(|k| {
                    if out[0][k].is_finite() && out[1][k].is_finite() {
                        (out[0][k] * dt, out[1][k] * dt)
                    } else {
                        (0.0, 0.0)
                    }
                })
                .collect(),
        )
    }

    pub fn emitter_method(&mut self, name: &str, method: &str, args: Args) -> Res<Value> {
        match method {
            "start" | "stop" => {
                self.bind(method, args, &[], &[])?;
                if let Some(e) = self.parts.get_mut(name) {
                    if method == "start" {
                        e.start()
                    } else {
                        e.stop()
                    }
                }
                Ok(Value::None)
            }
            "burst" => {
                let m = self.bind(method, args, &["count"], &["count"])?;
                let n = self.int(&m["count"], "`count`")?;
                if n < 0 {
                    return Err(e1("`burst` needs a count of 0 or more"));
                }
                if let Some(e) = self.parts.get_mut(name) {
                    e.burst(n as usize);
                }
                Ok(Value::None)
            }
            _ => Err(Diagnostic::new(
                "E-NAM-003",
                format!("an emitter has no operation `{method}`"),
            )
            .help("emitters have `start()`, `stop()` and `burst(n)`, and the member `count`")),
        }
    }

    pub fn emitter_count(&self, name: &str) -> usize {
        self.parts.get(name).map_or(0, |e| e.particles().len())
    }

    /// Step every emitter to this tick and describe the live particles for the snapshot.
    pub fn particles_tick(&mut self, tick: i64, tf: &BTreeMap<String, Transform>) -> Option<J> {
        if self.emitters.is_empty() {
            return None;
        }
        let steps = if tick > 0 {
            self.world_cfg.substeps.max(1)
        } else {
            1
        };
        let dt = if tick > 0 {
            1.0 / self.fps.to_f64() / steps as f64
        } else {
            0.0
        };
        let world = self.phys.as_ref().map(|p| (&p.world, p.ppm));
        let names: Vec<String> = self.emitters.keys().cloned().collect();
        let mut out = vec![];
        for name in names {
            let cfg = self.emitters[&name].clone();
            // where particles are born: a point, or a random point inside the shape's faces
            let (polys, bbox) = match &cfg.source {
                Source::Shape(s) => {
                    let sh = &self.shapes[s];
                    let t = tf.get(s).copied().unwrap_or_else(|| sh.transform());
                    let polys: Vec<Vec<[f64; 2]>> = sh
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
                    if polys.is_empty() {
                        let c = t.apply(sh.bbox_center());
                        (vec![], ([c[0]; 2], [c[1]; 2]))
                    } else {
                        (polys, ([lo[0], hi[0]], [lo[1], hi[1]]))
                    }
                }
                Source::Point(p) => (vec![], ([p[0]; 2], [p[1]; 2])),
            };
            let pos_seed = stream_seed(
                self.seed,
                &format!("emitter-pos:{}:{name}", self.scene_name),
            );
            let mut spawn = |i: u64| -> Vec2 {
                let (xr, yr) = (&bbox.0, &bbox.1);
                if polys.is_empty() {
                    return v2((xr[0] + xr[1]) / 2.0, (yr[0] + yr[1]) / 2.0);
                }
                for k in 0..16 {
                    let x = xr[0] + (xr[1] - xr[0]) * hash01(pos_seed, (i * 32 + 2 * k) as i64);
                    let y = yr[0] + (yr[1] - yr[0]) * hash01(pos_seed, (i * 32 + 2 * k + 1) as i64);
                    if polys.iter().any(|p| inside(p, [x, y])) {
                        return v2(x, y);
                    }
                }
                v2((xr[0] + xr[1]) / 2.0, (yr[0] + yr[1]) / 2.0)
            };
            let t_now = tick as f64 / self.fps.to_f64();
            for _ in 0..steps {
                if let Some(dv) = self.force_deltas(&name, dt, t_now) {
                    let em = self.parts.get_mut(&name).expect("reset at scene start");
                    for (p, d) in em.particles_mut().iter_mut().zip(dv) {
                        p.v.x += d.0;
                        p.v.y += d.1;
                    }
                }
                let em = self.parts.get_mut(&name).expect("reset at scene start");
                em.step(dt, world, &mut spawn);
            }
            let em = self.parts.get_mut(&name).expect("reset at scene start");
            let (c0, c1) = (to_linear(cfg.color), to_linear(cfg.color_end));
            let items: Vec<J> = em
                .particles()
                .iter()
                .map(|p| {
                    let t = (p.age / p.life).clamp(0.0, 1.0);
                    let mix = |a: f64, b: f64| a + (b - a) * t;
                    let radius = mix(p.size, cfg.size_end.unwrap_or(p.size)).max(0.0);
                    let opacity = mix(cfg.opacity.0, cfg.opacity.1).clamp(0.0, 1.0);
                    let c = [
                        mix(c0[0], c1[0]),
                        mix(c0[1], c1[1]),
                        mix(c0[2], c1[2]),
                        mix(c0[3], c1[3]) * opacity,
                    ];
                    json!([p.p.x, p.p.y, radius, hex(to_srgb8(c))])
                })
                .collect();
            out.push(json!({"name": name, "items": items}));
        }
        Some(J::Array(out))
    }
}

/// Even-odd point in polygon.
fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
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
