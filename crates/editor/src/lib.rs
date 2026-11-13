//! The AnimScript editor. The code is the document: every panel shows what the last run of the code
//! produced (frames, the run journal, diagnostics), and clicking something leads back to its source line.
#![allow(clippy::result_large_err)]

pub mod app;
pub mod engine;
pub mod highlight;
pub mod model;
pub mod panels;
