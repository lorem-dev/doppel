//! The template variable names Doppel binds itself.
//!
//! The list lives here rather than beside the code that binds it, because two
//! places need it and they sit on opposite sides of the dependency direction:
//! `doppel-render` binds these into a context, and validation here has to be able
//! to say when a mock declares one of them. `doppel-core` depends on neither, so
//! the shared fact -- the names -- lives in the crate both can see.

/// Every name Doppel binds itself, sorted.
///
/// Reserved: an extraction of the same name is overwritten by the system value,
/// so a template always means what the documentation says it means. Sorted
/// because `startup_advisories` reports them in this order and a test asserts the
/// list against what is actually bound.
pub const RESERVED: &[&str] = &[
    "doppel_version",
    "host",
    "method",
    "mock_name",
    "path",
    "peer_ip",
    "proxy_name",
    "real_ip",
    "request_id",
];

/// Whether a name is one of Doppel's own.
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_has_no_duplicates() {
        // `startup_advisories` reports in this order, and a sorted list is also
        // how a reader checks whether a name is on it.
        let mut sorted = RESERVED.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, RESERVED);
    }

    #[test]
    fn a_name_that_is_not_ours_is_not_reserved() {
        assert!(is_reserved("proxy_name"));
        assert!(!is_reserved("resource_id"));
        // Case matters: a name is reserved as written, and `Proxy_Name` is a
        // different variable in Jinja.
        assert!(!is_reserved("Proxy_Name"));
    }
}
