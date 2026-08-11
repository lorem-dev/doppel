//! Reading a configuration back out of the five tables.

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
        // One transaction at `REPEATABLE READ`, so every statement below sees
        // the same snapshot.
        //
        // Without it each query took its own connection and its own snapshot,
        // and a `save` committing in between produced a configuration
        // assembled from two of them -- caught by the revision check at the
        // end of this function, but reported as "the rows and the revision
        // column have diverged", which accuses the database of corruption
        // for what is ordinary concurrency. Measured at 37 failures in 600
        // loads against one concurrent writer before this changed.
        //
        // Read-only, so there is nothing to commit and no lock to hold: the
        // snapshot is the whole point.
        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|err| StoreError::Unavailable(format!("cannot begin: {err}")))?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(query_failed)?;

        let row = sqlx::query("SELECT * FROM configurations WHERE name = $1")
            .bind(&self.config_name)
            .fetch_optional(&mut *tx)
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
            admin: self.admin_from(&mut tx, &row).await?,
            proxies: self.proxies(&mut tx).await?,
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

    async fn admin_from(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        row: &PgRow,
    ) -> Result<AdminConfig, StoreError> {
        let tokens = sqlx::query(
            "SELECT name, \"group\", token FROM admin_tokens WHERE config = $1 ORDER BY ordinal",
        )
        .bind(&self.config_name)
        .fetch_all(&mut **tx)
        .await
        .map_err(query_failed)?
        .into_iter()
        .map(|token| {
            Ok(TokenConfig {
                name: name(&token, "name")?,
                group: name(&token, "group")?,
                token: stored_token(&token, "token")?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;

        Ok(AdminConfig {
            enable: row.try_get("admin_enable").map_err(query_failed)?,
            host: parse_host(&text(row, "admin_host")?)?,
            port: port(row, "admin_port")?,
            auth: AuthConfig {
                header: header_name(row, "admin_auth_header")?,
            },
            tokens,
            access: json_column::<AccessConfig>(row, "admin_access")?,
            upload: UploadConfig {
                limit: byte_size(row, "admin_upload_limit")?,
            },
        })
    }

    async fn proxies(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<Vec<ProxyConfig>, StoreError> {
        let rows = sqlx::query("SELECT * FROM proxies WHERE config = $1 ORDER BY ordinal")
            .bind(&self.config_name)
            .fetch_all(&mut **tx)
            .await
            .map_err(query_failed)?;

        let mut proxies = Vec::with_capacity(rows.len());
        for row in &rows {
            let name = name(row, "name")?;
            proxies.push(ProxyConfig {
                mocks: self.mocks(tx, name.as_str()).await?,
                name,
                kind: match text(row, "kind")?.as_str() {
                    "http" => ProxyKind::Http,
                    // Including `tcp`, which earlier versions of this schema
                    // could store: the variant is gone, so a row holding it is
                    // a configuration this binary cannot serve, reported rather
                    // than silently coerced to `http`.
                    other => return Err(corrupt("proxies.kind", &format!("is `{other}`"))),
                },
                url: url(row, "url")?,
                timeout: timeout(row, "timeout_seconds")?,
                resolve: ResolveConfig {
                    kind: match text(row, "resolve_kind")?.as_str() {
                        "default" => ResolveKind::Default,
                        "header" => ResolveKind::Header,
                        other => {
                            return Err(corrupt("proxies.resolve_kind", &format!("is `{other}`")));
                        }
                    },
                    header: optional_header_name(row, "resolve_header")?,
                },
                access: optional_json::<ProxyAccessConfig>(row, "access")?,
                headers: json_column(row, "headers")?,
                loss: loss_from(row)?,
                latency: latency_from(row)?,
                replace: optional_ratio(row, "replace_ratio")?,
                rewrite_redirects: row.try_get("rewrite_redirects").map_err(query_failed)?,
                body_limit: byte_size(row, "body_limit")?,
            });
        }
        Ok(proxies)
    }

    async fn mocks(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        proxy: &str,
    ) -> Result<Vec<MockConfig>, StoreError> {
        sqlx::query("SELECT * FROM mocks WHERE config = $1 AND proxy = $2 ORDER BY ordinal")
            .bind(&self.config_name)
            .bind(proxy)
            .fetch_all(&mut **tx)
            .await
            .map_err(query_failed)?
            .iter()
            .map(|row| {
                Ok(MockConfig {
                    name: name(row, "name")?,
                    request: MockRequest {
                        method: method(row, "method")?,
                        url: pattern(row, "url_pattern")?,
                        headers: json_column(row, "request_headers")?,
                        query: json_column(row, "request_query")?,
                        body: json_column(row, "request_body")?,
                    },
                    response: MockResponse {
                        status: status(row, "status")?,
                        body: row.try_get("body").map_err(query_failed)?,
                        json: row.try_get("json").map_err(query_failed)?,
                        template: optional_template_name(row, "template")?,
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
            percentage: ratio(percentage, "proxies.loss_percentage")?,
            status: narrow_status(status, "proxies.loss_status")?,
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
            percentage: ratio(percentage, "proxies.latency_percentage")?,
            min: seconds(min, "proxies.latency_min")?,
            max: seconds(max, "proxies.latency_max")?,
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

/// A stored name, checked on the way in.
///
/// The database has no opinion about what a name may contain, so a row edited
/// by hand can hold one the configuration format would refuse. Parsing here
/// means a `Config` this store produces is subject to the same rule as one
/// read from YAML, rather than a second, laxer standard nobody wrote down.
/// Generic over the cap so one function serves both a `Name` and the tighter
/// `ProxyName`, and the column is checked against the limit that type actually
/// carries rather than against whichever one this helper happened to name.
fn name<const MAX_LEN: usize>(
    row: &PgRow,
    column: &str,
) -> Result<doppel_core::config::Name<MAX_LEN>, StoreError> {
    let raw: String = row.try_get(column).map_err(query_failed)?;
    doppel_core::config::Name::parse(raw).map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored token, checked on the way in, for the same reason as `name`.
///
/// The error names the column and not the value: a corruption message about a
/// token is still somewhere a token would end up.
fn stored_token(row: &PgRow, column: &str) -> Result<doppel_core::config::Token, StoreError> {
    let raw: String = row.try_get(column).map_err(query_failed)?;
    doppel_core::config::Token::parse(raw).map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored port, checked on the way in.
///
/// `i32` is the column type, so the range check has two halves: the value has
/// to fit a `u16` at all, and then it has to be one `Port` accepts. Both are
/// reachable from a hand-edited row, and neither is reachable from YAML.
fn port(row: &PgRow, column: &str) -> Result<doppel_core::config::Port, StoreError> {
    let value: i32 = row.try_get(column).map_err(query_failed)?;
    let narrowed =
        u16::try_from(value).map_err(|_| corrupt(column, &format!("is {value}, not a port")))?;
    doppel_core::config::Port::parse(narrowed).map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored status, checked on the way in.
///
/// Two halves, as with `port`: the `i32` column has to narrow to a `u16` at
/// all, and then the number has to be one `HttpStatus` accepts. Both are
/// reachable from a hand-edited row and neither is reachable from YAML.
fn status(row: &PgRow, column: &str) -> Result<doppel_core::config::HttpStatus, StoreError> {
    let value: i32 = row.try_get(column).map_err(query_failed)?;
    narrow_status(value, column)
}

fn narrow_status(value: i32, column: &str) -> Result<doppel_core::config::HttpStatus, StoreError> {
    let narrowed =
        u16::try_from(value).map_err(|_| corrupt(column, &format!("is {value}, not a status")))?;
    doppel_core::config::HttpStatus::parse(narrowed)
        .map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored method, checked on the way in.
fn method(row: &PgRow, column: &str) -> Result<doppel_core::config::HttpMethod, StoreError> {
    let raw: String = row.try_get(column).map_err(query_failed)?;
    raw.parse()
        .map_err(|err: doppel_core::config::MethodError| corrupt(column, &err.to_string()))
}

/// A stored timeout, checked on the way in.
///
/// The column is a nullable `bigint`, so both halves of the range are
/// reachable from a hand-edited row: a negative value, and a positive one
/// past what a timeout may be.
fn timeout(
    row: &PgRow,
    column: &str,
) -> Result<Option<doppel_core::config::TimeoutSeconds>, StoreError> {
    optional_u64(row, column)?
        .map(|value| {
            doppel_core::config::TimeoutSeconds::parse(value)
                .map_err(|err| corrupt(column, &err.to_string()))
        })
        .transpose()
}

/// A stored probability, checked on the way in.
fn ratio(value: f64, column: &str) -> Result<doppel_core::config::Ratio, StoreError> {
    doppel_core::config::Ratio::parse(value).map_err(|err| corrupt(column, &err.to_string()))
}

fn optional_ratio(
    row: &PgRow,
    column: &str,
) -> Result<Option<doppel_core::config::Ratio>, StoreError> {
    let value: Option<f64> = row.try_get(column).map_err(query_failed)?;
    value.map(|value| ratio(value, column)).transpose()
}

/// A stored latency, checked on the way in.
fn seconds(value: f64, column: &str) -> Result<doppel_core::config::Seconds, StoreError> {
    doppel_core::config::Seconds::parse(value).map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored template file name, checked on the way in.
fn optional_template_name(
    row: &PgRow,
    column: &str,
) -> Result<Option<doppel_core::config::TemplateName>, StoreError> {
    let raw: Option<String> = row.try_get(column).map_err(query_failed)?;
    raw.map(|raw| {
        doppel_core::config::TemplateName::parse(raw)
            .map_err(|err| corrupt(column, &err.to_string()))
    })
    .transpose()
}

/// A stored url pattern, checked on the way in.
fn pattern(row: &PgRow, column: &str) -> Result<doppel_core::config::Pattern, StoreError> {
    let raw: String = row.try_get(column).map_err(query_failed)?;
    doppel_core::config::Pattern::parse(raw).map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored header name, checked on the way in.
fn header_name(row: &PgRow, column: &str) -> Result<doppel_core::config::HeaderName, StoreError> {
    let raw: String = row.try_get(column).map_err(query_failed)?;
    doppel_core::config::HeaderName::parse(raw).map_err(|err| corrupt(column, &err.to_string()))
}

fn optional_header_name(
    row: &PgRow,
    column: &str,
) -> Result<Option<doppel_core::config::HeaderName>, StoreError> {
    let raw: Option<String> = row.try_get(column).map_err(query_failed)?;
    raw.map(|raw| {
        doppel_core::config::HeaderName::parse(raw).map_err(|err| corrupt(column, &err.to_string()))
    })
    .transpose()
}

/// A stored upstream URL, checked on the way in.
fn url(row: &PgRow, column: &str) -> Result<doppel_core::config::UpstreamUrl, StoreError> {
    let raw: String = row.try_get(column).map_err(query_failed)?;
    doppel_core::config::UpstreamUrl::parse(&raw).map_err(|err| corrupt(column, &err.to_string()))
}

/// A stored byte limit, checked on the way in.
fn byte_size(row: &PgRow, column: &str) -> Result<ByteSize, StoreError> {
    let value: i64 = row.try_get(column).map_err(query_failed)?;
    let unsigned = u64::try_from(value).map_err(|_| corrupt(column, "is negative"))?;
    ByteSize::parse(unsigned).map_err(|err| corrupt(column, &err.to_string()))
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
