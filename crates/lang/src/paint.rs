//! Painting: `brush(...)` values, `paint_layer()` shapes and their strokes. A paint layer is a shape without
//! geometry (so it has a position, scale, rotation, opacity, z and visibility like any shape) that owns a list of
//! strokes. A stroke is a brush, the points of the pen path and a *progress* (0 to 1) that animates like any
//! property, so `art.stroke(ink, points) over 2.seconds` draws it in. The snapshot carries the strokes; the
//! renderer replays them with libmypaint (see `animscript-render`), so the picture is a pure function of the snapshot.
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::*;
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

type Args = Vec<(Option<String>, Value)>;

/// A brush as the program describes it: a bundled name or a `.myb` file, plus settings that override it.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushSpec {
    /// A bundled brush name, or the path given for a file brush.
    pub source: String,
    /// The text of the `.myb` file for a file brush (bundled brushes are found by name).
    pub myb: Option<String>,
    /// libmypaint settings to override, by name.
    pub set: Vec<(String, f64)>,
    pub color: Option<[f64; 3]>,
}

impl BrushSpec {
    /// The overrides as libmypaint wants them, in the order given.
    pub fn json(&self) -> J {
        json!({"source": self.source, "myb": self.myb, "set": self.set.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(), "color": self.color})
    }
}

/// A stroke is repeated around a centre (libmypaint's symmetry types).
#[derive(Clone, Debug, PartialEq)]
pub struct SymSpec {
    /// `vertical`, `horizontal`, `both`, `radial` or `snowflake`.
    pub kind: String,
    pub lines: i64,
    pub center: [f64; 2],
    /// The turn of the symmetry lines, in degrees.
    pub angle: f64,
}

impl SymSpec {
    pub fn json(&self) -> J {
        json!({"kind": self.kind, "lines": self.lines, "center": self.center, "angle": self.angle})
    }
}

#[derive(Clone, Debug)]
pub struct StrokeDef {
    pub brush: usize,
    /// `[x, y, pressure, seconds since the previous point, x tilt, y tilt]`.
    pub points: Vec<[f64; 6]>,
    pub progress: f64,
    /// Pen twist in turns.
    pub barrel: f64,
    pub symmetry: Option<SymSpec>,
    /// The compositing mode (`normal`, `multiply`, ...) and how much of the stroke is composited (0 to 1).
    pub blend: String,
    pub strength: f64,
}

#[derive(Clone, Debug)]
pub struct PaintLayer {
    pub brushes: Vec<BrushSpec>,
    pub strokes: Vec<StrokeDef>,
    /// Applied to strokes made afterwards that do not say otherwise.
    pub symmetry: Option<SymSpec>,
    /// How the whole layer is composited onto the scene.
    pub blend: String,
}

/// Strokes get channels of their own, so two strokes of one layer can be drawn at the same time.
pub const STROKE_CHANNEL: u32 = 0x4000_0000;

fn brush_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-BRU-001", msg)
}
fn stroke_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-BRU-002", msg)
}
fn mode_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-BRU-003", msg)
}

const SYMMETRIES: &[&str] = &["vertical", "horizontal", "both", "radial", "snowflake"];

impl Interp {
    /// `brush("pencil", radius: 4, color: #223, opacity: 0.8, hardness: 0.9, eraser: true, slow_tracking: 2, ...)`:
    /// a bundled brush (`pencil`, `pen`, `marker`, `charcoal`, `calligraphy`, `ink`, `smudge`, `eraser`, `rounded`)
    /// or the path of a `.myb` file. Named arguments set libmypaint settings by name; `radius` is in pixels.
    pub(crate) fn brush_builtin(&mut self, args: Args) -> Res<Value> {
        let mut it = args.into_iter();
        let Some((None, first)) = it.next() else {
            return Err(brush_err(
                "`brush` needs the brush's name or `.myb` path first",
            ));
        };
        let Value::Str(source) = first else {
            return Err(brush_err(format!(
                "a brush is named by a string, found {}",
                first.type_name()
            )));
        };
        let mut spec = BrushSpec {
            source: source.clone(),
            myb: None,
            set: vec![],
            color: None,
        };
        let mut probe = if animscript_brush::Brush::bundled_names().contains(&source.as_str()) {
            animscript_brush::Brush::bundled(&source).expect("bundled brushes load")
        } else if source.ends_with(".myb") || source.contains('/') {
            let text = std::fs::read_to_string(self.base_dir.join(&source))
                .map_err(|e| brush_err(format!("cannot read the brush file `{source}`: {e}")))?;
            let b = animscript_brush::Brush::from_myb(&text)
                .map_err(|e| brush_err(format!("`{source}`: {e}")))?;
            spec.myb = Some(text);
            b
        } else {
            return Err(
                brush_err(format!("there is no bundled brush `{source}`")).help(format!(
                    "bundled brushes: {}; or give the path of a `.myb` file",
                    animscript_brush::Brush::bundled_names().join(", ")
                )),
            );
        };
        for (name, v) in it {
            let Some(name) = name else {
                return Err(brush_err(
                    "settings of a brush are named, such as `radius: 4`",
                ));
            };
            let name = if name == "opacity" {
                "opaque".to_string()
            } else {
                name
            }; // the friendly name for libmypaint's `opaque`
            match (name.as_str(), &v) {
                ("color", Value::Color(c)) => {
                    spec.color = Some([
                        c[0] as f64 / 255.0,
                        c[1] as f64 / 255.0,
                        c[2] as f64 / 255.0,
                    ]);
                }
                ("color", other) => {
                    return Err(brush_err(format!(
                        "`color` needs a colour, found {}",
                        other.type_name()
                    )))
                }
                (_, v) => {
                    let n = match v {
                        Value::Bool(b) => f64::from(u8::from(*b)),
                        other => other.as_f64().ok_or_else(|| {
                            brush_err(format!(
                                "`{name}` needs a number, found {}",
                                other.type_name()
                            ))
                        })?,
                    };
                    // `radius` is in pixels, libmypaint keeps its logarithm
                    let (key, val) = if name == "radius" {
                        if n <= 0.0 {
                            return Err(brush_err("`radius` must be positive"));
                        }
                        ("radius_logarithmic".to_string(), n.ln())
                    } else {
                        (name.clone(), n)
                    };
                    probe.set(&key, val as f32).map_err(brush_err)?; // unknown settings are caught here
                    spec.set.push((key, val));
                }
            }
        }
        Ok(Value::Brush(Rc::new(spec)))
    }

    /// `paint_layer(z: 0, resolution: 1)`: a layer the size of the canvas that strokes are painted on. Declare it like
    /// a shape: `shape art = paint_layer()`. A `resolution` above 1 paints at more pixels per canvas pixel (for zoom).
    pub(crate) fn paint_layer_builtin(&mut self, args: Args) -> Res<Value> {
        let m = self.bind("paint_layer", args, &["z", "resolution"], &[])?;
        let mut sh = Shape::new("paint");
        if let Some(z) = m.get("z") {
            sh.z = self.int(z, "`z`")?;
        }
        let res = m
            .get("resolution")
            .map(|v| self.num(v, "`resolution`"))
            .transpose()?
            .unwrap_or(1.0);
        if !(0.25..=4.0).contains(&res) {
            return Err(brush_err("`resolution` is between 0.25 and 4"));
        }
        sh.paint = Some(PaintInfo {
            size: [self.canvas.0, self.canvas.1],
            resolution: res,
        });
        Ok(Value::Fresh(Box::new(sh)))
    }

    /// Called when a shape is registered: a paint layer gets its stroke list and a pivot at the canvas centre.
    pub(crate) fn register_paint(&mut self, name: &str) {
        let c = [self.canvas.0 as f64 / 2.0, self.canvas.1 as f64 / 2.0];
        if let Some(sh) = self.shapes.get_mut(name) {
            if sh.paint.is_some() {
                sh.pivot = Some(c);
                self.paints.insert(
                    name.to_string(),
                    PaintLayer {
                        brushes: vec![],
                        strokes: vec![],
                        symmetry: None,
                        blend: "normal".into(),
                    },
                );
            }
        }
    }

    /// A symmetry from the `symmetry`, `lines`, `center` and `angle` arguments: `Ok(None)` for `none`, and the layer's
    /// default when `symmetry` is absent.
    fn symmetry_args(
        &self,
        kind: Option<&Value>,
        m: &HashMap<&'static str, Value>,
    ) -> Res<Option<Option<SymSpec>>> {
        let kind = match kind {
            None => return Ok(None),
            Some(Value::None) => return Ok(Some(None)),
            Some(Value::Str(k)) => k.clone(),
            Some(o) => {
                return Err(mode_err(format!(
                    "`symmetry` names a kind or is `none`, found {}",
                    o.type_name()
                )))
            }
        };
        if !SYMMETRIES.contains(&kind.as_str()) {
            return Err(mode_err(format!("there is no symmetry `{kind}`"))
                .help(format!("kinds: {}", SYMMETRIES.join(", "))));
        }
        let lines = m
            .get("lines")
            .map(|v| self.int(v, "`lines`"))
            .transpose()?
            .unwrap_or(6);
        if !(2..=64).contains(&lines) {
            return Err(mode_err("`lines` is between 2 and 64"));
        }
        let center = match m.get("center") {
            Some(v) => self.point(v, "`center`")?,
            None => [self.canvas.0 as f64 / 2.0, self.canvas.1 as f64 / 2.0],
        };
        let angle = m
            .get("angle")
            .map(|v| self.angle(v, "`angle`"))
            .transpose()?
            .unwrap_or(0.0)
            * 360.0;
        Ok(Some(Some(SymSpec {
            kind,
            lines,
            center,
            angle,
        })))
    }

    /// `layer.stroke(brush, points, speed:, over:, ease:, tilt:, barrel:, symmetry:, lines:, center:, angle:, blend:,
    /// strength:)`, `layer.symmetry(kind, lines:, center:, angle:)` (a default for later strokes; `none` clears it)
    /// and `layer.clear_strokes()`.
    pub(crate) fn paint_method(
        &mut self,
        sn: &str,
        name: &str,
        args: Args,
        mode: Mode,
        span: Span,
    ) -> Res<Value> {
        if name == "clear_strokes" {
            self.bind(name, args, &[], &[])?;
            self.paints
                .get_mut(sn)
                .expect("a paint layer")
                .strokes
                .clear();
            return Ok(Value::None);
        }
        if name == "symmetry" {
            let m = self.bind(name, args, &["kind", "lines", "center", "angle"], &["kind"])?;
            let sym = self
                .symmetry_args(m.get("kind"), &m)?
                .expect("kind is required");
            self.paints.get_mut(sn).expect("a paint layer").symmetry = sym;
            return Ok(Value::None);
        }
        let m = self.bind(
            name,
            args,
            &[
                "brush", "points", "speed", "over", "ease", "tilt", "barrel", "symmetry", "lines",
                "center", "angle", "blend", "strength",
            ],
            &["brush", "points"],
        )?;
        let Value::Brush(spec) = &m["brush"] else {
            return Err(stroke_err(format!(
                "`stroke` needs a brush from `brush(...)`, found {}",
                m["brush"].type_name()
            )));
        };
        let Value::List(items) = &m["points"] else {
            return Err(stroke_err(format!(
                "the points of a stroke are a list of (x, y) tuples, found {}",
                m["points"].type_name()
            )));
        };
        let speed = m
            .get("speed")
            .map(|v| self.num(v, "`speed`"))
            .transpose()?
            .unwrap_or(300.0);
        if speed <= 0.0 {
            return Err(stroke_err("`speed` must be positive (pixels per second)"));
        }
        // the tilt of the whole stroke: a pair, or `follow` (the pen leans in the direction of travel)
        enum Tilt {
            None,
            Fixed([f64; 2]),
            Follow,
        }
        let tilt = match m.get("tilt") {
            None | Some(Value::None) => Tilt::None,
            Some(Value::Str(s)) if s == "follow" => Tilt::Follow,
            Some(v) => {
                let t = self.point(v, "`tilt`").map_err(|_| {
                    mode_err("`tilt` is a pair `(x, y)` from -1 to 1, or `\"follow\"`")
                })?;
                if t.iter().any(|x| !(-1.0..=1.0).contains(x)) {
                    return Err(stroke_err("tilt is between -1 and 1"));
                }
                Tilt::Fixed(t)
            }
        };
        let barrel = m
            .get("barrel")
            .map(|v| self.angle(v, "`barrel`"))
            .transpose()?
            .unwrap_or(0.0);
        let blend = match m.get("blend") {
            None => "normal".to_string(),
            Some(Value::Str(b)) if animscript_brush::Blend::from_name(b).is_some() => b.clone(),
            Some(Value::Str(b)) => {
                return Err(
                    mode_err(format!("there is no blend mode `{b}`")).help(format!(
                        "modes: {}",
                        animscript_brush::Blend::names().join(", ")
                    )),
                )
            }
            Some(o) => {
                return Err(mode_err(format!(
                    "`blend` names a mode, found {}",
                    o.type_name()
                )))
            }
        };
        let strength = m
            .get("strength")
            .map(|v| self.num(v, "`strength`"))
            .transpose()?
            .unwrap_or(1.0);
        if !(0.0..=1.0).contains(&strength) {
            return Err(mode_err("`strength` is between 0 and 1"));
        }
        let explicit_sym = self.symmetry_args(m.get("symmetry"), &m)?;
        let mut pts: Vec<[f64; 6]> = vec![];
        let mut explicit_tilt: Vec<bool> = vec![];
        for p in items {
            let Value::Tuple(t) = p else {
                return Err(stroke_err(format!("a stroke point is a tuple `(x, y)`, `(x, y, pressure)`, `(x, y, pressure, seconds)` or with the tilt `(x, y, pressure, seconds, xtilt, ytilt)`, found {}", p.type_name())));
            };
            if !(2..=6).contains(&t.len()) || t.len() == 5 {
                return Err(stroke_err("a stroke point has 2, 3, 4 or 6 numbers: x, y, pressure, seconds since the last point, x tilt, y tilt"));
            }
            let n: Vec<f64> = t
                .iter()
                .map(|v| self.num(v, "a stroke point"))
                .collect::<Res<_>>()?;
            let pressure = n.get(2).copied().unwrap_or(0.7);
            if !(0.0..=1.0).contains(&pressure) {
                return Err(stroke_err("pressure is between 0 and 1"));
            }
            let dt = match n.get(3) {
                Some(d) if *d >= 0.0 => *d,
                Some(_) => return Err(stroke_err("the time between points cannot be negative")),
                None => match pts.last() {
                    Some(q) => {
                        (((n[0] - q[0]).powi(2) + (n[1] - q[1]).powi(2)).sqrt() / speed).max(0.001)
                    }
                    None => 0.01,
                },
            };
            let (xt, yt) = (
                n.get(4).copied().unwrap_or(0.0),
                n.get(5).copied().unwrap_or(0.0),
            );
            if !(-1.0..=1.0).contains(&xt) || !(-1.0..=1.0).contains(&yt) {
                return Err(stroke_err("tilt is between -1 and 1"));
            }
            explicit_tilt.push(n.len() == 6);
            pts.push([n[0], n[1], pressure, dt, xt, yt]);
        }
        if pts.is_empty() {
            return Err(stroke_err("a stroke needs at least one point"));
        }
        for i in 0..pts.len() {
            if explicit_tilt[i] {
                continue;
            }
            match tilt {
                Tilt::None => {}
                Tilt::Fixed(t) => {
                    pts[i][4] = t[0];
                    pts[i][5] = t[1];
                }
                Tilt::Follow => {
                    let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(pts.len() - 1)]);
                    let d = (b[1] - a[1]).atan2(b[0] - a[0]);
                    if (b[0] - a[0]).abs() + (b[1] - a[1]).abs() > 1e-9 {
                        pts[i][4] = 0.6 * d.cos();
                        pts[i][5] = 0.6 * d.sin();
                    }
                }
            }
        }
        let timed = self.over_frames(&m, mode, span)?;
        let layer = self.paints.get_mut(sn).expect("a paint layer");
        let symmetry = match explicit_sym {
            Some(s) => s,
            None => layer.symmetry.clone(),
        };
        let brush = match layer.brushes.iter().position(|b| b == &**spec) {
            Some(i) => i,
            None => {
                layer.brushes.push((**spec).clone());
                layer.brushes.len() - 1
            }
        };
        let idx = layer.strokes.len() as u32;
        // while the scene runs a timed stroke starts empty and is drawn in; otherwise it is there at once
        layer.strokes.push(StrokeDef {
            brush,
            points: pts,
            progress: if timed.is_some() && mode == Mode::Run {
                0.0
            } else {
                1.0
            },
            barrel,
            symmetry,
            blend,
            strength,
        });
        match timed {
            Some((frames, ease)) if mode == Mode::Run => Ok(Value::Pending(Box::new(ActionReq {
                shape: sn.to_string(),
                label: format!("{sn}.stroke"),
                frames,
                ease: if m.contains_key("ease") {
                    ease
                } else {
                    Ease::Linear
                },
                channels: vec![STROKE_CHANNEL + idx],
                comps: vec![(PropId::StrokeProgress(idx), Tgt::To(1.0))],
                motions: vec![],
            }))),
            _ => Ok(Value::None),
        }
    }

    /// `layer.blend = "multiply"`: how the whole layer is composited onto the scene.
    pub(crate) fn set_layer_blend(&mut self, sn: &str, v: &Value) -> Res<()> {
        match v {
            Value::Str(b) if animscript_brush::Blend::from_name(b).is_some() => {
                self.paints.get_mut(sn).expect("a paint layer").blend = b.clone();
                Ok(())
            }
            Value::Str(b) => Err(
                mode_err(format!("there is no blend mode `{b}`")).help(format!(
                    "modes: {}",
                    animscript_brush::Blend::names().join(", ")
                )),
            ),
            o => Err(mode_err(format!(
                "`blend` names a mode, found {}",
                o.type_name()
            ))),
        }
    }

    /// The `paint` part of a layer's snapshot entry: brushes and strokes with their current progress.
    pub fn paint_snapshot(&self, name: &str, progress: &BTreeMap<(String, u32), f64>) -> Option<J> {
        let layer = self.paints.get(name)?;
        let info = self.shapes[name].paint?;
        let strokes: Vec<J> = layer
            .strokes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let p = progress
                    .get(&(name.to_string(), i as u32))
                    .copied()
                    .unwrap_or(s.progress)
                    .clamp(0.0, 1.0);
                json!({
                    "brush": s.brush, "progress": p, "points": s.points,
                    "barrel": s.barrel, "symmetry": s.symmetry.as_ref().map(|y| y.json()),
                    "blend": s.blend, "strength": s.strength,
                })
            })
            .collect();
        Some(json!({
            "size": info.size, "resolution": info.resolution, "blend": layer.blend,
            "brushes": layer.brushes.iter().map(|b| b.json()).collect::<Vec<_>>(),
            "strokes": strokes,
        }))
    }
}
