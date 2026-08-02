//! Shared harness for the admin API integration tests.
//!
//! Every file under `tests/` compiles this module separately, so a helper
//! used by one file is genuinely dead code in another. The allow is scoped to
//! this module rather than to individual items so it does not have to be
//! maintained as files come and go.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use doppel_admin::AdminState;
use doppel_core::store::{ConfigStore, FileStore, Revision, StoreError, TemplateFile};
use doppel_core::{Config, Runtime, RuntimeHolder};
use tempfile::TempDir;
use tokio::sync::Mutex;
use tower::ServiceExt;

/// A configuration with two proxies, one admin token and one token that
/// holds no write rights. Tests that need something else edit it through the
/// API rather than hand-writing a second document.
pub const BASE_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 18080
admin:
  host: "127.0.0.1"
  port: 18081
  tokens:
    - name: root
      group: admin
      token: root-token
    - name: reader
      group: user
      token: reader-token
  access:
    list: public
    read: public
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 1Mi
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
  - name: beta
    type: http
    url: "https://beta.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
"#;

/// A configuration whose `alpha` proxy declares two template files, and whose
/// upload limit is small enough to exceed in a test without allocating
/// anything large.
pub const TEMPLATE_CONFIG: &str = r#"
server:
  host: "127.0.0.1"
  port: 18080
admin:
  host: "127.0.0.1"
  port: 18081
  tokens:
    - name: root
      group: admin
      token: root-token
    - name: reader
      group: user
      token: reader-token
  access:
    list: public
    read: public
    create: ["admin"]
    update: ["admin"]
    delete: ["admin"]
    upload: ["admin"]
  upload:
    limit: 64
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
    mocks:
      - name: one
        request:
          method: GET
          url: /one/
        response:
          status: 200
          template: one.json.j2
      - name: two
        request:
          method: GET
          url: /two/
        response:
          status: 200
          template: two.json.j2
  - name: beta
    type: http
    url: "https://beta.example.com/api/"
    resolve:
      type: header
      header: X-Proxy-Name
"#;

pub struct Harness {
    /// Held for its Drop: the temporary directory disappears with it.
    _dir: TempDir,
    pub config_path: PathBuf,
    pub templates_dir: PathBuf,
    pub store: Arc<dyn ConfigStore>,
    /// The runtime the process would be serving from. Built from the same
    /// document the store starts with, so `/status` and the store agree
    /// until a test makes them disagree on purpose.
    pub holder: Arc<RuntimeHolder>,
    pub startup: Arc<Config>,
    pub reload_lock: Arc<Mutex<()>>,
    /// A recorder of this harness's own, never the global one: a global
    /// recorder is process-wide, so every test would see every other test's
    /// counters and the assertions would depend on execution order.
    pub recorder: Arc<metrics_exporter_prometheus::PrometheusRecorder>,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_config(BASE_CONFIG)
    }

    pub fn with_config(yaml: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("main.yaml");
        let templates_dir = dir.path().join("templates");
        std::fs::write(&config_path, yaml).expect("write config");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        let store: Arc<dyn ConfigStore> = Arc::new(FileStore::new(&config_path, &templates_dir));

        let startup = Arc::new(
            doppel_core::config::load_from_path(&config_path).expect("harness config parses"),
        );
        let revision = Revision::of_config(&startup);
        let holder = Arc::new(RuntimeHolder::new(
            Runtime::compile(Arc::clone(&startup), revision).expect("harness config compiles"),
        ));

        Self {
            _dir: dir,
            config_path,
            templates_dir,
            store,
            holder,
            startup,
            reload_lock: Arc::new(Mutex::new(())),
            recorder: Arc::new(doppel_core::metrics::build().expect("recorder builds")),
        }
    }

    /// Replace the stored document without touching the running runtime, so
    /// a test can make the two disagree and then reload.
    pub fn overwrite_config(&self, yaml: &str) {
        std::fs::write(&self.config_path, yaml).expect("overwrite config");
    }

    /// Replace the store with one wrapping it, for the concurrency tests.
    pub fn wrap_store(&mut self, wrap: impl FnOnce(Arc<dyn ConfigStore>) -> Arc<dyn ConfigStore>) {
        let inner = Arc::clone(&self.store);
        self.store = wrap(inner);
    }

    pub fn router(&self) -> Router {
        doppel_admin::router(AdminState::new(
            Arc::clone(&self.store),
            Arc::clone(&self.holder),
            Arc::clone(&self.startup),
            Arc::clone(&self.reload_lock),
            self.recorder.handle(),
            Instant::now(),
        ))
    }

    pub fn template_path(&self, proxy: &str, file: &str) -> PathBuf {
        self.templates_dir.join(proxy).join(file)
    }

    pub fn write_template(&self, proxy: &str, file: &str, body: &str) {
        let dir = self.templates_dir.join(proxy);
        std::fs::create_dir_all(&dir).expect("create proxy template dir");
        std::fs::write(dir.join(file), body).expect("write template");
    }

    /// The configuration currently on disk, parsed. Tests assert against this
    /// rather than against a response body when they mean "the write landed".
    pub fn stored(&self) -> Config {
        doppel_core::config::load_from_path(&self.config_path).expect("stored config parses")
    }
}

/// A request builder that is shorter than axum's at the call site.
pub struct Call {
    method: &'static str,
    uri: String,
    token: Option<&'static str>,
    if_match: Option<String>,
    body: Option<String>,
}

impl Call {
    pub fn get(uri: impl Into<String>) -> Self {
        Self::new("GET", uri)
    }

    pub fn post(uri: impl Into<String>) -> Self {
        Self::new("POST", uri)
    }

    pub fn put(uri: impl Into<String>) -> Self {
        Self::new("PUT", uri)
    }

    pub fn delete(uri: impl Into<String>) -> Self {
        Self::new("DELETE", uri)
    }

    fn new(method: &'static str, uri: impl Into<String>) -> Self {
        Self {
            method,
            uri: uri.into(),
            token: None,
            if_match: None,
            body: None,
        }
    }

    #[must_use]
    pub fn token(mut self, token: &'static str) -> Self {
        self.token = Some(token);
        self
    }

    #[must_use]
    pub fn if_match(mut self, value: impl Into<String>) -> Self {
        self.if_match = Some(value.into());
        self
    }

    #[must_use]
    pub fn json(mut self, body: serde_json::Value) -> Self {
        self.body = Some(body.to_string());
        self
    }

    /// A body that is not necessarily valid JSON, for the malformed cases.
    #[must_use]
    pub fn raw(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub async fn send(self, router: Router) -> Reply {
        let mut builder = Request::builder().method(self.method).uri(&self.uri);
        if let Some(token) = self.token {
            builder = builder.header("X-Proxy-Authorization", format!("Bearer {token}"));
        }
        if let Some(value) = &self.if_match {
            builder = builder.header("If-Match", value);
        }
        let body = match self.body {
            Some(body) => {
                // Set explicitly: `Request::builder` does not derive it, and
                // without it the server's announced-length check is never
                // reached, so a bug in it would go unseen by every test here
                // while every real client sends the header.
                builder = builder
                    .header("Content-Type", "application/json")
                    .header("Content-Length", body.len().to_string());
                Body::from(body)
            }
            None => Body::empty(),
        };
        let request = builder.body(body).expect("build request");
        let response = router.oneshot(request).await.expect("router responds");
        Reply::from_response(response).await
    }
}

/// A response with its body already collected, so assertions do not have to
/// be async.
pub struct Reply {
    pub status: StatusCode,
    pub etag: Option<String>,
    pub location: Option<String>,
    pub content_type: Option<String>,
    pub allow: Option<String>,
    pub body: String,
}

impl Reply {
    async fn from_response(response: Response<Body>) -> Self {
        let status = response.status();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(ToOwned::to_owned)
        };
        let etag = header("etag");
        let location = header("location");
        let content_type = header("content-type");
        let allow = header("allow");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("collect body");
        Self {
            status,
            etag,
            location,
            content_type,
            allow,
            body: String::from_utf8(bytes.to_vec()).expect("body is utf-8"),
        }
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("body is not JSON ({err}): {}", self.body))
    }

    /// The error envelope's `code`, asserting the envelope shape along the
    /// way: an error response that is missing `status: error` is a bug even
    /// if its code is right.
    pub fn error_code(&self) -> String {
        let json = self.json();
        assert_eq!(
            json.get("status").and_then(serde_json::Value::as_str),
            Some("error"),
            "error envelope must carry status=error: {}",
            self.body
        );
        assert!(
            json.get("message")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|m| !m.is_empty()),
            "error envelope must carry a non-empty message: {}",
            self.body
        );
        json.get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("error envelope must carry a code: {}", self.body))
            .to_owned()
    }

    pub fn revision(&self) -> String {
        self.json()
            .get("revision")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("body has no revision: {}", self.body))
            .to_owned()
    }
}

/// A valid proxy body, for create and update.
///
/// Every proxy in these tests resolves by header rather than by being the
/// default: at most one proxy may be the default, so a helper that produced
/// default-resolving proxies could not be used twice in one configuration.
pub fn proxy_json(name: &str, url: &str) -> serde_json::Value {
    serde_json::json!({
        "proxy": {
            "name": name,
            "type": "http",
            "url": url,
            "resolve": { "type": "header", "header": "X-Proxy-Name" },
        }
    })
}

/// Wraps a store and, before the first `save` calls delegated to it, writes a
/// competing change straight through the inner store. That makes the
/// handler's compare-and-swap fail exactly once, which is what the retry
/// path exists for.
///
/// `edits` is consumed one entry per `save`: an entry of `Some(f)` runs `f`
/// against the inner store first, `None` lets the save through untouched.
/// Driving it from a list rather than a counter keeps each test's intent
/// ("collide once, then succeed") visible at the construction site.
pub struct RacingStore {
    inner: Arc<dyn ConfigStore>,
    edits: std::sync::Mutex<std::vec::IntoIter<Option<Edit>>>,
}

/// What a racing writer does to the stored configuration. Applied through the
/// inner store's own `save`, so it produces a genuine revision change rather
/// than a simulated one.
pub type Edit = Box<dyn Fn(&mut doppel_core::Config) + Send + Sync>;

impl RacingStore {
    pub fn new(inner: Arc<dyn ConfigStore>, edits: Vec<Option<Edit>>) -> Self {
        Self {
            inner,
            edits: std::sync::Mutex::new(edits.into_iter()),
        }
    }

    /// An edit that retitles a proxy's upstream, changing both that proxy's
    /// revision and the whole-config one.
    pub fn touch(proxy: &'static str, url: &'static str) -> Option<Edit> {
        Some(Box::new(move |config: &mut doppel_core::Config| {
            for candidate in &mut config.proxies {
                if candidate.name == proxy {
                    candidate.url = url.parse().expect("test url parses");
                }
            }
        }))
    }
}

#[async_trait::async_trait]
impl ConfigStore for RacingStore {
    async fn load(&self) -> Result<(doppel_core::Config, Revision), StoreError> {
        self.inner.load().await
    }

    async fn save(
        &self,
        config: &doppel_core::Config,
        expected: Option<Revision>,
    ) -> Result<Revision, StoreError> {
        let edit = self.edits.lock().expect("edits lock").next().flatten();
        if let Some(edit) = edit {
            let (mut current, current_rev) = self.inner.load().await?;
            edit(&mut current);
            self.inner.save(&current, Some(current_rev)).await?;
        }
        self.inner.save(config, expected).await
    }

    async fn load_templates(&self, proxy: &str) -> Result<Vec<TemplateFile>, StoreError> {
        self.inner.load_templates(proxy).await
    }

    async fn save_template(&self, proxy: &str, file: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.inner.save_template(proxy, file, bytes).await
    }

    async fn delete_template(&self, proxy: &str, file: &str) -> Result<bool, StoreError> {
        self.inner.delete_template(proxy, file).await
    }

    async fn retain_templates(&self, proxy: &str, keep: &[String]) -> Result<(), StoreError> {
        self.inner.retain_templates(proxy, keep).await
    }
}

/// Asserts a path does not exist, with a message naming it.
pub fn assert_absent(path: &Path) {
    assert!(!path.exists(), "expected {} to be gone", path.display());
}
