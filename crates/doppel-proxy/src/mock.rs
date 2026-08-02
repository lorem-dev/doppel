//! Matching a request against a proxy's mocks, and binding its variables.

use axum::http::{HeaderMap, Method};
use doppel_core::{CompiledMock, CompiledProxy, Error};
use doppel_render::{Selector, Variables};

/// Finds the first mock, in configuration order, whose method and path match
/// the request, and binds its named capture groups into [`Variables`].
///
/// A mock matches when its method equals `method` exactly -- case-sensitive,
/// since V17 already rejects a lowercase method at config load -- and its
/// pattern matches somewhere in `path`.
///
/// The pattern is unanchored, deliberately (spec section 4, section 10): a
/// pattern for `/api/v1/resource/` also matches `/api/v1/resource/42/`,
/// because it matches any path containing that substring, and only
/// declaration order distinguishes the two. Do not "fix" this by anchoring
/// the pattern here -- doing so would silently change what an existing,
/// unedited configuration means.
pub fn match_mock<'a>(
    proxy: &'a CompiledProxy,
    method: &Method,
    path: &str,
) -> Option<(&'a CompiledMock, Variables)> {
    proxy.mocks.iter().find_map(|mock| {
        if mock.method != method.as_str() {
            return None;
        }

        let captures = mock.pattern.captures(path)?;

        let mut vars = Variables::new();
        for name in &mock.capture_names {
            if let Some(value) = captures.name(name) {
                vars.insert(name, serde_json::Value::String(value.as_str().to_owned()));
            }
        }

        Some((mock, vars))
    })
}

/// Binds the mock's declared header variables. A header the request does not
/// carry binds nothing -- referencing it in a template is an undefined
/// variable, per section 5 of the design.
pub fn bind_headers(mock: &CompiledMock, headers: &HeaderMap, vars: &mut Variables) {
    for (name, header) in &mock.header_vars {
        if let Some(value) = headers.get(header.as_str()).and_then(|v| v.to_str().ok()) {
            vars.insert(name, serde_json::Value::String(value.to_owned()));
        }
    }
}

/// Binds the mock's declared query variables. The raw query string is parsed
/// into a flat JSON object first, so query selectors are evaluated with
/// exactly the same `Selector` grammar and code path as body selectors --
/// `filter: .filter` addresses the `filter` key the same way a body selector
/// addresses a JSON object's key.
///
/// Percent-decoding goes through `reqwest::Url::query_pairs`, an inherent
/// method on the `Url` type `reqwest` already re-exports as `reqwest::Url`
/// (see `doppel_core::runtime::compile_proxy`'s own use of it) -- this needs
/// no dependency on the `url` or `form_urlencoded` crates directly. A key
/// repeated in the query string keeps its first value, matching this
/// codebase's existing convention for a repeated header (see the
/// `x-forwarded-for` handling in `upstream.rs`).
pub fn bind_query(
    mock: &CompiledMock,
    query: Option<&str>,
    vars: &mut Variables,
) -> Result<(), Error> {
    if mock.query_vars.is_empty() {
        return Ok(());
    }
    let root = query_object(query.unwrap_or(""));
    bind_selectors(&mock.query_vars, &root, vars)
}

/// Binds the mock's declared body variables against an already-parsed body.
/// The caller is responsible for buffering and parsing the body only when
/// `mock.body_vars` is non-empty, per section 6 of the design.
pub fn bind_body(
    mock: &CompiledMock,
    root: &serde_json::Value,
    vars: &mut Variables,
) -> Result<(), Error> {
    bind_selectors(&mock.body_vars, root, vars)
}

fn bind_selectors(
    pairs: &[(String, String)],
    root: &serde_json::Value,
    vars: &mut Variables,
) -> Result<(), Error> {
    for (name, raw_selector) in pairs {
        let selector = Selector::parse(raw_selector)?;
        if let Some(value) = selector.eval(root) {
            vars.insert(name, value.clone());
        }
    }
    Ok(())
}

/// A URL good only for giving `Url::parse` an absolute form to parse the raw
/// query string against; nothing about it (scheme, host) is ever inspected.
fn query_object(query: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Ok(url) = reqwest::Url::parse(&format!("http://doppel.invalid/?{query}")) {
        for (key, value) in url.query_pairs() {
            map.entry(key.into_owned())
                .or_insert_with(|| serde_json::Value::String(value.into_owned()));
        }
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use doppel_core::MockBody;

    /// Builds a `CompiledMock` the way `Runtime::compile` would, without going
    /// through config parsing: a compiled pattern and its named capture
    /// groups in declaration order.
    fn mock(name: &str, method: &str, pattern: &str) -> CompiledMock {
        let regex = regex::Regex::new(pattern).unwrap();
        let capture_names = regex.capture_names().flatten().map(str::to_owned).collect();
        CompiledMock {
            name: name.to_owned(),
            method: method.to_owned(),
            pattern: regex,
            capture_names,
            header_vars: Vec::new(),
            query_vars: Vec::new(),
            body_vars: Vec::new(),
            status: 200,
            body: MockBody::None,
            headers: Vec::new(),
            replace: None,
            loss: None,
            latency: None,
        }
    }

    fn proxy(mocks: Vec<CompiledMock>) -> CompiledProxy {
        CompiledProxy {
            name: "p1".to_owned(),
            base_url: reqwest::Url::parse("https://example.com/").unwrap(),
            timeout: Duration::from_secs(5),
            headers: Vec::new(),
            loss: None,
            latency: None,
            replace: 1.0,
            resolve_header: None,
            mocks,
            body_limit: 1024 * 1024,
        }
    }

    #[test]
    fn matches_on_method_and_path() {
        let p = proxy(vec![mock("m1", "GET", "^/widgets/[0-9]+/$")]);
        let (matched, _vars) = match_mock(&p, &Method::GET, "/widgets/42/").unwrap();
        assert_eq!(matched.name, "m1");
    }

    #[test]
    fn a_mismatched_method_does_not_match_even_though_the_path_does() {
        let p = proxy(vec![mock("m1", "GET", "^/widgets/[0-9]+/$")]);
        assert!(match_mock(&p, &Method::POST, "/widgets/42/").is_none());
    }

    #[test]
    fn a_mismatched_path_does_not_match_even_though_the_method_does() {
        let p = proxy(vec![mock("m1", "GET", "^/widgets/[0-9]+/$")]);
        assert!(match_mock(&p, &Method::GET, "/other/").is_none());
    }

    /// The pattern is unanchored, deliberately (spec section 4 and section
    /// 10): a pattern for `/api/v1/resource/` also matches
    /// `/api/v1/resource/42/`, because it matches any path containing that
    /// substring. This pins the behaviour so it is not later "fixed" by
    /// adding anchors, which would silently change what an existing,
    /// unedited configuration means.
    #[test]
    fn an_unanchored_pattern_matches_a_path_that_merely_contains_it() {
        let p = proxy(vec![mock("m1", "GET", "/api/v1/resource/")]);
        let (matched, _vars) = match_mock(&p, &Method::GET, "/api/v1/resource/42/").unwrap();
        assert_eq!(matched.name, "m1");
    }

    /// Fixture names are adversarial on purpose: `zeta` is declared first and
    /// `alpha` second, so a regression that sorted mocks (alphabetically, or
    /// by any other accidental order) rather than preserving declaration
    /// order would return `alpha` here and this test would catch it.
    #[test]
    fn the_first_declared_mock_wins_when_both_patterns_match() {
        let p = proxy(vec![
            mock("zeta", "GET", "/api/v1/resource/"),
            mock("alpha", "GET", "/api/v1/resource/42/"),
        ]);
        let (matched, _vars) = match_mock(&p, &Method::GET, "/api/v1/resource/42/").unwrap();
        assert_eq!(
            matched.name, "zeta",
            "zeta is declared first and both patterns match; declaration order must win"
        );
    }

    /// Capture group names are adversarial on purpose: `z` is declared before
    /// `a` in the pattern, the reverse of alphabetical order, so a regression
    /// that bound captures by sorted name rather than declaration order (or
    /// mixed up which value goes with which name) would be caught here.
    #[test]
    fn capture_groups_bind_by_name_in_declaration_order() {
        let p = proxy(vec![mock(
            "m1",
            "GET",
            "^/widgets/(?P<z>[0-9]+)/(?P<a>[a-z]+)/$",
        )]);
        let (_matched, vars) = match_mock(&p, &Method::GET, "/widgets/42/red/").unwrap();
        let ctx = vars.as_context();
        assert_eq!(ctx.get_attr("z").unwrap().as_str(), Some("42"));
        assert_eq!(ctx.get_attr("a").unwrap().as_str(), Some("red"));
    }

    #[test]
    fn a_mock_with_no_captures_binds_nothing() {
        let p = proxy(vec![mock("m1", "GET", "^/health/$")]);
        let (_matched, vars) = match_mock(&p, &Method::GET, "/health/").unwrap();
        assert_eq!(vars, Variables::new());
    }

    #[test]
    fn a_proxy_with_no_mocks_returns_none() {
        let p = proxy(Vec::new());
        assert!(match_mock(&p, &Method::GET, "/anything/").is_none());
    }
}
