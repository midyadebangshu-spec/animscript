//! AnimScript language front end and runtime: lexer, parser, interpreter, tick loop and snapshots.
#![allow(clippy::result_large_err)]

pub mod ast;
pub mod check;
pub mod color;
pub mod defs;
pub mod derive;
pub mod field;
pub mod fluid;
pub mod fluid3;
pub mod freehand;
pub mod imports;
pub mod interp;
pub mod journal;
pub mod layers;
pub mod lexer;
pub mod mathfx;
pub mod mesh3d;
pub mod paint;
pub mod parser;
pub mod particles;
pub mod phys;
pub mod phys3;
pub mod procedural;
pub mod ragdoll;
pub mod rig;
pub mod runtime;
pub mod snapshot;
pub mod soft;
pub mod soft3;
pub mod value;

pub use animscript_core::{Diagnostic, Res, Severity, Span};
pub use check::{check, check_in};
pub use interp::Interp;
pub use journal::Journal;
pub use runtime::{run, run_in, run_partial, run_partial_in, RunOptions, RunResult};
pub use snapshot::snapshot_hash;

/// Parse a program and run its build phase (top-level statements and `shape` blocks).
pub fn load(src: &str) -> Res<Interp> {
    load_in(src, std::path::Path::new("."))
}

/// Like `load`, with the folder that relative asset paths are resolved from.
pub fn load_in(src: &str, base: &std::path::Path) -> Res<Interp> {
    let src = &imports::expand_imports(src, base)?;
    let program = parser::parse(src)?;
    let mut it = Interp::new(src, program)?;
    it.base_dir = base.to_path_buf();
    it.build()?;
    Ok(it)
}

/// Run `f` on a thread with a large stack: a user `fn` that calls itself recurses on the Rust stack,
/// and the default stack of a test or worker thread is small. A panic inside `f` is re-raised here.
pub(crate) fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let handle = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .expect("a thread for the interpreter");
    match handle.join() {
        Ok(v) => v,
        Err(e) => std::panic::resume_unwind(e),
    }
}
