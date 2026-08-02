//! Admin tokens, parsed rather than validated.
//!
//! A token is the whole of the admin API's authentication: whoever presents it
//! is whoever it names. That makes two things properties of the type rather
//! than habits of the code that handles it -- a token is long enough to be
//! worth presenting, and it does not appear in a debug dump.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The shortest token accepted.
///
/// 32 characters is roughly 128 bits at four bits a character even for the
/// least dense encoding anyone uses, hex. Shorter than that and the length
/// bound stops describing a secret and starts describing a password.
const MIN: usize = 32;

/// The longest token accepted.
///
/// A token travels in a header on every admin request, and 255 is the point
/// past which a longer one buys nothing. It also keeps a whole `tokens:` block
/// comfortably inside the header-size limits proxies impose by default.
const MAX: usize = 255;

/// An admin token.
///
/// Printable ASCII with no spaces, 32 to 255 characters. The character set is
/// the one a header value admits, because that is where the token is read
/// from: a token containing a space or a control character could be written
/// into a configuration and would then never match anything a client could
/// send.
///
/// There is no required *form*. A UUID is the recommended shape and what
/// `generate` produces, but an operator pasting a token from a secret manager
/// should not have to reformat it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

/// Why a token was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// The length is quoted, but the value is not: an error message about a
    /// secret is still a place the secret would end up.
    #[error("a token must be at least {MIN} characters, this one is {0}")]
    TooShort(usize),
    #[error("a token must be at most {MAX} characters, this one is {0}")]
    TooLong(usize),
    #[error(
        "a token must be printable ASCII with no spaces, so that it can be sent \
         in a header; this one contains {0:?}"
    )]
    BadCharacter(char),
}

impl Token {
    /// Check a string and keep it, or say why not.
    pub fn parse(value: impl Into<String>) -> Result<Self, TokenError> {
        let value = value.into();

        // Counted in characters for the same reason `Name` does: the length
        // message is produced before the character-set check, so it has to be
        // right on its own terms even for input that check will reject.
        let length = value.chars().count();
        if length < MIN {
            return Err(TokenError::TooShort(length));
        }
        if length > MAX {
            return Err(TokenError::TooLong(length));
        }
        // `is_ascii_graphic` is exactly 0x21..=0x7E: printable ASCII with the
        // space excluded. That is the set a header value can carry without
        // quoting or folding.
        if let Some(bad) = value.chars().find(|c| !c.is_ascii_graphic()) {
            return Err(TokenError::BadCharacter(bad));
        }

        Ok(Self(value))
    }

    /// A fresh token: a version 4 UUID.
    ///
    /// 36 characters, comfortably past the lower bound, and a shape everyone
    /// recognises as a generated secret rather than a chosen one. The
    /// randomness comes from `rand`'s thread generator, which is a CSPRNG
    /// seeded from the operating system -- the same source a dedicated UUID
    /// crate would reach for, so pulling one in would add a dependency and no
    /// security.
    #[must_use]
    pub fn generate() -> Self {
        use rand::Rng as _;

        let mut bytes = [0u8; 16];
        rand::rng().fill(&mut bytes);
        // Version 4, variant 1, per RFC 9562. Neither affects the strength;
        // they make the value identifiable as a random UUID rather than a
        // time-based or name-based one.
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;

        let hex =
            |slice: &[u8]| -> String { slice.iter().map(|byte| format!("{byte:02x}")).collect() };
        let value = format!(
            "{}-{}-{}-{}-{}",
            hex(&bytes[0..4]),
            hex(&bytes[4..6]),
            hex(&bytes[6..8]),
            hex(&bytes[8..10]),
            hex(&bytes[10..16]),
        );

        Self::parse(value).expect("a generated UUID is a legal token")
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// Whether a presented string is this token.
    ///
    /// The comparison does not stop at the first differing byte. A comparison
    /// that did would take longer the more of a guess was correct, which is
    /// how a token gets recovered one character at a time from a remote
    /// timing measurement. This is not a hardened implementation -- a
    /// dedicated crate would also resist the compiler noticing what the loop
    /// computes -- but the naive early return is the part actually worth not
    /// writing.
    ///
    /// The length is compared first and does leak, which is why the length
    /// bounds above are a range rather than a secret.
    #[must_use]
    pub fn matches(&self, presented: &str) -> bool {
        let expected = self.0.as_bytes();
        let presented = presented.as_bytes();
        if expected.len() != presented.len() {
            return false;
        }
        let mut difference = 0u8;
        for (a, b) in expected.iter().zip(presented) {
            difference |= a ^ b;
        }
        std::hint::black_box(difference) == 0
    }
}

/// Redacted.
///
/// A `Config` is debug-formatted in error paths and, with `RUST_LOG=debug`,
/// in log lines. Deriving `Debug` here would put every admin token into
/// whatever collects those.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Also redacted, for the same reason. The value goes out through `as_str`
/// and `Serialize`, both of which are explicit about what they are doing;
/// `{}` in a message is not.
impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl FromStr for Token {
    type Err = TokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for Token {
    /// The real value: this is what writes `main.yaml` back out and what the
    /// revision is computed over. Redaction belongs in the formatting impls,
    /// where the audience is a human reading a log.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Token {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Token {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .min_length(Some(MIN))
            .max_length(Some(MAX))
            .description(Some(
                "An admin token: printable ASCII with no spaces, 32 to 255 \
                 characters. A version 4 UUID is the recommended shape.",
            ))
            .into()
    }
}

impl utoipa::ToSchema for Token {}

#[cfg(test)]
mod tests {
    use super::*;

    fn thirty_two() -> String {
        "a".repeat(MIN)
    }

    #[test]
    fn a_generated_token_is_a_uuid_and_parses() {
        let token = Token::generate();
        assert_eq!(token.as_str().len(), 36);
        assert_eq!(
            token.as_str().chars().filter(|c| *c == '-').count(),
            4,
            "{}",
            token.as_str()
        );
        // Version 4, variant 1: the two nibbles RFC 9562 fixes.
        let bytes: Vec<char> = token.as_str().chars().collect();
        assert_eq!(bytes[14], '4', "version nibble: {}", token.as_str());
        assert!(
            matches!(bytes[19], '8' | '9' | 'a' | 'b'),
            "variant nibble: {}",
            token.as_str()
        );
        assert!(Token::parse(token.as_str()).is_ok());
    }

    #[test]
    fn two_generated_tokens_differ() {
        // Not a test of the generator's quality -- one collision in 2^122 is
        // not something a test finds. It catches the generator that returns a
        // constant, which is the failure that actually happens.
        assert_ne!(Token::generate().as_str(), Token::generate().as_str());
    }

    #[test]
    fn a_short_token_is_refused_without_quoting_it() {
        let err = Token::parse("hunter2").unwrap_err();
        assert!(matches!(err, TokenError::TooShort(7)), "{err:?}");
        let message = err.to_string();
        assert!(message.contains("at least 32"), "{message}");
        assert!(
            !message.contains("hunter2"),
            "a message about a secret must not carry it: {message}"
        );
    }

    #[test]
    fn thirty_two_characters_is_the_boundary() {
        assert!(Token::parse(thirty_two()).is_ok());
        assert!(matches!(
            Token::parse("a".repeat(MIN - 1)),
            Err(TokenError::TooShort(31))
        ));
        assert!(Token::parse("a".repeat(MAX)).is_ok());
        assert!(matches!(
            Token::parse("a".repeat(MAX + 1)),
            Err(TokenError::TooLong(256))
        ));
    }

    #[test]
    fn a_token_must_be_sendable_in_a_header() {
        // Each of these would be accepted into a configuration and then never
        // match anything a client could put in a header value.
        for bad in [" ", "\t", "\n", "\u{7f}", "\u{e9}"] {
            let value = format!("{}{bad}", "a".repeat(MIN));
            assert!(
                matches!(Token::parse(&value), Err(TokenError::BadCharacter(_))),
                "{bad:?} must be refused"
            );
        }
        // Everything a secret manager is likely to hand over is fine.
        for good in [
            "0123456789abcdef0123456789abcdef",
            "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
            "sk_live_51H8xY2eZvKYlo2C0abcdefghijkl",
            "aGVsbG8td29ybGQtaGVsbG8td29ybGQtaGVsbG8=",
        ] {
            assert!(Token::parse(good).is_ok(), "`{good}` should be legal");
        }
    }

    #[test]
    fn a_token_does_not_appear_in_its_own_debug_or_display() {
        let token = Token::parse("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(format!("{token:?}"), "Token(<redacted>)");
        assert_eq!(format!("{token}"), "<redacted>");
        // The value is still reachable where it is meant to be.
        assert_eq!(token.as_str(), "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn a_token_inside_a_struct_is_redacted_too() {
        // The point of the impl: it is the derived `Debug` on the struct
        // holding the token that would otherwise leak it.
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            token: Token,
        }
        let holder = Holder {
            token: Token::parse("0123456789abcdef0123456789abcdef").unwrap(),
        };
        let text = format!("{holder:?}");
        assert!(!text.contains("0123456789"), "{text}");
    }

    #[test]
    fn matches_accepts_the_token_and_rejects_everything_else() {
        let token = Token::parse("0123456789abcdef0123456789abcdef").unwrap();
        assert!(token.matches("0123456789abcdef0123456789abcdef"));
        // Differs in the last character only -- the case an early-returning
        // comparison would answer measurably faster.
        assert!(!token.matches("0123456789abcdef0123456789abcdeg"));
        // Differs in the first.
        assert!(!token.matches("z123456789abcdef0123456789abcdef"));
        assert!(!token.matches(""));
        assert!(!token.matches("0123456789abcdef0123456789abcdef "));
    }

    #[test]
    fn a_token_round_trips_through_yaml_with_its_value_intact() {
        // Redaction must not reach serialization: `config pull` writes this
        // back out, and the revision is computed over it.
        let token = Token::parse("0123456789abcdef0123456789abcdef").unwrap();
        let yaml = serde_norway::to_string(&token).unwrap();
        assert!(yaml.contains("0123456789abcdef"), "{yaml}");
        assert_eq!(serde_norway::from_str::<Token>(&yaml).unwrap(), token);
    }

    #[test]
    fn deserializing_a_bad_token_fails_with_the_reason_but_not_the_value() {
        let err = serde_norway::from_str::<Token>("\"short\"").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("at least 32"), "{message}");
        assert!(!message.contains("short"), "{message}");
    }
}
