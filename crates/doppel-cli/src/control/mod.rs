//! The Unix socket control channel: newline-delimited JSON, one request and one
//! response per connection. The tagged shapes leave room for the `status` and
//! `drain` commands planned for later phases without a protocol change.

pub mod client;
pub mod server;

use doppel_core::{ErrorCode, Violation};
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
        /// Names of top-level sections (`server`, `logging`, `control`,
        /// `templates`, `sentry`, `admin`) that changed in the newly loaded
        /// config but were not applied: `Runtime::compile` only ever reads
        /// `config.proxies`, so a reload that also edited, say,
        /// `server.port` is validated, counted into the new revision, and
        /// then quietly has no effect on the running listener. Empty on the
        /// common case where only `proxies` changed.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        unapplied: Vec<String>,
    },
    Error {
        code: ErrorCode,
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
            unapplied: Vec::new(),
        })
        .unwrap();
        assert_eq!(text, r#"{"status":"ok","revision":3,"proxies":2}"#);
    }

    #[test]
    fn ok_response_lists_unapplied_sections_when_present() {
        let text = serde_json::to_string(&ControlResponse::Ok {
            revision: 3,
            proxies: 2,
            unapplied: vec!["logging".to_owned()],
        })
        .unwrap();
        assert_eq!(
            text,
            r#"{"status":"ok","revision":3,"proxies":2,"unapplied":["logging"]}"#
        );
    }

    #[test]
    fn error_response_matches_the_documented_shape() {
        let response = ControlResponse::Error {
            code: ErrorCode::ConfigInvalid,
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
            code: ErrorCode::NotFound,
            errors: vec![],
        })
        .unwrap();
        assert_eq!(text, r#"{"status":"error","code":"NOT_FOUND"}"#);
    }

    #[test]
    fn error_code_round_trips_through_the_control_response() {
        let text = serde_json::to_string(&ControlResponse::Error {
            code: ErrorCode::StoreError,
            errors: Vec::new(),
        })
        .unwrap();
        let parsed: ControlResponse = serde_json::from_str(&text).unwrap();
        assert_eq!(
            parsed,
            ControlResponse::Error {
                code: ErrorCode::StoreError,
                errors: Vec::new(),
            }
        );
    }
}
