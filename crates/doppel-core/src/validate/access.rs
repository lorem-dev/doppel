//! Rules V26, V27, V34 and V36. V28 is enforced by `ProxyAccessConfig`, and V29
//! by `ByteSize`, which refuses a limit of zero for every field that uses
//! it rather than once per field.

use std::collections::BTreeSet;

use super::Violations;
use crate::config::{Config, Subjects};

/// Groups that always exist, whether or not a token carries them.
const PREDEFINED_GROUPS: [&str; 2] = ["admin", "user"];

pub(super) fn check(config: &Config, v: &mut Violations) {
    // V26
    let mut seen_names = BTreeSet::new();
    let mut seen_tokens = BTreeSet::new();
    for (i, token) in config.admin.tokens.iter().enumerate() {
        if !seen_names.insert(token.name.as_str()) {
            v.push(
                format!("admin.tokens[{i}].name"),
                format!("duplicate token name `{}`", token.name),
            );
        }
        if !seen_tokens.insert(token.token.as_str()) {
            v.push(format!("admin.tokens[{i}].token"), "duplicate token value");
        }
    }

    // V34: a write action granted to `public` lets any unauthenticated caller
    // rewrite the proxy set. That is far more often a mistake than an intent,
    // and startup is the last cheap moment to catch it. Reads stay allowed to
    // be public -- `/status` and a proxy listing give nothing away.
    let access = &config.admin.access;
    for (action, subjects) in [
        ("create", &access.create),
        ("update", &access.update),
        ("delete", &access.delete),
        ("upload", &access.upload),
    ] {
        if matches!(subjects, Subjects::Public) {
            v.push(
                format!("admin.access.{action}"),
                format!(
                    "`{action}` must not be public: an unauthenticated caller could \
                     rewrite the proxy set. Name a token or a group."
                ),
            );
        }
    }

    // V27 and V36 read the same places, so the walk happens once and both
    // rules run per site. They used to be one loop each, which is how a rule
    // added later ends up covering the admin block and quietly forgetting the
    // per-proxy overrides.
    let known = known_subjects(config);
    for (path, subjects) in access_sites(config) {
        // V27
        check_subjects(subjects, &known, &path, v);
        // V36
        check_allowed_groups(subjects, config.admin.allowed_groups(), &path, v);
    }
}

/// Every place a configuration names who may do something, with the path to
/// report it under: the admin block's six actions, then each proxy's four
/// overrides.
fn access_sites(config: &Config) -> Vec<(String, &Subjects)> {
    let access = &config.admin.access;
    let mut sites: Vec<(String, &Subjects)> = [
        ("list", &access.list),
        ("read", &access.read),
        ("create", &access.create),
        ("update", &access.update),
        ("delete", &access.delete),
        ("upload", &access.upload),
    ]
    .into_iter()
    .map(|(action, subjects)| (format!("admin.access.{action}"), subjects))
    .collect();

    for (i, proxy) in config.proxies.iter().enumerate() {
        let Some(overrides) = &proxy.access else {
            continue;
        };
        for (action, subjects) in [
            ("read", &overrides.read),
            ("update", &overrides.update),
            ("delete", &overrides.delete),
            ("upload", &overrides.upload),
        ] {
            if let Some(subjects) = subjects {
                sites.push((format!("proxies[{i}].access.{action}"), subjects));
            }
        }
    }
    sites
}

/// V36: a name `access` references has to be one `admin.groups` allows.
///
/// `Subjects::Public` is never checked. `public` is the absence of a subject
/// rather than a name, so an allow-list has nothing to say about it -- and a
/// configuration reduced to `groups: []` still has to be able to express "anyone
/// may read this".
fn check_allowed_groups(
    subjects: &Subjects,
    allowed: &[crate::config::AllowedGroup],
    path: &str,
    v: &mut Violations,
) {
    let Subjects::Names(names) = subjects else {
        return;
    };
    for name in names {
        if allowed.iter().any(|entry| entry.permits(name.as_str())) {
            continue;
        }
        // The message names the list rather than only the rejection: the reader
        // has to decide whether to change the reference or widen the list, and
        // cannot do either without seeing what is currently permitted.
        let permitted = if allowed.is_empty() {
            "`admin.groups` is empty, so only `public` may be used".to_owned()
        } else {
            format!(
                "`admin.groups` allows only {}",
                allowed
                    .iter()
                    .map(|entry| format!("`{entry}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        v.push(
            path,
            format!("`{name}` is not an allowed group: {permitted}"),
        );
    }
}

fn known_subjects(config: &Config) -> BTreeSet<String> {
    let mut known: BTreeSet<String> = PREDEFINED_GROUPS.iter().map(|g| (*g).to_owned()).collect();
    for token in &config.admin.tokens {
        known.insert(token.name.to_string());
        known.insert(token.group.to_string());
    }
    known
}

fn check_subjects(subjects: &Subjects, known: &BTreeSet<String>, path: &str, v: &mut Violations) {
    let Subjects::Names(names) = subjects else {
        return;
    };
    for name in names {
        v.require(
            known.contains(name.as_str()),
            path,
            format!("unknown token or group `{name}`"),
        );
    }
}
