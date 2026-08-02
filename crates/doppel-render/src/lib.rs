//! Extraction and rendering for Doppel's mock engine.
//!
//! This crate knows nothing about HTTP: it takes already-extracted pieces
//! (a header value, a query parameter, a parsed body) and returns bound
//! variables or rendered bytes. That split is what makes rendering testable
//! without a network, the same reason validation lives in `doppel-core`.

pub mod extract;
pub mod render;

pub use extract::{Variables, parse_body};
pub use render::Renderer;
