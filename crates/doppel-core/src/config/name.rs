//! Names, parsed rather than validated.
//!
//! A proxy name, a mock name, a token name and a group name are all the same
//! kind of thing: a short label an operator writes, that ends up in a path
//! component, a metric label, a log line and a URL. Making it one type means
//! the rules are stated once and cannot be enforced in three places and
//! forgotten in a fourth.
//!
//! The check happens while the document is being parsed, so a `Config` in hand
//! is one whose names are already known good -- there is no later moment at
//! which an unchecked name exists.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The shortest name accepted.
///
/// Two rather than one: a single character is almost always a slip, and
/// nothing legitimate is shorter than `p1` or `ci`. Not four, which was the
/// first proposal -- it refuses `ops`, and a group nobody can name for being
/// three letters long is a rule getting in the way of its own purpose.
const MIN: usize = 2;

/// The longest name accepted.
///
/// A name becomes a path component, and 128 leaves room under the 255-byte
/// limit both target platforms impose even once a prefix and an extension are
/// added around it.
const MAX: usize = 128;

/// A validated name.
///
/// Letters, digits, `.`, `-` and `_`, between 2 and 128 characters. The dot is
/// allowed because the reference configuration already documents names like
/// `Billing.API.v2`, and removing a spelling the documentation teaches is a
/// cost with no matching benefit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

/// Why a name was refused.
///
/// One variant per rule rather than one message for all of them, so the text
/// can say what is wrong instead of restating the rule and leaving the reader
/// to spot which part they broke.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a name must be at least {MIN} characters, `{0}` is {len}", len = .0.chars().count())]
    TooShort(String),
    #[error("a name must be at most {MAX} characters, this one is {0}")]
    TooLong(usize),
    #[error("a name may contain letters, digits, `.`, `-` and `_`; `{0}` contains {1:?}")]
    BadCharacter(String, char),
    /// A name becomes a directory component, so the shapes that stop it being
    /// one are refused here rather than at the moment a file is written --
    /// which is a path error reported for a configuration mistake, and much
    /// later.
    #[error("a name must not start with a dot: `{0}`")]
    LeadingDot(String),
    #[error("a name must not contain `..`: `{0}`")]
    DotDot(String),
}

impl Name {
    /// Check a string and keep it, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, NameError> {
        let value = value.into();

        // Counted in characters, not bytes: a name of two accented letters is
        // two characters and four bytes, and refusing it for being "too long"
        // or accepting it as "long enough" by byte count would both be
        // surprising. The character set below happens to be ASCII, so the two
        // agree for anything that gets past it -- but the length message is
        // produced before that check, and has to be right on its own terms.
        let length = value.chars().count();
        if length < MIN {
            return Err(NameError::TooShort(value));
        }
        if length > MAX {
            return Err(NameError::TooLong(length));
        }
        if let Some(bad) = value
            .chars()
            .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-' | '_'))
        {
            return Err(NameError::BadCharacter(value, bad));
        }

        // The character set alone would admit `..` and `.hidden`, and a name
        // is a directory component. Catching them here is what lets the
        // separate validation rule that used to do it go away: one check, at
        // the moment the name comes into existence.
        if value.starts_with('.') {
            return Err(NameError::LeadingDot(value));
        }
        if value.contains("..") {
            return Err(NameError::DotDot(value));
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

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Name {
    type Err = NameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// So a `BTreeMap<Name, _>` or a `Vec<Name>` can be looked up by `&str`
/// without building a `Name` to throw away.
impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Name> for str {
    fn eq(&self, other: &Name) -> bool {
        self == other.0
    }
}

impl Serialize for Name {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Name {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .pattern(Some(r"^[A-Za-z0-9._-]+$"))
            .min_length(Some(MIN))
            .max_length(Some(MAX))
            .description(Some(
                "Letters, digits, `.`, `-` and `_`, between 2 and 128 characters.",
            ))
            .into()
    }
}

impl utoipa::ToSchema for Name {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_people_actually_write_are_accepted() {
        for name in [
            "p1",
            "ops",
            "alpha",
            "billing-api",
            "billing_api",
            "Billing.API.v2",
            "a".repeat(MAX).as_str(),
        ] {
            assert!(Name::parse(name).is_ok(), "`{name}` should be a legal name");
        }
    }

    #[test]
    fn a_single_character_is_refused_but_two_are_not() {
        // Two, not four. Four refuses `ops`, and a group nobody can name for
        // being three letters long is a rule obstructing its own purpose.
        assert!(matches!(Name::parse("a"), Err(NameError::TooShort(_))));
        assert!(Name::parse("ci").is_ok());
        assert!(Name::parse("ops").is_ok());
    }

    #[test]
    fn the_message_says_what_is_wrong_not_just_that_something_is() {
        // The whole point of one variant per rule. A reader who sees the rule
        // restated still has to work out which part they broke.
        let short = Name::parse("a").unwrap_err().to_string();
        assert!(short.contains("at least 2"), "{short}");
        assert!(short.contains("is 1"), "{short}");

        let bad = Name::parse("a/b").unwrap_err().to_string();
        assert!(
            bad.contains('/'),
            "the offending character must be named: {bad}"
        );

        let long = Name::parse("a".repeat(MAX + 1)).unwrap_err().to_string();
        assert!(long.contains("at most 128"), "{long}");
        assert!(long.contains("129"), "{long}");
    }

    #[test]
    fn a_name_that_could_escape_a_directory_is_refused() {
        // A name becomes a path component. These are the shapes that stop it
        // being one, and the reason the store's `sanitize` and this type agree
        // about what a name is.
        for name in ["..", "a/b", "a\\b", "a b", "a\u{0}b", ".."] {
            assert!(Name::parse(name).is_err(), "`{name}` must be refused");
        }
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // Two accented letters are two characters and four bytes. The length
        // check runs before the character set check, so its message has to be
        // right even for input the next check will reject.
        let err = Name::parse("é").unwrap_err();
        assert!(
            matches!(err, NameError::TooShort(_)),
            "one character is short regardless of its byte length: {err:?}"
        );
        let two = Name::parse("éé").unwrap_err();
        assert!(
            matches!(two, NameError::BadCharacter(..)),
            "two characters are long enough, and then refused for the character: {two:?}"
        );
    }

    #[test]
    fn a_name_round_trips_through_yaml() {
        let name = Name::parse("billing-api").unwrap();
        let yaml = serde_norway::to_string(&name).unwrap();
        assert_eq!(serde_norway::from_str::<Name>(&yaml).unwrap(), name);
    }

    #[test]
    fn deserializing_a_bad_name_fails_with_the_reason() {
        let err = serde_norway::from_str::<Name>("\"a/b\"").unwrap_err();
        assert!(err.to_string().contains('/'), "{err}");
    }
}
