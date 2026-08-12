//! The admin HTTP API: proxy CRUD, template upload, reload, status, metrics.

pub mod access;
pub mod body;
pub mod dashboard;
pub mod openapi;
pub mod proxies;
pub mod response;
pub mod rights;
pub mod schema;
pub mod state;
pub mod status;
pub mod templates;

use tower::Layer as _;
use tower_http::normalize_path::{NormalizePath, NormalizePathLayer};

pub use access::{Action, Caller, authorize, caller_from_headers};
pub use response::ApiError;
pub use state::AdminState;

/// The admin router, ready to serve.
///
/// A router rather than a server: `serve` owns the listener and the shutdown
/// signal, and the tests drive this directly without binding a port.
pub fn router(state: AdminState) -> axum::Router {
    // Read from the startup configuration, not the reloaded one: routes are
    // built once, so turning the dashboard on or off takes a restart -- the same
    // rule `admin.enable` already follows, and `main.example.yaml` says so.
    let dashboard = state.startup().admin.is_dashboard_enabled();

    let mut router = axum::Router::new()
        .merge(proxies::routes())
        .merge(templates::routes())
        .merge(status::routes())
        .merge(rights::routes())
        .merge(schema::routes())
        .merge(openapi::routes());

    if dashboard {
        router = router.merge(dashboard::routes());
    }

    // Without these two, a typo'd path answers 404 with an empty body and a wrong
    // method answers 405 with one. The contract is that every error carries the
    // envelope, and the paths a client is most likely to hit by accident were the
    // ones that did not.
    //
    // With the dashboard on, the 404 half becomes its fallback: a GET outside
    // `/api/` and `/static/` is a client-side route being reloaded and gets the
    // page, and everything else keeps the envelope. That division is what makes
    // "everything under /api/ is the API, everything else is the page" true rather
    // than merely intended.
    if dashboard {
        router = router.fallback(dashboard::fallback);
    } else {
        router = router.fallback(not_found);
    }

    router
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
}

async fn not_found(method: axum::http::Method, uri: axum::http::Uri) -> ApiError {
    // The path is echoed because it is the client's own, and naming it is
    // what turns "404" into "you asked for something that is not here".
    doppel_core::Error::new(
        doppel_core::ErrorCode::NotFound,
        format!("no route for {method} {}", uri.path()),
    )
    .into()
}

async fn method_not_allowed(method: axum::http::Method, uri: axum::http::Uri) -> ApiError {
    // Deliberately distinct from the 404 above: the resource exists and the
    // verb is wrong, which a client fixes differently from a wrong path.
    doppel_core::Error::new(
        doppel_core::ErrorCode::MethodNotAllowed,
        format!("{} does not accept {method}", uri.path()),
    )
    .into()
}

/// The router with a trailing slash trimmed before routing.
///
/// `/metrics/` and `/api/v1/status/` answer as `/metrics` and `/api/v1/status`
/// do. Axum stopped redirecting between the two spellings in 0.8, and a 404 for
/// a slash is a poor answer for a path an operator typed or a scrape config
/// carried over from somewhere else.
///
/// A layer around the router rather than on it: `Router::layer` runs after
/// routing has already decided there is nothing there, which is too late to
/// rewrite the path.
///
/// The proxy listener deliberately gets none of this. A proxied path is relayed
/// byte for byte -- `/orders/` and `/orders` are two resources upstream, and one
/// of this project's own mocks matches `^/api/v1/resource/9/$`.
#[must_use]
pub fn app(state: AdminState) -> NormalizePath<axum::Router> {
    NormalizePathLayer::trim_trailing_slash().layer(router(state))
}

/// Bind nothing, serve until `shutdown` resolves.
///
/// Mirrors `doppel_proxy::serve`: the caller owns the listener and the
/// shutdown signal, so `serve` in the CLI can bind both ports before either
/// starts and fail startup as a whole if one of them cannot be had.
pub async fn serve(
    state: AdminState,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    // `into_make_service` from `axum::ServiceExt`, not the `Router` method: the
    // trailing-slash layer makes this a `Service` rather than a `Router`.
    axum::serve(
        listener,
        axum::ServiceExt::<axum::extract::Request>::into_make_service(app(state)),
    )
    .with_graceful_shutdown(shutdown)
    .await
}
