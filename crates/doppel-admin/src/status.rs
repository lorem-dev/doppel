//! Reloading the running configuration, and reporting what it is.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use doppel_core::config::{ResolveKind, redact_credentials};
use doppel_core::{Error, ErrorCode};
use serde::Serialize;

use crate::access::{Action, authorize, caller_from_headers};
use crate::response::{ApiError, config_invalid};
use crate::state::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/status", get(status))
        .route("/metrics", get(exposition))
        .route("/api/v1/config/reload", post(reload))
}

#[derive(Debug, Serialize)]
struct Status {
    uptime_seconds: u64,
    revision: String,
    proxies: Vec<ProxyStatus>,
}

#[derive(Debug, Serialize)]
struct ProxyStatus {
    name: String,
    upstream: String,
    resolve: String,
    mocks: usize,
}

#[derive(Debug, Serialize)]
struct ReloadReport {
    revision: String,
    proxies: usize,
    /// Sections that changed but need a restart. Absent when empty, so the
    /// common answer stays quiet rather than carrying an empty list every
    /// time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unapplied: Vec<String>,
}

/// What the process is serving right now.
///
/// Read from the running runtime, not from the store: a configuration that
/// has been written but not reloaded is not what this process is doing, and
/// reporting it would make `/status` agree with the file while disagreeing
/// with reality.
///
/// Unauthenticated by design -- it is the endpoint a load balancer calls, and
/// that is why every upstream goes through `redact_credentials` on the way
/// out.
async fn status(State(state): State<AdminState>) -> Response {
    let runtime = state.holder().load();
    let proxies = runtime
        .config
        .proxies
        .iter()
        .map(|proxy| ProxyStatus {
            name: proxy.name.clone(),
            upstream: redact_credentials(&proxy.url),
            resolve: match proxy.resolve.kind {
                ResolveKind::Default => "default".to_owned(),
                ResolveKind::Header => proxy
                    .resolve
                    .header
                    .as_ref()
                    .map_or_else(|| "header".to_owned(), |name| format!("header:{name}")),
            },
            mocks: proxy.mocks.len(),
        })
        .collect();

    axum::Json(Status {
        uptime_seconds: state.uptime().as_secs(),
        revision: runtime.revision.to_string(),
        proxies,
    })
    .into_response()
}

/// The Prometheus exposition.
///
/// Unauthenticated, like `/status`: a scraper is a machine on the operator's
/// network with no place to put a token, and the exposition names proxies and
/// counts -- never a token, a URL or a header value.
async fn exposition(State(state): State<AdminState>) -> Response {
    (
        // The text exposition format's registered content type. Without it
        // some scrapers fall back to guessing, and a guess of `text/plain`
        // without the version parameter is not the same contract.
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics().render(),
    )
        .into_response()
}

/// Promote the stored configuration to the running one.
async fn reload(State(state): State<AdminState>, headers: HeaderMap) -> Result<Response, ApiError> {
    // Authorized against the *running* configuration, unlike the CRUD
    // handlers, which authorize against the stored one. Each authorizes
    // against the policy governing the thing it changes: CRUD edits the
    // stored document, so that document's policy applies and a new proxy's
    // `access` override takes effect with the proxy itself. Reload changes
    // what the process serves, so the policy the operator actually put into
    // effect applies. Reading `admin.access` from the stored config here
    // would let that config authorise its own promotion -- anyone able to
    // write the file out of band could grant themselves the right to make
    // the process run it.
    let running = state.holder().load();
    let caller = caller_from_headers(&running.config.admin, &headers);
    authorize(&running.config.admin, None, Action::Update, &caller)?;
    // Dropped before the reload swaps the runtime: holding an arc-swap guard
    // across the swap would keep the old runtime alive for no reason.
    drop(running);

    // The same mutex the control socket takes. Two reloads that interleave
    // can swap in the wrong order and leave the process running the older of
    // the two configurations, so both entry points serialise on one lock --
    // see `AdminState::new`.
    let _guard = state.reload_lock().lock().await;

    let outcome = doppel_core::reload(state.holder(), state.store(), state.startup())
        .await
        .map_err(|failure| match failure.code {
            ErrorCode::ConfigInvalid => config_invalid(&failure.violations),
            code => Error::new(
                code,
                failure
                    .violations
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        })?;

    Ok(axum::Json(ReloadReport {
        // The hex form the rest of this API uses. The control socket reports
        // the same revision as a number, because that protocol has always
        // done so; within one API a single spelling matters more than
        // matching the other one.
        revision: outcome.revision.to_string(),
        proxies: outcome.proxies,
        unapplied: outcome
            .unapplied
            .into_iter()
            .map(ToOwned::to_owned)
            .collect(),
    })
    .into_response())
}
