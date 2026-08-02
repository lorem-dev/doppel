//! Semantic validation. Rules are identified V1..V35; V1..V33 come from the
//! phase 1 spec, V34 and V35 were added in phase 3.
//!
//! Validation is pure: it inspects the config and nothing else. Checks that
//! depend on the machine (does the templates directory exist, is the socket
//! parent writable) are startup preflight, not validation, so that
//! `doppel config validate` gives the same answer everywhere.

mod access;
mod mock;
mod proxy;
mod server;

#[cfg(test)]
mod test_support;

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// One validation failure, located by its path in the config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub path: String,
    pub message: String,
}

impl Violation {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // A violation with no path is one about the document as a whole --
        // a parse failure, say -- rather than about a field. Printing the
        // usual `path: message` for it produces a leading `: `, which reads
        // like a missing value rather than an absent one.
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Accumulator. Every rule appends rather than returning early, so a config
/// with five mistakes takes one round trip to fix instead of five.
#[derive(Debug, Default)]
pub struct Violations(Vec<Violation>);

impl Violations {
    pub fn push(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.0.push(Violation::new(path, message));
    }

    /// Append a violation unless `ok` holds.
    pub fn require(&mut self, ok: bool, path: impl Into<String>, message: impl Into<String>) {
        if !ok {
            self.push(path, message);
        }
    }

    pub fn into_result(self) -> Result<(), Vec<Violation>> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self.0)
        }
    }
}

/// Validate a parsed config. Returns every violation found.
pub fn validate(config: &Config) -> Result<(), Vec<Violation>> {
    let mut v = Violations::default();
    server::check(config, &mut v);
    access::check(config, &mut v);
    proxy::check(config, &mut v);
    v.into_result()
}

/// RFC 7230 token test, used for header names in rules V11, V15 and V24.
#[must_use]
pub fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Visible ASCII plus space and horizontal tab, used for rule V15.
#[must_use]
pub fn is_valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Subjects, load_from_str};

    /// A config that passes every rule. Tests mutate one thing at a time.
    fn good() -> String {
        r#"
server:
  host: "127.0.0.1"
  port: 8080
admin:
  host: "127.0.0.1"
  port: 8081
  tokens:
    - name: user1
      group: admin
      token: t1
  access:
    read: public
    update: user1
  upload:
    limit: 1M
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#
        .to_owned()
    }

    use super::test_support::{assert_violation, violations};

    #[test]
    fn good_config_passes() {
        let config = load_from_str(&good()).unwrap();
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn v1_ports_must_be_nonzero_and_distinct() {
        assert_violation(
            &good().replace("port: 8080", "port: 0"),
            "server.port",
            "must not be 0",
        );
        assert_violation(
            &good().replace("port: 8081", "port: 8080"),
            "admin.port",
            "must differ",
        );
    }

    #[test]
    fn v2_bad_host_fails_at_parse_time() {
        let err = load_from_str(&good().replace(r#""127.0.0.1""#, r#""not-an-ip""#)).unwrap_err();
        assert!(err.to_string().contains("invalid"), "got: {err}");
    }

    #[test]
    fn v3_workers_must_be_at_least_one() {
        assert_violation(
            &good().replace("port: 8080", "port: 8080\n  workers: 0"),
            "server.workers",
            "at least 1",
        );
    }

    #[test]
    fn v4_unknown_log_level_fails_at_parse_time() {
        let text = format!("{}\nlogging:\n  level: chatty\n", good());
        assert!(load_from_str(&text).is_err());
    }

    #[test]
    fn v26_token_names_and_values_must_be_unique() {
        let dup_name = good().replace(
            "      token: t1\n",
            "      token: t1\n    - name: user1\n      group: user\n      token: t2\n",
        );
        assert_violation(&dup_name, "admin.tokens[1].name", "duplicate");

        let dup_token = good().replace(
            "      token: t1\n",
            "      token: t1\n    - name: user2\n      group: user\n      token: t1\n",
        );
        assert_violation(&dup_token, "admin.tokens[1].token", "duplicate");
    }

    #[test]
    fn v27_access_must_reference_known_subjects() {
        assert_violation(
            &good().replace("read: public", "read: ghost"),
            "admin.access.read",
            "unknown token or group `ghost`",
        );
    }

    #[test]
    fn v27_predefined_groups_are_always_valid() {
        let text = good().replace("read: public", r#"read: ["admin", "user"]"#);
        let config = load_from_str(&text).unwrap();
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn v27_custom_group_must_be_carried_by_a_token() {
        let text = good().replace("read: public", "read: reviewers");
        assert_violation(
            &text,
            "admin.access.read",
            "unknown token or group `reviewers`",
        );

        let with_group = good()
            .replace("group: admin", "group: reviewers")
            .replace("read: public", "read: reviewers");
        let config = load_from_str(&with_group).unwrap();
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn v27_proxy_access_override_must_reference_known_subjects() {
        let text = good().replace(
            r#"    url: "https://example.com/""#,
            "    url: \"https://example.com/\"\n    access:\n      read: ghost",
        );
        assert_violation(
            &text,
            "proxies[0].access.read",
            "unknown token or group `ghost`",
        );
    }

    #[test]
    fn v27_proxy_access_override_accepts_known_subjects() {
        let text = good().replace(
            r#"    url: "https://example.com/""#,
            "    url: \"https://example.com/\"\n    access:\n      read: admin",
        );
        let config = load_from_str(&text).unwrap();
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn v28_overriding_create_on_a_proxy_fails_at_parse_time() {
        let text = good().replace(
            r#"    url: "https://example.com/""#,
            "    url: \"https://example.com/\"\n    access:\n      create: admin",
        );
        assert!(load_from_str(&text).is_err());
    }

    #[test]
    fn v34_a_write_action_may_not_be_public() {
        for action in ["create", "update", "delete", "upload"] {
            // Replace the whole block rather than appending a line: the
            // fixture already sets `update`, and appending would produce a
            // duplicate YAML key that fails to parse before the rule is
            // reached -- a test that fails for the wrong reason.
            let text = good().replace(
                "  access:\n    read: public\n    update: user1",
                &format!("  access:\n    read: public\n    {action}: public"),
            );
            assert_violation(
                &text,
                &format!("admin.access.{action}"),
                "must not be public",
            );
        }
    }

    #[test]
    fn v34_reads_may_still_be_public() {
        let text = good().replace(
            "  access:\n    read: public",
            "  access:\n    read: public\n    list: public",
        );
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    #[test]
    fn an_omitted_access_block_defaults_writes_to_admin_not_public() {
        // The most common configuration is the one nobody wrote, so the
        // default has to be the safe one. Rule V34 refuses an explicit public
        // write; this pins that the implicit case is safe too.
        let text = good().replace(
            "  access:\n    read: public\n    update: user1\n",
            "  access: {}\n",
        );
        let config = load_from_str(&text).expect("an empty access block must parse");
        assert_eq!(config.admin.access.read, Subjects::Public);
        assert_eq!(
            config.admin.access.create,
            Subjects::Names(vec!["admin".to_owned()])
        );
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn v29_upload_limit_must_be_positive() {
        assert_violation(
            &good().replace("limit: 1M", "limit: 0"),
            "admin.upload.limit",
            "greater than 0",
        );
    }

    #[test]
    fn all_violations_are_reported_at_once() {
        let text = good().replace("port: 8080", "port: 0");
        let text = text.replace("limit: 1M", "limit: 0");
        let found = violations(&text);
        assert!(
            found.len() >= 2,
            "expected several violations, got {found:?}"
        );
    }

    #[test]
    fn header_name_and_value_helpers() {
        assert!(is_valid_header_name("X-Request-ID"));
        assert!(!is_valid_header_name("X Request ID"));
        assert!(!is_valid_header_name(""));
        assert!(is_valid_header_value("Bearer abc"));
        assert!(!is_valid_header_value("bad\nvalue"));
    }

    #[test]
    fn reference_config_is_valid() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../main.example.yaml"
        ))
        .unwrap();
        let config = load_from_str(&text).unwrap();
        assert_eq!(validate(&config), Ok(()), "main.example.yaml must be valid");
    }
}
