//! Runtime values, exact rationals for durations, and easing functions.
use crate::ast::Expr;
use crate::interp::Vars;
use animscript_core::{ElemRef, ImageFill, Kind, Shape};
use std::rc::Rc;

/// An exact rational number (durations are exact; spec §3.1, §5.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rat {
    pub n: i128,
    pub d: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[allow(clippy::should_implement_trait)]
impl Rat {
    pub fn new(n: i128, d: i128) -> Rat {
        let g = gcd(n, d).max(1);
        let s = if d < 0 { -1 } else { 1 };
        Rat {
            n: s * n / g,
            d: s * d / g,
        }
    }
    pub fn int(n: i64) -> Rat {
        Rat { n: n as i128, d: 1 }
    }
    /// Parse a decimal such as `0.25` or `-3` exactly. Exponent forms are not accepted.
    pub fn parse_decimal(s: &str) -> Option<Rat> {
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        if frac.len() > 30
            || !int.chars().chain(frac.chars()).all(|c| c.is_ascii_digit())
            || (int.is_empty() && frac.is_empty())
        {
            return None;
        }
        let n: i128 = format!("{int}{frac}").parse().ok()?;
        let d = 10i128.checked_pow(frac.len() as u32)?;
        Some(Rat::new(if neg { -n } else { n }, d))
    }
    /// Exact decimal reading of an f64's shortest representation (so `0.6` means 3/5, not a binary fraction).
    pub fn from_f64(x: f64) -> Option<Rat> {
        if !x.is_finite() {
            return None;
        }
        Rat::parse_decimal(&format!("{x}"))
    }
    pub fn to_f64(self) -> f64 {
        self.n as f64 / self.d as f64
    }
    pub fn add(self, o: Rat) -> Rat {
        Rat::new(self.n * o.d + o.n * self.d, self.d * o.d)
    }
    pub fn sub(self, o: Rat) -> Rat {
        Rat::new(self.n * o.d - o.n * self.d, self.d * o.d)
    }
    pub fn mul(self, o: Rat) -> Rat {
        Rat::new(self.n * o.n, self.d * o.d)
    }
    pub fn div(self, o: Rat) -> Option<Rat> {
        if o.n == 0 {
            None
        } else {
            Some(Rat::new(self.n * o.d, self.d * o.n))
        }
    }
    pub fn cmp(self, o: Rat) -> std::cmp::Ordering {
        (self.n * o.d).cmp(&(o.n * self.d))
    }
    /// Round half to even (spec §5.3).
    pub fn round_half_even(self) -> i128 {
        let q = self.n.div_euclid(self.d);
        let r = self.n.rem_euclid(self.d);
        match (2 * r).cmp(&self.d) {
            std::cmp::Ordering::Less => q,
            std::cmp::Ordering::Greater => q + 1,
            std::cmp::Ordering::Equal => {
                if q % 2 == 0 {
                    q
                } else {
                    q + 1
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    /// A damped spring: `bounces` overshoots, decaying with `damping`, landing exactly on 1.
    Spring {
        bounces: f64,
        damping: f64,
    },
    /// CSS-style `cubic_bezier(x1, y1, x2, y2)`.
    Bezier([f64; 4]),
    Linear,
    In,
    Out,
    InOut,
    InCubic,
    OutCubic,
    InOutCubic,
    OutBack,
    Step,
}

impl Ease {
    pub fn from_name(n: &str) -> Option<Ease> {
        Some(match n {
            "linear" => Ease::Linear,
            "ease_in" => Ease::In,
            "ease_out" => Ease::Out,
            "ease_in_out" => Ease::InOut,
            "ease_in_cubic" => Ease::InCubic,
            "ease_out_cubic" => Ease::OutCubic,
            "ease_in_out_cubic" => Ease::InOutCubic,
            "ease_out_back" => Ease::OutBack,
            "step" => Ease::Step,
            _ => return None,
        })
    }
    /// Easing maps progress `p` in [0, 1] to [0, 1] with the fixed formulas of spec §7.4.
    pub fn apply(self, p: f64) -> f64 {
        match self {
            Ease::Linear => p,
            Ease::Bezier([x1, y1, x2, y2]) => {
                // find t with x(t) = p (Newton, then bisection), and return y(t)
                let bez = |a: f64, b: f64, t: f64| {
                    let u = 1.0 - t;
                    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
                };
                let dbez = |a: f64, b: f64, t: f64| {
                    let u = 1.0 - t;
                    3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
                };
                if p <= 0.0 || p >= 1.0 {
                    return p.clamp(0.0, 1.0);
                }
                let mut t = p;
                for _ in 0..8 {
                    let err = bez(x1, x2, t) - p;
                    if err.abs() < 1e-12 {
                        break;
                    }
                    let d = dbez(x1, x2, t);
                    if d.abs() < 1e-9 {
                        break;
                    }
                    t -= err / d;
                }
                if !(0.0..=1.0).contains(&t) || (bez(x1, x2, t) - p).abs() > 1e-7 {
                    let (mut lo, mut hi) = (0.0, 1.0);
                    for _ in 0..60 {
                        t = 0.5 * (lo + hi);
                        if bez(x1, x2, t) < p {
                            lo = t;
                        } else {
                            hi = t;
                        }
                    }
                }
                bez(y1, y2, t)
            }
            Ease::In => p * p,
            Ease::Out => 1.0 - (1.0 - p) * (1.0 - p),
            Ease::InOut => {
                if p < 0.5 {
                    2.0 * p * p
                } else {
                    1.0 - 2.0 * (1.0 - p) * (1.0 - p)
                }
            }
            Ease::InCubic => p * p * p,
            Ease::OutCubic => 1.0 - (1.0 - p).powi(3),
            Ease::InOutCubic => {
                if p < 0.5 {
                    4.0 * p * p * p
                } else {
                    1.0 - 4.0 * (1.0 - p).powi(3)
                }
            }
            Ease::OutBack => {
                let k = 1.70158;
                1.0 + (k + 1.0) * (p - 1.0).powi(3) + k * (p - 1.0).powi(2)
            }
            Ease::Step => {
                if p < 1.0 {
                    0.0
                } else {
                    1.0
                }
            }
            Ease::Spring { bounces, damping } => {
                let raw = |q: f64| {
                    1.0 - (-damping * q).exp() * (std::f64::consts::TAU * bounces * q).cos()
                };
                raw(p) / raw(1.0)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    To([f64; 2]),
    By([f64; 2]),
}

/// One animatable scalar: a vertex coordinate or a component of the shape transform (spec 7.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropId {
    Vx(u32),
    Vy(u32),
    PosX,
    PosY,
    ScaleX,
    ScaleY,
    Rot,
    /// 3D: depth, turns about x and y, and scale along z (meshes and `camera3`).
    PosZ,
    RotX,
    RotY,
    ScaleZ,
    /// A mesh vertex coordinate (vertex index, axis 0 x, 1 y, 2 z).
    Mv(u32, u8),
    /// One component (0 r, 1 g, 2 b, 3 a; linear light) of a face's own fill.
    FillC(u32, u8),
    /// ... of an edge's own stroke colour.
    StrokeC(u32, u8),
    /// ... of the shape's fill, stroke colour and tint.
    ShapeFillC(u8),
    ShapeStrokeC(u8),
    TintC(u8),
    /// An edge's own stroke width and the shape's.
    EdgeWidth(u32),
    ShapeWidth,
    Opacity,
    /// How much of a paint layer's stroke is painted (0 to 1).
    StrokeProgress(u32),
}

impl PropId {
    pub fn is_colour(self) -> bool {
        matches!(
            self,
            PropId::FillC(..)
                | PropId::StrokeC(..)
                | PropId::ShapeFillC(_)
                | PropId::ShapeStrokeC(_)
                | PropId::TintC(_)
        )
    }
}

// Channels other than vertex ids (which are small): each colour and width has its own.
pub fn face_fill_channel(id: u32) -> u32 {
    0x4000_0000 | id
}
pub fn edge_stroke_channel(id: u32) -> u32 {
    0x6000_0000 | id
}
pub fn edge_width_channel(id: u32) -> u32 {
    0x5000_0000 | id
}
pub const CH_SHAPE_FILL: u32 = 0x7000_0001;
pub const CH_SHAPE_STROKE: u32 = 0x7000_0002;
pub const CH_OPACITY: u32 = 0x7000_0003;
pub const CH_TINT: u32 = 0x7000_0004;
pub const CH_SHAPE_WIDTH: u32 = 0x7000_0005;

/// How a scalar moves during an action: eased between `from` and `to`, or along an analytic path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Eased,
    /// Constant acceleration `g` from speed `v0`, optionally bouncing on a floor with restitution.
    Fall {
        v0: f64,
        g: f64,
        floor: Option<f64>,
        restitution: f64,
    },
    /// Constant speed `v`, reflected between the walls `lo` and `hi`.
    Reflect {
        lo: f64,
        hi: f64,
        v: f64,
    },
}

/// The value of a physics-style motion `t` seconds after it started at `x0` (stateless and exact).
pub fn motion_value(m: &Motion, x0: f64, t: f64) -> f64 {
    match *m {
        Motion::Eased => x0,
        Motion::Reflect { lo, hi, v } => {
            let w = hi - lo;
            if w <= 0.0 {
                return lo;
            }
            let u = (x0.clamp(lo, hi) - lo + v * t).rem_euclid(2.0 * w);
            lo + if u > w { 2.0 * w - u } else { u }
        }
        Motion::Fall {
            v0,
            g,
            floor,
            restitution,
        } => {
            let Some(f) = floor else {
                return x0 + v0 * t + 0.5 * g * t * t;
            };
            let (mut p, mut v, mut left) = (x0, v0, t);
            if p >= f {
                p = f;
                if v > 0.0 {
                    v *= -restitution;
                    if v.abs() < 0.5 {
                        return f;
                    }
                }
            }
            if g <= 0.0 {
                return p + v * left;
            }
            for _ in 0..200 {
                let disc = v * v + 2.0 * g * (f - p);
                let tau = (-v + disc.max(0.0).sqrt()) / g;
                if left <= tau {
                    return p + v * left + 0.5 * g * left * left;
                }
                left -= tau;
                let hit = v + g * tau;
                p = f;
                v = -restitution * hit;
                if restitution == 0.0 || v.abs() < 0.5 {
                    return f;
                }
            }
            f
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKind {
    Wobble,
    Noise,
    /// An equation (see `mathfx`): the values come from `ModReq::drive`, not from `comps`.
    Drive,
}

/// An equation program ready to run: the interpreter, or (with the `jit` feature, for many points) machine code
/// that gives the very same bits.
#[derive(Debug)]
pub enum Kernel {
    Interp(animscript_math::Program),
    #[cfg(feature = "jit")]
    Jit(Box<animscript_math::Compiled>),
}

impl Kernel {
    pub fn eval_n(
        &self,
        n: usize,
        point: &[&[f64]],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<Vec<f64>>, animscript_math::EvalError> {
        match self {
            Kernel::Interp(p) => p.eval_n(n, point, frame, params),
            #[cfg(feature = "jit")]
            Kernel::Jit(c) => c.eval_n(n, point, frame, params),
        }
    }
    pub fn is_jit(&self) -> bool {
        #[cfg(feature = "jit")]
        if matches!(self, Kernel::Jit(_)) {
            return true;
        }
        false
    }
}

/// When `drive` equations are compiled to machine code (only in a build with the `jit` feature; results never
/// depend on it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JitMode {
    /// For drives over many vertices.
    #[default]
    Auto,
    Always,
    Off,
}

/// What a `drive` computes: a compiled equation program and where each output goes.
#[derive(Debug)]
pub struct Drive {
    /// Point inputs `x, y, i, u`; frame inputs `t, n`; one output per entry of `outs`.
    pub prog: Kernel,
    /// The vertices the equation runs over (empty for shape-level properties, which see one point).
    pub verts: Vec<u32>,
    pub outs: Vec<DriveOut>,
    /// Replace the rest value instead of adding to it.
    pub replace: bool,
    /// Multiplies time (`t` is seconds since the drive started, times this).
    pub speed: f64,
    /// Extra point inputs after `x, y, i, u`: a field's channel sampled at each point (field name, channel).
    pub fields: Vec<(String, usize)>,
    /// A drive of a mesh's vertices: `z` is available and fields are read at the vertex's (x, z).
    pub mesh: bool,
}

/// One output of a drive: `props[k]` receives point `k`'s value (vertex drives) or all receive point 0 (shape level).
#[derive(Clone, Debug)]
pub struct DriveOut {
    pub props: Vec<PropId>,
}

/// One scalar driven by a procedural modifier.
#[derive(Clone, Debug)]
pub struct ModComp {
    pub prop: PropId,
    pub amount: f64,
    /// Cycles per second (wobble) or lattice steps per second (noise).
    pub speed: f64,
    /// The seeded stream (spec 6.11); a pure function of it and the time gives the value.
    pub seed: u64,
    pub phase: f64,
}

/// A procedural modifier (layer 2): adds an offset on top of the layers below.
#[derive(Clone, Debug)]
pub struct ModReq {
    pub shape: String,
    pub label: String,
    pub kind: ModKind,
    pub comps: Vec<ModComp>,
    /// `None` runs until cancelled or the scene ends.
    pub frames: Option<i64>,
    pub blend_in: i64,
    pub blend_out: i64,
    pub drive: Option<std::rc::Rc<Drive>>,
}

/// What a `start` statement launches without blocking.
#[derive(Clone, Debug)]
pub enum StartReq {
    Act(ActionReq),
    Mod(ModReq),
    /// A user-defined action started in the background.
    Task(Rc<UserCall>),
}

/// A scalar target: an absolute value or a change relative to the value when the action starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tgt {
    To(f64),
    By(f64),
}

/// The channel of a shape's own transform (vertices use their id).
pub const BODY: u32 = u32::MAX;

/// A request to start a timed action on some scalar properties of one shape.
#[derive(Clone, Debug)]
pub struct ActionReq {
    pub shape: String,
    pub label: String,
    pub frames: i64,
    pub ease: Ease,
    /// Channels occupied for the whole duration (acquired atomically).
    pub channels: Vec<u32>,
    pub comps: Vec<(PropId, Tgt)>,
    /// Parallel to `comps`, or empty when every component is eased.
    pub motions: Vec<Motion>,
}

impl ActionReq {
    /// An action that moves whole vertices; each vertex is its own channel.
    pub fn vertices(
        shape: String,
        label: String,
        frames: i64,
        ease: Ease,
        items: Vec<(u32, Target)>,
    ) -> ActionReq {
        let mut comps = vec![];
        for (vid, t) in &items {
            let (x, y) = match t {
                Target::To(p) => (Tgt::To(p[0]), Tgt::To(p[1])),
                Target::By(d) => (Tgt::By(d[0]), Tgt::By(d[1])),
            };
            comps.push((PropId::Vx(*vid), x));
            comps.push((PropId::Vy(*vid), y));
        }
        ActionReq {
            shape,
            label,
            frames,
            ease,
            channels: items.iter().map(|x| x.0).collect(),
            comps,
            motions: vec![],
        }
    }
}

/// A set of keys for one scalar (spec 7.7), ready to become a layer-3 contribution.
#[derive(Clone, Debug)]
pub struct KeySetSpec {
    pub shape: String,
    pub prop: PropId,
    /// `+=` keys add an offset instead of overriding.
    pub add: bool,
    /// `(tick, value, easing)`: relative to the statement's start, or absolute scene ticks.
    pub keys: Vec<(i64, f64, Ease)>,
    pub absolute: bool,
    pub hold_before: bool,
    pub hold_after: bool,
    pub blend_in: i64,
    pub blend_out: i64,
}

#[derive(Clone, Debug)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Dur(Rat),
    Frames(i64),
    /// Stored in turns, like every Angle (spec §3.1).
    Angle(f64),
    Color([u8; 4]),
    Tuple(Vec<Value>),
    List(Vec<Value>),
    Shape(String),
    Elem(String, ElemRef),
    /// `shape.v`, `shape.e`, `shape.f`, waiting for `[n]`.
    Index(String, Kind),
    Entity(String),
    Ease(Ease),
    SceneObj,
    Fresh(Box<Shape>),
    Pending(Box<ActionReq>),
    Lambda(Rc<LambdaData>),
    /// A modifier waiting for the statement that called it to run it.
    Modifier(Box<ModReq>),
    /// A running action or modifier started with `start`.
    Handle(u32),
    /// `rect(x, y, w, h)`.
    Rect([f64; 4]),
    /// A box in a 3D scene: x, y, z of the near top-left corner, then width, height, depth.
    Box3([f64; 6]),
    /// A physics joint, by index.
    Joint(usize),
    /// A particle emitter, by name.
    Emitter(String),
    /// A brush made by `brush(...)`.
    Brush(Rc<crate::paint::BrushSpec>),
    /// A fluid, by name.
    Fluid(String),
    /// A field (user equations stepped over a grid), by name.
    Field(String),
    /// A loaded picture, for `shape.fill = image("file.png")`.
    Image(ImageFill),
    /// An armature, by name; its bones and IK handles are members.
    Armature(String),
    /// A score declared with `score name:`, by name; `play` and `stop` are its methods.
    Score(String),
    /// A call of a user-defined `action`, ready to run (see `defs.rs`).
    Call(Rc<UserCall>),
}

/// An `action` with its arguments bound to its parameters.
#[derive(Debug)]
pub struct UserCall {
    pub def: Rc<crate::ast::Def>,
    pub scope: std::collections::HashMap<String, Value>,
}

/// A lambda with the variables visible where it was written (captured by value).
#[derive(Clone, Debug)]
pub struct LambdaData {
    pub params: Vec<String>,
    pub body: Rc<Expr>,
    pub vars: Vars,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::None => "None",
            Value::Bool(_) => "Bool",
            Value::Int(_) => "Int",
            Value::Num(_) => "Number",
            Value::Str(_) => "String",
            Value::Dur(_) | Value::Frames(_) => "Duration",
            Value::Angle(_) => "Angle",
            Value::Color(_) => "Color",
            Value::Tuple(_) => "Vec2",
            Value::List(_) => "List",
            Value::Shape(_) | Value::Fresh(_) => "Shape",
            Value::Elem(_, r) => match r.kind {
                Kind::V => "Vertex",
                Kind::E => "Edge",
                Kind::F => "Face",
            },
            Value::Index(..) => "ElementIndex",
            Value::Entity(_) => "Entity",
            Value::Ease(_) => "Easing",
            Value::SceneObj => "Scene",
            Value::Brush(_) => "Brush",
            Value::Pending(_) => "Action",
            Value::Lambda(_) => "Lambda",
            Value::Modifier(_) => "Modifier",
            Value::Handle(_) => "Handle",
            Value::Rect(_) | Value::Box3(_) => "Region",
            Value::Joint(_) => "Joint",
            Value::Emitter(_) => "Emitter",
            Value::Fluid(_) => "Fluid",
            Value::Field(_) => "Field",
            Value::Image(_) => "Image",
            Value::Armature(_) => "Armature",
            Value::Score(_) => "Score",
            Value::Call(_) => "Action call",
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
}
