//! The 0005 backfill, held to the one property that matters.
//!
//! Migration 0005 replaces a column per configuration field with two JSON
//! documents. The revision of a stored configuration is a hash of its canonical
//! YAML, and every write compare-and-swaps against it -- so if the JSON the
//! backfill builds parsed into a configuration that differed from the one the
//! old columns described, in any way at all, the revision would move. The next
//! write from a client holding the revision it had read would be refused as a
//! conflict that never happened, and the operator would see contention where
//! there was corruption.
//!
//! So this suite plants rows in the pre-0005 shape, applies the migration file,
//! and checks that what comes out parses back into exactly the configuration
//! that went in -- and that its revision is unchanged.
//!
//! It does not go through `PostgresStore`: `connect` refuses a schema whose
//! `_sqlx_migrations` bookkeeping is incomplete, and a test that applies
//! migration files by hand cannot forge that bookkeeping, because sqlx
//! checksums it. The store's own read path is covered by `load.rs` and
//! `conformance.rs`; what is under test here is the SQL.

use doppel_core::config::{Config, ProxyConfig};
use doppel_core::store::Revision;
use doppel_store_postgres::test_support::{TestSchema, require_database};

/// The schema as it stood before 0005, applied in order.
const BEFORE: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_proxy_rewrite_redirects.sql"),
    include_str!("../migrations/0003_admin_groups.sql"),
    include_str!("../migrations/0004_admin_public.sql"),
];

const JSONB: &str = include_str!("../migrations/0005_settings_as_jsonb.sql");

/// A configuration touching every column 0005 has to carry across: a sentry
/// section, tokens, a group allow-list, a non-default `enable` and auth header,
/// both resolve kinds, both fault sections, a replace ratio, an explicit
/// `rewrite_redirects`, a per-proxy access override, all three mock body forms,
/// a mock-level proxy override, all three selector maps, and a proxy with no
/// mocks at all.
///
/// Deliberately without `dashboard` and `title`: this is what a configuration
/// stored before those fields existed looks like, which is the only kind of row
/// a backfill can encounter.
const FIXTURE: &str = r#"
server:
  host: "127.0.0.1"
  port: 18080
logging:
  level: debug
  format: text
control:
  socket: /tmp/doppel-migrate.sock
templates:
  dir: /tmp/doppel-migrate-templates
sentry:
  dsn: "https://key@sentry.example.com/1"
admin:
  enable: false
  host: "127.0.0.1"
  port: 18081
  auth:
    header: X-Admin-Token
  public: false
  groups: ["admin", "ci"]
  tokens:
    - name: ci
      group: ci
      token: "0123456789abcdef0123456789abcdef"
  access:
    list: public
    read: [ci]
    create: [admin]
    update: [admin]
    delete: [admin]
    upload: [admin]
  upload:
    limit: 2Mi
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    timeout: 12
    body_limit: 4Mi
    replace: 0.25
    rewrite_redirects: false
    headers:
      X-Injected: "yes"
    access:
      read: [ci]
    loss:
      percentage: 0.055
      status: 503
    latency:
      percentage: 0.5
      min: 0.1
      max: 1.5
    mocks:
      - name: first
        request:
          method: POST
          url: "^/widgets$"
          headers:
            trace: X-Trace-Id
          query:
            filter: ".filter"
          body:
            items: ".content.items"
        response:
          status: 201
          json: '{"ok": true}'
          headers:
            X-Mocked: "first"
        proxy:
          replace: 1.0
          latency:
            percentage: 1.0
            min: 0.2
            max: 0.4
      - name: second
        request:
          method: GET
          url: "^/health$"
        response:
          status: 200
          body: "ok"
      - name: third
        request:
          method: GET
          url: "^/rendered$"
        response:
          status: 200
          template: page.json.j2
  - name: beta
    type: http
    url: "https://beta.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
"#;

#[tokio::test]
async fn the_backfill_preserves_every_stored_revision() {
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    for migration in BEFORE {
        schema.execute(migration).await;
    }

    let expected = doppel_core::config::load_from_str(FIXTURE).expect("the fixture parses");
    let revision = Revision::of_config(&expected);
    insert_pre_0005(&schema, "default", &expected, as_i64(revision)).await;

    schema.execute(JSONB).await;

    // Reassembled the way `load` does: the settings document holds everything
    // but the proxies, which come from their own rows in `ordinal` order.
    let settings = schema
        .json_rows("SELECT settings FROM configurations WHERE name = 'default'")
        .await;
    let mut loaded: Config =
        serde_json::from_value(settings[0].clone()).expect("the backfilled settings parse");
    loaded.proxies = schema
        .json_rows("SELECT document FROM proxies WHERE config = 'default' ORDER BY ordinal")
        .await
        .into_iter()
        .map(|document| {
            serde_json::from_value::<ProxyConfig>(document).expect("a backfilled proxy parses")
        })
        .collect();

    // Compared whole rather than field by field: a field this test forgot to
    // list is exactly the field the backfill is most likely to have dropped.
    assert_eq!(loaded, expected);
    assert_eq!(
        Revision::of_config(&loaded),
        revision,
        "the backfill changed a stored revision, which would refuse the next write as a conflict"
    );

    // And the tables the documents replaced are gone, so nothing can read them
    // and quietly disagree with the document.
    for table in ["admin_tokens", "mocks"] {
        let present: Vec<serde_json::Value> = schema
            .json_rows(&format!(
                "SELECT to_jsonb(count(*)) FROM information_schema.tables \
                 WHERE table_schema = current_schema() AND table_name = '{table}'"
            ))
            .await;
        assert_eq!(present[0], serde_json::json!(0), "{table} still exists");
    }

    schema.drop().await;
}

#[tokio::test]
async fn a_configuration_with_no_tokens_no_sentry_and_no_mocks_survives() {
    // The absent cases are where a backfill goes wrong: an empty token list has
    // to stay `[]` rather than becoming null, and `sentry` has a required field,
    // so `{"dsn": null}` stripped down to `{}` would fail to parse -- which is
    // why that section is built conditionally rather than stripped.
    let Some(url) = require_database() else {
        return;
    };
    let schema = TestSchema::create(&url).await;
    for migration in BEFORE {
        schema.execute(migration).await;
    }

    let bare = doppel_core::config::load_from_str(
        r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens: []
  access: {}
  upload:
    limit: 1Mi
proxies:
  - name: solo
    type: http
    url: "https://example.com/"
"#,
    )
    .expect("the fixture parses");
    let revision = Revision::of_config(&bare);
    insert_pre_0005(&schema, "default", &bare, as_i64(revision)).await;

    schema.execute(JSONB).await;

    let settings = schema
        .json_rows("SELECT settings FROM configurations WHERE name = 'default'")
        .await;
    let mut loaded: Config =
        serde_json::from_value(settings[0].clone()).expect("the backfilled settings parse");
    assert!(loaded.sentry.is_none(), "sentry must stay absent");
    assert!(loaded.admin.tokens.is_empty());

    loaded.proxies = schema
        .json_rows("SELECT document FROM proxies WHERE config = 'default' ORDER BY ordinal")
        .await
        .into_iter()
        .map(|document| serde_json::from_value::<ProxyConfig>(document).expect("a proxy parses"))
        .collect();
    assert_eq!(loaded, bare);
    assert_eq!(Revision::of_config(&loaded), revision);

    schema.drop().await;
}

fn as_i64(revision: Revision) -> i64 {
    i64::from_ne_bytes(revision.0.to_ne_bytes())
}

/// Write a configuration in the shape the schema had before 0005.
///
/// Lifted from the version of `tests/load.rs` that planted rows this way, which
/// is why it interpolates SQL by hand: the whole point is to produce rows the
/// current code cannot write, so it cannot go through `save`.
#[allow(clippy::too_many_lines)]
async fn insert_pre_0005(schema: &TestSchema, name: &str, config: &Config, revision: i64) {
    let admin = &config.admin;
    let nullable = |value: Option<String>| value.unwrap_or_else(|| "NULL".to_owned());

    schema
        .execute(&format!(
            "INSERT INTO configurations (name, revision, server_host, server_port, log_level, \
             log_format, control_socket, templates_dir, sentry_dsn, admin_enable, admin_host, \
             admin_port, admin_auth_header, admin_upload_limit, admin_access, admin_public, \
             admin_groups) VALUES \
             ('{name}', {revision}, '{}', {}, '{}', '{}', '{}', '{}', {}, {}, '{}', {}, '{}', {}, \
             '{}', {}, {})",
            config.server.host,
            config.server.port,
            serde_json::to_value(config.logging.level)
                .unwrap()
                .as_str()
                .unwrap(),
            serde_json::to_value(config.logging.format)
                .unwrap()
                .as_str()
                .unwrap(),
            config.control.socket.display(),
            config.templates.dir.display(),
            nullable(config.sentry.as_ref().map(|s| format!("'{}'", s.dsn))),
            admin.enable,
            admin.host,
            admin.port,
            admin.auth.header,
            admin.upload.limit.get(),
            serde_json::to_string(&admin.access).unwrap(),
            nullable(admin.public.map(|p| p.to_string())),
            nullable(
                admin
                    .groups
                    .as_ref()
                    .map(|g| format!("'{}'", serde_json::to_string(g).unwrap()))
            ),
        ))
        .await;

    for (ordinal, token) in admin.tokens.iter().enumerate() {
        schema
            .execute(&format!(
                "INSERT INTO admin_tokens (config, name, \"group\", token, ordinal) \
                 VALUES ('{name}', '{}', '{}', '{}', {ordinal})",
                // `as_str`, not `{}`: `Token`'s `Display` is redacted, so
                // interpolating it would write the literal `<redacted>` into
                // every row.
                token.name,
                token.group,
                token.token.as_str()
            ))
            .await;
    }

    for (ordinal, proxy) in config.proxies.iter().enumerate() {
        schema
            .execute(&format!(
                "INSERT INTO proxies (config, name, ordinal, kind, url, timeout_seconds, \
                 body_limit, replace_ratio, rewrite_redirects, resolve_kind, resolve_header, \
                 loss_percentage, loss_status, latency_percentage, latency_min, latency_max, \
                 headers, access) VALUES ('{name}', '{}', {ordinal}, '{}', '{}', {}, {}, {}, {}, \
                 '{}', {}, {}, {}, {}, {}, {}, '{}', {})",
                proxy.name,
                serde_json::to_value(proxy.kind).unwrap().as_str().unwrap(),
                proxy.url,
                nullable(proxy.timeout.map(|t| t.to_string())),
                proxy.body_limit.get(),
                nullable(proxy.replace.map(|r| r.to_string())),
                nullable(proxy.rewrite_redirects.map(|r| r.to_string())),
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
