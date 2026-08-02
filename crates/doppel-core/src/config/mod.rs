//! The configuration model and its YAML representation.

pub mod admin;
pub mod mock;
pub mod proxy;
pub mod server;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use admin::{
    AccessConfig, AdminConfig, AuthConfig, ByteSize, ProxyAccessConfig, Subjects, TokenConfig,
    UploadConfig,
};
pub use mock::{MockConfig, MockProxyOverride, MockRequest, MockResponse};
pub use proxy::{LatencyConfig, LossConfig, ProxyConfig, ProxyKind, ResolveConfig, ResolveKind};
pub use server::{
    ControlConfig, LogFormat, LogLevel, LoggingConfig, SentryConfig, ServerConfig, TemplatesConfig,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub control: ControlConfig,
    #[serde(default)]
    pub templates: TemplatesConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentry: Option<SentryConfig>,
    pub admin: AdminConfig,
    #[serde(default)]
    pub proxies: Vec<ProxyConfig>,
}

/// Failure to read or parse a config. Validation failures are separate: see
/// `crate::validate::Violation`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    NotFound(PathBuf),
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse config: {0}")]
    Parse(#[from] serde_norway::Error),
}

/// Parse a config from YAML text. Does not validate; call
/// `crate::validate::validate` afterwards.
pub fn load_from_str(text: &str) -> Result<Config, ConfigError> {
    Ok(serde_norway::from_str(text)?)
}

/// Read and parse a config file. A missing file is reported distinctly from a
/// malformed one, because the two need different remedies.
pub fn load_from_path(path: &Path) -> Result<Config, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfigError::NotFound(path.to_path_buf()));
        }
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    load_from_str(&text)
}

/// Serialize a config back to YAML.
pub fn to_yaml(config: &Config) -> Result<String, serde_norway::Error> {
    canonical_yaml(config)
}

/// The single serialization path behind "canonical serialization": both
/// `to_yaml` and `Revision::of_proxy`/`Revision::of_config` go through this
/// rather than each calling `serde_norway::to_string` on their own, so the
/// two can never spell "canonical" differently. Identical to
/// `serde_norway::to_string` today, but the point is that there is exactly
/// one place to change if that ever needs to stop being true.
pub(crate) fn canonical_yaml<T: Serialize>(value: &T) -> Result<String, serde_norway::Error> {
    serde_norway::to_string(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1Mi
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#;

    #[test]
    fn reference_config_loads() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../main.example.yaml"
        ))
        .unwrap();
        let config = load_from_str(&text).unwrap();
        assert_eq!(config.proxies.len(), 2);
        assert_eq!(config.proxies[0].mocks.len(), 6);
        assert_eq!(config.proxies[1].resolve.kind, ResolveKind::Header);
        assert_eq!(
            config.proxies[1].resolve.header.as_deref(),
            Some("X-Proxy-Name")
        );
    }

    #[test]
    fn optional_sections_get_defaults() {
        let config = load_from_str(MINIMAL).unwrap();
        assert_eq!(config.logging.level, LogLevel::Info);
        assert_eq!(config.logging.format, LogFormat::Json);
        assert_eq!(
            config.control.socket,
            std::path::Path::new("/tmp/doppel.sock")
        );
        assert_eq!(config.templates.dir, std::path::Path::new("./templates"));
        assert!(config.sentry.is_none());
        assert_eq!(config.admin.auth.header, "X-Proxy-Authorization");
        assert_eq!(config.proxies[0].resolve.kind, ResolveKind::Default);
        assert_eq!(config.proxies[0].timeout, None);
        assert!(config.proxies[0].mocks.is_empty());
    }

    #[test]
    fn the_admin_listener_is_on_unless_it_is_turned_off() {
        // The default has to be the one nobody wrote, and a proxy with no way
        // to administer it is not what an omitted field should mean.
        assert!(load_from_str(MINIMAL).unwrap().admin.enable);

        let off = MINIMAL.replace("  port: 8081", "  port: 8081\n  enable: false");
        assert!(!load_from_str(&off).unwrap().admin.enable);
    }

    #[test]
    fn server_workers_is_no_longer_a_configuration_field() {
        // It sizes the tokio runtime, and a database store cannot be opened
        // before that runtime exists -- so the value has to be known before
        // the configuration is read. It moved to `--workers`. A document
        // still carrying it is an unknown field, which names it, rather than
        // a field that is silently ignored.
        let text = MINIMAL.replace("port: 8080", "port: 8080\n  workers: 4");
        let err = load_from_str(&text).unwrap_err();
        assert!(
            err.to_string().contains("workers"),
            "the error must name the removed field, got: {err}"
        );
    }

    #[test]
    fn unknown_field_is_rejected() {
        let text = MINIMAL.replace("port: 8080", "port: 8080\n  wrokers: 4");
        let err = load_from_str(&text).unwrap_err();
        assert!(
            err.to_string().contains("wrokers"),
            "error should name the offending key, got: {err}"
        );
    }

    #[test]
    fn subjects_accept_all_three_spellings() {
        assert_eq!(parse_subjects("public"), Subjects::Public);
        assert_eq!(
            parse_subjects("user1"),
            Subjects::Names(vec!["user1".into()])
        );
        assert_eq!(
            parse_subjects(r#"["admin", "user1"]"#),
            Subjects::Names(vec!["admin".into(), "user1".into()])
        );
        assert_eq!(parse_subjects("[]"), Subjects::Public);
    }

    #[test]
    fn tcp_type_deserializes_so_validation_can_reject_it_with_a_good_message() {
        let text = MINIMAL.replace("type: http", "type: tcp");
        let config = load_from_str(&text).unwrap();
        assert_eq!(config.proxies[0].kind, ProxyKind::Tcp);
    }

    #[test]
    fn missing_file_is_distinct_from_malformed_file() {
        let err = load_from_path(std::path::Path::new("/nonexistent/doppel.yaml")).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound(_)));
    }

    #[test]
    fn upload_limit_plain_integer_deserializes() {
        let text = MINIMAL.replace("limit: 1Mi", "limit: 4096");
        let config = load_from_str(&text).unwrap();
        assert_eq!(config.admin.upload.limit.0, 4096);

        let text = MINIMAL.replace("limit: 1Mi", "limit: 0");
        let config = load_from_str(&text).unwrap();
        assert_eq!(config.admin.upload.limit.0, 0);
    }

    #[test]
    fn upload_limit_negative_integer_is_rejected() {
        let text = MINIMAL.replace("limit: 1Mi", "limit: -1");
        let err = load_from_str(&text);
        assert!(err.is_err());
    }

    #[test]
    fn body_limit_defaults_to_one_mebibyte_when_absent() {
        let config = load_from_str(MINIMAL).unwrap();
        assert_eq!(config.proxies[0].body_limit.0, 1024 * 1024);
    }

    #[test]
    fn body_limit_written_as_512k_parses_through_byte_size() {
        let text = MINIMAL.replace(
            "    url: \"https://example.com/\"",
            "    url: \"https://example.com/\"\n    body_limit: 512Ki",
        );
        let config = load_from_str(&text).unwrap();
        assert_eq!(config.proxies[0].body_limit.0, 512 * 1024);
    }

    fn parse_subjects(yaml: &str) -> Subjects {
        serde_norway::from_str(yaml).unwrap()
    }

    #[test]
    fn subjects_serialization_agrees_with_deserialization_for_the_public_shapes() {
        // `Names(vec![])` and a `Names` list containing `"public"` both parse
        // back as `Public` (see `subjects_accept_all_three_spellings` above),
        // so serializing either of those two shapes must also produce
        // `"public"` -- otherwise a round trip through YAML would silently
        // change what a config means.
        for names in [Vec::new(), vec!["public".to_owned()]] {
            let yaml = serde_norway::to_string(&Subjects::Names(names.clone())).unwrap();
            assert_eq!(
                yaml.trim(),
                "public",
                "Names({names:?}) must serialize as `public`"
            );
        }
    }
}
