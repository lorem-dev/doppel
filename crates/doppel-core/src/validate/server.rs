//! Rule V1. V2 and V4 are enforced by the config types themselves. V3
//! governed `server.workers`, which is no longer a configuration field.

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
}
