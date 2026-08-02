//! Template file name checking, for names that arrive at runtime.
//!
//! The rule itself is `config::TemplateName`. This is the store's door onto
//! it: an upload's file name comes off an HTTP request rather than out of a
//! configuration, so it needs the same question asked with a `StoreError`
//! for an answer. The rule is not restated here -- it was, once, and a
//! configuration and an upload could then have disagreed about what a name is.

use doppel_core_self::config::TemplateName;

use super::StoreError;

use crate as doppel_core_self;

/// Check a template file name and return it unchanged if it is safe to join
/// to a directory.
pub fn sanitize(name: &str) -> Result<&str, StoreError> {
    TemplateName::check(name).map_err(|reason| StoreError::BadTemplateName {
        name: name.to_owned(),
        reason: reason.to_string(),
    })?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bound lives on the type; repeated here only so the boundary
    /// tests below can name it.
    const MAX_LEN: usize = 200;

    /// Assert `result` is specifically `StoreError::BadTemplateName`, with
    /// both fields populated as documented, rather than merely `is_err()`.
    /// A regression that returned the right verdict through the wrong
    /// variant (or with an empty `reason`) would still pass an `is_err()`
    /// check, so that alone does not pin the contract.
    fn assert_bad_template_name(result: Result<&str, StoreError>, expected_name: &str) {
        match result {
            Err(StoreError::BadTemplateName { name, reason }) => {
                assert_eq!(name, expected_name);
                assert!(!reason.is_empty(), "reason must be populated");
            }
            other => panic!("expected BadTemplateName for `{expected_name}`, got {other:?}"),
        }
    }

    #[test]
    fn plain_names_are_accepted() {
        assert_eq!(sanitize("delete.html.j2").unwrap(), "delete.html.j2");
        assert_eq!(sanitize("put.json.j2").unwrap(), "put.json.j2");
        assert_eq!(sanitize("a-b_c.1.j2").unwrap(), "a-b_c.1.j2");
    }

    #[test]
    fn traversal_is_rejected_not_normalized() {
        for bad in ["../etc/passwd", "..", "a/b", "a\\b", "/abs", "./x"] {
            assert_bad_template_name(sanitize(bad), bad);
        }
    }

    #[test]
    fn hidden_and_empty_names_are_rejected() {
        assert_bad_template_name(sanitize(".hidden"), ".hidden");
        assert_bad_template_name(sanitize(""), "");
        assert_bad_template_name(sanitize("   "), "   ");
    }

    #[test]
    fn control_characters_and_nul_are_rejected() {
        assert_bad_template_name(sanitize("a\nb.j2"), "a\nb.j2");
        assert_bad_template_name(sanitize("a\0b.j2"), "a\0b.j2");
    }

    #[test]
    fn overlong_names_are_rejected() {
        let long = "a".repeat(256) + ".j2";
        assert_bad_template_name(sanitize(&long), &long);
    }

    #[test]
    fn a_name_at_the_maximum_length_is_accepted_and_one_byte_more_is_rejected() {
        // An off-by-one in the `> MAX_LEN` comparison (e.g. `>=`) would
        // reject exactly this boundary while every other test here keeps
        // passing, since none of them exercises the limit itself.
        let at_max = "n".repeat(MAX_LEN - 3) + ".j2";
        assert_eq!(at_max.len(), MAX_LEN);
        assert_eq!(sanitize(&at_max).unwrap(), at_max);

        let over_max = format!("{at_max}x");
        assert_eq!(over_max.len(), MAX_LEN + 1);
        assert_bad_template_name(sanitize(&over_max), &over_max);
    }
}
