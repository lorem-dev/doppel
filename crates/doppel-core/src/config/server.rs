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
    /// Where clients reach this Doppel, when that is not `host:port`.
    ///
    /// Behind a container port mapping, a load balancer or an ingress, the
    /// address Doppel bound is not the address a client used -- and a rewritten
    /// `Location` has to name the second. Doppel cannot infer it: `Host` is a
    /// claim by the caller, and building a redirect out of it hands the caller
    /// the redirect.
    ///
    /// Absent, `host` and `port` are used, with a wildcard bind (`0.0.0.0`, `::`)
    /// read as loopback -- which is right for a laptop or a pod reached at its
    /// own address, and wrong behind a port mapping or an ingress, where the
    /// client used neither. Doppel logs the address it settled on at startup.
    ///
    /// `DOPPEL_EXTERNAL_URL` overrides it. Part of `server`, so a change takes a
    /// restart like the rest of that section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_url: Option<super::ExternalUrl>,
}

impl ServerConfig {
    /// Where a client reaches this Doppel, as far as the configuration knows.
    ///
    /// `external_url` when it is set, and `http://host:port/` when it is not --
    /// which is right for a Doppel bound to an address a client can dial, and is
    /// most of them: a laptop on `127.0.0.1`, a pod on its own address.
    ///
    /// A wildcard bind (`0.0.0.0`, `::`) becomes loopback: `0.0.0.0` is every
    /// address this host has, so it names none of them, and
    /// `http://0.0.0.0:8080/` in a `Location` is a URL that fails or means
    /// something else depending on the client. `127.0.0.1` is the address that
    /// is right for the case a wildcard bind is usually about -- a container or
    /// a laptop reached from the same machine.
    ///
    /// It is a guess, and the one place this can be wrong: a port mapping or an
    /// ingress means the client used neither this address nor this port. That
    /// deployment says where it is reached with `external_url` or
    /// `DOPPEL_EXTERNAL_URL`, which is why both exist.
    #[must_use]
    pub fn public_url(&self) -> Option<super::ExternalUrl> {
        if let Some(configured) = &self.external_url {
            return Some(configured.clone());
        }
        // Brackets around an IPv6 literal: `http://::1:8080/` does not parse,
        // and the port would read as part of the address.
        let host = match self.host {
            IpAddr::V4(address) if address.is_unspecified() => "127.0.0.1".to_owned(),
            IpAddr::V6(address) if address.is_unspecified() => "[::1]".to_owned(),
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("[{address}]"),
        };
        super::ExternalUrl::parse(&format!("http://{host}:{}/", self.port.get())).ok()
    }
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
    ///
    /// `DOPPEL_SENTRY_DSN` overrides it. A DSN carries the key that authorises
    /// sending events, so a deployment that keeps credentials in the environment
    /// can leave this field out entirely; an empty variable counts as unset and
    /// leaves this value in force.
    #[schema(examples("https://key@o0.ingest.sentry.io/0"))]
    pub dsn: String,
}

#[cfg(test)]
mod public_url_tests {
    use super::*;
    use crate::config::load_from_str;

    fn config_with(server: &str) -> ServerConfig {
        let text = format!(
            r#"
server:
{server}
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {{}}
  upload:
    limit: 1Mi
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#
        );
        load_from_str(&text).expect("fixture parses").server
    }

    #[test]
    fn a_dialable_host_names_itself() {
        let server = config_with("  host: \"127.0.0.1\"\n  port: 8080");
        assert_eq!(
            server.public_url().map(|url| url.as_str().to_owned()),
            Some("http://127.0.0.1:8080/".to_owned())
        );
    }

    #[test]
    fn an_ipv6_host_is_bracketed() {
        // `http://::1:8080/` does not parse, and the port reads as part of the
        // address. Without the brackets this is a URL nobody can follow.
        let server = config_with("  host: \"::1\"\n  port: 8080");
        assert_eq!(
            server.public_url().map(|url| url.as_str().to_owned()),
            Some("http://[::1]:8080/".to_owned())
        );
    }

    #[test]
    fn a_wildcard_bind_becomes_loopback() {
        // `0.0.0.0` is every address this host has, so it names none of them and
        // cannot go in a `Location`. Loopback is the address that is right for
        // what a wildcard bind is usually about, and a deployment reached
        // anywhere else says so with `external_url`.
        for (host, expected) in [
            ("0.0.0.0", "http://127.0.0.1:8080/"),
            ("::", "http://[::1]:8080/"),
        ] {
            let server = config_with(&format!("  host: \"{host}\"\n  port: 8080"));
            assert_eq!(
                server.public_url().map(|url| url.as_str().to_owned()),
                Some(expected.to_owned()),
                "{host}"
            );
        }
    }

    #[test]
    fn external_url_wins_over_the_bind_address() {
        let server = config_with(
            "  host: \"127.0.0.1\"\n  port: 8080\n  external_url: \"https://doppel.example.com/\"",
        );
        assert_eq!(
            server.public_url().map(|url| url.as_str().to_owned()),
            Some("https://doppel.example.com/".to_owned())
        );
    }
}
