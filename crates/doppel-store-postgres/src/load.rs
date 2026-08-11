//! Reading a configuration back out of two tables.
//!
//! Both tables hold JSON documents, so this file has no per-column mapping and
//! no per-column checks: `serde_json::from_value` runs the same `Deserialize`
//! impls the YAML loader runs, which is what makes a row read here subject to
//! exactly the rules a configuration file is subject to. The previous version of
//! this file carried twenty-five helpers doing that job by hand, one per column
//! type, and every new configuration field needed another edit here.

use doppel_core::config::{Config, ProxyConfig};
use doppel_core::store::{Revision, StoreError};
use sqlx::{Row, postgres::PgRow};

use crate::PostgresStore;

impl PostgresStore {
    /// Read this store's configuration.
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

        let row = sqlx::query("SELECT revision, settings FROM configurations WHERE name = $1")
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

        // The stored document omits `proxies`, which is a `#[serde(default)]`
        // field, so this yields a configuration with an empty proxy list and the
        // rows below fill it in.
        let mut config: Config = document(&row, "settings")?;
        config.proxies = self.proxies(&mut tx).await?;

        // The stored revision has to agree with the content it labels. They
        // diverge when a row was edited by hand, or when this code forgot to
        // read something `save` wrote -- and either way every compare-and-swap
        // downstream would then fail in a way that looks like contention rather
        // than like corruption.
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

    async fn proxies(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<Vec<ProxyConfig>, StoreError> {
        let rows =
            sqlx::query("SELECT name, document FROM proxies WHERE config = $1 ORDER BY ordinal")
                .bind(&self.config_name)
                .fetch_all(&mut **tx)
                .await
                .map_err(query_failed)?;

        let mut proxies = Vec::with_capacity(rows.len());
        for row in &rows {
            // The keyed name is read first so a document that fails to parse can
            // be reported against the proxy it belongs to. serde names the field
            // it could not read (`missing field 'max'`) but not the row, and
            // "some proxy is malformed" is not a report anybody can act on.
            let keyed: String = row.try_get("name").map_err(query_failed)?;
            let proxy: ProxyConfig = document(row, &format!("proxies[{keyed}].document"))?;

            // The name is both a column and a field of the document, because the
            // column is the primary key and the field is what the configuration
            // format holds. A row whose two disagree is addressable under one
            // name and serves another, so it is reported rather than resolved in
            // favour of either.
            if proxy.name.as_str() != keyed {
                return Err(corrupt(
                    "proxies.name",
                    &format!(
                        "row `{keyed}` holds a document naming `{}`",
                        proxy.name.as_str()
                    ),
                ));
            }

            proxies.push(proxy);
        }
        Ok(proxies)
    }
}

/// A stored JSON document, parsed through the configuration's own types.
///
/// `label` is what the violation is reported under and is not always the column
/// name -- a proxy's document is reported as `proxies[alpha].document`, because
/// serde says which field it could not read but nothing about which row held it.
fn document<T: serde::de::DeserializeOwned>(row: &PgRow, label: &str) -> Result<T, StoreError> {
    let column = label.rsplit('.').next().unwrap_or(label);
    let value: serde_json::Value = row.try_get(column).map_err(query_failed)?;
    serde_json::from_value(value).map_err(|err| corrupt(label, &err.to_string()))
}

fn query_failed(err: sqlx::Error) -> StoreError {
    StoreError::Unavailable(format!("query failed: {err}"))
}

/// A row the database accepted and the configuration format does not.
///
/// Reported as `Invalid` rather than `Unavailable`: the database is working
/// perfectly, and what is wrong is the content -- which is a different thing for
/// an operator to do something about.
fn corrupt(column: &str, detail: &str) -> StoreError {
    StoreError::Invalid(vec![doppel_core::Violation::new(
        column,
        format!("stored value is not one this version accepts: {detail}"),
    )])
}
