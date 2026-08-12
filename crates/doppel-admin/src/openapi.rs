//! The OpenAPI document, derived from the handlers themselves.
//!
//! Nothing here restates a route, a status or a body shape. Every one of
//! those is an attribute on the handler that implements it, so the document
//! cannot describe an endpoint this binary does not serve -- which is the
//! failure mode of a hand-maintained file, and the whole reason for the
//! dependency.

use axum::Router;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::state::AdminState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Doppel admin API",
        description = "Drive the proxy configuration over HTTP: proxy CRUD, \
                       template upload, reload, status and metrics.",
    ),
    paths(
        crate::proxies::list,
        crate::proxies::create,
        crate::proxies::read,
        crate::proxies::update,
        crate::proxies::remove,
        crate::templates::list,
        crate::templates::upload,
        crate::templates::remove,
        crate::rights::rights,
        crate::schema::schema,
        crate::status::reload,
        crate::status::status,
        crate::status::exposition,
    ),
    components(schemas(
        crate::proxies::ProxyView,
        crate::proxies::ProxyList,
        crate::proxies::ProxyRequest,
        crate::templates::TemplateEntry,
        crate::templates::TemplateList,
        crate::rights::AccessReport,
        crate::rights::ActionRights,
        crate::rights::ProxyRights,
        crate::rights::CallerView,
        crate::status::Status,
        crate::status::ProxyStatus,
        crate::status::ReloadReport,
        doppel_core::ErrorBody,
    )),
    modifiers(&TokenScheme),
    tags(
        (name = "proxies", description = "The proxy set"),
        (name = "templates", description = "Template files belonging to a proxy"),
        (name = "process", description = "What this process is doing"),
    ),
)]
pub struct ApiDoc;

/// The token header, added as a modifier rather than declared inline.
///
/// The header name is `admin.auth.header` and therefore configurable, so the
/// document can only name the default. Saying so in the description is more
/// honest than presenting a configurable name as fixed.
struct TokenScheme;

impl Modify for TokenScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);
        components.add_security_scheme(
            "token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "X-Proxy-Authorization",
                "`Bearer {token}`, where the token is one of `admin.tokens`. \
                 The header name shown here is the default; `admin.auth.header` \
                 changes it.",
            ))),
        );
    }
}

/// `/openapi.json` and `/swagger-ui`.
///
/// Both outside `/api/`, and unversioned. Neither is a resource of the API: the
/// document *describes* every version this binary knows, and the UI is a page
/// served to a browser. `/api/` exists to keep endpoints from colliding with the
/// dashboard's pages, and these two are on the pages' side of that line -- which
/// is also where every tool looks for them, `/openapi.json` being the
/// conventional path the way `/metrics` is.
///
/// The Swagger UI assets are vendored into the binary rather than fetched by
/// the build script, which is the crate's default. A build that reaches the
/// network cannot run in an offline or egress-restricted environment, and the
/// artifact would depend on a remote host still serving the same file.
pub fn routes() -> Router<AdminState> {
    utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/openapi.json", ApiDoc::openapi())
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every route the admin listener actually serves, as the specification
    /// lists them. The document is checked against this rather than against
    /// itself, so an endpoint added without an annotation is a failure here
    /// instead of an absence nobody notices.
    const DOCUMENTED: &[(&str, &str)] = &[
        ("get", "/api/v1/proxies"),
        ("post", "/api/v1/proxies"),
        ("get", "/api/v1/proxies/{name}"),
        ("put", "/api/v1/proxies/{name}"),
        ("delete", "/api/v1/proxies/{name}"),
        ("get", "/api/v1/proxies/{name}/templates"),
        ("post", "/api/v1/proxies/{name}/templates/{file}"),
        ("delete", "/api/v1/proxies/{name}/templates/{file}"),
        ("post", "/api/v1/config/reload"),
        ("get", "/api/v1/access"),
        ("get", "/api/v1/schema"),
        ("get", "/api/v1/status"),
        ("get", "/metrics"),
    ];

    fn document() -> serde_json::Value {
        serde_json::from_str(&ApiDoc::openapi().to_json().expect("serializes")).expect("is JSON")
    }

    #[test]
    fn every_route_in_the_specification_is_documented() {
        let doc = document();
        for (method, path) in DOCUMENTED {
            let operation = doc.pointer(&format!(
                "/paths/{}/{method}",
                path.replace('~', "~0").replace('/', "~1")
            ));
            assert!(
                operation.is_some_and(|op| !op.is_null()),
                "{} {path} is missing from the document",
                method.to_uppercase()
            );
        }
    }

    #[test]
    fn the_document_describes_no_route_that_is_not_served() {
        let doc = document();
        let paths = doc["paths"].as_object().expect("paths object");
        let mut found = Vec::new();
        for (path, item) in paths {
            for method in item.as_object().expect("path item").keys() {
                found.push((method.clone(), path.clone()));
            }
        }
        assert_eq!(
            found.len(),
            DOCUMENTED.len(),
            "document has {} operations, the specification lists {}: {found:?}",
            found.len(),
            DOCUMENTED.len()
        );
    }

    #[test]
    fn every_error_the_api_returns_is_a_documented_response() {
        // A client writes its error handling from this document. A status the
        // API can produce but the document omits is a branch nobody writes.
        let doc = document();
        let expectations: &[(&str, &str, &[&str])] = &[
            (
                "post",
                "/api/v1/proxies",
                &["201", "400", "401", "403", "409"],
            ),
            (
                "put",
                "/api/v1/proxies/{name}",
                &["200", "400", "401", "403", "404", "409", "428"],
            ),
            (
                "post",
                "/api/v1/proxies/{name}/templates/{file}",
                &["204", "400", "401", "403", "404", "413", "422"],
            ),
        ];

        for (method, path, statuses) in expectations {
            let responses = doc
                .pointer(&format!(
                    "/paths/{}/{method}/responses",
                    path.replace('/', "~1")
                ))
                .unwrap_or_else(|| panic!("{method} {path} has no responses"));
            for status in *statuses {
                assert!(
                    responses.get(*status).is_some(),
                    "{method} {path} does not document {status}: {responses}"
                );
            }
        }
    }

    #[test]
    fn the_error_envelope_is_a_component_with_all_three_fields() {
        let doc = document();
        let schema = &doc["components"]["schemas"]["ErrorBody"];
        for field in ["status", "message", "code"] {
            assert!(
                schema["properties"].get(field).is_some(),
                "ErrorBody is missing `{field}`: {schema}"
            );
        }
    }

    #[test]
    fn the_token_header_is_declared_as_a_security_scheme() {
        let doc = document();
        let scheme = &doc["components"]["securitySchemes"]["token"];
        assert_eq!(scheme["in"], "header", "{scheme}");
        assert_eq!(scheme["name"], "X-Proxy-Authorization", "{scheme}");
    }

    #[test]
    fn a_subjects_field_documents_both_of_its_wire_forms() {
        // `Subjects` has a hand-written `Deserialize` accepting a string or a
        // list. A derived schema would describe the Rust enum instead, and a
        // client generated from it would send something the server rejects.
        let doc = document();
        // The field is a `$ref`, so the shape lives on the component.
        assert_eq!(
            doc["components"]["schemas"]["ProxyAccessConfig"]["properties"]["read"]["oneOf"][1]["$ref"],
            "#/components/schemas/Subjects"
        );
        let subjects = &doc["components"]["schemas"]["Subjects"];
        let forms = subjects["oneOf"].as_array().expect("a oneOf: {subjects}");
        assert_eq!(forms.len(), 2, "{subjects}");
        assert_eq!(forms[0]["type"], "string", "{subjects}");
        assert_eq!(forms[1]["type"], "array", "{subjects}");
        assert_eq!(forms[1]["items"]["type"], "string", "{subjects}");
    }

    #[test]
    fn a_byte_size_field_documents_both_of_its_wire_forms() {
        // Same reasoning as `Subjects`: `body_limit` accepts `1048576` and
        // `"1Mi"`, and a client generated from an integer-only schema could
        // not send the form the reference configuration uses.
        let doc = document();
        let size = &doc["components"]["schemas"]["ByteSize"];
        let forms = size["oneOf"].as_array().expect("a oneOf: {size}");
        assert_eq!(forms.len(), 2, "{size}");
        assert_eq!(forms[0]["type"], "integer", "{size}");
        assert_eq!(forms[1]["type"], "string", "{size}");
    }
}
