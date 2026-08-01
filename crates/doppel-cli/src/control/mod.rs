//! The Unix socket control channel: newline-delimited JSON, one request and one
//! response per connection. The tagged shapes leave room for the `status` and
//! `drain` commands planned for later phases without a protocol change.

pub mod client;
pub mod server;

use doppel_core::Violation;
use serde::{Deserialize, Serialize};

pub use server::ControlServer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlRequest {
    Reload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    Ok {
        revision: u64,
        proxies: usize,
    },
    Error {
        code: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        errors: Vec<Violation>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_request_is_tagged_by_command() {
        let text = serde_json::to_string(&ControlRequest::Reload).unwrap();
        assert_eq!(text, r#"{"command":"reload"}"#);
    }

    #[test]
    fn unknown_command_fails_to_deserialize() {
        assert!(serde_json::from_str::<ControlRequest>(r#"{"command":"explode"}"#).is_err());
    }

    #[test]
    fn ok_response_matches_the_documented_shape() {
        let text = serde_json::to_string(&ControlResponse::Ok {
            revision: 3,
            proxies: 2,
        })
        .unwrap();
        assert_eq!(text, r#"{"status":"ok","revision":3,"proxies":2}"#);
    }

    #[test]
    fn error_response_matches_the_documented_shape() {
        let response = ControlResponse::Error {
            code: "CONFIG_INVALID".to_owned(),
            errors: vec![doppel_core::Violation::new(
                "proxies[0].latency.min",
                "min must be <= max",
            )],
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["status"], "error");
        assert_eq!(value["code"], "CONFIG_INVALID");
        assert_eq!(value["errors"][0]["path"], "proxies[0].latency.min");
    }

    #[test]
    fn error_response_without_errors_omits_the_list() {
        let text = serde_json::to_string(&ControlResponse::Error {
            code: "NOT_FOUND".to_owned(),
            errors: vec![],
        })
        .unwrap();
        assert_eq!(text, r#"{"status":"error","code":"NOT_FOUND"}"#);
    }
}
