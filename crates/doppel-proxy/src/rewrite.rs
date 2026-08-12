//! Rewriting the upstream's own address out of a relayed body.
//!
//! `rewrite_redirects` keeps a client inside Doppel when the upstream answers a
//! redirect. This is the same problem one layer down: a page, a script or a JSON
//! document that names `https://api.example.com/v2/` sends the client straight
//! there on its next request, past every injected fault and every mock, with
//! nothing logged and nothing failing. `nginx` has `sub_filter` for this.
//!
//! Three rules, and each is a decision worth reading before changing:
//!
//! - **Exact host only.** `https://cdn.api.example.com/` is a different host, is
//!   not proxied by this proxy, and is left alone -- pointing it at Doppel would
//!   break the page rather than keep it working. That falls out of matching on
//!   `scheme://host` and then requiring the next character not to continue a
//!   hostname, which is also what stops `https://api.example.com.evil.test/` from
//!   being mangled into Doppel's address with a suffix.
//! - **Text only, and only what the upstream did not compress.** A body Doppel
//!   would have to decompress to search is relayed untouched, and so is one whose
//!   content type is not text: an image cannot contain a URL that matters, and
//!   buffering one to look is how a proxy runs out of memory.
//! - **Bounded.** Rewriting needs the whole body, so it is buffered -- up to the
//!   proxy's `body_limit`, the same ceiling that bounds a mock's request buffering.
//!   A body that exceeds it is streamed on from where the buffering stopped,
//!   untouched, rather than held in memory or truncated.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, header};
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};

/// The content types worth searching for a URL.
///
/// A prefix list rather than a parser: the header may carry parameters
/// (`text/html; charset=utf-8`), and only the type and subtype decide this.
/// `+json` and `+xml` catch the structured suffixes -- `application/problem+json`,
/// `image/svg+xml` -- which are text and do carry URLs.
const TEXTUAL: &[&str] = &[
    "text/",
    "application/json",
    "application/javascript",
    "application/xml",
    "application/xhtml+xml",
    "application/x-javascript",
    "application/graphql",
    "application/ld+json",
];

/// Whether a body with these headers is one to rewrite.
#[must_use]
pub fn is_rewritable(headers: &HeaderMap) -> bool {
    // A compressed body would have to be decompressed, rewritten and recompressed,
    // and the client asked the upstream for that encoding. Relayed as it came.
    if headers.contains_key(header::CONTENT_ENCODING) {
        return false;
    }

    let Some(kind) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        // No content type: nothing says this is text, and guessing by sniffing the
        // bytes is a second, worse rule to maintain.
        return false;
    };
    let kind = kind.trim().to_ascii_lowercase();
    TEXTUAL.iter().any(|prefix| kind.starts_with(prefix))
        || kind.contains("+json")
        || kind.contains("+xml")
}

/// The body, with the upstream's address replaced by Doppel's where it appears.
///
/// Returns the body to relay, and the new length when something was replaced. The
/// caller needs the second half: a rewritten body is not the entity the upstream
/// sent, so its length and its validators both have to be restated.
pub async fn rewrite_body<S>(
    stream: S,
    base: &reqwest::Url,
    external: &reqwest::Url,
    limit: u64,
) -> (Body, Option<usize>)
where
    S: futures_util::Stream<Item = reqwest::Result<Bytes>> + Send + Unpin + 'static,
{
    let mut stream = stream;
    let mut buffered: Vec<u8> = Vec::new();
    let cap = usize::try_from(limit).unwrap_or(usize::MAX);

    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            // A failed read is the caller's problem to see, and the stream is the
            // only thing that can report it: hand back what is left, errors and
            // all, rather than swallowing it into a truncated 200.
            let head = Bytes::from(buffered);
            return (Body::from_stream(prepend(head, stream)), None);
        };
        buffered.extend_from_slice(&chunk);
        if buffered.len() > cap {
            // Too big to hold. What was read is sent on in front of the rest,
            // untouched -- the alternative is buffering without a bound.
            tracing::debug!(
                bytes = buffered.len(),
                limit,
                "body over the limit; relayed without rewriting urls"
            );
            let head = Bytes::from(buffered);
            return (Body::from_stream(prepend(head, stream)), None);
        }
    }

    let text = match String::from_utf8(buffered) {
        Ok(text) => text,
        // Declared as text and not valid UTF-8. The bytes are already buffered, so
        // they go out exactly as they came: re-encoding somebody else's body is not
        // this proxy's business, and dropping it would be worse.
        Err(not_text) => return (Body::from(not_text.into_bytes()), None),
    };

    let (rewritten, changed) = replace_addresses(&text, base, external);
    let length = rewritten.len();
    (Body::from(rewritten), changed.then_some(length))
}

/// Put `head` back in front of `rest`.
fn prepend<S>(head: Bytes, rest: S) -> impl futures_util::Stream<Item = reqwest::Result<Bytes>>
where
    S: futures_util::Stream<Item = reqwest::Result<Bytes>>,
{
    futures_util::stream::once(async move { Ok(head) }).chain(rest.into_stream())
}

/// `text` with the upstream's base and origin replaced by Doppel's.
///
/// The base goes first and the origin second, so a URL under the proxied path
/// loses that prefix -- `https://api.example.com/v2/orders` becomes
/// `http://doppel/orders`, which is where Doppel actually serves it -- while a URL
/// on the same host outside that path keeps its own, the way a rewritten redirect
/// does.
fn replace_addresses(text: &str, base: &reqwest::Url, external: &reqwest::Url) -> (String, bool) {
    let to = external.as_str().trim_end_matches('/').to_owned();

    let (text, base_changed) = replace_at_boundary(text, base.as_str(), &format!("{to}/"));
    let origin = origin_of(base);
    let (text, origin_changed) = replace_at_boundary(&text, &origin, &to);
    (text, base_changed || origin_changed)
}

/// `scheme://host[:port]` of a URL, which is what a body writes when it names a
/// host without a path.
fn origin_of(url: &reqwest::Url) -> String {
    let mut origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    origin
}

/// Every occurrence of `needle` that is not the beginning of a longer hostname,
/// replaced by `replacement`.
///
/// The boundary check is the whole point. `https://api.example.com` is a prefix of
/// `https://api.example.com.evil.test`, and a plain string replacement would turn
/// that into Doppel's address with somebody else's suffix glued on. A character
/// that could continue a hostname -- a letter, a digit, a dot or a hyphen -- means
/// this is a different host, and it is left alone.
fn replace_at_boundary(text: &str, needle: &str, replacement: &str) -> (String, bool) {
    if needle.is_empty() {
        return (text.to_owned(), false);
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find(needle) {
        let after = &rest[at + needle.len()..];
        let continues_hostname = after
            .chars()
            .next()
            .is_some_and(|next| next.is_ascii_alphanumeric() || next == '.' || next == '-');
        out.push_str(&rest[..at]);
        if continues_hostname && !needle.ends_with('/') {
            out.push_str(needle);
        } else {
            out.push_str(replacement);
            changed = true;
        }
        rest = after;
    }
    out.push_str(rest);
    (out, changed)
}

/// Drop what a rewritten body invalidates, and state its new length.
///
/// The bytes are no longer the ones the upstream hashed, so `ETag` and the digest
/// headers are claims about an entity that no longer exists -- a conditional
/// request carrying that validator would be answered `304` for content the client
/// has never seen. `Content-Length` is the other half: it described the body
/// before the substitution.
pub fn restate_body_headers(headers: &mut HeaderMap, length: usize) {
    headers.remove(header::ETAG);
    headers.remove("content-md5");
    headers.remove("digest");
    headers.remove("repr-digest");
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> reqwest::Url {
        reqwest::Url::parse(text).expect("a test url parses")
    }

    #[test]
    fn a_url_under_the_proxied_path_loses_that_path() {
        // `/v2/orders` upstream is `/orders` through Doppel, which is what
        // `join_upstream` does in the other direction.
        let (out, changed) = replace_addresses(
            r#"{"next":"https://api.example.com/v2/orders?page=2"}"#,
            &url("https://api.example.com/v2/"),
            &url("http://127.0.0.1:8080/"),
        );

        assert!(changed);
        assert_eq!(out, r#"{"next":"http://127.0.0.1:8080/orders?page=2"}"#);
    }

    #[test]
    fn a_url_on_the_same_host_outside_the_path_keeps_its_own() {
        let (out, changed) = replace_addresses(
            r#"<a href="https://api.example.com/login">in</a>"#,
            &url("https://api.example.com/v2/"),
            &url("http://127.0.0.1:8080/"),
        );

        assert!(changed);
        assert_eq!(out, r#"<a href="http://127.0.0.1:8080/login">in</a>"#);
    }

    #[test]
    fn a_subdomain_is_a_different_host_and_is_left_alone() {
        let (out, changed) = replace_addresses(
            r#"<img src="https://cdn.api.example.com/logo.png">"#,
            &url("https://api.example.com/"),
            &url("http://127.0.0.1:8080/"),
        );

        assert!(!changed, "{out}");
        assert!(out.contains("cdn.api.example.com"), "{out}");
    }

    #[test]
    fn a_host_that_merely_starts_the_same_is_not_rewritten() {
        // The one that would be a security bug rather than a cosmetic one: a plain
        // string replacement turns this into Doppel's address with `.evil.test`
        // glued on the end, which is a URL pointing somewhere nobody named.
        let (out, changed) = replace_addresses(
            r#"<a href="https://api.example.com.evil.test/x">out</a>"#,
            &url("https://api.example.com/"),
            &url("http://127.0.0.1:8080/"),
        );

        assert!(!changed, "{out}");
        assert_eq!(
            out,
            r#"<a href="https://api.example.com.evil.test/x">out</a>"#
        );
    }

    #[test]
    fn a_port_is_part_of_the_host_it_matches() {
        let (out, changed) = replace_addresses(
            "see http://127.0.0.1:9000/thing",
            &url("http://127.0.0.1:9000/"),
            &url("https://doppel.example.com/"),
        );

        assert!(changed);
        assert_eq!(out, "see https://doppel.example.com/thing");
        // And a different port on the same address is a different upstream.
        let (out, changed) = replace_addresses(
            "see http://127.0.0.1:9001/thing",
            &url("http://127.0.0.1:9000/"),
            &url("https://doppel.example.com/"),
        );
        assert!(!changed, "{out}");
    }

    #[test]
    fn doppels_own_path_prefix_survives() {
        let (out, changed) = replace_addresses(
            "https://api.example.com/v2/orders",
            &url("https://api.example.com/v2/"),
            &url("https://gw.example.com/doppel/"),
        );

        assert!(changed);
        assert_eq!(out, "https://gw.example.com/doppel/orders");
    }

    #[test]
    fn a_body_with_nothing_to_replace_is_unchanged() {
        let (out, changed) = replace_addresses(
            "<p>no urls here</p>",
            &url("https://api.example.com/"),
            &url("http://127.0.0.1:8080/"),
        );

        assert!(!changed);
        assert_eq!(out, "<p>no urls here</p>");
    }

    #[test]
    fn only_text_and_only_uncompressed() {
        let textual = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
            is_rewritable(&headers)
        };

        assert!(textual("text/html; charset=utf-8"));
        assert!(textual("application/json"));
        assert!(textual("application/problem+json"));
        assert!(textual("image/svg+xml"));
        assert!(textual("TEXT/CSS"));
        assert!(!textual("image/png"));
        assert!(!textual("application/octet-stream"));
        assert!(!textual("font/woff2"));

        // No content type at all: nothing says this is text.
        assert!(!is_rewritable(&HeaderMap::new()));

        // Compressed, and the client asked the upstream for that encoding.
        let mut gzipped = HeaderMap::new();
        gzipped.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
        gzipped.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        assert!(!is_rewritable(&gzipped));
    }

    #[test]
    fn a_rewritten_body_loses_the_validators_it_invalidated() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ETAG, HeaderValue::from_static("\"abc\""));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("11"));
        headers.insert("digest", HeaderValue::from_static("sha-256=..."));

        restate_body_headers(&mut headers, 42);

        // A conditional request carrying the upstream's ETag would otherwise be
        // answered 304 for content the client has never seen.
        assert!(!headers.contains_key(header::ETAG));
        assert!(!headers.contains_key("digest"));
        assert_eq!(headers.get(header::CONTENT_LENGTH).unwrap(), "42");
    }
}
