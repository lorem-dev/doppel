//! Things worth saying about a configuration that are not reasons to refuse it.
//!
//! `validate` answers one question -- may this configuration run? -- and
//! answering it with anything other than yes or no would make every caller
//! decide what a partial answer means. An advisory is the other kind of
//! remark: the configuration is legal, it will work, and it still probably is
//! not what the operator meant.
//!
//! These are emitted once at startup rather than folded into `validate`,
//! which stays a pure function of the document. A warning printed on every
//! `doppel config validate` in a CI loop is a warning people stop reading.

use crate::config::Config;

/// Non-fatal remarks about `config`, in the order they should be logged.
///
/// Empty for a configuration with nothing surprising in it, which is the
/// common case and the one worth staying quiet about.
#[must_use]
pub fn startup_advisories(config: &Config) -> Vec<String> {
    let mut out = Vec::new();

    for (field, port) in [
        ("server.port", config.server.port),
        ("admin.port", config.admin.port),
    ] {
        if port.is_privileged() {
            out.push(format!(
                "{field} is {port}, which needs elevated privilege to bind on \
                 most systems; if that was not deliberate, it is usually a \
                 typo, and if it was, the process needs the capability or a \
                 redirect in front of it"
            ));
        }
    }

    if config.admin.is_public() {
        // Said first and unconditionally: an unauthenticated admin API is the
        // single most consequential thing a configuration can turn on, and it is
        // reachable two ways -- `public: true`, or the `groups: []` that means
        // the same. Someone who wrote only the latter may not realise which they
        // chose.
        out.push(
            if config.admin.public.unwrap_or(false) {
                "admin.public is true: the whole admin API is served \
                 unauthenticated, including the actions that rewrite the proxy \
                 set"
            } else {
                "admin.groups is an empty list, which names nobody and therefore \
                 means the same as `admin.public: true`: the whole admin API is \
                 served unauthenticated. Set `public: true` if that was the \
                 intent, or name the groups you meant to allow"
            }
            .to_owned(),
        );

        // Anything the document still says about who may do what is dead, and
        // saying so is the difference between an operator seeing an override and
        // an operator believing a token still guards something.
        //
        // Every test here is against what the *document* holds, not against the
        // resolved value. `allowed_groups()` returns the `["*"]` default for an
        // absent `groups`, so asking it would have reported `admin.groups` as
        // overridden under `public: true` in a configuration that never
        // mentioned it -- naming a field the operator did not write as one of
        // their settings being ignored.
        let mut overridden: Vec<&str> = Vec::new();
        if config.admin.access != crate::config::AccessConfig::default() {
            overridden.push("admin.access");
        }
        if config
            .admin
            .groups
            .as_deref()
            .is_some_and(|groups| !groups.is_empty())
        {
            overridden.push("admin.groups");
        }
        if config.proxies.iter().any(|proxy| proxy.access.is_some()) {
            overridden.push("a proxy's access overrides");
        }
        if !overridden.is_empty() {
            out.push(format!(
                "{} {} ignored while the admin API is public; every action \
                 answers as `public` regardless",
                overridden.join(", "),
                if overridden.len() == 1 { "is" } else { "are" }
            ));
        }
    }

    for proxy in &config.proxies {
        if proxy.url.has_credentials() {
            out.push(format!(
                "proxy `{}` has a username or password in its upstream url; that \
                 credential is part of the proxy document the admin API \
                 returns, so anyone who may read a proxy holds it",
                proxy.name
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_from_str;

    fn config(server: u16, admin: u16) -> Config {
        load_from_str(&raw(server, admin)).unwrap()
    }

    fn raw(server: u16, admin: u16) -> String {
        format!(
            r#"
server:
  host: "127.0.0.1"
  port: {server}
admin:
  host: "127.0.0.1"
  port: {admin}
  tokens: []
  access: {{}}
  upload:
    limit: 1Mi
proxies:
  - name: p1
    type: http
    url: "https://example.com/"
"#
        )
    }

    #[test]
    fn an_ordinary_configuration_says_nothing() {
        assert!(startup_advisories(&config(8080, 8081)).is_empty());
    }

    #[test]
    fn a_privileged_port_is_named_along_with_the_field_that_holds_it() {
        let notes = startup_advisories(&config(80, 8081));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("server.port"), "{}", notes[0]);
        assert!(notes[0].contains("80"), "{}", notes[0]);

        // Both listeners are checked, not just the first one found.
        let both = startup_advisories(&config(80, 443));
        assert_eq!(both.len(), 2, "{both:?}");
        assert!(both[1].contains("admin.port"), "{}", both[1]);
    }

    #[test]
    fn a_credential_in_an_upstream_url_is_named_with_its_proxy() {
        let text = format!("{}", config(8080, 8081).proxies[0].url);
        assert_eq!(text, "https://example.com/", "fixture changed");

        let with_credentials = load_from_str(
            &raw(8080, 8081).replace("https://example.com/", "https://user:secret@example.com/"),
        )
        .unwrap();
        let notes = startup_advisories(&with_credentials);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("`p1`"), "{}", notes[0]);
        // The advisory must not carry the secret it is warning about.
        assert!(!notes[0].contains("secret"), "{}", notes[0]);
    }

    /// An unauthenticated admin API is the most consequential thing a
    /// configuration can turn on, so it is said out loud even when nothing is
    /// being overridden.
    #[test]
    fn declaring_the_admin_api_public_is_reported() {
        let text = raw(8080, 8081).replacen("  tokens:", "  public: true\n  tokens:", 1);
        let notes = startup_advisories(&load_from_str(&text).unwrap());
        assert!(
            notes
                .iter()
                .any(|note| note.contains("admin.public is true")),
            "{notes:?}"
        );
        // This fixture writes no `access` and no `groups`, so nothing is being
        // overridden and nothing may claim to be. Asking `allowed_groups()`
        // rather than the field reported `admin.groups` here, naming a setting
        // the operator never wrote.
        assert!(
            !notes.iter().any(|note| note.contains("ignored while")),
            "nothing was overridden: {notes:?}"
        );
    }

    /// The spelling an operator reaches by trying to *restrict* things says the
    /// same thing, so the advisory names which of the two they wrote and how to
    /// say the other.
    #[test]
    fn an_empty_groups_list_is_reported_as_meaning_public() {
        let text = raw(8080, 8081).replacen("  tokens:", "  groups: []\n  tokens:", 1);
        let notes = startup_advisories(&load_from_str(&text).unwrap());
        let note = notes
            .iter()
            .find(|note| note.contains("admin.groups is an empty list"))
            .unwrap_or_else(|| panic!("{notes:?}"));
        assert!(note.contains("public: true"), "{note}");
    }

    /// What the document still says about who may do what is dead under a public
    /// API, and the difference between saying so and not is an operator who
    /// believes a token still guards something.
    #[test]
    fn access_and_groups_overridden_by_a_public_api_are_named() {
        let text = raw(8080, 8081)
            .replacen(
                "  tokens:",
                "  public: true\n  groups: [\"ci\"]\n  tokens:",
                1,
            )
            .replace("  access: {}", "  access:\n    read: public");
        let notes = startup_advisories(&load_from_str(&text).unwrap());
        let note = notes
            .iter()
            .find(|note| note.contains("ignored while the admin API is public"))
            .unwrap_or_else(|| panic!("{notes:?}"));
        assert!(note.contains("admin.access"), "{note}");
        assert!(note.contains("admin.groups"), "{note}");
    }

    /// And a configuration that is not public says none of it.
    #[test]
    fn a_private_admin_api_produces_no_public_advisory() {
        let notes = startup_advisories(&config(8080, 8081));
        assert!(
            !notes.iter().any(|note| note.contains("public")),
            "{notes:?}"
        );
    }

    #[test]
    fn a_privileged_port_is_still_a_legal_configuration() {
        // The point of an advisory rather than a rule: running on port 80
        // behind a capability is a real deployment, and refusing it would
        // break it for looking unusual.
        assert_eq!(crate::validate::validate(&config(80, 443)), Ok(()));
    }
}
