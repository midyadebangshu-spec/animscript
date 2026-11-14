//! 3D in the language: meshes are shapes (`shape box = cube(size: 100)`), animated through the same properties as any
//! shape plus `position.z`, `rotation.x`, `rotation.y` and `scale.z`; `camera3` is a hidden shape that moves and turns
//! the 3D camera; `stage3(...)` sets the lens and the light. The renderer draws all meshes with a z-buffer.
//!
//! Axes are the canvas's (x right, y down) plus z away from the viewer, and the default camera shows the `z = 0` plane
//! at canvas scale, so a mesh placed at `(400, 300, 0)` appears at `(400, 300)` next to 2D shapes.
use crate::interp::Interp;
use crate::value::*;
use animscript_core::mesh::{Mesh, Transform3};
use animscript_core::*;
use serde_json::{json, Value as J};
use std::collections::HashMap;

type Args = Vec<(Option<String>, Value)>;

fn msh(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-MSH-001", msg)
}

/// The lens and the light shared by every mesh.
#[derive(Clone, Debug)]
pub struct Stage3 {
    /// Vertical field of view in degrees.
    pub fov: f64,
    /// The direction the light travels (unit length).
    pub light: [f64; 3],
    pub ambient: f64,
    /// Shading levels (0 or 1: smooth lighting without bands).
    pub bands: u32,
    /// Edges between faces turned by more than this many degrees are inked (silhouettes always are).
    pub crease: f64,
    /// Shading is smooth across edges between faces turned by less than this many degrees (0: flat facets).
    pub smooth: f64,
    /// The sky mirrors and water reflect where nothing else is: (top, horizon). Default: from the background.
    pub sky: Option<([u8; 4], [u8; 4])>,
    /// Photographic lighting: multiplies the light before the tone-map (1 = as lit).
    pub exposure: f64,
    /// Shadow quality: "low", "medium" or "high" (the number of rays a soft shadow takes).
    pub quality: String,
    /// Ambient occlusion distance in px (0 = off).
    pub ao: f64,
    /// Bloom strength (0 = off).
    pub bloom: f64,
}

impl Default for Stage3 {
    fn default() -> Stage3 {
        let l = [-0.4f64, 0.7, 0.6];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        Stage3 {
            fov: 40.0,
            light: [l[0] / n, l[1] / n, l[2] / n],
            ambient: 0.35,
            bands: 3,
            crease: 40.0,
            smooth: 50.0,
            sky: None,
            exposure: 1.0,
            quality: "medium".into(),
            ao: 60.0,
            bloom: 0.0,
        }
    }
}

impl Interp {
    pub fn ensure_camera3(&mut self) {
        if !self.shapes.contains_key("camera3") {
            let mut sh = Shape::new("camera3");
            sh.visible = false;
            self.shapes.insert("camera3".to_string(), sh);
        }
    }

    fn positive(
        &self,
        m: &HashMap<&'static str, Value>,
        key: &'static str,
        default: f64,
    ) -> Res<f64> {
        let v = match m.get(key) {
            Some(v) => self.num(v, &format!("`{key}`"))?,
            None => default,
        };
        if v <= 0.0 || !v.is_finite() {
            return Err(msh(format!("`{key}` must be a positive number")));
        }
        Ok(v)
    }

    fn count(
        &self,
        m: &HashMap<&'static str, Value>,
        key: &'static str,
        default: i64,
        min: i64,
    ) -> Res<usize> {
        let v = match m.get(key) {
            Some(v) => self.int(v, &format!("`{key}`"))?,
            None => default,
        };
        if v < min || v > 2000 {
            return Err(msh(format!(
                "`{key}` must be a whole number from {min} to 2000"
            )));
        }
        Ok(v as usize)
    }

    fn at3(&self, m: &HashMap<&'static str, Value>) -> Res<Option<[f64; 3]>> {
        self.point3(m, "at")
    }

    fn pivot3(&self, m: &HashMap<&'static str, Value>) -> Res<Option<[f64; 3]>> {
        self.point3(m, "pivot")
    }

    fn point3(&self, m: &HashMap<&'static str, Value>, key: &'static str) -> Res<Option<[f64; 3]>> {
        let Some(v) = m.get(key) else {
            return Ok(None);
        };
        match v {
            Value::Tuple(t) if t.len() == 2 || t.len() == 3 => {
                let mut p = [0.0; 3];
                for (k, x) in t.iter().enumerate() {
                    p[k] = self.num(x, &format!("`{key}`"))?;
                }
                Ok(Some(p))
            }
            o => Err(msh(format!(
                "`{key}` must be (x, y) or (x, y, z), found {}",
                o.type_name()
            ))),
        }
    }

    /// A mesh as a new shape: centred on the middle of its bounding box (its pivot), which goes to `at`.
    fn mesh_shape(&self, mesh: Mesh, at: Option<[f64; 3]>, turn_about: Option<[f64; 3]>) -> Value {
        let (lo, hi) = mesh.bbox();
        let pivot = [
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ];
        let mut sh = Shape::new("mesh");
        sh.pivot3 = pivot;
        if let Some(a) = at {
            sh.position = [a[0] - pivot[0], a[1] - pivot[1]];
            sh.pos_z = a[2] - pivot[2];
        }
        // `pivot` is a point in the scene: the mesh turns about it
        if let Some(p) = turn_about {
            sh.pivot3 = [
                p[0] - sh.position[0],
                p[1] - sh.position[1],
                p[2] - sh.pos_z,
            ];
        }
        sh.mesh = Some(mesh);
        Value::Fresh(Box::new(sh))
    }

    pub(crate) fn mesh_builtin(&mut self, name: &str, args: Args) -> Res<Value> {
        match name {
            "cube" => {
                let m = self.bind(
                    name,
                    args,
                    &[
                        "size",
                        "width",
                        "height",
                        "depth",
                        "at",
                        "pivot",
                        "subdivide",
                    ],
                    &[],
                )?;
                let size = self.positive(&m, "size", 100.0)?;
                let (w, h, d) = (
                    self.positive(&m, "width", size)?,
                    self.positive(&m, "height", size)?,
                    self.positive(&m, "depth", size)?,
                );
                let cells = self.count(&m, "subdivide", 1, 1)?;
                if cells > 48 {
                    return Err(Diagnostic::new(
                        "E-MSH-001",
                        "`subdivide` can be at most 48",
                    ));
                }
                let mesh = Mesh::cube(w, h, d).subdivided(cells);
                Ok(self.mesh_shape(mesh, self.at3(&m)?, self.pivot3(&m)?))
            }
            "sphere" => {
                let m = self.bind(
                    name,
                    args,
                    &["radius", "segments", "rings", "at", "pivot"],
                    &[],
                )?;
                let r = self.positive(&m, "radius", 50.0)?;
                let (s, g) = (
                    self.count(&m, "segments", 24, 3)?,
                    self.count(&m, "rings", 12, 2)?,
                );
                Ok(self.mesh_shape(Mesh::sphere(r, s, g), self.at3(&m)?, self.pivot3(&m)?))
            }
            "cylinder" | "cone" => {
                let m = self.bind(
                    name,
                    args,
                    &["radius", "height", "segments", "at", "pivot"],
                    &[],
                )?;
                let (r, h) = (
                    self.positive(&m, "radius", 40.0)?,
                    self.positive(&m, "height", 100.0)?,
                );
                let s = self.count(&m, "segments", 32, 3)?;
                let mesh = if name == "cylinder" {
                    Mesh::cylinder(r, h, s)
                } else {
                    Mesh::cone(r, h, s)
                };
                Ok(self.mesh_shape(mesh, self.at3(&m)?, self.pivot3(&m)?))
            }
            "plane" => {
                let m = self.bind(
                    name,
                    args,
                    &["width", "depth", "nx", "nz", "at", "pivot"],
                    &[],
                )?;
                let (w, d) = (
                    self.positive(&m, "width", 400.0)?,
                    self.positive(&m, "depth", 400.0)?,
                );
                let (nx, nz) = (self.count(&m, "nx", 1, 1)?, self.count(&m, "nz", 1, 1)?);
                Ok(self.mesh_shape(Mesh::plane(w, d, nx, nz), self.at3(&m)?, self.pivot3(&m)?))
            }
            "lathe" => {
                let m = self.bind(
                    name,
                    args,
                    &["profile", "segments", "at", "pivot", "double_sided"],
                    &["profile"],
                )?;
                let Value::List(items) = &m["profile"] else {
                    return Err(msh("`profile` must be a list of (radius, height) points"));
                };
                let pts: Vec<[f64; 2]> = items
                    .iter()
                    .map(|p| self.point(p, "a profile point"))
                    .collect::<Res<_>>()?;
                let s = self.count(&m, "segments", 32, 3)?;
                let mut mesh = Mesh::lathe(&pts, s).map_err(msh)?;
                // an open strip (a band, a ring, a bowl) can be seen from both sides
                match m.get("double_sided") {
                    None => {}
                    Some(Value::Bool(b)) => mesh.double_sided = *b,
                    Some(other) => {
                        return Err(msh(format!(
                            "`double_sided` must be true or false, found {}",
                            other.type_name()
                        )))
                    }
                }
                Ok(self.mesh_shape(mesh, self.at3(&m)?, self.pivot3(&m)?))
            }
            "extrude" => {
                let m = self.bind(name, args, &["of", "depth", "points", "at", "pivot"], &[])?;
                let poly: Vec<[f64; 2]> = match (m.get("of"), m.get("points")) {
                    (Some(shape), None) => self.outline_points(shape, true)?,
                    (None, Some(Value::List(items))) => items
                        .iter()
                        .map(|p| self.point(p, "a polygon point"))
                        .collect::<Res<_>>()?,
                    (None, Some(o)) => {
                        return Err(msh(format!(
                            "`points` must be a list of points, found {}",
                            o.type_name()
                        )))
                    }
                    (None, None) => return Err(Diagnostic::new(
                        "E-TYP-003",
                        "`extrude` needs a shape (`extrude(star, depth: 30)`) or `points: [...]`",
                    )),
                    (Some(_), Some(_)) => {
                        return Err(msh("give `extrude` a shape or `points`, not both"))
                    }
                };
                let depth = self.positive(&m, "depth", 40.0)?;
                // a closed outline repeats its first point
                let poly = match (poly.first(), poly.last()) {
                    (Some(a), Some(b))
                        if poly.len() > 3
                            && (a[0] - b[0]).abs() < 1e-9
                            && (a[1] - b[1]).abs() < 1e-9 =>
                    {
                        poly[..poly.len() - 1].to_vec()
                    }
                    _ => poly,
                };
                let mesh = Mesh::extrude(&poly, depth).map_err(msh)?;
                Ok(self.mesh_shape(mesh, self.at3(&m)?, self.pivot3(&m)?))
            }
            "focal3" => {
                self.bind(name, args, &[], &[])?;
                Ok(Value::Num(
                    (self.canvas.1 as f64 / 2.0) / (self.stage3.fov.to_radians() / 2.0).tan(),
                ))
            }
            "stage3" => {
                let m = self.bind(
                    name,
                    args,
                    &[
                        "fov", "light", "ambient", "bands", "crease", "smooth", "sky", "exposure",
                        "quality", "ao", "bloom",
                    ],
                    &[],
                )?;
                let mut st = self.stage3.clone();
                if let Some(v) = m.get("fov") {
                    st.fov = self.num(v, "`fov`")?;
                    if !(st.fov > 0.0 && st.fov < 170.0) {
                        return Err(msh("`fov` (the vertical field of view in degrees) must be between 0 and 170"));
                    }
                }
                if let Some(v) = m.get("light") {
                    let Value::Tuple(t) = v else {
                        return Err(msh("`light` must be a direction (x, y, z)"));
                    };
                    if t.len() != 3 {
                        return Err(msh("`light` must be a direction (x, y, z)"));
                    }
                    let d = [
                        self.num(&t[0], "`light`")?,
                        self.num(&t[1], "`light`")?,
                        self.num(&t[2], "`light`")?,
                    ];
                    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    if l == 0.0 || !l.is_finite() {
                        return Err(msh("`light` needs a direction, not (0, 0, 0)"));
                    }
                    st.light = [d[0] / l, d[1] / l, d[2] / l];
                }
                if let Some(v) = m.get("ambient") {
                    st.ambient = self.num(v, "`ambient`")?;
                    if !(0.0..=1.0).contains(&st.ambient) {
                        return Err(msh("`ambient` is between 0 and 1"));
                    }
                }
                if let Some(v) = m.get("ao") {
                    st.ao = self.num(v, "`ao`")?;
                    if st.ao.is_nan() || st.ao < 0.0 {
                        return Err(msh("`ao` is the occlusion distance in px (0 turns it off)"));
                    }
                }
                if let Some(v) = m.get("bloom") {
                    st.bloom = self.num(v, "`bloom`")?;
                    if !(0.0..=4.0).contains(&st.bloom) {
                        return Err(msh("`bloom` is a strength between 0 (off) and 4"));
                    }
                }
                if let Some(v) = m.get("quality") {
                    match v {
                        Value::Str(q) if matches!(q.as_str(), "low" | "medium" | "high") => {
                            st.quality = q.clone()
                        }
                        _ => return Err(msh("`quality` is \"low\", \"medium\" or \"high\"")),
                    }
                }
                if let Some(v) = m.get("exposure") {
                    st.exposure = self.num(v, "`exposure`")?;
                    if !(st.exposure > 0.0 && st.exposure <= 100.0) {
                        return Err(msh("`exposure` is above 0 (1 leaves the light as it is)"));
                    }
                }
                if let Some(v) = m.get("bands") {
                    let b = self.int(v, "`bands`")?;
                    if !(0..=16).contains(&b) {
                        return Err(msh("`bands` is 0 to 16 (0 or 1: smooth lighting)"));
                    }
                    st.bands = b as u32;
                }
                if let Some(v) = m.get("crease") {
                    let Value::Angle(a) = v else {
                        return Err(msh("`crease` must be an angle, like `40.deg`"));
                    };
                    st.crease = a * 360.0;
                    if !(0.0..=180.0).contains(&st.crease) {
                        return Err(msh("`crease` is between 0 and 180 degrees"));
                    }
                }
                if let Some(v) = m.get("smooth") {
                    let Value::Angle(a) = v else {
                        return Err(msh(
                            "`smooth` must be an angle, like `50.deg` (0.deg: flat facets)",
                        ));
                    };
                    st.smooth = a * 360.0;
                    if !(0.0..=180.0).contains(&st.smooth) {
                        return Err(msh("`smooth` is between 0 and 180 degrees"));
                    }
                }
                if let Some(v) = m.get("sky") {
                    let (Value::Tuple(t), true) = (v, matches!(v, Value::Tuple(t) if t.len() == 2))
                    else {
                        return Err(msh(
                            "`sky` must be two colours (top, horizon), like `(#8fc7e6, #d9ecee)`",
                        ));
                    };
                    match (&t[0], &t[1]) {
                        (Value::Color(a), Value::Color(b)) => st.sky = Some((*a, *b)),
                        _ => return Err(msh(
                            "`sky` must be two colours (top, horizon), like `(#8fc7e6, #d9ecee)`",
                        )),
                    }
                }
                self.stage3 = st;
                Ok(Value::None)
            }
            _ => unreachable!("dispatched by name"),
        }
    }

    pub fn is_material_method(name: &str) -> bool {
        matches!(name, "glass" | "mirror" | "matte" | "surface" | "pattern")
    }

    /// `glass(opacity:, reflect:, refract:)`, `mirror(reflect:)` and `matte()` on a mesh shape.
    pub fn material_method(&mut self, sn: &str, name: &str, args: Args) -> Res<Value> {
        if !self.is_mesh_shape(sn) {
            return Err(msh(format!(
                "`{name}` is for 3D meshes, and `{sn}` is not one"
            )));
        }
        let unit = |this: &Interp, m: &HashMap<&'static str, Value>, k: &str, d: f64| -> Res<f64> {
            let v = match m.get(k) {
                Some(v) => this.num(v, &format!("`{k}`"))?,
                None => d,
            };
            if !(0.0..=1.0).contains(&v) {
                return Err(msh(format!("`{k}` is between 0 and 1")));
            }
            Ok(v)
        };
        if name == "pattern" {
            return self.pattern_method(sn, args);
        }
        if name == "surface" {
            let m = self.bind(name, args, &["roughness", "metallic", "emissive"], &[])?;
            let mut s = Surface::default();
            for (k, slot, max) in [
                ("roughness", &mut s.roughness, 1.0),
                ("metallic", &mut s.metallic, 1.0),
                ("emissive", &mut s.emissive, 50.0),
            ] {
                if let Some(v) = m.get(k) {
                    let x = self.num(v, &format!("`{k}`"))?;
                    if !(0.0..=max).contains(&x) {
                        return Err(Diagnostic::new(
                            "E-LIT-002",
                            format!("`{k}` is between 0 and {max}"),
                        ));
                    }
                    *slot = x;
                }
            }
            self.shapes.get_mut(sn).expect("shape").surface = Some(s);
            return Ok(Value::None);
        }
        let material = match name {
            "glass" => {
                let m = self.bind(name, args, &["opacity", "reflect", "refract"], &[])?;
                Some(MeshMaterial {
                    kind: "glass".into(),
                    opacity: unit(self, &m, "opacity", 0.2)?,
                    reflect: unit(self, &m, "reflect", 0.25)?,
                    refract: unit(self, &m, "refract", 1.0)?,
                })
            }
            "mirror" => {
                let m = self.bind(name, args, &["reflect"], &[])?;
                Some(MeshMaterial {
                    kind: "mirror".into(),
                    opacity: 1.0,
                    reflect: unit(self, &m, "reflect", 0.5)?,
                    refract: 0.0,
                })
            }
            _ => {
                self.bind(name, args, &[], &[])?;
                None
            }
        };
        self.shapes.get_mut(sn).expect("shape").material = material;
        Ok(Value::None)
    }

    /// `pattern(mask:, hole:, color:, glow:)`: equations of the surface coordinates `u` and `v` (each 0 to 1).
    fn pattern_method(&mut self, sn: &str, args: Args) -> Res<Value> {
        let (m, consts) = self.split_math_args(
            "pattern",
            args,
            &["mask", "hole", "color", "glow"],
            &["u", "v"],
        )?;
        let has_uv = self.shapes[sn]
            .mesh
            .as_ref()
            .is_some_and(|me| !me.uv.is_empty() && me.uv.len() == me.faces.len());
        if !has_uv {
            return Err(msh(format!("`{sn}` has no surface coordinates to draw a pattern on"))
                .help("a pattern works on meshes with a surface: lathe, sphere, cylinder, cone, cube and plane (not extrude)"));
        }
        let mut pat = Pattern::default();
        for key in ["mask", "hole"] {
            let Some(v) = m.get(key) else { continue };
            let Value::Str(src) = v else {
                return Err(msh(format!(
                    "`{key}` is an equation in quotes, like \"fract(u * 8) < 0.5\", found {}",
                    v.type_name()
                )));
            };
            let mut b = animscript_math::Builder::new();
            let (u, vv) = (
                b.input("u", animscript_math::Rate::Point),
                b.input("v", animscript_math::Rate::Point),
            );
            let mut env = animscript_math::Env::new().with("u", u).with("v", vv);
            for (k, c) in &consts {
                let id = b.constant(*c);
                env = env.with(k, id);
            }
            animscript_math::parse(src, &mut b, &env).map_err(|e| {
                msh(format!("the `{key}` equation `{src}` is not valid: {e}"))
                    .help("it may use `u` and `v` (0 to 1 across and along the surface) and the math functions (sin, fract, abs, min, max, smoothstep, noise, ...)")
            })?;
            if key == "mask" {
                pat.mask = Some(src.clone());
            } else {
                pat.hole = Some(src.clone());
            }
        }
        if pat.mask.is_none() && pat.hole.is_none() {
            return Err(msh("`pattern` needs a `mask:` or a `hole:` equation"));
        }
        if let Some(c) = m.get("color") {
            let Value::Color(rgba) = c else {
                return Err(msh(format!("`color` is a colour, found {}", c.type_name())));
            };
            pat.color = Some(format!("#{:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2]));
        }
        if let Some(g) = m.get("glow") {
            let g = self.num(g, "`glow`")?;
            if !(0.0..=50.0).contains(&g) {
                return Err(msh("`glow` is between 0 and 50"));
            }
            pat.glow = g;
        }
        pat.consts = consts.into_iter().collect();
        self.shapes.get_mut(sn).expect("shape").pattern = Some(pat);
        Ok(Value::None)
    }

    pub fn any_visible_mesh(&self) -> bool {
        self.shape_order
            .iter()
            .any(|n| self.shapes[n].mesh.is_some() && self.shapes[n].visible)
    }

    /// The camera for the renderer: where it is and which way it looks (positions in canvas units).
    pub fn camera3_json(&self, resolved: &crate::snapshot::Resolved) -> J {
        let (w, h) = (self.canvas.0 as f64, self.canvas.1 as f64);
        let f0 = (h / 2.0) / (self.stage3.fov.to_radians() / 2.0).tan();
        let (pos, scale, roll, t3) = match self.shapes.get("camera3") {
            Some(sh) => {
                let tf = resolved
                    .tf
                    .get("camera3")
                    .copied()
                    .unwrap_or_else(|| sh.transform());
                let t3 = resolved
                    .t3
                    .get("camera3")
                    .copied()
                    .unwrap_or([sh.pos_z, sh.rot_x, sh.rot_y, sh.scale_z]);
                (tf.position, tf.scale[0], tf.rotation, t3)
            }
            None => ([0.0, 0.0], 1.0, 0.0, [0.0, 0.0, 0.0, 1.0]),
        };
        let eye = [w / 2.0 + pos[0], h / 2.0 + pos[1], -f0 + t3[0]];
        let rot = Transform3 {
            position: [0.0; 3],
            rot: [t3[1], t3[2], roll],
            scale: [1.0; 3],
            pivot: [0.0; 3],
            orient: None,
        };
        let (right, down, forward) = (
            rot.apply([1.0, 0.0, 0.0]),
            rot.apply([0.0, 1.0, 0.0]),
            rot.apply([0.0, 0.0, 1.0]),
        );
        // exact basis for the common case of an unturned camera
        let clean = |v: [f64; 3]| v.map(|x| if x.abs() < 1e-15 { 0.0 } else { x });
        json!({"eye": eye, "right": clean(right), "down": clean(down), "forward": clean(forward), "focal": f0 * scale, "cx": w / 2.0, "cy": h / 2.0})
    }

    pub fn light3_json(&self) -> J {
        let s = &self.stage3;
        let mut j = json!({"dir": s.light, "ambient": s.ambient, "bands": s.bands, "crease": s.crease, "smooth": s.smooth});
        if s.ao != 60.0 {
            j["ao"] = json!(s.ao);
        }
        if s.bloom != 0.0 {
            j["bloom"] = json!(s.bloom);
        }
        if s.quality != "medium" {
            j["quality"] = json!(s.quality);
        }
        if s.exposure != 1.0 {
            j["exposure"] = json!(s.exposure);
        }
        if let Some((top, horizon)) = s.sky {
            j["sky"] = json!([crate::color::hex(top), crate::color::hex(horizon)]);
        }
        j
    }
}

/// A mesh in world space for the snapshot.
#[allow(clippy::too_many_arguments)]
pub fn mesh_json(
    sh: &Shape,
    tf: &Transform,
    t3: Option<[f64; 4]>,
    orient: Option<[f64; 4]>,
    over: Option<&std::collections::BTreeMap<u32, [f64; 3]>>,
    fill: J,
    outline: J,
    width: f64,
) -> J {
    let mesh = sh.mesh.as_ref().expect("a mesh shape");
    let t3 = t3.unwrap_or([sh.pos_z, sh.rot_x, sh.rot_y, sh.scale_z]);
    let t = Transform3 {
        position: [tf.position[0], tf.position[1], t3[0]],
        rot: [t3[1], t3[2], tf.rotation],
        scale: [tf.scale[0], tf.scale[1], t3[3]],
        pivot: sh.pivot3,
        orient: orient.or(sh.orient),
    };
    let verts: Vec<J> = mesh
        .verts
        .iter()
        .enumerate()
        .map(|(i, v)| json!(t.apply(over.and_then(|o| o.get(&(i as u32))).copied().unwrap_or(*v))))
        .collect();
    let mut j = json!({"verts": verts, "faces": mesh.faces, "double_sided": mesh.double_sided, "fill": fill, "outline": outline, "width": width});
    if let Some(p) = &sh.pattern {
        j["pattern"] = json!({"mask": p.mask, "hole": p.hole, "color": p.color, "glow": p.glow, "consts": p.consts.iter().cloned().collect::<std::collections::BTreeMap<_, _>>()});
        j["uv"] = json!(mesh.uv);
    }
    if let Some(s) = &sh.surface {
        j["surface"] = json!([s.roughness, s.metallic, s.emissive]);
    }
    if let Some(m) = &sh.material {
        j["material"] = json!({"kind": m.kind, "reflect": m.reflect, "refract": m.refract, "opacity": m.opacity});
    }
    j
}
