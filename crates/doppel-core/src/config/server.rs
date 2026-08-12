//! Server, logging, control channel, template and Sentry settings.

use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
/// No `workers` here. It sizes the tokio runtime, and a database-backed
/// store cannot be opened before that runtime exists -- so the value has to
/// be known before the configuration is read, which puts it on the same side
/// of the boundary as the connection settings. It is `--workers` /
/// `DOPPEL_WORKERS`.
pub struct ServerConfig {
    /// An IP address, not a hostname: a name would have to be resolved,
    /// and which address it resolves to is not the configuration's to
    /// decide. `utoipa` has no schema for `IpAddr`, so it is described
    /// here as the string it is written as.
    #[schema(value_type = String, examples("127.0.0.1"))]
    pub host: IpAddr,
    /// The TCP port proxied traffic arrives on. Must differ from
    /// `admin.port`.
    pub port: super::Port,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    /// The lowest level that is logged. `RUST_LOG` overrides it when set and
    /// non-empty.
    #[serde(default = "default_level")]
    pub level: LogLevel,
    /// `json` for machines, `text` for a terminal.
    #[serde(default = "default_format")]
    pub format: LogFormat,
}

fn default_level() -> LogLevel {
    LogLevel::Info
}

fn default_format() -> LogFormat {
    LogFormat::Json
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_level(),
            format: default_format(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    /// Path to the control socket, created with mode 0600 and removed on
    /// shutdown. Its parent directory must already exist.
    #[serde(default = "default_socket")]
    /// A filesystem path. `utoipa` has no schema for `PathBuf`, so it is
    /// described as the string it is written as.
    #[schema(value_type = String, examples("/tmp/doppel.sock"))]
    pub socket: PathBuf,
}

fn default_socket() -> PathBuf {
    PathBuf::from("/tmp/doppel.sock")
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            socket: default_socket(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplatesConfig {
    /// Directory holding mock templates, one subdirectory per proxy. Created
    /// at startup if absent.
    #[serde(default = "default_templates_dir")]
    /// A filesystem path. `utoipa` has no schema for `PathBuf`, so it is
    /// described as the string it is written as.
    #[schema(value_type = String, examples("./templates"))]
    pub dir: PathBuf,
}

fn default_templates_dir() -> PathBuf {
    PathBuf::from("./templates")
}

impl Default for TemplatesConfig {
    fn default() -> Self {
        Self {
            dir: default_templates_dir(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SentryConfig {
    /// The Sentry DSN to report to. Empty disables reporting, so a deployment
    /// can blank it without removing the section.
    #[schema(examples("https://key@o0.ingest.sentry.io/0"))]
    pub dsn: String,
}
