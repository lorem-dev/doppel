//! Mock definitions. Parsed and validated in phase 1, served in phase 2.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::proxy::{LatencyConfig, LossConfig};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockConfig {
    /// Names this mock in the `mock` log field and the hit counter. Unique
    /// within the proxy.
    pub name: crate::config::Name,
    /// What this mock matches, and what it takes out of the request.
    pub request: MockRequest,
    /// What it answers with.
    pub response: MockResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Per-mock overrides of the proxy's `replace`, `loss` and `latency`.
    pub proxy: Option<MockProxyOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockRequest {
    /// The method this mock answers, matched exactly and upper case.
    pub method: crate::config::HttpMethod,
    /// A regex matched against the request path, unanchored. Named capture
    /// groups become template variables.
    pub url: crate::config::Pattern,
    /// Variable name -> request header name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, crate::config::HeaderName>,
    /// Variable name -> selector such as `.filter`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, crate::config::Selector>,
    /// Variable name -> selector such as `.content.items`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body: BTreeMap<String, crate::config::Selector>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MockResponse {
    /// The status to answer with.
    pub status: crate::config::HttpStatus,
    /// A template rendered and sent as `text/plain`. Exclusive with `json` and
    /// `template`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// A template whose rendered output must be valid JSON, sent as
    /// `application/json`. Exclusive with `body` and `template`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    /// A template file under this proxy's template directory. Read per request,
    /// so it may be uploaded after the configuration was loaded. Exclusive with
    /// `body` and `json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<crate::config::TemplateName>,
    /// Header name -> template producing the value.
    ///
    /// The value is a template, not a header value: what it renders to is
    /// only a header value once a request has been served, which is checked
    /// there. The name is a name now.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<crate::config::HeaderName, String>,
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
    /// Overrides the proxy's `replace` for requests this mock matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<crate::config::Ratio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Drops a share of the requests this mock would have answered. Not
    /// inherited from the proxy: a mock without it is never dropped.
    pub loss: Option<LossConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Replaces the proxy's `latency` for this mock's responses, rather than
    /// adding to it.
    pub latency: Option<LatencyConfig>,
}
