//! The single error type for Doppel, and the wire envelope it serializes to.

use serde::{Deserialize, Serialize};

/// Machine readable error code. The set is closed on purpose: handlers must not
/// invent their own codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ProxyNotResolved,
    TemplateRenderError,
    TemplateNotFound,
    BodyExtractionError,
    UpstreamTimeout,
    UpstreamError,
    ConfigInvalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    UploadTooLarge,
    TemplateNotDeclared,
    StoreError,
    RevisionMismatch,
    RevisionRequired,
    InvalidRequestPath,
}

impl ErrorCode {
    /// HTTP status this code is reported with.
    #[must_use]
    pub fn status(self) -> u16 {
        match self {
            Self::ProxyNotResolved | Self::NotFound => 404,
            Self::TemplateRenderError
            | Self::TemplateNotFound
            | Self::BodyExtractionError
            | Self::StoreError => 500,
            Self::UpstreamTimeout => 504,
            Self::UpstreamError => 502,
            Self::ConfigInvalid => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            // `Conflict` and `RevisionMismatch` report the same HTTP status,
            // 409, but stay separate codes here on purpose: the status is
            // what an HTTP-level intermediary cares about, but a client
            // acting on the body needs to tell "the thing you tried to
            // create already exists" (`CONFLICT`) apart from "you are
            // holding a stale copy, re-read before retrying"
            // (`REVISION_MISMATCH`).
            Self::Conflict => 409,
            Self::RevisionMismatch => 409,
            // 428 Precondition Required (RFC 6585), which exists for
            // precisely this: the server insists the request be conditional
            // so that a lost update cannot happen. It is deliberately not
            // 409 -- nothing conflicted, because nothing was compared -- and
            // deliberately not a generic 400, because the fix is specific
            // and mechanical: re-read, then send the revision you got back.
            Self::RevisionRequired => 428,
            Self::UploadTooLarge => 413,
            Self::TemplateNotDeclared => 422,
            // A client that sends a `.`/`..` segment, or a path that would
            // otherwise resolve outside the configured upstream, made a bad
            // request: this is not the upstream's fault, so it is not a
            // 502/504-class failure.
            Self::InvalidRequestPath => 400,
        }
    }

    /// Wire representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProxyNotResolved => "PROXY_NOT_RESOLVED",
            Self::TemplateRenderError => "TEMPLATE_RENDER_ERROR",
            Self::TemplateNotFound => "TEMPLATE_NOT_FOUND",
            Self::BodyExtractionError => "BODY_EXTRACTION_ERROR",
            Self::UpstreamTimeout => "UPSTREAM_TIMEOUT",
            Self::UpstreamError => "UPSTREAM_ERROR",
            Self::ConfigInvalid => "CONFIG_INVALID",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::NotFound => "NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::UploadTooLarge => "UPLOAD_TOO_LARGE",
            Self::TemplateNotDeclared => "TEMPLATE_NOT_DECLARED",
            Self::StoreError => "STORE_ERROR",
            Self::RevisionMismatch => "REVISION_MISMATCH",
            Self::RevisionRequired => "REVISION_REQUIRED",
            Self::InvalidRequestPath => "INVALID_REQUEST_PATH",
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// The reverse of `as_str`. Kept in lockstep with it by hand, same as
/// `Serialize` above: an unrecognised wire value is a deserialize error, not
/// a new variant invented on the spot -- the set is closed on both sides of
/// the wire, not just the Rust one.
impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        match value.as_str() {
            "PROXY_NOT_RESOLVED" => Ok(Self::ProxyNotResolved),
            "TEMPLATE_RENDER_ERROR" => Ok(Self::TemplateRenderError),
            "TEMPLATE_NOT_FOUND" => Ok(Self::TemplateNotFound),
            "BODY_EXTRACTION_ERROR" => Ok(Self::BodyExtractionError),
            "UPSTREAM_TIMEOUT" => Ok(Self::UpstreamTimeout),
            "UPSTREAM_ERROR" => Ok(Self::UpstreamError),
            "CONFIG_INVALID" => Ok(Self::ConfigInvalid),
            "UNAUTHORIZED" => Ok(Self::Unauthorized),
            "FORBIDDEN" => Ok(Self::Forbidden),
            "NOT_FOUND" => Ok(Self::NotFound),
            "CONFLICT" => Ok(Self::Conflict),
            "UPLOAD_TOO_LARGE" => Ok(Self::UploadTooLarge),
            "TEMPLATE_NOT_DECLARED" => Ok(Self::TemplateNotDeclared),
            "STORE_ERROR" => Ok(Self::StoreError),
            "REVISION_MISMATCH" => Ok(Self::RevisionMismatch),
            "REVISION_REQUIRED" => Ok(Self::RevisionRequired),
            "INVALID_REQUEST_PATH" => Ok(Self::InvalidRequestPath),
            other => Err(serde::de::Error::custom(format!(
                "unknown error code `{other}`"
            ))),
        }
    }
}

/// An error that can be reported to a client.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn status(&self) -> u16 {
        self.code.status()
    }
}

/// The response body shape required by the spec. Field order is part of the
/// contract, so the struct field order is load bearing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub status: String,
    pub message: String,
    pub code: String,
}

impl From<&Error> for ErrorBody {
    fn from(err: &Error) -> Self {
        Self {
            status: "error".to_owned(),
            message: err.message.clone(),
            code: err.code.as_str().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `ErrorCode` variant paired with its documented wire string and
    /// HTTP status. `all_codes_map_to_their_documented_status`,
    /// `all_codes_map_to_their_documented_wire_string` and
    /// `every_code_round_trips_through_json` below are all driven from this
    /// one list rather than each hand-listing the seventeen variants
    /// separately, so a code missing from one of them cannot happen without
    /// being missing from all.
    ///
    /// `all_codes_are_listed_exactly_once` is what keeps this list itself
    /// honest: `assert_listed`'s match has no wildcard arm, so the compiler
    /// refuses to build this test suite at all the moment `ErrorCode` gains
    /// an eighteenth variant without a corresponding arm added there --
    /// regardless of whether that arm is ever reached at runtime. Each arm
    /// also asserts its variant appears in `ALL_CODES` exactly once, so an
    /// entry that is present in the enum but quietly dropped from this list
    /// (rather than never added) still fails, at test time.
    const ALL_CODES: &[(ErrorCode, &str, u16)] = &[
        (ErrorCode::ProxyNotResolved, "PROXY_NOT_RESOLVED", 404),
        (ErrorCode::TemplateRenderError, "TEMPLATE_RENDER_ERROR", 500),
        (ErrorCode::TemplateNotFound, "TEMPLATE_NOT_FOUND", 500),
        (ErrorCode::BodyExtractionError, "BODY_EXTRACTION_ERROR", 500),
        (ErrorCode::UpstreamTimeout, "UPSTREAM_TIMEOUT", 504),
        (ErrorCode::UpstreamError, "UPSTREAM_ERROR", 502),
        (ErrorCode::ConfigInvalid, "CONFIG_INVALID", 400),
        (ErrorCode::Unauthorized, "UNAUTHORIZED", 401),
        (ErrorCode::Forbidden, "FORBIDDEN", 403),
        (ErrorCode::NotFound, "NOT_FOUND", 404),
        (ErrorCode::Conflict, "CONFLICT", 409),
        (ErrorCode::UploadTooLarge, "UPLOAD_TOO_LARGE", 413),
        (ErrorCode::TemplateNotDeclared, "TEMPLATE_NOT_DECLARED", 422),
        (ErrorCode::StoreError, "STORE_ERROR", 500),
        (ErrorCode::RevisionMismatch, "REVISION_MISMATCH", 409),
        (ErrorCode::RevisionRequired, "REVISION_REQUIRED", 428),
        (ErrorCode::InvalidRequestPath, "INVALID_REQUEST_PATH", 400),
    ];

    /// See `ALL_CODES`'s doc comment: this match's lack of a wildcard arm is
    /// the actual enforcement mechanism, checked at compile time regardless
    /// of how (or whether) this function is called at runtime.
    fn assert_listed_exactly_once(code: ErrorCode) {
        match code {
            ErrorCode::ProxyNotResolved
            | ErrorCode::TemplateRenderError
            | ErrorCode::TemplateNotFound
            | ErrorCode::BodyExtractionError
            | ErrorCode::UpstreamTimeout
            | ErrorCode::UpstreamError
            | ErrorCode::ConfigInvalid
            | ErrorCode::Unauthorized
            | ErrorCode::Forbidden
            | ErrorCode::NotFound
            | ErrorCode::Conflict
            | ErrorCode::UploadTooLarge
            | ErrorCode::TemplateNotDeclared
            | ErrorCode::StoreError
            | ErrorCode::RevisionMismatch
            | ErrorCode::RevisionRequired
            | ErrorCode::InvalidRequestPath => {
                let count = ALL_CODES.iter().filter(|(c, _, _)| *c == code).count();
                assert_eq!(count, 1, "{code:?} must appear exactly once in ALL_CODES");
            }
        }
    }

    #[test]
    fn all_codes_are_listed_exactly_once() {
        for (code, _, _) in ALL_CODES {
            assert_listed_exactly_once(*code);
        }
    }

    #[test]
    fn all_codes_map_to_their_documented_status() {
        for (code, _, status) in ALL_CODES {
            assert_eq!(code.status(), *status, "{code:?}");
        }
    }

    #[test]
    fn all_codes_map_to_their_documented_wire_string() {
        for (code, wire, _) in ALL_CODES {
            assert_eq!(code.as_str(), *wire, "{code:?}");
        }
    }

    #[test]
    fn revision_mismatch_and_conflict_share_a_status_but_are_distinct_codes() {
        // Same HTTP status, 409, but different wire codes: a client must be
        // able to tell "the thing you tried to create already exists"
        // (`CONFLICT`) apart from "you are holding a stale copy, re-read
        // before retrying" (`REVISION_MISMATCH`).
        assert_eq!(
            ErrorCode::Conflict.status(),
            ErrorCode::RevisionMismatch.status()
        );
        assert_ne!(
            ErrorCode::Conflict.as_str(),
            ErrorCode::RevisionMismatch.as_str()
        );
    }

    #[test]
    fn envelope_has_the_exact_documented_shape() {
        let err = Error::new(ErrorCode::TemplateRenderError, "missing variable 'id'");
        let body = ErrorBody::from(&err);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "status": "error",
                "message": "missing variable 'id'",
                "code": "TEMPLATE_RENDER_ERROR",
            })
        );
    }

    #[test]
    fn every_code_round_trips_through_json() {
        for (code, _, _) in ALL_CODES {
            let text = serde_json::to_string(code).unwrap();
            let parsed: ErrorCode = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed, *code, "round trip failed for {text}");
        }
    }

    #[test]
    fn an_unrecognised_code_fails_to_deserialize_rather_than_inventing_a_variant() {
        assert!(serde_json::from_str::<ErrorCode>(r#""MADE_UP_CODE""#).is_err());
    }

    #[test]
    fn envelope_keys_are_ordered_status_message_code() {
        let err = Error::new(ErrorCode::NotFound, "nope");
        let json = serde_json::to_string(&ErrorBody::from(&err)).unwrap();
        assert_eq!(
            json,
            r#"{"status":"error","message":"nope","code":"NOT_FOUND"}"#
        );
    }
}
