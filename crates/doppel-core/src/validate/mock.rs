//! Rules V16, V19..V21, V25 and V30.
//!
//! V17 (a known, upper-case method), V18 (a compilable url pattern), V22 (a
//! status in 100..=599), V23 (a well-formed selector) and V24 (a header name)
//! are gone: `HttpMethod`, `Pattern`, `HttpStatus`, `Selector` and
//! `HeaderName` refuse the same values while the document is being parsed,
//! with the same messages. So does V31: `TemplateName` is the rule the store
//! applies to an uploaded file name, and a mock naming a template is held to
//! it at parse time rather than by a copy of it here. What is left needs more
//! than one field to decide.

use std::collections::BTreeSet;

use super::Violations;
use crate::config::{MockRequest, MockResponse, ProxyConfig};

pub(super) fn check(proxy: &ProxyConfig, proxy_path: &str, v: &mut Violations) {
    let mut seen = BTreeSet::new();

    for (i, mock) in proxy.mocks.iter().enumerate() {
        let path = format!("{proxy_path}.mocks[{i}]");

        // V16
        if !seen.insert(mock.name.as_str()) {
            v.push(
                format!("{path}.name"),
                format!("duplicate mock name `{}`", mock.name),
            );
        }

        check_request(&mock.request, &path, v);
        check_response(&mock.response, &path, v);

        // V25
        if let Some(over) = &mock.proxy {
            super::proxy::check_faults(over.latency.as_ref(), &format!("{path}.proxy"), v);
        }
    }
}

fn check_request(request: &MockRequest, path: &str, v: &mut Violations) {
    // No V18 here: `config::Pattern` compiled it at parse time, so what is
    // left is reading the capture names off a regex that is known good.
    let captures: BTreeSet<String> = request.url.capture_names().into_iter().collect();

    // V19. V24 is `config::HeaderName`, on the value side of this map.
    for variable in request.headers.keys() {
        v.require(
            !captures.contains(variable),
            format!("{path}.request.headers.{variable}"),
            format!("variable `{variable}` collides with a capture group in `url`"),
        );
    }

    // V19. V23 is `config::Selector`.
    for (source, selectors) in [("query", &request.query), ("body", &request.body)] {
        for variable in selectors.keys() {
            v.require(
                !captures.contains(variable),
                format!("{path}.request.{source}.{variable}"),
                format!("variable `{variable}` collides with a capture group in `url`"),
            );
        }
    }
}

fn check_response(response: &MockResponse, path: &str, v: &mut Violations) {
    // V20
    v.require(
        response.body_sources() <= 1,
        format!("{path}.response"),
        "at most one of `body`, `json`, `template` may be set",
    );

    // V30
    if response.status.forbids_a_body() {
        v.require(
            response.body_sources() == 0,
            format!("{path}.response"),
            format!("status {} forbids a body", response.status),
        );
    }

    // V21
    let env = minijinja::Environment::new();
    for (field, source) in [("body", &response.body), ("json", &response.json)] {
        if let Some(source) = source
            && let Err(err) = env.template_from_str(source)
        {
            v.push(
                format!("{path}.response.{field}"),
                format!("`{field}` is not a valid template: {err}"),
            );
        }
    }
    for (name, template) in &response.headers {
        if let Err(err) = env.template_from_str(template) {
            v.push(
                format!("{path}.response.headers.{name}"),
                format!("header template is not a valid template: {err}"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::load_from_str;
    use crate::validate::test_support::assert_violation;
    use crate::validate::validate;

    fn good() -> String {
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
  - name: p1
    type: http
    url: "https://example.com/"
    mocks:
      - name: m1
        request:
          method: GET
          url: /api/(?P<id>\d+)/
          headers:
            request_id: X-Request-ID
          query:
            filter: .filter
          body:
            itemName: .content.name
        response:
          status: 200
          json: '{"id": "{{ id }}"}'
          headers:
            X-Id: "{{ id }}"
"#
        .to_owned()
    }

    #[test]
    fn good_mock_passes() {
        assert_eq!(validate(&load_from_str(&good()).unwrap()), Ok(()));
    }

    #[test]
    fn v16_mock_names_must_be_unique_within_a_proxy() {
        let text = good()
            + "      - name: m1\n        request:\n          method: PUT\n          url: /x/\n        response:\n          status: 200\n";
        assert_violation(
            &text,
            "proxies[0].mocks[1].name",
            "duplicate mock name `m1`",
        );
    }

    #[test]
    fn an_unknown_method_fails_at_load() {
        // This was V17. `config::HttpMethod` refuses the value while the
        // document is being parsed, with the same message; what is asserted
        // here is that the document does not load, and that the message still
        // says the list is a typo guard rather than a protocol restriction.
        let err = load_from_str(&good().replace("method: GET", "method: FETCH"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a method Doppel knows"), "{err}");
        assert!(err.contains("add it"), "{err}");
    }

    #[test]
    fn query_is_a_known_method() {
        assert_eq!(
            validate(&load_from_str(&good().replace("method: GET", "method: QUERY")).unwrap()),
            Ok(())
        );
    }

    #[test]
    fn webdav_methods_are_known() {
        assert_eq!(
            validate(&load_from_str(&good().replace("method: GET", "method: PROPFIND")).unwrap()),
            Ok(())
        );
    }

    #[test]
    fn a_lower_case_method_fails_at_load_and_says_what_to_write() {
        // The other half of V17, and the reason the method type hand-writes
        // its `Deserialize`: a derived one would answer `get` by listing
        // seventeen variants rather than naming the one that was meant.
        let err = load_from_str(&good().replace("method: GET", "method: get"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("case-sensitive"), "{err}");
        assert!(err.contains("use `GET`"), "{err}");
    }

    #[test]
    fn a_url_pattern_that_does_not_compile_fails_at_load() {
        // This was V18. `config::Pattern` compiles it at parse time and keeps
        // the result, so the compile that used to happen twice -- once to
        // check, once for real -- happens once.
        let err = load_from_str(&good().replace(r"/api/(?P<id>\d+)/", "/api/(unclosed/"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a valid regex"), "{err}");
    }

    #[test]
    fn v19_capture_groups_must_not_collide_with_declared_variables() {
        let text = good().replace(
            "            request_id: X-Request-ID",
            "            id: X-Request-ID",
        );
        assert_violation(
            &text,
            "proxies[0].mocks[0].request.headers.id",
            "collides with a capture group",
        );
    }

    #[test]
    fn v19_capture_groups_must_not_collide_with_declared_query_variables() {
        let text = good().replace("filter: .filter", "id: .filter");
        assert_violation(
            &text,
            "proxies[0].mocks[0].request.query.id",
            "collides with a capture group",
        );
    }

    #[test]
    fn v19_capture_groups_must_not_collide_with_declared_body_variables() {
        let text = good().replace("itemName: .content.name", "id: .content.name");
        assert_violation(
            &text,
            "proxies[0].mocks[0].request.body.id",
            "collides with a capture group",
        );
    }

    #[test]
    fn v20_at_most_one_body_source() {
        let text = good().replace(
            r#"          json: '{"id": "{{ id }}"}'"#,
            "          json: '{}'\n          body: 'x'",
        );
        assert_violation(
            &text,
            "proxies[0].mocks[0].response",
            "at most one of `body`, `json`, `template`",
        );
    }

    #[test]
    fn v20_none_is_allowed() {
        let text = good().replace(r#"          json: '{"id": "{{ id }}"}'"#, "");
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    #[test]
    fn v21_templates_must_parse() {
        assert_violation(
            &good().replace(r#"json: '{"id": "{{ id }}"}'"#, r#"json: '{{ id '"#),
            "proxies[0].mocks[0].response.json",
            "is not a valid template",
        );
        assert_violation(
            &good().replace(r#"X-Id: "{{ id }}""#, r#"X-Id: "{% if %}""#),
            "proxies[0].mocks[0].response.headers.X-Id",
            "is not a valid template",
        );
    }

    #[test]
    fn a_status_outside_the_http_range_fails_at_load() {
        // This was V22, now `config::HttpStatus`.
        for bad in ["99", "600", "700"] {
            let text = good().replace("status: 200", &format!("status: {bad}"));
            let err = load_from_str(&text)
                .expect_err(&format!("status {bad} must not parse"))
                .to_string();
            assert!(err.contains("100 to 599"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_malformed_selector_fails_at_load() {
        // This was V23, and `doppel_render::Selector` parsed the same grammar
        // a second time per request. Both are `config::Selector` now.
        for (from, to, expected) in [
            ("filter: .filter", "filter: filter", "must start with `.`"),
            (".content.name", ".content..name", "empty path segment"),
            ("filter: .filter", "filter: .", "at least one field"),
        ] {
            let err = load_from_str(&good().replace(from, to))
                .expect_err(&format!("`{to}` must not parse"))
                .to_string();
            assert!(err.contains(expected), "{to}: {err}");
        }
    }

    #[test]
    fn a_declared_header_source_that_is_not_a_header_name_fails_at_load() {
        // This was V24, now the value type of the `headers` map.
        let text = good().replace("request_id: X-Request-ID", "request_id: \"X Request ID\"");
        let err = load_from_str(&text).unwrap_err().to_string();
        assert!(err.contains("not a valid header name"), "{err}");
    }

    #[test]
    fn a_response_header_name_is_checked_too() {
        // The gap V15 and V24 left between them: neither rule looked at a
        // mock's response header names, so `X Id:` was a configuration that
        // loaded, validated, and then produced a header no client could
        // parse. The type reaches it because it is the same type.
        let text = good().replace(r#"X-Id: "{{ id }}""#, r#""X Id": "{{ id }}""#);
        let err = load_from_str(&text).unwrap_err().to_string();
        assert!(err.contains("not a valid header name"), "{err}");
    }

    #[test]
    fn v25_mock_fault_override_obeys_the_same_bounds() {
        // V25 is the claim that a mock's `proxy` block is held to the same
        // standard as the proxy's own. Both halves of that are still true;
        // they now arrive through different doors, so the test uses both.
        let ordering = good()
            + "        proxy:\n          latency:\n            percentage: 0.5\n            min: 0.9\n            max: 0.1\n";
        assert_violation(
            &ordering,
            "proxies[0].mocks[0].proxy.latency.min",
            "min must be <= max",
        );

        let out_of_range = good() + "        proxy:\n          replace: 2.0\n";
        let err = load_from_str(&out_of_range)
            .expect_err("a replace of 2.0 must not parse")
            .to_string();
        assert!(err.contains("between 0.0 and 1.0"), "{err}");
    }

    #[test]
    fn v30_bodiless_statuses_must_declare_no_body() {
        let text = good().replace("status: 200", "status: 204");
        assert_violation(
            &text,
            "proxies[0].mocks[0].response",
            "status 204 forbids a body",
        );
    }

    #[test]
    fn v30_bodiless_status_without_a_body_passes() {
        let text = good()
            .replace("status: 200", "status: 204")
            .replace(r#"          json: '{"id": "{{ id }}"}'"#, "");
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    #[test]
    fn v30_304_must_declare_no_body() {
        let text = good().replace("status: 200", "status: 304");
        assert_violation(
            &text,
            "proxies[0].mocks[0].response",
            "status 304 forbids a body",
        );
    }

    #[test]
    fn v31_a_normal_template_name_passes() {
        let text = good()
            .replace(r#"          json: '{"id": "{{ id }}"}'"#, "")
            .replace(
                "          status: 200",
                "          status: 200\n          template: put.json.j2",
            );
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    #[test]
    fn a_traversal_template_name_fails_at_load() {
        // This was V31. `config::TemplateName` is the rule, and the store's
        // upload path asks the same type -- so a name a mock may declare and
        // a name a client may upload can no longer disagree.
        for (bad, expected) in [
            ("../../etc/passwd", "must not start with a dot"),
            ("sub/dir.j2", "path separator"),
            (".hidden.j2", "must not start with a dot"),
        ] {
            let text = good()
                .replace(r#"          json: '{"id": "{{ id }}"}'"#, "")
                .replace(
                    "          status: 200",
                    &format!("          status: 200\n          template: \"{bad}\""),
                );
            let err = load_from_str(&text)
                .expect_err(&format!("`{bad}` must not parse"))
                .to_string();
            assert!(err.contains(expected), "{bad}: {err}");
        }
    }

    #[test]
    fn v30_304_without_a_body_passes() {
        let text = good()
            .replace("status: 200", "status: 304")
            .replace(r#"          json: '{"id": "{{ id }}"}'"#, "");
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }
}
