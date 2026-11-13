//! Nodes, the builder (hash-consing, constant folding, conservative simplification) and finished programs.
use std::collections::HashMap;

pub type Id = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Un {
    Neg,
    Abs,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Log,
    Log2,
    Log10,
    Floor,
    Ceil,
    Round,
    Trunc,
    Fract,
    Sign,
    /// 1 when the operand is 0, else 0.
    Not,
    /// A deterministic hash of the float's bits, in [0, 1).
    Hash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    /// `fmod`: the remainder with the sign of the dividend.
    Rem,
    Pow,
    Min,
    Max,
    Atan2,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    /// 1 when both operands are non-zero.
    And,
    Or,
}

/// How often an input changes: per point of the batch, or once per evaluation (like the time `t`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    Point,
    Frame,
}

/// One operation. Constants are kept as their bits so nodes can be compared and hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Const(u64),
    /// The `n`-th declared input.
    Input(u32),
    /// The `n`-th declared parameter (a number set when the program runs).
    Param(u32),
    Un(Un, Id),
    Bin(Bin, Id, Id),
    /// `cond != 0 ? a : b`, evaluating both sides.
    Select(Id, Id, Id),
    /// Smooth value noise in [-1, 1] of 1 to 3 coordinates (unused ones repeat the first), seeded.
    Noise {
        dims: u8,
        seed: u32,
        args: [Id; 3],
    },
}

/// The IEEE semantics of a unary operation (the one definition every backend must match).
pub fn un_apply(op: Un, a: f64) -> f64 {
    match op {
        Un::Neg => -a,
        Un::Abs => a.abs(),
        Un::Sqrt => a.sqrt(),
        Un::Sin => libm::sin(a),
        Un::Cos => libm::cos(a),
        Un::Tan => libm::tan(a),
        Un::Asin => libm::asin(a),
        Un::Acos => libm::acos(a),
        Un::Atan => libm::atan(a),
        Un::Sinh => libm::sinh(a),
        Un::Cosh => libm::cosh(a),
        Un::Tanh => libm::tanh(a),
        Un::Exp => libm::exp(a),
        Un::Log => libm::log(a),
        Un::Log2 => libm::log2(a),
        Un::Log10 => libm::log10(a),
        Un::Floor => a.floor(),
        Un::Ceil => a.ceil(),
        Un::Round => a.round(),
        Un::Trunc => a.trunc(),
        Un::Fract => a - a.floor(),
        Un::Sign => {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Un::Not => f64::from(a == 0.0),
        Un::Hash => {
            let mut z = a.to_bits().wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            (z >> 11) as f64 / (1u64 << 53) as f64
        }
    }
}

/// The IEEE semantics of a binary operation.
pub fn bin_apply(op: Bin, a: f64, b: f64) -> f64 {
    match op {
        Bin::Add => a + b,
        Bin::Sub => a - b,
        Bin::Mul => a * b,
        Bin::Div => a / b,
        Bin::Rem => libm::fmod(a, b),
        Bin::Pow => libm::pow(a, b),
        Bin::Min => {
            if a.is_nan() || b < a {
                b
            } else {
                a
            }
        }
        Bin::Max => {
            if a.is_nan() || b > a {
                b
            } else {
                a
            }
        }
        Bin::Atan2 => libm::atan2(a, b),
        Bin::Lt => f64::from(a < b),
        Bin::Le => f64::from(a <= b),
        Bin::Gt => f64::from(a > b),
        Bin::Ge => f64::from(a >= b),
        Bin::Eq => f64::from(a == b),
        Bin::Ne => f64::from(a != b),
        Bin::And => f64::from(a != 0.0 && b != 0.0),
        Bin::Or => f64::from(a != 0.0 || b != 0.0),
    }
}

fn lattice(seed: u32, ix: i64, iy: i64, iz: i64) -> f64 {
    let mut z = (seed as u64) << 32 ^ 0x9E37_79B9_7F4A_7C15;
    for v in [ix, iy, iz] {
        z = (z ^ (v as u64)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^= z >> 29;
    }
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// Smooth value noise in [-1, 1]: random values on the integer lattice, blended with a smoothstep. Zero on every
/// lattice point's neighbourhood is not guaranteed: it is `lattice` value at the points, so it is not zero-mean per cell.
pub fn noise_apply(dims: u8, seed: u32, x: f64, y: f64, z: f64) -> f64 {
    if !(x.is_finite() && y.is_finite() && z.is_finite()) {
        return f64::NAN;
    }
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let (tx, ty, tz) = (
        s(x - fx),
        if dims >= 2 { s(y - fy) } else { 0.0 },
        if dims >= 3 { s(z - fz) } else { 0.0 },
    );
    let (ix, iy, iz) = (
        fx as i64,
        if dims >= 2 { fy as i64 } else { 0 },
        if dims >= 3 { fz as i64 } else { 0 },
    );
    let mix = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let corner = |dx: i64, dy: i64, dz: i64| {
        lattice(
            seed,
            ix.wrapping_add(dx),
            iy.wrapping_add(dy),
            iz.wrapping_add(dz),
        )
    };
    let plane = |dz: i64| {
        let (a, b) = (
            mix(corner(0, 0, dz), corner(1, 0, dz), tx),
            if dims >= 2 {
                mix(corner(0, 1, dz), corner(1, 1, dz), tx)
            } else {
                0.0
            },
        );
        if dims >= 2 {
            mix(a, b, ty)
        } else {
            a
        }
    };
    if dims >= 3 {
        mix(plane(0), plane(1), tz)
    } else {
        plane(0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDecl {
    pub name: String,
    pub rate: Rate,
}

/// Builds a graph. Adding a node that exists already returns the existing one (common sub-expressions are shared),
/// operations on constants are computed at once, and a few exact simplifications are applied.
#[derive(Clone, Debug, Default)]
pub struct Builder {
    pub(crate) nodes: Vec<Node>,
    intern: HashMap<Node, Id>,
    pub(crate) inputs: Vec<InputDecl>,
    pub(crate) params: Vec<String>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder::default()
    }

    fn push(&mut self, n: Node) -> Id {
        if let Some(id) = self.intern.get(&n) {
            return *id;
        }
        let id = self.nodes.len() as Id;
        self.nodes.push(n);
        self.intern.insert(n, id);
        id
    }

    pub fn constant(&mut self, v: f64) -> Id {
        self.push(Node::Const(v.to_bits()))
    }
    /// The value of a node if it is a constant.
    pub fn as_const(&self, id: Id) -> Option<f64> {
        match self.nodes[id as usize] {
            Node::Const(b) => Some(f64::from_bits(b)),
            _ => None,
        }
    }
    pub fn node(&self, id: Id) -> Node {
        self.nodes[id as usize]
    }

    /// Declare an input (or find it again by name).
    pub fn input(&mut self, name: &str, rate: Rate) -> Id {
        let idx = match self.inputs.iter().position(|i| i.name == name) {
            Some(i) => i,
            None => {
                self.inputs.push(InputDecl {
                    name: name.to_string(),
                    rate,
                });
                self.inputs.len() - 1
            }
        };
        self.push(Node::Input(idx as u32))
    }
    pub fn param(&mut self, name: &str) -> Id {
        let idx = match self.params.iter().position(|p| p == name) {
            Some(i) => i,
            None => {
                self.params.push(name.to_string());
                self.params.len() - 1
            }
        };
        self.push(Node::Param(idx as u32))
    }

    pub fn un(&mut self, op: Un, a: Id) -> Id {
        if let Some(x) = self.as_const(a) {
            return self.constant(un_apply(op, x));
        }
        match (op, self.nodes[a as usize]) {
            (Un::Neg, Node::Un(Un::Neg, inner)) => return inner,
            (Un::Abs, Node::Un(Un::Abs, _)) => return a,
            _ => {}
        }
        self.push(Node::Un(op, a))
    }

    pub fn bin(&mut self, op: Bin, a: Id, b: Id) -> Id {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            return self.constant(bin_apply(op, x, y));
        }
        let (ca, cb) = (self.as_const(a), self.as_const(b));
        // exact identities only: nothing that changes the sign of zero, NaNs or infinities
        match op {
            Bin::Mul if cb == Some(1.0) => return a,
            Bin::Mul if ca == Some(1.0) => return b,
            Bin::Div if cb == Some(1.0) => return a,
            Bin::Sub if cb == Some(0.0) && cb.is_some_and(|v| v.is_sign_positive()) => return a,
            Bin::Pow if cb == Some(1.0) => return a,
            _ => {}
        }
        // commutative operations are stored with the smaller id first, so `a + b` and `b + a` are one node
        let (a, b) = if matches!(
            op,
            Bin::Add | Bin::Mul | Bin::Min | Bin::Max | Bin::Eq | Bin::Ne | Bin::And | Bin::Or
        ) && a > b
        {
            (b, a)
        } else {
            (a, b)
        };
        self.push(Node::Bin(op, a, b))
    }

    pub fn select(&mut self, c: Id, a: Id, b: Id) -> Id {
        if let Some(v) = self.as_const(c) {
            return if v != 0.0 { a } else { b };
        }
        if a == b {
            return a;
        }
        self.push(Node::Select(c, a, b))
    }

    pub fn noise(&mut self, seed: u32, args: &[Id]) -> Id {
        let dims = args.len().clamp(1, 3) as u8;
        let mut a = [args[0]; 3];
        for (k, id) in args.iter().take(3).enumerate() {
            a[k] = *id;
        }
        if args.iter().take(3).all(|i| self.as_const(*i).is_some()) {
            let v: Vec<f64> = a.iter().map(|i| self.as_const(*i).unwrap()).collect();
            return self.constant(noise_apply(dims, seed, v[0], v[1], v[2]));
        }
        self.push(Node::Noise {
            dims,
            seed,
            args: a,
        })
    }

    // ---- composites -------------------------------------------------------------------------------

    pub fn add(&mut self, a: Id, b: Id) -> Id {
        self.bin(Bin::Add, a, b)
    }
    pub fn sub(&mut self, a: Id, b: Id) -> Id {
        self.bin(Bin::Sub, a, b)
    }
    pub fn mul(&mut self, a: Id, b: Id) -> Id {
        self.bin(Bin::Mul, a, b)
    }
    pub fn div(&mut self, a: Id, b: Id) -> Id {
        self.bin(Bin::Div, a, b)
    }
    pub fn neg(&mut self, a: Id) -> Id {
        self.un(Un::Neg, a)
    }
    pub fn clamp(&mut self, x: Id, lo: Id, hi: Id) -> Id {
        let m = self.bin(Bin::Max, x, lo);
        self.bin(Bin::Min, m, hi)
    }
    /// `a + (b - a) * t`
    pub fn mix(&mut self, a: Id, b: Id, t: Id) -> Id {
        let d = self.sub(b, a);
        let m = self.mul(d, t);
        self.add(a, m)
    }
    /// 0 below `e0`, 1 above `e1`, a smooth step between.
    pub fn smoothstep(&mut self, e0: Id, e1: Id, x: Id) -> Id {
        let (num, den) = (self.sub(x, e0), self.sub(e1, e0));
        let t = self.div(num, den);
        let (zero, one, two, three) = (
            self.constant(0.0),
            self.constant(1.0),
            self.constant(2.0),
            self.constant(3.0),
        );
        let t = self.clamp(t, zero, one);
        let a = self.mul(two, t);
        let b = self.sub(three, a);
        let t2 = self.mul(t, t);
        self.mul(t2, b)
    }
    /// 0 below `edge`, else 1.
    pub fn step(&mut self, edge: Id, x: Id) -> Id {
        self.bin(Bin::Ge, x, edge)
    }

    /// Finish: keep what the outputs need, in the order it was built.
    pub fn finish(&self, outputs: &[Id]) -> Program {
        let mut keep = vec![false; self.nodes.len()];
        let mut stack: Vec<Id> = outputs.to_vec();
        while let Some(id) = stack.pop() {
            if std::mem::replace(&mut keep[id as usize], true) {
                continue;
            }
            match self.nodes[id as usize] {
                Node::Un(_, a) => stack.push(a),
                Node::Bin(_, a, b) => stack.extend([a, b]),
                Node::Select(c, a, b) => stack.extend([c, a, b]),
                Node::Noise { dims, args, .. } => stack.extend(args.iter().take(dims as usize)),
                _ => {}
            }
        }
        let mut remap = vec![0 as Id; self.nodes.len()];
        let mut nodes = vec![];
        for (i, n) in self.nodes.iter().enumerate() {
            if !keep[i] {
                continue;
            }
            remap[i] = nodes.len() as Id;
            let r = |x: Id| remap[x as usize];
            nodes.push(match *n {
                Node::Un(o, a) => Node::Un(o, r(a)),
                Node::Bin(o, a, b) => Node::Bin(o, r(a), r(b)),
                Node::Select(c, a, b) => Node::Select(r(c), r(a), r(b)),
                Node::Noise { dims, seed, args } => {
                    let mut a = args;
                    for (k, id) in args.iter().enumerate() {
                        a[k] = if k < dims as usize {
                            r(*id)
                        } else {
                            r(args[0])
                        };
                    }
                    Node::Noise {
                        dims,
                        seed,
                        args: a,
                    }
                }
                other => other,
            });
        }
        Program {
            nodes,
            outputs: outputs.iter().map(|o| remap[*o as usize]).collect(),
            inputs: self.inputs.clone(),
            params: self.params.clone(),
        }
    }
}

/// A finished program: nodes in dependency order and the ids of its outputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub nodes: Vec<Node>,
    pub outputs: Vec<Id>,
    pub inputs: Vec<InputDecl>,
    pub params: Vec<String>,
}

impl Program {
    pub fn input_names(&self) -> Vec<&str> {
        self.inputs.iter().map(|i| i.name.as_str()).collect()
    }
    pub fn param_names(&self) -> Vec<&str> {
        self.params.iter().map(String::as_str).collect()
    }
    /// Which nodes depend on a per-point input (the rest are computed once per evaluation).
    pub fn varying(&self) -> Vec<bool> {
        let mut v = vec![false; self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            v[i] = match *n {
                Node::Const(_) | Node::Param(_) => false,
                Node::Input(k) => self.inputs[k as usize].rate == Rate::Point,
                Node::Un(_, a) => v[a as usize],
                Node::Bin(_, a, b) => v[a as usize] || v[b as usize],
                Node::Select(c, a, b) => v[c as usize] || v[a as usize] || v[b as usize],
                Node::Noise { dims, args, .. } => {
                    args.iter().take(dims as usize).any(|a| v[*a as usize])
                }
            };
        }
        v
    }
}
