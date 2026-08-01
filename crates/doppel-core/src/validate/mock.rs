//! Mock rules V16..V25 and V30. Implemented in Task 6.

use super::Violations;
use crate::config::ProxyConfig;

// Not yet called: `proxy::check` starts calling this in Task 5. Allowed dead
// code until then so this placeholder does not trip `-D warnings`.
#[allow(dead_code)]
pub(super) fn check(_proxy: &ProxyConfig, _path: &str, _v: &mut Violations) {}
