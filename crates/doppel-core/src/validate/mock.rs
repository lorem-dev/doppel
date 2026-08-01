//! Rules V16..V25 and V30.

use std::collections::BTreeSet;

use super::{Violations, is_valid_header_name};
use crate::config::{MockRequest, MockResponse, ProxyConfig};

/// Methods Doppel will match on. Deliberately a closed list: a typo like
/// `FETCH` should be a config error, not a mock that silently never matches.
const KNOWN_METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT",
];

/// Statuses that must not carry a body, per RFC 9110.
const BODILESS_STATUSES: [u16; 2] = [204, 304];

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
            super::proxy::check_faults(
                over.loss.as_ref(),
                over.latency.as_ref(),
                over.replace,
                &format!("{path}.proxy"),
                v,
            );
        }
    }
}

fn check_request(request: &MockRequest, path: &str, v: &mut Violations) {
    // V17
    let method = request.method.to_ascii_uppercase();
    v.require(
        KNOWN_METHODS.contains(&method.as_str()),
        format!("{path}.request.method"),
        format!("unknown HTTP method `{}`", request.method),
    );

    // V18
    let captures: BTreeSet<String> = match regex::Regex::new(&request.url) {
        Ok(re) => re.capture_names().flatten().map(str::to_owned).collect(),
        Err(err) => {
            v.push(
                format!("{path}.request.url"),
                format!("`{}` is not a valid regex: {err}", request.url),
            );
            BTreeSet::new()
        }
    };

    // V19 and V24
    for (variable, header) in &request.headers {
        v.require(
            !captures.contains(variable),
            format!("{path}.request.headers.{variable}"),
            format!("variable `{variable}` collides with a capture group in `url`"),
        );
        v.require(
            is_valid_header_name(header),
            format!("{path}.request.headers.{variable}"),
            format!("`{header}` is not a valid header name"),
        );
    }

    // V19 and V23
    for (source, selectors) in [("query", &request.query), ("body", &request.body)] {
        for (variable, selector) in selectors {
            v.require(
                !captures.contains(variable),
                format!("{path}.request.{source}.{variable}"),
                format!("variable `{variable}` collides with a capture group in `url`"),
            );
            if let Err(reason) = check_selector(selector) {
                v.push(format!("{path}.request.{source}.{variable}"), reason);
            }
        }
    }
}

/// V23: a selector is a leading `.` followed by non-empty dot-separated segments.
fn check_selector(selector: &str) -> Result<(), String> {
    let Some(rest) = selector.strip_prefix('.') else {
        return Err(format!("selector `{selector}` must start with `.`"));
    };
    if rest.is_empty() {
        return Err("selector must name at least one field".to_owned());
    }
    if rest.split('.').any(str::is_empty) {
        return Err(format!("selector `{selector}` has an empty path segment"));
    }
    Ok(())
}

fn check_response(response: &MockResponse, path: &str, v: &mut Violations) {
    // V22
    v.require(
        (100..=599).contains(&response.status),
        format!("{path}.response.status"),
        "must be between 100 and 599",
    );

    // V20
    v.require(
        response.body_sources() <= 1,
        format!("{path}.response"),
        "at most one of `body`, `json`, `template` may be set",
    );

    // V30
    if BODILESS_STATUSES.contains(&response.status) {
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
    limit: 1M
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
            requestId: X-Request-ID
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
    fn v17_method_must_be_known() {
        assert_violation(
            &good().replace("method: GET", "method: FETCH"),
            "proxies[0].mocks[0].request.method",
            "unknown HTTP method `FETCH`",
        );
    }

    #[test]
    fn v18_url_must_compile_as_a_regex() {
        assert_violation(
            &good().replace(r"/api/(?P<id>\d+)/", "/api/(unclosed/"),
            "proxies[0].mocks[0].request.url",
            "is not a valid regex",
        );
    }

    #[test]
    fn v19_capture_groups_must_not_collide_with_declared_variables() {
        let text = good().replace(
            "            requestId: X-Request-ID",
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
    fn v22_status_must_be_a_real_status() {
        assert_violation(
            &good().replace("status: 200", "status: 700"),
            "proxies[0].mocks[0].response.status",
            "between 100 and 599",
        );
    }

    #[test]
    fn v23_selectors_must_be_well_formed() {
        assert_violation(
            &good().replace("filter: .filter", "filter: filter"),
            "proxies[0].mocks[0].request.query.filter",
            "must start with `.`",
        );
        assert_violation(
            &good().replace(".content.name", ".content..name"),
            "proxies[0].mocks[0].request.body.itemName",
            "empty path segment",
        );
    }

    #[test]
    fn v24_declared_header_sources_must_be_header_names() {
        assert_violation(
            &good().replace("requestId: X-Request-ID", "requestId: \"X Request ID\""),
            "proxies[0].mocks[0].request.headers.requestId",
            "not a valid header name",
        );
    }

    #[test]
    fn v25_mock_fault_override_obeys_the_same_bounds() {
        let text = good()
            + "        proxy:\n          replace: 2.0\n          latency:\n            percentage: 0.5\n            min: 0.9\n            max: 0.1\n";
        assert_violation(
            &text,
            "proxies[0].mocks[0].proxy.replace",
            "between 0.0 and 1.0",
        );
        assert_violation(
            &text,
            "proxies[0].mocks[0].proxy.latency.min",
            "min must be <= max",
        );
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
    fn v30_304_without_a_body_passes() {
        let text = good()
            .replace("status: 200", "status: 304")
            .replace(r#"          json: '{"id": "{{ id }}"}'"#, "");
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }
}
