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
pub const MIN: usize = 2;

/// The longest name accepted by default.
///
/// A name becomes a path component, and 64 leaves generous room under the
/// 255-byte limit both target platforms impose even once a prefix and an
/// extension are added around it. It was 128, which was the platform limit
/// reasoned about rather than a length anyone would type.
pub const MAX: usize = 64;

/// The longest proxy name accepted.
///
/// Tighter than the rest, because a proxy name travels further than any other:
/// it is a directory under `templates.dir`, a `proxy` label on every metric, a
/// field in every log line, and the value a client writes into a resolution
/// header on every request. Each of those is somewhere a long name is paid for
/// repeatedly rather than once.
pub const MAX_PROXY: usize = 32;

/// The character class a name is built from, as a regex fragment.
///
/// Public so `AllowedGroup`'s schema can compose its own pattern from it rather
/// than restating the class -- two spellings of one rule drift, and the schema is
/// the copy nobody compiles.
pub const CHARACTERS: &str = "[A-Za-z0-9_-]";

/// A validated name.
///
/// Letters, digits, `-` and `_`, between [`MIN`] and `MAX_LEN` characters --
/// [`MAX`] by default, [`MAX_PROXY`] for a [`ProxyName`].
///
/// The dot used to be allowed, for names like `Billing.API.v2`. It is not any
/// more, and dropping it removed two rules with it: a name becomes a directory
/// component, so `.hidden` and `..` each had to be refused separately. Without
/// the dot neither shape can be written, and the type is a character set and a
/// length again rather than a character set, a length and two exceptions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name<const MAX_LEN: usize = MAX>(String);

/// A proxy name: the same rules, capped at [`MAX_PROXY`].
pub type ProxyName = Name<MAX_PROXY>;

/// Why a name was refused.
///
/// One variant per rule rather than one message for all of them, so the text
/// can say what is wrong instead of restating the rule and leaving the reader
/// to spot which part they broke.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a name must be at least {MIN} characters, `{0}` is {len}", len = .0.chars().count())]
    TooShort(String),
    #[error("a name must be at most {1} characters, this one is {0}")]
    TooLong(usize, usize),
    /// The dot gets its own message. It was accepted until 0.3.0, and the
    /// reference configuration taught it, so somebody hitting this is more
    /// likely to be carrying an old name forward than to have mistyped -- and
    /// "contains '.'" alone reads like the character set is being restated.
    #[error("a name may no longer contain `.`; `{0}` does -- use `-` or `_` instead")]
    Dot(String),
    #[error("a name may contain letters, digits, `-` and `_`; `{0}` contains {1:?}")]
    BadCharacter(String, char),
}

impl<const MAX_LEN: usize> Name<MAX_LEN> {
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
        if length > MAX_LEN {
            return Err(NameError::TooLong(length, MAX_LEN));
        }
        // Checked before the general character test so the dot gets its own
        // message rather than being reported as just another rejected
        // character.
        if value.contains('.') {
            return Err(NameError::Dot(value));
        }
        if let Some(bad) = value
            .chars()
            .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_'))
        {
            return Err(NameError::BadCharacter(value, bad));
        }

        // No `.hidden` or `..` check is needed: neither can be written without a
        // dot, so a name is a safe directory component by construction rather
        // than by a further rule.
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

impl<const MAX_LEN: usize> fmt::Display for Name<MAX_LEN> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const MAX_LEN: usize> FromStr for Name<MAX_LEN> {
    type Err = NameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<const MAX_LEN: usize> AsRef<str> for Name<MAX_LEN> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// So a `BTreeMap<Name, _>` or a `Vec<Name>` can be looked up by `&str`
/// without building a `Name` to throw away.
impl<const MAX_LEN: usize> Borrow<str> for Name<MAX_LEN> {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl<const MAX_LEN: usize> PartialEq<str> for Name<MAX_LEN> {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl<const MAX_LEN: usize> PartialEq<&str> for Name<MAX_LEN> {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl<const MAX_LEN: usize> PartialEq<Name<MAX_LEN>> for str {
    fn eq(&self, other: &Name<MAX_LEN>) -> bool {
        self == other.0
    }
}

impl<const MAX_LEN: usize> Serialize for Name<MAX_LEN> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de, const MAX_LEN: usize> Deserialize<'de> for Name<MAX_LEN> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl<const MAX_LEN: usize> utoipa::PartialSchema for Name<MAX_LEN> {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .pattern(Some(format!("^{CHARACTERS}{{{MIN},{MAX_LEN}}}$")))
            .min_length(Some(MIN))
            // The cap that actually applies, so a proxy name's schema says 32
            // and every other name's says 64 rather than both restating one
            // number that is right for neither.
            .max_length(Some(MAX_LEN))
            .description(Some(format!(
                "Letters, digits, `-` and `_`, between {MIN} and {MAX_LEN} characters."
            )))
            .into()
    }
}

impl<const MAX_LEN: usize> utoipa::ToSchema for Name<MAX_LEN> {
    /// One schema name per cap.
    ///
    /// `utoipa` derives a component name from the type name alone, so
    /// `Name<64>` and `Name<32>` would both be `Name` and the second would
    /// silently overwrite the first -- leaving the generated schema claiming a
    /// proxy name may be 64 characters when the type refuses 33. `utoipa`'s own
    /// documentation warns about exactly this for generic types.
    fn name() -> std::borrow::Cow<'static, str> {
        match MAX_LEN {
            MAX_PROXY => std::borrow::Cow::Borrowed("ProxyName"),
            MAX => std::borrow::Cow::Borrowed("Name"),
            // No other cap exists today; if one is added it gets a name of its
            // own rather than colliding with these two.
            _ => std::borrow::Cow::Owned(format!("Name{MAX_LEN}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default cap, spelled out: `Name` alone leaves the const parameter to
    /// be inferred, and in a test there is nothing to infer it from.
    type Default = Name<MAX>;

    #[test]
    fn the_names_people_actually_write_are_accepted() {
        for name in [
            "p1",
            "ops",
            "alpha",
            "billing-api",
            "billing_api",
            "BillingAPIv2",
            "a".repeat(MAX).as_str(),
        ] {
            assert!(
                Default::parse(name).is_ok(),
                "`{name}` should be a legal name"
            );
        }
    }

    #[test]
    fn a_single_character_is_refused_but_two_are_not() {
        // Two, not four. Four refuses `ops`, and a group nobody can name for
        // being three letters long is a rule obstructing its own purpose.
        assert!(matches!(Default::parse("a"), Err(NameError::TooShort(_))));
        assert!(Default::parse("ci").is_ok());
        assert!(Default::parse("ops").is_ok());
    }

    /// The dot was legal until 0.3.0 and the reference configuration taught
    /// `Billing.API.v2`, so it gets a message that says it was removed and what
    /// to write instead -- not one that reads like the character set being
    /// restated.
    #[test]
    fn a_dot_is_refused_and_the_message_offers_a_replacement() {
        let err = Default::parse("Billing.API.v2").unwrap_err();
        assert!(matches!(err, NameError::Dot(_)), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("no longer"), "{text}");
        assert!(text.contains('-') && text.contains('_'), "{text}");
    }

    /// Without the dot there is no `..` and no `.hidden` to check for
    /// separately: a name is a usable directory component by construction.
    #[test]
    fn the_shapes_that_needed_their_own_rules_are_unwritable_now() {
        for name in ["..", ".hidden", "a..b", "..a"] {
            let err = Default::parse(name).unwrap_err();
            assert!(
                matches!(err, NameError::Dot(_) | NameError::TooShort(_)),
                "`{name}` -> {err:?}"
            );
        }
    }

    #[test]
    fn the_message_says_what_is_wrong_not_just_that_something_is() {
        // The whole point of one variant per rule. A reader who sees the rule
        // restated still has to work out which part they broke.
        let short = Default::parse("a").unwrap_err().to_string();
        assert!(short.contains("at least 2"), "{short}");
        assert!(short.contains("is 1"), "{short}");

        let bad = Default::parse("a/b").unwrap_err().to_string();
        assert!(
            bad.contains('/'),
            "the offending character must be named: {bad}"
        );

        let long = Default::parse("a".repeat(MAX + 1)).unwrap_err().to_string();
        assert!(long.contains("at most 64"), "{long}");
        assert!(long.contains("65"), "{long}");
    }

    /// A proxy name travels further than any other -- a directory, a metric
    /// label, a log field, and a header value on every request -- so it is
    /// capped tighter. The message has to quote the cap that was applied, not
    /// the default one.
    #[test]
    fn a_proxy_name_is_capped_shorter_than_other_names() {
        let thirty_three = "a".repeat(MAX_PROXY + 1);
        assert!(
            Default::parse(&thirty_three).is_ok(),
            "still fine for a token or a group"
        );

        let err = ProxyName::parse(&thirty_three).unwrap_err();
        assert!(matches!(err, NameError::TooLong(33, 32)), "{err:?}");
        let text = err.to_string();
        assert!(text.contains("at most 32"), "{text}");

        assert!(ProxyName::parse("a".repeat(MAX_PROXY)).is_ok());
    }

    #[test]
    fn a_name_that_could_escape_a_directory_is_refused() {
        // A name becomes a path component. These are the shapes that stop it
        // being one, and the reason the store's `sanitize` and this type agree
        // about what a name is.
        for name in ["..", "a/b", "a\\b", "a b", "a\u{0}b"] {
            assert!(Default::parse(name).is_err(), "`{name}` must be refused");
        }
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // Two accented letters are two characters and four bytes. The length
        // check runs before the character set check, so its message has to be
        // right even for input the next check will reject.
        let err = Default::parse("é").unwrap_err();
        assert!(
            matches!(err, NameError::TooShort(_)),
            "one character is short regardless of its byte length: {err:?}"
        );
        let two = Default::parse("éé").unwrap_err();
        assert!(
            matches!(two, NameError::BadCharacter(..)),
            "two characters are long enough, and then refused for the character: {two:?}"
        );
    }

    #[test]
    fn a_name_round_trips_through_yaml() {
        let name = Default::parse("billing-api").unwrap();
        let yaml = serde_norway::to_string(&name).unwrap();
        assert_eq!(serde_norway::from_str::<Default>(&yaml).unwrap(), name);
    }

    #[test]
    fn deserializing_a_bad_name_fails_with_the_reason() {
        let err = serde_norway::from_str::<Default>("\"a/b\"").unwrap_err();
        assert!(err.to_string().contains('/'), "{err}");
    }

    /// The cap belongs to the type, so it has to survive deserialization rather
    /// than only being applied by a direct `parse`.
    #[test]
    fn the_proxy_cap_applies_when_deserializing_too() {
        let long = format!("\"{}\"", "a".repeat(MAX_PROXY + 1));
        assert!(serde_norway::from_str::<Default>(&long).is_ok());
        let err = serde_norway::from_str::<ProxyName>(&long).unwrap_err();
        assert!(err.to_string().contains("at most 32"), "{err}");
    }
}
