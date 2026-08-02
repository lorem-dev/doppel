//! Server, logging, control channel, template and Sentry settings.

use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// No `workers` here. It sizes the tokio runtime, and a database-backed
/// store cannot be opened before that runtime exists -- so the value has to
/// be known before the configuration is read, which puts it on the same side
/// of the boundary as the connection settings. It is `--workers` /
/// `DOPPEL_WORKERS`.
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_level")]
    pub level: LogLevel,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    #[serde(default = "default_socket")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplatesConfig {
    #[serde(default = "default_templates_dir")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SentryConfig {
    pub dsn: String,
}
