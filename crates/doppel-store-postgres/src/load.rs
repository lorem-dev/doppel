//! Reading a configuration back out of the five tables.

use std::collections::BTreeMap;

use doppel_core::config::{
    AccessConfig, AdminConfig, AuthConfig, ByteSize, Config, ControlConfig, LatencyConfig,
    LogFormat, LogLevel, LoggingConfig, LossConfig, MockConfig, MockProxyOverride, MockRequest,
    MockResponse, ProxyAccessConfig, ProxyConfig, ProxyKind, ResolveConfig, ResolveKind,
    SentryConfig, ServerConfig, TemplatesConfig, TokenConfig, UploadConfig,
};
use doppel_core::store::{Revision, StoreError};
use sqlx::{Row, postgres::PgRow};

use crate::PostgresStore;

impl PostgresStore {
    /// Read this store's configuration.
    ///
    /// Public rather than crate-private because the `ConfigStore` impl that
    /// will delegate to it lands with `save` (task 5), and a method nothing
    /// can call is a method nothing tests.
    pub async fn load_config(&self) -> Result<(Config, Revision), StoreError> {
        let row = sqlx::query("SELECT * FROM configurations WHERE name = $1")
            .bind(&self.config_name)
            .fetch_optional(self.pool())
            .await
            .map_err(query_failed)?
            .ok_or_else(|| StoreError::NotFound(self.config_name.clone().into()))?;

        let stored_revision = Revision(u64::from_ne_bytes(
            row.try_get::<i64, _>("revision")
                .map_err(query_failed)?
                .to_ne_bytes(),
        ));

        let config = Config {
            server: server_from(&row)?,
            logging: logging_from(&row)?,
            control: control_from(&row)?,
            templates: templates_from(&row)?,
            sentry: sentry_from(&row)?,
            admin: self.admin_from(&row).await?,
            proxies: self.proxies().await?,
        };

        // The stored revision has to agree with the content it labels. They
        // diverge when a row was edited by hand, or when this code forgot to
        // read a column that `save` wrote -- and either way every
        // compare-and-swap downstream would then fail in a way that looks
        // like contention rather than like corruption.
        let computed = Revision::of_config(&config);
        if computed != stored_revision {
            return Err(StoreError::Invalid(vec![doppel_core::Violation::new(
                "",
                format!(
                    "the stored revision ({stored_revision}) does not match the configuration \
                     it labels ({computed}); the rows and the revision column have diverged"
                ),
            )]));
        }

        Ok((config, stored_revision))
    }

    async fn admin_from(&self, row: &PgRow) -> Result<AdminConfig, StoreError> {
        let tokens = sqlx::query(
            "SELECT name, \"group\", token FROM admin_tokens WHERE config = $1 ORDER BY ordinal",
        )
        .bind(&self.config_name)
        .fetch_all(self.pool())
        .await
        .map_err(query_failed)?
        .into_iter()
        .map(|token| {
            Ok(TokenConfig {
                name: text(&token, "name")?,
                group: text(&token, "group")?,
                token: text(&token, "token")?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;

        Ok(AdminConfig {
            enable: row.try_get("admin_enable").map_err(query_failed)?,
            host: parse_host(&text(row, "admin_host")?)?,
            port: port(row, "admin_port")?,
            auth: AuthConfig {
                header: text(row, "admin_auth_header")?,
            },
            tokens,
            access: json_column::<AccessConfig>(row, "admin_access")?,
            upload: UploadConfig {
                limit: ByteSize(
                    u64::try_from(
                        row.try_get::<i64, _>("admin_upload_limit")
                            .map_err(query_failed)?,
                    )
                    .map_err(|_| corrupt("admin_upload_limit", "is negative"))?,
                ),
            },
        })
    }

    async fn proxies(&self) -> Result<Vec<ProxyConfig>, StoreError> {
        let rows = sqlx::query("SELECT * FROM proxies WHERE config = $1 ORDER BY ordinal")
            .bind(&self.config_name)
            .fetch_all(self.pool())
            .await
            .map_err(query_failed)?;

        let mut proxies = Vec::with_capacity(rows.len());
        for row in &rows {
            let name = text(row, "name")?;
            proxies.push(ProxyConfig {
                mocks: self.mocks(&name).await?,
                name,
                kind: match text(row, "kind")?.as_str() {
                    "http" => ProxyKind::Http,
                    "tcp" => ProxyKind::Tcp,
                    other => return Err(corrupt("proxies.kind", &format!("is `{other}`"))),
                },
                url: text(row, "url")?,
                timeout: optional_u64(row, "timeout_seconds")?,
                resolve: ResolveConfig {
                    kind: match text(row, "resolve_kind")?.as_str() {
                        "default" => ResolveKind::Default,
                        "header" => ResolveKind::Header,
                        other => {
                            return Err(corrupt("proxies.resolve_kind", &format!("is `{other}`")));
                        }
                    },
                    header: row.try_get("resolve_header").map_err(query_failed)?,
                },
                access: optional_json::<ProxyAccessConfig>(row, "access")?,
                headers: json_column::<BTreeMap<String, String>>(row, "headers")?,
                loss: loss_from(row)?,
                latency: latency_from(row)?,
                replace: row.try_get("replace_ratio").map_err(query_failed)?,
                body_limit: ByteSize(
                    u64::try_from(row.try_get::<i64, _>("body_limit").map_err(query_failed)?)
                        .map_err(|_| corrupt("proxies.body_limit", "is negative"))?,
                ),
            });
        }
        Ok(proxies)
    }

    async fn mocks(&self, proxy: &str) -> Result<Vec<MockConfig>, StoreError> {
        sqlx::query("SELECT * FROM mocks WHERE config = $1 AND proxy = $2 ORDER BY ordinal")
            .bind(&self.config_name)
            .bind(proxy)
            .fetch_all(self.pool())
            .await
            .map_err(query_failed)?
            .iter()
            .map(|row| {
                Ok(MockConfig {
                    name: text(row, "name")?,
                    request: MockRequest {
                        method: text(row, "method")?,
                        url: text(row, "url_pattern")?,
                        headers: json_column(row, "request_headers")?,
                        query: json_column(row, "request_query")?,
                        body: json_column(row, "request_body")?,
                    },
                    response: MockResponse {
                        status: status(row, "status")?,
                        body: row.try_get("body").map_err(query_failed)?,
                        json: row.try_get("json").map_err(query_failed)?,
                        template: row.try_get("template").map_err(query_failed)?,
                        headers: json_column(row, "response_headers")?,
                    },
                    proxy: optional_json::<MockProxyOverride>(row, "proxy_override")?,
                })
            })
            .collect()
    }
}

fn server_from(row: &PgRow) -> Result<ServerConfig, StoreError> {
    Ok(ServerConfig {
        host: parse_host(&text(row, "server_host")?)?,
        port: port(row, "server_port")?,
    })
}

fn logging_from(row: &PgRow) -> Result<LoggingConfig, StoreError> {
    // Through serde rather than a hand-written match: the string in the
    // column is the one the YAML uses, and going through the same
    // deserializer keeps the two spellings from drifting apart.
    Ok(LoggingConfig {
        level: enum_from_text::<LogLevel>("log_level", &text(row, "log_level")?)?,
        format: enum_from_text::<LogFormat>("log_format", &text(row, "log_format")?)?,
    })
}

fn control_from(row: &PgRow) -> Result<ControlConfig, StoreError> {
    Ok(ControlConfig {
        socket: text(row, "control_socket")?.into(),
    })
}

fn templates_from(row: &PgRow) -> Result<TemplatesConfig, StoreError> {
    Ok(TemplatesConfig {
        dir: text(row, "templates_dir")?.into(),
    })
}

/// An absent `sentry_dsn` and an empty one both mean the section is absent.
///
/// `Some(SentryConfig { dsn: String::new() })` would round-trip as
/// `sentry:\n  dsn: ""`, which is a different document from one with no
/// `sentry` section -- and a different revision.
fn sentry_from(row: &PgRow) -> Result<Option<SentryConfig>, StoreError> {
    let dsn: Option<String> = row.try_get("sentry_dsn").map_err(query_failed)?;
    Ok(dsn
        .filter(|dsn| !dsn.is_empty())
        .map(|dsn| SentryConfig { dsn }))
}

/// `loss` is present exactly when both its columns are.
///
/// One of the two set on its own is a row nobody could have written through
/// `save`, so it is reported rather than silently treated as absent.
fn loss_from(row: &PgRow) -> Result<Option<LossConfig>, StoreError> {
    let percentage: Option<f64> = row.try_get("loss_percentage").map_err(query_failed)?;
    let status: Option<i32> = row.try_get("loss_status").map_err(query_failed)?;
    match (percentage, status) {
        (None, None) => Ok(None),
        (Some(percentage), Some(status)) => Ok(Some(LossConfig {
            percentage,
            status: u16::try_from(status)
                .map_err(|_| corrupt("proxies.loss_status", "is not a status"))?,
        })),
        _ => Err(corrupt(
            "proxies.loss_*",
            "has some columns set and others null",
        )),
    }
}

fn latency_from(row: &PgRow) -> Result<Option<LatencyConfig>, StoreError> {
    let percentage: Option<f64> = row.try_get("latency_percentage").map_err(query_failed)?;
    let min: Option<f64> = row.try_get("latency_min").map_err(query_failed)?;
    let max: Option<f64> = row.try_get("latency_max").map_err(query_failed)?;
    match (percentage, min, max) {
        (None, None, None) => Ok(None),
        (Some(percentage), Some(min), Some(max)) => Ok(Some(LatencyConfig {
            percentage,
            min,
            max,
        })),
        _ => Err(corrupt(
            "proxies.latency_*",
            "has some columns set and others null",
        )),
    }
}

fn text(row: &PgRow, column: &str) -> Result<String, StoreError> {
    row.try_get(column).map_err(query_failed)
}

fn port(row: &PgRow, column: &str) -> Result<u16, StoreError> {
    let value: i32 = row.try_get(column).map_err(query_failed)?;
    u16::try_from(value).map_err(|_| corrupt(column, &format!("is {value}, not a port")))
}

fn status(row: &PgRow, column: &str) -> Result<u16, StoreError> {
    let value: i32 = row.try_get(column).map_err(query_failed)?;
    u16::try_from(value).map_err(|_| corrupt(column, &format!("is {value}, not a status")))
}

fn optional_u64(row: &PgRow, column: &str) -> Result<Option<u64>, StoreError> {
    let value: Option<i64> = row.try_get(column).map_err(query_failed)?;
    value
        .map(|value| u64::try_from(value).map_err(|_| corrupt(column, "is negative")))
        .transpose()
}

fn json_column<T: serde::de::DeserializeOwned>(row: &PgRow, column: &str) -> Result<T, StoreError> {
    let value: serde_json::Value = row.try_get(column).map_err(query_failed)?;
    serde_json::from_value(value).map_err(|err| corrupt(column, &err.to_string()))
}

fn optional_json<T: serde::de::DeserializeOwned>(
    row: &PgRow,
    column: &str,
) -> Result<Option<T>, StoreError> {
    let value: Option<serde_json::Value> = row.try_get(column).map_err(query_failed)?;
    value
        .map(|value| serde_json::from_value(value).map_err(|err| corrupt(column, &err.to_string())))
        .transpose()
}

/// Deserialize an enum from the same text its YAML representation uses.
fn enum_from_text<T: serde::de::DeserializeOwned>(
    column: &str,
    value: &str,
) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|_| corrupt(column, &format!("is `{value}`")))
}

fn parse_host(value: &str) -> Result<std::net::IpAddr, StoreError> {
    value
        .parse()
        .map_err(|_| corrupt("host", &format!("`{value}` is not an IP address")))
}

fn query_failed(err: sqlx::Error) -> StoreError {
    StoreError::Unavailable(format!("query failed: {err}"))
}

/// A row that cannot be turned into a configuration.
///
/// Reported as `Invalid` rather than `Unavailable`: the database answered
/// perfectly well, and what it holds is the problem.
fn corrupt(column: &str, detail: &str) -> StoreError {
    StoreError::Invalid(vec![doppel_core::Violation::new(
        column,
        format!("stored value {detail}"),
    )])
}
