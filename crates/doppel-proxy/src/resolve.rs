//! Choosing which proxy handles a request.

use axum::http::HeaderMap;
use doppel_core::{CompiledProxy, Error, ErrorCode, Runtime};

/// Select the proxy for a request.
///
/// Resolution headers are walked in configuration order, so a request carrying
/// two of them resolves identically on every process and every run. A header
/// naming a proxy that resolves on a *different* header does not match, because
/// otherwise any proxy could be reached through any resolution header.
pub fn resolve<'a>(runtime: &'a Runtime, headers: &HeaderMap) -> Result<&'a CompiledProxy, Error> {
    for header_name in &runtime.resolve_headers {
        let Some(value) = headers.get(header_name.as_str()) else {
            continue;
        };
        let Ok(name) = value.to_str() else { continue };
        if let Some(proxy) = runtime.proxy_by_name(name)
            && proxy.resolve_header.as_deref() == Some(header_name.as_str())
        {
            return Ok(proxy);
        }
    }

    runtime.default().ok_or_else(|| {
        Error::new(
            ErrorCode::ProxyNotResolved,
            "no proxy matched the request and no default proxy is configured",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use doppel_core::config::load_from_str;
    use doppel_core::store::Revision;

    const CONFIG: &str = r#"
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
  - name: byname
    type: http
    url: "https://one.example.com/"
    resolve:
      type: header
      header: X-Proxy-Name
  - name: byalias
    type: http
    url: "https://two.example.com/"
    resolve:
      type: header
      header: X-Alias
  - name: fallback
    type: http
    url: "https://three.example.com/"
    resolve:
      type: default
"#;

    fn runtime(text: &str) -> Runtime {
        let config = std::sync::Arc::new(load_from_str(text).unwrap());
        Runtime::compile(config, Revision(1)).unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn resolves_by_header() {
        let rt = runtime(CONFIG);
        let proxy = resolve(&rt, &headers(&[("x-proxy-name", "byname")])).unwrap();
        assert_eq!(proxy.name, "byname");
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let rt = runtime(CONFIG);
        let proxy = resolve(&rt, &headers(&[("X-PROXY-NAME", "byname")])).unwrap();
        assert_eq!(proxy.name, "byname");
    }

    #[test]
    fn falls_back_to_default_when_no_header_is_present() {
        let rt = runtime(CONFIG);
        assert_eq!(resolve(&rt, &HeaderMap::new()).unwrap().name, "fallback");
    }

    #[test]
    fn falls_back_when_the_header_names_an_unknown_proxy() {
        let rt = runtime(CONFIG);
        let proxy = resolve(&rt, &headers(&[("x-proxy-name", "ghost")])).unwrap();
        assert_eq!(proxy.name, "fallback");
    }

    #[test]
    fn falls_back_when_the_header_names_a_proxy_that_resolves_on_another_header() {
        let rt = runtime(CONFIG);
        // `byalias` resolves on X-Alias, so naming it via X-Proxy-Name must not win.
        let proxy = resolve(&rt, &headers(&[("x-proxy-name", "byalias")])).unwrap();
        assert_eq!(proxy.name, "fallback");
    }

    #[test]
    fn two_resolution_headers_pick_the_one_declared_first() {
        let rt = runtime(CONFIG);
        let proxy = resolve(
            &rt,
            &headers(&[("x-alias", "byalias"), ("x-proxy-name", "byname")]),
        )
        .unwrap();
        assert_eq!(
            proxy.name, "byname",
            "X-Proxy-Name is declared first in the config"
        );
    }

    #[test]
    fn no_default_and_no_match_is_an_error() {
        let text = CONFIG.replace(
            "      type: default",
            "      type: header\n      header: X-Third",
        );
        let rt = runtime(&text);
        let err = resolve(&rt, &HeaderMap::new()).unwrap_err();
        assert_eq!(err.code, doppel_core::ErrorCode::ProxyNotResolved);
        assert_eq!(err.status(), 404);
    }

    #[test]
    fn a_non_ascii_header_value_does_not_panic() {
        let rt = runtime(CONFIG);
        let mut map = HeaderMap::new();
        map.insert(
            "x-proxy-name",
            HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        assert_eq!(resolve(&rt, &map).unwrap().name, "fallback");
    }
}
