//! The HTTP methods this project recognises by name.

/// Methods a mock may declare, and the only values that ever reach a metric
/// label.
///
/// Two callers share this list on purpose. Validation uses it to refuse a
/// mock whose `request.method` is a typo -- `FETCH` should be a config error,
/// not a mock that silently never matches. That is a typo guard, not a
/// protocol restriction: a genuinely non-standard method just needs adding
/// here. Metrics use it to bound the
/// `method` label: a method arrives from the wire, so an unrecognised one is
/// attacker-controlled, and a metric label taking arbitrary strings is the
/// same cardinality explosion as a path label. Anything not on this list is
/// recorded as `OTHER`.
pub const KNOWN_METHODS: &[&str] = &[
    "GET",
    "HEAD",
    "POST",
    "PUT",
    "PATCH",
    "DELETE",
    "OPTIONS",
    "TRACE",
    "CONNECT",
    // The safe, idempotent method for a request that carries a body -- the
    // "GET with a body" gap.
    "QUERY",
    // WebDAV methods (RFC 4918).
    "PROPFIND",
    "PROPPATCH",
    "MKCOL",
    "COPY",
    "MOVE",
    "LOCK",
    "UNLOCK",
];

/// What to record for `method` in a metric label.
///
/// Returns an entry of `KNOWN_METHODS` or the literal `OTHER`, never the
/// caller's string. The return type being `&'static str` is the guarantee:
/// no value derived from a request can escape into a label, because none can
/// satisfy it.
#[must_use]
pub fn method_label(method: &str) -> &'static str {
    KNOWN_METHODS
        .iter()
        .find(|known| **known == method)
        .copied()
        .unwrap_or("OTHER")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_method_labels_as_itself() {
        assert_eq!(method_label("GET"), "GET");
        assert_eq!(method_label("QUERY"), "QUERY");
        assert_eq!(method_label("UNLOCK"), "UNLOCK");
    }

    #[test]
    fn an_unknown_method_collapses_to_one_bounded_value() {
        // The point is the bound, so a thousand distinct inputs must give one
        // label. Anything else is a way to fill a metrics backend from the
        // outside.
        for i in 0..1000 {
            assert_eq!(method_label(&format!("BREW{i}")), "OTHER");
        }
        assert_eq!(method_label(""), "OTHER");
    }

    #[test]
    fn matching_is_case_sensitive() {
        // HTTP methods are case sensitive, and `get` is not `GET`. Folding
        // case here would silently merge a malformed request with a real one.
        assert_eq!(method_label("get"), "OTHER");
    }
}
