//! `config migrate` and `config migrate --status`, driven through the binary.
//!
//! Through the binary because the exit code is half of what `--status`
//! promises: a deploy gate branches on it rather than parsing the text.

use doppel_store_postgres::test_support::{TestSchema, require_database};
use std::process::Command;

fn doppel(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_doppel"))
        .args(args)
        .env_remove("DOPPEL_DATABASE_URL")
        .output()
        .expect("doppel runs")
}

fn status(url: &str) -> (i32, String) {
    let out = doppel(&["config", "migrate", "--status", "--database-url", url]);
    (
        out.status.code().expect("the process exited normally"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[tokio::test]
async fn an_untouched_database_reports_behind_and_exits_one() {
    let Some(url) = require_database() else {
        return;
    };
    // Created but not migrated: the bookkeeping table does not exist, which
    // is exactly what a fresh database looks like.
    let schema = TestSchema::create(&url).await;

    let (code, text) = status(&schema.url());
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("no migrations have been applied"), "{text}");
    assert!(text.contains("doppel config migrate"), "{text}");

    schema.drop().await;
}

#[tokio::test]
async fn a_migrated_database_reports_its_version_and_exits_zero() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    let (code, text) = status(&schema.url());
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("up to date"), "{text}");

    // Read from the embedded migrations rather than written down. Hardcoding
    // `1` here meant the first migration ever added broke this test for no
    // reason -- it is about `--status` reporting the version it found, not about
    // which version that happens to be today.
    //
    // The version, not the count, is what identifies the schema. They were the
    // same number while there was one migration, so the assertion names the
    // word to stay honest once they diverge.
    let newest = doppel_store_postgres::MIGRATOR
        .iter()
        .map(|m| m.version)
        .max()
        .expect("the crate embeds at least one migration");
    assert!(text.contains(&format!("schema version {newest}")), "{text}");

    schema.drop().await;
}

#[tokio::test]
async fn an_applied_migration_that_no_longer_matches_its_file_is_reported() {
    // The reason this reads sqlx's table rather than a single stored revision
    // number: a number cannot notice that the file behind it changed. Until
    // the first release, migrations are merged into the initial one rather
    // than appended, so this is the mistake most likely to actually happen.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;
    schema
        .execute(r"UPDATE _sqlx_migrations SET checksum = '\x00'::bytea WHERE version = 1")
        .await;

    let (code, text) = status(&schema.url());
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("changed: migration 1"), "{text}");
    assert!(
        text.contains("does not match the file this binary carries"),
        "{text}"
    );

    schema.drop().await;
}

#[tokio::test]
async fn a_migration_recorded_as_incomplete_is_reported() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;
    schema
        .execute("UPDATE _sqlx_migrations SET success = false WHERE version = 1")
        .await;

    let (code, text) = status(&schema.url());
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("failed: migration 1"), "{text}");

    schema.drop().await;
}

#[tokio::test]
async fn a_migration_this_binary_does_not_carry_is_named_but_not_treated_as_behind() {
    // An older binary looking at a database a newer one has migrated. Worth
    // saying, and not this binary's own requirement being unmet -- everything
    // it carries is applied, so it exits 0 and says what it saw.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;
    schema
        .execute(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (99, 'from_the_future', now(), true, '\\x00'::bytea, 0)",
        )
        .await;

    let (code, text) = status(&schema.url());
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("unknown: migration 99"), "{text}");
    assert!(text.contains("from_the_future"), "{text}");
    assert!(text.contains("up to date"), "{text}");
    // And the version reported is the highest one the database carries, not
    // the highest this binary knows about.
    assert!(text.contains("schema version 99"), "{text}");

    schema.drop().await;
}

#[tokio::test]
async fn status_changes_nothing() {
    // The promise in the flag's own help. A `--status` that migrated would be
    // the worst possible surprise in a deploy gate.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;

    let (code, _) = status(&schema.url());
    assert_eq!(code, 1);
    let (code, _) = status(&schema.url());
    assert_eq!(code, 1, "a second status must still find nothing applied");

    schema.drop().await;
}
