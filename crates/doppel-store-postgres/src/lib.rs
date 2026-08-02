//! The PostgreSQL configuration store.
//!
//! A crate of its own rather than a module of `doppel-core`, which everything
//! depends on: a database driver and its TLS stack have no business inside the
//! proxy, the renderer or the admin API, none of which talk to a database.

mod load;

use doppel_core::store::StoreError;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// The migrations, embedded at compile time from the repository root.
///
/// `migrate!` reads the directory while the crate is being built and bakes the
/// files into the binary. It needs no database to do so, which is what keeps
/// `cargo build` working with nothing running -- unlike `query!`, which is
/// deliberately not used here (phase 4 specification, section 4).
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

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
    pub async fn connect(url: &str, config_name: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .connect(url)
            .await
            .map_err(|err| StoreError::Unavailable(format!("cannot reach the database: {err}")))?;

        Self::require_migrated(&pool).await?;

        Ok(Self {
            pool,
            config_name: config_name.to_owned(),
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
