//! Logging setup. Metrics and Sentry join this crate in phase 3.

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
    fn init_is_idempotent_within_a_process() {
        // The global subscriber can only be set once; a second call must report
        // that rather than panic, because tests and `serve` both call it.
        let first = init_logging(LogLevel::Info, LogFormat::Json);
        let second = init_logging(LogLevel::Info, LogFormat::Json);
        assert!(first.is_ok() || matches!(first, Err(TelemetryError::AlreadyInitialized)));
        assert!(matches!(second, Err(TelemetryError::AlreadyInitialized)));
    }
}
