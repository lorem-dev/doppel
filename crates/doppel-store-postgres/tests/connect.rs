//! Connecting, and refusing to connect to a schema that has not been
//! migrated.

mod common;

use common::{TestSchema, require_database};

#[tokio::test]
async fn connect_succeeds_against_a_migrated_schema() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    doppel_store_postgres::PostgresStore::connect(&schema.url(), "default")
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

    let err = doppel_store_postgres::PostgresStore::connect(&schema.url(), "default")
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

    doppel_store_postgres::PostgresStore::connect(&schema.url(), "default")
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
            "INSERT INTO configurations (name, revision, server_host, server_port, \
                  log_level, log_format, control_socket, templates_dir, admin_host, \
                  admin_port, admin_auth_header, admin_upload_limit, admin_access) \
                  VALUES ('only-in-first', 0, '127.0.0.1', 8080, 'info', 'json', \
                  '/tmp/d.sock', './templates', '127.0.0.1', 8081, 'X-Proxy-Authorization', \
                  1048576, '{}')",
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
