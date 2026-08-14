//! Reading a configuration out of the tables.

use doppel_core::store::Revision;
use doppel_store_postgres::PostgresStore;
use doppel_store_postgres::test_support::{TestSchema, require_database};

/// A configuration exercising every column the schema has: both resolve
/// kinds, both fault sections, a mock with all three selector maps, a
/// per-proxy access override, a mock-level proxy override, and a sentry
/// section.
const FIXTURE: &str = r#"
server:
  host: "127.0.0.1"
  port: 18080
logging:
  level: debug
  format: text
control:
  socket: /tmp/doppel-load.sock
templates:
  dir: ./tpl
sentry:
  dsn: "https://key@sentry.example.com/1"
admin:
  host: "127.0.0.1"
  port: 18081
  auth:
    header: X-Custom-Auth
  tokens:
    - name: root
      group: admin
      token: root-token-0000000000000000000000000
    - name: reader
      group: user
      token: reader-token-00000000000000000000000
  access:
    list: ["admin"]
    read: public
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 2Mi
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    timeout: 45
    body_limit: 512Ki
    replace: 0.25
    resolve:
      type: default
    access:
      read: ["reader"]
    headers:
      Authorization: "Bearer upstream"
      X-Trace: "on"
    loss:
      percentage: 0.1
      status: 503
    latency:
      percentage: 0.5
      min: 0.05
      max: 0.2
    mocks:
      - name: first
        request:
          method: GET
          url: /widgets/(?P<id>\d+)/
          headers:
            request_id: X-Request-ID
          query:
            filter: .filter
          body:
            items: .content.items
        response:
          status: 200
          json: '{"id": "{{ id }}"}'
          headers:
            X-Widget: "{{ id }}"
        proxy:
          replace: 0.75
          latency:
            percentage: 1.0
            min: 0.0
            max: 0.1
      - name: second
        request:
          method: DELETE
          url: /widgets/(?P<id>\d+)/
        response:
          status: 204
  - name: beta
    type: http
    url: "https://beta.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
"#;

fn parse(yaml: &str) -> doppel_core::Config {
    doppel_core::config::load_from_str(yaml).expect("the fixture parses")
}

/// Write a configuration into the tables the long way, so `load` is tested
/// against rows this test wrote rather than against `save`'s idea of them.
///
/// "The long way" is much shorter than it was: both tables hold a JSON
/// document, so this plants the same two shapes `load` reads instead of
/// interpolating a column per field. The point is unchanged -- these are rows
/// the store did not produce, so `load` can be wrong in isolation and be
/// caught.
///
/// Dollar quoting rather than escaped apostrophes: a document contains `"` and
/// may contain `'`, and `$tag$` sidesteps both.
async fn insert(schema: &TestSchema, name: &str, config: &doppel_core::Config, revision: i64) {
    let mut settings = serde_json::to_value(config).expect("a config serializes");
    settings
        .as_object_mut()
        .expect("a config is an object")
        .remove("proxies");

    schema
        .execute(&format!(
            "INSERT INTO configurations (name, revision, settings) \
             VALUES ('{name}', {revision}, $tag${settings}$tag$)"
        ))
        .await;

    for (ordinal, proxy) in config.proxies.iter().enumerate() {
        let document = serde_json::to_value(proxy).expect("a proxy serializes");
        schema
            .execute(&format!(
                "INSERT INTO proxies (config, name, ordinal, document) \
                 VALUES ('{name}', '{}', {ordinal}, $tag${document}$tag$)",
                proxy.name
            ))
            .await;
    }
}

#[tokio::test]
async fn a_configuration_loads_with_every_field_intact() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    let expected = parse(FIXTURE);
    let revision = Revision::of_config(&expected);
    insert(
        &schema,
        "default",
        &expected,
        i64::from_ne_bytes(revision.0.to_ne_bytes()),
    )
    .await;

    let store = PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect");
    let (loaded, loaded_revision) = store.load_config().await.expect("load");

    // Compared whole rather than field by field: a field this test forgot to
    // list is exactly the field `load` is most likely to have dropped.
    assert_eq!(loaded, expected);
    assert_eq!(loaded_revision, revision);

    schema.drop().await;
}

#[tokio::test]
async fn document_order_survives_a_round_trip() {
    // Mock patterns are unanchored, so a general one placed before a specific
    // one shadows it. Reordering on load would change which mock answers a
    // request, silently.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    let expected = parse(FIXTURE);
    let revision = Revision::of_config(&expected);
    insert(
        &schema,
        "default",
        &expected,
        i64::from_ne_bytes(revision.0.to_ne_bytes()),
    )
    .await;

    let store = PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect");
    let (loaded, _) = store.load_config().await.expect("load");

    let names: Vec<_> = loaded.proxies.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alpha", "beta"]);
    let mocks: Vec<_> = loaded.proxies[0]
        .mocks
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(mocks, ["first", "second"]);

    schema.drop().await;
}

#[tokio::test]
async fn a_revision_that_disagrees_with_its_rows_is_refused() {
    // Serving it would break every compare-and-swap downstream in a way that
    // looks like contention rather than like corruption.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    let config = parse(FIXTURE);
    insert(&schema, "default", &config, 12345).await;

    let store = PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect");
    let err = store
        .load_config()
        .await
        .expect_err("a mismatched revision must be refused");
    let message = format!("{err:?}");
    assert!(message.contains("diverged"), "{message}");

    schema.drop().await;
}

#[tokio::test]
async fn an_unknown_configuration_name_is_not_found() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    let store = PostgresStore::connect(&schema.url(), "no-such-config", schema.templates_dir())
        .await
        .expect("connect");
    let err = store.load_config().await.expect_err("must be NotFound");
    assert!(
        matches!(err, doppel_core::store::StoreError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );

    schema.drop().await;
}

#[tokio::test]
async fn a_half_written_latency_is_reported_rather_than_read_as_absent() {
    // `latency` is an optional section with three required fields. A stored
    // object missing one is something nothing could have written through
    // `save`, so reading it as "no latency configured" -- or as a zero -- would
    // serve a configuration nobody wrote.
    //
    // This used to be three columns, where the same hazard was a row with some
    // of them set. The shape changed; the hazard did not, so neither did the
    // test.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    schema.migrate().await;

    let config = parse(FIXTURE);
    let revision = Revision::of_config(&config);
    insert(
        &schema,
        "default",
        &config,
        i64::from_ne_bytes(revision.0.to_ne_bytes()),
    )
    .await;
    schema
        .execute("UPDATE proxies SET document = document #- '{latency,max}' WHERE name = 'alpha'")
        .await;

    let store = PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect");
    let err = store.load_config().await.expect_err("must be refused");
    let message = format!("{err:?}");
    // The field serde could not read, and the proxy whose row held it. Either
    // alone leaves an operator guessing: `max` does not say where, and the proxy
    // name does not say what.
    assert!(message.contains("max"), "{message}");
    assert!(message.contains("alpha"), "{message}");

    schema.drop().await;
}
