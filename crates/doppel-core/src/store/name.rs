//! Template file name checking.
//!
//! Names are rejected rather than normalized. Silently rewriting a path an
//! operator asked for is worse than refusing it: the operator then believes a
//! file landed somewhere it did not.

use super::StoreError;

/// Longest accepted template file name. Comfortably under the 255 byte limit
/// that both target platforms impose on a single path component.
const MAX_LEN: usize = 200;

/// Check a template file name and return it unchanged if it is safe to join to a
/// directory.
pub fn sanitize(name: &str) -> Result<&str, StoreError> {
    let reject = |reason: &str| {
        Err(StoreError::BadTemplateName {
            name: name.to_owned(),
            reason: reason.to_owned(),
        })
    };

    if name.is_empty() || name.trim().is_empty() {
        return reject("name is empty");
    }
    if name.len() > MAX_LEN {
        return reject("name is longer than 200 bytes");
    }
    if name.starts_with('.') {
        return reject("name must not start with a dot");
    }
    if name.contains('/') || name.contains('\\') {
        return reject("name must not contain a path separator");
    }
    if name.contains("..") {
        return reject("name must not contain `..`");
    }
    if name.bytes().any(|b| b.is_ascii_control()) {
        return reject("name must not contain control characters");
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

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
