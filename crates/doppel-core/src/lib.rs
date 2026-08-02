//! Configuration model, validation, storage, and runtime state for Doppel.

pub mod config;
pub mod error;
pub mod method;
pub mod metrics;
pub mod redact;
pub mod reload;
pub mod runtime;
pub mod store;
pub mod validate;

pub use config::{Config, ConfigError};
pub use error::{Error, ErrorBody, ErrorCode};
pub use redact::redact_credentials;
pub use reload::{ReloadFailure, ReloadOutcome, reload};
pub use runtime::{CompiledMock, CompiledProxy, MockBody, Runtime, RuntimeHolder};
pub use store::{Revision, StoreError};
pub use validate::{Violation, validate};
