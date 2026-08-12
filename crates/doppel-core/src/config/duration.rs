//! Durations, parsed rather than validated.
//!
//! Two types, because the two things Doppel measures in seconds have
//! different shapes and different sensible bounds: an injected latency is a
//! fraction of a second and may be zero, an upstream timeout is a whole
//! number of seconds and may not.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The longest latency Doppel will inject, in seconds.
///
/// Five minutes. This is a sanity bound, not a protocol one: past it, every
/// client has already given up, so the request is not being delayed, it is
/// being made to fail in a way nothing in Doppel reports. A number bigger
/// than this is a unit mistake -- milliseconds written into a seconds field
/// -- far more often than it is an intent.
const MAX_LATENCY_SECONDS: f64 = 300.0;

/// The longest upstream timeout, in seconds. One hour, for the same reason.
const MAX_TIMEOUT_SECONDS: u64 = 3600;

/// A latency, in seconds: a finite number from 0.0 to 300.0.
///
/// Zero is allowed. `min: 0.0` is how a configuration says "between nothing
/// and 200ms", and refusing it would make the useful case awkward for the
/// sake of a value that means exactly what it says.
///
/// Not `Eq`/`Ord`: the inner value is an `f64`. NaN cannot get in, but
/// deriving the total versions would claim more than the standard library's
/// own bounds support.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Seconds(f64);

/// Why a latency was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SecondsError {
    #[error("a latency must not be negative, and `{0}` is")]
    Negative(f64),
    #[error(
        "`{0}` seconds is longer than the {MAX_LATENCY_SECONDS} second maximum; \
         if this was meant to be milliseconds, a latency is written in seconds"
    )]
    TooLong(f64),
    #[error("a latency must be a finite number of seconds, not `{0}`")]
    NotFinite(String),
}

impl Seconds {
    pub const ZERO: Self = Self(0.0);

    /// Check a number and keep it, or say why not.
    pub fn parse(value: f64) -> Result<Self, SecondsError> {
        if !value.is_finite() {
            return Err(SecondsError::NotFinite(value.to_string()));
        }
        if value < 0.0 {
            return Err(SecondsError::Negative(value));
        }
        if value > MAX_LATENCY_SECONDS {
            return Err(SecondsError::TooLong(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }

    /// The value as a `Duration`, for the code that actually sleeps.
    ///
    /// Total, unlike `Duration::try_from_secs_f64`: the constructor has
    /// already excluded every input that could fail -- negative, NaN,
    /// infinite, or larger than a `Duration` can hold.
    #[must_use]
    pub fn as_duration(self) -> Duration {
        Duration::from_secs_f64(self.0)
    }
}

impl fmt::Display for Seconds {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Seconds> for f64 {
    fn from(seconds: Seconds) -> Self {
        seconds.get()
    }
}

impl TryFrom<f64> for Seconds {
    type Error = SecondsError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for Seconds {
    type Err = SecondsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let number: f64 = trimmed
            .parse()
            .map_err(|_| SecondsError::NotFinite(trimmed.to_owned()))?;
        Self::parse(number)
    }
}

impl Serialize for Seconds {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Seconds {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Seconds {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Number)
            .minimum(Some(0.0))
            .maximum(Some(MAX_LATENCY_SECONDS))
            .description(Some("A latency in seconds, 0 to 300."))
            .examples([serde_json::json!(0.25)])
            .into()
    }
}

impl utoipa::ToSchema for Seconds {}

/// An upstream timeout, in whole seconds: 1 to 3600.
///
/// Zero is refused rather than read as "no timeout". A request with no
/// timeout at all holds a connection until the upstream decides otherwise,
/// which is the failure mode a proxy exists to bound; a configuration that
/// wants a long one can say so in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeoutSeconds(u64);

/// Why a timeout was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeoutError {
    #[error("a timeout of 0 would mean no timeout at all; leave `timeout` out to use the default")]
    Zero,
    #[error(
        "`{0}` is longer than the {MAX_TIMEOUT_SECONDS} second maximum; a \
         timeout is written in seconds, not milliseconds"
    )]
    TooLong(i64),
    #[error("`{0}` is not a whole number of seconds")]
    NotANumber(String),
}

impl TimeoutSeconds {
    /// Check a number and keep it, or say why not.
    pub fn parse(value: u64) -> Result<Self, TimeoutError> {
        if value == 0 {
            return Err(TimeoutError::Zero);
        }
        if value > MAX_TIMEOUT_SECONDS {
            return Err(TimeoutError::TooLong(
                i64::try_from(value).unwrap_or(i64::MAX),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub fn as_duration(self) -> Duration {
        Duration::from_secs(self.0)
    }
}

impl fmt::Display for TimeoutSeconds {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<TimeoutSeconds> for u64 {
    fn from(timeout: TimeoutSeconds) -> Self {
        timeout.get()
    }
}

impl TryFrom<u64> for TimeoutSeconds {
    type Error = TimeoutError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for TimeoutSeconds {
    type Err = TimeoutError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let number: i64 = trimmed
            .parse()
            .map_err(|_| TimeoutError::NotANumber(trimmed.to_owned()))?;
        let unsigned =
            u64::try_from(number).map_err(|_| TimeoutError::NotANumber(number.to_string()))?;
        Self::parse(unsigned)
    }
}

impl Serialize for TimeoutSeconds {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for TimeoutSeconds {
    /// Read as `i64` before narrowing, so a negative timeout is refused with
    /// a message about timeouts rather than serde's "expected u64".
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = i64::deserialize(d)?;
        let unsigned = u64::try_from(value).map_err(|_| {
            serde::de::Error::custom(format!("a timeout must not be negative, and `{value}` is"))
        })?;
        Self::parse(unsigned).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for TimeoutSeconds {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(1.0))
            .maximum(Some(MAX_TIMEOUT_SECONDS as f64))
            .description(Some("An upstream timeout in whole seconds, 1 to 3600."))
            .examples([serde_json::json!(30)])
            .into()
    }
}

impl utoipa::ToSchema for TimeoutSeconds {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_latency_may_be_zero_and_may_be_fractional() {
        // `min: 0.0` is how a configuration says "between nothing and 200ms".
        assert_eq!(Seconds::parse(0.0).unwrap().get(), 0.0);
        assert_eq!(Seconds::parse(0.2).unwrap().get(), 0.2);
        assert_eq!(Seconds::ZERO.get(), 0.0);
    }

    #[test]
    fn a_negative_latency_and_a_millisecond_one_are_told_apart() {
        assert!(matches!(
            Seconds::parse(-1.0),
            Err(SecondsError::Negative(_))
        ));
        // 500 in a seconds field is eight minutes, which no client waits for.
        let err = Seconds::parse(500.0).unwrap_err();
        assert!(matches!(err, SecondsError::TooLong(_)), "{err:?}");
        assert!(err.to_string().contains("milliseconds"), "{err}");
    }

    #[test]
    fn the_latency_bound_is_inclusive() {
        assert!(Seconds::parse(MAX_LATENCY_SECONDS).is_ok());
        assert!(Seconds::parse(MAX_LATENCY_SECONDS + 0.1).is_err());
    }

    #[test]
    fn a_latency_that_is_not_a_number_is_refused_before_the_range() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(Seconds::parse(value), Err(SecondsError::NotFinite(_))),
                "{value} should be refused as not finite"
            );
        }
    }

    #[test]
    fn a_latency_converts_to_a_duration_without_a_fallible_step() {
        // The constructor has already excluded everything
        // `Duration::from_secs_f64` panics on, which is why `as_duration`
        // can be total.
        assert_eq!(
            Seconds::parse(1.5).unwrap().as_duration(),
            Duration::from_millis(1500)
        );
        assert_eq!(Seconds::ZERO.as_duration(), Duration::ZERO);
    }

    #[test]
    fn a_timeout_of_zero_is_refused_with_what_to_do_instead() {
        let err = TimeoutSeconds::parse(0).unwrap_err();
        assert_eq!(err, TimeoutError::Zero);
        assert!(err.to_string().contains("leave `timeout` out"), "{err}");
    }

    #[test]
    fn the_timeout_bound_is_inclusive_and_names_the_unit() {
        assert!(TimeoutSeconds::parse(MAX_TIMEOUT_SECONDS).is_ok());
        let err = TimeoutSeconds::parse(MAX_TIMEOUT_SECONDS + 1).unwrap_err();
        assert!(
            err.to_string().contains("seconds, not milliseconds"),
            "{err}"
        );
        // The mistake it catches: 30 seconds written as 30000 milliseconds.
        assert!(TimeoutSeconds::parse(30_000).is_err());
        assert!(TimeoutSeconds::parse(30).is_ok());
    }

    #[test]
    fn a_negative_timeout_names_a_timeout_not_a_rust_type() {
        let err = serde_norway::from_str::<TimeoutSeconds>("-1").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("must not be negative"), "{message}");
        assert!(!message.contains("u64"), "{message}");
    }

    #[test]
    fn both_types_round_trip_as_numbers() {
        let latency = Seconds::parse(0.25).unwrap();
        let yaml = serde_norway::to_string(&latency).unwrap();
        assert_eq!(yaml.trim(), "0.25");
        assert_eq!(serde_norway::from_str::<Seconds>(&yaml).unwrap(), latency);

        let timeout = TimeoutSeconds::parse(30).unwrap();
        let yaml = serde_norway::to_string(&timeout).unwrap();
        assert_eq!(yaml.trim(), "30");
        assert_eq!(
            serde_norway::from_str::<TimeoutSeconds>(&yaml).unwrap(),
            timeout
        );
    }
}
