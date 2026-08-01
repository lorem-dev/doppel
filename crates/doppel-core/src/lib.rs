//! Configuration model, validation, storage, and runtime state for Doppel.

pub mod config;
pub mod error;
pub mod validate;

pub use config::{Config, ConfigError};
pub use error::{Error, ErrorBody, ErrorCode};
pub use validate::{Violation, validate};
