//! Turning a `doppel_core::Error` into the one response envelope the whole
//! product uses, and mapping store failures onto it.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use doppel_core::store::StoreError;
use doppel_core::validate::Violation;
use doppel_core::{Error, ErrorBody, ErrorCode};

/// A `doppel_core::Error` on its way out of a handler.
///
/// Handlers return `Result<T, ApiError>` so that `?` works on the core error
/// type, and every failure path lands in exactly one place that decides the
/// status and the body. There is no second way to write an error response.
#[derive(Debug)]
pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(err: Error) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status())
            // Every `ErrorCode::status` is a literal in a closed match, so
            // this cannot be reached without someone adding an invalid one;
            // reporting 500 beats panicking inside a response conversion.
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(ErrorBody::from(&self.0))).into_response()
    }
}

/// A validation failure, with every violation named.
///
/// The violations go in the envelope's `message` rather than in a field of
/// their own: requirement 11 fixes the body shape at exactly `status`,
/// `message` and `code`, and a client that has to parse a fourth field to
/// learn what went wrong is worse off than one that can print the message.
#[must_use]
pub fn config_invalid(violations: &[Violation]) -> Error {
    let detail = violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    Error::new(
        ErrorCode::ConfigInvalid,
        format!("configuration is invalid: {detail}"),
    )
}

/// Map a store failure onto a client-facing error.
///
/// Anything that names a local path or an internal serialization detail is
/// reported as a flat `STORE_ERROR` and logged instead. `list` and `read` are
/// public by default, so a store error is reachable without a token, and an
/// unauthenticated caller has no business learning where on the filesystem
/// the configuration lives.
#[must_use]
pub fn store_error(err: &StoreError) -> Error {
    match err {
        StoreError::Invalid(violations) => config_invalid(violations),
        StoreError::BadTemplateName { name, reason } => Error::new(
            ErrorCode::ConfigInvalid,
            format!("template name `{name}` rejected: {reason}"),
        ),
        // Reached only if a compare-and-swap failure escapes the retry loop
        // that is supposed to handle it. Reporting it as a conflict is
        // honest; it is contention, not a stale client copy.
        StoreError::RevisionMismatch { .. } => Error::new(
            ErrorCode::Conflict,
            "the stored configuration changed while this request was being applied",
        ),
        StoreError::NotFound(_) | StoreError::Io { .. } | StoreError::Serialize(_) => {
            tracing::error!(error = %err, "configuration store failure");
            Error::new(
                ErrorCode::StoreError,
                "the configuration store is unavailable; see the server log",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_io_failure_does_not_leak_the_path_to_the_client() {
        let err = store_error(&StoreError::Io {
            path: "/srv/secret/main.yaml".into(),
            source: std::io::Error::other("boom"),
        });
        assert_eq!(err.code, ErrorCode::StoreError);
        assert!(
            !err.message.contains("/srv/secret"),
            "message leaked a path: {}",
            err.message
        );
    }

    #[test]
    fn a_missing_config_file_is_a_store_error_not_a_not_found() {
        // `NOT_FOUND` on this API means "no such proxy". Reusing it for "the
        // configuration file is gone" would tell a client to stop retrying
        // when the right answer is that the server is broken.
        let err = store_error(&StoreError::NotFound("/srv/main.yaml".into()));
        assert_eq!(err.code, ErrorCode::StoreError);
        assert_eq!(err.status(), 500);
    }

    #[test]
    fn violations_are_all_reported_not_just_the_first() {
        let err = config_invalid(&[
            Violation::new("proxies[0].url", "must be absolute"),
            Violation::new("proxies[1].name", "must be unique"),
        ]);
        assert_eq!(err.code, ErrorCode::ConfigInvalid);
        assert!(err.message.contains("proxies[0].url"), "{}", err.message);
        assert!(err.message.contains("proxies[1].name"), "{}", err.message);
    }
}
