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
    //
    // Skipped for a public configuration. V34's job is to stop an
    // unauthenticated writable proxy set happening by *omission*; `public: true`
    // -- or the `groups: []` that means the same -- is the operator saying it in
    // as many words, and refusing it would leave the flag with no effect it could
    // ever have.
    let access = &config.admin.access;
    for (action, subjects) in [
        ("create", &access.create),
        ("update", &access.update),
        ("delete", &access.delete),
        ("upload", &access.upload),
    ]
    .into_iter()
    .filter(|_| !config.admin.is_public())
    {
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
    let allowed = config.admin.allowed_groups();
    // A public configuration references nothing: `access` is answered as
    // `public` whatever it says. Checking it anyway would refuse the very names
    // `public: true` exists to override -- and those are reported as a startup
    // advisory instead, which is a remark rather than a refusal.
    let public = config.admin.is_public();
    for (path, subjects) in access_sites(config) {
        // V27
        check_subjects(subjects, &known, &path, v);
        // V36
        if !public {
            check_allowed_groups(subjects, allowed, &path, v);
        }
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

/// The group `admin.groups` cannot exclude.
///
/// Every action defaults to `admin`, and V34 refuses `public` for the four write
/// actions -- so a list omitting `admin` leaves `create`, `update`, `delete` and
/// `upload` with no legal value at all. That is not a lockdown, it is a document
/// nobody can write:
///
/// ```text
/// admin.access.create: `admin` is not an allowed group; ...
/// admin.access.create: `create` must not be public: ...Name a token or a group.
/// ```
///
/// So `admin` is exempt, exactly as `public` is. An allow-list bounds the names
/// an operator may *hand access to*; it cannot revoke the fallback every action
/// already has, because forbidding that produces no reachable state.
///
/// This is about a *non-empty* list that happens to omit `admin` --
/// `groups: []` no longer reaches here at all, since naming nobody is read as
/// `public: true` and V36 is skipped for a public configuration.
///
/// `user`, the other predefined group, is *not* exempt: nothing defaults to it,
/// so refusing it forecloses nothing.
const ALWAYS_ALLOWED_GROUP: &str = "admin";

/// V36: a name `access` references has to be one `admin.groups` allows.
///
/// `Subjects::Public` is never checked: `public` is the absence of a subject
/// rather than a name, so an allow-list has nothing to say about it. `admin` is
/// exempt too; see [`ALWAYS_ALLOWED_GROUP`].
///
/// Not called at all for a public configuration -- see the caller.
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
        if name == ALWAYS_ALLOWED_GROUP || allowed.iter().any(|entry| entry.permits(name.as_str()))
        {
            continue;
        }
        // The message names everything permitted rather than only the
        // rejection: the reader has to decide whether to change the reference or
        // widen the list, and cannot do either without seeing what is currently
        // allowed.
        //
        // Assembled as a set so `admin` appears once whether or not the list
        // names it. Spelling the exemptions as a suffix printed
        // "`admin`, `ci`, `admin` and `public`" for `groups: ["admin", "ci"]`,
        // which reads like a bug in the very message meant to clarify things.
        let mut permitted: Vec<String> = allowed.iter().map(|entry| entry.to_string()).collect();
        for exempt in [ALWAYS_ALLOWED_GROUP, "public"] {
            if !permitted.iter().any(|name| name == exempt) {
                permitted.push(exempt.to_owned());
            }
        }
        permitted.sort_unstable();
        let permitted = permitted
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");

        v.push(
            path,
            format!("`{name}` is not an allowed group; `admin.access` may name only {permitted}"),
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
