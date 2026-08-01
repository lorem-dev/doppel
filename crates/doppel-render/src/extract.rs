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

    /// A minijinja context built from the bound variables, for the renderer
    /// to pass to `Environment::render_str`/`render`.
    #[must_use]
    pub fn as_context(&self) -> minijinja::Value {
        minijinja::Value::from_serialize(&self.0)
    }
}

/// A parsed `.a.b.c` selector: dot-separated segments after a leading dot,
/// addressing object keys only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(Vec<String>);

impl Selector {
    /// Parses a selector of the form `.a.b.c`. Config validation (V23)
    /// already enforces this shape, so a failure here is defence in depth
    /// rather than the primary check.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        let rest = raw.strip_prefix('.').ok_or_else(|| {
            Error::new(
                ErrorCode::BodyExtractionError,
                format!("selector `{raw}` must start with `.`"),
            )
        })?;

        let segments: Vec<String> = rest.split('.').map(str::to_owned).collect();
        if segments.iter().any(String::is_empty) {
            return Err(Error::new(
                ErrorCode::BodyExtractionError,
                format!("selector `{raw}` has an empty segment"),
            ));
        }

        Ok(Self(segments))
    }

    /// Walks `root` by the selector's segments. Only object keys are
    /// addressed; a segment that reaches an array yields the array itself,
    /// and a missing key yields `None` rather than an error, since an
    /// absent field is a normal outcome.
    #[must_use]
    pub fn eval<'v>(&self, root: &'v serde_json::Value) -> Option<&'v serde_json::Value> {
        self.0
            .iter()
            .try_fold(root, |value, segment| value.as_object()?.get(segment))
    }
}

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
    fn a_single_segment_selector_parses() {
        assert_eq!(
            Selector::parse(".filter").unwrap(),
            Selector(vec!["filter".to_owned()])
        );
    }

    #[test]
    fn a_multi_segment_selector_parses() {
        assert_eq!(
            Selector::parse(".content.items").unwrap(),
            Selector(vec!["content".to_owned(), "items".to_owned()])
        );
    }

    #[test]
    fn a_selector_without_a_leading_dot_is_rejected() {
        let err = Selector::parse("content.items").unwrap_err();
        assert_eq!(err.code, ErrorCode::BodyExtractionError);
    }

    #[test]
    fn a_lone_dot_is_rejected_as_an_empty_segment() {
        let err = Selector::parse(".").unwrap_err();
        assert_eq!(err.code, ErrorCode::BodyExtractionError);
    }

    #[test]
    fn a_missing_segment_yields_none() {
        let root = serde_json::json!({"content": {"id": 1}});
        let selector = Selector::parse(".content.missing").unwrap();
        assert_eq!(selector.eval(&root), None);
    }

    #[test]
    fn a_segment_reaching_an_array_yields_the_array_itself() {
        let root = serde_json::json!({"content": {"items": [1, 2, 3]}});
        let selector = Selector::parse(".content.items").unwrap();
        assert_eq!(selector.eval(&root), Some(&serde_json::json!([1, 2, 3])));
    }

    #[test]
    fn a_selector_against_a_scalar_root_yields_none_rather_than_panicking() {
        let root = serde_json::json!(42);
        let selector = Selector::parse(".content.items").unwrap();
        assert_eq!(selector.eval(&root), None);
    }

    #[test]
    fn a_deeply_nested_path_resolves() {
        let root = serde_json::json!({"a": {"b": {"c": {"d": "found"}}}});
        let selector = Selector::parse(".a.b.c.d").unwrap();
        assert_eq!(selector.eval(&root), Some(&serde_json::json!("found")));
    }

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
