//! Rules V5..V15, plus dispatch into the mock rules.

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

        // V8
        match reqwest::Url::parse(&proxy.url) {
            Ok(url) => v.require(
                matches!(url.scheme(), "http" | "https"),
                format!("{path}.url"),
                "url scheme must be http or https",
            ),
            Err(err) => {
                v.push(
                    format!("{path}.url"),
                    format!("url must be absolute: {err}"),
                );
            }
        }

        // V9
        if let Some(timeout) = proxy.timeout {
            v.require(
                timeout > 0,
                format!("{path}.timeout"),
                "timeout must be greater than 0",
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

        // V12, V13, V14
        check_faults(
            proxy.loss.as_ref(),
            proxy.latency.as_ref(),
            proxy.replace,
            &path,
            v,
        );

        // V15
        for (name, value) in &proxy.headers {
            v.require(
                is_valid_header_name(name),
                format!("{path}.headers"),
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

/// Shared by the proxy block and, via rule V25, by each mock's `proxy` override.
pub(super) fn check_faults(
    loss: Option<&crate::config::LossConfig>,
    latency: Option<&crate::config::LatencyConfig>,
    replace: Option<f64>,
    path: &str,
    v: &mut Violations,
) {
    if let Some(loss) = loss {
        v.require(
            is_ratio(loss.percentage),
            format!("{path}.loss.percentage"),
            "must be between 0.0 and 1.0",
        );
        v.require(
            (100..=599).contains(&loss.status),
            format!("{path}.loss.status"),
            "must be between 100 and 599",
        );
    }
    if let Some(latency) = latency {
        v.require(
            is_ratio(latency.percentage),
            format!("{path}.latency.percentage"),
            "must be between 0.0 and 1.0",
        );
        v.require(
            latency.min >= 0.0,
            format!("{path}.latency.min"),
            "must be >= 0",
        );
        v.require(
            latency.max >= 0.0,
            format!("{path}.latency.max"),
            "must be >= 0",
        );
        v.require(
            latency.min <= latency.max,
            format!("{path}.latency.min"),
            "min must be <= max",
        );
    }
    if let Some(replace) = replace {
        v.require(
            is_ratio(replace),
            format!("{path}.replace"),
            "must be between 0.0 and 1.0",
        );
    }
}

fn is_ratio(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
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
    fn v7_tcp_is_rejected_with_a_specific_message() {
        assert_violation(
            &good().replace("type: http", "type: tcp"),
            "proxies[0].type",
            "TCP proxying is not implemented yet",
        );
    }

    #[test]
    fn v8_url_must_be_absolute_http_or_https() {
        assert_violation(
            &good().replace(r#""https://example.com/""#, r#""/api""#),
            "proxies[0].url",
            "absolute",
        );
        assert_violation(
            &good().replace("https://example.com/", "ftp://example.com/"),
            "proxies[0].url",
            "http or https",
        );
    }

    #[test]
    fn v9_timeout_must_be_positive() {
        assert_violation(
            &good().replace("timeout: 30", "timeout: 0"),
            "proxies[0].timeout",
            "greater than 0",
        );
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
    fn v12_percentages_are_bounded() {
        assert_violation(
            &good().replace("percentage: 0.1", "percentage: 1.5"),
            "proxies[0].loss.percentage",
            "between 0.0 and 1.0",
        );
        assert_violation(
            &good().replace("replace: 1.0", "replace: -0.1"),
            "proxies[0].replace",
            "between 0.0 and 1.0",
        );
    }

    #[test]
    fn v13_loss_status_must_be_a_real_status() {
        assert_violation(
            &good().replace("status: 503", "status: 99"),
            "proxies[0].loss.status",
            "between 100 and 599",
        );
    }

    #[test]
    fn v14_latency_bounds_must_be_ordered_and_nonnegative() {
        assert_violation(
            &good().replace("min: 0.1", "min: 0.9"),
            "proxies[0].latency.min",
            "must be <= max",
        );
        assert_violation(
            &good().replace("min: 0.1", "min: -1.0"),
            "proxies[0].latency.min",
            "must be >= 0",
        );
    }

    #[test]
    fn v15_upstream_headers_must_be_well_formed() {
        assert_violation(
            &good().replace("      Authorization:", "      \"Bad Header\":"),
            "proxies[0].headers",
            "not a valid header name",
        );
        assert_violation(
            &good().replace(r#""Bearer x""#, r#""bad\nvalue""#),
            "proxies[0].headers.Authorization",
            "not a valid header value",
        );
    }
}
