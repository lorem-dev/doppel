//! The variables Doppel puts in every template context itself.
//!
//! Everything else a template can see is the operator's own: a named capture in
//! a mock's path pattern, a `headers`, `query` or `body` entry. Those describe
//! one mock. These describe the request and the process, are the same in every
//! template, and are worth having without declaring them nine times.
//!
//! **They are reserved.** They are bound *after* the operator's extractions, so
//! a mock that declares `proxy_name` finds the system value in its template
//! rather than its own -- the alternative is a template whose meaning depends on
//! which mock rendered it. `startup_advisories` names any mock that shadows one,
//! because the extraction still happens and still costs a header read while its
//! result goes unused.
//!
//! Named in `snake_case`, which is also the convention this project's own
//! configuration follows for extracted names. Jinja has no trouble with either,
//! but two conventions in one context read as two sources.

use std::net::IpAddr;

use crate::extract::Variables;

/// The version this binary reports, from the workspace's one version number.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every name Doppel binds itself, from `doppel-core` -- validation needs the
/// same list and sits on the other side of the dependency direction.
pub use doppel_core::template::RESERVED;

/// What Doppel knows about one request, independent of any mock.
///
/// Built once per request and used twice: to render a mock's response, and to
/// render `server.external_url` when that is a template. The second is why this
/// lives here rather than beside the mock code -- a redirect is rewritten for a
/// forwarded request too, where there is no mock at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemVars {
    /// The proxy that resolved, or empty when none did.
    pub proxy_name: String,
    /// The mock answering, or empty when the request is being forwarded.
    pub mock_name: String,
    /// The id echoed in `X-Request-ID`, minted when the client sent none.
    pub request_id: String,
    pub method: String,
    /// The request path, without the query string.
    pub path: String,
    /// The `Host` the client asked for, or empty when it sent none.
    ///
    /// A claim by the caller, not a fact about this process. It is here because
    /// a template that reports what it was asked for needs it, and it is called
    /// `host` rather than something reassuring so nobody forgets which it is.
    pub host: String,
    /// The address the connection came from: the socket's own, and the only one
    /// of the two nobody can fake.
    pub peer_ip: String,
    /// Who the request is *said* to be from: `X-Real-IP`, else the first entry
    /// of `X-Forwarded-For`, else `peer_ip`.
    pub real_ip: String,
}

impl SystemVars {
    /// `real_ip`, by the order a proxy chain writes it.
    ///
    /// `X-Real-IP` first because a single proxy in front sets exactly that and
    /// nothing else; then the leftmost `X-Forwarded-For`, which is the original
    /// client in a chain that appends; then the peer, so the variable always has
    /// a value and a template never has to write `| default(...)`.
    ///
    /// Every field line of `X-Forwarded-For` is considered, not only the first:
    /// a chain split across several lines is legal and some proxies emit it, and
    /// taking `get()` alone would read the second hop as the client.
    #[must_use]
    pub fn resolve_real_ip(
        real_ip_header: Option<&str>,
        forwarded_for: &[&str],
        peer: Option<IpAddr>,
    ) -> String {
        if let Some(value) = real_ip_header.map(str::trim).filter(|v| !v.is_empty()) {
            return value.to_owned();
        }
        for line in forwarded_for {
            if let Some(first) = line
                .split(',')
                .map(str::trim)
                .find(|entry| !entry.is_empty())
            {
                return first.to_owned();
            }
        }
        peer.map(|address| address.to_string()).unwrap_or_default()
    }

    /// Bind these into `vars`, overwriting anything of the same name.
    ///
    /// Last, deliberately: see the module comment.
    pub fn bind(&self, vars: &mut Variables) {
        let pairs: [(&str, &str); 9] = [
            ("proxy_name", &self.proxy_name),
            ("mock_name", &self.mock_name),
            ("request_id", &self.request_id),
            ("method", &self.method),
            ("path", &self.path),
            ("host", &self.host),
            ("peer_ip", &self.peer_ip),
            ("real_ip", &self.real_ip),
            ("doppel_version", VERSION),
        ];
        for (name, value) in pairs {
            vars.insert(name, serde_json::Value::String(value.to_owned()));
        }
    }

    /// These alone, for rendering something that has no mock behind it.
    #[must_use]
    pub fn as_variables(&self) -> Variables {
        let mut vars = Variables::new();
        self.bind(&mut vars);
        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reserved_name_is_bound_and_every_bound_name_is_reserved() {
        // The two lists are written out separately -- one for validation to read,
        // one for the binding -- so this is what keeps them the same list.
        let bound = SystemVars::default().as_variables();
        let mut names: Vec<String> = bound.names().map(ToOwned::to_owned).collect();
        names.sort();
        assert_eq!(
            names, RESERVED,
            "the reserved list and the binding disagree"
        );
    }

    #[test]
    fn the_version_is_the_one_this_binary_reports() {
        let vars = SystemVars::default().as_variables();
        assert_eq!(
            vars.get("doppel_version").and_then(|v| v.as_str()),
            Some(VERSION)
        );
        assert!(VERSION.starts_with(char::is_numeric), "{VERSION}");
    }

    #[test]
    fn real_ip_prefers_the_header_a_single_proxy_sets() {
        let peer: IpAddr = "10.0.0.9".parse().unwrap();
        assert_eq!(
            SystemVars::resolve_real_ip(Some("203.0.113.7"), &["198.51.100.1"], Some(peer)),
            "203.0.113.7"
        );
    }

    #[test]
    fn then_the_leftmost_forwarded_for_entry() {
        // The original client in a chain that appends, not the hop next to us.
        let peer: IpAddr = "10.0.0.9".parse().unwrap();
        assert_eq!(
            SystemVars::resolve_real_ip(None, &["198.51.100.1, 10.0.0.8"], Some(peer)),
            "198.51.100.1"
        );
        // A chain split across field lines, which is legal and does happen.
        assert_eq!(
            SystemVars::resolve_real_ip(None, &["198.51.100.1", "10.0.0.8"], Some(peer)),
            "198.51.100.1"
        );
    }

    #[test]
    fn then_the_peer_which_nobody_can_fake() {
        let peer: IpAddr = "10.0.0.9".parse().unwrap();
        assert_eq!(
            SystemVars::resolve_real_ip(None, &[], Some(peer)),
            "10.0.0.9"
        );
        // Empty rather than absent, so a template never needs `| default`.
        assert_eq!(SystemVars::resolve_real_ip(None, &[], None), "");
    }

    #[test]
    fn an_empty_or_blank_header_is_not_an_answer() {
        let peer: IpAddr = "10.0.0.9".parse().unwrap();
        assert_eq!(
            SystemVars::resolve_real_ip(Some("   "), &[], Some(peer)),
            "10.0.0.9"
        );
        assert_eq!(
            SystemVars::resolve_real_ip(Some(""), &[" , "], Some(peer)),
            "10.0.0.9"
        );
    }

    #[test]
    fn a_system_name_wins_over_an_extraction_of_the_same_name() {
        let mut vars = Variables::new();
        vars.insert(
            "proxy_name",
            serde_json::Value::String("from-a-header".into()),
        );
        SystemVars {
            proxy_name: "alpha".to_owned(),
            ..SystemVars::default()
        }
        .bind(&mut vars);

        assert_eq!(
            vars.get("proxy_name").and_then(|v| v.as_str()),
            Some("alpha"),
            "a mock must not be able to change what a system variable means"
        );
    }
}
