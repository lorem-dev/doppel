//! Admin server settings: tokens, access control lists, upload limits.

use std::net::IpAddr;
use std::str::FromStr;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    pub host: IpAddr,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workers: Option<usize>,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    #[serde(default)]
    pub access: AccessConfig,
    pub upload: UploadConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default = "default_auth_header")]
    pub header: String,
}

fn default_auth_header() -> String {
    "X-Proxy-Authorization".to_owned()
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            header: default_auth_header(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenConfig {
    pub name: String,
    pub group: String,
    pub token: String,
}

/// Who may perform an action. `public` means no token is required.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Subjects {
    #[default]
    Public,
    Names(Vec<String>),
}

impl Serialize for Subjects {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Public => s.serialize_str("public"),
            Self::Names(names) => s.collect_seq(names),
        }
    }
}

impl<'de> Deserialize<'de> for Subjects {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = Subjects;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"public\", a name, or a list of names")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Subjects, E> {
                if value == "public" {
                    Ok(Subjects::Public)
                } else {
                    Ok(Subjects::Names(vec![value.to_owned()]))
                }
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Subjects, A::Error> {
                let mut names = Vec::new();
                while let Some(name) = seq.next_element::<String>()? {
                    if name == "public" {
                        return Ok(Subjects::Public);
                    }
                    names.push(name);
                }
                // An empty list means public, per the config comments.
                if names.is_empty() {
                    Ok(Subjects::Public)
                } else {
                    Ok(Subjects::Names(names))
                }
            }
        }

        d.deserialize_any(V)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessConfig {
    #[serde(default)]
    pub list: Subjects,
    #[serde(default)]
    pub read: Subjects,
    #[serde(default)]
    pub create: Subjects,
    #[serde(default)]
    pub update: Subjects,
    #[serde(default)]
    pub delete: Subjects,
    #[serde(default)]
    pub upload: Subjects,
}

/// Per-proxy override. Only these four actions may be overridden (rule V28 is
/// expressed in the type, so a config overriding `create` fails at parse time).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyAccessConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Subjects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Subjects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<Subjects>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<Subjects>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadConfig {
    pub limit: ByteSize,
}

/// A byte count written as `4096`, `512K`, `1M` or `2G`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSize(pub u64);

impl FromStr for ByteSize {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("byte size is empty".to_owned());
        }
        let upper = trimmed.to_ascii_uppercase();
        let digits_end = upper
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(upper.len());
        let (digits, suffix) = upper.split_at(digits_end);
        if digits.is_empty() {
            return Err(format!("`{trimmed}` does not start with a number"));
        }
        let value: u64 = digits
            .parse()
            .map_err(|_| format!("`{digits}` is not a number"))?;
        let multiplier = match suffix.trim_end_matches('B') {
            "" => 1_u64,
            "K" => 1024,
            "M" => 1024 * 1024,
            "G" => 1024 * 1024 * 1024,
            other => return Err(format!("unknown size suffix `{other}`")),
        };
        value
            .checked_mul(multiplier)
            .map(ByteSize)
            .ok_or_else(|| format!("`{trimmed}` overflows a byte count"))
    }
}

impl Serialize for ByteSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;

        impl<'de> Visitor<'de> for V {
            type Value = ByteSize;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a byte count such as 4096 or 1M")
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<ByteSize, E> {
                Ok(ByteSize(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<ByteSize, E> {
                if value < 0 {
                    return Err(E::custom(format!("byte size cannot be negative: {value}")));
                }
                Ok(ByteSize(value as u64))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<ByteSize, E> {
                value.parse().map_err(E::custom)
            }
        }

        d.deserialize_any(V)
    }
}
