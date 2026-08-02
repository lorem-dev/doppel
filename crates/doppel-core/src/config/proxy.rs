//! Per-proxy settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::admin::{ByteSize, ProxyAccessConfig};
use super::mock::MockConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Http,
    Tcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolveKind {
    Default,
    Header,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossConfig {
    pub percentage: f64,
    pub status: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyConfig {
    pub percentage: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
