//! Writing a configuration into two tables.
//!
//! The document is stored as JSON rather than spread across a column per field.
//! That is a deliberate reversal of the original schema, and the reason is the
//! migration it saves: between 0.3.0 and 0.4.1 three fields were added to the
//! configuration format, and each one needed a migration whose only content was
//! `ADD COLUMN` -- plus a matching edit here, in `load`, and in two hand-written
//! statements that a test existed solely to keep in step. A field added to the
//! serde model now reaches the database with no schema change at all.
//!
//! What stays a column is what SQL actually uses: `name` and `revision` for the
//! compare-and-swap, `ordinal` for proxy order, `config` for the foreign key.
//! Nothing ever queried `admin_host`.

use doppel_core::config::Config;
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
        let settings = settings_of(config)?;

        let mut tx = self
            .pool()
            .begin()
            .await
            .map_err(|err| StoreError::Unavailable(format!("cannot begin: {err}")))?;

        match expected {
            Some(expected) => {
                self.update_header(&mut tx, &settings, revision, expected)
                    .await?;
            }
            None => self.upsert_header(&mut tx, &settings, revision).await?,
        }

        // Deleted and rewritten rather than diffed. A diff is more code whose
        // failure mode is a row it missed -- a configuration silently
        // disagreeing with the document that produced it. This is one
        // transaction either way, so the rewrite costs nothing an operator can
        // observe.
        //
        // `templates` deliberately does not cascade from here: dropping a
        // proxy's files is a separate decision made after the configuration
        // write, never as part of it.
        sqlx::query("DELETE FROM proxies WHERE config = $1")
            .bind(&self.config_name)
            .execute(&mut *tx)
            .await
            .map_err(write_failed)?;

        for (ordinal, proxy) in config.proxies.iter().enumerate() {
            sqlx::query(
                "INSERT INTO proxies (config, name, ordinal, document) VALUES ($1, $2, $3, $4)",
            )
            .bind(&self.config_name)
            .bind(proxy.name.as_str())
            .bind(ordinal_of(ordinal)?)
            .bind(as_json(proxy)?)
            .execute(&mut *tx)
            .await
            .map_err(write_failed)?;
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
        settings: &serde_json::Value,
        revision: Revision,
        expected: Revision,
    ) -> Result<(), StoreError> {
        let affected = sqlx::query(UPDATE_HEADER)
            .bind(as_i64(revision))
            .bind(&self.config_name)
            .bind(as_i64(expected))
            .bind(settings)
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
        settings: &serde_json::Value,
        revision: Revision,
    ) -> Result<(), StoreError> {
        sqlx::query(UPSERT_HEADER)
            .bind(&self.config_name)
            .bind(as_i64(revision))
            .bind(settings)
            .execute(&mut **tx)
            .await
            .map_err(write_failed)?;
        Ok(())
    }
}

/// The two header statements, written out rather than assembled.
///
/// sqlx refuses SQL built by formatting, by design, because that is where
/// injection lives -- so these are literals. There are two columns to keep in
/// step now rather than seventeen, which is most of the point of storing the
/// document as JSON.
const UPDATE_HEADER: &str = "UPDATE configurations SET revision = $1, settings = $4, \
     updated_at = now() WHERE name = $2 AND revision = $3";

const UPSERT_HEADER: &str = "INSERT INTO configurations (name, revision, settings) \
     VALUES ($1, $2, $3) \
     ON CONFLICT (name) DO UPDATE SET revision = $2, settings = $3, updated_at = now()";

/// The configuration as it is stored: the whole document except its proxies.
///
/// The proxies come out because each one is a row of its own -- they are
/// addressed by name, ordered by `ordinal`, and read back individually. Nothing
/// else is transformed, which is what makes this immune to a field being added:
/// `serde_json::to_value` follows the same model `Deserialize` reads back, so
/// there is no second description of the configuration to keep in step.
fn settings_of(config: &Config) -> Result<serde_json::Value, StoreError> {
    let mut document = as_json(config)?;
    document
        .as_object_mut()
        .ok_or_else(|| {
            StoreError::Unavailable("a configuration did not serialize to an object".to_owned())
        })?
        .remove("proxies");
    Ok(document)
}

fn as_json<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, StoreError> {
    serde_json::to_value(value)
        .map_err(|err| StoreError::Unavailable(format!("cannot serialize for storage: {err}")))
}

/// A list index as the column's type.
///
/// Fails rather than truncating: a configuration with more than two billion
/// proxies is not a real configuration, and a wrapped ordinal would silently
/// reorder the list.
fn ordinal_of(index: usize) -> Result<i32, StoreError> {
    i32::try_from(index)
        .map_err(|_| StoreError::Unavailable(format!("ordinal {index} does not fit a column")))
}

/// A `u64` revision in a signed column, bit for bit.
///
/// Postgres has no unsigned integer, and the alternative -- storing it as text
/// -- would make the compare-and-swap a string comparison.
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

    /// Both statements have to write both columns.
    ///
    /// The hazard this guards used to be much larger -- seventeen columns
    /// spread across two literals, where one added to the update and forgotten
    /// in the upsert made a configuration differ depending on whether it was
    /// created or replaced. Two columns is small enough to read, and cheap
    /// enough to keep asserting.
    #[test]
    fn both_header_statements_write_the_revision_and_the_settings() {
        for statement in [UPDATE_HEADER, UPSERT_HEADER] {
            assert!(
                statement.contains("revision = $"),
                "no revision assignment: {statement}"
            );
            assert!(
                statement.contains("settings = $"),
                "no settings assignment: {statement}"
            );
            assert!(
                statement.contains("updated_at = now()"),
                "no updated_at: {statement}"
            );
        }
    }

    /// The conditional half must stay conditional.
    ///
    /// Without the revision in the `WHERE`, `save_config` would overwrite a
    /// configuration that had moved under the caller, which is the one thing
    /// compare-and-swap exists to prevent, and no other test here would notice.
    #[test]
    fn the_update_is_guarded_by_the_expected_revision() {
        assert!(
            UPDATE_HEADER.contains("WHERE name = $2 AND revision = $3"),
            "{UPDATE_HEADER}"
        );
    }

    #[test]
    fn the_settings_document_holds_everything_except_the_proxies() {
        let config = doppel_core::config::load_from_str(
            r#"
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
"#,
        )
        .unwrap();

        let settings = settings_of(&config).unwrap();
        let object = settings.as_object().unwrap();
        assert!(
            !object.contains_key("proxies"),
            "proxies belong in their own rows: {settings}"
        );
        for section in ["server", "admin", "logging", "control", "templates"] {
            assert!(object.contains_key(section), "{section} is missing");
        }

        // And what came out parses straight back into a configuration, which is
        // what `load` relies on. An absent `proxies` is legal, so the reparsed
        // document is a whole configuration with an empty proxy list.
        let reparsed: Config = serde_json::from_value(settings).unwrap();
        assert!(reparsed.proxies.is_empty());
        assert_eq!(reparsed.admin.port, config.admin.port);
    }

    #[test]
    fn a_revision_survives_the_trip_through_a_signed_column() {
        // The high bit is the case that matters: a revision past i64::MAX has to
        // come back as the same u64 rather than as a negative number.
        for revision in [
            Revision(0),
            Revision(1),
            Revision(u64::MAX),
            Revision(1 << 63),
        ] {
            assert_eq!(from_i64(as_i64(revision)), revision);
        }
    }

    #[test]
    fn an_ordinal_that_cannot_fit_is_refused_rather_than_wrapped() {
        assert!(ordinal_of(0).is_ok());
        assert!(ordinal_of(usize::MAX).is_err());
    }
}
