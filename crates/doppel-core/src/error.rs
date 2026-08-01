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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    #[test]
    fn codes_map_to_documented_statuses() {
        assert_eq!(ErrorCode::ProxyNotResolved.status(), 404);
        assert_eq!(ErrorCode::TemplateRenderError.status(), 500);
        assert_eq!(ErrorCode::UpstreamTimeout.status(), 504);
        assert_eq!(ErrorCode::UpstreamError.status(), 502);
        assert_eq!(ErrorCode::UploadTooLarge.status(), 413);
        assert_eq!(ErrorCode::TemplateNotDeclared.status(), 422);
        assert_eq!(ErrorCode::Conflict.status(), 409);
        assert_eq!(ErrorCode::RevisionMismatch.status(), 409);
        assert_eq!(ErrorCode::InvalidRequestPath.status(), 400);
    }

    #[test]
    fn code_serializes_as_screaming_snake_case() {
        assert_eq!(ErrorCode::ProxyNotResolved.as_str(), "PROXY_NOT_RESOLVED");
        assert_eq!(
            ErrorCode::TemplateRenderError.as_str(),
            "TEMPLATE_RENDER_ERROR"
        );
        assert_eq!(ErrorCode::RevisionMismatch.as_str(), "REVISION_MISMATCH");
        assert_eq!(
            ErrorCode::InvalidRequestPath.as_str(),
            "INVALID_REQUEST_PATH"
        );
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
        let all = [
            ErrorCode::ProxyNotResolved,
            ErrorCode::TemplateRenderError,
            ErrorCode::TemplateNotFound,
            ErrorCode::BodyExtractionError,
            ErrorCode::UpstreamTimeout,
            ErrorCode::UpstreamError,
            ErrorCode::ConfigInvalid,
            ErrorCode::Unauthorized,
            ErrorCode::Forbidden,
            ErrorCode::NotFound,
            ErrorCode::Conflict,
            ErrorCode::UploadTooLarge,
            ErrorCode::TemplateNotDeclared,
            ErrorCode::StoreError,
            ErrorCode::RevisionMismatch,
            ErrorCode::InvalidRequestPath,
        ];
        for code in all {
            let text = serde_json::to_string(&code).unwrap();
            let parsed: ErrorCode = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed, code, "round trip failed for {text}");
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
