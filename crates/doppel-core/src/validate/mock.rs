//! Mock rules V16..V25 and V30. Implemented in Task 6.

use super::Violations;
use crate::config::ProxyConfig;

pub(super) fn check(_proxy: &ProxyConfig, _path: &str, _v: &mut Violations) {}
