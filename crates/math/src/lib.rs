//! The mathematical IR of AnimScript: an equation of `x`, `y`, `t` and parameters is built as a graph of scalar
//! operations (one node per distinct sub-expression), simplified, differentiated if wanted, and evaluated over a
//! whole batch of points at once. Everything is IEEE `f64` arithmetic with the transcendental functions from the
//! pure-Rust `libm` crate (never the platform's), so the same program on the same inputs gives the same bits on
//! every machine. Backends (a JIT, say) must reproduce the interpreter bit for bit.
mod diff;
mod eval;
mod ir;
mod parse;

pub use diff::DiffError;
pub use eval::EvalError;
pub use ir::{bin_apply, noise_apply, un_apply, Bin, Builder, Id, Node, Program, Rate, Un};
pub use parse::{parse, Env, ParseError};
