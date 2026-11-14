//! Lights: `sun_light`, `point_light`, `spot_light` and `ambient_light` make hidden shapes that carry a
//! `LightDef`. Their position (`at`), turns (which rotate `dir`), fill colour and opacity (a multiplier on
//! `intensity`) are animated like any shape's, so moves, fades, colour changes and equations all work on
//! them. The snapshot lists every light under `lights`; a scene with at least one light is lit
//! photographically by the renderer.
use crate::interp::Interp;
use crate::value::*;
use animscript_core::mesh::Transform3;
use animscript_core::*;
use serde_json::{json, Value as J};

type Args = Vec<(Option<String>, Value)>;

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-LIT-001", msg)
}
fn e2(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-LIT-002", msg)
}

pub const LIGHT_BUILTINS: &[&str] = &["sun_light", "point_light", "spot_light", "ambient_light"];

impl Interp {
    fn light_triple(&self, v: &Value, what: &str) -> Res<[f64; 3]> {
        match v {
            Value::Tuple(t) if t.len() == 2 || t.len() == 3 => {
                let mut p = [0.0; 3];
                for (k, x) in t.iter().enumerate() {
                    p[k] = self.num(x, what).map_err(|e| e2(e.message))?;
                }
                Ok(p)
            }
            o => Err(e2(format!(
                "{what} must be (x, y, z), found {}",
                o.type_name()
            ))),
        }
    }

    pub(crate) fn light_builtin(&mut self, name: &str, args: Args) -> Res<Value> {
        let kind = name.trim_end_matches("_light");
        let allowed: &[&str] = match kind {
            "sun" => &["dir", "color", "intensity", "size", "shadows"],
            "point" => &["at", "color", "intensity", "reach", "size", "shadows"],
            "spot" => &[
                "at",
                "dir",
                "color",
                "intensity",
                "reach",
                "size",
                "angle",
                "softness",
                "shadows",
            ],
            _ => &["color", "intensity", "ground"],
        };
        let required: &[&str] = match kind {
            "point" | "spot" => &["at"],
            "sun" => &[],
            _ => &[],
        };
        for (k, _) in &args {
            match k {
                None => {
                    return Err(e1(format!(
                        "`{name}` takes named settings only, such as `intensity: 2`"
                    )))
                }
                Some(k) if !allowed.contains(&k.as_str()) => {
                    return Err(e1(format!("`{name}` has no setting `{k}`"))
                        .help(format!("settings: {}", allowed.join(", "))))
                }
                _ => {}
            }
        }
        for r in required {
            if !args.iter().any(|(k, _)| k.as_deref() == Some(r)) {
                return Err(e1(format!("`{name}` needs `{r}:`")));
            }
        }
        let m = self.bind(name, args, allowed, &[])?;
        let positive = |this: &Interp, key: &str, d: f64| -> Res<f64> {
            match m.get(key) {
                None => Ok(d),
                Some(v) => {
                    let x = this
                        .num(v, &format!("`{key}`"))
                        .map_err(|e| e2(e.message))?;
                    if x.is_nan() || x < 0.0 || !x.is_finite() {
                        return Err(e2(format!("`{key}` cannot be negative")));
                    }
                    Ok(x)
                }
            }
        };
        let colour = |key: &str| -> Res<Option<[u8; 4]>> {
            match m.get(key) {
                None => Ok(None),
                Some(Value::Color(c)) => Ok(Some(*c)),
                Some(o) => Err(e2(format!(
                    "`{key}` must be a colour such as #ffd0a0, found {}",
                    o.type_name()
                ))),
            }
        };
        let mut def = LightDef {
            kind: kind.to_string(),
            dir: [0.0, 1.0, 0.0],
            intensity: match kind {
                "ambient" => 0.3,
                "sun" => 3.0,
                _ => 2.0,
            },
            reach: 200.0,
            size: match kind {
                "sun" => 0.5,
                "ambient" => 0.0,
                _ => 6.0,
            },
            angle: 40f64.to_radians(),
            softness: 0.25,
            ground: colour("ground")?,
            shadows: match m.get("shadows") {
                None => true,
                Some(Value::Bool(b)) => *b,
                Some(o) => {
                    return Err(e2(format!(
                        "`shadows` is true or false, found {}",
                        o.type_name()
                    )))
                }
            },
        };
        def.intensity = positive(self, "intensity", def.intensity)?;
        def.reach = positive(self, "reach", def.reach)?;
        if def.reach == 0.0 {
            return Err(e2("`reach` must be above zero"));
        }
        def.size = positive(self, "size", def.size)?;
        if let Some(v) = m.get("dir") {
            let d = self.light_triple(v, "`dir`")?;
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if l < 1e-12 {
                return Err(e2("`dir` needs a direction, not (0, 0, 0)"));
            }
            def.dir = [d[0] / l, d[1] / l, d[2] / l];
        }
        if let Some(v) = m.get("angle") {
            let a = self.angle(v, "`angle`").map_err(|e| e2(e.message))? * std::f64::consts::TAU;
            if !(a > 0.0 && a < std::f64::consts::FRAC_PI_2 + 1e-9) {
                return Err(e2(
                    "a spot's `angle` is a half-angle between 0 and 90 degrees",
                ));
            }
            def.angle = a;
        }
        if let Some(v) = m.get("softness") {
            let s = self.num(v, "`softness`").map_err(|e| e2(e.message))?;
            if !(0.0..=1.0).contains(&s) {
                return Err(e2("`softness` is between 0 and 1"));
            }
            def.softness = s;
        }
        let mut sh = Shape::new(name);
        sh.visible = false;
        sh.fill = Some(colour("color")?.unwrap_or([255, 255, 255, 255]));
        if let Some(v) = m.get("at") {
            let p = self.light_triple(v, "`at`")?;
            sh.position = [p[0], p[1]];
            sh.pos_z = p[2];
        }
        sh.light = Some(def);
        Ok(Value::Fresh(Box::new(sh)))
    }

    /// `shape.lit(bevel: 0, roughness: 0.6, metallic: 0, emissive: 0, occluder: false)` on a 2D shape: the
    /// scene's lights shade it (as a flat sheet at the shape's depth, `pos_z`, rounded at its edges by `bevel`).
    pub fn lit_method(&mut self, sn: &str, args: Args) -> Res<Value> {
        if self.is_mesh_shape(sn) || sn.contains('.') {
            return Err(e1(format!("`lit` is for flat 2D shapes; `{sn}` is not one (meshes are always lit when the scene has lights)")));
        }
        let m = self.bind(
            "lit",
            args,
            &[
                "bevel",
                "roughness",
                "metallic",
                "emissive",
                "occluder",
                "depth",
            ],
            &[],
        )?;
        let mut lit = Lit2D {
            bevel: 0.0,
            surface: Surface::default(),
            occluder: false,
            depth: 0.0,
        };
        let num = |this: &Interp, k: &str, lo: f64, hi: f64| -> Res<Option<f64>> {
            match m.get(k) {
                None => Ok(None),
                Some(v) => {
                    let x = this.num(v, &format!("`{k}`")).map_err(|e| e2(e.message))?;
                    if !(lo..=hi).contains(&x) {
                        return Err(e2(format!("`{k}` is between {lo} and {hi}")));
                    }
                    Ok(Some(x))
                }
            }
        };
        if let Some(x) = num(self, "bevel", 0.0, 1000.0)? {
            lit.bevel = x;
        }
        if let Some(x) = num(self, "roughness", 0.0, 1.0)? {
            lit.surface.roughness = x;
        }
        if let Some(x) = num(self, "metallic", 0.0, 1.0)? {
            lit.surface.metallic = x;
        }
        if let Some(x) = num(self, "depth", -1e6, 1e6)? {
            lit.depth = x;
        }
        if let Some(x) = num(self, "emissive", 0.0, 50.0)? {
            lit.surface.emissive = x;
        }
        match m.get("occluder") {
            None => {}
            Some(Value::Bool(b)) => lit.occluder = *b,
            Some(o) => {
                return Err(e2(format!(
                    "`occluder` is true or false, found {}",
                    o.type_name()
                )))
            }
        }
        self.shapes.get_mut(sn).expect("shape").lit = Some(lit);
        Ok(Value::None)
    }

    /// Every light as the renderer needs it this tick (colour in linear light, `intensity` already
    /// multiplied by the opacity); `None` when the program has no lights.
    pub fn lights_json(&self, resolved: &crate::snapshot::Resolved) -> Option<J> {
        let mut out = vec![];
        for n in &self.shape_order {
            let sh = &self.shapes[n];
            let Some(def) = &sh.light else { continue };
            let tf = resolved
                .tf
                .get(n)
                .copied()
                .unwrap_or_else(|| sh.transform());
            let t3 = resolved
                .t3
                .get(n)
                .copied()
                .unwrap_or([sh.pos_z, sh.rot_x, sh.rot_y, sh.scale_z]);
            let opacity = resolved
                .opacity
                .get(n)
                .copied()
                .unwrap_or(sh.opacity)
                .clamp(0.0, 1.0);
            let col = resolved.shape_fill.get(n).copied().unwrap_or_else(|| {
                crate::color::to_linear(sh.fill.unwrap_or([255, 255, 255, 255]))
            });
            let turn = Transform3 {
                position: [0.0; 3],
                rot: [t3[1], t3[2], tf.rotation],
                scale: [1.0; 3],
                pivot: [0.0; 3],
                orient: None,
            };
            let d = turn.apply(def.dir);
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
            let clean = |v: f64| if v.abs() < 1e-15 { 0.0 } else { v };
            let mut j = json!({
                "id": n, "kind": def.kind,
                "pos": [tf.position[0], tf.position[1], t3[0]],
                "dir": [clean(d[0] / l), clean(d[1] / l), clean(d[2] / l)],
                "color": [col[0], col[1], col[2]],
                "intensity": def.intensity * opacity,
                "reach": def.reach, "size": def.size, "angle": def.angle, "softness": def.softness, "shadows": def.shadows,
            });
            if let Some(g) = def.ground {
                let g = crate::color::to_linear(g);
                j["ground"] = json!([g[0], g[1], g[2]]);
            }
            out.push(j);
        }
        (!out.is_empty()).then_some(J::Array(out))
    }
}
