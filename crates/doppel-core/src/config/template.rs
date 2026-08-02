//! Template file names, parsed rather than validated.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Longest accepted template file name, in bytes.
///
/// Comfortably under the 255-byte limit both target platforms impose on a
/// single path component, with room for the directory prefix around it.
const MAX_BYTES: usize = 200;

/// The name of a template file, safe to join to a directory.
///
/// Names are refused rather than normalised. Silently rewriting a path an
/// operator asked for is worse than refusing it: they then believe a file
/// landed somewhere it did not.
///
/// Two callers share this. A mock's `response.template` comes from a
/// configuration, where the check used to be rule V31; an upload's file name
/// arrives over HTTP, where the store checks it per request. Both are the
/// same question, so both ask it of the same type -- see
/// `store::name::sanitize`, which is now a thin wrapper that only changes the
/// error into the store's own.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TemplateName(String);

/// Why a template file name was refused.
///
/// One variant per rule, so the message can say which shape was the problem
/// rather than restating the whole list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateNameError {
    #[error("name is empty")]
    Empty,
    #[error("name is longer than {MAX_BYTES} bytes")]
    TooLong,
    #[error("name must not start with a dot")]
    LeadingDot,
    #[error("name must not contain a path separator")]
    PathSeparator,
    #[error("name must not contain `..`")]
    DotDot,
    #[error("name must not contain control characters")]
    ControlCharacter,
}

impl TemplateName {
    /// Check a string and keep it, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, TemplateNameError> {
        let value = value.into();
        Self::check(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// The rule itself, borrowed rather than owned, so the store's upload
    /// path can ask the question without building a `String` it will throw
    /// away when the answer is no.
    pub fn check(name: &str) -> Result<(), TemplateNameError> {
        if name.is_empty() || name.trim().is_empty() {
            return Err(TemplateNameError::Empty);
        }
        if name.len() > MAX_BYTES {
            return Err(TemplateNameError::TooLong);
        }
        if name.starts_with('.') {
            return Err(TemplateNameError::LeadingDot);
        }
        if name.contains('/') || name.contains('\\') {
            return Err(TemplateNameError::PathSeparator);
        }
        if name.contains("..") {
            return Err(TemplateNameError::DotDot);
        }
        if name.bytes().any(|b| b.is_ascii_control()) {
            return Err(TemplateNameError::ControlCharacter);
        }
        Ok(())
    }
}

impl fmt::Display for TemplateName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for TemplateName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for TemplateName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for TemplateName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for TemplateName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl FromStr for TemplateName {
    type Err = TemplateNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for TemplateName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TemplateName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for TemplateName {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .max_length(Some(MAX_BYTES))
            .description(Some(
                "A template file name: one path component, no separators, no \
                 leading dot, no `..`.",
            ))
            .examples([serde_json::json!("put.json.j2")])
            .into()
    }
}

impl utoipa::ToSchema for TemplateName {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_a_configuration_writes_are_accepted() {
        for name in ["delete.html.j2", "put.json.j2", "a-b_c.1.j2"] {
            assert_eq!(TemplateName::parse(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn each_refusal_names_the_shape_that_caused_it() {
        // One variant per rule: a reader who sees the whole list restated
        // still has to work out which part they broke.
        for (name, expected) in [
            ("", TemplateNameError::Empty),
            ("   ", TemplateNameError::Empty),
            (".hidden", TemplateNameError::LeadingDot),
            ("a/b.j2", TemplateNameError::PathSeparator),
            ("a\\b.j2", TemplateNameError::PathSeparator),
            ("a..b.j2", TemplateNameError::DotDot),
            ("a\u{0}b.j2", TemplateNameError::ControlCharacter),
        ] {
            assert_eq!(TemplateName::parse(name), Err(expected), "for `{name}`");
        }
        assert_eq!(
            TemplateName::parse("a".repeat(MAX_BYTES + 1)),
            Err(TemplateNameError::TooLong)
        );
        assert!(TemplateName::parse("a".repeat(MAX_BYTES)).is_ok());
    }

    #[test]
    fn a_traversal_is_refused_however_it_is_spelled() {
        for name in ["../../etc/passwd", "..", "../x", "x/../y"] {
            assert!(
                TemplateName::parse(name).is_err(),
                "`{name}` must be refused"
            );
        }
    }

    #[test]
    fn check_answers_the_same_question_without_allocating() {
        // What the store's upload path uses: the same rule, borrowed, so a
        // rejected name never becomes a `String`.
        assert_eq!(TemplateName::check("put.json.j2"), Ok(()));
        // `../x` trips the leading-dot check first: the rules are ordered,
        // and the first one a name breaks is the one reported.
        assert_eq!(
            TemplateName::check("../x"),
            Err(TemplateNameError::LeadingDot)
        );
        assert_eq!(
            TemplateName::check("x/../y"),
            Err(TemplateNameError::PathSeparator)
        );
    }

    #[test]
    fn a_template_name_round_trips_through_yaml() {
        let name = TemplateName::parse("put.json.j2").unwrap();
        let yaml = serde_norway::to_string(&name).unwrap();
        assert_eq!(serde_norway::from_str::<TemplateName>(&yaml).unwrap(), name);
    }

    #[test]
    fn deserializing_a_bad_name_carries_the_reason() {
        let err = serde_norway::from_str::<TemplateName>("\"../../etc/passwd\"").unwrap_err();
        assert!(
            err.to_string().contains("must not start with a dot"),
            "{err}"
        );
        let separator = serde_norway::from_str::<TemplateName>("\"a/b.j2\"").unwrap_err();
        assert!(
            separator.to_string().contains("path separator"),
            "{separator}"
        );
    }
}
