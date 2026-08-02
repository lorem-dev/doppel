//! TCP ports, parsed rather than validated.

use std::fmt;
use std::num::NonZeroU16;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The highest port that needs privilege to bind on every platform Doppel
/// targets.
const HIGHEST_PRIVILEGED: u16 = 1023;

/// A port Doppel can bind or connect to: 1 to 65535.
///
/// Zero is the only value excluded, and it is excluded because of what it
/// means rather than because it is out of range: `bind` treats port 0 as "give
/// me any free port", so a configuration saying `port: 0` describes a server
/// whose address nobody -- including the operator who wrote it -- can predict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Port(NonZeroU16);

/// Why a port was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    /// Distinguished from `OutOfRange` because 0 is a perfectly good number
    /// and the operator who wrote it was not asking for a random port.
    #[error(
        "port 0 means `any free port` to the operating system, which is not \
         something a configuration can name; write the port you want"
    )]
    Zero,
    #[error("`{0}` is not a port: a port is a number from 1 to 65535")]
    OutOfRange(i64),
    /// Only reachable through `FromStr`; a YAML document that says
    /// `port: http` fails on the type before this gets a chance.
    #[error("`{0}` is not a number: a port is a number from 1 to 65535")]
    NotANumber(String),
}

impl Port {
    /// Check a number and keep it, or say why not.
    pub fn parse(value: u16) -> Result<Self, PortError> {
        NonZeroU16::new(value).map(Self).ok_or(PortError::Zero)
    }

    #[must_use]
    pub fn get(self) -> u16 {
        self.0.get()
    }

    /// Whether binding this port needs elevated privilege on a typical
    /// Unix system.
    ///
    /// Advisory only. Nothing in Doppel refuses a privileged port -- running
    /// behind a capability or a redirect is a legitimate deployment, and a
    /// configuration that works must not be rejected for looking unusual. It
    /// is worth a line at startup because the far more common cause is a
    /// typo, and the failure it produces otherwise is a bare
    /// `Permission denied` from `bind`.
    #[must_use]
    pub fn is_privileged(self) -> bool {
        self.0.get() <= HIGHEST_PRIVILEGED
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Port> for u16 {
    fn from(port: Port) -> Self {
        port.get()
    }
}

impl TryFrom<u16> for Port {
    type Error = PortError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for Port {
    type Err = PortError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        let number: i64 = trimmed
            .parse()
            .map_err(|_| PortError::NotANumber(trimmed.to_owned()))?;
        let narrowed = u16::try_from(number).map_err(|_| PortError::OutOfRange(number))?;
        Self::parse(narrowed)
    }
}

impl Serialize for Port {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(self.0.get())
    }
}

impl<'de> Deserialize<'de> for Port {
    /// Read as `i64` rather than `u16` so that an out-of-range number gets a
    /// message naming a port, instead of serde's generic "invalid value:
    /// integer `70000`, expected u16" -- which tells a reader the Rust type
    /// and not the rule.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = i64::deserialize(d)?;
        let narrowed = u16::try_from(value)
            .map_err(|_| serde::de::Error::custom(PortError::OutOfRange(value)))?;
        Self::parse(narrowed).map_err(serde::de::Error::custom)
    }
}

impl utoipa::PartialSchema for Port {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::Integer)
            .minimum(Some(1.0))
            .maximum(Some(f64::from(u16::MAX)))
            .description(Some("A TCP port, 1 to 65535."))
            .into()
    }
}

impl utoipa::ToSchema for Port {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ports_a_configuration_names_are_accepted() {
        for value in [1u16, 80, 1023, 1024, 8080, 65535] {
            assert_eq!(Port::parse(value).unwrap().get(), value);
        }
    }

    #[test]
    fn zero_is_refused_with_the_reason_it_is_refused() {
        // Not "out of range": 0 is a perfectly good `u16`. The message has to
        // explain what it would do, because an operator who wrote it was not
        // asking for a random port.
        let err = Port::parse(0).unwrap_err();
        assert_eq!(err, PortError::Zero);
        let message = err.to_string();
        assert!(message.contains("any free port"), "{message}");
    }

    #[test]
    fn a_number_above_the_range_names_a_port_not_a_rust_type() {
        // The whole reason `Deserialize` goes through `i64`: serde's own
        // message for a too-large `u16` says "expected u16", which describes
        // the implementation rather than the rule.
        let err = serde_norway::from_str::<Port>("70000").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("1 to 65535"), "{message}");
        assert!(!message.contains("u16"), "{message}");

        let negative = serde_norway::from_str::<Port>("-1").unwrap_err();
        assert!(negative.to_string().contains("1 to 65535"), "{negative}");
    }

    #[test]
    fn zero_is_refused_through_yaml_too() {
        let err = serde_norway::from_str::<Port>("0").unwrap_err();
        assert!(err.to_string().contains("any free port"), "{err}");
    }

    #[test]
    fn privileged_is_everything_up_to_and_including_1023() {
        assert!(Port::parse(1).unwrap().is_privileged());
        assert!(Port::parse(80).unwrap().is_privileged());
        assert!(Port::parse(1023).unwrap().is_privileged());
        assert!(!Port::parse(1024).unwrap().is_privileged());
        assert!(!Port::parse(8080).unwrap().is_privileged());
    }

    #[test]
    fn a_port_round_trips_as_a_number() {
        // Not as a string: a configuration written by `config pull` has to be
        // readable by anything that reads YAML, and quoting a number where
        // the schema says integer is the kind of difference that breaks a
        // downstream tool for no reason.
        let port = Port::parse(8080).unwrap();
        let yaml = serde_norway::to_string(&port).unwrap();
        assert_eq!(yaml.trim(), "8080");
        assert_eq!(serde_norway::from_str::<Port>(&yaml).unwrap(), port);
    }

    #[test]
    fn parsing_from_a_string_covers_the_same_ground() {
        assert_eq!("8080".parse::<Port>().unwrap().get(), 8080);
        assert_eq!(" 8080 ".parse::<Port>().unwrap().get(), 8080);
        assert_eq!("0".parse::<Port>(), Err(PortError::Zero));
        assert!(matches!(
            "70000".parse::<Port>(),
            Err(PortError::OutOfRange(70000))
        ));
        assert!(matches!(
            "http".parse::<Port>(),
            Err(PortError::NotANumber(value)) if value == "http"
        ));
    }
}
