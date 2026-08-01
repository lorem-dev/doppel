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
            Self::Conflict => 409,
            Self::UploadTooLarge => 413,
            Self::TemplateNotDeclared => 422,
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
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
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
    }

    #[test]
    fn code_serializes_as_screaming_snake_case() {
        assert_eq!(ErrorCode::ProxyNotResolved.as_str(), "PROXY_NOT_RESOLVED");
        assert_eq!(
            ErrorCode::TemplateRenderError.as_str(),
            "TEMPLATE_RENDER_ERROR"
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
    fn envelope_keys_are_ordered_status_message_code() {
        let err = Error::new(ErrorCode::NotFound, "nope");
        let json = serde_json::to_string(&ErrorBody::from(&err)).unwrap();
        assert_eq!(
            json,
            r#"{"status":"error","message":"nope","code":"NOT_FOUND"}"#
        );
    }
}
