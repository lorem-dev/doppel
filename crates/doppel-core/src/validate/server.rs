//! Rules V1 and V3. V2 and V4 are enforced by the config types themselves.

use super::Violations;
use crate::config::Config;

pub(super) fn check(config: &Config, v: &mut Violations) {
    // V1
    v.require(config.server.port != 0, "server.port", "port must not be 0");
    v.require(config.admin.port != 0, "admin.port", "port must not be 0");
    v.require(
        config.server.port != config.admin.port,
        "admin.port",
        "admin port must differ from the server port",
    );

    // V3
    if let Some(workers) = config.server.workers {
        v.require(workers >= 1, "server.workers", "workers must be at least 1");
    }
    if let Some(workers) = config.admin.workers {
        v.require(workers >= 1, "admin.workers", "workers must be at least 1");
    }
}
