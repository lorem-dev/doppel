//! Rules V5..V7, V10..V11, V14 and V15, plus dispatch into the mock rules.
//!
//! V9 (a positive timeout), V12 and V13 (a probability in 0..=1, a status in
//! 100..=599) and the non-negative half of V14 are gone: `TimeoutSeconds`,
//! `Ratio`, `HttpStatus` and `Seconds` refuse those values while the document
//! is being parsed. So is V33: `ByteSize` refuses a limit of zero, which is
//! what that rule said about `body_limit` and V29 said about `upload.limit`.
//! V8 and V32 are `config::UpstreamUrl`, which keeps the URL parsed rather
//! than as text -- so the request path never parses it again either.
//!
//! V35 is gone: it applied `sanitize` to a proxy name, and `config::Name` now
//! refuses the same shapes while the document is being parsed. One check, at
//! the moment the name comes into existence, rather than a type that admits a
//! bad name and a rule that catches it later.

use std::collections::BTreeSet;

use super::{Violations, is_valid_header_name, is_valid_header_value, mock};
use crate::config::{Config, ProxyKind, ResolveKind};

pub(super) fn check(config: &Config, v: &mut Violations) {
    // V5
    v.require(
        !config.proxies.is_empty(),
        "proxies",
        "at least one proxy is required",
    );

    let mut seen_names = BTreeSet::new();
    let mut default_seen = false;

    for (i, proxy) in config.proxies.iter().enumerate() {
        let path = format!("proxies[{i}]");

        // V6
        if !seen_names.insert(proxy.name.as_str()) {
            v.push(
                format!("{path}.name"),
                format!("duplicate proxy name `{}`", proxy.name),
            );
        }

        // V7
        if proxy.kind == ProxyKind::Tcp {
            v.push(
                format!("{path}.type"),
                "TCP proxying is not implemented yet",
            );
        }

        // V10 and V11
        match proxy.resolve.kind {
            ResolveKind::Default => {
                if default_seen {
                    v.push(
                        format!("{path}.resolve"),
                        "only one proxy may use `type: default`",
                    );
                }
                default_seen = true;
            }
            ResolveKind::Header => match proxy.resolve.header.as_deref() {
                None => v.push(
                    format!("{path}.resolve.header"),
                    "`header` is required when `type: header`",
                ),
                Some(header) => v.require(
                    is_valid_header_name(header),
                    format!("{path}.resolve.header"),
                    format!("`{header}` is not a valid header name"),
                ),
            },
        }

        // V14
        check_faults(proxy.latency.as_ref(), &path, v);

        // V15 -- both an invalid name and an invalid value are reported at
        // the same specific `headers.<name>` path, per the convention that
        // every message carries the config path of the thing it is about.
        for (name, value) in &proxy.headers {
            v.require(
                is_valid_header_name(name),
                format!("{path}.headers.{name}"),
                format!("`{name}` is not a valid header name"),
            );
            v.require(
                is_valid_header_value(value),
                format!("{path}.headers.{name}"),
                "value is not a valid header value",
            );
        }

        mock::check(proxy, &path, v);
    }
}

/// V14, shared by the proxy block and, via rule V25, by each mock's `proxy`
/// override.
///
/// One check is left of what used to be three rules. Every bound on a single
/// number -- the probabilities, the status, the sign of a latency -- is now a
/// type. What remains needs two fields at once, and no type over one value
/// can express it.
pub(super) fn check_faults(
    latency: Option<&crate::config::LatencyConfig>,
    path: &str,
    v: &mut Violations,
) {
    if let Some(latency) = latency {
        v.require(
            latency.min <= latency.max,
            format!("{path}.latency.min"),
            "min must be <= max",
        );
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
    timeout: 30
    resolve:
      type: default
    headers:
      Authorization: "Bearer x"
    loss:
      percentage: 0.1
      status: 503
    latency:
      percentage: 0.5
      min: 0.1
      max: 0.2
    replace: 1.0
"#
        .to_owned()
    }

    #[test]
    fn good_proxy_passes() {
        assert_eq!(validate(&load_from_str(&good()).unwrap()), Ok(()));
    }

    #[test]
    fn v5_proxies_must_not_be_empty() {
        let text = good().split("proxies:").next().unwrap().to_owned() + "proxies: []\n";
        assert_violation(&text, "proxies", "at least one proxy");
    }

    #[test]
    fn v6_proxy_names_must_be_unique() {
        let text = good() + "  - name: p1\n    type: http\n    url: \"https://other.com/\"\n";
        assert_violation(&text, "proxies[1].name", "duplicate proxy name `p1`");
    }

    #[test]
    fn a_proxy_name_that_is_not_a_usable_directory_component_fails_at_load() {
        // This was rule V35, applying `sanitize` to the name. It is now the
        // `Name` type, which refuses the same shapes while the document is
        // being parsed -- earlier, and in the one place a name comes into
        // existence. The claim is worth pinning at this level too, because
        // this is the level an operator reads: a bad name must stop the
        // document, not merely fail later at a template write.
        for (name, expected) in [
            ("a", "at least 2"),
            ("..", "must not start with a dot"),
            ("a/b", "contains"),
            ("a..b", "must not contain `..`"),
            (".hidden", "must not start with a dot"),
        ] {
            let text = good().replace("name: p1", &format!("name: '{name}'"));
            let err = load_from_str(&text)
                .expect_err(&format!("`{name}` must not parse"))
                .to_string();
            assert!(err.contains(expected), "`{name}`: got {err}");
        }
    }

    #[test]
    fn the_names_people_actually_use_still_load() {
        for name in ["p1", "ops", "billing-api", "billing_api", "Billing.API.v2"] {
            let text = good().replace("name: p1", &format!("name: '{name}'"));
            assert_eq!(
                validate(&load_from_str(&text).unwrap()),
                Ok(()),
                "`{name}` should be a legal proxy name"
            );
        }
    }

    #[test]
    fn v7_tcp_is_rejected_with_a_specific_message() {
        assert_violation(
            &good().replace("type: http", "type: tcp"),
            "proxies[0].type",
            "TCP proxying is not implemented yet",
        );
    }

    #[test]
    fn an_upstream_url_doppel_cannot_forward_over_fails_at_load() {
        // V8 and V32, now `config::UpstreamUrl`. The last of them is the
        // reason the type exists rather than just a check: the forwarding
        // path replaces the query with the incoming request's, so a query
        // configured here is discarded on every request rather than merged.
        for (bad, expected) in [
            ("/api", "must be absolute"),
            ("ftp://example.com/", "http or https"),
            ("https://example.com/?key=abc", "query string or fragment"),
            ("https://example.com/#frag", "query string or fragment"),
        ] {
            let text = good().replace("https://example.com/", bad);
            let err = load_from_str(&text)
                .expect_err(&format!("`{bad}` must not parse"))
                .to_string();
            assert!(err.contains(expected), "{bad}: {err}");
        }
    }

    #[test]
    fn a_timeout_that_is_not_a_timeout_fails_at_load() {
        // This was V9, now `config::TimeoutSeconds`. The bound gained an
        // upper end with the type: 30000 is a timeout written in
        // milliseconds, and 0 is not a timeout at all.
        for (bad, expected) in [
            ("0", "leave `timeout` out"),
            ("-1", "must not be negative"),
            ("30000", "seconds, not milliseconds"),
        ] {
            let text = good().replace("timeout: 30", &format!("timeout: {bad}"));
            let err = load_from_str(&text)
                .expect_err(&format!("timeout {bad} must not parse"))
                .to_string();
            assert!(err.contains(expected), "{bad}: {err}");
        }
    }

    #[test]
    fn v10_at_most_one_default_proxy() {
        let text = good()
            + "  - name: p2\n    type: http\n    url: \"https://other.com/\"\n    resolve:\n      type: default\n";
        assert_violation(
            &text,
            "proxies[1].resolve",
            "only one proxy may use `type: default`",
        );
    }

    #[test]
    fn v10_zero_defaults_is_legal() {
        let text = good().replace(
            "      type: default",
            "      type: header\n      header: X-Proxy-Name",
        );
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    #[test]
    fn v11_header_resolution_requires_a_valid_header() {
        assert_violation(
            &good().replace("      type: default", "      type: header"),
            "proxies[0].resolve.header",
            "required when `type: header`",
        );
        assert_violation(
            &good().replace(
                "      type: default",
                "      type: header\n      header: \"bad header\"",
            ),
            "proxies[0].resolve.header",
            "not a valid header name",
        );
    }

    #[test]
    fn a_probability_outside_zero_to_one_fails_at_load() {
        // This was V12, now `config::Ratio`. The message gained the hint the
        // rule never had: someone writing `50` meant fifty percent.
        for (field, from, to) in [
            ("loss.percentage", "percentage: 0.1", "percentage: 1.5"),
            ("replace", "replace: 1.0", "replace: -0.1"),
            ("loss.percentage", "percentage: 0.1", "percentage: 50"),
        ] {
            let err = load_from_str(&good().replace(from, to))
                .expect_err(&format!("{field} = {to} must not parse"))
                .to_string();
            assert!(err.contains("between 0.0 and 1.0"), "{to}: {err}");
            assert!(err.contains("50% is `0.5`"), "{to}: {err}");
        }
    }

    #[test]
    fn a_loss_status_outside_the_http_range_fails_at_load() {
        // This was the second half of V13. `config::HttpStatus` refuses the
        // value while the document is being parsed; the claim that a document
        // carrying one must not load is still worth pinning here.
        for bad in ["99", "600", "700"] {
            let text = good().replace("status: 503", &format!("status: {bad}"));
            let err = load_from_str(&text)
                .expect_err(&format!("status {bad} must not parse"))
                .to_string();
            assert!(err.contains("100 to 599"), "{bad}: {err}");
        }
    }

    #[test]
    fn v14_latency_bounds_must_be_ordered() {
        // All that is left of V14. Ordering needs both fields, so it stays a
        // rule; the sign and the upper bound moved into `config::Seconds`.
        assert_violation(
            &good().replace("min: 0.1", "min: 0.9"),
            "proxies[0].latency.min",
            "must be <= max",
        );
    }

    #[test]
    fn a_latency_that_is_not_a_latency_fails_at_load() {
        for (bad, expected) in [
            ("-1.0", "must not be negative"),
            ("500.0", "300 second maximum"),
        ] {
            let err = load_from_str(&good().replace("min: 0.1", &format!("min: {bad}")))
                .expect_err(&format!("min {bad} must not parse"))
                .to_string();
            assert!(err.contains(expected), "{bad}: {err}");
        }
    }

    #[test]
    fn v15_upstream_headers_must_be_well_formed() {
        assert_violation(
            &good().replace("      Authorization:", "      \"Bad Header\":"),
            "proxies[0].headers.Bad Header",
            "not a valid header name",
        );
        assert_violation(
            &good().replace(r#""Bearer x""#, r#""bad\nvalue""#),
            "proxies[0].headers.Authorization",
            "not a valid header value",
        );
    }

    #[test]
    fn a_body_limit_that_is_not_a_limit_fails_at_load() {
        // This was V33, and V29 said the same thing about `upload.limit`.
        // Both are now one check in `ByteSize`.
        for bad in ["0", "-1", "2Gi"] {
            let text = good() + &format!("    body_limit: {bad}\n");
            assert!(
                load_from_str(&text).is_err(),
                "`body_limit: {bad}` must not load"
            );
        }
    }
}
