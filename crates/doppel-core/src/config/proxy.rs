//! Per-proxy settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::admin::ProxyAccessConfig;
use super::mock::MockConfig;
use super::size::ByteSize;

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
    pub header: Option<super::HeaderName>,
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
    pub percentage: super::Ratio,
    pub status: super::HttpStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencyConfig {
    pub percentage: super::Ratio,
    pub min: super::Seconds,
    pub max: super::Seconds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    pub name: crate::config::Name,
    #[serde(rename = "type")]
    pub kind: ProxyKind,
    pub url: super::UpstreamUrl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<super::TimeoutSeconds>,
    #[serde(default)]
    pub resolve: ResolveConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<ProxyAccessConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<super::HeaderName, super::HeaderValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<LossConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<LatencyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<super::Ratio>,
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
    ByteSize::parse(1024 * 1024).expect("1 MiB is within the accepted range")
}
