//! Per-proxy settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::admin::{ByteSize, ProxyAccessConfig};
use super::mock::MockConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Http,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResolveKind {
    Default,
    Header,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveConfig {
    #[serde(rename = "type", default = "default_resolve_kind")]
    pub kind: ResolveKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}

fn default_resolve_kind() -> ResolveKind {
    ResolveKind::Default
}

impl Default for ResolveConfig {
    fn default() -> Self {
        Self {
            kind: default_resolve_kind(),
            header: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LossConfig {
    pub percentage: f64,
    pub status: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencyConfig {
    pub percentage: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ProxyKind,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub resolve: ResolveConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<ProxyAccessConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<LossConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<LatencyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<f64>,
    /// Bounds the request body a matched mock is allowed to buffer in order
    /// to extract from it; phase 1 streams bodies deliberately, and reading
    /// `.content.items` needs the whole thing in hand. See rule V33.
    #[serde(default = "default_body_limit")]
    pub body_limit: ByteSize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mocks: Vec<MockConfig>,
}

/// 1 MiB: enough for a typical JSON body without making an unbounded buffer
/// the default for every proxy.
fn default_body_limit() -> ByteSize {
    ByteSize(1024 * 1024)
}

/// A proxy URL with any credentials removed, for anywhere it is shown.
///
/// `url` accepts `https://user:secret@host/` and no validation rule refuses
/// it, so an upstream behind basic auth can legitimately be configured that
/// way. Anything that displays an upstream to someone who is not already
/// trusted with the configuration -- `/status`, which is public, is the
/// first such place -- has to go through this.
///
/// A string that does not parse is redacted whole. It should not be
/// reachable, because validation parses every proxy URL, but the safe
/// failure for a redactor is to reveal less rather than to fall back to the
/// input it could not understand.
#[must_use]
pub fn redact_credentials(url: &str) -> String {
    const REDACTED: &str = "<redacted>";

    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return REDACTED.to_owned();
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return url.to_owned();
    }
    // Both setters fail only for a URL that cannot have a host -- `mailto:`
    // and friends -- which proxy validation already rejects.
    if parsed.set_password(None).is_err() || parsed.set_username("").is_err() {
        return REDACTED.to_owned();
    }
    parsed.to_string()
}

#[cfg(test)]
mod redaction_tests {
    use super::redact_credentials;

    #[test]
    fn a_plain_url_is_returned_unchanged() {
        // Byte for byte, not merely equivalent: `/status` shows this to an
        // operator comparing it against what they configured.
        let url = "https://alpha.example.com/api/v1/";
        assert_eq!(redact_credentials(url), url);
    }

    #[test]
    fn a_password_is_removed_and_the_host_is_kept() {
        let redacted = redact_credentials("https://user:secret@alpha.example.com/api/");
        assert!(!redacted.contains("secret"), "{redacted}");
        assert!(!redacted.contains("user"), "{redacted}");
        assert!(redacted.contains("alpha.example.com"), "{redacted}");
    }

    #[test]
    fn a_username_without_a_password_is_also_removed() {
        let redacted = redact_credentials("https://token@alpha.example.com/api/");
        assert!(!redacted.contains("token"), "{redacted}");
    }

    #[test]
    fn an_at_sign_outside_the_authority_is_not_treated_as_credentials() {
        // The `@` here belongs to the path. A hand-rolled "split on @"
        // redactor mangles this one; the real parser does not.
        let url = "https://alpha.example.com/api/user@example.com/";
        assert_eq!(redact_credentials(url), url);
    }

    #[test]
    fn an_unparsable_url_is_redacted_whole_rather_than_echoed() {
        assert_eq!(redact_credentials("not a url"), "<redacted>");
    }
}
