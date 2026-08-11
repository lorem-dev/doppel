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
            Some(expected) => {
                self.update_header(&mut tx, config, revision, expected)
                    .await?
            }
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
            .bind(token.name.as_str())
            .bind(token.group.as_str())
            .bind(token.token.as_str())
            .bind(ordinal_of(ordinal)?)
            .execute(&mut *tx)
            .await
            .map_err(write_failed)?;
        }

        for (ordinal, proxy) in config.proxies.iter().enumerate() {
            self.insert_proxy(&mut tx, proxy, ordinal_of(ordinal)?)
                .await?;
            for (mock_ordinal, mock) in proxy.mocks.iter().enumerate() {
                self.insert_mock(
                    &mut tx,
                    proxy.name.as_str(),
                    mock,
                    ordinal_of(mock_ordinal)?,
                )
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
        let affected = sqlx::query(UPDATE_HEADER)
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

        let actual: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM configurations WHERE name = $1")
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
        sqlx::query(UPSERT_HEADER)
            .bind(&self.config_name)
            .bind(as_i64(revision))
            .pipe_header(config)
            .execute(&mut **tx)
            .await
            .map_err(write_failed)?;
        Ok(())
    }

    async fn clear_children(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), StoreError> {
        // Written out rather than looped over a table name, so no SQL here is
        // built by formatting. Order matters only in that `proxies` comes
        // last: `mocks` cascades from it, and deleting the parent first would
        // make the explicit child delete a no-op that reads as if it did
        // something.
        for statement in [
            "DELETE FROM mocks WHERE config = $1",
            "DELETE FROM admin_tokens WHERE config = $1",
            "DELETE FROM proxies WHERE config = $1",
        ] {
            sqlx::query(statement)
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
             replace_ratio, rewrite_redirects, resolve_kind, resolve_header, loss_percentage, \
             loss_status, latency_percentage, latency_min, latency_max, headers, access) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
             $18)",
        )
        .bind(&self.config_name)
        .bind(proxy.name.as_str())
        .bind(ordinal)
        .bind(as_text(&proxy.kind)?)
        .bind(proxy.url.as_str())
        .bind(
            proxy
                .timeout
                .map(|t| i64::try_from(t.get()).unwrap_or(i64::MAX)),
        )
        .bind(i64::try_from(proxy.body_limit.get()).unwrap_or(i64::MAX))
        .bind(proxy.replace.map(doppel_core::config::Ratio::get))
        .bind(proxy.rewrite_redirects)
        .bind(as_text(&proxy.resolve.kind)?)
        .bind(
            proxy
                .resolve
                .header
                .as_ref()
                .map(doppel_core::config::HeaderName::as_str),
        )
        .bind(proxy.loss.as_ref().map(|l| l.percentage.get()))
        .bind(proxy.loss.as_ref().map(|l| i32::from(l.status.get())))
        .bind(proxy.latency.as_ref().map(|l| l.percentage.get()))
        .bind(proxy.latency.as_ref().map(|l| l.min.get()))
        .bind(proxy.latency.as_ref().map(|l| l.max.get()))
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
        .bind(mock.name.as_str())
        .bind(ordinal)
        .bind(mock.request.method.as_str())
        .bind(mock.request.url.as_str())
        .bind(i32::from(mock.response.status.get()))
        .bind(mock.response.body.as_deref())
        .bind(mock.response.json.as_deref())
        .bind(
            mock.response
                .template
                .as_ref()
                .map(doppel_core::config::TemplateName::as_str),
        )
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

/// The two header statements, written out rather than assembled.
///
/// Assembling them from a shared column list would keep them in step
/// automatically, but only by building SQL with `format!` -- which sqlx
/// refuses by design, because that is where injection lives, and silencing
/// the refusal here would weaken it everywhere. They are literals instead,
/// and `the_two_header_statements_write_the_same_columns` below is what keeps
/// them in step: a column added to one and forgotten in the other would make
/// a configuration differ depending on whether it was created or updated.
///
/// The bind order is the one `pipe_header` writes, and the tests below
/// check the statements against it.
const UPDATE_HEADER: &str = "UPDATE configurations SET revision = $1, \
     admin_enable = $4, server_host = $5, server_port = $6, log_level = $7, \
     log_format = $8, control_socket = $9, templates_dir = $10, sentry_dsn = $11, \
     admin_host = $12, admin_port = $13, admin_auth_header = $14, \
     admin_upload_limit = $15, admin_access = $16, admin_groups = $17, \
     admin_public = $18, updated_at = now() \
     WHERE name = $2 AND revision = $3";

const UPSERT_HEADER: &str = "INSERT INTO configurations \
     (name, revision, admin_enable, server_host, server_port, log_level, log_format, \
      control_socket, templates_dir, sentry_dsn, admin_host, admin_port, \
      admin_auth_header, admin_upload_limit, admin_access, admin_groups, \
      admin_public) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
             $17) \
     ON CONFLICT (name) DO UPDATE SET revision = $2, \
     admin_enable = $3, server_host = $4, server_port = $5, log_level = $6, \
     log_format = $7, control_socket = $8, templates_dir = $9, sentry_dsn = $10, \
     admin_host = $11, admin_port = $12, admin_auth_header = $13, \
     admin_upload_limit = $14, admin_access = $15, admin_groups = $16, \
     admin_public = $17, updated_at = now()";

/// Bind the header values in `HEADER_COLUMNS` order.
///
/// A trait rather than a free function so the call reads in the same place the
/// query does: the order of these binds and the order of that list are the one
/// thing a reader has to check, and putting them side by side is what makes
/// that possible.
trait BindHeader<'q> {
    fn pipe_header(self, config: &'q Config) -> Self;
}

impl<'q> BindHeader<'q> for sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments> {
    fn pipe_header(self, config: &'q Config) -> Self {
        self.bind(config.admin.enable)
            .bind(config.server.host.to_string())
            .bind(i32::from(config.server.port.get()))
            .bind(text_of(&config.logging.level))
            .bind(text_of(&config.logging.format))
            .bind(config.control.socket.to_string_lossy().into_owned())
            .bind(config.templates.dir.to_string_lossy().into_owned())
            .bind(config.sentry.as_ref().map(|s| s.dsn.clone()))
            .bind(config.admin.host.to_string())
            .bind(i32::from(config.admin.port.get()))
            .bind(config.admin.auth.header.as_str())
            .bind(i64::try_from(config.admin.upload.limit.get()).unwrap_or(i64::MAX))
            .bind(serde_json::to_value(&config.admin.access).unwrap_or(serde_json::Value::Null))
            // `None` binds SQL NULL, which is what an absent `groups` has to
            // round-trip as: see the 0003 migration for why materialising the
            // default would break every pre-existing configuration's revision.
            .bind(
                config
                    .admin
                    .groups
                    .as_ref()
                    .map(|groups| serde_json::to_value(groups).unwrap_or(serde_json::Value::Null)),
            )
            .bind(config.admin.public)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The header columns, in the order `pipe_header` binds them.
    ///
    /// Lives here rather than beside the statements because only these tests
    /// consume it, and a constant nothing in the binary reads is dead code the
    /// compiler is right to refuse.
    const HEADER_BINDS: &[&str] = &[
        "admin_enable",
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
        "admin_groups",
        "admin_public",
    ];

    /// Every `column = $n` assignment in a statement.
    fn assigned_columns(sql: &str) -> Vec<&str> {
        sql.split(',')
            .filter_map(|fragment| fragment.split_once('='))
            .map(|(column, _)| column.trim().rsplit(' ').next().unwrap_or(column).trim())
            .filter(|column| !column.is_empty())
            .collect()
    }

    #[test]
    fn the_two_header_statements_write_the_same_columns() {
        // The one thing a shared, formatted column list would have given for
        // free, bought back with a test instead -- because building SQL by
        // formatting is what sqlx refuses, and silencing that refusal here
        // would weaken it everywhere else in the crate.
        //
        // A column added to one statement and forgotten in the other makes a
        // configuration differ depending on whether it was created or
        // updated, which is the kind of bug that only appears on the second
        // deployment.
        let mut update: Vec<_> = assigned_columns(UPDATE_HEADER)
            .into_iter()
            .filter(|column| *column != "revision" && *column != "name")
            .collect();
        let mut upsert: Vec<_> = assigned_columns(UPSERT_HEADER)
            .into_iter()
            .filter(|column| *column != "revision")
            .collect();
        update.sort_unstable();
        upsert.sort_unstable();
        assert_eq!(update, upsert, "the two header statements have drifted");
    }

    #[test]
    fn both_statements_write_exactly_the_columns_pipe_header_binds() {
        // The other half: the statements agreeing with each other is no use if
        // they disagree with the bind order.
        let mut expected: Vec<_> = HEADER_BINDS.to_vec();
        expected.push("updated_at");
        expected.sort_unstable();

        let mut written: Vec<_> = assigned_columns(UPDATE_HEADER)
            .into_iter()
            .filter(|column| *column != "revision" && *column != "name")
            .collect();
        written.sort_unstable();
        assert_eq!(written, expected);
    }

    #[test]
    fn a_revision_survives_the_trip_through_a_signed_column() {
        // PostgreSQL has no unsigned 64-bit integer, and a hash above
        // `i64::MAX` is perfectly ordinary. The mapping reuses the bits rather
        // than converting the value, so it has to be exactly reversible.
        for raw in [0, 1, u64::MAX, u64::MAX / 2, 0x8000_0000_0000_0000] {
            assert_eq!(from_i64(as_i64(Revision(raw))), Revision(raw));
        }
    }

    #[test]
    fn an_ordinal_that_cannot_fit_is_refused_rather_than_wrapped() {
        // Wrapping would reorder the document silently, and order decides
        // which mock answers a request.
        assert!(ordinal_of(0).is_ok());
        assert!(ordinal_of(usize::MAX).is_err());
    }
}
