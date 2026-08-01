//! Configuration storage. `FileStore` here, `PostgresStore` in phase 4.

pub mod name;

use std::path::PathBuf;

use crate::validate::Violation;

/// A revision counter, bumped on every successful save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Revision(pub u64);

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("config not found: {0}")]
    NotFound(PathBuf),
    #[error("config is invalid")]
    Invalid(Vec<Violation>),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("serialization failed: {0}")]
    Serialize(String),
    #[error("template name `{name}` rejected: {reason}")]
    BadTemplateName { name: String, reason: String },
}
