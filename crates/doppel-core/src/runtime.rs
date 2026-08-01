//! The configuration compiled into the form the request path needs, and the
//! holder that swaps it atomically on reload.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::{ArcSwap, Guard};

use crate::config::{Config, LatencyConfig, LossConfig, ProxyConfig, ResolveKind};
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

    Ok(CompiledProxy {
        name: proxy.name.clone(),
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
    limit: 1M
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
}
