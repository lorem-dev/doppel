//! A PostgreSQL schema per test.
//!
//! Every file under `tests/` compiles this module separately, so a helper one
//! file does not use is genuinely dead code there.
#![allow(dead_code)]

use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool, Row};
use tempfile::TempDir;

/// The database these tests run against, or `None`.
///
/// Returning `None` rather than failing keeps `cargo test` green on a machine
/// with no database, which is the whole point of the variable.
///
/// The printed line is not the safeguard. `cargo test` captures the output of
/// a test that passes, so nobody ever reads it -- which was measured, not
/// assumed: a full gate run produced zero occurrences of it. `DOPPEL_REQUIRE_DATABASE`
/// is the safeguard. Setting it turns a missing URL into a failure, so a CI
/// job that expects to exercise PostgreSQL cannot pass by skipping. The
/// `run-tests-and-linters` skill sets both.
pub fn require_database() -> Option<String> {
    match std::env::var("DOPPEL_TEST_DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            assert!(
                std::env::var("DOPPEL_REQUIRE_DATABASE").is_err(),
                "DOPPEL_REQUIRE_DATABASE is set but DOPPEL_TEST_DATABASE_URL is not, so this \
                 test would have skipped silently. Start the database (`docker compose up -d`) \
                 and set the URL, or unset DOPPEL_REQUIRE_DATABASE to allow skipping."
            );
            eprintln!(
                "SKIPPED: DOPPEL_TEST_DATABASE_URL is not set. \
                 Run `docker compose up -d` and set it to exercise the PostgreSQL store."
            );
            None
        }
    }
}

/// A schema of this test's own, so tests can run in parallel against one
/// database without seeing each other's rows.
pub struct TestSchema {
    name: String,
    base_url: String,
    /// A single connection, not a pool, held for this schema's whole life.
    ///
    /// It carries the session-scoped advisory lock that marks the schema live.
    /// A pool would hand that lock to an arbitrary connection and could then
    /// recycle it, releasing the lock while the test is still running.
    guard: PgConnection,
    lock_key: i64,
    /// A template mirror of this test's own. Held for its Drop, so nothing
    /// survives the test and no test writes into the repository.
    templates: TempDir,
}

impl TestSchema {
    pub async fn create(base_url: &str) -> Self {
        let mut guard = PgConnection::connect(base_url)
            .await
            .expect("connect to the test database");

        // A name from the database's own generator. `gen_random_uuid` is built
        // in since PostgreSQL 13 and needs no extension, no sequence and no
        // DDL. An earlier version of this harness created a sequence with
        // `CREATE SEQUENCE IF NOT EXISTS`, which is not atomic against a
        // concurrent create: four tests starting at once hit a unique
        // violation on `pg_class`.
        let suffix: String = sqlx::query("SELECT replace(gen_random_uuid()::text, '-', '')")
            .fetch_one(&mut guard)
            .await
            .expect("draw a schema name")
            .get(0);
        let name = format!("doppel_test_{suffix}");
        let lock_key = lock_key_for(&name);

        // Taken before the schema exists and held until `drop`. A sweeper can
        // then tell a live schema from an abandoned one by whether this lock
        // is free, and a test that panics or is killed has it released by the
        // server when its connection closes. That is the whole reason the
        // marker is an advisory lock rather than a timestamp with an arbitrary
        // staleness window.
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(lock_key)
            .execute(&mut guard)
            .await
            .expect("mark the schema live");

        Self::sweep_abandoned(base_url).await;

        sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {name}")))
            .execute(&mut guard)
            .await
            .expect("create the test schema");

        Self {
            name,
            base_url: base_url.to_owned(),
            guard,
            lock_key,
            templates: tempfile::tempdir().expect("a templates directory"),
        }
    }

    /// Drop schemas left behind by tests that panicked or were killed.
    ///
    /// A candidate is abandoned exactly when its advisory lock can be taken: a
    /// running test holds that lock, and a dead one has had it released by the
    /// server. `pg_try_advisory_lock` distinguishes the two without a clock,
    /// without a staleness window, and without any risk of deleting a schema
    /// another test is using -- which an earlier version of this function did,
    /// because it assumed everything matching the prefix was finished.
    async fn sweep_abandoned(base_url: &str) {
        let Ok(mut conn) = PgConnection::connect(base_url).await else {
            return;
        };
        let rows =
            sqlx::query("SELECT nspname FROM pg_namespace WHERE nspname LIKE 'doppel\\_test\\_%'")
                .fetch_all(&mut conn)
                .await
                .unwrap_or_default();

        for row in rows {
            let name: String = row.get(0);
            let key = lock_key_for(&name);
            let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                .bind(key)
                .fetch_one(&mut conn)
                .await
                .unwrap_or(false);
            if !acquired {
                continue;
            }
            let _ = sqlx::raw_sql(AssertSqlSafe(format!(
                "DROP SCHEMA IF EXISTS {name} CASCADE"
            )))
            .execute(&mut conn)
            .await;
            let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(key)
                .execute(&mut conn)
                .await;
        }
        let _ = conn.close().await;
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The mirror directory a `PostgresStore` opened against this schema
    /// should use.
    pub fn templates_dir(&self) -> &std::path::Path {
        self.templates.path()
    }

    /// Open a store against this schema, with this schema's mirror directory.
    pub async fn store(&self) -> doppel_store_postgres::PostgresStore {
        doppel_store_postgres::PostgresStore::connect(&self.url(), "default", self.templates_dir())
            .await
            .expect("connect")
    }

    /// A URL that puts this schema first on the search path, so the migrations
    /// and every query land in it rather than in `public`.
    pub fn url(&self) -> String {
        let separator = if self.base_url.contains('?') {
            '&'
        } else {
            '?'
        };
        format!(
            "{}{separator}options=-c%20search_path%3D{}",
            self.base_url, self.name
        )
    }

    pub async fn migrate(&self) {
        let pool = PgPool::connect(&self.url())
            .await
            .expect("connect with the schema on the search path");
        doppel_store_postgres::migrate(&pool)
            .await
            .expect("run the migrations");
        pool.close().await;
    }

    pub async fn execute(&self, sql: &str) {
        let pool = PgPool::connect(&self.url()).await.expect("connect");
        sqlx::raw_sql(AssertSqlSafe(sql.to_owned()))
            .execute(&pool)
            .await
            .expect("execute");
        pool.close().await;
    }

    pub async fn count(&self, table: &str) -> i64 {
        let pool = PgPool::connect(&self.url()).await.expect("connect");
        let count: i64 = sqlx::raw_sql(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&pool)
            .await
            .expect("count")
            .get(0);
        pool.close().await;
        count
    }

    pub async fn drop(mut self) {
        let _ = sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.name
        )))
        .execute(&mut self.guard)
        .await;
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(self.lock_key)
            .execute(&mut self.guard)
            .await;
        let _ = self.guard.close().await;
    }
}

/// A stable key for a schema name, for `pg_advisory_lock`.
///
/// FNV-1a, the same function the revision uses and for the same reason: a
/// `DefaultHasher` is not guaranteed stable across Rust versions, and a
/// sweeper has to compute the same key as the test that took the lock.
fn lock_key_for(name: &str) -> i64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    // `pg_advisory_lock` takes a signed 64-bit key, so the bits are reused
    // rather than the value converted; the mapping only has to be injective.
    i64::from_ne_bytes(hash.to_ne_bytes())
}
