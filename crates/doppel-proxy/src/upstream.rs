//! Forwarding a request to the configured upstream and relaying the response.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use doppel_core::{CompiledProxy, Error, ErrorCode};
use futures_util::TryStreamExt;

/// Headers that describe a single connection and must not be relayed.
pub const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
];

/// What happened upstream, for the log line in Task 14.
#[derive(Debug, Clone, Copy)]
pub struct UpstreamOutcome {
    pub status: u16,
    pub duration: Duration,
}

/// Build the upstream URL.
///
/// The base is treated as a directory even when it lacks a trailing slash,
/// because `Url::join` would otherwise replace its last path segment and
/// silently drop part of the configured prefix.
///
/// Client-controlled input never reaches `Url::join`: per RFC 3986
/// reference resolution, an absolute or scheme-relative reference (e.g. a
/// request path of `/https://evil.example.com/x` or `///evil.example.com/x`,
/// both legal HTTP origin-form request targets) replaces the base's
/// authority entirely, letting a client redirect the proxy -- with its
/// configured upstream headers -- to a host of its choosing. Instead, the
/// path is grafted onto the base by manipulating only the path component
/// (`Url::set_path`, which cannot touch the scheme or authority), and a
/// request path containing a `.` or `..` segment is rejected outright rather
/// than normalised, so it can never walk the result outside the configured
/// base. A post-condition re-checks that the scheme, host and port did not
/// move, so the invariant "a proxy for one upstream can only ever talk to
/// that upstream" is enforced, not merely arranged by the steps above.
pub fn join_upstream(
    base: &reqwest::Url,
    path: &str,
    query: Option<&str>,
) -> Result<reqwest::Url, Error> {
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(Error::new(
            ErrorCode::InvalidRequestPath,
            format!("request path `{path}` contains a `.` or `..` segment"),
        ));
    }

    let mut base = base.clone();
    if !base.path().ends_with('/') {
        let directory = format!("{}/", base.path());
        base.set_path(&directory);
    }

    let relative = path.strip_prefix('/').unwrap_or(path);
    let mut url = base.clone();
    url.set_path(&format!("{}{relative}", base.path()));
    url.set_query(query);

    if url.scheme() != base.scheme()
        || url.host() != base.host()
        || url.port_or_known_default() != base.port_or_known_default()
    {
        return Err(Error::new(
            ErrorCode::InvalidRequestPath,
            "request path would resolve outside the configured upstream",
        ));
    }

    Ok(url)
}

/// Forward one request and relay the response, streaming both bodies.
pub async fn forward(
    client: &reqwest::Client,
    proxy: &CompiledProxy,
    request: Request,
    peer: Option<IpAddr>,
) -> Result<(Response, UpstreamOutcome), Error> {
    let (parts, body) = request.into_parts();
    let url = join_upstream(&proxy.base_url, parts.uri.path(), parts.uri.query())?;

    let mut headers = sanitize_headers(&parts.headers);
    // reqwest derives Host from the URL; relaying the client's would send the
    // wrong authority upstream.
    headers.remove("host");
    apply_forwarded_for(&mut headers, &parts.headers, peer);
    for (name, value) in &proxy.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }

    let upstream_body = reqwest::Body::wrap_stream(TryStreamExt::map_err(
        body.into_data_stream(),
        std::io::Error::other,
    ));

    let started = Instant::now();
    let response = client
        .request(parts.method, url)
        .headers(headers)
        .timeout(proxy.timeout)
        .body(upstream_body)
        .send()
        .await
        .map_err(map_upstream_error)?;
    let duration = started.elapsed();

    let status = response.status();
    let relayed = sanitize_headers(response.headers());
    let stream = response.bytes_stream();

    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        headers.extend(relayed);
    }
    let response = builder.body(Body::from_stream(stream)).map_err(|e| {
        Error::new(
            ErrorCode::UpstreamError,
            format!("cannot relay response: {e}"),
        )
    })?;

    Ok((
        response,
        UpstreamOutcome {
            status: status.as_u16(),
            duration,
        },
    ))
}

fn sanitize_headers(headers: &HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| !HOP_BY_HOP.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn apply_forwarded_for(target: &mut HeaderMap, original: &HeaderMap, peer: Option<IpAddr>) {
    let Some(peer) = peer else { return };
    let chain = match original
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        Some(existing) => format!("{existing}, {peer}"),
        None => peer.to_string(),
    };
    if let Ok(value) = HeaderValue::from_str(&chain) {
        target.insert("x-forwarded-for", value);
    }
}

fn map_upstream_error(err: reqwest::Error) -> Error {
    if err.is_timeout() {
        Error::new(ErrorCode::UpstreamTimeout, "upstream timed out")
    } else {
        Error::new(
            ErrorCode::UpstreamError,
            format!("upstream request failed: {err}"),
        )
    }
}

/// Render an `Error` as the documented envelope.
pub fn error_response(err: &Error) -> Response {
    let status = StatusCode::from_u16(err.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_string(&doppel_core::ErrorBody::from(err)).unwrap_or_else(|_| {
        r#"{"status":"error","message":"serialization failed","code":"STORE_ERROR"}"#.to_owned()
    });
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("static response is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, Method, StatusCode};
    use axum::{Router, routing::any};
    use std::time::Duration;

    fn url(text: &str) -> reqwest::Url {
        reqwest::Url::parse(text).unwrap()
    }

    #[test]
    fn joins_a_directory_style_base() {
        let joined = join_upstream(&url("https://host/api/v1/"), "/resource/42", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/resource/42");
    }

    #[test]
    fn treats_a_base_without_a_trailing_slash_as_a_directory_too() {
        // Url::join would otherwise replace the last segment, silently dropping
        // `v1` from every upstream request.
        let joined = join_upstream(&url("https://host/api/v1"), "/resource/42", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/resource/42");
    }

    #[test]
    fn joins_a_root_base() {
        let joined = join_upstream(&url("https://host/"), "/resource", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/resource");
    }

    #[test]
    fn preserves_the_query_string_verbatim() {
        let joined = join_upstream(
            &url("https://host/api/"),
            "/x",
            Some("filter=a%20b&sort=-id"),
        )
        .unwrap();
        assert_eq!(joined.as_str(), "https://host/api/x?filter=a%20b&sort=-id");
    }

    #[test]
    fn an_empty_path_maps_to_the_base() {
        let joined = join_upstream(&url("https://host/api/"), "/", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/");
    }

    // -- Security: client input must never choose the upstream host --------
    //
    // `http`'s origin-form parser accepts `:` in a path, so each of these is
    // a legal request target and `Uri::path()` returns it verbatim. Under
    // the old implementation (`base.join(relative)`), RFC 3986 reference
    // resolution reinterpreted an absolute or scheme-relative reference as
    // replacing the base's authority, letting a client send the proxy --
    // with its configured `Authorization` header -- to a host of its
    // choosing, or escape the configured base path with `..`.

    #[test]
    fn a_path_disguised_as_an_absolute_https_url_does_not_take_over_the_host() {
        let joined = join_upstream(
            &url("https://host/api/v1/"),
            "/https://evil.example.com/x",
            None,
        )
        .unwrap();
        assert_eq!(joined.host_str(), Some("host"));
        assert_eq!(joined.scheme(), "https");
        assert_eq!(
            joined.as_str(),
            "https://host/api/v1/https://evil.example.com/x"
        );
    }

    #[test]
    fn a_path_disguised_as_an_absolute_http_url_does_not_take_over_the_host_or_downgrade_tls() {
        let joined = join_upstream(
            &url("https://host/api/v1/"),
            "/http://evil.example.com/x",
            None,
        )
        .unwrap();
        assert_eq!(joined.host_str(), Some("host"));
        assert_eq!(joined.scheme(), "https");
        assert_eq!(
            joined.as_str(),
            "https://host/api/v1/http://evil.example.com/x"
        );
    }

    #[test]
    fn a_scheme_relative_path_does_not_take_over_the_host() {
        let joined =
            join_upstream(&url("https://host/api/v1/"), "///evil.example.com/x", None).unwrap();
        assert_eq!(joined.host_str(), Some("host"));
        assert_eq!(joined.as_str(), "https://host/api/v1///evil.example.com/x");
    }

    #[test]
    fn dot_dot_segments_are_rejected_rather_than_normalised_past_the_base() {
        // `/../../admin` would resolve to `https://host/admin` under the old
        // `Url::join`-based implementation, escaping the configured
        // `api/v1` prefix. This project prefers rejection over silent
        // normalisation for caller-supplied paths elsewhere, for the same
        // reason.
        let err = join_upstream(&url("https://host/api/v1/"), "/../../admin", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn a_single_dot_segment_is_rejected() {
        let err = join_upstream(&url("https://host/api/v1/"), "/./admin", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn a_percent_encoded_slash_is_forwarded_unchanged_not_decoded_into_a_separator() {
        // Neither decode nor double-encode client input: `%2F` must reach
        // the upstream exactly as sent. If it were decoded, `x%2Fy` would
        // become two path segments (`x`, `y`) instead of the one segment
        // the client actually named.
        let joined = join_upstream(&url("https://host/api/v1/"), "/x%2Fy", None).unwrap();
        assert_eq!(joined.path(), "/api/v1/x%2Fy");
        assert_eq!(joined.path_segments().unwrap().count(), 3);
    }

    #[test]
    fn hop_by_hop_list_is_lowercase_and_complete() {
        assert!(HOP_BY_HOP.contains(&"connection"));
        assert!(HOP_BY_HOP.contains(&"transfer-encoding"));
        assert!(HOP_BY_HOP.contains(&"proxy-authorization"));
        assert!(HOP_BY_HOP.iter().all(|h| h.to_ascii_lowercase() == *h));
    }

    /// Start an upstream that echoes what it received as JSON, and one route
    /// that stalls so the timeout path can be exercised.
    async fn upstream() -> String {
        let app = Router::new()
            .route(
                "/slow",
                any(|| async {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    "never"
                }),
            )
            .route(
                "/echo/big",
                any(|body: axum::body::Bytes| async move { body.len().to_string() }),
            )
            .route(
                "/redirect",
                any(|| async {
                    (
                        StatusCode::FOUND,
                        [("location", "http://example.invalid/target")],
                        "",
                    )
                }),
            )
            // The 8 MiB streaming test below exceeds axum's default 2 MiB
            // whole-body extractor limit; this is a property of this mock
            // upstream's route, not of `forward`, so raise it here only.
            .layer(axum::extract::DefaultBodyLimit::disable())
            .fallback(any(|req: axum::extract::Request| async move {
                let mut headers: Vec<String> = req
                    .headers()
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.to_str().unwrap_or("?")))
                    .collect();
                headers.sort();
                let body = serde_json::json!({
                    "path": req.uri().path(),
                    "query": req.uri().query().unwrap_or(""),
                    "headers": headers,
                })
                .to_string();
                ([("x-upstream", "yes"), ("connection", "close")], body)
            }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/")
    }

    fn proxy(base: &str) -> doppel_core::CompiledProxy {
        doppel_core::CompiledProxy {
            name: "p1".to_owned(),
            base_url: url(base),
            timeout: Duration::from_millis(300),
            headers: vec![("authorization".to_owned(), "Bearer injected".to_owned())],
            loss: None,
            latency: None,
            replace: 1.0,
            resolve_header: None,
        }
    }

    fn request(method: Method, uri: &str) -> axum::extract::Request {
        axum::extract::Request::builder()
            .method(method)
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn forwards_path_and_query() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let response = forward(
            &client,
            &proxy(&base),
            request(Method::GET, "/thing?a=1&b=2"),
            None,
        )
        .await
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response.0).await).unwrap();
        assert_eq!(body["path"], "/thing");
        assert_eq!(body["query"], "a=1&b=2");
    }

    #[tokio::test]
    async fn injects_configured_headers_and_overrides_the_client() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("authorization", HeaderValue::from_static("Bearer client"));

        let response = forward(&client, &proxy(&base), req, None).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response.0).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            headers.iter().any(|h| h == "authorization=Bearer injected"),
            "configured header must win, got {headers:?}"
        );
        assert!(!headers.iter().any(|h| h == "authorization=Bearer client"));
    }

    #[tokio::test]
    async fn strips_hop_by_hop_headers_in_both_directions() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("te", HeaderValue::from_static("trailers"));

        let (response, _) = forward(&client, &proxy(&base), req, None).await.unwrap();
        assert!(
            response.headers().get("connection").is_none(),
            "upstream Connection must be stripped"
        );
        assert_eq!(response.headers().get("x-upstream").unwrap(), "yes");

        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            !headers
                .iter()
                .any(|h| h.as_str().unwrap().starts_with("te="))
        );
    }

    #[tokio::test]
    async fn appends_to_x_forwarded_for_rather_than_replacing_it() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_static("10.0.0.1"));

        let (response, _) = forward(
            &client,
            &proxy(&base),
            req,
            Some("10.0.0.2".parse().unwrap()),
        )
        .await
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            headers
                .iter()
                .any(|h| h == "x-forwarded-for=10.0.0.1, 10.0.0.2"),
            "got {headers:?}"
        );
    }

    #[tokio::test]
    async fn the_upstream_receives_its_own_host_not_the_clients() {
        // `headers.remove("host")` has no test of its own anywhere else: if
        // it were deleted, every other test would still pass, since none of
        // them inspects the Host the upstream actually received.
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("host", HeaderValue::from_static("evil.example"));

        let (response, _) = forward(&client, &proxy(&base), req, None).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            !headers
                .iter()
                .any(|h| h.as_str().unwrap().starts_with("host=evil.example")),
            "the client's Host must not reach the upstream, got {headers:?}"
        );
        assert!(
            headers
                .iter()
                .any(|h| h.as_str().unwrap().starts_with("host=127.0.0.1")),
            "reqwest must derive Host from the upstream url, got {headers:?}"
        );
    }

    #[tokio::test]
    async fn an_upstream_redirect_is_relayed_rather_than_followed() {
        // A forwarding proxy must relay a `3xx` to its caller, not resolve
        // it itself -- the target and the decision belong to the original
        // client, and a streamed request body cannot be replayed to
        // wherever the upstream's `Location` points. This mirrors the
        // client configuration in `doppel_core::Runtime::compile`
        // (`.redirect(reqwest::redirect::Policy::none())`).
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, outcome) = forward(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response.headers().get("location").unwrap(),
            "http://example.invalid/target"
        );
        assert_eq!(outcome.status, 302);
    }

    #[tokio::test]
    async fn error_response_renders_the_documented_envelope_for_upstream_timeout() {
        let err = Error::new(ErrorCode::UpstreamTimeout, "upstream timed out");
        let response = error_response(&err);
        assert_eq!(response.status().as_u16(), 504);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(
            body_string(response).await,
            r#"{"status":"error","message":"upstream timed out","code":"UPSTREAM_TIMEOUT"}"#
        );
    }

    #[tokio::test]
    async fn error_response_renders_the_documented_envelope_for_invalid_request_path() {
        let err = Error::new(
            ErrorCode::InvalidRequestPath,
            "request path `/../x` contains a `.` or `..` segment",
        );
        let response = error_response(&err);
        assert_eq!(response.status().as_u16(), 400);
        assert_eq!(
            body_string(response).await,
            r#"{"status":"error","message":"request path `/../x` contains a `.` or `..` segment","code":"INVALID_REQUEST_PATH"}"#
        );
    }

    #[tokio::test]
    async fn streams_a_body_larger_than_a_buffer() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let payload = vec![b'x'; 8 * 1024 * 1024];
        let req = axum::extract::Request::builder()
            .method(Method::POST)
            .uri("/echo/big")
            .body(axum::body::Body::from(payload))
            .unwrap();

        let mut p = proxy(&base);
        p.timeout = Duration::from_secs(30);
        let (response, _) = forward(&client, &p, req, None).await.unwrap();
        assert_eq!(body_string(response).await, (8 * 1024 * 1024).to_string());
    }

    #[tokio::test]
    async fn a_timeout_becomes_504() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let err = forward(&client, &proxy(&base), request(Method::GET, "/slow"), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, doppel_core::ErrorCode::UpstreamTimeout);
        assert_eq!(err.status(), StatusCode::GATEWAY_TIMEOUT.as_u16());
    }

    #[tokio::test]
    async fn a_refused_connection_becomes_502() {
        let client = reqwest::Client::new();
        // Port 1 on loopback has nothing listening.
        let err = forward(
            &client,
            &proxy("http://127.0.0.1:1/"),
            request(Method::GET, "/x"),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, doppel_core::ErrorCode::UpstreamError);
        assert_eq!(err.status(), 502);
    }

    #[tokio::test]
    async fn reports_the_upstream_status_and_duration() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let (_, outcome) = forward(&client, &proxy(&base), request(Method::GET, "/thing"), None)
            .await
            .unwrap();
        assert_eq!(outcome.status, 200);
        assert!(outcome.duration > Duration::ZERO);
    }
}
