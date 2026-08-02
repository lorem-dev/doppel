//! The store conformance suite, against `PostgresStore`.
//!
//! The same assertions run against `FileStore` in `doppel-core`. That both
//! callers exist is the phase's exit criterion: "PostgreSQL works" is the
//! weaker claim, and it would let two implementations of one trait drift into
//! different semantics without any test noticing.

use doppel_store_postgres::test_support::{TestSchema, require_database};

#[tokio::test]
async fn the_postgres_store_satisfies_the_contract() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;
    let store = schema.store().await;

    doppel_core::conformance::run_all(&store).await;

    schema.drop().await;
}
