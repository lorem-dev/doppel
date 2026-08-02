//! Admin server settings: tokens, access control lists, upload limits.

use std::net::IpAddr;
use std::str::FromStr;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    /// Whether to run the admin listener at all.
    ///
    /// Defaults to on. Off means the port is never bound and no admin task
    /// starts; the proxy and the control socket are untouched, so
    /// `doppel config reload` still works and is then the only way in.
    ///
    /// The validation rules do not consult this. A configuration that is only
    /// safe because nothing serves it is a trap set for whoever turns the
    /// listener on later, and they will not re-read the rules first.
    #[serde(default = "enabled")]
    pub enable: bool,
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
    pub name: super::Name,
    pub group: super::Name,
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

/// The admin listener runs unless a configuration says otherwise.
fn enabled() -> bool {
    true
}

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
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
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

/// A byte count written as `4096`, `512Ki`, `1Mi` or `2Gi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSize(pub u64);

impl FromStr for ByteSize {
    type Err = String;

    /// `4096`, `512Ki`, `1Mi`, `2Gi` for binary units; `1MB`, `2GB` for
    /// decimal ones.
    ///
    /// A bare `K`, `M` or `G` is refused rather than guessed at -- see the
    /// match arm that handles it.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("byte size is empty".to_owned());
        }

        let digits_end = trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len());
        let (digits, suffix) = trimmed.split_at(digits_end);
        if digits.is_empty() {
            return Err(format!("`{trimmed}` does not start with a number"));
        }
        let value: u64 = digits
            .parse()
            .map_err(|_| format!("`{digits}` is not a number"))?;

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
                return Err(format!(
                    "`{trimmed}` is ambiguous: write `{value}{binary}` for the binary unit \
                     or `{value}{decimal}` for the decimal one"
                ));
            }
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
                f.write_str("a byte count such as 4096 or 1Mi")
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
    fn binary_and_decimal_size_suffixes_mean_different_things() {
        // The distinction the ambiguity was hiding: `1Mi` and `1MB` are not
        // the same number, and a configuration that meant one while getting
        // the other is off by five percent with nothing to show for it.
        assert_eq!("1Mi".parse::<ByteSize>().unwrap().0, 1_048_576);
        assert_eq!("1MB".parse::<ByteSize>().unwrap().0, 1_000_000);
        assert_eq!("512Ki".parse::<ByteSize>().unwrap().0, 524_288);
        assert_eq!("512kB".parse::<ByteSize>().unwrap().0, 512_000);
        assert_eq!("2Gi".parse::<ByteSize>().unwrap().0, 2 * 1024 * 1024 * 1024);
        assert_eq!("2GB".parse::<ByteSize>().unwrap().0, 2_000_000_000);
        // `MiB` is the same as `Mi`, spelled in full.
        assert_eq!("1MiB".parse::<ByteSize>().unwrap().0, 1_048_576);
        // Case is not meaningful on input; the documentation gives one
        // spelling and this accepts what people type.
        assert_eq!("1mib".parse::<ByteSize>().unwrap().0, 1_048_576);
        // A plain count, with or without the unit letter.
        assert_eq!("4096".parse::<ByteSize>().unwrap().0, 4096);
        assert_eq!("4096B".parse::<ByteSize>().unwrap().0, 4096);
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
            let err = input.parse::<ByteSize>().unwrap_err();
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
}

/// `Subjects` and `ByteSize` carry hand-written schemas because both have
/// hand-written `Serialize`/`Deserialize` impls: a derived schema would
/// describe the Rust shape rather than the wire shape, which is the one thing
/// a generated document exists to get right.
mod schemas {
    use utoipa::PartialSchema;
    use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, OneOfBuilder, Type};
    use utoipa::openapi::{RefOr, Schema};

    impl PartialSchema for super::Subjects {
        fn schema() -> RefOr<Schema> {
            OneOfBuilder::new()
                .description(Some(
                    "Who may perform an action: `public`, one token or group \
                     name, or a list of them. An empty list means public.",
                ))
                .item(
                    ObjectBuilder::new()
                        .schema_type(Type::String)
                        .examples([serde_json::json!("public")]),
                )
                .item(
                    ArrayBuilder::new()
                        .items(ObjectBuilder::new().schema_type(Type::String))
                        .examples([serde_json::json!(["admin", "user1"])]),
                )
                .into()
        }
    }

    impl utoipa::ToSchema for super::Subjects {}

    impl PartialSchema for super::ByteSize {
        fn schema() -> RefOr<Schema> {
            OneOfBuilder::new()
                .description(Some(
                    "A byte count, as a plain integer or with a `K`, `M` or \
                     `G` suffix. Always serialized back as an integer.",
                ))
                .item(
                    ObjectBuilder::new()
                        .schema_type(Type::Integer)
                        .minimum(Some(0.0))
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

    impl utoipa::ToSchema for super::ByteSize {}
}
