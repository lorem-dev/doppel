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

    #[test]
    fn plain_names_are_accepted() {
        assert_eq!(sanitize("delete.html.j2").unwrap(), "delete.html.j2");
        assert_eq!(sanitize("put.json.j2").unwrap(), "put.json.j2");
        assert_eq!(sanitize("a-b_c.1.j2").unwrap(), "a-b_c.1.j2");
    }

    #[test]
    fn traversal_is_rejected_not_normalized() {
        for bad in ["../etc/passwd", "..", "a/b", "a\\b", "/abs", "./x"] {
            assert!(sanitize(bad).is_err(), "`{bad}` must be rejected");
        }
    }

    #[test]
    fn hidden_and_empty_names_are_rejected() {
        assert!(sanitize(".hidden").is_err());
        assert!(sanitize("").is_err());
        assert!(sanitize("   ").is_err());
    }

    #[test]
    fn control_characters_and_nul_are_rejected() {
        assert!(sanitize("a\nb.j2").is_err());
        assert!(sanitize("a\0b.j2").is_err());
    }

    #[test]
    fn overlong_names_are_rejected() {
        let long = "a".repeat(256) + ".j2";
        assert!(sanitize(&long).is_err());
    }
}
