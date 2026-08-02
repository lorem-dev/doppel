//! HTTP status codes, parsed rather than validated.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The lowest status code HTTP defines.
const MIN: u16 = 100;

/// The highest. Not 999: RFC 9110 fixes the three-digit range at 1xx to 5xx,
/// and a status of 700 is a typo every time.
const MAX: u16 = 599;

/// Statuses that must not carry a body, per RFC 9110.
const BODILESS: [u16; 2] = [204, 304];

/// A status code a mock or a fault injector may return: 100 to 599.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HttpStatus(u16);

/// Why a status was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatusError {
    #[error("`{0}` is not an HTTP status: a status is a number from {MIN} to {MAX}")]
    OutOfRange(i64),
    /// Only reachable through `FromStr`; a YAML document that says
    /// `status: ok` fails on the type before this gets a chance.
    #[error("`{0}` is not a number: a status is a number from {MIN} to {MAX}")]
    NotANumber(String),
}

impl HttpStatus {
    /// Check a number and keep it, or say why not.
    pub fn parse(value: u16) -> Result<Self, StatusError> {
        if (MIN..=MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(StatusError::OutOfRange(i64::from(value)))
        }
    }

    #[must_use]
    pub fn get(self) -> u16 {
        self.0
    }

    /// Whether RFC 9110 forbids a body with this status.
    ///
    /// On the type rather than in the validation rule that uses it (V30),
    /// because it is a fact about the status and not about a configuration.
    /// The rule stays a rule: it compares the status against whether a body
    /// was declared, which is two fields.
    #[must_use]
    pub fn forbids_a_body(self) -> bool {
        BODILESS.contains(&self.0)
    }
}

impl fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<HttpStatus> for u16 {
    fn from(status: HttpStatus) -> Self {
        status.get()
    }
}

impl TryFrom<u16> for HttpStatus {
    type Error = StatusError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for HttpStatus {
    type Err = StatusError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let number: i64 = trimmed
            .parse()
            .map_err(|_| StatusError::NotANumber(trimmed.to_owned()))?;
        let narrowed = u16::try_from(number).map_err(|_| StatusError::OutOfRange(number))?;
        Self::parse(narrowed)
    }
}

impl Serialize for HttpStatus {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for HttpStatus {
    /// Read as `i64` before narrowing, so `status: 70000` is refused with a
    /// message about statuses rather than serde's "expected u16".
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = i64::deserialize(d)?;
        let narrowed = u16::try_from(value)
            .map_err(|_| serde::de::Error::custom(StatusError::OutOfRange(value)))?;
        Self::parse(narrowed).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for HttpStatus {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(f64::from(MIN)))
            .maximum(Some(f64::from(MAX)))
            .description(Some("An HTTP status code, 100 to 599."))
            .into()
    }
}

impl utoipa::ToSchema for HttpStatus {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_is_inclusive_at_both_ends() {
        assert_eq!(HttpStatus::parse(100).unwrap().get(), 100);
        assert_eq!(HttpStatus::parse(599).unwrap().get(), 599);
        assert!(HttpStatus::parse(99).is_err());
        assert!(HttpStatus::parse(600).is_err());
    }

    #[test]
    fn the_statuses_a_mock_actually_returns_are_accepted() {
        for value in [
            200u16, 201, 204, 301, 304, 400, 401, 403, 404, 429, 500, 503,
        ] {
            assert!(HttpStatus::parse(value).is_ok(), "{value} should be legal");
        }
    }

    #[test]
    fn an_impossible_status_names_a_status_not_a_rust_type() {
        let err = serde_norway::from_str::<HttpStatus>("700").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("100 to 599"), "{message}");

        let huge = serde_norway::from_str::<HttpStatus>("70000").unwrap_err();
        assert!(huge.to_string().contains("100 to 599"), "{huge}");
        assert!(!huge.to_string().contains("u16"), "{huge}");

        let negative = serde_norway::from_str::<HttpStatus>("-1").unwrap_err();
        assert!(negative.to_string().contains("100 to 599"), "{negative}");
    }

    #[test]
    fn exactly_two_statuses_forbid_a_body() {
        assert!(HttpStatus::parse(204).unwrap().forbids_a_body());
        assert!(HttpStatus::parse(304).unwrap().forbids_a_body());
        for value in [200u16, 201, 203, 205, 303, 305, 404, 500] {
            assert!(
                !HttpStatus::parse(value).unwrap().forbids_a_body(),
                "{value} does not forbid a body"
            );
        }
    }

    #[test]
    fn a_status_round_trips_as_a_number() {
        let status = HttpStatus::parse(404).unwrap();
        let yaml = serde_norway::to_string(&status).unwrap();
        assert_eq!(yaml.trim(), "404");
        assert_eq!(serde_norway::from_str::<HttpStatus>(&yaml).unwrap(), status);
    }

    #[test]
    fn parsing_from_a_string_covers_the_same_ground() {
        assert_eq!("404".parse::<HttpStatus>().unwrap().get(), 404);
        assert!(matches!(
            "700".parse::<HttpStatus>(),
            Err(StatusError::OutOfRange(700))
        ));
        assert!(matches!(
            "ok".parse::<HttpStatus>(),
            Err(StatusError::NotANumber(value)) if value == "ok"
        ));
    }
}
