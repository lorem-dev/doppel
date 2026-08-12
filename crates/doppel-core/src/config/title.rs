//! The dashboard's heading, parsed rather than validated.
//!
//! A title is the one configuration value this project renders into a web page,
//! so it is the one value whose bounds are about a browser rather than about a
//! filesystem or a metric label. It is checked while the document is parsed,
//! like every other value, so no unchecked title exists at any later moment.
//!
//! Escaping is *not* this type's job and deliberately so. A title is legal text
//! that happens to be displayed; making the type refuse `<` would forbid a
//! perfectly reasonable heading in order to paper over a rendering bug, and
//! would still leave every other injection sink unprotected. The escaping
//! belongs where the HTML is produced, in `doppel-admin`, and has a test of its
//! own there.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The longest title accepted.
///
/// It sits in a page header next to the theme and sign-in controls, so the bound
/// is about what fits rather than about what a machine can hold: 64 characters
/// is a long service name and a short sentence, and anything past it is being
/// used as a subtitle.
pub const MAX_LEN: usize = 64;

/// The default heading, used when `admin.title` is absent.
pub const DEFAULT: &str = "Doppel";

/// A validated dashboard title.
///
/// Non-empty, at most [`MAX_LEN`] characters, no control characters. Not
/// restricted to ASCII: this is a UI string, which `AGENTS.md` names as the
/// exception to the repository's ASCII rule, and an operator naming their
/// staging environment in their own language is doing nothing wrong.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdminTitle(String);

/// Why a title was refused.
///
/// One variant per rule, so the message says what is wrong rather than restating
/// the rule and leaving the reader to work out which part they broke.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminTitleError {
    /// Empty and whitespace-only are one case on purpose: both render as a
    /// header with nothing in it, and an operator who wrote `title: " "` meant
    /// to write something.
    #[error("a title must not be empty")]
    Empty,
    #[error("a title must be at most {MAX_LEN} characters, this one is {0}")]
    TooLong(usize),
    /// A tab or a newline in a heading is either a copy-paste accident or an
    /// attempt to do something to the markup. Neither is worth accepting, and
    /// naming the character is what turns this into a fixable report.
    #[error("a title must not contain control characters; this one contains {0:?}")]
    Control(char),
}

impl AdminTitle {
    /// Check a string and keep it, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, AdminTitleError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(AdminTitleError::Empty);
        }
        // Counted in characters rather than bytes. A title is displayed, and
        // what a reader sees is characters; measuring the bound in bytes would
        // make the limit depend on the language the title is written in.
        let length = value.chars().count();
        if length > MAX_LEN {
            return Err(AdminTitleError::TooLong(length));
        }
        // `char::is_control` rather than `is_ascii_control`: the Unicode class
        // includes the ASCII one, and a title is not restricted to ASCII, so
        // checking only the ASCII range would let a directional override
        // through -- a character whose whole purpose is to make displayed text
        // disagree with the text that is stored.
        if let Some(bad) = value.chars().find(|c| c.is_control()) {
            return Err(AdminTitleError::Control(bad));
        }

        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AdminTitle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AdminTitle {
    type Err = AdminTitleError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl AsRef<str> for AdminTitle {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Serialize for AdminTitle {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AdminTitle {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for AdminTitle {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .min_length(Some(1))
            .max_length(Some(MAX_LEN))
            .description(Some(format!(
                "The heading the dashboard shows, at most {MAX_LEN} characters \
                 and free of control characters. Defaults to `{DEFAULT}`."
            )))
            .examples([serde_json::json!("billing-api (staging)")])
            .into()
    }
}

impl utoipa::ToSchema for AdminTitle {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_titles_people_actually_write_are_accepted() {
        for title in [
            DEFAULT,
            "Doppel (staging)",
            "billing-api doppelganger",
            "a".repeat(MAX_LEN).as_str(),
        ] {
            assert!(
                AdminTitle::parse(title).is_ok(),
                "`{title}` should be a legal title"
            );
        }
    }

    #[test]
    fn an_empty_or_blank_title_is_refused() {
        assert_eq!(AdminTitle::parse(""), Err(AdminTitleError::Empty));
        assert_eq!(AdminTitle::parse("   "), Err(AdminTitleError::Empty));
        assert_eq!(AdminTitle::parse("\t"), Err(AdminTitleError::Empty));
    }

    #[test]
    fn the_bound_is_counted_in_characters_not_bytes() {
        // Two bytes per character, so a byte-counted bound would refuse this at
        // half the stated length -- making the limit depend on the language the
        // title happens to be written in.
        let cyrillic = "\u{044f}".repeat(MAX_LEN);
        assert_eq!(
            cyrillic.len(),
            MAX_LEN * 2,
            "fixture is not two bytes a char"
        );
        assert!(AdminTitle::parse(&cyrillic).is_ok());

        assert_eq!(
            AdminTitle::parse("x".repeat(MAX_LEN + 1)),
            Err(AdminTitleError::TooLong(MAX_LEN + 1))
        );
    }

    #[test]
    fn a_control_character_is_refused_and_named() {
        assert_eq!(
            AdminTitle::parse("two\nlines"),
            Err(AdminTitleError::Control('\n'))
        );
        assert_eq!(
            AdminTitle::parse("a\u{0}b"),
            Err(AdminTitleError::Control('\u{0}'))
        );
    }

    /// Markup is legal text, and refusing it here would be the wrong place to
    /// fix the wrong problem: the escaping belongs where the HTML is written,
    /// and `doppel-admin` has a test that a title like this cannot close the
    /// script element it is injected into.
    #[test]
    fn markup_is_accepted_because_escaping_is_not_this_types_job() {
        let hostile = "</script><script>alert(1)</script>";
        assert!(AdminTitle::parse(hostile).is_ok());
    }

    #[test]
    fn a_title_round_trips_through_yaml() {
        let title: AdminTitle = serde_norway::from_str("\"Doppel (staging)\"").unwrap();
        assert_eq!(title.as_str(), "Doppel (staging)");
        assert_eq!(
            serde_norway::to_string(&title).unwrap().trim(),
            "Doppel (staging)"
        );
        assert!(serde_norway::from_str::<AdminTitle>("\"\"").is_err());
    }
}
