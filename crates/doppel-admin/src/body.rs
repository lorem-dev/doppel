//! Reading a request body under a bound this API chose.
//!
//! axum's `DefaultBodyLimit` would do the bounding, but it answers with its
//! own plain-text 413 from a layer that knows nothing about this API's error
//! envelope -- and, for an upload, nothing about `admin.upload.limit` either.
//! Every route that takes a body therefore disables that layer and comes
//! here, so one shape of error reaches every client.

use axum::body::Body;
use axum::http::{HeaderMap, header};
use doppel_core::{Error, ErrorCode};

/// The most a configuration document may weigh.
///
/// Not `admin.upload.limit`, which is the operator's number for template
/// files. This one bounds a JSON proxy definition, and a megabyte is already
/// several orders of magnitude more than any real one; the point is that the
/// buffer has a ceiling at all, not where the ceiling is.
pub const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

/// Collect a body, refusing anything over `limit` bytes.
pub async fn read_body(body: Body, headers: &HeaderMap, limit: u64) -> Result<Vec<u8>, Error> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);

    // A `Content-Length` over the limit is refused before a single byte is
    // read. This is also what makes the mapping below safe: with the
    // announced-length case handled here, `to_bytes` failing is either a
    // chunked body that ran past the limit or a transfer that broke, and
    // both mean the same thing to whoever sent it -- no file was stored.
    let announced = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if announced.is_some_and(|announced| announced > limit as u64) {
        return Err(too_large(limit));
    }

    axum::body::to_bytes(body, limit)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| {
            tracing::debug!(error = %err, "upload body rejected");
            too_large(limit)
        })
}

pub(crate) fn too_large(limit: usize) -> Error {
    // Deliberately not "the configured upload limit": this bounds a template
    // upload in one place and a configuration document in another, and naming
    // the wrong setting sends the reader to edit something irrelevant.
    Error::new(
        ErrorCode::UploadTooLarge,
        format!("request body exceeds the limit of {limit} bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_body_at_the_limit_is_kept_and_one_over_it_is_refused() {
        let headers = HeaderMap::new();
        let at = read_body(Body::from(vec![b'x'; 8]), &headers, 8)
            .await
            .unwrap();
        assert_eq!(at.len(), 8);

        let over = read_body(Body::from(vec![b'x'; 9]), &headers, 8)
            .await
            .unwrap_err();
        assert_eq!(over.code, ErrorCode::UploadTooLarge);
        assert_eq!(over.status(), 413);
    }

    #[tokio::test]
    async fn an_announced_length_of_exactly_the_limit_is_accepted() {
        // The boundary, on the header path rather than the buffered one. An
        // exclusive comparison here would reject the exact size an operator
        // configured, and only a body that announces its length would hit it.
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, "8".parse().unwrap());
        let body = read_body(Body::from(vec![b'x'; 8]), &headers, 8)
            .await
            .unwrap();
        assert_eq!(body.len(), 8);
    }

    #[tokio::test]
    async fn an_announced_length_over_the_limit_is_refused_without_reading() {
        // The body here is empty, so only the header can have caused the
        // rejection: a client that lies large is stopped before it can stream
        // anything at all.
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, "999".parse().unwrap());
        let err = read_body(Body::empty(), &headers, 8).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UploadTooLarge);
    }
}
