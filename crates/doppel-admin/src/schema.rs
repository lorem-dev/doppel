//! The configuration schema, served by the process the configuration is for.
//!
//! `doppel-config.schema.json` in the repository and the copy attached to every
//! release are the same document, and both answer "what does a configuration look
//! like" for whichever version you fetched. This answers it for the version that is
//! *running*, which is the question a client editing a live configuration is
//! actually asking.
//!
//! The dashboard is that client: it takes the field bounds it enforces while
//! someone types -- patterns, lengths, ranges -- and the validation in its YAML
//! editor from here, so there is no second, laxer copy of the rules written in
//! TypeScript. A rule added to a newtype in `doppel-core` reaches the page without
//! anyone editing the page.

use std::sync::OnceLock;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::state::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new().route("/api/v1/schema", get(schema))
}

/// The schema, serialized once.
///
/// Building it walks every `ToSchema` implementation in the configuration types
/// and rewrites every `$ref`, which is not work worth repeating per request for a
/// document that cannot change while the process lives.
fn body() -> &'static str {
    static BODY: OnceLock<String> = OnceLock::new();
    BODY.get_or_init(|| {
        serde_json::to_string(&doppel_core::config::schema::json_schema())
            .expect("a json value serializes")
    })
}

/// `GET /api/v1/schema` -- the configuration document as a JSON Schema.
///
/// Unauthenticated, like `/api/v1/access` and `/api/v1/status`: this describes the
/// shape of a configuration, never the contents of one. The identical bytes are
/// published on GitHub and attached to each release, so a token here would guard
/// nothing while stopping the dashboard from validating anything before a caller
/// signs in.
#[utoipa::path(
    get, path = "/api/v1/schema", tag = "process",
    responses((status = 200, description = "The configuration schema, JSON Schema 2020-12")),
)]
pub(crate) async fn schema() -> Response {
    (
        [
            // The registered type for a JSON Schema document. Every JSON parser
            // reads it as JSON regardless, and an editor that looks at the header
            // learns something true from this one.
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/schema+json"),
            ),
            // It cannot change without a restart, and a restart is usually a new
            // version -- so a short cache saves the bytes on a page reload without
            // outliving an upgrade by long.
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=600"),
            ),
        ],
        body(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::body;

    #[test]
    fn the_served_bytes_are_the_schema_the_code_generates() {
        // The same document `doppel config schema` prints and the repository keeps,
        // minus the pretty-printing. A second serialization here would be a second
        // schema, and the drift test in `doppel-core` would not see it.
        let served: serde_json::Value =
            serde_json::from_str(body()).expect("served bytes are JSON");
        assert_eq!(served, doppel_core::config::schema::json_schema());
    }

    #[test]
    fn the_schema_carries_the_bounds_a_client_validates_with() {
        // What the dashboard reads out of it. If a refactor stopped emitting these,
        // the page would quietly accept anything and let the server refuse it --
        // which is the behaviour this endpoint exists to replace.
        let served: serde_json::Value =
            serde_json::from_str(body()).expect("served bytes are JSON");
        let name = &served["$defs"]["ProxyName"];
        assert!(name["pattern"].is_string(), "no pattern on ProxyName");
        assert!(name["maxLength"].is_number(), "no maxLength on ProxyName");
        assert!(
            served["$defs"]["Ratio"]["maximum"].is_number(),
            "no maximum on Ratio"
        );
        assert!(
            served["$defs"]["ProxyConfig"]["properties"]["url"].is_object(),
            "no url property on ProxyConfig"
        );
    }
}
