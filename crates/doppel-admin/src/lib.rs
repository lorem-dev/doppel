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

use axum::extract::{MatchedPath, Request};
use tower::Layer as _;
use tower::util::{MapRequest, MapRequestLayer};
use tower_http::compression::CompressionLayer;

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

/// The admin service: the routes, compressed, with a trailing slash trimmed
/// before routing.
pub type App = MapRequest<axum::Router, fn(Request) -> Request>;

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
/// Compression, and where it applies.
///
/// The dashboard is 140 KB of JavaScript and CSS uncompressed and 74 KB gzipped,
/// and every visitor was paying the difference: the assets were embedded as bytes
/// and served as bytes. The Swagger UI's own stylesheet is another 150 KB. Both
/// are text, both are served by this listener, and every browser asks for them
/// compressed.
///
/// `br` and `gzip`, negotiated from `Accept-Encoding` -- brotli first when the
/// client takes both, because it is smaller on text and every browser that speaks
/// it says so. A client that asks for neither gets the bytes as they are.
///
/// On this listener only. The proxy listener relays what the upstream sent,
/// byte for byte and encoding included: a client under test asking for
/// `identity` and getting gzip because a proxy decided to help is a bug in the
/// thing it is testing against.
#[must_use]
pub fn app(state: AdminState) -> App {
    // Latency inside compression, so what it measures is the work rather than the
    // deflating: a scrape of a 150 KB stylesheet should not read as a slow route.
    // Also inside routing, which is what makes `MatchedPath` available -- the
    // template is the label, and a label built from the path itself would put one
    // series per proxy name and per query string into the exposition.
    //
    // Compression on the router rather than around it: `axum::serve` wants a
    // service whose body is axum's own, and `Router::layer` is what converts the
    // compressed body back into one. Running after routing costs nothing here --
    // a response is compressed whatever matched it.
    MapRequestLayer::new(trim_trailing_slash as fn(Request) -> Request).layer(
        router(state)
            .layer(axum::middleware::from_fn(record_admin_request))
            .layer(CompressionLayer::new().br(true).gzip(true)),
    )
}

/// One admin request, timed and recorded by the route it matched.
///
/// `MatchedPath` is the template the router matched -- `/api/v1/proxies/{name}`,
/// never `/api/v1/proxies/alpha` -- so a deployment with a hundred proxies has one
/// series and a query string has none. A request that matched nothing has no
/// template, and is recorded under the empty route for the same reason an
/// unresolved proxy request is: the total has to be the total.
async fn record_admin_request(
    matched: Option<MatchedPath>,
    request: Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = request.method().as_str().to_owned();
    let route = matched.map_or_else(
        || doppel_core::metrics::UNMATCHED.to_owned(),
        |matched| matched.as_str().to_owned(),
    );

    let started = std::time::Instant::now();
    let response = next.run(request).await;
    doppel_core::metrics::record_admin(
        &method,
        &route,
        response.status().as_u16(),
        started.elapsed(),
    );
    response
}

/// `/metrics/` becomes `/metrics`, and two paths are left exactly as they came.
///
/// `/` is not a trailing slash to be trimmed; trimming it would leave an empty
/// path that matches nothing.
///
/// The Swagger UI is the other one, and it is not a matter of taste:
/// `utoipa-swagger-ui` answers the bare `/swagger-ui` with `303 See Other` to
/// `/swagger-ui/`, so trimming the slash off turns the pair into a redirect that
/// arrives back where it started -- measured as exactly that, `303` with
/// `location: /swagger-ui/`, on a request for `/swagger-ui/`. Its own assets are
/// resolved relative to that path too, which is the second reason the subtree is
/// left alone.
///
/// Hand-written rather than `tower-http`'s `NormalizePathLayer`, which trims
/// unconditionally and has no way to exempt a subtree. It was that layer, and
/// this is what it broke.
fn trim_trailing_slash(mut request: Request) -> Request {
    let uri = request.uri();
    let path = uri.path();
    if path == "/" || !path.ends_with('/') || path.starts_with("/swagger-ui") {
        return request;
    }

    let trimmed = path.trim_end_matches('/');
    let rebuilt = match uri.query() {
        Some(query) => format!("{trimmed}?{query}"),
        None => trimmed.to_owned(),
    };
    // A path that came in parseable stays parseable with fewer bytes on the end;
    // if it somehow does not, the request goes through untouched rather than
    // being answered with a 400 nobody can act on.
    if let Ok(uri) = rebuilt.parse() {
        *request.uri_mut() = uri;
    }
    request
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
