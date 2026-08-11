//! Writing a configuration, under compare-and-swap.

use doppel_core::store::{Revision, StoreError};
use doppel_store_postgres::PostgresStore;
use doppel_store_postgres::test_support::{TestSchema, require_database};

const BASE: &str = r#"
server:
  host: "127.0.0.1"
  port: 18080
admin:
  host: "127.0.0.1"
  port: 18081
  tokens:
    - name: root
      group: admin
      token: root-token-0000000000000000000000000
  access: {}
  upload:
    limit: 1Mi
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    mocks:
      - name: only
        request:
          method: GET
          url: /widgets/
        response:
          status: 200
          body: 'hello'
"#;

fn parse(yaml: &str) -> doppel_core::Config {
    doppel_core::config::load_from_str(yaml).expect("the fixture parses")
}

async fn store(schema: &TestSchema) -> PostgresStore {
    PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect")
}

async fn migrated(url: &str) -> TestSchema {
    let schema = TestSchema::create(url).await;
    schema.migrate().await;
    schema
}

#[tokio::test]
async fn save_then_load_round_trips() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    let config = parse(BASE);
    let saved = store.save_config(&config, None).await.expect("provision");
    let (loaded, revision) = store.load_config().await.expect("load");

    assert_eq!(loaded, config);
    assert_eq!(revision, saved);
    assert_eq!(saved, Revision::of_config(&config));

    schema.drop().await;
}

#[tokio::test]
async fn a_stale_expected_revision_is_refused_and_changes_nothing() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    let first = parse(BASE);
    let saved = store.save_config(&first, None).await.expect("provision");

    let second = parse(&BASE.replace("alpha.example.com", "moved.example.com"));
    let err = store
        .save_config(&second, Some(Revision(saved.0 ^ 1)))
        .await
        .expect_err("a stale revision must be refused");
    assert!(
        matches!(err, StoreError::RevisionMismatch { .. }),
        "expected RevisionMismatch, got {err:?}"
    );

    // The whole transaction rolled back, so not one row moved.
    let (loaded, revision) = store.load_config().await.expect("load");
    assert_eq!(loaded, first);
    assert_eq!(revision, saved);

    schema.drop().await;
}

#[tokio::test]
async fn the_current_revision_is_accepted() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    let first = parse(BASE);
    let saved = store.save_config(&first, None).await.expect("provision");

    let second = parse(&BASE.replace("alpha.example.com", "moved.example.com"));
    let next = store
        .save_config(&second, Some(saved))
        .await
        .expect("the current revision must be accepted");

    assert_ne!(next, saved);
    let (loaded, _) = store.load_config().await.expect("load");
    assert_eq!(loaded, second);

    schema.drop().await;
}

#[tokio::test]
async fn a_conditional_save_against_nothing_is_not_found_not_a_mismatch() {
    // "You are holding a stale copy" and "there is nothing here to update"
    // send a caller in different directions, so the two cannot share a code.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    let err = store
        .save_config(&parse(BASE), Some(Revision(1)))
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, StoreError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );

    schema.drop().await;
}

#[tokio::test]
async fn an_invalid_configuration_writes_nothing() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    // Two proxies both claiming the default resolver: refused by the rule set.
    let invalid = parse(&format!(
        "{BASE}  - name: beta\n    type: http\n    url: \"https://beta.example.com/api/\"\n"
    ));
    let err = store
        .save_config(&invalid, None)
        .await
        .expect_err("validation must refuse it");
    assert!(
        matches!(err, StoreError::Invalid(_)),
        "expected Invalid, got {err:?}"
    );

    assert_eq!(
        schema.count("configurations").await,
        0,
        "a refused save must not have written a header row"
    );

    schema.drop().await;
}

#[tokio::test]
async fn a_rewrite_leaves_no_rows_from_the_previous_version() {
    // Proxy and mock rows are deleted and rewritten rather than diffed, so a
    // proxy that the new document drops must be gone -- not left behind to be
    // loaded back.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    let two_proxies = parse(&format!(
        "{BASE}  - name: beta\n    type: http\n    url: \"https://beta.example.com/api/\"\n    \
         resolve:\n      type: header\n      header: X-Proxy-Name\n"
    ));
    let saved = store
        .save_config(&two_proxies, None)
        .await
        .expect("provision");
    assert_eq!(schema.count("proxies").await, 2);

    let one_proxy = parse(BASE);
    store
        .save_config(&one_proxy, Some(saved))
        .await
        .expect("shrink");

    assert_eq!(schema.count("proxies").await, 1);
    let (loaded, _) = store.load_config().await.expect("load");
    assert_eq!(loaded, one_proxy);
    // A dropped proxy's mocks used to be a `mocks` row count; they now live
    // inside the proxy's own document, so deleting the row takes them with it and
    // what is left to check is that the surviving document kept its own.
    assert_eq!(loaded.proxies[0].mocks.len(), 1);

    schema.drop().await;
}

#[tokio::test]
async fn two_saves_from_the_same_revision_produce_exactly_one_winner() {
    // The point of the compare-and-swap. Both start from the same revision;
    // the database serialises them, and the loser must be told rather than
    // silently overwriting.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;

    let provisioning = store(&schema).await;
    let base = provisioning
        .save_config(&parse(BASE), None)
        .await
        .expect("provision");

    // Two independent stores, so the two writes go through two pools rather
    // than being serialised by one connection.
    let first = store(&schema).await;
    let second = store(&schema).await;
    let left = parse(&BASE.replace("alpha.example.com", "left.example.com"));
    let right = parse(&BASE.replace("alpha.example.com", "right.example.com"));

    let (a, b) = tokio::join!(
        first.save_config(&left, Some(base)),
        second.save_config(&right, Some(base))
    );

    let winners = usize::from(a.is_ok()) + usize::from(b.is_ok());
    assert_eq!(winners, 1, "exactly one save must win: {a:?} / {b:?}");
    let loser = if a.is_err() { a } else { b };
    assert!(
        matches!(loser, Err(StoreError::RevisionMismatch { .. })),
        "the loser must be told it lost: {loser:?}"
    );

    schema.drop().await;
}

/// Multi-threaded on purpose: the reads and the writes have to be able to
/// interleave in the database, not merely at this runtime's await points.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_load_racing_a_save_never_reports_the_rows_as_diverged() {
    // A configuration is five tables. Each query used to take its own
    // connection and its own snapshot, so a `save` committing in between
    // produced a configuration assembled from two of them -- which the
    // revision check at the end of `load` caught and reported as "the rows
    // and the revision column have diverged". True as far as it goes, and a
    // message that sends an operator hunting for a hand-edited row that does
    // not exist.
    //
    // Measured at 37 failures in 600 loads before `load_config` took one
    // `REPEATABLE READ` snapshot for the whole read. The assertion is exact
    // rather than a threshold: with one snapshot this is not a probability.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let writer = store(&schema).await;
    let reader = store(&schema).await;
    writer
        .save_config(&parse(BASE), None)
        .await
        .expect("provision");

    let write = tokio::spawn(async move {
        for i in 0..300 {
            let text = BASE.replace("alpha.example.com", &format!("host{i}.example.com"));
            let _ = writer.save_config(&parse(&text), None).await;
        }
    });

    let mut diverged = Vec::new();
    for _ in 0..600 {
        if let Err(err) = reader.load_config().await {
            diverged.push(err.to_string());
        }
    }
    write.await.expect("the writer finishes");

    assert!(
        diverged.is_empty(),
        "{} of 600 concurrent loads failed; first: {}",
        diverged.len(),
        diverged.first().map_or("", String::as_str)
    );

    schema.drop().await;
}

#[tokio::test]
async fn an_unconditional_save_replaces_whatever_was_there() {
    // `expected: None` is for provisioning and `config push`, where the caller
    // is deliberately not building on what is stored.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    store.save_config(&parse(BASE), None).await.expect("first");
    let replacement = parse(&BASE.replace("alpha.example.com", "replaced.example.com"));
    store
        .save_config(&replacement, None)
        .await
        .expect("an unconditional save must not need a revision");

    let (loaded, _) = store.load_config().await.expect("load");
    assert_eq!(loaded, replacement);

    schema.drop().await;
}

#[tokio::test]
async fn a_disabled_admin_listener_survives_the_round_trip() {
    // Every other fixture leaves `admin.enable` at its default, so a store
    // that always read it back as `true` would pass all of them. This is the
    // only test that can tell.
    let Some(url) = require_database() else {
        return;
    };
    let schema = migrated(&url).await;
    let store = store(&schema).await;

    let disabled = parse(&BASE.replace("  port: 18081", "  port: 18081\n  enable: false"));
    assert!(
        !disabled.admin.enable,
        "the fixture must actually disable it"
    );

    store.save_config(&disabled, None).await.expect("provision");
    let (loaded, _) = store.load_config().await.expect("load");

    assert!(!loaded.admin.enable);
    assert_eq!(loaded, disabled);

    schema.drop().await;
}
