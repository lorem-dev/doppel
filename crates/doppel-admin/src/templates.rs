//! Template files: list, upload, delete.

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use doppel_core::config::ProxyConfig;
use doppel_core::store::name::sanitize;
use doppel_core::{Config, Error, ErrorBody, ErrorCode};
use serde::Serialize;

use crate::access::{Action, authorize, caller_from_headers};
use crate::proxies::{find, load, not_found};
use crate::response::{ApiError, store_error};
use crate::state::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/api/v1/proxies/{name}/templates", get(list))
        .route(
            "/api/v1/proxies/{name}/templates/{file}",
            post(upload).delete(remove),
        )
        // Uploads are bounded by `admin.upload.limit`, which an operator sets
        // and which can legitimately exceed axum's 2 MB default. Leaving the
        // default in place would reject those with a response that is not
        // this API's error envelope, from a layer that cannot know what the
        // configured limit is. The body is still bounded -- `read_body`
        // below streams into a buffer that stops at the configured limit --
        // so disabling the default replaces one bound with a better one
        // rather than removing it.
        .route_layer(DefaultBodyLimit::disable())
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TemplateEntry {
    pub name: String,
    /// Size in bytes.
    pub size: usize,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TemplateList {
    pub templates: Vec<TemplateEntry>,
}

/// Every template file name the proxy's mocks declare.
///
/// Only `response.template` names a file. A mock's `response.headers` values
/// are inline templates rendered from the string itself, so they name
/// nothing on disk.
#[must_use]
pub fn declared(proxy: &ProxyConfig) -> Vec<String> {
    let mut names: Vec<String> = proxy
        .mocks
        .iter()
        .filter_map(|mock| mock.response.template.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

#[utoipa::path(
    get, path = "/api/v1/proxies/{name}/templates", tag = "templates",
    params(("name" = String, Path, description = "Proxy name")),
    responses(
        (status = 200, description = "Template files present for this proxy", body = TemplateList),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, description = "No such proxy", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn list(
    State(state): State<AdminState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let config = load(&state).await?;
    let caller = caller_from_headers(&config.admin, &headers);
    // Listing a proxy's files is a read of that proxy, so `read` governs it
    // rather than `upload`: seeing which templates are present is not a
    // change.
    authorize(&config.admin, find(&config, &name), Action::Read, &caller)?;
    require_proxy(&config, &name)?;

    let files = state
        .store()
        .load_templates(&name)
        .await
        .map_err(|err| store_error(&err))?;
    let templates = files
        .into_iter()
        .map(|file| TemplateEntry {
            name: file.name,
            size: file.content.len(),
        })
        .collect();

    Ok(axum::Json(TemplateList { templates }).into_response())
}

#[utoipa::path(
    post, path = "/api/v1/proxies/{name}/templates/{file}", tag = "templates",
    params(
        ("name" = String, Path, description = "Proxy name"),
        ("file" = String, Path, description = "File name, which some mock must name in `response.template`"),
    ),
    request_body(content = String, description = "The template source, as a raw body", content_type = "text/plain"),
    responses(
        (status = 204, description = "Stored. Uploading the same file again replaces it"),
        (status = 400, description = "The file name is not usable as one", body = ErrorBody),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, description = "No such proxy", body = ErrorBody),
        (status = 413, description = "Body over `admin.upload.limit`", body = ErrorBody),
        (status = 422, description = "No mock of this proxy declares that file", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn upload(
    State(state): State<AdminState>,
    Path((name, file)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let config = load(&state).await?;
    let caller = caller_from_headers(&config.admin, &headers);
    authorize(&config.admin, find(&config, &name), Action::Upload, &caller)?;

    // The three checks in the order the specification fixes: the name is
    // safe, the name is wanted, and only then the body. Reading the body
    // first would mean a request that was never going to be stored still
    // cost the bandwidth and the buffer.
    sanitize(&file).map_err(|err| store_error(&err))?;
    let proxy = require_proxy(&config, &name)?;
    if !declared(proxy).contains(&file) {
        return Err(Error::new(
            ErrorCode::TemplateNotDeclared,
            format!(
                "no mock of proxy `{name}` names `{file}` in `response.template`; \
                 declare it before uploading it"
            ),
        )
        .into());
    }

    let bytes = read_body(body, &headers, config.admin.upload.limit.0).await?;
    state
        .store()
        .save_template(&name, &file, &bytes)
        .await
        .map_err(|err| store_error(&err))?;

    // No body, and the same answer whether the file was new or replaced.
    // Upload is idempotent -- two identical uploads leave identical state --
    // and a `201` on the first would make a client's retry look like a
    // different outcome than the original.
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[utoipa::path(
    delete, path = "/api/v1/proxies/{name}/templates/{file}", tag = "templates",
    params(
        ("name" = String, Path, description = "Proxy name"),
        ("file" = String, Path, description = "File name"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 400, description = "The file name is not usable as one", body = ErrorBody),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, description = "No such proxy, or no such file", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn remove(
    State(state): State<AdminState>,
    Path((name, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let config = load(&state).await?;
    let caller = caller_from_headers(&config.admin, &headers);
    authorize(&config.admin, find(&config, &name), Action::Upload, &caller)?;

    sanitize(&file).map_err(|err| store_error(&err))?;
    require_proxy(&config, &name)?;

    let existed = state
        .store()
        .delete_template(&name, &file)
        .await
        .map_err(|err| store_error(&err))?;
    if !existed {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!("proxy `{name}` has no template file `{file}`"),
        )
        .into());
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn require_proxy<'a>(config: &'a Config, name: &str) -> Result<&'a ProxyConfig, Error> {
    find(config, name).ok_or_else(|| not_found(name))
}

/// Collect an upload, refusing anything over `limit` bytes.
async fn read_body(body: Body, headers: &HeaderMap, limit: u64) -> Result<Vec<u8>, Error> {
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

fn too_large(limit: usize) -> Error {
    Error::new(
        ErrorCode::UploadTooLarge,
        format!("template body exceeds the configured upload limit of {limit} bytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration whose one proxy carries the given mocks, so `declared`
    /// is exercised against documents built the way real ones are -- by
    /// parsing -- rather than by hand-assembling structs that could drift
    /// from the schema.
    fn proxy_with(mocks: &str) -> ProxyConfig {
        let yaml = format!(
            r#"
server:
  host: "127.0.0.1"
  port: 18080
admin:
  host: "127.0.0.1"
  port: 18081
  upload:
    limit: 1M
proxies:
  - name: alpha
    type: http
    url: "https://alpha.example.com/api/"
    mocks:{mocks}
"#
        );
        doppel_core::config::load_from_str(&yaml)
            .expect("test config parses")
            .proxies
            .remove(0)
    }

    const ONE_MOCK: &str = r#"
      - name: m
        request:
          method: GET
          url: /x/
        response:
          status: 200
          template: {file}
"#;

    fn mocks(files: &[&str]) -> String {
        files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                ONE_MOCK
                    .replace("name: m", &format!("name: m{index}"))
                    .replace("{file}", file)
            })
            .collect()
    }

    #[test]
    fn declared_names_every_template_once() {
        // Sorted and deduplicated: two mocks may render the same file, and a
        // duplicate in the keep list would make `retain_templates` compare
        // the same name twice for no reason.
        let proxy = proxy_with(&mocks(&["b.j2", "a.j2", "b.j2"]));
        assert_eq!(declared(&proxy), vec!["a.j2".to_owned(), "b.j2".to_owned()]);
    }

    #[test]
    fn a_mock_without_a_template_declares_nothing() {
        let inline = r#"
      - name: inline
        request:
          method: GET
          url: /x/
        response:
          status: 200
          json: '{"ok": true}'
          headers:
            X-Trace: "{{ trace }}"
"#;
        // `response.headers` values are rendered from the string itself, so
        // they name no file however template-like they look.
        assert!(declared(&proxy_with(inline)).is_empty());
    }

    #[test]
    fn a_proxy_with_no_mocks_declares_nothing() {
        assert!(declared(&proxy_with(" []")).is_empty());
    }

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
        // configured, and only a body that announces its length would hit
        // it.
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
        // rejection: a client that lies large is stopped before it can
        // stream anything at all.
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, "999".parse().unwrap());
        let err = read_body(Body::empty(), &headers, 8).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UploadTooLarge);
    }
}
