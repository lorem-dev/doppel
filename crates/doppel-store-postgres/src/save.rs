//! Writing a configuration into the five tables.

use doppel_core::config::{Config, ProxyConfig};
use doppel_core::store::{Revision, StoreError};
use doppel_core::validate::validate;
use sqlx::{Postgres, Transaction};

use crate::PostgresStore;

impl PostgresStore {
    /// Validate and write, as one transaction, under compare-and-swap.
    ///
    /// `expected: Some(rev)` means the caller built this change from revision
    /// `rev`; the conditional `UPDATE` below writes nothing and the whole
    /// transaction rolls back if the stored revision has moved. `None` is an
    /// unconditional upsert, for first-time provisioning and `config push`.
    ///
    /// The database serialises the transaction, so this needs no analogue of
    /// the advisory file lock `FileStore` has to take.
    pub async fn save_config(
        &self,
        config: &Config,
        expected: Option<Revision>,
    ) -> Result<Revision, StoreError> {
        validate(config).map_err(StoreError::Invalid)?;
        let revision = Revision::of_config(config);

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|err| StoreError::Unavailable(format!("cannot begin: {err}")))?;

        match expected {
            Some(expected) => self.update_header(&mut tx, config, revision, expected).await?,
            None => self.upsert_header(&mut tx, config, revision).await?,
        }

        // Deleted and rewritten rather than diffed. A diff is more code whose
        // failure mode is a row it missed -- a configuration silently
        // disagreeing with the document that produced it. This is one
        // transaction either way, so the rewrite costs nothing an operator can
        // observe.
        //
        // `mocks` and `admin_tokens` go with their parents through
        // `ON DELETE CASCADE`; `templates` deliberately does not, so it is
        // untouched here. Dropping a proxy's files is a separate decision made
        // after the configuration write, never as part of it.
        self.clear_children(&mut tx).await?;

        for (ordinal, token) in config.admin.tokens.iter().enumerate() {
            sqlx::query(
                "INSERT INTO admin_tokens (config, name, \"group\", token, ordinal) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&self.config_name)
            .bind(&token.name)
            .bind(&token.group)
            .bind(&token.token)
            .bind(ordinal_of(ordinal)?)
            .execute(&mut *tx)
            .await
            .map_err(write_failed)?;
        }

        for (ordinal, proxy) in config.proxies.iter().enumerate() {
            self.insert_proxy(&mut tx, proxy, ordinal_of(ordinal)?).await?;
            for (mock_ordinal, mock) in proxy.mocks.iter().enumerate() {
                self.insert_mock(&mut tx, &proxy.name, mock, ordinal_of(mock_ordinal)?)
                    .await?;
            }
        }

        tx.commit()
            .await
            .map_err(|err| StoreError::Unavailable(format!("cannot commit: {err}")))?;
        Ok(revision)
    }

    /// The conditional half of the compare-and-swap.
    ///
    /// Zero rows affected means either the revision moved or the
    /// configuration does not exist. The two are distinguished by a follow-up
    /// read, because "you are holding a stale copy" and "there is nothing here
    /// to update" send a caller in different directions.
    async fn update_header(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        config: &Config,
        revision: Revision,
        expected: Revision,
    ) -> Result<(), StoreError> {
        let affected = sqlx::query(&format!(
            "UPDATE configurations SET revision = $1, {} WHERE name = $2 AND revision = $3",
            header_assignments(4)
        ))
        .bind(as_i64(revision))
        .bind(&self.config_name)
        .bind(as_i64(expected))
        .pipe_header(config)
        .execute(&mut **tx)
        .await
        .map_err(write_failed)?
        .rows_affected();

        if affected == 1 {
            return Ok(());
        }

        let actual: Option<i64> = sqlx::query_scalar("SELECT revision FROM configurations WHERE name = $1")
            .bind(&self.config_name)
            .fetch_optional(&mut **tx)
            .await
            .map_err(write_failed)?;

        Err(match actual {
            Some(actual) => StoreError::RevisionMismatch {
                expected,
                actual: from_i64(actual),
            },
            None => StoreError::NotFound(self.config_name.clone().into()),
        })
    }

    async fn upsert_header(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        config: &Config,
        revision: Revision,
    ) -> Result<(), StoreError> {
        sqlx::query(&format!(
            "INSERT INTO configurations (name, revision, server_host, server_port, log_level, \
             log_format, control_socket, templates_dir, sentry_dsn, admin_host, admin_port, \
             admin_auth_header, admin_upload_limit, admin_access) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
             ON CONFLICT (name) DO UPDATE SET revision = $2, {}, updated_at = now()",
            header_assignments(3)
        ))
        .bind(&self.config_name)
        .bind(as_i64(revision))
        .pipe_header(config)
        .execute(&mut **tx)
        .await
        .map_err(write_failed)?;
        Ok(())
    }

    async fn clear_children(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), StoreError> {
        for table in ["mocks", "admin_tokens", "proxies"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE config = $1"))
                .bind(&self.config_name)
                .execute(&mut **tx)
                .await
                .map_err(write_failed)?;
        }
        Ok(())
    }

    async fn insert_proxy(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        proxy: &ProxyConfig,
        ordinal: i32,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO proxies (config, name, ordinal, kind, url, timeout_seconds, body_limit, \
             replace_ratio, resolve_kind, resolve_header, loss_percentage, loss_status, \
             latency_percentage, latency_min, latency_max, headers, access) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
        )
        .bind(&self.config_name)
        .bind(&proxy.name)
        .bind(ordinal)
        .bind(as_text(&proxy.kind)?)
        .bind(&proxy.url)
        .bind(proxy.timeout.map(|t| i64::try_from(t).unwrap_or(i64::MAX)))
        .bind(i64::try_from(proxy.body_limit.0).unwrap_or(i64::MAX))
        .bind(proxy.replace)
        .bind(as_text(&proxy.resolve.kind)?)
        .bind(proxy.resolve.header.as_deref())
        .bind(proxy.loss.as_ref().map(|l| l.percentage))
        .bind(proxy.loss.as_ref().map(|l| i32::from(l.status)))
        .bind(proxy.latency.as_ref().map(|l| l.percentage))
        .bind(proxy.latency.as_ref().map(|l| l.min))
        .bind(proxy.latency.as_ref().map(|l| l.max))
        .bind(as_json(&proxy.headers)?)
        .bind(proxy.access.as_ref().map(as_json).transpose()?)
        .execute(&mut **tx)
        .await
        .map_err(write_failed)?;
        Ok(())
    }

    async fn insert_mock(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        proxy: &str,
        mock: &doppel_core::config::MockConfig,
        ordinal: i32,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO mocks (config, proxy, name, ordinal, method, url_pattern, status, body, \
             json, template, request_headers, request_query, request_body, response_headers, \
             proxy_override) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
             $14, $15)",
        )
        .bind(&self.config_name)
        .bind(proxy)
        .bind(&mock.name)
        .bind(ordinal)
        .bind(&mock.request.method)
        .bind(&mock.request.url)
        .bind(i32::from(mock.response.status))
        .bind(mock.response.body.as_deref())
        .bind(mock.response.json.as_deref())
        .bind(mock.response.template.as_deref())
        .bind(as_json(&mock.request.headers)?)
        .bind(as_json(&mock.request.query)?)
        .bind(as_json(&mock.request.body)?)
        .bind(as_json(&mock.response.headers)?)
        .bind(mock.proxy.as_ref().map(as_json).transpose()?)
        .execute(&mut **tx)
        .await
        .map_err(write_failed)?;
        Ok(())
    }
}

/// The header's assignment list, starting at `$n`.
///
/// Written once and used by both the conditional update and the upsert, so the
/// two cannot drift into writing different sets of columns -- which would show
/// up as a configuration that changes depending on whether it was created or
/// updated.
fn header_assignments(first: usize) -> String {
    HEADER_COLUMNS
        .iter()
        .enumerate()
        .map(|(offset, column)| format!("{column} = ${}", first + offset))
        .collect::<Vec<_>>()
        .join(", ")
}

const HEADER_COLUMNS: &[&str] = &[
    "server_host",
    "server_port",
    "log_level",
    "log_format",
    "control_socket",
    "templates_dir",
    "sentry_dsn",
    "admin_host",
    "admin_port",
    "admin_auth_header",
    "admin_upload_limit",
    "admin_access",
];

/// Bind the header values in `HEADER_COLUMNS` order.
///
/// A trait rather than a free function so the call reads in the same place the
/// query does: the order of these binds and the order of that list are the one
/// thing a reader has to check, and putting them side by side is what makes
/// that possible.
trait BindHeader<'q> {
    fn pipe_header(self, config: &'q Config) -> Self;
}

impl<'q> BindHeader<'q>
    for sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>
{
    fn pipe_header(self, config: &'q Config) -> Self {
        self.bind(config.server.host.to_string())
            .bind(i32::from(config.server.port))
            .bind(text_of(&config.logging.level))
            .bind(text_of(&config.logging.format))
            .bind(config.control.socket.to_string_lossy().into_owned())
            .bind(config.templates.dir.to_string_lossy().into_owned())
            .bind(config.sentry.as_ref().map(|s| s.dsn.clone()))
            .bind(config.admin.host.to_string())
            .bind(i32::from(config.admin.port))
            .bind(config.admin.auth.header.clone())
            .bind(i64::try_from(config.admin.upload.limit.0).unwrap_or(i64::MAX))
            .bind(serde_json::to_value(&config.admin.access).unwrap_or(serde_json::Value::Null))
    }
}

/// An enum's wire spelling, which is the one the YAML and the column share.
fn text_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

fn as_text<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| StoreError::Serialize("an enum did not serialize as a string".to_owned()))
}

fn as_json<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(value).map_err(|err| StoreError::Serialize(err.to_string()))
}

/// A list position as the column's type.
///
/// Fails rather than truncating: a configuration with more than two billion
/// proxies is not one anybody wrote, and silently wrapping the ordinal would
/// reorder the document.
fn ordinal_of(index: usize) -> Result<i32, StoreError> {
    i32::try_from(index)
        .map_err(|_| StoreError::Serialize(format!("position {index} does not fit a column")))
}

/// The revision as the column holds it: the same bits, not a numeric
/// conversion. PostgreSQL has no unsigned 64-bit integer, and a value above
/// `i64::MAX` is perfectly ordinary for a hash.
fn as_i64(revision: Revision) -> i64 {
    i64::from_ne_bytes(revision.0.to_ne_bytes())
}

fn from_i64(value: i64) -> Revision {
    Revision(u64::from_ne_bytes(value.to_ne_bytes()))
}

fn write_failed(err: sqlx::Error) -> StoreError {
    StoreError::Unavailable(format!("write failed: {err}"))
}
