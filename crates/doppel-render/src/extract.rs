//! Variable extraction: turning already-extracted request pieces (path
//! captures, header values, query parameters, a parsed body) into a
//! [`Variables`] map that rendering consumes.

use std::collections::BTreeMap;

use doppel_core::{Error, ErrorCode};

/// The variables bound for one request, ready to render a template against.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Variables(BTreeMap<String, serde_json::Value>);

impl Variables {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: &str, value: serde_json::Value) {
        self.0.insert(name.to_owned(), value);
    }

    /// One bound value, for a caller that needs to read back what it bound.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.0.get(name)
    }

    /// Every name bound, in order. `BTreeMap`, so the order is the names'.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// A minijinja context built from the bound variables, for the renderer
    /// to pass to `Environment::render_str`/`render`.
    #[must_use]
    pub fn as_context(&self) -> minijinja::Value {
        minijinja::Value::from_serialize(&self.0)
    }
}

// There was a `Selector` here, parsing `.a.b.c` on every request and calling
// itself "defence in depth" behind rule V23. Both are gone: the selector is
// `doppel_core::config::Selector`, parsed once when the document is read, and
// walking it is a method on that type. One grammar, in one place.

/// Parses a JSON request body once, for every body selector of one mock.
///
/// An empty body yields JSON `null` rather than an error, so a mock that
/// declares body selectors but receives no body binds nothing instead of
/// failing; non-JSON input is a client mistake worth reporting.
pub fn parse_body(bytes: &[u8]) -> Result<serde_json::Value, Error> {
    if bytes.is_empty() {
        return Ok(serde_json::Value::Null);
    }

    serde_json::from_slice(bytes).map_err(|err| {
        Error::new(
            ErrorCode::BodyExtractionError,
            format!("request body is not valid json: {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_json_body_yields_body_extraction_error() {
        let err = parse_body(b"not json").unwrap_err();
        assert_eq!(err.code, ErrorCode::BodyExtractionError);
    }

    #[test]
    fn an_empty_body_yields_json_null() {
        assert_eq!(parse_body(b"").unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn variables_as_context_round_trips_a_value_into_minijinja() {
        let mut vars = Variables::new();
        vars.insert("name", serde_json::json!("ada"));
        vars.insert("count", serde_json::json!(3));

        let ctx = vars.as_context();
        assert_eq!(ctx.get_attr("name").unwrap().as_str(), Some("ada"));
        assert_eq!(ctx.get_attr("count").unwrap().as_i64(), Some(3));
    }
}
