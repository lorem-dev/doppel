//! The PostgreSQL configuration store.
//!
//! A crate of its own rather than a module of `doppel-core`, which everything
//! depends on: a database driver and its TLS stack have no business inside the
//! proxy, the renderer or the admin API, none of which talk to a database.

mod load;
mod save;
mod templates;

#[cfg(feature = "test-support")]
pub mod test_support;

use doppel_core::store::StoreError;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// The migrations, embedded at compile time from this crate.
///
/// `migrate!` reads the directory while the crate is being built and bakes the
/// files into the binary. It needs no database to do so, which is what keeps
/// `cargo build` working with nothing running -- unlike `query!`, which is
/// deliberately not used here (phase 4 specification, section 4).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Apply any migrations the database has not seen.
///
/// Called by `doppel config migrate` and by the tests. Never by `connect`: a
/// process that silently alters a shared schema on startup turns a rollback
/// into data loss, and the operator who rolled back is the one least
/// expecting it.
/// Generic over the target so a caller can hand it a pool or a single
/// connection. `config migrate` uses one connection: a pool retries until its
/// acquire timeout, which turns a refused connection into thirty seconds of
/// silence before the error an operator is waiting for.
pub async fn migrate<'a, A>(target: A) -> Result<(), StoreError>
where
    A: sqlx::Acquire<'a, Database = sqlx::Postgres>,
{
    MIGRATOR
        .run(target)
        .await
        .map_err(|err| StoreError::Unavailable(format!("cannot apply migrations: {err}")))
}

pub struct PostgresStore {
    pool: PgPool,
    config_name: String,
    /// Where template rows are mirrored. Passed in rather than read from the
    /// configuration, exactly as `FileStore` takes it: template operations
    /// have to work before the first successful load, and the configuration is
    /// behind that load.
    templates_dir: std::path::PathBuf,
}

/// A hand-written `Debug`, like `StoreArgs`'s, rather than the derived one.
/// `PgPool`'s own `Debug` prints its connect options, and those carry the
/// password: anyone who ever formats a store -- in a log line, a panic
/// message, a test's `expect_err` -- must not be able to make it come out in
/// the clear.
impl std::fmt::Debug for PostgresStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresStore")
            .field("config_name", &self.config_name)
            .finish_non_exhaustive()
    }
}

impl PostgresStore {
    /// Open a pool and refuse to proceed against a schema that has not been
    /// migrated.
    ///
    /// The refusal names `doppel config migrate` rather than letting the first
    /// query fail: an operator who has just pointed a deployment at an empty
    /// database needs to be told what to run, not that some relation does not
    /// exist.
    pub async fn connect(
        url: &str,
        config_name: &str,
        templates_dir: impl Into<std::path::PathBuf>,
    ) -> Result<Self, StoreError> {
        // Probed with a single connection before the pool is built. A pool
        // retries until its acquire timeout, so a refused connection would sit
        // silent for thirty seconds at startup before reporting what the
        // operating system returned immediately. Lowering that timeout instead
        // would fix startup by making the running server impatient under load:
        // one knob serving two requirements, so the requirements are separated
        // here rather than traded off against each other.
        {
            use sqlx::Connection;
            let probe = sqlx::PgConnection::connect(url).await.map_err(|err| {
                StoreError::Unavailable(format!("cannot reach the database: {err}"))
            })?;
            let _ = probe.close().await;
        }

        let pool = PgPoolOptions::new()
            .connect(url)
            .await
            .map_err(|err| StoreError::Unavailable(format!("cannot reach the database: {err}")))?;

        Self::require_migrated(&pool).await?;

        Ok(Self {
            pool,
            config_name: config_name.to_owned(),
            templates_dir: templates_dir.into(),
        })
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    #[must_use]
    pub fn config_name(&self) -> &str {
        &self.config_name
    }

    #[must_use]
    pub fn templates_dir(&self) -> &std::path::Path {
        &self.templates_dir
    }

    /// Every embedded migration must already be applied.
    ///
    /// Checked by version rather than by count: a database carrying three
    /// migrations that are not these three is not one this binary can use, and
    /// counting would call it ready.
    async fn require_migrated(pool: &PgPool) -> Result<(), StoreError> {
        let not_ready = |detail: &str| {
            StoreError::Unavailable(format!(
                "the database schema is not ready ({detail}); run `doppel config migrate`"
            ))
        };

        let applied: Vec<i64> = match sqlx::query_scalar("SELECT version FROM _sqlx_migrations")
            .fetch_all(pool)
            .await
        {
            Ok(versions) => versions,
            // The bookkeeping table itself is absent, which is what an
            // untouched database looks like.
            Err(_) => return Err(not_ready("no migrations have been applied")),
        };

        for migration in MIGRATOR.iter() {
            if !applied.contains(&migration.version) {
                return Err(not_ready(&format!(
                    "migration {} ({}) has not been applied",
                    migration.version, migration.description
                )));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl doppel_core::store::ConfigStore for PostgresStore {
    async fn load(&self) -> Result<(doppel_core::Config, doppel_core::Revision), StoreError> {
        self.load_config().await
    }

    async fn save(
        &self,
        config: &doppel_core::Config,
        expected: Option<doppel_core::Revision>,
    ) -> Result<doppel_core::Revision, StoreError> {
        self.save_config(config, expected).await
    }

    async fn load_templates(
        &self,
        proxy: &str,
    ) -> Result<Vec<doppel_core::store::TemplateFile>, StoreError> {
        self.load_template_rows(proxy).await
    }

    async fn save_template(&self, proxy: &str, file: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.save_template_row(proxy, file, bytes).await
    }

    async fn delete_template(&self, proxy: &str, file: &str) -> Result<bool, StoreError> {
        self.delete_template_row(proxy, file).await
    }

    async fn retain_templates(&self, proxy: &str, keep: &[String]) -> Result<(), StoreError> {
        self.retain_template_rows(proxy, keep).await
    }

    async fn rename_templates(&self, from: &str, to: &str) -> Result<(), StoreError> {
        self.rename_template_rows(from, to).await
    }

    async fn materialize_templates(&self, _dir: &std::path::Path) -> Result<(), StoreError> {
        // The directory is `self.templates_dir`, fixed when the store was
        // opened, so the parameter is redundant here. It exists on the trait
        // because a future store might not know where to write until it is
        // told; ignoring it is honest, silently writing somewhere else would
        // not be.
        self.materialize().await
    }
}
