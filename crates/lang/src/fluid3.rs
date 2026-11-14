//! Fluids in 3D scenes: a `fluid` whose `fill` is a `box(...)` or a mesh (or with `space: "3d"`),
//! its `pour` / `stop` / `add` / `level`, and the stepping together with the rigid world and the soft
//! bodies. The engine is `animscript-physics3`; the picture is made by the renderer, which fills the
//! particles' density field and extracts a surface mesh from it (marching cubes).
use crate::color::hex;
use crate::fluid::{FluidCfg, Region};
use crate::interp::Interp;
use crate::value::*;
use animscript_core::*;
use animscript_physics3 as p3;
use p3::{v3, Fluid3, FluidParams3, V3};
use serde_json::{json, Value as J};

type Args = Vec<(Option<String>, Value)>;

fn e4(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-FLD-004", msg)
}

struct Pour3 {
    at: [f64; 3],
    speed: [f64; 3],
    width: f64,
    active: f64,
    emitted: u64,
}

pub struct FluidSim3 {
    pub(crate) names: Vec<String>,
    pub(crate) fluids: Vec<Fluid3>,
    pours: Vec<Vec<Pour3>>,
    ppm: f64,
}

/// Standard base64 (with padding) of a byte string.
pub fn base64(bytes: &[u8]) -> String {
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

fn f32s(values: impl Iterator<Item = f64>) -> String {
    let mut bytes = vec![];
    for v in values {
        bytes.extend_from_slice(&(v as f32).to_le_bytes());
    }
    base64(&bytes)
}

impl Interp {
    /// Is this fluid part of a 3D scene?
    pub fn fluid_is_3d(&self, cfg: &FluidCfg) -> bool {
        cfg.space3
            || match &cfg.fill {
                Some(Region::Box3(_)) => true,
                Some(Region::Shape(n)) => self.is_mesh_shape(n),
                _ => false,
            }
    }

    pub fn reset_fluids3(&mut self) {
        self.fluid3 = None;
        let ppm = self.world_cfg.scale;
        let threads = self.world_cfg.fluid_threads;
        let mut sim = FluidSim3 {
            names: vec![],
            fluids: vec![],
            pours: vec![],
            ppm,
        };
        for (name, cfg) in &self.fluid_cfg {
            if !self.fluid_is_3d(cfg) {
                continue;
            }
            let spacing_px = if cfg.spacing_given {
                cfg.spacing_px
            } else {
                12.0
            };
            let mut f = Fluid3::new(FluidParams3 {
                spacing: spacing_px / ppm,
                density: cfg.params.density,
                viscosity: cfg.params.viscosity,
                cohesion: cfg.params.cohesion,
                max: cfg.params.max.min(20_000),
                foam: cfg.params.foam,
                threads,
                ..FluidParams3::default()
            });
            f.set_seed(crate::procedural::stream_seed(
                self.seed,
                &format!("fluid3:{}:{name}", self.scene_name),
            ));
            let (cw, ch) = (self.canvas.0 as f64, self.canvas.1 as f64);
            f.set_bounds(
                v3(-3.0 * cw / ppm, -4.0 * ch / ppm, -6.0 * cw / ppm),
                v3(4.0 * cw / ppm, 3.0 * ch / ppm, 6.0 * cw / ppm),
            );
            sim.names.push(name.clone());
            sim.fluids.push(f);
            sim.pours.push(vec![]);
        }
        if !sim.names.is_empty() {
            self.fluid3 = Some(sim);
            // liquids are drawn through the 3D camera, which exists once something 3D does
            self.ensure_camera3();
        }
    }

    fn spacing3(&self, cfg: &FluidCfg) -> f64 {
        if cfg.spacing_given {
            cfg.spacing_px
        } else {
            12.0
        }
    }

    fn region_points3(&self, cfg: &FluidCfg, region: &Region) -> Res<Vec<[f64; 3]>> {
        let s = self.spacing3(cfg);
        match region {
            Region::Box3([x, y, z, w, h, d]) => {
                let (nx, ny, nz) = (
                    ((w / s) + 1e-9).floor() as usize,
                    ((h / s) + 1e-9).floor() as usize,
                    ((d / s) + 1e-9).floor() as usize,
                );
                let mut out = Vec::with_capacity(nx * ny * nz);
                for k in 0..nz {
                    for j in 0..ny {
                        for i in 0..nx {
                            out.push([
                                x + (i as f64 + 0.5) * s,
                                y + (j as f64 + 0.5) * s,
                                z + (k as f64 + 0.5) * s,
                            ]);
                        }
                    }
                }
                Ok(out)
            }
            Region::Shape(name) if self.is_mesh_shape(name) => {
                let mesh = self.shapes[name].mesh.as_ref().expect("mesh");
                let t = self.mesh_transform(name);
                let verts: Vec<V3> = mesh
                    .verts
                    .iter()
                    .map(|v| V3::from_arr(t.apply(*v)))
                    .collect();
                let mut tris = vec![];
                for f in &mesh.faces {
                    for k in 1..f.len().saturating_sub(1) {
                        tris.push([f[0] as usize, f[k] as usize, f[k + 1] as usize]);
                    }
                }
                let (mut lo, mut hi) = (verts[0], verts[0]);
                for v in &verts {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
                let mut out = vec![];
                let mut z = lo.z + s / 2.0;
                while z < hi.z {
                    let mut y = lo.y + s / 2.0;
                    while y < hi.y {
                        let mut x = lo.x + s / 2.0;
                        while x < hi.x {
                            if p3::point_in_mesh(v3(x, y, z), &verts, &tris) {
                                out.push([x, y, z]);
                            }
                            x += s;
                        }
                        y += s;
                    }
                    z += s;
                }
                Ok(out)
            }
            _ => Err(
                e4("a 3D fluid fills a `box(x, y, z, w, h, d)` or a closed mesh shape")
                    .help("write `fill: box(100, 300, 0, 400, 200, 300)` or `fill: some_cube`"),
            ),
        }
    }

    /// Put each 3D fluid's `fill` region on the lattice (after `setup:`, so shapes are in place).
    pub fn fill_fluids3(&mut self) -> Res<()> {
        let Some(sim) = self.fluid3.as_ref() else {
            return Ok(());
        };
        let names = sim.names.clone();
        for (i, name) in names.iter().enumerate() {
            let cfg = self.fluid_cfg[name].clone();
            let Some(region) = &cfg.fill else { continue };
            let pts = self.region_points3(&cfg, region)?;
            let sim = self.fluid3.as_mut().expect("checked");
            if sim.fluids[i].len() + pts.len() > sim.fluids[i].params().max {
                return Err(e4(format!(
                    "`{name}` has room for {} particles but its fill region needs {}",
                    sim.fluids[i].params().max,
                    pts.len()
                ))
                .help("raise `max`, make the region smaller or raise `spacing`"));
            }
            let ppm = sim.ppm;
            let m: Vec<V3> = pts
                .iter()
                .map(|p| v3(p[0] / ppm, p[1] / ppm, p[2] / ppm))
                .collect();
            sim.fluids[i].add(&m);
        }
        // the world the fluids step with exists even when nothing rigid was declared
        if self.phys3.is_none() {
            let g = self.world_cfg.gravity3;
            self.phys3 = Some(crate::phys3::Physics3::empty(
                p3::World::new(v3(g[0], g[1], g[2])),
                self.world_cfg.scale,
                self.world_cfg.substeps,
            ));
        }
        Ok(())
    }

    pub fn fluid3_count(&self, name: &str) -> Option<usize> {
        let s = self.fluid3.as_ref()?;
        let i = s.names.iter().position(|n| n == name)?;
        Some(s.fluids[i].len())
    }

    pub fn fluid3_method(&mut self, name: &str, method: &str, args: Args) -> Res<Value> {
        let cfg = self.fluid_cfg[name].clone();
        let ppm = self.world_cfg.scale;
        let i = self
            .fluid3
            .as_ref()
            .and_then(|s| s.names.iter().position(|n| n == name))
            .expect("checked");
        let s_px = self.spacing3(&cfg);
        let triple = |this: &Interp, v: &Value, what: &str| -> Res<[f64; 3]> {
            match v {
                Value::Tuple(t) if t.len() == 3 => Ok([
                    this.num(&t[0], what)?,
                    this.num(&t[1], what)?,
                    this.num(&t[2], what)?,
                ]),
                other => Err(e4(format!(
                    "{what} must be a 3D point like `(100, 40, 200)`, found {}",
                    other.type_name()
                ))),
            }
        };
        match method {
            "pour" => {
                let m = self.bind(method, args, &["at", "speed", "width"], &["at", "speed"])?;
                let at = triple(self, &m["at"], "`at`")?;
                let speed = triple(self, &m["speed"], "`speed`")?;
                let width = m
                    .get("width")
                    .map(|v| self.num(v, "`width`").map_err(|e| e4(e.message)))
                    .transpose()?
                    .unwrap_or(0.0);
                if speed == [0.0; 3] {
                    return Err(e4("a pour needs a speed that is not (0, 0, 0)")
                        .help("for example `speed: (0, 200, 0)` pours straight down"));
                }
                if width < 0.0 {
                    return Err(e4("`width` (the diameter of the jet) cannot be negative"));
                }
                self.fluid3.as_mut().expect("found").pours[i].push(Pour3 {
                    at,
                    speed,
                    width,
                    active: 0.0,
                    emitted: 0,
                });
                Ok(Value::None)
            }
            "stop" => {
                self.bind(method, args, &[], &[])?;
                self.fluid3.as_mut().expect("found").pours[i].clear();
                Ok(Value::None)
            }
            "add" => {
                let m = self.bind(method, args, &["region"], &["region"])?;
                let region = match &m["region"] {
                    Value::Box3(b) => Region::Box3(*b),
                    Value::Shape(s) => Region::Shape(s.clone()),
                    other => {
                        return Err(e4(format!(
                            "`add` needs a `box(x, y, z, w, h, d)` or a mesh shape, found {}",
                            other.type_name()
                        )))
                    }
                };
                let pts: Vec<V3> = self
                    .region_points3(&cfg, &region)?
                    .iter()
                    .map(|p| v3(p[0] / ppm, p[1] / ppm, p[2] / ppm))
                    .collect();
                self.fluid3.as_mut().expect("found").fluids[i].add(&pts);
                Ok(Value::None)
            }
            "level" => {
                let m = self.bind(method, args, &["x", "z"], &["x", "z"])?;
                let (x, z) = (self.num(&m["x"], "`x`")?, self.num(&m["z"], "`z`")?);
                let sim = self.fluid3.as_ref().expect("found");
                let top = sim.fluids[i]
                    .pos
                    .iter()
                    .filter(|p| {
                        (p.x * ppm - x).abs() <= s_px && (p.z * ppm - z).abs() <= s_px
                    })
                    .map(|p| p.y * ppm)
                    .fold(f64::MAX, f64::min);
                Ok(if top == f64::MAX { Value::None } else { Value::Num(top) })
            }
            _ => Err(Diagnostic::new("E-NAM-003", format!("a fluid has no operation `{method}`"))
                .help("fluids have `pour(at:, speed:, width:)`, `stop()`, `add(region)`, `level(x, z)` (3D) and the member `count`")),
        }
    }

    /// Emit the jets of this frame (a slice of the jet's disc of particles every spacing it travels).
    pub fn fluid3_emit(&mut self, dt: f64) {
        let Some(sim) = self.fluid3.as_mut() else {
            return;
        };
        for (i, pours) in sim.pours.iter_mut().enumerate() {
            let s_px = if self.fluid_cfg[&sim.names[i]].spacing_given {
                self.fluid_cfg[&sim.names[i]].spacing_px
            } else {
                12.0
            };
            for p in pours.iter_mut() {
                p.active += dt;
                let sp = (p.speed[0].powi(2) + p.speed[1].powi(2) + p.speed[2].powi(2)).sqrt();
                let target = (sp * p.active / s_px + 1e-9).floor() as u64;
                let due = target - p.emitted;
                let dir = v3(p.speed[0] / sp, p.speed[1] / sp, p.speed[2] / sp);
                let u = dir.any_perp();
                let w = dir.cross(u);
                let r = p.width / 2.0;
                let steps = (r / s_px).floor() as i32;
                let mut disc = vec![];
                for a in -steps..=steps {
                    for b in -steps..=steps {
                        let (x, y) = (a as f64 * s_px, b as f64 * s_px);
                        if x * x + y * y <= r * r + 1e-9 {
                            disc.push((x, y));
                        }
                    }
                }
                if disc.is_empty() {
                    disc.push((0.0, 0.0));
                }
                let mut pts = vec![];
                for j in 0..due {
                    let t = (j as f64 + 0.5) / due as f64 * dt;
                    let back = sp * (dt - t);
                    for (x, y) in &disc {
                        let c = v3(
                            p.at[0] + dir.x * back,
                            p.at[1] + dir.y * back,
                            p.at[2] + dir.z * back,
                        ) + u * *x
                            + w * *y;
                        pts.push((
                            v3(c.x / sim.ppm, c.y / sim.ppm, c.z / sim.ppm),
                            v3(
                                p.speed[0] / sim.ppm,
                                p.speed[1] / sim.ppm,
                                p.speed[2] / sim.ppm,
                            ),
                        ));
                    }
                }
                p.emitted = target;
                sim.fluids[i].add_with_velocity(&pts);
            }
        }
    }

    pub fn fluid3_emit_for(&mut self, dt: f64) {
        self.fluid3_emit(dt);
    }

    pub fn has_fluid3(&self) -> bool {
        self.fluid3.is_some()
    }

    pub fn fluid3_snapshot(&self) -> Option<J> {
        let sim = self.fluid3.as_ref()?;
        let out: Vec<J> = sim
            .names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let c = &self.fluid_cfg[n];
                let f = &sim.fluids[i];
                let ppm = sim.ppm;
                let particles = f32s(f.pos.iter().flat_map(|p| [p.x * ppm, p.y * ppm, p.z * ppm]));
                let foam =
                    f32s(f.foam.iter().flat_map(|q| {
                        [q.pos.x * ppm, q.pos.y * ppm, q.pos.z * ppm, q.age / q.life]
                    }));
                json!({
                    "name": n, "spacing": self.spacing3(c), "smoothness": c.smoothness,
                    "color": hex(c.color), "color_end": hex(c.color_end),
                    "render": if c.blob { "surface" } else { "dots" },
                    "count": f.len(), "particles": particles,
                    "foam_count": f.foam.len(), "foam": foam,
                })
            })
            .collect();
        Some(J::Array(out))
    }
}
