//! Proxy create, read, update, delete and list.

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use doppel_core::config::ProxyConfig;
use doppel_core::store::{ConfigStore, Revision, StoreError};
use doppel_core::validate::validate;
use doppel_core::{Config, Error, ErrorBody, ErrorCode};
use serde::{Deserialize, Serialize};

use crate::access::{Action, authorize, caller_from_headers_with_env, policy};
use crate::body::{MAX_DOCUMENT_BYTES, read_body};
use crate::response::{ApiError, config_invalid, store_error};
use crate::state::AdminState;

/// How many times a handler rebuilds its change after losing a
/// compare-and-swap race.
///
/// The store's token covers the whole configuration, so an edit to an
/// unrelated proxy invalidates it too. Retrying absorbs that; the bound is
/// what stops a process that is being written to continuously from spinning
/// forever. Four is enough that ordinary contention never reaches it and
/// small enough that a pathological writer is reported rather than waited
/// out.
const MAX_SAVE_ATTEMPTS: usize = 4;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/api/v1/proxies", get(list).post(create))
        .route(
            "/api/v1/proxies/{name}",
            get(read).put(update).delete(remove),
        )
        // Replaced, not removed: `read_body` below bounds these bodies at
        // `MAX_DOCUMENT_BYTES`. axum's own limit answers with a plain-text
        // 413 from a layer that cannot produce this API's error envelope.
        .route_layer(DefaultBodyLimit::disable())
}

/// One proxy as the API represents it: the configuration document, plus the
/// revision a client must send back to change it.
///
/// The revision is a sibling of the proxy rather than a field inside it so
/// that `proxy` stays exactly the document that goes in a `main.yaml` -- a
/// client can lift it out of a response and paste it into a file.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProxyView {
    /// Sixteen hex digits. Send it back in `If-Match` to update.
    pub revision: String,
    pub proxy: ProxyConfig,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProxyList {
    pub proxies: Vec<ProxyView>,
}

/// The request body for create and update. Symmetrical with `ProxyView` on
/// purpose: what a client reads is what it sends back.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProxyRequest {
    /// Required on update, refused on create.
    #[serde(default)]
    pub revision: Option<String>,
    pub proxy: ProxyConfig,
}

#[utoipa::path(
    get, path = "/api/v1/proxies", tag = "proxies",
    responses(
        (status = 200, description = "Every proxy, each with its revision", body = ProxyList),
        (status = 401, description = "No token, where one is required", body = ErrorBody),
        (status = 403, description = "Token without the `list` right", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn list(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // Policy from the running configuration, data from the store: see
    // `access::policy`.
    let policy = policy(&state);
    let caller = caller_from_headers_with_env(&policy.admin, state.env_tokens(), &headers);
    authorize(&policy.admin, None, Action::List, &caller)?;

    let config = load(&state).await?;
    let proxies = config.proxies.iter().map(view).collect();
    Ok(axum::Json(ProxyList { proxies }).into_response())
}

#[utoipa::path(
    get, path = "/api/v1/proxies/{name}", tag = "proxies",
    params(("name" = String, Path, description = "Proxy name")),
    responses(
        (status = 200, description = "The proxy and its revision, also in `ETag`", body = ProxyView),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, description = "No such proxy", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn read(
    State(state): State<AdminState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let policy = policy(&state);
    let caller = caller_from_headers_with_env(&policy.admin, state.env_tokens(), &headers);
    // Authorization first, existence second. Swapping these turns the pair of
    // statuses into a way to enumerate proxy names -- see `authorize`.
    //
    // The per-proxy override comes from the running configuration too, so a
    // proxy that exists only in the store is authorized under the global
    // policy. That is the safe direction: an override can widen access, and
    // one nobody has reloaded is not yet in force.
    authorize(&policy.admin, find(&policy, &name), Action::Read, &caller)?;

    let config = load(&state).await?;
    let proxy = find(&config, &name).ok_or_else(|| not_found(&name))?;
    Ok(with_etag(StatusCode::OK, view(proxy), None))
}

#[utoipa::path(
    post, path = "/api/v1/proxies", tag = "proxies",
    request_body = ProxyRequest,
    responses(
        (status = 201, description = "Created; `Location` and `ETag` are set", body = ProxyView),
        (status = 400, description = "Malformed body, or a `revision` was sent", body = ErrorBody),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 409, description = "A proxy of that name exists, or the store is under contention", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn create(
    State(state): State<AdminState>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let policy = policy(&state);
    let caller = caller_from_headers_with_env(&policy.admin, state.env_tokens(), &headers);
    authorize(&policy.admin, None, Action::Create, &caller)?;
    drop(policy);

    let request = parse_request(&read_body(body, &headers, MAX_DOCUMENT_BYTES).await?)?;
    if request.revision.is_some() {
        // A revision names a version of something that already exists, so a
        // create cannot have one. Ignoring it would let a client that meant
        // to send a PUT overwrite a proxy it never read.
        return Err(Error::new(
            ErrorCode::ConfigInvalid,
            "`revision` must not be sent when creating a proxy; it identifies \
             a version that does not exist yet",
        )
        .into());
    }
    if headers.contains_key(header::IF_MATCH) {
        return Err(Error::new(
            ErrorCode::ConfigInvalid,
            "`If-Match` must not be sent when creating a proxy",
        )
        .into());
    }

    let proxy = request.proxy;
    let name = proxy.name.clone();
    // Checked before anything is written, because the success response has to
    // carry this name in a `Location` header and a value that cannot go in
    // one would otherwise surface as a panic after the write had landed.
    let location = HeaderValue::from_str(&format!("/api/v1/proxies/{name}")).map_err(|_| {
        Error::new(
            ErrorCode::ConfigInvalid,
            "proxy name cannot be represented in a URL",
        )
    })?;

    let created = commit(state.store(), |config| {
        if find(config, name.as_str()).is_some() {
            return Err(Error::new(
                ErrorCode::Conflict,
                format!("proxy `{name}` already exists; update it instead"),
            ));
        }
        config.proxies.push(proxy.clone());
        Ok(view(&proxy))
    })
    .await?;

    // A brand-new proxy owns no files, so anything under its name is a
    // leftover -- from a delete whose template sweep failed after the
    // configuration write had landed, which reports a 500 but cannot undo the
    // write. Without this, re-creating a proxy with the same name and the same
    // mock would silently render the *old* file. Nothing legitimate can be
    // here: an upload requires the proxy to exist already.
    state
        .store()
        .retain_templates(name.as_str(), &[])
        .await
        .map_err(|err| store_error(&err))?;

    Ok(with_etag(StatusCode::CREATED, created, Some(location)))
}

/// The rename landed in the configuration and the templates did not follow.
///
/// The configuration is written first, deliberately: the write is what authorises
/// moving anything. So a failure here leaves a proxy under its new name whose
/// template files are still under the old one, and the mocks naming them render
/// errors until somebody moves them. That is worth saying in full rather than
/// reporting as a store failure, because the fix is a one-line `mv` and the message
/// is the only place a reader could learn which two names to use.
fn renamed_but_stranded(from: &str, to: &str, err: &StoreError) -> Error {
    Error::new(
        ErrorCode::StoreError,
        format!(
            "`{from}` was renamed to `{to}` in the configuration, but its templates \
             could not be moved: {err}. The templates are still stored under `{from}`, \
             so any mock of `{to}` naming a template file will fail to render until \
             they are moved."
        ),
    )
}

#[utoipa::path(
    put, path = "/api/v1/proxies/{name}", tag = "proxies",
    params(
        ("name" = String, Path, description = "Proxy name; a different name in the body renames it"),
        ("If-Match" = Option<String>, Header, description = "The revision read earlier, quoted"),
    ),
    request_body = ProxyRequest,
    responses(
        (status = 200, description = "Replaced; the new revision is in the body and `ETag`", body = ProxyView),
        (status = 400, description = "Malformed body, a taken name, or two disagreeing revisions", body = ErrorBody),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 409, description = "The proxy changed since it was read", body = ErrorBody),
        (status = 428, description = "No revision was supplied", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn update(
    State(state): State<AdminState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let policy = policy(&state);
    let caller = caller_from_headers_with_env(&policy.admin, state.env_tokens(), &headers);
    authorize(&policy.admin, find(&policy, &name), Action::Update, &caller)?;
    drop(policy);

    let request = parse_request(&read_body(body, &headers, MAX_DOCUMENT_BYTES).await?)?;
    let expected = required_revision(&headers, request.revision.as_deref())?;

    let proxy = request.proxy;
    // A name in the body that differs from the path is a rename, not a mistake. It
    // used to be refused, because a proxy's name is also where its templates live
    // and a rename that left them behind would point every mock at a file that is
    // not there -- so the rename moves them, below.
    let renamed = (proxy.name.as_str() != name).then(|| proxy.name.to_string());

    let updated = commit(state.store(), |config| {
        let index = position(config, &name).ok_or_else(|| not_found(&name))?;
        // Re-checked on every attempt, against whatever the store holds now.
        // That is what makes a retry safe: an unrelated edit passes this
        // again, an edit to the same proxy does not.
        if Revision::of_proxy(&config.proxies[index]) != expected {
            return Err(stale(&name));
        }
        if let Some(new_name) = &renamed
            && position(config, new_name).is_some()
        {
            // Caught here rather than left to validation, which would report it
            // against a position in the document -- `proxies[3].name` -- when what
            // the caller needs to know is that the name is taken.
            return Err(Error::new(
                ErrorCode::ConfigInvalid,
                format!("a proxy named `{new_name}` already exists"),
            ));
        }
        config.proxies[index] = proxy.clone();
        Ok(view(&proxy))
    })
    .await?;

    // Both of these follow the configuration write, and in this order. The write is
    // what authorises touching the templates at all -- the same reason `delete` waits
    // for it -- and a rename has to happen before the files are pruned, because after
    // it they are the new name's.
    if let Some(new_name) = &renamed {
        state
            .store()
            .rename_templates(name.as_str(), new_name)
            .await
            .map_err(|err| renamed_but_stranded(&name, new_name, &err))?;
    }
    // The update landed, so the mocks it removed are gone and the files only
    // they named can no longer be rendered.
    state
        .store()
        .retain_templates(
            renamed.as_deref().unwrap_or(name.as_str()),
            &crate::templates::declared(&updated.proxy),
        )
        .await
        .map_err(|err| store_error(&err))?;

    Ok(with_etag(StatusCode::OK, updated, None))
}

#[utoipa::path(
    delete, path = "/api/v1/proxies/{name}", tag = "proxies",
    params(
        ("name" = String, Path, description = "Proxy name"),
        ("If-Match" = Option<String>, Header, description = "Optional: delete only if unchanged"),
    ),
    responses(
        (status = 204, description = "Deleted, along with its template files"),
        (status = 400, description = "Deleting it would leave an invalid configuration", body = ErrorBody),
        (status = 401, body = ErrorBody), (status = 403, body = ErrorBody),
        (status = 404, body = ErrorBody),
        (status = 409, description = "The proxy changed since it was read", body = ErrorBody),
    ),
    security(("token" = [])),
)]
pub(crate) async fn remove(
    State(state): State<AdminState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let policy = policy(&state);
    let caller = caller_from_headers_with_env(&policy.admin, state.env_tokens(), &headers);
    authorize(&policy.admin, find(&policy, &name), Action::Delete, &caller)?;
    drop(policy);

    // Optional, unlike on update. A delete names its target completely and
    // carries no fields the client might be overwriting unread, so there is
    // no lost update for a precondition to prevent -- but a client that wants
    // to say "only if it is still the one I looked at" is honoured.
    let expected = header_revision(&headers)?;

    commit(state.store(), |config| {
        let index = position(config, &name).ok_or_else(|| not_found(&name))?;
        if let Some(expected) = expected
            && Revision::of_proxy(&config.proxies[index]) != expected
        {
            return Err(stale(&name));
        }
        config.proxies.remove(index);
        Ok(())
    })
    .await?;

    // Only after the configuration no longer names the proxy. The other order
    // has a window in which a live proxy has had its templates deleted, which
    // turns every request it serves into a render failure; this order's worst
    // case is orphaned files that nothing reads.
    state
        .store()
        .retain_templates(name.as_str(), &[])
        .await
        .map_err(|err| store_error(&err))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Load, apply a change, validate the result and save it under
/// compare-and-swap, retrying when -- and only when -- the collision was with
/// someone else's unrelated write.
///
/// `apply` runs again from scratch on every attempt, so any check it makes
/// (existence, the client's per-proxy revision) is evaluated against the
/// configuration actually being written to. An error from `apply` is the
/// client's and returns immediately: retrying a genuine conflict would either
/// hide it or eventually overwrite it.
async fn commit<T>(
    store: &dyn ConfigStore,
    mut apply: impl FnMut(&mut Config) -> Result<T, Error>,
) -> Result<T, Error> {
    for _ in 0..MAX_SAVE_ATTEMPTS {
        let (mut config, base) = store.load().await.map_err(|err| store_error(&err))?;
        let outcome = apply(&mut config)?;
        // The full rule set, not just the rules about the proxy that changed:
        // a single-proxy edit can break a whole-config invariant such as
        // "at most one default resolver".
        validate(&config).map_err(|violations| config_invalid(&violations))?;

        match store.save(&config, Some(base)).await {
            Ok(_) => return Ok(outcome),
            Err(StoreError::RevisionMismatch { .. }) => {}
            Err(err) => return Err(store_error(&err)),
        }
    }

    // Deliberately not `REVISION_MISMATCH`: the client's revision was current
    // every time it was checked. Telling them to re-read would be advice that
    // cannot help, because nothing about their request is wrong.
    Err(Error::new(
        ErrorCode::Conflict,
        format!(
            "the configuration is being changed concurrently; \
             gave up after {MAX_SAVE_ATTEMPTS} attempts"
        ),
    ))
}

pub(crate) async fn load(state: &AdminState) -> Result<Config, Error> {
    state
        .store()
        .load()
        .await
        .map(|(config, _)| config)
        .map_err(|err| store_error(&err))
}

pub(crate) fn find<'a>(config: &'a Config, name: &str) -> Option<&'a ProxyConfig> {
    config.proxies.iter().find(|proxy| proxy.name == name)
}

fn position(config: &Config, name: &str) -> Option<usize> {
    config.proxies.iter().position(|proxy| proxy.name == name)
}

fn view(proxy: &ProxyConfig) -> ProxyView {
    ProxyView {
        revision: Revision::of_proxy(proxy).to_string(),
        proxy: proxy.clone(),
    }
}

pub(crate) fn not_found(name: &str) -> Error {
    Error::new(ErrorCode::NotFound, format!("no proxy named `{name}`"))
}

fn stale(name: &str) -> Error {
    Error::new(
        ErrorCode::RevisionMismatch,
        format!("proxy `{name}` has changed since you read it; re-read it and retry"),
    )
}

fn parse_request(body: &[u8]) -> Result<ProxyRequest, Error> {
    serde_json::from_slice(body).map_err(|err| {
        Error::new(
            ErrorCode::ConfigInvalid,
            format!("request body is not a valid proxy document: {err}"),
        )
    })
}

/// Attach the revision as a strong `ETag`, and optionally a `Location`.
fn with_etag(status: StatusCode, body: ProxyView, location: Option<HeaderValue>) -> Response {
    // A revision is sixteen hex digits, so the quoted form is always a valid
    // header value; `from_str` is still used rather than an unchecked
    // constructor so that a future change to the revision format fails here
    // instead of producing a malformed header.
    let etag = HeaderValue::from_str(&format!("\"{}\"", body.revision));
    let mut response = (status, axum::Json(body)).into_response();
    if let Ok(etag) = etag {
        response.headers_mut().insert(header::ETAG, etag);
    }
    if let Some(location) = location {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

/// The revision carried by `If-Match`, if any.
///
/// Quotes are optional on the way in and always present on the way out: a
/// client that copies the `ETag` verbatim and one that copies the body's
/// `revision` field are both understood, and neither has to know which form
/// this server prefers.
fn header_revision(headers: &HeaderMap) -> Result<Option<Revision>, Error> {
    let Some(value) = headers.get(header::IF_MATCH) else {
        return Ok(None);
    };
    let text = value
        .to_str()
        .map_err(|_| bad_revision("<non-ASCII>"))?
        .trim();

    if text == "*" {
        // RFC 9110 reads `If-Match: *` as "if the resource exists at all",
        // which here means "overwrite whatever is there" -- exactly the lost
        // update this precondition exists to stop. Accepting it would let a
        // client disable the check by sending one character.
        return Err(Error::new(
            ErrorCode::RevisionRequired,
            "`If-Match: *` is not accepted; send the revision you read",
        ));
    }

    let unquoted = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text);
    parse_revision(unquoted).map(Some)
}

fn parse_revision(text: &str) -> Result<Revision, Error> {
    text.parse().map_err(|_| bad_revision(text))
}

fn bad_revision(text: &str) -> Error {
    Error::new(
        ErrorCode::ConfigInvalid,
        format!("`{text}` is not a revision: expected 16 hexadecimal digits"),
    )
}

/// The revision an update must carry, from `If-Match` or from the body.
fn required_revision(headers: &HeaderMap, body: Option<&str>) -> Result<Revision, Error> {
    let from_header = header_revision(headers)?;
    let from_body = body.map(parse_revision).transpose()?;

    match (from_header, from_body) {
        (Some(header), Some(body)) if header != body => Err(Error::new(
            ErrorCode::ConfigInvalid,
            "`If-Match` and the body's `revision` disagree; send one of them, \
             or make them equal",
        )),
        (Some(revision), _) | (None, Some(revision)) => Ok(revision),
        (None, None) => Err(Error::new(
            ErrorCode::RevisionRequired,
            "an update must carry the revision it was built from, in `If-Match` \
             or as the body's `revision` field; read the proxy to obtain one",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(if_match: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_str(if_match).unwrap());
        headers
    }

    #[test]
    fn a_quoted_and_an_unquoted_revision_parse_to_the_same_value() {
        let quoted = header_revision(&headers("\"00000000000000ff\"")).unwrap();
        let bare = header_revision(&headers("00000000000000ff")).unwrap();
        assert_eq!(quoted, Some(Revision(255)));
        assert_eq!(quoted, bare);
    }

    #[test]
    fn a_revision_round_trips_through_its_wire_form() {
        // Whatever `view` puts in a response must come back through
        // `If-Match` as the same value, including for a revision with
        // leading zeroes, which is where a non-padded format would break.
        for raw in [0, 1, 255, u64::MAX, 0x0123_4567_89ab_cdef] {
            let wire = Revision(raw).to_string();
            assert_eq!(wire.len(), 16, "{wire}");
            assert_eq!(
                header_revision(&headers(&wire)).unwrap(),
                Some(Revision(raw))
            );
        }
    }

    #[test]
    fn a_truncated_revision_is_rejected_rather_than_read_as_a_smaller_number() {
        // `ff` parses fine as hex. Accepting it would turn a client bug into
        // a revision mismatch, which advises a re-read that cannot fix it.
        let err = header_revision(&headers("\"ff\"")).unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigInvalid);
    }

    #[test]
    fn an_absent_if_match_is_not_an_error_on_its_own() {
        assert_eq!(header_revision(&HeaderMap::new()).unwrap(), None);
    }

    #[test]
    fn if_match_wildcard_is_revision_required_not_a_parse_error() {
        let err = header_revision(&headers("*")).unwrap_err();
        assert_eq!(err.code, ErrorCode::RevisionRequired);
        assert_eq!(err.status(), 428);
    }

    #[test]
    fn required_revision_accepts_either_source_and_rejects_neither() {
        let wire = Revision(42).to_string();
        assert_eq!(
            required_revision(&headers(&wire), None).unwrap(),
            Revision(42)
        );
        assert_eq!(
            required_revision(&HeaderMap::new(), Some(&wire)).unwrap(),
            Revision(42)
        );
        assert_eq!(
            required_revision(&headers(&wire), Some(&wire)).unwrap(),
            Revision(42)
        );
        assert_eq!(
            required_revision(&HeaderMap::new(), None).unwrap_err().code,
            ErrorCode::RevisionRequired
        );
    }

    #[test]
    fn required_revision_refuses_two_different_answers() {
        let err = required_revision(
            &headers(&Revision(1).to_string()),
            Some(&Revision(2).to_string()),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ConfigInvalid);
    }
}
