//! Body and query selectors, parsed rather than validated.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A `.a.b.c` selector: dot-separated segments after a leading dot,
/// addressing object keys only.
///
/// Kept parsed. There used to be two of these -- rule V23 checked the shape
/// at configuration time and `doppel_render::Selector` parsed it again per
/// request, its own doc comment calling the second parse "defence in depth
/// rather than the primary check". Two definitions of the same grammar in two
/// crates is a thing to keep in step, not a defence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Selector {
    /// The text as written, so serialization reproduces the document.
    raw: String,
    segments: Vec<String>,
}

/// Why a selector was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectorError {
    #[error("selector `{0}` must start with `.`")]
    NoLeadingDot(String),
    #[error("a selector must name at least one field; `.` names none")]
    NoFields,
    #[error("selector `{0}` has an empty path segment")]
    EmptySegment(String),
}

impl Selector {
    /// Check a string and keep it parsed, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, SelectorError> {
        let raw = value.into();
        let Some(rest) = raw.strip_prefix('.') else {
            return Err(SelectorError::NoLeadingDot(raw));
        };
        if rest.is_empty() {
            return Err(SelectorError::NoFields);
        }
        if rest.split('.').any(str::is_empty) {
            return Err(SelectorError::EmptySegment(raw));
        }
        let segments = rest.split('.').map(str::to_owned).collect();
        Ok(Self { raw, segments })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The object keys to walk, in order. Never empty.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }

    /// Walks `root` by the selector's segments.
    ///
    /// Only object keys are addressed; a segment that reaches an array
    /// yields the array itself, and a missing key yields `None` rather than
    /// an error, since an absent field is a normal outcome for a request
    /// that simply did not carry it.
    ///
    /// Infallible, which is the point of parsing at configuration time: the
    /// request path used to re-parse the text on every request and had to
    /// handle a failure that a loaded configuration could not produce.
    #[must_use]
    pub fn eval<'v>(&self, root: &'v serde_json::Value) -> Option<&'v serde_json::Value> {
        self.segments
            .iter()
            .try_fold(root, |value, segment| value.as_object()?.get(segment))
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl FromStr for Selector {
    type Err = SelectorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for Selector {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Selector {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Selector {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .pattern(Some(r"^\.[^.]+(\.[^.]+)*$"))
            .description(Some(
                "A selector addressing object keys: a leading dot, then \
                 dot-separated field names, as in `.content.items`.",
            ))
            .examples([serde_json::json!(".content.items")])
            .into()
    }
}

impl utoipa::ToSchema for Selector {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selector_keeps_its_segments_in_order() {
        let selector = Selector::parse(".content.items").unwrap();
        assert_eq!(selector.segments(), ["content", "items"]);
        assert_eq!(selector.as_str(), ".content.items");

        let single = Selector::parse(".filter").unwrap();
        assert_eq!(single.segments(), ["filter"]);
    }

    #[test]
    fn the_three_ways_to_write_it_wrong_are_told_apart() {
        // One variant each, because the fixes differ: a missing dot is a
        // forgotten prefix, a lone dot is an unfinished selector, and a
        // doubled dot is a typo in the middle of one.
        assert_eq!(
            Selector::parse("content.items"),
            Err(SelectorError::NoLeadingDot("content.items".to_owned()))
        );
        assert_eq!(Selector::parse("."), Err(SelectorError::NoFields));
        assert_eq!(
            Selector::parse(".content..items"),
            Err(SelectorError::EmptySegment(".content..items".to_owned()))
        );
        assert_eq!(
            Selector::parse(".content."),
            Err(SelectorError::EmptySegment(".content.".to_owned()))
        );
        assert_eq!(
            Selector::parse(""),
            Err(SelectorError::NoLeadingDot(String::new()))
        );
    }

    #[test]
    fn segments_are_never_empty_for_a_selector_that_parsed() {
        // What lets the walker be total: there is no parsed selector that
        // addresses nothing.
        for value in [".a", ".a.b", ".a.b.c", ".x-y_z"] {
            assert!(!Selector::parse(value).unwrap().segments().is_empty());
        }
    }

    #[test]
    fn eval_walks_object_keys_and_stops_at_anything_else() {
        let root = serde_json::json!({
            "content": {"items": [1, 2], "name": "x"},
            "flat": 7
        });
        assert_eq!(
            Selector::parse(".content.name").unwrap().eval(&root),
            Some(&serde_json::json!("x"))
        );
        // A segment reaching an array yields the array itself.
        assert_eq!(
            Selector::parse(".content.items").unwrap().eval(&root),
            Some(&serde_json::json!([1, 2]))
        );
        // A missing key is `None`, not an error: a request that did not
        // carry the field is a normal outcome, not a malformed one.
        assert_eq!(Selector::parse(".absent").unwrap().eval(&root), None);
        assert_eq!(
            Selector::parse(".content.absent").unwrap().eval(&root),
            None
        );
        // Walking through a non-object stops rather than panicking.
        assert_eq!(Selector::parse(".flat.deeper").unwrap().eval(&root), None);
        // Nor does a scalar root.
        assert_eq!(
            Selector::parse(".content.items")
                .unwrap()
                .eval(&serde_json::json!(42)),
            None
        );
        // Depth is not special-cased anywhere.
        assert_eq!(
            Selector::parse(".a.b.c.d")
                .unwrap()
                .eval(&serde_json::json!({"a": {"b": {"c": {"d": "found"}}}})),
            Some(&serde_json::json!("found"))
        );
    }

    #[test]
    fn a_selector_round_trips_as_the_text_that_was_written() {
        let selector = Selector::parse(".content.items").unwrap();
        let yaml = serde_norway::to_string(&selector).unwrap();
        assert_eq!(yaml.trim(), ".content.items");
        assert_eq!(serde_norway::from_str::<Selector>(&yaml).unwrap(), selector);
    }

    #[test]
    fn deserializing_a_bad_selector_carries_the_reason() {
        let err = serde_norway::from_str::<Selector>("\"content\"").unwrap_err();
        assert!(err.to_string().contains("must start with `.`"), "{err}");
    }
}
