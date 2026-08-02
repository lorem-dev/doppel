//! Byte counts, parsed rather than validated.
//!
//! Its own module rather than a corner of `admin`: two unrelated settings use
//! it -- the admin upload limit and a proxy's request-body limit -- and the
//! unit spellings below are the fiddliest thing in the configuration format.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The largest limit accepted: one gibibyte.
///
/// Both fields this type serves bound something Doppel buffers in memory per
/// request or per upload. A number past this is not a larger limit, it is the
/// absence of one, written in a way that looks deliberate. A configuration
/// that genuinely wants to stream something bigger needs a feature Doppel
/// does not have, not a bigger number here.
const MAX: u64 = 1024 * 1024 * 1024;

/// A byte count written as `4096`, `512Ki`, `1Mi` or `2Gi`.
///
/// At least one byte, at most one gibibyte. Zero is refused because both
/// fields using it are limits, and a limit of zero rejects everything -- which
/// surfaces as a confusing 413 on every request rather than as the
/// configuration mistake it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteSize(u64);

/// Why a byte size was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ByteSizeError {
    #[error("a size limit of 0 would reject everything; write the limit you want")]
    Zero,
    #[error("`{value}` is larger than the {MAX} byte maximum (1 GiB)")]
    TooLarge { value: u64 },
    #[error("byte size is empty")]
    Empty,
    #[error("`{0}` does not start with a number")]
    NoNumber(String),
    /// The one unit spelling that is refused rather than guessed at. See
    /// `FromStr`.
    #[error(
        "`{written}` is ambiguous: write `{value}{binary}` for the binary unit \
         or `{value}{decimal}` for the decimal one"
    )]
    AmbiguousUnit {
        written: String,
        value: u64,
        binary: &'static str,
        decimal: &'static str,
    },
    #[error("unknown size suffix `{0}`")]
    UnknownSuffix(String),
    #[error("`{0}` overflows a byte count")]
    Overflow(String),
}

impl ByteSize {
    /// Check a number and keep it, or say why not.
    pub fn parse(value: u64) -> Result<Self, ByteSizeError> {
        if value == 0 {
            return Err(ByteSizeError::Zero);
        }
        if value > MAX {
            return Err(ByteSizeError::TooLarge { value });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ByteSize> for u64 {
    fn from(size: ByteSize) -> Self {
        size.get()
    }
}

impl TryFrom<u64> for ByteSize {
    type Error = ByteSizeError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl FromStr for ByteSize {
    type Err = ByteSizeError;

    /// `4096`, `512Ki`, `1Mi`, `2Gi` for binary units; `1kB`, `2MB` for
    /// decimal ones.
    ///
    /// A bare `K`, `M` or `G` is refused rather than guessed at -- see the
    /// match arm that handles it.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ByteSizeError::Empty);
        }

        let digits_end = trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len());
        let (digits, suffix) = trimmed.split_at(digits_end);
        if digits.is_empty() {
            return Err(ByteSizeError::NoNumber(trimmed.to_owned()));
        }
        let value: u64 = digits
            .parse()
            .map_err(|_| ByteSizeError::Overflow(trimmed.to_owned()))?;

        // Case-insensitive on input, because `MiB` and `mib` cannot mean two
        // different things, while the documentation gives one spelling.
        //
        // The whole suffix is matched at once rather than having a trailing
        // `B` stripped first: stripping would collapse `MB` into `M`, which is
        // exactly the pair that must stay distinguishable -- one is decimal
        // and the other is the ambiguous spelling being retired.
        let multiplier: u64 = match suffix.to_ascii_uppercase().as_str() {
            "" | "B" => 1,
            "KI" | "KIB" => 1024,
            "MI" | "MIB" => 1024 * 1024,
            "GI" | "GIB" => 1024 * 1024 * 1024,
            "KB" => 1000,
            "MB" => 1000 * 1000,
            "GB" => 1000 * 1000 * 1000,
            // Bare `K`, `M`, `G`. This spelling meant the binary unit in this
            // project's own past, which contradicts SI, and silently
            // reinterpreting it as decimal would change the size of every
            // buffer already configured without a word. Refusing makes it fail
            // loudly, once, with both replacements named.
            unit @ ("K" | "M" | "G") => {
                let (binary, decimal) = match unit {
                    "K" => ("Ki", "kB"),
                    "M" => ("Mi", "MB"),
                    _ => ("Gi", "GB"),
                };
                return Err(ByteSizeError::AmbiguousUnit {
                    written: trimmed.to_owned(),
                    value,
                    binary,
                    decimal,
                });
            }
            other => return Err(ByteSizeError::UnknownSuffix(other.to_owned())),
        };

        let bytes = value
            .checked_mul(multiplier)
            .ok_or_else(|| ByteSizeError::Overflow(trimmed.to_owned()))?;
        Self::parse(bytes)
    }
}

impl Serialize for ByteSize {
    /// Always as a plain integer, whatever spelling it was written with.
    /// `config pull` produces the canonical form the revision is computed
    /// over, and two documents meaning the same size must not hash apart for
    /// having spelled it `1Mi` and `1048576`.
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = ByteSize;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a byte count such as 4096 or 1Mi")
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<ByteSize, E> {
                ByteSize::parse(value).map_err(E::custom)
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<ByteSize, E> {
                let unsigned = u64::try_from(value).map_err(|_| {
                    E::custom(format!("a size must not be negative, and `{value}` is"))
                })?;
                ByteSize::parse(unsigned).map_err(E::custom)
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<ByteSize, E> {
                value.parse().map_err(E::custom)
            }
        }

        d.deserialize_any(V)
    }
}

impl utoipa::PartialSchema for ByteSize {
    /// Hand-written because `Serialize`/`Deserialize` are: a derived schema
    /// would describe the Rust shape rather than the wire shape, which is the
    /// one thing a generated document exists to get right.
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{ObjectBuilder, OneOfBuilder, Type};

        OneOfBuilder::new()
            .description(Some(
                "A byte count from 1 to 1073741824, as a plain integer or with \
                 a binary (`Ki`, `Mi`, `Gi`) or decimal (`kB`, `MB`, `GB`) \
                 suffix. Always serialized back as an integer.",
            ))
            .item(
                ObjectBuilder::new()
                    .schema_type(Type::Integer)
                    .minimum(Some(1.0))
                    .maximum(Some(MAX as f64))
                    .examples([serde_json::json!(1_048_576u64)]),
            )
            .item(
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .examples([serde_json::json!("1Mi")]),
            )
            .into()
    }
}

impl utoipa::ToSchema for ByteSize {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_and_decimal_size_suffixes_mean_different_things() {
        // The distinction the ambiguity was hiding: `1Mi` and `1MB` are not
        // the same number, and a configuration that meant one while getting
        // the other is off by five percent with nothing to show for it.
        assert_eq!("1Mi".parse::<ByteSize>().unwrap().get(), 1_048_576);
        assert_eq!("1MB".parse::<ByteSize>().unwrap().get(), 1_000_000);
        assert_eq!("512Ki".parse::<ByteSize>().unwrap().get(), 524_288);
        assert_eq!("512kB".parse::<ByteSize>().unwrap().get(), 512_000);
        assert_eq!("1Gi".parse::<ByteSize>().unwrap().get(), 1024 * 1024 * 1024);
        assert_eq!("1GB".parse::<ByteSize>().unwrap().get(), 1_000_000_000);
        // `MiB` is the same as `Mi`, spelled in full.
        assert_eq!("1MiB".parse::<ByteSize>().unwrap().get(), 1_048_576);
        // Case is not meaningful on input; the documentation gives one
        // spelling and this accepts what people type.
        assert_eq!("1mib".parse::<ByteSize>().unwrap().get(), 1_048_576);
        // A plain count, with or without the unit letter.
        assert_eq!("4096".parse::<ByteSize>().unwrap().get(), 4096);
        assert_eq!("4096B".parse::<ByteSize>().unwrap().get(), 4096);
    }

    #[test]
    fn a_bare_k_m_or_g_is_refused_with_both_replacements_named() {
        // It meant the binary unit here once, which contradicts SI. Silently
        // reinterpreting it as decimal would shrink every configured buffer
        // without a word, so it fails instead -- and the message has to say
        // what to write, because the operator cannot guess which was meant.
        for (input, binary, decimal) in [
            ("1M", "1Mi", "1MB"),
            ("8K", "8Ki", "8kB"),
            ("3G", "3Gi", "3GB"),
        ] {
            let err = input.parse::<ByteSize>().unwrap_err().to_string();
            assert!(err.contains(binary), "{input}: {err}");
            assert!(err.contains(decimal), "{input}: {err}");
        }
    }

    #[test]
    fn nonsense_suffixes_are_still_refused() {
        for input in ["5BB", "1X", "banana", "", "Mi"] {
            assert!(
                input.parse::<ByteSize>().is_err(),
                "`{input}` must not parse"
            );
        }
    }

    #[test]
    fn zero_is_refused_because_both_fields_using_it_are_limits() {
        // This was rules V29 and V33, one per field, saying the same thing
        // twice about the same type.
        assert_eq!(ByteSize::parse(0), Err(ByteSizeError::Zero));
        assert_eq!("0".parse::<ByteSize>(), Err(ByteSizeError::Zero));
        assert_eq!("0Mi".parse::<ByteSize>(), Err(ByteSizeError::Zero));
        let err = serde_norway::from_str::<ByteSize>("0").unwrap_err();
        assert!(err.to_string().contains("would reject everything"), "{err}");
    }

    #[test]
    fn the_upper_bound_is_inclusive_and_catches_the_absence_of_a_limit() {
        assert!(ByteSize::parse(MAX).is_ok());
        assert!(matches!(
            ByteSize::parse(MAX + 1),
            Err(ByteSizeError::TooLarge { .. })
        ));
        assert!("1Gi".parse::<ByteSize>().is_ok());
        assert!("2Gi".parse::<ByteSize>().is_err());
    }

    #[test]
    fn an_overflowing_count_is_reported_as_overflow_not_as_too_large() {
        // `18446744073709551615Gi` overflows the multiplication before any
        // range check could see it, and reporting a bound it never reached
        // would be a message about the wrong thing.
        let err = "18446744073709551615Gi".parse::<ByteSize>().unwrap_err();
        assert!(matches!(err, ByteSizeError::Overflow(_)), "{err:?}");
    }

    #[test]
    fn a_negative_size_names_a_size_not_a_rust_type() {
        let err = serde_norway::from_str::<ByteSize>("-1").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("must not be negative"), "{message}");
        assert!(!message.contains("u64"), "{message}");
    }

    #[test]
    fn a_size_round_trips_as_an_integer_whatever_it_was_written_as() {
        let size: ByteSize = "1Mi".parse().unwrap();
        let yaml = serde_norway::to_string(&size).unwrap();
        assert_eq!(yaml.trim(), "1048576");
        assert_eq!(serde_norway::from_str::<ByteSize>(&yaml).unwrap(), size);
    }
}
