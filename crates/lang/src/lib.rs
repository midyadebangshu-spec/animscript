//! AnimScript language front end and runtime: lexer, parser, interpreter, tick loop and snapshots.
#![allow(clippy::result_large_err)]

pub mod ast;
pub mod check;
pub mod color;
pub mod fluid;
pub mod interp;
pub mod layers;
pub mod lexer;
pub mod parser;
pub mod particles;
pub mod phys;
pub mod procedural;
pub mod runtime;
pub mod snapshot;
pub mod soft;
pub mod value;

pub use animscript_core::{Diagnostic, Res, Severity, Span};
pub use check::check;
pub use interp::Interp;
pub use runtime::{run, RunOptions, RunResult};
pub use snapshot::snapshot_hash;

/// Parse a program and run its build phase (top-level statements and `shape` blocks).
pub fn load(src: &str) -> Res<Interp> {
    let program = parser::parse(src)?;
    let mut it = Interp::new(src, program)?;
    it.build()?;
    Ok(it)
}
