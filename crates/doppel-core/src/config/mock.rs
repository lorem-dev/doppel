//! Mock definitions. Parsed and validated in phase 1, served in phase 2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::proxy::{LatencyConfig, LossConfig};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockConfig {
    pub name: crate::config::Name,
    pub request: MockRequest,
    pub response: MockResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<MockProxyOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockRequest {
    pub method: crate::config::HttpMethod,
    pub url: String,
    /// Variable name -> request header name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Variable name -> selector such as `.filter`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    /// Variable name -> selector such as `.content.items`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockResponse {
    pub status: crate::config::HttpStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Header name -> template producing the value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl MockResponse {
    /// The response body source, if any. Used by validation rules V20 and V30
    /// and by the phase 2 renderer.
    #[must_use]
    pub fn body_sources(&self) -> usize {
        usize::from(self.body.is_some())
            + usize::from(self.json.is_some())
            + usize::from(self.template.is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockProxyOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<LossConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<LatencyConfig>,
}
