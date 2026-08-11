//! Admin server settings: tokens, access control lists, upload limits.

use std::net::IpAddr;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
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
    /// An IP address, not a hostname: a name would have to be resolved,
    /// and which address it resolves to is not the configuration's to
    /// decide. `utoipa` has no schema for `IpAddr`, so it is described
    /// here as the string it is written as.
    #[schema(value_type = String, examples("127.0.0.1"))]
    pub host: IpAddr,
    /// The TCP port the admin API listens on. Must differ from
    /// `server.port`.
    pub port: super::Port,
    /// Which header carries the bearer token.
    #[serde(default)]
    pub auth: AuthConfig,
    /// The tokens that may call the admin API. Names and token values are
    /// each unique. `DOPPEL_ADMIN_TOKENS` can supply these instead.
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    /// Who may perform each admin action. Every action defaults to the
    /// `admin` group, reads included.
    #[serde(default)]
    pub access: AccessConfig,
    /// Bounds an uploaded template file.
    pub upload: UploadConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// The header a caller presents its token in, as `Bearer <token>`.
    /// Defaults to `X-Proxy-Authorization`.
    #[serde(default = "default_auth_header")]
    pub header: super::HeaderName,
}

fn default_auth_header() -> super::HeaderName {
    super::HeaderName::parse("X-Proxy-Authorization").expect("a literal token")
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            header: default_auth_header(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenConfig {
    /// What to call this token in `access` lists and in logs. Never the
    /// secret itself.
    pub name: super::Name,
    /// The group it belongs to. `admin` and `user` are predefined; any other
    /// name must be carried by at least one token.
    pub group: super::Name,
    /// The secret the caller sends. A version 4 UUID is the recommended
    /// shape.
    pub token: super::Token,
}

/// Who may perform an action. `public` means no token is required.
///
/// The names are `Name`s: every entry has to be a token name or a group name,
/// and those are `Name`s wherever they are defined. Holding them as `String`
/// here would have let an access list reference something no token could ever
/// be called, and rule V27 would report it as unknown rather than as
/// unwritable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Subjects {
    #[default]
    Public,
    Names(Vec<super::Name>),
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
                    return Ok(Subjects::Public);
                }
                // `public` is checked first and never parsed: it is a keyword
                // in this position, not a name, even though it happens to be
                // spelled like a legal one.
                let name = super::Name::parse(value).map_err(E::custom)?;
                Ok(Subjects::Names(vec![name]))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Subjects, A::Error> {
                let mut names = Vec::new();
                while let Some(raw) = seq.next_element::<String>()? {
                    if raw == "public" {
                        return Ok(Subjects::Public);
                    }
                    names.push(super::Name::parse(raw).map_err(de::Error::custom)?);
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessConfig {
    /// List the proxies. A listing exposes upstream URLs and injected headers.
    #[serde(default = "admin_only")]
    pub list: Subjects,
    /// Read one proxy document, credentials in its `url` included.
    #[serde(default = "admin_only")]
    pub read: Subjects,
    /// Add a proxy. Refused for `public` by rule V34.
    #[serde(default = "admin_only")]
    pub create: Subjects,
    /// Replace a proxy. Refused for `public` by rule V34.
    #[serde(default = "admin_only")]
    pub update: Subjects,
    /// Remove a proxy. Refused for `public` by rule V34.
    #[serde(default = "admin_only")]
    pub delete: Subjects,
    /// Upload or delete a template file. Refused for `public` by rule V34.
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
    Subjects::Names(vec![super::Name::parse("admin").expect("a literal name")])
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
    /// Who may read this proxy's document. Absent leaves the global rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Subjects>,
    /// Who may replace this proxy. Absent leaves the global rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Subjects>,
    /// Who may remove this proxy. Absent leaves the global rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<Subjects>,
    /// Who may upload templates for this proxy. Absent leaves the global rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<Subjects>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadConfig {
    /// Largest template file the admin API accepts. A larger upload is
    /// refused with `413`.
    pub limit: super::ByteSize,
}

/// `Subjects` carries a hand-written schema because it has hand-written
/// `Serialize`/`Deserialize` impls: a derived schema would describe the Rust
/// shape rather than the wire shape, which is the one thing a generated
/// document exists to get right.
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
}
