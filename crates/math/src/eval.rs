//! The reference interpreter: one pass over the nodes in order, each computed for the whole batch of points at once
//! (nodes that depend on no per-point input are computed once).
use crate::ir::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalError {
    /// The program has `expected` per-point inputs, `got` slices were given.
    PointInputs {
        expected: usize,
        got: usize,
    },
    FrameInputs {
        expected: usize,
        got: usize,
    },
    Params {
        expected: usize,
        got: usize,
    },
    /// The per-point slices have different lengths.
    Length,
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::PointInputs { expected, got } => write!(
                f,
                "the program takes {expected} per-point inputs, {got} were given"
            ),
            EvalError::FrameInputs { expected, got } => write!(
                f,
                "the program takes {expected} per-frame inputs, {got} were given"
            ),
            EvalError::Params { expected, got } => write!(
                f,
                "the program takes {expected} parameters, {got} were given"
            ),
            EvalError::Length => write!(f, "the per-point inputs have different lengths"),
        }
    }
}

enum Reg {
    U(f64),
    V(Vec<f64>),
}

fn at(r: &Reg, i: usize) -> f64 {
    match r {
        Reg::U(v) => *v,
        Reg::V(v) => v[i],
    }
}

impl Program {
    /// Where each declared input lives: its rate and its index among the inputs of that rate.
    pub(crate) fn input_slots(&self) -> Vec<(Rate, usize)> {
        let (mut pi, mut fi) = (0, 0);
        self.inputs
            .iter()
            .map(|i| match i.rate {
                Rate::Point => {
                    pi += 1;
                    (Rate::Point, pi - 1)
                }
                Rate::Frame => {
                    fi += 1;
                    (Rate::Frame, fi - 1)
                }
            })
            .collect()
    }

    /// Check the arguments of an evaluation and say where each declared input lives (its rate and its index among
    /// the inputs of that rate).
    pub(crate) fn slots(
        &self,
        n: usize,
        point: &[&[f64]],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<(Rate, usize)>, EvalError> {
        let (np, nf) = (
            self.inputs.iter().filter(|i| i.rate == Rate::Point).count(),
            self.inputs.iter().filter(|i| i.rate == Rate::Frame).count(),
        );
        if point.len() != np {
            return Err(EvalError::PointInputs {
                expected: np,
                got: point.len(),
            });
        }
        if frame.len() != nf {
            return Err(EvalError::FrameInputs {
                expected: nf,
                got: frame.len(),
            });
        }
        if params.len() != self.params.len() {
            return Err(EvalError::Params {
                expected: self.params.len(),
                got: params.len(),
            });
        }
        if point.iter().any(|p| p.len() != n) {
            return Err(EvalError::Length);
        }
        Ok(self.input_slots())
    }
}

impl Program {
    /// Evaluate over `n` points. `point` holds one slice per per-point input (in the order they were declared) and
    /// `frame` one number per per-frame input; the result has one vector of `n` numbers per output.
    pub fn eval_n(
        &self,
        n: usize,
        point: &[&[f64]],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<Vec<f64>>, EvalError> {
        let slot = self.slots(n, point, frame, params)?;
        let varying = self.varying();
        let mut regs: Vec<Reg> = Vec::with_capacity(self.nodes.len());
        for (idx, node) in self.nodes.iter().enumerate() {
            let reg = match *node {
                Node::Const(b) => Reg::U(f64::from_bits(b)),
                Node::Param(k) => Reg::U(params[k as usize]),
                Node::Input(k) => match slot[k as usize] {
                    (Rate::Point, s) => Reg::V(point[s].to_vec()),
                    (Rate::Frame, s) => Reg::U(frame[s]),
                },
                Node::Un(op, a) => {
                    if varying[idx] {
                        Reg::V(
                            (0..n)
                                .map(|i| un_apply(op, at(&regs[a as usize], i)))
                                .collect(),
                        )
                    } else {
                        Reg::U(un_apply(op, at(&regs[a as usize], 0)))
                    }
                }
                Node::Bin(op, a, b) => {
                    if varying[idx] {
                        Reg::V(
                            (0..n)
                                .map(|i| {
                                    bin_apply(
                                        op,
                                        at(&regs[a as usize], i),
                                        at(&regs[b as usize], i),
                                    )
                                })
                                .collect(),
                        )
                    } else {
                        Reg::U(bin_apply(
                            op,
                            at(&regs[a as usize], 0),
                            at(&regs[b as usize], 0),
                        ))
                    }
                }
                Node::Select(c, a, b) => {
                    let pick = |i: usize| {
                        if at(&regs[c as usize], i) != 0.0 {
                            at(&regs[a as usize], i)
                        } else {
                            at(&regs[b as usize], i)
                        }
                    };
                    if varying[idx] {
                        Reg::V((0..n).map(pick).collect())
                    } else {
                        Reg::U(pick(0))
                    }
                }
                Node::Noise { dims, seed, args } => {
                    let f = |i: usize| {
                        noise_apply(
                            dims,
                            seed,
                            at(&regs[args[0] as usize], i),
                            at(&regs[args[1] as usize], i),
                            at(&regs[args[2] as usize], i),
                        )
                    };
                    if varying[idx] {
                        Reg::V((0..n).map(f).collect())
                    } else {
                        Reg::U(f(0))
                    }
                }
            };
            regs.push(reg);
        }
        Ok(self
            .outputs
            .iter()
            .map(|o| match &regs[*o as usize] {
                Reg::U(v) => vec![*v; n],
                Reg::V(v) => v.clone(),
            })
            .collect())
    }

    /// Like `eval_n`, with the number of points taken from the first per-point input (one point if there is none).
    pub fn eval(
        &self,
        point: &[&[f64]],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<Vec<f64>>, EvalError> {
        self.eval_n(point.first().map_or(1, |p| p.len()), point, frame, params)
    }

    /// Evaluate at one point.
    pub fn eval1(
        &self,
        point: &[f64],
        frame: &[f64],
        params: &[f64],
    ) -> Result<Vec<f64>, EvalError> {
        let cols: Vec<[f64; 1]> = point.iter().map(|v| [*v]).collect();
        let slices: Vec<&[f64]> = cols.iter().map(|c| c.as_slice()).collect();
        Ok(self
            .eval_n(1, &slices, frame, params)?
            .into_iter()
            .map(|v| v[0])
            .collect())
    }
}
