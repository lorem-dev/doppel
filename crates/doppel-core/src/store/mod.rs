//! Configuration storage. `FileStore` here, `PostgresStore` in phase 4.

pub mod file;
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

pub use file::FileStore;

/// One template file, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFile {
    pub name: String,
    pub content: Vec<u8>,
}

/// Where configuration lives. `FileStore` implements it now; `PostgresStore`
/// implements it in phase 4 without any change to callers.
#[async_trait::async_trait]
pub trait ConfigStore: Send + Sync {
    /// Load and validate the configuration.
    async fn load(&self) -> Result<crate::Config, StoreError>;

    /// Validate and persist the configuration, returning the new revision.
    async fn save(
        &self,
        config: &crate::Config,
        actor: Option<&str>,
    ) -> Result<Revision, StoreError>;

    /// Every template file belonging to a proxy. An unknown proxy yields an
    /// empty list rather than an error: having no templates is normal.
    async fn load_templates(&self, proxy: &str) -> Result<Vec<TemplateFile>, StoreError>;

    async fn save_template(&self, proxy: &str, file: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// Returns whether the file existed.
    async fn delete_template(&self, proxy: &str, file: &str) -> Result<bool, StoreError>;

    /// Drop every template for `proxy` except those named in `keep`. An empty
    /// `keep` removes the proxy's storage entirely.
    async fn retain_templates(&self, proxy: &str, keep: &[String]) -> Result<(), StoreError>;
}
