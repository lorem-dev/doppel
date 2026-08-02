//! The admin HTTP API: proxy CRUD, template upload, reload, status, metrics.

pub mod access;

pub use access::{Action, Caller, authorize, caller_from_headers};
