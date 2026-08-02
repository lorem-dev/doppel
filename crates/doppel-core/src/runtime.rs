//! The configuration compiled into the form the request path needs, and the
//! holder that swaps it atomically on reload.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::{ArcSwap, Guard};

use crate::config::{Config, LatencyConfig, LossConfig, MockConfig, ProxyConfig, ResolveKind};
use crate::store::Revision;
use crate::{Error, ErrorCode};

/// Upstream timeout when a proxy does not set one.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A proxy with everything the hot path needs already parsed.
#[derive(Debug, Clone)]
pub struct CompiledProxy {
    pub name: String,
    pub base_url: reqwest::Url,
    pub timeout: Duration,
    /// Lowercased header name and its value, ready to inject upstream.
    pub headers: Vec<(String, String)>,
    pub loss: Option<LossConfig>,
    pub latency: Option<LatencyConfig>,
    pub replace: f64,
    pub resolve_header: Option<String>,
    /// Mocks in configuration order. Matching is first-wins, so this order
    /// is load-bearing, not incidental.
    pub mocks: Vec<CompiledMock>,
    /// Bytes a body-extracting mock may buffer from the request body before
    /// `UPLOAD_TOO_LARGE`. Always set: the config field this comes from
    /// resolves its own default at parse time.
    pub body_limit: u64,
}

/// A mock with everything the hot path needs already parsed: its pattern
/// compiled, its extraction sources split out by kind, and its response
/// shape resolved to a single variant. Mocks match first-wins (section 4 of
/// the design), so `Runtime::compile` keeps this in the same order as the
/// `mocks` list it came from.
#[derive(Debug, Clone)]
pub struct CompiledMock {
    pub name: String,
    pub method: String,
    pub pattern: regex::Regex,
    /// Named capture groups, in the order the pattern declares them.
    pub capture_names: Vec<String>,
    /// Variable name -> request header name, lowercased so the request path
    /// never has to case-fold a header name again.
    pub header_vars: Vec<(String, String)>,
    /// Variable name -> raw `.a.b` query selector.
    pub query_vars: Vec<(String, String)>,
    /// Variable name -> raw `.a.b` body selector.
    pub body_vars: Vec<(String, String)>,
    pub status: u16,
    pub body: MockBody,
    /// Response header name -> template source, rendered per request.
    pub headers: Vec<(String, String)>,
    pub replace: Option<f64>,
    pub loss: Option<LossConfig>,
    pub latency: Option<LatencyConfig>,
}

/// The response body a mock produces, resolved from whichever of the three
/// mutually exclusive `response` fields the config declared (validation rule
/// V20 guarantees at most one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockBody {
    /// No body field was set; only valid for a bodiless status (rule V30).
    None,
    /// `response.body`: a template rendered to bytes and returned as-is.
    Text(String),
    /// `response.json`: a template whose rendered output is additionally
    /// checked as JSON before being sent.
    Json(String),
    /// `response.template`: names a file under `<templates.dir>/<proxy>/`.
    ///
    /// This carries only the file name, not its contents. Every other field
    /// compiled here obeys "nothing parses on the hot path" -- this is the
    /// one deliberate exception. Phase 3 uploads template files at runtime,
    /// so a mock may legitimately name a file that does not exist yet, and a
    /// config reload has to pick up whatever is on disk *at that moment*,
    /// not whatever existed when the config was last compiled. Reading the
    /// file therefore happens per request (Task 6), where a missing file is
    /// a `TEMPLATE_NOT_FOUND` at request time rather than a reload failure.
    Template(String),
}

/// An immutable snapshot of everything derived from one configuration.
pub struct Runtime {
    pub revision: Revision,
    pub config: Arc<Config>,
    pub proxies: Vec<CompiledProxy>,
    /// Index into `proxies` of the proxy with `resolve.type: default`.
    pub default_proxy: Option<usize>,
    /// Lowercased resolution header names, in configuration order and
    /// deduplicated. Order is specified so that a request carrying two
    /// resolution headers resolves identically on every process and run.
    pub resolve_headers: Vec<String>,
    pub client: reqwest::Client,
}

impl Runtime {
    /// Compile a validated config. Assumes `crate::validate::validate` already
    /// passed; anything it would have caught is reported here as a 500-class
    /// error rather than silently ignored.
    pub fn compile(config: Arc<Config>, revision: Revision) -> Result<Self, Error> {
        let mut proxies = Vec::with_capacity(config.proxies.len());
        let mut default_proxy = None;
        let mut resolve_headers: Vec<String> = Vec::new();

        for (index, proxy) in config.proxies.iter().enumerate() {
            let compiled = compile_proxy(proxy)?;
            if proxy.resolve.kind == ResolveKind::Default && default_proxy.is_none() {
                default_proxy = Some(index);
            }
            if let Some(header) = &compiled.resolve_header
                && !resolve_headers.contains(header)
            {
                resolve_headers.push(header.clone());
            }
            proxies.push(compiled);
        }

        // A forwarding proxy must relay a redirect, not resolve it: the
        // upstream's `3xx` and `Location` belong to the original caller, and
        // a streamed request body cannot be replayed to the redirect target
        // the way reqwest's default following behaviour requires.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                Error::new(
                    ErrorCode::UpstreamError,
                    format!("cannot build http client: {e}"),
                )
            })?;

        Ok(Self {
            revision,
            config,
            proxies,
            default_proxy,
            resolve_headers,
            client,
        })
    }

    #[must_use]
    pub fn proxy_by_name(&self, name: &str) -> Option<&CompiledProxy> {
        self.proxies.iter().find(|p| p.name == name)
    }

    #[must_use]
    pub fn default(&self) -> Option<&CompiledProxy> {
        self.default_proxy.map(|i| &self.proxies[i])
    }
}

fn compile_proxy(proxy: &ProxyConfig) -> Result<CompiledProxy, Error> {
    let base_url = reqwest::Url::parse(&proxy.url).map_err(|e| {
        Error::new(
            ErrorCode::ConfigInvalid,
            format!("proxy `{}` has an unusable url: {e}", proxy.name),
        )
    })?;

    let mut mocks = Vec::with_capacity(proxy.mocks.len());
    for mock in &proxy.mocks {
        mocks.push(compile_mock(mock, proxy.name.as_str())?);
    }

    Ok(CompiledProxy {
        name: proxy.name.to_string(),
        base_url,
        timeout: proxy.timeout.map_or(DEFAULT_TIMEOUT, Duration::from_secs),
        headers: proxy
            .headers
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .collect(),
        loss: proxy.loss,
        latency: proxy.latency,
        replace: proxy.replace.unwrap_or(1.0),
        resolve_header: match proxy.resolve.kind {
            ResolveKind::Header => proxy
                .resolve
                .header
                .as_ref()
                .map(|h| h.to_ascii_lowercase()),
            ResolveKind::Default => None,
        },
        mocks,
        body_limit: proxy.body_limit.0,
    })
}

/// Compiles one mock: the regex once, here (a failure is defence in depth
/// behind rule V18, which already rejects an unparseable pattern), the three
/// extraction maps split out by source, and the response resolved to a
/// single `MockBody`. See `MockBody::Template` for why the template case does
/// not read the file it names.
fn compile_mock(mock: &MockConfig, proxy_name: &str) -> Result<CompiledMock, Error> {
    let pattern = regex::Regex::new(&mock.request.url).map_err(|e| {
        Error::new(
            ErrorCode::ConfigInvalid,
            format!(
                "proxy `{proxy_name}` mock `{}` has an unusable url pattern: {e}",
                mock.name
            ),
        )
    })?;

    let capture_names = pattern
        .capture_names()
        .flatten()
        .map(str::to_owned)
        .collect();

    let header_vars = mock
        .request
        .headers
        .iter()
        .map(|(var, header)| (var.clone(), header.to_ascii_lowercase()))
        .collect();
    let query_vars = mock
        .request
        .query
        .iter()
        .map(|(var, selector)| (var.clone(), selector.clone()))
        .collect();
    let body_vars = mock
        .request
        .body
        .iter()
        .map(|(var, selector)| (var.clone(), selector.clone()))
        .collect();

    let body = if let Some(text) = &mock.response.body {
        MockBody::Text(text.clone())
    } else if let Some(json) = &mock.response.json {
        MockBody::Json(json.clone())
    } else if let Some(template) = &mock.response.template {
        MockBody::Template(template.clone())
    } else {
        MockBody::None
    };

    let headers = mock
        .response
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let (replace, loss, latency) = match &mock.proxy {
        Some(over) => (over.replace, over.loss, over.latency),
        None => (None, None, None),
    };

    Ok(CompiledMock {
        name: mock.name.to_string(),
        method: mock.request.method.clone(),
        pattern,
        capture_names,
        header_vars,
        query_vars,
        body_vars,
        status: mock.response.status,
        body,
        headers,
        replace,
        loss,
        latency,
    })
}

/// Holds the live runtime. Readers take a guard; a reload replaces the whole
/// value. In-flight requests finish against the runtime they started with.
pub struct RuntimeHolder(ArcSwap<Runtime>);

impl RuntimeHolder {
    #[must_use]
    pub fn new(runtime: Runtime) -> Self {
        Self(ArcSwap::from_pointee(runtime))
    }

    #[must_use]
    pub fn load(&self) -> Guard<Arc<Runtime>> {
        self.0.load()
    }

    pub fn store(&self, runtime: Runtime) {
        self.0.store(Arc::new(runtime));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_from_str;

    const TWO_PROXIES: &str = r#"
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
  - name: p1
    type: http
    url: "https://one.example.com/api/"
    timeout: 5
    headers:
      Authorization: "Bearer x"
    resolve:
      type: header
      header: X-Proxy-Name
  - name: p2
    type: http
    url: "https://two.example.com/"
    resolve:
      type: default
"#;

    fn compile(text: &str) -> Runtime {
        let config = std::sync::Arc::new(load_from_str(text).unwrap());
        Runtime::compile(config, Revision(1)).unwrap()
    }

    #[test]
    fn compiles_proxies_in_config_order() {
        let rt = compile(TWO_PROXIES);
        assert_eq!(rt.proxies[0].name, "p1");
        assert_eq!(rt.proxies[1].name, "p2");
    }

    #[test]
    fn records_the_default_proxy() {
        let rt = compile(TWO_PROXIES);
        assert_eq!(
            rt.default_proxy.map(|i| rt.proxies[i].name.as_str()),
            Some("p2")
        );
    }

    #[test]
    fn collects_resolve_headers_in_config_order_without_duplicates() {
        let text = TWO_PROXIES.replace(
            "  - name: p2",
            "  - name: p3\n    type: http\n    url: \"https://three.example.com/\"\n    resolve:\n      type: header\n      header: X-Proxy-Name\n  - name: p2",
        );
        let rt = compile(&text);
        assert_eq!(rt.resolve_headers, vec!["x-proxy-name".to_owned()]);
    }

    #[test]
    fn no_default_proxy_is_representable() {
        let text = TWO_PROXIES.replace(
            "      type: default",
            "      type: header\n      header: X-Other",
        );
        let rt = compile(&text);
        assert!(rt.default_proxy.is_none());
        assert_eq!(
            rt.resolve_headers,
            vec!["x-proxy-name".to_owned(), "x-other".to_owned()]
        );
    }

    #[test]
    fn header_names_are_lowercased_and_values_precompiled() {
        let rt = compile(TWO_PROXIES);
        let headers = &rt.proxies[0].headers;
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "authorization");
        assert_eq!(headers[0].1, "Bearer x");
    }

    #[test]
    fn timeout_falls_back_to_the_documented_default() {
        let rt = compile(TWO_PROXIES);
        assert_eq!(rt.proxies[0].timeout, std::time::Duration::from_secs(5));
        assert_eq!(rt.proxies[1].timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn replace_defaults_to_one() {
        let rt = compile(TWO_PROXIES);
        assert!((rt.proxies[0].replace - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compile_reports_an_unparseable_proxy_url_rather_than_panicking() {
        // `validate` would normally reject a url like this before `compile`
        // ever sees it, so this builds the `Config` directly rather than
        // through `load_from_str`/validation, to reach `compile_proxy`'s
        // `Url::parse` failure branch -- the second line of defence -- on
        // its own.
        let mut config = load_from_str(TWO_PROXIES).unwrap();
        config.proxies[0].url = "not a url at all".to_owned();

        let err = match Runtime::compile(std::sync::Arc::new(config), Revision(1)) {
            Ok(_) => panic!("expected compile to reject an unparseable proxy url"),
            Err(err) => err,
        };
        assert_eq!(err.code, crate::ErrorCode::ConfigInvalid);
        assert!(
            err.message.contains("p1"),
            "the offending proxy's name should be in the message, got: {}",
            err.message
        );
    }

    #[test]
    fn lookup_by_name_works() {
        let rt = compile(TWO_PROXIES);
        assert!(rt.proxy_by_name("p1").is_some());
        assert!(rt.proxy_by_name("ghost").is_none());
    }

    #[test]
    fn holder_swaps_atomically_and_old_readers_keep_their_value() {
        let holder = RuntimeHolder::new(compile(TWO_PROXIES));
        let before = holder.load();
        assert_eq!(before.revision, Revision(1));

        let mut config = (*before.config).clone();
        config.proxies.truncate(1);
        holder.store(Runtime::compile(std::sync::Arc::new(config), Revision(2)).unwrap());

        // The guard taken before the swap still sees the old runtime.
        assert_eq!(before.proxies.len(), 2);
        assert_eq!(holder.load().proxies.len(), 1);
        assert_eq!(holder.load().revision, Revision(2));
    }

    /// A minimal HTTP/1.1 server, hand rolled instead of pulled in as a
    /// dependency, that always answers with a 302 to prove whether the
    /// client sitting in front of it resolves the redirect itself or hands
    /// it back untouched.
    async fn redirecting_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let response = b"HTTP/1.1 302 Found\r\n\
                Location: http://example.invalid/target\r\n\
                Content-Length: 0\r\n\
                Connection: close\r\n\
                \r\n";
            let _ = stream.write_all(response).await;
            let _ = stream.flush().await;
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn the_compiled_client_relays_upstream_redirects_instead_of_following_them() {
        // A forwarding proxy must relay a `3xx` to its caller, not resolve it
        // itself: the target and the decision belong to the original client,
        // and a streamed request body cannot be replayed to wherever the
        // upstream's `Location` points.
        let rt = compile(TWO_PROXIES);
        let base = redirecting_server().await;

        let response = rt.client.get(base).send().await.unwrap();

        assert_eq!(response.status(), 302);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "http://example.invalid/target"
        );
    }

    mod mocks {
        use super::*;

        const WITH_MOCKS: &str = r#"
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
  - name: p1
    type: http
    url: "https://example.com/"
    mocks:
      - name: m1
        request:
          method: GET
          url: /api/(?P<zebra>\d+)/(?P<alpha>\w+)/
        response:
          status: 200
          body: 'plain'
"#;

        fn reference_config() -> Config {
            let text = std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../main.example.yaml"
            ))
            .unwrap();
            load_from_str(&text).unwrap()
        }

        fn compile_reference() -> Runtime {
            Runtime::compile(Arc::new(reference_config()), Revision(1)).unwrap()
        }

        #[test]
        fn every_mock_in_the_reference_config_compiles_with_the_right_counts_per_proxy() {
            let rt = compile_reference();

            let proxy1 = rt.proxy_by_name("proxy1").unwrap();
            assert_eq!(proxy1.mocks.len(), 6);
            let proxy2 = rt.proxy_by_name("proxy2").unwrap();
            assert_eq!(proxy2.mocks.len(), 0);
        }

        #[test]
        fn a_proxy_with_no_mocks_compiles_to_an_empty_vector_rather_than_failing() {
            let rt = compile(TWO_PROXIES);
            assert!(rt.proxies[0].mocks.is_empty());
            assert!(rt.proxies[1].mocks.is_empty());
        }

        #[test]
        fn capture_names_are_extracted_in_declaration_order_for_multiple_groups() {
            let config = Arc::new(load_from_str(WITH_MOCKS).unwrap());
            let rt = Runtime::compile(config, Revision(1)).unwrap();
            // Adversarially named: `zebra` is declared before `alpha`, so a
            // regression that sorted the capture names instead of preserving
            // the order the pattern declares them in would flip this and fail.
            assert_eq!(
                mock(rt.proxy_by_name("p1").unwrap(), "m1").capture_names,
                vec!["zebra".to_owned(), "alpha".to_owned()]
            );
        }

        /// Look a reference-config mock up by name rather than by position.
        ///
        /// These tests are about what compilation produces, not about the
        /// order `main.example.yaml` happens to list its mocks in -- and that
        /// order is deliberately significant to matching, so it changes when
        /// the example is improved. Indexing by position coupled these tests
        /// to a decision they do not care about; five of them broke the first
        /// time the example was reordered.
        fn mock<'a>(proxy: &'a CompiledProxy, name: &str) -> &'a CompiledMock {
            proxy
                .mocks
                .iter()
                .find(|m| m.name == name)
                .unwrap_or_else(|| panic!("the reference config must define `{name}`"))
        }

        #[test]
        fn a_mock_with_no_captures_yields_an_empty_capture_names() {
            let rt = compile_reference();
            let mock1 = mock(rt.proxy_by_name("proxy1").unwrap(), "mock1");
            assert_eq!(mock1.name, "mock1");
            assert!(mock1.capture_names.is_empty());
        }

        #[test]
        fn the_three_config_body_fields_map_to_the_matching_mock_body_variant() {
            let rt = compile_reference();
            let proxy1 = rt.proxy_by_name("proxy1").unwrap();
            assert_eq!(
                mock(proxy1, "mock1").body,
                MockBody::Text(r#"{"message": "Success"}"#.to_owned())
            );
            assert!(matches!(mock(proxy1, "mock2").body, MockBody::Json(_)));
            assert_eq!(
                mock(proxy1, "mock6").body,
                MockBody::Template("put.json.j2".to_owned())
            );

            // mock5 answers 204, which forbids a body (rule V30).
            assert_eq!(mock(proxy1, "mock5").body, MockBody::None);
        }

        #[test]
        fn a_mock_proxy_override_survives_compilation_and_a_mock_without_one_leaves_them_none() {
            let rt = compile_reference();
            let proxy1 = rt.proxy_by_name("proxy1").unwrap();

            // mock1 declares no per-mock override at all.
            assert_eq!(mock(proxy1, "mock1").replace, None);
            assert_eq!(mock(proxy1, "mock1").loss, None);
            assert_eq!(mock(proxy1, "mock1").latency, None);

            // mock3 overrides replace and latency, but not loss.
            assert_eq!(mock(proxy1, "mock3").replace, Some(0.5));
            assert_eq!(mock(proxy1, "mock3").loss, None);
            assert!(mock(proxy1, "mock3").latency.is_some());
            assert!((mock(proxy1, "mock3").latency.unwrap().percentage - 0.5).abs() < f64::EPSILON);

            // mock6 overrides loss only.
            assert_eq!(mock(proxy1, "mock6").replace, None);
            assert_eq!(mock(proxy1, "mock6").latency, None);
            assert!(mock(proxy1, "mock6").loss.is_some());
            assert_eq!(mock(proxy1, "mock6").loss.unwrap().status, 503);
        }

        #[test]
        fn header_variable_names_are_lowercased() {
            let rt = compile_reference();
            let proxy1 = rt.proxy_by_name("proxy1").unwrap();

            // mock4 declares `requestId: X-Request-ID`.
            assert_eq!(
                mock(proxy1, "mock4").header_vars,
                vec![("requestId".to_owned(), "x-request-id".to_owned())]
            );
        }

        #[test]
        fn query_and_body_vars_carry_the_raw_selector_unmodified() {
            let rt = compile_reference();
            let proxy1 = rt.proxy_by_name("proxy1").unwrap();

            // mock2 declares query.filter/.sort and three body selectors.
            assert!(
                mock(proxy1, "mock2")
                    .query_vars
                    .contains(&("filter".to_owned(), ".filter".to_owned()))
            );
            assert!(
                mock(proxy1, "mock2")
                    .body_vars
                    .contains(&("resourceItems".to_owned(), ".content.items".to_owned()))
            );
        }

        #[test]
        fn body_limit_reaches_compiled_proxy() {
            let text =
                TWO_PROXIES.replace("    timeout: 5", "    timeout: 5\n    body_limit: 512Ki");
            let rt = compile(&text);
            assert_eq!(rt.proxies[0].body_limit, 512 * 1024);
            // p2 never sets body_limit, so it falls back to the config
            // model's own default rather than picking up p1's value.
            assert_eq!(rt.proxies[1].body_limit, 1024 * 1024);
        }
    }
}
