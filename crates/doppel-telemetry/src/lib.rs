//! Logging setup, and optional Sentry reporting.

pub mod sentry;

pub use sentry::{Sentry, SentryStatus};

use doppel_core::config::{LogFormat, LogLevel};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("logging is already initialized")]
    AlreadyInitialized,
    #[error("invalid log filter: {0}")]
    BadFilter(String),
    /// The DSN in the message is already masked -- see
    /// `doppel_core::redact_credentials`. A Sentry DSN carries the key that
    /// authorises sending events in its userinfo, so echoing the operator's
    /// value back into a log line would publish it.
    #[error("invalid sentry dsn `{dsn}`: {reason}")]
    BadSentryDsn { dsn: String, reason: String },
}

/// Resolve the filter directive. `RUST_LOG` wins over the config, because that
/// is what operators reach for when they need more detail from a running
/// process and cannot edit the config.
fn filter_directive(level: LogLevel, rust_log: Option<String>) -> String {
    match rust_log {
        Some(value) if !value.trim().is_empty() => value,
        _ => level.as_str().to_owned(),
    }
}

/// Install the global subscriber writing to stdout.
pub fn init_logging(level: LogLevel, format: LogFormat) -> Result<(), TelemetryError> {
    let directive = filter_directive(level, std::env::var("RUST_LOG").ok());
    let filter =
        EnvFilter::try_new(&directive).map_err(|e| TelemetryError::BadFilter(e.to_string()))?;

    let registry = tracing_subscriber::registry().with(filter);
    // Added whether or not a client is bound. `sentry_tracing` resolves the
    // hub per event, so with no client the layer drops everything, which is
    // what lets `init_logging` run before `sentry::init` and still capture
    // the warnings that initialisation itself emits.
    #[cfg(feature = "sentry")]
    let registry = registry.with(sentry_tracing::layer());
    let result = match format {
        LogFormat::Json => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::io::stdout),
            )
            .try_init(),
        LogFormat::Text => registry
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
            .try_init(),
    };
    result.map_err(|_| TelemetryError::AlreadyInitialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use doppel_core::config::{LogFormat, LogLevel};

    #[test]
    fn filter_uses_the_configured_level() {
        assert_eq!(filter_directive(LogLevel::Debug, None), "debug");
        assert_eq!(filter_directive(LogLevel::Warn, None), "warn");
    }

    #[test]
    fn rust_log_overrides_the_configured_level() {
        assert_eq!(
            filter_directive(LogLevel::Error, Some("doppel_proxy=trace".to_owned())),
            "doppel_proxy=trace"
        );
    }

    #[test]
    fn empty_rust_log_is_ignored() {
        assert_eq!(
            filter_directive(LogLevel::Info, Some(String::new())),
            "info"
        );
    }

    #[test]
    fn an_invalid_filter_directive_yields_bad_filter_with_a_useful_message() {
        // Exercises exactly the two steps `init_logging` performs before it
        // ever touches the global subscriber: resolving the directive, then
        // building the filter from it. `init_logging` itself cannot safely
        // be called here to prove this -- the global subscriber can only be
        // installed once per process (see `init_is_idempotent_within_a_process`
        // below), so a second, unrelated test in this same binary could
        // already have consumed that one shot, and this test would then
        // observe `AlreadyInitialized` instead of `BadFilter` depending on
        // test execution order. The directive-resolution and
        // filter-construction steps below have no such shared state.
        let directive =
            filter_directive(LogLevel::Info, Some("not a valid directive===".to_owned()));
        let parsed = EnvFilter::try_new(&directive);
        assert!(
            parsed.is_err(),
            "the chosen directive must actually be invalid for this test to mean anything"
        );

        // The same mapping `init_logging` applies to this same error.
        let err = parsed.map_err(|e| TelemetryError::BadFilter(e.to_string()));
        let TelemetryError::BadFilter(message) = err.unwrap_err() else {
            panic!("expected BadFilter");
        };
        assert!(!message.is_empty(), "the message must say something useful");
    }

    #[test]
    fn init_is_idempotent_within_a_process() {
        // The global subscriber can only be set once; a second call must report
        // that rather than panic, because tests and `serve` both call it.
        let first = init_logging(LogLevel::Info, LogFormat::Json);
        let second = init_logging(LogLevel::Info, LogFormat::Json);
        assert!(first.is_ok() || matches!(first, Err(TelemetryError::AlreadyInitialized)));
        assert!(matches!(second, Err(TelemetryError::AlreadyInitialized)));
    }
}
