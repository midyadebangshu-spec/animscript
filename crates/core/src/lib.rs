//! AnimScript core: diagnostics and the editable 2D shape model.
#![allow(clippy::result_large_err)]

pub mod diag;
pub mod geometry;

pub use diag::{Diagnostic, Res, Severity, Span};
pub use geometry::*;
