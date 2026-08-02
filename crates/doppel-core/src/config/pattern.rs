//! Mock URL patterns, parsed rather than validated.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A mock's `request.url`: a regular expression matched against a request
/// path.
///
/// Kept compiled. Rule V18 compiled it to find out whether it was valid and
/// threw the result away; `Runtime::compile` then compiled it again for real,
/// with its own failure branch for a pattern validation should already have
/// refused. One compile now, at the moment the pattern comes into existence.
///
/// The pattern is unanchored, deliberately: one written for
/// `/api/v1/resource/` also matches `/api/v1/resource/42/`, and only
/// declaration order distinguishes the two. That is a documented property of
/// how mocks match, not something to fix here.
#[derive(Debug, Clone)]
pub struct Pattern {
    /// The text as written, so serialization reproduces the document.
    raw: String,
    regex: regex::Regex,
}

/// Why a pattern was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    #[error("`{pattern}` is not a valid regex: {reason}")]
    NotARegex { pattern: String, reason: String },
}

impl Pattern {
    /// Check a string and keep it compiled, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, PatternError> {
        let raw = value.into();
        let regex = regex::Regex::new(&raw).map_err(|err| PatternError::NotARegex {
            pattern: raw.clone(),
            reason: err.to_string(),
        })?;
        Ok(Self { raw, regex })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    #[must_use]
    pub fn as_regex(&self) -> &regex::Regex {
        &self.regex
    }

    /// The named capture groups, in the order the pattern declares them.
    #[must_use]
    pub fn capture_names(&self) -> Vec<String> {
        self.regex
            .capture_names()
            .flatten()
            .map(str::to_owned)
            .collect()
    }
}

/// Two patterns are the same when the same text was written.
///
/// Hand-written because `regex::Regex` is not `PartialEq`, and comparing the
/// source is the right answer anyway: the revision is computed over the
/// document, so two configurations differ exactly when their text differs.
impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for Pattern {}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl FromStr for Pattern {
    type Err = PatternError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for Pattern {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Pattern {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(
                "A regular expression matched against the request path, \
                 unanchored. Named capture groups become template variables.",
            ))
            .examples([serde_json::json!(r"/api/(?P<id>\d+)/")])
            .into()
    }
}

impl utoipa::ToSchema for Pattern {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_exposes_its_capture_names_in_order() {
        let pattern = Pattern::parse(r"/api/(?P<zebra>\d+)/(?P<alpha>\w+)/").unwrap();
        assert_eq!(pattern.capture_names(), ["zebra", "alpha"]);
    }

    #[test]
    fn a_pattern_with_no_captures_has_none() {
        assert!(
            Pattern::parse("/health/")
                .unwrap()
                .capture_names()
                .is_empty()
        );
    }

    #[test]
    fn an_uncompilable_pattern_is_refused_with_the_regex_error() {
        // This was V18. The message keeps the regex crate's own explanation,
        // which says which construct is unclosed and where.
        let err = Pattern::parse("/api/(unclosed/").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("not a valid regex"), "{message}");
        assert!(message.contains("/api/(unclosed/"), "{message}");
    }

    #[test]
    fn a_pattern_is_unanchored_and_that_is_deliberate() {
        // Documented behaviour, pinned here because the type now owns the
        // compiled regex and could quietly grow anchors.
        let pattern = Pattern::parse("/api/v1/resource/").unwrap();
        assert!(pattern.as_regex().is_match("/api/v1/resource/"));
        assert!(pattern.as_regex().is_match("/api/v1/resource/42/"));
        assert!(pattern.as_regex().is_match("/prefix/api/v1/resource/"));
    }

    #[test]
    fn equality_is_the_text_that_was_written() {
        // `regex::Regex` is not `PartialEq`, and the source is what the
        // revision is computed over.
        assert_eq!(
            Pattern::parse("/a/").unwrap(),
            Pattern::parse("/a/").unwrap()
        );
        assert_ne!(
            Pattern::parse("/a/").unwrap(),
            Pattern::parse("/b/").unwrap()
        );
    }

    #[test]
    fn a_pattern_round_trips_as_the_text_that_was_written() {
        let raw = r"/api/(?P<id>\d+)/";
        let pattern = Pattern::parse(raw).unwrap();
        let yaml = serde_norway::to_string(&pattern).unwrap();
        assert_eq!(serde_norway::from_str::<Pattern>(&yaml).unwrap(), pattern);
        assert_eq!(
            serde_norway::from_str::<Pattern>(&yaml).unwrap().as_str(),
            raw
        );
    }

    #[test]
    fn deserializing_a_bad_pattern_carries_the_reason() {
        let err = serde_norway::from_str::<Pattern>("\"/api/(unclosed/\"").unwrap_err();
        assert!(err.to_string().contains("not a valid regex"), "{err}");
    }
}
