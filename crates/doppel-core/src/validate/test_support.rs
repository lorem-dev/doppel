//! Test-only helpers shared by the validation rule tests.

use super::{Violation, validate};
use crate::config::load_from_str;

/// Parse a config that is expected to be well formed but invalid, and return
/// every violation. Panics on a parse error, since a rule test that cannot even
/// parse its fixture is a broken test rather than a failing rule.
pub(super) fn violations(text: &str) -> Vec<Violation> {
    let config = load_from_str(text).expect("fixture should parse");
    validate(&config).expect_err("fixture should be invalid")
}

/// Assert that validating `text` produces a violation at `path` whose message
/// contains `needle`.
pub(super) fn assert_violation(text: &str, path: &str, needle: &str) {
    let found = violations(text);
    assert!(
        found
            .iter()
            .any(|v| v.path == path && v.message.contains(needle)),
        "expected a violation at `{path}` containing `{needle}`, got {found:?}"
    );
}
