//! Sculpting in the language. `sculpt(size:, voxel:, at:)` makes a mesh shape backed by a signed-distance volume;
//! shapes written as distance functions (`sdf_sphere(...)`, `sdf_expr("...")`, with `.at()`, `.twist()`, `.noise()` ...)
//! are added to it, subtracted from it or intersected with it; brushes work on it along paths; noise roughens it;
//! vortices and flows stir it. After every operation the mesh of the shape is rebuilt, so the picture always shows
//! the current sculpt. The sculpt works in world pixels like any mesh.
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_core::mesh::Mesh as CoreMesh;
use animscript_core::*;
use animscript_sculpt as sc;
use animscript_sculpt::brush::{Brush, Falloff, Kind as BrushKind};
use animscript_sculpt::flow::{advect, reinitialize, Field, Vortex};
use animscript_sculpt::noise::{Kind as NoiseKind, Spec};
use animscript_sculpt::sdf::{self, Op, Sdf};
use std::rc::Rc;
use std::sync::Arc;

type Args = Vec<(Option<String>, Value)>;

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-SCL-001", msg)
}
fn e2(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-SCL-002", msg)
}
fn e4(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-SCL-004", msg)
}

/// A distance function as a value.
pub struct SdfBox(pub Sdf);
impl std::fmt::Debug for SdfBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sdf(..)")
    }
}

pub struct SculptState {
    pub vol: sc::Volume,
    pub lo: sc::V3,
    pub hi: sc::V3,
}

pub const SDF_BUILTINS: &[&str] = &[
    "sdf_sphere",
    "sdf_box",
    "sdf_torus",
    "sdf_capsule",
    "sdf_cylinder",
    "sdf_cone",
    "sdf_ellipsoid",
    "sdf_plane",
    "sdf_expr",
];
pub const SCULPT_METHODS: &[&str] = &[
    "add",
    "subtract",
    "intersect",
    "brush",
    "noise",
    "vortex",
    "flow",
    "wind",
    "reinit",
    "clear",
    "volume",
];
const SDF_METHODS: &[&str] = &[
    "at",
    "rotate",
    "scale",
    "twist",
    "bend",
    "repeat",
    "mirror",
    "shell",
    "offset",
    "noise",
    "union",
    "subtract",
    "intersect",
];

impl Interp {
    fn sc_triple(&self, v: &Value, what: &str) -> Res<sc::V3> {
        match v {
            Value::Tuple(t) if t.len() == 3 => Ok([
                self.num(&t[0], what)?,
                self.num(&t[1], what)?,
                self.num(&t[2], what)?,
            ]),
            o => Err(e1(format!(
                "{what} must be (x, y, z), found {}",
                o.type_name()
            ))),
        }
    }
    fn sc_num(
        &self,
        m: &std::collections::HashMap<&'static str, Value>,
        k: &str,
        d: f64,
    ) -> Res<f64> {
        match m.get(k) {
            None => Ok(d),
            Some(v) => self.num(v, &format!("`{k}`")).map_err(|e| e1(e.message)),
        }
    }
    fn sc_axis(&self, v: &Value) -> Res<usize> {
        match v {
            Value::Str(s) if s == "x" => Ok(0),
            Value::Str(s) if s == "y" => Ok(1),
            Value::Str(s) if s == "z" => Ok(2),
            _ => Err(e2("an axis is \"x\", \"y\" or \"z\"")),
        }
    }

    pub(crate) fn sculpt_builtin(&mut self, name: &str, args: Args) -> Res<Value> {
        match name {
            "sculpt" => {
                let m = self.bind(name, args, &["size", "voxel", "at", "band"], &["size"])?;
                let size = self.sc_triple(&m["size"], "`size`")?;
                if size.iter().any(|s| *s <= 0.0) {
                    return Err(e1("`size` must be positive in every direction"));
                }
                let h = self.sc_num(&m, "voxel", size[0].min(size[1]).min(size[2]) / 48.0)?;
                if h <= 0.0 {
                    return Err(e1("`voxel` (the size of one voxel in px) must be positive"));
                }
                let dims = [size[0] / h, size[1] / h, size[2] / h];
                if dims.iter().any(|d| *d > 400.0) || dims[0] * dims[1] * dims[2] > 20_000_000.0 {
                    return Err(e4(format!(
                        "a sculpt of {:.0} x {:.0} x {:.0} voxels is too big; use a larger `voxel`",
                        dims[0], dims[1], dims[2]
                    )));
                }
                let at = match m.get("at") {
                    Some(v) => self.sc_triple(v, "`at`")?,
                    None => [0.0; 3],
                };
                let band = self.sc_num(&m, "band", 3.0)?;
                if band < 1.5 {
                    return Err(e1("`band` is at least 1.5 voxels"));
                }
                let id = self.sculpts.len() as u32;
                let half = [size[0] / 2.0, size[1] / 2.0, size[2] / 2.0];
                self.sculpts.push(SculptState {
                    vol: sc::Volume::new(h, band),
                    lo: [at[0] - half[0], at[1] - half[1], at[2] - half[2]],
                    hi: [at[0] + half[0], at[1] + half[1], at[2] + half[2]],
                });
                let mut sh = Shape::new("mesh");
                sh.pivot3 = at;
                sh.mesh = Some(CoreMesh {
                    verts: vec![at],
                    faces: vec![],
                    double_sided: false,
                });
                sh.sculpt = Some(id);
                Ok(Value::Fresh(Box::new(sh)))
            }
            _ => {
                let sdf = self.sdf_primitive(name, args)?;
                Ok(Value::Sdf(Rc::new(SdfBox(sdf))))
            }
        }
    }

    fn sdf_primitive(&mut self, name: &str, args: Args) -> Res<Sdf> {
        Ok(match name {
            "sdf_sphere" => {
                let m = self.bind(name, args, &["radius"], &["radius"])?;
                Sdf::Sphere(self.sc_num(&m, "radius", 1.0)?)
            }
            "sdf_box" => {
                let m = self.bind(name, args, &["size", "round"], &["size"])?;
                let s = self.sc_triple(&m["size"], "`size`")?;
                Sdf::Box(
                    [s[0] / 2.0, s[1] / 2.0, s[2] / 2.0],
                    self.sc_num(&m, "round", 0.0)?,
                )
            }
            "sdf_torus" => {
                let m = self.bind(name, args, &["major", "minor"], &["major", "minor"])?;
                Sdf::Torus(
                    self.sc_num(&m, "major", 1.0)?,
                    self.sc_num(&m, "minor", 1.0)?,
                )
            }
            "sdf_capsule" => {
                let m = self.bind(name, args, &["a", "b", "radius"], &["a", "b", "radius"])?;
                Sdf::Capsule(
                    self.sc_triple(&m["a"], "`a`")?,
                    self.sc_triple(&m["b"], "`b`")?,
                    self.sc_num(&m, "radius", 1.0)?,
                )
            }
            "sdf_cylinder" => {
                let m = self.bind(name, args, &["radius", "height"], &["radius", "height"])?;
                Sdf::Cylinder(
                    self.sc_num(&m, "radius", 1.0)?,
                    self.sc_num(&m, "height", 1.0)? / 2.0,
                )
            }
            "sdf_cone" => {
                let m = self.bind(
                    name,
                    args,
                    &["radius", "height", "top"],
                    &["radius", "height"],
                )?;
                Sdf::Cone(
                    self.sc_num(&m, "radius", 1.0)?,
                    self.sc_num(&m, "top", 0.0)?,
                    self.sc_num(&m, "height", 1.0)? / 2.0,
                )
            }
            "sdf_ellipsoid" => {
                let m = self.bind(name, args, &["radii"], &["radii"])?;
                Sdf::Ellipsoid(self.sc_triple(&m["radii"], "`radii`")?)
            }
            "sdf_plane" => {
                let m = self.bind(name, args, &["normal", "offset"], &["normal"])?;
                let n = self.sc_triple(&m["normal"], "`normal`")?;
                if sc::len(n) < 1e-12 {
                    return Err(e1("a plane's `normal` cannot be (0, 0, 0)"));
                }
                Sdf::Plane(sc::norm(n), self.sc_num(&m, "offset", 0.0)?)
            }
            _ => {
                let m = self.bind(name, args, &["equation"], &["equation"])?;
                let Value::Str(src) = &m["equation"] else {
                    return Err(e1(
                        "`sdf_expr` takes text such as \"sqrt(x*x + y*y + z*z) - 50\"",
                    ));
                };
                Sdf::Expr(Arc::new(sdf::Expr::compile(src).map_err(|e| e2(format!("the equation `{src}` is not valid: {e}")).help("it may use x, y, z and the usual functions; its value is the distance (negative inside)"))?))
            }
        })
    }

    fn noise_spec(&self, m: &std::collections::HashMap<&'static str, Value>) -> Res<Spec> {
        let kind = match m.get("kind") {
            None => NoiseKind::Fbm,
            Some(Value::Str(s)) => NoiseKind::parse(s).ok_or_else(|| {
                e2(format!("unknown noise `{s}`"))
                    .help("perlin, value, worley, fbm, ridged, billow")
            })?,
            Some(o) => {
                return Err(e2(format!(
                    "`kind` is a noise name, found {}",
                    o.type_name()
                )))
            }
        };
        let scale = self.sc_num(m, "scale", 20.0)?;
        if scale <= 0.0 {
            return Err(e2(
                "a noise `scale` (the size of its features in px) must be positive",
            ));
        }
        let oct = self.sc_num(m, "octaves", 3.0)?;
        let seed = self.sc_num(m, "seed", 0.0)?;
        Ok(Spec::new(kind, scale, oct as u32, seed as u32))
    }

    pub(crate) fn sdf_method(&mut self, s: Rc<SdfBox>, name: &str, args: Args) -> Res<Value> {
        if !SDF_METHODS.contains(&name) {
            return Err(Diagnostic::new(
                "E-NAM-003",
                format!("a distance shape has no member `{name}`"),
            )
            .help(format!("methods: {}", SDF_METHODS.join(", "))));
        }
        let me = s.0.clone();
        let out = match name {
            "at" => {
                let m = self.bind(name, args, &["position"], &["position"])?;
                Sdf::Translate(Box::new(me), self.sc_triple(&m["position"], "`position`")?)
            }
            "rotate" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let Value::Tuple(t) = &m["by"] else {
                    return Err(e1(
                        "`rotate` takes three angles, like `(0.deg, 45.deg, 0.deg)`",
                    ));
                };
                if t.len() != 3 {
                    return Err(e1("`rotate` takes three angles (about x, y and z)"));
                }
                let a = |v: &Value| -> Res<f64> {
                    match v {
                        Value::Angle(a) => Ok(*a),
                        Value::Num(_) | Value::Int(_) => self.num(v, "an angle"),
                        _ => Err(e1("`rotate` takes angles")),
                    }
                };
                Sdf::Rotate(Box::new(me), [a(&t[0])?, a(&t[1])?, a(&t[2])?])
            }
            "scale" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                let k = self.sc_num(&m, "by", 1.0)?;
                if k <= 0.0 {
                    return Err(e1("`scale` must be positive"));
                }
                Sdf::Scale(Box::new(me), k)
            }
            "twist" => {
                let m = self.bind(name, args, &["turns", "axis"], &["turns"])?;
                let axis = m
                    .get("axis")
                    .map(|v| self.sc_axis(v))
                    .transpose()?
                    .unwrap_or(1);
                Sdf::Twist(Box::new(me), self.sc_num(&m, "turns", 0.0)?, axis)
            }
            "bend" => {
                let m = self.bind(name, args, &["curvature"], &["curvature"])?;
                Sdf::Bend(Box::new(me), self.sc_num(&m, "curvature", 0.0)?)
            }
            "repeat" => {
                let m = self.bind(name, args, &["every"], &["every"])?;
                Sdf::Repeat(Box::new(me), self.sc_triple(&m["every"], "`every`")?)
            }
            "mirror" => {
                let m = self.bind(name, args, &["axis"], &["axis"])?;
                Sdf::Mirror(Box::new(me), self.sc_axis(&m["axis"])?)
            }
            "shell" => {
                let m = self.bind(name, args, &["thickness"], &["thickness"])?;
                Sdf::Shell(Box::new(me), self.sc_num(&m, "thickness", 1.0)?)
            }
            "offset" => {
                let m = self.bind(name, args, &["by"], &["by"])?;
                Sdf::Offset(Box::new(me), self.sc_num(&m, "by", 0.0)?)
            }
            "noise" => {
                let m = self.bind(
                    name,
                    args,
                    &["kind", "scale", "amount", "seed", "octaves"],
                    &[],
                )?;
                let spec = self.noise_spec(&m)?;
                Sdf::Noise(
                    Box::new(me),
                    Arc::new(spec),
                    self.sc_num(&m, "amount", 3.0)?,
                )
            }
            _ => {
                let m = self.bind(name, args, &["other", "smooth"], &["other"])?;
                let Value::Sdf(o) = &m["other"] else {
                    return Err(e1(format!(
                        "`{name}` takes another distance shape, found {}",
                        m["other"].type_name()
                    )));
                };
                let k = self.sc_num(&m, "smooth", 0.0)?.max(0.0);
                match name {
                    "union" => Sdf::Union(vec![me, o.0.clone()], k),
                    "subtract" => Sdf::Subtract(Box::new(me), Box::new(o.0.clone()), k),
                    _ => Sdf::Intersect(vec![me, o.0.clone()], k),
                }
            }
        };
        Ok(Value::Sdf(Rc::new(SdfBox(out))))
    }

    /// Rebuild the mesh of a sculpt shape from its volume.
    fn sculpt_refresh(&mut self, sn: &str, id: usize) {
        let m = self.sculpts[id].vol.mesh();
        let sh = self.shapes.get_mut(sn).expect("shape");
        if m.tris.is_empty() {
            let c = sh.pivot3;
            sh.mesh = Some(CoreMesh {
                verts: vec![c],
                faces: vec![],
                double_sided: false,
            });
        } else {
            sh.mesh = Some(CoreMesh {
                verts: m.verts,
                faces: m.tris.iter().map(|t| vec![t[0], t[1], t[2]]).collect(),
                double_sided: false,
            });
        }
    }

    pub(crate) fn sculpt_method(
        &mut self,
        sn: &str,
        name: &str,
        args: Args,
        _mode: Mode,
    ) -> Res<Value> {
        let id = self.shapes[sn].sculpt.expect("a sculpt") as usize;
        match name {
            "add" | "subtract" | "intersect" => {
                let m = self.bind(name, args, &["shape", "smooth"], &["shape"])?;
                let Value::Sdf(s) = &m["shape"] else {
                    return Err(e1(format!("`{name}` takes a distance shape such as `sdf_sphere(radius: 40)`, found {}", m["shape"].type_name())));
                };
                let k = self.sc_num(&m, "smooth", 0.0)?.max(0.0);
                let op = match name {
                    "add" => Op::Add,
                    "subtract" => Op::Subtract,
                    _ => Op::Intersect,
                };
                let (lo, hi) = (self.sculpts[id].lo, self.sculpts[id].hi);
                sdf::apply(&mut self.sculpts[id].vol, &s.0, op, k, lo, hi);
                self.sculpt_clip(id);
            }
            "brush" => {
                let m = self.bind(
                    name,
                    args,
                    &[
                        "kind",
                        "along",
                        "at",
                        "radius",
                        "strength",
                        "falloff",
                        "invert",
                        "spacing",
                        "mirror",
                        "mirror_at",
                        "passes",
                        "part",
                    ],
                    &["kind"],
                )?;
                let Value::Str(k) = &m["kind"] else {
                    return Err(e2("`brush` needs the kind of brush as text"));
                };
                let kind = BrushKind::parse(k).ok_or_else(|| {
                    e2(format!("there is no brush `{k}`"))
                        .help(format!("brushes: {}", BrushKind::NAMES.join(", ")))
                })?;
                let radius = self.sc_num(&m, "radius", 20.0)?;
                if radius <= 0.0 {
                    return Err(e2("a brush `radius` must be positive"));
                }
                let mut b = Brush::new(kind, radius, self.sc_num(&m, "strength", 0.5)?);
                if let Some(Value::Str(f)) = m.get("falloff") {
                    b.falloff = Falloff::parse(f).ok_or_else(|| {
                        e2(format!("unknown falloff `{f}`")).help("smooth, linear, sharp, constant")
                    })?;
                }
                b.invert = matches!(m.get("invert"), Some(Value::Bool(true)));
                b.spacing = self.sc_num(&m, "spacing", 0.25)?.clamp(0.02, 2.0);
                b.passes = self.sc_num(&m, "passes", 1.0)?.clamp(1.0, 64.0) as u32;
                if let Some(Value::Str(s)) = m.get("mirror") {
                    for c in s.chars() {
                        match c {
                            'x' => b.mirror[0] = true,
                            'y' => b.mirror[1] = true,
                            'z' => b.mirror[2] = true,
                            _ => {
                                return Err(e2("`mirror` is text of axes, such as \"x\" or \"xz\""))
                            }
                        }
                    }
                }
                if let Some(v) = m.get("mirror_at") {
                    b.mirror_at = self.sc_triple(v, "`mirror_at`")?;
                } else {
                    let (lo, hi) = (self.sculpts[id].lo, self.sculpts[id].hi);
                    b.mirror_at = [
                        (lo[0] + hi[0]) / 2.0,
                        (lo[1] + hi[1]) / 2.0,
                        (lo[2] + hi[2]) / 2.0,
                    ];
                }
                let mut path: Vec<sc::V3> = vec![];
                match (m.get("along"), m.get("at")) {
                    (Some(Value::List(items)), None) => {
                        for p in items {
                            path.push(self.sc_triple(p, "a point of `along`")?);
                        }
                    }
                    (None, Some(p)) => path.push(self.sc_triple(p, "`at`")?),
                    _ => {
                        return Err(e1(
                            "a brush stroke needs `along: [(x, y, z), ...]` or `at: (x, y, z)`",
                        ))
                    }
                }
                if path.is_empty() {
                    return Err(e1("`along` has no points"));
                }
                if let Some(Value::Tuple(t)) = m.get("part") {
                    if t.len() == 2 {
                        let (a, bb) = (self.num(&t[0], "`part`")?, self.num(&t[1], "`part`")?);
                        if !(0.0..=1.0).contains(&a) || !(0.0..=1.0).contains(&bb) || a > bb {
                            return Err(e2(format!("`part` is (from, to) with 0 <= from <= to <= 1: the share of the stroke to apply now (got {a}, {bb})")));
                        }
                        path = slice_path(&path, a, bb);
                    }
                }
                if !path.is_empty() {
                    b.stroke(&mut self.sculpts[id].vol, &path);
                }
            }
            "noise" => {
                let m = self.bind(
                    name,
                    args,
                    &["kind", "scale", "amount", "seed", "octaves", "reinit"],
                    &[],
                )?;
                let spec = self.noise_spec(&m)?;
                let amount = self.sc_num(&m, "amount", 3.0)?;
                let vol = &mut self.sculpts[id].vol;
                if let Some((lo, hi)) = vol.active_box() {
                    let band = vol.band;
                    vol.edit(lo, hi, &mut |p, d| {
                        if d.abs() < band * 0.999 {
                            d + amount * spec.eval(p)
                        } else {
                            d
                        }
                    });
                    if !matches!(m.get("reinit"), Some(Value::Bool(false))) {
                        reinitialize(vol, 6);
                    }
                }
            }
            "vortex" => {
                let m = self.bind(
                    name,
                    args,
                    &["at", "axis", "speed", "radius", "time", "steps"],
                    &["at"],
                )?;
                let axis = match m.get("axis") {
                    Some(v) => self.sc_triple(v, "`axis`")?,
                    None => [0.0, 1.0, 0.0],
                };
                if sc::len(axis) < 1e-12 {
                    return Err(e1("a vortex `axis` cannot be (0, 0, 0)"));
                }
                let radius = self.sc_num(&m, "radius", 60.0)?;
                if radius <= 0.0 {
                    return Err(e2("a vortex `radius` must be positive"));
                }
                let vx = Vortex {
                    center: self.sc_triple(&m["at"], "`at`")?,
                    axis: sc::norm(axis),
                    omega: self.sc_num(&m, "speed", 1.0)?,
                    sigma: radius,
                };
                let (time, steps) = (
                    self.sc_num(&m, "time", 1.0)?,
                    self.sc_num(&m, "steps", 6.0)?.clamp(1.0, 64.0) as u32,
                );
                let vol = &mut self.sculpts[id].vol;
                advect(vol, &|p| vx.velocity(p), time, steps);
                reinitialize(vol, 4);
            }
            "flow" => {
                let m = self.bind(
                    name,
                    args,
                    &[
                        "kind", "scale", "amount", "seed", "octaves", "time", "steps",
                    ],
                    &[],
                )?;
                let spec = Arc::new(self.noise_spec(&m)?);
                let f = Field::Curl(spec, self.sc_num(&m, "amount", 30.0)?);
                let (time, steps) = (
                    self.sc_num(&m, "time", 1.0)?,
                    self.sc_num(&m, "steps", 6.0)?.clamp(1.0, 64.0) as u32,
                );
                let vol = &mut self.sculpts[id].vol;
                advect(vol, &|p| f.velocity(p), time, steps);
                reinitialize(vol, 4);
            }
            "wind" => {
                let m = self.bind(name, args, &["by", "time", "steps"], &["by"])?;
                let v = self.sc_triple(&m["by"], "`by`")?;
                let (time, steps) = (
                    self.sc_num(&m, "time", 1.0)?,
                    self.sc_num(&m, "steps", 4.0)?.clamp(1.0, 64.0) as u32,
                );
                advect(&mut self.sculpts[id].vol, &|_| v, time, steps);
            }
            "reinit" => {
                let m = self.bind(name, args, &["iterations"], &[])?;
                let it = self.sc_num(&m, "iterations", 8.0)?.clamp(1.0, 100.0) as u32;
                reinitialize(&mut self.sculpts[id].vol, it);
            }
            "clear" => {
                self.bind(name, args, &[], &[])?;
                let (h, band) = (
                    self.sculpts[id].vol.h,
                    self.sculpts[id].vol.band / self.sculpts[id].vol.h,
                );
                self.sculpts[id].vol = sc::Volume::new(h, band);
            }
            _ => {
                self.bind(name, args, &[], &[])?;
                return Ok(Value::Num(self.sculpts[id].vol.volume()));
            }
        }
        self.sculpt_refresh(sn, id);
        Ok(Value::None)
    }

    /// Keep material inside the sculpt's box: anything the box does not contain is cut off.
    fn sculpt_clip(&mut self, id: usize) {
        let (lo, hi) = (self.sculpts[id].lo, self.sculpts[id].hi);
        let boxs = Sdf::Box(
            [
                (hi[0] - lo[0]) / 2.0,
                (hi[1] - lo[1]) / 2.0,
                (hi[2] - lo[2]) / 2.0,
            ],
            0.0,
        );
        let c = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let clip = Sdf::Translate(Box::new(boxs), c);
        sdf::apply(&mut self.sculpts[id].vol, &clip, Op::Intersect, 0.0, lo, hi);
    }
}

/// The part of a polyline between the fractions `a` and `b` of its length.
fn slice_path(path: &[sc::V3], a: f64, b: f64) -> Vec<sc::V3> {
    if path.len() < 2 {
        return if b > a { path.to_vec() } else { vec![] };
    }
    let total: f64 = path.windows(2).map(|w| sc::len(sc::sub(w[1], w[0]))).sum();
    if total <= 0.0 || b <= a {
        return vec![];
    }
    let (da, db) = (a * total, b * total);
    let mut out = vec![];
    let mut run = 0.0;
    for w in path.windows(2) {
        let seg = sc::len(sc::sub(w[1], w[0]));
        let (s0, s1) = (run, run + seg);
        run = s1;
        if s1 < da || s0 > db || seg <= 0.0 {
            continue;
        }
        let lerp = |t: f64| sc::add(w[0], sc::mul(sc::sub(w[1], w[0]), t));
        let t0 = ((da - s0) / seg).clamp(0.0, 1.0);
        let t1 = ((db - s0) / seg).clamp(0.0, 1.0);
        if out.is_empty() {
            out.push(lerp(t0));
        }
        out.push(lerp(t1));
    }
    out
}
