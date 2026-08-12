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
pub const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "trailer",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
];

/// What happened upstream, for the log line in Task 14.
///
/// Both fields stop at the response headers, not the end of the body: the
/// clock (`Instant::now()` below) is read as soon as `send` returns, and the
/// body is streamed back to the caller afterwards, unread by this function.
/// A consumer reading `upstream_duration_ms` as end-to-end latency will be
/// wrong for any response with a body of consequential size or slowness.
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
/// (`Url::set_path`, which cannot touch the scheme or authority).
///
/// A backslash and a decoded `.`/`..` segment are rejected up front so that
/// the common cases fail with a clear, specific message -- but that filter
/// is not what makes this function safe, and it does not try to be
/// exhaustive: `Url::set_path` performs WHATWG dot-segment removal, which
/// also recognises percent-encoded spellings of `.`/`..` (`%2e`, `.%2E`, and
/// so on), treats `\` as a path separator for special schemes, and silently
/// strips ASCII tab/CR/LF before segmenting, any of which can turn an
/// innocent-looking raw segment into a real `..` once normalised. Predicting
/// every such case here would mean re-modelling `url`'s normalisation rules
/// (and the next `url` release could add more). The actual guarantee is a
/// post-condition on the *result*: the resulting URL's scheme, host and port
/// must equal the base's, and its path must still start with the base's
/// (trailing-slash-normalised) path. A proxy configured for one upstream and
/// one base path can only ever reach paths under that base on that upstream,
/// and that is enforced by checking the outcome, not by inferring it from
/// the input.
pub fn join_upstream(
    base: &reqwest::Url,
    path: &str,
    query: Option<&str>,
) -> Result<reqwest::Url, Error> {
    // `\` is a path separator for special schemes (http/https among them) in
    // `Url::set_path`'s parser, exactly like `/`; `http`'s path parser
    // accepts a literal backslash in a request target, so this arrives over
    // the wire. Reject it outright rather than modelling that separator
    // rule ourselves.
    if path.contains('\\') {
        return Err(Error::new(
            ErrorCode::InvalidRequestPath,
            format!("request path `{path}` contains a backslash"),
        ));
    }

    // Decode only the `%2e`/`%2E` percent-encoding of `.` -- there is no
    // case-insensitivity to exploit beyond the `e`/`E` letter itself, since
    // hex digits have no case -- before comparing a whole segment against
    // `.`/`..`. This exists solely to produce a clear rejection message for
    // the same inputs `url` itself treats as dot segments; it is not the
    // safety net (see the containment check below), so it compares whole
    // decoded segments for equality and never widens to "contains".
    if path.split('/').any(|segment| {
        let decoded = segment.replace("%2e", ".").replace("%2E", ".");
        decoded == "." || decoded == ".."
    }) {
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

    // The decisive guarantee: whatever `Url::set_path` actually did with the
    // input -- including any dot-segment, separator or control-character
    // handling the checks above did not anticipate -- the result must still
    // be a path under the configured base, on the configured scheme, host
    // and port.
    if url.scheme() != base.scheme()
        || url.host() != base.host()
        || url.port_or_known_default() != base.port_or_known_default()
        || !url.path().starts_with(base.path())
    {
        return Err(Error::new(
            ErrorCode::InvalidRequestPath,
            "request path would resolve outside the configured upstream",
        ));
    }

    Ok(url)
}

/// Forward one request and relay the response, streaming both bodies.
///
/// `resolve_headers` are the runtime's configured resolution header names
/// (lowercased); each is stripped from the outbound request so a proxy
/// resolved via e.g. `X-Proxy-Name` does not leak Doppel's own routing
/// vocabulary upstream, and a chained Doppel does not re-resolve on it.
///
/// `request_id` is sent upstream as `x-request-id` so a single request can be
/// followed across services, per the core design spec's logging section.
pub async fn forward(
    client: &reqwest::Client,
    proxy: &CompiledProxy,
    request: Request,
    peer: Option<IpAddr>,
    resolve_headers: &[String],
    request_id: &str,
    // Doppel's own address, when the deployment knows it. Only redirect
    // rewriting reads it -- see `rewritten_location`.
    external: Option<&reqwest::Url>,
) -> Result<(Response, UpstreamOutcome), Error> {
    let (parts, body) = request.into_parts();
    let url = join_upstream(&proxy.base_url, parts.uri.path(), parts.uri.query())?;

    let mut headers = sanitize_headers(&parts.headers);
    // reqwest derives Host from the URL; relaying the client's would send the
    // wrong authority upstream. What the client asked for is not thrown away
    // though -- it goes on as `X-Forwarded-Host` below.
    //
    // Read before the removal, and from `parts.headers` rather than from the
    // sanitized copy. HTTP/2 carries the authority in `:authority` rather than
    // in a `Host` header, which `axum` surfaces on the URI, so both are tried.
    let client_authority = parts
        .headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| parts.uri.authority().map(ToString::to_string));
    headers.remove("host");
    for name in resolve_headers {
        headers.remove(name.as_str());
    }
    apply_forwarded_for(&mut headers, &parts.headers, peer);
    if let Some(authority) = client_authority.as_deref() {
        apply_forwarded_host_and_proto(&mut headers, &parts.headers, authority);
    }
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert("x-request-id", value);
    }
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
        // Cloned because the redirect rewriting below resolves the upstream's
        // `Location` against the URL that was actually requested, and reqwest
        // takes ownership here.
        .request(parts.method, url.clone())
        .headers(headers)
        .timeout(proxy.timeout)
        .body(upstream_body)
        .send()
        .await
        .map_err(map_upstream_error)?;
    // `send` resolves once the response headers arrive; the body below is
    // streamed back to the caller unread, so `duration` covers only the time
    // to first byte of the headers, not the full response body. See the
    // `UpstreamOutcome` doc comment.
    let duration = started.elapsed();

    let status = response.status();
    let mut relayed = sanitize_headers(response.headers());
    if proxy.rewrite_redirects
        && let Some(rewritten) = rewritten_location(&relayed, &url, &proxy.base_url, external)
    {
        relayed.insert("location", rewritten);
    }
    let stream = response.bytes_stream();

    let mut builder = Response::builder().status(status);
    if let Some(headers) = builder.headers_mut() {
        headers.extend(relayed);
        // Returned to the client on every response, not just when it happened
        // to send one itself, so a single request can be followed across
        // services regardless of who minted the id.
        if let Ok(value) = HeaderValue::from_str(request_id) {
            headers.insert("x-request-id", value);
        }
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

/// Strip the standard hop-by-hop headers plus, per RFC 9110 section 7.6.1,
/// every header the `Connection` field itself names. `HeaderName` is always
/// lowercase, so each comma-separated `Connection` token is lowercased before
/// comparison rather than assuming the sender wrote it that way.
fn sanitize_headers(headers: &HeaderMap) -> HeaderMap {
    let named_by_connection: Vec<String> = headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect();

    headers
        .iter()
        .filter(|(name, _)| {
            !HOP_BY_HOP.contains(&name.as_str())
                && !named_by_connection
                    .iter()
                    .any(|token| token.as_str() == name.as_str())
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Join every `X-Forwarded-For` field line (not just the first -- `HeaderMap::get`
/// would silently drop the rest of a chain split across several lines, which
/// some proxies emit) with the peer appended last.
fn apply_forwarded_for(target: &mut HeaderMap, original: &HeaderMap, peer: Option<IpAddr>) {
    let Some(peer) = peer else { return };
    let existing: Vec<&str> = original
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    let chain = if existing.is_empty() {
        peer.to_string()
    } else {
        format!("{}, {peer}", existing.join(", "))
    };
    if let Ok(value) = HeaderValue::from_str(&chain) {
        target.insert("x-forwarded-for", value);
    }
}

/// A `Location` the client can follow without leaving Doppel, or `None` when
/// the header should be relayed exactly as it arrived.
///
/// The problem this solves is a consequence of not relaying `Host`: the upstream
/// answers a redirect with its *own* authority in `Location`, and a client that
/// follows it talks to the backend directly from then on -- past every injected
/// fault and every mock, with nothing logged and nothing failing. `nginx` has
/// `proxy_redirect` and Apache `ProxyPassReverse` for exactly this, both on by
/// default, because forgetting it is so easy.
///
/// The value is resolved against the URL that was actually requested upstream,
/// which is what the upstream meant by it, and then judged against the proxy's
/// base and against `external`, Doppel's own address when one is configured:
///
/// - **Under `base`** -- same origin, and a path below its path prefix. With no
///   `external`, a root-relative path plus query and fragment, so Doppel never
///   has to guess its own public name: a client resolves it against the URL it
///   used. RFC 9110 has allowed a relative `Location` since 7231 superseded
///   2616. With `external`, the same tail against that address, absolutely.
/// - **Same origin, outside `base`** -- `/login` under a base of `/api/v1/`.
///   Without `external` this is relayed absolutely, pointing at the upstream:
///   a root-relative form would be re-resolved against Doppel and forwarded
///   back as `/api/v1/login`, a resource nobody asked for, and there is no host
///   to name instead. With `external` the client is kept on Doppel --
///   `https://doppel.example.com/login` -- because a client that leaves is a
///   client past every fault and mock, which is the whole point of the rewrite.
///   The path is kept as the upstream wrote it, which is what `nginx`'s
///   `proxy_redirect` does; whether Doppel serves that path is a question about
///   the configuration, and one the operator can see.
/// - **Anywhere else** -- another host entirely, and left alone in every case.
///   Doppel has no route to it, and naming itself in a redirect to somewhere it
///   does not proxy would be a lie.
///
/// Not covered: `Content-Location` names where a payload lives rather than where
/// to go next, `Refresh` is not a standard header, and `Set-Cookie`'s `Domain`
/// needs its own rewriting rule. Each would be its own decision.
fn rewritten_location(
    relayed: &HeaderMap,
    requested: &reqwest::Url,
    base: &reqwest::Url,
    external: Option<&reqwest::Url>,
) -> Option<HeaderValue> {
    let location = relayed.get("location")?.to_str().ok()?;
    let resolved = requested.join(location).ok()?;

    let base_path = base.path();
    let same_origin = resolved.origin() == base.origin();
    let under_base = same_origin
        && resolved.path().starts_with(base_path)
        // A base path is a directory prefix, so `/api/v1x` must not count as
        // being under `/api/v1/`. `join_upstream` treats the base the same way.
        && (base_path.ends_with('/') || resolved.path().len() == base_path.len());

    let value = if under_base {
        let tail = tail_after(&resolved, base_path);
        match external {
            Some(external) => under(external, &tail),
            None => format!("/{tail}"),
        }
    } else if same_origin {
        match external {
            Some(external) => under(external, &tail_after(&resolved, "/")),
            None => resolved.to_string(),
        }
    } else {
        resolved.to_string()
    };

    // Nothing to say if the header already reads that way -- returning `None`
    // keeps the original bytes rather than a re-serialized equivalent.
    if value == location {
        return None;
    }
    HeaderValue::from_str(&value).ok()
}

/// The part of `url` after `prefix`, with its query and fragment, and no
/// leading slash.
fn tail_after(url: &reqwest::Url, prefix: &str) -> String {
    let mut tail = url
        .path()
        .trim_start_matches(prefix)
        .trim_start_matches('/')
        .to_owned();
    if let Some(query) = url.query() {
        tail.push('?');
        tail.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        tail.push('#');
        tail.push_str(fragment);
    }
    tail
}

/// `tail` under `external`, keeping any path `external` itself carries -- a
/// Doppel reached at `https://gw.example.com/doppel/` is reached there in a
/// rewritten `Location` too.
///
/// Built by hand rather than with `Url::join`, which reads a leading slash as
/// "from the root" and would drop that prefix.
fn under(external: &reqwest::Url, tail: &str) -> String {
    let mut out = external.as_str().trim_end_matches('/').to_owned();
    out.push('/');
    out.push_str(tail);
    out
}

/// Records what the client asked for, since `Host` is not relayed: without
/// these the upstream cannot tell a request through Doppel from a direct one,
/// and cannot build a URL that points back here.
///
/// An incoming value is left alone rather than overwritten. That is the same
/// treatment `X-Forwarded-For` gets above, and for the same reason: with Doppel
/// behind another proxy, the value that arrived names the authority the client
/// really used, and this hop's own `Host` is an internal detail. The cost is
/// that a client talking to Doppel directly can put whatever it likes in them --
/// which is true of every proxy that preserves the chain, and is why these
/// headers are only ever as trustworthy as the hop that set them.
///
/// `X-Forwarded-Proto` is `http` when Doppel sets it: its listeners are plain
/// TCP and it terminates no TLS, so `https` would be a claim about a hop that
/// does not exist here.
fn apply_forwarded_host_and_proto(target: &mut HeaderMap, original: &HeaderMap, authority: &str) {
    if !original.contains_key("x-forwarded-host")
        && let Ok(value) = HeaderValue::from_str(authority)
    {
        target.insert("x-forwarded-host", value);
    }
    if !original.contains_key("x-forwarded-proto") {
        target.insert("x-forwarded-proto", HeaderValue::from_static("http"));
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

    // -- Security round 2: the segment filter must not be the safety net ---
    //
    // `Url::set_path` performs WHATWG dot-segment removal, which also
    // recognises several percent-encoded spellings of `.`/`..`, and treats
    // `\` as a path separator for special schemes (http/https among them).
    // `http`'s path parser accepts a literal backslash in a request target,
    // so both arrive over the wire. A filter that only compares raw,
    // undecoded `/`-split segments against the literal strings `.` and `..`
    // misses all of these; the containment check below is what actually
    // closes the class.

    #[test]
    fn rejects_a_double_dot_disguised_as_lowercase_percent_encoding() {
        let err =
            join_upstream(&url("https://host/api/v1/"), "/%2e%2e/%2e%2e/admin", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn rejects_a_double_dot_disguised_as_uppercase_percent_encoding() {
        let err =
            join_upstream(&url("https://host/api/v1/"), "/%2E%2E/%2E%2E/admin", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn rejects_a_double_dot_disguised_as_mixed_literal_and_encoded_dots() {
        let err =
            join_upstream(&url("https://host/api/v1/"), "/.%2e/.%2e/admin", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn rejects_a_path_containing_a_backslash() {
        let err = join_upstream(&url("https://host/api/v1/"), "/..\\..\\admin", None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn a_backslash_cannot_be_used_to_cross_into_a_sibling_tenant() {
        let err = join_upstream(
            &url("https://host/api/v1/"),
            "/x/..\\..\\..\\tenant-b/secret",
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
    }

    #[test]
    fn an_encoded_dot_dot_slash_taken_as_one_segment_is_safe_and_stays_literal() {
        // `%2e%2e%2f` decodes to `../`, not to a `..` segment (the trailing
        // `%2f` is not a `/`), so the whole thing is one ordinary segment
        // that does not traverse. This must keep passing, unrejected: the
        // rule is equality against a decoded segment, not "contains".
        let joined = join_upstream(&url("https://host/api/v1/"), "/%2e%2e%2fadmin", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/%2e%2e%2fadmin");
    }

    #[test]
    fn a_percent_encoded_backslash_is_forwarded_unchanged_not_treated_as_a_separator() {
        // `%5C` decodes to a literal backslash, but it is not a raw `\` byte
        // in the request target itself, so neither the backslash filter
        // (which only rejects a literal `\`) nor `Url::set_path`'s
        // separator handling (which only treats an actual `\` byte as a
        // separator for special schemes) has any reason to touch it. A
        // future "hardening" that widened the backslash filter to a
        // `contains` check on the raw text would start refusing this valid,
        // non-traversing path.
        let joined = join_upstream(&url("https://host/api/v1/"), "/x%5Cy", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/x%5Cy");
    }

    #[test]
    fn a_double_encoded_dot_is_not_a_dot_segment_and_is_forwarded_unchanged() {
        // `%252e` decodes once to `%2e`, and only reaches `.` on a *second*
        // decoding pass -- but nothing here, and nothing in `Url::set_path`,
        // ever double-decodes. It must be treated as an ordinary,
        // non-traversing segment, not rejected as a disguised `.`. A future
        // "hardening" of the decoded-segment filter into a `contains` check
        // would start refusing this valid path too.
        let joined = join_upstream(&url("https://host/api/v1/"), "/%252e/admin", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/%252e/admin");
    }

    #[test]
    fn a_filename_containing_literal_dots_is_not_mistaken_for_a_dot_segment() {
        let joined = join_upstream(&url("https://host/api/v1/"), "/file..name.json", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/file..name.json");
    }

    #[test]
    fn a_version_like_segment_is_not_mistaken_for_a_dot_segment() {
        let joined = join_upstream(&url("https://host/api/v1/"), "/v1.2/x", None).unwrap();
        assert_eq!(joined.as_str(), "https://host/api/v1/v1.2/x");
    }

    #[test]
    fn the_containment_check_catches_a_dot_segment_hidden_by_a_stripped_control_character() {
        // WHATWG URL parsing silently strips ASCII tab/CR/LF from its input
        // before segmenting it, so a literal tab hidden inside a segment
        // that is not, on its own, `.` or `..` can still collapse to `..`
        // once `Url::set_path` strips it -- a case neither the backslash
        // check nor the decoded-segment check above catches, because ".\t."
        // is not equal to ".." before that stripping happens. This is
        // exactly the class of vector the containment check exists for.
        //
        // This is the sole test that actually exercises the containment
        // check's `starts_with` clause in `join_upstream` -- every other
        // rejection in this file is already caught earlier by the backslash
        // or decoded-segment filters, so removing the `starts_with` check
        // would not fail any test but this one. Do not delete this case as
        // "redundant" without first checking whether it still is.
        let path = "/.\t./admin";
        let err = join_upstream(&url("https://host/api/v1/"), path, None).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequestPath);
        assert_eq!(err.status(), 400);
    }

    #[test]
    fn hop_by_hop_list_is_lowercase_and_complete() {
        assert!(HOP_BY_HOP.contains(&"connection"));
        assert!(HOP_BY_HOP.contains(&"transfer-encoding"));
        assert!(HOP_BY_HOP.contains(&"proxy-authorization"));
        assert!(HOP_BY_HOP.contains(&"proxy-connection"));
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
            // Three redirects that differ only in how the upstream spelled
            // `Location`, which is what decides whether it can be rewritten:
            // its own absolute URL, a path relative to its root, and a path
            // relative to the current one.
            .route(
                "/redirect/self",
                // The authority is read off the request rather than taken from
                // an extractor: `axum::extract::Host` moved to `axum-extra` in
                // axum 0.8, and this workspace does not depend on it.
                any(|req: axum::extract::Request| async move {
                    let host = req
                        .headers()
                        .get("host")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("127.0.0.1")
                        .to_owned();
                    (
                        StatusCode::FOUND,
                        [("location", format!("http://{host}/moved?keep=1#frag"))],
                        "",
                    )
                }),
            )
            .route(
                "/redirect/rooted",
                any(|| async { (StatusCode::FOUND, [("location", "/moved")], "") }),
            )
            .route(
                "/redirect/relative",
                any(|| async { (StatusCode::FOUND, [("location", "sibling")], "") }),
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
            rewrite_redirects: true,
            resolve_header: None,
            mocks: Vec::new(),
            body_limit: 1024 * 1024,
        }
    }

    /// The same proxy with redirect rewriting turned off, for the tests that
    /// pin what `rewrite_redirects: false` relays.
    fn proxy_relaying_redirects(base: &str) -> doppel_core::CompiledProxy {
        doppel_core::CompiledProxy {
            rewrite_redirects: false,
            ..proxy(base)
        }
    }

    async fn location_of(response: axum::response::Response) -> String {
        response
            .headers()
            .get("location")
            .expect("a redirect must carry a Location")
            .to_str()
            .unwrap()
            .to_owned()
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

    /// A stand-in request id for tests that do not exercise request-id
    /// propagation itself.
    const TEST_REQUEST_ID: &str = "test-request-id";

    /// `forward` with the defaults every test that is not itself about
    /// resolve-header stripping or request-id propagation wants: no
    /// resolution headers to strip, and a fixed request id.
    async fn fwd(
        client: &reqwest::Client,
        proxy: &doppel_core::CompiledProxy,
        request: axum::extract::Request,
        peer: Option<IpAddr>,
    ) -> Result<(Response, UpstreamOutcome), Error> {
        forward(client, proxy, request, peer, &[], TEST_REQUEST_ID, None).await
    }

    /// `fwd` for the tests that are about `server.external_url`.
    async fn fwd_external(
        client: &reqwest::Client,
        proxy: &doppel_core::CompiledProxy,
        request: axum::extract::Request,
        external: &str,
    ) -> Result<(Response, UpstreamOutcome), Error> {
        let external = reqwest::Url::parse(external).expect("a test url parses");
        forward(
            client,
            proxy,
            request,
            None,
            &[],
            TEST_REQUEST_ID,
            Some(&external),
        )
        .await
    }

    #[tokio::test]
    async fn forwards_path_and_query() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let response = fwd(
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

        let response = fwd(&client, &proxy(&base), req, None).await.unwrap();
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

        let (response, _) = fwd(&client, &proxy(&base), req, None).await.unwrap();
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

    #[test]
    fn sanitize_headers_strips_headers_named_by_the_connection_field() {
        // RFC 9110 section 7.6.1: a proxy must remove any header the
        // `Connection` field names, on top of the fixed hop-by-hop set.
        let mut headers = HeaderMap::new();
        headers.insert("connection", HeaderValue::from_static("X-Custom, x-other"));
        headers.insert("x-custom", HeaderValue::from_static("secret"));
        headers.insert("x-other", HeaderValue::from_static("also secret"));
        headers.insert("x-kept", HeaderValue::from_static("kept"));

        let sanitized = sanitize_headers(&headers);
        assert!(sanitized.get("connection").is_none());
        assert!(sanitized.get("x-custom").is_none());
        assert!(sanitized.get("x-other").is_none());
        assert_eq!(sanitized.get("x-kept").unwrap(), "kept");
    }

    #[tokio::test]
    async fn a_header_named_by_connection_is_stripped_from_the_outbound_request() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("connection", HeaderValue::from_static("x-secret"));
        req.headers_mut()
            .insert("x-secret", HeaderValue::from_static("leak-me-not"));

        let (response, _) = fwd(&client, &proxy(&base), req, None).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            !headers
                .iter()
                .any(|h| h.as_str().unwrap().starts_with("x-secret=")),
            "got {headers:?}"
        );
    }

    /// `Host` is replaced by the upstream's own authority, so without these the
    /// upstream has no way to learn what the client actually asked for and
    /// cannot build a URL pointing back at Doppel.
    #[tokio::test]
    async fn sends_the_clients_authority_as_x_forwarded_host_and_http_as_the_proto() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("host", HeaderValue::from_static("public.example.com"));

        let (response, _) = fwd(&client, &proxy(&base), req, None).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            headers
                .iter()
                .any(|h| h == "x-forwarded-host=public.example.com"),
            "got {headers:?}"
        );
        // `http`, not `https`: Doppel terminates no TLS, so claiming otherwise
        // would describe a hop that does not exist.
        assert!(
            headers.iter().any(|h| h == "x-forwarded-proto=http"),
            "got {headers:?}"
        );
    }

    /// Behind another proxy, the value that arrived names the authority the
    /// client really used; this hop's own `Host` is an internal detail. Same
    /// treatment `X-Forwarded-For` gets, and the reason the two tests sit
    /// together.
    #[tokio::test]
    async fn an_incoming_forwarded_host_and_proto_are_left_as_they_arrived() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("host", HeaderValue::from_static("inner.internal"));
        req.headers_mut().insert(
            "x-forwarded-host",
            HeaderValue::from_static("edge.example.com"),
        );
        req.headers_mut()
            .insert("x-forwarded-proto", HeaderValue::from_static("https"));

        let (response, _) = fwd(&client, &proxy(&base), req, None).await.unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            headers
                .iter()
                .any(|h| h == "x-forwarded-host=edge.example.com"),
            "the outermost authority must survive: {headers:?}"
        );
        assert!(
            headers.iter().any(|h| h == "x-forwarded-proto=https"),
            "an upstream TLS terminator's proto must survive: {headers:?}"
        );
        assert!(
            !headers
                .iter()
                .any(|h| h == "x-forwarded-host=inner.internal"),
            "this hop's own Host must not replace it: {headers:?}"
        );
    }

    #[tokio::test]
    async fn appends_to_x_forwarded_for_rather_than_replacing_it() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_static("10.0.0.1"));

        let (response, _) = fwd(
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
    async fn a_chain_split_across_several_x_forwarded_for_lines_is_joined_not_truncated() {
        // `HeaderMap::get` returns only the first field line of a repeated
        // header; some proxies emit the chain split across several lines,
        // which is legal. Losing everything but the first before appending
        // the peer would silently drop the earlier hops.
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .append("x-forwarded-for", HeaderValue::from_static("10.0.0.1"));
        req.headers_mut()
            .append("x-forwarded-for", HeaderValue::from_static("10.0.0.2"));

        let (response, _) = fwd(
            &client,
            &proxy(&base),
            req,
            Some("10.0.0.3".parse().unwrap()),
        )
        .await
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            headers
                .iter()
                .any(|h| h == "x-forwarded-for=10.0.0.1, 10.0.0.2, 10.0.0.3"),
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

        let (response, _) = fwd(&client, &proxy(&base), req, None).await.unwrap();
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

        let (response, outcome) = fwd(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        // Untouched even with rewriting on: `example.invalid` is not under this
        // proxy's base, so the target really is elsewhere and claiming otherwise
        // would be a lie. See `rewritten_location`.
        assert_eq!(
            response.headers().get("location").unwrap(),
            "http://example.invalid/target"
        );
        assert_eq!(outcome.status, 302);
    }

    /// The default, and the whole reason it is the default: `Host` is replaced
    /// with the upstream's authority, so the upstream answers with its own name
    /// in `Location`, and a client following that leaves Doppel -- past every
    /// injected fault and every mock, with nothing logged.
    ///
    /// Rewritten to a root-relative path rather than an absolute URL so Doppel
    /// never has to guess its own public name; the client resolves it against
    /// the URL it used. Query and fragment survive.
    #[tokio::test]
    async fn a_redirect_into_the_proxied_space_is_rewritten_to_point_back_at_doppel() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect/self"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(location_of(response).await, "/moved?keep=1#frag");
    }

    /// `rewrite_redirects: false` relays the header byte for byte, which is what
    /// a client under test *for its redirect handling* needs to see.
    #[tokio::test]
    async fn rewrite_redirects_false_relays_the_upstreams_own_authority() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd(
            &client,
            &proxy_relaying_redirects(&base),
            request(Method::GET, "/redirect/self"),
            None,
        )
        .await
        .unwrap();
        let location = location_of(response).await;
        assert!(
            location.starts_with("http://127.0.0.1:") && location.ends_with("/moved?keep=1#frag"),
            "expected the upstream's own absolute URL, got {location}"
        );
    }

    /// A base with a path prefix is where the quieter bug lives. The upstream
    /// means `/moved` against its own root, which is *outside* the `/api/v1/`
    /// prefix; relayed as-is, the client would resolve it against Doppel and
    /// come back asking for `/api/v1/moved` -- a different resource nobody named.
    /// Stated absolutely, the escape is at least honest.
    #[tokio::test]
    async fn a_root_relative_redirect_outside_the_base_path_is_made_absolute() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let mut with_prefix = proxy(&base);
        with_prefix.base_url = url(&format!("{base}redirect/"));

        // `/rooted` under the base `…/redirect/` reaches the upstream's
        // `/redirect/rooted` route, which answers `Location: /moved`.
        let (response, _) = fwd(&client, &with_prefix, request(Method::GET, "/rooted"), None)
            .await
            .unwrap();
        let location = location_of(response).await;
        assert_eq!(location, format!("{base}moved"));
        assert!(
            !location.starts_with('/'),
            "a target outside the base cannot be expressed relative to Doppel: {location}"
        );
    }

    /// With an external url, the same redirect names Doppel outright.
    ///
    /// This is what a deployment behind a port mapping or an ingress needs: the
    /// relative form is correct but only a client that resolves it against the
    /// URL it used benefits, and anything logging or storing the `Location` --
    /// a test report, a browser's history, a client that just prints it -- gets
    /// a path with no host in it.
    #[tokio::test]
    async fn an_external_url_puts_doppels_own_host_in_the_location() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd_external(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect/self"),
            "https://doppel.example.com/",
        )
        .await
        .unwrap();
        assert_eq!(
            location_of(response).await,
            "https://doppel.example.com/moved?keep=1#frag"
        );
    }

    /// The case the relative form cannot express, and the reason this setting
    /// exists: the upstream sends the client to its own host *outside* the
    /// proxied path. Without an external url that is relayed as the upstream's
    /// own absolute URL and the client leaves Doppel -- past every fault and
    /// every mock. With one, the client stays.
    #[tokio::test]
    async fn a_redirect_outside_the_base_is_kept_on_doppel_when_it_has_a_name() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let mut with_prefix = proxy(&base);
        with_prefix.base_url = url(&format!("{base}redirect/"));

        let (response, _) = fwd_external(
            &client,
            &with_prefix,
            request(Method::GET, "/rooted"),
            "https://doppel.example.com/",
        )
        .await
        .unwrap();
        // The upstream said `/moved` against its own root. The path is kept as
        // the upstream wrote it, which is what nginx's `proxy_redirect` does.
        assert_eq!(
            location_of(response).await,
            "https://doppel.example.com/moved"
        );
    }

    /// A Doppel reached under a path prefix keeps it. `Url::join` would not:
    /// a leading slash reads as "from the root" and drops the prefix.
    #[tokio::test]
    async fn an_external_url_with_a_path_keeps_that_path() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd_external(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect/self"),
            "https://gw.example.com/doppel/",
        )
        .await
        .unwrap();
        assert_eq!(
            location_of(response).await,
            "https://gw.example.com/doppel/moved?keep=1#frag"
        );
    }

    /// Another host stays another host, external url or not. Doppel does not
    /// proxy `example.invalid` and saying otherwise would send the client to
    /// itself for something it cannot serve.
    #[tokio::test]
    async fn a_foreign_host_is_left_alone_even_with_an_external_url() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd_external(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect"),
            "https://doppel.example.com/",
        )
        .await
        .unwrap();
        assert_eq!(location_of(response).await, "http://example.invalid/target");
    }

    /// `rewrite_redirects: false` still means no rewriting at all.
    #[tokio::test]
    async fn an_external_url_does_not_override_rewrite_redirects_false() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd_external(
            &client,
            &proxy_relaying_redirects(&base),
            request(Method::GET, "/redirect/self"),
            "https://doppel.example.com/",
        )
        .await
        .unwrap();
        let location = location_of(response).await;
        assert!(
            location.starts_with("http://127.0.0.1:"),
            "expected the upstream's own authority, got {location}"
        );
    }

    /// A `Location` relative to the current path resolves inside the base, so it
    /// comes back relative -- and correctly, which relaying would not have been:
    /// the upstream meant `sibling` next to `/redirect/relative`.
    #[tokio::test]
    async fn a_path_relative_redirect_inside_the_base_stays_relative() {
        let base = upstream().await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        let (response, _) = fwd(
            &client,
            &proxy(&base),
            request(Method::GET, "/redirect/relative"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(location_of(response).await, "/redirect/sibling");
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
        let (response, _) = fwd(&client, &p, req, None).await.unwrap();
        assert_eq!(body_string(response).await, (8 * 1024 * 1024).to_string());
    }

    #[tokio::test]
    async fn a_timeout_becomes_504() {
        let base = upstream().await;
        let client = reqwest::Client::new();
        let err = fwd(&client, &proxy(&base), request(Method::GET, "/slow"), None)
            .await
            .unwrap_err();
        assert_eq!(err.code, doppel_core::ErrorCode::UpstreamTimeout);
        assert_eq!(err.status(), StatusCode::GATEWAY_TIMEOUT.as_u16());
    }

    #[tokio::test]
    async fn a_refused_connection_becomes_502() {
        let client = reqwest::Client::new();
        // Port 1 on loopback has nothing listening.
        let err = fwd(
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
        let (_, outcome) = fwd(&client, &proxy(&base), request(Method::GET, "/thing"), None)
            .await
            .unwrap();
        assert_eq!(outcome.status, 200);
        assert!(outcome.duration > Duration::ZERO);
    }

    #[tokio::test]
    async fn resolve_headers_are_stripped_from_the_outbound_request() {
        // A request routed by e.g. `X-Proxy-Name` must not carry that header
        // upstream: it leaks Doppel's own routing vocabulary and would make
        // a chained Doppel re-resolve on it.
        let base = upstream().await;
        let client = reqwest::Client::new();
        let mut req = request(Method::GET, "/thing");
        req.headers_mut()
            .insert("x-proxy-name", HeaderValue::from_static("p1"));
        req.headers_mut()
            .insert("x-kept", HeaderValue::from_static("kept"));

        let resolve_headers = vec!["x-proxy-name".to_owned()];
        let (response, _) = forward(
            &client,
            &proxy(&base),
            req,
            None,
            &resolve_headers,
            TEST_REQUEST_ID,
            None,
        )
        .await
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            !headers
                .iter()
                .any(|h| h.as_str().unwrap().starts_with("x-proxy-name=")),
            "got {headers:?}"
        );
        assert!(headers.iter().any(|h| h == "x-kept=kept"));
    }

    #[tokio::test]
    async fn the_client_supplied_request_id_is_sent_upstream_unchanged() {
        let base = upstream().await;
        let client = reqwest::Client::new();

        let (response, _) = forward(
            &client,
            &proxy(&base),
            request(Method::GET, "/thing"),
            None,
            &[],
            "caller-chosen-id",
            None,
        )
        .await
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&body_string(response).await).unwrap();
        let headers = body["headers"].as_array().unwrap();
        assert!(
            headers.iter().any(|h| h == "x-request-id=caller-chosen-id"),
            "got {headers:?}"
        );
    }

    #[tokio::test]
    async fn the_request_id_is_returned_to_the_client_on_the_response() {
        let base = upstream().await;
        let client = reqwest::Client::new();

        let (response, _) = forward(
            &client,
            &proxy(&base),
            request(Method::GET, "/thing"),
            None,
            &[],
            "caller-chosen-id",
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            response.headers().get("x-request-id").unwrap(),
            "caller-chosen-id"
        );
    }
}
