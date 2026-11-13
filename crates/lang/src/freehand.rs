//! Freehand drawing helpers: fitting hand-drawn points with Bezier curves, and working with point lists
//! (`freehand`, `smooth`, `simplify`, `resample`, `path_length`, `points_along`, `outline`, `jitter`).
//! The fitting itself is `animscript_core::fit_curve` (Schneider's algorithm with corner detection).
use crate::interp::Interp;
use crate::procedural::{stream_seed, value_noise};
use crate::value::*;
use animscript_core::*;

type Args = Vec<(Option<String>, Value)>;

fn fre_err(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-FRE-001", msg)
}

fn point_list(pts: &[P]) -> Value {
    Value::List(
        pts.iter()
            .map(|p| Value::Tuple(vec![Value::Num(p[0]), Value::Num(p[1])]))
            .collect(),
    )
}

/// A polyline through a sampled Bezier or straight edge, as dense points (more than any spacing asked for).
fn edge_polyline(ctrl: &[P]) -> Vec<P> {
    let n = if ctrl.len() == 2 { 1 } else { 96 };
    (0..=n)
        .map(|k| {
            let t = k as f64 / n as f64;
            let u = 1.0 - t;
            match ctrl.len() {
                2 => [
                    ctrl[0][0] * u + ctrl[1][0] * t,
                    ctrl[0][1] * u + ctrl[1][1] * t,
                ],
                3 => [
                    u * u * ctrl[0][0] + 2.0 * u * t * ctrl[1][0] + t * t * ctrl[2][0],
                    u * u * ctrl[0][1] + 2.0 * u * t * ctrl[1][1] + t * t * ctrl[2][1],
                ],
                _ => [
                    u * u * u * ctrl[0][0]
                        + 3.0 * u * u * t * ctrl[1][0]
                        + 3.0 * u * t * t * ctrl[2][0]
                        + t * t * t * ctrl[3][0],
                    u * u * u * ctrl[0][1]
                        + 3.0 * u * u * t * ctrl[1][1]
                        + 3.0 * u * t * t * ctrl[2][1]
                        + t * t * t * ctrl[3][1],
                ],
            }
        })
        .collect()
}

impl Interp {
    /// The numbers of a list of points; each is a tuple with at least x and y.
    fn points_of(&self, v: &Value, what: &str) -> Res<Vec<P>> {
        let Value::List(items) = v else {
            return Err(fre_err(format!(
                "{what} must be a list of points, found {}",
                v.type_name()
            )));
        };
        items
            .iter()
            .map(|p| match p {
                Value::Tuple(t) if t.len() >= 2 => {
                    Ok([self.num(&t[0], "a point")?, self.num(&t[1], "a point")?])
                }
                other => Err(fre_err(format!(
                    "a point is a tuple `(x, y)`, found {}",
                    other.type_name()
                ))),
            })
            .collect()
    }

    pub(crate) fn is_freehand_fn(name: &str) -> bool {
        matches!(
            name,
            "freehand"
                | "smooth"
                | "simplify"
                | "resample"
                | "path_length"
                | "points_along"
                | "outline"
                | "jitter"
        )
    }

    pub(crate) fn freehand_fn(&mut self, name: &str, args: Args) -> Res<Value> {
        match name {
            "freehand" | "smooth" => {
                let params: &[&'static str] = if name == "freehand" {
                    &["points", "tolerance", "corner", "closed"]
                } else {
                    &["points", "tolerance", "corner", "closed", "spacing"]
                };
                let m = self.bind(name, args, params, &["points"])?;
                let pts = self.points_of(&m["points"], "the points")?;
                let tol = m
                    .get("tolerance")
                    .map(|v| self.num(v, "`tolerance`"))
                    .transpose()?
                    .unwrap_or(2.0);
                let corner = m
                    .get("corner")
                    .map(|v| self.angle(v, "`corner`"))
                    .transpose()?
                    .unwrap_or(40.0 / 360.0)
                    * 360.0;
                let closed = match m.get("closed") {
                    None => false,
                    Some(Value::Bool(b)) => *b,
                    Some(o) => {
                        return Err(fre_err(format!(
                            "`closed` is true or false, found {}",
                            o.type_name()
                        )))
                    }
                };
                if tol <= 0.0 {
                    return Err(fre_err("`tolerance` must be positive (the largest distance a point may be from the curve)"));
                }
                let segs = fit_curve(&pts, tol, corner, closed);
                if segs.is_empty() {
                    return Err(fre_err(
                        "there is nothing to fit: give at least two different points",
                    )
                    .help("a freehand path needs two or more points that are not all the same"));
                }
                if name == "freehand" {
                    let closed_path =
                        closed && pts.len() >= 3 && segs.last().is_some_and(|s| s.p3 == segs[0].p0);
                    return Ok(Value::Fresh(Box::new(bezier_shape(&segs, closed_path))));
                }
                // `smooth`: the fitted curve as a list of points `spacing` apart
                let spacing = m
                    .get("spacing")
                    .map(|v| self.num(v, "`spacing`"))
                    .transpose()?
                    .unwrap_or(4.0);
                if spacing <= 0.0 {
                    return Err(fre_err("`spacing` must be positive"));
                }
                let mut dense: Vec<P> = vec![];
                for s in &segs {
                    for k in 0..=64 {
                        let p = s.at(k as f64 / 64.0);
                        if dense.last().is_none_or(|q| {
                            (q[0] - p[0]).abs() > 1e-9 || (q[1] - p[1]).abs() > 1e-9
                        }) {
                            dense.push(p);
                        }
                    }
                }
                Ok(point_list(&resample(&dense, spacing)))
            }
            "simplify" | "resample" => {
                let second = if name == "simplify" {
                    "tolerance"
                } else {
                    "spacing"
                };
                let m = self.bind(name, args, &["points", second], &["points", second])?;
                let pts = self.points_of(&m["points"], "the points")?;
                let k = self.num(&m[second], "the second argument")?;
                if k <= 0.0 {
                    return Err(fre_err(format!("`{second}` must be positive")));
                }
                Ok(point_list(&if name == "simplify" {
                    simplify(&pts, k)
                } else {
                    resample(&pts, k)
                }))
            }
            "path_length" => {
                let m = self.bind(name, args, &["points"], &["points"])?;
                Ok(Value::Num(path_length(
                    &self.points_of(&m["points"], "the points")?,
                )))
            }
            "points_along" | "outline" => {
                let m = self.bind(name, args, &["of", "spacing"], &["of"])?;
                let spacing = m
                    .get("spacing")
                    .map(|v| self.num(v, "`spacing`"))
                    .transpose()?
                    .unwrap_or(4.0);
                if spacing <= 0.0 {
                    return Err(fre_err("`spacing` must be positive"));
                }
                let line = self.outline_points(&m["of"], name == "outline")?;
                Ok(point_list(&resample(&line, spacing)))
            }
            "jitter" => {
                let m = self.bind(
                    name,
                    args,
                    &["points", "amount", "scale", "seed"],
                    &["points"],
                )?;
                let pts = self.points_of(&m["points"], "the points")?;
                let amount = m
                    .get("amount")
                    .map(|v| self.num(v, "`amount`"))
                    .transpose()?
                    .unwrap_or(1.5);
                let scale = m
                    .get("scale")
                    .map(|v| self.num(v, "`scale`"))
                    .transpose()?
                    .unwrap_or(30.0);
                if scale <= 0.0 {
                    return Err(fre_err("`scale` must be positive (the length over which the wobble changes, in pixels)"));
                }
                let seed = match m.get("seed") {
                    Some(Value::Str(s)) => s.clone(),
                    None => "jitter".to_string(),
                    Some(o) => {
                        return Err(fre_err(format!(
                            "`seed` names the random stream, found {}",
                            o.type_name()
                        )))
                    }
                };
                let (sx, sy) = (
                    stream_seed(self.seed, &format!("{seed}.x")),
                    stream_seed(self.seed, &format!("{seed}.y")),
                );
                let mut run = 0.0;
                let out: Vec<P> = pts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if i > 0 {
                            run += ((p[0] - pts[i - 1][0]).powi(2)
                                + (p[1] - pts[i - 1][1]).powi(2))
                            .sqrt();
                        }
                        let t = run / scale;
                        [
                            p[0] + amount * value_noise(sx, t),
                            p[1] + amount * value_noise(sy, t),
                        ]
                    })
                    .collect();
                Ok(point_list(&out))
            }
            _ => unreachable!("is_freehand_fn"),
        }
    }

    /// A dense polyline along an edge, or along the outline of a shape (its first face's boundary, else its edges
    /// chained in order), in world coordinates with the shape's current transform.
    pub(crate) fn outline_points(&self, v: &Value, whole_shape: bool) -> Res<Vec<P>> {
        let (sn, edges): (&String, Vec<(u32, i8)>) = match v {
            Value::Elem(sn, r) if r.kind == Kind::E && !whole_shape => (sn, vec![(r.id, 1)]),
            Value::Shape(sn) if whole_shape => {
                let sh = &self.shapes[sn];
                let edges = match sh.live_ids(Kind::F).first() {
                    Some(f) => sh.faces[f].loop_.clone(),
                    None => sh.live_ids(Kind::E).into_iter().map(|e| (e, 1i8)).collect(),
                };
                (sn, edges)
            }
            other => {
                return Err(fre_err(format!(
                    "{} needs {}, found {}",
                    if whole_shape {
                        "`outline`"
                    } else {
                        "`points_along`"
                    },
                    if whole_shape {
                        "a shape"
                    } else {
                        "an edge such as `s.e1`"
                    },
                    other.type_name()
                )))
            }
        };
        let sh = &self.shapes[sn];
        let t = self
            .live_tf
            .get(sn)
            .copied()
            .unwrap_or_else(|| sh.transform());
        let mut out: Vec<P> = vec![];
        for (eid, dir) in edges {
            let mut line = edge_polyline(&sh.edge_points_with(eid, None));
            if dir < 0 {
                line.reverse();
            }
            for p in line {
                let p = t.apply(p);
                if out
                    .last()
                    .is_none_or(|q| (q[0] - p[0]).abs() > 1e-9 || (q[1] - p[1]).abs() > 1e-9)
                {
                    out.push(p);
                }
            }
        }
        if out.len() < 2 {
            return Err(fre_err("there is no path here: the shape has no edges"));
        }
        Ok(out)
    }
}
