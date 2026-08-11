//! Semantic validation. Rules carry stable `V<n>` identifiers; V1..V33 come
//! from the phase 1 spec and V34 was added in phase 3. A retired number is
//! never reused, so a message quoted in an old issue keeps meaning what it
//! meant: V3 went with `server.workers`, and V35 was subsumed by
//! `config::Name`.
//!
//! Validation is pure: it inspects the config and nothing else. Checks that
//! depend on the machine (does the templates directory exist, is the socket
//! parent writable) are startup preflight, not validation, so that
//! `doppel config validate` gives the same answer everywhere. Remarks that do
//! not refuse a configuration live in `advisory` for the same reason.

mod access;
mod advisory;
mod mock;
mod proxy;
mod server;

pub use advisory::startup_advisories;

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
      token: t1-00000000000000000000000000000000
  access:
    read: public
    update: user1
  upload:
    limit: 1Mi
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
    fn v1_the_two_listeners_must_not_share_a_port() {
        assert_violation(
            &good().replace("port: 8081", "port: 8080"),
            "admin.port",
            "must differ",
        );
    }

    #[test]
    fn a_zero_port_fails_at_parse_time() {
        // This half of V1 is now `config::Port`. Kept as a test at this level
        // because the claim worth pinning is about the document -- a
        // configuration naming port 0 must not load -- and because the
        // message has to explain what 0 would do rather than restate a range.
        for field in ["port: 8080", "port: 8081"] {
            let err = load_from_str(&good().replace(field, "port: 0")).unwrap_err();
            assert!(err.to_string().contains("any free port"), "{field}: {err}");
        }
    }

    #[test]
    fn v2_bad_host_fails_at_parse_time() {
        let err = load_from_str(&good().replace(r#""127.0.0.1""#, r#""not-an-ip""#)).unwrap_err();
        assert!(err.to_string().contains("invalid"), "got: {err}");
    }

    #[test]
    fn v4_unknown_log_level_fails_at_parse_time() {
        let text = format!("{}\nlogging:\n  level: chatty\n", good());
        assert!(load_from_str(&text).is_err());
    }

    #[test]
    fn v26_token_names_and_values_must_be_unique() {
        let dup_name = good().replace(
            "      token: t1-00000000000000000000000000000000\n",
            "      token: t1-00000000000000000000000000000000\n    - name: user1\n      group: user\n      token: t2-00000000000000000000000000000000\n",
        );
        assert_violation(&dup_name, "admin.tokens[1].name", "duplicate");

        let dup_token = good().replace(
            "      token: t1-00000000000000000000000000000000\n",
            "      token: t1-00000000000000000000000000000000\n    - name: user2\n      group: user\n      token: t1-00000000000000000000000000000000\n",
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

    /// The default is `["*"]`, and an allow-list nobody asked for would only
    /// surprise -- so a configuration that never mentions `groups` is unaffected.
    #[test]
    fn v36_an_absent_groups_list_allows_anything() {
        let text = good().replace("read: public", r#"read: ["admin", "user"]"#);
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    #[test]
    fn v36_a_concrete_list_refuses_a_name_it_does_not_carry() {
        let text = good()
            .replace("  access:", "  groups: [\"admin\", \"user1\"]\n  access:")
            .replace("read: public", r#"read: ["admin", "user"]"#);
        assert_violation(&text, "admin.access.read", "`user` is not an allowed group");
    }

    /// The message has to name what *is* permitted: the reader has to choose
    /// between changing the reference and widening the list, and cannot do
    /// either without seeing the list.
    #[test]
    fn v36_the_message_names_the_permitted_entries() {
        let text = good()
            .replace("  access:", "  groups: [\"admin\", \"ci\"]\n  access:")
            .replace("read: public", "read: user");
        assert_violation(
            &text,
            "admin.access.read",
            "may name only `admin`, `ci`, `public`",
        );
    }

    /// `groups: []` names nobody, so nothing can be granted to anyone and the
    /// only reading that describes a runnable configuration is all-public.
    ///
    /// It used to be unsatisfiable: V36 refused the `admin` every action
    /// defaults to, V34 refused `public` for the four writes, and no value
    /// existed that both would accept. The regression is worth naming because
    /// nothing failed -- the configuration was simply impossible to write.
    #[test]
    fn v36_an_empty_list_means_public_rather_than_an_impossible_document() {
        let text = good().replace("  access:", "  groups: []\n  access:");
        let config = load_from_str(&text).unwrap();
        assert_eq!(validate(&config), Ok(()));
        assert!(config.admin.is_public());
    }

    /// The same for the flag that says it in as many words. V34 does not apply:
    /// its job is to stop an unauthenticated writable proxy set happening by
    /// omission, and this is the opposite of an omission.
    #[test]
    fn v34_does_not_refuse_writes_when_the_admin_api_is_declared_public() {
        let text = good()
            .replace("  access:", "  public: true\n  access:")
            .replace("update: user1", "update: public\n    create: public");
        let config = load_from_str(&text).unwrap();
        assert_eq!(validate(&config), Ok(()));
        assert!(config.admin.is_public());
    }

    /// And `public` still refuses a write when nothing declared the API public,
    /// which is the case V34 was written for.
    #[test]
    fn v34_still_refuses_a_public_write_by_omission() {
        assert_violation(
            &good().replace("update: user1", "update: public"),
            "admin.access.update",
            "must not be public",
        );
    }

    /// A list that omits `admin` would otherwise recreate the impossible
    /// document: every action defaults to `admin`, so refusing it leaves the
    /// writes with no legal value again.
    #[test]
    fn v36_never_refuses_admin_even_when_the_list_omits_it() {
        let text = good()
            .replace("  access:", "  groups: [\"ci\"]\n  access:")
            .replace("update: user1", "update: admin");
        assert_eq!(validate(&load_from_str(&text).unwrap()), Ok(()));
    }

    /// A proxy's overrides are checked too. V27 and V36 walk one shared list of
    /// access sites precisely so a rule cannot cover the admin block and forget
    /// these.
    #[test]
    fn v36_governs_a_proxys_overrides_as_well() {
        let text = good()
            .replace("  access:", "  groups: [\"admin\"]\n  access:")
            .replace(
                "    url: \"https://example.com/\"",
                "    url: \"https://example.com/\"\n    access:\n      read: user",
            );
        assert_violation(
            &text,
            "proxies[0].access.read",
            "`user` is not an allowed group",
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
    fn an_omitted_access_block_grants_nothing_to_anyone_anonymous() {
        // The most common configuration is the one nobody wrote, so the
        // default has to be the safe one -- for reads as much as for writes.
        // A proxy document carries the headers this proxy injects upstream,
        // an `Authorization` among them in this project's own reference
        // configuration, so a public listing publishes credentials.
        let text = good().replace(
            "  access:\n    read: public\n    update: user1\n",
            "  access: {}\n",
        );
        let config = load_from_str(&text).expect("an empty access block must parse");

        let admin = Subjects::Names(vec![crate::config::Name::parse("admin").unwrap()]);
        for (action, subjects) in [
            ("list", &config.admin.access.list),
            ("read", &config.admin.access.read),
            ("create", &config.admin.access.create),
            ("update", &config.admin.access.update),
            ("delete", &config.admin.access.delete),
            ("upload", &config.admin.access.upload),
        ] {
            assert_eq!(*subjects, admin, "`{action}` must not default to public");
        }
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn v34_still_refuses_a_public_write_when_the_listener_is_off() {
        // The rules do not depend on whether the listener runs. Someone who
        // turns it on later will not re-read them, and a configuration that
        // was only safe because nothing served it is a trap waiting for that
        // moment.
        let text = good()
            .replace(
                "  access:\n    read: public\n",
                "  enable: false\n  access:\n",
            )
            .replace("    update: user1\n", "    update: public\n");
        assert_violation(&text, "admin.access.update", "public");
    }

    #[test]
    fn an_explicit_public_read_is_still_honoured() {
        // The default is safe; the choice stays the operator's. A
        // configuration with no secrets in it may legitimately expose reads.
        let text = good().replace(
            "  access:\n    read: public\n    update: user1\n",
            "  access:\n    read: public\n    list: public\n",
        );
        let config = load_from_str(&text).expect("parses");
        assert_eq!(config.admin.access.read, Subjects::Public);
        assert_eq!(validate(&config), Ok(()));
    }

    #[test]
    fn all_violations_are_reported_at_once() {
        // Two rule violations, not a parse failure and a rule violation:
        // parsing stops at the first error, so a document that fails to parse
        // could never demonstrate that the rule set collects.
        let text = good().replace("port: 8081", "port: 8080");
        let text = text.replace("    update: user1", "    update: nobody");
        let found = violations(&text);
        assert!(
            found.len() >= 2,
            "expected several violations, got {found:?}"
        );
    }

    /// Every rule still in the rule set.
    ///
    /// Written out rather than counted, because it is the thing the tests
    /// below compare the source and the documentation against.
    const LIVE: [u8; 15] = [1, 6, 10, 11, 14, 16, 19, 20, 21, 25, 26, 27, 30, 34, 36];

    /// Every rule that has been retired, and is therefore never reused.
    const RETIRED: [u8; 21] = [
        2, 3, 4, 5, 7, 8, 9, 12, 13, 15, 17, 18, 22, 23, 24, 28, 29, 31, 32, 33, 35,
    ];

    #[test]
    fn every_number_up_to_the_highest_is_accounted_for() {
        // The two lists are the map. If a rule is retired and dropped from
        // `LIVE` without being added to `RETIRED`, its number silently
        // becomes available for reuse -- and a message quoted in an old
        // issue would then mean two different things.
        //
        // The range runs to whichever number is highest rather than to a
        // literal: this asserted `1..=35` and had to be edited to add V36,
        // which is one more thing to remember at exactly the moment a rule
        // number is being chosen. A gap still fails, which is the point.
        let mut all: Vec<u8> = LIVE.iter().chain(RETIRED.iter()).copied().collect();
        all.sort_unstable();
        let highest = *all.last().expect("there is at least one rule");
        assert_eq!(all, (1..=highest).collect::<Vec<u8>>());
    }

    #[test]
    fn each_live_rule_is_marked_in_the_source_that_implements_it() {
        // A rule that leaves the code without leaving `LIVE` would otherwise
        // sit in the documentation as something this program still checks.
        let source = concat!(
            include_str!("server.rs"),
            include_str!("access.rs"),
            include_str!("proxy.rs"),
            include_str!("mock.rs"),
        );
        for rule in LIVE {
            let marker = format!("// V{rule}");
            assert!(
                source.contains(&marker),
                "V{rule} is listed as live but no `{marker}` marks where it runs"
            );
        }
    }

    #[test]
    fn the_documented_retired_rules_are_exactly_the_retired_rules() {
        // The table exists so a reader who hits an old message can find out
        // where the check went. Compared as a set in both directions: a
        // retirement missing from the table leaves a number nothing
        // explains, and a number in the table that is not retired says a
        // check moved when it did not.
        //
        // The first cut of this searched the whole document for `V<n>` and
        // passed while claiming V7 was documented -- `V7` occurs in the
        // sentence listing the rules that remain, and `V3` occurs inside
        // `V35`. Hence parsing the table's first column rather than
        // substring-matching prose.
        let docs = include_str!("../../../../docs/usage/configuration.md");
        let after = docs
            .split_once("### Retired rules")
            .expect("the retired-rules section must exist")
            .1;
        let table = after.split_once("\n## ").map_or(after, |(head, _)| head);

        let mut documented: Vec<u8> = table
            .lines()
            // A row marked `(part)` is a rule that lost one of its checks to
            // a type and kept the rest. It is still live, so it belongs in
            // `LIVE`, and the row is there to say where the other half went.
            // V14 is the only one: the sign of a latency became `Seconds`,
            // while `min <= max` needs both fields and stayed a rule.
            .filter(|line| !line.contains("(part)"))
            .filter_map(|line| line.strip_prefix("| V"))
            .flat_map(|row| {
                let cell = row.split('|').next().unwrap_or_default();
                cell.split(", V")
                    .filter_map(|number| {
                        number
                            .split_whitespace()
                            .next()?
                            .trim_end_matches(&[',', ' '][..])
                            .parse::<u8>()
                            .ok()
                    })
                    .collect::<Vec<u8>>()
            })
            .collect();
        documented.sort_unstable();
        documented.dedup();

        let mut retired = RETIRED.to_vec();
        retired.sort_unstable();
        assert_eq!(
            documented, retired,
            "the retired-rules table and `RETIRED` disagree"
        );
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
