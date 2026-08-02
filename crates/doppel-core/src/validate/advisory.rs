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

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_from_str;

    fn config(server: u16, admin: u16) -> Config {
        load_from_str(&format!(
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
        ))
        .unwrap()
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
    fn a_privileged_port_is_still_a_legal_configuration() {
        // The point of an advisory rather than a rule: running on port 80
        // behind a capability is a real deployment, and refusing it would
        // break it for looking unusual.
        assert_eq!(crate::validate::validate(&config(80, 443)), Ok(()));
    }
}
