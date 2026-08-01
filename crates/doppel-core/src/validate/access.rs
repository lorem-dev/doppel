//! Rules V26, V27 and V29. V28 is enforced by `ProxyAccessConfig`.

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

    // V29
    v.require(
        config.admin.upload.limit.0 > 0,
        "admin.upload.limit",
        "upload limit must be greater than 0",
    );

    // V27
    let known = known_subjects(config);
    let access = &config.admin.access;
    for (action, subjects) in [
        ("list", &access.list),
        ("read", &access.read),
        ("create", &access.create),
        ("update", &access.update),
        ("delete", &access.delete),
        ("upload", &access.upload),
    ] {
        check_subjects(subjects, &known, &format!("admin.access.{action}"), v);
    }

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
                check_subjects(
                    subjects,
                    &known,
                    &format!("proxies[{i}].access.{action}"),
                    v,
                );
            }
        }
    }
}

fn known_subjects(config: &Config) -> BTreeSet<String> {
    let mut known: BTreeSet<String> = PREDEFINED_GROUPS.iter().map(|g| (*g).to_owned()).collect();
    for token in &config.admin.tokens {
        known.insert(token.name.clone());
        known.insert(token.group.clone());
    }
    known
}

fn check_subjects(subjects: &Subjects, known: &BTreeSet<String>, path: &str, v: &mut Violations) {
    let Subjects::Names(names) = subjects else {
        return;
    };
    for name in names {
        v.require(
            known.contains(name),
            path,
            format!("unknown token or group `{name}`"),
        );
    }
}
