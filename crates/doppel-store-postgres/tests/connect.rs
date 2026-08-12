//! Connecting, and refusing to connect to a schema that has not been
//! migrated.

use doppel_store_postgres::test_support::{TestSchema, require_database};

#[tokio::test]
async fn connect_succeeds_against_a_migrated_schema() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    doppel_store_postgres::PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect to a migrated schema");

    schema.drop().await;
}

#[tokio::test]
async fn connect_to_an_unmigrated_schema_names_the_command_that_fixes_it() {
    // Refusing is the point, but so is the message: an operator who has just
    // pointed a new deployment at an empty database needs to be told what to
    // run, not that some relation does not exist.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;

    let err = doppel_store_postgres::PostgresStore::connect(
        &schema.url(),
        "default",
        schema.templates_dir(),
    )
    .await
    .expect_err("an unmigrated schema must be refused");
    let message = err.to_string();
    assert!(
        message.contains("doppel config migrate"),
        "the message must name the command, got: {message}"
    );

    schema.drop().await;
}

#[tokio::test]
async fn migrations_are_idempotent() {
    // `config migrate` is something an operator will run twice, if only
    // because they cannot remember whether they already did.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;
    schema.migrate().await;

    doppel_store_postgres::PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect after a repeated migration");

    schema.drop().await;
}

#[tokio::test]
async fn two_schemas_do_not_see_each_other() {
    // The whole reason tests get a schema each: they run in parallel against
    // one database, and a shared table would make them order-dependent.
    let Some(url) = require_database() else {
        return;
    };
    let first = TestSchema::create(&url).await;
    let second = TestSchema::create(&url).await;
    first.migrate().await;
    second.migrate().await;

    assert_ne!(first.name(), second.name());
    first
        .execute(
            "INSERT INTO configurations (name, revision, settings) \
                  VALUES ('only-in-first', 0, '{}')",
        )
        .await;

    assert_eq!(first.count("configurations").await, 1);
    assert_eq!(
        second.count("configurations").await,
        0,
        "one schema's rows must not be visible from another"
    );

    first.drop().await;
    second.drop().await;
}
