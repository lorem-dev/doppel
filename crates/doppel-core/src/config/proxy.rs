//! Per-proxy settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::admin::ProxyAccessConfig;
use super::mock::MockConfig;
use super::size::ByteSize;

/// What a proxy forwards. Only `http` exists.
///
/// There was a `Tcp` variant, admitted by the parser so that rule V7 could
/// reject it afterwards with a message better than serde's. Both are gone: the
/// variant meant every layer downstream -- the runtime, the store, this schema --
/// had to carry a case that could never be reached, and the good message is
/// available without it. `Deserialize` below is written by hand for exactly that
/// reason, so `type: tcp` still says *why* rather than only that the value is
/// not one of the accepted ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    /// Forward HTTP, the only kind Doppel implements.
    Http,
}

impl<'de> Deserialize<'de> for ProxyKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "http" => Ok(Self::Http),
            "tcp" => Err(serde::de::Error::custom(
                "`tcp` proxying is not implemented; `http` is the only proxy type",
            )),
            other => Err(serde::de::Error::custom(format!(
                "`{other}` is not a proxy type; `http` is the only one"
            ))),
        }
    }
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
    /// `default` takes anything unclaimed; `header` takes requests naming
    /// this proxy in `header`.
    #[serde(rename = "type", default = "default_resolve_kind")]
    pub kind: ResolveKind,
    /// The header carrying the proxy name. Required when `type: header`, and
    /// meaningless otherwise.
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
    /// The share of requests to drop, as a fraction. `0.1` is one in ten.
    pub percentage: super::Ratio,
    /// The status a dropped request is answered with, rather than being left
    /// to hang.
    pub status: super::HttpStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LatencyConfig {
    /// The share of requests to delay, as a fraction.
    pub percentage: super::Ratio,
    /// Lower bound of the delay, in seconds. The delay is a target for the
    /// whole response: time the upstream already spent is subtracted.
    pub min: super::Seconds,
    /// Upper bound of the delay, in seconds. Must be at least `min`.
    pub max: super::Seconds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    /// Names this proxy in `X-Proxy-Name`, in metrics labels, in log lines
    /// and as its template subdirectory. Unique within the document.
    pub name: crate::config::ProxyName,
    /// What it forwards. `http` is the only value.
    #[serde(rename = "type")]
    pub kind: ProxyKind,
    /// The upstream base. A request path is grafted underneath it, and the
    /// result can never escape it -- so a base with a path confines the proxy
    /// to that subtree.
    pub url: super::UpstreamUrl,
    /// Bounds the whole upstream exchange, in seconds. Exceeding it is
    /// `504 UPSTREAM_TIMEOUT`. Defaults to 30.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<super::TimeoutSeconds>,
    /// How a request is matched to this proxy: by header, or as the default.
    #[serde(default)]
    pub resolve: ResolveConfig,
    /// Overrides the admin `access` rules for this proxy alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<ProxyAccessConfig>,
    /// Headers injected into every outbound request, overriding whatever the
    /// client sent by the same name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<super::HeaderName, super::HeaderValue>,
    /// Drops a share of requests rather than forwarding them. Not applied to
    /// a request a mock answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss: Option<LossConfig>,
    /// Makes a share of requests take a chosen time. Applies to mocked
    /// responses too; a mock may override the figure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<LatencyConfig>,
    /// What share of requests a matching mock actually answers; the rest go
    /// upstream. Defaults to 1.0, so a matching mock answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<super::Ratio>,
    /// Whether a redirect whose `Location` points back into the space this
    /// proxy forwards is rewritten to point at Doppel instead. Absent means
    /// enabled.
    ///
    /// On by default because the alternative is a silent failure: `Host` is
    /// replaced with the upstream's authority, so the upstream's `Location`
    /// names the upstream, and a client following it leaves Doppel -- along with
    /// every fault and every mock -- with nothing reported. Turn it off to have
    /// the response relayed byte for byte, which is what a client being tested
    /// *against redirect handling itself* needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_redirects: Option<bool>,
    /// Bounds the request body a matched mock is allowed to buffer in order
    /// to extract from it; phase 1 streams bodies deliberately, and reading
    /// `.content.items` needs the whole thing in hand. See rule V33.
    #[serde(default = "default_body_limit")]
    pub body_limit: ByteSize,
    /// Mocks in the order they are tried. First match wins, and patterns are
    /// unanchored, so a general one placed first shadows the rest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mocks: Vec<MockConfig>,
}

/// 1 MiB: enough for a typical JSON body without making an unbounded buffer
/// the default for every proxy.
fn default_body_limit() -> ByteSize {
    ByteSize::parse(1024 * 1024).expect("1 MiB is within the accepted range")
}
