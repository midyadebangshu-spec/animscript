//! Symbolic differentiation with respect to one input: the derivative of a node is built as new nodes in the same
//! builder (so shared sub-expressions stay shared and the usual simplifications apply).
use crate::ir::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffError(pub &'static str);

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot differentiate {}", self.0)
    }
}

impl Builder {
    /// `d(id) / d(wrt)`, where `wrt` is an input node. Step-like functions (floor, round, comparisons, sign) have
    /// derivative 0; noise cannot be differentiated.
    pub fn diff(&mut self, id: Id, wrt: Id) -> Result<Id, DiffError> {
        let Node::Input(k) = self.node(wrt) else {
            return Err(DiffError("with respect to something that is not an input"));
        };
        let mut memo = HashMap::new();
        self.d(id, k, &mut memo)
    }

    fn d(&mut self, id: Id, k: u32, memo: &mut HashMap<Id, Id>) -> Result<Id, DiffError> {
        if let Some(r) = memo.get(&id) {
            return Ok(*r);
        }
        let zero = self.constant(0.0);
        let one = self.constant(1.0);
        let node = self.node(id);
        let r = match node {
            Node::Const(_) | Node::Param(_) => zero,
            Node::Input(i) => {
                if i == k {
                    one
                } else {
                    zero
                }
            }
            Node::Select(c, a, b) => {
                let (da, db) = (self.d(a, k, memo)?, self.d(b, k, memo)?);
                self.select(c, da, db)
            }
            Node::Noise { .. } => return Err(DiffError("noise")),
            Node::Un(op, a) => {
                let da = self.d(a, k, memo)?;
                if da == zero
                    && !matches!(
                        op,
                        Un::Floor
                            | Un::Ceil
                            | Un::Round
                            | Un::Trunc
                            | Un::Sign
                            | Un::Not
                            | Un::Hash
                    )
                {
                    zero
                } else {
                    match op {
                        Un::Neg => self.neg(da),
                        Un::Abs => {
                            let s = self.un(Un::Sign, a);
                            self.mul(s, da)
                        }
                        Un::Sqrt => {
                            let two = self.constant(2.0);
                            let s = self.mul(two, id);
                            self.div(da, s)
                        }
                        Un::Sin => {
                            let c = self.un(Un::Cos, a);
                            self.mul(c, da)
                        }
                        Un::Cos => {
                            let s = self.un(Un::Sin, a);
                            let m = self.mul(s, da);
                            self.neg(m)
                        }
                        Un::Tan => {
                            let c = self.un(Un::Cos, a);
                            let c2 = self.mul(c, c);
                            self.div(da, c2)
                        }
                        Un::Asin | Un::Acos => {
                            let a2 = self.mul(a, a);
                            let r = self.sub(one, a2);
                            let s = self.un(Un::Sqrt, r);
                            let q = self.div(da, s);
                            if op == Un::Asin {
                                q
                            } else {
                                self.neg(q)
                            }
                        }
                        Un::Atan => {
                            let a2 = self.mul(a, a);
                            let den = self.add(one, a2);
                            self.div(da, den)
                        }
                        Un::Sinh => {
                            let c = self.un(Un::Cosh, a);
                            self.mul(c, da)
                        }
                        Un::Cosh => {
                            let s = self.un(Un::Sinh, a);
                            self.mul(s, da)
                        }
                        Un::Tanh => {
                            let t2 = self.mul(id, id);
                            let f = self.sub(one, t2);
                            self.mul(f, da)
                        }
                        Un::Exp => self.mul(id, da),
                        Un::Log => self.div(da, a),
                        Un::Log2 | Un::Log10 => {
                            let base = self.constant(if op == Un::Log2 { 2.0 } else { 10.0 });
                            let ln = self.un(Un::Log, base);
                            let den = self.mul(a, ln);
                            self.div(da, den)
                        }
                        Un::Fract => da,
                        Un::Floor
                        | Un::Ceil
                        | Un::Round
                        | Un::Trunc
                        | Un::Sign
                        | Un::Not
                        | Un::Hash => zero,
                    }
                }
            }
            Node::Bin(op, a, b) => {
                let (da, db) = (self.d(a, k, memo)?, self.d(b, k, memo)?);
                match op {
                    Bin::Add => self.add(da, db),
                    Bin::Sub => self.sub(da, db),
                    Bin::Mul => {
                        // a zero derivative drops its term outright (x * 0 is not folded: x could be infinite)
                        let l = if da == zero { zero } else { self.mul(da, b) };
                        let r = if db == zero { zero } else { self.mul(a, db) };
                        self.add(l, r)
                    }
                    Bin::Div => {
                        let (l, r) = (self.mul(da, b), self.mul(a, db));
                        let num = self.sub(l, r);
                        let den = self.mul(b, b);
                        self.div(num, den)
                    }
                    Bin::Pow => {
                        if db == zero {
                            // b * a^(b - 1) * da
                            let bm1 = self.sub(b, one);
                            let p = self.bin(Bin::Pow, a, bm1);
                            let t = self.mul(b, p);
                            self.mul(t, da)
                        } else {
                            // a^b * (db ln a + b da / a)
                            let ln = self.un(Un::Log, a);
                            let t1 = self.mul(db, ln);
                            let q = self.div(da, a);
                            let t2 = self.mul(b, q);
                            let s = self.add(t1, t2);
                            self.mul(id, s)
                        }
                    }
                    Bin::Min => {
                        let c = self.bin(Bin::Lt, a, b);
                        self.select(c, da, db)
                    }
                    Bin::Max => {
                        let c = self.bin(Bin::Gt, a, b);
                        self.select(c, da, db)
                    }
                    Bin::Rem => {
                        // fmod(a, b) = a - b * trunc(a / b): between jumps the derivative is da - db * trunc(a / b)
                        let q = self.div(a, b);
                        let t = self.un(Un::Trunc, q);
                        let m = self.mul(db, t);
                        self.sub(da, m)
                    }
                    Bin::Atan2 => {
                        // atan2(y, x): (x dy - y dx) / (x^2 + y^2)
                        let (l, r) = (self.mul(b, da), self.mul(a, db));
                        let num = self.sub(l, r);
                        let (a2, b2) = (self.mul(a, a), self.mul(b, b));
                        let den = self.add(a2, b2);
                        self.div(num, den)
                    }
                    Bin::Lt
                    | Bin::Le
                    | Bin::Gt
                    | Bin::Ge
                    | Bin::Eq
                    | Bin::Ne
                    | Bin::And
                    | Bin::Or => zero,
                }
            }
        };
        memo.insert(id, r);
        Ok(r)
    }
}
