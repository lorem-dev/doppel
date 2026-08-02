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
        // `Names(vec![])` and a `Names` list containing the literal string
        // `"public"` both parse back as `Public` (see `Deserialize` below),
        // so they must also *serialize* as `"public"` -- otherwise
        // `to_yaml(parse(to_yaml(x)))` would differ from `to_yaml(x)` for
        // those two shapes, and two `Config` values that mean the same
        // thing (one built in memory, one round-tripped through YAML) would
        // disagree on `Revision::of_config`.
        match self {
            Self::Public => s.serialize_str("public"),
            Self::Names(names) if names.is_empty() || names.iter().any(|name| name == "public") => {
                s.serialize_str("public")
            }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessConfig {
    #[serde(default = "admin_only")]
    pub list: Subjects,
    #[serde(default = "admin_only")]
    pub read: Subjects,
    #[serde(default = "admin_only")]
    pub create: Subjects,
    #[serde(default = "admin_only")]
    pub update: Subjects,
    #[serde(default = "admin_only")]
    pub delete: Subjects,
    #[serde(default = "admin_only")]
    pub upload: Subjects,
}

/// The default for every action that changes something.
///
/// Reads default to public: a proxy listing and `/status` give nothing away.
/// Writes do not, because the alternative is that omitting an `access` block
/// hands any unauthenticated caller the ability to rewrite the proxy set --
/// and the most common configuration is the one nobody wrote. Rule V34 then
/// refuses an *explicit* public write, so the safe state cannot be reached by
/// accident in either direction.
/// The default for every action, reads included.
///
/// Reads were public here at first, on the reasoning that only writes are
/// dangerous. That reasoning was wrong: a proxy document carries the
/// `headers` this proxy injects upstream -- an `Authorization` among them in
/// the project's own reference configuration -- and a `url` that may itself
/// contain `user:password@`. Listing proxies therefore publishes upstream
/// credentials, which is not a lesser harm than rewriting the proxy set.
///
/// An operator whose configuration holds no secrets can still say
/// `read: public` deliberately. What must not happen is that leaving the
/// section out does it for them.
fn admin_only() -> Subjects {
    Subjects::Names(vec!["admin".to_owned()])
}

impl Default for AccessConfig {
    /// Kept in step with the `serde` defaults above by construction: an empty
    /// `access:` block and `AccessConfig::default()` must describe the same
    /// permissions, or the two ways of getting one would disagree.
    fn default() -> Self {
        Self {
            list: admin_only(),
            read: admin_only(),
            create: admin_only(),
            update: admin_only(),
            delete: admin_only(),
            upload: admin_only(),
        }
    }
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
        // Strip at most one trailing `B` (`1KB` means the same as `1K`); a
        // second one is not an alternate spelling but a mistake, so `5BB`
        // must be rejected rather than silently parsed as 5 bytes.
        let unit = suffix.strip_suffix('B').unwrap_or(suffix);
        let multiplier = match unit {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_number_is_bytes() {
        assert_eq!("4096".parse::<ByteSize>().unwrap().0, 4096);
    }

    #[test]
    fn each_suffix_applies_its_multiplier() {
        assert_eq!("512K".parse::<ByteSize>().unwrap().0, 512 * 1024);
        assert_eq!("1M".parse::<ByteSize>().unwrap().0, 1024 * 1024);
        assert_eq!("2G".parse::<ByteSize>().unwrap().0, 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn a_single_trailing_b_is_the_same_unit() {
        assert_eq!(
            "1KB".parse::<ByteSize>().unwrap().0,
            "1K".parse::<ByteSize>().unwrap().0
        );
        assert_eq!("5B".parse::<ByteSize>().unwrap().0, 5);
    }

    #[test]
    fn a_doubled_trailing_b_is_rejected_rather_than_silently_stripped() {
        // Without the fix, `trim_end_matches('B')` stripped every trailing
        // `B`, so `5BB` parsed as 5 bytes instead of being rejected.
        let err = "5BB".parse::<ByteSize>().unwrap_err();
        assert!(err.contains("unknown size suffix"), "got `{err}`");

        let err = "10KBB".parse::<ByteSize>().unwrap_err();
        assert!(err.contains("unknown size suffix"), "got `{err}`");
    }

    #[test]
    fn an_unknown_suffix_is_rejected() {
        let err = "5T".parse::<ByteSize>().unwrap_err();
        assert!(err.contains("unknown size suffix"), "got `{err}`");
    }
}
