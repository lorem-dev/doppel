//! Probabilities, parsed rather than validated.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A probability: a finite number from 0.0 to 1.0 inclusive.
///
/// Doppel's fault-injection fields are fractions rather than percentages
/// despite being spelled `percentage`, which is the older name kept for
/// compatibility. The type is what makes the distinction unmissable: `50`
/// does not parse, and the message says the range.
///
/// `PartialEq` but not `Eq`, and `PartialOrd` but not `Ord`, because the
/// inner value is an `f64`. NaN cannot get in -- `parse` refuses it -- but
/// deriving the total versions would be claiming something about the type
/// that the standard library's own bounds do not support.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Ratio(f64);

/// Why a ratio was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RatioError {
    /// Kept apart from `OutOfRange` because the fix is different: a reader
    /// who wrote `50` meant fifty percent and needs to write `0.5`, while a
    /// reader who produced a NaN has a generator problem.
    #[error("`{0}` is not between 0.0 and 1.0; this is a fraction, so 50% is `0.5`, not `50`")]
    OutOfRange(f64),
    #[error("a probability must be a finite number, not `{0}`")]
    NotFinite(String),
}

impl Ratio {
    /// Never fails: the two constants are the only ones worth naming, and
    /// both are in range by construction.
    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);

    /// Check a number and keep it, or say why not.
    pub fn parse(value: f64) -> Result<Self, RatioError> {
        if !value.is_finite() {
            return Err(RatioError::NotFinite(value.to_string()));
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(RatioError::OutOfRange(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Ratio> for f64 {
    fn from(ratio: Ratio) -> Self {
        ratio.get()
    }
}

impl TryFrom<f64> for Ratio {
    type Error = RatioError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for Ratio {
    type Err = RatioError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let number: f64 = trimmed
            .parse()
            .map_err(|_| RatioError::NotFinite(trimmed.to_owned()))?;
        Self::parse(number)
    }
}

impl Serialize for Ratio {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Ratio {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(d)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Ratio {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Number)
            .minimum(Some(0.0))
            .maximum(Some(1.0))
            .description(Some("A probability from 0.0 to 1.0. 50% is `0.5`."))
            .examples([serde_json::json!(0.25)])
            .into()
    }
}

impl utoipa::ToSchema for Ratio {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_is_inclusive_at_both_ends() {
        assert_eq!(Ratio::parse(0.0).unwrap().get(), 0.0);
        assert_eq!(Ratio::parse(1.0).unwrap().get(), 1.0);
        assert_eq!(Ratio::parse(0.5).unwrap().get(), 0.5);
        assert_eq!(Ratio::ZERO.get(), 0.0);
        assert_eq!(Ratio::ONE.get(), 1.0);
    }

    #[test]
    fn a_percentage_written_as_a_percentage_is_told_what_to_write() {
        // The mistake the type exists for: `percentage: 50` used to validate
        // as out of range with no hint that the field wanted a fraction.
        let err = Ratio::parse(50.0).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("50% is `0.5`"), "{message}");
    }

    #[test]
    fn out_of_range_in_either_direction_is_refused() {
        assert!(matches!(Ratio::parse(-0.1), Err(RatioError::OutOfRange(_))));
        assert!(matches!(Ratio::parse(1.1), Err(RatioError::OutOfRange(_))));
    }

    #[test]
    fn nan_and_infinity_are_refused_before_the_range_is_consulted() {
        // `(0.0..=1.0).contains(&f64::NAN)` is false, so a range check alone
        // would refuse NaN with "not between 0.0 and 1.0" -- true, useless,
        // and it would let the reader think they wrote a number.
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(Ratio::parse(value), Err(RatioError::NotFinite(_))),
                "{value} should be refused as not finite"
            );
        }
    }

    #[test]
    fn a_ratio_round_trips_as_a_number() {
        let ratio = Ratio::parse(0.25).unwrap();
        let yaml = serde_norway::to_string(&ratio).unwrap();
        assert_eq!(yaml.trim(), "0.25");
        assert_eq!(serde_norway::from_str::<Ratio>(&yaml).unwrap(), ratio);
    }

    #[test]
    fn deserializing_out_of_range_carries_the_hint() {
        let err = serde_norway::from_str::<Ratio>("50").unwrap_err();
        assert!(err.to_string().contains("0.5"), "{err}");
    }

    #[test]
    fn parsing_from_a_string_covers_the_same_ground() {
        assert_eq!("0.5".parse::<Ratio>().unwrap().get(), 0.5);
        assert!(matches!(
            "2".parse::<Ratio>(),
            Err(RatioError::OutOfRange(_))
        ));
        assert!(matches!(
            "half".parse::<Ratio>(),
            Err(RatioError::NotFinite(_))
        ));
    }
}
