//! Reading a configuration out of the tables.

mod common;

use common::{TestSchema, require_database};
use doppel_core::store::Revision;
use doppel_store_postgres::PostgresStore;

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
      token: root-token
    - name: reader
      group: user
      token: reader-token
  access:
    list: ["admin"]
    read: public
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 2M
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    timeout: 45
    body_limit: 512K
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
            requestId: X-Request-ID
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
/// Once `save` exists the conformance suite covers the pair; until then this
/// is the only way `load` can be wrong in isolation and be caught.
async fn insert(schema: &TestSchema, name: &str, config: &doppel_core::Config, revision: i64) {
    let admin = &config.admin;
    schema
        .execute(&format!(
            "INSERT INTO configurations (name, revision, server_host, server_port, log_level, \
             log_format, control_socket, templates_dir, sentry_dsn, admin_host, admin_port, \
             admin_auth_header, admin_upload_limit, admin_access) VALUES \
             ('{name}', {revision}, '{}', {}, '{}', '{}', '{}', '{}', {}, '{}', {}, '{}', {}, '{}')",
            config.server.host,
            config.server.port,
            serde_json::to_value(config.logging.level).unwrap().as_str().unwrap(),
            serde_json::to_value(config.logging.format).unwrap().as_str().unwrap(),
            config.control.socket.display(),
            config.templates.dir.display(),
            config
                .sentry
                .as_ref()
                .map_or("NULL".to_owned(), |s| format!("'{}'", s.dsn)),
            admin.host,
            admin.port,
            admin.auth.header,
            admin.upload.limit.0,
            serde_json::to_string(&admin.access).unwrap(),
        ))
        .await;

    for (ordinal, token) in admin.tokens.iter().enumerate() {
        schema
            .execute(&format!(
                "INSERT INTO admin_tokens (config, name, \"group\", token, ordinal) \
                 VALUES ('{name}', '{}', '{}', '{}', {ordinal})",
                token.name, token.group, token.token
            ))
            .await;
    }

    for (ordinal, proxy) in config.proxies.iter().enumerate() {
        let nullable = |value: Option<String>| value.unwrap_or_else(|| "NULL".to_owned());
        schema
            .execute(&format!(
                "INSERT INTO proxies (config, name, ordinal, kind, url, timeout_seconds, \
                 body_limit, replace_ratio, resolve_kind, resolve_header, loss_percentage, \
                 loss_status, latency_percentage, latency_min, latency_max, headers, access) \
                 VALUES ('{name}', '{}', {ordinal}, '{}', '{}', {}, {}, {}, '{}', {}, {}, {}, \
                 {}, {}, {}, '{}', {})",
                proxy.name,
                serde_json::to_value(proxy.kind).unwrap().as_str().unwrap(),
                proxy.url,
                nullable(proxy.timeout.map(|t| t.to_string())),
                proxy.body_limit.0,
                nullable(proxy.replace.map(|r| r.to_string())),
                serde_json::to_value(proxy.resolve.kind)
                    .unwrap()
                    .as_str()
                    .unwrap(),
                nullable(proxy.resolve.header.as_ref().map(|h| format!("'{h}'"))),
                nullable(proxy.loss.as_ref().map(|l| l.percentage.to_string())),
                nullable(proxy.loss.as_ref().map(|l| l.status.to_string())),
                nullable(proxy.latency.as_ref().map(|l| l.percentage.to_string())),
                nullable(proxy.latency.as_ref().map(|l| l.min.to_string())),
                nullable(proxy.latency.as_ref().map(|l| l.max.to_string())),
                serde_json::to_string(&proxy.headers).unwrap(),
                nullable(
                    proxy
                        .access
                        .as_ref()
                        .map(|a| format!("'{}'", serde_json::to_string(a).unwrap()))
                ),
            ))
            .await;

        for (mock_ordinal, mock) in proxy.mocks.iter().enumerate() {
            schema
                .execute(&format!(
                    "INSERT INTO mocks (config, proxy, name, ordinal, method, url_pattern, \
                     status, body, json, template, request_headers, request_query, \
                     request_body, response_headers, proxy_override) VALUES \
                     ('{name}', '{}', '{}', {mock_ordinal}, '{}', $tag${}$tag$, {}, {}, {}, {}, \
                     '{}', '{}', '{}', '{}', {})",
                    proxy.name,
                    mock.name,
                    mock.request.method,
                    mock.request.url,
                    mock.response.status,
                    nullable(
                        mock.response
                            .body
                            .as_ref()
                            .map(|b| format!("$tag${b}$tag$"))
                    ),
                    nullable(
                        mock.response
                            .json
                            .as_ref()
                            .map(|j| format!("$tag${j}$tag$"))
                    ),
                    nullable(mock.response.template.as_ref().map(|t| format!("'{t}'"))),
                    serde_json::to_string(&mock.request.headers).unwrap(),
                    serde_json::to_string(&mock.request.query).unwrap(),
                    serde_json::to_string(&mock.request.body).unwrap(),
                    serde_json::to_string(&mock.response.headers).unwrap(),
                    nullable(
                        mock.proxy
                            .as_ref()
                            .map(|p| format!("'{}'", serde_json::to_string(p).unwrap()))
                    ),
                ))
                .await;
        }
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
    // Three columns hold one optional section. A row with some of them set is
    // one nothing could have written through `save`, so treating it as "no
    // latency configured" would serve a configuration nobody wrote.
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
        .execute("UPDATE proxies SET latency_max = NULL WHERE name = 'alpha'")
        .await;

    let store = PostgresStore::connect(&schema.url(), "default", schema.templates_dir())
        .await
        .expect("connect");
    let err = store.load_config().await.expect_err("must be refused");
    let message = format!("{err:?}");
    assert!(message.contains("latency"), "{message}");

    schema.drop().await;
}
