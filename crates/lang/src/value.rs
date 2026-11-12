//! Runtime values, exact rationals for durations, and easing functions.
use crate::ast::Expr;
use crate::interp::Vars;
use animscript_core::{ElemRef, Kind, Shape};
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    To([f64; 2]),
    By([f64; 2]),
}

/// A request to start a timed action on some vertices of one shape.
#[derive(Clone, Debug)]
pub struct ActionReq {
    pub shape: String,
    pub label: String,
    pub frames: i64,
    pub ease: Ease,
    pub items: Vec<(u32, Target)>,
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
            Value::Pending(_) => "Action",
            Value::Lambda(_) => "Lambda",
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
