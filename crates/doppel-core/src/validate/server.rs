//! Rule V1. V2 and V4 are enforced by the config types themselves. V3
//! governed `server.workers`, which is no longer a configuration field.
//!
//! V1 used to also refuse a `0` port on each listener. `config::Port` does
//! that now, at parse time and with a message that says what 0 would mean.
//! What is left here is the part that is genuinely a rule: it compares two
//! fields, and no single value can be checked for it.

use super::Violations;
use crate::config::Config;

pub(super) fn check(config: &Config, v: &mut Violations) {
    // V1
    v.require(
        config.server.port != config.admin.port,
        "admin.port",
        "admin port must differ from the server port",
    );
}
