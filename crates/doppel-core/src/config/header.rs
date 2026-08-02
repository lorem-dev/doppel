//! Header names and values, parsed rather than validated.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An HTTP field name: a non-empty RFC 9110 token.
///
/// Case is kept as written. Header names are case-insensitive on the wire, but
/// folding here would rewrite what an operator typed on every `config pull`,
/// and comparing case-insensitively would silently merge two map entries into
/// one. Neither is worth doing quietly; the code that matches an incoming
/// header folds case at that point instead.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeaderName(String);

/// An HTTP field value: visible ASCII, plus space and horizontal tab.
///
/// The exclusion that matters is CR and LF. A value carrying either would end
/// the header early and let whatever followed be read as a header of its own
/// -- response splitting, written into a configuration file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HeaderValue(String);

/// Why a header name was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderNameError {
    #[error("a header name must not be empty")]
    Empty,
    #[error("`{name}` is not a valid header name: it contains {character:?}")]
    BadCharacter { name: String, character: char },
}

/// Why a header value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderValueError {
    /// Named apart from the general case because it is the dangerous one and
    /// the reason deserves saying.
    #[error(
        "a header value must not contain a line break; one would end the \
         header early and let the rest be read as headers of its own"
    )]
    LineBreak,
    #[error("`{value}` is not a valid header value: it contains {character:?}")]
    BadCharacter { value: String, character: char },
}

/// RFC 9110 `tchar`.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '.'
                | '^'
                | '_'
                | '`'
                | '|'
                | '~'
        )
}

impl HeaderName {
    /// Check a string and keep it, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, HeaderNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(HeaderNameError::Empty);
        }
        if let Some(character) = value.chars().find(|c| !is_token_char(*c)) {
            return Err(HeaderNameError::BadCharacter {
                name: value,
                character,
            });
        }
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

    /// The name folded to lower case, for matching an incoming header.
    #[must_use]
    pub fn to_lowercase(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl HeaderValue {
    /// Check a string and keep it, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, HeaderValueError> {
        let value = value.into();
        if value.contains('\r') || value.contains('\n') {
            return Err(HeaderValueError::LineBreak);
        }
        if let Some(character) = value
            .chars()
            .find(|c| *c != '\t' && !('\u{20}'..='\u{7e}').contains(c))
        {
            return Err(HeaderValueError::BadCharacter { value, character });
        }
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
}

macro_rules! string_newtype_impls {
    ($type:ident, $error:ident, $description:expr) => {
        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $type {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        /// So a `BTreeMap` keyed by this type can be looked up with a `&str`.
        impl Borrow<str> for $type {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $type {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $type {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl FromStr for $type {
            type Err = $error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl Serialize for $type {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let value = String::deserialize(d)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }

        impl utoipa::PartialSchema for $type {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                utoipa::openapi::schema::ObjectBuilder::new()
                    .schema_type(utoipa::openapi::schema::Type::String)
                    .description(Some($description))
                    .into()
            }
        }

        impl utoipa::ToSchema for $type {}
    };
}

string_newtype_impls!(
    HeaderName,
    HeaderNameError,
    "An HTTP header name: a non-empty RFC 9110 token."
);
string_newtype_impls!(
    HeaderValue,
    HeaderValueError,
    "An HTTP header value: visible ASCII, space and tab, with no line breaks."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_names_a_configuration_writes_are_accepted() {
        for name in [
            "X-Request-ID",
            "Authorization",
            "content-type",
            "X-Proxy-Name",
            "x_custom.header~1",
        ] {
            assert!(HeaderName::parse(name).is_ok(), "`{name}` should parse");
        }
    }

    #[test]
    fn a_name_with_a_space_or_a_colon_is_refused() {
        for name in ["X Request ID", "X-Request-ID:", "X-Req\tID", "héader"] {
            assert!(
                matches!(
                    HeaderName::parse(name),
                    Err(HeaderNameError::BadCharacter { .. })
                ),
                "`{name}` must be refused"
            );
        }
        assert_eq!(HeaderName::parse(""), Err(HeaderNameError::Empty));
    }

    #[test]
    fn a_bad_name_says_which_character_is_the_problem() {
        let err = HeaderName::parse("X Request ID").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("' '"), "{message}");
    }

    #[test]
    fn case_is_kept_as_written() {
        // Folding would rewrite the operator's spelling on every
        // `config pull`; the matching code folds at the point of comparison
        // instead.
        let name = HeaderName::parse("X-Request-ID").unwrap();
        assert_eq!(name.as_str(), "X-Request-ID");
        assert_eq!(name.to_lowercase(), "x-request-id");
    }

    #[test]
    fn a_value_with_a_line_break_is_refused_by_name() {
        // The one exclusion worth its own variant: a CR or LF would end the
        // header early and let the rest be read as headers of its own.
        for value in ["a\r\nX-Injected: 1", "a\nb", "a\rb"] {
            assert_eq!(
                HeaderValue::parse(value),
                Err(HeaderValueError::LineBreak),
                "{value:?} must be refused"
            );
        }
        let message = HeaderValueError::LineBreak.to_string();
        assert!(message.contains("read as headers"), "{message}");
    }

    #[test]
    fn the_header_values_a_configuration_writes_are_accepted() {
        for value in ["Bearer abc", "", "a\tb", "application/json; charset=utf-8"] {
            assert!(
                HeaderValue::parse(value).is_ok(),
                "{value:?} should be legal"
            );
        }
    }

    #[test]
    fn a_value_outside_visible_ascii_is_refused() {
        for value in ["caf\u{e9}", "a\u{0}b", "a\u{7f}b"] {
            assert!(
                matches!(
                    HeaderValue::parse(value),
                    Err(HeaderValueError::BadCharacter { .. })
                ),
                "{value:?} must be refused"
            );
        }
    }

    #[test]
    fn both_types_round_trip_through_yaml() {
        let name = HeaderName::parse("X-Request-ID").unwrap();
        let yaml = serde_norway::to_string(&name).unwrap();
        assert_eq!(serde_norway::from_str::<HeaderName>(&yaml).unwrap(), name);

        let value = HeaderValue::parse("Bearer abc").unwrap();
        let yaml = serde_norway::to_string(&value).unwrap();
        assert_eq!(serde_norway::from_str::<HeaderValue>(&yaml).unwrap(), value);
    }

    #[test]
    fn a_header_name_works_as_a_map_key_in_both_directions() {
        // The `Borrow<str>` impl is what lets a lookup use a plain `&str`,
        // and the map has to survive a round trip through YAML with the key
        // parsed rather than taken on trust.
        use std::collections::BTreeMap;

        let yaml = "X-Request-ID: Bearer abc\n";
        let map: BTreeMap<HeaderName, HeaderValue> = serde_norway::from_str(yaml).unwrap();
        assert_eq!(map.get("X-Request-ID").unwrap().as_str(), "Bearer abc");
        assert_eq!(serde_norway::to_string(&map).unwrap(), yaml);

        let bad: Result<BTreeMap<HeaderName, HeaderValue>, _> =
            serde_norway::from_str("X Request ID: a\n");
        assert!(bad.is_err(), "a bad key must fail the map, not the lookup");
    }
}
